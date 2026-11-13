//! People: the player's own figure, and the template other folk are to be made from.
//!
//! A figure is 18 cells tall, each a little under 0.1 m, the scale the plants and
//! lanterns are built at: legs with knees, a torso, arms with elbows, a neck and a head
//! three cells each way. It is not part of the voxel world, since it moves: each part is
//! a handful of boxes drawn with the boats and the glider, turned about its joint. Who
//! someone is comes from their [`Look`]: skin, hair and clothes, and what they carry.
//!
//! In the figure's own frame x is to its left hand side, y up from the feet, z ahead.

use glam::{DVec3, Vec3};
use worldgen::scale::m;

use crate::{particles::ParticleInstance, player::Mode};

/// Cells from the soles to the crown.
const CELLS: f32 = 18.0;
/// One cell, in voxels.
const CELL: f32 = m(1.76) / CELLS;

/// Colours are sRGB.
type Rgb = [u8; 3];

const LEATHER: Rgb = [0x8d, 0x5c, 0x37];
const EYE: Rgb = [0x1d, 0x18, 0x15];
const ROLL: Rgb = [0xe0, 0xd5, 0xb8];

/// What sets one person apart from another.
#[derive(Clone, Copy)]
pub struct Look {
    pub skin: Rgb,
    pub hair: Rgb,
    pub top: Rgb,
    pub bottom: Rgb,
    pub boots: Rgb,
    /// The scarf.
    pub accent: Rgb,
    /// Carries a pack with a bedroll on it.
    pub pack: bool,
}

/// The player.
pub const WANDERER: Look = Look {
    skin: [0xd9, 0xa3, 0x7a],
    hair: [0x3d, 0x2b, 0x1f],
    top: [0x5f, 0x7d, 0x3a],
    bottom: [0x6b, 0x4a, 0x33],
    boots: [0x3b, 0x2d, 0x25],
    accent: [0xbd, 0x3a, 0x30],
    pack: true,
};

/// How a figure holds itself: each joint's turn about the axis through the hips, in
/// radians, positive swinging the limb back; left then right.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Pose {
    pub leg: [f32; 2],
    /// The knee, from straight.
    pub shin: [f32; 2],
    pub arm: [f32; 2],
    /// The elbow, from straight.
    pub fore: [f32; 2],
    /// The head, turned to its left.
    pub head: f32,
    /// How far the whole figure is raised, in voxels.
    pub lift: f32,
}

impl Pose {
    fn values(&self) -> [f32; 10] {
        [self.leg[0], self.leg[1], self.shin[0], self.shin[1], self.arm[0], self.arm[1], self.fore[0], self.fore[1], self.head, self.lift]
    }

    fn from_values(v: [f32; 10]) -> Self {
        Self { leg: [v[0], v[1]], shin: [v[2], v[3]], arm: [v[4], v[5]], fore: [v[6], v[7]], head: v[8], lift: v[9] }
    }

    /// Part of the way from this pose to `other`.
    fn toward(&self, other: &Pose, t: f32) -> Pose {
        let (a, b) = (self.values(), other.values());
        Pose::from_values(std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t))
    }
}

/// What a figure is doing, as far as its limbs are concerned.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Doing {
    /// On the ground, moving at this many metres a second (or standing, at none).
    Afoot(f32),
    Falling,
    Swimming,
    Gliding,
    Sitting,
}

impl Doing {
    /// What the player is doing.
    pub fn of(mode: Mode, on_ground: bool, swimming: bool, speed: f32) -> Self {
        match mode {
            Mode::Glide => Doing::Gliding,
            Mode::Boat => Doing::Sitting,
            Mode::Walk if swimming => Doing::Swimming,
            Mode::Walk if on_ground => Doing::Afoot(speed),
            Mode::Walk => Doing::Falling,
        }
    }
}

/// A figure's movement from moment to moment: where it is in its stride, and the pose
/// it has come to, which follows what it is doing without snapping.
pub struct Gait {
    phase: f32,
    clock: f32,
    pose: Pose,
    /// Which way the body faces, as a camera's yaw.
    pub facing: f32,
}

/// Metres a second at which the stride is full.
const FULL_STRIDE: f32 = 3.0;

impl Gait {
    pub fn new(facing: f32) -> Self {
        Self { phase: 0.0, clock: 0.0, pose: Pose::default(), facing }
    }

