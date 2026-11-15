//! Ambient life: wind, and the small moving things that make a landscape feel alive.
//!
//! Particles are simulated on the CPU (a few thousand at most) and drawn as tiny cubes.
//! Each kind has its own spawn rule, tied to where the player is and the time of day.

use bytemuck::{Pod, Zeroable};
use glam::{DVec3, Vec2, Vec3, Vec4};
use voxel::Kind;
use worldgen::{
    Cascade, SpringMark, Surf,
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
    /// Pitch (about the cube's own x) and roll (about its own z), radians, turned
    /// before the yaw: a leaf tumbling, a wing beating.
    pub tilt: [f32; 2],
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
    Droplet,
    Rain,
    Snow,
    /// A speck thrown up where a raindrop lands.
    Splash,
    /// Rising from a spring's floor; it pops into a ripple at the surface.
    Bubble,
    /// A speck of a ring spreading out across a spring's surface.
    Ripple,
    /// Thrown up by a fire, bright for a moment.
    Spark,
    /// Rising from a fire and thinning as it goes.
    Smoke,
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
    /// Size (width, thickness, length) in old voxels.
    fn size(self) -> Vec3 {
        match self {
            // A needle: a thin sliver, several times longer than it is wide.
            LeafShape::Needle => Vec3::new(0.022, 0.018, 0.2),
            LeafShape::Slip => Vec3::new(0.07, 0.03, 0.36),
            LeafShape::Broad => Vec3::new(0.2, 0.03, 0.24),
            LeafShape::Autumn => Vec3::new(0.26, 0.03, 0.28),
            LeafShape::Petal => Vec3::new(0.1, 0.02, 0.1),
        }
    }
    /// How fast it falls in still air, old voxels a second: a needle drops, a petal floats.
    fn fall(self) -> f32 {
        match self {
            LeafShape::Needle => 1.9,
            LeafShape::Slip => 1.5,
            LeafShape::Broad => 1.15,
            LeafShape::Autumn => 1.05,
            LeafShape::Petal => 0.75,
        }
    }
    /// Whether it is long and thin, and so spins about its length as it falls rather
    /// than swinging from side to side.
    fn spins(self) -> bool {
        matches!(self, LeafShape::Needle | LeafShape::Slip)
    }
}

/// A place in a tree's crown that leaves fall from.
pub struct LeafSite {
    /// The top leaf voxel of the crown here (its corner).
    pub pos: DVec3,
    /// The height of the crown's underside, where it is open below.
    pub under: Option<f64>,
    /// The leaf's colour as modelled, linear.
    pub color: [f32; 3],
    pub shape: LeafShape,
    /// Its plant kind (voxel::plant), for its year.
    pub plant: u8,
    /// Its tree's tag (features::tree_tag), 0 if not known.
    pub tag: u8,
}

/// How readily this site sheds a leaf on date `s`, and the colour of the leaf: that of
/// the leaves still on its tree, autumn's gold and red included. A tree sheds most in
/// the days before its own leaves are down, and a bare one sheds none.
fn shedding(site: &LeafSite, s: f32, leaf_fall: f32) -> (f32, [f32; 3]) {
    use voxel::plant;
    let c = Vec3::from(site.color);
    match site.plant {
        plant::DECIDUOUS | plant::AUTUMN => {
            let maple = site.plant == plant::AUTUMN;
            let vox = site.pos.floor().as_ivec3();
            let leaf = crate::season::leaf(s, vox, maple, site.tag);
            if !leaf.keep {
                return (0.0, site.color);
            }
            let colour = crate::season::leaf_colour(c, &leaf, maple, s);
            // Turned leaves let go; most of all in the last days before the tree is bare.
            let last = smooth(leaf.fall_at - 0.035, leaf.fall_at, s);
            let w = 0.03 + 2.2 * leaf.turn * leaf.turn + 4.5 * last;
            (if maple { w * 1.3 } else { w }, colour.to_array())
        }
        // Evergreens drop a few old needles all year, a few more in autumn.
        _ if site.shape == LeafShape::Needle => (0.06 + 0.3 * leaf_fall, site.color),
        _ => (0.04 + 0.15 * leaf_fall, site.color),
    }
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
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
    /// A falling leaf's heading, pitch and roll.
    rot: Vec3,
    /// A falling leaf's ways: swing rate, spiral rate, tumble rate, and how fast it falls.
    spin: Vec4,
}

/// What the spawners need to know about the player's surroundings this frame.
pub struct Surroundings<'a> {
    pub camera: DVec3,
    pub biome: Biome,
    /// Sun elevation (sine), negative at night.
    pub sun_elevation: f32,
    pub cascades: &'a [Cascade],
    /// Spring pools near the player.
    pub springs: &'a [SpringMark],
    /// Where the sea breaks on rock near the player.
    pub surf: &'a [Surf],
    /// Places in the trees near the player that leaves fall from, refreshed by the caller.
    pub leaves: &'a [LeafSite],
    /// Flowers in bloom near the player: the voxel each stands in, and its colour.
    pub flowers: &'a [(DVec3, [f32; 3])],
    /// Fires near the player, out to [`SMOKE_SIGHT`]: the heart of each, and how much it
    /// smokes, 1 for one burning.
    pub fires: &'a [(DVec3, f32)],
    pub player_entered_water: Option<DVec3>,
    /// How much of the year's leaf fall is on now, 0 to 1 (see `season::leaf_fall`).
    pub leaf_fall: f32,
    /// The time of year, 0 to 1 (see season.rs), which each tree's leaves follow.
    pub date: f32,
    /// Weather: rain and snow strength and how hard the wind blows beyond usual, 0..1.
    pub rain: f32,
    pub snow: f32,
    pub storm: f32,
    pub time: f64,
    /// The height of the land at a place, roughly (see [`land_under`]), or None where it
    /// is not known yet: what flocks keep above.
    pub land: &'a dyn Fn(f64, f64) -> Option<f64>,
}

/// The height of the land around `(x, z)` from the continent's map, in voxels: the
/// highest of the map's cells around it, so a flock kept above it clears the hills on it
/// wherever between the cells they rise. None until the continent there is ready.
pub fn land_under(world: &worldgen::world::GameWorld, x: f64, z: f64) -> Option<f64> {
    use worldgen::macro_layout::{MACRO_SIZE, cell_of};
    let (mx, mz) = cell_of(x.floor() as i32, z.floor() as i32);
    let c = world.continent_if_ready(mx, mz)?;
    let cs = c.cell_size() as f64;
    let (lx, lz) = ((x - (mx * MACRO_SIZE) as f64) / cs, (z - (mz * MACRO_SIZE) as f64) / cs);
    let max = c.size as i32 - 1;
    let mut highest = f32::MIN;
    for gz in (lz.floor() as i32 - 1).clamp(0, max)..=(lz.floor() as i32 + 1).clamp(0, max) {
        for gx in (lx.floor() as i32 - 1).clamp(0, max)..=(lx.floor() as i32 + 1).clamp(0, max) {
            highest = highest.max(c.height[gz as usize * c.size + gx as usize]);
        }
    }
    Some((highest * worldgen::scale::OLD_VOXEL) as f64)
}

