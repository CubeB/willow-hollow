//! The journal: a quiet record of the walk, written as it is walked.
//!
//! Nothing is asked of the player to keep it. The first of each kind of thing gathered
//! is entered with where and when it was found; each named place is entered when it is
//! first reached on foot (or by boat, for water); the distance gone is kept by the way it
//! was gone, with the highest ground stood on; and the notable moments are set down in
//! a short log, a day at a time. Its window has a page for each. The shrines where
//! incense has been lit (#40) are kept too, with what each one's smoke told of.
//!
//! A journal is plain data, kept with its world's save as lines of text, so that a save
//! from an older or a newer game still reads.

use std::{collections::VecDeque, sync::Arc};

use glam::{DVec2, DVec3, Vec2};
use worldgen::{
    continent::Continent,
    features::plants::Flower,
    materials as mat,
    names::PlaceKind,
    scale::{OLD_VOXEL, VOXELS_PER_METRE, m},
    world::GameWorld,
};

use crate::inventory::{self, Item, Shape};

/// Most notes the log keeps; the oldest are let go first.
const NOTES: usize = 400;
/// How near a small place (a cairn, a waterfall, a tor) must be come to be reached.
const NEAR: f32 = m(45.0);
/// How near a lake's shore, and a river's bank, must be come for its name to be known.
/// Water lies open and is seen from well off, and a walk that keeps to the hillside
/// above a river has still come to it: only a small place has to be stood at.
pub const LAKE_REACH: f32 = m(150.0);
pub const RIVER_REACH: f32 = m(100.0);
/// How near a place must be for a find to be written down as made beside it.
const BESIDE: f32 = m(250.0);
/// How near a river's bank counts as walking along it.
const BANK: f32 = m(40.0);
/// How near the end of a river counts as having come to its mouth, in metres along it.
const MOUTH: f32 = 150.0;
/// The most a river must be walked down for it to count as followed to the sea, in
/// metres; a shorter river must be walked for half its length.
const FOLLOWED: f32 = 1500.0;
/// The most that can be gone in one frame, in voxels: more is a leap from one place to
/// another, not travel.
const LEAP: f64 = 40.0;

/// When something happened, in the world's own time.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Stamp {
    /// Time of year, 0 to 1 from the spring equinox.
    pub date: f32,
    /// Hour of the day.
    pub hour: f32,
    /// Which day of the walk, from 1.
    pub day: u32,
    /// Which year of the world, from 1 for its first (its year 0); the year turns at the
    /// spring equinox.
    pub year: u32,
}

const SEASONS: [&str; 4] = ["spring", "summer", "autumn", "winter"];

impl Stamp {
    fn to_text(self) -> String {
        format!("{},{},{},{}", self.date, self.hour, self.day, self.year)
    }

    fn from_text(text: &str) -> Option<Self> {
        let mut parts = text.split(',').map(str::trim);
        let date = parts.next()?.parse::<f32>().ok().filter(|v| v.is_finite())?.rem_euclid(1.0);
        let hour = parts.next()?.parse::<f32>().ok().filter(|v| v.is_finite())?.rem_euclid(24.0);
        let day = parts.next()?.parse::<u32>().ok()?.max(1);
        let year = parts.next()?.parse::<u32>().ok()?.max(1);
        Some(Self { date, hour, day, year })
    }

    /// Which season it is, 0 for spring to 3 for winter.
    fn quarter(self) -> usize {
        ((self.date.rem_euclid(1.0) * 4.0) as usize).min(3)
    }

    /// "early summer", "midwinter".
    pub fn season(self) -> String {
        let q = self.quarter();
        let third = ((self.date.rem_euclid(1.0) * 4.0).fract() * 3.0) as usize;
        match (third, q) {
            (0, _) => format!("early {}", SEASONS[q]),
            (1, 1) => "high summer".into(),
            (1, 3) => "midwinter".into(),
            (1, _) => format!("mid-{}", SEASONS[q]),
            _ => format!("late {}", SEASONS[q]),
        }
    }

    /// "at dusk", "in the morning".
    pub fn time_of_day(self) -> &'static str {
        match self.hour.rem_euclid(24.0) {
            h if h < 4.5 => "in the night",
            h if h < 7.0 => "at dawn",
            h if h < 11.0 => "in the morning",
            h if h < 14.0 => "about noon",
            h if h < 18.0 => "in the afternoon",
            h if h < 20.5 => "at dusk",
            _ => "in the night",
        }
    }

    /// "early summer of the first year".
    pub fn when(self) -> String {
        const NTH: [&str; 12] = ["first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth", "ninth", "tenth", "eleventh", "twelfth"];
        match NTH.get(self.year.saturating_sub(1) as usize) {
            Some(nth) => format!("{} of the {nth} year", self.season()),
            None => format!("{}, year {}", self.season(), self.year),
        }
    }
}

/// The first of a kind of thing gathered.
#[derive(Clone, PartialEq, Debug)]
pub struct Find {
    /// The thing, with all there is to know of it, as a pack's slot holds it.
    pub item: Item,
    /// Where it was: "by Carn Hollow", "in the Greywater Vale", or nothing.
    pub at: String,
    pub when: Stamp,
}

/// A named place reached.
#[derive(Clone, PartialEq, Debug)]
pub struct Visit {
    pub name: String,
    /// What kind of place, as `worldgen::names::PlaceKind::key` has it: "lake", "cairn".
    pub kind: String,
    /// The region it lies in; its own name, for a region.
    pub region: String,
    pub when: Stamp,
}

/// A river walked beside.
#[derive(Clone, PartialEq, Debug)]
pub struct River {
    pub name: String,
    /// The furthest up it from its mouth that its bank has been walked, in metres.
    pub far: f32,
    /// When it was followed down to the sea, if it has been.
    pub followed: Option<Stamp>,
}

/// A line of the log.
#[derive(Clone, PartialEq, Debug)]
pub struct Note {
    pub when: Stamp,
    pub text: String,
}

/// How the player is getting about.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Way {
    Foot,
    Glide,
    Sail,
    Swim,
}

#[derive(Default)]
pub struct Journal {
    finds: Vec<Find>,
    places: Vec<Visit>,
    rivers: Vec<River>,
    notes: VecDeque<Note>,
    /// Metres gone, by each way of going.
    pub walked: f64,
    pub glided: f64,
    pub sailed: f64,
    pub swum: f64,
    /// The highest ground stood on, in metres above the sea, with where and when.
    highest: Option<(f32, String, Stamp)>,
    /// The time of year as the journal last knew it, once the walk has begun.
    date: Option<f32>,
    /// Game days since the midnight before the walk began.
    days: f64,
    /// Seasons that have turned since the walk began, and the world's year as of the last
    /// tick (from 0). It is saved so that a world saved before it kept its own year can
    /// take its year from here.
    seasons: u32,
    years: u32,
    /// The year whose first snow has been written down.
    snow_year: u32,
    /// Changed since it was last saved.
    pub unsaved: bool,
    /// The present moment, and where the player stands, as of the last look.
    now: Stamp,
    here: String,
    last_pos: Option<DVec3>,
    /// The people of the settlements met (#204): who each is (see `villagers::Who`),
    /// their name, and how many times they have been greeted.
    met: Vec<Met>,
    /// The requests read on notice boards (#207), offered to the player: for errands to
    /// take up (#46).
    offered: Vec<Offer>,
    /// The settlements guild houses' wayfarers have marked on the map (#207).
    charted: Vec<Chart>,
    /// The shrines incense has been lit at (#40), the first first.
    shrines: Vec<Shrine>,
}

/// A shrine where incense has been lit (#40).
#[derive(Clone, Debug, PartialEq)]
pub struct Shrine {
    /// Where its burner is, world x and z.
    pub at: DVec2,
    /// What it is called in the journal: "the shrine by Hollin Tarn".
    pub name: String,
    /// How many sticks have been burned there.
    pub times: u32,
    /// The spring or shrine its smoke first told of, world x and z, if any.
    pub told: Option<DVec2>,
    /// When incense was first lit there.
    pub when: Stamp,
}

/// Which way `to` lies from `from` (world x and z, north toward -z), in a word or two.
pub fn compass(from: DVec2, to: DVec2) -> &'static str {
    const WAYS: [&str; 8] = ["east", "north-east", "north", "north-west", "west", "south-west", "south", "south-east"];
    let d = to - from;
    let turn = (-d.y).atan2(d.x).rem_euclid(std::f64::consts::TAU) / std::f64::consts::FRAC_PI_4;
    WAYS[(turn.round() as usize) % 8]
}

/// A request read on a settlement's notice board (#207).
#[derive(Clone, Debug, PartialEq)]
pub struct Offer {
    /// Which it is: `villagers::notices::Request::key`.
    pub key: String,
    /// Who asked, and of where.
    pub name: String,
    pub town: String,
    pub text: String,
    pub when: Stamp,
}

