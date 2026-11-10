//! The inputs to the continent simulation: where land is, how strongly it is uplifted
//! and how easily it erodes. All are smooth functions of position, so they can be
//! sampled at any grid resolution.

use crate::{
    macro_layout::{Archetype, CellDesc, CellKind},
    noise::{fbm2, hash2, perlin2, ridged2, unit},
    structures::Field,
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
    /// Peninsulas of a lobed continent, from the centre out: `(end x, end z, width)`.
    lobes: Vec<(f32, f32, f32)>,
    /// The mega-structures passing through this cell.
    pub field: Field,
    /// The eight cells around this one, as `(dx, dz, shape)`, when a range could carry
    /// land to a border; their uplift is shared near the borders (see [`Shape::uplift`]).
    neighbours: Vec<(i32, i32, Shape)>,
}

/// How far from a border, in cell half-widths, uplift is shared with the neighbour.
const SHARED_UPLIFT: f32 = 0.16;

impl Shape {
    pub fn new(desc: CellDesc) -> Self {
        let mut shape = Self::bare(desc);
        if shape.field.structures().iter().any(|s| s.kind == crate::structures::Kind::Range) {
            for dz in -1..=1 {
                for dx in -1..=1 {
                    if (dx, dz) != (0, 0) {
                        let n = crate::macro_layout::cell_desc(desc.world_seed, desc.mx + dx, desc.mz + dz);
                        shape.neighbours.push((dx, dz, Self::bare(n)));
                    }
                }
            }
        }
        shape
    }

    /// A shape that does not look at its neighbours.
    fn bare(desc: CellDesc) -> Self {
        let mut blobs = Vec::new();
        let mut lobes = Vec::new();
        let r = |k: i32| unit(hash2(desc.seed ^ 0x15_1a2d, k, 7));
        match (desc.kind, desc.archetype) {
            (CellKind::Islands, Archetype::Volcanic) => {
                // A few round cones, with room between them.
                let count = 1 + (r(0) * 3.0) as i32;
                for i in 0..count {
                    let (x, z) = ((r(i * 3 + 1) - 0.5) * 0.8, (r(i * 3 + 2) - 0.5) * 0.8);
                    blobs.push((x, z, 0.13 + r(i * 3 + 3) * 0.12));
                }
            }
            (CellKind::Islands, Archetype::Chain) => {
                // Strung along a gentle arc through the cell.
                let count = 5 + (r(0) * 4.0) as i32;
                let (dx, dz) = (desc.angle.cos(), desc.angle.sin());
                let bend = (r(1) - 0.5) * 0.8;
                for i in 0..count {
                    let t = (i as f32 / (count - 1) as f32 - 0.5) * 1.25 + (r(i * 3 + 2) - 0.5) * 0.08;
                    let off = bend * (t * t - 0.13) + (r(i * 3 + 4) - 0.5) * 0.08;
                    let (x, z) = (dx * t - dz * off, dz * t + dx * off);
                    blobs.push((x.clamp(-0.62, 0.62), z.clamp(-0.62, 0.62), 0.07 + r(i * 3 + 3) * 0.09));
                }
            }
            (CellKind::Islands, _) => {
                let count = 3 + (r(0) * 5.0) as i32;
                for i in 0..count {
                    // Kept well inside the cell so no island is cut off by the border sea.
                    let (x, z) = ((r(i * 3 + 1) - 0.5) * 1.05, (r(i * 3 + 2) - 0.5) * 1.05);
                    blobs.push((x, z, 0.10 + r(i * 3 + 3) * 0.17));
                }
            }
            (CellKind::Continent, Archetype::Lobed) => {
                let count = 3 + (r(20) * 3.0) as i32;
                let start = r(21) * std::f32::consts::TAU;
                for i in 0..count {
                    let a = start + (i as f32 + (r(22 + i) - 0.5) * 0.5) * std::f32::consts::TAU / count as f32;
                    let len = 0.4 + r(30 + i) * 0.24;
                    let (x, z) = (desc.center.0 + a.cos() * len, desc.center.1 + a.sin() * len);
                    lobes.push((x, z, 0.1 + r(40 + i) * 0.08));
                }
            }
            _ => {}
        }
        let field = Field::new(desc.world_seed, desc.mx, desc.mz);
        Self { desc, blobs, lobes, field, neighbours: Vec::new() }
    }

