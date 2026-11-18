//! People: the player's own figure, and everyone else's on the same body.
//!
//! A figure is 18 cells tall, each a little under 0.1 m, the scale the plants and
//! lanterns are built at: legs with knees, a torso, arms with elbows, a neck and a head
//! three cells each way. It is not part of the voxel world, since it moves: each part is
//! a handful of boxes drawn with the boats and the glider, turned about its joint. Who
//! someone is comes from their [`Look`]: their build, skin and hair, their clothes, and
//! what they carry. Hair, cloaks and skirts hang from the joints they would, so they go
//! with the body; `people.rs` makes a whole person's look from a seed.
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
pub type Rgb = [u8; 3];

const LEATHER: Rgb = [0x8d, 0x5c, 0x37];
const EYE: Rgb = [0x1d, 0x18, 0x15];
const ROLL: Rgb = [0xe0, 0xd5, 0xb8];
/// The glider furled: its pole, its cloth, and the tie about it (as vehicles.rs has them).
const POLE: Rgb = [123, 108, 94];
const FURL: Rgb = [220, 147, 133];
const TIE: Rgb = [237, 222, 199];
/// What tools are made of.
const WOOD: Rgb = [0x8a, 0x66, 0x40];
const IRON: Rgb = [0x5e, 0x60, 0x64];
const WICKER: Rgb = [0xb4, 0x8e, 0x56];
const CLAY: Rgb = [0xa8, 0x62, 0x3e];
const TIN: Rgb = [0x9a, 0x98, 0x90];
const GREENS: Rgb = [0x5a, 0x7a, 0x34];
const FLETCH: Rgb = [0xe2, 0xdc, 0xcc];
/// A beekeeper's veil, pale and fine enough to see through from inside.
const VEIL: Rgb = [0xe6, 0xe2, 0xd4];
/// The twigs of a besom.
const BESOM: Rgb = [0xa8, 0x8a, 0x52];

/// How someone is built: the player's body, taller or shorter, broader or slighter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Build {
    /// The size of every cell against the player's.
    pub scale: f32,
    /// Breadth and depth of the body against the player's; the limbs thicken less.
    pub girth: f32,
    /// The head's size against the body's: more than 1 for a child.
    pub head: f32,
    /// How far the back is bent forward from the hips, in radians, with age.
    pub stoop: f32,
}

impl Build {
    pub const PLAYER: Build = Build { scale: 1.0, girth: 1.0, head: 1.0, stoop: 0.0 };

    /// Someone `metres` tall from the soles to the crown, with a head `head` the size
    /// of the player's against their body.
    pub fn standing(metres: f32, girth: f32, head: f32, stoop: f32) -> Self {
        Build { scale: metres / ((CELLS - 3.0 + 3.0 * head) * 1.76 / CELLS), girth, head, stoop }
    }

    /// From the soles to the crown, standing straight, in metres.
    pub fn metres(&self) -> f32 {
        (CELLS - 3.0 + 3.0 * self.head) * self.scale * 1.76 / CELLS
    }
}

/// How the hair is worn.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Hair {
    Cropped,
    Short,
    Shoulder,
    Long,
    Plaited,
    Bun,
    TopKnot,
    Shaved,
    Receding,
}

impl Hair {
    #[cfg(test)]
    pub const ALL: [Hair; 9] = [Hair::Cropped, Hair::Short, Hair::Shoulder, Hair::Long, Hair::Plaited, Hair::Bun, Hair::TopKnot, Hair::Shaved, Hair::Receding];
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Beard {
    None,
    Moustache,
    Short,
    Full,
}

impl Beard {
    #[cfg(test)]
    pub const ALL: [Beard; 4] = [Beard::None, Beard::Moustache, Beard::Short, Beard::Full];
}

/// What is worn next to the skin: a tunic and trousers (the player's), or a smock over
/// trousers to the knee, or a dress to the shin, or a robe to the ankle.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Garment {
    Tunic,
    Smock,
    Dress,
    Robe,
}

impl Garment {
    #[cfg(test)]
    pub const ALL: [Garment; 4] = [Garment::Tunic, Garment::Smock, Garment::Dress, Garment::Robe];
}

/// What is worn over the garment, in the `over` colour.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Layer {
    None,
    /// From the waist to the knee.
    Apron,
    /// An apron with a bib over the chest: the smith's leather, the innkeeper's linen.
    Bib,
    Waistcoat,
    Cloak,
    Shawl,
}

impl Layer {
    #[cfg(test)]
    pub const ALL: [Layer; 6] = [Layer::None, Layer::Apron, Layer::Bib, Layer::Waistcoat, Layer::Cloak, Layer::Shawl];
}

/// What is worn on the head, in the `hat` colour.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Headwear {
    None,
    StrawHat,
    Cap,
    Hood,
    Headscarf,
    /// Cloth wound about the head, in hot country.
    Wrap,
    FurHat,
    /// The beekeeper's: a broad hat with a veil hung from its brim.
    Veil,
    /// The fisher's oilskin hat, its brim long behind.
    Souwester,
}

impl Headwear {
    #[cfg(test)]
    pub const ALL: [Headwear; 9] = [
        Headwear::None,
        Headwear::StrawHat,
        Headwear::Cap,
        Headwear::Hood,
        Headwear::Headscarf,
        Headwear::Wrap,
        Headwear::FurHat,
        Headwear::Veil,
        Headwear::Souwester,
    ];

