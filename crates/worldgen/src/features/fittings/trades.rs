//! The fittings of each trade: the smith's anvil and forge, the woodworker's bench and
//! sawhorse, the forager's racks of drying plants, the hunter's hide frame and antlers,
//! the fisher's nets, racks and traps, and the basket weaver's willow.
//!
//! Every piece is built in model space as `fittings` describes: in the columns of its
//! size, its back along `z = 0`, its front toward +Z. Boxes are in cells, a tenth of a
//! metre.

use glam::IVec3;
use voxel::Voxel;

use super::{Hook, Kit, damp, dried, weed_colour};
use crate::features::{landmarks::Palette, plants::Flower, shape::Rng};
use crate::materials as mat;

/// The smith's anvil on a block of tree trunk, a hammer left on its face, a quenching
/// bucket and tongs by it. Decayed, the block is grey and the anvil has fallen from it
/// and rusted.
pub(super) fn anvil(k: &mut Kit, _rng: &mut Rng, p: &Palette, decayed: bool) {
    let bark = if decayed { mat::WOOD_DEAD } else { p.wood };
    k.vox(1, 0, 1, bark);
    k.b((3, 0, 4), (9, 6, 8), bark);
    k.b((4, 0, 3), (8, if decayed { 5 } else { 6 }, 9), bark);
    k.b((4, 6, 4), (8, 7, 8), if decayed { mat::PLANK_DARK } else { mat::PLANK_PALE });
    if !decayed {
        let iron = mat::IRON;
        k.b((3, 7, 4), (9, 8, 8), iron);
        k.b((4, 8, 5), (8, 10, 7), iron);
        k.b((3, 10, 5), (9, 12, 7), iron);
        k.b((1, 11, 5), (3, 12, 7), iron);
        k.b((0, 11, 5), (1, 12, 6), iron);
        k.b((9, 11, 5), (10, 12, 7), iron);
        k.b((5, 12, 4), (6, 13, 6), iron);
        k.b((6, 12, 5), (10, 13, 6), p.wood);
        // The quenching bucket, and the tongs on the ground.
        k.b((0, 0, 9), (3, 1, 12), p.plank);
        k.b((0, 1, 9), (3, 3, 10), p.plank);
        k.b((0, 1, 11), (3, 3, 12), p.plank);
        k.b((0, 1, 10), (1, 3, 11), p.plank);
        k.b((2, 1, 10), (3, 3, 11), p.plank);
        k.b((1, 1, 10), (2, 2, 11), mat::WATER_STILL);
        k.b((9, 0, 10), (12, 1, 11), iron);
        return;
    }
    let rust = mat::RUST;
    k.b((9, 0, 3), (11, 2, 9), rust);
    k.b((9, 0, 9), (11, 1, 11), rust);
    k.b((9, 0, 2), (11, 1, 3), rust);
    k.b((0, 0, 9), (3, 1, 12), mat::PLANK_DARK);
    k.b((0, 1, 10), (1, 2, 12), mat::PLANK_DARK);
}

