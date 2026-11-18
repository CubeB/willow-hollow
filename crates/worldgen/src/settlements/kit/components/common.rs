//! Components found in more than one region: fittings set against a wall or in the yard,
//! a bench, a woodpile, what grows on a roof, and a banner.

use glam::IVec3;
use voxel::Voxel;

use super::Cx;
use crate::{
    features::{fittings::Fitting, plants::Flower},
    materials as mat,
    settlements::{
        Region,
        kit::{AnchorKind, Cover},
    },
};

/// Where something `w` voxels along the wall and `d` out from it stands against the wall
/// at the anchor: the layer it stands on and its lowest z, best first. Beside a door it
/// stands clear of the way in, either side; at a corner, along the wall from the corner;
/// elsewhere, at the middle of the stretch.
pub(super) fn spots(cx: &Cx, w: i32, d: i32) -> Vec<(i32, i32)> {
    let deep = (-cx.ground()).max(2);
    let zs: Vec<i32> = match cx.r.anchor.kind {
        AnchorKind::Door => match cx.doorway() {
            Some((z0, z1, _)) => vec![z1 + 2, z0 - 1 - w, z1 + 3, z0 - 2 - w],
            None => Vec::new(),
        },
        AnchorKind::Corner => vec![-(w - 1), -w],
        _ => vec![-(w - 1) / 2, -(w - 1) / 2 - 1, -(w - 1) / 2 + 1],
    };
    zs.into_iter().filter_map(|z| Some((cx.level(0..=d - 1, z..=z + w - 1, 0, deep)?, z))).collect()
}

/// Whether an inclusive box of voxels of the frame can take a component whole.
pub(super) fn fits_box(cx: &Cx, lo: IVec3, hi: IVec3) -> bool {
    (lo.y..=hi.y).all(|y| (lo.z..=hi.z).all(|z| (lo.x..=hi.x).all(|x| cx.free(IVec3::new(x, y, z))))) && cx.room(lo * 4, (hi + 1) * 4)
}

/// Sets a fitting with its back to the wall (x = -1) and its front out, on layer `y`,
/// from `z` along the wall, if it has room.
fn set_fitting(cx: &mut Cx, kind: Fitting, y: i32, z: i32) -> bool {
    let s = kind.local(cx.r.region).size();
    if !fits_box(cx, IVec3::new(0, y, z), IVec3::new(s.z - 1, y + s.y - 1, z + s.x - 1)) {
        return false;
    }
    // Three turns face a fitting's front, +Z as it is built, out along +X.
    cx.fitting(kind, IVec3::new(0, y, z + s.x - 1), 3);
    true
}

/// Up to `n` of a fitting against the wall (more than one only for the better off).
pub(super) fn against_wall(cx: &mut Cx, kind: Fitting, n: usize) -> Option<()> {
    let s = kind.local(cx.r.region).size();
    let n = if cx.wealth() > 0.5 { n } else { 1 };
    let mut set = 0;
    for (y, z) in spots(cx, s.x, s.z) {
        if set < n && set_fitting(cx, kind, y, z) {
            set += 1;
        }
    }
    (set > 0).then_some(())
}

/// A fitting hung under the eaves, its top against the roof.
pub(super) fn hung(cx: &mut Cx, kind: Fitting) -> Option<()> {
    let s = kind.local(cx.r.region).size();
    let z = -(s.x - 1) / 2;
    set_fitting(cx, kind, 1 - s.y, z).then_some(())
}

/// Fittings in a row in the yard, a voxel apart, all or none.
pub(super) fn in_yard(cx: &mut Cx, kinds: &[Fitting]) -> Option<()> {
    let sizes: Vec<IVec3> = kinds.iter().map(|k| k.local(cx.r.region).size()).collect();
    let long = sizes.iter().map(|s| s.x).sum::<i32>() + kinds.len() as i32 - 1;
    let deep = sizes.iter().map(|s| s.z).max()?;
    for (y, z0) in spots(cx, long, deep) {
        let mut z = z0;
        let all = sizes.iter().all(|s| {
            let ok = fits_box(cx, IVec3::new(0, y, z), IVec3::new(s.z - 1, y + s.y - 1, z + s.x - 1));
            z += s.x + 1;
            ok
        });
        if all {
            let mut z = z0;
            for (k, s) in kinds.iter().zip(&sizes) {
                cx.fitting(*k, IVec3::new(0, y, z + s.x - 1), 3);
                z += s.x + 1;
            }
            return Some(());
        }
    }
    None
}

