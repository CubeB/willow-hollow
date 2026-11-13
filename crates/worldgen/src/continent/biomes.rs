//! Choosing each cell's biome from its climate and its ground, and the terrain some
//! biomes bring with them.

use super::{Level, NO_LAKE, flow::Flow, shape::Shape};
use crate::{
    biome::{Biome, Site},
    noise::{fbm2, ridged2},
};

/// Slow noise over world position, -1..1, that hands stretches of country to one
/// sub-biome or another. It depends on the world seed and position alone, so it runs on
/// unbroken across cell borders.
fn variety(shape: &Shape, px: f32, pz: f32) -> f32 {
    let d = shape.desc();
    let (u, v) = (d.mx as f64 * 2.0 + px as f64 + 1.0, d.mz as f64 * 2.0 + pz as f64 + 1.0);
    (fbm2(d.world_seed ^ 0xb10e, u * 1.7 + 0.31, v * 1.7 + 0.53, 3, 2.0, 0.5) * 1.6).clamp(-1.0, 1.0)
}

/// Rise over run at grid cell `(x, z)`, from its four neighbours.
fn slope(level: &Level, height: &[f32], x: usize, z: usize) -> f32 {
    let w = level.w;
    let at = |x: usize, z: usize| height[z * w + x].max(0.0);
    let (x0, x1) = (x.saturating_sub(1), (x + 1).min(w - 1));
    let (z0, z1) = (z.saturating_sub(1), (z + 1).min(w - 1));
    let gx = (at(x1, z) - at(x0, z)) / ((x1 - x0).max(1) as f32 * level.dx);
    let gz = (at(x, z1) - at(x, z0)) / ((z1 - z0).max(1) as f32 * level.dx);
    gx.hypot(gz)
}

/// Spreads `v` outward, losing `fall` per cell, so each cell holds the largest value
/// near it. Two separable passes; close enough to round for this.
fn spread(w: usize, v: &mut [f32], fall: f32) {
    for z in 0..w {
        let row = z * w;
        for x in 1..w {
            v[row + x] = v[row + x].max(v[row + x - 1] - fall);
        }
        for x in (0..w - 1).rev() {
            v[row + x] = v[row + x].max(v[row + x + 1] - fall);
        }
    }
    for x in 0..w {
        for z in 1..w {
            v[z * w + x] = v[z * w + x].max(v[(z - 1) * w + x] - fall);
        }
        for z in (0..w - 1).rev() {
            v[z * w + x] = v[z * w + x].max(v[(z + 1) * w + x] - fall);
        }
    }
}

