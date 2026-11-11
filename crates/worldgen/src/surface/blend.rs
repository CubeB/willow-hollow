//! Which biome a voxel column is in. Biomes are chosen per cell of the simulation grid
//! (25.6 m), so taken straight from the grid their borders would be rows of squares. The
//! lookup position is warped instead, by noise at three scales and a little per-column
//! dither, so borders wander, interlock and fray into each other over a few metres.
//! Everything here depends on the world seed and position alone, so the columns either
//! side of a chunk border always agree.

use crate::{
    biome::Biome,
    continent::Continent,
    macro_layout::MACRO_SIZE,
    noise::{hash2, perlin2, unit},
    scale::m64,
};

/// How far the lookup is pushed, in cells, for one axis (`salt` tells the axes apart).
fn warp(seed: u32, salt: u32, wx: f64, wz: f64, lx: i32, lz: i32) -> f32 {
    perlin2(seed ^ salt, wx / m64(80.0), wz / m64(80.0)) * 1.0
        + perlin2(seed ^ salt ^ 0x51, wx / m64(24.0), wz / m64(24.0)) * 0.45
        + perlin2(seed ^ salt ^ 0x52, wx / m64(7.0), wz / m64(7.0)) * 0.18
        + (unit(hash2(seed ^ salt, lx, lz)) - 0.5) * 0.3
}

/// The biome at continent-local voxel column `(lx, lz)`.
pub fn biome_at(c: &Continent, seed: u32, lx: i32, lz: i32) -> Biome {
    let cs = c.cell_size();
    let wx = (c.desc.mx as i64 * MACRO_SIZE as i64 + lx as i64) as f64;
    let wz = (c.desc.mz as i64 * MACRO_SIZE as i64 + lz as i64) as f64;
    let (px, pz) = (lx as f32 + 0.5, lz as f32 + 0.5);
    let cell = |v: f32| (v as i32).clamp(0, c.size as i32 - 1) as usize;
    let (bx, bz) = (cell(px / cs + warp(seed, 0xb1, wx, wz, lx, lz)), cell(pz / cs + warp(seed, 0xb2, wx, wz, lx, lz)));
    let warped = Biome::from_u8(c.biome[bz * c.size + bx]);
    if warped != Biome::Ocean {
        return warped;
    }
    // Land never takes the sea's biome from a cell offshore: it keeps its own cell's.
    Biome::from_u8(c.biome[cell(pz / cs) * c.size + cell(px / cs)])
}
