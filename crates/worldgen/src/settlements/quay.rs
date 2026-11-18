//! Quays and the ways boats go from them (#205): a timber jetty at each end of a river's
//! trade run, where the boats between two partner settlements tie up, and one where a
//! fishing settlement on the sea or a lake puts out from, unless it has a landing already.
//!
//! The runs themselves are worked out here too, once, from the continent's rivers: each is
//! the river's own middle line from one quay to the other, as the surface sampler draws the
//! channel (the same curves through the same meandering nodes), with the level of the
//! water along it. A fishing quay keeps a few grounds out on open water, each reached in a
//! straight line from its berth over nothing but water. The timetables that put boats on
//! these lines are in [`super::travel`].
//!
//! A jetty stands on the bank nearest its settlement, square to the bank, and reaches out
//! no more than a third of the way across a river, so a river is never blocked. It keeps
//! clear of the plots, lanes and tree ground of the settlement, and of anything already
//! standing beside the water.

use std::collections::HashSet;

use glam::{IVec2, IVec3, Vec2, Vec3, vec2};
use voxel::{AIR, Voxel};

use super::{Livelihood, Water};
use crate::{
    continent::{Continent, NO_LAKE},
    features::{landmarks::Palette, shape::Rng},
    materials as mat,
    noise::{hash2, unit},
    scale::m,
    surface::{MIN_RIVER_Q, Sampler, fall_drop, node, pool_level, river_level},
    trails::{Network, build::palette_at},
};

/// What a quay serves.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Serves {
    /// The trade boats of a river, between partner settlements.
    River,
    /// The fishing boats of a settlement on the sea or a lake.
    Fishing,
}

/// One quay: a jetty built for the boats, or a landing the settlement had already.
#[derive(Clone, Debug)]
pub struct Quay {
    /// Whose it is, by index in `Network::settlements`.
    pub settlement: usize,
    pub serves: Serves,
    /// Where a boat lies at it, on the water's surface (continent-local voxels), and the
    /// way its bow points there, radians as a camera's yaw.
    pub berth: Vec3,
    pub heading: f32,
    /// The landing it is, by index in `Network::sites`, when the settlement had one.
    pub landing: Option<usize>,
    /// A jetty built here: the middle of its foot on the bank, and of its end out over
    /// the water. The same point twice for a landing.
    pub foot: Vec2,
    pub head: Vec2,
    /// The water's surface there: the layer above the topmost water voxel.
    pub level: i32,
    /// The columns its jetty stands on; none for a landing.
    pub footprint: Vec<IVec2>,
    /// The map cell of the river beside it (a river quay), for finding the run.
    pub cell: usize,
}

/// The way along a river between two partners' quays.
#[derive(Clone, Debug)]
pub struct Run {
    /// The two settlements, the upstream one first, and their quays, by index in
    /// [`Waterways::quays`].
    pub a: usize,
    pub b: usize,
    pub from: usize,
    pub to: usize,
    /// From the first quay's berth to the second's, on the water's surface: down the
    /// middle of the river, a few voxels between points.
    pub line: Vec<Vec3>,
    /// How far each point of `line` is along it, in voxels.
    pub along: Vec<f32>,
    /// The map cells the river runs through between them, for checking.
    pub cells: Vec<usize>,
}

impl Run {
    pub fn length(&self) -> f32 {
        self.along.last().copied().unwrap_or(0.0)
    }

    /// Where on the water a boat `s` voxels along from the first quay is, and the way
    /// downstream there (radians, as a camera's yaw).
    pub fn at(&self, s: f32) -> (Vec3, f32) {
        along_line(&self.line, &self.along, s)
    }
}

/// Where a fishing boat goes from its quay: out in a straight line over open water.
#[derive(Clone, Debug)]
pub struct Ground {
    /// Its quay, by index in [`Waterways::quays`].
    pub quay: usize,
    /// The middle of the ground, on the water's surface.
    pub at: Vec3,
}

