//! Ambient life: wind, and the small moving things that make a landscape feel alive.
//!
//! Particles are simulated on the CPU (a few thousand at most) and drawn as tiny cubes.
//! Each kind has its own spawn rule, tied to where the player is and the time of day.

use bytemuck::{Pod, Zeroable};
use glam::{DVec3, Vec3};
use voxel::Kind;
use worldgen::{Cascade, biome::Biome, noise, scale::OLD_VOXEL as U};

use crate::player::Voxels;

/// GPU instance for one particle: a cube at `pos` (camera-relative).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct ParticleInstance {
    pub pos: [f32; 3],
    /// Rotation about the vertical axis, radians.
    pub yaw: f32,
    pub size: [f32; 3],
    /// Brightness multiplier for things that glow; 0 means lit by the sun like terrain.
    pub glow: f32,
    /// Linear rgb and alpha.
    pub color: [f32; 4],
}

/// Prevailing wind: a slowly wandering vector, as in nature never quite steady.
pub struct Wind {
    pub vector: Vec3,
    /// The same, lagging a little behind: for things that respond slowly.
    pub lagged: Vec3,
}

impl Wind {
    pub fn new() -> Self {
        Self {
            vector: Vec3::ZERO,
            lagged: Vec3::ZERO,
        }
    }

    pub fn update(&mut self, time: f64, dt: f32) {
        let t = time * 0.05;
        let x = noise::perlin2(0x1d, t, 0.3) * 1.6;
        let z = noise::perlin2(0x2e, t, 7.1) * 1.6;
        // Gusts on top of the drift.
        let gust = 0.5 + 0.5 * noise::perlin2(0x3f, time * 0.4, 1.0);
        self.vector = Vec3::new(x, 0.0, z) * (0.8 + gust * 1.4);
        self.lagged += (self.vector - self.lagged) * (1.0 - (-0.4 * dt).exp());
    }
}

#[derive(Clone, Copy, PartialEq)]
enum PKind {
    Spray,
    Mist,
    Leaf,
    Firefly,
    Mote,
    Butterfly,
    Bird,
    Droplet,
}

struct Particle {
    kind: PKind,
    pos: DVec3,
    vel: Vec3,
    age: f32,
    life: f32,
    size: Vec3,
    color: [f32; 3],
    /// Per-particle random phase for sway, blinking, flapping.
    phase: f32,
    /// Leaves that have come to rest.
    settled: bool,
}

/// What the spawners need to know about the player's surroundings this frame.
pub struct Surroundings<'a> {
    pub camera: DVec3,
    pub biome: Biome,
    /// Sun elevation (sine), negative at night.
    pub sun_elevation: f32,
    pub cascades: &'a [Cascade],
    /// Leaf voxels near the player, as (position, colour), refreshed by the caller.
    pub leaves: &'a [(DVec3, [f32; 3])],
    /// Flower voxels near the player.
    pub flowers: &'a [(DVec3, [f32; 3])],
    pub player_entered_water: Option<DVec3>,
    pub time: f64,
}

pub struct Particles {
    particles: Vec<Particle>,
    instances: Vec<ParticleInstance>,
    rng: u32,
    bird_timer: f32,
    pub wind: Wind,
}

const MAX_PARTICLES: usize = 6000;

impl Particles {
    pub fn new() -> Self {
        Self {
            particles: Vec::new(),
            instances: Vec::new(),
            rng: 0x2545_f491,
            bird_timer: 20.0,
            wind: Wind::new(),
        }
    }

    fn unit(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        noise::unit(self.rng)
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }

    fn count(&self, kind: PKind) -> usize {
        self.particles.iter().filter(|p| p.kind == kind).count()
    }

    fn spawn(&mut self, p: Particle) {
        if self.particles.len() < MAX_PARTICLES {
            self.particles.push(p);
        }
    }

