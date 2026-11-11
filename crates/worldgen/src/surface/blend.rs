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

/// How far the lookup is pushed, in cells, for one axis (`salt` tells the axes apart),
/// before the per-column dither.
fn warp(seed: u32, salt: u32, wx: f64, wz: f64) -> f32 {
    perlin2(seed ^ salt, wx / m64(80.0), wz / m64(80.0)) * 1.0
        + perlin2(seed ^ salt ^ 0x51, wx / m64(24.0), wz / m64(24.0)) * 0.45
        + perlin2(seed ^ salt ^ 0x52, wx / m64(7.0), wz / m64(7.0)) * 0.18
}

/// Where a voxel column looks its biome up.
#[derive(Clone, Copy)]
pub struct Lookup {
    /// The column, continent-local.
    at: (i32, i32),
    /// Where it looks, in cells, before the per-column dither.
    smooth: (f32, f32),
    /// The dither, in cells.
    dither: (f32, f32),
}

/// Where continent-local voxel column `(lx, lz)` looks its biome up.
pub fn lookup(c: &Continent, seed: u32, lx: i32, lz: i32) -> Lookup {
    let cs = c.cell_size();
    let wx = (c.desc.mx as i64 * MACRO_SIZE as i64 + lx as i64) as f64;
    let wz = (c.desc.mz as i64 * MACRO_SIZE as i64 + lz as i64) as f64;
    let (px, pz) = (lx as f32 + 0.5, lz as f32 + 0.5);
    let dither = |salt: u32| (unit(hash2(seed ^ salt, lx, lz)) - 0.5) * 0.3;
    Lookup {
        at: (lx, lz),
        smooth: (px / cs + warp(seed, 0xb1, wx, wz), pz / cs + warp(seed, 0xb2, wx, wz)),
        dither: (dither(0xb1), dither(0xb2)),
    }
}

/// The cell a position in cells lies in, along one axis.
fn cell(c: &Continent, v: f32) -> usize {
    (v as i32).clamp(0, c.size as i32 - 1) as usize
}

/// The biome of the land cell whose centre is nearest `(qx, qz)` (in cells), looking a
/// couple of cells each way. The border between two such biomes is as far from the
/// centres of both, so it follows the lines of the grid no more than `(qx, qz)` does.
fn nearest_land(c: &Continent, qx: f32, qz: f32) -> Option<Biome> {
    let n = c.size as i32;
    let (ix, iz) = (cell(c, qx) as i32, cell(c, qz) as i32);
    let mut best: Option<(f32, Biome)> = None;
    for z in (iz - 2).max(0)..=(iz + 2).min(n - 1) {
        for x in (ix - 2).max(0)..=(ix + 2).min(n - 1) {
            let biome = Biome::from_u8(c.biome[(z * n + x) as usize]);
            let d = (x as f32 + 0.5 - qx).powi(2) + (z as f32 + 0.5 - qz).powi(2);
            if biome != Biome::Ocean && best.is_none_or(|(least, _)| d < least) {
                best = Some((d, biome));
            }
        }
    }
    best.map(|(_, biome)| biome)
}

impl Lookup {
    /// The column's biome.
    pub fn biome(&self, c: &Continent) -> Biome {
        let cs = c.cell_size();
        let (qx, qz) = (self.smooth.0 + self.dither.0, self.smooth.1 + self.dither.1);
        let warped = Biome::from_u8(c.biome[cell(c, qz) * c.size + cell(c, qx)]);
        if warped != Biome::Ocean {
            return warped;
        }
        // Land never takes the sea's biome from a cell offshore: it takes that of the
        // land nearest where it looked. (Its own cell's would do, but then along the
        // coast the borders between biomes would run straight down the sides of the
        // grid's squares.)
        let (lx, lz) = self.at;
        let own = Biome::from_u8(c.biome[cell(c, (lz as f32 + 0.5) / cs) * c.size + cell(c, (lx as f32 + 0.5) / cs)]);
        if own == Biome::Ocean { own } else { nearest_land(c, qx, qz).unwrap_or(own) }
    }

    /// The biome of the land at the column or, out on the sea's side of the grid's
    /// coastline, of the nearest land: the country a shore belongs to. [`Biome::Ocean`]
    /// only where no land is near. Its borders wander as [`Lookup::biome`]'s do but are
    /// not dithered column by column, so what a shore is made of changes along a line
    /// and not through a band of speckle.
    pub fn land_biome(&self, c: &Continent) -> Biome {
        let (qx, qz) = self.smooth;
        match Biome::from_u8(c.biome[cell(c, qz) * c.size + cell(c, qx)]) {
            Biome::Ocean => nearest_land(c, qx, qz).unwrap_or(Biome::Ocean),
            land => land,
        }
    }
}

/// The biome at continent-local voxel column `(lx, lz)`.
pub fn biome_at(c: &Continent, seed: u32, lx: i32, lz: i32) -> Biome {
    lookup(c, seed, lx, lz).biome(c)
}
