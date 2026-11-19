//! The settlements' people (#204): the same people in the same places every time, all
//! in bed at night, more at the tavern in the evening than at noon, walking only where
//! the lanes and paths go, drawn within their budget, and naming real places when
//! greeted, with the greeting remembered in the save.

use std::sync::{Arc, OnceLock};

use glam::{DVec3, Vec2, Vec3};
use worldgen::{
    continent::{Continent, SimConfig},
    scale::m,
    settlements::Kind,
    world::GameWorld,
};

use super::{
    day::{self, Now},
    dress, notices,
    lanes::flat,
    talk,
    town::Town,
    *,
};
use crate::{character::Look, journal::Journal};

const SEED: u32 = 3;

fn continent() -> &'static Arc<Continent> {
    static C: OnceLock<Arc<Continent>> = OnceLock::new();
    C.get_or_init(|| GameWorld::with_config(SEED, SimConfig { size: 256 }).continent(0, 0))
}

/// The biggest settlement, a village and a hamlet, made with their people.
fn towns() -> &'static Vec<Town> {
    static T: OnceLock<Vec<Town>> = OnceLock::new();
    T.get_or_init(|| {
        let c = continent();
        let net = &c.trails;
        let mut pick = Vec::new();
        pick.extend((0..net.settlements.len()).max_by_key(|&k| (net.settlements[k].kind, std::cmp::Reverse(k))));
        for want in [Kind::Village, Kind::Hamlet] {
            pick.extend((0..net.settlements.len()).find(|&k| net.settlements[k].kind == want && !pick.contains(&k)));
        }
        pick.into_iter().filter_map(|k| Town::build(c, SEED, (0, 0), k)).collect()
    })
}

/// A day that is not the day of rest. Whether it is market day is the settlement's own
/// (see `market_day_fills_the_stalls_and_other_days_leave_them_bare`).
const WORKDAY: i64 = 7 * 40 + 1;

/// Everyone's day `day` in `town`, in a year of the default length.
fn plans_of(town: &Town, day: i64) -> Vec<day::Plan> {
    day::plans(town, day, crate::season::DEFAULT_YEAR_DAYS)
}

#[test]
fn market_day_fills_the_stalls_and_other_days_leave_them_bare() {
    let mut seen = false;
    for town in towns().iter().filter(|t| !t.stalls.is_empty()) {
        seen = true;
        let days = WORKDAY..WORKDAY + 14;
        let market = days.clone().find(|&d| town.market_on(d)).unwrap_or_else(|| panic!("{}: no market in a fortnight", town.name));
        let ordinary = days.clone().find(|&d| !town.market_on(d) && day::weekday(d) != day::REST_DAY).unwrap();
        let at_stalls = |plans: &[day::Plan], hour: f32| plans.iter().filter(|p| matches!(p.now(hour), Now::At(i) if town.stalls.contains(&p.stays[i].place))).count();
        let plans = plans_of(town, market);
        // Every stall that has a keeper has them behind it through the market's hours.
        let kept: Vec<usize> = town.market.iter().filter_map(|m| m.keeper).collect();
        assert!(!kept.is_empty(), "{}: nobody keeps a stall", town.name);
        for &n in &kept {
            let s = town.keeps(n).unwrap();
            for hour in [9.0, 12.0, 15.5] {
                let Now::At(i) = plans[n].now(hour) else { panic!("{}: keeper {n} walking at {hour}", town.name) };
                assert_eq!(plans[n].stays[i].spot.at, town.behind[s].at, "{}: keeper {n} away at {hour}", town.name);
            }
        }
        // Buyers come and go all morning: more than the keepers at the stalls at its
        // busiest, and different people at different hours.
        let busiest = (0..24).map(|k| at_stalls(&plans, 8.5 + k as f32 * 0.25)).max().unwrap();
        assert!(busiest >= kept.len() + town.stalls.len(), "{}: at most {busiest} at {} stalls", town.name, town.stalls.len());
        let who = |hour: f32| -> Vec<usize> { (0..plans.len()).filter(|&n| !kept.contains(&n) && matches!(plans[n].now(hour), Now::At(i) if town.stalls.contains(&plans[n].stays[i].place))).collect() };
        assert_ne!(who(9.5), who(12.0), "{}: the same buyers all morning", town.name);
        // On another day nobody goes to them, nor before the market opens or after it shuts,
        // but for the stalls set out every day, whose keepers keep them (#203).
        let other = plans_of(town, ordinary);
        let at_plain = |plans: &[day::Plan], hour: f32| plans.iter().filter(|p| matches!(p.now(hour), Now::At(i) if town.stalls.iter().position(|&s| s == p.stays[i].place).is_some_and(|s| !town.kept_daily(s)))).count();
        assert_eq!((0..48).map(|k| at_plain(&other, k as f32 * 0.5)).max(), Some(0), "{}", town.name);
        for &n in kept.iter().filter(|&&n| town.kept_daily(town.keeps(n).unwrap())) {
            let s = town.keeps(n).unwrap();
            let Now::At(i) = other[n].now(12.0) else { panic!("{}: daily keeper {n} walking at noon", town.name) };
            assert_eq!(other[n].stays[i].spot.at, town.behind[s].at, "{}: daily keeper {n} away", town.name);
        }
        assert_eq!(at_stalls(&plans, 6.0) + at_stalls(&plans, 18.0), 0, "{}", town.name);
    }
    assert!(seen, "none of the towns looked at has a stall");
}

#[test]
fn the_same_people_are_in_the_same_places_every_time() {
    let c = continent();
    for town in towns() {
        assert!(!town.residents.is_empty(), "{} has nobody", town.name);
        let again = Town::build(c, SEED, (0, 0), town.k).unwrap();
        let key = |t: &Town| t.residents.iter().map(|r| (r.person.name.clone(), r.person.trade, r.home, r.member, r.role)).collect::<Vec<_>>();
        assert_eq!(key(town), key(&again));
        for day in [WORKDAY, WORKDAY + 2, WORKDAY + 5] {
            let (a, b) = (plans_of(town, day), plans_of(&again, day));
            for hour in [3.0, 7.0, 8.5, 12.3, 19.7, 22.9] {
                for (p, q) in a.iter().zip(&b) {
                    assert_eq!(p.now(hour), q.now(hour));
                    assert_eq!(p.bound(hour).spot.at, q.bound(hour).spot.at);
                }
            }
        }
        // And from one day to the next people keep their own hours.
        let (a, b) = (plans_of(town, WORKDAY), plans_of(town, WORKDAY + 1));
        let differ = a.iter().zip(&b).filter(|(p, q)| p.stays.len() != q.stays.len() || p.stays.iter().zip(&q.stays).any(|(x, y)| x.arrive != y.arrive)).count();
        assert!(differ * 2 > a.len(), "{}: only {differ} of {} have a day of their own", town.name, a.len());
    }
}

#[test]
fn every_household_fills_its_beds() {
    let c = continent();
    for town in towns() {
        let net = &c.trails;
        let plots = &net.plots[net.layouts[town.k].plots.clone()];
        for (i, p) in plots.iter().enumerate() {
            let living = town.residents.iter().filter(|r| r.home == i).count();
            if town.plots[i].place != town.green_place {
                assert_eq!(living, p.sleepers as usize, "{}: plot {i} ({:?})", town.name, p.purpose);
            }
        }
        // Each sleeps in a bed of their own.
        let mut beds: Vec<_> = town.residents.iter().map(|r| (r.bed.at.x.to_bits(), r.bed.at.y.to_bits(), r.bed.at.z.to_bits())).collect();
        beds.sort();
        let n = beds.len();
        beds.dedup();
        assert_eq!(beds.len(), n, "{}: two share a bed", town.name);
    }
}

