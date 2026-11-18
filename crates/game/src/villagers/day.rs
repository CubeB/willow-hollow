//! Everyone's day in a settlement (#204): in bed through the night, to the well in the
//! morning, at work through the day with the market at midday, the tavern or the green in
//! the evening, then home with the lights on and to bed. A day is made for the whole
//! settlement at once, so that two at the well or the tavern are never in the same
//! place, and it is the same every time for the same day: a pure function of the
//! settlement and which day it is. Each person keeps their own hours, and the day of the
//! week changes things: the seventh is a day of rest. Market day comes round on the
//! settlement's own beat (`worldgen::settlements::calendar`, #207): its stalls are kept
//! from morning to afternoon by their keepers, and its people come and go to them all
//! morning and at midday; on other days the stalls stand bare and nobody goes to them.

use glam::{Vec2, Vec3};
use worldgen::noise::{hash2, unit};
use worldgen::settlements::calendar;

use super::{
    lanes::flat,
    town::{Resident, Role, Spot, Town, Where},
};
use crate::character::Task;
use crate::people::Trade;

/// How fast people walk, in voxels a game hour: 130 m, a steady walk of about 1.3 m a
/// second at the default length of day.
pub const PACE: f32 = worldgen::scale::m(130.0);
/// By when everyone is in bed.
pub const BED_BY: f32 = 23.0;
/// The days of the week, and which of them is the day of rest. Market days are the
/// settlement's own ([`Town::market_on`]).
pub const WEEK: i64 = 7;
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

/// The day numbered `day` (from the world's first, in a year of `year_days` days) for
/// everyone of `town`, in the order of [`Town::residents`].
pub fn plans(town: &Town, day: i64, year_days: f32) -> Vec<Plan> {
    let mut taken = Taken { wells: vec![0; town.wells.len()], stalls: vec![0; town.stalls.len()], ..Default::default() };
    let date = (day as f64 / year_days.max(1.0) as f64).rem_euclid(1.0) as f32;
    town.residents.iter().map(|r| plan(town, r, day, date, &mut taken)).collect()
}

/// When the sun sets on `date` (0 to 1 from the spring equinox), as `env.rs` has it
/// ride the sky: at six at the equinoxes, later in summer and sooner in winter.
pub fn sunset(date: f32) -> f32 {
    18.0 + (0.3 * crate::season::declination(date)).asin() * 12.0 / std::f32::consts::PI
}

/// The longest walk out to work in the land and back again the same day, each way, in
/// hours: further, and the work is let be for the work about the house.
pub const FURTHEST_OUT: f32 = 4.2;

/// What is done in a field of `crop` at `date` (0 to 1 from the spring equinox), the
/// `second` thing of the day or the first, as the fields are seen to grow
/// (`farmland::crops`): sowing in spring, hoeing the rows through early summer, reaping
/// as the grain turns gold and carrying the sheaves off the stubble after, sowing again
/// in the autumn, and the ground worked over through the winter; roots lifted in the
/// autumn; hay mown as it seeds; trees and vines tended all year.
pub fn field_task(crop: worldgen::settlements::farmland::Crop, date: f32, second: bool) -> Task {
    use worldgen::settlements::farmland::Crop;
    let date = date.rem_euclid(1.0);
    let pick = |a: Task, b: Task| if second { b } else { a };
    match crop {
        Crop::Wheat | Crop::Barley | Crop::Rye | Crop::Flax => match date {
            d if d < 0.25 => pick(Task::Sow, Task::Hoe),
            d if (0.36..0.445).contains(&d) => pick(Task::Reap, Task::Sheaves),
            d if (0.445..0.5).contains(&d) => Task::Sheaves,
            d if (0.6..0.75).contains(&d) => pick(Task::Hoe, Task::Sow),
            _ => Task::Hoe,
        },
        Crop::Roots => match date {
            d if d < 0.25 => pick(Task::Sow, Task::Hoe),
            d if (0.5..0.62).contains(&d) => pick(Task::Tend, Task::Carry),
            _ => pick(Task::Hoe, Task::Tend),
        },
        Crop::Hay if (0.27..0.33).contains(&date) => pick(Task::Reap, Task::Sheaves),
        Crop::Orchard | Crop::Olives | Crop::Vines | Crop::Hay => Task::Tend,
        _ => Task::Hoe,
    }
}

