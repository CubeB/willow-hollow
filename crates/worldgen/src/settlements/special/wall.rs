//! The old walls of ancient towns and the harbour walls of coastal great towns (#203).
//!
//! Both follow the ground, so they are not models of their own but courses laid along a
//! line of columns, wherever the layout runs them: [`old_wall`] and [`mole`] give their
//! voxels in the coordinates of that line. The structure editor shows a straight stretch
//! of each ([`stretch`], [`harbour`]) on flat ground, built by the same code.
//!
//! An old wall is worn as its years have it, deterministically from its seed and the
//! distance along it: its top ragged, stretches fallen to a course or two of rubble
//! with the stones lying beside it, its ends stepped down where it is broken, moss on
//! its stones and ivy hanging down its faces.

use std::collections::{HashMap, HashSet};

use glam::{IVec2, IVec3, Vec2};
use voxel::{Voxel, mesh::PlantBox};

use super::{Built, frame::Frame};
use crate::{
    features::landmarks::Palette,
    materials as mat,
    noise::{hash2, hash3, unit},
    settlements::kit::{Area, Door},
};

/// How high an old wall stands where it is whole, above the ground at its foot: 2.8 m.
pub const HIGH: i32 = 7;
/// Its thickness, in columns either side of its line: three columns, 1.2 m, in all.
pub const HALF: i32 = 1;
/// The half-width of a harbour wall's top, walked along: five columns in all.
pub const MOLE_HALF: i32 = 2;

/// What a course of wall is: its voxels and fine parts, in the coordinates of its line.
#[derive(Clone, Debug, Default)]
pub struct Laid {
    pub voxels: Vec<(IVec3, Voxel)>,
    pub fine: Vec<PlantBox>,
}

/// The way along a line at its point `i`, and across it.
fn frame_at(path: &[IVec2], i: usize) -> (Vec2, Vec2) {
    let n = path.len();
    let t = (path[(i + 2).min(n - 1)] - path[i.saturating_sub(2)]).as_vec2().normalize_or(Vec2::X);
    (t, Vec2::new(-t.y, t.x))
}

/// Whether the stretch of old wall about distance `s` along it has fallen.
pub fn fallen(seed: u32, age: f32, s: i32) -> bool {
    unit(hash2(seed ^ 0x0fa1_1e17, s.div_euclid(11), 3)) < 0.08 + 0.3 * age
}

