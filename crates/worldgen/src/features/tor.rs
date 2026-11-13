//! Tors: the stacks of jointed, rounded granite slabs that crown the summits of the moor,
//! with the clitter of fallen blocks strewn down the slope below them, and the old
//! stonework of the moor (see [`super::moorstones`]), which is built of the same blocks.
//!
//! Granite is cut by joints, some upright and some lying flat. It rotted along them, and
//! once the rotten rock was stripped away the sound rock was left standing as stacks of
//! rounded slabs, like piled loaves. So a tor here is grown, not stamped from a model: a
//! few piles in a line along the summit with grassy avenues between them, each pile a
//! stack of layers that thin and narrow upward, each layer split by upright joints into
//! blocks, and each block a box rounded off at its edges and corners, the more so where it
//! is most weathered. Blocks the frost broke off lie downhill as clitter, thickest just
//! below the tor and on one side, thinning out into single boulders in the grass.
//!
//! Every tor is laid out once per world-space cell, from the continent's grid (which
//! summit is the cell's) and the surface the sampler gives there, and cached; every chunk
//! draws its voxels from the same description, so tors cross chunk borders whole.

use std::{
    collections::HashMap,
    f32::consts::{PI, TAU},
    sync::{Arc, Mutex},
};

use glam::{IVec3, Vec2, Vec3, vec2};
use voxel::Voxel;

use super::shape::Rng;
use crate::{
    biome::Biome,
    continent::{Continent, NO_LAKE},
    macro_layout::MACRO_SIZE,
    materials as mat,
    noise::{hash2, hash3, perlin2, perlin3, unit},
    scale::{m, m64},
    surface::{Column, Cover, Sampler},
};

/// Edge of the world-space cell each tor is laid out from, in voxels (about 205 m). A
/// continent holds a whole number of them.
pub const CELL: i32 = 512;
/// Farthest any rock of a tor (its stacks or its clitter) or of the stonework beside it
/// lies from the summit it stands on, in voxels.
pub const REACH: i32 = 120;
/// How much higher than the surrounding moor the summit needs to stand, at the least, in
/// simulation units, for even a small outcrop.
const OUTCROP_RELIEF: f32 = 1.2;

/// How big a tor is: an outcrop barely knee to head high, a tor of a few stacks, or one
/// of the great tors that stand on the moor's highest summits and are seen from far off.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Class {
    Outcrop,
    Tor,
    Great,
}

/// Where a cell's tor stands, as the continent's grid tells it: enough to name it and
/// mark it on a map without laying it out.
#[derive(Clone, Copy, Debug)]
pub struct Site {
    pub class: Class,
    /// The summit's grid cell.
    pub cell: usize,
    /// Continent-local voxel position of the middle of that grid cell.
    pub at: Vec2,
    /// How far, in voxels, the summit is from any higher ground (up to the distance looked).
    pub dominance: f32,
}

/// The summit a tor cell's tor stands on, if it has one: the highest moor in the cell,
/// sized by how far it is from any higher ground. Depends on the continent's grid alone.
pub fn site(seed: u32, c: &Continent, gx: i32, gz: i32) -> Option<Site> {
    let (ox, oz) = (c.desc.mx * MACRO_SIZE, c.desc.mz * MACRO_SIZE);
    let (lx0, lz0) = (gx * CELL - ox, gz * CELL - oz);
    // Tors keep off the edges of their continent, which are open sea anyway.
    let margin = REACH + 64;
    if lx0 < margin || lz0 < margin || lx0 + CELL > MACRO_SIZE - margin || lz0 + CELL > MACRO_SIZE - margin {
        return None;
    }
    let n = c.size as i32;
    let cs = c.cell_size();
    let moor = |i: usize| c.biome[i] == Biome::Moor as u8 && !c.ocean[i] && c.lake[i] == NO_LAKE;
    // The highest moor among the grid cells whose middles lie in this cell.
    let (x0, x1) = ((lx0 as f32 / cs).floor() as i32, ((lx0 + CELL) as f32 / cs).ceil() as i32 - 1);
    let (z0, z1) = ((lz0 as f32 / cs).floor() as i32, ((lz0 + CELL) as f32 / cs).ceil() as i32 - 1);
    let mut best: Option<(f32, usize, i32, i32)> = None;
    for z in z0.max(1)..=z1.min(n - 2) {
        for x in x0.max(1)..=x1.min(n - 2) {
            let i = (z * n + x) as usize;
            if moor(i) && best.is_none_or(|b| c.height[i] > b.0) {
                best = Some((c.height[i], i, x, z));
            }
        }
    }
    let (h, i, x, z) = best?;
    // Out on the moor, not at its edge.
    let around = [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, 1), (-1, 1), (1, -1)];
    let moorish = around.iter().filter(|(dx, dz)| moor(((z + dz) * n + x + dx) as usize)).count();
    if moorish < 6 {
        return None;
    }
    // How far it is to higher ground, looked for out to a good distance.
    let far = ((m(400.0) / cs).ceil() as i32).max(2);
    let mut dominance = far as f32 * cs;
    'rings: for d in 1..=far {
        for k in -d..=d {
            for (dx, dz) in [(k, -d), (k, d), (-d, k), (d, k)] {
                let (px, pz) = ((x + dx).clamp(0, n - 1), (z + dz).clamp(0, n - 1));
                if c.height[(pz * n + px) as usize] > h {
                    dominance = (((dx * dx + dz * dz) as f32).sqrt() - 0.5) * cs;
                    break 'rings;
                }
            }
        }
    }
    // How far it stands above the ground some way round it.
    let r = ((m(60.0) / cs).round() as i32).max(1);
    let mean = around.iter().map(|(dx, dz)| c.height[(((z + dz * r).clamp(0, n - 1)) * n + (x + dx * r).clamp(0, n - 1)) as usize]).sum::<f32>() / 8.0;
    let relief = h - mean;
    let roll = unit(hash2(seed ^ 0x7023_51, gx, gz));
    let class = if dominance >= m(380.0) && relief > 2.5 && roll < 0.85 {
        Class::Great
    } else if dominance >= m(130.0) && relief > 1.5 && roll < 0.8 {
        Class::Tor
    } else if relief > OUTCROP_RELIEF && roll < 0.55 {
        Class::Outcrop
    } else {
        return None;
    };
    Some(Site {
        class,
        cell: i,
        at: vec2((x as f32 + 0.5) * cs, (z as f32 + 0.5) * cs),
        dominance,
    })
}

