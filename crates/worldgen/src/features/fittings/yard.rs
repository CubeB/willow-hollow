//! What stands in the yards and lanes: troughs, carts and water butts, the washing line,
//! wells of three kinds, a market stall, the hen coop, hives, hay ricks, log stacks and
//! hurdles.
//!
//! Every piece is built in model space as `fittings` describes: in the columns of its
//! size, its back along `z = 0`, its front toward +Z. Boxes are in cells, a tenth of a
//! metre.

use glam::IVec3;
use voxel::Voxel;

use super::{Hook, Kit, damp, region, round_rows, weed_colour};
use crate::features::{landmarks::Palette, shape::Rng};
use crate::{materials as mat, settlements::Region};

/// A trough for beasts to drink from, hewn from a block of the local stone, or in the
/// forests a hollowed log. The body is voxels; the rim stands above them round the
/// water. Decayed, dry, broken at the rim, with grass growing in the mud in it.
pub(super) fn water_trough(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    let log = matches!(region(p), Region::Northern | Region::OldGrowth | Region::Rainforest);
    let v = if log { if decayed { mat::WOOD_DEAD } else { p.wood } } else { p.stone };
    for x in 0..4 {
        for z in 0..2 {
            k.vox(x, 0, z, if log || !decayed { v } else { p.block(rng) });
        }
    }
    let rim = if !log && decayed && damp(p) { mat::MOSSY_STONE } else { v };
    k.b((0, 4, 0), (16, 6, 1), rim);
    k.b((0, 4, 1), (1, 6, 7), v);
    k.b((15, 4, 1), (16, 6, 7), v);
    if !decayed {
        k.b((0, 4, 7), (16, 6, 8), v);
        k.b((1, 4, 1), (15, 5, 7), mat::WATER_STILL);
        k.hook(Hook::Water(IVec3::new(8, 5, 4)));
        return;
    }
    // A piece of the front broken out, and mud and grass where the water was.
    let gap = rng.int(3, 9);
    k.b((0, 4, 7), (gap, 6, 8), v);
    k.b((gap + 3, 4, 7), (16, 5, 8), v);
    k.b((1, 4, 1), (15, 5, 7), mat::MUD);
    let weed = weed_colour(p);
    for x in [3, 8, 12] {
        k.weed((x, 5, rng.int(2, 5)), (x + 1, rng.int(7, 8), 6), weed);
    }
}

/// One cart wheel standing upright on the ground, facing along Z between `z` and `z + 1`,
/// centred on column of cells `cx`: a rim, four spokes and a hub.
fn wheel(k: &mut Kit, cx: f32, z: i32, v: Voxel) {
    k.round_xy((cx, 4.5), 4.5, 3.5, (z, z + 1), v);
    let c = cx.floor() as i32;
    k.b((c, 1, z), (c + 1, 8, z + 1), v);
    k.b((c - 3, 4, z), (c + 4, 5, z + 1), v);
}

/// A handcart: a box of boards on two wheels, its shafts resting on a pair of legs, a
/// sack or two in it. Decayed, a wheel is gone and the body lies broken on the ground.
pub(super) fn handcart(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    if !decayed {
        let (wood, board) = (p.wood, p.plank);
        wheel(k, 6.5, 0, wood);
        wheel(k, 6.5, 11, wood);
        k.b((6, 4, 1), (7, 5, 11), wood);
        k.b((0, 5, 2), (12, 6, 10), board);
        k.b((0, 6, 2), (12, 9, 3), board);
        k.b((0, 6, 9), (12, 9, 10), board);
        k.b((0, 6, 3), (1, 9, 9), board);
        k.b((11, 6, 3), (12, 9, 9), board);
        for z in [2, 9] {
            k.b((12, 5, z), (20, 6, z + 1), wood);
            k.b((11, 0, z), (12, 5, z + 1), wood);
        }
        k.b((19, 5, 3), (20, 6, 9), wood);
        if rng.unit() < 0.8 {
            k.b((2, 6, 4), (6, 9, 8), mat::SACKCLOTH);
            k.b((3, 9, 5), (5, 10, 7), mat::ROPE);
        }
        if rng.unit() < 0.5 {
            k.b((6, 6, 3), (10, 8, 8), mat::SACKCLOTH);
        }
        return;
    }
    let (wood, board) = (mat::WOOD_DEAD, mat::PLANK_DARK);
    wheel(k, 6.5, 0, wood);
    k.b((6, 4, 1), (7, 5, 3), wood);
    k.b((0, 0, 2), (5, 1, 10), board);
    k.b((6, 0, 3), (12, 1, 10), board);
    k.b((0, 0, 10), (12, 1, 11), board);
    k.b((12, 0, 2), (19, 1, 3), wood);
    let weed = weed_colour(p);
    for (x, z) in [(5, 5), (5, 8), (2, 11)] {
        k.weed((x, 0, z), (x + 1, rng.int(3, 5), z + 1), weed);
    }
}