#[test]
fn at_night_everyone_is_indoors() {
    for town in towns() {
        for day in WORKDAY..WORKDAY + 7 {
            let plans = plans_of(town, day);
            for (r, plan) in town.residents.iter().zip(&plans) {
                for hour in [0.0, 2.5, 4.5, 23.1, 23.9] {
                    match plan.now(hour) {
                        Now::At(i) => {
                            let s = &plan.stays[i];
                            assert_eq!(s.spot.task, Task::Lie, "{} ({:?}) at {hour} on day {day}: {:?}", r.person.name, r.role, s);
                            // Abed at home, or under the lean-to of a camp too far out to
                            // come home from each day (#64).
                            if !(town.stays_out(r) && town.places[s.place].what == town::Where::Camp) {
                                assert_eq!(s.spot.inside, Some(r.home));
                            }
                        }
                        now => panic!("{} ({:?}) is out at {hour} on day {day}: {now:?}", r.person.name, r.role),
                    }
                }
            }
            // And in the day most of its own people are up and about.
            let own: Vec<&day::Plan> = town.residents.iter().zip(&plans).filter(|(r, _)| r.role != town::Role::Visitor).map(|(_, p)| p).collect();
            let up = own.iter().filter(|p| p.bound(10.0).spot.task != Task::Lie).count();
            assert!(up * 10 >= own.len() * 9, "{}: {up} of {} up at ten", town.name, own.len());
        }
    }
}

/// How many are at the tavern at `hour`, who do not live there.
fn at_tavern(town: &Town, plans: &[day::Plan], hour: f32) -> usize {
    let Some(t) = town.tavern else { return 0 };
    town.residents.iter().zip(plans).filter(|(r, p)| r.home != t && matches!(p.now(hour), Now::At(i) if p.stays[i].spot.inside == Some(t))).count()
}

#[test]
fn the_tavern_is_fuller_in_the_evening_than_at_noon() {
    let mut seen = false;
    for town in towns().iter().filter(|t| t.tavern.is_some()) {
        seen = true;
        let plans = plans_of(town, WORKDAY);
        let (noon, evening) = (at_tavern(town, &plans, 12.0), at_tavern(town, &plans, 20.5));
        assert!(evening > noon && evening >= 2, "{}: {noon} at noon, {evening} in the evening", town.name);
        // And fuller still at the end of the week.
        let weekend = plans_of(town, WORKDAY - day::weekday(WORKDAY) + 5);
        assert!(at_tavern(town, &weekend, 20.5) >= evening, "{}", town.name);
    }
    assert!(seen, "none of the towns looked at has a tavern");
}

#[test]
fn the_well_is_busy_in_the_morning_and_work_in_the_day() {
    for town in towns().iter().filter(|t| !t.wells.is_empty()) {
        let plans = plans_of(town, WORKDAY);
        let at = |hour: f32, test: &dyn Fn(&day::Stay) -> bool| plans.iter().filter(|p| matches!(p.now(hour), Now::At(i) if test(&p.stays[i]))).count();
        let mut well = 0;
        for k in 0..40 {
            well = well.max(at(6.0 + k as f32 * 0.05, &|s| town.wells.contains(&s.place)));
        }
        assert!(well >= 2, "{}: at most {well} at the well in the morning", town.name);
        assert_eq!(at(14.0, &|s| town.wells.contains(&s.place)), 0, "{}", town.name);
        let working = at(10.0, &|s| matches!(s.spot.task, Task::Chop | Task::Hoe | Task::Hammer | Task::Fish | Task::Carry | Task::Sweep | Task::Tend | Task::Sit | Task::Talk));
        assert!(working * 3 >= town.residents.len(), "{}: {working} of {} busy at ten", town.name, town.residents.len());
    }
}

/// How far `p` is from the nearest of `ways`, across the ground.
fn off(ways: &[(Vec2, Vec2)], p: Vec2) -> f32 {
    ways.iter()
        .map(|&(a, b)| {
            let ab = b - a;
            let t = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
            (a + ab * t).distance(p)
        })
        .fold(f32::MAX, f32::min)
}

#[test]
fn walking_keeps_to_the_lanes_and_paths() {
    for town in towns() {
        // The lanes, the paths in to every place, and the steps through each door.
        let mut ways: Vec<(Vec2, Vec2)> = town.lanes.ways().map(|(a, b)| (flat(a), flat(b))).collect();
        for p in &town.places {
            let mut last = flat(town.lanes.nodes[p.anchor as usize]);
            for &s in &p.spur {
                ways.push((last, flat(s)));
                last = flat(s);
            }
        }
        for b in &town.plots {
            ways.push((flat(b.doorway), flat(town.places[b.place].at)));
        }
        let mut checked = 0;
        let plans = plans_of(town, WORKDAY);
        // What anyone does is done near where it is reached from: the last steps off the
        // way are few.
        for (r, plan) in town.residents.iter().zip(&plans) {
            for s in plan.stays.iter().filter(|s| s.spot.inside.is_none() && s.place != town.green_place) {
                let d = flat(s.spot.at).distance(flat(town.places[s.place].at));
                assert!(d < 50.0, "{}: {} {} {d:.0} voxels from {}", town.name, r.person.name, s.spot.task.word(), town.place_word(s.place));
            }
        }
        for plan in &plans {
            for w in plan.stays.windows(2) {
                let (a, b) = (&w[0], &w[1]);
                let walk = town.walk((a.place, &a.spot), (b.place, &b.spot));
                let len = walk.length();
                // From room to room indoors there are no lanes to keep to.
                let indoors = a.spot.inside.is_some() && a.spot.inside == b.spot.inside;
                // Out in the land, about a field, the camp, the quarry or a fold, they cross
                // it as they like, as on the green (#64).
                let in_land = a.place == b.place && town.places[a.place].what.out() && a.spot.inside.is_none();
                let steps = (len / 2.0).ceil() as usize;
                for k in (0..=steps).filter(|_| !indoors) {
                    let (p, _) = walk.at(len * k as f32 / steps.max(1) as f32);
                    let p2 = flat(p);
                    // Leaving a spot or coming to one, they step off the way, from the
                    // place it is at (the well, the door) to it, round the house if it is
                    // in the way; on the green they cross it as they like.
                    let last = |s: &day::Stay| {
                        let mut at = flat(town.places[s.place].at);
                        let mut legs = Vec::new();
                        for step in town.last_steps(s.place, &s.spot).1 {
                            legs.push((at, flat(step)));
                            at = flat(step);
                        }
                        off(&legs, p2) < 1.5
                    };
                    let near_end = last(a) || last(b);
                    let on_green = p2.distance(flat(town.centre)) < town.green * 0.5 + 1.0;
                    let d = off(&ways, p2);
                    assert!(d < 1.5 || near_end || on_green || in_land, "{}: a walk strays {d:.1} voxels from the lanes at {p}", town.name);
                    // And never through a wall: into a building only by its door.
                    for x in &town.plots {
                        let within = x.within(p2, -0.3);
                        assert!(
                            !within || p2.distance(flat(x.doorway)) < 3.0,
                            "{}: a walk from {} ({:?}) to {} ({:?}) goes through the walls of a {} ({}..{}, round {}, door {} out {}) at {p}",
                            town.name,
                            town.place_word(a.place),
                            a.spot,
                            town.place_word(b.place),
                            b.spot,
                            x.purpose.word(),
                            x.lo,
                            x.hi,
                            x.round,
                            x.doorway,
                            x.out
                        );
                    }
                    checked += 1;
                }
                // The time allowed for it is the time it takes.
                let allowed = (b.arrive - a.leave) * day::PACE;
                assert!((allowed - len).abs() < 1.0 + len * 0.02, "{}: {allowed} allowed for a walk of {len}", town.name);
            }
        }
        assert!(checked > 100, "{}: only {checked} points walked", town.name);
    }
}

