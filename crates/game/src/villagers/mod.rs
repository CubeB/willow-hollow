//! The people of the settlements (#204). Everyone who sleeps in a settlement's beds has
//! a name, a household, a trade and a day: up in the morning and to the well, at work
//! through the day, to the market at midday, the tavern or the green in the evening, and
//! home with the lights on before bed. The player can greet them (E), and they stop and
//! say a word about where they live.
//!
//! A settlement's people are made when the player comes near it, on a thread of their
//! own, and let go when the player has gone well away (town.rs). Their days are a pure
//! function of the settlement and the day (day.rs), so wherever and whenever the player
//! comes, the same people are in the same places. They walk only along the settlement's
//! lanes and the paths from its doors (lanes.rs), and are seen walking only near: further
//! off each is shown where they are going. Indoors they are out of sight unless the
//! player is in there with them.

pub mod day;
// What a settlement puts out on market days and festivals (#207).
pub mod dress;
pub mod lanes;
// What is pinned on its boards (#207).
pub mod notices;
pub mod talk;
pub mod town;
#[cfg(test)]
mod tests;

use std::{
    collections::HashMap,
    sync::{Arc, mpsc},
};

use glam::{DVec3, Vec2, Vec3};
use worldgen::{
    continent::Continent,
    macro_layout::{MACRO_SIZE, cell_of},
    noise::{hash2, unit},
    scale::{m, m64},
    world::GameWorld,
};

use crate::{
    character::{self, Carried, Doing, Gait, Task},
    particles::{self, ParticleInstance},
};
use day::{Now, Plan};
use lanes::{Walk, flat};
pub use town::{Resident, Town};

/// The most of a frame's props the settlements' people take: the nearest drawn whole and
/// more further off in a few boxes each. render.rs makes room for this many beside its
/// own.
pub const MOST_BOXES: usize = MOST_NEAR * FIGURE_BOXES + MOST_FAR * FAR_BOXES;
/// How many are drawn whole, the nearest first, and how many more in a few boxes.
pub const MOST_NEAR: usize = 24;
pub const MOST_FAR: usize = 64;
/// The most boxes one figure takes, whole with a stool under them, and far (tests.rs
/// measures every look against these).
pub const FIGURE_BOXES: usize = 48;
pub const FAR_BOXES: usize = 10;
/// How far people are seen walking along the lanes, in metres; beyond, each is shown
/// where they are going, or not at all if that is indoors.
pub const WALK_SIGHT: f32 = 150.0;
/// How near the edge of a settlement the player comes before its people are made, and
/// how far beyond it they are let go, in metres.
const COME: f32 = 250.0;
const GO: f32 = 600.0;
/// The widest a settlement reaches from its middle, in metres.
const WIDEST: f32 = 130.0;
/// How often the settlements about are looked for, in seconds.
const EVERY: f32 = 2.0;
/// How near someone has to be to be greeted, in metres; how long they stop to talk, and
/// how long they take to get back to what they were doing, in seconds.
pub const GREET_REACH: f32 = 3.0;
const STOP: f32 = 8.0;
const EASE: f32 = 2.0;
/// How fast people are seen to walk, in metres a second: the pace of day.rs at the
/// default length of day.
const STEP_SPEED: f32 = 1.3;
/// How far one step takes them, in metres.
const STEP: f32 = 0.7;

/// One of the people of the settlements: their settlement's continent and its index
/// there, and their number in it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Who {
    pub cell: (i32, i32),
    pub k: usize,
    pub n: usize,
}

impl Who {
    /// How the save knows them.
    pub fn key(self) -> String {
        format!("{},{},{},{}", self.cell.0, self.cell.1, self.k, self.n)
    }
}

/// A settlement near the player, with everyone's day today.
struct Near {
    town: Arc<Town>,
    day: i64,
    plans: Vec<Plan>,
    /// The walks being drawn, by who is walking and from which of their stays.
    routes: HashMap<(usize, usize), Walk>,
}