    /// Covers the hair all round, so nothing of it hangs out.
    fn covers(self) -> bool {
        matches!(self, Headwear::Hood | Headwear::Headscarf | Headwear::Wrap | Headwear::Veil)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Footwear {
    Boots,
    Shoes,
    Sandals,
    Bare,
}

impl Footwear {
    #[cfg(test)]
    pub const ALL: [Footwear; 4] = [Footwear::Boots, Footwear::Shoes, Footwear::Sandals, Footwear::Bare];
}

/// What is carried, the signs of a trade, kept to a few simple boxes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Carried {
    None,
    /// Over the right shoulder, held by the haft.
    Axe,
    Hoe,
    Rod,
    /// Hung from a hand.
    Basket,
    Hammer,
    Smoker,
    Jug,
    /// Planted beside the left foot at each step.
    Staff,
    Crook,
    /// A walking stick, for the old.
    Stick,
    /// A bow and a quiver across the back.
    Bow,
    /// A purse at the belt.
    Purse,
    /// A wooden pail hung from the right hand, for fetching water (#204).
    Bucket,
    /// A besom, over the shoulder or swept along the ground before the feet.
    Broom,
    /// A bundle of firewood held in both arms before the chest.
    Load,
}

impl Carried {
    #[cfg(test)]
    pub const ALL: [Carried; 16] = [
        Carried::None,
        Carried::Axe,
        Carried::Hoe,
        Carried::Rod,
        Carried::Basket,
        Carried::Hammer,
        Carried::Smoker,
        Carried::Jug,
        Carried::Staff,
        Carried::Crook,
        Carried::Stick,
        Carried::Bow,
        Carried::Purse,
        Carried::Bucket,
        Carried::Broom,
        Carried::Load,
    ];
}

/// What sets one person apart from another.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub skin: Rgb,
    pub hair: Rgb,
    /// The garment's body and sleeves.
    pub top: Rgb,
    /// The trousers, or the skirt of a dress or robe.
    pub bottom: Rgb,
    pub boots: Rgb,
    /// The scarf, and the tie of a plait.
    pub accent: Rgb,
    /// Carries a pack with a bedroll on it.
    pub pack: bool,
    pub build: Build,
    pub cut: Hair,
    pub beard: Beard,
    pub garment: Garment,
    pub layer: Layer,
    pub headwear: Headwear,
    pub footwear: Footwear,
    pub carried: Carried,
    /// The layer over the garment, and the headwear.
    pub over: Rgb,
    pub hat: Rgb,
    pub belt: Rgb,
    /// Wears the scarf; or else the neck shows.
    pub scarf: bool,
    /// Sleeves rolled to the elbow, for work.
    pub rolled: bool,
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
    build: Build::PLAYER,
    cut: Hair::Short,
    beard: Beard::None,
    garment: Garment::Tunic,
    layer: Layer::None,
    headwear: Headwear::None,
    footwear: Footwear::Boots,
    carried: Carried::None,
    over: [0x6b, 0x4a, 0x33],
    hat: [0x6b, 0x4a, 0x33],
    belt: LEATHER,
    scarf: true,
    rolled: false,
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

pub(crate) fn shade(c: Rgb, by: f32) -> Rgb {
    c.map(|v| (v as f32 * by).round().clamp(0.0, 255.0) as u8)
}

pub(crate) fn blend(a: Rgb, b: Rgb, t: f32) -> Rgb {
    [0, 1, 2].map(|k| (a[k] as f32 + (b[k] as f32 - a[k] as f32) * t).round() as u8)
}

fn linear(c: Rgb) -> [f32; 4] {
    let l = |v: u8| (v as f32 / 255.0).powf(2.2);
    [l(c[0]), l(c[1]), l(c[2]), 1.0]
}

//// Where a figure stands and which way it faces, for setting its parts in the world.
struct Frame {
    /// The feet, from the camera, raised as the pose has them.
    base: Vec3,
    yaw: f32,
    /// One cell of this figure, in voxels.
    cell: f32,
    /// How far the whole figure is laid back about the axis through its feet, as a part
    /// is pitched about its joint: a quarter turn back for someone lying in bed. Only the
    /// settlements' people lie down; for the player it is always 0.
    lie: f32,
}

impl Frame {
    fn new(feet: DVec3, cam: DVec3, facing: f32, lift: f32, cell: f32) -> Self {
        // The figure looks along its own z; a camera at yaw 0 looks along x.
        Frame { base: (feet - cam).as_vec3() + Vec3::Y * lift, yaw: facing - std::f32::consts::FRAC_PI_2, cell, lie: 0.0 }
    }

    /// Boxes of a part hung from `joint` (cells, in the figure's frame), the part turned
    /// by `pitch` about the hips' axis and by `turn` about the upright. Each box is first
    /// rolled by `roll` about the part's own z, for what is slung aslant.
    fn part(&self, out: &mut Vec<ParticleInstance>, joint: Vec3, pitch: f32, turn: f32, roll: f32, pieces: &[Piece]) {
        let yaw = self.yaw;
        let (sp, cp) = pitch.sin_cos();
        let (sy, cy) = (yaw + turn).sin_cos();
        for (lo, size, color) in pieces {
            let (lo, size) = (Vec3::from(*lo), Vec3::from(*size));
            let mut mid = lo + size * 0.5;
            if roll != 0.0 {
                let (sr, cr) = roll.sin_cos();
                mid = Vec3::new(mid.x * cr - mid.y * sr, mid.x * sr + mid.y * cr, mid.z);
            }
            let tipped = Vec3::new(mid.x, mid.y * cp - mid.z * sp, mid.y * sp + mid.z * cp);
            // The joint turns with the body; the part about its joint with its own turn.
            let (by, bx) = yaw.sin_cos();
            let (at, tilt) = if self.lie == 0.0 {
                let at = Vec3::new(joint.x * bx - joint.z * by, joint.y, joint.x * by + joint.z * bx)
                    + Vec3::new(tipped.x * cy - tipped.z * sy, tipped.y, tipped.x * sy + tipped.z * cy);
                (at, pitch)
            } else {
                // Laid back whole about the feet, which is one more pitch of every part about
                // the same axis (a lying figure's head is not turned, so the turn is none).
                let (st, ct) = turn.sin_cos();
                let local = joint + Vec3::new(tipped.x * ct - tipped.z * st, tipped.y, tipped.x * st + tipped.z * ct);
                let (sl, cl) = self.lie.sin_cos();
                let laid = Vec3::new(local.x, local.y * cl - local.z * sl, local.y * sl + local.z * cl);
                (Vec3::new(laid.x * bx - laid.z * by, laid.y, laid.x * by + laid.z * bx), pitch + self.lie)
            };
            out.push(ParticleInstance {
                pos: (self.base + at * self.cell).into(),
                yaw: yaw + turn,
                size: (size * self.cell).into(),
                glow: 0.0,
                color: linear(*color),
                tilt: [tilt, roll],
                dissolve: 0.0,
            });
        }
    }
}

/// Where a joint `down` cells along a limb hung from `joint` at `pitch` has got to.
fn along(joint: Vec3, pitch: f32, down: f32) -> Vec3 {
    joint + Vec3::new(0.0, -down * pitch.cos(), -down * pitch.sin())
}

/// The hips, the joint the legs hang from and the body leans about.
const HIPS: Vec3 = Vec3::new(0.0, 9.0, 0.0);

/// Where a point of the upper body has got to with the body leaning `lean` from the hips.
fn leaning(p: Vec3, lean: f32) -> Vec3 {
    let (o, (sl, cl)) = (p - HIPS, lean.sin_cos());
    HIPS + Vec3::new(o.x, o.y * cl - o.z * sl, o.y * sl + o.z * cl)
}

/// A tool on the shoulder holds the right arm up to it: the turn at the shoulder and at
/// the elbow that puts the hand before the shoulder, and the haft's own pitch from there
/// up over it.
const SHOULDERED: (f32, f32, f32) = (-0.45, -1.9, -1.06);

/// The pose a figure takes with what it carries: a tool on the shoulder holds the right
/// arm still, a staff the left arm out to it; what hangs from a hand lets it swing.
fn holding(carried: Carried, pose: &Pose) -> Pose {
    let mut p = *pose;
    match carried {
        Carried::Axe | Carried::Hoe | Carried::Rod | Carried::Broom => {
            p.arm[0] = SHOULDERED.0;
            p.fore[0] = SHOULDERED.1;
        }
        Carried::Staff | Carried::Crook | Carried::Stick => {
            p.arm[1] = -0.3 + p.arm[1] * 0.3;
            p.fore[1] = -0.55;
        }
        _ => {}
    }
    p
}

impl Garment {
    /// Whether it hangs as a skirt, hiding the thighs.
    pub fn skirted(self) -> bool {
        matches!(self, Garment::Dress | Garment::Robe)
    }

