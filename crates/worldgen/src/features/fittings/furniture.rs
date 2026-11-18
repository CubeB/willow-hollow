//! Furniture: the beds, tables, chests and hearths a house is lived in with.
//!
//! Every piece is built in model space as `fittings` describes: in the columns of its
//! size, its back along `z = 0`, its front toward +Z. Boxes are in cells, a tenth of a
//! metre.

use voxel::Voxel;

use super::{Hook, Kit, aged};
use crate::features::{landmarks::Palette, props::Prop, shape::Rng};
use crate::materials as mat;

/// A straw mattress in a ticking cover on the floor, a bolster at its head and a blanket
/// turned down over the rest. Decayed, flattened and rotting at one end, the straw spilt
/// and the blanket a faded heap.
pub(super) fn straw_pallet(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    if !decayed {
        k.b((1, 0, 0), (19, 2, 7), mat::SACKCLOTH);
        // Straw working out at the seams.
        for (x, z) in [(19, 2), (19, 5), (rng.int(4, 8), 7), (rng.int(11, 16), 7)] {
            let lo = if x == 19 { (19, 0, z) } else { (x, 0, 7) };
            let hi = if x == 19 { (20, 1, z + 1) } else { (x + 1, 1, 8) };
            k.b(lo, hi, mat::THATCH);
        }
        k.b((2, 2, 1), (5, 3, 6), mat::CLOTH_PALE);
        let turn = rng.int(6, 8);
        k.b((turn, 2, 0), (19, 3, 7), p.accent);
        k.b((turn, 3, 0), (turn + 2, 4, 7), p.accent);
        k.hook(Hook::Sleeper(glam::IVec3::new(3, 3, 3)));
        return;
    }
    let torn = rng.int(10, 13);
    k.b((1, 0, 0), (torn, 1, 7), mat::SACKCLOTH);
    k.b((torn, 0, 1), (18, 1, 6), mat::HAY_ROTTEN);
    k.b((2, 1, 1), (5, 2, 6), mat::SACKCLOTH);
    k.b((torn + 1, 1, 2), (torn + 5, 2, 5), aged(p.accent));
    k.b((19, 0, 3), (20, 1, 4), mat::HAY_ROTTEN);
}

/// A cupboard bed: a platform with a box of boards over it, roofed, open to the front
/// with a curtain drawn back, a mattress, bolster and blanket inside. Sleeps two.
/// Decayed, the roof has fallen in on the rotten mattress and one end has broken away.
pub(super) fn box_bed(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    let board = if decayed { mat::PLANK_DARK } else { p.plank };
    let frame = if decayed { mat::WOOD_DEAD } else { p.wood };
    // The platform is voxels, so a player bumps into the bed.
    for x in 0..6 {
        for z in 0..3 {
            k.vox(x, 0, z, board);
        }
    }
    if !decayed {
        k.b((0, 4, 0), (1, 20, 12), board);
        k.b((23, 4, 0), (24, 20, 12), board);
        k.b((1, 4, 0), (23, 19, 1), board);
        k.b((1, 19, 0), (23, 20, 12), board);
        k.b((1, 15, 11), (23, 19, 12), board);
        k.b((1, 4, 11), (3, 15, 12), frame);
        k.b((21, 4, 11), (23, 15, 12), frame);
        // The curtain, drawn back to the head end.
        k.b((3, 6, 11), (rng.int(6, 8), 15, 12), p.accent);
        k.b((1, 4, 1), (23, 6, 11), mat::SACKCLOTH);
        k.b((2, 6, 2), (6, 7, 10), mat::CLOTH_PALE);
        k.b((8, 6, 1), (23, 7, 11), p.accent);
        k.b((8, 4, 11), (20, 7, 12), p.accent);
        k.hook(Hook::Sleeper(glam::IVec3::new(4, 7, 4)));
        k.hook(Hook::Sleeper(glam::IVec3::new(4, 7, 8)));
        return;
    }
    k.b((0, 4, 0), (1, 20, 12), board);
    k.b((23, 4, 0), (24, 4 + rng.int(5, 10), 12), board);
    k.b((1, 4, 0), (23, 12, 1), board);
    k.b((1, 12, 0), (10, 20, 1), board);
    // What is left of the roof, held by the end that stands.
    k.b((1, 19, 1), (9, 20, 12), board);
    k.b((1, 4, 11), (3, 15, 12), frame);
    k.b((21, 4, 11), (23, 4 + rng.int(3, 8), 12), frame);
    k.b((3, 4, 1), (21, 5, 11), mat::HAY_ROTTEN);
    k.b((3, 5, 2), (7, 6, 10), mat::SACKCLOTH);
    // Boards of the roof fallen in.
    k.b((9, 5, 3), (21, 6, 6), mat::PLANK_DARK);
    k.b((11, 5, 7), (19, 6, 9), mat::PLANK_DARK);
}

