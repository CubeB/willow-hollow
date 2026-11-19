//! One macro-cell's landmass, simulated as a whole: tectonic uplift worn down by rivers
//! until valleys, ridges and a complete drainage network emerge.
//!
//! The method is the stream-power landscape evolution model with an implicit solver
//! (Braun & Willett 2013; Cordonnier et al. 2016), run coarse-to-fine so large-scale
//! drainage is settled cheaply before fine detail is added.

mod biomes;
mod border;
pub mod climate;
pub mod flow;
pub mod scarp;
pub mod shape;

use crate::{
    biome::Biome,
    macro_layout::{CellDesc, MACRO_SIZE, SIM_SPAN},
    noise::{hash2, unit},
    structures::{self, Field},
};
pub use scarp::Scarp;
use flow::Flow;
use shape::Shape;

/// Marks cells with no standing water in [`Continent::lake`].
pub const NO_LAKE: f32 = -1.0e9;

#[derive(Clone, Copy, Debug)]
pub struct SimConfig {
    /// Final grid resolution (cells per side). A power of two, at least 128.
    pub size: usize,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self { size: 1024 }
    }
}

pub struct Continent {
    pub desc: CellDesc,
    /// Cells per side.
    pub size: usize,
    /// Terrain surface in simulation units (0.8 m each); sea level is 0. Under a lake
    /// this is the lake bed. Heights, lakes and river widths stay in these units here
    /// and are converted to voxels by the surface sampler.
    pub height: Vec<f32>,
    /// Lake surface level where a cell holds standing water, else [`NO_LAKE`].
    pub lake: Vec<f32>,
    /// Level that water flowing through a land cell has: roughly the terrain height,
    /// the spill level inside a depression, or the flat surface of a lake. It never
    /// rises along a flow path, so rivers built from it cannot run uphill. 0 for ocean.
    pub water_level: Vec<f32>,
    /// Index of the cell each land cell drains into; ocean cells point to themselves.
    pub rcv: Vec<u32>,
    /// Water carried through each cell, in units of one cell's rainfall at rain = 1.
    pub discharge: Vec<f32>,
    /// Largest value in `discharge`.
    pub max_discharge: f32,
    pub ocean: Vec<bool>,
    /// Rainfall relative to the continent average.
    pub rain: Vec<f32>,
    /// 0 = polar, 1 = tropical, already lowered for altitude.
    pub temperature: Vec<f32>,
    /// [`Biome`] as u8.
    pub biome: Vec<u8>,
    /// Trails and landmarks. Filled in after simulation, since building them needs the
    /// finished voxel surface; empty for a continent fresh from [`generate`].
    pub trails: crate::trails::Network,
    /// Fault scarps, for drawing each step as a sharp face (see [`Continent::scarp`]).
    pub scarps: scarp::Scarps,
    /// How managed each spot is, about the settlements (see
    /// [`crate::settlements::farmland`]). Worked out after the trails; empty for a
    /// continent fresh from [`generate`].
    pub land: crate::settlements::farmland::Land,
}

impl Continent {
    /// Voxels per grid cell.
    pub fn cell_size(&self) -> f32 {
        MACRO_SIZE as f32 / self.size as f32
    }

    /// Simulation units per grid cell.
    pub fn sim_cell_size(&self) -> f32 {
        SIM_SPAN / self.size as f32
    }

    /// River width in simulation units for a given discharge.
    pub fn river_width(discharge: f32) -> f32 {
        0.45 * discharge.powf(0.36)
    }
}

// --- Simulation parameters -------------------------------------------------------------
// Lengths are simulation units. Only the products K*dt and U*dt matter to the implicit scheme.

/// Erodibility x timestep.
const K_DT: f32 = 12.0;
/// Uplift x timestep for an uplift value of 1, expressed through the ratio U/K, which
/// sets how steep the equilibrium landscape is.
const U_OVER_K: f32 = 1.6;
/// Drainage-area exponent of the stream power law.
const M_EXP: f32 = 0.45;
/// Iterations at each resolution, coarsest first; the last entry is the final grid.
const ITERATIONS: [u32; 4] = [40, 40, 28, 16];
/// Discharge at which a river starts to noticeably widen its valley floor.
const VALLEY_Q: f32 = 400.0;