/// Someone stopped to talk to the player.
struct Stopped {
    who: Who,
    /// Where they stopped, continent-local, and the way to the player.
    at: Vec3,
    facing: f32,
    since: f32,
    /// The building they were in, if any.
    inside: Option<usize>,
}

/// Someone where they are this moment, as drawn.
#[derive(Clone, Copy, Debug)]
pub struct Figure {
    pub who: Who,
    /// The feet in the world (for someone in bed, the soles).
    pub feet: DVec3,
    pub facing: f32,
    /// What they are at, or None while walking.
    pub task: Option<Task>,
    /// How many steps they have taken on this walk.
    pub stride: f32,
    pub carried: Carried,
    pub stool: bool,
    pub inside: Option<usize>,
}

/// What was drawn last frame.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Drawn {
    pub near: usize,
    pub far: usize,
    pub boxes: usize,
}

/// A board read (#207): what is on it, and what of it goes in the journal.
#[derive(Clone, Debug)]
pub struct Reading {
    pub page: crate::reading::Page,
    /// The settlement it is in: how the journal knows it, and its name.
    pub key: String,
    pub town: String,
    /// The requests on a notice board.
    pub requests: Vec<notices::Request>,
    /// For a guild house with wayfarers, the settlements they mark on the map, by name
    /// and where each is (world x and z), and the middle of the country marked and its
    /// reach.
    pub chart: Option<(Vec<(String, glam::DVec2)>, glam::DVec2, f64)>,
}

/// Someone greeted, and what they said.
#[derive(Clone, Debug)]
pub struct Greeting {
    pub name: String,
    pub town: String,
    pub line: String,
}

/// The settlements near the player and their people.
pub struct Villagers {
    near: Vec<Near>,
    /// Those being made on threads of their own.
    asked: Vec<((i32, i32), usize)>,
    sender: mpsc::Sender<Town>,
    made: mpsc::Receiver<Town>,
    timer: f32,
    clock: f32,
    stopped: Option<Stopped>,
    /// This frame's figures, nearest the camera first.
    seen: Vec<Figure>,
    pub drawn: Drawn,
    /// The time of year and the days to a year, as last updated: for what is dressed
    /// at a festival (#207).
    date: f32,
    year_days: f32,
    /// How many boxes the dressing took last frame.
    pub dressed: usize,
}

impl Default for Villagers {
    fn default() -> Self {
        Self::new()
    }
}

/// Which day of the world it is (from the first), for `hour` of time of year `date` in
/// `year`, with `year_days` days to a year: it turns at midnight.
pub fn day_number(hour: f32, date: f32, year: u32, year_days: f32) -> i64 {
    ((year as f64 + date.rem_euclid(1.0) as f64) * year_days.max(1.0) as f64 - hour.rem_euclid(24.0) as f64 / 24.0).round() as i64
}

/// The settlements near `at` whose people should be about, each with its continent:
/// waiting for the continent `at` is on if `wait`, and taking any other only if it is
/// ready.
fn wanted(world: &GameWorld, at: DVec3, wait: bool) -> Vec<((i32, i32), usize, Arc<Continent>)> {
    let reach = m(COME + WIDEST) as i32;
    let (x, z) = (at.x.floor() as i32, at.z.floor() as i32);
    let (lo, hi) = (cell_of(x - reach, z - reach), cell_of(x + reach, z + reach));
    let mut out = Vec::new();
    for mz in lo.1..=hi.1 {
        for mx in lo.0..=hi.0 {
            let c = if wait && (mx, mz) == cell_of(x, z) { Some(world.continent(mx, mz)) } else { world.continent_if_ready(mx, mz) };
            let Some(c) = c else { continue };
            let local = Vec2::new((at.x - (mx * MACRO_SIZE) as f64) as f32, (at.z - (mz * MACRO_SIZE) as f64) as f32);
            for (k, s) in c.trails.settlements.iter().enumerate() {
                if k < c.trails.layouts.len() && s.at().distance(local) < s.radius + m(COME) {
                    out.push(((mx, mz), k, c.clone()));
                }
            }
        }
    }
    out
}

