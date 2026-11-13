//! Camping: resting by a fire or in a tent while the day or the night goes by.
//!
//! Resting is the one way the player moves time. Nothing is skipped: the clock runs
//! fast, and the sun goes down, the stars wheel over and the weather passes as they
//! would have, only sooner. By day a rest lasts until dark; after dark, until morning.
//! Any step ends it.

use crate::env::Environment;

/// How fast the clock runs in the thick of a rest, in hours a second.
const HOURS_A_SECOND: f32 = 1.4;
/// How long a rest takes to gather that pace, in seconds.
const SETTLING: f32 = 2.5;
/// How long the sun is fully up by: the rest that waits for dawn ends here.
const MORNING: f32 = 0.1;
/// How far down it is when the stars are all out: the rest that waits for dusk ends here.
const DARK: f32 = -0.24;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Until {
    Dawn,
    Dusk,
}

impl Until {
    /// What a rest begun now would wait for: morning if the sun is down or going, dark
    /// if the day is still up.
    pub fn from(env: &Environment) -> Self {
        if env.sun_elevation < 0.03 || env.time >= 19.0 || env.time < 4.0 { Until::Dawn } else { Until::Dusk }
    }

    pub fn word(self) -> &'static str {
        match self {
            Until::Dawn => "dawn",
            Until::Dusk => "dark",
        }
    }

    /// How much further the sun has to go, as the sine of its height, or nothing if
    /// the time waited for has come.
    fn left(self, env: &Environment) -> Option<f32> {
        let morning = env.time < 12.0;
        match self {
            Until::Dawn if morning => Some(MORNING - env.sun_elevation).filter(|d| *d > 0.0),
            Until::Dusk if !morning => Some(env.sun_elevation - DARK).filter(|d| *d > 0.0),
            // The other half of the day is still to get through.
            _ => Some(1.0),
        }
    }
}

/// A rest being taken.
pub struct Rest {
    pub until: Until,
    /// Lying down in a tent, not sat by a fire.
    pub lying: bool,
    age: f32,
}

impl Rest {
    pub fn begin(env: &Environment, lying: bool) -> Self {
        Self { until: Until::from(env), lying, age: 0.0 }
    }

    /// How many hours go by in the next `dt` seconds of this rest: none once it is over.
    /// It settles into its pace, and eases out of it as the time waited for comes.
    pub fn hours(&mut self, env: &Environment, dt: f32) -> Option<f32> {
        let left = self.until.left(env)?;
        self.age += dt;
        let settled = (self.age / SETTLING).clamp(0.0, 1.0);
        let settled = settled * settled * (3.0 - 2.0 * settled);
        let nearing = (left / 0.22).clamp(0.1, 1.0);
        Some(HOURS_A_SECOND * settled * nearing * dt)
    }

    /// How far the eye is lowered from standing, in metres, this far into the rest.
    pub fn lowered(&self) -> f32 {
        if self.lying { 1.12 } else { 0.72 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rests from `hour`, and tells when it ended and how many seconds it took.
    fn rest_from(hour: f32, date: f32) -> (Until, f32, f32) {
        let mut env = Environment::at(hour, date);
        let mut rest = Rest::begin(&env, true);
        let (mut seconds, mut gone) = (0.0, 0.0);
        while let Some(hours) = rest.hours(&env, 1.0 / 60.0) {
            assert!(hours >= 0.0 && hours < 0.05, "the clock never jumps: {hours}");
            gone += hours;
            env = Environment::at(env.time + hours, date);
            seconds += 1.0 / 60.0;
            assert!(seconds < 120.0, "a rest from {hour} never ended (at {})", env.time);
        }
        assert!(gone < 24.0);
        (rest.until, env.time, seconds)
    }

    #[test]
    fn a_rest_by_night_ends_in_the_morning_and_one_by_day_at_dark() {
        for date in [0.0, 0.25, 0.5, 0.75] {
            for hour in [19.5, 22.0, 0.5, 3.0] {
                let (until, woke, seconds) = rest_from(hour, date);
                assert_eq!(until, Until::Dawn, "from {hour}");
                assert!((5.0..10.0).contains(&woke), "from {hour} on date {date}, woke at {woke}");
                assert!(Environment::at(woke, date).sun_elevation >= MORNING - 0.01);
                assert!(seconds > 1.5 && seconds < 40.0, "from {hour}: {seconds} s");
            }
            for hour in [8.0, 12.0, 16.0] {
                let (until, woke, seconds) = rest_from(hour, date);
                assert_eq!(until, Until::Dusk, "from {hour}");
                assert!((17.0..23.0).contains(&woke), "from {hour} on date {date}, it was dark at {woke}");
                assert!(seconds > 1.5 && seconds < 40.0, "from {hour}: {seconds} s");
            }
        }
    }
}
