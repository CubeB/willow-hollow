//! Pebbles and small rocks, modelled from blocks a quarter of a voxel (0.1 m) across,
//! lying on top of the ground. Like the plants they are decided by one column alone and
//! stay inside it, so neighbouring chunks agree without looking at each other, and the
//! boxes of one scatter never overlap.
//!
//! Where they lie follows the ground: gravel bars and river beds, bare rock and scree,
//! the shingle of beaches, and a sparse lichen-spotted scatter in heath and tundra. The
//! stones take their colours from the local rock.

use voxel::{
    Voxel,
    mesh::{CELLS, PlantBox},
};

use super::shape::Rng;
use crate::{
    biome::Biome,
    materials as mat,
    noise::{hash3, unit},
    surface::{Column, Cover},
};

/// What a pebble field is made of: the colours its stones come in.
struct Rock {
    main: Voxel,
    other: Voxel,
    /// A rare third colour: lichen, a shell, an ironstone.
    fleck: Voxel,
}

/// The local rock for a biome.
fn local_rock(biome: Biome) -> Rock {
    match biome {
        Biome::Desert | Biome::Savanna => Rock { main: mat::SANDSTONE, other: mat::SANDSTONE_DARK, fleck: mat::STONE },
        Biome::Alpine | Biome::Tundra | Biome::Taiga => Rock { main: mat::STONE, other: mat::SLATE, fleck: mat::ROCK_DARK },
        _ => Rock { main: mat::STONE, other: mat::ROCK_DARK, fleck: mat::GRAVEL },
    }
}

/// How densely a ground type is strewn, as a share of columns, and the rock it carries.
fn field(col: &Column, surface: Voxel) -> Option<(f32, Rock)> {
    if col.cover == Cover::Trail {
        return None;
    }
    let rock = local_rock(col.biome);
    Some(match (col.cover, surface) {
        // Beaches: sea-smoothed pebbles and the odd shell.
        (Cover::Beach, _) => (0.07, Rock { main: mat::GRAVEL, other: rock.main, fleck: mat::MUSHROOM_STEM }),
        // Gravel bars and river beds.
        (Cover::Bank | Cover::RiverBed, mat::GRAVEL) => (0.45, Rock { main: mat::GRAVEL, other: rock.main, fleck: rock.other }),
        (Cover::RiverBed, mat::SAND) => (0.06, Rock { main: mat::GRAVEL, other: rock.main, fleck: rock.other }),
        // Bare rock and scree: loose stones lie thickest on it.
        (_, mat::STONE | mat::ROCK_DARK) => (0.35, rock),
        // Heath and tundra: a few stones in the turf, some of them lichen-spotted.
        (Cover::Land, s) if matches!(col.biome, Biome::Heath | Biome::Tundra) && s == mat::ground_cover(col.biome) => {
            (0.035, Rock { main: rock.main, other: rock.main, fleck: mat::MOSSY_STONE })
        }
        _ => return None,
    })
}

/// The four 2x2 quarters of a voxel's footprint, by their low corners.
const QUARTERS: [(i32, i32); 4] = [(0, 0), (2, 0), (0, 2), (2, 2)];

