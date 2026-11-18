//! The land about each settlement: generations of people clearing it and growing food
//! on it (stage 4 of the settlements plan, #64).
//!
//! - [`influence`]: one shared measure of how managed each spot is, 0 in the wild to 1 in
//!   a settlement's yards, and the rings of land use it falls into. Trees, plants and
//!   whatever comes later read it.
//! - [`fields`]: the worked land cut into fields, each with its use, so that every chunk
//!   finds the same field at a column whichever asks.
//! - [`bounds`]: what stands between two fields, from a fence or a wall by the houses to
//!   a ragged hedge, a line of scrub and a stone or two out at the edge of the wild.
//! - [`crops`]: what grows in the fields, as small plants that keep the farming year.
//! - [`orchard`]: the fruit and olive trees standing in rows in an orchard.
//! - [`terrace`]: an old place's steep fields stepped into shelves, each held up by a
//!   low bank of drystone or turf.
//! - [`farmstead`]: the barns, byres and field sheds out among the fields far from the
//!   houses.
//! - [`lanes`]: the lanes out through the fields of the lowlands and the uplands, hedged
//!   or walled on both sides.
//! - [`rotation`]: which crop a field carries in a given year and season, as its
//!   rotation comes round (#207).
//! - [`paddy`]: the flat wet floors of warm valleys levelled into paddies of standing
//!   water, held by low earth bunds.
//! - [`ditch`]: the straight ditches that drain the wet ground about an old place, with
//!   plank bridges where the lanes cross them.
//! - [`water`]: both of those, shaped into the ground column by column.
//!
//! Inside a settlement's radius the village layout has the ground; nothing here is laid
//! there, nor on a trail, in water, on a cliff or in a bog.

pub mod bounds;
pub mod crops;
pub mod ditch;
pub mod farmstead;
pub mod fields;
pub mod fixtures;
pub mod influence;
pub mod lanes;
pub mod orchard;
pub mod paddy;
pub mod quarry;
pub mod rotation;
pub mod terrace;
pub mod water;

pub use fields::{Crop, Plot, plot};
pub use influence::{Farm, Land, Use};

use voxel::Voxel;

use crate::{continent::Continent, materials as mat, settlements::years::When, surface::Column};

/// How the ground of a field looks from too far off for its crops and hedges to be
/// drawn (see [`far`]).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Far {
    /// The ground's top voxel: the field's soil, or the hedge or wall running across.
    pub surface: Voxel,
    /// The colour its crop gives the ground (see `plants::wash_at`), if the crop has one.
    pub wash: Option<u8>,
    /// How grazed its grass is, 0 to 7, if it is grazed (see [`crops::grazed`]).
    pub wear: Option<u8>,
}

/// The ground of the coarse cell `step` voxels across with its corner at continent-local
/// column `(lx, lz)` of `c`, whose ground is `col`, if it lies in a field: its soil and
/// crop's colour, or the hedge or wall between it and the next field across, so the far
/// fields lie in a patchwork as the near ones do. `seed` is the world's. Out in the wild
/// this is one lookup.
pub fn far(c: &Continent, seed: u32, col: &Column, lx: i32, lz: i32, step: i32) -> Option<Far> {
    if c.land.managed(lx, lz) < influence::EDGE || !fields::worked(col) {
        return None;
    }
    let p = plot(c, seed, lx, lz)?;
    for (dx, dz) in [(step, 0), (0, step)] {
        if let Some(q) = plot(c, seed, lx + dx, lz + dz)
            && q.id != p.id
            && p.crop != Crop::Paddy
            && q.crop != Crop::Paddy
        {
            let surface = match bounds::style(&c.land, &p, &q, col) {
                bounds::Style::Wall => c.land.farms[p.owner.min(q.owner)].stone,
                bounds::Style::Hedge => mat::HEDGE_FAR,
                bounds::Style::Thorn => mat::PLOUGH_DRY,
                _ => continue,
            };
            return Some(Far { surface, wash: None, wear: None });
        }
    }
    Some(Far {
        surface: crops::ground(p.crop, col).unwrap_or_else(|| mat::ground_cover(col.biome)),
        wash: crops::wash(p.crop),
        wear: crops::grazed(p.crop),
    })
}

