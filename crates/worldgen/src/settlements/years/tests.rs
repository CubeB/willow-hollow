//! Settlements over the years (#207): the same seed and year give the same
//! settlements; year 0 is today's world; each settlement changes a thing or two a year
//! and never holds more than its land and water let it; what has changed looks it, and
//! agrees across chunk borders.

use voxel::{CHUNK, PAD};

use super::*;
use crate::{
    continent::{Continent, SimConfig},
    world::GameWorld,
};

fn world(seed: u32) -> GameWorld {
    GameWorld::with_config(seed, SimConfig { size: 256 })
}

fn lives(c: &Continent, seed: u32, year: i32, season: u8) -> Lives {
    Lives::new(&c.trails, seed, (0, 0), When::new(year, season), &Boons::default())
}

/// The settlements with homes that change, by index.
fn settled(net: &Network) -> Vec<usize> {
    (0..net.settlements.len()).filter(|&k| net.layouts[k].plots.clone().any(|i| churns(net.plots[i].purpose))).collect()
}

/// A stamp as it is seen, to compare.
fn seen(s: Option<Cow<Stamp>>) -> Option<(Vec<(IVec3, Voxel)>, Vec<([i32; 3], [i32; 3], Voxel, u8)>)> {
    s.map(|s| (s.voxels.clone(), s.fine.iter().map(|f| (f.lo, f.hi, f.voxel, f.flap)).collect()))
}

/// The roof covering of building stamp `s`, as seen at `l`.
fn roof(l: &Lives, s: &Stamp) -> Vec<Voxel> {
    let Some(Tag::Building(b)) = s.tag.as_deref() else { return Vec::new() };
    let Some(v) = l.view(s) else { return Vec::new() };
    v.voxels.iter().enumerate().filter(|&(j, &(p, v))| b.inside[j / 64] >> (j % 64) & 1 == 0 && p.y >= b.eaves && mat::ROOFING.contains(&v)).map(|(_, &(_, v))| v).collect()
}

#[test]
fn boons_read_back_as_written() {
    let mut boons = Boons::default();
    assert!(boons.grant(Granted { cell: (3, -1), settlement: 12, boon: Boon::SecondWell, year: 4 }));
    assert!(boons.grant(Granted { cell: (0, 0), settlement: 0, boon: Boon::GuildHall, year: 0 }));
    assert!(!boons.grant(Granted { cell: (3, -1), settlement: 12, boon: Boon::SecondWell, year: 9 }), "granted twice");
    assert_eq!(Boons::from_text(&boons.to_text()), boons);
    assert_eq!(Boons::from_text("second-well 1 2\nnonsense 1 2 3 4\n"), Boons::default());
    for b in Boon::ALL {
        assert_eq!(Boon::by_name(b.name()), Some(b));
    }
}

#[test]
fn the_same_seed_and_year_give_the_same_settlements() {
    let (a, b) = (world(3).continent(0, 0), world(3).continent(0, 0));
    for (year, season) in [(1, 0), (2, 1), (5, 2), (17, 3)] {
        let (x, y) = (lives(&a, 3, year, season), lives(&b, 3, year, season));
        let states = |l: &Lives, c: &Continent| (0..c.trails.plots.len()).map(|i| l.life(i)).collect::<Vec<_>>();
        assert_eq!(states(&x, &a), states(&y, &b), "year {year}");
        for (s, t) in a.trails.stamps().zip(b.trails.stamps()) {
            assert!(seen(x.view(s)) == seen(y.view(t)), "a stamp seen differently in year {year}");
        }
    }
}

#[test]
fn year_zero_is_todays_world_in_every_season() {
    let c = world(3).continent(0, 0);
    let mut boons_unseen = 0;
    for season in 0..4 {
        let l = lives(&c, 3, 0, season);
        assert!(l.is_today());
        for s in c.trails.stamps() {
            match l.view(s) {
                Some(Cow::Borrowed(v)) => assert!(std::ptr::eq(v, s)),
                Some(Cow::Owned(_)) => panic!("a stamp changed in year 0"),
                None => {
                    assert!(matches!(s.tag.as_deref(), Some(Tag::Boon { .. })), "a stamp gone in year 0");
                    boons_unseen += 1;
                }
            }
        }
    }
    assert!(boons_unseen > 0, "no boon has anywhere to go");
    // And every plot as laid out.
    let l = lives(&c, 3, 0, 2);
    assert!((0..c.trails.plots.len()).all(|i| l.life(i) == TODAY));
}

