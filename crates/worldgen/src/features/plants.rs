//! Small plants: grasses, flowers, reeds, mushrooms and lily pads, modelled from
//! blocks a quarter of a voxel across that stand on top of the voxel grid. Each is
//! decided by its own column alone and stays inside it, so neighbouring chunks agree
//! without looking at each other. The boxes of one plant never overlap, because faces
//! sharing a plane flicker against each other.

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

/// The kinds of grass, each with its own form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Grass {
    /// Short, dense and low.
    Lawn,
    /// Tall stems, many of them carrying seed heads.
    Meadow,
    /// Wide tapering blades, for wet and shaded ground.
    Broad,
    /// A tight dry clump whose tips splay outward.
    Bunch,
    /// A few short separate blades on nearly every block, for the cold and the heights.
    Alpine,
    /// Fine upright blades in a line, beside the water.
    Sedge,
}

impl Grass {
    /// The share of columns that grow it.
    fn share(self, sward: &Sward) -> f32 {
        let thickness = match self {
            // Short enough to carpet the ground, whatever the biome's usual density.
            Grass::Alpine => return ALPINE_SHARE,
            Grass::Lawn => 1.3,
            Grass::Meadow | Grass::Broad => 1.0,
            Grass::Bunch => 0.6,
            Grass::Sedge => 1.6,
        };
        sward.density * thickness
    }
}

/// The grass of one biome: its two colours, the share of columns that grow any, and
/// the two kinds that take turns across the land in drifts.
struct Sward {
    main: Voxel,
    other: Voxel,
    density: f32,
    common: Grass,
    drift: Grass,
}

fn sward(biome: Biome) -> Option<Sward> {
    let s = |main, other, density, common, drift| Sward { main, other, density, common, drift };
    Some(match biome {
        Biome::Grassland => s(mat::TUFT_MEADOW, mat::TUFT, 0.06, Grass::Meadow, Grass::Lawn),
        Biome::Deciduous => s(mat::TUFT, mat::TUFT_LUSH, 0.035, Grass::Lawn, Grass::Broad),
        Biome::Woodland => s(mat::TUFT, mat::TUFT_MEADOW, 0.04, Grass::Lawn, Grass::Bunch),
        Biome::OldGrowth => s(mat::TUFT_LUSH, mat::TUFT, 0.02, Grass::Broad, Grass::Lawn),
        Biome::Rainforest => s(mat::TUFT_LUSH, mat::TUFT, 0.05, Grass::Broad, Grass::Lawn),
        Biome::Tundra => s(mat::TUFT_ALPINE, mat::TUFT_DRY, 0.035, Grass::Alpine, Grass::Alpine),
        Biome::Taiga => s(mat::TUFT_ALPINE, mat::TUFT_DRY, 0.025, Grass::Alpine, Grass::Alpine),
        Biome::Heath => s(mat::TUFT_DRY, mat::TUFT, 0.025, Grass::Bunch, Grass::Lawn),
        Biome::Savanna => s(mat::TUFT_DRY, mat::TUFT_MEADOW, 0.04, Grass::Bunch, Grass::Meadow),
        _ => return None,
    })
}

/// Noise that varies over about `metres`, for things that grow in patches.
fn patch(seed: u32, salt: u32, x: i32, z: i32, metres: f64) -> f32 {
    perlin2(seed ^ salt, x as f64 / m64(metres), z as f64 / m64(metres))
}

/// How far from a river's edge sedge can take over from the local grass.
const SEDGE_REACH: f32 = m(3.2);

/// How much of the cold ground alpine grass covers.
const ALPINE_SHARE: f32 = 0.2;

/// Below this temperature, which already allows for altitude, only alpine grass grows.
const COLD: f32 = 0.32;

