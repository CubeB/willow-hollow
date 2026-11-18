//! Temperate lowland and the upland moors: porches, window boxes, shutters, roses and
//! ivy, a dovecote; and on the moor, peat, drystone, and stones holding the thatch down.

use glam::IVec3;

use super::{
    Cx,
    common::{fits_box, spots},
};
use crate::{
    features::plants::Flower,
    materials as mat,
    settlements::{
        Region,
        kit::{AnchorKind, Cover},
    },
};

/// A porch before the door: a roof over the way in on two timber posts, roofed as the
/// house is. Walled (`walled`, the moor's way), its sides are drystone against the wind.
pub(super) fn porch(cx: &mut Cx, walled: bool) -> Option<()> {
    let (z0, z1, h) = cx.doorway()?;
    let deep = 2 + (cx.wealth() > 0.6) as i32;
    let top = h.max(5);
    let (za, zb) = (z0 - 2, z1 + 2);
    let down = top - 1 - cx.ground();
    let (p, cover) = (*cx.pal(), cx.roofing());
    // The sides first, as the roof rests on them: a post at each front corner, or a
    // wall the depth of the porch.
    for z in [za, zb] {
        // A wall starts a voxel out where the door's leaf stands open against the house.
        let from = if cx.room(IVec3::new(0, 0, z * 4), IVec3::new(4, top * 4, z * 4 + 4)) { 0 } else { 1 };
        let xs = if walled { from..deep } else { deep - 1..deep };
        for x in xs {
            let base = cx.stands(x, z, top - 1, down)?;
            for y in base..top {
                if !cx.put_free(IVec3::new(x, y, z), if walled { p.stone } else { p.wood }) {
                    return None;
                }
            }
        }
    }
    // The roof over it, where the house leaves room: under deep eaves it is the house's.
    for x in 0..deep {
        for z in za..=zb {
            cx.put_free(IVec3::new(x, top, z), cover);
        }
    }
    if !walled {
        // Braces from each post up under the roof.
        for z in [za, zb] {
            let c = IVec3::new((deep - 1) * 4, top * 4, z * 4);
            cx.part(c + IVec3::new(-3, -2, 1), c + IVec3::new(0, -1, 3), p.wood);
        }
    }
    Some(())
}

/// The flowers grown in window boxes in a region.
fn box_flowers(region: Region) -> &'static [Flower] {
    match region {
        Region::Alpine => &[Flower::Gentian, Flower::Edelweiss, Flower::Campion, Flower::Poppy],
        Region::Desert => &[Flower::DesertMarigold, Flower::Poppy],
        Region::Upland | Region::Northern => &[Flower::Heather, Flower::Campion],
        _ => &[Flower::Campion, Flower::Poppy, Flower::Daisy, Flower::Primrose, Flower::Foxglove],
    }
}

/// A box under a window, below its sill, full of flowers in their season, the fuller the
/// richer the household. Where nobody keeps it, it is empty but for dead stalks.
pub(super) fn window_box(cx: &mut Cx) -> Option<()> {
    let (z0, z1, _) = cx.window()?;
    if !cx.wall(IVec3::new(-1, -1, z0)) {
        return None;
    }
    let (za, zb) = (z0 * 4 - 1, (z1 + 1) * 4 + 1);
    if !cx.room(IVec3::new(0, -4, za), IVec3::new(4, -1, zb)) || !cx.room(IVec3::new(2, -1, za + 1), IVec3::new(5, 5, zb - 1)) {
        return None;
    }
    let p = *cx.pal();
    let wood = if cx.wealth() > 0.7 { mat::PAINT_GREEN } else { p.plank };
    cx.part(IVec3::new(0, -4, za), IVec3::new(4, -1, zb), wood);
    cx.part(IVec3::new(2, -1, za + 1), IVec3::new(4, 0, zb - 1), mat::DIRT);
    let flowers = box_flowers(cx.r.region);
    for z in (za + 1..zb - 1).step_by(2) {
        if cx.decayed() {
            if cx.rng.unit() < 0.3 {
                cx.part(IVec3::new(3, 0, z), IVec3::new(4, 2, z + 1), mat::DRIED_BROWN);
            }
            continue;
        }
        if cx.rng.unit() > 0.35 + cx.wealth() * 0.65 {
            continue;
        }
        let f = flowers[(cx.rng.next_u32() as usize) % flowers.len()];
        let tall = 2 + cx.rng.int(0, 2);
        let x = 2 + cx.rng.int(0, 1);
        cx.growth(IVec3::new(x, 0, z), IVec3::new(x + 1, tall, z + 1), mat::STEM, f.leaf_code());
        cx.growth(IVec3::new(x, tall, z), IVec3::new(x + 1, tall + 1, z + 1), f.petals(), f.bloom_code());
        if cx.rng.unit() < 0.3 {
            // Something trailing over the front.
            cx.growth(IVec3::new(4, -3, z), IVec3::new(5, 0, z + 1), mat::LEAF_GLOSS, f.leaf_code());
        }
    }
    Some(())
}