/// An old wall along `path` (columns in order, each beside the one before), the first of
/// them `s0` columns along the whole wall so its wear runs on from piece to piece.
/// `ground` is the highest solid voxel of a column and `may` whether anything of the wall
/// may stand in one; `broken` says, for each end, whether the wall stops there in a gap
/// (and so steps down to it) rather than meeting something built.
pub fn old_wall(path: &[IVec2], s0: i32, broken: (bool, bool), ground: impl Fn(IVec2) -> i32, may: impl Fn(IVec2) -> bool, pal: &Palette, seed: u32, age: f32) -> Laid {
    let mut out = Laid::default();
    let n = path.len();
    if n == 0 {
        return out;
    }
    // Every column of the wall: its height above the ground, and the point of the line it
    // was laid from.
    let mut cols: HashMap<IVec2, (i32, usize)> = HashMap::new();
    let mut order: Vec<IVec2> = Vec::new();
    for (i, &p) in path.iter().enumerate() {
        let s = s0 + i as i32;
        let mut top = if fallen(seed, age, s) {
            (unit(hash2(seed ^ 0x2b, s, 1)) * 2.0) as i32
        } else {
            // Ragged, the more so the older, in steps of a few columns.
            let worn = unit(hash2(seed ^ 0x7a66, s.div_euclid(3), 2));
            HIGH - (worn * worn * (1.0 + 4.0 * age)) as i32
        };
        // A broken end steps down to the gap.
        let from_end = [(broken.0, i), (broken.1, n - 1 - i)].iter().filter(|(b, _)| *b).map(|&(_, k)| k as i32).min();
        if let Some(k) = from_end {
            top = top.min(1 + 2 * k);
        }
        for dz in -HALF..=HALF {
            for dx in -HALF..=HALF {
                let c = p + IVec2::new(dx, dz);
                if !cols.contains_key(&c) {
                    cols.insert(c, (top, i));
                    order.push(c);
                }
            }
        }
    }
    order.sort_unstable_by_key(|c| (c.x, c.y));
    let moss = pal.moss * (0.3 + 0.6 * age) + 0.1 * age;
    for &c in &order {
        if !may(c) {
            continue;
        }
        let (top, i) = cols[&c];
        let g = ground(c);
        let s = s0 + i as i32;
        let whole = !fallen(seed, age, s);
        for y in g - 3..=g + top {
            let r = unit(hash3(seed ^ 0x570e, c.x, y, c.y));
            let v = if y == g + top && whole && top >= HIGH - 1 {
                pal.stone_dark
            } else if r < moss {
                mat::MOSSY_STONE
            } else if r < 0.4 {
                pal.stone_dark
            } else {
                pal.stone
            };
            out.voxels.push((IVec3::new(c.x, y, c.y), v));
        }
    }
    // Stones fallen from it lie beside a fallen stretch; ivy hangs down its faces.
    for (i, &p) in path.iter().enumerate() {
        let s = s0 + i as i32;
        let (_, nrm) = frame_at(path, i);
        for side in [-1.0f32, 1.0] {
            let beside = (p.as_vec2() + 0.5 + nrm * side * (HALF as f32 + 1.5)).floor().as_ivec2();
            if cols.contains_key(&beside) || !may(beside) {
                continue;
            }
            let h = hash2(seed ^ 0x5106, s, side as i32);
            if !fallen(seed, age, s) {
                let patch = unit(hash2(seed ^ 0x1717, s.div_euclid(4), side as i32 + 5));
                let (top, _) = cols[&p];
                if patch < 0.12 + 0.45 * age && top >= 3 {
                    let wall_top = ground(p) + top;
                    let long = 2 + (h % 4) as i32;
                    let foot = ground(beside);
                    for y in (wall_top - long + 1).max(foot + 1)..=wall_top {
                        let v = [mat::IVY_GLOSS, mat::IVY_GLOSS_LIT, mat::IVY_GLOSS_NEW][(hash3(seed, beside.x, y, beside.y) % 3) as usize];
                        out.voxels.push((IVec3::new(beside.x, y, beside.y), v));
                    }
                }
            } else if unit(h) < 0.4 {
                let v = if h % 2 == 0 { pal.stone } else { mat::MOSSY_STONE };
                out.voxels.push((IVec3::new(beside.x, ground(beside) + 1, beside.y), v));
            }
        }
    }
    out
}

