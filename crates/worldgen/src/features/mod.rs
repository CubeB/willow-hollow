//! Things standing on the terrain: trees, bushes, boulders, logs. Where they go and
//! how they are stamped into chunks.
//!
//! Placement works on fixed grids of cells in world space, one grid per size class,
//! with at most one feature per cell at a hashed position. Any chunk can therefore
//! work out which features overlap it by looking at the cells around it, so a tree
//! spanning many chunks is stamped identically by each of them.

pub mod aquatic;
pub mod attach;
pub mod cave;
pub mod climber;
pub mod cloth;
pub mod designs;
pub mod fittings;
pub mod landmarks;
pub mod moorstones;
pub mod moss;
pub mod nests;
pub mod parts;
pub mod pebbles;
pub mod plants;
pub mod props;
pub mod roots;
pub mod shape;
pub mod tor;
pub mod tree_detail;
pub mod trees;

use std::{collections::HashMap, sync::Mutex};

use glam::{IVec2, IVec3, Vec2};
use voxel::{Kind, Materials, Voxel};

use crate::{
    biome::Biome,
    materials as mat,
    noise::{fbm2, hash2, hash3, unit},
    scale::{BLOCK, m, m64},
    surface::{Column, Cover, Sampler},
};
use shape::{Template, rotate};
use trees::Species;

/// Every species' pre-built variants. Built once per process.
pub struct Library {
    /// Each species' variants, by the species' number: looked up for every voxel read
    /// out of a model, so by index and not by hashing.
    templates: Vec<Vec<Template>>,
}

impl Default for Library {
    fn default() -> Self {
        Self::build()
    }
}

impl Library {
    pub fn build() -> Self {
        let mut templates: Vec<Vec<Template>> = Species::ALL.iter().map(|_| Vec::new()).collect();
        for sp in Species::ALL {
            templates[sp as usize] = (0..sp.variants())
                .map(|i| sp.build(0x51_7e3d ^ (i as u32).wrapping_mul(0x9e37_79b9)))
                .collect();
        }
        Self { templates }
    }

    #[inline]
    pub fn get(&self, species: Species, variant: usize) -> &Template {
        let v = &self.templates[species as usize];
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
    /// Roots bared where the ground falls away from a tree.
    pub roots: Option<roots::Roots>,
}

impl Placement {
    /// How far the far end of the model is above its origin, or below if negative.
    fn rise(&self) -> i32 {
        self.lie.map_or(0, |l| l.rise)
    }

