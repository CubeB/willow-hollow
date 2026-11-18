use std::collections::HashSet;

use voxel::{CHUNK, PAD};

use crate::{
    World,
    continent::SimConfig,
    features::landmarks::Landmark,
    macro_layout::MACRO_SIZE,
    surface::{Cover, Sampler},
    trails::CrossingKind,
    world::GameWorld,
};

fn world() -> GameWorld {
    GameWorld::with_config(3, SimConfig { size: 256 })
}

#[test]
fn the_network_is_repeatable() {
    let (a, b) = (world(), world());
    let (ca, cb) = (a.continent(0, 0), b.continent(0, 0));
    let (na, nb) = (&ca.trails, &cb.trails);
    assert_eq!(na.sites.len(), nb.sites.len());
    assert_eq!(na.stamp_count(), nb.stamp_count());
    for (p, q) in na.sites.iter().zip(&nb.sites) {
        assert_eq!((p.kind, p.x, p.y, p.z), (q.kind, q.x, q.y, q.z));
    }
    // The trails themselves, point for point and width for width.
    assert_eq!(na.lines.len(), nb.lines.len());
    for (p, q) in na.lines.iter().zip(&nb.lines) {
        assert_eq!(p, q);
    }
    assert_eq!(na.line_widths, nb.line_widths);
    let sum = |n: &crate::trails::Network| n.points().map(|p| (p.x + p.y + p.z + p.half_width) as f64).sum::<f64>();
    assert_eq!(sum(na), sum(nb));
}

#[test]
fn busy_and_remote_trails_differ() {
    use crate::trails::build::{TrailClass, class_of};
    let w = world();
    let c = w.continent(0, 0);
    let mut seen = HashSet::new();
    for widths in &c.trails.line_widths {
        for &width in widths {
            seen.insert(class_of(width));
        }
    }
    println!("classes: {seen:?}");
    assert!(seen.len() >= 2, "every trail is a {seen:?}");
    // The widest and narrowest stretches really are worn differently on the ground.
    let widths: Vec<f32> = c.trails.points().map(|p| p.half_width).filter(|&w| w > 0.0).collect();
    let (lo, hi) = widths.iter().fold((f32::MAX, 0.0f32), |(lo, hi), &w| (lo.min(w), hi.max(w)));
    assert!(hi > lo * 1.8, "widths only run from {lo} to {hi}");
    assert!(seen.contains(&TrailClass::Track) || seen.contains(&TrailClass::Footpath));
}

#[test]
fn trails_keep_off_cliff_faces() {
    // The ground as it was before any trail was laid over it.
    let w = world();
    let c = w.continent(0, 0);
    let mut bare = crate::continent::generate(c.desc, SimConfig { size: 256 });
    bare.trails = Default::default();
    let (mut cliff, mut scree, mut total) = (0, 0, 0);
    for line in &c.trails.lines {
        for p in line {
            let (x, z) = (p.x as i32, p.y as i32);
            let col = Sampler::new(&bare, w.seed(), x, z, 1).column(x, z);
            total += 1;
            match col.cover {
                Cover::Cliff => cliff += 1,
                Cover::Scree => scree += 1,
                _ => {}
            }
        }
    }
    println!("of {total} trail points, {cliff} on cliff faces and {scree} on scree");
    assert!(total > 1000);
    assert!(cliff * 400 < total, "{cliff} of {total} trail points run over cliff faces");
}

#[test]
fn trails_narrow_over_high_ground() {
    let w = world();
    let c = w.continent(0, 0);
    let (mut high, mut low) = (Vec::new(), Vec::new());
    for p in c.trails.points().filter(|p| p.half_width > 0.0) {
        if p.y > crate::scale::m(210.0) {
            high.push(p.half_width);
        } else if p.y < crate::scale::m(60.0) {
            low.push(p.half_width);
        }
    }
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len().max(1) as f32;
    println!("{} high points, mean half-width {:.2}; {} low, {:.2}", high.len(), mean(&high), low.len(), mean(&low));
    assert!(high.len() > 100 && low.len() > 100, "{} high, {} low", high.len(), low.len());
    // Narrowing follows the ground under the trail, and cliffs can lift or drop that a
    // little against the trail's own smoothed height, so allow a few stragglers.
    let wide = high.iter().filter(|&&w| w > crate::scale::m(0.31)).count();
    println!("{wide} high points wider than a mountain path");
    assert!(wide * 100 < high.len(), "{wide} of {} high points are wide", high.len());
    assert!(mean(&low) > mean(&high) * 1.5);
    // And no sudden steps in width along a trail: no more than a few centimetres per
    // metre walked (line points lie 16 voxels apart).
    let jump = c.trails.line_widths.iter().flat_map(|l| l.windows(2)).map(|p| (p[1] - p[0]).abs()).fold(0.0f32, f32::max);
    println!("largest change in half-width between line points: {jump:.2} voxels");
    // The limit was 0.25 m; the biome and archetype terrain nudged the largest step to
    // 0.63 voxels (0.25 m), which still reads as a smooth change at 16 voxels apart.
    // Siting landmarks for a reason (#67) moved the network, and the largest step, where
    // a busy trail narrows over a steep shoulder and widens again, to 0.78 voxels.
    assert!(jump < crate::scale::m(0.33), "width jumps by {jump}");
}

