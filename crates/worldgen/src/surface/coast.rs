//! Where rivers meet the sea: estuary sand bars and deltas.

use glam::Vec2;

use crate::{
    continent::Continent,
    noise::{hash2, perlin2},
    scale::{OLD_VOXEL, m, m64},
};

use super::{Cover, funnel, node, smoothstep};

/// Smallest discharge that builds a delta.
const DELTA_Q: f32 = 1500.0;

/// A small delta: a fan of low mud flats built out into shallow sea in front of a big,
/// slow river, split by a few distributary channels.
pub struct Delta {
    /// Where the river reaches the coast, and the way it flows out to sea.
    pos: Vec2,
    dir: Vec2,
    /// Width of the river at its mouth, in voxels.
    width: f32,
    /// How far the fan reaches out to sea at most.
    pub radius: f32,
    salt: u32,
}

impl Delta {
    /// The delta at the mouth of the river leaving node `i`, if it builds one: only big
    /// rivers whose last reach is already down at sea level, so slow and silty.
    pub fn at(c: &Continent, seed: u32, i: usize) -> Option<Delta> {
        let r = c.rcv[i] as usize;
        let q = c.discharge[i];
        if c.ocean[i] || !c.ocean[r] || q < DELTA_Q || c.water_level[i] > 3.0 {
            return None;
        }
        let (a, b) = (node(c, i), node(c, r));
        let width = Continent::river_width(q) * OLD_VOXEL * funnel(c, r, q);
        Some(Delta {
            pos: (a + b) * 0.5,
            dir: (b - a).try_normalize()?,
            width,
            radius: (width * 5.0).clamp(m(40.0), m(110.0)),
            salt: hash2(seed ^ 0xde17, i as i32, 1),
        })
    }

    /// Builds the delta into a sea-side column at `p` whose ground is `ground`. Returns
    /// the cover of new flats, or `None` where the delta leaves the column alone or cuts
    /// a channel through it (the sea then fills the channel).
    pub fn shape(&self, p: Vec2, wx: f64, wz: f64, ground: &mut f32) -> Option<Cover> {
        let v = p - self.pos;
        let d = v.length();
        let along = v.dot(self.dir);
        if d > self.radius * 1.2 || along < 0.0 || *ground < -m(5.0) {
            return None;
        }
        let side = self.dir.perp_dot(v);
        let angle = side.atan2(along);
        // The fan's edge, lobed.
        let lobes = perlin2(self.salt, angle as f64 * 2.5, 0.5) * 0.3;
        let reach = self.radius * (1.0 - (angle / 1.3).powi(2)).max(0.0) * (0.85 + lobes);
        if d > reach {
            return None;
        }
        let t = d / reach.max(1.0);
        // Distributaries: the main channel straight out and two that branch off it,
        // each wandering and narrowing towards the sea.
        for (k, a0) in [0.0f32, -0.6, 0.55].into_iter().enumerate() {
            let wander = perlin2(self.salt ^ (k as u32 + 1), d as f64 / m64(35.0), k as f64 * 3.1) * 0.35;
            let lateral = (angle - a0 - wander).abs() * d;
            let half = self.width * if k == 0 { 0.4 } else { 0.25 } * (1.0 - 0.4 * t);
            // Branches split off a little way out.
            let started = k == 0 || d > self.width * 1.5;
            if started && lateral < half {
                let depth = m(1.0) + m(0.8) * (1.0 - lateral / half);
                *ground = ground.min(-depth);
                return None;
            }
        }
        // Flats a little above the sea, fading into it at the fan's edge.
        let flat = m(0.3) + m(0.6) * (1.0 - t) + perlin2(self.salt ^ 0x77, wx / m64(15.0), wz / m64(15.0)) * m(0.3);
        let edge = smoothstep(0.82, 1.0, t);
        *ground = ground.max(flat + (*ground - flat) * edge);
        Some(if edge > 0.3 { Cover::Beach } else { Cover::Bank })
    }
}

/// Raises sand bars on the bed of a tidal river channel. `bed` is the channel bed and
/// `across` runs from 0 mid-stream to 1 at the channel edge. Some bars break the surface
/// as low sand islands.
pub fn sand_bar(seed: u32, wx: f64, wz: f64, across: f32, bed: f32) -> f32 {
    if across > 0.85 {
        return bed;
    }
    let b = perlin2(seed ^ 0xba55, wx / m64(16.0), wz / m64(16.0));
    let lift = smoothstep(0.15, 0.45, b) * (1.0 - smoothstep(0.6, 0.85, across));
    bed + (m(0.5) - bed) * lift
}
