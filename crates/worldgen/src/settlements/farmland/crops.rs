//! What grows in the fields (#64): the crops as small plants, and the ground under them.
//!
//! A crop is built once, as it stands when it is ripe, and the seasons do the rest: each
//! of its boxes keeps a row of the table of seasons (see `plants::seasons`), so the
//! renderer shows it green while it grows, golden at harvest, brown for a few days, and
//! then not at all once it is cut; the stubble left behind keeps a row of its own, there
//! from harvest until the field is ploughed. So a field of wheat is green in summer,
//! gold in August, stubble in the autumn and bare plough in winter, without the world
//! being built again. Which crop a field grows does not change from year to year: the
//! world is drawn on a date but made without one.
//!
//! Plants are kept few: a grain column is three boxes, a root row two, a paddy's rice
//! two, and ploughed ground, pasture and fallow none beyond what the ground grows anyway.
//!
//! Rice keeps the grapes' season: the paddies lie open and shining in spring and early
//! summer, the rice green over them from July, gold by the end of August and cut through
//! September, and the water open again from then through the winter.

use voxel::{
    Materials, Voxel,
    mesh::{CELLS, PlantBox},
};

use super::fields::{Crop, Plot};
use crate::{
    biome::Biome,
    features::plants::{Season, Wash, washes},
    materials as mat,
    noise::{hash3, unit},
    surface::Column,
};

// Rows of the table of seasons the crops keep, after the flowers' blooms and leaves
// (1 to 54; see `plants::seasons`).
/// Wheat and barley: green from April, ripe at the end of July, cut by the middle of
/// September.
pub const GRAIN: u8 = 55;
/// Rye and oats, and flax's stems: a little later.
pub const LATE_GRAIN: u8 = 56;
/// Stubble: from harvest until the field is ploughed in late autumn.
pub const STUBBLE: u8 = 57;
/// The leaves of turnips and beet: from May until they are lifted in October.
pub const ROOTS: u8 = 58;
/// Flax's blue flowers, at midsummer.
pub const FLAX: u8 = 59;
/// The long grass of a hay meadow: in seed in June, mown in July.
pub const HAY: u8 = 60;
/// Grapes, ripe in September.
pub const GRAPES: u8 = 61;

/// The seasons the crops and orchards keep, each with the row of the table it goes in.
pub fn seasons() -> [(u8, Season); 9] {
    let s = |start: f32, length: f32, bud: f32, seed: f32| Season { start, length, bud, seed };
    [
        (GRAIN, s(0.33, 0.07, 0.29, 0.02)),
        (LATE_GRAIN, s(0.35, 0.06, 0.30, 0.02)),
        (STUBBLE, s(0.42, 0.17, 0.0, 0.03)),
        (ROOTS, s(0.14, 0.42, 0.08, 0.02)),
        (FLAX, s(0.22, 0.06, 0.12, 0.03)),
        (HAY, s(0.21, 0.10, 0.17, 0.01)),
        (GRAPES, s(0.42, 0.10, 0.12, 0.02)),
        (super::orchard::BLOSSOM, s(0.10, 0.05, 0.04, 0.01)),
        (super::orchard::FRUIT, s(0.45, 0.09, 0.14, 0.01)),
    ]
}

/// Gives the crops' materials the years they keep, where a box with no season of its own
/// shows them: the vines' leaves turn and fall with the broadleaves, an orchard's and a
/// hedge's too; the long grass of a meadow dries with the grass; the stems of a hedge and
/// the dead thorn piled between dry fields keep their colour.
pub fn classify(m: &mut Materials) {
    use voxel::plant as pk;
    for v in [mat::VINE_ROW, mat::ORCHARD_LEAF, mat::HEDGE] {
        m.set_plant(v, pk::DECIDUOUS);
    }
    m.set_plant(mat::OLIVE_LEAF, pk::EVERGREEN);
    for v in [mat::MEADOW_TUFT, mat::WHEAT, mat::BARLEY, mat::RYE, mat::STUBBLE, mat::FLAX, mat::ROOT_LEAF, mat::RICE] {
        m.set_plant(v, pk::GRASS);
    }
    for v in [mat::HEDGE_WOOD, mat::THORN, mat::BARK_OLIVE] {
        m.set_plant(v, pk::VINE);
    }
    for v in [mat::FLAX_BLOOM, mat::GRAPE, mat::BLOSSOM, mat::APPLE, mat::FRUIT_GOLD, mat::WHEAT_EAR, mat::ROOT_CROWN] {
        m.set_plant(v, pk::FLOWER);
    }
}