#[test]
fn the_lanes_hold_together() {
    for town in towns() {
        let reached = town.lanes.reach(0);
        let n = reached.iter().filter(|&&r| r).count();
        assert!(n * 10 >= town.lanes.nodes.len() * 9, "{}: {n} of {} lane nodes reached from the green", town.name, town.lanes.nodes.len());
        // Nearly every home has its way in.
        let homes = town.plots.iter().filter(|b| b.place != town.green_place).count();
        assert!(homes * 10 >= town.plots.len() * 9, "{}: {homes} of {} buildings reached", town.name, town.plots.len());
    }
}

/// Every look the people of a settlement have, each carrying what each task needs.
fn looks() -> Vec<Look> {
    towns().iter().flat_map(|t| t.residents.iter().map(|r| r.person.look)).collect()
}

#[test]
fn every_figure_keeps_to_its_boxes() {
    let cam = DVec3::new(30.0, 12.0, -20.0);
    let mut most = (0, 0);
    for look in looks() {
        for t in Task::ALL {
            let look = Look { carried: t.carried(look.carried), ..look };
            let pose = character::task_pose(t, &look.build, 1.3, 0.4);
            let mut out = Vec::new();
            character::draw_stool(DVec3::ZERO, 0.3, cam, &mut out);
            character::draw_at(&look, DVec3::ZERO, 0.3, &pose, t, cam, &mut out);
            most.0 = most.0.max(out.len());
            out.clear();
            character::draw_far(&look, DVec3::ZERO, 0.3, &pose, cam, &mut out);
            most.1 = most.1.max(out.len());
        }
        for carried in [look.carried, Carried::Bucket, Carried::Load] {
            let look = Look { carried, ..look };
            let mut g = Gait::new(0.0);
            g.update(Doing::Afoot(1.3), 3.3, None, false, None, 0.0, 0.0);
            let mut out = Vec::new();
            character::draw(&look, DVec3::ZERO, 0.3, &g.pose(), None, cam, &mut out);
            most.0 = most.0.max(out.len());
        }
    }
    assert!(most.0 <= FIGURE_BOXES && most.1 <= FAR_BOXES, "{most:?} boxes, near and far");
    println!("the most boxes to a figure: {} whole, {} far; {MOST_BOXES} in all at most", most.0, most.1);
}

#[test]
fn a_town_full_of_people_is_drawn_within_its_budget() {
    for town in towns() {
        let mut v = Villagers::new();
        let k = town.k;
        v.add(Town::build(continent(), SEED, (0, 0), k).unwrap(), WORKDAY, 0);
        let centre = town.origin + town.centre.as_dvec3();
        for (hour, eye) in [(12.0, centre + DVec3::Y * m(30.0) as f64), (7.0, centre + DVec3::Y * m(1.7) as f64), (19.5, centre)] {
            let mut out = Vec::new();
            v.draw(centre, eye, hour, &mut out);
            let d = v.drawn;
            assert!(d.near <= MOST_NEAR && d.far <= MOST_FAR && d.boxes <= MOST_BOXES && out.len() == d.boxes, "{}: {d:?}", town.name);
            if town.residents.len() > 30 && hour == 12.0 {
                assert!(d.near + d.far >= 10, "{}: only {d:?} at noon", town.name);
            }
            println!("{} at {hour}: {d:?}, {} in sight", town.name, v.seen().len());
        }
        // At night nobody is out, and nobody is drawn but from inside.
        let mut out = Vec::new();
        v.draw(centre, centre + DVec3::Y * m(30.0) as f64, 2.0, &mut out);
        assert_eq!(v.drawn.boxes, 0, "{}", town.name);
    }
}

#[test]
fn someone_indoors_is_seen_only_from_inside() {
    let town = &towns()[0];
    let mut v = Villagers::new();
    v.add(Town::build(continent(), SEED, (0, 0), town.k).unwrap(), WORKDAY, 0);
    let r = &town.residents[0];
    let b = &town.plots[r.home];
    let bed = town.origin + r.bed.at.as_dvec3();
    // From the doorway inside, at night, they are in bed; from the lane, they are not.
    let inside = town.origin + (b.doorway - Vec3::new(b.out.x, 0.0, b.out.y) * 1.5).as_dvec3();
    let mut out = Vec::new();
    v.draw(inside, inside + DVec3::Y * m(1.6) as f64, 2.0, &mut out);
    assert!(v.seen().iter().any(|f| f.who.n == r.n && f.task == Some(Task::Lie)), "{} not seen in bed at {bed}", r.person.name);
    let out_there = town.origin + town.places[b.place].at.as_dvec3() + DVec3::new(b.out.x as f64, 0.0, b.out.y as f64) * 4.0;
    v.draw(out_there, out_there + DVec3::Y * m(1.6) as f64, 2.0, &mut out);
    assert!(v.seen().is_empty());
}

#[test]
fn a_greeting_names_real_places() {
    let c = continent();
    let names = &c.trails.names;
    let real: Vec<String> = names.places.iter().map(|p| p.name.clone()).collect();
    for town in towns() {
        for (k, r) in town.residents.iter().enumerate().take(40) {
            let met = (k % 6) as u32;
            let line = talk::greeting(town, r, met, 6.0 + (k % 16) as f32, (k % 4) as f32 * 0.25 + 0.1);
            assert!(!line.contains('{') && !line.contains('}'), "{line}");
            // A name with its own article is said without it after "the".
            let named = real.iter().any(|n| line.contains(n.trim_start_matches("The "))) || line.contains(&town.name) || line.contains("village tree");
            assert!(named, "{} of {} says '{line}', naming nowhere", r.person.name, town.name);
            assert!(["spring", "summer", "autumn", "winter"].iter().any(|s| line.contains(s)) || line.contains(&town.name) || town.near.as_ref().is_some_and(|n| line.contains(n.as_str())) || town.river.as_ref().is_some_and(|n| line.contains(n.trim_start_matches("The "))) || line.contains("tree"), "{line}");
            assert!(!line.to_lowercase().contains("the the "), "an article twice: {line}");
        }
        assert!(real.contains(&town.name), "{} is not a name of the gazetteer", town.name);
    }
}

#[test]
fn every_line_fills() {
    for name in talk::SECTIONS {
        let lines = talk::lines_of(name);
        assert!(!lines.is_empty(), "lines.txt has no [{name}]");
        for l in lines {
            let slots = l.matches('{').count();
            assert_eq!(slots, l.matches('}').count(), "{l}");
        }
    }
}

