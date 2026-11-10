//! Fault scarps at full resolution.
//!
//! A fault's step goes onto the simulation grid as a ramp a few grid cells wide: the
//! grid (25 m a cell) can hold nothing sharper, and from the ground a 40 m step spread
//! over 80 m reads as a shadowed slope, not a fault. So the grid also keeps, for the
//! cells either side of each fault line, exactly what the ramp added there. The
//! surface sampler interpolates that the same way it interpolates the heights, takes
//! it back off, and puts in its place the step as it really stands: a face at the line
//! itself, with fallen stone banked against its foot. See [`Continent::scarp`].

use std::collections::HashMap;

use super::Continent;
use crate::{
    macro_layout::{MACRO_SIZE, SIM_SPAN},
    noise::perlin2,
    scale::OLD_VOXEL,
    structures::{Field, Kind, Structure, fault_block},
};

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Height of the stone banked against the foot of a face, as a share of the step.
const TALUS: f32 = 0.28;
/// Tallest face a step keeps above its talus, simulation units (about 14 m).
const FACE: f32 = 18.0;
/// Slope of that stone, rise over run.
const TALUS_SLOPE: f32 = 0.75;
/// Steps lower than this, in simulation units, stay the grid's slope; above the
/// second they are drawn wholly as faces.
const LOW_STEP: f32 = 8.0;
const FULL_STEP: f32 = 20.0;

/// The fault scarps of one continent.
#[derive(Default)]
pub struct Scarps {
    world_seed: u32,
    mx: i32,
    mz: i32,
    /// Grid cells per side.
    size: usize,
    /// Half-width of the grid's ramp, simulation units.
    soft: f32,
    /// The faults crossing the cell.
    faults: Vec<Structure>,
    /// Grid cells near a fault line: what the fault added to the height there, and the
    /// full height of its step there, both in simulation units. A land cell near a line
    /// that is missing had nothing added.
    nodes: HashMap<u32, (f32, f32)>,
}

/// A fault scarp at one column, from [`Continent::scarp`]. Lengths in voxels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scarp {
    /// Add to the ground height the grid gives here: lifts the top of the face to its
    /// full height and drops its foot, in place of the grid's ramp.
    pub lift: f32,
    /// Height of the face standing above its talus.
    pub height: f32,
    /// Distance from the face, positive on the raised side.
    pub dist: f32,
    /// True for the columns along the top of the face, whose exposed side is the face:
    /// bare rock.
    pub face: bool,
    /// Chance, 0 to 1, that the column is loose stone: the talus under the face.
    pub scree: f32,
}

impl Scarps {
    pub(super) fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Records grid cell `i`: `offset` added by faults, `step` the step's full height.
    pub(super) fn note(&mut self, i: u32, offset: f32, step: f32) {
        self.nodes.insert(i, (offset, step));
    }

    pub(super) fn finish(&mut self, field: &Field, mx: i32, mz: i32, size: usize, soft: f32) {
        self.world_seed = field.world_seed();
        (self.mx, self.mz, self.size, self.soft) = (mx, mz, size, soft);
        self.faults = field.structures().iter().filter(|s| s.kind == Kind::Fault).cloned().collect();
    }

    /// Fault offset and step height at grid position `(fx, fz)` (cell centres at whole
    /// numbers): the offset interpolated exactly as the surface interpolates heights
    /// (Catmull-Rom), the step bilinearly.
    fn sample(&self, fx: f32, fz: f32) -> (f32, f32) {
        let node = |x: isize, z: isize| {
            let max = self.size as isize - 1;
            let i = z.clamp(0, max) as usize * self.size + x.clamp(0, max) as usize;
            self.nodes.get(&(i as u32)).copied().unwrap_or((0.0, 0.0))
        };
        let weights = |t: f32| {
            let (t2, t3) = (t * t, t * t * t);
            [
                -0.5 * t3 + t2 - 0.5 * t,
                1.5 * t3 - 2.5 * t2 + 1.0,
                -1.5 * t3 + 2.0 * t2 + 0.5 * t,
                0.5 * t3 - 0.5 * t2,
            ]
        };
        let (x1, z1) = (fx.floor() as isize, fz.floor() as isize);
        let (tx, tz) = (fx - x1 as f32, fz - z1 as f32);
        let (wx, wz) = (weights(tx), weights(tz));
        let mut offset = 0.0;
        for (j, wzj) in wz.iter().enumerate() {
            for (i, wxi) in wx.iter().enumerate() {
                offset += node(x1 - 1 + i as isize, z1 - 1 + j as isize).0 * wxi * wzj;
            }
        }
        let s = |x: isize, z: isize| node(x, z).1;
        let a = s(x1, z1) + (s(x1 + 1, z1) - s(x1, z1)) * tx;
        let b = s(x1, z1 + 1) + (s(x1 + 1, z1 + 1) - s(x1, z1 + 1)) * tx;
        (offset, a + (b - a) * tz)
    }