#[test]
fn each_settlement_changes_a_thing_or_two_a_year() {
    let c = world(3).continent(0, 0);
    let net = &c.trails;
    let ks = settled(net);
    assert!(!ks.is_empty());
    let mut prev = lives(&c, 3, 0, 2);
    let mut kinds = [0usize; 4];
    for year in 1..=50 {
        let now = lives(&c, 3, year, 2);
        for &k in &ks {
            let plots = net.layouts[k].plots.clone();
            let changed: Vec<(State, State)> = plots.filter(|&i| prev.state(i) != now.state(i)).map(|i| (prev.state(i), now.state(i))).collect();
            let hives = (prev.hives(k) != now.hives(k)) as usize;
            let n = changed.len() + hives;
            assert!((1..=2).contains(&n), "{:?} {k} changed {n} things in year {year}: {changed:?}", net.settlements[k].kind);
            for (a, b) in changed {
                kinds[match (a, b) {
                    (State::Standing { .. }, State::Abandoned { .. }) => 0,
                    (State::Abandoned { .. }, State::Standing { .. }) => 1,
                    (State::Standing { .. }, State::Standing { .. }) => 2,
                    _ => 3,
                }] += 1;
            }
        }
        prev = now;
    }
    assert!(kinds[0] > 0 && kinds[1] > 0 && kinds[2] > 0, "falls, rebuildings and thatchings: {kinds:?}");
    assert_eq!(kinds[3], 0, "a change of an unlooked-for kind");
}

#[test]
fn no_settlement_ever_holds_more_than_its_land_and_water_let_it() {
    let c = world(3).continent(0, 0);
    let net = &c.trails;
    for year in 0..=50 {
        for season in [0, 1, 3] {
            let l = lives(&c, 3, year, season);
            for k in settled(net) {
                let cap = l.capacity(k);
                let homes: Vec<usize> = net.layouts[k].plots.clone().filter(|&i| churns(net.plots[i].purpose)).collect();
                let standing = homes.iter().filter(|&&i| l.standing(i)).count();
                assert_eq!(cap.homes, homes.len());
                assert!(standing <= cap.homes);
                assert!(homes.len() - standing <= cap.empty, "{k} in year {year}: {} of {} empty, {} let", homes.len() - standing, homes.len(), cap.empty);
                assert!((FEWEST_HIVES..=HIVES).contains(&l.hives(k)));
            }
        }
    }
}

#[test]
fn a_second_well_soon_fills_the_empty_homes() {
    let c = world(3).continent(0, 0);
    let net = &c.trails;
    let Some(&k) = settled(net).iter().max_by_key(|&&k| net.layouts[k].plots.len()) else { panic!("no settlement") };
    let mut boons = Boons::default();
    boons.grant(Granted { cell: (0, 0), settlement: k as u32, boon: Boon::SecondWell, year: 10 });
    let homes: Vec<usize> = net.layouts[k].plots.clone().filter(|&i| churns(net.plots[i].purpose)).collect();
    for year in 14..=40 {
        let l = Lives::new(net, 3, (0, 0), When::new(year, 3), &boons);
        assert!(l.granted(k, Boon::SecondWell));
        let empty = homes.iter().filter(|&&i| !l.standing(i)).count();
        assert!(empty <= 1, "{empty} empty in year {year} with a second well");
    }
    let before = Lives::new(net, 3, (0, 0), When::new(9, 3), &boons);
    assert!(!before.granted(k, Boon::SecondWell), "granted before its year");
}

#[test]
fn smoke_rises_only_from_homes_lived_in() {
    let c = world(3).continent(0, 0);
    let net = &c.trails;
    assert_eq!(net.years.chimney_plot.len(), net.chimneys.len());
    let mut cold = 0;
    for year in [0, 3, 7, 12] {
        let l = lives(&c, 3, year, 1);
        for (i, &p) in net.years.chimney_plot.iter().enumerate() {
            assert_eq!(l.smokes(net, i), l.standing(p as usize));
            cold += !l.smokes(net, i) as usize;
        }
    }
    assert!(cold > 0, "no chimney ever stood cold");
}

/// The first building stamp, plot and year at which `want` holds of its state.
fn find<'a>(c: &'a Continent, want: impl Fn(State) -> bool) -> (&'a Stamp, usize, Lives) {
    for year in 1..=30 {
        for season in 0..4 {
            let l = lives(c, 3, year, season);
            for s in c.trails.stamps() {
                if let Some(Tag::Building(b)) = s.tag.as_deref()
                    && want(l.state(b.plot as usize))
                {
                    return (s, b.plot as usize, l);
                }
            }
        }
    }
    panic!("no building ever was as wanted");
}

