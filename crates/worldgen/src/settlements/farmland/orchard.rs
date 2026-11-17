//! The trees of the worked land (#64): which of the wild's trees still stand where the
//! land is farmed, and the fruit and olive trees planted in rows in an orchard.
//!
//! Of the wild's trees only where each stands changes here; what each is like is the
//! tree models' business. The fields have been cleared of trees, but a hedgerow keeps its
//! oaks and ashes, and now and then a pasture keeps one big old tree in the middle of it,
//! for the shade. Out in the rough the trees are thinned and cut back to coppice stools,
//! and at the edge of the wild the woods come back gradually. About a young settlement
//! the stumps of the trees felled to make its fields are still to be seen.
//!
//! An orchard's trees are its own: small, pruned low and planted in rows, apple trees
//! white with blossom in spring and red with fruit in September, or grey-green olives in
//! warm country. They stand on a grid laid over the whole continent, where it falls in an
//! orchard, so every chunk a crown reaches into knows the tree it belongs to.

use glam::IVec3;
use voxel::{
    Voxel,
    mesh::{CELLS, PlantBox},
};

use super::{
    fields::{Crop, plot, worked},
    influence::{CROPS, EDGE, PASTURE, ROUGH},
};
use crate::{
    biome::Biome,
    continent::Continent,
    features::trees::Species,
    materials as mat,
    noise::{hash3, unit},
    scale::m,
    settlements::Age,
    surface::{Column, Sampler},
};

/// How far apart an orchard's trees stand, along its rows and between them, in voxels.
const ROWS: i32 = 14;
/// How far from its trunk any of an orchard tree reaches.
pub const CROWN: i32 = 3;

// Rows of the table of seasons an orchard keeps, after the crops' (see crops.rs).
/// Apple blossom, at the end of April.
pub const BLOSSOM: u8 = 62;
/// Apples: green from July, red in September, picked by October.
pub const FRUIT: u8 = 63;

/// One tree of an orchard.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrchardTree {
    /// The world voxel its trunk stands on, just above the ground.
    pub base: IVec3,
    pub olive: bool,
    /// Its own roll of the dice, for its shape.
    pub hash: u32,
}

impl OrchardTree {
    /// The highest voxel it reaches.
    pub fn top(&self) -> i32 {
        self.base.y + 7
    }
}

/// The orchard trees whose crowns reach the square of continent-local columns from
/// `(lx0, lz0)`, `span` wide, in continent `c`; `sampler` must cover it and [`CROWN`]
/// beyond. `origin` is the continent's corner in world voxels.
pub fn trees_near(c: &Continent, seed: u32, sampler: &Sampler, lx0: i32, lz0: i32, span: i32, origin: (i32, i32)) -> Vec<OrchardTree> {
    let mut out = Vec::new();
    if c.land.wild_square(lx0 - CROWN, lz0 - CROWN, span + 2 * CROWN) {
        return out;
    }
    let cells = |v0: i32| (v0 - CROWN - ROWS / 2).div_euclid(ROWS)..=(v0 + span + CROWN - ROWS / 2).div_euclid(ROWS);
    for j in cells(lz0) {
        for i in cells(lx0) {
            let (lx, lz) = (i * ROWS + ROWS / 2, j * ROWS + ROWS / 2);
            if lx + CROWN < lx0 || lz + CROWN < lz0 || lx - CROWN >= lx0 + span || lz - CROWN >= lz0 + span {
                continue;
            }
            if c.land.managed(lx, lz) < ROUGH {
                continue;
            }
            let Some(field) = plot(c, seed, lx, lz).filter(|p| p.crop.trees()) else { continue };
            let col = sampler.column(lx, lz);
            if !worked(&col) || col.clear < m(2.4) {
                continue;
            }
            // Not on a bank too steep to stand a ladder against.
            let rise = (sampler.column(lx + 1, lz).ground - sampler.column(lx - 1, lz).ground)
                .abs()
                .max((sampler.column(lx, lz + 1).ground - sampler.column(lx, lz - 1).ground).abs());
            if rise > 1.6 {
                continue;
            }
            out.push(OrchardTree {
                base: IVec3::new(origin.0 + lx, col.top() + 1, origin.1 + lz),
                olive: field.crop == Crop::Olives,
                hash: hash3(seed ^ 0x0c4a, lx, 7, lz),
            });
        }
    }
    out
}

