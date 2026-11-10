//! World generation. Pure CPU: everything here is a deterministic function of the seed.

pub mod biome;
pub mod continent;
pub mod features;
pub mod macro_layout;
pub mod materials;
pub mod names;
pub mod noise;
pub mod scale;
pub mod structures;
pub mod surface;
pub mod trails;
pub mod world;

use glam::Vec3;
use voxel::{Materials, PaddedChunk, mesh::PlantBox};

/// A place where flowing water drops to a lower level: the lip of a fall or the step
/// of a cascade. Spray and sound come from here.
#[derive(Clone, Copy, Debug)]
pub struct Cascade {
    /// World position of the water surface at the lip.
    pub pos: Vec3,
    /// Horizontal direction the water falls toward (unit length).
    pub dir: Vec3,
    /// Height of the drop in voxels.
    pub drop: f32,
}

/// Everything generated for one chunk column.
pub struct ColumnOutput {
    /// `(chunk y, voxels)`, lowest first with none skipped.
    pub chunks: Vec<(i32, PaddedChunk)>,
    pub cascades: Vec<Cascade>,
    /// Small plants standing on the columns inside this chunk column.
    pub plants: Vec<PlantBox>,
}

/// A source of voxel terrain, generated one chunk column at a time.
///
/// Columns are the unit of work because the expensive part of generation (surface
/// height, rivers, biomes) is 2D and shared by every chunk stacked in the column.
pub trait World: Send + Sync {
    fn materials(&self) -> &Materials;

    /// The chunks of column `(cx, cz)`: every chunk from just under the lowest ground
    /// to the top of the highest thing in the column, with none skipped. Below that
    /// range is solid rock and above it is air.
    fn generate_column(&self, cx: i32, cz: i32) -> ColumnOutput;
}

#[cfg(test)]
mod world_tests;
