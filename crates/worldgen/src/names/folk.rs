//! People's names: a given name and a family name in the tongue of where they live.
//!
//! Each tongue in `words.txt` lists given names for women and for men, and family names
//! taken from a trade or a place ("Weaver", "of the Ford", "Moen"), some of them made
//! from the tongue's own words as place names are ("Ashford", "Haldvik"). A name is a
//! pure function of a seed and the tongue, so a person is called the same by every
//! player; a household shares one family name, made from the household's seed.

use super::{
    UNFORTUNATE, make_root, pick,
    words::{Tongue, words},
};
use crate::{noise::hash2, settlements::Region};

/// Which list a person's given name comes from.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Sex {
    Female,
    Male,
}

/// Every tongue people can be named in, in the order `words.txt` gives them.
pub fn tongues() -> Vec<&'static str> {
    words().tongues.iter().map(|t| t.name.as_str()).collect()
}

/// The tongue most often spoken where a region's people build, for when nothing better
/// is known (a settlement's own tongue comes from the gazetteer, by its climate).
pub fn tongue_of(region: Region) -> &'static str {
    match region {
        Region::Temperate | Region::Wetland => "old",
        Region::Upland | Region::OldGrowth => "vale",
        Region::Northern | Region::Alpine => "north",
        Region::Desert => "dune",
        Region::Rainforest => "isle",
    }
}

fn tongue(name: &str) -> &'static Tongue {
    let w = words();
    &w.tongues[w.tongue_named(name)]
}

/// A tongue's own list for people, kept with its patterns by key; its place names'
/// given names where it has none.
fn list(t: &'static Tongue, key: &str) -> &'static [String] {
    t.patterns.get(key).map(|v| v.as_slice()).filter(|v| !v.is_empty()).unwrap_or(&t.given)
}

/// A given name for someone of `sex`, from `seed`.
pub fn given_name(seed: u32, tongue_name: &str, sex: Sex) -> &'static str {
    let t = tongue(tongue_name);
    let names = list(t, if sex == Sex::Female { "women" } else { "men" });
    pick(names, hash2(seed, 0x6e61, 1)).unwrap_or("Ash")
}

/// A family name from `seed`: a trade, a place, or a word of the tongue.
pub fn family_name(seed: u32, tongue_name: &str) -> String {
    let t = tongue(tongue_name);
    let patterns = list(t, "family");
    for attempt in 0..16 {
        let r = |k: i32| hash2(seed ^ 0x00fa_3117, attempt, k);
        let Some(pattern) = pick(patterns, r(1)) else { break };
        let (root, stem) = make_root(t, r(2));
        if UNFORTUNATE.iter().any(|bad| root.to_lowercase().contains(bad)) {
            continue;
        }
        let man = pick(list(t, "men"), r(3)).unwrap_or("Ash");
        // Filled by hand rather than as a place name is, which would capitalise the
        // "ap", "al-" and "of the" that open some of these.
        let name = pattern.replace("{root}", &root).replace("{stem}", stem).replace("{man}", man);
        if !name.is_empty() && !name.contains('{') {
            return name;
        }
    }
    make_root(t, seed).0
}

/// A whole name, given and family, both from `seed`. In a household the family name
/// comes from the household instead: [`given_name`] and [`family_name`] apart.
pub fn full_name(seed: u32, tongue_name: &str, sex: Sex) -> String {
    joined(given_name(seed, tongue_name, sex), &family_name(seed, tongue_name))
}

/// A given name and a family name put together.
pub fn joined(given: &str, family: &str) -> String {
    format!("{given} {family}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tongue_has_names_for_people() {
        for t in &words().tongues {
            for key in ["women", "men", "family"] {
                assert!(t.patterns.get(key).is_some_and(|v| v.len() >= 8), "tongue {} has too few {key}", t.name);
            }
        }
    }

    #[test]
    fn a_name_is_the_same_each_time_and_varies_with_the_seed() {
        for t in tongues() {
            let names: Vec<String> = (0..60).map(|s| full_name(s, t, if s % 2 == 0 { Sex::Female } else { Sex::Male })).collect();
            let again: Vec<String> = (0..60).map(|s| full_name(s, t, if s % 2 == 0 { Sex::Female } else { Sex::Male })).collect();
            assert_eq!(names, again);
            let families: std::collections::HashSet<String> = (0..60).map(|s| family_name(s, t)).collect();
            assert!(families.len() >= 12, "{t}: only {} family names in 60", families.len());
            for n in &names {
                assert!(!n.contains('{') && n.split(' ').count() >= 2, "{t}: {n}");
            }
            println!("{t}: {}", names[..10].join(", "));
        }
    }

    #[test]
    fn women_and_men_are_named_from_their_own_lists() {
        let north = tongue("north");
        for s in 0..40 {
            assert!(north.patterns["women"].iter().any(|n| n == given_name(s, "north", Sex::Female)));
            assert!(north.patterns["men"].iter().any(|n| n == given_name(s, "north", Sex::Male)));
        }
    }
}
