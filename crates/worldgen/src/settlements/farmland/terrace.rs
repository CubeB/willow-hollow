//! Terraces (#64): where an old place's fields fall steeply, the ground is stepped into
//! level shelves, each held up along its lower edge by a low bank.
//!
//! The shelves follow the ground. A terraced column is levelled to the nearest step of
//! [`RISE`] voxels to its own ground, so no column moves by more than a voxel and the
//! edge of each shelf runs along a contour wherever the hillside takes it. On ground no
//! steeper than a walker can climb, two shelves side by side stand a step of [`RISE`]
//! apart, which a walker climbs as they would a stair: the hillside steps down gently.
//!
//! Upland, mountain and dry country, and any ancient place, hold their shelves with
//! drystone of the local stone; the green lowlands with banks of turf (lynchets). Only
//! fields of an established or ancient place are terraced, and only arable, hay, fruit
//! and vines: pasture and the rough are left to lie as they fall.
//!
//! Nothing within [`CLEAR`] of a trail, a lane or a building's levelled ground is
//! touched, so a way through the fields stays exactly as it was laid, and whatever was
//! set beside one (a lantern, a milestone, a seat) stands on the ground it was set on.
//!
//! Which column is terraced is decided from the column and its field alone, so the
//! surface sampler levels it the same for every chunk that asks, and chunk generation
//! faces the banks of the shelves it finds (see [`riser`]).

use voxel::Voxel;

use super::{
    fields::{Crop, Plot, worked},
    influence::{Farm, Land, PASTURE},
};
use crate::{
    biome::Biome,
    continent::Continent,
    materials as mat,
    noise::{hash3, unit},
    scale::{BLOCK, m},
    settlements::{Age, Region},
    surface::Column,
};

/// How high one shelf stands over the next, in voxels: as high as a walker steps up.
pub const RISE: i32 = BLOCK;
/// How far from the edge of a trail or a building's ground a column must lie to be
/// terraced. Wider than anything stood beside a trail reaches, and than a pad blends.
pub const CLEAR: f32 = m(4.8);

/// What holds a terrace's shelves up.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Bank {
    /// A low drystone wall of the local stone, its top course a coping level with the
    /// shelf above.
    Drystone,
    /// A bank of turf, grassed over.
    Earth,
}

/// Whether the fields of `farm` are ever terraced: an established or ancient place, in
/// country that terraces its hills.
pub fn terraces(farm: &Farm) -> bool {
    farm.age != Age::Young && matches!(farm.region, Region::Temperate | Region::Upland | Region::Alpine | Region::Desert | Region::Rainforest)
}

/// How steep a field's ground must be for `farm` to have terraced it, in voxels of rise
/// per voxel: an ancient place has stepped gentler slopes than a younger one has.
fn steep_enough(farm: &Farm) -> f32 {
    if farm.age == Age::Ancient { 0.13 } else { 0.22 }
}

/// The bank a field of `farm` growing `crop`, managed `managed` at its middle where the
/// ground falls `slope` voxels a voxel, is terraced with, if it is terraced.
pub(super) fn of(farm: &Farm, crop: Crop, managed: f32, slope: f32) -> Option<Bank> {
    if !terraces(farm) || managed < PASTURE || slope < steep_enough(farm) {
        return None;
    }
    if matches!(crop, Crop::Pasture | Crop::Rough | Crop::Coppice) {
        return None;
    }
    use Biome::*;
    let stony = matches!(farm.region, Region::Upland | Region::Alpine | Region::Desert)
        || farm.age == Age::Ancient
        || matches!(farm.biome, Moor | Heath | Tundra | AlpineMeadow | Volcanic | Steppe | RedRock);
    Some(if stony { Bank::Drystone } else { Bank::Earth })
}

/// Whether the column at continent-local `(lx, lz)`, of field `p`, whose ground is `col`,
/// is stepped into its field's terraces, and with what bank. `land` is the continent's.
pub fn stepped(land: &Land, p: &Plot, lx: i32, lz: i32, col: &Column) -> Option<Bank> {
    let bank = p.terrace?;
    if col.clear < CLEAR || !worked(col) {
        return None;
    }
    // The column's own settlement must terrace too: where two places' land meets, the
    // field of one may reach over ground the other works.
    let owner = land.owner(lx, lz)?;
    (terraces(&land.farms[owner]) && land.managed(lx, lz) >= PASTURE).then_some(bank)
}