/// The settlements about a guild house, marked on the map by its wayfarers (#207); or,
/// under a key of their own (`villagers::notices::masons_key`), the quarries its masons
/// mark.
#[derive(Clone, Debug, PartialEq)]
pub struct Chart {
    /// The settlement the guild house is in: `villagers::Town::key`, and its name.
    pub key: String,
    pub town: String,
    /// The middle of the country marked, world x and z, and how far it reaches round.
    pub at: DVec2,
    pub reach: f64,
}

/// Someone of a settlement the player has greeted.
#[derive(Clone, Debug, PartialEq)]
pub struct Met {
    /// Which settlement, and which of its people: `villagers::Who::key`.
    pub key: String,
    pub name: String,
    pub times: u32,
}

fn clean(s: &str) -> String {
    s.replace(['|', '\n', '\r'], " ")
}

/// `name` as it reads in the middle of a sentence.
fn lower(name: &str) -> String {
    let mut c = name.chars();
    c.next().map_or_else(String::new, |f| f.to_lowercase().chain(c).collect())
}

/// The continent under world position `pos`, if it has been made, and `pos` in it.
pub fn continent_at(world: &GameWorld, pos: DVec3, wait: bool) -> Option<(Arc<Continent>, Vec2)> {
    use worldgen::macro_layout::{MACRO_SIZE, cell_of};
    let (mx, mz) = cell_of(pos.x.floor() as i32, pos.z.floor() as i32);
    let c = if wait { world.continent(mx, mz) } else { world.continent_if_ready(mx, mz)? };
    let local = Vec2::new((pos.x - mx as f64 * MACRO_SIZE as f64) as f32, (pos.z - mz as f64 * MACRO_SIZE as f64) as f32);
    Some((c, local))
}

