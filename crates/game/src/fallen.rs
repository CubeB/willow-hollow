//! Sticks fallen from the trees, lying on the ground to be picked up.
//!
//! The world does not know them: where each lies is worked out here from the trees the
//! world grows and a hash of where they stand, so the same seed always has the same
//! sticks in the same places, and none of it changes how the world is generated. A wood
//! has plenty, under every few trees; a tree standing alone in a field seldom has any.
//! A stick picked up is remembered by the pack as a spot gathered from, as a picked
//! flower is, so it stays gone until more have fallen there a year or two on.
//!
//! Resin (#40) is found the same way: now and then a resinous tree (a pine, a juniper,
//! an acacia, a tree of the rainforest) has wept a bead of it from its bark at hand
//! height, and a bead taken weeps again a year or two on.

use std::collections::HashSet;

use glam::{DVec3, IVec3};
use voxel::CHUNK;
use worldgen::{
    features::trees::Species,
    noise::{hash3, unit},
    scale::{m, m64},
    world::GameWorld,
};

use crate::{
    inventory,
    particles::{ParticleInstance, dither_edge, dither_since},
    placed,
    player::WorldVoxels,
};

/// How far from the camera sticks are looked for, and drawn, in metres.
pub const REACH: f64 = 20.0;
/// How far round a tree other trees are counted, to tell a wood from a lone tree.
const WOOD: f64 = 12.0;
/// The most boxes the sticks in view may come to.
const MOST_BOXES: usize = 280;

/// What there is to be picked up: sticks on the ground, or resin on a trunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Find {
    Sticks,
    Resin,
}

/// A few sticks lying together on the ground, or a bead of resin on the bark.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stick {
    /// Where they lie, on the top of the ground, in world voxels; for resin, where it
    /// stands out from the bark.
    pub pos: DVec3,
    /// Which way they lie; for resin, the way out from the trunk.
    pub yaw: f32,
    /// How many: one or two, and resin one.
    pub count: u32,
    pub what: Find,
}

impl Stick {
    /// The column a pack remembers them gathered by.
    pub fn spot(&self) -> (i32, i32) {
        (self.pos.x.floor() as i32, self.pos.z.floor() as i32)
    }

    /// What it goes into the pack as.
    pub fn item(&self) -> inventory::Item {
        match self.what {
            Find::Sticks => inventory::sticks(),
            Find::Resin => inventory::resin(),
        }
    }
}

/// The share of resinous trees that have a bead of resin on them.
const RESIN_CHANCE: f32 = 0.12;
/// How high on the trunk it is, in metres: at the height of a hand.
const RESIN_HEIGHT: f64 = 1.25;

/// Whether a tree of `species` weeps resin: the conifers, the junipers, the acacias
/// whose gum the dry country burns, and the great trees of the rainforest, whose resin
/// is copal.
fn weeps_resin(species: Species) -> bool {
    matches!(species, Species::Spruce | Species::TallPine | Species::ScotsPine | Species::Juniper | Species::Acacia | Species::Jungle)
}

/// The resin on `trees` (each a trunk's place and its species) whose tree is within
/// `reach` voxels of `centre`: on a few of the resinous ones, a bead on one side. `bark`
/// gives where the bark is, out from the middle of a trunk whose foot is `foot` at
/// height `y`, along the way `yaw`, if there is a trunk there at that height.
pub fn resin_on(seed: u32, trees: &[(IVec3, Species)], centre: DVec3, reach: f64, mut bark: impl FnMut(IVec3, f64, f32) -> Option<DVec3>) -> Vec<Stick> {
    let mut out = Vec::new();
    for &(foot, species) in trees {
        let (fx, fz) = (foot.x as f64 + 0.5, foot.z as f64 + 0.5);
        if !weeps_resin(species) || (fx - centre.x).hypot(fz - centre.z) > reach {
            continue;
        }
        let roll = |n: i32| unit(hash3(seed ^ 0x2e51_7a, foot.x, n, foot.z));
        if roll(0) >= RESIN_CHANCE {
            continue;
        }
        let yaw = roll(1) * std::f32::consts::TAU;
        let y = foot.y as f64 + m64(RESIN_HEIGHT) * (0.85 + 0.3 * roll(2) as f64);
        if let Some(pos) = bark(foot, y, yaw) {
            out.push(Stick { pos, yaw, count: 1, what: Find::Resin });
        }
    }
    out
}

/// Whether sticks fall from a tree of `species`: not from a palm or a cactus, nor from
/// what is not a tree at all.
fn drops_sticks(species: Species) -> bool {
    crate::lod::stands_tall(species) && !matches!(species, Species::Palm | Species::Cactus)
}

