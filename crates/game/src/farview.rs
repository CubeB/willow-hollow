//! The far view: beyond the radius where voxels are loaded, land is drawn straight
//! from the continent simulation as a low-detail surface reaching to the horizon,
//! with lakes, the sea, and rivers as ribbons.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        mpsc::{Receiver, Sender, channel},
    },
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

impl Skylines {
    fn pack(tangent: f32) -> u8 {
        // More steps for the low angles, where the sun spends dawn and dusk.
        ((tangent.clamp(0.0, SKYLINE_MAX) / SKYLINE_MAX).sqrt() * 255.0).round() as u8
    }

    fn of(c: &Continent) -> Self {
        use rayon::prelude::*;
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
        let angles = (0..size * size)
            .into_par_iter()
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

    /// The skylines of a continent, worked out once and kept while it is in view.
    pub fn get(c: &Continent) -> Arc<Skylines> {
        static KEPT: std::sync::Mutex<Vec<((u32, i32, i32), Arc<Skylines>)>> = std::sync::Mutex::new(Vec::new());
        let key = (c.desc.world_seed, c.desc.mx, c.desc.mz);
        let mut kept = KEPT.lock().unwrap();
        if let Some((_, s)) = kept.iter().find(|(k, _)| *k == key) {
            return s.clone();
        }
        // The continent in view and its neighbours, and no more.
        if kept.len() >= 12 {
            kept.remove(0);
        }
        let made = Arc::new(Self::of(c));
        kept.push((key, made.clone()));
        made
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
    pending: HashSet<(i32, i32)>,
    requests: Sender<((i32, i32), usize)>,
    results: Receiver<CellMesh>,
    /// Index buffers for a tile grid at each stride: (stride, buffer, index count).
    grids: Vec<(usize, wgpu::Buffer, u32)>,
}

/// Detail for the continent the camera is on, and for its neighbours.
const NEAR_STRIDE: usize = 1;
const FAR_STRIDE: usize = 4;

impl FarView {
    pub fn new(gpu: &Gpu, world: Arc<GameWorld>) -> Self {
        let (requests, request_rx) = channel::<((i32, i32), usize)>();
        let (result_tx, results) = channel();
        // One background thread: simulating a continent takes seconds and must never
        // stall rendering or compete with chunk generation for every core.
        std::thread::Builder::new()
            .name("far-view".into())
            .spawn(move || {
                let canopies = canopies(&world);
                while let Ok((cell, stride)) = request_rx.recv() {
                    let continent = world.continent(cell.0, cell.1);
                    if result_tx.send(build_cell(&continent, world.materials(), &canopies, cell, stride)).is_err() {
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
            pending: HashSet::new(),
            requests,
            results,
            grids,
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

    /// True once the camera's own continent is ready to draw.
    pub fn ready(&self, camera: DVec3) -> bool {
        self.cells.contains_key(&cell_of(camera))
    }

    pub fn update(&mut self, gpu: &Gpu, camera: DVec3) {
        let here = cell_of(camera);
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
            if !have && self.pending.insert(cell) {
                let _ = self.requests.send((cell, stride));
            }
        }
        self.cells
            .retain(|c, _| (c.0 - here.0).abs() <= 1 && (c.1 - here.1).abs() <= 1);

        while let Ok(mesh) = self.results.try_recv() {
            self.pending.remove(&mesh.cell);
            // A tile the GPU has no room for is simply not drawn.
            let upload = |verts: &[FarVertex], label| gpu.try_vertex_buffer(label, bytemuck::cast_slice(verts));
            let tiles = mesh
                .tiles
                .iter()
                .filter_map(|t| {
                    Some(FarTile {
                        buffer: upload(&t.verts, "far tile")?,
                        lo: t.lo,
                        hi: t.hi,
                    })
                })
                .collect();
            let rivers = if mesh.rivers.is_empty() {
                None
            } else {
                upload(&mesh.rivers, "far rivers").map(|b| (b, mesh.rivers.len() as u32 / 4))
            };
            self.cells.insert(
                mesh.cell,
                FarCell {
                    cell: mesh.cell,
                    stride: mesh.stride,
                    tiles,
                    rivers,
                },
            );
        }
    }

    /// Blocks until the camera's continent is drawable (for screenshots).
    pub fn wait_ready(&mut self, gpu: &Gpu, camera: DVec3) {
        while !self.ready(camera) {
            self.update(gpu, camera);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
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
}

/// The land at a cell as the far surface draws it. The ground is the one the voxel world
/// gives it (see `world::surface_materials`), each biome with its own soil and rock, the
/// voxel-to-voxel speckle and ragged edges between them averaged out as distance does.
fn look(c: &Continent, mats: &voxel::Materials, canopies: &[Option<Canopy>], i: usize, slope: f32) -> Look {
    let color = |v: voxel::Voxel| mats.get(v).color.map(f32::from);
    if c.ocean[i] {
        // Under the sea plane; only seen through gaps, so keep it sandy.
        let sand = color(mat::SAND);
        return Look { ground: sand, far: sand, plant: 0, lift: 0.0 };
    }
    let biome = Biome::from_u8(c.biome[i]);
    // Sand as far in from the shore as the voxel world's beach reaches, which the
    // triangles shade into the land around it.
    let cs = c.cell_size();
    let centre = glam::Vec2::new((i % c.size) as f32 + 0.5, (i / c.size) as f32 + 0.5) * cs;
    let sand = surface::beach_share(c, centre, c.height[i] * OLD_VOXEL);
    let beach = sand > 0.5;
    let mut ground = mix(color(mat::ground_cover(biome)), color(mat::SAND), sand);
    // The voxel world's snow line wanders by 0.015 either way of 0.10.
    let snow = ((0.115 - c.temperature[i]) / 0.03).clamp(0.0, 1.0) * (1.0 - sand);
    ground = mix(ground, color(mat::SNOW), snow);
    // Ground too steep to hold soil is bare rock, a quarter of it the darker kind. The
    // grid's slopes are gentler than the voxels', hence the steeper threshold.
    let (light, dark) = mat::bedrock(biome);
    let rocky = smoothstep(1.3, 1.9, slope);
    ground = mix(ground, mix(color(light), color(dark), 0.25), rocky);

    // Trees take no root on rock, on the beach, or where it is too cold.
    let Some(canopy) = canopies[biome as usize].filter(|_| !beach) else {
        return Look { ground, far: ground, plant: 0, lift: 0.0 };
    };
    let grows = (1.0 - rocky) * smoothstep(0.125, 0.14, c.temperature[i]);
    Look {
        ground,
        far: mix(ground, canopy.color, canopy.cover * 0.85 * grows),
        plant: pack_plant(canopy.plant, canopy.winter_green),
        lift: canopy.cover * canopy.height * 0.6 * grows,
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
/// the continent is not made yet.
pub fn skylines_at(world: &GameWorld, x: i32, z: i32) -> [u8; 2] {
    let (mx, mz) = worldgen::macro_layout::cell_of(x, z);
    world.continent_if_ready(mx, mz).map_or([0; 2], |c| {
        Skylines::get(&c).blended((x - mx * MACRO_SIZE) as f32, (z - mz * MACRO_SIZE) as f32)
    })
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

/// Each grid cell of `c` as a map shows it, sRGB: the far view's land seen from above
/// (forests as their canopy), lakes, the sea and the rivers it draws as ribbons.
pub fn map_colors(world: &GameWorld, c: &Continent) -> Vec<[f32; 3]> {
    let canopies = canopies(world);
    let mats = world.materials();
    let size = c.size;
    let cs = c.cell_size();
    let water = mats.get(mat::WATER).color.map(f32::from);
    let sea = [32.0, 78.0, 128.0];
    (0..size * size)
        .map(|i| {
            let (x, z) = (i % size, i / size);
            if c.ocean[i] {
                return sea;
            }
            if ribbon_at(c, i) || wetness(c, x, z, 1).1 > 0.0 {
                return water;
            }
            let h = |x: usize, z: usize| c.height[z.min(size - 1) * size + x.min(size - 1)] * OLD_VOXEL;
            let gx = (h(x + 1, z) - h(x.saturating_sub(1), z)) / (2.0 * cs);
            let gz = (h(x, z + 1) - h(x, z.saturating_sub(1))) / (2.0 * cs);
            look(c, mats, &canopies, i, gx.hypot(gz)).far
        })
        .collect()
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

fn build_cell(c: &Continent, mats: &voxel::Materials, canopies: &[Option<Canopy>], cell: (i32, i32), stride: usize) -> CellMesh {
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
                    let pos = |y: f32| Vec3::new((x as f32 + 0.5) * cs, y, (z as f32 + 0.5) * cs);
                    let (level, depth) = wetness(c, x, z, stride);
                    let mut land = look(c, mats, canopies, idx, gx.hypot(gz));
                    if depth > 0.0 {
                        // Nothing grows in the water.
                        land.far = land.ground;
                        land.lift = 0.0;
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
    }
}