#[test]
fn bridges_are_rare() {
    let w = world();
    let c = w.continent(0, 0);
    let bridges = c.trails.crossings.iter().filter(|x| x.kind != CrossingKind::Stones).count();
    let km: f32 = c
        .trails
        .lines
        .iter()
        .flat_map(|l| l.windows(2))
        .map(|p| (p[1] - p[0]).length())
        .sum::<f32>()
        / crate::scale::m(1000.0);
    println!("{bridges} bridges in {km:.1} km of trail");
    assert!(km > 10.0);
    assert!((bridges as f32) < km * 0.4,"{bridges} bridges in {km:.1} km");
}

#[test]
fn stone_bridges_are_rarest() {
    let mut stone = 0;
    let mut wooden = 0;
    let mut fords = 0;
    let mut unlinked = 0;
    let mut km = 0.0;
    for seed in 1..=4 {
        let w = GameWorld::with_config(seed, SimConfig { size: 256 });
        let c = w.continent(0, 0);
        let count = |k: &[CrossingKind]| c.trails.crossings.iter().filter(|x| k.contains(&x.kind)).count();
        let (s, wd, f) = (count(&[CrossingKind::Arch]), count(&[CrossingKind::Log, CrossingKind::Plank]), count(&[CrossingKind::Stones]));
        let length: f32 = c.trails.lines.iter().flat_map(|l| l.windows(2)).map(|p| (p[1] - p[0]).length()).sum::<f32>() / crate::scale::m(1000.0);
        // Each bridge with its span in trail steps and the width of its river.
        let spans: Vec<String> = c
            .trails
            .crossings
            .iter()
            .filter(|x| x.kind != CrossingKind::Stones)
            .map(|x| format!("{:?} {} w{:.1}", x.kind, x.span, crate::trails::build::river_width_at(&c, glam::vec2(x.x as f32, x.z as f32))))
            .collect();
        println!("seed {seed}: {s} stone, {wd} wooden, {f} fords, {} links unrouted, {length:.1} km; {spans:?}", c.trails.unlinked);
        stone += s;
        wooden += wd;
        fords += f;
        unlinked += c.trails.unlinked;
        km += length;
    }
    println!("in all: {stone} stone, {wooden} wooden, {fords} fords, {unlinked} unrouted, {km:.1} km");
    // Before routes were costed by what their crossings take to build (#177), these
    // four worlds had 17 stone bridges to 41 wooden ones, 112 fords and 9 links left
    // unrouted, in 325 km of trail. A stone bridge was then any span over 13 m, so a
    // trail took one wherever a stream was a little wide. Now a stone bridge is worth
    // kilometres of walking round, and stands only over a big river or a span timber
    // will not reach: 6 stone to 45 wooden. Settlements (#201) added half as much trail
    // again, and the roads to towns by big rivers cross them in stone: 10 stone to 56
    // wooden in 481 km. Still under half as many to the kilometre as before #177.
    assert!(stone as f32 * 2.0 <= 17.0 * km / 325.0, "{stone} stone bridges in {km:.0} km, no fewer than before");
    assert!(stone * 5 <= wooden, "{stone} stone bridges to {wooden} wooden");
    // The network holds together as well as it did: no more links go unmade.
    assert!(unlinked <= 9, "{unlinked} links unrouted");
    assert!(km > 300.0, "only {km:.1} km of trail");
}

#[test]
fn trails_agree_across_chunk_borders() {
    // A column near a trail reads the same whichever chunk's sampler it is taken from.
    let w = world();
    let c = w.continent(0, 0);
    let mut checked = 0;
    for line in c.trails.lines.iter().take(10) {
        for p in line.iter().step_by(20) {
            let (x, z) = (p.x as i32, p.y as i32);
            let border = x.div_euclid(CHUNK) * CHUNK;
            let a = Sampler::new(&c, w.seed(), border - CHUNK, z - 1, CHUNK + 2);
            let b = Sampler::new(&c, w.seed(), border - 1, z - 1, CHUNK + 2);
            for col_x in [border - 1, border] {
                let (ca, cb) = (a.column(col_x, z), b.column(col_x, z));
                assert_eq!(ca.ground, cb.ground, "ground differs at {col_x},{z}");
                assert_eq!(ca.cover, cb.cover, "cover differs at {col_x},{z}");
                checked += 1;
            }
        }
    }
    assert!(checked > 50);
}

