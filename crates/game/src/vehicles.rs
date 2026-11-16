//! Boats tied up at landings, and how boats and the glider are drawn.
//!
//! Boats are not part of the voxel world, since they move. Each landing starts with one
//! moored; a boat you have sailed stays wherever you left it for the rest of the session.

use std::{collections::HashSet, sync::Arc};

use glam::{DVec3, Vec3};
use worldgen::{
    macro_layout::MACRO_SIZE,
    scale::{OLD_VOXEL, m64},
    world::GameWorld,
};

use crate::particles::ParticleInstance;

/// Farthest away boats are looked for and drawn.
const VIEW: f64 = m64(360.0);
/// How often the moorings around the player are looked up, in seconds.
const REFRESH: f32 = 1.0;

/// One moored boat's identity: its continent and which landing.
type MooringKey = (i32, i32, u32);

#[derive(Clone, Copy, Debug)]
pub struct Boat {
    /// On the water surface, at the middle of the hull.
    pub pos: DVec3,
    /// Direction the bow points, radians.
    pub heading: f32,
    mooring: Option<MooringKey>,
}

pub struct Fleet {
    world: Arc<GameWorld>,
    /// Moorings whose boat has been taken.
    taken: HashSet<MooringKey>,
    /// Boats left wherever the player went ashore.
    drifting: Vec<Boat>,
    /// Boats currently near the player.
    near: Vec<Boat>,
    timer: f32,
}

impl Fleet {
    pub fn new(world: Arc<GameWorld>) -> Self {
        Self {
            world,
            taken: HashSet::new(),
            drifting: Vec::new(),
            near: Vec::new(),
            timer: 0.0,
        }
    }

    pub fn update(&mut self, pos: DVec3, dt: f32) {
        self.timer -= dt;
        if self.timer > 0.0 {
            return;
        }
        self.timer = REFRESH;
        self.refresh(pos);
    }

    /// Rebuilds the list of boats near `pos`.
    pub fn refresh(&mut self, pos: DVec3) {
        self.near.clear();
        let cell = |v: f64| (v / MACRO_SIZE as f64).floor() as i32;
        let (cx, cz) = (cell(pos.x), cell(pos.z));
        for mz in cz - 1..=cz + 1 {
            for mx in cx - 1..=cx + 1 {
                let Some(c) = self.world.continent_if_ready(mx, mz) else { continue };
                let origin = DVec3::new((mx * MACRO_SIZE) as f64, 0.0, (mz * MACRO_SIZE) as f64);
                for (i, site) in c.trails.sites.iter().enumerate() {
                    let Some(m) = site.mooring else { continue };
                    let key = (mx, mz, i as u32);
                    let boat_pos = origin + m.pos.as_dvec3();
                    if !self.taken.contains(&key) && boat_pos.distance_squared(pos) < VIEW * VIEW {
                        self.near.push(Boat {
                            pos: boat_pos,
                            heading: m.heading,
                            mooring: Some(key),
                        });
                    }
                }
            }
        }
        self.near.extend(self.drifting.iter().filter(|b| b.pos.distance_squared(pos) < VIEW * VIEW));
    }

