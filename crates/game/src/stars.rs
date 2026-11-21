//! The figures in the night sky (#37): a few constellations among the scattered stars,
//! the same in a world every night and every time it is played, and named by its people.
//!
//! They are made from the world's seed alone. Each lies on the sphere the stars turn on
//! (the frame `stars()` in sky.wgsl turns the view into), so they wheel about the pole
//! with the rest, once a night and once more over the year: at the same hour each
//! season has its own figures overhead. They are drawn as the other stars are, only a
//! little brighter, so they are found by looking rather than shown.

use std::f32::consts::TAU;

use glam::Vec3;
use worldgen::noise::{hash2, unit};

/// How many figures a sky has, and the most stars one is drawn with; the sky shader
/// reads them in groups of this many.
pub const FIGURES: usize = 7;
pub const EACH: usize = 8;
/// How far apart the figures' middles are at the least, as the cosine of the angle.
const APART: f32 = 0.82;
/// How far from the turning sphere's equator a figure may lie: nearer the pole it would
/// only ever creep along the horizon.
const BAND: f32 = 0.55;
/// How high a figure must stand to be made out, as the sine of its height: low down the
/// haze swallows it.
const WELL_UP: f32 = 0.42;
/// How dark it must be for the figures to be made out, as the sine of the sun's height.
const DARK: f32 = -0.2;

/// One of the figures.
#[derive(Clone, Debug, PartialEq)]
pub struct Figure {
    pub name: String,
    /// Its stars, as directions on the turning sphere, with how bright each is.
    pub stars: Vec<(Vec3, f32)>,
    /// Its middle, and the cosine of how far from it its furthest star lies.
    pub middle: Vec3,
    pub reach: f32,
}

/// The constellations of a world's sky.
#[derive(Clone, Debug, PartialEq)]
pub struct Sky {
    pub figures: Vec<Figure>,
}

/// What the sky shader is given of them: for each figure its middle and reach, and
/// its stars, the unused ones left dark.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Lights {
    pub groups: [[f32; 4]; FIGURES],
    pub stars: [[f32; 4]; FIGURES * EACH],
}

impl Default for Lights {
    fn default() -> Self {
        bytemuck::Zeroable::zeroed()
    }
}

/// How far the stars have wheeled round at `hour` on `date`, in radians: once a day and
/// once more over the year, so each season has its own evening sky.
pub fn wheel(hour: f32, date: f32) -> f32 {
    (hour / 24.0 + date) * TAU
}

/// Where a direction on the turning sphere is in the sky, turned `angle` round: the
/// inverse of the turn the sky shader gives a view.
pub fn in_sky(star: Vec3, angle: f32) -> Vec3 {
    let (s, c) = angle.sin_cos();
    Vec3::new(star.x * c - star.y * s, star.x * s + star.y * c, star.z)
}

/// A unit direction from two numbers in 0..1, within `band` of the equator about z.
fn direction(u: f32, v: f32, band: f32) -> Vec3 {
    let z = (v * 2.0 - 1.0) * band;
    let a = u * TAU;
    let r = (1.0 - z * z).sqrt();
    Vec3::new(r * a.cos(), r * a.sin(), z)
}

