//! Ravines: narrow deep cuts with sheer rock walls.
//!
//! Two kinds. River gorges form where a sizeable river runs steeply through hard rock:
//! its banks become a strip of gravel and boulders at the water's level under sheer
//! walls, and where the land beside the river is lower than the gorge is deep, the rim
//! is raised and fades back into the natural slope. Dry fault ravines follow lines of
//! weakness that have nothing to do with the rivers: long, narrow slots with a rubble
//! floor that shallow out at their ends.
//!
//! Both reuse the cliff covers, so their walls show rock strata and their floors scree.

use crate::{
    continent::{Continent, NO_LAKE},
    macro_layout::MACRO_SIZE,
    noise::{hash2, perlin2, unit},
    scale::{m, m64},
};

use super::{cliff::{self, Part}, node, smoothstep};

/// Depth, in voxels, of the gorge the river leaving node `i` runs in, or 0.
///
/// Gorges need a river of some size, a steep reach (a knickpoint, where the river is
/// still cutting down) and hard rock that stands in walls rather than slumping.
pub fn gorge_depth(c: &Continent, seed: u32, i: usize) -> f32 {
    let r = c.rcv[i] as usize;
    if r == i || c.ocean[i] || c.ocean[r] || c.lake[i] != NO_LAKE || c.lake[r] != NO_LAKE {
        return 0.0;
    }
    let big = smoothstep(100.0, 300.0, c.discharge[i]);
    if big <= 0.0 {
        return 0.0;
    }
    let diagonal = (i % c.size != r % c.size) && (i / c.size != r / c.size);
    let run = if diagonal { std::f32::consts::SQRT_2 } else { 1.0 } * c.sim_cell_size();
    let steep = smoothstep(0.02, 0.045, (c.water_level[i] - c.water_level[r]) / run);
    if steep <= 0.0 {
        return 0.0;
    }
    let p = node(c, i);
    let wx = (c.desc.mx as i64 * MACRO_SIZE as i64) as f64 + p.x as f64;
    let wz = (c.desc.mz as i64 * MACRO_SIZE as i64) as f64 + p.y as f64;
    let hard = cliff::hardness(seed, wx, wz);
    let deep = smoothstep(-0.4, 0.4, perlin2(seed ^ 0x6026, wx / m64(500.0), wz / m64(500.0)));
    (m(10.0) + m(14.0) * deep) * big * steep * hard
}

/// Farthest from the channel edge a gorge of this depth changes the ground.
pub fn gorge_reach(depth: f32) -> f32 {
    m(6.0) + depth * 2.5
}

/// Ground at `sd` voxels from the edge of a river channel of half-width `hw` that runs
/// in a gorge `depth` deep below the river's smooth water `level`, where the water's
/// banks stand at `bank` and the land would
/// naturally be at `natural`; and what part of the gorge that is.
#[allow(clippy::too_many_arguments)]
pub fn gorge_wall(seed: u32, wx: f64, wz: f64, sd: f32, hw: f32, bank: f32, level: f32, depth: f32, natural: f32) -> (f32, Part) {
    // The floor beside the water widens and narrows, and the wall is ragged.
    // Narrow runnels, where water coming off the land has cut back into the walls.
    let runnel = smoothstep(0.84, 0.97, 1.0 - perlin2(seed ^ 0x6029, wx / m64(3.0), wz / m64(3.0)).abs());
    let rough = perlin2(seed ^ 0x6027, wx / m64(7.0), wz / m64(7.0)) * m(1.2)
        + (unit(hash2(seed ^ 0x6028, wx as i32, wz as i32)) - 0.5) * m(0.4)
        + runnel * m(0.8);
    let wall = (m(1.0) + hw * 0.25 + rough).max(m(0.6));
    if sd < wall {
        // Boulders lie scattered on the gravel, away from the water's edge.
        let h = hash2(seed ^ 0xb01d, wx as i32 >> 1, wz as i32 >> 1);
        let boulder = if sd > m(0.6) && unit(h) < 0.12 { m(0.4) + m(0.4) * unit(h.rotate_left(9)) } else { 0.0 };
        return (bank + boulder, Part::Foot(1.0));
    }
    // Beyond the wall the land stands at least `depth` above the water, fading back
    // to its natural height.
    // The rim follows the smooth water level, not the stepped pools, so it does not
    // jump at every drop.
    let rim = level + depth;
    let t = ((sd - wall) / (depth * 2.5)).min(1.0);
    let ground = natural + (rim - natural).max(0.0) * (1.0 - t);
    let part = if sd - wall < m(1.0) && ground - bank > m(1.6) { Part::Face(ground - bank) } else { Part::None };
    (ground, part)
}

/// Cuts any dry fault ravine into a column whose ground is `ground`, scaled by `fade`
/// (0 to 1, used to keep ravines away from rivers). `fade` is only called on the few
/// columns near a fault line, since finding the nearest river is not cheap.
pub fn dry(seed: u32, wx: f64, wz: f64, fade: impl FnOnce() -> f32, ground: &mut f32) -> Part {
    const LINE: f64 = m64(700.0);
    // Faults run along the zero lines of one broad noise field.
    let n = perlin2(seed ^ 0xfa17, wx / LINE, wz / LINE);
    if n.abs() > 0.05 || *ground < m(10.0) {
        return Part::None;
    }
    // Only some stretches of each line open up.
    let open = smoothstep(0.32, 0.45, perlin2(seed ^ 0xfa18, wx / m64(400.0), wz / m64(400.0)));
    if open <= 0.0 {
        return Part::None;
    }
    let open = open * fade();
    if open <= 0.0 {
        return Part::None;
    }
    // Distance to the line: the noise value over its gradient.
    let e = m64(2.0);
    let gx = perlin2(seed ^ 0xfa17, (wx + e) / LINE, wz / LINE) - perlin2(seed ^ 0xfa17, (wx - e) / LINE, wz / LINE);
    let gz = perlin2(seed ^ 0xfa17, wx / LINE, (wz + e) / LINE) - perlin2(seed ^ 0xfa17, wx / LINE, (wz - e) / LINE);
    let grad = gx.hypot(gz) / (2.0 * e as f32);
    if grad < 1e-7 {
        return Part::None;
    }
    let dist = n.abs() / grad;
    let width = perlin2(seed ^ 0xfa19, wx / m64(60.0), wz / m64(60.0));
    let rough = perlin2(seed ^ 0xfa1a, wx / m64(6.0), wz / m64(6.0)) * m(0.8)
        + (unit(hash2(seed ^ 0xfa1b, wx as i32, wz as i32)) - 0.5) * m(0.4);
    let half = (m(3.0) + m(2.0) * width) * open + rough * open;
    if dist > half + 3.0 {
        return Part::None;
    }
    if dist >= half {
        // The top of the wall.
        return if open > 0.3 { Part::Face(m(8.0) * open) } else { Part::None };
    }
    let deep = smoothstep(-0.4, 0.4, perlin2(seed ^ 0xfa1c, wx / m64(250.0), wz / m64(250.0)));
    let depth = ((m(8.0) + m(10.0) * deep) * open).min(*ground - m(4.0)).max(0.0);
    // A rubble floor.
    let h = hash2(seed ^ 0xb01e, wx as i32 >> 1, wz as i32 >> 1);
    let boulder = if unit(h) < 0.15 { m(0.4) + m(0.4) * unit(h.rotate_left(9)) } else { 0.0 };
    *ground -= depth - boulder.min(depth);
    if depth > m(1.6) { Part::Foot(1.0) } else { Part::None }
}