/// The ground a terraced column is levelled to, from its own: the nearest step, with
/// the top voxel in the middle of its layer.
pub fn level(ground: f32) -> f32 {
    let rise = RISE as f32;
    (ground / rise).round() * rise + 0.5
}

/// The ground of the column at continent-local `(lx, lz)` of `c` once terraced, if it
/// is terraced; `col` is the column as the land and the trails left it. `seed` is the
/// world's. Out of the fields this is a comparison and a lookup or two; in the fields of
/// a place that terraces, the column's field is worked out.
pub fn step(c: &Continent, seed: u32, lx: i32, lz: i32, col: &Column) -> Option<f32> {
    if col.clear < CLEAR || !matches!(col.cover, crate::surface::Cover::Land | crate::surface::Cover::Bank) {
        return None;
    }
    let land = &c.land;
    let owner = land.owner(lx, lz)?;
    if !terraces(&land.farms[owner]) || land.managed(lx, lz) < PASTURE || !worked(col) {
        return None;
    }
    let p = super::plot(c, seed, lx, lz)?;
    stepped(land, &p, lx, lz, col)?;
    Some(level(col.ground))
}

/// The face of a terrace's bank on a stepped column whose top is `top` and the lowest of
/// whose four neighbours stands at `low`, where the shelf stands a step or more over the
/// next: the lowest voxel of the face (the voxels from there up to `top` are the bank's),
/// and how it is built. Of the place's stone (`farm`'s) or of turf. A face is never more
/// than a couple of steps deep: below that, whatever the neighbour falls away to was
/// there before the terraces.
pub fn riser(bank: Bank, farm: &Farm, top: i32, low: i32, col: &Column) -> Option<(i32, Riser)> {
    if top - low < RISE {
        return None;
    }
    let riser = Riser { bank, stone: farm.stone, dark: farm.stone_dark, moss: !col.biome.arid(), cover: mat::ground_cover(col.biome) };
    Some(((low + 1).max(top - 2 * RISE + 1), riser))
}

/// How a bank's face is built, voxel by voxel (see [`riser`]).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Riser {
    bank: Bank,
    stone: Voxel,
    dark: Voxel,
    moss: bool,
    cover: Voxel,
}

impl Riser {
    /// The voxel of the bank at world `(x, y, z)`, `top` being the shelf's surface there.
    /// `seed` is the world's.
    pub fn at(&self, x: i32, y: i32, z: i32, top: i32, seed: u32) -> Voxel {
        match self.bank {
            Bank::Earth => self.cover,
            Bank::Drystone => {
                let r = unit(hash3(seed ^ 0x7e7a, x, y, z));
                if y == top {
                    // The coping: the bigger, darker stones laid along the top.
                    if r < 0.7 { self.dark } else { self.stone }
                } else if self.moss && r < 0.18 {
                    mat::MOSSY_STONE
                } else if r < 0.45 {
                    self.dark
                } else {
                    self.stone
                }
            }
        }
    }

