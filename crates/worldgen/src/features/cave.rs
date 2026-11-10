//! Small caves, and the decoration of caves and small ravines.
//!
//! The terrain is a heightfield, so caves are carved out of the filled chunk voxels
//! afterwards. Each is laid out once per world-space cell, from the surface the
//! sampler gives there, and cached: a short tunnel 2 to 5 m tall and 5 to 40 m long,
//! now and then opening into a small chamber, that runs into a hillside, a cliff face or
//! the wall of a small ravine from a mouth at the foot of the slope. Every chunk carves
//! the same shape from the same world-space description, so caves cross chunk borders
//! whole.
//!
//! A cave is only kept where it is well clear of water, rivers, trails and landmarks,
//! with rock enough over it that the roof is never broken except at the mouth, and its
//! floor is flat or gently sloping and stays solid.
//!
//! Inside, the walls show the local rock strata, darkening step by step away from the
//! mouth (the renderer has no sky occlusion, so the dark is put in the colour).
//! Stalactites hang from the ceiling and stalagmites rise from the floor as tapering
//! stacks of 0.1 m boxes, the biggest on a full voxel, and some meet as pillars. Rubble
//! lies on the floors. Moss grows only near the mouth, where the rock keeps its
//! ordinary colours (see `moss`), since the darker cave rock is no host for it.
//!
//! Small ravines are decorated here too: rubble on their floors and ferns in the
//! shade under their walls.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use glam::{IVec3, Vec2, Vec3, vec2};
use voxel::{
    AIR, CHUNK, PAD, PaddedChunk, Voxel,
    mesh::{CELLS, PlantBox},
};

use super::{Placement, shape::Rng};
use crate::{
    continent::Continent,
    macro_layout::MACRO_SIZE,
    materials as mat,
    noise::{hash2, hash3, perlin2, unit},
    scale::{m, m64},
    surface::{self, Column, Cover, Sampler},
};

/// Edge of the world-space cell each cave is laid out from, in voxels (about 205 m).
/// A continent holds a whole number of them.
pub const CELL: i32 = 512;
/// Chance that a cell tries to hold a cave.
const CHANCE: f32 = 0.6;
/// How far in from its cell's edges a cave's mouth lies.
const INSET: i32 = 48;
/// Farthest any part of a cave lies from its cell, in voxels.
const REACH: i32 = 120;
/// Candidate mouths tried per cell.
const TRIES: usize = 48;

/// One point along a cave's centreline.
#[derive(Clone, Copy, Debug)]
struct Node {
    /// Position relative to the cave's cell corner, in voxels.
    p: Vec2,
    /// Floor height (the lowest air voxel).
    floor: f32,
    /// Half its width at the floor.
    hw: f32,
    /// Height of the roof above the floor along the centreline.
    height: f32,
    /// Distance from the mouth along the centreline.
    s: f32,
}

/// One cave, in world voxels.
#[derive(Clone, Debug)]
pub struct Cave {
    /// World corner of the cell it is laid out from.
    origin: (i32, i32),
    nodes: Vec<Node>,
    salt: u32,
    /// World bounding box in plan, inclusive.
    lo: (i32, i32),
    hi: (i32, i32),
    /// Lowest floor.
    pub floor: i32,
    /// Where to stand to look into the mouth, and the yaw (degrees) that looks in.
    pub mouth: Vec3,
    pub yaw: f32,
    /// A point inside, near the end, and how long the cave is in metres.
    pub inside: Vec3,
    pub length: f32,
}

/// What a cave does to one column: air from `floor` up to below `roof`, at `s` from
/// the mouth along the centreline.
#[derive(Clone, Copy, Debug)]
struct Span {
    floor: i32,
    roof: i32,
    s: f32,
}

impl Cave {
    fn touches(&self, x0: i32, z0: i32, x1: i32, z1: i32) -> bool {
        self.lo.0 <= x1 && self.hi.0 >= x0 && self.lo.1 <= z1 && self.hi.1 >= z0
    }

