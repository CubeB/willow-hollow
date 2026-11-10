//! Small plants: grass tufts, flowers, reeds, mushrooms and lily pads, modelled from
//! blocks a quarter of a voxel across that stand on top of the voxel grid. Each is
//! decided by its own column alone and stays inside it, so neighbouring chunks agree
//! without looking at each other.

use voxel::{
    Voxel,
    mesh::{CELLS, PlantBox},
};

use super::shape::Rng;
use crate::{
    biome::Biome,
    materials as mat,
    noise::{hash3, perlin2, unit},
    scale::{BLOCK, m, m64},
    surface::{Column, Cover},
};

/// Tuft colours and the share of columns that grow one.
fn tufts(biome: Biome) -> Option<(Voxel, Voxel, f32)> {
    Some(match biome {
        Biome::Grassland => (mat::TUFT_MEADOW, mat::TUFT, 0.12),
        Biome::Deciduous => (mat::TUFT, mat::TUFT_LUSH, 0.07),
        Biome::Woodland => (mat::TUFT, mat::TUFT_MEADOW, 0.08),
        Biome::OldGrowth => (mat::TUFT_LUSH, mat::TUFT, 0.04),
        Biome::Rainforest => (mat::TUFT_LUSH, mat::TUFT, 0.10),
        Biome::Taiga => (mat::TUFT_DRY, mat::TUFT, 0.03),
        Biome::Heath => (mat::TUFT_DRY, mat::TUFT, 0.05),
        Biome::Savanna => (mat::TUFT_DRY, mat::TUFT_MEADOW, 0.08),
        _ => return None,
    })
}

/// The petals that go with a flowering ground voxel.
fn bloom(surface: Voxel) -> Option<Voxel> {
    Some(match surface {
        mat::FLOWER_RED => mat::BLOOM_RED,
        mat::FLOWER_YELLOW => mat::BLOOM_YELLOW,
        mat::FLOWER_WHITE => mat::BLOOM_WHITE,
        mat::FLOWER_PURPLE | mat::HEATHER => mat::BLOOM_PURPLE,
        mat::FLOWER_BLUE => mat::BLOOM_BLUE,
        mat::FLOWER_PINK => mat::BLOOM_PINK,
        _ => return None,
    })
}

/// Lays out boxes in cells relative to the corner of one voxel.
struct Plot<'a> {
    origin: [i32; 3],
    out: &'a mut Vec<PlantBox>,
}

impl Plot<'_> {
    /// A box from `lo` up to, but not including, `hi`. Its ends sway with the wind if
    /// their flags say so: a stem is rooted at the bottom and free at the top.
    fn add(&mut self, lo: [i32; 3], hi: [i32; 3], voxel: Voxel, sway: (bool, bool)) {
        let at = |p: [i32; 3]| [self.origin[0] + p[0], self.origin[1] + p[1], self.origin[2] + p[2]];
        self.out.push(PlantBox {
            lo: at(lo),
            hi: at(hi),
            voxel,
            sway_lo: sway.0,
            sway_hi: sway.1,
        });
    }

    /// A vertical stem one cell square, rooted on the ground.
    fn stem(&mut self, x: i32, z: i32, height: i32, voxel: Voxel) {
        self.add([x, 0, z], [x + 1, height, z + 1], voxel, (false, true));
    }
}

fn tuft(p: &mut Plot, rng: &mut Rng, main: Voxel, other: Voxel) {
    for blade in 0..rng.int(4, 7) {
        let (x, z) = (rng.int(0, CELLS - 1), rng.int(0, CELLS - 1));
        let height = rng.int(2, 5);
        let colour = if blade % 3 == 2 { other } else { main };
        p.stem(x, z, height, colour);
        // Some blades bend over at the tip.
        if rng.unit() < 0.3 {
            let lean = if x < CELLS / 2 { 1 } else { -1 };
            p.add([x + lean, height, z], [x + lean + 1, height + rng.int(1, 2), z + 1], colour, (true, true));
        }
    }
}

