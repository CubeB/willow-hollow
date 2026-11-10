//! Assembles a continent's network: settles the sites on real ground, joins them up,
//! lays each trail over the terrain and builds what it needs to cross water.

use std::{
    cmp::Reverse,
    collections::{BTreeMap, BinaryHeap, HashMap},
};

use glam::{IVec3, Vec2, Vec3, vec2};
use voxel::Voxel;

use super::{
    Crossing, CrossingKind, Mooring, Network, Pad, Site, TrailPoint,
    route::{self, Router},
    sites,
};
use crate::{
    continent::Continent,
    features::{
        landmarks::{self, Landmark, Palette},
        parts,
        shape::{Rng, Shape, rotate},
    },
    materials as mat,
    noise::{hash2, perlin2, unit},
    scale::{BLOCK, OLD_VOXEL, m},
    surface::{Column, Cover, Sampler},
};

/// Half-width of the worn strip by the traffic a trail carries: (traffic, metres). A
/// remote trace is a single broken line, a footpath a narrow one, a track a broad firm
/// strip, and a road between busy places wider still. Widths blend between these.
/// Today's landmarks bring well-used trails up to tracks; it takes settlements, whose
/// trips count many times more, to make roads.
const WIDTHS: [(f32, f32); 4] = [(5.0, 0.25), (20.0, 0.55), (90.0, 0.95), (500.0, 1.5)];
/// Narrower than this, a trail is a trace that comes and goes.
const TRACE: f32 = m(0.4);
/// Trails at least this wide get lanterns.
const LIT: f32 = m(0.8);
/// Trips between sites further apart than this count for less, in voxels.
const TRIP_FALLOFF: f32 = m(2500.0);
/// When fitting a trail to the ground: trail steps between the points that may shift
/// sideways, and how far each step of the shift goes.
const FIT_SPACING: usize = 8;
const FIT_STEP: f32 = m(3.2);
const FIT_SHIFTS: usize = 7;
/// Steepest a trail may rise, as a slope.
const MAX_SLOPE: f32 = 0.45;
/// Longest water a trail will bridge, in trail steps.
const MAX_SPAN: usize = 90;
/// Longest link in the site graph.
const MAX_LINK: f32 = m(4800.0);
/// Trail steps between lanterns near landmarks, and much further apart elsewhere.
const LANTERN_NEAR: usize = 64;
const LANTERN_FAR: usize = 170;
/// How far from a landmark counts as near it.
const NEAR_SITE: f32 = m(240.0);

