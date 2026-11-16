//! A murmuration: on the odd evening of late autumn, hundreds of starlings gathering
//! over the reeds or the fields before they go down to roost, wheeling as one cloud
//! that billows, folds and pours itself into new shapes.
//!
//! The birds do not flock as the flocks in particles.rs do, each minding its
//! neighbours: with hundreds that is too dear, and it is not what the eye sees anyway.
//! The cloud is a shape that moves, a ball of places stretched, turned, warped and
//! rippled by slow noise, and each bird makes for its own place in it, a little behind,
//! which turns the shape's changes into streams and swirls of birds. One small dark
//! cube a bird, drawn with the other particles.

use glam::{DVec3, Vec3};
use worldgen::{biome::Biome, noise, scale::VOXELS_PER_METRE};

use crate::particles::ParticleInstance;

/// Of the evenings when one could happen (late autumn, dry, over open country), how
/// many see one: few, so that it is something to have seen.
const CHANCE: f32 = 0.22;
/// The part of the year they gather in: late autumn, before winter.
const SEASON: (f32, f32) = (0.6, 0.72);
/// Metres off at which the birds are out of sight.
const SIGHT: f32 = 700.0;
/// How long the birds take to go down to the roost once it is dark, seconds.
const SETTLE: f32 = 28.0;

/// Whether this is country starlings roost in: reed beds, and open fields and grass.
pub fn roosting_country(biome: Biome) -> bool {
    matches!(biome, Biome::Marsh | Biome::Grassland | Biome::Steppe | Biome::Heath | Biome::Moor | Biome::Bog)
}

/// Whether the date and the hour are those of a murmuration: dusk in late autumn, from
/// a little before sunset until the light is gone.
pub fn the_hour(date: f32, hour: f32, sun_elevation: f32) -> bool {
    date > SEASON.0 && date < SEASON.1 && hour > 13.0 && hour < 21.0 && sun_elevation < 0.16 && sun_elevation > -0.1
}

struct Bird {
    pos: DVec3,
    vel: Vec3,
    /// Its place in the cloud's ball, before the cloud's shape is put on it.
    home: Vec3,
    flap: f32,
}

struct Murmuration {
    /// Over the roost, where the cloud wheels about, in voxels.
    roost: DVec3,
    /// The roost's ground.
    ground: f64,
    age: f32,
    /// Seconds into going down to roost, once it has begun.
    settling: Option<f32>,
    seed: f64,
    birds: Vec<Bird>,
}

/// Whether there is a murmuration this evening, and the birds when there is.
pub struct Starlings {
    show: Option<Murmuration>,
    /// Whether this evening has been decided, and how.
    tonight: Option<bool>,
    rng: u32,
    /// Make one happen whatever the date, the hour and the country (`--murmuration`),
    /// ahead of the camera along this way, flat.
    pub forced: Option<Vec3>,
}

impl Starlings {
    pub fn new() -> Self {
        Self { show: None, tonight: None, rng: 0x3c6e_f372, forced: None }
    }

    /// A murmuration gathering now ahead along `dir`, as `forced` has one, in place of
    /// any already up or any the evening has decided against.
    pub fn summon(&mut self, dir: Vec3) {
        self.forced = Some(dir);
        self.show = None;
        self.tonight = None;
    }

    fn unit(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        noise::unit(self.rng)
    }

    /// Moves the evening on. `calm` is whether the weather lets them gather (dry, and no
    /// gale); `land` the ground's height (particles::land_under).
    #[allow(clippy::too_many_arguments)]
    pub fn update(&mut self, dt: f32, camera: DVec3, biome: Biome, date: f32, hour: f32, sun_elevation: f32, calm: bool, land: &dyn Fn(f64, f64) -> Option<f64>) {
        let hour_now = the_hour(date, hour, sun_elevation) || self.forced.is_some();
        if !hour_now {
            self.tonight = None;
        } else if self.tonight.is_none() {
            // Once an evening, so that leaving and coming back does not roll again.
            let roll = self.unit();
            self.tonight = Some(self.forced.is_some() || (roosting_country(biome) && calm && roll < CHANCE));
        }
        if self.show.is_none() && self.tonight == Some(true) {
            self.begin(camera, land);
        }
        let Some(m) = &mut self.show else { return };
        m.age += dt;
        // Dark, or the weather turned: down they go.
        let done = sun_elevation < -0.08 && self.forced.is_none() || !calm && self.forced.is_none();
        if done && m.settling.is_none() {
            m.settling = Some(0.0);
        }
        if let Some(s) = &mut m.settling {
            *s += dt;
        }
        m.fly(dt);
        let far = ((m.roost - camera).length() as f32 / VOXELS_PER_METRE) > SIGHT + 300.0;
        if m.settling.is_some_and(|s| s > SETTLE) || far {
            self.show = None;
            // Once gone it is gone for the evening.
            self.tonight = self.tonight.map(|_| false);
            if self.forced.is_some() {
                self.tonight = None;
            }
        }
    }