struct Level {
    w: usize,
    /// Simulation units per cell.
    dx: f32,
    land: Vec<f32>,
    ocean: Vec<bool>,
    uplift: Vec<f32>,
    erodibility: Vec<f32>,
}

impl Level {
    fn new(shape: &Shape, w: usize) -> Self {
        let n = w * w;
        let mut l = Level {
            w,
            dx: SIM_SPAN / w as f32,
            land: vec![0.0; n],
            ocean: vec![false; n],
            uplift: vec![0.0; n],
            erodibility: vec![1.0; n],
        };
        for z in 0..w {
            for x in 0..w {
                let (px, pz) = l.norm(x, z);
                let i = z * w + x;
                let land = shape.land(px, pz);
                l.land[i] = land;
                // The outer ring is forced to ocean so land cells always have 8 neighbours.
                l.ocean[i] = land <= 0.0 || x == 0 || z == 0 || x == w - 1 || z == w - 1;
                if !l.ocean[i] {
                    l.uplift[i] = shape.uplift(px, pz, land);
                    l.erodibility[i] = shape.erodibility(px, pz);
                }
            }
        }
        l
    }

    /// Cell centre in cell-normalised coordinates (-1..1).
    fn norm(&self, x: usize, z: usize) -> (f32, f32) {
        norm(self.w, x, z)
    }
}

/// Cell-normalised position (-1..1) of cell `(x, z)` on a `w`-wide grid. The outer ring
/// is placed on the border itself, not the cell centre: that is what lets the cells
/// either side of a border compute exactly the same values there.
fn norm(w: usize, x: usize, z: usize) -> (f32, f32) {
    let f = 2.0 / w as f32;
    let snap = |i: usize| match i {
        0 => -1.0,
        i if i == w - 1 => 1.0,
        i => (i as f32 + 0.5) * f - 1.0,
    };
    (snap(x), snap(z))
}

fn sea_floor(land: f32) -> f32 {
    // A shelf that deepens away from the coast.
    -1.0 - 79.0 * (-land / 0.2).clamp(0.0, 1.0).powf(1.5)
}

/// Bilinear 2x refinement of a cell-centred grid.
fn upsample(src: &[f32], w: usize) -> Vec<f32> {
    let w2 = w * 2;
    let mut out = vec![0.0; w2 * w2];
    for z in 0..w2 {
        let fz = ((z as f32 + 0.5) * 0.5 - 0.5).clamp(0.0, (w - 1) as f32);
        let (z0, tz) = (fz as usize, fz.fract());
        let z1 = (z0 + 1).min(w - 1);
        for x in 0..w2 {
            let fx = ((x as f32 + 0.5) * 0.5 - 0.5).clamp(0.0, (w - 1) as f32);
            let (x0, tx) = (fx as usize, fx.fract());
            let x1 = (x0 + 1).min(w - 1);
            let a = src[z0 * w + x0] + (src[z0 * w + x1] - src[z0 * w + x0]) * tx;
            let b = src[z1 * w + x0] + (src[z1 * w + x1] - src[z1 * w + x0]) * tx;
            out[z * w2 + x] = a + (b - a) * tz;
        }
    }
    out
}

/// Mean of `src` over the square of radius `r` cells around each cell of a `w`-wide
/// grid, clipped at the grid's edges. Two running-sum passes, so any radius costs the
/// same.
fn box_mean(src: &[f32], w: usize, r: usize) -> Vec<f32> {
    let mut tmp = vec![0.0f32; w * w];
    let mut out = vec![0.0f32; w * w];
    // One pass along x (rows) or z (columns) from `from` into `to`.
    let pass = |from: &[f32], to: &mut [f32], along_x: bool| {
        let idx = |line: usize, k: usize| if along_x { line * w + k } else { k * w + line };
        for line in 0..w {
            let mut sum = 0.0f64;
            for k in 0..=r.min(w - 1) {
                sum += from[idx(line, k)] as f64;
            }
            for k in 0..w {
                let (lo, hi) = (k.saturating_sub(r), (k + r).min(w - 1));
                to[idx(line, k)] = (sum / (hi - lo + 1) as f64) as f32;
                if k + r + 1 < w {
                    sum += from[idx(line, k + r + 1)] as f64;
                }
                if k >= r {
                    sum -= from[idx(line, k - r)] as f64;
                }
            }
        }
    };
    pass(src, &mut tmp, true);
    pass(&tmp, &mut out, false);
    out
}

