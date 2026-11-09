//! World generation. Pure CPU: everything here is a deterministic function of the seed.

pub mod biome;
pub mod continent;
pub mod features;
pub mod macro_layout;
pub mod materials;
pub mod noise;
pub mod surface;
pub mod world;

use voxel::{Materials, PaddedChunk};

/// A source of voxel terrain, generated one chunk column at a time.
///
/// Columns are the unit of work because the expensive part of generation (surface
/// height, rivers, biomes) is 2D and shared by every chunk stacked in the column.
pub trait World: Send + Sync {
    fn materials(&self) -> &Materials;

    /// The chunks of column `(cx, cz)` as `(chunk y, voxels)`, lowest first: every
    /// chunk from just under the lowest ground to the top of the highest thing in the
    /// column, with none skipped. Below that range is solid rock and above it is air.
    fn generate_column(&self, cx: i32, cz: i32) -> Vec<(i32, PaddedChunk)>;
}

#[cfg(test)]
mod world_tests;