/// How far, in metres of world, waterlogging and the coast reach inland.
const WET_REACH: f32 = 700.0;
const COAST_REACH: f32 = 420.0;
/// Half-width, in metres, of the country weighed to judge how sheltered a coast is.
const SHELTER_REACH: f32 = 2400.0;
/// Half-width, in metres, of the country whose mean height and slope say whether it is a
/// broad upland (see [`Site::broad_slope`]).
const BROAD_REACH: f32 = 250.0;
/// Half-width, in metres, of the ground whose mean slope is a spot's slope.
const LOCAL_REACH: f32 = 60.0;
/// Least area, in square metres, of a patch of one biome. A smaller patch is a speck no
/// walker would notice as a place of its own, and is handed to the country around it.
const LEAST_PATCH: f32 = 150_000.0;

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Per-cell biome.
pub(super) fn classify(
    level: &Level,
    shape: &Shape,
    flow: &Flow,
    height: &[f32],
    lake: &[f32],
    temperature: &[f32],
    rain: &[f32],
) -> Vec<u8> {
    let (w, n) = (level.w, level.w * level.w);
    let metres = level.dx * 0.8;
    // Big rivers and lakes waterlog the flat ground around them.
    let mut wet: Vec<f32> = (0..n)
        .map(|i| {
            if level.ocean[i] {
                0.0
            } else if lake[i] != NO_LAKE {
                1.0
            } else {
                // Catchment in square kilometres: about 0.6 at 30, 1 at 300.
                let km2 = flow.discharge[i] * metres * metres / 1.0e6;
                ((km2.max(1e-6).log10() - 0.3) * 0.45).clamp(0.0, 1.0)
            }
        })
        .collect();
    spread(w, &mut wet, metres / WET_REACH);
    let sea: Vec<f32> = level.ocean.iter().map(|&o| if o { 1.0 } else { 0.0 }).collect();
    // Share of the country around that is open sea: about half on a straight coast,
    // much more on a headland or a small island, much less at the head of a bay.
    let open = super::box_mean(&sea, w, (SHELTER_REACH / metres).round().max(2.0) as usize);
    let mut coast = sea;
    spread(w, &mut coast, metres / COAST_REACH);
    // The lie of the country around, for telling a broad upland from a lone ridge.
    let broad = (BROAD_REACH / metres).round().max(1.0) as usize;
    let slopes: Vec<f32> = (0..n).map(|i| if level.ocean[i] { 1.0 } else { slope(level, height, i % w, i / w) }).collect();
    let broad_slope = super::box_mean(&slopes, w, broad);
    let heights: Vec<f32> = height.iter().map(|h| h.max(0.0)).collect();
    let broad_height = super::box_mean(&heights, w, broad);
    // The slope a biome answers to is that of the ground for a few dozen metres around,
    // not of one cell: otherwise every hummock on a floodplain breaks its marsh into
    // specks.
    let local_slope = super::box_mean(&slopes, w, (LOCAL_REACH / metres).round() as usize);

    let mut out = vec![Biome::Ocean as u8; n];
    for z in 0..w {
        for x in 0..w {
            let i = z * w + x;
            if level.ocean[i] {
                continue;
            }
            let (px, pz) = level.norm(x, z);
            // The border ring is shared with the neighbouring cell, which sees different
            // ground around it; there only what both cells know may count.
            let ring = x == 0 || z == 0 || x == w - 1 || z == w - 1;
            let site = Site {
                temperature: temperature[i],
                rain: rain[i],
                height: height[i],
                slope: if ring { 0.1 } else { local_slope[i] },
                wet: if ring { 0.0 } else { wet[i] },
                coast: if ring { 0.0 } else { coast[i] },
                shelter: if ring { 0.0 } else { 1.0 - smoothstep(0.36, 0.56, open[i]) },
                variety: variety(shape, px, pz),
                volcanic: shape.volcanic(px, pz),
                broad_height: if ring { 0.0 } else { broad_height[i] },
                broad_slope: if ring { 1.0 } else { broad_slope[i] },
            };
            out[i] = Biome::select(&site) as u8;
        }
    }
    absorb_specks(level, &mut out, (LEAST_PATCH / (metres * metres)).round() as usize);
    out
}

