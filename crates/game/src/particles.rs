//! Ambient life: wind, and the small moving things that make a landscape feel alive.
//!
//! Particles are simulated on the CPU (a few thousand at most) and drawn as tiny cubes.
//! Each kind has its own spawn rule, tied to where the player is and the time of day.

use bytemuck::{Pod, Zeroable};
use glam::{DVec3, Vec3};
use voxel::Kind;
use worldgen::{
    Cascade,
    biome::Biome,
    noise,
    scale::{OLD_VOXEL as U, VOXELS_PER_METRE},
};

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
    /// How hard it blows beyond the usual, 0..1: a gale (see weather.rs).
    pub storm: f32,
}

impl Wind {
    pub fn new() -> Self {
        Self {
            vector: Vec3::ZERO,
            lagged: Vec3::ZERO,
            storm: 0.0,
        }
    }

    pub fn update(&mut self, time: f64, dt: f32) {
        let t = time * 0.05;
        let x = noise::perlin2(0x1d, t, 0.3) * 1.6;
        let z = noise::perlin2(0x2e, t, 7.1) * 1.6;
        // Gusts on top of the drift.
        let gust = 0.5 + 0.5 * noise::perlin2(0x3f, time * 0.4, 1.0);
        self.vector = Vec3::new(x, 0.0, z) * (0.8 + gust * 1.4) * (1.0 + self.storm * (0.8 + gust * 1.2));
        self.lagged += (self.vector - self.lagged) * (1.0 - (-0.4 * dt).exp());
    }
}

#[derive(Clone, Copy, PartialEq)]
enum PKind {
    Spray,
    Mist,
    Leaf,
    /// A fallen leaf floating on water, carried along by any current.
    Afloat,
    Firefly,
    Mote,
    Butterfly,
    Bird,
    Droplet,
    Rain,
    Snow,
}

/// What a falling leaf looks like, by the tree it came from.
#[derive(Clone, Copy, PartialEq)]
pub enum LeafShape {
    /// Needle flecks under spruce and pine.
    Needle,
    /// Long slips under willow, acacia and palm.
    Slip,
    /// Broad leaves: oak, birch, jungle, baobab.
    Broad,
    /// Big autumn leaves; these fall in heavier numbers.
    Autumn,
    /// Cherry blossom petals.
    Petal,
}

impl LeafShape {
    /// Size (width, thickness, length) in old voxels, and how many extra falls it brings.
    fn size(self) -> Vec3 {
        match self {
            LeafShape::Needle => Vec3::new(0.04, 0.03, 0.14),
            LeafShape::Slip => Vec3::new(0.07, 0.04, 0.36),
            LeafShape::Broad => Vec3::new(0.2, 0.06, 0.24),
            LeafShape::Autumn => Vec3::new(0.26, 0.06, 0.28),
            LeafShape::Petal => Vec3::new(0.1, 0.03, 0.1),
        }
    }
    fn weight(self) -> f32 {
        match self {
            LeafShape::Needle => 0.5,
            LeafShape::Slip => 0.8,
            LeafShape::Broad | LeafShape::Petal => 1.0,
            LeafShape::Autumn => 3.5,
        }
    }
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
    pub leaves: &'a [(DVec3, [f32; 3], LeafShape)],
    /// Flower voxels near the player.
    pub flowers: &'a [(DVec3, [f32; 3])],
    pub player_entered_water: Option<DVec3>,
    /// How much of the year's leaf fall is on now, 0 to 1 (see `season::leaf_fall`).
    pub leaf_fall: f32,
    /// Weather: rain and snow strength and how hard the wind blows beyond usual, 0..1.
    pub rain: f32,
    pub snow: f32,
    pub storm: f32,
    pub time: f64,
}

pub struct Particles {
    particles: Vec<Particle>,
    instances: Vec<ParticleInstance>,
    rng: u32,
    bird_timer: f32,
    pub wind: Wind,
}

