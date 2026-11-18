use std::collections::HashSet;

use glam::IVec3;

use super::*;
use crate::settlements::kit::{
    self, rules,
    tests::{Site as Ground, palette, spec, walked},
};

/// The bare homes components are tried on: every region and step, poorest, middling and
/// richest, a few of each.
fn bare_homes() -> Vec<(Spec, Palette, Building)> {
    let mut out = Vec::new();
    for region in Region::ALL {
        let pal = palette(region);
        for step in Step::ALL {
            for wealth in [0.0, 0.5, 1.0] {
                for seed in 4..7 {
                    let s = Spec { wealth, ..spec(region, step, seed) };
                    out.push((s, pal, kit::generate_bare(&s, &pal, rules::rules(region))));
                }
            }
        }
    }
    out
}

/// Every component, tried at every anchor it takes on homes of every region, makes only
/// what fits whole there; each goes in somewhere, and somewhere in every region whose
/// kit file names it; and new or let go, every one of its parts is held.
#[test]
fn every_component_builds_for_every_region() {
    let homes = bare_homes();
    let sites: Vec<Site> = homes.iter().map(|(_, _, b)| Site::new(b)).collect();
    let mut never = Vec::new();
    for &c in Component::ALL {
        let mut placed: HashSet<Region> = HashSet::new();
        for ((s, pal, b), site) in homes.iter().zip(&sites) {
            for (k, a) in b.anchors.iter().filter(|a| c.takes(a.kind)).enumerate() {
                let req = Request {
                    anchor: *a,
                    region: s.region,
                    step: s.step,
                    style: b.style,
                    pal,
                    wealth: s.wealth,
                    age: s.age,
                    decayed: k % 2 == 1,
                    great_town: true,
                    seed: s.seed ^ (k as u32) << 8,
                };
                if let Some(m) = generate(c, &req, b, site) {
                    assert!(site.fits(&m, a.kind == AnchorKind::Door), "{} at {a:?}", c.word());
                    placed.insert(s.region);
                }
            }
        }
        for r in c.regions() {
            if !placed.contains(&r) {
                never.push(format!("{} in {}", c.word(), r.word()));
            }
        }
    }
    assert!(never.is_empty(), "never go on a home of a region whose kit names them: {never:?}");
}

/// Every component appears in some region's kit file, and every kit file names some
/// component for most kinds of anchor its homes have.
#[test]
fn every_region_names_its_components() {
    for &c in Component::ALL {
        assert!(!c.regions().is_empty(), "no region's kit names {}", c.word());
    }
    for region in Region::ALL {
        let r = rules::rules(region);
        let named = r.components.iter().filter(|(_, list)| list.iter().any(|c| c.what != "nothing")).count();
        assert!(named >= 4, "{} names components for only {named} kinds of anchor", region.word());
    }
    // And a name in the wrong place is a fault in the file.
    let err = Rules::parse(Region::Temperate, "hut:\ntimber:\nstone:\ncomponents:\n  window: porch\n").unwrap_err();
    assert!(err.starts_with("line 5:") && err.contains("porch"), "{err}");
}

/// Homes dressed in every region: the same each time, their components where their
/// anchors were, the door's way in still walked through to every floor voxel, and
/// furnished as well.
#[test]
fn dressed_homes_are_walked_into_and_furnished() {
    use crate::settlements::{Livelihood, furnish};
    let mut kinds: HashSet<Component> = HashSet::new();
    for region in Region::ALL {
        let pal = palette(region);
        for step in Step::ALL {
            for seed in 0..12 {
                let s = spec(region, step, seed);
                let how = Dress { great_town: seed % 3 == 0 };
                let (b, attached) = kit::generate_dressed(&s, &pal, rules::rules(region), &how);
                let (again, twice) = kit::generate_dressed(&s, &pal, rules::rules(region), &how);
                assert!(b == again && attached == twice, "{}: the same each time", kit::tests::name(&s));
                kinds.extend(attached.iter().map(|a| a.component));
                for a in &attached {
                    assert!(!b.anchors.contains(&a.anchor), "{}: {:?} is taken", kit::tests::name(&s), a.anchor);
                    assert!(a.component != Component::WindCatcher || how.great_town, "wind catchers stand in great towns only");
                }
                let ground = Ground::new(&b);
                let reached = walked(&b, &ground);
                let missed = b.inside.floor.iter().filter(|f| !reached.contains(f)).count();
                assert_eq!(missed, 0, "{}: floor out of reach with {:?}", kit::tests::name(&s), attached.iter().map(|a| a.component.word()).collect::<Vec<_>>());
                let household = furnish::Household { region, sleepers: s.sleepers, children: 0, trade: (seed % 2 == 0).then_some(Livelihood::ALL[seed as usize % Livelihood::ALL.len()]), wealth: s.wealth, neglect: s.age, lived_in: true };
                let f = furnish::furnish(&b, &household, &pal, s.seed);
                let mut with = b.clone();
                with.voxels.extend(f.voxels.iter().copied());
                let ground = Ground::new(&with);
                let reached = walked(&with, &ground);
                assert!(b.inside.beds.iter().all(|bed| bed.cells().any(|c| reached.contains(&(c - IVec3::Y)) || reached.contains(&(c + bed.facing * bed.wide - IVec3::Y)))), "{}: furnished, every bed is reached", kit::tests::name(&s));
            }
        }
    }
    assert!(kinds.len() >= Component::ALL.len() * 2 / 3, "only {} kinds of component went on the homes", kinds.len());
}