    /// The air this cave carves out of world column `(wx, wz)`.
    fn span(&self, wx: i32, wz: i32) -> Option<Span> {
        if wx < self.lo.0 || wx > self.hi.0 || wz < self.lo.1 || wz > self.hi.1 {
            return None;
        }
        let p = vec2((wx - self.origin.0) as f32 + 0.5, (wz - self.origin.1) as f32 + 0.5);
        // Walls are ragged at a metre or so, and bulge and pinch over a few.
        let wobble = 1.0
            + 0.16 * perlin2(self.salt ^ 0x3a11, wx as f64 / m64(2.4), wz as f64 / m64(2.4))
            + 0.1 * (unit(hash2(self.salt ^ 0x3a12, wx, wz)) - 0.5);
        let lumpy = 1.0 + 0.12 * perlin2(self.salt ^ 0x3a13, wx as f64 / m64(1.6), wz as f64 / m64(1.6));
        let mut best: Option<Span> = None;
        for w in self.nodes.windows(2) {
            let (a, b) = (w[0], w[1]);
            let ab = b.p - a.p;
            let t = ((p - a.p).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
            let d = (p - (a.p + ab * t)).length();
            // Eased, so a chamber swells and closes roundly.
            let e = t * t * (3.0 - 2.0 * t);
            let hw = (a.hw + (b.hw - a.hw) * e) * wobble;
            if d >= hw {
                continue;
            }
            let floor = a.floor + (b.floor - a.floor) * t;
            let height = (a.height + (b.height - a.height) * e) * lumpy;
            let across = d / hw;
            let roof = floor + height * (1.0 - across * across).sqrt();
            let (f, r) = (floor.round() as i32, roof.round() as i32);
            if r <= f {
                continue;
            }
            let s = a.s + (b.s - a.s) * t;
            best = Some(match best {
                None => Span { floor: f, roof: r, s },
                Some(o) => Span { floor: o.floor.min(f), roof: o.roof.max(r), s: o.s.min(s) },
            });
        }
        best
    }
}

/// Caves laid out so far, by cell.
#[derive(Default)]
pub struct Cache {
    cells: Mutex<HashMap<(i32, i32), Arc<Vec<Cave>>>>,
}

impl Cache {
    /// The caves of world cell `(gx, gz)`, which must lie in continent `c`.
    pub fn get(&self, seed: u32, c: &Continent, gx: i32, gz: i32) -> Arc<Vec<Cave>> {
        if let Some(v) = self.cells.lock().unwrap().get(&(gx, gz)) {
            return v.clone();
        }
        // Laid out outside the lock: two threads may both do it, with the same result.
        let caves = Arc::new(plan(seed, c, gx, gz));
        self.cells.lock().unwrap().entry((gx, gz)).or_insert(caves).clone()
    }
}

/// Lays out the caves of world cell `(gx, gz)` in continent `c`.
pub fn plan(seed: u32, c: &Continent, gx: i32, gz: i32) -> Vec<Cave> {
    let h = hash2(seed ^ 0xca5e, gx, gz);
    if unit(h) >= CHANCE {
        return Vec::new();
    }
    let (ox, oz) = (c.desc.mx * MACRO_SIZE, c.desc.mz * MACRO_SIZE);
    let origin = (gx * CELL, gz * CELL);
    let (lx0, lz0) = (origin.0 - ox, origin.1 - oz);
    // Caves keep off the edges of their continent, which are open sea anyway.
    if lx0 < REACH || lz0 < REACH || lx0 + CELL > MACRO_SIZE - REACH || lz0 + CELL > MACRO_SIZE - REACH {
        return Vec::new();
    }
    let s = Sampler::new(c, seed, lx0 - REACH, lz0 - REACH, CELL + 2 * REACH);
    // Columns by position relative to the cell corner.
    let col = |p: Vec2| s.column(lx0 + p.x.floor() as i32, lz0 + p.y.floor() as i32);
    let mut rng = Rng::new(h.rotate_left(7));

    // Candidate mouths: the walls of any small ravine here first, then anywhere.
    let mut tries = Vec::new();
    for r in s.ravines() {
        let q = r.centre - vec2(lx0 as f32, lz0 as f32);
        if q.min_element() < INSET as f32 || q.max_element() > (CELL - INSET) as f32 {
            continue;
        }
        let perp = vec2(-r.dir.y, r.dir.x);
        for k in 0..4 {
            let t = (k as f32 / 3.0 - 0.5) * r.half_len * 0.8;
            let side = if rng.unit() < 0.5 { 1.0 } else { -1.0 };
            tries.push(q + r.dir * t + perp * side * (r.half_width + 2.0));
        }
    }
    for _ in 0..TRIES {
        tries.push(vec2(rng.range(INSET as f32, (CELL - INSET) as f32), rng.range(INSET as f32, (CELL - INSET) as f32)));
    }
    for q in tries {
        if let Some(cave) = lay_out(seed, &col, q, origin, &mut rng) {
            return vec![cave];
        }
    }
    Vec::new()
}

/// Whether a column is somewhere a cave may run under, or open beside.
fn clear(c: &Column) -> bool {
    c.water.is_none() && c.river > m(10.0) && c.clear > m(6.0) && c.ground > m(6.0)
}

/// Tries to lay out a cave whose mouth is at `q` (relative to the cell corner).
fn lay_out(seed: u32, col: &impl Fn(Vec2) -> Column, q: Vec2, origin: (i32, i32), rng: &mut Rng) -> Option<Cave> {
    let here = col(q);
    if !clear(&here) || here.river < m(20.0) || here.clear < m(10.0) || !matches!(here.cover, Cover::Land | Cover::Cliff | Cover::Scree) {
        return None;
    }
    // Uphill, across a couple of metres: a cave runs into steep ground.
    let e = m(1.2);
    let g = |d: Vec2| col(q + d).ground;
    let grad = vec2(g(vec2(e, 0.0)) - g(vec2(-e, 0.0)), g(vec2(0.0, e)) - g(vec2(0.0, -e))) / (2.0 * e);
    let steep = grad.length();
    if steep < 0.75 {
        return None;
    }
    let mut dir = grad / steep;
    // The floor is the ground just below the slope, so the mouth opens onto it.
    let foot = q - dir * m(1.6);
    let below = col(foot);
    if !clear(&below) {
        return None;
    }
    let floor = (below.top() + 1) as f32;
    if here.ground < floor + m(1.0) {
        return None;
    }
    let target = m(5.0) + m(35.0) * rng.unit().powf(1.5);
    let start = Node { p: q - dir * m(2.2), floor, hw: rng.range(m(1.0), m(1.6)), height: rng.range(m(2.2), m(3.0)), s: 0.0 };
    let mut nodes = vec![start];
    let mut heading = dir.y.atan2(dir.x);
    // Whether the roof has closed over the passage yet, and where.
    let mut roofed: Option<f32> = None;
    let mut chambered = false;
    'grow: while roofed.is_none_or(|r| nodes.last().unwrap().s - r < target) {
        let a = *nodes.last().unwrap();
        let first = nodes.len() == 1;
        let seg = if first { m(4.0) } else { rng.range(m(3.0), m(7.0)) };
        heading += if first { rng.range(-0.15, 0.15) } else { rng.range(-0.55, 0.55) };
        dir = vec2(heading.cos(), heading.sin());
        let into = a.s - roofed.unwrap_or(a.s) + seg;
        let mut b = Node {
            p: a.p + dir * seg,
            floor: if first { a.floor } else { (a.floor + rng.range(-0.06, 0.12) * seg).max(m(5.0)) },
            hw: rng.range(m(1.0), m(2.1)),
            height: rng.range(m(2.0), m(3.4)),
            s: a.s + seg,
        };
        // A small chamber at the end, or now and then along the way.
        if !chambered && !first && (into >= target || rng.unit() < 0.15) && rng.unit() < 0.6 {
            chambered = true;
            b.hw = rng.range(m(2.6), m(4.5));
            b.height = rng.range(m(3.2), m(5.0));
        }
        // Check the ground over and around it, past the end by the width of the cap.
        let perp = vec2(-dir.y, dir.x);
        let steps = ((seg + b.hw) / m(1.2)).ceil() as i32;
        for k in 1..=steps {
            let d = k as f32 * m(1.2);
            let t = (d / seg).min(1.0);
            let c = a.p + dir * d;
            let hw = a.hw + (b.hw - a.hw) * t;
            let top = a.floor + (b.floor - a.floor) * t + a.height + (b.height - a.height) * t;
            let mut roof = f32::MAX;
            for o in [-1.0f32, -0.6, 0.0, 0.6, 1.0] {
                let cl = col(c + perp * o * hw * 1.15);
                if !clear(&cl) {
                    if roofed.is_some() {
                        break 'grow;
                    }
                    return None;
                }
                roof = roof.min(cl.ground - top * if o.abs() > 0.9 { 0.7 } else { 1.0 });
            }
            // Ground a little way out to the sides must not be water either.
            for o in [-1.0f32, 1.0] {
                if !clear(&col(c + perp * o * (hw + m(2.5)))) {
                    if roofed.is_some() {
                        break 'grow;
                    }
                    return None;
                }
            }
            match roofed {
                None if roof > m(1.8) && col(c).ground > top + m(2.0) => roofed = Some(a.s + d),
                // The mouth must close over within a few metres.
                None if a.s + d > m(9.0) => return None,
                None => {}
                // Once roofed, the passage ends where its roof would grow thin.
                Some(_) if roof < m(1.6) => break 'grow,
                Some(_) => {}
            }
        }
        nodes.push(b);
        if nodes.len() > 12 {
            break;
        }
    }
    let roofed = roofed?;
    let length = nodes.last().unwrap().s - roofed;
    if length < m(5.0) || nodes.len() < 3 {
        return None;
    }
    // Bounding box, with room for ragged walls.
    let (mut lo, mut hi) = (Vec2::MAX, Vec2::MIN);
    for n in &nodes {
        let r = Vec2::splat(n.hw * 1.35 + 2.0);
        lo = lo.min(n.p - r);
        hi = hi.max(n.p + r);
    }
    let floor = nodes.iter().map(|n| n.floor.round() as i32).min().unwrap();
    let w = |v: Vec2| Vec3::new(origin.0 as f32 + v.x, 0.0, origin.1 as f32 + v.y);
    let first = nodes[0];
    let inward = (nodes[1].p - first.p).normalize();
    let mut mouth = w(first.p - inward * m(5.0));
    mouth.y = first.floor + m(1.7);
    let deep = nodes[nodes.len() - 2];
    let mut inside = w(deep.p);
    inside.y = deep.floor + m(1.5);
    Some(Cave {
        origin,
        salt: hash2(seed ^ 0xca5f, origin.0, origin.1),
        lo: (origin.0 + lo.x.floor() as i32, origin.1 + lo.y.floor() as i32),
        hi: (origin.0 + hi.x.ceil() as i32, origin.1 + hi.y.ceil() as i32),
        nodes,
        floor,
        mouth,
        // The camera's yaw: 0 looks along +x, 90 along +z.
        yaw: inward.y.atan2(inward.x).to_degrees(),
        inside,
        length: length / m(1.0),
    })
}

