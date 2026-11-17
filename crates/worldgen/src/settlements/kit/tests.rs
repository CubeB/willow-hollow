use std::collections::{HashMap, HashSet, VecDeque};

use glam::{IVec2, IVec3};
use voxel::{AIR, Kind, Materials, Voxel, mesh::CELLS, plant};

use super::{rules::Rules, *};
use crate::{
    materials as mat,
    noise::{hash2, unit},
};

/// The palette a region's homes take where it stands on its usual rock.
pub(crate) fn palette(region: Region) -> Palette {
    let biome = region.biome();
    Palette::local(biome, mat::bedrock(biome).0)
}

/// A spread of homes: the corners of age and wealth first, then whatever the seed says,
/// asking for anything from nobody to more than the step sleeps.
pub(crate) fn spec(region: Region, step: Step, seed: u32) -> Spec {
    let h = |k: i32| unit(hash2(seed ^ 0x51ec_0b1d, k, step as i32 * 8 + region as i32));
    let (age, wealth) = match seed {
        0 => (0.0, 0.0),
        1 => (1.0, 1.0),
        2 => (0.0, 1.0),
        3 => (1.0, 0.0),
        _ => (h(1), h(2)),
    };
    let sleepers = if seed < 4 { step.max_sleepers() } else { (h(3) * (step.max_sleepers() as f32 + 3.0)) as u8 };
    Spec { region, step, age, wealth, sleepers, seed }
}

pub(crate) const SEEDS: u32 = 24;

fn every_home() -> impl Iterator<Item = (Spec, Building)> {
    Region::ALL.into_iter().flat_map(|region| {
        let pal = palette(region);
        Step::ALL.into_iter().flat_map(move |step| {
            (0..SEEDS).map(move |seed| {
                let s = spec(region, step, seed);
                (s, build(&s, &pal))
            })
        })
    })
}

pub(crate) fn name(s: &Spec) -> String {
    format!("{} {} seed {} (age {:.2}, wealth {:.2}, {} sleepers)", s.region.word(), s.step.word(), s.seed, s.age, s.wealth, s.sleepers)
}

/// The building's voxels by place, standing on flat ground: below `y = 0` is earth
/// wherever the building says nothing.
pub(crate) struct Site {
    pub(crate) at: HashMap<IVec3, Voxel>,
}

impl Site {
    pub(crate) fn new(b: &Building) -> Self {
        Site { at: b.voxels.iter().copied().collect() }
    }

    pub(crate) fn solid(&self, q: IVec3) -> bool {
        match self.at.get(&q) {
            Some(&v) => v != AIR,
            None => q.y < 0,
        }
    }
}

/// The player's body in voxels: 0.512 m wide is two columns, 1.76 m tall is five layers.
const BODY: (i32, i32) = (2, 5);
/// How high a person steps up without jumping, and how far they drop without care.
const STEP_UP: i32 = 2;
const DROP: i32 = 3;

/// Every floor voxel a person walking in from outside the door stands over.
pub(crate) fn walked(b: &Building, site: &Site) -> HashSet<IVec3> {
    let fits = |p: IVec3| (0..BODY.1).all(|y| (0..BODY.0).all(|z| (0..BODY.0).all(|x| !site.solid(p + IVec3::new(x, y, z)))));
    let stands = |p: IVec3| (0..BODY.0).any(|z| (0..BODY.0).any(|x| site.solid(p + IVec3::new(x, -1, z))));
    let top = b.voxels.iter().map(|(q, _)| q.y).max().unwrap();
    let (lo, hi) = (b.footprint.lo - IVec2::splat(4), b.footprint.hi + IVec2::splat(4));
    let inside = |p: IVec3| p.x >= lo.x && p.z >= lo.y && p.x <= hi.x && p.z <= hi.y && p.y >= -2 && p.y <= top + 2;
    let start = IVec3::new(hi.x - 1, 0, b.door.floor.z);
    assert!(fits(start) && stands(start), "the ground before the door is open");
    let mut seen = HashSet::from([start]);
    let mut queue = VecDeque::from([start]);
    let mut floor = HashSet::new();
    while let Some(p) = queue.pop_front() {
        for x in 0..BODY.0 {
            for z in 0..BODY.0 {
                floor.insert(p + IVec3::new(x, -1, z));
            }
        }
        for d in [IVec3::X, IVec3::NEG_X, IVec3::Z, IVec3::NEG_Z] {
            for dy in -DROP..=STEP_UP {
                let q = p + d + IVec3::Y * dy;
                if !inside(q) || seen.contains(&q) || !fits(q) || !stands(q) {
                    continue;
                }
                // Room overhead to rise before stepping across, or to step across before
                // dropping.
                let between = if dy > 0 { p + IVec3::Y * dy } else { p + d };
                if !fits(between) {
                    continue;
                }
                seen.insert(q);
                queue.push_back(q);
            }
        }
    }
    floor
}

