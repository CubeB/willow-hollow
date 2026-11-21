//! What is pinned on a settlement's boards (#207): on the tavern's, a request or two a
//! week from its people, and on a guild house's, what each of its guilds is.
//!
//! A week's requests are a pure function of the settlement and the week, like its
//! people's days: each is asked by one of its residents, by name, and speaks of real
//! places about it, a hill or a river things were lost by, a settlement a letter is to
//! go to. Reading the board writes them in the journal as offered. They are the start of
//! errands (#46): [`Request`] says what each asks in a form a quest can take up, and
//! [`for_week`] lists a settlement's for any week.
//!
//! A guild house's masons know the quarries worked within a long day's walk, the stone
//! each gives and who their master is, and mark the quarries on the map as the wayfarers
//! mark the settlements ([`quarry_chart`]).

use glam::{DVec2, Vec2};
use voxel::Voxel;
use worldgen::{
    continent::Continent,
    materials as mat,
    names::PlaceKind,
    noise::hash3,
    scale::VOXELS_PER_METRE,
    settlements::{
        Kind, Region,
        calendar::{self, Festival},
        special::Guild,
    },
    surface::local_rock,
};

use super::{
    day,
    town::{Resident, Role, Town},
};
use crate::{
    people::{Age, Trade},
    reading::Page,
};

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
    let asking: Vec<usize> = town.residents.iter().filter(|r| !matches!(r.role, Role::Child | Role::Visitor)).map(|r| r.n).collect();
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
/// the map: `charted` names the settlements newly marked, None if they were before. The
/// masons say which quarries they work and the stone of each, and who their master is.
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
            Guild::Masons => text += &masons(town),
            _ => {}
        }
        page.lines.push((g.name().into(), text));
    }
    page
}

/// A quarry the masons of a guild house know of (#207).
#[derive(Clone, Debug, PartialEq)]
pub struct Pit {
    /// The settlement that works it, by index in `Network::settlements`, and its name.
    pub owner: usize,
    pub town: String,
    /// The middle of its floor, continent-local voxels, where the map marks it; and how far
    /// that is from the middle of the settlement it is known from.
    pub at: Vec2,
    pub far: f32,
    /// The stone it gives, in words: the bed its floor is cut to first, then the others
    /// up its faces.
    pub stone: Vec<&'static str>,
}

/// How far about a guild house its masons count the quarries as their own: a long day's
/// walk, the country its wayfarers chart.
pub const QUARRY_REACH: f32 = super::town::NEIGHBOURS;

/// How far round a quarry it counts as marked on the map.
const MARKED: f32 = worldgen::scale::m(30.0);

/// Every quarry of continent `c`, of the world of `world_seed`, as known from `at` (the
/// middle of a settlement, continent-local): nearest first.
pub fn quarries_about(c: &Continent, world_seed: u32, at: Vec2) -> Vec<Pit> {
    let mut out: Vec<Pit> = c
        .land
        .quarries
        .iter()
        .map(|q| {
            // The rock of the ground the pit is cut into, as the quarry itself is built of
            // it: the bed of its floor, then each bed up the faces as deep as it was cut.
            let biome = c.land.farms[q.owner].biome;
            let mut stone = Vec::new();
            for y in q.floor.y - 1..q.floor.y + q.depth.max(1) {
                let word = stone_word(local_rock(world_seed, biome, q.floor.x, y, q.floor.z));
                if !stone.contains(&word) && stone.len() < 3 {
                    stone.push(word);
                }
            }
            let mid = Vec2::new(q.floor.x as f32 + 0.5, q.floor.z as f32 + 0.5);
            let town = c.trails.names.settlement(q.owner).map_or_else(|| "a settlement nearby".to_string(), |p| p.name.clone());
            Pit { owner: q.owner, town, at: mid, far: mid.distance(at), stone }
        })
        .collect();
    out.sort_by(|a, b| a.far.total_cmp(&b.far));
    out
}