#[test]
fn greeting_someone_stops_them_and_the_save_remembers() {
    let town = &towns()[0];
    let mut v = Villagers::new();
    v.add(Town::build(continent(), SEED, (0, 0), town.k).unwrap(), WORKDAY, 0);
    // Stand beside someone at work at ten, and greet them.
    let plans = plans_of(town, WORKDAY);
    let (r, stay) = town.residents.iter().zip(&plans).find_map(|(r, p)| matches!(p.now(10.0), Now::At(i) if p.stays[i].spot.inside.is_none()).then(|| (r, *p.bound(10.0)))).expect("nobody out at ten");
    let feet = town.origin + (stay.spot.at + Vec3::new(1.5, 0.0, 0.0)).as_dvec3();
    let mut out = Vec::new();
    v.draw(feet, feet + DVec3::Y * m(1.6) as f64, 10.0, &mut out);
    let (who, name) = v.within_reach(feet).expect("nobody within reach");
    let name = name.to_string();
    let mut journal = Journal::new();
    let met = journal.met(&who.key(), &name);
    assert_eq!(met, 0);
    let said = v.greet(who, feet, met, 10.0, 0.3).unwrap();
    assert!(said.line.contains(&said.town) || said.line.len() > 10, "{}", said.line);
    assert_eq!(journal.meet(&who.key(), &said.name, &said.town), 1);
    assert_eq!(journal.meet(&who.key(), &said.name, &said.town), 2);
    // They stop and turn to the player.
    v.draw(feet, feet + DVec3::Y * m(1.6) as f64, 10.2, &mut out);
    let f = v.seen().iter().find(|f| f.who == who).unwrap();
    assert_eq!(f.task, Some(Task::Talk));
    let to = DVec3::new(feet.x - f.feet.x, 0.0, feet.z - f.feet.z);
    assert!((to.z.atan2(to.x) as f32 - f.facing).abs() < 1e-3, "{} does not face the player", r.person.name);
    // The save keeps the count, and a note of the first meeting.
    let back = Journal::from_text(&journal.to_text());
    assert_eq!(back.met(&who.key(), &name), 2);
    assert_eq!(back.people_met().len(), 1);
    assert!(back.notes().any(|n| n.text.contains(&name)));
    // Someone else with the same number in a world made since is met afresh.
    assert_eq!(back.met(&who.key(), "Someone Else"), 0);
}

#[test]
fn the_notice_board_names_real_people_and_places_and_is_the_same_every_time() {
    let names = &continent().trails.names;
    for town in towns() {
        let mut texts = std::collections::HashSet::new();
        for week in 40..52 {
            let board = notices::for_week(town, week);
            assert_eq!(board, notices::for_week(town, week), "{}", town.name);
            assert!((1..=2).contains(&board.len()), "{}: {} requests", town.name, board.len());
            for r in &board {
                assert_eq!(town.residents[r.who].person.name, r.name);
                assert!(r.text.contains(&r.name), "{}", r.text);
                match &r.errand {
                    notices::Errand::Lost { near, .. } => assert!(names.named(near).is_some() && r.text.contains(near.as_str()), "{}: {near} is no place", r.text),
                    notices::Errand::Carry { to, town: there, .. } => {
                        assert_eq!(names.settlement(*to).map(|p| &p.name), Some(there));
                        assert!(*to != town.k && r.text.contains(there.as_str()));
                    }
                    notices::Errand::Gather { count, .. } => assert!((3..=8).contains(count)),
                }
                texts.insert(r.text.clone());
            }
        }
        assert!(texts.len() >= 8, "{}: only {} requests in twelve weeks", town.name, texts.len());
    }
    // Read, they go in the journal as offered, once each, and the save keeps them.
    let town = &towns()[0];
    let mut journal = Journal::new();
    let board = notices::for_week(town, 40);
    for r in &board {
        assert!(journal.offer(&r.key, &r.name, &town.name, &r.text));
        assert!(!journal.offer(&r.key, &r.name, &town.name, &r.text));
    }
    let back = Journal::from_text(&journal.to_text());
    let said = |j: &Journal| j.offered().iter().map(|o| (o.key.clone(), o.name.clone(), o.town.clone(), o.text.clone())).collect::<Vec<_>>();
    assert_eq!(said(&back), said(&journal));
    assert_eq!(back.offered().len(), board.len());
}

#[test]
fn the_wayfarers_mark_the_country_about_on_the_map() {
    use worldgen::settlements::special::Guild;
    let c = continent();
    let net = &c.trails;
    let real = (0..net.layouts.len()).find(|&k| net.guild_halls.iter().any(|g| net.layouts[k].plots.contains(&g.0) && g.1.contains(&Guild::Wayfarers)));
    let mut town = Town::build(c, SEED, (0, 0), real.unwrap_or(towns()[0].k)).unwrap();
    if real.is_none() {
        // The test continent is too small for a guild house: the biggest settlement's
        // tavern, and its board, stand in for one.
        let tavern = town.tavern.expect("no tavern");
        assert!(town.boards.iter().any(|b| b.0 == tavern), "the tavern has no board");
        town.guilds = Some((tavern, vec![Guild::Merchants, Guild::Wayfarers]));
    }
    let (plot, guilds) = town.guilds.clone().expect("the guild house is not known");
    let &(_, at, facing) = town.boards.iter().find(|b| b.0 == plot).expect("the guild house has no board");
    // Stood before its board, looking at it.
    let eye = at + Vec3::new(facing.x, 0.0, facing.y) * m(1.4);
    let forward = (at - eye).normalize();
    let eye = town.origin + eye.as_dvec3();
    let mut v = Villagers::new();
    v.add(town.clone(), WORKDAY, 0);
    let (which, word) = v.board_in_sight(eye, forward).expect("the board is not in sight");
    assert_eq!(word, "the guilds' board");
    assert!(v.board_in_sight(eye, -forward).is_none(), "read with its back to it");
    let read = v.read(which, |_| false).unwrap();
    for g in &guilds {
        assert!(read.page.lines.iter().any(|l| l.0 == g.name() && l.1.len() > 60), "{} not on the board", g.name());
    }
    let (places, middle, reach) = read.chart.clone().expect("no chart from the wayfarers");
    assert_eq!(places.len(), town.neighbours.len());
    for (name, p) in &places {
        assert!(net.names.named(name).is_some_and(|p| p.kind.is_settlement()), "{name}");
        assert!(p.distance(middle) <= reach, "{name} is beyond the chart");
        assert!(read.page.text().contains(name.as_str()), "{name} is not said to be marked");
    }
    // The journal keeps it, once, and the save with it; read again, the board says the
    // country is marked already.
    let mut journal = Journal::new();
    assert!(journal.chart(&read.key, &read.town, middle, reach));
    assert!(!journal.chart(&read.key, &read.town, middle, reach));
    let back = Journal::from_text(&journal.to_text());
    assert_eq!(back.charted(), journal.charted());
    let again = v.read(which, |key| back.charted().iter().any(|c| c.key == key)).unwrap();
    assert!(again.page.text().contains("marked on your map from their charts"), "{}", again.page.text());
}

#[test]
fn market_days_and_festivals_are_dressed_within_their_budget() {
    use worldgen::settlements::calendar::{Festival, festival_on};
    let year_days = 24.0;
    let when = |day: i64, hour: f32, lit: f32| dress::When { day, date: ((day as f32 + 0.5) / year_days).fract(), year_days, hour, lit, clock: 1.0 };
    let town = towns().iter().find(|t| t.market.iter().any(|m| m.keeper.is_some() && m.awning.is_some())).expect("no stall to dress");
    let plain = |d: i64| festival_on(d, year_days).is_none();
    let market = (WORKDAY..WORKDAY + 30).find(|&d| town.market_on(d) && plain(d)).unwrap();
    let ordinary = (WORKDAY..WORKDAY + 30).find(|&d| !town.market_on(d) && plain(d)).unwrap();
    let stall = town.places[town.stalls[0]].at;
    let count = |w: dress::When, at: Vec3| {
        let mut out = Vec::new();
        let n = dress::draw(std::iter::once((town, w)), town.origin + (at + Vec3::Y * m(1.6)).as_dvec3(), &mut out);
        assert!(n == out.len() && n <= dress::BUDGET, "{n} boxes");
        n
    };
    // The stalls are dressed through the market's hours on market day, and not before
    // it opens, nor on another day.
    let open = count(when(market, 12.0, 0.0), stall);
    assert!(open > count(when(ordinary, 12.0, 0.0), stall) + 20, "{open} boxes on market day");
    assert_eq!(count(when(market, 6.0, 0.0), stall), count(when(ordinary, 12.0, 0.0), stall));
    // At Midwinter lanterns hang, lit at night; at Midsummer garlands go up.
    let winter = (0..48).find(|&d| festival_on(d, year_days) == Some(Festival::Midwinter)).unwrap();
    let summer = (0..48).find(|&d| festival_on(d, year_days) == Some(Festival::Midsummer)).unwrap();
    let ord = (0..48).find(|&d| plain(d) && !town.market_on(d)).unwrap();
    let middle = town.centre;
    assert!(count(when(winter, 22.0, 1.0), middle) > count(when(ord, 22.0, 1.0), middle), "no lanterns at Midwinter");
    assert!(count(when(summer, 12.0, 0.0), middle) > count(when(ord, 12.0, 0.0), middle), "no garlands at Midsummer");
    let lamps = |w: dress::When| dress::lamps(std::iter::once((town, w)), town.origin + (town.spare.first().copied().or(town.posts.first().map(|p| p.0)).unwrap_or(middle)).as_dvec3());
    assert!(!town.spare.is_empty() || !town.posts.is_empty(), "{}: nowhere to hang a lantern", town.name);
    assert!(!lamps(when(winter, 22.0, 1.0)).is_empty(), "no lantern lit at Midwinter");
    assert!(lamps(when(winter, 12.0, 0.0)).is_empty() && lamps(when(ord, 22.0, 1.0)).is_empty());
}