    fn begin(&mut self, camera: DVec3, land: &dyn Fn(f64, f64) -> Option<f64>) {
        let vpm = VOXELS_PER_METRE as f64;
        // Off over the fields, far enough to be seen whole.
        let dir = match self.forced {
            Some(d) => DVec3::new(d.x as f64, 0.0, d.z as f64).normalize_or(DVec3::X),
            None => {
                let a = self.unit() as f64 * std::f64::consts::TAU;
                DVec3::new(a.cos(), 0.0, a.sin())
            }
        };
        let off = if self.forced.is_some() { 190.0 } else { 160.0 + self.unit() as f64 * 120.0 };
        let mut roost = camera + dir * off * vpm;
        let ground = land(roost.x, roost.z).unwrap_or(camera.y - 2.0 * vpm);
        roost.y = ground + (55.0 + self.unit() as f64 * 25.0) * vpm;
        let n = if self.forced.is_some() { 900 } else { 450 + (self.unit() * 500.0) as usize };
        let seed = self.unit() as f64 * 1000.0;
        let mut m = Murmuration { roost, ground, age: 0.0, settling: None, seed, birds: Vec::with_capacity(n) };
        for _ in 0..n {
            // Evenly through a ball, a little thicker toward the middle.
            let home = loop {
                let v = Vec3::new(self.unit() * 2.0 - 1.0, self.unit() * 2.0 - 1.0, self.unit() * 2.0 - 1.0);
                if v.length_squared() <= 1.0 {
                    break v * (0.4 + 0.6 * v.length());
                }
            };
            let flap = self.unit() * std::f32::consts::TAU;
            m.birds.push(Bird { pos: DVec3::ZERO, vel: Vec3::ZERO, home, flap });
        }
        // Already gathered when first seen, each in its place.
        for i in 0..m.birds.len() {
            let to = m.place(m.birds[i].home);
            m.birds[i].pos = to;
        }
        self.show = Some(m);
    }

    /// The birds, one dark cube apiece, camera-relative.
    pub fn draw(&self, camera: DVec3, out: &mut Vec<ParticleInstance>) {
        let Some(m) = &self.show else { return };
        let vpm = VOXELS_PER_METRE;
        // Going down, the last of them drop into the reeds out of sight.
        let fading = m.settling.map_or(1.0, |s| (1.0 - (s - SETTLE * 0.6) / (SETTLE * 0.4)).clamp(0.0, 1.0));
        for b in &m.birds {
            let rel = (b.pos - camera).as_vec3();
            let metres = rel.length() / vpm;
            let alpha = fading * (1.0 - ((metres - (SIGHT - 120.0)) / 120.0).clamp(0.0, 1.0));
            if alpha <= 0.0 {
                continue;
            }
            // Drawn bigger than life far off, as the flocks are, though less: it is
            // their numbers that make the cloud dark, not their size.
            let grow = (metres / 30.0).max(1.0).powf(0.72) * vpm;
            let speed = b.vel.length().max(1e-3);
            let pitch = -(b.vel.y / speed).clamp(-0.7, 0.7).asin();
            // Wings out and in: wide and then narrow seen from afar.
            let span = 0.38 * (0.45 + 0.55 * b.flap.sin().abs());
            out.push(ParticleInstance {
                pos: rel.to_array(),
                yaw: (-b.vel.x).atan2(b.vel.z),
                size: [span * grow, 0.05 * grow, 0.2 * grow],
                glow: 0.0,
                color: [0.025, 0.024, 0.028, alpha],
                tilt: [pitch, 0.0],
            });
        }
    }

