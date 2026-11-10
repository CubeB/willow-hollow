//! Springs: the small pools where streams begin.
//!
//! A river node with no river feeding it is where water comes out of the ground. There
//! the stream starts from a round pool a little upstream of the node, ringed with wet,
//! mossy stone.

use glam::Vec2;

use super::{MIN_RIVER_Q, fall_drop, node, pool_level, river_level};
use crate::{continent::{Continent, NO_LAKE}, noise::{hash2, unit}, scale::m};

/// Width of the rim held at the pool's level, so its water cannot leak out.
const RIM: f32 = m(0.8);
/// Width of the wet stone and moss around the pool.
const WET: f32 = m(2.0);

#[derive(Clone, Copy, Debug)]
pub struct Spring {
    /// Centre of the pool, in continent-local voxels.
    pub pos: Vec2,
    /// Radius of the pool's water.
    pub radius: f32,
    /// Water surface plane of the pool.
    pub water: i32,
}

impl Spring {
    /// The spring at the source node `i`, if it is one.
    pub fn at(c: &Continent, world_seed: u32, i: usize) -> Option<Spring> {
        let size = c.size as isize;
        let (x, z) = ((i % c.size) as isize, (i / c.size) as isize);
        if x < 1 || z < 1 || x >= size - 1 || z >= size - 1 {
            return None;
        }
        let is_river = |j: usize| !c.ocean[j] && c.discharge[j] >= MIN_RIVER_Q;
        if !is_river(i) || c.lake[i] != NO_LAKE || fall_drop(c, i) > 0.0 {
            return None;
        }
        let r = c.rcv[i] as usize;
        if c.ocean[r] {
            return None;
        }
        for dz in -1..=1isize {
            for dx in -1..=1isize {
                let d = (i as isize + dz * size + dx) as usize;
                if d != i && c.rcv[d] as usize == i && is_river(d) {
                    return None;
                }
            }
        }
        let (pi, pr) = (node(c, i), node(c, r));
        let down = (pr - pi).normalize_or_zero();
        let h = hash2(world_seed ^ 0x5e71, i as i32, 17);
        let radius = m(1.4) + unit(h) * m(1.2);
        Some(Spring {
            pos: pi - down * radius * 0.6,
            radius,
            water: pool_level(c.desc.seed, river_level(c, i)),
        })
    }

    /// Farthest from the centre this spring changes anything.
    pub fn reach(&self) -> f32 {
        self.radius + RIM + WET
    }

    /// Shapes a column `d` voxels from the centre. Returns the pool's water plane if
    /// the column is under it, and the distance from the pool's edge.
    pub fn shape(&self, d: f32, wx: f64, wz: f64, ground: &mut f32) -> (Option<i32>, f32) {
        let k = self.water as f32;
        let edge = d - self.radius;
        if edge < 0.0 {
            // A rounded bowl, with a little noise so the bed is not a perfect dish.
            let across = d / self.radius;
            let wobble = unit(hash2(0x5b0b, wx as i32, wz as i32)) * 0.4;
            let bed = k - 1.2 - m(0.9) * (1.0 - across * across) - wobble;
            *ground = ground.min(bed);
            (Some(self.water), edge)
        } else {
            // The rim holds the level, then eases back to the ground around it.
            let rim = k - 0.5;
            let t = super::smoothstep(RIM, RIM + WET, edge);
            *ground = rim + (*ground - rim) * t;
            (None, edge)
        }
    }

    /// Whether a column `edge` voxels outside the pool is on its wet stone rim.
    pub fn wet(edge: f32) -> bool {
        (0.0..RIM + WET * 0.6).contains(&edge)
    }
}