#[test]
fn a_day_turns_at_midnight() {
    let days = 24.0;
    let d = |hour: f32, date: f32| day_number(hour, date, 2, days);
    // An hour of the day moves the date on by a 24th of a day.
    let step = 1.0 / days / 24.0;
    let start = 0.31;
    let base = d(10.0, start);
    for h in 0..14 {
        assert_eq!(d(10.0 + h as f32, start + step * h as f32), base, "at {}", 10 + h);
    }
    assert_eq!(d(0.5, start + step * 14.5), base + 1);
}

/// Over the years (#207) nobody lives in a home standing empty, taken down or rising
/// again, a home built again has a new household, and everyone keeps the name the save
/// knows them by.
#[test]
fn nobody_lives_in_a_home_not_standing() {
    use worldgen::settlements::years::{Boons, Lives, When};
    let c = continent();
    let net = &c.trails;
    let mut empty_seen = 0;
    let mut anew = 0;
    for town in towns() {
        let first = net.layouts[town.k].plots.start;
        let today = Town::build(c, SEED, (0, 0), town.k).unwrap();
        // In year 0 everyone is known by their number, as before there were years; those
        // in from the hamlets for the market (#207) are not of it.
        assert!(today.residents.iter().filter(|r| r.role != town::Role::Visitor).all(|r| r.id == r.n));
        for (year, season) in [(1, 2), (2, 0), (3, 1), (5, 2), (9, 1), (14, 3)] {
            let lives = Lives::new(net, SEED, (0, 0), When::new(year, season), &Boons::default());
            let then = Town::build_in(c, SEED, (0, 0), town.k, &lives).unwrap();
            for r in then.residents.iter().filter(|r| r.role != town::Role::Visitor) {
                assert!(lives.standing(first + r.home), "{} lives in a home not standing in year {year}", r.person.name);
            }
            let homes: Vec<usize> = (0..then.plots.len()).filter(|&i| net.plots[first + i].sleepers > 0).collect();
            empty_seen += homes.iter().filter(|&&i| !lives.standing(first + i)).count();
            // The same people where nothing has changed, known by the same names.
            for r in then.residents.iter().filter(|r| r.role != town::Role::Visitor) {
                match today.residents.iter().find(|q| q.id == r.id) {
                    Some(q) if lives.generation(first + r.home) == 0 => assert_eq!(q.person.name, r.person.name),
                    Some(_) => panic!("a new household known by an old one's name"),
                    None => {
                        assert!(lives.generation(first + r.home) > 0, "{} unknown in year {year}", r.person.name);
                        anew += 1;
                    }
                }
            }
        }
    }
    assert!(empty_seen > 0, "no home ever stood empty");
    assert!(anew > 0, "no home was ever built again for a new household");
}

/// Whose plan takes them out to work at a place out in the land, and to which.
fn out_at(town: &Town, plan: &day::Plan) -> Option<usize> {
    plan.stays.iter().find(|s| town.places[s.place].what.out()).map(|s| s.place)
}

#[test]
fn farmers_go_out_to_a_field_of_their_own_and_are_home_by_dusk() {
    let mut out = 0;
    for town in towns() {
        for f in &town.fields {
            assert_eq!(f.plot.owner, town.k, "{}: a field of another settlement's", town.name);
        }
        // A summer's working day, and a winter's.
        for day in [WORKDAY, WORKDAY + 7 * 9] {
            let date = (day as f32 / crate::season::DEFAULT_YEAR_DAYS).rem_euclid(1.0);
            let dusk = day::sunset(date);
            let plans = plans_of(town, day);
            for (r, plan) in town.residents.iter().zip(&plans) {
                let Some(place) = out_at(town, plan) else { continue };
                if let town::Where::Field(f) = town.places[place].what {
                    assert!(town.fields.get(f).is_some(), "{}: {} goes to a field not of theirs", town.name, r.person.name);
                    assert_eq!(r.person.trade, crate::people::Trade::Farmer, "{}: {} in the fields", town.name, r.person.name);
                }
                // At work there through the middle of the day.
                assert!(town.places[plan.bound(11.0).place].what.out(), "{}: {} not out at work at 11", town.name, r.person.name);
                out += 1;
                // Home before the sun is down, and in by the evening; but those who stay
                // out at a camp too far for that sleep under its lean-to until the last
                // working day.
                if town.stays_out(r) && day::weekday(day) < day::REST_DAY - 1 {
                    let night = plan.stays.last().unwrap();
                    assert!(town.places[night.place].what == town::Where::Camp && night.spot.task == Task::Lie, "{}: {} not abed at the camp", town.name, r.person.name);
                    continue;
                }
                let home = town.plots[r.home].place;
                let back = plan.stays.iter().rev().find(|s| town.places[s.place].what.out()).map_or(0.0, |s| s.leave);
                let next = plan.stays.iter().skip_while(|s| s.leave < back).nth(1);
                let (to, walk) = next.map_or((usize::MAX, 24.0), |s| (s.place, s.arrive));
                assert!(to == home && walk <= dusk, "{}: {} home from {} at {walk:.2}, dusk at {dusk:.2}", town.name, r.person.name, town.place_word(place));
            }
        }
        // Those who stay out at the camp come home on the last working day, by dusk.
        let last = WORKDAY - day::weekday(WORKDAY) + day::REST_DAY - 1;
        let date = (last as f32 / crate::season::DEFAULT_YEAR_DAYS).rem_euclid(1.0);
        for (r, plan) in town.residents.iter().zip(&plans_of(town, last)).filter(|(r, _)| town.stays_out(r)) {
            let home = plan.stays.iter().find(|s| s.place == town.plots[r.home].place && s.spot.inside.is_some()).map_or(24.0, |s| s.arrive);
            assert!(home <= day::sunset(date), "{}: {} home from the camp at {home:.2}", town.name, r.person.name);
        }
        // Nobody's household is left with nobody at home.
        let plans = plans_of(town, WORKDAY);
        for home in 0..town.plots.len() {
            let all: Vec<usize> = town.residents.iter().filter(|r| r.home == home).map(|r| r.n).collect();
            let grown: Vec<usize> = all.iter().copied().filter(|&n| town.residents[n].role != town::Role::Child).collect();
            if grown.is_empty() || grown.iter().any(|&n| town.drives(n).is_some()) {
                continue;
            }
            assert!(grown.iter().any(|&n| out_at(town, &plans[n]).is_none()), "{}: the whole household of a {} goes out to the land", town.name, town.plots[home].purpose.word());
        }
    }
    assert!(out > 0, "nobody went out to work in the land");
}

