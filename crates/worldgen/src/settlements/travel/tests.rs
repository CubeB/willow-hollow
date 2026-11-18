use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, OnceLock},
};

use super::*;
use crate::{
    continent::SimConfig,
    settlements::quay::{Run, Serves, calm},
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
/// Calm water, rough all day, and turning rough in the morning.
const WATERS: [Water; 3] = [Water::CALM, Water::ROUGH, Water { rough_since: Some(10.0) }];

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
                for water in WATERS {
                    assert_eq!(a.at(hour, 0.3, water), b.at(hour, 0.3, water), "day {day} at {hour}");
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
            let Some(scene) = p.at(10.0, 0.3, Water::CALM) else { continue };
            let here = travel.near(c, day, flat(scene.at), m(40.0));
            let found = here.iter().find(|q| q.id() == p.id()).expect("the party is found near where it is");
            assert_eq!(found.at(10.0, 0.3, Water::CALM), Some(scene));
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
                .filter_map(|p| p.at(20.0, 0.3, Water::CALM))
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
                let s = p.at(20.0, 0.3, Water::CALM).expect("a camper");
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
                    let Some(s) = p.at(hour, 0.3, Water::CALM) else { continue };
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
                    let Some(s) = p.at(hour, 0.3, Water::CALM) else { continue };
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
                let (Some(evening), Some(next)) = (p.at(20.0, 0.3, Water::CALM), travel.near(c, day + 1, Vec2::ZERO, f32::INFINITY).into_iter().find(|q| q.id() == p.id())) else { continue };
                let Some(morning) = next.at(3.0, 0.3, Water::CALM) else { continue };
                assert_eq!(evening.fire.map(|f| f.0), morning.fire.map(|f| f.0), "they woke somewhere other than where they slept");
                assert_eq!(evening.tents, morning.tents);
                assert_eq!(evening.carts, morning.carts);
                checked += 1;
            }
        }
        assert!(checked > 0);
    }
}

/// How far along `run`'s line the point of it nearest `p` is.
fn nearest_s(run: &Run, p: Vec2) -> f32 {
    let k = (0..run.line.len()).min_by(|&a, &b| flat(run.line[a]).distance_squared(p).total_cmp(&flat(run.line[b]).distance_squared(p))).unwrap_or(0);
    run.along[k]
}

/// Where the river falls on `run`: the middle of each drop.
fn drops(c: &Continent, run: &Run) -> Vec<Vec2> {
    run.cells[..run.cells.len().saturating_sub(1)]
        .iter()
        .filter(|&&i| fall_drop(c, i) > 0.0)
        .map(|&i| (crate::surface::node(c, i) + crate::surface::node(c, c.rcv[i] as usize)) * 0.5)
        .collect()
}