    /// The nearest boat within `reach`, by index into the boats near the player.
    pub fn nearest(&self, pos: DVec3, reach: f64) -> Option<usize> {
        self.near
            .iter()
            .enumerate()
            .map(|(i, b)| (i, b.pos.distance_squared(pos)))
            .filter(|&(_, d)| d < reach * reach)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    /// Takes a boat out of the world so the player can sail it.
    pub fn take(&mut self, index: usize) -> Boat {
        let boat = self.near.swap_remove(index);
        match boat.mooring {
            Some(key) => {
                self.taken.insert(key);
            }
            None => self.drifting.retain(|b| b.pos != boat.pos),
        }
        boat
    }

    /// Leaves a boat afloat.
    pub fn leave(&mut self, pos: DVec3, heading: f32) {
        let boat = Boat {
            pos,
            heading,
            mooring: None,
        };
        self.drifting.push(boat);
        self.near.push(boat);
    }

    pub fn boats(&self) -> &[Boat] {
        &self.near
    }
}

/// `(centre, size, colour)` in the model's own frame: +X forward, +Y up.
type Part = ([f32; 3], [f32; 3], [f32; 3]);

const HULL: [f32; 3] = [0.21, 0.13, 0.07];
const HULL_LIGHT: [f32; 3] = [0.42, 0.28, 0.15];
const MAST: [f32; 3] = [0.20, 0.14, 0.09];
const SAIL: [f32; 3] = [0.82, 0.76, 0.62];

const BOAT: [Part; 11] = [
    // Floor and sides.
    ([0.0, 0.05, 0.0], [4.6, 0.3, 1.5], HULL),
    ([0.0, 0.45, 0.85], [4.6, 0.6, 0.2], HULL_LIGHT),
    ([0.0, 0.45, -0.85], [4.6, 0.6, 0.2], HULL_LIGHT),
    ([-2.3, 0.45, 0.0], [0.2, 0.6, 1.9], HULL_LIGHT),
    // A blunt bow in two steps.
    ([2.65, 0.3, 0.0], [0.9, 0.5, 1.1], HULL),
    ([3.25, 0.5, 0.0], [0.5, 0.5, 0.6], HULL_LIGHT),
    // Thwarts.
    ([-1.0, 0.4, 0.0], [0.5, 0.12, 1.5], HULL_LIGHT),
    ([1.1, 0.4, 0.0], [0.5, 0.12, 1.5], HULL_LIGHT),
    // Mast, yard and sail.
    ([0.4, 2.6, 0.0], [0.16, 4.6, 0.16], MAST),
    ([0.4, 4.5, 0.0], [0.12, 0.12, 3.2], MAST),
    ([0.32, 3.0, 0.0], [0.05, 2.8, 2.9], SAIL),
];

const CANOPY: [f32; 3] = [0.72, 0.30, 0.24];
const CANOPY_DEEP: [f32; 3] = [0.56, 0.21, 0.18];
const CANOPY_LIGHT: [f32; 3] = [0.85, 0.74, 0.58];
const FRAME: [f32; 3] = [0.20, 0.15, 0.11];
const TUBE: [f32; 3] = [0.25, 0.23, 0.22];
const WIRE: [f32; 3] = [0.55, 0.54, 0.52];

/// The wing's shape, in the model's frame (old voxels, from the eye): a swept delta, its
/// nose ahead of the hands and its tips back and a little up, the way a hang glider's
/// is. It is laid out in strips across the span, each a rib's width: few and thick,
/// so its blocks are of a size with the plants' and the props' (#197).
const HALF_SPAN: f32 = 3.8;
const STRIPS: usize = 6;
/// How thick the cloth is.
const CLOTH: f32 = 0.08;
/// Where the wing hangs from, over the hands: it banks and pitches about this, so the
/// frame the hands hold stays where the hands are.
const HANG: [f32; 3] = [0.4, 1.0, 0.0];
/// The leading and trailing edges `z` out from the keel.
fn leading(z: f32) -> f32 {
    2.0 - 0.55 * z
}
fn trailing(z: f32) -> f32 {
    -1.35 + 0.18 * z
}

fn push_parts(parts: &[Part], origin: DVec3, heading: f32, cam: DVec3, out: &mut Vec<ParticleInstance>) {
    let (s, c) = heading.sin_cos();
    let base = (origin - cam).as_vec3();
    for (center, size, color) in parts {
        let p = Vec3::from(*center) * OLD_VOXEL;
        let turned = Vec3::new(p.x * c - p.z * s, p.y, p.x * s + p.z * c);
        out.push(ParticleInstance {
            pos: (base + turned).into(),
            yaw: heading,
            size: (Vec3::from(*size) * OLD_VOXEL).into(),
            glow: 0.0,
            color: [color[0], color[1], color[2], 1.0],
            tilt: [0.0; 2],
        });
    }
}

/// Draws a boat with its hull rocking gently on the water.
pub fn draw_boat(pos: DVec3, heading: f32, time: f32, cam: DVec3, out: &mut Vec<ParticleInstance>) {
    let bob = (time * 1.1 + pos.x as f32 * 0.3).sin() * 0.05 * OLD_VOXEL;
    push_parts(&BOAT, pos + DVec3::new(0.0, bob as f64, 0.0), heading, cam, out);
}

/// How the wing sits in the air: banked into a turn and pitched with the climb or dive,
/// both in radians, and how hard the air is going past it (0 still, 1 cruising), which
/// is what sets its trailing edge fluttering.
#[derive(Clone, Copy, Default)]
pub struct Flight {
    /// Positive lowers the right wing (the figure's right, model +z).
    pub bank: f32,
    /// Positive raises the nose.
    pub pitch: f32,
    pub wind: f32,
    /// Seconds, for the flutter.
    pub time: f32,
}

impl Flight {
    /// The wing's attitude for a glider banked `bank` (as the player leans) and rising at
    /// `rise` metres a second: nose a touch up in level flight, down in a dive.
    pub fn of(bank: f32, rise: f32, time: f32) -> Self {
        Self { bank, pitch: (0.06 + rise * 0.035).clamp(-0.3, 0.16), wind: 1.0, time }
    }
}

/// Rotates `v` about the model's x (forward) axis by `bank`, after about its z (across)
/// axis by `pitch`: the particle shader's `turn` without its yaw.
fn attitude(v: Vec3, bank: f32, pitch: f32) -> Vec3 {
    let (sp, cp) = pitch.sin_cos();
    let a = Vec3::new(v.x * cp - v.y * sp, v.x * sp + v.y * cp, v.z);
    let (sb, cb) = bank.sin_cos();
    Vec3::new(a.x, a.y * cb - a.z * sb, a.y * sb + a.z * cb)
}

/// Pieces of the glider, in the model's frame, gathered before they are turned to its
/// heading: each box carries its own attitude, so a tube can run any way.
struct Kit {
    parts: Vec<([f32; 3], [f32; 3], [f32; 3], [f32; 2])>,
}

impl Kit {
    fn new() -> Self {
        Self { parts: Vec::with_capacity(96) }
    }

