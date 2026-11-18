//! The taiga and tundra, and the mountains: carved gables, stores on posts, hides across
//! the door against the cold, balconies of drying hay, a bell on the ridge.

use glam::IVec3;

use super::{Cx, common::fits_box};
use crate::{
    materials as mat,
    settlements::{Region, kit::Cover},
};

/// Carved boards along the underside of the gable's verge, with pendants hanging from
/// them, and a carved sun under the peak.
pub(super) fn carved_boards(cx: &mut Cx) -> Option<()> {
    let colour = if cx.wealth() > 0.6 && cx.r.region == Region::Northern { mat::PAINT_RED } else { cx.pal().wood };
    let mut boards = 0;
    let mut peak: Option<(i32, i32)> = None;
    for z in -10..=10 {
        let Some(under) = (-4..=12).find(|&y| cx.wall(IVec3::new(0, y, z)) && cx.open(IVec3::new(0, y - 1, z))) else { continue };
        let (lo, hi) = (IVec3::new(0, under * 4 - 2, z * 4), IVec3::new(1, under * 4, z * 4 + 4));
        if !cx.room(lo, hi) {
            continue;
        }
        cx.part(lo, hi, colour);
        boards += 1;
        if z.rem_euclid(2) == 0 && cx.room(IVec3::new(0, under * 4 - 4, z * 4 + 1), IVec3::new(1, under * 4 - 2, z * 4 + 3)) {
            cx.part(IVec3::new(0, under * 4 - 4, z * 4 + 1), IVec3::new(1, under * 4 - 2, z * 4 + 3), colour);
        }
        if peak.is_none_or(|(_, u)| under > u) {
            peak = Some((z, under));
        }
    }
    if let Some((z, under)) = peak {
        let (lo, hi) = (IVec3::new(0, under * 4 - 9, z * 4), IVec3::new(1, under * 4 - 5, z * 4 + 4));
        if cx.wall(IVec3::new(-1, (under * 4 - 9).div_euclid(4), z)) && cx.room(lo, hi) {
            cx.part(lo, hi, colour);
        }
    }
    (boards >= 3).then_some(())
}

/// A store raised on four posts out of the reach of mice: a log box with its door up a
/// ladder, roofed as the house is.
pub(super) fn raised_store(cx: &mut Cx) -> Option<()> {
    if cx.level(0..=3, -2..=1, 0, 1) != Some(0) || !fits_box(cx, IVec3::new(-1, 0, -2), IVec3::new(4, 7, 1)) {
        return None;
    }
    let p = *cx.pal();
    let roof = if cx.r.style.roof == Cover::Turf { mat::ROOF_TURF } else { cx.roofing() };
    for (x, z) in [(0, -2), (3, -2), (0, 1), (3, 1)] {
        for y in 0..=1 {
            cx.put(IVec3::new(x, y, z), p.wood);
        }
    }
    for x in 0..=3 {
        for z in -2..=1 {
            cx.put(IVec3::new(x, 2, z), p.plank);
            let edge = x == 0 || x == 3 || z == -2 || z == 1;
            for y in 3..=5 {
                let door = x == 3 && z == -1 && y < 5;
                if edge && !door {
                    cx.put(IVec3::new(x, y, z), p.wood);
                }
            }
        }
    }
    for x in -1..=4 {
        for z in -2..=1 {
            cx.put(IVec3::new(x, 6, z), roof);
        }
    }
    for x in 1..=2 {
        for z in -2..=1 {
            cx.put(IVec3::new(x, 7, z), roof);
        }
    }
    // The ladder up to the door.
    for z in [-4, -1] {
        cx.part(IVec3::new(16, 0, z), IVec3::new(17, 12, z + 1), p.wood);
    }
    for y in (2..12).step_by(3) {
        cx.part(IVec3::new(16, y, -3), IVec3::new(17, y + 1, -1), p.wood);
    }
    Some(())
}

