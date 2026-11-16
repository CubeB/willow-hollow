//! A crow made of boxes, and a nest of sticks.
//!
//! A crow is drawn as the boats and the figure are, as props: opaque boxes lit like the
//! terrain, casting shadows. Its feathers are near black, and a negative `glow` asks the
//! particle shader for the blue-purple gloss a crow's plumage has where the light
//! catches it. Sizes are of a carrion crow, in metres: some 0.46 m beak to tail, a metre
//! across the wings.

use glam::{DVec3, Vec3};
use worldgen::scale::VOXELS_PER_METRE;

use super::sites::Nest;
use crate::particles::ParticleInstance;

/// Near black, a little blue: lit by the sun it shows its shape without looking grey.
const FEATHER: [f32; 3] = [0.016, 0.016, 0.021];
const WING: [f32; 3] = [0.014, 0.014, 0.02];
const BEAK: [f32; 3] = [0.008, 0.008, 0.009];
const LEG: [f32; 3] = [0.012, 0.011, 0.011];
/// How strongly the feathers gloss (see `particles.wgsl`).
const SHEEN: f32 = -1.0;

/// Where the body's middle is above the feet when it stands, in metres.
pub const STANDING: f32 = 0.155;

/// The wings: folded along the back, or open.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Wings {
    Folded,
    Open {
        /// How far the arm is raised above level, radians.
        raise: f32,
        /// How much further the hand is raised than the arm (negative: bent down).
        hand: f32,
        /// How far the hand is swept back, radians.
        sweep: f32,
    },
}

/// What the legs are doing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Legs {
    /// Standing on what is under them, the feet at this height.
    Standing,
    /// Let down ahead of landing.
    Reaching,
    /// Drawn up into the belly feathers in flight.
    Tucked,
}

/// How a crow is held this moment.
#[derive(Clone, Copy, Debug)]
pub struct Pose {
    /// Where its feet are, or would be: the body is [`STANDING`] above it.
    pub feet: DVec3,
    pub yaw: f32,
    /// Nose down is positive.
    pub pitch: f32,
    pub bank: f32,
    pub wings: Wings,
    /// The head turned from the body's heading, and bent down (pecking).
    pub head_turn: f32,
    pub head_dip: f32,
    /// The beak opened, radians.
    pub beak: f32,
    /// The tail spread, 0 to 1, and raised (negative) or lowered.
    pub fan: f32,
    pub tail: f32,
    pub legs: Legs,
}

/// The particle shader's `turn`: roll about z, then pitch about x, then yaw about y.
fn turn(v: Vec3, yaw: f32, pitch: f32, roll: f32) -> Vec3 {
    let (sr, cr) = roll.sin_cos();
    let a = Vec3::new(v.x * cr - v.y * sr, v.x * sr + v.y * cr, v.z);
    let (sp, cp) = pitch.sin_cos();
    let b = Vec3::new(a.x, a.y * cp - a.z * sp, a.y * sp + a.z * cp);
    let (s, c) = yaw.sin_cos();
    Vec3::new(b.x * c - b.z * s, b.y, b.x * s + b.z * c)
}