fn flower(p: &mut Plot, rng: &mut Rng, petals: Voxel) {
    let (x, z) = (rng.int(1, CELLS - 2), rng.int(1, CELLS - 2));
    let height = rng.int(3, 5);
    p.stem(x, z, height, mat::TUFT_LUSH);
    // A leaf partway up.
    let side = if rng.unit() < 0.5 { 1 } else { -1 };
    p.add([x + side, 1, z], [x + side + 1, 2, z + 1], mat::TUFT, (false, true));
    // Petals in a cross, with a contrasting eye on top.
    p.add([x - 1, height, z], [x + 2, height + 1, z + 1], petals, (true, true));
    p.add([x, height, z - 1], [x + 1, height + 1, z + 2], petals, (true, true));
    let eye = if petals == mat::BLOOM_YELLOW { mat::REED_HEAD } else { mat::BLOOM_YELLOW };
    p.add([x, height + 1, z], [x + 1, height + 2, z + 1], eye, (true, true));
}

fn reeds(p: &mut Plot, rng: &mut Rng) {
    for _ in 0..rng.int(2, 3) {
        let (x, z) = (rng.int(0, CELLS - 2), rng.int(0, CELLS - 2));
        let height = rng.int(10, 20);
        p.stem(x, z, height, mat::REED);
        if rng.unit() < 0.5 {
            p.add([x, height - 5, z], [x + 2, height - 1, z + 2], mat::REED_HEAD, (true, true));
        }
    }
}

fn mushroom(p: &mut Plot, rng: &mut Rng) {
    let cap = if rng.unit() < 0.5 { mat::MUSHROOM_RED } else { mat::MUSHROOM_TAN };
    let (x, z) = (rng.int(1, CELLS - 2), rng.int(1, CELLS - 2));
    p.add([x, 0, z], [x + 1, 2, z + 1], mat::MUSHROOM_STEM, (false, false));
    p.add([x - 1, 2, z - 1], [x + 2, 3, z + 2], cap, (false, false));
    p.add([x, 3, z], [x + 1, 4, z + 1], cap, (false, false));
}

/// A pad floating on the surface plane `y` (in cells, relative to the plot).
fn lily(p: &mut Plot, rng: &mut Rng, y: i32) {
    let top = y;
    let size = rng.int(3, CELLS);
    let (x, z) = (rng.int(0, CELLS - size), rng.int(0, CELLS - size));
    p.add([x, top - 1, z], [x + size, top, z + size - 1], mat::LILY, (false, false));
    // The other row is shorter, leaving the notch a lily pad has.
    p.add([x, top - 1, z + size - 1], [x + size - 1, top, z + size], mat::LILY, (false, false));
    if rng.unit() < 0.2 {
        p.add([x + 1, top, z + 1], [x + 2, top + 1, z + 2], mat::BLOOM_PINK, (false, false));
    }
}