/// What a far field of `crop` looks like, where its plants are too far off to draw: the
/// colours its ground takes (see [`Wash`]). None for one whose ground shows as it is, or
/// whose plants are the ordinary ones of the ground.
fn wash_of(crop: Crop) -> Option<Wash> {
    let green = (mat::TUFT_LUSH, mat::TUFT, 0.3);
    let w = |body: (Voxel, Voxel, f32), bloom: (Voxel, f32), season: u8, cover: f32| Some(Wash { body, bloom, season, cover });
    match crop {
        Crop::Wheat => w(green, (mat::WHEAT, 0.9), GRAIN, 0.8),
        Crop::Barley => w(green, (mat::BARLEY, 0.9), GRAIN, 0.8),
        Crop::Rye => w(green, (mat::RYE, 0.9), LATE_GRAIN, 0.8),
        Crop::Flax => w(green, (mat::FLAX_BLOOM, 0.6), FLAX, 0.7),
        // Dark rows of leaves on the plough, and the plough alone once they are lifted.
        Crop::Roots => w((mat::PLOUGH, mat::PLOUGH, 0.0), (mat::ROOT_LEAF, 0.85), ROOTS, 0.7),
        Crop::Vines => w((mat::VINE_ROW, mat::PLOUGH_DRY, 0.45), (mat::GRAPE, 0.25), GRAPES, 0.7),
        Crop::Hay => w((mat::MEADOW_TUFT, mat::TUFT_LUSH, 0.4), (mat::SEED_HEAD, 0.45), HAY, 0.65),
        // The trees' heads, close set in their rows, white in blossom in spring.
        Crop::Orchard => w((mat::ORCHARD_LEAF, mat::TUFT_LUSH, 0.35), (mat::BLOSSOM, 0.35), super::orchard::BLOSSOM, 0.6),
        Crop::Olives => w((mat::OLIVE_LEAF, mat::PLOUGH_DRY, 0.45), (voxel::AIR, 0.0), 0, 0.6),
        // Open water with the sky in it, and the rice over it in its season.
        Crop::Paddy => w((mat::PADDY_FAR, mat::PADDY_FAR, 0.0), (mat::RICE, 0.85), GRAPES, 0.75),
        _ => None,
    }
}

/// The washes of the crops, for `plants::washes` to hold with the rest.
pub fn washes_of_crops() -> Vec<Wash> {
    Crop::ALL.iter().filter_map(|&c| wash_of(c)).collect()
}

/// The wash code of a field of `crop` (see `plants::wash_at`), or None for one whose
/// ground keeps the wash of whatever grows on it.
pub fn wash(crop: Crop) -> Option<u8> {
    static CODES: std::sync::OnceLock<Vec<(Crop, u8)>> = std::sync::OnceLock::new();
    let codes = CODES.get_or_init(|| {
        Crop::ALL
            .iter()
            .filter_map(|&c| {
                let w = wash_of(c)?;
                let i = washes().iter().position(|v| *v == w).expect("every crop's wash is among the washes");
                Some((c, i as u8 + 1))
            })
            .collect()
    });
    codes.iter().find(|w| w.0 == crop).map(|w| w.1)
}

/// The top voxel of a column of a field of `crop`, if the field changes it: tilled
/// ground is plough the year round, which the crop stands on in its season, and a
/// paddy's floor is mud under its water (its bunds are grass).
pub fn ground(crop: Crop, col: &Column) -> Option<Voxel> {
    if crop == Crop::Paddy {
        return super::paddy::flooded(col).then_some(mat::MUD);
    }
    soil(crop, col.biome, col.temperature)
}

/// [`ground`] in country of `biome` as warm as `temperature` (0 polar, 1 tropical): in
/// dry or hot country the plough is pale and dusty.
pub fn soil(crop: Crop, biome: Biome, temperature: f32) -> Option<Voxel> {
    let dry = biome.arid() || matches!(biome, Biome::Savanna | Biome::Steppe) || temperature > 0.66;
    match crop {
        Crop::Vines => Some(mat::PLOUGH_DRY),
        // Seen from far off, a paddy is its water.
        Crop::Paddy => Some(mat::PADDY_FAR),
        c if c.tilled() => Some(if dry { mat::PLOUGH_DRY } else { mat::PLOUGH }),
        _ => None,
    }
}