    /// How far it hangs below the hips, in cells.
    fn hem(self) -> f32 {
        match self {
            Garment::Tunic => 0.0,
            Garment::Smock => 3.6,
            Garment::Dress => 6.4,
            Garment::Robe => 8.0,
        }
    }
}

/// How a figure's measures follow from its build: the half width of a leg, the
/// thickness of an arm, and half the depth of the body, in cells.
fn measures(b: &Build) -> (f32, f32, f32) {
    let g = b.girth;
    // The limbs thicken less than the body, and never so far as to meet.
    (g.min(1.0 + (g - 1.0) * 0.5), 1.0 + (g - 1.0) * 0.4, 1.0 + (g - 1.0) * 0.8)
}

/// The colour of the legs below a skirt: stockings, or bare above sandals.
fn shins(look: &Look) -> Rgb {
    if !look.garment.skirted() {
        shade(look.bottom, 0.94)
    } else if matches!(look.footwear, Footwear::Sandals | Footwear::Bare) {
        shade(look.skin, 0.96)
    } else {
        blend(look.boots, [0x5a, 0x52, 0x4a], 0.5)
    }
}

/// Draws a figure standing with its feet at `feet`, for a camera at `cam`. `furled` is
/// the glider carried as a rod: 0 strapped upright to the back beside the pack, up to 1
/// swung overhead to where the wing opens from; None once the wing is out.
pub fn draw(look: &Look, feet: DVec3, facing: f32, pose: &Pose, furled: Option<f32>, cam: DVec3, out: &mut Vec<ParticleInstance>) {
    draw_doing(look, feet, facing, pose, furled, None, cam, out);
}

/// Draws someone of the settlements at `task` (#204), as [`draw`] does: in the pose
/// [`task_pose`] gives (or any other), with what the task is done with in their hands
/// rather than on the shoulder, and laid back in bed for [`Task::Lie`], when `feet` is
/// where their soles are and `facing` the way from their head to them.
pub fn draw_at(look: &Look, feet: DVec3, facing: f32, pose: &Pose, task: Task, cam: DVec3, out: &mut Vec<ParticleInstance>) {
    draw_doing(look, feet, facing, pose, None, Some(task), cam, out);
}

#[allow(clippy::too_many_arguments)]
fn draw_doing(look: &Look, feet: DVec3, facing: f32, pose: &Pose, furled: Option<f32>, task: Option<Task>, cam: DVec3, out: &mut Vec<ParticleInstance>) {
    // A tool being worked with is in the hands, which go where the task puts them.
    let in_use = task.is_some_and(|t| t.in_use(look.carried));
    let pose = &if in_use { *pose } else { holding(look.carried, pose) };
    let b = look.build;
    let mut frame = Frame::new(feet, cam, facing, pose.lift * b.scale, CELL * b.scale);
    if task == Some(Task::Lie) {
        frame.lie = -std::f32::consts::FRAC_PI_2;
    }
    let mut part = |joint: Vec3, pitch: f32, turn: f32, pieces: &[Piece]| frame.part(out, joint, pitch, turn, 0.0, pieces);
    let lean = pose.lean + b.stoop;
    let leant = |p: Vec3| leaning(p, lean);
    let hips = HIPS;
    let g = b.girth;
    let (limb, arm, deep) = measures(&b);
    let skirted = look.garment.skirted();
    let shin = shins(look);

    for (k, side) in [-1.0f32, 1.0].into_iter().enumerate() {
        // Thigh, then shin and boot from the knee. Under a skirt the thighs never show,
        // and under a robe the shins neither.
        let hip = Vec3::new(side * g, 9.0, 0.0);
        if !skirted {
            part(hip, pose.leg[k], 0.0, &[([-limb, -4.0, -limb], [2.0 * limb, 4.0, 2.0 * limb], look.bottom)]);
        }
        let knee = along(hip, pose.leg[k], 4.0);
        let (ankle, foot, toe) = match look.footwear {
            Footwear::Boots => (3.0, look.boots, look.boots),
            Footwear::Shoes => (4.0, look.boots, look.boots),
            Footwear::Sandals => (4.0, look.boots, look.skin),
            Footwear::Bare => (4.0, shade(look.skin, 0.93), look.skin),
        };
        let leg: [Piece; 3] = [
            ([-limb, -ankle, -limb], [2.0 * limb, ankle, 2.0 * limb], shin),
            ([-limb, -5.0, -limb], [2.0 * limb, 5.0 - ankle, 2.0 * limb], foot),
            ([-limb, -5.0, limb], [2.0 * limb, 1.0, 1.0], toe),
        ];
        part(knee, pose.leg[k] + pose.shin[k], 0.0, if look.garment == Garment::Robe { &leg[1..] } else { &leg });
        // Upper arm, then forearm and hand from the elbow.
        let shoulder = leant(Vec3::new(side * (2.0 * g + 0.5 * arm), 13.5, 0.0));
        part(shoulder, pose.arm[k], 0.0, &[([-0.5 * arm, -2.5, -arm], [arm, 3.0, 2.0 * arm], look.top)]);
        let elbow = along(shoulder, pose.arm[k], 2.5);
        let sleeve = if look.rolled { shade(look.skin, 0.97) } else { shade(look.top, 0.94) };
        part(
            elbow,
            pose.arm[k] + pose.fore[k],
            0.0,
            &[([-0.5 * arm, -2.0, -arm], [arm, 2.0, 2.0 * arm], sleeve), ([-0.5 * arm, -3.0, -arm], [arm, 1.0, 2.0 * arm], look.skin)],
        );
    }

    let strap = shade(LEATHER, 0.85);
    let mut body: Vec<Piece> = vec![
        ([-2.0 * g, 0.0, -deep], [4.0 * g, 1.0, 2.0 * deep], look.belt),
        ([-2.0 * g, 1.0, -deep], [4.0 * g, 4.0, 2.0 * deep], look.top),
    ];
    if look.scarf {
        body.extend([
            // The scarf, wound about the neck, an end hanging in front.
            ([-2.0 * g, 5.0, -2.0 * deep], [4.0 * g, 1.0, 4.0 * deep], look.accent),
            ([-2.0 * g, 3.0, deep], [1.0, 2.0, 1.0], shade(look.accent, 0.92)),
        ]);
    } else {
        // The neck, between the collar and the head.
        body.push(([-1.0, 5.0, -0.85], [2.0, 1.0, 1.7], look.skin));
    }
    let mut pack: Vec<Piece> = Vec::new();
    if look.pack {
        // No two of these, nor any and the body or scarf, share a face: the bag stands
        // clear below the scarf, its flap overhangs it, the bedroll rides behind the
        // scarf's knot, and the straps stop short of the scarf and clear of its end.
        let back = deep - 1.0;
        pack.extend([
            // Set to one side, leaving room beside it for the furled glider.
            ([-1.75, 1.2, -2.5 - back], [2.75, 2.7, 1.45], LEATHER),
            ([-1.9, 3.9, -2.7 - back], [3.0, 0.6, 1.65], shade(LEATHER, 1.18)),
            ([-1.98, 4.5, -2.95 - back], [3.1, 0.8, 0.85], ROLL),
        ]);
        body.extend([([-0.95, 1.05, deep], [0.45, 3.9, 0.08], strap), ([1.2, 1.05, deep], [0.45, 3.9, 0.08], strap)]);
    }
    // What is worn over the body: none of it on the player, so it comes after.
    let wide = 2.0 * g + 1.1;
    match look.layer {
        Layer::Bib => body.push(([-1.3 * g, 1.3, deep], [2.6 * g, 3.5, 0.12], look.over)),
        Layer::Waistcoat => body.extend([
            ([-2.0 * g - 0.08, 1.0, -deep - 0.08], [4.0 * g + 0.16, 3.9, 2.0 * deep + 0.16], look.over),
            // The shirt shows down the front between its edges.
            ([-0.45 * g, 1.0, deep + 0.08], [0.9 * g, 3.9, 0.05], look.top),
        ]),
        Layer::Shawl => body.extend([
            ([-wide, 3.5, -deep - 0.25], [2.0 * wide, 1.6, 2.0 * deep + 0.5], look.over),
            // Its point hangs down the back.
            ([-1.4 * g, 1.6, -deep - 0.3], [2.8 * g, 1.9, 0.14], shade(look.over, 0.94)),
        ]),
        // Over the shoulders, the cloak itself hung from behind them.
        Layer::Cloak => body.push(([-wide - 0.05, 4.4, -deep - 0.3], [2.0 * wide + 0.1, 1.0, 2.0 * deep + 0.6], look.over)),
        Layer::None | Layer::Apron => {}
    }
    if look.carried == Carried::Purse {
        body.push(([0.7 * g, -0.9, deep], [0.8, 1.2, 0.55], shade(LEATHER, 0.7)));
    }
    part(hips, lean, 0.0, &body);
    // The pack, riding a little loose on the back.
    part(hips + Vec3::Y * (pose.jiggle[0] / (CELL * b.scale)), lean, 0.0, &pack);

    if let Some(swing) = furled {
        // From the back, by the shoulder, up and over to lie along the wing's middle.
        let t = swing.clamp(0.0, 1.0);
        let e = t * t * (3.0 - 2.0 * t);
        // Behind the scarf, against the side of the pack.
        let on_back = leant(Vec3::new(1.58, 14.0, -2.45));
        let (ahead, up) = grip();
        let overhead = Vec3::new(0.0, up / CELL + 5.6, ahead / CELL + 2.0);
        // Strapped by its top, it swings a little as the figure goes; in the hand it does not.
        let pitch = lean + pose.jiggle[1] * (1.0 - e) + (std::f32::consts::FRAC_PI_2 - lean) * e;
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
    // Hair cropped close shows the skin through it; a shaved head is all scalp.
    let (crown, nape) = match look.cut {
        Hair::Shaved => (blend(look.skin, look.hair, 0.12), blend(look.skin, look.hair, 0.12)),
        Hair::Cropped => (blend(look.hair, look.skin, 0.3), blend(look.hair, look.skin, 0.3)),
        _ => (look.hair, look.hair),
    };
    let mut head: Vec<Piece> = vec![
        ([-1.5, 0.0, -1.5], [3.0, 1.0, 3.0], look.skin),
        ([-1.5, 1.0, -1.5], [3.0, 1.0, 1.0], nape),
        ([-1.5, 1.0, -0.5], [3.0, 1.0, 1.0], look.skin),
        ([-1.5, 1.0, 0.5], [1.0, 1.0, 1.0], eye),
        ([-0.5, 1.0, 0.5], [1.0, 1.0, 1.0], look.skin),
        ([0.5, 1.0, 0.5], [1.0, 1.0, 1.0], eye),
    ];
    if look.cut == Hair::Receding {
        // Bare at the brow, the hair kept further back.
        head.extend([([-1.5, 2.0, -1.5], [3.0, 1.0, 2.0], crown), ([-1.5, 2.0, 0.5], [3.0, 1.0, 1.0], look.skin)]);
    } else {
        head.push(([-1.5, 2.0, -1.5], [3.0, 1.0, 3.0], crown));
    }
    hair(look, &mut head);
    beard(look, &mut head);
    headwear(look, &mut head);
    let hk = b.head;
    for (lo, size, _) in &mut head {
        *lo = lo.map(|v| v * hk);
        *size = size.map(|v| v * hk);
    }
    // A stooping head is lifted to look ahead.
    part(leant(Vec3::new(0.0, 15.0, 0.0)), lean * 0.4 - b.stoop * 0.9, pose.head, &head);

    // A skirt hangs from the hips in two tiers, the lower flared. It goes with both legs,
    // and deepens as the stride opens so they stay inside it; the apron on its front.
    let spread = (pose.leg[0] - pose.leg[1]).abs();
    let swing = (pose.leg[0] + pose.leg[1]) * 0.5;
    let hem = look.garment.hem();
    let upper = hem.min(4.0);
    let front = deep + 0.15 + spread * 0.3;
    let apron = |top: f32, length: f32, z: f32| ([-1.6 * g, -length, z], [3.2 * g, length + top, 0.12], look.over);
    if hem > 0.0 {
        let cloth = if skirted { look.bottom } else { shade(look.top, 0.97) };
        let mut tier = vec![([-2.0 * g - 0.12, -upper, -front], [4.0 * g + 0.24, upper + 0.25, 2.0 * front], cloth)];
        if matches!(look.layer, Layer::Apron | Layer::Bib) {
            tier.push(apron(0.2, upper - 0.2, front));
        }
        part(hips, swing, 0.0, &tier);
        if hem > upper {
            let flare = deep + 0.45 + spread * 1.2;
            part(
                along(hips, swing, upper),
                swing * 0.3,
                0.0,
                &[([-2.0 * g - 0.3, -(hem - upper), -flare], [4.0 * g + 0.6, hem - upper + 0.3, 2.0 * flare], shade(cloth, 0.96))],
            );
        }
    } else if matches!(look.layer, Layer::Apron | Layer::Bib) {
        // Over trousers it rides on the leading thigh, which would push through it.
        part(hips, pose.leg[0].min(pose.leg[1]), 0.0, &[apron(0.2, 4.6, limb + 0.05)]);
    }
    if look.layer == Layer::Cloak {
        // Hung from behind the shoulders, it lies along the back and streams out
        // behind as the figure goes.
        let flare = lean.max(0.0) + 0.05 + spread * 0.12;
        let length = 14.3 - 3.2;
        part(
            leant(Vec3::new(0.0, 14.3, -deep - 0.12)),
            flare,
            0.0,
            &[([-wide + 0.75, -length, -0.26], [2.0 * wide - 1.5, length + 0.2, 0.26], shade(look.over, 0.95))],
        );
    }

    // What is carried: on the shoulder, in a hand, or slung.
    let hand = |k: usize| {
        let side = [-1.0f32, 1.0][k];
        let elbow = along(leant(Vec3::new(side * (2.0 * g + 0.5 * arm), 13.5, 0.0)), pose.arm[k], 2.5);
        along(elbow, pose.arm[k] + pose.fore[k], 2.5)
    };
    let dark = shade(WOOD, 0.75);
    match look.carried {
        // Worked with: gripped near the end of the haft, in line with the forearm, so it
        // swings as the arms do; the rod held out over the water with its line hanging.
        Carried::Axe | Carried::Hoe | Carried::Broom if in_use => {
            let tool: &[Piece] = match look.carried {
                Carried::Axe => &[([-0.17, -8.0, -0.17], [0.34, 8.8, 0.34], WOOD), ([-0.12, -8.2, -1.5], [0.24, 1.4, 1.45], IRON)],
                Carried::Hoe => &[([-0.15, -11.0, -0.15], [0.3, 12.2, 0.3], WOOD), ([-0.75, -11.2, -1.6], [1.5, 0.35, 1.6], IRON)],
                _ => &[([-0.15, -10.0, -0.15], [0.3, 11.0, 0.3], WOOD), ([-0.8, -11.6, -0.55], [1.6, 1.8, 1.1], BESOM)],
            };
            // The broom's head is held down to the ground before the feet, swept from side
            // to side; the others swing with the forearm.
            let (pitch, turn) = match look.carried {
                Carried::Broom => (-0.5, pose.head),
                _ => (pose.arm[0] + pose.fore[0], 0.0),
            };
            part(hand(0), pitch, turn, tool);
        }
        Carried::Rod if in_use => {
            let h = hand(0);
            let pitch = 1.1 + pose.jiggle[1];
            part(h, pitch, 0.0, &[([-0.1, -1.5, -0.1], [0.2, 15.5, 0.2], dark), ([-0.18, -0.5, -0.18], [0.36, 1.4, 0.36], shade(LEATHER, 0.7))]);
            // The line from the tip straight down, to below the feet.
            let tip = along(h, pitch, -14.0);
            part(tip, 0.0, 0.0, &[([-0.04, -tip.y - 3.0, -0.04], [0.08, tip.y + 3.0, 0.08], FLETCH)]);
        }
        Carried::Bucket => {
            part(
                hand(0) + Vec3::new(-0.4, 0.0, 0.0),
                0.0,
                0.0,
                &[([-0.9, -3.2, -0.9], [1.8, 1.9, 1.8], shade(WOOD, 1.08)), ([-0.95, -2.2, -0.95], [1.9, 0.25, 1.9], IRON), ([-0.08, -1.3, -0.08], [0.16, 1.3, 0.16], IRON)],
            );
        }
        Carried::Load => {
            // Split logs across the arms, held to the chest.
            part(
                hips,
                lean,
                0.0,
                &[([-1.8 * g, 1.3, deep + 0.15], [3.6 * g, 1.0, 2.8], WOOD), ([-1.6 * g, 2.3, deep + 0.3], [3.2 * g, 0.9, 2.5], shade(WOOD, 0.82))],
            );
        }
        Carried::Axe | Carried::Hoe | Carried::Rod | Carried::Broom => {
            // Held by the haft before the shoulder, the head behind.
            let tool: &[Piece] = match look.carried {
                Carried::Axe => &[([-0.17, -1.2, -0.17], [0.34, 8.6, 0.34], WOOD), ([-0.12, 6.1, -1.5], [0.24, 1.4, 1.45], IRON)],
                Carried::Hoe => &[([-0.15, -1.5, -0.15], [0.3, 12.5, 0.3], WOOD), ([-0.75, 10.6, -1.6], [1.5, 0.35, 1.6], IRON)],
                Carried::Broom => &[([-0.15, -1.5, -0.15], [0.3, 11.0, 0.3], WOOD), ([-0.8, 9.5, -0.55], [1.6, 1.8, 1.1], BESOM)],
                _ => &[([-0.1, -1.5, -0.1], [0.2, 15.5, 0.2], dark), ([-0.18, 0.5, -0.18], [0.36, 1.4, 0.36], shade(LEATHER, 0.7))],
            };
            part(hand(0), SHOULDERED.2, 0.0, tool);
        }
        Carried::Hammer => {
            // Gripped, so it goes in line with the forearm.
            part(
                hand(0),
                pose.arm[0] + pose.fore[0],
                0.0,
                &[([-0.15, -3.0, -0.15], [0.3, 3.6, 0.3], WOOD), ([-0.3, -3.6, -0.8], [0.6, 0.7, 1.6], IRON)],
            );
        }
        Carried::Basket => {
            // Hung from the left hand by its handle, held out clear of the leg.
            part(
                hand(1) + Vec3::new(0.8, 0.0, 0.0),
                0.0,
                0.0,
                &[
                    ([-0.1, -1.4, -0.1], [0.2, 1.5, 0.2], shade(WICKER, 0.8)),
                    ([-0.85, -3.0, -1.25], [1.7, 1.45, 2.5], WICKER),
                    ([-0.92, -1.62, -1.32], [1.84, 0.22, 2.64], shade(WICKER, 0.8)),
                    ([-0.7, -1.5, -1.05], [1.4, 0.3, 2.1], GREENS),
                ],
            );
        }
        Carried::Smoker | Carried::Jug => {
            let pieces: [Piece; 2] = if look.carried == Carried::Smoker {
                [([-0.5, -2.0, -0.5], [1.0, 1.3, 1.0], TIN), ([-0.22, -0.75, -0.22], [0.44, 0.6, 0.44], shade(TIN, 0.8))]
            } else {
                [([-0.6, -2.3, -0.6], [1.2, 1.6, 1.2], CLAY), ([-0.35, -0.75, -0.35], [0.7, 0.55, 0.7], shade(CLAY, 0.88))]
            };
            part(hand(0) + Vec3::new(-0.6, 0.0, 0.0), 0.0, 0.0, &pieces);
        }
        Carried::Staff | Carried::Crook | Carried::Stick => {
            // Upright from the ground beside the left foot, through the hand.
            let h = hand(1);
            let at = Vec3::new(h.x + 0.2, 0.0, h.z);
            match look.carried {
                Carried::Stick => part(at, 0.0, 0.0, &[([-0.15, 0.0, -0.15], [0.3, h.y + 0.5, 0.3], dark)]),
                Carried::Staff => part(at, 0.0, 0.0, &[([-0.17, 0.0, -0.17], [0.34, 19.0, 0.34], WOOD)]),
                _ => part(
                    at,
                    0.0,
                    0.0,
                    &[
                        ([-0.17, 0.0, -0.17], [0.34, 18.6, 0.34], WOOD),
                        ([-0.17, 18.6, -0.17], [0.34, 0.34, 1.6], WOOD),
                        ([-0.17, 17.4, 1.09], [0.34, 1.2, 0.34], WOOD),
                    ],
                ),
            }
        }
        Carried::Bow => {
            // Slung aslant across the back, the quiver over the other shoulder.
            frame.part(out, leant(Vec3::new(0.0, 11.5, -deep - 0.35)), lean, 0.0, 0.5, &[([-0.15, -6.2, -0.15], [0.3, 12.4, 0.3], dark)]);
            frame.part(
                out,
                leant(Vec3::new(0.9 * g, 12.0, -deep - 0.6)),
                lean,
                0.0,
                -0.35,
                &[([-0.5, -2.6, -0.45], [1.0, 4.4, 0.9], shade(LEATHER, 0.75)), ([-0.4, 1.8, -0.36], [0.8, 0.8, 0.72], FLETCH)],
            );
        }
        Carried::None | Carried::Purse => {}
    }
    if task == Some(Task::Sketch) {
        // The book open on the left hand, its far edge tipped up toward the eyes, reaching
        // in toward the middle of the body; the pencil in the right hand, held to the page.
        let h = hand(1);
        frame.part(out, h, -0.7, 0.0, 0.0, &[([-4.0, -0.35, -1.2], [4.0, 0.22, 2.6], shade(LEATHER, 0.8)), ([-3.9, -0.13, -1.1], [3.8, 0.1, 2.4], PAGE)]);
        frame.part(out, hand(0), pose.arm[0] + pose.fore[0], 0.0, 0.0, &[([-0.08, -1.3, -0.08], [0.16, 1.1, 0.16], shade(WOOD, 0.6))]);
    }
}

/// The pages of a sketchbook.
const PAGE: Rgb = [0xe8, 0xe0, 0xc8];

/// What of the hair shows beyond the head's own three layers: what hangs down the back,
/// a plait, a bun or a knot. None of it shows from under a hood or a scarf.
fn hair(look: &Look, head: &mut Vec<Piece>) {
    if look.headwear.covers() {
        return;
    }
    let hair = look.hair;
    match look.cut {
        Hair::Shoulder | Hair::Long => {
            let low = if look.cut == Hair::Long { -4.4 } else { -1.4 };
            head.extend([
                ([-1.62, low, -1.76], [3.24, 3.1 - low, 0.34], hair),
                // Over the ears, framing the face.
                ([-1.64, -0.6, -1.45], [0.14, 3.65, 1.95], hair),
                ([1.5, -0.6, -1.45], [0.14, 3.65, 1.95], hair),
            ]);
        }
        Hair::Plaited => head.extend([
            ([-0.38, -4.6, -1.95], [0.76, 5.0, 0.6], hair),
            ([-0.46, -4.9, -2.0], [0.92, 0.45, 0.7], look.accent),
        ]),
        Hair::Bun => head.push(([-0.8, 1.7, -2.35], [1.6, 1.5, 0.9], hair)),
        // A hat sits on it, so it shows only bare-headed.
        Hair::TopKnot if look.headwear == Headwear::None => head.push(([-0.6, 3.0, -0.7], [1.2, 1.0, 1.2], hair)),
        _ => {}
    }
}

/// A moustache, alone or over a beard.
fn beard(look: &Look, head: &mut Vec<Piece>) {
    let low = match look.beard {
        Beard::None => return,
        Beard::Moustache => None,
        Beard::Short => Some(-0.25),
        Beard::Full => Some(-1.15),
    };
    if let Some(low) = low {
        head.push(([-1.56, low, -0.35], [3.12, 0.72 - low, 1.95], look.hair));
    }
    head.push(([-1.0, 0.62, 1.5], [2.0, 0.36, 0.14], look.hair));
}

fn headwear(look: &Look, head: &mut Vec<Piece>) {
    let hat = look.hat;
    let rim = shade(hat, 0.9);
    match look.headwear {
        Headwear::None => {}
        Headwear::StrawHat | Headwear::Veil => {
            head.extend([([-1.6, 2.5, -1.6], [3.2, 1.25, 3.2], hat), ([-2.7, 2.55, -2.7], [5.4, 0.2, 5.4], rim)]);
            if look.headwear == Headwear::Veil {
                head.push(([-1.85, -0.7, -1.85], [3.7, 3.25, 3.7], VEIL));
            }
        }
        Headwear::Cap => head.extend([([-1.6, 2.45, -1.6], [3.2, 0.8, 3.2], hat), ([-1.45, 2.45, 1.6], [2.9, 0.18, 1.0], rim)]),
        Headwear::Hood => head.extend([
            ([-1.75, 2.6, -1.75], [3.5, 0.6, 3.4], hat),
            ([-1.75, -0.6, -1.75], [3.5, 3.2, 0.45], hat),
            ([-1.75, -0.6, -1.3], [0.3, 3.2, 2.85], rim),
            ([1.45, -0.6, -1.3], [0.3, 3.2, 2.85], rim),
        ]),
        Headwear::Headscarf => head.extend([
            ([-1.65, 2.35, -1.7], [3.3, 0.85, 3.25], hat),
            ([-1.65, -0.5, -1.75], [3.3, 2.9, 0.48], rim),
        ]),
        Headwear::Wrap => head.extend([([-1.7, 1.95, -1.7], [3.4, 1.4, 3.4], hat), ([-1.25, 3.35, -1.25], [2.5, 0.45, 2.5], rim)]),
        Headwear::FurHat => head.extend([
            ([-1.85, 2.2, -1.85], [3.7, 1.7, 3.7], hat),
            ([-1.95, 0.5, -1.0], [0.3, 1.8, 2.0], rim),
            ([1.65, 0.5, -1.0], [0.3, 1.8, 2.0], rim),
        ]),
        Headwear::Souwester => head.extend([([-1.6, 2.4, -1.6], [3.2, 1.0, 3.2], hat), ([-2.2, 2.35, -2.8], [4.4, 0.2, 4.6], rim)]),
    }
}

/// A figure far off, in eight boxes or fewer, coloured as the near one: the legs, the
/// body and arms, the head and what is on it, and the skirt, cloak or pack, whichever
/// shows most from a distance.
pub fn draw_far(look: &Look, feet: DVec3, facing: f32, pose: &Pose, cam: DVec3, out: &mut Vec<ParticleInstance>) {
    let pose = &holding(look.carried, pose);
    let b = look.build;
    let frame = Frame::new(feet, cam, facing, pose.lift * b.scale, CELL * b.scale);
    let mut part = |joint: Vec3, pitch: f32, pieces: &[Piece]| frame.part(out, joint, pitch, 0.0, 0.0, pieces);
    let lean = pose.lean + b.stoop;
    let g = b.girth;
    let (limb, arm, deep) = measures(&b);
    let legs = if look.garment.skirted() { shins(look) } else { look.bottom };
    for (k, side) in [-1.0f32, 1.0].into_iter().enumerate() {
        part(Vec3::new(side * g, 9.0, 0.0), pose.leg[k], &[([-limb, -9.0, -limb], [2.0 * limb, 9.0, 2.0 * limb], legs)]);
        let shoulder = leaning(Vec3::new(side * (2.0 * g + 0.5 * arm), 13.5, 0.0), lean);
        part(shoulder, pose.arm[k], &[([-0.5 * arm, -5.0, -arm], [arm, 5.5, 2.0 * arm], look.top)]);
    }
    part(HIPS, lean, &[([-2.0 * g, 0.0, -deep], [4.0 * g, 6.0, 2.0 * deep], look.top)]);
    let hk = b.head;
    let crown = match look.headwear {
        Headwear::None => ([-1.55, 2.0, -1.55], [3.1, 1.05, 3.1], look.hair),
        Headwear::StrawHat | Headwear::Veil => ([-2.6, 2.5, -2.6], [5.2, 0.9, 5.2], look.hat),
        _ => ([-1.7, 1.9, -1.7], [3.4, 1.5, 3.4], look.hat),
    };
    let head = [([-1.5, 0.0, -1.5], [3.0, 2.0, 3.0], look.skin), crown].map(|(lo, size, c)| (lo.map(|v| v * hk), size.map(|v| v * hk), c));
    part(leaning(Vec3::new(0.0, 15.0, 0.0), lean), lean * 0.4 - b.stoop * 0.9, &head);
    let spread = (pose.leg[0] - pose.leg[1]).abs();
    let hem = look.garment.hem();
    if hem > 0.0 {
        let front = deep + 0.3 + spread * 0.8;
        let cloth = if look.garment.skirted() { look.bottom } else { look.top };
        part(HIPS, (pose.leg[0] + pose.leg[1]) * 0.5, &[([-2.0 * g - 0.2, -hem, -front], [4.0 * g + 0.4, hem + 0.3, 2.0 * front], cloth)]);
    } else if look.layer == Layer::Cloak {
        let flare = lean.max(0.0) + 0.05 + spread * 0.12;
        part(leaning(Vec3::new(0.0, 14.3, -deep - 0.12), lean), flare, &[([-2.0 * g - 0.3, -11.1, -0.3], [4.0 * g + 0.6, 11.3, 0.3], look.over)]);
    } else if look.pack {
        part(HIPS, lean, &[([-1.75, 1.2, -2.5 - (deep - 1.0)], [3.0, 4.1, 1.5], LEATHER)]);
    }
}

/// What someone of the settlements is busy at (#204): the pose it holds them in, and
/// what is in their hands for it. The player is never drawn at any of them.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Task {
    /// At ease, looking about.
    Stand,
    /// An axe swung up overhead and down onto the block.
    Chop,
    /// A hoe worked into a garden bed.
    Hoe,
    /// A hammer on the anvil, the other hand holding the work in tongs.
    Hammer,
    /// A rod held out over the water.
    Fish,
    /// Firewood held in both arms.
    Carry,
    /// A pail of water in the hand.
    Fetch,
    /// A besom swept from side to side before the feet.
    Sweep,
    /// On a seat a stool high (see [`SEAT`]).
    Sit,
    /// On the ground, legs out before them, leaning back on their hands.
    SitLow,
    /// Talking with someone, a hand going as they speak.
    Talk,
    /// Bent to a hive or a rack, hands busy at it.
    Tend,
    /// On their back in bed.
    Lie,
    /// At the oars of a boat, facing its stern, pulling (#205). Sat as the player sits in
    /// a boat: the hips as high over `feet` as standing, so `feet` is set that far below
    /// the thwart.
    Row,
    /// On a seat a stool high, a sketchbook held up in the left hand and the right at work
    /// on it (#205).
    Sketch,
}

/// How high a seat is, in metres: a stool's, and about a bench's.
pub const SEAT: f32 = 0.5;

impl Task {
    #[cfg(test)]
    pub const ALL: [Task; 15] = [
        Task::Stand,
        Task::Chop,
        Task::Hoe,
        Task::Hammer,
        Task::Fish,
        Task::Carry,
        Task::Fetch,
        Task::Sweep,
        Task::Sit,
        Task::SitLow,
        Task::Talk,
        Task::Tend,
        Task::Lie,
        Task::Row,
        Task::Sketch,
    ];