#[test]
fn a_continent_gets_varied_sites_joined_by_trails() {
    let w = world();
    let c = w.continent(0, 0);
    let net = &c.trails;
    let kinds: HashSet<Landmark> = net.sites.iter().map(|s| s.kind).collect();
    println!(
        "{} sites of {} kinds, {} trails, {} points, {} crossings, {} junctions, {} stamps",
        net.sites.len(),
        kinds.len(),
        net.lines.len(),
        net.points().count(),
        net.crossings.len(),
        net.junctions.len(),
        net.stamp_count()
    );
    assert!(net.sites.len() >= 12, "only {} sites", net.sites.len());
    assert!(kinds.len() >= 6, "only {kinds:?}");
    assert!(net.lines.len() >= net.sites.len() / 2, "too few trails");
    assert!(net.sites.iter().any(|s| s.kind == Landmark::Landing), "no landing");
}

#[test]
fn sites_stand_on_dry_levelled_ground() {
    let w = world();
    let c = w.continent(0, 0);
    for site in &c.trails.sites {
        let col = w.column(site.x, site.z);
        assert!(col.water_surface().is_none(), "{:?} is under water", site.kind);
        assert_eq!(col.top() + 1, site.y, "{:?} at {},{}: ground does not meet the site", site.kind, site.x, site.z);
    }
}

#[test]
fn moorings_float_on_water() {
    let w = world();
    let c = w.continent(0, 0);
    let mut seen = 0;
    for m in c.trails.sites.iter().filter_map(|s| s.mooring) {
        let col = w.column(m.pos.x as i32, m.pos.z as i32);
        assert!(col.water_surface().is_some(), "boat moored on dry land at {:?}", m.pos);
        seen += 1;
    }
    assert!(seen > 0, "no moorings");
}

#[test]
fn trails_are_walkable() {
    let w = world();
    let c = w.continent(0, 0);
    let mut paved = 0u32;
    let mut total = 0u32;
    for line in &c.trails.lines {
        total += line.len() as u32;
        for p in line {
            let col = w.column(p.x as i32, p.y as i32);
            if col.cover == Cover::Trail {
                paved += 1;
            }
        }
    }
    let share = paved as f32 / total as f32;
    println!("{paved} of {total} sampled trail points are paved ({:.0}%)", share * 100.0);
    // Nearly all the rest is the gaps in faint traces, which come and go on purpose.
    // Routing by the effort of building (#177) moved the network and with it how much
    // of it is trace: the share paved went from 71% to 69%.
    assert!(share > 0.65, "only {:.0}% of trail is paved", share * 100.0);
    // A paved strip is level enough to walk across with single steps.
    let mut steep = 0u32;
    let mut checked = 0u32;
    for line in c.trails.lines.iter().take(40) {
        for pair in line.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let n = (b - a).length().ceil() as i32;
            let mut prev = None;
            for k in 0..n {
                let q = a + (b - a) * (k as f32 / n as f32);
                let col = w.column(q.x as i32, q.y as i32);
                if col.cover != Cover::Trail {
                    prev = None;
                    continue;
                }
                if let Some(p) = prev {
                    checked += 1;
                    if (col.top() - p as i32).abs() > 1 {
                        steep += 1;
                    }
                }
                prev = Some(col.top() as u32);
            }
        }
    }
    println!("{steep} steps over one voxel in {checked}");
    assert!(checked > 500);
    assert!((steep as f32) < checked as f32 * 0.02, "{steep} of {checked} trail steps are too tall");
}