    pub fn pose(&self) -> Pose {
        self.pose
    }

    /// The pose `doing` asks for at this moment of the stride.
    fn wanted(&self, doing: Doing) -> Pose {
        let (s, c) = self.phase.sin_cos();
        let breath = (self.clock * 1.7).sin();
        match doing {
            Doing::Afoot(speed) if speed > 0.3 => {
                let stride = (speed / FULL_STRIDE).min(1.0);
                let a = 0.52 * stride.sqrt();
                Pose {
                    leg: [-s * a, s * a],
                    shin: [c.max(0.0) * a * 1.7, (-c).max(0.0) * a * 1.7],
                    arm: [s * a * 0.85, -s * a * 0.85],
                    fore: [-(0.12 + 0.45 * (-s).max(0.0) * stride), -(0.12 + 0.45 * s.max(0.0) * stride)],
                    head: 0.0,
                    lift: (c.abs() - 1.0) * m(0.03) * stride,
                }
            }
            Doing::Afoot(_) => Pose {
                arm: [breath * 0.04, -breath * 0.04],
                fore: [-0.08; 2],
                head: (self.clock * 0.55).sin() * 0.25,
                lift: breath * m(0.006),
                ..Default::default()
            },
            Doing::Falling => Pose { leg: [-0.35, 0.2], shin: [0.5, 0.35], arm: [-0.7, -0.5], fore: [-0.5; 2], ..Default::default() },
            Doing::Swimming => {
                let k = (self.clock * 3.0).sin();
                Pose { leg: [k * 0.35, -k * 0.35], shin: [0.3; 2], arm: [-1.2 + k * 0.5, -1.2 - k * 0.5], fore: [-0.4; 2], ..Default::default() }
            }
            // Hanging from the wing's frame, legs trailing.
            Doing::Gliding => Pose { leg: [0.45, 0.6], shin: [0.3, 0.2], arm: [-2.9; 2], fore: [-0.1; 2], ..Default::default() },
            Doing::Sitting => Pose { leg: [-1.45; 2], shin: [1.45; 2], arm: [-0.5; 2], fore: [-0.7; 2], ..Default::default() },
        }
    }

    /// Moves the figure on by `dt` seconds of `doing`, turning it to face `heading` (a
    /// camera's yaw) if it has one to turn to.
    pub fn update(&mut self, doing: Doing, heading: Option<f32>, dt: f32) {
        self.clock += dt;
        if let Doing::Afoot(speed) = doing {
            // A step a little under a metre: quicker feet, not longer legs, when running.
            self.phase = (self.phase + dt * (2.2 + speed * 0.9).min(9.0) * if speed > 0.3 { 1.0 } else { 0.0 }).rem_euclid(std::f32::consts::TAU);
        }
        let ease = |rate: f32| if dt > 0.0 { 1.0 - (-dt * rate).exp() } else { 1.0 };
        self.pose = self.pose.toward(&self.wanted(doing), ease(14.0));
        if let Some(heading) = heading {
            let pi = std::f32::consts::PI;
            let turn = (heading - self.facing + pi).rem_euclid(2.0 * pi) - pi;
            self.facing += turn * ease(10.0);
        }
    }

    /// A figure caught in mid stride, for a still.
    pub fn striding(facing: f32) -> Self {
        let mut g = Self { phase: 0.9, clock: 0.0, pose: Pose::default(), facing };
        g.pose = g.wanted(Doing::Afoot(FULL_STRIDE));
        g
    }
}

/// A box of a part: its low corner and size in cells, from the part's joint.
type Piece = ([f32; 3], [f32; 3], Rgb);

fn shade(c: Rgb, by: f32) -> Rgb {
    c.map(|v| (v as f32 * by).round().clamp(0.0, 255.0) as u8)
}

fn blend(a: Rgb, b: Rgb, t: f32) -> Rgb {
    [0, 1, 2].map(|k| (a[k] as f32 + (b[k] as f32 - a[k] as f32) * t).round() as u8)
}

fn linear(c: Rgb) -> [f32; 4] {
    let l = |v: u8| (v as f32 / 255.0).powf(2.2);
    [l(c[0]), l(c[1]), l(c[2]), 1.0]
}

