//! Marsh, mangrove and coast; the rainforest; the old forest: boardwalks and rope rails,
//! reed and nets and boats; woven screens, hammocks and carved posts; lanterns, baskets
//! and fungus-grown logs.

use glam::IVec3;
use voxel::mesh::PlantBox;

use super::{
    Cx,
    common::{fits_box, spots},
};
use crate::{
    features::plants::Flower,
    materials as mat,
    settlements::{Region, kit::AnchorKind},
};

/// Boards laid across the way on from the foot of the door's steps (or from the door, on
/// the ground) out over the wet, as far as they go, on stakes along their sides. Boards
/// are fine parts, so they never stand in anyone's way; let go, some are rotten black and
/// some are gone.
pub(super) fn boardwalk(cx: &mut Cx) -> Option<()> {
    let (z0, z1, _) = cx.doorway()?;
    let (g, deep) = (cx.ground(), (-cx.ground()).max(1));
    let mid = (z0 + z1) / 2;
    let foot = (0..14).find(|&x| cx.stands(x, mid, 0, deep) == Some(g))?;
    let long = 5 + cx.rng.int(0, 3);
    let p = *cx.pal();
    let (za, zb) = (z0 * 4 - 2, (z1 + 1) * 4 + 2);
    let mut laid = 0;
    for x in foot..foot + long {
        let (lo, hi) = (IVec3::new(x * 4, g * 4, za), IVec3::new(x * 4 + 3, g * 4 + 1, zb));
        if !(z0..=z1).all(|z| cx.stands(x, z, g, 0) == Some(g)) || !cx.room(lo, hi) {
            break;
        }
        laid += 1;
        let roll = cx.rng.unit();
        if cx.decayed() && roll < 0.15 {
            continue;
        }
        cx.part(lo, hi, if cx.decayed() && roll < 0.45 { mat::PLANK_DARK } else { p.plank });
        // A stake at each side every other board, its top above the boards.
        if (x - foot) % 2 == 0 {
            for z in [za - 2, zb] {
                let (lo, hi) = (IVec3::new(x * 4 + 1, g * 4, z), IVec3::new(x * 4 + 3, g * 4 + 5, z + 2));
                if cx.room(lo, hi) {
                    cx.part(lo, hi, p.wood);
                }
            }
        }
    }
    (laid >= 4).then_some(())
}

/// Rope rails on posts either side of the platform and steps before the door, as far
/// as they go out and down.
pub(super) fn rope_rail(cx: &mut Cx) -> Option<()> {
    let (z0, z1, _) = cx.doorway()?;
    let p = *cx.pal();
    let rope = if cx.decayed() { mat::ROPE_ROTTEN } else { mat::ROPE };
    let deep = (-cx.ground()).max(1);
    let mut posts = 0;
    // Each side's posts stand just beside the way, at the edge of the column next to it.
    for (zc, inner) in [(z0 * 4 - 1, z0), ((z1 + 1) * 4, z1)] {
        let vz = zc.div_euclid(4);
        let mut tops: Vec<IVec3> = Vec::new();
        for x in (1..12).step_by(2) {
            let Some(base) = cx.stands(x, vz, 0, deep) else { break };
            // Waist high over the way where the post stands, the platform or a step.
            let tread = cx.stands(x, inner, 0, deep).unwrap_or(base).max(base);
            let top = tread * 4 + 8;
            let (lo, hi) = (IVec3::new(x * 4 + 1, base * 4, zc), IVec3::new(x * 4 + 2, top + 1, zc + 1));
            if !cx.room(lo, hi) {
                break;
            }
            cx.part(lo, hi, p.wood);
            tops.push(IVec3::new(x * 4 + 1, top, zc));
            if tread <= cx.ground() {
                break;
            }
        }
        // The rope from post to post, sagging between them, and running down with the
        // steps without a gap.
        let mut rope_parts = Vec::new();
        for w in tops.windows(2) {
            let (a, b) = (w[0], w[1]);
            let mut was = a.y;
            for xc in a.x + 1..b.x {
                let t = (xc - a.x) as f32 / (b.x - a.x) as f32;
                let y = (a.y as f32 + (b.y - a.y) as f32 * t - 4.0 * t * (1.0 - t)).round() as i32;
                rope_parts.push((IVec3::new(xc, y.min(was), zc), IVec3::new(xc + 1, y.max(was) + 1, zc + 1)));
                was = y;
            }
        }
        if !rope_parts.iter().all(|&(lo, hi)| cx.room(lo, hi)) {
            return None;
        }
        for (lo, hi) in rope_parts {
            cx.part(lo, hi, rope);
        }
        posts += tops.len();
    }
    (posts >= 4).then_some(())
}