    pub fn update(&mut self, dt: f32, s: &Surroundings, voxels: &mut impl Voxels) {
        self.wind.update(s.time, dt);
        let day = s.sun_elevation > 0.0;
        let dusk = s.sun_elevation < 0.08;

        // --- Spawning -------------------------------------------------------------
        // Falls: spray thrown off the lip, mist rising from the plunge.
        for c in s.cascades {
            let dist = (c.pos.as_dvec3() - s.camera).length() as f32;
            if dist > 140.0 * U {
                continue;
            }
            // Rates and speeds were tuned against the drop in old voxels.
            let drop = c.drop / U;
            let near = 1.0 - dist / (150.0 * U);
            // Finer voxels mean more lips along the same waterfall.
            let rate = (drop.min(6.0) * 2.5 + 1.0) * near / U;
            let n = (rate * dt + self.unit()).floor() as u32;
            for _ in 0..n {
                let side = Vec3::new(-c.dir.z, 0.0, c.dir.x) * self.range(-0.5, 0.5) * U;
                let speed = 1.2 + drop * 0.35;
                let particle = Particle {
                    kind: PKind::Spray,
                    pos: c.pos.as_dvec3() + side.as_dvec3() + DVec3::new(0.0, 0.1 * U as f64, 0.0),
                    vel: c.dir * (speed * self.range(0.6, 1.2)) + Vec3::Y * self.range(0.8, 1.6 + drop * 0.4)
                        + Vec3::new(self.range(-0.4, 0.4), 0.0, self.range(-0.4, 0.4)),
                    age: 0.0,
                    life: self.range(0.7, 1.4),
                    size: Vec3::splat(self.range(0.1, 0.22)),
                    color: [0.85, 0.92, 0.98],
                    phase: 0.0,
                    settled: false,
                };
                self.spawn(particle);
            }
            if drop >= 3.0 && self.unit() < dt * drop * 0.6 * near / U {
                let foot = c.pos + c.dir * (1.5 + drop * 0.3) * U - Vec3::Y * c.drop;
                let particle = Particle {
                    kind: PKind::Mist,
                    pos: foot.as_dvec3() + (Vec3::new(self.range(-1.5, 1.5), 0.3, self.range(-1.5, 1.5)) * U).as_dvec3(),
                    vel: Vec3::new(self.range(-0.3, 0.3), self.range(0.5, 1.1), self.range(-0.3, 0.3)),
                    age: 0.0,
                    life: self.range(2.0, 3.5),
                    size: Vec3::splat(self.range(0.5, 1.0)),
                    color: [0.9, 0.94, 1.0],
                    phase: 0.0,
                    settled: false,
                };
                self.spawn(particle);
            }
        }

        // Leaves let go in the wind.
        if !s.leaves.is_empty() {
            let windiness = 0.3 + self.wind.vector.length() * 0.4;
            let rate = 2.5 * windiness * (s.leaves.len() as f32 / 400.0).min(1.5);
            let n = (rate * dt + self.unit()).floor() as u32;
            for _ in 0..n {
                let (pos, color) = s.leaves[(self.unit() * s.leaves.len() as f32) as usize % s.leaves.len()];
                let particle = Particle {
                    kind: PKind::Leaf,
                    pos: pos + DVec3::new(self.range(0.1, 0.9) as f64, -0.1, self.range(0.1, 0.9) as f64),
                    vel: Vec3::ZERO,
                    age: 0.0,
                    life: self.range(9.0, 14.0),
                    size: Vec3::new(0.22, 0.08, 0.22),
                    color,
                    phase: self.range(0.0, std::f32::consts::TAU),
                    settled: false,
                };
                self.spawn(particle);
            }
        }

        // Fireflies in warm, damp country at dusk and through the night.
        let firefly_country = matches!(
            s.biome,
            Biome::Deciduous | Biome::Grassland | Biome::OldGrowth | Biome::Woodland | Biome::Rainforest
        );
        if dusk && firefly_country && self.count(PKind::Firefly) < 120 && self.unit() < dt * 7.0 {
            let (x, z) = (self.range(-26.0, 26.0) * U, self.range(-26.0, 26.0) * U);
            if let Some(ground) = ground_below(voxels, s.camera + DVec3::new(x as f64, (12.0 * U) as f64, z as f64), (40.0 * U) as i32) {
                let particle = Particle {
                    kind: PKind::Firefly,
                    pos: ground + DVec3::new(0.0, (self.range(0.8, 3.0) * U) as f64, 0.0),
                    vel: Vec3::ZERO,
                    age: 0.0,
                    life: self.range(14.0, 26.0),
                    size: Vec3::splat(0.24),
                    color: if self.unit() < 0.85 { [0.75, 1.0, 0.25] } else { [0.45, 1.0, 0.35] },
                    phase: self.range(0.0, 100.0),
                    settled: false,
                };
                self.spawn(particle);
            }
        }

        // Pollen and dust catching the daylight.
        if day && self.count(PKind::Mote) < 160 && self.unit() < dt * 6.0 {
            let (x, z) = (self.range(-14.0, 14.0) * U, self.range(-14.0, 14.0) * U);
            if let Some(ground) = ground_below(voxels, s.camera + DVec3::new(x as f64, (10.0 * U) as f64, z as f64), (30.0 * U) as i32) {
                let particle = Particle {
                    kind: PKind::Mote,
                    pos: ground + DVec3::new(0.0, (self.range(0.4, 5.0) * U) as f64, 0.0),
                    vel: Vec3::ZERO,
                    age: 0.0,
                    life: self.range(5.0, 9.0),
                    size: Vec3::splat(0.05),
                    color: [1.0, 0.95, 0.75],
                    phase: self.range(0.0, std::f32::consts::TAU),
                    settled: false,
                };
                self.spawn(particle);
            }
        }

        // Butterflies over the flowers.
        if day && !s.flowers.is_empty() && self.count(PKind::Butterfly) < 18 && self.unit() < dt * 0.8 {
            let (pos, color) = s.flowers[(self.unit() * s.flowers.len() as f32) as usize % s.flowers.len()];
            let particle = Particle {
                kind: PKind::Butterfly,
                pos: pos + DVec3::new(0.5, (1.2 * U) as f64, 0.5),
                vel: Vec3::ZERO,
                age: 0.0,
                life: self.range(18.0, 30.0),
                size: Vec3::new(0.3, 0.05, 0.22),
                color: [color[0] * 0.8 + 0.2, color[1] * 0.8 + 0.1, color[2] * 0.8 + 0.1],
                phase: self.range(0.0, std::f32::consts::TAU),
                settled: false,
            };
            self.spawn(particle);
        }

        // Now and then a flock passes high overhead.
        self.bird_timer -= dt;
        if day && self.bird_timer <= 0.0 {
            self.bird_timer = self.range(35.0, 80.0);
            let heading = self.range(0.0, std::f32::consts::TAU);
            let dir = Vec3::new(heading.cos(), 0.0, heading.sin());
            let start = s.camera - (dir * 220.0 * U).as_dvec3() + DVec3::new(0.0, (self.range(45.0, 90.0) * U) as f64, 0.0);
            let n = 5 + (self.unit() * 5.0) as i32;
            for i in 0..n {
                // A V: alternate sides, each bird further back and out.
                let k = ((i + 1) / 2) as f32;
                let side = if i % 2 == 0 { 1.0 } else { -1.0 };
                let offset = -dir * (k * 2.2) + Vec3::new(-dir.z, 0.0, dir.x) * (k * 1.6 * side);
                let particle = Particle {
                    kind: PKind::Bird,
                    pos: start + (offset * U).as_dvec3(),
                    vel: dir * 9.0,
                    age: 0.0,
                    life: 55.0,
                    size: Vec3::new(0.5, 0.12, 0.9),
                    color: [0.08, 0.08, 0.1],
                    phase: self.range(0.0, std::f32::consts::TAU),
                    settled: false,
                };
                self.spawn(particle);
            }
        }

        // Splash when the player goes in.
        if let Some(at) = s.player_entered_water {
            for _ in 0..28 {
                let a = self.range(0.0, std::f32::consts::TAU);
                let r = self.range(1.0, 3.0);
                let particle = Particle {
                    kind: PKind::Droplet,
                    pos: at,
                    vel: Vec3::new(a.cos() * r, self.range(2.5, 5.0), a.sin() * r),
                    age: 0.0,
                    life: self.range(0.5, 0.9),
                    size: Vec3::splat(self.range(0.08, 0.18)),
                    color: [0.8, 0.9, 0.98],
                    phase: 0.0,
                    settled: false,
                };
                self.spawn(particle);
            }
        }

        // --- Simulation -----------------------------------------------------------
        let wind = self.wind.vector;
        let t = s.time as f32;
        for p in &mut self.particles {
            p.age += dt;
            match p.kind {
                PKind::Spray | PKind::Droplet => {
                    p.vel.y -= 9.0 * dt;
                    p.pos += (p.vel * U * dt).as_dvec3();
                }
                PKind::Mist => {
                    p.vel += (wind * 0.3 - p.vel * 0.5) * dt;
                    p.vel.y = p.vel.y.max(0.3);
                    p.pos += (p.vel * U * dt).as_dvec3();
                    p.size += Vec3::splat(0.25 * dt);
                }
                PKind::Leaf => {
                    if p.settled {
                        continue;
                    }
                    // A hover, then a slow swaying fall carried along by the wind.
                    let fall = if p.age < 0.6 { 0.0 } else { -1.1 };
                    let sway = (t * 2.2 + p.phase).sin() * 0.8;
                    let drift = wind * 0.6;
                    p.vel += (Vec3::new(drift.x + sway, fall, drift.z + sway * 0.5) - p.vel) * (1.0 - (-3.0 * dt).exp());
                    let next = p.pos + (p.vel * U * dt).as_dvec3();
                    let below = next.floor().as_ivec3();
                    match voxels.kind(below.x, below.y, below.z) {
                        Kind::Solid => {
                            p.settled = true;
                            p.pos.y = below.y as f64 + 1.02;
                            p.life = p.age + 6.0;
                        }
                        Kind::Water => {
                            p.settled = true;
                            p.pos = next;
                            p.life = p.age + 5.0;
                        }
                        _ => p.pos = next,
                    }
                }
                PKind::Firefly => {
                    // Two layers of wandering: broad drift and fine jitter.
                    let n1 = Vec3::new(
                        noise::perlin2(11, t as f64 * 0.25 + p.phase as f64, 0.0),
                        noise::perlin2(12, t as f64 * 0.25 + p.phase as f64, 3.0),
                        noise::perlin2(13, t as f64 * 0.25 + p.phase as f64, 6.0),
                    );
                    let n2 = Vec3::new(
                        noise::perlin2(14, t as f64 * 1.8 + p.phase as f64, 0.0),
                        noise::perlin2(15, t as f64 * 1.8 + p.phase as f64, 3.0),
                        noise::perlin2(16, t as f64 * 1.8 + p.phase as f64, 6.0),
                    );
                    p.vel = n1 * 1.2 + n2 * 0.5;
                    p.pos += (p.vel * U * dt).as_dvec3();
                }
                PKind::Mote => {
                    let sway = Vec3::new((t * 0.7 + p.phase).sin(), (t * 0.9 + p.phase * 1.3).cos() * 0.6, (t * 0.8 + p.phase * 0.7).cos());
                    p.vel = wind * 0.35 + sway * 0.25;
                    p.pos += (p.vel * U * dt).as_dvec3();
                }
                PKind::Butterfly => {
                    let wander = Vec3::new((t * 0.6 + p.phase).sin(), 0.0, (t * 0.5 + p.phase * 1.7).cos()) * 1.1;
                    let bob = (t * 7.0 + p.phase).sin() * 0.9;
                    p.vel += (Vec3::new(wander.x, bob + 0.1, wander.z) - p.vel) * (1.0 - (-4.0 * dt).exp());
                    p.pos += (p.vel * U * dt).as_dvec3();
                    // Wings: the body flattens and widens as it flaps.
                    p.size.x = 0.18 + 0.2 * ((t * 14.0 + p.phase).sin()).abs();
                }
                PKind::Bird => {
                    p.pos += (p.vel * U * dt).as_dvec3();
                    p.size.x = 0.3 + 0.5 * ((t * 6.0 + p.phase).sin()).abs();
                }
            }
        }
        self.particles.retain(|p| p.age < p.life);

        // --- Instances --------------------------------------------------------------
        self.instances.clear();
        for p in &self.particles {
            let fade_in = (p.age / 0.2).min(1.0);
            let fade_out = ((p.life - p.age) / 0.8).clamp(0.0, 1.0);
            let mut alpha = fade_in * fade_out;
            let mut glow = 0.0;
            let mut color = p.color;
            match p.kind {
                PKind::Mist => alpha *= 0.16,
                PKind::Spray | PKind::Droplet => alpha *= 0.75,
                PKind::Firefly => {
                    // Long dark gaps and short bright blinks.
                    let cycle = (t * 0.55 + p.phase).rem_euclid(3.2);
                    let on = if cycle < 1.1 { (cycle * std::f32::consts::PI / 1.1).sin() } else { 0.0 };
                    if on <= 0.02 {
                        continue;
                    }
                    alpha *= on;
                    glow = 18.0 * on;
                }
                PKind::Mote => {
                    glow = 1.4;
                    color = [1.0, 0.95, 0.75];
                }
                _ => {}
            }
            self.instances.push(ParticleInstance {
                pos: (p.pos - s.camera).as_vec3().to_array(),
                yaw: p.phase,
                size: (p.size * U).to_array(),
                glow,
                color: [color[0], color[1], color[2], alpha],
            });
        }
    }

    pub fn instances(&self) -> &[ParticleInstance] {
        &self.instances
    }

    pub fn len(&self) -> usize {
        self.particles.len()
    }
}

/// Top of the ground at the column under `from`, searching downward.
fn ground_below(voxels: &mut impl Voxels, from: DVec3, depth: i32) -> Option<DVec3> {
    let (x, z) = (from.x.floor() as i32, from.z.floor() as i32);
    let top = from.y.floor() as i32;
    let mut prev = Kind::Air;
    for y in (top - depth..=top).rev() {
        let k = voxels.kind(x, y, z);
        if k == Kind::Solid && prev == Kind::Air {
            return Some(DVec3::new(x as f64 + 0.5, y as f64 + 1.0, z as f64 + 0.5));
        }
        if k == Kind::Water {
            return None;
        }
        prev = k;
    }
    None
}
