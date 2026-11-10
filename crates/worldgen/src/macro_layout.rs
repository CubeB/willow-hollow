//! The endless world at the largest scale: an infinite grid of macro-cells, each
//! holding one independent landmass (or open sea).
//!
//! Ordinary land never touches a cell border, so every landmass is a bounded problem
//! that can be simulated on its own, and cells can be generated lazily in any order.
//! The rare exceptions, mountain ranges carried across a border by the structure field
//! ([`crate::structures`]), are held to a height both cells share along the border.
//!
//! Each cell's land also has an [`Archetype`], so continents differ in outline, coast
//! and relief rather than all being alike.

use crate::{
    noise::{hash2, unit},
    scale::OLD_VOXEL,
    structures::in_vast_ocean,
};

/// Macro-cell edge length in simulation units (0.8 m each).
pub const SIM_SPAN: f32 = 32_768.0;
/// Macro-cell edge length in voxels.
pub const MACRO_SIZE: i32 = (SIM_SPAN * OLD_VOXEL) as i32;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CellKind {
    /// One large landmass filling most of the cell.
    Continent,
    /// A scattering of smaller islands.
    Islands,
    /// Open sea.
    Ocean,
}

/// The character of a cell's land: its outline, coast and how it is raised.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Archetype {
    /// A broad rounded landmass with ranges scattered through it.
    Massif,
    /// A long landmass with a mountain spine down its length.
    Elongated,
    /// A core with several long peninsulas, and deep bays between them.
    Lobed,
    /// A high, flat-topped tableland ending in cliffs.
    Tableland,
    /// Low gentle country of broad basins, slow rivers and marsh.
    Lowland,
    /// High, ragged country with an intricate coast of inlets.
    Rugged,
    /// A loose scattering of islands.
    Archipelago,
    /// A few volcanic cones standing out of the sea.
    Volcanic,
    /// Islands strung along an arc.
    Chain,
}

/// Everything that distinguishes one macro-cell from another, derived from the seed.
#[derive(Clone, Copy, Debug)]
pub struct CellDesc {
    pub mx: i32,
    pub mz: i32,
    /// Seed of the world, for what is shared between cells (see [`crate::structures`]).
    pub world_seed: u32,
    /// Seed for all of this cell's noise.
    pub seed: u32,
    pub kind: CellKind,
    pub archetype: Archetype,
    /// Elongation of the landmass (1 = round) and the direction of its long axis.
    pub aspect: f32,
    pub angle: f32,
    /// Multiplier on the size of bays and headlands.
    pub coast: f32,
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
        _ if in_vast_ocean(world_seed, mx, mz) => CellKind::Ocean,
        k if k < 0.55 => CellKind::Continent,
        k if k < 0.80 => CellKind::Islands,
        _ => CellKind::Ocean,
    };
    // The origin keeps the plain massif every world has always started on.
    let a = r(8);
    let archetype = match kind {
        _ if origin => Archetype::Massif,
        CellKind::Continent if a < 0.28 => Archetype::Massif,
        CellKind::Continent if a < 0.46 => Archetype::Elongated,
        CellKind::Continent if a < 0.64 => Archetype::Lobed,
        CellKind::Continent if a < 0.76 => Archetype::Tableland,
        CellKind::Continent if a < 0.88 => Archetype::Lowland,
        CellKind::Continent => Archetype::Rugged,
        CellKind::Islands if a < 0.45 => Archetype::Archipelago,
        CellKind::Islands if a < 0.72 => Archetype::Volcanic,
        CellKind::Islands => Archetype::Chain,
        CellKind::Ocean => Archetype::Archipelago,
    };
    let base_rugged = 0.25 + r(4) * 0.75;
    let ruggedness = match archetype {
        _ if origin => 0.75,
        Archetype::Rugged => 0.85 + r(4) * 0.15,
        Archetype::Lowland => 0.1 + r(4) * 0.2,
        Archetype::Tableland => 0.3 + r(4) * 0.25,
        Archetype::Elongated => 0.6 + r(4) * 0.4,
        _ => base_rugged,
    };
    let (aspect, radius) = match archetype {
        Archetype::Elongated => (1.8 + r(9) * 0.9, 0.5 + r(3) * 0.1),
        Archetype::Lobed => (1.0 + r(9) * 0.3, 0.36 + r(3) * 0.1),
        Archetype::Lowland => (1.0 + r(9) * 0.4, 0.6 + r(3) * 0.12),
        Archetype::Tableland => (1.0 + r(9) * 0.5, 0.48 + r(3) * 0.14),
        Archetype::Rugged => (1.0 + r(9) * 0.6, 0.54 + r(3) * 0.14),
        _ => (1.0, 0.56 + r(3) * 0.14),
    };
    // The long axis must still end in sea well inside the cell.
    let radius = radius.min(0.68 / aspect.sqrt());
    let coast = match archetype {
        Archetype::Rugged => 1.7,
        Archetype::Lobed => 1.3,
        Archetype::Lowland => 0.9,
        Archetype::Tableland | Archetype::Volcanic => 0.6,
        _ => 1.0,
    };
    CellDesc {
        mx,
        mz,
        world_seed,
        seed,
        kind,
        archetype,
        aspect,
        angle: r(10) * std::f32::consts::PI,
        coast,
        center: ((r(1) - 0.5) * 0.24, (r(2) - 0.5) * 0.24),
        radius,
        ruggedness,
        temperature: if origin { 0.52 } else { 0.12 + r(5) * 0.80 },
        wetness: if origin { 1.0 } else { 0.55 + r(6) * 0.9 },
        wind: r(7) * std::f32::consts::TAU,
    }
}

/// Macro-cell containing a world voxel coordinate.
pub fn cell_of(x: i32, z: i32) -> (i32, i32) {
    (x.div_euclid(MACRO_SIZE), z.div_euclid(MACRO_SIZE))
}