/// A screen of reeds bound in two places, standing a little out from the wall or across
/// the yard.
pub(super) fn reed_screen(cx: &mut Cx) -> Option<()> {
    let x = if cx.r.anchor.kind == AnchorKind::WallFace { 1 } else { 0 };
    if !(-2..=2).all(|z| cx.stands(x, z, 0, 1) == Some(0)) || !cx.room(IVec3::new(x * 4, 0, -8), IVec3::new(x * 4 + 2, 17, 12)) {
        return None;
    }
    for zc in -8..12 {
        if cx.decayed() && cx.rng.unit() < 0.2 {
            continue;
        }
        let tall = 13 + cx.rng.int(0, 3);
        cx.part(IVec3::new(x * 4, 0, zc), IVec3::new(x * 4 + 1, tall, zc + 1), mat::REED);
    }
    for y in [4, 10] {
        cx.part(IVec3::new(x * 4 + 1, y, -8), IVec3::new(x * 4 + 2, y + 1, 12), mat::ROPE);
    }
    Some(())
}

/// Nets hung from the edge of the eaves to dry, with floats along their foot.
pub(super) fn drying_nets(cx: &mut Cx) -> Option<()> {
    let mut hung = 0;
    for z in [-1, 1] {
        let Some((x, under)) = cx.eaves_edge(z) else { continue };
        let (top, drop) = (under * 4, 8 + cx.rng.int(0, 4));
        let (lo, hi) = (IVec3::new(x * 4 + 1, top - drop - 1, z * 4 - 2), IVec3::new(x * 4 + 2, top, z * 4 + 6));
        if !cx.room(lo, hi) {
            continue;
        }
        cx.plant(PlantBox { lo: [lo.x, lo.y + 1, lo.z], hi: hi.to_array(), voxel: mat::NET, sway_lo: true, sway_hi: false, thin: 0, season: 0, flap: 0 });
        for zc in (lo.z..hi.z).step_by(3) {
            cx.part(IVec3::new(lo.x, lo.y, zc), IVec3::new(lo.x + 1, lo.y + 1, zc + 1), mat::PLANK_PALE);
        }
        hung += 1;
    }
    (hung > 0).then_some(())
}

/// A flat-bottomed punt drawn up in the yard, its pole lying in it; let go, it has holes.
pub(super) fn punt(cx: &mut Cx) -> Option<()> {
    if cx.level(0..=1, -3..=3, 0, 1) != Some(0) || !fits_box(cx, IVec3::new(0, 0, -3), IVec3::new(1, 0, 3)) || !cx.room(IVec3::new(0, 4, -13), IVec3::new(8, 8, 17)) {
        return None;
    }
    let hull = if cx.wealth() > 0.5 { cx.pal().plank } else { mat::PLANK_DARK };
    for x in 0..=1 {
        for z in -3..=3i32 {
            if !(cx.decayed() && z.abs() < 2 && cx.rng.unit() < 0.3) {
                cx.put(IVec3::new(x, 0, z), hull);
            }
        }
    }
    for x in [0, 7] {
        cx.part(IVec3::new(x, 4, -12), IVec3::new(x + 1, 7, 16), hull);
    }
    cx.part(IVec3::new(0, 4, -13), IVec3::new(8, 8, -12), hull);
    cx.part(IVec3::new(0, 4, 16), IVec3::new(8, 8, 17), hull);
    if cx.wall(IVec3::new(0, 0, -1)) || !cx.decayed() {
        cx.part(IVec3::new(3, 4, -11), IVec3::new(4, 5, 15), cx.pal().wood);
    }
    Some(())
}

