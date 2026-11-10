//! Geological mega-structures: rare features far larger than one macro-cell.
//!
//! Each macro-cell simulates its own landmass, so anything spanning several cells has to
//! be decided above them. This is that layer: a pure function of the world seed and a
//! world position, which every cell consults and so agrees with its neighbours on.
//!
//! Structures are curves in macro-cell units (one cell = 1.0), from a jittered grid
//! of super-cells coarse enough that the largest fits within a few of them:
//! - **Ranges**: long belts of uplift with a spine of land, which can carry land right
//!   across a cell border.
//! - **Faults**: one side raised by tens of voxels along a line far longer than a cell.
//! - **Rifts**: troughs and narrow ravines cut across whatever they meet.
//! - **Vast oceans**: discs many cells wide with no land at all.
//!
//! Faults, rifts and the oceans only ever change land already inside a cell, so they
//! need no agreement at borders beyond sharing a position. Ranges do reach borders. That
//! does not break the cell's bounded drainage, because the land there is pinned to a
//! height shared with the neighbour and always rises toward the border, which makes
//! every border crossing a drainage divide (see `continent::border`).

use crate::{
    macro_layout::{SIM_SPAN},
    noise::{fbm2, hash2, perlin2, ridged2, unit},
};

/// Macro-cells per side of a structure super-cell.
const SUPER: i32 = 8;
/// Spacing of the points along a structure's curve, in macro-cells.
const STEP: f64 = 0.125;
/// Longest possible structure from its anchor to one end, in macro-cells.
const MAX_HALF_LENGTH: f64 = 16.0;
/// Macro-cells per side of a vast-ocean super-cell.
const OCEAN_SUPER: i32 = 24;
/// Largest vast-ocean radius in macro-cells, including its wobble.
const MAX_OCEAN_RADIUS: f64 = 15.0;
/// No vast ocean comes within this many macro-cells of the origin's cell.
const OCEAN_ORIGIN_CLEAR: f64 = 4.0;
/// Share of the cell's half-width inside which border land is held to its shared height.
pub const BORDER_BAND: f32 = 0.10;
/// How high a range's spine stands where it crosses a border, in simulation units.
const BORDER_CREST: f32 = 680.0;
/// Sim units in one macro-cell.
const CELL: f64 = SIM_SPAN as f64;

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Range,
    Fault,
    Rift,
}

/// One structure's curve, already clipped to the part near the cell that asked for it.
#[derive(Clone, Debug)]
pub struct Structure {
    pub kind: Kind,
    pub seed: u32,
    /// Points along the curve, world position in macro-cells.
    pub points: Vec<(f64, f64)>,
    /// Index in the full curve of `points[0]`, so distance along it survives clipping.
    first: usize,
    /// Whole length of the curve in macro-cells.
    pub length: f64,
    /// Half-width in macro-cells: of the uplift belt, or the rift.
    pub half_width: f64,
    /// Range: uplift rate. Fault: height of the step, negative raising the other side.
    /// Rift: depth. Both of the latter in simulation units.
    pub amplitude: f32,
    /// Farthest from the curve this structure has any effect, in macro-cells.
    reach: f64,
    /// Bounds of `points` grown by `reach`.
    lo: (f64, f64),
    hi: (f64, f64),
}

/// Where a position lies relative to a structure.
#[derive(Clone, Copy, Debug)]
struct Hit {
    /// Distance from the curve, in macro-cells.
    dist: f64,
    /// Which side: positive left of the direction of travel.
    side: f64,
    /// Distance along the curve from its start, in macro-cells.
    along: f64,
}