/// Boxes for one crow in `pose`, seen from `cam`. Fewer, and a little bigger than life,
/// the further off it is, so a crow far off still reads as a crow and not a speck.
pub fn draw(pose: &Pose, cam: DVec3, out: &mut Vec<ParticleInstance>) {
    let metres = (pose.feet.distance(cam) / VOXELS_PER_METRE as f64) as f32;
    let flying = pose.legs != Legs::Standing;
    let grow = if flying { (metres / 45.0).max(1.0).powf(0.6).min(2.6) } else { 1.0 };
    let s = VOXELS_PER_METRE * grow;
    let (near, mid) = (metres < 28.0, metres < 90.0);
    let centre = (pose.feet - cam).as_vec3() + Vec3::Y * STANDING * s;
    let (yaw, pitch, bank) = (pose.yaw, pose.pitch, pose.bank);
    let mut part = |o: Vec3, size: Vec3, turned: [f32; 3], color: [f32; 3], sheen: f32| {
        out.push(ParticleInstance {
            pos: (centre + turn(o * s, yaw, pitch, bank)).to_array(),
            yaw: yaw + turned[0],
            size: (size * s).to_array(),
            glow: sheen,
            color: [color[0], color[1], color[2], 1.0],
            tilt: [pitch + turned[1], bank + turned[2]],
        });
    };

    // The body, deep in the chest, and the neck into the head.
    part(Vec3::ZERO, Vec3::new(0.115, 0.115, 0.24), [0.0; 3], FEATHER, SHEEN);
    if near {
        part(Vec3::new(0.0, -0.042, 0.06), Vec3::new(0.098, 0.06, 0.11), [0.0; 3], FEATHER, SHEEN * 0.6);
        part(Vec3::new(0.0, 0.03, 0.1), Vec3::new(0.072, 0.075, 0.07), [0.0; 3], FEATHER, SHEEN);
    }
    // The head, turned and bent about the top of the neck, and the heavy beak.
    let neck = Vec3::new(0.0, 0.035, 0.1);
    let (ht, hd) = (pose.head_turn, pose.head_dip);
    let about_neck = |v: Vec3| {
        let (sd, cd) = hd.sin_cos();
        let a = Vec3::new(v.x, v.y * cd - v.z * sd, v.y * sd + v.z * cd);
        let (st, ct) = ht.sin_cos();
        neck + Vec3::new(a.x * ct + a.z * st, a.y, -a.x * st + a.z * ct)
    };
    part(about_neck(Vec3::new(0.0, 0.022, 0.055)), Vec3::new(0.084, 0.08, 0.1), [-ht, hd, 0.0], FEATHER, SHEEN);
    if mid {
        part(about_neck(Vec3::new(0.0, 0.016, 0.135)), Vec3::new(0.03, 0.024, 0.07), [-ht, hd + 0.16, 0.0], BEAK, 0.0);
        if near {
            part(about_neck(Vec3::new(0.0, 0.0, 0.125)), Vec3::new(0.024, 0.012, 0.058), [-ht, hd + 0.16 + pose.beak, 0.0], BEAK, 0.0);
        }
    }
    // The tail, hinged at the rump, rounder when it is spread.
    let tail = pose.tail;
    let hinge = Vec3::new(0.0, 0.015, -0.1);
    let along = Vec3::new(0.0, 0.085 * tail.sin(), -0.085 * tail.cos());
    part(hinge + along, Vec3::new(0.085 + 0.075 * pose.fan, 0.016, 0.17), [0.0, tail, 0.0], FEATHER, SHEEN * 0.7);

    match pose.wings {
        Wings::Folded => {
            for side in [1.0f32, -1.0] {
                part(Vec3::new(side * 0.06, 0.014, -0.05), Vec3::new(0.035, 0.085, 0.28), [0.0, -0.06, side * 0.12], WING, SHEEN);
            }
        }
        Wings::Open { raise, hand, sweep } => {
            for side in [1.0f32, -1.0] {
                // The arm out from the shoulder, the hand on from the wrist, and the long
                // primaries spread at the tip like fingers, as a crow's are.
                let shoulder = Vec3::new(side * 0.045, 0.028, 0.025);
                let out_dir = |a: f32| Vec3::new(side * a.cos(), a.sin(), 0.0);
                let arm = 0.2;
                part(shoulder + out_dir(raise) * arm * 0.5 + Vec3::new(0.0, 0.0, -0.025), Vec3::new(arm, 0.026, 0.19), [0.0, 0.0, side * raise], WING, SHEEN);
                let wrist = shoulder + out_dir(raise) * arm;
                let a2 = raise + hand;
                let swept = |v: Vec3, k: f32| Vec3::new(v.x * (k * sweep).cos(), v.y, -v.x.abs() * (k * sweep).sin()) + v.z * Vec3::Z;
                let hand_len = if mid { 0.17 } else { 0.22 };
                let hand_mid = wrist + swept(out_dir(a2) * hand_len * 0.5, 1.0) + Vec3::new(0.0, 0.0, -0.04);
                part(hand_mid, Vec3::new(hand_len, 0.02, 0.15), [-side * sweep, 0.0, side * a2], WING, SHEEN);
                if mid {
                    let tip = wrist + swept(out_dir(a2) * hand_len, 1.0) + Vec3::new(0.0, 0.0, -0.04);
                    let fingers = if near { 4 } else { 2 };
                    for f in 0..fingers {
                        // Fanned from the leading edge back.
                        let k = f as f32 / (fingers - 1).max(1) as f32;
                        let fan = sweep + 0.1 + 0.55 * k;
                        let back = Vec3::new(0.0, 0.0, 0.05 - 0.1 * k);
                        let dir = Vec3::new(side * fan.cos() * a2.cos(), a2.sin(), -fan.sin());
                        part(tip + back + dir * 0.04, Vec3::new(0.085, 0.01, 0.024), [-side * fan, 0.0, side * a2], WING, SHEEN * 0.8);
                    }
                }
            }
        }
    }

    if !mid {
        return;
    }
    match pose.legs {
        Legs::Standing => {
            for side in [1.0f32, -1.0] {
                // From the hip straight down to the foot on the branch or the ground.
                let hip = turn(Vec3::new(side * 0.028, -0.045, 0.0) * s, yaw, pitch, bank);
                let foot = turn(Vec3::new(side * 0.028, 0.0, 0.0) * s, yaw, 0.0, 0.0) - Vec3::Y * STANDING * s;
                let leg = hip - foot;
                let len = leg.length();
                let fwd = Vec3::new(-yaw.sin(), 0.0, yaw.cos());
                let lean = leg.dot(fwd).atan2(leg.y);
                out.push(ParticleInstance {
                    pos: (centre + (hip + foot) * 0.5).to_array(),
                    yaw,
                    size: [0.016 * s, len, 0.016 * s],
                    glow: 0.0,
                    color: [LEG[0], LEG[1], LEG[2], 1.0],
                    tilt: [lean, 0.0],
                });
                if near {
                    out.push(ParticleInstance {
                        pos: (centre + foot + fwd * 0.014 * s + Vec3::Y * 0.005 * s).to_array(),
                        yaw,
                        size: [0.03 * s, 0.01 * s, 0.06 * s],
                        glow: 0.0,
                        color: [LEG[0], LEG[1], LEG[2], 1.0],
                        tilt: [0.0, 0.0],
                    });
                }
            }
        }
        Legs::Reaching => {
            for side in [1.0f32, -1.0] {
                // Let down and forward, ready to take the branch.
                part(Vec3::new(side * 0.028, -0.09, 0.03), Vec3::new(0.016, 0.075, 0.016), [0.0, -0.5, 0.0], LEG, 0.0);
            }
        }
        Legs::Tucked => {}
    }
}