#[test]
fn every_kit_file_reads() {
    assert!(rules::problems().is_empty(), "kit files that could not be read: {:?}", rules::problems());
    for region in Region::ALL {
        assert_eq!(rules::rules(region).region, region);
        assert!(!rules::rules(region).about.is_empty(), "{} says what it is", region.word());
    }
}

#[test]
fn a_wrong_line_is_named() {
    let text = "hut:\n  width: 9 12\ntimber:\n  walls: cob, straw\nstone:\n";
    let err = Rules::parse(Region::Temperate, text).unwrap_err();
    assert!(err.starts_with("line 4:") && err.contains("straw"), "{err}");
    let err = Rules::parse(Region::Temperate, "hut:\ntimber:\n").unwrap_err();
    assert!(err.contains("stone"), "{err}");
    let err = Rules::parse(Region::Temperate, "hut:\n  colour: red\ntimber:\nstone:\n").unwrap_err();
    assert!(err.starts_with("line 2:"), "{err}");
    let ok = Rules::parse(Region::Temperate, "hut:\n  walls: cob 3, wattle 2 poor\n  lean-to: 0.3 rich\ntimber:\nstone:\ncomponents:\n  door: porch 2 rich, nothing\n").unwrap();
    assert_eq!(ok.step(Step::Hut).walls.len(), 2);
    assert_eq!(ok.components(AnchorKind::Door).len(), 2);
}

#[test]
fn every_region_has_three_kinds_of_home() {
    // A region's huts, timber houses and stone houses are told apart by what they are made
    // of, whatever the seed.
    for region in Region::ALL {
        let pal = palette(region);
        let styles: Vec<HashSet<(WallKind, Cover)>> = Step::ALL
            .iter()
            .map(|&step| (0..SEEDS).map(|seed| build(&spec(region, step, seed), &pal).style).map(|s| (s.walls, s.roof)).collect())
            .collect();
        assert!(styles[0] != styles[2], "{}: huts and stone houses differ", region.word());
    }
}

#[test]
fn people_walk_in_to_every_floor_voxel() {
    for (s, b) in every_home() {
        let site = Site::new(&b);
        let reached = walked(&b, &site);
        let missed: Vec<&IVec3> = b.inside.floor.iter().filter(|f| !reached.contains(f)).collect();
        assert!(missed.is_empty(), "{}: {} of {} floor voxels out of reach, e.g. {:?} ({})", name(&s), missed.len(), b.inside.floor.len(), &missed[..missed.len().min(4)], b.style.describe());
        assert!(!b.inside.floor.is_empty(), "{}: a floor", name(&s));
        // The doorway is as the building says.
        assert!(b.door.width >= 2 && b.door.height >= 5, "{}: a doorway a person goes through", name(&s));
        assert!(site.solid(b.door.floor), "{}: the door's floor is underfoot", name(&s));
        for y in 1..=b.door.height {
            assert!(!site.solid(b.door.floor + IVec3::Y * y), "{}: the doorway is open", name(&s));
        }
    }
}

#[test]
fn everyone_has_a_bed() {
    for (s, b) in every_home() {
        let want = s.sleepers.min(s.step.max_sleepers()) as usize;
        assert_eq!(b.inside.beds.len(), want, "{}: a bed for each sleeper", name(&s));
        let floor: HashSet<IVec3> = b.inside.floor.iter().copied().collect();
        let mut taken = HashSet::new();
        for spot in b.inside.beds.iter().chain(&b.inside.furniture) {
            for c in spot.cells() {
                assert!(floor.contains(&(c - IVec3::Y)), "{}: {spot:?} stands on the floor", name(&s));
                assert!(taken.insert(c), "{}: spots overlap at {c}", name(&s));
            }
        }
        let hearth = b.inside.hearth.unwrap_or_else(|| panic!("{}: a hearth", name(&s)));
        // A central hearth is on the floor; a fireplace is in a wall, opening onto it.
        assert_eq!(hearth.central, floor.contains(&(hearth.at - IVec3::Y)), "{}: {hearth:?}", name(&s));
        if !hearth.central {
            assert!(floor.contains(&(hearth.at + hearth.facing - IVec3::Y)), "{}: the fireplace opens onto the floor", name(&s));
        }
    }
}

