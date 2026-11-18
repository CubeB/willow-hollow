//! The ways out across the land beyond the lanes (#64): to a field, the logging camp,
//! the quarry or a pasture. Found once, when the settlement's people are made, and kept
//! with its places (see `town.rs`), so that a day's walks are the same lookups as any
//! other.
//!
//! The ground is read from the world's columns as the land was laid out (`Sampler`),
//! a small square at a time and only where the search goes, with what stands on it: the
//! hedges, walls and fences between fields (`farmland::bounds`), the trees as they stand
//! this year (`features::place_trees`, felled about a logging camp by `logging::fell`)
//! and an orchard's rows. Nobody walks through water or a river's channel but where a
//! trail crosses it, up a cliff or over scree, through a boundary but at a gateway or
//! where it has fallen or died back, or up a step of more than two voxels (and of two
//! only where there is no easier way); the trails are walked gladly, and sown ground is
//! skirted where it can be. A way much further round than the straight line is not
//! looked for: there is taken to be none.

use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap, HashSet},
};

use glam::{IVec2, Vec2, Vec3};
use worldgen::{
    continent::Continent,
    features::{self, Library},
    noise::{CellHash, hash3, perlin2, unit},
    scale::m64,
    settlements::{
        farmland::{
            self,
            bounds::{self, Edge, Style},
            fields::Patch,
        },
        logging,
        years::When,
    },
    surface::{Column, Cover, Sampler},
};

/// The side of the squares the ground is read in, in columns.
const TILE: i32 = 32;
/// How far round a square its ground is read besides, for the boundaries along its edge.
const BORDER: i32 = 2;
/// What it costs to cross a column, in tenths of a plain one; nothing where it cannot be.
const BLOCKED: u8 = 0;
const TRAIL: u8 = 6;
const PLAIN: u8 = 10;
/// Across a sown field, or among an orchard's rows.
const SOWN: u8 = 13;
/// Up or down a step of two voxels, besides.
const BANK: u16 = 30;
/// How much the search is drawn straight on toward where it is going, against finding
/// the very cheapest way: a little, and it looks at far less ground.
const HASTE: f32 = 1.15;
/// The furthest a way out is looked for, in voxels as the crow flies.
pub const FURTHEST: f32 = worldgen::scale::m(700.0);
/// The most columns looked at in finding one way.
const MOST: usize = 250_000;
/// How much further than the straight line, at a plain column's cost, a way is looked
/// for, and a little besides: past that there is taken to be none.
const ROUND: f32 = 2.0;
const ROUND_BESIDES: f32 = 400.0;
/// How far apart the points of a way found are, along it.
const SPACING: f32 = 2.0;

/// Ground kept clear of: a rectangle of columns, inclusive, or the ellipse within it.
#[derive(Clone, Copy, Debug)]
pub struct Keep {
    pub lo: IVec2,
    pub hi: IVec2,
    pub round: bool,
}

impl Keep {
    pub fn rect(lo: IVec2, hi: IVec2) -> Keep {
        Keep { lo: lo.min(hi), hi: lo.max(hi), round: false }
    }

    fn holds(&self, x: i32, z: i32) -> bool {
        if x < self.lo.x || x > self.hi.x || z < self.lo.y || z > self.hi.y {
            return false;
        }
        if !self.round {
            return true;
        }
        let mid = (self.lo.as_vec2() + self.hi.as_vec2() + 1.0) * 0.5;
        let half = (self.hi - self.lo + IVec2::ONE).as_vec2() * 0.5;
        ((Vec2::new(x as f32 + 0.5, z as f32 + 0.5) - mid) / half).length_squared() < 1.0
    }
}

/// A settlement's own ground, which is walked only along its lanes and the trails
/// through it: its yards, gardens and the things that stand about its houses are no way
/// out.
pub struct Within {
    pub centre: Vec2,
    pub reach: f32,
    /// The columns along its lanes.
    pub ways: HashSet<(i32, i32), CellHash>,
}

impl Within {
    /// The ground of a settlement about `centre` out to `reach`, its lanes `lanes` (lines
    /// of points, continent-local) walked a little to either side of their middles.
    pub fn new(centre: Vec2, reach: f32, lanes: &[Vec<Vec2>]) -> Within {
        let mut ways = HashSet::default();
        for lane in lanes {
            for w in lane.windows(2) {
                let n = (w[0].distance(w[1]) / 0.5).ceil().max(1.0) as usize;
                for k in 0..=n {
                    let p = w[0].lerp(w[1], k as f32 / n as f32);
                    for dz in -2..=2 {
                        for dx in -2..=2 {
                            let q = (p.x.floor() as i32 + dx, p.y.floor() as i32 + dz);
                            if Vec2::new(q.0 as f32 + 0.5, q.1 as f32 + 0.5).distance(p) <= 2.0 {
                                ways.insert(q);
                            }
                        }
                    }
                }
            }
        }
        Within { centre, reach, ways }
    }
}