/// What a mason calls each rock the ground is made of (see `surface::local_rock`).
const STONES: [(Voxel, &str); 13] = [
    (mat::STONE, "grey limestone"),
    (mat::ROCK_DARK, "dark whinstone"),
    (mat::ROCK_PALE, "pale freestone"),
    (mat::ROCK_BROWN, "brown gritstone"),
    (mat::SANDSTONE, "sandstone"),
    (mat::SANDSTONE_RED, "red sandstone"),
    (mat::SANDSTONE_DARK, "brown sandstone"),
    (mat::RED_ROCK, "red stone"),
    (mat::RED_ROCK_DARK, "dark red stone"),
    (mat::GRANITE, "granite"),
    (mat::GRANITE_GREY, "grey granite"),
    (mat::GRANITE_PALE, "pale granite"),
    (mat::BASALT, "black basalt"),
];

/// The mason's word for rock `v`.
pub fn stone_word(v: Voxel) -> &'static str {
    STONES.iter().find(|s| s.0 == v).map_or("stone", |s| s.1)
}

/// The way `d` points, as a compass word: east along +x, north toward -z.
fn compass(d: Vec2) -> &'static str {
    let k = (d.y.atan2(d.x) / std::f32::consts::FRAC_PI_4).round() as i32;
    ["east", "south-east", "south", "south-west", "west", "north-west", "north", "north-east"][k.rem_euclid(8) as usize]
}

/// A distance of `voxels`, in words: to the nearest fifty metres, or half kilometre.
fn how_far(voxels: f32) -> String {
    let metres = voxels / VOXELS_PER_METRE;
    if metres < 950.0 {
        return format!("{} m", ((metres / 50.0).round() as i32).max(1) * 50);
    }
    let halves = (metres / 500.0).round() as i32;
    if halves % 2 == 0 { format!("{} km", halves / 2) } else { format!("{}.5 km", halves / 2) }
}

/// The guild's master mason in `town`: the eldest of its masons, the first of them among
/// its people where two are as old.
pub fn master(town: &Town) -> Option<&Resident> {
    town.residents.iter().filter(|r| r.person.trade == Trade::Mason && r.role != Role::Child).min_by_key(|r| (r.person.age != Age::Elder, r.n))
}

/// The quarries within a long day's walk of `town`, nearest first.
fn quarries_near(town: &Town) -> Vec<&Pit> {
    town.quarries.iter().filter(|p| p.far <= QUARRY_REACH).collect()
}

