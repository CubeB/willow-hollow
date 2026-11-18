//! The console: a line of typed commands for changing the weather, the hour and the
//! date, going somewhere else, and bringing things into the world, to try things out
//! without restarting with other flags.
//!
//! Each command is one entry in [`COMMANDS`]: its name, how it is used, a line about it,
//! what Tab offers for its first word, and the function that reads its words and acts on
//! a [`Game`]. The running game is one Game and a screenshot being set up another, so
//! `--command` works on both; anything a game cannot do, it says so.

use std::collections::VecDeque;

use glam::DVec3;

use crate::{inventory, placed, season, weather::Forced};

/// What a command says back: a line or two to show, or what was wrong.
pub type Reply = Result<String, String>;

/// Where the player is, as the console reports it.
pub struct Here {
    /// The eye, in world voxels.
    pub eye: DVec3,
    /// The height of the ground (or water) under it.
    pub ground: f64,
    /// Which way the view looks, in degrees.
    pub yaw: f32,
    pub pitch: f32,
    pub flying: bool,
}

/// A named place, found for `tp` and `places`.
pub struct Named {
    pub name: String,
    pub kind: &'static str,
    pub x: f64,
    pub z: f64,
}

/// A hamlet, village or town, found for `settlements` and `tp village`.
pub struct Settlement {
    pub name: String,
    /// What it is, in words: hamlet, village, large village, town or great town.
    pub kind: &'static str,
    /// Its people, age, water and trades, in a line.
    pub facts: String,
    pub x: f64,
    pub z: f64,
}

/// Something `spawn` can bring.
#[derive(Clone, Debug, PartialEq)]
pub enum Spawn {
    /// A camp a few steps ahead, its fire in this state.
    Camp(u8),
    Murmuration,
    /// Crows near a point ahead, doing this (`--crows` has the words).
    Crows(String),
    /// A boat on the nearest water ahead.
    Boat,
    /// This many of a thing from the pack's list, set down at the feet.
    Thing(String, u32),
    /// This many people of the country about, standing in rows ahead.
    People(u32),
    /// A herd of animals of a kind ahead: this many, or as many as a herd of them has.
    Herd(worldgen::fauna::Kind, Option<u32>),
    /// A pen of animals of a kind ahead, fenced.
    Pen(worldgen::fauna::Kind),
}

/// What the commands act on. Only what every game can say is required; the rest can't
/// be done unless the game says otherwise.
pub trait Game {
    fn seed(&self) -> u32;
    fn here(&self) -> Here;
    /// The hour, and the date as a fraction of the year from the spring equinox.
    fn clock(&self) -> (f32, f32);
    fn set_clock(&mut self, hour: f32, date: f32);
    /// Which year of the world it is, counting its first as 0.
    fn year(&self) -> u32 {
        0
    }
    fn set_year(&mut self, _year: u32) -> Reply {
        cannot("changing the year")
    }
    /// What the weather is doing, in words.
    fn weather(&self) -> String;
    /// Holds a weather, or with None lets it go its own way.
    fn set_weather(&mut self, weather: Option<Forced>) -> Reply;
    /// Goes to `(x, z)`, `h` voxels above the ground or water there (the eye's height
    /// when not given).
    fn teleport(&mut self, x: f64, z: f64, h: Option<f64>) -> Reply;
    /// Where the world starts a new walk.
    fn home(&self) -> (f64, f64);
    fn back(&mut self) -> Reply {
        cannot("going back")
    }
    /// The named places nearest, those whose names have all of `query`'s words in them
    /// (all of them when it is empty), at most `n`.
    fn places(&self, _query: &str, _n: usize) -> Vec<Named> {
        Vec::new()
    }
    /// The settlements nearest, only those of `kind` when given, at most `n`.
    fn settlements(&self, _kind: Option<worldgen::settlements::Kind>, _n: usize) -> Vec<Settlement> {
        Vec::new()
    }
    /// Flies, or walks, or with None turns from one to the other.
    fn fly(&mut self, _on: Option<bool>) -> Reply {
        cannot("flying")
    }
    fn spawn(&mut self, _what: Spawn) -> Reply {
        cannot("spawning")
    }
    /// The northern lights out, or never, or with None as the night has them.
    fn aurora(&mut self, _on: Option<bool>) -> Reply {
        cannot("the northern lights")
    }
    fn give(&mut self, _item: &str, _n: u32) -> Reply {
        cannot("giving")
    }
    /// Walks this many times faster, or with None says how fast.
    fn pace(&mut self, _times: Option<f32>) -> Reply {
        cannot("speed")
    }
    /// Turns between the view through the eyes and the one from behind the figure.
    fn third(&mut self) -> Reply {
        cannot("the view")
    }
    /// Who lives in the settlement nearest, and where they all are now (#204).
    fn people(&mut self, _all: bool) -> Reply {
        cannot("looking for the people of a settlement")
    }
    /// Greets whoever of a settlement is nearest, as E does.
    fn greet(&mut self) -> Reply {
        cannot("greeting")
    }
}

fn cannot(what: &str) -> Reply {
    Err(format!("{what} can't be done here"))
}

/// One command.
pub struct Spec {
    pub name: &'static str,
    /// Other names it answers to.
    pub also: &'static [&'static str],
    pub usage: &'static str,
    pub about: &'static str,
    /// What its first word can be, for Tab.
    pub words: fn() -> Vec<&'static str>,
    pub run: fn(&[&str], &mut dyn Game) -> Reply,
}

fn none() -> Vec<&'static str> {
    Vec::new()
}

fn here_words() -> Vec<&'static str> {
    vec!["here", "all"]
}