impl Structure {
    fn hit(&self, wx: f64, wz: f64) -> Option<Hit> {
        if wx < self.lo.0 || wz < self.lo.1 || wx > self.hi.0 || wz > self.hi.1 {
            return None;
        }
        let mut best = (f64::MAX, 0.0, 0.0);
        for k in 0..self.points.len() - 1 {
            let (a, b) = (self.points[k], self.points[k + 1]);
            let (abx, abz) = (b.0 - a.0, b.1 - a.1);
            let (apx, apz) = (wx - a.0, wz - a.1);
            let t = ((apx * abx + apz * abz) / (abx * abx + abz * abz).max(1e-12)).clamp(0.0, 1.0);
            let (dx, dz) = (apx - abx * t, apz - abz * t);
            let d2 = dx * dx + dz * dz;
            if d2 < best.0 {
                best = (d2, abx * apz - abz * apx, (self.first + k) as f64 + t);
            }
        }
        let dist = best.0.sqrt();
        (dist <= self.reach).then_some(Hit {
            dist,
            side: best.1,
            along: best.2 * STEP,
        })
    }

    /// Strength along the curve: fading in over the ends.
    fn taper(&self, along: f64) -> f32 {
        smoothstep(0.0, 1.5, along.min(self.length - along) as f32)
    }

    /// Height profile along a range: separate massifs instead of a uniform wall.
    fn massif(&self, along: f64) -> f32 {
        (0.78 + 0.42 * perlin2(self.seed ^ 1, along * 0.38, 0.5) + 0.18 * perlin2(self.seed ^ 2, along * 1.4, 0.5))
            .clamp(0.30, 1.25)
    }

    /// Half-width of a range's dry spine; negative where it breaks into sea.
    fn spine(&self, along: f64) -> f32 {
        self.half_width as f32 * (0.46 + 0.62 * perlin2(self.seed ^ 3, along * 0.75, 0.5)) * self.taper(along)
    }

    /// Signed distance across a fault, positive on the raised side, with a ragged line.
    fn fault_side(&self, wx: f64, wz: f64, h: &Hit) -> f32 {
        let sign = (h.side >= 0.0) == (self.amplitude > 0.0);
        let d = if sign { h.dist } else { -h.dist };
        (d + fbm2(self.seed ^ 5, wx * 70.0 + 0.13, wz * 70.0 + 0.61, 3, 2.07, 0.5) as f64 * 0.0012) as f32
    }
}

/// One candidate structure, if this slot of the super-cell holds one.
fn roll_structure(world_seed: u32, sx: i32, sz: i32, slot: i32) -> Option<Structure> {
    let seed = hash2(world_seed ^ 0x4d3a_c7f1, sx * 17 + slot, sz.wrapping_mul(31) ^ slot);
    let r = |k: i32| unit(hash2(seed, k, 0x6e0c));
    let kind = match (slot, r(0)) {
        (0, k) if k < 0.28 => Kind::Range,
        (1, k) if k < 0.34 => Kind::Fault,
        (1, k) if k < 0.56 => Kind::Rift,
        _ => return None,
    };
    let (length, half_width, amplitude, wiggle, reach) = match kind {
        Kind::Range => {
            let length = 8.0 + r(1) as f64 * 20.0;
            let hw = 0.24 + r(2) as f64 * 0.20;
            let amplitude = 0.45 + r(3) * 0.35;
            (length, hw, amplitude, 1.0, hw * 2.4 + 0.1)
        }
        Kind::Fault => {
            let length = 12.0 + r(1) as f64 * 20.0;
            let amplitude = (28.0 + r(3) * 36.0) * if r(2) < 0.5 { 1.0 } else { -1.0 };
            (length, 0.0, amplitude, 1.0, FAULT_FALLOFF as f64)
        }
        Kind::Rift => {
            let length = 6.0 + r(1) as f64 * 12.0;
            // About one in three is a narrow ravine, the rest wide troughs.
            let narrow = r(4) < 0.35;
            let hw = if narrow { 0.014 + r(2) as f64 * 0.016 } else { 0.05 + r(2) as f64 * 0.07 };
            let amplitude = if narrow { 45.0 + r(3) * 45.0 } else { 34.0 + r(3) * 60.0 };
            (length, hw, amplitude, 0.7, hw * 2.0 + 0.05)
        }
    };
    let anchor = (
        (sx as f64 + r(5) as f64) * SUPER as f64,
        (sz as f64 + r(6) as f64) * SUPER as f64,
    );
    let heading = r(7) as f64 * std::f64::consts::TAU;

    // Walk out from the anchor both ways, the heading drifting slowly.
    let half = ((length * 0.5).min(MAX_HALF_LENGTH) / STEP) as i32;
    let mut points = Vec::with_capacity(2 * half as usize + 1);
    for dir in [-1.0f64, 1.0] {
        let mut p = anchor;
        let mut side = Vec::with_capacity(half as usize);
        for k in 1..=half {
            let s = k as f64 * STEP * dir;
            let turn = perlin2(seed ^ 7, s * 0.09, 0.5) as f64 * 0.7 + perlin2(seed ^ 8, s * 0.31, 0.5) as f64 * 0.25;
            let h = heading + turn;
            p = (p.0 + h.cos() * STEP * dir, p.1 + h.sin() * STEP * dir);
            side.push(p);
        }
        if dir < 0.0 {
            points.extend(side.into_iter().rev());
            points.push(anchor);
        } else {
            points.extend(side);
        }
    }
    // Sideways wander on a scale the turning cannot produce.
    let n = points.len();
    let base = points.clone();
    for k in 1..n - 1 {
        let (a, b) = (base[k - 1], base[k + 1]);
        let (tx, tz) = (b.0 - a.0, b.1 - a.1);
        let len = (tx * tx + tz * tz).sqrt().max(1e-9);
        let s = k as f64 * STEP;
        let off = (perlin2(seed ^ 9, s * 0.7, 0.5) as f64 * 0.22 + perlin2(seed ^ 10, s * 1.9, 0.5) as f64 * 0.07) * wiggle;
        points[k] = (points[k].0 - tz / len * off, points[k].1 + tx / len * off);
    }
    Some(Structure {
        kind,
        seed,
        points,
        first: 0,
        length,
        half_width,
        amplitude,
        reach,
        lo: (0.0, 0.0),
        hi: (0.0, 0.0),
    })
}

