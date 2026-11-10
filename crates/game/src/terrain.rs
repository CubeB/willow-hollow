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
    mesh::{CELLS, ChunkMesh, Vertex, add_plants, mesh_chunk, mesh_chunk_with_currents},
};
use worldgen::{Cascade, SpringMark, Surf, World, scale::m, world::GameWorld};

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
}

/// Levels of detail, the first being full-size voxels.
pub const LEVELS: usize = 3;

/// A column at some level of detail: (level, x, z), in columns of that level.
type Col = (u8, i32, i32);

/// Where each level is loaded, as (inner, outer) in full-size columns from the camera.
/// Each ring is twice as far out as the one inside it, and reaches four of its own
/// columns in under it. The finer ring is only ever complete to a little short of its
/// edge, so the coarser one must be there underneath to take over.
fn ring(level: u8, radius: i32) -> (f32, f32) {
    let outer = |level: u8| radius as f32 * step(level);
    match level {
        0 => (0.0, outer(0)),
        _ => ((outer(level - 1) - 4.0 * step(level)).max(0.0), outer(level)),
    }
}

/// How many full-size columns one column of a level spans.
fn step(level: u8) -> f32 {
    (1 << level) as f32
}

struct ColumnResult {
    col: Col,
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
        let (inner, outer) = ring(col.0, self.radius.load(Ordering::Relaxed));
        let (d, slack) = (self.distance(col), slack as f32 * step(col.0));
        d <= outer + slack && (col.0 == 0 || d >= inner - slack)
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
    /// Columns still to request, farthest first (so the nearest is popped first).
    wanted: Vec<Col>,
    center: Option<(i32, i32)>,
    focus: Arc<Focus>,
    max_in_flight: usize,
    /// The largest load radius the GPU has shown it can hold.
    radius_limit: i32,
    pub stats: Stats,
}