/// A square of ground as read: the feet's level on each column, what crossing it costs,
/// and whether a boundary stands on it; row by row in z.
struct Tile {
    feet: Vec<i16>,
    cost: Vec<u8>,
    bound: Vec<bool>,
}

/// The land about a settlement, read as the search for a way goes.
pub struct Country<'a> {
    c: &'a Continent,
    seed: u32,
    when: When,
    /// The continent's corner in the world, in voxels.
    origin: (i32, i32),
    tiles: HashMap<(i32, i32), Tile, CellHash>,
    keep: Vec<Keep>,
    within: Option<Within>,
    /// Columns looked at by the searches so far, for the log.
    pub looked: usize,
}

/// Whether boundary `edge` on the column at world `(x, z)` stands there to be walked
/// into, for a world of `seed`: as `bounds::build` raises it, open at its gateways (but
/// for their posts), where a wall has tumbled or a hedge died back or been broken
/// through, where a fence's rails are gone, and between the scattered stones and scrub
/// of those out at the edge of the wild.
fn stands(edge: &Edge, x: i32, z: i32, seed: u32) -> bool {
    let h = hash3(seed ^ 0xb0d5, x, 0, z);
    let r = |k: u32| unit(h.rotate_left(k * 7).wrapping_mul(0x9e37_79b9 ^ k));
    let noise = |salt: u32, scale: f64| perlin2(seed ^ salt, x as f64 / m64(scale), z as f64 / m64(scale));
    let decay = edge.decay;
    let gap = noise(0xb0d6, 3.2) > 0.5 - 0.45 * decay;
    let fence_gap = noise(0xb0da, 3.0) > 0.7 - 0.4 * decay;
    match edge.style {
        _ if edge.gate => edge.core,
        Style::Wall => !gap,
        Style::Hedge => (gap && edge.core && r(1) < 0.3) || (!gap && (edge.core || r(6) < 0.7)),
        Style::Scrub => edge.core && noise(0xb0d9, 5.0) > 0.05 && r(1) < 0.7,
        Style::Stones => edge.core && r(1) < 0.1 + 0.12 * (1.0 - decay),
        Style::Thorn => !gap && (edge.core || r(6) < 0.5) && r(1) < 0.8,
        Style::Fence | Style::Hurdle | Style::Driftwood => edge.core && !fence_gap,
        // An old field's grassed bank is a layer high, and stepped over (#64).
        Style::Bank => false,
    }
}

/// A column's place in its tile.
fn split(x: i32, z: i32) -> ((i32, i32), usize) {
    let (tx, tz) = (x.div_euclid(TILE), z.div_euclid(TILE));
    ((tx, tz), ((z - tz * TILE) * TILE + (x - tx * TILE)) as usize)
}

const STEPS: [(i32, i32); 8] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)];

impl<'a> Country<'a> {
    /// The land of continent `c` (whose corner in the world is `origin`) as it is at
    /// `when`, for a world of `seed`, keeping clear of `keep`, and to the lanes `within`
    /// a settlement.
    pub fn new(c: &'a Continent, seed: u32, when: When, origin: (i32, i32), keep: Vec<Keep>, within: Option<Within>) -> Self {
        Country { c, seed, when, origin, tiles: HashMap::default(), keep, within, looked: 0 }
    }