    /// A box with its centre at `at`, square to the frame turned by `tilt`.
    fn bx(&mut self, at: Vec3, size: [f32; 3], color: [f32; 3], tilt: [f32; 2]) {
        self.parts.push((at.into(), size, color, tilt));
    }

    /// A tube from `a` to `b`, `thick` across: one box, long along its own y, tipped so
    /// that y runs from `a` to `b` (see the shader's `turn`, which rolls about z before
    /// it pitches about x).
    fn tube(&mut self, a: Vec3, b: Vec3, thick: f32, color: [f32; 3]) {
        let d = b - a;
        let len = d.length();
        if len < 1e-4 {
            return;
        }
        let u = d / len;
        let roll = (-u.x).clamp(-1.0, 1.0).asin();
        let pitch = u.z.atan2(u.y);
        self.bx((a + b) * 0.5, [thick, len, thick], color, [pitch, roll]);
    }

    fn push(&self, origin: DVec3, heading: f32, cam: DVec3, out: &mut Vec<ParticleInstance>) {
        let (s, c) = heading.sin_cos();
        let base = (origin - cam).as_vec3();
        for (center, size, color, tilt) in &self.parts {
            let p = Vec3::from(*center) * OLD_VOXEL;
            let turned = Vec3::new(p.x * c - p.z * s, p.y, p.x * s + p.z * c);
            out.push(ParticleInstance {
                pos: (base + turned).into(),
                yaw: heading,
                size: (Vec3::from(*size) * OLD_VOXEL).into(),
                glow: 0.0,
                color: [color[0], color[1], color[2], 1.0],
                tilt: *tilt,
            });
        }
    }
}

/// Draws the wing above someone gliding, with feet at `pos`: `open` of the way unfolded,
/// from a rod lying fore and aft along its keel (where the figure swings it up to from
/// its back, see character::draw) to its full span, with the frame the hands hold and
/// the wires under it that brace it.
///
/// Unfolding, the wing sweeps out from the keel and lengthens at once, overshooting its
/// span a little and settling, as cloth does when it fills. In the air it banks into
/// turns and noses down into dives about where it hangs, the frame under it staying in
/// the hands, and the back of the sail ripples in the wind.
pub fn draw_glider(pos: DVec3, heading: f32, open: f32, flight: Flight, cam: DVec3, out: &mut Vec<ParticleInstance>) {
    let t = open.clamp(0.0, 1.0);
    let smooth = t * t * (3.0 - 2.0 * t);
    // Across, a spring settling: past full span by a few percent, then back.
    let k = t - 1.0;
    let span = (1.0 + 2.2 * k * k * k + 1.2 * k * k).max(0.04);
    // Fore and aft, from the rod's length to the keel's.
    let chord = 0.42 + 0.58 * smooth;
    let hang = Vec3::from(HANG);
    // The attitude comes on as the wing fills: a furled rod does not bank.
    let (bank, pitch) = (flight.bank * smooth, flight.pitch * smooth);
    let fold = |x: f32| hang.x + (x - hang.x) * chord;
    let place = |p: Vec3| hang + attitude(p - hang, bank, pitch);
    let tilt = [bank, pitch];
    let mut kit = Kit::new();

    // Bobbing a little on the air it rides.
    let bob = (flight.time * 1.3).sin() * 0.025 * flight.wind * smooth;
    let width = HALF_SPAN / STRIPS as f32;
    for side in [-1.0f32, 1.0] {
        for i in 0..STRIPS {
            let z = (i as f32 + 0.5) * width;
            let out_along = z / HALF_SPAN;
            let (le, te) = (fold(leading(z)), fold(trailing(z)));
            // Dihedral: the tips ride a little higher than the keel.
            let y = hang.y + 0.05 * z + bob;
            let at_z = side * z * span;
            let across = (width * span * 1.04).max(0.05);
            // Bands of cloth from the nose back: deep red under the leading edge, red,
            // a pale stripe, red, and a pale trailing edge that flutters.
            let bands: [(f32, f32, [f32; 3]); 5] = [
                (0.0, 0.12, CANOPY_DEEP),
                (0.12, 0.42, CANOPY),
                (0.42, 0.55, CANOPY_LIGHT),
                (0.55, 0.88, CANOPY),
                (0.88, 1.0, CANOPY_LIGHT),
            ];
            for (b, (from, to, color)) in bands.into_iter().enumerate() {
                let (front, back) = (le + (te - le) * from, le + (te - le) * to);
                let aft = (b as f32 - 1.0).max(0.0) / 4.0;
                // The farther back and the farther out, the more the cloth moves.
                let flutter = (flight.time * 11.0 + z * 2.1 + side).sin() * 0.03 * aft * aft * (0.3 + out_along) * flight.wind * smooth;
                // The sail billows up between the tubes, most at the middle of the chord.
                let billow = 0.05 * (std::f32::consts::PI * (from + to) * 0.5).sin() * (1.0 - out_along * 0.6) * smooth;
                let mid = Vec3::new((front + back) * 0.5, y + billow + flutter, at_z);
                kit.bx(place(mid), [(front - back).abs() + 0.02, CLOTH, across], color, tilt);
            }
        }
        // Tip: a short upturned batten closing the wing.
        let tip = HALF_SPAN * span;
        let (le, te) = (fold(leading(HALF_SPAN)), fold(trailing(HALF_SPAN)));
        kit.bx(place(Vec3::new((le + te) * 0.5, hang.y + 0.05 * HALF_SPAN + 0.06 + bob, side * tip)), [(le - te).abs() + 0.06, 0.12, 0.08], CANOPY_DEEP, tilt);
        // The leading-edge tube, swept back in one piece from the nose to the tip.
        let nose_at = Vec3::new(fold(leading(0.0)), hang.y + 0.05 + bob, 0.0);
        let tip_at = Vec3::new(le + 0.02, hang.y + 0.05 * HALF_SPAN + 0.05 + bob, side * tip);
        kit.tube(place(nose_at), place(tip_at), 0.11, TUBE);
    }
    // The keel under the cloth, nose to tail, and a cap on the nose.
    let (nose, tail) = (fold(leading(0.0) + 0.1), fold(trailing(0.0) - 0.1));
    kit.bx(place(Vec3::new((nose + tail) * 0.5, hang.y - 0.03 + bob, 0.0)), [nose - tail, 0.1, 0.11], TUBE, tilt);
    kit.bx(place(Vec3::new(nose, hang.y + 0.02 + bob, 0.0)), [0.22, 0.13, 0.26], FRAME, tilt);

    if smooth > 0.3 {
        // No king post over the wing, nor wires from it: thin lines across the top of
        // a wing made of blocks looked out of place (#197).
        let mid_z = HALF_SPAN * 0.62;
        // The control frame: two downtubes from the hang point to the ends of the bar
        // the hands hold (character::grip). It does not bank with the wing, so the bar
        // stays in the hands as the wing tips over it.
        let (ahead, up) = crate::character::grip();
        let (x, y) = (ahead / OLD_VOXEL, (up - crate::player::EYE_HEIGHT as f32) / OLD_VOXEL);
        let apex = place(hang);
        let half_bar = 0.55;
        kit.bx(Vec3::new(x, y, 0.0), [0.07, 0.07, half_bar * 2.0 + 0.08], FRAME, [0.0; 2]);
        for side in [-1.0f32, 1.0] {
            let end = Vec3::new(x, y, side * half_bar);
            kit.tube(apex, end, 0.055, TUBE);
            // Side wires from the bar's corners out to the leading edges.
            kit.tube(end, place(Vec3::new(fold(leading(mid_z)), hang.y + 0.05 * mid_z, side * mid_z * span)), 0.022, WIRE);
        }
    }
    let eye = pos + DVec3::new(0.0, crate::player::EYE_HEIGHT, 0.0);
    kit.push(eye, heading, cam, out);
}