impl Villagers {
    pub fn new() -> Self {
        let (sender, made) = mpsc::channel();
        Self {
            near: Vec::new(),
            asked: Vec::new(),
            sender,
            made,
            timer: 0.0,
            clock: 0.0,
            stopped: None,
            seen: Vec::new(),
            drawn: Drawn::default(),
            date: 0.0,
            year_days: 1.0,
            dressed: 0,
        }
    }

    /// Each settlement about, with when it is there (#207).
    fn whens(&self, hour: f32, lit: f32) -> impl Iterator<Item = (&Town, dress::When)> + '_ {
        let (date, year_days, clock) = (self.date, self.year_days, self.clock);
        self.near.iter().map(move |n| (&*n.town, dress::When { day: n.day, date, year_days, hour, lit, clock }))
    }

    /// Draws what the settlements about have out at `hour` (#207): their stalls on market
    /// day, and a festival's lanterns and garlands, `lit` as brightly as a lantern burns
    /// now (0 by day). Keeps to [`dress::BUDGET`] boxes.
    pub fn dress(&mut self, cam: DVec3, hour: f32, lit: f32, out: &mut Vec<ParticleInstance>) {
        self.dressed = dress::draw(self.whens(hour, lit), cam, out);
    }

    /// The festival lanterns burning near `cam`, as lamps.
    pub fn lamps(&self, cam: DVec3, hour: f32, lit: f32) -> Vec<(DVec3, f32)> {
        dress::lamps(self.whens(hour, lit), cam)
    }

    /// Every settlement whose people are about, nearest `at` first.
    pub fn towns(&self) -> impl Iterator<Item = &Town> {
        self.near.iter().map(|n| &*n.town)
    }

    /// The people of settlement `k` of continent `cell`, if they are about.
    pub fn residents(&self, cell: (i32, i32), k: usize) -> Option<&[Resident]> {
        self.near.iter().find(|n| n.town.cell == cell && n.town.k == k).map(|n| &n.town.residents[..])
    }

    fn add(&mut self, town: Town, day: i64) {
        self.asked.retain(|&a| a != (town.cell, town.k));
        if self.near.iter().any(|n| n.town.cell == town.cell && n.town.k == town.k) {
            return;
        }
        let plans = day::plans(&town, day);
        log::debug!("the people of {} are about: {}", town.name, town.residents.len());
        self.near.push(Near { town: Arc::new(town), day, plans, routes: HashMap::new() });
    }

    /// Looks for the settlements about `at` now and then, making the people of any
    /// newly near on a thread of their own, and letting go of those left far behind; and
    /// keeps everyone's day the day it is.
    #[allow(clippy::too_many_arguments)]
    pub fn update(&mut self, dt: f32, world: &GameWorld, at: DVec3, hour: f32, date: f32, year: u32, year_days: f32) {
        self.clock += dt;
        (self.date, self.year_days) = (date, year_days);
        let today = day_number(hour, date, year, year_days);
        while let Ok(town) = self.made.try_recv() {
            self.add(town, today);
        }
        self.timer -= dt;
        if self.timer <= 0.0 {
            self.timer = EVERY;
            for (cell, k, c) in wanted(world, at, false) {
                if self.asked.contains(&(cell, k)) || self.near.iter().any(|n| n.town.cell == cell && n.town.k == k) {
                    continue;
                }
                self.asked.push((cell, k));
                let (tx, seed) = (self.sender.clone(), world.seed());
                let made = std::thread::Builder::new().name("villagers".into()).spawn(move || {
                    crate::threads::background();
                    if let Some(town) = Town::build(&c, seed, cell, k) {
                        let _ = tx.send(town);
                    }
                });
                if made.is_err() {
                    log::warn!("could not start a thread to make a settlement's people");
                }
            }
            self.near.retain(|n| {
                let t = &n.town;
                let d = (at - t.origin).as_vec3();
                flat(d).distance(flat(t.centre)) < t.radius + m(GO)
            });
        }
        for n in &mut self.near {
            if n.day != today {
                n.plans = day::plans(&n.town, today);
                n.day = today;
                n.routes.clear();
            }
        }
    }

    /// Makes the people of every settlement about `at` at once, waiting for them: for a
    /// screenshot, and the console.
    pub fn settle(&mut self, world: &GameWorld, at: DVec3, hour: f32, date: f32, year: u32, year_days: f32) {
        let today = day_number(hour, date, year, year_days);
        (self.date, self.year_days) = (date, year_days);
        for (cell, k, c) in wanted(world, at, true) {
            if self.near.iter().any(|n| n.town.cell == cell && n.town.k == k) {
                continue;
            }
            if let Some(town) = Town::build(&c, world.seed(), cell, k) {
                self.add(town, today);
            }
        }
        for n in &mut self.near {
            if n.day != today {
                n.plans = day::plans(&n.town, today);
                n.day = today;
                n.routes.clear();
            }
        }
    }

    /// Where everyone near enough to be drawn is at `hour`, seen from `cam` by the player
    /// standing at `player`, nearest the camera first.
    fn place_all(&mut self, player: DVec3, cam: DVec3, hour: f32) {
        self.seen.clear();
        let reach = m(character::SIGHT);
        let clock = self.clock;
        if self.stopped.as_ref().is_some_and(|s| clock - s.since > STOP + EASE) {
            self.stopped = None;
        }
        for near in &mut self.near {
            let town = &near.town;
            let eye = (cam - town.origin).as_vec3();
            if flat(eye).distance(flat(town.centre)) > town.radius + reach {
                continue;
            }
            let feet = (player - town.origin).as_vec3();
            let indoors = town.inside(feet);
            for r in &town.residents {
                let who = Who { cell: town.cell, k: town.k, n: r.n };
                let Some(mut f) = figure(town, &near.plans[r.n], r, hour, eye, &mut near.routes) else { continue };
                if let Some(s) = self.stopped.as_ref().filter(|s| s.who == who) {
                    let t = clock - s.since;
                    let local = (f.0, f.1);
                    if t < STOP {
                        (f.0, f.1, f.2, f.6) = (s.at, s.facing, Some(Task::Talk), s.inside);
                    } else {
                        let k = ((t - STOP) / EASE).clamp(0.0, 1.0);
                        f.0 = s.at.lerp(local.0, k);
                    }
                }
                // Indoors, only for someone in there with them.
                if f.6.is_some() && f.6 != indoors {
                    continue;
                }
                if f.0.distance(eye) > reach {
                    continue;
                }
                self.seen.push(Figure { who, feet: town.origin + f.0.as_dvec3(), facing: f.1, task: f.2, stride: f.3, carried: f.4, stool: f.5, inside: f.6 });
            }
        }
        self.seen.sort_by(|a, b| a.feet.distance_squared(cam).total_cmp(&b.feet.distance_squared(cam)));
    }

    /// Draws everyone in sight at `hour`, seen from `cam` by the player standing at
    /// `player`: the nearest [`MOST_NEAR`] whole, then [`MOST_FAR`] more in a few boxes,
    /// and never more than [`MOST_BOXES`] boxes in all.
    pub fn draw(&mut self, player: DVec3, cam: DVec3, hour: f32, out: &mut Vec<ParticleInstance>) {
        self.place_all(player, cam, hour);
        let from = out.len();
        let mut drawn = Drawn::default();
        let near_m = m64(character::NEAR as f64);
        for f in &self.seen {
            let Some(r) = self.near.iter().find(|n| n.town.cell == f.who.cell && n.town.k == f.who.k).map(|n| &n.town.residents[f.who.n]) else { continue };
            let d = f.feet.distance(cam);
            let whole = d < near_m && drawn.near < MOST_NEAR;
            if whole && out.len() - from + FIGURE_BOXES <= MOST_BOXES {
                draw_one(r, f, self.clock, cam, out);
                drawn.near += 1;
            } else if !whole && drawn.far < MOST_FAR && f.task != Some(Task::Lie) && out.len() - from + FAR_BOXES <= MOST_BOXES {
                let start = out.len();
                let look = character::Look { carried: f.carried, ..r.person.look };
                let pose = pose_of(r, f, self.clock);
                character::draw_far(&look, f.feet, f.facing, &pose, cam, out);
                particles::dither_since(out, start, particles::dither_edge(d, m64(character::SIGHT as f64), m64(20.0)));
                drawn.far += 1;
            }
        }
        drawn.boxes = out.len() - from;
        self.drawn = drawn;
    }

    /// The board looked at along `forward` from `eye`, near enough to read (#207): which
    /// settlement about it is in and which of its boards, and what it is called.
    pub fn board_in_sight(&self, eye: DVec3, forward: Vec3) -> Option<((usize, usize), &'static str)> {
        for (t, n) in self.near.iter().enumerate() {
            let town = &n.town;
            let e = (eye - town.origin).as_vec3();
            for (b, &(plot, at, facing)) in town.boards.iter().enumerate() {
                if notices::readable(at, facing, e, forward, m(crate::reading::REACH)) {
                    let guilds = town.guilds.as_ref().is_some_and(|g| g.0 == plot);
                    return Some(((t, b), if guilds { "the guilds' board" } else { "the notice board" }));
                }
            }
        }
        None
    }

    /// Reads board `which` (from [`Villagers::board_in_sight`]): `charted` says whether
    /// the wayfarers of the settlement known by a key have marked the country about on the
    /// map already.
    pub fn read(&self, which: (usize, usize), charted: impl Fn(&str) -> bool) -> Option<Reading> {
        let n = self.near.get(which.0)?;
        let town = &n.town;
        let &(plot, ..) = town.boards.get(which.1)?;
        let key = format!("{},{},{}", town.cell.0, town.cell.1, town.k);
        let charted = charted(&key);
        let world = |p: Vec2| glam::DVec2::new(town.origin.x + p.x as f64, town.origin.z + p.y as f64);
        match &town.guilds {
            Some((at, guilds)) if *at == plot => {
                let chart = guilds.contains(&worldgen::settlements::special::Guild::Wayfarers).then(|| {
                    let places: Vec<(String, glam::DVec2)> = town.neighbours.iter().map(|nb| (nb.name.clone(), world(nb.at))).collect();
                    // A little beyond the neighbours' middles, so that the map's mark for
                    // each, which stands on its green, is within it too.
                    (places, world(flat(town.centre)), (town::NEIGHBOURS + m(150.0)) as f64)
                });
                let newly: Option<Vec<String>> = chart.as_ref().filter(|_| !charted).map(|c| c.0.iter().map(|p| p.0.clone()).collect());
                let page = notices::guild_page(town, guilds, n.day, newly.as_deref());
                Some(Reading { page, key, town: town.name.clone(), requests: Vec::new(), chart })
            }
            _ => {
                let (page, requests) = notices::board_page(town, n.day, self.year_days);
                Some(Reading { page, key, town: town.name.clone(), requests, chart: None })
            }
        }
    }

    /// Whoever is near enough to `feet` to greet, and in sight: the nearest.
    pub fn within_reach(&self, feet: DVec3) -> Option<(Who, &str)> {
        let reach = m64(GREET_REACH as f64);
        self.seen
            .iter()
            .filter(|f| f.task != Some(Task::Lie))
            .map(|f| (f, (f.feet.x - feet.x).hypot(f.feet.z - feet.z)))
            .filter(|(f, d)| *d < reach && (f.feet.y - feet.y).abs() < m64(1.6))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .and_then(|(f, _)| Some((f.who, self.resident(f.who)?.person.name.as_str())))
    }

    /// Someone, if they are about.
    pub fn resident(&self, who: Who) -> Option<&Resident> {
        self.near.iter().find(|n| n.town.cell == who.cell && n.town.k == who.k).and_then(|n| n.town.residents.get(who.n))
    }

    /// Greets `who`, met `met` times before, as the player stands at `feet` at `hour` on
    /// `date`: they stop and turn to the player, and say a word.
    pub fn greet(&mut self, who: Who, feet: DVec3, met: u32, hour: f32, date: f32) -> Option<Greeting> {
        let near = self.near.iter().find(|n| n.town.cell == who.cell && n.town.k == who.k)?;
        let town = &near.town;
        let r = town.residents.get(who.n)?;
        let (at, inside) = match self.seen.iter().find(|f| f.who == who) {
            Some(f) => ((f.feet - town.origin).as_vec3(), f.inside),
            None => {
                let mut routes = HashMap::new();
                let f = figure(town, &near.plans[who.n], r, hour, (feet - town.origin).as_vec3(), &mut routes)?;
                (f.0, f.6)
            }
        };
        let to = flat((feet - town.origin).as_vec3()) - flat(at);
        let facing = to.y.atan2(to.x);
        let line = talk::greeting(town, r, met, hour, date);
        let greeting = Greeting { name: r.person.name.clone(), town: town.name.clone(), line };
        self.stopped = Some(Stopped { who, at, facing, since: self.clock, inside });
        Some(greeting)
    }

    /// Who is about at `hour`, for the console: the settlement nearest `at`, how many of
    /// its people are where, and the nearest few of them, or with `all` every one.
    pub fn report(&self, at: DVec3, hour: f32, all: bool) -> String {
        let Some(near) = self.near.iter().min_by(|a, b| {
            let d = |n: &Near| flat((at - n.town.origin).as_vec3()).distance(flat(n.town.centre));
            d(a).total_cmp(&d(b))
        }) else {
            return "nobody is about: there is no settlement near".into();
        };
        let town = &near.town;
        let residents = self.residents(town.cell, town.k).unwrap_or_default();
        let mut count: Vec<(String, usize)> = Vec::new();
        let mut tally = |what: String| match count.iter_mut().find(|c| c.0 == what) {
            Some(c) => c.1 += 1,
            None => count.push((what, 1)),
        };
        for r in residents {
            let plan = &near.plans[r.n];
            let what = match plan.now(hour) {
                Now::Walking(..) => "walking".to_string(),
                Now::At(i) => {
                    let s = &plan.stays[i];
                    match (s.spot.task, s.spot.inside) {
                        (Task::Lie, _) => "in bed".into(),
                        (_, Some(b)) if Some(b) == town.tavern && r.home != b => "at the tavern".into(),
                        (_, Some(_)) => "indoors".into(),
                        (t, None) => format!("{} at {}", t.word(), town.place_word(s.place)),
                    }
                }
            };
            tally(what);
        }
        count.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let mut out = format!(
            "{}, a {}: {} people in {} households, at {:02}:{:02}",
            town.name,
            town.kind.word(),
            residents.len(),
            residents.iter().filter(|r| r.member == 0).count(),
            hour as u32,
            ((hour.fract()) * 60.0) as u32
        );
        for (what, n) in &count {
            out += &format!("\n  {n} {what}");
        }
        // Where its haunts are, ready for tp.
        let mut haunts: Vec<(usize, &str)> = town.wells.iter().map(|&p| (p, "well")).collect();
        haunts.extend(town.stalls.first().map(|&p| (p, "market")));
        for (p, word) in [(town.tavern.map(|t| town.plots[t].place), "tavern"), (town.tree, "village tree"), (town.shrine, "shrine"), (town.water, "water")] {
            haunts.extend(p.map(|p| (p, word)));
        }
        let spots: Vec<String> = haunts
            .iter()
            .map(|&(p, word)| {
                let at = town.origin + town.places[p].at.as_dvec3();
                format!("{word} {:.0},{:.0}", at.x, at.z)
            })
            .collect();
        let middle = town.origin + town.centre.as_dvec3();
        out += &format!("\n  green {:.0},{:.0}; {}", middle.x, middle.z, spots.join("; "));
        if let Some(t) = town.tavern {
            // Just inside its door, looking in, for `--over` and `--look`.
            let b = &town.plots[t];
            let inside = town.origin + (b.doorway - Vec3::new(b.out.x, 0.0, b.out.y) * 2.0).as_dvec3();
            let look = (-b.out.y).atan2(-b.out.x).to_degrees().rem_euclid(360.0);
            out += &format!("\n  inside the tavern {:.0},{:.0} looking {look:.0}", inside.x, inside.z);
        }
        // #207: its market days, today's festival and its boards, for shots of them.
        use worldgen::settlements::calendar;
        let every = calendar::market_every(town.kind);
        if !town.market.is_empty() && every > 0 {
            let next = (0..every * 2).find(|&n| town.market_on(near.day + n)).unwrap_or(0);
            let noon = (near.day + next) as f64 / self.year_days.max(1.0) as f64 + 0.5 / self.year_days.max(1.0) as f64;
            let when = if next == 0 { "today".to_string() } else { format!("in {next} days") };
            out += &format!("\n  market every {every} days, next {when}: noon then is --year {} --date {:.4}", noon.floor(), noon.fract());
        }
        if let Some(f) = calendar::festival_on(near.day, self.year_days) {
            out += &format!("\n  {} today", f.name());
        }
        for &(plot, at, facing) in &town.boards {
            let what = match &town.guilds {
                Some((at, guilds)) if *at == plot => format!("the guilds' board ({})", guilds.iter().map(|g| g.name()).collect::<Vec<_>>().join(", ")),
                _ => "the notice board".into(),
            };
            // A pace and a half before it, looking at it.
            let feet = town.origin + (at + Vec3::new(facing.x, 0.0, facing.y) * m(1.4)).as_dvec3();
            let look = (-facing.y).atan2(-facing.x).to_degrees().rem_euclid(360.0);
            out += &format!("\n  {what} {:.1},{:.1},{:.1}, read from {:.1},{:.1} looking {look:.0}", town.origin.x + at.x as f64, town.origin.y + at.y as f64, town.origin.z + at.z as f64, feet.x, feet.z);
        }
        if all {
            // Everyone, by household, and where each is.
            out += "\neveryone:";
            for r in residents {
                let plan = &near.plans[r.n];
                let (doing, at) = match plan.now(hour) {
                    Now::At(i) => {
                        let s = &plan.stays[i];
                        (format!("{} at {}", s.spot.task.word(), town.place_word(s.place)), s.spot.at)
                    }
                    Now::Walking(i, t) => {
                        let (a, b) = (&plan.stays[i], &plan.stays[i + 1]);
                        (format!("walking to {}", town.place_word(b.place)), a.spot.at.lerp(b.spot.at, t))
                    }
                };
                let at = town.origin + at.as_dvec3();
                out += &format!("\n  {} ({} {}, {:?}), {doing}, at {:.0},{:.0}", r.person.name, r.person.age.word(), r.person.trade.word(), r.role, at.x, at.z);
            }
            return out;
        }
        let nearest: Vec<&Figure> = self.seen.iter().filter(|f| f.who.cell == town.cell && f.who.k == town.k).take(8).collect();
        if !nearest.is_empty() {
            out += "\nnearest:";
        }
        for f in nearest {
            let r = &town.residents[f.who.n];
            let doing = f.task.map_or("walking", |t| t.word());
            let metres = f.feet.distance(at) / m64(1.0);
            out += &format!("\n  {} ({} {}), {doing}, {metres:.0} m, at {:.0},{:.0}", r.person.name, r.person.age.word(), r.person.trade.word(), f.feet.x, f.feet.z);
        }
        out
    }

    /// The figures in sight last drawn, nearest the camera first.
    pub fn seen(&self) -> &[Figure] {
        &self.seen
    }
}

