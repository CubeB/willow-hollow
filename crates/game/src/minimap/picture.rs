//! The map's picture of one continent: the land in the far view's colours, hill-shaded,
//! with the sea, lakes, rivers and paths drawn over it. One texel per grid cell of the
//! simulation (about 26 m), built once per continent on the map's worker thread.

use worldgen::{
    World,
    continent::Continent,
    materials as mat,
    scale::OLD_VOXEL,
    trails::build::{TrailClass, class_of},
    world::GameWorld,
};

use crate::farview;

/// Texels per side of a continent's picture.
pub const TEX: usize = 1024;

/// Least water through a cell for it to be drawn as a stream.
const MIN_STREAM: f32 = 150.0;
/// Sea near the shore and far out (sRGB).
const SHALLOW: [f32; 3] = [84.0, 132.0, 158.0];
const DEEP: [f32; 3] = [50.0, 88.0, 124.0];
/// Paths, as they darken the ground they cross.
const PATH: [f32; 3] = [98.0, 78.0, 58.0];

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * t)
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The continent as `TEX` square RGBA (sRGB), north up: row 0 is its low-z edge, with
/// its hills shaded as on a printed relief map. For `--map FILE`.
pub fn picture(world: &GameWorld, c: &Continent) -> Vec<u8> {
    paint(world, c, true)
}

/// As [`picture`], with no shading: the colours of the land alone, for the map's model,
/// which has its own light (see [`super::light`]).
pub fn flat(world: &GameWorld, c: &Continent) -> Vec<u8> {
    paint(world, c, false)
}

fn paint(world: &GameWorld, c: &Continent, shaded: bool) -> Vec<u8> {
    let n = TEX;
    let size = c.size;
    let land = farview::map_land(world, c);
    let water = world.materials().get(mat::WATER).color.map(f32::from);
    // The lakes a little lighter than the far view has them, so they read on the map.
    let lake = mix(water, [150.0, 190.0, 210.0], 0.2);
    let cell = |t: usize| (t * size / n).min(size - 1);
    let at = |x: usize, z: usize| z * size + x;
    let h = |x: usize, z: usize| c.height[at(x, z)] * OLD_VOXEL;
    // Voxels per cell, for slopes.
    let cs = c.cell_size();
    let light = {
        let (x, y, z) = (-0.5f32, 0.62, -0.6);
        let l = (x * x + y * y + z * z).sqrt();
        (x / l, y / l, z / l)
    };
    let mut rgb = vec![[0.0f32; 3]; n * n];
    let mut is_water = vec![false; n * n];
    for tz in 0..n {
        for tx in 0..n {
            let (x, z) = (cell(tx), cell(tz));
            let i = at(x, z);
            let t = tz * n + tx;
            if c.ocean[i] {
                // Sim units below sea level; the shelf shades off within 50 m or so.
                rgb[t] = mix(SHALLOW, DEEP, smoothstep(0.0, 60.0, -c.height[i]));
                is_water[t] = true;
                continue;
            }
            if farview::lake_at(c, x, z) {
                rgb[t] = lake;
                is_water[t] = true;
                continue;
            }
            // Light from the north-west, as on a printed relief map. One-sided at the
            // grid's edge, so a range crossing it draws no line along the border.
            let (x0, x1) = (x.saturating_sub(1), (x + 1).min(size - 1));
            let (z0, z1) = (z.saturating_sub(1), (z + 1).min(size - 1));
            let gx = (h(x1, z) - h(x0, z)) / ((x1 - x0).max(1) as f32 * cs);
            let gz = (h(x, z1) - h(x, z0)) / ((z1 - z0).max(1) as f32 * cs);
            let (nx, ny, nz) = (-gx * 2.5, 1.0, -gz * 2.5);
            let len = (nx * nx + ny * ny + nz * nz).sqrt();
            let shade = ((nx * light.0 + ny * light.1 + nz * light.2) / len).clamp(0.0, 1.0);
            rgb[t] = if shaded { land[i].map(|v| v * (0.68 + 0.55 * shade)) } else { land[i] };
        }
    }

    // Rivers: a line from each carrying cell to the cell it drains into, faint for a
    // stream and solid for a river.
    let texel = |v: f32| v * n as f32 / size as f32;
    let voxels_per_texel = worldgen::macro_layout::MACRO_SIZE as f32 / n as f32;
    let mut wet = vec![0.0f32; n * n];
    for z in 0..size {
        for x in 0..size {
            let i = at(x, z);
            let q = c.discharge[i];
            if c.ocean[i] || q < MIN_STREAM {
                continue;
            }
            let r = c.rcv[i] as usize;
            let width = Continent::river_width(q) * OLD_VOXEL / voxels_per_texel;
            let strength = (0.35 + width * 0.9).min(1.0);
            let radius = (width * 0.5).max(0.45);
            let a = (texel(x as f32 + 0.5), texel(z as f32 + 0.5));
            let b = (texel((r % size) as f32 + 0.5), texel((r / size) as f32 + 0.5));
            splat_line(&mut wet, n, a, b, radius, strength);
        }
    }
    // Paths, darker for the more used.
    let mut path = vec![0.0f32; n * n];
    for (line, widths) in c.trails.lines.iter().zip(&c.trails.line_widths) {
        for (p, w) in line.iter().zip(widths) {
            let strength = match class_of(*w) {
                TrailClass::Road => 0.6,
                TrailClass::Track => 0.5,
                TrailClass::Footpath => 0.38,
                TrailClass::Trace => continue,
            };
            let (tx, tz) = ((p.x / voxels_per_texel) as usize, (p.y / voxels_per_texel) as usize);
            if tx < n && tz < n {
                let t = tz * n + tx;
                path[t] = path[t].max(strength);
            }
        }
    }

    let mut out = Vec::with_capacity(n * n * 4);
    for t in 0..n * n {
        let mut c = rgb[t];
        if !is_water[t] {
            c = mix(c, water, wet[t]);
            c = mix(c, PATH, path[t]);
        }
        out.extend(c.map(|v| v.round().clamp(0.0, 255.0) as u8));
        out.push(255);
    }
    out
}