/// Adds whatever grows on the column at world position `(x, z)`. `surface` is its top
/// voxel, or the flower the ground carries there.
pub fn grow(col: &Column, surface: Voxel, x: i32, z: i32, seed: u32, out: &mut Vec<PlantBox>) {
    let h = hash3(seed ^ 0x91a7, x, 0, z);
    let a = unit(h);
    let y = col.top() + 1;
    let mut rng = Rng::new(h.rotate_left(13));
    let mut plot = Plot {
        origin: [x * CELLS, y * CELLS, z * CELLS],
        out,
    };
    // Plants of one kind grow in clumps.
    let patch = |salt: u32, metres: f64| perlin2(seed ^ salt, x as f64 / m64(metres), z as f64 / m64(metres));

    match col.cover {
        Cover::Bank if a < 0.25 && patch(0x4eed, 3.2) > 0.05 => reeds(&mut plot, &mut rng),
        Cover::LakeBed => {
            if let Some(k) = col.water_surface()
                && k - col.top() >= 2 * BLOCK
                && a < 0.10
                && patch(0x11e5, 4.8) > 0.2
            {
                lily(&mut plot, &mut rng, (k - y) * CELLS);
            }
        }
        Cover::Land if col.clear >= m(0.8) && col.water_surface().is_none() => {
            if let Some(petals) = bloom(surface) {
                flower(&mut plot, &mut rng, petals);
                return;
            }
            let forest = matches!(col.biome, Biome::OldGrowth | Biome::Rainforest | Biome::Taiga);
            if forest && a < 0.012 && patch(0x3051, 6.4) > 0.15 {
                mushroom(&mut plot, &mut rng);
                return;
            }
            if let Some((main, other, density)) = tufts(col.biome)
                && surface == mat::ground_cover(col.biome)
                && a < density
            {
                tuft(&mut plot, &mut rng, main, other);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(cover: Cover, biome: Biome) -> Column {
        Column {
            ground: 40.0,
            water: None,
            cover,
            biome,
            temperature: 0.5,
            river: f32::MAX,
            clear: f32::MAX,
        }
    }

    fn grown(col: &Column, surface: Voxel) -> Vec<PlantBox> {
        let mut out = Vec::new();
        for x in 0..300 {
            for z in 0..300 {
                grow(col, surface, x, z, 5, &mut out);
            }
        }
        out
    }

    #[test]
    fn meadows_grow_tufts_and_bare_ground_grows_nothing() {
        let meadow = column(Cover::Land, Biome::Grassland);
        assert!(grown(&meadow, mat::GRASS_MEADOW).iter().any(|p| p.voxel == mat::TUFT_MEADOW));
        assert!(grown(&column(Cover::Land, Biome::Desert), mat::DESERT_SAND).is_empty());
        assert!(grown(&column(Cover::Trail, Biome::Grassland), mat::PATH_DIRT).is_empty());
    }

    #[test]
    fn nothing_grows_close_to_a_trail() {
        let mut col = column(Cover::Land, Biome::Grassland);
        col.clear = 0.0;
        assert!(grown(&col, mat::GRASS_MEADOW).is_empty());
    }

    #[test]
    fn every_kind_of_plant_grows_and_stays_in_its_own_column() {
        let banks = column(Cover::Bank, Biome::Deciduous);
        let mut lake = column(Cover::LakeBed, Biome::Deciduous);
        lake.water = Some(40 + 8 * BLOCK);
        let forest = column(Cover::Land, Biome::OldGrowth);
        let mut all = grown(&banks, mat::GRAVEL);
        all.extend(grown(&lake, mat::MUD));
        all.extend(grown(&forest, mat::GRASS_LUSH));
        all.extend(grown(&column(Cover::Land, Biome::Grassland), mat::FLOWER_BLUE));
        for wanted in [mat::REED, mat::REED_HEAD, mat::LILY, mat::MUSHROOM_STEM, mat::BLOOM_BLUE] {
            assert!(all.iter().any(|p| p.voxel == wanted), "never grew {wanted}");
        }
        for b in &all {
            for axis in [0, 2] {
                assert_eq!(b.lo[axis].div_euclid(CELLS), (b.hi[axis] - 1).div_euclid(CELLS), "{b:?} leaves its column");
            }
            assert!((0..3).all(|axis| b.lo[axis] < b.hi[axis]), "{b:?} is empty");
        }
    }

    #[test]
    fn no_plant_is_a_whole_voxel_block() {
        let mut all = grown(&column(Cover::Land, Biome::Grassland), mat::GRASS_MEADOW);
        all.extend(grown(&column(Cover::Land, Biome::Grassland), mat::FLOWER_RED));
        all.extend(grown(&column(Cover::Bank, Biome::Deciduous), mat::GRAVEL));
        assert!(!all.is_empty());
        for b in &all {
            assert!(b.hi[0] - b.lo[0] < CELLS && b.hi[2] - b.lo[2] < CELLS, "{b:?} is a voxel wide");
        }
    }

    #[test]
    fn the_same_column_always_grows_the_same_plant() {
        let col = column(Cover::Land, Biome::Grassland);
        let (a, b) = (grown(&col, mat::GRASS_MEADOW), grown(&col, mat::GRASS_MEADOW));
        assert_eq!(a.len(), b.len());
        assert!(a.iter().zip(&b).all(|(p, q)| p.lo == q.lo && p.hi == q.hi && p.voxel == q.voxel));
    }
}