/// One block of granite: a box, turned about the vertical, rounded off at its corners and
/// edges. Positions are relative to its tor's origin, in voxels.
#[derive(Clone, Copy, Debug)]
pub struct Block {
    /// Middle.
    pub c: Vec3,
    /// Half extents along its own axes (u, y, v).
    pub e: Vec3,
    /// Cosine and sine of its turn about the vertical.
    cos: f32,
    sin: f32,
    /// How round its corners are in plan.
    round: f32,
    /// How round its top and bottom edges are.
    top: f32,
    bottom: f32,
    /// How rough its surface is, in voxels.
    rough: f32,
    /// Low and high corners of the voxels it can fill, relative to the tor's origin.
    pub lo: IVec3,
    pub hi: IVec3,
    pub part: Part,
}

/// What a block is part of, which decides its colour and whether far views keep it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    /// A layer of a stack.
    Stack,
    /// A fallen block in the clitter below.
    Clitter,
    /// Worked stone: a standing stone, a cross, the stones of a row, circle or hut.
    Worked,
}

impl Block {
    #[allow(clippy::too_many_arguments)]
    pub fn new(c: Vec3, e: Vec3, yaw: f32, round: f32, top: f32, bottom: f32, rough: f32, part: Part) -> Self {
        let (sin, cos) = yaw.sin_cos();
        let reach = (e.x * e.x + e.z * e.z).sqrt() + rough + 1.0;
        let lo = IVec3::new((c.x - reach).floor() as i32, (c.y - e.y - rough - 1.0).floor() as i32, (c.z - reach).floor() as i32);
        let hi = IVec3::new((c.x + reach).ceil() as i32, (c.y + e.y + rough + 1.0).ceil() as i32, (c.z + reach).ceil() as i32);
        // Each rounding no more than the block has room for.
        let top = top.min(e.y * 0.95).max(0.0);
        let bottom = bottom.min(e.y * 0.95).max(0.0);
        let flat = e.x.min(e.z) - top.max(bottom);
        Self { c, e, cos, sin, round: round.min(flat * 0.98).max(0.0), top, bottom, rough, lo, hi, part }
    }

    /// Signed distance from the block's smooth surface to point `p` (relative to the tor's
    /// origin), negative inside.
    pub fn distance(&self, p: Vec3) -> f32 {
        let d = p - self.c;
        let u = d.x * self.cos + d.z * self.sin;
        let v = -d.x * self.sin + d.z * self.cos;
        let rv = if d.y > 0.0 { self.top } else { self.bottom };
        // A rounded rectangle in plan, shrunk by the vertical rounding...
        let (ex, ez) = (self.e.x - rv, self.e.z - rv);
        let (kx, kz) = (u.abs() - (ex - self.round), v.abs() - (ez - self.round));
        let plan = vec2(kx.max(0.0), kz.max(0.0)).length() + kx.max(kz).min(0.0) - self.round;
        // ...drawn up into a slab whose top and bottom edges are rounded too.
        let wy = d.y.abs() - (self.e.y - rv);
        vec2(plan.max(0.0), wy.max(0.0)).length() + plan.max(wy).min(0.0) - rv
    }
}

/// A tor and the stonework near it: every block of granite one cell holds.
#[derive(Clone, Debug)]
pub struct Tor {
    /// World position the blocks are placed from: the summit's top voxel.
    pub origin: IVec3,
    pub class: Option<Class>,
    pub blocks: Vec<Block>,
    /// World-space bounds of every voxel the blocks can fill, inclusive.
    pub lo: IVec3,
    pub hi: IVec3,
    /// Rock basins in the tops of the highest slabs: world position of the middle of the
    /// bowl's rim, radius and depth, in voxels.
    basins: Vec<(Vec3, f32, f32)>,
    /// The stacks' footprints, relative to the origin: middle and radius, and the patch
    /// of short green turf at their feet.
    piles: Vec<(Vec2, f32)>,
    turf: Vec<(Vec2, f32)>,
    salt: u32,
    /// The old stonework laid out with it, and where each piece stands (world).
    pub works: Vec<(super::moorstones::Work, IVec3)>,
}

impl Tor {
    fn empty(origin: IVec3, salt: u32) -> Self {
        Self { origin, class: None, blocks: Vec::new(), lo: IVec3::MAX, hi: IVec3::MIN, basins: Vec::new(), piles: Vec::new(), turf: Vec::new(), salt, works: Vec::new() }
    }

