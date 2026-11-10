//! Cliffs: rare, sheer rock faces on steep ground where the rock is hard.
//!
//! A cliff is made by remapping ground height within one height band. The band's
//! natural slope is squeezed into a single vertical drop: the ground just above the
//! drop is lifted and the ground just below it lowered, each by half the face height,
//! fading back to the natural surface across an apron on either side. The mapping
//! only ever reduces the slope of the aprons, so the ground around a cliff keeps its
//! natural fall rather than being stepped into terraces, and the lip is crisp. Because
//! the face sits where the ground crosses one height, it runs along the slope like a
//! real escarpment; the band level is warped and the face height fades in and out with
//! noise a hundred metres or so across, so a face starts and ends instead of ringing
//! the hill.
//!
//! Which ground gets cliffs: only outcrop zones a few hundred metres across where the
//! rock is hard, only where the simulated slope is steep, and only in some height bands.

use crate::{
    noise::{hash2, perlin2, unit},
    scale::{m, m64},
};

use super::smoothstep;

/// What part of a cliff a column is.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Part {
    None,
    /// The top of a face (its exposed side is the face) or a ledge on it.
    Face(f32),
    /// The ground at its foot, with the chance (0 to 1) that it is loose stone.
    Foot(f32),
}

/// Simulated slope, in voxels of rise per voxel, at which cliffs begin to form.
const STEEP: f32 = 0.55;
/// Height of one cliff band, in voxels. Each band holds at most one cliff.
const PERIOD: f32 = m(64.0);

/// One band's cliff, in voxels relative to the band base.
struct Band {
    /// Height at which the face stands.
    at: f32,
    /// Full face height.
    height: f32,
    /// Fraction of the face below the ledge, if it has one.
    split: f32,
    /// Natural height range given to the ledge at its widest.
    ledge: f32,
    /// Apron below and above the face, as multiples of half the face height.
    below: f32,
    above: f32,
    /// Noise seed for where along the slope this band has a face.
    salt: u32,
}

/// The cliff, if any, of height band `n` near world position `(wx, wz)`. Face height
/// changes smoothly from place to place, so one band holds both low and tall faces.
fn band(seed: u32, n: i32, wx: f64, wz: f64) -> Band {
    let salt = hash2(seed ^ 0xc11c, n, 1);
    let r = |k: i32| unit(hash2(salt, k, 7));
    // Mostly 6 to 25 m, more often low than high.
    let t = smoothstep(-0.45, 0.45, perlin2(salt ^ 0x4e16, wx / m64(260.0), wz / m64(260.0)));
    let height = m(6.0) + m(19.0) * t.powf(1.3);
    let below = 1.4 + 0.9 * r(2);
    let above = 1.3 + 1.1 * r(3);
    let ledge = if r(4) < 0.45 { m(1.2) + m(2.4) * r(5) } else { 0.0 };
    let span = height * 0.5 * (below + above) + ledge;
    let slack = (PERIOD - span).max(0.0);
    Band {
        at: height * 0.5 * below + slack * r(6),
        height,
        split: 0.3 + 0.4 * r(7),
        ledge,
        below,
        above,
        salt,
    }
}

