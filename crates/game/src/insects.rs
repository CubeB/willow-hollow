//! The bees and wasps about their nests (worldgen's `features::nests`).
//!
//! A few bees come and go at a colony's door, flying out to the flowers in bloom near
//! it, hanging over one a moment and flying home; a few wasps hang about their globe and
//! now and then dart off a little way and back. Only near the player, by day, in the
//! months they are out: there are never more than a few dozen, so they cost nothing to
//! speak of.

use glam::{DVec3, IVec3, Vec3};
use worldgen::{
    features::nests::{Nest, NestKind},
    noise,
    scale::VOXELS_PER_METRE,
};

use crate::particles::ParticleInstance;

/// How near a nest must be, in metres, for its insects to be about: a bee is a speck
/// further off than this.
pub const NEST_SIGHT: f64 = 32.0;
/// How far from its nest a bee goes to a flower, in metres.
pub const FORAGE: f64 = 18.0;
/// Bees out at a colony at once, and wasps about a nest.
const BEES: usize = 7;
const WASPS: usize = 5;

#[derive(Clone, Copy, PartialEq)]
enum Doing {
    /// Making for `to`.
    Flying,
    /// Hanging over a flower, or at the door, for `timer` seconds more.
    Hovering,
}

struct Insect {
    kind: NestKind,
    /// The nest it belongs to, by its door.
    nest: IVec3,
    pos: DVec3,
    vel: Vec3,
    to: DVec3,
    /// Whether `to` is home.
    homing: bool,
    doing: Doing,
    timer: f32,
    seed: f32,
    yaw: f32,
}

/// Every bee and wasp in the air.
pub struct Insects {
    insects: Vec<Insect>,
    rng: u32,
}

/// Whether bees (or wasps) fly on date `date` (0 at the spring equinox): from the first
/// warm days of spring to the middle of autumn, wasps starting later and lasting a
/// little longer into the autumn than the bees.
pub fn on_the_wing(kind: NestKind, date: f32) -> bool {
    match kind {
        NestKind::Bees => date > 0.02 && date < 0.6,
        NestKind::Wasps => date > 0.1 && date < 0.63,
    }
}

impl Insects {
    pub fn new() -> Self {
        Self { insects: Vec::new(), rng: 0x6a09_e667 }
    }

    fn unit(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        noise::unit(self.rng)
    }

