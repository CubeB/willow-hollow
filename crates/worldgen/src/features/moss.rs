//! Moss and lichen: colonies of 0.1 m pieces on trunks, boulders, walls and dark ground.
//!
//! Each candidate face gets a score from shade, moisture, the way it faces and how old
//! its host is, and a slow noise groups the winners into connected colonies. Everything
//! is decided from the air voxel the moss occupies and the voxels next to it, through
//! [`attach`], so chunk borders agree. `MOSS` and `MOSSY_STONE` stay as the painted
//! voxels they always were: they carry the green into the distance, beyond where these
//! pieces are drawn.

use std::collections::HashSet;

use glam::IVec3;
use voxel::{Materials, Voxel, mesh::PlantBox};

use super::attach::{ColumnVoxels, FacePlot, Raster, Site, Toward, Voxels, for_each_site};
use super::shape::Rng;
use crate::{
    biome::Biome,
    materials as mat,
    noise::{hash3, perlin3},
    scale::m,
    surface::{Column, Cover},
};

/// Most boxes of moss in one chunk column: at 5 quads a box, about 80 KB of vertices.
pub const COLUMN_BOXES: usize = 500;
/// Faces considered per chunk column before the budget is applied, most deserving first.
const COLUMN_SITES: usize = 2400;

/// What a voxel is, as far as moss cares.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Host {
    Ground,
    Bark,
    Rock,
    Built,
}

/// Which hosts moss grows on, and `None` for everything else: snow, sand, flowers, roofs.
pub fn host_of(v: Voxel) -> Option<Host> {
    Some(match v {
        mat::GRASS | mat::GRASS_LUSH | mat::GRASS_JUNGLE | mat::GRASS_TAIGA | mat::GRASS_WOODLAND | mat::DIRT | mat::MUD | mat::MOSS => Host::Ground,
        mat::BARK_OAK | mat::BARK_PINE | mat::BARK_REDWOOD | mat::BARK_JUNGLE | mat::BARK_WILLOW | mat::BARK_BIRCH | mat::WOOD_DEAD => Host::Bark,
        mat::STONE | mat::ROCK_DARK => Host::Rock,
        mat::MOSSY_STONE | mat::SANDSTONE | mat::SANDSTONE_DARK | mat::PLANK | mat::PLANK_PALE | mat::PLANK_DARK | mat::PLANK_RED => Host::Built,
        _ => return None,
    })
}

/// The kinds of moss, each with its own form and colours.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Moss {
    /// Soft mounds.
    Cushion,
    /// A flat mat that spreads.
    Carpet,
    /// Shaggy, with strands that hang.
    Shaggy,
    /// A thin yellow or grey crust on stone and bark.
    Lichen,
}

impl Moss {
    pub const ALL: [Moss; 4] = [Moss::Cushion, Moss::Carpet, Moss::Shaggy, Moss::Lichen];

    /// How common it is on a host, relative to the others.
    fn weight(self, host: Host) -> f32 {
        match (self, host) {
            (Moss::Carpet, Host::Ground) => 5.0,
            (Moss::Cushion, Host::Ground) => 3.0,
            (Moss::Shaggy, Host::Ground) => 1.0,
            (Moss::Lichen, Host::Ground) => 0.6,
            (Moss::Shaggy, Host::Bark) => 3.5,
            (Moss::Carpet, Host::Bark) => 3.5,
            (Moss::Cushion, Host::Bark) => 2.0,
            (Moss::Lichen, Host::Bark) => 1.0,
            (Moss::Lichen, Host::Rock) => 4.5,
            (Moss::Cushion, Host::Rock) => 2.5,
            (Moss::Carpet, Host::Rock) => 2.5,
            (Moss::Shaggy, Host::Rock) => 0.5,
            (Moss::Carpet, Host::Built) => 3.5,
            (Moss::Lichen, Host::Built) => 3.0,
            (Moss::Cushion, Host::Built) => 2.5,
            (Moss::Shaggy, Host::Built) => 1.0,
        }
    }