/// A rope bed for warm country: four turned legs, a frame, and a lattice of rope across
/// it, with a pillow and a rolled rug. Decayed, a leg gone, the rope broken and hanging.
pub(super) fn rope_bed(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    let wood = if decayed { mat::WOOD_DEAD } else { p.wood };
    let rope = if decayed { mat::ROPE_ROTTEN } else { mat::ROPE };
    let gone = if decayed { rng.int(0, 3) } else { -1 };
    for (i, (x, z)) in [(0, 0), (18, 0), (0, 6), (18, 6)].into_iter().enumerate() {
        if i as i32 != gone {
            k.b((x, 0, z), (x + 2, 5, z + 2), wood);
        }
    }
    k.b((2, 4, 0), (18, 5, 1), wood);
    k.b((2, 4, 7), (18, 5, 8), wood);
    k.b((0, 4, 2), (1, 5, 6), wood);
    k.b((19, 4, 2), (20, 5, 6), wood);
    if !decayed {
        for z in [2, 4] {
            k.b((1, 4, z), (19, 5, z + 1), rope);
        }
        for x in [5, 8, 11, 14, 17] {
            k.b((x, 5, 1), (x + 1, 6, 7), rope);
        }
        k.b((1, 5, 2), (4, 6, 6), mat::CLOTH_PALE);
        k.b((14, 6, 1), (18, 7, 7), p.accent);
        k.hook(Hook::Sleeper(glam::IVec3::new(2, 6, 4)));
        return;
    }
    // A strand still across, and broken ends hanging from the frame.
    k.b((1, 4, 3), (19, 5, 4), rope);
    k.b((1, 1, 3), (2, 4, 4), rope);
    k.b((18, 2, 3), (19, 4, 4), rope);
    k.b((8, 2, 3), (9, 4, 4), rope);
}

/// A plank table on four legs with stretchers between them, and a few things on it.
/// Decayed, a leg is gone, half the top has fallen and what was on it lies broken.
pub(super) fn table(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    let wood = if decayed { mat::WOOD_DEAD } else { p.wood };
    let gone = if decayed { 2 + rng.int(0, 1) as usize } else { 9 };
    for (i, (x, z)) in [(1, 1), (14, 1), (1, 6), (14, 6)].into_iter().enumerate() {
        if i != gone {
            k.b((x, 0, z), (x + 1, 7, z + 1), wood);
        }
    }
    k.b((1, 2, 2), (2, 3, 6), wood);
    k.b((14, 2, 2), (15, 3, 6), wood);
    k.b((2, 2, 3), (14, 3, 5), wood);
    if !decayed {
        k.b((0, 7, 0), (16, 8, 8), p.plank);
        // A few things left out: a bowl, a jug, a loaf, a candle.
        if rng.unit() < 0.8 {
            k.b((2, 8, 2), (5, 9, 5), mat::CLAY);
        }
        if rng.unit() < 0.7 {
            k.b((10, 8, 2), (12, 11, 4), mat::CLAY);
            k.b((12, 9, 2), (13, 10, 3), mat::CLAY);
        }
        if rng.unit() < 0.5 {
            k.b((6, 8, 5), (9, 9, 7), mat::PLANK_PALE);
        }
        if rng.unit() < 0.5 {
            k.b((12, 8, 5), (13, 10, 6), mat::CLOTH_PALE);
            k.b((12, 10, 5), (13, 11, 6), mat::LANTERN);
        }
        return;
    }
    k.b((0, 7, 0), (16, 8, 4), mat::PLANK_DARK);
    k.b((3, 0, 4), (13, 1, 8), mat::PLANK_DARK);
    k.b((4, 1, 5), (6, 2, 6), mat::CLAY);
    k.b((9, 1, 6), (10, 2, 7), mat::CLAY);
}

