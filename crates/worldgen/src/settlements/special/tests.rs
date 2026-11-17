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
                let facts = Facts { willows: seed % 2 == 0, stone: seed % 3 == 0, trade: seed % 5 < 2, colour: seed % 7 < 3 };
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
fn the_well_head_and_the_gate_are_not_made_here() {
    for which in [Special::WellHead, Special::Gate] {
        assert!(build(&SpecialSpec::new(which, Region::Temperate, 1), &palette(Region::Temperate, 1)).is_none());
    }
    for which in Special::ALL {
        assert_eq!(MADE.contains(&which), build(&SpecialSpec::new(which, Region::Temperate, 1), &palette(Region::Temperate, 1)).is_some(), "{which:?}");
    }
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
        let all = Facts { willows: true, stone: true, trade: true, colour: true };
        assert_eq!(guilds(&all, 0.1, seed).len(), 2);
        assert_eq!(guilds(&all, 0.9, seed).len(), 3);
        let weavers = Facts { willows: true, ..none };
        assert!(guilds(&weavers, 0.2, seed).iter().all(|g| g.suits(&weavers) || *g == Guild::Merchants || *g == Guild::Painters));
    }
    let spec = SpecialSpec { facts: Facts { willows: true, stone: true, trade: false, colour: false }, wealth: 0.2, ..SpecialSpec::new(Special::GuildHouse, Region::Temperate, 9) };
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