/// How far along `line` from its start the point of it nearest `p` is, and how long the
/// whole line is, in voxels.
fn along(line: &[Vec2], p: Vec2) -> (f32, f32) {
    let (mut best, mut at, mut length) = (f32::MAX, 0.0, 0.0);
    for pair in line.windows(2) {
        let ab = pair[1] - pair[0];
        let t = ((p - pair[0]).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
        let d = (pair[0] + ab * t).distance(p);
        if d < best {
            (best, at) = (d, length + ab.length() * t);
        }
        length += ab.length();
    }
    (at, length)
}

/// Whether the sea lies within a few cells of continent-local `at`.
fn by_sea(c: &Continent, at: Vec2) -> bool {
    let (cs, n) = (c.cell_size(), c.size as i32);
    let (cx, cz) = ((at.x / cs) as i32, (at.y / cs) as i32);
    (-3..=3).any(|dz| {
        (-3..=3).any(|dx| {
            let (x, z) = (cx + dx, cz + dz);
            x >= 0 && z >= 0 && x < n && z < n && c.ocean[(z * n + x) as usize]
        })
    })
}

impl Journal {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn finds(&self) -> &[Find] {
        &self.finds
    }

    pub fn places(&self) -> &[Visit] {
        &self.places
    }

    pub fn rivers(&self) -> &[River] {
        &self.rivers
    }

    pub fn notes(&self) -> impl DoubleEndedIterator<Item = &Note> {
        self.notes.iter()
    }

    /// The find of the thing called `id`, if one has been made.
    pub fn find(&self, id: &str) -> Option<&Find> {
        self.finds.iter().find(|f| f.item.id == id)
    }

    /// How many times the person known by `key` has been greeted, if they are the one
    /// called `name` (a world made since may have someone else there).
    pub fn met(&self, key: &str, name: &str) -> u32 {
        self.met.iter().find(|m| m.key == key && m.name == name).map_or(0, |m| m.times)
    }

    /// Everyone greeted so far.
    #[cfg(test)]
    pub fn people_met(&self) -> &[Met] {
        &self.met
    }

    /// Writes down a greeting of `name` (known by `key`) of `town`, and returns how many
    /// times they have now been greeted. The first meeting goes in the notes.
    pub fn meet(&mut self, key: &str, name: &str, town: &str) -> u32 {
        self.unsaved = true;
        match self.met.iter_mut().find(|m| m.key == key) {
            Some(m) if m.name == name => {
                m.times += 1;
                m.times
            }
            found => {
                let m = Met { key: key.into(), name: name.into(), times: 1 };
                match found {
                    Some(old) => *old = m,
                    None => self.met.push(m),
                }
                self.note(format!("Met {name} of {town}."));
                1
            }
        }
    }

    /// Writes down a request read on `town`'s notice board, asked by `name`, unless it
    /// is written already; true if it was new. It goes in the notes too.
    pub fn offer(&mut self, key: &str, name: &str, town: &str, text: &str) -> bool {
        if self.offered.iter().any(|o| o.key == key) {
            return false;
        }
        self.offered.push(Offer { key: key.into(), name: name.into(), town: town.into(), text: text.into(), when: self.now });
        self.note(format!("Read on the notice board at {town}: {text}"));
        true
    }

    /// The requests read on notice boards, the first read first.
    pub fn offered(&self) -> &[Offer] {
        &self.offered
    }

    /// Writes down that the wayfarers of `town` (known by `key`) marked the country
    /// `reach` round `at` on the map, unless they have already; true if it was new.
    pub fn chart(&mut self, key: &str, town: &str, at: DVec2, reach: f64) -> bool {
        if self.charted.iter().any(|c| c.key == key) {
            return false;
        }
        self.charted.push(Chart { key: key.into(), town: town.into(), at, reach });
        self.note(format!("The wayfarers at {town} marked the settlements about on the map."));
        true
    }

    /// Writes down that the masons of `town` marked the quarries `reach` round `at` on the
    /// map (#207), under `key` (`villagers::notices::masons_key`), unless they have
    /// already; true if it was new. Kept and saved with the wayfarers' charts.
    pub fn chart_quarries(&mut self, key: &str, town: &str, at: DVec2, reach: f64) -> bool {
        if self.charted.iter().any(|c| c.key == key) {
            return false;
        }
        self.charted.push(Chart { key: key.into(), town: town.into(), at, reach });
        self.note(format!("The masons at {town} marked the quarries they know on the map."));
        true
    }

    /// What the wayfarers, and the masons, have marked on the map.
    pub fn charted(&self) -> &[Chart] {
        &self.charted
    }

    /// Writes down that a stick of incense was lit, now, at the burner at `at` (world x
    /// and z), before the shrine `named` (if it has a name) or on a `wayside` shelf, and
    /// that its smoke drifted off toward `told` (a place, and whether it is a shrine
    /// rather than a spring), if it told of anything. The first time at a shrine it is
    /// entered among them; every time it goes in the notes.
    pub fn light(&mut self, at: DVec2, wayside: bool, named: Option<&str>, told: Option<(DVec2, bool)>) {
        let what = if wayside { "a wayside shrine" } else { "the shrine" };
        let name = match named {
            Some(name) if !wayside => name.to_string(),
            _ if self.here.is_empty() => what.to_string(),
            _ => format!("{what} {}", self.here),
        };
        let k = match self.shrines.iter().position(|s| s.at.distance(at) < m(8.0) as f64) {
            Some(k) => {
                self.shrines[k].times += 1;
                self.shrines[k].told = self.shrines[k].told.or(told.map(|t| t.0));
                k
            }
            None => {
                self.shrines.push(Shrine { at, name: clean(&name), times: 1, told: told.map(|t| t.0), when: self.now });
                self.shrines.len() - 1
            }
        };
        let again = if self.shrines[k].times > 1 { " again" } else { "" };
        let lit = format!("Lit incense{again} at {} {}.", self.shrines[k].name, self.now.time_of_day());
        self.note(match told {
            Some((place, shrine)) => format!("{lit} Its smoke drifted away {}, toward {}.", compass(at, place), if shrine { "another shrine" } else { "a spring" }),
            None => format!("{lit} Its smoke went straight up."),
        });
    }

    /// The shrines incense has been lit at.
    pub fn shrines(&self) -> &[Shrine] {
        &self.shrines
    }

    /// The places the smoke of incense has told of, for the map.
    pub fn told(&self) -> Vec<DVec2> {
        self.shrines.iter().filter_map(|s| s.told).collect()
    }

    /// Seasons that have turned since the walk began.
    pub fn seasons(&self) -> u32 {
        self.seasons
    }

    /// Which day of the walk it is, from 1.
    pub fn day(&self) -> u32 {
        self.days.floor() as u32 + 1
    }

    /// The world's year as the journal last knew it, from 0, if it has known the world at
    /// all. Until the world kept its own year the journal counted the springs, so this is
    /// the year of a world saved before then.
    pub fn year(&self) -> Option<u32> {
        self.date.map(|_| self.years)
    }

    fn note(&mut self, text: String) {
        log::debug!("journal, day {}: {text}", self.now.day);
        self.notes.push_back(Note { when: self.now, text });
        while self.notes.len() > NOTES {
            self.notes.pop_front();
        }
        self.unsaved = true;
    }

    /// Moves the journal's clock on by `days` game days, to time of year `date` in the
    /// world's `year` (from 0), and `hour` of the day; `snowing` if snow is falling on the
    /// player. The seasons are read from the date as it changes, and the year is the
    /// world's, so the journal agrees with the world however its clock came to be where it
    /// is.
    pub fn tick(&mut self, days: f32, date: f32, year: u32, hour: f32, snowing: bool) {
        let (date, hour) = (date.rem_euclid(1.0), hour.rem_euclid(24.0));
        if days > 0.0 {
            self.days += days as f64;
            self.unsaved = true;
        }
        let before = match self.date.replace(date) {
            Some(before) => before,
            None => {
                // The first day began at the midnight before the walk did.
                self.days = hour as f64 / 24.0;
                self.unsaved = true;
                date
            }
        };
        if self.years != year {
            self.years = year;
            self.unsaved = true;
        }
        self.now = Stamp { date, hour, day: self.day(), year: self.years + 1 };
        if self.now.quarter() != (Stamp { date: before, ..self.now }).quarter() {
            self.seasons += 1;
            let season = SEASONS[self.now.quarter()];
            self.note(format!("{}{} came in.", season[..1].to_uppercase(), &season[1..]));
        }
        // A winter runs on into the spring of the next year: its snow is still its own.
        let winter = self.years + 1 + (date >= 0.5) as u32;
        if snowing && self.snow_year != winter {
            self.snow_year = winter;
            self.note("First snow.".into());
        }
    }

    /// Takes note of where the player is now, at `pos` (the feet, in world voxels), going
    /// by `way`, or by none when not in the world as a traveller; `grounded` if standing.
    pub fn moved(&mut self, pos: DVec3, way: Option<Way>, grounded: bool) {
        let from = self.last_pos.replace(pos);
        let Some(way) = way else {
            self.last_pos = None;
            return;
        };
        let gone = from.map_or(0.0, |f| (pos.x - f.x).hypot(pos.z - f.z));
        if gone > 0.0 && gone < LEAP {
            let metres = gone / VOXELS_PER_METRE as f64;
            match way {
                Way::Foot => self.walked += metres,
                Way::Glide => self.glided += metres,
                Way::Sail => self.sailed += metres,
                Way::Swim => self.swum += metres,
            }
            self.unsaved = true;
        }
        let height = pos.y as f32 / VOXELS_PER_METRE;
        if way == Way::Foot && grounded && height > self.highest.as_ref().map_or(0.0, |h| h.0 + 0.5) {
            self.highest = Some((height, self.here.clone(), self.now));
            self.unsaved = true;
        }
    }

    /// Enters the thing as found, here and now, if it is the first of its kind. Whether
    /// it was.
    pub fn found(&mut self, item: &Item) -> bool {
        if self.find(&item.id).is_some() {
            return false;
        }
        self.finds.push(Find { item: item.clone(), at: self.here.clone(), when: self.now });
        let at = if self.here.is_empty() { String::new() } else { format!(", {}", self.here) };
        self.note(format!("The first {}{at}.", lower(&item.name)));
        true
    }

    /// Enters a place as reached, now, if it has not been before. Whether it was new.
    pub fn reach(&mut self, name: &str, kind: &str, region: &str) -> bool {
        if name.is_empty() || self.places.iter().any(|v| v.name == name && v.kind == kind) {
            return false;
        }
        self.places.push(Visit { name: clean(name), kind: clean(kind), region: clean(region), when: self.now });
        let text = match kind {
            "region" | "range" | "rift" | "fault" => format!("Came into {name}."),
            "river" | "bay" | "inlet" => format!("Came to {name} {}.", self.now.time_of_day()),
            _ => format!("Reached {name} {}.", self.now.time_of_day()),
        };
        self.note(text);
        true
    }

    /// Looks round from continent-local `at` on continent `c` for the named places the
    /// player has come to, and for the name of where they stand. `afoot` unless gliding:
    /// a summit flown over has not been reached.
    pub fn survey(&mut self, c: &Continent, at: Vec2, afoot: bool) {
        let names = &c.trails.names;
        let region_of = |p: Vec2| names.region_at(p).and_then(|i| names.places.get(i)).map_or("", |p| p.name.as_str());
        let region = region_of(at);
        self.reach(region, "region", region);
        let mut beside: Option<(f32, &str)> = None;
        for p in &names.places {
            let d = p.distance(at);
            let centre = p.at.distance(at);
            let (reached, landmark) = match p.kind {
                PlaceKind::Region => continue,
                PlaceKind::Range | PlaceKind::Fault | PlaceKind::Rift => (d <= 0.0, false),
                PlaceKind::Island | PlaceKind::Bay | PlaceKind::Inlet => (centre <= p.radius * 0.7, false),
                PlaceKind::Lake => (d <= LAKE_REACH + p.radius * 0.1, true),
                PlaceKind::River => (d <= RIVER_REACH, true),
                PlaceKind::Peak => (afoot && (d <= 0.0 || centre <= NEAR), true),
                PlaceKind::Falls | PlaceKind::Tor | PlaceKind::Landmark(_) => (afoot && d <= NEAR, true),
                PlaceKind::Hamlet | PlaceKind::Village | PlaceKind::Town | PlaceKind::GreatTown => (afoot && d <= NEAR, true),
            };
            if landmark && d <= BESIDE && beside.is_none_or(|b| d < b.0) {
                beside = Some((d, &p.name));
            }
            if p.kind == PlaceKind::River && d <= BANK {
                self.follow(c, p, at);
            }
            if reached {
                self.reach(&p.name, p.kind.key(), region_of(p.at));
            }
        }
        // Out among a place's fields, the field the player stands in, as its people call
        // it (#64).
        let field = || worldgen::settlements::farmland::fieldnames::field_name_at(c, at);
        self.here = match beside {
            Some((_, name)) => format!("by {name}"),
            None => match field() {
                Some(name) => format!("in {name}"),
                None if !region.is_empty() => format!("in {region}"),
                None => String::new(),
            },
        };
    }

    /// Takes note of how far up river `p` the player, on its bank at `at`, is; and if
    /// they have come down a good way of it to where it meets the sea, of that.
    fn follow(&mut self, c: &Continent, p: &worldgen::names::Place, at: Vec2) {
        if p.line.len() < 2 {
            return;
        }
        let (up, length) = along(&p.line, at);
        let (up, length) = (up / VOXELS_PER_METRE, length / VOXELS_PER_METRE);
        let k = match self.rivers.iter().position(|r| r.name == p.name) {
            Some(k) => k,
            None => {
                self.rivers.push(River { name: clean(&p.name), far: up, followed: None });
                self.unsaved = true;
                self.rivers.len() - 1
            }
        };
        // Not every step up the bank is worth a save: a stride or two is.
        if up > self.rivers[k].far + 5.0 {
            self.rivers[k].far = up;
            self.unsaved = true;
        }
        let r = &self.rivers[k];
        if r.followed.is_none() && up <= MOUTH && r.far >= (length * 0.5).min(FOLLOWED) && by_sea(c, p.line[0]) {
            self.rivers[k].followed = Some(self.now);
            self.note(format!("Followed {} down to the sea.", p.name));
        }
    }

    /// The places reached, a region at a time in the order the regions were first come
    /// to: each region's name, the visit to the region itself if there was one, and the
    /// places in it.
    pub fn by_region(&self) -> Vec<(&str, Option<&Visit>, Vec<&Visit>)> {
        let mut out: Vec<(&str, Option<&Visit>, Vec<&Visit>)> = Vec::new();
        for v in &self.places {
            let k = match out.iter().position(|r| r.0 == v.region) {
                Some(k) => k,
                None => {
                    out.push((v.region.as_str(), None, Vec::new()));
                    out.len() - 1
                }
            };
            if v.kind == "region" {
                out[k].1 = Some(v);
            } else {
                out[k].2.push(v);
            }
        }
        out
    }

    /// The journal as a save holds it: a line to each thing written in it.
    pub fn to_text(&self) -> String {
        let mut out = String::from("version = 1\n");
        if let Some(date) = self.date {
            out += &format!("date = {date}\ndays = {}\nseasons = {}\nyears = {}\n", self.days, self.seasons, self.years);
        }
        out += &format!("walked = {}\nglided = {}\nsailed = {}\nswum = {}\n", self.walked, self.glided, self.sailed, self.swum);
        if let Some((height, at, when)) = &self.highest {
            out += &format!("highest = {height}|{}|{}\n", when.to_text(), clean(at));
        }
        if self.snow_year > 0 {
            out += &format!("snow = {}\n", self.snow_year);
        }
        for f in &self.finds {
            let [r, g, b] = f.item.color;
            let i = &f.item;
            out += &format!("find = {}|{}|{}|{r},{g},{b}|{}|{}|{}\n", f.when.to_text(), clean(&f.at), i.shape.word(), clean(&i.id), clean(&i.name), clean(&i.about));
        }
        for v in &self.places {
            out += &format!("place = {}|{}|{}|{}\n", v.when.to_text(), clean(&v.kind), clean(&v.region), clean(&v.name));
        }
        for r in &self.rivers {
            out += &format!("river = {}|{}|{}\n", r.far, r.followed.map_or_else(|| "-".into(), Stamp::to_text), clean(&r.name));
        }
        for n in &self.notes {
            out += &format!("note = {}|{}\n", n.when.to_text(), clean(&n.text));
        }
        for m in &self.met {
            out += &format!("met = {}|{}|{}\n", clean(&m.key), m.times, clean(&m.name));
        }
        for o in &self.offered {
            out += &format!("offered = {}|{}|{}|{}|{}\n", clean(&o.key), o.when.to_text(), clean(&o.town), clean(&o.name), clean(&o.text));
        }
        for c in &self.charted {
            out += &format!("charted = {}|{},{}|{}|{}\n", clean(&c.key), c.at.x, c.at.y, c.reach, clean(&c.town));
        }
        for s in &self.shrines {
            let told = s.told.map_or_else(|| "-".into(), |t| format!("{},{}", t.x, t.y));
            out += &format!("shrine = {},{}|{}|{told}|{}|{}\n", s.at.x, s.at.y, s.times, s.when.to_text(), clean(&s.name));
        }
        out
    }

    /// Reads a journal back, leaving out any line that does not make sense.
    pub fn from_text(text: &str) -> Self {
        let mut j = Self::new();
        let number = |v: &str| v.parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0);
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            let (key, value) = (key.trim(), value.trim());
            match key {
                "date" => j.date = number(value).map(|v| v.rem_euclid(1.0) as f32),
                "days" => j.days = number(value).unwrap_or(0.0),
                "seasons" => j.seasons = value.parse().unwrap_or(0),
                "years" => j.years = value.parse().unwrap_or(0),
                "walked" => j.walked = number(value).unwrap_or(0.0),
                "glided" => j.glided = number(value).unwrap_or(0.0),
                "sailed" => j.sailed = number(value).unwrap_or(0.0),
                "swum" => j.swum = number(value).unwrap_or(0.0),
                "snow" => j.snow_year = value.parse().unwrap_or(0),
                "highest" => {
                    let parts: Vec<&str> = value.splitn(3, '|').collect();
                    if let [height, when, at] = parts[..]
                        && let (Some(height), Some(when)) = (number(height.trim()), Stamp::from_text(when))
                    {
                        j.highest = Some((height as f32, at.trim().into(), when));
                    }
                }
                "find" => {
                    let parts: Vec<&str> = value.splitn(7, '|').collect();
                    let find = (|| {
                        let [when, at, shape, color, id, name, about] = parts[..] else { return None };
                        let rgb: Vec<u8> = color.split(',').filter_map(|v| v.trim().parse().ok()).collect();
                        let item = Item::new(id.trim(), name.trim(), about.trim(), <[u8; 3]>::try_from(rgb).ok()?, Shape::from_word(shape.trim())?);
                        (!item.id.is_empty()).then_some(Find { item, at: at.trim().into(), when: Stamp::from_text(when)? })
                    })();
                    if let Some(f) = find
                        && j.find(&f.item.id).is_none()
                    {
                        j.finds.push(f);
                    }
                }
                "place" => {
                    let parts: Vec<&str> = value.splitn(4, '|').collect();
                    if let [when, kind, region, name] = parts[..]
                        && let Some(when) = Stamp::from_text(when)
                        && !name.trim().is_empty()
                        && !j.places.iter().any(|v| v.name == name.trim() && v.kind == kind.trim())
                    {
                        j.places.push(Visit { name: name.trim().into(), kind: kind.trim().into(), region: region.trim().into(), when });
                    }
                }
                "river" => {
                    let parts: Vec<&str> = value.splitn(3, '|').collect();
                    if let [far, followed, name] = parts[..]
                        && let Some(far) = number(far.trim())
                        && !name.trim().is_empty()
                    {
                        j.rivers.push(River { name: name.trim().into(), far: far as f32, followed: Stamp::from_text(followed) });
                    }
                }
                "met" => {
                    let parts: Vec<&str> = value.splitn(3, '|').collect();
                    if let [key, times, name] = parts[..]
                        && let Ok(times) = times.trim().parse::<u32>()
                        && !key.trim().is_empty()
                        && !j.met.iter().any(|m| m.key == key.trim())
                    {
                        j.met.push(Met { key: key.trim().into(), name: name.trim().into(), times });
                    }
                }
                "offered" => {
                    let parts: Vec<&str> = value.splitn(5, '|').collect();
                    if let [key, when, town, name, text] = parts[..]
                        && let Some(when) = Stamp::from_text(when)
                        && !key.trim().is_empty()
                        && !j.offered.iter().any(|o| o.key == key.trim())
                    {
                        j.offered.push(Offer { key: key.trim().into(), name: name.trim().into(), town: town.trim().into(), text: text.trim().into(), when });
                    }
                }
                "charted" => {
                    let parts: Vec<&str> = value.splitn(4, '|').collect();
                    if let [key, at, reach, town] = parts[..]
                        && let Some((x, z)) = at.split_once(',')
                        && let (Ok(x), Ok(z), Some(reach)) = (x.trim().parse::<f64>(), z.trim().parse::<f64>(), number(reach.trim()))
                        && !key.trim().is_empty()
                        && !j.charted.iter().any(|c| c.key == key.trim())
                    {
                        j.charted.push(Chart { key: key.trim().into(), town: town.trim().into(), at: DVec2::new(x, z), reach });
                    }
                }
                "shrine" => {
                    let parts: Vec<&str> = value.splitn(5, '|').collect();
                    let place = |v: &str| v.split_once(',').and_then(|(x, z)| Some(DVec2::new(x.trim().parse().ok()?, z.trim().parse().ok()?))).filter(|p| p.is_finite());
                    if let [at, times, told, when, name] = parts[..]
                        && let (Some(at), Ok(times), Some(when)) = (place(at), times.trim().parse::<u32>(), Stamp::from_text(when))
                        && !j.shrines.iter().any(|s| s.at == at)
                    {
                        j.shrines.push(Shrine { at, name: name.trim().into(), times: times.max(1), told: place(told), when });
                    }
                }
                "note" => {
                    if let Some((when, text)) = value.split_once('|')
                        && let Some(when) = Stamp::from_text(when)
                        && j.notes.len() < NOTES
                    {
                        j.notes.push_back(Note { when, text: text.trim().into() });
                    }
                }
                _ => {}
            }
        }
        j
    }

    /// A journal as it might stand after a walk of some weeks round continent-local `at`
    /// on continent `c`, to `count` of its nearest places, for screenshots.
    /// It is written in the world's `year` (from 0).
    pub fn sample(c: &Continent, at: Vec2, count: usize, catalogue: &[Item], year: u32) -> Self {
        let mut j = Self::new();
        j.tick(0.0, 0.04, year, 7.5, false);
        if count == 0 {
            return j;
        }
        let names = &c.trails.names;
        let mut near: Vec<&worldgen::names::Place> = names
            .places
            .iter()
            .filter(|p| !matches!(p.kind, PlaceKind::Region | PlaceKind::Range | PlaceKind::Fault | PlaceKind::Rift | PlaceKind::Island))
            .collect();
        near.sort_by(|a, b| a.distance(at).total_cmp(&b.distance(at)));
        near.truncate(count);
        let (cs, n) = (c.cell_size(), c.size);
        let ground = |p: Vec2| {
            let cell = |v: f32| ((v / cs).max(0.0) as usize).min(n - 1);
            c.height[cell(p.y) * n + cell(p.x)] * OLD_VOXEL
        };
        // Things are found in the order the country offers them, more or less.
        let order = [10, 12, 3, 13, 14, 2, 15, 30, 11, 5, 6, 18, 19, 8, 27, 16, 28, 9, 17, 29, 20];
        let mut finds = order.iter().filter_map(|k| catalogue.get(*k));
        let mut last = at;
        for (k, p) in near.iter().enumerate() {
            // A day or so between one place and the next, and the hour moving round.
            let stride = 0.9 + 0.17 * (k % 5) as f32;
            j.tick(stride, j.now.date + stride / 24.0, year, j.now.hour + stride * 24.0, false);
            j.tick(0.0, j.now.date, year, j.now.hour, j.now.quarter() == 3);
            let region = names.region_at(p.at).and_then(|i| names.places.get(i)).map_or("", |r| r.name.as_str());
            j.reach(region, "region", region);
            j.here = format!("by {}", p.name);
            j.reach(&p.name, p.kind.key(), region);
            let gone = p.at.distance(last) as f64 / VOXELS_PER_METRE as f64 * 1.3;
            match p.kind {
                PlaceKind::Lake | PlaceKind::Bay | PlaceKind::Inlet => (j.walked, j.sailed) = (j.walked + gone * 0.7, j.sailed + gone * 0.3),
                PlaceKind::Peak => (j.walked, j.glided) = (j.walked + gone * 0.8, j.glided + gone * 0.2),
                _ => j.walked += gone,
            }
            last = p.at;
            let height = ground(p.at) / VOXELS_PER_METRE;
            if p.kind != PlaceKind::River && height > j.highest.as_ref().map_or(0.0, |h| h.0) {
                j.highest = Some((height, j.here.clone(), j.now));
            }
            if p.kind == PlaceKind::River && p.line.len() > 1 && !j.rivers.iter().any(|r| r.followed.is_some()) {
                let length = along(&p.line, p.at).1 / VOXELS_PER_METRE;
                j.rivers.push(River { name: p.name.clone(), far: length, followed: Some(j.now) });
                j.note(format!("Followed {} down to the sea.", p.name));
            }
            for _ in 0..1 + (k % 3 == 0) as usize {
                if let Some(item) = finds.next() {
                    j.found(item);
                }
            }
        }
        j.swum = j.walked * 0.012;
        j
    }
}