    /// Where the murmuration is and how many birds it has, if there is one.
    pub fn now(&self) -> Option<(DVec3, usize)> {
        self.show.as_ref().map(|m| (m.roost, m.birds.len()))
    }
}

impl Murmuration {
    /// Where the bird with place `home` in the ball should be now, in world voxels.
    fn place(&self, home: Vec3) -> DVec3 {
        let vpm = VOXELS_PER_METRE;
        let (t, s) = (self.age as f64, self.seed);
        let n = |k: u32, rate: f64| noise::perlin2(k, s, t * rate) as f32;
        // The whole cloud: stretched long and short, flattened into a sheet and fattened
        // again, and turning, all slowly and each in its own time.
        let radii = Vec3::new(42.0 * (0.75 + 0.6 * n(0x51, 0.05)), 13.0 * (0.7 + 0.9 * n(0x52, 0.07)).max(0.25), 26.0 * (0.75 + 0.6 * n(0x53, 0.045)));
        let turn = 0.16 * t as f32 + 2.5 * n(0x54, 0.03);
        let lean = 0.25 * n(0x55, 0.06);
        let mut p = home * radii;
        // Billows: the ball's places pushed about by a slow warp, so it folds over on
        // itself and streams out at the edges.
        let (u, w) = (home.x as f64 * 1.3, home.z as f64 * 1.3 + home.y as f64 * 0.7);
        let warp = Vec3::new(
            noise::perlin2(0x61, u + t * 0.09 + s, w - t * 0.05) as f32,
            noise::perlin2(0x62, u - t * 0.07, w + t * 0.08 + s) as f32,
            noise::perlin2(0x63, w + t * 0.06 + s, u + t * 0.07) as f32,
        );
        p += warp * Vec3::new(26.0, 14.0, 22.0);
        // A wave running through it, which shows as a dark band where the birds bunch.
        let wave = (home.x * 3.1 + home.z * 1.7 - t as f32 * 1.1).sin();
        p.y += 5.0 * wave;
        p.x *= 1.0 + 0.18 * wave;
        // Leaning and turning about the middle.
        let (sl, cl) = lean.sin_cos();
        p = Vec3::new(p.x * cl - p.y * sl, p.x * sl + p.y * cl, p.z);
        let (st, ct) = turn.sin_cos();
        p = Vec3::new(p.x * ct - p.z * st, p.y, p.x * st + p.z * ct);
        // The middle wanders over the roost, and at the end comes down onto it, the cloud
        // pouring down into the reeds.
        let wander = Vec3::new(n(0x56, 0.025) * 60.0, n(0x57, 0.04) * 10.0, n(0x58, 0.025) * 60.0);
        let mut centre = self.roost + (wander * vpm).as_dvec3();
        if let Some(settle) = self.settling {
            let k = (settle / SETTLE).clamp(0.0, 1.0);
            centre.y += (self.ground + 1.0 * vpm as f64 - centre.y) * (k * k * (3.0 - 2.0 * k)) as f64;
            p *= 1.0 - 0.8 * k;
            // Funnelling: the lower the more they gather in.
            p.y = p.y.min(0.0) * (1.0 - k) + p.y.max(0.0) * (1.0 + k);
        }
        let mut at = centre + (p * vpm).as_dvec3();
        // Until they go down they keep well over the reeds: the cloud's underside skims
        // flat along above them rather than dipping into the ground.
        if self.settling.is_none() {
            at.y = at.y.max(self.ground + 12.0 * vpm as f64);
        }
        at
    }

