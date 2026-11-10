//! Things standing on the terrain: trees, bushes, boulders, logs. Where they go and
//! how they are stamped into chunks.
//!
//! Placement works on fixed grids of cells in world space, one grid per size class,
//! with at most one feature per cell at a hashed position. Any chunk can therefore
//! work out which features overlap it by looking at the cells around it, so a tree
//! spanning many chunks is stamped identically by each of them.

pub mod aquatic;
pub mod attach;
pub mod climber;
pub mod landmarks;
pub mod moss;
pub mod parts;
pub mod pebbles;
pub mod plants;
pub mod shape;
pub mod trees;

use std::collections::HashMap;

use glam::{IVec2, IVec3, Vec2};
use voxel::{Kind, Materials, Voxel};

use crate::{
    biome::Biome,
    materials as mat,
    noise::{fbm2, hash3, unit},
    scale::{BLOCK, m, m64},
    surface::{Column, Cover, Sampler},
};
use shape::{Template, rotate};
use trees::Species;

/// Every species' pre-built variants. Built once per process.
pub struct Library {
    templates: HashMap<Species, Vec<Template>>,
}

impl Default for Library {
    fn default() -> Self {
        Self::build()
    }
}

impl Library {
    pub fn build() -> Self {
        let templates = Species::ALL
            .iter()
            .map(|&sp| {
                let variants = (0..sp.variants())
                    .map(|i| sp.build(0x51_7e3d ^ (i as u32).wrapping_mul(0x9e37_79b9)))
                    .collect();
                (sp, variants)
            })
            .collect();
        Self { templates }
    }

    pub fn get(&self, species: Species, variant: usize) -> &Template {
        let v = &self.templates[&species];
        &v[variant % v.len()]
    }
}

/// One feature to stamp: species variant at a world position (`y` is the ground
/// surface, where the model's origin goes), turned by quarter turns.
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    pub pos: IVec3,
    pub species: Species,
    pub variant: usize,
    pub turns: u32,
    /// How the model follows the ground, for those that lie along it.
    pub lie: Option<Lie>,
}

impl Placement {
    /// How far the far end of the model is above its origin, or below if negative.
    fn rise(&self) -> i32 {
        self.lie.map_or(0, |l| l.rise)
    }

    /// The highest voxel the model can reach.
    pub fn top(&self, lib: &Library) -> i32 {
        self.pos.y + lib.get(self.species, self.variant).top + self.rise().max(0)
    }

    /// The lowest voxel the model can reach.
    pub fn bottom(&self) -> i32 {
        self.pos.y - 4 * BLOCK + self.rise().min(0)
    }
}

/// How a long model lies on sloping ground: it is sheared from its origin to its far
/// end, `length` voxels away along `dir`, which is `rise` voxels higher. Anything past
/// `length` is left off, so a log can stop short of a drop or a river.
#[derive(Clone, Copy, Debug)]
pub struct Lie {
    dir: Vec2,
    length: f32,
    rise: i32,
}

impl Lie {
    /// How far to raise the voxel column at this offset from the origin, or None if the
    /// log does not reach it.
    fn height_at(&self, x: i32, z: i32) -> Option<i32> {
        let along = (Vec2::new(x as f32, z as f32).dot(self.dir)).max(0.0);
        (along <= self.length).then(|| (along / self.length * self.rise as f32).round() as i32)
    }
}

/// Steepest slope a log will lie on, in voxels of rise per voxel.
const LOG_MAX_SLOPE: f32 = 0.7;
/// How far the ground may stray from a straight log, up or down: a log bridges a small
/// dip and beds into a small hump.
const LOG_MAX_GAP: f32 = 3.0;
/// Shares of its full length a log may be shortened to, to fit the ground.
const LOG_LENGTHS: [f32; 3] = [1.0, 0.8, 0.6];

