//! What stands between two fields (#64): by the houses a fence, a wattle hurdle or a
//! wall; further out a hedge; out in the rough a ragged hedgerow or a tumbled wall; and at
//! the edge of the wild a line of scrub or a stone or two where a wall once ran.
//!
//! The place decides what they are made of: stone walls in the uplands, on rocky ground
//! and round the oldest places, of the local stone; hedges in the green lowlands;
//! driftwood and reed hurdles by the sea and the marshes; piled thorn and rubble in the
//! dry country. Every kind has its gaps and gateways, fallen stretches and dead patches,
//! more of them the further out it runs, and none stands on a trail: where a trail
//! crosses a boundary there is a gateway.
//!
//! A boundary is worked out from the fields of a column and its neighbours alone, so two
//! chunks agree about every one on their border. Of the two columns either side of the
//! line between two fields, the one in the field with the lower number is its core,
//! which every kind stands on; a wall or a hedge is thickened onto the other side too.

use glam::IVec3;
use voxel::{
    Voxel,
    mesh::{CELLS, PlantBox},
};

use super::{
    fields::{Patch, Plot},
    influence::{CROPS, EDGE, Farm, Land, PASTURE, ROUGH},
};
use crate::{
    biome::Biome,
    materials as mat,
    noise::{hash2, hash3, perlin2, unit},
    scale::{m, m64},
    settlements::{Age, Region},
    surface::Column,
};

/// What a boundary is.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Style {
    /// Posts and rails.
    Fence,
    /// Woven hazel hurdles between stakes.
    Hurdle,
    /// A drystone wall of the local stone.
    Wall,
    /// A hedge of thorn and hazel, with the odd holly in it.
    Hedge,
    /// A line of scrub where a hedge was let go.
    Scrub,
    /// A stone here and there where a wall once ran.
    Stones,
    /// Dead thorn piled up, as dry country fences its fields.
    Thorn,
    /// Driftwood posts and a rail, by the sea and in the marshes.
    Driftwood,
}

/// The boundary a column carries.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Edge {
    pub style: Style,
    /// Whether it is the core of the line (see the module's notes), or only the far side
    /// of a wall or hedge.
    pub core: bool,
    /// How far gone to ruin it is: 0 by the houses, 1 at the edge of the wild.
    pub decay: f32,
    /// The settlement whose it is, by its index in [`Land::farms`].
    pub owner: usize,
    /// Whether the core runs on into the next column each way: +x, -x, +z, -z.
    pub links: [bool; 4],
    /// Whether it is beside a trail, where it ends in a gatepost.
    pub gate: bool,
}

/// The four directions across the ground, as [`Edge::links`] has them.
const SIDES: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

/// How close to a trail's edge a boundary comes: a gateway's half width.
const GATEWAY: f32 = m(1.2);
/// Within this of a trail's edge a boundary ends in a gatepost.
const GATEPOST: f32 = m(2.0);

/// What stands between fields `a` and `b` of `land`, whose line runs over ground `col`.
pub(super) fn style(land: &Land, a: &Plot, b: &Plot, col: &Column) -> Style {
    let (lo, hi) = if a.id < b.id { (a, b) } else { (b, a) };
    let farm = &land.farms[lo.owner];
    let managed = (lo.managed + hi.managed) * 0.5;
    let r = unit(hash2(lo.id, hi.id as i32, 0x5e1d));
    use Biome::*;
    let stony = matches!(farm.region, Region::Upland | Region::Alpine)
        || matches!(col.biome, Moor | Heath | Tundra | AlpineMeadow | Volcanic | Steppe)
        || matches!(farm.biome, Moor | Heath | Tundra | AlpineMeadow | Volcanic);
    let ancient = farm.age == Age::Ancient;
    let dry = farm.region == Region::Desert || col.biome.arid() || col.biome == Savanna;
    let wet = farm.region == Region::Wetland || matches!(col.biome, Marsh | Bog | Mangrove) || (farm.coast && col.ground < m(5.0));
    if managed < ROUGH {
        return if stony || ancient || dry { Style::Stones } else { Style::Scrub };
    }
    if managed >= CROPS {
        return match () {
            _ if dry => if r < 0.5 { Style::Wall } else { Style::Thorn },
            _ if stony || (ancient && r < 0.7) => Style::Wall,
            _ if wet => if farm.coast { Style::Driftwood } else { Style::Hurdle },
            _ if farm.age == Age::Young => if r < 0.7 { Style::Fence } else { Style::Hedge },
            _ if farm.age == Age::Established && r < 0.25 => Style::Hurdle,
            _ if farm.age == Age::Established && r < 0.45 => Style::Fence,
            _ => Style::Hedge,
        };
    }
    if managed >= PASTURE {
        return match () {
            _ if dry => if r < 0.6 { Style::Thorn } else { Style::Wall },
            _ if stony || (ancient && r < 0.4) => Style::Wall,
            _ if wet && r < 0.5 => if farm.coast { Style::Driftwood } else { Style::Hurdle },
            _ if farm.age == Age::Young && r < 0.4 => Style::Fence,
            _ => Style::Hedge,
        };
    }
    match () {
        _ if dry => Style::Thorn,
        _ if stony => Style::Wall,
        _ => Style::Hedge,
    }
}