/// How far from a fault its raised block still lifts the land, in macro-cells.
const FAULT_FALLOFF: f32 = 1.1;

/// Keeps only the part of `s` that can touch the macro-cell, or `None` if none can.
fn clip(mut s: Structure, mx: i32, mz: i32) -> Option<Structure> {
    let (x0, z0) = (mx as f64 - s.reach, mz as f64 - s.reach);
    let (x1, z1) = (mx as f64 + 1.0 + s.reach, mz as f64 + 1.0 + s.reach);
    let inside = |p: &(f64, f64)| p.0 >= x0 && p.0 <= x1 && p.1 >= z0 && p.1 <= z1;
    let first = s.points.iter().position(inside)?;
    let last = s.points.iter().rposition(inside)?;
    let (a, b) = (first.saturating_sub(1), (last + 2).min(s.points.len()));
    s.points = s.points[a..b].to_vec();
    s.first = a;
    s.lo = s.points.iter().fold((f64::MAX, f64::MAX), |m, p| (m.0.min(p.0), m.1.min(p.1)));
    s.hi = s.points.iter().fold((f64::MIN, f64::MIN), |m, p| (m.0.max(p.0), m.1.max(p.1)));
    s.lo = (s.lo.0 - s.reach, s.lo.1 - s.reach);
    s.hi = (s.hi.0 + s.reach, s.hi.1 + s.reach);
    (s.points.len() >= 2).then_some(s)
}

/// A vast ocean: a disc in macro-cells.
#[derive(Clone, Copy, Debug)]
struct Basin {
    x: f64,
    z: f64,
    radius: f64,
    seed: u32,
}

fn roll_basin(world_seed: u32, sx: i32, sz: i32) -> Option<Basin> {
    let seed = hash2(world_seed ^ 0x0ce4_a175, sx, sz);
    let r = |k: i32| unit(hash2(seed, k, 0x0ce4));
    if r(0) >= 0.14 {
        return None;
    }
    let basin = Basin {
        x: (sx as f64 + 0.15 + r(1) as f64 * 0.7) * OCEAN_SUPER as f64,
        z: (sz as f64 + 0.15 + r(2) as f64 * 0.7) * OCEAN_SUPER as f64,
        radius: 6.0 + r(3) as f64 * 7.0,
        seed,
    };
    // Every world starts with land around it.
    let clear = basin.radius + 2.0 + OCEAN_ORIGIN_CLEAR;
    ((basin.x - 0.5).hypot(basin.z - 0.5) >= clear).then_some(basin)
}

