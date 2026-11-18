//! A settlement's water side (#203): the fishers' boat shed, its open back to the water
//! and a slipway down into it, and a coastal great town's harbour wall, a curving mole
//! of stone out from the shore beside its landing with a beacon burning at its head.
//!
//! Jetties are not built here: the landing the trails set on the shore stays as it is,
//! and the harbour wall is kept well clear of it and its mooring. A stone quay along the
//! shore from beside the landing to where the wall meets the shore
//! ([`super::Harbour::root`]) is, so the wall is walked out to; the wall stands well up
//! out of the sea however shallow it is, and of the ways it might run the one in the
//! deepest water is built.

use glam::{IVec2, IVec3, Vec2, vec2};
use voxel::{AIR, Voxel};

use super::{Lot, PLOT, Purpose, THING, Town, WAY, Want, edges::{Harbour, Shed}, flat, world};
use crate::{
    features::{designs::Special, parts, shape::rotate},
    materials as mat,
    noise::hash2,
    scale::m,
    settlements::{
        Kind, Livelihood,
        kit::{Building, Step},
        special::{self, wall},
    },
    surface::Sampler,
    trails::{Network, Site},
};

/// How far behind a boat shed's open back its slipway may run to reach the water.
const SLIP: i32 = 14;

/// How far the top of a harbour wall stands over the sea's surface layer.
const MOLE_RISE: i32 = 3;

