//! The gates on a town's main ways in, and the old wall of an ancient town (#203).
//!
//! A town's gate stands where its main way in (every way in, in a great town) comes in
//! a little inside its edge: always open, lit at night, in its country's way, with a
//! short run of hedge, fence or wall leading off either side of it and dying away, for
//! the gate marks arriving and encloses nothing. An ancient town has none of these but
//! the wall it was founded inside, smaller than the town is now: stone, worn, fallen in
//! stretches, grown with ivy, broken wherever a lane or a later building came through,
//! and a stone gatehouse where each main way passes it, which the town has in places
//! grown out past.
//!
//! A gate stands square to the world, so its way must run along a row or a column where
//! it stands. Where a way in comes slantwise all about the ring, a stretch of it near
//! the ring is laid anew: swinging gently off its old line onto a straight that runs
//! square through where the gate will be, and back. Where even that cannot be done
//! (water, a bridge, another way joining), the gate goes wherever further in the way
//! runs square of itself.
//!
//! The ground is kept before any plot is chosen ([`Town::keep_for_gates`]); the gates
//! and the wall are built once every plot and path is down ([`Town::raise_gates`],
//! [`Town::raise_wall`]), the wall routed between whatever stands on its line.

use std::ops::Range;

use glam::{IVec2, IVec3, Vec2};
use voxel::{AIR, Voxel, mesh::PlantBox};

use super::{Disc, GREEN, Grid, KEEP, PLOT, Point, THING, Town, WAY, WIDEST, Way, Worn, edges::Gate, turn_rect, world};
use crate::{
    features::{designs::Special, parts, shape::rotate},
    materials as mat,
    noise::{hash2, perlin2, unit},
    scale::m,
    settlements::{
        Age, Kind, Region,
        kit::Building,
        special::{self, Built, Facts, SpecialSpec, gate, wall},
    },
    surface::Sampler,
    trails::{
        Network, TrailPoint,
        build::{is_wet, profile, sample, turns_toward},
    },
};

/// How far out past the ring a gate wanting to stand on it may be, and a third as far
/// in: the way is followed until it runs straight along a row or column of the world,
/// on dry ground with nothing else on it.
const BAND: f32 = 12.0;
/// The band a gate is looked for in first, out from its ring and in.
const NEAR_BAND: (f32, f32) = (-3.0 * BAND, BAND);
/// And where none stands in that, the further band a way may be laid anew to pass
/// square through one (see [`Town::squared_site`]): a little further out, or anywhere
/// in to the green, the nearest the ring the best. A way in over a marsh or an
/// estuary, or slantwise across a slope, may have nowhere nearer.
const WIDE_BAND: (f32, f32) = (f32::NEG_INFINITY, 2.0 * BAND);
/// How far beyond a gate's own depth a way laid anew through it runs straight before
/// swinging back to its old line, either side.
const STRAIGHT: i32 = 3;
/// The least run over which it swings across, either side of its straight.
const SWING: f32 = 8.0;
/// How many places along a way are tried for laying it anew.
const TRIES: usize = 16;
/// How far an old wall's line may stray in or out of its ring to get round what stands
/// on it, in columns.
const STRAY: i32 = 4;