    /// Moves them on by `dt`. `flying` is whether the weather and the light let them out
    /// at all; `flowers` are the flowers in bloom about the player.
    pub fn update(&mut self, dt: f32, camera: DVec3, nests: &[Nest], flowers: &[(DVec3, [f32; 3])], date: f32, flying: bool, time: f64) {
        let vpm = VOXELS_PER_METRE as f64;
        let near = |n: &Nest| (n.mouth() - camera).length() < NEST_SIGHT * vpm;
        // Those whose nest is out of sight go, and all of them when the day or the year
        // closes; one gone home in the dark is not seen to go, being out of sight.
        self.insects.retain(|i| flying && on_the_wing(i.kind, date) && nests.iter().any(|n| n.door == i.nest && near(n)));
        if flying {
            for n in nests.iter().filter(|n| near(n) && on_the_wing(n.kind, date)) {
                let want = if n.kind == NestKind::Bees { BEES } else { WASPS };
                let have = self.insects.iter().filter(|i| i.nest == n.door).count();
                // One at a time, from the door: they come out, not appear in the air.
                if have < want && self.unit() < dt * 2.0 {
                    let seed = self.unit() * 1000.0;
                    let home = n.mouth();
                    self.insects.push(Insect {
                        kind: n.kind,
                        nest: n.door,
                        pos: home,
                        vel: Vec3::ZERO,
                        to: home,
                        homing: true,
                        doing: Doing::Hovering,
                        timer: 0.3,
                        seed,
                        yaw: 0.0,
                    });
                }
            }
        }
        for k in 0..self.insects.len() {
            let Some(nest) = nests.iter().find(|n| n.door == self.insects[k].nest).copied() else { continue };
            let home = nest.mouth();
            if self.insects[k].doing == Doing::Hovering {
                self.insects[k].timer -= dt;
                if self.insects[k].timer <= 0.0 {
                    // Done here: off to the next place.
                    let (homing, to) = if self.insects[k].homing {
                        (false, self.away(&nest, flowers))
                    } else {
                        (true, home)
                    };
                    let i = &mut self.insects[k];
                    i.homing = homing;
                    i.to = to;
                    i.doing = Doing::Flying;
                }
            }
            let roll = self.unit();
            let i = &mut self.insects[k];
            let (speed, wobble): (f32, f32) = match i.kind {
                NestKind::Bees => (3.2, 0.9),
                NestKind::Wasps => (2.4, 1.4),
            };
            let a = time + i.seed as f64;
            let jitter = Vec3::new(
                noise::perlin2(0x1b, a * 2.3, 0.0) as f32,
                noise::perlin2(0x2b, a * 2.3, 4.0) as f32 * 0.6,
                noise::perlin2(0x3b, a * 2.3, 8.0) as f32,
            ) * wobble;
            let off = (i.to - i.pos).as_vec3() / VOXELS_PER_METRE;
            let want = match i.doing {
                // Slowing as it comes in, so it does not overshoot the door or the flower.
                Doing::Flying => off.normalize_or_zero() * speed.min(off.length() * 2.5 + 0.4) + jitter,
                // Holding over the spot with small darts about it.
                Doing::Hovering => off * 3.0 + jitter * 0.8,
            };
            i.vel += (want - i.vel) * (1.0 - (-6.0 * dt).exp());
            i.pos += (i.vel * VOXELS_PER_METRE * dt).as_dvec3();
            if i.doing == Doing::Flying && off.length() < 0.25 {
                i.doing = Doing::Hovering;
                i.timer = if i.homing {
                    // A moment at the door before going in and out again.
                    0.5 + roll * 2.0
                } else {
                    1.5 + roll * 3.5
                };
            }
            if i.vel.x.hypot(i.vel.z) > 0.1 {
                i.yaw = (-i.vel.x).atan2(i.vel.z);
            }
        }
    }

    /// Where an insect leaving `nest` goes: a bee to a flower in bloom near enough, or
    /// over the ground about if there is none; a wasp a short way off, to come back.
    fn away(&mut self, nest: &Nest, flowers: &[(DVec3, [f32; 3])]) -> DVec3 {
        let vpm = VOXELS_PER_METRE as f64;
        let home = nest.mouth();
        if nest.kind == NestKind::Bees {
            let within: Vec<DVec3> = flowers.iter().map(|f| f.0).filter(|&f| (f - home).length() < FORAGE * vpm).collect();
            if !within.is_empty() {
                let f = within[(self.unit() * within.len() as f32) as usize % within.len()];
                // Just over the bloom, which stands in the voxel above the ground.
                return f + DVec3::new(0.5, 0.75, 0.5);
            }
        }
        let reach = if nest.kind == NestKind::Bees { self.unit() as f64 * 8.0 + 4.0 } else { self.unit() as f64 * 3.0 + 0.8 };
        let a = self.unit() as f64 * std::f64::consts::TAU;
        let drop = if nest.kind == NestKind::Bees { -(self.unit() as f64) * 1.5 } else { (self.unit() as f64 - 0.5) * 1.2 };
        home + DVec3::new(a.cos() * reach, drop, a.sin() * reach) * vpm
    }

