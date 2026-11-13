//! The large map's model: the land the player has seen, as a slab of blocks seen from
//! above and to one side, with its water, paths and woods standing on it.
//!
//! The model is a square of `N` columns each way around the middle of the map, each
//! column `step` voxels wide, a power of two chosen from the zoom. Close in, every
//! column is read from the voxel world's own surface, so the map shows the banks,
//! cliffs and beaches that are there; further out, where a column is a grid cell of the
//! continent or more, it is read from the continent's picture. Heights are drawn taller
//! the further out the map is, or a range would be a ripple. Columns not yet seen are
//! left out, which gives the slab its ragged edge.
//!
//! Built on the map's relief thread, for [`super::gpu::ReliefGpu`] to draw.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

use bytemuck::{Pod, Zeroable};
use glam::{IVec3, Vec2};
use voxel::Kind;
use worldgen::{
    World,
    continent::{Continent, NO_LAKE},
    features::{
        self, Placement,
        shape::rotate,
        trees::{self, Species},
    },
    macro_layout::MACRO_SIZE,
    materials as mat,
    scale::OLD_VOXEL,
    surface::Sampler,
    trails::build::{TrailClass, class_of},
    world::GameWorld,
};

use super::{Key, picture::TEX};
use crate::farview;

/// Columns per side of the large map's model, and of the corner map's. Whole tiles.
pub const N: usize = 384;
pub const SMALL: usize = 192;
/// The narrowest and widest a column is, in voxels.
pub const MIN_STEP: i32 = 16;
pub const MAX_STEP: i32 = 512;
/// The width of the corner map's columns, in voxels: fine enough to tell tree from tree.
pub const FINE_STEP: i32 = 4;
/// The widest columns the world's own trees are drawn on one by one, each where it stands
/// and in its own shape. On wider columns read from the voxel world, a column a tree
/// stands in has a tree on it; on those read from the continent, a wood is a scatter of
/// trees as thick as the biome's.
pub const TREE_STEP: i32 = MIN_STEP;
/// How tall a tree is drawn as tall as it is, in voxels; above that, it is drawn shorter.
const TREE_TALL: f32 = worldgen::scale::m(12.0);
/// Columns to a side of the squares the voxel surface is sampled in.
const TILE: usize = 32;
/// A column nobody has seen.
pub const UNSEEN: i16 = i16::MIN;
/// How many blocks of earth the slab shows under its lowest ground.
const FOOT: i16 = 3;

/// Walked ground, and a path, as they colour the column they cross (sRGB).
const WALKED: [f32; 3] = [252.0, 246.0, 228.0];
const PATH: [f32; 3] = [206.0, 178.0, 118.0];
const EARTH: [f32; 3] = [104.0, 84.0, 64.0];
const TRUNK: [u8; 3] = [92, 70, 50];

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Vertex {
    /// Columns east and south of the model's corner, and blocks up.
    pub pos: [f32; 3],
    /// sRGB, then which way the face looks: 0 up, 1 east, 2 west, 3 south, 4 north.
    pub color: [u8; 4],
}

pub struct Request {
    /// Which of the map's models this is for; handed back with it.
    pub tag: u8,
    /// Columns per side.
    pub n: usize,
    pub step: i32,
    /// The model's north-west corner, in columns of `step` voxels from the world's origin.
    pub origin: (i32, i32),
    /// Seen and walked for each column, row by row.
    pub mask: Vec<[u8; 2]>,
    /// The continents the model lies over, with their pictures.
    pub sheets: Vec<(Key, Arc<Continent>, Arc<Vec<u8>>)>,
    /// What the map had seen when this was asked for.
    pub version: u64,
}