/// The colours of the sticks of a nest: grey-brown, weathered.
const STICKS: [[f32; 3]; 3] = [[0.07, 0.055, 0.04], [0.095, 0.075, 0.055], [0.055, 0.045, 0.035]];

/// Boxes for a nest: a dark cup of twigs, and sticks laid crosswise over and round it,
/// some poking out, as a rook's nest is untidy.
pub fn draw_nest(nest: &Nest, cam: DVec3, out: &mut Vec<ParticleInstance>) {
    let s = VOXELS_PER_METRE;
    let (w, h) = (nest.size, nest.height());
    let base = (nest.pos - cam).as_vec3();
    out.push(ParticleInstance {
        pos: (base + Vec3::Y * h * 0.45 * s).to_array(),
        yaw: nest.yaw,
        size: [w * 0.8 * s, h * 0.8 * s, w * 0.75 * s],
        glow: 0.0,
        color: [0.04, 0.032, 0.025, 1.0],
        tilt: [0.0, 0.0],
    });
    let roll = |k: u32| worldgen::noise::unit(worldgen::noise::hash2(nest.salt, k as i32, 7));
    for k in 0..7u32 {
        let a = nest.yaw + k as f32 * 0.9 + roll(k) * 0.6;
        let up = h * (0.25 + 0.6 * roll(k + 20));
        let off = Vec3::new(a.cos(), 0.0, a.sin()) * w * 0.18 * roll(k + 40);
        let c = STICKS[k as usize % STICKS.len()];
        out.push(ParticleInstance {
            pos: (base + (off + Vec3::Y * up) * s).to_array(),
            yaw: a,
            size: [0.03 * s, 0.03 * s, w * (1.0 + 0.5 * roll(k + 60)) * s],
            glow: 0.0,
            color: [c[0], c[1], c[2], 1.0],
            tilt: [(roll(k + 80) - 0.5) * 0.7, 0.0],
        });
    }
}