/// Every quay of a continent, the river runs between them and the fishing grounds.
#[derive(Clone, Debug, Default)]
pub struct Waterways {
    pub quays: Vec<Quay>,
    pub runs: Vec<Run>,
    pub grounds: Vec<Ground>,
}

impl Waterways {
    /// The fishing grounds of the quay at `q`.
    pub fn grounds_of(&self, q: usize) -> impl Iterator<Item = &Ground> {
        self.grounds.iter().filter(move |g| g.quay == q)
    }
}

/// A point `s` voxels along a line, and the way the line runs there.
pub(crate) fn along_line(line: &[Vec3], along: &[f32], s: f32) -> (Vec3, f32) {
    if line.len() < 2 {
        return (line.first().copied().unwrap_or(Vec3::ZERO), 0.0);
    }
    let s = s.clamp(0.0, along[along.len() - 1]);
    let k = along.partition_point(|&a| a <= s).clamp(1, line.len() - 1);
    let (a, b) = (line[k - 1], line[k]);
    let t = ((s - along[k - 1]) / (along[k] - along[k - 1]).max(1e-4)).clamp(0.0, 1.0);
    let d = b - a;
    (a + d * t, d.z.atan2(d.x))
}

/// How far each point of a line is along it.
pub(crate) fn measure(line: &[Vec3]) -> Vec<f32> {
    let mut s = 0.0;
    let mut out = Vec::with_capacity(line.len());
    for (k, p) in line.iter().enumerate() {
        if k > 0 {
            s += vec2(p.x - line[k - 1].x, p.z - line[k - 1].z).length();
        }
        out.push(s);
    }
    out
}

/// Steepest a river may fall and still be calm for boats: as `place.rs` reckons the
/// partners, so that a run never takes in water it would not.
const CALM: f32 = 0.04;
/// Segments each river curve is cut into, as the sampler does.
const SEGS: usize = 6;
/// How many cells up and down its river a settlement's quay may stand from the cell it
/// was settled by.
const QUAY_CELLS: usize = 4;
/// Farthest a quay is looked for from a settlement's middle, beyond its buildings.
const QUAY_REACH: f32 = m(140.0);
/// Most of a river's width a jetty takes, and the least water it leaves beyond its end.
const JETTY_SHARE: f32 = 0.3;
const OPEN_BEYOND: f32 = m(2.4);
/// Shortest and longest a jetty runs out over the water.
const SHORTEST: f32 = m(1.2);
const LONGEST: f32 = m(6.0);
/// A run shorter than this is not worth a boat: the two places are a walk apart.
const SHORTEST_RUN: f32 = m(150.0);
/// How far out fishing grounds are looked for, and the least they must be out.
const GROUND_FAR: f32 = m(420.0);
const GROUND_NEAR: f32 = m(40.0);
/// How far short of land a fishing ground lies: further than a boat drifts about it.
pub const GROUND_SHORT: f32 = m(20.0);
/// How far a fishing boat drifts about its ground while the lines are out.
pub const DRIFT: f32 = m(6.0);

/// Whether the river leaving cell `i` is calm all the way to the next: as
/// `place::Map::calm` has it.
pub(crate) fn calm(c: &Continent, i: usize) -> bool {
    let r = c.rcv[i] as usize;
    if c.lake[i] != NO_LAKE && c.lake[r] != NO_LAKE {
        return true;
    }
    if fall_drop(c, i) > 0.0 {
        return false;
    }
    let (x, z) = ((i % c.size) as f32, (i / c.size) as f32);
    let (rx, rz) = ((r % c.size) as f32, (r / c.size) as f32);
    let run = (x - rx).hypot(z - rz) * c.sim_cell_size();
    (c.water_level[i] - c.water_level[r]) / run.max(1.0) <= CALM
}

fn is_river(c: &Continent, i: usize) -> bool {
    !c.ocean[i] && c.discharge[i] >= MIN_RIVER_Q
}