/// Everything there is to find, in the order the journal shows it: the flowers and
/// mushrooms, then what the bushes and the reed beds bear, then the resin of the trees
/// and the incense made of it (#40).
pub fn catalogue(materials: &voxel::Materials) -> Vec<Item> {
    let flowers = Flower::ALL.iter().map(|f| inventory::of_flower(*f));
    let borne = [mat::GORSE_BLOOM, mat::HOLLY_BERRY, mat::IVY_BERRY, mat::REED_HEAD].into_iter().filter_map(|v| inventory::of_voxel(v, materials));
    flowers.chain(borne).chain([inventory::resin(), inventory::incense()]).collect()
}

/// The months the thing called `id` is to be found in, if it is a flower or a mushroom.
pub fn months(id: &str) -> Option<String> {
    use worldgen::features::plants::MONTHS;
    let f = Flower::ALL.iter().find(|f| f.id() == id)?;
    let word = if f.is_mushroom() { "Up" } else { "In flower" };
    let (first, last) = f.months();
    Some(if f.all_year() {
        format!("{word} all the year round.")
    } else {
        format!("{word} from {} to {}.", MONTHS[first as usize - 1], MONTHS[last as usize - 1])
    })
}

/// The kind of country the thing called `id` grows in, if that is known.
pub fn country(id: &str) -> Option<&'static str> {
    if let Some(f) = Flower::ALL.iter().find(|f| f.id() == id) {
        return Some(match f {
            Flower::Pimpernel => "Open ground almost anywhere, in drifts of red.",
            Flower::Cinquefoil => "Open ground and grassland, in drifts of yellow.",
            Flower::Stitchwort => "Open ground and grassland, in drifts of white.",
            Flower::Violet => "Open ground in most country, in drifts of purple.",
            Flower::Speedwell => "Open ground outside the woods, in drifts of blue.",
            Flower::Campion => "Open ground in most country, in drifts of pink.",
            Flower::Heather => "The heath and the open moor.",
            Flower::WaterLily => "Still water, a little way out from a lake's shore.",
            Flower::FlyAgaric => "The floor of old forest, taiga and rainforest, and birch groves.",
            Flower::PennyBun => "The floor of old forest, taiga and rainforest, and broadleaf woods.",
            Flower::Chanterelle => "In patches in the moss of woods and forest, out of the tropics.",
            Flower::StGeorge => "Grassland and steppe, and broadleaf woods.",
            Flower::Bluebell => "Broadleaf woods and birch groves, in carpets.",
            Flower::Foxglove => "The edges of woods, and the moor.",
            Flower::Primrose => "Broadleaf woods.",
            Flower::Daisy | Flower::Buttercup | Flower::Poppy => "The grassland.",
            Flower::Lupin => "Grassland and the alpine meadows.",
            Flower::CottonGrass => "The bog and the wet moor.",
            Flower::MarshMarigold | Flower::FlagIris => "The marsh, and the banks of streams in mild country.",
            Flower::Thrift => "Dunes and shingle by the sea, out of the tropics.",
            Flower::Gentian => "The alpine meadows.",
            Flower::Edelweiss => "Alone, here and there, on alpine meadow and tundra.",
            Flower::Orchid | Flower::Heliconia => "The rainforest.",
            Flower::DesertMarigold => "The desert and the red rock country.",
            Flower::Thistle => "The steppe.",
        });
    }
    Some(match id {
        "gorse-blossom" => "Among the thorns of a gorse bush, wherever gorse has taken hold.",
        "holly-berries" => "On the holly trees, when the berries are red.",
        "ivy-berries" => "On old ivy, up trunks and walls, in winter.",
        "reed-mace" => "The reed beds along rivers and the shores of lakes.",
        "resin" => "Here and there on the bark of pines and junipers, acacias, and the great trees of the rainforest.",
        "incense" => "Made, not found: resin rolled with violets, heather, gorse blossom, marigolds or orchids.",
        _ => return None,
    })
}