/// A barrel on a stone stand under the eaves, catching the rain, with a tap near its
/// foot. Decayed, it has burst on its stand and a rusted hoop lies before it.
pub(super) fn water_butt(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    k.vox(1, 0, 1, p.stone);
    if !decayed {
        let staves = p.plank;
        k.vox(1, 1, 1, staves);
        k.vox(1, 2, 1, staves);
        k.b((3, 4, 3), (9, 6, 9), staves);
        k.b((3, 10, 3), (9, 12, 9), staves);
        for (y0, y1, v) in [(6, 7, mat::IRON), (7, 9, staves), (9, 10, mat::IRON)] {
            k.b((2, y0, 3), (10, y1, 9), v);
            k.b((3, y0, 2), (9, y1, 10), v);
        }
        k.b((4, 12, 4), (8, 13, 8), mat::PLANK_DARK);
        k.b((5, 5, 9), (6, 6, 11), p.wood);
        k.hook(Hook::Water(IVec3::new(6, 12, 6)));
        return;
    }
    k.b((3, 4, 3), (9, 5, 9), mat::PLANK_DARK);
    let staves = [((3, 3), (4, 6)), ((3, 6), (4, 9)), ((4, 3), (6, 4)), ((6, 3), (9, 4)), ((8, 4), (9, 6)), ((8, 6), (9, 9)), ((4, 8), (6, 9)), ((6, 8), (8, 9))];
    for ((x0, z0), (x1, z1)) in staves {
        if rng.unit() < 0.5 {
            k.b((x0, 5, z0), (x1, rng.int(6, 10), z1), mat::PLANK_DARK);
        }
    }
    for (lo, hi) in [((1, 9), (7, 10)), ((1, 11), (7, 12)), ((1, 10), (2, 11)), ((6, 10), (7, 11))] {
        k.b((lo.0, 0, lo.1), (hi.0, 1, hi.1), mat::RUST);
    }
    k.b((9, 0, 1), (10, 1, 8), mat::PLANK_DARK);
}

/// The desert's water butt: a great clay jar, round in the belly, stood in a ring of
/// stones with a board over its mouth. Decayed, cracked across, its top half fallen in
/// shards about it.
pub(super) fn water_jar(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    k.round_xz((6.0, 6.0), 5.0, 3.0, (0, 1), p.stone);
    k.vox(1, 0, 1, mat::CLAY);
    if !decayed {
        k.vox(1, 1, 1, mat::CLAY);
        k.b((3, 1, 3), (9, 8, 9), mat::CLAY);
        k.b((2, 2, 4), (10, 6, 8), mat::CLAY);
        k.b((4, 2, 2), (8, 6, 10), mat::CLAY);
        k.b((5, 8, 5), (7, 10, 7), mat::CLAY);
        k.b((4, 10, 4), (8, 11, 8), mat::CLAY);
        k.b((4, 11, 4), (8, 12, 8), p.plank);
        k.hook(Hook::Water(IVec3::new(6, 12, 6)));
        return;
    }
    k.b((3, 1, 3), (9, 4, 9), mat::CLAY);
    k.b((4, 4, 4), (8, 5, 8), mat::NEST_HOLLOW);
    for _ in 0..4 {
        // Outside the ring of stones it stood in.
        let x = rng.int(0, 9);
        k.b((x, 0, 11), (x + 2, 1, 12), mat::CLAY);
    }
}

/// Two posts with a bar across the top of each and a line sagging between them, propped
/// in the middle on some days. The line's ends are its [`Hook::Line`], where the washing
/// will hang. Decayed, one post has broken and the line trails on the ground.
pub(super) fn washing_line(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    let wood = if decayed { mat::WOOD_DEAD } else { p.wood };
    k.b((1, 0, 1), (3, 18, 3), wood);
    k.b((0, 16, 1), (4, 17, 3), wood);
    if !decayed {
        k.b((37, 0, 1), (39, 18, 3), wood);
        k.b((36, 16, 1), (40, 17, 3), wood);
        let propped = rng.unit() < 0.5;
        let low = if propped { 13.5 } else { 12.5 };
        k.rod((3.0, 15.5, 2.5), (20.5, low, 2.5), 1, mat::ROPE);
        k.rod((20.5, low, 2.5), (37.0, 15.5, 2.5), 1, mat::ROPE);
        if propped {
            k.rod((20.5, 0.0, 3.5), (20.5, low - 1.0, 2.5), 1, p.wood);
        }
        let sag = (15.5 - low).round() as i32;
        k.hook(Hook::Line { from: IVec3::new(3, 16, 2), to: IVec3::new(37, 16, 2), sag });
        return;
    }
    k.b((37, 0, 1), (39, rng.int(3, 6), 3), wood);
    k.b((26, 0, 1), (36, 1, 3), wood);
    k.rod((3.0, 15.5, 2.5), (10.0, 0.5, 0.5), 1, mat::ROPE_ROTTEN);
    k.b((10, 0, 0), (24, 1, 1), mat::ROPE_ROTTEN);
}