/// The biggest river flowing into each cell, or `u32::MAX` for none.
fn inflows(c: &Continent) -> Vec<u32> {
    let mut up = vec![u32::MAX; c.size * c.size];
    let mut best = vec![0.0f32; c.size * c.size];
    for i in 0..c.size * c.size {
        let r = c.rcv[i] as usize;
        if r == i || !is_river(c, i) {
            continue;
        }
        if c.discharge[i] > best[r] {
            best[r] = c.discharge[i];
            up[r] = i as u32;
        }
    }
    up
}

/// The cells from `from` down to `to`, both in, if `to` lies downstream on water that runs
/// calm all the way.
pub(crate) fn down_to(c: &Continent, from: usize, to: usize) -> Option<Vec<usize>> {
    let mut cells = vec![from];
    let mut i = from;
    for _ in 0..c.size * 4 {
        if i == to {
            return Some(cells);
        }
        let r = c.rcv[i] as usize;
        if r == i || c.ocean[r] || !calm(c, i) {
            return None;
        }
        i = r;
        cells.push(i);
    }
    None
}

/// The water's surface over river cell `i` at simulated level `level`: the layer above
/// its topmost water voxel, as the sampler fills it.
fn surface(c: &Continent, i: usize, level: f32) -> i32 {
    let k = pool_level(c.desc.seed, level);
    if c.lake[i] != NO_LAKE { k.min(pool_level(c.desc.seed, c.lake[i])) } else { k }
}

/// The middle line of the river through `cells` (each draining into the next), with the
/// water's surface along it and the cell each point is beside: the curve the sampler
/// draws the channel about, from the middle of the way into the first cell to the middle
/// of the way out of the last.
pub(crate) fn middle(c: &Continent, up: &[u32], cells: &[usize]) -> Vec<(Vec3, usize)> {
    let mut out = Vec::new();
    for (k, &i) in cells.iter().enumerate() {
        let prev = if k > 0 { Some(cells[k - 1]) } else { Some(up[i] as usize).filter(|_| up[i] != u32::MAX) };
        let next = cells.get(k + 1).copied().unwrap_or(c.rcv[i] as usize);
        let (pi, pn) = (node(c, i), node(c, next));
        let p0 = prev.map_or(pi, |p| (node(c, p) + pi) * 0.5);
        let p2 = (pi + pn) * 0.5;
        let li = river_level(c, i);
        let l0 = prev.map_or(li, |p| (river_level(c, p) + li) * 0.5);
        let l2 = (li + river_level(c, next)) * 0.5;
        let last = k + 1 == cells.len();
        for s in 0..SEGS + last as usize {
            let t = s as f32 / SEGS as f32;
            let p = p0 * ((1.0 - t) * (1.0 - t)) + pi * (2.0 * (1.0 - t) * t) + p2 * (t * t);
            let level = if t < 0.5 { l0 + (li - l0) * t * 2.0 } else { li + (l2 - li) * (t * 2.0 - 1.0) };
            out.push((Vec3::new(p.x, surface(c, i, level) as f32, p.y), i));
        }
    }
    out
}

/// Everything the siting of a quay must keep clear of about one settlement: the plots,
/// lanes and tree ground of the settlements, the pads of the landmarks, what already
/// stands by the trails, and the jetties already built.
struct Clear<'a> {
    plots: Vec<(IVec2, IVec2)>,
    discs: Vec<(Vec2, f32)>,
    lanes: Vec<Vec2>,
    taken: &'a HashSet<IVec2>,
}