/// Builds the crop of field `plot` on the column at world `(x, z)`, whose ground is
/// `col`. Returns false where the field grows nothing of its own on it, leaving the
/// column to grow what its ground does (pasture's grass, fallow's flowers).
pub fn grow(plot: &Plot, col: &Column, x: i32, z: i32, seed: u32, out: &mut Vec<PlantBox>) -> bool {
    let h = hash3(seed ^ 0xc409, x, 0, z);
    let y = col.top() + 1;
    let origin = [x * CELLS, y * CELLS, z * CELLS];
    let mut add = |lo: [i32; 3], hi: [i32; 3], voxel: Voxel, sway: bool, season: u8| {
        out.push(PlantBox {
            lo: [origin[0] + lo[0], origin[1] + lo[1], origin[2] + lo[2]],
            hi: [origin[0] + hi[0], origin[1] + hi[1], origin[2] + hi[2]],
            voxel,
            sway_lo: false,
            sway_hi: sway,
            thin: 0,
            season,
            flap: 0,
        });
    };
    // Rows run the nearer way across the voxels to the way the field's rows run, along
    // the contour. A box spans the voxel along its row; `across` is where across it.
    let along_x = plot.rows.x.abs() >= plot.rows.y.abs();
    // A box `c0..c1` across the row, `a0..a1` along it and `lo..hi` up, in cells.
    let rect = |(c0, c1): (i32, i32), (a0, a1): (i32, i32), (lo, hi): (i32, i32)| -> ([i32; 3], [i32; 3]) {
        if along_x { ([a0, lo, c0], [a1, hi, c1]) } else { ([c0, lo, a0], [c1, hi, a1]) }
    };
    let row = |across: i32, a0: i32, a1: i32, lo: i32, hi: i32| rect((across, across + 1), (a0, a1), (lo, hi));
    // Which row of voxels across the field this column is in, and how far along it.
    let (lane, along) = if along_x { (z, x) } else { (x, z) };
    let bit = |k: u32, n: u32| ((h >> k) % n) as i32;
    match plot.crop {
        Crop::Wheat | Crop::Barley | Crop::Rye | Crop::Flax => {
            let (voxel, season, tall) = match plot.crop {
                Crop::Wheat => (mat::WHEAT, GRAIN, 9),
                Crop::Barley => (mat::BARLEY, GRAIN, 7),
                Crop::Rye => (mat::RYE, LATE_GRAIN, 11),
                _ => (mat::FLAX, LATE_GRAIN, 8),
            };
            // One drill to a voxel, a hand's width of plants standing a little taller
            // or shorter along it, so a field is rows and not boards.
            let across = 1 + bit(20, 2);
            for k in 0..2 {
                let height = tall - 1 + bit(4 + 3 * k as u32, 3);
                let (lo, hi) = row(across, 2 * k, 2 * k + 2, 0, height);
                add(lo, hi, voxel, true, season);
                if plot.crop == Crop::Flax && bit(12 + k as u32, 2) == 0 {
                    let a = 2 * k + bit(16 + k as u32, 2);
                    let (lo, hi) = row(across, a, a + 1, height, height + 1);
                    add(lo, hi, mat::FLAX_BLOOM, true, FLAX);
                }
            }
            let (lo, hi) = row(across, 0, CELLS, 0, 2);
            add(lo, hi, mat::STUBBLE, false, STUBBLE);
            true
        }
        Crop::Roots => {
            // Rows of leafy tops a voxel apart, with bare plough between.
            if lane.rem_euclid(2) == 0 {
                let (lo, hi) = rect((1, 3), (0, CELLS), (0, 2 + bit(4, 2)));
                add(lo, hi, mat::ROOT_LEAF, true, ROOTS);
                let a = bit(8, 3);
                let (lo, hi) = rect((0, CELLS), (a, a + 2), (0, 3));
                add(lo, hi, mat::ROOT_LEAF, true, ROOTS);
            }
            true
        }
        Crop::Vines => {
            // A row of vines every third voxel, on posts every fourth, the grapes
            // hanging beside the leaves.
            if lane.rem_euclid(3) != 0 {
                return true;
            }
            if along.rem_euclid(4) == 0 {
                let (lo, hi) = rect((1, 2), (1, 2), (0, 9));
                add(lo, hi, mat::PLANK_DARK, false, 0);
            }
            let (lo, hi) = rect((1, 2), (2, 3), (0, 4));
            add(lo, hi, mat::TWIG, false, 0);
            let (lo, hi) = rect((0, 3), (0, CELLS), (4, 8));
            add(lo, hi, mat::VINE_ROW, true, 0);
            for k in 0..2u32 {
                if bit(10 + k, 3) != 0 {
                    let a = 1 + 2 * k as i32;
                    let (lo, hi) = rect((3, 4), (a, a + 1), (3, 5));
                    add(lo, hi, mat::GRAPE, true, GRAPES);
                }
            }
            true
        }
        Crop::Hay => {
            // Long grass gone to seed, three tufts to the voxel; the column now and then
            // carries a flower instead (see [`flowers`]).
            for k in 0..3u32 {
                let (cx, cz) = (bit(4 + 4 * k, 4), bit(6 + 4 * k, 4));
                let tall = 5 + bit(16 + 2 * k, 3);
                add([cx, 0, cz], [cx + 1, tall, cz + 1], mat::MEADOW_TUFT, true, HAY);
            }
            true
        }
        Crop::Paddy => {
            // Rice planted out in hills, two rows to the voxel, standing up out of the
            // water from the mud under it; nothing on the bunds but their grass.
            if !super::paddy::flooded(col) {
                return false;
            }
            for k in 0..2 {
                let a = (bit(4, 4) + 2 * k) % CELLS;
                let height = 8 + bit(8 + 2 * k as u32, 3);
                let (lo, hi) = rect((2 * k, 2 * k + 1), (a, (a + 2).min(CELLS)), (0, height));
                add(lo, hi, mat::RICE, true, GRAPES);
            }
            true
        }
        Crop::Ploughed => true,
        _ => false,
    }
}

