//! The real world: continents generated on demand and turned into voxel chunks.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

use voxel::{AIR, CHUNK, Materials, PAD, PaddedChunk, Voxel};

use crate::{
    World,
    continent::{self, Continent, NO_LAKE, SimConfig},
    macro_layout::{MACRO_SIZE, cell_desc, cell_of},
    materials as mat,
    noise::{hash3, perlin2, unit},
    surface::{Column, Cover, Sampler, node},
};

type Slot = Arc<OnceLock<Arc<Continent>>>;

pub struct GameWorld {
    seed: u32,
    materials: Materials,
    config: SimConfig,
    /// Simulated macro-cells. Each slot is filled once, by whichever thread asks first;
    /// others asking for the same cell wait for it rather than simulating it again.
    continents: Mutex<HashMap<(i32, i32), Slot>>,
}

impl GameWorld {
    pub fn new(seed: u32) -> Self {
        Self::with_config(seed, SimConfig::default())
    }

    pub fn with_config(seed: u32, config: SimConfig) -> Self {
        Self {
            seed,
            materials: mat::build(),
            config,
            continents: Mutex::new(HashMap::new()),
        }
    }

    pub fn seed(&self) -> u32 {
        self.seed
    }

    /// The simulated macro-cell `(mx, mz)`, generating it if needed (about 3 seconds).
    pub fn continent(&self, mx: i32, mz: i32) -> Arc<Continent> {
        let slot = self
            .continents
            .lock()
            .expect("continent map poisoned")
            .entry((mx, mz))
            .or_default()
            .clone();
        slot.get_or_init(|| {
            Arc::new(continent::generate(cell_desc(self.seed, mx, mz), self.config))
        })
        .clone()
    }

    /// The surface at one world voxel column. For spawn placement and tools; chunk
    /// generation samples in bulk instead.
    pub fn column(&self, x: i32, z: i32) -> Column {
        let (mx, mz) = cell_of(x, z);
        let c = self.continent(mx, mz);
        let (lx, lz) = (x - mx * MACRO_SIZE, z - mz * MACRO_SIZE);
        Sampler::new(&c, self.seed, lx, lz, 1).column(lx, lz)
    }

    /// A good place to begin: on the bank of a river in the origin continent, in a
    /// valley with high ground around it, looking at the water.
    pub fn spawn_point(&self) -> Spawn {
        let c = self.continent(0, 0);
        let n = c.size as i32;
        // Score river cells by how much the land rises around them.
        let reach = (24 * n / 1024).max(4);
        let stride = (reach / 4).max(1) as usize;
        let mut best: Option<(f32, usize)> = None;
        for i in (0..c.size * c.size).step_by(3) {
            let q = c.discharge[i];
            if c.ocean[i] || c.lake[i] != NO_LAKE || !(2500.0..=40_000.0).contains(&q) {
                continue;
            }
            let (x, z) = ((i % c.size) as i32, (i / c.size) as i32);
            if c.height[i] < 8.0 || x < reach || z < reach || x >= n - reach || z >= n - reach {
                continue;
            }
            let mut relief = 0.0f32;
            for dz in (-reach..=reach).step_by(stride) {
                for dx in (-reach..=reach).step_by(stride) {
                    let j = ((z + dz) * n + x + dx) as usize;
                    relief = relief.max(c.height[j] - c.height[i]);
                }
            }
            // Hills to look at, but not so steep the start is a gorge.
            let score = relief.min(160.0) + q.ln() * 4.0;
            if best.is_none_or(|b| score > b.0) {
                best = Some((score, i));
            }
        }
        let Some((_, cell)) = best else {
            // No suitable river: the middle of the continent will do.
            return Spawn {
                x: MACRO_SIZE / 2,
                z: MACRO_SIZE / 2,
                yaw: 0.0,
            };
        };

        // Walk outward from the river until there is dry, open ground just above it.
        let river = node(&c, cell);
        let sampler_at = |x: i32, z: i32| Sampler::new(&c, self.seed, x, z, 1).column(x, z);
        for ring in 0..40 {
            let dist = 8.0 + ring as f32 * 1.5;
            for step in 0..16 {
                let angle = step as f32 / 16.0 * std::f32::consts::TAU;
                let (x, z) = (
                    (river.x + angle.cos() * dist) as i32,
                    (river.y + angle.sin() * dist) as i32,
                );
                let col = sampler_at(x, z);
                if col.cover == Cover::Land && col.water_surface().is_none() {
                    let (dx, dz) = (river.x - x as f32, river.y - z as f32);
                    return Spawn {
                        x,
                        z,
                        yaw: dz.atan2(dx).to_degrees(),
                    };
                }
            }
        }
        Spawn {
            x: river.x as i32,
            z: river.y as i32,
            yaw: 0.0,
        }
    }
}

/// Where and which way a new player starts.
#[derive(Clone, Copy, Debug)]
pub struct Spawn {
    pub x: i32,
    pub z: i32,
    /// Degrees; 0 looks along +X.
    pub yaw: f32,
}

