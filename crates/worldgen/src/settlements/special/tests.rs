use std::collections::HashSet;

use glam::IVec3;
use voxel::AIR;

use super::*;
use crate::{materials as mat, settlements::kit};

/// The palette a building of `region` is seen in: its own country's.
fn palette(region: Region, seed: u32) -> Palette {
    let biome = region.biome();
    Palette::local(biome, mat::bedrock(biome).0).varied(seed)
}

/// A spread of buildings: every kind, in every region, old and new, rich and poor, with
/// and without a stream, and the guild house with every fact.
fn every(seeds: u32) -> Vec<SpecialSpec> {
    let mut out = Vec::new();
    for which in MADE {
        for region in Region::ALL {
            for seed in 0..seeds {
                let h = |k: u32| crate::noise::unit(crate::noise::hash2(seed ^ 0x7e57, which as i32 * 8 + region as i32, k as i32));
                let stream = match seed % 4 {
                    0 => None,
                    1 => Some(IVec3::Z),
                    2 => Some(IVec3::NEG_Z),
                    _ => Some(IVec3::NEG_X),
                };
                let facts = Facts { willows: seed % 2 == 0, stone: seed % 3 == 0, trade: seed % 5 < 2, colour: seed % 7 < 3, water: seed % 3 != 1, ancient: seed % 4 == 0 };
                out.push(SpecialSpec { which, region, age: h(1), wealth: h(2), seed: seed * 7919 + 13, stream, facts });
            }
        }
    }
    out
}

#[test]
fn the_same_spec_builds_the_same_building() {
    for spec in every(2) {
        let pal = palette(spec.region, spec.seed);
        let (a, b) = (build(&spec, &pal).unwrap(), build(&spec, &pal).unwrap());
        assert!(a.building == b.building, "{spec:?}");
        assert_eq!((&a.guest_beds, &a.uses, a.fire, a.wheel, &a.guilds, a.sign), (&b.guest_beds, &b.uses, b.fire, b.wheel, &b.guilds, b.sign));
    }
}

#[test]
fn the_well_head_is_not_made_here() {
    assert!(build(&SpecialSpec::new(Special::WellHead, Region::Temperate, 1), &palette(Region::Temperate, 1)).is_none());
    for which in Special::ALL {
        let made = MADE.contains(&which) || STRUCTURES.contains(&which);
        assert_eq!(made, build(&SpecialSpec::new(which, Region::Temperate, 1), &palette(Region::Temperate, 1)).is_some(), "{which:?}");
    }
}

/// Every gate and gatehouse, of every region and every width, leaves its way through
/// open from end to end and from the ground to `CLEAR` up, is lit, and builds the same
/// each time; nothing in it is built into anything else.
#[test]
fn every_gate_is_open_and_lit() {
    for which in [Special::Gate, Special::Gatehouse] {
        for region in Region::ALL {
            // Enough seeds that every roof's reach and pitch comes up.
            for seed in 0..16u32 {
                for passage in [gate::PASSAGE - 2, gate::PASSAGE, 9, gate::WIDEST] {
                    let spec = SpecialSpec { age: (seed % 4) as f32 / 3.0, wealth: (seed / 4) as f32 / 3.0, ..SpecialSpec::new(which, region, seed * 31 + 7) };
                    let pal = palette(region, spec.seed);
                    let f = frame::Frame::new(&spec, &pal);
                    let (built, faults) = gate::build(f, passage);
                    let name = format!("{which:?} of {region:?}, seed {}, {passage} wide", spec.seed);
                    assert!(faults.is_empty(), "{name}: {faults:?}");
                    let again = super::gate(&spec, &pal, passage).unwrap();
                    assert!(again.building == built.building, "{name}: not the same twice");
                    let b = &built.building;
                    let h = (passage.clamp(gate::PASSAGE - 2, gate::WIDEST) | 1) / 2;
                    assert_eq!(b.door.width, 2 * h + 1, "{name}");
                    let (x0, x1) = (b.footprint.lo.x - 1, b.footprint.hi.x + 1);
                    for &(q, v) in &b.voxels {
                        if v != AIR && q.z.abs() <= h && (x0..=x1).contains(&q.x) && (0..gate::CLEAR).contains(&q.y) {
                            panic!("{name}: {v:?} in the way at {q}");
                        }
                    }
                    for f in &b.fine {
                        let (lo, hi) = (IVec3::from_array(f.lo).div_euclid(IVec3::splat(CELLS)), (IVec3::from_array(f.hi) - 1).div_euclid(IVec3::splat(CELLS)));
                        let way = lo.z <= h && hi.z >= -h && lo.y < gate::CLEAR && hi.y >= 0;
                        assert!(!way, "{name}: a fine part in the way from {lo} to {hi}");
                    }
                    let lamps = b.voxels.iter().filter(|(_, v)| *v == mat::LANTERN).count();
                    assert!(lamps >= 2, "{name}: {lamps} lamps");
                }
            }
        }
    }
}