    /// What it is, in words, for logs and the console.
    pub fn word(self) -> &'static str {
        match self {
            Task::Stand => "standing",
            Task::Chop => "chopping",
            Task::Hoe => "hoeing",
            Task::Hammer => "hammering",
            Task::Fish => "fishing",
            Task::Carry => "carrying wood",
            Task::Fetch => "fetching water",
            Task::Sweep => "sweeping",
            Task::Sit => "sitting",
            Task::SitLow => "sitting on the ground",
            Task::Talk => "talking",
            Task::Tend => "tending",
            Task::Lie => "asleep",
            Task::Row => "rowing",
            Task::Sketch => "sketching",
        }
    }

    /// What it is done with, for someone who would otherwise carry `own`; their own
    /// thing where the task needs nothing in particular, and nothing on the shoulder to
    /// sit or lie down with.
    pub fn carried(self, own: Carried) -> Carried {
        match self {
            Task::Chop => Carried::Axe,
            Task::Hoe => Carried::Hoe,
            Task::Hammer => Carried::Hammer,
            Task::Fish => Carried::Rod,
            Task::Carry => Carried::Load,
            Task::Fetch => Carried::Bucket,
            Task::Sweep => Carried::Broom,
            Task::Tend if own == Carried::Smoker => own,
            Task::Tend | Task::Sit | Task::SitLow | Task::Lie => match own {
                Carried::Purse | Carried::Bow => own,
                _ => Carried::None,
            },
            Task::Stand | Task::Talk => own,
            // Both hands are busy: with the oars, and with the book and the pencil.
            Task::Row | Task::Sketch => match own {
                Carried::Purse | Carried::Bow => own,
                _ => Carried::None,
            },
        }
    }

