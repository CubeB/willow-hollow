//! Settlements laid out (#202): every building counted in, standing on its own ground
//! within the settlement, clear of the others, the lanes, the trees and the water, with
//! a walk from its door to a lane; the same every time and across chunk borders.

use std::collections::{HashMap, HashSet, VecDeque};

use glam::{IVec2, Vec2, vec2};
use voxel::{AIR, CHUNK, PAD};

use super::{DOOR_PATH, Plot};
use crate::{
    continent::{Continent, SimConfig},
    scale::m,
    macro_layout::MACRO_SIZE,
    settlements::{Kind, Region, calendar},
    surface::Sampler,
    trails::{Network, build::is_wet},
    world::GameWorld,
};

fn world(seed: u32) -> GameWorld {
    GameWorld::with_config(seed, SimConfig { size: 256 })
}

fn plots_of(net: &Network, k: usize) -> &[Plot] {
    &net.plots[net.layouts[k].plots.clone()]
}

/// Whether column `c` is under a plot's building.
fn under(p: &Plot, c: IVec2) -> bool {
    c.cmpge(p.lo).all() && c.cmple(p.hi).all()
}

/// A few settlements of a continent to look at closely: the biggest, a village and a
/// hamlet, by index.
fn sample(net: &Network) -> Vec<usize> {
    let mut out = Vec::new();
    let biggest = (0..net.settlements.len()).max_by_key(|&k| (net.settlements[k].kind, std::cmp::Reverse(k)));
    out.extend(biggest);
    for want in [Kind::Village, Kind::Hamlet] {
        out.extend((0..net.settlements.len()).find(|&k| net.settlements[k].kind == want && !out.contains(&k)));
    }
    out
}

#[test]
fn settlements_are_laid_out_the_same_every_time() {
    let key = |c: &Continent| {
        let net = &c.trails;
        let plots: Vec<_> = net.plots.iter().map(|p| (p.settlement, p.purpose, p.step, p.origin, p.turns, p.door, p.lo, p.hi, p.sleepers)).collect();
        let layouts: Vec<_> = net.layouts.iter().map(|l| (l.lanes.clone(), l.wells.clone(), l.stalls.clone(), l.plots.clone())).collect();
        let chimneys: Vec<_> = net.chimneys.iter().map(|v| v.to_array().map(f32::to_bits)).collect();
        (plots, layouts, chimneys, net.stamp_count())
    };
    let (a, b) = (world(3).continent(0, 0), world(3).continent(0, 0));
    assert!(!a.trails.plots.is_empty(), "no buildings on the test continent");
    assert!(key(&a) == key(&b), "settlements laid out differently");
    // Every stamp in a settlement: its buildings and the things about them.
    let settled = |c: &Continent, lo: glam::IVec3| {
        c.trails.settlements.iter().any(|s| (vec2(lo.x as f32, lo.z as f32)).distance(s.at()) < s.radius + m(4.0))
    };
    let (sa, sb): (Vec<_>, Vec<_>) = (a.trails.stamps().filter(|s| settled(&a, s.lo)).collect(), b.trails.stamps().filter(|s| settled(&b, s.lo)).collect());
    assert_eq!(sa.len(), sb.len());
    for (s, t) in sa.iter().zip(&sb) {
        assert!(s.lo == t.lo && s.voxels == t.voxels && s.fine.len() == t.fine.len(), "a stamp differs at {}", s.lo);
    }
}