/// Every command, in the order `help` lists them.
pub const COMMANDS: &[Spec] = &[
    Spec { name: "help", also: &["?"], usage: "help [command]", about: "list the commands, or say more of one", words: command_names, run: help },
    Spec { name: "clear", also: &[], usage: "clear", about: "empty the console", words: none, run: |_, _| Ok(String::new()) },
    Spec { name: "weather", also: &[], usage: "weather [kind|auto]", about: "hold a weather (clear, fair, broken, overcast, rain, snow, storm, fog), or let it go its own way", words: weather_words, run: weather },
    Spec { name: "time", also: &[], usage: "time [hour|+h|-h]", about: "set the hour (18, 18:30, dusk), or move it on or back", words: hour_words, run: time },
    Spec { name: "date", also: &[], usage: "date [month|0-1]", about: "set the time of year: a month, or a fraction of the year from the spring equinox", words: month_words, run: date },
    Spec { name: "season", also: &[], usage: "season <spring|summer|autumn|winter>", about: "set the time of year to the middle of a season", words: season_words, run: set_season },
    Spec { name: "year", also: &[], usage: "year [n|+n|-n]", about: "say which year of the world it is (its first is 0), or set it", words: none, run: year },
    Spec { name: "tp", also: &["teleport", "go"], usage: "tp <x> <z> [h] | tp x,z[,h] | tp <place> | tp <hamlet|village|town|...> | tp home | tp back", about: "go to voxel x, z (h above the ground), to a named place, to the nearest settlement of a kind, to the world's start, or back", words: tp_words, run: tp },
    Spec { name: "back", also: &[], usage: "back", about: "go back to where the last tp left from", words: none, run: |_, game| game.back() },
    Spec { name: "pos", also: &["where"], usage: "pos", about: "say where you are, ready for tp and for --over and --look", words: none, run: pos },
    Spec { name: "places", also: &[], usage: "places [name]", about: "the named places nearest, or those with this in their name", words: none, run: places },
    Spec { name: "settlements", also: &["villages", "towns"], usage: "settlements [hamlet|village|largevillage|town|greattown]", about: "the hamlets, villages and towns nearest, or those of one kind, with their people and trades", words: kind_words, run: settlements },
    Spec { name: "people", also: &[], usage: "people [here|all]", about: "who lives in the settlement nearest, and where they all are at this hour; all names each of them", words: here_words, run: |args, game| game.people(args.first() == Some(&"all")) },
    Spec { name: "greet", also: &["hello"], usage: "greet", about: "greet whoever of a settlement is nearest, as E does", words: none, run: |_, game| game.greet() },
    Spec { name: "fly", also: &[], usage: "fly [on|off]", about: "fly free of the ground, or walk again", words: on_off, run: |args, game| game.fly(switch(args, "fly")?) },
    Spec { name: "view", also: &["third"], usage: "view", about: "look through the eyes, or from behind the figure (V)", words: none, run: |_, game| game.third() },
    Spec { name: "spawn", also: &["summon"], usage: "spawn <camp [laid|lit|out|burnt] | murmuration | crows [how] | boat | people [n] | herd <animal> [n] | pen <animal> | thing [n]>", about: "make camp ahead, call starlings or crows, put a boat on the water ahead, stand people of the country ahead, bring a herd or a pen of animals, or set things down", words: spawn_words, run: spawn },
    Spec { name: "aurora", also: &[], usage: "aurora [on|off|auto]", about: "the northern lights out every night, never, or as the night has them", words: on_off_auto, run: aurora },
    Spec { name: "give", also: &[], usage: "give <thing> [n]", about: "put things in the pack: sticks, lantern, a flower (poppy, bluebell...), holly-berries...", words: item_words, run: give },
    Spec { name: "speed", also: &[], usage: "speed [times]", about: "walk, run and swim this many times faster (1 is usual)", words: none, run: speed },
    Spec { name: "seed", also: &[], usage: "seed", about: "say which world this is", words: none, run: |_, game| Ok(format!("world seed {0} (run with --seed {0} to come back to it)", game.seed())) },
];

/// The command a word names, by its name or another it answers to.
pub fn find(word: &str) -> Option<&'static Spec> {
    let word = word.to_ascii_lowercase();
    COMMANDS.iter().find(|c| c.name == word || c.also.contains(&word.as_str()))
}

fn command_names() -> Vec<&'static str> {
    COMMANDS.iter().map(|c| c.name).collect()
}

/// A line split into words, without the slash some type first out of habit.
pub fn words(line: &str) -> Vec<&str> {
    let line = line.trim();
    line.strip_prefix('/').unwrap_or(line).split_whitespace().collect()
}

/// Runs one line on `game`.
pub fn run(line: &str, game: &mut dyn Game) -> Reply {
    let words = words(line);
    let Some((&first, args)) = words.split_first() else { return Ok(String::new()) };
    let Some(spec) = find(first) else {
        let initial = first.chars().next().map(|c| c.to_ascii_lowercase());
        let near: Vec<&str> = command_names().into_iter().filter(|n| n.chars().next() == initial).collect();
        let hint = if near.is_empty() { String::new() } else { format!(" (perhaps {})", near.join(", ")) };
        return Err(format!("there is no command '{first}'{hint}: 'help' lists them"));
    };
    (spec.run)(args, game).map_err(|e| if e.starts_with("usage") { e } else { format!("{e}   (usage: {})", spec.usage) })
}

fn help(args: &[&str], _: &mut dyn Game) -> Reply {
    if let Some(word) = args.first() {
        let spec = find(word).ok_or_else(|| format!("there is no command '{word}'"))?;
        let also = if spec.also.is_empty() { String::new() } else { format!("\n  also: {}", spec.also.join(", ")) };
        return Ok(format!("{}\n  {}{also}", spec.usage, spec.about));
    }
    let wide = COMMANDS.iter().map(|c| c.usage.len().min(34)).max().unwrap_or(0);
    let mut out = String::from("Commands (Tab completes, Up and Down go through what was run):");
    for c in COMMANDS {
        out += &format!("\n  {:wide$}  {}", c.usage, c.about);
    }
    Ok(out)
}

/// A number, or a clear word about why it isn't one.
fn number(word: &str, what: &str) -> Result<f64, String> {
    word.parse::<f64>().ok().filter(|v| v.is_finite()).ok_or_else(|| format!("'{word}' is not a number, for {what}"))
}

/// The hours words stand for, for `time`.
const HOURS: &[(&str, f32)] = &[
    ("midnight", 0.0),
    ("dawn", 5.5),
    ("sunrise", 6.0),
    ("morning", 9.0),
    ("noon", 12.0),
    ("midday", 12.0),
    ("afternoon", 15.0),
    ("dusk", 18.3),
    ("sunset", 18.0),
    ("evening", 19.5),
    ("night", 22.0),
];

fn hour_words() -> Vec<&'static str> {
    HOURS.iter().map(|h| h.0).collect()
}

