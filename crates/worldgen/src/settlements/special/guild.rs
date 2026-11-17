//! The guild house of a great town, shared by two or three guilds the town suits (see
//! [`super::guilds`]). Their banners stream from poles over the door; inside, the hall
//! has a long table and a fire, and upstairs each guild has a room of its own, its colours
//! on the wall, beside the warden who keeps the house.
//!
//! Its plan is the tavern's: the stairs up the middle from the low z end, a passage on
//! from them upstairs between rooms, and the chimney at the far end.

use glam::{IVec2, IVec3};

use super::{
    Built, Guild, Use, guilds,
    frame::{Block, Frame, Roof, STOREY, v3},
};
use crate::{
    features::{cloth, fittings::Fitting, props::Prop},
    settlements::kit::{Area, Hearth},
};

pub(super) fn build(mut f: Frame) -> (Built, Vec<String>) {
    f.guilds = guilds(&f.spec.facts, f.spec.wealth, f.spec.seed);
    let (ix0, ix1, iz0, iz1) = (-8, 8, -8, 7);
    let fy = f.look.raised.min(2);
    let up = fy + STOREY;
    let b = Block::new((ix0 - 1, iz0 - 1), (ix1 + 1, iz1 + 1), fy, up + 5);
    f.shell(&b);
    f.roof(&b, &Roof { shape: f.look.shape, along_z: true, overhang: f.look.overhang, pitch: f.look.pitch, cover: f.look.cover });
    f.slab(&b, up);
    let under = |f: &Frame, c: IVec2| f.roof_low(c).unwrap_or(b.plate + 1) - 1;
    for x in [-2, 2] {
        for z in iz0..=iz1 {
            let c = IVec2::new(x, z);
            let top = under(&f, c);
            f.partition(v3(c, up + 1), v3(c, top));
        }
    }
    for (x0, x1) in [(ix0, -3), (3, ix1)] {
        for x in x0..=x1 {
            let c = IVec2::new(x, iz0 + 9);
            let top = under(&f, c);
            f.partition(v3(c, up + 1), v3(c, top));
        }
    }
    for x in [-2, 2] {
        for z in [iz0 + 7, iz0 + 10] {
            f.hollow(IVec3::new(x, up + 1, z), IVec3::new(x, up + 5, z + 1));
        }
    }
    f.stairs(IVec2::new(-1, iz0), IVec2::Y, IVec2::X, fy);
    // The fire at the far end, under the chimney.
    f.chimney(IVec2::new(-2, iz1 - 1), IVec2::new(2, iz1 + 1), fy, IVec2::new(0, iz1), fy + 4);
    f.hollow(IVec3::new(-1, fy + 1, iz1 - 1), IVec3::new(1, fy + 3, iz1));
    for x in -1..=1 {
        for z in iz1 - 1..=iz1 {
            f.put(IVec3::new(x, fy, z), f.pal.stone_dark);
        }
    }
    f.fire(IVec3::new(0, fy, iz1), true);
    f.hearth = Some(Hearth { at: IVec3::new(0, fy + 1, iz1), facing: IVec3::NEG_Z, central: false });
    f.uses.push((Use::Hearth, IVec3::new(0, fy, iz1 - 3)));

    let dz = -2;
    f.front_door(&b, dz, 3, 5);
    f.door_leaf(&b, dz, 3, 5);
    f.front_steps(&b, dz, 3, 2);
    for z in [iz0 + 2, iz1 - 3] {
        f.window(IVec2::new(ix1 + 1, z), fy + 2, 2, 2, IVec3::X);
        f.window(IVec2::new(ix0 - 1, z), fy + 2, 2, 2, IVec3::NEG_X);
    }
    for z in [iz0 + 5, iz0 + 10] {
        f.window(IVec2::new(ix1 + 1, z), up + 3, 2, 2, IVec3::X);
        f.window(IVec2::new(ix0 - 1, z), up + 3, 2, 2, IVec3::NEG_X);
    }

    // The hall: a long table down the front half, benches either side of it, the
    // guilds' chests and shelves along the back.
    let feet = b.feet();
    for zt in [iz0 + 1, iz1 - 6] {
        f.fit(Fitting::Table, IVec3::new(4, feet, zt), IVec3::X, false);
        f.prop(Prop::Bench, IVec3::new(3, feet, zt + 3), 3, false);
        f.prop(Prop::Bench, IVec3::new(6, feet, zt + 3), 3, false);
        f.uses.push((Use::Table, IVec3::new(7, fy, zt + 1)));
    }
    f.fit(Fitting::Shelf, IVec3::new(ix0, feet, iz0 + 9), IVec3::X, false);
    f.fit(Fitting::Chest, IVec3::new(ix0, feet, iz0 + 13), IVec3::X, false);
    f.furniture_spot(IVec3::new(ix0, feet, iz0 + 9), IVec3::Z, IVec3::X, 6, 2);

    // Upstairs: the warden's room behind the stairs, and a room for each guild.
    let feet_up = up + 1;
    f.bed(IVec3::new(ix0, feet_up, iz0), IVec3::Z, IVec3::X, false);
    f.bed(IVec3::new(-3, feet_up, iz0), IVec3::Z, IVec3::NEG_X, false);
    let rooms = [(IVec3::new(3, feet_up, iz0), true), (IVec3::new(3, feet_up, iz0 + 10), true), (IVec3::new(ix0, feet_up, iz0 + 10), false)];
    let gs = f.guilds.clone();
    for (k, &(lo, front)) in rooms.iter().enumerate() {
        let Some(&g) = gs.get(k) else {
            // A room no guild has: the house's records, on shelves.
            f.fit(Fitting::Shelf, IVec3::new(lo.x + 1, feet_up, lo.z + 5), IVec3::NEG_Z, false);
            f.fit(Fitting::Chest, IVec3::new(lo.x + 4, feet_up, lo.z + 4), IVec3::NEG_Z, false);
            continue;
        };
        // Each guild's room: its table and stools, a chest, and its colours on the wall.
        f.fit(Fitting::Table, IVec3::new(lo.x + 1, feet_up, lo.z + 2), IVec3::Z, false);
        f.fit(Fitting::Stool, IVec3::new(lo.x + 1, feet_up, lo.z + 1), IVec3::Z, false);
        f.fit(Fitting::Stool, IVec3::new(lo.x + 4, feet_up, lo.z + 4), IVec3::NEG_Z, false);
        let wall_x = if front { (ix1 + 1) * 4 } else { ix0 * 4 };
        wall_banner(&mut f, wall_x, if front { -1 } else { 1 }, lo.z * 4 + 10, feet_up * 4 + 2, g);
        f.uses.push((Use::Table, IVec3::new(lo.x + 2, up, lo.z + 4)));
    }

    // The guilds' banners on poles over the door and either side of it, between the
    // windows upstairs, streaming in the wind and ending above the doorway.
    let face = (b.hi.x + 1) * 4;
    let y = (up + 3) * 4;
    let at: &[i32] = if gs.len() > 2 { &[-20, -2, 18] } else { &[-20, 18] };
    for (&g, &z) in gs.iter().zip(at) {
        banner(&mut f, face, y, z, g);
    }
    f.porch(&b, dz, 3);
    f.clear.push((IVec3::new(-1, feet, iz0), IVec3::new(0, up + 5, iz0 + 7)));
    f.finish(Area { lo: b.lo, hi: b.hi }, fy)
}

