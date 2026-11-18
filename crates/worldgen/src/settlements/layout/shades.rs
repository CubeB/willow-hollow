//! Sail shades over the lanes of hot country (#203). In a desert or savanna town or large
//! village, a few of the lanes nearest the market and the green have sails of cloth
//! stretched across them between the buildings either side: from the wall of one to the
//! wall of the one facing it across the lane, tied high at one side and lower at the
//! other and twisted as sails are, so they sag and billow in the wind (see
//! [`public::lane_shade`]). Where a wall does not stand high enough to tie to, a pole is
//! set against it to tie to instead. Hot country's places are built loosely, and where
//! no building faces one across its lane near enough, its shade runs out from its wall
//! over the lane to a pair of poles past the lane's far edge.
//!
//! A shade hangs well over anyone going under it, [`HEADROOM`] voxels at least over the
//! highest ground it spans, and only the lane and the open ground beside it are under
//! it: nothing built, no step, no stall. It is fine parts alone, so it blocks no one.

use std::collections::HashSet;

use glam::{IVec2, IVec3, Vec2};
use voxel::{
    AIR, CHUNK, Voxel,
    mesh::{CELLS, PlantBox},
};

use super::{GREEN, KEEP, THING, Town, WAY, Way, edges::Shade};
use crate::{
    materials as mat,
    noise::hash2,
    scale::m,
    settlements::{Age, Kind, Region, kit::components::public},
    surface::Sampler,
    trails::{Network, Stamp},
};

/// How high a shade's ties are over the highest ground under it, in voxels. Its lowest
/// corner, at the low side of an old shade, hangs a voxel and a quarter under its ties
/// and the cloth a cell under that, so this keeps [`HEADROOM`] under it with a voxel to
/// spare for the wind.
const TIE: i32 = 11;
/// The least room left under a shade at rest, in voxels.
pub const HEADROOM: i32 = 8;
/// The fewest and most columns a shade spans across its lane, from wall to wall.
const SPAN: (i32, i32) = (6, 26);
/// How far out from the edge of the green the lanes are shaded, and how far apart two
/// shades are at the least.
const NEAR: f32 = m(40.0);
const APART: f32 = m(9.0);
/// The most columns a shade runs along its lane.
const WIDEST: i32 = 8;

/// A shade chosen, before it is hung.
struct Plan {
    /// The open columns at either side on its middle row, against the walls or past the
    /// lane, and how many columns either side of that row it spans.
    sides: [IVec2; 2],
    rows: (i32, i32),
    /// Along its rows, square to the way across.
    along: IVec2,
    /// The height of the ties, and of the highest ground under it.
    tie: i32,
    ground: i32,
    /// Whether either side is tied to poles, no wall standing to the ties there.
    poles: [bool; 2],
}

/// Where a shade could be hung: across a lane from a building's wall to the wall of the
/// building facing it, or to the lane's far edge.
struct Gap {
    /// How far out from the green's edge the lane is there.
    out: f32,
    /// Whether a building faces the first across the lane.
    faced: bool,
    /// The open columns at either side on its middle row, against the walls or past the
    /// lane, and the way across.
    sides: [IVec2; 2],
    d: IVec2,
    lane: usize,
    at: Vec2,
}