/// The hour `word` asks for, from `now`: 18, 18.5, 18:30, a word (dusk), or +2 or -1.5
/// hours on from now. Always within the day.
pub fn hour(word: &str, now: f32) -> Result<f32, String> {
    let word = word.to_ascii_lowercase();
    if let Some(&(_, h)) = HOURS.iter().find(|h| h.0 == word) {
        return Ok(h);
    }
    if let Some(by) = word.strip_prefix('+').or(word.strip_prefix('-').map(|_| word.as_str())) {
        let by = number(by, "hours to move the time by")? as f32;
        return Ok((now + by).rem_euclid(24.0));
    }
    let h = match word.split_once(':') {
        Some((h, m)) => {
            let (h, m) = (number(h, "the hour")?, number(m, "the minutes")?);
            if !(0.0..60.0).contains(&m) {
                return Err(format!("'{word}': there are 60 minutes to an hour"));
            }
            h + m / 60.0
        }
        None => number(&word, "the hour").map_err(|e| format!("{e} (try 18, 18:30, dusk or +2)"))?,
    };
    if !(0.0..=24.0).contains(&h) {
        return Err(format!("'{word}' is not an hour of the day: 0 to 24"));
    }
    Ok((h as f32).rem_euclid(24.0))
}

/// An hour as a clock shows it.
pub fn clock_face(hour: f32) -> String {
    let minutes = (hour.rem_euclid(24.0) * 60.0).round() as u32 % (24 * 60);
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

fn time(args: &[&str], game: &mut dyn Game) -> Reply {
    let (now, date) = game.clock();
    let Some(word) = args.first() else { return Ok(format!("it is {}", clock_face(now))) };
    let h = hour(word, now)?;
    game.set_clock(h, date);
    Ok(format!("the time is {}", clock_face(h)))
}

fn month_words() -> Vec<&'static str> {
    vec!["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"]
}

fn season_words() -> Vec<&'static str> {
    vec!["spring", "summer", "autumn", "winter"]
}

/// The time of year `word` asks for: a month, a season, or a fraction of the year from
/// the spring equinox.
pub fn date_of(word: &str) -> Result<f32, String> {
    if let Some(d) = season::month(word).or_else(|| season::named(&word.to_ascii_lowercase())) {
        return Ok(d);
    }
    let d = number(word, "the time of year").map_err(|_| format!("'{word}' is not a month, a season or a number from 0 to 1"))?;
    if !(0.0..=1.0).contains(&d) {
        return Err(format!("'{word}': the time of year is a fraction from 0 to 1 (0 the spring equinox)"));
    }
    Ok((d as f32).rem_euclid(1.0))
}

fn date_words(date: f32) -> String {
    format!("{} ({date:.2} of the year from the spring equinox)", season::month_name(date))
}

fn date(args: &[&str], game: &mut dyn Game) -> Reply {
    let (hour, now) = game.clock();
    let Some(word) = args.first() else { return Ok(format!("it is {}", date_words(now))) };
    let d = date_of(word)?;
    game.set_clock(hour, d);
    Ok(format!("it is now {}", date_words(d)))
}

/// The year `word` asks for, from `now`: a year counted from 0, or so many on or back.
fn year_of(word: &str, now: u32) -> Result<u32, String> {
    let wrong = || format!("'{word}' is not a year: a whole number from 0 (the world's first year), or +n or -n");
    let step = |n: &str| n.parse::<u32>().map_err(|_| wrong());
    match word.as_bytes().first() {
        Some(b'+') => Ok(now.saturating_add(step(&word[1..])?)),
        Some(b'-') => Ok(now.saturating_sub(step(&word[1..])?)),
        _ => step(word),
    }
}

fn year(args: &[&str], game: &mut dyn Game) -> Reply {
    let now = game.year();
    let Some(word) = args.first() else { return Ok(format!("it is year {now} of this world (its first is 0)")) };
    let y = year_of(word, now)?;
    game.set_year(y)?;
    Ok(format!("it is now year {y} of this world"))
}

fn set_season(args: &[&str], game: &mut dyn Game) -> Reply {
    let word = args.first().ok_or("which season: spring, summer, autumn or winter?")?;
    let d = season::named(&word.to_ascii_lowercase()).ok_or_else(|| format!("'{word}' is not a season: spring, summer, autumn or winter"))?;
    let (hour, _) = game.clock();
    game.set_clock(hour, d);
    Ok(format!("it is now {}", date_words(d)))
}

fn weather_words() -> Vec<&'static str> {
    let mut w = Forced::NAMES.to_vec();
    w.push("auto");
    w
}

fn weather(args: &[&str], game: &mut dyn Game) -> Reply {
    let Some(word) = args.first() else { return Ok(game.weather()) };
    let word = word.to_ascii_lowercase();
    if word == "auto" {
        return game.set_weather(None);
    }
    let w = Forced::named(&word).ok_or_else(|| format!("there is no weather '{word}': {} or auto", Forced::NAMES.join(", ")))?;
    game.set_weather(Some(w))
}

/// Where `tp` is to go.
#[derive(Debug, PartialEq)]
pub enum Goto {
    At(f64, f64, Option<f64>),
    Home,
    Back,
    Place(String),
    /// The nearest settlement of a kind.
    Nearest(worldgen::settlements::Kind),
}

/// Reads `tp`'s words: two or three numbers, apart or joined by commas as `pos` and
/// `--over` have them, `home`, `back`, or the name of a place.
pub fn goto(args: &[&str]) -> Result<Goto, String> {
    let joined = args.join(" ");
    let parts: Vec<&str> = joined.split([',', ' ']).filter(|p| !p.is_empty()).collect();
    match parts.as_slice() {
        [] => Err("where to? tp x z, tp x,z,h, tp home, or the name of a place".into()),
        ["home"] => Ok(Goto::Home),
        ["back"] => Ok(Goto::Back),
        [x, ..] if x.parse::<f64>().is_err() => Ok(match worldgen::settlements::Kind::named(&joined) {
            Some(kind) => Goto::Nearest(kind),
            None => Goto::Place(joined.trim().to_string()),
        }),
        [x, z] => Ok(Goto::At(number(x, "x")?, number(z, "z")?, None)),
        [x, z, h] => Ok(Goto::At(number(x, "x")?, number(z, "z")?, Some(number(h, "the height above the ground")?))),
        _ => Err(format!("'{joined}': tp takes x and z, and a height if you like")),
    }
}

fn tp_words() -> Vec<&'static str> {
    let mut words = vec!["home", "back"];
    words.extend(kind_words());
    words
}

fn kind_words() -> Vec<&'static str> {
    worldgen::settlements::Kind::ALL.iter().map(|k| k.key()).collect()
}

