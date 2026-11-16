//! The northern lights: on some nights in the cold country, curtains of green going to
//! violet ripple across the northern sky (drawn in sky.wgsl).
//!
//! How strong they are is worked out here once a frame, from three things: how active the
//! sky is tonight, a smooth wander through the year that comes and goes over a day or
//! so, so that the lights are out on some nights and not others and rise and fade over
//! hours rather than switching; how cold and far north the country is; and how dark it
//! is. The sky pass is told the one number, through a uniform of its own, so nothing
//! else about the sky or its lighting changes for it.

use glam::Vec3;
use worldgen::noise;

use crate::env::Environment;

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How active the sky is on date `date`, 0 to 1: smooth noise that goes once round the
/// year, so the year's end meets its start, changing every day and a half or so. Most nights
/// it is 0; on about one in four it climbs for a while and falls away again.
pub fn activity(date: f32, year_days: f32, seed: u32) -> f32 {
    let r = year_days.max(4.0) / (1.5 * std::f32::consts::TAU);
    // Averaged over a day either side, which takes the noise's steeper turns out: the
    // lights should rise and fade over hours, never come on between one hour and the next.
    let day = 1.0 / year_days.max(4.0);
    let n = (-4..=4)
        .map(|k| {
            let a = (date + k as f32 * day / 4.0) * std::f32::consts::TAU;
            noise::perlin2(seed ^ 0xa0_7a1a, (a.cos() * r) as f64, (a.sin() * r) as f64) as f32
        })
        .sum::<f32>()
        / 9.0;
    smooth(0.08, 0.42, n)
}

/// How far into the cold country the camera is, 0 to 1: from the warmth of its biome
/// (season::warmth, below 0 for the cold ones) and, where the continent is ready, the
/// climate of the land under it (0 polar to 1 tropical), which is lower toward the north
/// of each continent and on high ground.
pub fn cold(warmth: f32, climate: Option<f32>) -> f32 {
    let by_biome = smooth(0.12, -0.2, warmth);
    let by_climate = climate.map_or(0.0, |t| smooth(0.34, 0.2, t));
    by_biome.max(by_climate)
}

/// The land's climate at a place (see [`cold`]), None until its continent is ready.
pub fn climate_at(world: &worldgen::world::GameWorld, x: f64, z: f64) -> Option<f32> {
    use worldgen::macro_layout::{MACRO_SIZE, cell_of};
    let (mx, mz) = cell_of(x.floor() as i32, z.floor() as i32);
    let c = world.continent_if_ready(mx, mz)?;
    let cs = c.cell_size() as f64;
    let max = c.size as i32 - 1;
    let gx = (((x - (mx * MACRO_SIZE) as f64) / cs).floor() as i32).clamp(0, max);
    let gz = (((z - (mz * MACRO_SIZE) as f64) / cs).floor() as i32).clamp(0, max);
    Some(c.temperature[gz as usize * c.size + gx as usize])
}

/// How bright the lights are now, 0 to 1: only once it is properly dark, and fading in
/// and out with the dusk and the dawn as well as with the night's activity.
pub fn strength(env: &Environment, activity: f32, cold: f32) -> f32 {
    activity * cold * smooth(-0.1, -0.26, env.sun_elevation)
}

/// Sets the lights in the sky for tonight's `out` (activity and cold together, 0 to 1),
/// and puts their faint glow on what the moon lights, snow most of all: a touch of green
/// in the moonlight and the night sky. Cheap, and no more than a hint, as it is.
pub fn light(env: &mut Environment, out: f32) {
    let strength = self::strength(env, out, 1.0);
    env.aurora = strength;
    if strength <= 0.0 || env.daylight {
        return;
    }
    let s = strength * (1.0 - env.clouds).clamp(0.2, 1.0);
    env.sun_color += Vec3::new(0.003, 0.02, 0.009) * s;
    env.sky_zenith += Vec3::new(0.0006, 0.004, 0.002) * s;
    env.sky_horizon += Vec3::new(0.0004, 0.0025, 0.0012) * s;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lights_come_out_on_some_nights_only_and_ease_in_and_out() {
        let year = 48.0;
        let nights: Vec<f32> = (0..year as usize).map(|d| activity(d as f32 / year, year, 7)).collect();
        let on = nights.iter().filter(|&&a| a > 0.5).count();
        assert!(on >= 2 && on <= 24, "out on {on} nights of {year}");
        // No sudden changes from one hour to the next.
        for k in 0..(year as usize * 24) {
            let (a, b) = (activity(k as f32 / (year * 24.0), year, 7), activity((k + 1) as f32 / (year * 24.0), year, 7));
            assert!((a - b).abs() < 0.25, "jumped from {a} to {b} in an hour");
        }
    }

    #[test]
    fn only_in_the_cold_and_only_in_the_dark() {
        assert_eq!(cold(0.25, Some(0.5)), 0.0);
        assert!(cold(-0.3, None) > 0.99);
        assert!(cold(0.25, Some(0.15)) > 0.99, "the far north of a temperate continent");
        let night = Environment::at(1.0, 0.8);
        let dusk = Environment::at(18.0, 0.3);
        let noon = Environment::at(12.0, 0.8);
        assert!(strength(&night, 1.0, 1.0) > 0.9);
        assert_eq!(strength(&noon, 1.0, 1.0), 0.0);
        assert!(strength(&dusk, 1.0, 1.0) < 0.05);
    }
}