impl<'a> Clear<'a> {
    /// What lies within `reach` of `centre`.
    fn around(net: &Network, centre: Vec2, reach: f32, taken: &'a HashSet<IVec2>) -> Self {
        let near = |p: Vec2, r: f32| p.distance(centre) < reach + r;
        let mut discs: Vec<(Vec2, f32)> = net.pads.iter().map(|p| (vec2(p.x, p.z), p.radius)).collect();
        discs.extend(net.layouts.iter().filter_map(|l| l.tree).map(|(at, r)| (at, r + m(2.0))));
        discs.extend(net.markers.iter().map(|(_, at)| (vec2(at.x as f32, at.z as f32), m(1.6))));
        discs.extend(net.delights.iter().map(|d| (vec2(d.at.x as f32, d.at.z as f32), m(2.4))));
        discs.extend(net.lanterns.iter().map(|l| (l.at, m(1.6))));
        for l in &net.layouts {
            discs.extend(l.wells.iter().chain(&l.stalls).map(|w| (vec2(w.x as f32, w.z as f32), m(2.0))));
        }
        discs.retain(|&(at, r)| near(at, r));
        let plots = net
            .plots
            .iter()
            .filter(|p| near((p.lo + p.hi).as_vec2() * 0.5, (p.hi - p.lo).as_vec2().length()))
            .map(|p| (p.lo - IVec2::splat(2), p.hi + IVec2::splat(2)))
            .collect();
        let lanes = net.layouts.iter().flat_map(|l| l.lanes.iter().flatten().copied()).filter(|&p| near(p, m(2.0))).collect();
        Clear { plots, discs, lanes, taken }
    }

    /// Whether a jetty on `foot` columns keeps clear of everything.
    fn fits(&self, foot: &[IVec2]) -> bool {
        foot.iter().all(|&c| {
            let p = c.as_vec2() + 0.5;
            !self.taken.contains(&c)
                && !self.plots.iter().any(|&(lo, hi)| c.x >= lo.x && c.x <= hi.x && c.y >= lo.y && c.y <= hi.y)
                && !self.discs.iter().any(|&(at, r)| at.distance_squared(p) < r * r)
                && !self.lanes.iter().any(|l| l.distance_squared(p) < m(1.6) * m(1.6))
        })
    }
}

/// The columns about a jetty kept from any other, so that two never stand side by side.
fn margin_of(foot: &[IVec2]) -> impl Iterator<Item = IVec2> + '_ {
    foot.iter().flat_map(|&c| (-2..=2).flat_map(move |dz| (-2..=2).map(move |dx| c + IVec2::new(dx, dz))))
}

/// A jetty as it would stand: its columns, and the voxels to build it of.
struct Jetty {
    foot: Vec2,
    head: Vec2,
    level: i32,
    footprint: Vec<IVec2>,
    voxels: Vec<(IVec3, Voxel)>,
}

/// Where along `n` from `from` the water first gives out (the first dry column), as a
/// distance, looking no further than `reach`; None if it never does.
fn water_ends(s: &Sampler, from: Vec2, n: Vec2, reach: f32) -> Option<f32> {
    let mut d = 0.0;
    while d < reach {
        let p = from + n * d;
        if s.column(p.x.floor() as i32, p.y.floor() as i32).water_surface().is_none() {
            return Some(d);
        }
        d += 0.5;
    }
    None
}

/// A jetty from the bank `bank` (the first dry ground), running out along `n` for `long`
/// over water at surface `level`: a deck of boards three voxels wide on posts down to the
/// bed, a post rising at each corner of its end for the boats to tie to. None where the
/// bank stands too high over the water to step down onto it.
fn jetty(s: &Sampler, pal: &Palette, seed: u32, bank: Vec2, n: Vec2, long: f32, level: i32) -> Option<Jetty> {
    let across = vec2(-n.y, n.x);
    let col = |p: Vec2| s.column(p.x.floor() as i32, p.y.floor() as i32);
    if col(bank).top() > level + 1 {
        return None;
    }
    let mut rng = Rng::new(hash2(seed ^ 0x0a7e_77e5, bank.x as i32, bank.y as i32));
    let mut seen = HashSet::new();
    let mut voxels = Vec::new();
    let mut footprint = Vec::new();
    // Its boards, one row across at a time from the bank out, a dark board now and then.
    let mut u = -m(0.8);
    let mut row = 0;
    while u <= long {
        let board = if rng.unit() < 0.25 { mat::PLANK_DARK } else { pal.plank };
        let mut w = -1.5;
        while w <= 1.5 {
            let p = bank + n * u + across * w;
            let c = IVec2::new(p.x.floor() as i32, p.y.floor() as i32);
            if seen.insert(c) {
                let here = col(p);
                // On the bank the boards lie on the ground, and where it stands higher
                // than the deck they stop.
                if here.water_surface().is_some() || here.top() <= level {
                    footprint.push(c);
                    voxels.push((IVec3::new(c.x, level, c.y), board));
                    // Air over them, for any grass the bank grew.
                    voxels.push((IVec3::new(c.x, level + 1, c.y), AIR));
                }
            }
            w += 0.5;
        }
        u += 0.5;
        row += 1;
    }
    if row < 4 || footprint.is_empty() {
        return None;
    }
    // Posts down to the bed at the corners of the end and every two metres along the
    // sides, the end ones standing up past the deck.
    let mut posts = vec![long];
    let mut at = long - m(2.0);
    while at > 0.0 {
        posts.push(at);
        at -= m(2.0);
    }
    for (k, &u) in posts.iter().enumerate() {
        for w in [-1.5f32, 1.5] {
            let p = bank + n * u + across * w;
            let c = IVec2::new(p.x.floor() as i32, p.y.floor() as i32);
            let here = col(p);
            let rise = if k == 0 { 2 } else { 0 };
            for y in (here.top() + 1).min(level)..=level + rise {
                voxels.push((IVec3::new(c.x, y, c.y), pal.wood));
            }
        }
    }
    Some(Jetty { foot: bank, head: bank + n * long, level, footprint, voxels })
}