/// Fits a log running from the origin to offset `far`. `start` is the height it rests
/// at on its origin column, and `ground` the same for the column at an offset, or None
/// where a log may not lie.
fn fit_log(far: IVec2, start: i32, ground: impl Fn(i32, i32) -> Option<i32>) -> Option<Lie> {
    let full = far.as_vec2().length();
    if full < 1.0 {
        return None;
    }
    let dir = far.as_vec2() / full;
    let at = |s: f32| {
        let p = dir * s;
        ground(p.x.round() as i32, p.y.round() as i32)
    };
    'lengths: for share in LOG_LENGTHS {
        let length = full * share;
        let Some(end) = at(length) else { continue };
        let rise = end - start;
        if rise.abs() as f32 > length * LOG_MAX_SLOPE {
            continue;
        }
        let samples = (length / 2.0).ceil() as i32;
        for k in 1..samples {
            let t = k as f32 / samples as f32;
            let Some(g) = at(length * t) else { continue 'lengths };
            if (start as f32 + rise as f32 * t - g as f32).abs() > LOG_MAX_GAP {
                continue 'lengths;
            }
        }
        return Some(Lie { dir, length, rise });
    }
    None
}

/// Where a small model with a footprint `base` voxels in radius should stand so that no
/// side of it hangs in the air, or None if the ground is too uneven. `ground` gives the
/// height of the surface at an offset, or None over water.
fn settle(species: Species, base: i32, start: i32, ground: impl Fn(i32, i32) -> Option<i32>) -> Option<i32> {
    let (mut lowest, mut highest) = (start, start);
    for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
        if let Some(g) = ground(dx * base, dz * base) {
            lowest = lowest.min(g);
            highest = highest.max(g);
        }
    }
    if matches!(species, Species::Boulder | Species::MossyBoulder) {
        // Rock can sit on any ground, but should not sink out of sight.
        return Some(lowest.max(start - 2 * BLOCK));
    }
    (highest - lowest <= 3 * BLOCK).then_some(lowest)
}

/// Size classes. Each is a grid of square cells in world space.
#[derive(Clone, Copy)]
struct Layer {
    cell: i32,
    /// Farthest any model in this layer reaches from its origin, horizontally.
    reach: i32,
    salt: u32,
}

const GIANTS: Layer = Layer {
    cell: 48 * BLOCK,
    reach: 24 * BLOCK,
    salt: 0x61a7,
};
const TREES: Layer = Layer {
    cell: 11 * BLOCK,
    reach: 10 * BLOCK,
    salt: 0x7ee5,
};
const SMALL: Layer = Layer {
    cell: 6 * BLOCK,
    reach: 12 * BLOCK,
    salt: 0x5a11,
};

/// What grows in a biome: weighted species per layer and how full each layer is.
struct Flora {
    giants: &'static [(Species, f32)],
    giant_density: f32,
    trees: &'static [(Species, f32)],
    tree_density: f32,
    small: &'static [(Species, f32)],
    small_density: f32,
}