/// The sticks lying under `trees` (each a trunk's place and its species) whose tree is
/// within `reach` voxels of `centre`, across the ground. `ground` gives the height of the
/// land at `(x, z)` as it lies, and the top of what is there to lie on, if there is
/// anything (not water, nor a boulder sat there, nor a trunk). `trees` must hold every tree within [`WOOD`] of those
/// within reach, for each to be told rightly whether it stands in a wood.
pub fn under(seed: u32, trees: &[(IVec3, Species)], centre: DVec3, reach: f64, mut ground: impl FnMut(f64, f64) -> (f64, Option<f64>)) -> Vec<Stick> {
    let wood = m(WOOD as f32) as i64;
    let mut out = Vec::new();
    let mut taken = HashSet::new();
    for &(foot, species) in trees {
        let (fx, fz) = (foot.x as f64 + 0.5, foot.z as f64 + 0.5);
        if !drops_sticks(species) || (fx - centre.x).hypot(fz - centre.z) > reach {
            continue;
        }
        // The more trees about it, the more wood has come down under it.
        let others = trees
            .iter()
            .filter(|(o, s)| *o != foot && drops_sticks(*s) && {
                let (dx, dz) = ((o.x - foot.x) as i64, (o.z - foot.z) as i64);
                dx * dx + dz * dz <= wood * wood
            })
            .count();
        let chance = (0.21 + 0.28 * others as f32).min(0.8);
        // The great trees stand on a wider trunk.
        let girth = if matches!(species, Species::Redwood | Species::ElderOak | Species::Baobab | Species::Kapok) { 1.2 } else { 0.0 };
        // The land the trunk stands on, looked for only once a stick is to lie by it.
        let mut base = None;
        for k in 0..1 + others.min(3) as i32 {
            let roll = |n: i32| unit(hash3(seed ^ 0x571c_c5, foot.x, k * 8 + n, foot.z));
            if roll(0) >= chance {
                continue;
            }
            let (a, r) = (roll(1) * std::f32::consts::TAU, m(1.0 + girth + 2.8 * roll(2)) as f64);
            let (x, z) = (fx + r * a.cos() as f64, fz + r * a.sin() as f64);
            // On the ground the tree stands on, not down a bank or up a rise.
            let at = *base.get_or_insert_with(|| ground(fx, fz).0);
            let Some(y) = ground(x, z).1.filter(|y| (y - at).abs() <= m64(1.6)) else { continue };
            let stick = Stick { pos: DVec3::new(x, y, z), yaw: roll(3) * std::f32::consts::TAU, count: 1 + (roll(4) > 0.6) as u32, what: Find::Sticks };
            // Two that would lie in the same spot are one.
            if taken.insert(stick.spot()) {
                out.push(stick);
            }
        }
    }
    out
}

/// The sticks lying within [`REACH`] of `centre`, from the trees the world grows there.
/// Generates the chunk columns round about that are not held yet, so it is for a
/// worker thread, or a screenshot.
pub fn near(world: &GameWorld, voxels: &mut WorldVoxels, centre: DVec3) -> Vec<Stick> {
    let reach = m64(REACH);
    // Out as far as a tree counted for one within reach could stand.
    let out = ((reach + m64(WOOD)) / CHUNK as f64).ceil() as i32 + 1;
    let (cx, cz) = ((centre.x / CHUNK as f64).floor() as i32, (centre.z / CHUNK as f64).floor() as i32);
    let mut seen = HashSet::new();
    let mut trees = Vec::new();
    for z in cz - out..=cz + out {
        for x in cx - out..=cx + out {
            for p in voxels.trees((x, z)) {
                if seen.insert(p.pos) {
                    trees.push((p.pos, p.species));
                }
            }
        }
    }
    // The land as it lies, and what is on it there: a stick lies on the ground, not on a
    // boulder or the roots of a tree, nor under water.
    let mut sticks = under(world.seed(), &trees, centre, reach, |x, z| {
        let land = (world.column(x.floor() as i32, z.floor() as i32).top() + 1) as f64;
        (land, crate::ground_near(voxels, x, land, z).filter(|y| (y - land).abs() < 1.0))
    });
    // The bark is where the trunk's wood ends, felt for outward from its middle; a
    // tree whose trunk is not there at that height (felled, or thinner than a voxel) has
    // none.
    sticks.extend(resin_on(world.seed(), &trees, centre, reach, |foot, y, yaw| {
        use crate::player::Voxels;
        let (dx, dz) = (yaw.cos() as f64, yaw.sin() as f64);
        let (cx, cz) = (foot.x as f64 + 0.5, foot.z as f64 + 0.5);
        let mut solid = |d: f64| voxels.kind((cx + dx * d).floor() as i32, y.floor() as i32, (cz + dz * d).floor() as i32) == voxel::Kind::Solid;
        if !solid(0.0) {
            return None;
        }
        let d = (1..16).map(|k| k as f64 * 0.25).find(|&d| !solid(d))?;
        Some(DVec3::new(cx + dx * d, y, cz + dz * d))
    }));
    log::debug!("{} sticks under {} trees near {:.0} {:.0}", sticks.len(), trees.len(), centre.x, centre.z);
    for s in &sticks {
        log::trace!("{} sticks at {:.0} {:.0} {:.0}", s.count, s.pos.x, s.pos.y, s.pos.z);
    }
    sticks
}