/// A harbour wall along `path`, out from the shore: a broad top of flags `MOLE_HALF`
/// columns either side of its line at `top`, built up from `seabed` (the highest solid
/// voxel of a column, under the water or not), a parapet along its side toward the open
/// sea, which `seaward(i)` gives at each point as +1 or -1 across the line, and mooring
/// posts along the other; at its end a round head for a beacon. `may` says where it may
/// stand.
pub fn mole(path: &[IVec2], top: i32, seabed: impl Fn(IVec2) -> i32, seaward: impl Fn(usize) -> f32, may: impl Fn(IVec2) -> bool, pal: &Palette, seed: u32) -> Laid {
    let mut out = Laid::default();
    let n = path.len();
    if n == 0 {
        return out;
    }
    let mut cols: HashMap<IVec2, Voxel> = HashMap::new();
    let mut parapets: HashSet<IVec2> = HashSet::new();
    for (i, &p) in path.iter().enumerate() {
        let (_, nrm) = frame_at(path, i);
        let side = seaward(i);
        for k in -MOLE_HALF * 2..=MOLE_HALF * 2 {
            let c = (p.as_vec2() + 0.5 + nrm * k as f32 * 0.5).floor().as_ivec2();
            cols.entry(c).or_insert(if (k.abs() + i as i32) % 5 == 0 { pal.stone_dark } else { pal.stone });
        }
        // The parapet's column, toward the sea.
        let wall = (p.as_vec2() + 0.5 + nrm * side * (MOLE_HALF as f32 + 1.0)).floor().as_ivec2();
        cols.insert(wall, pal.stone_dark);
        parapets.insert(wall);
    }
    // The round head.
    let head = path[n - 1];
    for dz in -4..=4 {
        for dx in -4..=4 {
            if dx * dx + dz * dz <= 18 {
                let c = head + IVec2::new(dx, dz);
                cols.insert(c, pal.stone_dark);
                parapets.remove(&c);
            }
        }
    }
    let mut order: Vec<(IVec2, Voxel)> = cols.into_iter().collect();
    order.sort_unstable_by_key(|(c, _)| (c.x, c.y));
    for (c, v) in order {
        if !may(c) {
            continue;
        }
        let parapet = parapets.contains(&c);
        let foot = seabed(c).min(top) - 2;
        for y in foot..=top {
            let r = unit(hash3(seed ^ 0x3013, c.x, y, c.y));
            // Weed and wet stone low down where the sea washes it.
            let w = if y == top && !parapet { v } else if y < top - 2 && r < 0.5 { mat::MOSSY_STONE } else if r < 0.35 { pal.stone_dark } else { pal.stone };
            out.voxels.push((IVec3::new(c.x, y, c.y), w));
        }
        if parapet {
            for y in top + 1..=top + 2 {
                out.voxels.push((IVec3::new(c.x, y, c.y), pal.stone_dark));
            }
        }
    }
    // Mooring posts along the sheltered side.
    for (i, &p) in path.iter().enumerate().skip(6).step_by(9) {
        let (_, nrm) = frame_at(path, i);
        let c = (p.as_vec2() + 0.5 - nrm * seaward(i) * MOLE_HALF as f32).floor().as_ivec2();
        if may(c) {
            out.voxels.push((IVec3::new(c.x, top + 1, c.y), pal.wood));
        }
    }
    out
}

/// A straight stretch of old wall on flat ground, along z through the origin, broken off
/// at both ends: how an ancient town's wall looks, for the structure editor.
pub(super) fn stretch(mut f: Frame) -> (Built, Vec<String>) {
    let path: Vec<IVec2> = (-18..=18).map(|z| IVec2::new(0, z)).collect();
    // From a point along the wall the seed gives, so each variant shows other wear.
    let s0 = (f.spec.seed % 97) as i32 * 7;
    let laid = old_wall(&path, s0, (true, true), |_| -1, |_| true, f.pal, f.spec.seed, f.spec.age);
    put(&mut f, laid);
    f.door = Some(Door { floor: IVec3::new(3, -1, 0), facing: IVec3::X, width: 1, height: 1 });
    f.finish(Area { lo: IVec2::new(-HALF, -18), hi: IVec2::new(HALF, 18) }, 0)
}

/// A stretch of harbour wall on flat ground, curving out to its head, its parapet on
/// the outer side, for the structure editor; it is shown standing on dry land with its
/// top a little above the ground, where in the world the sea comes up its sides.
pub(super) fn harbour(mut f: Frame) -> (Built, Vec<String>) {
    let mut path: Vec<IVec2> = Vec::new();
    for k in 0..=40 {
        let a = k as f32 / 40.0 * 1.2;
        let p = (Vec2::new(-22.0, 0.0) + Vec2::new(a.sin(), 1.0 - a.cos()) * 30.0).round().as_ivec2();
        if path.last() != Some(&p) {
            path.push(p);
        }
    }
    let laid = mole(&path, 2, |_| -1, |_| 1.0, |_| true, f.pal, f.spec.seed);
    put(&mut f, laid);
    f.door = Some(Door { floor: IVec3::new(6, -1, 0), facing: IVec3::X, width: 1, height: 1 });
    let (lo, hi) = path.iter().fold((path[0], path[0]), |(a, b), &p| (a.min(p), b.max(p)));
    f.finish(Area { lo, hi }, 0)
}

fn put(f: &mut Frame, laid: Laid) {
    for (q, v) in laid.voxels {
        f.put(q, v);
    }
    f.fine.extend(laid.fine);
}