fn flora(biome: Biome) -> Flora {
    use Species::*;
    macro_rules! f {
        ($g:expr, $gd:expr, $t:expr, $td:expr, $s:expr, $sd:expr) => {
            Flora {
                giants: &$g,
                giant_density: $gd,
                trees: &$t,
                tree_density: $td,
                small: &$s,
                small_density: $sd,
            }
        };
    }
    match biome {
        Biome::Ocean | Biome::Alpine => f!([], 0.0, [], 0.0, [(Boulder, 1.0)], 0.06),
        Biome::Tundra => f!([], 0.0, [(DeadSnag, 1.0)], 0.01, [(Boulder, 3.0), (Bush, 1.0)], 0.08),
        Biome::Taiga => f!(
            [(TallPine, 1.0)],
            0.12,
            [(Spruce, 10.0), (Birch, 1.0), (DeadSnag, 0.4)],
            0.62,
            [(Boulder, 2.0), (MossyBoulder, 2.0), (FallenLog, 2.0), (Bush, 1.0), (Stump, 0.5)],
            0.14
        ),
        Biome::Heath => f!([], 0.0, [(Birch, 2.0), (Spruce, 1.0), (DeadSnag, 0.5)], 0.05, [(Boulder, 3.0), (Bush, 2.0)], 0.1),
        Biome::Grassland => f!([(ElderOak, 1.0)], 0.02, [(Oak, 3.0), (Birch, 1.0)], 0.04, [(Bush, 2.0), (Boulder, 1.0)], 0.07),
        Biome::Deciduous => f!(
            [(ElderOak, 1.0)],
            0.05,
            [(Oak, 7.0), (Birch, 2.5), (Maple, 1.5), (Cherry, 0.4)],
            0.58,
            [(Bush, 4.0), (FallenLog, 1.5), (Stump, 0.6), (MossyBoulder, 0.8), (Boulder, 0.4)],
            0.16
        ),
        Biome::OldGrowth => f!(
            [(Redwood, 1.0)],
            0.6,
            [(Spruce, 2.0), (Oak, 1.0), (Birch, 0.5)],
            0.2,
            [(Fern, 5.0), (MossyBoulder, 2.0), (FallenLog, 2.5), (Stump, 0.8)],
            0.24
        ),
        Biome::Desert => f!([], 0.0, [(Cactus, 1.0)], 0.03, [(Boulder, 2.0), (Cactus, 1.0)], 0.05),
        Biome::Savanna => f!([(Baobab, 1.0)], 0.1, [(Acacia, 1.0)], 0.07, [(Bush, 3.0), (Boulder, 1.0)], 0.08),
        Biome::Woodland => f!(
            [(ElderOak, 1.0)],
            0.03,
            [(Oak, 5.0), (Cherry, 2.0), (Birch, 1.5), (Acacia, 0.5)],
            0.4,
            [(Bush, 4.0), (FallenLog, 1.0), (Boulder, 0.6)],
            0.14
        ),
        Biome::Rainforest => f!(
            [(Kapok, 1.0)],
            0.45,
            [(Jungle, 8.0), (Palm, 1.0)],
            0.7,
            [(Fern, 6.0), (Bush, 3.0), (FallenLog, 1.5), (MossyBoulder, 1.0)],
            0.3
        ),
        Biome::Marsh => f!([], 0.0, [(Willow, 4.0), (Birch, 1.0)], 0.07, [(Bush, 3.0), (Stump, 0.5)], 0.08),
        Biome::BirchGrove => f!(
            [],
            0.0,
            [(Birch, 12.0), (Spruce, 0.6), (Oak, 0.4)],
            0.55,
            [(Fern, 3.0), (Bush, 2.0), (FallenLog, 1.0), (MossyBoulder, 0.6)],
            0.16
        ),
        Biome::AlpineMeadow => f!([], 0.0, [(Spruce, 1.0), (DeadSnag, 0.3)], 0.01, [(Boulder, 3.0), (MossyBoulder, 1.0)], 0.08),
        Biome::SnowyPine => f!(
            [(TallPine, 1.0)],
            0.1,
            [(Spruce, 12.0), (DeadSnag, 0.3)],
            0.7,
            [(Boulder, 2.0), (FallenLog, 1.0), (Stump, 0.5)],
            0.08
        ),
        Biome::Bog => f!([], 0.0, [(DeadSnag, 2.0), (Spruce, 1.5), (Birch, 0.6)], 0.06, [(Bush, 3.0), (MossyBoulder, 1.0), (Stump, 1.0)], 0.1),
        Biome::Steppe => f!([(ElderOak, 1.0)], 0.006, [(Oak, 1.0), (Acacia, 0.3)], 0.006, [(Bush, 2.0), (Boulder, 1.5)], 0.05),
        Biome::DuneSea => f!([], 0.0, [], 0.0, [(Boulder, 1.0)], 0.004),
        Biome::RedRock => f!([], 0.0, [(Cactus, 1.0)], 0.015, [(Boulder, 3.0), (Bush, 1.0), (Cactus, 1.0)], 0.06),
        Biome::Mangrove => f!([], 0.0, [(Willow, 5.0), (Jungle, 1.0), (Palm, 0.6)], 0.4, [(Fern, 3.0), (Bush, 2.0)], 0.12),
        Biome::Volcanic => f!([], 0.0, [(Palm, 1.0), (DeadSnag, 0.5)], 0.02, [(Fern, 2.0), (Boulder, 3.0)], 0.08),
    }
}