/// Draws a figure standing with its feet at `feet`, for a camera at `cam`.
pub fn draw(look: &Look, feet: DVec3, facing: f32, pose: &Pose, cam: DVec3, out: &mut Vec<ParticleInstance>) {
    // The figure looks along its own z; a camera at yaw 0 looks along x.
    let yaw = facing - std::f32::consts::FRAC_PI_2;
    let base = (feet - cam).as_vec3() + Vec3::Y * pose.lift;
    // Boxes of a part hung from `joint` (cells, in the figure's frame), the part turned
    // by `pitch` about the hips' axis and by `turn` about the upright.
    let mut part = |joint: Vec3, pitch: f32, turn: f32, pieces: &[Piece]| {
        let (sp, cp) = pitch.sin_cos();
        let (sy, cy) = (yaw + turn).sin_cos();
        for (lo, size, color) in pieces {
            let (lo, size) = (Vec3::from(*lo), Vec3::from(*size));
            let mid = lo + size * 0.5;
            let tipped = Vec3::new(mid.x, mid.y * cp - mid.z * sp, mid.y * sp + mid.z * cp);
            // The joint turns with the body; the part about its joint with its own turn.
            let (by, bx) = yaw.sin_cos();
            let at = Vec3::new(joint.x * bx - joint.z * by, joint.y, joint.x * by + joint.z * bx)
                + Vec3::new(tipped.x * cy - tipped.z * sy, tipped.y, tipped.x * sy + tipped.z * cy);
            out.push(ParticleInstance {
                pos: (base + at * CELL).into(),
                yaw: yaw + turn,
                size: (size * CELL).into(),
                glow: 0.0,
                color: linear(*color),
                tilt: [pitch, 0.0],
            });
        }
    };
    // Where a joint `down` cells along a limb hung from `joint` at `pitch` has got to.
    let along = |joint: Vec3, pitch: f32, down: f32| joint + Vec3::new(0.0, -down * pitch.cos(), -down * pitch.sin());

    for (k, side) in [-1.0f32, 1.0].into_iter().enumerate() {
        // Thigh, then shin and boot from the knee.
        let hip = Vec3::new(side, 9.0, 0.0);
        part(hip, pose.leg[k], 0.0, &[([-1.0, -4.0, -1.0], [2.0, 4.0, 2.0], look.bottom)]);
        let knee = along(hip, pose.leg[k], 4.0);
        part(
            knee,
            pose.leg[k] + pose.shin[k],
            0.0,
            &[
                ([-1.0, -3.0, -1.0], [2.0, 3.0, 2.0], shade(look.bottom, 0.94)),
                ([-1.0, -5.0, -1.0], [2.0, 2.0, 2.0], look.boots),
                ([-1.0, -5.0, 1.0], [2.0, 1.0, 1.0], look.boots),
            ],
        );
        // Upper arm, then forearm and hand from the elbow.
        let shoulder = Vec3::new(side * 2.5, 13.5, 0.0);
        part(shoulder, pose.arm[k], 0.0, &[([-0.5, -2.5, -1.0], [1.0, 3.0, 2.0], look.top)]);
        let elbow = along(shoulder, pose.arm[k], 2.5);
        part(
            elbow,
            pose.arm[k] + pose.fore[k],
            0.0,
            &[([-0.5, -2.0, -1.0], [1.0, 2.0, 2.0], shade(look.top, 0.94)), ([-0.5, -3.0, -1.0], [1.0, 1.0, 2.0], look.skin)],
        );
    }

    let strap = shade(LEATHER, 0.85);
    let mut body: Vec<Piece> = vec![
        ([-2.0, 0.0, -1.0], [4.0, 1.0, 2.0], LEATHER),
        ([-2.0, 1.0, -1.0], [4.0, 4.0, 2.0], look.top),
        // The scarf, wound about the neck, an end hanging in front.
        ([-2.0, 5.0, -2.0], [4.0, 1.0, 4.0], look.accent),
        ([-2.0, 3.0, 1.0], [1.0, 2.0, 1.0], shade(look.accent, 0.92)),
    ];
    if look.pack {
        body.extend([
            ([-1.5, 1.4, -2.6], [3.0, 3.0, 1.6], LEATHER),
            ([-1.5, 3.8, -2.75], [3.0, 0.7, 1.75], shade(LEATHER, 1.18)),
            ([-2.0, 4.5, -2.3], [4.0, 0.9, 1.0], ROLL),
            ([-1.6, 1.0, 0.98], [0.5, 4.0, 0.1], strap),
            ([1.1, 1.0, 0.98], [0.5, 4.0, 0.1], strap),
        ]);
    }
    part(Vec3::new(0.0, 9.0, 0.0), 0.0, 0.0, &body);

    let eye = blend(look.skin, EYE, 0.7);
    part(
        Vec3::new(0.0, 15.0, 0.0),
        0.0,
        pose.head,
        &[
            ([-1.5, 0.0, -1.5], [3.0, 1.0, 3.0], look.skin),
            ([-1.5, 1.0, -1.5], [3.0, 1.0, 1.0], look.hair),
            ([-1.5, 1.0, -0.5], [3.0, 1.0, 1.0], look.skin),
            ([-1.5, 1.0, 0.5], [1.0, 1.0, 1.0], eye),
            ([-0.5, 1.0, 0.5], [1.0, 1.0, 1.0], look.skin),
            ([0.5, 1.0, 0.5], [1.0, 1.0, 1.0], eye),
            ([-1.5, 2.0, -1.5], [3.0, 1.0, 3.0], look.hair),
        ],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxes(pose: &Pose, facing: f32) -> Vec<ParticleInstance> {
        let mut out = Vec::new();
        draw(&WANDERER, DVec3::ZERO, facing, pose, DVec3::ZERO, &mut out);
        out
    }

    #[test]
    fn a_figure_standing_is_as_tall_as_the_player_and_stands_on_its_feet() {
        let out = boxes(&Pose::default(), 0.3);
        let low = out.iter().map(|b| b.pos[1] - b.size[1] * 0.5).fold(f32::MAX, f32::min);
        let high = out.iter().map(|b| b.pos[1] + b.size[1] * 0.5).fold(f32::MIN, f32::max);
        assert!(low.abs() < 1e-4, "feet at {low}");
        assert!((high - crate::player::HEIGHT as f32).abs() < 1e-3, "crown at {high}");
        // Narrow enough to pass wherever the player's body does, pack aside.
        assert!(out.iter().all(|b| b.pos[0].hypot(b.pos[2]) < m(0.45)));
    }

    #[test]
    fn it_faces_the_way_a_camera_at_that_yaw_looks() {
        for facing in [0.0, 1.0, 2.5, -1.2] {
            let out = boxes(&Pose::default(), facing);
            // The toes are ahead of the heels.
            let toes: Vec<_> = out.iter().filter(|b| (b.size[1] - CELL).abs() < 1e-4 && (b.size[2] - CELL).abs() < 1e-4 && b.pos[1] < CELL).collect();
            assert_eq!(toes.len(), 2);
            let ahead = Vec3::new(facing.cos(), 0.0, facing.sin());
            for t in toes {
                assert!(Vec3::from(t.pos).dot(ahead) > CELL, "{facing}: toe at {:?}", t.pos);
            }
        }
    }

    #[test]
    fn a_stride_swings_the_legs_apart_and_the_arms_against_them() {
        let g = Gait::striding(0.0);
        let p = g.pose();
        assert!(p.leg[0] * p.leg[1] < 0.0 && p.arm[0] * p.arm[1] < 0.0);
        assert!(p.leg[0] * p.arm[0] < 0.0, "an arm swings with the other leg");
        assert!(p.shin.iter().all(|&k| k >= 0.0), "knees bend one way only");
        // Standing still, it comes to rest upright.
        let mut g = g;
        for _ in 0..200 {
            g.update(Doing::Afoot(0.0), None, 1.0 / 60.0);
        }
        assert!(g.pose().leg.iter().all(|l| l.abs() < 1e-3));
    }

    #[test]
    fn a_figure_turns_the_short_way_round_to_its_heading() {
        let mut g = Gait::new(3.0);
        for _ in 0..120 {
            g.update(Doing::Afoot(2.0), Some(-3.0), 1.0 / 60.0);
        }
        let pi = std::f32::consts::PI;
        let off = (g.facing + 3.0 + pi).rem_euclid(2.0 * pi) - pi;
        assert!(off.abs() < 0.01, "{}", g.facing);
        assert!(g.facing > 3.0, "went the long way: {}", g.facing);
    }
}
