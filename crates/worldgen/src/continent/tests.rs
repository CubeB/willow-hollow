use super::*;
use crate::macro_layout::{CellKind, cell_desc};

/// Small grids keep the tests quick; the algorithms are resolution-independent.
const SIZE: usize = 256;

fn continents() -> Vec<Continent> {
    // A spread of cells from two seeds, forcing each kind to appear at least once.
    let mut out = Vec::new();
    let mut kinds = std::collections::HashSet::new();
    for seed in [1u32, 77] {
        for mx in -2..=2 {
            for mz in -1..=1 {
                let desc = cell_desc(seed, mx, mz);
                if kinds.insert(desc.kind) || (mx, mz) == (0, 0) {
                    out.push(generate(desc, SimConfig { size: SIZE }));
                }
            }
        }
    }
    assert!(kinds.contains(&CellKind::Continent) && kinds.contains(&CellKind::Islands));
    out
}

#[test]
fn generation_is_deterministic() {
    let desc = cell_desc(5, 0, 0);
    let a = generate(desc, SimConfig { size: SIZE });
    let b = generate(desc, SimConfig { size: SIZE });
    assert!(a.height == b.height, "heights differ between runs");
    assert!(a.rcv == b.rcv && a.discharge == b.discharge && a.lake == b.lake);
    assert!(a.biome == b.biome);
}

#[test]
fn origin_cell_is_always_a_continent_with_land() {
    for seed in [0u32, 1, 2, 99, 12345] {
        let desc = cell_desc(seed, 0, 0);
        assert_eq!(desc.kind, CellKind::Continent);
        let c = generate(desc, SimConfig { size: 128 });
        let land = c.ocean.iter().filter(|&&o| !o).count();
        assert!(land > 128 * 128 / 10, "seed {seed}: only {land} land cells");
    }
}

#[test]
fn land_reaches_the_cell_border_only_along_a_range() {
    for c in continents() {
        let margin = c.size / 32;
        let field = crate::structures::Field::new(c.desc.world_seed, c.desc.mx, c.desc.mz);
        let ranges = field.structures().iter().any(|s| s.kind == crate::structures::Kind::Range);
        for z in 0..c.size {
            for x in 0..c.size {
                let edge = x < margin || z < margin || x >= c.size - margin || z >= c.size - margin;
                if edge && !ranges {
                    assert!(c.ocean[z * c.size + x], "land at ({x}, {z}) in {:?}", c.desc);
                }
            }
        }
    }
}

/// A macro-cell pair sharing a border that a range crosses, found by scanning seeds.
/// Returns `(seed, mx, mz)` where the range crosses the +X edge of the cell.
fn cells_sharing_a_range() -> (u32, i32, i32) {
    for seed in 0u32..64 {
        for mz in -6..6 {
            for mx in -6..6 {
                let field = crate::structures::Field::new(seed, mx, mz);
                let crossed = (1..16).any(|k| {
                    let (px, pz) = (1.0, -1.0 + k as f32 / 8.0);
                    field.land(px, pz).is_some_and(|l| l > 0.1)
                });
                if crossed {
                    return (seed, mx, mz);
                }
            }
        }
    }
    panic!("no border-crossing range in the seeds scanned");
}

#[test]
fn a_range_crossing_a_border_matches_on_both_sides() {
    let (seed, mx, mz) = cells_sharing_a_range();
    let a = generate(cell_desc(seed, mx, mz), SimConfig { size: SIZE });
    let b = generate(cell_desc(seed, mx + 1, mz), SimConfig { size: SIZE });
    let mut land = 0;
    for z in 0..SIZE {
        let ia = z * SIZE + SIZE - 1;
        let ib = z * SIZE;
        assert_eq!(a.ocean[ia], b.ocean[ib], "land and sea disagree at border row {z}");
        assert_eq!(a.height[ia], b.height[ib], "height differs at border row {z}");
        if !a.ocean[ia] {
            land += 1;
            assert_eq!(a.rain[ia], b.rain[ib], "rain differs at border row {z}");
            assert_eq!(a.biome[ia], b.biome[ib], "biome differs at border row {z}");
            // No standing water beside the border, where it would face the neighbour's air.
            for k in 1..4 {
                assert_eq!(a.lake[z * SIZE + SIZE - 1 - k], NO_LAKE, "lake beside the border at row {z}");
                assert_eq!(b.lake[z * SIZE + k], NO_LAKE, "lake beside the border at row {z}");
            }
        }
    }
    assert!(land > 0, "scan found a crossing but no land reached the border");
}

