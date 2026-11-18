//! The desert and the savanna: shade above all, cloth and fronds over doors, yards and
//! flat roofs; water kept cool in clay; and colour where the walls are white.

use glam::IVec3;

use super::{Cx, common::fits_box};
use crate::{
    materials as mat,
    settlements::kit::{AnchorKind, RoofShape},
};

/// An awning of cloth: over a door on two poles, or over a window on two arms.
pub(super) fn awning(cx: &mut Cx) -> Option<()> {
    if cx.r.anchor.kind == AnchorKind::Window {
        return window_awning(cx);
    }
    let (z0, z1, h) = cx.doorway()?;
    let deep = 3 + (cx.wealth() > 0.6) as i32;
    let (za, zb, x1) = ((z0 - 1) * 4 + 2, (z1 + 2) * 4 - 2, deep * 4);
    // Tied to the wall at the lintel or above it, wherever the wall is there to tie to
    // and the eaves leave room.
    let back = (h..h + 4).find(|&y| (z0 - 1..=z1 + 1).all(|z| cx.wall(IVec3::new(-1, y, z))) && cx.room(IVec3::new(0, y * 4 - 6, za), IVec3::new(x1 + 1, y * 4 + 4, zb)))?;
    let yb = back * 4 + 3;
    // Sloping out to the front, but never lower than a person walking under it.
    let yf = (yb - deep * 2).max(5 * 4 + 2).min(yb);
    let colour = cx.cloth();
    // Let go, one pole has fallen and its corner hangs down.
    let fallen = if cx.decayed() { cx.rng.int(0, 1) } else { -1 };
    let deep_floor = (-cx.ground()).max(1);
    let mut ties = [yf, yf];
    for (k, z) in [za, zb - 1].into_iter().enumerate() {
        if fallen == k as i32 {
            ties[k] = yf - 8;
            continue;
        }
        let stand = cx.stands(deep - 1, z.div_euclid(4), 0, deep_floor)?;
        cx.part(IVec3::new(x1 - 1, stand * 4, z), IVec3::new(x1, yf, z + 1), cx.pal().wood);
    }
    cx.sheet([IVec3::new(0, yb, za), IVec3::new(0, yb, zb), IVec3::new(x1, ties[0], za), IVec3::new(x1, ties[1], zb)], colour);
    Some(())
}

/// A small awning over a window, tied above it and held out on two arms.
fn window_awning(cx: &mut Cx) -> Option<()> {
    let (z0, z1, top) = cx.window()?;
    if !(z0 - 1..=z1 + 1).all(|z| cx.wall(IVec3::new(-1, top, z))) {
        return None;
    }
    let (za, zb) = ((z0 - 1) * 4 + 2, (z1 + 2) * 4 - 2);
    let (yb, yf) = (top * 4 + 3, top * 4 + 1);
    if !cx.room(IVec3::new(0, top * 4, za), IVec3::new(7, top * 4 + 4, zb)) {
        return None;
    }
    let colour = cx.cloth();
    for z in [za, zb - 1] {
        cx.part(IVec3::new(0, top * 4, z), IVec3::new(6, top * 4 + 1, z + 1), cx.pal().wood);
    }
    cx.sheet([IVec3::new(0, yb, za), IVec3::new(0, yb, zb), IVec3::new(6, yf, za), IVec3::new(6, yf, zb)], colour);
    Some(())
}