/// Builds the quays of every settlement that needs one, and works out the runs and the
/// fishing grounds. Called once the settlements are laid out and everything else by the
/// trails stands, before the village trees are found among what is left.
pub(crate) fn build(c: &Continent, seed: u32, net: &mut Network) {
    let mut ways = Waterways::default();
    let up = inflows(c);
    let mut taken = HashSet::new();
    let mut stamps = Vec::new();
    // The river quays first: each settlement with a partner has one on its river.
    let mut river_quay: Vec<Option<usize>> = vec![None; net.settlements.len()];
    for k in 0..net.settlements.len() {
        let s = &net.settlements[k];
        let (Some(r), false) = (s.river, s.partners.is_empty()) else { continue };
        if let Some((quay, voxels)) = river_quay_for(c, seed, net, &up, &taken, k, r) {
            taken.extend(margin_of(&quay.footprint));
            river_quay[k] = Some(ways.quays.len());
            ways.quays.push(quay);
            stamps.push(voxels);
        }
    }
    // Then the runs between them.
    for a in 0..net.settlements.len() {
        for &b in &net.settlements[a].partners {
            if b <= a {
                continue;
            }
            let (Some(qa), Some(qb)) = (river_quay[a], river_quay[b]) else { continue };
            if let Some(run) = run_between(c, &up, &ways.quays, (a, qa), (b, qb)) {
                ways.runs.push(run);
            }
        }
    }
    // The fishing quays: a settlement on the sea or a lake that fishes, or is a port.
    for k in 0..net.settlements.len() {
        let s = &net.settlements[k];
        let fishes = s.has(Livelihood::Fisher) || s.has(Livelihood::Port);
        if !fishes || !(s.coast || s.water == Water::Lake) {
            continue;
        }
        // A quay the settlement has on its river serves its fishing boats too, where
        // they can get out from it to open water.
        if let Some(q) = river_quay[k] {
            let grounds = grounds_for(c, seed, net, &ways.quays[q], q);
            if !grounds.is_empty() || ways.quays[q].landing.is_some() {
                ways.grounds.extend(grounds);
                continue;
            }
        }
        let quay = match s.port.and_then(|site| net.sites[site].mooring.map(|moor| (site, moor))) {
            Some((site, mooring)) => Some(Quay {
                settlement: k,
                serves: Serves::Fishing,
                berth: mooring.pos,
                heading: mooring.heading,
                landing: Some(site),
                foot: vec2(mooring.pos.x, mooring.pos.z),
                head: vec2(mooring.pos.x, mooring.pos.z),
                level: mooring.pos.y.round() as i32,
                footprint: Vec::new(),
                cell: 0,
            }),
            None => shore_quay_for(c, seed, net, &taken, k).map(|(quay, voxels)| {
                taken.extend(margin_of(&quay.footprint));
                stamps.push(voxels);
                quay
            }),
        };
        let Some(quay) = quay else { continue };
        let q = ways.quays.len();
        ways.grounds.extend(grounds_for(c, seed, net, &quay, q));
        ways.quays.push(quay);
    }
    for voxels in stamps {
        net.add_stamp_with(voxels, Vec::new());
    }
    net.waterways = ways;
}

