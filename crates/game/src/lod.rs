//! Coarser voxel terrain for the rings beyond full detail: chunks whose voxels are 2 or
//! 4 times larger, covering 2 or 4 times the ground each.
//!
//! The rings meet at circles around the camera, with no walls at the join, so a coarse
//! ring must never stand above the finer ground in front of it or the view would pass
//! under its cut edge. Each coarse cell therefore takes the lowest of the ground under
//! it, rounded down.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use glam::IVec3;
use voxel::{AIR, CHUNK, Kind, PAD, PaddedChunk, Voxel};
use worldgen::{
    World,
    biome::Biome,
    features::{self, shape::rotate, trees::Species},
    macro_layout::{MACRO_SIZE, cell_of},
    materials as mat,
    scale::BLOCK,
    surface::{Column, Cover, Sampler},
    world::GameWorld,
};

/// Whether a feature stands tall enough to be seen from the coarse rings.
pub fn stands_tall(species: Species) -> bool {
    !matches!(
        species,
        Species::Bush | Species::Fern | Species::Boulder | Species::MossyBoulder | Species::FallenLog | Species::Stump
    )
}

type ModelKey = (Species, usize, u32, u8);

/// Voxel columns between the ground heights sampled under each coarse cell.
/// Corners alone (4) measured as safe on today's hills, but cliffs would find the gaps.
const SAMPLE_STRIDE: usize = 2;

/// Trees rebuilt at coarse resolution, made once per model, turn and level.
#[derive(Default)]
pub struct Models {
    cache: Mutex<HashMap<ModelKey, Arc<Vec<(IVec3, Voxel)>>>>,
}

impl Models {
    fn get(&self, world: &GameWorld, species: Species, variant: usize, turns: u32, level: u8) -> Arc<Vec<(IVec3, Voxel)>> {
        let key = (species, variant % species.variants(), turns & 3, level);
        if let Some(model) = self.cache.lock().expect("model cache poisoned").get(&key) {
            return model.clone();
        }
        // Built outside the lock. Two threads may do it at once, which is harmless.
        let model = Arc::new(downsample(world, species, variant, turns, level));
        self.cache.lock().expect("model cache poisoned").insert(key, model.clone());
        model
    }
}

/// A tree's voxels gathered into cells `2^level` voxels wide. Wood survives if it fills
/// a good part of a cell; leaves need a third of it, so crowns do not swell.
fn downsample(world: &GameWorld, species: Species, variant: usize, turns: u32, level: u8) -> Vec<(IVec3, Voxel)> {
    let step = 1 << level;
    let volume = (step * step * step) as u32;
    let mats = world.materials();
    let mut cells: HashMap<IVec3, (u32, u32, Vec<(Voxel, u32)>)> = HashMap::new();
    for (offset, v) in &world.library().get(species, variant).voxels {
        let q = rotate(*offset, turns);
        let cell = cells.entry(IVec3::new(q.x.div_euclid(step), q.y.div_euclid(step), q.z.div_euclid(step))).or_default();
        if mats.get(*v).kind == Kind::Foliage {
            cell.1 += 1;
        } else {
            cell.0 += 1;
        }
        match cell.2.iter_mut().find(|c| c.0 == *v) {
            Some(c) => c.1 += 1,
            None => cell.2.push((*v, 1)),
        }
    }
    let mut out: Vec<(IVec3, Voxel)> = cells
        .into_iter()
        .filter(|(_, (wood, leaf, _))| *wood * 8 >= volume || *leaf * 3 >= volume)
        .map(|(cell, (wood, _, kinds))| {
            let wooden = |v: Voxel| mats.get(v).kind != Kind::Foliage;
            let best = kinds
                .iter()
                .filter(|(v, _)| wood == 0 || wooden(*v))
                .max_by_key(|c| c.1)
                .expect("a cell has at least one voxel");
            (cell, best.0)
        })
        .collect();
    out.sort_unstable_by_key(|(p, _)| (p.y, p.x, p.z));
    out
}