/// A page of the journal.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Page {
    #[default]
    Finds,
    Places,
    Walk,
    Days,
}

impl Page {
    /// Each page with its word on the command line and its title.
    pub const ALL: [(Page, &'static str, &'static str); 4] =
        [(Page::Finds, "finds", "Finds"), (Page::Places, "places", "Places"), (Page::Walk, "walk", "The walk"), (Page::Days, "days", "Days")];

    pub fn named(word: &str) -> Option<Self> {
        Self::ALL.iter().find(|p| p.1 == word).map(|p| p.0)
    }
}

/// What the journal's window remembers from frame to frame.
pub struct View {
    pub page: Page,
    /// The find being read about, by its id.
    selected: Option<String>,
    catalogue: Vec<Item>,
    /// Whose journal it is, if the player has given their name on the figure screen
    /// (#204): it goes on the cover.
    pub owner: String,
}

impl View {
    pub fn new(materials: &voxel::Materials) -> Self {
        Self { page: Page::default(), selected: None, catalogue: catalogue(materials), owner: String::new() }
    }

    /// What the journal is headed: the journal, or the journal of whoever keeps it.
    pub fn heading(&self) -> String {
        match self.owner.trim() {
            "" => "Journal".into(),
            name => format!("The journal of {name}"),
        }
    }

    /// Everything there is to find.
    pub fn catalogue(&self) -> &[Item] {
        &self.catalogue
    }
}

const PANEL: egui::Color32 = egui::Color32::from_rgba_premultiplied(22, 27, 34, 238);
const SLOT: egui::Color32 = egui::Color32::from_rgb(44, 52, 62);
const SLOT_LIT: egui::Color32 = egui::Color32::from_rgb(60, 70, 82);
const UNKNOWN: egui::Color32 = egui::Color32::from_rgb(31, 37, 45);
const SHADE: egui::Color32 = egui::Color32::from_rgb(58, 67, 78);
const EDGE: egui::Color32 = egui::Color32::from_rgb(86, 98, 112);
const PICKED: egui::Color32 = egui::Color32::from_rgb(236, 214, 150);
const INK: egui::Color32 = egui::Color32::from_rgb(240, 236, 226);
const FAINT: egui::Color32 = egui::Color32::from_rgb(160, 168, 176);

/// The window's width inside its margins, and the height of a page.
const WIDTH: f32 = 644.0;
const HEIGHT: f32 = 372.0;
/// Finds across the page, their size and the gap between them.
const ACROSS: usize = 11;
const SIDE: f32 = 52.0;
const GAP: f32 = 7.2;

fn text(s: impl Into<String>, size: f32, color: egui::Color32) -> egui::RichText {
    egui::RichText::new(s).size(size).color(color)
}

/// The outline of a thing not yet found.
fn silhouette(painter: &egui::Painter, rect: egui::Rect, shape: Shape) {
    let side = (rect.width().min(rect.height()) / 8.0).floor().max(1.0);
    let corner = rect.center() - egui::vec2(side * 4.0, side * 4.0);
    for (row, line) in shape.picture().iter().enumerate() {
        for (col, ch) in line.chars().enumerate() {
            if ch != '.' {
                let at = corner + egui::vec2(col as f32 * side, row as f32 * side);
                painter.rect_filled(egui::Rect::from_min_size(at, egui::vec2(side, side)), 0.0, SHADE);
            }
        }
    }
}

/// A distance in metres, as it is said.
fn distance(metres: f64) -> String {
    match metres {
        d if d < 0.5 => "\u{2014}".into(),
        d if d < 1000.0 => format!("{d:.0} m"),
        d => format!("{:.1} km", d / 1000.0),
    }
}

/// What a kind of place is called.
fn kind_word(kind: &str) -> &str {
    match kind {
        "falls" => "waterfall",
        "stones" => "standing stones",
        "range" => "mountains",
        "arch" => "rock arch",
        other => other,
    }
}

