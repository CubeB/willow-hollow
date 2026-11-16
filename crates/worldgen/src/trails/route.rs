//! Finding a way across the land: best-first search on the continent grid, then turning
//! the staircase of cells into a line someone might actually have walked.

use std::{cmp::Reverse, collections::BinaryHeap};

use glam::{Vec2, vec2};

use super::CrossingKind;
use crate::{
    biome::Biome,
    continent::{Continent, NO_LAKE},
    noise::perlin2,
    scale::{OLD_VOXEL, m},
    surface::MIN_RIVER_Q,
};

/// Search gives up after expanding this many cells.
const MAX_EXPANDED: u32 = 400_000;

/// Widest water, in trail steps, waded on stepping stones.
pub const FORD_SPAN: usize = 5;
/// Widest water, in trail steps, a timber bridge is put across: past this a trestle
/// would need so many bents in the current that only stone will do.
pub const TIMBER_SPAN: usize = 28;
/// A river at least this wide, in simulation units, is taken over on stone whatever
/// the span: its floods and the timber they carry down would take a wooden bridge away
/// within a few winters.
pub const STONE_RIVER: f32 = 16.0;
/// Below this width, in simulation units, a river is a stream whose water spreads slack
/// and shallow where a trail meets it at a slant or it opens out, so a trestle can
/// stand in it further out: timber reaches [`SLACK_SPAN`] more steps over it.
const SLACK_RIVER: f32 = 7.0;
const SLACK_SPAN: usize = 16;

/// What each kind of crossing costs to build, as so much easy ground walked, in
/// simulation units. People would far rather walk a long way round than build a stone
/// bridge: it takes a mason, a quarry and a summer, where a footbridge takes a few
/// felled trees and a ford nothing at all. So the stone bridge costs a fixed sum worth
/// several kilometres of walking, and a timber one only a few hundred metres, rising
/// with its span.
const FORD_COST: f32 = 40.0;
const FORD_PER_STEP: f32 = 8.0;
const TIMBER_COST: f32 = 220.0;
const TIMBER_PER_STEP: f32 = 30.0;
const STONE_COST: f32 = 7000.0;
const STONE_PER_STEP: f32 = 150.0;

/// How a trail gets over water `span` trail steps across, in a river `width`
/// simulation units wide (0 for standing water). `shallow` is whether it can be waded.
/// The router and the builder both decide by this, so a route is costed by the
/// crossing that will really be built.
pub fn crossing_kind(span: usize, width: f32, shallow: bool) -> CrossingKind {
    if span <= FORD_SPAN && shallow {
        CrossingKind::Stones
    } else if width < STONE_RIVER && span <= TIMBER_SPAN + if width < SLACK_RIVER { SLACK_SPAN } else { 0 } {
        CrossingKind::Plank
    } else {
        CrossingKind::Arch
    }
}

/// The span, in trail steps, a trail should expect to bridge to cross a river `width`
/// simulation units wide. The water at the surface spreads a little wider than the
/// simulated channel, and a trail seldom crosses quite square.
pub fn expected_span(width: f32) -> usize {
    (width * 1.3 + 1.0).ceil() as usize
}

/// The effort of getting across a river `width` simulation units wide, as so much easy
/// ground walked.
pub fn crossing_cost(width: f32) -> f32 {
    let span = expected_span(width);
    // Only brooks are shallow enough to wade.
    match crossing_kind(span, width, width < 3.0) {
        CrossingKind::Stones => FORD_COST + FORD_PER_STEP * span as f32,
        CrossingKind::Arch => STONE_COST + STONE_PER_STEP * span as f32,
        _ => TIMBER_COST + TIMBER_PER_STEP * span as f32 + 2.0 * (span * span) as f32,
    }
}

/// A slow unevenness over the land that a map does not show: a thicket here, a field
/// wall or a soft patch there, a way someone happened to go first. It bends routes in
/// long gentle curves, but only where the land leaves a choice, since slope and water
/// still cost far more. 1 to [`MEANDER`] + 1, smooth over about [`MEANDER_SCALE`].
const MEANDER: f32 = 0.6;
const MEANDER_SCALE: f32 = m(500.0);

fn meander(c: &Continent, i: usize) -> f32 {
    let cs = c.cell_size();
    let (x, z) = ((i % c.size) as f32 * cs, (i / c.size) as f32 * cs);
    let seed = c.desc.seed ^ 0x3ea7;
    let n = perlin2(seed, (x / MEANDER_SCALE) as f64, (z / MEANDER_SCALE) as f64)
        + 0.5 * perlin2(seed ^ 0x51, (x / MEANDER_SCALE * 2.3) as f64, (z / MEANDER_SCALE * 2.3) as f64);
    1.0 + MEANDER * (n * 0.7 + 0.5).clamp(0.0, 1.0)
}

