//! The calendar: where in the year the world is, as a fraction from 0 to 1.
//!
//! 0 is the spring equinox, 0.25 midsummer, 0.5 the autumn equinox and 0.75 midwinter.
//! The date is a continuous number, never four states, so everything that follows it
//! changes gradually. It advances with the time of day, so a year is a set number of game
//! days, and it is saved with the player's place, so a world is in the same season
//! wherever and whenever it is loaded again.

use std::ops::RangeInclusive;

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

#[allow(dead_code)]
/// How much snow lies on low ground at this date and cold: 0 in summer, 1 in deep winter.
pub fn snow_cover(date: f32) -> f32 {
    smoothstep(0.62, 0.80, date).max(1.0 - smoothstep(0.0, 0.12, date))
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
        assert_eq!(named("winter"), Some(0.86));
        assert_eq!(named("monsoon"), None);
    }
}
