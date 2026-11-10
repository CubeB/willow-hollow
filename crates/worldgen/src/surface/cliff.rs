//! Cliffs: rare rock faces on steep ground where the rock is hard.
//!
//! A cliff is made by remapping ground height within one height band. The band's
//! natural slope is squeezed into a single steep drop: the ground just above the drop
//! is lifted and the ground just below it lowered, each by half the face height,
//! fading back to the natural surface across an apron on either side. The mapping
//! only ever reduces the slope of the aprons, so the ground around a cliff keeps its
//! natural fall rather than being stepped into terraces. Because the face sits where
//! the ground crosses one height, it runs along the slope like a real escarpment.
//!
//! Faces are modest: mostly 4 to 14 m, and only rarely, in the hardest rock high in
//! the mountains, up to about 30 m. The bands are short, so a long steep slope rises
//! in tiers of short faces with sloping, grassy benches between them, and each face
//! comes and goes along the slope with stretches of plain steep ground between. Some
//! faces stand sheer; others lean back as steep, stepped rock.
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
/// Height of one cliff band, in voxels. Each band holds at most one cliff, so a long
/// steep slope rises in tiers about this far apart.
const PERIOD: f32 = m(34.0);
/// Tallest a face gets, in voxels.
const TALLEST: f32 = m(30.0);

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
/// `tall`, 0 to 1, is how far the rock here allows the rare tall face.
fn band(seed: u32, n: i32, wx: f64, wz: f64, tall: f32) -> Band {
    let salt = hash2(seed ^ 0xc11c, n, 1);
    let r = |k: i32| unit(hash2(salt, k, 7));
    // Mostly 4 to 14 m, more often low than high.
    let t = smoothstep(-0.5, 0.5, perlin2(salt ^ 0x4e16, wx / m64(200.0), wz / m64(200.0)));
    let height = (m(4.0) + m(10.0) * t.powf(1.5) + m(16.0) * tall).min(TALLEST);
    let mut below = 1.3 + 0.8 * r(2);
    let mut above = 1.2 + 1.0 * r(3);
    let ledge = if r(4) < 0.4 { m(1.0) + m(2.0) * r(5) } else { 0.0 };
    // A tall face would overflow the band: steepen its aprons to fit.
    let fit = ((PERIOD - ledge) / (height * 0.5 * (below + above))).min(1.0);
    below *= fit;
    above *= fit;
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
///
/// `guard` is given the natural height of the bottom of the band the face gathers
/// its drop from, and returns how strongly (0 to 1) a face may stand there. It keeps
/// faces off valley floors: below a face the ground is lowered as far as that height,
/// which beside a river would dig the valley out below the water. It is only called
/// for the few columns a face could touch.
pub fn apply(seed: u32, slope: f32, wx: f64, wz: f64, ground: &mut f32, guard: impl FnOnce(f32) -> f32) -> Part {
    // Gentle and low ground is the overwhelming majority: skip it first.
    if slope < STEEP || *ground < m(8.0) {
        return Part::None;
    }
    let hard = hard_rock(seed, wx, wz);
    let zone = smoothstep(0.1, 0.26, hard);
    if zone <= 0.0 {
        return Part::None;
    }
    // The rare tall face: only the very hardest rock, high in the mountains, in places.
    let tall = smoothstep(0.42, 0.62, hard)
        * smoothstep(0.25, 0.5, perlin2(seed ^ 0xc1a1, wx / m64(500.0), wz / m64(500.0)))
        * smoothstep(m(45.0), m(90.0), *ground);
    // The band level drifts, so the faces do not follow the contours exactly.
    let warp = perlin2(seed ^ 0xc11e, wx / m64(170.0), wz / m64(170.0)) * m(8.0);
    let g = *ground + warp;
    let n = (g / PERIOD).floor() as i32;
    let b = band(seed, n, wx, wz, tall);
    let x = g - n as f32 * PERIOD - b.at;
    let half = b.height * 0.5;
    let (lo, hi) = (-half * b.below, b.ledge + half * b.above);
    if x < lo || x > hi {
        return Part::None;
    }
    // Faces come and go along the slope, a few tens of metres long with stretches of
    // plain steep ground between, and end over a short distance rather than
    // dwindling into a long run of low steps.
    let along = perlin2(b.salt, wx / m64(75.0), wz / m64(75.0)) + 0.35 * perlin2(b.salt ^ 0x9a7, wx / m64(24.0), wz / m64(24.0));
    let patch = smoothstep(0.08, 0.22, along);
    let strength = zone.min(smoothstep(STEEP, STEEP + 0.15, slope)).min(patch);
    // Keep the foot of a face clear of the shore.
    let base = *ground - x + lo;
    let strength = strength.min(smoothstep(m(4.0), m(10.0), base));
    if strength <= 0.0 {
        return Part::None;
    }
    let strength = strength.min(guard(base));
    if strength <= 0.0 {
        return Part::None;
    }
    let ledge = if b.ledge > 0.0 {
        // The ledge widens and pinches out along the face.
        b.ledge * smoothstep(-0.1, 0.3, perlin2(b.salt ^ 0x1ed9, wx / m64(28.0), wz / m64(28.0)))
    } else {
        0.0
    };
    // Some faces stand sheer, others lean back: horizontal run per unit of height.
    let lean = ((perlin2(b.salt ^ 0x1ea0, wx / m64(45.0), wz / m64(45.0)) + 0.1) * 1.4).clamp(0.0, 0.8);
    let face = Face { x, lo, hi, height: b.height, ledge, split: b.split, lean, salt: b.salt, seed };
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
    /// How far the face leans back: its run across the slope per unit of height, 0 for
    /// sheer.
    pub lean: f32,
    pub salt: u32,
    /// World seed, which places the rock layers the shelves follow.
    pub seed: u32,
}

/// Thickness of one rock layer in a cliff, in the units of [`strata_height`]: the
/// strata the chunk fill paints on faces, which `materials::strata` bands in the
/// same steps.
const STRATA: f32 = 4.0;

/// How far the rock layers are shifted up or down at world position `(wx, wz)`, in
/// voxels, so they dip from place to place instead of lying dead level.
pub fn strata_tilt(seed: u32, wx: f64, wz: f64) -> f32 {
    perlin2(seed ^ 0xc11d, wx / m64(70.0), wz / m64(70.0)) * m(2.4)
        + perlin2(seed ^ 0xc11f, wx / m64(240.0), wz / m64(240.0)) * m(12.0)
}

/// Voxel height `y` measured in the rock's own layering, given the local `tilt`.
/// Layers are [`STRATA`] apart in these units, but the units stretch and squeeze with
/// height, so beds are of uneven thickness: thin bands, then a thick one.
pub fn strata_height(seed: u32, y: f32, tilt: f32) -> f32 {
    let s = y + tilt;
    s + 7.0 * perlin2(seed ^ 0x57c1, s as f64 / 26.0, 0.37) + 2.0 * perlin2(seed ^ 0x57c2, s as f64 / 11.0, 0.71)
}

impl Face {
    /// Raises or lowers `ground` to cut the face, scaled by `strength` (0 to 1), on
    /// ground whose natural slope is `slope`.
    ///
    /// The ground is lowered by up to half the face height below it and raised by up to
    /// half above it, fading back to the natural surface at the edges of the band, so
    /// the aprons keep a reduced natural slope.
    pub fn cut(&self, strength: f32, slope: f32, wx: f64, wz: f64, ground: &mut f32) -> Part {
        let (lo, hi) = (self.lo, self.hi);
        let ridge = |salt: u32, scale: f32| 1.0 - perlin2(self.salt ^ salt, wx / scale as f64, wz / scale as f64).abs();
        // The top edge rises and falls along the face: taller buttresses, lower
        // stretches, and the odd notch where water has cut down from the lip. A face is
        // only a few voxels across, so any pattern in plan view reads as vertical on it.
        let along = (1.0 + 0.34 * perlin2(self.salt ^ 0x7e11, wx / m64(30.0), wz / m64(30.0))).clamp(0.5, 1.25);
        let notch = smoothstep(0.88, 0.97, ridge(0x7e12, m(17.0)));
        let h = self.height * strength * along * (1.0 - 0.45 * notch);
        let ledge = self.ledge * strength;
        // A ragged face: buttresses and bays a few metres across. Moving the height at
        // which the face stands moves it sideways; `rough` is how far, in voxels across
        // the slope.
        let mut rough = perlin2(self.salt ^ 0xb077, wx / m64(14.0), wz / m64(14.0)) * m(4.5)
            + perlin2(self.salt ^ 0xb078, wx / m64(4.5), wz / m64(4.5)) * m(1.2);
        // Now and then a blocky pillar standing out from the face or a recess, where the
        // rock has split along its joints.
        let joint = hash2(self.salt ^ 0x7e14, (wx / m64(2.0)).floor() as i32, (wz / m64(2.0)).floor() as i32);
        rough += match unit(joint) {
            u if u < 0.07 => m(1.2),
            u if u < 0.13 => -m(0.8),
            _ => 0.0,
        };
        // Water erosion: gullies behind each notch, and here and there a runnel set
        // back into the rock, only where the rock is wet.
        let wet = smoothstep(0.1, 0.4, perlin2(self.salt ^ 0x7e18, wx / m64(35.0), wz / m64(35.0)));
        let runnel = smoothstep(0.9, 0.98, ridge(0x7e13, m(5.0))) * wet;
        rough -= notch * m(2.4) + runnel * m(1.0);
        let x = self.x + rough * slope * strength;
        let top = h * 0.5 - 0.8 * ledge;
        // The sheer profile: fallen stone leaning against the foot at about 35 degrees,
        // the face, any ledge, and the lifted ground above easing back to natural.
        let profile = |x: f32| {
            if x < 0.0 {
                let t = ((x - lo) / -lo).max(0.0);
                let talus = (h * 0.22 + x / slope * 0.7).max(0.0);
                -h * 0.5 * t + talus
            } else if x < ledge {
                -h * 0.5 + h * self.split - 0.8 * x
            } else {
                top * (1.0 - ((x - ledge) / (hi - ledge)).min(1.0))
            }
        };
        // A leaning face spreads its drop over a run across the slope, as steep stepped
        // rock: `w` is half that run in natural height.
        let w = (h * self.lean * slope * 0.5).min(-lo * 0.7).min((hi - ledge) * 0.7);
        let w = if w > m(0.3) { w } else { 0.0 };
        let leaning = x.abs() < w;
        let mut shelf = false;
        let lift = if leaning {
            let (a, b) = (-w + profile(-w), w + profile(w));
            a + (b - a) * (x + w) / (2.0 * w) - x
        } else if x < 0.0 {
            let mut lift = profile(x);
            // Shelves: runs of rock a voxel or two deep standing out from the face, their
            // tops on a bedding plane, so the face is broken by ledges along its strata.
            let across = x / slope;
            if across > -m(0.9) && h > m(4.0) && notch < 0.3 {
                let run = perlin2(self.salt ^ 0x7e15, wx / m64(9.0), wz / m64(9.0));
                if run > 0.15 {
                    let up = 0.55 + 0.25 * perlin2(self.salt ^ 0x7e16, wx / m64(24.0), wz / m64(24.0));
                    // Natural height of the face here, which stands half way up it.
                    let y = *ground - x + h * (up - 0.5);
                    let s = strata_height(self.seed, y, strata_tilt(self.seed, wx, wz));
                    let bed = y - s.rem_euclid(STRATA) - 0.5;
                    if bed - *ground > lift {
                        lift = bed - *ground;
                        shelf = true;
                    }
                }
            }
            lift
        } else {
            profile(x)
        };
        *ground += lift;

        // Distance behind the lip across the slope, in voxels.
        let behind = (x - w.max(ledge)) / slope;
        if h < m(1.6) {
            Part::None
        } else if shelf || leaning || (x >= 0.0 && (x < ledge || behind < 1.2)) {
            Part::Face(h)
        } else if x < 0.0 && x > lo * 0.6 {
            // Most stone lies close under the face, on and below the talus, with blocks
            // fallen from it lying about.
            let near = 1.0 - x / (lo * 0.6);
            let block = hash2(self.salt ^ 0x7e17, (wx / 3.0).floor() as i32, (wz / 3.0).floor() as i32);
            if unit(block) < 0.1 * near && h > m(3.0) {
                *ground += m(0.5) + m(0.9) * unit(block.rotate_left(11));
            }
            Part::Foot((0.4 + 0.6 * near) * smoothstep(m(2.0), m(6.0), h))
        } else {
            Part::None
        }
    }
}

/// How hard the rock at world position `(wx, wz)` is, unbounded: cliffs begin at about
/// 0.1 and the hardest rock is above about 0.5.
fn hard_rock(seed: u32, wx: f64, wz: f64) -> f32 {
    perlin2(seed ^ 0xc110, wx / m64(420.0), wz / m64(420.0)) + 0.4 * perlin2(seed ^ 0xc112, wx / m64(150.0), wz / m64(150.0))
}

/// How much the rock at world position `(wx, wz)` stands as cliffs, 0 to 1: hard rock
/// outcrops are a minority of the land, a few hundred metres across.
pub fn hardness(seed: u32, wx: f64, wz: f64) -> f32 {
    smoothstep(0.1, 0.26, hard_rock(seed, wx, wz))
}