    /// Whether `carried` is being worked with, and so held in the hands.
    fn in_use(self, carried: Carried) -> bool {
        matches!((self, carried), (Task::Chop, Carried::Axe) | (Task::Hoe, Carried::Hoe) | (Task::Fish, Carried::Rod) | (Task::Sweep, Carried::Broom))
    }
}

/// The pose for `task` at `clock` seconds, for someone of `build`; `phase` (0 to 1) puts
/// each person at their own point of the work, so that two at it are not in step.
pub fn task_pose(task: Task, build: &Build, clock: f32, phase: f32) -> Pose {
    let tau = std::f32::consts::TAU;
    let wave = |rate: f32, k: f32| (clock * rate + (phase + k) * tau).sin();
    // How far through a stroke: up slowly, then down fast.
    let stroke = |rate: f32, high: f32, low: f32| {
        let u = (clock * rate + phase).rem_euclid(1.0);
        if u < 0.62 {
            let s = u / 0.62;
            low + (high - low) * s * s * (3.0 - 2.0 * s)
        } else {
            let s = (u - 0.62) / 0.38;
            high + (low - high) * s * s
        }
    };
    // Seated with the hips at `height` metres above the soles.
    let seated = |height: f32| m(height) / build.scale - 9.0 * CELL;
    let breath = wave(1.7, 0.0);
    match task {
        Task::Stand => Pose { arm: [breath * 0.04, -breath * 0.04], fore: [-0.1; 2], head: wave(0.45, 0.3) * 0.3, lift: breath * m(0.006), ..Default::default() },
        Task::Chop => {
            let a = stroke(0.75, -2.9, -1.05);
            Pose { leg: [-0.18, 0.18], arm: [a; 2], fore: [-0.25; 2], lean: 0.12 + 0.18 * ((a + 2.9) / 1.85), ..Default::default() }
        }
        Task::Hoe => {
            let a = stroke(0.5, -1.7, -0.55);
            Pose { leg: [-0.25, 0.15], shin: [0.15; 2], arm: [a, a + 0.15], fore: [-0.3; 2], lean: 0.3, ..Default::default() }
        }
        Task::Hammer => {
            let a = stroke(1.3, -1.95, -0.7);
            Pose { leg: [-0.12, 0.12], arm: [a, -0.75], fore: [-0.8, -0.7], lean: 0.15, ..Default::default() }
        }
        Task::Fish => Pose { arm: [-0.95, -0.85], fore: [-0.35, -0.45], lean: 0.05, jiggle: [0.0, wave(0.6, 0.0) * 0.04], ..Default::default() },
        Task::Carry => Pose { arm: [-0.75; 2], fore: [-0.95; 2], lean: -0.04, lift: breath * m(0.006), ..Default::default() },
        Task::Fetch => Pose { arm: [0.04, -0.12], fore: [0.0, -0.2], lean: 0.03, ..Default::default() },
        Task::Sweep => {
            let s = wave(0.9, 0.0);
            Pose { leg: [-0.2, 0.15], arm: [-0.7 + 0.12 * s, -0.5 - 0.12 * s], fore: [-0.35, -0.55], head: s * 0.4, lean: 0.22, ..Default::default() }
        }
        Task::Sit => {
            // Hands in the lap, working at something now and then.
            let busy = wave(1.1, 0.5).max(0.0) * 0.25;
            Pose { leg: [-1.5; 2], shin: [1.5; 2], arm: [-0.55; 2], fore: [-0.7 - busy, -0.7 + busy * 0.5], head: wave(0.3, 0.2) * 0.25, lift: seated(SEAT), ..Default::default() }
        }
        Task::SitLow => Pose { leg: [-1.5, -1.4], shin: [0.1, 0.3], arm: [0.35; 2], fore: [-0.05; 2], head: wave(0.35, 0.1) * 0.3, lean: -0.2, lift: seated(0.1), ..Default::default() },
        Task::Talk => {
            let say = wave(2.2, 0.0).max(0.0);
            Pose { arm: [-0.3 - 0.4 * say, 0.02], fore: [-0.9 - 0.35 * say, -0.15], head: wave(0.8, 0.4) * 0.12, lift: breath * m(0.006), ..Default::default() }
        }
        Task::Tend => {
            let w = wave(0.8, 0.0);
            Pose { leg: [-0.25, 0.2], shin: [0.3, 0.3], arm: [-1.0 + 0.12 * w, -0.85 - 0.1 * w], fore: [-0.55, -0.75], lean: 0.45, ..Default::default() }
        }
        // Flat, the arms along the sides, breathing; a cell up off the mattress.
        Task::Lie => Pose { arm: [0.05; 2], fore: [-0.25; 2], lift: CELL + breath * m(0.008), ..Default::default() },
        Task::Row => {
            // In time with the oars (`wave` at the rate a boat's oars swing, the phase
            // given as 0): reaching out toward the stern with the body forward as the
            // blades swing to the bow, then pulling the handles in to the chest, leaning
            // back, as they come aft.
            let s = wave(2.4, 0.0);
            Pose { leg: [-1.4; 2], shin: [1.25; 2], arm: [-0.9 - 0.4 * s; 2], fore: [-0.85 + 0.75 * s; 2], lean: 0.05 + 0.3 * s, ..Default::default() }
        }
        Task::Sketch => {
            // The book held up before the chest, the pencil hand moving over it, the head
            // still; the knees up a little, as on a stool lower than a bench.
            let w = wave(1.6, 0.0);
            Pose {
                leg: [-1.72; 2],
                shin: [1.72; 2],
                arm: [-0.75 + 0.06 * w, -0.85],
                fore: [-1.05 - 0.12 * w, -1.25],
                lean: 0.12,
                lift: seated(SEAT),
                ..Default::default()
            }
        }
    }
}