pub struct Relief {
    pub tag: u8,
    pub n: usize,
    pub step: i32,
    pub origin: (i32, i32),
    pub version: u64,
    /// The top of each column in blocks, or [`UNSEEN`].
    pub tops: Vec<i16>,
    /// The middle height of what is shown, in blocks: where the map looks.
    pub level: f32,
    /// Where the trees drawn one by one stand, world voxels x and z of each trunk.
    pub trees: Vec<(i32, i32)>,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl Relief {
    /// The top of the column holding world voxel `(x, z)`, in voxels, if it is shown.
    pub fn top_at(&self, x: f64, z: f64) -> Option<f32> {
        let s = self.step as f64;
        let (cx, cz) = ((x / s).floor() as i64 - self.origin.0 as i64, (z / s).floor() as i64 - self.origin.1 as i64);
        if cx < 0 || cz < 0 || cx >= self.n as i64 || cz >= self.n as i64 {
            return None;
        }
        let t = self.tops[cz as usize * self.n + cx as usize];
        (t != UNSEEN).then(|| t as f32 * self.step as f32)
    }
}

/// How much taller than life the land is drawn with columns `step` voxels wide.
pub fn tallness(step: i32) -> f32 {
    (step as f32 / MIN_STEP as f32).powf(0.45).max(1.0)
}

#[derive(Clone, Copy)]
struct Tree {
    color: [u8; 3],
    /// Blocks tall.
    height: f32,
    conifer: bool,
    /// How many columns wide the sizes meant for the coarsest near columns come to.
    spread: f32,
    /// For one of the world's trees: where in its column it stands, east and south,
    /// 0 to 1, and how far its crown reaches, in columns.
    stands: Option<(f32, f32, f32)>,
}

#[derive(Clone, Copy)]
struct Cell {
    top: i16,
    color: [f32; 3],
    water: bool,
    tree: Option<Tree>,
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * t)
}

/// 0..1, the same for a column however it is reached.
fn hash(x: i32, z: i32, salt: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x27d4_eb2d) ^ (z as u32).wrapping_mul(0x1656_67b1).wrapping_add(salt.wrapping_mul(0x9e37_79b9));
    h = (h ^ (h >> 15)).wrapping_mul(0x2c1b_3c6d);
    h = (h ^ (h >> 12)).wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    (h & 0xffff) as f32 / 65536.0
}

fn conifer(species: Option<Species>) -> bool {
    matches!(species, Some(Species::Spruce | Species::TallPine | Species::Redwood | Species::ScotsPine | Species::Juniper))
}