#[test]
fn the_flocks_are_driven_out_and_home_and_the_hens_fed() {
    let mut droves = 0;
    let mut fed = 0;
    for town in towns() {
        let plans = plans_of(town, WORKDAY);
        for d in &town.droves {
            let hours = d.herd.hours(WORKDAY, day::sunset((WORKDAY as f32 / crate::season::DEFAULT_YEAR_DAYS).rem_euclid(1.0)));
            use crate::animals::herd::Stage;
            assert_eq!(d.herd.now(&hours, 6.0).0, Stage::Folded, "{}: a flock out before dawn", town.name);
            assert_eq!(d.herd.now(&hours, 10.0).0, Stage::Grazing, "{}: a flock not out by mid-morning", town.name);
            assert_eq!(d.herd.now(&hours, 19.0).0, Stage::Folded, "{}: a flock not in by evening", town.name);
            // Its herder by the fold as the gate is opened and as it is shut, out with
            // the flock in between, and home after.
            let Some(n) = d.herder else { continue };
            let plan = &plans[n];
            for hour in [hours.open, hours.shut - 0.05, 11.0] {
                assert_eq!(plan.bound(hour).place, d.place, "{}: the herder away from the fold at {hour:.2}", town.name);
            }
            assert_eq!(plan.bound(20.0).place, town.plots[town.residents[n].home].place, "{}: the herder not home in the evening", town.name);
            if let Some((a, b)) = hours.milk {
                assert_eq!(plan.bound((a + b) * 0.5).spot.task, crate::character::Task::Milk, "{}: nobody milking at {a:.2}..{b:.2}: {:?} {:?}", town.name, hours, plan.stays.iter().map(|s| (s.arrive, s.leave, s.spot.task.word())).collect::<Vec<_>>());
            }
            droves += 1;
        }
        for h in &town.hens {
            let Some(n) = h.feeder else { continue };
            let (from, until) = crate::animals::herd::feeding(h.key, WORKDAY);
            let s = plans[n].bound((from + until) * 0.5);
            if s.spot.task == crate::character::Task::Feed {
                fed += 1;
            }
        }
    }
    assert!(droves > 0, "no flock driven out");
    assert!(fed > 0, "no hens fed");
}

#[test]
fn folk_come_in_from_the_hamlets_only_on_market_day() {
    let mut markets = 0;
    for town in towns().iter().filter(|t| !t.stalls.is_empty() && !t.hamlets.is_empty()) {
        let visitors: Vec<usize> = town.residents.iter().filter(|r| r.role == town::Role::Visitor).map(|r| r.n).collect();
        assert!(!visitors.is_empty(), "{}: nobody from its hamlets", town.name);
        let away = |s: &day::Stay| s.spot.inside == Some(town::AWAY);
        for day in WORKDAY..WORKDAY + 14 {
            let plans = plans_of(town, day);
            let came: Vec<usize> = visitors.iter().copied().filter(|&n| plans[n].stays.iter().any(|s| !away(s))).collect();
            if !town.market_on(day) {
                assert!(came.is_empty(), "{}: {} in from the hamlets on day {day}, no market day", town.name, came.len());
                continue;
            }
            markets += 1;
            assert!(came.len() * 4 >= visitors.len() * 3, "{}: only {} of {} in for the market", town.name, came.len(), visitors.len());
            for &n in &came {
                let p = &plans[n];
                let name = &town.residents[n].person.name;
                // Away in the night, in by the middle of the day, and home by evening.
                assert!(away(p.bound(6.0)) && away(p.bound(19.0)), "{}: {name} not away at night", town.name);
                let arrive = p.stays.iter().find(|s| !away(s)).map_or(24.0, |s| s.arrive);
                assert!(arrive < 12.5, "{}: {name} in at {arrive:.2}", town.name);
                // At the stalls while they are kept.
                assert!(p.stays.iter().any(|s| town.stalls.contains(&s.place) && s.arrive < worldgen::settlements::calendar::MARKET_CLOSES), "{}: {name} never at a stall", town.name);
            }
        }
    }
    assert!(markets > 0, "no market day with folk in from the hamlets");
}

#[test]
fn the_grain_is_reaped_while_it_stands_gold_and_the_sheaves_carried_off_the_stubble() {
    use worldgen::{
        features::plants::{Phase, Season},
        settlements::farmland::{Crop, crops},
    };
    let season = |row: u8| crops::seasons().into_iter().find(|(r, _)| *r == row).unwrap().1;
    let (grain, stubble) = (season(crops::GRAIN), season(crops::STUBBLE));
    // How much of a field shows one of `phases` of `s` on `date`, its plants coming out
    // a few days apart.
    let share = |s: &Season, date: f32, phases: &[Phase]| {
        let n = (0..400).filter(|&i| phases.contains(&s.phase(date, i % 20, i / 20))).count();
        n as f32 / 400.0
    };
    let mut reaped = 0;
    for i in 0..400 {
        let date = i as f32 / 400.0;
        for crop in [Crop::Wheat, Crop::Barley] {
            for second in [false, true] {
                match day::field_task(crop, date, second) {
                    Task::Reap => {
                        reaped += 1;
                        assert!(share(&grain, date, &[Phase::Bloom, Phase::Seed]) >= 0.5, "{crop:?} reaped at {date} with the grain not gold");
                    }
                    Task::Sheaves if !second => {
                        assert!(share(&stubble, date, &[Phase::Bloom]) >= 0.5, "{crop:?} sheaves carried at {date} with no stubble");
                    }
                    _ => {}
                }
            }
        }
        // With the whole field gold, it is being reaped.
        if share(&grain, date, &[Phase::Bloom]) == 1.0 {
            assert_eq!(day::field_task(Crop::Wheat, date, false), Task::Reap, "the wheat stands gold at {date} and nobody reaps it");
        }
    }
    assert!(reaped > 0);
}

/// Everyone of `town` at `hour` by `plans` as drawn from `eye`, stepping round each other
/// as they walk or, without `keep_off`, each going their own way regardless.
fn placed_at(town: &Town, plans: &[day::Plan], hour: f32, eye: Vec3, routes: &mut avoid::Routes, scratch: &mut avoid::Scratch, keep_off: bool) -> (Vec<Option<(Placed, avoid::State)>>, f64) {
    let mut figs: Vec<_> = town.residents.iter().map(|r| figure(town, &plans[r.n], r, hour, eye, routes)).collect();
    let started = std::time::Instant::now();
    if keep_off {
        avoid::keep_off(town, plans, eye, routes, &mut figs, scratch);
    }
    (figs, started.elapsed().as_secs_f64() * 1e6)
}

