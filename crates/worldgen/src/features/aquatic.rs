//! Plants of the water, chosen from what the surface sampler knows of a column: its
//! cover (lake bed, river bed, bank, sea floor), the depth of water over it and how warm
//! the climate is. Selection is a lookup in `kind_at`; the models are small boxes a
//! quarter of a voxel across, like the other plants, each deciding by its own column
//! alone and staying inside it, so neighbouring chunks agree.
//!
//! Reeds on banks and lily pads on lake beds are the plant layer's own; this adds
//! cattails and algae to still water, rushes, trailing weed and mossy stones to rivers,
//! and eelgrass, kelp and mangrove roots to the sea and its river mouths.

use voxel::{
    Voxel,
    mesh::{CELLS, PlantBox},
};

use super::shape::Rng;
use crate::{
    materials as mat,
    noise::{hash3, perlin2, unit},
    scale::{BLOCK, m, m64},
    surface::{Column, Cover},
};

/// An aquatic plant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Aquatic {
    /// Tall brown-headed stems at the edge of still water.
    Cattail,
    /// A thin green mat floating on warm, still water.
    Algae,
    /// Short stiff stems on river banks.
    Rush,
    /// Long soft strands streaming from the river bed.
    Weed,
    /// A stone on the river bed with moss on top.
    MossyStone,
    /// Short blades in sandy sea shallows.
    Eelgrass,
    /// Tall stems rising from deeper sea floor.
    Kelp,
    /// Stilt roots at a warm river mouth.
    MangroveRoots,
}

/// Noise that varies over about `metres`, for things that grow in patches.
fn patch(seed: u32, salt: u32, x: i32, z: i32, metres: f64) -> f32 {
    perlin2(seed ^ salt, x as f64 / m64(metres), z as f64 / m64(metres))
}

/// Depth of water over the column's ground, in voxels.
fn depth(col: &Column) -> f32 {
    col.water_surface().map_or(0.0, |k| k as f32 - col.ground)
}

/// Warm enough for algae and mangroves.
const WARM: f32 = 0.62;
const HOT: f32 = 0.72;
/// How deep the sea is still shallow, and where kelp stops growing.
const SHALLOWS: f32 = m(2.0);
const KELP_MAX: f32 = m(14.0);

/// What grows on this column, and the share of such columns that grow it. `a` is the
/// column's own roll in 0..1.
pub fn kind_at(col: &Column, seed: u32, x: i32, z: i32) -> Option<(Aquatic, f32)> {
    let wet = depth(col);
    match col.cover {
        // Beside still water, and in its shallows.
        Cover::LakeBed if wet > 0.0 && wet <= 2.0 * BLOCK as f32 => {
            if col.temperature > WARM && patch(seed, 0x16a3, x, z, 6.4) > 0.1 {
                Some((Aquatic::Algae, 0.5))
            } else if patch(seed, 0xca77, x, z, 3.6) > -0.05 {
                Some((Aquatic::Cattail, 0.22))
            } else {
                None
            }
        }
        Cover::Bank if col.water_surface().is_none() && col.temperature > HOT && col.ground < m(1.6) => {
            Some((Aquatic::MangroveRoots, 0.12))
        }
        Cover::Bank if col.water_surface().is_none() => Some((Aquatic::Rush, 0.10)),
        Cover::RiverBed if wet > 0.0 => {
            if patch(seed, 0xee1d, x, z, 3.2) > 0.1 {
                Some((Aquatic::Weed, 0.16))
            } else {
                Some((Aquatic::MossyStone, 0.025))
            }
        }
        Cover::SeaFloor if wet > 0.0 && wet <= SHALLOWS => {
            (patch(seed, 0x5ea6, x, z, 5.6) > -0.1).then_some((Aquatic::Eelgrass, 0.45))
        }
        Cover::SeaFloor if wet > SHALLOWS && wet <= KELP_MAX => {
            (patch(seed, 0x4e1b, x, z, 8.0) > 0.0).then_some((Aquatic::Kelp, 0.30))
        }
        // Kelp holds fast to the bare rock of a wave-cut shelf. Shelves only form on
        // sea coasts, and the bed must lie below sea level, which keeps it out of fresh water.
        Cover::Shelf if wet > m(1.0) && wet <= KELP_MAX && col.ground < 0.0 => {
            (patch(seed, 0x5e1f, x, z, 6.4) > -0.15).then_some((Aquatic::Kelp, 0.35))
        }
        _ => None,
    }
}