/// Columns stay loaded this far beyond the load radius, so small movements don't thrash.
const UNLOAD_SLACK: i32 = 2;
/// The view is never pulled in closer than this for lack of GPU memory.
const MIN_RADIUS: i32 = 8;
/// How far from the camera small plants are drawn.
pub const PLANT_REACH: f32 = m(100.0);
/// Columns within this many columns of the camera carry their small plants: the reach
/// of the plants, plus a margin so they are ready before they come into it.
const PLANT_COLUMNS: i32 = (PLANT_REACH / CHUNK as f32) as i32 + 3;
/// Roughly what one column takes on the GPU, for sizing the mesh buffer up front.
const COLUMN_BYTES: u64 = 130 << 10;
/// Roughly how much of that the coarse rings add, as a share of the full-size columns.
const RINGS_SHARE: f64 = 1.2;
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
            center: None,
            focus: Arc::new(Focus {
                cx: AtomicI32::new(0),
                cz: AtomicI32::new(0),
                radius: AtomicI32::new(radius),
            }),
            max_in_flight: threads * 3,
            radius_limit: i32::MAX,
            stats: Stats::default(),
        }
    }

    pub fn radius(&self) -> i32 {
        self.focus.radius.load(Ordering::Relaxed)
    }

    pub fn set_radius(&mut self, radius: i32) {
        let radius = radius.min(self.radius_limit);
        if radius != self.radius() {
            self.focus.radius.store(radius, Ordering::Relaxed);
            self.center = None; // force a rebuild of the wanted list
        }
    }

    /// For each level, the horizontal distance from the camera out to which it, and
    /// every finer level, is fully loaded, in voxels. A level is drawn from the previous
    /// level's distance to its own. Beyond the last, the far view covers for terrain not
    /// loaded yet.
    pub fn bands(&self) -> [f32; LEVELS] {
        let mut reached = [0.0; LEVELS];
        if self.center.is_none() {
            return reached;
        }
        let radius = self.radius();
        let mut inside = 0.0f32;
        for level in 0..LEVELS as u8 {
            let (inner, outer) = ring(level, radius);
            let s = step(level);
            // A column being rebuilt with its plants is still there, so it is no gap. And
            // starting from the edge of the ring that walking uncovers keeps the boundary
            // still as the camera crosses columns, rather than pulsing in and out.
            let nearest_missing = self
                .pending
                .iter()
                .chain(&self.wanted)
                .filter(|c| c.0 == level && !self.columns.contains_key(*c))
                .map(|c| self.focus.distance(*c))
                .filter(|d| (*d + s) * CHUNK as f32 >= inside)
                .fold(outer - s * std::f32::consts::SQRT_2, f32::min);
            // The camera can be anywhere in its column, and columns are square.
            let edge = ((nearest_missing - 1.5 * s) * CHUNK as f32).max(0.0);
            // A level that doesn't reach in as far as the one before it ends can't take over.
            inside = if inside < inner * CHUNK as f32 { inside } else { edge.max(inside) };
            reached[level as usize] = inside;
        }
        reached
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
                    let Some(chunks) = self.upload_column(gpu, result.col, meshes) else {
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
        self.stats.columns = self.columns.len();
        self.stats.queued = self.wanted.len();
        self.stats.in_flight = self.pending.len();
        self.stats.arena_bytes = self.arena.capacity_bytes();
    }

    /// Blocks until everything in range is loaded (used for screenshots).
    pub fn wait_idle(&mut self, gpu: &Gpu, cam: DVec3) {
        loop {
            self.update(gpu, cam, f32::INFINITY);
            if self.is_idle() {
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
            let reach = (ring(level, r).1 / step(level)).ceil() as i32 + 1;
            for dx in -reach..=reach {
                for dz in -reach..=reach {
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
        self.wanted.sort_unstable_by_key(|c| std::cmp::Reverse((focus.distance(*c) * 16.0) as i64));
    }

    /// Uploads a whole column, or nothing at all if the GPU cannot hold it.
    fn upload_column(&mut self, gpu: &Gpu, col: Col, meshes: Vec<(i32, ChunkMesh)>) -> Option<Vec<GpuChunk>> {
        let mut chunks = Vec::with_capacity(meshes.len());
        for (cy, mesh) in meshes {
            let Some(chunk) = self.upload(gpu, col.0, IVec3::new(col.1, cy, col.2), mesh) else {
                release(&mut self.arena, &chunks);
                self.recount();
                return None;
            };
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
        if next < now {
            log::warn!("out of GPU memory: load radius reduced from {now} to {next}");
            self.focus.radius.store(next, Ordering::Relaxed);
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
    for (cy, chunk) in chunks {
        let voxels = chunk.voxels();
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

fn build_column(world: &GameWorld, models: &lod::Models, focus: &Focus, col: Col, plants: bool) -> ColumnResult {
    if !focus.in_range(col, UNLOAD_SLACK) {
        return ColumnResult {
            col,
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
    let output = world.generate_column(col.1, col.2);
    let t1 = Instant::now();
    let chunks = output
        .chunks
        .iter()
        .map(|(cy, chunk)| {
            let mut mesh = mesh_chunk_with_currents(chunk, world.materials(), &output.currents);
            let origin = [col.1 * CHUNK * CELLS, *cy * CHUNK * CELLS, col.2 * CHUNK * CELLS];
            // Run even with no plants: it also records where each face group ends.
            let inside = output.plants.iter().filter(|p| plants && p.lo[1].div_euclid(CHUNK * CELLS) == *cy);
            add_plants(&mut mesh, inside, origin, world.materials());
            (*cy, mesh)
        })
        .filter(|(_, mesh)| !mesh.is_empty())
        .collect();
    let t2 = Instant::now();
    let lamps = lamp_voxels(&output.chunks, col, world.materials());
    ColumnResult {
        col,
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
    let chunks = lod::generate(world, models, col.0, col.1, col.2);
    let t1 = Instant::now();
    let meshes = chunks
        .iter()
        .map(|(cy, chunk)| (*cy, mesh_chunk(chunk, world.materials())))
        .filter(|(_, mesh)| !mesh.is_empty())
        .collect();
    let t2 = Instant::now();
    ColumnResult {
        col,
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