const MAX_PARTICLES: usize = 8000;
/// Rain and snow alive at once at full strength.
const RAIN_COUNT: f32 = 2200.0;
const SNOW_COUNT: f32 = 1500.0;

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
        self.wind.storm = s.storm;
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
                // A fast river throws its spray further out over the drop.
                let speed = 1.2 + drop * 0.35 + c.speed * VOXELS_PER_METRE / U;
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
            let gust = self.wind.vector.length();
            // Gusts shake leaves loose far faster than a steady breeze does.
            let windiness = 0.3 + gust * 0.4 + (gust - 1.8).max(0.0) * 2.0;
            // Autumn groves (maple and the like) shed much more.
            let weight = s.leaves.iter().map(|l| l.2.weight()).sum::<f32>() / s.leaves.len() as f32
                // Leaves hold on through summer, let go in autumn, and few are left by winter.
                * (0.2 + 2.4 * s.leaf_fall);
            let rate = 2.5 * windiness * weight * (s.leaves.len() as f32 / 400.0).min(1.5);
            let n = (rate * dt + self.unit()).floor() as u32;
            for _ in 0..n {
                let (pos, color, shape) = s.leaves[(self.unit() * s.leaves.len() as f32) as usize % s.leaves.len()];
                let particle = Particle {
                    kind: PKind::Leaf,
                    pos: pos + DVec3::new(self.range(0.1, 0.9) as f64, -0.1, self.range(0.1, 0.9) as f64),
                    vel: Vec3::ZERO,
                    age: 0.0,
                    life: self.range(9.0, 14.0),
                    size: shape.size() * self.range(0.8, 1.2),
                    color,
                    phase: self.range(0.0, std::f32::consts::TAU),
                    settled: false,
                };
                self.spawn(particle);
            }
        }

        // Rain and snow fall around the player, from a little above, and stop where they land.
        let (wide, high) = (14.0 / 0.8, 14.0 / 0.8);
        for (kind, strength, count, life) in [(PKind::Rain, s.rain, RAIN_COUNT, 1.7), (PKind::Snow, s.snow, SNOW_COUNT, 8.0)] {
            let n = (strength * count / life * dt + self.unit()).floor() as u32;
            for _ in 0..n {
                let (dx, dz) = (self.range(-wide, wide), self.range(-wide, wide));
                let rain = kind == PKind::Rain;
                let particle = Particle {
                    kind,
                    pos: s.camera + DVec3::new(dx as f64 * U as f64, self.range(0.4, 1.0) as f64 * high as f64 * U as f64, dz as f64 * U as f64),
                    vel: Vec3::new(0.0, if rain { -self.range(10.0, 12.5) } else { -self.range(1.0, 1.7) }, 0.0),
                    age: 0.0,
                    life: life * 1.5,
                    size: if rain { Vec3::new(0.025, 0.4, 0.025) } else { Vec3::splat(self.range(0.04, 0.08)) },
                    color: if rain { [0.72, 0.8, 0.9] } else { [0.95, 0.97, 1.0] },
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
        let mut splashes: Vec<DVec3> = Vec::new();
        for p in &mut self.particles {
            p.age += dt;
            match p.kind {
                PKind::Rain | PKind::Snow => {
                    let rain = p.kind == PKind::Rain;
                    // Wind slants the rain and swings the snow about.
                    let sway = if rain { 0.0 } else { (t * 1.3 + p.phase).sin() * 0.5 };
                    let drift = wind * if rain { 1.2 } else { 0.7 };
                    p.vel.x += (drift.x + sway - p.vel.x) * (1.0 - (-2.0 * dt).exp());
                    p.vel.z += (drift.z + sway * 0.6 - p.vel.z) * (1.0 - (-2.0 * dt).exp());
                    p.pos += (p.vel * U * dt).as_dvec3();
                    let at = p.pos.floor().as_ivec3();
                    match voxels.kind(at.x, at.y, at.z) {
                        Kind::Solid | Kind::Foliage => p.age = p.life,
                        Kind::Water => {
                            p.age = p.life;
                            splashes.push(p.pos);
                        }
                        _ => {}
                    }
                }
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
                            // Leaves gather where they land and lie a good while.
                            p.life = p.age + 30.0;
                        }
                        Kind::Water => {
                            // It floats on the top voxel of the water, and goes where that takes it.
                            let mut top = below.y;
                            while top < below.y + 8 && voxels.kind(below.x, top + 1, below.z) == Kind::Water {
                                top += 1;
                            }
                            p.kind = PKind::Afloat;
                            p.pos = DVec3::new(next.x, top as f64 + 0.9, next.z);
                            p.vel = Vec3::ZERO;
                            p.life = p.age + 20.0;
                        }
                        _ => p.pos = next,
                    }
                }
                PKind::Afloat => {
                    // Carried at the speed of the current (metres a second, here in old
                    // voxels a second), with a breath of wind; on still water that is all.
                    let at = p.pos.floor().as_ivec3();
                    let flow = voxels.current(at.x, at.z) * (VOXELS_PER_METRE / U);
                    let target = Vec3::new(flow.x, 0.0, flow.y) + Vec3::new(wind.x, 0.0, wind.z) * 0.04;
                    p.vel += (target - p.vel) * (1.0 - (-1.5 * dt).exp());
                    // It turns slowly as it goes, more the faster the water.
                    p.phase += p.vel.length() * 0.25 * dt;
                    let next = p.pos + (p.vel * U * dt).as_dvec3();
                    let to = next.floor().as_ivec3();
                    if (to.x, to.z) == (at.x, at.z) {
                        p.pos = next;
                        continue;
                    }
                    // Into the next column only where the water there carries it on: at the
                    // same level, or down the step of a cascade. Against a bank it slides
                    // along whichever way is open, or waits there.
                    let tries = [(to.x, to.z), (to.x, at.z), (at.x, to.z)];
                    let open = tries.into_iter().filter(|&c| c != (at.x, at.z)).find_map(|(x, z)| water_top(voxels, x, at.y, z).map(|y| (x, z, y)));
                    match open {
                        Some((x, z, y)) => {
                            if x == at.x {
                                p.vel.x = 0.0;
                            }
                            if z == at.z {
                                p.vel.z = 0.0;
                            }
                            let keep = |v: f64, c: i32, a: i32| if c == a { v.clamp(a as f64 + 0.01, a as f64 + 0.99) } else { v };
                            p.pos = DVec3::new(keep(next.x, x, at.x), y as f64 + 0.9, keep(next.z, z, at.z));
                        }
                        None => p.vel = Vec3::ZERO,
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
        // A few of the drops that hit water throw up a speck.
        for (i, at) in splashes.into_iter().enumerate() {
            if i % 5 == 0 {
                let particle = Particle {
                    kind: PKind::Droplet,
                    pos: at,
                    vel: Vec3::new(self.range(-0.6, 0.6), self.range(1.0, 2.2), self.range(-0.6, 0.6)),
                    age: 0.0,
                    life: self.range(0.15, 0.3),
                    size: Vec3::splat(0.04),
                    color: [0.8, 0.9, 0.98],
                    phase: 0.0,
                    settled: false,
                };
                self.spawn(particle);
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
                PKind::Rain => {
                    // Lit by the day, not by the sun: streaks must read in dull light and dark.
                    // Drops this close would be thick white bars, so they fade out.
                    alpha *= 0.35 * near_fade(p.pos, s.camera);
                    glow = 0.25 + 0.9 * s.sun_elevation.clamp(0.0, 1.0);
                }
                PKind::Snow => {
                    alpha *= near_fade(p.pos, s.camera);
                    glow = 0.35 + 1.1 * s.sun_elevation.clamp(0.0, 1.0);
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

/// Rain and snow within a metre or two of the eye fade out: up close they are thick bars.
fn near_fade(pos: DVec3, camera: DVec3) -> f32 {
    let metres = (pos - camera).length() as f32 / worldgen::scale::VOXELS_PER_METRE;
    ((metres - 2.0) / 4.0).clamp(0.0, 1.0)
}

/// The top water voxel of column `(x, z)` for something floating in from a neighbour
/// whose top water voxel is `y`: the same level, or lower down a step. `None` where the
/// way is blocked by ground, or by water standing higher.
fn water_top(voxels: &mut impl Voxels, x: i32, y: i32, z: i32) -> Option<i32> {
    match voxels.kind(x, y, z) {
        Kind::Water => (voxels.kind(x, y + 1, z) != Kind::Water).then_some(y),
        Kind::Air => (1..=8).map(|d| y - d).find(|&y| voxels.kind(x, y, z) != Kind::Air).filter(|&y| voxels.kind(x, y, z) == Kind::Water),
        _ => None,
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

#[cfg(test)]
mod leaf_tests {
    use super::LeafShape::*;

    #[test]
    fn shapes_differ_and_autumn_falls_heavier() {
        assert!(Needle.size().z < Broad.size().z && Slip.size().z > Broad.size().z);
        assert!(Autumn.weight() > Broad.weight());
    }
}
