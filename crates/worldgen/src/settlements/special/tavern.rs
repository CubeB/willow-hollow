//! The tavern: a common room with a bar and its barrels, tables and benches and a fire
//! that is never let go out, under rooms for travellers and the keeper's own, up a
//! flight of stairs. A sign hangs by the door.
//!
//! As built, the front (+X) is a long side, the door in its middle. The stairs rise
//! along the middle of the building from the low z end; upstairs a passage runs on from
//! them between the rooms, the keeper's and the largest guest room beside the stairs and
//! the rest beyond, a bed in each, and the chimney at the far end. Downstairs the bar runs
//! along the back of the common room, the fire is under the chimney and the tables are
//! along the front.

use glam::{IVec2, IVec3};

use super::{
    Built, Use,
    frame::{Block, Frame, Roof, STOREY, v3},
};
use crate::{
    features::{fittings::Fitting, props::Prop},
    materials as mat,
    settlements::kit::{Area, Hearth},
};

/// The guest rooms on each side of the passage beyond the stairs, by wealth: one or two.
fn rooms_a_side(wealth: f32) -> i32 {
    if wealth < 0.5 { 1 } else { 2 }
}

pub(super) fn build(mut f: Frame) -> (Built, Vec<String>) {
    let n = rooms_a_side(f.spec.wealth);
    // Inside the walls: x from -8 to 8 (the rooms at the back, the passage, the rooms at
    // the front), z from iz0 to iz1: the stairs and the rooms beside them, then n rooms a
    // side, each six long with a partition between.
    let (ix0, ix1) = (-8, 8);
    let iz0 = -(9 + 7 * n) / 2;
    let iz1 = iz0 + 8 + 7 * n;
    let fy = f.look.raised.min(3);
    let up = fy + STOREY;
    let b = Block::new((ix0 - 1, iz0 - 1), (ix1 + 1, iz1 + 1), fy, up + 5);
    f.shell(&b);
    f.roof(&b, &Roof { shape: f.look.shape, along_z: true, overhang: f.look.overhang, pitch: f.look.pitch, cover: f.look.cover });
    f.slab(&b, up);

    // Upstairs: the passage between partitions at x = -2 and x = 2, and the rooms off it.
    let roof_under = |f: &Frame, c: IVec2| f.roof_low(c).unwrap_or(b.plate + 1) - 1;
    for x in [-2, 2] {
        for z in iz0..=iz1 {
            let c = IVec2::new(x, z);
            let top = roof_under(&f, c);
            f.partition(v3(c, up + 1), v3(c, top));
        }
    }
    // Cross walls: after the rooms beside the stairs, and between the rooms beyond.
    let walls_z: Vec<i32> = (0..n).map(|k| iz0 + 9 + 7 * k).collect();
    for &z in &walls_z {
        for (x0, x1) in [(ix0, -3), (3, ix1)] {
            for x in x0..=x1 {
                let c = IVec2::new(x, z);
                let top = roof_under(&f, c);
                f.partition(v3(c, up + 1), v3(c, top));
            }
        }
    }
    // Doorways off the passage: the rooms by the stairs open where the floor above the
    // stairs begins, the others at their low z end.
    let doorway = |f: &mut Frame, x: i32, z: i32| f.hollow(IVec3::new(x, up + 1, z), IVec3::new(x, up + 5, z + 1));
    for x in [-2, 2] {
        doorway(&mut f, x, iz0 + 7);
        for k in 0..n {
            doorway(&mut f, x, iz0 + 10 + 7 * k);
        }
    }
    // The stairs, up the middle from the low z end.
    f.stairs(IVec2::new(-1, iz0), IVec2::Y, IVec2::X, fy);

    // The chimney at the high z end: a breast two deep inside the end wall, the fireplace
    // in it, and the stack up through the roof with the flue over the fire.
    let breast = (IVec2::new(-2, iz1 - 1), IVec2::new(2, iz1 + 1));
    f.chimney(breast.0, breast.1, fy, IVec2::new(0, iz1), fy + 4);
    f.hollow(IVec3::new(-1, fy + 1, iz1 - 1), IVec3::new(1, fy + 3, iz1));
    for x in -1..=1 {
        for z in iz1 - 1..=iz1 {
            f.put(IVec3::new(x, fy, z), f.pal.stone_dark);
        }
    }
    f.fire(IVec3::new(0, fy, iz1), true);
    f.hearth = Some(Hearth { at: IVec3::new(0, fy + 1, iz1), facing: IVec3::NEG_Z, central: false });
    f.uses.push((Use::Hearth, IVec3::new(0, fy, iz1 - 3)));
    // A kettle on the hearth stone, to one side of the fire.
    f.prop(Prop::Kettle, IVec3::new(-1, fy + 1, iz1 - 1), 2, false);

    // The door, in the middle of the common room's front.
    let dz = (iz0 + iz1) / 2 - 1;
    f.front_door(&b, dz, 3, 5);
    f.door_leaf(&b, dz, 3, 5);
    f.front_steps(&b, dz, 3, 2);

    // Windows: downstairs either side of the door, upstairs one to each room.
    for z in [iz0 + 2, iz1 - 3] {
        f.window(IVec2::new(ix1 + 1, z), fy + 2, 2, 2, IVec3::X);
    }
    f.window(IVec2::new(ix0 - 1, iz0 + 2), fy + 2, 2, 2, IVec3::NEG_X);
    f.window(IVec2::new(ix1 + 1, iz0 + 4), up + 3, 2, 2, IVec3::X);
    f.window(IVec2::new(ix0 - 1, iz0 + 4), up + 3, 2, 2, IVec3::NEG_X);
    for k in 0..n {
        let a = iz0 + 10 + 7 * k;
        f.window(IVec2::new(ix1 + 1, a + 2), up + 3, 2, 2, IVec3::X);
        f.window(IVec2::new(ix0 - 1, a + 2), up + 3, 2, 2, IVec3::NEG_X);
    }

    // The keeper's room, behind the stairs: their bed and their partner's, and a chest.
    let feet_up = up + 1;
    f.bed(IVec3::new(ix0, feet_up, iz0), IVec3::Z, IVec3::X, false);
    f.bed(IVec3::new(-3, feet_up, iz0), IVec3::Z, IVec3::NEG_X, false);
    f.fit(Fitting::Chest, IVec3::new(-6, feet_up, iz0), IVec3::Z, false);
    f.furniture_spot(IVec3::new(-6, feet_up, iz0), IVec3::X, IVec3::Z, 2, 2);
    // The big guest room in front of the stairs: a bed along the outer wall, and in a
    // tavern of one room a side another along the partition, so there are three at least.
    f.bed(IVec3::new(ix1, feet_up, iz0), IVec3::Z, IVec3::NEG_X, true);
    if n == 1 || f.spec.wealth > 0.8 {
        f.bed(IVec3::new(3, feet_up, iz0), IVec3::Z, IVec3::X, true);
    }
    // A bed in each room beyond, along its far wall, and a stool.
    for k in 0..n {
        let a = iz0 + 10 + 7 * k;
        f.bed(IVec3::new(ix1, feet_up, a + 5), IVec3::NEG_X, IVec3::NEG_Z, true);
        f.bed(IVec3::new(ix0, feet_up, a + 5), IVec3::X, IVec3::NEG_Z, true);
        f.fit(Fitting::Stool, IVec3::new(ix1 - 1, feet_up, a + 1), IVec3::NEG_X, false);
        f.fit(Fitting::Stool, IVec3::new(ix0 + 1, feet_up, a + 1), IVec3::X, false);
        f.furniture_spot(IVec3::new(ix1, feet_up, a), IVec3::Z, IVec3::NEG_X, 2, 2);
    }

    // The bar along the back of the common room, beyond the stairs: a counter, the
    // keeper's side of it behind, the barrels against the wall.
    let feet = b.feet();
    let (bar0, bar1) = (iz0 + 8, iz0 + 13);
    let counter = f.pal.plank;
    for z in bar0..=bar1 {
        for y in feet..feet + 2 {
            f.put(IVec3::new(-4, y, z), counter);
        }
    }
    // The top of the counter overhangs the drinkers' side, a darker board.
    let top = (feet + 2) * 4;
    f.bx(IVec3::new(-4 * 4 - 1, top, bar0 * 4), IVec3::new(-3 * 4 + 2, top + 1, (bar1 + 1) * 4), mat::PLANK_DARK);
    // Cups and a jug standing on it.
    for (z, tall) in [(bar0 + 1, 2), (bar0 + 3, 3), (bar1 - 1, 2)] {
        let c = IVec3::new(-4 * 4 + 1, top + 1, z * 4 + 1);
        f.bx(c, c + IVec3::new(2, tall, 2), if tall > 2 { mat::CLAY } else { mat::PLANK_PALE });
    }
    // The barrels stand a hand from the wall, each with room about it.
    for z in (bar0..=bar1).step_by(3) {
        f.prop(Prop::Barrel, IVec3::new(ix0 + 1, feet, z), 0, false);
    }
    f.uses.push((Use::Bar, IVec3::new(-3, fy, (bar0 + bar1) / 2)));
    f.furniture_spot(IVec3::new(ix0, feet, bar0), IVec3::Z, IVec3::X, bar1 - bar0 + 1, 2);

    // Tables along the front, each with a bench either side, clear of the way in.
    let mut tables = vec![iz0 + 2, iz1 - 3];
    if n > 1 {
        tables.extend([iz0 + 6, iz1 - 7]);
    }
    for zt in tables {
        if zt + 2 >= dz - 1 && zt - 1 <= dz + 3 {
            continue;
        }
        f.fit(Fitting::Table, IVec3::new(4, feet, zt), IVec3::Z, false);
        f.prop(Prop::Bench, IVec3::new(4, feet, zt - 1), 0, false);
        f.prop(Prop::Bench, IVec3::new(5, feet, zt + 2), 0, false);
        f.uses.push((Use::Table, IVec3::new(3, fy, zt)));
        // Each bench, three voxels long, seats two, one at either end, facing the table.
        f.seats.extend([(IVec3::new(4, feet, zt - 1), IVec3::Z), (IVec3::new(6, feet, zt - 1), IVec3::Z), (IVec3::new(5, feet, zt + 2), IVec3::NEG_Z), (IVec3::new(7, feet, zt + 2), IVec3::NEG_Z)]);
    }

    // The sign beside the door, painted with the keeper's colours.
    let paint = [(mat::CLOTH_RED, mat::CLOTH_YELLOW), (mat::CLOTH_GREEN, mat::CLOTH_PALE), (mat::CLOTH_BLUE, mat::CLOTH_YELLOW)][(f.spec.seed % 3) as usize];
    f.hanging_sign(&b, dz + 4, fy + 6, paint);
    f.porch(&b, dz, 3);

    // Keep the stairs and the rooms clear of props.
    f.clear.push((IVec3::new(-1, feet, iz0), IVec3::new(0, up + 5, iz0 + 7)));
    f.clear.push((IVec3::new(ix0, feet, iz0), IVec3::new(ix1, feet + 4, iz1)));
    f.finish(Area { lo: b.lo, hi: b.hi }, fy)
}