/// What the masons' roll says of the quarries about `town`, of the old walls of an
/// ancient town, and of their master.
fn masons(town: &Town) -> String {
    let middle = Vec2::new(town.centre.x, town.centre.z);
    let whence = |p: &Pit| {
        let way = compass(p.at - middle);
        if p.owner == town.k {
            format!("their own quarry, {} out to the {way}", how_far(p.far))
        } else {
            format!("the quarry of {}, {} to the {way}", p.town, how_far(p.far))
        }
    };
    let gives = |p: &Pit| list(&p.stone.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    let near = quarries_near(town);
    let mut text = match (near.len(), town.quarries.first()) {
        (0, None) => " No stone is quarried anywhere in this country: they build with what the fields and the river beds give, and dress it by hand.".to_string(),
        (0, Some(p)) => format!(" No quarry is worked within a long day's walk of here. The last stone came by cart from {}: {}. That quarry is marked on your map.", whence(p), gives(p)),
        (1, _) => format!(" They work {}, for {}. It is marked on your map.", whence(near[0]), gives(near[0])),
        (n, _) => {
            let each: Vec<String> = near.iter().map(|&p| format!("{}, for {}", whence(p), gives(p))).collect();
            let count = COUNTS.get(n).map_or_else(|| n.to_string(), |c| c.to_string());
            format!(" They work {count} quarries within a long day's walk: {}. Each is marked on your map.", each.join("; "))
        }
    };
    if town.ancient {
        text += " Most of their days go on the old walls, setting back the stones the frost has thrown down and cutting new ones where the old are past mending.";
    }
    match master(town) {
        Some(r) => text += &format!(" Their master is {}.", r.person.name),
        None => {
            if let Some(p) = town.quarries.first().filter(|p| p.owner != town.k) {
                text += &format!(" Their master lives out at {}, by the quarry.", p.town);
            }
        }
    }
    text
}

/// The quarries the masons of `town`'s guild house mark on the map when its board
/// `board` (an index in [`Town::boards`]) is read (#207), as the wayfarers mark the
/// settlements: where each is, in world x and z, and the middle and reach of the country
/// marked, which takes in every one of them. Those within a long day's walk; or where
/// there are none, the one stone was last brought from. None for any other board, a
/// guild house without masons, or a country with no quarry.
pub fn quarry_chart(town: &Town, board: usize) -> Option<(Vec<DVec2>, DVec2, f64)> {
    let (plot, guilds) = town.guilds.as_ref()?;
    if town.boards.get(board)?.0 != *plot || !guilds.contains(&Guild::Masons) {
        return None;
    }
    let world = |p: Vec2| DVec2::new(town.origin.x + p.x as f64, town.origin.z + p.y as f64);
    let near = quarries_near(town);
    // Just the reach they count as theirs, and a voxel over for the rounding of the
    // world's x and z, so that the map takes in exactly the quarries the roll names.
    let (marked, middle, reach) = match near.is_empty() {
        false => (near, world(Vec2::new(town.centre.x, town.centre.z)), QUARRY_REACH + 1.0),
        true => {
            let last = town.quarries.first()?;
            (vec![last], world(last.at), MARKED)
        }
    };
    Some((marked.iter().map(|p| world(p.at)).collect(), middle, reach as f64))
}

/// How the journal knows the masons' marks of the settlement it knows by `key` (see
/// [`super::Reading::key`]), kept beside the wayfarers' charts.
pub fn masons_key(key: &str) -> String {
    format!("{key} masons")
}

/// Whether a chart in the journal is the masons' (see [`masons_key`]).
pub fn by_masons(key: &str) -> bool {
    key.ends_with(" masons")
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, OnceLock};

    use glam::Vec3;
    use worldgen::{
        continent::SimConfig,
        scale::m,
        world::GameWorld,
    };

    use super::*;
    use crate::{
        journal::Journal,
        minimap::{marks, told_of},
    };

    /// A small world with quarries in it (see `farmland::quarry`'s tests).
    const SEED: u32 = 2;

    fn continent() -> &'static Arc<Continent> {
        static C: OnceLock<Arc<Continent>> = OnceLock::new();
        C.get_or_init(|| GameWorld::with_config(SEED, SimConfig { size: 256 }).continent(0, 0))
    }

    /// The settlement with a tavern nearest a quarry, with a guild house of masons and
    /// merchants: its tavern and board stand in for one, as the world is too small for one.
    fn quarrying_town() -> Town {
        static T: OnceLock<Town> = OnceLock::new();
        T.get_or_init(|| {
            let c = continent();
            let net = &c.trails;
            let pits: Vec<Vec2> = c.land.quarries.iter().map(|q| Vec2::new(q.floor.x as f32, q.floor.z as f32)).collect();
            assert!(!pits.is_empty(), "the test world has no quarry");
            let near = |k: usize| pits.iter().map(|p| p.distance(net.settlements[k].at())).fold(f32::MAX, f32::min);
            let mut ks: Vec<usize> = (0..net.settlements.len()).filter(|&k| net.settlements[k].kind >= Kind::Village).collect();
            ks.sort_by(|&a, &b| near(a).total_cmp(&near(b)));
            let mut town = ks
                .into_iter()
                .filter_map(|k| Town::build(c, SEED, (0, 0), k))
                .find(|t| t.tavern.is_some_and(|tavern| t.boards.iter().any(|b| b.0 == tavern)))
                .expect("no settlement with a tavern and a board by it");
            town.guilds = Some((town.tavern.unwrap(), vec![Guild::Masons, Guild::Merchants]));
            town
        })
        .clone()
    }

    /// What the masons' line of the guild house board says.
    fn masons_line(town: &Town) -> String {
        let page = guild_page(town, &[Guild::Masons], 7 * 40 + 1, None);
        page.lines.iter().find(|l| l.0 == Guild::Masons.name()).expect("no masons on the board").1.clone()
    }

    #[test]
    fn the_masons_name_the_quarries_within_reach_and_the_stone_of_each() {
        let c = continent();
        let town = quarrying_town();
        assert_eq!(town.quarries.len(), c.land.quarries.len(), "every quarry is known, near or far");
        assert!(town.quarries.windows(2).all(|w| w[0].far <= w[1].far), "not nearest first");
        let near: Vec<&Pit> = town.quarries.iter().filter(|p| p.far <= QUARRY_REACH).collect();
        assert!(!near.is_empty(), "no quarry within reach of {}", town.name);
        for p in &town.quarries {
            assert!(!p.stone.is_empty() && p.stone.iter().all(|s| STONES.iter().any(|t| t.1 == *s)), "{}: {:?}", p.town, p.stone);
        }
        let line = masons_line(&town);
        for p in &near {
            let whose = if p.owner == town.k { "their own quarry".to_string() } else { format!("the quarry of {}", p.town) };
            assert!(line.contains(&whose), "{line}");
            assert!(p.stone.iter().all(|s| line.contains(s)), "{line}: {:?}", p.stone);
        }
        assert!(line.contains("marked on your map"), "{line}");
        // Only an ancient town's masons keep its old walls.
        let walls = |ancient: bool| masons_line(&Town { ancient, ..town.clone() }).contains("the old walls");
        assert!(walls(true) && !walls(false), "{line}");
        // A quarry beyond a long day's walk is not among them.
        for p in town.quarries.iter().filter(|p| p.far > QUARRY_REACH) {
            assert!(!line.contains(&p.town), "{line}");
        }
        // Their master is the eldest of the settlement's own masons, or with none of its
        // own, lives out by the quarry.
        match master(&town) {
            Some(r) => assert!(line.contains(&format!("Their master is {}.", r.person.name)), "{line}"),
            None => assert!(near[0].owner == town.k || line.contains(&format!("Their master lives out at {}", near[0].town)), "{line}"),
        }
        let mut masons = town.clone();
        let grown: Vec<usize> = (0..masons.residents.len()).filter(|&i| masons.residents[i].role != Role::Child).take(3).collect();
        assert_eq!(grown.len(), 3, "too few grown people");
        for (n, &i) in grown.iter().enumerate() {
            let p = &mut masons.residents[i].person;
            p.trade = Trade::Mason;
            p.age = if n == 1 { Age::Elder } else { Age::Adult };
        }
        let elder = master(&masons).expect("no master among masons");
        assert!(elder.person.trade == Trade::Mason && elder.person.age == Age::Elder, "{:?}", elder.person.name);
        assert!(masons_line(&masons).contains(&format!("Their master is {}.", elder.person.name)), "{}", masons_line(&masons));
    }

    #[test]
    fn with_no_quarry_near_the_masons_say_where_stone_last_came_from_or_that_none_is_cut() {
        let mut town = quarrying_town();
        let mut far = town.quarries[0].clone();
        far.owner = usize::MAX;
        far.town = "Stonebury".into();
        far.far = QUARRY_REACH * 2.0;
        far.at = Vec2::new(town.centre.x, town.centre.z) + Vec2::new(QUARRY_REACH * 2.0, 0.0);
        town.quarries = vec![far.clone()];
        let line = masons_line(&town);
        assert!(line.contains("No quarry is worked within a long day's walk"), "{line}");
        assert!(line.contains("The last stone came by cart from the quarry of Stonebury") && line.contains("to the east"), "{line}");
        // That one quarry, and only it, is marked.
        let which = town.boards.iter().position(|b| b.0 == town.guilds.as_ref().unwrap().0).unwrap();
        let (pits, middle, reach) = quarry_chart(&town, which).expect("nothing marked");
        assert_eq!(pits.len(), 1);
        assert!(pits[0].distance(middle) <= reach && reach < m(100.0) as f64);
        // No quarry in all the country.
        town.quarries.clear();
        let line = masons_line(&town);
        assert!(line.contains("No stone is quarried anywhere in this country"), "{line}");
        assert!(quarry_chart(&town, which).is_none());
    }

    #[test]
    fn the_masons_mark_their_quarries_on_the_map_and_the_journal_keeps_them() {
        let c = continent();
        let town = quarrying_town();
        let (plot, _) = town.guilds.clone().unwrap();
        let which = town.boards.iter().position(|b| b.0 == plot).expect("the guild house has no board");
        // Only the guild house's board, and only with masons.
        for b in (0..town.boards.len()).filter(|&b| b != which) {
            assert!(quarry_chart(&town, b).is_none());
        }
        let mut merchants = town.clone();
        merchants.guilds = Some((plot, vec![Guild::Merchants, Guild::Wayfarers]));
        assert!(quarry_chart(&merchants, which).is_none());

        let (pits, middle, reach) = quarry_chart(&town, which).expect("the masons mark nothing");
        assert_eq!(pits.len(), town.quarries.iter().filter(|p| p.far <= QUARRY_REACH).count());
        // The map's marks of the quarries: those the roll names are known as if visited,
        // and no other.
        let quarried = [(middle, reach)];
        let marks: Vec<marks::Mark> = marks::find(c, SEED).into_iter().filter(|mk| mk.kind == marks::Kind::Quarry).collect();
        assert_eq!(marks.len(), c.land.quarries.len(), "a quarry has no mark");
        for (mk, p) in marks.iter().zip(&c.land.quarries) {
            let named = town.quarries.iter().any(|q| q.far <= QUARRY_REACH && q.at == Vec2::new(p.floor.x as f32 + 0.5, p.floor.z as f32 + 0.5));
            assert_eq!(told_of(mk, false, &[], &quarried), named, "quarry of {} at {}", p.owner, p.floor);
            assert!(!told_of(mk, false, &[], &[]));
            if named {
                assert!(pits.iter().any(|q| q.distance(mk.at) < 1.0), "the roll's quarry is not where the map marks it");
            }
        }
        // A settlement is never taken for a quarry, nor a quarry for a settlement charted.
        let settlement = marks::Mark { kind: marks::Kind::Settlement, at: middle, reach: 1.0, name: None, hint: true, shrine: false };
        assert!(!told_of(&settlement, true, &[], &quarried));
        assert!(!told_of(&marks[0], false, &quarried, &[]));

        // The journal keeps it once, apart from the wayfarers' chart, and the save keeps it.
        let key = masons_key("0,0,4");
        assert!(by_masons(&key) && !by_masons("0,0,4"));
        let mut journal = Journal::new();
        assert!(journal.chart("0,0,4", &town.name, middle, reach));
        assert!(journal.chart_quarries(&key, &town.name, middle, reach));
        assert!(!journal.chart_quarries(&key, &town.name, middle, reach));
        let back = Journal::from_text(&journal.to_text());
        assert_eq!(back.charted(), journal.charted());
        assert_eq!(back.charted().iter().filter(|c| by_masons(&c.key)).count(), 1);
    }

    /// The whole board, read as the player reads it, says what the masons know.
    #[test]
    fn the_guild_house_board_reads_the_masons_roll() {
        let town = quarrying_town();
        let (plot, _) = town.guilds.clone().unwrap();
        let &(_, at, facing) = town.boards.iter().find(|b| b.0 == plot).unwrap();
        let eye = at + Vec3::new(facing.x, 0.0, facing.y) * m(1.4);
        let forward = (at - eye).normalize();
        let mut v = super::super::Villagers::new();
        v.add(town.clone(), 7 * 40 + 1, 0);
        let (which, _) = v.board_in_sight(town.origin + eye.as_dvec3(), forward).expect("the board is not in sight");
        let read = v.read(which, |_| false).unwrap();
        let line = read.page.lines.iter().find(|l| l.0 == Guild::Masons.name()).expect("no masons on the board");
        assert_eq!(line.1, format!("{}{}", about(Guild::Masons), masons(&town)));
        assert!(v.towns().nth(which.0).and_then(|t| quarry_chart(t, which.1)).is_some());
    }
}


