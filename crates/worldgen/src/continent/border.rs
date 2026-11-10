//! Land held to a shared height along a macro-cell border.
//!
//! A mountain range can cross from one cell into the next, but each cell still simulates
//! its own bounded landmass: the cell cannot know what its neighbour's rivers do. So
//! where a range reaches a border its land is held, in a narrow band, to a height both
//! cells compute from the world position alone, rising toward the border on both sides.
//! Water therefore runs away from the border into each cell's own sea, which makes the
//! crossing a drainage divide; no river ever needs to cross it, and heights match.

use super::{Level, NO_LAKE, flow::Flow, shape::Shape};
use crate::structures::{BORDER_BAND, Field};

/// A grid cell in the band, held toward `target` by `weight` (1 at the border).
pub(super) struct Pin {
    pub i: u32,
    pub weight: f32,
    /// Weight of the final blend toward `target`, once the simulation is done.
    pub blend: f32,
    pub target: f32,
}

/// The cells of `level` in the border band that lie on a range.
pub(super) fn pins(level: &Level, shape: &Shape) -> Vec<Pin> {
    let mut out = Vec::new();
    if shape.field.is_empty() {
        return out;
    }
    let w = level.w;
    let band = ((w as f32 * BORDER_BAND * 0.5).ceil() as usize + 1).min(w / 2);
    for z in 0..w {
        for x in 0..w {
            // Cheap test first: most of the grid is nowhere near a border.
            if x >= band && z >= band && x < w - band && z < w - band {
                continue;
            }
            let (px, pz) = level.norm(x, z);
            let edge = px.abs().max(pz.abs());
            if let Some((target, weight, blend)) = shape.field.border_pin(px, pz, edge) {
                out.push(Pin { i: (z * w + x) as u32, weight, blend, target });
            }
        }
    }
    out
}

/// Pulls the pinned land cells to their held height; the ring is settled at the end.
pub(super) fn hold(level: &Level, pins: &[Pin], height: &mut [f32]) {
    for p in pins {
        let i = p.i as usize;
        if !level.ocean[i] {
            height[i] = (height[i] + (p.target - height[i]) * p.weight).max(0.3);
        }
    }
}

/// Once erosion is over, eases the land across the whole band toward the held profile,
/// so the crossing is a broad massif rather than a narrow crest along the border.
pub(super) fn settle(level: &Level, pins: &[Pin], height: &mut [f32]) {
    for p in pins {
        let i = p.i as usize;
        if !level.ocean[i] {
            let w = p.blend.max(p.weight);
            height[i] = (height[i] + (p.target - height[i]) * w).max(0.3);
        }
    }
}

/// Keeps standing water out of the band. Routing treats the ring as closed, so a hollow
/// beside it could fill above the shared border height, and its water would then stand
/// against open air in the neighbouring cell. Raising each band cell to its spill level
/// leaves nothing to fill.
pub(super) fn dry(level: &Level, flow: &Flow, pins: &[Pin], height: &mut [f32]) {
    for p in pins {
        let i = p.i as usize;
        if !level.ocean[i] && p.blend > 0.02 {
            height[i] = height[i].max(flow.filled[i]);
        }
    }
}

/// True if grid cell `i` is on the outermost ring of a `w`-wide grid.
fn on_ring(w: usize, i: usize) -> bool {
    let (x, z) = (i % w, i / w);
    x == 0 || z == 0 || x == w - 1 || z == w - 1
}