impl Town<'_> {
    /// A few sail shades across the lanes nearest the green of a hot country's town or
    /// large village, each stamped as fine parts alone, and kept in `Layout::shades`.
    /// For once every building and gate is stamped; `fin` is the ground as it is now.
    pub(super) fn shades(&mut self, net: &mut Network, fin: &Sampler) {
        if self.s.region != Region::Desert || self.s.kind < Kind::LargeVillage {
            return;
        }
        let seed = self.s.seed ^ 0x5ade;
        let want = match self.s.kind {
            Kind::LargeVillage => 2,
            Kind::Town => 3,
            _ => 4,
        } + (hash2(seed, 0, 1) % 2) as usize;
        let solid = self.solid_about(net);
        let age = match self.s.age {
            Age::Young => 0.2,
            Age::Established => 0.45,
            Age::Ancient => 0.75,
        };
        let mut hung: Vec<Vec2> = Vec::new();
        for (n, gap) in self.gaps().into_iter().enumerate() {
            if hung.len() >= want {
                break;
            }
            if hung.iter().any(|q| q.distance(gap.at) < APART) {
                continue;
            }
            let h = hash2(seed, n as i32, gap.lane as i32);
            let wide = WIDEST - 2 + (h % 3) as i32;
            let Some(plan) = self.plan_shade(fin, &solid, gap.sides, gap.d, wide) else { continue };
            // Tied high on one side or the other.
            let high = (h >> 8) as usize % 2;
            let [a, b] = [plan.sides[high], plan.sides[1 - high]].map(|q| IVec3::new(q.x, plan.tie, q.y));
            let mut fine = public::lane_shade(a, b, wide, self.s.prosperity, age, h);
            let low = fine.iter().map(|f| f.lo[1]).min().unwrap_or(i32::MAX);
            if fine.is_empty() || low < (plan.ground + 1 + HEADROOM) * CELLS {
                continue;
            }
            // The poles where no wall stood to the ties, at the corners on that side.
            let mut feet = Vec::new();
            for (k, &side) in plan.sides.iter().enumerate() {
                if !plan.poles[k] {
                    continue;
                }
                let toward = (side - plan.sides[1 - k]).signum();
                for r in [plan.rows.0, plan.rows.1] {
                    let q = side + plan.along * r;
                    feet.push(q);
                    fine.push(pole(q, toward, plan.along * r.signum(), fin.column(q.x, q.y).top(), plan.tie, self.pal.wood));
                }
            }
            if self.grid.any(&feet, THING | KEEP) {
                continue;
            }
            for &q in &feet {
                self.grid.mark(q, THING);
            }
            let (lo, hi) = (plan.sides[0] + plan.along * plan.rows.0, plan.sides[1] + plan.along * plan.rows.1);
            let poles = if high == 0 { plan.poles } else { [plan.poles[1], plan.poles[0]] };
            self.layout.shades.push(Shade { a, b, wide, lane: gap.lane, lo: lo.min(hi), hi: lo.max(hi), low, poles });
            net.add_stamp_with(Vec::new(), fine);
            hung.push(gap.at);
        }
    }

    /// Where shades could be hung near the green: across a lane from the wall of a
    /// building on it, square to the world as every building stands, to the wall of the
    /// building facing it across the lane, or where none does near, to the lane's far
    /// edge. Those between two buildings come first, and of each the nearest the green.
    /// Each is on the row the lane crosses nearest the middle of the wall.
    fn gaps(&self) -> Vec<Gap> {
        let reach = self.green.radius + NEAR + m(12.0);
        let walls: Vec<(IVec2, IVec2)> = self.plots.iter().map(|p| p.walls).collect();
        let built = |q: IVec2| walls.iter().any(|&(lo, hi)| q.cmpge(lo).all() && q.cmple(hi).all());
        let mut out = Vec::new();
        for &(lo, hi) in walls.iter().filter(|&&(lo, hi)| ((lo + hi + IVec2::ONE).as_vec2() * 0.5).distance(self.centre) < reach) {
            for (d, s) in [(IVec2::X, 1), (IVec2::X, -1), (IVec2::Y, 1), (IVec2::Y, -1)] {
                let o = IVec2::new(d.y, d.x);
                // The wall's line, and its rows, which a shade's middle row is among.
                let wall = if s > 0 { hi.dot(d) } else { lo.dot(d) };
                let (r0, r1) = (lo.dot(o), hi.dot(o));
                let middle = (lo.dot(o) + hi.dot(o)) as f32 * 0.5 + 0.5;
                // The lane before it, nearest the middle of the wall.
                let mut best: Option<(f32, usize, Vec2)> = None;
                // By its index among the layout's lanes, which leave out the green's edge.
                for (li, lane) in self.lanes.iter().filter(|l| l.way != Way::Green).enumerate() {
                    for &p in &lane.pts {
                        let (ahead, po) = ((p.dot(d.as_vec2()) - wall as f32) * s as f32, p.dot(o.as_vec2()));
                        if ahead < 2.5 || ahead > (SPAN.1 - 2) as f32 || po < r0 as f32 || po >= (r1 + 1) as f32 {
                            continue;
                        }
                        let off = (po - middle).abs();
                        if best.is_none_or(|b| off < b.0) {
                            best = Some((off, li, p));
                        }
                    }
                }
                let Some((_, lane, at)) = best else { continue };
                let from = at.distance(self.centre) - self.green.radius;
                if !(m(1.0)..NEAR).contains(&from) {
                    continue;
                }
                // Out across the lane on that row, to the next building's wall or to the
                // first column past the lane.
                let row = (at.dot(o.as_vec2()).floor() as i32).clamp(r0, r1);
                // The lane is crossed once past the column its point is in.
                let start = d * wall + o * row;
                let lane_at = (at.dot(d.as_vec2()).floor() as i32 - wall) * s;
                let (mut crossed, mut far) = (false, None);
                for k in 1..=SPAN.1 + 1 {
                    let q = start + d * s * k;
                    if built(q) {
                        far = (k > lane_at).then_some((k - 1, true));
                        break;
                    }
                    let flags = self.grid.get(q);
                    if flags & GREEN != 0 {
                        break;
                    }
                    if flags & WAY != 0 {
                        crossed |= k >= lane_at;
                    } else if crossed {
                        far = Some((k, false));
                        break;
                    }
                }
                let Some((k, faced)) = far else { continue };
                if !(SPAN.0..=SPAN.1).contains(&k) {
                    continue;
                }
                let (near, far) = (start + d * s, start + d * s * k);
                let sides = if s > 0 { [near, far] } else { [far, near] };
                out.push(Gap { out: from, faced, sides, d, lane, at });
            }
        }
        out.sort_by(|a, b| b.faced.cmp(&a.faced).then(a.out.total_cmp(&b.out)).then(a.lane.cmp(&b.lane)).then(a.sides[0].to_array().cmp(&b.sides[0].to_array())));
        out
    }

    /// Every solid voxel stamped about the green, as far out as shades are hung and from
    /// a little under the green to well over it.
    fn solid_about(&self, net: &Network) -> HashSet<IVec3> {
        let reach = self.green.radius + NEAR + m(12.0);
        let (lo, hi) = ((self.centre - reach).floor().as_ivec2(), (self.centre + reach).ceil().as_ivec2());
        let (y0, y1) = (self.green.y as i32 - 24, self.green.y as i32 + 48);
        let mut seen: HashSet<*const Stamp> = HashSet::new();
        let mut out = HashSet::new();
        for cz in lo.y.div_euclid(CHUNK)..=hi.y.div_euclid(CHUNK) {
            for cx in lo.x.div_euclid(CHUNK)..=hi.x.div_euclid(CHUNK) {
                for s in net.stamps_at(cx, cz) {
                    if s.hi.y < y0 || s.lo.y > y1 || !seen.insert(std::ptr::from_ref(s)) {
                        continue;
                    }
                    for &(q, v) in &s.voxels {
                        if v != AIR && v != mat::WATER && (y0..=y1).contains(&q.y) && q.x >= lo.x && q.x <= hi.x && q.z >= lo.y && q.z <= hi.y {
                            out.insert(q);
                        }
                    }
                }
            }
        }
        out
    }

    /// A shade from the open column `sides[0]` to `sides[1]` across `d` (along x or
    /// along z), on their row and `wide` rows about it, if one can be hung there: a
    /// building's wall against a side at least, and nothing but open ground under it
    /// all the way across.
    fn plan_shade(&self, fin: &Sampler, solid: &HashSet<IVec3>, sides: [IVec2; 2], d: IVec2, wide: i32) -> Option<Plan> {
        let along = IVec2::new(d.y, d.x);
        let rows = (-(wide - 1) / 2, wide / 2);
        let ground = |q: IVec2| fin.column(q.x, q.y).top();
        let at = |q: IVec2, y: i32| solid.contains(&IVec3::new(q.x, y, q.y));
        let built = |q: IVec2| self.plots.iter().any(|p| q.cmpge(p.walls.0).all() && q.cmple(p.walls.1).all());
        // A building's wall standing at `q` on its own ground: a window or a door may
        // open in it.
        let wall = |q: IVec2| built(q) && {
            let g = ground(q);
            (g + 1..=g + 4).filter(|&y| at(q, y)).count() >= 2
        };
        // Every column it spans is open from its ground to over the ties.
        let n = (sides[1] - sides[0]).dot(d);
        let cols: Vec<IVec2> = (rows.0..=rows.1).flat_map(|r| (0..=n).map(move |k| sides[0] + along * r + d * k)).collect();
        let high = cols.iter().map(|&q| ground(q)).max()?;
        let tie = high + TIE;
        if cols.iter().any(|&q| (ground(q) + 1..=tie + 1).any(|y| at(q, y))) || self.grid.any(&cols, GREEN | KEEP) {
            return None;
        }
        // Each side tied to the wall where one stands to the ties at both its corners,
        // and to poles where not; one side at least against a wall.
        let mut poles = [false; 2];
        let mut walled = false;
        for (k, s) in [-1, 1].into_iter().enumerate() {
            for r in [rows.0, rows.1] {
                let w = sides[k] + along * r + d * s;
                walled |= wall(w);
                poles[k] |= !wall(w) || !at(w, tie);
            }
        }
        if !walled {
            return None;
        }
        Some(Plan { sides, rows, along, tie, ground: high, poles })
    }
}