/// A four-legged stool with a rung. Decayed, it lies on its side with a leg gone.
pub(super) fn stool(k: &mut Kit, _rng: &mut Rng, p: &Palette, decayed: bool) {
    if !decayed {
        for (x, z) in [(0, 0), (3, 0), (0, 3), (3, 3)] {
            k.b((x, 0, z), (x + 1, 4, z + 1), p.wood);
        }
        k.b((1, 1, 0), (3, 2, 1), p.wood);
        k.b((0, 4, 0), (4, 5, 4), p.plank);
        return;
    }
    k.b((0, 0, 0), (1, 4, 4), mat::PLANK_DARK);
    for (y, z) in [(0, 0), (0, 3), (3, 0)] {
        k.b((1, y, z), (4, y + 1, z + 1), mat::WOOD_DEAD);
    }
}

/// What can stand on a shelf: (width, how high, material, and a second part's material
/// on top, if any). A jar with a stopper, a bowl, a bottle and its cork, a crock and its
/// lid, a basket, a folded cloth.
fn shelf_things(p: &Palette) -> [(i32, i32, Voxel, Option<Voxel>); 6] {
    [
        (2, 3, mat::CLAY, Some(mat::PLANK_DARK)),
        (3, 1, mat::CLAY, None),
        (1, 2, mat::GLASS_GREEN, Some(mat::PLANK_PALE)),
        (3, 2, mat::CLAY, Some(p.plank)),
        (3, 2, mat::WICKER, None),
        (3, 1, p.accent, None),
    ]
}

/// Lays things along a shelf whose top is at `y`, from `x0` to `x1`, none taller than
/// `room`.
fn stock_shelf(k: &mut Kit, rng: &mut Rng, p: &Palette, y: i32, (x0, x1): (i32, i32), room: i32, broken: bool) {
    let things = shelf_things(p);
    let mut x = x0 + rng.int(0, 1);
    while x < x1 {
        let (w, h, v, top) = things[rng.int(0, things.len() as i32 - 1) as usize];
        if x + w > x1 {
            break;
        }
        if h + top.is_some() as i32 > room {
            x += 1;
            continue;
        }
        if broken {
            // A shard where something stood.
            if rng.unit() < 0.5 {
                k.b((x, y, 1), (x + 1, y + 1, 2), mat::CLAY);
            }
        } else {
            k.b((x, y, 0), (x + w, y + h, 3), v);
            if let Some(t) = top {
                k.b((x, y + h, 1), (x + w.min(2), y + h + 1, 2), t);
            }
        }
        x += w + rng.int(0, 1);
    }
}

/// Shelves standing against a wall, three boards and a top between two uprights, with
/// jars, bowls and bottles on them. Decayed, a board and the top are down and what was
/// on them lies broken on the floor.
pub(super) fn shelf(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    let (wood, board) = if decayed { (mat::WOOD_DEAD, mat::PLANK_DARK) } else { (p.wood, p.plank) };
    k.b((0, 0, 0), (1, 16, 4), wood);
    k.b((11, 0, 0), (12, if decayed { 13 } else { 16 }, 4), wood);
    if !decayed {
        for y in [1, 6, 11, 15] {
            k.b((1, y, 0), (11, y + 1, 4), board);
        }
        for (y, room) in [(2, 4), (7, 4), (12, 3)] {
            stock_shelf(k, rng, p, y, (1, 11), room, false);
        }
        return;
    }
    k.b((1, 1, 0), (11, 2, 4), board);
    k.b((1, 6, 0), (11, 7, 4), board);
    k.b((1, 11, 0), (6, 12, 4), board);
    stock_shelf(k, rng, p, 7, (1, 11), 4, true);
    // What fell, broken under the bottom board.
    k.b((2, 0, 1), (4, 1, 3), mat::CLAY);
    k.b((6, 0, 0), (7, 1, 2), mat::CLAY);
    k.b((8, 0, 2), (10, 1, 3), mat::GLASS_GREEN);
}