#[test]
fn river_boats_keep_off_rough_water_and_tie_up_when_it_turns() {
    let mut runs = 0;
    for (seed, c, travel) in worlds() {
        let ways = &c.trails.waterways;
        for (r, run) in ways.runs.iter().enumerate() {
            runs += 1;
            // Never over a fall, nor down a stretch too steep to be calm, but between the
            // stages of a portage.
            let falls = drops(c, run);
            if run.portages.is_empty() {
                assert!(falls.is_empty(), "run {r} goes over a fall with no portage");
                for &cell in &run.cells[..run.cells.len().saturating_sub(1)] {
                    assert!(calm(c, cell), "run {r} runs down rough water with no portage");
                }
            }
            for d in &falls {
                assert!(run.rough(nearest_s(run, *d)), "run {r} goes over the fall at {d} with no portage round it");
            }
            assert_eq!(ways.quays[run.from].serves, Serves::River);
            assert_eq!(ways.quays[run.to].serves, Serves::River);
            // The whole way on water, but where it is carried.
            let mut pts = Vec::new();
            let mut s = 0.0;
            while s < run.length() {
                let (p, _) = run.at(s);
                if !run.rough(s) {
                    pts.push(flat(p));
                }
                s += m(2.0);
            }
            let w = wet(c, *seed, &pts);
            let dry = w.iter().filter(|w| !**w).count();
            assert!(dry * 50 <= w.len(), "run {r}: {dry} of {} points out of the water", w.len());
            let mut seen_moving = false;
            for day in 0..30 {
                for p in travel.near(c, day, flat(run.line[0]), m(10.0)).iter().filter(|p| p.kind() == Kind::RiverBoat) {
                    for hour in [3.0, 7.5, 9.0, 11.0, 13.0, 15.0, 18.5, 22.0] {
                        for water in WATERS {
                            let boat = p.at(hour, 0.3, water).expect("a boat").boats[0];
                            seen_moving |= water == Water::CALM && boat.moving;
                            if !boat.ashore {
                                // Afloat only off the rough water and well away from a fall.
                                let s = nearest_s(run, flat(boat.at));
                                let rough = run.portages.iter().any(|q| s > q.above + m(4.0) && s < q.below - m(4.0));
                                assert!(!rough, "run {r}: a boat afloat on rough water, {s} along, day {day} at {hour}");
                                for d in &falls {
                                    assert!(flat(boat.at).distance(*d) > m(8.0), "run {r}: a boat afloat by the fall at {d}");
                                }
                            }
                            // Tied up all day once it is rough, and by night once it has turned.
                            let tied = water == Water::ROUGH || (water.rough_at(hour) && hour > 19.0);
                            assert!(!(tied && boat.moving), "run {r}: a boat out in rough water, day {day} at {hour}");
                        }
                    }
                }
            }
            assert!(seen_moving, "no boat moved on run {r} in a month");
        }
    }
    assert!(runs > 0, "no river runs in the worlds tested");
}

#[test]
fn portages_keep_to_dry_ground_clear_of_the_plots_and_lanes() {
    let mut portages = 0;
    for (seed, c, _) in worlds().iter().chain(portage_worlds()) {
        let net = &c.trails;
        let lanes: Vec<Vec2> = net.layouts.iter().flat_map(|l| l.lanes.iter().flatten().copied()).collect();
        for (r, run) in net.waterways.runs.iter().enumerate() {
            for (k, p) in run.portages.iter().enumerate() {
                assert!(p.above < p.below && p.above > 0.0 && p.below < run.length(), "run {r} portage {k} out of order");
                assert!(p.path.len() > 2 && p.length() > m(10.0), "run {r} portage {k}: a path {} long", p.length());
                // Every step of the path on dry ground, but where it comes down to the stages.
                let ends = m(1.6);
                let inner: Vec<Vec2> = p.path.iter().zip(&p.along).filter(|(_, a)| **a > ends && **a < p.length() - ends).map(|(q, _)| flat(*q)).collect();
                let w = wet(c, *seed, &inner);
                assert!(w.iter().all(|w| !w), "run {r} portage {k}: the path crosses water at {:?}", inner.iter().zip(&w).find(|(_, w)| **w).map(|(q, _)| *q));
                for q in &p.path {
                    let col = q.floor().as_ivec3();
                    let inside = net.plots.iter().any(|pl| col.x >= pl.lo.x && col.x <= pl.hi.x && col.z >= pl.lo.y && col.z <= pl.hi.y);
                    assert!(!inside, "run {r} portage {k}: the path on a plot at {q}");
                    assert!(lanes.iter().all(|l| l.distance(flat(*q)) > m(1.0)), "run {r} portage {k}: the path on a lane at {q}");
                }
                // Steady underfoot: no step up or down steeper than a path.
                for w in p.path.windows(2) {
                    let (rise, run_) = ((w[1].y - w[0].y).abs(), flat(w[0]).distance(flat(w[1])));
                    assert!(rise <= run_ * 0.6 + 1.0, "run {r} portage {k}: a step of {rise} in {run_} at {}", w[0]);
                }
                for (berth, _) in &p.berths {
                    assert!(wet(c, *seed, &[flat(*berth)])[0], "run {r} portage {k}: a stage's berth out of the water at {berth}");
                }
                for col in &p.footprint {
                    let inside = net.plots.iter().any(|pl| col.x >= pl.lo.x && col.x <= pl.hi.x && col.y >= pl.lo.y && col.y <= pl.hi.y);
                    assert!(!inside, "run {r} portage {k}: a stage on a plot at {col}");
                }
                portages += 1;
            }
        }
        // No two stages or quays share a column.
        let mut all = HashSet::new();
        for col in net.waterways.quays.iter().flat_map(|q| &q.footprint).chain(net.waterways.runs.iter().flat_map(|r| r.portages.iter().flat_map(|p| &p.footprint))) {
            assert!(all.insert(*col), "two jetties on one column at {col}");
        }
    }
    assert!(portages > 0, "no portages in the worlds tested");
}

