//! Boardwalks where the homes stand on stilts (#203): in the marsh and the mangroves,
//! and on a rainforest's coast. From the foot of each raised home's steps a boardwalk
//! runs out to the lane it faces, others run from door to door between neighbours, and
//! where a lane would cross water it crosses on one. Each is laid in straight runs along
//! x and z ([`public::boardwalk`]), three columns across, turning square where it must.
//!
//! The boards lie one layer up: on the ground where it is dry, and over the water just
//! above its surface, on posts driven into the bed, with a rope along the edges there.
//! Where the ground under them rises, so do they, a step at a time, so a walk across a
//! bank and out over the water stands clear of both. They are solid voxels, so the
//! player walks on them; the villagers walk them too, standing on the boards wherever
//! one runs ([`Layout::walk_height`]), and every walk is among the lanes they walk.
//!
//! A crossing is planned as its lane is laid, before any plot, so the plots keep off it;
//! it and the walks to the doors are built once every plot is down, each only over
//! ground it can stand on, clear of every building, garden and thing set out.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use glam::{IVec2, IVec3, Vec2};
use voxel::{
    CHUNK, Voxel,
    mesh::{CELLS, PlantBox},
};

use super::{KEEP, Layout, PLOT, Point, THING, Town, WAY};
use crate::{
    scale::m,
    settlements::{Age, Region, kit::components::public},
    surface::{Column, Sampler},
    trails::{Network, TrailPoint, build::is_wet},
};

/// How many columns across a boardwalk is.
const WIDE: i32 = 3;
/// The deepest water a boardwalk is laid over, from its surface to the bed, in voxels:
/// as deep as its posts are driven.
const DEEPEST: i32 = 5;
/// The longest stretch of water a lane crosses on a boardwalk, in its points (two voxels
/// apart): about thirty metres.
pub(super) const LONGEST: usize = 38;
/// The furthest apart two doors' steps are for a boardwalk to join them, in voxels.
const LINK: i32 = 26;
/// How far a crossing runs along before it steps across, in voxels.
const STEP: i32 = 8;
/// The most the boards rise or fall from one column to the next, in layers: a step.
const RISE: i32 = 1;
/// How many points of the lanes a walk from a home's steps tries for, the nearest first,
/// where it cannot reach the one its path goes to, and how far off they may be, in voxels.
const SPUR_TRIES: usize = 12;
const SPUR_REACH: i32 = 48;

/// A settlement's boardwalks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Boardwalks {
    /// Each straight run, from the column at the middle of one end to the column at the
    /// middle of the other, [`WIDE`] columns across.
    pub runs: Vec<(IVec2, IVec2)>,
    /// Every column under the boards, with the layer they lie in, by z and then x.
    decks: Vec<(IVec2, i32)>,
}

impl Boardwalks {
    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    /// The layer the boards lie in over continent-local column `c`, if any do.
    pub fn deck_at(&self, c: IVec2) -> Option<i32> {
        self.decks.binary_search_by(|(q, _)| (q.y, q.x).cmp(&(c.y, c.x))).ok().map(|i| self.decks[i].1)
    }

    /// Every column under the boards, with the layer they lie in.
    pub fn decks(&self) -> &[(IVec2, i32)] {
        &self.decks
    }
}

impl Layout {
    /// Where someone stands over continent-local `p`, `ground` being where they would
    /// stand on the ground or the water there: on the boards where a boardwalk runs.
    pub fn walk_height(&self, p: Vec2, ground: f32) -> f32 {
        match self.boardwalks.deck_at(p.floor().as_ivec2()) {
            Some(d) => ground.max(d as f32 + 1.0),
            None => ground,
        }
    }
}

/// The layer a boardwalk's boards lie in over a column taken alone: on the ground, or
/// just above the surface of water; None over water deeper than its posts reach.
fn surface(col: &Column) -> Option<i32> {
    match col.water_surface() {
        Some(k) if k - 1 - col.top() > DEEPEST => None,
        Some(k) => Some(k),
        None => Some(col.top() + 1),
    }
}