#[test]
fn nothing_floats() {
    for (s, b) in every_home() {
        let site = Site::new(&b);
        let solid: Vec<IVec3> = b.voxels.iter().filter(|(_, v)| *v != AIR).map(|(q, _)| *q).collect();
        let set: HashSet<IVec3> = solid.iter().copied().collect();
        // From what is in the ground, or stands on it, through everything touching.
        let mut held: HashSet<IVec3> = solid.iter().copied().filter(|q| q.y < 0 || (q.y == 0 && site.solid(*q - IVec3::Y))).collect();
        let mut queue: VecDeque<IVec3> = held.iter().copied().collect();
        while let Some(q) = queue.pop_front() {
            for d in [IVec3::X, IVec3::NEG_X, IVec3::Y, IVec3::NEG_Y, IVec3::Z, IVec3::NEG_Z] {
                let n = q + d;
                if set.contains(&n) && held.insert(n) {
                    queue.push_back(n);
                }
            }
        }
        let loose: Vec<&IVec3> = solid.iter().filter(|q| !held.contains(q)).collect();
        assert!(loose.is_empty(), "{}: {} voxels hang in the air, e.g. {:?} ({})", name(&s), loose.len(), &loose[..loose.len().min(6)], b.style.describe());
        // Every fine part touches something solid, or a part that does (a ledge on a door).
        let c = CELLS;
        let on_voxel = |part: &PlantBox| {
            let lo = IVec3::from(part.lo.map(|v| (v - 1).div_euclid(c)));
            let hi = IVec3::from(part.hi.map(|v| v.div_euclid(c)));
            (lo.y..=hi.y).any(|y| (lo.z..=hi.z).any(|z| (lo.x..=hi.x).any(|x| site.solid(IVec3::new(x, y, z)))))
        };
        let touch = |a: &PlantBox, b: &PlantBox| (0..3).all(|i| a.lo[i] <= b.hi[i] && b.lo[i] <= a.hi[i]);
        let mut fixed: Vec<bool> = b.fine.iter().map(on_voxel).collect();
        for _ in 0..3 {
            for i in 0..b.fine.len() {
                if !fixed[i] && (0..b.fine.len()).any(|j| fixed[j] && touch(&b.fine[i], &b.fine[j])) {
                    fixed[i] = true;
                }
            }
        }
        for (part, ok) in b.fine.iter().zip(fixed) {
            assert!(ok, "{}: a fine part {part:?} touches nothing", name(&s));
        }
        // And footings go into the ground under every wall.
        let lowest = b.voxels.iter().filter(|(_, v)| *v != AIR).map(|(q, _)| q.y).min().unwrap();
        assert!(lowest < 0, "{}: footings", name(&s));
    }
}

/// Whether snow falling past voxel `v` stops on it, as the voxel mesher decides.
fn keeps_snow_off(mats: &Materials, v: Voxel) -> bool {
    let m = mats.get(v);
    m.kind != Kind::Air && !matches!(m.plant, plant::DECIDUOUS | plant::AUTUMN | plant::VINE)
}

#[test]
fn under_the_roof_no_snow_lies_and_no_moss_grows() {
    let mats = mat::build();
    // features::moss looks this far up for a roof, from the air beside a surface.
    let reach = 20;
    let roofing = |v: Voxel| matches!(v, mat::THATCH | mat::SLATE | mat::ROOF_TILE | mat::PLANK | mat::PLANK_PALE | mat::PLANK_DARK | mat::PLANK_RED) || mat::ROOFING.contains(&v);
    for (s, b) in every_home() {
        let site = Site::new(&b);
        let top = b.voxels.iter().map(|(q, _)| q.y).max().unwrap();
        for f in &b.inside.floor {
            let above = |y: i32| site.at.get(&IVec3::new(f.x, y, f.z)).copied().unwrap_or(AIR);
            assert!((f.y + 1..=top).any(|y| keeps_snow_off(&mats, above(y))), "{}: snow falls on the floor at {f}", name(&s));
            let feet = f.y + 1;
            assert!((feet + 1..=feet + reach).any(|y| roofing(above(y))), "{}: moss would take the floor at {f} for outdoors", name(&s));
        }
    }
}