/// The caves that reach one chunk column (with its one-voxel border).
pub struct Carve {
    caves: Vec<Cave>,
}

impl Carve {
    /// The caves reaching the padded chunk column whose corner is world `(wx0, wz0)`,
    /// in continent `c`.
    pub fn new(cache: &Cache, seed: u32, c: &Continent, wx0: i32, wz0: i32) -> Self {
        let (ox, oz) = (c.desc.mx * MACRO_SIZE, c.desc.mz * MACRO_SIZE);
        let mut caves = Vec::new();
        let (x1, z1) = (wx0 + PAD as i32, wz0 + PAD as i32);
        for gz in (wz0 - REACH).div_euclid(CELL)..=(z1 + REACH).div_euclid(CELL) {
            for gx in (wx0 - REACH).div_euclid(CELL)..=(x1 + REACH).div_euclid(CELL) {
                // Only cells of this continent: caves never reach out of theirs.
                if gx * CELL < ox || gz * CELL < oz || (gx + 1) * CELL > ox + MACRO_SIZE || (gz + 1) * CELL > oz + MACRO_SIZE {
                    continue;
                }
                for cave in cache.get(seed, c, gx, gz).iter() {
                    if cave.touches(wx0 - 1, wz0 - 1, x1, z1) {
                        caves.push(cave.clone());
                    }
                }
            }
        }
        Self { caves }
    }