/// How a spot of the land beyond the voxels looks, at continent-local `(lx, lz)` of `c`,
/// if it is worked: the soil of the field there (None for grass), its crop's colour
/// (see `plants::wash_at`), and how managed it is, which is how much of the wild's woods
/// has been cleared from it. For the far view, which draws a continent's grid cells, each
/// some of a field's width: so seen from the hills the land about a town is a soft
/// patchwork of plough, crops and grass. The field carries the crop it has at `when`, as
/// the near ground has it (see [`rotation::crop`]): in year 0, its own.
pub fn far_cell(c: &Continent, lx: i32, lz: i32, when: When) -> Option<(Option<Voxel>, Option<u8>, f32)> {
    let managed = c.land.managed(lx, lz);
    if managed < influence::EDGE {
        return None;
    }
    let seed = c.desc.world_seed;
    let p = plot(c, seed, lx, lz);
    let (soil, wash) = p.map_or((None, None), |p| {
        let crop = rotation::crop(&p, seed, when);
        let cs = c.cell_size();
        let i = ((lz as f32 / cs) as usize).min(c.size - 1) * c.size + ((lx as f32 / cs) as usize).min(c.size - 1);
        let biome = crate::biome::Biome::from_u8(c.biome[i]);
        (crops::soil(crop, biome, c.temperature[i]), crops::wash(crop))
    });
    Some((soil, wash, managed))
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use glam::Vec2;
    use voxel::{CHUNK, PAD};

    use crate::{
        World,
        continent::SimConfig,
        materials as mat,
        surface::{Cover, Sampler},
        world::GameWorld,
    };

    fn world() -> GameWorld {
        GameWorld::with_config(3, SimConfig { size: 256 })
    }

    /// Chunk columns out among the fields of the test continent's settlements.
    fn farmed_chunks(w: &GameWorld, count: usize) -> Vec<(i32, i32)> {
        let c = w.continent(0, 0);
        let mut out = Vec::new();
        for (k, f) in c.land.farms.iter().enumerate() {
            for a in 0..3 {
                let p = f.at + Vec2::from_angle(k as f32 * 1.7 + a as f32 * 2.1) * (f.radius + f.reach * (0.2 + 0.25 * a as f32));
                let (cx, cz) = (p.x as i32 / CHUNK, p.y as i32 / CHUNK);
                if c.land.managed(p.x as i32, p.y as i32) > 0.3 {
                    out.push((cx, cz));
                }
            }
        }
        out.truncate(count);
        assert!(out.len() >= count.min(8), "only {} farmed chunks found", out.len());
        out
    }

    const FARMED: [voxel::Voxel; 8] = [mat::PLOUGH, mat::PLOUGH_DRY, mat::HEDGE, mat::HEDGE_WOOD, mat::THORN, mat::WHEAT, mat::BARLEY, mat::RYE];

    #[test]
    fn the_fields_agree_across_chunks_and_keep_off_trails_water_and_yards() {
        let w = world();
        let c = w.continent(0, 0);
        let (mut farmed, mut cropped) = (0, 0);
        for (cx, cz) in farmed_chunks(&w, 14) {
            let a = w.generate_column(cx, cz);
            let b = w.generate_column(cx + 1, cz);
            // The columns either side share (x = PAD - 2, PAD - 1 here; 0, 1 there).
            for (cy, ca) in &a.chunks {
                let Some((_, cb)) = b.chunks.iter().find(|(y, _)| y == cy) else { continue };
                for z in 0..PAD {
                    for y in 0..PAD {
                        assert_eq!(ca.get(PAD - 2, y, z), cb.get(0, y, z), "chunks {cx},{cz} and east differ at y {y} z {z} of {cy}");
                        assert_eq!(ca.get(PAD - 1, y, z), cb.get(1, y, z), "chunks {cx},{cz} and east differ at y {y} z {z} of {cy}");
                    }
                }
            }
            // Nothing of the fields on a trail, in the water, or inside a settlement.
            let (wx0, wz0) = (cx * CHUNK - 1, cz * CHUNK - 1);
            let sampler = Sampler::new(&c, w.seed(), wx0, wz0, PAD as i32);
            for px in 1..=CHUNK as usize {
                for pz in 1..=CHUNK as usize {
                    let (x, z) = (wx0 + px as i32, wz0 + pz as i32);
                    let col = sampler.column(x, z);
                    let top = col.top();
                    let voxels: Vec<voxel::Voxel> = (top..top + 5)
                        .filter_map(|y| {
                            let (cy, ly) = (y.div_euclid(CHUNK), y.rem_euclid(CHUNK));
                            a.chunks.iter().find(|(k, _)| *k == cy).map(|(_, ch)| ch.get(px, ly as usize + 1, pz))
                        })
                        .collect();
                    let farmland = voxels.iter().any(|v| FARMED.contains(v));
                    farmed += farmland as u32;
                    let crops = a.plants.iter().any(|p| p.lo[0].div_euclid(4) == x && p.lo[2].div_euclid(4) == z && (55..=61).contains(&p.season));
                    cropped += crops as u32;
                    let off = col.cover == Cover::Trail || col.water_surface().is_some() || c.land.yard(x, z);
                    if off {
                        assert!(!farmland && !crops, "fields at {x},{z} on {:?}, water {:?}, yard {}", col.cover, col.water, c.land.yard(x, z));
                    }
                }
            }
        }
        assert!(farmed > 1000, "only {farmed} columns of plough and hedge");
        assert!(cropped > 500, "only {cropped} columns of crops");
    }

    /// The far view's cells about the fields agree with the near ground in every year: in
    /// year 0 each field's own crop, as before there were years, and later the crop its
    /// rotation has come round to.
    #[test]
    fn far_off_fields_carry_the_crop_of_the_year_and_season_seen() {
        use super::{crops, far_cell, rotation};
        use crate::settlements::years::When;
        let w = world();
        let c = w.continent(0, 0);
        let seed = c.desc.world_seed;
        let cs = c.cell_size();
        let (mut fields, mut turned) = (0, 0);
        for f in &c.land.farms {
            for k in 0..64 {
                let at = f.at + Vec2::from_angle(k as f32 * 0.7) * (f.radius + f.reach * (k % 8) as f32 / 8.0);
                let (lx, lz) = (at.x as i32, at.y as i32);
                let Some(p) = super::plot(&c, seed, lx, lz) else { continue };
                let i = ((lz as f32 / cs) as usize).min(c.size - 1) * c.size + ((lx as f32 / cs) as usize).min(c.size - 1);
                let biome = crate::biome::Biome::from_u8(c.biome[i]);
                let of = |crop| (crops::soil(crop, biome, c.temperature[i]), crops::wash(crop));
                let managed = c.land.managed(lx, lz);
                if managed < super::influence::EDGE {
                    continue;
                }
                for season in 0..4 {
                    let (soil, wash) = of(p.crop);
                    assert_eq!(far_cell(&c, lx, lz, When::new(0, season)), Some((soil, wash, managed)), "field {} in year 0", p.id);
                }
                for q in 4..4 + 4 * 5 {
                    let when = When::new(q / 4, (q % 4) as u8);
                    let now = rotation::crop(&p, seed, when);
                    let (soil, wash) = of(now);
                    assert_eq!(far_cell(&c, lx, lz, when), Some((soil, wash, managed)), "field {} at {when:?}", p.id);
                    turned += (now != p.crop) as u32;
                }
                fields += 1;
            }
        }
        assert!(fields > 50 && turned > 50, "{fields} far cells in fields, {turned} with another crop");
    }

    #[test]
    fn the_wild_costs_nothing_and_the_fields_little() {
        // Not a benchmark: only that the fields are not most of a chunk's work, and that
        // out in the wild a chunk finds there is nothing to do at once. Run with
        // --nocapture to see the times.
        let w = world();
        let c = w.continent(0, 0);
        let near = farmed_chunks(&w, 12);
        let wild: Vec<(i32, i32)> = (0..2048)
            .map(|k| ((k * 37) % 2048, (k * 91) % 2048))
            .filter(|&(cx, cz)| {
                let (x, z) = (cx * CHUNK, cz * CHUNK);
                c.land.managed(x, z) == 0.0 && c.land.wild_square(x - 40, z - 40, 112) && w.column(x, z).ground > 2.0
            })
            .take(12)
            .collect();
        let time = |chunks: &[(i32, i32)]| {
            let start = Instant::now();
            for &(cx, cz) in chunks {
                std::hint::black_box(w.generate_column(cx, cz));
            }
            start.elapsed().as_secs_f64() * 1000.0 / chunks.len() as f64
        };
        // Once to warm the caches, then for the time.
        time(&near);
        time(&wild);
        let (t_near, t_wild) = (time(&near), time(&wild));
        // The fields' own work: each chunk's patch of fields and its boundaries, from
        // columns already sampled.
        let n = PAD + 2;
        let sampled: Vec<_> = near
            .iter()
            .map(|&(cx, cz)| {
                let (x0, z0) = (cx * CHUNK - 2, cz * CHUNK - 2);
                let sampler = Sampler::new(&c, w.seed(), x0, z0, n as i32);
                ((x0, z0), (0..n * n).map(|i| sampler.column(x0 + (i / n) as i32, z0 + (i % n) as i32)).collect::<Vec<_>>())
            })
            .collect();
        let start = Instant::now();
        let mut edges = 0;
        for ((x0, z0), columns) in &sampled {
            let patch = super::fields::Patch::new(&c, w.seed(), *x0, *z0, n, columns);
            for i in 1..n - 1 {
                for j in 1..n - 1 {
                    edges += super::bounds::edge(&c.land, &patch, &columns[i * n + j], i, j).is_some() as u32;
                }
            }
        }
        let t_fields = start.elapsed().as_secs_f64() * 1000.0 / near.len() as f64;
        let start = Instant::now();
        for &(cx, cz) in &wild {
            std::hint::black_box(c.land.wild_square(cx * CHUNK - 2, cz * CHUNK - 2, PAD as i32 + 2));
        }
        let t_wild_check = start.elapsed().as_secs_f64() * 1e6 / wild.len() as f64;
        println!(
            "chunk column near a settlement {t_near:.2} ms (of which fields and their boundaries {t_fields:.2} ms, {edges} boundary columns), in the wild {t_wild:.2} ms; the wild's check {t_wild_check:.2} us"
        );
        assert!(t_wild_check < 50.0);
    }
}
