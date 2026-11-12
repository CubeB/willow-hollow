//! Overhangs: here and there the lip of a sheer face juts out a voxel or a few over
//! the ground below, where a harder bed at the top has outlasted the rock under it.
//!
//! The terrain is a heightfield, so this is a small pass of its own over the filled
//! columns: a column below and in front of a lip gets a slab of rock hanging from the
//! lip's height. Everything is decided from world-space noise and the heights of the
//! columns involved, so neighbouring chunks agree on every slab.

use crate::{
    noise::{hash2, perlin2, unit},
    scale::m64,
};

use super::smoothstep;

/// Farthest a lip juts out, in voxels.
pub const REACH: i32 = 4;

/// What the overhang pass needs to know about one column.
#[derive(Clone, Copy)]
pub struct Spot {
    /// Top solid voxel.
    pub top: i32,
    /// Bare rock face (cliff cover).
    pub cliff: bool,
    /// A slab may hang over it: no water, trail or built thing.
    pub open: bool,
    /// Plants grow on it, so it needs more headroom under a slab.
    pub grown: bool,
}

/// How far, in voxels, the lip at world column `(wx, wz)` juts out: 0 on most faces,
/// and on the minority that overhang it comes and goes along the lip.
pub fn reach(seed: u32, wx: i32, wz: i32) -> i32 {
    let (fx, fz) = (wx as f64, wz as f64);
    let gate = smoothstep(0.2, 0.4, perlin2(seed ^ 0x0e41, fx / m64(45.0), fz / m64(45.0)));
    if gate <= 0.0 {
        return 0;
    }
    let v = perlin2(seed ^ 0x0e42, fx / m64(5.0), fz / m64(5.0)) * 0.5 + 0.5;
    let jitter = unit(hash2(seed ^ 0x0e43, wx, wz)) * 0.8 - 0.4;
    ((gate * (5.2 * v - 0.6) + jitter).floor() as i32).clamp(0, REACH)
}

/// The rock, as an inclusive range of voxel heights, hanging over the column at world
/// `(wx, wz)` from a lip behind it, if any. `at` describes any column within
/// [`REACH`] + 1 of it, and `reach` is [`reach`] there: the caller's to remember, each
/// column's being asked for by all those around it.
pub fn slab(wx: i32, wz: i32, at: &impl Fn(i32, i32) -> Spot, reach: &impl Fn(i32, i32) -> i32) -> Option<(i32, i32)> {
    let p = at(wx, wz);
    if !p.open {
        return None;
    }
    let headroom = if p.grown { 4 } else { 2 };
    let mut best: Option<(i32, i32)> = None;
    'dirs: for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        // How far a lip at each distance behind this column would jut out. Most reach
        // nowhere near it, and then what the columns back there are need not be asked,
        // which for those beyond the chunk's own would mean sampling them.
        let reaches: [i32; REACH as usize] = std::array::from_fn(|i| reach(wx - dx * (i as i32 + 1), wz - dz * (i as i32 + 1)));
        if reaches.iter().zip(1..).all(|(r, d)| *r < d) {
            continue;
        }
        for d in 1..=REACH {
            // The lip, `d` columns behind this one.
            let (qx, qz) = (wx - dx * d, wz - dz * d);
            let q = at(qx, qz);
            if !q.cliff {
                continue;
            }
            let r = reaches[d as usize - 1];
            if r < d {
                continue 'dirs;
            }
            // It hangs off solid rock, not off a lone pinnacle.
            if at(qx - dx, qz - dz).top < q.top - 1 {
                continue 'dirs;
            }
            // Thicker near the face, at least two voxels at the tip, with air beneath it
            // all the way out, so it is never a loose slab.
            let thick = |k: i32| 2 + (r - k).min(2);
            for k in 1..=d {
                let s = if k == d { p } else { at(qx + dx * k, qz + dz * k) };
                if !s.open || s.top + if s.grown { 4 } else { 2 } > q.top - thick(k) {
                    continue 'dirs;
                }
            }
            if p.top + headroom > q.top - thick(d) {
                continue 'dirs;
            }
            let span = (q.top - thick(d) + 1, q.top);
            if best.is_none_or(|b| span.1 > b.1) {
                best = Some(span);
            }
            continue 'dirs;
        }
    }
    best
}
