//! The far view: beyond the radius where voxels are loaded, land is drawn straight
//! from the continent simulation as a low-detail surface reaching to the horizon,
//! with lakes, the sea, and rivers as ribbons.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{Receiver, Sender, channel},
    },
    time::{Duration, Instant},
};

use bytemuck::{Pod, Zeroable};
use glam::{DVec3, Vec3};
use wgpu::util::DeviceExt;
use worldgen::{
    World,
    biome::Biome,
    continent::{Continent, NO_LAKE},
    features::{self, trees},
    macro_layout::MACRO_SIZE,
    scale::OLD_VOXEL,
    materials as mat,
    settlements::years::When,
    surface::{self, Piece},
    world::GameWorld,
};

use crate::gfx::Gpu;

/// Cells per tile edge. Tiles are the unit of frustum culling.
const TILE: usize = 64;
/// The far surface sits this far below the true height so that, where it meets voxel
/// terrain, it tucks under rather than standing proud of it.
const SINK: f32 = worldgen::scale::m(0.8);
/// Narrowest river drawn in the distance, in voxels.
const MIN_RIBBON_WIDTH: f32 = worldgen::scale::m(2.0);

/// Forest height carried per step of a land vertex's `flags`, in voxels.
const LIFT_UNIT: f32 = 0.75;
/// Largest `flags` value land may use.
const MAX_LIFT_FLAGS: f32 = 100.0;

/// Vertex of the far surface. `flags`: below 128 = land, in steps of how high its forest
/// stands, lakes included (see `far`); 128 = the sea plane; 255 = river ribbon
/// (drawn over the land it lies on).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct FarVertex {
    /// Position relative to the continent's origin corner.
    pub pos: [f32; 3],
    /// Land: its ground, as seen while the tree models still stand on it.
    pub color: [u8; 3],
    pub flags: u8,
    /// Terrain gradient (rise per voxel along x and z), scaled by 1/4 into -1..1.
    pub grad: [i16; 2],
    /// Land: rgb, how it looks out beyond the tree models, its forest seen as a canopy
    /// over the ground; then how far standing water rises above the ground, 128 at the
    /// shore, by `127 / WET_RANGE` a voxel (see [`wetness`]).
    pub far: [u8; 4],
    /// The biome (as u8), for the snow lying there (see `season::Places`); then the plant
    /// kind (voxel::plant) of its forest's canopy, 0 with none, with how much of that
    /// forest keeps its leaves in winter in the five bits above it (see [`pack_plant`]);
    /// then its [`Skylines`] toward sunrise and sunset.
    pub place: [u8; 4],
    /// Land: the wash of the flowers or heather that lie over it (its number in the
    /// globals' tables, 0 for none), which the shader colours by the date as it does the
    /// nearer ground; how much of the ground's colour it takes the place of; and how much
    /// of that is still seen under the land's forest, out beyond the tree models. Last,
    /// how far the land lies in a hollow or on a ridge (see [`hollow`]).
    pub wash: [u8; 4],
}

impl FarVertex {
    /// A vertex of water: the sea plane, or a lake or river in the surface.
    pub fn water(pos: [f32; 3], color: [u8; 3], flags: u8) -> Self {
        Self {
            pos,
            color,
            flags,
            grad: [0, 0],
            far: [color[0], color[1], color[2], 0],
            // The sea takes the snow and ice of where the camera is (`Biome::Ocean`).
            place: [Biome::Ocean as u8, 0, 0, 0],
            wash: [0; 4],
        }
    }
}

/// How high the land stands against the sky from each grid cell of a continent, looking
/// toward where the sun rises and where it sets. With these the far view knows when a
/// hill has the low sun behind it, at any hour, without a shadow map.
pub struct Skylines {
    size: usize,
    cell: f32,
    /// Per grid cell: the skyline toward sunrise, then toward sunset, each the tangent
    /// of its angle above level as [`Skylines::pack`] stores it.
    pub angles: Vec<[u8; 2]>,
}

/// The sun's bearing when it is low, morning and evening (x, z): it rises and sets a
/// little to one side of the x axis (see `Environment::at`).
const SUN_BEARINGS: [(f32, f32); 2] = [(0.905, 0.426), (-0.905, 0.426)];
/// The steepest skyline kept, as a tangent. A sun higher than this clears every hill.
const SKYLINE_MAX: f32 = 1.5;
/// How far a hill is looked for, in grid cells, nearest first: closely nearby, where a
/// cell matters, and more loosely out to a few kilometres.
const SKYLINE_REACH: [f32; 22] = [
    1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0, 12.0, 14.0, 16.0, 20.0, 24.0, 28.0, 32.0, 40.0, 48.0, 56.0, 64.0, 80.0, 96.0,
    120.0,
];

/// A continent's skylines, worked out once by whoever asks first, and kept.
#[derive(Default)]
struct SkySlot {
    done: OnceLock<Arc<Skylines>>,
    /// Whether someone has started on them, so no one else need.
    claimed: AtomicBool,
    /// When they were last asked for (see `now_ms`), to forget the longest unused.
    used: AtomicU64,
}

/// The skylines kept, by world seed and continent. The lock is only ever held to find or
/// add a slot, never while skylines are worked out: they are worked out in the slot.
static KEPT: Mutex<Vec<((u32, i32, i32), Arc<SkySlot>)>> = Mutex::new(Vec::new());
/// The continent the camera is on (see [`Skylines::look_from`]): its skylines and those
/// of the continents around it are never forgotten.
static VIEW: Mutex<Option<(i32, i32)>> = Mutex::new(None);
/// How many continents' skylines are kept: more only while all are in view or in use.
const SKYLINES_KEPT: usize = 12;
/// Skylines asked for this lately, in ms, are in use, and kept.
const SKYLINES_IN_USE: u64 = 10_000;

fn now_ms() -> u64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now).elapsed().as_millis() as u64
}

// Skylines are worked out by whoever asks first, so that none waits for a thread that may
// never ask. That is only safe because the work is done in a plain loop: nothing in it
// calls rayon (a parallel iterator, `join`, `scope` or `install` from a pool worker lets
// the worker run other queued jobs while it waits, and one of them may want these same
// skylines on the same thread), and nothing in it asks for skylines. Keep it so. Pool
// jobs never wait for them at all: they ask with `soon`, and come back later.
impl Skylines {
    fn pack(tangent: f32) -> u8 {
        // More steps for the low angles, where the sun spends dawn and dusk.
        ((tangent.clamp(0.0, SKYLINE_MAX) / SKYLINE_MAX).sqrt() * 255.0).round() as u8
    }

    fn of(c: &Continent) -> Self {
        let size = c.size;
        let cell = c.cell_size();
        // Ground or sea, in voxels.
        let ground: Vec<f32> = c.height.iter().map(|h| (h * OLD_VOXEL).max(0.0)).collect();
        let at = |x: f32, z: f32| {
            let (x, z) = (x.clamp(0.0, size as f32 - 1.001), z.clamp(0.0, size as f32 - 1.001));
            let (i, j) = (x as usize, z as usize);
            let (fx, fz) = (x - i as f32, z - j as f32);
            let row = |j: usize| ground[j * size + i] * (1.0 - fx) + ground[j * size + i + 1] * fx;
            row(j) * (1.0 - fz) + row(j + 1) * fz
        };
        // Serially, on purpose: see above.
        let angles = (0..size * size)
            .map(|idx| {
                let (x, z) = ((idx % size) as f32, (idx / size) as f32);
                let here = ground[idx];
                SUN_BEARINGS.map(|(dx, dz)| {
                    let mut steepest = 0.0f32;
                    for reach in SKYLINE_REACH {
                        let rise = at(x + dx * reach, z + dz * reach) - here;
                        steepest = steepest.max(rise / (reach * cell));
                    }
                    Self::pack(steepest)
                })
            })
            .collect();
        Self { size, cell, angles }
    }

    #[cfg(test)]
    fn forget_all() {
        KEPT.lock().unwrap().clear();
    }