    pub fn desc(&self) -> &CellDesc {
        &self.desc
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
        let rough = self.desc.coast;
        let mut coast = fbm2(s ^ 0x33, x * 2.4, z * 2.4, 4, 2.0, 0.55) * 0.30
            + (fbm2(s ^ 0x44, x * 6.5, z * 6.5, 3, 2.0, 0.5) * 0.11
                + fbm2(s ^ 0x45, x * 17.0, z * 17.0, 3, 2.0, 0.5) * 0.035)
                * rough;
        if rough > 1.2 {
            // Long narrow inlets, cut along the creases of ridged noise.
            coast -= ridged2(s ^ 0x46, x * 9.0, z * 9.0, 2, 2.0, 0.5).powi(4) * 0.12 * (rough - 1.0);
        }
        let base = match self.desc.kind {
            CellKind::Ocean => -1.0,
            CellKind::Continent => {
                let (dx, dz) = (px + wx - self.desc.center.0, pz + wz - self.desc.center.1);
                // Stretched along the long axis, keeping the same area.
                let (u, v) = self.axes(dx, dz);
                let a = self.desc.aspect.sqrt();
                let body = self.desc.radius - ((u / a).powi(2) + (v * a).powi(2)).sqrt();
                // Peninsulas reach out from the core, narrowing toward their tips.
                self.lobes.iter().fold(body, |best, &(ex, ez, width)| {
                    let (sx, sz) = (ex - self.desc.center.0, ez - self.desc.center.1);
                    let t = ((dx * sx + dz * sz) / (sx * sx + sz * sz)).clamp(0.0, 1.0);
                    let (qx, qz) = (dx - sx * t, dz - sz * t);
                    best.max(width * (1.0 - 0.5 * t) - (qx * qx + qz * qz).sqrt())
                })
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
        let own = (base + coast).min((EDGE - edge) * 3.0);
        // A mountain range's spine may run out over the border sea, right to the edge.
        match self.field.land(px, pz) {
            Some(spine) => own.max(spine),
            None => own,
        }
    }

    /// Tectonic uplift rate, 0..~1. High values become mountains.
    ///
    /// Near a border a range crosses, two cells simulate the same land, and if each
    /// raised it by its own rules their mountains would not meet. So there the rate
    /// eases toward a weighted mean of every cell meeting at that point, all of which
    /// compute the same mean on the border itself.
    pub fn uplift(&self, px: f32, pz: f32, land: f32) -> f32 {
        let own = self.own_uplift(px, pz, land);
        if self.neighbours.is_empty() {
            return own;
        }
        // Weight of a neighbour across one side: full on the border, none past the reach.
        let near = |p: f32, d: i32| -> f32 {
            match d {
                0 => 1.0,
                d => 1.0 - smoothstep(0.0, SHARED_UPLIFT, 1.0 - p * d as f32),
            }
        };
        let mut terms: Vec<(f32, f32)> = vec![(own, 1.0)];
        for (dx, dz, s) in &self.neighbours {
            let w = near(px, *dx) * near(pz, *dz);
            if w > 0.0 {
                let (qx, qz) = (px - 2.0 * *dx as f32, pz - 2.0 * *dz as f32);
                terms.push((s.own_uplift(qx, qz, s.land(qx, qz)), w));
            }
        }
        if terms.len() == 1 {
            return own;
        }
        // Summed in a fixed order, so every cell gets the same bits on the border.
        terms.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        let (sum, total) = terms.iter().fold((0.0f32, 0.0f32), |(s, t), &(u, w)| (s + u * w, t + w));
        sum / total
    }

    /// Position along and across the landmass's long axis.
    fn axes(&self, dx: f32, dz: f32) -> (f32, f32) {
        let (c, s) = (self.desc.angle.cos(), self.desc.angle.sin());
        (dx * c + dz * s, -dx * s + dz * c)
    }

    /// 0..1 up a volcanic cone: 0 at its shore, 1 at its summit.
    fn cone(&self, px: f32, pz: f32) -> f32 {
        if self.desc.archetype != Archetype::Volcanic || self.desc.kind != CellKind::Islands {
            return 0.0;
        }
        self.blobs
            .iter()
            .map(|&(bx, bz, br)| (1.0 - (px - bx).hypot(pz - bz) / br).clamp(0.0, 1.0))
            .fold(0.0, f32::max)
    }

    /// 0..1, how much a spot belongs to a volcano.
    pub fn volcanic(&self, px: f32, pz: f32) -> f32 {
        smoothstep(0.3, 0.55, self.cone(px, pz))
    }

    fn own_uplift(&self, px: f32, pz: f32, land: f32) -> f32 {
        let s = self.desc.seed;
        let (x, z) = (px as f64, pz as f64);
        let arch = self.desc.archetype;
        // Some coasts rise straight from the sea; most have a lowland fringe.
        let cliffy = match arch {
            Archetype::Tableland => 1.0,
            _ => smoothstep(0.15, 0.55, perlin2(s ^ 0x55, x * 2.0, z * 2.0)),
        };
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
        let rugged = self.desc.ruggedness;
        let ordinary = match arch {
            // One broad high table, cut at its edges into canyons.
            Archetype::Tableland => {
                0.03 + interior * (0.2 + 0.05 * hills + 0.06 * plateau + 0.5 * rugged * belts * upland)
            }
            Archetype::Lowland => {
                0.02 + interior * (0.01 + upland * (0.03 + 0.05 * hills) + 0.5 * rugged * belts * upland)
            }
            // Cones rising steeply to a summit crater.
            Archetype::Volcanic => {
                let c = self.cone(px, pz);
                let crater = 1.0 - 0.75 * smoothstep(0.86, 1.0, c);
                0.02 + interior * (0.03 + 0.05 * hills) + 1.3 * c.powf(1.4) * crater
            }
            _ => {
                let mut u = 0.02
                    + upland * (0.09 + 0.17 * hills + 0.12 * plateau)
                    + 0.95 * rugged * belts * (0.4 + 0.6 * upland);
                if arch == Archetype::Elongated {
                    // A spine of mountains down the long axis.
                    let (dx, dz) = (px - self.desc.center.0, pz - self.desc.center.1);
                    let (_, v) = self.axes(dx, dz);
                    let v = v + perlin2(s ^ 0x5a, x * 2.2, z * 2.2) * 0.08;
                    u += 0.55 * rugged * (-(v / 0.12).powi(2)).exp() * (0.5 + 0.5 * belts.max(hills));
                }
                if arch == Archetype::Rugged {
                    u += 0.12 * hills;
                }
                0.03 + interior * u
            }
        };
        // Great ranges stand on narrow spines of land too, so reach the coast sooner.
        let (range, keep) = self.field.uplift(px, pz, smoothstep(0.0, 0.07, land));
        (ordinary + range) * keep
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