/// A chest of planks: a voxel body, a lid that overhangs it at the front, iron bands over
/// the lid and down the front, and a hasp. Decayed, the lid lies before it and it stands
/// open and empty.
pub(super) fn chest(k: &mut Kit, _rng: &mut Rng, p: &Palette, decayed: bool) {
    let board = if decayed { mat::PLANK_DARK } else { p.plank };
    let iron = if decayed { mat::RUST } else { mat::IRON };
    k.vox(0, 0, 0, board);
    k.vox(1, 0, 0, board);
    for x in [1, 6] {
        k.b((x, 1, 4), (x + 1, 4, 5), iron);
    }
    if !decayed {
        k.b((0, 4, 0), (8, 5, 5), board);
        for x in [1, 6] {
            k.b((x, 5, 0), (x + 1, 6, 5), iron);
        }
        k.b((3, 2, 4), (5, 4, 5), iron);
        return;
    }
    // Open: a dark inside within the rim.
    k.b((0, 4, 0), (8, 5, 1), board);
    k.b((0, 4, 3), (8, 5, 4), board);
    k.b((0, 4, 1), (1, 5, 3), board);
    k.b((7, 4, 1), (8, 5, 3), board);
    k.b((1, 4, 1), (7, 5, 3), mat::NEST_HOLLOW);
    k.b((0, 0, 5), (8, 1, 8), board);
    for x in [1, 6] {
        k.b((x, 1, 5), (x + 1, 2, 8), iron);
    }
}

/// A rocking cradle with a hood at its head, a pillow and a little blanket. Decayed, the
/// hood has fallen in, the foot is gone and the bedding has rotted.
pub(super) fn cradle(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    let (wood, board) = if decayed { (mat::WOOD_DEAD, mat::PLANK_DARK) } else { (p.wood, p.plank) };
    for x in [1, 10] {
        k.b((x, 0, 2), (x + 1, 1, 6), wood);
        if !(decayed && x == 10 && rng.unit() < 0.7) {
            k.b((x, 1, 1), (x + 1, 2, 3), wood);
        }
        k.b((x, 1, 5), (x + 1, 2, 7), wood);
    }
    k.b((1, 2, 1), (11, 3, 7), board);
    k.b((1, 3, 1), (11, 6, 2), board);
    if !decayed {
        k.b((1, 3, 6), (11, 6, 7), board);
        k.b((1, 3, 2), (2, 9, 6), board);
        k.b((10, 3, 2), (11, 6, 6), board);
        k.b((1, 6, 1), (5, 9, 2), board);
        k.b((1, 6, 6), (5, 9, 7), board);
        k.b((1, 9, 1), (5, 10, 7), board);
        k.b((2, 3, 2), (10, 4, 6), mat::CLOTH_PALE);
        k.b((2, 4, 2), (4, 5, 6), mat::CLOTH_PALE);
        k.b((4, 4, 2), (10, 5, 6), p.accent);
        k.hook(Hook::Sleeper(glam::IVec3::new(3, 5, 4)));
        return;
    }
    k.b((1, 3, 6), (11, 4, 7), board);
    k.b((1, 3, 2), (2, 7, 6), board);
    k.b((2, 3, 2), (5, 4, 6), mat::SACKCLOTH);
    k.b((5, 3, 2), (9, 4, 6), board);
}

/// The hearth in the middle of a hut: the ring of stones a camp has (the landmarks'
/// [`Prop::Hearth`], which serves as it is), with the fire alive in it, a pot hung over
/// it on a chain from a bar between two forked posts, and a few logs by it. Decayed, the
/// fire is long out, the bar is down and the pot lies rusting by the ring.
pub(super) fn hut_hearth(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool, seed: u32) {
    // The ring is centred on cell (10, 10), the middle of the hearth's room.
    k.prop(Prop::Hearth, p, (4, 0, 4), 0, decayed, seed);
    let wood = if decayed { mat::WOOD_DEAD } else { p.wood };
    if !decayed {
        k.b((9, 2, 9), (11, 3, 10), mat::EMBER);
        k.b((10, 2, 10), (11, 3, 12), mat::EMBER);
        k.hook(Hook::Fire(glam::IVec3::new(10, 3, 10)));
        // The posts stand outside the ring, where no stone of it rolls.
        for x in [1, 18] {
            k.b((x, 0, 10), (x + 1, 13, 11), wood);
            k.b((x - 1, 13, 10), (x, 15, 11), wood);
            k.b((x + 1, 13, 10), (x + 2, 15, 11), wood);
        }
        k.b((0, 13, 10), (20, 14, 11), wood);
        k.b((10, 8, 10), (11, 13, 11), mat::IRON);
        k.b((8, 5, 8), (13, 8, 13), mat::IRON);
        k.b((8, 8, 8), (13, 9, 9), mat::IRON);
        k.b((8, 8, 12), (13, 9, 13), mat::IRON);
        k.b((9, 4, 9), (12, 5, 12), mat::IRON);
        // Logs for the fire, stacked by it.
        k.b((15, 0, 0), (20, 2, 2), p.wood);
        k.b((16, 0, 2), (20, 2, 4), p.wood);
        k.b((15, 2, 1), (19, 4, 3), mat::PLANK_PALE);
        return;
    }
    k.b((18, 0, 10), (19, rng.int(5, 9), 11), wood);
    k.b((3, 0, 1), (17, 1, 2), wood);
    k.b((16, 0, 16), (19, 3, 19), mat::RUST);
    k.b((15, 0, 17), (16, 1, 18), mat::RUST);
}

