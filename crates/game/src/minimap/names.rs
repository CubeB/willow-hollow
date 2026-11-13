//! The names on the large map: which places are named at which zoom, how large, and
//! which give way when two would lie over one another.
//!
//! A name belongs to a place, and is drawn upright over it however the map is turned.
//! The greater the place, the larger its name and the further out it shows: a region's
//! across its country, then ranges, islands, lakes, rivers, peaks and bays, then the
//! small places (falls, tors and what people built), which show only close in. Where
//! two names would overlap, the lesser is left out.

use glam::Vec2;
use worldgen::names::PlaceKind;

/// How much a kind of place matters on the map.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Rank {
    /// Falls, tors, and what people built.
    Small,
    /// Ranges, islands, lakes, rivers, peaks, bays.
    Feature,
    Region,
}

pub fn rank(kind: PlaceKind) -> Rank {
    match kind {
        PlaceKind::Region => Rank::Region,
        PlaceKind::Range | PlaceKind::Fault | PlaceKind::Rift | PlaceKind::Island | PlaceKind::Lake | PlaceKind::River | PlaceKind::Bay | PlaceKind::Inlet | PlaceKind::Peak => {
            Rank::Feature
        }
        PlaceKind::Falls | PlaceKind::Landmark(_) | PlaceKind::Hamlet | PlaceKind::Town | PlaceKind::Tor => Rank::Small,
    }
}

/// Whether a kind of place is water, whose name is written in blue.
pub fn watery(kind: PlaceKind) -> bool {
    matches!(kind, PlaceKind::Lake | PlaceKind::River | PlaceKind::Bay | PlaceKind::Inlet | PlaceKind::Falls)
}

