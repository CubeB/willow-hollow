use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, OnceLock},
};

use super::*;
use crate::{
    continent::SimConfig,
    settlements::quay::{Serves, calm},
    surface::fall_drop,
    world::GameWorld,
};

/// The continents of a few small worlds, made once for all the tests here.
fn worlds() -> &'static [(u32, Arc<Continent>, Travel)] {
    static WORLDS: OnceLock<Vec<(u32, Arc<Continent>, Travel)>> = OnceLock::new();
    WORLDS.get_or_init(|| {
        (1..=3)
            .map(|seed| {
                let c = GameWorld::with_config(seed, SimConfig { size: 256 }).continent(0, 0);
                let t = Travel::of(&c, seed);
                (seed, c, t)
            })
            .collect()
    })
}

/// Some days through a few years, near and far.
const DAYS: [i64; 8] = [0, 3, 9, 40, 41, 100, 233, 1000];

/// Whether each of `points` holds water or lies in a river's channel, looked at a tile of
/// the continent at a time. (Where a river meets the sea its channel can run dry, its bed
/// at the level of the sea: a boat goes up it there all the same.)
fn wet(c: &Continent, seed: u32, points: &[Vec2]) -> Vec<bool> {
    const TILE: i32 = 192;
    let mut by_tile: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (k, p) in points.iter().enumerate() {
        by_tile.entry(((p.x as i32).div_euclid(TILE), (p.y as i32).div_euclid(TILE))).or_default().push(k);
    }
    let mut out = vec![false; points.len()];
    for ((tx, tz), ks) in by_tile {
        let sampler = Sampler::new(c, seed, tx * TILE - 2, tz * TILE - 2, TILE + 4);
        for k in ks {
            let p = points[k];
            let col = sampler.column(p.x.floor() as i32, p.y.floor() as i32);
            out[k] = col.water_surface().is_some() || col.river < 0.0;
        }
    }
    out
}

/// How far `p` is from the worn edge of the nearest trail, and whether there is one near.
fn off_trail(net: &Network, p: Vec2) -> f32 {
    let (pts, _) = net.gather(p, p);
    pts.iter().map(|t| vec2(t.x, t.z).distance(p) - t.half_width.max(0.0)).fold(f32::MAX, f32::min)
}

#[test]
fn the_day_turns_at_midnight() {
    let year_days = 24.0;
    for day in [0i64, 1, 23, 24, 25, 100, 777] {
        for hour in [0.01f32, 6.0, 12.0, 23.99] {
            let t = (day as f64 + hour as f64 / 24.0) / year_days as f64;
            let (year, date) = (t.floor() as u32, t.fract() as f32);
            assert_eq!(day_of(year, date, hour, year_days), day, "day {day} at {hour}");
        }
    }
}

#[test]
fn the_same_travellers_are_in_the_same_places_for_the_same_seed_and_time() {
    let (seed, c, travel) = &worlds()[0];
    // The continent made again from nothing, and its travellers worked out again.
    let again = GameWorld::with_config(*seed, SimConfig { size: 256 }).continent(0, 0);
    let travel_again = Travel::of(&again, *seed);
    let key = |c: &Continent| c.trails.waterways.quays.iter().map(|q| (q.settlement, q.berth.to_array().map(f32::to_bits), q.footprint.clone())).collect::<Vec<_>>();
    assert_eq!(key(c), key(&again), "the quays differ");
    assert_eq!(c.trails.waterways.runs.len(), again.trails.waterways.runs.len());
    assert_eq!(travel.roads.len(), travel_again.roads.len());
    let mut seen = 0;
    for day in DAYS {
        let a = travel.near(c, day, Vec2::ZERO, f32::INFINITY);
        let b = travel_again.near(&again, day, Vec2::ZERO, f32::INFINITY);
        assert_eq!(a.len(), b.len(), "day {day}");
        for (a, b) in a.iter().zip(&b) {
            for hour in [2.0, 7.9, 10.0, 12.5, 16.0, 19.0, 23.0] {
                for calm in [true, false] {
                    assert_eq!(a.at(hour, 0.3, calm), b.at(hour, 0.3, calm), "day {day} at {hour}");
                    seen += 1;
                }
            }
        }
    }
    assert!(seen > 100, "only {seen} looked at");
    // What a nearby player sees is what a far one would: asking about a small piece of
    // the map gives the same parties there as asking about all of it.
    for day in DAYS {
        let all = travel.near(c, day, Vec2::ZERO, f32::INFINITY);
        for p in all.iter().take(6) {
            let Some(scene) = p.at(10.0, 0.3, true) else { continue };
            let here = travel.near(c, day, flat(scene.at), m(40.0));
            let found = here.iter().find(|q| q.id() == p.id()).expect("the party is found near where it is");
            assert_eq!(found.at(10.0, 0.3, true), Some(scene));
        }
    }
}