/// A dovecote high on a gable: a box of boards under its own little roof, with holes and
/// a perch below each.
pub(super) fn dovecote(cx: &mut Cx) -> Option<()> {
    if !(0..=1).all(|y| (-1..=0).all(|z| cx.wall(IVec3::new(-1, y, z)))) || !fits_box(cx, IVec3::new(0, 0, -1), IVec3::new(0, 1, 0)) || !cx.room(IVec3::new(0, 8, -6), IVec3::new(6, 10, 6)) || !cx.room(IVec3::new(4, 0, -4), IVec3::new(6, 8, 4)) {
        return None;
    }
    let p = *cx.pal();
    let body = if cx.wealth() > 0.6 { mat::WHITEWASH } else { p.plank };
    for y in 0..=1 {
        for z in -1..=0 {
            cx.put(IVec3::new(0, y, z), body);
        }
    }
    cx.part(IVec3::new(0, 8, -6), IVec3::new(6, 9, 6), mat::PLANK_DARK);
    cx.part(IVec3::new(0, 9, -2), IVec3::new(6, 10, 2), mat::PLANK_DARK);
    for (y, z) in [(1, -3), (1, 1), (5, -3), (5, 1)] {
        cx.part(IVec3::new(4, y + 1, z), IVec3::new(5, y + 3, z + 2), mat::CHARCOAL);
        cx.part(IVec3::new(4, y, z), IVec3::new(6, y + 1, z + 2), p.wood);
    }
    Some(())
}

/// A climber up the wall: a rose (`rose`), its stems and leaves bare in winter and in
/// flower in summer, by a door or along a wall; or ivy, evergreen, thick up a wall to the
/// eaves. It grows only where there is wall behind it, so never over a window.
pub(super) fn climber(cx: &mut Cx, rose: bool) -> Option<()> {
    let kind = cx.r.anchor.kind;
    let (zs, base, arch): (Vec<i32>, i32, Option<(i32, i32, i32)>) = match kind {
        AnchorKind::Door => {
            let (z0, z1, h) = cx.doorway()?;
            let side = [z0 - 1, z1 + 1].into_iter().find(|&z| cx.room(IVec3::new(0, 0, z * 4), IVec3::new(1, h * 4, z * 4 + 4)))?;
            (vec![side], 0, Some((z0, z1, h)))
        }
        AnchorKind::Corner => (vec![-3, -2, -1], 0, None),
        _ => ((-2..=2).collect(), 0, None),
    };
    let tall = if rose { 7 } else { 10 };
    let rose_flower = Flower::Campion;
    let mut grown = 0;
    for &z in &zs {
        let mut y = base;
        while y < base + tall && cx.wall(IVec3::new(-1, y, z)) && cx.open(IVec3::new(0, y, z)) {
            for k in 0..4 {
                let cy = y * 4 + k;
                // Thickest low down, thinning toward the top.
                let thick = 0.9 - (y - base) as f32 / tall as f32 * 0.5;
                if rose {
                    if (z + k) % 2 == 0 && cx.room(IVec3::new(0, cy, z * 4 + 1), IVec3::new(1, cy + 1, z * 4 + 2)) {
                        cx.part(IVec3::new(0, cy, z * 4 + 1), IVec3::new(1, cy + 1, z * 4 + 2), mat::STEM);
                        grown += 1;
                    }
                    let o = cx.rng.int(0, 3);
                    if cx.rng.unit() < thick * 0.6 && cx.room(IVec3::new(0, cy, z * 4 + o), IVec3::new(2, cy + 1, z * 4 + o + 1)) {
                        cx.growth(IVec3::new(0, cy, z * 4 + o), IVec3::new(1, cy + 1, z * 4 + o + 1), mat::LEAF_GLOSS, rose_flower.leaf_code());
                        if cx.rng.unit() < 0.35 {
                            let v = if cx.rng.unit() < 0.5 { mat::BLOOM_PINK } else { mat::BLOOM_RED };
                            cx.growth(IVec3::new(1, cy, z * 4 + o), IVec3::new(2, cy + 1, z * 4 + o + 1), v, rose_flower.bloom_code());
                        }
                        grown += 1;
                    }
                } else if k % 2 == 0 {
                    for o in [0, 2] {
                        if cx.rng.unit() < thick && cx.room(IVec3::new(0, cy, z * 4 + o), IVec3::new(1, cy + 2, z * 4 + o + 2)) {
                            cx.part(IVec3::new(0, cy, z * 4 + o), IVec3::new(1, cy + 2, z * 4 + o + 2), mat::IVY_GLOSS);
                            grown += 1;
                        }
                    }
                }
            }
            y += 1;
        }
    }
    // Over a door, a rose arches across under the lintel's top.
    if let Some((z0, z1, h)) = arch
        && rose
    {
        for z in z0 * 4..(z1 + 1) * 4 {
            if cx.wall(IVec3::new(-1, h, z.div_euclid(4))) && cx.rng.unit() < 0.6 && cx.room(IVec3::new(0, h * 4, z), IVec3::new(1, h * 4 + 2, z + 1)) {
                cx.growth(IVec3::new(0, h * 4, z), IVec3::new(1, h * 4 + 2, z + 1), mat::LEAF_GLOSS, rose_flower.leaf_code());
            }
        }
    }
    (grown >= 4).then_some(())
}

