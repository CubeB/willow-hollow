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
/// stands; 128 = standing water in the surface itself (lake, sea); 255 = river ribbon
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
    /// over the ground. The fourth byte is unused.
    pub far: [u8; 4],
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
        }
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
            let crown = features::canopy_tree(biome)
                .and_then(|species| trees::impostor(lib.get(species, 0), mats).into_iter().max_by_key(|b| b.hi.y))
                .map_or(leaves, |b| mats.get(b.voxel).color);
            Some(Canopy {
                color: crown.map(f32::from),
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
        return Look { ground: sand, far: sand, lift: 0.0 };
    }
    let biome = Biome::from_u8(c.biome[i]);
    let beach = c.height[i] < 2.5 && near_ocean(c, i);
    let mut ground = color(if beach { mat::SAND } else { mat::ground_cover(biome) });
    // The voxel world's snow line wanders by 0.015 either way of 0.10.
    let snow = if beach { 0.0 } else { ((0.115 - c.temperature[i]) / 0.03).clamp(0.0, 1.0) };
    ground = mix(ground, color(mat::SNOW), snow);
    // Ground too steep to hold soil is bare rock, a quarter of it the darker kind. The
    // grid's slopes are gentler than the voxels', hence the steeper threshold.
    let (light, dark) = mat::bedrock(biome);
    let rocky = smoothstep(1.3, 1.9, slope);
    ground = mix(ground, mix(color(light), color(dark), 0.25), rocky);

    // Trees take no root on rock, on the beach, or where it is too cold.
    let Some(canopy) = canopies[biome as usize].filter(|_| !beach) else {
        return Look { ground, far: ground, lift: 0.0 };
    };
    let grows = (1.0 - rocky) * smoothstep(0.125, 0.14, c.temperature[i]);
    Look {
        ground,
        far: mix(ground, canopy.color, canopy.cover * 0.85 * grows),
        lift: canopy.cover * canopy.height * 0.6 * grows,
    }
}

/// Whether the far surface shows water at continent-local voxel `(lx, lz)`: a lake its
/// coarse grid spreads over the shore, or land it lays below the sea. The true shore
/// can lie well inside either, so things standing there are better left out.
pub fn shows_water(c: &Continent, lx: f32, lz: f32) -> bool {
    let cs = c.cell_size();
    let max = (c.size - 1) as f32;
    let (fx, fz) = ((lx / cs - 0.5).clamp(0.0, max), (lz / cs - 0.5).clamp(0.0, max));
    let (x0, z0) = ((fx as usize).min(c.size - 2), (fz as usize).min(c.size - 2));
    let (tx, tz) = (fx - x0 as f32, fz - z0 as f32);
    let (mut lake, mut height, mut ocean) = (0.0, 0.0, false);
    for (dx, dz, w) in [(0, 0, (1.0 - tx) * (1.0 - tz)), (1, 0, tx * (1.0 - tz)), (0, 1, (1.0 - tx) * tz), (1, 1, tx * tz)] {
        let i = (z0 + dz) * c.size + x0 + dx;
        ocean |= c.ocean[i];
        if c.lake[i] != NO_LAKE {
            lake += w;
            height += w * surface::pool_level(c.desc.seed, c.lake[i]) as f32;
        } else {
            height += w * land_y(c.height[i] * OLD_VOXEL);
        }
    }
    // The shader paints water only close to a lake's own vertices; half way is generous,
    // allowing for the voxel shore not matching the grid.
    lake > 0.5 || (ocean && height < SEA_LEVEL + 1.0)
}

/// Where the sea plane lies.
pub const SEA_LEVEL: f32 = -0.125;

/// The far surface's height for land whose ground is at `h`: sunk a little, but never
/// from above the sea to below it, or low-lying land would be drowned by the sea plane.
fn land_y(h: f32) -> f32 {
    (h - SINK).max(h.min(SEA_LEVEL + 0.2))
}

fn near_ocean(c: &Continent, i: usize) -> bool {
    let s = c.size as isize;
    [-1, 1, -s, s]
        .iter()
        .any(|&o| c.ocean[(i as isize + o).clamp(0, (c.size * c.size) as isize - 1) as usize])
}

fn build_cell(c: &Continent, mats: &voxel::Materials, canopies: &[Option<Canopy>], cell: (i32, i32), stride: usize) -> CellMesh {
    let cs = c.cell_size();
    let size = c.size;
    let tiles_per_side = size / TILE;
    let n = TILE / stride;
    let water = mats.get(mat::WATER).color;
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
                    let pos = |y: f32| Vec3::new((x as f32 + 0.5) * cs, y, (z as f32 + 0.5) * cs);
                    if c.lake[idx] != NO_LAKE {
                        let p = pos(surface::pool_level(c.desc.seed, c.lake[idx]) as f32 - 0.125);
                        lo = lo.min(p);
                        hi = hi.max(p);
                        verts.push(FarVertex::water(p.to_array(), water, FLAG_WATER));
                        continue;
                    }
                    let land = look(c, mats, canopies, idx, gx.hypot(gz));
                    // Slight variation from cell to cell, standing in for the texture
                    // voxels have up close.
                    let noise = worldgen::noise::hash2(c.desc.seed ^ 0xfa2, x as i32, z as i32);
                    let f = 0.93 + worldgen::noise::unit(noise) * 0.12;
                    let srgb = |rgb: [f32; 3]| rgb.map(|v| (v * f).round().clamp(0.0, 255.0) as u8);
                    let far = srgb(land.far);
                    // Forested land stands higher by about the height of its canopy. That
                    // is carried to the shader, which raises it only beyond the tree models.
                    let flags = (land.lift / LIFT_UNIT).round().min(MAX_LIFT_FLAGS) as u8;
                    let p = pos(land_y(h(x, z)));
                    lo = lo.min(p);
                    hi = hi.max(p + Vec3::Y * land.lift);
                    let pack = |g: f32| ((g / 4.0).clamp(-1.0, 1.0) * 32767.0) as i16;
                    verts.push(FarVertex {
                        pos: p.to_array(),
                        color: srgb(land.ground),
                        flags,
                        grad: [pack(gx), pack(gz)],
                        far: [far[0], far[1], far[2], 0],
                    });
                }
            }
            tiles.push(TileMesh { verts, lo, hi });
        }
    }

    // Rivers wide enough to see from afar, as flat ribbons along their curves.
    let mut rivers = Vec::new();
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
                    FarVertex::water([q.x, y, q.y], water, FLAG_RIBBON)
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