/// A well head of the local stone: a ring of voxels two high round the shaft, a coping
/// over them, two posts carrying a windlass with its rope and bucket, and a little gable
/// roof of whatever the place roofs with. Decayed, the roof and windlass are gone, the
/// coping fallen and a post broken off.
pub(super) fn stone_well(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    let corners = [(1, 1), (4, 1), (1, 4), (4, 4)];
    for y in 0..2 {
        for z in 1..5 {
            for x in 1..5 {
                let hole = (2..4).contains(&x) && (2..4).contains(&z);
                if hole || corners.contains(&(x, z)) {
                    continue;
                }
                // Decayed, stones are gone from the top course on the front and back.
                if decayed && y == 1 && (z == 1 || z == 4) && rng.unit() < 0.5 {
                    continue;
                }
                k.vox(x, y, z, p.block(rng));
            }
        }
    }
    // The shaft, dark going down.
    k.b((8, 0, 8), (16, 1, 16), mat::NEST_HOLLOW);
    k.hook(Hook::Water(IVec3::new(12, 0, 12)));
    let wood = if decayed { mat::WOOD_DEAD } else { p.wood };
    if decayed {
        k.b((4, 8, 11), (6, rng.int(11, 15), 13), wood);
        k.b((2, 0, 21), (22, 2, 23), wood);
        k.b((20, 0, 0), (23, 1, 3), mat::PLANK_DARK);
        k.b((1, 0, 20), (4, 1, 22), p.stone_dark);
        let weed = weed_colour(p);
        for (x, z) in [(3, 6), (21, 15), (12, 23)] {
            k.weed((x, 0, z), (x + 1, rng.int(3, 5), z + 1), weed);
        }
        return;
    }
    let cope = p.stone_dark;
    k.b((7, 8, 3), (17, 9, 8), cope);
    k.b((7, 8, 16), (17, 9, 21), cope);
    k.b((3, 8, 7), (8, 9, 17), cope);
    k.b((16, 8, 7), (21, 9, 17), cope);
    for x in [3, 19] {
        k.b((x, 9, 11), (x + 2, 24, 13), wood);
    }
    // The windlass: its drum of rope, the crank outside the left post, the bucket hung
    // over the shaft.
    k.b((5, 16, 11), (19, 18, 13), wood);
    k.b((1, 16, 11), (3, 18, 13), wood);
    k.b((0, 12, 11), (1, 18, 12), mat::IRON);
    k.b((0, 12, 12), (1, 13, 14), wood);
    k.b((9, 15, 10), (15, 19, 14), mat::ROPE);
    k.b((12, 12, 12), (13, 15, 13), mat::ROPE);
    k.b((10, 9, 10), (14, 12, 14), p.plank);
    // The roof, its ridge along X on the posts' tops.
    k.b((1, 24, 11), (23, 26, 13), wood);
    let roof = p.roof;
    k.b((0, 26, 10), (24, 27, 14), roof);
    for (i, (z0, z1)) in [(7, 11), (4, 8), (1, 5)].into_iter().enumerate() {
        let y = 25 - i as i32;
        k.b((0, y, z0), (24, y + 1, z1), roof);
        k.b((0, y, 24 - z1), (24, y + 1, 24 - z0), roof);
    }
}

/// A shaduf for lifting water in dry country: a long beam pivoting on a bar between two
/// posts, a stone weight lashed under its short end and a leather bucket hung from its
/// long end over a low ring of stones round the well. Decayed, the beam lies on the
/// ground with its weight, and sand has filled the well.
pub(super) fn shaduf(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    let wood = if decayed { mat::WOOD_DEAD } else { p.wood };
    k.round_xz((34.5, 6.5), 5.5, 3.5, (0, 4), p.stone);
    if !decayed {
        for z in [2, 8] {
            k.b((12, 0, z), (14, 22, z + 2), wood);
        }
        k.b((12, 22, 2), (14, 24, 10), wood);
        k.rod((1.5, 20.0, 6.0), (34.5, 32.0, 6.0), 2, wood);
        k.b((0, 14, 4), (5, 19, 8), p.stone_dark);
        k.b((1, 17, 3), (6, 18, 9), mat::ROPE);
        k.b((34, 10, 6), (35, 31, 7), wood);
        k.b((32, 6, 4), (37, 10, 9), mat::LEATHER);
        k.round_xz((34.5, 6.5), 3.5, 0.0, (0, 1), mat::NEST_HOLLOW);
        k.hook(Hook::Water(IVec3::new(34, 0, 6)));
        return;
    }
    k.b((12, 0, 2), (14, 22, 4), wood);
    k.b((12, 0, 8), (14, rng.int(8, 14), 10), wood);
    k.b((2, 0, 5), (28, 2, 7), wood);
    k.b((0, 0, 8), (4, 4, 12), p.stone_dark);
    k.round_xz((34.5, 6.5), 3.5, 0.0, (0, 2), mat::DESERT_SAND);
    k.hook(Hook::Water(IVec3::new(34, 2, 6)));
}