    pub(super) fn push(&mut self, b: Block) {
        self.lo = self.lo.min(self.origin + b.lo);
        self.hi = self.hi.max(self.origin + b.hi);
        self.blocks.push(b);
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Whether any of it can reach the world square from `(x0, z0)`, `span` wide.
    pub fn touches(&self, x0: i32, z0: i32, span: i32) -> bool {
        !self.is_empty() && self.lo.x < x0 + span && self.hi.x >= x0 && self.lo.z < z0 + span && self.hi.z >= z0
    }

    /// The rock at world voxel `q`, if the tor has any there.
    pub fn voxel(&self, q: IVec3) -> Option<Voxel> {
        if q.cmplt(self.lo).any() || q.cmpgt(self.hi).any() {
            return None;
        }
        let local = q - self.origin;
        for b in &self.blocks {
            if local.cmpge(b.lo).all() && local.cmple(b.hi).all()
                && let Some(v) = self.block_voxel(b, q)
            {
                return Some(v);
            }
        }
        None
    }

    /// The voxel block `b` has at world voxel `q`, if it reaches it.
    fn block_voxel(&self, b: &Block, q: IVec3) -> Option<Voxel> {
        let p = (q - self.origin).as_vec3() + Vec3::splat(0.5);
        let d = b.distance(p);
        // Weathered, not machined: the surface wanders by up to a voxel or so. The noise is
        // only worked out near the surface, where it can make a difference.
        if d >= b.rough * 1.1 {
            return None;
        }
        if d > -b.rough * 1.1 {
            let (wx, wy, wz) = (q.x as f64, q.y as f64, q.z as f64);
            let rough = (perlin3(self.salt ^ 0x70f1, wx / 5.3, wy / 5.3, wz / 5.3) + (unit(hash3(self.salt ^ 0x70f2, q.x, q.y, q.z)) - 0.5) * 0.12) * b.rough;
            if d + rough >= 0.0 {
                return None;
            }
        }
        // The bowl of a rock basin, holding the last rain.
        for &(c, r, depth) in &self.basins {
            let d = q.as_vec3() + Vec3::splat(0.5) - c;
            let across = (d.x * d.x + d.z * d.z).sqrt() / r;
            // (Only the bowl and the air just over it: not whatever stands higher beside it.)
            if across < 1.0 && d.y < 3.0 && d.y > -depth * (1.0 - across * across).sqrt() - 0.5 {
                let floor = d.y < -depth * (1.0 - across * across).sqrt() + 0.6;
                return if floor && d.y < 0.0 { Some(mat::WATER) } else { Some(voxel::AIR) };
            }
        }
        Some(granite(self.salt, b, q, p))
    }

    /// Calls `put` with every voxel of rock (or water, or air in a basin) in the world
    /// square from `(x0, z0)`, `span` wide.
    pub fn raster(&self, x0: i32, z0: i32, span: i32, mut put: impl FnMut(IVec3, Voxel)) {
        for b in &self.blocks {
            let (lo, hi) = (self.origin + b.lo, self.origin + b.hi);
            let (ax, bx) = (lo.x.max(x0), hi.x.min(x0 + span - 1));
            let (az, bz) = (lo.z.max(z0), hi.z.min(z0 + span - 1));
            for x in ax..=bx {
                for z in az..=bz {
                    for y in lo.y..=hi.y {
                        let q = IVec3::new(x, y, z);
                        if let Some(v) = self.block_voxel(b, q) {
                            put(q, v);
                        }
                    }
                }
            }
        }
    }

    /// [`Tor::raster`] at a coarser level of detail: one sample from the middle of each
    /// cell `step` voxels wide, given in cells. Clitter and worked stone too small to see
    /// are left out.
    pub fn raster_coarse(&self, step: i32, x0: i32, z0: i32, span: i32, mut put: impl FnMut(IVec3, Voxel)) {
        for b in &self.blocks {
            if b.e.max_element() * 2.0 < step as f32 * 1.5 {
                continue;
            }
            let (lo, hi) = ((self.origin + b.lo).div_euclid(IVec3::splat(step)), (self.origin + b.hi).div_euclid(IVec3::splat(step)));
            for cx in lo.x.max(x0)..=hi.x.min(x0 + span - 1) {
                for cz in lo.z.max(z0)..=hi.z.min(z0 + span - 1) {
                    for cy in lo.y..=hi.y {
                        let q = IVec3::new(cx, cy, cz) * step + IVec3::splat(step / 2);
                        if let Some(v) = self.block_voxel(b, q).filter(|&v| v != voxel::AIR) {
                            put(IVec3::new(cx, cy, cz), if v == mat::WATER { mat::GRANITE } else { v });
                        }
                    }
                }
            }
        }
    }

    /// Whether world column `(x, z)` is short green turf, grazed and trodden at the foot of
    /// the stacks where animals shelter.
    pub fn turf(&self, x: i32, z: i32) -> bool {
        let p = vec2((x - self.origin.x) as f32 + 0.5, (z - self.origin.z) as f32 + 0.5);
        let ragged = perlin2(self.salt ^ 0x7a2f, x as f64 / m64(1.6), z as f64 / m64(1.6)) * m(1.2);
        self.turf.iter().any(|&(c, r)| p.distance(c) < r + ragged)
    }

    /// Whether a tree, bush or boulder standing at world `p` would be inside the rock.
    pub fn crowds(&self, p: IVec3) -> bool {
        let q = p - self.origin;
        let flat = vec2(q.x as f32, q.z as f32);
        self.piles.iter().any(|&(c, r)| flat.distance(c) < r + m(2.0))
            || self.blocks.iter().any(|b| b.part != Part::Stack && q.x >= b.lo.x - 2 && q.x <= b.hi.x + 2 && q.z >= b.lo.z - 2 && q.z <= b.hi.z + 2)
    }

    /// Boxes standing for the stacks and big blocks from far away: world low corner and
    /// size, in voxels, with the colour of the rock.
    pub fn far_boxes(&self) -> impl Iterator<Item = (IVec3, IVec3)> + '_ {
        self.blocks.iter().filter(|b| b.part == Part::Stack || b.e.y > 3.0).map(|b| {
            // A rounded block fills less than its box: the box is drawn a little inside it.
            let half = Vec3::new((b.e.x * b.cos.abs() + b.e.z * b.sin.abs()) * 0.85, b.e.y * 0.9, (b.e.x * b.sin.abs() + b.e.z * b.cos.abs()) * 0.85);
            let lo = (b.c - half).round().as_ivec3() + self.origin;
            let size = (half * 2.0).round().as_ivec3().max(IVec3::ONE);
            (lo, size)
        })
    }
}