/// Canopy colour, relative cover and height in 0.8 m blocks of a biome's forest, for
/// drawing it from afar.
pub fn canopy(biome: Biome, mats: &Materials) -> Option<([u8; 3], f32, f32)> {
    let fl = flora(biome);
    let (species, _) = fl.trees.iter().chain(fl.giants).max_by(|a, b| a.1.total_cmp(&b.1))?;
    let leaves = match species {
        Species::Spruce | Species::TallPine => mat::NEEDLES,
        Species::Redwood => mat::NEEDLES_REDWOOD,
        Species::Jungle | Species::Kapok => mat::LEAVES_JUNGLE,
        Species::Acacia | Species::Baobab => mat::LEAVES_ACACIA,
        Species::Birch => mat::LEAVES_BIRCH,
        Species::Cactus | Species::DeadSnag => return None,
        _ => mat::LEAVES_OAK,
    };
    let cover = (fl.tree_density * 1.3 + fl.giant_density * 0.6).min(1.0);
    let height = match species {
        Species::Redwood => 60.0,
        Species::Kapok => 40.0,
        Species::TallPine => 30.0,
        _ => 14.0,
    };
    Some((mats.get(leaves).color, cover, height))
}

/// The tree a biome's forest is mostly made of, as [`canopy`] chooses it, for drawing
/// that forest from afar in its own colours.
pub fn canopy_tree(biome: Biome) -> Option<Species> {
    let fl = flora(biome);
    fl.trees.iter().chain(fl.giants).max_by(|a, b| a.1.total_cmp(&b.1)).map(|t| t.0)
}

fn weighted(list: &[(Species, f32)], roll: f32) -> Species {
    let total: f32 = list.iter().map(|w| w.1).sum();
    let mut r = roll * total;
    for &(sp, w) in list {
        if r < w {
            return sp;
        }
        r -= w;
    }
    list[list.len() - 1].0
}

/// Features overlapping the square of columns at continent-local `(lx0, lz0)`, `span`
/// wide. `sampler` must cover that square plus the largest reach.
pub fn place(
    world_seed: u32,
    lib: &Library,
    sampler: &Sampler,
    lx0: i32,
    lz0: i32,
    span: i32,
    macro_origin: (i32, i32),
) -> Vec<Placement> {
    let mut out = Vec::new();
    let mut giants: Vec<IVec3> = Vec::new();
    for layer in [GIANTS, TREES, SMALL] {
        let (wx0, wz0) = (macro_origin.0 + lx0, macro_origin.1 + lz0);
        let (c0x, c1x) = ((wx0 - layer.reach).div_euclid(layer.cell), (wx0 + span + layer.reach).div_euclid(layer.cell));
        let (c0z, c1z) = ((wz0 - layer.reach).div_euclid(layer.cell), (wz0 + span + layer.reach).div_euclid(layer.cell));
        for cz in c0z..=c1z {
            for cx in c0x..=c1x {
                let h = hash3(world_seed ^ layer.salt, cx, cz, 0);
                let roll = |k: u32| unit(h.rotate_left(k * 5).wrapping_mul(0x2c1b_3c6d ^ k));
                // Position within the cell, keeping a margin so neighbours do not touch.
                let x = cx * layer.cell + BLOCK + (roll(1) * (layer.cell - 2 * BLOCK) as f32) as i32;
                let z = cz * layer.cell + BLOCK + (roll(2) * (layer.cell - 2 * BLOCK) as f32) as i32;
                let (lx, lz) = (x - macro_origin.0, z - macro_origin.1);
                let col = sampler.column(lx, lz);
                let Some(species) = choose(&col, &layer, roll(3), roll(4), x, z, world_seed) else {
                    continue;
                };
                let mut pos = IVec3::new(x, col.top() + 1, z);
                if layer.salt == GIANTS.salt {
                    giants.push(pos);
                } else if layer.salt == TREES.salt
                    && giants.iter().any(|g| (g.x - x).abs() < 6 * BLOCK && (g.z - z).abs() < 6 * BLOCK)
                {
                    continue;
                }
                // Steep ground: nothing roots there but the rock itself.
                let rise = (sampler.column(lx + 1, lz).ground - sampler.column(lx - 1, lz).ground)
                    .abs()
                    .max((sampler.column(lx, lz + 1).ground - sampler.column(lx, lz - 1).ground).abs());
                if rise > 1.6 && !matches!(species, Species::Boulder | Species::MossyBoulder) {
                    continue;
                }
                let variant = (roll(5) * 64.0) as usize;
                let turns = (roll(6) * 4.0) as u32;
                let mut lie = None;
                if layer.salt == SMALL.salt {
                    let model = lib.get(species, variant);
                    if let Some(axis) = model.axis {
                        // A log needs clear, dry ground along its whole length.
                        let bed = |dx: i32, dz: i32| {
                            let c = sampler.column(lx + dx, lz + dz);
                            let dry = matches!(c.cover, Cover::Land | Cover::Bank) && c.water_surface().is_none();
                            (dry && c.clear >= m(1.2)).then_some(c.top() + 1)
                        };
                        let far = rotate(axis, turns);
                        lie = fit_log(IVec2::new(far.x, far.z), pos.y, bed);
                        if lie.is_none() {
                            continue;
                        }
                        // Bedded in a little, so neither end shows daylight.
                        pos.y -= 1;
                    } else {
                        let surface = |dx: i32, dz: i32| {
                            let c = sampler.column(lx + dx, lz + dz);
                            c.water_surface().is_none().then_some(c.top() + 1)
                        };
                        let Some(y) = settle(species, model.base, pos.y, surface) else {
                            continue;
                        };
                        pos.y = y;
                    }
                }
                out.push(Placement {
                    pos,
                    species,
                    variant,
                    turns,
                    lie,
                });
            }
        }
    }
    out
}