/// A pole in column `q`, against the wall the way `toward` and at the end of the shade
/// the way `end` along its rows, from its foot in ground `ground` to a hand over the tie
/// at `tie`. In cells.
fn pole(q: IVec2, toward: IVec2, end: IVec2, ground: i32, tie: i32, wood: Voxel) -> PlantBox {
    let half = CELLS / 2;
    let off = |a: i32, b: i32| if a + b > 0 { half } else { 0 };
    let (ox, oz) = (off(toward.x, end.x), off(toward.y, end.y));
    let lo = IVec3::new(q.x * CELLS + ox, ground * CELLS + half, q.y * CELLS + oz);
    let hi = IVec3::new(lo.x + half, (tie + 1) * CELLS + 1, lo.z + half);
    PlantBox { lo: lo.to_array(), hi: hi.to_array(), voxel: wood, sway_lo: false, sway_hi: false, thin: 0, season: 0, flap: 0 }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use voxel::{CHUNK, mesh::CELLS};

    use super::*;
    use crate::{ColumnOutput, World, continent::SimConfig, macro_layout::MACRO_SIZE, world::GameWorld};

    fn world(seed: u32) -> GameWorld {
        GameWorld::with_config(seed, SimConfig { size: 256 })
    }

    /// Hot continents of small worlds with towns or large villages of hot country: the
    /// world's seed and the continent. The first continent of every world is temperate.
    const PLACES: [(u32, i32, i32); 2] = [(1, 1, -1), (1, -2, 3)];

    /// Where continent `(mx, mz)` starts in the world, in voxels.
    fn start(mx: i32, mz: i32) -> IVec3 {
        IVec3::new(mx * MACRO_SIZE, 0, mz * MACRO_SIZE)
    }

    /// The voxel at `q`, in the world, of the chunk columns made so far, making its own
    /// if it is not among them.
    fn voxel_at(w: &GameWorld, made: &mut HashMap<(i32, i32), ColumnOutput>, q: IVec3) -> Option<Voxel> {
        let key = (q.x.div_euclid(CHUNK), q.z.div_euclid(CHUNK));
        let column = made.entry(key).or_insert_with(|| w.generate_column_bare(key.0, key.1));
        let (cy, ly) = (q.y.div_euclid(CHUNK), q.y.rem_euclid(CHUNK));
        column.chunks.iter().find(|(k, _)| *k == cy).map(|(_, ch)| ch.get((q.x - key.0 * CHUNK + 1) as usize, ly as usize + 1, (q.z - key.1 * CHUNK + 1) as usize))
    }

    /// Each shade hangs across one of its town's lanes, from walls or poles either side,
    /// with [`HEADROOM`] at least clear over everything under it all the way across; only
    /// hot country's towns and large villages have them.
    #[test]
    fn shades_hang_across_lanes_with_room_under_them() {
        let mut found = 0;
        for (seed, mx, mz) in PLACES {
            let w = world(seed);
            let c = w.continent(mx, mz);
            let o = start(mx, mz);
            let net = &c.trails;
            let mut made = HashMap::new();
            for (k, s) in net.settlements.iter().enumerate() {
                let l = &net.layouts[k];
                if s.region != Region::Desert || s.kind < Kind::LargeVillage {
                    assert!(l.shades.is_empty(), "seed {seed}: a {:?} {:?} has shades", s.region, s.kind);
                    continue;
                }
                for sh in &l.shades {
                    found += 1;
                    let (lo, hi) = (sh.lo, sh.hi);
                    let over = |p: &Vec2| p.floor().as_ivec2().cmpge(lo).all() && p.floor().as_ivec2().cmple(hi).all();
                    assert!(l.lanes[sh.lane].iter().any(over), "seed {seed}: the shade at {} is over none of its lane", sh.a);
                    // Room under it over the highest thing in each column it spans.
                    let top = sh.low.div_euclid(CELLS);
                    for z in lo.y..=hi.y {
                        for x in lo.x..=hi.x {
                            let under = (top - 40..=top).rev().find(|&y| voxel_at(&w, &mut made, o + IVec3::new(x, y, z)).is_some_and(|v| v != AIR)).expect("ground under a shade");
                            assert!(sh.low >= (under + 1 + HEADROOM) * CELLS, "seed {seed}: the shade at {} hangs to cell {} over {x},{under},{z}", sh.a, sh.low);
                        }
                    }
                    // Its ties at its corners are open, against a wall where it is not tied
                    // to poles.
                    let along = if sh.a.x == sh.b.x { IVec3::X } else { IVec3::Z };
                    for (q, from, pole) in [(sh.a, sh.b, sh.poles[0]), (sh.b, sh.a, sh.poles[1])] {
                        for corner in [q - along * ((sh.wide - 1) / 2), q + along * (sh.wide / 2)] {
                            assert_eq!(voxel_at(&w, &mut made, o + corner), Some(AIR), "seed {seed}: the shade's tie at {corner} is not open");
                            if !pole {
                                let wall = corner + (q - from).signum();
                                assert!(voxel_at(&w, &mut made, o + wall).is_some_and(|v| v != AIR), "seed {seed}: the shade's tie at {corner} has no wall to tie to");
                            }
                        }
                    }
                }
            }
        }
        assert!(found >= 2, "only {found} shades on the test worlds");
    }

    /// Every part of a shade is drawn once, by one chunk column, whichever chunks it
    /// spans; and the shades are the same however often the continent is made.
    #[test]
    fn shades_are_drawn_once_and_the_same_every_time() {
        let mut spanning = 0;
        for (seed, mx, mz) in PLACES {
            let w = world(seed);
            let c = w.continent(mx, mz);
            let again = world(seed);
            let d = again.continent(mx, mz);
            let o = start(mx, mz);
            let (on, cells) = (IVec2::new(o.x, o.z).div_euclid(IVec2::splat(CHUNK)), o * CELLS);
            for (k, l) in c.trails.layouts.iter().enumerate() {
                assert_eq!(format!("{:?}", l.shades), format!("{:?}", d.trails.layouts[k].shades), "seed {seed}");
                for sh in &l.shades {
                    let (c0, c1) = (sh.lo.div_euclid(IVec2::splat(CHUNK)), sh.hi.div_euclid(IVec2::splat(CHUNK)));
                    // Each part by what it is: a plant box is no more than its fields.
                    let mut parts: Vec<String> = Vec::new();
                    for cz in c0.y - 1..=c1.y + 1 {
                        for cx in c0.x - 1..=c1.x + 1 {
                            for t in c.trails.stamps_at(cx, cz).filter(|t| t.voxels.is_empty() && t.lo.x >= sh.lo.x - 1 && t.hi.x <= sh.hi.x + 1 && t.lo.z >= sh.lo.y - 1 && t.hi.z <= sh.hi.y + 1) {
                                for b in &t.fine {
                                    let b = PlantBox { lo: (IVec3::from_array(b.lo) + cells).to_array(), hi: (IVec3::from_array(b.hi) + cells).to_array(), ..*b };
                                    let b = format!("{b:?}");
                                    if !parts.contains(&b) {
                                        parts.push(b);
                                    }
                                }
                            }
                        }
                    }
                    assert!(!parts.is_empty(), "seed {seed}: no parts stamped for the shade at {}", sh.a);
                    let mut drawn: Vec<String> = Vec::new();
                    for cz in c0.y - 1..=c1.y + 1 {
                        for cx in c0.x - 1..=c1.x + 1 {
                            drawn.extend(w.generate_column(on.x + cx, on.y + cz).plants.iter().map(|p| format!("{p:?}")).filter(|p| parts.contains(p)));
                        }
                    }
                    for b in &parts {
                        assert_eq!(drawn.iter().filter(|p| *p == b).count(), 1, "seed {seed}: a part of the shade at {} is drawn other than once: {b}", sh.a);
                    }
                    spanning += (c0 != c1) as i32;
                }
            }
        }
        assert!(spanning > 0, "no shade spans a chunk border");
    }

    /// Lists the hot country's towns and large villages of a world (seed from `SEED`, 1
    /// if unset; a small world with `SMALL` set) and their shades, for aiming
    /// screenshots: `cargo test -p worldgen list_shades -- --ignored --nocapture`. The
    /// first continent is temperate; `CX` and `CZ` choose another, and its
    /// coordinates are its own (add 65536 times each for the world's).
    #[test]
    #[ignore]
    fn list_shades() {
        let seed = std::env::var("SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
        let w = if std::env::var("SMALL").is_ok() { world(seed) } else { GameWorld::new(seed) };
        let cell = |v: &str| std::env::var(v).ok().and_then(|s| s.parse().ok()).unwrap_or(0);
        let c = w.continent(cell("CX"), cell("CZ"));
        for (k, s) in c.trails.settlements.iter().enumerate() {
            if s.region != Region::Desert || s.kind < Kind::LargeVillage {
                continue;
            }
            let l = &c.trails.layouts[k];
            println!("{k} {:?} {:?} at {},{} y {} r {:.0}: {} shades of {} lanes", s.kind, s.age, s.x, s.z, s.y, s.radius, l.shades.len(), l.lanes.len());
            for sh in &l.shades {
                println!("    shade from {} to {} wide {} lane {} low {} poles {:?}", sh.a, sh.b, sh.wide, sh.lane, sh.low, sh.poles);
            }
        }
    }
}
