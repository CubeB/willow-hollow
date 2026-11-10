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
    assert!(share > 0.7, "only {:.0}% of trail is paved", share * 100.0);
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