/// A sail of cloth over a yard or a flat roof, tied to posts, or to walls where they
/// stand at a corner: two corners high and two low, so it twists as sails do.
pub(super) fn sail_shade(cx: &mut Cx) -> Option<()> {
    let roof = cx.r.anchor.kind == AnchorKind::RoofTop;
    if roof && (cx.r.style.shape != RoofShape::Flat || !cx.wall(IVec3::new(0, -1, 0))) {
        return None;
    }
    // As big as there is room for, across the posts.
    let sizes: &[(i32, i32)] = if roof { &[(7, 6), (6, 5), (5, 4), (4, 3)] } else { &[(8, 6), (7, 6), (6, 5), (5, 4)] };
    let (high, low) = if roof { (6, 5) } else { (8, 6) };
    let fallen = if cx.decayed() { cx.rng.int(0, 3) } else { -1 };
    for &(w, d) in sizes {
        let (x0, z0) = (-d / 2, -w / 2);
        let (x1, z1) = (x0 + d, z0 + w);
        let corners = [(x0, z0), (x1, z0), (x1, z1), (x0, z1)];
        let tall = |k: usize| if k % 2 == 0 { high } else { low };
        let Some(bases) = corners.iter().map(|&(x, z)| cx.stands(x, z, 0, 2)).collect::<Option<Vec<i32>>>() else { continue };
        // Each post stands clear, or a wall stands there to tie to.
        let clear = |k: usize| {
            let ((x, z), base) = (corners[k], bases[k]);
            cx.wall(IVec3::new(x, base + tall(k) - 1, z)) || (base..base + tall(k)).all(|y| cx.free(IVec3::new(x, y, z)))
        };
        if !(0..4).all(clear) {
            continue;
        }
        let mut ties = Vec::new();
        for (k, &(x, z)) in corners.iter().enumerate() {
            let base = bases[k];
            // Inner corner of the post, or of the wall standing there.
            let cell = IVec3::new(if x == x0 { x * 4 + 4 } else { x * 4 }, (base + tall(k)) * 4 - 1, if z == z0 { z * 4 + 4 } else { z * 4 });
            if fallen == k as i32 {
                ties.push(cell - IVec3::Y * 10);
                continue;
            }
            if !cx.wall(IVec3::new(x, base + tall(k) - 1, z)) {
                for y in base..base + tall(k) {
                    cx.put(IVec3::new(x, y, z), cx.pal().wood);
                }
            }
            ties.push(cell);
        }
        let colour = cx.cloth();
        cx.sheet([ties[0], ties[1], ties[2], ties[3]], colour);
        return Some(());
    }
    None
}

/// A flat roof lived on: a rug, cushions and a low table under a shade on poles, and
/// jars at its edge.
pub(super) fn terrace(cx: &mut Cx) -> Option<()> {
    if cx.r.style.shape != RoofShape::Flat || !cx.wall(IVec3::new(0, -1, 0)) || !cx.room(IVec3::new(-8, 0, -8), IVec3::new(12, 21, 12)) {
        return None;
    }
    let (p, colour) = (*cx.pal(), cx.cloth());
    cx.part(IVec3::new(-6, 0, -6), IVec3::new(10, 1, 10), if cx.decayed() { mat::SACKCLOTH } else { colour });
    for (x, z) in [(-5, -2), (-5, 4), (6, 1)] {
        let c = cx.cloth();
        cx.part(IVec3::new(x, 1, z), IVec3::new(x + 3, 2, z + 3), c);
    }
    cx.part(IVec3::new(0, 1, 0), IVec3::new(4, 3, 4), p.plank);
    for (x, z) in [(-8, -8), (10, -8)] {
        cx.part(IVec3::new(x, 0, z), IVec3::new(x + 2, 4, z + 2), mat::CLAY);
    }
    for (x, z) in [(-6, -6), (9, -6), (-6, 9), (9, 9)] {
        cx.part(IVec3::new(x, 0, z), IVec3::new(x + 1, 20, z + 1), p.wood);
    }
    let shade = cx.cloth();
    cx.sheet([IVec3::new(-6, 20, -6), IVec3::new(10, 20, -6), IVec3::new(10, 19, 10), IVec3::new(-6, 19, 10)], shade);
    Some(())
}