/// Columns under a list of points. Reads the bare terrain: the network being built is
/// not attached to the continent yet.
fn sample(c: &Continent, seed: u32, pts: &[Vec2]) -> Vec<Column> {
    let mut out = Vec::with_capacity(pts.len());
    for run in pts.chunks(48) {
        let (mut lo, mut hi) = (Vec2::MAX, Vec2::MIN);
        for p in run {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        let (x0, z0) = (lo.x.floor() as i32, lo.y.floor() as i32);
        let span = ((hi.x.ceil() as i32 - x0).max(hi.y.ceil() as i32 - z0)).max(1) + 2;
        let sampler = Sampler::new(c, seed, x0, z0, span);
        out.extend(run.iter().map(|p| sampler.column(p.x.floor() as i32, p.y.floor() as i32)));
    }
    out
}

fn is_wet(col: &Column) -> bool {
    col.water_surface().is_some()
}

/// Half-width of the worn strip for a trail carrying `traffic`, in voxels.
pub fn width_for(traffic: f32) -> f32 {
    let t = traffic.max(WIDTHS[0].0).ln();
    let metres = WIDTHS
        .windows(2)
        .find(|w| t <= w[1].0.ln())
        .map_or(WIDTHS[WIDTHS.len() - 1].1, |w| {
            let f = (t - w[0].0.ln()) / (w[1].0.ln() - w[0].0.ln());
            w[0].1 + (w[1].1 - w[0].1) * f
        });
    m(metres)
}

/// Ground height over which trails narrow to a mountain path, in voxels.
const HIGH: (f32, f32) = (m(100.0), m(200.0));
/// Grade over which trails narrow, rising so much per step along.
const STEEP: (f32, f32) = (0.2, 0.4);
/// Half-width of a path over a peak or up a steep face: one voxel or two wide.
const MOUNTAIN_PATH: f32 = m(0.3);

fn smoothstep(lo: f32, hi: f32, x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Over high ground and up steep slopes, however busy, a trail is a narrow worn line
/// picking its way along, and it widens again gradually coming down. Narrow enough, it
/// becomes a trace that comes and goes, so it vanishes over bare rock and picks up again.
fn narrow_high(widths: &[f32], cols: &[Column]) -> Vec<f32> {
    let n = cols.len();
    let rough: Vec<f32> = (0..n)
        .map(|i| {
            let (a, b) = (i.saturating_sub(4), (i + 4).min(n - 1));
            let grade = (cols[b].ground - cols[a].ground).abs() / ((b - a).max(1) as f32 * OLD_VOXEL);
            smoothstep(HIGH.0, HIGH.1, cols[i].ground).max(smoothstep(STEEP.0, STEEP.1, grade) * 0.8)
        })
        .collect();
    // Averaged along the way, so the width blends without visible steps.
    const BLEND: usize = 20;
    (0..n)
        .map(|i| {
            let (lo, hi) = (i.saturating_sub(BLEND), (i + BLEND + 1).min(n));
            let t = rough[lo..hi].iter().sum::<f32>() / (hi - lo) as f32;
            let w = widths[i];
            w + (w.min(MOUNTAIN_PATH) - w) * t
        })
        .collect()
}

/// The narrowest a trail is worn.
fn widths_min() -> f32 {
    m(WIDTHS[0].1)
}

/// The class a trail of this half-width reads as, for maps and statistics.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TrailClass {
    Trace,
    Footpath,
    Track,
    Road,
}

pub fn class_of(half_width: f32) -> TrailClass {
    if half_width < TRACE {
        TrailClass::Trace
    } else if half_width < m(0.75) {
        TrailClass::Footpath
    } else if half_width < m(1.25) {
        TrailClass::Track
    } else {
        TrailClass::Road
    }
}

/// Fits a trail to the ground at a finer scale than the map it was routed on: each
/// point every [`FIT_SPACING`] steps may shift sideways, and the shifts chosen keep the
/// grade gentle, stay out of water and off river banks, and otherwise keep to the
/// line. So the trail bends round small rises and dips and finds the narrow place to
/// cross a stream, and every bend has a reason. The ends stay where they are.
fn fit(c: &Continent, seed: u32, pts: &[Vec2]) -> Vec<Vec2> {
    let n = pts.len();
    if n < FIT_SPACING * 3 {
        return pts.to_vec();
    }
    let mut at: Vec<usize> = (0..n).step_by(FIT_SPACING).collect();
    if *at.last().expect("points") != n - 1 {
        at.push(n - 1);
    }
    let mid = FIT_SHIFTS / 2;
    let shift = |k: usize| (k as f32 - mid as f32) * FIT_STEP;
    let normals: Vec<Vec2> = at
        .iter()
        .map(|&i| {
            let t = (pts[(i + 2).min(n - 1)] - pts[i.saturating_sub(2)]).normalize_or_zero();
            vec2(-t.y, t.x)
        })
        .collect();
    let mut probes = Vec::with_capacity(at.len() * FIT_SHIFTS);
    for (j, &i) in at.iter().enumerate() {
        for k in 0..FIT_SHIFTS {
            probes.push(pts[i] + normals[j] * shift(k));
        }
    }
    let cols = sample(c, seed, &probes);
    let last = at.len() - 1;
    let place = |j: usize, k: usize| -> f32 {
        if (j == 0 || j == last) && k != mid {
            return f32::INFINITY;
        }
        let col = &cols[j * FIT_SHIFTS + k];
        let mut cost = 0.35 * (k as f32 - mid as f32).abs();
        if is_wet(col) {
            cost += 400.0;
        } else if col.river < m(2.4) {
            cost += 30.0;
        }
        cost
    };
    // Cheapest shifts by dynamic programming: each point's shift differs from the one
    // before by at most one step.
    let mut best = vec![f32::INFINITY; at.len() * FIT_SHIFTS];
    let mut from = vec![0u8; at.len() * FIT_SHIFTS];
    for k in 0..FIT_SHIFTS {
        best[k] = place(0, k);
    }
    for j in 1..at.len() {
        let dist = (pts[at[j]] - pts[at[j - 1]]).length().max(1.0);
        for k in 0..FIT_SHIFTS {
            let here = place(j, k);
            if here.is_infinite() {
                continue;
            }
            for p in k.saturating_sub(1)..=(k + 1).min(FIT_SHIFTS - 1) {
                let prev = best[(j - 1) * FIT_SHIFTS + p];
                if prev.is_infinite() {
                    continue;
                }
                let rise = (cols[j * FIT_SHIFTS + k].ground - cols[(j - 1) * FIT_SHIFTS + p].ground).abs();
                let grade = rise / dist;
                let cost = prev + here + dist / 16.0 * (20.0 * grade + 400.0 * grade * grade) + 1.0 * k.abs_diff(p) as f32;
                if cost < best[j * FIT_SHIFTS + k] {
                    best[j * FIT_SHIFTS + k] = cost;
                    from[j * FIT_SHIFTS + k] = p as u8;
                }
            }
        }
    }
    let mut k = mid;
    let mut chosen = vec![mid; at.len()];
    for j in (1..at.len()).rev() {
        chosen[j] = k;
        k = from[j * FIT_SHIFTS + k] as usize;
    }
    chosen[0] = k;
    if chosen.iter().all(|&k| k == mid) {
        return pts.to_vec();
    }
    // Eased, so the trail swings round in curves rather than stepping sideways.
    let mut offsets: Vec<f32> = chosen.iter().map(|&k| shift(k)).collect();
    for _ in 0..2 {
        let copy = offsets.clone();
        for j in 1..offsets.len() - 1 {
            offsets[j] = (copy[j - 1] + copy[j] * 2.0 + copy[j + 1]) * 0.25;
        }
    }
    let control: Vec<Vec2> = at.iter().zip(&offsets).zip(&normals).map(|((&i, &off), &nrm)| pts[i] + nrm * off).collect();
    let mut out = route::resample(&control);
    // Resampling can fall a fraction of a step short of the end.
    if out.last().is_some_and(|p| p.distance(pts[n - 1]) > 0.5) {
        out.push(pts[n - 1]);
    }
    out
}

/// Traffic on the trails: trips between every pair of sites on the same land, along the
/// shortest way through the network. A trip counts the product of the two sites'
/// importance, less for distant pairs. Returns a value per map cell.
fn traffic(c: &Continent, used: &[u32], sites: &[(usize, f32)]) -> Vec<f32> {
    let n = c.size as i32;
    let cs = c.cell_size();
    let mut traffic = vec![0.0f32; used.len()];
    let mut dist = vec![f32::INFINITY; used.len()];
    let mut prev = vec![u32::MAX; used.len()];
    let mut touched: Vec<usize> = Vec::new();
    for (s, &(start, weight_s)) in sites.iter().enumerate() {
        if used[start] == 0 {
            continue;
        }
        for &i in &touched {
            dist[i] = f32::INFINITY;
            prev[i] = u32::MAX;
        }
        touched.clear();
        dist[start] = 0.0;
        touched.push(start);
        let mut open = BinaryHeap::new();
        open.push(Reverse((0u32, start as u32)));
        while let Some(Reverse((_, i))) = open.pop() {
            let i = i as usize;
            let (x, z) = (i as i32 % n, i as i32 / n);
            for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
                let (nx, nz) = (x + dx, z + dz);
                if nx < 0 || nz < 0 || nx >= n || nz >= n {
                    continue;
                }
                let j = (nz * n + nx) as usize;
                if used[j] == 0 {
                    continue;
                }
                let d = dist[i] + if dx != 0 && dz != 0 { std::f32::consts::SQRT_2 } else { 1.0 };
                if d < dist[j] {
                    if dist[j].is_infinite() {
                        touched.push(j);
                    }
                    dist[j] = d;
                    prev[j] = i as u32;
                    open.push(Reverse(((d * 16.0) as u32, j as u32)));
                }
            }
        }
        // Each pair once.
        for &(end, weight_e) in &sites[s + 1..] {
            if dist[end].is_infinite() {
                continue;
            }
            let trip = weight_s * weight_e / (1.0 + dist[end] * cs / TRIP_FALLOFF);
            let mut at = end;
            while at != start {
                traffic[at] += trip;
                at = prev[at] as usize;
            }
            traffic[start] += trip;
        }
    }
    traffic
}

/// Index ranges of water along a trail, with brief dry interruptions merged.
fn wet_runs(cols: &[Column]) -> Vec<(usize, usize)> {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for (i, col) in cols.iter().enumerate() {
        if !is_wet(col) {
            continue;
        }
        match runs.last_mut() {
            Some(last) if i - last.1 <= 4 => last.1 = i,
            _ => runs.push((i, i)),
        }
    }
    runs
}

