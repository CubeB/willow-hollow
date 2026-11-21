//! World generation. Pure CPU: everything here is a deterministic function of the seed.

pub mod biome;
pub mod continent;
pub mod fauna;
pub mod features;
pub mod macro_layout;
pub mod materials;
pub mod names;
pub mod noise;
pub mod scale;
pub mod settlements;
pub mod structures;
pub mod surface;
pub mod trails;
pub mod world;

use glam::Vec3;
use voxel::{
    Materials, PaddedChunk,
    mesh::{Current, PlantBox},
};

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
    /// How fast the river runs over the lip, metres per second (0 for still water).
    pub speed: f32,
    /// The river's [`Regime`](continent::Regime) as u8, which says how much spray it
    /// throws in spring and in late summer.
    pub regime: u8,
}

/// Where a stream wells up out of the ground into a small pool. Bubbles come from here.
#[derive(Clone, Copy, Debug)]
pub struct SpringMark {
    /// World position of the centre of the pool's water surface.
    pub pos: Vec3,
    /// Radius of the pool, in voxels.
    pub radius: f32,
}

/// Where sea water breaks against rock: a shelf, boulders or a sea cliff. Surf spray
/// is thrown up here.
#[derive(Clone, Copy, Debug)]
pub struct Surf {
    /// World position of the sea surface at the foot of the rock.
    pub pos: Vec3,
    /// Horizontal direction from the water towards the rock (unit length).
    pub dir: Vec3,
}

/// Everything generated for one chunk column.
pub struct ColumnOutput {
    /// `(chunk y, voxels)`, lowest first with none skipped.
    pub chunks: Vec<(i32, PaddedChunk)>,
    pub cascades: Vec<Cascade>,
    /// Springs whose pool centre is in this chunk column.
    pub springs: Vec<SpringMark>,
    /// A few places in this chunk column where the sea breaks on rock.
    pub surf: Vec<Surf>,
    /// The current in each column, indexed `x * PAD + z` in padded coordinates like the
    /// chunks' voxels, or empty if no water here runs.
    pub currents: Vec<Current>,
    /// Small plants standing on the columns inside this chunk column.
    pub plants: Vec<PlantBox>,
    /// For each padded column (`x * PAD + z`), the height of its ground and the colour the
    /// plant growing there gives it from afar (see [`features::plants::wash_at`]; 0 for none),
    /// and how the grass grows there (its [`surface::sward::code`]).
    pub wash: Vec<(i32, u8, u8)>,
    /// The trees and other features reaching into this column, in the order they were
    /// stamped: what [`features::leaf_tag`] reads to tell whose leaf is whose.
    pub trees: Vec<features::Placement>,
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