    /// The slot for a continent's skylines, made if there is none.
    fn slot(c: &Continent) -> Arc<SkySlot> {
        let key = (c.desc.world_seed, c.desc.mx, c.desc.mz);
        let mut kept = KEPT.lock().unwrap();
        let slot = match kept.iter().find(|(k, _)| *k == key) {
            Some((_, s)) => s.clone(),
            None => {
                if kept.len() >= SKYLINES_KEPT {
                    let view = *VIEW.lock().unwrap();
                    Self::forget_one(&mut kept, view, now_ms());
                }
                let s = Arc::new(SkySlot::default());
                kept.push((key, s.clone()));
                s
            }
        };
        slot.used.store(now_ms(), Ordering::Relaxed);
        slot
    }

    /// Forgets the skylines asked for longest ago, of those worked out, not in view and
    /// not in use, with the camera on continent `view` and the time `now`. If every one
    /// is, none is forgotten for now.
    fn forget_one(kept: &mut Vec<((u32, i32, i32), Arc<SkySlot>)>, view: Option<(i32, i32)>, now: u64) {
        let spare = kept
            .iter()
            .enumerate()
            .filter(|(_, ((_, mx, mz), s))| {
                let in_view = view.is_some_and(|(vx, vz)| (mx - vx).abs() <= 1 && (mz - vz).abs() <= 1);
                let used = s.used.load(Ordering::Relaxed);
                s.done.get().is_some() && !in_view && now.saturating_sub(used) > SKYLINES_IN_USE
            })
            .min_by_key(|(_, (_, s))| s.used.load(Ordering::Relaxed))
            .map(|(i, _)| i);
        if let Some(i) = spare {
            kept.remove(i);
        }
    }

    /// Tells the skylines which continent the camera is on, so that it and the ones
    /// around it are kept.
    pub fn look_from(cell: (i32, i32)) {
        *VIEW.lock().unwrap() = Some(cell);
    }

    /// The skylines of a continent, worked out on this thread if no one has yet, or
    /// waiting for whoever is. For the far-view thread and the thread that made the
    /// continent; pool jobs ask with [`Skylines::soon`].
    pub fn get(c: &Continent) -> Arc<Skylines> {
        let slot = Self::slot(c);
        slot.claimed.store(true, Ordering::Relaxed);
        slot.done.get_or_init(|| Arc::new(Self::of(c))).clone()
    }

    /// The skylines of a continent if they are ready. If not, None at once, and they are
    /// worked out on a thread of their own if no one has started on them.
    pub fn soon(c: &Arc<Continent>) -> Option<Arc<Skylines>> {
        let slot = Self::slot(c);
        if let Some(s) = slot.done.get() {
            return Some(s.clone());
        }
        if !slot.claimed.swap(true, Ordering::Relaxed) {
            let (c, s) = (c.clone(), slot.clone());
            let started = std::thread::Builder::new().name("skylines".into()).spawn(move || {
                s.done.get_or_init(|| Arc::new(Self::of(&c)));
            });
            if started.is_err() {
                // Whoever asks next tries again.
                slot.claimed.store(false, Ordering::Relaxed);
            }
        }
        None
    }

    /// For [`GameWorld::after_made`]: works out a continent's skylines straight after it
    /// is made, on the thread that made it, which holds nothing then. Except on the main
    /// thread, which must not stop for them, and hands them to a thread of their own.
    fn after_made(c: &Arc<Continent>) {
        if std::thread::current().name() == Some("main") {
            Self::soon(c);
        } else {
            Self::get(c);
        }
    }

    /// The skylines at a place on the continent, blended between the grid's points.
    pub fn blended(&self, lx: f32, lz: f32) -> [u8; 2] {
        let top = self.size as f32 - 1.001;
        let (x, z) = ((lx / self.cell).clamp(0.0, top), (lz / self.cell).clamp(0.0, top));
        let (i, j) = (x as usize, z as usize);
        let (fx, fz) = (x - i as f32, z - j as f32);
        [0, 1].map(|k| {
            let at = |i: usize, j: usize| self.angles[j * self.size + i][k] as f32;
            let row = |j: usize| at(i, j) * (1.0 - fx) + at(i + 1, j) * fx;
            (row(j) * (1.0 - fz) + row(j + 1) * fz).round() as u8
        })
    }

    /// The skylines at a place on the continent (voxels from its origin corner).
    pub fn at(&self, lx: f32, lz: f32) -> [u8; 2] {
        let i = ((lx / self.cell) as usize).min(self.size - 1);
        let j = ((lz / self.cell) as usize).min(self.size - 1);
        self.angles[j * self.size + i]
    }
}

pub const FLAG_WATER: u8 = 128;
pub const FLAG_RIBBON: u8 = 255;

struct TileMesh {
    verts: Vec<FarVertex>,
    lo: Vec3,
    hi: Vec3,
}

struct CellMesh {
    cell: (i32, i32),
    stride: usize,
    tiles: Vec<TileMesh>,
    /// River ribbons as quads (4 vertices each).
    rivers: Vec<FarVertex>,
    /// The year and season its fields were coloured for (#207).
    when: When,
}

pub struct FarTile {
    pub buffer: wgpu::Buffer,
    /// Bounds relative to the continent origin.
    pub lo: Vec3,
    pub hi: Vec3,
}

pub struct FarCell {
    pub cell: (i32, i32),
    pub stride: usize,
    pub tiles: Vec<FarTile>,
    pub rivers: Option<(wgpu::Buffer, u32)>,
}

impl FarCell {
    /// Position of the continent's origin corner relative to the camera.
    pub fn offset(&self, camera: DVec3) -> Vec3 {
        let origin = DVec3::new(
            self.cell.0 as f64 * MACRO_SIZE as f64,
            0.0,
            self.cell.1 as f64 * MACRO_SIZE as f64,
        );
        (origin - camera).as_vec3()
    }
}

pub struct FarView {
    cells: HashMap<(i32, i32), FarCell>,
    /// A continent that has arrived and is partly on the GPU. Its tiles go up a frame's
    /// budget at a time, and it replaces what is drawn of it only once all of them are
    /// there, so it never shows in pieces. Whatever was drawn of it before stays until
    /// then.
    staged: Option<Staged>,
    /// Buffers of continents no longer drawn, still to be freed.
    retired: Vec<wgpu::Buffer>,
    /// The continents asked for and not yet arrived, each with the finest detail asked.
    pending: HashMap<(i32, i32), usize>,
    requests: Sender<((i32, i32), usize)>,
    results: Receiver<Arrival>,
    /// Where the camera is, for the far-view thread to choose what to make next by.
    view: Arc<Mutex<DVec3>>,
    /// Index buffers for a tile grid at each stride: (stride, buffer, index count).
    grids: Vec<(usize, wgpu::Buffer, u32)>,
    world: Arc<GameWorld>,
    /// The year and season the fields are drawn for (#207): when the world's turns, each
    /// continent drawn with fields on it is made again.
    when: When,
}

/// What the far-view thread sends back for a continent asked for.
enum Arrival {
    Built(CellMesh),
    /// Not made, the camera having left it behind before it was started on.
    Dropped((i32, i32), usize),
}

/// The continents asked of the far-view thread and not yet started on, each once, with
/// the finest detail asked for it.
#[derive(Default)]
struct Asked(Vec<((i32, i32), usize)>);

impl Asked {
    fn add(&mut self, cell: (i32, i32), stride: usize) {
        match self.0.iter_mut().find(|a| a.0 == cell) {
            Some(a) => a.1 = a.1.min(stride),
            None => self.0.push((cell, stride)),
        }
    }