    /// The lowest voxel a cave here leaves solid, if there is a cave.
    pub fn lowest(&self) -> Option<i32> {
        self.caves.iter().map(|c| c.floor - 2).min()
    }

    fn span(&self, wx: i32, wz: i32) -> Option<Span> {
        let mut best: Option<Span> = None;
        for c in &self.caves {
            if let Some(s) = c.span(wx, wz) {
                best = Some(match best {
                    None => s,
                    Some(o) => Span { floor: o.floor.min(s.floor), roof: o.roof.max(s.roof), s: o.s.min(s.s) },
                });
            }
        }
        best
    }

    /// Carves the caves out of a chunk column's voxels and decorates them, and the
    /// floors of any small ravines in it. `out` holds the chunks from `cy_min` up,
    /// `columns` its padded columns, whose corner is world `(wx0, wz0)` and
    /// continent-local `(lx0, lz0)`. Plants and features standing on ground carved
    /// away are removed.
    #[allow(clippy::too_many_arguments)]
    pub fn apply(
        &self,
        seed: u32,
        sampler: &Sampler,
        out: &mut [(i32, PaddedChunk)],
        cy_min: i32,
        columns: &[Column],
        (wx0, wz0): (i32, i32),
        (lx0, lz0): (i32, i32),
        plants: &mut Vec<PlantBox>,
        placements: &mut Vec<Placement>,
    ) {
        let mut grid = Grid { out, base: cy_min * CHUNK };
        let mut decor = Vec::new();
        if !self.caves.is_empty() {
            self.carve(seed, &mut grid, columns, (wx0, wz0), plants, placements, &mut decor);
        }
        ravine_floors(seed, sampler, columns, (wx0, wz0), (lx0, lz0), &mut decor);
        plants.extend(decor);
    }