#[test]
fn no_water_leaves_across_a_border() {
    // Whatever land there is at the border, rivers must end inside the cell.
    let (seed, mx, mz) = cells_sharing_a_range();
    let c = generate(cell_desc(seed, mx, mz), SimConfig { size: SIZE });
    let ring = |i: usize| i % SIZE == 0 || i / SIZE == 0 || i % SIZE == SIZE - 1 || i / SIZE == SIZE - 1;
    for i in 0..SIZE * SIZE {
        if !c.ocean[i] {
            // The ring may still be sea beside a crossing, and that sea is shared.
            let r = c.rcv[i] as usize;
            assert!(!ring(r) || c.ocean[r], "cell {i} drains onto land at the border ring");
        }
    }
}

#[test]
fn structures_are_decided_by_the_seed_alone() {
    let a = crate::structures::describe(99, 3, -4);
    let b = crate::structures::describe(99, 3, -4);
    assert_eq!(a, b);
    let mut differ = false;
    for mx in 0..30 {
        if crate::structures::describe(99, mx, 0) != crate::structures::describe(100, mx, 0) {
            differ = true;
        }
    }
    assert!(differ, "different world seeds gave the same structures");
}

#[test]
fn vast_oceans_exist_but_spare_the_origin() {
    let mut oceans = 0u32;
    for seed in 0..8u32 {
        assert!(!crate::structures::in_vast_ocean(seed, 0, 0));
        for z in -60..60 {
            for x in -60..60 {
                if crate::structures::in_vast_ocean(seed, x, z) {
                    assert_eq!(cell_desc(seed, x, z).kind, CellKind::Ocean);
                    oceans += 1;
                }
            }
        }
    }
    assert!(oceans > 0, "no vast ocean anywhere in eight seeds");
}

#[test]
fn every_land_cell_drains_downhill_to_the_sea() {
    for c in continents() {
        let n = c.size * c.size;
        for start in 0..n {
            if c.ocean[start] {
                assert_eq!(c.rcv[start] as usize, start);
                continue;
            }
            let r = c.rcv[start] as usize;
            assert_ne!(r, start, "land cell {start} drains nowhere");
            let (dx, dz) = (
                (r % c.size) as i32 - (start % c.size) as i32,
                (r / c.size) as i32 - (start / c.size) as i32,
            );
            assert!(dx.abs() <= 1 && dz.abs() <= 1, "receiver is not a neighbour");
            assert!(
                c.water_level[r] <= c.water_level[start],
                "water would flow uphill from {start} to {r}"
            );
            assert!(c.water_level[start] > 0.0);
            // Water is at the ground or above it, bar a sliver at lake outlets.
            assert!(c.water_level[start] >= c.height[start] - 0.3);
        }
        // And walk a sample of full paths to the sea as a direct check.
        for start in (0..n).step_by(97).filter(|&i| !c.ocean[i]) {
            let (mut i, mut steps) = (start, 0);
            while !c.ocean[i] {
                i = c.rcv[i] as usize;
                steps += 1;
                assert!(steps <= n, "cycle in drainage from {start}");
            }
        }
    }
}

#[test]
fn rivers_only_grow_downstream() {
    for c in continents() {
        for i in (0..c.size * c.size).filter(|&i| !c.ocean[i]) {
            assert!(c.discharge[i] > 0.0);
            let r = c.rcv[i] as usize;
            if !c.ocean[r] {
                assert!(c.discharge[r] > c.discharge[i], "river shrinks at {i}");
            }
        }
    }
}

#[test]
fn lakes_are_flat_and_hold_water() {
    let mut total = 0;
    for c in continents() {
        for i in 0..c.size * c.size {
            if c.lake[i] == NO_LAKE {
                continue;
            }
            total += 1;
            assert!(!c.ocean[i]);
            assert!(c.lake[i] > c.height[i], "lake surface below its bed at {i}");
            // Neighbouring cells of the same lake share one surface level.
            for d in [1usize, c.size] {
                let j = i + d;
                if c.lake[j] != NO_LAKE {
                    assert_eq!(c.lake[i], c.lake[j], "lake surface is not flat at {i}");
                }
            }
        }
    }
    assert!(total > 0, "no lakes generated in any test continent");
}

#[test]
fn climate_is_sane() {
    for c in continents() {
        for i in (0..c.size * c.size).filter(|&i| !c.ocean[i]) {
            assert!((0.05..=3.6).contains(&c.rain[i]), "rain {} at {i}", c.rain[i]);
            assert!(c.temperature[i].is_finite());
            assert_ne!(Biome::from_u8(c.biome[i]), Biome::Ocean);
        }
    }
}