#[test]
fn campers_stay_a_week_and_are_gone_the_next() {
    for (_, c, travel) in worlds() {
        assert!(!travel.stretches.is_empty(), "no long stretches of trail");
        let campers = |day: i64| -> Vec<(u64, [u32; 3])> {
            travel
                .near(c, day, Vec2::ZERO, f32::INFINITY)
                .into_iter()
                .filter(|p| p.kind() == Kind::Camper)
                .filter_map(|p| p.at(20.0, 0.3, true))
                .map(|s| (s.id, s.fire.expect("a fire").0.to_array().map(f32::to_bits)))
                .collect()
        };
        let mut changed = 0;
        for week in [0i64, 5, 13, 40] {
            let first = campers(week * WEEK);
            assert!(!first.is_empty(), "no campers in week {week}");
            // About one to a long stretch.
            assert!(first.len() as f32 <= travel.stretches.len() as f32, "more campers than stretches");
            for d in 1..WEEK {
                assert_eq!(campers(week * WEEK + d), first, "the campers moved within week {week}");
            }
            let next = campers((week + 1) * WEEK);
            let places = |v: &[(u64, [u32; 3])]| v.iter().map(|x| x.1).collect::<HashSet<_>>();
            assert!(places(&first).is_disjoint(&places(&next)), "a camper stayed on into week {}", week + 1);
            changed += (first != next) as usize;
        }
        assert_eq!(changed, 4);
    }
}

#[test]
fn campers_camp_off_the_trail_on_dry_ground() {
    for (seed, c, travel) in worlds() {
        for day in [0, 70, 140] {
            for p in travel.near(c, day, Vec2::ZERO, f32::INFINITY).iter().filter(|p| p.kind() == Kind::Camper) {
                let s = p.at(20.0, 0.3, true).expect("a camper");
                let (fire, _) = s.fire.expect("a fire");
                assert!(off_trail(&c.trails, flat(fire)) > m(0.8), "a camper's fire on the trail at {fire}");
                let props: Vec<Vec2> = std::iter::once(flat(fire)).chain(s.tents.iter().chain(&s.bedrolls).map(|t| flat(t.0))).collect();
                assert!(wet(c, *seed, &props).iter().all(|w| !w), "a camp in the water at {fire}");
            }
        }
    }
}