/// A low ring of stones about the water of an oasis, reeds at its edge and a clay jar
/// left by it. Decayed, the ring broken and sand drifted in, a puddle left in the middle.
pub(super) fn oasis_well(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    let rows = round_rows(10.0, 10.0, 8.0, 6.0);
    for (i, &(z0, z1, x0, x1)) in rows.iter().enumerate() {
        if decayed && i % 3 == 1 {
            continue;
        }
        k.b((x0, 0, z0), (x1, 3, z1), p.stone);
    }
    if !decayed {
        for (x, z) in [(10, 2), (3, 9), (17, 11), (9, 17)] {
            k.b((x, 3, z), (x + 2, 4, z + 1), p.stone_dark);
        }
        k.round_xz((10.0, 10.0), 6.0, 0.0, (0, 2), mat::WATER_STILL);
        k.hook(Hook::Water(IVec3::new(10, 2, 10)));
        for (x, z) in [(2, 3), (18, 6), (3, 17), (16, 18)] {
            k.weed((x, 0, z), (x + 1, rng.int(5, 8), z + 1), mat::REED);
        }
        k.b((16, 0, 1), (19, 4, 4), mat::CLAY);
        k.b((17, 4, 2), (18, 5, 3), mat::CLAY);
        return;
    }
    k.round_xz((10.0, 10.0), 6.0, 2.0, (0, 1), mat::DESERT_SAND);
    k.round_xz((10.0, 10.0), 2.0, 0.0, (0, 1), mat::WATER_STILL);
    k.hook(Hook::Water(IVec3::new(10, 1, 10)));
    for (x, z) in [(1, 2), (17, 1), (18, 17)] {
        k.b((x, 0, z), (x + 2, 2, z + 2), p.stone);
    }
}

/// A market stall: four posts, the back pair taller, with rails round their tops for an
/// awning (its [`Hook::Awning`]); a counter across the front on trestles with goods laid
/// out on it, and a bench behind with sacks. Decayed, the counter has come down, two
/// posts have gone, and broken pots lie on the boards.
pub(super) fn market_stall(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    let (wood, board) = if decayed { (mat::WOOD_DEAD, mat::PLANK_DARK) } else { (p.wood, p.plank) };
    k.b((1, 0, 1), (2, 22, 2), wood);
    k.b((2, 5, 2), (22, 6, 5), board);
    for x in [2, 21] {
        k.b((x, 0, 2), (x + 1, 5, 3), wood);
    }
    if !decayed {
        k.b((22, 0, 1), (23, 22, 2), wood);
        k.b((1, 0, 10), (2, 18, 11), wood);
        k.b((22, 0, 10), (23, 18, 11), wood);
        k.b((2, 21, 1), (22, 22, 2), wood);
        k.b((2, 17, 10), (22, 18, 11), wood);
        k.b((1, 17, 2), (2, 18, 10), wood);
        k.b((22, 17, 2), (23, 18, 10), wood);
        k.hook(Hook::Awning { back: IVec3::new(1, 22, 1), front: IVec3::new(23, 18, 11) });
        k.b((2, 9, 7), (22, 10, 12), board);
        k.b((2, 1, 11), (22, 9, 12), board);
        for x in [3, 20] {
            k.b((x, 0, 7), (x + 1, 9, 8), wood);
        }
        // Goods along the counter, a different spread on each stall.
        let mut x = 3;
        while x < 20 {
            let w = match rng.int(0, 4) {
                0 => {
                    k.b((x, 10, 8), (x + 4, 12, 11), mat::WICKER);
                    k.b((x + 1, 12, 9), (x + 3, 13, 10), if rng.unit() < 0.5 { mat::CLOTH_RED } else { mat::CLOTH_GREEN });
                    4
                }
                1 => {
                    k.b((x, 10, 8), (x + 2, 13, 10), mat::CLAY);
                    k.b((x + 2, 10, 9), (x + 4, 12, 11), mat::CLAY);
                    4
                }
                2 => {
                    k.b((x, 10, 8), (x + 4, 12, 10), p.accent);
                    k.b((x, 10, 10), (x + 4, 11, 11), mat::CLOTH_PALE);
                    4
                }
                3 => {
                    k.b((x, 10, 8), (x + 3, 11, 10), mat::PLANK_PALE);
                    k.b((x, 10, 10), (x + 2, 11, 11), mat::PLANK_PALE);
                    3
                }
                _ => 2,
            };
            x += w + 1;
        }
        k.b((3, 6, 2), (7, 9, 5), mat::SACKCLOTH);
        k.b((8, 6, 2), (11, 8, 4), mat::SACKCLOTH);
        return;
    }
    k.b((22, 0, 1), (23, rng.int(6, 12), 2), wood);
    k.b((1, 0, 10), (2, 18, 11), wood);
    k.b((2, 0, 7), (22, 1, 10), board);
    k.b((3, 0, 5), (21, 1, 6), wood);
    k.b((6, 1, 8), (8, 2, 9), mat::CLAY);
    k.b((13, 1, 7), (14, 2, 9), mat::CLAY);
}