pub fn build(world: &GameWorld, req: &Request) -> Relief {
    let step = req.step;
    let n = req.n;
    debug_assert!(n % TILE == 0 && req.mask.len() == n * n);
    let tall = tallness(step);
    let block = |voxels: f32| (voxels * tall / step as f32).round().clamp(-200.0, 2000.0) as i16;
    let mats = world.materials();
    let water = mats.get(mat::WATER).color.map(f32::from);
    let sheet = |key: Key| req.sheets.iter().find(|s| s.0 == key);
    let mut cells: Vec<Option<Cell>> = vec![None; n * n];
    // World voxel at the middle of a column.
    let mid = |c: usize, o: i32| (o + c as i32) * step + step / 2;
    let key_of = |x: i32, z: i32| (x.div_euclid(MACRO_SIZE), z.div_euclid(MACRO_SIZE));

    let spread = (MIN_STEP as f32 / step as f32).max(1.0);
    // How tall a tree `y` voxels tall is drawn, in voxels: the map is looked down on at a
    // slant, so the tallest, the redwoods and kapoks, are drawn shorter than life, or one
    // would lie across half the corner map.
    let squash = |y: i32| if y as f32 <= TREE_TALL { y as f32 } else { TREE_TALL + (y as f32 - TREE_TALL) * 0.35 };
    // The trees standing on the land shown, as the world has them, close in.
    let mut stands: Vec<Placement> = Vec::new();
    if step < MACRO_SIZE / TEX as i32 {
        // From the voxel world's surface, a square of columns at a time.
        for tz in (0..n).step_by(TILE) {
            for tx in (0..n).step_by(TILE) {
                let seen = |x: usize, z: usize| req.mask[z * n + x][0] >= 128;
                if !(tz..tz + TILE).any(|z| (tx..tx + TILE).any(|x| seen(x, z))) {
                    continue;
                }
                let key = key_of(mid(tx + TILE / 2, req.origin.0), mid(tz + TILE / 2, req.origin.1));
                let Some((_, c, _)) = sheet(key) else { continue };
                let (ox, oz) = (key.0 * MACRO_SIZE, key.1 * MACRO_SIZE);
                let (lx0, lz0) = ((req.origin.0 + tx as i32) * step - ox, (req.origin.1 + tz as i32) * step - oz);
                let span = TILE as i32 * step;
                // Out as far as a tree that could stand in the square reads the ground, so
                // that each comes out as the world grows it.
                let margin = features::MAX_REACH + 1;
                let sampler = Sampler::new(c, world.seed(), lx0 - margin, lz0 - margin, span + 2 * margin);
                let tors = world.tors_near(c, ox + lx0, oz + lz0, span);
                for p in features::place_trees(world.seed(), world.library(), &sampler, lx0, lz0, span, (ox, oz)) {
                    // Each tree belongs to the square its trunk is in; the world leaves out
                    // those a tor's rock would be in.
                    let inside = (0..span).contains(&(p.pos.x - ox - lx0)) && (0..span).contains(&(p.pos.z - oz - lz0));
                    if inside && crate::lod::stands_tall(p.species) && !tors.iter().any(|t| t.crowds(p.pos)) {
                        stands.push(p);
                    }
                }
                for z in tz..tz + TILE {
                    for x in tx..tx + TILE {
                        let (wx, wz) = (mid(x, req.origin.0), mid(z, req.origin.1));
                        if !seen(x, z) || key_of(wx, wz) != key {
                            continue;
                        }
                        let col = sampler.column(wx - ox, wz - oz);
                        let flooded = col.water.filter(|&w| col.ground < (w - 1) as f32);
                        let cell = if let Some(w) = flooded {
                            Cell { top: block((w - 1) as f32), color: water, water: true, tree: None }
                        } else if col.river < step as f32 * 0.3 {
                            // A stream too narrow to fill the column still shows as a thread.
                            Cell { top: block(col.ground), color: mix(water, [150.0, 190.0, 210.0], 0.25), water: true, tree: None }
                        } else {
                            let (surface, _) = crate::lod::skin(&col, 0.0);
                            // The trees are the world's own (`stands`), put on after.
                            Cell { top: block(col.ground), color: mats.get(surface).color.map(f32::from), water: false, tree: None }
                        };
                        cells[z * n + x] = Some(cell);
                    }
                }
            }
        }
        // The paths, drawn through from point to point so that none is left in dots.
        let mut path = vec![false; n * n];
        for (key, c, _) in &req.sheets {
            let o = Vec2::new((key.0 * MACRO_SIZE) as f32, (key.1 * MACRO_SIZE) as f32) / step as f32 - Vec2::new(req.origin.0 as f32, req.origin.1 as f32);
            for (line, widths) in c.trails.lines.iter().zip(&c.trails.line_widths) {
                for (pair, w) in line.windows(2).zip(widths) {
                    if class_of(*w) == TrailClass::Trace {
                        continue;
                    }
                    let (a, b) = (pair[0] / step as f32 + o, pair[1] / step as f32 + o);
                    let steps = ((b - a).abs().max_element() * 2.0).ceil().max(1.0) as usize;
                    for k in 0..=steps {
                        let p = a.lerp(b, k as f32 / steps as f32);
                        if p.x >= 0.0 && p.y >= 0.0 && (p.x as usize) < n && (p.y as usize) < n {
                            path[p.y as usize * n + p.x as usize] = true;
                        }
                    }
                }
            }
        }
        for (cell, on) in cells.iter_mut().zip(path) {
            if let Some(cell) = cell
                && on
                && !cell.water
            {
                cell.color = mix(cell.color, PATH, 0.8);
                cell.tree = None;
            }
        }
        if step > TREE_STEP {
            // Columns as wide as a tree's crown or wider: a tree on each column one of the
            // world's trees stands in, of its kind, its leaves and about its height, so
            // the woods, their edges and their clearings are where they are.
            for p in std::mem::take(&mut stands) {
                let (cx, cz) = (p.pos.x.div_euclid(step) - req.origin.0, p.pos.z.div_euclid(step) - req.origin.1);
                if cx < 0 || cz < 0 || cx >= n as i32 || cz >= n as i32 {
                    continue;
                }
                let Some(cell) = cells[cz as usize * n + cx as usize].as_mut().filter(|c| !c.water) else { continue };
                let model = world.library().get(p.species, p.variant);
                let height = squash(model.top) * (1.0 + 0.6 / spread) / step as f32;
                // The tallest standing in the column is the one seen.
                if cell.tree.is_some_and(|t| t.height >= height) {
                    continue;
                }
                let parts = shape(world, p.species, p.variant, false);
                let color = parts.first().map_or(TRUNK, |part| part.color);
                // Half the crown's breadth, as wide as the world's tree, but not so thin it
                // is lost.
                let reach = parts.iter().map(|q| (q.hi.x - q.lo.x).max(q.hi.z - q.lo.z)).max().unwrap_or(4) as f32 * 0.5 / step as f32;
                let within = |v: i32| (v.rem_euclid(step) as f32 + 0.5) / step as f32;
                let stands = Some((within(p.pos.x), within(p.pos.z), reach.max(0.22)));
                cell.tree = Some(Tree { color, height: height.clamp(1.0, 9.0 * spread), conifer: conifer(Some(p.species)), spread, stands });
            }
        }
    } else {
        // From the continent's grid and its picture.
        for z in 0..n {
            for x in 0..n {
                if req.mask[z * n + x][0] < 128 {
                    continue;
                }
                let (wx, wz) = (mid(x, req.origin.0), mid(z, req.origin.1));
                let key = key_of(wx, wz);
                let Some((_, c, picture)) = sheet(key) else { continue };
                let size = c.size;
                let cs = c.cell_size();
                let grid = |v: i32| (((v as f32) / cs) as usize).min(size - 1);
                let (gx, gz) = (grid(wx - key.0 * MACRO_SIZE), grid(wz - key.1 * MACRO_SIZE));
                let i = gz * size + gx;
                let t = ((gz * TEX / size) * TEX + gx * TEX / size) * 4;
                let color = [picture[t] as f32, picture[t + 1] as f32, picture[t + 2] as f32];
                let cell = if c.ocean[i] {
                    Cell { top: 0, color, water: true, tree: None }
                } else if farview::lake_at(c, gx, gz) {
                    let level = if c.lake[i] != NO_LAKE { c.lake[i] } else { c.height[i] };
                    Cell { top: block(level * OLD_VOXEL).max(1), color, water: true, tree: None }
                } else {
                    let biome = worldgen::biome::Biome::from_u8(c.biome[i]);
                    let tree = features::canopy(biome, mats).filter(|t| hash(wx, wz, 1) < t.1 * 0.6).map(|(color, _, _)| Tree {
                        color,
                        height: 0.45 + 0.4 * hash(wx, wz, 2),
                        conifer: false,
                        spread: 1.0,
                        stands: None,
                    });
                    // The middle height of the ground the column covers, or a range seen
                    // from far off would be a row of spikes.
                    let reach = ((step as f32 / cs) as usize / 2).min(4);
                    let (mut sum, mut count) = (0.0, 0);
                    for qz in gz.saturating_sub(reach)..=(gz + reach).min(size - 1) {
                        for qx in gx.saturating_sub(reach)..=(gx + reach).min(size - 1) {
                            sum += c.height[qz * size + qx].max(0.0);
                            count += 1;
                        }
                    }
                    Cell { top: block(sum / count as f32 * OLD_VOXEL).max(1), color, water: false, tree }
                };
                cells[z * n + x] = Some(cell);
            }
        }
    }

    // Where the player has walked, lighter.
    for (cell, m) in cells.iter_mut().zip(&req.mask) {
        if let Some(cell) = cell
            && m[1] >= 160
            && !cell.water
        {
            cell.color = mix(cell.color, WALKED, 0.38);
        }
    }

    let tops: Vec<i16> = cells.iter().map(|c| c.map_or(UNSEEN, |c| c.top)).collect();
    let shown = tops.iter().filter(|&&t| t != UNSEEN);
    let lowest = shown.clone().min().copied().unwrap_or(0);
    let count = shown.clone().count().max(1);
    let level = shown.map(|&t| t as f32).sum::<f32>() / count as f32;
    let base = lowest - FOOT;

    let mut mesh = Mesh { vertices: Vec::with_capacity(count * 12), indices: Vec::with_capacity(count * 18) };
    for z in 0..n {
        for x in 0..n {
            let Some(cell) = cells[z * n + x] else { continue };
            let (wx, wz) = (req.origin.0 + x as i32, req.origin.1 + z as i32);
            // Each block a touch lighter or darker than the next, as cut blocks are.
            let grain = if cell.water { 1.0 } else { 0.965 + 0.07 * hash(wx, wz, 3) };
            let color = cell.color.map(|v| v * grain);
            let (x0, z0, y) = (x as f32, z as f32, cell.top as f32);
            mesh.quad([[x0, y, z0], [x0 + 1.0, y, z0], [x0 + 1.0, y, z0 + 1.0], [x0, y, z0 + 1.0]], color, 0);
            let under = if cell.water { color.map(|v| v * 0.8) } else { mix(color, EARTH, 0.65) };
            let beside = |dx: i32, dz: i32| {
                let (nx, nz) = (x as i32 + dx, z as i32 + dz);
                if nx < 0 || nz < 0 || nx >= n as i32 || nz >= n as i32 {
                    return base;
                }
                match tops[nz as usize * n + nx as usize] {
                    UNSEEN => base,
                    t => t,
                }
            };
            for (face, dx, dz) in [(1u8, 1, 0), (2, -1, 0), (3, 0, 1), (4, 0, -1)] {
                let low = beside(dx, dz);
                if low >= cell.top {
                    continue;
                }
                // The edge of the column this face stands on, seen from outside.
                let (a, b) = match face {
                    1 => ([x0 + 1.0, z0 + 1.0], [x0 + 1.0, z0]),
                    2 => ([x0, z0], [x0, z0 + 1.0]),
                    3 => ([x0, z0 + 1.0], [x0 + 1.0, z0 + 1.0]),
                    _ => ([x0 + 1.0, z0], [x0, z0]),
                };
                let mut wall = |top: f32, bottom: f32, color: [f32; 3]| {
                    mesh.quad([[a[0], top, a[1]], [b[0], top, b[1]], [b[0], bottom, b[1]], [a[0], bottom, a[1]]], color, face);
                };
                let turf = (y - 1.0).max(low as f32);
                wall(y, turf, color);
                if (low as f32) < turf {
                    wall(turf, low as f32, under);
                }
            }
            if let Some(tree) = cell.tree {
                let shade = 0.9 + 0.2 * hash(wx, wz, 4);
                let leaf = tree.color.map(|v| v as f32 * shade);
                let k = tree.spread;
                let (mut cx, mut cz) = (x0 + 0.5 + 0.3 * k * (hash(wx, wz, 5) - 0.5), z0 + 0.5 + 0.3 * k * (hash(wx, wz, 6) - 0.5));
                let mut width = (0.42 + 0.22 * hash(wx, wz, 7)) * k;
                if let Some((sx, sz, reach)) = tree.stands {
                    (cx, cz, width) = (x0 + sx, z0 + sz, reach);
                }
                let h = tree.height;
                if h < 1.0 {
                    // A wood too far off to tell its trees apart: a bump of canopy.
                    mesh.block(cx, cz, 0.42, y, y + h, leaf);
                } else if tree.conifer {
                    let skirt = y + (0.3 * k).min(h * 0.2);
                    let waist = skirt + (y + h - skirt) * 0.5;
                    mesh.block(cx, cz, 0.12 * k, y, skirt, TRUNK.map(f32::from));
                    mesh.block(cx, cz, width, skirt, waist, leaf);
                    mesh.block(cx, cz, width * 0.57, waist, y + h, leaf.map(|v| v * 1.06));
                } else {
                    let crown = (h * 0.62).max(0.9).min(h);
                    if h - crown > 0.05 {
                        mesh.block(cx, cz, 0.14 * k, y, y + h - crown, TRUNK.map(f32::from));
                    }
                    if k > 1.0 {
                        // Big enough to have a shape: a broad crown with a smaller one on it.
                        let shoulder = y + h - crown * 0.35;
                        mesh.block(cx, cz, width, y + h - crown, shoulder, leaf);
                        mesh.block(cx, cz, width * 0.62, shoulder, y + h, leaf.map(|v| v * 1.07));
                    } else {
                        mesh.block(cx, cz, width, y + h - crown, y + h, leaf);
                    }
                }
            }
        }
    }

    // The world's own trees, each on the column its trunk stands in, as tall and as wide
    // as it is and in its own leaves.
    let corner = (req.origin.0 * step, req.origin.1 * step);
    let mut trees = Vec::with_capacity(stands.len());
    for p in &stands {
        let (cx, cz) = (p.pos.x.div_euclid(step) - req.origin.0, p.pos.z.div_euclid(step) - req.origin.1);
        if cx < 0 || cz < 0 || cx >= n as i32 || cz >= n as i32 {
            continue;
        }
        let Some(cell) = cells[cz as usize * n + cx as usize] else { continue };
        trees.push((p.pos.x, p.pos.z));
        let shade = 0.9 + 0.2 * hash(p.pos.x, p.pos.z, 4);
        // From voxels about the trunk to the model's columns and blocks.
        let at = |v: IVec3| {
            [
                (p.pos.x - corner.0 + v.x) as f32 / step as f32,
                cell.top as f32 + squash(v.y) * tall / step as f32,
                (p.pos.z - corner.1 + v.z) as f32 / step as f32,
            ]
        };
        for part in shape(world, p.species, p.variant, step < MIN_STEP).iter() {
            // Turned as the tree stands.
            let (a, b) = (rotate(part.lo, p.turns), rotate(part.hi - IVec3::ONE, p.turns));
            let (mut lo, mut hi) = (at(a.min(b)), at(a.max(b) + IVec3::ONE));
            // Never so thin that it is lost between the columns.
            for k in [0, 2] {
                let (mid, half) = ((lo[k] + hi[k]) * 0.5, ((hi[k] - lo[k]) * 0.5).max(0.2));
                (lo[k], hi[k]) = (mid - half, mid + half);
            }
            hi[1] = hi[1].max(lo[1] + 0.25);
            let color = part.color.map(|v| v as f32 * if part.leaf { shade * part.light } else { 1.0 });
            mesh.cuboid(lo, hi, color);
        }
    }
    Relief { tag: req.tag, step, n, origin: req.origin, version: req.version, tops, level, trees, vertices: mesh.vertices, indices: mesh.indices }
}