    /// Whether the shelf's surface is the bank's own (a drystone coping) rather than the
    /// field's soil, so nothing is sown on it.
    pub fn capped(&self) -> bool {
        self.bank == Bank::Drystone
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec2;
    use voxel::{CHUNK, PAD};

    use super::*;
    use crate::{World, continent::SimConfig, surface::Sampler, world::GameWorld};

    /// Chunk columns among the fields of the test continent's places that terrace, where
    /// some field is stepped.
    fn terraced_chunks(w: &GameWorld) -> Vec<(i32, i32)> {
        let c = w.continent(0, 0);
        let mut out = Vec::new();
        for f in c.land.farms.iter().filter(|f| terraces(f)) {
            for a in 0..24 {
                let p = f.at + Vec2::from_angle(a as f32 * 0.9) * (f.radius + f.reach * (0.1 + 0.03 * a as f32));
                let (x, z) = (p.x as i32, p.y as i32);
                if super::super::plot(&c, w.seed(), x, z).is_some_and(|p| p.terrace.is_some()) {
                    out.push((x.div_euclid(CHUNK), z.div_euclid(CHUNK)));
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// The terraces step the fields, by no more than a voxel anywhere, a step a walker
    /// climbs from one shelf to the next wherever the hillside was no steeper, and leave
    /// every trail, lane and building's ground, and the ground beside them, as it was.
    #[test]
    fn terraces_step_the_fields_and_leave_the_ways_alone() {
        let w = GameWorld::with_config(3, SimConfig { size: 256 });
        let c = w.continent(0, 0);
        let chunks = terraced_chunks(&w);
        let (mut stepped, mut shelves) = (0, 0);
        for &(cx, cz) in &chunks {
            let (x0, z0) = (cx * CHUNK - 1, cz * CHUNK - 1);
            let n = PAD as i32 + 1;
            let sampler = Sampler::new(&c, w.seed(), x0, z0, n);
            let cols: Vec<_> = (0..n * n).map(|i| (sampler.column(x0 + i / n, z0 + i % n), sampler.unstepped(x0 + i / n, z0 + i % n))).collect();
            for i in 0..n {
                for j in 0..n {
                    let (col, bare) = &cols[(i * n + j) as usize];
                    let (x, z) = (x0 + i, z0 + j);
                    assert!((col.top() - bare.top()).abs() <= 1, "{x},{z} moved from {} to {}", bare.ground, col.ground);
                    if col.ground != bare.ground {
                        stepped += 1;
                        assert!(col.clear >= CLEAR, "{x},{z} stepped {} from a way or a building's ground", col.clear);
                    }
                    if col.clear < CLEAR {
                        assert_eq!(col.ground, bare.ground, "{x},{z} by a way was stepped");
                    }
                    // From one shelf to the next is a step a walker climbs wherever the
                    // ground was no steeper than that before.
                    for (di, dj) in [(1, 0), (0, 1)] {
                        if i + di >= n || j + dj >= n {
                            continue;
                        }
                        let (next, next_bare) = &cols[((i + di) * n + j + dj) as usize];
                        let (rise, was) = ((col.top() - next.top()).abs(), (bare.top() - next_bare.top()).abs());
                        if was <= RISE {
                            assert!(rise <= RISE, "a step of {rise} at {x},{z} where the ground rose {was}");
                        }
                        shelves += (rise == RISE && col.ground != bare.ground) as u32;
                    }
                }
            }
        }
        println!("{} chunks with terraces: {stepped} columns stepped, {shelves} shelf edges", chunks.len());
        assert!(stepped > 500 && shelves > 50, "only {stepped} columns stepped and {shelves} shelf edges in {} chunks", chunks.len());
    }

    /// The chunks either side of a border agree on the terraces' shelves and banks.
    #[test]
    fn terraces_agree_across_chunks() {
        let w = GameWorld::with_config(3, SimConfig { size: 256 });
        for (cx, cz) in terraced_chunks(&w).into_iter().take(10) {
            let a = w.generate_column(cx, cz);
            let b = w.generate_column(cx, cz + 1);
            for (cy, ca) in &a.chunks {
                let Some((_, cb)) = b.chunks.iter().find(|(y, _)| y == cy) else { continue };
                for x in 0..PAD {
                    for y in 0..PAD {
                        assert_eq!(ca.get(x, y, PAD - 2), cb.get(x, y, 0), "chunks {cx},{cz} and south differ at x {x} y {y} of {cy}");
                        assert_eq!(ca.get(x, y, PAD - 1), cb.get(x, y, 1), "chunks {cx},{cz} and south differ at x {x} y {y} of {cy}");
                    }
                }
            }
        }
    }

    #[test]
    fn levelling_moves_the_ground_by_a_voxel_at_most_and_lands_on_a_step() {
        for k in -400..400 {
            let g = k as f32 * 0.137 + 0.01;
            let l = level(g);
            assert!((l.floor() as i32 - g.floor() as i32).abs() <= 1, "{g} levelled to {l}");
            assert_eq!((l.floor() as i32).rem_euclid(RISE), 0, "{g} levelled to {l}");
        }
    }
}
