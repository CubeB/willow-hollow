//! Paddies (#64): in warm wet country the flat floor of a valley beside its stream is
//! levelled into paddies, basins of standing water each held in by low earth bunds, where
//! rice stands green through the summer and gold at harvest. In spring, before it shows,
//! and from the harvest through the winter, the water lies open and shines.
//!
//! A paddy field is cut into straight-sided basins, long along its rows and short across
//! them, each with its own level: its water stands where the ground lay along its middle,
//! and its floor is dug a voxel under that. A bund runs round every basin, level with the
//! water; a wider one runs along the field's own edges, which bend as every field's do,
//! and round anything the paddies keep off (a way and what stands beside it, a building's
//! ground, a river's banks), so the water is held on every side wherever the paddy ends.
//! Where the ground beside a basin stands higher, the bund is a step down into it, and
//! where it lies lower, the bund stands as a low dyke.
//!
//! Which column is water and which is bund is decided from the column and its field
//! alone, as a terrace's shelf is (see [`super::terrace`]), so the surface sampler shapes
//! it the same for every chunk that asks.

use std::cell::RefCell;

use glam::{Vec2, vec2};

use super::{
    fields::{Crop, Found, fall, locate, worked},
    influence::{Farm, PASTURE},
    water::Shape,
};
use crate::{
    biome::Biome,
    continent::{Continent, NO_LAKE},
    noise::{hash3, unit},
    scale::m,
    settlements::Region,
    surface::{Column, Cover, MIN_RIVER_Q},
};

/// The share of the fields on a warm wet valley floor that are paddies.
pub const SHARE: f32 = 0.8;
/// How steep a field's ground may fall and still be levelled into paddies, in voxels of
/// rise per voxel: only a valley's flat floor.
const FLAT: f32 = 0.045;
/// The size of a basin, across the field's rows and along them, in voxels.
pub const ACROSS: f32 = 14.0;
pub const ALONG: f32 = 26.0;
/// How far in from the edge of its basin a column is bund, in voxels: enough that the
/// bund is unbroken wherever its edge runs across the voxels.
const BUND: f32 = 1.0;
/// How far in from the edge of its field: more, since the plane the fields are cut in is
/// bent, and a voxel's step can carry a column further across a field's edge there.
const DYKE: f32 = 1.7;
/// How far the paddies keep from a way or a building's ground, as a terrace does, and
/// from the edge of a river's channel, past the banks it shapes.
const CLEAR: f32 = super::terrace::CLEAR;
const RIVER: f32 = m(21.0);
/// How wide the bund round what the paddies keep off is, in voxels.
const RING: f32 = 2.0;

/// Whether `farm` is in country that grows rice in paddies: warm and wet, the lowlands,
/// the marshes and the rainforest.
pub fn grows(farm: &Farm) -> bool {
    farm.temperature > 0.6
        && matches!(farm.region, Region::Temperate | Region::Wetland | Region::Rainforest)
        && !farm.biome.arid()
        && !matches!(farm.biome, Biome::Savanna | Biome::Steppe | Biome::Desert | Biome::RedRock)
}

/// Whether a field of `farm` whose middle is at continent-local `at`, in `biome`, lies on
/// the flat wet floor of a valley that `farm` would make paddies of: near a stream or a
/// lake, and nearly level.
pub(super) fn valley(c: &Continent, farm: &Farm, at: Vec2, biome: Biome, _wet: bool) -> bool {
    if !grows(farm) || biome.arid() || matches!(biome, Biome::Savanna | Biome::Steppe | Biome::Bog | Biome::Mangrove | Biome::Alpine) {
        return false;
    }
    let cs = c.cell_size();
    let max = c.size as i32 - 1;
    let (ci, cj) = ((at.x / cs) as i32, (at.y / cs) as i32);
    // A stream or a lake within a couple of cells of the grid: the valley's own water.
    let water = (-2i32..=2).any(|dz| {
        (-2i32..=2).any(|dx| {
            let k = (cj + dz).clamp(0, max) as usize * c.size + (ci + dx).clamp(0, max) as usize;
            !c.ocean[k] && (c.discharge[k] >= MIN_RIVER_Q || c.lake[k] != NO_LAKE)
        })
    });
    water && fall(c, at).1 < FLAT
}

/// Whether column `col` holds a paddy's water (or a ditch's, which is the same to look
/// at): standing water on ground that is otherwise dry land.
pub fn flooded(col: &Column) -> bool {
    col.cover == Cover::Land && col.water_surface().is_some()
}