fn tp(args: &[&str], game: &mut dyn Game) -> Reply {
    match goto(args)? {
        Goto::At(x, z, h) => game.teleport(x, z, h),
        Goto::Home => {
            let (x, z) = game.home();
            game.teleport(x, z, None)
        }
        Goto::Back => game.back(),
        Goto::Place(name) => {
            let found = game.places(&name, 1);
            let p = found.first().ok_or_else(|| format!("no place called '{name}' in this part of the world ('places' lists those near)"))?;
            let (x, z, said) = (p.x, p.z, format!("{} ({})", p.name, p.kind));
            game.teleport(x, z, None).map(|line| format!("{said}: {line}"))
        }
        Goto::Nearest(kind) => {
            let found = game.settlements(Some(kind), 1);
            let s = found.first().ok_or_else(|| format!("no {} in this part of the world", kind.word()))?;
            let (x, z, said) = (s.x, s.z, format!("{} ({})", s.name, s.kind));
            game.teleport(x, z, None).map(|line| format!("{said}: {line}"))
        }
    }
}

fn settlements(args: &[&str], game: &mut dyn Game) -> Reply {
    let words = args.join(" ");
    let kind = match words.trim() {
        "" => None,
        w => Some(worldgen::settlements::Kind::named(w).ok_or_else(|| format!("'{w}' is not a kind of settlement: {}", kind_words().join(", ")))?),
    };
    let found = game.settlements(kind, 6);
    if found.is_empty() {
        return Err("no settlements in this part of the world".into());
    }
    let here = game.here().eye;
    let mut out = String::new();
    for s in found {
        let (dx, dz) = (s.x - here.x, s.z - here.z);
        let metres = dx.hypot(dz) / worldgen::scale::VOXELS_PER_METRE as f64;
        let far = if metres >= 1000.0 { format!("{:.1} km", metres / 1000.0) } else { format!("{metres:.0} m") };
        if !out.is_empty() {
            out.push('\n');
        }
        out += &format!("{} ({}), {far} {}   tp {:.0},{:.0}\n  {}", s.name, s.kind, compass(dx, dz), s.x, s.z, s.facts);
    }
    Ok(out)
}

fn pos(_: &[&str], game: &mut dyn Game) -> Reply {
    let h = game.here();
    let above = h.eye.y - h.ground;
    Ok(format!(
        "{:.0},{:.0},{:.0}   --over {:.0},{:.0},{:.0} --look {:.0},{:.0}   ({}, the eye at height {:.0}, {:.1} m over the ground)",
        h.eye.x.floor(),
        h.eye.z.floor(),
        above,
        h.eye.x.floor(),
        h.eye.z.floor(),
        above,
        h.yaw.rem_euclid(360.0),
        h.pitch,
        if h.flying { "flying" } else { "on foot" },
        h.eye.y,
        above / worldgen::scale::VOXELS_PER_METRE as f64,
    ))
}

/// Which way `(dx, dz)` lies, as a compass has it: north is toward -z.
pub fn compass(dx: f64, dz: f64) -> &'static str {
    let a = (dx.atan2(-dz).to_degrees() + 360.0 + 22.5).rem_euclid(360.0);
    ["N", "NE", "E", "SE", "S", "SW", "W", "NW"][(a / 45.0) as usize % 8]
}

fn places(args: &[&str], game: &mut dyn Game) -> Reply {
    let query = args.join(" ");
    let found = game.places(&query, 10);
    if found.is_empty() {
        return Err(if query.is_empty() { "no named places known here yet".into() } else { format!("no place called '{query}' in this part of the world") });
    }
    let here = game.here().eye;
    let mut out = String::new();
    for p in found {
        let (dx, dz) = (p.x - here.x, p.z - here.z);
        let metres = dx.hypot(dz) / worldgen::scale::VOXELS_PER_METRE as f64;
        let far = if metres >= 1000.0 { format!("{:.1} km", metres / 1000.0) } else { format!("{metres:.0} m") };
        if !out.is_empty() {
            out.push('\n');
        }
        out += &format!("{} ({}), {far} {}   tp {:.0},{:.0}", p.name, p.kind, compass(dx, dz), p.x, p.z);
    }
    Ok(out)
}

fn on_off() -> Vec<&'static str> {
    vec!["on", "off"]
}

fn on_off_auto() -> Vec<&'static str> {
    vec!["on", "off", "auto"]
}

/// `on`, `off` or nothing (None: turn it over).
fn switch(args: &[&str], what: &str) -> Result<Option<bool>, String> {
    match args.first().map(|w| w.to_ascii_lowercase()).as_deref() {
        None => Ok(None),
        Some("on" | "yes" | "1") => Ok(Some(true)),
        Some("off" | "no" | "0") => Ok(Some(false)),
        Some(other) => Err(format!("'{other}': {what} is on or off")),
    }
}

fn aurora(args: &[&str], game: &mut dyn Game) -> Reply {
    let on = match args.first().map(|w| w.to_ascii_lowercase()).as_deref() {
        Some("auto") => None,
        None => Some(true),
        Some(_) => Some(switch(args, "aurora")?.unwrap_or(true)),
    };
    game.aurora(on)
}

/// The fire states `spawn camp` takes, and how they read.
const FIRES: &[(&str, u8)] = &[("laid", placed::LAID), ("lit", placed::BURNING), ("out", placed::OUT), ("burnt", placed::BURNT)];

/// The state of fire a word asks for.
pub fn fire_state(word: &str) -> Option<u8> {
    FIRES.iter().find(|f| f.0 == word).map(|f| f.1)
}

/// The word for a fire's state.
pub fn fire_word(state: u8) -> &'static str {
    FIRES.iter().find(|f| f.1 == state).map_or("laid", |f| f.0)
}

/// What crows can be called in to do.
pub const CROWS: &[&str] = &["ground", "perch", "tree", "rock", "fly", "land", "roost", "any"];

fn spawn_words() -> Vec<&'static str> {
    let mut w = vec!["camp", "murmuration", "crows", "boat", "people", "herd", "pen"];
    w.extend(item_words().into_iter().filter(|i| *i != "lantern"));
    w
}