impl Town<'_> {
    /// How old and how well kept what stands at the town's edges is, from its age and
    /// prosperity.
    fn edge_spec(&self, which: Special, seed: u32) -> SpecialSpec {
        let age = match self.s.age {
            Age::Young => 0.15,
            Age::Established => 0.4,
            Age::Ancient => 0.75,
        } + 0.2 * unit(seed ^ 0x0a6e);
        SpecialSpec { which, region: self.s.region, age: age.min(1.0), wealth: self.s.prosperity, seed, stream: None, facts: Facts::of(&self.s) }
    }

    /// The ring an ancient town's old wall follows: the town as it was founded, a little
    /// under half its size now, nearer in on some sides than others, so the town has
    /// grown far past it there.
    pub(super) fn old_radius(&self, p: Vec2) -> f32 {
        let (gr, r) = (self.green.radius, self.s.radius);
        let d = (p - self.centre).normalize_or(Vec2::X);
        let base = if self.s.kind == Kind::GreatTown { 0.42 } else { 0.5 };
        let wobble = perlin2(self.s.seed ^ 0x01d0_a11, d.x as f64 * 1.3 + 4.1, d.y as f64 * 1.3 - 2.3);
        gr + 8.0 + (r - gr - 8.0) * (base + 0.2 * wobble)
    }

    /// Keeps the ground for a gate on each main way in of a town (every way in of a great
    /// town), where it comes in a little inside the edge; in an ancient town, for a
    /// gatehouse where the way passes the old wall instead. Each is built now and kept
    /// for [`Self::raise_gates`]; its columns are kept from every plot.
    pub(super) fn keep_for_gates(&mut self, net: &mut Network) {
        if self.s.kind < Kind::Town {
            return;
        }
        let which = if self.s.age == Age::Ancient { Special::Gatehouse } else { Special::Gate };
        let great = self.s.kind == Kind::GreatTown;
        for li in 0..self.lanes.len() {
            let Way::Arrival { main } = self.lanes[li].way else { continue };
            if !(main || great) {
                continue;
            }
            // Where the way runs nearly square to the world near the ring, as it is; else
            // a stretch of it laid anew to; else wherever further in it runs so.
            let site = match self.gate_site(li, which, net, NEAR_BAND, None) {
                Some(found) => Some(found),
                None => match self.squared_site(li, which, net) {
                    Some(found) => Some(found),
                    None => self.gate_site(li, which, net, WIDE_BAND, None),
                },
            };
            if let Some((built, g)) = site {
                // All of it kept, what stands at walking height taken.
                for z in g.lo.y..=g.hi.y {
                    for x in g.lo.x..=g.hi.x {
                        self.grid.mark(IVec2::new(x, z), KEEP);
                    }
                }
                for q in standing(&built.building) {
                    let c = world(IVec3::new(g.origin.x, 0, g.origin.z), q, g.turns);
                    self.grid.mark(c, THING);
                }
                let (a, b) = (g.lo.as_vec2(), g.hi.as_vec2() + 1.0);
                self.discs.push(Disc { at: (a + b) * 0.5, r: (b - a).length() * 0.5 + 1.0, y: None, owner: None });
                self.edge.gates.push((built, g));
            }
        }
    }

    /// Where on way `li` a gate of kind `which` stands, built for it: the point nearest
    /// its ring where the way runs most nearly along a row or column of the world, so
    /// its way through, turned square to the world, holds the road; with a way through
    /// just wide enough that nothing of it at walking height stands on any worn ground,
    /// on ground nothing else stands on.
    /// `band` is how far out past its ring and in a gate may stand; `span`, if given,
    /// the points of the way it may stand at.
    fn gate_site(&self, li: usize, which: Special, net: &Network, band: (f32, f32), span: Option<Range<usize>>) -> Option<(Built, Gate)> {
        let lane = &self.lanes[li];
        let n = lane.pts.len();
        if n < 20 {
            return None;
        }
        let ancient = which == Special::Gatehouse;
        let (_, deep) = gate::reach(which);
        let seed = hash2(self.s.seed ^ 0x6a7e_0000, li as i32, 1);
        let spec = self.edge_spec(which, seed);
        // The gate at each width it may be, and its columns at walking height in its own
        // space, built once.
        let models: Vec<(Built, Vec<IVec2>)> = (gate::PASSAGE..=gate::WIDEST)
            .step_by(2)
            .filter_map(|w| special::gate(&spec, &self.pal, w))
            .map(|built| {
                let cols = standing(&built.building);
                (built, cols)
            })
            .collect();
        let mut cands: Vec<(f32, usize, IVec2, usize)> = Vec::new();
        let span = span.map_or(4..n - 4, |r| r.start.max(4)..r.end.min(n - 4));
        for i in span {
            let p = lane.pts[i];
            let d = p.distance(self.centre);
            let ring = self.gate_ring(p, ancient, deep);
            if !(band.0..=band.1).contains(&(d - ring)) || d < self.green.radius + 12.0 {
                continue;
            }
            let t = (lane.pts[i + 4] - lane.pts[i - 4]).normalize_or_zero();
            let out = if t.dot(p - self.centre) >= 0.0 { t } else { -t };
            let axis = if out.x.abs() >= out.y.abs() { IVec2::new(out.x.signum() as i32, 0) } else { IVec2::new(0, out.y.signum() as i32) };
            let (c, turns) = (p.floor().as_ivec2(), turns_toward(axis.as_vec2()));
            // The narrowest whose posts and lamps all stand off the worn ground: the road
            // and any way beside it pass between them, however the road runs through. A
            // post may stand at the very edge of the wear, so a road running slantwise
            // still finds room through an old gatehouse.
            let off_way = |q: IVec2| {
                let w = world(IVec3::new(c.x, 0, c.y), q, turns);
                let p = w.as_vec2() + 0.5;
                self.grid.get(w) & WAY == 0 || !self.worn.any(p, super::WIDEST + 1.0, |r| !r.door && r.at.distance(p) < r.hw + 0.5)
            };
            let Some(k) = models.iter().position(|(_, cols)| cols.iter().all(|&q| off_way(q))) else { continue };
            cands.push(((1.0 - out.dot(axis.as_vec2())) * 12.0 + (d - ring).abs() * 0.08 + k as f32 * 0.1, i, axis, k));
        }
        cands.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        for &(_, i, axis, k) in cands.iter().take(32) {
            let (built, model_cols) = &models[k];
            let b = &built.building;
            let c = lane.pts[i].floor().as_ivec2();
            let turns = turns_toward(axis.as_vec2());
            let at = IVec3::new(c.x, 0, c.y);
            // The road's height under the way through: the gate stands on its highest.
            let h = b.door.width / 2;
            let (mut top, mut low) = (i32::MIN, i32::MAX);
            for along in -deep..=deep {
                for k in -h..=h {
                    let q = world(at, IVec2::new(along, k), turns);
                    if let Some(p) = self.worn.nearest(q.as_vec2() + 0.5, 3.0) {
                        top = top.max(p.y.floor() as i32);
                        low = low.min(p.y.floor() as i32);
                    }
                }
            }
            if top == i32::MIN || top - low > 3 {
                continue;
            }
            let (lo, hi) = turn_rect((b.footprint.lo, b.footprint.hi), turns);
            let (lo, hi) = (lo + c, hi + c);
            let cols: Vec<IVec2> = (lo.y..=hi.y).flat_map(|z| (lo.x..=hi.x).map(move |x| IVec2::new(x, z))).collect();
            let standing: Vec<IVec2> = model_cols.iter().map(|&q| world(at, q, turns)).collect();
            if self.grid.any(&cols, PLOT | GREEN | KEEP | THING) || cols.iter().any(|q| q.as_vec2().distance(self.centre) > self.s.radius + 2.0) {
                continue;
            }
            if standing.iter().any(|q| {
                let col = self.sam.column(q.x, q.y);
                col.water_surface().is_some() || col.river < m(1.2)
            }) {
                continue;
            }
            // Clear of whatever the trails set beside the road here before the town was
            // laid out: a lantern, a milestone, a delight.
            let near = |q: IVec2| q.x >= lo.x - 1 && q.y >= lo.y - 1 && q.x <= hi.x + 1 && q.y <= hi.y + 1;
            let trails = net.lanterns.iter().any(|l| l.footprint.iter().any(|&q| near(q)))
                || net.markers.iter().any(|(_, p)| near(IVec2::new(p.x, p.z)))
                || net.delights.iter().any(|d| d.footprint.iter().any(|&q| near(q)));
            if trails {
                continue;
            }
            let origin = IVec3::new(c.x, top + 1, c.y);
            return Some((built.clone(), Gate { which, origin, turns, out: axis, passage: b.door.width, lo, hi, lane: li }));
        }
        None
    }

    /// The ring a gate stands near: an ancient town's gatehouse on its old wall's, any
    /// other gate a little inside the town's edge, as far in as it is deep.
    fn gate_ring(&self, p: Vec2, ancient: bool, deep: i32) -> f32 {
        if ancient { self.old_radius(p) } else { (self.s.radius - deep as f32 - 10.0).min(self.s.radius * 0.92) }
    }

    /// Where on way `li` a gate of kind `which` stands once a stretch of the way is laid
    /// anew to pass square through it: for a way running slantwise to every row and
    /// column of the world about its ring, or over nothing dry there. The points nearest
    /// the ring are tried first, a little further in and out than a gate on the way as it
    /// was may stand ([`WIDE_BAND`]). At each the way is laid anew ([`Self::square`]) and a
    /// gate looked for on its straight; if none will stand there, the way is put back as
    /// it was. Only a stretch a gate is found on goes into the network.
    fn squared_site(&mut self, li: usize, which: Special, net: &mut Network) -> Option<(Built, Gate)> {
        let ancient = which == Special::Gatehouse;
        let (_, deep) = gate::reach(which);
        let pts = self.lanes[li].pts.clone();
        let n = pts.len();
        if n < 20 {
            return None;
        }
        // Which of its points the way was laid on as it is: not where it was drawn
        // straight over water or ended at water too wide to cross, nor on a bridge. Only
        // about those is it laid anew.
        let (lo, hi) = pts.iter().fold((Vec2::MAX, Vec2::MIN), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
        let (points, _) = net.gather(lo, hi);
        let real: Vec<bool> = pts.iter().map(|p| points.iter().any(|t| t.x == p.x && t.z == p.y && t.half_width > 0.0)).collect();
        let mut order: Vec<(f32, usize)> = (8..n - 8)
            .filter_map(|i| {
                let p = pts[i];
                let d = p.distance(self.centre);
                let off = d - self.gate_ring(p, ancient, deep);
                let laid = real[i - 8..=i + 8].iter().all(|&r| r) && !is_wet(&self.sam.column(p.x.floor() as i32, p.y.floor() as i32));
                ((WIDE_BAND.0..=WIDE_BAND.1).contains(&off) && d >= self.green.radius + 12.0 && laid).then_some((off.abs(), i))
            })
            .collect();
        order.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut tried: Vec<usize> = Vec::new();
        for (_, i) in order {
            if tried.len() >= TRIES {
                break;
            }
            if tried.iter().any(|&t| t.abs_diff(i) < 3) {
                continue;
            }
            tried.push(i);
            let Some(sq) = self.square(li, i, deep, net) else { continue };
            // What laying it anew changes in the town's own reckoning, to put back.
            let worn = Worn { lo: self.worn.lo, n: self.worn.n, bins: self.worn.bins.clone() };
            let grid = Grid { x0: self.grid.x0, z0: self.grid.z0, n: self.grid.n, cells: self.grid.cells.clone() };
            let (lane, hw, laid) = (self.lanes[li].pts.clone(), self.lanes[li].hw.clone(), self.layout.lanes[li].clone());
            self.lay_square(&sq);
            if let Some(found) = self.gate_site(li, which, net, WIDE_BAND, Some(sq.straight.clone())) {
                sq.commit(net);
                return Some(found);
            }
            (self.worn, self.grid) = (worn, grid);
            (self.lanes[li].pts, self.lanes[li].hw, self.layout.lanes[li]) = (lane, hw, laid);
        }
        None
    }

    /// A stretch of way `li` about its point `i` laid anew to run square to the world,
    /// straight along the row or column nearest its own heading for as far as a gate
    /// `deep` deep needs and a little more, and swinging gently across from its old line
    /// to the straight and back either side. None where the stretch would be laid over
    /// water or a river's edge, kept or built ground, or anything the trails set beside
    /// a way; nor where another way joins it there or shares its line, or a bridge
    /// carries it, which are left as they were. Its height follows the ground as any
    /// lane's does, drawn to the way where it meets it again and to any it crosses.
    fn square(&self, li: usize, i: usize, deep: i32, net: &Network) -> Option<Squared> {
        let pts = &self.lanes[li].pts;
        let n = pts.len();
        let c = pts[i];
        let fwd = (pts[(i + 2).min(n - 1)] - pts[i.saturating_sub(2)]).normalize_or_zero();
        if fwd == Vec2::ZERO {
            return None;
        }
        let axis = if fwd.x.abs() >= fwd.y.abs() { Vec2::new(fwd.x.signum(), 0.0) } else { Vec2::new(0.0, fwd.y.signum()) };
        let half = (deep + STRAIGHT) as f32;
        // The further it must cross, the longer it takes to.
        let swing = SWING.max(2.5 * half * fwd.perp_dot(axis).abs());
        let near = i.saturating_sub(4)..(i + 4).min(n - 1);
        let spacing = near.clone().map(|j| pts[j].distance(pts[j + 1])).sum::<f32>() / near.len().max(1) as f32;
        let k = ((half + swing) / spacing.max(0.5)).ceil() as usize;
        if i < k + 1 || i + k + 1 >= n {
            return None;
        }
        let (from, to) = (i - k, i + k);
        let tangent = |j: usize| (pts[j + 1] - pts[j - 1]).normalize_or(fwd);
        let (a, b) = (c - axis * half, c + axis * half);
        let mut line = hermite(pts[from], tangent(from), a, axis);
        line.extend(hermite(b, axis, pts[to], tangent(to)));
        let laid = along(&line, spacing);
        if laid.len() < 3 {
            return None;
        }
        let old: Vec<Vec2> = pts[from + 1..to].to_vec();
        // The way as it was laid there: each point its own, none on a bridge, and no
        // other way joining it.
        let (lo, hi) = old.iter().chain(&laid).fold((Vec2::MAX, Vec2::MIN), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
        let (points, _) = net.gather(lo, hi);
        let mut hw = 0.0;
        for p in &old {
            let mut same = points.iter().filter(|t| t.x == p.x && t.z == p.y);
            match (same.next(), same.next()) {
                (Some(t), None) if t.half_width > 0.0 => hw += t.half_width,
                _ => return None,
            }
        }
        let hw = hw / old.len() as f32;
        if net.junctions.iter().any(|j| old.iter().any(|p| p.distance(*j) < 4.0)) {
            return None;
        }
        let ok = |p: Vec2| {
            let col = self.sam.column(p.x.floor() as i32, p.y.floor() as i32);
            !is_wet(&col)
                && col.river >= m(2.4)
                && p.distance(self.centre) <= self.s.radius + 4.0
                && self.discs.iter().all(|d| d.at == self.centre || d.at.distance(p) >= d.r + hw + 2.0)
                && self.grid.get(p.floor().as_ivec2()) & (PLOT | KEEP | THING) == 0
        };
        if !laid[1..laid.len() - 1].iter().all(|&p| ok(p)) {
            return None;
        }
        let on = |q: IVec2| laid[1..laid.len() - 1].iter().any(|p| (q.as_vec2() + 0.5).distance(*p) < hw + 0.5);
        if net.lanterns.iter().any(|l| l.footprint.iter().any(|&q| on(q))) || net.markers.iter().any(|(_, p)| on(IVec2::new(p.x, p.z))) || net.delights.iter().any(|d| d.footprint.iter().any(|&q| on(q))) {
            return None;
        }
        // Heights: the ground's, drawn to the way at either end and to any it crosses
        // (the nearest point of any way about, but the stretch it replaces).
        let y_at = |p: Vec2| self.worn.nearest(p, 0.5).map_or(self.green.y, |q| q.y);
        let mut pins = vec![(0, y_at(laid[0])), (laid.len() - 1, y_at(laid[laid.len() - 1]))];
        for (j, &p) in laid.iter().enumerate().take(laid.len() - 1).skip(1) {
            let mut best: Option<(f32, f32)> = None;
            self.worn.any(p, 4.0, |q| {
                let d = q.at.distance(p);
                if !q.door && !old.contains(&q.at) && best.is_none_or(|b| d < b.0) {
                    best = Some((d, q.y));
                }
                false
            });
            if let Some((_, y)) = best {
                pins.push((j, y));
            }
        }
        let heights = profile(&sample(self.c, self.seed, &laid), &pins);
        let new: Vec<TrailPoint> = (1..laid.len() - 1).map(|j| TrailPoint { x: laid[j].x, z: laid[j].y, y: heights[j], half_width: hw }).collect();
        // Its straight: the points of the lane, once laid, square through the gate.
        let square: Vec<usize> = new.iter().enumerate().filter(|(_, t)| (Vec2::new(t.x, t.z) - c).dot(axis).abs() <= half - 1.0).map(|(j, _)| from + 1 + j).collect();
        let straight = *square.first()?..*square.last()? + 1;
        Some(Squared { li, from, to, old, new, straight })
    }

    /// Lays `sq` into the town's own reckoning of its ways: the worn ground, the columns
    /// a way covers, and the lane.
    fn lay_square(&mut self, sq: &Squared) {
        let worn = &mut self.worn;
        for p in &sq.old {
            let (i, j) = worn.bin(*p);
            if (0..worn.n).contains(&i) && (0..worn.n).contains(&j) {
                let bin = &mut worn.bins[(j * worn.n + i) as usize];
                if let Some(k) = bin.iter().position(|q| !q.door && q.at == *p) {
                    bin.remove(k);
                }
            }
        }
        for t in &sq.new {
            worn.add(Point { at: Vec2::new(t.x, t.z), hw: t.half_width, y: t.y, door: false });
        }
        // The columns either stretch covers are reckoned afresh from every way about.
        let reach = WIDEST + 2.0;
        let all = sq.old.iter().copied().chain(sq.new.iter().map(|t| Vec2::new(t.x, t.z)));
        let (lo, hi) = all.fold((Vec2::MAX, Vec2::MIN), |(lo, hi), p| (lo.min(p), hi.max(p)));
        let (lo, hi) = ((lo - reach).floor().as_ivec2(), (hi + reach).ceil().as_ivec2());
        for z in lo.y..=hi.y {
            for x in lo.x..=hi.x {
                let (i, j) = (x - self.grid.x0, z - self.grid.z0);
                if (0..self.grid.n).contains(&i) && (0..self.grid.n).contains(&j) {
                    self.grid.cells[(j * self.grid.n + i) as usize] &= !WAY;
                }
            }
        }
        let (a, b) = (worn.bin(lo.as_vec2() - reach), worn.bin(hi.as_vec2() + reach));
        for j in a.1.max(0)..=b.1.min(worn.n - 1) {
            for i in a.0.max(0)..=b.0.min(worn.n - 1) {
                for q in &worn.bins[(j * worn.n + i) as usize] {
                    self.grid.disc(q.at, q.hw + 1.0, WAY);
                }
            }
        }
        let at: Vec<Vec2> = sq.new.iter().map(|t| Vec2::new(t.x, t.z)).collect();
        let hw: Vec<f32> = sq.new.iter().map(|t| t.half_width).collect();
        let lane = &mut self.lanes[sq.li];
        lane.pts.splice(sq.from + 1..sq.to, at.iter().copied());
        lane.hw.splice(sq.from + 1..sq.to, hw);
        self.layout.lanes[sq.li].splice(sq.from + 1..sq.to, at);
    }

    /// Builds every gate kept for, but any a door's path has since come through the
    /// posts of, with the short wings either side of a town's gate.
    pub(super) fn raise_gates(&mut self, net: &mut Network, fin: &Sampler) {
        for (built, g) in std::mem::take(&mut self.edge.gates) {
            let b = &built.building;
            let o = g.origin;
            let flat_o = IVec3::new(o.x, 0, o.z);
            let posts: Vec<IVec2> = standing(b).into_iter().map(|q| world(flat_o, q, g.turns)).collect();
            let crossed = posts.iter().any(|c| {
                let p = c.as_vec2() + 0.5;
                self.worn.any(p, 3.0, |q| q.door && q.at.distance(p) < q.hw + 0.5)
            });
            if crossed {
                continue;
            }
            let voxels: Vec<(IVec3, Voxel)> = b.voxels.iter().map(|&(q, v)| (o + rotate(q, g.turns), v)).collect();
            net.add_stamp_with(voxels, parts::place_fine(&b.fine, o, g.turns));
            if g.which == Special::Gate {
                self.wings(net, fin, &g, b);
            }
            self.layout.gates.push(g);
        }
    }

    /// The short runs leading off either side of a town's gate along its line, in the
    /// country's way, lower and more broken the further they go until they give out: a
    /// hedge in the lowland and the wetland, a dry-stone wall in the upland, a fence of
    /// rails in the north, the mountains and the old forests, mud brick in the desert, a
    /// palisade of poles in the rainforest. Each stops at the first thing in its way.
    fn wings(&mut self, net: &mut Network, fin: &Sampler, g: &Gate, b: &Building) {
        let salt = hash2(self.s.seed ^ 0x3129_0000, g.origin.x, g.origin.z);
        let long = 9 + (salt % 6) as i32;
        let reach = b.walls.hi.y.max(-b.walls.lo.y) + 1;
        let region = self.s.region;
        let along = world(IVec3::ZERO, IVec2::new(0, 1), g.turns);
        for s in [-1, 1] {
            let (mut voxels, mut fine): (Vec<(IVec3, Voxel)>, Vec<PlantBox>) = (Vec::new(), Vec::new());
            for k in 0..long {
                let c = world(IVec3::new(g.origin.x, 0, g.origin.z), IVec2::new(0, s * (reach + k)), g.turns);
                let own = c.cmpge(g.lo).all() && c.cmple(g.hi).all();
                let f = self.grid.get(c);
                if f & (WAY | PLOT | THING | GREEN) != 0 || (f & KEEP != 0 && !own) {
                    break;
                }
                let col = fin.column(c.x, c.y);
                if col.water_surface().is_some() || col.river < m(1.2) {
                    break;
                }
                let fade = k as f32 / long as f32;
                let h = hash2(salt, k, s);
                if k > 1 && unit(h) < fade * fade * 0.8 {
                    continue;
                }
                let high = ((1.0 - fade) * 4.0).ceil().max(1.0) as i32;
                let y0 = col.top() + 1;
                let at = |dy: i32| IVec3::new(c.x, y0 + dy, c.y);
                match region {
                    Region::Temperate | Region::Wetland => {
                        voxels.push((at(-1), mat::HEDGE_WOOD));
                        for dy in 0..high.min(3) {
                            let v = if unit(h >> (dy + 3)) < 0.12 { mat::LEAVES_HOLLY } else { mat::HEDGE };
                            voxels.push((at(dy), v));
                        }
                    }
                    Region::Upland => {
                        for dy in -2..high.min(3) {
                            let v = if unit(h >> (dy + 4)) < self.pal.moss * 0.6 { mat::MOSSY_STONE } else if (k + dy) % 2 == 0 { self.pal.stone } else { self.pal.stone_dark };
                            voxels.push((at(dy), v));
                        }
                    }
                    Region::Desert => {
                        for dy in -2..high.min(3) {
                            let v = if dy < 0 { self.pal.stone_dark } else if unit(h >> (dy + 4)) < 0.3 { mat::MUD_BRICK } else { mat::MUD_PLASTER };
                            voxels.push((at(dy), v));
                        }
                    }
                    Region::Rainforest => {
                        for dy in -2..high + (k % 2) {
                            voxels.push((at(dy), self.pal.wood));
                        }
                    }
                    Region::Northern | Region::Alpine | Region::OldGrowth => {
                        // A post every third column, two rails between.
                        if k % 3 == 0 {
                            for dy in -2..high.min(3) {
                                voxels.push((at(dy), self.pal.wood));
                            }
                        }
                        let cells = voxel::mesh::CELLS;
                        let (x, z) = (c.x * cells, c.y * cells);
                        for (r, dy) in [(0, 1), (1, 2)] {
                            if dy >= high.min(3) && r > 0 {
                                continue;
                            }
                            let y = (y0 + dy) * cells + 1;
                            let (lo, hi) = if along.x != 0 { (IVec3::new(x, y, z + 1), IVec3::new(x + cells, y + 1, z + 3)) } else { (IVec3::new(x + 1, y, z), IVec3::new(x + 3, y + 1, z + cells)) };
                            fine.push(PlantBox { lo: lo.to_array(), hi: hi.to_array(), voxel: self.pal.wood, sway_lo: false, sway_hi: false, thin: 0, season: 0, flap: 0 });
                        }
                    }
                }
                self.grid.mark(c, THING);
            }
            if !voxels.is_empty() || !fine.is_empty() {
                net.add_stamp_with(voxels, fine);
            }
        }
    }

    /// The old wall of an ancient town round the ring it was founded inside, routed
    /// between whatever stands on it: no plot, lane, path from a door, tree or water is
    /// built over, and where there is no way round, or a lane crosses, there is a gap.
    /// It meets the gatehouses on its ways in; elsewhere its broken ends step down.
    pub(super) fn raise_wall(&mut self, net: &mut Network, fin: &Sampler) {
        if self.s.age != Age::Ancient || self.s.kind < Kind::Town {
            return;
        }
        let tau = std::f32::consts::TAU;
        let mean = self.old_radius(self.centre + Vec2::X);
        let n = (tau * mean).ceil() as usize;
        // Under a gatehouse's eaves the wall may stand, against its piers; elsewhere it
        // keeps off all kept ground.
        let gates: Vec<(IVec2, IVec2)> = self.layout.gates.iter().map(|g| (g.lo, g.hi)).collect();
        let in_gate = |c: IVec2| gates.iter().any(|&(lo, hi)| c.cmpge(lo).all() && c.cmple(hi).all());
        let open = |t: &Self, c: IVec2| {
            let f = t.grid.get(c);
            f & (WAY | PLOT | THING | GREEN) == 0 && (f & KEEP == 0 || in_gate(c))
        };
        let dry = |c: IVec2| {
            let col = fin.column(c.x, c.y);
            col.water_surface().is_none() && col.river >= m(2.4)
        };
        let free = |t: &Self, c: IVec2| (-wall::HALF..=wall::HALF).all(|dz| (-wall::HALF..=wall::HALF).all(|dx| open(t, c + IVec2::new(dx, dz)))) && dry(c);
        // The ring drawn in or out to meet each gatehouse where its way passes.
        let bumps: Vec<(Vec2, f32)> = gates
            .iter()
            .map(|&(lo, hi)| {
                let mid = (lo.as_vec2() + hi.as_vec2() + 1.0) * 0.5;
                ((mid - self.centre).normalize_or(Vec2::X), mid.distance(self.centre) - self.old_radius(mid))
            })
            .collect();
        let ring_at = |t: &Self, d: Vec2| {
            // Over 24 columns of its length either side, or more where the gatehouse
            // stands far off the ring, so the wall bends to it rather than kinking.
            let bump: f32 = bumps.iter().map(|&(g, dr)| dr * (1.0 - d.angle_to(g).abs() * mean / (dr.abs() * 2.0 / 3.0).max(24.0)).max(0.0)).sum();
            t.old_radius(t.centre + d) + bump
        };
        // The column at each step round the ring and each stray in or out of it.
        let width = (2 * STRAY + 1) as usize;
        let mut at: Vec<Vec<Option<IVec2>>> = Vec::with_capacity(n);
        for i in 0..n {
            let d = Vec2::from_angle(i as f32 / n as f32 * tau);
            let ring = ring_at(self, d);
            at.push((-STRAY..=STRAY).map(|k| Some((self.centre + d * (ring + k as f32)).floor().as_ivec2()).filter(|&c| free(self, c))).collect());
        }
        // Round from a step with nowhere to stand, so no piece runs past where it began.
        let start = (0..n).find(|&i| at[i].iter().all(Option::is_none)).unwrap_or(0);
        let mut pieces: Vec<Vec<IVec2>> = Vec::new();
        let mut cost: Vec<f32> = vec![f32::INFINITY; width];
        let mut from: Vec<Vec<usize>> = Vec::new();
        let mut run: Vec<usize> = Vec::new();
        let flush = |cost: &[f32], from: &[Vec<usize>], run: &[usize], at: &[Vec<Option<IVec2>>], pieces: &mut Vec<Vec<IVec2>>| {
            if run.len() < 8 {
                return;
            }
            let Some(mut k) = (0..width).filter(|&k| cost[k].is_finite()).min_by(|&a, &b| cost[a].total_cmp(&cost[b])) else { return };
            let mut cols = vec![IVec2::ZERO; run.len()];
            for j in (0..run.len()).rev() {
                cols[j] = at[run[j]][k].unwrap_or_default();
                if j > 0 {
                    k = from[j][k];
                }
            }
            // Each column beside the one before, so the wall runs unbroken.
            let mut line: Vec<IVec2> = vec![cols[0]];
            for &c in &cols[1..] {
                let last = *line.last().unwrap();
                let steps = (c - last).abs().max_element();
                for s in 1..=steps {
                    let p = (last.as_vec2() + (c - last).as_vec2() * (s as f32 / steps as f32)).round().as_ivec2();
                    if line.last() != Some(&p) {
                        line.push(p);
                    }
                }
            }
            pieces.push(line);
        };
        for step in 0..=n {
            let i = (start + step) % n;
            let mut next = vec![f32::INFINITY; width];
            let mut back = vec![0usize; width];
            let mut any = false;
            if step < n {
                for k in 0..width {
                    if at[i][k].is_none() {
                        continue;
                    }
                    let stray = (k as i32 - STRAY).abs() as f32 * 0.15;
                    let (mut best, mut arg) = (if run.is_empty() { stray } else { f32::INFINITY }, k);
                    if !run.is_empty() {
                        for j in k.saturating_sub(1)..=(k + 1).min(width - 1) {
                            let c = cost[j] + (j as i32 - k as i32).abs() as f32 * 0.6 + stray;
                            if c < best {
                                (best, arg) = (c, j);
                            }
                        }
                    }
                    if best.is_finite() {
                        next[k] = best;
                        back[k] = arg;
                        any = true;
                    }
                }
            }
            if !any {
                flush(&cost, &from, &run, &at, &mut pieces);
                run.clear();
                from.clear();
                cost = vec![f32::INFINITY; width];
                // A step with somewhere to stand but no way on from the last starts afresh.
                if step < n && at[i].iter().any(Option::is_some) {
                    for k in 0..width {
                        if at[i][k].is_some() {
                            cost[k] = (k as i32 - STRAY).abs() as f32 * 0.15;
                        }
                    }
                    run.push(i);
                    from.push(vec![0; width]);
                }
                continue;
            }
            cost = next;
            run.push(i);
            from.push(back);
        }
        // Built in pieces of a few dozen columns, each its own stamp.
        let by_gate = |c: IVec2| gates.iter().any(|&(lo, hi)| c.x >= lo.x - 3 && c.y >= lo.y - 3 && c.x <= hi.x + 3 && c.y <= hi.y + 3);
        let seed = self.s.seed ^ 0x0d0a_11;
        let age = (0.55 + 0.4 * unit(seed)).min(1.0);
        let mut s0 = 0;
        let may = |t: &Self, c: IVec2| open(t, c) && dry(c);
        for line in pieces {
            let ends = (!by_gate(line[0]), !by_gate(line[line.len() - 1]));
            let n = line.len();
            for (j, part) in line.chunks(40).enumerate() {
                let last = (j + 1) * 40 >= n;
                let laid = wall::old_wall(part, s0, (ends.0 && j == 0, ends.1 && last), |c| fin.column(c.x, c.y).top(), |c| may(self, c), &self.pal, seed, age);
                s0 += part.len() as i32;
                for &(q, _) in &laid.voxels {
                    self.grid.mark(IVec2::new(q.x, q.z), THING);
                }
                if !laid.voxels.is_empty() {
                    net.add_stamp_with(laid.voxels, laid.fine);
                }
            }
            self.layout.wall.push(line);
            s0 += 11;
        }
    }
}

/// A stretch of a town's way laid anew to pass square through a gate (see
/// [`Town::square`]): the points of lane `li` it replaces, those between its points
/// `from` and `to`, and the trail points laid instead; `straight` is where the lane, so
/// laid, runs square through the gate.
struct Squared {
    li: usize,
    from: usize,
    to: usize,
    old: Vec<Vec2>,
    new: Vec<TrailPoint>,
    straight: Range<usize>,
}

impl Squared {
    /// Lays it into the network in place of the stretch it replaces, the way's line on
    /// the maps and for travellers following it there.
    fn commit(&self, net: &mut Network) {
        net.take_points(&self.old);
        net.add_points(&self.new);
        let (lo, hi) = self.old.iter().fold((Vec2::MAX, Vec2::MIN), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
        let last = self.new.len() - 1;
        for line in &mut net.lines {
            for q in line.iter_mut() {
                if q.cmplt(lo).any() || q.cmpgt(hi).any() {
                    continue;
                }
                if let Some(k) = self.old.iter().position(|p| p == q) {
                    let t = &self.new[(k as f32 / (self.old.len() - 1).max(1) as f32 * last as f32).round() as usize];
                    *q = Vec2::new(t.x, t.z);
                }
            }
        }
    }
}

/// A curve from `a`, heading `ta`, to `b`, heading `tb` (both unit), finely drawn.
fn hermite(a: Vec2, ta: Vec2, b: Vec2, tb: Vec2) -> Vec<Vec2> {
    let k = a.distance(b);
    (0..=32)
        .map(|s| {
            let u = s as f32 / 32.0;
            let (u2, u3) = (u * u, u * u * u);
            a * (2.0 * u3 - 3.0 * u2 + 1.0) + ta * k * (u3 - 2.0 * u2 + u) + b * (3.0 * u2 - 2.0 * u3) + tb * k * (u3 - u2)
        })
        .collect()
}

/// Points `step` apart along `line`, from its first to its last, which are kept as they
/// are (the last a little nearer the one before if need be).
fn along(line: &[Vec2], step: f32) -> Vec<Vec2> {
    let mut out = vec![line[0]];
    // How far along the line it is since the last point.
    let mut since = 0.0;
    for pair in line.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let seg = a.distance(b);
        let mut at = step - since;
        while at <= seg {
            out.push(a + (b - a) * (at / seg.max(1e-6)));
            at += step;
        }
        since = seg - (at - step);
    }
    let end = line[line.len() - 1];
    if out.len() > 1 && out[out.len() - 1].distance(end) < step * 0.5 {
        out.pop();
    }
    out.push(end);
    out
}

/// The columns a gate stands in at walking height, in its own space: its posts and
/// piers, and their lamps.
fn standing(b: &Building) -> Vec<IVec2> {
    let cells = voxel::mesh::CELLS;
    let mut out: Vec<IVec2> = b.voxels.iter().filter(|&&(q, v)| v != AIR && (0..gate::CLEAR).contains(&q.y)).map(|&(q, _)| IVec2::new(q.x, q.z)).collect();
    for f in &b.fine {
        let (lo, hi) = (IVec3::from_array(f.lo).div_euclid(IVec3::splat(cells)), (IVec3::from_array(f.hi) - 1).div_euclid(IVec3::splat(cells)));
        if lo.y < gate::CLEAR && hi.y >= 0 {
            for z in lo.z..=hi.z {
                for x in lo.x..=hi.x {
                    out.push(IVec2::new(x, z));
                }
            }
        }
    }
    out.sort_unstable_by_key(|c| (c.x, c.y));
    out.dedup();
    out
}