    /// A body and a blur of wings for each, camera-relative.
    pub fn draw(&self, camera: DVec3, time: f64, out: &mut Vec<ParticleInstance>) {
        let vpm = VOXELS_PER_METRE;
        for i in &self.insects {
            let rel = (i.pos - camera).as_vec3();
            // A little bigger than life further off, or a bee is lost a few steps away.
            let grow = (rel.length() / vpm / 3.0).clamp(1.4, 3.0);
            let (body, colour) = match i.kind {
                NestKind::Bees => ([0.045, 0.04, 0.075], [0.34, 0.2, 0.05]),
                NestKind::Wasps => ([0.04, 0.04, 0.085], [0.72, 0.56, 0.06]),
            };
            out.push(ParticleInstance {
                pos: rel.to_array(),
                yaw: i.yaw,
                size: body.map(|s| s * vpm * grow),
                glow: 0.0,
                color: [colour[0], colour[1], colour[2], 1.0],
                tilt: [0.0, 0.0],
            });
            // The wings beat too fast to see: a pale blur over the back, shimmering.
            let shimmer = 0.25 + 0.15 * ((time * 60.0 + i.seed as f64).sin() as f32);
            out.push(ParticleInstance {
                pos: (rel + Vec3::Y * 0.03 * vpm * grow).to_array(),
                yaw: i.yaw,
                size: [0.11 * vpm * grow, 0.005 * vpm * grow, 0.05 * vpm * grow],
                glow: 0.0,
                color: [0.85, 0.88, 0.9, shimmer],
                tilt: [0.0, 0.0],
            });
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.insects.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use worldgen::features::nests::Host;

    fn nest(kind: NestKind) -> Nest {
        let (host, out) = if kind == NestKind::Bees { (Host::Hollow, IVec3::X) } else { (Host::Branch, IVec3::NEG_Y) };
        Nest { kind, host, door: IVec3::new(10, 20, 10), out, host_at: IVec3::new(8, 15, 10) }
    }

    #[test]
    fn bees_come_and_go_between_their_nest_and_the_flowers() {
        let n = nest(NestKind::Bees);
        let flower = DVec3::new(40.0, 15.0, 12.0);
        let flowers = [(flower, [1.0, 0.0, 0.0])];
        let mut bees = Insects::new();
        let (mut at_flower, mut at_home) = (0, 0);
        for step in 0..6000 {
            let t = step as f64 / 60.0;
            bees.update(1.0 / 60.0, DVec3::new(20.0, 18.0, 20.0), &[n], &flowers, 0.3, true, t);
            for b in &bees.insects {
                if (b.pos - (flower + DVec3::new(0.5, 0.75, 0.5))).length() < 1.5 {
                    at_flower += 1;
                }
                if (b.pos - n.mouth()).length() < 1.5 {
                    at_home += 1;
                }
                // Never far from the way between the two.
                assert!((b.pos - n.mouth()).length() < (FORAGE * VOXELS_PER_METRE as f64) + 10.0);
            }
        }
        assert!(bees.len() > 3 && bees.len() <= BEES, "{} bees", bees.len());
        assert!(at_flower > 500 && at_home > 500, "at the flower {at_flower}, at home {at_home}");
        // None out in the winter, nor when it is too dark or wet to fly.
        bees.update(1.0 / 60.0, DVec3::new(20.0, 18.0, 20.0), &[n], &flowers, 0.3, false, 100.0);
        assert_eq!(bees.len(), 0);
        for step in 0..600 {
            bees.update(1.0 / 60.0, DVec3::new(20.0, 18.0, 20.0), &[n], &flowers, 0.8, true, step as f64 / 60.0);
        }
        assert_eq!(bees.len(), 0);
    }

    #[test]
    fn wasps_keep_about_their_nest_and_go_when_it_is_left_behind() {
        let n = nest(NestKind::Wasps);
        let mut wasps = Insects::new();
        for step in 0..3000 {
            wasps.update(1.0 / 60.0, DVec3::new(20.0, 18.0, 20.0), &[n], &[], 0.4, true, step as f64 / 60.0);
            for w in &wasps.insects {
                assert!((w.pos - n.mouth()).length() < 6.0 * VOXELS_PER_METRE as f64);
            }
        }
        assert!(wasps.len() >= 3);
        let mut out = Vec::new();
        wasps.draw(DVec3::ZERO, 1.0, &mut out);
        assert_eq!(out.len(), wasps.len() * 2);
        // Walked away from, they are gone.
        wasps.update(1.0 / 60.0, DVec3::new(2000.0, 18.0, 20.0), &[n], &[], 0.4, true, 60.0);
        assert_eq!(wasps.len(), 0);
    }
}