    /// Picks a moss for a host from a value in 0..1 that is steady across a colony.
    fn pick(host: Host, roll: f32) -> Moss {
        let total: f32 = Moss::ALL.iter().map(|m| m.weight(host)).sum();
        let mut r = roll.clamp(0.0, 0.999) * total;
        for moss in Moss::ALL {
            if r < moss.weight(host) {
                return moss;
            }
            r -= moss.weight(host);
        }
        Moss::Carpet
    }

    /// The main colour and a lighter one for tips and edges, chosen per colony.
    fn colours(self, plant: f32) -> (Voxel, Voxel) {
        match self {
            Moss::Cushion if plant < 0.5 => (mat::MOSS_CUSHION, mat::MOSS_CUSHION_LIT),
            Moss::Cushion => (mat::MOSS_CUSHION_DEEP, mat::MOSS_CUSHION),
            Moss::Carpet if plant < 0.5 => (mat::MOSS_CARPET, mat::MOSS_CARPET_LIT),
            Moss::Carpet => (mat::MOSS_CARPET_DEEP, mat::MOSS_CARPET),
            Moss::Shaggy if plant < 0.6 => (mat::MOSS_SHAG, mat::MOSS_SHAG_TIP),
            Moss::Shaggy => (mat::MOSS_CARPET_DEEP, mat::MOSS_SHAG),
            Moss::Lichen if plant < 0.55 => (mat::LICHEN_YELLOW, mat::LICHEN_GREY),
            Moss::Lichen => (mat::LICHEN_GREY, mat::LICHEN_YELLOW),
        }
    }
}

/// How damp a place is, from 0 (nothing grows here) to 1.
fn moisture(col: &Column) -> f32 {
    // The wettest and driest newer biomes are set here; the rest grow moss as their parent does.
    let base = match col.biome {
        Biome::Marsh | Biome::Bog | Biome::Mangrove => 0.9,
        Biome::RedRock | Biome::DuneSea | Biome::Volcanic => 0.0,
        b => match b.parent() {
            Biome::Rainforest | Biome::OldGrowth => 1.0,
            Biome::Taiga => 0.8,
            Biome::Deciduous => 0.72,
            Biome::Woodland => 0.55,
            Biome::Heath => 0.4,
            Biome::Grassland | Biome::Tundra => 0.28,
            Biome::Alpine | Biome::Ocean => 0.18,
            Biome::Savanna => 0.08,
            _ => 0.0,
        },
    };
    if base == 0.0 {
        return 0.0;
    }
    let river = ((m(9.0) - col.river) / m(9.0)).clamp(0.0, 1.0);
    (base + river * 0.25).min(1.0)
}

/// The score above which a face grows moss, from its circumstances alone: shade (0..1),
/// moisture (0..1), a wall's aspect and the age of what it grows on.
fn appeal(host: Host, toward: Toward, shade: f32, wet: f32) -> f32 {
    let (age, facing) = match host {
        Host::Bark => (0.6, 0.0),
        Host::Rock => (0.75, 0.0),
        Host::Built => (0.9, 0.0),
        Host::Ground => (0.45, 0.0),
    };
    // The sun tracks across the +Z side of the sky, so -Z walls stay in the shade.
    let aspect = facing
        + match toward {
            Toward::NegZ => 0.26,
            Toward::PosX | Toward::NegX => 0.10,
            Toward::PosZ => -0.1,
            Toward::Below => 0.05,
            Toward::Above => 0.2,
        };
    let (shade, wet) = (shade.clamp(0.0, 1.0), wet.clamp(0.0, 1.0));
    match host {
        // Bare ground needs real shade and damp, or a meadow would turn green.
        Host::Ground => shade * 0.75 + wet * 0.6 + aspect - 1.0,
        _ => shade * 0.45 + wet * 0.55 + aspect + age * 0.2 - 0.82,
    }
}

/// One face that will grow moss.
struct Patch {
    site: Site,
    host: Host,
    /// How strongly it grows, 0..1.
    strength: f32,
    /// What it is competing with for the column's budget.
    score: f32,
}