/// One box of a tree as the map draws it, in voxels about where its trunk stands,
/// unturned: its low and high corners, its colour, whether it is leaves, and how much
/// lighter than the rest it is drawn (the top of a crown, in the sun).
struct Part {
    lo: IVec3,
    hi: IVec3,
    color: [u8; 3],
    leaf: bool,
    light: f32,
}

/// The boxes a tree model is drawn with on the map, worked out once each from the model
/// itself, as the far trees' are (trees::impostor). `fine` keeps them all, its trunk and
/// each layer of its crown; otherwise, where a column is as wide as a trunk is tall, its
/// crown is drawn alone, in two layers.
fn shape(world: &GameWorld, species: Species, variant: usize, fine: bool) -> Arc<Vec<Part>> {
    type Shapes = Mutex<HashMap<(Species, usize, bool), Arc<Vec<Part>>>>;
    static SHAPES: OnceLock<Shapes> = OnceLock::new();
    let shapes = SHAPES.get_or_init(Shapes::default);
    let key = (species, variant % species.variants(), fine);
    if let Some(s) = shapes.lock().expect("tree shapes poisoned").get(&key) {
        return s.clone();
    }
    let mats = world.materials();
    let model = world.library().get(species, variant);
    let mut parts: Vec<Part> = trees::impostor(model, mats)
        .iter()
        .map(|b| {
            let m = mats.get(b.voxel);
            let leaf = m.kind == Kind::Foliage;
            // The far trees' boxes leave out an eighth of the crown on each side, which
            // seen from above makes it look half its breadth: here only the stray tips
            // are left out.
            let (mut lo, mut hi) = (b.lo, b.hi);
            let layer: Vec<IVec3> = model.voxels.iter().filter(|(p, v)| (b.lo.y..b.hi.y).contains(&p.y) && (mats.get(*v).kind == Kind::Foliage) == leaf).map(|(p, _)| *p).collect();
            if !layer.is_empty() {
                let trim = layer.len() / 30;
                for axis in [0, 2] {
                    let mut along: Vec<i32> = layer.iter().map(|p| p[axis]).collect();
                    along.sort_unstable();
                    (lo[axis], hi[axis]) = (along[trim], along[along.len() - 1 - trim] + 1);
                }
            }
            // A maple is modelled in its autumn colours, and is green until its autumn
            // (season.wgsl): the map shows the summer's green.
            let color = if m.plant == voxel::plant::AUTUMN {
                let luma = m.color[0] as f32 * 0.3 + m.color[1] as f32 * 0.59 + m.color[2] as f32 * 0.11;
                [0.30, 0.95, 0.20].map(|k: f32| (k * luma * 1.1).min(255.0) as u8)
            } else {
                m.color
            };
            Part { lo, hi, color, leaf, light: 1.0 }
        })
        .collect();
    let crown: Vec<&Part> = parts.iter().filter(|p| p.leaf).collect();
    if !crown.is_empty() {
        if fine {
            let top = crown.iter().map(|p| p.hi.y).max().unwrap_or(0);
            for p in parts.iter_mut().filter(|p| p.leaf && p.hi.y == top) {
                p.light = 1.06;
            }
        } else {
            // Its lower layers and its upper, each a box about as wide as they are taken
            // together: a broad crown with a smaller one on it, or a spire.
            let (lower, upper) = crown.split_at(crown.len().div_ceil(2));
            let merge = |half: &[&Part], light: f32| {
                let weight = |p: &Part| (p.hi.y - p.lo.y).max(1) as f32;
                let total: f32 = half.iter().map(|p| weight(p)).sum();
                let mean = |f: fn(&Part) -> i32| (half.iter().map(|p| f(p) as f32 * weight(p)).sum::<f32>() / total).round() as i32;
                let bottom = half.iter().map(|p| p.lo.y).min().unwrap_or(0);
                let top = half.iter().map(|p| p.hi.y).max().unwrap_or(1);
                let lo = IVec3::new(mean(|p| p.lo.x), bottom, mean(|p| p.lo.z));
                let hi = IVec3::new(mean(|p| p.hi.x), top, mean(|p| p.hi.z));
                Part { lo, hi: hi.max(lo + IVec3::ONE), color: half[0].color, leaf: true, light }
            };
            let mut merged = vec![merge(lower, 1.0)];
            if !upper.is_empty() {
                merged.push(merge(upper, 1.06));
            }
            parts = merged;
        }
    }
    let s = Arc::new(parts);
    shapes.lock().expect("tree shapes poisoned").insert(key, s.clone());
    s
}

