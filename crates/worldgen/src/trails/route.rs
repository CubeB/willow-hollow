//! Finding a way across the land: best-first search on the continent grid, then turning
//! the staircase of cells into a line someone might actually have walked.

use std::{cmp::Reverse, collections::BinaryHeap};

use glam::{Vec2, vec2};

use crate::{
    continent::{Continent, NO_LAKE},
    noise::perlin2,
    scale::OLD_VOXEL,
    surface::MIN_RIVER_Q,
};

/// Search gives up after expanding this many cells.
const MAX_EXPANDED: u32 = 400_000;
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
        }
    }

    fn step_cost(&self, a: usize, b: usize, dist: f32) -> Option<f32> {
        let c = self.c;
        if c.ocean[b] || c.lake[b] != NO_LAKE {
            return None;
        }
        let slope = (c.height[b] - c.height[a]).abs() / dist;
        let mut cost = dist * (1.0 + 6.0 * slope + 60.0 * slope * slope);
        let q = c.discharge[b];
        if q >= MIN_RIVER_Q {
            // Crossing water costs more the wider it is: a brook is nothing, a great
            // river is worth a long detour to find a good place to bridge.
            let width = Continent::river_width(q);
            cost += 40.0 + width * width * 2.0 + if width > 10.0 { 300.0 + width * 25.0 } else { 0.0 };
        }
        if self.used[b] != 0 {
            cost *= REUSE;
        }
        Some(cost)
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
