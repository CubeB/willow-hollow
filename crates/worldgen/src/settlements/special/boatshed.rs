//! The fishers' boat shed, which is the fisher's home as well: a long shed under one
//! roof, its gable end open to the water, a boat drawn up on the slipway inside and the
//! nets hung on a rack along the wall, and the household's room at the landward end, its
//! door on the lane.
//!
//! As built the door is at +X and the water at -X: the layout turns it so its back meets
//! the shore.

use glam::{IVec2, IVec3};
use voxel::AIR;

use super::{
    Built, Use,
    frame::{Block, Frame, Roof, v3},
};
use crate::{
    features::fittings::Fitting,
    materials as mat,
    settlements::{
        Region,
        kit::{Area, RoofShape},
    },
};

pub(super) fn build(mut f: Frame) -> (Built, Vec<String>) {
    let (ix0, ix1, iz0, iz1) = (-10, 5, -4, 4);
    let fy = 0;
    let b = Block::new((ix0 - 1, iz0 - 1), (ix1 + 1, iz1 + 1), fy, fy + 7);
    let feet = b.feet();
    f.shell(&b);
    // A pitched roof whatever the region's houses have: the gable stands over the water.
    let shape = if f.look.shape == RoofShape::Flat { RoofShape::Gable } else { f.look.shape };
    let shape = if shape == RoofShape::Hip { RoofShape::Gable } else { shape };
    let cover = if f.look.shape == RoofShape::Flat { crate::settlements::kit::Cover::Palm } else { f.look.cover };
    f.roof(&b, &Roof { shape, along_z: false, overhang: f.look.overhang.min(2), pitch: f.look.pitch, cover });

    // The slipway: level under the boat, then down toward the water, through the open end.
    let slip = |x: i32| match x {
        -8 => -1,
        -9 => -2,
        x if x <= -10 => -3,
        _ => 0,
    };
    for x in b.lo.x..=-8 {
        for z in iz0..=iz1 {
            let y = slip(x);
            f.put(IVec3::new(x, y, z), f.pal.plank);
            for k in y + 1..=0 {
                f.put(IVec3::new(x, k, z), AIR);
            }
        }
    }
    // The open end: the wall taken away between its corner posts, down to the slipway.
    for z in iz0..=iz1 {
        f.hollow(IVec3::new(b.lo.x, slip(b.lo.x) + 1, z), IVec3::new(b.lo.x, b.plate - 1, z));
    }
    // Rails down the slipway under the keel.
    let wood = f.pal.wood;
    for z in [-2, 0] {
        for x in b.lo.x..=-1 {
            let y = slip(x) + 1;
            f.bx(IVec3::new(x * 4, y * 4, z * 4 + 1), IVec3::new(x * 4 + 4, y * 4 + 1, z * 4 + 3), wood);
        }
    }
    boat(&mut f, -7, -1, -2, 0, feet);
    f.uses.push((Use::Boat, IVec3::new(-4, fy, 2)));

    // Between the boat and the home, a partition with a doorway.
    for z in iz0..=iz1 {
        let c = IVec2::new(0, z);
        let top = f.roof_low(c).unwrap_or(b.plate + 1) - 1;
        f.partition(v3(c, feet), v3(c, top));
    }
    f.hollow(IVec3::new(0, feet, -1), IVec3::new(0, feet + 4, 0));

    // The nets on their rack, pots and traps by the boat.
    f.fit(Fitting::NetRack, IVec3::new(-6, feet, iz1 - 1), IVec3::NEG_Z, false);
    let catch = match f.region() {
        Region::Wetland | Region::OldGrowth | Region::Rainforest => Fitting::EelTrap,
        _ => Fitting::LobsterPot,
    };
    f.fit(catch, IVec3::new(-6, feet, iz0), IVec3::Z, false);

    // The fisher's room: beds either side, a table and a stool between.
    f.bed(IVec3::new(ix1, feet, iz1), IVec3::NEG_X, IVec3::NEG_Z, false);
    f.bed(IVec3::new(ix1, feet, iz0), IVec3::NEG_X, IVec3::Z, false);
    f.fit(Fitting::Table, IVec3::new(2, feet, 1), IVec3::NEG_Z, false);
    f.fit(Fitting::Stool, IVec3::new(3, feet, 0), IVec3::Z, false);
    f.uses.push((Use::Table, IVec3::new(2, fy, -1)));

    f.front_door(&b, -1, 3, 5);
    f.door_leaf(&b, -1, 3, 5);
    f.window(IVec2::new(2, b.hi.y), feet + 1, 2, 2, IVec3::Z);
    f.window(IVec2::new(2, b.lo.y), feet + 1, 2, 2, IVec3::NEG_Z);
    f.window(IVec2::new(-5, b.hi.y), feet + 2, 2, 2, IVec3::Z);
    // Fish drying on a rack outside the long wall.
    f.fit(Fitting::FishRack, IVec3::new(-3, 0, b.lo.y - 2), IVec3::NEG_Z, false);
    f.porch(&b, -1, 3);
    f.clear.push((IVec3::new(b.lo.x, feet, iz0), IVec3::new(ix1, feet + 4, iz1)));
    f.finish(Area { lo: b.lo, hi: b.hi }, fy)
}

