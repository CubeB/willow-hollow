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
/// The glider furled: its pole, its cloth, and the tie about it (as vehicles.rs has them).
const POLE: Rgb = [123, 108, 94];
const FURL: Rgb = [220, 147, 133];
const TIE: Rgb = [237, 222, 199];

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
    hair: [0x9a, 0x5a, 0x2c],
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
    /// How far the body leans forward from the hips.
    pub lean: f32,
    /// What is carried, shaken by the going: how far the pack rides up, in voxels, and
    /// how far the furled glider swings from where it is strapped, in radians.
    pub jiggle: [f32; 2],
}

impl Pose {
    fn values(&self) -> [f32; 11] {
        [self.leg[0], self.leg[1], self.shin[0], self.shin[1], self.arm[0], self.arm[1], self.fore[0], self.fore[1], self.head, self.lift, self.lean]
    }

    fn from_values(v: [f32; 11]) -> Self {
        Self { leg: [v[0], v[1]], shin: [v[2], v[3]], arm: [v[4], v[5]], fore: [v[6], v[7]], head: v[8], lift: v[9], lean: v[10], jiggle: [0.0; 2] }
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
    /// The pack and the furled glider, each hung on a spring: how far it is from where
    /// it rests and how fast it is going, in metres and metres a second.
    carried: [(f32, f32); 2],
    /// How fast the body was rising a moment ago, in metres a second, and its lift then.
    rising: f32,
    lifted: f32,
}

/// The springs the pack and the furled glider hang on: how fast each swings (radians a
/// second), how soon it settles (1 would be at once), and how much of a jolt it takes.
const SPRINGS: [(f32, f32, f32); 2] = [(30.0, 0.32, 0.22), (21.0, 0.22, 0.3)];
/// The furthest either is thrown, in metres.
const THROW: f32 = 0.05;
/// The arm that reaches for the furled glider on the back: at the shoulder and the
/// elbow, as the rod leaves the back.
const STOW: (f32, f32) = (-3.5, -1.3);

/// Metres a second below which a figure is only shuffling, and at which it is running
/// flat out.
const SHUFFLE: f32 = 1.0;
const FLAT_OUT: f32 = 6.9;
/// The arms' turn at the shoulder that puts the hands on the glider's bar (see
/// [`GRIP`]), elbows straight.
const REACH: f32 = -2.63;

/// Where the hands hold the glider, from the feet: ahead and up, in voxels. The arms are
/// five cells from shoulder to the middle of the hand.
pub fn grip() -> (f32, f32) {
    let arm = 5.0 * CELL;
    (-arm * REACH.sin(), 13.5 * CELL - arm * REACH.cos())
}

impl Gait {
    pub fn new(facing: f32) -> Self {
        Self { phase: 0.0, clock: 0.0, pose: Pose::default(), facing, carried: [(0.0, 0.0); 2], rising: 0.0, lifted: 0.0 }
    }

    pub fn pose(&self) -> Pose {
        // The pack rides up and down; the rod, hung by its top, swings like a pendulum.
        let jiggle = [m(self.carried[0].0), self.carried[1].0 * 5.0];
        Pose { jiggle, ..self.pose }
    }

    /// The pose for `doing` while the glider is being taken from the back or put away:
    /// `swing` of the way from strapped there to overhead. The arm on its side reaches
    /// over the shoulder for it and goes up with it, and the head turns that way.
    fn stowing(&self, doing: Doing, swing: f32) -> Pose {
        // Until the wing is out there is no bar to hang from.
        let hanging = doing == Doing::Gliding;
        let mut p = self.wanted(if hanging { Doing::Falling } else { doing });
        let s = swing.clamp(0.0, 1.0);
        p.arm[1] = STOW.0 + (REACH - STOW.0) * s;
        p.fore[1] = STOW.1 * (1.0 - s);
        p.head = 0.5 * (1.0 - s);
        if hanging {
            p.arm[0] += (REACH - p.arm[0]) * s * s;
            p.fore[0] *= 1.0 - s * s;
        }
        p
    }