#[test]
fn an_empty_home_looks_it() {
    let c = world(3).continent(0, 0);
    let (s, plot, l) = find(&c, |st| matches!(st, State::Abandoned { .. }));
    let v = l.view(s).expect("an empty home still stands");
    assert!(!v.voxels.iter().any(|&(_, v)| v == mat::WINDOW_LIT || v == mat::EMBER), "a light or fire in an empty home");
    assert!(!v.fine.iter().any(|f| f.voxel == mat::WINDOW_LIT || f.voxel == mat::EMBER), "a lamp or fire left burning in an empty home");
    let (was, now) = (roof(&Lives::today(&c.trails), s), roof(&l, s));
    assert!(now.len() < was.len(), "its roof has not fallen in");
    let thatched = now.iter().filter(|&&v| thatch(v)).count();
    assert!(thatched == 0 || now.iter().all(|&v| !thatch(v) || v == mat::THATCH_OLD), "its thatch not weathered alike");
    assert!(!l.standing(plot));
}

#[test]
fn a_home_rising_again_stands_in_scaffolding_without_its_roof() {
    let c = world(3).continent(0, 0);
    let (s, _, l) = find(&c, |st| st == State::Scaffold);
    let v = l.view(s).expect("scaffolding stands");
    assert!(!v.voxels.iter().any(|&(_, v)| mat::ROOFING.contains(&v)), "roofed already");
    assert!(v.fine.iter().any(|f| f.voxel == mat::BARK_PINE), "no poles");
    assert!(v.voxels.iter().filter(|&&(_, v)| v != AIR).count() < s.voxels.iter().filter(|&&(_, v)| v != AIR).count());
}

#[test]
fn fresh_thatch_is_one_colour_all_over_and_fades_by_the_season() {
    let c = world(3).continent(0, 0);
    let (s, plot, l) = find(&c, |st| matches!(st, State::Standing { thatched: Some(_), .. }));
    let State::Standing { thatched: Some(q), .. } = l.state(plot) else { unreachable!() };
    let mut shades = Vec::new();
    for since in 0..6 {
        let at = Lives::new(&c.trails, 3, (0, 0), When::new((q + since) / 4, ((q + since) % 4) as u8), &Boons::default());
        if at.state(plot) != l.state(plot) {
            break;
        }
        let r: Vec<Voxel> = roof(&at, s).into_iter().filter(|&v| thatch(v)).collect();
        assert!(!r.is_empty());
        assert!(r.iter().all(|&v| v == r[0]), "a roof of more than one thatch {since} seasons on: {r:?}");
        shades.push(r[0]);
    }
    assert!(shades.len() >= 2 && shades[0] != shades[shades.len() - 1], "it never faded: {shades:?}");
}

#[test]
fn what_has_changed_agrees_across_chunk_borders() {
    let w = world(3);
    let c = w.continent(0, 0);
    let net = &c.trails;
    let mut compared = 0;
    for (year, season) in [(1, 1), (3, 0), (4, 1), (6, 2), (9, 3), (13, 1)] {
        w.set_now(When::new(year, season), &Boons::default());
        let l = w.lives(0, 0, &c);
        let mut here = 0;
        for (i, p) in net.plots.iter().enumerate() {
            if l.life(i) == TODAY {
                continue;
            }
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
                            assert_eq!(va, vb, "{:?} in year {year}: chunks {:?} and {:?} differ", l.state(i), pair.0, pair.1);
                        }
                    }
                }
            }
            here += 1;
            if here == 3 {
                break;
            }
        }
        compared += here;
    }
    assert!(compared > 0, "nothing changed straddles a chunk border");
}

#[test]
fn a_world_set_to_year_zero_builds_as_one_never_set() {
    let (a, b) = (world(3), world(3));
    assert!(!b.set_now(When::new(0, 2), &Boons::default()), "year 0 changed something");
    let c = a.continent(0, 0);
    let p = &c.trails.plots[c.trails.plots.len() / 2];
    let (cx, cz) = (p.origin.x.div_euclid(CHUNK), p.origin.z.div_euclid(CHUNK));
    let (x, y) = (a.generate_column_bare(cx, cz).chunks, b.generate_column_bare(cx, cz).chunks);
    assert_eq!(x.len(), y.len());
    for ((ya, ca), (yb, cb)) in x.iter().zip(&y) {
        assert_eq!(ya, yb);
        assert!((0..PAD).all(|i| (0..PAD).all(|j| (0..PAD).all(|k| ca.get(i, j, k) == cb.get(i, j, k)))));
    }
    // A year on, the same chunk is made again differently only where its plots changed.
    assert!(b.set_now(When::new(1, 2), &Boons::default()));
    assert!(b.epoch() > a.epoch());
}