    /// Takes out what to make next, with the camera at `camera`: of the continents still
    /// around it, the nearest of those already simulated (`ready`), or if none is, the
    /// nearest. Those the camera has left behind are taken out too, and returned to be
    /// told as dropped.
    ///
    /// Taken in the order asked, the land the camera wanted now waited behind the
    /// simulation of every continent asked for before it, seconds each and far longer on
    /// a busy machine, many of them left behind by then; and the far trees, made as soon
    /// as their continent is, stood over nothing all that time.
    fn next(&mut self, camera: DVec3, ready: impl Fn((i32, i32)) -> bool) -> (Vec<((i32, i32), usize)>, Option<((i32, i32), usize)>) {
        let here = cell_of(camera);
        let near = |c: (i32, i32)| (c.0 - here.0).abs() <= 1 && (c.1 - here.1).abs() <= 1;
        let (kept, dropped): (Vec<_>, Vec<_>) = self.0.drain(..).partition(|a| near(a.0));
        self.0 = kept;
        // How far the camera is from a continent's square, in voxels: 0 on it.
        let away = |c: (i32, i32)| {
            let size = MACRO_SIZE as f64;
            let gap = |p: f64, lo: f64| (lo - p).max(p - (lo + size)).max(0.0);
            gap(camera.x, c.0 as f64 * size).hypot(gap(camera.z, c.1 as f64 * size))
        };
        let order = |a: &((i32, i32), usize), b: &((i32, i32), usize)| away(a.0).total_cmp(&away(b.0)).then(a.1.cmp(&b.1));
        let best = (0..self.0.len())
            .filter(|&i| ready(self.0[i].0))
            .min_by(|&i, &j| order(&self.0[i], &self.0[j]))
            .or_else(|| (0..self.0.len()).min_by(|&i, &j| order(&self.0[i], &self.0[j])));
        (dropped, best.map(|i| self.0.remove(i)))
    }
}

/// A continent's mesh on its way to the GPU, with the tiles uploaded so far.
struct Staged {
    mesh: CellMesh,
    tiles: Vec<FarTile>,
    /// How many tiles of `mesh` have been dealt with: uploaded, or left out for lack of
    /// room on the GPU.
    done: usize,
}

/// How long a frame may spend uploading far tiles, in milliseconds. At least one tile
/// goes up each frame whatever the budget, so a continent always gets there. A whole
/// continent at full detail measured up to 7 ms of uploads, so it takes a few frames.
const UPLOAD_MS: f32 = 1.0;

/// Detail for the continent the camera is on, and for its neighbours.
const NEAR_STRIDE: usize = 1;
const FAR_STRIDE: usize = 4;