/// A plank bench against the wall, three voxels long. Let go, a leg has gone and the
/// seat has darkened.
pub(super) fn bench(cx: &mut Cx) -> Option<()> {
    let (y, z) = spots(cx, 3, 1).into_iter().find(|&(y, z)| fits_box(cx, IVec3::new(0, y, z), IVec3::new(0, y + 1, z + 2)))?;
    let (p, c) = (*cx.pal(), IVec3::new(0, y * 4, z * 4));
    let seat = if cx.decayed() { mat::PLANK_DARK } else { p.plank };
    cx.part(c + IVec3::new(0, 4, 0), c + IVec3::new(4, 5, 12), seat);
    for (k, (x, dz)) in [(0, 1), (3, 1), (0, 10), (3, 10)].into_iter().enumerate() {
        if cx.decayed() && k == 3 {
            continue;
        }
        cx.part(c + IVec3::new(x, 0, dz), c + IVec3::new(x + 1, 4, dz + 1), p.wood);
    }
    Some(())
}

/// Split logs stacked against the wall, their cut ends out. In the north and the
/// mountains, where winter is long, they go up to the eaves, two deep.
pub(super) fn woodpile(cx: &mut Cx) -> Option<()> {
    let north = matches!(cx.r.region, Region::Northern | Region::Alpine);
    let len = if cx.r.anchor.kind == AnchorKind::Corner { 3 } else { 4 + cx.rng.int(0, 1) };
    let deep = if north { 2 } else { 1 };
    let most = if north { 10 } else { 2 + (cx.wealth() > 0.5) as i32 };
    let p = *cx.pal();
    for (y, z) in spots(cx, len, deep) {
        let layer = |cx: &Cx, h: i32| (0..deep).all(|x| (z..z + len).all(|zz| cx.free(IVec3::new(x, y + h, zz))));
        let mut h = 0;
        while h < most && layer(cx, h) {
            h += 1;
        }
        if h < 2 || !cx.room(IVec3::new(0, y, z) * 4, IVec3::new(deep, y + h, z + len) * 4 + IVec3::new(1, 0, 0)) {
            continue;
        }
        // Let go, the top of the stack has been burnt down unevenly.
        for zz in z..z + len {
            let top = if cx.decayed() { h - cx.rng.int(0, h / 2) } else { h };
            for yy in 0..top.max(1) {
                for x in 0..deep {
                    cx.put(IVec3::new(x, y + yy, zz), p.wood);
                }
            }
            // A cut end on the face of each log, a little to one side or the other.
            for yy in 0..top.max(1) {
                let o = cx.rng.int(0, 2);
                cx.part(IVec3::new(deep * 4, (y + yy) * 4 + o, zz * 4 + o), IVec3::new(deep * 4 + 1, (y + yy) * 4 + o + 2, zz * 4 + o + 2), mat::PLANK_PALE);
            }
        }
        // A board laid over a stack the eaves do not cover.
        if !north && cx.room(IVec3::new(0, (y + h) * 4, z * 4), IVec3::new(deep * 4 + 1, (y + h) * 4 + 1, (z + len) * 4)) {
            cx.part(IVec3::new(0, (y + h) * 4, z * 4), IVec3::new(deep * 4 + 1, (y + h) * 4 + 1, (z + len) * 4), p.plank);
        }
        return Some(());
    }
    None
}

/// The flowers that seed themselves on roofs in a region.
fn roof_flowers(region: Region) -> &'static [Flower] {
    match region {
        Region::Northern => &[Flower::Stitchwort, Flower::Heather, Flower::CottonGrass, Flower::Campion],
        Region::Upland => &[Flower::Heather, Flower::Thistle, Flower::Cinquefoil],
        Region::Alpine => &[Flower::Gentian, Flower::Edelweiss],
        _ => &[Flower::Daisy, Flower::Buttercup, Flower::Campion],
    }
}