/// How strongly a name of this rank and `importance` (the gazetteer's) shows on a map
/// `span` metres across: 0 for not at all, 1 in full, fading between over a little zoom.
pub fn fade(rank: Rank, importance: f32, span: f32) -> f32 {
    let smooth = |a: f32, b: f32, x: f32| {
        let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    match rank {
        // Close in, the map is all one region, and its name is at the top.
        Rank::Region => smooth(1100.0, 1700.0, span),
        Rank::Feature => {
            // The greater the place, the further out its name is kept.
            let limit = 2600.0 * importance.clamp(3.0, 16.0).powf(0.72);
            1.0 - smooth(limit, limit * 1.3, span)
        }
        Rank::Small => 1.0 - smooth(2600.0 + 450.0 * importance.min(3.0), 3400.0 + 600.0 * importance.min(3.0), span),
    }
}

/// How tall a name is written, in pixels on a window 720 high.
pub fn size(rank: Rank, importance: f32) -> f32 {
    match rank {
        Rank::Region => 19.0,
        Rank::Feature => 13.5 + 3.0 * ((importance - 4.0) / 10.0).clamp(0.0, 1.0),
        Rank::Small => 12.0,
    }
}

/// A region's name as an atlas has it: in capitals, spaced out.
pub fn spaced(name: &str) -> String {
    let mut out = String::with_capacity(name.len() * 2);
    for (k, c) in name.chars().enumerate() {
        if k > 0 {
            out.push(' ');
        }
        out.extend(c.to_uppercase());
    }
    out
}

/// How wide `text` is when written `size` pixels tall, about: the map lays its names
/// out before anything measures them, so this errs a little wide.
pub fn width(text: &str, size: f32) -> f32 {
    let em: f32 = text
        .chars()
        .map(|c| match c {
            ' ' => 0.27,
            'i' | 'j' | 'l' | 't' | 'f' | 'r' | '\'' | '.' | ',' | 'I' => 0.32,
            'm' | 'w' | 'M' | 'W' => 0.84,
            c if c.is_uppercase() || c.is_ascii_digit() => 0.66,
            _ => 0.54,
        })
        .sum();
    em * size
}

/// A name that may be drawn.
#[derive(Clone, Debug)]
pub struct Name {
    pub text: String,
    /// Where its middle goes, in pixels, and where it goes instead if there is no room
    /// for it there.
    pub at: Vec2,
    pub or: Option<Vec2>,
    /// How tall it is written, in pixels.
    pub size: f32,
    /// Which names it gives way to: the higher is kept.
    pub weight: f32,
    /// sRGB and how strongly it shows.
    pub color: [u8; 4],
}

impl Name {
    /// The rectangle it fills: its low and high corners.
    pub fn rect(&self) -> (Vec2, Vec2) {
        self.rect_at(self.at)
    }

    fn rect_at(&self, at: Vec2) -> (Vec2, Vec2) {
        let half = Vec2::new(width(&self.text, self.size), self.size * 1.2) * 0.5;
        (at - half, at + half)
    }
}

fn overlap(a: (Vec2, Vec2), b: (Vec2, Vec2), pad: f32) -> bool {
    a.0.x - pad < b.1.x && b.0.x - pad < a.1.x && a.0.y - pad < b.1.y && b.0.y - pad < a.1.y
}

/// The names that are drawn, of all that could be: the weightiest first, each kept only
/// if it lies wholly within `within` (low and high corners), clear by `pad` pixels of
/// every name already kept and of the rectangles in `taken` (what else is written on
/// the screen), where it would go or else where it may go instead (it comes back with
/// `at` where it was put). Names of equal weight keep the order they were given in, so
/// the choice does not flicker.
pub fn declutter(mut names: Vec<Name>, within: (Vec2, Vec2), taken: &[(Vec2, Vec2)], pad: f32) -> Vec<Name> {
    names.sort_by(|a, b| b.weight.total_cmp(&a.weight));
    let mut kept: Vec<Name> = Vec::new();
    let mut rects: Vec<(Vec2, Vec2)> = taken.to_vec();
    for mut name in names {
        let fits = |at: &Vec2| {
            let r = name.rect_at(*at);
            r.0.x >= within.0.x && r.0.y >= within.0.y && r.1.x <= within.1.x && r.1.y <= within.1.y && !rects.iter().any(|&other| overlap(r, other, pad))
        };
        let Some(at) = [Some(name.at), name.or].into_iter().flatten().find(fits) else { continue };
        name.at = at;
        rects.push(name.rect());
        kept.push(name);
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;
    use worldgen::features::landmarks::Landmark;

    fn name(text: &str, x: f32, y: f32, weight: f32) -> Name {
        Name { text: text.into(), at: Vec2::new(x, y), or: None, size: 14.0, weight, color: [255; 4] }
    }

    const SCREEN: (Vec2, Vec2) = (Vec2::ZERO, Vec2::new(1280.0, 720.0));

    #[test]
    fn no_two_names_kept_overlap_and_the_weightier_is_the_one_kept() {
        // A crowd of names about one spot, and some well apart.
        let mut names = Vec::new();
        for k in 0..40 {
            let (x, y) = (300.0 + (k % 8) as f32 * 23.0, 200.0 + (k / 8) as f32 * 9.0);
            names.push(name(&format!("Place {k}"), x, y, (k * 7 % 40) as f32));
        }
        names.push(name("Far Peak", 900.0, 500.0, 0.5));
        names.push(name("Lone Tarn", 1000.0, 100.0, 0.2));
        let kept = declutter(names.clone(), SCREEN, &[], 4.0);
        assert!(kept.len() >= 3 && kept.len() < names.len(), "{} kept", kept.len());
        for (i, a) in kept.iter().enumerate() {
            for b in &kept[i + 1..] {
                assert!(!overlap(a.rect(), b.rect(), 4.0), "{} over {}", a.text, b.text);
            }
        }
        // The weightiest of all is kept, and those with nothing near them.
        let best = names.iter().max_by(|a, b| a.weight.total_cmp(&b.weight)).expect("names");
        assert!(kept.iter().any(|n| n.text == best.text));
        assert!(kept.iter().any(|n| n.text == "Far Peak") && kept.iter().any(|n| n.text == "Lone Tarn"));
        // Every name left out lies over one that is weightier, or as weighty and earlier.
        for n in names.iter().filter(|n| !kept.iter().any(|k| k.text == n.text)) {
            assert!(kept.iter().any(|k| k.weight >= n.weight && overlap(k.rect(), n.rect(), 4.0)), "{} was left out for nothing", n.text);
        }
    }

    #[test]
    fn names_keep_within_the_screen_and_clear_of_what_is_written_there() {
        let names = vec![name("Half Off The Edge", 20.0, 300.0, 9.0), name("Under The Title", 1100.0, 40.0, 9.0), name("In The Open", 600.0, 400.0, 1.0)];
        let title = (Vec2::new(1000.0, 20.0), Vec2::new(1270.0, 70.0));
        let kept = declutter(names, SCREEN, &[title], 4.0);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].text, "In The Open");
    }

    #[test]
    fn a_name_with_no_room_goes_where_it_may_go_instead() {
        let mut second = name("Marren's Hut", 420.0, 300.0, 1.0);
        second.or = Some(Vec2::new(420.0, 330.0));
        let kept = declutter(vec![name("Mount Brae", 400.0, 300.0, 5.0), second], SCREEN, &[], 4.0);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[1].at, Vec2::new(420.0, 330.0));
        assert!(!overlap(kept[0].rect(), kept[1].rect(), 4.0));
    }

    #[test]
    fn the_same_names_are_kept_whatever_order_they_come_in_unless_they_tie() {
        let names = vec![name("Alder Water", 400.0, 300.0, 3.0), name("Mount Brae", 430.0, 304.0, 5.0), name("Cold Beck", 800.0, 300.0, 2.0)];
        let mut turned = names.clone();
        turned.reverse();
        let texts = |v: Vec<Name>| {
            let mut t: Vec<String> = v.into_iter().map(|n| n.text).collect();
            t.sort();
            t
        };
        assert_eq!(texts(declutter(names, SCREEN, &[], 4.0)), texts(declutter(turned, SCREEN, &[], 4.0)));
    }

    #[test]
    fn greater_places_are_named_larger_and_from_further_out() {
        assert!(rank(PlaceKind::Region) > rank(PlaceKind::Peak) && rank(PlaceKind::Lake) > rank(PlaceKind::Landmark(Landmark::Hut)));
        assert!(size(Rank::Region, 30.0) > size(Rank::Feature, 14.0));
        assert!(size(Rank::Feature, 14.0) > size(Rank::Feature, 4.0));
        assert!(size(Rank::Feature, 4.0) > size(Rank::Small, 1.0));
        // Close in: the small places and the features, not the region.
        assert_eq!(fade(Rank::Small, 1.0, 700.0), 1.0);
        assert_eq!(fade(Rank::Feature, 6.0, 700.0), 1.0);
        assert_eq!(fade(Rank::Region, 30.0, 700.0), 0.0);
        // Far out: the region, and only the great features.
        assert_eq!(fade(Rank::Region, 30.0, 20000.0), 1.0);
        assert_eq!(fade(Rank::Small, 3.0, 20000.0), 0.0);
        assert_eq!(fade(Rank::Feature, 4.0, 20000.0), 0.0);
        assert!(fade(Rank::Feature, 16.0, 14000.0) > 0.9);
        // A name comes and goes over a little zoom, not all at once.
        let mid = (1..200).map(|k| fade(Rank::Small, 1.0, 2500.0 + k as f32 * 10.0)).filter(|&f| f > 0.05 && f < 0.95).count();
        assert!(mid > 10, "{mid}");
    }

    #[test]
    fn a_region_is_written_in_spaced_capitals_and_wider_text_is_reckoned_wider() {
        assert_eq!(spaced("Rendale"), "R E N D A L E");
        assert!(width("Wenna's Lodge", 14.0) > width("Tor", 14.0));
        assert!(width("Tor", 20.0) > width("Tor", 14.0));
        // About half its height to a letter.
        let w = width("Greymere Landing", 10.0);
        assert!((60.0..110.0).contains(&w), "{w}");
    }
}
