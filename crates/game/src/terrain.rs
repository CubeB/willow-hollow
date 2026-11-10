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
    mesh::{CELLS, ChunkMesh, Vertex, add_plants, mesh_chunk},
};
use worldgen::{Cascade, World};

use crate::{
    arena::{MeshSpan, VertexArena},
    gfx::Gpu,
};

pub struct GpuChunk {
    /// Chunk coordinates (world voxel position / 32).
    pub pos: IVec3,
    pub opaque: Option<MeshSpan>,
    pub water: Option<MeshSpan>,
    /// Where each face direction starts within `opaque`, in quads.
    pub faces: [u32; 13],
}

type Col = (i32, i32);

struct ColumnResult {
    col: Col,
    /// None if the job was skipped because the camera had moved away.
    chunks: Option<Vec<(i32, ChunkMesh)>>,
    cascades: Vec<Cascade>,
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
    fn in_range(&self, col: Col, slack: i32) -> bool {
        let dx = (col.0 - self.cx.load(Ordering::Relaxed)) as i64;
        let dz = (col.1 - self.cz.load(Ordering::Relaxed)) as i64;
        let r = (self.radius.load(Ordering::Relaxed) + slack) as i64;
        dx * dx + dz * dz <= r * r
    }
}

pub struct Streamer {
    world: Arc<dyn World>,
    pool: rayon::ThreadPool,
    tx: Sender<ColumnResult>,
    rx: Receiver<ColumnResult>,
    columns: HashMap<Col, Vec<GpuChunk>>,
    arena: VertexArena,
    /// Cascade lips in each loaded column, for spray and sound.
    cascades: HashMap<Col, Vec<Cascade>>,
    pending: HashSet<Col>,
    /// Columns still to request, farthest first (so the nearest is popped first).
    wanted: Vec<Col>,
    center: Option<Col>,
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

impl Streamer {
    pub fn new(gpu: &Gpu, world: Arc<dyn World>, radius: i32) -> Self {
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
        Self {
            world,
            pool,
            tx,
            rx,
            columns: HashMap::new(),
            arena: VertexArena::new(gpu),
            cascades: HashMap::new(),
            pending: HashSet::new(),
            wanted: Vec::new(),
            center: None,
            focus: Arc::new(Focus {
                cx: AtomicI32::new(0),
                cz: AtomicI32::new(0),
                radius: AtomicI32::new(radius),
            }),
            max_in_flight: threads * 2,
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

    /// Horizontal distance from the camera within which every column is loaded, in
    /// voxels. Outside it the far view must cover for terrain still being generated.
    pub fn complete_radius(&self) -> f32 {
        let Some(center) = self.center else { return 0.0 };
        let dist = |c: &Col| {
            let (dx, dz) = ((c.0 - center.0) as f32, (c.1 - center.1) as f32);
            dx.hypot(dz)
        };
        // `wanted` is sorted farthest first, so its last entry is the nearest gap.
        let nearest_missing = self
            .pending
            .iter()
            .map(dist)
            .chain(self.wanted.last().map(dist))
            .fold(self.radius() as f32 + 1.0, f32::min);
        // The camera can be anywhere in its column, and columns are square.
        ((nearest_missing - 1.5) * CHUNK as f32).max(0.0)
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
            .flat_map(move |dz| (-r..=r).map(move |dx| (cx + dx, cz + dz)))
            .filter_map(move |col| self.cascades.get(&col))
            .flatten()
            .filter(move |c| (c.pos.x - p.x).hypot(c.pos.z - p.z) <= radius)
    }

    /// Call once per frame. Uploads at most `upload_budget` finished columns.
    pub fn update(&mut self, gpu: &Gpu, cam: DVec3, upload_budget: usize) {
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
            let (world, tx, focus) = (self.world.clone(), self.tx.clone(), self.focus.clone());
            self.pool.spawn(move || {
                let result = build_column(&*world, &focus, col);
                let _ = tx.send(result);
            });
        }

        for _ in 0..upload_budget {
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
                    }
                    if !result.cascades.is_empty() {
                        self.cascades.insert(result.col, result.cascades);
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
            self.update(gpu, cam, usize::MAX);
            if self.is_idle() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    fn rebuild_wanted(&mut self, center: Col) {
        let r = self.radius();
        let unload = (r + UNLOAD_SLACK) as i64;
        let dist2 = |c: &Col| {
            let (dx, dz) = ((c.0 - center.0) as i64, (c.1 - center.1) as i64);
            dx * dx + dz * dz
        };
        let before = self.columns.len();
        let arena = &mut self.arena;
        self.columns.retain(|c, chunks| {
            let keep = dist2(c) <= unload * unload;
            if !keep {
                release(arena, chunks);
            }
            keep
        });
        self.cascades.retain(|c, _| dist2(c) <= unload * unload);
        if self.columns.len() != before {
            self.recount();
        }

        self.wanted.clear();
        for dx in -r..=r {
            for dz in -r..=r {
                let col = (center.0 + dx, center.1 + dz);
                if dx * dx + dz * dz <= r * r
                    && !self.columns.contains_key(&col)
                    && !self.pending.contains(&col)
                {
                    self.wanted.push(col);
                }
            }
        }
        self.wanted.sort_unstable_by_key(|c| std::cmp::Reverse(dist2(c)));
    }

    /// Uploads a whole column, or nothing at all if the GPU cannot hold it.
    fn upload_column(&mut self, gpu: &Gpu, col: Col, meshes: Vec<(i32, ChunkMesh)>) -> Option<Vec<GpuChunk>> {
        let mut chunks = Vec::with_capacity(meshes.len());
        for (cy, mesh) in meshes {
            let Some(chunk) = self.upload(gpu, IVec3::new(col.0, cy, col.1), mesh) else {
                release(&mut self.arena, &chunks);
                self.recount();
                return None;
            };
            chunks.push(chunk);
        }
        Some(chunks)
    }

    /// Stores one chunk's meshes, or None if the GPU has no room for them.
    fn upload(&mut self, gpu: &Gpu, pos: IVec3, mesh: ChunkMesh) -> Option<GpuChunk> {
        let opaque = self.store(gpu, &mesh.opaque)?;
        let Some(water) = self.store(gpu, &mesh.water) else {
            if let Some(span) = opaque {
                self.arena.remove(span);
            }
            return None;
        };
        self.stats.chunks += 1;
        Some(GpuChunk {
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

fn release(arena: &mut VertexArena, chunks: &[GpuChunk]) {
    for span in chunks.iter().flat_map(|c| c.opaque.iter().chain(&c.water)) {
        arena.remove(*span);
    }
}

fn build_column(world: &dyn World, focus: &Focus, col: Col) -> ColumnResult {
    if !focus.in_range(col, UNLOAD_SLACK) {
        return ColumnResult {
            col,
            chunks: None,
            cascades: Vec::new(),
            gen_ms: 0.0,
            mesh_ms: 0.0,
        };
    }
    let t0 = Instant::now();
    let output = world.generate_column(col.0, col.1);
    let t1 = Instant::now();
    let chunks = output
        .chunks
        .iter()
        .map(|(cy, chunk)| {
            let mut mesh = mesh_chunk(chunk, world.materials());
            let origin = [col.0 * CHUNK * CELLS, *cy * CHUNK * CELLS, col.1 * CHUNK * CELLS];
            let inside = output.plants.iter().filter(|p| p.lo[1].div_euclid(CHUNK * CELLS) == *cy);
            add_plants(&mut mesh, inside, origin, world.materials());
            (*cy, mesh)
        })
        .filter(|(_, mesh)| !mesh.is_empty())
        .collect();
    let t2 = Instant::now();
    ColumnResult {
        col,
        chunks: Some(chunks),
        cascades: output.cascades,
        gen_ms: (t1 - t0).as_secs_f32() * 1000.0,
        mesh_ms: (t2 - t1).as_secs_f32() * 1000.0,
    }
}