/// A run of paling fence along X at row `z`, from `x0` to `x1`: posts every few cells, two
/// rails on the side toward `inward` (+1 or -1 in Z), and palings between. Decayed, most
/// palings and a rail are gone.
fn fence_x(k: &mut Kit, rng: &mut Rng, p: &Palette, (x0, x1): (i32, i32), z: i32, inward: i32, decayed: bool) {
    let (wood, board) = if decayed { (mat::WOOD_DEAD, mat::PLANK_DARK) } else { (p.wood, p.plank) };
    let rz = z + inward;
    for x in (x0..x1).step_by(6).chain(std::iter::once(x1 - 1)) {
        k.b((x, 0, z), (x + 1, 9, z + 1), wood);
    }
    for (n, y) in [3, 7].into_iter().enumerate() {
        if !(decayed && n == 1) {
            k.b((x0, y, rz), (x1, y + 1, rz + 1), wood);
        }
    }
    for x in (x0 + 1..x1 - 1).filter(|x| (x - x0) % 6 != 0 && (x - x0) % 2 == 1) {
        if !decayed || rng.unit() < 0.35 {
            k.b((x, 0, z), (x + 1, if decayed { rng.int(3, 7) } else { 8 }, z + 1), board);
        }
    }
}

/// As [`fence_x`], along Z at column `x`.
fn fence_z(k: &mut Kit, rng: &mut Rng, p: &Palette, x: i32, (z0, z1): (i32, i32), inward: i32, decayed: bool) {
    let (wood, board) = if decayed { (mat::WOOD_DEAD, mat::PLANK_DARK) } else { (p.wood, p.plank) };
    let rx = x + inward;
    for z in (z0..z1).step_by(6).chain(std::iter::once(z1 - 1)) {
        k.b((x, 0, z), (x + 1, 9, z + 1), wood);
    }
    for (n, y) in [3, 7].into_iter().enumerate() {
        if !(decayed && n == 1) {
            k.b((rx, y, z0), (rx + 1, y + 1, z1), wood);
        }
    }
    for z in (z0 + 1..z1 - 1).filter(|z| (z - z0) % 6 != 0 && (z - z0) % 2 == 1) {
        if !decayed || rng.unit() < 0.35 {
            k.b((x, 0, z), (x + 1, if decayed { rng.int(3, 7) } else { 8 }, z + 1), board);
        }
    }
}

/// A hen house on four legs with a gable roof, a pop hole on the side toward its run and a
/// ramp down to it, and the run fenced with palings, a water dish and a feeding trough in
/// it. Empty: there are no hens yet. The run's floor is its [`Hook::Pen`]. Decayed, the
/// roof has holes, the ramp has fallen, and the fence is down to a few palings.
pub(super) fn chicken_coop(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    let (wood, board) = if decayed { (mat::WOOD_DEAD, mat::PLANK_DARK) } else { (p.wood, p.plank) };
    for (x, z) in [(1, 1), (12, 1), (1, 10), (12, 10)] {
        let h = if decayed && (x, z) == (12, 10) { 2 } else { 4 };
        k.b((x, 0, z), (x + 1, h, z + 1), wood);
    }
    k.b((1, 4, 1), (13, 5, 11), board);
    k.b((1, 5, 1), (13, 12, 11), board);
    k.b((13, 5, 4), (14, 8, 7), mat::NEST_HOLLOW);
    // The gable, filled under the roof.
    k.b((1, 12, 3), (13, 13, 9), board);
    k.b((1, 13, 5), (13, 14, 7), board);
    let roof = p.roof;
    for (i, (z0, z1)) in [(0, 3), (2, 5), (4, 8)].into_iter().enumerate() {
        let y = 12 + i as i32;
        if decayed && i == 2 {
            continue;
        }
        k.b((0, y, z0), (14, y + 1, z1), roof);
        if !(decayed && i == 1) {
            k.b((0, y, 12 - z1), (14, y + 1, 12 - z0), roof);
        }
    }
    if decayed {
        k.b((15, 0, 4), (20, 1, 7), board);
    } else {
        for i in 0..5 {
            let lo = if i == 0 { 13 } else { 14 + i };
            k.b((lo, 4 - i, 4), (16 + i, 5 - i, 7), board);
        }
        k.b((20, 0, 15), (23, 1, 18), mat::CLAY);
        k.b((16, 0, 8), (24, 2, 10), board);
        k.hook(Hook::Pen(IVec3::new(14, 0, 2), IVec3::new(30, 0, 22)));
    }
    // The run's fence: along the back from the coop to the far corner, down the far side,
    // across the front, and up the near side to the coop.
    fence_x(k, rng, p, (13, 31), 0, 1, decayed);
    fence_z(k, rng, p, 30, (1, 23), -1, decayed);
    fence_x(k, rng, p, (1, 31), 23, -1, decayed);
    fence_z(k, rng, p, 1, (11, 23), 1, decayed);
}