/// The smith's forge against a wall: a hearth of the local stone at waist height with a
/// stone back and a hood over the fire, the coals alight, the bellows on a frame beside
/// it blowing through an iron pipe, and a quenching trough before it. Decayed, the hood
/// has fallen, the fire is ash and the bellows' leather has rotted away.
pub(super) fn forge(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    for y in 0..2 {
        for z in 0..2 {
            for x in 0..3 {
                k.vox(x, y, z, p.block(rng));
            }
        }
    }
    for y in 2..5 {
        for x in 0..3 {
            if !(decayed && y == 4 && x != 1) {
                k.vox(x, y, 0, p.block(rng));
            }
        }
    }
    let (wood, board) = if decayed { (mat::WOOD_DEAD, mat::PLANK_DARK) } else { (p.wood, p.plank) };
    if !decayed {
        for x in 0..3 {
            k.vox(x, 4, 1, p.stone_dark);
        }
        k.vox(1, 5, 0, p.stone_dark);
        k.b((2, 8, 4), (10, 9, 8), mat::CHARCOAL);
        k.b((4, 9, 5), (8, 10, 7), mat::EMBER);
        k.hook(Hook::Fire(IVec3::new(6, 10, 6)));
        k.b((0, 8, 5), (2, 9, 6), mat::IRON);
    } else {
        k.b((2, 8, 4), (10, 9, 8), mat::ASH);
        k.b((4, 9, 5), (7, 10, 7), mat::CHARCOAL);
        // Stones of the hood, fallen.
        k.b((13, 0, 9), (16, 2, 12), p.stone_dark);
        k.b((16, 0, 10), (18, 1, 12), p.stone);
    }
    k.b((10, 8, 5), (13, 9, 6), if decayed { mat::RUST } else { mat::IRON });
    // The bellows on their frame.
    for (x, z) in [(13, 3), (19, 3), (13, 7), (19, 7)] {
        k.b((x, 0, z), (x + 1, 6, z + 1), wood);
    }
    k.b((13, 6, 3), (20, 7, 8), board);
    if !decayed {
        k.b((12, 7, 5), (13, 8, 6), mat::IRON);
        k.b((13, 7, 4), (20, 9, 7), mat::LEATHER);
        k.b((13, 9, 3), (20, 10, 8), board);
        k.b((19, 10, 5), (20, 15, 6), wood);
        k.b((19, 14, 3), (20, 15, 5), wood);
        k.b((19, 14, 6), (20, 15, 8), wood);
    }
    // The quenching trough before the hearth.
    k.b((1, 0, 8), (11, 1, 12), board);
    k.b((1, 1, 8), (11, 4, 9), board);
    if !decayed {
        k.b((1, 1, 11), (11, 4, 12), board);
        k.b((1, 1, 9), (2, 4, 11), board);
        k.b((10, 1, 9), (11, 4, 11), board);
        k.b((2, 1, 9), (10, 3, 11), mat::WATER_STILL);
    } else {
        k.b((10, 1, 9), (11, 3, 11), board);
    }
}

/// A grindstone: a wheel of the local stone on an iron axle between two uprights, its
/// foot in a trough of water, turned by a crank. Decayed, the wheel lies cracked in two
/// on the ground and half the frame is gone.
pub(super) fn grindstone(k: &mut Kit, _rng: &mut Rng, p: &Palette, decayed: bool) {
    let wood = if decayed { mat::WOOD_DEAD } else { p.wood };
    k.b((1, 0, 2), (11, 1, 3), wood);
    k.b((5, 0, 2), (7, if decayed { 7 } else { 9 }, 3), wood);
    if !decayed {
        k.b((1, 0, 9), (11, 1, 10), wood);
        k.b((5, 0, 9), (7, 9, 10), wood);
        k.round_xy((6.0, 7.0), 4.5, 0.0, (5, 7), p.stone);
        k.b((5, 6, 3), (7, 8, 5), mat::IRON);
        k.b((5, 6, 7), (7, 8, 9), mat::IRON);
        k.b((5, 6, 10), (7, 8, 11), mat::IRON);
        k.b((6, 8, 10), (7, 12, 11), mat::IRON);
        k.b((6, 11, 11), (7, 12, 12), p.wood);
        // The trough the wheel turns through.
        k.b((2, 0, 4), (10, 1, 8), p.plank);
        k.b((2, 1, 4), (10, 3, 5), p.plank);
        k.b((2, 1, 7), (10, 3, 8), p.plank);
        k.b((2, 1, 5), (3, 3, 7), p.plank);
        k.b((9, 1, 5), (10, 3, 7), p.plank);
        k.b((3, 1, 5), (9, 2, 7), mat::WATER_STILL);
        return;
    }
    // The wheel, cracked in two where it fell.
    for (z0, z1, x0, x1) in super::round_rows(7.5, 6.0, 4.0, 0.0) {
        if x0 < 6 {
            k.b((x0, 0, z0), (x1.min(6), 2, z1), p.stone);
        }
        if x1 > 6 {
            k.b((x0.max(6) + 1, 0, z0 + 1), (x1 + 1, 1, z1 + 1), p.stone_dark);
        }
    }
}

