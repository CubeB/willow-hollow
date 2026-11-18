//! Smoke from the chimneys of the homes near the player (#202): which of them are
//! smoking, and how much at this hour.
//!
//! A home's fire is built up in the morning and again for the evening meal, let down
//! through the middle of the day and banked for the night, so smoke rises thickest at
//! breakfast and supper, thin at noon, and from only a few chimneys, faintly, in the small
//! hours. Each household keeps its own hours a little, so a village's chimneys do not all
//! start and stop at once. The puffs themselves are particles.rs's.

use glam::DVec3;
use worldgen::{noise, scale::VOXELS_PER_METRE, world::GameWorld};

/// How many of the nearest chimneys smoke at once. A town in view sends up a thread from
/// each of these and costs no more however big it is: the far side of a town is seen by
/// the threads from its near side.
const MOST: usize = 24;
/// How often the chimneys about are looked for again, in seconds.
const EVERY: f32 = 2.0;
/// Below this much a chimney sends up nothing worth a puff: a fire banked for the night
/// in a household that keeps it low.
const THREAD: f32 = 0.08;

/// The chimneys near the player and how much each is smoking.
pub struct Chimneys {
    /// The nearest chimney tops, nearest first, as last looked for.
    near: Vec<DVec3>,
    /// Chimney tops set out by hand (`--chimneys`), to see the smoke where there is no
    /// village yet.
    pub extra: Vec<DVec3>,
    timer: f32,
    /// This frame's: each chimney smoking, and how much, 0 to 1.
    smoking: Vec<(DVec3, f32)>,
}

impl Chimneys {
    pub fn new() -> Self {
        Self { near: Vec::new(), extra: Vec::new(), timer: 0.0, smoking: Vec::new() }
    }

    /// Looks again for the chimneys near `camera` every couple of seconds, and works out
    /// how much each sends up at `hour` (0 to 24).
    pub fn update(&mut self, dt: f32, world: &GameWorld, camera: DVec3, hour: f32) {
        self.timer -= dt;
        if self.timer <= 0.0 {
            self.timer = EVERY;
            let reach = (crate::particles::SMOKE_SIGHT * VOXELS_PER_METRE as f64) as i32;
            self.near = world.chimneys_near(camera.x.floor() as i32, camera.z.floor() as i32, reach);
            self.near.extend_from_slice(&self.extra);
            self.near.sort_by(|a, b| a.distance_squared(camera).total_cmp(&b.distance_squared(camera)));
            self.near.truncate(MOST);
        }
        self.smoking.clear();
        for &top in &self.near {
            let much = smokes(top, hour);
            if much >= THREAD {
                self.smoking.push((top, much));
            }
        }
    }

    /// Each chimney smoking this frame, and how much, 0 to 1.
    pub fn smoking(&self) -> &[(DVec3, f32)] {
        &self.smoking
    }
}

/// How much a home's hearth smokes at `hour` (0 to 24), 0 to 1: most when the fire is
/// built up for breakfast and again for supper, a little more at noon for the midday
/// meal than through the rest of the day, and least in the night, banked under its ash.
pub fn rhythm(hour: f32) -> f32 {
    let bump = |at: f32, wide: f32| {
        let d = (hour - at + 12.0).rem_euclid(24.0) - 12.0;
        (-(d * d) / (2.0 * wide * wide)).exp()
    };
    (0.12 + 0.88 * bump(7.5, 1.6) + 0.8 * bump(18.6, 1.8) + 0.25 * bump(12.5, 1.5)).min(1.0)
}

/// How much the chimney at `top` smokes at `hour`: the day's rhythm as its household keeps
/// it, up to an hour early or late, and some fires always kept lower than others.
fn smokes(top: DVec3, hour: f32) -> f32 {
    let (x, z) = (top.x.floor() as i32, top.z.floor() as i32);
    let h = |k: u32| noise::unit(noise::hash2(0xc41_3e75 ^ k, x, z));
    let early = (h(1) - 0.5) * 2.0;
    rhythm(hour + early) * (0.5 + 0.5 * h(2))
}

/// `n` chimney tops set out by hand ahead of someone whose eye is at `eye`, looking
/// along `yaw`, as if a village stood there: four to a row across the view, the first row
/// `far` metres ahead and the rest behind it, each a house's height above the ground that
/// `ground` finds under it (none where it finds none). For `--chimneys`.
pub fn ahead(n: usize, eye: DVec3, yaw: f32, far: f64, mut ground: impl FnMut(f64, f64) -> Option<f64>) -> Vec<DVec3> {
    let m = VOXELS_PER_METRE as f64;
    let ahead = DVec3::new(yaw.cos() as f64, 0.0, yaw.sin() as f64);
    let across = DVec3::new(-yaw.sin() as f64, 0.0, yaw.cos() as f64);
    let mut out = Vec::new();
    for k in 0..n {
        let (row, col) = ((k / 4) as f64, (k % 4) as f64 - 1.5);
        let p = eye + ahead * ((far + row * 14.0) * m) + across * ((col * 12.0 + row * 3.0) * m);
        if let Some(y) = ground(p.x, p.z) {
            out.push(DVec3::new(p.x, y + 6.0 * m, p.z));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke_is_thickest_at_mealtimes_and_thin_at_night() {
        let (morning, noon, evening, night) = (rhythm(7.5), rhythm(13.0), rhythm(18.6), rhythm(2.0));
        assert!(morning > 0.95 && evening > 0.85, "{morning} {evening}");
        assert!(noon < 0.45 && noon > night, "{noon} {night}");
        assert!(night < 0.15);
        // Every chimney keeps to it within an hour, and some smoke at night, faintly.
        let tops: Vec<DVec3> = (0..200).map(|k| DVec3::new(k as f64 * 37.0, 40.0, k as f64 * -11.0)).collect();
        let at = |hour: f32| tops.iter().filter(|&&t| smokes(t, hour) >= THREAD).count();
        assert_eq!(at(7.5), tops.len());
        assert!((20..180).contains(&at(2.0)), "{} smoking at night", at(2.0));
    }
}