/// Columns' climate and the things to keep clear of, for one chunk column.
pub struct Soil<'a> {
    pub columns: &'a [Column],
    /// World position of the padded corner of `columns`.
    pub x0: i32,
    pub z0: i32,
    pub seed: u32,
    /// Air voxels already holding plants that stand on the ground, or vines.
    pub taken: &'a HashSet<IVec3>,
}

impl Soil<'_> {
    fn column(&self, x: i32, z: i32) -> &Column {
        let (px, pz) = ((x - self.x0) as usize, (z - self.z0) as usize);
        &self.columns[px * voxel::PAD + pz]
    }
}

/// Grows the moss for a chunk column.
pub fn grow(voxels: &ColumnVoxels, mats: &Materials, soil: &Soil, out: &mut Vec<PlantBox>) {
    if !soil.columns.iter().any(|c| moisture(c) > 0.0) {
        return;
    }
    let mut patches: Vec<Patch> = Vec::new();
    for_each_site(
        voxels,
        mats,
        |v| host_of(v).is_some(),
        |site| {
            if let Some(p) = judge(site, soil) {
                patches.push(p);
            }
        },
    );
    if patches.len() > COLUMN_SITES {
        patches.sort_unstable_by(|a, b| b.score.total_cmp(&a.score));
        patches.truncate(COLUMN_SITES);
    }
    let mut raster = Raster::new();
    for p in &patches {
        plant(p, soil.seed, voxels, &mut raster);
    }
    let start = out.len();
    raster.finish(out, |_| (false, false));
    // If it still overspends, thin it evenly so no part of the column is left bare.
    if out.len() - start > COLUMN_BOXES {
        let mut mine: Vec<PlantBox> = out.drain(start..).collect();
        mine.sort_unstable_by_key(|b| hash3(soil.seed ^ 0xb0c5, b.lo[0], b.lo[1], b.lo[2]));
        mine.truncate(COLUMN_BOXES);
        out.append(&mut mine);
    }
}

/// Decides whether a site grows moss.
fn judge(site: &Site, soil: &Soil) -> Option<Patch> {
    if soil.taken.contains(&site.air) {
        return None;
    }
    let host = host_of(site.host)?;
    let col = soil.column(site.air.x, site.air.z);
    if !matches!(col.cover, Cover::Land | Cover::Bank) || col.water_surface().is_some() {
        return None;
    }
    // Under an overhang counts as shaded even with no canopy.
    let shade = (site.canopy as f32 / 5.0).min(1.0) + if site.toward == Toward::Above { 0.6 } else { 0.0 };
    let wet = moisture(col);
    if wet == 0.0 {
        return None;
    }
    let (x, y, z) = (site.air.x as f64, site.air.y as f64, site.air.z as f64);
    // Bigger colonies, with a noisy edge.
    let colony = perlin3(soil.seed ^ 0x3055, x / m(5.6) as f64, y / m(5.6) as f64, z / m(5.6) as f64)
        + 0.45 * perlin3(soil.seed ^ 0x3056, x / m(1.4) as f64, y / m(1.4) as f64, z / m(1.4) as f64);
    let score = appeal(host, site.toward, shade, wet) + colony * 0.55;
    (score > 0.0).then_some(Patch { site: *site, host, strength: (score * 2.5).clamp(0.0, 1.0), score })
}

