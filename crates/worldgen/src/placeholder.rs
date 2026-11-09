//! Stand-in terrain used to bring up the engine before the real world generator exists.
//! Plain fractal noise hills with a sea; no rivers, biomes or features.

use voxel::{CHUNK, Materials, PAD, PaddedChunk};

use crate::{World, materials as mat, noise};

pub struct PlaceholderWorld {
    seed: u32,
    materials: Materials,
}

const SEA_LEVEL: i32 = 0;

impl PlaceholderWorld {
    pub fn new(seed: u32) -> Self {
        Self {
            seed,
            materials: mat::build(),
        }
    }

    fn height(&self, x: i32, z: i32) -> i32 {
        let (fx, fz) = (x as f64, z as f64);
        let continent = noise::fbm2(self.seed, fx / 900.0, fz / 900.0, 3, 2.0, 0.5);
        let hills = noise::fbm2(self.seed ^ 0x51, fx / 180.0, fz / 180.0, 5, 2.0, 0.5);
        let ridges = noise::ridged2(self.seed ^ 0xa7, fx / 420.0, fz / 420.0, 4, 2.1, 0.5);
        let mountain = ((continent - 0.05) * 3.0).clamp(0.0, 1.0);
        (continent * 60.0 + hills * 22.0 + ridges * mountain * 150.0 + 8.0).round() as i32
    }
}

impl World for PlaceholderWorld {
    fn materials(&self) -> &Materials {
        &self.materials
    }

    fn generate_column(&self, cx: i32, cz: i32) -> Vec<(i32, PaddedChunk)> {
        // Surface heights for the chunk footprint plus its one-voxel border.
        let mut heights = [0i32; PAD * PAD];
        let (mut lo, mut hi) = (i32::MAX, i32::MIN);
        for px in 0..PAD {
            for pz in 0..PAD {
                let h = self.height(cx * CHUNK + px as i32 - 1, cz * CHUNK + pz as i32 - 1);
                heights[px * PAD + pz] = h;
                lo = lo.min(h);
                hi = hi.max(h);
            }
        }
        // Only chunks that can contain a visible face: from just under the lowest
        // surface up to the highest surface or the sea, whichever is higher.
        let cy_min = (lo - 2).div_euclid(CHUNK);
        let cy_max = hi.max(SEA_LEVEL).div_euclid(CHUNK);

        let mut out = Vec::new();
        for cy in cy_min..=cy_max {
            let base = cy * CHUNK - 1; // world y of padded y = 0
            let mut chunk = PaddedChunk::new();
            for px in 0..PAD {
                for pz in 0..PAD {
                    let h = heights[px * PAD + pz];
                    let col = chunk.column_mut(px, pz);
                    for (py, v) in col.iter_mut().enumerate() {
                        let y = base + py as i32;
                        *v = if y > h {
                            if y <= SEA_LEVEL { mat::WATER } else { voxel::AIR }
                        } else if y == h {
                            if h <= SEA_LEVEL + 2 {
                                mat::SAND
                            } else if h > 150 {
                                mat::SNOW
                            } else if h > 110 {
                                mat::STONE
                            } else {
                                mat::GRASS
                            }
                        } else if y > h - 4 {
                            if h <= SEA_LEVEL + 2 { mat::SAND } else { mat::DIRT }
                        } else {
                            mat::STONE
                        };
                    }
                }
            }
            if !chunk.is_uniform() {
                out.push((cy, chunk));
            }
        }
        out
    }
}