/// Hides hung across the doorway from its lintel, against the cold; let go, some have
/// fallen. They are cloth, and stir in a draught.
pub(super) fn hide_curtain(cx: &mut Cx) -> Option<()> {
    let (z0, z1, h) = cx.doorway()?;
    if !cx.wall(IVec3::new(-1, h, z0)) || !cx.wall(IVec3::new(-1, h, z1)) {
        return None;
    }
    let long = (h * 4 - 4).min(voxel::mesh::RIBBON_BENDS);
    let mut hung = 0;
    for z in (z0 * 4..(z1 + 1) * 4).step_by(2) {
        if cx.decayed() && cx.rng.unit() < 0.4 {
            continue;
        }
        let colour = if cx.rng.unit() < 0.7 { mat::HIDE } else { mat::LEATHER };
        let short = cx.rng.int(0, 3);
        cx.ribbon(IVec3::new(-1, h * 4, z), long - short, colour, true);
        hung += 1;
    }
    (hung > 0).then_some(())
}

/// A timber balcony across the gable, on brackets, with hay hung over its rail to dry.
pub(super) fn balcony(cx: &mut Cx) -> Option<()> {
    let (zl, zh) = (-3, 3);
    if !(zl..=zh).all(|z| cx.wall(IVec3::new(-1, -1, z))) || !fits_box(cx, IVec3::new(0, -1, zl), IVec3::new(1, -1, zh)) || !cx.room(IVec3::new(0, 0, zl * 4), IVec3::new(9, 12, (zh + 1) * 4)) {
        return None;
    }
    let p = *cx.pal();
    for x in 0..=1 {
        for z in zl..=zh {
            cx.put(IVec3::new(x, -1, z), p.plank);
        }
    }
    let (za, zb) = (zl * 4, (zh + 1) * 4);
    for z in (za..zb).step_by(4).chain([zb - 1]) {
        cx.part(IVec3::new(7, 0, z), IVec3::new(8, 5, z + 1), p.wood);
    }
    cx.part(IVec3::new(7, 5, za), IVec3::new(8, 6, zb), p.wood);
    for z in [za, zb - 1] {
        cx.part(IVec3::new(0, 5, z), IVec3::new(7, 6, z + 1), p.wood);
    }
    for z in [zl, zh] {
        cx.part(IVec3::new(0, -6, z * 4 + 1), IVec3::new(1, -4, z * 4 + 3), p.wood);
        cx.part(IVec3::new(1, -5, z * 4 + 1), IVec3::new(4, -4, z * 4 + 3), p.wood);
    }
    let hay = if cx.decayed() { mat::HAY_ROTTEN } else { mat::HAY };
    for z in (za + 1..zb - 3).step_by(3) {
        if cx.rng.unit() < 0.75 {
            let low = 1 + cx.rng.int(0, 2);
            cx.part(IVec3::new(6, low, z), IVec3::new(9, 6, z + 3), hay);
        }
    }
    Some(())
}

/// A small bell under a little roof between two posts on the ridge.
pub(super) fn bell(cx: &mut Cx) -> Option<()> {
    if !cx.wall(IVec3::new(0, -1, -1)) || !cx.wall(IVec3::new(0, -1, 1)) || !cx.room(IVec3::new(-1, 0, -4), IVec3::new(5, 12, 8)) {
        return None;
    }
    let p = *cx.pal();
    for z in [-3, 5] {
        cx.part(IVec3::new(1, 0, z), IVec3::new(3, 10, z + 2), p.wood);
    }
    cx.part(IVec3::new(1, 10, -3), IVec3::new(3, 11, 7), p.wood);
    cx.part(IVec3::new(-1, 11, -4), IVec3::new(5, 12, 8), mat::PLANK_DARK);
    cx.part(IVec3::new(1, 9, 1), IVec3::new(3, 10, 3), mat::IRON);
    cx.part(IVec3::new(0, 5, 0), IVec3::new(4, 9, 4), mat::IRON);
    Some(())
}

/// Crossed horns at an end of the ridge, rising over the roof either side.
pub(super) fn horns(cx: &mut Cx) -> Option<()> {
    if !cx.wall(IVec3::new(0, -1, 0)) || !cx.room(IVec3::new(-5, 0, 1), IVec3::new(9, 13, 3)) {
        return None;
    }
    let colour = if cx.r.region == Region::Rainforest && cx.wealth() > 0.6 { mat::PAINT_RED } else { cx.pal().wood };
    for k in 0..6 {
        for side in [-1, 1] {
            let lo = IVec3::new(1 + side * k, k * 2, 1);
            cx.part(lo, lo + IVec3::new(2, 3, 2), colour);
        }
    }
    Some(())
}