/// Decides whether a feature grows at a column and which, or None.
fn choose(col: &Column, layer: &Layer, roll_density: f32, roll_species: f32, x: i32, z: i32, seed: u32) -> Option<Species> {
    // Trails and landmarks keep their surroundings clear; big trunks need more room.
    let room = match layer.salt {
        s if s == GIANTS.salt => m(6.4),
        s if s == TREES.salt => m(2.8),
        _ => m(1.2),
    };
    if col.clear < room {
        return None;
    }
    let fl = flora(col.biome);
    // Forests thin out and open into clearings on a slow noise, so they are not
    // uniform carpets.
    let openness = fbm2(seed ^ 0xc1ea, x as f64 / m64(76.0), z as f64 / m64(76.0), 3, 2.0, 0.5);
    let clearing = ((openness + 0.35) / 0.5).clamp(0.0, 1.0);
    let (list, density): (&[(Species, f32)], f32) = match layer.salt {
        s if s == GIANTS.salt => (fl.giants, fl.giant_density * (0.5 + 0.5 * clearing)),
        s if s == TREES.salt => (fl.trees, fl.tree_density * clearing),
        _ => (fl.small, fl.small_density),
    };
    let too_cold = col.temperature < 0.13;
    match col.cover {
        Cover::Beach if layer.salt == TREES.salt && col.temperature > 0.6 && roll_density < 0.12 => {
            return Some(Species::Palm);
        }
        Cover::Land => {}
        Cover::Bank if layer.salt == SMALL.salt => {}
        _ => return None,
    }
    if col.water_surface().is_some() {
        return None;
    }
    // Riverbanks in temperate and warm country grow willows; in the hot dry uplands,
    // the water makes oases of palms.
    if layer.salt == TREES.salt
        && col.river < m(7.2)
        && col.temperature > 0.3
        && !matches!(col.biome, Biome::Desert | Biome::Savanna | Biome::DuneSea)
        && roll_density < 0.35
    {
        return Some(if col.biome == Biome::RedRock { Species::Palm } else { Species::Willow });
    }
    if list.is_empty() || roll_density >= density {
        return None;
    }
    let mut species = weighted(list, roll_species);
    // Trees grow in groves of their own kind rather than evenly mixed: a slow noise
    // hands whole stretches of forest to one of the minority species.
    if layer.salt == TREES.salt && matches!(species, Species::Oak | Species::Birch | Species::Maple | Species::Cherry) {
        let grove = fbm2(seed ^ 0x9e0f, x as f64 / m64(112.0), z as f64 / m64(112.0), 2, 2.0, 0.5);
        species = match col.biome {
            Biome::Deciduous | Biome::Woodland if grove > 0.38 => Species::Maple,
            Biome::Woodland if grove < -0.40 => Species::Cherry,
            Biome::Deciduous if grove < -0.42 => Species::Birch,
            Biome::Deciduous | Biome::Woodland if matches!(species, Species::Maple | Species::Cherry) => Species::Oak,
            _ => species,
        };
    }
    if too_cold && !matches!(species, Species::Boulder | Species::MossyBoulder) {
        return None;
    }
    Some(species)
}