/// A straw skep on a flat stone set on a post: coils of straw in a dome, light and dark,
/// with its door at the foot. Decayed, the skep has slumped to its lowest coils and the
/// old comb shows black through the top.
pub(super) fn skep(k: &mut Kit, _rng: &mut Rng, p: &Palette, decayed: bool) {
    k.b((3, 0, 3), (5, 5, 5), if decayed { mat::WOOD_DEAD } else { p.wood });
    k.b((0, 5, 0), (8, 6, 8), p.stone);
    let coils: &[(f32, Voxel)] = if decayed {
        &[(3.0, mat::HAY_ROTTEN), (2.8, mat::HAY_ROTTEN)]
    } else {
        &[(3.0, mat::THATCH), (3.0, mat::HAY), (2.8, mat::THATCH), (2.5, mat::HAY), (2.0, mat::THATCH), (1.3, mat::HAY)]
    };
    for (i, &(r, v)) in coils.iter().enumerate() {
        let y = 6 + i as i32;
        k.round_xz((4.0, 4.0), r, 0.0, (y, y + 1), v);
    }
    if decayed {
        k.b((3, 8, 3), (5, 9, 5), mat::WASP_PAPER_DARK);
        return;
    }
    k.b((3, 12, 3), (5, 13, 5), mat::THATCH);
    k.b((3, 6, 7), (5, 7, 8), mat::NEST_HOLLOW);
    k.hook(Hook::Hive(IVec3::new(4, 6, 8)));
}

/// A hollow log stood on end for bees, as in the northern forests, a board and a stone on
/// its top and a ledge under its door. Decayed, split and grey, its cover fallen, with a
/// bracket fungus on it where the air is damp.
pub(super) fn log_hive(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    let bark = if decayed { mat::WOOD_DEAD } else { p.wood };
    k.vox(1, 0, 1, bark);
    k.vox(1, 1, 1, bark);
    k.b((3, 0, 4), (9, 9, 8), bark);
    k.b((4, 0, 3), (8, 9, 9), bark);
    if !decayed {
        k.b((2, 9, 2), (10, 10, 10), p.plank);
        k.b((5, 10, 5), (7, 11, 7), p.stone);
        k.b((5, 3, 9), (7, 5, 10), mat::NEST_HOLLOW);
        k.b((5, 2, 9), (7, 3, 11), p.wood);
        k.hook(Hook::Hive(IVec3::new(6, 4, 10)));
        return;
    }
    k.b((6, 1, 9), (7, 8, 10), mat::NEST_HOLLOW);
    k.b((0, 0, 9), (6, 1, 12), mat::PLANK_DARK);
    k.b((9, 0, 9), (11, 1, 11), p.stone);
    if damp(p) || rng.unit() < 0.3 {
        k.b((9, 5, 5), (10, 6, 7), mat::MUSHROOM_TAN);
    }
}

/// A wooden box hive on four legs, its front painted, with a landing board under the slot
/// of its door and a pitched cover. Decayed, the cover lies on the ground, the paint has
/// faded and old comb shows through the open top.
pub(super) fn box_hive(k: &mut Kit, _rng: &mut Rng, p: &Palette, decayed: bool) {
    let (wood, board) = if decayed { (mat::WOOD_DEAD, mat::PLANK_DARK) } else { (p.wood, p.plank) };
    for (x, z) in [(1, 1), (6, 1), (1, 6), (6, 6)] {
        k.b((x, 0, z), (x + 1, 4, z + 1), wood);
    }
    k.b((0, 4, 0), (8, 5, 8), board);
    k.b((1, 5, 1), (7, 12, 7), board);
    k.b((1, 5, 7), (7, 12, 8), if decayed { mat::SACKCLOTH } else { p.accent });
    k.b((2, 5, 8), (6, 6, 10), board);
    k.b((2, 6, 8), (6, 7, 9), mat::NEST_HOLLOW);
    if !decayed {
        k.b((0, 12, 0), (8, 13, 9), p.roof);
        k.b((0, 13, 3), (8, 14, 6), p.roof);
        k.hook(Hook::Hive(IVec3::new(4, 6, 9)));
        return;
    }
    k.b((2, 12, 2), (6, 13, 6), mat::WASP_PAPER_DARK);
    k.b((0, 0, 10), (8, 1, 12), p.roof);
}

