//! Everyone's day in a settlement (#204): in bed through the night, to the well in the
//! morning, at work through the day with the market at midday, the tavern or the green in
//! the evening, then home with the lights on and to bed. A day is made for the whole
//! settlement at once, so that two at the well or the tavern are never in the same
//! place, and it is the same every time for the same day: a pure function of the
//! settlement and which day it is. Each person keeps their own hours, and the day of the
//! week changes things: the fourth is market day and the seventh a day of rest.

use glam::{Vec2, Vec3};
use worldgen::noise::{hash2, unit};

use super::{
    lanes::flat,
    town::{Resident, Role, Spot, Town},
};
use crate::character::Task;
use crate::people::Trade;

/// How fast people walk, in voxels a game hour: 130 m, a steady walk of about 1.3 m a
/// second at the default length of day.
pub const PACE: f32 = worldgen::scale::m(130.0);
/// By when everyone is in bed.
pub const BED_BY: f32 = 23.0;
/// The days of the week, and which of them is market day and which the day of rest.
pub const WEEK: i64 = 7;
pub const MARKET_DAY: i64 = 3;
pub const REST_DAY: i64 = 6;
/// The least anyone stays anywhere, in hours.
const LEAST: f32 = 0.05;

/// Somewhere someone is for a while: from when they get there to when they set off.
#[derive(Clone, Copy, Debug)]
pub struct Stay {
    pub place: usize,
    pub spot: Spot,
    pub arrive: f32,
    pub leave: f32,
}

/// One person's day, its stays in order: the first and last in bed.
#[derive(Clone, Debug, Default)]
pub struct Plan {
    pub stays: Vec<Stay>,
}

/// Where someone is in their day.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Now {
    /// At their `n`th stay.
    At(usize),
    /// On the way from their `n`th stay to the next, this far through the walk (0 to 1).
    Walking(usize, f32),
}

impl Plan {
    pub fn now(&self, hour: f32) -> Now {
        for (i, s) in self.stays.iter().enumerate() {
            if hour < s.leave {
                if i == 0 || hour >= s.arrive {
                    return Now::At(i);
                }
                let p = &self.stays[i - 1];
                return Now::Walking(i - 1, ((hour - p.leave) / (s.arrive - p.leave).max(1e-4)).clamp(0.0, 1.0));
            }
        }
        Now::At(self.stays.len().saturating_sub(1))
    }

    /// The stay someone is at or walking to at `hour`.
    #[cfg(test)]
    pub fn bound(&self, hour: f32) -> &Stay {
        match self.now(hour) {
            Now::At(i) => &self.stays[i],
            Now::Walking(i, _) => &self.stays[i + 1],
        }
    }
}

/// Somewhere someone means to be: from `start` (or as soon as they can, if that has
/// passed), for at least `stay` hours.
#[derive(Clone, Copy, Debug)]
struct Want {
    place: usize,
    spot: Spot,
    start: f32,
    stay: f32,
}

/// What is taken already, as the settlement's day is made person by person.
#[derive(Default)]
struct Taken {
    tavern: usize,
    green: usize,
    wells: Vec<usize>,
    stalls: Vec<usize>,
    tree: usize,
    shrine: usize,
    water: usize,
}

/// The day numbered `day` (from the world's first) for everyone of `town`, in the order
/// of [`Town::residents`].
pub fn plans(town: &Town, day: i64) -> Vec<Plan> {
    let mut taken = Taken { wells: vec![0; town.wells.len()], stalls: vec![0; town.stalls.len()], ..Default::default() };
    town.residents.iter().map(|r| plan(town, r, day, &mut taken)).collect()
}

/// The day of the week `day` falls on, 0 to 6.
pub fn weekday(day: i64) -> i64 {
    day.rem_euclid(WEEK)
}