/// Reads `spawn`'s words.
pub fn spawning(args: &[&str]) -> Result<Spawn, String> {
    let Some((what, rest)) = args.split_first() else { return Err("spawn what? camp, murmuration, crows, boat, or a thing (sticks, poppy...)".into()) };
    let what = what.to_ascii_lowercase();
    Ok(match what.as_str() {
        "camp" | "fire" | "campfire" => {
            let word = rest.first().map_or("lit".to_string(), |w| w.to_ascii_lowercase());
            Spawn::Camp(fire_state(&word).ok_or_else(|| format!("'{word}': a fire is laid, lit, out or burnt"))?)
        }
        "murmuration" | "starlings" => Spawn::Murmuration,
        "crows" | "crow" => {
            let how = rest.first().map_or("any".to_string(), |w| w.to_ascii_lowercase());
            if !CROWS.contains(&how.as_str()) {
                return Err(format!("'{how}': crows can be called to {}", CROWS.join(", ")));
            }
            Spawn::Crows(how)
        }
        "boat" => Spawn::Boat,
        "people" | "folk" => Spawn::People(rest.first().map_or(Ok(12), |n| count(n))?.min(60)),
        "herd" | "herds" | "animals" | "flock" => {
            // The kind may be more than a word ("wild boar"), the count last.
            let (n, words) = match rest.split_last() {
                Some((last, before)) if last.chars().all(|c| c.is_ascii_digit()) => (Some(count(last)?.min(40)), before),
                _ => (None, rest),
            };
            Spawn::Herd(animal(words)?, n)
        }
        "pen" | "coop" | "pasture" | "fold" => Spawn::Pen(animal(rest)?),
        _ => {
            let n = match rest.first() {
                Some(n) => count(n)?,
                None if what == "sticks" => inventory::FIRE_STICKS,
                None => 1,
            };
            Spawn::Thing(what, n)
        }
    })
}

/// The animal some words name: "sheep", "wild boar", "wild-goats", "hens".
fn animal(words: &[&str]) -> Result<worldgen::fauna::Kind, String> {
    use worldgen::fauna::Kind;
    let all = || Kind::ALL.iter().map(|k| k.key()).collect::<Vec<_>>().join(", ");
    if words.is_empty() {
        return Err(format!("of what? {}", all()));
    }
    Kind::named(&words.join("-")).ok_or_else(|| format!("there is no animal called '{}': {}", words.join(" "), all()))
}

fn spawn(args: &[&str], game: &mut dyn Game) -> Reply {
    let what = spawning(args)?;
    game.spawn(what)
}

/// How many of a thing: a whole number from 1 to 999.
fn count(word: &str) -> Result<u32, String> {
    word.parse::<u32>().ok().filter(|n| (1..=999).contains(n)).ok_or_else(|| format!("'{word}' is not a count from 1 to 999"))
}

fn give(args: &[&str], game: &mut dyn Game) -> Reply {
    let Some((what, rest)) = args.split_first() else { return Err("give what? sticks, lantern, or a flower (poppy, bluebell...)".into()) };
    let n = rest.first().map_or(Ok(1), |n| count(n))?;
    game.give(&what.to_ascii_lowercase(), n)
}

fn speed(args: &[&str], game: &mut dyn Game) -> Reply {
    let Some(word) = args.first() else { return game.pace(None) };
    let times = number(word, "how many times faster")?;
    if !(0.1..=20.0).contains(&times) {
        return Err(format!("'{word}': speed goes from 0.1 to 20 times the usual"));
    }
    game.pace(Some(times as f32))
}

/// What can be carried in the pack, by the ids its things have.
pub fn item_words() -> Vec<&'static str> {
    use worldgen::features::plants::Flower;
    let mut w = vec!["sticks", "lantern", "gorse-blossom", "holly-berries", "ivy-berries", "reed-mace"];
    w.extend(Flower::ALL.iter().map(|f| f.id()));
    w
}

/// The thing a word names: its id, or its name, with spaces, dashes and underscores
/// alike ("water lily", "water_lily", "Holly berries").
pub fn item_named(word: &str, materials: &voxel::Materials) -> Option<inventory::Item> {
    use worldgen::{features::plants::Flower, materials as mat};
    let plain = |s: &str| s.to_ascii_lowercase().replace([' ', '-', '_'], "");
    let word = plain(word);
    if word == "sticks" || word == "stick" {
        return Some(inventory::sticks());
    }
    if word == "lantern" {
        return Some(inventory::lantern());
    }
    if let Some(f) = Flower::ALL.iter().find(|f| plain(f.id()) == word || plain(f.name()) == word) {
        return Some(inventory::of_flower(*f));
    }
    [mat::GORSE_BLOOM, mat::HOLLY_BERRY, mat::IVY_BERRY, mat::REED_HEAD]
        .into_iter()
        .filter_map(|v| inventory::of_voxel(v, materials))
        .find(|i| plain(&i.id) == word || plain(&i.name) == word)
}

/// What Tab makes of `input`: the command name, or its first word, filled out as far as
/// the choices agree, and the choices when there is more than one.
pub fn complete(input: &str) -> (String, Vec<&'static str>) {
    let slash = if input.trim_start().starts_with('/') { "/" } else { "" };
    let words = words(input);
    let open = input.ends_with(' ') || words.is_empty();
    let (before, partial, choices) = match (words.as_slice(), open) {
        ([], _) => (String::new(), "", command_names()),
        ([first], false) => (String::new(), *first, COMMANDS.iter().flat_map(|c| std::iter::once(c.name).chain(c.also.iter().copied())).collect()),
        ([first], true) | ([first, _], false) => {
            let Some(spec) = find(first) else { return (input.to_string(), Vec::new()) };
            (format!("{} ", first), words.get(1).copied().unwrap_or(""), (spec.words)())
        }
        _ => return (input.to_string(), Vec::new()),
    };
    let lower = partial.to_ascii_lowercase();
    let mut fits: Vec<&'static str> = choices.into_iter().filter(|c| c.starts_with(&lower)).collect();
    fits.dedup();
    match fits.as_slice() {
        [] => (input.to_string(), Vec::new()),
        [one] => (format!("{slash}{before}{one} "), Vec::new()),
        many => {
            let mut common = many[0].to_string();
            for m in &many[1..] {
                let same = common.chars().zip(m.chars()).take_while(|(a, b)| a == b).count();
                common.truncate(common.char_indices().nth(same).map_or(common.len(), |(i, _)| i));
            }
            let common = if common.len() > lower.len() { common } else { partial.to_string() };
            (format!("{slash}{before}{common}"), fits)
        }
    }
}

/// How a line in the console's scrollback came about.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Said {
    /// What was typed.
    Typed,
    Answer,
    Wrong,
}

/// How many lines of what was said the console keeps, and how many of what was typed.
const SCROLLBACK: usize = 200;
const HISTORY: usize = 100;
/// How many lines of the scrollback show at once.
const SHOWN: f32 = 12.0;

/// The console's window, what was typed into it and what it said back.
#[derive(Default)]
pub struct Console {
    pub open: bool,
    input: String,
    history: Vec<String>,
    /// How far back through the history Up has gone, and what was being typed before.
    browsing: Option<usize>,
    draft: String,
    lines: VecDeque<(Said, String)>,
    /// The key that opened it may have typed itself; cleared after the first frame.
    fresh: bool,
    /// The text was changed under the cursor: put the cursor at its end.
    to_end: bool,
}