impl Basin {
    /// 1 well inside the ocean, falling to 0 at its wandering shore.
    fn depth(&self, wx: f64, wz: f64) -> f32 {
        let d = (wx - self.x).hypot(wz - self.z);
        if d > self.radius + 2.0 {
            return 0.0;
        }
        let edge = self.radius
            + perlin2(self.seed ^ 1, wx * 0.14, wz * 0.14) as f64 * 1.6
            + perlin2(self.seed ^ 2, wx * 0.45, wz * 0.45) as f64 * 0.6;
        smoothstep(0.0, 1.5, (edge - d) as f32)
    }
}

fn basins_near(world_seed: u32, wx: f64, wz: f64, reach: f64) -> Vec<Basin> {
    let (sx, sz) = ((wx / OCEAN_SUPER as f64).floor() as i32, (wz / OCEAN_SUPER as f64).floor() as i32);
    let span = ((MAX_OCEAN_RADIUS + reach) / OCEAN_SUPER as f64).ceil() as i32;
    let mut out = Vec::new();
    for z in sz - span..=sz + span {
        for x in sx - span..=sx + span {
            if let Some(b) = roll_basin(world_seed, x, z) {
                out.push(b);
            }
        }
    }
    out
}

/// True if a macro-cell lies within a vast ocean, so must hold no land of its own.
pub fn in_vast_ocean(world_seed: u32, mx: i32, mz: i32) -> bool {
    let (wx, wz) = (mx as f64 + 0.5, mz as f64 + 0.5);
    basins_near(world_seed, wx, wz, 1.0).iter().any(|b| b.depth(wx, wz) > 0.5)
}

/// Every structure that touches a macro-cell, for that cell to consult.
pub struct Field {
    world_seed: u32,
    mx: i32,
    mz: i32,
    items: Vec<Structure>,
    basins: Vec<Basin>,
}

impl Field {
    pub fn new(world_seed: u32, mx: i32, mz: i32) -> Self {
        let span = ((MAX_HALF_LENGTH + FAULT_FALLOFF as f64) / SUPER as f64).ceil() as i32 + 1;
        let (sx, sz) = (mx.div_euclid(SUPER), mz.div_euclid(SUPER));
        let mut items = Vec::new();
        for z in sz - span..=sz + span {
            for x in sx - span..=sx + span {
                for slot in 0..2 {
                    if let Some(s) = roll_structure(world_seed, x, z, slot).and_then(|s| clip(s, mx, mz)) {
                        items.push(s);
                    }
                }
            }
        }
        let basins = basins_near(world_seed, mx as f64 + 0.5, mz as f64 + 0.5, 3.0)
            .into_iter()
            .filter(|b| (b.x - mx as f64 - 0.5).hypot(b.z - mz as f64 - 0.5) < b.radius + 4.0)
            .collect();
        Self { world_seed, mx, mz, items, basins }
    }