/// How long someone of `build` is from heel to crown lying down, in voxels: how far their
/// feet are from their head in bed.
pub fn lying_length(build: &Build) -> f32 {
    m(build.metres())
}

/// A stool set under someone sitting where there is no seat of their own, outside their
/// door or in a yard, with its middle at `at` (the floor beneath it), its seat [`SEAT`]
/// high. Three boxes.
pub fn draw_stool(at: DVec3, facing: f32, cam: DVec3, out: &mut Vec<ParticleInstance>) {
    let frame = Frame::new(at, cam, facing, 0.0, m(0.1));
    let top = SEAT * 10.0;
    frame.part(
        out,
        Vec3::ZERO,
        0.0,
        0.0,
        0.0,
        &[
            ([-1.6, top - 0.5, -1.6], [3.2, 0.5, 3.2], shade(WOOD, 0.92)),
            ([-1.4, 0.0, -1.4], [0.6, top - 0.5, 2.8], shade(WOOD, 0.8)),
            ([0.8, 0.0, -1.4], [0.6, top - 0.5, 2.8], shade(WOOD, 0.8)),
        ],
    );
}

/// How near a figure is drawn whole, and how far it is drawn at all, in metres. Beyond,
/// a settlement's people are only its lit windows.
pub const NEAR: f32 = 60.0;
pub const SIGHT: f32 = 200.0;