/// Draws an antialiased line of `radius` texels into `mask`, keeping the larger value.
fn splat_line(mask: &mut [f32], n: usize, a: (f32, f32), b: (f32, f32), radius: f32, strength: f32) {
    let len = (b.0 - a.0).hypot(b.1 - a.1);
    let steps = (len * 2.0).ceil().max(1.0) as i32;
    let reach = (radius + 1.0).ceil() as i32;
    for s in 0..=steps {
        let t = s as f32 / steps as f32;
        let (cx, cz) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
        let (ix, iz) = (cx.floor() as i32, cz.floor() as i32);
        for dz in -reach..=reach {
            for dx in -reach..=reach {
                let (qx, qz) = (ix + dx, iz + dz);
                if qx < 0 || qz < 0 || qx >= n as i32 || qz >= n as i32 {
                    continue;
                }
                let d = (qx as f32 + 0.5 - cx).hypot(qz as f32 + 0.5 - cz);
                let cover = (radius + 0.5 - d).clamp(0.0, 1.0) * strength;
                let m = &mut mask[qz as usize * n + qx as usize];
                *m = m.max(cover);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_solid_along_its_middle_and_clear_beside_it() {
        let n = 32;
        let mut mask = vec![0.0; n * n];
        splat_line(&mut mask, n, (4.0, 10.5), (28.0, 10.5), 1.0, 1.0);
        assert!(mask[10 * n + 16] > 0.99);
        assert_eq!(mask[20 * n + 16], 0.0);
        // Never more than asked for.
        let mut faint = vec![0.0; n * n];
        splat_line(&mut faint, n, (4.0, 10.5), (28.0, 10.5), 1.0, 0.3);
        assert!(faint.iter().all(|&v| v <= 0.3 + 1e-6));
    }
}
