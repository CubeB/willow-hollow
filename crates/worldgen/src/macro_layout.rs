//! The endless world at the largest scale: an infinite grid of macro-cells, each
//! holding one independent landmass (or open sea).
//!
//! Land never touches a cell border, so every landmass is a bounded problem that can be
//! simulated on its own, and cells can be generated lazily in any order.

use crate::noise::{hash2, unit};

/// Macro-cell edge length in voxels.
pub const MACRO_SIZE: i32 = 32_768;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CellKind {
    /// One large landmass filling most of the cell.
    Continent,
    /// A scattering of smaller islands.
    Islands,
    /// Open sea.
    Ocean,
}

/// Everything that distinguishes one macro-cell from another, derived from the seed.
#[derive(Clone, Copy, Debug)]
pub struct CellDesc {
    pub mx: i32,
    pub mz: i32,
    /// Seed for all of this cell's noise.
    pub seed: u32,
    pub kind: CellKind,
    /// Landmass centre in cell-normalised coordinates (-1..1).
    pub center: (f32, f32),
    /// Landmass radius in the same units.
    pub radius: f32,
    /// 0 = gentle rolling country, 1 = alpine.
    pub ruggedness: f32,
    /// Sea-level temperature at the cell centre: 0 = polar, 1 = tropical.
    pub temperature: f32,
    /// Overall rainfall multiplier.
    pub wetness: f32,
    /// Direction the prevailing wind blows *toward*, radians.
    pub wind: f32,
}

pub fn cell_desc(world_seed: u32, mx: i32, mz: i32) -> CellDesc {
    let seed = hash2(world_seed, mx, mz);
    let r = |k: u32| unit(hash2(seed, k as i32, 0x5eed));
    // The origin cell is always a temperate continent so every world starts on land.
    let origin = mx == 0 && mz == 0;
    let kind = match r(0) {
        _ if origin => CellKind::Continent,
        k if k < 0.55 => CellKind::Continent,
        k if k < 0.80 => CellKind::Islands,
        _ => CellKind::Ocean,
    };
    CellDesc {
        mx,
        mz,
        seed,
        kind,
        center: ((r(1) - 0.5) * 0.24, (r(2) - 0.5) * 0.24),
        radius: 0.56 + r(3) * 0.14,
        ruggedness: if origin { 0.75 } else { 0.25 + r(4) * 0.75 },
        temperature: if origin { 0.52 } else { 0.12 + r(5) * 0.80 },
        wetness: if origin { 1.0 } else { 0.55 + r(6) * 0.9 },
        wind: r(7) * std::f32::consts::TAU,
    }
}

/// Macro-cell containing a world voxel coordinate.
pub fn cell_of(x: i32, z: i32) -> (i32, i32) {
    (x.div_euclid(MACRO_SIZE), z.div_euclid(MACRO_SIZE))
}