    #[allow(clippy::too_many_arguments)]
    fn carve(
        &self,
        seed: u32,
        grid: &mut Grid,
        columns: &[Column],
        (wx0, wz0): (i32, i32),
        plants: &mut Vec<PlantBox>,
        placements: &mut Vec<Placement>,
        decor: &mut Vec<PlantBox>,
    ) {
        // Spans for the padded columns and one more all round, so walls at the border
        // know about the air beyond it. Clipped to the ground: above it is open air.
        const W: usize = PAD + 2;
        let mut spans: Vec<Option<Span>> = vec![None; W * W];
        let top_at = |px: i32, pz: i32| {
            let (x, z) = (px.clamp(0, PAD as i32 - 1) as usize, pz.clamp(0, PAD as i32 - 1) as usize);
            columns[x * PAD + z].top()
        };
        for x in 0..W as i32 {
            for z in 0..W as i32 {
                if let Some(mut s) = self.span(wx0 + x - 1, wz0 + z - 1) {
                    s.roof = s.roof.min(top_at(x - 1, z - 1) + 1);
                    if s.roof > s.floor {
                        spans[x as usize * W + z as usize] = Some(s);
                    }
                }
            }
        }
        let span = |px: i32, pz: i32| spans[(px + 1) as usize * W + (pz + 1) as usize];
        if spans.iter().all(Option::is_none) {
            return;
        }

        // Air.
        for px in 0..PAD as i32 {
            for pz in 0..PAD as i32 {
                if let Some(s) = span(px, pz) {
                    for y in s.floor..s.roof {
                        if grid.get(px, y, pz).is_some_and(|v| v != mat::WATER) {
                            grid.set(px, y, pz, AIR);
                        }
                    }
                }
            }
        }

        // Walls, floor and roof: the rock strata, darker further in, and rubble
        // underfoot. Decided per voxel from world position, so borders agree.
        for px in 0..PAD as i32 {
            for pz in 0..PAD as i32 {
                let own = span(px, pz);
                let near = [span(px + 1, pz), span(px - 1, pz), span(px, pz + 1), span(px, pz - 1)];
                let (mut lo, mut hi, mut s) = (i32::MAX, i32::MIN, f32::MAX);
                for n in near.iter().chain(std::iter::once(&own)).flatten() {
                    lo = lo.min(n.floor - 1);
                    hi = hi.max(n.roof);
                    s = s.min(n.s);
                }
                if lo > hi {
                    continue;
                }
                let col = &columns[px as usize * PAD + pz as usize];
                let (wx, wz) = (wx0 + px, wz0 + pz);
                let tilt = surface::strata_tilt(seed, wx as f64, wz as f64);
                for y in lo..=hi.min(col.top() - 1) {
                    if own.is_some_and(|o| (o.floor..o.roof).contains(&y)) {
                        continue;
                    }
                    let floor = own.is_some_and(|o| o.floor == y + 1);
                    let wall = floor
                        || own.is_some_and(|o| o.roof == y)
                        || near.iter().flatten().any(|n| (n.floor..n.roof).contains(&y));
                    if !wall || grid.get(px, y, pz).is_none_or(|v| v == AIR || v == mat::WATER) {
                        continue;
                    }
                    let h = hash3(seed ^ 0xca71, wx, y, wz);
                    let rock = if floor && unit(h) < 0.55 {
                        mat::GRAVEL
                    } else {
                        strata(seed, col, wx, y, wz, tilt)
                    };
                    grid.set(px, y, pz, mat::cave_rock(rock, tier(s, h)));
                }
            }
        }

        // Plants and features left standing on carved ground go.
        let carved = |x: i32, y: i32, z: i32| {
            let (px, pz) = (x - wx0, z - wz0);
            (0..PAD as i32).contains(&px)
                && (0..PAD as i32).contains(&pz)
                && span(px, pz).is_some_and(|s| (s.floor - 1..=s.roof).contains(&y))
        };
        plants.retain(|b| {
            let q = IVec3::from_array(b.lo).div_euclid(IVec3::splat(CELLS));
            !carved(q.x, q.y - 1, q.z)
        });
        placements.retain(|p| !carved(p.pos.x, p.pos.y - 1, p.pos.z) && !carved(p.pos.x, p.pos.y, p.pos.z));

        // Stalactites, stalagmites, pillars and rubble, in the chunk column itself.
        for px in 1..=CHUNK {
            for pz in 1..=CHUNK {
                let Some(sp) = span(px, pz) else { continue };
                let col = &columns[px as usize * PAD + pz as usize];
                let (wx, wz) = (wx0 + px, wz0 + pz);
                let tilt = surface::strata_tilt(seed, wx as f64, wz as f64);
                let roofed = sp.roof <= col.top();
                let air = sp.roof - sp.floor;
                let h = hash3(seed ^ 0x57a1, wx, sp.roof, wz);
                let mut rng = Rng::new(h);
                let t = tier(sp.s, h);
                // Not right at the mouth, where the passage is still open to the sky.
                let deep = roofed && sp.s > m(4.0);
                let colour = |rng: &mut Rng, y: i32| {
                    let rock = if rng.unit() < 0.3 { mat::FLOWSTONE } else { strata(seed, col, wx, y, wz, tilt) };
                    mat::cave_rock(rock, t)
                };
                // Full voxels only where the neighbouring chunks will never look.
                let inner = (2..CHUNK).contains(&px) && (2..CHUNK).contains(&pz);
                let mut floor_taken = false;
                if deep && air >= 4 && rng.unit() < 0.11 {
                    let v = colour(&mut rng, sp.roof);
                    if inner && air <= 9 && rng.unit() < 0.16 {
                        pillar(grid, decor, (px, pz), (wx, wz), sp, v);
                        floor_taken = true;
                    } else {
                        let most = (air * CELLS) as f32 * 0.45;
                        let len = (rng.range(3.0, most.min(22.0).max(3.5)) as i32).max(2);
                        let mut top = sp.roof * CELLS;
                        let mut w0 = if len > 12 { 4 } else if len > 6 { 3 } else { 2 };
                        if inner && len >= 14 {
                            // The biggest hang from a voxel of solid rock.
                            grid.set(px, sp.roof - 1, pz, v);
                            top -= CELLS;
                            w0 = 3;
                        }
                        spire(decor, wx, wz, top, len - (sp.roof * CELLS - top), w0, true, v, &mut rng);
                        // Under a dripping tip a stalagmite often grows to meet it.
                        if rng.unit() < 0.45 {
                            let up = (len as f32 * rng.range(0.35, 0.7)) as i32;
                            let up = up.min((air * CELLS - len) / 2).max(0);
                            if up >= 2 {
                                spire(decor, wx, wz, sp.floor * CELLS, up, w0.min(3), false, v, &mut rng);
                                floor_taken = true;
                            }
                        }
                    }
                }
                let g = hash3(seed ^ 0x57a2, wx, sp.floor, wz);
                let mut rng = Rng::new(g);
                if !floor_taken && deep && air >= 4 && unit(g) < 0.05 {
                    let v = colour(&mut rng, sp.floor);
                    let up = (rng.range(3.0, ((air * CELLS) as f32 * 0.35).clamp(3.5, 14.0))) as i32;
                    spire(decor, wx, wz, sp.floor * CELLS, up, if up > 8 { 3 } else { 2 }, false, v, &mut rng);
                    floor_taken = true;
                }
                if !floor_taken && sp.floor <= col.top() && unit(g.rotate_left(16)) < 0.2 {
                    let v = |rng: &mut Rng| mat::cave_rock(if rng.unit() < 0.5 { mat::GRAVEL } else { strata(seed, col, wx, sp.floor, wz, tilt) }, t);
                    rubble(decor, wx, sp.floor, wz, &mut rng, v);
                }
            }
        }
    }
}