/// The colour of a voxel of granite: grey, bleached pale where lichen crusts the weathered
/// tops, streaked darker where the rain runs down, and dark along the joints at the
/// bottom of each block.
fn granite(salt: u32, b: &Block, q: IVec3, p: Vec3) -> Voxel {
    let (wx, wy, wz) = (q.x as f64, q.y as f64, q.z as f64);
    let up = (p.y - (b.c.y - b.e.y)) / (2.0 * b.e.y).max(1.0);
    let patch = perlin3(salt ^ 0x7c01, wx / m64(1.4), wy / m64(1.4), wz / m64(1.4));
    // Streaks run straight down the faces: noise in plan only.
    let streak = perlin2(salt ^ 0x7c02, wx / m64(0.7), wz / m64(0.7));
    match b.part {
        Part::Worked => {
            if patch > 0.3 { mat::GRANITE_PALE } else if patch < -0.35 { mat::GRANITE_GREY } else { mat::GRANITE }
        }
        _ if p.y - (b.c.y - b.e.y) < 1.0 && b.part == Part::Stack => mat::GRANITE_GREY,
        _ if streak > 0.5 && up < 0.8 => mat::GRANITE_GREY,
        _ if up > 0.7 && patch > 0.05 => mat::GRANITE_PALE,
        _ => mat::GRANITE,
    }
}

/// Tors laid out so far, by cell.
#[derive(Default)]
pub struct Cache {
    cells: Mutex<HashMap<(i32, i32), Arc<Tor>>>,
}

impl Cache {
    /// The tor (and stonework) of world cell `(gx, gz)`, which must lie in continent `c`.
    pub fn get(&self, seed: u32, c: &Continent, gx: i32, gz: i32) -> Arc<Tor> {
        if let Some(t) = self.cells.lock().expect("tor cache poisoned").get(&(gx, gz)) {
            return t.clone();
        }
        // Laid out outside the lock: two threads may both do it, with the same result.
        let tor = Arc::new(plan(seed, c, gx, gz));
        self.cells.lock().expect("tor cache poisoned").entry((gx, gz)).or_insert(tor).clone()
    }

    /// Every tor of continent `c` whose rock can reach the world square from `(x0, z0)`,
    /// `span` wide.
    pub fn near(&self, seed: u32, c: &Continent, x0: i32, z0: i32, span: i32) -> Vec<Arc<Tor>> {
        let (ox, oz) = (c.desc.mx * MACRO_SIZE, c.desc.mz * MACRO_SIZE);
        let mut out = Vec::new();
        for gz in (z0 - REACH).div_euclid(CELL)..=(z0 + span + REACH).div_euclid(CELL) {
            for gx in (x0 - REACH).div_euclid(CELL)..=(x0 + span + REACH).div_euclid(CELL) {
                // Only cells of this continent: tors keep well away from its edges.
                if gx * CELL < ox || gz * CELL < oz || (gx + 1) * CELL > ox + MACRO_SIZE || (gz + 1) * CELL > oz + MACRO_SIZE {
                    continue;
                }
                let t = self.get(seed, c, gx, gz);
                if t.touches(x0, z0, span) {
                    out.push(t);
                }
            }
        }
        out
    }
}

/// Lays out the tor of world cell `(gx, gz)` in continent `c`, and the stonework near it.
pub fn plan(seed: u32, c: &Continent, gx: i32, gz: i32) -> Tor {
    let salt = hash2(seed ^ 0x7012, gx, gz);
    let (ox, oz) = (c.desc.mx * MACRO_SIZE, c.desc.mz * MACRO_SIZE);
    let nothing = Tor::empty(IVec3::new(gx * CELL, 0, gz * CELL), salt);
    let Some(site) = site(seed, c, gx, gz) else {
        return super::moorstones::lay_out(seed, c, gx, gz, nothing);
    };
    let span = CELL + 2 * REACH + 16;
    let (sx0, sz0) = (gx * CELL - ox - REACH - 8, gz * CELL - oz - REACH - 8);
    let s = Sampler::new(c, seed, sx0, sz0, span);
    // Columns by world position.
    let col = |x: i32, z: i32| s.column(x - ox, z - oz);
    let mut rng = Rng::new(salt.rotate_left(9));
    // The summit itself, to the voxel: the highest open ground near the grid's.
    let (cx, cz) = (site.at.x as i32 + ox, site.at.y as i32 + oz);
    let half = (c.cell_size() * 0.5) as i32 + 4;
    let mut top: Option<(f32, i32, i32)> = None;
    for dz in (-half..=half).step_by(6) {
        for dx in (-half..=half).step_by(6) {
            let (x, z) = (cx + dx, cz + dz);
            let k = col(x, z);
            if open(&k) && k.clear > m(14.0) && top.is_none_or(|t| k.ground > t.0) {
                top = Some((k.ground, x, z));
            }
        }
    }
    let Some((_, x, z)) = top else {
        return super::moorstones::lay_out(seed, c, gx, gz, nothing);
    };
    // Keep it inside its own cell, so the reach the chunks look for holds.
    let (x, z) = (x.clamp(gx * CELL, gx * CELL + CELL - 1), z.clamp(gz * CELL, gz * CELL + CELL - 1));
    let origin = IVec3::new(x, col(x, z).top(), z);
    let mut tor = Tor::empty(origin, salt);
    tor.class = Some(site.class);
    let ground = |x: i32, z: i32| col(origin.x + x, origin.z + z);

    // The granite of a region is jointed the same way throughout, so neighbouring tors
    // agree on it; each turns a little from it.
    let joints = perlin2(seed ^ 0x70a1, x as f64 / m64(2500.0), z as f64 / m64(2500.0)) * PI + rng.range(-0.15, 0.15);
    // Which way the ground falls, and which way the summit runs as a ridge.
    let e = m(12.0) as i32;
    let fall = vec2(ground(-e, 0).ground - ground(e, 0).ground, ground(0, -e).ground - ground(0, e).ground);
    let downhill = if fall.length() > 0.3 { fall.normalize() } else { vec2(1.0, 0.0) };
    let reach = m(16.0) as i32;
    let ridge = (0..8)
        .map(|k| {
            let a = k as f32 * PI / 8.0;
            let d = vec2(a.cos(), a.sin()) * reach as f32;
            let h = ground(d.x as i32, d.y as i32).ground + ground(-d.x as i32, -d.y as i32).ground;
            (h, a)
        })
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map_or(joints, |r| r.1);

    let (piles, height) = match site.class {
        Class::Great => (rng.int(2, 4), m(rng.range(6.5, 12.0))),
        Class::Tor => (rng.int(1, 3), m(rng.range(3.0, 6.0))),
        Class::Outcrop => (rng.int(1, 2), m(rng.range(0.9, 2.4))),
    };
    // The piles in a line along the ridge, the tallest on the summit, with grassy gaps
    // between them, one now and then wide enough to walk through.
    let axis = vec2(ridge.cos(), ridge.sin());
    let side = vec2(-axis.y, axis.x);
    let mut along = [0.0f32; 2];
    for k in 0..piles {
        let h = if k == 0 { height } else { height * rng.range(0.35, 0.8) };
        let a = h * rng.range(0.45, 0.7) + m(rng.range(2.0, 4.0));
        let b = a * rng.range(0.6, 0.9);
        let centre = if k == 0 {
            Vec2::ZERO
        } else {
            let end = (k % 2) as usize;
            let dir = if end == 0 { 1.0 } else { -1.0 };
            let gap = m(rng.range(1.6, 4.5));
            let at = along[end] + gap + a * 0.9;
            along[end] = at + a * 0.9;
            axis * at * dir + side * rng.range(-1.0, 1.0) * a * 0.4
        };
        if k == 0 {
            along = [a * 0.9, a * 0.9];
        }
        if centre.length() > (REACH - 24) as f32 {
            break;
        }
        pile(&mut tor, &mut rng, &ground, centre, a, b, h, joints, k == 0 && site.class != Class::Outcrop);
    }
    clitter(&mut tor, &mut rng, &ground, site.class, downhill);
    super::moorstones::lay_out(seed, c, gx, gz, tor)
}