/// Plants one face. A colony's moss kind and shade come from a slow noise, so patches read
/// as one plant.
fn plant(p: &Patch, seed: u32, voxels: &ColumnVoxels, raster: &mut Raster) {
    let site = &p.site;
    let (x, y, z) = (site.air.x, site.air.y, site.air.z);
    let at = |f: f64| (x as f64 * f, y as f64 * f, z as f64 * f);
    let (cx, cy, cz) = at(1.0 / m(3.2) as f64);
    let kind = perlin3(seed ^ 0x8a51, cx, cy, cz) * 0.5 + 0.5;
    let (sx, sy, sz) = at(1.0 / m(2.0) as f64);
    let shade = perlin3(seed ^ 0x8a52, sx, sy, sz) * 0.5 + 0.5;
    let roll = (kind * 1.6 - 0.3).clamp(0.0, 0.999);
    let mut moss = Moss::pick(p.host, roll);
    let h = hash3(seed ^ 0xe7ba, x, y * 7 + site.toward as i32, z);
    let mut rng = Rng::new(h);
    // Only hanging moss clings to a ceiling.
    if site.toward == Toward::Above {
        moss = Moss::Shaggy;
    }
    let (main, tip) = moss.colours(shade);
    let mut plot = site.plot(raster);
    match moss {
        Moss::Carpet => carpet(&mut plot, &mut rng, p.strength, main, tip),
        Moss::Cushion => cushion(&mut plot, &mut rng, p.strength, main, tip),
        Moss::Shaggy => shaggy(site, voxels, &mut plot, &mut rng, p.strength, main, tip),
        Moss::Lichen => lichen(&mut plot, &mut rng, p.strength, main, tip),
    }
}

/// A flat mat, ragged where the colony thins.
fn carpet(plot: &mut FacePlot, rng: &mut Rng, strength: f32, main: Voxel, tip: Voxel) {
    let solid = strength > 0.85;
    let mut cells = [[false; 4]; 4];
    for (v, row) in cells.iter_mut().enumerate() {
        for (u, cell) in row.iter_mut().enumerate() {
            let edge = u == 0 || v == 0 || u == 3 || v == 3;
            let chance = if solid { 1.0 } else if edge { strength * 0.75 } else { (strength + 0.25).min(1.0) };
            *cell = rng.unit() < chance;
        }
    }
    for (v, row) in cells.iter().enumerate() {
        for (u, cell) in row.iter().enumerate() {
            if *cell {
                plot.fill(u as i32, u as i32 + 1, v as i32, v as i32 + 1, 0, 1, main);
            }
        }
    }
    // A few raised spots of a lighter green break up the flat plane.
    for _ in 0..rng.int(0, 2) {
        let (u, v) = (rng.int(0, 3), rng.int(0, 3));
        if cells[v as usize][u as usize] {
            plot.fill(u, u + 1, v, v + 1, 1, 2, tip);
        }
    }
}

/// Low, uneven mounds: a few overlapping humps of different sizes, each column of cells
/// its own height, so neighbouring faces never line up into one even block.
fn cushion(plot: &mut FacePlot, rng: &mut Rng, strength: f32, main: Voxel, tip: Voxel) {
    let mut height = [[0i32; 4]; 4];
    let mounds = 1 + (strength > 0.45) as i32 + (rng.unit() < 0.35) as i32;
    for _ in 0..mounds {
        let (cu, cv) = (rng.range(-0.5, 3.5), rng.range(-0.5, 3.5));
        let r = rng.range(1.1, 2.4);
        let peak = rng.range(0.8, 1.4 + 1.2 * strength);
        for (v, row) in height.iter_mut().enumerate() {
            for (u, h) in row.iter_mut().enumerate() {
                let d2 = (u as f32 - cu).powi(2) + (v as f32 - cv).powi(2);
                let here = peak * (1.0 - d2 / (r * r)) + rng.range(-0.35, 0.35);
                *h = (*h).max(here.round() as i32);
            }
        }
    }
    let top = height.iter().flatten().copied().max().unwrap_or(0);
    for (v, row) in height.iter().enumerate() {
        for (u, &h) in row.iter().enumerate() {
            let h = h.min(3);
            if h <= 0 {
                continue;
            }
            let (u, v) = (u as i32, v as i32);
            // The crowns of the taller humps catch the light.
            if h >= 2 && h == top.min(3) && rng.unit() < 0.6 {
                plot.fill(u, u + 1, v, v + 1, 0, h - 1, main);
                plot.fill(u, u + 1, v, v + 1, h - 1, h, tip);
            } else {
                plot.fill(u, u + 1, v, v + 1, 0, h, main);
            }
        }
    }
}