/// Working buffers for [`erode`], reused across iterations.
#[derive(Default)]
struct Scratch {
    copy: Vec<f32>,
    /// Discharge of the nearest large river a cell's water is heading for, decaying
    /// with each step away from it. Drives valley widening.
    valley: Vec<f32>,
}

/// One implicit erosion step over a routed grid.
fn erode(level: &Level, flow: &Flow, cell_area: f32, height: &mut [f32], s: &mut Scratch) {
    s.valley.clear();
    s.valley.resize(height.len(), 0.0);
    for &i in &flow.order {
        let i = i as usize;
        let r = flow.rcv[i] as usize;
        let dist = flow.rcv_dist[i] * level.dx;
        let down = height[r];
        let q = flow.discharge[i];
        // Stream power: dh/dt = U - K * A^m * slope, solved implicitly for this cell
        // given its already-updated receiver. Unconditionally stable for any timestep.
        let f = K_DT * level.erodibility[i] * q.powf(M_EXP) / dist;
        let mut h = (height[i] + K_DT * U_OVER_K * level.uplift[i] + f * down) / (1.0 + f);

        // Slopes steeper than the rock can hold collapse (thermal erosion). Hard rock
        // stands as cliffs; soft rock slumps to gentle hillsides.
        let mut max_slope = (0.75 / level.erodibility[i].powf(0.7)).clamp(0.45, 2.2);
        // Big rivers also cut sideways. A cell draining into a much larger stream is a
        // valley side, and is held to a gentle slope that steepens with distance from
        // the river: this is what gives large rivers broad floors instead of gorges.
        let side = !level.ocean[r] && q < 0.4 * flow.discharge[r];
        if side {
            max_slope /= 1.0 + (s.valley[r] / VALLEY_Q).sqrt();
            s.valley[i] = q.max(s.valley[r] * 0.5);
        } else {
            s.valley[i] = q;
        }
        h = h.min(down + max_slope * dist);
        height[i] = h;
    }

    // Hillslope diffusion rounds ridges and spurs. It fades out where water gathers so
    // it never fills in the channels the rivers have cut.
    s.copy.clear();
    s.copy.extend_from_slice(height);
    let w = level.w as isize;
    for &i in &flow.order {
        let i = i as usize;
        let ii = i as isize;
        let mean = (s.copy[(ii - 1) as usize]
            + s.copy[(ii + 1) as usize]
            + s.copy[(ii - w) as usize]
            + s.copy[(ii + w) as usize])
            * 0.25;
        let cells = flow.discharge[i] / cell_area;
        let rate = 0.30 / (1.0 + cells * cells * 0.06);
        height[i] += (mean - s.copy[i]) * rate;
    }
}