/// The four ways across the ground.
const SIDES: [(f32, f32); 4] = [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)];
/// How far each layer of a stack is bedded into the one below, voxels (see `pile`).
const BEDDED: f32 = 0.75;
/// How much of itself a block of a stack must rest on one below, voxels each way.
const REST: f32 = 3.0;
/// How far a logan stone is sunk into the slab it sits on, voxels.
const LOGAN_SEAT: f32 = 2.0;

/// Whether a column is open ground a tor or a stone can stand on.
pub(crate) fn open(k: &Column) -> bool {
    matches!(k.cover, Cover::Land | Cover::Scree | Cover::Cliff) && k.water.is_none() && k.river > m(6.0)
}

/// Grows one stack of slabs on the ground at `centre` (relative to the tor's origin),
/// `a` by `b` voxels across at its foot and `h` tall.
#[allow(clippy::too_many_arguments)]
fn pile(tor: &mut Tor, rng: &mut Rng, ground: &impl Fn(i32, i32) -> Column, centre: Vec2, a: f32, b: f32, h: f32, yaw: f32, crown: bool) {
    let (cos, sin) = (yaw.cos(), yaw.sin());
    let to_plan = |u: f32, v: f32| centre + vec2(u * cos - v * sin, u * sin + v * cos);
    // The ground under it: the stack's foot is buried below the lowest of it.
    let mut lowest = f32::MAX;
    for i in -2..=2 {
        for j in -2..=2 {
            let p = to_plan(i as f32 * a * 0.45, j as f32 * b * 0.45);
            lowest = lowest.min(ground(p.x.round() as i32, p.y.round() as i32).ground);
        }
    }
    let here = ground(centre.x.round() as i32, centre.y.round() as i32).ground;
    let origin_y = tor.origin.y as f32;
    let base = lowest.floor() - origin_y - 2.0;
    let summit = here.floor() - origin_y + 1.0 + h;
    if summit - base < 3.0 {
        return;
    }
    tor.piles.push((centre, a.max(b)));
    // Turf at the foot, mostly on the side away from the weather.
    let lee = vec2(rng.range(-1.0, 1.0), rng.range(-1.0, 1.0)) * a * 0.5;
    tor.turf.push((centre + lee, a.max(b) + m(rng.range(2.5, 5.0))));

    // The upright joints, through the whole stack: two sets at right angles.
    let joints = |rng: &mut Rng, half: f32, lo: f32, hi: f32| {
        let mut cuts = Vec::new();
        let mut t = -half + rng.range(lo * 0.5, lo);
        while t < half - lo * 0.5 {
            cuts.push(t);
            t += rng.range(lo, hi);
        }
        cuts
    };
    let ucuts = joints(rng, a * 1.2, m(3.2), m(6.0));
    let vcuts = joints(rng, b * 1.2, m(3.5), m(6.5));

    // Layers from the buried foot up: thick sheets low down, thinner ones high up.
    let mut y = base;
    let (mut du, mut dv) = (0.0f32, 0.0f32);
    let narrow = rng.range(0.35, 0.65);
    let mut below: Vec<(f32, f32, f32, f32)> = Vec::new();
    let mut layer = 0;
    while y < summit - 1.5 {
        let f = ((y - base) / (summit - base)).clamp(0.0, 1.0);
        let mut thick = m(rng.range(1.1, 2.1)) * (1.2 - 0.5 * f);
        thick = thick.max(3.0).min(summit - y);
        if summit - (y + thick) < 2.0 {
            thick = summit - y;
        }
        let mid = y + thick * 0.5;
        let f = ((mid - base) / (summit - base)).clamp(0.0, 1.0);
        // Each layer narrower than the last, and set a little off it, more so near the top.
        let shrink = 1.0 - (1.0 - narrow) * f.powf(1.3);
        let (la, lb) = (a * shrink * rng.range(0.92, 1.06), b * shrink * rng.range(0.9, 1.08));
        du = (du + rng.range(-1.0, 1.0) * (0.3 + f * 1.0)).clamp(-a * 0.2, a * 0.2);
        dv = (dv + rng.range(-1.0, 1.0) * (0.3 + f * 0.8)).clamp(-b * 0.2, b * 0.2);
        // Where along each axis the layer's joints fall: a joint may be tight in one layer
        // and open in the next, and is nudged a voxel or so from layer to layer.
        let spans = |cuts: &[f32], lo: f32, hi: f32, rng: &mut Rng, open: f32| {
            let mut out = Vec::new();
            let mut start = lo;
            for &c in cuts {
                let c = c + rng.range(-0.8, 0.8);
                if c > lo + 2.0 && c < hi - 2.0 && rng.unit() < open {
                    out.push((start, c));
                    start = c;
                }
            }
            out.push((start, hi));
            out
        };
        let open = 0.35 + 0.45 * f;
        let us = spans(&ucuts, du - la, du + la, rng, open);
        let vs = spans(&vcuts, dv - lb, dv + lb, rng, open * 0.8);
        let mut this: Vec<(f32, f32, f32, f32)> = Vec::new();
        let last = summit - (y + thick) < 0.5;
        for &(u0, u1) in &us {
            for &(v0, v1) in &vs {
                let (uc, vc) = ((u0 + u1) * 0.5, (v0 + v1) * 0.5);
                let outer = u0 <= du - la + 0.1 || u1 >= du + la - 0.1 || v0 <= dv - lb + 0.1 || v1 >= dv + lb - 0.1;
                // High up the frost has taken some of the outer blocks.
                if layer > 0 && outer && f > 0.4 && rng.unit() < 0.22 * f {
                    continue;
                }
                // Open joints gape wider high up, where they have weathered longest.
                let gap = rng.range(0.3, 0.6) + f * 0.5;
                let (hu, hv) = ((u1 - u0) * 0.5 - gap, (v1 - v0) * 0.5 - gap);
                if hu < 1.0 || hv < 1.0 {
                    continue;
                }
                // A block needs one under it, and to rest a good part of itself on it: the
                // two overlap by a few voxels each way, not merely meet at a corner.
                let rests = |&(a0, a1, b0, b1): &(f32, f32, f32, f32)| {
                    let (over_u, over_v) = ((uc + hu).min(a1) - (uc - hu).max(a0), (vc + hv).min(b1) - (vc - hv).max(b0));
                    over_u >= REST.min(2.0 * hu) && over_v >= REST.min(2.0 * hv)
                };
                if layer > 0 && !below.iter().any(rests) {
                    continue;
                }
                let p = to_plan(uc, vc);
                // Each layer is bedded a little into the one under it, so that for all the
                // roughness of both there is rock between them: the joint between two
                // layers shows as a groove round their rounded edges, never as daylight.
                let ey = thick * 0.5 + BEDDED;
                let block = Block::new(
                    Vec3::new(p.x, y + thick * 0.5, p.y),
                    Vec3::new(hu, ey, hv),
                    yaw + rng.range(-0.04, 0.04),
                    m(rng.range(0.5, 1.1)) + f * m(0.6),
                    ey * rng.range(0.55, 0.8),
                    ey * rng.range(0.3, 0.5),
                    0.25 + 0.2 * f,
                    Part::Stack,
                );
                tor.push(block);
                this.push((uc - hu, uc + hu, vc - hv, vc + hv));
            }
        }
        if this.is_empty() {
            break;
        }
        // The top: sometimes a logan stone, a slab left balanced on a small pivot.
        if last && crown {
            crown_stone(tor, rng, &this, y + thick, &to_plan, yaw);
        }
        below = this;
        y += thick;
        layer += 1;
    }
}