impl FarView {
    pub fn new(gpu: &Gpu, world: Arc<GameWorld>) -> Self {
        let (requests, request_rx) = channel::<((i32, i32), usize)>();
        let (result_tx, results) = channel();
        let view = Arc::new(Mutex::new(DVec3::ZERO));
        let camera = view.clone();
        // Each continent's skylines are worked out by the thread that made it, as soon
        // as it has, so the workers building its columns seldom have to wait for them.
        world.after_made(Skylines::after_made);
        let (owner, when) = (world.clone(), world.now());
        // One background thread: simulating a continent takes seconds and must never
        // stall rendering or compete with chunk generation for every core.
        std::thread::Builder::new()
            .name("far-view".into())
            .spawn(move || {
                crate::threads::background();
                let canopies = canopies(&world);
                let mut asked = Asked::default();
                loop {
                    // Every ask waiting is gathered first, so that the most pressing is
                    // chosen from all of them; with none, the thread waits for one.
                    if asked.0.is_empty() {
                        let Ok((cell, stride)) = request_rx.recv() else { break };
                        asked.add(cell, stride);
                    }
                    while let Ok((cell, stride)) = request_rx.try_recv() {
                        asked.add(cell, stride);
                    }
                    let camera = *camera.lock().expect("far view camera poisoned");
                    let (dropped, next) = asked.next(camera, |c| world.continent_if_ready(c.0, c.1).is_some());
                    let mut sent = dropped.into_iter().all(|(cell, stride)| result_tx.send(Arrival::Dropped(cell, stride)).is_ok());
                    if let Some((cell, stride)) = next {
                        let continent = world.continent(cell.0, cell.1);
                        let when = world.now();
                        sent &= result_tx.send(Arrival::Built(build_cell(&continent, world.materials(), &canopies, cell, stride, when))).is_ok();
                    }
                    if !sent {
                        break;
                    }
                }
            })
            .expect("far-view thread");
        let grids = [NEAR_STRIDE, FAR_STRIDE]
            .into_iter()
            .map(|stride| {
                let indices = grid_indices(TILE / stride);
                let buffer = gpu
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("far grid indices"),
                        contents: bytemuck::cast_slice(&indices),
                        usage: wgpu::BufferUsages::INDEX,
                    });
                (stride, buffer, indices.len() as u32)
            })
            .collect();
        Self {
            cells: HashMap::new(),
            staged: None,
            retired: Vec::new(),
            pending: HashMap::new(),
            requests,
            results,
            view,
            grids,
            world: owner,
            when,
        }
    }

    pub fn cells(&self) -> impl Iterator<Item = &FarCell> {
        self.cells.values()
    }

    pub fn grid(&self, stride: usize) -> (&wgpu::Buffer, u32) {
        let (_, buffer, count) = self
            .grids
            .iter()
            .find(|g| g.0 == stride)
            .expect("grid for stride");
        (buffer, *count)
    }

    /// A line on what the far view holds, for the log of a timed run.
    pub fn progress(&self) -> String {
        let mut drawn: Vec<String> = self.cells.values().map(|c| format!("{:?}/{}", c.cell, c.stride)).collect();
        drawn.sort();
        let staged = self.staged.as_ref().map(|s| format!("{:?}/{} {} of {}", s.mesh.cell, s.mesh.stride, s.done, s.mesh.tiles.len()));
        format!("far view drew {}, staged {:?}, being made {:?}, {} buffers to free", drawn.join(" "), staged, self.pending, self.retired.len())
    }

    /// True once the camera's own continent is ready to draw.
    pub fn ready(&self, camera: DVec3) -> bool {
        self.cells.contains_key(&cell_of(camera))
    }

    /// Call once per frame. Asks for the continents around the camera, and uploads those
    /// that have arrived for about [`UPLOAD_MS`] at most.
    pub fn update(&mut self, gpu: &Gpu, camera: DVec3) {
        self.update_within(gpu, camera, UPLOAD_MS);
    }

    fn update_within(&mut self, gpu: &Gpu, camera: DVec3, upload_ms: f32) {
        let here = cell_of(camera);
        Skylines::look_from(here);
        *self.view.lock().expect("far view camera poisoned") = camera;
        // The camera's continent in full detail first, then the ring around it.
        let mut wanted = vec![(here, NEAR_STRIDE)];
        for dz in -1..=1 {
            for dx in -1..=1 {
                if (dx, dz) != (0, 0) {
                    wanted.push(((here.0 + dx, here.1 + dz), FAR_STRIDE));
                }
            }
        }
        for &(cell, stride) in &wanted {
            // A continent already held in finer detail is kept as it is.
            let have = self.cells.get(&cell).is_some_and(|c| c.stride <= stride);
            // One asked for in coarser detail than now wanted (the camera has come onto
            // it) is asked for again: the far-view thread makes the finer if it has not
            // started on the coarser.
            let asked = self.pending.get(&cell).is_some_and(|s| *s <= stride);
            if !have && !asked {
                self.pending.insert(cell, stride);
                let _ = self.requests.send((cell, stride));
            }
        }
        let gone: Vec<(i32, i32)> = self.cells.keys().filter(|c| (c.0 - here.0).abs() > 1 || (c.1 - here.1).abs() > 1).copied().collect();
        for c in gone {
            let old = self.cells.remove(&c);
            self.retire(old);
        }
        // The season or the year has turned (#207): the far fields carry other crops, so
        // each continent drawn with fields on it is made again at the detail it is drawn
        // in, and shown as it was until it is. Every season of year 0 is the same.
        let now = self.world.now();
        if now != self.when {
            self.when = now;
            for (&cell, drawn) in &self.cells {
                if self.pending.get(&cell).is_none_or(|&s| s > drawn.stride) && farmed(&self.world, cell) {
                    self.pending.insert(cell, drawn.stride);
                    let _ = self.requests.send((cell, drawn.stride));
                }
            }
        }

        let near = |cell: (i32, i32)| (cell.0 - here.0).abs() <= 1 && (cell.1 - here.1).abs() <= 1;
        // A tile the GPU has no room for is simply not drawn.
        let upload = |verts: &[FarVertex], label| gpu.try_vertex_buffer(label, bytemuck::cast_slice(verts));
        let began = Instant::now();
        let mut first = true;
        loop {
            let staged = match &mut self.staged {
                Some(staged) => staged,
                None => {
                    let mesh = match self.results.try_recv() {
                        Ok(Arrival::Built(mesh)) => mesh,
                        Ok(Arrival::Dropped(cell, stride)) => {
                            settle(&mut self.pending, cell, stride);
                            continue;
                        }
                        Err(_) => break,
                    };
                    self.staged.insert(Staged { tiles: Vec::with_capacity(mesh.tiles.len()), mesh, done: 0 })
                }
            };
            // The camera has left it behind while it was being made or uploaded, or it is
            // drawn in finer detail already.
            let finer = self.cells.get(&staged.mesh.cell).is_some_and(|c| c.stride < staged.mesh.stride);
            if !near(staged.mesh.cell) || finer {
                settle(&mut self.pending, staged.mesh.cell, staged.mesh.stride);
                self.staged = None;
                continue;
            }
            while staged.done < staged.mesh.tiles.len() && (first || began.elapsed().as_secs_f32() * 1000.0 < upload_ms) {
                first = false;
                let t = &mut staged.mesh.tiles[staged.done];
                staged.done += 1;
                // Each tile's vertices are freed as it goes up, within the budget: a
                // whole continent's freed at once took a few milliseconds.
                let verts = std::mem::take(&mut t.verts);
                let up = upload(&verts, "far tile");
                drop(verts);
                if let Some(buffer) = up {
                    staged.tiles.push(FarTile { buffer, lo: t.lo, hi: t.hi });
                }
            }
            if staged.done < staged.mesh.tiles.len() {
                break;
            }
            let Some(Staged { mesh, tiles, .. }) = self.staged.take() else { break };
            settle(&mut self.pending, mesh.cell, mesh.stride);
            // Made before the season turned: drawn, and made again.
            if mesh.when != self.when && !self.pending.contains_key(&mesh.cell) && farmed(&self.world, mesh.cell) {
                self.pending.insert(mesh.cell, mesh.stride);
                let _ = self.requests.send((mesh.cell, mesh.stride));
            }
            let rivers = if mesh.rivers.is_empty() {
                None
            } else {
                upload(&mesh.rivers, "far rivers").map(|b| (b, mesh.rivers.len() as u32 / 4))
            };
            let old = self.cells.insert(
                mesh.cell,
                FarCell {
                    cell: mesh.cell,
                    stride: mesh.stride,
                    tiles,
                    rivers,
                },
            );
            self.retire(old);
        }
        // The hundreds of buffers of a continent replaced or left behind are let go a
        // frame's budget at a time too, rather than all in the frame it goes.
        while began.elapsed().as_secs_f32() * 1000.0 < upload_ms {
            let Some(buffer) = self.retired.pop() else { break };
            drop(buffer);
        }
    }

    /// Puts the buffers of a continent no longer drawn aside, to be freed a few at a
    /// time.
    fn retire(&mut self, cell: Option<FarCell>) {
        if let Some(cell) = cell {
            self.retired.extend(cell.tiles.into_iter().map(|t| t.buffer));
            self.retired.extend(cell.rivers.map(|r| r.0));
        }
    }

    /// Blocks until the camera's continent is drawable (for screenshots). If nothing
    /// arrives for [`crate::terrain::wait_limit`], says what was still being made and
    /// exits the process with a failure, rather than hang.
    pub fn wait_ready(&mut self, gpu: &Gpu, camera: DVec3) {
        let (began, limit) = (Instant::now(), crate::terrain::wait_limit());
        let mut progress = (Instant::now(), self.cells.len());
        log::info!("stage: waiting for the far view of continent {:?}", cell_of(camera));
        while !self.ready(camera) {
            self.update_within(gpu, camera, f32::INFINITY);
            if self.cells.len() != progress.1 {
                progress = (Instant::now(), self.cells.len());
            }
            if progress.0.elapsed() > limit {
                log::error!(
                    "hung: the far view of continent {:?} not ready after {:.0} s with nothing arriving for {:.0} s; {} continents drawn, still being made: {:?}",
                    cell_of(camera),
                    began.elapsed().as_secs_f32(),
                    progress.0.elapsed().as_secs_f32(),
                    self.cells.len(),
                    self.pending
                );
                std::process::exit(3);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        log::info!("stage: far view ready after {:.2} s", began.elapsed().as_secs_f32());
    }
}

/// Takes a continent out of those asked for once what has arrived of it, or been
/// dropped, is the finest detail asked for: one asked for again in finer detail stays.
fn settle(pending: &mut HashMap<(i32, i32), usize>, cell: (i32, i32), stride: usize) {
    if pending.get(&cell) == Some(&stride) {
        pending.remove(&cell);
    }
}

/// Whether continent `cell` has fields, whose crops turn with the seasons (#207). Never
/// blocks: a continent not simulated yet has none drawn either.
fn farmed(world: &GameWorld, cell: (i32, i32)) -> bool {
    world.continent_if_ready(cell.0, cell.1).is_some_and(|c| !c.land.farms.is_empty())
}

fn cell_of(camera: DVec3) -> (i32, i32) {
    (
        (camera.x / MACRO_SIZE as f64).floor() as i32,
        (camera.z / MACRO_SIZE as f64).floor() as i32,
    )
}

/// Triangle indices for an `n` x `n` grid of quads over `(n + 1)^2` vertices.
fn grid_indices(n: usize) -> Vec<u16> {
    let row = (n + 1) as u16;
    let mut out = Vec::with_capacity(n * n * 6);
    for z in 0..n as u16 {
        for x in 0..n as u16 {
            let a = z * row + x;
            let (b, c, d) = (a + 1, a + row, a + row + 1);
            // Counter-clockwise seen from above (+Y), with z increasing down the rows.
            out.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    out
}

/// How a biome's forest looks from beyond the tree models.
#[derive(Clone, Copy)]
struct Canopy {
    /// What its trees' crowns show from above (sRGB).
    color: [f32; 3],
    /// How much of the ground they hide, 0..1.
    cover: f32,
    /// How tall they stand, in voxels.
    height: f32,
    /// Their crowns' plant kind (voxel::plant), 0 if unknown.
    plant: u8,
    /// How much of the forest keeps its leaves through the winter, 0..1: the hollies,
    /// yews and pines among its broadleaves (`features::winter_green_share`).
    winter_green: f32,
}

/// A canopy's plant kind with, above its three bits, the share of the forest that stays
/// green in winter in 31sts: what far.wgsl reads back from a vertex's `place.y`.
fn pack_plant(plant: u8, winter_green: f32) -> u8 {
    (plant & 7) | (((winter_green.clamp(0.0, 1.0) * 31.0).round() as u8) << 3)
}

/// Each biome's forest, indexed by biome, or None where too few trees grow to tell.
fn canopies(world: &GameWorld) -> Vec<Option<Canopy>> {
    let (lib, mats) = (world.library(), world.materials());
    Biome::ALL
        .iter()
        .map(|&biome| {
            let (leaves, cover, height) = features::canopy(biome, mats)?;
            // The crown the far tree models give the biome's commonest tree, so its
            // forest keeps its colour where they hand over to the canopy: willow in a
            // mangrove, palms on a volcano.
            let top = features::canopy_tree(biome)
                .and_then(|species| trees::impostor(lib.get(species, 0), mats).into_iter().max_by_key(|b| b.hi.y))
                .map(|b| mats.get(b.voxel));
            let crown = top.map_or(leaves, |m| m.color);
            Some(Canopy {
                color: crown.map(f32::from),
                // Palms and pines stay green through the winter, whatever their colour.
                plant: top.map_or(0, |m| m.plant),
                winter_green: features::winter_green_share(biome),
                cover,
                height: height * OLD_VOXEL,
            })
        })
        .collect()
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * t)
}

/// What the far surface shows of the land at a cell.
struct Look {
    /// Its ground (sRGB).
    ground: [f32; 3],
    /// The ground under its forest's canopy, as seen from beyond the tree models.
    far: [f32; 3],
    /// The plant kind of the canopy (voxel::plant), 0 with none, and how much of it stays
    /// green in winter, as [`pack_plant`] puts them in one byte.
    plant: u8,
    /// How high that canopy stands above the ground, in voxels.
    lift: f32,
    /// The wash of the flowers that lie over it, how much of the ground's colour it takes,
    /// and how much of that shows under the canopy (see [`FarVertex::wash`]).
    wash: (u8, f32, f32),
}

/// The land at a cell as the far surface draws it. The ground is the one the voxel world
/// gives it (see `world::surface_materials`), each biome with its own soil and rock, the
/// voxel-to-voxel speckle and ragged edges between them averaged out as distance does.
fn look(c: &Continent, mats: &voxel::Materials, canopies: &[Option<Canopy>], i: usize, slope: f32, when: When) -> Look {
    let color = |v: voxel::Voxel| mats.get(v).color.map(f32::from);
    if c.ocean[i] {
        // Under the sea plane; only seen through gaps, so keep it sandy.
        let sand = color(mat::SAND);
        return Look { ground: sand, far: sand, plant: 0, lift: 0.0, wash: (0, 0.0, 0.0) };
    }
    let biome = Biome::from_u8(c.biome[i]);
    // Sand as far in from the shore as the voxel world's beach reaches, which the
    // triangles shade into the land around it.
    let cs = c.cell_size();
    let centre = glam::Vec2::new((i % c.size) as f32 + 0.5, (i / c.size) as f32 + 0.5) * cs;
    let sand = surface::beach_share(c, centre, c.height[i] * OLD_VOXEL);
    let beach = sand > 0.5;
    let mut ground = color(mat::ground_cover(biome));
    // Drifts of flowers and beds of heather colour the land they lie over, as the ground
    // under each plant is coloured nearer to (see `wash` in terrain.wgsl). The shader
    // lays the colour on, since it changes with the date: purple while the heather is
    // out, rust when it is over.
    let (wx, wz) = (c.desc.mx * MACRO_SIZE + centre.x as i32, c.desc.mz * MACRO_SIZE + centre.y as i32);
    let (wash, mut washed) = features::plants::far_wash(biome, wx, wz, c.desc.world_seed).unwrap_or((0, 0.0));
    ground = mix(ground, color(mat::SAND), sand);
    // The voxel world's snow line wanders by 0.015 either way of 0.10.
    let snow = ((0.115 - c.temperature[i]) / 0.03).clamp(0.0, 1.0) * (1.0 - sand);
    ground = mix(ground, color(mat::SNOW), snow);
    // Ground too steep to hold soil is bare rock, a quarter of it the darker kind. The
    // grid's slopes are gentler than the voxels', hence the steeper threshold.
    let (light, dark) = mat::bedrock(biome);
    let rocky = smoothstep(1.3, 1.9, slope);
    ground = mix(ground, mix(color(light), color(dark), 0.25), rocky);
    // Nothing flowers on sand, snow or rock.
    washed *= (1.0 - sand) * (1.0 - snow) * (1.0 - rocky);
    // About a settlement, the fields: plough and crops in their seasons, each the crop it
    // carries in the year and season seen (#207), and the woods cleared from them (see
    // worldgen::settlements::farmland).
    let open = (1.0 - sand) * (1.0 - snow) * (1.0 - rocky);
    let mut cleared = 0.0;
    let mut wash = wash;
    if let Some((soil, crop, managed)) = worldgen::settlements::farmland::far_cell(c, centre.x as i32, centre.y as i32, when) {
        let share = smoothstep(0.1, 0.45, managed) * open;
        if let Some(soil) = soil {
            ground = mix(ground, color(soil), share * 0.85);
        }
        if let Some(crop) = crop {
            (wash, washed) = (crop, share * 0.8);
        }
        cleared = share;
    }

    // Trees take no root on rock, on the beach, or where it is too cold.
    let Some(canopy) = canopies[biome as usize].filter(|_| !beach) else {
        return Look { ground, far: ground, plant: 0, lift: 0.0, wash: (wash, washed, 1.0) };
    };
    let grows = (1.0 - rocky) * smoothstep(0.125, 0.14, c.temperature[i]) * (1.0 - 0.8 * cleared);
    Look {
        ground,
        far: mix(ground, canopy.color, canopy.cover * 0.85 * grows),
        plant: pack_plant(canopy.plant, canopy.winter_green),
        lift: canopy.cover * canopy.height * 0.6 * grows,
        wash: (wash, washed, 1.0 - canopy.cover * 0.85 * grows),
    }
}

/// Water depth, in voxels, at which the far surface's wetness saturates (see [`wetness`]).
const WET_RANGE: f32 = 32.0;

/// The standing water at the far-surface vertex on cell `(x, z)`, its neighbours
/// `stride` cells away: the highest lake level among them, in voxels, as the voxel world
/// floods a column to the highest lake at the corners of its grid square; and how far
/// that level stands above the vertex's ground, or `-WET_RANGE` with no lake.
///
/// Both vary linearly across the surface's triangles, so where they meet, the shore,
/// is a smooth line close to the voxel world's, not a cell of the grid.
fn wetness(c: &Continent, x: usize, z: usize, stride: usize) -> (Option<f32>, f32) {
    let (s, max) = (stride as isize, c.size as isize - 1);
    let mut level: Option<f32> = None;
    for dz in [-s, 0, s] {
        for dx in [-s, 0, s] {
            let (nx, nz) = ((x as isize + dx).clamp(0, max) as usize, (z as isize + dz).clamp(0, max) as usize);
            let l = c.lake[nz * c.size + nx];
            if l != NO_LAKE {
                let k = surface::pool_level(c.desc.seed, l) as f32;
                level = Some(level.map_or(k, |m| m.max(k)));
            }
        }
    }
    let ground = c.height[z * c.size + x] * OLD_VOXEL;
    // Water fills the voxels above the ground up to one below the level.
    let w = level.map_or(-WET_RANGE, |k| (k - 1.0 - ground).clamp(-WET_RANGE, WET_RANGE));
    (level, w)
}

/// Whether the far surface shows water at continent-local voxel `(lx, lz)`: a lake, or
/// land it lays below the sea. The true shore can lie a little inside either, so things
/// standing there are better left out.
pub fn shows_water(c: &Continent, lx: f32, lz: f32) -> bool {
    let cs = c.cell_size();
    let max = (c.size - 1) as f32;
    let (fx, fz) = ((lx / cs - 0.5).clamp(0.0, max), (lz / cs - 0.5).clamp(0.0, max));
    let (x0, z0) = ((fx as usize).min(c.size - 2), (fz as usize).min(c.size - 2));
    let (tx, tz) = (fx - x0 as f32, fz - z0 as f32);
    let (mut wet, mut height, mut ocean) = (0.0, 0.0, false);
    for (dx, dz, w) in [(0, 0, (1.0 - tx) * (1.0 - tz)), (1, 0, tx * (1.0 - tz)), (0, 1, (1.0 - tx) * tz), (1, 1, tx * tz)] {
        let (x, z) = (x0 + dx, z0 + dz);
        let i = z * c.size + x;
        ocean |= c.ocean[i];
        let (level, depth) = wetness(c, x, z, 1);
        wet += w * depth;
        height += w * vertex_y(c.height[i] * OLD_VOXEL, level, depth);
    }
    // The shader's triangles and this square differ a little: half a voxel's grace.
    wet > -0.5 || (ocean && height < SEA_LEVEL + 1.0)
}

/// The biome (as u8) of the continent's grid cell holding continent-local voxel `(lx, lz)`.
pub fn biome_in(c: &Continent, lx: f32, lz: f32) -> u8 {
    let cs = c.cell_size();
    let cell = |v: f32| ((v / cs).max(0.0) as usize).min(c.size - 1);
    c.biome[cell(lz) * c.size + cell(lx)]
}

/// The biome (as u8) at world voxel column `(x, z)`, exactly as the voxel terrain has
/// it, or `Biome::Ocean`, which stands for wherever the camera is, while its continent
/// has not been simulated. Never blocks.
/// The skylines toward sunrise and sunset at a world position, in voxels; level where
/// the continent is not made yet. None while it is made but its skylines are still being
/// worked out (they are then started on if no one has): ask again later. Never blocks.
pub fn skylines_at(world: &GameWorld, x: i32, z: i32) -> Option<[u8; 2]> {
    let (mx, mz) = worldgen::macro_layout::cell_of(x, z);
    let Some(c) = world.continent_if_ready(mx, mz) else { return Some([0; 2]) };
    Some(Skylines::soon(&c)?.blended((x - mx * MACRO_SIZE) as f32, (z - mz * MACRO_SIZE) as f32))
}

/// The lie of the land at a world position, in voxels: the x and z of the unit normal of
/// the continent's ground, taken over a grid cell each way (about 50 m), each as a signed
/// byte. Level where the continent is not made yet, and over the sea.
pub fn slope_at(world: &GameWorld, x: i32, z: i32) -> [i8; 2] {
    let (mx, mz) = worldgen::macro_layout::cell_of(x, z);
    world.continent_if_ready(mx, mz).map_or([0; 2], |c| {
        let (size, cell) = (c.size, c.cell_size());
        let (gx, gz) = ((x - mx * MACRO_SIZE) as f32 / cell, (z - mz * MACRO_SIZE) as f32 / cell);
        // Ground or sea, in voxels, between the grid's points.
        let at = |x: f32, z: f32| {
            let (x, z) = (x.clamp(0.0, size as f32 - 1.001), z.clamp(0.0, size as f32 - 1.001));
            let (i, j) = (x as usize, z as usize);
            let (fx, fz) = (x - i as f32, z - j as f32);
            let h = |i: usize, j: usize| (c.height[j * size + i] * OLD_VOXEL).max(0.0);
            let row = |j: usize| h(i, j) * (1.0 - fx) + h(i + 1, j) * fx;
            row(j) * (1.0 - fz) + row(j + 1) * fz
        };
        let n = glam::Vec3::new(at(gx - 1.0, gz) - at(gx + 1.0, gz), 2.0 * cell, at(gx, gz - 1.0) - at(gx, gz + 1.0)).normalize();
        [(n.x * 127.0).round() as i8, (n.z * 127.0).round() as i8]
    })
}

pub fn biome_at(world: &GameWorld, x: i32, z: i32) -> u8 {
    let (mx, mz) = worldgen::macro_layout::cell_of(x, z);
    world.continent_if_ready(mx, mz).map_or(Biome::Ocean as u8, |c| {
        worldgen::surface::blend::biome_at(&c, world.seed(), x - mx * MACRO_SIZE, z - mz * MACRO_SIZE) as u8
    })
}

/// The far surface's height at a vertex whose ground is at `h`, beside water at `level`
/// standing `depth` above it: the water's surface where it is wet, else the land, which
/// is never sunk from above the water to below it.
fn vertex_y(h: f32, level: Option<f32>, depth: f32) -> f32 {
    match level {
        Some(k) if depth > 0.0 => k - 0.125,
        Some(k) => land_y(h).max(h.min(k - 0.125)),
        None => land_y(h),
    }
}

/// Where the sea plane lies.
pub const SEA_LEVEL: f32 = -0.125;

/// The far surface's height for land whose ground is at `h`: sunk a little, but never
/// from above the sea to below it, or low-lying land would be drowned by the sea plane.
fn land_y(h: f32) -> f32 {
    (h - SINK).max(h.min(SEA_LEVEL + 0.2))
}

/// The colour of each grid cell of `c` on a map (sRGB): the land as the far surface has
/// it, its forests seen as a canopy, under its lakes and rivers too; the sea plain.
pub fn map_land(world: &GameWorld, c: &Continent) -> Vec<[f32; 3]> {
    let canopies = canopies(world);
    let mats = world.materials();
    let when = world.now();
    let size = c.size;
    let cs = c.cell_size();
    let sea = [32.0, 78.0, 128.0];
    (0..size * size)
        .map(|i| {
            let (x, z) = (i % size, i / size);
            if c.ocean[i] {
                return sea;
            }
            let h = |x: usize, z: usize| c.height[z.min(size - 1) * size + x.min(size - 1)] * OLD_VOXEL;
            let gx = (h(x + 1, z) - h(x.saturating_sub(1), z)) / (2.0 * cs);
            let gz = (h(x, z + 1) - h(x, z.saturating_sub(1))) / (2.0 * cs);
            look(c, mats, &canopies, i, gx.hypot(gz), when).far
        })
        .collect()
}

/// Whether the far surface floods grid cell `(x, z)` with a lake.
pub fn lake_at(c: &Continent, x: usize, z: usize) -> bool {
    wetness(c, x, z, 1).1 > 0.0
}

/// Whether grid cell `i` carries a river drawn as a ribbon.
fn ribbon_at(c: &Continent, i: usize) -> bool {
    !c.ocean[i] && Continent::river_width(c.discharge[i]) * OLD_VOXEL >= MIN_RIBBON_WIDTH
}

/// The far surface's height (before the shader's forest lift) at continent-local voxel
/// `(lx, lz)`, from its vertices `stride` cells apart, split into triangles as
/// [`grid_indices`] splits them.
fn far_surface_y(c: &Continent, stride: usize, lx: f32, lz: f32) -> f32 {
    let cs = c.cell_size();
    let size = c.size;
    let vy = |x: usize, z: usize| {
        let (x, z) = (x.min(size - 1), z.min(size - 1));
        let (level, depth) = wetness(c, x, z, stride);
        vertex_y(c.height[z * size + x] * OLD_VOXEL, level, depth)
    };
    let s = stride as f32;
    let u = ((lx / cs - 0.5) / s).max(0.0);
    let v = ((lz / cs - 0.5) / s).max(0.0);
    let (gi, gj) = (u.floor() as usize, v.floor() as usize);
    let (fu, fv) = ((u - gi as f32).min(1.0), (v - gj as f32).min(1.0));
    let (x0, z0, x1, z1) = (gi * stride, gj * stride, (gi + 1) * stride, (gj + 1) * stride);
    let (a, b, cc, d) = (vy(x0, z0), vy(x1, z0), vy(x0, z1), vy(x1, z1));
    // Triangles a-c-b and b-c-d: the diagonal runs from b to c.
    if fu + fv <= 1.0 {
        a + (b - a) * fu + (cc - a) * fv
    } else {
        d + (cc - d) * (1.0 - fu) + (b - d) * (1.0 - fv)
    }
}

/// Grid cells out to the two rings of land a point is compared with in [`hollow`]: about
/// 75 and 150 m, the size of the folds of hills that the eye reads from far off.
const HOLLOW_RINGS: [f32; 2] = [3.0, 6.0];
/// What a hollow byte counts per unit of [`hollow`]'s measure (HOLLOW_SCALE in
/// common.wgsl).
const HOLLOW_SCALE: f32 = 320.0;

/// How far the land at grid point `(gx, gz)` of `c` (cell centres at whole numbers) lies
/// below the land around it, a hollow, over 128, or above it, a ridge, under 128: the
/// rise from it to the mean of the land round it over the distance to that land, in
/// [`HOLLOW_SCALE`]ths. Less of the sky reaches down into a valley than onto a ridge, and
/// that, more than the sun, is what shows the shape of distant hills. It is the same
/// whatever the detail of the mesh it is drawn on, so the far view at any stride and the
/// voxel terrain (see [`hollow_at`]) agree where they meet. The sea counts as level.
fn hollow(c: &Continent, gx: f32, gz: f32) -> u8 {
    let size = c.size;
    let at = |x: f32, z: f32| {
        let (x, z) = (x.clamp(0.0, size as f32 - 1.001), z.clamp(0.0, size as f32 - 1.001));
        let (i, j) = (x as usize, z as usize);
        let (fx, fz) = (x - i as f32, z - j as f32);
        let h = |i: usize, j: usize| (c.height[j * size + i] * OLD_VOXEL).max(0.0);
        let row = |j: usize| h(i, j) * (1.0 - fx) + h(i + 1, j) * fx;
        row(j) * (1.0 - fz) + row(j + 1) * fz
    };
    let here = at(gx, gz);
    let mut sum = 0.0;
    for r in HOLLOW_RINGS {
        let around = [(r, 0.0), (-r, 0.0), (0.0, r), (0.0, -r), (r, r), (-r, r), (r, -r), (-r, -r)];
        let mean = around.iter().map(|&(dx, dz)| at(gx + dx, gz + dz)).sum::<f32>() / 8.0;
        sum += (mean - here) / (r * c.cell_size());
    }
    let k = sum / HOLLOW_RINGS.len() as f32;
    (128.0 + k * HOLLOW_SCALE).round().clamp(1.0, 255.0) as u8
}

/// [`hollow`] at a world position, in voxels: 128, level, where the continent is not made
/// yet.
pub fn hollow_at(world: &GameWorld, x: i32, z: i32) -> u8 {
    let (mx, mz) = worldgen::macro_layout::cell_of(x, z);
    world.continent_if_ready(mx, mz).map_or(128, |c| {
        let cell = c.cell_size();
        // The far view's vertices stand in the middle of their cells.
        hollow(&c, (x - mx * MACRO_SIZE) as f32 / cell - 0.5, (z - mz * MACRO_SIZE) as f32 / cell - 0.5)
    })
}

fn build_cell(c: &Continent, mats: &voxel::Materials, canopies: &[Option<Canopy>], cell: (i32, i32), stride: usize, when: When) -> CellMesh {
    let cs = c.cell_size();
    let size = c.size;
    let tiles_per_side = size / TILE;
    let n = TILE / stride;
    let mut tiles = Vec::new();
    let skylines = Skylines::get(c);

    for tz in 0..tiles_per_side {
        for tx in 0..tiles_per_side {
            // Open sea is covered by the sea plane.
            let any_land = (0..=TILE).any(|dz| {
                (0..=TILE).any(|dx| {
                    let (x, z) = ((tx * TILE + dx).min(size - 1), (tz * TILE + dz).min(size - 1));
                    !c.ocean[z * size + x]
                })
            });
            if !any_land {
                continue;
            }
            let mut verts = Vec::with_capacity((n + 1) * (n + 1));
            let (mut lo, mut hi) = (Vec3::MAX, Vec3::MIN);
            for j in 0..=n {
                for i in 0..=n {
                    let x = (tx * TILE + i * stride).min(size - 1);
                    let z = (tz * TILE + j * stride).min(size - 1);
                    let idx = z * size + x;
                    let h = |x: usize, z: usize| c.height[z.min(size - 1) * size + x.min(size - 1)] * OLD_VOXEL;
                    let gx = (h(x + 1, z) - h(x.saturating_sub(1), z)) / (2.0 * cs);
                    let gz = (h(x, z + 1) - h(x, z.saturating_sub(1))) / (2.0 * cs);
                    let hollow = hollow(c, x as f32, z as f32);
                    let pos = |y: f32| Vec3::new((x as f32 + 0.5) * cs, y, (z as f32 + 0.5) * cs);
                    let (level, depth) = wetness(c, x, z, stride);
                    let mut land = look(c, mats, canopies, idx, gx.hypot(gz), when);
                    if depth > 0.0 {
                        // Nothing grows in the water.
                        land.far = land.ground;
                        land.lift = 0.0;
                        land.wash = (0, 0.0, 0.0);
                    }
                    // The forest comes down to the ground beside water, so the water is
                    // not tilted up to the canopy across the triangles of its shore.
                    let s = stride as isize;
                    let by_water = [(-s, -s), (0, -s), (s, -s), (-s, 0), (s, 0), (-s, s), (0, s), (s, s)].iter().any(|&(dx, dz)| {
                            let max = size as isize - 1;
                            let (nx, nz) = ((x as isize + dx).clamp(0, max) as usize, (z as isize + dz).clamp(0, max) as usize);
                            wetness(c, nx, nz, stride).1 > 0.0
                        });
                    // Still a forest, for the seasons, though it no longer stands tall.
                    let forest = land.lift > 0.0;
                    // Nor does it stand tall over a river, which would be hidden under it.
                    let by_river = || {
                        let max = size as isize - 1;
                        (-s..=s).any(|dz| {
                            (-s..=s).any(|dx| {
                                let (nx, nz) = ((x as isize + dx).clamp(0, max) as usize, (z as isize + dz).clamp(0, max) as usize);
                                ribbon_at(c, nz * size + nx)
                            })
                        })
                    };
                    if by_water || (land.lift > 0.0 && by_river()) {
                        land.lift = 0.0;
                    }
                    // Slight variation from cell to cell, standing in for the texture
                    // voxels have up close.
                    let noise = worldgen::noise::hash2(c.desc.seed ^ 0xfa2, x as i32, z as i32);
                    let f = 0.93 + worldgen::noise::unit(noise) * 0.12;
                    let srgb = |rgb: [f32; 3]| rgb.map(|v| (v * f).round().clamp(0.0, 255.0) as u8);
                    let far = srgb(land.far);
                    // Forested land stands higher by about the height of its canopy. That
                    // is carried to the shader, which raises it only beyond the tree models.
                    let mut flags = (land.lift / LIFT_UNIT).round().min(MAX_LIFT_FLAGS) as u8;
                    if forest {
                        flags = flags.max(1);
                    }
                    let p = pos(vertex_y(h(x, z), level, depth));
                    let wet = (128.0 + depth / WET_RANGE * 127.0).round().clamp(0.0, 255.0) as u8;
                    lo = lo.min(p);
                    hi = hi.max(p + Vec3::Y * land.lift);
                    let pack = |g: f32| ((g / 4.0).clamp(-1.0, 1.0) * 32767.0) as i16;
                    verts.push(FarVertex {
                        pos: p.to_array(),
                        color: srgb(land.ground),
                        flags,
                        grad: [pack(gx), pack(gz)],
                        far: [far[0], far[1], far[2], wet],
                        place: [c.biome[idx], land.plant, skylines.angles[idx][0], skylines.angles[idx][1]],
                        wash: [land.wash.0, (land.wash.1.clamp(0.0, 1.0) * 255.0).round() as u8, (land.wash.2.clamp(0.0, 1.0) * 255.0).round() as u8, hollow],
                    });
                }
            }
            tiles.push(TileMesh { verts, lo, hi });
        }
    }

    // Rivers wide enough to see from afar, as flat ribbons along their curves.
    let mut rivers = Vec::new();
    let water = mats.get(mat::WATER).color;
    let mut pieces: Vec<Piece> = Vec::new();
    for i in 0..size * size {
        if !ribbon_at(c, i) {
            continue;
        }
        pieces.clear();
        surface::add_pieces(c, i, &mut pieces);
        for piece in &pieces {
            let segs = piece.pts.len() - 1;
            // Every other point is plenty at this distance; the last is always kept.
            let mut ks: Vec<usize> = (0..segs).step_by(2).collect();
            ks.push(segs);
            // Consecutive quads share their corners, each point's side square to the
            // curve's tangent there, so bends leave no gaps or overlaps.
            let corners: Vec<[FarVertex; 2]> = ks
                .iter()
                .enumerate()
                .map(|(n, &k)| {
                    let prev = piece.pts[ks[n.saturating_sub(1)]];
                    let next = piece.pts[ks[(n + 1).min(ks.len() - 1)]];
                    let dir = (next - prev).normalize_or_zero();
                    let side = glam::vec2(-dir.y, dir.x);
                    let t = k as f32 / segs as f32;
                    let p = piece.pts[k];
                    // On the far surface as its triangles lie at the centreline, so the
                    // coarse ground never stands over the water and cuts it into dashes.
                    let level = surface::pool_level(c.desc.seed, piece.level(t)) as f32 - 0.125;
                    let y = level.max(far_surface_y(c, stride, p.x, p.y));
                    let hw = piece.half_width(t).max(worldgen::scale::m(1.0));
                    [1.0f32, -1.0].map(|sign| {
                        let q = p + side * (hw * sign);
                        let mut v = FarVertex::water([q.x, y, q.y], water, FLAG_RIBBON);
                        v.place[0] = biome_in(c, p.x, p.y);
                        // The shader widens the ribbon to a pixel along this.
                        let pack = |g: f32| (g.clamp(-1.0, 1.0) * 32767.0) as i16;
                        v.grad = [pack(side.x * sign), pack(side.y * sign)];
                        v
                    })
                })
                .collect();
            for w in corners.windows(2) {
                // Counter-clockwise seen from above.
                rivers.extend_from_slice(&[w[0][0], w[1][0], w[1][1], w[0][1]]);
            }
        }
    }

    CellMesh {
        cell,
        stride,
        tiles,
        rivers,
        when,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::mpsc, time::Duration};
    use worldgen::continent::SimConfig;

    /// Pool jobs asking for a continent's skylines, queued beside more jobs than there
    /// are threads, half of which do nothing and so leave their threads free to steal,
    /// all come back. If the first asker split the work among its pool, it would wait for
    /// the part another thread took by running a queued job, and one that asked for the
    /// skylines again would wait on the asker itself: on the same thread, for ever.
    #[test]
    fn skylines_asked_for_by_pool_jobs_beside_idle_ones_always_come_back() {
        let world = GameWorld::with_config(3, SimConfig { size: 256 });
        let c = world.continent(0, 0);
        let (finished, all_done) = mpsc::channel();
        std::thread::spawn(move || {
            let pool = rayon::ThreadPoolBuilder::new().num_threads(4).build().unwrap();
            for _ in 0..40 {
                Skylines::forget_all();
                let (done, back) = mpsc::channel();
                const JOBS: usize = 64;
                for k in 0..JOBS {
                    let (c, done) = (c.clone(), done.clone());
                    pool.spawn(move || {
                        if k % 2 == 0 {
                            Skylines::get(&c);
                        }
                        let _ = done.send(());
                    });
                }
                for _ in 0..JOBS {
                    back.recv().unwrap();
                }
            }
            let _ = finished.send(());
        });
        all_done.recv_timeout(Duration::from_secs(120)).expect("pool jobs asking for skylines never came back: deadlocked");
    }

    /// Asked for with `soon`, skylines not ready come back as None at once, are worked
    /// out elsewhere, and come back the same as asked for outright.
    #[test]
    fn skylines_asked_for_soon_never_wait_and_come_out_the_same() {
        let world = GameWorld::with_config(4, SimConfig { size: 256 });
        let c = world.continent(0, 0);
        Skylines::forget_all();
        let pool = rayon::ThreadPoolBuilder::new().num_threads(2).build().unwrap();
        let began = Instant::now();
        let soon = loop {
            let (c2, (tx, rx)) = (c.clone(), mpsc::channel());
            pool.spawn(move || {
                let t = Instant::now();
                let s = Skylines::soon(&c2);
                let _ = tx.send((s, t.elapsed()));
            });
            let (s, took) = rx.recv_timeout(Duration::from_secs(60)).expect("asking soon never came back");
            assert!(took < Duration::from_millis(50), "asking soon took {took:?}");
            if let Some(s) = s {
                break s;
            }
            assert!(began.elapsed() < Duration::from_secs(60), "the skylines never came");
            std::thread::sleep(Duration::from_millis(2));
        };
        Skylines::forget_all();
        assert_eq!(soon.angles, Skylines::get(&c).angles);
    }

    /// The skylines forgotten are those of continents asked for longest ago, never of one
    /// in view or around it.
    #[test]
    fn the_skylines_kept_are_those_in_view_and_lately_used() {
        let mut kept = Vec::new();
        let now = 1_000_000u64;
        for k in 0..SKYLINES_KEPT as i32 {
            let s = Arc::new(SkySlot::default());
            let _ = s.done.set(Arc::new(Skylines { size: 1, cell: 1.0, angles: vec![[0; 2]] }));
            // All asked for long ago, the first longest ago.
            s.used.store(now.saturating_sub(SKYLINES_IN_USE + 1000 - k as u64), Ordering::Relaxed);
            kept.push(((99, k, 0), s));
        }
        // The two asked for longest ago are in view, and the next is in use.
        kept[2].1.used.store(now, Ordering::Relaxed);
        Skylines::forget_one(&mut kept, Some((0, 0)), now);
        let left: Vec<i32> = kept.iter().map(|((_, mx, _), _)| *mx).collect();
        assert!(!left.contains(&3) && left.len() == SKYLINES_KEPT - 1, "{left:?}");
        // None is forgotten while all are in use.
        for (_, s) in &kept {
            s.used.store(now, Ordering::Relaxed);
        }
        Skylines::forget_one(&mut kept, None, now + 1);
        assert_eq!(kept.len(), SKYLINES_KEPT - 1);
    }

    /// Of the continents asked for, the camera's own comes first, then the nearest that
    /// is ready, not the first asked; those left behind are dropped, and the finest
    /// detail asked for a continent is the one made.
    #[test]
    fn the_far_view_makes_what_the_camera_needs_first() {
        let size = MACRO_SIZE as f64;
        // Near the east edge of continent (0, 0), as the camera was crossing to (1, 0).
        let camera = DVec3::new(size * 0.97, 0.0, size * 0.5);
        let mut asked = Asked::default();
        // Asked for in the order the camera's earlier continent wanted them.
        for (cell, stride) in [((-1, -1), 4), ((0, -1), 4), ((1, -1), 4), ((-1, 0), 4), ((1, 0), 4), ((0, 0), 4), ((0, 0), 1), ((-2, 0), 4)] {
            asked.add(cell, stride);
        }
        let all = |_: (i32, i32)| true;
        let (dropped, next) = asked.next(camera, all);
        assert_eq!(dropped, vec![((-2, 0), 4)], "the continent left behind is dropped");
        assert_eq!(next, Some(((0, 0), 1)), "the camera's own continent, in full detail");
        let (_, next) = asked.next(camera, all);
        assert_eq!(next, Some(((1, 0), 4)), "then the one it is heading into");
        // A nearer continent not simulated yet waits for one that is.
        let (_, next) = asked.next(camera, |c| c != (1, -1));
        assert!(next.is_some_and(|n| n.0 != (1, -1)), "{next:?}");
        // With none ready, the nearest is made all the same.
        let (_, next) = asked.next(camera, |_| false);
        assert_eq!(next, Some(((1, -1), 4)));
        // Crossing into (1, 0): what is left around (0, 0) only is dropped.
        let camera = DVec3::new(size * 1.02, 0.0, size * 0.5);
        let (dropped, _) = asked.next(camera, all);
        assert!(dropped.iter().all(|(c, _)| c.0 == -1), "{dropped:?}");
    }

    /// A continent stays asked for until what arrives of it is the finest asked for.
    #[test]
    fn a_continent_asked_for_in_finer_detail_stays_asked_for() {
        let mut pending = HashMap::new();
        pending.insert((1, 0), 1);
        settle(&mut pending, (1, 0), 4);
        assert_eq!(pending.get(&(1, 0)), Some(&1));
        settle(&mut pending, (1, 0), 1);
        assert!(pending.is_empty());
    }
}
