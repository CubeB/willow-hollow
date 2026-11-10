//! Small ravines: short, narrow cuts with rock walls in hilly or rocky ground.
//!
//! Each is 20 to 80 m long, 2 to 6 m wide and 3 to 10 m deep, shallowing and narrowing
//! to nothing at both ends, with a slight bend. They are laid out from world-space
//! cells, so every chunk agrees on where they are; whether one opens is decided from
//! the ground at its centre, and it fades out wherever it would come near a river, a
//! trail or a landmark. Their walls reuse the cliff cover, so they show the rock
//! strata, and their floors are rubble or soil.

use glam::{Vec2, vec2};

use crate::{
    noise::{hash2, perlin2, unit},
    scale::{m, m64},
};

use super::smoothstep;

/// Edge of the world-space cell each ravine is laid out from, in voxels (200 m).
pub const CELL: i32 = 500;
/// Chance that a cell holds a ravine, before the ground is checked.
const CHANCE: f32 = 0.5;
/// Farthest from its centre a ravine reaches, in voxels.
pub const REACH: f32 = m(46.0);

/// One small ravine, in continent-local voxels.
#[derive(Clone, Copy, Debug)]
pub struct Ravine {
    pub centre: Vec2,
    /// Unit vector along its length.
    pub dir: Vec2,
    pub half_len: f32,
    pub half_width: f32,
    pub depth: f32,
    salt: u32,
}

/// Where a column lies in a ravine.
#[derive(Clone, Copy, Debug)]
pub struct Spot {
    /// Distance in from the nearer wall (negative outside it).
    pub inside: f32,
    /// Depth of the cut here before any fading, in voxels.
    pub depth: f32,
}

impl Ravine {
    /// The ravine laid out from world cell `(gx, gz)`, if it has one, with its centre in
    /// world voxels. `origin` is the continent's corner in world voxels.
    pub fn in_cell(seed: u32, gx: i32, gz: i32, origin: (i64, i64)) -> Option<Ravine> {
        let h = hash2(seed ^ 0x5a41, gx, gz);
        if unit(h) >= CHANCE {
            return None;
        }
        let r = |k: u32| unit(hash2(h ^ k.wrapping_mul(0x9e37_79b9), gx, gz));
        // Kept in from the cell edges so a ravine is seen whole by the cells around it.
        let inset = REACH * 0.2;
        let span = CELL as f32 - 2.0 * inset;
        let wx = gx as i64 * CELL as i64 + (inset + span * r(1)) as i64;
        let wz = gz as i64 * CELL as i64 + (inset + span * r(2)) as i64;
        let a = r(3) * std::f32::consts::TAU;
        // Short ones are commoner than long ones.
        let half_len = m(10.0) + m(30.0) * r(4) * r(4).sqrt();
        Some(Ravine {
            centre: vec2((wx - origin.0) as f32, (wz - origin.1) as f32),
            dir: vec2(a.cos(), a.sin()),
            half_len,
            half_width: m(1.0) + m(2.0) * r(5),
            depth: m(3.0) + m(7.0) * r(6),
            salt: h,
        })
    }

    /// Whether `p` is anywhere near the ravine.
    pub fn near(&self, p: Vec2) -> bool {
        (p - self.centre).length_squared() < (self.half_len + self.half_width * 2.0 + m(4.0)).powi(2)
    }

    /// Where continent-local point `p` (world position `wx`, `wz`) lies in the ravine,
    /// if within a couple of voxels of it.
    pub fn at(&self, p: Vec2, wx: f64, wz: f64) -> Option<Spot> {
        let q = p - self.centre;
        let t = q.dot(self.dir);
        let u = t / self.half_len;
        if u.abs() >= 1.0 {
            return None;
        }
        // A gentle bend along its length.
        let bend = perlin2(self.salt ^ 0xbe0d, t as f64 / m64(28.0), 0.5) * m(3.0);
        let s = (q.perp_dot(self.dir) - bend).abs();
        let end = 1.0 - u * u;
        let rough = perlin2(self.salt ^ 0x7a1e, wx / m64(4.0), wz / m64(4.0)) * m(0.6)
            + (unit(hash2(self.salt ^ 0x7a1f, wx as i32, wz as i32)) - 0.5) * m(0.3);
        let half = self.half_width * end.sqrt() * (1.0 + 0.3 * perlin2(self.salt ^ 0x3d1, t as f64 / m64(9.0), 0.5)) + rough;
        let inside = half - s;
        if inside < -3.0 {
            return None;
        }
        // Deepest in the middle, shallowing towards the ends.
        let deep = 0.75 + 0.25 * perlin2(self.salt ^ 0xdee9, t as f64 / m64(12.0), 0.5);
        Some(Spot { inside, depth: self.depth * end.powf(0.7) * deep * smoothstep(0.0, 1.5, inside.max(0.0)).max(0.35) })
    }
}

/// Whether a ravine may open at a centre whose natural ground is `ground` with a
/// simulated slope `slope` (rise per voxel) and rock `hardness` (0 to 1).
pub fn suits(ground: f32, slope: f32, hardness: f32) -> bool {
    ground > m(14.0) && (slope > 0.2 || (hardness > 0.5 && slope > 0.08))
}