/// Builds orchard tree `t`: its wood and leaves through `put` (a world voxel, its
/// material, and whether it displaces what is there), and its blossom and fruit as small
/// boxes into `fine` on the columns `own` says are the chunk's own.
pub fn grow(t: &OrchardTree, put: &mut impl FnMut(IVec3, Voxel, bool), fine: Option<&mut Vec<PlantBox>>, own: impl Fn(i32, i32) -> bool) {
    let r = |k: u32| unit(t.hash.rotate_left(k * 5).wrapping_mul(0x2c1b_3c6d ^ k));
    let (bark, leaf) = if t.olive { (mat::BARK_OLIVE, mat::OLIVE_LEAF) } else { (mat::BARK_OAK, mat::ORCHARD_LEAF) };
    // A short trunk, an olive's leaning; pruned to an open, rounded head.
    let trunk = if t.olive { 2 } else { 2 + (r(1) < 0.5) as i32 };
    let lean = if t.olive { IVec3::new((r(2) * 3.0) as i32 - 1, 0, (r(3) * 3.0) as i32 - 1) } else { IVec3::ZERO };
    for k in 0..trunk {
        put(t.base + IVec3::Y * k + if k + 1 == trunk { lean } else { IVec3::ZERO }, bark, true);
    }
    let middle = t.base + lean + IVec3::Y * (trunk + 1);
    let (across, up) = if t.olive { (2.3 + r(4) * 0.5, 1.4) } else { (2.2 + r(4) * 0.7, 1.8 + r(5) * 0.4) };
    // The main limbs, which a bare winter tree shows.
    for (dx, dz) in [(1, 1), (-1, -1), (1, -1), (-1, 1)] {
        if r(6 + dx as u32 + 2 * (dz + 1) as u32) < 0.7 {
            put(middle + IVec3::new(dx, -1, dz), bark, true);
        }
    }
    let mut leaves = Vec::new();
    let reach = across.ceil() as i32;
    let tall = up.ceil() as i32;
    for dy in -tall..=tall {
        for dz in -reach..=reach {
            for dx in -reach..=reach {
                let d = (dx as f32 / across).powi(2) + (dy as f32 / up).powi(2) + (dz as f32 / across).powi(2);
                let q = middle + IVec3::new(dx, dy, dz);
                let h = unit(hash3(t.hash, q.x, q.y, q.z));
                // Ragged at the edge, and open here and there inside.
                if d > 1.0 || (d > 0.6 && h < 0.25) || h < 0.05 {
                    continue;
                }
                leaves.push((q, d));
                put(q, leaf, false);
            }
        }
    }
    let Some(out) = fine else { return };
    if t.olive {
        return;
    }
    let mut add = |q: IVec3, lo: [i32; 3], hi: [i32; 3], voxel: Voxel, season: u8| {
        let o = q * CELLS;
        out.push(PlantBox {
            lo: [o.x + lo[0], o.y + lo[1], o.z + lo[2]],
            hi: [o.x + hi[0], o.y + hi[1], o.z + hi[2]],
            voxel,
            sway_lo: true,
            sway_hi: true,
            thin: 0,
            season,
            flap: 0,
        });
    };
    let leafy = |q: IVec3| leaves.iter().any(|&(p, _)| p == q);
    for &(q, d) in &leaves {
        if !own(q.x, q.z) {
            continue;
        }
        let h = hash3(t.hash ^ 0xb105, q.x, q.y, q.z);
        // Blossom over the top of the head, apples hanging from its sides.
        if !leafy(q + IVec3::Y) && unit(h) < 0.6 {
            let (a, b) = ((h >> 8) as i32 & 1, (h >> 9) as i32 & 1);
            add(q, [a, CELLS, b], [a + 2, CELLS + 1, b + 2], mat::BLOSSOM, BLOSSOM);
        }
        if d > 0.4 && unit(h.rotate_left(11)) < 0.3 {
            let side = [(1, 0), (-1, 0), (0, 1), (0, -1)][(h >> 12) as usize % 4];
            if !leafy(q + IVec3::new(side.0, 0, side.1)) {
                let (x, z) = (if side.0 > 0 { CELLS } else if side.0 < 0 { -1 } else { 1 + (h >> 14) as i32 % 2 }, if side.1 > 0 { CELLS } else if side.1 < 0 { -1 } else { 1 + (h >> 15) as i32 % 2 });
                add(q, [x, 1, z], [x + 1, 2, z + 1], mat::APPLE, FRUIT);
            }
        }
    }
}

/// Which layer of the world's features a tree is chosen in (see `features::place`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stand {
    /// The biggest trees, far apart.
    Giant,
    /// Ordinary trees.
    Tree,
    /// Bushes, boulders, logs and the like.
    Small,
}