/// The day of the week `day` falls on, 0 to 6.
pub fn weekday(day: i64) -> i64 {
    day.rem_euclid(WEEK)
}

fn plan(town: &Town, r: &Resident, day: i64, date: f32, taken: &mut Taken) -> Plan {
    let seed = r.person.seed ^ (day as u32).wrapping_mul(0x9e37_79b9) ^ ((day >> 32) as u32);
    let d = |k: i32| unit(hash2(seed, k, 0x3d17));
    if r.role == Role::Visitor {
        return visit(town, r, town.market_on(day), &d, taken);
    }
    let dow = weekday(day);
    let rest = dow == REST_DAY;
    let market = town.market_on(day);
    // Whoever keeps a stall on market day works it, rest day or not; a seller with no
    // market to keep keeps the house.
    let keeps = if market { town.keeps(r.n) } else { None };
    let role = match (r.role, keeps) {
        (Role::Child | Role::Elder, _) => r.role,
        (_, Some(_)) => Role::Seller,
        (Role::Seller, None) => Role::Homemaker,
        (role, None) => role,
    };
    let home = town.plots[r.home].place;
    let inside = Want { place: home, spot: town.inside_spot(r.home, r.member, Task::Stand), start: f32::NEG_INFINITY, stay: LEAST };
    let at = |start: f32, w: Want| Want { start, ..w };
    let for_ = |stay: f32, w: Want| Want { stay, ..w };
    // Out at a logging camp too far to come home from each day (#64), they sleep out
    // there from the first working day to the last.
    let camp_week = keeps.is_none() && r.role == Role::Worker && town.stays_out(r);
    let camp_bed = r.jobs.get(3).filter(|_| camp_week).map(|j| Want { place: j.place, spot: j.spot, start: 0.0, stay: 0.0 });
    let slept_out = camp_bed.filter(|_| (1..REST_DAY).contains(&dow));
    let mut w: Vec<Want> = vec![slept_out.unwrap_or(Want { place: home, spot: r.bed, start: 0.0, stay: 0.0 })];

    // Each of these is somewhere to go, and the spot there, as it is taken.
    // Water is fetched from the well nearest home.
    let well = |taken: &mut Taken, k: i32| -> Option<Want> {
        let i = (0..town.wells.len()).min_by(|&a, &b| town.dist[home][town.wells[a]].total_cmp(&town.dist[home][town.wells[b]]))?;
        let slot = taken.wells[i];
        taken.wells[i] += 1;
        Some(Want { place: town.wells[i], spot: around(town, town.wells[i], slot, Task::Fetch), start: f32::NEG_INFINITY, stay: 0.2 + 0.15 * d(k + 1) })
    };
    // A stall to buy at, only on market day, when they are kept. Those at one at once
    // stand in a few rows before it; those who come later take the places of those gone.
    let stall = |taken: &mut Taken, k: i32| -> Option<Want> {
        if town.stalls.is_empty() || !market {
            return None;
        }
        let i = (d(k) * town.stalls.len() as f32) as usize % town.stalls.len();
        let slot = taken.stalls[i] % 6;
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
    // Someone whose work is out in the land and who stays in works about the house.
    let about_house = Want { place: home, spot: town.by_door(r.home, r.n as u32 + 1, Task::Carry), start: f32::NEG_INFINITY, stay: 0.5 };
    let job = |n: usize| {
        if town.works_out(r) {
            return Some(about_house);
        }
        r.jobs.get(n % r.jobs.len().max(1)).map(|j| Want { place: j.place, spot: j.spot, start: f32::NEG_INFINITY, stay: 0.5 })
    };

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

    // Out in the land (#64, #206): the herder drives the fold's flock every day; those
    // whose work is out in a field, at the camp or the quarry go on a working day, if it
    // is near enough to be there and back by dusk. A field's work is the season's.
    let drove = town.drives(r.n).filter(|_| keeps.is_none());
    let land_job = |n: usize| -> Option<Want> {
        let j = r.jobs.get(n)?;
        let mut spot = j.spot;
        if let Where::Field(f) = town.places[j.place].what
            && n < 2
        {
            spot.task = field_task(town.fields[f].crop, date, n == 1);
        }
        Some(Want { place: j.place, spot, start: f32::NEG_INFINITY, stay: 0.5 })
    };
    let far_out = r.jobs.first().map_or(f32::MAX, |j| town.walk_length((home, &inside.spot), (j.place, &j.spot)) / PACE);
    let goes_out = keeps.is_none() && !rest && drove.is_none() && town.works_out(r) && far_out <= FURTHEST_OUT;
    // Home before the sun is down.
    let dusk = (sunset(date) - 0.4).min(18.2);

    match role {
        Role::Worker if drove.is_some() => {
            let dv = &town.droves[drove.unwrap_or(0)];
            let hours = dv.herd.hours(day, sunset(date));
            let local = |p: glam::DVec2| Vec2::new((p.x - town.origin.x) as f32, (p.y - town.origin.z) as f32);
            let (gate, graze) = (local(dv.herd.gate), local(dv.herd.graze));
            let out = dv.herd.out.as_vec2();
            let (side, dir, length) = (out.perp(), (graze - gate).normalize_or(out), graze.distance(gate));
            let y = town.places[dv.place].at.y;
            let here = |p: Vec2, face: Vec2, task: Task, start: f32, stay: f32| Want { place: dv.place, spot: Spot::new(Vec3::new(p.x, y, p.y), face, task), start, stay };
            let by_gate = town.places[dv.place].at;
            // Behind the flock, this far off its middle.
            let trail = worldgen::scale::m(2.5);
            // Up in time to be at the fold as the gate is opened.
            let wake = (5.4 + 0.5 * d(1)).min(hours.open - 0.4 - town.walk_out(r));
            w.push(for_(0.3, at(wake, inside)));
            // To the fold, and the gate opened.
            w.push(here(flat(by_gate), -side, Task::Stand, hours.open - 0.1, 0.1));
            // Out behind the flock to the grass, stopping as it stops.
            let from = hours.open + 0.1;
            for k in 1..=3 {
                let s = k as f32 / 3.0;
                let p = gate + dir * (length * s - trail).max(2.0) + side * 1.4;
                w.push(here(p, dir, Task::Stand, from + (hours.out - from) * s, LEAST));
            }
            // Watching over it through the day; milking one out on the grass, and down
            // on the ground at midday.
            let watch = graze - dir * (trail + 2.0) + side * 2.0;
            if let Some((a, b)) = hours.milk {
                let (_, _, stool, face) = dv.herd.milking();
                let mut milk = here(local(stool), face.as_vec2(), Task::Milk, a, b - a);
                milk.spot.stool = true;
                w.push(milk);
            }
            w.push(here(watch, dir, Task::Stand, f32::NEG_INFINITY, 1.0));
            w.push(here(watch + side * 0.8, dir, Task::SitLow, 12.0 + 0.4 * d(5), 0.8));
            w.push(here(watch, dir, Task::Stand, 13.2 + 0.4 * d(6), 0.5));
            // Round beyond it, and back behind it to the fold, the gate shut on it.
            w.push(here(graze + dir * (trail + 1.0), -dir, Task::Stand, hours.gather - 0.02, LEAST));
            let to = hours.shut - 0.1;
            for k in 1..=2 {
                let s = k as f32 / 3.0;
                let p = gate + dir * (length * (1.0 - s) + trail) - side * 1.4;
                w.push(here(p, -dir, Task::Stand, hours.gather + (to - hours.gather) * s, LEAST));
            }
            w.push(here(flat(by_gate), -side, Task::Stand, to, 0.25));
            w.push(for_(0.8, at(hours.shut + 0.3, inside)));
            evening(&mut w, taken, if dow >= 5 { 0.45 } else { 0.25 });
            w.push(bed(22.0 + 0.6 * d(10)));
        }
        Role::Worker if camp_week && !rest => {
            // Up at the camp, or at home on the first working day and out to it; at work
            // there all day with a bite at midday, and the evening by the lean-to, but on
            // the last working day, when it is home before dusk for the day of rest.
            let up = land_job(2).map_or(inside, |x| Want { spot: Spot { task: Task::Stand, ..x.spot }, ..x });
            let wake = 5.0 + 0.5 * d(1);
            w.push(for_(0.3, at(wake, if slept_out.is_some() { up } else { inside })));
            w.extend(land_job(0).map(|x| at(6.8 + 0.4 * d(4), x)));
            // On the last working day, the afternoon's work only if there is time for it
            // and the long walk home.
            let homeward = dow == REST_DAY - 1;
            if !homeward || 13.4 + town.walk_out(r) <= dusk {
                w.extend(land_job(2).map(|x| at(11.8 + 0.4 * d(5), for_(0.6, x))));
                w.extend(land_job(1).map(|x| at(12.6 + 0.3 * d(6), x)));
            }
            if homeward {
                w.push(for_(0.8, at(dusk - 0.3 * d(7), inside)));
                evening(&mut w, taken, 0.5);
                w.push(bed(22.0 + 0.6 * d(10)));
            } else {
                w.extend(land_job(2).map(|x| at(18.4 + 0.5 * d(8), for_(1.5, x))));
                w.extend(camp_bed.map(|b| Want { start: 21.0 + 0.6 * d(10), ..b }));
            }
        }
        Role::Worker if goes_out => {
            // Up early, out to the work in the land, a bite at midday there, and home
            // before dusk.
            let wake = 5.0 + 0.5 * d(1);
            w.push(for_(0.3, at(wake, inside)));
            w.extend(land_job(0).map(|x| at(7.0 + 0.4 * d(4), x)));
            w.extend(land_job(2).map(|x| at(11.8 + 0.4 * d(5), for_(0.6, x))));
            w.extend(land_job(1).map(|x| at(12.6 + 0.3 * d(6), x)));
            w.push(for_(0.8, at(dusk - 0.3 * d(7), inside)));
            evening(&mut w, taken, if dow >= 5 { 0.45 } else { 0.22 });
            w.push(bed(22.0 + 0.6 * d(10)));
        }
        Role::Worker | Role::Seller if !rest || keeps.is_some() => {
            let wake = 5.6 + d(1);
            w.push(for_(0.3, at(wake, inside)));
            // A stall's keeper is off to set it up instead of to the well.
            if keeps.is_none() && d(2) < if r.role == Role::Seller { 0.3 } else { 0.4 } {
                w.extend(well(taken, 3).map(|x| at(wake + 0.6 + 0.3 * d(20), x)));
                w.push(for_(0.3, inside));
            }
            let morning = 7.8 + 0.5 * d(4);
            let noon = 11.9 + 0.3 * d(5);
            let afternoon = 13.1 + 0.3 * d(6);
            let evening_in = 17.0 + 0.6 * d(7);
            match keeps {
                Some(s) => {
                    // Out to set the stall up before it opens, and behind it until it
                    // closes (#207).
                    let open = calendar::MARKET_OPENS - 0.25 + 0.2 * d(4);
                    let shut = calendar::MARKET_CLOSES + 0.1 + 0.25 * d(5);
                    w.push(Want { place: town.stalls[s], spot: town.behind[s], start: open, stay: shut - open });
                    w.push(for_(0.6, inside));
                }

                None => {
                    w.extend(job(0).map(|x| at(morning, x)));
                    // To the market at midday on market day.
                    match stall(taken, 8).filter(|_| d(9) < 0.85) {
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
            // A day of rest that is market day too takes most of them to the stalls.
            let shopping = if d(18) < 0.6 { stall(taken, 19) } else { None };
            let morning = if let Some(s) = shopping {
                Some(s)
            } else if u < 0.3 {
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
            // To market in the morning, any time from its opening to midday.
            match stall(taken, 8).filter(|_| d(9) < 0.9) {
                Some(s) => w.push(at(calendar::MARKET_OPENS + 0.6 + 3.0 * d(16), for_(0.4 + 0.5 * d(17), s))),
                None if rest && d(9) < 0.6 => w.extend(haunt(taken, town.shrine, Task::Stand).map(|x| at(10.2, x))),
                None => {}
            }
            w.push(for_(1.0, at(12.4 + 0.3 * d(13), inside)));
            let u = d(14);
            if let Some(s) = stall(taken, 21).filter(|_| d(22) < 0.35) {
                // Back to the market in the afternoon, for what was forgotten.
                w.push(at(14.0 + 1.2 * d(15), for_(0.4, s)));
            } else if u < 0.45 {
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
        // Their day is `visit`'s.
        Role::Visitor => {}
        Role::Elder => {
            let wake = 5.4 + d(1);
            w.push(for_(0.8, at(wake + 0.2, inside)));
            w.push(at(8.0 + 0.6 * d(4), for_(2.5, door_sit(r.member as u32))));
            if market && d(9) < 0.7 {
                w.extend(stall(taken, 8).map(|s| at(10.0 + 1.6 * d(19), s)));
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
    // The hens fed of a morning, every day (#206), at the hour they gather for it.
    if let Some(h) = town.hens.iter().find(|h| h.feeder == Some(r.n)) {
        let (from, until) = crate::animals::herd::feeding(h.key, day);
        let feed = Want { place: town.plots[h.home].place, spot: h.spot, start: from, stay: until - from };
        let i = (1..w.len()).find(|&i| w[i].start.is_finite() && w[i].start > from).unwrap_or(w.len());
        w.insert(i, feed);
    }
    for x in &mut w {
        if x.spot.task == Task::Sit && x.spot.inside.is_none() && x.place == town.green_place {
            x.spot.stool = true;
        }
    }
    settle(town, w)
}

/// The day of `r`, in from one of the hamlets (#207), `d` their day's dice: away all
/// day but on `market` day, when they walk in along the trail in the morning, go round
/// the stalls, stop on the green a while, and walk home in the afternoon.
fn visit(town: &Town, r: &Resident, market: bool, d: &dyn Fn(i32) -> f32, taken: &mut Taken) -> Plan {
    let away = Want { place: r.jobs[0].place, spot: r.bed, start: 0.0, stay: 0.0 };
    let mut w = vec![away];
    if market && !town.stalls.is_empty() {
        let mut start = calendar::MARKET_OPENS + 0.4 + 1.6 * d(1);
        for k in 0..2 + (d(2) < 0.5) as i32 {
            let i = (d(10 + k) * town.stalls.len() as f32) as usize % town.stalls.len();
            let slot = taken.stalls[i] % 6;
            taken.stalls[i] += 1;
            let stay = 0.3 + 0.35 * d(20 + k);
            w.push(Want { place: town.stalls[i], spot: around(town, town.stalls[i], slot, Task::Talk), start, stay });
            start += stay + 0.1;
        }
        if d(3) < 0.6 {
            let k = taken.green;
            taken.green += 1;
            w.push(Want { place: town.green_place, spot: talking(town, k, Task::Talk), start: f32::NEG_INFINITY, stay: 0.4 + 0.4 * d(4) });
        }
        // Home again in the afternoon, before the stalls are packed away.
        w.push(Want { start: 13.4 + 1.8 * d(5), ..away });
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
