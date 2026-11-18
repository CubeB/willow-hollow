//! What stage 3 (#203) adds about a town's buildings rather than among them, as the
//! layout records it: the gates on its main ways in, the old wall of an ancient town,
//! its fishers' boat sheds and a coastal great town's harbour wall, and a great town's
//! park. None of it counts among the settlement's buildings.
//!
//! Each is planned in two steps from `Town::build`: its ground is kept before any plot
//! is chosen, so no building is put through it, and it is built once every plot and path
//! is down, so it is built round them. See `gates`, `port` and `park`.

use glam::{IVec2, IVec3, Vec2};

use crate::{features::designs::Special, settlements::special::Built};

/// A gate on a way into a town, or a gatehouse in an ancient town's old wall.
#[derive(Clone, Debug)]
pub struct Gate {
    /// [`Special::Gate`] or [`Special::Gatehouse`].
    pub which: Special,
    /// Where its model origin stands, continent-local: the middle of its way through, on
    /// the first layer above the highest of the road under it.
    pub origin: IVec3,
    /// The quarter turns that took its +X, the way out of the town, to `out`.
    pub turns: u32,
    pub out: IVec2,
    /// How wide its way through is, in voxels.
    pub passage: i32,
    /// Every column it covers, its roof's eaves among them, continent-local, inclusive.
    pub lo: IVec2,
    pub hi: IVec2,
    /// The way it stands on, by index in [`super::Layout::lanes`]: always a way that
    /// arrives, and the main one but in a great town.
    pub lane: usize,
}

/// A fishers' boat shed, which is a walk-in building among the plots: where its back,
/// open to the water, meets it.
#[derive(Clone, Copy, Debug)]
pub struct Shed {
    /// Its plot's model origin, continent-local: as [`super::Plot::origin`].
    pub origin: IVec3,
    /// The foot of its slipway at the water's edge, continent-local, and the way out
    /// down it onto the water.
    pub slip: IVec3,
    pub out: IVec2,
}

/// A coastal great town's harbour wall, curving out from the shore to shelter its
/// landing, with a beacon at its head.
#[derive(Clone, Debug)]
pub struct Harbour {
    /// The columns of its line from the shore out to its head, continent-local.
    pub line: Vec<IVec2>,
    /// The layer walked on along its top.
    pub top: i32,
    /// Where it meets the shore: the first column of its line, on its top. A quay along
    /// the shore would meet the harbour wall here.
    pub root: IVec3,
    /// The foot of the beacon on its head.
    pub beacon: IVec3,
    /// The landing it shelters, by index in `Network::sites`: the same landing the
    /// town's fishing quay is (`quay::Quay::landing`), so the port has the one.
    pub landing: usize,
    /// The stone quay along the shore from beside the landing to the wall's root, which
    /// the wall is walked out to along: the columns of its deck, continent-local, in
    /// order from the landing. Empty where the shore left no room for one.
    pub quay: Vec<IVec2>,
}

/// A great town's park, kept round its old tree.
#[derive(Clone, Debug)]
pub struct Park {
    pub at: Vec2,
    pub radius: f32,
    /// The pond's middle at its water's surface, and its radius.
    pub pond: Option<(IVec3, i32)>,
    /// The pavilion's model origin, and its turns.
    pub pavilion: Option<(IVec3, u32)>,
    /// The games lawn, continent-local columns inclusive.
    pub lawn: Option<(IVec2, IVec2)>,
    /// Where each bench stands, continent-local.
    pub benches: Vec<IVec3>,
}

/// What a town's edges are planned with between keeping their ground and building them.
#[derive(Default)]
pub(super) struct Edges {
    /// Each gate kept for, built already in its model space, and its record.
    pub gates: Vec<(Built, Gate)>,
    /// The seed of the fishers' workplace want built as a boat shed, if one is.
    pub shed: Option<u32>,
    /// The park kept for, its middle and radius.
    pub park: Option<(Vec2, f32)>,
}