/// How the column at continent-local `(lx, lz)` of `c`, whose ground is `col`, lies in a
/// paddy, if it does: levelled to its basin's floor under its water, or raised or cut to
/// a bund level with it. `seed` is the world's.
pub(super) fn shape(c: &Continent, seed: u32, lx: i32, lz: i32, col: &Column) -> Option<Shape> {
    // Nothing is a paddy so far out in a place's land: the quick way out for most columns.
    if col.cover != Cover::Land || !worked(col) || c.land.managed(lx, lz) < PASTURE - 0.15 {
        return None;
    }
    let keep = (col.clear - CLEAR).min(col.river - RIVER);
    if keep < 0.0 {
        return None;
    }
    let found = locate(c, seed, lx, lz)?;
    if found.plot.crop != Crop::Paddy {
        return None;
    }
    let (k, kk, edge) = basin(&found);
    let level = level(c, seed, &found, k, kk);
    Some(if edge < BUND || found.edge < DYKE || keep < RING {
        Shape { ground: level as f32 + 0.5, water: None }
    } else {
        Shape { ground: (level - 1) as f32 + 0.5, water: Some(level + 1) }
    })
}

/// The two ways a paddy field's basins are laid: along its rows, and across them.
fn axes(found: &Found) -> (Vec2, Vec2) {
    let rows = found.plot.rows;
    (rows, vec2(-rows.y, rows.x))
}

/// The field's own offsets of its basins, each 0 to 1 of a basin.
fn offsets(found: &Found) -> (f32, f32) {
    (unit(hash3(found.field, 1, 0, 0x9add)), unit(hash3(found.field, 2, 0, 0x9add)))
}

/// Which basin of its field the column of `found` lies in, across and along the rows,
/// and how far in from the basin's edge it is, in voxels. The basins are laid out on the
/// ground itself from the field's middle, so their sides run straight.
fn basin(found: &Found) -> (i32, i32, f32) {
    let (rows, across) = axes(found);
    let (r1, r2) = offsets(found);
    let d = found.p - found.site;
    let (u, v) = (d.dot(across) / ACROSS + r1, d.dot(rows) / ALONG + r2);
    let (k, kk) = (u.floor(), v.floor());
    let edge = ((u - k).min(k + 1.0 - u) * ACROSS).min((v - kk).min(kk + 1.0 - v) * ALONG);
    (k as i32, kk as i32, edge)
}

thread_local! {
    /// The levels of the basins worked out last on this thread: most columns of a chunk
    /// share a basin with the one before. Only ever the same answer, sooner.
    static LEVELS: RefCell<Vec<Option<((u32, u32, i32, i32), i32)>>> = RefCell::new(vec![None; 256]);
}

/// The voxel layer basin `(k, kk)` of the field of `found` holds its water in: where the
/// ground lay along its middle, so it is dug down as much as it is built up. `seed` is
/// the world's.
fn level(c: &Continent, seed: u32, found: &Found, k: i32, kk: i32) -> i32 {
    let key = (c.desc.seed ^ seed.rotate_left(7), found.field, k, kk);
    let slot = (hash3(key.0 ^ key.1, k, kk, 0x1e7e) & 255) as usize;
    if let Some((at, level)) = LEVELS.with(|l| l.borrow()[slot])
        && at == key
    {
        return level;
    }
    let (rows, across) = axes(found);
    let (r1, r2) = offsets(found);
    let p = found.site + across * ((k as f32 + 0.5 - r1) * ACROSS) + rows * ((kk as f32 + 0.5 - r2) * ALONG);
    let ground = [-0.3f32, 0.0, 0.3].iter().map(|&t| crate::surface::natural_at(c, seed, p + rows * (t * ALONG), true)).sum::<f32>() / 3.0;
    let level = ground.floor() as i32;
    LEVELS.with(|l| l.borrow_mut()[slot] = Some((key, level)));
    level
}

#[cfg(test)]
mod tests {
    use voxel::{CHUNK, PAD};

    use super::*;
    use crate::{World, continent::SimConfig, surface::Sampler, world::GameWorld};

    /// A world and the chunk columns of its paddies, over the first few seeds that have
    /// any: (world, chunks).
    pub(crate) fn paddy_chunks(want: usize) -> Option<(GameWorld, Vec<(i32, i32)>)> {
        for seed in 1..40 {
            let w = GameWorld::with_config(seed, SimConfig { size: 256 });
            let c = w.continent(0, 0);
            let mut out = Vec::new();
            for f in c.land.farms.iter().filter(|f| grows(f)) {
                for a in 0..96 {
                    let p = f.at + Vec2::from_angle(a as f32 * 0.7) * (f.radius + f.reach * (0.05 + 0.006 * a as f32));
                    let (x, z) = (p.x as i32, p.y as i32);
                    if super::super::plot(&c, w.seed(), x, z).is_some_and(|p| p.crop == Crop::Paddy) {
                        out.push((x.div_euclid(CHUNK), z.div_euclid(CHUNK)));
                    }
                }
            }
            out.sort();
            out.dedup();
            if out.len() >= want {
                return Some((w, out));
            }
        }
        None
    }