/// Adds whatever stones lie on the column at world position `(x, z)`. `surface` is its
/// top voxel. The stones sit on top of the ground, in the cells above `col.top()`.
pub fn scatter(col: &Column, surface: Voxel, x: i32, z: i32, seed: u32, out: &mut Vec<PlantBox>) {
    let h = hash3(seed ^ 0x7eb1, x, 0, z);
    let Some((share, rock)) = field(col, surface) else { return };
    if unit(h) >= share {
        return;
    }
    let mut rng = Rng::new(h.rotate_left(11));
    let base = [x * CELLS, (col.top() + 1) * CELLS, z * CELLS];
    let mut add = |lx: i32, lz: i32, w: i32, d: i32, height: i32, voxel: Voxel| {
        out.push(PlantBox {
            lo: [base[0] + lx, base[1], base[2] + lz],
            hi: [base[0] + lx + w, base[1] + height, base[2] + lz + d],
            voxel,
            sway_lo: false,
            sway_hi: false,
            thin: 0,
        });
    };
    let pick = |rng: &mut Rng| match rng.unit() {
        r if r < 0.08 => rock.fleck,
        r if r < 0.45 => rock.other,
        _ => rock.main,
    };

    // Now and then a small rock, 0.3 m by 0.2 m and as high as 0.2 m, on its own.
    if rng.unit() < 0.12 {
        let colour = pick(&mut rng);
        add(rng.int(0, 1), rng.int(0, 2), 3, 2, 2, colour);
        return;
    }
    // Otherwise one to three pebbles, each in a quarter of its own, 0.1 to 0.2 m across.
    let mut quarters = QUARTERS;
    for i in 0..3 {
        quarters.swap(i, rng.int(i as i32, 3) as usize);
    }
    let count = 1 + (rng.unit() < 0.5) as usize + (rng.unit() < 0.2) as usize;
    for &(qx, qz) in &quarters[..count] {
        let colour = pick(&mut rng);
        let (w, d) = match rng.int(0, 2) {
            0 => (1, 1),
            1 => (2, 1),
            _ => (2, 2),
        };
        add(qx + rng.int(0, 2 - w), qz + rng.int(0, 2 - d), w, d, 1, colour);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(cover: Cover, biome: Biome) -> Column {
        Column { ground: 40.0, water: None, cover, biome, temperature: 0.5, river: f32::MAX, clear: f32::MAX }
    }

    fn strewn(col: &Column, surface: Voxel, seed: u32) -> Vec<Vec<PlantBox>> {
        let mut all = Vec::new();
        for x in 0..120 {
            for z in 0..120 {
                let mut out = Vec::new();
                scatter(col, surface, x, z, seed, &mut out);
                all.push(out);
            }
        }
        all
    }

    #[test]
    fn stones_stay_in_their_column_and_never_overlap_or_fill_a_voxel() {
        let cases = [
            (column(Cover::RiverBed, Biome::Deciduous), mat::GRAVEL),
            (column(Cover::Land, Biome::Alpine), mat::STONE),
            (column(Cover::Beach, Biome::Ocean), mat::SAND),
            (column(Cover::Land, Biome::Heath), mat::ground_cover(Biome::Heath)),
            (column(Cover::Bank, Biome::Desert), mat::GRAVEL),
        ];
        for (col, surface) in cases {
            let all = strewn(&col, surface, 3);
            assert!(all.iter().filter(|s| !s.is_empty()).count() > 50, "{:?} grew few stones", col.cover);
            for (n, boxes) in all.iter().enumerate() {
                let (x, z) = ((n / 120) as i32, (n % 120) as i32);
                for (i, a) in boxes.iter().enumerate() {
                    assert!((0..3).all(|k| a.lo[k] < a.hi[k]));
                    assert_eq!(a.lo[0].div_euclid(CELLS), x);
                    assert_eq!((a.hi[0] - 1).div_euclid(CELLS), x, "{a:?} leaves its column");
                    assert_eq!(a.lo[2].div_euclid(CELLS), z);
                    assert_eq!((a.hi[2] - 1).div_euclid(CELLS), z, "{a:?} leaves its column");
                    assert!(a.hi[0] - a.lo[0] < CELLS && a.hi[2] - a.lo[2] < CELLS, "{a:?} is a voxel wide");
                    assert_eq!(a.lo[1], 41 * CELLS, "stones lie on the ground");
                    for b in &boxes[i + 1..] {
                        let overlap = (0..3).all(|k| a.lo[k] < b.hi[k] && b.lo[k] < a.hi[k]);
                        assert!(!overlap, "{a:?} overlaps {b:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_column_decides_alone_so_chunk_borders_change_nothing() {
        let col = column(Cover::RiverBed, Biome::Taiga);
        let a = strewn(&col, mat::GRAVEL, 9);
        let b = strewn(&col, mat::GRAVEL, 9);
        for (p, q) in a.iter().zip(&b) {
            assert_eq!(p.len(), q.len());
            assert!(p.iter().zip(q).all(|(s, t)| s.lo == t.lo && s.hi == t.hi && s.voxel == t.voxel));
        }
        // The same world column gives the same stones whichever chunk asks, including
        // across a chunk border and at negative coordinates.
        for (x, z) in [(-33, 64), (31, 32), (-1, -1), (0, 0)] {
            let (mut here, mut there) = (Vec::new(), Vec::new());
            scatter(&col, mat::GRAVEL, x, z, 9, &mut here);
            scatter(&col, mat::GRAVEL, x, z, 9, &mut there);
            assert_eq!(here.len(), there.len());
            assert!(here.iter().zip(&there).all(|(s, t)| s.lo == t.lo && s.hi == t.hi));
        }
    }

    #[test]
    fn they_take_the_local_rock_and_skip_trails_and_turf() {
        let savanna = strewn(&column(Cover::Land, Biome::Savanna), mat::STONE, 1);
        assert!(savanna.iter().flatten().any(|b| b.voxel == mat::SANDSTONE));
        assert!(strewn(&column(Cover::Trail, Biome::Heath), mat::STONE, 1).iter().all(Vec::is_empty));
        assert!(strewn(&column(Cover::Land, Biome::Grassland), mat::GRASS_MEADOW, 1).iter().all(Vec::is_empty));
        let beach = strewn(&column(Cover::Beach, Biome::Ocean), mat::SAND, 2);
        assert!(beach.iter().flatten().any(|b| b.voxel == mat::MUSHROOM_STEM), "shells");
    }
}