/// Rewrites the approach to each crossing as a straight line, so a bridge goes directly
/// over the water instead of curving with the path.
fn straighten(pts: &[Vec2], runs: &[(usize, usize)]) -> Vec<Vec2> {
    let mut out = Vec::with_capacity(pts.len());
    let mut at = 0;
    for &(s, e) in runs {
        let a = s.saturating_sub(5);
        let b = (e + 5).min(pts.len() - 1);
        if a < at || b <= a {
            continue;
        }
        out.extend_from_slice(&pts[at..a]);
        let (pa, pb) = (pts[a], pts[b]);
        let len = ((pb - pa).length() / OLD_VOXEL).ceil().max(1.0) as usize;
        out.extend((0..len).map(|k| pa + (pb - pa) * (k as f32 / len as f32)));
        at = b;
    }
    out.extend_from_slice(&pts[at..]);
    out
}

/// A world voxel on the deck grid, offset sideways from the centreline.
fn deck_voxel(p: Vec2, side: Vec2, offset: f32, y: i32) -> IVec3 {
    let q = p + side * offset;
    IVec3::new(q.x.floor() as i32, y, q.y.floor() as i32)
}

/// Stones across a narrow stream, proud of the water, with a flush ford between them so
/// walking across never leaves you stuck.
fn stepping_stones(pal: &Palette, rng: &mut Rng, pts: &[Vec2], cols: &[Column], run: (usize, usize)) -> Vec<(IVec3, Voxel)> {
    let mut s = Shape::raw();
    let dir = (pts[run.1] - pts[run.0]).normalize_or_zero();
    let side = vec2(-dir.y, dir.x);
    for i in run.0..=run.1 {
        let Some(k) = cols[i].water_surface() else { continue };
        let proud = (i - run.0) % 3 != 2;
        for along in 0..BLOCK {
            let p = pts[i] + dir * along as f32;
            let mut offset = -0.5 * OLD_VOXEL;
            while offset <= 0.5 * OLD_VOXEL {
                for y in cols[i].top() + 1..=k + proud as i32 * BLOCK - 1 {
                    s.set(deck_voxel(p, side, offset, y), pal.block(rng));
                }
                offset += 1.0;
            }
        }
    }
    s.into_voxels()
}

/// Where one column of a bridge lies: `t` voxels along from where the water begins, `u`
/// voxels across from the centreline, the deck height `y` over it, and the trail sample
/// `i` it belongs to.
#[derive(Clone, Copy)]
struct Under {
    t: f32,
    u: f32,
    y: i32,
    i: usize,
}

/// One arch of a stone bridge: from `a` to `b` voxels along, its underside a circle of
/// `radius` centred at height `centre`.
struct Opening {
    a: f32,
    b: f32,
    centre: f32,
    radius: f32,
}

impl Opening {
    /// Height of the underside of the arch `t` voxels along the bridge.
    fn intrados(&self, t: f32) -> f32 {
        let x = t - (self.a + self.b) * 0.5;
        self.centre + (self.radius * self.radius - x * x).max(0.0).sqrt()
    }
}

/// Thickness of a stone bridge's piers along the bridge, in voxels.
const PIER: f32 = 3.0;

/// What one bridge is built from and how it is laid out, for building it column by column.
struct Deck<'a> {
    pal: &'a Palette,
    cols: &'a [Column],
    /// Length of the water crossed, in voxels along.
    len: f32,
    /// Half-width of the walking surface, in voxels.
    half: f32,
    /// Where the bridge begins and ends, in voxels along.
    ends: (f32, f32),
    kmax: i32,
    piers: Vec<f32>,
    openings: Vec<Opening>,
}

impl Deck<'_> {
    fn over(&self, t: f32) -> bool {
        t >= 0.0 && t < self.len
    }

    fn at_end(&self, t: f32) -> bool {
        t < self.ends.0 + 1.0 || t >= self.ends.1 - 1.0
    }

    /// How far into the bank `t` is, or 0 over the water.
    fn ashore(&self, t: f32) -> f32 {
        if t < 0.0 { -t } else { (t - self.len).max(0.0) }
    }

    /// A deck board, now and then one replaced in a different wood.
    fn board(&self, n: i32) -> Voxel {
        if unit(hash2(0xb0a2, n, 0)) < 0.2 {
            if self.pal.plank == mat::PLANK_DARK { mat::PLANK } else { mat::PLANK_DARK }
        } else {
            self.pal.plank
        }
    }
}

/// A timber footbridge: boards across stringers, a kerb and handrail on posts each side,
/// trestle bents in the water on longer spans, stone abutments and ramps on the banks.
fn plank_column(s: &mut Shape, d: &Deck, rng: &mut Rng, x: i32, z: i32, c: &Under) {
    let mut put = |yy: i32, v: Voxel| s.set(IVec3::new(x, yy, z), v);
    let col = &d.cols[c.i];
    let (au, y, t) = (c.u.abs(), c.y, c.t);
    let (wet, bed) = (is_wet(col), col.top());
    if au > d.half + 1.0 {
        return;
    }
    // On the banks, stonework carries the deck: the abutments and ramps.
    if !wet {
        for yy in bed - 2..y {
            put(yy, d.pal.block(rng));
        }
    }
    if au > d.half {
        // The kerb beam the posts stand on, posts every 2 m and taller at the ends, and
        // a handrail at 1.2 m.
        put(y, d.pal.wood);
        let end = d.at_end(t);
        if t.rem_euclid(5.0) < 1.0 || end {
            for yy in y + 1..=y + 3 + end as i32 {
                put(yy, d.pal.wood);
            }
        }
        put(y + 3, d.pal.wood);
    } else {
        put(y, d.board(t.floor() as i32));
        if wet && (au > d.half - 1.0 || au < 0.5) {
            put(y - 1, d.pal.wood);
        }
    }
    // Trestle bents in the water every 4 m: a cap beam under the deck, legs to the bed
    // under the edge stringers, and a tie halfway down the taller ones.
    if wet && d.over(t) && d.len > 12.0 && t > 2.0 && t < d.len - 2.0 && (t + 5.0).rem_euclid(10.0) < 1.0 {
        put(y - 1, d.pal.wood);
        if au > d.half - 1.0 && au <= d.half {
            for yy in bed - 1..y - 1 {
                put(yy, d.pal.wood);
            }
        }
        if y - bed > 5 && au <= d.half {
            put((bed + y) / 2, d.pal.wood);
        }
    }
}

