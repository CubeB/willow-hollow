//! The calendar: where in the year the world is, as a fraction from 0 to 1.
//!
//! 0 is the spring equinox, 0.25 midsummer, 0.5 the autumn equinox and 0.75 midwinter.
//! The date is a continuous number, never four states, so everything that follows it
//! changes gradually. It advances with the time of day, so a year is a set number of game
//! days, and it is saved with the player's place, so a world is in the same season
//! wherever and whenever it is loaded again.

use std::ops::RangeInclusive;

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

/// How much of the year's leaf fall is happening now, 0 to 1, peaking in mid autumn. The
/// falling leaf particles follow it.
pub fn leaf_fall(date: f32) -> f32 {
    smoothstep(0.46, 0.58, date) * (1.0 - smoothstep(0.66, 0.78, date))
}

/// How much snow lies on low ground at this date and cold: 0 in summer, 1 in deep winter.
pub fn snow_cover(date: f32) -> f32 {
    smoothstep(0.62, 0.80, date).max(1.0 - smoothstep(0.0, 0.12, date))
}

/// How warm a biome is against the year: the share of the winter that must pass before
/// snow settles there. Negative for the cold biomes, where it settles earlier and deeper;
/// above 1 where it never does.
pub fn warmth(biome: Biome) -> f32 {
    match biome {
        // Snowy pine is snow-covered by definition; the other newer biomes follow their parent.
        Biome::SnowyPine => -0.3,
        Biome::Alpine => -0.5,
        Biome::Tundra => -0.35,
        Biome::Taiga => -0.2,
        Biome::Heath | Biome::Grassland | Biome::Deciduous | Biome::Woodland | Biome::OldGrowth | Biome::Ocean => 0.25,
        Biome::Savanna | Biome::Desert | Biome::Rainforest => 1.5,
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
}

impl Ground {
    /// The ground at `date` in a biome of this warmth, with `fallen` snow from the weather.
    pub fn at(date: f32, warmth: f32, fallen: f32, wet: f32) -> Self {
        let season = snow_cover(date);
        let settled = if warmth < 0.0 { season * (1.0 - warmth) } else { (season - warmth).max(0.0) / (1.0 - warmth).max(0.05) };
        // Ponds freeze only after the ground has been snowy a while.
        let frozen = ((season - 0.45 - warmth.max(0.0) * 0.5) * 2.0).clamp(0.0, 1.0);
        Self { snow: settled.max(fallen).clamp(0.0, 1.0), frozen, wet }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_year_wraps_and_the_seasons_come_in_order() {
        assert!((advance(0.9, 3.0, 24.0) - 0.025).abs() < 1e-5);
        assert!(declination(0.25) > 0.99 && declination(0.75) < -0.99);
        assert!(leaf_fall(0.58) > 0.99 && leaf_fall(DEFAULT_DATE) == 0.0 && leaf_fall(0.9) == 0.0);
        assert!(snow_cover(0.86) > 0.99 && snow_cover(DEFAULT_DATE) == 0.0);
        assert!(snow_cover(0.7) > snow_cover(0.65));
        let temperate = warmth(Biome::Deciduous);
        assert_eq!(Ground::at(DEFAULT_DATE, temperate, 0.0, 0.0).snow, 0.0);
        assert!(Ground::at(0.86, warmth(Biome::Tundra), 0.0, 0.0).snow > Ground::at(0.7, temperate, 0.0, 0.0).snow);
        assert_eq!(Ground::at(0.86, warmth(Biome::Desert), 0.0, 0.0).snow, 0.0);
        assert!(Ground::at(0.86, temperate, 0.0, 0.0).frozen > 0.8 && Ground::at(0.5, temperate, 0.0, 0.0).frozen == 0.0);
        assert_eq!(Ground::at(DEFAULT_DATE, temperate, 0.6, 0.0).snow, 0.6);
        assert_eq!(named("winter"), Some(0.86));
        assert_eq!(named("monsoon"), None);
    }
}