/// Darkness step (0 to 3) at `s` voxels in from the mouth, dithered by `h`.
fn tier(s: f32, h: u32) -> u8 {
    let s = s + (unit(h.rotate_left(5)) - 0.5) * m(2.0);
    match s {
        s if s < m(5.0) => 0,
        s if s < m(9.0) => 1,
        s if s < m(14.0) => 2,
        _ => 3,
    }
}

/// The strata rock at world voxel `(wx, y, wz)`, with ragged layer edges.
fn strata(seed: u32, col: &Column, wx: i32, y: i32, wz: i32, tilt: f32) -> Voxel {
    let h = hash3(seed ^ 0x57a9, wx, y, wz);
    let jitter = if unit(h) < 0.3 { (h % 3) as i32 - 1 } else { 0 };
    mat::strata(seed, col.biome, y + jitter * 2, tilt)
}

/// A tapering spire of boxes on the voxel column `(wx, wz)`, from the face at cell
/// height `from`: `len` cells down (a stalactite) or up (a stalagmite), `w0` cells wide
/// at the root and one at the tip.
#[allow(clippy::too_many_arguments)]
fn spire(out: &mut Vec<PlantBox>, wx: i32, wz: i32, from: i32, len: i32, w0: i32, down: bool, v: Voxel, rng: &mut Rng) {
    if len <= 0 {
        return;
    }
    let widths: Vec<i32> = (1..=w0.clamp(1, 4)).rev().collect();
    let n = widths.len() as i32;
    // The root steps are short and the tip is long, like a drip.
    let share = (len / (n + 1)).max(1);
    let mut y = from;
    let mut left = len;
    for (i, &w) in widths.iter().enumerate() {
        if left <= 0 {
            break;
        }
        let mut h = if i as i32 == n - 1 { left } else { share.min(left) };
        left -= h;
        let off = (CELLS - w + rng.int(0, 1)) / 2;
        let (x, z) = (wx * CELLS + off.min(CELLS - w), wz * CELLS + off.min(CELLS - w));
        let (lo, hi) = if down {
            // The underside of a hanging box is drawn half a cell up from its foot, and
            // only at some heights within a voxel (see `PlantBox::thin`).
            let mut lo = y - h;
            if lo.rem_euclid(CELLS) == 1 {
                lo -= 1;
                h += 1;
                left -= 1;
            }
            let r = (lo, y);
            y = lo;
            r
        } else {
            let r = (y, y + h);
            y += h;
            r
        };
        out.push(PlantBox {
            lo: [x, lo, z],
            hi: [x + w, hi, z + w],
            voxel: v,
            sway_lo: false,
            sway_hi: false,
            thin: if down { 1 << 3 } else { 0 },
        });
    }
}