/// A felled trunk with its top adzed flat, resting a step onto each bank on stone, a
/// pole handrail on one side, branch stubs on the other, and stone steps down.
fn log_column(s: &mut Shape, d: &Deck, rng: &mut Rng, x: i32, z: i32, c: &Under) {
    let mut put = |yy: i32, v: Voxel| s.set(IVec3::new(x, yy, z), v);
    let col = &d.cols[c.i];
    let (au, y, t) = (c.u.abs(), c.y, c.t);
    let bed = col.top();
    let on_log = t >= -OLD_VOXEL && t < d.len + OLD_VOXEL;
    if !on_log {
        if au <= d.half + 0.5 {
            for yy in bed - 2..=y {
                put(yy, d.pal.block(rng));
            }
        }
        return;
    }
    if au <= d.half {
        let inner = au <= 0.6;
        put(y, if inner { mat::WOOD_DEAD } else { d.pal.wood });
        put(y - 1, d.pal.wood);
        if inner {
            put(y - 2, d.pal.wood);
        }
        // The ends rest on stone; a long log has a stone prop under its middle.
        if !d.over(t) || (d.len > 20.0 && (t - d.len * 0.5).abs() < 1.5) {
            for yy in bed - 2..y - 2 {
                put(yy, d.pal.block(rng));
            }
        }
    } else if au <= d.half + 1.0 {
        if c.u > 0.0 {
            if t.rem_euclid(6.0) < 1.0 || d.at_end(t) {
                for yy in y - 1..=y + 3 {
                    put(yy, d.pal.wood);
                }
            }
            put(y + 3, d.pal.wood);
        } else if d.over(t) && (t.floor() as i32).rem_euclid(9) == 4 {
            put(y - 1, d.pal.wood);
        }
    }
}

/// A stone bridge: a run of arches with voussoir rings, spandrels, piers with cutwaters,
/// paved deck, parapets with coping, and wing walls where it meets the banks.
fn arch_column(s: &mut Shape, d: &Deck, rng: &mut Rng, x: i32, z: i32, c: &Under) {
    let mut put = |yy: i32, v: Voxel| s.set(IVec3::new(x, yy, z), v);
    let col = &d.cols[c.i];
    let (au, y, t) = (c.u.abs(), c.y, c.t);
    let bed = col.top();
    let over = d.over(t);
    if au > d.half + 1.0 {
        if !over {
            if d.ashore(t) < 2.0 * OLD_VOXEL && au <= d.half + 2.0 {
                for yy in bed - 2..y {
                    put(yy, d.pal.block(rng));
                }
            }
            return;
        }
        // Cutwaters: each pier comes to a point upstream and down.
        for &p in &d.piers {
            let reach = PIER * 0.5 * (1.0 - (au - d.half - 1.0) / 2.5);
            if (t - p).abs() <= reach {
                for yy in bed - 2..=d.kmax {
                    put(yy, d.pal.block(rng));
                }
                put(d.kmax + 1, d.pal.stone_dark);
            }
        }
        return;
    }
    // The body under the deck: an arch ring two voxels deep over each opening, solid on
    // the piers and the banks.
    let (from, ring) = match d.openings.iter().find(|o| over && t >= o.a && t < o.b) {
        Some(o) => {
            let lo = ((o.intrados(t) - 0.5).ceil() as i32).max(bed - 2);
            (lo, lo + 2)
        }
        None => (bed - 2, bed - 2),
    };
    let voussoir = if (t.floor() as i32).rem_euclid(2) == 0 { d.pal.stone_dark } else { d.pal.stone };
    for yy in from..y {
        put(yy, if yy < ring { voussoir } else { d.pal.block(rng) });
    }
    if au > d.half {
        // A string course, then the parapet with its coping, a post higher at the ends.
        put(y, d.pal.stone_dark);
        put(y + 1, d.pal.block(rng));
        put(y + 2, d.pal.stone_dark);
        if d.at_end(t) {
            put(y + 3, d.pal.stone_dark);
        }
    } else {
        put(y, d.pal.block(rng));
    }
}

/// Bridges a run of water. Returns the voxels and the sample range the structure covers,
/// ramps included: the trail is carried by the bridge across that range.
#[allow(clippy::too_many_arguments)]
fn bridge(
    kind: CrossingKind,
    pal: &Palette,
    rng: &mut Rng,
    pts: &[Vec2],
    cols: &[Column],
    run: (usize, usize),
) -> (Vec<(IVec3, Voxel)>, (usize, usize)) {
    let (s0, e0) = run;
    let kmax = (s0..=e0).filter_map(|i| cols[i].water_surface()).max().unwrap_or(0);
    // A stone bridge stands higher, so its arches have room to open.
    let deck = kmax + BLOCK * if kind == CrossingKind::Arch { 3 } else { 1 };
    let span = (e0 - s0 + 1) as f32;
    let dir = (pts[e0.min(pts.len() - 1)] - pts[s0]).normalize_or_zero();
    let side = vec2(-dir.y, dir.x);
    // A log lies a step onto each bank before the steps down begin.
    let flat = (kind == CrossingKind::Log) as i32;

    // Ramps run down each side at one in two until they meet the ground.
    let ramp = |from: usize, step: isize| -> (usize, Vec<i32>) {
        let (mut i, mut ys) = (from, Vec::new());
        for k in 1..=24 {
            let next = from as isize + step * k;
            if next < 0 || next as usize >= pts.len() {
                break;
            }
            let y = deck - ((k as i32 - flat) * BLOCK + 1) / 2;
            i = next as usize;
            let ground = cols[i].top();
            if is_wet(&cols[i]) || y > ground {
                ys.push(y.max(ground));
            } else {
                break;
            }
        }
        (i, ys)
    };
    let (foot_a, up) = ramp(s0, -1);
    let (foot_b, down) = ramp(e0, 1);

    // Height of the deck at each sample, with a gentle hump on the bigger bridges.
    let rise = if kind == CrossingKind::Arch { (span / 10.0).clamp(1.0, 4.0) * OLD_VOXEL } else { 0.0 };
    let mut samples: Vec<(usize, i32)> = Vec::new();
    for (k, &y) in up.iter().enumerate().rev() {
        samples.push((s0 - (k + 1), y));
    }
    for i in s0..=e0 {
        let t = (i - s0) as f32 / span.max(1.0);
        samples.push((i, deck + (rise * (t * std::f32::consts::PI).sin()).round() as i32));
    }
    for (k, &y) in down.iter().enumerate() {
        samples.push((e0 + k + 1, y));
    }

    let first = samples[0].0;
    let last = samples[samples.len() - 1].0;
    let y_at = |i: usize| samples[(i - first).min(samples.len() - 1)].1;
    let len = span * OLD_VOXEL;
    // Half-width of the walking surface, and how far out anything reaches.
    let (half, reach) = match kind {
        CrossingKind::Arch => (3.1, 6.6),
        CrossingKind::Log => (1.1, 2.1),
        _ => (2.1, 3.1),
    };

    // Every column the bridge covers, with where it lies along and across. Half-steps
    // along and across so a diagonal bridge has no holes; a column takes the sample
    // nearest the centreline.
    let mut under: BTreeMap<(i32, i32), Under> = BTreeMap::new();
    for &(i, y) in &samples {
        let base = (i as f32 - s0 as f32) * OLD_VOXEL;
        let mut along = 0.0f32;
        while along < OLD_VOXEL {
            let p = pts[i] + dir * along;
            let mut u = -reach;
            while u <= reach {
                let q = deck_voxel(p, side, u, y);
                let here = Under { t: base + along, u, y, i };
                under.entry((q.x, q.z)).and_modify(|o| if u.abs() < o.u.abs() { *o = here }).or_insert(here);
                u += 0.5;
            }
            along += 0.5;
        }
    }

    // A stone bridge's arches: about 5 m each, between piers, each rising from below
    // the water to two voxels under the deck at its crown.
    let (mut piers, mut openings) = (Vec::new(), Vec::new());
    if kind == CrossingKind::Arch {
        let n = ((len / 14.0).round() as usize).max(1);
        piers = (1..n).map(|j| len * j as f32 / n as f32).collect::<Vec<f32>>();
        for j in 0..n {
            let a = if j == 0 { 0.0 } else { piers[j - 1] + PIER * 0.5 };
            let b = if j + 1 == n { len } else { piers[j] - PIER * 0.5 };
            let c = (b - a) * 0.5;
            let within = |lo: f32, hi: f32| (s0..=e0).filter(move |&i| {
                let t = (i - s0) as f32 * OLD_VOXEL;
                t + OLD_VOXEL > lo && t < hi
            });
            let crown = within(a + c * 0.5, b - c * 0.5).map(&y_at).min().unwrap_or(deck) as f32 - 2.0;
            let bed = within(a, b).map(|i| cols[i].top()).min().unwrap_or(kmax) as f32;
            let r = (c * 0.8).min(crown - bed).max(2.0);
            let radius = (c * c + r * r) / (2.0 * r);
            openings.push(Opening { a, b, centre: crown - radius, radius });
        }
    }
    let d = Deck {
        pal,
        cols,
        len,
        half,
        ends: ((first as f32 - s0 as f32) * OLD_VOXEL, (last as f32 + 1.0 - s0 as f32) * OLD_VOXEL),
        kmax,
        piers,
        openings,
    };

    let mut s = Shape::raw();
    for (&(x, z), c) in &under {
        match kind {
            CrossingKind::Arch => arch_column(&mut s, &d, rng, x, z, c),
            CrossingKind::Log => log_column(&mut s, &d, rng, x, z, c),
            _ => plank_column(&mut s, &d, rng, x, z, c),
        }
    }
    (s.into_voxels(), (foot_a, foot_b))
}

