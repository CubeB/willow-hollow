//! Distant trees: beyond the coarse voxel rings, each tree is a few boxes built from
//! its real model, drawn instanced. They reach out as far as trees can be told apart
//! from the forest colour of the far view.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        mpsc::{Receiver, Sender, channel},
    },
};

use bytemuck::{Pod, Zeroable};
use glam::{DVec3, IVec3};
use worldgen::{
    World,
    features::{self, shape::rotate, trees::{self, Species}},
    macro_layout::{MACRO_SIZE, cell_of},
    surface::Sampler,
    world::GameWorld,
};

use crate::{
    arena::{MeshSpan, VertexArena},
    gfx::Gpu,
    lod,
};

/// Voxels along the edge of a square of trees loaded together.
const TILE: i32 = 256;
/// How far from the camera trees are drawn, in voxels.
pub const REACH: f32 = worldgen::scale::m(2800.0);
/// Squares stay loaded this many squares beyond the reach.
const UNLOAD_SLACK: i32 = 2;
/// Squares being built at once.
const MAX_IN_FLIGHT: usize = 6;
/// Frames between looking again at squares whose continent wasn't simulated yet.
const RETRY_FRAMES: u32 = 120;

/// One box of one tree, as the GPU reads it.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct BoxInstance {
    /// World voxel position of the box's low corner.
    pub lo: [i32; 3],
    pub size: [u8; 3],
    pub _pad: u8,
    pub color: [u8; 3],
    pub _pad2: u8,
}

/// The boxes of a model, turned, relative to where it stands.
type ModelBoxes = Arc<Vec<(IVec3, [u8; 3], [u8; 3])>>;
type ModelKey = (Species, usize, u32);

#[derive(Default)]
struct Models {
    cache: Mutex<HashMap<ModelKey, ModelBoxes>>,
}

impl Models {
    fn get(&self, world: &GameWorld, species: Species, variant: usize, turns: u32) -> ModelBoxes {
        let key = (species, variant % species.variants(), turns & 3);
        if let Some(model) = self.cache.lock().expect("model cache poisoned").get(&key) {
            return model.clone();
        }
        let mats = world.materials();
        let boxes = trees::impostor(world.library().get(species, variant), mats)
            .into_iter()
            .map(|b| {
                let (a, c) = (rotate(b.lo, turns), rotate(b.hi - IVec3::ONE, turns));
                let (lo, hi) = (a.min(c), a.max(c) + IVec3::ONE);
                let size = (hi - lo).min(IVec3::splat(255));
                (lo, [size.x as u8, size.y as u8, size.z as u8], mats.get(b.voxel).color)
            })
            .collect();
        let model: ModelBoxes = Arc::new(boxes);
        self.cache.lock().expect("model cache poisoned").insert(key, model.clone());
        model
    }
}

/// A square of trees on the GPU.
pub struct Tile {
    pub span: MeshSpan,
    /// Bounds in world voxels.
    pub lo: IVec3,
    pub hi: IVec3,
}

struct Built {
    tile: (i32, i32),
    /// None if its continent hasn't been simulated yet.
    boxes: Option<Vec<BoxInstance>>,
}

pub struct Impostors {
    world: Arc<GameWorld>,
    models: Arc<Models>,
    pool: rayon::ThreadPool,
    tx: Sender<Built>,
    rx: Receiver<Built>,
    tiles: HashMap<(i32, i32), Option<Tile>>,
    pending: HashSet<(i32, i32)>,
    /// Still to build, farthest first.
    wanted: Vec<(i32, i32)>,
    /// Waiting for their continent.
    waiting: Vec<(i32, i32)>,
    retry: u32,
    center: Option<(i32, i32)>,
    arena: VertexArena,
    /// Boxes loaded, for statistics.
    pub boxes: u64,
}

impl Impostors {
    pub fn new(gpu: &Gpu, world: Arc<GameWorld>) -> Self {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(2)
            .thread_name(|i| format!("impostors-{i}"))
            .build()
            .expect("thread pool");
        let (tx, rx) = channel();
        Self {
            world,
            models: Arc::default(),
            pool,
            tx,
            rx,
            tiles: HashMap::new(),
            pending: HashSet::new(),
            wanted: Vec::new(),
            waiting: Vec::new(),
            retry: 0,
            center: None,
            arena: VertexArena::new(gpu, "tree impostors", 32 << 20, size_of::<BoxInstance>() as u64),
            boxes: 0,
        }
    }

    pub fn buffer(&self) -> &wgpu::Buffer {
        self.arena.buffer()
    }

    pub fn tiles(&self) -> impl Iterator<Item = &Tile> {
        self.tiles.values().flatten()
    }

    fn in_reach(tile: (i32, i32), center: (i32, i32), slack: i32) -> bool {
        let (dx, dz) = (tile.0 - center.0, tile.1 - center.1);
        let r = (REACH / TILE as f32) as i32 + slack;
        dx * dx + dz * dz <= r * r
    }