/// What sits on top of a tall stack: now and then a logan stone, and now and then a rock
/// basin worn into the top slab.
fn crown_stone(tor: &mut Tor, rng: &mut Rng, top: &[(f32, f32, f32, f32)], y: f32, to_plan: &impl Fn(f32, f32) -> Vec2, yaw: f32) {
    let Some(&(u0, u1, v0, v1)) = top.iter().max_by(|a, b| ((a.1 - a.0) * (a.3 - a.2)).total_cmp(&((b.1 - b.0) * (b.3 - b.2)))) else {
        return;
    };
    let (uc, vc) = ((u0 + u1) * 0.5, (v0 + v1) * 0.5);
    let roll = rng.unit();
    if roll < 0.3 {
        // A logan stone: wider than what it sits on, rounded right under so it touches
        // only in the middle, and set a little off centre.
        let (hu, hv) = ((u1 - u0) * 0.5 * rng.range(0.8, 1.05), (v1 - v0) * 0.5 * rng.range(0.8, 1.05));
        let ey = m(rng.range(0.7, 1.1));
        let p = to_plan(uc + rng.range(-1.0, 1.0), vc + rng.range(-1.0, 1.0));
        // Sunk far enough into its seat that, rough as both are, it is never clear of it.
        tor.push(Block::new(Vec3::new(p.x, y + ey - LOGAN_SEAT, p.y), Vec3::new(hu, ey, hv), yaw + rng.range(-0.4, 0.4), hu.min(hv) * 0.7, ey * 0.6, ey * 0.8, 0.5, Part::Stack));
    } else if roll < 0.75 {
        // A rock basin: a smooth round bowl in the top, holding rainwater.
        let r = ((u1 - u0).min(v1 - v0) * 0.5 - 1.5).min(m(1.3));
        if r >= 1.8 {
            let p = to_plan(uc, vc);
            let o = tor.origin.as_vec3();
            tor.basins.push((Vec3::new(p.x + o.x, y + o.y + 0.2, p.y + o.z), r, (r * 0.45).clamp(1.0, 2.2)));
        }
    }
}