/// A fence of driftwood stakes of every height, with two rails along it; let go, some
/// stakes are gone.
pub(super) fn driftwood_fence(cx: &mut Cx) -> Option<()> {
    if !(-4..=4).all(|z| cx.stands(1, z, 0, 1) == Some(0)) || !cx.room(IVec3::new(4, 0, -16), IVec3::new(7, 15, 20)) {
        return None;
    }
    for zc in (-16..19).step_by(3) {
        if cx.decayed() && cx.rng.unit() < 0.25 && zc > -16 {
            continue;
        }
        let tall = 11 + cx.rng.int(0, 3);
        cx.part(IVec3::new(4, 0, zc), IVec3::new(6, tall, zc + 2), mat::WOOD_DEAD);
    }
    for y in [4, 9] {
        cx.part(IVec3::new(6, y, -16), IVec3::new(7, y + 1, 20), mat::WOOD_DEAD);
    }
    Some(())
}

/// An old boat turned over for a shed, its stern closed with planks and its bow open.
pub(super) fn upturned_boat(cx: &mut Cx) -> Option<()> {
    if cx.level(0..=4, -4..=4, 0, 1) != Some(0) {
        return None;
    }
    let mut hull = Vec::new();
    for z in -4..=4 {
        let rows: &[(i32, &[i32])] = &[(0, &[-2, 2]), (1, &[-2, 2]), (2, &[-2, -1, 1, 2]), (3, &[-1, 0, 1])];
        for &(y, dxs) in rows {
            if z == -4 && y < 3 {
                continue;
            }
            for &dx in dxs {
                hull.push(IVec3::new(2 + dx, y, z));
            }
        }
        if z == 4 {
            for y in 0..=2 {
                for dx in -1..=1 {
                    if !(y == 2 && dx != 0) {
                        hull.push(IVec3::new(2 + dx, y, z));
                    }
                }
            }
        }
    }
    hull.sort_unstable_by_key(|q| (q.y, q.x, q.z));
    hull.dedup();
    if !hull.iter().all(|&q| cx.free(q)) {
        return None;
    }
    let plank = if cx.decayed() { mat::WOOD_DEAD } else { mat::PLANK_DARK };
    for q in hull {
        cx.put(q, plank);
    }
    Some(())
}

/// Sheaves of cut reed stood up to dry, tied round the middle.
pub(super) fn reed_stack(cx: &mut Cx) -> Option<()> {
    let (y, z) = spots(cx, 3, 1).into_iter().find(|&(y, z)| fits_box(cx, IVec3::new(0, y, z), IVec3::new(0, y + 3, z + 2)))?;
    for k in 0..5 {
        let zc = z * 4 + 1 + k * 2;
        let tall = 14 - (k % 2) * 2;
        cx.part(IVec3::new(0, y * 4, zc), IVec3::new(3, y * 4 + tall, zc + 2), mat::REED);
        cx.part(IVec3::new(3, y * 4 + 6, zc), IVec3::new(4, y * 4 + 7, zc + 2), mat::ROPE);
    }
    Some(())
}

/// A hammock slung between two posts: a sheet of cloth, or of net for the poor, that
/// sags in still air and sways in a breeze.
pub(super) fn hammock(cx: &mut Cx) -> Option<()> {
    if ![-3, 3].iter().all(|&z| cx.stands(0, z, 0, 1) == Some(0) && (0..=4).all(|y| cx.free(IVec3::new(0, y, z)))) || !cx.room(IVec3::new(0, 8, -8), IVec3::new(4, 17, 12)) {
        return None;
    }
    let p = *cx.pal();
    for z in [-3, 3] {
        for y in 0..=4 {
            cx.put(IVec3::new(0, y, z), p.wood);
        }
    }
    let colour = if cx.wealth() < 0.35 { mat::NET } else { cx.cloth() };
    // Slung from the tops of the posts and sagging to the middle, in two halves.
    let sag = if cx.decayed() { 4 } else { 6 };
    let (end_a, end_b, low) = (-8, 12, 2);
    cx.sheet([IVec3::new(0, 16, end_a), IVec3::new(4, 16, end_a), IVec3::new(4, 16 - sag, low), IVec3::new(0, 16 - sag, low)], colour);
    cx.sheet([IVec3::new(0, 16 - sag, low), IVec3::new(4, 16 - sag, low), IVec3::new(4, 16, end_b), IVec3::new(0, 16, end_b)], colour);
    Some(())
}

