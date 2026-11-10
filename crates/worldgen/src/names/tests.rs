use std::collections::HashSet;

use glam::Vec2;

use super::{Facts, Gazetteer, PlaceKind, Request, words};
use crate::{continent::SimConfig, features::landmarks::Landmark, scale::m, world::GameWorld};

fn world() -> GameWorld {
    GameWorld::with_config(3, SimConfig { size: 256 })
}

#[test]
fn the_word_lists_are_complete() {
    let w = words::words();
    assert!(w.tongues.len() >= 4);
    for t in &w.tongues {
        assert!(!t.first.is_empty() && !t.second.is_empty() && !t.given.is_empty(), "tongue {} is missing parts", t.name);
    }
    let mut kinds: Vec<PlaceKind> = Landmark::ALL.iter().map(|&l| PlaceKind::Landmark(l)).collect();
    kinds.extend([
        PlaceKind::Region,
        PlaceKind::Range,
        PlaceKind::Fault,
        PlaceKind::Rift,
        PlaceKind::Island,
        PlaceKind::Lake,
        PlaceKind::River,
        PlaceKind::Bay,
        PlaceKind::Inlet,
        PlaceKind::Peak,
        PlaceKind::Falls,
        PlaceKind::Hamlet,
        PlaceKind::Town,
    ]);
    for k in kinds {
        assert!(w.kinds.get(k.key()).is_some_and(|k| !k.pattern.is_empty()), "no patterns for {}", k.key());
    }
}

/// Many landmarks in one small region, named from scratch.
fn crowded(seed: u32) -> Gazetteer {
    let mut g = Gazetteer::new(seed);
    g.set_regions(&[(Vec2::ZERO, "north"), (Vec2::splat(m(20000.0)), "vale")]);
    let facts = [
        Facts { tags: vec!["bog", "hill"], residents: Vec::new() },
        Facts { tags: vec!["maple", "lake"], residents: Vec::new() },
        Facts { tags: vec!["taiga"], residents: Vec::new() },
        Facts { tags: vec!["sea", "shore"], residents: vec!["Marren".into()] },
    ];
    for k in 0..240u32 {
        let kind = Landmark::ALL[k as usize % Landmark::ALL.len()];
        let at = Vec2::new((k % 16) as f32 * m(150.0), (k / 16) as f32 * m(150.0));
        g.add(Request {
            kind: PlaceKind::Landmark(kind),
            at,
            radius: kind.radius(),
            line: Vec::new(),
            importance: 1.0,
            facts: &facts[k as usize % facts.len()],
            salt: k.wrapping_mul(2654435761),
        });
    }
    g
}

#[test]
fn names_depend_only_on_the_seed() {
    let names = |g: &Gazetteer| g.places.iter().map(|p| p.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&crowded(7)), names(&crowded(7)));
    assert_ne!(names(&crowded(7)), names(&crowded(8)));
}

#[test]
fn no_two_places_in_a_region_share_a_name() {
    let g = crowded(11);
    let mut seen = HashSet::new();
    for p in &g.places {
        assert!(!p.name.is_empty());
        assert!(seen.insert(p.name.clone()), "{} named twice", p.name);
    }
    println!("{:#?}", g.places.iter().take(40).map(|p| &p.name).collect::<Vec<_>>());
}

#[test]
fn every_place_on_a_continent_is_named_once_and_the_same_each_time() {
    let (a, b) = (world(), world());
    let (ca, cb) = (a.continent(0, 0), b.continent(0, 0));
    let (na, nb) = (&ca.trails.names, &cb.trails.names);
    let names = |g: &Gazetteer| g.places.iter().map(|p| (p.kind, p.name.clone())).collect::<Vec<_>>();
    assert_eq!(names(na), names(nb));
    let mut seen = HashSet::new();
    for p in &na.places {
        assert!(seen.insert(&p.name), "{} named twice", p.name);
    }
    for k in 0..ca.trails.sites.len() {
        assert!(na.site(k).is_some(), "landmark {k} has no name");
    }
    assert!(na.places.iter().any(|p| p.kind == PlaceKind::Region));
    for p in &na.places {
        println!("{:<10} {:<8} {}", p.kind.key(), na.tongue_at(p.at), p.name);
    }
}