/// How long making every stamp of a world's first continent as it stands takes, and how
/// many change, a few years on
/// (`SEED=1 cargo test -p worldgen -- time_views --ignored --nocapture`).
#[test]
#[ignore]
fn time_views() {
    let seed = std::env::var("SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
    let w = GameWorld::new(seed);
    let c = w.continent(0, 0);
    for (year, season) in [(0, 0), (1, 1), (5, 1), (20, 2), (50, 3)] {
        let t = std::time::Instant::now();
        let l = Lives::new(&c.trails, seed, (0, 0), When::new(year, season), &Boons::default());
        let lives_ms = t.elapsed().as_secs_f64() * 1000.0;
        let t = std::time::Instant::now();
        let (mut owned, mut voxels) = (0, 0);
        for s in c.trails.stamps() {
            if let Some(Cow::Owned(v)) = l.view(s) {
                owned += 1;
                voxels += v.voxels.len();
            }
        }
        println!("year {year} season {season}: worked out in {lives_ms:.2} ms; {owned} of {} stamps changed ({voxels} voxels), made in {:.1} ms", c.trails.stamps().count(), t.elapsed().as_secs_f64() * 1000.0);
    }
}

/// What changes in the settlement nearest a place over its first years, and which of
/// its fields change crop, where to look for each: for screenshots
/// (`SEED=1 AT=50524,27324 cargo test -p worldgen -- list_years --ignored --nocapture`).
#[test]
#[ignore]
fn list_years() {
    use crate::macro_layout::{MACRO_SIZE, cell_of};
    let seed = std::env::var("SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
    let (x, z) = std::env::var("AT").ok().and_then(|s| s.split_once(',').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))).unwrap_or((50524, 27324));
    let w = GameWorld::new(seed);
    let (mx, mz) = cell_of(x, z);
    let c = w.continent(mx, mz);
    let net = &c.trails;
    let (ox, oz) = (mx * MACRO_SIZE, mz * MACRO_SIZE);
    let local = glam::Vec2::new((x - ox) as f32, (z - oz) as f32);
    let k = (0..net.settlements.len()).min_by(|&a, &b| net.settlements[a].at().distance(local).total_cmp(&net.settlements[b].at().distance(local))).expect("a settlement");
    let s = &net.settlements[k];
    println!("{:?} {} at {},{} (world {},{}), {} homes, capacity {:?}", s.kind, k, s.x, s.z, s.x + ox, s.z + oz, net.layouts[k].plots.len(), lives(&c, seed, 1, 0).capacity(k));
    for (year, season) in [(1, 0), (1, 1), (1, 2), (1, 3), (2, 0), (2, 1), (2, 2), (5, 0), (5, 1), (5, 2)] {
        let l = Lives::new(net, seed, (mx, mz), When::new(year, season), &Boons::default());
        for i in net.layouts[k].plots.clone() {
            let st = l.state(i);
            if st != (State::Standing { rebuilt: 0, thatched: None }) {
                let p = &net.plots[i];
                println!("  year {year} season {season}: plot {i} {:?} door {},{},{} facing {:?}: {st:?}", p.purpose, p.door.x + ox, p.door.y, p.door.z + oz, p.facing);
            }
        }
    }
    // The fields about it whose crop changes from one season to the next.
    let farm = &c.land.farms[k];
    let r = (farm.radius + farm.reach) as i32;
    let mut seen_ids = std::collections::HashSet::new();
    let mut shown = 0;
    for dz in (-r..=r).step_by(12) {
        for dx in (-r..=r).step_by(12) {
            let (lx, lz) = (s.x + dx, s.z + dz);
            let Some(plot) = super::super::farmland::plot(&c, seed, lx, lz) else { continue };
            if !seen_ids.insert(plot.id) || shown >= 12 {
                continue;
            }
            let crops: Vec<_> = [(0, 0), (1, 0), (1, 1), (1, 2), (1, 3), (2, 0), (2, 2), (5, 1), (5, 2)]
                .iter()
                .map(|&(y, q)| super::super::farmland::rotation::crop(&plot, seed, When::new(y, q)))
                .collect();
            if crops.iter().any(|c| *c != crops[0]) {
                shown += 1;
                println!("  field {} at {},{}: {:?}", plot.id, lx + ox, lz + oz, crops);
            }
        }
    }
}
