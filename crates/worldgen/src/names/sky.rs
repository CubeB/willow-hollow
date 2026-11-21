//! The names of the stars: what the people of a world call the figures they see in its
//! night sky (#37).
//!
//! The sky is the same over the whole of a world, so it has one tongue, chosen by the
//! world's seed, and its figures are named as its places are: some plainly in English
//! ("The Heron", "The Lame Shepherd"), some after someone of the tongue ("Sigrid's
//! Spindle"), some with a word of the tongue ("The Harp of Haldvik"). Nothing here is
//! read by the world's generation, so it changes no part of the world.

use super::{UNFORTUNATE, make_root, pick, words::words};
use crate::noise::hash2;

/// What the figures are seen as: things of the country and of the hearth.
const FIGURES: &[&str] = &[
    "Heron", "Plough", "Harp", "Hare", "Lantern", "Weaver", "Shepherd", "Boat", "Crown", "Kettle", "Hound", "Ladder",
    "Sickle", "Swan", "Antlers", "Wheel", "Spindle", "Bell", "Ram", "Otter", "Owl", "Cradle", "Bow", "Gate", "Key",
    "Stag", "Salmon", "Fiddle", "Loom", "Mare", "Goose", "Hearth", "Wren", "Comb", "Anvil", "Coracle", "Crook",
];
/// What is said of them.
const ADJECTIVES: &[&str] =
    &["Long", "Little", "Lame", "Sleeping", "Old", "Lost", "Wandering", "Patient", "Drowned", "Weary", "Hidden", "Broken", "Bright"];

/// The tongue the stars of world `seed` are named in, of those in `words.txt`.
pub fn tongue(seed: u32) -> &'static str {
    let tongues = &words().tongues;
    tongues.get(hash2(seed, 0x05c7, 0) as usize % tongues.len().max(1)).map_or("old", |t| t.name.as_str())
}

/// A name for the `k`th figure of the sky of world `seed`, in `tongue_name`. Different
/// `k` mostly give different names; the caller keeps the ones it uses apart.
pub fn constellation(seed: u32, k: u32, tongue_name: &str) -> String {
    let w = words();
    let t = &w.tongues[w.tongue_named(tongue_name)];
    let r = |n: i32| hash2(seed ^ 0x57a2_0c11, k as i32, n);
    let figure = pick(FIGURES, r(1)).unwrap_or("Heron");
    match r(2) % 10 {
        0..=2 => format!("The {figure}"),
        3..=5 => format!("The {} {figure}", pick(ADJECTIVES, r(3)).unwrap_or("Old")),
        6 | 7 => match pick(&t.given, r(4)) {
            Some(given) => format!("{given}'s {figure}"),
            None => format!("The {figure}"),
        },
        _ => {
            let (root, _) = make_root(t, r(5));
            if UNFORTUNATE.iter().any(|bad| root.to_lowercase().contains(bad)) {
                format!("The {figure}")
            } else {
                format!("The {figure} of {root}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stars_are_named_the_same_every_time_and_in_a_tongue_of_the_world() {
        let names: Vec<String> = (0..12).map(|k| constellation(7, k, tongue(7))).collect();
        assert_eq!(names, (0..12).map(|k| constellation(7, k, tongue(7))).collect::<Vec<_>>());
        assert!(names.iter().all(|n| n.len() > 4 && !n.contains('{')), "{names:?}");
        let mut apart = names.clone();
        apart.sort();
        apart.dedup();
        assert!(apart.len() >= 8, "{names:?}");
        // Another world names its sky otherwise.
        assert_ne!(names, (0..12).map(|k| constellation(8, k, tongue(8))).collect::<Vec<_>>());
        assert!(words().tongues.iter().any(|t| t.name == tongue(7)));
    }
}
