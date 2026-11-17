//! Furnishing a home or a workplace: what stands inside it (stage 2, #202).
//!
//! The village layout builds each building from the house kit and then asks here what
//! goes in it; what is set outside, in yards and lanes, is the layout's own business.

use glam::IVec3;
use voxel::{Voxel, mesh::PlantBox};

use super::{Livelihood, Region, kit::Building};
use crate::features::landmarks::Palette;

/// Who lives in a building and what is done there.
#[derive(Clone, Debug)]
pub struct Household {
    pub region: Region,
    /// How many sleep there, children among them.
    pub sleepers: u8,
    pub children: u8,
    /// The trade worked there, if it is a workplace as well as (or instead of) a home.
    pub trade: Option<Livelihood>,
    /// 0 poor to 1 well off: how much is in it, and how fine.
    pub wealth: f32,
    /// 0 well kept to 1 run down: how much of it is worn or broken.
    pub neglect: f32,
    /// Whether anyone lives there now. An empty home keeps a little of what was left.
    pub lived_in: bool,
}

/// What is set in a building, in the building's own model space as the kit built it
/// (before it is turned), ready to stamp with it.
#[derive(Clone, Debug, Default)]
pub struct Furnished {
    pub voxels: Vec<(IVec3, Voxel)>,
    pub fine: Vec<PlantBox>,
}

/// Furnishes `b` for the household `h`.
pub fn furnish(_b: &Building, _h: &Household, _pal: &Palette, _seed: u32) -> Furnished {
    Furnished::default()
}