/// A woodworker's bench: a thick top on four legs with a shelf below, a board on the
/// wall behind it hung with a saw, a mallet and chisels, a plane and shavings on the top.
/// Decayed, half the top has fallen, and only a rusted saw is left on the wall.
pub(super) fn workbench(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    let (wood, board) = if decayed { (mat::WOOD_DEAD, mat::PLANK_DARK) } else { (p.wood, p.plank) };
    for (x, z) in [(1, 1), (17, 1), (1, 5), (17, 5)] {
        let h = if decayed && (x, z) == (17, 5) { 4 } else { 7 };
        k.b((x, 0, z), (x + 2, h, z + 2), wood);
    }
    k.b((3, 2, 2), (17, 3, 6), board);
    k.b((0, 9, 0), (20, 16, 1), board);
    let iron = if decayed { mat::RUST } else { mat::IRON };
    k.b((2, 11, 1), (7, 13, 2), iron);
    k.b((7, 11, 1), (9, 13, 2), wood);
    if !decayed {
        k.b((0, 7, 0), (20, 9, 8), board);
        k.b((12, 10, 1), (13, 12, 2), p.wood);
        k.b((11, 12, 1), (14, 14, 2), p.wood);
        for x in [15, 17] {
            k.b((x, 10, 1), (x + 1, 12, 2), iron);
            k.b((x, 12, 1), (x + 1, 14, 2), p.wood);
        }
        k.b((12, 9, 3), (16, 10, 5), p.wood);
        k.b((13, 10, 3), (14, 11, 5), iron);
        k.b((1, 9, 5), (10, 10, 7), p.plank);
        k.b((4, 10, 5), (6, 11, 6), mat::PLANK_PALE);
        k.b(((rng.int(5, 8)), 0, 6), (rng.int(10, 13), 1, 8), mat::PLANK_PALE);
        return;
    }
    k.b((0, 7, 0), (20, 9, 4), board);
    k.b((4, 0, 6), (16, 1, 8), board);
}

/// A sawbuck: two crossed pairs of legs joined by a stretcher, a log in the crooks of
/// them with a saw left in the cut, sawdust under it. Decayed, one end has fallen, the
/// log lies grey on the ground and the saw rusts beside it.
pub(super) fn sawhorse(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    if !decayed {
        for x in [2.5, 9.5] {
            k.rod((x, 0.0, 0.5), (x, 9.0, 7.5), 1, p.wood);
            k.rod((x, 0.0, 7.5), (x, 9.0, 0.5), 1, p.wood);
        }
        k.b((3, 4, 3), (9, 5, 5), p.wood);
        k.b((1, 6, 3), (11, 9, 6), p.wood);
        k.b((0, 6, 3), (1, 9, 6), mat::PLANK_PALE);
        k.b((11, 6, 3), (12, 9, 6), mat::PLANK_PALE);
        k.b((6, 7, 1), (7, 10, 7), mat::IRON);
        k.b((6, 8, 0), (7, 11, 1), p.wood);
        k.b((6, 8, 7), (7, 11, 8), p.wood);
        k.b((4, 0, 3), (8, 1, 6), mat::PLANK_PALE);
        return;
    }
    let dead = mat::WOOD_DEAD;
    k.rod((9.5, 0.0, 0.5), (9.5, 9.0, 7.5), 1, dead);
    k.rod((9.5, 0.0, 7.5), (9.5, 9.0, 0.5), 1, dead);
    k.b((1, 0, 0), (2, 1, 8), dead);
    k.b((0, 0, 3), (3, 1, 4), dead);
    k.b((2, 0, 4), (9, 3, 7), dead);
    k.b((3, 0, 1), (8, 1, 2), mat::RUST);
    if damp(p) && rng.unit() < 0.8 {
        k.b((5, 3, 5), (6, 4, 6), mat::MUSHROOM_STEM);
        k.b((4, 4, 4), (7, 5, 7), mat::MUSHROOM_TAN);
    }
}

/// One log lying along X from `x0` to `x1`, its foot at layer of cells `y` and its side
/// at `z`: four cells across, round enough, with its sawn ends pale.
fn log(k: &mut Kit, (x0, x1): (i32, i32), y: i32, z: i32, bark: Voxel, end: Voxel) {
    for (a, b) in [(x0, x0 + 1), (x0 + 1, x1 - 1), (x1 - 1, x1)] {
        let v = if a == x0 + 1 { bark } else { end };
        k.b((a, y, z + 1), (b, y + 4, z + 3), v);
        k.b((a, y + 1, z), (b, y + 3, z + 4), v);
    }
}