#[test]
fn the_same_spec_builds_the_same_home() {
    for region in Region::ALL {
        let pal = palette(region);
        for step in Step::ALL {
            for seed in [5, 77] {
                let s = spec(region, step, seed);
                assert_eq!(build(&s, &pal), build(&s, &pal), "{}", name(&s));
            }
        }
    }
    // And different seeds build different homes.
    let pal = palette(Region::Temperate);
    let a = build(&spec(Region::Temperate, Step::Timber, 5), &pal);
    let b = build(&spec(Region::Temperate, Step::Timber, 6), &pal);
    assert_ne!(a.voxels, b.voxels);
}

#[test]
fn a_turned_home_is_the_same_home() {
    let pal = palette(Region::Desert);
    let b = build(&spec(Region::Desert, Step::Stone, 9), &pal);
    for turns in 1..4 {
        let t = b.turned(turns);
        assert_eq!(t.voxels.len(), b.voxels.len());
        assert_eq!(t.door.facing, rotate(IVec3::X, turns));
        assert_eq!(t.inside.floor.len(), b.inside.floor.len());
        assert_eq!(t.turned(4 - turns), b, "turning all the way round gives it back");
    }
}

#[test]
fn anchors_are_in_the_open() {
    for (s, b) in every_home() {
        let site = Site::new(&b);
        for a in &b.anchors {
            assert!(!site.solid(a.at), "{}: {a:?} is inside something", name(&s));
        }
        for kind in [AnchorKind::Door, AnchorKind::RoofTop, AnchorKind::Yard] {
            assert!(b.anchors.iter().any(|a| a.kind == kind), "{}: a {} anchor", name(&s), kind.word());
        }
    }
}

/// Prints a home in slices, for looking into a failure: `KIT_SHOW="alpine timber 7"
/// cargo test ... kit::tests::show -- --ignored --nocapture`. `#` is solid, `.` air the
/// building clears, `~` the ground, `b` a bed, `h` the hearth, `D` the door's floor.
#[test]
#[ignore]
fn show() {
    let Ok(want) = std::env::var("KIT_SHOW") else { return };
    let words: Vec<&str> = want.split_whitespace().collect();
    let region = Region::ALL.into_iter().find(|r| r.word() == words[0]).expect("a region");
    let step = Step::ALL.into_iter().find(|s| s.word() == words[1]).expect("a step");
    let s = spec(region, step, words[2].parse().unwrap());
    let b = build(&s, &palette(region));
    let site = Site::new(&b);
    println!("{}: {}  door {:?}", name(&s), b.style.describe(), b.door);
    let (lo, hi) = (b.footprint.lo - IVec2::splat(2), b.footprint.hi + IVec2::splat(2));
    let top = b.voxels.iter().map(|(q, _)| q.y).max().unwrap();
    let ch = |q: IVec3| {
        if q == b.door.floor {
            'D'
        } else if b.inside.beds.iter().any(|s| s.cells().any(|c| c == q)) {
            'b'
        } else if b.inside.hearth.is_some_and(|h| h.at == q) {
            'h'
        } else {
            match site.at.get(&q) {
                Some(&AIR) => '.',
                Some(_) => '#',
                None if q.y < 0 => '~',
                None => ' ',
            }
        }
    };
    let fy = b.inside.floor.first().map_or(0, |f| f.y);
    let extra: Vec<i32> = std::env::var("KIT_Y").ok().map(|v| v.split_whitespace().filter_map(|w| w.parse().ok()).collect()).unwrap_or_default();
    for y in [fy, fy + 1].into_iter().chain(extra) {
        println!("plan at y {y} (x across, z down from {}):", lo.y);
        for z in lo.y..=hi.y {
            println!("{z:3} {}", (lo.x..=hi.x).map(|x| ch(IVec3::new(x, y, z))).collect::<String>());
        }
    }
    let z = std::env::var("KIT_Z").ok().and_then(|v| v.parse().ok()).unwrap_or(b.door.floor.z);
    println!("section along x at z {z} (x across from {}, y up):", lo.x);
    for y in (-4..=top).rev() {
        println!("{y:3} {}", (lo.x..=hi.x).map(|x| ch(IVec3::new(x, y, z))).collect::<String>());
    }
    let reached = walked(&b, &site);
    println!("reached {} of {} floor voxels", b.inside.floor.iter().filter(|f| reached.contains(f)).count(), b.inside.floor.len());
}