/// The grass that belongs at this column, if the biome grows any.
fn grass_at(col: &Column, seed: u32, x: i32, z: i32) -> Option<(Grass, Sward)> {
    let sward = sward(col.biome)?;
    let reach = SEDGE_REACH * (1.0 + patch(seed, 0x5ed6, x, z, 2.4));
    if col.river < reach {
        return Some((Grass::Sedge, sward));
    }
    if col.temperature + patch(seed, 0xc01d, x, z, 9.6) * 0.03 < COLD {
        return Some((Grass::Alpine, sward));
    }
    // A broad drift with a ragged edge.
    let drift = patch(seed, 0x6a55, x, z, 14.4) + patch(seed, 0x6a56, x, z, 3.6) * 0.3;
    let kind = if drift > 0.08 { sward.drift } else { sward.common };
    Some((kind, sward))
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

/// The four 2x2 quarters of a voxel's footprint, by their low corners.
const QUARTERS: [(i32, i32); 4] = [(0, 0), (2, 0), (0, 2), (2, 2)];

/// The quarters in random order.
fn shuffled_quarters(rng: &mut Rng) -> [(i32, i32); 4] {
    let mut q = QUARTERS;
    for i in 0..3 {
        q.swap(i, rng.int(i as i32, 3) as usize);
    }
    q
}

fn lawn(p: &mut Plot, rng: &mut Rng, main: Voxel, other: Voxel) {
    for (qx, qz) in QUARTERS {
        if rng.unit() < 0.2 {
            continue;
        }
        let colour = if rng.unit() < 0.3 { other } else { main };
        let height = rng.int(1, 2);
        // A low mat over half the quarter, and sometimes a taller blade in the other half.
        let across = rng.unit() < 0.5;
        let (w, d) = if across { (2, 1) } else { (1, 2) };
        p.add([qx, 0, qz], [qx + w, height, qz + d], colour, (false, true));
        if rng.unit() < 0.5 {
            let along = rng.int(0, 1);
            let (x, z) = if across { (qx + along, qz + 1) } else { (qx + 1, qz + along) };
            p.stem(x, z, rng.int(2, 3), colour);
        }
    }
}

fn meadow(p: &mut Plot, rng: &mut Rng, main: Voxel, other: Voxel) {
    let quarters = shuffled_quarters(rng);
    let stems = rng.int(3, 4) as usize;
    for (i, &(qx, qz)) in quarters[..stems].iter().enumerate() {
        let (x, z) = (qx + rng.int(0, 1), qz + rng.int(0, 1));
        let height = rng.int(5, 9);
        p.stem(x, z, height, if i % 3 == 2 { other } else { main });
        if i < 2 && rng.unit() < 0.75 {
            p.add([x, height, z], [x + 1, height + 3, z + 1], mat::SEED_HEAD, (true, true));
        }
    }
}

fn broad(p: &mut Plot, rng: &mut Rng) {
    let quarters = shuffled_quarters(rng);
    let blades = rng.int(3, 4) as usize;
    for (i, &(qx, qz)) in quarters[..blades].iter().enumerate() {
        let colour = if i % 3 == 2 { mat::TUFT } else { mat::TUFT_LUSH };
        let height = rng.int(3, 6);
        let across = rng.unit() < 0.5;
        let shift = rng.int(0, 1);
        // Two cells wide and one thick, narrowing to a single cell at the tip.
        let (x, z, w, d) = if across { (qx, qz + shift, 2, 1) } else { (qx + shift, qz, 1, 2) };
        p.add([x, 0, z], [x + w, height, z + d], colour, (false, true));
        let along = rng.int(0, 1);
        let (tx, tz) = if across { (x + along, z) } else { (x, z + along) };
        p.add([tx, height, tz], [tx + 1, height + rng.int(1, 2), tz + 1], colour, (true, true));
    }
}

fn bunch(p: &mut Plot, rng: &mut Rng) {
    for (x, z) in [(1, 1), (2, 1), (1, 2), (2, 2)] {
        if rng.unit() < 0.15 {
            continue;
        }
        let colour = if rng.unit() < 0.4 { mat::TUFT_DRY } else { mat::TUFT_STRAW };
        let height = rng.int(3, 6);
        p.stem(x, z, height, colour);
        // The tip leans out over the edge of the clump.
        if rng.unit() < 0.7 {
            let outward = if rng.unit() < 0.5 {
                (if x == 1 { 0 } else { 3 }, z)
            } else {
                (x, if z == 1 { 0 } else { 3 })
            };
            p.add([outward.0, height, outward.1], [outward.0 + 1, height + rng.int(1, 3), outward.1 + 1], colour, (true, true));
        }
    }
}

fn alpine(p: &mut Plot, rng: &mut Rng, main: Voxel, other: Voxel) {
    // One colour to a block, and each blade alone in its own quarter of the voxel.
    let colour = if rng.unit() < 0.25 { other } else { main };
    let quarters = shuffled_quarters(rng);
    let blades = rng.int(3, 4) as usize;
    for &(qx, qz) in &quarters[..blades] {
        p.stem(qx + rng.int(0, 1), qz + rng.int(0, 1), rng.int(2, 3), colour);
    }
}

fn sedge(p: &mut Plot, rng: &mut Rng) {
    let along_x = rng.unit() < 0.5;
    let line = rng.int(1, 2);
    for i in 0..CELLS {
        let off = line + rng.int(0, 1);
        let (x, z) = if along_x { (i, off) } else { (off, i) };
        let height = rng.int(4, 9);
        p.stem(x, z, height, if rng.unit() < 0.25 { mat::TUFT_LUSH } else { mat::TUFT_SEDGE });
        if rng.unit() < 0.12 {
            p.add([x, height, z], [x + 1, height + 2, z + 1], mat::REED_HEAD, (true, true));
        }
    }
}

fn grass(p: &mut Plot, rng: &mut Rng, kind: Grass, sward: &Sward) {
    match kind {
        Grass::Lawn => lawn(p, rng, sward.main, sward.other),
        Grass::Meadow => meadow(p, rng, sward.main, sward.other),
        Grass::Broad => broad(p, rng),
        Grass::Bunch => bunch(p, rng),
        Grass::Alpine => alpine(p, rng, sward.main, sward.other),
        Grass::Sedge => sedge(p, rng),
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
    p.add([x, height, z - 1], [x + 1, height + 1, z], petals, (true, true));
    p.add([x, height, z + 1], [x + 1, height + 1, z + 2], petals, (true, true));
    let eye = if petals == mat::BLOOM_YELLOW { mat::REED_HEAD } else { mat::BLOOM_YELLOW };
    p.add([x, height + 1, z], [x + 1, height + 2, z + 1], eye, (true, true));
}

fn reeds(p: &mut Plot, rng: &mut Rng) {
    // Each stem has a quarter of the voxel to itself, room for a head around it.
    let mut quarters = [(0, 0), (2, 0), (0, 2), (2, 2)];
    let stems = rng.int(2, 3) as usize;
    for i in 0..stems {
        quarters.swap(i, rng.int(i as i32, 3) as usize);
    }
    for &(qx, qz) in &quarters[..stems] {
        let (x, z) = (qx + rng.int(0, 1), qz + rng.int(0, 1));
        let height = rng.int(10, 20);
        if rng.unit() < 0.5 {
            // The stem stops under the head and a spike carries on above it.
            let (below, above) = (height - 5, height - 1);
            p.stem(x, z, below, mat::REED);
            p.add([qx, below, qz], [qx + 2, above, qz + 2], mat::REED_HEAD, (true, true));
            p.add([x, above, z], [x + 1, height, z + 1], mat::REED, (true, true));
        } else {
            p.stem(x, z, height, mat::REED);
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

    match col.cover {
        Cover::Bank if a < 0.25 && patch(seed, 0x4eed, x, z, 3.2) > 0.05 => reeds(&mut plot, &mut rng),
        Cover::LakeBed => {
            if let Some(k) = col.water_surface()
                && k - col.top() >= 2 * BLOCK
                && a < 0.10
                && patch(seed, 0x11e5, x, z, 4.8) > 0.2
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
            if forest && a < 0.012 && patch(seed, 0x3051, x, z, 6.4) > 0.15 {
                mushroom(&mut plot, &mut rng);
                return;
            }
            if let Some((kind, sward)) = grass_at(col, seed, x, z)
                && surface == mat::ground_cover(col.biome)
                && a < kind.share(&sward)
            {
                grass(&mut plot, &mut rng, kind, &sward);
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
    fn the_boxes_of_a_plant_never_overlap() {
        let mut lake = column(Cover::LakeBed, Biome::Deciduous);
        lake.water = Some(40 + 8 * BLOCK);
        let kinds = [
            (column(Cover::Bank, Biome::Deciduous), mat::GRAVEL),
            (lake, mat::MUD),
            (column(Cover::Land, Biome::OldGrowth), mat::GRASS_LUSH),
            (column(Cover::Land, Biome::Grassland), mat::GRASS_MEADOW),
            (column(Cover::Land, Biome::Grassland), mat::FLOWER_YELLOW),
            (column(Cover::Land, Biome::Heath), mat::ground_cover(Biome::Heath)),
            (column(Cover::Land, Biome::Tundra), mat::ground_cover(Biome::Tundra)),
            (column(Cover::Land, Biome::Savanna), mat::ground_cover(Biome::Savanna)),
        ];
        let mut plants = 0;
        for (col, surface) in &kinds {
            for x in 0..300 {
                for z in 0..300 {
                    let mut boxes = Vec::new();
                    grow(col, *surface, x, z, 5, &mut boxes);
                    plants += !boxes.is_empty() as u32;
                    for (i, a) in boxes.iter().enumerate() {
                        for b in &boxes[i + 1..] {
                            let overlap = (0..3).all(|axis| a.lo[axis] < b.hi[axis] && b.lo[axis] < a.hi[axis]);
                            assert!(!overlap, "{a:?} overlaps {b:?}");
                        }
                    }
                }
            }
        }
        assert!(plants > 100);
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

    const ALL_GRASSES: [Grass; 6] = [Grass::Lawn, Grass::Meadow, Grass::Broad, Grass::Bunch, Grass::Alpine, Grass::Sedge];

    fn model(kind: Grass, salt: u32) -> Vec<PlantBox> {
        let mut out = Vec::new();
        let mut plot = Plot { origin: [0, 0, 0], out: &mut out };
        grass(&mut plot, &mut Rng::new(hash3(77, salt as i32, 0, 3)), kind, &sward(Biome::Grassland).unwrap());
        out
    }

    #[test]
    fn every_grass_is_well_formed() {
        for kind in ALL_GRASSES {
            for salt in 0..2000 {
                let boxes = model(kind, salt);
                for (i, a) in boxes.iter().enumerate() {
                    assert!((0..3).all(|axis| a.lo[axis] < a.hi[axis]), "{kind:?}: {a:?} is empty");
                    for axis in [0, 2] {
                        assert!(a.lo[axis] >= 0 && a.hi[axis] <= CELLS, "{kind:?}: {a:?} leaves its column");
                    }
                    assert!(a.hi[0] - a.lo[0] < CELLS && a.hi[2] - a.lo[2] < CELLS, "{kind:?}: {a:?} is a voxel wide");
                    for b in &boxes[i + 1..] {
                        let overlap = (0..3).all(|axis| a.lo[axis] < b.hi[axis] && b.lo[axis] < a.hi[axis]);
                        assert!(!overlap, "{kind:?}: {a:?} overlaps {b:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_grasses_differ_in_shape() {
        let height = |kind| (0..500).map(|s| model(kind, s).iter().map(|b| b.hi[1]).max().unwrap_or(0) as f32).sum::<f32>() / 500.0;
        assert!(height(Grass::Meadow) > height(Grass::Lawn) * 2.0);
        assert!(height(Grass::Sedge) > height(Grass::Bunch));
        assert!((0..500).all(|s| model(Grass::Alpine, s).iter().all(|b| b.hi[1] <= 3)), "alpine grass grows low");
        for salt in 0..500 {
            let blades = model(Grass::Alpine, salt);
            assert!((3..=4).contains(&blades.len()));
            assert!(blades.iter().all(|b| b.hi[0] - b.lo[0] == 1 && b.hi[2] - b.lo[2] == 1), "one microvoxel across");
        }
        for salt in 0..200 {
            assert!(model(Grass::Broad, salt).iter().any(|b| b.hi[0] - b.lo[0] == 2 || b.hi[2] - b.lo[2] == 2));
            assert!(model(Grass::Sedge, salt).iter().all(|b| b.hi[0] - b.lo[0] == 1 && b.hi[2] - b.lo[2] == 1));
        }
    }

    #[test]
    fn grass_kinds_form_drifts_and_sedge_keeps_to_the_water() {
        let dry = column(Cover::Land, Biome::Grassland);
        let at = |x, z| grass_at(&dry, 5, x, z).unwrap().0;
        let (mut lawn, mut meadow, mut neighbours_agree, mut total) = (0, 0, 0, 0);
        for x in 0..1500 {
            for z in 0..1500 {
                match at(x, z) {
                    Grass::Lawn => lawn += 1,
                    Grass::Meadow => meadow += 1,
                    other => panic!("{other:?} in a dry meadow"),
                }
                if x % 7 == 0 && z % 7 == 0 {
                    total += 1;
                    neighbours_agree += (at(x, z) == at(x + 1, z)) as u32;
                }
            }
        }
        assert!(lawn > 100_000 && meadow > 100_000, "lawn {lawn}, meadow {meadow}");
        assert!(neighbours_agree as f32 > total as f32 * 0.9, "{neighbours_agree} of {total}");

        let mut wet = column(Cover::Land, Biome::Grassland);
        wet.river = 1.0;
        assert_eq!(grass_at(&wet, 5, 10, 10).unwrap().0, Grass::Sedge);
    }

    #[test]
    fn the_cold_and_the_heights_grow_alpine_grass() {
        // A warm-climate biome high enough up to be cold.
        let mut high = column(Cover::Land, Biome::Grassland);
        high.temperature = 0.26;
        assert_eq!(grass_at(&high, 5, 40, 40).unwrap().0, Grass::Alpine);

        let tundra = column(Cover::Land, Biome::Tundra);
        assert!(grown(&tundra, mat::ground_cover(Biome::Tundra)).iter().any(|p| p.voxel == mat::TUFT_ALPINE));
    }

    #[test]
    fn the_same_column_always_grows_the_same_plant() {
        let col = column(Cover::Land, Biome::Grassland);
        let (a, b) = (grown(&col, mat::GRASS_MEADOW), grown(&col, mat::GRASS_MEADOW));
        assert_eq!(a.len(), b.len());
        assert!(a.iter().zip(&b).all(|(p, q)| p.lo == q.lo && p.hi == q.hi && p.voxel == q.voxel));
    }
}
