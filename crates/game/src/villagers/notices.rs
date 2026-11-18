//! What is pinned on a settlement's boards (#207): on the tavern's, a request or two a
//! week from its people, and on a guild house's, what each of its guilds is.
//!
//! A week's requests are a pure function of the settlement and the week, like its
//! people's days: each is asked by one of its residents, by name, and speaks of real
//! places about it, a hill or a river things were lost by, a settlement a letter is to
//! go to. Reading the board writes them in the journal as offered. They are the start of
//! errands (#46): [`Request`] says what each asks in a form a quest can take up, and
//! [`for_week`] lists a settlement's for any week.

use glam::Vec2;
use worldgen::{
    names::PlaceKind,
    noise::hash3,
    settlements::{
        Kind, Region,
        calendar::{self, Festival},
        special::Guild,
    },
};

use super::{
    day,
    town::{Role, Town},
};
use crate::{people::Trade, reading::Page};

/// What someone asks.
#[derive(Clone, Debug, PartialEq)]
pub enum Errand {
    /// So many of something brought to them.
    Gather { what: &'static str, count: u32 },
    /// Something of theirs lost, last seen toward a named place about the settlement.
    Lost { what: &'static str, near: String },
    /// Something to be taken to someone of theirs in another settlement: its index in
    /// `Network::settlements` and its name.
    Carry { what: &'static str, kin: &'static str, to: usize, town: String },
}

/// A request pinned to a tavern's board.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    /// How the journal knows it: the settlement, the week and which of the week's.
    pub key: String,
    /// Who asks: their number among the settlement's residents, and their name.
    pub who: usize,
    pub name: String,
    pub errand: Errand,
    /// How it reads on the board.
    pub text: String,
}

/// Which week day `day` falls in (see `villagers::day_number`): the board is changed at
/// the start of each.
pub fn week_of(day: i64) -> i64 {
    day.div_euclid(day::WEEK)
}

/// The requests on `town`'s board in `week`: one or two, from different people.
pub fn for_week(town: &Town, week: i64) -> Vec<Request> {
    let seed = town.seed ^ 0x40b1_ce5d;
    let h = |a: i32, b: i32| hash3(seed, week as i32, a, b);
    // The grown, who would pin something up: not the children.
    let asking: Vec<usize> = town.residents.iter().filter(|r| r.role != Role::Child).map(|r| r.n).collect();
    if asking.is_empty() {
        return Vec::new();
    }
    let many = if h(0, 0) % 3 == 0 { 1 } else { 2 };
    let mut out: Vec<Request> = Vec::new();
    for i in 0..many.min(asking.len()) {
        let mut pick = h(1, i as i32) as usize % asking.len();
        while out.iter().any(|r| r.who == asking[pick]) {
            pick = (pick + 1) % asking.len();
        }
        let r = &town.residents[asking[pick]];
        let name = r.person.name.clone();
        let roll = h(2, i as i32);
        // The kinds of errand each week in turn, so that two on the board differ.
        let first = (h(3, 0) % 3) as usize;
        let errand = match (first + i) % 3 {
            0 => gather(town.region, r.person.trade, roll),
            1 => lost(town, roll),
            _ => carry(town, roll),
        }
        .or_else(|| gather(town.region, r.person.trade, roll))
        .expect("something is always wanted");
        let text = say(&errand, &name, r.person.trade, roll);
        out.push(Request { key: format!("{},{},{},{week},{i}", town.cell.0, town.cell.1, town.k), who: r.n, name, errand, text });
    }
    out
}

const COUNTS: [&str; 9] = ["", "one", "two", "three", "four", "five", "six", "seven", "eight"];

/// Something someone of `trade` in `region` would ask to have brought.
fn gather(region: Region, trade: Trade, roll: u32) -> Option<Errand> {
    let wet = matches!(region, Region::Wetland | Region::Temperate | Region::Rainforest);
    let what = match trade {
        Trade::Weaver if wet => "bundles of reed, cut green",
        Trade::Weaver => "bundles of willow withies",
        Trade::Smith => "sacks of charcoal",
        Trade::Beekeeper => "armfuls of heather",
        Trade::Herder => "sheaves of hay",
        Trade::Innkeeper => ["baskets of mushrooms", "bundles of firewood", "strings of trout"][(roll % 3) as usize],
        Trade::Farmer => "baskets of seed potatoes",
        Trade::Mason => "good flat stones for a wall",
        Trade::Fisher => "lengths of good rope",
        _ => match region {
            Region::Desert => ["baskets of dates", "jars of water from the spring", "bundles of palm leaves"][(roll % 3) as usize],
            Region::Northern => ["baskets of cloudberries", "rolls of birch bark", "bundles of firewood"][(roll % 3) as usize],
            Region::Alpine | Region::Upland => ["baskets of bilberries", "bundles of firewood", "armfuls of bracken"][(roll % 3) as usize],
            Region::Rainforest | Region::Wetland => ["bundles of reed", "baskets of wild figs", "lengths of vine"][(roll % 3) as usize],
            _ => ["baskets of blackberries", "bundles of firewood", "baskets of hazelnuts"][(roll % 3) as usize],
        },
    };
    Some(Errand::Gather { what, count: 3 + (roll >> 4) % 6 })
}

/// Something lost about `town`, by a place of its country.
fn lost(town: &Town, roll: u32) -> Option<Errand> {
    let natural = |k: PlaceKind| {
        matches!(k, PlaceKind::Lake | PlaceKind::River | PlaceKind::Bay | PlaceKind::Inlet | PlaceKind::Peak | PlaceKind::Falls | PlaceKind::Tor | PlaceKind::Landmark(_))
    };
    let near: Vec<&String> = town.about.iter().filter(|p| natural(p.1)).map(|p| &p.0).take(4).collect();
    let near = near.get((roll >> 3) as usize % near.len().max(1))?;
    let what = match town.region {
        Region::Desert => ["a white she-camel", "a brown goat", "a blue head-cloth"],
        Region::Northern => ["a reindeer calf", "a grey dog", "a bone-handled knife"],
        Region::Alpine | Region::Upland => ["a black-faced ewe", "a brown goat", "a sheepdog"],
        _ => ["a brown goat", "a speckled hen", "a brass ring"],
    }[(roll % 3) as usize];
    Some(Errand::Lost { what, near: (*near).clone() })
}

/// Something to take to another settlement near `town`.
fn carry(town: &Town, roll: u32) -> Option<Errand> {
    let near: Vec<_> = town.neighbours.iter().take(3).collect();
    let to = near.get((roll >> 5) as usize % near.len().max(1))?;
    let what = ["a letter", "a parcel of seed", "a wrapped shawl", "a letter"][(roll % 4) as usize];
    let kin = ["sister", "brother", "aunt", "son", "daughter", "cousin"][((roll >> 2) % 6) as usize];
    Some(Errand::Carry { what, kin, to: to.k, town: to.name.clone() })
}

/// How `errand` reads, pinned up by `name`.
fn say(errand: &Errand, name: &str, trade: Trade, roll: u32) -> String {
    match errand {
        Errand::Gather { what, count } => {
            let for_ = match trade {
                Trade::Weaver => "for the baskets",
                Trade::Smith => "for the forge",
                Trade::Innkeeper => "for the kitchen",
                Trade::Beekeeper => "for the hives",
                Trade::Herder => "for the beasts",
                Trade::Mason => "for the mending",
                _ => ["before the weather turns", "for the house", "for the winter store"][(roll % 3) as usize],
            };
            format!("Wanted: {} {what}, {for_}. {name} will be glad of them.", COUNTS[*count as usize])
        }
        Errand::Lost { what, near } => format!("Lost: {what}, last seen toward {near}. Whoever comes on it, word to {name}, please."),
        Errand::Carry { what, kin, town, .. } => format!("{} to go to {town}, to my {kin} there, by anyone walking that way. Ask for {name}.", capital(what)),
    }
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

/// The page of the tavern's board at `town` on `day`, with the week's requests.
pub fn board_page(town: &Town, day: i64, year_days: f32) -> (Page, Vec<Request>) {
    let requests = for_week(town, week_of(day));
    let mut page = Page::new(format!("The notice board, {}", town.name));
    if let Some(word) = festival_word(day, year_days) {
        page.lines.push((String::new(), word));
    }
    if town.market_on(day) {
        page.lines.push((String::new(), "Market today on the green.".into()));
    }
    for r in &requests {
        page.lines.push((String::new(), r.text.clone()));
    }
    if requests.is_empty() {
        page.lines.push((String::new(), "Nothing pinned up this week.".into()));
    }
    page.foot = Some("Written in the journal as offered.".into());
    (page, requests)
}

/// When day `n` days from today is, in words.
fn days_off(n: i64) -> String {
    match n {
        0 => "today".into(),
        1 => "tomorrow".into(),
        n => format!("in {n} days"),
    }
}

/// What a guild is, as its board in the guild house tells it.
pub fn about(g: Guild) -> &'static str {
    match g {
        Guild::Weavers => {
            "They cut willow and reed by the water, soak them and weave them into baskets, creels and fish traps, hurdles for the folds and mats for the floor. Each weaver's work is their own, and they know it again wherever they see it."
        }
        Guild::Masons => {
            "They quarry and dress the stone the houses and bridges stand on, and cut the old marks into standing stones and cairns: for water, for shelter, for danger, for a good view and for the way home."
        }
        Guild::Painters => {
            "They grind their colours from what the land gives, ochre from the banks, charcoal from the fire, the juice of petals, and paint the boards of shrines, the signs over doors and the rims of baskets."
        }
        Guild::Merchants => {
            "They keep the market days of the country about and carry goods between them. Nothing is sold for coin: a thing is traded for a thing, or given."
        }
        Guild::Wayfarers => {
            "They walk the roads in every weather and keep the charts of the country: every settlement, track and ford within a long day's walk. Anyone who asks may have theirs marked from the charts."
        }
    }
}

/// The page of the guild house's board at `town`, read on `today`: each guild and
/// what it is, with the market days the merchants keep and what the wayfarers mark on
/// the map: `charted` names the settlements newly marked, None if they were before.
pub fn guild_page(town: &Town, guilds: &[Guild], today: i64, charted: Option<&[String]>) -> Page {
    let mut page = Page::new(format!("The guild house, {}", town.name));
    for &g in guilds {
        let mut text = about(g).to_string();
        match g {
            Guild::Merchants => {
                let mut days = Vec::new();
                let next = |kind: Kind, seed: u32| (0..60).find(|&n| calendar::is_market_day_of(kind, seed, today + n));
                if let Some(n) = next(town.kind, town.seed).filter(|_| !town.market.is_empty()) {
                    days.push(format!("here {}", days_off(n)));
                }
                for nb in town.neighbours.iter().filter(|nb| calendar::market_every(nb.kind) > 0).take(4) {
                    if let Some(n) = next(nb.kind, nb.seed) {
                        days.push(format!("at {} {}", nb.name, days_off(n)));
                    }
                }
                if !days.is_empty() {
                    text += &format!(" Next market: {}.", days.join("; "));
                }
            }
            Guild::Wayfarers => match charted {
                _ if town.neighbours.is_empty() => text += " There is no other settlement within a long day's walk of here.",
                None => text += " The settlements of the country about are marked on your map from their charts.",
                Some(names) => text += &format!(" They mark on your map {}.", list(names)),
            },
            _ => {}
        }
        page.lines.push((g.name().into(), text));
    }
    page
}

/// "A", "A and B", "A, B and C".
fn list(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// What the board says of a festival on `day`, if there is one.
fn festival_word(day: i64, year_days: f32) -> Option<String> {
    calendar::festival_on(day, year_days).map(|f| match f {
        Festival::Midsummer => "Midsummer: garlands on every door.".to_string(),
        Festival::Midwinter => "Midwinter: lanterns lit at dusk.".to_string(),
    })
}

/// Whether a board facing `facing` at `at` (continent-local) is before `eye` and looked
/// at along `forward`, near enough to read.
pub fn readable(at: glam::Vec3, facing: Vec2, eye: glam::Vec3, forward: glam::Vec3, reach: f32) -> bool {
    let to = at - eye;
    let d = to.length();
    let flat = Vec2::new(-to.x, -to.z);
    d < reach && to.y.abs() < 2.5 * worldgen::scale::m(1.0) && flat.dot(facing) > 0.0 && to.normalize_or_zero().dot(forward) > 0.6
}


