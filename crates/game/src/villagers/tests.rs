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

/// A day that is no day in particular: neither market day nor the day of rest.
const WORKDAY: i64 = 7 * 40 + 1;

#[test]
fn the_same_people_are_in_the_same_places_every_time() {
    let c = continent();
    for town in towns() {
        assert!(!town.residents.is_empty(), "{} has nobody", town.name);
        let again = Town::build(c, SEED, (0, 0), town.k).unwrap();
        let key = |t: &Town| t.residents.iter().map(|r| (r.person.name.clone(), r.person.trade, r.home, r.member, r.role)).collect::<Vec<_>>();
        assert_eq!(key(town), key(&again));
        for day in [WORKDAY, WORKDAY + 2, WORKDAY + 5] {
            let (a, b) = (day::plans(town, day), day::plans(&again, day));
            for hour in [3.0, 7.0, 8.5, 12.3, 19.7, 22.9] {
                for (p, q) in a.iter().zip(&b) {
                    assert_eq!(p.now(hour), q.now(hour));
                    assert_eq!(p.bound(hour).spot.at, q.bound(hour).spot.at);
                }
            }
        }
        // And from one day to the next people keep their own hours.
        let (a, b) = (day::plans(town, WORKDAY), day::plans(town, WORKDAY + 1));
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
            let plans = day::plans(town, day);
            for (r, plan) in town.residents.iter().zip(&plans) {
                for hour in [0.0, 2.5, 4.5, 23.1, 23.9] {
                    match plan.now(hour) {
                        Now::At(i) => {
                            let s = &plan.stays[i];
                            assert_eq!(s.spot.task, Task::Lie, "{} ({:?}) at {hour} on day {day}: {:?}", r.person.name, r.role, s);
                            assert_eq!(s.spot.inside, Some(r.home));
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
        let plans = day::plans(town, WORKDAY);
        let (noon, evening) = (at_tavern(town, &plans, 12.0), at_tavern(town, &plans, 20.5));
        assert!(evening > noon && evening >= 2, "{}: {noon} at noon, {evening} in the evening", town.name);
        // And fuller still at the end of the week.
        let weekend = day::plans(town, WORKDAY - day::weekday(WORKDAY) + 5);
        assert!(at_tavern(town, &weekend, 20.5) >= evening, "{}", town.name);
    }
    assert!(seen, "none of the towns looked at has a tavern");
}

#[test]
fn the_well_is_busy_in_the_morning_and_work_in_the_day() {
    for town in towns().iter().filter(|t| !t.wells.is_empty()) {
        let plans = day::plans(town, WORKDAY);
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
        let plans = day::plans(town, WORKDAY);
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
                    assert!(d < 1.5 || near_end || on_green, "{}: a walk strays {d:.1} voxels from the lanes at {p}", town.name);
                    // And never through a wall: into a building only by its door.
                    for x in &town.plots {
                        let (lo, hi) = (x.lo.as_vec2() + 1.3, x.hi.as_vec2() - 0.3);
                        let within = p2.cmpgt(lo).all() && p2.cmplt(hi).all();
                        assert!(
                            !within || p2.distance(flat(x.doorway)) < 3.0,
                            "{}: a walk from {} ({:?}) to {} ({:?}) goes through the walls of a {} ({}..{}) at {p}",
                            town.name,
                            town.place_word(a.place),
                            a.spot,
                            town.place_word(b.place),
                            b.spot,
                            x.purpose.word(),
                            x.lo,
                            x.hi
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
        v.add(Town::build(continent(), SEED, (0, 0), k).unwrap(), WORKDAY);
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
    v.add(Town::build(continent(), SEED, (0, 0), town.k).unwrap(), WORKDAY);
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
            let named = real.iter().any(|n| line.contains(n.as_str())) || line.contains(&town.name) || line.contains("village tree");
            assert!(named, "{} of {} says '{line}', naming nowhere", r.person.name, town.name);
            assert!(["spring", "summer", "autumn", "winter"].iter().any(|s| line.contains(s)) || line.contains(&town.name) || town.near.as_ref().is_some_and(|n| line.contains(n.as_str())) || line.contains("tree"), "{line}");
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
    v.add(Town::build(continent(), SEED, (0, 0), town.k).unwrap(), WORKDAY);
    // Stand beside someone at work at ten, and greet them.
    let plans = day::plans(town, WORKDAY);
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