/// The kit's shutters painted, the house's colour for them; where the house is let go,
/// some hang askew from one hinge and some are gone.
pub(super) fn shutters(cx: &mut Cx) -> Option<()> {
    let (z0, z1, h) = cx.window()?;
    let colour = if cx.wealth() > 0.35 { [mat::PAINT_GREEN, mat::PAINT_BLUE, mat::PAINT_RED, mat::PAINT_OCHRE][(cx.rng.next_u32() % 4) as usize] } else { mat::PLANK_DARK };
    let mut found = 0;
    for z in [z0 - 1, z1 + 1] {
        for i in cx.parts_within(IVec3::new(0, 0, z * 4), IVec3::new(1, h * 4, z * 4 + 4)) {
            found += 1;
            if cx.decayed() && cx.rng.unit() < 0.6 {
                cx.refit(i, None);
                if cx.rng.unit() < 0.6 && cx.room(IVec3::new(0, -3, z * 4), IVec3::new(1, h * 4 - 3, z * 4 + 4)) {
                    cx.part(IVec3::new(0, -3, z * 4), IVec3::new(1, h * 4 - 3, z * 4 + 4), mat::PLANK_DARK);
                }
            } else {
                cx.refit(i, Some(colour));
                for y in [1, h * 4 - 2] {
                    cx.part(IVec3::new(1, y, z * 4 + 1), IVec3::new(2, y + 1, z * 4 + 3), mat::IRON);
                }
            }
        }
    }
    (found > 0).then_some(())
}

/// Ropes over the thatch, each weighted with a stone hanging at the eaves.
pub(super) fn thatch_weights(cx: &mut Cx) -> Option<()> {
    if !matches!(cx.r.style.roof, Cover::Thatch | Cover::Reed | Cover::Turf | Cover::Palm) {
        return None;
    }
    let stone = cx.pal().stone;
    let mut hung = 0;
    for z in -1..=1 {
        let Some((x, under)) = cx.eaves_edge(z) else { continue };
        let long = 3 + cx.rng.int(0, 3);
        let (c, top) = (IVec3::new(x * 4, 0, z * 4), under * 4);
        if !cx.room(c + IVec3::new(0, top - long - 3, 0), c + IVec3::new(3, top, 3)) {
            continue;
        }
        let rope = if cx.decayed() { mat::ROPE_ROTTEN } else { mat::ROPE };
        cx.part(c + IVec3::new(1, top - long, 1), c + IVec3::new(2, top, 2), rope);
        cx.part(c + IVec3::new(0, top - long - 3, 0), c + IVec3::new(3, top - long, 3), stone);
        hung += 1;
    }
    (hung > 0).then_some(())
}

