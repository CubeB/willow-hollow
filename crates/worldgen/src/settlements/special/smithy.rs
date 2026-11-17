//! The smithy: the forge against the back wall, alight day and night under its chimney,
//! the anvil before it, the bellows and the quenching trough of the forge itself, a bench
//! with the smith's tools hung on the wall over it and a grindstone by the door; and the
//! smith's home up a flight of stairs above it all, warmed by the chimney going up through
//! it.
//!
//! As built the wide door is in the +X wall, the forge facing it across the floor, so the
//! glow is seen from the lane. The stairs climb along the low z wall toward the back.

use glam::{IVec2, IVec3};

use super::{
    Built, Use,
    frame::{Block, Frame, Roof, STOREY},
};
use crate::{
    features::{fittings::Fitting, props::Prop},
    materials as mat,
    settlements::kit::{Area, Hearth, rules::FloorKind},
};

pub(super) fn build(mut f: Frame) -> (Built, Vec<String>) {
    // An earth or flagged floor: no boards by a forge.
    if f.look.floor == FloorKind::Boards {
        f.look.floor = FloorKind::Earth;
    }
    let (ix0, ix1, iz0, iz1) = (-6, 6, -5, 5);
    let fy = 0;
    let up = fy + STOREY;
    let b = Block::new((ix0 - 1, iz0 - 1), (ix1 + 1, iz1 + 1), fy, up + 5);
    f.shell(&b);
    f.roof(&b, &Roof { shape: f.look.shape, along_z: true, overhang: f.look.overhang, pitch: f.look.pitch, cover: f.look.cover });
    f.slab(&b, up);
    let feet = b.feet();

    // The stairs along the low z wall, from the front toward the back.
    f.stairs(IVec2::new(5, iz0), IVec2::NEG_X, IVec2::Y, fy);

    // The forge against the back wall, facing the door: its fire on the hearth, and the
    // stack over it up through the home above and the roof, its flue straight over the
    // fire so the smoke goes up it.
    // The top stone of the forge's back stands in the floor above, which the stack takes.
    f.put(IVec3::new(ix0, up, 1), voxel::AIR);
    let forge = f.fit(Fitting::Forge, IVec3::new(ix0, feet, -2), IVec3::X, false);
    let ember = forge.at(IVec3::new(1, 1, 1));
    f.fire(ember, false);
    let stack = [forge.at(IVec3::new(0, 5, 0)), forge.at(IVec3::new(2, 5, 2))];
    let flue = forge.at(IVec3::new(1, 5, 1));
    let (lo, hi) = (stack[0].min(stack[1]), stack[0].max(stack[1]));
    f.chimney(IVec2::new(lo.x - 1, lo.z), IVec2::new(hi.x, hi.z), up, IVec2::new(flue.x, flue.z), up);
    f.hearth = Some(Hearth { at: ember + IVec3::Y, facing: IVec3::X, central: false });
    f.uses.push((Use::Forge, forge.at(IVec3::new(2, -1, 3))));

    // The anvil before the forge, its face to it, and the grindstone by the door.
    f.fit(Fitting::Anvil, IVec3::new(-1, feet, -1), IVec3::NEG_X, false);
    f.uses.push((Use::Anvil, IVec3::new(1, fy, -3)));
    f.fit(Fitting::Grindstone, IVec3::new(4, feet, 2), IVec3::NEG_X, false);
    // The bench against the high z wall, and the tools hung over it.
    f.fit(Fitting::Workbench, IVec3::new(-1, feet, iz1 - 1), IVec3::NEG_Z, false);
    f.furniture_spot(IVec3::new(-1, feet, iz1), IVec3::X, IVec3::NEG_Z, 5, 2);
    tools(&mut f, (iz1 + 1) * 4, -1, 4, feet + 3);
    // Charcoal in sacks in the corner by the forge.
    f.prop(Prop::Sack, IVec3::new(ix0, feet, iz1 - 1), 0, false);
    f.prop(Prop::Sack, IVec3::new(ix0 + 2, feet, iz1 - 1), 0, false);

    // The wide door, and a horseshoe nailed over it.
    let dz = -2;
    f.front_door(&b, dz, 4, 5);
    let face = (b.hi.x + 1) * 4;
    let (zc, yc) = (dz * 4 + 6, (fy + 6) * 4 + 1);
    let iron = mat::IRON;
    f.bx(IVec3::new(face, yc, zc), IVec3::new(face + 1, yc + 3, zc + 1), iron);
    f.bx(IVec3::new(face, yc, zc + 3), IVec3::new(face + 1, yc + 3, zc + 4), iron);
    f.bx(IVec3::new(face, yc - 1, zc + 1), IVec3::new(face + 1, yc, zc + 3), iron);
    f.window(IVec2::new(ix1 + 1, 3), feet + 1, 2, 2, IVec3::X);
    f.window(IVec2::new(ix1 + 1, -4), up + 3, 2, 2, IVec3::X);
    f.window(IVec2::new(ix1 + 1, 2), up + 3, 2, 2, IVec3::X);
    f.window(IVec2::new(-2, iz0 - 1), up + 3, 2, 2, IVec3::NEG_Z);

    // The smith's home above: beds along the far wall and by the back wall, a table and a
    // chest.
    let feet_up = up + 1;
    let sleepers = 2 + (f.spec.wealth > 0.5) as i32;
    f.bed(IVec3::new(ix1, feet_up, iz1), IVec3::NEG_X, IVec3::NEG_Z, false);
    f.bed(IVec3::new(-2, feet_up, iz1), IVec3::NEG_X, IVec3::NEG_Z, false);
    if sleepers > 2 {
        f.bed(IVec3::new(ix0, feet_up, iz0), IVec3::Z, IVec3::X, false);
    }
    f.fit(Fitting::Table, IVec3::new(0, feet_up, 0), IVec3::Z, false);
    f.fit(Fitting::Stool, IVec3::new(1, feet_up, 2), IVec3::NEG_Z, false);
    f.fit(Fitting::Stool, IVec3::new(2, feet_up, -1), IVec3::Z, false);
    f.fit(Fitting::Chest, IVec3::new(ix1 - 1, feet_up, -1), IVec3::NEG_X, false);
    f.uses.push((Use::Table, IVec3::new(1, up, -2)));

    f.porch(&b, dz, 4);
    f.clear.push((IVec3::new(ix0, feet, iz0), IVec3::new(ix1, feet + 4, iz1)));
    f.finish(Area { lo: b.lo, hi: b.hi }, fy)
}