pub fn generate(desc: CellDesc, cfg: SimConfig) -> Continent {
    assert!(cfg.size.is_power_of_two() && cfg.size >= 128);
    let shape = Shape::new(desc);

    // Resolutions from coarse to fine, ending at the requested size.
    let levels = ITERATIONS.len().min((cfg.size / 64).trailing_zeros() as usize);
    let sizes: Vec<usize> = (0..levels).map(|l| cfg.size >> (levels - 1 - l)).collect();
    let iterations = &ITERATIONS[ITERATIONS.len() - levels..];

    let mut height: Vec<f32> = Vec::new();
    let mut scratch = Scratch::default();
    let mut last: Option<(Level, Flow, Vec<f32>, Vec<border::Pin>)> = None;
    // Rainfall, computed once the coarse terrain has taken shape and refined with it.
    let mut rain_coarse: Option<Vec<f32>> = None;
    let mut scarps = scarp::Scarps::default();

    for (li, &w) in sizes.iter().enumerate() {
        let level = Level::new(&shape, w);
        let n = w * w;
        // Open sea needs no simulation, unless a range reaches this cell's border: its
        // neighbour then holds land there, and so must this cell, however little.
        if li == 0 && level.ocean.iter().all(|&o| o) && !border::touches(&shape, cfg.size) {
            return open_sea(desc, cfg.size, &shape);
        }
        if li == 0 {
            height = vec![0.0; n];
            for z in 0..w {
                for x in 0..w {
                    let i = z * w + x;
                    let (px, pz) = level.norm(x, z);
                    // A gentle dome so drainage heads for the coast, under irregular
                    // relief so it does not run there in straight lines.
                    height[i] = 1.0 + level.land[i] * 40.0 + shape.initial_relief(px, pz);
                }
            }
        } else {
            height = upsample(&height, w / 2);
            for z in 0..w {
                for x in 0..w {
                    let i = z * w + x;
                    let (px, pz) = level.norm(x, z);
                    // New relief at the scale this level can newly resolve.
                    height[i] += shape.detail(px, pz, li as u32) * level.dx * 0.06;
                }
            }
        }
        for i in 0..n {
            height[i] = if level.ocean[i] {
                sea_floor(level.land[i].min(0.0))
            } else {
                height[i].max(0.3)
            };
        }
        let pins = border::pins(&level, &shape);
        border::hold(&level, &pins, &mut height);

        let mut rain = match rain_coarse.take() {
            Some(coarse) => upsample(&coarse, w / 2),
            None => vec![desc.wetness; n],
        };
        let cell_area = ((cfg.size / w) * (cfg.size / w)) as f32;
        let mut flow = Flow::new(w, desc.seed);
        for _ in 0..iterations[li] {
            flow.outlets = border::outlets(&level, &pins, &height);
            flow.route(&height, &level.ocean, &rain, cell_area);
            erode(&level, &flow, cell_area, &mut height, &mut scratch);
            border::hold(&level, &pins, &mut height);
        }
        if li == sizes.len() - 1 {
            // Fault steps and rift troughs go in last: erosion would wear them away.
            scarps = displace(&level, &shape, &mut height);
            border::settle(&level, &shape, &pins, &mut height);
        }
        // Route once more so the stored drainage matches the final terrain exactly.
        flow.outlets = border::outlets(&level, &pins, &height);
        flow.route(&height, &level.ocean, &rain, cell_area);
        if li == sizes.len() - 1 {
            dam_lakes(&level, &flow, &mut height, desc.seed);
        }
        if li >= 1 || sizes.len() == 1 {
            // From here on the wet side of each range carries more water.
            rain = climate::rainfall(w, &height, &level.ocean, &desc);
            flow.outlets = border::outlets(&level, &pins, &height);
            flow.route(&height, &level.ocean, &rain, cell_area);
        }
        rain_coarse = Some(rain.clone());
        last = Some((level, flow, rain, pins));
    }

    let (mut level, mut flow, mut rain, pins) = last.expect("at least one level");
    border::dry(&level, &flow, &pins, &mut height);
    let mut lake = find_lakes(&level, &flow, &mut height);
    // Flatten the water level across each lake, then make sure nothing downstream sits
    // above what feeds it (a lake's surface is its lowest point, so an outlet cell can
    // otherwise end up a hair higher than the lake).
    let mut water_level = flow.filled.clone();
    for i in 0..water_level.len() {
        if lake[i] != NO_LAKE {
            water_level[i] = lake[i];
        }
    }
    for &i in flow.order.iter().rev() {
        let r = flow.rcv[i as usize] as usize;
        if !level.ocean[r] && water_level[r] > water_level[i as usize] {
            water_level[r] = water_level[i as usize];
        }
    }
    border::finish(&mut level, &mut flow, &mut height, &mut water_level, &mut lake, &rain, &pins);
    let mut temperature = climate::temperature(level.w, &height, &desc);
    border::blend_climate(&level, &pins, &shape, &height, &mut temperature, &mut rain);
    let biome = biomes::classify(&level, &shape, &flow, &height, &lake, &temperature, &rain);
    let mut held = vec![false; height.len()];
    for p in &pins {
        held[p.i as usize] = true;
    }
    biomes::dunes(&level, &shape, &flow, &biome, &held, &lake, &mut height, &mut water_level);
    Continent {
        desc,
        size: cfg.size,
        height,
        lake,
        water_level,
        rcv: flow.rcv,
        max_discharge: (0..flow.discharge.len())
            .filter(|&i| !level.ocean[i])
            .map(|i| flow.discharge[i])
            .fold(0.0, f32::max),
        discharge: flow.discharge,
        ocean: level.ocean,
        rain,
        temperature,
        biome,
        trails: Default::default(),
        scarps,
        land: Default::default(),
    }
}