/// Cut peat stacked to dry, three long and stepping in at the top, a turf over it.
pub(super) fn peat_stack(cx: &mut Cx) -> Option<()> {
    let (y, z) = spots(cx, 3, 2).into_iter().find(|&(y, z)| fits_box(cx, IVec3::new(0, y, z), IVec3::new(1, y + 2, z + 2)))?;
    for (x, yy) in [(0, 0), (1, 0), (0, 1), (1, 1), (0, 2)] {
        for zz in z..z + 3 {
            if !(cx.decayed() && yy == 2 && zz == z + 2) {
                cx.put(IVec3::new(x, y + yy, zz), mat::PEAT);
            }
        }
    }
    cx.part(IVec3::new(0, (y + 3) * 4, z * 4 + 1), IVec3::new(4, (y + 3) * 4 + 1, (z + 3) * 4 - 1), mat::TURF);
    Some(())
}

/// A curve of drystone between the yard and the weather, lower at its ends; let go,
/// there is a gap in it and stones lie at its foot.
pub(super) fn lee_wall(cx: &mut Cx) -> Option<()> {
    let stone = cx.pal().stone;
    let gap = if cx.decayed() { cx.rng.int(-1, 1) } else { 99 };
    let mut cols = Vec::new();
    for z in -4..=4 {
        let x = 2 - z * z / 6;
        let tall = if z == -4 || z == 4 { 1 } else { 2 };
        if cx.stands(x, z, 0, 1) != Some(0) || !(0..tall).all(|y| cx.free(IVec3::new(x, y, z))) {
            return None;
        }
        cols.push((x, z, tall));
    }
    for (x, z, tall) in cols {
        if (z - gap).abs() <= 0 {
            cx.part(IVec3::new(x * 4 + 4, 0, z * 4), IVec3::new(x * 4 + 7, 2, z * 4 + 3), stone);
            continue;
        }
        for y in 0..tall {
            cx.put(IVec3::new(x, y, z), stone);
        }
        cx.part(IVec3::new(x * 4, tall * 4, z * 4), IVec3::new(x * 4 + 4, tall * 4 + 1, z * 4 + 4), mat::TURF);
    }
    Some(())
}

/// A bench of stone against the wall with turf on its seat.
pub(super) fn turf_bench(cx: &mut Cx) -> Option<()> {
    let (y, z) = spots(cx, 3, 1).into_iter().find(|&(y, z)| fits_box(cx, IVec3::new(0, y, z), IVec3::new(0, y + 1, z + 2)))?;
    let stone = cx.pal().stone;
    for zz in z..z + 3 {
        cx.put(IVec3::new(0, y, zz), stone);
    }
    cx.part(IVec3::new(0, (y + 1) * 4, z * 4), IVec3::new(4, (y + 1) * 4 + 1, (z + 3) * 4), mat::TURF);
    for k in 0..4 {
        let zz = z * 4 + 1 + k * 3;
        cx.growth(IVec3::new(1 + k % 2, (y + 1) * 4 + 1, zz), IVec3::new(2 + k % 2, (y + 1) * 4 + 3, zz + 1), mat::TUFT, 0);
    }
    Some(())
}

/// A small fold of drystone for sheep, its gate away from the house closed by a hurdle.
pub(super) fn sheep_fold(cx: &mut Cx) -> Option<()> {
    let (x1, z0, z1) = (5, -2, 2);
    let mut ring = Vec::new();
    for x in 0..=x1 {
        for z in z0..=z1 {
            let edge = x == 0 || x == x1 || z == z0 || z == z1;
            if !edge {
                continue;
            }
            if cx.stands(x, z, 0, 1) != Some(0) || !(0..2).all(|y| cx.free(IVec3::new(x, y, z))) {
                return None;
            }
            if (x, z) != (x1, 0) {
                ring.push((x, z));
            }
        }
    }
    let stone = cx.pal().stone;
    for (x, z) in ring {
        let tall = if cx.decayed() && cx.rng.unit() < 0.3 { 1 } else { 2 };
        for y in 0..tall {
            cx.put(IVec3::new(x, y, z), stone);
        }
    }
    cx.part(IVec3::new(x1 * 4 + 1, 0, 0), IVec3::new(x1 * 4 + 2, 6, 4), mat::WITHY);
    Some(())
}