    pub fn structures(&self) -> &[Structure] {
        &self.items
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn world_seed(&self) -> u32 {
        self.world_seed
    }

    /// World position, in macro-cells, of a cell-normalised position (-1..1).
    fn world(&self, px: f32, pz: f32) -> (f64, f64) {
        (self.mx as f64 + (px as f64 + 1.0) * 0.5, self.mz as f64 + (pz as f64 + 1.0) * 0.5)
    }

    fn basin_depth(&self, wx: f64, wz: f64) -> f32 {
        self.basins.iter().map(|b| b.depth(wx, wz)).fold(0.0, f32::max)
    }

    /// Height of a range's spine above the sea at world position `(wx, wz)`, or `None`
    /// off every spine, plus its landness. A pure function of position, so cells either
    /// side of a border read the same value there.
    fn spine_at(&self, wx: f64, wz: f64) -> Option<(f32, f32)> {
        let mut best: Option<(f32, f32)> = None;
        for s in self.items.iter().filter(|s| s.kind == Kind::Range) {
            let Some(hit) = s.hit(wx, wz) else { continue };
            let spine = s.spine(hit.along);
            let coast = fbm2(s.seed ^ 4, wx * 7.0 + 0.29, wz * 7.0 + 0.53, 3, 2.07, 0.5) * 0.045;
            let land = (spine - hit.dist as f32) * 2.0 + coast;
            if best.is_some_and(|b| land <= b.0) {
                continue;
            }
            let ratio = (land / (spine.max(0.01) * 2.0)).clamp(0.0, 1.0);
            // Ridged relief, in world coordinates like everything here, so the held band
            // reads as more of the same mountains rather than a smooth causeway.
            let relief = ridged2(s.seed ^ 13, wx * 24.0 + 0.37, wz * 24.0 + 0.71, 4, 2.07, 0.5);
            let crest = BORDER_CREST * s.amplitude * s.massif(hit.along) * s.taper(hit.along) * ratio.powf(0.7) * (0.3 + 1.1 * relief);
            best = Some((land, crest));
        }
        best
    }

    /// Landness contributed by the structures at a cell-normalised position; `None` far
    /// from any range. Positive is dry land, which may reach the cell border.
    pub fn land(&self, px: f32, pz: f32) -> Option<f32> {
        if self.items.is_empty() {
            return None;
        }
        let (wx, wz) = self.world(px, pz);
        let (land, _) = self.spine_at(wx, wz)?;
        Some(land - self.basin_depth(wx, wz) * 4.0)
    }

    /// Change to the uplift field: what ranges add, and how much a rift's floor is
    /// relieved of the uplift around it (0 = none of it).
    pub fn uplift(&self, px: f32, pz: f32, interior: f32) -> (f32, f32) {
        if self.items.is_empty() {
            return (0.0, 1.0);
        }
        let (wx, wz) = self.world(px, pz);
        let (mut add, mut keep) = (0.0f32, 1.0f32);
        let mut block_add = 0.0f32;
        for s in &self.items {
            let Some(hit) = s.hit(wx, wz) else { continue };
            match s.kind {
                Kind::Range => {
                    // A little waviness in the belt edge, so it is no ruled strip.
                    let wobble = fbm2(s.seed ^ 6, wx * 3.0 + 0.41, wz * 3.0 + 0.17, 3, 2.07, 0.5) as f64 * 0.25;
                    let d = (hit.dist * (1.0 + wobble) / s.half_width) as f32;
                    let belt = (-d * d * 1.6).exp();
                    add = add.max(s.amplitude * s.massif(hit.along) * s.taper(hit.along) * belt);
                }
                Kind::Rift => {
                    let d = (hit.dist / s.half_width.max(1e-6)) as f32;
                    let trough = (1.0 - smoothstep(0.4, 1.6, d)) * s.taper(hit.along);
                    keep = keep.min(1.0 - 0.75 * trough);
                }
                Kind::Fault => {
                    // At map scale a fault reads as two different countries: an upland
                    // block on one side and a sagging basin on the other.
                    let up = s.fault_side(wx, wz, &hit);
                    let falloff = (1.0 - (up.abs() / FAULT_FALLOFF).min(1.0)).powi(2) * s.taper(hit.along);
                    if up > 0.0 {
                        block_add += 0.16 * falloff;
                    } else {
                        keep = keep.min(1.0 - 0.5 * falloff);
                    }
                }
            }
        }
        ((add + block_add) * interior, keep)
    }

    /// Change in ground height at a land position, in simulation units, once the land
    /// has been simulated: the fault steps and rift troughs that erosion must not soften.
    /// `land` is the landness there, so both fade out toward the sea.
    pub fn offset(&self, px: f32, pz: f32, land: f32) -> f32 {
        if self.items.is_empty() {
            return 0.0;
        }
        let (wx, wz) = self.world(px, pz);
        let coast = smoothstep(0.015, 0.08, land);
        let mut total = 0.0f32;
        for s in &self.items {
            let Some(hit) = s.hit(wx, wz) else { continue };
            match s.kind {
                Kind::Range => {}
                Kind::Fault => {
                    let up = s.fault_side(wx, wz, &hit);
                    // Sim units half-width of the scarp, as a share of a macro-cell.
                    let w = (FAULT_SCARP / CELL) as f32;
                    let step = smoothstep(-w, w, up);
                    let block = (1.0 - (up.max(0.0) / FAULT_FALLOFF).min(1.0)).powi(2);
                    let along = 0.75 + 0.3 * perlin2(s.seed ^ 11, hit.along * 0.5, 0.5);
                    total += s.amplitude.abs() * step * block * along * s.taper(hit.along);
                }
                Kind::Rift => {
                    let hw = s.half_width as f32;
                    // Ragged walls, and a floor of fallen blocks and fans rather than a
                    // plane: on a plane the drainage runs in dead-straight parallel lines.
                    let ragged = fbm2(s.seed ^ 14, wx * 40.0 + 0.23, wz * 40.0 + 0.67, 3, 2.07, 0.5);
                    let d = hit.dist as f32 + ragged * hw * 0.2;
                    let trough = 1.0 - smoothstep(hw * 0.65, hw, d);
                    let along = 0.7 + 0.4 * perlin2(s.seed ^ 12, hit.along * 0.8, 0.5);
                    let floor = fbm2(s.seed ^ 15, wx * 60.0 + 0.31, wz * 60.0 + 0.11, 4, 2.07, 0.5) * 28.0;
                    total -= (s.amplitude * along * s.taper(hit.along) + floor) * trough;
                }
            }
        }
        total * coast
    }

    /// How land is held near a border: `(target height, weight while simulating, weight
    /// of the final blend)` if the position is on a range spine inside the border band.
    /// `edge` is the max-norm distance from the cell centre. `None` elsewhere.
    ///
    /// Holding alone leaves a narrow crest, because erosion keeps cutting the held cells
    /// down and pulls the true profile in toward the border; the final blend, applied
    /// once the simulation is over, spreads the transition across the whole band.
    pub fn border_pin(&self, px: f32, pz: f32, edge: f32) -> Option<(f32, f32, f32)> {
        let b = 1.0 - edge;
        if b >= BORDER_BAND || self.items.is_empty() {
            return None;
        }
        let (wx, wz) = self.world(px, pz);
        let (land, crest) = self.spine_at(wx, wz)?;
        if land - self.basin_depth(wx, wz) * 4.0 <= 0.0 {
            return None;
        }
        // Rising to the border on both sides, so water runs away from it on each.
        let tilt = (1.0 - b / BORDER_BAND).powi(2) * (8.0 + crest * 0.05);
        let hard = BORDER_BAND * 0.05;
        let weight = if b < hard { 1.0 } else { (1.0 - (b - hard) / (BORDER_BAND - hard)).powf(2.5) };
        let blend = 1.0 - smoothstep(0.0, BORDER_BAND, b);
        Some(((crest + tilt).max(0.5), weight, blend))
    }

    /// How strongly a position is held to the border profile (0 inland, 1 at a border),
    /// used to fade out everything that is not shared across the border.
    pub fn border_hold(edge: f32) -> f32 {
        1.0 - smoothstep(BORDER_BAND * 0.45, BORDER_BAND * 1.4, 1.0 - edge)
    }
}

/// Sim units over which a fault's step happens, either side of the line.
const FAULT_SCARP: f64 = 40.0;

/// Every structure touching a cell, with a short description, for tools.
pub fn describe(world_seed: u32, mx: i32, mz: i32) -> Vec<String> {
    let field = Field::new(world_seed, mx, mz);
    field
        .items
        .iter()
        .map(|s| {
            let (a, b) = (s.points[0], s.points[s.points.len() - 1]);
            format!(
                "{:?} seed {:08x}, length {:.0} cells, width {:.2}, amplitude {:.1}, near cell {:.2},{:.2} to {:.2},{:.2}",
                s.kind, s.seed, s.length, s.half_width, s.amplitude, a.0, a.1, b.0, b.1
            )
        })
        .collect()
}