/// The corners of a walk from column `a` to column `b` in straight runs along x and z,
/// keeping near the line between them: along the way it mostly runs at most `step`
/// columns at a time, then across.
fn corners(a: IVec2, b: IVec2, step: i32) -> Vec<IVec2> {
    let d = b - a;
    let along_x = d.x.abs() >= d.y.abs();
    let n = (d.x.abs().max(d.y.abs()) + step - 1) / step;
    let mut out = vec![a];
    let mut at = a;
    for i in 1..=n.max(1) {
        let p = a + (d.as_vec2() * i as f32 / n.max(1) as f32).round().as_ivec2();
        let turn = if along_x { IVec2::new(p.x, at.y) } else { IVec2::new(at.x, p.y) };
        out.extend([turn, p]);
        at = p;
    }
    tidy(out)
}

/// `pts` without repeats, and without the points in the middle of a straight run.
fn tidy(pts: Vec<IVec2>) -> Vec<IVec2> {
    let mut out: Vec<IVec2> = Vec::new();
    for p in pts {
        if out.last() == Some(&p) {
            continue;
        }
        if out.len() >= 2 {
            let (a, b) = (out[out.len() - 2], out[out.len() - 1]);
            if (a.x == b.x && b.x == p.x) || (a.y == b.y && b.y == p.y) {
                out.pop();
            }
        }
        out.push(p);
    }
    out
}

/// Every column a walk through `corners` covers, [`WIDE`] across, in order.
fn columns(corners: &[IVec2]) -> Vec<IVec2> {
    let mut out = BTreeSet::new();
    for w in corners.windows(2) {
        let (a, b) = (w[0], w[1]);
        let along_x = (b.x - a.x).abs() >= (b.y - a.y).abs();
        let (lo, hi) = (a.min(b), a.max(b));
        for s in if along_x { lo.x..=hi.x } else { lo.y..=hi.y } {
            for k in -(WIDE - 1) / 2..=WIDE / 2 {
                let c = if along_x { IVec2::new(s, a.y + k) } else { IVec2::new(a.x + k, s) };
                out.insert((c.y, c.x));
            }
        }
    }
    out.into_iter().map(|(z, x)| IVec2::new(x, z)).collect()
}

/// The ways a walk might go from `foot`, at the foot of a home's steps facing `out`, to a
/// lane at `to`: straight out and then along; out a little, across, and on; or out a
/// little and then stepping along the line to it.
fn spurs(foot: IVec2, out: IVec2, to: IVec2) -> Vec<Vec<IVec2>> {
    let (along, stub) = ((to - foot).dot(out), foot + out * 2);
    let mut tries = Vec::new();
    if along >= 2 {
        tries.push(tidy(vec![foot, foot + out * along, to]));
    }
    tries.push(tidy(vec![foot, stub, stub + (to - stub) * IVec2::new(out.y.abs(), out.x.abs()), to]));
    let mut stairs = vec![foot];
    stairs.extend(corners(stub, to, STEP / 2));
    tries.push(tidy(stairs));
    tries
}

/// Whether boards over `cols` (as [`columns`] gives them), each in the layer `level`
/// gives, never rise or fall more than [`RISE`] from one column to the next.
fn smooth(cols: &[IVec2], level: &mut dyn FnMut(IVec2) -> Option<i32>) -> bool {
    let has = |c: IVec2| cols.binary_search_by(|q| (q.y, q.x).cmp(&(c.y, c.x))).is_ok();
    cols.iter().all(|&c| {
        [IVec2::X, IVec2::Y].into_iter().all(|o| !has(c + o) || matches!((level(c), level(c + o)), (Some(a), Some(b)) if (a - b).abs() <= RISE))
    })
}