/// A screen of turned wood before a window, under a little hood: to look out unseen and
/// keep the sun off. Painted where the household can afford it.
pub(super) fn lattice(cx: &mut Cx) -> Option<()> {
    let (z0, z1, top) = cx.window()?;
    if !cx.wall(IVec3::new(-1, top, z0)) {
        return None;
    }
    let (za, zb) = (z0 * 4, (z1 + 1) * 4);
    if !cx.room(IVec3::new(0, 0, za), IVec3::new(3, top * 4, zb)) || !cx.room(IVec3::new(0, top * 4, za - 2), IVec3::new(4, top * 4 + 1, zb + 2)) {
        return None;
    }
    let colour = if cx.wealth() > 0.6 { [mat::PAINT_GREEN, mat::PAINT_BLUE][(cx.rng.next_u32() & 1) as usize] } else { cx.pal().wood };
    cx.part(IVec3::new(0, top * 4, za - 2), IVec3::new(4, top * 4 + 1, zb + 2), colour);
    cx.part(IVec3::new(0, 0, za), IVec3::new(3, 1, zb), colour);
    for z in (za..zb).step_by(2) {
        cx.part(IVec3::new(2, 1, z), IVec3::new(3, top * 4, z + 1), colour);
    }
    for y in (3..top * 4).step_by(3) {
        cx.part(IVec3::new(2, y, za), IVec3::new(3, y + 1, zb), colour);
    }
    Some(())
}

/// Palm fronds laid over poles: a lean-to of shade against a wall, or a roof of it on
/// four poles in the yard. The fronds' tips hang over the front.
pub(super) fn palm_shade(cx: &mut Cx) -> Option<()> {
    let deep = 3;
    let (za, zb) = (-2 * 4, 2 * 4);
    let yard = cx.r.anchor.kind == AnchorKind::Yard;
    let wall = |cx: &Cx, y: i32| yard || (-2..=1).all(|z| cx.wall(IVec3::new(-1, y, z)));
    let fronds = |y: i32| (IVec3::new(0, y * 4, za - 1), IVec3::new(deep * 4 + 1, y * 4 + 4, zb + 1));
    let y = (5..=6).rev().find(|&y| wall(cx, y) && cx.room(fronds(y).0, fronds(y).1) && cx.stands(deep - 1, -2, 0, 1) == Some(0))?;
    let p = *cx.pal();
    let top = y * 4 + 1;
    let slope = if yard { 0 } else { 1 };
    let poles: &[i32] = if yard { &[0, deep * 4 - 1] } else { &[deep * 4 - 1] };
    for &x in poles {
        for z in [za, zb - 1] {
            cx.part(IVec3::new(x, 0, z), IVec3::new(x + 1, top - slope * deep, z + 1), p.wood);
        }
    }
    // One colour to a shade, as a thatched roof has: fresh green fronds, or dried brown
    // where it was laid long ago and left.
    let v = if cx.decayed() { mat::PALM_THATCH } else { mat::LEAVES_PALM };
    for x in 0..deep {
        let yy = top - slope * x;
        for z in (za - 1..zb + 1).step_by(3) {
            cx.part(IVec3::new(x * 4, yy, z), IVec3::new(x * 4 + 4 + 1, yy + 1, (z + 3).min(zb + 1)), v);
        }
    }
    let front = top - slope * (deep - 1);
    for z in (za..zb).step_by(2) {
        let drop = 2 + cx.rng.int(0, 2);
        cx.plant(voxel::mesh::PlantBox { lo: [deep * 4, front - drop, z], hi: [deep * 4 + 1, front, z + 1], voxel: v, sway_lo: true, sway_hi: false, thin: 0, season: 0, flap: 0 });
    }
    Some(())
}

/// The door's jambs and lintel painted: blue or green where the household is well off,
/// ochre where not, flaking where it is let go.
pub(super) fn door_frame(cx: &mut Cx) -> Option<()> {
    let (z0, z1, h) = cx.doorway()?;
    let colour = if cx.wealth() > 0.45 { [mat::PAINT_BLUE, mat::PAINT_GREEN, mat::PAINT_BLUE][(cx.rng.next_u32() % 3) as usize] } else { mat::PAINT_OCHRE };
    let mut painted = 0;
    let mut frame: Vec<IVec3> = (0..h).flat_map(|y| [IVec3::new(-1, y, z0 - 1), IVec3::new(-1, y, z1 + 1)]).collect();
    frame.extend((z0 - 1..=z1 + 1).map(|z| IVec3::new(-1, h, z)));
    for q in frame {
        if cx.wall(q) && !(cx.decayed() && cx.rng.unit() < 0.35) {
            cx.paint(q, colour);
            painted += 1;
        }
    }
    (painted >= 3).then_some(())
}