/// What the top of a coarse cell is made of, and the layer under it: the choice
/// `world::surface_materials` makes, each biome with its own ground, soil and rock,
/// taking the commonest of its voxel-to-voxel speckle, which would only shimmer at this
/// distance. Cliffs are banded by height as they are filled (see [`cliff_rock`]).
fn skin(col: &Column, slope: f32) -> (Voxel, Voxel) {
    let cover = mat::ground_cover(col.biome);
    match col.cover {
        Cover::RiverBed => (mat::GRAVEL, mat::GRAVEL),
        Cover::LakeBed => (mat::MUD, mat::MUD),
        Cover::SeaFloor | Cover::Beach => (mat::SAND, mat::SAND),
        Cover::Cliff => (mat::STONE, mat::STONE),
        Cover::Scree => (mat::SCREE, mat::SCREE),
        Cover::Shingle => (mat::GRAVEL, mat::GRAVEL),
        Cover::Boulders => (mat::STONE, mat::STONE),
        Cover::Shelf => (mat::ROCK_DARK, mat::STONE),
        Cover::Dune if matches!(col.biome, Biome::Desert | Biome::DuneSea) => (mat::DUNE_SAND, mat::DUNE_SAND),
        Cover::Dune => (mat::SAND, mat::SAND),
        Cover::Trail => (
            match col.biome.parent() {
                Biome::Desert | Biome::Savanna => mat::PATH_SAND,
                Biome::Alpine | Biome::Tundra | Biome::Ocean => mat::PATH_GRAVEL,
                Biome::OldGrowth | Biome::Rainforest | Biome::Taiga => mat::PATH_DARK,
                _ => mat::PATH_DIRT,
            },
            mat::DIRT,
        ),
        // Tidal flats.
        Cover::Bank if col.ground < worldgen::scale::m(1.0) => (mat::MUD, mat::MUD),
        Cover::Bank | Cover::Land if slope > 0.95 => {
            let (light, _) = mat::bedrock(col.biome);
            (light, light)
        }
        Cover::Bank | Cover::Land if col.temperature < 0.10 => (mat::SNOW, mat::STONE),
        Cover::Bank | Cover::Land => (cover, mat::subsoil(cover)),
    }
}

/// The rock a coarse cliff shows in its cell at coarse height `y`: the band of the
/// voxel world's strata at the middle of the cell.
fn cliff_rock(seed: u32, biome: Biome, y: i32, step: i32) -> Voxel {
    mat::strata(seed, biome, y * step + step / 2, 0.0)
}

/// For each cell, the top of the lowest voxel column under it, edges included, so that
/// the cell can sit at or below all the ground under it. Just one column higher than the
/// finer ground at a ring's edge would leave a crack to see through.
///
/// Only every `stride`-th column each way is sampled, sampling being most of the cost of
/// a coarse column. The columns skipped are allowed for by going one voxel lower.
/// `corners` are the columns at the cell corners, `PAD + 1` each way, already sampled.
fn lowest_ground(sampler: &Sampler, corners: &[Column], lx0: i32, lz0: i32, step: i32, stride: usize) -> Vec<i32> {
    let n = PAD + 1;
    let s = step as usize;
    let stride = stride.clamp(1, s);
    let per_cell = s / stride;
    let fine = PAD * per_cell + 1;
    let mut tops = vec![0i32; fine * fine];
    for x in 0..fine {
        for z in 0..fine {
            tops[x * fine + z] = if x % per_cell == 0 && z % per_cell == 0 {
                corners[(x / per_cell) * n + z / per_cell].top()
            } else {
                sampler.column(lx0 + (x * stride) as i32, lz0 + (z * stride) as i32).top()
            };
        }
    }
    let skipped = if stride > 1 { 1 } else { 0 };
    let mut lowest = vec![i32::MAX; PAD * PAD];
    for cx in 0..PAD {
        for cz in 0..PAD {
            let cell = &mut lowest[cx * PAD + cz];
            for x in cx * per_cell..=(cx + 1) * per_cell {
                for z in cz * per_cell..=(cz + 1) * per_cell {
                    *cell = (*cell).min(tops[x * fine + z]);
                }
            }
            *cell -= skipped;
        }
    }
    lowest
}

