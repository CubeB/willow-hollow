//! The calendar: where in the year the world is, as a fraction from 0 to 1.
//!
//! 0 is the spring equinox, 0.25 midsummer, 0.5 the autumn equinox and 0.75 midwinter.
//! The date is a continuous number, never four states, so everything that follows it
//! changes gradually. It advances with the time of day, so a year is a set number of game
//! days, and it is saved with the player's place, so a world is in the same season
//! wherever and whenever it is loaded again.

use std::ops::RangeInclusive;

use glam::{IVec3, Vec3};
use worldgen::biome::Biome;

/// Game days in a year, unless the settings say otherwise.
pub const DEFAULT_YEAR_DAYS: f32 = 24.0;
pub const YEAR_DAYS: RangeInclusive<f32> = 4.0..=240.0;
/// A new world starts in high summer, which is how the world looked before seasons.
pub const DEFAULT_DATE: f32 = 0.32;

/// Named dates, for the command line.
pub fn named(name: &str) -> Option<f32> {
    Some(match name {
        "spring" => 0.08,
        "summer" => DEFAULT_DATE,
        "autumn" | "fall" => 0.58,
        "winter" => 0.86,
        _ => return None,
    })
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Moves a date on by `days` game days.
pub fn advance(date: f32, days: f32, year_days: f32) -> f32 {
    (date + days / year_days.max(1.0)).rem_euclid(1.0)
}

/// How far north the sun stands of the equator through the year: 1 at midsummer, -1 at
/// midwinter. It lengthens the days and raises the noon sun in summer.
pub fn declination(date: f32) -> f32 {
    (date * std::f32::consts::TAU).sin()
}

/// The date by which every broadleaf, maple or not, has dropped its last leaf: early
/// winter, before midwinter (0.75). season.wgsl's `season_leaf` keeps to it.
pub const LEAVES_DOWN: f32 = 0.70;

/// How much of the year's leaf fall is happening now, 0 to 1, peaking in mid autumn and
/// over by `LEAVES_DOWN`. The falling leaf particles follow it. Each tree drops its leaves
/// on its own date (season.wgsl's `season_leaf`), the first from about 0.58 (maples) and
/// the last by 0.70, so the fall is at its height from 0.6 until most trees are bare.
pub fn leaf_fall(date: f32) -> f32 {
    smoothstep(0.50, 0.60, date) * (1.0 - smoothstep(0.665, LEAVES_DOWN, date))
}

/// How wintry the date is, 0 in summer and 1 in deep winter: how far below the snowline
/// cold reaches down the hills (the lying snow itself comes from the weather).
pub fn snow_cover(date: f32) -> f32 {
    smoothstep(0.62, 0.80, date).max(1.0 - smoothstep(0.0, 0.12, date))
}

/// From this warmth up a biome never sees snow: none falls, none lies, and only real high
/// mountains above `Ground::snowline` keep a cap.
pub const NEVER_SNOWS: f32 = 0.9;

/// How warm a biome is: 0.25 for the temperate lowlands, negative for the cold biomes,
/// where winters are long and snowy, and `NEVER_SNOWS` or more where it never snows.
/// weather.rs turns it into a temperature.
pub fn warmth(biome: Biome) -> f32 {
    match biome {
        Biome::SnowyPine => -0.3,
        Biome::Alpine => -0.5,
        Biome::Tundra => -0.35,
        Biome::Taiga => -0.2,
        Biome::Heath | Biome::Grassland | Biome::Deciduous | Biome::OldGrowth | Biome::Ocean => 0.25,
        // Flowering grass below the summit snow: snowed on in winter, green in summer.
        Biome::AlpineMeadow => 0.25,
        // Steppe is the dry, warm side of the heath: snow is rare there.
        Biome::Steppe => 0.45,
        // Woodland is the wet tropics between savanna and rainforest.
        Biome::Woodland => 1.1,
        Biome::Savanna | Biome::Desert | Biome::Rainforest => 1.5,
        // Volcanic islands are warm ground; only their high summits see snow.
        Biome::Volcanic => 1.0,
        // The other newer biomes (mangrove, dune sea, red rock, marsh, bog...) follow their parent.
        b => warmth(b.parent()),
    }
}

/// What the ground carries, for the shaders: snow cover and ice on still water, with how
/// wet it is (the rain's doing, see weather.rs).
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Ground {
    /// Snow at sea level, 0..1. Higher ground has more.
    pub snow: f32,
    /// Ice on water, 0..1.
    pub frozen: f32,
    pub wet: f32,
    /// How much the cold of height adds to the snow on hills, 0..1: winter, in a biome that snows.
    pub highland: f32,
    /// Metres above which summits keep snow all year.
    pub snowline: f32,
}

impl Ground {
    /// The ground at `date` in a biome of this warmth, with `lying` snow and `ice` from the
    /// weather (weather::lying).
    pub fn at(date: f32, warmth: f32, lying: f32, ice: f32, wet: f32) -> Self {
        if warmth >= NEVER_SNOWS {
            return Self { snow: 0.0, frozen: 0.0, wet, highland: 0.0, snowline: Self::snowline(warmth) };
        }
        let highland = snow_cover(date) * ((NEVER_SNOWS - warmth) / (NEVER_SNOWS - 0.25)).clamp(0.0, 1.0);
        // In winter the snowline comes down the hills, as far as `winter_snowline`.
        let summer = Self::snowline(warmth);
        let snowline = summer + (Self::winter_snowline(warmth) - summer) * highland;
        Self { snow: lying.clamp(0.0, 1.0), frozen: ice.clamp(0.0, 1.0), wet, highland, snowline }
    }

    /// The height of the year-round summit snow, metres: lower in the cold, and only on
    /// real high mountains where it never snows below. Biomes already go colder with
    /// height (Alpine is the high summits), so summer snow is kept to those: 320 m in the
    /// temperate lands, clear of their hills and of the alpine meadows at about 300 m;
    /// 170 m on the Alpine summits; 380 m and up where it never snows.
    pub fn snowline(warmth: f32) -> f32 {
        if warmth >= NEVER_SNOWS { 380.0 + (warmth - NEVER_SNOWS) * 150.0 } else { 270.0 + 200.0 * warmth.max(-0.5) }
    }

    /// How far the snowline comes down in deep winter, metres, where it snows at all:
    /// 90 m in the temperate lands, 30 m on the Alpine summits.
    pub fn winter_snowline(warmth: f32) -> f32 {
        70.0 + 80.0 * warmth.max(-0.5)
    }
}

/// The ground everywhere in view: what it carries where the camera is, and in each biome,
/// so that a far cold range is white while the desert in front of it is bare. The sea, and
/// ground whose biome is not known yet, take the camera's.
#[derive(Clone, Copy, Debug)]
pub struct Places {
    pub here: Ground,
    /// Indexed by biome (`Biome as usize`).
    pub biomes: [Ground; Biome::ALL.len()],
}

/// season.wgsl's `season_hash`, bit for bit.
fn hash(p: IVec3) -> f32 {
    let mut h = (p.x as u32)
        .wrapping_mul(374761393)
        .wrapping_add((p.y as u32).wrapping_mul(668265263))
        .wrapping_add((p.z as u32).wrapping_mul(2246822519));
    h = (h ^ (h >> 13)).wrapping_mul(1274126177);
    h ^= h >> 16;
    (h & 0xff_ffff) as f32 / 16777216.0
}

/// One broadleaf leaf's year on this date, as season.wgsl's `Leaf`.
pub struct Leaf {
    /// False once it has fallen, or before it has budded.
    pub keep: bool,
    /// 0..1 through the colours of autumn.
    pub turn: f32,
    /// 1 for a new leaf, easing to 0 as it matures.
    pub bud: f32,
    /// The tree's own random, 0..1.
    pub tree: f32,
    /// The date it falls.
    pub fall_at: f32,
}

/// The leaf at voxel `vox` on a tree tagged `tag` (features::tree_tag, 0 if unknown):
/// season.wgsl's `season_leaf`, so a falling leaf has the colour and the date of the
/// leaves still on its tree. Keep the two in step.
pub fn leaf(s: f32, vox: IVec3, maple: bool, tag: u8) -> Leaf {
    let h = hash(vox);
    let branch = hash(vox.div_euclid(IVec3::splat(3)) + IVec3::new(11, 3, 7));
    let tree = if tag == 0 {
        hash(IVec3::new(vox.x.div_euclid(10), 7, vox.z.div_euclid(10)))
    } else {
        (tag - 1) as f32 / 62.0
    };
    let early = if maple { 0.025 } else { 0.0 };
    let bud_at = 0.02 + 0.10 * tree + 0.03 * h;
    let fall_at = 0.625 + 0.05 * tree + 0.016 * (branch - 0.5) + 0.03 * (h - 0.5) - early;
    let turn_at = 0.47 + 0.06 * tree + 0.02 * (branch - 0.5) - early;
    Leaf {
        // Midsummer: every leaf is on (season_gone).
        keep: (s > 0.27 && s < 0.40) || (s >= bud_at && s < fall_at),
        turn: smoothstep(turn_at, turn_at + 0.10, s),
        bud: 1.0 - smoothstep(bud_at, bud_at + 0.12, s),
        tree,
        fall_at,
    }
}

/// The colour of a broadleaf leaf still on, from its modelled linear colour `c`:
/// season.wgsl's `season_apply` for a deciduous or maple leaf.
pub fn leaf_colour(c: Vec3, leaf: &Leaf, maple: bool, s: f32) -> Vec3 {
    if !maple && s > 0.27 && s < 0.40 {
        return c;
    }
    let mut o = c;
    if maple {
        o = Vec3::new(0.30, 0.95, 0.20) * c.dot(Vec3::new(0.3, 0.59, 0.11)) * 1.1;
    }
    o = o.lerp(o * Vec3::new(1.45, 1.3, 0.6) + Vec3::new(0.02, 0.03, 0.0), leaf.bud * 0.7);
    if maple {
        return o.lerp(c, leaf.turn);
    }
    let (gold, red) = (Vec3::new(0.62, 0.36, 0.04), Vec3::new(0.5, 0.1, 0.03));
    let tint = gold.lerp(red, smoothstep(0.3, 0.8, leaf.tree)) * (0.45 + c.y * 2.0);
    o.lerp(tint, leaf.turn * 0.92)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_year_wraps_and_the_seasons_come_in_order() {
        assert!((advance(0.9, 3.0, 24.0) - 0.025).abs() < 1e-5);
        assert!(declination(0.25) > 0.99 && declination(0.75) < -0.99);
        assert!(leaf_fall(0.62) > 0.99 && leaf_fall(DEFAULT_DATE) == 0.0 && leaf_fall(0.9) == 0.0);
        assert!(snow_cover(0.86) > 0.99 && snow_cover(DEFAULT_DATE) == 0.0);
        assert!(snow_cover(0.7) > snow_cover(0.65));
        let temperate = warmth(Biome::Deciduous);
        assert_eq!(Ground::at(DEFAULT_DATE, temperate, 0.6, 0.0, 0.0).snow, 0.6);
        assert_eq!(named("winter"), Some(0.86));
        assert_eq!(named("monsoon"), None);
    }

    #[test]
    fn falling_leaves_turn_with_their_tree() {
        let green = Vec3::new(0.05, 0.22, 0.03);
        let v = IVec3::new(40, 80, -12);
        // As modelled in summer; gold or red, and on, partway through autumn.
        assert_eq!(leaf_colour(green, &leaf(0.32, v, false, 9), false, 0.32), green);
        let l = leaf(0.6, v, false, 9);
        let c = leaf_colour(green, &l, false, 0.6);
        assert!(l.keep && c.x > c.y && c.x > green.x);
        // Gone by early winter; one tree drops later than another.
        assert!(!leaf(0.7, v, false, 9).keep);
        assert!(leaf(0.6, v, false, 60).fall_at > leaf(0.6, v, false, 2).fall_at);
    }

    #[test]
    fn leaves_are_down_by_early_winter() {
        assert!(leaf_fall(0.62) > 0.99);
        assert_eq!(leaf_fall(LEAVES_DOWN), 0.0);
        assert!(LEAVES_DOWN < 0.75);
        // The shader's dates (season_leaf): each tree's own, then a branch's and a leaf's
        // spread either side of it.
        let (tree, branch, leaf) = ((0.625, 0.675), 0.008, 0.015);
        let latest = tree.1 + branch + leaf;
        assert!(latest <= LEAVES_DOWN + 1e-6);
        // Trees go some way apart, so one can be bare while its neighbour holds half its
        // leaves.
        assert!(tree.1 - tree.0 >= 0.04 && tree.1 - tree.0 > 2.0 * (branch + leaf));
        // Maples go a little before the rest.
        assert!(latest - 0.025 < LEAVES_DOWN);
        // The particles fall while trees are dropping their leaves.
        assert!(leaf_fall(tree.0) > 0.99 && leaf_fall(0.5 * (tree.0 + tree.1)) > 0.9);
    }

    #[test]
    fn summer_snow_is_only_on_high_peaks() {
        // Temperate hills and alpine meadows (about 300 m) are clear in summer.
        for b in [Biome::Deciduous, Biome::Grassland, Biome::AlpineMeadow, Biome::BirchGrove] {
            assert!(Ground::snowline(warmth(b)) >= 300.0, "{b:?}");
        }
        assert!(Ground::snowline(warmth(Biome::Taiga)) >= 200.0);
        assert!(Ground::snowline(warmth(Biome::Alpine)) < Ground::snowline(warmth(Biome::Taiga)));
        // It comes down the hills in winter, and is back up by summer.
        let temperate = warmth(Biome::Deciduous);
        assert!(Ground::at(0.86, temperate, 0.0, 0.0, 0.0).snowline <= 100.0);
        assert!(Ground::at(DEFAULT_DATE, temperate, 0.0, 0.0, 0.0).snowline >= 300.0);
    }

    #[test]
    fn warm_biomes_never_snow() {
        let warm = [
            Biome::Desert,
            Biome::Savanna,
            Biome::Rainforest,
            Biome::Woodland,
            Biome::Mangrove,
            Biome::DuneSea,
            Biome::RedRock,
            Biome::Volcanic,
        ];
        for b in warm {
            assert!(warmth(b) >= NEVER_SNOWS, "{b:?}");
            for i in 0..100 {
                let g = Ground::at(i as f32 / 100.0, warmth(b), 1.0, 1.0, 0.0);
                assert_eq!((g.snow, g.frozen, g.highland), (0.0, 0.0, 0.0), "{b:?}");
                assert!(g.snowline >= 380.0);
            }
        }
        for b in [Biome::Tundra, Biome::Alpine, Biome::Taiga, Biome::SnowyPine, Biome::Deciduous, Biome::Bog, Biome::Steppe] {
            assert!(warmth(b) < NEVER_SNOWS, "{b:?}");
        }
    }
}