/// A guild's banner on a pole out from the wall face at cell `face` (a +X face), at
/// height `y` and cell `z`: ribbons of its colours side by side, the stripe in the middle.
fn banner(f: &mut Frame, face: i32, y: i32, z: i32, g: Guild) {
    let (field, stripe) = g.colours();
    let wood = f.pal.wood;
    f.bx(IVec3::new(face, y, z), IVec3::new(face + 14, y + 1, z + 1), wood);
    f.bx(IVec3::new(face + 13, y + 1, z), IVec3::new(face + 14, y + 2, z + 1), crate::materials::IRON);
    for k in 0..6 {
        let colour = if k == 2 || k == 3 { stripe } else { field };
        // Along the pole, each ribbon two cells wide along x.
        let knot = IVec3::new(face + 1 + k * 2, y, z);
        let length = 12 - (k % 2) * 2;
        f.fine.extend(cloth::ribbon(knot, length, colour, 0));
    }
}

/// A guild's colours hung flat on the inside of a wall at cell `x` (`dir` into the room),
/// centred on cell `z`, from cell `y` up: still air indoors, so they hang still.
fn wall_banner(f: &mut Frame, x: i32, dir: i32, z: i32, y: i32, g: Guild) {
    let (field, stripe) = g.colours();
    let (x0, x1) = if dir < 0 { (x - 1, x) } else { (x, x + 1) };
    let (s0, s1) = if dir < 0 { (x - 2, x - 1) } else { (x + 1, x + 2) };
    f.bx(IVec3::new(x0, y, z - 4), IVec3::new(x1, y + 14, z + 4), field);
    f.bx(IVec3::new(s0, y + 2, z - 1), IVec3::new(s1, y + 12, z + 1), stripe);
    f.bx(IVec3::new(s0, y + 14, z - 5), IVec3::new(s1, y + 15, z + 5), f.pal.wood);
}