/// The quay of settlement `k` on its river (whose cell it was settled by is `r`): on the
/// nearest stretch to its middle, a few cells up or down, where a jetty fits.
fn river_quay_for(c: &Continent, seed: u32, net: &Network, up: &[u32], taken: &HashSet<IVec2>, k: usize, r: usize) -> Option<(Quay, Vec<(IVec3, Voxel)>)> {
    let s = &net.settlements[k];
    // The stretch a few cells either way, as far as it runs calm.
    let mut cells = vec![r];
    for _ in 0..QUAY_CELLS {
        let first = cells[0];
        let u = up[first];
        if u == u32::MAX || !calm(c, u as usize) {
            break;
        }
        cells.insert(0, u as usize);
    }
    for _ in 0..QUAY_CELLS {
        let last = *cells.last().expect("a cell");
        let next = c.rcv[last] as usize;
        if next == last || c.ocean[next] || !calm(c, last) {
            break;
        }
        cells.push(next);
    }
    let line = middle(c, up, &cells);
    let centre = s.at();
    let reach = s.radius + QUAY_REACH;
    let clear = Clear::around(net, centre, reach + m(10.0), taken);
    let mut order: Vec<usize> = (1..line.len().saturating_sub(1)).filter(|&j| vec2(line[j].0.x, line[j].0.z).distance(centre) < reach).collect();
    order.sort_by(|&i, &j| {
        let d = |j: usize| vec2(line[j].0.x, line[j].0.z).distance_squared(centre);
        d(i).total_cmp(&d(j)).then(i.cmp(&j))
    });
    let span = (2.0 * reach + m(80.0)) as i32;
    let sampler = Sampler::with_network(c, seed, centre.x as i32 - span / 2, centre.y as i32 - span / 2, span, net);
    // A landing the settlement already has serves, where a boat can row from it to the
    // river's middle over nothing but water.
    if let Some((site, mooring)) = s.port.and_then(|site| net.sites[site].mooring.map(|moor| (site, moor))) {
        let at = vec2(mooring.pos.x, mooring.pos.z);
        let near = (1..line.len().saturating_sub(1)).min_by(|&i, &j| {
            let d = |j: usize| vec2(line[j].0.x, line[j].0.z).distance_squared(at);
            d(i).total_cmp(&d(j)).then(i.cmp(&j))
        });
        if let Some(j) = near {
            let (p, cell) = line[j];
            let to = vec2(p.x, p.z);
            let steps = (to.distance(at) / m(1.0)).ceil() as i32;
            let wet = to.distance(at) < m(60.0)
                && (0..=steps).all(|t| {
                    let q = at.lerp(to, t as f32 / steps.max(1) as f32);
                    sampler.column(q.x.floor() as i32, q.y.floor() as i32).water_surface().is_some()
                });
            if wet {
                let quay = Quay {
                    settlement: k,
                    serves: Serves::River,
                    berth: mooring.pos,
                    heading: mooring.heading,
                    landing: Some(site),
                    foot: at,
                    head: at,
                    level: mooring.pos.y.round() as i32,
                    footprint: Vec::new(),
                    cell,
                };
                return Some((quay, Vec::new()));
            }
        }
    }
    for j in order.into_iter().take(48) {
        let (p, cell) = line[j];
        let mid = vec2(p.x, p.z);
        let along = vec2(line[j + 1].0.x - line[j - 1].0.x, line[j + 1].0.z - line[j - 1].0.z).normalize_or_zero();
        let mut n = vec2(-along.y, along.x);
        if n.dot(centre - mid) < 0.0 {
            n = -n;
        }
        if n == Vec2::ZERO {
            continue;
        }
        // The middle must be water, the near bank close and the far one beyond it.
        if sampler.column(mid.x.floor() as i32, mid.y.floor() as i32).water_surface().is_none() {
            continue;
        }
        let (Some(near), Some(far)) = (water_ends(&sampler, mid, n, m(40.0)), water_ends(&sampler, mid, -n, m(60.0))) else { continue };
        let wide = near + far;
        let long = (wide * JETTY_SHARE).clamp(SHORTEST, LONGEST);
        if wide - long < OPEN_BEYOND + m(1.6) {
            continue;
        }
        let bank = mid + n * near;
        let level = p.y as i32;
        let pal = palette_at(c, seed, sampler.column(bank.x as i32, bank.y as i32).biome, bank.x as i32, level, bank.y as i32);
        let Some(j) = jetty(&sampler, &pal, s.seed, bank, -n, long, level) else { continue };
        if !clear.fits(&j.footprint) {
            continue;
        }
        // A boat lies off its end, along the bank, its bow upstream.
        let berth = j.head - n * m(1.6);
        let heading = (-along.y).atan2(-along.x);
        let quay = Quay {
            settlement: k,
            serves: Serves::River,
            berth: Vec3::new(berth.x, level as f32, berth.y),
            heading,
            landing: None,
            foot: j.foot,
            head: j.head,
            level: j.level,
            footprint: j.footprint,
            cell,
        };
        return Some((quay, j.voxels));
    }
    None
}