#[test]
fn settlers_keep_to_the_trails_and_camp_off_them() {
    let mut walking = 0;
    let mut camps = 0;
    for (seed, c, travel) in worlds() {
        let net = &c.trails;
        let mut far = 0;
        let mut looked = 0;
        for day in 0..60 {
            for p in travel.near(c, day, Vec2::ZERO, f32::INFINITY).iter().filter(|p| p.kind() == Kind::Settlers) {
                // On the road: every walker on the trail, but where they cross a green
                // or step from one trail to another where two meet.
                for hour in [8.5, 10.0, 11.9, 12.5, 14.0, 17.0] {
                    let Some(s) = p.at(hour, 0.3, true) else { continue };
                    assert!((5..=8).contains(&s.people.len()), "{} settlers", s.people.len());
                    assert!((1..=2).contains(&s.carts.len()), "{} carts", s.carts.len());
                    for f in &s.people {
                        let at = flat(f.feet);
                        if net.settlements.iter().any(|t| t.at().distance(at) < t.kind.green() + JOIN + m(2.0)) {
                            continue;
                        }
                        let d = off_trail(net, at);
                        looked += 1;
                        far += (d > m(1.0)) as usize;
                        assert!(d < JOIN, "a settler {d} off the trail at {at}");
                        walking += 1;
                    }
                }
                // In camp: the fire, the carts and the tents off the trail and dry.
                for hour in [3.0, 20.0] {
                    let Some(s) = p.at(hour, 0.3, true) else { continue };
                    let (fire, _) = s.fire.expect("a camp has a fire");
                    assert!(off_trail(net, flat(fire)) > m(3.0), "a settlers' fire on the trail at {fire}");
                    for t in s.carts.iter().map(|c| c.at).chain(s.tents.iter().map(|t| t.0)) {
                        assert!(off_trail(net, flat(t)) > m(0.5), "a cart or tent on the trail at {t}");
                    }
                    let props: Vec<Vec2> = std::iter::once(flat(fire)).chain(s.tents.iter().map(|t| flat(t.0))).collect();
                    assert!(wet(c, *seed, &props).iter().all(|w| !w), "a settlers' camp in the water at {fire}");
                    camps += 1;
                }
            }
        }
        assert!(looked > 0);
        assert!((far as f32) < 0.03 * looked as f32, "{far} of {looked} settlers off the trail");
    }
    assert!(walking > 100, "only {walking} settlers seen walking");
    assert!(camps > 10, "only {camps} camps seen");
}

#[test]
fn settlers_walk_on_from_where_they_camped() {
    for (_, c, travel) in worlds() {
        let mut checked = 0;
        for day in 0..40 {
            for p in travel.near(c, day, Vec2::ZERO, f32::INFINITY).iter().filter(|p| p.kind() == Kind::Settlers) {
                let (Some(evening), Some(next)) = (p.at(20.0, 0.3, true), travel.near(c, day + 1, Vec2::ZERO, f32::INFINITY).into_iter().find(|q| q.id() == p.id())) else { continue };
                let Some(morning) = next.at(3.0, 0.3, true) else { continue };
                assert_eq!(evening.fire.map(|f| f.0), morning.fire.map(|f| f.0), "they woke somewhere other than where they slept");
                assert_eq!(evening.tents, morning.tents);
                assert_eq!(evening.carts, morning.carts);
                checked += 1;
            }
        }
        assert!(checked > 0);
    }
}

#[test]
fn river_boats_keep_to_calm_water_and_stay_in_when_it_is_rough() {
    let mut runs = 0;
    for (seed, c, travel) in worlds() {
        let ways = &c.trails.waterways;
        for (r, run) in ways.runs.iter().enumerate() {
            runs += 1;
            // Never over a fall, nor down a stretch too steep to be calm.
            for &cell in &run.cells[..run.cells.len().saturating_sub(1)] {
                assert_eq!(fall_drop(c, cell), 0.0, "run {r} goes over a fall");
                assert!(calm(c, cell), "run {r} runs down rough water");
            }
            assert_eq!(ways.quays[run.from].serves, Serves::River);
            assert_eq!(ways.quays[run.to].serves, Serves::River);
            // The whole way on water.
            let mut pts = Vec::new();
            let mut s = 0.0;
            while s < run.length() {
                let (p, _) = run.at(s);
                pts.push(flat(p));
                s += m(2.0);
            }
            let w = wet(c, *seed, &pts);
            let dry = w.iter().filter(|w| !**w).count();
            assert!(dry * 50 <= w.len(), "run {r}: {dry} of {} points out of the water", w.len());
            let berths = [ways.quays[run.from].berth, ways.quays[run.to].berth];
            let mut seen_moving = false;
            for day in 0..30 {
                for p in travel.near(c, day, flat(run.line[0]), m(10.0)).iter().filter(|p| p.kind() == Kind::RiverBoat) {
                    for hour in [3.0, 7.5, 9.0, 11.0, 13.0, 15.0, 18.5, 22.0] {
                        seen_moving |= p.at(hour, 0.3, true).expect("a boat").boats[0].moving;
                        // On rough days, tied up at a quay all day.
                        let rough = p.at(hour, 0.3, false).expect("a boat");
                        assert!(!rough.boats[0].moving, "a boat out in rough water");
                        assert!(berths.iter().any(|q| q.distance(rough.boats[0].at) < 1.0), "a boat away from the quays in rough water");
                    }
                }
            }
            assert!(seen_moving, "no boat moved on run {r} in a month");
        }
    }
    assert!(runs > 0, "no river runs in the worlds tested");
}