/// The pavilion is stood under: walked into from its open side, right to the middle.
#[test]
fn the_pavilion_is_stood_under() {
    for region in Region::ALL {
        for seed in 0..3 {
            let spec = SpecialSpec::new(Special::Pavilion, region, seed);
            let (built, faults) = make(&spec, &palette(region, seed)).unwrap();
            assert!(faults.is_empty(), "{region:?}: {faults:?}");
            let w = kit::walk(&built.building);
            assert!(w.ok(), "{region:?}: {:?}", w.problems);
            assert!(w.floor.contains(&IVec3::new(0, 0, 0)), "{region:?}: nobody gets under the middle of it");
        }
    }
}

#[test]
fn the_beacon_burns() {
    for region in Region::ALL {
        let built = build(&SpecialSpec::new(Special::Beacon, region, 3), &palette(region, 3)).unwrap();
        let fire = built.fire.expect("a beacon has a fire");
        assert!(built.building.voxels.contains(&(fire, mat::EMBER)), "{region:?}");
        assert!(fire.y > 10, "{region:?}: its fire is low, at {fire}");
    }
}

/// An old wall keeps out of the columns it is told to, and wears the more the older.
#[test]
fn an_old_wall_keeps_off_what_it_must_and_falls_with_age() {
    let pal = palette(Region::Temperate, 5);
    let path: Vec<glam::IVec2> = (0..200).map(|z| glam::IVec2::new(z / 3, z)).collect();
    let may = |c: glam::IVec2| c.y % 40 >= 4;
    let laid = wall::old_wall(&path, 0, (true, false), |_| 10, may, &pal, 5, 0.6);
    assert!(!laid.voxels.is_empty());
    assert!(laid.voxels.iter().all(|(q, _)| may(glam::IVec2::new(q.x, q.z))));
    let again = wall::old_wall(&path, 0, (true, false), |_| 10, may, &pal, 5, 0.6);
    assert_eq!(laid.voxels, again.voxels);
    let fallen = |age: f32| (0..2000).filter(|&s| wall::fallen(9, age, s)).count();
    assert!(fallen(0.1) < fallen(0.9), "{} fallen young, {} old", fallen(0.1), fallen(0.9));
    // The broken end steps down to the gap.
    let top = |z: i32| laid.voxels.iter().filter(|(q, _)| q.z == z && q.x == z / 3).map(|(q, _)| q.y).max();
    assert!(top(0).unwrap_or(0) <= 12, "{:?}", top(0));
}

/// Every building of every kind and region is walked into through its door, to every
/// bed and every place something is used from, and nothing in it is built into its walls
/// or into anything else.
#[test]
fn every_building_is_walked_into_and_nothing_is_built_into_anything() {
    let mut wrong = Vec::new();
    for spec in every(6) {
        let pal = palette(spec.region, spec.seed);
        let (built, faults) = make(&spec, &pal).unwrap();
        let b = &built.building;
        let w = kit::walk(b);
        let name = format!("{} of {} (seed {}, wealth {:.2})", spec.which.label(), spec.region.word(), spec.seed, spec.wealth);
        for p in &w.problems {
            wrong.push(format!("{name}: {p}"));
        }
        for f in faults {
            wrong.push(format!("{name}: {f}"));
        }
        let floor: HashSet<IVec3> = w.floor.iter().copied().collect();
        for (u, q) in &built.uses {
            if !floor.contains(q) {
                wrong.push(format!("{name}: nobody gets to the {u:?} at {q}"));
            }
        }
        for s in &built.guest_beds {
            assert!(b.inside.beds.contains(s), "{name}: a guest bed that is not among its beds");
        }
    }
    assert!(wrong.is_empty(), "{} things wrong:\n{}", wrong.len(), wrong.join("\n"));
}

#[test]
fn a_tavern_has_three_guest_beds_at_least_and_the_door_leads_to_each() {
    for spec in every(6).into_iter().filter(|s| s.which == Special::Tavern) {
        let built = build(&spec, &palette(spec.region, spec.seed)).unwrap();
        assert!((3..=6).contains(&built.guest_beds.len()), "{} guest beds in {spec:?}", built.guest_beds.len());
        let w = kit::walk(&built.building);
        assert!(w.cut_off.is_empty(), "{spec:?}: {:?}", w.problems);
        // Each traveller lies on the floor under their bed, inside the walls.
        for p in built.guest_beds_at(IVec3::ZERO, 0) {
            let q = p.floor().as_ivec3();
            assert!(w.floor.contains(&(q - IVec3::Y)), "{spec:?}: a bed at {p} is not over a floor walked on");
        }
    }
}

#[test]
fn guest_beds_turn_with_the_building() {
    let spec = SpecialSpec::new(Special::Tavern, Region::Temperate, 5);
    let built = build(&spec, &palette(spec.region, spec.seed)).unwrap();
    let origin = IVec3::new(100, 20, -40);
    for turns in 0..4 {
        let turned = built.turned(turns);
        let at = built.guest_beds_at(origin, turns);
        assert_eq!(at.len(), turned.guest_beds.len());
        for (p, s) in at.iter().zip(&turned.guest_beds) {
            // In the turned building, the point lies over one of the bed's own cells.
            let q = (p.floor().as_ivec3() - origin).with_y(s.at.y);
            assert!(s.cells().any(|c| c == q), "turns {turns}: {p} not over {s:?}");
        }
    }
}