/// Raises and lowers the finished land along faults and rifts, fading to nothing where
/// the land meets a border band or the coast. Returns what the surface needs to draw
/// each fault's step as a face, which the grid is too coarse to hold.
fn displace(level: &Level, shape: &Shape, height: &mut [f32]) -> scarp::Scarps {
    let mut scarps = scarp::Scarps::default();
    if shape.field.is_empty() {
        return scarps;
    }
    // Steps narrower than this the grid would draw as a lone notch, not a slope.
    let soft = structures::FAULT_SCARP.max(level.dx * 1.5);
    // Grid cells whose fault offset the surface reads back, either side of the line.
    // Beyond the reach of the surface's 4x4 interpolation from any column it changes.
    let near = soft + level.dx * 4.0;
    for z in 0..level.w {
        for x in 0..level.w {
            let i = z * level.w + x;
            if level.ocean[i] {
                continue;
            }
            let (px, pz) = level.norm(x, z);
            let o = shape.field.offset(px, pz, level.land[i], soft);
            if o.total == 0.0 && o.nearest >= near {
                continue;
            }
            let keep = 1.0 - Field::border_hold(px.abs().max(pz.abs()));
            // A cut never takes the land below a share of its old height, so a rift
            // floor near the coast keeps its relief instead of flattening at the sea.
            if o.total != 0.0 {
                let floor = height[i] * 0.3 + 0.3;
                height[i] = (height[i] + o.total * keep).max(floor);
            }
            if o.nearest < near && o.step * keep > 0.0 {
                // The floor only ever holds back a rift's deep cut, so the fault's share
                // is as computed.
                scarps.note(i as u32, o.fault * keep, o.step * keep);
            }
        }
    }
    if !scarps.is_empty() {
        let d = shape.desc();
        scarps.finish(&shape.field, d.mx, d.mz, level.w, soft);
    }
    scarps
}

/// A cell with no land: just the sea floor.
fn open_sea(desc: CellDesc, size: usize, shape: &Shape) -> Continent {
    let n = size * size;
    let height = (0..n)
        .map(|i| {
            let (px, pz) = norm(size, i % size, i / size);
            sea_floor(shape.land(px, pz).min(0.0))
        })
        .collect();
    Continent {
        desc,
        size,
        height,
        lake: vec![NO_LAKE; n],
        water_level: vec![0.0; n],
        rcv: (0..n as u32).collect(),
        discharge: vec![0.0; n],
        max_discharge: 0.0,
        ocean: vec![true; n],
        rain: vec![desc.wetness; n],
        temperature: vec![desc.temperature; n],
        biome: vec![Biome::Ocean as u8; n],
        trails: Default::default(),
        scarps: Default::default(),
        land: Default::default(),
    }
}

