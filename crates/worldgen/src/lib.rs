//! World generation. Pure CPU: everything here is a deterministic function of the seed.

pub mod biome;
pub mod continent;
pub mod macro_layout;
pub mod materials;
pub mod noise;
pub mod placeholder;

use voxel::{Materials, PaddedChunk};

/// A source of voxel terrain, generated one chunk column at a time.
///
/// Columns are the unit of work because the expensive part of generation (surface
/// height, rivers, biomes) is 2D and shared by every chunk stacked in the column.
pub trait World: Send + Sync {
    fn materials(&self) -> &Materials;

    /// All chunks in column `(cx, cz)` that can contain a visible face, as
    /// `(chunk y, voxels)`. Chunks that are entirely air or entirely buried are omitted.
    fn generate_column(&self, cx: i32, cz: i32) -> Vec<(i32, PaddedChunk)>;
}