/// Long logs laid up on two skids, three, two and one, waiting for the saw. Decayed, the
/// skids have rotted, the logs lie grey on the ground with fungus on them.
pub(super) fn log_deck(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    if !decayed {
        k.b((4, 0, 0), (6, 2, 12), p.wood);
        k.b((26, 0, 0), (28, 2, 12), p.wood);
        // The middle of the pile is voxels, so a player bumps into it.
        for x in 1..7 {
            k.vox(x, 1, 1, p.wood);
        }
        let len = |rng: &mut Rng| (1 + rng.int(0, 1), 30 + rng.int(0, 1));
        for z in [0, 4, 8] {
            log(k, len(rng), 2, z, p.wood, mat::PLANK_PALE);
        }
        for z in [2, 6] {
            log(k, len(rng), 5, z, p.wood, mat::PLANK_PALE);
        }
        log(k, len(rng), 8, 4, p.wood, mat::PLANK_PALE);
        return;
    }
    for z in [0, 4, 8] {
        log(k, (1 + rng.int(0, 2), 29 + rng.int(0, 2)), 0, z, mat::WOOD_DEAD, mat::PLANK_DARK);
    }
    log(k, (6, 26), 3, 2, mat::WOOD_DEAD, mat::PLANK_DARK);
    if damp(p) {
        for x in [9, 17, 22] {
            if rng.unit() < 0.7 {
                k.b((x, 7, 4), (x + 1, 8, 5), mat::MUSHROOM_STEM);
                k.b((x - 1, 8, 3), (x + 2, 9, 6), if rng.unit() < 0.3 { mat::MUSHROOM_RED } else { mat::MUSHROOM_TAN });
            }
        }
    }
    // Grass come up through the gaps along the top.
    let weed = weed_colour(p);
    for x in [3, 14, 27] {
        k.weed((x, 4, 1), (x + 1, rng.int(6, 8), 2), weed);
    }
}

/// One bunch of a plant hung head down from a rail whose underside is at `top`: the tie,
/// the stems and the heads, centred on column of cells `x` and row `z`. A mushroom is
/// dried in slices on a string instead. Brown when it has hung too long.
fn bunch(k: &mut Kit, x: i32, top: i32, z: i32, f: Flower, old: bool) {
    let tie = if old { mat::ROPE_ROTTEN } else { mat::ROPE };
    let head = if old { mat::DRIED_BROWN } else { dried(f) };
    if f.is_mushroom() {
        k.hang((x, top - 6, z), (x + 1, top, z + 1), tie);
        for y in [top - 4, top - 2] {
            k.hang((x - 1, y, z - 1), (x + 2, y + 1, z + 2), head);
        }
        return;
    }
    k.b((x, top - 1, z), (x + 1, top, z + 1), tie);
    k.hang((x, top - 3, z), (x + 1, top - 1, z + 1), if old { mat::DRIED_BROWN } else { mat::DRIED_HERB });
    k.part((x - 1, top - 5, z - 1), (x + 2, top - 3, z + 2), head, true, true);
}

/// The forager's drying frame: two end frames joined by rails, hung with bunches of the
/// plants about the place, a kind to a bunch. Decayed, a rail is down, and the few
/// bunches left have gone brown.
pub(super) fn herb_rack(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool, herbs: &[Flower]) {
    let wood = if decayed { mat::WOOD_DEAD } else { p.wood };
    for x in [0, 11] {
        k.b((x, 0, 0), (x + 1, 1, 8), wood);
        k.b((x, 1, 1), (x + 1, 16, 2), wood);
        k.b((x, 1, 6), (x + 1, 16, 7), wood);
        k.b((x, 15, 2), (x + 1, 16, 6), wood);
        k.b((x, 9, 2), (x + 1, 10, 6), wood);
    }
    let mut n = rng.int(0, herbs.len() as i32 - 1) as usize;
    let mut next = |rng: &mut Rng| {
        n += 1 + (rng.unit() < 0.3) as usize;
        herbs[n % herbs.len()]
    };
    if !decayed {
        k.b((1, 15, 1), (11, 16, 2), wood);
        k.b((1, 15, 6), (11, 16, 7), wood);
        k.b((1, 9, 3), (11, 10, 5), wood);
        for x in [2, 5, 8] {
            bunch(k, x, 15, 1, next(rng), false);
            bunch(k, x, 15, 6, next(rng), false);
        }
        for x in [3, 6, 9] {
            bunch(k, x, 9, 4, next(rng), false);
        }
        return;
    }
    k.b((1, 15, 1), (11, 16, 2), wood);
    k.b((1, 0, 3), (11, 1, 5), wood);
    for x in [2, 8] {
        if rng.unit() < 0.7 {
            bunch(k, x, 15, 1, next(rng), true);
        }
    }
    k.b((4, 0, 6), (7, 1, 8), mat::DRIED_BROWN);
}