/// A screen woven of withy hung over a window from a bar above it; let go, it is rolled
/// half up and left.
pub(super) fn woven_screen(cx: &mut Cx) -> Option<()> {
    let (z0, z1, top) = cx.window()?;
    if !(z0..=z1).all(|z| cx.wall(IVec3::new(-1, top, z))) {
        return None;
    }
    let (za, zb) = (z0 * 4, (z1 + 1) * 4);
    let drop = if cx.decayed() { top * 2 } else { top * 4 };
    if !cx.room(IVec3::new(0, top * 4, za - 2), IVec3::new(1, top * 4 + 1, zb + 2)) || !cx.room(IVec3::new(1, top * 4 - drop, za), IVec3::new(2, top * 4, zb)) {
        return None;
    }
    cx.part(IVec3::new(0, top * 4, za - 2), IVec3::new(1, top * 4 + 1, zb + 2), cx.pal().wood);
    for y in top * 4 - drop..top * 4 {
        let v = if y % 2 == 0 { mat::WICKER } else { mat::WITHY };
        cx.part(IVec3::new(1, y, za), IVec3::new(2, y + 1, zb), v);
    }
    Some(())
}

/// Carved posts either side of the door, as tall as its lintel or more: bands cut round
/// them, a face near the top, and a cap.
pub(super) fn carved_posts(cx: &mut Cx) -> Option<()> {
    let (z0, z1, h) = cx.doorway()?;
    let deep = (-cx.ground()).max(1);
    let p = *cx.pal();
    let colour = if cx.wealth() > 0.6 { mat::PAINT_RED } else { p.accent };
    let mut posts = Vec::new();
    for z in [z0 - 2, z1 + 2] {
        let base = cx.stands(1, z, 0, deep)?;
        let c = IVec3::new(4, 0, z * 4);
        let cap = |y: i32| (c + IVec3::new(-1, y * 4 + 4, -1), c + IVec3::new(5, y * 4 + 6, 5));
        // As high as it is open, under the roof and with room for its cap.
        let mut top = (base..=h + 1).take_while(|&y| cx.free(IVec3::new(1, y, z))).last()?;
        while top > base && !cx.room(cap(top).0, cap(top).1) {
            top -= 1;
        }
        if top - base < 3 || top < h - 2 {
            return None;
        }
        posts.push((z, base, top, cap(top)));
    }
    for (z, base, top, (cap_lo, cap_hi)) in posts {
        for y in base..=top {
            cx.put(IVec3::new(1, y, z), p.wood);
        }
        let c = IVec3::new(4, 0, z * 4);
        // The bands, on each side that is open, and the face looking out.
        let cut = |cx: &mut Cx, lo: IVec3, hi: IVec3, v| {
            if cx.room(c + lo, c + hi) {
                cx.part(c + lo, c + hi, v);
            }
        };
        for y in (base + 1..top).step_by(3) {
            let yc = y * 4 + 1;
            cut(cx, IVec3::new(4, yc, 0), IVec3::new(5, yc + 1, 4), colour);
            cut(cx, IVec3::new(-1, yc, 0), IVec3::new(0, yc + 1, 4), colour);
            cut(cx, IVec3::new(0, yc, -1), IVec3::new(4, yc + 1, 0), colour);
            cut(cx, IVec3::new(0, yc, 4), IVec3::new(4, yc + 1, 5), colour);
        }
        let ty = top * 4;
        cut(cx, IVec3::new(4, ty, 1), IVec3::new(5, ty + 1, 2), mat::CHARCOAL);
        cut(cx, IVec3::new(4, ty, 2), IVec3::new(5, ty + 1, 3), mat::CHARCOAL);
        cut(cx, IVec3::new(4, ty - 2, 1), IVec3::new(6, ty - 1, 3), colour);
        cx.part(cap_lo, cap_hi, colour);
    }
    Some(())
}