impl Sky {
    /// The sky of world `seed`.
    pub fn of(seed: u32) -> Self {
        let r = |a: i32, b: i32| unit(hash2(seed ^ 0x0057_a125, a, b));
        let tongue = worldgen::names::sky::tongue(seed);
        let mut figures: Vec<Figure> = Vec::new();
        let mut names: Vec<String> = Vec::new();
        let mut attempt = 0;
        while figures.len() < FIGURES && attempt < 400 {
            attempt += 1;
            let middle = direction(r(attempt, 0), r(attempt, 1), BAND);
            if figures.iter().any(|f| f.middle.dot(middle) > APART) {
                continue;
            }
            let k = figures.len() as i32;
            // Its stars run out from the first in a wandering line, now and then
            // branching from one before; each a few degrees from the last.
            let count = 4 + (r(attempt, 2) * 4.0) as usize;
            let (east, north) = {
                let east = Vec3::Z.cross(middle).normalize_or(Vec3::X);
                (east, middle.cross(east))
            };
            let mut flat: Vec<glam::Vec2> = vec![glam::Vec2::ZERO];
            let mut heading = r(attempt, 3) * TAU;
            let mut tries = 0;
            while flat.len() < count && tries < 60 {
                tries += 1;
                let n = tries + 10;
                let from = if flat.len() > 2 && r(attempt * 31 + k, n) < 0.25 { flat[(r(attempt, n + 70) * flat.len() as f32) as usize % flat.len()] } else { *flat.last().unwrap() };
                heading += (r(attempt, n + 140) - 0.5) * 2.4;
                let step = (4.0 + r(attempt, n + 210) * 5.0f32).to_radians();
                let at = from + glam::Vec2::from_angle(heading) * step;
                // Kept apart from each other, and within the figure.
                if at.length() > 16f32.to_radians() || flat.iter().any(|p| p.distance(at) < 2.6f32.to_radians()) {
                    continue;
                }
                flat.push(at);
            }
            if flat.len() < 4 {
                continue;
            }
            // About the figure's own middle, so its reach is as small as it can be.
            let centre = flat.iter().copied().sum::<glam::Vec2>() / flat.len() as f32;
            let brightest = (r(attempt, 4) * flat.len() as f32) as usize % flat.len();
            let stars: Vec<(Vec3, f32)> = flat
                .iter()
                .enumerate()
                .map(|(s, p)| {
                    let p = *p - centre;
                    let dir = (middle + east * p.x + north * p.y).normalize();
                    let bright = if s == brightest { 3.6 } else { 2.2 + r(attempt, 300 + s as i32) * 1.0 };
                    (dir, bright)
                })
                .collect();
            let reach = stars.iter().map(|(d, _)| d.dot(middle)).fold(1.0f32, f32::min) - 0.002;
            // Named apart from the others.
            let name = (0..24)
                .map(|salt| worldgen::names::sky::constellation(seed, (k as u32) * 32 + salt, tongue))
                .find(|n| !names.iter().any(|m| m == n || m.rsplit(' ').next() == n.rsplit(' ').next()))
                .unwrap_or_else(|| format!("The {} Stars", ["First", "Second", "Third", "Fourth", "Fifth", "Sixth", "Seventh"][figures.len()]));
            names.push(name.clone());
            figures.push(Figure { name, stars, middle, reach });
        }
        Self { figures }
    }

    /// What the sky shader is given.
    pub fn lights(&self) -> Lights {
        let mut l = Lights::default();
        for (k, f) in self.figures.iter().take(FIGURES).enumerate() {
            l.groups[k] = [f.middle.x, f.middle.y, f.middle.z, f.reach];
            for (s, (d, b)) in f.stars.iter().take(EACH).enumerate() {
                l.stars[k * EACH + s] = [d.x, d.y, d.z, *b];
            }
        }
        l
    }

    /// Where figure `k`'s middle is in the sky at `hour` on `date`.
    pub fn place(&self, k: usize, hour: f32, date: f32) -> Option<Vec3> {
        Some(in_sky(self.figures.get(k)?.middle, wheel(hour, date)))
    }