/// Finds where a landmark can actually stand near a map cell.
fn settle(c: &Continent, seed: u32, kind: Landmark, cell: usize) -> Option<(i32, i32, f32)> {
    let cs = c.cell_size();
    let n = c.size;
    let (cx, cz) = (((cell % n) as f32 + 0.5) * cs, ((cell / n) as f32 + 0.5) * cs);
    let radius = kind.radius();
    let probe = |x: i32, z: i32| -> Option<f32> {
        let r = radius as i32 + 1;
        let sampler = Sampler::new(c, seed, x - r, z - r, 2 * r + 1);
        let (mut lo, mut hi, mut sum) = (f32::MAX, f32::MIN, 0.0);
        let rim = [(0, 0), (r, 0), (-r, 0), (0, r), (0, -r), (r * 2 / 3, r * 2 / 3), (-r * 2 / 3, r * 2 / 3), (r * 2 / 3, -r * 2 / 3), (-r * 2 / 3, -r * 2 / 3)];
        for (dx, dz) in rim {
            let col = sampler.column(x + dx, z + dz);
            if is_wet(&col) || col.river < m(4.8) || !matches!(col.cover, Cover::Land | Cover::Beach) {
                return None;
            }
            lo = lo.min(col.ground);
            hi = hi.max(col.ground);
            sum += col.ground;
        }
        (hi - lo <= m(5.6)).then_some(sum / rim.len() as f32)
    };
    for ring in 0..12 {
        let reach = ring as f32 * cs * 0.25;
        let steps = if ring == 0 { 1 } else { 8 + ring * 2 };
        for k in 0..steps {
            let a = k as f32 / steps as f32 * std::f32::consts::TAU;
            let (x, z) = ((cx + a.cos() * reach) as i32, (cz + a.sin() * reach) as i32);
            if let Some(y) = probe(x, z) {
                return Some((x, z, y));
            }
        }
    }
    None
}

/// A landing needs a shoreline and open water straight out from it.
struct Shore {
    x: i32,
    z: i32,
    level: i32,
    turns: u32,
    length: i32,
}

fn find_shore(c: &Continent, seed: u32, cell: usize) -> Option<Shore> {
    let cs = c.cell_size();
    let n = c.size;
    let (cx, cz) = (((cell % n) as f32 + 0.5) * cs, ((cell / n) as f32 + 0.5) * cs);
    let reach = (cs * 4.0) as i32;
    let mut best: Option<Shore> = None;
    for offset in [0, 12, -12, 24, -24].map(|o| o * BLOCK) {
        for (turns, (dx, dz)) in [(1i32, 0i32), (0, 1), (-1, 0), (0, -1)].into_iter().enumerate() {
            let (sx, sz) = (dz * offset + cx as i32, dx * offset + cz as i32);
            let (lo_x, lo_z) = (sx.min(sx + dx * reach) - 1, sz.min(sz + dz * reach) - 1);
            let sampler = Sampler::new(c, seed, lo_x, lo_z, reach + 3);
            // March out to the water's edge, then see how far the water goes.
            let mut last_dry = None;
            let mut run = 0;
            let mut level = 0;
            for d in 0..reach {
                let col = sampler.column(sx + dx * d, sz + dz * d);
                match col.water_surface() {
                    Some(k) if last_dry.is_some() => {
                        if col.river < 0.0 {
                            break;
                        }
                        level = k;
                        run += 1;
                        if run >= 22 * BLOCK {
                            break;
                        }
                    }
                    Some(_) => {}
                    None if run > 0 => break,
                    None => last_dry = Some(d),
                }
            }
            if let Some(d) = last_dry
                && run >= 14 * BLOCK
                && best.as_ref().is_none_or(|b| run > b.length + 2 * BLOCK)
            {
                best = Some(Shore {
                    x: sx + dx * d,
                    z: sz + dz * d,
                    level,
                    turns: turns as u32,
                    length: run - 2 * BLOCK,
                });
            }
        }
    }
    best
}

fn absolute(voxels: Vec<(IVec3, Voxel)>, origin: IVec3, turns: u32) -> Vec<(IVec3, Voxel)> {
    voxels.into_iter().map(|(p, v)| (origin + rotate(p, turns), v)).collect()
}