/// A component saved from the editor reads back as it was written, is chosen against
/// the generator by weight, and is built where it fits, turned to its anchor.
#[test]
fn a_saved_component_reads_and_builds() {
    let pal = palette(Region::Temperate);
    let d = copy(Component::WaterJar, 3, "Two jars", |r, v| palette(r).varied(v)).expect("a water jar goes somewhere");
    assert_eq!(d.kind, designs::Kind::Component(Component::WaterJar));
    let text = d.to_text();
    assert!(text.contains("component: water-jar"), "{text}");
    let back = Design::parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    assert_eq!(back, d);
    // Cloth keeps its flap through a file.
    let awning = copy(Component::Awning, 1, "Striped", |r, v| palette(r).varied(v)).expect("an awning goes somewhere");
    assert!(awning.fine.iter().any(|f| f.flap != 0), "an awning is cloth");
    assert_eq!(Design::parse(&awning.to_text()).unwrap(), awning);
    // In a folder beside a generator weight of 0, the saved one is always chosen.
    let files = [("component/two-jars.txt", text.as_str()), ("component/generator.txt", "weight: 0\n")];
    let (lib, problems) = designs::gather(files.iter().copied());
    assert!(problems.is_empty(), "{problems:?}");
    let kind = designs::Kind::Component(Component::WaterJar);
    assert_eq!(lib[&kind].generator_weight(), 0);
    assert!(lib[&kind].choose(kind, 7).is_some());
    // Built on a stand, it is what was saved.
    let s = stand(Component::WaterJar, 3, Some(&back), |r, v| palette(r).varied(v)).expect("it fits where it was drawn");
    assert_eq!(s.made.voxels.len(), d.cells.len());
    // A file saying it is a component but not which is a fault.
    assert!(Design::parse("kind: component\nname: what\n").is_err());
    let _ = pal;
}

/// What the public spaces get: a sail shade across a lane, an awning over a stall with
/// its fringe, lanterns on a line, a lantern under a branch, a boardwalk, and a jar set
/// on its own.
#[test]
fn public_spaces_are_dressed() {
    use crate::features::fittings;
    let pal = palette(Region::Desert);
    let shade = public::lane_shade(IVec3::new(0, 7, 0), IVec3::new(6, 7, 0), 3, 0.8, 0.2, 5);
    assert!(!shade.is_empty() && shade.iter().all(|p| p.cloth() != voxel::mesh::Cloth::None));
    let stall = fittings::build(Fitting::MarketStall, &pal, 3, false);
    let awning = public::stall_awning(&stall.hooks, 0.9, 0.1, 3);
    assert!(awning.len() > 4, "an awning and its fringe");
    let line = public::strung(Hook::Line { from: IVec3::new(0, 20, 2), to: IVec3::new(40, 20, 2), sag: 3 }, public::Strung::Lanterns { lit: true }, 1);
    assert!(line.iter().any(|p| p.voxel == crate::materials::LANTERN));
    assert_eq!(public::branch_lantern(IVec3::new(4, 60, 4), 10, &pal, true).len(), 4);
    let (boards, posts) = public::boardwalk(glam::IVec2::new(0, 0), glam::IVec2::new(8, 0), 2, 1, |_, _| -1, &pal, 0.7, 2);
    assert_eq!(boards.iter().filter(|(q, _)| q.y == 1).count(), 9 * 2);
    assert!(!posts.is_empty());
    let jar = public::standalone(Component::WaterJar, IVec3::new(10, 3, 10), IVec3::NEG_Z, Region::Desert, &pal, 0.5, 0.2, 9).expect("a jar stands anywhere");
    assert!(jar.voxels.iter().all(|(q, _)| q.y >= 3));
    assert!(public::standalone(Component::WindowBox, IVec3::ZERO, IVec3::X, Region::Temperate, &pal, 0.5, 0.2, 9).is_none(), "no window, no window box");
}