/// A wind catcher on a flat roof, as great towns build them: a hollow tower with vents at
/// the top on every side, which turns any breeze down into the rooms below.
pub(super) fn wind_catcher(cx: &mut Cx) -> Option<()> {
    if !cx.r.great_town || cx.r.style.shape != RoofShape::Flat || !cx.wall(IVec3::new(0, -1, 0)) {
        return None;
    }
    let tall = 5 + (cx.wealth() > 0.7) as i32;
    let p = *cx.pal();
    for y in 0..=tall {
        for x in -1..=1 {
            for z in -1..=1 {
                let middle = x == 0 && z == 0;
                let vent = y >= tall - 2 && y < tall && (x == 0 || z == 0);
                if (middle && y < tall) || vent {
                    continue;
                }
                let v = if y == tall { mat::MUD_PLASTER } else { mat::MUD_BRICK };
                if !cx.put_free(IVec3::new(x, y, z), v) {
                    return None;
                }
            }
        }
    }
    // Slats across each vent.
    for (x, z) in [(0, -1), (0, 1), (-1, 0), (1, 0)] {
        for k in 0..3 {
            let y = (tall - 2) * 4 + 1 + k * 3;
            let (lo, hi) = if x == 0 { (IVec3::new(0, y, z * 4 + 1), IVec3::new(4, y + 1, z * 4 + 3)) } else { (IVec3::new(x * 4 + 1, y, 0), IVec3::new(x * 4 + 3, y + 1, 4)) };
            cx.part(lo, hi, p.wood);
        }
    }
    Some(())
}

/// A beehive oven of clay in the yard, its mouth away from the house and ash before it.
pub(super) fn oven(cx: &mut Cx) -> Option<()> {
    if cx.level(0..=2, -1..=1, 0, 1) != Some(0) || !fits_box(cx, IVec3::new(0, 0, -1), IVec3::new(2, 1, 1)) {
        return None;
    }
    let shell = if cx.decayed() { mat::MUD_BRICK } else { mat::CLAY };
    for x in 0..=2 {
        for z in -1..=1 {
            if (x, z) != (1, 0) && (x, z) != (2, 0) {
                cx.put(IVec3::new(x, 0, z), shell);
            }
        }
    }
    for (x, z) in [(0, 0), (1, -1), (1, 1), (1, 0)] {
        if !(cx.decayed() && (x, z) == (1, 0)) {
            cx.put(IVec3::new(x, 1, z), shell);
        }
    }
    cx.part(IVec3::new(5, 0, 1), IVec3::new(11, 1, 3), mat::CHARCOAL);
    Some(())
}

/// Mats on a flat roof with dates and peppers spread on them to dry, and a basket.
pub(super) fn drying_mats(cx: &mut Cx) -> Option<()> {
    if cx.r.style.shape != RoofShape::Flat || !cx.wall(IVec3::new(0, -1, 0)) {
        return None;
    }
    let mut laid = 0;
    let n = 2 + (cx.wealth() > 0.5) as i32;
    for k in 0..6 {
        if laid >= n {
            break;
        }
        let (x, z) = (cx.rng.int(-3, 1) * 4 + (k & 1) * 2, cx.rng.int(-3, 1) * 4);
        let (lo, hi) = (IVec3::new(x, 0, z), IVec3::new(x + 8, 2, z + 8));
        if !cx.room(lo, hi) || cx.stands(x.div_euclid(4), z.div_euclid(4), 0, 0) != Some(0) || cx.stands((x + 7).div_euclid(4), (z + 7).div_euclid(4), 0, 0) != Some(0) {
            continue;
        }
        cx.part(lo, IVec3::new(x + 8, 1, z + 8), mat::WICKER);
        let crop = [mat::DRIED_RED, mat::DRIED_BROWN, mat::DRIED_GOLD][(cx.rng.next_u32() % 3) as usize];
        for _ in 0..8 {
            let (a, b) = (cx.rng.int(0, 6), cx.rng.int(0, 6));
            cx.part(IVec3::new(x + a, 1, z + b), IVec3::new(x + a + 1, 2, z + b + 1), crop);
        }
        laid += 1;
    }
    (laid > 0).then_some(())
}
