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
    biome::Biome,
    continent::{Continent, NO_LAKE},
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
/// stands; 128 = standing water in the surface itself (lake, sea); 255 = river ribbon
/// (drawn over the land it lies on).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct FarVertex {
    /// Position relative to the continent's origin corner.
    pub pos: [f32; 3],
    pub color: [u8; 3],
    pub flags: u8,
    /// Terrain gradient (rise per voxel along x and z), scaled by 1/4 into -1..1.
    pub grad: [i16; 2],
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
                while let Ok((cell, stride)) = request_rx.recv() {
                    let continent = world.continent(cell.0, cell.1);
                    if result_tx.send(build_cell(&continent, cell, stride)).is_err() {
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

/// Colour and flags of the far surface at a cell.
fn cell_color(c: &Continent, mats: &voxel::Materials, i: usize, slope: f32) -> ([u8; 3], u8) {
    let color = |v: voxel::Voxel| mats.get(v).color;
    if c.ocean[i] {
        // Under the sea plane; only seen through gaps, so keep it sandy.
        return (color(mat::SAND), 0);
    }
    if c.lake[i] != NO_LAKE {
        return (color(mat::WATER), FLAG_WATER);
    }
    let biome = Biome::from_u8(c.biome[i]);
    // Forest shows as its canopy colour from a distance.
    if slope <= 1.6 && c.temperature[i] >= 0.13
        && let Some((canopy, cover, _)) = worldgen::features::canopy(biome, mats)
    {
        let ground = color(mat::ground_cover(biome));
        let t = cover * 0.85;
        let mixed = [0, 1, 2].map(|k| (ground[k] as f32 * (1.0 - t) + canopy[k] as f32 * t) as u8);
        return (mixed, 0);
    }
    let v = if slope > 1.6 {
        mat::STONE
    } else if c.temperature[i] < 0.10 {
        mat::SNOW
    } else if c.height[i] < 2.5 && near_ocean(c, i) {
        mat::SAND
    } else {
        mat::ground_cover(biome)
    };
    (color(v), 0)
}

fn near_ocean(c: &Continent, i: usize) -> bool {
    let s = c.size as isize;
    [-1, 1, -s, s]
        .iter()
        .any(|&o| c.ocean[(i as isize + o).clamp(0, (c.size * c.size) as isize - 1) as usize])
}

fn build_cell(c: &Continent, cell: (i32, i32), stride: usize) -> CellMesh {
    let mats = mat::build();
    let cs = c.cell_size();
    let size = c.size;
    let tiles_per_side = size / TILE;
    let n = TILE / stride;
    let mut tiles = Vec::new();

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
                    let (mut color, mut flags) = cell_color(c, &mats, idx, gx.hypot(gz));
                    if flags == 0 {
                        // Slight variation from cell to cell, standing in for the
                        // texture voxels have up close.
                        let h = worldgen::noise::hash2(c.desc.seed ^ 0xfa2, x as i32, z as i32);
                        let f = 0.93 + worldgen::noise::unit(h) * 0.12;
                        color = color.map(|v| (v as f32 * f).min(255.0) as u8);
                    }
                    let mut lift = 0.0;
                    let y = if flags == FLAG_WATER {
                        surface::pool_level(c.desc.seed, c.lake[idx]) as f32 - 0.125
                    } else {
                        // Forested land stands higher by about the height of its canopy.
                        lift = worldgen::features::canopy(Biome::from_u8(c.biome[idx]), &mats)
                            .filter(|_| c.temperature[idx] >= 0.13 && gx.hypot(gz) <= 1.6)
                            .map_or(0.0, |(_, cover, height)| cover * height * 0.6 * OLD_VOXEL);
                        h(x, z) - SINK
                    };
                    if flags == 0 {
                        // Carried to the shader, which raises it only beyond the tree models.
                        flags = (lift / LIFT_UNIT).round().min(MAX_LIFT_FLAGS) as u8;
                    }
                    let pos = Vec3::new((x as f32 + 0.5) * cs, y, (z as f32 + 0.5) * cs);
                    lo = lo.min(pos);
                    hi = hi.max(pos + Vec3::Y * lift);
                    let pack = |g: f32| ((g / 4.0).clamp(-1.0, 1.0) * 32767.0) as i16;
                    verts.push(FarVertex {
                        pos: pos.to_array(),
                        color,
                        flags,
                        grad: if flags == FLAG_WATER { [0, 0] } else { [pack(gx), pack(gz)] },
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
        if c.ocean[i] || Continent::river_width(c.discharge[i]) * OLD_VOXEL < MIN_RIBBON_WIDTH {
            continue;
        }
        pieces.clear();
        surface::add_pieces(c, i, &mut pieces);
        for piece in &pieces {
            let segs = piece.pts.len() - 1;
            // Every other point is plenty at this distance.
            for k in (0..segs).step_by(2) {
                let (a, b) = (piece.pts[k], piece.pts[(k + 2).min(segs)]);
                let dir = (b - a).normalize_or_zero();
                let side = glam::vec2(-dir.y, dir.x);
                let (ta, tb) = (k as f32 / segs as f32, (k + 2).min(segs) as f32 / segs as f32);
                let corner = |p: glam::Vec2, t: f32, sign: f32| {
                    let q = p + side * (piece.half_width(t).max(worldgen::scale::m(1.0)) * sign);
                    let y = surface::pool_level(c.desc.seed, piece.level(t)) as f32 - 0.125;
                    FarVertex {
                        pos: [q.x, y, q.y],
                        color: water,
                        flags: FLAG_RIBBON,
                        grad: [0, 0],
                    }
                };
                // Counter-clockwise seen from above.
                rivers.extend_from_slice(&[
                    corner(a, ta, 1.0),
                    corner(b, tb, 1.0),
                    corner(b, tb, -1.0),
                    corner(a, ta, -1.0),
                ]);
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