    /// See [`Continent::scarp`].
    pub fn at(&self, lx: i32, lz: i32) -> Option<Scarp> {
        if self.nodes.is_empty() {
            return None;
        }
        let n = self.size;
        let cs = MACRO_SIZE as f32 / n as f32;
        let (vx, vz) = (lx as f32 + 0.5, lz as f32 + 0.5);
        // Position in the grid of cell centres, as the surface sampler has it.
        let max = (n - 1) as f32;
        let (fx, fz) = ((vx / cs - 0.5).clamp(0.0, max), (vz / cs - 0.5).clamp(0.0, max));
        // Most of the land is nowhere near a fault: one lookup settles it.
        let near = fz.round() as usize * n + fx.round() as usize;
        if !self.nodes.contains_key(&(near as u32)) {
            return None;
        }
        let (wx, wz) = (
            self.mx as f64 + vx as f64 / MACRO_SIZE as f64,
            self.mz as f64 + vz as f64 / MACRO_SIZE as f64,
        );
        let up = self
            .faults
            .iter()
            .filter_map(|s| s.fault_dist(wx, wz))
            .min_by(|a, b| a.abs().total_cmp(&b.abs()))?;
        // Buttresses and bays along the face, a few metres to a few tens across, so it
        // is no ruled line. In simulation units, like everything below.
        let (gx, gz) = (wx * MACRO_SIZE as f64, wz * MACRO_SIZE as f64);
        let ragged = perlin2(self.world_seed ^ 0x5ca1, gx / 60.0, gz / 60.0) * 3.0
            + perlin2(self.world_seed ^ 0x5ca2, gx / 17.0, gz / 17.0) * 1.0;
        let d = up * SIM_SPAN + ragged;
        let reach = self.soft + SIM_SPAN / n as f32;
        if d.abs() >= reach {
            return None;
        }
        let (smooth, step) = self.sample(fx, fz);
        let strength = smoothstep(LOW_STEP, FULL_STEP, step);
        if strength <= 0.0 {
            return None;
        }
        // The face stands at most FACE high: the rest of a big step is a long bank of
        // fallen stone, as old scarps crumble back.
        let banked = step * TALUS.max(1.0 - FACE / step.max(1.0));
        let talus = (banked + d * TALUS_SLOPE).max(0.0);
        let sharp = if d >= 0.0 { step * fault_block(up) } else { talus };
        // Hand back to the grid's own surface at the edge of the reach.
        let fade = 1.0 - smoothstep(self.soft, reach, d.abs());
        let lift = (sharp - smooth) * strength * fade;
        let height = (step - banked) * strength;
        Some(Scarp {
            lift: lift * OLD_VOXEL,
            height: height * OLD_VOXEL,
            dist: d * OLD_VOXEL,
            face: strength > 0.5 && (0.0..0.75).contains(&d),
            scree: if d < 0.0 && talus > 0.0 { (0.4 + 0.6 * talus / banked) * strength } else { 0.0 },
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        continent::{SimConfig, generate},
        macro_layout::{MACRO_SIZE, cell_desc},
        scale::OLD_VOXEL,
    };

    #[test]
    fn a_large_fault_step_stands_as_a_face() {
        // Seed 1 has a fault with a tall step across cell -5,3.
        let c = generate(cell_desc(1, -5, 3), SimConfig { size: 256 });
        assert!(!c.scarps.is_empty(), "no fault scarp recorded");
        let cs = MACRO_SIZE as f32 / c.size as f32;
        // Walk columns across every recorded cell, both ways, and find the largest drop
        // between neighbouring columns once the grid's ramp is swapped for the face.
        let ground = |lx: i32, lz: i32| {
            let (fx, fz) = ((lx as f32 + 0.5) / cs - 0.5, (lz as f32 + 0.5) / cs - 0.5);
            let base = c.scarps.sample(fx, fz).0 * OLD_VOXEL;
            base + c.scarp(lx, lz).map_or(0.0, |s| s.lift)
        };
        let (mut best, mut faces) = (0.0f32, 0);
        let mut tall: Vec<(u32, f32)> =
            c.scarps.nodes.iter().map(|(&i, &(_, s))| (i, s)).filter(|t| t.1 > 22.0).collect();
        tall.sort_by_key(|t| t.0);
        for &(i, step) in tall.iter().step_by(7).take(40) {
            let (x, z) = ((i as usize % c.size) as f32 * cs, (i as usize / c.size) as f32 * cs);
            for k in 0..cs as i32 * 2 {
                let (lx, lz) = (x as i32 - cs as i32 / 2 + k, z as i32);
                let drop = (ground(lx + 1, lz) - ground(lx, lz)).abs();
                let (ax, az) = (x as i32, z as i32 - cs as i32 / 2 + k);
                let drop = drop.max((ground(ax, az + 1) - ground(ax, az)).abs());
                best = best.max(drop / (step * OLD_VOXEL));
                faces += c.scarp(lx, lz).is_some_and(|s| s.face) as i32;
            }
        }
        assert!(faces > 0, "no face columns");
        assert!(best > 0.4, "the step is still a ramp: largest drop between columns {best:.2} of the step");
    }
}

impl Continent {
    /// The fault scarp at continent-local voxel column `(lx, lz)`, if one runs there.
    ///
    /// The grid draws a fault's step as a ramp a few cells wide; this is the step as it
    /// stands, for the surface to draw: add [`Scarp::lift`] to the ground interpolated
    /// from [`Continent::height`] (Catmull-Rom, cell centres at `(i + 0.5) *
    /// cell_size`), and the ramp becomes a sheer face at the fault line over a bank of
    /// talus. Small steps are left as the grid's slope. Cheap where there is no fault:
    /// one hash lookup.
    pub fn scarp(&self, lx: i32, lz: i32) -> Option<Scarp> {
        self.scarps.at(lx, lz)
    }
}