/// Settled sites with their map cell.
fn place_sites(c: &Continent, seed: u32, comp: &[u32], net: &mut Network) -> Vec<usize> {
    let mut lists = sites::candidates(c, comp);
    let mut placed: Vec<(f32, f32)> = Vec::new();
    let mut cells = Vec::new();
    let mut remaining: Vec<usize> = lists.iter().map(|l| l.1).collect();
    let mut cursor = vec![0usize; lists.len()];
    let cs = c.cell_size();
    // Take turns, so scarce kinds are not crowded out by whichever scores highest.
    let mut progress = true;
    while progress {
        progress = false;
        for (k, list) in lists.iter_mut().enumerate() {
            if remaining[k] == 0 {
                continue;
            }
            while cursor[k] < list.2.len() {
                let cell = list.2[cursor[k]];
                cursor[k] += 1;
                let (wx, wz) = (((cell % c.size) as f32 + 0.5) * cs, ((cell / c.size) as f32 + 0.5) * cs);
                if !sites::spaced(&placed, wx, wz) {
                    continue;
                }
                let site_seed = hash2(c.desc.seed ^ 0x517e, cell as i32, k as i32);
                let site = if list.0 == Landmark::Landing {
                    place_landing(c, seed, cell, site_seed, net)
                } else {
                    place_land(c, seed, list.0, cell, site_seed, net)
                };
                if let Some(site) = site {
                    placed.push((site.x as f32, site.z as f32));
                    net.sites.push(site);
                    cells.push(cell);
                    remaining[k] -= 1;
                    progress = true;
                    break;
                }
            }
        }
    }
    cells
}

fn place_land(c: &Continent, seed: u32, kind: Landmark, cell: usize, site_seed: u32, net: &mut Network) -> Option<Site> {
    let (x, z, ground) = settle(c, seed, kind, cell)?;
    let biome = Sampler::new(c, seed, x, z, 1).column(x, z).biome;
    let palette = Palette::of(biome);
    let y = ground.round();
    net.pads.push(Pad {
        x: x as f32 + 0.5,
        z: z as f32 + 0.5,
        radius: kind.radius(),
        y,
    });
    let origin = IVec3::new(x, y as i32 + 1, z);
    let turns = site_seed & 3;
    let (voxels, fine) = landmarks::build_parts(kind, &palette, site_seed, 0);
    net.add_stamp_with(absolute(voxels, origin, turns), parts::place_fine(&fine, origin, turns));
    Some(Site {
        kind,
        x,
        z,
        y: origin.y,
        mooring: None,
    })
}

fn place_landing(c: &Continent, seed: u32, cell: usize, site_seed: u32, net: &mut Network) -> Option<Site> {
    let shore = find_shore(c, seed, cell)?;
    let biome = Sampler::new(c, seed, shore.x, shore.z, 1).column(shore.x, shore.z).biome;
    let palette = Palette::of(biome);
    let y = shore.level as f32 + 0.5;
    net.pads.push(Pad {
        x: shore.x as f32 + 0.5,
        z: shore.z as f32 + 0.5,
        radius: Landmark::Landing.radius(),
        y,
    });
    let origin = IVec3::new(shore.x, shore.level + 1, shore.z);
    let (pier, fine) = landmarks::build_parts(Landmark::Landing, &palette, site_seed, shore.length / BLOCK);
    net.add_stamp_with(absolute(pier, origin, shore.turns), parts::place_fine(&fine, origin, shore.turns));
    let along = rotate(IVec3::new(shore.length - 3 * BLOCK, 0, 4 * BLOCK), shore.turns);
    let heading = shore.turns as f32 * std::f32::consts::FRAC_PI_2;
    Some(Site {
        kind: Landmark::Landing,
        x: shore.x,
        z: shore.z,
        y: origin.y,
        mooring: Some(Mooring {
            pos: Vec3::new(
                (shore.x + along.x) as f32 + 0.5,
                shore.level as f32,
                (shore.z + along.z) as f32 + 0.5,
            ),
            heading,
        }),
    })
}

/// Pairs of sites to connect: a relative neighbourhood graph within each landmass, which
/// is sparse but always connected as long as no link is too long.
fn links(sites_at: &[(f32, f32, u32)]) -> Vec<(usize, usize, f32)> {
    let dist = |a: usize, b: usize| (sites_at[a].0 - sites_at[b].0).hypot(sites_at[a].1 - sites_at[b].1);
    let n = sites_at.len();
    let mut out = Vec::new();
    for a in 0..n {
        for b in a + 1..n {
            if sites_at[a].2 != sites_at[b].2 {
                continue;
            }
            let d = dist(a, b);
            if d > MAX_LINK {
                continue;
            }
            let shadowed = (0..n).any(|k| k != a && k != b && dist(a, k).max(dist(b, k)) < d);
            if !shadowed {
                out.push((a, b, d));
            }
        }
    }
    out.sort_by(|p, q| p.2.total_cmp(&q.2));
    out
}

/// Heights along a trail: terrain smoothed by a moving average, then clamped so the
/// slope never exceeds what is comfortable to walk.
fn profile(cols: &[Column], pins: &[(usize, f32)]) -> Vec<f32> {
    const WINDOW: usize = 12;
    let max_step = MAX_SLOPE * OLD_VOXEL;
    let n = cols.len();
    let mut smoothed = Vec::with_capacity(n);
    for i in 0..n {
        let (lo, hi) = (i.saturating_sub(WINDOW), (i + WINDOW + 1).min(n));
        let (mut sum, mut count) = (0.0, 0);
        for col in &cols[lo..hi] {
            if !is_wet(col) {
                sum += col.ground;
                count += 1;
            }
        }
        smoothed.push(if count > 0 { sum / count as f32 } else { cols[i].ground });
    }
    for i in 1..n {
        smoothed[i] = smoothed[i].clamp(smoothed[i - 1] - max_step, smoothed[i - 1] + max_step);
    }
    for i in (0..n.saturating_sub(1)).rev() {
        smoothed[i] = smoothed[i].clamp(smoothed[i + 1] - max_step, smoothed[i + 1] + max_step);
    }
    // Pinned points (landmarks, bridge feet) draw the profile to them within reach.
    for &(at, y) in pins {
        let reach = 64usize;
        for i in at.saturating_sub(reach)..(at + reach + 1).min(n) {
            let room = i.abs_diff(at) as f32 * max_step;
            smoothed[i] = smoothed[i].clamp(y - room, y + room);
        }
    }
    smoothed
}

/// Lays one stretch of trail over the land and builds its crossings.
struct Layer<'a> {
    c: &'a Continent,
    seed: u32,
    /// Landmark centres, for lantern spacing.
    site_points: Vec<Vec2>,
}