    pub fn update(&mut self, gpu: &Gpu, camera: DVec3) {
        let here = (
            (camera.x / TILE as f64).floor() as i32,
            (camera.z / TILE as f64).floor() as i32,
        );
        if self.center != Some(here) {
            self.center = Some(here);
            self.rebuild_wanted(here);
        }
        self.retry += 1;
        if self.retry >= RETRY_FRAMES && !self.waiting.is_empty() {
            self.retry = 0;
            self.wanted.append(&mut self.waiting);
            self.sort_wanted(here);
        }

        while self.pending.len() < MAX_IN_FLIGHT {
            let Some(tile) = self.wanted.pop() else { break };
            self.pending.insert(tile);
            let (world, models, tx) = (self.world.clone(), self.models.clone(), self.tx.clone());
            self.pool.spawn(move || {
                let boxes = build_tile(&world, &models, tile);
                let _ = tx.send(Built { tile, boxes });
            });
        }

        while let Ok(built) = self.rx.try_recv() {
            self.pending.remove(&built.tile);
            if !Self::in_reach(built.tile, here, UNLOAD_SLACK) {
                continue;
            }
            let Some(boxes) = built.boxes else {
                self.waiting.push(built.tile);
                continue;
            };
            let tile = self.upload(gpu, built.tile, &boxes);
            self.boxes += boxes.len() as u64;
            self.tiles.insert(built.tile, tile);
        }
    }

    fn upload(&mut self, gpu: &Gpu, tile: (i32, i32), boxes: &[BoxInstance]) -> Option<Tile> {
        if boxes.is_empty() {
            return None;
        }
        // Trees the GPU has no room for are simply not drawn.
        let span = self.arena.add(gpu, boxes)?;
        let (mut lo_y, mut hi_y) = (i32::MAX, i32::MIN);
        for b in boxes {
            lo_y = lo_y.min(b.lo[1]);
            hi_y = hi_y.max(b.lo[1] + b.size[1] as i32);
        }
        Some(Tile {
            span,
            lo: IVec3::new(tile.0 * TILE, lo_y, tile.1 * TILE),
            hi: IVec3::new((tile.0 + 1) * TILE, hi_y, (tile.1 + 1) * TILE),
        })
    }

    fn rebuild_wanted(&mut self, here: (i32, i32)) {
        let arena = &mut self.arena;
        let mut dropped = 0u64;
        self.tiles.retain(|t, tile| {
            let keep = Self::in_reach(*t, here, UNLOAD_SLACK);
            if !keep && let Some(tile) = tile {
                dropped += tile.span.quads as u64;
                arena.remove(tile.span);
            }
            keep
        });
        self.boxes = self.boxes.saturating_sub(dropped);
        self.waiting.retain(|t| Self::in_reach(*t, here, UNLOAD_SLACK));
        self.wanted.clear();
        let r = (REACH / TILE as f32) as i32;
        for dx in -r..=r {
            for dz in -r..=r {
                let t = (here.0 + dx, here.1 + dz);
                if Self::in_reach(t, here, 0) && !self.tiles.contains_key(&t) && !self.pending.contains(&t) && !self.waiting.contains(&t) {
                    self.wanted.push(t);
                }
            }
        }
        self.sort_wanted(here);
    }

    fn sort_wanted(&mut self, here: (i32, i32)) {
        self.wanted.sort_unstable_by_key(|t| {
            let (dx, dz) = ((t.0 - here.0) as i64, (t.1 - here.1) as i64);
            std::cmp::Reverse(dx * dx + dz * dz)
        });
    }

    /// Blocks until every square that can be built has been (for screenshots).
    pub fn wait_idle(&mut self, gpu: &Gpu, camera: DVec3) {
        loop {
            self.update(gpu, camera);
            if self.wanted.is_empty() && self.pending.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
}

fn build_tile(world: &GameWorld, models: &Models, tile: (i32, i32)) -> Option<Vec<BoxInstance>> {
    let (x0, z0) = (tile.0 * TILE, tile.1 * TILE);
    let (mx, mz) = cell_of(x0, z0);
    let continent = world.continent_if_ready(mx, mz)?;
    let (lx0, lz0) = (x0 - mx * MACRO_SIZE, z0 - mz * MACRO_SIZE);
    let margin = features::MAX_REACH + 1;
    let sampler = Sampler::new(&continent, world.seed(), lx0 - margin, lz0 - margin, TILE + 2 * margin);
    let lib = world.library();
    let mut boxes = Vec::new();
    for p in features::place(world.seed(), lib, &sampler, lx0, lz0, TILE, (mx * MACRO_SIZE, mz * MACRO_SIZE)) {
        // Each tree belongs to the square its trunk is in.
        let inside = (x0..x0 + TILE).contains(&p.pos.x) && (z0..z0 + TILE).contains(&p.pos.z);
        if inside && lod::stands_tall(p.species) {
            for (lo, size, color) in models.get(world, p.species, p.variant, p.turns).iter() {
                let at = p.pos + *lo;
                boxes.push(BoxInstance {
                    lo: at.to_array(),
                    size: *size,
                    _pad: 0,
                    color: *color,
                    _pad2: 0,
                });
            }
        }
    }
    Some(boxes)
}