/// Bunches of the plants about the place hung head down from a batten under the eaves.
/// The batten's top is the top of the fitting: it is fixed to whatever it hangs from.
/// Decayed, a couple of brown bunches and bare ties are left.
pub(super) fn herb_bunches(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool, herbs: &[Flower]) {
    k.b((0, 6, 0), (16, 8, 2), if decayed { mat::WOOD_DEAD } else { p.wood });
    let first = rng.int(0, herbs.len() as i32 - 1) as usize;
    for (i, x) in [1, 4, 7, 10, 13].into_iter().enumerate() {
        let f = herbs[(first + i) % herbs.len()];
        if !decayed {
            bunch(k, x, 6, 1, f, false);
        } else if rng.unit() < 0.4 {
            bunch(k, x, 6, 1, f, true);
        } else {
            k.hang((x, 4, 1), (x + 1, 6, 2), mat::ROPE_ROTTEN);
        }
    }
}

/// A hide stretched on a square frame of poles, laced to it all round, the frame stood
/// upright on two props. Decayed, the hide has torn away to a rag on one side, a pole has
/// broken and the top bar lies in the grass.
pub(super) fn hide_frame(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    let wood = if decayed { mat::WOOD_DEAD } else { p.wood };
    let rope = if decayed { mat::ROPE_ROTTEN } else { mat::ROPE };
    k.b((1, 0, 3), (2, 19, 4), wood);
    k.b((18, 0, 3), (19, if decayed { rng.int(9, 12) } else { 19 }, 4), wood);
    k.b((0, 3, 3), (20, 4, 4), wood);
    k.rod((1.5, 16.5, 2.5), (1.5, 0.0, 0.5), 1, wood);
    if !decayed {
        k.rod((18.5, 16.5, 2.5), (18.5, 0.0, 0.5), 1, wood);
        k.b((0, 17, 3), (20, 18, 4), wood);
        k.b((6, 5, 3), (14, 6, 4), mat::HIDE);
        k.b((4, 6, 3), (16, 15, 4), mat::HIDE);
        k.b((6, 15, 3), (14, 16, 4), mat::HIDE);
        for y in [7, 10, 13] {
            k.b((2, y, 3), (4, y + 1, 4), rope);
            k.b((16, y, 3), (18, y + 1, 4), rope);
        }
        for x in [7, 10, 13] {
            k.b((x, 4, 3), (x + 1, 5, 4), rope);
            k.b((x, 16, 3), (x + 1, 17, 4), rope);
        }
        return;
    }
    k.b((4, 5, 3), (9, 12, 4), mat::LEATHER);
    for y in [7, 10] {
        k.b((2, y, 3), (4, y + 1, 4), rope);
    }
    k.b((0, 0, 5), (20, 1, 6), wood);
    k.b((10, 0, 6), (14, 1, 8), mat::LEATHER);
}

/// One set of antlers on its skull plate, centred on column of cells `cx`, the plate's
/// foot at `y`, standing out from a board whose face is at `z`.
fn antlers(k: &mut Kit, cx: i32, y: i32, z: i32, v: Voxel) {
    k.b((cx - 1, y, z), (cx + 1, y + 3, z + 1), v);
    for s in [-1, 1] {
        // Mirrored: a part from `a` to `b` cells out from the middle.
        let mut side = |a: i32, b: i32, y0: i32, y1: i32| {
            let (x0, x1) = if s < 0 { (cx - b, cx - a) } else { (cx + a, cx + b) };
            k.b((x0, y + y0, z), (x1, y + y1, z + 1), v);
        };
        side(1, 2, 2, 4);
        side(2, 3, 3, 7);
        side(3, 5, 5, 6);
        side(3, 4, 6, 9);
    }
}