pub struct Particles {
    particles: Vec<Particle>,
    instances: Vec<ParticleInstance>,
    rng: u32,
    bird_timer: f32,
    flocks: Vec<Flock>,
    /// Each leaf site's shedding weight and leaf colour this frame (see `shedding`).
    shed: Vec<(f32, [f32; 3])>,
    pub wind: Wind,
}

const MAX_PARTICLES: usize = 8000;
/// Leaves in the air at once, at most; more wait their turn on the tree.
const MAX_FALLING: usize = 1500;
/// How far off (voxels, across) a falling leaf is let go of, being out of sight.
const LEAF_REACH: f64 = 110.0 * VOXELS_PER_METRE as f64;
/// The colours butterflies come in: whites, a sulphur yellow, oranges, a blue and a brown,
/// as the common ones of meadows are.
const BUTTERFLY_COLOURS: [[f32; 3]; 7] = [
    [0.95, 0.95, 0.9],
    [0.93, 0.9, 0.55],
    [0.95, 0.55, 0.15],
    [0.85, 0.35, 0.1],
    [0.45, 0.6, 0.95],
    [0.5, 0.33, 0.2],
    [0.98, 0.98, 0.98],
];

/// How far off (voxels) a raindrop's splash is still drawn.
const SPLASH_REACH: f64 = 11.0 * VOXELS_PER_METRE as f64;
/// Rain and snow alive at once at full strength.
const RAIN_COUNT: f32 = 2200.0;
const SNOW_COUNT: f32 = 1500.0;
/// How far off a fire's smoke is still seen, in metres: a column of it is how a camp
/// is told from across a valley.
pub const SMOKE_SIGHT: f64 = 600.0;
/// How far off its sparks are, in metres, and the finer puffs low down.
const SPARK_SIGHT: f64 = 90.0;
/// Puffs of smoke in the air at once, at most, so that a row of fires leaves room for
/// the rain and the leaves.
const MAX_SMOKE: usize = 1400;

impl Particles {
    pub fn new() -> Self {
        Self {
            particles: Vec::new(),
            instances: Vec::new(),
            rng: 0x2545_f491,
            bird_timer: 20.0,
            flocks: Vec::new(),
            shed: Vec::new(),
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
                // A fast river throws its spray further out over the drop, but only a
                // little: it stays close to the sheet of falling water.
                let speed = (0.6 + drop * 0.15 + 0.5 * c.speed * VOXELS_PER_METRE / U).min(2.2);
                let particle = Particle {
                    kind: PKind::Spray,
                    pos: c.pos.as_dvec3() + side.as_dvec3() + DVec3::new(0.0, 0.1 * U as f64, 0.0),
                    vel: c.dir * (speed * self.range(0.6, 1.2)) + Vec3::Y * self.range(0.8, 1.6 + drop * 0.4)
                        + Vec3::new(self.range(-0.25, 0.25), 0.0, self.range(-0.25, 0.25)),
                    age: 0.0,
                    life: self.range(0.7, 1.4),
                    size: Vec3::splat(self.range(0.1, 0.22)),
                    color: [0.85, 0.92, 0.98],
                    phase: 0.0,
                    settled: false,
                    rot: Vec3::ZERO,
                    spin: Vec4::ZERO,
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
                    rot: Vec3::ZERO,
                    spin: Vec4::ZERO,
                };
                self.spawn(particle);
            }
        }

        // Surf: now and then a wave breaks on the rocks and throws up a burst of spray,
        // more often and higher in wind and storm.
        let rough = 1.0 + self.wind.vector.length() * 0.3 + s.storm * 2.5;
        for sf in s.surf {
            let dist = (sf.pos.as_dvec3() - s.camera).length() as f32;
            if dist > 60.0 * VOXELS_PER_METRE || self.unit() >= dt * 0.12 * rough {
                continue;
            }
            let n = (6.0 + 6.0 * rough.min(3.0)) as u32;
            for _ in 0..n {
                let up = self.range(1.5, 3.0) * rough.sqrt();
                let side = Vec3::new(-sf.dir.z, 0.0, sf.dir.x) * self.range(-1.2, 1.2) * U;
                let particle = Particle {
                    kind: PKind::Spray,
                    pos: sf.pos.as_dvec3() + side.as_dvec3() + DVec3::new(0.0, 0.2, 0.0),
                    vel: sf.dir * self.range(0.2, 1.2) + Vec3::Y * up + Vec3::new(self.range(-0.5, 0.5), 0.0, self.range(-0.5, 0.5)),
                    age: 0.0,
                    life: self.range(0.6, 1.2),
                    size: Vec3::splat(self.range(0.1, 0.24)),
                    color: [0.92, 0.95, 0.98],
                    phase: 0.0,
                    settled: false,
                    rot: Vec3::ZERO,
                    spin: Vec4::ZERO,
                };
                self.spawn(particle);
            }
        }

        // Springs: now and then a bubble wells up from the floor of the pool.
        for sp in s.springs {
            let dist = (sp.pos.as_dvec3() - s.camera).length() as f32;
            if dist > 40.0 * VOXELS_PER_METRE {
                continue;
            }
            if self.unit() < dt * 1.5 {
                let (a, r) = (self.range(0.0, std::f32::consts::TAU), sp.radius * 0.6 * self.unit().sqrt());
                let particle = Particle {
                    kind: PKind::Bubble,
                    pos: sp.pos.as_dvec3() + DVec3::new((a.cos() * r) as f64, -1.0 - self.range(0.3, 1.2) as f64, (a.sin() * r) as f64),
                    vel: Vec3::new(0.0, self.range(0.8, 1.2), 0.0),
                    age: 0.0,
                    life: 6.0,
                    size: Vec3::splat(self.range(0.05, 0.1)),
                    color: [0.85, 0.94, 1.0],
                    phase: self.range(0.0, std::f32::consts::TAU),
                    settled: false,
                    rot: Vec3::ZERO,
                    spin: Vec4::ZERO,
                };
                self.spawn(particle);
            }
        }

