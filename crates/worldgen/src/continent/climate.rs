//! Rainfall and temperature over a continent grid.

use crate::macro_layout::{CellDesc, SIM_SPAN};

/// Rainfall relative to the continent's average (which is `desc.wetness`).
///
/// Moist air is carried across the grid by the prevailing wind. It recharges over sea,
/// rains lightly over flat land and heavily where the ground forces it upward, and
/// loses what it drops. Windward slopes end up wet and the far side of a range dry:
/// a rain shadow, which in turn makes windward rivers larger.
pub fn rainfall(w: usize, height: &[f32], ocean: &[bool], desc: &CellDesc) -> Vec<f32> {
    let n = w * w;
    let dx = SIM_SPAN / w as f32;
    let (wind_x, wind_z) = (desc.wind.cos(), desc.wind.sin());
    // Air parcels step this many cells per update; scaled so behaviour is the same at
    // every grid resolution.
    let step = (w as f32 / 256.0).max(1.5);
    let step_voxels = step * dx;

    // Visit cells from upwind to downwind so each can read the air arriving at it.
    let mut order: Vec<u32> = (0..n as u32).collect();
    let proj = |i: u32| (i as usize % w) as f32 * wind_x + (i as usize / w) as f32 * wind_z;
    order.sort_unstable_by(|&a, &b| proj(a).total_cmp(&proj(b)));

    let ground = |i: usize| height[i].max(0.0);
    let bilinear = |field: &[f32], x: f32, z: f32, fallback: f32| -> f32 {
        if x < 0.0 || z < 0.0 || x > (w - 1) as f32 || z > (w - 1) as f32 {
            return fallback;
        }
        let (x0, z0) = (x as usize, z as usize);
        let (x1, z1) = ((x0 + 1).min(w - 1), (z0 + 1).min(w - 1));
        let (tx, tz) = (x.fract(), z.fract());
        let a = field[z0 * w + x0] + (field[z0 * w + x1] - field[z0 * w + x0]) * tx;
        let b = field[z1 * w + x0] + (field[z1 * w + x1] - field[z1 * w + x0]) * tx;
        a + (b - a) * tz
    };

    let ground_field: Vec<f32> = (0..n).map(ground).collect();
    let mut moisture = vec![1.0f32; n];
    let mut rain = vec![0.0f32; n];
    for &i in &order {
        let i = i as usize;
        let (x, z) = ((i % w) as f32, (i / w) as f32);
        let (ux, uz) = (x - wind_x * step, z - wind_z * step);
        let arriving = bilinear(&moisture, ux, uz, 1.0);
        if ocean[i] {
            moisture[i] = arriving + (1.0 - arriving) * 0.10;
            continue;
        }
        let climb = (ground_field[i] - bilinear(&ground_field, ux, uz, 0.0)).max(0.0);
        let lift = climb / step_voxels;
        // Fraction of the air's moisture that falls on this step. Tuned so air keeps
        // about 60% of its moisture across 20,000 voxels of flat land, and loses about
        // half climbing a 400-voxel range: enough for a clear wet side and dry side
        // without the interior turning to desert.
        let fraction = (step_voxels * (2.6e-5 + 1.9e-3 * lift)).min(0.5);
        rain[i] = arriving * fraction / step_voxels;
        moisture[i] = arriving * (1.0 - fraction);
    }

    // Soften, then compress the range and normalise so the land averages `wetness`.
    let mut tmp = vec![0.0f32; n];
    let radius = ((w as f32 / 128.0).round() as i32).max(1);
    for _ in 0..2 {
        blur_land(&rain, &mut tmp, ocean, w, radius, true);
        blur_land(&tmp, &mut rain, ocean, w, radius, false);
    }
    let land: Vec<usize> = (0..n).filter(|&i| !ocean[i]).collect();
    if land.is_empty() {
        return vec![1.0; n];
    }
    for &i in &land {
        rain[i] = rain[i].powf(0.6);
    }
    let mean = land.iter().map(|&i| rain[i]).sum::<f32>() / land.len() as f32;
    let scale = desc.wetness / mean.max(1e-6);
    for i in 0..n {
        rain[i] = if ocean[i] { desc.wetness } else { (rain[i] * scale).clamp(0.08, 3.5) };
    }
    rain
}

/// One axis of a box blur that averages over land cells only.
fn blur_land(src: &[f32], dst: &mut [f32], ocean: &[bool], w: usize, radius: i32, along_x: bool) {
    for z in 0..w as i32 {
        for x in 0..w as i32 {
            let (mut sum, mut count) = (0.0, 0.0);
            for k in -radius..=radius {
                let (sx, sz) = if along_x { (x + k, z) } else { (x, z + k) };
                if sx < 0 || sz < 0 || sx >= w as i32 || sz >= w as i32 {
                    continue;
                }
                let j = sz as usize * w + sx as usize;
                if !ocean[j] {
                    sum += src[j];
                    count += 1.0;
                }
            }
            dst[z as usize * w + x as usize] = if count > 0.0 { sum / count } else { 0.0 };
        }
    }
}

/// Temperature, 0 = polar to 1 = tropical: the cell's base climate, a north-south
/// gradient across it, and cooling with altitude.
pub fn temperature(w: usize, height: &[f32], desc: &CellDesc) -> Vec<f32> {
    let mut t = vec![0.0f32; w * w];
    for z in 0..w {
        let south = (z as f32 + 0.5) / w as f32 - 0.5; // -0.5 north edge .. 0.5 south edge
        for x in 0..w {
            let i = z * w + x;
            t[i] = desc.temperature + south * 0.34 - height[i].max(0.0) * 0.00115;
        }
    }
    t
}