/// A hay rick: a block of hay voxels with a thatched roof along its length, ropes over the
/// thatch and stones hung from them at the eaves against the wind. Decayed, slumped to
/// half its height, its thatch gone and rotting, grass growing on it.
pub(super) fn hay_rick(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    if decayed {
        for x in 1..7 {
            for z in 1..6 {
                k.vox(x, 0, z, mat::HAY_ROTTEN);
                if (2..6).contains(&x) && (2..5).contains(&z) {
                    k.vox(x, 1, z, mat::HAY_ROTTEN);
                }
            }
        }
        k.b((9, 8, 9), (23, 10, 19), mat::HAY_ROTTEN);
        k.b((9, 10, 13), (23, 11, 14), mat::ROPE_ROTTEN);
        let weed = weed_colour(p);
        for (x, z) in [(10, 10), (16, 15), (21, 11)] {
            k.weed((x, 10, z), (x + 1, rng.int(12, 14), z + 1), weed);
        }
        return;
    }
    for y in 0..4 {
        for z in 1..6 {
            for x in 1..7 {
                k.vox(x, y, z, mat::HAY);
            }
        }
    }
    // Wisps standing out of the sides, below the stones.
    for _ in 0..8 {
        let (x, y) = (rng.int(5, 26), rng.int(1, 8));
        let z = if rng.unit() < 0.5 { 3 } else { 24 };
        k.b((x, y, z), (x + rng.int(1, 3), y + 1, z + 1), mat::HAY);
    }
    for i in 0..6 {
        let y = 16 + 2 * i;
        k.b((2, y, 3 + 2 * i), (30, y + 2, 6 + 2 * i), mat::THATCH);
        k.b((2, y, 22 - 2 * i), (30, y + 2, 25 - 2 * i), mat::THATCH);
        if i < 4 {
            k.b((4, y, 6 + 2 * i), (28, y + 2, 22 - 2 * i), mat::HAY);
        }
    }
    for x in [8, 16, 23] {
        for i in 0..4 {
            let y = 18 + 2 * i;
            k.b((x, y, 3 + 2 * i), (x + 1, y + 1, 5 + 2 * i), mat::ROPE);
            k.b((x, y, 23 - 2 * i), (x + 1, y + 1, 25 - 2 * i), mat::ROPE);
        }
        // Over the ridge.
        k.b((x, 26, 11), (x + 1, 27, 12), mat::ROPE);
        k.b((x, 28, 12), (x + 1, 29, 16), mat::ROPE);
        k.b((x, 26, 16), (x + 1, 27, 17), mat::ROPE);
        k.b((x, 12, 3), (x + 1, 16, 4), mat::ROPE);
        k.b((x - 1, 9, 1), (x + 2, 12, 4), p.stone);
        k.b((x, 12, 24), (x + 1, 16, 25), mat::ROPE);
        k.b((x - 1, 9, 24), (x + 2, 12, 27), p.stone);
    }
}

/// Hay built up round a tall pole, as in the north and the mountains, wide at the foot and
/// drawn in to a cap of hay about the pole. Decayed, slumped to a low heap of rotten hay
/// with the grey pole standing out of it.
pub(super) fn pole_rick(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    let hay = if decayed { mat::HAY_ROTTEN } else { mat::HAY };
    let plus = [(2, 1), (1, 2), (2, 2), (3, 2), (2, 3)];
    let wide: Vec<(i32, i32)> = (1..4).flat_map(|x| (1..4).map(move |z| (x, z))).chain([(0, 2), (4, 2), (2, 0), (2, 4)]).collect();
    let square: Vec<(i32, i32)> = (1..4).flat_map(|x| (1..4).map(move |z| (x, z))).collect();
    let layers: Vec<&[(i32, i32)]> = if decayed { vec![&wide, &plus] } else { vec![&wide, &wide, &square, &plus, &[(2, 2)]] };
    for (y, cols) in layers.iter().enumerate() {
        for &(x, z) in cols.iter() {
            k.vox(x, y as i32, z, hay);
        }
    }
    let top = layers.len() as i32 * 4;
    let pole = if decayed { mat::WOOD_DEAD } else { p.wood };
    k.b((9, 1, 9), (11, if decayed { 24 } else { 31 }, 11), pole);
    if !decayed {
        // The cap is drawn in round the pole, which goes up through the middle of it.
        k.round_xz((10.0, 10.0), 3.0, 1.0, (top, top + 2), hay);
        k.round_xz((10.0, 10.0), 2.0, 1.0, (top + 2, top + 4), hay);
        return;
    }
    let weed = weed_colour(p);
    for (x, z) in [(6, 9), (13, 10)] {
        k.weed((x, top, z), (x + 1, top + rng.int(2, 4), z + 1), weed);
    }
}