/// A board between two posts with a set of antlers mounted on it, and shed antlers
/// heaped at its foot. Decayed, the board has fallen and the antlers lie in the grass.
pub(super) fn antler_rack(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    let (wood, board) = if decayed { (mat::WOOD_DEAD, mat::PLANK_DARK) } else { (p.wood, p.plank) };
    for x in [0, 13] {
        k.b((x, 0, 1), (x + 3, 1, 7), wood);
        let h = if decayed && x == 13 { rng.int(8, 12) } else { 18 };
        k.b((x + 1, 1, 3), (x + 2, h, 4), wood);
    }
    if !decayed {
        k.b((2, 10, 3), (14, 13, 4), board);
        antlers(k, 8, 10, 4, mat::ANTLER);
        // Smaller ones, spikes of a young stag, on each post's top.
        for x in [1, 14] {
            k.b((x, 18, 3), (x + 1, 19, 4), mat::ANTLER);
        }
        k.b((4, 0, 5), (9, 1, 6), mat::ANTLER);
        k.b((6, 0, 6), (7, 1, 8), mat::ANTLER);
        k.b((9, 0, 4), (12, 1, 5), mat::ANTLER);
        k.b((10, 1, 4), (11, 2, 5), mat::ANTLER);
        return;
    }
    k.b((3, 0, 4), (13, 1, 7), board);
    k.b((4, 1, 4), (8, 2, 5), mat::ANTLER);
    k.b((6, 1, 5), (7, 3, 6), mat::ANTLER);
    k.b((9, 1, 6), (12, 2, 7), mat::ANTLER);
}

/// One side of a net hanging from the pole, a mesh of tarred cord at `z` from `bottom` up
/// to the pole, with its floats or weights along its foot. Torn, strands are missing.
fn net(k: &mut Kit, rng: &mut Rng, z: i32, bottom: i32, foot: Voxel, cord: Voxel, torn: bool) {
    k.b((0, 17, z), (24, 19, z + 1), cord);
    for y in (bottom + 1..17).step_by(2) {
        if !(torn && y < 12) {
            k.b((0, y, z), (24, y + 1, z + 1), cord);
        }
    }
    for x in (0..24).step_by(2) {
        let lo = if torn { rng.int(bottom, 13) } else { bottom };
        if !(torn && rng.unit() < 0.4) {
            k.b((x, lo, z), (x + 1, 17, z + 1), cord);
        }
    }
    if !torn {
        for x in [1, 7, 13, 19] {
            k.b((x, bottom - 1, z), (x + 2, bottom, z + 1), foot);
        }
    }
}

/// Nets hung over a pole on three posts to dry and be mended, one side with its corks
/// along its foot and the other with its stone weights. Decayed, the middle post has
/// broken and the nets hang in rotten tatters.
pub(super) fn net_rack(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    let wood = if decayed { mat::WOOD_DEAD } else { p.wood };
    for x in [1, 11, 21] {
        let h = if decayed && x == 11 { rng.int(6, 11) } else { 18 };
        k.b((x, 0, 3), (x + 2, h, 5), wood);
    }
    k.b((0, 18, 3), (24, 19, 5), wood);
    let cord = if decayed { mat::ROPE_ROTTEN } else { mat::NET };
    net(k, rng, 5, 6, mat::PLANK_PALE, cord, decayed);
    net(k, rng, 2, 10, p.stone_dark, cord, decayed);
}

/// A frame of two posts with two rails, fish hung over the rails by their tails in pairs,
/// one each side. Decayed, the lower rail is gone and only a few fish bones hang.
pub(super) fn fish_rack(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    let wood = if decayed { mat::WOOD_DEAD } else { p.wood };
    for x in [0, 17] {
        k.b((x, 0, 1), (x + 3, 1, 7), wood);
        k.b((x + 1, 1, 3), (x + 2, 19, 5), wood);
    }
    let rails: &[i32] = if decayed { &[18] } else { &[18, 10] };
    for &y in rails {
        k.b((2, y, 3), (18, y + 1, 5), wood);
        for x in [3, 6, 9, 12, 15] {
            if decayed && rng.unit() < 0.5 {
                continue;
            }
            let tie = if decayed { mat::ROPE_ROTTEN } else { mat::ROPE };
            k.b((x, y + 1, 2), (x + 1, y + 2, 6), tie);
            for z in [2, 5] {
                k.b((x, y, z), (x + 1, y + 1, z + 1), tie);
                if decayed {
                    k.hang((x, y - 5, z), (x + 1, y, z + 1), mat::ANTLER);
                    continue;
                }
                let fish = if rng.unit() < 0.25 { mat::FISH } else { mat::FISH_DRIED };
                k.hang((x, y - 5, z), (x + 2, y, z + 1), fish);
                k.hang((x, y - 6, z), (x + 2, y - 5, z + 1), mat::FISH_DRIED);
            }
        }
    }
}