#[test]
fn every_settlement_has_the_buildings_and_beds_it_counts() {
    for seed in 1..=4 {
        let c = world(seed).continent(0, 0);
        let net = &c.trails;
        assert_eq!(net.layouts.len(), net.settlements.len());
        for (k, s) in net.settlements.iter().enumerate() {
            let plots = plots_of(net, k);
            let beds: u32 = plots.iter().map(|p| p.sleepers as u32).sum();
            let words: Vec<_> = plots.iter().map(|p| p.purpose.word()).collect();
            println!("seed {seed} {} {:?}: {} of {} buildings, {beds} beds for {} people: {words:?}", k, s.kind, plots.len(), s.buildings, s.people);
            assert!(plots.iter().all(|p| p.settlement == k));
            assert_eq!(plots.len(), s.buildings as usize, "seed {seed}: settlement {k} ({:?}) at {},{}", s.kind, s.x, s.z);
            if s.kind == Kind::Hamlet {
                assert!((2..=4).contains(&plots.len()));
            }
            assert!(beds <= s.people as u32 && beds as f32 >= s.people as f32 * 0.9, "seed {seed}: settlement {k} sleeps {beds} of {}", s.people);
            // The special buildings its kind calls for.
            if s.kind >= Kind::LargeVillage {
                for want in [super::Purpose::Tavern, super::Purpose::Smithy] {
                    assert!(plots.iter().any(|p| p.purpose == want), "seed {seed}: settlement {k} has no {}", want.word());
                }
            }
        }
    }
}

#[test]
fn buildings_stand_within_their_settlement_clear_of_each_other_the_lanes_and_water() {
    let w = world(3);
    let c = w.continent(0, 0);
    let net = &c.trails;
    for (k, s) in net.settlements.iter().enumerate() {
        let plots = plots_of(net, k);
        let reach = s.radius + m(8.0);
        let (lo, hi) = (s.at() - reach, s.at() + reach);
        let (points, _) = net.gather(lo, hi);
        let sam = Sampler::new(&c, w.seed(), lo.x as i32, lo.y as i32, (2.0 * reach) as i32 + 2);
        for (i, p) in plots.iter().enumerate() {
            let at = format!("settlement {k} ({:?}) {} at {}", s.kind, p.purpose.word(), p.origin);
            // Within the settlement, its pad and all.
            for &(c, r) in &p.pads {
                assert!(c.distance(s.at()) + r <= s.radius + 1.5, "{at}: beyond the settlement");
            }
            for c in [p.lo, p.hi, IVec2::new(p.lo.x, p.hi.y), IVec2::new(p.hi.x, p.lo.y)] {
                assert!((c.as_vec2() + 0.5).distance(s.at()) <= s.radius + 1.5, "{at}: beyond the settlement");
            }
            // Clear of the other buildings and their pads.
            for q in &plots[i + 1..] {
                let apart = p.hi.x < q.lo.x || q.hi.x < p.lo.x || p.hi.y < q.lo.y || q.hi.y < p.lo.y;
                assert!(apart, "{at} overlaps the {} at {}", q.purpose.word(), q.origin);
                for &(a, r) in &p.pads {
                    assert!(q.pads.iter().all(|&(b, s)| a.distance(b) >= r + s), "{at}: its pads overlap the {}'s", q.purpose.word());
                }
            }
            // Clear of every lane and path; no path runs under it.
            for t in &points {
                let c = vec2(t.x, t.z);
                let near = Vec2::new(c.x.clamp(p.lo.x as f32, p.hi.x as f32 + 1.0), c.y.clamp(p.lo.y as f32, p.hi.y as f32 + 1.0));
                let hw = if t.half_width > 0.0 { t.half_width } else { m(1.2) };
                let ok = if (t.half_width - DOOR_PATH).abs() < 1e-3 { near.distance(c) > 0.0 } else { near.distance(c) >= hw };
                assert!(ok, "{at}: a way runs under it at {c}");
            }
            // On dry ground.
            for z in p.lo.y..=p.hi.y {
                for x in p.lo.x..=p.hi.x {
                    let col = sam.column(x, z);
                    assert!(!is_wet(&col) && col.river > 0.0, "{at}: in the water at {x},{z}");
                }
            }
        }
    }
}

