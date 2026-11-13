//! Chunk streaming: generates and meshes chunk columns around the camera on worker
//! threads and keeps their GPU buffers.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicI32, Ordering},
        mpsc::{Receiver, Sender, channel},
    },
    time::Instant,
};

use glam::{DVec3, IVec3};
use voxel::{
    CHUNK,
    mesh::{CELLS, ChunkMesh, Falls, Vertex, add_plants, cover_columns, mesh_chunk_under_sky, tag_trees, wash_map},
};
use worldgen::{Cascade, SpringMark, Surf, World, features, scale::m, world::GameWorld};

use crate::{
    arena::{MeshSpan, VertexArena},
    gfx::Gpu,
    lod,
};

pub struct GpuChunk {
    /// Level of detail: this chunk's voxels are `2^level` times the size of real ones.
    pub level: u8,
    /// Chunk coordinates at this level (world voxel position / (32 * 2^level)).
    pub pos: IVec3,
    pub opaque: Option<MeshSpan>,
    pub water: Option<MeshSpan>,
    /// Where each face direction starts within `opaque`, in quads.
    pub faces: [u32; 13],
    /// The biomes (as u8) on a 5 x 5 grid over the column, corners and edges included
    /// (shared with the neighbours), x fastest, a byte each, for the snow lying there
    /// (see `season::Places` and `grid_snow` in ground.wgsl).
    pub biomes: [u32; 8],
    /// The skylines toward sunrise and toward sunset (`farview::Skylines`) at the four
    /// corners of the column, x fastest, so hills shade it beyond the shadow maps.
    pub skylines: [[u8; 2]; 4],
    /// The lie of the land (`farview::slope_at`) at the same four corners, to light it by
    /// once its voxels are too small to tell apart.
    pub slopes: [[i8; 2]; 4],
}

/// Levels of detail, the first being full-size voxels.
pub const LEVELS: usize = 3;

/// A column at some level of detail: (level, x, z), in columns of that level.
type Col = (u8, i32, i32);

/// How far out each level is loaded, in full-size columns from the camera. Each reaches
/// twice as far as the one finer than it, and all the way in underneath it: whenever the
/// finer one is not there yet (at first, or flying faster than it can be built), the
/// coarser one is drawn in its place, rather than nothing at all. Being coarse, the part
/// lying under the finer levels is an eighth of a level's columns.
fn reach(level: u8, radius: i32) -> f32 {
    radius as f32 * step(level)
}

/// How many full-size columns one column of a level spans.
fn step(level: u8) -> f32 {
    (1 << level) as f32
}

/// What a column's chunks carry of the land around them, for lighting and snow: see
/// the fields of the same names on [`GpuChunk`].
#[derive(Clone, Copy, Default)]
struct Marks {
    biomes: [u32; 8],
    skylines: [[u8; 2]; 4],
    slopes: [[i8; 2]; 4],
}

impl Marks {
    fn of(world: &GameWorld, col: Col) -> Self {
        // A grid of points in world voxels, whose edges neighbouring columns share.
        let size = CHUNK << col.0;
        let (x, z) = (col.1 * size, col.2 * size);
        let mut biomes = [0u32; 8];
        for k in 0..25 {
            let (dx, dz) = ((k % 5) as i32 * size / 4, (k / 5) as i32 * size / 4);
            biomes[k / 4] |= (crate::farview::biome_at(world, x + dx, z + dz) as u32) << (8 * (k % 4));
        }
        let corners = [(0, 0), (size, 0), (0, size), (size, size)];
        Self {
            biomes,
            skylines: corners.map(|(dx, dz)| crate::farview::skylines_at(world, x + dx, z + dz)),
            slopes: corners.map(|(dx, dz)| crate::farview::slope_at(world, x + dx, z + dz)),
        }
    }
}

struct ColumnResult {
    col: Col,
    marks: Marks,
    /// None if the job was skipped because the camera had moved away.
    chunks: Option<Vec<(i32, ChunkMesh)>>,
    /// Whether the small plants are in these meshes.
    planted: bool,
    cascades: Vec<Cascade>,
    springs: Vec<SpringMark>,
    surf: Vec<Surf>,
    /// Lit lantern voxels in these chunks, in world voxels.
    lamps: Vec<IVec3>,
    gen_ms: f32,
    mesh_ms: f32,
}