use crate::features::fittings::Hook;

/// Traces one component on the bare homes of one region, for looking into a failure:
/// `COMPONENT_SHOW="carved-posts rainforest" cargo test ... components::tests::trace --
/// --ignored --nocapture`.
#[test]
#[ignore]
fn trace() {
    let Ok(want) = std::env::var("COMPONENT_SHOW") else { return };
    let words: Vec<&str> = want.split_whitespace().collect();
    let c = Component::by_word(words[0]).expect("a component");
    let region = Region::ALL.into_iter().find(|r| r.word() == words[1]).expect("a region");
    for (s, pal, b) in bare_homes().into_iter().filter(|(s, _, _)| s.region == region) {
        let site = Site::new(&b);
        for a in b.anchors.iter().filter(|a| c.takes(a.kind)) {
            let req = Request { anchor: *a, region, step: s.step, style: b.style, pal: &pal, wealth: s.wealth, age: s.age, decayed: false, great_town: true, seed: 3 };
            let mut cx = Cx::new(&req, &b, &site);
            let door = cx.doorway();
            let stands: Vec<Option<i32>> = (0..8).map(|x| cx.stands(x, -3, 0, (-cx.ground()).max(1))).collect();
            let r = build_one(c, &mut cx);
            let m = cx.finish();
            let own: HashSet<IVec3> = m.voxels.iter().map(|(q, _)| *q).collect();
            eprintln!("{} {a:?} door {door:?} ground {} stands at z -3 {stands:?}: built {r:?}, {} voxels, {} fine, fits {}, held {}", s.step.word(), -a.at.y, m.voxels.len(), m.fine.len(), site.fits(&m, a.kind == AnchorKind::Door), site.held(&m, &own));
        }
    }
}

/// What components add to a home, region by region: how many go on, and the voxels and
/// fine parts they bring, against the bare home's. `cargo test ...
/// components::tests::what_they_add -- --ignored --nocapture`.
#[test]
#[ignore]
fn what_they_add() {
    for region in Region::ALL {
        let (mut n, mut on, mut voxels, mut fine, mut bare_v, mut bare_f, mut most) = (0, 0, 0i64, 0i64, 0, 0, 0i64);
        for (s, pal, bare) in bare_homes().into_iter().filter(|(s, _, _)| s.region == region) {
            let (b, attached) = kit::generate_dressed(&s, &pal, rules::rules(region), &Dress::default());
            n += 1;
            on += attached.len();
            voxels += b.voxels.len() as i64 - bare.voxels.len() as i64;
            fine += b.fine.len() as i64 - bare.fine.len() as i64;
            most = most.max(b.fine.len() as i64 - bare.fine.len() as i64);
            bare_v += bare.voxels.len();
            bare_f += bare.fine.len();
        }
        let n = n as f32;
        println!(
            "{:10} {:4.1} on a home, {:+5.1} voxels on {:6.1}, {:+5.1} fine parts on {:5.1} (most {most})",
            region.word(),
            on as f32 / n,
            voxels as f32 / n,
            bare_v as f32 / n,
            fine as f32 / n,
            bare_f as f32 / n
        );
    }
    for &c in Component::ALL {
        let made: Vec<Made> = (0..8).filter_map(|v| stand(c, v, None, |r, _| palette(r)).map(|s| s.made)).collect();
        let n = made.len().max(1) as f32;
        let (v, f) = made.iter().fold((0, 0), |(v, f), m| (v + m.voxels.len(), f + m.fine.len()));
        println!("{:18} {:5.1} voxels {:6.1} fine parts", c.word(), v as f32 / n, f as f32 / n);
    }
}