/// The solid voxels the network's stamps put in each column about a settlement.
fn stamp_solids(net: &Network, lo: Vec2, hi: Vec2) -> HashMap<IVec2, Vec<i32>> {
    let mut seen = HashSet::new();
    let mut out: HashMap<IVec2, Vec<i32>> = HashMap::new();
    for cz in (lo.y as i32).div_euclid(CHUNK)..=(hi.y as i32).div_euclid(CHUNK) {
        for cx in (lo.x as i32).div_euclid(CHUNK)..=(hi.x as i32).div_euclid(CHUNK) {
            for st in net.stamps_at(cx, cz) {
                if !seen.insert(st as *const _) {
                    continue;
                }
                for &(q, v) in &st.voxels {
                    if v != AIR {
                        out.entry(IVec2::new(q.x, q.z)).or_default().push(q.y);
                    }
                }
            }
        }
    }
    out
}

#[test]
fn every_door_opens_onto_ground_with_a_walk_to_a_lane() {
    let w = world(3);
    let c = w.continent(0, 0);
    let net = &c.trails;
    let mut doors = 0;
    for (k, s) in net.settlements.iter().enumerate() {
        let plots = plots_of(net, k);
        let reach = s.radius + m(16.0);
        let (lo, hi) = (s.at() - reach, s.at() + reach);
        let (x0, z0, span) = (lo.x as i32, lo.y as i32, (2.0 * reach) as i32 + 2);
        let sam = Sampler::new(&c, w.seed(), x0, z0, span);
        let solids = stamp_solids(net, lo, hi);
        let (points, pads) = net.gather(lo, hi);
        // Where a lane or the green is underfoot.
        let mut lane = HashSet::new();
        for t in points.iter().filter(|t| t.half_width > DOOR_PATH + 1e-3) {
            let r = t.half_width;
            for z in (t.z - r).floor() as i32..=(t.z + r).ceil() as i32 {
                for x in (t.x - r).floor() as i32..=(t.x + r).ceil() as i32 {
                    if vec2(x as f32 + 0.5, z as f32 + 0.5).distance(vec2(t.x, t.z)) <= r {
                        lane.insert(IVec2::new(x, z));
                    }
                }
            }
        }
        let green = pads.iter().find(|p| vec2(p.x, p.z).distance(s.at()) < 2.0).map(|p| (vec2(p.x, p.z), p.radius));
        let solid = |c: IVec2, y: i32| solids.get(&c).is_some_and(|ys| ys.contains(&y));
        // The heights one can stand at in a column: on its ground or on anything stamped
        // there, with two voxels of room above.
        let stands = |c: IVec2| -> Vec<i32> {
            let col = sam.column(c.x, c.y);
            let g = col.top();
            let mut out = Vec::new();
            if col.water_surface().is_none() && !solid(c, g + 1) && !solid(c, g + 2) {
                out.push(g);
            }
            for &y in solids.get(&c).into_iter().flatten() {
                if y >= g && !solid(c, y + 1) && !solid(c, y + 2) {
                    out.push(y);
                }
            }
            out
        };
        for p in plots {
            doors += 1;
            let at = format!("settlement {k} ({:?}) {} at {}", s.kind, p.purpose.word(), p.origin);
            let start = (IVec2::new(p.door.x, p.door.z), p.door.y);
            let mut seen = HashSet::from([start]);
            let mut queue = VecDeque::from([start]);
            let mut found = false;
            while let Some((c, h)) = queue.pop_front() {
                let on_green = green.is_some_and(|(g, r)| (c.as_vec2() + 0.5).distance(g) < r - 1.0);
                if lane.contains(&c) || on_green {
                    found = true;
                    break;
                }
                for d in [IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y] {
                    let n = c + d;
                    if (n.as_vec2() + 0.5).distance(s.at()) > reach - 2.0 {
                        continue;
                    }
                    // Through a building only out of its own door, straight ahead.
                    let ahead = (n - start.0) * p.facing;
                    let out_of_door = (n - start.0).perp_dot(p.facing) == 0 && ahead.x + ahead.y > 0;
                    if plots.iter().any(|q| under(q, n)) && !(under(p, n) && out_of_door) {
                        continue;
                    }
                    for y in stands(n) {
                        if y - h <= 1 && h - y <= 3 && seen.insert((n, y)) {
                            queue.push_back((n, y));
                        }
                    }
                }
            }
            assert!(found, "{at}: no walk from its door at {} facing {} to a lane", p.door, p.facing);
        }
    }
    assert!(doors > 0);
}