/// Draws the sticks near enough to `cam` that have not been picked up.
pub fn draw(sticks: &[Stick], pack: &inventory::Pack, cam: DVec3, out: &mut Vec<ParticleInstance>) {
    let limit = out.len() + MOST_BOXES;
    let item = inventory::sticks();
    for s in sticks {
        if out.len() >= limit {
            break;
        }
        let d = s.pos.distance(cam);
        if !pack.gathered(s.spot().0, s.spot().1) && d <= m64(REACH) {
            let from = out.len();
            match s.what {
                Find::Sticks => placed::draw_one(&placed::Thing::new(item.clone(), s.count, s.pos, s.yaw), cam, 0.0, out),
                Find::Resin => placed::draw_resin(s.pos, s.yaw, cam, out),
            }
            dither_since(out, from, dither_edge(d, m64(REACH), m64(4.0)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trees in a square grid `apart` voxels apart, `n` each way, of one species.
    fn grove(n: i32, apart: i32, species: Species) -> Vec<(IVec3, Species)> {
        (0..n * n).map(|i| (IVec3::new((i % n) * apart, 10, (i / n) * apart), species)).collect()
    }

    fn flat(_: f64, _: f64) -> (f64, Option<f64>) {
        (10.0, Some(10.0))
    }

    #[test]
    fn a_wood_has_more_sticks_than_lone_trees_and_always_the_same_ones() {
        let middle = DVec3::new(100.0, 10.0, 100.0);
        let wood = under(7, &grove(20, 10, Species::Oak), middle, 1e9, flat);
        let lone = under(7, &grove(20, 200, Species::Oak), middle, 1e9, flat);
        assert!(wood.len() as f32 / 400.0 > 0.8, "{} under 400 trees in a wood", wood.len());
        assert!((lone.len() as f32 / 400.0) < 0.35, "{} under 400 lone trees", lone.len());
        assert_eq!(wood, under(7, &grove(20, 10, Species::Oak), middle, 1e9, flat));
        assert_ne!(wood, under(8, &grove(20, 10, Species::Oak), middle, 1e9, flat));
        // Each lies a little way out from a trunk, on the ground, in a spot of its own.
        for s in &wood {
            let nearest = grove(20, 10, Species::Oak).iter().map(|(t, _)| (t.x as f64 + 0.5 - s.pos.x).hypot(t.z as f64 + 0.5 - s.pos.z)).fold(f64::MAX, f64::min);
            assert!(nearest <= m64(3.9) && s.pos.y == 10.0 && (1..=2).contains(&s.count));
        }
        assert_eq!(wood.iter().map(Stick::spot).collect::<HashSet<_>>().len(), wood.len());
    }

    #[test]
    fn no_sticks_from_palms_or_cacti_nor_where_there_is_no_ground() {
        let middle = DVec3::new(50.0, 10.0, 50.0);
        assert!(under(1, &grove(10, 10, Species::Palm), middle, 1e9, flat).is_empty());
        assert!(under(1, &grove(10, 10, Species::Cactus), middle, 1e9, flat).is_empty());
        assert!(under(1, &grove(10, 10, Species::Oak), middle, 1e9, |_, _| (10.0, None)).is_empty());
        // Trees on knolls with the ground falling steeply away round them.
        let knolls = |x: f64, z: f64| {
            let y = if x.rem_euclid(10.0) < 1.0 && z.rem_euclid(10.0) < 1.0 { 30.0 } else { 10.0 };
            (y, Some(y))
        };
        assert!(under(1, &grove(10, 10, Species::Oak), middle, 1e9, knolls).is_empty(), "down a bank");
        // Only under the trees within reach.
        let some = under(1, &grove(10, 10, Species::Oak), middle, m64(8.0), flat);
        assert!(!some.is_empty() && some.iter().all(|s| s.pos.distance(middle) < m64(8.0) + m64(3.9)));
    }

    #[test]
    fn a_few_resinous_trees_have_resin_on_the_bark_and_other_trees_none() {
        let middle = DVec3::new(100.0, 10.0, 100.0);
        let on_bark = |foot: IVec3, y: f64, yaw: f32| Some(DVec3::new(foot.x as f64 + 0.5 + 1.5 * yaw.cos() as f64, y, foot.z as f64 + 0.5 + 1.5 * yaw.sin() as f64));
        let pines = resin_on(3, &grove(20, 10, Species::ScotsPine), middle, 1e9, on_bark);
        let share = pines.len() as f32 / 400.0;
        assert!(share > RESIN_CHANCE * 0.6 && share < RESIN_CHANCE * 1.5, "{} of 400 pines", pines.len());
        assert!(pines.iter().all(|s| s.what == Find::Resin && s.item().id == "resin" && s.pos.y > 10.0 + m64(0.9) && s.pos.y < 10.0 + m64(1.7)));
        assert_eq!(pines, resin_on(3, &grove(20, 10, Species::ScotsPine), middle, 1e9, on_bark));
        assert!(resin_on(3, &grove(20, 10, Species::Oak), middle, 1e9, on_bark).is_empty());
        assert!(resin_on(3, &grove(20, 10, Species::Spruce), middle, 1e9, |_, _, _| None).is_empty(), "no bark, no resin");
    }
}
