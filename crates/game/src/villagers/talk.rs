//! What someone says when the player greets them (#204): a short, kind line from
//! lines.txt, naming the places they know (their own settlement, the next one along, the
//! river, the village tree) and the season.

use worldgen::noise::hash2;

use super::town::{Resident, Role, Town};
use crate::people::{Age, Trade};

const LINES: &str = include_str!("lines.txt");

/// The lines of one section of lines.txt.
fn section(name: &str) -> Vec<&'static str> {
    let mut on = false;
    let mut out = Vec::new();
    for line in LINES.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(head) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            on = head == name;
        } else if on {
            out.push(line);
        }
    }
    out
}

/// The season of `date` (0 to 1 from the spring equinox), in a word.
pub fn season(date: f32) -> &'static str {
    ["spring", "summer", "autumn", "winter"][((date.rem_euclid(1.0) * 4.0) as usize).min(3)]
}

/// The part of the day `hour` is in, as it is said in a greeting.
pub fn part_of_day(hour: f32) -> &'static str {
    match hour {
        h if h < 12.0 => "morning",
        h if h < 17.5 => "afternoon",
        _ => "evening",
    }
}

/// Fills the slots of `line`, or None if it has one the place has nothing for.
fn fill(line: &str, town: &Town, r: &Resident, hour: f32, date: f32) -> Option<String> {
    let mut out = String::new();
    let mut rest = line;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let close = rest[open..].find('}')? + open;
        let word: String = match &rest[open + 1..close] {
            "town" => town.name.clone(),
            "neighbour" => town.near.clone()?,
            "river" => town.river.clone()?,
            "season" => season(date).into(),
            "hamlet" => town.hamlets.get(r.from?)?.name.clone(),
            "time" => part_of_day(hour).into(),
            "trade" => match r.person.trade {
                Trade::None | Trade::Traveller => return None,
                t => t.word().into(),
            },
            _ => return None,
        };
        // "the {river}" of a river whose name has its own article: "the Nantdur", not
        // "the The Nantdur".
        let word = match word.strip_prefix("The ") {
            Some(bare) if out.to_lowercase().ends_with("the ") => bare.to_string(),
            _ => word,
        };
        out.push_str(&word);
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    // A line beginning with a slot begins with its capital.
    let mut chars = out.chars();
    Some(match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => out,
    })
}

/// One of `lines`, picked by `pick`, with its slots filled: the first that can be from
/// there on.
fn one(lines: &[&str], pick: u32, town: &Town, r: &Resident, hour: f32, date: f32) -> Option<String> {
    let n = lines.len();
    (0..n).find_map(|k| fill(lines[(pick as usize + k) % n.max(1)], town, r, hour, date))
}

/// What `r` says on being greeted for the `met`th time (from 0) at `hour` on `date`.
pub fn greeting(town: &Town, r: &Resident, met: u32, hour: f32, date: f32) -> String {
    let h = |k: i32| hash2(r.person.seed, met as i32, k);
    let open = match (r.person.age, met) {
        (Age::Child, _) => "open child",
        (_, 0) if r.role == Role::Visitor => "open visitor",
        (_, 0) => "open first",
        (_, 1..=3) => "open again",
        _ => "open often",
    };
    // What they talk of: anyone of the places about, and their own concerns.
    let mut about: Vec<&str> = vec!["remark season", "remark neighbour", "remark river", "remark tree"];
    match r.role {
        Role::Child => about = vec!["remark child", "remark child", "remark tree", "remark neighbour"],
        Role::Elder => about.extend(["remark elder", "remark elder"]),
        Role::Worker | Role::Seller | Role::Innkeeper => about.push("remark trade"),
        Role::Homemaker => {}
        Role::Visitor => about = vec!["remark visitor", "remark visitor", "remark season"],
    }
    let start = h(2) as usize % about.len();
    let remark = (0..about.len()).find_map(|k| one(&section(about[(start + k) % about.len()]), h(3), town, r, hour, date));
    let opening = one(&section(open), h(1), town, r, hour, date).unwrap_or_else(|| format!("Welcome to {}.", town.name));
    match remark {
        Some(remark) => format!("{opening} {remark}"),
        None => opening,
    }
}

/// Every section the greetings use, for checking lines.txt.
#[cfg(test)]
pub const SECTIONS: [&str; 13] = [
    "open first",
    "open again",
    "open often",
    "open child",
    "remark river",
    "remark neighbour",
    "remark tree",
    "remark season",
    "remark trade",
    "remark child",
    "remark elder",
    "open visitor",
    "remark visitor",
];

#[cfg(test)]
pub fn lines_of(name: &str) -> Vec<&'static str> {
    section(name)
}