#[derive(Default, Clone, Copy)]
pub struct Stats {
    pub columns: usize,
    pub chunks: usize,
    pub quads: u64,
    pub mesh_bytes: u64,
    pub arena_bytes: u64,
    pub queued: usize,
    pub in_flight: usize,
    /// Smoothed per-column times on a worker thread.
    pub gen_ms: f32,
    pub mesh_ms: f32,
    /// For each level: columns built, and the worker time generating and meshing them
    /// in total, in milliseconds.
    pub built: [(u32, f32, f32); LEVELS],
}

/// Camera column and load radius, shared with workers so stale jobs can bail out.
struct Focus {
    cx: AtomicI32,
    cz: AtomicI32,
    radius: AtomicI32,
}

impl Focus {
    /// Distance from the camera's column to the middle of `col`, in full-size columns.
    fn distance(&self, col: Col) -> f32 {
        let s = step(col.0);
        let dx = (col.1 as f32 + 0.5) * s - (self.cx.load(Ordering::Relaxed) as f32 + 0.5);
        let dz = (col.2 as f32 + 0.5) * s - (self.cz.load(Ordering::Relaxed) as f32 + 0.5);
        dx.hypot(dz)
    }

    fn in_range(&self, col: Col, slack: i32) -> bool {
        let outer = reach(col.0, self.radius.load(Ordering::Relaxed));
        self.distance(col) <= outer + slack as f32 * step(col.0)
    }
}

pub struct Streamer {
    world: Arc<GameWorld>,
    /// Trees rebuilt for the coarser levels.
    models: Arc<lod::Models>,
    pool: rayon::ThreadPool,
    tx: Sender<ColumnResult>,
    rx: Receiver<ColumnResult>,
    columns: HashMap<Col, Vec<GpuChunk>>,
    /// Loaded columns that were built without their small plants, being too far away
    /// for them to be drawn.
    bare: HashSet<Col>,
    arena: VertexArena,
    /// Cascade lips in each loaded column, for spray and sound.
    cascades: HashMap<Col, Vec<Cascade>>,
    springs: HashMap<Col, Vec<SpringMark>>,
    surf: HashMap<Col, Vec<Surf>>,
    /// Lantern voxels in each loaded column, to light the ground around them.
    lamps: HashMap<Col, Vec<IVec3>>,
    pending: HashSet<Col>,
    /// Columns still to request, the most pressing last (so it is popped first).
    wanted: Vec<Col>,
    /// How far out each level is drawn (see `bands`), following what is loaded.
    shown: [f32; LEVELS],
    last_update: Option<Instant>,
    center: Option<(i32, i32)>,
    focus: Arc<Focus>,
    max_in_flight: usize,
    /// The largest load radius the GPU has shown it can hold.
    radius_limit: i32,
    /// The load radius asked for, which the view goes back out to as room allows.
    asked: i32,
    /// When the view was last pulled in or let out for GPU memory.
    resized: Instant,
    pub stats: Stats,
}

/// Columns stay loaded this far beyond the load radius, so small movements don't thrash.
const UNLOAD_SLACK: i32 = 2;
/// The view is never pulled in closer than this for lack of GPU memory.
const MIN_RADIUS: i32 = 8;
/// How long the edge of a level takes to move most of the way out to newly loaded
/// columns, in seconds: it glides out rather than jumping a column at a time.
const BAND_EASE: f32 = 0.2;
/// How much further ahead of the finest level each coarser one is kept while there is
/// more to build than time to build it, as a multiple per level of the twice that their
/// sizes give them. The far ground is what a view is made of, and is cheap for its area.
const COARSE_LEAD: f32 = 1.5;
/// Seconds between steps back out of a view that was pulled in for lack of GPU memory.
const REGROW_SECONDS: f32 = 4.0;
/// How far from the camera small plants are drawn.
pub const PLANT_REACH: f32 = m(100.0);
/// Columns within this many columns of the camera carry their small plants: the reach
/// of the plants, plus a margin so they are ready before they come into it.
const PLANT_COLUMNS: i32 = (PLANT_REACH / CHUNK as f32) as i32 + 3;
/// Roughly what one column takes on the GPU, for sizing the mesh buffer up front.
const COLUMN_BYTES: u64 = 130 << 10;
/// Roughly how much of that the coarse rings add, as a share of the full-size columns.
const RINGS_SHARE: f64 = 1.35;
/// Room reserved beyond that, for rugged or densely wooded land whose meshes run
/// larger than average. Growing later means a stall while everything is copied.
const HEADROOM: f64 = 1.5;