/// A column of rock from floor to roof, flared where it meets them.
fn pillar(grid: &mut Grid, out: &mut Vec<PlantBox>, (px, pz): (i32, i32), (wx, wz): (i32, i32), sp: Span, v: Voxel) {
    for y in sp.floor..sp.roof {
        grid.set(px, y, pz, v);
    }
    let (x, z) = (wx * CELLS, wz * CELLS);
    let ring = |lo: i32, hi: i32, grow: i32, down: bool| PlantBox {
        lo: [x - grow, lo, z - grow],
        hi: [x + CELLS + grow, hi, z + CELLS + grow],
        voxel: v,
        sway_lo: false,
        sway_hi: false,
        thin: if down { 1 << 3 } else { 0 },
    };
    let (f, r) = (sp.floor * CELLS, sp.roof * CELLS);
    out.push(ring(f, f + 3, 1, false));
    out.push(ring(f, f + 1, 2, false));
    // Bottoms at 2 and 0 cells into a voxel, which a hanging box's underside can be.
    out.push(ring(r - 2, r, 1, true));
    out.push(ring(r - 4, r - 2, 0, true));
}

/// Loose stones on the floor of voxel column `(wx, wz)`, whose lowest air is `y`.
fn rubble(out: &mut Vec<PlantBox>, wx: i32, y: i32, wz: i32, rng: &mut Rng, mut colour: impl FnMut(&mut Rng) -> Voxel) {
    let base = [wx * CELLS, y * CELLS, wz * CELLS];
    let mut add = |lx: i32, lz: i32, w: i32, d: i32, h: i32, v: Voxel| {
        out.push(PlantBox {
            lo: [base[0] + lx, base[1], base[2] + lz],
            hi: [base[0] + lx + w, base[1] + h, base[2] + lz + d],
            voxel: v,
            sway_lo: false,
            sway_hi: false,
            thin: 0,
        });
    };
    if rng.unit() < 0.25 {
        // A fallen block.
        let v = colour(rng);
        add(rng.int(0, 1), rng.int(0, 1), 3, 3, rng.int(1, 2), v);
        return;
    }
    for (qx, qz) in [(0, 0), (2, 0), (0, 2), (2, 2)] {
        if rng.unit() < 0.45 {
            let v = colour(rng);
            let (w, d) = if rng.unit() < 0.5 { (2, 2) } else { (1, 2) };
            add(qx, qz + rng.int(0, 2 - d), w, d, 1, v);
        }
    }
}

/// Rubble and ferns on the floors of small ravines.
fn ravine_floors(seed: u32, sampler: &Sampler, columns: &[Column], (wx0, wz0): (i32, i32), (lx0, lz0): (i32, i32), out: &mut Vec<PlantBox>) {
    if sampler.ravines().is_empty() {
        return;
    }
    for px in 1..=CHUNK {
        for pz in 1..=CHUNK {
            let col = &columns[px as usize * PAD + pz as usize];
            if !matches!(col.cover, Cover::Scree | Cover::Land) || col.water.is_some() || col.clear < m(1.0) {
                continue;
            }
            let Some(inside) = sampler.ravine_at(lx0 + px, lz0 + pz) else { continue };
            let (wx, wz) = (wx0 + px, wz0 + pz);
            let h = hash3(seed ^ 0x5a44, wx, 0, wz);
            let mut rng = Rng::new(h);
            let y = col.top() + 1;
            // Ferns in the shade at the foot of the walls, on soil.
            if col.cover == Cover::Land && inside < m(1.4) && unit(h) < 0.3 && (3..=CHUNK - 2).contains(&px) && (3..=CHUNK - 2).contains(&pz) {
                fern(out, wx, y, wz, &mut rng);
                continue;
            }
            if unit(h.rotate_left(9)) < if col.cover == Cover::Scree { 0.35 } else { 0.1 } {
                let tilt = surface::strata_tilt(seed, wx as f64, wz as f64);
                rubble(out, wx, y, wz, &mut rng, |rng| if rng.unit() < 0.4 { mat::GRAVEL } else { strata(seed, col, wx, y - 1, wz, tilt) });
            }
        }
    }
}

/// A small fern: fronds arching out from a crown on voxel column `(wx, wz)`, standing
/// on voxel `y - 1`.
fn fern(out: &mut Vec<PlantBox>, wx: i32, y: i32, wz: i32, rng: &mut Rng) {
    let (cx, cy, cz) = (wx * CELLS + 2, y * CELLS, wz * CELLS + 2);
    let fronds = rng.int(5, 7);
    let turn = rng.range(0.0, std::f32::consts::TAU);
    let leaf = |rng: &mut Rng| if rng.unit() < 0.3 { mat::IVY_LEAF } else { mat::LEAVES_JUNGLE };
    let mut add = |x: i32, y: i32, z: i32, w: i32, d: i32, v: Voxel, sway: bool| {
        out.push(PlantBox { lo: [x, y, z], hi: [x + w, y + 1, z + d], voxel: v, sway_lo: sway, sway_hi: sway, thin: 0 });
    };
    add(cx - 1, cy, cz - 1, 2, 2, mat::LEAVES_JUNGLE, false);
    for k in 0..fronds {
        let a = turn + k as f32 / fronds as f32 * std::f32::consts::TAU + rng.range(-0.25, 0.25);
        let (dx, dz) = (a.cos(), a.sin());
        let reach = rng.int(4, 6);
        let v = leaf(rng);
        for i in 1..=reach {
            // Up over the first cells, level, then drooping at the tip.
            let rise = match i {
                1 => 1,
                2 => 2,
                _ if i == reach => 1,
                _ => 2,
            };
            let (x, z) = (cx + (dx * i as f32).round() as i32, cz + (dz * i as f32).round() as i32);
            add(x, cy + rise, z, 1, 1, v, i > 1);
        }
    }
}

