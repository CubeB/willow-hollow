//! Land held to a shared height along a macro-cell border.
//!
//! A mountain range can cross from one cell into the next, but each cell still simulates
//! its own bounded landmass: the cell cannot know what its neighbour's rivers do. So
//! where a range reaches a border its land is held, in a narrow band, to a height both
//! cells compute from the world position alone, rising toward the border on both sides.
//! Mostly water runs away from the border into each cell's own sea, so the crossing is a
//! drainage divide. Where one side stands higher than the shared height, though, its
//! water has to leave across the border, so the band has outlets there (see [`outlets`]).

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
            if shape.field.border_pin(px, pz, edge, 1.0).is_none() {
                continue;
            }
            // Shared with the neighbours on the border itself, so the held height is too.
            let u = shape.uplift(px, pz, shape.land(px, pz));
            if let Some((target, weight, blend)) = shape.field.border_pin(px, pz, edge, u) {
                out.push(Pin { i: (z * w + x) as u32, weight, blend, target });
            }
        }
    }
    // A cell beside the ring is an outlet (see [`outlets`]); it stands at least as high
    // as the ring beyond it, so water leaving there never has to climb.
    let ring: std::collections::HashMap<u32, f32> =
        out.iter().filter(|p| on_ring(w, p.i as usize)).map(|p| (p.i, p.target)).collect();
    for p in &mut out {
        let i = p.i as usize;
        if beside_ring(w, i) {
            for (dx, dz) in NEIGHBOURS {
                let j = (i as isize + dz * w as isize + dx) as u32;
                if let Some(&t) = ring.get(&j) {
                    p.target = p.target.max(t);
                }
            }
            p.weight = 1.0;
        }
    }
    out
}

const NEIGHBOURS: [(isize, isize); 8] =
    [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, 1), (1, -1), (-1, -1)];

/// True if grid cell `i` is in the row just inside the outer ring.
fn beside_ring(w: usize, i: usize) -> bool {
    let (x, z) = (i % w, i / w);
    !on_ring(w, i) && (x == 1 || z == 1 || x == w - 2 || z == w - 2)
}

/// Where the land inside a crossing stands higher than the shared border height, its
/// water must cross the border: a closed border would turn the land into a basin, which
/// fills to a flat plateau. So the pinned land beside the ring is where the flood starts,
/// as well as the coast, at its own height. Water reaching it then runs on into the ring.
pub(super) fn outlets(level: &Level, pins: &[Pin], height: &[f32]) -> Vec<(u32, f32)> {
    pins.iter()
        .filter(|p| !level.ocean[p.i as usize] && beside_ring(level.w, p.i as usize))
        .map(|p| (p.i, height[p.i as usize]))
        .collect()
}

/// True if a range holds land anywhere on the outer ring of a `w`-wide grid.
pub(super) fn touches(shape: &Shape, w: usize) -> bool {
    if shape.field.is_empty() {
        return false;
    }
    (0..w).any(|k| {
        [(k, 0), (k, w - 1), (0, k), (w - 1, k)].into_iter().any(|(x, z)| {
            let (px, pz) = super::norm(w, x, z);
            shape.field.border_pin(px, pz, 1.0, 1.0).is_some()
        })
    })
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

/// Keeps standing water out of the band: a lake there could stand against open air in
/// the neighbouring cell. Raising each band cell to its spill level leaves nothing to
/// fill, and with outlets beside the ring the spill level is never far above it.
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
/// the land inside drained into them. They become land at their shared height, where
/// water leaves the cell. A cell that drained into them from below that height is
/// pointed down into the cell instead, with its water added on downstream so discharge
/// still only grows.
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

    // The ring cells are where water leaves; they carry what routing sent them.
    for p in &ring {
        let i = p.i as usize;
        flow.rcv[i] = i as u32;
        water_level[i] = height[i];
        flow.discharge[i] += rain[i].max(0.05);
    }
    // Land that drained into the ring from below its height turns inland instead.
    let mut moved: Vec<(usize, f32)> = Vec::new();
    for i in 0..w * w {
        let r = flow.rcv[i] as usize;
        if level.ocean[i] || pinned[i] || !pinned[r] || water_level[i] >= water_level[r] {
            continue;
        }
        if let Some(r) = lowest(i, water_level) {
            flow.rcv[i] = r as u32;
            water_level[i] = water_level[i].max(water_level[r]);
            moved.push((i, flow.discharge[i]));
        }
    }
    // Their water joins the cells below; order does not matter since each only adds.
    for (start, q) in moved {
        let mut at = flow.rcv[start] as usize;
        let mut steps = 0;
        while !level.ocean[at] && steps < w * w {
            flow.discharge[at] += q;
            if flow.rcv[at] as usize == at {
                break;
            }
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