/// Whether column `(i, j)` of `patch` is the core of a boundary: in a field, beside
/// (or kitty-corner to) a field with a higher number. The columns so found make an
/// unbroken line along the boundary, which walls and fences can follow.
fn core_of(patch: &Patch, i: usize, j: usize) -> Option<(Plot, Plot)> {
    let p = *patch.at(i, j)?;
    if i == 0 || j == 0 || i + 1 >= patch.n || j + 1 >= patch.n {
        return None;
    }
    // The sides first, so the far field is the one across the line where there is one.
    for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
        if let Some(q) = patch.at((i as i32 + dx) as usize, (j as i32 + dz) as usize)
            && q.id > p.id
        {
            return Some((p, *q));
        }
    }
    None
}

/// The boundary column `(i, j)` of `patch` carries, if any, whose ground is `col`. Needs
/// the columns all round it in the patch, and for its links, all round them too.
pub fn edge(land: &Land, patch: &Patch, col: &Column, i: usize, j: usize) -> Option<Edge> {
    let (p, q, core) = match core_of(patch, i, j) {
        Some((p, q)) => (p, q, true),
        None => {
            let p = *patch.at(i, j)?;
            if i == 0 || j == 0 || i + 1 >= patch.n || j + 1 >= patch.n {
                return None;
            }
            let q = SIDES.iter().find_map(|&(dx, dz)| patch.at((i as i32 + dx) as usize, (j as i32 + dz) as usize).filter(|q| q.id < p.id))?;
            (p, *q, false)
        }
    };
    // Nothing stands across a trail, nor on its verge.
    if col.clear < GATEWAY {
        return None;
    }
    let style = style(land, &p, &q, col);
    let managed = (p.managed + q.managed) * 0.5;
    let decay = 1.0 - ((managed - EDGE) / (CROPS - EDGE)).clamp(0.0, 1.0);
    let links = SIDES.map(|(dx, dz)| {
        let (a, b) = (i as i32 + dx, j as i32 + dz);
        a >= 0 && b >= 0 && core_of(patch, a as usize, b as usize).is_some()
    });
    Some(Edge { style, core, decay, owner: p.owner.min(q.owner), links, gate: col.clear < GATEPOST })
}

