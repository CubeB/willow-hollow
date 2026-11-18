//! A town's market square (#203). A town's green, and a great town's beside its market
//! hall, is paved over in the region's stone, worn darker along the ways that cross it,
//! with its market stalls set round it near the edge, facing in, and the middle left open
//! for the crowd on market day. The well stands on it as it did on the green, kept clear
//! by the stalls as by everything set there; the village tree stands beside it, never on
//! it, and a great town's in its park.
//!
//! The green's pad is marked paved before anything is laid ([`Town::build`]), so the
//! ground there is worn bare all over when it is sampled again and nothing grows on it,
//! and here every column of it the paving can lie on is stamped with stone in its top
//! layer: flush with the ground, so a walker crosses it as they would the green, and
//! even, a stone a column with no speckle, the worn ways crisp bands across it.

use std::collections::BTreeMap;

use glam::{IVec2, IVec3, Vec2, vec2};
use voxel::{CHUNK, Voxel};

use super::Town;
use crate::{
    materials as mat,
    settlements::Kind,
    surface::{Cover, Sampler},
    trails::Network,
};

/// A town's market square: its green paved over.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Square {
    /// Its middle, continent-local, and how far out from it the paving reaches.
    pub at: Vec2,
    pub radius: f32,
    /// The layer the paving lies in: the top of the ground across the square.
    pub y: i32,
    /// The stone it is paved with, the region's own, and the darker stone of the ways
    /// worn across it.
    pub stone: Voxel,
    pub worn: Voxel,
}

impl Square {
    /// Whether continent-local column `c` is within the square, as its paving is laid.
    pub fn covers(&self, c: IVec2) -> bool {
        (c.as_vec2() + 0.5).distance(self.at) < self.radius
    }
}

/// `stone` as the ways worn across a square of it show it: the same stone, darker where
/// feet and wheels have worn it smooth and dirt has worked into it. Each is the next
/// darker of its kind.
fn worn_stone(stone: Voxel) -> Voxel {
    match stone {
        mat::STONE | mat::GRANITE_GREY => mat::CAVE1_STONE,
        mat::SANDSTONE => mat::SANDSTONE_DARK,
        mat::RED_ROCK => mat::RED_ROCK_DARK,
        mat::RED_ROCK_DARK => mat::CAVE2_SANDSTONE_RED,
        mat::GRANITE => mat::GRANITE_GREY,
        mat::GRANITE_PALE => mat::GRANITE,
        mat::BASALT => mat::CAVE3_ROCK_DARK,
        // ROCK_DARK, ROCK_PALE, ROCK_BROWN, SANDSTONE_DARK and SANDSTONE_RED are each a
        // rock the caves show darker.
        _ => mat::cave_rock(stone, 1),
    }
}