#[test]
fn river_boats_are_never_where_there_is_no_water() {
    for (seed, c, travel) in worlds() {
        let mut pts = Vec::new();
        for day in 0..20 {
            for p in travel.near(c, day, Vec2::ZERO, f32::INFINITY).iter().filter(|p| matches!(p.kind(), Kind::RiverBoat | Kind::FishingBoat)) {
                let mut hour = 0.0;
                while hour < 24.0 {
                    for calm in [true, false] {
                        pts.push(flat(p.at(hour, 0.3, calm).expect("a boat").boats[0].at));
                    }
                    hour += 0.5;
                }
            }
        }
        let w = wet(c, *seed, &pts);
        let dry = w.iter().filter(|w| !**w).count();
        assert!(dry * 100 <= w.len(), "{dry} of {} boat positions out of the water", w.len());
    }
}

#[test]
fn fishing_boats_are_out_by_mid_morning_and_back_by_afternoon() {
    let mut boats = 0;
    for (_, c, travel) in worlds() {
        for day in DAYS {
            for date in [0.0, 0.25, 0.5, 0.75] {
                for p in travel.near(c, day, Vec2::ZERO, f32::INFINITY).iter().filter(|p| p.kind() == Kind::FishingBoat) {
                    let at = |hour: f32, calm: bool| p.at(hour, date, calm).expect("a boat").boats[0];
                    let berth = at(3.0, true).at;
                    assert!(!at(3.0, true).moving, "out before dawn");
                    assert!(at(10.0, true).at.distance(berth) > m(30.0), "not out on the water at mid-morning");
                    for hour in [13.0, 14.5, 17.0, 21.0] {
                        assert!(at(hour, true).at.distance(berth) < 1.0, "not back by the afternoon");
                    }
                    for hour in [3.0, 8.0, 10.0, 11.0, 14.0] {
                        assert!(at(hour, false).at.distance(berth) < 1.0, "out in rough weather");
                    }
                    boats += 1;
                }
            }
        }
    }
    assert!(boats > 0, "no fishing boats in the worlds tested");
}

#[test]
fn quays_never_sit_on_a_plot_or_lane_and_never_block_a_river() {
    let mut quays = 0;
    for (seed, c, _) in worlds() {
        let net = &c.trails;
        let lanes: Vec<Vec2> = net.layouts.iter().flat_map(|l| l.lanes.iter().flatten().copied()).collect();
        for q in &net.waterways.quays {
            for col in &q.footprint {
                let inside = net.plots.iter().any(|p| col.x >= p.lo.x && col.x <= p.hi.x && col.y >= p.lo.y && col.y <= p.hi.y);
                assert!(!inside, "a quay on a plot at {col}");
                let p = col.as_vec2() + 0.5;
                assert!(lanes.iter().all(|l| l.distance(p) > m(1.0)), "a quay on a lane at {col}");
                for l in &net.layouts {
                    if let Some((at, r)) = l.tree {
                        assert!(at.distance(p) > r, "a quay on the tree ground at {col}");
                    }
                }
            }
            if q.serves == Serves::River && !q.footprint.is_empty() {
                // Open water beyond the jetty's end, so boats can pass.
                let out = (q.head - q.foot).normalize_or_zero();
                let beyond: Vec<Vec2> = (1..=6).map(|k| q.head + out * (k as f32 * m(0.4))).collect();
                assert!(wet(c, *seed, &beyond).iter().all(|w| *w), "a jetty across its river at {}", q.head);
            }
            quays += 1;
        }
        // No two jetties share a column.
        let mut all = HashSet::new();
        for q in &net.waterways.quays {
            for col in &q.footprint {
                assert!(all.insert(*col), "two quays on one column at {col}");
            }
        }
    }
    assert!(quays > 0);
}

