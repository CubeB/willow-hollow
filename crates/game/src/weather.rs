//! Weather: slowly changing cloud, rain, snow, fog and gales.
//!
//! Weather follows from the date alone, so every player in a world sees the same sky: a
//! smooth noise "front" that circles once around the year, with a precipitation threshold
//! that moves with the season (spring showers, rare summer storms, autumn gales, winter
//! snow and rain), and a second slow noise for mild and cold spells that decides which
//! falls as snow (`temperature`). What the player actually sees eases toward that target
//! over seconds, so a front rolls in rather than switching on. The same values feed the
//! sky, the wind and the rain and snow particles. The snow on the ground (`lying`) is the
//! weather since late summer played forward: it builds in snow spells and melts in rain
//! and thaws, so a temperate winter is white only now and then, and the warm biomes
//! (season::NEVER_SNOWS) never see snow at all.

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
    /// Ice on still water, 0..1.
    pub ice: f32,
    forced: Option<Forced>,
    /// The date and warmth `lying` was last worked out for, and what it gave.
    lying_for: Option<(f32, f32)>,
    lying: (f32, f32),
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

/// Precipitation falls as snow below about this temperature, as rain above (rough °C).
const SNOW_BELOW: f32 = 0.5;
/// Lying snow, as a share of full cover: what a game day of snowfall at full strength adds,
/// what each degree above freezing melts in a day, what a day of full rain washes away,
/// and the slow loss in any weather.
const SNOW_PER_DAY: f32 = 3.0;
const MELT_PER_DEGREE_DAY: f32 = 0.1;
const RAIN_MELT_PER_DAY: f32 = 0.8;
const SETTLING_PER_DAY: f32 = 0.03;
/// Ice on still water: how fast it forms per degree of frost, and melts per degree of
/// thaw, per game day.
const FREEZE_PER_DEGREE_DAY: f32 = 0.08;
const THAW_PER_DEGREE_DAY: f32 = 0.15;
/// Late summer, when every biome is bare of snow and ice: lying snow is played from here.
const BARE: f32 = 0.42;

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Smooth noise that circles once around the year, so the year's end meets its start,
/// changing about once every `days_per_change` game days.
fn around_year(date: f32, year_days: f32, days_per_change: f32, seed: u32) -> f32 {
    let r = year_days.max(4.0) / (days_per_change * std::f32::consts::TAU);
    let a = date * std::f32::consts::TAU;
    noise::perlin2(seed, (a.cos() * r) as f64, (a.sin() * r) as f64)
}

/// The air at sea level, in rough degrees Celsius: the season's mean for a biome of this
/// warmth (season::warmth), plus mild and cold spells of a few days. A temperate winter
/// hovers just above freezing, so its cold spells bring snow and its mild ones
/// rain and thaw; the cold biomes stay below freezing for months.
pub fn temperature(date: f32, year_days: f32, warmth: f32, seed: u32) -> f32 {
    // Warmest just after midsummer, coldest just after midwinter.
    let mean = 9.5 + 9.0 * ((date - 0.05) * std::f32::consts::TAU).sin() + (warmth - 0.25) * 14.0;
    mean + 8.0 * around_year(date, year_days, 2.2, seed ^ 0x3c1d)
}

/// The weather the date calls for, before easing. `warmth` is the biome's (season::warmth).
pub fn natural(date: f32, year_days: f32, warmth: f32, seed: u32) -> Sky {
    // A front that wanders in smooth noise and comes round again at the year's end, with
    // about one change every day and a half.
    let front = around_year(date, year_days, 1.5, seed ^ 0x77ea);
    // Spring showers, summer storms, autumn gales, winter snow and rain.
    let chance = through_year(date, [(0.10, 0.35), (0.32, 0.12), (0.58, 0.40), (0.85, 0.40)]);
    let heaviness = through_year(date, [(0.10, 0.55), (0.32, 1.0), (0.58, 0.85), (0.85, 0.6)]);
    let gale = through_year(date, [(0.10, 0.3), (0.32, 0.8), (0.58, 1.0), (0.85, 0.5)]);
    let foggy = through_year(date, [(0.10, 0.3), (0.32, 0.05), (0.58, 0.7), (0.85, 0.5)]);
    let threshold = 0.5 - chance;
    let precip = smooth(threshold, threshold + 0.25, front) * heaviness;
    // Snow in a cold spell, rain in a mild one; never snow where it is always warm.
    let cold = if warmth >= season::NEVER_SNOWS {
        0.0
    } else {
        1.0 - smooth(SNOW_BELOW - 1.0, SNOW_BELOW + 1.0, temperature(date, year_days, warmth, seed))
    };
    Sky {
        rain: precip * (1.0 - cold),
        snow: precip * cold,
        overcast: (precip * 0.9).max(smooth(threshold - 0.35, threshold, front) * 0.6),
        storm: precip * gale,
        fog: smooth(threshold - 0.5, threshold - 0.1, front) * (1.0 - precip) * foggy,
    }
}