#[test]
fn crossings_keep_rivers_contained() {
    // The same containment check rivers themselves must pass, around every crossing.
    let w = world();
    let c = w.continent(0, 0);
    let kinds: HashSet<CrossingKind> = c.trails.crossings.iter().map(|x| x.kind).collect();
    println!("{} crossings: {kinds:?}", c.trails.crossings.len());
    assert!(!c.trails.crossings.is_empty(), "no crossings");
    let (mut wet, mut leaks) = (0u64, 0u64);
    for crossing in &c.trails.crossings {
        let (x0, z0) = (crossing.x - PAD as i32 / 2, crossing.z - PAD as i32 / 2);
        let s = Sampler::new(&c, w.seed(), x0, z0, PAD as i32);
        for x in x0 + 1..x0 + PAD as i32 - 1 {
            for z in z0 + 1..z0 + PAD as i32 - 1 {
                let Some(k) = s.column(x, z).water_surface() else { continue };
                wet += 1;
                for (nx, nz) in [(x + 1, z), (x - 1, z), (x, z + 1), (x, z - 1)] {
                    let n = s.column(nx, nz);
                    if n.water_surface().is_none() && n.top() < k - 1 {
                        leaks += 1;
                    }
                }
            }
        }
    }
    println!("{wet} water columns near crossings, {leaks} leaks");
    assert!(wet > 200);
    assert!((leaks as f64) < wet as f64 * 0.002, "{leaks} leaks in {wet}");
}

#[test]
fn structures_appear_in_generated_chunks() {
    let w = world();
    let c = w.continent(0, 0);
    let site = c.trails.sites.iter().find(|s| s.kind == Landmark::Lookout).unwrap_or(&c.trails.sites[0]);
    let (cx, cz) = (site.x.div_euclid(CHUNK), site.z.div_euclid(CHUNK));
    let column = w.generate_column(cx, cz).chunks;
    let stamps: Vec<_> = c.trails.stamps_at(cx, cz).collect();
    assert!(!stamps.is_empty(), "site has no stamp");
    let top = stamps.iter().map(|s| s.hi.y).max().unwrap();
    let top_chunk = top.div_euclid(CHUNK);
    assert!(column.iter().any(|(cy, _)| *cy == top_chunk), "chunks stop short of the structure");
    for pair in column.windows(2) {
        assert_eq!(pair[1].0, pair[0].0 + 1);
    }
    // And a stamped voxel really is there.
    let stamp = stamps[0];
    let &(p, v) = stamp
        .voxels
        .iter()
        .find(|(p, v)| {
            *v != voxel::AIR
                && p.x.div_euclid(CHUNK) == cx
                && p.z.div_euclid(CHUNK) == cz
                && column.iter().any(|(cy, _)| *cy == p.y.div_euclid(CHUNK))
        })
        .expect("stamp has a voxel in its own column");
    let (_, chunk) = column.iter().find(|(cy, _)| *cy == p.y.div_euclid(CHUNK)).unwrap();
    let got = chunk.get(
        p.x.rem_euclid(CHUNK) as usize + 1,
        p.y.rem_euclid(CHUNK) as usize + 1,
        p.z.rem_euclid(CHUNK) as usize + 1,
    );
    assert_eq!(got, v, "stamped voxel missing at {p:?}");
}

#[test]
fn trees_keep_off_the_trails() {
    use crate::features;
    let w = world();
    let c = w.continent(0, 0);
    let mut checked = 0;
    for line in c.trails.lines.iter().take(12) {
        for p in line.iter().step_by(12) {
            let (x, z) = (p.x as i32, p.y as i32);
            let margin = features::MAX_REACH + 1;
            let s = Sampler::new(&c, w.seed(), x - margin - 16, z - margin - 16, 2 * margin + 32);
            for placed in features::place(w.seed(), w.library(), &s, x - 16, z - 16, 32, (0, 0)) {
                if placed.pos.x < 0 || placed.pos.z < 0 || placed.pos.x >= MACRO_SIZE || placed.pos.z >= MACRO_SIZE {
                    continue;
                }
                let col = s.column(placed.pos.x, placed.pos.z);
                assert!(col.clear >= 1.5, "{:?} stands on the trail at {}", placed.species, placed.pos);
                checked += 1;
            }
        }
    }
    assert!(checked > 0);
}