/// What a day of walking about `town` came to: how often a walker was in someone's way.
#[derive(Default, Debug)]
struct Crowding {
    /// Moments a walker on the way touched someone still, or another walker: at all, a
    /// little into them (their figures meeting) and more than half a body into them; and
    /// those within a few steps of either end of their walk or of a doorway, where they
    /// come to their spot or go in, and those indoors.
    still: [usize; 3],
    passing: [usize; 3],
    at_ends: usize,
    indoors: usize,
    /// Of those on the way a little into someone, how many had room beside it to step
    /// aside into.
    with_room: usize,
    /// The nearest any two came, edge to edge (below 0, into each other), in voxels.
    nearest: f32,
    /// The biggest jump from one look to the next beyond what walking allows, in voxels,
    /// and how many moments were looked at.
    jump: f32,
    jump_at: String,
    jump_who: Option<(usize, f32)>,
    looks: usize,
    /// How long stepping aside took a look, in microseconds, on the whole and at the
    /// most, and how many were walking at once at the most.
    took: f64,
    most: f64,
    walkers: usize,
    /// Where the walkers on the way were in someone's way most, and who was in theirs.
    worst: std::collections::BTreeMap<String, usize>,
    example: std::collections::BTreeMap<String, String>,
    /// How many times anyone jumped from one look to the next.
    jumps: usize,
    /// Two walkers coming abreast going opposite ways, away from the ends of their walks:
    /// how many stepped aside the same way each (both to their right, or both to their
    /// left), and how many one one way and one the other, as if they were dancing.
    met: [usize; 2],
    /// Moments a walker coming to a crowd before a well, a stall, the tree, the shrine or
    /// the water, or leaving one, touched someone standing in it, was a little into them
    /// and more than half a body into them, wherever along their walk they were (#206).
    crowd: [usize; 3],
}

/// How far into each other two people are, edge to edge, before their figures meet, in
/// voxels: a body is reckoned a little broader than a figure is drawn.
const INTO: f32 = 0.3;