/// What becomes of the feature chosen for a spot, once the land about it is worked.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fate {
    /// It stands as the wild would have it (or nothing does, if nothing was chosen).
    Wild,
    /// It was cleared away.
    Felled,
    /// It was felled, and its stump is left.
    Stump,
    /// Cut back to the ground, it has grown again as a bush of stems.
    Coppiced,
    /// Kept, or let grow, in a hedgerow or as the one old tree of its field.
    Kept(Species),
}

/// What becomes of the feature of layer `stand` that the wild would have at continent-
/// local column `(lx, lz)` (`chosen`, None if it would have none), whose ground is `col`,
/// in continent `c`. `a` and `b` are the spot's own rolls of the dice, 0 to 1. Out in
/// the wild this is a single lookup.
#[allow(clippy::too_many_arguments)]
pub fn fate(c: &Continent, seed: u32, stand: Stand, chosen: Option<Species>, col: &Column, lx: i32, lz: i32, a: f32, b: f32) -> Fate {
    let land = &c.land;
    let managed = land.managed(lx, lz);
    if managed < EDGE * 0.5 || land.yard(lx, lz) {
        return Fate::Wild;
    }
    let Some(field) = plot(c, seed, lx, lz).filter(|_| worked(col)) else {
        return Fate::Wild;
    };
    let farm = &land.farms[field.owner];
    let young = farm.age == Age::Young;
    // A broadleaf for a hedgerow, of the country's kind.
    let hedgerow_tree = || match col.biome {
        Biome::Taiga | Biome::Tundra => Species::Birch,
        _ if col.temperature < 0.32 => Species::Birch,
        Biome::Savanna | Biome::Desert | Biome::RedRock | Biome::Steppe => Species::Acacia,
        b if b.parent() == Biome::Desert => Species::Acacia,
        _ if col.temperature > 0.66 => Species::HolmOak,
        _ => Species::Oak,
    };
    let near_edge = || {
        let d = 3;
        [(d, 0), (-d, 0), (0, d), (0, -d)].iter().any(|&(dx, dz)| plot(c, seed, lx + dx, lz + dz).is_some_and(|p| p.id != field.id))
    };
    match field.crop {
        // An orchard's own trees stand in it, and nothing else (see [`trees_near`]).
        Crop::Orchard | Crop::Olives => Fate::Felled,
        Crop::Rough | Crop::Coppice => {
            // Thinned: the rougher the ground, the more trees, until the wild's own.
            let left = ((PASTURE - managed) / (PASTURE - EDGE)).clamp(0.0, 1.0);
            let coppice = if field.crop == Crop::Coppice { 0.6 } else { 0.2 };
            match stand {
                Stand::Giant if a < 0.35 + 0.65 * left => Fate::Wild,
                Stand::Giant => Fate::Felled,
                Stand::Tree if chosen.is_some() && a < coppice * (1.0 - left * 0.5) => Fate::Coppiced,
                Stand::Tree if a < 0.3 + 0.7 * left => Fate::Wild,
                Stand::Tree if young && b < 0.5 => Fate::Stump,
                Stand::Tree => Fate::Felled,
                Stand::Small => Fate::Wild,
            }
        }
        _ => {
            // The fields proper, and the land at the edge of the wild between them.
            if managed < ROUGH {
                let left = ((ROUGH - managed) / (ROUGH - EDGE * 0.5)).clamp(0.0, 1.0);
                return if a < 0.25 + 0.75 * left { Fate::Wild } else { Fate::Felled };
            }
            match stand {
                Stand::Giant if field.crop == Crop::Pasture && a < 0.15 => Fate::Wild,
                Stand::Giant => Fate::Felled,
                Stand::Tree if near_edge() => {
                    if a < 0.45 {
                        Fate::Kept(chosen.filter(|s| !matches!(s, Species::DeadSnag | Species::Cactus)).unwrap_or_else(hedgerow_tree))
                    } else {
                        Fate::Felled
                    }
                }
                Stand::Tree if matches!(field.crop, Crop::Pasture | Crop::Hay) && a < 0.035 && managed < CROPS => {
                    Fate::Kept(chosen.unwrap_or_else(hedgerow_tree))
                }
                Stand::Tree if young && chosen.is_some() && b < 0.35 => Fate::Stump,
                Stand::Tree => Fate::Felled,
                // Bushes stand in the hedges; stones and logs were cleared from the
                // fields and piled at their edges.
                Stand::Small if near_edge() => Fate::Wild,
                Stand::Small if matches!(field.crop, Crop::Pasture | Crop::Fallow) && a < 0.15 => Fate::Wild,
                Stand::Small => Fate::Felled,
            }
        }
    }
}