/// The leaning plane of the upright loom: how far out from the wall its frame stands at
/// height `y`, in cells.
fn lean(y: f32) -> f32 {
    6.5 - 5.0 * y / 19.0
}

/// An upright loom leant against a wall: two uprights, the cloth beam across their tops
/// with the cloth woven so far hanging from it in stripes, the warp below held taut by
/// clay and stone weights, and the heddle bar across. Decayed, the beam has come down,
/// the warp is gone and the weights lie scattered.
pub(super) fn loom(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    let wood = if decayed { mat::WOOD_DEAD } else { p.wood };
    for x in [2.0, 14.0] {
        k.rod((x, 0.0, lean(0.0)), (x, 19.0, lean(19.0)), 2, wood);
    }
    if !decayed {
        k.b((0, 18, 0), (16, 20, 3), wood);
        // The cloth so far, in bands of colour.
        let pale = mat::CLOTH_PALE;
        for y in 12..18 {
            let z = lean(y as f32 + 0.5).floor() as i32;
            let v = if y % 3 == 1 { pale } else { p.accent };
            k.b((3, y, z), (13, y + 1, z + 1), v);
        }
        // The warp below it, each thread down to its weight.
        for (n, x) in [4, 6, 8, 10, 12].into_iter().enumerate() {
            let (top, foot) = (lean(12.0).floor(), lean(3.0).floor());
            k.rod((x as f32 + 0.5, 3.0, foot + 0.5), (x as f32 + 0.5, 12.0, top + 0.5), 1, pale);
            let w = if n % 2 == 0 { mat::CLAY } else { p.stone_dark };
            let z = foot as i32;
            k.b((x - 1, 1, z), (x + 1, 3, z + 1), w);
        }
        let z = lean(9.5).floor() as i32 + 1;
        k.b((3, 9, z), (13, 10, z + 1), wood);
        return;
    }
    k.b((0, 0, 7), (16, 2, 8), wood);
    for x in [4, 7, 10] {
        let z = rng.int(2, 4);
        k.b((x, 0, z), (x + 2, 2, z + 1), if x == 7 { p.stone_dark } else { mat::CLAY });
    }
    k.b((4, 0, 0), (10, 1, 2), mat::SACKCLOTH);
}

/// A loom pegged out flat on the ground: two beams staked down, the warp between them,
/// the cloth woven so far at the front in stripes, the beater laid across, and a mat for
/// the weaver. Decayed, the warp has rotted to a few threads, the cloth to a rag.
pub(super) fn ground_loom(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    let wood = if decayed { mat::WOOD_DEAD } else { p.wood };
    for z in [1, 9] {
        k.b((1, 0, z), (19, 2, z + 2), wood);
        for x in [0, 19] {
            if !(decayed && rng.unit() < 0.4) {
                k.b((x, 0, z), (x + 1, 3, z + 2), wood);
            }
        }
    }
    if !decayed {
        for x in (3..18).step_by(2) {
            k.b((x, 1, 3), (x + 1, 2, 6), mat::CLOTH_PALE);
        }
        for (z, v) in [(6, p.accent), (7, mat::CLOTH_PALE), (8, p.accent)] {
            k.b((2, 1, z), (18, 2, z + 1), v);
        }
        k.b((2, 2, 5), (18, 3, 6), p.plank);
        k.b((3, 0, 11), (17, 1, 12), mat::SACKCLOTH);
        return;
    }
    for x in [5, 11, 14] {
        k.b((x, 1, 3), (x + 1, 2, 9), mat::ROPE_ROTTEN);
    }
    k.b((3, 0, 5), (9, 1, 8), mat::SACKCLOTH);
}