/// Walking along an existing trail costs this fraction of breaking new ground, which is
/// what makes trails join up instead of running side by side.
const REUSE: f32 = 0.35;
/// Distance between neighbouring points on a finished trail, in voxels. Everything that
/// counts along a trail counts these, so its meaning does not change with voxel size.
const STEP: f32 = OLD_VOXEL;

pub struct Router<'a> {
    c: &'a Continent,
    /// Search state, reused between searches. A cell's entries are valid only if its
    /// stamp matches the current search.
    best: Vec<f32>,
    from: Vec<u32>,
    stamp: Vec<u32>,
    current: u32,
    /// Trail occupying each cell (0 for none, otherwise id + 1).
    pub used: Vec<u32>,
    /// How hard each cell's ground is to cross, as a multiple of easy open ground.
    terrain: Vec<f32>,
    /// Extra cost per unit distance of crossing each cell for being low ground at the
    /// edge of the sea or a lake (see [`shallows`]).
    shallows: Vec<f32>,
}

/// Extra cost, per simulation unit walked, of low ground at the edge of the sea or a
/// lake. On the map such a cell is land, but at the surface its low parts are under
/// water, so a trail across it ends up on a long bridge over shallows and marsh.
pub fn shallows(c: &Continent, i: usize) -> f32 {
    const NEAR: i32 = 2;
    let n = c.size as i32;
    let (x, z) = (i as i32 % n, i as i32 / n);
    let mut water = f32::MIN;
    for dz in -NEAR..=NEAR {
        for dx in -NEAR..=NEAR {
            let (nx, nz) = (x + dx, z + dz);
            if nx < 0 || nz < 0 || nx >= n || nz >= n {
                continue;
            }
            let j = (nz * n + nx) as usize;
            if c.ocean[j] {
                water = water.max(0.0);
            } else if c.lake[j] != NO_LAKE {
                water = water.max(c.lake[j]);
            }
        }
    }
    if water == f32::MIN {
        return 0.0;
    }
    // Within about 2 m of the water's height the ground floods in places.
    let above = c.height[i] - water;
    6.0 * (1.0 - ((above - 0.5) / 2.5).clamp(0.0, 1.0))
}

/// How hard a cell's ground is to cross, as a multiple of open level ground. People
/// keep to valley floors and gentle ground, and go around steep faces, exposed high
/// ground and dense old forest. Direction of travel is handled separately, in the grade.
pub fn terrain_factor(c: &Continent, i: usize) -> f32 {
    let n = c.size as i32;
    let cs = c.sim_cell_size();
    let (x, z) = (i as i32 % n, i as i32 / n);
    let h = |dx: i32, dz: i32| c.height[((z + dz).clamp(0, n - 1) * n + (x + dx).clamp(0, n - 1)) as usize];
    let here = c.height[i];
    // Steepness of the ground itself, whichever way it is crossed: a steep face is hard
    // going even along the contour, and beyond a point it is a scramble.
    let steep = [(1, 0), (-1, 0), (0, 1), (0, -1)]
        .iter()
        .map(|&(dx, dz)| (h(dx, dz) - here).abs())
        .fold(0.0f32, f32::max)
        / cs;
    // Height above the ground around: positive on knolls and ridges, negative in valleys.
    let r = 4;
    let around = [(r, 0), (-r, 0), (0, r), (0, -r), (r, r), (r, -r), (-r, r), (-r, -r)]
        .iter()
        .map(|&(dx, dz)| h(dx, dz))
        .sum::<f32>()
        / 8.0;
    let prominence = here - around;
    let mut f = 1.0 + 1.5 * (steep - 0.4).max(0.0) + 4.0 * (steep - 0.8).max(0.0);
    // Small hills are gone around rather than over.
    f += (prominence * 0.02).clamp(0.0, 0.4);
    // Beside a stream the ground is soft, and a trail kept there would wind across it
    // again and again. Better a terrace a little above.
    let q = |dx: i32, dz: i32| c.discharge[((z + dz).clamp(0, n - 1) * n + (x + dx).clamp(0, n - 1)) as usize];
    if [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)].iter().any(|&(dx, dz)| q(dx, dz) >= MIN_RIVER_Q) {
        f += 0.5;
    }
    // High ground is exposed and slow.
    f += ((here - 260.0) / 250.0).clamp(0.0, 0.8);
    f += match Biome::from_u8(c.biome[i]) {
        Biome::OldGrowth | Biome::Rainforest => 0.3,
        Biome::Alpine => 0.3,
        // Sodden ground: every step sinks, and a path across it needs a causeway.
        Biome::Marsh | Biome::Bog => 1.2,
        Biome::Mangrove => 1.5,
        _ => 0.0,
    };
    f.max(0.8) * meander(c, i)
}