/// Shaggy moss: a short mat with strands that fall down the face, or stand as tufts on a
/// floor.
fn shaggy(site: &Site, voxels: &ColumnVoxels, plot: &mut FacePlot, rng: &mut Rng, strength: f32, main: Voxel, tip: Voxel) {
    plot.fill(0, 4, 2, 4, 0, 1, main);
    if site.toward == Toward::Below {
        // A low mat of feathery tufts.
        for _ in 0..rng.int(2, 4) {
            let (u, v, h) = (rng.int(0, 3), rng.int(0, 3), rng.int(2, 3));
            plot.fill(u, u + 1, v, v + 1, 0, h, if rng.unit() < 0.4 { tip } else { main });
        }
        return;
    }
    let strands = if strength > 0.5 { 3 } else { 2 };
    for _ in 0..strands {
        let u = rng.int(0, 3);
        let len = rng.int(2, 4);
        let colour = if rng.unit() < 0.5 { main } else { tip };
        if site.toward == Toward::Above {
            // Strands hanging from a branch or overhang, stopping if there is no air below.
            let below = voxels.voxel(site.air - IVec3::Y);
            let reach = if below == voxel::AIR { len + 3 } else { len };
            plot.fill(u, u + 1, 1, 2, 0, reach, colour);
        } else {
            let v = rng.int(0, 3);
            plot.fill(u, u + 1, v - len, v + 1, 0, 2, colour);
        }
    }
}

/// A thin crust in scattered flat blotches.
fn lichen(plot: &mut FacePlot, rng: &mut Rng, strength: f32, main: Voxel, tip: Voxel) {
    let blotches = if strength > 0.5 { 3 } else { 2 };
    for i in 0..blotches {
        let (u, v) = (rng.int(0, 3), rng.int(0, 3));
        let (w, h) = (rng.int(1, 3), rng.int(1, 3));
        plot.fill(u, u + w, v, v + h, 0, 1, if i == 0 { main } else { tip });
    }
}

/// The share of columns of open, dry ground that end up with moss.
#[cfg(test)]
fn sunny_ground_coverage(biome: Biome, canopy: u32, cover: Cover, host: Voxel) -> f32 {
    let col = Column { ground: 40.0, water: None, cover, biome, temperature: 0.5, river: f32::MAX, clear: f32::MAX };
    let cols = vec![col; voxel::PAD * voxel::PAD];
    let mut grown = 0;
    let total = 2000;
    for i in 0..total {
        let site = Site { air: IVec3::new(i * 3, 41, i * 5 + 1), toward: Toward::Below, host, canopy };
        let soil = Soil { columns: &cols, x0: site.air.x, z0: site.air.z, seed: 21, taken: &HashSet::new() };
        grown += judge(&site, &soil).is_some() as u32;
    }
    grown as f32 / total as f32
}

#[cfg(test)]
mod tests {
    use voxel::{CHUNK, PaddedChunk, mesh::CELLS};

    use super::*;
    use crate::{World, continent::SimConfig, materials::build, world::GameWorld};

    fn column(biome: Biome) -> Column {
        Column { ground: 40.0, water: None, cover: Cover::Land, biome, temperature: 0.5, river: f32::MAX, clear: f32::MAX }
    }