/// One eel trap lying along X with its side at `z`: a wicker funnel, its dark mouth at
/// X 0 and its plugged tail at the far end.
fn eel_trap_at(k: &mut Kit, z: i32, v: Voxel) {
    k.b((0, 0, z + 1), (1, 1, z + 3), v);
    k.b((0, 3, z + 1), (1, 4, z + 3), v);
    k.b((0, 1, z), (1, 3, z + 1), v);
    k.b((0, 1, z + 3), (1, 3, z + 4), v);
    k.b((0, 1, z + 1), (1, 3, z + 3), mat::NEST_HOLLOW);
    k.b((1, 0, z + 1), (5, 4, z + 3), v);
    k.b((1, 1, z), (5, 3, z + 4), v);
    k.b((5, 0, z + 1), (9, 3, z + 3), v);
    k.b((9, 0, z + 1), (11, 2, z + 3), v);
    k.b((11, 0, z + 1), (12, 1, z + 3), mat::PLANK_DARK);
}

/// Two wicker eel traps laid side by side on the bank. Decayed, one is gone and the
/// other crushed flat, its withies coming loose.
pub(super) fn eel_trap(k: &mut Kit, rng: &mut Rng, _p: &Palette, decayed: bool) {
    if !decayed {
        eel_trap_at(k, 0, mat::WITHY);
        if rng.unit() < 0.8 {
            eel_trap_at(k, 4, mat::WITHY);
        }
        return;
    }
    let v = mat::ROPE_ROTTEN;
    k.b((0, 0, 1), (10, 2, 4), v);
    k.b((10, 0, 2), (12, 1, 3), v);
    for (x, z) in [(2, 4), (5, 5), (8, 4)] {
        k.b((x, 0, z), (x + 1, 1, z + rng.int(1, 3)), v);
    }
}

/// One lobster pot with its base at layer of cells `y`: a board base, a wicker dome with
/// its mouth on top, and two darker hoops over it.
fn lobster_pot_at(k: &mut Kit, y: i32, base: Voxel) {
    k.b((1, y, 1), (7, y + 1, 7), base);
    k.b((1, y + 1, 1), (7, y + 4, 7), mat::WICKER);
    k.b((2, y + 4, 2), (6, y + 5, 3), mat::WICKER);
    k.b((2, y + 4, 5), (6, y + 5, 6), mat::WICKER);
    k.b((2, y + 4, 3), (3, y + 5, 5), mat::WICKER);
    k.b((5, y + 4, 3), (6, y + 5, 5), mat::WICKER);
    k.b((3, y + 4, 3), (5, y + 5, 5), mat::NEST_HOLLOW);
    for x in [2, 5] {
        k.b((x, y + 1, 0), (x + 1, y + 4, 1), mat::WITHY);
        k.b((x, y + 1, 7), (x + 1, y + 4, 8), mat::WITHY);
        k.b((x, y + 5, 2), (x + 1, y + 6, 6), mat::WITHY);
    }
}

/// Lobster pots on the shore, one, or one stacked on another. Decayed, a pot crushed flat
/// on its rotten base, broken hoops standing out of it.
pub(super) fn lobster_pot(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    if !decayed {
        lobster_pot_at(k, 0, p.plank);
        if rng.unit() < 0.6 {
            lobster_pot_at(k, 6, p.plank);
        }
        return;
    }
    k.b((1, 0, 1), (7, 1, 4), mat::PLANK_DARK);
    k.b((1, 0, 5), (5, 1, 7), mat::PLANK_DARK);
    k.b((1, 1, 1), (7, 2, 7), mat::ROPE_ROTTEN);
    k.b((2, 2, 1), (3, 4, 2), mat::ROPE_ROTTEN);
    k.b((5, 2, 5), (6, 3, 6), mat::ROPE_ROTTEN);
}

/// One bundle of willow rods lying along X, its corner at `(z, y)`: rods tied in two
/// places, their butts ragged at one end and their tips thin at the other.
fn withies(k: &mut Kit, rng: &mut Rng, y: i32, z: i32, v: Voxel) {
    for (a, b, w) in [(1, 4, v), (4, 5, mat::ROPE), (5, 13, v), (13, 14, mat::ROPE), (14, 17, v)] {
        k.b((a, y, z), (b, y + 3, z + 3), w);
    }
    k.b((0, y + 1, z + 1), (1, y + 2, z + 2), v);
    for _ in 0..3 {
        let (dy, dz) = (rng.int(0, 2), rng.int(0, 2));
        k.b((17, y + dy, z + dz), (18 + rng.int(0, 1), y + dy + 1, z + dz + 1), v);
    }
}