#[test]
fn boats_go_round_the_portages_by_their_paths() {
    let mut carried = 0;
    for (_, c, travel) in portage_worlds() {
        for (r, run) in c.trails.waterways.runs.iter().enumerate().filter(|(_, r)| !r.portages.is_empty()) {
            let paths: Vec<&Portage> = run.portages.iter().collect();
            for day in 0..24 {
                for p in travel.near(c, day, flat(run.line[0]), m(10.0)).iter().filter(|p| p.kind() == Kind::RiverBoat) {
                    let mut hour = 7.0;
                    while hour < 19.0 {
                        let scene = p.at(hour, 0.3, Water::CALM).expect("a boat");
                        let boat = scene.boats[0];
                        if boat.ashore {
                            // On the path, its crew with it.
                            let on = paths.iter().any(|q| q.path.iter().any(|x| x.distance(boat.at) < m(1.0)));
                            assert!(on, "run {r}: a boat ashore off its path at {}", boat.at);
                            assert!(scene.people.iter().all(|f| f.feet.distance(boat.at) < m(3.0)), "run {r}: a boat carried with nobody under it");
                            carried += 1;
                        }
                        hour += 0.25;
                    }
                }
            }
        }
    }
    assert!(carried > 0, "no boat seen carried round a portage");
}

#[test]
fn boats_tie_up_by_the_bank_at_night_on_a_long_voyage() {
    let mut nights = 0;
    for (seed, c, travel) in worlds().iter().chain(portage_worlds()) {
        for day in 0..30 {
            for p in travel.near(c, day, Vec2::ZERO, f32::INFINITY).iter().filter(|p| p.kind() == Kind::RiverBoat) {
                let s = p.at(22.0, 0.3, Water::CALM).expect("a boat");
                let Some((fire, _)) = s.fire else { continue };
                // Not out in the middle of the river: off the bank, its crew ashore by a fire,
                // and in the morning the same.
                assert!(!s.boats[0].moving);
                let ashore: Vec<Vec2> = std::iter::once(flat(fire)).chain(s.people.iter().map(|f| flat(f.feet))).chain(s.bedrolls.iter().map(|b| flat(b.0))).collect();
                assert!(wet(c, *seed, &ashore).iter().all(|w| !w), "a crew camped in the water at {fire}");
                assert!(off_trail(&c.trails, flat(fire)) > m(0.8), "a crew's fire on a trail at {fire}");
                assert!(s.people.iter().all(|f| f.doing == Doing::Asleep));
                if let Some(next) = travel.near(c, day + 1, flat(fire), m(10.0)).into_iter().find(|q| q.id() == p.id()) {
                    let morning = next.at(5.0, 0.3, Water::CALM).expect("a boat");
                    assert_eq!(morning.fire.map(|f| f.0), Some(fire), "they woke somewhere other than where they slept");
                    assert_eq!(morning.boats[0].at, s.boats[0].at);
                }
                nights += 1;
            }
        }
    }
    assert!(nights > 0, "no boat seen tied up by the bank for the night");
}

