//! Sea cliffs: where high, steep ground meets the sea, a sheer face standing on a
//! wave-cut rock shelf at about sea level, with fallen blocks at its foot and the odd
//! sea stack out on the shelf.
//!
//! Like inland cliffs (see `cliff`) the face is made by remapping natural ground height
//! within one band: ground below the height where the face stands is cut down to the
//! shelf, ground above it is lifted to the clifftop, fading back to the natural surface
//! at the top of the band and out over the sea floor below it. The face therefore runs
//! along the coast wherever the ground crosses that height.

use crate::{
    noise::{hash2, perlin2, unit},
    scale::{m, m64},
};

use super::{cliff, coast::Shore, smoothstep};

/// What part of a sea cliff a column is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    None,
    /// The top of the face (its seaward side is the face), or a sea stack.
    Face,
    /// Fallen blocks along the foot of the face.
    Foot,
    /// The wave-cut platform.
    Shelf,
}

/// Applies any sea cliff to a coastal column whose natural ground is `ground` (voxels)
/// and whose simulated slope is `slope` (voxels per voxel), changing `ground` in place.
pub fn apply(seed: u32, shore: &Shore, slope: f32, wx: f64, wz: f64, ground: &mut f32) -> Part {
    let x0 = *ground;
    if slope < 0.28 || shore.hinter < m(6.0) || x0 < -m(6.0) {
        return Part::None;
    }
    let n = |salt: u32, metres: f64| perlin2(seed ^ salt, wx / m64(metres), wz / m64(metres));
    // Steep, high coast, mostly where the rock is hard or the sea hits hardest, in
    // stretches that come and go along the shore, never at a river mouth.
    let steep = smoothstep(0.28, 0.5, slope);
    let tall = smoothstep(m(6.0), m(13.0), shore.hinter);
    let rock = smoothstep(0.05, 0.45, cliff::hardness(seed, wx, wz) * 0.6 + shore.exposure * 0.7 + n(0x5eac, 260.0) * 0.35);
    let strength = steep * tall * rock * (1.0 - shore.mouth) * smoothstep(0.02, 0.15, shore.seaness);
    if strength <= 0.0 {
        return Part::None;
    }

    // Natural height at which the face stands, the shelf's height at its foot, the face
    // height (most of the way up to the highest ground nearby) and the clifftop apron.
    let foot = m(1.4) + m(1.2) * n(0x5ec1, 90.0);
    let shelf = m(0.15) + m(0.45) * n(0x5ec2, 40.0);
    let height = ((shore.hinter - foot) * (0.5 + 0.2 * n(0x5ec3, 150.0))).clamp(0.0, m(22.0));
    let apron = height * (1.25 + 0.4 * n(0x5ec4, 120.0));
    // A ragged face: moving the height the face stands at moves it along the slope.
    let rough = n(0x5ec5, 12.0) * m(4.0) + n(0x5ec6, 4.0) * m(1.2) + (unit(hash2(seed ^ 0x5ec7, wx as i32, wz as i32)) - 0.5) * m(0.5);
    let x = x0 + rough * slope.min(1.5);

    // Seaward edge of the shelf, as natural height, and the blend back to the sea floor.
    const LOW: f32 = -m(2.0);
    const BLEND: f32 = m(2.0);
    let (target, landward) = if x >= foot {
        // Lifted to the clifftop, easing back to the natural surface at the top of the
        // band. Measured from `x` so the face's raggedness carries up, and continuous
        // with the untouched ground beyond.
        let lift = if x < foot + apron { shelf + height + (x - foot) / apron * (foot + apron - shelf - height) - x } else { 0.0 };
        (x0 + lift, true)
    } else {
        // Cut down to the shelf, which dips gently seaward and blends into the sea floor.
        let top = shelf - (foot - x) * 0.06;
        let t = smoothstep(LOW - BLEND, LOW, x);
        (x0 + (top - x0) * t, false)
    };
    *ground = x0 + (target - x0) * strength;
    let face = height * strength;
    if face < m(2.0) {
        return Part::None;
    }
    if landward {
        let across = (x - foot) / slope;
        return if across < 2.0 && *ground >= m(5.0) { Part::Face } else { Part::None };
    }
    // Sea stacks: the odd pillar left standing out on the shelf.
    if x < foot - slope * m(3.0) && x > LOW && shore.exposure > 0.25 {
        let v = n(0x57ac, 11.0) + 0.4 * n(0x57ad, 3.5);
        if v > 0.6 {
            let stack = shelf + face * 0.8 * smoothstep(0.6, 0.66, v);
            if stack >= m(5.0) {
                *ground = ground.max(stack);
                return Part::Face;
            }
        }
    }
    if x > foot - slope * m(2.0) { Part::Foot } else { Part::Shelf }
}