/// Picks the voxel for the top of a column and for the layers just beneath it.
///
/// `roll` is per-voxel noise; `patch` varies smoothly over several voxels.
fn surface_materials(col: &Column, steepness: i32, roll: f32, patch: f32) -> (Voxel, Voxel) {
    let cover = mat::ground_cover(col.biome);
    match col.cover {
        // Beds change in broad patches: speckle shows through the water as static.
        Cover::RiverBed => (if patch < 0.62 { mat::GRAVEL } else { mat::SAND }, mat::GRAVEL),
        Cover::LakeBed => (if patch < 0.55 { mat::MUD } else { mat::SAND }, mat::MUD),
        Cover::SeaFloor | Cover::Beach => (mat::SAND, mat::SAND),
        Cover::Bank if roll < 0.30 => (mat::GRAVEL, mat::DIRT),
        Cover::Bank if roll < 0.42 => (mat::MUD, mat::DIRT),
        Cover::Bank | Cover::Land => {
            // Ground too steep to hold soil shows bare rock.
            let rocky = steepness >= 3 || (steepness == 2 && roll < 0.5);
            if rocky {
                let rock = if roll < 0.25 { mat::ROCK_DARK } else { mat::STONE };
                (rock, mat::STONE)
            } else if col.temperature < 0.10 + (roll - 0.5) * 0.03 {
                (mat::SNOW, mat::STONE)
            } else if cover == mat::DESERT_SAND {
                (cover, mat::DESERT_SAND)
            } else {
                (cover, mat::DIRT)
            }
        }
    }
}

impl World for GameWorld {
    fn materials(&self) -> &Materials {
        &self.materials
    }

    fn generate_column(&self, cx: i32, cz: i32) -> Vec<(i32, PaddedChunk)> {
        // A chunk never straddles macro-cells, and its one-voxel border can only reach
        // into the uniform deep sea that separates them.
        let (mx, mz) = cell_of(cx * CHUNK, cz * CHUNK);
        let continent = self.continent(mx, mz);
        let (wx0, wz0) = (cx * CHUNK - 1, cz * CHUNK - 1);
        let (lx0, lz0) = (wx0 - mx * MACRO_SIZE, wz0 - mz * MACRO_SIZE);
        let sampler = Sampler::new(&continent, self.seed, lx0, lz0, PAD as i32);

        let mut columns = Vec::with_capacity(PAD * PAD);
        let (mut lo, mut hi) = (i32::MAX, i32::MIN);
        for px in 0..PAD {
            for pz in 0..PAD {
                let col = sampler.column(lx0 + px as i32, lz0 + pz as i32);
                let top = col.top();
                lo = lo.min(top);
                hi = hi.max(top).max(col.water_surface().unwrap_or(i32::MIN));
                columns.push(col);
            }
        }
        let top_of = |px: usize, pz: usize| columns[px * PAD + pz].top();

        // Surface and subsurface voxel per column.
        let mut skins = Vec::with_capacity(PAD * PAD);
        for px in 0..PAD {
            for pz in 0..PAD {
                let top = top_of(px, pz);
                // Largest step to a neighbour, as a measure of steepness.
                let mut steep = 0;
                for (dx, dz) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                    let (nx, nz) = (px as i32 + dx, pz as i32 + dz);
                    if nx >= 0 && nz >= 0 && nx < PAD as i32 && nz < PAD as i32 {
                        steep = steep.max((top - top_of(nx as usize, nz as usize)).abs());
                    }
                }
                let roll = unit(hash3(self.seed ^ 0x5cf, wx0 + px as i32, top, wz0 + pz as i32));
                let (wx, wz) = ((wx0 + px as i32) as f64, (wz0 + pz as i32) as f64);
                let patch = perlin2(self.seed ^ 0x9a7c, wx / 9.0, wz / 9.0) * 0.5 + 0.5;
                skins.push(surface_materials(&columns[px * PAD + pz], steep, roll, patch));
            }
        }

        // Only chunks that can hold a visible face: from just under the lowest ground
        // to the highest ground or water.
        let cy_min = (lo - 2).div_euclid(CHUNK);
        let cy_max = hi.div_euclid(CHUNK);
        let mut out = Vec::new();
        for cy in cy_min..=cy_max {
            let base = cy * CHUNK - 1; // world y of padded y = 0
            let mut chunk = PaddedChunk::new();
            for px in 0..PAD {
                for pz in 0..PAD {
                    let col = &columns[px * PAD + pz];
                    let (surface, under) = skins[px * PAD + pz];
                    let top = col.top();
                    let water_top = col.water_surface().map_or(i32::MIN, |k| k - 1);
                    for (py, v) in chunk.column_mut(px, pz).iter_mut().enumerate() {
                        let y = base + py as i32;
                        *v = if y > top {
                            if y <= water_top { mat::WATER } else { AIR }
                        } else if y == top {
                            surface
                        } else if y >= top - 3 {
                            under
                        } else {
                            mat::STONE
                        };
                    }
                }
            }
            out.push((cy, chunk));
        }
        out
    }
}