#[test]
fn draught_beasts_walk_in_the_shafts_where_their_country_keeps_them() {
    let (mut drawn, mut by_hand) = (0, 0);
    for (seed, c, travel) in worlds() {
        let net = &c.trails;
        for day in 0..60 {
            for p in travel.near(c, day, Vec2::ZERO, f32::INFINITY).iter().filter(|p| p.kind() == Kind::Settlers) {
                let Some(s) = p.at(10.0, 0.3, Water::CALM) else { continue };
                let kept = crate::fauna::kept_by(&net.settlements[s.from.expect("from")]);
                let keeps = kept.contains(&crate::fauna::Kind::Ox) || kept.contains(&crate::fauna::Kind::Donkey);
                assert_eq!(s.beasts.len(), if keeps { s.carts.len() } else { 0 }, "settlers from a country {} draught beasts", if keeps { "with" } else { "without" });
                for cart in &s.carts {
                    match cart.beast {
                        Some(b) => {
                            let beast = s.beasts[b];
                            assert!(kept.contains(&beast.kind));
                            // Ahead of its cart, the shafts' length off, and walking.
                            let d = flat(beast.feet).distance(flat(cart.at));
                            assert!(d > m(1.2) && d < m(3.0), "a beast {d} from its cart");
                            assert!(beast.walking);
                            assert!(off_trail(net, flat(beast.feet)) < JOIN, "a beast off the trail");
                            drawn += 1;
                        }
                        None => by_hand += 1,
                    }
                }
                assert!(s.people.iter().filter(|f| f.doing == Doing::Leading).count() == s.beasts.len(), "a beast with nobody at its head");
                // Tethered in camp on dry ground, off the trail.
                for hour in [3.0, 20.0] {
                    let Some(camp) = p.at(hour, 0.3, Water::CALM) else { continue };
                    let feet: Vec<Vec2> = camp.beasts.iter().map(|b| flat(b.feet)).collect();
                    assert!(wet(c, *seed, &feet).iter().all(|w| !w), "a beast tethered in the water");
                    assert!(camp.beasts.iter().all(|b| !b.walking));
                }
            }
        }
    }
    assert!(drawn > 0, "no cart drawn by a beast in the worlds tested");
    assert!(by_hand > 0, "no cart pulled by hand in the worlds tested");
}