/// The basket weaver's willow: bundles of rods, cut and peeled, stacked two and one.
/// Decayed, a single grey row with the rods spilling loose and grass through them.
pub(super) fn withy_bundles(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    if !decayed {
        withies(k, rng, 0, 1, mat::WITHY);
        withies(k, rng, 0, 4, mat::WITHY);
        let z = 2 + rng.int(0, 1);
        withies(k, rng, 3, z, mat::WICKER);
        return;
    }
    let dead = mat::WOOD_DEAD;
    k.b((1, 0, 1), (17, 2, 4), dead);
    for (x, z, len) in [(2, 5, 12), (6, 6, 9), (0, 4, 7)] {
        k.b((x, 0, z), (x + len, 1, z + 1), dead);
    }
    let weed = weed_colour(p);
    for x in [4, 11] {
        k.weed((x, 2, 2), (x + 1, rng.int(4, 6), 3), weed);
    }
}

/// The weaver's soaking trough: a long box of boards full of water with willow rods lying
/// in it, held down by a stone and coming up over one end. Decayed, dry and fallen open
/// at the front, grass growing in it.
pub(super) fn soaking_trough(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    let board = if decayed { mat::PLANK_DARK } else { p.plank };
    k.b((1, 0, 1), (23, 1, 7), board);
    k.b((1, 1, 1), (23, 5, 2), board);
    k.b((0, 0, 1), (1, 5, 7), board);
    k.b((23, 0, 1), (24, 5, 7), board);
    if !decayed {
        k.b((1, 1, 6), (23, 5, 7), board);
        k.b((1, 1, 2), (23, 4, 6), mat::WATER_STILL);
        k.b((2, 4, 3), (22, 5, 4), mat::WITHY);
        k.b((3, 4, 4), (21, 5, 5), mat::WITHY);
        k.b((21, 5, 3), (24, 6, 4), mat::WITHY);
        k.b((9, 5, 2), (13, 7, 6), p.stone);
        return;
    }
    k.b((1, 0, 7), (23, 1, 8), board);
    k.b((3, 1, 3), (18, 2, 4), mat::WOOD_DEAD);
    let weed = weed_colour(p);
    for x in [4, 9, 15, 20] {
        k.weed((x, 1, 4), (x + 1, rng.int(3, 6), 5), weed);
    }
}

/// A stack of square baskets, a tall creel with its straps, and a small basket with a
/// handle. Decayed, the stack crushed, the creel fallen on its side and rotting.
pub(super) fn basket_stack(k: &mut Kit, _rng: &mut Rng, _p: &Palette, decayed: bool) {
    if !decayed {
        let (w, d) = (mat::WICKER, mat::WITHY);
        k.b((1, 0, 1), (7, 3, 7), w);
        k.b((1, 3, 1), (7, 4, 7), d);
        k.b((1, 4, 1), (7, 6, 7), w);
        // The top basket's rim round its dark inside.
        k.b((1, 6, 1), (7, 7, 2), d);
        k.b((1, 6, 6), (7, 7, 7), d);
        k.b((1, 6, 2), (2, 7, 6), d);
        k.b((6, 6, 2), (7, 7, 6), d);
        k.b((2, 6, 2), (6, 7, 6), mat::NEST_HOLLOW);
        k.b((8, 0, 1), (11, 7, 4), w);
        k.b((8, 7, 1), (11, 8, 2), d);
        k.b((8, 7, 3), (11, 8, 4), d);
        k.b((8, 7, 2), (9, 8, 3), d);
        k.b((10, 7, 2), (11, 8, 3), d);
        k.b((9, 7, 2), (10, 8, 3), mat::NEST_HOLLOW);
        k.b((8, 1, 0), (9, 6, 1), mat::LEATHER);
        k.b((10, 1, 0), (11, 6, 1), mat::LEATHER);
        k.b((8, 0, 5), (11, 2, 8), w);
        k.b((8, 2, 6), (9, 4, 7), d);
        k.b((10, 2, 6), (11, 4, 7), d);
        k.b((8, 4, 6), (11, 5, 7), d);
        return;
    }
    let rot = mat::ROPE_ROTTEN;
    k.b((1, 0, 1), (7, 2, 7), rot);
    k.b((2, 2, 2), (3, 4, 3), rot);
    k.b((5, 2, 5), (6, 3, 6), rot);
    k.b((8, 0, 1), (12, 3, 4), rot);
    k.b((8, 0, 5), (9, 1, 7), mat::LEATHER);
}