/// Hands each patch of one biome smaller than `least` cells to the land biome it shares
/// the most edge with, so the map is made of places rather than confetti. Snow, the
/// meadows under it and volcanic ground keep their small patches: they mark summits and
/// cones, however small. So does any patch on the border ring, whose biome the cell next
/// door must agree on. Each round judges every patch against the map as it stood before
/// the round, so the outcome does not hang on the order patches are visited in; a second
/// round settles specks whose neighbours were all specks themselves.
fn absorb_specks(level: &Level, biome: &mut [u8], least: usize) {
    let w = level.w;
    let n = w * w;
    let keep = |b: u8| b == Biome::Alpine as u8 || b == Biome::AlpineMeadow as u8 || b == Biome::Volcanic as u8;
    // The cells beside `i` that lie on the grid.
    let around = |i: usize| {
        let (x, z) = (i % w, i / w);
        [(x > 0, i.wrapping_sub(1)), (x + 1 < w, i + 1), (z > 0, i.wrapping_sub(w)), (z + 1 < w, i + w)]
            .into_iter()
            .filter_map(|(inside, j)| inside.then_some(j))
    };
    let mut seen = vec![false; n];
    let mut cells: Vec<usize> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    for _ in 0..2 {
        seen.fill(false);
        let mut changes: Vec<(usize, u8)> = Vec::new();
        for start in 0..n {
            if level.ocean[start] || seen[start] {
                continue;
            }
            let b = biome[start];
            cells.clear();
            seen[start] = true;
            stack.push(start);
            let mut ring = false;
            while let Some(i) = stack.pop() {
                cells.push(i);
                let (x, z) = (i % w, i / w);
                ring |= x == 0 || z == 0 || x == w - 1 || z == w - 1;
                for j in around(i) {
                    if !level.ocean[j] && !seen[j] && biome[j] == b {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
            if ring || keep(b) || cells.len() >= least {
                continue;
            }
            // The biome along the most of its edge.
            let mut edge = [0u32; Biome::ALL.len()];
            for &i in &cells {
                for j in around(i) {
                    if !level.ocean[j] && biome[j] != b {
                        edge[biome[j] as usize] += 1;
                    }
                }
            }
            let mut best = None;
            for (k, &count) in edge.iter().enumerate() {
                if count > 0 && best.is_none_or(|(_, c)| count > c) {
                    best = Some((k, count));
                }
            }
            if let Some((k, _)) = best {
                changes.extend(cells.iter().map(|&i| (i, k as u8)));
            }
        }
        if changes.is_empty() {
            break;
        }
        for (i, b) in changes {
            biome[i] = b;
        }
    }
}

/// Heaps the sand of a dune sea into ridges across the wind, once drainage is settled.
///
/// Dunes come last because their hollows would otherwise fill as lakes, and a dune sea
/// has none. Streams keep their beds; the sand is heaped only on ground no stream runs
/// over, and the water level of whatever drains onto it rises with it so water still
/// only runs downhill.
#[allow(clippy::too_many_arguments)]
pub(super) fn dunes(
    level: &Level,
    shape: &Shape,
    flow: &Flow,
    biome: &[u8],
    held: &[bool],
    lake: &[f32],
    height: &mut [f32],
    water_level: &mut [f32],
) {
    let desc = shape.desc();
    let w = level.w;
    let metres = level.dx * 0.8;
    let (cos, sin) = (desc.wind.cos() as f64, desc.wind.sin() as f64);
    // Ridges about 170 m apart, but never finer than the grid can draw.
    let wavelength = (210.0f32).max(level.dx * 4.0) as f64;
    let span = super::SIM_SPAN as f64;
    // Sand only where the whole neighbourhood is dune sea, so it eases in at the edges.
    let mut sand: Vec<f32> = biome.iter().map(|&b| if b == Biome::DuneSea as u8 { 1.0 } else { 0.0 }).collect();
    let mut not: Vec<f32> = sand.iter().map(|s| 1.0 - s).collect();
    spread(w, &mut not, 0.25);
    for i in 0..w * w {
        sand[i] *= 1.0 - not[i];
    }
    let mut raised = false;
    for z in 0..w {
        for x in 0..w {
            let i = z * w + x;
            if sand[i] <= 0.0 || level.ocean[i] || held[i] || lake[i] != NO_LAKE {
                continue;
            }
            // Streams keep their beds: sand thins out toward them.
            let km2 = flow.discharge[i] * metres * metres / 1.0e6;
            let dry = (1.0 - km2 / 0.3).clamp(0.0, 1.0);
            if dry <= 0.0 {
                continue;
            }
            let (px, pz) = level.norm(x, z);
            // World position in sim units, so dunes run on between cells.
            let (gx, gz) = (
                desc.mx as f64 * span + (px as f64 + 1.0) * 0.5 * span,
                desc.mz as f64 * span + (pz as f64 + 1.0) * 0.5 * span,
            );
            let along = (gx * cos + gz * sin) / wavelength;
            let across = (-gx * sin + gz * cos) / wavelength;
            // Crests wander and break up along their length.
            let warp = fbm2(desc.world_seed ^ 0xd0e5, along * 0.25, across * 0.25, 2, 2.0, 0.5) as f64 * 1.4;
            let ridge = ridged2(desc.world_seed ^ 0xd0e6, along + warp, across * 0.18, 2, 2.0, 0.5);
            let swell = fbm2(desc.world_seed ^ 0xd0e7, along * 0.08, across * 0.08, 2, 2.0, 0.5) * 0.5 + 0.7;
            height[i] += sand[i] * dry * ridge * ridge * 36.0 * swell;
            raised = true;
        }
    }
    if !raised {
        return;
    }
    // Receivers come before their donors in `order`.
    for &i in &flow.order {
        let i = i as usize;
        let r = flow.rcv[i] as usize;
        let below = if level.ocean[r] { 0.0 } else { water_level[r] };
        water_level[i] = water_level[i].max(height[i]).max(below);
    }
}