#[test]
fn river_boats_are_never_where_there_is_no_water() {
    for (seed, c, travel) in worlds() {
        let mut pts = Vec::new();
        for day in 0..20 {
            for p in travel.near(c, day, Vec2::ZERO, f32::INFINITY).iter().filter(|p| matches!(p.kind(), Kind::RiverBoat | Kind::FishingBoat)) {
                let mut hour = 0.0;
                while hour < 24.0 {
                    for water in WATERS {
                        let boat = p.at(hour, 0.3, water).expect("a boat").boats[0];
                        if !boat.ashore {
                            pts.push(flat(boat.at));
                        }
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
                    let at = |hour: f32, water: Water| p.at(hour, date, water).expect("a boat").boats[0];
                    let berth = at(3.0, Water::CALM).at;
                    assert!(!at(3.0, Water::CALM).moving, "out before dawn");
                    assert!(at(10.0, Water::CALM).at.distance(berth) > m(30.0), "not out on the water at mid-morning");
                    for hour in [13.0, 14.5, 17.0, 21.0] {
                        assert!(at(hour, Water::CALM).at.distance(berth) < 1.0, "not back by the afternoon");
                    }
                    for hour in [3.0, 8.0, 10.0, 11.0, 14.0] {
                        assert!(at(hour, Water::ROUGH).at.distance(berth) < 1.0, "out in rough weather");
                    }
                    // Caught out at mid-morning, home from where they were, not from the
                    // ground; and in by midday.
                    let turned = Water::rough_since(10.0);
                    let was = at(10.0, Water::CALM).at;
                    let soon = at(10.1, turned);
                    assert!(soon.moving && soon.at.distance(was) < m(30.0), "not making for home from where they were");
                    assert!(soon.at.distance(berth) < was.distance(berth), "not making for home");
                    assert!(at(12.0, turned).at.distance(berth) < 1.0, "not home out of the weather");
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

/// A world with a run past rough water, for the portages' tests: the small worlds have
/// none, their rivers too narrow for quays where they run down cascades, so a full-sized
/// one. (In the first twenty full-sized worlds no run goes over a fall itself: the
/// settlements are on the lowland rivers, below their falls. Seeds 10 and 18 have runs
/// down cascades, each with one portage.)
const PORTAGE_SEEDS: [u32; 1] = [18];

/// The continents of the worlds of [`PORTAGE_SEEDS`], made once for the tests that need a
/// run with portages on it.
fn portage_worlds() -> &'static [(u32, Arc<Continent>, Travel)] {
    static WORLDS: OnceLock<Vec<(u32, Arc<Continent>, Travel)>> = OnceLock::new();
    WORLDS.get_or_init(|| {
        PORTAGE_SEEDS
            .iter()
            .map(|&seed| {
                let c = GameWorld::with_config(seed, SimConfig { size: 1024 }).continent(0, 0);
                let t = Travel::of(&c, seed);
                (seed, c, t)
            })
            .collect()
    })
}

/// Prints the runs with portages of small worlds and of a full-sized one, and the days a
/// boat is carried round them, for finding a world to look at (`PROBE_SEEDS`, `PROBE_SIZE`).
#[test]
#[ignore]
fn probe_portages() {
    let size = std::env::var("PROBE_SIZE").ok().and_then(|s| s.parse().ok()).unwrap_or(256);
    let seeds: Vec<u32> = std::env::var("PROBE_SEEDS").ok().map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_else(|| (1..=12).collect());
    for seed in seeds {
        let c = GameWorld::with_config(seed, SimConfig { size }).continent(0, 0);
        let ways = &c.trails.waterways;
        let carried: Vec<_> = ways.runs.iter().filter(|r| !r.portages.is_empty()).collect();
        println!("seed {seed}: {} runs, {} past falls, {} drops in all on runs", ways.runs.len(), carried.len(), ways.runs.iter().map(|r| drops(&c, r).len()).sum::<usize>());
        if carried.is_empty() {
            continue;
        }
        let travel = Travel::of(&c, seed);
        for run in carried {
            for p in &run.portages {
                println!("  run {}-{}: portage {:.0}..{:.0} of {:.0}, path {:.0} m from {} to {}", run.a, run.b, p.above, p.below, run.length(), p.length() / m(1.0), p.path[0], p.path[p.path.len() - 1]);
            }
            for day in 0..12 {
                for plan in travel.near(&c, day, flat(run.line[0]), m(10.0)).iter().filter(|p| p.kind() == Kind::RiverBoat) {
                    let mut hour = 7.0;
                    while hour < 19.0 {
                        let s = plan.at(hour, 0.3, Water::CALM).expect("a boat");
                        if s.boats[0].ashore {
                            println!("    day {day} at {hour:.2}: {} at {}", s.doing, s.boats[0].at);
                            break;
                        }
                        hour += 0.25;
                    }
                }
            }
        }
    }
}

/// Prints where the trade boats of a world tie up by the bank for the night, and the days,
/// for finding one to look at (`PROBE_SEEDS`, `PROBE_SIZE`).
#[test]
#[ignore]
fn probe_moorings() {
    let size = std::env::var("PROBE_SIZE").ok().and_then(|s| s.parse().ok()).unwrap_or(1024);
    let seeds: Vec<u32> = std::env::var("PROBE_SEEDS").ok().map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_else(|| vec![1]);
    for seed in seeds {
        let c = GameWorld::with_config(seed, SimConfig { size }).continent(0, 0);
        let longest = c.trails.waterways.runs.iter().map(|r| r.length() / m(1.0)).fold(0.0, f32::max);
        println!("seed {seed}: {} runs, the longest {longest:.0} m", c.trails.waterways.runs.len());
        let travel = Travel::of(&c, seed);
        for day in 0..12 {
            for plan in travel.near(&c, day, Vec2::ZERO, f32::INFINITY).iter().filter(|p| p.kind() == Kind::RiverBoat) {
                let s = plan.at(21.0, 0.3, Water::CALM).expect("a boat");
                if s.doing.contains("bank") {
                    println!("seed {seed} day {day}: {} {:?}-{:?}, boat at {}, crew at {:?}", s.doing, s.from, s.to, s.boats[0].at, s.people.first().map(|f| f.feet));
                }
            }
        }
    }
}

/// Prints the falls of a world nearest the trade boats' runs and the river quays, where
/// the boats never go, for looking at by hand (`PROBE_SEED`, `PROBE_SIZE`).
#[test]
#[ignore]
fn probe_falls() {
    let size = std::env::var("PROBE_SIZE").ok().and_then(|s| s.parse().ok()).unwrap_or(1024);
    let seed = std::env::var("PROBE_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
    let c = GameWorld::with_config(seed, SimConfig { size }).continent(0, 0);
    let ways = &c.trails.waterways;
    let wet: Vec<Vec2> = ways.runs.iter().flat_map(|r| r.line.iter().map(|p| vec2(p.x, p.z))).chain(ways.quays.iter().filter(|q| q.serves == Serves::River).map(|q| q.head)).collect();
    let mut falls: Vec<(f32, Vec2, Vec2, f32)> = (0..c.size * c.size)
        .filter(|&i| fall_drop(c.as_ref(), i) > 0.0)
        .map(|i| {
            let (p, below) = (crate::surface::node(c.as_ref(), i), crate::surface::node(c.as_ref(), c.rcv[i] as usize));
            let near = wet.iter().map(|w| w.distance(p)).fold(f32::INFINITY, f32::min);
            (near, p, below, fall_drop(c.as_ref(), i))
        })
        .collect();
    falls.sort_by(|a, b| a.0.total_cmp(&b.0));
    println!("{} falls in all", falls.len());
    for (near, p, below, drop) in falls.iter().take(8) {
        println!("a fall at ({:.0}, {:.0}) to ({:.0}, {:.0}), dropping {drop:.1}, {:.0} m from the nearest run or river quay", p.x, p.y, below.x, below.y, near / m(1.0));
    }
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
                if let Some(s) = p.at(hour, 0.3, Water::CALM) {
                    println!("  {:?} {:>5.1}h at ({:.0}, {:.0}, {:.0}): {} people, {} carts, {} beasts, {} boats: {}", s.kind, hour, s.at.x, s.at.y, s.at.z, s.people.len(), s.carts.len(), s.beasts.len(), s.boats.len(), s.doing);
                }
            }
        }
    }
}

/// Iron goes by boat only to a settlement with a smithy, on some of the voyages there and
/// not all; a boat has the same load every day of a voyage, the same each time it is
/// asked for, and lands its iron on the quay it was bound for, where it lies in a stack
/// while the boat is tied up there (#205).
#[test]
fn boats_bring_iron_only_to_the_smithies_and_keep_their_load_all_voyage() {
    let (mut voyages, mut iron, mut landed) = (0, 0, 0);
    for (seed, c, travel) in worlds() {
        // The same continent with a smithy in every settlement and with none, so that
        // every run is tried both ways whatever smithies this world happens to have.
        let mut every = Travel::of(c, *seed);
        every.smithies.iter_mut().for_each(|s| *s = true);
        let mut none = Travel::of(c, *seed);
        none.smithies.iter_mut().for_each(|s| *s = false);
        for (r, run) in c.trails.waterways.runs.iter().enumerate() {
            for b in 0..boats_on(run.length(), travel.seed, r) {
                let mut last: Option<RiverPlan> = None;
                for day in 0..120 {
                    let plan = travel.river_boat(c, r, b, day);
                    assert!(!plan.iron || travel.smithies[plan.to], "seed {seed} run {r}: iron for {}, which has no smithy", plan.to);
                    let again = travel.river_boat(c, r, b, day);
                    assert_eq!((plan.cargo, plan.iron, plan.stack), (again.cargo, again.iron, again.stack), "seed {seed} run {r} day {day}");
                    assert!(!none.river_boat(c, r, b, day).iron, "seed {seed} run {r} day {day}: iron where there is no smithy");
                    let all = every.river_boat(c, r, b, day);
                    assert_eq!(all.cargo, plan.cargo, "the load is the voyage's, not the smithy's");
                    // A voyage runs on while the boat goes the same way.
                    match &last {
                        Some(before) if before.down == all.down => assert_eq!((before.cargo, before.iron), (all.cargo, all.iron), "seed {seed} run {r} day {day}: the load changed mid-voyage"),
                        _ => {
                            voyages += 1;
                            iron += all.iron as usize;
                        }
                    }
                    // Its iron is ashore only while it lies at the quay it was bound for,
                    // and never still aboard there.
                    let quay = &c.trails.waterways.quays[if all.down { run.to } else { run.from }];
                    for hour in [7.0, 12.0, 17.0] {
                        let s = all.at(hour, Water::CALM);
                        let tied = s.boats[0].at.distance(quay.berth) < 0.01;
                        if matches!(all.leg, Leg::Moored) || (tied && all.iron) {
                            assert!(!s.boats[0].iron, "seed {seed} run {r} day {day}: the iron is still aboard at the quay");
                            assert_eq!(s.iron, all.stack.into_iter().collect::<Vec<_>>(), "seed {seed} run {r} day {day} {hour}h");
                        } else {
                            assert!(s.iron.is_empty(), "seed {seed} run {r} day {day} {hour}h: iron ashore while the boat is away");
                            assert_eq!(s.boats[0].iron, all.iron, "seed {seed} run {r} day {day} {hour}h");
                        }
                        if let Some((p, _)) = s.iron.first() {
                            landed += 1;
                            assert!(vec2(p.x, p.z).distance(quay.foot) < m(12.0), "seed {seed} run {r}: the iron is stacked far off its quay");
                        }
                    }
                    last = Some(all);
                }
            }
        }
    }
    assert!(voyages > 20, "too few voyages to tell: {voyages}");
    let share = iron as f32 / voyages as f32;
    assert!((0.15..0.65).contains(&share), "iron on {iron} of {voyages} voyages");
    assert!(landed > 0, "no iron was ever landed");
}

/// Prints the boats that carry iron and the quays it is landed on, for aiming shots
/// (#205): `PROBE_SEED=1 cargo test -p worldgen probe_iron -- --ignored --nocapture`.
#[test]
#[ignore]
fn probe_iron() {
    let seed = std::env::var("PROBE_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
    let c: Arc<Continent> = GameWorld::new(seed).continent(0, 0);
    let travel = Travel::of(&c, seed);
    let net = &c.trails;
    let name = |k: usize| net.names.settlement(k).map_or_else(|| format!("#{k}"), |p| p.name.clone());
    for (k, s) in net.settlements.iter().enumerate().filter(|(k, _)| travel.smithies[*k]) {
        println!("smithy at {} ({:?}) {},{}: coast {} river {:?}", name(k), s.kind, s.x, s.z, s.coast, s.river);
    }
    for (plot, guilds) in &net.guild_halls {
        let k = net.plots[*plot].settlement;
        let s = &net.settlements[k];
        let quarries: Vec<String> = c.land.quarries.iter().map(|q| format!("{} at {} {:.0} m", name(q.owner), q.floor, vec2(q.floor.x as f32, q.floor.z as f32).distance(s.at()) / 2.5)).collect();
        println!("guild house of {} {},{}: {guilds:?}; quarries {quarries:?}", name(k), s.x, s.z);
    }
    let ways = &net.waterways;
    for (r, run) in ways.runs.iter().enumerate() {
        let (qa, qb) = (&ways.quays[run.from], &ways.quays[run.to]);
        println!("run {r}: {} -> {}, smithies {} {}, quays at {} / {} (landings {:?} {:?}), {:.0} m", name(run.a), name(run.b), travel.smithies[run.a], travel.smithies[run.b], qa.foot, qb.foot, qa.landing, qb.landing, run.length() / 2.5);
        for day in 0..48 {
            for b in 0..boats_on(run.length(), travel.seed, r) {
                let plan = travel.river_boat(&c, r, b, day);
                if !plan.iron {
                    continue;
                }
                for hour in [9.0, 12.0, 16.0] {
                    let s = plan.at(hour, Water::CALM);
                    println!("  day {day} boat {b} {hour}h at {:.0},{:.0},{:.0}: {} (iron aboard {}, stacks {:?})", s.at.x, s.at.y, s.at.z, s.doing, s.boats[0].iron, s.iron);
                }
            }
        }
    }
}