/// Voxel access to a chunk column's chunks by world height and padded column.
struct Grid<'a> {
    out: &'a mut [(i32, PaddedChunk)],
    /// World height of the bottom of the lowest chunk.
    base: i32,
}

impl Grid<'_> {
    fn get(&self, px: i32, y: i32, pz: i32) -> Option<Voxel> {
        let py = y - self.base;
        if py < 0 {
            return None;
        }
        let (ci, ly) = ((py / CHUNK) as usize, py.rem_euclid(CHUNK) as usize);
        self.out.get(ci).map(|(_, c)| c.get(px as usize, ly + 1, pz as usize))
    }

    /// Sets a voxel in its own chunk and in the borders of the chunks above and below.
    fn set(&mut self, px: i32, y: i32, pz: i32, v: Voxel) {
        let py = y - self.base;
        if py < 0 {
            return;
        }
        let (ci, ly) = ((py / CHUNK) as usize, py.rem_euclid(CHUNK) as usize);
        let (x, z) = (px as usize, pz as usize);
        if let Some((_, c)) = self.out.get_mut(ci) {
            c.set(x, ly + 1, z, v);
        }
        if ly == 0 && ci > 0 {
            self.out[ci - 1].1.set(x, PAD - 1, z, v);
        }
        if ly == CHUNK as usize - 1
            && let Some((_, c)) = self.out.get_mut(ci + 1)
        {
            c.set(x, 0, z, v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{continent::{SimConfig, generate}, macro_layout::cell_desc};

    /// Prints how many caves open per square kilometre and where to look at some:
    /// `cargo test --release -p worldgen cave_survey -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn cave_survey() {
        let seed = 1;
        let c = generate(cell_desc(seed, 0, 0), SimConfig { size: 1024 });
        let n = MACRO_SIZE / CELL;
        let (mut caves, mut land_cells) = (Vec::new(), 0);
        let t0 = std::time::Instant::now();
        for gz in 0..n {
            for gx in 0..n {
                let (x, z) = (gx * CELL + CELL / 2, gz * CELL + CELL / 2);
                let i = (z as f32 / c.cell_size()) as usize * c.size + (x as f32 / c.cell_size()) as usize;
                if c.ocean[i] {
                    continue;
                }
                land_cells += 1;
                caves.extend(plan(seed, &c, gx, gz));
            }
        }
        let km2 = land_cells as f32 * (CELL as f32 / m(1000.0)).powi(2);
        eprintln!(
            "{} caves on {km2:.0} km² of land: {:.2} per km² ({:.1} ms per cell)",
            caves.len(),
            caves.len() as f32 / km2,
            t0.elapsed().as_secs_f64() * 1000.0 / land_cells as f64
        );
        let mut lengths: Vec<f32> = caves.iter().map(|c| c.length).collect();
        lengths.sort_by(f32::total_cmp);
        if !lengths.is_empty() {
            eprintln!("length p10 {:.0} m, median {:.0} m, p90 {:.0} m", lengths[lengths.len() / 10], lengths[lengths.len() / 2], lengths[lengths.len() * 9 / 10]);
        }
        for cave in caves.iter().step_by((caves.len() / 12).max(1)).take(12) {
            let (m0, i0) = (cave.mouth, cave.inside);
            eprintln!(
                "  cave {:.0} m: --pos {:.0},{:.0},{:.0} --look {:.0},-5   inside --pos {:.0},{:.0},{:.0}",
                cave.length, m0.x, m0.y, m0.z, cave.yaw, i0.x, i0.y, i0.z
            );
        }
    }

    #[test]
    fn caves_are_the_same_from_every_chunk() {
        // A cave crossing a chunk border is carved the same from both sides.
        let seed = 1;
        let c = generate(cell_desc(seed, 0, 0), SimConfig { size: 256 });
        let n = MACRO_SIZE / CELL;
        let mut found = 0;
        'cells: for gz in 0..n {
            for gx in 0..n {
                for cave in plan(seed, &c, gx, gz) {
                    for wx in cave.lo.0..=cave.hi.0 {
                        for wz in cave.lo.1..=cave.hi.1 {
                            let a = cave.span(wx, wz).map(|s| (s.floor, s.roof));
                            let again = plan(seed, &c, gx, gz);
                            assert_eq!(a, again[0].span(wx, wz).map(|s| (s.floor, s.roof)));
                            if a.is_some() {
                                found += 1;
                                if found > 50 {
                                    break 'cells;
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(found > 0, "no cave in the test continent");
    }
}