impl Streamer {
    pub fn new(gpu: &Gpu, world: Arc<GameWorld>, radius: i32) -> Self {
        let threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .saturating_sub(2)
            .max(1);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|i| format!("chunk-{i}"))
            .build()
            .expect("thread pool");
        let (tx, rx) = channel();
        let expected = (std::f64::consts::PI * (radius * radius) as f64 * (1.0 + RINGS_SHARE) * HEADROOM) as u64 * COLUMN_BYTES;
        Self {
            world,
            models: Arc::default(),
            pool,
            tx,
            rx,
            columns: HashMap::new(),
            bare: HashSet::new(),
            arena: VertexArena::new(gpu, "chunk arena", expected, 4 * size_of::<Vertex>() as u64),
            cascades: HashMap::new(),
            springs: HashMap::new(),
            surf: HashMap::new(),
            lamps: HashMap::new(),
            pending: HashSet::new(),
            wanted: Vec::new(),
            shown: [0.0; LEVELS],
            last_update: None,
            center: None,
            focus: Arc::new(Focus {
                cx: AtomicI32::new(0),
                cz: AtomicI32::new(0),
                radius: AtomicI32::new(radius),
            }),
            max_in_flight: threads * 3,
            radius_limit: i32::MAX,
            asked: radius,
            resized: Instant::now(),
            stats: Stats::default(),
        }
    }

    pub fn radius(&self) -> i32 {
        self.focus.radius.load(Ordering::Relaxed)
    }

    pub fn set_radius(&mut self, radius: i32) {
        self.asked = radius;
        let radius = radius.min(self.radius_limit);
        if radius != self.radius() {
            self.focus.radius.store(radius, Ordering::Relaxed);
            self.center = None; // force a rebuild of the wanted list
        }
    }

    /// For each level, the horizontal distance from the camera out to which it is drawn,
    /// in voxels: a level is drawn from the previous level's distance to its own. Beyond
    /// the last, the far view covers for terrain not loaded yet.
    pub fn bands(&self) -> [f32; LEVELS] {
        self.shown
    }

    /// For each level, the horizontal distance from the camera out to which it, or a
    /// finer level, is fully loaded, in voxels.
    fn loaded_bands(&self) -> [f32; LEVELS] {
        let mut reached = [0.0; LEVELS];
        if self.center.is_none() {
            return reached;
        }
        let radius = self.radius();
        let mut inside = 0.0f32;
        for level in 0..LEVELS as u8 {
            let s = step(level);
            // A column being rebuilt with its plants is still there, so it is no gap; nor
            // is one missing from under the finer levels. And starting from the edge that
            // walking uncovers keeps the boundary still as the camera crosses columns,
            // rather than pulsing in and out.
            let nearest_missing = self
                .pending
                .iter()
                .chain(&self.wanted)
                .filter(|c| c.0 == level && !self.columns.contains_key(*c))
                .map(|c| self.focus.distance(*c))
                .filter(|d| (*d + s) * CHUNK as f32 >= inside)
                .fold(reach(level, radius) - s * std::f32::consts::SQRT_2, f32::min);
            // The camera can be anywhere in its column, and columns are square.
            let edge = ((nearest_missing - 1.5 * s) * CHUNK as f32).max(0.0);
            inside = edge.max(inside);
            reached[level as usize] = inside;
        }
        reached
    }

    /// Moves the edges of the levels toward what is loaded: in at once, as what is gone
    /// cannot be drawn, and out smoothly.
    fn follow_bands(&mut self, dt: f32) {
        let loaded = self.loaded_bands();
        let ease = 1.0 - (-dt / BAND_EASE).exp();
        let mut inside = 0.0f32;
        for (shown, loaded) in self.shown.iter_mut().zip(loaded) {
            // The last voxels are closed up outright, or the edge would creep for ever.
            let eased = *shown + (loaded - *shown) * ease;
            *shown = if loaded - eased < 2.0 { loaded } else { eased }.min(loaded).max(inside);
            inside = *shown;
        }
    }

    /// A line on how far loading has got, for the log of a timed run.
    pub fn progress(&self) -> String {
        let count = |level: u8, cols: &mut dyn Iterator<Item = &Col>| cols.filter(|c| c.0 == level).count();
        let levels: Vec<String> = (0..LEVELS as u8)
            .map(|l| format!("{}+{}q+{}p", count(l, &mut self.columns.keys()), count(l, &mut self.wanted.iter()), count(l, &mut self.pending.iter())))
            .collect();
        let bands = self.bands().map(|b| (b / CHUNK as f32).round());
        format!("radius {}, columns loaded+queued+building by level {}, bands {bands:?} columns, meshes {} of {} MiB", self.radius(), levels.join(" "), self.stats.mesh_bytes >> 20, self.arena.capacity_bytes() >> 20)
    }

    pub fn is_idle(&self) -> bool {
        self.wanted.is_empty() && self.pending.is_empty()
    }

    pub fn chunks(&self) -> impl Iterator<Item = &GpuChunk> {
        self.columns.values().flatten()
    }

    /// The buffer every chunk's vertices live in.
    pub fn vertices(&self) -> &wgpu::Buffer {
        self.arena.buffer()
    }

    /// Cascade lips within `radius` voxels of `pos` (horizontally), from loaded columns.
    pub fn cascades_near(&self, pos: DVec3, radius: f32) -> impl Iterator<Item = &Cascade> {
        let r = (radius / CHUNK as f32).ceil() as i32 + 1;
        let (cx, cz) = ((pos.x / CHUNK as f64).floor() as i32, (pos.z / CHUNK as f64).floor() as i32);
        let p = pos.as_vec3();
        (-r..=r)
            .flat_map(move |dz| (-r..=r).map(move |dx| (0, cx + dx, cz + dz)))
            .filter_map(move |col| self.cascades.get(&col))
            .flatten()
            .filter(move |c| (c.pos.x - p.x).hypot(c.pos.z - p.z) <= radius)
    }

    /// Surf on rock within `radius` voxels of `pos` (horizontally), from loaded columns.
    pub fn surf_near(&self, pos: DVec3, radius: f32) -> impl Iterator<Item = &Surf> {
        let r = (radius / CHUNK as f32).ceil() as i32 + 1;
        let (cx, cz) = ((pos.x / CHUNK as f64).floor() as i32, (pos.z / CHUNK as f64).floor() as i32);
        let p = pos.as_vec3();
        (-r..=r)
            .flat_map(move |dz| (-r..=r).map(move |dx| (0, cx + dx, cz + dz)))
            .filter_map(move |col| self.surf.get(&col))
            .flatten()
            .filter(move |s| (s.pos.x - p.x).hypot(s.pos.z - p.z) <= radius)
    }

    /// Spring pools within `radius` voxels of `pos` (horizontally), from loaded columns.
    pub fn springs_near(&self, pos: DVec3, radius: f32) -> impl Iterator<Item = &SpringMark> {
        let r = (radius / CHUNK as f32).ceil() as i32 + 1;
        let (cx, cz) = ((pos.x / CHUNK as f64).floor() as i32, (pos.z / CHUNK as f64).floor() as i32);
        let p = pos.as_vec3();
        (-r..=r)
            .flat_map(move |dz| (-r..=r).map(move |dx| (0, cx + dx, cz + dz)))
            .filter_map(move |col| self.springs.get(&col))
            .flatten()
            .filter(move |s| (s.pos.x - p.x).hypot(s.pos.z - p.z) <= radius)
    }

    /// Lantern voxels within `radius` voxels of `pos` (horizontally), from loaded columns.
    pub fn lamps_near(&self, pos: DVec3, radius: f32) -> impl Iterator<Item = IVec3> + '_ {
        let r = (radius / CHUNK as f32).ceil() as i32 + 1;
        let (cx, cz) = ((pos.x / CHUNK as f64).floor() as i32, (pos.z / CHUNK as f64).floor() as i32);
        (-r..=r)
            .flat_map(move |dz| (-r..=r).map(move |dx| (0, cx + dx, cz + dz)))
            .filter_map(move |col| self.lamps.get(&col))
            .flatten()
            .copied()
            .filter(move |p| ((p.x as f64 - pos.x).hypot(p.z as f64 - pos.z)) <= radius as f64)
    }

    /// Call once per frame. Uploads finished columns for about `upload_ms` at most.
    pub fn update(&mut self, gpu: &Gpu, cam: DVec3, upload_ms: f32) {
        let center = (
            (cam.x / CHUNK as f64).floor() as i32,
            (cam.z / CHUNK as f64).floor() as i32,
        );
        if self.center != Some(center) {
            self.center = Some(center);
            self.focus.cx.store(center.0, Ordering::Relaxed);
            self.focus.cz.store(center.1, Ordering::Relaxed);
            self.rebuild_wanted(center);
        }

        while self.pending.len() < self.max_in_flight {
            let Some(col) = self.wanted.pop() else { break };
            self.pending.insert(col);
            let plants = col.0 == 0 && in_plant_range(col, center, PLANT_COLUMNS + 1);
            let (world, models, tx, focus) = (self.world.clone(), self.models.clone(), self.tx.clone(), self.focus.clone());
            self.pool.spawn(move || {
                let result = build_column(&world, &models, &focus, col, plants);
                let _ = tx.send(result);
            });
        }

        let began = Instant::now();
        while began.elapsed().as_secs_f32() * 1000.0 < upload_ms {
            let Ok(result) = self.rx.try_recv() else { break };
            self.pending.remove(&result.col);
            let in_range = self.focus.in_range(result.col, UNLOAD_SLACK);
            match result.chunks {
                Some(meshes) if in_range => {
                    let s = &mut self.stats;
                    s.gen_ms += (result.gen_ms - s.gen_ms) * 0.05;
                    s.mesh_ms += (result.mesh_ms - s.mesh_ms) * 0.05;
                    let b = &mut s.built[result.col.0 as usize];
                    *b = (b.0 + 1, b.1 + result.gen_ms, b.2 + result.mesh_ms);
                    let Some(chunks) = self.upload_column(gpu, result.col, meshes, result.marks) else {
                        // Out of GPU memory. The rest can wait until the view has shrunk.
                        self.shrink_view();
                        break;
                    };
                    if let Some(old) = self.columns.insert(result.col, chunks) {
                        release(&mut self.arena, &old);
                        self.forget(&old);
                    }
                    if result.planted {
                        self.bare.remove(&result.col);
                    } else {
                        self.bare.insert(result.col);
                    }
                    if !result.cascades.is_empty() {
                        self.cascades.insert(result.col, result.cascades);
                    }
                    if !result.springs.is_empty() {
                        self.springs.insert(result.col, result.springs);
                    }
                    if !result.surf.is_empty() {
                        self.surf.insert(result.col, result.surf);
                    }
                    if result.lamps.is_empty() {
                        self.lamps.remove(&result.col);
                    } else {
                        self.lamps.insert(result.col, result.lamps);
                    }
                }
                // Skipped by the worker but wanted again by now: put it back in the queue.
                None if self.focus.in_range(result.col, 0) => self.wanted.push(result.col),
                _ => {}
            }
        }
        self.regrow_view();
        let now = Instant::now();
        let dt = self.last_update.map_or(f32::INFINITY, |t| (now - t).as_secs_f32());
        self.last_update = Some(now);
        self.follow_bands(dt);
        self.stats.columns = self.columns.len();
        self.stats.queued = self.wanted.len();
        self.stats.in_flight = self.pending.len();
        self.stats.arena_bytes = self.arena.capacity_bytes();
    }

    /// Builds again the column the voxel column `(x, z)` stands in, as soon as may be:
    /// something on it has been picked, or has grown back. What is there stays until
    /// the new one is ready.
    pub fn replant(&mut self, x: i32, z: i32) {
        let col = (0, x.div_euclid(CHUNK), z.div_euclid(CHUNK));
        if self.columns.contains_key(&col) && !self.wanted.contains(&col) {
            self.wanted.push(col);
        }
    }

    /// Blocks until everything in range is loaded (used for screenshots).
    pub fn wait_idle(&mut self, gpu: &Gpu, cam: DVec3) {
        loop {
            self.update(gpu, cam, f32::INFINITY);
            if self.is_idle() {
                self.follow_bands(f32::INFINITY);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    fn rebuild_wanted(&mut self, center: (i32, i32)) {
        let r = self.radius();
        let focus = self.focus.clone();
        let before = self.columns.len();
        let arena = &mut self.arena;
        self.columns.retain(|c, chunks| {
            let keep = focus.in_range(*c, UNLOAD_SLACK);
            if !keep {
                release(arena, chunks);
            }
            keep
        });
        self.cascades.retain(|c, _| focus.in_range(*c, UNLOAD_SLACK));
        self.springs.retain(|c, _| focus.in_range(*c, UNLOAD_SLACK));
        self.surf.retain(|c, _| focus.in_range(*c, UNLOAD_SLACK));
        self.lamps.retain(|c, _| focus.in_range(*c, UNLOAD_SLACK));
        self.bare.retain(|c| focus.in_range(*c, UNLOAD_SLACK));
        if self.columns.len() != before {
            self.recount();
        }

        self.wanted.clear();
        for level in 0..LEVELS as u8 {
            let s = step(level) as i32;
            let (cx, cz) = (center.0.div_euclid(s), center.1.div_euclid(s));
            // A column of this level counts by its middle, so reach one further each way.
            let span = (reach(level, r) / step(level)).ceil() as i32 + 1;
            for dx in -span..=span {
                for dz in -span..=span {
                    let col = (level, cx + dx, cz + dz);
                    if !focus.in_range(col, 0) || self.pending.contains(&col) {
                        continue;
                    }
                    // A column that has come within reach of its plants is built again with them.
                    let incomplete = !self.columns.contains_key(&col) || (self.bare.contains(&col) && in_plant_range(col, center, PLANT_COLUMNS));
                    if incomplete {
                        self.wanted.push(col);
                    }
                }
            }
        }
        // Each level is built outward at its own scale and the coarser ones a little
        // sooner still, so that they keep well ahead of the camera rather than waiting
        // for the finest to finish: at speed it never does, and they would never start.
        // Columns only wanted again for their plants come after everything missing.
        let columns = &self.columns;
        self.wanted.sort_unstable_by_key(|c| {
            let replant = if columns.contains_key(c) { 1 << 40 } else { 0 };
            let lead = step(c.0) * COARSE_LEAD.powi(c.0 as i32);
            std::cmp::Reverse((focus.distance(*c) / lead * 16.0) as i64 + replant)
        });
    }

    /// Uploads a whole column, or nothing at all if the GPU cannot hold it.
    fn upload_column(&mut self, gpu: &Gpu, col: Col, meshes: Vec<(i32, ChunkMesh)>, marks: Marks) -> Option<Vec<GpuChunk>> {
        let mut chunks = Vec::with_capacity(meshes.len());
        for (cy, mesh) in meshes {
            let Some(mut chunk) = self.upload(gpu, col.0, IVec3::new(col.1, cy, col.2), mesh) else {
                release(&mut self.arena, &chunks);
                self.recount();
                return None;
            };
            chunk.biomes = marks.biomes;
            chunk.skylines = marks.skylines;
            chunk.slopes = marks.slopes;
            chunks.push(chunk);
        }
        Some(chunks)
    }

    /// Stores one chunk's meshes, or None if the GPU has no room for them.
    fn upload(&mut self, gpu: &Gpu, level: u8, pos: IVec3, mesh: ChunkMesh) -> Option<GpuChunk> {
        let opaque = self.store(gpu, &mesh.opaque)?;
        let Some(water) = self.store(gpu, &mesh.water) else {
            if let Some(span) = opaque {
                self.arena.remove(span);
            }
            return None;
        };
        self.stats.chunks += 1;
        Some(GpuChunk {
            level,
            pos,
            opaque,
            water,
            faces: mesh.faces,
            biomes: [0; 8],
            skylines: [[0; 2]; 4],
            slopes: [[0; 2]; 4],
        })
    }

    /// Puts vertices in the arena. The outer None means the GPU has no room; the inner
    /// one that there was nothing to store.
    fn store(&mut self, gpu: &Gpu, verts: &[Vertex]) -> Option<Option<MeshSpan>> {
        if verts.is_empty() {
            return Some(None);
        }
        let span = self.arena.add(gpu, verts)?;
        self.stats.quads += span.quads as u64;
        self.stats.mesh_bytes += size_of_val(verts) as u64;
        Some(Some(span))
    }

    /// Pulls the view in after running out of GPU memory, which unloads the outer
    /// columns to make room, and stops it being pushed back out.
    fn shrink_view(&mut self) {
        let now = self.radius();
        let next = (now - (now / 8).max(1)).max(MIN_RADIUS);
        self.radius_limit = next;
        self.resized = Instant::now();
        if next < now {
            log::warn!("out of GPU memory: load radius reduced from {now} to {next}");
            self.focus.radius.store(next, Ordering::Relaxed);
            self.center = None; // force a rebuild of the wanted list
        }
    }

    /// Lets a view that was pulled in back out a column at a time, once the meshes loaded
    /// leave room to spare: what filled the GPU was the land there, dense forest say, and
    /// the view should not stay small for the rest of the session on its account.
    fn regrow_view(&mut self) {
        let roomy = self.stats.mesh_bytes * 10 < self.arena.capacity_bytes() * 7;
        if self.radius_limit < self.asked && roomy && self.is_idle() && self.resized.elapsed().as_secs_f32() > REGROW_SECONDS {
            self.radius_limit += 1;
            self.resized = Instant::now();
            self.focus.radius.store(self.radius_limit, Ordering::Relaxed);
            self.center = None; // force a rebuild of the wanted list
        }
    }

    /// Takes replaced chunks out of the statistics.
    fn forget(&mut self, chunks: &[GpuChunk]) {
        let s = &mut self.stats;
        s.chunks = s.chunks.saturating_sub(chunks.len());
        for span in chunks.iter().flat_map(|c| c.opaque.iter().chain(&c.water)) {
            s.quads = s.quads.saturating_sub(span.quads as u64);
            s.mesh_bytes = s.mesh_bytes.saturating_sub(span.quads as u64 * 4 * size_of::<Vertex>() as u64);
        }
    }

    fn recount(&mut self) {
        let (mut chunks, mut quads) = (0, 0u64);
        for c in self.columns.values().flatten() {
            chunks += 1;
            quads += c.opaque.as_ref().map_or(0, |m| m.quads as u64);
            quads += c.water.as_ref().map_or(0, |m| m.quads as u64);
        }
        self.stats.chunks = chunks;
        self.stats.quads = quads;
        self.stats.mesh_bytes = quads * 4 * size_of::<Vertex>() as u64;
    }
}

/// The lit voxels (lanterns, embers) in a column's chunks, in world voxels.
fn lamp_voxels(chunks: &[(i32, voxel::PaddedChunk)], col: Col, mats: &voxel::Materials) -> Vec<IVec3> {
    use voxel::PAD;
    let lit: Vec<bool> = (0..mats.len()).map(|i| mats.get(i as voxel::Voxel).emissive).collect();
    let mut out = Vec::new();
    // Nearly every chunk holds nothing lit, and is passed over a run of voxels at a time
    // by asking only whether any is in the range of the lit materials.
    let Some(first) = lit.iter().position(|&l| l) else { return out };
    let span = (lit.iter().rposition(|&l| l).unwrap_or(first) - first) as voxel::Voxel;
    let in_range = |run: &[voxel::Voxel]| run.iter().fold(false, |any, &v| any | (v.wrapping_sub(first as voxel::Voxel) <= span));
    for (cy, chunk) in chunks {
        let voxels = chunk.voxels();
        if !voxels.chunks(256).any(in_range) {
            continue;
        }
        for (i, &v) in voxels.iter().enumerate() {
            if !lit[v as usize] {
                continue;
            }
            // Index order is (x * PAD + z) * PAD + y, with a one-voxel border.
            let (y, z, x) = (i % PAD, (i / PAD) % PAD, i / (PAD * PAD));
            if (1..=CHUNK as usize).contains(&x) && (1..=CHUNK as usize).contains(&z) && (1..=CHUNK as usize).contains(&y) {
                out.push(IVec3::new(col.1 * CHUNK + x as i32 - 1, cy * CHUNK + y as i32 - 1, col.2 * CHUNK + z as i32 - 1));
            }
        }
    }
    out
}

fn in_plant_range(col: Col, center: (i32, i32), columns: i32) -> bool {
    let (dx, dz) = (col.1 - center.0, col.2 - center.1);
    dx * dx + dz * dz <= columns * columns
}

fn release(arena: &mut VertexArena, chunks: &[GpuChunk]) {
    for span in chunks.iter().flat_map(|c| c.opaque.iter().chain(&c.water)) {
        arena.remove(*span);
    }
}

/// For each chunk of a column (in order of height, with no gaps), which of its padded
/// columns have something opaque or water in a chunk above it: those voxels never see
/// the sky, so no snow settles on them.
fn roofs(chunks: &[(i32, voxel::PaddedChunk)], mats: &voxel::Materials) -> Vec<Vec<bool>> {
    let mut out = vec![Vec::new(); chunks.len()];
    let mut covered = vec![false; voxel::PAD * voxel::PAD];
    for (i, (_, chunk)) in chunks.iter().enumerate().rev() {
        out[i] = covered.clone();
        cover_columns(chunk, mats, &mut covered);
    }
    out
}

fn build_column(world: &GameWorld, models: &lod::Models, focus: &Focus, col: Col, plants: bool) -> ColumnResult {
    if !focus.in_range(col, UNLOAD_SLACK) {
        return ColumnResult {
            col,
            marks: Marks::default(),
            chunks: None,
            planted: false,
            cascades: Vec::new(),
            springs: Vec::new(),
            surf: Vec::new(),
            lamps: Vec::new(),
            gen_ms: 0.0,
            mesh_ms: 0.0,
        };
    }
    if col.0 > 0 {
        return build_coarse_column(world, models, col);
    }
    let t0 = Instant::now();
    // Growing the small plants is most of the work of a column: not for one too far
    // away to show them.
    let output = if plants { world.generate_column(col.1, col.2) } else { world.generate_column_bare(col.1, col.2) };
    let t1 = Instant::now();
    let roofs = roofs(&output.chunks, world.materials());
    // From the top down, so that falling water carries on from each chunk into the next.
    let mut falls = Falls::default();
    let mut chunks: Vec<(i32, ChunkMesh)> = output
        .chunks
        .iter()
        .zip(&roofs)
        .rev()
        .map(|((cy, chunk), roof)| {
            let wash = wash_map(chunk, world.materials(), *cy, &output.wash);
            let mut mesh = mesh_chunk_under_sky(chunk, world.materials(), &output.currents, roof, &wash, &mut falls);
            let origin = [col.1 * CHUNK * CELLS, *cy * CHUNK * CELLS, col.2 * CHUNK * CELLS];
            // Run even with no plants: it also records where each face group ends.
            let inside = output.plants.iter().filter(|p| p.lo[1].div_euclid(CHUNK * CELLS) == *cy);
            add_plants(&mut mesh, inside, chunk, origin, world.materials());
            // Each tree's leaves go on its own date.
            let corner = [col.1 * CHUNK, *cy * CHUNK, col.2 * CHUNK];
            let mut tags = features::LeafTags::new(world.library(), world.materials(), &output.trees, IVec3::from_array(corner));
            tag_trees(&mut mesh, corner, |q| tags.tag(IVec3::from_array(q)));
            (*cy, mesh)
        })
        .filter(|(_, mesh)| !mesh.is_empty())
        .collect();
    chunks.reverse();
    let t2 = Instant::now();
    let lamps = lamp_voxels(&output.chunks, col, world.materials());
    ColumnResult {
        col,
        marks: Marks::of(world, col),
        chunks: Some(chunks),
        planted: plants,
        cascades: output.cascades,
        springs: output.springs,
        surf: output.surf,
        lamps,
        gen_ms: (t1 - t0).as_secs_f32() * 1000.0,
        mesh_ms: (t2 - t1).as_secs_f32() * 1000.0,
    }
}

/// A column of one of the coarse rings: no plants, cascades or spray this far out.
fn build_coarse_column(world: &GameWorld, models: &lod::Models, col: Col) -> ColumnResult {
    let t0 = Instant::now();
    let (chunks, trees, wash) = lod::generate(world, models, col.0, col.1, col.2);
    let t1 = Instant::now();
    // Trunks in coarse cells, for giving each tree's leaves its own date: here simply the
    // nearest tree's, as coarse cells are too big to say exactly.
    let step = 1 << col.0;
    let trunks: Vec<(IVec3, u8)> = trees.iter().map(|p| (p.pos.div_euclid(IVec3::splat(step)), features::tree_tag(p.pos))).collect();
    let roofs = roofs(&chunks, world.materials());
    let mut falls = Falls::default();
    let mut meshes: Vec<(i32, ChunkMesh)> = chunks
        .iter()
        .zip(&roofs)
        .rev()
        .map(|((cy, chunk), roof)| {
            let wash = wash_map(chunk, world.materials(), *cy, &wash);
            let mut mesh = mesh_chunk_under_sky(chunk, world.materials(), &[], roof, &wash, &mut falls);
            tag_trees(&mut mesh, [col.1 * CHUNK, *cy * CHUNK, col.2 * CHUNK], |q| {
                trunks
                    .iter()
                    .min_by_key(|(t, _)| (t.x - q[0]).pow(2) + (t.z - q[2]).pow(2))
                    .map_or(0, |t| t.1)
            });
            (*cy, mesh)
        })
        .filter(|(_, mesh)| !mesh.is_empty())
        .collect();
    meshes.reverse();
    let t2 = Instant::now();
    ColumnResult {
        col,
        marks: Marks::of(world, col),
        chunks: Some(meshes),
        // Coarse columns never need building again with plants.
        planted: true,
        cascades: Vec::new(),
        springs: Vec::new(),
        surf: Vec::new(),
        lamps: Vec::new(),
        gen_ms: (t1 - t0).as_secs_f32() * 1000.0,
        mesh_ms: (t2 - t1).as_secs_f32() * 1000.0,
    }
}