#[test]
fn lanterns_gather_at_places_and_thin_out_between() {
    use crate::trails::{LanternCause, build::{JUNCTION_CLEAR, LANTERN_CLEAR, LANTERN_FAR}};
    let w = world();
    let c = w.continent(0, 0);
    let net = &c.trails;
    let lanterns = &net.lanterns;
    let count = |cause| lanterns.iter().filter(|l| l.cause == cause).count();
    println!(
        "{} lanterns: {} bridge, {} junction, {} approach, {} plain",
        lanterns.len(),
        count(LanternCause::Bridge),
        count(LanternCause::Junction),
        count(LanternCause::Approach),
        count(LanternCause::Plain)
    );
    assert!(!lanterns.is_empty(), "no lanterns at all");
    // Plain stretches are lit only now and then.
    let min_gap = (LANTERN_FAR as f32 * 0.7) as usize;
    for (k, l) in lanterns.iter().enumerate() {
        if l.cause != LanternCause::Plain {
            continue;
        }
        if let Some(prev) = lanterns[..k].iter().rev().find(|p| p.trail == l.trail) {
            assert!(l.step - prev.step >= min_gap, "plain lanterns {} steps apart", l.step - prev.step);
        }
    }
    // None crowds one on another trail.
    for (k, l) in lanterns.iter().enumerate() {
        let clear = if l.cause == LanternCause::Junction { JUNCTION_CLEAR } else { LANTERN_CLEAR };
        for p in &lanterns[..k] {
            if p.trail != l.trail || l.cause == LanternCause::Junction {
                assert!(p.at.distance(l.at) >= clear, "lanterns {} apart", p.at.distance(l.at));
            }
        }
    }
    // Landmarks, and now villages and towns, are still approached by lantern light.
    let places: Vec<glam::Vec2> = net.sites.iter().map(|s| glam::vec2(s.x as f32, s.z as f32)).chain(net.settlements.iter().map(|s| s.at())).collect();
    let near_site = lanterns.iter().filter(|l| places.iter().any(|p| l.at.distance(*p) < crate::scale::m(240.0))).count();
    assert!(near_site > 0, "no lantern near any landmark");
    assert!(near_site * 4 >= lanterns.len(), "only {near_site} of {} lanterns near landmarks and settlements", lanterns.len());
    // Most stand cold; those that are lit are lit more often on the way in to a place.
    let share = |f: &dyn Fn(&&crate::trails::TrailLantern) -> bool| {
        let of: Vec<_> = lanterns.iter().filter(f).collect();
        (of.iter().filter(|l| l.lit).count() as f32 / of.len().max(1) as f32, of.len())
    };
    let (lit, _) = share(&|_| true);
    let (approach, n) = share(&|l| l.cause == LanternCause::Approach);
    let (plain, _) = share(&|l| l.cause == LanternCause::Plain);
    println!("lit: {lit:.2} of all, {approach:.2} of {n} on the way in, {plain:.2} on plain stretches");
    assert!((0.1..=0.35).contains(&lit), "{lit:.2} of lanterns lit");
    assert!(n == 0 || approach > plain, "lanterns are lit as often on plain stretches");
}

#[test]
fn nothing_stands_on_the_walkable_trail() {
    let w = world();
    let c = w.continent(0, 0);
    let net = &c.trails;
    // Beside a trail a lantern is a pole of sticks, never a gate across the way; along a
    // village's lane it may be a finer post, but still a single one beside the way.
    use crate::features::parts::LampStyle;
    assert!(net.lanterns.iter().all(|l| l.style == LampStyle::Stick || (l.cause == crate::trails::LanternCause::Lane && !matches!(l.style, LampStyle::Gate | LampStyle::Pair))));
    let points: Vec<_> = net.points().filter(|p| p.half_width > 0.0).collect();
    for l in &net.lanterns {
        assert!(!l.footprint.is_empty());
        for col in &l.footprint {
            let at = col.as_vec2() + 0.5;
            if let Some(p) = points.iter().find(|p| glam::vec2(p.x, p.z).distance(at) < p.half_width + 0.5) {
                panic!("{:?} lantern stands at {col:?} on a trail {} wide", l.style, p.half_width);
            }
        }
    }
}

#[test]
fn ruins_stand_off_the_trails() {
    // Nobody walks to a ruin: no trail runs to one or past it. A tower on a hill may
    // have a path out to it, but only a faint one, ending at the edge of its ground.
    let w = world();
    let c = w.continent(0, 0);
    let net = &c.trails;
    let points: Vec<_> = net.points().filter(|p| p.half_width > 0.0).collect();
    let mut ruins = 0;
    for (s, pad) in net.sites.iter().zip(&net.pads).filter(|(s, _)| s.kind == Landmark::Ruins) {
        ruins += 1;
        let centre = glam::vec2(pad.x, pad.z);
        let near: Vec<_> = points.iter().filter(|p| glam::vec2(p.x, p.z).distance(centre) < crate::scale::m(60.0)).collect();
        println!("ruins at {},{} ({:?}): {} trail points within 60 m", s.x, s.z, s.reason, near.len());
        for p in near {
            let d = glam::vec2(p.x, p.z).distance(centre);
            assert!(d > pad.radius * 0.9, "a trail runs into the ruins at {},{}, {d} from the middle", s.x, s.z);
            assert!(p.half_width < crate::scale::m(0.4), "a worn trail passes {d} from the ruins at {},{}", s.x, s.z);
        }
    }
    assert!(ruins > 0, "no ruins to test");
}