impl Console {
    pub fn set_open(&mut self, open: bool) {
        self.open = open;
        if open {
            self.input.clear();
            self.browsing = None;
            self.fresh = true;
        }
    }

    fn say(&mut self, said: Said, text: &str) {
        for line in text.lines() {
            self.lines.push_back((said, line.to_string()));
        }
        while self.lines.len() > SCROLLBACK {
            self.lines.pop_front();
        }
    }

    /// Runs a line as if typed, keeping it and what it gave in the scrollback.
    pub fn run(&mut self, line: &str, game: &mut dyn Game) -> Reply {
        let line = line.trim();
        if line.is_empty() {
            return Ok(String::new());
        }
        if self.history.last().is_none_or(|l| l != line) {
            self.history.push(line.to_string());
            if self.history.len() > HISTORY {
                self.history.remove(0);
            }
        }
        if words(line).first().is_some_and(|w| find(w).is_some_and(|c| c.name == "clear")) {
            self.lines.clear();
            return Ok(String::new());
        }
        self.say(Said::Typed, &format!("> {line}"));
        let reply = run(line, game);
        match &reply {
            Ok(text) => self.say(Said::Answer, text),
            Err(text) => self.say(Said::Wrong, text),
        }
        reply
    }

    /// Runs what has been typed (Enter).
    pub fn submit(&mut self, game: &mut dyn Game) -> Option<Reply> {
        let line = std::mem::take(&mut self.input);
        self.browsing = None;
        (!line.trim().is_empty()).then(|| self.run(&line, game))
    }

    /// Goes one line further back through what was run (Up).
    pub fn older(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let at = match self.browsing {
            None => {
                self.draft = self.input.clone();
                self.history.len() - 1
            }
            Some(at) => at.saturating_sub(1),
        };
        self.browsing = Some(at);
        self.input = self.history[at].clone();
        self.to_end = true;
    }

    /// Comes one line forward again (Down), to what was being typed.
    pub fn newer(&mut self) {
        let Some(at) = self.browsing else { return };
        if at + 1 < self.history.len() {
            self.browsing = Some(at + 1);
            self.input = self.history[at + 1].clone();
        } else {
            self.browsing = None;
            self.input = std::mem::take(&mut self.draft);
        }
        self.to_end = true;
    }