/// Points along the middle of a walk through `corners`, about two voxels apart, at the
/// middles of columns: the lane the villagers walk on it.
fn path(corners: &[IVec2]) -> Vec<Vec2> {
    let mut out: Vec<Vec2> = Vec::new();
    for w in corners.windows(2) {
        let (a, b) = (w[0].as_vec2() + 0.5, w[1].as_vec2() + 0.5);
        let n = (a.distance(b) / 2.0).ceil().max(1.0) as usize;
        for i in 0..n {
            out.push(a.lerp(b, i as f32 / n as f32));
        }
    }
    out.extend(corners.last().map(|c| c.as_vec2() + 0.5));
    out
}

impl Town<'_> {
    /// Whether its homes stand on stilts, joined by boardwalks: in the marsh and the
    /// mangroves, and on a rainforest's coast.
    pub(super) fn stilted(&self) -> bool {
        self.s.region == Region::Wetland || (self.s.region == Region::Rainforest && self.s.coast)
    }

    /// Whether a lane's point `p` over `col` could cross on a boardwalk rather than be
    /// broken off: it is kept from there only by the water, which is shallow enough.
    pub(super) fn can_cross(&self, p: Vec2, col: &Column) -> bool {
        (is_wet(col) || col.river < m(2.4))
            && surface(col).is_some()
            && p.distance(self.centre) <= self.s.radius - 2.0
            && self.discs.iter().all(|d| d.at == self.centre || d.at.distance(p) >= d.r + 2.0)
    }

    /// Plans the boardwalk carrying a lane over the water from its point `a` on one bank
    /// to `b` on the other, and keeps its ground from the plots; it is built with the rest
    /// (see [`Self::boardwalks`]). Whether it could be.
    pub(super) fn plan_crossing(&mut self, a: Vec2, b: Vec2) -> bool {
        let corners = corners(a.floor().as_ivec2(), b.floor().as_ivec2(), STEP);
        let cols = columns(&corners);
        let within = |c: &IVec2| (c.as_vec2() + 0.5).distance(self.centre) <= self.s.radius - 1.0;
        if !cols.iter().all(|c| within(c) && surface(&self.sam.column(c.x, c.y)).is_some()) || self.grid.any(&cols, PLOT | KEEP | THING) {
            return false;
        }
        let path = path(&corners);
        for p in &path {
            self.worn.add(Point { at: *p, hw: m(0.6), y: 0.0, door: true });
        }
        for &c in &cols {
            self.grid.mark(c, WAY);
        }
        self.layout.lanes.push(path);
        self.crossings.push(corners);
        true
    }

    /// Builds the boardwalks of a place whose homes stand on stilts: the crossings planned
    /// with its lanes, a walk from the foot of each raised home's steps out to its lane,
    /// and walks between neighbours' steps; each recorded in the layout, and each walk to
    /// a door among its lanes.
    pub(super) fn boardwalks(&mut self, net: &mut Network, fin: &Sampler) {
        if !self.stilted() {
            return;
        }
        // The layer of each column's boards, as asked for: a layer over the highest ground
        // about it, so they stand clear of a bank beside them as of the water; None where
        // they cannot stand.
        let mut ground: HashMap<IVec2, Option<i32>> = HashMap::new();
        let mut known: HashMap<IVec2, Option<i32>> = HashMap::new();
        let mut level = |c: IVec2| -> Option<i32> {
            *known.entry(c).or_insert_with(|| {
                let mut surface_at = |c: IVec2| *ground.entry(c).or_insert_with(|| surface(&fin.column(c.x, c.y)));
                let own = surface_at(c)?;
                Some((-1..=1).flat_map(|dz| (-1..=1).map(move |dx| IVec2::new(dx, dz))).filter_map(|d| surface_at(c + d)).fold(own, i32::max))
            })
        };
        let mut walks: Vec<Vec<IVec2>> = std::mem::take(&mut self.crossings);
        let crossings = walks.len();
        // Where a walk may go now every plot is down: within the place, over ground the
        // boards can stand on, clear of every building, garden and thing set out, and never
        // more than a step up or down from one column to the next.
        let buildings: Vec<(IVec2, IVec2)> = self.plots.iter().flat_map(|p| std::iter::once((p.lo, p.hi)).chain(p.garden)).collect();
        let free = |town: &Town, walk: &[IVec2], level: &mut dyn FnMut(IVec2) -> Option<i32>| {
            let cols = columns(walk);
            let clear = cols.iter().all(|&c| {
                (c.as_vec2() + 0.5).distance(town.centre) <= town.s.radius - 1.0
                    && !buildings.iter().any(|&(lo, hi)| c.cmpge(lo).all() && c.cmple(hi).all())
                    && town.grid.get(c) & (THING | KEEP) == 0
                    && level(c).is_some()
            });
            clear && smooth(&cols, level)
        };
        // From the foot of each raised home's steps out to the point of the lane its path
        // goes to (see `spurs`), or where none of those ways is clear, or that point is on
        // a stretch of the lane never laid, to the nearest point of a lane one is clear to.
        let feet: Vec<(IVec2, IVec2, IVec2)> = self
            .door_ways
            .iter()
            .filter(|w| self.plots[w.0].door.y - self.plots[w.0].origin.y >= 2)
            .map(|&(_, foot, out, to)| (foot, out, to.floor().as_ivec2()))
            .collect();
        let lanes: Vec<IVec2> = self.layout.lanes.iter().flatten().map(|p| p.floor().as_ivec2()).collect();
        for &(foot, out, to) in &feet {
            let stub = foot + out * 2;
            let laid = lanes.iter().any(|&p| (p - to).abs().max_element() <= 1);
            let mut near: Vec<IVec2> = lanes.iter().copied().filter(|&p| p != to && (p - stub).abs().max_element() <= SPUR_REACH).collect();
            near.sort_by_key(|&p| ((p - stub).length_squared(), p.x, p.y));
            near.dedup();
            let targets = laid.then_some(to).into_iter().chain(near.into_iter().take(SPUR_TRIES));
            if let Some(walk) = targets.flat_map(|to| spurs(foot, out, to)).find(|w| w.len() >= 2 && free(self, w, &mut level)) {
                walks.push(walk);
            }
        }
        // From door to door, each to its nearest neighbour not yet joined to it.
        let mut joined: BTreeSet<(usize, usize)> = BTreeSet::new();
        for (i, &(a, oa, _)) in feet.iter().enumerate() {
            let Some(j) = (0..feet.len()).filter(|&j| j != i && (feet[j].0 - a).abs().max_element() <= LINK).min_by_key(|&j| ((feet[j].0 - a).length_squared(), j)) else {
                continue;
            };
            if !joined.insert((i.min(j), i.max(j))) {
                continue;
            }
            let (b, ob) = (feet[j].0, feet[j].1);
            let (sa, sb) = (a + oa * 2, b + ob * 2);
            let tries = [tidy(vec![a, sa, IVec2::new(sb.x, sa.y), sb, b]), tidy(vec![a, sa, IVec2::new(sa.x, sb.y), sb, b])];
            if let Some(walk) = tries.into_iter().find(|w| free(self, w, &mut level)) {
                walks.push(walk);
            }
        }
        let age = match self.s.age {
            Age::Young => 0.2,
            Age::Established => 0.45,
            Age::Ancient => 0.75,
        };
        let mut voxels: BTreeMap<(i32, i32, i32), Voxel> = BTreeMap::new();
        let mut fine: Vec<PlantBox> = Vec::new();
        let mut decks: BTreeMap<(i32, i32), i32> = BTreeMap::new();
        for (n, walk) in walks.into_iter().enumerate() {
            // A crossing was planned on the ground as it was before the plots: built only if
            // its boards can still stand all the way, a step at a time.
            let cols = columns(&walk);
            let Some(levels) = cols.iter().map(|&c| level(c)).collect::<Option<Vec<i32>>>() else { continue };
            if !smooth(&cols, &mut level) {
                continue;
            }
            let levels: HashMap<IVec2, i32> = cols.iter().copied().zip(levels).collect();
            let bed = |x: i32, z: i32| fin.column(x, z).top();
            let wet = |x: i32, z: i32| fin.column(x, z).water_surface().is_some();
            for w in walk.windows(2) {
                let (v, f) = public::boardwalk(w[0], w[1], WIDE, |x, z| levels[&IVec2::new(x, z)], bed, wet, &self.pal, age);
                for (q, v) in v {
                    voxels.entry((q.x, q.y, q.z)).or_insert(v);
                }
                fine.extend(f);
                self.layout.boardwalks.runs.push((w[0], w[1]));
            }
            for (&c, &d) in &levels {
                decks.insert((c.y, c.x), d);
                self.grid.mark(c, THING);
            }
            // Trees and bushes keep off it, as off a bridge: points down its middle that
            // wear nothing, kept as far back from a building as a bridge's (the path from
            // the door keeps them off the last few steps).
            let lane = path(&walk);
            let off = |p: Vec2| self.plots.iter().all(|q| p.clamp(q.lo.as_vec2(), (q.hi + 1).as_vec2()).distance(p) >= m(1.2));
            let points: Vec<TrailPoint> =
                lane.iter().filter(|&&p| off(p)).map(|p| TrailPoint { x: p.x, z: p.y, y: levels[&p.floor().as_ivec2()] as f32 + 1.0, half_width: 0.0 }).collect();
            net.add_points(&points);
            // A crossing's lane is among the lanes already.
            if n >= crossings {
                self.layout.lanes.push(lane);
            }
        }
        self.layout.boardwalks.decks = decks.into_iter().map(|((z, x), d)| (IVec2::new(x, z), d)).collect();
        // A stamp for each chunk, each part in the chunk of its first corner, every post
        // top shared by two runs kept once.
        let mut by_chunk: BTreeMap<(i32, i32), (Vec<(IVec3, Voxel)>, Vec<PlantBox>)> = BTreeMap::new();
        for ((x, y, z), v) in voxels {
            by_chunk.entry((x.div_euclid(CHUNK), z.div_euclid(CHUNK))).or_default().0.push((IVec3::new(x, y, z), v));
        }
        let mut seen = BTreeSet::new();
        for b in fine {
            if seen.insert((b.lo, b.hi)) {
                let at = IVec2::new(b.lo[0], b.lo[2]).div_euclid(IVec2::splat(CELLS * CHUNK));
                by_chunk.entry((at.x, at.y)).or_default().1.push(b);
            }
        }
        for (_, (voxels, fine)) in by_chunk {
            net.add_stamp_with(voxels, fine);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, HashSet, VecDeque},
        sync::{Arc, OnceLock},
    };

    use voxel::AIR;

    use super::*;
    use crate::{
        ColumnOutput,
        continent::{Continent, SimConfig},
        macro_layout::MACRO_SIZE,
        world::GameWorld,
    };

    /// Continents of small worlds with settlements of the marsh: a village of nine raised
    /// homes, and a town of nineteen; the world's seed and the continent.
    const PLACES: [(u32, i32, i32); 2] = [(3, 0, 0), (2, 0, -1)];

    fn places() -> &'static Vec<(GameWorld, Arc<Continent>, IVec3)> {
        static P: OnceLock<Vec<(GameWorld, Arc<Continent>, IVec3)>> = OnceLock::new();
        P.get_or_init(|| {
            PLACES
                .iter()
                .map(|&(seed, mx, mz)| {
                    let w = GameWorld::with_config(seed, SimConfig { size: 256 });
                    let c = w.continent(mx, mz);
                    (w, c, IVec3::new(mx * MACRO_SIZE, 0, mz * MACRO_SIZE))
                })
                .collect()
        })
    }

    /// The voxel at `q`, in the world, of the chunk columns made so far, making its own
    /// if it is not among them.
    fn voxel_at(w: &GameWorld, made: &mut HashMap<(i32, i32), ColumnOutput>, q: IVec3) -> Option<Voxel> {
        let key = (q.x.div_euclid(CHUNK), q.z.div_euclid(CHUNK));
        let column = made.entry(key).or_insert_with(|| w.generate_column_bare(key.0, key.1));
        let (cy, ly) = (q.y.div_euclid(CHUNK), q.y.rem_euclid(CHUNK));
        column.chunks.iter().find(|(k, _)| *k == cy).map(|(_, ch)| ch.get((q.x - key.0 * CHUNK + 1) as usize, ly as usize + 1, (q.z - key.1 * CHUNK + 1) as usize))
    }

    /// In every settlement whose homes stand on stilts, a boardwalk runs from the foot of
    /// every raised home's steps to a lane, its boards over the water and the ground both,
    /// in the world where they were laid with room over them to walk, never more than a
    /// step up or down from one column to the next; and nowhere else has one.
    #[test]
    fn boardwalks_join_every_stilt_home_to_the_lanes_clear_of_the_water() {
        let mut homes = 0;
        for (w, c, o) in places() {
            let seed = w.seed();
            let net = &c.trails;
            let mut made = HashMap::new();
            for (k, s) in net.settlements.iter().enumerate() {
                let l = &net.layouts[k];
                let stilted = s.region == Region::Wetland || (s.region == Region::Rainforest && s.coast);
                if !stilted {
                    assert!(l.boardwalks.is_empty(), "seed {seed}: a {:?} {:?} has boardwalks", s.region, s.kind);
                    continue;
                }
                let at = format!("seed {seed}: the {:?} at {},{}", s.kind, s.x + o.x, s.z + o.z);
                let decks: HashMap<IVec2, i32> = l.boardwalks.decks().iter().copied().collect();
                // The lanes off the boards, which the walks come out to.
                let lanes: Vec<Vec2> = l.lanes.iter().flatten().copied().filter(|p| !decks.contains_key(&p.floor().as_ivec2())).collect();
                for p in &net.plots[l.plots.clone()] {
                    if p.door.y - p.origin.y < 2 {
                        continue;
                    }
                    homes += 1;
                    // The boards at the foot of its steps, and every board joined to them.
                    // Its steps come down out of its plot straight out from the door.
                    let door = IVec2::new(p.door.x, p.door.z);
                    let foot = (1..64).map(|d| door + p.facing * d).find(|&c| !(c.cmpge(p.lo).all() && c.cmple(p.hi).all())).unwrap();
                    let start: Vec<IVec2> = (-2..=2).flat_map(|dz| (-2..=2).map(move |dx| foot + IVec2::new(dx, dz))).filter(|c| decks.contains_key(c)).collect();
                    assert!(!start.is_empty(), "{at}: no boards at the foot of the {}'s steps at {}", p.purpose.word(), p.door);
                    let mut seen: HashSet<IVec2> = start.iter().copied().collect();
                    let mut todo: VecDeque<IVec2> = start.into_iter().collect();
                    let mut out = false;
                    while let Some(q) = todo.pop_front() {
                        if lanes.iter().any(|l| l.distance(q.as_vec2() + 0.5) < 2.5) {
                            out = true;
                            break;
                        }
                        for d in [IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y] {
                            if decks.contains_key(&(q + d)) && seen.insert(q + d) {
                                todo.push_back(q + d);
                            }
                        }
                    }
                    assert!(out, "{at}: the boards from the {} at {} come out to no lane", p.purpose.word(), p.door);
                }
                // Over the water and clear of the ground, in the world, with room to walk.
                let lo = decks.keys().fold(IVec2::MAX, |a, &b| a.min(b));
                let hi = decks.keys().fold(IVec2::MIN, |a, &b| a.max(b));
                if decks.is_empty() {
                    continue;
                }
                let sam = Sampler::with_network(c, seed, lo.x - 1, lo.y - 1, (hi - lo).max_element() + 3, net);
                let mut wet = 0;
                for (&q, &d) in &decks {
                    let col = sam.column(q.x, q.y);
                    match col.water_surface() {
                        Some(top) => {
                            wet += 1;
                            assert!(d >= top, "{at}: the boards at {q} lie in the water");
                        }
                        None => assert!(d > col.top(), "{at}: the boards at {q} lie in the ground"),
                    }
                    for n in [q + IVec2::X, q + IVec2::Y] {
                        if let Some(&e) = decks.get(&n) {
                            assert!((d - e).abs() <= 2, "{at}: the boards drop {} layers from {q} to {n}", (d - e).abs());
                        }
                    }
                    let v = IVec3::new(q.x, d, q.y) + *o;
                    assert!(voxel_at(w, &mut made, v).is_some_and(|v| v != AIR), "{at}: no board in the world at {q}");
                    for up in 1..=2 {
                        assert_eq!(voxel_at(w, &mut made, v + IVec3::Y * up), Some(AIR), "{at}: no room over the boards at {q}, {up} up");
                    }
                }
                println!("{at}: {} runs over {} columns, {wet} of them over water", l.boardwalks.runs.len(), decks.len());
            }
        }
        assert!(homes >= 9, "only {homes} homes on stilts");
    }

    /// The boardwalks come out the same every time, and agree across the borders of the
    /// chunks they cross.
    #[test]
    fn boardwalks_are_the_same_every_time_and_agree_across_chunk_borders() {
        let (w, c, o) = &places()[0];
        let again = GameWorld::with_config(PLACES[0].0, SimConfig { size: 256 }).continent(PLACES[0].1, PLACES[0].2);
        let walks = |c: &Continent| c.trails.layouts.iter().map(|l| l.boardwalks.clone()).collect::<Vec<_>>();
        assert!(walks(c) == walks(&again), "boardwalks laid differently");
        let mut compared = 0;
        for l in &c.trails.layouts {
            for &(q, d) in l.boardwalks.decks() {
                let v = IVec3::new(q.x, d, q.y) + *o;
                // Boards at the east edge of a chunk, with the next chunk east of them.
                if v.x.rem_euclid(CHUNK) != CHUNK - 1 || compared >= 4 {
                    continue;
                }
                let (cx, cz) = (v.x.div_euclid(CHUNK), v.z.div_euclid(CHUNK));
                let (a, b) = (w.generate_column_bare(cx, cz).chunks, w.generate_column_bare(cx + 1, cz).chunks);
                for (cy, ca) in &a {
                    let Some((_, cb)) = b.iter().find(|e| e.0 == *cy) else { continue };
                    for (i, j) in [(voxel::PAD - 2, 0), (voxel::PAD - 1, 1)] {
                        for y in 0..voxel::PAD {
                            for z in 0..voxel::PAD {
                                assert_eq!(ca.get(i, y, z), cb.get(j, y, z), "chunks {cx},{cz} and the next east differ at {i} y {y} z {z} cy {cy}");
                            }
                        }
                    }
                }
                compared += 1;
            }
        }
        assert!(compared > 0, "no boardwalk crosses a chunk border");
    }

    #[test]
    fn walks_turn_square_and_keep_near_their_line() {
        let (a, b) = (IVec2::new(0, 0), IVec2::new(20, 13));
        let c = corners(a, b, STEP);
        assert_eq!((c[0], *c.last().unwrap()), (a, b));
        for w in c.windows(2) {
            assert!(w[0].x == w[1].x || w[0].y == w[1].y, "{c:?}");
        }
        // No corner strays more than a step from the line.
        let d = (b - a).as_vec2().normalize();
        for p in &c {
            let off = (p.as_vec2() - a.as_vec2()).perp_dot(d).abs();
            assert!(off <= STEP as f32, "{p} off by {off}");
        }
        // A straight walk is one run.
        assert_eq!(corners(IVec2::new(3, 5), IVec2::new(3, 30), STEP), vec![IVec2::new(3, 5), IVec2::new(3, 30)]);
        let cols = columns(&[IVec2::new(0, 0), IVec2::new(4, 0)]);
        assert_eq!(cols.len(), 5 * WIDE as usize);
    }
}