fn finds_page(ui: &mut egui::Ui, journal: &Journal, view: &mut View) {
    // Things found that this game knows nothing of (from a newer one) still have a place.
    let mut all: Vec<&Item> = view.catalogue.iter().collect();
    all.extend(journal.finds().iter().map(|f| &f.item).filter(|i| !view.catalogue.iter().any(|c| c.id == i.id)));
    let found = journal.finds().len();
    let selected = view.selected.clone().or_else(|| journal.finds().last().map(|f| f.item.id.clone()));
    let mut pick = None;
    egui::Grid::new("journal finds").spacing([GAP, GAP]).show(ui, |ui| {
        for (k, item) in all.iter().enumerate() {
            let (rect, response) = ui.allocate_exact_size(egui::vec2(SIDE, SIDE), egui::Sense::click());
            let painter = ui.painter();
            let known = journal.find(&item.id).is_some();
            painter.rect_filled(rect, 7.0, if !known { UNKNOWN } else if response.hovered() { SLOT_LIT } else { SLOT });
            let chosen = selected.as_deref() == Some(item.id.as_str());
            let edge = if chosen { egui::Stroke::new(2.0, PICKED) } else { egui::Stroke::new(1.0, if known { EDGE } else { SHADE }) };
            painter.rect_stroke(rect, 7.0, edge, egui::StrokeKind::Inside);
            if known {
                inventory::picture(painter, rect.shrink(8.0), item, 1.0);
            } else {
                silhouette(painter, rect.shrink(8.0), item.shape);
            }
            if response.clicked() {
                pick = Some(item.id.clone());
            }
            if known {
                response.on_hover_text(&item.name);
            }
            if k % ACROSS == ACROSS - 1 {
                ui.end_row();
            }
        }
    });
    if pick.is_some() {
        view.selected = pick;
    }
    ui.add_space(14.0);
    let chosen = selected.as_deref().and_then(|id| all.iter().find(|i| i.id == id));
    match chosen.map(|item| (item, journal.find(&item.id))) {
        Some((_, Some(find))) => {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(64.0, 64.0), egui::Sense::hover());
                inventory::picture(ui.painter(), rect, &find.item, 1.0);
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 5.0;
                    ui.label(text(&find.item.name, 18.0, INK));
                    ui.label(text(&find.item.about, 14.0, INK));
                    let at = if find.at.is_empty() { String::new() } else { format!(" {},", find.at) };
                    ui.label(text(format!("First found{at} in {}.", find.when.when()), 13.0, FAINT));
                    if let Some(country) = country(&find.item.id) {
                        ui.label(text(format!("Where it grows: {}", lower(country)), 13.0, FAINT));
                    }
                    if let Some(months) = months(&find.item.id) {
                        ui.label(text(months, 13.0, FAINT));
                    }
                });
            });
        }
        Some((item, None)) => {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(64.0, 64.0), egui::Sense::hover());
                silhouette(ui.painter(), rect, item.shape);
                ui.add_space(8.0);
                ui.label(text("Not found yet. It is out there somewhere.", 14.0, FAINT));
            });
        }
        None => {
            ui.label(text("Nothing found yet. The first of each kind of thing you gather is written in here.", 14.0, FAINT));
        }
    }
    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
        ui.label(text(format!("{found} of {} kinds found", all.len()), 13.0, FAINT));
    });
}

fn places_page(ui: &mut egui::Ui, journal: &Journal) {
    if journal.places().is_empty() {
        ui.label(text("No named places reached yet. Each is written in here when you first come to it.", 14.0, FAINT));
        return;
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        for (k, (region, came, places)) in journal.by_region().into_iter().enumerate() {
            if k > 0 {
                ui.add_space(10.0);
            }
            ui.horizontal(|ui| {
                ui.label(text(if region.is_empty() { "Elsewhere" } else { region }, 17.0, PICKED));
                if let Some(v) = came {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(10.0);
                        ui.label(text(format!("first walked in {}", v.when.when()), 13.0, FAINT));
                    });
                }
            });
            for v in places {
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    ui.label(text(&v.name, 14.5, INK));
                    ui.label(text(kind_word(&v.kind), 13.0, FAINT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(10.0);
                        ui.label(text(v.when.when(), 13.0, FAINT));
                    });
                });
            }
        }
    });
}

fn walk_page(ui: &mut egui::Ui, journal: &Journal, kinds: usize) {
    let now = journal.now;
    ui.label(text(format!("Day {}, {} in {}.", journal.day(), now.time_of_day(), now.when()), 15.0, INK));
    ui.add_space(14.0);
    let plural = |n: u32, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let mut rows: Vec<(&str, String)> = Vec::new();
    let seasons = journal.seasons();
    let turned = if seasons == 0 { String::new() } else { format!(", through {}", plural(seasons, "turn of the season", "turns of the season")) };
    rows.push(("On the road", format!("{}{turned}", plural(journal.day(), "day", "days"))));
    rows.push(("Walked", distance(journal.walked)));
    rows.push(("Glided", distance(journal.glided)));
    rows.push(("Sailed", distance(journal.sailed)));
    rows.push(("Swum", distance(journal.swum)));
    rows.push(("Highest ground stood on", match &journal.highest {
        Some((height, at, _)) if at.is_empty() => format!("{height:.0} m above the sea"),
        Some((height, at, _)) => format!("{height:.0} m above the sea, {at}"),
        None => "\u{2014}".into(),
    }));
    let regions = journal.places().iter().filter(|v| v.kind == "region").count() as u32;
    let places = journal.places().len() as u32 - regions;
    rows.push(("Named places reached", match (places, regions) {
        (0, 0) => "\u{2014}".into(),
        (p, 0) => p.to_string(),
        (p, r) => format!("{p}, in {}", plural(r, "region", "regions")),
    }));
    rows.push(("Kinds of thing found", format!("{} of {kinds}", journal.finds().len())));
    let followed: Vec<&str> = journal.rivers().iter().filter(|r| r.followed.is_some()).map(|r| r.name.as_str()).collect();
    rows.push(("Rivers followed to the sea", match followed.len() {
        0 => "\u{2014}".into(),
        1..=3 => followed.join(", "),
        n => format!("{}, and {} more", followed[..3].join(", "), n - 3),
    }));
    // What the settlements' boards have asked (#207), for errands to come (#46).
    rows.push(("Asked on the boards", match journal.offered() {
        [] => "\u{2014}".into(),
        all @ [.., last] => format!("{}, the latest by {} of {}", all.len(), last.name, last.town),
    }));
    // Where incense has been lit (#40).
    rows.push(("Incense lit at", match journal.shrines() {
        [] => "\u{2014}".into(),
        [one] => one.name.clone(),
        all @ [.., last] => format!("{}, the latest {}", plural(all.len() as u32, "shrine", "shrines"), last.name),
    }));
    ui.spacing_mut().item_spacing.y = 9.0;
    for (what, value) in rows {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(214.0, 20.0), egui::Sense::hover());
            ui.painter().text(rect.left_center(), egui::Align2::LEFT_CENTER, what, egui::FontId::proportional(14.5), FAINT);
            ui.add(egui::Label::new(text(value, 14.5, INK)).truncate());
        });
    }
}

fn days_page(ui: &mut egui::Ui, journal: &Journal) {
    if journal.notes.is_empty() {
        ui.label(text("Nothing written yet. What happens on the way is set down here, a day at a time.", 14.0, FAINT));
        return;
    }
    // The latest day first; within a day, in the order things happened.
    let mut days: Vec<Vec<&Note>> = Vec::new();
    for n in journal.notes() {
        match days.last_mut() {
            Some(day) if day[0].when.day == n.when.day && day[0].when.year == n.when.year => day.push(n),
            _ => days.push(vec![n]),
        }
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        for (k, day) in days.iter().rev().enumerate() {
            if k > 0 {
                ui.add_space(10.0);
            }
            let when = day[0].when;
            ui.horizontal(|ui| {
                ui.label(text(format!("Day {}", when.day), 16.0, PICKED));
                ui.label(text(when.when(), 13.0, FAINT));
            });
            for n in day {
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    ui.add(egui::Label::new(text(&n.text, 14.5, INK)).wrap());
                });
            }
        }
    });
}

