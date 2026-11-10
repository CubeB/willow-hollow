//! A map of the continent around a point, as an image (#26).
//!
//! [`build`] is a pure function of the world and a position, so it can run on a worker
//! thread; the in-game minimap will draw what it returns. `--map FILE` writes one to a
//! PNG.

use glam::DVec3;
use worldgen::{macro_layout::MACRO_SIZE, scale::OLD_VOXEL, world::GameWorld};

use crate::farview;

/// Pixels per side of the map.
pub const MAP_SIZE: u32 = 2048;

/// The continent under `pos` (world voxels), north up, as `MAP_SIZE` square RGBA:
/// the far view's colours, hill-shaded from the north-west.
pub fn build(world: &GameWorld, pos: DVec3) -> image::RgbaImage {
    let (mx, mz) = (
        (pos.x / MACRO_SIZE as f64).floor() as i32,
        (pos.z / MACRO_SIZE as f64).floor() as i32,
    );
    let c = world.continent(mx, mz);
    let colors = farview::map_colors(world, &c);
    let n = c.size;
    let cs = c.cell_size();
    let at = |x: usize, z: usize| z.min(n - 1) * n + x.min(n - 1);
    let height = |x: usize, z: usize| c.height[at(x, z)] * OLD_VOXEL;
    image::RgbaImage::from_fn(MAP_SIZE, MAP_SIZE, |px, pz| {
        // Position in the grid of cell centres.
        let f = |p: u32| ((p as f32 + 0.5) / MAP_SIZE as f32 * n as f32 - 0.5).clamp(0.0, (n - 1) as f32);
        let (fx, fz) = (f(px), f(pz));
        let (x0, z0) = (fx as usize, fz as usize);
        let (tx, tz) = (fx - x0 as f32, fz - z0 as f32);
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let mut rgb = [0.0f32; 3];
        for (k, v) in rgb.iter_mut().enumerate() {
            let c = |x, z| colors[at(x, z)][k];
            *v = lerp(lerp(c(x0, z0), c(x0 + 1, z0), tx), lerp(c(x0, z0 + 1), c(x0 + 1, z0 + 1), tx), tz);
        }
        // Hillshade on land: light from the north-west, slope from the grid.
        let i = at(x0, z0);
        let land = !c.ocean[i];
        let shade = if land {
            let gx = (height(x0 + 1, z0) - height(x0.saturating_sub(1), z0)) / (2.0 * cs);
            let gz = (height(x0, z0 + 1) - height(x0, z0.saturating_sub(1))) / (2.0 * cs);
            (1.0 - (gx + gz) * 0.9).clamp(0.55, 1.35)
        } else {
            1.0
        };
        let px = rgb.map(|v| (v * shade).round().clamp(0.0, 255.0) as u8);
        image::Rgba([px[0], px[1], px[2], 255])
    })
}