    fn read(&self, tx: i32, tz: i32) -> Tile {
        let (x0, z0) = (tx * TILE, tz * TILE);
        let n = (TILE + 2 * BORDER) as usize;
        let (bx, bz) = (x0 - BORDER, z0 - BORDER);
        let sam = Sampler::new(self.c, self.seed, bx, bz, n as i32);
        let cols: Vec<Column> = (0..n * n).map(|i| sam.column(bx + (i / n) as i32, bz + (i % n) as i32)).collect();
        let patch = Patch::new(self.c, self.seed, bx, bz, n, &cols);
        let t = TILE as usize;
        let mut tile = Tile { feet: vec![0; t * t], cost: vec![PLAIN; t * t], bound: vec![false; t * t] };
        for j in 0..t {
            for i in 0..t {
                let (pi, pj) = (i + BORDER as usize, j + BORDER as usize);
                let col = &cols[pi * n + pj];
                let k = j * t + i;
                let (x, z) = (x0 + i as i32, z0 + j as i32);
                tile.feet[k] = (col.top() + 1) as i16;
                let wet = col.water_surface().is_some() || col.river < 0.0;
                let trail = col.worn.is_some() || col.cover == Cover::Trail;
                let rough = matches!(col.cover, Cover::RiverBed | Cover::LakeBed | Cover::SeaFloor | Cover::Cliff | Cover::Scree | Cover::Boulders | Cover::Shelf);
                tile.cost[k] = if trail {
                    // Over the water where a trail crosses it, by its bridge or its ford,
                    // at the water's level.
                    if let Some(w) = col.water_surface() {
                        tile.feet[k] = w as i16;
                    }
                    TRAIL
                } else if wet || rough {
                    BLOCKED
                } else if patch.at(pi, pj).is_some_and(|p| p.crop.tilled() || p.crop.trees()) {
                    SOWN
                } else {
                    PLAIN
                };
                tile.bound[k] = !patch.is_empty() && bounds::edge(&self.c.land, &patch, col, pi, pj).is_some_and(|e| stands(&e, x + self.origin.0, z + self.origin.1, self.seed));
                if let Some(w) = &self.within {
                    let inside = Vec2::new(x as f32 + 0.5, z as f32 + 0.5).distance(w.centre) < w.reach;
                    if inside && col.worn.is_none() && !w.ways.contains(&(x, z)) {
                        tile.cost[k] = BLOCKED;
                    }
                }
            }
        }
        let mut block = |x: i32, z: i32| {
            let (i, j) = (x - x0, z - z0);
            if (0..TILE).contains(&i) && (0..TILE).contains(&j) {
                tile.cost[(j * TILE + i) as usize] = BLOCKED;
            }
        };
        // The trunks of the trees standing this year, and the stumps of those felled.
        let mut trees = features::place_trees(self.seed, Library::shared(), &sam, x0, z0, TILE, self.origin);
        logging::fell(self.c, self.when, self.origin, &mut trees);
        for t in &trees {
            let (x, z) = (t.pos.x - self.origin.0, t.pos.z - self.origin.1);
            for (dx, dz) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
                block(x + dx, z + dz);
            }
        }
        for t in farmland::orchard::trees_near(self.c, self.seed, &sam, x0, z0, TILE, self.origin) {
            let (x, z) = (t.base.x - self.origin.0, t.base.z - self.origin.1);
            for (dx, dz) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
                block(x + dx, z + dz);
            }
        }
        for k in &self.keep {
            if k.hi.x < x0 || k.lo.x >= x0 + TILE || k.hi.y < z0 || k.lo.y >= z0 + TILE {
                continue;
            }
            for z in k.lo.y.max(z0)..=k.hi.y.min(z0 + TILE - 1) {
                for x in k.lo.x.max(x0)..=k.hi.x.min(x0 + TILE - 1) {
                    if k.holds(x, z) {
                        block(x, z);
                    }
                }
            }
        }
        tile
    }

    /// The column at `(x, z)`, continent-local: its feet's level, what crossing it
    /// costs, and whether a boundary stands on it.
    fn at(&mut self, x: i32, z: i32) -> (i32, u8, bool) {
        let (key, k) = split(x, z);
        if !self.tiles.contains_key(&key) {
            let tile = self.read(key.0, key.1);
            self.tiles.insert(key, tile);
        }
        let t = &self.tiles[&key];
        (t.feet[k] as i32, t.cost[k], t.bound[k])
    }

    /// Where the feet are on the column under `p`.
    pub fn feet(&mut self, p: Vec2) -> f32 {
        self.at(p.x.floor() as i32, p.y.floor() as i32).0 as f32
    }

    /// Whether `p` can be stood on: open ground, no boundary, clear of what is kept clear
    /// of.
    pub fn open(&mut self, p: Vec2) -> bool {
        let (_, cost, bound) = self.at(p.x.floor() as i32, p.y.floor() as i32);
        cost != BLOCKED && !bound
    }

    /// What stepping from `a` onto `b` costs, if it can be done at all.
    fn step(&mut self, a: (i32, i32), b: (i32, i32)) -> Option<u16> {
        let (fa, _, _) = self.at(a.0, a.1);
        let (fb, cost, bound) = self.at(b.0, b.1);
        if cost == BLOCKED || (fb - fa).abs() > 2 || bound {
            return None;
        }
        // Up or down a bank of two, a terrace's or a steep hillside's, only if there is
        // no easier way.
        Some(cost as u16 + if (fb - fa).abs() == 2 { BANK } else { 0 })
    }

    /// The way across the land to `to` from whichever of `from` it is shortest from,
    /// continent-local: which, and the points along it with the feet's level, that one
    /// first and `to` last; none if it would be far out of the way.
    pub fn way(&mut self, from: &[Vec2], to: Vec2) -> Option<(usize, Vec<Vec3>)> {
        let (i, cells) = self.search(from, to)?;
        let mut pts = self.pull(&cells);
        // From exactly where it was asked, to exactly where it was asked.
        if let Some(first) = pts.first_mut() {
            *first = Vec3::new(from[i].x, first.y, from[i].y);
        }
        if let Some(last) = pts.last_mut() {
            *last = Vec3::new(to.x, last.y, to.y);
        }
        Some((i, pts))
    }

    /// The columns of the cheapest way to `to` from any of `from`, near enough, by a
    /// weighted A* from all of them at once; and which it starts from.
    fn search(&mut self, from: &[Vec2], to: Vec2) -> Option<(usize, Vec<(i32, i32)>)> {
        let cell = |p: Vec2| (p.x.floor() as i32, p.y.floor() as i32);
        let goal_open = self.near_open(cell(to))?;
        let mut starts: HashMap<(i32, i32), usize, CellHash> = HashMap::default();
        for (i, &p) in from.iter().enumerate() {
            if p.distance(to) <= FURTHEST
                && let Some(c) = self.near_open(cell(p))
            {
                starts.entry(c).or_insert(i);
            }
        }
        let octile = |c: (i32, i32)| {
            let (dx, dz) = ((c.0 - goal_open.0).abs(), (c.1 - goal_open.1).abs());
            let (lo, hi) = (dx.min(dz), dx.max(dz));
            (hi - lo) as f32 * PLAIN as f32 + lo as f32 * PLAIN as f32 * 1.41
        };
        // At a little over a trail's cost: quick, and near enough the best.
        let h = |c: (i32, i32)| octile(c) * HASTE;
        // Nothing much further round than the straight line is looked at.
        let nearest = starts.keys().map(|&c| octile(c)).fold(f32::INFINITY, f32::min);
        if !nearest.is_finite() {
            return None;
        }
        let furthest = (nearest * ROUND + ROUND_BESIDES) as u32;
        let mut best: HashMap<(i32, i32), (u32, (i32, i32)), CellHash> = HashMap::default();
        let mut open = BinaryHeap::new();
        for &start in starts.keys() {
            best.insert(start, (0, start));
            open.push(Reverse((h(start) as u32, 0u32, start)));
        }
        let mut looked = 0;
        let mut end = None;
        while let Some(Reverse((_, g, c))) = open.pop() {
            if best.get(&c).is_some_and(|b| b.0 < g) {
                continue;
            }
            looked += 1;
            if c == goal_open || looked > MOST {
                end = (c == goal_open).then_some(c);
                break;
            }
            for (dx, dz) in STEPS {
                let n = (c.0 + dx, c.1 + dz);
                let Some(cost) = self.step(c, n) else { continue };
                // Not between two corners.
                if dx != 0 && dz != 0 && (self.step(c, (c.0 + dx, c.1)).is_none() || self.step(c, (c.0, c.1 + dz)).is_none()) {
                    continue;
                }
                let g2 = g + if dx != 0 && dz != 0 { cost as u32 * 141 / 100 } else { cost as u32 };
                if g2 + octile(n) as u32 > furthest {
                    continue;
                }
                if best.get(&n).is_none_or(|b| g2 < b.0) {
                    best.insert(n, (g2, c));
                    open.push(Reverse((g2 + h(n) as u32, g2, n)));
                }
            }
        }
        self.looked += looked;
        let mut c = end?;
        let mut cells = vec![c];
        while best[&c].1 != c {
            c = best[&c].1;
            cells.push(c);
        }
        cells.reverse();
        Some((starts[&c], cells))
    }

    /// The nearest open column to `c`, within a few.
    fn near_open(&mut self, c: (i32, i32)) -> Option<(i32, i32)> {
        for r in 0..=4i32 {
            for dz in -r..=r {
                for dx in -r..=r {
                    if dx.abs().max(dz.abs()) != r {
                        continue;
                    }
                    let n = (c.0 + dx, c.1 + dz);
                    let (_, cost, bound) = self.at(n.0, n.1);
                    if cost != BLOCKED && !bound {
                        return Some(n);
                    }
                }
            }
        }
        None
    }

    /// Whether the straight line from `a` to `b` keeps to columns that can be walked,
    /// none dearer than `worst` and no step on it higher than a voxel.
    fn straight(&mut self, a: (i32, i32), b: (i32, i32), worst: u8) -> bool {
        let (pa, pb) = (Vec2::new(a.0 as f32 + 0.5, a.1 as f32 + 0.5), Vec2::new(b.0 as f32 + 0.5, b.1 as f32 + 0.5));
        let n = (pa.distance(pb) / 0.4).ceil().max(1.0) as usize;
        let mut last = self.at(a.0, a.1).0;
        for k in 1..=n {
            let p = pa.lerp(pb, k as f32 / n as f32);
            let (feet, cost, bound) = self.at(p.x.floor() as i32, p.y.floor() as i32);
            if cost == BLOCKED || cost > worst || bound || (feet - last).abs() > 1 {
                return false;
            }
            last = feet;
        }
        true
    }

    /// The columns of a way pulled straight where the ground lets it, and set out as
    /// points [`SPACING`] apart with the feet's level under each.
    fn pull(&mut self, cells: &[(i32, i32)]) -> Vec<Vec3> {
        let mut corners = vec![cells[0]];
        let mut i = 0;
        while i + 1 < cells.len() {
            let mut worst = 0;
            let mut j = i + 1;
            let mut reach = j;
            // As far ahead as is clear, a stretch at a time.
            while j < cells.len() && j - i <= 64 {
                let c = self.at(cells[j].0, cells[j].1);
                worst = worst.max(if c.2 { u8::MAX } else { c.1 });
                if worst == u8::MAX && j > i + 1 {
                    break;
                }
                if j == i + 1 || self.straight(cells[i], cells[j], worst.max(PLAIN)) {
                    reach = j;
                }
                j += 1;
            }
            corners.push(cells[reach]);
            i = reach;
        }
        let mut pts = Vec::new();
        for w in corners.windows(2) {
            let (a, b) = (Vec2::new(w[0].0 as f32 + 0.5, w[0].1 as f32 + 0.5), Vec2::new(w[1].0 as f32 + 0.5, w[1].1 as f32 + 0.5));
            let n = (a.distance(b) / SPACING).ceil().max(1.0) as usize;
            for k in 0..n {
                let p = a.lerp(b, k as f32 / n as f32);
                pts.push(Vec3::new(p.x, self.feet(p), p.y));
            }
        }
        let last = corners[corners.len() - 1];
        let p = Vec2::new(last.0 as f32 + 0.5, last.1 as f32 + 0.5);
        pts.push(Vec3::new(p.x, self.feet(p), p.y));
        pts
    }
}