/// The run of the river between two settlements' quays, `(settlement, quay)` each: down
/// the river's middle from the upstream one to the other. None where they are on no one
/// calm stretch, or so close a boat is no use.
fn run_between(c: &Continent, up: &[u32], quays: &[Quay], a: (usize, usize), b: (usize, usize)) -> Option<Run> {
    let (qa, qb) = (&quays[a.1], &quays[b.1]);
    let ((a, qa_i, qa), (b, qb_i, qb), cells) = match down_to(c, qa.cell, qb.cell) {
        Some(cells) => ((a.0, a.1, qa), (b.0, b.1, qb), cells),
        None => ((b.0, b.1, qb), (a.0, a.1, qa), down_to(c, qb.cell, qa.cell)?),
    };
    let mid = middle(c, up, &cells);
    let flat = |p: Vec3| vec2(p.x, p.z);
    let nearest = |q: &Quay| (0..mid.len()).min_by(|&i, &j| flat(mid[i].0).distance_squared(q.head).total_cmp(&flat(mid[j].0).distance_squared(q.head))).unwrap_or(0);
    let (from, to) = (nearest(qa), nearest(qb));
    if to <= from + 2 {
        return None;
    }
    let mut line = vec![qa.berth];
    line.extend(mid[from..=to].iter().map(|p| p.0));
    line.push(qb.berth);
    let along = measure(&line);
    if along[along.len() - 1] < SHORTEST_RUN {
        return None;
    }
    Some(Run { a, b, from: qa_i, to: qb_i, line, along, cells })
}