/// Where `r` is at `hour` by their `plan`, seen from `eye` (all continent-local): their
/// feet, facing, task, steps, what they carry, whether on a stool, and the building they
/// are inside; None when they are on the way somewhere indoors, too far off to be seen
/// walking.
type Placed = (Vec3, f32, Option<Task>, f32, Carried, bool, Option<usize>);

fn figure(town: &Town, plan: &Plan, r: &Resident, hour: f32, eye: Vec3, routes: &mut HashMap<(usize, usize), Walk>) -> Option<Placed> {
    let own = r.person.look.carried;
    let at = |s: &day::Stay| -> Placed {
        let spot = &s.spot;
        let mut feet = spot.at;
        if spot.task == Task::Lie {
            let way = Vec2::from_angle(spot.facing);
            feet += Vec3::new(way.x, 0.0, way.y) * character::lying_length(&r.person.look.build) * 0.92;
        }
        (feet, spot.facing, Some(spot.task), 0.0, spot.task.carried(own), spot.stool, spot.inside)
    };
    match plan.now(hour) {
        Now::At(i) => Some(at(&plan.stays[i])),
        Now::Walking(i, t) => {
            let (a, b) = (&plan.stays[i], &plan.stays[i + 1]);
            let rough = a.spot.at.lerp(b.spot.at, t);
            if rough.distance(eye) > m(WALK_SIGHT) {
                // Too far to be seen walking: where they are going, if it is outdoors.
                let there = at(b);
                return there.6.is_none().then_some(there);
            }
            routes.entry((r.n, i)).or_insert_with(|| town.walk((a.place, &a.spot), (b.place, &b.spot)));
            if routes.len() > 4096 {
                routes.clear();
                return None;
            }
            let walk = &routes[&(r.n, i)];
            let along = t * walk.length();
            let (feet, way) = walk.at(along);
            let carried = match a.spot.task {
                Task::Fetch => Carried::Bucket,
                Task::Carry => Carried::Load,
                _ => own,
            };
            // Going from room to room, they are still indoors.
            let inside = a.spot.inside.filter(|_| a.spot.inside == b.spot.inside);
            Some((feet, way.y.atan2(way.x), None, along / m(STEP), carried, false, inside))
        }
    }
}

