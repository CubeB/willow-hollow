//! Weather: slowly changing cloud, rain, snow, fog and gales.
//!
//! Weather follows from the date alone, so every player in a world sees the same sky: a
//! smooth noise "front" that circles once around the year, with a precipitation threshold
//! that moves with the season (spring showers, rare summer storms, autumn gales, winter
//! snow). What the player actually sees eases toward that target over seconds, so a front
//! rolls in rather than switching on. The same values feed the sky, the wind and the
//! rain and snow particles, and what falls as snow stays on the ground (`snowpack`),
//! sharing the snow cover of the seasons.

use glam::Vec3;
use worldgen::{noise, scale::VOXELS_PER_METRE};

use crate::{env::Environment, season};

/// Weather a screenshot or a test can ask for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Forced {
    Clear,
    Overcast,
    Rain,
    Snow,
    Storm,
    Fog,
}

impl Forced {
    pub fn named(name: &str) -> Option<Self> {
        Some(match name {
            "clear" => Self::Clear,
            "overcast" | "cloudy" => Self::Overcast,
            "rain" => Self::Rain,
            "snow" => Self::Snow,
            "storm" => Self::Storm,
            "fog" => Self::Fog,
            _ => return None,
        })
    }
}

/// How the sky stands: every value 0..1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sky {
    pub rain: f32,
    pub snow: f32,
    pub overcast: f32,
    /// Gale: scales the gusts.
    pub storm: f32,
    pub fog: f32,
}

pub struct Weather {
    pub now: Sky,
    /// How wet the ground is, 0..1: it darkens, and takes a while to dry.
    pub wet: f32,
    /// Snow that has fallen and not melted, 0..1.
    pub snowpack: f32,
    forced: Option<Forced>,
}

/// A value that changes through the year: linear between dated keys, wrapping around.
fn through_year(date: f32, keys: [(f32, f32); 4]) -> f32 {
    let n = keys.len();
    for i in 0..n {
        let (d0, v0) = keys[i];
        let (mut d1, v1) = keys[(i + 1) % n];
        let mut d = date;
        if d1 <= d0 {
            d1 += 1.0;
            if d < d0 {
                d += 1.0;
            }
        }
        if d >= d0 && d <= d1 {
            return v0 + (v1 - v0) * (d - d0) / (d1 - d0);
        }
    }
    keys[0].1
}