/// Writes a placed feature into a column of chunks. `set` receives world voxel
/// positions and is expected to ignore anything outside its range.
pub fn stamp(lib: &Library, mats: &Materials, p: &Placement, mut set: impl FnMut(IVec3, Voxel, bool)) {
    let t = lib.get(p.species, p.variant);
    for &(offset, v) in &t.voxels {
        let turned = rotate(offset, p.turns);
        let mut q = p.pos + turned;
        if let Some(lie) = &p.lie {
            let Some(dy) = lie.height_at(turned.x, turned.z) else {
                continue;
            };
            q.y += dy;
        }
        // Wood and rock displace whatever is there; leaves only fill empty space.
        let force = mats.get(v).kind == Kind::Solid;
        set(q, v, force);
    }
}

/// The voxel a placed feature has at world position `q`, if any: the same answer
/// [`stamp`] gives, for reading a feature without stamping it.
pub fn model_voxel(lib: &Library, p: &Placement, q: IVec3) -> Option<Voxel> {
    let t = lib.get(p.species, p.variant);
    let d = q - p.pos;
    if d.x.abs().max(d.z.abs()) > t.reach || q.y > p.top(lib) || q.y < p.bottom() {
        return None;
    }
    let mut turned = d;
    if let Some(lie) = &p.lie {
        turned.y -= lie.height_at(d.x, d.z)?;
    }
    let offset = rotate(turned, p.turns.wrapping_neg());
    t.voxels.binary_search_by_key(&(offset.y, offset.x, offset.z), |(v, _)| (v.y, v.x, v.z)).ok().map(|i| t.voxels[i].1)
}

/// How high a feature's top can be above its origin, over all templates. Chunk
/// columns use it to know how far up to generate.
pub fn max_top(lib: &Library) -> i32 {
    lib.templates
        .values()
        .flatten()
        .map(|t| t.top)
        .max()
        .unwrap_or(0)
}

/// How far from a chunk column the ground can matter to it: the widest feature reach,
/// or a small feature that close whose far end looks at the ground as far again.
pub const MAX_REACH: i32 = if GIANTS.reach > 2 * SMALL.reach { GIANTS.reach } else { 2 * SMALL.reach };