/// Rivers alone erase every lake, so some are put back the way nature often makes
/// them: a barrier (moraine, landslide, lava) across a valley. Each dam raises a low
/// ridge with a spillway at the old channel; routing afterwards floods the valley
/// behind it to the spillway level, and the river leaves over the barrier as a fall.
fn dam_lakes(level: &Level, flow: &Flow, height: &mut [f32], seed: u32) {
    let w = level.w;
    let scale = w as f32 / 1024.0; // distances below are tuned for the full-size grid

    // Candidate sites: gentle reaches of streams and rivers, away from the coast.
    let mut sites: Vec<(u32, usize, f32, bool)> = Vec::new();
    for &i in &flow.order {
        let i = i as usize;
        let q = flow.discharge[i];
        let r = flow.rcv[i] as usize;
        if !(60.0..=40_000.0).contains(&q) || level.ocean[r] || height[i] < 6.0 {
            continue;
        }
        let slope = (height[i] - height[r]) / (flow.rcv_dist[i] * level.dx);
        let roll = hash2(seed ^ 0x1a4e, i as i32, 3);
        // Two kinds: long lakes in the broad valleys of real rivers, and ponds and
        // tarns on upland streams.
        let valley = q >= 1500.0 && slope < 0.02;
        let chance = if valley { 0.05 } else { 0.02 };
        if (0.0..0.08).contains(&slope) && unit(roll) < chance {
            sites.push((roll, i, slope, valley));
        }
    }
    sites.sort_unstable_by_key(|s| s.0);

    let mut placed: Vec<(f32, f32)> = Vec::new();
    for (roll, i, slope, valley) in sites {
        let (x, z) = ((i % w) as f32, (i / w) as f32);
        let spacing = (if valley { 80.0 } else { 50.0 } * scale).max(8.0);
        // The barrier must span the valley floor, which is wide for big rivers.
        let radius = (if valley { 7.0 } else { 3.5 } * scale).max(1.5);
        if placed.iter().any(|&(px, pz)| (px - x).hypot(pz - z) < spacing) {
            continue;
        }
        placed.push((x, z));
        // Rise chosen so the lake backs up from a few hundred voxels to a couple of
        // thousand.
        let t = unit(roll.rotate_left(13));
        let length = if valley { 900.0 + 1800.0 * t } else { 500.0 + 900.0 * t };
        let rise = (slope * length).clamp(3.0, 14.0);
        let crest = height[i] + rise;
        let r = radius.ceil() as i32;
        for dz in -r..=r {
            for dx in -r..=r {
                let d = ((dx * dx + dz * dz) as f32).sqrt();
                // A barrier by the edge of the grid stops at it: it neither reaches off
                // the top or bottom nor wraps round onto the far side.
                let (cx, cz) = (x as i32 + dx, z as i32 + dz);
                if d > radius || cx < 0 || cz < 0 || cx >= w as i32 || cz as usize >= height.len() / w {
                    continue;
                }
                let j = (cz * w as i32 + cx) as usize;
                if !level.ocean[j] && height[j] < crest + 1.5 {
                    height[j] = crest + 1.5;
                }
            }
        }
        height[i] = crest; // the spillway
    }
}

/// Labels each connected body of standing water left by routing with one flat surface
/// at its outlet level, and deepens the beds so lakes are more than puddles.
fn find_lakes(level: &Level, flow: &Flow, height: &mut [f32]) -> Vec<f32> {
    /// Depth at which a cell counts as part of a body of water. Kept small so the
    /// whole depression shares the lake's level, shallow margins included.
    const WET: f32 = 0.25;
    /// Depth that counts toward a body being a lake rather than a damp hollow.
    const DEEP: f32 = 1.5;
    let min_cells = (6 * level.w * level.w / (1024 * 1024)).max(3);
    let n = level.w * level.w;
    let mut lake = vec![NO_LAKE; n];
    let wet = |i: usize, height: &[f32]| !level.ocean[i] && flow.filled[i] - height[i] > WET;
    let mut seen = vec![false; n];
    let mut body = Vec::new();
    for start in 0..n {
        if seen[start] || !wet(start, height) {
            continue;
        }
        body.clear();
        body.push(start);
        seen[start] = true;
        let mut surface = f32::MAX;
        let mut next = 0;
        while next < body.len() {
            let i = body[next];
            next += 1;
            surface = surface.min(flow.filled[i]);
            for &o in &flow.offsets[..4] {
                let j = (i as isize + o) as usize;
                if !seen[j] && wet(j, height) {
                    seen[j] = true;
                    body.push(j);
                }
            }
        }
        let deep = body.iter().filter(|&&i| flow.filled[i] - height[i] > DEEP).count();
        if deep >= min_cells {
            for &i in &body {
                lake[i] = surface;
                // Deepen in proportion to existing depth: shores stay shallow.
                height[i] -= (surface - height[i]).max(0.0) * 0.8;
            }
        }
    }
    lake
}

#[cfg(test)]
mod tests;
