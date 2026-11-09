//! The inputs to the continent simulation: where land is, how strongly it is uplifted
//! and how easily it erodes. All are smooth functions of position, so they can be
//! sampled at any grid resolution.

use crate::{
    macro_layout::{CellDesc, CellKind},
    noise::{fbm2, hash2, perlin2, ridged2, unit},
};

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Beyond this distance from the cell centre (max-norm, cell half-width = 1) it is
/// always sea, which is what keeps landmasses from touching across cells.
const EDGE: f32 = 0.92;

pub struct Shape {
    desc: CellDesc,
    /// Island blobs for `CellKind::Islands`: centre and radius.
    blobs: Vec<(f32, f32, f32)>,
}

impl Shape {
    pub fn new(desc: CellDesc) -> Self {
        let mut blobs = Vec::new();
        if desc.kind == CellKind::Islands {
            let r = |k: i32| unit(hash2(desc.seed ^ 0x15_1a2d, k, 7));
            let count = 3 + (r(0) * 5.0) as i32;
            for i in 0..count {
                // Kept well inside the cell so no island is cut off by the border sea.
                let (x, z) = ((r(i * 3 + 1) - 0.5) * 1.05, (r(i * 3 + 2) - 0.5) * 1.05);
                blobs.push((x, z, 0.10 + r(i * 3 + 3) * 0.17));
            }
        }
        Self { desc, blobs }
    }

    /// Signed "landness" at a cell-normalised position (-1..1): positive is land, and
    /// the magnitude grows with distance from the coast (roughly 1 per cell half-width).
    pub fn land(&self, px: f32, pz: f32) -> f32 {
        let s = self.desc.seed;
        let (x, z) = (px as f64, pz as f64);
        // Warp the domain so coasts wander instead of following a circle.
        let wx = fbm2(s ^ 0x11, x * 1.4, z * 1.4, 3, 2.0, 0.5) * 0.22;
        let wz = fbm2(s ^ 0x22, x * 1.4, z * 1.4, 3, 2.0, 0.5) * 0.22;
        // Bays and headlands at three scales.
        let coast = fbm2(s ^ 0x33, x * 2.4, z * 2.4, 4, 2.0, 0.55) * 0.30
            + fbm2(s ^ 0x44, x * 6.5, z * 6.5, 3, 2.0, 0.5) * 0.11
            + fbm2(s ^ 0x45, x * 17.0, z * 17.0, 3, 2.0, 0.5) * 0.035;
        let base = match self.desc.kind {
            CellKind::Ocean => -1.0,
            CellKind::Continent => {
                let (dx, dz) = (px + wx - self.desc.center.0, pz + wz - self.desc.center.1);
                self.desc.radius - (dx * dx + dz * dz).sqrt()
            }
            CellKind::Islands => self
                .blobs
                .iter()
                .map(|&(bx, bz, br)| {
                    let (dx, dz) = (px + wx - bx, pz + wz - bz);
                    br - (dx * dx + dz * dz).sqrt()
                })
                .fold(-1.0, f32::max),
        };
        let edge = px.abs().max(pz.abs());
        (base + coast).min((EDGE - edge) * 3.0)
    }

    /// Tectonic uplift rate, 0..~1. High values become mountains.
    pub fn uplift(&self, px: f32, pz: f32, land: f32) -> f32 {
        let s = self.desc.seed;
        let (x, z) = (px as f64, pz as f64);
        // Some coasts rise straight from the sea; most have a lowland fringe.
        let cliffy = smoothstep(0.15, 0.55, perlin2(s ^ 0x55, x * 2.0, z * 2.0));
        let interior = smoothstep(0.0, 0.22, land).max(cliffy * smoothstep(0.0, 0.03, land));
        // Mountain belts: warped ridged noise thresholded into long ranges.
        let wx = perlin2(s ^ 0x66, x * 1.1, z * 1.1) as f64 * 0.35;
        let wz = perlin2(s ^ 0x77, x * 1.1, z * 1.1) as f64 * 0.35;
        let belts = ridged2(s ^ 0x88, (x + wx) * 1.5, (z + wz) * 1.5, 3, 2.0, 0.5);
        let belts = smoothstep(0.50, 0.92, belts);
        let hills = fbm2(s ^ 0x99, x * 4.5, z * 4.5, 4, 2.0, 0.5) * 0.5 + 0.5;
        let plateau = smoothstep(0.1, 0.5, perlin2(s ^ 0xaa, x * 1.7, z * 1.7));
        // Broad provinces: lowland basins that stay nearly flat versus uplands that the
        // rivers cut into hills. Without this every part of the land looks alike.
        let province = perlin2(s ^ 0xdd, x * 1.3, z * 1.3)
            + 0.3 * perlin2(s ^ 0xee, x * 3.1, z * 3.1);
        let upland = smoothstep(-0.30, 0.30, province);
        0.03 + interior
            * (0.02
                + upland * (0.09 + 0.17 * hills + 0.12 * plateau)
                + 0.95 * self.desc.ruggedness * belts * (0.4 + 0.6 * upland))
    }

    /// Large-scale relief the land starts with, in voxels. It decides which way the
    /// first rivers run, so it needs structure at every scale.
    pub fn initial_relief(&self, px: f32, pz: f32) -> f32 {
        let s = self.desc.seed;
        fbm2(s ^ 0xf1, px as f64 * 3.0, pz as f64 * 3.0, 5, 2.0, 0.55) * 45.0
    }

    /// Erodibility multiplier (~0.4..2.5). Bands of hard rock resist the rivers and
    /// leave knickpoints, which become rapids and waterfalls.
    pub fn erodibility(&self, px: f32, pz: f32) -> f32 {
        let s = self.desc.seed;
        let (x, z) = (px as f64, pz as f64);
        let rock = fbm2(s ^ 0xbb, x * 7.0, z * 7.0, 3, 2.0, 0.5);
        (rock * 1.1).exp()
    }

    /// Fine relief added when a grid is refined, so each level has detail to work on.
    pub fn detail(&self, px: f32, pz: f32, level: u32) -> f32 {
        let f = 6.0 * (1u32 << level) as f64;
        perlin2(self.desc.seed ^ (0xcc + level), px as f64 * f, pz as f64 * f)
    }
}