impl<'a> Router<'a> {
    pub fn new(c: &'a Continent) -> Self {
        let n = c.size * c.size;
        Self {
            c,
            best: vec![0.0; n],
            from: vec![0; n],
            stamp: vec![0; n],
            current: 0,
            used: vec![0; n],
            terrain: (0..n).map(|i| terrain_factor(c, i)).collect(),
            shallows: (0..n).map(|i| shallows(c, i)).collect(),
        }
    }

    fn step_cost(&self, a: usize, b: usize, dist: f32) -> Option<f32> {
        let c = self.c;
        if c.ocean[b] || c.lake[b] != NO_LAKE {
            return None;
        }
        // The grade along the way of travel: walking along a contour is cheap, straight
        // up the fall line dear, so steep ground is climbed by traverses and zigzags.
        let grade = (c.height[b] - c.height[a]).abs() / dist;
        let terrain = (self.terrain[a] + self.terrain[b]) * 0.5;
        let mut cost = dist * terrain * (1.0 + 6.0 * grade + 90.0 * grade * grade);
        // Low ground beside the sea or a lake is shallows, marsh and mud flats at the
        // scale people walk: a trail kept to it would need a long bridge over water
        // that a path a little inland goes round dry-shod.
        cost += dist * self.shallows[b];
        // Getting over water costs what the crossing it needs would take to build.
        // Every cell with a stream counts, as a trail kept in one would cross it again
        // and again. A diagonal step between two river cells slips through the corner
        // where the river turns, but at the surface it crosses that river all the same.
        let mut q = c.discharge[b];
        let n = c.size;
        let (ax, az, bx, bz) = (a % n, a / n, b % n, b / n);
        if ax != bx && az != bz {
            let (p, r) = (az * n + bx, bz * n + ax);
            if c.discharge[p] >= MIN_RIVER_Q && c.discharge[r] >= MIN_RIVER_Q && !c.ocean[p] && !c.ocean[r] {
                q = q.max(c.discharge[p].min(c.discharge[r]));
            }
        }
        let crossing = if q >= MIN_RIVER_Q { crossing_cost(Continent::river_width(q)) } else { 0.0 };
        if self.used[b] != 0 {
            // Along a trail already made, over the crossing already built for it.
            cost = (cost + crossing * 0.1) * REUSE;
        } else {
            cost += crossing;
        }
        Some(cost)
    }

    /// Makes the ground within `r` voxels of the middle of cell `at` `factor` times as
    /// hard to cross, so trails go round it: a ruin nobody walks to.
    pub fn avoid(&mut self, at: usize, r: f32, factor: f32) {
        let n = self.c.size as i32;
        let k = (r / self.c.cell_size()).ceil() as i32;
        let (x, z) = (at as i32 % n, at as i32 / n);
        for dz in -k..=k {
            for dx in -k..=k {
                let (nx, nz) = (x + dx, z + dz);
                if nx >= 0 && nz >= 0 && nx < n && nz < n && ((dx * dx + dz * dz) as f32).sqrt() <= k as f32 {
                    self.terrain[(nz * n + nx) as usize] *= factor;
                }
            }
        }
    }

    /// The cheapest path of cells from `a` to `b`, both included.
    pub fn search(&mut self, a: usize, b: usize) -> Option<Vec<u32>> {
        let n = self.c.size as i32;
        // Heights and river widths are in simulation units, so distances must be too.
        let cs = self.c.sim_cell_size();
        let goal = ((b as i32 % n) as f32, (b as i32 / n) as f32);
        let estimate = |i: usize| ((i as i32 % n) as f32 - goal.0).hypot((i as i32 / n) as f32 - goal.1) * cs;
        self.current += 1;
        let stamp = self.current;
        self.stamp[a] = stamp;
        self.best[a] = 0.0;
        self.from[a] = a as u32;
        let mut open = BinaryHeap::new();
        open.push(Reverse(((estimate(a) * 8.0) as u32, a as u32)));
        let mut expanded = 0u32;
        while let Some(Reverse((_, i))) = open.pop() {
            let i = i as usize;
            if i == b {
                let mut path = vec![b as u32];
                let mut at = b;
                while at != a {
                    at = self.from[at] as usize;
                    path.push(at as u32);
                }
                path.reverse();
                return Some(path);
            }
            expanded += 1;
            if expanded > MAX_EXPANDED {
                return None;
            }
            let (x, z) = (i as i32 % n, i as i32 / n);
            let g = self.best[i];
            for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
                let (nx, nz) = (x + dx, z + dz);
                if nx < 0 || nz < 0 || nx >= n || nz >= n {
                    continue;
                }
                let j = (nz * n + nx) as usize;
                let dist = if dx != 0 && dz != 0 { cs * std::f32::consts::SQRT_2 } else { cs };
                let Some(cost) = self.step_cost(i, j, dist) else { continue };
                let tentative = g + cost;
                if self.stamp[j] != stamp || tentative < self.best[j] {
                    self.stamp[j] = stamp;
                    self.best[j] = tentative;
                    self.from[j] = i as u32;
                    open.push(Reverse((((tentative + estimate(j)) * 8.0) as u32, j as u32)));
                }
            }
        }
        None
    }
}