        // Leaves let go in the wind, each tree on its own date and in its own colours.
        let falling = self.particles.iter().filter(|p| p.kind == PKind::Leaf && !p.settled).count();
        if !s.leaves.is_empty() && falling < MAX_FALLING {
            let gust = self.wind.vector.length();
            // Gusts shake leaves loose far faster than a steady breeze does.
            let windiness = 0.4 + gust * 0.35 + (gust - 1.8).max(0.0) * 1.5;
            self.shed.clear();
            self.shed.extend(s.leaves.iter().map(|l| shedding(l, s.date, s.leaf_fall)));
            let total: f32 = self.shed.iter().map(|w| w.0).sum();
            let most = self.shed.iter().map(|w| w.0).fold(0.0, f32::max);
            let rate = 0.15 * windiness * total;
            let n = (rate * dt + self.unit()).floor() as u32;
            for _ in 0..n {
                // A site by its weight: try a few at random, keeping each by how much it sheds.
                let mut pick = None;
                for _ in 0..12 {
                    let i = (self.unit() * s.leaves.len() as f32) as usize % s.leaves.len();
                    if self.unit() * most < self.shed[i].0 {
                        pick = Some(i);
                        break;
                    }
                }
                let Some(i) = pick else {
                    continue;
                };
                let (site, color) = (&s.leaves[i], self.shed[i].1);
                // Most fall from under the crown, where they come into view at once.
                let y = match site.under {
                    Some(under) if self.unit() < 0.75 => under - 0.15,
                    _ => site.pos.y - 0.1,
                };
                let shape = site.shape;
                // Each leaf a little lighter or darker, as the cubes of its tree are.
                let shade = self.range(0.88, 1.1);
                let tumble = if self.unit() < 0.5 { 1.0 } else { -1.0 };
                let particle = Particle {
                    kind: PKind::Leaf,
                    pos: DVec3::new(site.pos.x + self.range(0.1, 0.9) as f64, y, site.pos.z + self.range(0.1, 0.9) as f64),
                    vel: Vec3::ZERO,
                    age: 0.0,
                    // It lives until it comes down (see the simulation).
                    life: f32::INFINITY,
                    size: shape.size() * self.range(0.8, 1.2),
                    color: color.map(|c| c * shade),
                    phase: self.range(0.0, std::f32::consts::TAU),
                    settled: false,
                    rot: Vec3::new(self.range(0.0, std::f32::consts::TAU), 0.0, self.range(-1.0, 1.0)),
                    spin: Vec4::new(
                        // Swinging from side to side, about once every two or three seconds.
                        self.range(1.8, 3.2) * if shape == LeafShape::Petal { 1.4 } else { 1.0 },
                        // The heading turns slowly, so the swings trace a spiral.
                        self.range(0.25, 1.1) * if self.unit() < 0.5 { 1.0 } else { -1.0 },
                        // Spinning: needles and slips quickly about their length.
                        tumble * if shape.spins() { self.range(9.0, 16.0) } else { self.range(4.0, 8.0) },
                        shape.fall() * self.range(0.85, 1.15),
                    ),
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
                    rot: Vec3::ZERO,
                    spin: Vec4::ZERO,
                };
                self.spawn(particle);
            }
        }

        // Fires: sparks that fly up and wink out, and smoke that leans with the wind and
        // goes up a long way, so that a fire is seen from far off by its column of smoke.
        // Beyond where the sparks show, fewer and bigger puffs make the same column.
        let mut smoke = if s.fires.is_empty() { 0 } else { self.count(PKind::Smoke) };
        for &(fire, strength) in s.fires {
            let metres = (fire - s.camera).length() / VOXELS_PER_METRE as f64;
            if metres > SMOKE_SIGHT {
                continue;
            }
            let near = metres <= SPARK_SIGHT;
            if near && strength >= 1.0 && self.unit() < dt * 5.0 {
                let particle = Particle {
                    kind: PKind::Spark,
                    pos: fire + (Vec3::new(self.range(-0.12, 0.12), self.range(0.0, 0.2), self.range(-0.12, 0.12)) * U).as_dvec3(),
                    vel: Vec3::new(self.range(-0.35, 0.35), self.range(1.2, 2.6), self.range(-0.35, 0.35)),
                    age: 0.0,
                    life: self.range(0.6, 1.5),
                    size: Vec3::splat(self.range(0.04, 0.07)),
                    color: [1.0, self.range(0.45, 0.7), 0.12],
                    phase: self.range(0.0, 100.0),
                    settled: false,
                    rot: Vec3::ZERO,
                    spin: Vec4::ZERO,
                };
                self.spawn(particle);
            }
            let rate = if near { 8.0 } else { 4.0 } * strength;
            if smoke < MAX_SMOKE && self.unit() < dt * rate {
                smoke += 1;
                let shade = self.range(0.16, 0.3);
                // Smoke from a fire gone out is thin, and soon gone.
                let life = self.range(24.0, 34.0) * (0.3 + 0.65 * strength.min(1.0));
                let size = if near { self.range(0.12, 0.2) } else { self.range(0.6, 0.9) };
                let particle = Particle {
                    kind: PKind::Smoke,
                    pos: fire + (Vec3::new(self.range(-0.1, 0.1), self.range(0.3, 0.5), self.range(-0.1, 0.1)) * U).as_dvec3(),
                    vel: Vec3::new(self.range(-0.1, 0.1), self.range(1.4, 2.0), self.range(-0.1, 0.1)),
                    age: 0.0,
                    life,
                    size: Vec3::splat(size),
                    color: [shade, shade, shade * 1.05],
                    phase: self.range(0.0, std::f32::consts::TAU),
                    settled: false,
                    rot: Vec3::ZERO,
                    spin: Vec4::ZERO,
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
                    rot: Vec3::ZERO,
                    spin: Vec4::ZERO,
                };
                self.spawn(particle);
            }
        }