/// The quay of fishing settlement `k` on the shore of the sea or its lake: on the nearest
/// shore to its middle where open water runs out beyond a jetty.
fn shore_quay_for(c: &Continent, seed: u32, net: &Network, taken: &HashSet<IVec2>, k: usize) -> Option<(Quay, Vec<(IVec3, Voxel)>)> {
    let s = &net.settlements[k];
    let centre = s.at();
    let reach = s.radius + QUAY_REACH;
    let clear = Clear::around(net, centre, reach + m(10.0), taken);
    let span = (2.0 * reach + m(120.0)) as i32;
    let sampler = Sampler::with_network(c, seed, centre.x as i32 - span / 2, centre.y as i32 - span / 2, span, net);
    let col = |p: Vec2| sampler.column(p.x.floor() as i32, p.y.floor() as i32);
    // Where the water begins along each of a ring of ways out from the middle.
    let mut shores: Vec<(f32, Vec2, Vec2)> = Vec::new();
    for a in 0..48 {
        let angle = a as f32 / 48.0 * std::f32::consts::TAU;
        let n = vec2(angle.cos(), angle.sin());
        let mut d = s.kind.green();
        let mut dry = None;
        while d < reach {
            let p = centre + n * d;
            let here = col(p);
            // Open water: the sea or a lake, not a river's channel.
            if here.water_surface().is_some() && here.river >= 0.0 {
                if let Some(bank) = dry {
                    shores.push((d, bank, n));
                }
                break;
            }
            if here.water_surface().is_none() {
                dry = Some(p);
            }
            d += 1.0;
        }
    }
    shores.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (_, bank, n) in shores.into_iter().take(24) {
        // Open water straight out, well beyond the jetty's end.
        let water = bank + n * m(1.2);
        let Some(level) = col(water).water_surface() else { continue };
        let long = LONGEST;
        if (1..=24).any(|k| col(bank + n * (m(1.0) + k as f32 * m(1.25))).water_surface().is_none()) {
            continue;
        }
        let pal = palette_at(c, seed, col(bank).biome, bank.x as i32, level, bank.y as i32);
        let Some(j) = jetty(&sampler, &pal, s.seed, bank, n, long, level) else { continue };
        if !clear.fits(&j.footprint) {
            continue;
        }
        let across = vec2(-n.y, n.x);
        let berth = j.head - n * m(1.6) + across * m(1.5);
        let quay = Quay {
            settlement: k,
            serves: Serves::Fishing,
            berth: Vec3::new(berth.x, level as f32, berth.y),
            heading: n.y.atan2(n.x),
            landing: None,
            foot: j.foot,
            head: j.head,
            level,
            footprint: j.footprint,
            cell: 0,
        };
        return Some((quay, j.voxels));
    }
    None
}

/// The fishing grounds off a quay: a few ways out over open water, fanned about the way
/// the quay faces, each as far out as the water runs straight, up to a few hundred metres.
fn grounds_for(c: &Continent, seed: u32, net: &Network, quay: &Quay, q: usize) -> Vec<Ground> {
    let s = &net.settlements[quay.settlement];
    let from = vec2(quay.berth.x, quay.berth.z);
    let span = (2.0 * GROUND_FAR + m(20.0)) as i32;
    let sampler = Sampler::with_network(c, seed, from.x as i32 - span / 2, from.y as i32 - span / 2, span, net);
    let boats = if s.has(Livelihood::Port) { 3 } else if s.kind == super::Kind::Hamlet { 1 } else { 2 };
    let mut out = Vec::new();
    for b in 0..boats {
        // Out the way the jetty points, or near it; where that soon meets land, the boat
        // goes some other way, swinging wider each time, toward the open water.
        for t in 0..8 {
            let h = hash2(s.seed ^ 0x0f15_4e55, b * 8 + t, q as i32);
            let swing = (unit(h) - 0.5) * (1.6 + 0.6 * t as f32);
            let a = quay.heading + swing;
            let n = vec2(a.cos(), a.sin());
            let want = GROUND_NEAR + (GROUND_FAR - GROUND_NEAR) * (0.35 + 0.65 * unit(h.rotate_left(11)));
            let mut d = m(1.6);
            let mut level = None;
            while d < want {
                let p = from + n * d;
                match sampler.column(p.x.floor() as i32, p.y.floor() as i32).water_surface() {
                    Some(k) => level = Some(k),
                    None => break,
                }
                d += m(1.6);
            }
            // Stopped well short of any land it met, so that it drifts about clear of it.
            let d = d - GROUND_SHORT;
            let p = from + n * d;
            let open = (0..12).all(|r| {
                let o = p + Vec2::from_angle(r as f32 / 12.0 * std::f32::consts::TAU) * (DRIFT + m(3.0));
                sampler.column(o.x.floor() as i32, o.y.floor() as i32).water_surface().is_some()
            });
            if let Some(k) = level
                && d >= GROUND_NEAR
                && open
            {
                out.push(Ground { quay: q, at: Vec3::new(p.x, k as f32, p.y) });
                break;
            }
        }
    }
    out
}