/// Ramer-Douglas-Peucker: drops points that lie within `tolerance` of the line.
fn simplify(points: &[Vec2], tolerance: f32) -> Vec<Vec2> {
    fn keep(points: &[Vec2], lo: usize, hi: usize, tolerance: f32, out: &mut [bool]) {
        if hi <= lo + 1 {
            return;
        }
        let (a, b) = (points[lo], points[hi]);
        let ab = b - a;
        let len2 = ab.length_squared().max(1e-6);
        let (mut worst, mut at) = (0.0, lo);
        for (k, &p) in points.iter().enumerate().take(hi).skip(lo + 1) {
            let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
            let d = (p - (a + ab * t)).length();
            if d > worst {
                (worst, at) = (d, k);
            }
        }
        if worst > tolerance {
            out[at] = true;
            keep(points, lo, at, tolerance, out);
            keep(points, at, hi, tolerance, out);
        }
    }
    let mut mask = vec![false; points.len()];
    mask[0] = true;
    mask[points.len() - 1] = true;
    keep(points, 0, points.len() - 1, tolerance, &mut mask);
    points.iter().zip(&mask).filter(|(_, m)| **m).map(|(p, _)| *p).collect()
}

/// 1-2-1 smoothing, ends fixed.
fn relax(points: &mut [Vec2]) {
    if points.len() < 3 {
        return;
    }
    let copy = points.to_vec();
    for k in 1..points.len() - 1 {
        points[k] = (copy[k - 1] + copy[k] * 2.0 + copy[k + 1]) * 0.25;
    }
}

fn catmull(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2, t: f32) -> Vec2 {
    let (t2, t3) = (t * t, t * t * t);
    (p1 * 2.0 + (p2 - p0) * t + (p0 * 2.0 - p1 * 5.0 + p2 * 4.0 - p3) * t2 + (p1 * 3.0 - p0 - p2 * 3.0 + p3) * t3) * 0.5
}

/// Evenly spaced points, [`STEP`] apart, along a smooth curve through `control`.
pub fn resample(control: &[Vec2]) -> Vec<Vec2> {
    let at = |k: isize| control[k.clamp(0, control.len() as isize - 1) as usize];
    let mut dense = Vec::new();
    for k in 0..control.len() as isize - 1 {
        let span = (at(k + 1) - at(k)).length();
        let steps = (span * 2.0 / STEP).ceil().max(1.0) as usize;
        for s in 0..steps {
            dense.push(catmull(at(k - 1), at(k), at(k + 1), at(k + 2), s as f32 / steps as f32));
        }
    }
    dense.push(*control.last().expect("control points"));

    let mut out = vec![dense[0]];
    let mut carried = 0.0;
    for pair in dense.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let seg = (b - a).length();
        let mut along = STEP - carried;
        while along <= seg {
            out.push(a + (b - a) * (along / seg.max(1e-6)));
            along += STEP;
        }
        carried = (carried + seg).rem_euclid(STEP);
    }
    out
}

/// Turns a run of grid cells into a walkable curve between two exact end points.
pub fn smooth(c: &Continent, cells: &[u32], start: Vec2, end: Vec2, salt: u32) -> Vec<Vec2> {
    let cs = c.cell_size();
    let n = c.size as u32;
    let mut pts: Vec<Vec2> = cells
        .iter()
        .map(|&i| vec2((i % n) as f32 + 0.5, (i / n) as f32 + 0.5) * cs)
        .collect();
    let (first, last) = (0, pts.len() - 1);
    pts[first] = start;
    pts[last] = end;
    let mut pts = simplify(&pts, cs * 0.6);
    for _ in 0..2 {
        relax(&mut pts);
    }
    let mut out = resample(&pts);
    // A slow wander, so even long straights are not ruled lines. Faded at both ends so
    // the trail still arrives exactly where it should.
    let len = out.len();
    if len > 2 {
        for k in 1..len - 1 {
            let tangent = (out[(k + 1).min(len - 1)] - out[k - 1]).normalize_or_zero();
            let normal = vec2(-tangent.y, tangent.x);
            let fade = ((k.min(len - 1 - k)) as f32 / 40.0).min(1.0);
            let wander = perlin2(c.desc.seed ^ salt, k as f64 / 70.0, salt as f64 * 0.37) * 5.0 * OLD_VOXEL;
            out[k] += normal * wander * fade;
        }
    }
    out
}