struct Mesh {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
}

impl Mesh {
    fn quad(&mut self, corners: [[f32; 3]; 4], color: [f32; 3], face: u8) {
        let i = self.vertices.len() as u32;
        let c = color.map(|v| v.round().clamp(0.0, 255.0) as u8);
        self.vertices.extend(corners.map(|pos| Vertex { pos, color: [c[0], c[1], c[2], face] }));
        self.indices.extend([i, i + 1, i + 2, i, i + 2, i + 3]);
    }

    /// A box standing on `y0`, `half` wide each way from its middle: its top and sides.
    fn block(&mut self, cx: f32, cz: f32, half: f32, y0: f32, y1: f32, color: [f32; 3]) {
        self.cuboid([cx - half, y0, cz - half], [cx + half, y1, cz + half], color);
    }

    /// The box from `lo` to `hi`: its top and sides.
    fn cuboid(&mut self, lo: [f32; 3], hi: [f32; 3], color: [f32; 3]) {
        let ([x0, y0, z0], [x1, y1, z1]) = (lo, hi);
        self.quad([[x0, y1, z0], [x1, y1, z0], [x1, y1, z1], [x0, y1, z1]], color, 0);
        self.quad([[x1, y1, z1], [x1, y1, z0], [x1, y0, z0], [x1, y0, z1]], color, 1);
        self.quad([[x0, y1, z0], [x0, y1, z1], [x0, y0, z1], [x0, y0, z0]], color, 2);
        self.quad([[x0, y1, z1], [x1, y1, z1], [x1, y0, z1], [x0, y0, z1]], color, 3);
        self.quad([[x1, y1, z0], [x0, y1, z0], [x0, y0, z0], [x1, y0, z0]], color, 4);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(world: &GameWorld, step: i32, seen: impl Fn(usize, usize) -> bool) -> Request {
        let c = world.continent(0, 0);
        let picture = Arc::new(super::super::picture::picture(world, &c));
        // Around the middle of the continent.
        let centre = MACRO_SIZE / 2 / step;
        Request {
            tag: 0,
            n: N,
            step,
            origin: (centre - N as i32 / 2, centre - N as i32 / 2),
            mask: (0..N * N).map(|i| [if seen(i % N, i / N) { 255 } else { 0 }, 0]).collect(),
            sheets: vec![((0, 0), c, picture)],
            version: 7,
        }
    }

    #[test]
    fn only_what_has_been_seen_is_built_and_it_stands_on_a_foot_of_earth() {
        let world = GameWorld::with_config(3, worldgen::continent::SimConfig { size: 256 });
        for step in [FINE_STEP, MIN_STEP, 128] {
            let seen = |x: usize, z: usize| (150..230).contains(&x) && (160..220).contains(&z);
            let r = build(&world, &request(&world, step, seen));
            assert_eq!(r.version, 7);
            for (i, &t) in r.tops.iter().enumerate() {
                assert_eq!(t != UNSEEN, seen(i % N, i / N), "step {step} column {i}");
            }
            assert!(!r.vertices.is_empty() && r.indices.len() % 6 == 0);
            let (lo, hi) = r.vertices.iter().fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v.pos[1]), hi.max(v.pos[1])));
            let lowest = r.tops.iter().filter(|&&t| t != UNSEEN).min().copied().expect("some seen") as f32;
            assert_eq!(lo, lowest - FOOT as f32, "step {step}");
            assert!(hi >= lowest);
            // A tree on the edge may lean out over it, as far as a tree reaches.
            let lean = if step <= TREE_STEP { features::MAX_REACH as f32 / step as f32 } else { 0.0 };
            assert!(r.vertices.iter().all(|v| (146.0 - lean..=234.0 + lean).contains(&v.pos[0]) && v.color[3] <= 4));
            // A point in a seen column is found, one outside is not.
            let s = step as f64;
            let at = |x: usize, z: usize| ((r.origin.0 as f64 + x as f64 + 0.5) * s, (r.origin.1 as f64 + z as f64 + 0.5) * s);
            assert!(r.top_at(at(180, 180).0, at(180, 180).1).is_some());
            assert!(r.top_at(at(10, 10).0, at(10, 10).1).is_none());
        }
    }

    /// Close in, every tree the world grows on the ground shown is drawn, on the column
    /// its trunk stands in, and no other: worked out here as a chunk works out its own,
    /// square by square, each with its own sampler.
    #[test]
    fn the_trees_close_in_stand_where_the_world_grows_them() {
        let world = GameWorld::with_config(3, worldgen::continent::SimConfig { size: 256 });
        let c = world.continent(0, 0);
        let mut forests = 0;
        for step in [FINE_STEP, MIN_STEP] {
            // Squares of ground across the continent, until enough of them have trees.
            for k in 0..12 {
                // On whole columns of the coarser model.
                let (x0, z0) = (MACRO_SIZE / 8 + k * MACRO_SIZE / 16, (MACRO_SIZE / 3 + (k % 3) * MACRO_SIZE / 7) / 64 * 64);
                let side = 32 * 20;
                let seen = |x: i32, z: i32| (x0..x0 + side).contains(&x) && (z0..z0 + side).contains(&z);
                let origin = (x0 / step - 20, z0 / step - 20);
                let mut req = request(&world, step, |_, _| false);
                req.origin = origin;
                req.mask = (0..N * N).map(|i| [if seen((origin.0 + (i % N) as i32) * step, (origin.1 + (i / N) as i32) * step) { 255 } else { 0 }, 0]).collect();
                let r = build(&world, &req);
                let mut drawn = r.trees.clone();
                drawn.sort_unstable();
                // As the world grows them, a chunk's width at a time.
                let mut grown = Vec::new();
                let margin = features::MAX_REACH + 1;
                for sz in (z0..z0 + side).step_by(32) {
                    for sx in (x0..x0 + side).step_by(32) {
                        let sampler = Sampler::new(&c, world.seed(), sx - margin, sz - margin, 32 + 2 * margin);
                        let tors = world.tors_near(&c, sx, sz, 32);
                        for p in features::place(world.seed(), world.library(), &sampler, sx, sz, 32, (0, 0)) {
                            let inside = (sx..sx + 32).contains(&p.pos.x) && (sz..sz + 32).contains(&p.pos.z);
                            if inside &&crate::lod::stands_tall(p.species) && !tors.iter().any(|t| t.crowds(p.pos)) {
                                grown.push((p.pos.x, p.pos.z));
                            }
                        }
                    }
                }
                grown.sort_unstable();
                assert_eq!(drawn, grown, "step {step}, square at {x0} {z0}");
                forests += (grown.len() > 20) as usize;
            }
        }
        assert!(forests >= 4, "only {forests} squares had trees to compare");
    }

    #[test]
    fn the_land_is_drawn_taller_the_further_out_the_map_is() {
        assert_eq!(tallness(MIN_STEP), 1.0);
        assert!(tallness(64) > 1.5 && tallness(MAX_STEP) < 6.0);
    }
}