impl Town<'_> {
    /// Paves a town's green, its pad already marked paved, with the region's stone in the
    /// top layer of every column of it that lies level and dry, and darker stone along
    /// the ways across it; records the square in the layout.
    pub(super) fn pave_square(&mut self, net: &mut Network, fin: &Sampler) {
        if self.s.kind < Kind::Town {
            return;
        }
        let stone = self.pal.stone;
        let square = Square { at: self.centre, radius: self.green.radius, y: self.green.y.floor() as i32, stone, worn: worn_stone(stone) };
        // In a chunk's stamp each, so a chunk reads only the paving within it.
        let mut by_chunk: BTreeMap<(i32, i32), Vec<(IVec3, Voxel)>> = BTreeMap::new();
        let r = square.radius.ceil() as i32;
        let (cx, cz) = (self.centre.x.floor() as i32, self.centre.y.floor() as i32);
        for z in cz - r..=cz + r {
            for x in cx - r..=cx + r {
                let c = IVec2::new(x, z);
                if !square.covers(c) {
                    continue;
                }
                // Only where the ground was worn bare for it, so nothing grows through,
                // and where it lies at the square's level.
                let col = fin.column(x, z);
                if col.cover != Cover::Trail || col.water_surface().is_some() || col.top() != square.y {
                    continue;
                }
                let p = vec2(x as f32 + 0.5, z as f32 + 0.5);
                let way = self.worn.nearest(p, super::WIDEST + 1.0).is_some_and(|q| q.at.distance(p) < q.hw);
                let v = if way { square.worn } else { square.stone };
                by_chunk.entry((x.div_euclid(CHUNK), z.div_euclid(CHUNK))).or_default().push((IVec3::new(x, square.y, z), v));
            }
        }
        for (_, voxels) in by_chunk {
            net.add_stamp_with(voxels, Vec::new());
        }
        self.layout.square = Some(square);
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, HashSet},
        sync::{Arc, OnceLock},
    };

    use super::*;
    use crate::{
        ColumnOutput,
        continent::{Continent, SimConfig},
        scale::m,
        settlements::calendar,
        world::GameWorld,
    };

    /// The first continent of each of eight small worlds, with its world: between them
    /// every kind of settlement.
    fn worlds() -> &'static Vec<(GameWorld, Arc<Continent>)> {
        static W: OnceLock<Vec<(GameWorld, Arc<Continent>)>> = OnceLock::new();
        W.get_or_init(|| {
            (1..=8)
                .map(|seed| {
                    let w = GameWorld::with_config(seed, SimConfig { size: 256 });
                    let c = w.continent(0, 0);
                    (w, c)
                })
                .collect()
        })
    }

    /// The voxel at continent-local `q` of continent (0, 0), in the world, of the chunk
    /// columns made so far, making its own if it is not among them.
    fn voxel_at(w: &GameWorld, made: &mut HashMap<(i32, i32), ColumnOutput>, q: IVec3) -> Option<Voxel> {
        let key = (q.x.div_euclid(CHUNK), q.z.div_euclid(CHUNK));
        let column = made.entry(key).or_insert_with(|| w.generate_column_bare(key.0, key.1));
        let (cy, ly) = (q.y.div_euclid(CHUNK), q.y.rem_euclid(CHUNK));
        column.chunks.iter().find(|(k, _)| *k == cy).map(|(_, ch)| ch.get((q.x - key.0 * CHUNK + 1) as usize, ly as usize + 1, (q.z - key.1 * CHUNK + 1) as usize))
    }

    /// The ground a stall and its customers' first row take, in square metres.
    const STALL_AREA: f32 = 6.0;

    /// Every town and great town has a square, its green paved in the region's stone with
    /// the ways worn darker across it, and the paving in the world where it was laid; no
    /// building stands on it, the village tree stands off it, and its stalls stand round
    /// it on the paving, clear of the well and of its middle, which is room enough for
    /// everyone of the place at once. Nothing smaller has one.
    #[test]
    fn every_town_has_a_paved_square_clear_of_buildings() {
        let mut squares = 0;
        for (w, c) in worlds() {
            let seed = w.seed();
            let net = &c.trails;
            let mut made = HashMap::new();
            for (k, s) in net.settlements.iter().enumerate() {
                let l = &net.layouts[k];
                let Some(sq) = l.square else {
                    assert!(s.kind < Kind::Town, "seed {seed}: the {:?} at {},{} has no square", s.kind, s.x, s.z);
                    continue;
                };
                assert!(s.kind >= Kind::Town, "seed {seed}: a {:?} has a square", s.kind);
                squares += 1;
                let at = format!("seed {seed}: the {:?} at {},{}", s.kind, s.x, s.z);
                // Paved where it was laid, in stone, the worn ways darker.
                let r = sq.radius.ceil() as i32 + 1;
                let (lo, hi) = (sq.at.floor().as_ivec2() - r, sq.at.floor().as_ivec2() + r);
                // The paving's stamps, all of it in the one layer and nothing else; and
                // whatever else is stamped in that layer, the well's footing and the like,
                // which stands on the paving.
                let mut laid: BTreeMap<(i32, i32), Voxel> = BTreeMap::new();
                let mut over: HashSet<(i32, i32)> = HashSet::new();
                for st in net.stamps().filter(|st| st.lo.x <= hi.x && st.hi.x >= lo.x && st.lo.z <= hi.y && st.hi.z >= lo.y) {
                    let paving = st.fine.is_empty() && st.voxels.iter().all(|&(q, v)| q.y == sq.y && (v == sq.stone || v == sq.worn));
                    for &(q, v) in &st.voxels {
                        if !paving {
                            over.insert((q.x, q.z));
                        } else if sq.covers(IVec2::new(q.x, q.z)) {
                            laid.insert((q.x, q.z), v);
                        }
                    }
                }
                let covered = (lo.y..=hi.y).flat_map(|z| (lo.x..=hi.x).map(move |x| IVec2::new(x, z))).filter(|&c| sq.covers(c)).count();
                assert!(laid.len() * 10 >= covered * 7, "{at}: {} of {covered} columns paved", laid.len());
                let plain = laid.values().filter(|&&v| v == sq.stone).count();
                assert!(plain * 2 > laid.len() && sq.worn != sq.stone, "{at}: {plain} of {} plain", laid.len());
                for (&(x, z), &v) in laid.iter().filter(|(c, _)| !over.contains(c)).step_by(37).take(12) {
                    let q = IVec3::new(x, sq.y, z);
                    assert_eq!(voxel_at(w, &mut made, q), Some(v), "{at}: no paving in the world at {q}");
                }
                // No building on it, nor the village tree.
                for p in &net.plots[l.plots.clone()] {
                    for z in p.lo.y..=p.hi.y {
                        for x in p.lo.x..=p.hi.x {
                            assert!(!sq.covers(IVec2::new(x, z)), "{at}: the {} at {} stands on the square", p.purpose.word(), p.origin);
                        }
                    }
                }
                if let Some(t) = net.village_trees[k] {
                    assert!(Vec2::new(t.x as f32 + 0.5, t.z as f32 + 0.5).distance(sq.at) > sq.radius, "{at}: the village tree stands on the square");
                }
                // Its stalls round it on the paving, clear of the well and the middle.
                assert!(l.market.len() >= 5, "{at}: {} stalls", l.market.len());
                for m in &l.market {
                    let p = Vec2::new(m.at.x as f32 + 0.5, m.at.z as f32 + 0.5);
                    assert!(p.distance(sq.at) < sq.radius && p.distance(sq.at) > sq.radius * 0.4, "{at}: a stall at {}", m.at);
                    for well in &l.wells {
                        assert!(Vec2::new(well.x as f32 + 0.5, well.z as f32 + 0.5).distance(p) >= 3.0, "{at}: a stall by the well at {well}");
                    }
                }
                // Room for everyone of the place at once on market day, two square metres
                // each, among the stalls.
                let open = std::f32::consts::PI * (sq.radius / m(1.0)).powi(2) - l.market.len() as f32 * STALL_AREA;
                println!("{at}: a square {:.1} m across, {} stalls, {open:.0} m2 open for {} people", 2.0 * sq.radius / m(1.0), l.market.len(), s.people);
                assert!(open >= 2.0 * s.people as f32, "{at}: {open:.0} m2 for {} people", s.people);
            }
        }
        assert!(squares >= 2, "only {squares} squares in eight worlds");
    }

    /// What is set out every day follows the kind of place: a village's stalls only on
    /// market day; a couple of a large village's every day, and a few of a town's, never
    /// all of them, and never one fallen into disrepair.
    #[test]
    fn stalls_are_kept_every_day_by_the_kind_of_place() {
        let mut daily = [0; 5];
        for (w, c) in worlds() {
            let net = &c.trails;
            for (k, s) in net.settlements.iter().enumerate() {
                let market = &net.layouts[k].market;
                let sound = market.iter().filter(|m| m.awning.is_some()).count();
                let kept: Vec<&calendar::Stall> = market.iter().filter(|m| m.daily()).collect();
                let at = format!("seed {}: the {:?} at {},{}", w.seed(), s.kind, s.x, s.z);
                assert_eq!(kept.len(), calendar::daily_stalls(s.kind).min(sound.saturating_sub(1)), "{at}: {} of {sound} kept every day", kept.len());
                assert!(kept.iter().all(|m| m.awning.is_some()), "{at}: a broken stall kept every day");
                daily[s.kind as usize] += kept.len();
            }
        }
        println!("stalls kept every day by kind: {daily:?}");
        assert!(daily[Kind::LargeVillage as usize] > 0, "no large village keeps a stall every day");
    }
}