/// Applies any cliff to a column whose natural ground is `ground` (voxels) and whose
/// simulated slope is `slope` (voxels per voxel), changing `ground` in place.
pub fn apply(seed: u32, slope: f32, wx: f64, wz: f64, ground: &mut f32) -> Part {
    // Gentle and low ground is the overwhelming majority: skip it first.
    if slope < STEEP || *ground < m(8.0) {
        return Part::None;
    }
    // Hard rock outcrops: a minority of the land, a few hundred metres across.
    let hard = perlin2(seed ^ 0xc110, wx / m64(420.0), wz / m64(420.0))
        + 0.4 * perlin2(seed ^ 0xc112, wx / m64(150.0), wz / m64(150.0));
    let zone = smoothstep(0.1, 0.26, hard);
    if zone <= 0.0 {
        return Part::None;
    }
    // The band level drifts, so the faces do not follow the contours exactly.
    let warp = perlin2(seed ^ 0xc11e, wx / m64(170.0), wz / m64(170.0)) * m(10.0);
    let g = *ground + warp;
    let n = (g / PERIOD).floor() as i32;
    let b = band(seed, n, wx, wz);
    let x = g - n as f32 * PERIOD - b.at;
    let half = b.height * 0.5;
    let (lo, hi) = (-half * b.below, b.ledge + half * b.above);
    if x < lo || x > hi {
        return Part::None;
    }
    // Faces come and go along the slope, and end over a short distance rather than
    // dwindling into a long run of low steps.
    let patch = smoothstep(-0.04, 0.14, perlin2(b.salt, wx / m64(110.0), wz / m64(110.0)));
    let strength = zone.min(smoothstep(STEEP, STEEP + 0.15, slope)).min(patch);
    // Keep the foot of a face clear of the shore.
    let base = *ground - x + lo;
    let strength = strength.min(smoothstep(m(4.0), m(10.0), base));
    if strength <= 0.0 {
        return Part::None;
    }
    let ledge = if b.ledge > 0.0 {
        // The ledge widens and pinches out along the face.
        b.ledge * smoothstep(-0.1, 0.3, perlin2(b.salt ^ 0x1ed9, wx / m64(28.0), wz / m64(28.0)))
    } else {
        0.0
    };
    let face = Face { x, lo, hi, height: b.height, ledge, split: b.split, salt: b.salt };
    face.cut(strength, slope, wx, wz, ground)
}

/// One cliff face crossing a column's height band, in voxels.
pub struct Face {
    /// Natural height of the column above the foot of the face.
    pub x: f32,
    /// Bottom and top of the band the face gathers its drop from, relative to the face.
    pub lo: f32,
    pub hi: f32,
    /// Full height of the face.
    pub height: f32,
    /// Natural height range given to a ledge part-way up, or 0 for none.
    pub ledge: f32,
    /// Fraction of the face below the ledge.
    pub split: f32,
    pub salt: u32,
}

impl Face {
    /// Raises or lowers `ground` to cut the face, scaled by `strength` (0 to 1), on
    /// ground whose natural slope is `slope`.
    ///
    /// The ground is lowered by up to half the face height below it and raised by up to
    /// half above it, fading back to the natural surface at the edges of the band, so
    /// the aprons keep a reduced natural slope and the lip is crisp.
    pub fn cut(&self, strength: f32, slope: f32, wx: f64, wz: f64, ground: &mut f32) -> Part {
        let (lo, hi) = (self.lo, self.hi);
        let h = self.height * strength;
        let ledge = self.ledge * strength;
        // A ragged face: buttresses and bays a few metres across, and single columns
        // standing proud. Moving the height at which the face stands moves it sideways;
        // `rough` is how far, in voxels across the slope.
        let rough = perlin2(self.salt ^ 0xb077, wx / m64(14.0), wz / m64(14.0)) * m(4.5)
            + perlin2(self.salt ^ 0xb078, wx / m64(4.5), wz / m64(4.5)) * m(1.5)
            + (unit(hash2(self.salt, wx as i32, wz as i32)) - 0.5) * m(0.6);
        let x = self.x + rough * slope * strength;
        // Distance from the face across the slope, in voxels: about the natural height
        // difference divided by the slope.
        let across = x / slope;
        let lift = if x < 0.0 {
            let t = ((x - lo) / -lo).max(0.0);
            // Fallen stone leans against the bottom of the face at about 35 degrees.
            let talus = (h * 0.22 + across * 0.7).max(0.0);
            -h * 0.5 * t + talus
        } else if x < ledge {
            -h * 0.5 + h * self.split - 0.8 * x
        } else {
            let top = h * 0.5 - 0.8 * ledge;
            top * (1.0 - ((x - ledge) / (hi - ledge)).min(1.0))
        };
        *ground += lift;

        if h < m(1.6) {
            Part::None
        } else if x >= 0.0 && (x < ledge || across < 2.5 || (x - ledge) / slope < 2.5) {
            Part::Face(h)
        } else if x < 0.0 && x > lo * 0.6 {
            // Most stone lies close under the face, on and below the talus.
            let near = 1.0 - x / (lo * 0.6);
            Part::Foot((0.4 + 0.6 * near) * smoothstep(m(2.0), m(6.0), h))
        } else {
            Part::None
        }
    }
}