impl Layer<'_> {
    fn palette_at(&self, col: &Column) -> Palette {
        Palette::of(col.biome)
    }

    /// `widths` is the half-width the traffic calls for at each of `raw`.
    fn lay(&self, raw: Vec<Vec2>, widths: &[f32], start_y: Option<f32>, end_y: Option<f32>, salt: u32, net: &mut Network) {
        if raw.len() < 8 {
            return;
        }
        // Over water the trail must run straight, which needs the water found first.
        let cols = sample(self.c, self.seed, &raw);
        let pts = straighten(&raw, &wet_runs(&cols));
        // Straightening barely changes the number of points, so they map across in order.
        let widths: Vec<f32> = (0..pts.len()).map(|i| widths[i * widths.len() / pts.len()]).collect();
        let mut cols = sample(self.c, self.seed, &pts);
        let widths = narrow_high(&widths, &cols);
        let runs = wet_runs(&cols);

        // Water too wide to bridge ends the trail on each side of it.
        if let Some(&(s, e)) = runs.iter().find(|&&(s, e)| e - s + 1 > MAX_SPAN) {
            if s > 8 {
                self.lay(pts[..s - 6].to_vec(), &widths[..s - 6], start_y, None, salt.wrapping_add(11), net);
            }
            if e + 8 < pts.len() {
                self.lay(pts[e + 6..].to_vec(), &widths[e + 6..], None, end_y, salt.wrapping_add(23), net);
            }
            return;
        }

        let mut rng = Rng::new(hash2(self.c.desc.seed ^ 0xb71d, salt as i32, pts.len() as i32));
        // (start, end) of sample ranges carried by a bridge.
        let mut carried: Vec<(usize, usize)> = Vec::new();
        let mut pins: Vec<(usize, f32)> = Vec::new();
        for &(s, e) in &runs {
            let span = e - s + 1;
            let mid = (s + e) / 2;
            let palette = self.palette_at(&cols[mid]);
            let depth = (s..=e)
                .filter_map(|i| cols[i].water_surface().map(|k| k - cols[i].top()))
                .max()
                .unwrap_or(0);
            let kind = if span <= 5 && depth <= 3 * BLOCK {
                CrossingKind::Stones
            } else if span <= 16 {
                if rng.unit() < 0.5 { CrossingKind::Log } else { CrossingKind::Plank }
            } else {
                CrossingKind::Arch
            };
            net.crossings.push(Crossing {
                kind,
                x: pts[mid].x as i32,
                z: pts[mid].y as i32,
                span: span as i32,
            });
            if kind == CrossingKind::Stones {
                net.add_stamp(stepping_stones(&palette, &mut rng, &pts, &cols, (s, e)));
            } else {
                let (voxels, range) = bridge(kind, &palette, &mut rng, &pts, &cols, (s, e));
                net.add_stamp(voxels);
                carried.push(range);
                pins.push((range.0.saturating_sub(1), cols[range.0].ground));
                pins.push(((range.1 + 1).min(pts.len() - 1), cols[range.1].ground));
            }
        }
        if let Some(y) = start_y {
            pins.push((0, y));
        }
        if let Some(y) = end_y {
            pins.push((pts.len() - 1, y));
        }

        let heights = profile(&cols, &pins);
        let is_carried = |i: usize| carried.iter().any(|&(a, b)| (a..=b).contains(&i));
        let worn: Vec<f32> = (0..pts.len()).map(|i| self.worn(&pts, &widths, i, salt)).collect();
        let points: Vec<TrailPoint> = pts
            .iter()
            .enumerate()
            .map(|(i, p)| TrailPoint {
                x: p.x,
                z: p.y,
                y: heights[i],
                half_width: if is_carried(i) { 0.0 } else { worn[i] },
            })
            .collect();
        net.add_points(&points);
        net.lines.push(pts.iter().step_by(8).copied().collect());
        net.line_widths.push(widths.iter().step_by(8).copied().collect());
        cols.truncate(pts.len());
        self.furnish(&pts, &widths, &heights, &cols, &carried, salt, net);
    }

    /// How wide the trail is actually worn at point `i`: the width its traffic calls
    /// for, except that a faint trace comes and goes, fading out for stretches and
    /// picking up again, the fainter the longer its gaps.
    fn worn(&self, pts: &[Vec2], widths: &[f32], i: usize, salt: u32) -> f32 {
        let w = widths[i];
        if w >= TRACE {
            return w;
        }
        // Noise along the trail, so gaps are whole stretches, not single holes.
        let along = perlin2(self.c.desc.seed ^ 0x7ace ^ salt, i as f64 / 23.0, salt as f64 * 0.61) * 0.5 + 0.5;
        let faint = 1.0 - (w - widths_min()) / (TRACE - widths_min());
        // Never at the very ends, where it meets a site or another trail.
        let end = i.min(pts.len() - 1 - i) < 12;
        if !end && along < 0.15 + 0.35 * faint { 0.0 } else { w }
    }

    /// Lanterns along the way, closer together near landmarks and at bridge heads, and
    /// only on trails worn enough for anyone to light.
    #[allow(clippy::too_many_arguments)]
    fn furnish(
        &self,
        pts: &[Vec2],
        widths: &[f32],
        heights: &[f32],
        cols: &[Column],
        carried: &[(usize, usize)],
        salt: u32,
        net: &mut Network,
    ) {
        let mut since = 30usize;
        let mut side = if salt & 1 == 0 { 1.0 } else { -1.0 };
        for i in 8..pts.len().saturating_sub(8) {
            since += 1;
            let near_site = self.site_points.iter().any(|s| s.distance_squared(pts[i]) < NEAR_SITE * NEAR_SITE);
            let at_bridge = carried.iter().any(|&(a, b)| i + 3 == a || i == b + 3);
            let due = since >= if near_site { LANTERN_NEAR } else { LANTERN_FAR };
            if !(at_bridge || due) || widths[i] < LIT || carried.iter().any(|&(a, b)| (a..=b).contains(&i)) {
                continue;
            }
            let tangent = (pts[i + 4] - pts[i - 4]).normalize_or_zero();
            let normal = vec2(-tangent.y, tangent.x);
            let q = pts[i] + normal * side * (widths[i] + m(1.2));
            let col = cols[i];
            if is_wet(&col) || col.river < m(2.4) {
                continue;
            }
            let (x, z) = (q.x.floor() as i32, q.y.floor() as i32);
            let y = heights[i].floor() as i32 + 1;
            let height = 3 + (unit(hash2(salt, x, z)) * 2.0) as i32;
            let at = IVec3::new(x, y, z);
            let (voxels, fine) = landmarks::trail_lantern_parts(&self.palette_at(&col), height, hash2(salt, x, z));
            net.add_stamp_with(absolute(voxels, at, 0), parts::place_fine(&fine, at, 0));
            since = 0;
            side = -side;
        }
    }
}