/// A rowing boat of planks drawn up on the slipway, between columns `x0` and `x1` and
/// `z0` and `z1` (inclusive), its keel on the layer `feet`; its bow toward -X and the
/// water, oars laid along its thwarts.
fn boat(f: &mut Frame, x0: i32, x1: i32, z0: i32, z1: i32, feet: i32) {
    let (plank, dark, pale) = (f.pal.plank, mat::PLANK_DARK, mat::PLANK_PALE);
    let (a, b) = (x0 * 4 + 2, (x1 + 1) * 4 - 2);
    let (c, d) = (z0 * 4 + 1, (z1 + 1) * 4 - 1);
    let y = feet * 4;
    let mid = (c + d) / 2;
    // The keel, the bottom boards and the sides, drawn in toward the bow.
    f.bx(IVec3::new(a + 1, y, mid - 1), IVec3::new(b, y + 1, mid + 1), dark);
    f.bx(IVec3::new(a + 4, y + 1, c + 1), IVec3::new(b - 1, y + 2, d - 1), plank);
    f.bx(IVec3::new(a + 4, y + 2, c), IVec3::new(b, y + 6, c + 1), plank);
    f.bx(IVec3::new(a + 4, y + 2, d - 1), IVec3::new(b, y + 6, d), plank);
    f.bx(IVec3::new(a + 2, y + 2, c + 1), IVec3::new(a + 4, y + 6, c + 2), plank);
    f.bx(IVec3::new(a + 2, y + 2, d - 2), IVec3::new(a + 4, y + 6, d - 1), plank);
    f.bx(IVec3::new(a, y + 3, mid - 1), IVec3::new(a + 2, y + 8, mid + 1), dark);
    f.bx(IVec3::new(a + 1, y + 2, c + 2), IVec3::new(a + 2, y + 6, d - 2), plank);
    // The transom across the stern, gunwales along the top, and the thwarts.
    f.bx(IVec3::new(b - 1, y + 2, c + 1), IVec3::new(b, y + 6, d - 1), dark);
    f.bx(IVec3::new(a + 4, y + 6, c), IVec3::new(b, y + 7, c + 1), dark);
    f.bx(IVec3::new(a + 4, y + 6, d - 1), IVec3::new(b, y + 7, d), dark);
    for x in [a + 8, b - 7] {
        f.bx(IVec3::new(x, y + 4, c + 1), IVec3::new(x + 2, y + 5, d - 1), plank);
    }
    // Two oars along it, resting on the thwarts.
    for z in [mid - 2, mid + 1] {
        f.bx(IVec3::new(a + 9, y + 5, z), IVec3::new(b - 1, y + 6, z + 1), pale);
        f.bx(IVec3::new(a + 5, y + 5, z - 1), IVec3::new(a + 9, y + 6, z + 2), pale);
    }
}