fn plan(town: &Town, r: &Resident, day: i64, taken: &mut Taken) -> Plan {
    let seed = r.person.seed ^ (day as u32).wrapping_mul(0x9e37_79b9) ^ ((day >> 32) as u32);
    let d = |k: i32| unit(hash2(seed, k, 0x3d17));
    let dow = weekday(day);
    let rest = dow == REST_DAY;
    let market = dow == MARKET_DAY;
    let home = town.plots[r.home].place;
    let inside = Want { place: home, spot: town.inside_spot(r.home, r.member, Task::Stand), start: f32::NEG_INFINITY, stay: LEAST };
    let at = |start: f32, w: Want| Want { start, ..w };
    let for_ = |stay: f32, w: Want| Want { stay, ..w };
    let mut w: Vec<Want> = vec![Want { place: home, spot: r.bed, start: 0.0, stay: 0.0 }];

    // Each of these is somewhere to go, and the spot there, as it is taken.
    // Water is fetched from the well nearest home.
    let well = |taken: &mut Taken, k: i32| -> Option<Want> {
        let i = (0..town.wells.len()).min_by(|&a, &b| town.dist[home][town.wells[a]].total_cmp(&town.dist[home][town.wells[b]]))?;
        let slot = taken.wells[i];
        taken.wells[i] += 1;
        Some(Want { place: town.wells[i], spot: around(town, town.wells[i], slot, Task::Fetch), start: f32::NEG_INFINITY, stay: 0.2 + 0.15 * d(k + 1) })
    };
    let stall = |taken: &mut Taken, k: i32| -> Option<Want> {
        if town.stalls.is_empty() {
            return None;
        }
        let i = (d(k) * town.stalls.len() as f32) as usize % town.stalls.len();
        let slot = taken.stalls[i];
        taken.stalls[i] += 1;
        Some(Want { place: town.stalls[i], spot: around(town, town.stalls[i], slot, Task::Talk), start: f32::NEG_INFINITY, stay: 0.35 + 0.3 * d(k + 1) })
    };
    let tavern = |taken: &mut Taken| -> Option<Want> {
        let t = town.tavern?;
        // The keeper stands just inside the door; the guests take the seats.
        let k = taken.tavern;
        taken.tavern += 1;
        let seats = town.plots[t].seats.len();
        let spot = town.inside_spot(t, if k < seats { k } else { k + 1 }, Task::Talk);
        Some(Want { place: town.plots[t].place, spot, start: f32::NEG_INFINITY, stay: 0.5 })
    };
    let green = |taken: &mut Taken, task: Task| -> Want {
        let k = taken.green;
        taken.green += 1;
        Want { place: town.green_place, spot: talking(town, k, task), start: f32::NEG_INFINITY, stay: 0.4 }
    };
    let haunt = |taken: &mut Taken, which: Option<usize>, task: Task| -> Option<Want> {
        let p = which?;
        let slot = if Some(p) == town.tree {
            &mut taken.tree
        } else if Some(p) == town.shrine {
            &mut taken.shrine
        } else {
            &mut taken.water
        };
        let k = *slot;
        *slot += 1;
        let mut spot = around(town, p, k, task);
        spot.stool = task == Task::Sit;
        Some(Want { place: p, spot, start: f32::NEG_INFINITY, stay: 0.5 })
    };
    let door_sit = |k: u32| Want { place: home, spot: town.by_door(r.home, k, Task::Sit), start: f32::NEG_INFINITY, stay: 0.5 };
    let job = |n: usize| r.jobs.get(n % r.jobs.len().max(1)).map(|j| Want { place: j.place, spot: j.spot, start: f32::NEG_INFINITY, stay: 0.5 });

    // The evening out, for the grown: the tavern, the green, or outside the door.
    let evening = |w: &mut Vec<Want>, taken: &mut Taken, keen: f32| {
        let u = d(60);
        let out = 19.0 + 0.7 * d(61);
        if u < keen {
            if let Some(t) = tavern(taken) {
                w.push(at(out, t));
                w.push(at(21.6 + 0.6 * d(62), inside));
                return;
            }
        }
        if u < keen + 0.3 {
            w.push(at(out, green(taken, Task::Talk)));
            w.push(at(20.4 + 0.6 * d(63), inside));
        } else if u < keen + 0.6 {
            w.push(at(out, door_sit(r.member as u32)));
            w.push(at(20.2 + 0.6 * d(63), inside));
        }
    };
    let bed = |start: f32| Want { place: home, spot: r.bed, start, stay: 0.0 };

    match r.role {
        Role::Worker | Role::Seller if !rest => {
            let wake = 5.6 + d(1);
            w.push(for_(0.3, at(wake, inside)));
            if d(2) < if r.role == Role::Seller { 0.3 } else { 0.4 } {
                w.extend(well(taken, 3).map(|x| at(wake + 0.6 + 0.3 * d(20), x)));
                w.push(for_(0.3, inside));
            }
            let morning = 7.8 + 0.5 * d(4);
            let noon = 11.9 + 0.3 * d(5);
            let afternoon = 13.1 + 0.3 * d(6);
            let evening_in = 17.0 + 0.6 * d(7);
            match (r.role, r.stall) {
                (Role::Seller, Some(s)) => {
                    w.push(at(8.8 + 0.4 * d(4), Want { place: town.stalls[s], spot: town.behind[s], start: 0.0, stay: 0.5 }));
                    w.push(for_(0.6, at(15.0 + 0.4 * d(5), inside)));
                }
                _ => {
                    w.extend(job(0).map(|x| at(morning, x)));
                    // To the market at midday on market day, and some on other days.
                    match stall(taken, 8).filter(|_| market && d(9) < 0.75 || d(9) < 0.22) {
                        Some(s) => w.push(at(noon, s)),
                        None => w.push(at(noon, inside)),
                    }
                    w.extend(job(1).map(|x| at(afternoon, x)));
                }
            }
            w.push(for_(0.8, at(evening_in, inside)));
            let keen = if dow >= 5 { 0.55 } else { 0.32 };
            evening(&mut w, taken, keen);
            w.push(bed(22.2 + 0.55 * d(10)));
        }
        Role::Worker | Role::Seller => {
            // The day of rest: up later, out in the morning, and to the tavern at night.
            let wake = 6.4 + d(1);
            w.push(for_(0.3, at(wake, inside)));
            if d(2) < 0.3 {
                w.extend(well(taken, 3).map(|x| at(wake + 0.6 + 0.3 * d(20), x)));
                w.push(for_(0.5, inside));
            }
            let u = d(11);
            let out = 9.8 + 0.6 * d(12);
            let morning = if u < 0.3 {
                Some(green(taken, Task::Talk))
            } else if u < 0.5 {
                haunt(taken, town.tree, Task::SitLow)
            } else if u < 0.7 {
                haunt(taken, town.shrine, Task::Stand)
            } else if u < 0.85 {
                haunt(taken, town.water, if r.person.trade == Trade::Fisher { Task::Fish } else { Task::Stand })
            } else {
                None
            };
            w.push(at(out, for_(1.0, morning.unwrap_or(door_sit(r.member as u32)))));
            w.push(for_(1.0, at(12.2 + 0.4 * d(13), inside)));
            if d(14) < 0.5 {
                w.push(at(14.0 + 0.8 * d(15), for_(1.5, door_sit(r.member as u32))));
            } else {
                w.push(at(14.0 + 0.8 * d(15), for_(1.2, green(taken, Task::Talk))));
            }
            w.push(for_(0.8, at(17.2 + 0.5 * d(7), inside)));
            evening(&mut w, taken, 0.6);
            w.push(bed(22.3 + 0.45 * d(10)));
        }
        Role::Innkeeper => {
            let wake = 6.0 + 0.6 * d(1);
            w.push(for_(0.4, at(wake + 0.2, inside)));
            let door = job(1).unwrap_or(inside);
            w.push(at(7.4 + 0.3 * d(2), for_(0.7, door)));
            let bar = job(0).unwrap_or(inside);
            if market && d(3) < 0.6 {
                w.push(at(8.6, for_(2.5, bar)));
                w.extend(stall(taken, 8).map(|s| at(11.4, s)));
            }
            w.push(at(12.0, for_(10.0, bar)));
            w.push(bed(22.5 + 0.3 * d(10)));
        }
        Role::Homemaker => {
            let wake = 5.7 + 0.8 * d(1);
            w.push(for_(0.3, at(wake, inside)));
            if d(2) < 0.7 {
                w.extend(well(taken, 3).map(|x| at(wake + 0.5 + 0.4 * d(20), x)));
                w.push(for_(0.4, inside));
            }
            w.extend(job(0).filter(|_| !rest).map(|x| at(8.0 + 0.5 * d(4), for_(0.6, x))));
            w.extend(job(1).filter(|_| !rest && r.jobs.len() > 1).map(|x| at(9.2 + 0.4 * d(5), for_(1.2, x))));
            match stall(taken, 8).filter(|_| (market && d(9) < 0.85) || d(9) < 0.35) {
                Some(s) => w.push(at(11.2 + 0.6 * d(16), s)),
                None if rest && d(9) < 0.6 => w.extend(haunt(taken, town.shrine, Task::Stand).map(|x| at(10.2, x))),
                None => {}
            }
            w.push(for_(1.0, at(12.4 + 0.3 * d(13), inside)));
            let u = d(14);
            if u < 0.45 {
                w.push(at(15.0 + 0.5 * d(15), for_(1.2, door_sit(r.member as u32))));
            } else if u < 0.65 {
                w.push(at(15.0 + 0.5 * d(15), for_(1.0, green(taken, Task::Talk))));
            }
            w.push(for_(1.0, at(17.0 + 0.4 * d(7), inside)));
            evening(&mut w, taken, if dow >= 5 { 0.35 } else { 0.18 });
            w.push(bed(21.8 + 0.8 * d(10)));
        }
        Role::Child => {
            let wake = 6.6 + 0.7 * d(1);
            w.push(for_(0.6, at(wake + 0.2, inside)));
            play(town, &mut w, 8.3 + 0.4 * d(4), 11.8, seed, 20);
            w.push(for_(1.0, at(12.0 + 0.2 * d(5), inside)));
            if d(6) < 0.7 {
                play(town, &mut w, 13.3 + 0.3 * d(7), 17.2, seed, 40);
            } else {
                w.push(at(13.5, for_(2.5, Want { spot: town.by_door(r.home, r.member as u32, Task::SitLow), ..door_sit(0) })));
            }
            w.push(for_(0.8, at(17.5, inside)));
            if d(8) < 0.4 {
                play(town, &mut w, 18.4, 19.2, seed, 60);
                w.push(for_(0.2, at(19.4, inside)));
            }
            w.push(bed(19.6 + 0.8 * d(10)));
        }
        Role::Elder => {
            let wake = 5.4 + d(1);
            w.push(for_(0.8, at(wake + 0.2, inside)));
            w.push(at(8.0 + 0.6 * d(4), for_(2.5, door_sit(r.member as u32))));
            if market && d(9) < 0.5 {
                w.extend(stall(taken, 8).map(|s| at(11.4, s)));
            } else if rest && d(9) < 0.7 {
                w.extend(haunt(taken, town.shrine, Task::Stand).map(|x| at(10.8, x)));
            }
            w.push(for_(1.2, at(12.2 + 0.4 * d(13), inside)));
            let u = d(14);
            let out = 14.0 + 0.5 * d(15);
            if u < 0.3 {
                w.push(at(out, for_(2.0, green(taken, Task::Sit))));
            } else if u < 0.55 {
                w.extend(haunt(taken, town.tree.or(town.shrine), Task::Sit).map(|x| at(out, for_(2.0, x))));
            } else {
                w.push(at(out, for_(2.5, door_sit(r.member as u32))));
            }
            w.push(for_(0.8, at(17.2 + 0.4 * d(7), inside)));
            if d(16) < 0.5 {
                w.push(at(18.4, for_(1.0, door_sit(r.member as u32))));
            }
            w.push(for_(0.3, at(19.8 + 0.4 * d(17), inside)));
            w.push(bed(20.5 + d(10)));
        }
    }
    for x in &mut w {
        if x.spot.task == Task::Sit && x.spot.inside.is_none() && x.place == town.green_place {
            x.spot.stool = true;
        }
    }
    settle(town, w)
}