    fn fly(&mut self, dt: f32) {
        let vpm = VOXELS_PER_METRE;
        for i in 0..self.birds.len() {
            let to = self.place(self.birds[i].home);
            let b = &mut self.birds[i];
            // Each makes for its place with a little lag: a spring, nearly damped, so the
            // cloud's turns and folds become streams of birds catching up.
            let off = (to - b.pos).as_vec3() / vpm;
            let acc = off * 3.2 - b.vel * 2.6;
            b.vel += acc * dt;
            // Starlings fly at twenty metres a second or so, never hang still.
            let speed = b.vel.length();
            if speed > 24.0 {
                b.vel *= 24.0 / speed;
            }
            b.pos += (b.vel * vpm * dt).as_dvec3();
            b.flap += dt * if b.vel.y > 0.0 { 15.0 * std::f32::consts::TAU } else { 9.0 * std::f32::consts::TAU };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LATE_AUTUMN: f32 = 0.66;

    #[test]
    fn only_at_dusk_in_late_autumn() {
        assert!(the_hour(LATE_AUTUMN, 17.0, 0.05));
        assert!(!the_hour(LATE_AUTUMN, 7.0, 0.05), "not at dawn");
        assert!(!the_hour(LATE_AUTUMN, 15.0, 0.4), "not in the day");
        assert!(!the_hour(0.3, 19.5, 0.05), "not in summer");
        assert!(!the_hour(0.85, 16.0, 0.05), "not in deep winter");
        assert!(roosting_country(Biome::Marsh) && !roosting_country(Biome::Taiga));
    }

    #[test]
    fn rare_and_once_an_evening() {
        let mut seen = 0;
        let mut s = Starlings::new();
        for evening in 0..400 {
            let mut any = false;
            for k in 0..60 {
                s.update(0.5, DVec3::ZERO, Biome::Marsh, LATE_AUTUMN, 17.0 + k as f32 * 0.01, 0.05, true, &|_, _| Some(0.0));
                any |= s.now().is_some();
            }
            // Night falls, and the evening's chance with it.
            s.update(0.5, DVec3::ZERO, Biome::Marsh, LATE_AUTUMN, 23.0, -0.4, true, &|_, _| Some(0.0));
            s.show = None;
            seen += any as u32;
            let _ = evening;
        }
        assert!(seen > 40 && seen < 140, "{seen} murmurations in 400 evenings");
        // Never in woods, nor in the rain.
        let mut s = Starlings::new();
        for (biome, calm) in [(Biome::Taiga, true), (Biome::Marsh, false)] {
            for _ in 0..400 {
                s.update(0.5, DVec3::ZERO, biome, LATE_AUTUMN, 17.0, 0.05, calm, &|_, _| Some(0.0));
                assert!(s.now().is_none());
                s.update(0.5, DVec3::ZERO, biome, LATE_AUTUMN, 23.0, -0.4, calm, &|_, _| Some(0.0));
            }
        }
    }

    #[test]
    fn the_cloud_holds_together_and_keeps_moving_then_goes_to_roost() {
        let mut s = Starlings::new();
        s.forced = Some(Vec3::X);
        let land = |_: f64, _: f64| Some(0.0);
        let vpm = VOXELS_PER_METRE as f64;
        let mut last: Vec<DVec3> = Vec::new();
        for step in 0..60 * 90 {
            s.update(1.0 / 60.0, DVec3::ZERO, Biome::Marsh, LATE_AUTUMN, 17.0, 0.05, true, &land);
            if step % 600 == 599 {
                let m = s.show.as_ref().unwrap();
                let mid = m.birds.iter().map(|b| b.pos).sum::<DVec3>() / m.birds.len() as f64;
                // One cloud, never scattered across the sky, and well clear of the ground.
                let spread = m.birds.iter().map(|b| (b.pos - mid).length() / vpm).fold(0.0, f64::max);
                assert!(spread < 140.0, "a bird {spread} m from the middle");
                assert!(m.birds.iter().all(|b| b.pos.y > 5.0 * vpm), "a bird near the ground");
                // Its shape keeps changing: the birds do not hold their places.
                let now: Vec<DVec3> = m.birds.iter().map(|b| b.pos - mid).collect();
                if !last.is_empty() {
                    let moved = now.iter().zip(&last).map(|(a, b): (&DVec3, &DVec3)| (*a - *b).length() / vpm).sum::<f64>() / now.len() as f64;
                    assert!(moved > 4.0, "the cloud hardly changed: {moved} m");
                }
                last = now;
            }
        }
        let mut out = Vec::new();
        s.draw(DVec3::ZERO, &mut out);
        assert!(out.len() >= 800, "{} birds drawn", out.len());
        // At dark they go down, and are gone.
        s.forced = None;
        for _ in 0..60 * 40 {
            s.update(1.0 / 60.0, DVec3::ZERO, Biome::Marsh, LATE_AUTUMN, 19.0, -0.12, true, &land);
        }
        assert!(s.now().is_none());
    }
}