/// The flower-carrying ground a column of a field of `crop` grows instead of its own
/// plants, if it grows a flower: hay meadows and fallow are full of them. `roll` is the
/// column's own, 0 to 1, and `patch` varies slowly across the field.
pub fn flowers(crop: Crop, roll: f32, patch: f32) -> Option<Voxel> {
    let share = match crop {
        Crop::Fallow => 0.22 + 0.25 * patch.clamp(0.0, 1.0),
        Crop::Hay => 0.07,
        Crop::Pasture => 0.02,
        _ => return None,
    };
    if roll >= share {
        return None;
    }
    let pick = roll / share;
    // Poppies most of all in the fallow, then ox-eye daisies and buttercups, and blue.
    Some(match crop {
        Crop::Fallow if pick < 0.45 => mat::FLOWER_RED,
        Crop::Fallow if pick < 0.7 => mat::FLOWER_WHITE,
        Crop::Fallow if pick < 0.88 => mat::FLOWER_YELLOW,
        Crop::Fallow => mat::FLOWER_BLUE,
        _ if pick < 0.4 => mat::FLOWER_YELLOW,
        _ if pick < 0.75 => mat::FLOWER_WHITE,
        _ if pick < 0.9 => mat::FLOWER_RED,
        _ => mat::FLOWER_PURPLE,
    })
}

/// How grazed the grass of a field of `crop` is, 0 to 7 (the wear of a sward code, see
/// `surface::sward`), if the field grazes it.
pub fn grazed(crop: Crop) -> Option<u8> {
    match crop {
        Crop::Pasture => Some(4),
        Crop::Rough | Crop::Orchard | Crop::Olives => Some(2),
        _ => None,
    }
}

/// A roll of the dice for a column's flowers, 0 to 1.
pub fn roll(seed: u32, x: i32, z: i32) -> f32 {
    unit(hash3(seed ^ 0xf10e, x, 1, z))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_crops_keep_their_own_rows_and_washes() {
        let flowers = crate::features::plants::Flower::ALL.len() as u8;
        for (row, _) in seasons() {
            assert!(row > 2 * flowers, "row {row} is a flower's");
            assert!((row as usize) < crate::features::plants::SEASON_SLOTS);
        }
        let table = crate::features::plants::seasons();
        for (row, s) in seasons() {
            assert_eq!(table[row as usize][0], s.start);
        }
        assert!(washes().len() < 128, "{} washes, more than the renderer holds (WASH_SLOTS in game/src/render.rs)", washes().len());
        for c in Crop::ALL {
            if let Some(code) = wash(c) {
                assert_eq!(Some(washes()[code as usize - 1]), wash_of(c));
            }
        }
        // Wheat is green in June, gold in August, gone by October, and its stubble
        // stands in the autumn and is gone by winter.
        let grain = seasons()[0].1;
        let stubble = seasons()[2].1;
        use crate::features::plants::Phase;
        assert_eq!(grain.phase(0.18, 3, 4), Phase::Bud);
        assert_eq!(grain.phase(0.38, 3, 4), Phase::Bloom);
        assert_eq!(grain.phase(0.53, 3, 4), Phase::Gone);
        assert_eq!(stubble.phase(0.53, 3, 4), Phase::Bloom);
        assert_eq!(stubble.phase(0.80, 3, 4), Phase::Gone);
        assert_eq!(grain.phase(0.80, 3, 4), Phase::Gone);
    }
}
