//! Camping: resting by a fire, or in a tavern's guest bed, while the day or the night
//! goes by.
//!
//! Resting is the one way the player moves time. Nothing is skipped: the clock runs
//! fast, and the sun goes down, the stars wheel over and the weather passes as they
//! would have, only sooner. By day a rest lasts until dark; after dark, until morning.
//! Any step ends it. Lying by a fire at night, looking well up holds the clock to its
//! usual pace, to look at the stars as long as is wanted; looking down again, the night
//! goes on by as before.

use crate::env::Environment;

/// How fast the clock runs in the thick of a rest, in hours a second.
const HOURS_A_SECOND: f32 = 1.4;
/// How long a rest takes to gather that pace, in seconds.
const SETTLING: f32 = 2.5;
/// How long the sun is fully up by: the rest that waits for dawn ends here.
const MORNING: f32 = 0.1;
/// How far down it is when the stars are all out: the rest that waits for dusk ends here.
const DARK: f32 = -0.24;
/// How far up someone lying by a fire must look to be looking at the stars, as the sine
/// of the angle: about 40 degrees.
pub const LOOKING_UP: f32 = 0.64;
/// How far down the sun must be for there to be stars to look at.
const STARRY: f32 = -0.1;

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
    /// Lying down by the fire to sleep the night through, not sat up by it to wait for
    /// the dark.
    pub lying: bool,
    /// In a bed, as at an inn, not on the ground by a fire.
    pub bed: bool,
    /// Hours slept so far: those of a rest lying down that waits for the dawn.
    pub slept: f32,
    age: f32,
}

impl Rest {
    pub fn begin(env: &Environment) -> Self {
        let until = Until::from(env);
        Self { until, lying: until == Until::Dawn, bed: false, slept: 0.0, age: 0.0 }
    }

    /// A rest in a bed, as at an inn: lying down, the night slept through after dark, or
    /// by day, waiting there for the dark as by a fire.
    pub fn in_bed(env: &Environment) -> Self {
        Self { lying: true, bed: true, ..Self::begin(env) }
    }

    /// Whether someone at this rest whose eyes look `up` (the sine of how far above
    /// level) is looking at the stars: lying by a fire, not in a bed, in the night, with
    /// their eyes well up.
    pub fn gazing(&self, env: &Environment, up: f32) -> bool {
        self.lying && !self.bed && self.until == Until::Dawn && env.sun_elevation < STARRY && up > LOOKING_UP
    }

    /// How many hours go by in the next `dt` seconds of this rest, beyond the day's own
    /// pace, with the eyes looking `up` (as for `gazing`): none once it is over, and none
    /// while looking at the stars. It settles into its pace, again after each look up,
    /// and eases out of it as the time waited for comes. Only the hours that go by fast
    /// lying down for the night are counted as slept.
    pub fn hours(&mut self, env: &Environment, dt: f32, up: f32) -> Option<f32> {
        let left = self.until.left(env)?;
        if self.gazing(env, up) {
            self.age = 0.0;
            return Some(0.0);
        }
        self.age += dt;
        let settled = (self.age / SETTLING).clamp(0.0, 1.0);
        let settled = settled * settled * (3.0 - 2.0 * settled);
        let nearing = (left / 0.22).clamp(0.1, 1.0);
        let hours = HOURS_A_SECOND * settled * nearing * dt;
        if self.lying && self.until == Until::Dawn {
            self.slept += hours;
        }
        Some(hours)
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
        let mut rest = Rest::begin(&env);
        let (mut seconds, mut gone) = (0.0, 0.0);
        while let Some(hours) = rest.hours(&env, 1.0 / 60.0, 0.0) {
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
    fn a_night_in_a_bed_is_slept_lying_down_until_morning() {
        for hour in [8.0, 19.5, 23.0] {
            let env = Environment::at(hour, 0.3);
            let (fire, bed) = (Rest::begin(&env), Rest::in_bed(&env));
            assert!(bed.lying, "from {hour}");
            assert_eq!(bed.until, fire.until, "from {hour}: a bed waits for what a fire would");
        }
        let mut env = Environment::at(21.5, 0.3);
        let mut rest = Rest::in_bed(&env);
        while let Some(hours) = rest.hours(&env, 1.0 / 30.0, 0.0) {
            env = Environment::at(env.time + hours, 0.3);
        }
        assert!((5.0..10.0).contains(&env.time), "woke at {}", env.time);
        // A night's sleep from before ten is a whole one, enough to wake rested.
        assert!(rest.slept > 6.0 && crate::rested::Rested::of_sleep(rest.slept) == 1.0, "slept {} h", rest.slept);
        // Lying in a bed through the afternoon to wait for the dark is no sleep.
        let mut env = Environment::at(14.0, 0.3);
        let mut rest = Rest::in_bed(&env);
        while let Some(hours) = rest.hours(&env, 1.0 / 30.0, 0.0) {
            env = Environment::at(env.time + hours, 0.3);
        }
        assert_eq!(rest.slept, 0.0);
    }

    #[test]
    fn looking_up_at_the_stars_holds_the_clock_and_is_not_slept() {
        // Looking up by the fire from ten until midnight, at the day's own pace (a day in
        // forty minutes), then down to sleep: the night still ends at dawn, and the hours
        // gazed are not slept, though the rest of the night is.
        let day = 24.0 / (40.0 * 60.0);
        let mut env = Environment::at(22.0, 0.3);
        let mut rest = Rest::begin(&env);
        assert!(rest.gazing(&env, 0.9) && !rest.gazing(&env, 0.3) && !rest.gazing(&env, -0.5));
        let (mut seconds, mut gazed) = (0.0, 0.0);
        let mut dawn = false;
        while seconds < 2000.0 {
            let up = if env.time >= 22.0 { 0.9 } else { 0.0 };
            let Some(hours) = rest.hours(&env, 0.25, up) else {
                dawn = true;
                break;
            };
            if up > 0.0 {
                assert_eq!(hours, 0.0, "the clock runs fast while gazing at {}", env.time);
                gazed += day * 0.25;
            }
            env = Environment::at(env.time + hours + day * 0.25, 0.3);
            seconds += 0.25;
        }
        assert!(dawn && (5.0..10.0).contains(&env.time), "woke at {}", env.time);
        assert!((gazed - 2.0).abs() < 0.05, "gazed {gazed} h");
        assert!(rest.slept > 4.0 && rest.slept < 6.5, "slept {} h", rest.slept);
        // Not in a bed, nor by day waiting for the dark, nor once the dawn has come up.
        let bed = Rest::in_bed(&Environment::at(23.0, 0.3));
        assert!(!bed.gazing(&Environment::at(23.0, 0.3), 0.9));
        let noon = Environment::at(12.0, 0.3);
        assert!(!Rest::begin(&noon).gazing(&noon, 0.9));
        let night = Rest::begin(&Environment::at(23.0, 0.3));
        assert!(!night.gazing(&Environment::at(7.0, 0.3), 0.9));
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