/// Runs of cells not yet on a trail, each with what it connects to at either end.
fn fresh_runs(path: &[u32], used: &[u32]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < path.len() {
        if used[path[i] as usize] != 0 {
            i += 1;
            continue;
        }
        let start = i;
        while i < path.len() && used[path[i] as usize] == 0 {
            i += 1;
        }
        out.push((start, i - 1));
    }
    out
}

/// Builds the whole network for a continent. `world_seed` is needed to sample the same
/// surface the game will generate.
pub fn build(c: &Continent, world_seed: u32) -> Network {
    let mut net = Network::default();
    if c.ocean.iter().all(|&o| o) {
        return net;
    }
    let comp = sites::land_components(c);
    let cells = place_sites(c, world_seed, &comp, &mut net);
    let at: Vec<(f32, f32, u32)> = net
        .sites
        .iter()
        .zip(&cells)
        .map(|(s, &cell)| (s.x as f32 + 0.5, s.z as f32 + 0.5, comp[cell]))
        .collect();

    let layer = Layer {
        c,
        seed: world_seed,
        site_points: at.iter().map(|&(x, z, _)| vec2(x, z)).collect(),
    };
    let cs = c.cell_size();
    let cell_of = |x: f32, z: f32| (z / cs) as usize * c.size + (x / cs) as usize;
    let mut router = Router::new(c);
    let mut lines: HashMap<u32, Vec<Vec2>> = HashMap::new();
    let mut next_id = 1u32;
    /// A stretch of trail routed but not yet laid: (points, its map cells, start
    /// height, end height, id).
    type Pending = (Vec<Vec2>, Vec<u32>, Option<f32>, Option<f32>, u32);
    let mut pending: Vec<Pending> = Vec::new();
    let importance: Vec<f32> = net.sites.iter().map(|s| sites::importance(s.kind)).collect();

    // The links between the most important places first, so that lesser trails come
    // to join them rather than the other way about.
    let mut order = links(&at);
    order.sort_by(|p, q| {
        let key = |&(a, b, d): &(usize, usize, f32)| d / (importance[a] * importance[b]).sqrt();
        key(p).total_cmp(&key(q))
    });
    for (a, b, _) in order {
        let (sa, sb) = (net.sites[a], net.sites[b]);
        let (ca, cb) = (cell_of(at[a].0, at[a].1), cell_of(at[b].0, at[b].1));
        let Some(path) = router.search(ca, cb) else { continue };

        for (first, last) in fresh_runs(&path, &router.used) {
            // Where this piece begins and ends: at a site, or on the trail it joins.
            let join = |index: usize, lines: &HashMap<u32, Vec<Vec2>>, used: &[u32]| -> Option<Vec2> {
                let neighbour = path.get(index)?;
                let owner = used[*neighbour as usize];
                let cell = *neighbour;
                let target = vec2(((cell as usize % c.size) as f32 + 0.5) * cs, ((cell as usize / c.size) as f32 + 0.5) * cs);
                lines
                    .get(&owner)?
                    .iter()
                    .min_by(|p, q| p.distance_squared(target).total_cmp(&q.distance_squared(target)))
                    .copied()
            };
            let from_site = first == 0;
            let to_site = last == path.len() - 1;
            let start = if from_site {
                Some(vec2(at[a].0, at[a].1))
            } else {
                join(first - 1, &lines, &router.used)
            };
            let end = if to_site {
                Some(vec2(at[b].0, at[b].1))
            } else {
                join(last + 1, &lines, &router.used)
            };
            let (Some(start), Some(end)) = (start, end) else { continue };
            if last - first < 2 && !(from_site || to_site) {
                continue;
            }
            let cells_here = &path[first..=last];
            let pts = fit(c, world_seed, &route::smooth(c, cells_here, start, end, next_id));
            if !from_site {
                net.junctions.push(start);
            }
            if !to_site {
                net.junctions.push(end);
            }
            let start_y = from_site.then_some(sa.y as f32 - 1.0);
            let end_y = to_site.then_some(sb.y as f32 - 1.0);
            let id = next_id;
            next_id += 1;
            lines.insert(id, pts.clone());
            for &cell in cells_here {
                router.used[cell as usize] = id;
            }
            pending.push((pts, cells_here.to_vec(), start_y, end_y, id));
        }
    }

    // Now the whole network is known, how much each stretch is used sets how it is worn.
    let site_cells: Vec<(usize, f32)> = cells.iter().zip(&importance).map(|(&cell, &w)| (cell, w)).collect();
    let traffic = traffic(c, &router.used, &site_cells);
    for (pts, cells_here, start_y, end_y, id) in pending {
        // Each point takes the traffic of the map cell it was routed through, averaged
        // along the way so the width changes gradually.
        let raw: Vec<f32> = (0..pts.len()).map(|k| traffic[cells_here[k * cells_here.len() / pts.len()] as usize]).collect();
        const BLEND: usize = 40;
        let widths: Vec<f32> = (0..raw.len())
            .map(|k| {
                let (lo, hi) = (k.saturating_sub(BLEND), (k + BLEND + 1).min(raw.len()));
                width_for(raw[lo..hi].iter().sum::<f32>() / (hi - lo) as f32)
            })
            .collect();
        layer.lay(pts, &widths, start_y, end_y, id, &mut net);
    }

    // Signposts where trails meet.
    let mut junction_cells = std::collections::HashSet::new();
    for j in net.junctions.clone() {
        if !junction_cells.insert(((j.x / m(6.4)) as i32, (j.y / m(6.4)) as i32)) {
            continue;
        }
        let near: Vec<f32> = lines
            .values()
            .filter_map(|line| {
                let k = line.iter().position(|p| p.distance_squared(j) < m(16.0).powi(2) && p.distance_squared(j) > m(5.6).powi(2))?;
                let d = line[k] - j;
                Some(d.y.atan2(d.x))
            })
            .collect();
        let x = (j.x + m(2.4)) as i32;
        let z = (j.y + m(2.4)) as i32;
        let col = Sampler::new(c, world_seed, x, z, 1).column(x, z);
        if is_wet(&col) || col.river < m(2.4) {
            continue;
        }
        let (trail_y, _) = net
            .points()
            .fold((col.ground, f32::MAX), |(y, best), p| {
                let d = (p.x - j.x).hypot(p.z - j.y);
                if d < best { (p.y, d) } else { (y, best) }
            });
        let directions: &[f32] = if near.is_empty() { &[0.0, 2.1] } else { &near };
        let voxels = landmarks::signpost(&Palette::of(col.biome), directions);
        net.add_stamp(absolute(voxels, IVec3::new(x, trail_y.floor() as i32 + 1, z), 0));
    }

    net
}