    /// How far across the ground from `pos` any of it lies: its model, or a big tree's
    /// buttresses where they spread further.
    pub fn reach(&self, lib: &Library) -> i32 {
        let model = lib.get(self.species, self.variant).reach;
        self.roots.map_or(model, |r| model.max(r.spread()))
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
            [(Spruce, 10.0), (ScotsPine, 1.5), (Birch, 1.0), (DeadSnag, 0.4)],
            0.62,
            [(Boulder, 2.0), (MossyBoulder, 2.0), (FallenLog, 2.0), (Bush, 1.0), (Stump, 0.5)],
            0.14
        ),
        Biome::Heath => f!(
            [],
            0.0,
            [(Birch, 2.0), (ScotsPine, 1.6), (Juniper, 1.6), (Spruce, 0.6), (DeadSnag, 0.5)],
            0.06,
            [(Boulder, 3.0), (Gorse, 3.5), (Bush, 1.5)],
            0.14
        ),
        Biome::Grassland => f!([(ElderOak, 1.0)], 0.02, [(Oak, 3.0), (Birch, 1.0)], 0.04, [(Bush, 2.0), (Boulder, 1.0), (HollyBush, 0.6)], 0.08),
        Biome::Deciduous => f!(
            [(ElderOak, 1.0)],
            0.05,
            [(Oak, 7.0), (Birch, 2.5), (Maple, 1.5), (Cherry, 0.4)],
            0.58,
            [(Bush, 4.0), (FallenLog, 1.5), (Stump, 0.6), (MossyBoulder, 0.8), (Boulder, 0.4), (HollyBush, 1.6)],
            0.195
        ),
        Biome::OldGrowth => f!(
            [(Redwood, 1.0)],
            0.6,
            [(Spruce, 2.0), (Oak, 1.0), (Birch, 0.5), (Yew, 0.6), (Holly, 0.5)],
            0.24,
            [(Fern, 5.0), (MossyBoulder, 2.0), (FallenLog, 2.5), (Stump, 0.8), (HollyBush, 1.0)],
            0.26
        ),
        Biome::Desert => f!([], 0.0, [(Cactus, 1.0)], 0.03, [(Boulder, 2.0), (Cactus, 1.0)], 0.05),
        Biome::Savanna => f!([(Baobab, 1.0)], 0.1, [(Acacia, 1.0)], 0.07, [(Bush, 3.0), (Boulder, 1.0)], 0.08),
        Biome::Woodland => f!(
            [(ElderOak, 1.0)],
            0.03,
            [(Oak, 5.0), (Cherry, 2.0), (Birch, 1.5), (Acacia, 0.5)],
            0.4,
            [(Bush, 4.0), (FallenLog, 1.0), (Boulder, 0.6), (HollyBush, 1.4)],
            0.175
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
            [(Fern, 3.0), (Bush, 2.0), (FallenLog, 1.0), (MossyBoulder, 0.6), (HollyBush, 0.8)],
            0.18
        ),
        Biome::AlpineMeadow => f!([], 0.0, [(Spruce, 1.0), (Juniper, 0.9), (DeadSnag, 0.3)], 0.016, [(Boulder, 3.0), (MossyBoulder, 1.0)], 0.08),
        Biome::SnowyPine => f!(
            [(TallPine, 1.0)],
            0.1,
            [(Spruce, 12.0), (DeadSnag, 0.3)],
            0.7,
            [(Boulder, 2.0), (FallenLog, 1.0), (Stump, 0.5)],
            0.08
        ),
        Biome::Bog => f!(
            [],
            0.0,
            [(DeadSnag, 2.0), (Spruce, 1.5), (ScotsPine, 0.8), (Birch, 0.6)],
            0.07,
            [(Bush, 3.0), (MossyBoulder, 1.0), (Stump, 1.0)],
            0.1
        ),
        Biome::Steppe => f!([(ElderOak, 1.0)], 0.006, [(Oak, 1.0), (Juniper, 0.6), (Acacia, 0.3)], 0.009, [(Bush, 2.0), (Boulder, 1.5)], 0.05),
        Biome::DuneSea => f!([], 0.0, [], 0.0, [(Boulder, 1.0)], 0.004),
        Biome::RedRock => f!([], 0.0, [(Cactus, 1.0)], 0.015, [(Boulder, 3.0), (Bush, 1.0), (Cactus, 1.0)], 0.06),
        Biome::Mangrove => f!([], 0.0, [(Willow, 5.0), (Jungle, 1.0), (Palm, 0.6)], 0.4, [(Fern, 3.0), (Bush, 2.0)], 0.12),
        Biome::Volcanic => f!([], 0.0, [(Palm, 1.0), (DeadSnag, 0.5)], 0.02, [(Fern, 2.0), (Boulder, 3.0)], 0.08),
        // Almost no trees: a lone rowan-like birch or a juniper here and there. Gorse in
        // clumps and boulders in the grass; the tors and their clitter are features::tor.
        Biome::Moor => f!(
            [],
            0.0,
            [(Birch, 1.0), (Juniper, 0.8), (DeadSnag, 0.2)],
            0.0025,
            [(Gorse, 3.0), (Boulder, 2.0), (MossyBoulder, 0.8), (Bush, 0.4)],
            0.05
        ),
    }
}