/// The smith's tools hung on pegs along a wall whose inside face is at cell `face` (a high
/// z wall): a rail from column `x0`, `len` columns long, at layer `y`, with tongs, hammers
/// and a rasp hanging from it.
fn tools(f: &mut Frame, face: i32, x0: i32, len: i32, y: i32) {
    let (wood, iron) = (f.pal.wood, mat::IRON);
    let yc = y * 4 + 2;
    f.bx(IVec3::new(x0 * 4, yc, face - 1), IVec3::new((x0 + len) * 4, yc + 1, face), wood);
    for k in 0..len * 2 {
        let x = x0 * 4 + 1 + k * 2;
        match k % 4 {
            // Tongs: two long iron arms.
            0 => {
                f.bx(IVec3::new(x, yc - 6, face - 2), IVec3::new(x + 1, yc, face - 1), iron);
                f.bx(IVec3::new(x + 1, yc - 5, face - 2), IVec3::new(x + 2, yc - 1, face - 1), iron);
            }
            // A hammer, its head down.
            1 => {
                f.bx(IVec3::new(x, yc - 4, face - 2), IVec3::new(x + 1, yc, face - 1), wood);
                f.bx(IVec3::new(x - 1, yc - 5, face - 2), IVec3::new(x + 2, yc - 4, face - 1), iron);
            }
            // A rasp.
            2 => f.bx(IVec3::new(x, yc - 4, face - 2), IVec3::new(x + 1, yc, face - 1), iron),
            _ => {}
        }
    }
}