    /// Fills out the word being typed (Tab), listing the choices when it can't.
    pub fn complete(&mut self) {
        let (filled, choices) = complete(&self.input);
        if choices.len() > 1 {
            self.say(Said::Answer, &choices.join("  "));
        }
        self.input = filled;
        self.to_end = true;
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        let id = egui::Id::unique("console input");
        let width = (ctx.content_rect().width() * 0.62).clamp(320.0, 820.0);
        let mono = |text: &str, colour: egui::Color32| egui::RichText::new(text).monospace().size(14.0).color(colour);
        egui::Area::new(egui::Id::unique("console"))
            .anchor(egui::Align2::LEFT_BOTTOM, [14.0, -14.0])
            .fade_in(false)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(egui::Color32::from_rgba_premultiplied(12, 15, 20, 220))
                    .corner_radius(8.0)
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.set_width(width);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        egui::ScrollArea::vertical()
                            .max_height(SHOWN * 17.0)
                            .stick_to_bottom(true)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                for (said, text) in &self.lines {
                                    let colour = match said {
                                        Said::Typed => egui::Color32::from_rgb(150, 160, 172),
                                        Said::Answer => egui::Color32::from_rgb(226, 230, 222),
                                        Said::Wrong => egui::Color32::from_rgb(240, 140, 120),
                                    };
                                    ui.add(egui::Label::new(mono(text, colour)).wrap());
                                }
                            });
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(mono(">", egui::Color32::from_rgb(236, 200, 120)));
                            let edit = egui::TextEdit::singleline(&mut self.input)
                                .id(id)
                                .font(egui::TextStyle::Monospace)
                                .frame(egui::Frame::NONE)
                                .desired_width(f32::INFINITY)
                                .hint_text("help lists the commands");
                            ui.add(edit).request_focus();
                        });
                        ui.add_space(2.0);
                        ui.label(egui::RichText::new("Enter runs it   \u{b7}   Tab completes   \u{b7}   Up and Down go through what was run   \u{b7}   Esc closes").size(12.0).color(egui::Color32::from_gray(150)));
                    });
            });
        if std::mem::take(&mut self.fresh) {
            self.input.clear();
            self.to_end = true;
        }
        if std::mem::take(&mut self.to_end)
            && let Some(mut state) = egui::TextEdit::load_state(ctx, id)
        {
            let end = egui::text::CCursor::new(self.input.chars().count());
            state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
            state.store(ctx, id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A game that remembers what was asked of it.
    struct Mock {
        hour: f32,
        date: f32,
        year: u32,
        weather: Option<Option<Forced>>,
        went: Option<(f64, f64, Option<f64>)>,
        spawned: Vec<Spawn>,
        given: Vec<(String, u32)>,
        pace: f32,
    }

    impl Mock {
        fn new() -> Self {
            Self { hour: 10.0, date: 0.32, year: 0, weather: None, went: None, spawned: Vec::new(), given: Vec::new(), pace: 1.0 }
        }
    }

    impl Game for Mock {
        fn seed(&self) -> u32 {
            7
        }
        fn here(&self) -> Here {
            Here { eye: DVec3::new(100.5, 64.0, 200.5), ground: 60.0, yaw: 180.0, pitch: -10.0, flying: false }
        }
        fn clock(&self) -> (f32, f32) {
            (self.hour, self.date)
        }
        fn set_clock(&mut self, hour: f32, date: f32) {
            (self.hour, self.date) = (hour, date);
        }
        fn year(&self) -> u32 {
            self.year
        }
        fn set_year(&mut self, year: u32) -> Reply {
            self.year = year;
            Ok(String::new())
        }
        fn weather(&self) -> String {
            "fair".into()
        }
        fn set_weather(&mut self, weather: Option<Forced>) -> Reply {
            self.weather = Some(weather);
            Ok(String::new())
        }
        fn teleport(&mut self, x: f64, z: f64, h: Option<f64>) -> Reply {
            self.went = Some((x, z, h));
            Ok(String::new())
        }
        fn home(&self) -> (f64, f64) {
            (5.0, 6.0)
        }
        fn places(&self, query: &str, _n: usize) -> Vec<Named> {
            let all = [("Hollin Tarn", "lake", 10.0, 20.0), ("Old Hollin Cairn", "cairn", 30.0, 40.0)];
            all.iter().filter(|p| p.0.to_lowercase().contains(&query.to_lowercase())).map(|p| Named { name: p.0.into(), kind: p.1, x: p.2, z: p.3 }).collect()
        }
        fn settlements(&self, kind: Option<worldgen::settlements::Kind>, n: usize) -> Vec<Settlement> {
            use worldgen::settlements::Kind;
            let all = [("Ashcot", Kind::Hamlet, 50.0, 60.0), ("Brynell", Kind::GreatTown, 900.0, 80.0), ("Elmley", Kind::Village, 300.0, 400.0)];
            all.iter()
                .filter(|s| kind.is_none_or(|k| k == s.1))
                .take(n)
                .map(|s| Settlement { name: s.0.into(), kind: s.1.word(), facts: "9 people".into(), x: s.2, z: s.3 })
                .collect()
        }
        fn spawn(&mut self, what: Spawn) -> Reply {
            self.spawned.push(what);
            Ok(String::new())
        }
        fn give(&mut self, item: &str, n: u32) -> Reply {
            self.given.push((item.into(), n));
            Ok(String::new())
        }
        fn pace(&mut self, times: Option<f32>) -> Reply {
            self.pace = times.unwrap_or(self.pace);
            Ok(String::new())
        }
    }

    #[test]
    fn hours_read_as_numbers_clock_times_words_and_steps() {
        assert_eq!(hour("18", 10.0), Ok(18.0));
        assert_eq!(hour("18:30", 10.0), Ok(18.5));
        assert_eq!(hour("6.25", 10.0), Ok(6.25));
        assert_eq!(hour("noon", 3.0), Ok(12.0));
        assert_eq!(hour("+3", 22.0), Ok(1.0));
        assert_eq!(hour("-2.5", 1.0), Ok(22.5));
        assert_eq!(hour("24", 1.0), Ok(0.0));
        assert!(hour("25", 1.0).is_err());
        assert!(hour("12:75", 1.0).is_err());
        assert!(hour("teatime", 1.0).unwrap_err().contains("teatime"));
        assert_eq!(clock_face(18.5), "18:30");
        assert_eq!(clock_face(23.999), "00:00");
    }

    #[test]
    fn dates_read_as_months_seasons_and_fractions() {
        assert_eq!(date_of("winter"), Ok(season::named("winter").unwrap()));
        assert_eq!(date_of("May"), Ok(season::month("may").unwrap()));
        assert_eq!(date_of("0.5"), Ok(0.5));
        assert!(date_of("1.5").is_err());
        assert!(date_of("smarch").is_err());
    }

    #[test]
    fn the_year_is_said_and_set_from_0_or_moved_on_and_back() {
        let mut game = Mock::new();
        assert!(run("year", &mut game).unwrap().contains("year 0"));
        assert_eq!(run("year 4", &mut game), Ok("it is now year 4 of this world".into()));
        assert_eq!(game.year, 4);
        run("year +2", &mut game).unwrap();
        assert_eq!(game.year, 6);
        run("year -10", &mut game).unwrap();
        assert_eq!(game.year, 0);
        assert!(run("year last", &mut game).unwrap_err().contains("'last'"));
        assert!(run("year 1.5", &mut game).is_err());
        // The date is left as it was.
        assert_eq!((game.hour, game.date), (10.0, 0.32));
    }

    #[test]
    fn tp_reads_numbers_apart_or_joined_names_and_home() {
        assert_eq!(goto(&["42186", "30679"]), Ok(Goto::At(42186.0, 30679.0, None)));
        assert_eq!(goto(&["42186", "30679", "60"]), Ok(Goto::At(42186.0, 30679.0, Some(60.0))));
        assert_eq!(goto(&["42186,30679,60"]), Ok(Goto::At(42186.0, 30679.0, Some(60.0))));
        assert_eq!(goto(&["-12,", "40"]), Ok(Goto::At(-12.0, 40.0, None)));
        assert_eq!(goto(&["home"]), Ok(Goto::Home));
        assert_eq!(goto(&["back"]), Ok(Goto::Back));
        assert_eq!(goto(&["Hollin", "Tarn"]), Ok(Goto::Place("Hollin Tarn".into())));
        assert_eq!(goto(&["village"]), Ok(Goto::Nearest(worldgen::settlements::Kind::Village)));
        assert_eq!(goto(&["great", "town"]), Ok(Goto::Nearest(worldgen::settlements::Kind::GreatTown)));
        assert_eq!(goto(&["greattown"]), Ok(Goto::Nearest(worldgen::settlements::Kind::GreatTown)));
        assert!(goto(&[]).is_err());
        assert!(goto(&["1", "2", "3", "4"]).is_err());
        assert!(goto(&["1", "x"]).is_err());
    }

    #[test]
    fn commands_act_on_the_game() {
        let mut g = Mock::new();
        assert!(run("weather storm", &mut g).is_ok());
        assert_eq!(g.weather, Some(Some(Forced::Storm)));
        assert!(run("/weather auto", &mut g).is_ok());
        assert_eq!(g.weather, Some(None));
        assert!(run("weather sleet", &mut g).unwrap_err().contains("clear, fair"));
        run("time 21", &mut g).unwrap();
        run("time +4", &mut g).unwrap();
        assert_eq!(g.hour, 1.0);
        run("season autumn", &mut g).unwrap();
        assert_eq!(g.date, season::named("autumn").unwrap());
        assert_eq!(g.hour, 1.0, "the date leaves the hour be");
        run("tp 10 20 3", &mut g).unwrap();
        assert_eq!(g.went, Some((10.0, 20.0, Some(3.0))));
        run("tp home", &mut g).unwrap();
        assert_eq!(g.went, Some((5.0, 6.0, None)));
        assert!(run("tp hollin tarn", &mut g).unwrap().starts_with("Hollin Tarn (lake)"));
        assert_eq!(g.went, Some((10.0, 20.0, None)));
        assert!(run("tp nowhere at all", &mut g).is_err());
        assert!(run("tp greattown", &mut g).unwrap().starts_with("Brynell (great town)"));
        assert_eq!(g.went, Some((900.0, 80.0, None)));
        assert!(run("tp town", &mut g).is_err(), "there is no plain town here");
        let listed = run("settlements", &mut g).unwrap();
        assert!(listed.contains("Ashcot (hamlet)") && listed.contains("tp 300,400") && listed.contains("9 people"), "{listed}");
        assert!(!run("villages village", &mut g).unwrap().contains("Ashcot"));
        assert!(run("settlements city", &mut g).unwrap_err().contains("greattown"));
        run("spawn camp", &mut g).unwrap();
        run("spawn camp out", &mut g).unwrap();
        run("spawn crows perch", &mut g).unwrap();
        run("spawn sticks", &mut g).unwrap();
        run("spawn poppy 4", &mut g).unwrap();
        assert_eq!(g.spawned, [Spawn::Camp(placed::BURNING), Spawn::Camp(placed::OUT), Spawn::Crows("perch".into()), Spawn::Thing("sticks".into(), 3), Spawn::Thing("poppy".into(), 4)]);
        assert!(run("spawn camp roaring", &mut g).is_err());
        assert!(run("spawn crows dance", &mut g).is_err());
        run("give Poppy 12", &mut g).unwrap();
        assert_eq!(g.given, [("poppy".to_string(), 12)]);
        assert!(run("give poppy lots", &mut g).is_err());
        run("speed 2.5", &mut g).unwrap();
        assert_eq!(g.pace, 2.5);
        assert!(run("speed 100", &mut g).is_err());
        assert!(run("pos", &mut g).unwrap().contains("--over 100,200,4 --look 180,-10"));
        assert!(run("seed", &mut g).unwrap().contains('7'));
        // What the game cannot do, it says so.
        assert!(run("fly", &mut g).unwrap_err().contains("can't be done here"));
    }

    #[test]
    fn mistakes_say_what_was_wrong_and_how_it_goes() {
        let mut g = Mock::new();
        let e = run("wether rain", &mut g).unwrap_err();
        assert!(e.contains("no command 'wether'") && e.contains("weather"), "{e}");
        let e = run("time soon", &mut g).unwrap_err();
        assert!(e.contains("usage: time"), "{e}");
        assert_eq!(run("   ", &mut g), Ok(String::new()));
    }

    #[test]
    fn help_lists_every_command() {
        let mut g = Mock::new();
        let all = run("help", &mut g).unwrap();
        for c in COMMANDS {
            assert!(all.contains(c.usage), "{} is missing from help", c.name);
        }
        assert!(run("help tp", &mut g).unwrap().contains("teleport"));
        // Every name is the command's own.
        let mut names: Vec<&str> = COMMANDS.iter().flat_map(|c| std::iter::once(c.name).chain(c.also.iter().copied())).collect();
        names.sort();
        let n = names.len();
        names.dedup();
        assert_eq!(n, names.len());
    }

    #[test]
    fn tab_fills_out_names_and_first_words() {
        assert_eq!(complete("wea"), ("weather ".to_string(), vec![]));
        assert_eq!(complete("/wea"), ("/weather ".to_string(), vec![]));
        assert_eq!(complete("weather st"), ("weather storm ".to_string(), vec![]));
        let (filled, choices) = complete("s");
        assert_eq!(filled, "s");
        assert!(choices.contains(&"spawn") && choices.contains(&"speed") && choices.contains(&"season"));
        let (filled, choices) = complete("sp");
        assert_eq!(filled, "sp");
        assert_eq!(choices, ["spawn", "speed"]);
        assert_eq!(complete("spawn mu"), ("spawn murmuration ".to_string(), vec![]));
        assert_eq!(complete("give water"), ("give water_lily ".to_string(), vec![]));
        let (filled, choices) = complete("weather ");
        assert_eq!(filled, "weather ");
        assert_eq!(choices.len(), Forced::NAMES.len() + 1);
        // Nothing to offer: left as it was.
        assert_eq!(complete("tp 1 2"), ("tp 1 2".to_string(), vec![]));
        assert_eq!(complete("zz"), ("zz".to_string(), vec![]));
    }

    #[test]
    fn the_console_keeps_history_and_scrollback() {
        let mut g = Mock::new();
        let mut c = Console::default();
        c.set_open(true);
        for line in ["time 6", "weather rain", "weather rain", "time noon"] {
            c.input = line.into();
            c.submit(&mut g);
        }
        assert_eq!(c.history, ["time 6", "weather rain", "time noon"]);
        c.input = "half typed".into();
        c.older();
        assert_eq!(c.input, "time noon");
        c.older();
        c.older();
        c.older();
        assert_eq!(c.input, "time 6");
        c.newer();
        assert_eq!(c.input, "weather rain");
        c.newer();
        c.newer();
        assert_eq!(c.input, "half typed");
        assert!(c.lines.iter().any(|l| l.0 == Said::Typed && l.1 == "> time noon"));
        c.run("nonsense", &mut g).unwrap_err();
        assert_eq!(c.lines.back().unwrap().0, Said::Wrong);
        c.run("clear", &mut g).unwrap();
        assert!(c.lines.is_empty());
    }

    #[test]
    fn things_are_found_by_id_or_name() {
        let materials = worldgen::materials::build();
        assert_eq!(item_named("water lily", &materials).unwrap().id, "water_lily");
        assert_eq!(item_named("Holly berries", &materials).unwrap().id, "holly-berries");
        assert_eq!(item_named("stick", &materials).unwrap().id, "sticks");
        assert!(item_named("dragon", &materials).is_none());
        for w in item_words() {
            assert!(item_named(w, &materials).is_some(), "{w} is offered but not found");
        }
    }

    #[test]
    fn compass_points() {
        assert_eq!(compass(0.0, -10.0), "N");
        assert_eq!(compass(10.0, 0.0), "E");
        assert_eq!(compass(-10.0, 10.0), "SW");
    }

    #[test]
    fn people_are_called_by_the_dozen_or_as_many_as_asked() {
        assert_eq!(spawning(&["people"]), Ok(Spawn::People(12)));
        assert_eq!(spawning(&["people", "5"]), Ok(Spawn::People(5)));
        assert_eq!(spawning(&["people", "500"]), Ok(Spawn::People(60)));
        assert!(spawning(&["people", "some"]).is_err());
    }

    #[test]
    fn herds_and_pens_are_called_by_their_animal() {
        use worldgen::fauna::Kind;
        assert_eq!(spawning(&["herd", "sheep"]), Ok(Spawn::Herd(Kind::Sheep, None)));
        assert_eq!(spawning(&["herd", "wild", "boar", "5"]), Ok(Spawn::Herd(Kind::Boar, Some(5))));
        assert_eq!(spawning(&["herd", "wild-goats", "500"]), Ok(Spawn::Herd(Kind::WildGoat, Some(40))));
        assert_eq!(spawning(&["pen", "hens"]), Ok(Spawn::Pen(Kind::Chicken)));
        assert!(spawning(&["herd"]).is_err());
        assert!(spawning(&["pen", "dragons"]).is_err());
    }
}
