//! How big a voxel is. Everything that is really a size in the world (how tall the
//! player is, how far the fog reaches, how high the clouds float) is written in metres
//! and converted here, so the voxel can be resized by changing one number.

/// Voxels in one metre.
pub const VOXELS_PER_METRE: f32 = 2.5;

/// Voxels in the 0.8 m unit that particle and vehicle models were tuned in. They keep
/// their hand-tuned numbers and multiply by this where they meet the world.
pub const OLD_VOXEL: f32 = VOXELS_PER_METRE * 0.8;

/// Whole voxels along the edge of one 0.8 m block, the unit models are laid out in and
/// water levels step by.
pub const BLOCK: i32 = OLD_VOXEL as i32;

/// Metres to voxels.
pub const fn m(metres: f32) -> f32 {
    metres * VOXELS_PER_METRE
}

/// Metres to voxels, for positions and other double-precision values.
pub const fn m64(metres: f64) -> f64 {
    metres * VOXELS_PER_METRE as f64
}