/// The snow lying at sea level and the ice on still water at `date`, 0..1 each: the
/// weather since late summer, when every biome is bare, played through a few steps a day.
/// Snow builds in each snow spell and melts in rain and mild spells, so a temperate winter
/// comes and goes while a cold biome stays white. Deterministic, like the weather, so a
/// screenshot and the game agree.
pub fn lying(date: f32, year_days: f32, warmth: f32, seed: u32) -> (f32, f32) {
    if warmth >= season::NEVER_SNOWS {
        return (0.0, 0.0);
    }
    let span = (date - BARE).rem_euclid(1.0);
    let days = span * year_days.max(1.0);
    // About four steps a day, finer than any spell, at a bounded cost.
    let steps = ((days * 4.0).ceil() as usize).clamp(1, 600);
    let step_days = days / steps as f32;
    let (mut snow, mut ice) = (0.0f32, 0.0f32);
    for i in 1..=steps {
        let d = (BARE + span * i as f32 / steps as f32).rem_euclid(1.0);
        let sky = natural(d, year_days, warmth, seed);
        let t = temperature(d, year_days, warmth, seed);
        let melt = MELT_PER_DEGREE_DAY * t.max(0.0) + RAIN_MELT_PER_DAY * sky.rain + SETTLING_PER_DAY;
        snow = (snow + (sky.snow * SNOW_PER_DAY - melt) * step_days).clamp(0.0, 1.0);
        let freeze = if t < 0.0 { -t * FREEZE_PER_DEGREE_DAY } else { -t * THAW_PER_DEGREE_DAY };
        ice = (ice + freeze * step_days).clamp(0.0, 1.0);
    }
    (snow, ice)
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
        let mut w = Self { now: Sky::default(), wet: 0.0, snowpack: 0.0, ice: 0.0, forced, lying_for: None, lying: (0.0, 0.0) };
        if let Some(f) = forced {
            // A forced weather is already in place, and has been for a while.
            w.now = forced_sky(f);
            w.wet = w.now.rain.min(1.0);
            w.snowpack = w.now.snow;
        }
        w
    }

    /// The snow and ice the season has left lying (see `lying`), worked out again only when
    /// the date or the biome has moved on enough to change it.
    fn lying_now(&mut self, date: f32, year_days: f32, warmth: f32, seed: u32) -> (f32, f32) {
        let stale = self.lying_for.map_or(true, |(d, w)| {
            let moved = (date - d).rem_euclid(1.0).min((d - date).rem_euclid(1.0)) * year_days.max(1.0);
            moved > 0.05 || (warmth - w).abs() > 0.01
        });
        if stale {
            self.lying = lying(date, year_days, warmth, seed);
            self.lying_for = Some((date, warmth));
        }
        self.lying
    }

    /// Puts the ground straight into the state the season has left it in: the snow and
    /// ice lying at `date`, and on top of that whatever a forced weather has added.
    pub fn settle(&mut self, date: f32, year_days: f32, warmth: f32, seed: u32) {
        let (snow, ice) = self.lying_now(date, year_days, warmth, seed);
        self.snowpack = self.snowpack.max(snow);
        self.ice = ice;
    }

    /// What the ground carries now, for the shaders.
    pub fn ground(&self, date: f32, warmth: f32) -> season::Ground {
        season::Ground::at(date, warmth, self.snowpack, self.ice, self.wet)
    }

    /// Moves on by `dt` real seconds. `days` is the game days that passed in them.
    pub fn update(&mut self, dt: f32, days: f32, date: f32, year_days: f32, warmth: f32, seed: u32) {
        if self.lying_for.is_none() {
            self.settle(date, year_days, warmth, seed);
        }
        let (lying_snow, lying_ice) = self.lying_now(date, year_days, warmth, seed);
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
        let follow = 1.0 - (-dt / 4.0).exp();
        self.ice += (lying_ice - self.ice) * follow;
        if self.forced.is_none() {
            // The natural weather's snow is the season's lying snow, eased so that crossing
            // into a colder biome, or the date moving on, does not snap it.
            self.snowpack += (lying_snow - self.snowpack) * follow;
        } else {
            // A forced snow builds up while it falls and melts above freezing and in rain,
            // never below what the season has left lying.
            let t = temperature(date, year_days, warmth, seed);
            let melt = days * (SETTLING_PER_DAY + MELT_PER_DEGREE_DAY * t.max(0.0) + RAIN_MELT_PER_DAY * s.rain);
            self.snowpack = (self.snowpack + s.snow * dt * 0.012 - melt).clamp(lying_snow, 1.0);
        }
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
        assert!(winter().any(|(d, s)| natural(d, 24.0, 0.25, s).snow > 0.2));
        // The year closes: the end of it meets the beginning.
        let (a, b) = (natural(0.9999, 24.0, 0.25, 1), natural(0.0, 24.0, 0.25, 1));
        assert!((a.overcast - b.overcast).abs() < 0.05 && (a.snow - b.snow).abs() < 0.05);
    }

    /// Samples of deep winter, 0.68..0.98, across a few seeds: (date, seed).
    fn winter() -> impl Iterator<Item = (f32, u32)> {
        (1..=6).flat_map(|seed| (0..300).map(move |i| (0.68 + 0.3 * i as f32 / 300.0, seed)))
    }

    #[test]
    fn warm_biomes_never_see_snow() {
        use worldgen::biome::Biome;
        for b in [Biome::Desert, Biome::Savanna, Biome::Rainforest, Biome::Mangrove, Biome::DuneSea, Biome::RedRock, Biome::Woodland, Biome::Volcanic] {
            let warmth = season::warmth(b);
            for (date, seed) in winter() {
                assert_eq!(natural(date, 24.0, warmth, seed).snow, 0.0, "{b:?}");
            }
            for i in 0..40 {
                assert_eq!(lying(i as f32 / 40.0, 24.0, warmth, 1), (0.0, 0.0), "{b:?}");
            }
            let mut w = Weather::new(None);
            w.update(0.1, 0.0, 0.8, 24.0, warmth, 1);
            assert_eq!(w.ground(0.8, warmth).snow, 0.0);
        }
    }

    #[test]
    fn a_temperate_winter_mixes_snow_rain_and_dry_spells() {
        let warmth = season::warmth(worldgen::biome::Biome::Deciduous);
        let n = winter().count() as f32;
        let share = |f: &dyn Fn(f32, u32) -> bool| winter().filter(|&(d, s)| f(d, s)).count() as f32 / n;
        let snowing = share(&|d, s| natural(d, 24.0, warmth, s).snow > 0.1);
        let raining = share(&|d, s| natural(d, 24.0, warmth, s).rain > 0.1);
        let dry = share(&|d, s| {
            let k = natural(d, 24.0, warmth, s);
            k.rain + k.snow < 0.05
        });
        assert!(snowing > 0.05 && snowing < 0.35, "snowing {snowing}");
        assert!(raining > 0.05, "raining {raining}");
        assert!(dry > 0.3, "dry {dry}");
        // Lying snow comes and goes: white some of the time, green or patchy the rest.
        let white = share(&|d, s| lying(d, 24.0, warmth, s).0 > 0.35);
        let bare = share(&|d, s| lying(d, 24.0, warmth, s).0 < 0.15);
        assert!(white > 0.05 && white < 0.6, "white {white}");
        assert!(bare > 0.25, "bare {bare}");
        // It thaws as well as builds within a winter.
        for seed in 1..=6 {
            let pack: Vec<f32> = (0..300).map(|i| lying(0.66 + 0.32 * i as f32 / 300.0, 24.0, warmth, seed).0).collect();
            let thaws = pack.windows(2).filter(|w| w[1] < w[0] - 0.01).count();
            assert!(thaws > 0 || pack.iter().all(|&p| p < 0.15), "seed {seed}");
        }
    }

    /// A winter's weather and lying snow, day by day: `cargo test --release -p game winter_diary -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn winter_diary() {
        for (name, warmth) in [("temperate", 0.25), ("taiga", -0.2)] {
            for seed in 1..=3 {
                println!("{name} seed {seed}");
                for i in 0..48 {
                    let d = 0.60 + 0.4 * i as f32 / 48.0;
                    let k = natural(d, 24.0, warmth, seed);
                    let (snow, ice) = lying(d, 24.0, warmth, seed);
                    println!(
                        "  {d:.3} T {:5.1} snowing {:.2} rain {:.2} lying {snow:.2} ice {ice:.2}",
                        temperature(d, 24.0, warmth, seed),
                        k.snow,
                        k.rain
                    );
                }
            }
        }
    }

    #[test]
    fn cold_biomes_stay_white_through_midwinter() {
        use worldgen::biome::Biome;
        for b in [Biome::Tundra, Biome::Alpine, Biome::SnowyPine, Biome::Taiga] {
            let warmth = season::warmth(b);
            for seed in 1..=4 {
                let snow = lying(0.82, 24.0, warmth, seed).0;
                assert!(snow > 0.6, "{b:?} seed {seed}: {snow}");
            }
            // And bare again by midsummer.
            assert!(lying(0.3, 24.0, warmth, 1).0 < 0.05, "{b:?}");
        }
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