/// Builds boundary `edge` on the column at world `(x, z)`, whose ground is `col`: its
/// voxels through `put` (a world voxel, its material, and whether it displaces what is
/// there), and, if `fine` is given (the column is the chunk's own), its posts, rails and
/// hurdles as small boxes. `farm` is the settlement whose it is.
#[allow(clippy::too_many_arguments)]
pub fn build(edge: &Edge, farm: &Farm, col: &Column, x: i32, z: i32, seed: u32, put: &mut impl FnMut(IVec3, Voxel, bool), fine: Option<&mut Vec<PlantBox>>) {
    let y0 = col.top() + 1;
    let h = hash3(seed ^ 0xb0d5, x, 0, z);
    let r = |k: u32| unit(h.rotate_left(k * 7).wrapping_mul(0x9e37_79b9 ^ k));
    let noise = |salt: u32, scale: f64| perlin2(seed ^ salt, x as f64 / m64(scale), z as f64 / m64(scale));
    let decay = edge.decay;
    let at = |k: i32| IVec3::new(x, y0 + k, z);
    // Gaps where a stretch has fallen or died or been broken through, more of them out
    // in the rough; gateways where a trail goes through.
    let gap = noise(0xb0d6, 3.2) > 0.5 - 0.45 * decay;
    let mut posts: Vec<(Voxel, i32)> = Vec::new();
    match edge.style {
        Style::Wall => {
            if edge.gate {
                // A gatepost of bigger stones, a little taller than the wall.
                if edge.core {
                    for k in 0..4 {
                        put(at(k), farm.stone_dark, true);
                    }
                }
                return;
            }
            let moss = if col.biome.arid() { 0.0 } else { 0.08 + 0.4 * decay + if farm.age == Age::Ancient { 0.15 } else { 0.0 } };
            let stone = |k: u32| {
                let roll = r(k);
                if roll < moss {
                    mat::MOSSY_STONE
                } else if roll < moss + 0.3 {
                    farm.stone_dark
                } else {
                    farm.stone
                }
            };
            if gap {
                // Tumbled: a stone or two lying where the wall stood.
                if r(1) < 0.35 + 0.3 * decay {
                    put(at(0), stone(2), true);
                }
                return;
            }
            let fallen = noise(0xb0d7, 7.0) > 0.4 - 0.5 * decay;
            let height = (3.0 - 1.2 * decay - if fallen { 1.4 } else { 0.0 } + (r(3) - 0.5) * 0.6).round().max(1.0) as i32;
            for k in 0..height {
                put(at(k), stone(4 + k as u32), true);
            }
        }
        Style::Hedge => {
            // A gateway's post is a small box, with the fences' (below).
            if gap && !edge.gate {
                if edge.core && r(1) < 0.3 {
                    put(at(0), mat::HEDGE, false);
                }
            } else if !edge.gate && (edge.core || r(6) < 0.7) {
                // Patches of it have died back to bare stems.
                let dead = noise(0xb0d8, 4.5) > 0.45 - 0.25 * decay;
                let tall = if edge.core { 3.6 } else { 2.6 };
                let height = (tall + r(2) * 1.8 - 1.4 * decay).round().max(1.0) as i32;
                for k in 0..height {
                    let roll = r(5 + k as u32);
                    let v = if k == 0 && r(4) < 0.45 {
                        mat::HEDGE_WOOD
                    } else if dead {
                        if roll < 0.45 { mat::TWIG } else { continue }
                    } else if k == 1 && roll < 0.2 {
                        mat::TWIG
                    } else if roll < 0.07 {
                        mat::LEAVES_HOLLY
                    } else {
                        mat::HEDGE
                    };
                    put(at(k), v, false);
                }
            }
        }
        Style::Scrub => {
            if edge.core && !edge.gate && noise(0xb0d9, 5.0) > 0.05 && r(1) < 0.7 {
                let height = 1 + (r(2) < 0.4) as i32;
                for k in 0..height {
                    let roll = r(3 + k as u32);
                    put(at(k), if roll < 0.15 { mat::TWIG } else if roll < 0.25 { mat::LEAVES_HOLLY } else { mat::HEDGE }, false);
                }
            }
        }
        Style::Stones => {
            if edge.core && !edge.gate && r(1) < 0.1 + 0.12 * (1.0 - decay) {
                let v = if r(2) < 0.4 && !col.biome.arid() { mat::MOSSY_STONE } else { farm.stone };
                put(at(0), v, true);
                if r(3) < 0.2 {
                    put(at(1), farm.stone_dark, true);
                }
            }
        }
        Style::Thorn => {
            if !edge.gate && !gap && (edge.core || r(6) < 0.5) && r(1) < 0.8 {
                let height = 1 + (r(2) < 0.5 - 0.4 * decay) as i32;
                for k in 0..height {
                    put(at(k), if r(3 + k as u32) < 0.15 { farm.stone } else { mat::THORN }, false);
                }
            }
        }
        Style::Fence | Style::Hurdle | Style::Driftwood => {}
    }
    let Some(out) = fine else { return };
    if !edge.core {
        return;
    }
    let origin = [x * CELLS, y0 * CELLS, z * CELLS];
    let mut add = |lo: [i32; 3], hi: [i32; 3], voxel: Voxel| {
        out.push(PlantBox {
            lo: [origin[0] + lo[0], origin[1] + lo[1], origin[2] + lo[2]],
            hi: [origin[0] + hi[0], origin[1] + hi[1], origin[2] + hi[2]],
            voxel,
            sway_lo: false,
            sway_hi: false,
            thin: 0,
            season: 0,
            flap: 0,
        });
    };
    // A bar from the middle of the column to its edge on each side the line runs on, at
    // `lo..hi` cells up: a rail, or a hurdle's panel.
    let bars = |lo: i32, hi: i32, voxel: Voxel, keep: &dyn Fn(usize) -> bool, add: &mut dyn FnMut([i32; 3], [i32; 3], Voxel)| {
        for (k, &(dx, dz)) in SIDES.iter().enumerate() {
            if !edge.links[k] || !keep(k) {
                continue;
            }
            let (x0, x1) = if dx > 0 { (2, CELLS) } else if dx < 0 { (0, 2) } else { (1, 2) };
            let (z0, z1) = if dz > 0 { (2, CELLS) } else if dz < 0 { (0, 2) } else { (1, 2) };
            add([x0, lo, z0], [x1, hi, z1], voxel);
        }
    };
    let fence_gap = noise(0xb0da, 3.0) > 0.7 - 0.4 * decay;
    match edge.style {
        _ if edge.gate => posts.push((farm.wood, 13)),
        Style::Fence if !fence_gap => {
            if (x + 2 * z).rem_euclid(3) == 0 || edge.links.iter().filter(|&&l| l).count() != 2 {
                posts.push((farm.wood, 12));
            }
            // A rail now and then has rotted through out in the rough.
            let rail = mat::PLANK_DARK;
            bars(5, 6, rail, &|_| true, &mut add);
            bars(9, 10, rail, &|k| r(10 + k as u32) > decay * 0.5, &mut add);
        }
        Style::Hurdle if !fence_gap => {
            posts.push((mat::WITHY, 10));
            bars(0, 8, mat::WATTLE, &|k| r(10 + k as u32) > decay * 0.4, &mut add);
        }
        Style::Driftwood if !fence_gap => {
            if r(1) < 0.6 {
                posts.push((mat::WOOD_DEAD, 6 + (r(2) * 6.0) as i32));
            }
            bars(6, 7, mat::WOOD_DEAD, &|k| r(10 + k as u32) < 0.75, &mut add);
        }
        _ => {}
    }
    for (voxel, height) in posts {
        add([1, 0, 1], [3, height, 3], voxel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{continent::SimConfig, world::GameWorld};

    #[test]
    fn boundaries_run_unbroken_and_stand_off_the_trails() {
        let w = GameWorld::with_config(3, SimConfig { size: 256 });
        let c = w.continent(0, 0);
        let mut styles = std::collections::HashSet::new();
        let mut cores = 0;
        for f in c.land.farms.iter().take(6) {
            let (x0, z0) = (f.at.x as i32 + f.radius as i32, f.at.y as i32 - 40);
            let n = 80;
            let sampler = crate::surface::Sampler::new(&c, w.seed(), x0, z0, n as i32);
            let columns: Vec<Column> = (0..n * n).map(|i| sampler.column(x0 + (i / n) as i32, z0 + (i % n) as i32)).collect();
            let patch = Patch::new(&c, w.seed(), x0, z0, n, &columns);
            for i in 1..n - 1 {
                for j in 1..n - 1 {
                    let col = &columns[i * n + j];
                    let Some(e) = edge(&c.land, &patch, col, i, j) else { continue };
                    styles.insert(e.style);
                    assert!(col.clear >= GATEWAY, "a boundary on a trail at {i},{j}");
                    assert!(col.water_surface().is_none());
                    if e.core {
                        cores += 1;
                        // The core is a line: unless it ends at a gateway or the edge of
                        // the fields, it runs on to another core column.
                        let next = SIDES.iter().any(|&(dx, dz)| core_of(&patch, (i as i32 + dx) as usize, (j as i32 + dz) as usize).is_some());
                        let open = SIDES.iter().any(|&(dx, dz)| {
                            let k = (i as i32 + dx) as usize * n + (j as i32 + dz) as usize;
                            patch.at((i as i32 + dx) as usize, (j as i32 + dz) as usize).is_none() || columns[k].clear < GATEWAY
                        });
                        assert!(next || open || i <= 2 || j <= 2 || i >= n - 3 || j >= n - 3, "a lone boundary post at {i},{j}");
                    }
                }
            }
        }
        assert!(cores > 50, "only {cores} boundary columns");
        assert!(!styles.is_empty());
    }
}
