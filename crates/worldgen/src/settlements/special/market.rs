//! The market hall of a great town: an open arcade of pillars, the stalls in rows along
//! its sides, under the town's hall up a flight of stairs, where a long table stands for
//! the town's business. Where the sun is hot or the rain warm (the desert, the marsh and
//! the rainforest) it is one tall open hall under a high roof instead, with no room above.

use glam::{IVec2, IVec3};
use voxel::AIR;

use super::{
    Built, Use,
    frame::{Block, Frame, Roof, STOREY, v3},
};
use crate::{
    features::{fittings::Fitting, props::Prop},
    settlements::{
        Region,
        kit::{Area, WallKind},
    },
};

pub(super) fn build(mut f: Frame) -> (Built, Vec<String>) {
    let tall = matches!(f.region(), Region::Desert | Region::Wetland | Region::Rainforest);
    // Pillars every four columns: the sides are lengths of four and one.
    let (ix0, ix1, iz0, iz1) = (-9, 9, -11, 11);
    let fy = 0;
    let up = fy + STOREY;
    let plate = if tall { fy + 8 } else { up + 5 };
    let b = Block::new((ix0 - 1, iz0 - 1), (ix1 + 1, iz1 + 1), fy, plate);
    let feet = b.feet();
    f.shell(&b);
    // The arcade: open between the pillars from the floor to the beam over them.
    let beam = if tall { plate } else { up };
    let masonry = matches!(f.look.walls, WallKind::Stone | WallKind::Drystone | WallKind::Mudbrick | WallKind::Cob);
    for c in b.ring() {
        let along = if c.x == b.lo.x || c.x == b.hi.x { c.y - b.lo.y } else { c.x - b.lo.x };
        if along % 4 == 0 {
            // A pillar: of the walls' stone, or a post of the region's timber.
            if !masonry {
                for y in feet..beam {
                    f.put(v3(c, y), f.pal.wood);
                }
            }
            continue;
        }
        f.hollow(v3(c, feet), v3(c, beam - 1));
        let v = f.floor_voxel(v3(c, fy));
        f.put(v3(c, fy), v);
    }
    let shape = f.look.shape;
    f.roof(&b, &Roof { shape, along_z: true, overhang: f.look.overhang, pitch: f.look.pitch, cover: f.look.cover });
    let dz = 1;
    f.front_door(&b, dz, 3, 5);
    // The whole arcade is open; keep its floor clear of props.
    f.clear.push((IVec3::new(b.lo.x, feet, b.lo.y), IVec3::new(b.hi.x, feet + 4, b.hi.y)));

    // Stalls in a row along the back facing in, and along the front clear of the way in.
    let decayed = false;
    for z in [iz0, iz0 + 7, iz0 + 14] {
        f.fit(Fitting::MarketStall, IVec3::new(ix0, feet, z), IVec3::X, decayed);
        f.uses.push((Use::Stall, IVec3::new(ix0 + 4, fy, z + 2)));
    }
    for z in [iz0, iz0 + 6, iz0 + 15] {
        f.fit(Fitting::MarketStall, IVec3::new(ix1 - 2, feet, z), IVec3::NEG_X, decayed);
        f.uses.push((Use::Stall, IVec3::new(ix1 - 4, fy, z + 2)));
    }
    // Sacks and crates of goods by the stalls.
    f.prop(Prop::Crate, IVec3::new(ix0 + 4, feet, iz0 + 6), 0, decayed);
    f.prop(Prop::Sack, IVec3::new(ix0 + 4, feet, 2), 0, decayed);
    f.prop(Prop::Barrel, IVec3::new(ix0 + 4, feet, iz1 - 1), 0, decayed);

    if !tall {
        // The hall above, its floor on the beam over the pillars, up stairs at the far end.
        f.slab(&b, up);
        for c in b.ring() {
            let at = v3(c, up);
            let v = f.wall_voxel(at, &b);
            f.put(at, v);
        }
        f.stairs(IVec2::new(6, iz1 - 1), IVec2::NEG_X, IVec2::Y, fy);
        // A long table for the town's business, benches along it.
        let feet_up = up + 1;
        for zt in [iz0 + 1, iz0 + 6, iz0 + 11] {
            f.fit(Fitting::Table, IVec3::new(-2, feet_up, zt), IVec3::Z, false);
            f.prop(Prop::Bench, IVec3::new(-2, feet_up, zt - 1), 0, false);
            f.prop(Prop::Bench, IVec3::new(-1, feet_up, zt + 2), 0, false);
            f.uses.push((Use::Table, IVec3::new(2, up, zt)));
        }
        f.fit(Fitting::Chest, IVec3::new(ix0, feet_up, 0), IVec3::X, false);
        f.fit(Fitting::Shelf, IVec3::new(ix0, feet_up, 3), IVec3::X, false);
        // Windows all round the hall, between the pillars below.
        for z in [iz0 + 1, iz0 + 9, iz0 + 13, iz0 + 17] {
            f.window(IVec2::new(b.hi.x, z), up + 2, 2, 2, IVec3::X);
            f.window(IVec2::new(b.lo.x, z), up + 2, 2, 2, IVec3::NEG_X);
        }
        for x in [-5, 3] {
            f.window(IVec2::new(x, b.lo.y), up + 2, 2, 2, IVec3::NEG_Z);
            f.window(IVec2::new(x, b.hi.y), up + 2, 2, 2, IVec3::Z);
        }
    } else {
        // Under a high roof, the air over the stalls is open to it.
        for c in b.inner() {
            for y in feet..plate {
                if f.get(v3(c, y)).is_none() {
                    f.put(v3(c, y), AIR);
                }
            }
        }
    }
    f.porch(&b, dz, 3);
    f.finish(Area { lo: b.lo, hi: b.hi }, fy)
}