/// The chunks of one coarse column: `(chunk y, voxels)`, in coarse cells.
pub fn generate(world: &GameWorld, models: &Models, level: u8, cx: i32, cz: i32) -> Vec<(i32, PaddedChunk)> {
    let step = 1 << level;
    let span = CHUNK * step;
    let (wx0, wz0) = (cx * span - step, cz * span - step);
    let (mx, mz) = cell_of(cx * span, cz * span);
    let continent = world.continent(mx, mz);
    let (lx0, lz0) = (wx0 - mx * MACRO_SIZE, wz0 - mz * MACRO_SIZE);
    let margin = features::MAX_REACH + 1;
    let width = PAD as i32 * step;
    let sampler = Sampler::new(&continent, world.seed(), lx0 - margin, lz0 - margin, width + 2 * margin + step);

    // The ground at the corners of the cells: one more each way than there are cells.
    let n = PAD + 1;
    let mut corners = Vec::with_capacity(n * n);
    for x in 0..n {
        for z in 0..n {
            corners.push(sampler.column(lx0 + x as i32 * step, lz0 + z as i32 * step));
        }
    }
    let lowest_under = lowest_ground(&sampler, &corners, lx0, lz0, step, SAMPLE_STRIDE);

    struct Cell {
        top: i32,
        /// The top water cell, if the cell holds any.
        water: Option<i32>,
        surface: Voxel,
        under: Voxel,
        /// For a cliff, the biome whose rock bands it shows.
        cliff: Option<Biome>,
    }
    let mut cells = Vec::with_capacity(PAD * PAD);
    let (mut lo, mut hi) = (i32::MAX, i32::MIN);
    for x in 0..PAD {
        for z in 0..PAD {
            let four = [&corners[x * n + z], &corners[(x + 1) * n + z], &corners[x * n + z + 1], &corners[(x + 1) * n + z + 1]];
            let lowest = lowest_under[x * PAD + z];
            let highest = four.iter().map(|c| c.top()).max().expect("four corners");
            // The top of a column is its highest solid voxel, so its ground lies at
            // top + 1. The cell's ground must not lie above that, or the finer ring next
            // to it would show a crack where it is cut off.
            let mut top = (lowest + 1).div_euclid(step) - 1;
            // Any water at the corners floods the cell, so narrow rivers stay joined up.
            let water = four.iter().filter_map(|c| c.water_surface()).max().map(|k| k.div_euclid(step) - 1);
            // Rounding must not leave the bed poking through its own shallows.
            if let Some(w) = water {
                top = top.min(w - 1);
            }
            let low_corner = four.iter().map(|c| c.top()).min().expect("four corners");
            let (surface, under) = skin(four[0], (highest - low_corner) as f32 / step as f32);
            let cliff = (four[0].cover == Cover::Cliff).then_some(four[0].biome);
            lo = lo.min(top);
            hi = hi.max(top).max(water.unwrap_or(i32::MIN));
            cells.push(Cell { top, water, surface, under, cliff });
        }
    }

    let lib = world.library();
    let placements: Vec<_> = features::place(world.seed(), lib, &sampler, lx0, lz0, width, (mx * MACRO_SIZE, mz * MACRO_SIZE))
        .into_iter()
        .filter(|p| {
            let t = lib.get(p.species, p.variant);
            stands_tall(p.species)
                && p.pos.x + t.reach >= wx0
                && p.pos.x - t.reach < wx0 + width
                && p.pos.z + t.reach >= wz0
                && p.pos.z - t.reach < wz0 + width
        })
        .collect();
    for p in &placements {
        hi = hi.max(p.top(lib).div_euclid(step));
    }
    // Landmarks, bridges and lanterns: gathered from the full-size chunks underneath.
    let origin = IVec3::new(mx * MACRO_SIZE, 0, mz * MACRO_SIZE);
    let local = (cx * step - mx * (MACRO_SIZE / CHUNK), cz * step - mz * (MACRO_SIZE / CHUNK));
    let mut stamps = Vec::new();
    for dx in 0..step {
        for dz in 0..step {
            stamps.extend(continent.trails.stamps_at(local.0 + dx, local.1 + dz));
        }
    }
    for s in &stamps {
        hi = hi.max(s.hi.y.div_euclid(step));
        lo = lo.min(s.lo.y.div_euclid(step));
    }

    // Keep enough rock below for the slope's side faces.
    let cy_min = (lo - 2 * BLOCK).div_euclid(CHUNK);
    let cy_max = hi.div_euclid(CHUNK);
    let seed = world.seed();
    let mut chunks: Vec<(i32, PaddedChunk)> = Vec::with_capacity((cy_max - cy_min + 1) as usize);
    for cy in cy_min..=cy_max {
        let base = cy * CHUNK - 1;
        let mut chunk = PaddedChunk::new();
        for x in 0..PAD {
            for z in 0..PAD {
                let cell = &cells[x * PAD + z];
                for (py, v) in chunk.column_mut(x, z).iter_mut().enumerate() {
                    let y = base + py as i32;
                    *v = if y > cell.top {
                        if cell.water.is_some_and(|w| y <= w) { mat::WATER } else { AIR }
                    } else if let Some(biome) = cell.cliff {
                        // A face can be far taller than the soil: banded all the way down.
                        cliff_rock(seed, biome, y, step)
                    } else if y == cell.top {
                        cell.surface
                    } else if y >= cell.top - 2 {
                        cell.under
                    } else {
                        mat::STONE
                    };
                }
            }
        }
        chunks.push((cy, chunk));
    }

    let mut put = |q: IVec3, v: Voxel, force: bool| {
        let (px, pz) = (q.x - (cx * CHUNK - 1), q.z - (cz * CHUNK - 1));
        let py = q.y - cy_min * CHUNK;
        if px < 0 || pz < 0 || py < 0 || px >= PAD as i32 || pz >= PAD as i32 {
            return;
        }
        let (ci, ly) = ((py / CHUNK) as usize, py.rem_euclid(CHUNK) as usize);
        let (ux, uz) = (px as usize, pz as usize);
        let mut write = |index: usize, padded: usize| {
            if let Some((_, chunk)) = chunks.get_mut(index)
                && (force || chunk.get(ux, padded, uz) == AIR)
            {
                chunk.set(ux, padded, uz, v);
            }
        };
        write(ci, ly + 1);
        if ly == 0 && ci > 0 {
            write(ci - 1, PAD - 1);
        }
        if ly == CHUNK as usize - 1 {
            write(ci + 1, 0);
        }
    };
    let mats = world.materials();
    for p in &placements {
        let at = IVec3::new(p.pos.x.div_euclid(step), p.pos.y.div_euclid(step), p.pos.z.div_euclid(step));
        // A cell floods if water reaches any of its corners, so a tree on the shore can
        // find its cell under water. Better left out than seen growing from the lake.
        let (px, pz) = (at.x - (cx * CHUNK - 1), at.z - (cz * CHUNK - 1));
        if (0..PAD as i32).contains(&px)
            && (0..PAD as i32).contains(&pz)
            && cells[px as usize * PAD + pz as usize].water.is_some()
        {
            continue;
        }
        for (offset, v) in models.get(world, p.species, p.variant, p.turns, level).iter() {
            put(at + *offset, *v, mats.get(*v).kind == Kind::Solid);
        }
    }
    for s in &stamps {
        for &(p, v) in &s.voxels {
            let q = p + origin;
            put(IVec3::new(q.x.div_euclid(step), q.y.div_euclid(step), q.z.div_euclid(step)), v, true);
        }
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coarse_trees_keep_their_trunk_and_stay_smaller_in_cells() {
        let world = GameWorld::new(1);
        for level in 1..=2u8 {
            let cells = downsample(&world, Species::Oak, 0, 0, level);
            let voxels = world.library().get(Species::Oak, 0).voxels.len();
            assert!(!cells.is_empty(), "level {level} kept nothing");
            assert!(cells.len() < voxels, "coarse cells must be fewer");
            let wood = cells.iter().filter(|(_, v)| world.materials().get(*v).kind != Kind::Foliage).count();
            assert!(wood > 0, "level {level} lost the trunk");
        }
    }

    /// Sampling every other column must still keep almost every coarse cell's ground at
    /// or below all the full-size ground under it.
    #[test]
    fn sparse_sampling_keeps_cells_under_the_ground() {
        let world = GameWorld::new(1);
        let (mut cells, mut above) = (0, 0);
        // Hills in deciduous forest and steep taiga.
        for (x, z) in [(38944, 13600), (38560, 37024), (27808, 14368)] {
            let step = 4;
            let (mx, mz) = cell_of(x, z);
            let continent = world.continent(mx, mz);
            let (lx0, lz0) = (x - mx * MACRO_SIZE, z - mz * MACRO_SIZE);
            let sampler = Sampler::new(&continent, world.seed(), lx0, lz0, PAD as i32 * step + 1);
            let n = PAD + 1;
            let corners: Vec<_> = (0..n * n)
                .map(|i| sampler.column(lx0 + (i / n) as i32 * step, lz0 + (i % n) as i32 * step))
                .collect();
            let exact = lowest_ground(&sampler, &corners, lx0, lz0, step, 1);
            let sparse = lowest_ground(&sampler, &corners, lx0, lz0, step, SAMPLE_STRIDE);
            for (e, s) in exact.iter().zip(&sparse) {
                cells += 1;
                // Compared as the cells are built: ground rounded down to the cell size.
                if (s + 1).div_euclid(step) > (e + 1).div_euclid(step) {
                    above += 1;
                }
            }
        }
        println!("{above} of {cells} cells above the ground");
        assert!(above * 200 <= cells, "{above} of {cells} cells stand above the ground under them");
    }

    /// Times coarse column generation. Run with
    /// `cargo test --release -p game coarse_generation_time -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn coarse_generation_time() {
        let world = GameWorld::new(1);
        let models = Models::default();
        // Deciduous forest and taiga, as in the benchmarks.
        for (x, z) in [(38944, 13600), (38560, 37024)] {
            for level in 1..=2u8 {
                let span = CHUNK * (1 << level);
                let (cx, cz) = (x / span, z / span);
                generate(&world, &models, level, cx, cz); // continent and models warm
                let began = std::time::Instant::now();
                let n = 6;
                for i in 0..n {
                    generate(&world, &models, level, cx + i + 1, cz);
                }
                let ms = began.elapsed().as_secs_f32() * 1000.0 / n as f32;
                println!("({x}, {z}) level {level}: {ms:.1} ms per column");
            }
        }
    }
}