/// A lantern (`lantern`), lit unless the house is let go, or a basket of trailing plants,
/// hung on a chain or a rope from the roof's edge.
pub(super) fn hanging(cx: &mut Cx, lantern: bool) -> Option<()> {
    let (x, under) = cx.eaves_edge(0)?;
    let x = x.min(1);
    let long = 3 + cx.rng.int(0, 2);
    let (top, c) = (under * 4, IVec3::new(x * 4, 0, 0));
    if !cx.room(c + IVec3::new(0, top - long - 8, 0), c + IVec3::new(4, top, 4)) {
        return None;
    }
    let p = *cx.pal();
    let low = top - long;
    if lantern {
        cx.part(c + IVec3::new(1, low, 1), c + IVec3::new(2, top, 2), mat::IRON);
        cx.part(c + IVec3::new(0, low - 1, 0), c + IVec3::new(3, low, 3), mat::IRON);
        let glass = if cx.decayed() { mat::LANTERN_COLD } else { mat::LANTERN };
        cx.part(c + IVec3::new(0, low - 4, 0), c + IVec3::new(3, low - 1, 3), glass);
        cx.part(c + IVec3::new(0, low - 5, 0), c + IVec3::new(3, low - 4, 3), p.wood);
    } else {
        cx.part(c + IVec3::new(1, low, 1), c + IVec3::new(2, top, 2), mat::ROPE);
        cx.part(c + IVec3::new(0, low - 3, 0), c + IVec3::new(4, low, 4), mat::WICKER);
        let f = if cx.r.region == Region::Rainforest { Flower::Orchid } else { Flower::Campion };
        if !cx.decayed() {
            cx.growth(c + IVec3::new(0, low, 0), c + IVec3::new(4, low + 1, 1), mat::LEAF_GLOSS, 0);
            cx.growth(c + IVec3::new(0, low, 3), c + IVec3::new(4, low + 1, 4), mat::LEAF_GLOSS, 0);
            cx.growth(c + IVec3::new(1, low, 1), c + IVec3::new(2, low + 1, 2), f.petals(), f.bloom_code());
            for (dx, dz) in [(0, 0), (3, 3)] {
                cx.plant(PlantBox { lo: (c + IVec3::new(dx, low - 7, dz)).to_array(), hi: (c + IVec3::new(dx + 1, low - 3, dz + 1)).to_array(), voxel: mat::LEAF_GLOSS, sway_lo: true, sway_hi: false, thin: 0, season: 0, flap: 0 });
            }
        }
    }
    Some(())
}

/// Old logs laid by the wall or in the yard with shelf fungi on their sides, and in
/// autumn mushrooms on top.
pub(super) fn mushroom_logs(cx: &mut Cx) -> Option<()> {
    let (y, z) = spots(cx, 4, 1).into_iter().find(|&(y, z)| fits_box(cx, IVec3::new(0, y, z), IVec3::new(0, y + 1, z + 3)))?;
    let p = *cx.pal();
    for zz in z..z + 4 {
        cx.put(IVec3::new(0, y, zz), p.wood);
    }
    if cx.wealth() > 0.4 {
        cx.put(IVec3::new(0, y + 1, z + 1), p.wood);
        cx.put(IVec3::new(0, y + 1, z + 2), p.wood);
    }
    for k in 0..5 {
        let zc = z * 4 + 1 + k * 3;
        let yc = y * 4 + 1 + cx.rng.int(0, 2);
        cx.part(IVec3::new(4, yc, zc), IVec3::new(6, yc + 1, zc + 2), mat::MUSHROOM_TAN);
    }
    let f = Flower::FlyAgaric;
    let top = (y + 1 + (cx.wealth() > 0.4) as i32) * 4;
    let zc = (z + 1) * 4 + 1;
    cx.growth(IVec3::new(1, top, zc), IVec3::new(2, top + 2, zc + 1), mat::MUSHROOM_STEM, f.bloom_code());
    cx.growth(IVec3::new(0, top + 2, zc - 1), IVec3::new(3, top + 3, zc + 2), f.petals(), f.bloom_code());
    cx.part(IVec3::new(0, y * 4 + 4, z * 4), IVec3::new(4, y * 4 + 5, z * 4 + 3), mat::MOSS_CARPET);
    Some(())
}