/// The bark of the trees a biome's builders have to hand: its commonest tree, where it
/// is wooded enough to build from. `None` in open country, where timber is carried in.
pub fn timber(biome: Biome) -> Option<voxel::Voxel> {
    let fl = flora(biome);
    if fl.giant_density.max(fl.tree_density) < 0.2 {
        return None;
    }
    let list = if fl.giant_density > fl.tree_density { fl.giants } else { fl.trees };
    let (species, _) = list.iter().max_by(|a, b| a.1.total_cmp(&b.1))?;
    Some(match species {
        Species::Oak | Species::ElderOak | Species::Maple | Species::Cherry | Species::HolmOak => mat::BARK_OAK,
        Species::Birch => mat::BARK_BIRCH,
        Species::Spruce | Species::TallPine => mat::BARK_PINE,
        Species::ScotsPine => mat::BARK_SCOTS,
        Species::Redwood => mat::BARK_REDWOOD,
        Species::Jungle | Species::Kapok => mat::BARK_JUNGLE,
        Species::Willow => mat::BARK_WILLOW,
        Species::Acacia => mat::BARK_ACACIA,
        Species::Baobab => mat::BARK_BAOBAB,
        Species::Palm => mat::BARK_PALM,
        _ => return None,
    })
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
        Species::ScotsPine => mat::NEEDLES_PINE,
        Species::Juniper => mat::NEEDLES_JUNIPER,
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

/// How the evergreens are mixed into a biome's broadleaf trees: the share of its oaks,
/// birches, maples and cherries that each kind takes the place of on ordinary ground,
/// and how much further each goes where the ground suits it (see [`winter_green_tree`]).
struct Mix {
    /// Scots pine, on poor sandy ground: its share anywhere, and what a stretch of the
    /// poorest ground adds.
    pine: (f32, f32),
    /// Holly, in the shade of a close wood: its share in the open, and what shade adds.
    holly: (f32, f32),
    /// Yew, in shade: its share on ordinary ground, and what rich limy ground adds.
    yew: (f32, f32),
    /// Holm oak, toward the warm end of the biome.
    holm: f32,
    /// Juniper, on open poor ground.
    juniper: f32,
    /// Spruce, toward the cold end of the biome.
    spruce: f32,
    /// About how much of the wood they come to between them, over a broad stretch of it:
    /// what the far view keeps green of the forest in winter. Checked against the rest in
    /// the tests.
    share: f32,
}

fn mix(biome: Biome) -> Option<Mix> {
    Some(match biome {
        Biome::Deciduous => Mix { pine: (0.025, 0.32), holly: (0.015, 0.075), yew: (0.02, 0.09), holm: 0.05, juniper: 0.0, spruce: 0.08, share: 0.16 },
        Biome::BirchGrove => Mix { pine: (0.04, 0.25), holly: (0.01, 0.035), yew: (0.0, 0.0), holm: 0.0, juniper: 0.06, spruce: 0.0, share: 0.11 },
        // Warm woods have the evergreen oak where the temperate ones have holly and yew.
        Biome::Woodland => Mix { pine: (0.03, 0.12), holly: (0.0, 0.02), yew: (0.0, 0.0), holm: 0.15, juniper: 0.0, spruce: 0.0, share: 0.17 },
        Biome::Grassland => Mix { pine: (0.06, 0.2), holly: (0.05, 0.0), yew: (0.02, 0.0), holm: 0.08, juniper: 0.08, spruce: 0.0, share: 0.22 },
        _ => return None,
    })
}

/// How poor the soil is, about -1 (deep, rich, limy) to 1 (thin, sandy, acid), in
/// stretches a few hundred metres across. Pine, juniper and gorse take the poor ground;
/// yew the rich.
fn poor_soil(seed: u32, x: i32, z: i32) -> f32 {
    fbm2(seed ^ 0x50_11ed, x as f64 / m64(150.0), z as f64 / m64(150.0), 2, 2.0, 0.5)
}

fn ramp(a: f32, b: f32, x: f32) -> f32 {
    ((x - a) / (b - a)).clamp(0.0, 1.0)
}

/// The evergreen that stands in place of a broadleaf tree at a column, if one does:
/// always a minority, and each kind where the ground and the shade suit it. `shade` is
/// how close the wood stands there, 0 in a clearing to 1 in its depths.
fn winter_green_tree(col: &Column, shade: f32, x: i32, z: i32, seed: u32) -> Option<Species> {
    let mix = mix(col.biome)?;
    let poor = poor_soil(seed, x, z);
    let sandy = ramp(0.12, 0.42, poor);
    let rich = ramp(-0.12, -0.42, poor);
    let deep = ramp(0.55, 0.95, shade);
    // Where in the biome's own span of climates the column is: Woodland is the warm one.
    let warm = ramp(0.42, 0.56, col.temperature);
    let cool = ramp(0.36, 0.26, col.temperature);
    let shares = [
        (Species::ScotsPine, mix.pine.0 + mix.pine.1 * sandy),
        (Species::Holly, mix.holly.0 + mix.holly.1 * deep),
        (Species::Yew, (mix.yew.0 + mix.yew.1 * rich) * deep),
        (Species::HolmOak, mix.holm * if col.biome == Biome::Woodland { 1.0 } else { warm }),
        (Species::Juniper, mix.juniper * (1.0 - deep) * (0.4 + 0.6 * sandy)),
        (Species::Spruce, mix.spruce * cool),
    ];
    let mut roll = unit(hash2(seed ^ 0xe7e2_9a, x, z));
    for (species, share) in shares {
        if roll < share {
            return Some(species);
        }
        roll -= share;
    }
    None
}

/// Which evergreen shrub grows at a column where the biome's table calls for one: holly
/// in the shade, gorse on poor ground in the open, and laurel on good ground and in the
/// warm woods.
fn winter_green_shrub(col: &Column, shade: f32, x: i32, z: i32, seed: u32) -> Species {
    let poor = poor_soil(seed, x, z);
    let roll = unit(hash2(seed ^ 0xe7e2_9b, x, z));
    let deep = ramp(0.45, 0.85, shade);
    // Gorse wants light and does not mind poor ground; the wet old forest has none.
    let gorse = match col.biome {
        Biome::OldGrowth => 0.0,
        _ => (1.0 - deep) * (0.35 + 0.6 * ramp(0.0, 0.35, poor)),
    };
    let laurel = match col.biome {
        Biome::Woodland => 0.6,
        Biome::Grassland => 0.1,
        _ => 0.12 + 0.3 * ramp(-0.05, -0.4, poor),
    };
    if roll < gorse {
        Species::Gorse
    } else if roll < gorse + laurel * (1.0 - gorse) {
        Species::Laurel
    } else {
        Species::HollyBush
    }
}

/// About how much of a biome's forest keeps its leaves through the winter, 0 to 1: its
/// conifers and the evergreens mixed into its broadleaves, by their weights. The far view
/// keeps that much green in a bare winter forest, as the trees themselves do close to.
pub fn winter_green_share(biome: Biome) -> f32 {
    let fl = flora(biome);
    let mixed = mix(biome).map_or(0.0, |m| m.share);
    let (mut green, mut all) = (0.0, 0.0);
    for &(species, weight) in fl.trees {
        let keeps = match species {
            Species::DeadSnag | Species::Cactus => continue,
            Species::Oak | Species::Birch | Species::Maple | Species::Cherry => mixed,
            Species::Willow | Species::Acacia => 0.0,
            _ => 1.0,
        };
        green += weight * keeps;
        all += weight;
    }
    if all > 0.0 { green / all } else { 0.0 }
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

/// What one cell of a layer came to.
#[derive(Clone)]
struct Grown {
    /// Whether something was chosen to stand here, whatever the ground then made of it:
    /// a giant chosen keeps trees from under it even if it never grew.
    chosen: bool,
    feature: Option<Box<Placement>>,
}

/// Which cell of which layer, in which macro-cell (by its origin): cells run across the
/// borders between macro-cells, and each side reads its own continent.
type CellKey = ((i32, i32), u32, i32, i32);

/// Remembers what the cells of each layer came to. A chunk column asks about every cell
/// that could reach it, and the climbers on it about those a good way further out, so
/// neighbouring columns ask about the same cells many times over.
pub struct PlaceCache {
    shards: [Mutex<HashMap<CellKey, Grown>>; Self::SHARDS],
}

impl Default for PlaceCache {
    fn default() -> Self {
        Self { shards: std::array::from_fn(|_| Mutex::default()) }
    }
}

impl PlaceCache {
    const SHARDS: usize = 16;
    /// Cells a shard holds before it is emptied to start again: between them, enough
    /// for all the ground in view at full detail a few times over.
    const SHARD_CELLS: usize = 8192;

    fn shard(&self, key: &CellKey) -> &Mutex<HashMap<CellKey, Grown>> {
        &self.shards[hash2(key.1, key.2, key.3) as usize % Self::SHARDS]
    }

    fn get(&self, key: &CellKey) -> Option<Grown> {
        self.shard(key).lock().expect("placement cache poisoned").get(key).cloned()
    }

    fn insert(&self, key: CellKey, grown: Grown) {
        let mut shard = self.shard(&key).lock().expect("placement cache poisoned");
        if shard.len() >= Self::SHARD_CELLS {
            shard.clear();
        }
        shard.insert(key, grown);
    }
}

/// Features overlapping the square of columns at continent-local `(lx0, lz0)`, `span`
/// wide. `sampler` must cover that square plus the largest reach.
pub fn place(world_seed: u32, lib: &Library, sampler: &Sampler, lx0: i32, lz0: i32, span: i32, macro_origin: (i32, i32)) -> Vec<Placement> {
    place_in(None, world_seed, lib, sampler, lx0, lz0, span, macro_origin, &[GIANTS, TREES, SMALL], true, None)
}

/// The trees of [`place`] alone, the giants and the ordinary ones, each where it stands
/// in the world and of the same species, variant and turn, but without the bushes,
/// boulders and logs, and without working out the roots bared on a slope: several
/// times quicker, for drawing where the trees are (the map).
pub fn place_trees(world_seed: u32, lib: &Library, sampler: &Sampler, lx0: i32, lz0: i32, span: i32, macro_origin: (i32, i32)) -> Vec<Placement> {
    place_in(None, world_seed, lib, sampler, lx0, lz0, span, macro_origin, &[GIANTS, TREES], false, None)
}

/// The features of [`place`] whose species `keep` says yes to, in the same order, each
/// where it stands in the world and of the same species, variant, turn and lie, but
/// without the roots bared on a slope. A feature left out is dropped as soon as its
/// species is known, before the ground round it is read, so this is quicker than
/// filtering what [`place`] gives: for the coarse rings, which draw only what stands
/// tall.
#[allow(clippy::too_many_arguments)]
pub fn place_only(world_seed: u32, lib: &Library, sampler: &Sampler, lx0: i32, lz0: i32, span: i32, macro_origin: (i32, i32), keep: fn(Species) -> bool) -> Vec<Placement> {
    place_in(None, world_seed, lib, sampler, lx0, lz0, span, macro_origin, &[GIANTS, TREES, SMALL], false, Some(keep))
}

/// [`place`], remembering each cell in `cache` for the next to ask. A cache must only
/// ever be used with one seed and library.
#[allow(clippy::too_many_arguments)]
pub fn place_cached(
    cache: &PlaceCache,
    world_seed: u32,
    lib: &Library,
    sampler: &Sampler,
    lx0: i32,
    lz0: i32,
    span: i32,
    macro_origin: (i32, i32),
) -> Vec<Placement> {
    place_in(Some(cache), world_seed, lib, sampler, lx0, lz0, span, macro_origin, &[GIANTS, TREES, SMALL], true, None)
}

/// The features of `layers` (giants before trees, as a giant keeps trees from under it);
/// with `roots`, the roots bared beside each tree too; with `keep`, only the species it
/// says yes to. Only with all the layers and roots, and all species, may it remember
/// cells in `cache`.
#[allow(clippy::too_many_arguments)]
fn place_in(
    cache: Option<&PlaceCache>,
    world_seed: u32,
    lib: &Library,
    sampler: &Sampler,
    lx0: i32,
    lz0: i32,
    span: i32,
    macro_origin: (i32, i32),
    layers: &[Layer],
    roots: bool,
    keep: Option<fn(Species) -> bool>,
) -> Vec<Placement> {
    debug_assert!((roots && keep.is_none()) || cache.is_none());
    let mut out = Vec::new();
    // Where giants were chosen to stand, across the ground.
    let mut giants: Vec<(i32, i32)> = Vec::new();
    for &layer in layers {
        let (wx0, wz0) = (macro_origin.0 + lx0, macro_origin.1 + lz0);
        let (c0x, c1x) = ((wx0 - layer.reach).div_euclid(layer.cell), (wx0 + span + layer.reach).div_euclid(layer.cell));
        let (c0z, c1z) = ((wz0 - layer.reach).div_euclid(layer.cell), (wz0 + span + layer.reach).div_euclid(layer.cell));
        for cz in c0z..=c1z {
            for cx in c0x..=c1x {
                let cell = Cell::new(world_seed, &layer, cx, cz);
                // Nothing grows under a giant, whether it would have or not.
                if layer.salt == TREES.salt && giants.iter().any(|g| (g.0 - cell.x).abs() < 6 * BLOCK && (g.1 - cell.z).abs() < 6 * BLOCK) {
                    continue;
                }
                let key = (macro_origin, layer.salt, cx, cz);
                let grown = match cache.and_then(|c| c.get(&key)) {
                    Some(grown) => grown,
                    None => {
                        let grown = cell.grow(world_seed, lib, sampler, &layer, macro_origin, roots, keep);
                        // Kept only if the sampler covers all the ground the cell looked at,
                        // so that the next to ask gets what it would have worked out itself.
                        if let Some(cache) = cache
                            && sampler.covers(cell.x - macro_origin.0, cell.z - macro_origin.1, layer.reach + 1)
                        {
                            cache.insert(key, grown.clone());
                        }
                        grown
                    }
                };
                if layer.salt == GIANTS.salt && grown.chosen {
                    giants.push((cell.x, cell.z));
                }
                out.extend(grown.feature.map(|p| *p));
            }
        }
    }
    out
}

/// One cell of a layer: where in it a feature would stand, and its rolls of the dice.
struct Cell {
    hash: u32,
    /// World column.
    x: i32,
    z: i32,
}

impl Cell {
    fn new(world_seed: u32, layer: &Layer, cx: i32, cz: i32) -> Self {
        let mut cell = Self { hash: hash3(world_seed ^ layer.salt, cx, cz, 0), x: 0, z: 0 };
        // Position within the cell, keeping a margin so neighbours do not touch.
        cell.x = cx * layer.cell + BLOCK + (cell.roll(1) * (layer.cell - 2 * BLOCK) as f32) as i32;
        cell.z = cz * layer.cell + BLOCK + (cell.roll(2) * (layer.cell - 2 * BLOCK) as f32) as i32;
        cell
    }

    fn roll(&self, k: u32) -> f32 {
        unit(self.hash.rotate_left(k * 5).wrapping_mul(0x2c1b_3c6d ^ k))
    }

    /// What grows in the cell, if anything, reading the ground no further than the
    /// layer's reach from where it stands.
    #[allow(clippy::too_many_arguments)]
    fn grow(&self, world_seed: u32, lib: &Library, sampler: &Sampler, layer: &Layer, macro_origin: (i32, i32), with_roots: bool, keep: Option<fn(Species) -> bool>) -> Grown {
        let (x, z) = (self.x, self.z);
        let roll = |k: u32| self.roll(k);
        let nothing = |chosen: bool| Grown { chosen, feature: None };
        let (lx, lz) = (x - macro_origin.0, z - macro_origin.1);
        let col = sampler.column(lx, lz);
        let Some(mut species) = choose(&col, layer, roll(3), roll(4), x, z, world_seed) else {
            return nothing(false);
        };
        // About a landmark the trees have been felled back, the nearer the more, leaving
        // stumps; round a long-forgotten one, young growth is filling the clearing in.
        let mut cleared = false;
        if layer.salt != SMALL.salt
            && let Some((d, neglect)) = sampler.site_clearing(lx, lz)
        {
            let near = 1.0 - (d / crate::trails::CLEARING).clamp(0.0, 1.0);
            if roll(7) < near * (0.9 - 0.55 * neglect) {
                if layer.salt != TREES.salt || roll(8) > 0.6 {
                    return nothing(true);
                }
                (species, cleared) = (Species::Stump, true);
            } else if neglect > 0.45 && roll(9) < near * neglect {
                (species, cleared) = (Species::Bush, true);
            }
        }
        // Not wanted: chosen all the same, as from here on it always is, whether or not
        // it grows, so that a giant left out still keeps the trees from under it.
        if keep.is_some_and(|keep| !keep(species)) {
            return nothing(true);
        }
        let mut pos = IVec3::new(x, col.top() + 1, z);
        // Steep ground: nothing roots there but the rock itself.
        let rise = (sampler.column(lx + 1, lz).ground - sampler.column(lx - 1, lz).ground)
            .abs()
            .max((sampler.column(lx, lz + 1).ground - sampler.column(lx, lz - 1).ground).abs());
        if rise > 1.6 && !matches!(species, Species::Boulder | Species::MossyBoulder) {
            return nothing(true);
        }
        let variant = (roll(5) * 64.0) as usize;
        let turns = (roll(6) * 4.0) as u32;
        let mut lie = None;
        if layer.salt == SMALL.salt || cleared {
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
                    return nothing(true);
                }
                // Bedded in a little, so neither end shows daylight.
                pos.y -= 1;
            } else {
                let surface = |dx: i32, dz: i32| {
                    let c = sampler.column(lx + dx, lz + dz);
                    c.water_surface().is_none().then_some(c.top() + 1)
                };
                let Some(y) = settle(species, model.base, pos.y, surface) else {
                    return nothing(true);
                };
                pos.y = y;
            }
        }
        let roots = if layer.salt == SMALL.salt || cleared || !with_roots {
            None
        } else {
            let ground = |x: i32, z: i32| {
                let c = sampler.column(x - macro_origin.0, z - macro_origin.1);
                c.water_surface().is_none().then_some(c.top())
            };
            // Buttresses keep within the layer's reach: as far as a chunk looks for trees.
            roots::find(lib, species, variant, turns, pos, world_seed, layer.reach - 2, ground)
        };
        Grown {
            chosen: true,
            feature: Some(Box::new(Placement { pos, species, variant, turns, lie, roots })),
        }
    }
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
    // Some of a broadleaf wood is evergreen: a holly in the shade, a pine on a sandy rise.
    if layer.salt == TREES.salt && matches!(species, Species::Oak | Species::Birch | Species::Maple | Species::Cherry) {
        species = winter_green_tree(col, clearing, x, z, seed).unwrap_or(species);
    } else if species == Species::HollyBush {
        species = winter_green_shrub(col, clearing, x, z, seed);
    }
    // Bare tidal sand grows nothing: only boulders and driftwood lie on it.
    let bare = col.cover == Cover::Bank && !col.biome.arid() && crate::surface::flat_is_sand(col.ground, crate::surface::ground_patch(seed, x as f64, z as f64));
    if bare && !matches!(species, Species::Boulder | Species::FallenLog) {
        return None;
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
    if let Some(roots) = &p.roots {
        roots.voxels(p.pos, |q, v| set(q, v, true));
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

/// A tree's number for the seasons, 1..=63, from where its trunk stands: each tree turns
/// and drops its leaves on its own date from it (season.wgsl, `season_tree`). 0 is kept
/// for leaves of no known tree.
pub fn tree_tag(pos: IVec3) -> u8 {
    1 + (hash3(0x7ee7_a9, pos.x, 0, pos.z) % 63) as u8
}

/// The tag of the tree whose leaf is at world voxel `q`: the first of `placements` (in the
/// order they were stamped, as [`stamp`] lets the first fill a voxel) whose model holds a
/// leaf there; else, for a leaf cluster or a creeper grown beside it, the tree whose trunk
/// is nearest. 0 if there is none.
pub fn leaf_tag(lib: &Library, mats: &Materials, placements: &[Placement], q: IVec3) -> u8 {
    let mut nearest = (i32::MAX, 0);
    for p in placements {
        if let Some(v) = model_voxel(lib, p, q)
            && mats.get(v).kind == Kind::Foliage
        {
            return tree_tag(p.pos);
        }
        let d = (p.pos.x - q.x).pow(2) + (p.pos.z - q.z).pow(2);
        if d < nearest.0 {
            nearest = (d, tree_tag(p.pos));
        }
    }
    nearest.1
}

/// [`leaf_tag`] for the faces of one chunk's mesh. A leaf voxel is asked about once for
/// each of its faces, and each answer means looking through every tree that reaches the
/// column, so the answers are kept, voxel by voxel.
pub struct LeafTags<'a> {
    lib: &'a Library,
    mats: &'a Materials,
    placements: &'a [Placement],
    /// The voxel at the corner of the chunk's border.
    corner: IVec3,
    /// A tag for each voxel of the chunk, border included, or `UNKNOWN`. Empty until the
    /// first is asked for: most chunks hold no leaves.
    known: Vec<u8>,
}

impl<'a> LeafTags<'a> {
    const UNKNOWN: u8 = u8::MAX;
    const SIDE: i32 = voxel::PAD as i32;

    /// For the chunk whose own first voxel is world voxel `origin`.
    pub fn new(lib: &'a Library, mats: &'a Materials, placements: &'a [Placement], origin: IVec3) -> Self {
        Self { lib, mats, placements, corner: origin - IVec3::ONE, known: Vec::new() }
    }

    /// The tag of the tree whose leaf is at world voxel `q`.
    pub fn tag(&mut self, q: IVec3) -> u8 {
        let d = q - self.corner;
        if d.min_element() < 0 || d.max_element() >= Self::SIDE {
            return leaf_tag(self.lib, self.mats, self.placements, q);
        }
        if self.known.is_empty() {
            self.known = vec![Self::UNKNOWN; voxel::PAD * voxel::PAD * voxel::PAD];
        }
        let i = ((d.x * Self::SIDE + d.y) * Self::SIDE + d.z) as usize;
        if self.known[i] == Self::UNKNOWN {
            self.known[i] = leaf_tag(self.lib, self.mats, self.placements, q);
        }
        self.known[i]
    }
}

/// How high a feature's top can be above its origin, over all templates. Chunk
/// columns use it to know how far up to generate.
pub fn max_top(lib: &Library) -> i32 {
    lib.templates
        .iter()
        .flatten()
        .map(|t| t.top)
        .max()
        .unwrap_or(0)
}

/// How far from a chunk column the ground can matter to it: the widest feature reach,
/// or a small feature that close whose far end looks at the ground as far again.
pub const MAX_REACH: i32 = if GIANTS.reach > 2 * SMALL.reach { GIANTS.reach } else { 2 * SMALL.reach };

/// Whether `v` is something a plant bears that can be gathered from it, leaving the
/// plant: the berries of holly and ivy, the blossom of gorse, the head of a reed. A
/// column that has been picked ([`crate::world::GameWorld::pick`]) grows none of them.
pub fn borne(v: Voxel) -> bool {
    matches!(v, mat::HOLLY_BERRY | mat::GORSE_BLOOM | mat::IVY_BERRY | mat::IVY_BERRY_GREEN | mat::REED_HEAD)
}

/// Flowers and heather: drifts of colour on grass, one species per drift. The colour
/// stands for a flower of that colour, which `plants::flower_at` names by the biome.
pub fn flower(biome: Biome, x: i32, z: i32, seed: u32, roll: f32) -> Option<Voxel> {
    let (kinds, density) = flower_kinds(biome)?;
    // A fast noise marks the drifts; a slow one chooses what grows in each.
    let patch = fbm2(seed ^ 0xf10a, x as f64 / m64(27.2), z as f64 / m64(27.2), 2, 2.0, 0.5);
    if patch < 0.22 {
        return None;
    }
    let edge = ((patch - 0.22) / 0.2).clamp(0.3, 1.0);
    // Too high a roll for even a carpet: the slow noise need not be worked out.
    if roll >= density * edge * 2.2 {
        return None;
    }
    let kind = drift_kind(kinds, x, z, seed);
    (roll < drift_density(biome, kind, density) * edge).then_some(kind)
}

/// The flower of the drift at `(x, z)`, as [`flower`] chooses it, and the share of the
/// drift's columns that carry one: what a drift looks like from too far off to tell its
/// columns apart.
pub fn flower_drift(biome: Biome, x: i32, z: i32, seed: u32) -> Option<(Voxel, f32)> {
    let (kinds, density) = flower_kinds(biome)?;
    let patch = fbm2(seed ^ 0xf10a, x as f64 / m64(27.2), z as f64 / m64(27.2), 2, 2.0, 0.5);
    if patch < 0.22 {
        return None;
    }
    let edge = ((patch - 0.22) / 0.2).clamp(0.3, 1.0);
    let kind = drift_kind(kinds, x, z, seed);
    Some((kind, (drift_density(biome, kind, density) * edge).min(1.0)))
}

/// Which of a biome's flowers the drift at `(x, z)` grows.
fn drift_kind(kinds: &[Voxel], x: i32, z: i32, seed: u32) -> Voxel {
    let which = fbm2(seed ^ 0xf10b, x as f64 / m64(128.0), z as f64 / m64(128.0), 2, 2.0, 0.5) * 0.5 + 0.5;
    kinds[((which * kinds.len() as f32) as usize).min(kinds.len() - 1)]
}

/// How thickly a drift of `kind` grows, where the biome's flowers grow at `density`.
fn drift_density(biome: Biome, kind: Voxel, density: f32) -> f32 {
    // Bluebells carpet the floor of a broadleaf wood, thicker than other flowers grow.
    let carpet = kind == mat::FLOWER_BLUE && matches!(biome, Biome::Deciduous | Biome::Woodland | Biome::BirchGrove);
    if carpet { density * 2.2 } else { density }
}

/// The flowers a biome's drifts grow, and the share of a drift's columns that carry one.
fn flower_kinds(biome: Biome) -> Option<(&'static [Voxel], f32)> {
    Some(match biome {
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
        // Cotton grass, white over the wettest of the bog and here and there on the moor.
        Biome::Bog => (&[mat::FLOWER_WHITE], 0.12),
        Biome::Moor => (&[mat::HEATHER, mat::HEATHER, mat::HEATHER, mat::FLOWER_YELLOW, mat::FLOWER_WHITE], 0.42),
        // Desert marigold, thinly, where the sand holds a little water after rain.
        Biome::Desert | Biome::RedRock => (&[mat::FLOWER_YELLOW], 0.035),
        _ => return None,
    })
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
            Species::HollyBush,
            Species::Laurel,
            Species::Gorse,
        ];
        for species in small {
            for variant in 0..species.variants() {
                let reach = lib.get(species, variant).reach;
                assert!(reach <= SMALL.reach, "{species:?} reaches {reach}, chunks only look {}", SMALL.reach);
            }
        }
    }

    /// What `choose` grows in one layer over a broad stretch of level ground in a biome,
    /// species by species.
    fn census(biome: Biome, layer: &Layer, temperature: f32) -> HashMap<Species, u32> {
        let col = Column { ground: 40.0, water: None, cover: Cover::Land, biome, temperature, river: f32::MAX, clear: f32::MAX, worn: None };
        let mut counts = HashMap::new();
        for cx in 0..260 {
            for cz in 0..260 {
                let cell = Cell::new(5, layer, cx, cz);
                if let Some(species) = choose(&col, layer, cell.roll(3), cell.roll(4), cell.x, cell.z, 5) {
                    *counts.entry(species).or_insert(0) += 1;
                }
            }
        }
        counts
    }

    #[test]
    fn evergreens_are_a_minority_among_broadleaves_and_each_has_its_place() {
        for (biome, temperature) in [(Biome::Deciduous, 0.4), (Biome::BirchGrove, 0.4), (Biome::Woodland, 0.7), (Biome::Grassland, 0.4)] {
            let trees = census(biome, &TREES, temperature);
            let total: u32 = trees.values().sum();
            let green: u32 = trees.iter().filter(|(s, _)| s.winter_green() || **s == Species::Spruce).map(|(_, n)| n).sum();
            let share = green as f32 / total as f32;
            assert!(total > 500, "{biome:?} grew {total} trees");
            assert!(share > 0.06 && share < 0.3, "{biome:?}: {share} of its trees are evergreen");
            // The far view is told the same share, so a wood looks alike far and near.
            let told = winter_green_share(biome);
            assert!((told - share).abs() < 0.06, "{biome:?}: the far view keeps {told} green, the trees {share}");
        }
        let wood = census(Biome::Deciduous, &TREES, 0.4);
        for species in [Species::Holly, Species::Yew, Species::ScotsPine] {
            assert!(wood.get(&species).copied().unwrap_or(0) > 20, "a temperate wood grew too few of {species:?}");
        }
        // Holm oak keeps to the warm end of the temperate woods, spruce to the cold.
        assert!(!wood.contains_key(&Species::HolmOak) && !wood.contains_key(&Species::Spruce));
        assert!(census(Biome::Deciduous, &TREES, 0.56).contains_key(&Species::HolmOak));
        assert!(census(Biome::Deciduous, &TREES, 0.26).contains_key(&Species::Spruce));
        assert!(census(Biome::Woodland, &TREES, 0.7).get(&Species::HolmOak).copied().unwrap_or(0) > 100);
        // The evergreen shrubs: all three kinds under a wood, gorse above all on a heath.
        let shrubs = census(Biome::Deciduous, &SMALL, 0.4);
        for species in [Species::HollyBush, Species::Laurel, Species::Gorse] {
            assert!(shrubs.get(&species).copied().unwrap_or(0) > 20, "a temperate wood grew too few of {species:?}");
        }
        assert!(shrubs[&Species::Bush] > shrubs[&Species::HollyBush] + shrubs[&Species::Laurel] + shrubs[&Species::Gorse]);
        let heath = census(Biome::Heath, &SMALL, 0.4);
        assert!(heath[&Species::Gorse] > heath[&Species::Bush]);
        // Conifer forest counts as all green; a savanna of acacias as none.
        assert!(winter_green_share(Biome::Taiga) > 0.9);
        assert_eq!(winter_green_share(Biome::Savanna), 0.0);
    }

    #[test]
    fn evergreen_trees_fit_within_the_reach_chunks_look_for() {
        let lib = Library::build();
        for species in [Species::Holly, Species::Yew, Species::HolmOak, Species::ScotsPine, Species::Juniper] {
            for variant in 0..species.variants() {
                let reach = lib.get(species, variant).reach;
                assert!(reach <= TREES.reach, "{species:?} {variant} reaches {reach}, chunks only look {}", TREES.reach);
            }
        }
    }

    #[test]
    fn evergreen_leaves_read_as_evergreen_from_afar() {
        // season.wgsl's `season_evergreen`, which the far tree models are told apart by:
        // their colour alone must say they keep their leaves.
        let mats = mat::build();
        for v in [mat::LEAVES_HOLLY, mat::LEAVES_YEW, mat::LEAVES_HOLM, mat::NEEDLES_PINE, mat::NEEDLES_JUNIPER, mat::LEAVES_LAUREL, mat::GORSE] {
            let m = mats.get(v);
            assert_eq!(m.plant, voxel::plant::EVERGREEN);
            let q = m.color.map(|c| (c as f32 / 255.0).powf(1.1));
            assert!(q[2] > q[1] * 0.56 || q[0] < q[1] * 0.36, "{v} would turn and fall with the broadleaves in the far view: {:?}", m.color);
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
                        roots: None,
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