/// What grows on a roof about the anchor, across the roof and along a stretch of it:
/// grass and flowers on turf (`turf`, which only a turf roof takes), or moss, ferns and
/// mushrooms anywhere the forest is old. Each part sits on the voxel of roof under it.
pub(super) fn roof_growth(cx: &mut Cx, turf: bool) -> Option<()> {
    if turf && cx.r.style.roof != Cover::Turf {
        return None;
    }
    let flowers = roof_flowers(cx.r.region);
    let mut grown = 0;
    for x in -7..=7 {
        for z in -2..=1 {
            if x == 0 && z == 0 && cx.r.anchor.kind == AnchorKind::RoofTop {
                continue;
            }
            let Some(top) = (-12..=1).rev().find(|&y| cx.wall(IVec3::new(x, y, z)) && cx.open(IVec3::new(x, y + 1, z))) else { continue };
            let roll = cx.rng.unit();
            if roll > if turf { 0.45 } else { 0.4 } {
                continue;
            }
            let at = IVec3::new(x * 4 + cx.rng.int(0, 2), (top + 1) * 4, z * 4 + cx.rng.int(0, 2));
            let one = |w: i32, h: i32, d: i32| at + IVec3::new(w, h, d);
            if turf {
                if roll < 0.12 {
                    // A flower: its stem and leaves, then its bloom, each in its season.
                    let f = flowers[(cx.rng.next_u32() as usize) % flowers.len()];
                    if cx.room(at, one(1, 3, 1)) {
                        cx.growth(at, one(1, 2, 1), mat::STEM, f.leaf_code());
                        cx.growth(at + IVec3::Y * 2, one(1, 3, 1), f.petals(), f.bloom_code());
                        grown += 1;
                    }
                } else if cx.room(at, one(2, 3, 1)) {
                    let h = 2 + cx.rng.int(0, 1);
                    cx.growth(at, one(2, h, 1), mat::TUFT, 0);
                    grown += 1;
                }
            } else if roll < 0.25 {
                if cx.room(at, one(3, 1, 3)) {
                    cx.part(at, one(3, 1, 3), mat::MOSS_CARPET);
                    grown += 1;
                }
            } else if roll < 0.33 {
                if cx.room(at, one(1, 3, 1)) {
                    cx.growth(at, one(1, 3, 1), mat::LEAF_GLOSS, 0);
                    grown += 1;
                }
            } else {
                let f = if cx.rng.unit() < 0.5 { Flower::FlyAgaric } else { Flower::PennyBun };
                if cx.room(at, one(2, 2, 2)) {
                    cx.growth(at, one(1, 1, 1), mat::MUSHROOM_STEM, f.bloom_code());
                    cx.growth(at + IVec3::Y, one(2, 2, 2), f.petals(), f.bloom_code());
                    grown += 1;
                }
            }
        }
    }
    (grown > 0).then_some(())
}

/// A banner of cloth hung from a pole out from the wall, to stir in the wind.
pub(super) fn banner(cx: &mut Cx) -> Option<()> {
    if !cx.wall(IVec3::new(-1, 0, 0)) {
        return None;
    }
    let (pole_lo, pole_hi) = (IVec3::new(0, 1, 1), IVec3::new(9, 3, 3));
    if !cx.room(pole_lo, pole_hi) {
        return None;
    }
    // As long as it hangs clear of anything below, and of anything it could blow into.
    let long = (10..=24).rev().step_by(2).find(|&l| cx.room(IVec3::new(1, 1 - l, -1), IVec3::new(9, 1, 5)))?;
    let colour: Voxel = cx.cloth();
    cx.part(pole_lo, pole_hi, cx.pal().wood);
    cx.part(IVec3::new(9, 1, 1), IVec3::new(10, 3, 3), cx.pal().accent);
    for x in [2, 4, 6] {
        // Let go, it has frayed short.
        let frayed = if cx.decayed() { cx.rng.int(0, 8) } else { 0 };
        cx.ribbon(IVec3::new(x, 1, 1), long - frayed, colour, false);
    }
    Some(())
}