/// Prints where the runs of the small worlds leave the water, for looking at by hand.
#[test]
#[ignore]
fn probe_dry_runs() {
    for (seed, c, _) in worlds() {
        for (r, run) in c.trails.waterways.runs.iter().enumerate() {
            let mut s = 0.0;
            let mut last_dry = -1e9;
            while s < run.length() {
                let (p, _) = run.at(s);
                let sampler = Sampler::new(c, *seed, p.x as i32 - 4, p.z as i32 - 4, 9);
                let col = sampler.column(p.x.floor() as i32, p.z.floor() as i32);
                if col.water_surface().is_none() && s - last_dry > m(6.0) {
                    let k = run.along.partition_point(|&a| a <= s);
                    println!("seed {seed} run {r} at {s:.0} of {:.0} (point {k} of {}): {p} ground {} water {:?} river {:.1} cover {:?}", run.length(), run.line.len(), col.ground, col.water, col.river, col.cover);
                    last_dry = s;
                }
                s += m(2.0);
            }
        }
    }
}

/// Prints what the travellers of a world go by, for looking at by hand.
#[test]
#[ignore]
fn probe() {
    let size = std::env::var("PROBE_SIZE").ok().and_then(|s| s.parse().ok()).unwrap_or(1024);
    let seed = std::env::var("PROBE_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
    let t0 = std::time::Instant::now();
    let c: Arc<Continent> = GameWorld::with_config(seed, SimConfig { size }).continent(0, 0);
    println!("continent in {:?}", t0.elapsed());
    let ways = &c.trails.waterways;
    for (q, quay) in ways.quays.iter().enumerate() {
        println!("quay {q}: {:?} of {} at {:?} heading {:.2} landing {:?} level {} foot {}", quay.serves, quay.settlement, quay.berth, quay.heading, quay.landing, quay.level, quay.footprint.len());
    }
    for run in &ways.runs {
        println!("run {}-{} quays {}-{} length {:.0} m, {} points", run.a, run.b, run.from, run.to, run.length() / 2.5, run.line.len());
    }
    for g in &ways.grounds {
        println!("ground of quay {} at {:?}", g.quay, g.at);
    }
    let t1 = std::time::Instant::now();
    let travel = Travel::of(&c, seed);
    println!("travel in {:?}: {} roads, {} stretches, {} parties", t1.elapsed(), travel.roads.len(), travel.stretches.len(), travel.parties);
    let day: i64 = std::env::var("PROBE_DAY").ok().and_then(|s| s.parse().ok()).unwrap_or(100);
    for day in [day, day + 1, day + 7] {
        let t2 = std::time::Instant::now();
        let plans = travel.near(&c, day, Vec2::ZERO, f32::INFINITY);
        println!("day {day}: {} plans in {:?}", plans.len(), t2.elapsed());
        for p in &plans {
            for hour in [3.0, 10.0, 12.5, 15.0, 20.0] {
                if let Some(s) = p.at(hour, 0.3, true) {
                    println!("  {:?} {:>5.1}h at ({:.0}, {:.0}, {:.0}): {} people, {} carts, {} boats: {}", s.kind, hour, s.at.x, s.at.y, s.at.z, s.people.len(), s.carts.len(), s.boats.len(), s.doing);
                }
            }
        }
    }
}