/// Split logs stacked against a wall between two stakes, their ends showing on the face,
/// under a capping of boards. Decayed, sunk to two courses of grey wood, the capping
/// fallen before it, fungus on the ends where it is damp.
pub(super) fn log_stack(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    let courses = if decayed { 2 } else { 3 };
    let core = if decayed { mat::WOOD_DEAD } else { p.wood };
    for y in 0..courses {
        for x in 0..5 {
            k.vox(x, y, 0, core);
        }
    }
    let ends = if decayed { [mat::WOOD_DEAD, mat::PLANK_DARK] } else { [mat::PLANK_PALE, p.plank] };
    let skip = if decayed { 0.45 } else { 0.15 };
    for i in 0..10 {
        for j in 0..courses * 2 {
            if rng.unit() >= skip {
                k.b((2 * i, 2 * j, 4), (2 * i + 2, 2 * j + 2, 5), ends[((i + j) % 2) as usize]);
            }
        }
    }
    let top = courses * 4;
    if !decayed {
        for x in [0, 19] {
            k.b((x, 0, 5), (x + 1, top + 2, 6), p.wood);
        }
        k.b((0, top, 0), (20, top + 1, 5), p.plank);
        k.b((0, top + 1, 0), (20, top + 2, 3), p.plank);
        return;
    }
    k.b((0, 0, 5), (1, top + 1, 6), mat::WOOD_DEAD);
    k.b((2, 0, 6), (18, 1, 8), mat::PLANK_DARK);
    if damp(p) {
        k.b((7, 8, 1), (9, 9, 3), mat::MUSHROOM_TAN);
    }
}

/// A round stack of split logs as the north builds them: courses of log ends all round,
/// light and dark, about a core, under a cone of bark. Decayed, sunk to its lower courses,
/// grey, with logs rolled off into the grass.
pub(super) fn round_stack(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    let courses = if decayed { 2 } else { 4 };
    let core = if decayed { mat::WOOD_DEAD } else { p.wood };
    for y in 0..courses {
        for (x, z) in [(2, 1), (1, 2), (2, 2), (3, 2), (2, 3)] {
            k.vox(x, y, z, core);
        }
    }
    let ends = if decayed { [mat::WOOD_DEAD, mat::PLANK_DARK] } else { [mat::PLANK_PALE, p.plank] };
    let rows = round_rows(10.0, 10.0, 9.0, 6.5);
    for band in 0..courses * 2 {
        for (i, &(z0, z1, x0, x1)) in rows.iter().enumerate() {
            let y = band * 2;
            k.b((x0, y, z0), (x1, y + 2, z1), ends[(i + band as usize) % 2]);
        }
    }
    let top = courses * 4;
    if !decayed {
        for (n, r) in [10.0, 7.0, 4.0, 1.5].into_iter().enumerate() {
            let y = top + n as i32;
            k.round_xz((10.0, 10.0), r, 0.0, (y, y + 1), if n % 2 == 0 { p.wood } else { mat::PLANK_DARK });
        }
        return;
    }
    // Rolled off into the corners, clear of the ring.
    for (x, z) in [(0, 0), (17, 1), (0, 17)] {
        k.b((x, 0, z), (x + 3, 2, z + 2), ends[rng.int(0, 1) as usize]);
    }
}

/// A wattle hurdle: rods of willow woven in and out between upright sails, staked where it
/// stands. Decayed, the sails broken at odd heights and the weave rotted to a few rods.
pub(super) fn hurdle(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    let sails = [1, 4, 7, 10, 13, 16, 18];
    let wood = if decayed { mat::WOOD_DEAD } else { p.wood };
    let heights: Vec<i32> = sails.iter().map(|_| if decayed { rng.int(4, 12) } else { 12 }).collect();
    for (i, &x) in sails.iter().enumerate() {
        k.b((x, 0, 1), (x + 1, heights[i], 3), wood);
    }
    let rod = if decayed { mat::ROPE_ROTTEN } else { mat::WITHY };
    for j in 0..10 {
        let y = 1 + j;
        for i in 0..sails.len() - 1 {
            if decayed && (rng.unit() < 0.55 || y >= heights[i].max(heights[i + 1])) {
                continue;
            }
            let z = if (i as i32 + j) % 2 == 0 { 0 } else { 3 };
            k.b((sails[i], y, z), (sails[i + 1] + 1, y + 1, z + 1), rod);
        }
    }
}