    /// The figures that someone looking along `forward`, with `half` the angle of their
    /// view across, makes out at `hour` on `date`, with the sun at `sun` (the sine of its
    /// height) and `cloud` of the sky clouded over: those well up in a dark, clear sky
    /// with the whole of them in view.
    pub fn in_sight(&self, hour: f32, date: f32, sun: f32, cloud: f32, forward: Vec3, half: f32) -> Vec<usize> {
        if sun > DARK || cloud > 0.45 {
            return Vec::new();
        }
        let angle = wheel(hour, date);
        let within = (half * 0.85).cos();
        (0..self.figures.len())
            .filter(|&k| {
                let f = &self.figures[k];
                f.stars.iter().all(|(d, _)| {
                    let at = in_sky(*d, angle);
                    at.y > WELL_UP * 0.8 && at.dot(forward) > within
                }) && in_sky(f.middle, angle).y > WELL_UP
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_world_has_the_same_figures_every_time_and_another_world_others() {
        let sky = Sky::of(1);
        assert_eq!(sky, Sky::of(1));
        assert_eq!(sky.figures.len(), FIGURES);
        assert_ne!(sky.figures[0].middle, Sky::of(2).figures[0].middle);
        assert_ne!(sky.figures.iter().map(|f| &f.name).collect::<Vec<_>>(), Sky::of(2).figures.iter().map(|f| &f.name).collect::<Vec<_>>());
        for seed in [1, 2, 3, 77, 9001] {
            let sky = Sky::of(seed);
            assert_eq!(sky.figures.len(), FIGURES, "seed {seed}");
            for (k, f) in sky.figures.iter().enumerate() {
                assert!((4..=EACH).contains(&f.stars.len()), "seed {seed}: {} has {} stars", f.name, f.stars.len());
                assert!(f.stars.iter().all(|(d, b)| (d.length() - 1.0).abs() < 1e-4 && d.dot(f.middle) >= f.reach && *b > 1.0));
                // Small enough to take in at a glance, and none on top of another.
                assert!(f.reach > 18f32.to_radians().cos(), "seed {seed}: {} is too wide", f.name);
                for (i, a) in f.stars.iter().enumerate() {
                    for b in &f.stars[i + 1..] {
                        assert!(a.0.dot(b.0) < 2.4f32.to_radians().cos());
                    }
                }
                for g in &sky.figures[k + 1..] {
                    assert!(f.middle.dot(g.middle) <= APART, "seed {seed}: {} and {} overlap", f.name, g.name);
                    assert_ne!(f.name, g.name);
                }
            }
        }
    }

    #[test]
    fn the_figures_wheel_with_the_night_and_drift_with_the_seasons() {
        let sky = Sky::of(5);
        // The shader turns a view into the sphere's frame; the figures come back out of it.
        let star = sky.figures[0].stars[0].0;
        let angle = wheel(22.0, 0.3);
        let (s, c) = angle.sin_cos();
        let seen = in_sky(star, angle);
        let back = Vec3::new(seen.x * c + seen.y * s, seen.y * c - seen.x * s, seen.z);
        assert!(back.distance(star) < 1e-5);
        // At ten at night, some are overhead in one season that are not in another.
        let up = |date: f32| (0..FIGURES).filter(|&k| sky.place(k, 22.0, date).unwrap().y > WELL_UP).collect::<Vec<_>>();
        let seasons: Vec<Vec<usize>> = [0.0, 0.25, 0.5, 0.75].into_iter().map(up).collect();
        assert!(seasons.windows(2).any(|w| w[0] != w[1]), "{seasons:?}");
        // From one night to the next they have barely moved.
        let (a, b) = (sky.place(0, 22.0, 0.3).unwrap(), sky.place(0, 22.0, 0.3 + 1.0 / 365.0).unwrap());
        assert!(a.dot(b) > 0.999);
    }

    #[test]
    fn a_figure_is_made_out_only_overhead_in_the_dark_and_the_clear() {
        let sky = Sky::of(3);
        // Lying back at some hour of the night, looking straight up at the highest.
        let (hour, date) = (23.0, 0.4);
        let k = (0..FIGURES).max_by(|&a, &b| sky.place(a, hour, date).unwrap().y.total_cmp(&sky.place(b, hour, date).unwrap().y)).unwrap();
        let at = sky.place(k, hour, date).unwrap();
        assert!(at.y > WELL_UP, "none is up at {hour} on {date}");
        let half = 35f32.to_radians();
        assert!(sky.in_sight(hour, date, -0.5, 0.1, at, half).contains(&k));
        // Not by day, nor under cloud, nor looking away from it.
        assert!(sky.in_sight(hour, date, 0.2, 0.1, at, half).is_empty());
        assert!(sky.in_sight(hour, date, -0.5, 0.9, at, half).is_empty());
        assert!(!sky.in_sight(hour, date, -0.5, 0.1, -at, half).contains(&k));
        let lights = sky.lights();
        assert_eq!(lights.groups[k][3], sky.figures[k].reach);
        assert!(lights.stars.iter().filter(|s| s[3] > 0.0).count() >= FIGURES * 4);
    }
}