/// Draws a figure as near or as far as it is from `cam`: whole within [`NEAR`], in a
/// few boxes out to [`SIGHT`], dithering out over the last stretch before it.
pub fn draw_seen(look: &Look, feet: DVec3, facing: f32, pose: &Pose, cam: DVec3, out: &mut Vec<ParticleInstance>) {
    let d = feet.distance(cam);
    if d >= worldgen::scale::m64(SIGHT as f64) {
        return;
    }
    let from = out.len();
    if d < worldgen::scale::m64(NEAR as f64) {
        draw(look, feet, facing, pose, None, cam, out);
    } else {
        draw_far(look, feet, facing, pose, cam, out);
    }
    crate::particles::dither_since(out, from, crate::particles::dither_edge(d, worldgen::scale::m64(SIGHT as f64), worldgen::scale::m64(20.0)));
}

// Brass, and the glass of a lantern by day, when it is not lit.
const BRASS: Rgb = [150, 112, 58];
const GLASS: Rgb = [214, 200, 160];

/// Draws a lantern hung from the right hand of a figure drawn by `draw` with the same
/// `feet`, `facing` and `pose`, burning `lit` brightly (0 shuttered). Where its flame is,
/// in the world, for the light it throws.
// The player's figure is drawn at their own build now (#204), with `draw_lantern_by`;
// this stays for a figure of the wanderer's.
#[allow(dead_code)]
pub fn draw_lantern(feet: DVec3, facing: f32, pose: &Pose, lit: f32, cam: DVec3, out: &mut Vec<ParticleInstance>) -> DVec3 {
    draw_lantern_by(&Build::PLAYER, feet, facing, pose, lit, cam, out)
}

