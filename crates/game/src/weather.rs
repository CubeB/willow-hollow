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
use worldgen::{biome::Biome, noise, scale::{VOXELS_PER_METRE, m}};

use crate::{env::Environment, season};

/// Weather a screenshot or a test can ask for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Forced {
    Clear,
    /// Scattered fair-weather cloud, and broken cloud with the sun between.
    Fair,
    Broken,
    Overcast,
    Rain,
    Snow,
    Storm,
    Fog,
}

impl Forced {
    /// Every weather that can be asked for, by the name `named` reads.
    pub const NAMES: &[&str] = &["clear", "fair", "broken", "overcast", "rain", "snow", "storm", "fog"];

    pub fn name(self) -> &'static str {
        match self {
            Self::Clear => "clear",
            Self::Fair => "fair",
            Self::Broken => "broken",
            Self::Overcast => "overcast",
            Self::Rain => "rain",
            Self::Snow => "snow",
            Self::Storm => "storm",
            Self::Fog => "fog",
        }
    }

    pub fn named(name: &str) -> Option<Self> {
        Some(match name {
            "clear" => Self::Clear,
            "fair" => Self::Fair,
            "broken" => Self::Broken,
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
    /// How much of the sky the clouds fill.
    pub cloud: f32,
    /// How grey and dim the day is under them.
    pub overcast: f32,
    /// Gale: scales the gusts.
    pub storm: f32,
    pub fog: f32,
    /// How tall the clouds stand for their cover, a half the usual: towering as a front
    /// comes on, low and flat as it goes.
    pub swell: f32,
    /// How fast the clouds form and clear beyond their usual pace (`CHURN`): faster while
    /// the weather is on the change.
    pub stir: f32,
}

/// The clouds' slow cycle of forming and clearing (see cloud_mix in common.wgsl), in
/// radians a second: a stretch of sky goes from one cloud to another and back in about
/// sixteen minutes, in steady weather.
pub const CHURN: f32 = std::f32::consts::TAU / 960.0;

pub struct Weather {
    pub now: Sky,
    /// How wet the ground is, 0..1: it darkens, and takes a while to dry.
    pub wet: f32,
    /// Snow that has fallen and not melted, 0..1.
    pub snowpack: f32,
    /// Ice on still water, 0..1.
    pub ice: f32,
    /// How much the ground where the camera is lies under snow, 0..1, as the caller
    /// last told it: on white ground what falls is snow.
    pub snowy: f32,
    /// How far round their cycle the clouds have come, radians.
    pub churn: f32,
    forced: Option<Forced>,
    /// Whether the sky's cloud has been set to the date's yet.
    cloud_set: bool,
    /// The date and warmth `lying` was last worked out for, and what it gave.
    lying_for: Option<(f32, f32)>,
    lying: (f32, f32),
    /// The date each biome's lying snow and ice (`lying`, by `season::warmth`) was last
    /// worked out for, and what they were.
    places_for: Option<f32>,
    places: [(f32, f32); Biome::ALL.len()],
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
    // And where it will be a quarter of a day on: the clouds build ahead of what comes.
    let ahead = around_year(date + 0.25 / year_days.max(4.0), year_days, 1.5, seed ^ 0x77ea);
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
        overcast: greyness(cover(date, year_days, seed, front, threshold, precip), precip),
        storm: precip * gale,
        fog: smooth(threshold - 0.5, threshold - 0.1, front) * (1.0 - precip) * foggy,
        cloud: cover(date, year_days, seed, front, threshold, precip),
        // Heaps tower as the front that brings rain comes on and through the showers, and
        // sink and spread as it moves off, while it is still raining.
        swell: (0.35 + 0.55 * smooth(threshold - 0.4, threshold + 0.1, ahead) + 0.6 * (ahead - front)).clamp(0.0, 1.0),
        stir: ((ahead - front).abs() * 10.0).min(2.0),
    }
}

/// How much of the sky the clouds fill. Dry days have their own sky, changing about once
/// a day: some clear, some with scattered fair-weather cloud, some broken, some overcast.
/// The cloud builds as a front comes on (`front` nearing `threshold`, past which it rains
/// or snows), and is never thin while anything falls: a shower comes from broken cloud
/// with the sun between, a downpour from a full overcast.
fn cover(date: f32, year_days: f32, seed: u32, front: f32, threshold: f32, precip: f32) -> f32 {
    let fair = around_year(date, year_days, 1.0, seed ^ 0xc10d);
    let dry = smooth(-0.25, 0.42, fair);
    let coming = smooth(threshold - 0.35, threshold, front) * 0.5;
    let falling = if precip > 0.0 { 0.5 + 0.5 * smooth(0.0, 0.6, precip) } else { 0.0 };
    dry.max(coming).max(falling)
}

/// How grey and dim a sky with this much cloud is: hardly at all while the sun shows
/// between the clouds (their shadows do the dimming there), then more as they close over,
/// and more again in rain or snow.
fn greyness(cloud: f32, precip: f32) -> f32 {
    (smooth(0.45, 1.0, cloud) * 0.85).max(precip * 0.6)
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

/// What falls where the camera is, as (rain, snow), of the `rain` and `snow` the date
/// calls for: one or the other, never the two together as sleet, and snow wherever the
/// ground is white (`snowy`, 0..1), be it a cold biome in winter or a summit above the
/// snowline, though the air at sea level is mild enough for rain.
pub fn falling(rain: f32, snow: f32, snowy: f32) -> (f32, f32) {
    let all = rain + snow;
    if snowy >= 0.5 || snow >= rain { (0.0, all) } else { (all, 0.0) }
}

/// Game days between two dates, the short way round the year.
fn days_apart(a: f32, b: f32, year_days: f32) -> f32 {
    (a - b).rem_euclid(1.0).min((b - a).rem_euclid(1.0)) * year_days.max(1.0)
}

fn forced_sky(f: Forced) -> Sky {
    let usual = Sky { swell: 0.5, ..Sky::default() };
    match f {
        Forced::Clear => usual,
        Forced::Fair => Sky { cloud: FAIR, ..usual },
        Forced::Broken => Sky { cloud: 0.6, overcast: greyness(0.6, 0.0), swell: 0.6, ..usual },
        Forced::Overcast => Sky { cloud: 1.0, overcast: 0.8, swell: 0.4, ..usual },
        Forced::Rain => Sky { rain: 0.8, cloud: 0.92, overcast: 0.8, storm: 0.3, swell: 0.8, ..usual },
        Forced::Snow => Sky { snow: 0.8, cloud: 0.95, overcast: 0.7, storm: 0.1, swell: 0.4, ..usual },
        Forced::Storm => Sky { rain: 1.0, cloud: 1.0, overcast: 1.0, storm: 1.0, swell: 1.0, ..usual },
        Forced::Fog => Sky { cloud: 0.6, overcast: 0.3, fog: 0.9, swell: 0.3, ..usual },
    }
}

/// Scattered fair-weather cloud: the sky a screenshot takes when no weather is asked for.
const FAIR: f32 = 0.3;

impl Weather {
    pub fn new(forced: Option<Forced>) -> Self {
        // Until the first update brings in the date's own sky (straight away: see there),
        // a fair day.
        let mut w = Self { now: forced_sky(Forced::Fair), wet: 0.0, snowpack: 0.0, ice: 0.0, snowy: 0.0, churn: 0.0, forced, cloud_set: false, lying_for: None, lying: (0.0, 0.0), places_for: None, places: [(0.0, 0.0); Biome::ALL.len()] };
        if let Some(f) = forced {
            // A forced weather is already in place, and has been for a while.
            w.now = forced_sky(f);
            w.wet = w.now.rain.min(1.0);
            w.snowpack = w.now.snow;
        }
        w
    }

    /// The weather being held, if one is.
    pub fn forced(&self) -> Option<Forced> {
        self.forced
    }

    /// Holds `forced` from now on, or with None lets the weather go its own way again.
    /// A weather asked for is there at once, sky and all; going back to the natural
    /// weather eases into it as any change of weather does.
    pub fn force(&mut self, forced: Option<Forced>) {
        self.forced = forced;
        if let Some(f) = forced {
            self.now = forced_sky(f);
            self.wet = self.wet.max(self.now.rain.min(1.0));
        }
    }

    /// The snow and ice the season has left lying (see `lying`), worked out again only when
    /// the date or the biome has moved on enough to change it.
    fn lying_now(&mut self, date: f32, year_days: f32, warmth: f32, seed: u32) -> (f32, f32) {
        let stale = self.lying_for.map_or(true, |(d, w)| days_apart(date, d, year_days) > 0.05 || (warmth - w).abs() > 0.01);
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

    /// What the ground carries now where the camera is, in a biome of `warmth`.
    pub fn ground(&self, date: f32, warmth: f32) -> season::Ground {
        season::Ground::at(date, warmth, self.snowpack, self.ice, self.wet)
    }

    /// What the ground carries now in every biome, for the shaders: the snow and ice the
    /// season has left lying in each (worked out again only when the date has moved on),
    /// with what a forced snow has added here added everywhere it can snow. Call after
    /// `settle` or `update`.
    pub fn places(&mut self, date: f32, year_days: f32, warmth: f32, seed: u32) -> season::Places {
        if self.places_for.is_none_or(|d| days_apart(date, d, year_days) > 0.05) {
            let mut known: Vec<(f32, (f32, f32))> = Vec::new();
            for (k, b) in Biome::ALL.iter().enumerate() {
                let w = season::warmth(*b);
                let got = match known.iter().find(|(kw, _)| *kw == w) {
                    Some(&(_, got)) => got,
                    None => {
                        let got = lying(date, year_days, w, seed);
                        known.push((w, got));
                        got
                    }
                };
                self.places[k] = got;
            }
            self.places_for = Some(date);
        }
        let here = self.ground(date, warmth);
        // The natural weather's snowpack only eases toward what lies here; a forced snow
        // falls on all the country in view.
        let fallen = if self.forced.is_some() { (self.snowpack - self.lying.0).max(0.0) } else { 0.0 };
        let mut biomes = [here; Biome::ALL.len()];
        for (k, b) in Biome::ALL.iter().enumerate() {
            let (snow, ice) = self.places[k];
            biomes[k] = season::Ground::at(date, season::warmth(*b), snow + fallen, ice, self.wet);
        }
        biomes[Biome::Ocean as usize] = here;
        season::Places { here, biomes }
    }

    /// Moves on by `dt` real seconds. `days` is the game days that passed in them.
    pub fn update(&mut self, dt: f32, days: f32, date: f32, year_days: f32, warmth: f32, seed: u32) {
        if self.lying_for.is_none() {
            self.settle(date, year_days, warmth, seed);
        }
        let (lying_snow, lying_ice) = self.lying_now(date, year_days, warmth, seed);
        let mut target = self.forced.map_or_else(|| natural(date, year_days, warmth, seed), forced_sky);
        if self.forced.is_none() {
            (target.rain, target.snow) = falling(target.rain, target.snow, self.snowy);
        }
        let ease = 1.0 - (-dt / 8.0).exp();
        let s = &mut self.now;
        // The cloud is the date's from the start, not rolling in over the first seconds.
        if !std::mem::replace(&mut self.cloud_set, true) {
            s.cloud = target.cloud;
            s.overcast = target.overcast;
            s.swell = target.swell;
            // Each day's sky starts somewhere of its own in the clouds' cycle.
            self.churn = (date * year_days * 7.3).fract() * std::f32::consts::TAU;
        }
        // The cover follows the weather more slowly than the rain does: cloud gathers
        // ahead of a shower and lingers as it clears, each heap growing or thinning in
        // its own time rather than the whole sky fading together.
        let gather = 1.0 - (-dt / 20.0).exp();
        let before = s.cloud;
        s.cloud += (target.cloud - s.cloud) * gather;
        s.swell += (target.swell - s.swell) * gather;
        s.stir += (target.stir - s.stir) * ease;
        // The clouds churn faster while the cover is changing, so a sky clearing or
        // closing in is seen to break up or build.
        let changing = ((s.cloud - before).abs() / dt.max(1e-3) * 400.0).min(3.0);
        self.churn = (self.churn + dt * CHURN * (1.0 + s.stir + changing)).rem_euclid(std::f32::consts::TAU);
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
        // What does come through thick cloud has lost the low sun's colour.
        env.sun_color = env.sun_color.lerp(Vec3::splat(env.sun_color.dot(Vec3::new(0.3, 0.59, 0.11))), o * 0.8);
        let grey = |c: Vec3, k: f32| c.lerp(Vec3::splat(c.dot(Vec3::new(0.3, 0.59, 0.11)) * k), o * 0.8);
        env.sky_zenith = grey(env.sky_zenith, 0.85);
        env.sky_horizon = grey(env.sky_horizon, 0.95);
        env.sky_glow = grey(env.sky_glow, 0.95);
        env.glow *= 1.0 - 0.8 * o;
        env.clouds = s.cloud.clamp(0.0, 1.0);
        env.cloud_churn = self.churn;
        env.cloud_swell = s.swell.clamp(0.0, 1.0);
        let precip = (s.rain + s.snow).min(1.0);
        // Rain and fog are grey and lie low: they thicken the haze, not the clear air.
        env.haze *= 1.0 + 4.0 * precip + 26.0 * s.fog;
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
        assert!(dull.clouds > clear.clouds && dull.haze > clear.haze);
        Weather::new(None).apply(&mut clear);
        assert_eq!(Forced::named("storm"), Some(Forced::Storm));
    }

    #[test]
    fn it_snows_on_white_ground_and_never_sleets() {
        assert_eq!(falling(0.6, 0.0, 1.0), (0.0, 0.6));
        assert_eq!(falling(0.6, 0.0, 0.0), (0.6, 0.0));
        assert_eq!(falling(0.25, 0.5, 0.0), (0.0, 0.75));
        assert_eq!(falling(0.5, 0.25, 0.0), (0.75, 0.0));
        // Through a winter on snowy ground nothing but snow comes down.
        let mut w = Weather::new(None);
        w.snowy = 1.0;
        let mut fell = 0.0;
        for i in 0..3000 {
            w.update(20.0, 0.0, 0.68 + 0.3 * i as f32 / 3000.0, 24.0, 0.25, 2);
            assert!(w.now.rain < 1e-3, "rain {} on snowy ground", w.now.rain);
            fell += w.now.snow;
        }
        assert!(fell > 50.0, "little snow fell: {fell}");
    }

    #[test]
    fn dry_days_run_from_clear_to_overcast_and_rain_never_falls_from_a_clear_sky() {
        let (mut dry, mut clear, mut scattered, mut broken, mut overcast, mut wet) = (0, 0, 0, 0, 0, 0);
        for seed in 1..=4 {
            for i in 0..2000 {
                let date = i as f32 / 2000.0;
                let s = natural(date, 24.0, 0.25, seed);
                if s.rain + s.snow > 0.01 {
                    wet += 1;
                    assert!(s.cloud >= 0.5, "{date} seed {seed}: {s:?}");
                    if s.rain + s.snow > 0.6 {
                        assert!(s.cloud > 0.9 && s.overcast > 0.6, "{date} seed {seed}: {s:?}");
                    }
                    continue;
                }
                dry += 1;
                match s.cloud {
                    c if c < 0.08 => clear += 1,
                    c if c < 0.45 => scattered += 1,
                    c if c < 0.85 => broken += 1,
                    _ => overcast += 1,
                }
                // The sun shows between broken cloud: the day only greys as it closes over.
                if s.cloud < 0.45 {
                    assert!(s.overcast < 0.05, "{date} seed {seed}: {s:?}");
                }
            }
        }
        let share = |n: i32| n as f32 / dry as f32;
        println!("dry {dry} wet {wet}: clear {} scattered {} broken {} overcast {}", share(clear), share(scattered), share(broken), share(overcast));
        for (name, n) in [("clear", clear), ("scattered", scattered), ("broken", broken), ("overcast", overcast)] {
            assert!(share(n) > 0.1, "{name} {}", share(n));
        }
        // Some showers fall from broken cloud, not every one from a full overcast.
        let showers = (1..=4).flat_map(|seed| (0..2000).map(move |i| natural(i as f32 / 2000.0, 24.0, 0.25, seed)));
        assert!(showers.filter(|s| s.rain + s.snow > 0.01).any(|s| s.cloud < 0.8));
        // And the cover closes round at the year's end like the rest.
        assert!((natural(0.9999, 24.0, 0.25, 1).cloud - natural(0.0, 24.0, 0.25, 1).cloud).abs() < 0.05);
    }

    #[test]
    fn forced_weathers_bring_their_own_cloud() {
        let cloud = |f| {
            let mut env = Environment::at(12.0, 0.32);
            Weather::new(Some(f)).apply(&mut env);
            env.clouds
        };
        assert_eq!(cloud(Forced::Clear), 0.0);
        assert!(cloud(Forced::Fair) > 0.1 && cloud(Forced::Fair) < 0.5);
        assert_eq!(cloud(Forced::Overcast), 1.0);
        assert!(cloud(Forced::Rain) > 0.8 && cloud(Forced::Snow) > 0.8 && cloud(Forced::Storm) > 0.95);
        // The first update brings in the date's own sky at once.
        let mut w = Weather::new(None);
        w.update(0.016, 0.0, 0.3, 24.0, 0.25, 1);
        assert_eq!(w.now.cloud, natural(0.3, 24.0, 0.25, 1).cloud);
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

/// The height of the clouds' base where the camera is: `CLOUD_HEIGHT` over low country,
/// and lifted clear of the hilltops where the land round about stands higher, so that no
/// cloud is ever cut through by a hill. It follows the camera slowly, as the clouds are
/// seen to ride up over high ground on the way to it.
#[derive(Default)]
pub struct CloudBase {
    /// Where the land was last looked over from, and the highest ground found there.
    looked: Option<(glam::DVec3, f32)>,
    now: Option<f32>,
}

impl CloudBase {
    /// As far as clouds are drawn, and a little more.
    const REACH: f32 = m(4500.0);
    /// How far the camera goes before the land is looked over again.
    const STRIDE: f64 = 400.0;

    /// The base's height in voxels above sea level, `dt` seconds on from the last call.
    pub fn update(&mut self, world: &worldgen::world::GameWorld, pos: glam::DVec3, dt: f32) -> f32 {
        use worldgen::macro_layout::{MACRO_SIZE, cell_of};
        if self.looked.is_none_or(|(at, _)| at.distance(pos) > Self::STRIDE) {
            let (mx, mz) = cell_of(pos.x.floor() as i32, pos.z.floor() as i32);
            // Never waited for: until the land is known the clouds stay where they are.
            if let Some(c) = world.continent_if_ready(mx, mz) {
                let cs = c.cell_size();
                let (lx, lz) = ((pos.x - (mx * MACRO_SIZE) as f64) as f32 / cs, (pos.z - (mz * MACRO_SIZE) as f64) as f32 / cs);
                let r = Self::REACH / cs;
                let max = c.size as i32 - 1;
                let mut highest = 0.0f32;
                for z in ((lz - r) as i32).clamp(0, max)..=((lz + r) as i32).clamp(0, max) {
                    for x in ((lx - r) as i32).clamp(0, max)..=((lx + r) as i32).clamp(0, max) {
                        if (x as f32 + 0.5 - lx).hypot(z as f32 + 0.5 - lz) <= r {
                            highest = highest.max(c.height[z as usize * c.size + x as usize]);
                        }
                    }
                }
                self.looked = Some((pos, highest * worldgen::scale::OLD_VOXEL));
            }
        }
        let target = self.looked.map_or(crate::env::CLOUD_HEIGHT, |(_, highest)| crate::env::CLOUD_HEIGHT.max(highest + crate::env::CLOUD_CLEARANCE));
        let now = match self.now {
            Some(now) => now + (target - now) * (1.0 - (-0.2 * dt).exp()),
            None => target,
        };
        self.now = self.looked.map(|_| now);
        now
    }
}