/// Children at play on the green from `from` to `until`: running from spot to spot,
/// standing about, sitting in the grass.
fn play(town: &Town, w: &mut Vec<Want>, from: f32, until: f32, seed: u32, k: i32) {
    let d = |n: i32| unit(hash2(seed, k + n, 0x9a1));
    let mut t = from;
    let mut n = 0;
    let mut last: Option<Vec3> = None;
    while t < until && n < 16 {
        let a = d(n * 3) * std::f32::consts::TAU;
        let r = (town.green * 0.5 - 2.0).max(1.0) * (0.3 + 0.65 * d(n * 3 + 1).sqrt());
        let task = [Task::Stand, Task::Talk, Task::SitLow, Task::Stand][(d(n * 3 + 2) * 4.0) as usize % 4];
        let at = town.centre + Vec3::new(a.cos() * r, 0.0, a.sin() * r);
        let toward = (flat(town.centre) - flat(at)).normalize_or(Vec2::X).perp() * if n % 2 == 0 { 1.0 } else { -1.0 };
        let stay = 0.25 + 0.3 * d(n * 3 + 2);
        w.push(Want { place: town.green_place, spot: Spot::new(at, toward, task), start: if n == 0 { t } else { f32::NEG_INFINITY }, stay });
        // Running from one spot to the next takes its time too.
        t += stay + last.map_or(0.0, |l| l.distance(at) / PACE);
        last = Some(at);
        n += 1;
    }
}