/// The weather the date calls for, before easing. `warmth` is the biome's (season::warmth).
pub fn natural(date: f32, year_days: f32, warmth: f32, seed: u32) -> Sky {
    // A front that wanders in smooth noise and comes round again at the year's end, with
    // about one change every day and a half.
    let r = year_days.max(4.0) / (1.5 * std::f32::consts::TAU);
    let a = date * std::f32::consts::TAU;
    let front = noise::perlin2(seed ^ 0x77ea, (a.cos() * r) as f64, (a.sin() * r) as f64);
    // Spring showers, summer storms, autumn gales, winter snow.
    let chance = through_year(date, [(0.10, 0.35), (0.32, 0.12), (0.58, 0.40), (0.85, 0.40)]);
    let heaviness = through_year(date, [(0.10, 0.55), (0.32, 1.0), (0.58, 0.85), (0.85, 0.6)]);
    let gale = through_year(date, [(0.10, 0.3), (0.32, 0.8), (0.58, 1.0), (0.85, 0.5)]);
    let foggy = through_year(date, [(0.10, 0.3), (0.32, 0.05), (0.58, 0.7), (0.85, 0.5)]);
    let threshold = 0.5 - chance;
    let smooth = |a: f32, b: f32, x: f32| {
        let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let precip = smooth(threshold, threshold + 0.25, front) * heaviness;
    // Cold enough to fall as snow: late in the year, and in cold biomes.
    let cold = smooth(0.35, 0.55, season::snow_cover(date) - warmth * 0.5);
    Sky {
        rain: precip * (1.0 - cold),
        snow: precip * cold,
        overcast: (precip * 0.9).max(smooth(threshold - 0.35, threshold, front) * 0.6),
        storm: precip * gale,
        fog: smooth(threshold - 0.5, threshold - 0.1, front) * (1.0 - precip) * foggy,
    }
}

fn forced_sky(f: Forced) -> Sky {
    match f {
        Forced::Clear => Sky::default(),
        Forced::Overcast => Sky { overcast: 0.8, ..Sky::default() },
        Forced::Rain => Sky { rain: 0.8, overcast: 0.8, storm: 0.3, ..Sky::default() },
        Forced::Snow => Sky { snow: 0.8, overcast: 0.7, storm: 0.1, ..Sky::default() },
        Forced::Storm => Sky { rain: 1.0, overcast: 1.0, storm: 1.0, ..Sky::default() },
        Forced::Fog => Sky { overcast: 0.3, fog: 0.9, ..Sky::default() },
    }
}

impl Weather {
    pub fn new(forced: Option<Forced>) -> Self {
        let mut w = Self { now: Sky::default(), wet: 0.0, snowpack: 0.0, forced };
        if let Some(f) = forced {
            // A forced weather is already in place, and has been for a while.
            w.now = forced_sky(f);
            w.wet = w.now.rain.min(1.0);
            w.snowpack = w.now.snow;
        }
        w
    }

    /// Moves on by `dt` real seconds. `days` is the game days that passed in them.
    pub fn update(&mut self, dt: f32, days: f32, date: f32, year_days: f32, warmth: f32, seed: u32) {
        let target = self.forced.map_or_else(|| natural(date, year_days, warmth, seed), forced_sky);
        let ease = 1.0 - (-dt / 8.0).exp();
        let s = &mut self.now;
        s.rain += (target.rain - s.rain) * ease;
        s.snow += (target.snow - s.snow) * ease;
        s.overcast += (target.overcast - s.overcast) * ease;
        s.storm += (target.storm - s.storm) * ease;
        s.fog += (target.fog - s.fog) * ease;
        // Ground wets quickly in rain and dries over a game day or so.
        let rate = if s.rain > 0.15 { dt / 10.0 } else { -days * 1.2 };
        self.wet = (self.wet + rate * if s.rain > 0.15 { s.rain } else { 1.0 }).clamp(0.0, 1.0);
        // Snow builds up while it falls and melts in the warm: faster on a warm date.
        let warm = 1.0 - season::snow_cover(date).min(1.0);
        let melt = days * (0.15 + 1.2 * warm * (warmth + 0.6).clamp(0.0, 1.5));
        self.snowpack = (self.snowpack + s.snow * dt * 0.012 - melt).clamp(0.0, 1.0);
    }

    /// Greys and darkens the sky and light, thickens the cloud and fog.
    pub fn apply(&self, env: &mut Environment) {
        let s = self.now;
        let o = s.overcast.clamp(0.0, 1.0);
        env.sun_color *= 1.0 - 0.8 * o;
        let grey = |c: Vec3, k: f32| c.lerp(Vec3::splat(c.dot(Vec3::new(0.3, 0.59, 0.11)) * k), o * 0.8);
        env.sky_zenith = grey(env.sky_zenith, 0.85);
        env.sky_horizon = grey(env.sky_horizon, 0.95);
        env.clouds = 0.45 + 0.55 * o;
        let precip = (s.rain + s.snow).min(1.0);
        env.fog_density *= 1.0 + 6.0 * precip + 45.0 * s.fog;
        env.mist += s.fog * 0.003 / VOXELS_PER_METRE;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weather_is_deterministic_and_follows_the_season() {
        assert_eq!(natural(0.3, 24.0, 0.25, 1), natural(0.3, 24.0, 0.25, 1));
        let share = |date: f32, f: fn(&Sky) -> bool| {
            let n = 2000;
            (0..n).filter(|i| f(&natural(date + *i as f32 * 0.00008, 24.0, 0.25, 1))).count() as f32 / n as f32
        };
        // Wetter in autumn than in summer; snow only in the cold half of the year.
        assert!(share(0.5, |s| s.rain + s.snow > 0.2) > share(0.2, |s| s.rain + s.snow > 0.2));
        assert_eq!(share(0.25, |s| s.snow > 0.01), 0.0);
        assert!((0..2000).any(|i| natural(0.7 + i as f32 * 0.00015, 24.0, 0.25, 1).snow > 0.2));
        // The year closes: the end of it meets the beginning.
        let (a, b) = (natural(0.9999, 24.0, 0.25, 1), natural(0.0, 24.0, 0.25, 1));
        assert!((a.overcast - b.overcast).abs() < 0.05 && (a.snow - b.snow).abs() < 0.05);
    }

    #[test]
    fn a_forced_weather_is_in_place_and_darkens_the_day() {
        let w = Weather::new(Some(Forced::Rain));
        assert!(w.now.rain > 0.5 && w.wet > 0.5);
        let (mut clear, mut dull) = (Environment::at(12.0, 0.32), Environment::at(12.0, 0.32));
        w.apply(&mut dull);
        assert!(dull.sun_color.length() < clear.sun_color.length() * 0.5);
        assert!(dull.clouds > clear.clouds && dull.fog_density > clear.fog_density);
        Weather::new(None).apply(&mut clear);
        assert_eq!(Forced::named("storm"), Some(Forced::Storm));
    }

    #[test]
    fn snow_builds_while_it_falls_and_melts_when_warm() {
        let mut w = Weather::new(Some(Forced::Snow));
        w.snowpack = 0.0;
        for _ in 0..600 {
            w.update(0.1, 0.0, 0.8, 24.0, 0.25, 1);
        }
        assert!(w.snowpack > 0.3, "{}", w.snowpack);
        let mut thaw = Weather::new(Some(Forced::Clear));
        thaw.snowpack = 0.8;
        for _ in 0..200 {
            thaw.update(1.0, 0.01, 0.3, 24.0, 0.25, 1);
        }
        assert!(thaw.snowpack < 0.4);
    }
}
