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
    mesh::{ChunkMesh, Vertex, mesh_chunk},
};
use wgpu::util::DeviceExt;
use worldgen::World;

use crate::gfx::Gpu;

pub struct MeshBuf {
    pub buffer: wgpu::Buffer,
    pub quads: u32,
}

pub struct GpuChunk {
    /// Chunk coordinates (world voxel position / 32).
    pub pos: IVec3,
    pub opaque: Option<MeshBuf>,
    pub water: Option<MeshBuf>,
}

type Col = (i32, i32);

struct ColumnResult {
    col: Col,
    /// None if the job was skipped because the camera had moved away.
    chunks: Option<Vec<(i32, ChunkMesh)>>,
    gen_ms: f32,
    mesh_ms: f32,
}

#[derive(Default, Clone, Copy)]
pub struct Stats {
    pub columns: usize,
    pub chunks: usize,
    pub quads: u64,
    pub mesh_bytes: u64,
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
    pending: HashSet<Col>,
    /// Columns still to request, farthest first (so the nearest is popped first).
    wanted: Vec<Col>,
    center: Option<Col>,
    focus: Arc<Focus>,
    max_in_flight: usize,
    pub stats: Stats,
}

/// Columns stay loaded this far beyond the load radius, so small movements don't thrash.
const UNLOAD_SLACK: i32 = 2;

impl Streamer {
    pub fn new(world: Arc<dyn World>, radius: i32) -> Self {
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
            pending: HashSet::new(),
            wanted: Vec::new(),
            center: None,
            focus: Arc::new(Focus {
                cx: AtomicI32::new(0),
                cz: AtomicI32::new(0),
                radius: AtomicI32::new(radius),
            }),
            max_in_flight: threads * 2,
            stats: Stats::default(),
        }
    }

    pub fn radius(&self) -> i32 {
        self.focus.radius.load(Ordering::Relaxed)
    }

    pub fn set_radius(&mut self, radius: i32) {
        if radius != self.radius() {
            self.focus.radius.store(radius, Ordering::Relaxed);
            self.center = None; // force a rebuild of the wanted list
        }
    }

    pub fn is_idle(&self) -> bool {
        self.wanted.is_empty() && self.pending.is_empty()
    }

    pub fn chunks(&self) -> impl Iterator<Item = &GpuChunk> {
        self.columns.values().flatten()
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
                    let chunks = meshes
                        .into_iter()
                        .map(|(cy, mesh)| self.upload(gpu, IVec3::new(result.col.0, cy, result.col.1), mesh))
                        .collect();
                    self.columns.insert(result.col, chunks);
                }
                // Skipped by the worker but wanted again by now: put it back in the queue.
                None if self.focus.in_range(result.col, 0) => self.wanted.push(result.col),
                _ => {}
            }
        }
        self.stats.columns = self.columns.len();
        self.stats.queued = self.wanted.len();
        self.stats.in_flight = self.pending.len();
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
        self.columns.retain(|c, _| dist2(c) <= unload * unload);
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

    fn upload(&mut self, gpu: &Gpu, pos: IVec3, mesh: ChunkMesh) -> GpuChunk {
        let mut make = |verts: &[Vertex], label: &str| -> Option<MeshBuf> {
            if verts.is_empty() {
                return None;
            }
            let bytes: &[u8] = bytemuck::cast_slice(verts);
            self.stats.quads += verts.len() as u64 / 4;
            self.stats.mesh_bytes += bytes.len() as u64;
            Some(MeshBuf {
                buffer: gpu
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(label),
                        contents: bytes,
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
                quads: verts.len() as u32 / 4,
            })
        };
        let opaque = make(&mesh.opaque, "chunk opaque");
        let water = make(&mesh.water, "chunk water");
        self.stats.chunks += 1;
        GpuChunk { pos, opaque, water }
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

fn build_column(world: &dyn World, focus: &Focus, col: Col) -> ColumnResult {
    if !focus.in_range(col, UNLOAD_SLACK) {
        return ColumnResult {
            col,
            chunks: None,
            gen_ms: 0.0,
            mesh_ms: 0.0,
        };
    }
    let t0 = Instant::now();
    let voxels = world.generate_column(col.0, col.1);
    let t1 = Instant::now();
    let chunks = voxels
        .iter()
        .map(|(cy, chunk)| (*cy, mesh_chunk(chunk, world.materials())))
        .filter(|(_, mesh)| !mesh.is_empty())
        .collect();
    let t2 = Instant::now();
    ColumnResult {
        col,
        chunks: Some(chunks),
        gen_ms: (t1 - t0).as_secs_f32() * 1000.0,
        mesh_ms: (t2 - t1).as_secs_f32() * 1000.0,
    }
}