/// The pose of `f`, someone of `r`, at `clock` seconds.
fn pose_of(r: &Resident, f: &Figure, clock: f32) -> character::Pose {
    let phase = unit(hash2(r.person.seed, 7, 0x51e));
    match f.task {
        Some(t) => character::task_pose(t, &r.person.look.build, clock, phase),
        None => {
            let mut g = Gait::new(f.facing);
            g.update(Doing::Afoot(STEP_SPEED), f.stride, None, false, None, 0.0, 0.0);
            let mut pose = g.pose();
            if f.carried == Carried::Load {
                let held = character::task_pose(Task::Carry, &r.person.look.build, clock, phase);
                (pose.arm, pose.fore) = (held.arm, held.fore);
            }
            pose
        }
    }
}

/// Draws `f`, someone of `r`, whole.
fn draw_one(r: &Resident, f: &Figure, clock: f32, cam: DVec3, out: &mut Vec<ParticleInstance>) {
    let look = character::Look { carried: f.carried, ..r.person.look };
    let pose = pose_of(r, f, clock);
    match f.task {
        Some(t) => {
            if f.stool {
                character::draw_stool(f.feet, f.facing, cam, out);
            }
            character::draw_at(&look, f.feet, f.facing, &pose, t, cam, out);
        }
        None => character::draw(&look, f.feet, f.facing, &pose, None, cam, out),
    }
}