/// Lays out boxes in cells relative to the corner of one voxel.
struct Plot<'a> {
    origin: [i32; 3],
    out: &'a mut Vec<PlantBox>,
}

impl Plot<'_> {
    fn add(&mut self, lo: [i32; 3], hi: [i32; 3], voxel: Voxel, sway: (bool, bool)) {
        let at = |p: [i32; 3]| [self.origin[0] + p[0], self.origin[1] + p[1], self.origin[2] + p[2]];
        self.out.push(PlantBox { lo: at(lo), hi: at(hi), voxel, sway_lo: sway.0, sway_hi: sway.1, thin: 0 });
    }

    /// A vertical stem one cell square, rooted on the ground.
    fn stem(&mut self, x: i32, z: i32, from: i32, to: i32, voxel: Voxel, sway_top: bool) {
        self.add([x, from, z], [x + 1, to, z + 1], voxel, (false, sway_top));
    }
}

const QUARTERS: [(i32, i32); 4] = [(0, 0), (2, 0), (0, 2), (2, 2)];

/// `count` quarters of the voxel, chosen at random.
fn pick_quarters(rng: &mut Rng, count: usize) -> [(i32, i32); 4] {
    let mut q = QUARTERS;
    for i in 0..count.min(3) {
        q.swap(i, rng.int(i as i32, 3) as usize);
    }
    q
}

fn build(p: &mut Plot, rng: &mut Rng, kind: Aquatic, room: i32) {
    match kind {
        Aquatic::Cattail => {
            let n = rng.int(1, 3) as usize;
            for &(qx, qz) in &pick_quarters(rng, n)[..n] {
                let (x, z) = (qx + rng.int(0, 1), qz + rng.int(0, 1));
                let height = rng.int(10, 16).min(room);
                // A stem with a dark sausage of a head near the top.
                p.stem(x, z, 0, height - 4, mat::REED, true);
                p.add([qx, height - 4, qz], [qx + 2, height, qz + 2], mat::REED_HEAD, (true, true));
            }
        }
        Aquatic::Algae => {
            // A mat resting on the surface plane, at `room` cells above the bed.
            let (x, z) = (rng.int(0, 1), rng.int(0, 1));
            p.add([x, room - 1, z], [x + 3, room, z + 2], mat::TUFT_LUSH, (false, false));
            if rng.unit() < 0.4 {
                p.add([0, room - 1, 3], [2, room, 4], mat::MOSS, (false, false));
            }
        }
        Aquatic::Rush => {
            let n = rng.int(2, 3) as usize;
            for &(qx, qz) in &pick_quarters(rng, n)[..n] {
                p.stem(qx + rng.int(0, 1), qz + rng.int(0, 1), 0, rng.int(6, 11), mat::TUFT_SEDGE, true);
            }
        }
        Aquatic::Weed => {
            // Strands that lean downstream in the current: the tips sway.
            let n = rng.int(2, 3) as usize;
            for &(qx, qz) in &pick_quarters(rng, n)[..n] {
                let height = rng.int(5, 10).min(room);
                p.stem(qx + rng.int(0, 1), qz + rng.int(0, 1), 0, height, mat::TUFT_LUSH, true);
            }
        }
        Aquatic::MossyStone => {
            let (x, z) = (rng.int(0, 1), rng.int(0, 1));
            p.add([x, 0, z], [x + 3, 2, z + 3], mat::STONE, (false, false));
            p.add([x, 2, z], [x + 3, 3, z + 2], mat::MOSS, (false, false));
        }
        Aquatic::Eelgrass => {
            let n = rng.int(2, 3) as usize;
            for &(qx, qz) in &pick_quarters(rng, n)[..n] {
                p.stem(qx + rng.int(0, 1), qz + rng.int(0, 1), 0, rng.int(4, 9).min(room), mat::TUFT_LUSH, true);
            }
        }
        Aquatic::Kelp => {
            // One long frond per voxel, banded in two shades and swaying at the top.
            let (x, z) = (rng.int(0, 3), rng.int(0, 3));
            let height = rng.int(14, 30).min(room);
            let mid = height * 2 / 3;
            p.stem(x, z, 0, mid, mat::MOSS, false);
            p.stem(x, z, mid, height, mat::BARK_WILLOW, true);
            if rng.unit() < 0.5 {
                let (qx, qz) = ((x + 2) % 4, (z + 2) % 4);
                p.stem(qx, qz, 0, (height / 2).max(2), mat::MOSS, true);
            }
        }
        Aquatic::MangroveRoots => {
            // Stilts arching out of the mud: wide at the foot, narrowing as they rise.
            let n = rng.int(2, 3) as usize;
            for &(qx, qz) in &pick_quarters(rng, n)[..n] {
                let height = rng.int(6, 12);
                p.add([qx, 0, qz], [qx + 2, 2, qz + 2], mat::BARK_JUNGLE, (false, false));
                p.stem(qx + rng.int(0, 1), qz + rng.int(0, 1), 2, height, mat::BARK_WILLOW, false);
            }
        }
    }
}