    /// Paddies lie only on flat wet floors in warm country; each basin's water is level,
    /// a voxel deep over its dug floor, and held in on every side by bund or by water at
    /// its own level; and the chunks either side of a border agree.
    #[test]
    fn paddies_sit_level_and_hold_their_water() {
        let (w, chunks) = paddy_chunks(4).expect("no world among the first seeds has paddies");
        let c = w.continent(0, 0);
        let (mut water, mut bunds) = (0, 0);
        for &(cx, cz) in chunks.iter().take(12) {
            let (x0, z0) = (cx * CHUNK - 1, cz * CHUNK - 1);
            let n = PAD as i32;
            let sampler = Sampler::new(&c, w.seed(), x0 - 1, z0 - 1, n + 2);
            let col = |x: i32, z: i32| sampler.column(x, z);
            for i in 0..n {
                for j in 0..n {
                    let (x, z) = (x0 + i, z0 + j);
                    let here = col(x, z);
                    if !flooded(&here) {
                        continue;
                    }
                    let found = locate(&c, w.seed(), x, z).expect("a paddy's water is in a field");
                    if found.plot.crop != Crop::Paddy {
                        continue;
                    }
                    water += 1;
                    let farm = &c.land.farms[found.plot.owner];
                    assert!(grows(farm), "paddies at {x},{z} in country that grows none");
                    assert!(farm.temperature > 0.6);
                    let k = here.water_surface().unwrap();
                    assert_eq!(k - 1 - here.top(), 1, "the water at {x},{z} is not a voxel deep");
                    for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                        let next = col(x + dx, z + dz);
                        match next.water_surface() {
                            Some(nk) => assert_eq!(nk, k, "water at {x},{z} beside water at another level"),
                            None => {
                                assert!(next.top() >= k - 1, "the water at {x},{z} spills over {},{} (top {} under water {})", x + dx, z + dz, next.top(), k - 1);
                                bunds += 1;
                            }
                        }
                    }
                }
            }
        }
        println!("{water} columns of paddy water, {bunds} faces held by bunds");
        assert!(water > 200 && bunds > 20, "only {water} columns of water and {bunds} of bund");
        // The chunks either side of a border agree, water and all.
        for &(cx, cz) in chunks.iter().take(6) {
            let a = w.generate_column(cx, cz);
            let b = w.generate_column(cx + 1, cz);
            for (cy, ca) in &a.chunks {
                let Some((_, cb)) = b.chunks.iter().find(|(y, _)| y == cy) else { continue };
                for z in 0..PAD {
                    for y in 0..PAD {
                        assert_eq!(ca.get(PAD - 2, y, z), cb.get(0, y, z), "chunks {cx},{cz} and east differ at y {y} z {z} of {cy}");
                        assert_eq!(ca.get(PAD - 1, y, z), cb.get(1, y, z), "chunks {cx},{cz} and east differ at y {y} z {z} of {cy}");
                    }
                }
            }
        }
    }

    /// No paddy field anywhere but on a flat wet floor in warm country, over several
    /// worlds; and none at all where it is cold or dry.
    #[test]
    fn paddies_are_only_on_warm_wet_valley_floors() {
        let mut paddies = 0;
        for seed in 1..6 {
            let w = GameWorld::with_config(seed, SimConfig { size: 256 });
            let c = w.continent(0, 0);
            for f in &c.land.farms {
                for k in 0..300 {
                    let p = f.at + Vec2::from_angle(k as f32 * 0.61) * (f.radius + k as f32 * 2.0);
                    let Some(found) = locate(&c, w.seed(), p.x as i32, p.y as i32) else { continue };
                    if found.plot.crop != Crop::Paddy {
                        continue;
                    }
                    paddies += 1;
                    let farm = &c.land.farms[found.plot.owner];
                    assert!(grows(farm), "a paddy of a settlement in {:?} at {}", farm.region, farm.temperature);
                    assert!(fall(&c, found.site).1 < FLAT, "a paddy field on a slope at {p}");
                }
            }
        }
        println!("{paddies} columns of paddy fields");
    }
}