#[test]
fn no_tree_stands_in_a_house() {
    let w = world(3);
    let c = w.continent(0, 0);
    let net = &c.trails;
    let mut looked = 0;
    for k in sample(net) {
        let plots = plots_of(net, k);
        let columns: HashSet<(i32, i32)> = plots
            .iter()
            .flat_map(|p| [(p.lo.x, p.lo.y), (p.hi.x, p.hi.y), (p.lo.x, p.hi.y), (p.hi.x, p.lo.y)])
            .map(|(x, z)| (x.div_euclid(CHUNK), z.div_euclid(CHUNK)))
            .collect();
        let mut columns: Vec<_> = columns.into_iter().collect();
        columns.sort();
        for (cx, cz) in columns {
            for t in w.generate_column_bare(cx, cz).trees {
                let at = IVec2::new(t.pos.x, t.pos.z);
                let inside = plots.iter().find(|p| at.cmpge(p.lo - IVec2::ONE).all() && at.cmple(p.hi + IVec2::ONE).all());
                assert!(inside.is_none(), "a {:?} at {} stands in the {} at {}", t.species, t.pos, inside.unwrap().purpose.word(), inside.unwrap().origin);
            }
            looked += 1;
        }
    }
    assert!(looked > 0);
}

#[test]
fn buildings_agree_across_chunk_borders() {
    let w = world(3);
    let c = w.continent(0, 0);
    let net = &c.trails;
    let mut compared = 0;
    for p in &net.plots {
        // A building straddling the border between two chunk columns, east to west or
        // north to south.
        let (cx0, cx1) = (p.lo.x.div_euclid(CHUNK), p.hi.x.div_euclid(CHUNK));
        let (cz0, cz1) = (p.lo.y.div_euclid(CHUNK), p.hi.y.div_euclid(CHUNK));
        let pair = if cx0 != cx1 {
            ((cx0, cz0), (cx0 + 1, cz0), true)
        } else if cz0 != cz1 {
            ((cx0, cz0), (cx0, cz0 + 1), false)
        } else {
            continue;
        };
        let (a, b) = (w.generate_column_bare(pair.0.0, pair.0.1).chunks, w.generate_column_bare(pair.1.0, pair.1.1).chunks);
        for (cy, ca) in &a {
            let Some((_, cb)) = b.iter().find(|e| e.0 == *cy) else { continue };
            for (i, j) in [(PAD - 2, 0), (PAD - 1, 1)] {
                for y in 0..PAD {
                    for o in 0..PAD {
                        let (va, vb) = if pair.2 { (ca.get(i, y, o), cb.get(j, y, o)) } else { (ca.get(o, y, i), cb.get(o, y, j)) };
                        assert_eq!(va, vb, "the {} at {}: chunks {:?} and {:?} differ at {i} y {y} o {o} cy {cy}", p.purpose.word(), p.origin, pair.0, pair.1);
                    }
                }
            }
        }
        compared += 1;
        if compared == 6 {
            break;
        }
    }
    assert!(compared > 0, "no building straddles a chunk border");
}