/// The fallen blocks below a tor: a fan down the slope, thickest close under the stacks
/// and on one side, thinning out into lone boulders in the grass.
fn clitter(tor: &mut Tor, rng: &mut Rng, ground: &impl Fn(i32, i32) -> Column, class: Class, downhill: Vec2) {
    let count = match class {
        Class::Great => rng.int(60, 110),
        Class::Tor => rng.int(18, 40),
        Class::Outcrop => rng.int(0, 6),
    };
    let reach = match class {
        Class::Great => (REACH - 16) as f32,
        Class::Tor => m(28.0),
        Class::Outcrop => m(10.0),
    };
    let size = match class {
        Class::Great => m(1.3),
        Class::Tor => m(0.9),
        Class::Outcrop => m(0.55),
    };
    let fall = downhill.y.atan2(downhill.x);
    // One flank takes more of it than the other.
    let skew = rng.range(-0.7, 0.7);
    let piles = tor.piles.clone();
    let mut placed: Vec<(Vec2, f32)> = Vec::new();
    for _ in 0..count * 2 {
        if placed.len() >= count as usize {
            break;
        }
        let spread = (rng.unit() + rng.unit() - 1.0) * 1.6;
        let angle = fall + skew + spread;
        let near = rng.unit().powf(1.7);
        let from = piles.iter().map(|p| p.1).fold(0.0f32, f32::max) * 0.8;
        let start = piles.get(rng.int(0, piles.len().max(1) as i32 - 1) as usize).map_or(Vec2::ZERO, |p| p.0);
        let d = from + (reach - from) * near;
        let p = start + vec2(angle.cos(), angle.sin()) * d;
        if p.length() > reach + 4.0 {
            continue;
        }
        // Clear of the stacks and of each other.
        let half = size * rng.range(0.5, 1.2) * (1.0 - 0.55 * near);
        if piles.iter().any(|&(c, r)| p.distance(c) < r + half) || placed.iter().any(|&(q, r)| p.distance(q) < r + half + 0.5) {
            continue;
        }
        let k = ground(p.x.round() as i32, p.y.round() as i32);
        if !open(&k) || k.clear < m(2.0) {
            continue;
        }
        let half = half.max(1.2);
        let e = Vec3::new(half * rng.range(0.85, 1.35), half * rng.range(0.6, 0.95), half * rng.range(0.7, 1.1));
        // Sunk into the turf, as stones are that have lain there a long time: bedded in
        // the lowest ground under it, so that on a slope no side of it is left in the air.
        let r = e.x.max(e.z) * 0.7;
        let low = SIDES.iter().map(|&(dx, dz)| ground((p.x + dx * r).round() as i32, (p.y + dz * r).round() as i32).ground).fold(k.ground, f32::min);
        let y = low.floor() - tor.origin.y as f32 + 1.0 + e.y * rng.range(-0.3, 0.35);
        let yaw = rng.range(0.0, TAU);
        let c = Vec3::new(p.x, y, p.y);
        let block = |c: Vec3, e: Vec3, yaw: f32, rng: &mut Rng| {
            Block::new(c, e, yaw, e.x.min(e.z) * rng.range(0.45, 0.85), e.y * rng.range(0.6, 0.95), e.y * rng.range(0.2, 0.4), 0.7, Part::Clitter)
        };
        tor.push(block(c, e, yaw, rng));
        // Many are two lumps of one broken block, which breaks up the box of it.
        if half > 2.0 && rng.unit() < 0.45 {
            let (s, co) = yaw.sin_cos();
            let off = vec2(co, s) * e.x * rng.range(0.5, 0.8) + vec2(-s, co) * e.z * rng.range(-0.4, 0.4);
            let e2 = e * Vec3::new(rng.range(0.5, 0.75), rng.range(0.6, 0.9), rng.range(0.6, 0.85));
            tor.push(block(c + Vec3::new(off.x, e2.y - e.y, off.y), e2, yaw + rng.range(-0.6, 0.6), rng));
        }
        placed.push((p, half));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slab() -> Block {
        Block::new(Vec3::new(0.0, 5.0, 0.0), Vec3::new(8.0, 3.0, 5.0), 0.3, 2.0, 2.0, 1.5, 0.0, Part::Stack)
    }

    #[test]
    fn a_block_is_solid_inside_and_rounded_at_its_corners() {
        let b = slab();
        assert!(b.distance(Vec3::new(0.0, 5.0, 0.0)) < -2.5);
        // The middle of a face is on the surface; its corner is cut well back.
        let (s, c) = (0.3f32.sin(), 0.3f32.cos());
        let at = |u: f32, y: f32, v: f32| Vec3::new(u * c - v * s, y, u * s + v * c);
        assert!(b.distance(at(7.9, 5.0, 0.0)).abs() < 0.2);
        assert!(b.distance(at(7.9, 7.9, 4.9)) > 1.5);
        assert!(b.distance(at(0.0, 9.0, 0.0)) > 0.5);
    }

    /// The world voxel at `(x, y, z)` as chunk column output holds it, if it covers it.
    fn voxel_at(out: &crate::ColumnOutput, (cx, cz): (i32, i32), x: i32, y: i32, z: i32) -> Option<Voxel> {
        use voxel::CHUNK;
        let (px, pz) = (x - (cx * CHUNK - 1), z - (cz * CHUNK - 1));
        if !(0..voxel::PAD as i32).contains(&px) || !(0..voxel::PAD as i32).contains(&pz) {
            return None;
        }
        let (cy, ly) = (y.div_euclid(CHUNK), y.rem_euclid(CHUNK));
        let (_, chunk) = out.chunks.iter().find(|(k, _)| *k == cy)?;
        Some(chunk.get(px as usize, ly as usize + 1, pz as usize))
    }

    #[test]
    fn great_tors_stand_on_moor_summits_and_agree_across_chunks() {
        use crate::world::GameWorld;
        use voxel::CHUNK;
        let w = GameWorld::new(1);
        let c = w.continent(0, 0);
        let cells = MACRO_SIZE / CELL;
        let sites: Vec<(i32, i32, Site)> = (0..cells).flat_map(|gz| (0..cells).map(move |gx| (gx, gz))).filter_map(|(gx, gz)| site(1, &c, gx, gz).map(|s| (gx, gz, s))).collect();
        let great: Vec<_> = sites.iter().filter(|s| s.2.class == Class::Great).collect();
        assert!(great.len() >= 2 && sites.len() > great.len() * 3, "{} tors, {} great", sites.len(), great.len());
        let &&(gx, gz, _) = great.first().expect("a great tor");
        let tor = plan(1, &c, gx, gz);
        let again = plan(1, &c, gx, gz);
        assert_eq!(tor.blocks.len(), again.blocks.len());
        assert!(tor.blocks.iter().filter(|b| b.part == Part::Stack).count() >= 12);
        assert!(tor.blocks.iter().filter(|b| b.part == Part::Clitter).count() >= 30);
        // It stands well up from its summit.
        assert!(tor.hi.y - tor.origin.y >= m(5.0) as i32, "only {} voxels tall", tor.hi.y - tor.origin.y);
        // Drawn in quarters, it is the same as drawn whole.
        let span = (tor.hi.x - tor.lo.x + 1).max(tor.hi.z - tor.lo.z + 1);
        let mut whole = Vec::new();
        tor.raster(tor.lo.x, tor.lo.z, span, |q, v| whole.push((q, v)));
        let mut halves = Vec::new();
        let half = (span + 1) / 2;
        for (dx, dz) in [(0, 0), (half, 0), (0, half), (half, half)] {
            tor.raster(tor.lo.x + dx, tor.lo.z + dz, half, |q, v| halves.push((q, v)));
        }
        whole.sort_by_key(|(q, _)| (q.x, q.y, q.z));
        halves.sort_by_key(|(q, _)| (q.x, q.y, q.z));
        assert!(whole == halves, "{} voxels drawn whole, {} in quarters", whole.len(), halves.len());
        // Two chunk columns across the stack agree on the voxels both hold.
        let (cx, cz) = (tor.origin.x.div_euclid(CHUNK), tor.origin.z.div_euclid(CHUNK));
        let (a, b) = (w.generate_column_bare(cx, cz), w.generate_column_bare(cx + 1, cz));
        let mut rock = 0;
        for x in [(cx + 1) * CHUNK - 1, (cx + 1) * CHUNK] {
            for z in cz * CHUNK..(cz + 1) * CHUNK {
                for y in tor.origin.y - 8..tor.hi.y {
                    let (va, vb) = (voxel_at(&a, (cx, cz), x, y, z), voxel_at(&b, (cx + 1, cz), x, y, z));
                    if let (Some(va), Some(vb)) = (va, vb) {
                        assert_eq!(va, vb, "chunks disagree at {x},{y},{z}");
                        rock += matches!(va, mat::GRANITE | mat::GRANITE_GREY | mat::GRANITE_PALE) as u32;
                    }
                }
            }
        }
        assert!(rock > 0, "no granite where the chunks meet");
    }

    /// The rock of `tor` as the chunks draw it, and how many of its voxels hang in the air:
    /// those no chain of rock voxels, face to face, joins to the ground.
    fn loose(w: &crate::world::GameWorld, tor: &Tor) -> (usize, usize) {
        use std::collections::{HashMap, HashSet};
        let span = (tor.hi.x - tor.lo.x + 1).max(tor.hi.z - tor.lo.z + 1);
        let mut drawn: HashMap<IVec3, Voxel> = HashMap::new();
        tor.raster(tor.lo.x, tor.lo.z, span, |q, v| {
            drawn.insert(q, v);
        });
        let rock: HashSet<IVec3> = drawn.iter().filter(|(_, v)| !matches!(**v, voxel::AIR | mat::WATER)).map(|(q, _)| *q).collect();
        let mut tops: HashMap<(i32, i32), i32> = HashMap::new();
        let mut held: HashSet<IVec3> = HashSet::new();
        let mut front = Vec::new();
        for &q in &rock {
            let top = *tops.entry((q.x, q.z)).or_insert_with(|| w.column(q.x, q.z).top());
            if q.y <= top + 1 {
                held.insert(q);
                front.push(q);
            }
        }
        while let Some(q) = front.pop() {
            for d in [IVec3::X, IVec3::NEG_X, IVec3::Y, IVec3::NEG_Y, IVec3::Z, IVec3::NEG_Z] {
                if rock.contains(&(q + d)) && held.insert(q + d) {
                    front.push(q + d);
                }
            }
        }
        (rock.len(), rock.len() - held.len())
    }

    #[test]
    fn no_rock_of_a_tor_hangs_in_the_air() {
        use crate::world::GameWorld;
        let w = GameWorld::new(1);
        let c = w.continent(0, 0);
        let cells = MACRO_SIZE / CELL;
        let sites: Vec<(i32, i32, Site)> = (0..cells).flat_map(|gz| (0..cells).map(move |gx| (gx, gz))).filter_map(|(gx, gz)| site(1, &c, gx, gz).map(|s| (gx, gz, s))).collect();
        let mut seen = 0;
        let mut bad = Vec::new();
        for class in [Class::Great, Class::Tor, Class::Outcrop] {
            for (gx, gz, _) in sites.iter().filter(|s| s.2.class == class).take(14) {
                let tor = plan(1, &c, *gx, *gz);
                if tor.is_empty() {
                    continue;
                }
                let (all, hanging) = loose(&w, &tor);
                seen += 1;
                if hanging > 0 {
                    bad.push((class, tor.origin, all, hanging));
                }
            }
        }
        assert!(seen >= 20, "only {seen} tors looked at");
        assert!(bad.is_empty(), "{} of {seen} tors have rock hanging in the air: {bad:?}", bad.len());
    }

    #[test]
    fn bounds_hold_every_voxel_a_block_fills() {
        let mut rng = Rng::new(3);
        for _ in 0..50 {
            let b = Block::new(
                Vec3::new(rng.range(-5.0, 5.0), rng.range(-5.0, 5.0), rng.range(-5.0, 5.0)),
                Vec3::new(rng.range(1.0, 9.0), rng.range(1.0, 4.0), rng.range(1.0, 9.0)),
                rng.range(0.0, TAU),
                rng.range(0.0, 4.0),
                rng.range(0.0, 3.0),
                rng.range(0.0, 3.0),
                1.0,
                Part::Clitter,
            );
            for x in b.lo.x - 4..=b.hi.x + 4 {
                for y in b.lo.y - 4..=b.hi.y + 4 {
                    for z in b.lo.z - 4..=b.hi.z + 4 {
                        let q = IVec3::new(x, y, z);
                        if b.distance(q.as_vec3() + Vec3::splat(0.5)) < 1.0 {
                            assert!(q.cmpge(b.lo).all() && q.cmple(b.hi).all(), "{q} outside {:?}..{:?}", b.lo, b.hi);
                        }
                    }
                }
            }
        }
    }
}