/// As [`draw_lantern`], in the hand of a figure of `build`: the player as they have
/// made themselves (#204), taller or shorter, broader or slighter.
pub fn draw_lantern_by(build: &Build, feet: DVec3, facing: f32, pose: &Pose, lit: f32, cam: DVec3, out: &mut Vec<ParticleInstance>) -> DVec3 {
    let yaw = facing - std::f32::consts::FRAC_PI_2;
    let (sy, cy) = yaw.sin_cos();
    // The hand, as `draw` puts it: down the arm from the shoulder, and on down the
    // forearm from the elbow, with the body leaning from the hips.
    let hips = Vec3::new(0.0, 9.0, 0.0);
    let (sl, cl) = (pose.lean + build.stoop).sin_cos();
    let arm = measures(build).1;
    let shoulder = Vec3::new(-(2.0 * build.girth + 0.5 * arm), 13.5, 0.0) - hips;
    let shoulder = hips + Vec3::new(shoulder.x, shoulder.y * cl - shoulder.z * sl, shoulder.y * sl + shoulder.z * cl);
    let along = |joint: Vec3, pitch: f32, down: f32| joint + Vec3::new(0.0, -down * pitch.cos(), -down * pitch.sin());
    let elbow = along(shoulder, pose.arm[0], 2.5);
    // Held a little out from the side, so it swings clear of the leg.
    let hand = along(elbow, pose.arm[0] + pose.fore[0], 3.0) + Vec3::new(-0.7, 0.0, 0.0);
    let base = (feet - cam).as_vec3() + Vec3::Y * pose.lift * build.scale;
    let cell = CELL * build.scale;
    let world = |p: Vec3| base + Vec3::new(p.x * cy - p.z * sy, p.y, p.x * sy + p.z * cy) * cell;
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
            dissolve: 0.0,
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

    /// Every box the player's figure is drawn with in a few poses, summed with weights
    /// by place in the list, so that any change to it shows.
    fn player_sum() -> (usize, f64) {
        let poses = [
            (Gait::new(0.3).pose(), Some(0.0)),
            (Gait::striding(1.1, 4.16).pose(), Some(0.0)),
            (Gait::striding(-2.0, 6.9).pose(), None),
            (Gait::gliding(0.7).pose(), None),
            (Gait::reaching(2.2, 0.6).pose(), Some(0.6)),
        ];
        let (mut n, mut sum) = (0, 0.0f64);
        for (k, (pose, furled)) in poses.iter().enumerate() {
            let mut out = Vec::new();
            let facing = 0.4 + k as f32;
            draw(&WANDERER, DVec3::new(10.5, 3.25, -7.0), facing, pose, *furled, DVec3::new(12.0, 5.0, -3.0), &mut out);
            n += out.len();
            for (i, b) in out.iter().enumerate() {
                let fields = [b.pos[0], b.pos[1], b.pos[2], b.yaw, b.size[0], b.size[1], b.size[2], b.glow, b.color[0], b.color[1], b.color[2], b.color[3], b.tilt[0], b.tilt[1], b.dissolve];
                for (j, v) in fields.iter().enumerate() {
                    sum += *v as f64 * ((i * 17 + j + 1) as f64).sqrt();
                }
            }
        }
        (n, sum)
    }

    #[test]
    fn the_player_is_drawn_as_before() {
        // As drawn before anyone else could be (21215f0): the people drawn on the same
        // body must leave the player as they were, box for box.
        let (n, sum) = player_sum();
        assert_eq!(n, 162);
        assert!((sum - -740.8407614302).abs() < 1e-9, "player: {n} boxes, sum {sum:.12e}");
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
