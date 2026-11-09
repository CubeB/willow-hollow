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
fn land_never_reaches_the_cell_border() {
    for c in continents() {
        let margin = c.size / 32;
        for z in 0..c.size {
            for x in 0..c.size {
                let edge = x < margin || z < margin || x >= c.size - margin || z >= c.size - margin;
                if edge {
                    assert!(c.ocean[z * c.size + x], "land at ({x}, {z}) in {:?}", c.desc);
                }
            }
        }
    }
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