/// Samples a whole day of `town` every three seconds of the game's clock, from the eye at
/// its middle, for how often walkers are in each other's way and anyone at a stay's.
fn crowding(town: &Town, day: i64, keep_off: bool) -> Crowding {
    let plans = plans_of(town, day);
    let eye = town.centre + Vec3::Y * 4.0;
    let mut routes = avoid::Routes::new();
    let mut scratch = avoid::Scratch::default();
    let mut c = Crowding { nearest: f32::MAX, ..Default::default() };
    let dt = 3.0 / 3600.0;
    // A walker goes this far a look at most, a little more stepping aside.
    let allowed = day::PACE * dt * 1.6 + 0.02;
    let mut last: Vec<Option<(Placed, avoid::State)>> = Vec::new();
    let doors: Vec<Vec2> = town.plots.iter().map(|b| flat(b.doorway)).collect();
    let mut k = 0;
    loop {
        let hour = 5.5 + k as f32 * dt;
        if hour > 22.8 {
            break;
        }
        k += 1;
        let (figs, took) = placed_at(town, &plans, hour, eye, &mut routes, &mut scratch, keep_off);
        c.looks += 1;
        c.took += took;
        c.most = c.most.max(took);
        c.walkers = c.walkers.max(figs.iter().filter(|f| matches!(f, Some((_, avoid::State::Going(..))))).count());
        // Who is near whom, by squares.
        let mut squares: std::collections::HashMap<(i32, i32), Vec<usize>> = std::collections::HashMap::new();
        for (n, f) in figs.iter().enumerate() {
            if let Some((p, _)) = f {
                squares.entry(((p.0.x / 8.0).floor() as i32, (p.0.z / 8.0).floor() as i32)).or_default().push(n);
            }
        }
        for (n, f) in figs.iter().enumerate() {
            let Some((p, avoid::State::Going(i, d))) = f else { continue };
            let Some(route) = routes.get(&(n, *i)) else { continue };
            let here = flat(p.0);
            let length = route.walk.length();
            let end = d.min(length - d) < 2.5 || doors.iter().any(|q| q.distance(here) < 3.0);
            let (sx, sz) = ((p.0.x / 8.0).floor() as i32, (p.0.z / 8.0).floor() as i32);
            for dz in -1..=1 {
                for dx in -1..=1 {
                    for &m in squares.get(&(sx + dx, sz + dz)).into_iter().flatten() {
                        let Some((q, state)) = &figs[m] else { continue };
                        if m == n || q.6 != p.6 || (q.0.y - p.0.y).abs() > 3.0 {
                            continue;
                        }
                        let walking = matches!(state, avoid::State::Going(..));
                        // Each pair of walkers once.
                        if walking && m < n {
                            continue;
                        }
                        let (a, b, r) = match state {
                            avoid::State::At(s) => avoid::outline(&plans[m].stays[*s].spot, character::lying_length(&town.residents[m].person.look.build)),
                            _ => (flat(q.0), flat(q.0), avoid::BODY),
                        };
                        let ab = b - a;
                        let t = ((here - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
                        let gap = (a + ab * t).distance(here) - avoid::BODY - r;
                        if let avoid::State::Going(j, e) = state
                            && let Some(other) = routes.get(&(m, *j))
                            && !end
                            && e.min(other.walk.length() - e) > 2.5
                            && gap < 1.0
                        {
                            // How far each is off their way, to their right.
                            let off = |walk: &super::lanes::Walk, d: f32, at: Vec2| (at - flat(walk.at(d).0)).dot(walk.heading(d, 4.0).perp());
                            let (way, other_way) = (route.walk.heading(*d, 4.0), other.walk.heading(*e, 4.0));
                            let (mine, theirs) = (off(&route.walk, *d, here), off(&other.walk, *e, flat(q.0)));
                            if way.dot(other_way) < -0.8 && (flat(q.0) - here).dot(way).abs() < 0.5 && mine.abs().min(theirs.abs()) > 0.1 {
                                c.met[((mine > 0.0) != (theirs > 0.0)) as usize] += 1;
                            }
                        }
                        c.nearest = c.nearest.min(gap);
                        if gap >= 0.0 {
                            continue;
                        }
                        if let avoid::State::At(s) = state {
                            let there = plans[m].stays[*s].place;
                            let coming = plans[n].stays[*i + 1].place == there;
                            let mine = if coming { &plans[n].stays[*i + 1].spot } else { &plans[n].stays[*i].spot };
                            // Not someone come to the very spot another is still at, as
                            // those who come later to a stall take the places of those gone.
                            let same = flat(mine.at).distance(flat(plans[m].stays[*s].spot.at)) < avoid::BODY * 2.0;
                            if town.places[there].what.crowd() && !same && (plans[n].stays[*i].place == there || coming) {
                                for (k, deep) in [0.0, -INTO, -avoid::BODY].into_iter().enumerate() {
                                    c.crowd[k] += (gap < deep) as usize;
                                }
                            }
                        }
                        if end {
                            c.at_ends += 1;
                            continue;
                        }
                        let mut measured = route.clone();
                        measured.measure(town, *d - 6.0, *d + 6.0);
                        let room = measured.room_at(*d);
                        let how = [true, gap < -INTO, gap < -avoid::BODY];
                        let count = |into: &mut [usize; 3]| how.iter().zip(into.iter_mut()).for_each(|(h, c)| *c += *h as usize);
                        match walking {
                            false => count(&mut c.still),
                            true => count(&mut c.passing),
                        }
                        if gap >= -INTO {
                            continue;
                        }
                        c.indoors += p.6.is_some() as usize;
                        if room[0].max(room[1]) >= 1.0 {
                            c.with_room += 1;
                        }
                        let what = match state {
                            avoid::State::At(s) => {
                                let st = &plans[m].stays[*s];
                                format!("{} at {}", st.spot.task.word(), town.place_word(st.place))
                            }
                            _ => "walking".to_string(),
                        };
                        let (from, to) = (&plans[n].stays[*i], &plans[n].stays[*i + 1]);
                        let key = format!("{what}, passed going {} -> {}", town.place_word(from.place), town.place_word(to.place));
                        c.example.entry(key.clone()).or_insert_with(|| format!("{n} by {m} at {hour:.4} h, {d:.1} of {length:.1}, gap {gap:.2}, room {room:?}"));
                        *c.worst.entry(key).or_default() += 1;
                    }
                }
            }
        }
        // Nobody jumps from one look to the next: walking on, setting off or arriving (but
        // for those coming into sight, who were shown where they were going).
        for (n, (now, before)) in figs.iter().zip(&last).enumerate() {
            let (Some((p, s)), Some((q, t))) = (now, before) else { continue };
            let lying = |s: &avoid::State| matches!(s, avoid::State::At(i) if plans[n].stays[*i].spot.task == Task::Lie);
            let going = |s: &avoid::State| matches!(s, avoid::State::Going(..));
            let shown_ahead = |s: &avoid::State, h: f32| matches!(s, avoid::State::At(_)) && matches!(plans[n].now(h), day::Now::Walking(..));
            if (going(s) || going(t)) && p.6.is_none() && q.6.is_none() && !lying(s) && !lying(t) && !shown_ahead(s, hour) && !shown_ahead(t, hour - dt) {
                let jump = flat(p.0).distance(flat(q.0)) - allowed;
                c.jumps += (jump > 0.0) as usize;
                if jump > c.jump {
                    c.jump = jump;
                    c.jump_who = Some((n, hour));
                    c.jump_at = format!("{n} at {hour:.4} h: {t:?} at {:?} to {s:?} at {:?}", q.0, p.0);
                }
            }
        }
        last = figs;
    }
    c
}

/// The furthest resident `n` of `town` goes from one look to the next, `k` looks `dt`
/// hours apart from `from`, as drawn from its middle, and when.
fn biggest_step(town: &Town, plans: &[day::Plan], n: usize, from: f32, dt: f32, k: usize) -> (f32, f32) {
    let eye = town.centre + Vec3::Y * 4.0;
    let (mut routes, mut scratch) = (avoid::Routes::new(), avoid::Scratch::default());
    let (mut last, mut most) = (None, (0.0f32, from));
    for j in 0..=k {
        let h = from + j as f32 * dt;
        let (figs, _) = placed_at(town, plans, h, eye, &mut routes, &mut scratch, true);
        let at = figs[n].as_ref().map(|f| flat(f.0.0));
        if let (Some(a), Some(b)) = (at, last)
            && a.distance(b) > most.0
        {
            most = (a.distance(b), h);
        }
        last = at;
    }
    most
}

#[test]
fn walkers_keep_out_of_each_others_way() {
    for town in towns() {
        let market = (WORKDAY..WORKDAY + 14).find(|&d| town.market_on(d) && day::weekday(d) != day::REST_DAY);
        for day in [Some(WORKDAY), market].into_iter().flatten() {
            let before = crowding(town, day, false);
            let after = crowding(town, day, true);
            let show = |c: &Crowding| {
                let mut w: Vec<(&String, &usize)> = c.worst.iter().collect();
                w.sort_by(|a, b| b.1.cmp(a.1));
                let top: Vec<String> = w.iter().take(8).map(|(k, v)| format!("    {v} {k}  [{}]", c.example[*k])).collect();
                format!("still {:?}, passing {:?}, at ends {}, indoors {}, with room {}, nearest {:.2}, met {:?}, crowd {:?}, jumps {}, jump {:.3} ({}), looks {}, {:.1} us a look ({:.0} at most, {} walking at most)\n{}", c.still, c.passing, c.at_ends, c.indoors, c.with_room, c.nearest, c.met, c.crowd, c.jumps, c.jump, c.jump_at, c.looks, c.took / c.looks.max(1) as f64, c.most, c.walkers, top.join("\n"))
            };
            println!("{} ({} people) on day {day}{}:\n  before {}\n  after  {}", town.name, town.residents.len(), if Some(day) == market { " (market)" } else { "" }, show(&before), show(&after));
            let on = format!("{} on day {day}", town.name);
            // Into someone, and deep into them, less than half as often as when everyone
            // went their own way regardless; and where there was room to step aside, less
            // than half as often again.
            let halved = |a: usize, b: usize| a * 2 <= b + 4;
            assert!(halved(after.still[1], before.still[1]) && halved(after.still[2], before.still[2]), "{on}: walkers still go into those at a stay, {:?} against {:?}", after.still, before.still);
            assert!(halved(after.passing[1], before.passing[1]) && halved(after.passing[2], before.passing[2]), "{on}: walkers still go into each other, {:?} against {:?}", after.passing, before.passing);
            assert!(halved(after.with_room, before.with_room), "{on}: walkers go into someone with room to step aside {} times, against {}", after.with_room, before.with_room);
            // Two meeting step aside each the same way, to their right or both to their left,
            // nearly always.
            assert!(after.met[1] * 10 <= after.met[0], "{on}: of two meeting, one steps one way and the other the other {} times in {}", after.met[1], after.met[0] + after.met[1]);
            // Those coming to a crowd before a well or a stall, or leaving one, keep to the way
            // up its middle and hardly ever go deep into anyone standing in it (#206).
            assert!(after.crowd[2] <= 2, "{on}: walkers go deep into those standing before a well or a stall {} times ({:?})", after.crowd[2], after.crowd);
            // Nobody darts across the ground much faster than a quick step aside; and the
            // worst of those, looked at more and more often, moves less and less between
            // looks, as nobody who leaps from one place to another would.
            assert!(after.jump < 0.5, "{on}: someone jumps {:.2} voxels ({})", after.jump, after.jump_at);
            if let Some((n, hour)) = after.jump_who {
                let plans = plans_of(town, day);
                let tenth = 0.1 / 3600.0;
                let (coarse, at) = biggest_step(town, &plans, n, hour - 35.0 * tenth, tenth, 40);
                let (fine, _) = biggest_step(town, &plans, n, at - tenth, tenth * 0.3, 8);
                assert!(coarse < 0.1, "{on}: resident {n} leaps {coarse:.3} voxels in a tenth of a second about {hour:.4} h");
                assert!(fine < coarse * 0.6 + 0.002, "{on}: resident {n} leaps {fine:.3} voxels in three hundredths of a second about {at:.5} h");
            }
        }
    }
}

#[test]
fn nobody_in_a_crowd_stands_in_the_way_of_the_others() {
    // Coming to their place in the first three rows before a well, a stall, the tree,
    // the shrine or the water, or leaving it, nobody comes nearer anyone else standing
    // there than to pass them with the gap kept between two people (#206).
    let clear = avoid::BODY * 2.0 + avoid::GAP;
    for town in towns() {
        let from = town::Spot::new(town.centre, Vec2::X, Task::Stand);
        let crowds: Vec<usize> = (0..town.places.len()).filter(|&p| town.places[p].what.crowd()).collect();
        for &p in &crowds {
            let spots: Vec<town::Spot> = (0..6).map(|k| day::around(town, p, k, Task::Stand)).collect();
            for (k, s) in spots.iter().enumerate() {
                let walk = town.walk((town.green_place, &from), (p, s));
                let legs: Vec<(Vec2, Vec2)> = walk.pts.windows(2).map(|w| (flat(w[0]), flat(w[1]))).collect();
                for (j, o) in spots.iter().enumerate().filter(|&(j, _)| j != k) {
                    let near = off(&legs, flat(o.at));
                    assert!(
                        near > clear - 0.01,
                        "{}: going to the {k}th place before {} ({p}), they come {near:.2} voxels from the {j}th",
                        town.name,
                        town.place_word(p)
                    );
                }
            }
        }
        // Each is come to up the middle, from behind or from before them.
        let before = |p: usize| (flat(town.places[p].at) - flat(town.lanes.nodes[town.places[p].anchor as usize])).dot(town.places[p].face) <= 0.0;
        assert!(crowds.iter().all(|&p| town.places[p].aisle > 0.0 || before(p)), "{}: a crowd is come to through its middle", town.name);
    }
}