/// Adds whatever grows in or beside the water on the column at world position `(x, z)`.
pub fn grow(col: &Column, x: i32, z: i32, seed: u32, out: &mut Vec<PlantBox>) {
    let Some((kind, share)) = kind_at(col, seed, x, z) else { return };
    let h = hash3(seed ^ 0xa9c1, x, 0, z);
    if unit(h) >= share {
        return;
    }
    let y = col.top() + 1;
    // Cells of clear water above the bed, so nothing pokes through the surface.
    let room = col.water_surface().map_or(i32::MAX, |k| ((k - y) * CELLS - 1).max(0));
    // Algae floats on the surface plane; cattails stand up out of the water.
    let room = match kind {
        Aquatic::Algae => (k_of(col) - y) * CELLS,
        Aquatic::Cattail => 16,
        _ => room,
    };
    let needed = match kind {
        Aquatic::Cattail => 8,
        Aquatic::MossyStone => 3,
        _ => 2,
    };
    if room < needed {
        return;
    }
    let mut plot = Plot { origin: [x * CELLS, y * CELLS, z * CELLS], out };
    build(&mut plot, &mut Rng::new(h.rotate_left(9)), kind, room);
}

fn k_of(col: &Column) -> i32 {
    col.water_surface().unwrap_or(col.top() + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::biome::Biome;

    fn column(cover: Cover, biome: Biome, water_depth: Option<f32>, temperature: f32) -> Column {
        let ground = if cover == Cover::Bank { 1.0 } else { 10.0 };
        Column {
            ground,
            water: water_depth.map(|d| (ground + d).ceil() as i32 + 1),
            cover,
            biome,
            temperature,
            river: 0.0,
            clear: f32::MAX,
        }
    }

    fn strewn(col: &Column) -> Vec<(i32, i32, Vec<PlantBox>)> {
        let mut all = Vec::new();
        for x in 0..160 {
            for z in 0..160 {
                let mut out = Vec::new();
                grow(col, x, z, 4, &mut out);
                all.push((x, z, out));
            }
        }
        all
    }

    #[test]
    fn kelp_grows_on_sea_shelves_only() {
        let mut shelf = column(Cover::Shelf, Biome::Ocean, None, 0.5);
        shelf.ground = -m(4.0);
        shelf.water = Some(1);
        assert!(strewn(&shelf).iter().any(|(_, _, b)| b.iter().any(|b| b.voxel == mat::BARK_WILLOW)));
        // A shelf above sea level, under fresh water, grows none.
        let fresh = column(Cover::Shelf, Biome::Ocean, Some(m(4.0)), 0.5);
        assert!(strewn(&fresh).iter().all(|(_, _, b)| b.is_empty()));
    }

    #[test]
    fn selection_follows_cover_depth_and_climate() {
        let kind = |c: &Column| (0..2000).find_map(|i| kind_at(c, 4, i * 7, i * 3).map(|k| k.0));
        let shallow = column(Cover::SeaFloor, Biome::Ocean, Some(m(1.0)), 0.5);
        let deep = column(Cover::SeaFloor, Biome::Ocean, Some(m(6.0)), 0.5);
        assert_eq!(kind(&shallow), Some(Aquatic::Eelgrass));
        assert_eq!(kind(&deep), Some(Aquatic::Kelp));
        assert_eq!(kind(&column(Cover::SeaFloor, Biome::Ocean, Some(m(40.0)), 0.5)), None);
        assert!(matches!(kind(&column(Cover::RiverBed, Biome::Deciduous, Some(m(1.0)), 0.5)), Some(Aquatic::Weed | Aquatic::MossyStone)));
        assert_eq!(kind(&column(Cover::Bank, Biome::Deciduous, None, 0.5)), Some(Aquatic::Rush));
        assert_eq!(kind(&column(Cover::Bank, Biome::Rainforest, None, 0.9)), Some(Aquatic::MangroveRoots));
        let warm_pond = column(Cover::LakeBed, Biome::Rainforest, Some(2.0), 0.9);
        let cold_pond = column(Cover::LakeBed, Biome::Taiga, Some(2.0), 0.3);
        assert!((0..4000).any(|i| kind_at(&warm_pond, 4, i * 5, i).map(|k| k.0) == Some(Aquatic::Algae)));
        assert!((0..4000).all(|i| kind_at(&cold_pond, 4, i * 5, i).map(|k| k.0) != Some(Aquatic::Algae)));
        assert_eq!(kind(&column(Cover::Land, Biome::Grassland, None, 0.5)), None);
    }

    #[test]
    fn plants_stay_in_their_column_under_the_surface_and_never_overlap() {
        let cases = [
            column(Cover::SeaFloor, Biome::Ocean, Some(m(1.0)), 0.5),
            column(Cover::SeaFloor, Biome::Ocean, Some(m(6.0)), 0.5),
            column(Cover::RiverBed, Biome::Deciduous, Some(m(1.0)), 0.5),
            column(Cover::Bank, Biome::Deciduous, None, 0.5),
            column(Cover::Bank, Biome::Rainforest, None, 0.9),
            column(Cover::LakeBed, Biome::Deciduous, Some(2.0), 0.5),
            column(Cover::LakeBed, Biome::Rainforest, Some(2.0), 0.9),
        ];
        for col in &cases {
            let all = strewn(col);
            assert!(all.iter().filter(|s| !s.2.is_empty()).count() > 100, "{:?} grew little", col.cover);
            let surface = col.water_surface().map(|k| k * CELLS);
            for (x, z, boxes) in &all {
                for (i, a) in boxes.iter().enumerate() {
                    assert!((0..3).all(|k| a.lo[k] < a.hi[k]), "{a:?} is empty");
                    for (axis, c) in [(0, *x), (2, *z)] {
                        assert_eq!(a.lo[axis].div_euclid(CELLS), c, "{a:?} leaves its column");
                        assert_eq!((a.hi[axis] - 1).div_euclid(CELLS), c, "{a:?} leaves its column");
                    }
                    assert!(a.hi[0] - a.lo[0] < CELLS + 1 && a.hi[2] - a.lo[2] < CELLS + 1);
                    // Reeds and cattails stand up out of the water; the rest stay under it.
                    if let Some(s) = surface.filter(|_| !matches!(a.voxel, mat::REED | mat::REED_HEAD)) {
                        assert!(a.hi[1] <= s, "{a:?} pokes through the surface at {s}");
                    }
                    for b in &boxes[i + 1..] {
                        let overlap = (0..3).all(|k| a.lo[k] < b.hi[k] && b.lo[k] < a.hi[k]);
                        assert!(!overlap, "{a:?} overlaps {b:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_column_decides_alone_and_always_the_same() {
        let col = column(Cover::SeaFloor, Biome::Ocean, Some(m(6.0)), 0.5);
        // Across chunk borders and at negative coordinates, asking twice agrees.
        for (x, z) in [(-33, 64), (31, 32), (-1, -1), (0, 0), (63, -64)] {
            let (mut a, mut b) = (Vec::new(), Vec::new());
            grow(&col, x, z, 8, &mut a);
            grow(&col, x, z, 8, &mut b);
            assert_eq!(a.len(), b.len());
            assert!(a.iter().zip(&b).all(|(p, q)| p.lo == q.lo && p.hi == q.hi && p.voxel == q.voxel));
        }
        let (a, b) = (strewn(&col), strewn(&col));
        assert!(a.iter().zip(&b).all(|(p, q)| p.2.len() == q.2.len()));
    }
}