        // Pollen and dust catching the daylight, which rain and snow lay.
        let dry = s.rain + s.snow < 0.05;
        if day && dry && self.count(PKind::Mote) < 160 && self.unit() < dt * 6.0 {
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
                    rot: Vec3::ZERO,
                    spin: Vec4::ZERO,
                };
                self.spawn(particle);
            }
        }

        // Butterflies over the flowers, while they are out and the day is dry: a few, seen
        // now and then, not a cloud of them. Each is a colour of its own, never near the
        // flower it rose from, so it shows against the flowers instead of hiding in them.
        let in_flower = s.date > 0.08 && s.date < 0.66;
        if day && in_flower && dry && !s.flowers.is_empty() && self.count(PKind::Butterfly) < 6 && self.unit() < dt * 0.2 {
            let (pos, flower) = s.flowers[(self.unit() * s.flowers.len() as f32) as usize % s.flowers.len()];
            let first = (self.unit() * BUTTERFLY_COLOURS.len() as f32) as usize;
            let color = (0..BUTTERFLY_COLOURS.len())
                .map(|i| BUTTERFLY_COLOURS[(first + i) % BUTTERFLY_COLOURS.len()])
                .find(|c| (0..3).map(|k| (c[k] - flower[k]).powi(2)).sum::<f32>() > 0.12)
                .unwrap_or(BUTTERFLY_COLOURS[0]);
            let particle = Particle {
                kind: PKind::Butterfly,
                pos: pos + DVec3::new(0.5, (1.2 * U) as f64, 0.5),
                vel: Vec3::ZERO,
                age: 0.0,
                life: self.range(18.0, 30.0),
                size: Vec3::new(0.17, 0.02, 0.2),
                color,
                phase: self.range(0.0, std::f32::consts::TAU),
                settled: false,
                rot: Vec3::ZERO,
                spin: Vec4::ZERO,
            };
            self.spawn(particle);
        }

        // Now and then a flock passes overhead.
        self.bird_timer -= dt;
        if day && self.bird_timer <= 0.0 {
            self.bird_timer = self.range(25.0, 60.0);
            if self.flocks.len() < MAX_FLOCKS {
                self.spawn_flock(s.camera, s.land);
            }
        }
        for flock in &mut self.flocks {
            flock.update(dt, s.land);
        }
        self.flocks.retain(|f| !f.gone(s.camera));

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
                    rot: Vec3::ZERO,
                    spin: Vec4::ZERO,
                };
                self.spawn(particle);
            }
        }

        // --- Simulation -----------------------------------------------------------
        let wind = self.wind.vector;
        let t = s.time as f32;
        // Where raindrops landed this frame, and whether on water.
        let mut splashes: Vec<(DVec3, bool)> = Vec::new();
        let mut pops: Vec<DVec3> = Vec::new();
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
                    let hit = voxels.kind(at.x, at.y, at.z);
                    if matches!(hit, Kind::Solid | Kind::Foliage | Kind::Water) {
                        p.age = p.life;
                        // A drop splashes on the top of what it lands on.
                        if rain {
                            splashes.push((DVec3::new(p.pos.x, at.y as f64 + 1.03, p.pos.z), hit == Kind::Water));
                        }
                    }
                }
                PKind::Bubble => {
                    // A wobbling rise; at the top of the water it pops.
                    let wobble = (t * 9.0 + p.phase).sin() * 0.12;
                    p.pos += (Vec3::new(wobble, p.vel.y, wobble * 0.7) * U * dt).as_dvec3();
                    let at = p.pos.floor().as_ivec3();
                    if voxels.kind(at.x, at.y, at.z) == Kind::Air {
                        p.age = p.life;
                        pops.push(DVec3::new(p.pos.x, p.pos.y.floor() + 0.02, p.pos.z));
                    }
                }
                PKind::Ripple => {
                    p.pos += (p.vel * U * dt).as_dvec3();
                    p.vel *= (-1.5 * dt).exp();
                }
                PKind::Spray | PKind::Droplet | PKind::Splash => {
                    p.vel.y -= 9.0 * dt;
                    p.pos += (p.vel * U * dt).as_dvec3();
                }
                PKind::Spark => {
                    // Light enough to be tossed about by the fire's own draught.
                    let toss = Vec3::new((t * 7.0 + p.phase).sin(), 0.0, (t * 6.0 + p.phase * 1.7).cos());
                    p.vel += (wind * 0.5 + toss * 1.5 - p.vel * 0.8) * dt;
                    p.pos += (p.vel * U * dt).as_dvec3();
                }
                PKind::Smoke => {
                    // It rises fast off the fire, slows to a steady climb, and goes the
                    // way the wind does, spreading as it goes up.
                    let curl = Vec3::new((p.age * 1.3 + p.phase).sin(), 0.0, (p.age * 1.1 + p.phase * 2.0).cos()) * 0.25;
                    p.vel += (Vec3::new(wind.x * 0.45, 1.5, wind.z * 0.45) + curl - p.vel) * (0.6 * dt);
                    p.pos += (p.vel * U * dt).as_dvec3();
                    p.size += Vec3::splat(0.28 * dt);
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
                    // A leaf falls one of two ways, and changes between them as it goes:
                    // swinging from side to side like a pendulum, banking at each end of
                    // its swing, or spinning end over end and gliding off sideways as it
                    // does. Its heading wanders round, so either way it spirals. A needle
                    // or a slip of a leaf spins about its length nearly all the way down.
                    let a = p.age;
                    let [rate, spiral, spin, fall] = p.spin.to_array();
                    let gust = wind.length();
                    let wander = noise::perlin2(0x5a, (p.phase * 13.0) as f64, (a * 0.35) as f64);
                    let spinning = if p.size.z > p.size.x * 2.5 { wander > -0.6 } else { wander > 0.2 - gust * 0.08 };
                    p.rot.x += (spiral + wander * 0.8) * dt;
                    let across = Vec3::new(p.rot.x.cos(), 0.0, p.rot.x.sin());
                    let (swing, back) = (a * rate + p.phase).sin_cos();
                    let (want_pitch, glide, drop) = if spinning {
                        // Rolling over and over; the cube looks the same turned half round,
                        // so the roll is kept within a half turn either side.
                        p.rot.z = (p.rot.z + spin * dt + std::f32::consts::FRAC_PI_2).rem_euclid(std::f32::consts::PI)
                            - std::f32::consts::FRAC_PI_2;
                        (0.4 * (a * rate * 0.7 + p.phase).sin(), across * 0.5 * spin.signum(), fall * 1.25)
                    } else {
                        // Tipped furthest at either end of the swing, where it stalls, and
                        // falling fastest through the middle.
                        let roll = -0.9 * swing;
                        p.rot.z += (roll - p.rot.z) * (1.0 - (-6.0 * dt).exp());
                        (0.35 * back, across * (1.2 * back), fall * (0.45 + 0.8 * (1.0 - swing.abs())))
                    };
                    p.rot.y += (want_pitch - p.rot.y) * (1.0 - (-5.0 * dt).exp());
                    // It hangs a moment as it lets go.
                    let start = (a / 0.5).min(1.0);
                    let flutter = Vec3::new(
                        noise::perlin2(0x5b, (p.phase * 7.0) as f64, (a * 1.3) as f64),
                        0.0,
                        noise::perlin2(0x5c, (p.phase * 7.0) as f64, (a * 1.3) as f64),
                    ) * (0.3 + gust * 0.15);
                    let target = glide + flutter + Vec3::new(wind.x, 0.0, wind.z) * 0.6 - Vec3::Y * drop * start;
                    p.vel += (target - p.vel) * (1.0 - (-3.5 * dt).exp());
                    // Long gone from view, or caught somewhere for good: let it go.
                    let off = p.pos - s.camera;
                    if off.x.abs().max(off.z.abs()) > LEAF_REACH || a > 240.0 {
                        p.life = p.age;
                        continue;
                    }
                    let next = p.pos + (p.vel * U * dt).as_dvec3();
                    let below = next.floor().as_ivec3();
                    match voxels.kind(below.x, below.y, below.z) {
                        Kind::Solid => {
                            p.settled = true;
                            p.pos.y = below.y as f64 + 1.02;
                            // It lies flat where it lands, and a while later fades.
                            p.rot.y = 0.0;
                            p.rot.z = 0.0;
                            p.life = p.age + 14.0 + 8.0 * (p.phase / std::f32::consts::TAU);
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
                    // It heads the way it goes, turning no faster than a butterfly can.
                    if p.vel.x.hypot(p.vel.z) > 0.05 {
                        let want = (-p.vel.x).atan2(p.vel.z);
                        let off = (want - p.rot.x + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
                        p.rot.x += off * (1.0 - (-6.0 * dt).exp());
                    }
                }
            }
        }
        // Each drop that lands throws up a few specks, which hop and fall back; on water
        // some leave a ring as well. Only those near enough to be seen as more than rain.
        for (i, (at, water)) in splashes.into_iter().enumerate() {
            if (at - s.camera).length() > SPLASH_REACH {
                continue;
            }
            for _ in 0..if self.unit() < 0.5 { 2 } else { 3 } {
                let (a, out, up) = (self.range(0.0, std::f32::consts::TAU), self.range(0.3, 1.1), self.range(1.3, 2.6));
                let particle = Particle {
                    kind: PKind::Splash,
                    pos: at,
                    vel: Vec3::new(a.cos() * out, up, a.sin() * out),
                    age: 0.0,
                    // Until it is back down where it began (it falls at 9 a second squared).
                    life: 2.0 * up / 9.0,
                    size: Vec3::splat(self.range(0.03, 0.055)),
                    color: [0.8, 0.88, 0.96],
                    phase: a,
                    settled: false,
                    rot: Vec3::ZERO,
                    spin: Vec4::ZERO,
                };
                self.spawn(particle);
            }
            if water && i % 3 == 0 {
                for k in 0..6 {
                    let a = k as f32 * std::f32::consts::TAU / 6.0;
                    let particle = Particle {
                        kind: PKind::Ripple,
                        pos: at,
                        vel: Vec3::new(a.cos(), 0.0, a.sin()) * 0.5,
                        age: 0.0,
                        life: 0.6,
                        size: Vec3::new(0.06, 0.02, 0.06),
                        color: [0.9, 0.96, 1.0],
                        phase: a,
                        settled: false,
                        rot: Vec3::ZERO,
                        spin: Vec4::ZERO,
                    };
                    self.spawn(particle);
                }
            }
        }
        // Each bubble that reaches the surface spreads a small ring.
        for at in pops {
            for k in 0..8 {
                let a = k as f32 * std::f32::consts::TAU / 8.0;
                let particle = Particle {
                    kind: PKind::Ripple,
                    pos: at,
                    vel: Vec3::new(a.cos(), 0.0, a.sin()) * 0.7,
                    age: 0.0,
                    life: 0.9,
                    size: Vec3::new(0.08, 0.02, 0.08),
                    color: [0.9, 0.96, 1.0],
                    phase: a,
                    settled: false,
                    rot: Vec3::ZERO,
                    spin: Vec4::ZERO,
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
                // Smoke thins all the way up; a spark burns out.
                PKind::Smoke => {
                    alpha = fade_in * 0.32 * (1.0 - p.age / p.life).powf(0.9);
                    // Far off, the haze takes the edge off thin smoke and fewer puffs
                    // make the column: each is thicker, so the column still shows.
                    let metres = (p.pos - s.camera).length() as f32 / VOXELS_PER_METRE;
                    alpha = (alpha * (1.0 + ((metres - 40.0) / 70.0).clamp(0.0, 3.0))).min(0.9);
                    // Seen by the day's light, and low down by the fire's own.
                    glow = 0.06 + 0.9 * s.sun_elevation.clamp(0.0, 1.0) + 0.5 * (1.0 - p.age / 1.6).max(0.0);
                }
                PKind::Spark => {
                    glow = 5.5 * (1.0 - p.age / p.life).sqrt();
                    alpha = 1.0;
                }
                PKind::Spray | PKind::Droplet => alpha *= 0.75,
                PKind::Bubble => alpha *= 0.6,
                // A ripple fades as it spreads.
                PKind::Ripple => alpha = 0.5 * (1.0 - p.age / p.life),
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
                PKind::Splash => {
                    // Lit as the rain is, and gone as it comes down again.
                    alpha = 0.55 * (1.0 - p.age / p.life).sqrt();
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
            let (yaw, tilt) = match p.kind {
                PKind::Butterfly => {
                    // Two wings hinged along the body, clapping up over it and down flat.
                    let up = 0.15 + 1.1 * (0.5 + 0.5 * (t * 16.0 + p.phase).sin());
                    let rel = (p.pos - s.camera).as_vec3();
                    for side in [1.0f32, -1.0] {
                        let roll = side * up;
                        let centre = Vec3::new(side * roll.cos(), side * roll.sin(), 0.0) * (p.size.x * 0.5 * U);
                        self.instances.push(ParticleInstance {
                            pos: (rel + turn(centre, p.rot.x, [0.0, 0.0])).to_array(),
                            yaw: p.rot.x,
                            size: (p.size * U).to_array(),
                            glow,
                            color: [color[0], color[1], color[2], alpha],
                            tilt: [0.0, roll],
                        });
                    }
                    // A dark body between them.
                    self.instances.push(ParticleInstance {
                        pos: rel.to_array(),
                        yaw: p.rot.x,
                        size: [0.03 * U, 0.03 * U, p.size.z * U],
                        glow,
                        color: [0.03, 0.025, 0.02, alpha],
                        tilt: [0.0, 0.0],
                    });
                    continue;
                }
                PKind::Leaf => {
                    // Fading slowly where it lies.
                    if p.settled {
                        alpha = fade_in * ((p.life - p.age) / 3.0).clamp(0.0, 1.0);
                    }
                    (p.rot.x, [p.rot.y, p.rot.z])
                }
                _ => (p.phase, [0.0; 2]),
            };
            self.instances.push(ParticleInstance {
                pos: (p.pos - s.camera).as_vec3().to_array(),
                yaw,
                size: (p.size * U).to_array(),
                glow,
                color: [color[0], color[1], color[2], alpha],
                tilt,
            });
        }
        for flock in &self.flocks {
            flock.draw(s.camera, &mut self.instances);
        }
    }

    /// Where each flock in the sky is, its birds' middle, and how many birds it has.
    pub fn flocks(&self) -> Vec<(DVec3, usize)> {
        self.flocks.iter().map(|f| (f.birds.iter().map(|b| b.pos).sum::<DVec3>() / f.birds.len().max(1) as f64, f.birds.len())).collect()
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

// --- Birds ---------------------------------------------------------------------------

/// One bird of a flock, in voxels and voxels a second.
struct Bird {
    pos: DVec3,
    vel: Vec3,
    /// Through its wing beat, radians.
    flap: f32,
    /// Wing beats a second, as radians a second.
    rate: f32,
    /// How far its wings are raised now, radians.
    wing: f32,
    /// Leaning into a turn, radians.
    bank: f32,
    /// Its heading last frame, for how fast it is turning.
    yaw: f32,
    /// Its place in a V, behind and to one side of the lead (across, back), voxels.
    slot: Vec3,
    seed: f32,
}

/// Birds flying together overhead. Big ones (geese, gulls) keep a loose V and beat their
/// wings slowly; small ones bunch, stream and wheel about, beating fast between glides.
struct Flock {
    large: bool,
    /// The point the flock is making for: it runs on ahead of the birds.
    lead: DVec3,
    heading: f32,
    /// Voxels a second.
    speed: f32,
    seed: f32,
    age: f32,
    /// Bird length and half wingspan in metres.
    body: f32,
    span: f32,
    body_color: [f32; 3],
    wing_color: [f32; 3],
    birds: Vec<Bird>,
}

/// At most this many flocks in the sky at once.
const MAX_FLOCKS: usize = 2;
/// Metres off at which a flock is out of sight and let go.
const FLOCK_REACH: f32 = 380.0;

impl Particles {
    fn spawn_flock(&mut self, camera: DVec3, land: &dyn Fn(f64, f64) -> Option<f64>) {
        let vpm = VOXELS_PER_METRE;
        // Big birds a third of the time, and their flocks are smaller.
        let large = self.unit() < 0.4;
        let n = if large {
            // A pair, a skein of a dozen, or anything between.
            2 + (self.unit().powf(1.5) * 13.0) as usize
        } else {
            6 + (self.unit().powf(1.3) * 40.0) as usize
        };
        let heading = self.range(0.0, std::f32::consts::TAU);
        let dir = Vec3::new(heading.cos(), 0.0, heading.sin());
        let side = Vec3::new(-dir.z, 0.0, dir.x);
        let height = if large { self.range(35.0, 120.0) } else { self.range(18.0, 70.0) };
        // From out of sight on one side, passing over or near the player, and never
        // starting inside a hill out there.
        let mut start = camera - (dir * self.range(230.0, 300.0) * vpm).as_dvec3()
            + (side * self.range(-70.0, 70.0) * vpm).as_dvec3()
            + DVec3::new(0.0, (height * vpm) as f64, 0.0);
        if let Some(ground) = land(start.x, start.z) {
            start.y = start.y.max(ground + (Flock::clearance(large) + height * 0.5 * vpm) as f64);
        }
        let speed = if large { self.range(13.0, 17.0) } else { self.range(10.0, 14.0) } * vpm;
        let (body, span) = if large { (self.range(0.6, 0.9), self.range(0.65, 0.85)) } else { (0.22, 0.2) };
        // Geese and cormorants dark, gulls pale; small birds near black against the sky.
        let pale = large && self.unit() < 0.35;
        let (body_color, wing_color) = if pale {
            ([0.62, 0.62, 0.6], [0.5, 0.5, 0.5])
        } else if large {
            ([0.07, 0.06, 0.05], [0.1, 0.09, 0.08])
        } else {
            ([0.035, 0.035, 0.04], [0.045, 0.045, 0.05])
        };
        let mut birds = Vec::with_capacity(n);
        for i in 0..n {
            // A V for big birds: alternate sides, each further back and out, one arm
            // often longer than the other. Small birds start in a loose ball.
            let slot = if large {
                let k = ((i + 1) / 2) as f32;
                let s = if i % 2 == 0 { 1.0 } else { -1.0 };
                Vec3::new(k * self.range(1.6, 2.1) * s, self.range(-0.3, 0.3), k * self.range(1.7, 2.3)) * span * 2.0 * vpm
            } else {
                Vec3::ZERO
            };
            let scatter = if large {
                Vec3::ZERO
            } else {
                Vec3::new(self.range(-1.0, 1.0), self.range(-0.5, 0.5), self.range(-1.0, 1.0)) * 8.0 * vpm
            };
            let place = side * slot.x - dir * slot.z + Vec3::Y * slot.y + scatter;
            birds.push(Bird {
                pos: start + place.as_dvec3(),
                vel: dir * speed,
                flap: self.range(0.0, std::f32::consts::TAU),
                rate: if large { self.range(2.2, 2.9) } else { self.range(9.0, 12.0) } * std::f32::consts::TAU,
                wing: 0.0,
                bank: 0.0,
                yaw: heading,
                slot,
                seed: self.range(0.0, 1000.0),
            });
        }
        let seed = self.range(0.0, 1000.0);
        self.flocks.push(Flock {
            large,
            lead: start,
            heading,
            speed,
            seed,
            age: 0.0,
            body,
            span,
            body_color,
            wing_color,
            birds,
        });
    }
}

impl Flock {
    /// How high over the land, in voxels, a flock keeps at the least.
    fn clearance(large: bool) -> f32 {
        if large { 40.0 * VOXELS_PER_METRE } else { 25.0 * VOXELS_PER_METRE }
    }

    /// Keeps the lead clear of the land ahead: it rises to pass over a hill in its way,
    /// and where the land ahead climbs more steeply than it can, it turns toward the
    /// lower side, as birds go round a mountain rather than into it.
    fn keep_clear(&mut self, dt: f32, land: &dyn Fn(f64, f64) -> Option<f64>) {
        let vpm = VOXELS_PER_METRE as f64;
        let clear = Self::clearance(self.large) as f64;
        let dir = DVec3::new(self.heading.cos() as f64, 0.0, self.heading.sin() as f64);
        let side = DVec3::new(-dir.z, 0.0, dir.x);
        let lead = self.lead;
        let ahead = |d: f64, off: f64| land(lead.x + (dir.x * d + side.x * off) * vpm, lead.z + (dir.z * d + side.z * off) * vpm);
        let mut need = f64::MIN;
        let mut steep = false;
        for d in [0.0, 30.0, 60.0, 100.0] {
            let Some(ground) = ahead(d, 0.0) else { continue };
            let rise = ground + clear - lead.y;
            need = need.max(rise);
            // More to climb than it could by the time it got there, rising at a third of
            // its speed.
            if rise > d.max(10.0) * vpm / 3.0 {
                steep = true;
            }
        }
        if need > 0.0 {
            self.lead.y += need.min((self.speed * 0.5 * dt) as f64);
        }
        if steep {
            let (left, right) = (ahead(60.0, -60.0), ahead(60.0, 60.0));
            let way = if left.unwrap_or(f64::MAX) < right.unwrap_or(f64::MAX) { -1.0 } else { 1.0 };
            self.heading += way * 0.8 * dt;
        }
    }

    fn update(&mut self, dt: f32, land: &dyn Fn(f64, f64) -> Option<f64>) {
        self.keep_clear(dt, land);
        let vpm = VOXELS_PER_METRE;
        self.age += dt;
        let (a, seed) = (self.age as f64, self.seed as f64);
        // The lead wheels: big birds in long gentle curves, small ones in sudden sweeps
        // and rolls, rising and falling as they go.
        let turn = if self.large {
            0.06 * noise::perlin2(0x6a, seed, a * 0.05)
        } else {
            0.55 * noise::perlin2(0x6b, seed, a * 0.12) + 0.35 * noise::perlin2(0x6c, seed, a * 0.4)
        };
        self.heading += turn as f32 * dt;
        let dir = Vec3::new(self.heading.cos(), 0.0, self.heading.sin());
        let side = Vec3::new(-dir.z, 0.0, dir.x);
        let climb = if self.large { 0.0 } else { noise::perlin2(0x6d, seed, a * 0.15) as f32 * 3.0 * vpm };
        let lead_vel = dir * self.speed + Vec3::Y * climb;
        self.lead += (lead_vel * dt).as_dvec3();

        let n = self.birds.len();
        let centre = self.birds.iter().map(|b| b.pos).sum::<DVec3>() / n.max(1) as f64;
        let mean_vel = self.birds.iter().map(|b| b.vel).sum::<Vec3>() / n.max(1) as f32;
        // Personal space: a wingspan or two for big birds in their V, and more for
        // small ones, or a flock of them is a clump with no birds to be seen in it.
        let room = self.span * 2.0 * vpm * if self.large { 1.6 } else { 6.0 };
        for i in 0..n {
            let me = &self.birds[i];
            let mut apart = Vec3::ZERO;
            for (j, other) in self.birds.iter().enumerate() {
                if j == i {
                    continue;
                }
                let d = (me.pos - other.pos).as_vec3();
                let l = d.length();
                if l < room && l > 1e-3 {
                    apart += d / l * (1.0 - l / room);
                }
            }
            let mut want = if self.large {
                // Its place in the V, which breathes a little: a loose V, never a stiff one.
                let s = me.slot;
                let breathe = 1.0 + 0.12 * noise::perlin2(0x6e, me.seed as f64, a * 0.2) as f32;
                let place = self.lead.as_vec3() + side * s.x * breathe - dir * s.z * breathe + Vec3::Y * s.y;
                let to = (place.as_dvec3() - me.pos).as_vec3();
                lead_vel + to * 0.5 + (mean_vel - me.vel) * 0.2
            } else {
                // Cohesion, alignment and separation, and the lead to make for.
                let to_centre = (centre - me.pos).as_vec3();
                let to_lead = (self.lead - me.pos).as_vec3();
                let jitter = Vec3::new(
                    noise::perlin2(0x70, me.seed as f64, a * 0.7) as f32,
                    noise::perlin2(0x71, me.seed as f64, a * 0.7) as f32 * 0.4,
                    noise::perlin2(0x72, me.seed as f64, a * 0.7) as f32,
                ) * 2.0 * vpm;
                mean_vel + to_centre * 0.15 + to_lead * 0.12 + jitter
            };
            want += apart * self.speed * 1.2;
            let me = &mut self.birds[i];
            let pull = if self.large { 1.2 } else { 2.5 };
            me.vel += (want - me.vel) * (1.0 - (-pull * dt).exp());
            // Never stalls, never races.
            let speed = me.vel.length().clamp(self.speed * 0.7, self.speed * 1.35);
            me.vel = me.vel.normalize_or(dir) * speed;
            me.pos += (me.vel * dt).as_dvec3();
            // Leaning into its turns.
            let yaw = (-me.vel.x).atan2(me.vel.z);
            let rate = (yaw - me.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
            me.yaw = yaw;
            let lean = (rate / dt.max(1e-4) * if self.large { 1.2 } else { 0.5 }).clamp(-0.9, 0.9);
            me.bank += (lean - me.bank) * (1.0 - (-4.0 * dt).exp());
            // Beating, or gliding on held wings: a big bird mostly beats, slowly; a small
            // one beats in bursts between glides. Climbing takes beating.
            let glide = noise::perlin2(0x73, me.seed as f64, a * if self.large { 0.12 } else { 0.6 }) as f32;
            let gliding = me.vel.y < 0.3 * vpm && glide > if self.large { 0.25 } else { 0.05 };
            let raised = if gliding {
                0.12
            } else {
                me.flap += me.rate * dt;
                0.12 + if self.large { 0.65 } else { 0.8 } * me.flap.sin()
            };
            me.wing += (raised - me.wing) * (1.0 - (-25.0 * dt).exp());
        }
    }

    /// Body and wings for each bird, three cubes apiece.
    fn draw(&self, camera: DVec3, out: &mut Vec<ParticleInstance>) {
        let vpm = VOXELS_PER_METRE;
        for b in &self.birds {
            let rel = (b.pos - camera).as_vec3();
            let metres = rel.length() / vpm;
            let alpha = 1.0 - smooth(FLOCK_REACH - 60.0, FLOCK_REACH, metres);
            if alpha <= 0.0 {
                continue;
            }
            // Far off a bird is drawn a little bigger than life, so it still reads as a
            // bird with wings rather than a speck.
            let near = if self.large { 70.0 } else { 25.0 };
            let grow = (metres / near).max(1.0).powf(0.85) * vpm;
            let speed = b.vel.length().max(1e-3);
            let pitch = -(b.vel.y / speed).clamp(-0.6, 0.6).asin();
            let (body, span) = (self.body * grow, self.span * grow);
            out.push(ParticleInstance {
                pos: rel.to_array(),
                yaw: b.yaw,
                size: [body * 0.24, body * 0.2, body],
                glow: 0.0,
                color: [self.body_color[0], self.body_color[1], self.body_color[2], alpha],
                tilt: [pitch, b.bank],
            });
            for side in [1.0f32, -1.0] {
                // Each wing hinges at the body, raised by `wing` above the lean.
                let roll = b.bank + side * b.wing;
                let out_dir = Vec3::new(side * roll.cos(), side * roll.sin(), 0.0);
                let centre = out_dir * (span * 0.5) + Vec3::new(0.0, 0.0, body * 0.08);
                let at = rel + turn(centre, b.yaw, [pitch, 0.0]);
                out.push(ParticleInstance {
                    pos: at.to_array(),
                    yaw: b.yaw,
                    size: [span, body * 0.05, body * 0.38],
                    glow: 0.0,
                    color: [self.wing_color[0], self.wing_color[1], self.wing_color[2], alpha],
                    tilt: [pitch, roll],
                });
            }
        }
    }

    fn gone(&self, camera: DVec3) -> bool {
        let off = (self.lead - camera).as_vec3();
        let far = Vec2::new(off.x, off.z).length() / VOXELS_PER_METRE > FLOCK_REACH + 40.0;
        (self.age > 20.0 && far) || self.age > 160.0
    }
}

/// The particle shader's `turn`: roll about z, then pitch about x, then yaw about y.
fn turn(v: Vec3, yaw: f32, tilt: [f32; 2]) -> Vec3 {
    let (sr, cr) = tilt[1].sin_cos();
    let a = Vec3::new(v.x * cr - v.y * sr, v.x * sr + v.y * cr, v.z);
    let (sp, cp) = tilt[0].sin_cos();
    let b = Vec3::new(a.x, a.y * cp - a.z * sp, a.y * sp + a.z * cp);
    let (s, c) = yaw.sin_cos();
    Vec3::new(b.x * c - b.z * s, b.y, b.x * s + b.z * c)
}

#[cfg(test)]
mod leaf_tests {
    use super::LeafShape::*;

    #[test]
    fn shapes_differ_and_needles_drop_fastest() {
        assert!(Needle.size().z < Broad.size().z && Slip.size().z > Broad.size().z);
        // A needle is thin and long.
        assert!(Needle.size().z > 6.0 * Needle.size().x);
        assert!(Needle.fall() > Broad.fall() && Broad.fall() > Petal.fall());
        assert!(Needle.spins() && !Broad.spins());
    }

    /// Flat ground below y = 0.
    struct Ground;

    impl crate::player::Voxels for Ground {
        fn kind(&mut self, _x: i32, y: i32, _z: i32) -> voxel::Kind {
            if y < 0 { voxel::Kind::Solid } else { voxel::Kind::Air }
        }
    }

    fn still_air(camera: glam::DVec3, time: f64) -> super::Surroundings<'static> {
        super::Surroundings {
            camera,
            biome: worldgen::biome::Biome::Desert,
            sun_elevation: -0.5,
            cascades: &[],
            springs: &[],
            surf: &[],
            leaves: &[],
            flowers: &[],
            fires: &[],
            player_entered_water: None,
            leaf_fall: 0.0,
            date: 0.62,
            rain: 0.0,
            snow: 0.0,
            storm: 0.0,
            time,
            land: &|_, _| Some(0.0),
        }
    }

    #[test]
    fn leaves_turn_over_as_they_fall_and_last_until_they_land() {
        use super::*;
        let mut p = Particles::new();
        for (i, shape) in [Broad, Needle, Autumn, Petal].into_iter().enumerate() {
            p.spawn(Particle {
                kind: PKind::Leaf,
                pos: DVec3::new(i as f64 * 10.0, 40.0, 0.0),
                vel: Vec3::ZERO,
                age: 0.0,
                life: f32::INFINITY,
                size: shape.size(),
                color: [0.5, 0.3, 0.1],
                phase: i as f32 * 1.7,
                settled: false,
                rot: Vec3::ZERO,
                spin: Vec4::new(2.4, 0.6, if shape.spins() { 12.0 } else { 6.0 }, shape.fall()),
            });
        }
        let (mut rolls, mut drift) = (vec![(0.0f32, 0.0f32); 4], 0.0f64);
        for step in 0..60 * 120 {
            let falling = p.particles.iter().filter(|q| q.kind == PKind::Leaf && !q.settled).count();
            p.update(1.0 / 60.0, &still_air(DVec3::ZERO, step as f64 / 60.0), &mut Ground);
            let leaves: Vec<_> = p.particles.iter().filter(|q| q.kind == PKind::Leaf).collect();
            // None is lost on the way down (those lying on the ground fade in time).
            assert!(leaves.len() >= falling, "a leaf vanished in the air at step {step}");
            for q in &leaves {
                let i = (q.phase / 1.7).round() as usize;
                if !q.settled {
                    rolls[i] = (rolls[i].0.min(q.rot.z), rolls[i].1.max(q.rot.z));
                    drift = drift.max((q.pos.x - i as f64 * 10.0).abs().max(q.pos.z.abs()));
                }
            }
            if leaves.iter().all(|q| q.settled) {
                // Each tipped well over both ways, and wandered from side to side.
                assert!(rolls.iter().all(|r| r.0 < -0.5 && r.1 > 0.5), "{rolls:?}");
                assert!(drift > 1.0);
                // And lies flat on the ground.
                assert!(leaves.iter().all(|q| q.pos.y > 0.0 && q.pos.y < 0.1 && q.rot.y == 0.0 && q.rot.z == 0.0));
                return;
            }
        }
        panic!("the leaves never came down");
    }

    #[test]
    fn rain_splashes_where_it_lands_and_butterflies_keep_to_flowers_in_bloom() {
        use super::*;
        let mut p = Particles::new();
        let camera = DVec3::new(0.0, 4.0, 0.0);
        for step in 0..60 * 4 {
            let mut s = still_air(camera, step as f64 / 60.0);
            s.rain = 1.0;
            p.update(1.0 / 60.0, &s, &mut Ground);
        }
        let splashes: Vec<_> = p.particles.iter().filter(|q| q.kind == PKind::Splash).collect();
        assert!(splashes.len() > 30, "only {} splashes", splashes.len());
        // They start on the ground and stay near it.
        assert!(splashes.iter().all(|q| q.pos.y > -0.3 && q.pos.y < 2.0), "{:?}", splashes.iter().map(|q| q.pos.y).fold((f64::MAX, f64::MIN), |r, y| (r.0.min(y), r.1.max(y))));
        // Butterflies: over flowers on a dry day in the flowering months, and not otherwise.
        let flowers = [(DVec3::new(3.0, 0.0, 3.0), [0.8, 0.2, 0.2])];
        let count = |date: f32, rain: f32, sun: f32| {
            let mut p = Particles::new();
            for step in 0..60 * 30 {
                let mut s = still_air(camera, step as f64 / 60.0);
                (s.flowers, s.date, s.rain, s.sun_elevation) = (&flowers, date, rain, sun);
                p.update(1.0 / 60.0, &s, &mut Ground);
            }
            p.count(PKind::Butterfly)
        };
        assert!((2..=6).contains(&count(0.25, 0.0, 0.6)));
        assert_eq!((count(0.85, 0.0, 0.6), count(0.25, 0.6, 0.6), count(0.25, 0.0, -0.3)), (0, 0, 0));
        // None the colour of the red flower they rose from.
        let mut p = Particles::new();
        for step in 0..60 * 120 {
            let mut s = still_air(camera, step as f64 / 60.0);
            (s.flowers, s.date, s.sun_elevation) = (&flowers, 0.25, 0.6);
            p.update(1.0 / 60.0, &s, &mut Ground);
            assert!(p.particles.iter().filter(|q| q.kind == PKind::Butterfly).all(|q| (0..3).map(|k| (q.color[k] - flowers[0].1[k]).powi(2)).sum::<f32>() > 0.12));
        }
    }

    #[test]
    fn flocks_go_over_or_round_a_mountain_and_never_into_it() {
        use super::*;
        // A wall of mountain 300 m high, 150 m off, across the way the flock is going.
        let vpm = VOXELS_PER_METRE as f64;
        let land = |x: f64, _z: f64| Some(if x > 150.0 * vpm { 300.0 * vpm } else { 0.0 });
        let mut p = Particles::new();
        for _ in 0..12 {
            p.flocks.clear();
            p.spawn_flock(DVec3::ZERO, &land);
            let mut f = p.flocks.pop().unwrap();
            (f.heading, f.lead) = (0.0, DVec3::new(0.0, 40.0 * vpm, 0.0));
            for step in 0..60 * 60 {
                f.update(1.0 / 60.0, &land);
                let ground = land(f.lead.x, f.lead.z).unwrap();
                assert!(f.lead.y > ground, "into the mountain after {:.1} s, {:.0} m up at {:.0} m along", step as f64 / 60.0, f.lead.y / vpm, f.lead.x / vpm);
            }
        }
    }

    #[test]
    fn flocks_hold_together_and_beat_their_wings() {
        use super::*;
        let mut p = Particles::new();
        let mut kinds = (false, false);
        for _ in 0..12 {
            p.flocks.clear();
            p.spawn_flock(DVec3::ZERO, &|_, _| Some(0.0));
            let mut f = p.flocks.pop().unwrap();
            kinds = (kinds.0 || f.large, kinds.1 || !f.large);
            let mut wings = (f32::MAX, f32::MIN);
            for _ in 0..60 * 40 {
                f.update(1.0 / 60.0, &|_, _| Some(0.0));
                for b in &f.birds {
                    assert!(b.pos.is_finite() && b.vel.is_finite());
                    wings = (wings.0.min(b.wing), wings.1.max(b.wing));
                }
            }
            let n = f.birds.len() as f64;
            let centre = f.birds.iter().map(|b| b.pos).sum::<DVec3>() / n;
            let metres = |d: DVec3| d.length() / VOXELS_PER_METRE as f64;
            // Together after 40 s of wheeling, never piled on one another.
            let spread = f.birds.iter().map(|b| metres(b.pos - centre)).fold(0.0, f64::max);
            assert!(spread < if f.large { 60.0 } else { 40.0 }, "spread {spread} m in a flock of {n}");
            for (i, a) in f.birds.iter().enumerate() {
                for b in &f.birds[i + 1..] {
                    assert!(metres(a.pos - b.pos) > 0.15, "two birds in one place");
                }
            }
            // Small birds keep a few wingspans of air between them.
            if !f.large {
                let mut nearest: Vec<f64> = f.birds.iter().enumerate().map(|(i, a)| {
                    f.birds.iter().enumerate().filter(|&(j, _)| j != i).map(|(_, b)| metres(a.pos - b.pos)).fold(f64::MAX, f64::min)
                }).collect();
                nearest.sort_by(f64::total_cmp);
                let median = nearest[nearest.len() / 2];
                assert!(median > 1.8, "small birds only {median} m apart");
            }
            // Wings went up and down.
            assert!(wings.1 - wings.0 > 0.8, "wings {wings:?}");
        }
        assert!(kinds.0 && kinds.1, "both big birds and small came by");
    }
}