impl Town<'_> {
    /// Makes the fishers' workplace a boat shed where the place fishes or keeps a port
    /// and has open water near: the fisher's household lives in it, two of them, so a
    /// larger household lends the rest to homes with room, and one of one takes a
    /// lodger from a home of several. No shed if the beds will not come out even.
    pub(super) fn choose_shed(&mut self, wants: &mut [Want]) {
        let s = &self.s;
        if self.water.is_none() || !(s.has(Livelihood::Fisher) || s.has(Livelihood::Port)) {
            return;
        }
        let Some(i) = wants.iter().position(|w| w.purpose == Purpose::Workplace(Livelihood::Fisher)) else { return };
        let r = wants[i].residents as i32;
        let homes: Vec<usize> = (0..wants.len()).filter(|&j| j != i && wants[j].purpose == Purpose::Home).collect();
        if r > 2 {
            let room: i32 = homes.iter().map(|&j| wants[j].step.max_sleepers() as i32 - wants[j].residents as i32).sum();
            if room < r - 2 {
                return;
            }
            let mut left = r - 2;
            for &j in homes.iter().cycle().take(homes.len() * 10) {
                if left == 0 {
                    break;
                }
                if wants[j].residents < wants[j].step.max_sleepers() {
                    wants[j].residents += 1;
                    wants[j].beds += 1;
                    left -= 1;
                }
            }
        } else if r < 2 {
            let give: Vec<usize> = homes.iter().copied().filter(|&j| wants[j].residents >= 2).take((2 - r) as usize).collect();
            if give.len() < (2 - r) as usize {
                return;
            }
            for j in give {
                wants[j].residents -= 1;
                wants[j].beds -= 1;
            }
        }
        let w = &mut wants[i];
        w.residents = 2;
        w.beds = 2;
        w.step = Step::Timber;
        self.edge.shed = Some(w.seed);
    }

    /// Whether a plot for `want` with its origin at `origin`, turned by `turns`, will
    /// do: anything will but the boat shed, whose open back must meet the water.
    pub(super) fn backs_on_water(&self, lot: &Lot, want: &Want, origin: IVec2, turns: u32) -> bool {
        if self.edge.shed != Some(want.seed) || want.step == Step::Hut {
            return true;
        }
        water_behind(&self.sam, &lot.b, IVec3::new(origin.x, 0, origin.y), turns).is_some()
    }

    /// A slipway of planks down from each boat shed's open back into the water, on posts
    /// where the bank falls away, cut into it where it rises; and the harbour wall of a
    /// coastal great town.
    pub(super) fn harbour(&mut self, net: &mut Network, fin: &Sampler) {
        for k in 0..self.raised.len() {
            let r = &self.raised[k];
            if self.edge.shed != Some(r.want.seed) || r.want.step == Step::Hut {
                continue;
            }
            if let Some(shed) = self.slip(net, fin, k) {
                self.layout.sheds.push(shed);
            }
        }
        self.harbour_wall(net, fin);
    }

    fn slip(&mut self, net: &mut Network, fin: &Sampler, k: usize) -> Option<Shed> {
        let r = &self.raised[k];
        let (b, t, origin) = (&r.lot.b, r.spot.turns, r.origin);
        let reach = water_behind(fin, b, origin, t)?;
        let back = b.walls.lo.x;
        let out = flat(rotate(-IVec3::X, t));
        let own = |c: IVec2| self.discs.iter().filter(|d| d.owner == Some(k)).any(|d| d.at.distance(c.as_vec2() + 0.5) < d.r);
        let (mut voxels, mut foot): (Vec<(IVec3, Voxel)>, Option<IVec3>) = (Vec::new(), None);
        let mut y = origin.y - 3;
        for j in 1..=reach + 3 {
            let row: Vec<IVec2> = (-4..=4).map(|z| world(origin, IVec2::new(back - j, z), t)).collect();
            if row.iter().any(|&c| self.grid.get(c) & (WAY | THING) != 0 || (self.grid.get(c) & PLOT != 0 && !own(c))) {
                break;
            }
            if j % 2 == 0 {
                y -= 1;
            }
            let mut wet = false;
            for (n, &c) in row.iter().enumerate() {
                let col = fin.column(c.x, c.y);
                let ground = col.top();
                if let Some(s) = col.water_surface() {
                    wet = true;
                    y = y.max(s - 3);
                }
                voxels.push((IVec3::new(c.x, y, c.y), self.pal.plank));
                // Cut into the bank above it, posts under it at its edges where it stands
                // over the water or the falling ground.
                for h in y + 1..=ground.max(y) {
                    voxels.push((IVec3::new(c.x, h, c.y), AIR));
                }
                if (n == 0 || n == row.len() - 1) && j % 3 == 0 {
                    for h in ground - 1..y {
                        voxels.push((IVec3::new(c.x, h, c.y), self.pal.wood));
                    }
                }
            }
            let c0 = world(origin, IVec2::new(back - j, 0), t);
            foot = Some(IVec3::new(c0.x, y + 1, c0.y));
            if wet && j >= reach {
                break;
            }
        }
        let foot = foot?;
        net.add_stamp_with(voxels, Vec::new());
        Some(Shed { origin, slip: foot, out })
    }

    /// The harbour wall of a coastal great town with a landing: from the shore a little
    /// to one side of the landing, out into the sea and curving round in front of it, on
    /// ground the sea covers no deeper than a few metres, kept well clear of the landing
    /// and its mooring; a beacon on its head.
    fn harbour_wall(&mut self, net: &mut Network, fin: &Sampler) {
        if self.s.kind != Kind::GreatTown || !self.s.coast {
            return;
        }
        let Some((landing_at, site)) = self.s.port.and_then(|i| net.sites.get(i).map(|s| (i, *s))) else { return };
        let landing = vec2(site.x as f32 + 0.5, site.z as f32 + 0.5);
        let moor = site.mooring.map(|m| vec2(m.pos.x, m.pos.z)).unwrap_or(landing);
        let sea_dir = (if moor.distance(landing) > 2.0 { moor - landing } else { landing - self.centre }).normalize_or(Vec2::X);
        let across = vec2(-sea_dir.y, sea_dir.x);
        // Clear of the landing and its mooring, and leaving a mouth open straight out to
        // sea from the mooring, which the fishing boats put out through.
        let clear_of_landing = |p: Vec2| {
            let ab = moor - landing;
            let t = ((p - landing).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
            let out = (p - moor).dot(sea_dir).max(0.0);
            (landing + ab * t).distance(p) > m(7.0) && p.distance(moor) > m(8.0) && (moor + sea_dir * out).distance(p) > m(6.0)
        };
        let wet = |c: IVec2| fin.column(c.x, c.y).water_surface();
        // Of the ways it might run, out from the shore to one side of the landing or the
        // other, nearer it or further, the one standing in the deepest water is built,
        // so that its foot is well down in the sea rather than out on the flats.
        let mut best: Option<(f32, Vec<IVec2>, i32)> = None;
        for (side, off) in [(1.0, 30.0), (-1.0, 30.0), (1.0, 40.0), (-1.0, 40.0), (1.0, 22.0), (-1.0, 22.0)] {
            // Along a line out to sea beside the landing, the last dry column is the shore.
            let base = landing + across * side * off;
            let Some(k) = (-24..=30).find(|&k| wet((base + sea_dir * k as f32).floor().as_ivec2()).is_some()) else { continue };
            let start = base + sea_dir * (k - 1) as f32;
            let Some(sea) = wet((start + sea_dir).floor().as_ivec2()) else { continue };
            // Out, then round in front of the landing: its heading turns toward it. A
            // longer wall reaches further out before it turns, into deeper water where the
            // shore shelves gently.
            for long in [46, 66] {
                let mut p = start;
                let mut line: Vec<IVec2> = vec![p.floor().as_ivec2()];
                let mut ok = true;
                for j in 1..long {
                    let f = j as f32 / long as f32;
                    let turn = -side * 1.5 * f * f * (3.0 - 2.0 * f);
                    p += Vec2::from_angle(sea_dir.to_angle() + turn);
                    let c = p.floor().as_ivec2();
                    if line.last() == Some(&c) {
                        continue;
                    }
                    let deep = wet(c).map_or(j > 3, |s| s - fin.column(c.x, c.y).top() > 14);
                    if deep || !clear_of_landing(p) || self.grid.get(c) & (WAY | PLOT | THING) != 0 && self.grid.get(c) != u8::MAX {
                        ok = j > 28;
                        break;
                    }
                    line.push(c);
                }
                if !ok || line.len() < 24 {
                    continue;
                }
                let depth = line
                    .iter()
                    .map(|c| {
                        let col = fin.column(c.x, c.y);
                        col.water_surface().map_or(0, |s| (s - 1 - col.top()).clamp(0, 14)) as f32
                    })
                    .sum::<f32>()
                    / line.len() as f32;
                if best.as_ref().is_none_or(|b| depth > b.0 + 0.5) {
                    best = Some((depth, line, sea));
                }
            }
        }
        let Some((_, line, sea)) = best else { return };
        // It stands well up out of the sea, however shallow the water about its foot:
        // its top a man's height and more over the water, the parapet over that.
        let top = sea + MOLE_RISE;
        {
            let n = line.len();
            let mid = line[n / 2].as_vec2();
            let laid = wall::mole(
                &line,
                top,
                |c| fin.column(c.x, c.y).top(),
                |i| {
                    // The parapet on the side away from the landing, toward the open sea.
                    let a = line[(i + 2).min(n - 1)].as_vec2() - line[i.saturating_sub(2)].as_vec2();
                    let nrm = vec2(-a.y, a.x);
                    if nrm.dot(mid - landing) >= 0.0 { 1.0 } else { -1.0 }
                },
                |c| { let f = self.grid.get(c); f == u8::MAX || f & (WAY | PLOT | THING) == 0 },
                &self.pal,
                self.s.seed ^ 0x4a7b,
            );
            for &(q, _) in &laid.voxels {
                self.grid.mark(IVec2::new(q.x, q.z), THING);
            }
            net.add_stamp_with(laid.voxels, laid.fine);
            let head = line[n - 1];
            let beacon = IVec3::new(head.x, top + 1, head.y);
            let spec = self.edge_beacon(hash2(self.s.seed, head.x, head.y));
            if let Some(built) = special::build(&spec, &self.pal) {
                let b = &built.building;
                net.add_stamp_with(b.voxels.iter().map(|&(q, v)| (q + beacon, v)).collect(), parts::place_fine(&b.fine, beacon, 0));
            }
            let quay = self.shore_quay(net, fin, &site, line[0], top, sea_dir);
            self.layout.harbour = Some(Harbour { root: IVec3::new(line[0].x, top, line[0].y), line, top, beacon, landing: landing_at, quay });
        }
    }

    /// A stone quay along the shore from beside the landing to the harbour wall's root
    /// `root`, which the wall is walked out to along: three columns wide at the water's
    /// edge, the inner on the last of the dry ground and the outer two over the water,
    /// built up from the bed to a deck of flags that rises gently from the height of the
    /// landing's boards to the wall's top `top`, with a dark kerb and a mooring post
    /// now and then along its outer edge. It keeps off the landing's boards, every plot
    /// and whatever else is built about the shore, and gives way to ground standing
    /// higher than it; if too much is in its way there is none. Its deck's columns, in
    /// order from the landing.
    fn shore_quay(&mut self, net: &mut Network, fin: &Sampler, site: &Site, root: IVec2, top: i32, sea_dir: Vec2) -> Vec<IVec2> {
        let landing = vec2(site.x as f32 + 0.5, site.z as f32 + 0.5);
        let end = root.as_vec2() + 0.5;
        let across = vec2(-sea_dir.y, sea_dir.x);
        let side = if across.dot(end - landing) >= 0.0 { across } else { -across };
        // From the beach beside the landing's steps, clear of its boards and rails.
        let start = landing - sea_dir * 2.0 + side * 6.0;
        let n = start.distance(end).ceil() as usize;
        if n < 4 {
            return Vec::new();
        }
        let at = |i: usize| start.lerp(end, i as f32 / n as f32);
        let wet = |p: Vec2| fin.column(p.x.floor() as i32, p.y.floor() as i32).water_surface().is_some();
        // How far out along the sea's way the last dry column is at each step, filled in
        // where no water was found near and smoothed, so the quay follows the shore's
        // run without its every jog.
        let found: Vec<Option<f32>> = (0..=n).map(|i| (-10..=12).find(|&k| wet(at(i) + sea_dir * k as f32)).map(|k| k as f32 - 1.0)).collect();
        let Some(first) = found.iter().flatten().next().copied() else { return Vec::new() };
        let mut edge = Vec::with_capacity(found.len());
        let mut last = first;
        for f in &found {
            last = f.unwrap_or(last);
            edge.push(last);
        }
        let edge: Vec<f32> = (0..edge.len()).map(|i| {
            let (a, b) = (i.saturating_sub(3), (i + 3).min(edge.len() - 1));
            edge[a..=b].iter().sum::<f32>() / (b - a + 1) as f32
        }).collect();
        let on_landing = |p: Vec2| (p - landing).dot(side).abs() < 5.5 && (p - landing).dot(sea_dir) > -4.0;
        let mut deck: Vec<(IVec2, i32, bool, usize)> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut blocked = 0;
        for i in 0..=n {
            let t = i as f32 / n as f32;
            let level = (site.y as f32 + (top - site.y) as f32 * t * t * (3.0 - 2.0 * t)).round() as i32;
            for w in 0..3 {
                let p = at(i) + sea_dir * (edge[i] + w as f32 + 0.5);
                let c = p.floor().as_ivec2();
                if !seen.insert(c) {
                    continue;
                }
                // Beyond the town's own reckoning (all flags) nothing of it stands.
                let f = match self.grid.get(c) {
                    u8::MAX => 0,
                    f => f,
                };
                let by_wall = i + 6 >= n;
                if on_landing(p) || f & PLOT != 0 || (f & THING != 0 && !by_wall) {
                    blocked += 1;
                    continue;
                }
                if fin.column(c.x, c.y).top() > level {
                    continue;
                }
                deck.push((c, level, w == 2, i));
            }
        }
        if deck.len() < 3 * n / 2 || blocked * 5 > deck.len() {
            return Vec::new();
        }
        let mut voxels: Vec<(IVec3, Voxel)> = Vec::new();
        for &(c, level, outer, i) in &deck {
            let ground = fin.column(c.x, c.y).top();
            for y in ground.min(level) - 2..level {
                let r = hash2(self.s.seed ^ 0x9a4e, c.x * 31 + y, c.y);
                let v = if y < level - 2 && r % 2 == 0 { mat::MOSSY_STONE } else if r % 3 == 0 { self.pal.stone_dark } else { self.pal.stone };
                voxels.push((IVec3::new(c.x, y, c.y), v));
            }
            voxels.push((IVec3::new(c.x, level, c.y), if outer { self.pal.stone_dark } else { self.pal.stone }));
            if outer && i % 7 == 3 && i + 6 < n {
                voxels.push((IVec3::new(c.x, level + 1, c.y), self.pal.wood));
            }
            self.grid.mark(c, THING);
        }
        net.add_stamp_with(voxels, Vec::new());
        deck.into_iter().map(|d| d.0).collect()
    }

    fn edge_beacon(&self, seed: u32) -> special::SpecialSpec {
        special::SpecialSpec { which: Special::Beacon, region: self.s.region, age: 0.5, wealth: self.s.prosperity, seed, stream: None, facts: special::Facts::default() }
    }
}

/// How many columns behind a boat shed's open back, with its origin at `origin` and
/// turned by `turns`, the water starts, within [`SLIP`]: water across the middle of
/// its slipway's width.
fn water_behind(sam: &Sampler, b: &Building, origin: IVec3, turns: u32) -> Option<i32> {
    let back = b.walls.lo.x;
    (1..=SLIP).find(|&k| {
        [-2, 0, 2].iter().all(|&z| {
            let c = world(origin, IVec2::new(back - k, z), turns);
            let col = sam.column(c.x, c.y);
            col.water_surface().is_some() || col.river < 0.0
        })
    })
}