#[test]
fn the_smiths_forge_is_alight() {
    for spec in every(4).into_iter().filter(|s| s.which == Special::Smithy) {
        let built = build(&spec, &palette(spec.region, spec.seed)).unwrap();
        let fire = built.fire.unwrap_or_else(|| panic!("{spec:?} has no fire"));
        assert_eq!(built.building.voxels.iter().find(|(q, _)| *q == fire).map(|p| p.1), Some(mat::EMBER), "{spec:?}");
        assert!(built.uses.iter().any(|(u, _)| *u == Use::Forge), "{spec:?}");
        // The smoke comes out above the roof.
        let top = built.building.voxels.iter().filter(|(_, v)| *v != AIR).map(|(q, _)| q.y).max().unwrap();
        assert!(built.building.smoke.y > top - 3, "{spec:?}: smoke at {} under a top of {top}", built.building.smoke);
    }
}

#[test]
fn every_building_keeps_to_its_plot() {
    for spec in every(6) {
        let built = build(&spec, &palette(spec.region, spec.seed)).unwrap();
        let fp = built.building.footprint;
        let size = fp.hi - fp.lo + glam::IVec2::ONE;
        let (x, z) = largest(spec.which);
        assert!(size.x <= x && size.y <= z, "{spec:?}: {size} over {x} by {z}");
        // And it is walled where it says.
        let w = built.building.walls;
        assert!(w.lo.x >= fp.lo.x && w.hi.x <= fp.hi.x && w.lo.y >= fp.lo.y && w.hi.y <= fp.hi.y, "{spec:?}");
    }
}

#[test]
fn a_desert_tavern_is_not_a_northern_one() {
    let t = |r: Region| build(&SpecialSpec::new(Special::Tavern, r, 3), &palette(r, 3)).unwrap().building.style;
    let (desert, north) = (t(Region::Desert), t(Region::Northern));
    assert_ne!(desert.walls, north.walls);
    assert_ne!(desert.roof, north.roof);
    assert_eq!(desert.shape, kit::RoofShape::Flat);
}

#[test]
fn guild_houses_take_the_guilds_their_place_suits() {
    let none = Facts::default();
    for seed in 0..40 {
        let g = guilds(&none, 0.9, seed);
        assert!((2..=3).contains(&g.len()));
        assert!(g.contains(&Guild::Wayfarers), "wayfarers are everywhere");
        assert!(!g.contains(&Guild::Weavers) && !g.contains(&Guild::Masons));
        let all = Facts { willows: true, stone: true, trade: true, colour: true, water: true, ancient: true };
        assert_eq!(guilds(&all, 0.1, seed).len(), 2);
        assert_eq!(guilds(&all, 0.9, seed).len(), 3);
        let weavers = Facts { willows: true, ..none };
        assert!(guilds(&weavers, 0.2, seed).iter().all(|g| g.suits(&weavers) || *g == Guild::Merchants || *g == Guild::Painters));
        // An ancient place keeps masons for its old walls, on soft ground or stony, poor or
        // rich, however many other guilds suit it.
        for wealth in [0.1, 0.9] {
            assert!(guilds(&Facts { ancient: true, ..none }, wealth, seed).contains(&Guild::Masons));
            assert!(guilds(&all, wealth, seed).contains(&Guild::Masons));
        }
        // And a place neither ancient nor on stone has none.
        assert!(!guilds(&Facts { ancient: false, stone: false, ..all }, 0.9, seed).contains(&Guild::Masons));
    }
    let spec = SpecialSpec { facts: Facts { willows: true, stone: true, trade: false, colour: false, water: false, ancient: false }, wealth: 0.2, ..SpecialSpec::new(Special::GuildHouse, Region::Temperate, 9) };
    let built = build(&spec, &palette(spec.region, spec.seed)).unwrap();
    assert_eq!(built.guilds, guilds(&spec.facts, spec.wealth, spec.seed));
}

/// A copy to edit builds back into what was copied, door, beds and all.
#[test]
fn a_copy_builds_what_was_copied() {
    for which in MADE {
        let spec = SpecialSpec::new(which, Region::Upland, 4);
        let pal = palette(spec.region, spec.seed);
        let d = copy(&spec, &pal, "test").unwrap();
        assert_eq!(d.kind, Kind::Special(which));
        let back = kit::from_design(&d, &pal, spec.seed).unwrap();
        let built = build(&spec, &pal).unwrap().building;
        let solid = |b: &Building| -> HashSet<(IVec3, voxel::Voxel)> { b.voxels.iter().copied().filter(|(_, v)| *v != AIR).collect() };
        assert_eq!(solid(&back), solid(&built), "{which:?}");
        assert_eq!(back.inside.beds, built.inside.beds);
        assert!(kit::walk(&back).ok(), "{which:?}");
        // It saves and reads back.
        assert_eq!(Design::parse(&d.to_text()).unwrap(), d);
    }
}