/// A point of a camp, given in its own frame (its front, the saw pit's end, along +x),
/// continent-local.
pub fn in_camp(camp: &logging::Camp, x: f32, z: f32) -> Vec2 {
    let (x, z) = match camp.turns & 3 {
        0 => (x, z),
        1 => (-z, x),
        2 => (-x, -z),
        _ => (z, -x),
    };
    Vec2::new(camp.at.x as f32 + 0.5 + x, camp.at.z as f32 + 0.5 + z)
}

/// What stands in a logging camp's yard, kept clear of on the way in: the lean-to, the
/// saw pit with its log, the chopping block and the corners the stacks, the cart and the
/// sawhorse stand in (as `logging::build` lays them out, there in their own frame).
pub fn camp_keep(camp: &logging::Camp) -> Vec<Keep> {
    let rect = |a: (i32, i32), b: (i32, i32)| {
        let (p, q) = (in_camp(camp, a.0 as f32, a.1 as f32), in_camp(camp, b.0 as f32, b.1 as f32));
        Keep::rect(p.floor().as_ivec2(), q.floor().as_ivec2())
    };
    vec![
        rect((-13, -6), (-6, 6)),
        rect((0, -2), (13, 2)),
        rect((-3, -3), (-2, -2)),
        rect((1, 4), (14, 13)),
        rect((-6, 4), (0, 12)),
        rect((-6, -12), (0, -4)),
        rect((2, -13), (14, -4)),
    ]
}