/// Finishes the border crossings once routing is done.
///
/// Routing needs the outer ring to be sea, so pinned ring cells were sea up to now and
/// the first land cell inside drained into them. They become land at their shared
/// height, and both sets of cells are pointed down into the cell instead, with their
/// water added on downstream so discharge still only grows.
pub(super) fn finish(
    level: &mut Level,
    flow: &mut Flow,
    height: &mut [f32],
    water_level: &mut [f32],
    lake: &mut [f32],
    rain: &[f32],
    pins: &[Pin],
) {
    let w = level.w;
    let ring: Vec<&Pin> = pins.iter().filter(|p| on_ring(w, p.i as usize)).collect();
    if ring.is_empty() {
        return;
    }
    let mut pinned = vec![false; w * w];
    for p in &ring {
        let i = p.i as usize;
        pinned[i] = true;
        level.ocean[i] = false;
        lake[i] = NO_LAKE;
        height[i] = p.target;
    }

    // The lowest neighbour inside the grid proper, which water can reach from `i`.
    let lowest = |i: usize, level_of: &[f32]| -> Option<usize> {
        let (x, z) = ((i % w) as i32, (i / w) as i32);
        let mut best: Option<usize> = None;
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
            let (nx, nz) = (x + dx, z + dz);
            if nx < 1 || nz < 1 || nx >= w as i32 - 1 || nz >= w as i32 - 1 {
                continue;
            }
            let j = nz as usize * w + nx as usize;
            if best.is_none_or(|b| level_of[j] < level_of[b]) {
                best = Some(j);
            }
        }
        best
    };

    // Land cells that drained into the ring turn inland, to their lowest neighbour.
    let mut moved: Vec<(usize, f32)> = Vec::new();
    for i in 0..w * w {
        if level.ocean[i] || pinned[i] || !pinned[flow.rcv[i] as usize] {
            continue;
        }
        if let Some(r) = lowest(i, water_level) {
            flow.rcv[i] = r as u32;
            water_level[i] = water_level[i].max(water_level[r]);
            moved.push((i, flow.discharge[i]));
        }
    }
    for p in &ring {
        let i = p.i as usize;
        let r = lowest(i, water_level).expect("ring cell has an inner neighbour");
        flow.rcv[i] = r as u32;
        water_level[i] = height[i].max(water_level[r]);
        flow.discharge[i] = rain[i].max(0.05);
        moved.push((i, flow.discharge[i]));
    }
    // Their water joins the cells below; order does not matter since each only adds.
    for (start, q) in moved {
        let mut at = flow.rcv[start] as usize;
        let mut steps = 0;
        while !level.ocean[at] && steps < w * w {
            flow.discharge[at] += q;
            at = flow.rcv[at] as usize;
            steps += 1;
        }
    }
}

/// Blends climate near a crossing toward the same value on both sides of the border,
/// so the biome does not change abruptly along it.
pub(super) fn blend_climate(
    level: &Level,
    pins: &[Pin],
    shape: &Shape,
    height: &[f32],
    temperature: &mut [f32],
    rain: &mut [f32],
) {
    if pins.is_empty() {
        return;
    }
    // Sea-level climate of the nine macro-cells around this one, at their centres.
    let desc = shape.desc();
    let mut grid = Vec::with_capacity(9);
    for dz in -1..=1 {
        for dx in -1..=1 {
            let n = crate::macro_layout::cell_desc(desc.world_seed, desc.mx + dx, desc.mz + dz);
            grid.push((n.temperature, n.wetness));
        }
    }
    let at = |ix: i32, iz: i32| grid[((iz + 1) * 3 + ix + 1).clamp(0, 8) as usize];
    let shared = |px: f32, pz: f32| {
        // Position in macro-cells relative to this cell's corner; centres at .5.
        let (fx, fz) = ((px as f64 + 1.0) * 0.5, (pz as f64 + 1.0) * 0.5);
        let (cx, cz) = ((fx - 0.5).floor(), (fz - 0.5).floor());
        let (tx, tz) = ((fx - 0.5 - cx) as f32, (fz - 0.5 - cz) as f32);
        let (x0, z0) = (cx as i32, cz as i32);
        let (a, b) = (at(x0, z0), at(x0 + 1, z0));
        let (c, d) = (at(x0, z0 + 1), at(x0 + 1, z0 + 1));
        let lerp = |p: f32, q: f32, t: f32| p + (q - p) * t;
        (
            lerp(lerp(a.0, b.0, tx), lerp(c.0, d.0, tx), tz),
            lerp(lerp(a.1, b.1, tx), lerp(c.1, d.1, tx), tz),
        )
    };
    // Snow lines and biome edges are thresholds, so a blend only as wide as the band
    // would leave a straight edge where it ends. The climate therefore eases over a
    // third of the cell, which is far wider than the band.
    const REACH: f32 = 0.35;
    let mut held = vec![0.0f32; level.w * level.w];
    for p in pins {
        held[p.i as usize] = p.weight;
    }
    for z in 0..level.w {
        for x in 0..level.w {
            let i = z * level.w + x;
            if level.ocean[i] {
                continue;
            }
            let (px, pz) = level.norm(x, z);
            let b = 1.0 - px.abs().max(pz.abs());
            if b >= REACH && held[i] == 0.0 {
                continue;
            }
            let t = (b / REACH).clamp(0.0, 1.0);
            let hold = (1.0 - t * t * (3.0 - 2.0 * t)).max(held[i]).max(Field::border_hold(1.0 - b));
            let (sea_temp, wet) = shared(px, pz);
            temperature[i] += (sea_temp - height[i].max(0.0) * 0.00115 - temperature[i]) * hold;
            rain[i] += (wet - rain[i]) * hold;
        }
    }
}