/// The `k`th spot about place `p`, for someone at `task` there: before it, in a row
/// across the way it faces and then a row behind.
fn around(town: &Town, p: usize, k: usize, task: Task) -> Spot {
    let place = &town.places[p];
    let face = place.face;
    let across = face.perp();
    let (row, col) = ((k / 3) as f32, [0.0, -1.0, 1.0][k % 3]);
    let off = across * (col * 1.8 + row * 0.6) - face * (row * 1.9);
    let at = place.at + Vec3::new(off.x, 0.0, off.y);
    Spot::new(at, (face + across * col * -0.25).normalize_or(face), task)
}

/// The `k`th spot on the green for people talking there, two by two, facing each other:
/// the pairs about the middle, a few paces from it.
fn talking(town: &Town, k: usize, task: Task) -> Spot {
    let pair = (k / 2) as f32;
    let a = pair * 2.399_963 + 0.6;
    let reach = (town.green * 0.5 - 2.5).max(2.0);
    let r = reach * (0.45 + 0.5 * ((pair * 0.618_034).fract()));
    let mid = flat(town.centre) + Vec2::new(a.cos(), a.sin()) * r;
    let along = Vec2::new(-a.sin(), a.cos());
    let side = if k % 2 == 0 { 1.0 } else { -1.0 };
    let p = mid + along * side * 1.4;
    Spot::new(Vec3::new(p.x, town.centre.y, p.y), -along * side, task)
}