/// Flowers and heather: drifts of colour on grass, one species per drift.
pub fn flower(biome: Biome, x: i32, z: i32, seed: u32, roll: f32) -> Option<Voxel> {
    let (kinds, density): (&[Voxel], f32) = match biome {
        Biome::Grassland => (
            &[mat::FLOWER_YELLOW, mat::FLOWER_WHITE, mat::FLOWER_RED, mat::FLOWER_BLUE, mat::FLOWER_PURPLE],
            0.28,
        ),
        Biome::Deciduous | Biome::Woodland => (&[mat::FLOWER_WHITE, mat::FLOWER_YELLOW, mat::FLOWER_BLUE], 0.16),
        Biome::Heath => (&[mat::HEATHER, mat::HEATHER, mat::FLOWER_YELLOW], 0.5),
        Biome::Tundra => (&[mat::FLOWER_WHITE, mat::FLOWER_PURPLE], 0.1),
        Biome::Savanna => (&[mat::FLOWER_YELLOW], 0.08),
        Biome::Rainforest => (&[mat::FLOWER_RED, mat::FLOWER_PINK], 0.08),
        Biome::AlpineMeadow => (&[mat::FLOWER_BLUE, mat::FLOWER_PURPLE, mat::FLOWER_WHITE, mat::FLOWER_YELLOW], 0.45),
        Biome::Marsh => (&[mat::FLOWER_YELLOW, mat::FLOWER_WHITE], 0.12),
        Biome::BirchGrove => (&[mat::FLOWER_WHITE, mat::FLOWER_BLUE], 0.14),
        Biome::Steppe => (&[mat::FLOWER_YELLOW, mat::FLOWER_PURPLE], 0.06),
        Biome::Bog => (&[mat::FLOWER_WHITE], 0.06),
        _ => return None,
    };
    // A fast noise marks the drifts; a slow one chooses what grows in each.
    let patch = fbm2(seed ^ 0xf10a, x as f64 / m64(27.2), z as f64 / m64(27.2), 2, 2.0, 0.5);
    if patch < 0.22 {
        return None;
    }
    let which = fbm2(seed ^ 0xf10b, x as f64 / m64(128.0), z as f64 / m64(128.0), 2, 2.0, 0.5) * 0.5 + 0.5;
    let edge = ((patch - 0.22) / 0.2).clamp(0.3, 1.0);
    (roll < density * edge).then(|| kinds[((which * kinds.len() as f32) as usize).min(kinds.len() - 1)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_models_fit_within_the_reach_chunks_look_for() {
        let lib = Library::build();
        let small = [
            Species::Bush,
            Species::Fern,
            Species::Boulder,
            Species::MossyBoulder,
            Species::FallenLog,
            Species::Stump,
            Species::Cactus,
        ];
        for species in small {
            for variant in 0..species.variants() {
                let reach = lib.get(species, variant).reach;
                assert!(reach <= SMALL.reach, "{species:?} reaches {reach}, chunks only look {}", SMALL.reach);
            }
        }
    }

    #[test]
    fn a_log_lies_flush_along_a_slope() {
        let lib = Library::build();
        for variant in 0..Species::FallenLog.variants() {
            let model = lib.get(Species::FallenLog, variant);
            for turns in 0..4 {
                for slope in [-0.5f32, -0.2, 0.0, 0.3, 0.6] {
                    let far = rotate(model.axis.expect("logs have an axis"), turns);
                    let dir = Vec2::new(far.x as f32, far.z as f32).normalize();
                    let ground = move |x: i32, z: i32| 100 + (Vec2::new(x as f32, z as f32).dot(dir) * slope).round() as i32;
                    let lie = fit_log(IVec2::new(far.x, far.z), 100, |x, z| Some(ground(x, z))).expect("an even slope is fine");
                    let p = Placement {
                        pos: IVec3::new(0, 99, 0),
                        species: Species::FallenLog,
                        variant,
                        turns,
                        lie: Some(lie),
                    };
                    let mut bottoms: HashMap<(i32, i32), i32> = HashMap::new();
                    stamp(&lib, &mat::build(), &p, |q, _, _| {
                        let b = bottoms.entry((q.x, q.z)).or_insert(q.y);
                        *b = (*b).min(q.y);
                    });
                    assert!(!bottoms.is_empty());
                    // The columns on the log's centre line show how it sits on the ground.
                    for (&(x, z), &bottom) in &bottoms {
                        let v = Vec2::new(x as f32, z as f32);
                        let along = v.dot(dir);
                        if (v - dir * along).length() < 1.0 && along > 1.0 && along < lie.length - 1.0 {
                            let g = ground(x, z);
                            assert!(bottom <= g + 2 && bottom >= g - 3, "slope {slope}: bottom {bottom} on ground {g} at {x},{z}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_log_stops_short_of_a_drop_and_gives_up_on_a_cliff() {
        let far = IVec2::new(20, 0);
        // Water from 18 voxels out: the shorter length fits.
        let lie = fit_log(far, 50, |x, _| (x < 18).then_some(50)).expect("shortened");
        assert!(lie.length < 18.0);
        // A cliff right at the start: nothing fits.
        assert!(fit_log(far, 50, |x, _| Some(if x > 3 { 30 } else { 50 })).is_none());
        // A deep dip mid-way is too wide to bridge, a shallow one is not.
        let dip = |depth: i32| move |x: i32, _: i32| Some(if (8..12).contains(&x) { 50 - depth } else { 50 });
        assert!(fit_log(far, 50, dip(2)).is_some());
        assert!(fit_log(far, 50, dip(8)).is_none());
    }

    #[test]
    fn bushes_stand_on_the_lowest_ground_and_avoid_rough_ground() {
        let gentle = |x: i32, _: i32| Some(50 + x / 3);
        assert_eq!(settle(Species::Bush, 6, 50, gentle), Some(48));
        let rough = |x: i32, _: i32| Some(50 + x * 2);
        assert_eq!(settle(Species::Bush, 6, 50, rough), None);
        // Water does not drag a bush down into the river.
        let bank = |x: i32, _: i32| (x >= 0).then_some(50);
        assert_eq!(settle(Species::Bush, 6, 50, bank), Some(50));
        // Rock always lands, without sinking out of sight.
        assert_eq!(settle(Species::Boulder, 6, 50, rough), Some(50 - 2 * BLOCK));
    }
}
