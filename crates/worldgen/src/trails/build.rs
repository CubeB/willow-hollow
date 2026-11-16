//! Assembles a continent's network: settles the sites on real ground, joins them up,
//! lays each trail over the terrain and builds what it needs to cross water.

use std::{
    cmp::Reverse,
    collections::{BTreeMap, BinaryHeap, HashMap},
};

use glam::{IVec2, IVec3, Vec2, Vec3, vec2};
use voxel::Voxel;

use super::{
    Crossing, CrossingKind, LanternCause, Mooring, Network, Pad, Site, TrailLantern, TrailPoint,
    route::{self, Router},
    sites, siting,
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

/// Where a signpost stands from the point two trails meet at.
const SIGNPOST: Vec2 = Vec2::splat(m(2.4));
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
const FIT_SHIFTS: usize = 9;
/// How far, in shifts, a trail leans to either side of its routed line, and over how
/// many fitted points one swing of the lean runs.
const FIT_LEAN: f32 = 3.0;
const FIT_SWING: f64 = 9.0;
/// Steepest a trail may rise, as a slope.
const MAX_SLOPE: f32 = 0.45;
/// Longest water a trail will bridge, in trail steps.
const MAX_SPAN: usize = 90;
/// Longest link in the site graph.
const MAX_LINK: f32 = m(4800.0);
/// Trail steps between lanterns near landmarks, and much further apart elsewhere (the
/// latter jittered by 0.7 to 1.3 times).
const LANTERN_NEAR: usize = 150;
pub(crate) const LANTERN_FAR: usize = 400;
/// How close to a junction a lantern marks it.
const AT_JUNCTION: f32 = m(12.0);
/// No lantern this close to one on another trail, nor at a junction this close to any.
pub(crate) const LANTERN_CLEAR: f32 = m(9.0);
pub(crate) const JUNCTION_CLEAR: f32 = m(16.0);

/// Quarter turns that best point a model's +X along `d`.
fn turns_toward(d: Vec2) -> u32 {
    // rotate() takes +X to (0,1), (-1,0) and (0,-1) for one, two and three turns.
    let axes = [vec2(1.0, 0.0), vec2(0.0, 1.0), vec2(-1.0, 0.0), vec2(0.0, -1.0)];
    (0..4).max_by(|&a, &b| axes[a].dot(d).total_cmp(&axes[b].dot(d))).unwrap() as u32
}

/// Steps to the next lantern along a plain stretch, after one placed at step `i`.
fn plain_gap(salt: u32, i: usize) -> usize {
    (LANTERN_FAR as f32 * (0.7 + 0.6 * unit(hash2(salt ^ 0x1a77, i as i32, 17)))) as usize
}
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

/// Longest span, in trail steps, one felled trunk will cross.
const LOG_SPAN: usize = 16;

/// Width, in simulation units, of the river whose channel passes nearest continent-local
/// `p`, or 0 if none passes within reach (a lake's edge, say). Read from the drifted
/// river nodes the surface draws its channels between, not the grid cells, since a
/// river can wander a cell or more off its cell's centre.
pub fn river_width_at(c: &Continent, p: Vec2) -> f32 {
    let cs = c.cell_size();
    let n = c.size as i32;
    let (x, z) = ((p.x / cs) as i32, (p.y / cs) as i32);
    let (mut best, mut width) = (f32::MAX, 0.0);
    for dz in -2..=2 {
        for dx in -2..=2 {
            let (nx, nz) = (x + dx, z + dz);
            if nx < 0 || nz < 0 || nx >= n || nz >= n {
                continue;
            }
            let i = (nz * n + nx) as usize;
            let r = c.rcv[i] as usize;
            if c.ocean[i] || r == i || c.discharge[i] < crate::surface::MIN_RIVER_Q {
                continue;
            }
            let (a, b) = (crate::surface::node(c, i), crate::surface::node(c, r));
            let ab = b - a;
            let t = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
            let w = Continent::river_width(c.discharge[i]);
            // Measured from the bank rather than the middle, so the wide river is not
            // passed over for a brook whose centreline happens to be nearer.
            let d = (p - (a + ab * t)).length() - w * OLD_VOXEL * 0.5;
            if d < best {
                (best, width) = (d, w);
            }
        }
    }
    if best < m(20.0) { width } else { 0.0 }
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
pub(crate) const HIGH: (f32, f32) = (m(100.0), m(200.0));
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
    // The line the trail leans toward is not quite the routed one but a slow swing
    // either side of it, so a long stretch over open ground wanders as a path walked by
    // eye does. It is only a lean: a rise, a dip or wet ground outweighs it, so the
    // bends it makes are taken where the land allows them. Faded out at the ends.
    let salt = hash2(seed ^ 0x3e4d, pts[0].x as i32, pts[0].y as i32);
    let lean: Vec<f32> = (0..at.len())
        .map(|j| {
            let fade = (j.min(last - j) as f32 / 6.0).min(1.0);
            let swing = perlin2(salt, j as f64 / FIT_SWING, (salt % 997) as f64 * 0.37);
            (swing * 2.6).clamp(-1.0, 1.0) * FIT_LEAN * fade
        })
        .collect();
    let place = |j: usize, k: usize| -> f32 {
        if (j == 0 || j == last) && k != mid {
            return f32::INFINITY;
        }
        let col = &cols[j * FIT_SHIFTS + k];
        let mut cost = 0.35 * (k as f32 - mid as f32 - lean[j]).abs();
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
/// About how wide one arch of a stone bridge spans, in voxels.
const ARCH: f32 = 14.0;

/// Where along a stone bridge over the water run `run` (`len` voxels long) its piers
/// stand, in voxels from where the water begins.
///
/// The river's own channel, the deepest stretch of running water, is spanned by one
/// arch with a pier clear of it on either side, so the stream flows through the arch
/// rather than against a pier. Any shallows or marsh either side get arches of their own.
fn pier_sites(cols: &[Column], (s0, e0): (usize, usize), len: f32) -> Vec<f32> {
    let even = |from: f32, to: f32, out: &mut Vec<f32>| {
        let n = (((to - from) / ARCH).round() as usize).max(1);
        out.extend((1..n).map(|j| from + (to - from) * j as f32 / n as f32));
    };
    // Runs of river channel along the crossing, with the deepest water in each.
    let mut best: Option<(usize, usize, i32)> = None;
    let mut i = s0;
    while i <= e0 {
        let channel = |c: &Column| c.cover == Cover::RiverBed && c.water_surface().is_some();
        if !channel(&cols[i]) {
            i += 1;
            continue;
        }
        let start = i;
        let mut deep = 0;
        while i <= e0 && channel(&cols[i]) {
            deep = deep.max(cols[i].water_surface().map_or(0, |k| k - cols[i].top()));
            i += 1;
        }
        if best.is_none_or(|b| deep > b.2) {
            best = Some((start, i - 1, deep));
        }
    }
    let mut piers = Vec::new();
    // A broad river is crossed by a row of arches with piers standing in it.
    let Some((a, b, _)) = best.filter(|&(a, b, _)| (b - a + 1) as f32 * OLD_VOXEL <= ARCH * 1.6) else {
        even(0.0, len, &mut piers);
        return piers;
    };
    // The channel's extent along the bridge, with a little room either side.
    let lo = (a - s0) as f32 * OLD_VOXEL - 1.0 - PIER * 0.5;
    let hi = (b - s0 + 1) as f32 * OLD_VOXEL + 1.0 + PIER * 0.5;
    // Too little room for an arch of its own beside the channel: the channel's arch
    // runs on to the bank instead.
    const SIDE: f32 = 6.0;
    if lo > SIDE {
        even(0.0, lo, &mut piers);
        piers.push(lo);
    }
    if len - hi > SIDE {
        piers.push(hi);
        even(hi, len, &mut piers);
    }
    piers
}

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
        piers = pier_sites(cols, run, len);
        let n = piers.len() + 1;
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

/// How a landmark is laid out for the ways about it, in model space: which way its door
/// faces (as quarter turns of +X; None if it has none), how far out the door's step is,
/// the radius of the worn way round it as a share of the pad's, the radius of a worn
/// patch about its middle, and whether trails stop at that way round rather than
/// running on into the building.
struct Layout {
    front: Option<u32>,
    door: f32,
    ring: f32,
    hub: f32,
    trim: bool,
}

fn layout(kind: Landmark) -> Layout {
    let none = Layout { front: None, door: 0.0, ring: 0.0, hub: 0.0, trim: false };
    match kind {
        // The tower's door faces +X, its step 12 to 13 voxels out.
        Landmark::Lookout => Layout { front: Some(0), door: 13.5, ring: 0.88, hub: 0.0, trim: true },
        // The hut's on +Z, 10 to 12 out.
        Landmark::Hut => Layout { front: Some(1), door: 11.5, ring: 0.84, hub: 0.0, trim: true },
        // The way through the shrine's gate comes from -Z.
        Landmark::Shrine => Layout { front: Some(3), door: 9.0, ring: 0.92, hub: 0.0, trim: false },
        // The keep's broken door is on -Z. Its old paths run on in among the walls.
        Landmark::Ruins => Layout { front: Some(3), door: 17.0, ring: 0.88, hub: 0.0, trim: false },
        // People sit about the fire and walk round the cairn.
        Landmark::Camp => Layout { hub: m(3.0), ..none },
        Landmark::Cairn => Layout { hub: m(3.0), ..none },
        // The way leads through the arch, along its Z.
        Landmark::Arch => Layout { front: Some(1), ..none },
        Landmark::Stones | Landmark::Landing => none,
    }
}

/// The side of a landmark that is turned to face the way most people come, in quarter
/// turns from +X (1 is +Z), or None if it is turned as its seed has it. The structure
/// editor shows it, so a saved structure keeps its door where the trail arrives.
pub fn front(kind: Landmark) -> Option<u32> {
    layout(kind).front
}

/// A land site placed but not yet built: it is built once the trails to it are known,
/// so its door can face the way most people come.
struct PendingSite {
    index: usize,
    kind: Landmark,
    origin: IVec3,
    seed: u32,
    palette: Palette,
    water: Option<Vec2>,
    /// A ruin on a hilltop, built as a tumbledown tower and seen from the trails, with a
    /// faint path out to it.
    tower: bool,
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

/// The building materials of the place at continent-local column `(lx, lz)` and height
/// `y`: stone from the rock in the ground there, timber from the trees of the country.
pub(super) fn palette_at(c: &Continent, seed: u32, biome: crate::biome::Biome, lx: i32, y: i32, lz: i32) -> Palette {
    use crate::macro_layout::MACRO_SIZE;
    Palette::at(seed, biome, c.desc.mx * MACRO_SIZE + lx, y, c.desc.mz * MACRO_SIZE + lz)
}

fn absolute(voxels: Vec<(IVec3, Voxel)>, origin: IVec3, turns: u32) -> Vec<(IVec3, Voxel)> {
    voxels.into_iter().map(|(p, v)| (origin + rotate(p, turns), v)).collect()
}

/// Settled sites with their map cell, and the land sites still to build.
fn place_sites(c: &Continent, seed: u32, comp: &[u32], net: &mut Network) -> (Vec<usize>, Vec<PendingSite>) {
    let mut pending = Vec::new();
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
                    place_land(c, seed, list.0, cell, site_seed, net, &mut pending)
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
    (cells, pending)
}

/// Keeps a site's props back until the trails are down, so none is left on one.
fn hold_props(net: &mut Network, props: Vec<crate::features::props::Placed>, origin: IVec3, turns: u32) {
    for prop in props {
        let foot = prop.footprint().into_iter().map(|c| {
            let r = rotate(IVec3::new(c.x, 0, c.y), turns);
            IVec2::new(origin.x + r.x, origin.z + r.z)
        });
        let foot = foot.collect();
        net.held_props.push((absolute(prop.voxels, origin, turns), parts::place_fine(&prop.fine, origin, turns), foot));
    }
}

#[allow(clippy::too_many_arguments)]
fn place_land(
    c: &Continent,
    seed: u32,
    kind: Landmark,
    cell: usize,
    site_seed: u32,
    net: &mut Network,
    pending: &mut Vec<PendingSite>,
) -> Option<Site> {
    let found = siting::settle(c, seed, kind, cell)?;
    let (x, z) = (found.x, found.z);
    let biome = Sampler::new(c, seed, x, z, 1).column(x, z).biome;
    let y = found.y.round();
    let palette = palette_at(c, seed, biome, x, y as i32, z);
    let history = landmarks::History::of(kind, site_seed);
    let lay = layout(kind);
    let mut pad = Pad::plain(x as f32 + 0.5, z as f32 + 0.5, kind.radius(), y);
    pad.walled = true;
    // A kept place is trodden bare where people walk; a forgotten one has grassed over.
    pad.wear = ((history.care - 0.1) / 0.7).clamp(0.0, 1.0) * (1.0 - 0.3 * history.age);
    pad.neglect = history.neglect();
    pad.door = lay.door;
    pad.ring = lay.ring * kind.radius();
    pad.hub = lay.hub;
    net.pads.push(pad);
    let origin = IVec3::new(x, y as i32 + 1, z);
    // Only a ruin standing well up over the land about it, where a tower would be seen
    // from far off, is built as one.
    let tower = kind == Landmark::Ruins && found.reason == super::Reason::View && {
        let r = TOWER_RISE_AT as i32;
        let s = Sampler::new(c, seed, x - r, z - r, 2 * r + 1);
        let around = (0..8)
            .map(|k| {
                let a = k as f32 * std::f32::consts::FRAC_PI_4;
                s.column(x + (a.cos() * TOWER_RISE_AT) as i32, z + (a.sin() * TOWER_RISE_AT) as i32).ground
            })
            .sum::<f32>()
            / 8.0;
        found.y - around > TOWER_RISE
    };
    pending.push(PendingSite { index: net.sites.len(), kind, origin, seed: site_seed, palette, water: found.water, tower });
    Some(Site {
        kind,
        x,
        z,
        y: origin.y,
        mooring: None,
        reason: found.reason,
    })
}

/// Builds a land site turned so its door faces `toward` (the way most people come), or
/// as its seed has it if nobody comes or it has no door.
fn finish_site(net: &mut Network, site: &PendingSite, toward: Option<Vec2>) -> u32 {
    let lay = layout(site.kind);
    let turns = match (lay.front, toward) {
        (Some(front), Some(d)) => (turns_toward(d) + 4 - front) % 4,
        _ => site.seed & 3,
    };
    let (voxels, fine, props) = if site.tower {
        landmarks::build_ruined_tower(&site.palette, site.seed)
    } else {
        landmarks::build_placed(site.kind, &site.palette, site.seed, 0)
    };
    net.add_stamp_with(absolute(voxels, site.origin, turns), parts::place_fine(&fine, site.origin, turns));
    hold_props(net, props, site.origin, turns);
    turns
}

/// The way from the edge of a site's pad down to the fresh water near it, fitted to the
/// ground like any trail and stopping a pace short of the bank. None when the water is
/// too close to need a path, or the way to it is too short once it stops short.
fn water_path(c: &Continent, seed: u32, pad: &Pad, water: Vec2) -> Option<Vec<Vec2>> {
    let centre = vec2(pad.x, pad.z);
    let dir = (water - centre).normalize_or_zero();
    let (start, end) = (centre + dir * pad.radius, water - dir * m(1.6));
    if dir == Vec2::ZERO || start.distance(end) < m(8.0) {
        return None;
    }
    let pts = fit(c, seed, &route::resample(&[start, end]));
    let cols = sample(c, seed, &pts);
    let stop = cols.iter().position(|col| is_wet(col) || col.river < m(1.2)).unwrap_or(pts.len());
    let keep = stop.saturating_sub(2);
    (keep >= 10).then(|| pts[..keep].to_vec())
}

/// A ruin on ground standing this far over the land `TOWER_RISE_AT` about it is built
/// as a tower.
const TOWER_RISE: f32 = m(4.4);
const TOWER_RISE_AT: f32 = m(50.0);
/// How far about a ruin the trails keep away, going round it.
const RUIN_CLEAR: f32 = m(160.0);
/// The furthest a trail may be from a tower on a hill for a path to go out to it.
const RUIN_PATH_REACH: f32 = m(1600.0);

/// The faint way from a trail at `from` out to the edge of a ruin's pad, along the map
/// cells `cells` and fitted to the ground. None when it is too short to be worth a path,
/// or would need more than a log over a brook: nobody builds a bridge to reach a ruin,
/// though a stream may be stepped across.
fn ruin_path(c: &Continent, seed: u32, pad: &Pad, from: Vec2, cells: &[u32], salt: u32) -> Option<Vec<Vec2>> {
    let centre = vec2(pad.x, pad.z);
    let mut pts = route::smooth(c, cells, from, centre, salt);
    let stop = pts.iter().position(|p| p.distance(centre) < pad.radius)?;
    pts.truncate(stop);
    if pts.len() < 10 {
        return None;
    }
    let pts = fit(c, seed, &pts);
    let cols = sample(c, seed, &pts);
    wet_runs(&cols).iter().all(|&(s, e)| e - s < 10).then_some(pts)
}

fn place_landing(c: &Continent, seed: u32, cell: usize, site_seed: u32, net: &mut Network) -> Option<Site> {
    let shore = find_shore(c, seed, cell)?;
    let biome = Sampler::new(c, seed, shore.x, shore.z, 1).column(shore.x, shore.z).biome;
    let palette = palette_at(c, seed, biome, shore.x, shore.level, shore.z);
    let y = shore.level as f32 + 0.5;
    net.pads.push(Pad::plain(shore.x as f32 + 0.5, shore.z as f32 + 0.5, Landmark::Landing.radius(), y));
    let origin = IVec3::new(shore.x, shore.level + 1, shore.z);
    let (pier, fine, props) = landmarks::build_site(Landmark::Landing, &palette, site_seed, shore.length / BLOCK);
    net.add_stamp_with(absolute(pier, origin, shore.turns), parts::place_fine(&fine, origin, shore.turns));
    hold_props(net, props, origin, shore.turns);
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
        reason: super::Reason::Shore,
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
    /// Buildings trails stop short of: the middle and the radius of the worn way round
    /// it, which takes walkers on to the door.
    trims: Vec<(Vec2, f32)>,
}

impl Layer<'_> {
    /// The palette of the ground at trail point `p`, whose bare column is `col`.
    fn palette_at(&self, col: &Column, p: Vec2) -> Palette {
        palette_at(self.c, self.seed, col.biome, p.x.floor() as i32, col.top(), p.y.floor() as i32)
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
        let traffic = widths;
        let widths = narrow_high(&traffic, &cols);
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
        // Runs waded on stepping stones.
        let mut fords: Vec<(usize, usize)> = Vec::new();
        let mut pins: Vec<(usize, f32)> = Vec::new();
        for &(s, e) in &runs {
            let span = e - s + 1;
            let mid = (s + e) / 2;
            // Built of the rock of the bank it springs from.
            let bank = s.saturating_sub(1);
            let palette = self.palette_at(&cols[bank], pts[bank]);
            let depth = (s..=e)
                .filter_map(|i| cols[i].water_surface().map(|k| k - cols[i].top()))
                .max()
                .unwrap_or(0);
            // By the span and by the river: a long span over a small stream or a lake's
            // edge still gets timber, and only a long span or a big river stone. A log
            // will only reach so far; past that the timber bridge is planked on trestles.
            let width = river_width_at(self.c, pts[mid]);
            let kind = match route::crossing_kind(span, width, depth <= 3 * BLOCK) {
                CrossingKind::Plank if span <= LOG_SPAN && rng.unit() < 0.5 => CrossingKind::Log,
                kind => kind,
            };
            net.crossings.push(Crossing {
                kind,
                x: pts[mid].x as i32,
                z: pts[mid].y as i32,
                span: span as i32,
            });
            if kind == CrossingKind::Stones {
                fords.push((s, e));
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
        // Inside a building's way round, the trail gives way to the worn ground of the site.
        let inside = |p: Vec2| self.trims.iter().any(|&(c, r)| c.distance_squared(p) < r * r);
        let points: Vec<TrailPoint> = pts
            .iter()
            .enumerate()
            .map(|(i, p)| TrailPoint {
                x: p.x,
                z: p.y,
                y: heights[i],
                half_width: if is_carried(i) || inside(*p) { 0.0 } else { worn[i] },
            })
            .collect();
        net.add_points(&points);
        net.lines.push(pts.iter().step_by(8).copied().collect());
        net.line_widths.push(widths.iter().step_by(8).copied().collect());
        cols.truncate(pts.len());
        self.furnish(&pts, &widths, &heights, &cols, &carried, salt, net);
        // Milestones, benches, waymarks and cairns, held like a site's props until every
        // trail is down.
        let trail = super::roadside::Trail {
            pts: &pts,
            traffic: &traffic,
            widths: &widths,
            worn: &worn,
            heights: &heights,
            cols: &cols,
            carried: &carried,
            salt,
        };
        let taken: Vec<Vec2> = net
            .lanterns
            .iter()
            .map(|l| l.at)
            .chain(net.junctions.iter().copied())
            .chain(net.held_props.iter().filter_map(|(_, _, foot)| foot.first().map(|c| c.as_vec2() + 0.5)))
            .chain(net.held_markers.iter().map(|m| m.at))
            .chain(net.held_delights.iter().map(|d| d.at))
            .collect();
        let chosen = super::roadside::choose(self.c, self.seed, &trail, &taken);
        // The small delights, rarer by far, clear of all those and of the signposts
        // that will stand where trails meet.
        let taken: Vec<Vec2> = taken
            .into_iter()
            .filter(|p| !net.junctions.contains(p))
            .chain(chosen.iter().map(|m| m.at))
            .chain(net.junctions.iter().map(|j| *j + SIGNPOST))
            .collect();
        net.held_markers.extend(chosen);
        let delights = super::delights::choose(self.c, self.seed, &trail, &fords, &net.junctions, &taken);
        net.held_delights.extend(delights);
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

    /// Lanterns along the way: at bridge heads, where trails meet and on the approach to
    /// landmarks, and only now and then along plain stretches; never crowding one on
    /// another trail, and only on trails worn enough for anyone to light.
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
        let mut gap = plain_gap(salt, 0) / 2;
        let mut side = if salt & 1 == 0 { 1.0 } else { -1.0 };
        for i in 8..pts.len().saturating_sub(8) {
            since += 1;
            if widths[i] < LIT || carried.iter().any(|&(a, b)| (a..=b).contains(&i)) {
                continue;
            }
            let near_site = self.site_points.iter().any(|s| s.distance_squared(pts[i]) < NEAR_SITE * NEAR_SITE);
            let at_bridge = carried.iter().any(|&(a, b)| i + 3 == a || i == b + 3);
            let at_junction = since >= 12 && net.junctions.iter().any(|j| j.distance_squared(pts[i]) < AT_JUNCTION * AT_JUNCTION);
            let cause = if at_bridge {
                LanternCause::Bridge
            } else if at_junction {
                LanternCause::Junction
            } else if near_site && since >= LANTERN_NEAR {
                LanternCause::Approach
            } else if since >= gap {
                LanternCause::Plain
            } else {
                continue;
            };
            let tangent = (pts[i + 4] - pts[i - 4]).normalize_or_zero();
            let normal = vec2(-tangent.y, tangent.x);
            let col = cols[i];
            if is_wet(&col) || col.river < m(2.4) {
                continue;
            }
            // A pole of sticks beside the way, its lamp hung out toward it. Most are not
            // lit: only now and then one by a bridge or where trails meet, and more often
            // on the way in to a place, where someone is about to light them.
            let style = parts::LampStyle::Stick;
            let lit = unit(hash2(salt ^ 0x11e7, i as i32, 5))
                < match cause {
                    LanternCause::Approach => 0.4,
                    LanternCause::Bridge | LanternCause::Junction => 0.25,
                    LanternCause::Plain => 0.15,
                };
            let (q, turns) = (pts[i] + normal * side * (widths[i] + m(1.0)), turns_toward(-normal * side));
            let clear = if cause == LanternCause::Junction { JUNCTION_CLEAR } else { LANTERN_CLEAR };
            let crowded = net.lanterns.iter().any(|l| {
                (l.trail != salt || cause == LanternCause::Junction) && l.at.distance_squared(q) < clear * clear
            });
            if crowded {
                continue;
            }
            let (x, z) = (q.x.floor() as i32, q.y.floor() as i32);
            let y = heights[i].floor() as i32 + 1;
            let height = 3 + (unit(hash2(salt, x, z)) * 2.0) as i32;
            let at = IVec3::new(x, y, z);
            let (voxels, fine) = landmarks::trail_lantern_styled(&self.palette_at(&col, q), height, hash2(salt, x, z), style, turns, parts::GATE_HALF, lit);
            // Its pole, and whatever of it is low enough to walk into.
            let mut footprint: Vec<IVec2> = voxels
                .iter()
                .filter(|(p, _)| (0..=2).contains(&p.y))
                .map(|(p, _)| IVec2::new(x + p.x, z + p.z))
                .collect();
            footprint.insert(0, IVec2::new(x, z));
            // Nothing stands on the walkable way, of this trail or any laid before it.
            let on_way = footprint.iter().any(|c| {
                let c = c.as_vec2() + 0.5;
                pts[i.saturating_sub(40)..(i + 40).min(pts.len())]
                    .iter()
                    .zip(&widths[i.saturating_sub(40)..(i + 40).min(pts.len())])
                    .any(|(p, &w)| p.distance_squared(c) < (w + 0.5).powi(2))
            });
            if on_way {
                continue;
            }
            net.held_lanterns.push((net.lanterns.len(), absolute(voxels, at, 0), parts::place_fine(&fine, at, 0)));
            net.lanterns.push(TrailLantern { at: q, trail: salt, step: i, cause, style, lit, footprint });
            since = 0;
            gap = plain_gap(salt, i);
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
    let (cells, land_sites) = place_sites(c, world_seed, &comp, &mut net);
    let at: Vec<(f32, f32, u32)> = net
        .sites
        .iter()
        .zip(&cells)
        .map(|(s, &cell)| (s.x as f32 + 0.5, s.z as f32 + 0.5, comp[cell]))
        .collect();

    let ruin = |k: usize| net.sites[k].kind == Landmark::Ruins;
    let layer = Layer {
        c,
        seed: world_seed,
        // Nobody lights the way to a ruin.
        site_points: at.iter().enumerate().filter(|&(k, _)| !ruin(k)).map(|(_, &(x, z, _))| vec2(x, z)).collect(),
        trims: land_sites
            .iter()
            .filter(|s| layout(s.kind).trim)
            .map(|s| {
                let pad = &net.pads[s.index];
                (vec2(pad.x, pad.z), pad.ring)
            })
            .collect(),
    };
    let cs = c.cell_size();
    let cell_of = |x: f32, z: f32| (z / cs) as usize * c.size + (x / cs) as usize;
    let mut router = Router::new(c);
    let mut lines: HashMap<u32, Vec<Vec2>> = HashMap::new();
    let mut next_id = 1u32;
    /// A stretch of trail routed but not yet laid: (points, its map cells, start
    /// height, end height, id, the sites it starts and ends at).
    type Pending = (Vec<Vec2>, Vec<u32>, Option<f32>, Option<f32>, u32, (Option<usize>, Option<usize>));
    let mut pending: Vec<Pending> = Vec::new();
    let importance: Vec<f32> = net.sites.iter().map(|s| sites::importance(s.kind)).collect();

    // The links between the most important places first, so that lesser trails come
    // to join them rather than the other way about.
    // Ruins stand out in the wild, off the trails: no trail is made to one, and those
    // between other places go round them rather than through.
    let mut order = links(&at);
    order.retain(|&(a, b, _)| !ruin(a) && !ruin(b));
    for (k, &(x, z, _)) in at.iter().enumerate() {
        if ruin(k) {
            router.avoid(cell_of(x, z), RUIN_CLEAR, 8.0);
        }
    }
    order.sort_by(|p, q| {
        let key = |&(a, b, d): &(usize, usize, f32)| d / (importance[a] * importance[b]).sqrt();
        key(p).total_cmp(&key(q))
    });
    for (a, b, _) in order {
        let (sa, sb) = (net.sites[a], net.sites[b]);
        let (ca, cb) = (cell_of(at[a].0, at[a].1), cell_of(at[b].0, at[b].1));
        let Some(path) = router.search(ca, cb) else {
            net.unlinked += 1;
            continue;
        };

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
            let ends = (from_site.then_some(a), to_site.then_some(b));
            pending.push((pts, cells_here.to_vec(), start_y, end_y, id, ends));
        }
    }

    // Now the whole network is known, how much each stretch is used sets how it is worn.
    // Trips start from the cell each site settled in, where its trails were routed from,
    // which need not be the cell it was chosen by.
    let site_cells: Vec<(usize, f32)> = at.iter().zip(&importance).map(|(&(x, z, _), &w)| (cell_of(x, z), w)).collect();
    let traffic = traffic(c, &router.used, &site_cells);
    let mut stretches = Vec::with_capacity(pending.len());
    // Where each trail comes in to each site, and how busy it is there.
    let mut arrivals: Vec<Vec<(Vec2, f32)>> = vec![Vec::new(); net.sites.len()];
    for (pts, cells_here, start_y, end_y, id, ends) in pending {
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
        for (site, from_end) in [(ends.0, false), (ends.1, true)] {
            let Some(s) = site else { continue };
            let pad = net.pads[s];
            let centre = vec2(pad.x, pad.z);
            let mut walk: Box<dyn Iterator<Item = &Vec2>> = if from_end { Box::new(pts.iter().rev()) } else { Box::new(pts.iter()) };
            if let Some(p) = walk.find(|p| p.distance(centre) >= pad.radius) {
                let busy = if from_end { raw[raw.len() - 1] } else { raw[0] };
                arrivals[s].push((*p - centre, busy));
            }
        }
        stretches.push((pts, widths, start_y, end_y, id));
    }

    // A tumbledown tower on a hilltop is seen from the trails and draws the odd walker
    // off them: a faint path, broken and overgrown, from the nearest trail out to it,
    // which its door faces. The other ruins are left to whoever stumbles on them.
    let mut ruin_paths = Vec::new();
    for site in land_sites.iter().filter(|s| s.tower) {
        let pad = net.pads[site.index];
        let centre = vec2(pad.x, pad.z);
        let nearest = stretches
            .iter()
            .flat_map(|s| s.0.iter())
            // From somewhere along the way, not from the middle of a place it starts at.
            .filter(|p| !net.pads.iter().any(|q| vec2(q.x, q.z).distance(**p) < q.radius + m(8.0)))
            .min_by(|p, q| p.distance_squared(centre).total_cmp(&q.distance_squared(centre)))
            .copied();
        if let Some(from) = nearest.filter(|p| p.distance(centre) < RUIN_PATH_REACH)
            && let Some(cells) = router.search(cell_of(from.x, from.y), cell_of(centre.x, centre.y))
            && let Some(pts) = ruin_path(c, world_seed, &pad, from, &cells, site.seed)
        {
            arrivals[site.index].push((pts[pts.len() - 1] - centre, 0.0));
            ruin_paths.push((pts, pad.y));
        }
    }

    // The land sites, each turned so its door faces the busiest way in, with the worn
    // ways from every path round to that door, and a path down to water for those in use.
    let mut spurs = Vec::new();
    for site in &land_sites {
        let ways = &arrivals[site.index];
        let main = ways.iter().max_by(|a, b| a.1.total_cmp(&b.1)).map(|w| w.0);
        let turns = finish_site(&mut net, site, main);
        let lay = layout(site.kind);
        let pad = &mut net.pads[site.index];
        if let Some(front) = lay.front
            && lay.door > 0.0
        {
            let f = rotate(rotate(IVec3::X, front), turns);
            pad.front = (f.z as f32).atan2(f.x as f32);
        }
        for w in ways.iter().take(pad.arrivals.len()) {
            pad.arrivals[pad.arrival_count as usize] = w.0.y.atan2(w.0.x);
            pad.arrival_count += 1;
        }
        if let Some(water) = site.water
            && pad.wear > 0.3
            && matches!(site.kind, Landmark::Hut | Landmark::Camp | Landmark::Shrine | Landmark::Lookout)
            && let Some(spur) = water_path(c, world_seed, pad, water)
        {
            if (pad.arrival_count as usize) < pad.arrivals.len() {
                let d = spur[1] - spur[0];
                pad.arrivals[pad.arrival_count as usize] = d.y.atan2(d.x);
                pad.arrival_count += 1;
            }
            spurs.push((spur, pad.y));
        }
    }

    for (pts, widths, start_y, end_y, id) in stretches {
        layer.lay(pts, &widths, start_y, end_y, id, &mut net);
    }
    // The paths to water: narrow, from the edge of the pad to a pace short of the bank.
    let after = next_id + spurs.len() as u32;
    for (k, (pts, y)) in spurs.into_iter().enumerate() {
        let widths = vec![m(0.45); pts.len()];
        layer.lay(pts, &widths, Some(y), None, next_id + k as u32, &mut net);
    }
    // The ways out to the towers: as narrow as a trail is worn, so they come and go.
    for (k, (pts, y)) in ruin_paths.into_iter().enumerate() {
        let widths = vec![widths_min(); pts.len()];
        layer.lay(pts, &widths, None, Some(y), after + k as u32, &mut net);
    }

    // Lanterns, now every trail is down, except any a later trail runs over.
    let walked: Vec<Vec2> = net.points().filter(|p| p.half_width > 0.0).map(|p| vec2(p.x, p.z)).collect();
    let widths: Vec<f32> = net.points().filter(|p| p.half_width > 0.0).map(|p| p.half_width).collect();
    let mut keep = vec![false; net.lanterns.len()];
    for (k, voxels, fine) in std::mem::take(&mut net.held_lanterns) {
        let l = &net.lanterns[k];
        let near: Vec<usize> = (0..walked.len()).filter(|&n| walked[n].distance_squared(l.at) < m(12.0).powi(2)).collect();
        let on_way = l.footprint.iter().any(|c| {
            let c = c.as_vec2() + 0.5;
            near.iter().any(|&n| walked[n].distance_squared(c) < (widths[n] + 0.5).powi(2))
        });
        if !on_way {
            keep[k] = true;
            net.add_stamp_with(voxels, fine);
        }
    }
    let mut k = 0;
    net.lanterns.retain(|_| {
        k += 1;
        keep[k - 1]
    });

    // Site props, except any standing where a trail runs.
    for (voxels, fine, foot) in std::mem::take(&mut net.held_props) {
        let Some(first) = foot.first() else { continue };
        let first = first.as_vec2() + 0.5;
        let near: Vec<usize> = (0..walked.len()).filter(|&n| walked[n].distance_squared(first) < m(12.0).powi(2)).collect();
        let on_way = foot.iter().any(|c| {
            let c = c.as_vec2() + 0.5;
            near.iter().any(|&n| walked[n].distance_squared(c) < (widths[n] + 0.5).powi(2))
        });
        if !on_way {
            net.add_stamp_with(voxels, fine);
        }
    }

    // Trail markers, likewise.
    for marker in std::mem::take(&mut net.held_markers) {
        let near: Vec<usize> = (0..walked.len()).filter(|&n| walked[n].distance_squared(marker.at) < m(12.0).powi(2)).collect();
        let on_way = marker.footprint.iter().any(|c| {
            let c = c.as_vec2() + 0.5;
            near.iter().any(|&n| walked[n].distance_squared(c) < (widths[n] + 0.5).powi(2))
        });
        if !on_way {
            net.markers.push((marker.kind, marker.origin));
            net.add_stamp_with(marker.voxels, marker.fine);
        }
    }

    // The small delights, likewise; and none by a site, which has its own things, nor
    // two near each other, so that each stays a surprise.
    for d in std::mem::take(&mut net.held_delights) {
        use super::delights::{APART, APART_SAME, Standing};
        let near: Vec<usize> = (0..walked.len()).filter(|&n| walked[n].distance_squared(d.at) < m(16.0).powi(2)).collect();
        let on_way = d.footprint.iter().any(|c| {
            let c = c.as_vec2() + 0.5;
            near.iter().any(|&n| walked[n].distance_squared(c) < (widths[n] + 0.5).powi(2))
        });
        let at = vec2(d.origin.x as f32, d.origin.z as f32);
        let by_site = net.pads.iter().any(|p| vec2(p.x, p.z).distance(at) < p.radius + m(12.0));
        let crowded = net.delights.iter().any(|o| {
            let gap = vec2(o.at.x as f32, o.at.z as f32).distance(at);
            gap < APART || (o.kind == d.kind && gap < APART_SAME)
        });
        if !on_way && !by_site && !crowded {
            net.delights.push(Standing { kind: d.kind, at: d.origin, footprint: d.footprint });
            net.add_stamp_with(d.voxels, d.fine);
        }
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
        let x = (j.x + SIGNPOST.x) as i32;
        let z = (j.y + SIGNPOST.y) as i32;
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
        let voxels = landmarks::signpost(&palette_at(c, world_seed, col.biome, x, col.top(), z), directions);
        net.add_stamp(absolute(voxels, IVec3::new(x, trail_y.floor() as i32 + 1, z), 0));
    }

    terrace_walls(c, world_seed, &mut net);
    net
}

/// The dry-stone walls that hold each terraced pad: wherever the ground just outside a
/// pad steps two voxels or more from its neighbours (see the terrace in
/// [`Sampler::column`]), a wall fills the step, from a course below the lower side to a
/// coping at the higher, with a kerb along the top where the pad is built out over a
/// drop. Read from the finished ground, trails and all, so each wall meets the ground the
/// game will make. On a neglected site the coping has fallen here and there.
fn terrace_walls(c: &Continent, seed: u32, net: &mut Network) {
    let mut walls = Vec::new();
    for pad in net.pads.iter().filter(|p| p.walled) {
        let centre = vec2(pad.x, pad.z);
        let reach = (pad.radius + crate::surface::TERRACE_RING).ceil() as i32 + 1;
        let (cx, cz) = (pad.x.floor() as i32, pad.z.floor() as i32);
        let s = Sampler::with_network(c, seed, cx - reach - 1, cz - reach - 1, 2 * reach + 3, net);
        // The same stone as the place it holds up (see `place_land`).
        let pal = palette_at(c, seed, s.column(cx, cz).biome, cx, pad.y.round() as i32, cz);
        let mut rng = Rng::new(hash2(seed ^ 0x07e4_4ace, cx, cz));
        let pad_top = pad.y.floor() as i32;
        let mut voxels = Vec::new();
        for z in cz - reach..=cz + reach {
            for x in cx - reach..=cx + reach {
                let d = vec2(x as f32 + 0.5, z as f32 + 0.5).distance(centre) - pad.radius;
                if !(0.0..crate::surface::TERRACE_RING).contains(&d) {
                    continue;
                }
                let col = s.column(x, z);
                if is_wet(&col) {
                    continue;
                }
                let g = col.top();
                let mut top = g;
                for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let n = s.column(x + dx, z + dz);
                    if !is_wet(&n) {
                        top = top.max(n.top());
                    }
                }
                if top - g < 2 {
                    continue;
                }
                let worn = rng.unit() < pad.neglect * 0.5;
                let cap = if worn { top - 1 } else { top };
                for y in g - 1..=cap {
                    voxels.push((IVec3::new(x, y, z), if y == top { pal.stone_dark } else { pal.block(&mut rng) }));
                }
                // Built out over a drop: a kerb along the edge.
                if top == pad_top && top - g >= 4 && !worn {
                    voxels.push((IVec3::new(x, top + 1, z), pal.stone_dark));
                }
            }
        }
        walls.push(voxels);
    }
    for voxels in walls {
        net.add_stamp(voxels);
    }
}
