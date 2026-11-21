//! Being rested (#37): after a night's sleep by a fire or in a bed, the walk comes a
//! little easier and the glider floats a little longer, wearing off over the next day.
//!
//! It is a gift, never a need: nothing is worse for being without it, and it does not
//! nag. It wears off only with the hours spent up and about, so nothing is lost by
//! walking away from the game. Anything can grant a rest, strong or light, for as long
//! as it lasts (a night's sleep, the incense of a shrine); a weaker one never shortens a
//! stronger one already had.
//!
//! It is plain data, a strength and the hours it has left, kept with where the player
//! is (`world-N.txt`), so it is the same wherever it is read.

/// How much faster the walk is, rested to the full.
const WALK: f32 = 0.08;
/// How much slower the glider sinks, rested to the full.
const GLIDE: f32 = 0.18;
/// The share of its time over which a rest wears off, at its end: until then it holds.
const FADES: f32 = 0.5;

/// A night's sleep by a fire or in a bed: as rested as can be, for a day.
pub const NIGHT: (f32, f32) = (1.0, 24.0);
/// Hours of sleep that count for nothing, and that count for a whole night.
const DOZE: f32 = 2.0;
const SLEEP: f32 = 6.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rested {
    /// How rested, 0 to 1, while it holds.
    strength: f32,
    /// Hours up and about it has left, and that it lasted from the first.
    left: f32,
    hours: f32,
}

impl Rested {
    /// A rest of `strength` (0 to 1) lasting `hours` of the day: taken if it leaves the
    /// player more rested than they are, and whether it did.
    pub fn grant(&mut self, strength: f32, hours: f32) -> bool {
        let strength = strength.clamp(0.0, 1.0);
        // Written to turn nonsense (NaN) away as well.
        if !(strength > self.level() && hours > 0.0 && hours.is_finite()) {
            return false;
        }
        *self = Self { strength, left: hours, hours };
        true
    }

    /// How strong a rest `slept` hours of sleep give: nothing for a doze, all of a
    /// night's from six hours on.
    pub fn of_sleep(slept: f32) -> f32 {
        ((slept - DOZE) / (SLEEP - DOZE)).clamp(0.0, 1.0)
    }

    /// Wears it off by `hours` spent up and about.
    pub fn pass(&mut self, hours: f32) {
        if self.left > 0.0 && hours > 0.0 {
            self.left = (self.left - hours).max(0.0);
            if self.left == 0.0 {
                *self = Self::default();
            }
        }
    }

    /// How rested the player is now, 0 to 1: as strong as it was given until the last of
    /// its time, then less and less.
    pub fn level(&self) -> f32 {
        if self.hours <= 0.0 {
            return 0.0;
        }
        self.strength * (self.left / (self.hours * FADES)).clamp(0.0, 1.0)
    }

    /// How many times their usual pace the player goes at.
    pub fn pace(&self) -> f32 {
        1.0 + WALK * self.level()
    }

    /// How much of its usual rate the glider sinks at.
    pub fn sink(&self) -> f32 {
        1.0 - GLIDE * self.level()
    }

    /// As a save holds it, or nothing when there is none.
    pub fn to_text(&self) -> Option<String> {
        (self.level() > 0.0).then(|| format!("{},{},{}", self.strength, self.left, self.hours))
    }

    /// Reads it back, as none if it makes no sense.
    pub fn from_text(text: &str) -> Self {
        let v: Vec<f32> = text.split(',').filter_map(|p| p.trim().parse::<f32>().ok()).filter(|v| v.is_finite()).collect();
        match v[..] {
            [strength, left, hours] if hours > 0.0 && (0.0..=hours).contains(&left) => {
                Self { strength: strength.clamp(0.0, 1.0), left, hours }
            }
            _ => Self::default(),
        }
    }

    /// Exactly this rested, for a screenshot.
    pub fn shown(level: f32) -> Self {
        Self { strength: level.clamp(0.0, 1.0), left: 20.0, hours: 24.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_night_rests_fully_and_wears_off_over_the_next_day() {
        let mut r = Rested::default();
        assert_eq!((r.level(), r.pace(), r.sink()), (0.0, 1.0, 1.0));
        assert!(r.grant(NIGHT.0, NIGHT.1));
        assert_eq!(r.level(), 1.0);
        assert!(r.pace() > 1.05 && r.pace() < 1.15 && r.sink() < 0.9 && r.sink() > 0.7);
        // It holds through the first half of the day, then fades.
        r.pass(10.0);
        assert_eq!(r.level(), 1.0);
        r.pass(8.0);
        assert!((r.level() - 0.5).abs() < 1e-5, "{}", r.level());
        r.pass(5.9);
        assert!(r.level() > 0.0 && r.level() < 0.01);
        r.pass(1.0);
        assert_eq!((r.level(), r, r.to_text()), (0.0, Rested::default(), None));
    }

    #[test]
    fn a_lighter_rest_never_cuts_a_stronger_one_short() {
        let mut r = Rested::default();
        // Incense, say: light and short.
        assert!(r.grant(0.4, 6.0));
        assert!((r.level() - 0.4).abs() < 1e-6);
        assert!(r.grant(1.0, 24.0));
        assert!(!r.grant(0.4, 6.0));
        assert_eq!(r.level(), 1.0);
        // Once the stronger has worn below it, the lighter is worth having.
        r.pass(22.0);
        assert!(r.level() < 0.4);
        assert!(r.grant(0.4, 6.0));
        // Nothing, or nonsense, gives nothing.
        assert!(!r.grant(0.0, 10.0) && !r.grant(0.9, 0.0) && !r.grant(f32::NAN, 5.0));
    }

    #[test]
    fn how_long_was_slept_says_how_rested_and_it_reads_back_from_a_save() {
        assert_eq!(Rested::of_sleep(0.5), 0.0);
        assert_eq!(Rested::of_sleep(4.0), 0.5);
        assert_eq!(Rested::of_sleep(9.0), 1.0);
        let mut r = Rested::default();
        r.grant(0.75, 24.0);
        r.pass(3.5);
        let back = Rested::from_text(&r.to_text().expect("rested"));
        assert_eq!(back, r);
        for nonsense in ["", "1,2", "a,b,c", "1,30,24", "1,5,-3", "inf,2,3"] {
            assert_eq!(Rested::from_text(nonsense).level(), 0.0, "{nonsense}");
        }
    }
}
