//! The real world: continents generated on demand and turned into voxel chunks.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

use glam::{IVec3, Vec3};

use voxel::{AIR, CHUNK, Materials, PAD, PaddedChunk, Voxel, mesh::CELLS};

use crate::{
    Cascade, ColumnOutput, World,
    continent::{self, Continent, NO_LAKE, SimConfig},
    features::{self, Library},
    macro_layout::{MACRO_SIZE, cell_desc, cell_of},
    materials as mat,
    noise::{hash3, perlin2, unit},
    scale::{BLOCK, m, m64},
    surface::{Column, Cover, Sampler, node},
    trails,
};

type Slot = Arc<OnceLock<Arc<Continent>>>;

pub struct GameWorld {
    seed: u32,
    materials: Materials,
    config: SimConfig,
    /// Tree and feature models, built on first use.
    library: OnceLock<Library>,
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
            library: OnceLock::new(),
            continents: Mutex::new(HashMap::new()),
        }
    }

    pub fn seed(&self) -> u32 {
        self.seed
    }

    pub fn library(&self) -> &Library {
        self.library.get_or_init(Library::build)
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
            let mut c = continent::generate(cell_desc(self.seed, mx, mz), self.config);
            c.trails = trails::build(&c, self.seed);
            Arc::new(c)
        })
        .clone()
    }

    /// The macro-cell if it has already been simulated. Never blocks.
    pub fn continent_if_ready(&self, mx: i32, mz: i32) -> Option<Arc<Continent>> {
        let slot = self
            .continents
            .lock()
            .expect("continent map poisoned")
            .get(&(mx, mz))
            .cloned()?;
        slot.get().cloned()
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
            let dist = m(6.4) + ring as f32 * m(1.2);
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
fn surface_materials(col: &Column, steepness: f32, roll: f32, patch: f32) -> (Voxel, Voxel) {
    let cover = mat::ground_cover(col.biome);
    match col.cover {
        // Beds change in broad patches: speckle shows through the water as static.
        Cover::RiverBed => (if patch < 0.62 { mat::GRAVEL } else { mat::SAND }, mat::GRAVEL),
        Cover::LakeBed => (if patch < 0.55 { mat::MUD } else { mat::SAND }, mat::MUD),
        Cover::SeaFloor | Cover::Beach => (mat::SAND, mat::SAND),
        Cover::Trail => (trail_material(col, roll, patch), mat::DIRT),
        // Cliff faces take their colour from the rock layer at each height (see the fill
        // loop). The tops of ledges, which are nearly level, keep some soil.
        Cover::Cliff if steepness < 0.9 && roll < 0.55 => (cover, mat::STONE),
        Cover::Cliff => (mat::STONE, mat::STONE),
        Cover::Scree => (if roll < 0.8 { mat::SCREE } else { mat::GRAVEL }, mat::SCREE),
        Cover::Bank if roll < 0.30 => (mat::GRAVEL, mat::DIRT),
        Cover::Bank if roll < 0.42 => (mat::MUD, mat::DIRT),
        Cover::Bank | Cover::Land => {
            // Ground too steep to hold soil shows bare rock.
            // Soil gives way to rock between about 35 and 50 degrees, raggedly.
            let rocky = steepness > 0.72 + roll * 0.45;
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

/// What a trail is surfaced with: whatever feet would wear the local ground down to.
fn trail_material(col: &Column, roll: f32, patch: f32) -> Voxel {
    use crate::biome::Biome::*;
    let (main, other, mix) = match col.biome {
        Desert | Savanna => (mat::PATH_SAND, mat::PATH_DIRT, 0.25),
        Alpine | Tundra | Ocean => (mat::PATH_GRAVEL, mat::STONE, 0.3),
        OldGrowth | Rainforest => (mat::PATH_DARK, mat::MOSS, 0.15),
        Taiga => (mat::PATH_DARK, mat::PATH_GRAVEL, 0.3),
        Heath | Grassland | Deciduous | Woodland => (mat::PATH_DIRT, mat::PATH_GRAVEL, 0.3),
    };
    if roll * 0.6 + patch * 0.4 < mix { other } else { main }
}

impl World for GameWorld {
    fn materials(&self) -> &Materials {
        &self.materials
    }

    fn generate_column(&self, cx: i32, cz: i32) -> ColumnOutput {
        // A chunk never straddles macro-cells, and its one-voxel border can only reach
        // into the uniform deep sea that separates them.
        let (mx, mz) = cell_of(cx * CHUNK, cz * CHUNK);
        let continent = self.continent(mx, mz);
        let (wx0, wz0) = (cx * CHUNK - 1, cz * CHUNK - 1);
        let (lx0, lz0) = (wx0 - mx * MACRO_SIZE, wz0 - mz * MACRO_SIZE);
        // Features up to MAX_REACH away can lean into this column, so sample that far.
        let margin = features::MAX_REACH + 1;
        let sampler = Sampler::new(&continent, self.seed, lx0 - margin, lz0 - margin, PAD as i32 + 2 * margin);

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

        // Surface and subsurface voxel per column.
        let mut skins = Vec::with_capacity(PAD * PAD);
        let mut plants = Vec::new();
        for px in 0..PAD {
            for pz in 0..PAD {
                let top = columns[px * PAD + pz].top();
                // Slope of the ground here, in voxels of rise per voxel.
                let ground = |x: usize, z: usize| columns[x * PAD + z].ground;
                let (x0, x1) = (px.saturating_sub(1), (px + 1).min(PAD - 1));
                let (z0, z1) = (pz.saturating_sub(1), (pz + 1).min(PAD - 1));
                let gx = (ground(x1, pz) - ground(x0, pz)) / (x1 - x0) as f32;
                let gz = (ground(px, z1) - ground(px, z0)) / (z1 - z0) as f32;
                let steep = gx.hypot(gz);
                let roll = unit(hash3(self.seed ^ 0x5cf, wx0 + px as i32, top, wz0 + pz as i32));
                let (wx, wz) = ((wx0 + px as i32) as f64, (wz0 + pz as i32) as f64);
                let patch = perlin2(self.seed ^ 0x9a7c, wx / m64(7.2), wz / m64(7.2)) * 0.5 + 0.5;
                let col = &columns[px * PAD + pz];
                let skin = surface_materials(col, steep, roll, patch);
                let mut growing = skin.0;
                if skin.0 == mat::ground_cover(col.biome) && col.cover == Cover::Land
                    && let Some(f) = features::flower(col.biome, wx as i32, wz as i32, self.seed, roll)
                {
                    growing = f;
                }
                // The border columns belong to neighbouring chunk columns.
                if (1..=CHUNK as usize).contains(&px) && (1..=CHUNK as usize).contains(&pz) {
                    features::plants::grow(col, growing, wx as i32, wz as i32, self.seed, &mut plants);
                }
                skins.push(skin);
            }
        }

        let lib = self.library();
        let placements = features::place(
            self.seed,
            lib,
            &sampler,
            lx0,
            lz0,
            PAD as i32,
            (mx * MACRO_SIZE, mz * MACRO_SIZE),
        );
        // Only the features whose model can touch this column.
        let placements: Vec<_> = placements
            .into_iter()
            .filter(|p| {
                let t = lib.get(p.species, p.variant);
                p.pos.x + t.reach >= wx0
                    && p.pos.x - t.reach < wx0 + PAD as i32
                    && p.pos.z + t.reach >= wz0
                    && p.pos.z - t.reach < wz0 + PAD as i32
            })
            .collect();
        for p in &placements {
            hi = hi.max(p.top(lib));
            lo = lo.min(p.bottom());
        }
        // Landmarks, bridges and lanterns, whose voxels are continent-local.
        let local_chunk = (cx - mx * (MACRO_SIZE / CHUNK), cz - mz * (MACRO_SIZE / CHUNK));
        let stamps: Vec<_> = continent.trails.stamps_at(local_chunk.0, local_chunk.1).collect();
        for s in &stamps {
            hi = hi.max(s.hi.y);
            lo = lo.min(s.lo.y);
        }
        hi = plants.iter().fold(hi, |top, p| top.max((p.lo[1]).div_euclid(CELLS)));

        // Only chunks that can hold a visible face: from just under the lowest ground
        // to the highest ground or water or tree top.
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
                    let rock = (col.cover == Cover::Cliff).then(|| {
                        let (wx, wz) = ((wx0 + px as i32) as f64, (wz0 + pz as i32) as f64);
                        perlin2(self.seed ^ 0xc11d, wx / m64(70.0), wz / m64(70.0)) as f32 * m(2.4)
                    });
                    for (py, v) in chunk.column_mut(px, pz).iter_mut().enumerate() {
                        let y = base + py as i32;
                        *v = if y > top {
                            if y <= water_top { mat::WATER } else { AIR }
                        } else if y == top {
                            match rock {
                                Some(tilt) if surface == mat::STONE => mat::strata(self.seed, col.biome, y, tilt),
                                _ => surface,
                            }
                        } else if let Some(tilt) = rock
                            && y >= top - 3 * BLOCK
                            && under == mat::STONE
                        {
                            mat::strata(self.seed, col.biome, y, tilt)
                        } else if y >= top - 3 * BLOCK {
                            under
                        } else {
                            mat::STONE
                        };
                    }
                }
            }
            out.push((cy, chunk));
        }

        // Cascade lips: water columns whose neighbour's water surface is lower.
        let mut cascades = Vec::new();
        for px in 1..PAD - 1 {
            for pz in 1..PAD - 1 {
                let Some(k) = columns[px * PAD + pz].water_surface() else { continue };
                let (mut total, mut dir) = (0.0f32, Vec3::ZERO);
                for (dx, dz) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                    let n = &columns[(px as i32 + dx) as usize * PAD + (pz as i32 + dz) as usize];
                    if let Some(nk) = n.water_surface()
                        && nk < k
                    {
                        let drop = (k - nk) as f32;
                        total += drop;
                        dir += Vec3::new(dx as f32, 0.0, dz as f32) * drop;
                    }
                }
                if total > 0.0 {
                    cascades.push(Cascade {
                        pos: Vec3::new((wx0 + px as i32) as f32 + 0.5, k as f32, (wz0 + pz as i32) as f32 + 0.5),
                        dir: dir.normalize_or_zero(),
                        drop: total,
                    });
                }
            }
        }

        let mut put = |q: IVec3, v: Voxel, force: bool| {
            let (px, pz) = (q.x - wx0, q.z - wz0);
            // Height above the bottom of the lowest chunk (not its padded border).
            let py = q.y - cy_min * CHUNK;
            if px < 0 || pz < 0 || py < 0 || px >= PAD as i32 || pz >= PAD as i32 {
                return;
            }
            let (ci, ly) = ((py / CHUNK) as usize, py.rem_euclid(CHUNK) as usize);
            // Chunk `ci` holds padded y 0..PAD; rows 0 and 33 are the borders
            // of neighbouring chunks and are written there too.
            let mut write = |chunk_index: usize, padded_y: usize| {
                if let Some((_, chunk)) = out.get_mut(chunk_index) {
                    let (ux, uz) = (px as usize, pz as usize);
                    if force || chunk.get(ux, padded_y, uz) == AIR {
                        chunk.set(ux, padded_y, uz, v);
                    }
                }
            };
            // World y maps to padded y + 1 in its own chunk...
            write(ci, ly + 1);
            // ...and to the borders of the chunks below and above.
            if ly == 0 && ci > 0 {
                write(ci - 1, PAD - 1);
            }
            if ly == CHUNK as usize - 1 {
                write(ci + 1, 0);
            }
        };
        for p in &placements {
            features::stamp(lib, &self.materials, p, &mut put);
        }
        // Built things go last: they clear whatever grew or stood in their way.
        let origin = IVec3::new(mx * MACRO_SIZE, 0, mz * MACRO_SIZE);
        for s in &stamps {
            for &(p, v) in &s.voxels {
                put(p + origin, v, true);
            }
        }
        ColumnOutput {
            chunks: out,
            cascades,
            plants,
        }
    }
}