/// Shows the journal's window.
pub fn show(ctx: &egui::Context, journal: &Journal, view: &mut View) {
    // The arrow keys turn the pages.
    let turn = ctx.input(|i| i.key_pressed(egui::Key::ArrowRight) as i32 - i.key_pressed(egui::Key::ArrowLeft) as i32);
    if turn != 0 {
        let at = Page::ALL.iter().position(|p| p.0 == view.page).unwrap_or(0) as i32;
        view.page = Page::ALL[(at + turn).rem_euclid(Page::ALL.len() as i32) as usize].0;
    }
    egui::Area::new(egui::Id::unique("journal")).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).fade_in(false).show(ctx, |ui| {
        egui::Frame::new().fill(PANEL).stroke(egui::Stroke::new(1.0, EDGE)).corner_radius(10).inner_margin(egui::Margin::same(18)).show(ui, |ui| {
            ui.set_width(WIDTH);
            ui.horizontal(|ui| {
                ui.label(text(&view.heading(), 22.0, INK));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 18.0;
                    for (page, _, title) in Page::ALL.iter().rev() {
                        let color = if *page == view.page { PICKED } else { FAINT };
                        if ui.add(egui::Label::new(text(*title, 15.0, color)).sense(egui::Sense::click())).clicked() {
                            view.page = *page;
                        }
                    }
                });
            });
            ui.add_space(12.0);
            let kinds = view.catalogue.len().max(journal.finds().len());
            ui.allocate_ui_with_layout(egui::vec2(WIDTH, HEIGHT), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_min_size(egui::vec2(WIDTH, HEIGHT));
                ui.set_max_height(HEIGHT);
                match view.page {
                    Page::Finds => finds_page(ui, journal, view),
                    Page::Places => places_page(ui, journal),
                    Page::Walk => walk_page(ui, journal, kinds),
                    Page::Days => days_page(ui, journal),
                }
            });
            ui.add_space(8.0);
            ui.label(text("J or Esc to close.  Left and right turn the page.", 12.0, FAINT));
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thing(id: &str, name: &str) -> Item {
        Item::new(id, name, "About it.", [200, 100, 50], Shape::Flower)
    }

    fn world() -> Arc<Continent> {
        worldgen::world::GameWorld::with_config(3, worldgen::continent::SimConfig { size: 256 }).continent(0, 0)
    }

    #[test]
    fn the_first_of_a_kind_is_written_down_once_with_where_and_when() {
        let mut j = Journal::new();
        j.tick(0.0, 0.8, 0, 19.0, false);
        j.here = "by Carn Hollow".into();
        assert!(j.found(&thing("poppy", "Poppy")));
        assert!(!j.found(&thing("poppy", "Poppy")));
        j.tick(30.0, 0.05, 1, 9.0, false);
        j.here.clear();
        assert!(j.found(&thing("holly-berries", "Holly berries")));
        assert!(!j.found(&thing("poppy", "Poppy")));
        let poppy = j.find("poppy").expect("the poppy");
        assert_eq!((poppy.at.as_str(), poppy.when.day, poppy.when.year), ("by Carn Hollow", 1, 1));
        assert_eq!(poppy.when.when(), "early winter of the first year");
        let holly = j.find("holly-berries").expect("the holly");
        assert_eq!((holly.at.as_str(), holly.when.day, holly.when.year), ("", 31, 2));
        assert_eq!(j.finds().len(), 2);
        let notes: Vec<&str> = j.notes().map(|n| n.text.as_str()).collect();
        assert!(notes.contains(&"The first poppy, by Carn Hollow.") && notes.contains(&"The first holly berries."), "{notes:?}");
        assert!(j.unsaved);
    }

    #[test]
    fn a_journal_reads_back_as_it_was_saved_and_shrugs_off_nonsense() {
        let c = world();
        let centre = Vec2::splat(c.size as f32 * c.cell_size() * 0.5);
        let mats = mat::build();
        let j = Journal::sample(&c, centre, 12, &catalogue(&mats), 2);
        assert!(j.finds().len() >= 10 && j.places().len() >= 12 && j.notes().count() > 20 && j.walked > 100.0);
        // Written in the world's year 2, its third.
        assert!(j.finds().iter().all(|f| f.when.year == 3 && f.when.when().ends_with("of the third year")));
        let text = j.to_text();
        let back = Journal::from_text(&text);
        assert!(!back.unsaved);
        assert_eq!(back.year(), Some(2));
        assert_eq!(back.to_text(), text);
        assert_eq!((back.finds(), back.places(), back.rivers()), (j.finds(), j.places(), j.rivers()));
        assert_eq!((back.day(), back.seasons(), back.highest.clone()), (j.day(), j.seasons(), j.highest.clone()));
        // Lines that make no sense are left out, and the rest still reads; a thing or a
        // place written twice is entered once.
        let first_find = text.lines().find(|l| l.starts_with("find")).expect("a find");
        let noisy = format!(
            "rubbish\nfind = soon|x|flower|1,2,3|a|A|b\nfind = 0.1,2,1,1|x|wheel|1,2,3|a|A|b\nplace = never|lake|R|N\nnote = |x\nwalked = lots\nfuture = 7\n{text}{first_find}\nplace = 0.5,3,2,1|lake|Somewhere|Llyn Newydd\n"
        );
        let back = Journal::from_text(&noisy);
        assert_eq!(back.finds(), j.finds());
        assert_eq!(back.places().len(), j.places().len() + 1);
        assert_eq!(back.walked, j.walked);
        // Bars and line ends in what is written cannot break the save.
        let mut odd = Journal::new();
        odd.tick(0.0, 0.1, 0, 12.0, false);
        odd.here = "by the | Ford".into();
        odd.found(&Item::new("x", "A | thing", "One\nline.", [1, 2, 3], Shape::Reed));
        odd.reach("Bar | Tor", "tor", "The\rMoor");
        let back = Journal::from_text(&odd.to_text());
        assert_eq!((back.finds().len(), back.places().len()), (1, 1));
        assert_eq!((back.finds()[0].item.shape, back.places()[0].name.as_str()), (Shape::Reed, "Bar   Tor"));
        // No save at all is a new journal.
        let new = Journal::from_text("");
        assert!(new.finds().is_empty() && new.places().is_empty() && new.day() == 1 && !new.unsaved);
    }

    #[test]
    fn a_shrine_incense_is_lit_at_is_kept_with_what_its_smoke_told_of() {
        let mut j = Journal::new();
        j.tick(0.0, 0.3, 0, 19.5, false);
        j.here = "by Carn Hollow".into();
        let shrine = DVec2::new(100.0, 100.0);
        j.light(shrine, false, Some("Tres Sanctum"), Some((DVec2::new(100.0, -900.0), false)));
        // Lit again a step away, at the same shrine: counted, not entered twice, and
        // what it told of first is kept.
        j.light(shrine + DVec2::new(2.0, 0.0), false, Some("Tres Sanctum"), Some((DVec2::new(1100.0, 100.0), true)));
        j.light(DVec2::new(5000.0, 0.0), true, None, None);
        assert_eq!(j.shrines().len(), 2);
        assert_eq!((j.shrines()[0].name.as_str(), j.shrines()[0].times), ("Tres Sanctum", 2));
        assert_eq!(j.shrines()[1].name, "a wayside shrine by Carn Hollow");
        assert_eq!(j.told(), vec![DVec2::new(100.0, -900.0)]);
        let notes: Vec<&str> = j.notes().map(|n| n.text.as_str()).collect();
        assert!(notes.contains(&"Lit incense at Tres Sanctum at dusk. Its smoke drifted away north, toward a spring."), "{notes:?}");
        assert!(notes.iter().any(|n| n.starts_with("Lit incense again at Tres Sanctum") && n.ends_with("east, toward another shrine.")), "{notes:?}");
        let back = Journal::from_text(&j.to_text());
        assert_eq!(back.shrines(), j.shrines());
        assert_eq!(back.to_text(), j.to_text());
        assert_eq!(compass(DVec2::ZERO, DVec2::new(-10.0, 10.0)), "south-west");
    }

    #[test]
    fn places_are_entered_when_first_come_to_and_grouped_by_region() {
        let c = world();
        let names = &c.trails.names;
        let mut j = Journal::new();
        j.tick(0.0, 0.3, 0, 10.0, false);
        let small = |p: &&worldgen::names::Place| matches!(p.kind, PlaceKind::Landmark(_) | PlaceKind::Falls | PlaceKind::Lake);
        let places: Vec<&worldgen::names::Place> = names.places.iter().filter(small).take(6).collect();
        assert!(places.len() >= 3, "the test world has few places: {}", names.places.len());
        for p in &places {
            // Half a kilometre off it is not reached; standing at it, it is.
            j.survey(&c, p.at + Vec2::new(m(500.0) + p.radius * 1.2, 0.0), true);
            assert!(!j.places().iter().any(|v| v.name == p.name && v.kind == p.kind.key()), "{} from afar", p.name);
            j.survey(&c, p.at, true);
            let v = j.places().iter().find(|v| v.name == p.name).unwrap_or_else(|| panic!("{} not reached", p.name));
            assert_eq!(v.kind, p.kind.key());
            assert!(j.here.starts_with("by "), "{}", j.here);
        }
        // Coming back adds nothing.
        let (before, notes) = (j.places().len(), j.notes().count());
        for p in &places {
            j.survey(&c, p.at, true);
        }
        assert_eq!((j.places().len(), j.notes().count()), (before, notes));
        // Every place is under a region, and each region is there once.
        let regions = j.by_region();
        assert_eq!(regions.iter().map(|r| r.2.len()).sum::<usize>() + regions.iter().filter(|r| r.1.is_some()).count(), before);
        let mut seen: Vec<&str> = regions.iter().map(|r| r.0).collect();
        seen.dedup();
        assert_eq!(seen.len(), regions.len());
        for p in &places {
            let region = &j.places().iter().find(|v| v.name == p.name).expect("reached").region;
            assert!(regions.iter().any(|r| r.0 == region && r.1.is_some() && r.2.iter().any(|v| v.name == p.name)), "{} in {region}", p.name);
        }
        // A summit glided over has not been stood on.
        if let Some(peak) = names.places.iter().find(|p| p.kind == PlaceKind::Peak) {
            let mut j = Journal::new();
            j.survey(&c, peak.at, false);
            assert!(!j.places().iter().any(|v| v.kind == "peak"));
            j.survey(&c, peak.at, true);
            assert!(j.places().iter().any(|v| v.name == peak.name));
        }
    }

    /// A river or a lake is known by name from the hillside above it, a good way off, as
    /// a small place is not.
    #[test]
    fn rivers_and_lakes_are_come_to_from_well_off() {
        let c = world();
        let names = &c.trails.names;
        let mut tried = 0;
        for p in names.places.iter().filter(|p| matches!(p.kind, PlaceKind::River | PlaceKind::Lake)) {
            let reach = if p.kind == PlaceKind::Lake { LAKE_REACH + p.radius * 0.1 } else { RIVER_REACH };
            // Away from it, each way, until a point is the distance wanted from it.
            let away = |want: f32| {
                let from = p.line.get(p.line.len() / 2).copied().unwrap_or(p.at);
                (0..8).find_map(|k| {
                    let dir = Vec2::from_angle(k as f32 * 0.785);
                    let at = (1..400).map(|s| from + dir * s as f32 * m(2.0)).find(|q| p.distance(*q) >= want)?;
                    (p.distance(at) < want + m(5.0)).then_some(at)
                })
            };
            let (Some(near), Some(far)) = (away(reach - m(10.0)), away(reach + m(60.0))) else { continue };
            assert!(p.distance(near) > m(45.0), "{}", p.name);
            let mut j = Journal::new();
            j.survey(&c, far, true);
            assert!(!j.places().iter().any(|v| v.name == p.name), "{} from {} m", p.name, p.distance(far) / VOXELS_PER_METRE);
            j.survey(&c, near, true);
            assert!(j.places().iter().any(|v| v.name == p.name), "{} not from {} m", p.name, p.distance(near) / VOXELS_PER_METRE);
            tried += 1;
        }
        assert!(tried >= 2, "only {tried} rivers and lakes to try");
    }

    #[test]
    fn a_river_walked_down_to_its_mouth_is_followed_to_the_sea() {
        // Along a bent line: how far, and of how long.
        let line = [Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0), Vec2::new(100.0, 50.0)];
        assert_eq!(along(&line, Vec2::new(40.0, 7.0)), (40.0, 150.0));
        assert_eq!(along(&line, Vec2::new(120.0, 30.0)), (130.0, 150.0));
        let c = world();
        let rivers: Vec<&worldgen::names::Place> = c.trails.names.places.iter().filter(|p| p.kind == PlaceKind::River && p.line.len() > 1).collect();
        assert!(!rivers.is_empty(), "the test world has no rivers");
        let mut to_sea = 0;
        for p in rivers {
            let mut j = Journal::new();
            j.tick(0.0, 0.3, 0, 10.0, false);
            // Up from the mouth is not down to it.
            for at in &p.line {
                j.survey(&c, *at, true);
            }
            assert!(j.rivers().iter().all(|r| r.followed.is_none()), "{} followed upstream", p.name);
            let walk = j.rivers().iter().find(|r| r.name == p.name).expect("the river walked");
            assert!(walk.far > 100.0, "{} walked {} m up", p.name, walk.far);
            for at in p.line.iter().rev() {
                j.survey(&c, *at, true);
            }
            let followed = j.rivers().iter().find(|r| r.name == p.name).is_some_and(|r| r.followed.is_some());
            assert_eq!(followed, by_sea(&c, p.line[0]), "{}", p.name);
            to_sea += followed as usize;
            assert_eq!(j.notes().filter(|n| n.text.starts_with("Followed")).count(), j.rivers().iter().filter(|r| r.followed.is_some()).count());
        }
        assert!(to_sea > 0, "no river of the test world reaches the sea");
    }

    #[test]
    fn the_way_gone_is_counted_by_how_it_was_gone() {
        let mut j = Journal::new();
        let step = VOXELS_PER_METRE as f64;
        let mut x = 0.0;
        let mut go = |j: &mut Journal, way: Option<Way>, metres: u32, y: f64, grounded: bool| {
            for _ in 0..metres {
                x += step;
                j.moved(DVec3::new(x, y, 0.0), way, grounded);
            }
        };
        go(&mut j, Some(Way::Foot), 100, 50.0, true);
        go(&mut j, Some(Way::Glide), 40, 900.0, false);
        go(&mut j, Some(Way::Sail), 30, 0.0, false);
        go(&mut j, Some(Way::Swim), 20, 0.0, false);
        // The first step has nowhere to be measured from.
        assert_eq!((j.walked.round(), j.glided.round(), j.sailed.round(), j.swum.round()), (99.0, 40.0, 30.0, 20.0));
        // Flying free counts for nothing, nor does the leap back from it.
        go(&mut j, None, 500, 2000.0, false);
        go(&mut j, Some(Way::Foot), 1, 75.0, true);
        assert_eq!(j.walked.round(), 99.0);
        // Nor a leap from one place to another.
        j.moved(DVec3::new(x + 5000.0, 60.0, 0.0), Some(Way::Foot), true);
        assert_eq!(j.walked.round(), 99.0);
        // The highest ground is ground stood on, not air flown through.
        assert_eq!(j.highest.as_ref().map(|h| h.0), Some(75.0 / VOXELS_PER_METRE));
    }

    #[test]
    fn days_seasons_and_years_pass_and_the_first_snow_is_noted_once_a_winter() {
        let mut j = Journal::new();
        // Begun at six in the evening, in early summer.
        assert_eq!(j.year(), None, "a new journal knows no year");
        j.tick(0.0, 0.3, 0, 18.0, false);
        assert_eq!((j.day(), j.seasons(), j.now.year, j.year()), (1, 0, 1, Some(0)));
        // Midnight turns the day.
        j.tick(0.3, 0.3125, 0, 1.2, false);
        assert_eq!(j.day(), 2);
        // On into autumn, winter, and the next spring, a step at a time, the world's year
        // turning as the date comes round.
        let (mut date, mut year) = (0.3125f32, 0);
        for _ in 0..180 {
            (date, year) = crate::season::advance_year(date, year, 0.1, 24.0);
            j.tick(0.1, date, year, 12.0, j.now.quarter() == 3);
        }
        assert_eq!((j.seasons(), j.now.year, j.year()), (3, 2, Some(1)));
        let notes: Vec<&str> = j.notes().map(|n| n.text.as_str()).collect();
        assert_eq!(notes, ["Autumn came in.", "Winter came in.", "First snow.", "Spring came in."]);
        // The clock comes back from a save, and with it the year, for a world saved before
        // it kept its own.
        let mut back = Journal::from_text(&j.to_text());
        assert_eq!(back.year(), Some(1));
        back.tick(0.0, date, year, 12.0, false);
        assert_eq!((back.day(), back.seasons(), back.now), (j.day(), j.seasons(), j.now));
        // A year set by hand is the journal's year too.
        back.tick(0.0, date, 6, 12.0, false);
        assert_eq!(back.now.when(), "early spring of the seventh year");
        assert_eq!(Stamp { date: 0.62, hour: 19.0, day: 3, year: 1 }.when(), "mid-autumn of the first year");
        assert_eq!(Stamp { date: 0.9, hour: 19.0, day: 3, year: 14 }.when(), "midwinter, year 14");
        assert_eq!((Stamp { hour: 19.0, ..Stamp::default() }.time_of_day(), Stamp { hour: 5.5, ..Stamp::default() }.time_of_day()), ("at dusk", "at dawn"));
    }

    #[test]
    fn only_so_many_notes_are_kept() {
        let mut j = Journal::new();
        for k in 0..NOTES + 25 {
            j.note(format!("Note {k}."));
        }
        assert_eq!(j.notes().count(), NOTES);
        assert_eq!(j.notes().next().map(|n| n.text.as_str()), Some("Note 25."));
        assert_eq!(Journal::from_text(&j.to_text()).notes().count(), NOTES);
    }

    #[test]
    fn everything_there_is_to_find_has_its_country_and_every_page_its_word() {
        let all = catalogue(&mat::build());
        assert_eq!(all.len(), Flower::ALL.len() + 6);
        let ids: std::collections::HashSet<&str> = all.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids.len(), all.len());
        for item in &all {
            let country = country(&item.id).unwrap_or_else(|| panic!("{} grows nowhere", item.id));
            assert!(country.len() > 8 && country.ends_with('.'), "{country}");
        }
        assert!(country("a thing from a newer game").is_none());
        for (page, word, title) in Page::ALL {
            assert_eq!(Page::named(word), Some(page));
            assert!(!title.is_empty());
        }
        assert_eq!(Page::named("maps"), None);
    }
}