    /// The pose `doing` asks for at this moment of the stride.
    fn wanted(&self, doing: Doing) -> Pose {
        let (s, c) = self.phase.sin_cos();
        let breath = (self.clock * 1.7).sin();
        match doing {
            Doing::Afoot(speed) if speed > 0.3 => {
                // From a shuffle through a jog to a run: the legs reach further, the back
                // heel kicks up higher, the elbows bend and pump, the body leans into it.
                let run = ((speed - SHUFFLE) / (FLAT_OUT - SHUFFLE)).clamp(0.0, 1.0);
                let a = (0.45 + 0.4 * run) * (speed / SHUFFLE).min(1.0).sqrt();
                let kick = 1.7 + 0.9 * run;
                let elbow = 0.2 + 1.15 * run;
                Pose {
                    leg: [-s * a, s * a],
                    shin: [c.max(0.0) * a * kick, (-c).max(0.0) * a * kick],
                    arm: [s * a * 0.9, -s * a * 0.9],
                    fore: [-(elbow + 0.3 * (-s).max(0.0)), -(elbow + 0.3 * s.max(0.0))],
                    head: 0.0,
                    // Highest with the legs at full reach, both feet off the ground; lowest
                    // as one leg passes the other.
                    lift: (s.abs() - 0.6) * m(0.05) * (0.4 + run),
                    lean: 0.04 + 0.22 * run,
                    jiggle: [0.0; 2],
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
            // Hanging from the wing's bar by straight arms, legs trailing.
            Doing::Gliding => Pose { leg: [0.45, 0.6], shin: [0.3, 0.2], arm: [REACH; 2], ..Default::default() },
            Doing::Sitting => Pose { leg: [-1.45; 2], shin: [1.45; 2], arm: [-0.5; 2], fore: [-0.7; 2], ..Default::default() },
        }
    }

    /// Moves the figure on by `dt` seconds of `doing`. `stride` is how far it is through
    /// its steps, one to each whole number, a foot coming down as each is reached: the
    /// same count the footsteps are sounded from. It turns to face `heading` (a camera's
    /// yaw) if it has one to turn to, at once if it is `held` there.
    ///
    /// `stow` is how far the furled glider has swung up from the back, while it is on
    /// its way there or back, and `rise` how fast the figure is rising, in metres a
    /// second, which with the stride's own rise and fall shakes what it carries.
    pub fn update(&mut self, doing: Doing, stride: f32, heading: Option<f32>, held: bool, stow: Option<f32>, rise: f32, dt: f32) {
        self.clock += dt;
        // The legs are at full reach, the leading foot landing, as a step is counted.
        self.phase = (std::f32::consts::PI * (stride + 0.5)).rem_euclid(std::f32::consts::TAU);
        let ease = |rate: f32| if dt > 0.0 { 1.0 - (-dt * rate).exp() } else { 1.0 };
        // The stride itself is followed closely, or it would lag the feet's sound; a
        // change of what the figure is doing comes in a little slower.
        let rate = if matches!(doing, Doing::Afoot(v) if v > 0.3) { 30.0 } else { 12.0 };
        let wanted = match stow {
            Some(swing) => self.stowing(doing, swing),
            None => self.wanted(doing),
        };
        self.pose = self.pose.toward(&wanted, ease(if stow.is_some() { 22.0 } else { rate }));
        // What is carried lags the body: each change in how fast the body rises, from a
        // jump, a landing or the stride, is a jolt to it the other way.
        if dt > 0.0 {
            let lift = self.pose.lift / m(1.0);
            let rising = rise + (lift - self.lifted) / dt;
            let jolt = (rising - self.rising).clamp(-9.0, 9.0);
            (self.rising, self.lifted) = (rising, lift);
            let steps = (dt * 240.0).ceil().max(1.0) as u32;
            let h = dt / steps as f32;
            for (k, (swing, settle, give)) in SPRINGS.into_iter().enumerate() {
                let (mut at, mut speed) = self.carried[k];
                speed -= jolt * give;
                for _ in 0..steps {
                    speed += (-swing * swing * at - 2.0 * settle * swing * speed) * h;
                    at += speed * h;
                }
                self.carried[k] = (at.clamp(-THROW, THROW), speed);
            }
        }
        if let Some(heading) = heading {
            let pi = std::f32::consts::PI;
            let turn = (heading - self.facing + pi).rem_euclid(2.0 * pi) - pi;
            self.facing += turn * if held { 1.0 } else { ease(10.0) };
        }
    }

    /// A figure caught in mid stride at `speed` metres a second, for a still.
    pub fn striding(facing: f32, speed: f32) -> Self {
        let mut g = Self { phase: 0.9, ..Self::new(facing) };
        g.pose = g.wanted(Doing::Afoot(speed));
        g
    }

    /// A figure standing with its furled glider `swing` of the way up from its back, for
    /// a still.
    pub fn reaching(facing: f32, swing: f32) -> Self {
        let mut g = Self::new(facing);
        g.pose = g.stowing(Doing::Afoot(0.0), swing);
        g
    }

    /// A figure hanging from its glider, for a still.
    pub fn gliding(facing: f32) -> Self {
        let mut g = Self::new(facing);
        g.pose = g.wanted(Doing::Gliding);
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

/// Draws a figure standing with its feet at `feet`, for a camera at `cam`. `furled` is
/// the glider carried as a rod: 0 strapped upright to the back beside the pack, up to 1
/// swung overhead to where the wing opens from; None once the wing is out.
pub fn draw(look: &Look, feet: DVec3, facing: f32, pose: &Pose, furled: Option<f32>, cam: DVec3, out: &mut Vec<ParticleInstance>) {
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
    // Where a point of the upper body has got to with the body leaning from the hips.
    let hips = Vec3::new(0.0, 9.0, 0.0);
    let leant = |p: Vec3| {
        let (o, (sl, cl)) = (p - hips, pose.lean.sin_cos());
        hips + Vec3::new(o.x, o.y * cl - o.z * sl, o.y * sl + o.z * cl)
    };

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
        let shoulder = leant(Vec3::new(side * 2.5, 13.5, 0.0));
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
    let mut pack: Vec<Piece> = Vec::new();
    if look.pack {
        // No two of these, nor any and the body or scarf, share a face: the bag stands
        // clear below the scarf, its flap overhangs it, the bedroll rides behind the
        // scarf's knot, and the straps stop short of the scarf and clear of its end.
        pack.extend([
            // Set to one side, leaving room beside it for the furled glider.
            ([-1.75, 1.2, -2.5], [2.75, 2.7, 1.45], LEATHER),
            ([-1.9, 3.9, -2.7], [3.0, 0.6, 1.65], shade(LEATHER, 1.18)),
            ([-1.98, 4.5, -2.95], [3.1, 0.8, 0.85], ROLL),
        ]);
        body.extend([([-0.95, 1.05, 1.0], [0.45, 3.9, 0.08], strap), ([1.2, 1.05, 1.0], [0.45, 3.9, 0.08], strap)]);
    }
    part(hips, pose.lean, 0.0, &body);
    // The pack, riding a little loose on the back.
    part(hips + Vec3::Y * (pose.jiggle[0] / CELL), pose.lean, 0.0, &pack);

    if let Some(swing) = furled {
        // From the back, by the shoulder, up and over to lie along the wing's middle.
        let t = swing.clamp(0.0, 1.0);
        let e = t * t * (3.0 - 2.0 * t);
        // Behind the scarf, against the side of the pack.
        let on_back = leant(Vec3::new(1.58, 14.0, -2.45));
        let (ahead, up) = grip();
        let overhead = Vec3::new(0.0, up / CELL + 5.6, ahead / CELL + 2.0);
        // Strapped by its top, it swings a little as the figure goes; in the hand it does not.
        let pitch = pose.lean + pose.jiggle[1] * (1.0 - e) + (std::f32::consts::FRAC_PI_2 - pose.lean) * e;
        part(
            on_back.lerp(overhead, e),
            pitch,
            0.0,
            &[
                ([-0.22, -7.0, -0.22], [0.44, 12.0, 0.44], POLE),
                ([-0.4, -6.4, -0.4], [0.8, 9.8, 0.8], FURL),
                ([-0.42, -4.6, -0.42], [0.84, 0.6, 0.84], TIE),
                ([-0.42, 0.6, -0.42], [0.84, 0.6, 0.84], TIE),
            ],
        );
    }

    let eye = blend(look.skin, EYE, 0.7);
    part(
        leant(Vec3::new(0.0, 15.0, 0.0)),
        pose.lean * 0.4,
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

/// Brass, and the glass of a lantern by day, when it is not lit.
const BRASS: Rgb = [150, 112, 58];
const GLASS: Rgb = [214, 200, 160];

/// Draws a lantern hung from the right hand of a figure drawn by `draw` with the same
/// `feet`, `facing` and `pose`, burning `lit` brightly (0 shuttered). Where its flame is,
/// in the world, for the light it throws.
pub fn draw_lantern(feet: DVec3, facing: f32, pose: &Pose, lit: f32, cam: DVec3, out: &mut Vec<ParticleInstance>) -> DVec3 {
    let yaw = facing - std::f32::consts::FRAC_PI_2;
    let (sy, cy) = yaw.sin_cos();
    // The hand, as `draw` puts it: down the arm from the shoulder, and on down the
    // forearm from the elbow, with the body leaning from the hips.
    let hips = Vec3::new(0.0, 9.0, 0.0);
    let (sl, cl) = pose.lean.sin_cos();
    let shoulder = Vec3::new(-2.5, 13.5, 0.0) - hips;
    let shoulder = hips + Vec3::new(shoulder.x, shoulder.y * cl - shoulder.z * sl, shoulder.y * sl + shoulder.z * cl);
    let along = |joint: Vec3, pitch: f32, down: f32| joint + Vec3::new(0.0, -down * pitch.cos(), -down * pitch.sin());
    let elbow = along(shoulder, pose.arm[0], 2.5);
    // Held a little out from the side, so it swings clear of the leg.
    let hand = along(elbow, pose.arm[0] + pose.fore[0], 3.0) + Vec3::new(-0.7, 0.0, 0.0);
    let base = (feet - cam).as_vec3() + Vec3::Y * pose.lift;
    let world = |p: Vec3| base + Vec3::new(p.x * cy - p.z * sy, p.y, p.x * sy + p.z * cy) * CELL;
    // It hangs from its handle straight down, whatever the arm is doing.
    let pieces: [([f32; 3], [f32; 3], Rgb, f32); 5] = [
        ([0.0, -0.4, 0.0], [0.25, 0.8, 0.25], BRASS, 0.0),
        ([0.0, -1.0, 0.0], [1.5, 0.35, 1.5], BRASS, 0.0),
        ([0.0, -2.0, 0.0], [1.1, 1.7, 1.1], if lit > 0.0 { [255, 196, 110] } else { GLASS }, 4.0 * lit),
        ([0.0, -2.0, 0.0], [1.3, 1.6, 0.18], BRASS, 0.0),
        ([0.0, -3.0, 0.0], [1.6, 0.35, 1.6], BRASS, 0.0),
    ];
    for (at, size, color, glow) in pieces {
        out.push(ParticleInstance {
            pos: world(hand + Vec3::from(at)).into(),
            yaw,
            size: (Vec3::from(size) * CELL).into(),
            glow,
            color: linear(color),
            tilt: [0.0; 2],
        });
    }
    cam + world(hand + Vec3::new(0.0, -2.0, 0.0)).as_dvec3()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxes(pose: &Pose, facing: f32) -> Vec<ParticleInstance> {
        let mut out = Vec::new();
        draw(&WANDERER, DVec3::ZERO, facing, pose, None, DVec3::ZERO, &mut out);
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
    fn the_furled_glider_rides_on_the_back_clear_of_the_pack_and_the_arms() {
        let mut out = Vec::new();
        draw(&WANDERER, DVec3::ZERO, std::f32::consts::FRAC_PI_2, &Pose::default(), Some(0.0), DVec3::ZERO, &mut out);
        // Facing along z, the figure's own frame is the world's: x aside, z ahead.
        let furl = linear(FURL);
        let rod: Vec<_> = out.iter().filter(|b| b.color == furl).collect();
        assert_eq!(rod.len(), 1);
        let (lo, hi) = (|k: usize| rod[0].pos[k] - rod[0].size[k] * 0.5, |k: usize| rod[0].pos[k] + rod[0].size[k] * 0.5);
        assert!(hi(2) < -CELL, "behind the back: {}", hi(2) / CELL);
        assert!(hi(0).abs().max(lo(0).abs()) <= 2.0 * CELL + 1e-4, "inside the arms");
        assert!(rod[0].size[1] > 6.0 * CELL, "upright");
        // No box of the pack reaches into it.
        let leather = [linear(LEATHER), linear(shade(LEATHER, 1.18)), linear(ROLL)];
        for b in out.iter().filter(|b| leather.contains(&b.color) && b.pos[2] < -CELL) {
            let apart = (0..3).any(|k| (b.pos[k] - rod[0].pos[k]).abs() >= (b.size[k] + rod[0].size[k]) * 0.5 + 1e-4);
            assert!(apart, "pack box at {:?} meets the rod", b.pos);
        }
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
        let g = Gait::striding(0.0, 4.0);
        let p = g.pose();
        assert!(p.leg[0] * p.leg[1] < 0.0 && p.arm[0] * p.arm[1] < 0.0);
        assert!(p.leg[0] * p.arm[0] < 0.0, "an arm swings with the other leg");
        assert!(p.shin.iter().all(|&k| k >= 0.0), "knees bend one way only");
        // Standing still, it comes to rest upright.
        let mut g = g;
        for _ in 0..200 {
            g.update(Doing::Afoot(0.0), 0.0, None, false, None, 0.0, 1.0 / 60.0);
        }
        assert!(g.pose().leg.iter().all(|l| l.abs() < 1e-3));
    }

    #[test]
    fn a_foot_comes_down_as_each_step_is_counted_and_the_legs_change_over_between() {
        let pose_at = |stride: f32| {
            let mut g = Gait::new(0.0);
            g.update(Doing::Afoot(4.0), stride, None, false, None, 0.0, 0.0);
            g.pose()
        };
        // At each count the legs are at full reach, the other leg leading from the last.
        let (first, second) = (pose_at(1.0), pose_at(2.0));
        assert!(first.leg[0] * second.leg[0] < 0.0 && (first.leg[0] + second.leg[0]).abs() < 1e-4);
        let reach = first.leg[0].abs();
        for k in 1..10 {
            assert!(pose_at(1.0 + k as f32 / 10.0).leg[0].abs() < reach + 1e-4);
        }
        // Half way between, one leg is passing the other.
        assert!(pose_at(1.5).leg[0].abs() < 1e-4);
        // Running reaches further and leans more than jogging.
        let pose_of = |speed: f32| Gait::striding(0.0, speed).pose();
        assert!(pose_of(6.9).leg[0].abs() > pose_of(4.0).leg[0].abs() && pose_of(6.9).lean > pose_of(4.0).lean);
    }

    #[test]
    fn the_hands_of_a_figure_gliding_are_at_the_grip() {
        let g = Gait::gliding(0.0);
        let out = boxes(&g.pose(), 0.0);
        let (ahead, up) = grip();
        // The hands: the two boxes of skin a cell tall and wide, two deep.
        let skin = linear(WANDERER.skin);
        let hands: Vec<_> = out.iter().filter(|b| b.color == skin && (b.size[0] - CELL).abs() < 1e-4 && (b.size[2] - 2.0 * CELL).abs() < 1e-4 && (b.size[1] - CELL).abs() < 1e-4).collect();
        assert_eq!(hands.len(), 2);
        for h in hands {
            assert!((h.pos[0] - ahead).abs() < 0.02 && (h.pos[1] - up).abs() < 0.02, "hand at {:?}, grip {ahead} {up}", h.pos);
            assert!((h.pos[2].abs() - 2.5 * CELL).abs() < 1e-3);
        }
    }

    #[test]
    fn what_is_carried_is_shaken_by_a_landing_and_comes_to_rest() {
        let mut g = Gait::new(0.0);
        let dt = 1.0 / 60.0;
        // Falling at five metres a second, then stopped by the ground.
        g.update(Doing::Falling, 0.0, None, false, None, -5.0, dt);
        g.update(Doing::Falling, 0.0, None, false, None, -5.0, dt);
        let mut thrown = [0.0f32; 2];
        for _ in 0..30 {
            g.update(Doing::Afoot(0.0), 0.0, None, false, None, 0.0, dt);
            let j = g.pose().jiggle;
            thrown = [thrown[0].max(j[0].abs()), thrown[1].max(j[1].abs())];
        }
        assert!(thrown[0] > m(0.01) && thrown[0] <= m(THROW) + 1e-5, "the pack moved {}", thrown[0]);
        assert!(thrown[1] > 0.05 && thrown[1] <= 0.26, "the rod swung {}", thrown[1]);
        for _ in 0..240 {
            g.update(Doing::Afoot(0.0), 0.0, None, false, None, 0.0, dt);
        }
        let j = g.pose().jiggle;
        assert!(j[0].abs() < m(0.002) && j[1].abs() < 0.01, "still moving: {j:?}");
        // However long the frame, it stays within its throw.
        g.update(Doing::Falling, 0.0, None, false, None, -40.0, 0.1);
        assert!(g.pose().jiggle[0].abs() <= m(THROW) + 1e-5);
    }

    #[test]
    fn the_arm_by_the_furled_glider_reaches_for_it_and_carries_it_up() {
        let g = Gait::new(0.0);
        let start = g.stowing(Doing::Afoot(0.0), 0.0);
        // Up past the shoulder and bent back behind the head; the other arm at rest.
        assert!(start.arm[1] < -std::f32::consts::PI && start.fore[1] < -1.0 && start.arm[0].abs() < 0.2);
        assert!(start.head > 0.3, "looks round at it");
        let end = g.stowing(Doing::Gliding, 1.0);
        assert!((end.arm[1] - REACH).abs() < 1e-4 && (end.arm[0] - REACH).abs() < 1e-4 && end.fore == [0.0; 2]);
    }

    #[test]
    fn a_figure_turns_the_short_way_round_to_its_heading() {
        let mut g = Gait::new(3.0);
        for _ in 0..120 {
            g.update(Doing::Afoot(2.0), 0.0, Some(-3.0), false, None, 0.0, 1.0 / 60.0);
        }
        let pi = std::f32::consts::PI;
        let off = (g.facing + 3.0 + pi).rem_euclid(2.0 * pi) - pi;
        assert!(off.abs() < 0.01, "{}", g.facing);
        assert!(g.facing > 3.0, "went the long way: {}", g.facing);
    }
}