/// Lists every settlement of a full-size world (seed from `SEED`, 1 if unset) with its
/// buildings, for aiming screenshots: `cargo test -p worldgen list_buildings -- --ignored
/// --nocapture`.
#[test]
#[ignore]
fn list_buildings() {
    let seed = std::env::var("SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
    let w = GameWorld::new(seed);
    let c = w.continent(0, 0);
    let net = &c.trails;
    for (k, s) in net.settlements.iter().enumerate() {
        let l = &net.layouts[k];
        let tree = l.tree.map(|t| t.0);
        println!("{k} {:?} {:?} {:?} at {},{} y {} r {:.0}: lanes {}, tree {tree:?} stands at {:?}, wells {:?}, stalls {}", s.kind, s.region, s.age, s.x, s.z, s.y, s.radius, l.lanes.len(), net.village_trees.get(k).copied().flatten(), l.wells, l.stalls.len());
        for p in plots_of(net, k) {
            println!("    {:12} {:?} door {} facing {} lo {} hi {} sleeps {} garden {:?} yard {:?} coop {:?}", p.purpose.word(), p.step, p.door, p.facing, p.lo, p.hi, p.sleepers, p.garden, p.yard, p.coop);
        }
    }
    println!("{} stamps, {} buildings, {} chimneys", net.stamp_count(), net.plots.len(), net.chimneys.len());
    let voxels: usize = net.stamps().map(|s| s.voxels.len()).sum();
    let fine: usize = net.stamps().map(|s| s.fine.len()).sum();
    println!("{voxels} voxels and {fine} fine boxes in stamps");
}

/// Prints the settlements of `region` on the continents of a world near its first: the
/// world's seed from `SEED` (1, 2 and 3 if unset), a small world with `SMALL`, the
/// continents `SPAN` either way of the first (1 if unset). Coordinates are the world's,
/// for `--over` and `tp`; with each, a `--date` that is its market day at noon in the
/// first year (of 24 days) and one that is not.
fn list_region(region: Region) {
    let seeds: Vec<u32> = std::env::var("SEED").ok().and_then(|s| s.parse().ok()).map_or(vec![1, 2, 3], |s| vec![s]);
    let span: i32 = std::env::var("SPAN").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
    for seed in seeds {
        let w = if std::env::var("SMALL").is_ok() { world(seed) } else { GameWorld::new(seed) };
        for mz in -span..=span {
            for mx in -span..=span {
                let c = w.continent(mx, mz);
                let (ox, oz) = (mx * MACRO_SIZE, mz * MACRO_SIZE);
                for (k, s) in c.trails.settlements.iter().enumerate() {
                    if s.region != region {
                        continue;
                    }
                    let l = &c.trails.layouts[k];
                    let raised = plots_of(&c.trails, k).iter().filter(|p| p.door.y - (p.origin.y - 1) >= 3).count();
                    // Noon on day n of the first year is the date (n + 0.5) / 24.
                    let date = |market: bool| (0..24).find(|&n| calendar::is_market_day(s, n) == market).map(|n| (n as f32 + 0.5) / 24.0);
                    // Where the boards and the lanterns are, in the world, to aim at.
                    let runs: Vec<String> = l.boardwalks.runs.iter().take(4).map(|r| format!("{},{}", ox + r.0.x, oz + r.0.y)).collect();
                    let lamps: Vec<String> = l.lanterns.iter().take(4).map(|(q, _)| format!("{},{},{}", ox + q.x, q.y, oz + q.z)).collect();
                    println!(
                        "seed {seed} continent {mx},{mz} #{k} {:?} {:?} coast {} at {},{} y {} r {:.0}: {} buildings, {raised} raised, {} lanes, {} boardwalk runs {runs:?}, {} lanterns in its trees {lamps:?}, {} stalls, market --date {:?}, not {:?}",
                        s.kind,
                        s.age,
                        s.coast,
                        ox + s.x,
                        oz + s.z,
                        s.y,
                        s.radius,
                        l.plots.len(),
                        l.lanes.len(),
                        l.boardwalks.runs.len(),
                        l.lanterns.len(),
                        l.stalls.len(),
                        date(true),
                        date(false),
                    );
                }
            }
        }
    }
}

/// The settlements of marsh and mangrove, for aiming screenshots of their boardwalks and
/// choosing places for the tests: `cargo test -p worldgen list_marsh_settlements --
/// --ignored --nocapture` (see [`list_region`] for `SEED`, `SMALL` and `SPAN`).
#[test]
#[ignore]
fn list_marsh_settlements() {
    list_region(Region::Wetland);
}

/// The settlements of old-growth forest, for aiming screenshots of the lanterns in their
/// trees: `cargo test -p worldgen list_old_growth_settlements -- --ignored --nocapture`.
#[test]
#[ignore]
fn list_old_growth_settlements() {
    list_region(Region::OldGrowth);
}
