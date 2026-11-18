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
        // On another day nobody goes to them, nor before the market opens or after it shuts.
        let other = plans_of(town, ordinary);
        assert_eq!((0..48).map(|k| at_stalls(&other, k as f32 * 0.5)).max(), Some(0), "{}", town.name);
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
            // And in the day most are up and about.
            let up = plans.iter().filter(|p| p.bound(10.0).spot.task != Task::Lie).count();
            assert!(up * 10 >= plans.len() * 9, "{}: {up} of {} up at ten", town.name, plans.len());
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
                        for step in town.last_steps(s.place, &s.spot) {
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
        // In year 0 everyone is known by their number, as before there were years.
        assert!(today.residents.iter().all(|r| r.id == r.n));
        for (year, season) in [(1, 2), (2, 0), (3, 1), (5, 2), (9, 1), (14, 3)] {
            let lives = Lives::new(net, SEED, (0, 0), When::new(year, season), &Boons::default());
            let then = Town::build_in(c, SEED, (0, 0), town.k, &lives).unwrap();
            for r in &then.residents {
                assert!(lives.standing(first + r.home), "{} lives in a home not standing in year {year}", r.person.name);
            }
            let homes: Vec<usize> = (0..then.plots.len()).filter(|&i| net.plots[first + i].sleepers > 0).collect();
            empty_seen += homes.iter().filter(|&&i| !lives.standing(first + i)).count();
            // The same people where nothing has changed, known by the same names.
            for r in &then.residents {
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