/// Works out when each want is met: each walk set off from the one before in time to
/// arrive at its start, but no sooner than its least stay is up. Where the evening runs
/// late, its outings are given up until everyone is in bed by [`BED_BY`].
fn settle(town: &Town, mut w: Vec<Want>) -> Plan {
    loop {
        let plan = resolve(town, &w);
        let late = plan.stays.last().is_some_and(|s| s.arrive > BED_BY);
        if !late || w.len() <= 2 {
            return plan;
        }
        // The last outing before bed goes; failing any, the last stay of all.
        let n = w.len();
        let drop = (1..n - 1).rev().find(|&i| w[i].spot.inside.is_none()).unwrap_or(n - 2);
        w.remove(drop);
    }
}

fn resolve(town: &Town, w: &[Want]) -> Plan {
    let mut stays: Vec<Stay> = Vec::with_capacity(w.len());
    for (i, x) in w.iter().enumerate() {
        if i == 0 {
            stays.push(Stay { place: x.place, spot: x.spot, arrive: 0.0, leave: 24.0 });
            continue;
        }
        let prev = stays[i - 1];
        let walk = town.walk_length((prev.place, &prev.spot), (x.place, &x.spot)) / PACE;
        let earliest = if i == 1 { 0.0 } else { prev.arrive + w[i - 1].stay.max(LEAST) };
        let leave = (x.start - walk).max(earliest);
        stays[i - 1].leave = leave;
        stays.push(Stay { place: x.place, spot: x.spot, arrive: leave + walk, leave: 24.0 });
    }
    Plan { stays }
}