    /// A chunk column holding the five kinds of place moss should grow on: a trunk under
    /// leaves, a boulder, a built wall and the ground around them, at world coords
    /// starting at (origin, 0, origin).
    fn scene(origin: i32) -> (Vec<(i32, PaddedChunk)>, i32) {
        let p = |x: i32, y: i32, z: i32| IVec3::new(x, y, z);
        let mut map = std::collections::HashMap::new();
        // Ground, with a canopy of leaves over half of it.
        for x in origin - 1..origin + CHUNK + 1 {
            for z in origin - 1..origin + CHUNK + 1 {
                for y in 0..=7 {
                    map.insert(p(x, y, z), if y == 7 { mat::GRASS_LUSH } else { mat::DIRT });
                }
                if (z - origin) < 22 {
                    map.insert(p(x, 20, z), mat::LEAVES_OAK);
                    map.insert(p(x, 21, z), mat::LEAVES_OAK);
                    map.insert(p(x, 22, z), mat::LEAVES_OAK);
                    map.insert(p(x, 23, z), mat::LEAVES_OAK);
                    map.insert(p(x, 24, z), mat::LEAVES_OAK);
                }
            }
        }
        // A trunk.
        for y in 8..20 {
            for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                map.insert(p(origin + 5 + dx, y, origin + 5 + dz), mat::BARK_OAK);
            }
        }
        // A boulder.
        for x in 14..18 {
            for z in 5..9 {
                for y in 8..12 {
                    map.insert(p(origin + x, y, origin + z), mat::ROCK_DARK);
                }
            }
        }
        // A ruined wall of mossy stone blocks.
        for x in 22..28 {
            for z in 7..10 {
                for y in 8..14 {
                    map.insert(p(origin + x, y, origin + z), mat::MOSSY_STONE);
                }
            }
        }
        // A plank wall out in the open.
        for x in 8..12 {
            for y in 8..12 {
                map.insert(p(origin + x, y, origin + 28), mat::PLANK);
            }
        }
        let mut chunk = PaddedChunk::new();
        for (q, v) in map {
            let (px, py, pz) = (q.x - origin + 1, q.y + 1, q.z - origin + 1);
            if (0..voxel::PAD as i32).contains(&px) && (0..voxel::PAD as i32).contains(&py) && (0..voxel::PAD as i32).contains(&pz) {
                chunk.set(px as usize, py as usize, pz as usize, v);
            }
        }
        (vec![(0, chunk)], origin)
    }

    fn grown(biome: Biome, origin: i32, seed: u32) -> Vec<PlantBox> {
        let (chunks, origin) = scene(origin);
        let voxels = ColumnVoxels::new(&chunks, origin - 1, origin - 1);
        let cols = vec![column(biome); voxel::PAD * voxel::PAD];
        let soil = Soil { columns: &cols, x0: origin - 1, z0: origin - 1, seed, taken: &HashSet::new() };
        let mut out = Vec::new();
        grow(&voxels, &build(), &soil, &mut out);
        out
    }

    fn colour_host(voxel: Voxel) -> Option<Moss> {
        Some(match voxel {
            mat::MOSS_CUSHION | mat::MOSS_CUSHION_DEEP | mat::MOSS_CUSHION_LIT => Moss::Cushion,
            mat::MOSS_CARPET | mat::MOSS_CARPET_DEEP | mat::MOSS_CARPET_LIT => Moss::Carpet,
            mat::MOSS_SHAG | mat::MOSS_SHAG_TIP => Moss::Shaggy,
            mat::LICHEN_YELLOW | mat::LICHEN_GREY => Moss::Lichen,
            _ => return None,
        })
    }

    #[test]
    fn the_same_inputs_grow_the_same_moss() {
        let a = grown(Biome::OldGrowth, 64, 9);
        let b = grown(Biome::OldGrowth, 64, 9);
        assert!(!a.is_empty());
        assert_eq!(a.len(), b.len());
        assert!(a.iter().zip(&b).all(|(p, q)| p.lo == q.lo && p.hi == q.hi && p.voxel == q.voxel));
    }

    #[test]
    fn neighbouring_columns_agree_where_they_touch() {
        // Find two adjacent chunk columns with moss somewhere in a real world. Each must
        // only place moss in air it owns, against faces that the neighbour's own voxels
        // confirm are really there, and generating the same column twice must not vary.
        let w = GameWorld::with_config(11, SimConfig { size: 256 });
        let mats = build();
        let mut compared = 0;
        let (centre, stride) = (crate::macro_layout::MACRO_SIZE / CHUNK / 2, 3);
        'columns: for cx in (centre - 300..centre + 300).step_by(stride) {
            for cz in (centre - 300..centre + 300).step_by(stride) {
                let here = w.generate_column(cx, cz);
                let mossy = |p: &PlantBox| colour_host(p.voxel).is_some();
                if !here.plants.iter().any(mossy) {
                    continue;
                }
                let again = w.generate_column(cx, cz);
                let moss = |o: &crate::ColumnOutput| o.plants.iter().filter(|p| mossy(p)).map(|p| (p.lo, p.hi, p.voxel)).collect::<Vec<_>>();
                assert_eq!(moss(&here), moss(&again), "column {cx},{cz} changed between runs");
                let east = w.generate_column(cx + 1, cz);
                let south = w.generate_column(cx, cz + 1);
                let world_voxel = |q: IVec3| -> voxel::Voxel {
                    let lookup = |o: &crate::ColumnOutput, ox: i32, oz: i32| ColumnVoxels::new(&o.chunks, ox * CHUNK - 1, oz * CHUNK - 1).voxel(q);
                    if q.x >= (cx + 1) * CHUNK {
                        lookup(&east, cx + 1, cz)
                    } else if q.z >= (cz + 1) * CHUNK {
                        lookup(&south, cx, cz + 1)
                    } else {
                        lookup(&here, cx, cz)
                    }
                };
                for b in here.plants.iter().filter(|p| mossy(p)) {
                    assert_eq!(b.lo[0].div_euclid(CHUNK * CELLS), cx, "{b:?} is outside its column");
                    assert_eq!(b.lo[2].div_euclid(CHUNK * CELLS), cz, "{b:?} is outside its column");
                    assert_eq!((b.hi[0] - 1).div_euclid(CHUNK * CELLS), cx);
                    assert_eq!((b.hi[2] - 1).div_euclid(CHUNK * CELLS), cz);
                    // Every box sits in air and touches something solid that the voxels of
                    // the owning columns really contain, whichever side it is on.
                    let cell = IVec3::from_array(b.lo).div_euclid(IVec3::splat(CELLS));
                    assert_eq!(world_voxel(cell), voxel::AIR, "{b:?} is not in air");
                    let touching = Toward::ALL.iter().any(|t| {
                        let v = world_voxel(cell + t.step());
                        mats.get(v).kind == voxel::Kind::Solid
                    });
                    assert!(touching, "{b:?} floats");
                }
                compared += 1;
                if compared >= 6 {
                    break 'columns;
                }
            }
        }
        assert!(compared > 0, "found no mossy columns to compare");
    }

    #[test]
    fn budgets_hold_and_no_moss_is_a_whole_voxel() {
        for biome in [Biome::Rainforest, Biome::OldGrowth, Biome::Taiga, Biome::Deciduous] {
            let out = grown(biome, 64, 7);
            assert!(out.len() <= COLUMN_BOXES, "{biome:?}: {}", out.len());
            assert!(!out.is_empty(), "{biome:?} grew no moss");
            for b in &out {
                let dims = [b.hi[0] - b.lo[0], b.hi[1] - b.lo[1], b.hi[2] - b.lo[2]];
                // Plates are one cell thick in at least one direction, however far they merge.
                assert!(dims.iter().min().unwrap() < &CELLS, "{b:?} is a full voxel");
                assert!(!b.sway_lo && !b.sway_hi, "moss does not sway");
            }
        }
    }

    #[test]
    fn several_mosses_grow_on_each_kind_of_host() {
        let mut seen: std::collections::HashMap<Host, Vec<Moss>> = std::collections::HashMap::new();
        let col = column(Biome::OldGrowth);
        let cols = vec![col; voxel::PAD * voxel::PAD];
        for (host, voxel) in [(Host::Ground, mat::GRASS_LUSH), (Host::Bark, mat::BARK_OAK), (Host::Rock, mat::STONE), (Host::Built, mat::MOSSY_STONE)] {
            let found = seen.entry(host).or_default();
            for i in 0..6000 {
                let toward = if host == Host::Ground { Toward::Below } else { Toward::ALL[(i % 4 + 2) as usize] };
                let site = Site { air: IVec3::new(i, 41 + i % 13, i * 7 % 500), toward, host: voxel, canopy: 6 };
                let soil = Soil { columns: &cols, x0: site.air.x, z0: site.air.z, seed: 21, taken: &HashSet::new() };
                if let Some(p) = judge(&site, &soil) {
                    let (x, y, z) = (site.air.x, site.air.y, site.air.z);
                    let f = 1.0 / m(3.2) as f64;
                    let kind = perlin3(21 ^ 0x8a51, x as f64 * f, y as f64 * f, z as f64 * f) * 0.5 + 0.5;
                    let moss = Moss::pick(p.host, (kind * 1.6 - 0.3).clamp(0.0, 0.999));
                    if !found.contains(&moss) {
                        found.push(moss);
                    }
                }
            }
            assert!(found.len() >= 3, "{host:?} grew only {found:?}");
        }
    }

    #[test]
    fn moss_appears_on_trunk_rock_wall_and_shaded_ground() {
        let out = grown(Biome::OldGrowth, 64, 5);
        let hit = |lo: IVec3, hi: IVec3| {
            out.iter().filter(move |b| {
                let c = IVec3::new(b.lo[0], b.lo[1], b.lo[2]).div_euclid(IVec3::splat(CELLS));
                c.cmpge(lo).all() && c.cmple(hi).all()
            })
        };
        let trunk = hit(IVec3::new(68, 8, 68), IVec3::new(72, 19, 72)).count();
        let boulder = hit(IVec3::new(77, 8, 68), IVec3::new(83, 13, 73)).count();
        let wall = hit(IVec3::new(85, 8, 70), IVec3::new(93, 15, 75)).count();
        let shaded_ground = hit(IVec3::new(64, 8, 76), IVec3::new(96, 9, 85)).count();
        assert!(trunk > 0, "no moss on the trunk");
        assert!(boulder > 0, "no moss on the boulder");
        assert!(wall > 0, "no moss on the wall");
        assert!(shaded_ground > 0, "no moss on the shaded ground");
        assert!(out.iter().any(|b| colour_host(b.voxel) == Some(Moss::Lichen) || colour_host(b.voxel) == Some(Moss::Carpet)));
    }

    #[test]
    fn open_ground_and_dry_places_stay_almost_bare() {
        // Sunny open grass in the wettest climate has none; shade is what lets it in.
        assert!(sunny_ground_coverage(Biome::OldGrowth, 0, Cover::Land, mat::GRASS_LUSH) < 0.02, "open forest floor");
        assert!(sunny_ground_coverage(Biome::Grassland, 0, Cover::Land, mat::GRASS_MEADOW) < 0.01, "meadow");
        assert!(sunny_ground_coverage(Biome::OldGrowth, 6, Cover::Land, mat::GRASS_LUSH) > 0.15, "shaded forest floor should be mossy");
        // No moisture at all in the desert, even in deep shade.
        assert_eq!(sunny_ground_coverage(Biome::Desert, 6, Cover::Land, mat::DIRT), 0.0);
        let out = grown(Biome::Desert, 64, 5);
        assert!(out.is_empty(), "desert grew moss: {}", out.len());
    }

    #[test]
    fn colonies_are_patches_and_not_speckle() {
        // Most faces next to a mossy face are mossy too.
        let col = column(Biome::OldGrowth);
        let cols = vec![col; voxel::PAD * voxel::PAD];
        let judge_at = |x: i32, z: i32| {
            let soil = Soil { columns: &cols, x0: x, z0: z, seed: 3, taken: &HashSet::new() };
            let site = Site { air: IVec3::new(x, 41, z), toward: Toward::NegZ, host: mat::STONE, canopy: 4 };
            judge(&site, &soil).is_some()
        };
        let (mut same, mut total, mut any) = (0, 0, 0);
        for x in 0..600 {
            for z in 0..600 {
                if (x * 31 + z * 7) % 13 == 0 {
                    any += judge_at(x, z) as u32;
                    total += 1;
                    same += (judge_at(x, z) == judge_at(x + 1, z)) as u32;
                }
            }
        }
        assert!(any > 100 && any < total, "any {any} of {total}");
        assert!(same as f32 > total as f32 * 0.8, "{same} of {total} neighbours agree");
    }
}
