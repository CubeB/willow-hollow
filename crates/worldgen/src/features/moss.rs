//! Moss and lichen: colonies of 0.1 m pieces on trunks, boulders, walls and dark ground.
//!
//! Moss lies flat: a skin half a cell (0.05 m) thick against its face, with here and
//! there a cell raised to a full 0.1 m, never a mound or a hanging strand. On a ceiling
//! it is only skin, and only where the underside is well shaded.
//!
//! Each candidate face gets a score from shade, moisture, the way it faces, how old its
//! host is and how near the ground it is, and a slow noise groups the winners into
//! connected colonies. Moss is thickest round the foot of a trunk and on top of logs,
//! boulders and walls. Well inside a colony a face is covered edge to edge, so a colony
//! is an unbroken mat with a ragged rim; a column with more moss than its budget gives
//! up its lowest-scoring faces whole, and keeps the rest as they were. Everything
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

/// Most boxes of moss in one chunk column: at 5 quads a box, about 130 KB of vertices.
pub const COLUMN_BOXES: usize = 800;
/// Faces considered per chunk column before the budget is applied, most deserving first.
const COLUMN_SITES: usize = 2400;
/// How far an underside's score is held below a wall's.
const UNDERSIDE_PENALTY: f32 = 0.45;
/// Opaque voxels overhead, the ceiling's own included, before an underside is dark enough
/// for moss: a thick overhang, a cliff, or a branch under leaves, not a signboard.
const UNDERSIDE_CANOPY: u32 = 4;
/// Where a second slow noise falls below this, moss leaves bare gaps, so colonies are
/// sparser without breaking into speckle. Cuts out about a quarter to a third.
const GAP_CUT: f32 = -0.1;
/// Size of the bare gaps.
const GAP_SCALE: f32 = 3.5;

/// How much better the top of a log, boulder or wall scores than its sides.
const TOP_BONUS: f32 = 0.3;
/// How much better the foot of a trunk scores, and how much worse it scores high up.
const FOOT_BONUS: f32 = 0.2;
const HEIGHT_PENALTY: f32 = 0.6;
/// Moss is at its thickest up to here above the ground, and thinnest from `FOOT + FADE` up.
const FOOT: f32 = 1.2;
const FADE: f32 = 5.6;
/// At this strength and over a face is covered edge to edge, so that a colony is one
/// unbroken mat with a ragged rim, and neighbouring faces merge into a few boxes.
const SOLID: f32 = 0.5;

/// How far up from the damp at the foot of things a height is, 0 at the foot to 1 well up.
fn rise(height: f32) -> f32 {
    ((height - m(FOOT)) / m(FADE)).clamp(0.0, 1.0)
}

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
        mat::BARK_OAK | mat::BARK_PINE | mat::BARK_REDWOOD | mat::BARK_JUNGLE | mat::BARK_WILLOW | mat::BARK_BIRCH | mat::WOOD_DEAD | mat::BARK_HOLLY | mat::BARK_YEW => Host::Bark,
        // Every grey bed of the strata: a wall is built of whichever is in the ground
        // (`Palette::local`), and goes as green as one of plain stone.
        mat::STONE | mat::ROCK_DARK | mat::ROCK_PALE | mat::ROCK_BROWN | mat::GRANITE | mat::GRANITE_GREY | mat::GRANITE_PALE => Host::Rock,
        mat::MOSSY_STONE | mat::SANDSTONE | mat::SANDSTONE_DARK | mat::PLANK | mat::PLANK_PALE | mat::PLANK_DARK | mat::PLANK_RED => Host::Built,
        _ => return None,
    })
}

/// The kinds of moss, each with its own form and colours.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Moss {
    /// Soft, uneven pads with raised crowns.
    Cushion,
    /// A flat mat that spreads.
    Carpet,
    /// Shaggy, in streaks that run down a wall.
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
        // Wet upland: lichen and moss crust the granite, though the open ground is windy.
        Biome::Moor => 0.62,
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
fn appeal(host: Host, toward: Toward, shade: f32, wet: f32, height: f32) -> f32 {
    let (age, facing) = match host {
        Host::Bark => (0.6, 0.0),
        Host::Rock => (0.75, 0.0),
        Host::Built => (0.9, 0.0),
        Host::Ground => (0.45, 0.0),
    };
    // The sun tracks across the +Z side of the sky, so -Z walls stay in the shade.
    // Undersides are held well below walls: a little moss on a dark ceiling looks right,
    // a lot reads as something hanging.
    let aspect = facing
        + match toward {
            Toward::NegZ => 0.26,
            Toward::PosX | Toward::NegX => 0.10,
            Toward::PosZ => -0.1,
            // The top of a log, a boulder or a wall holds the damp: that is where moss caps.
            Toward::Below if host != Host::Ground => TOP_BONUS,
            Toward::Below => 0.05,
            Toward::Above => -UNDERSIDE_PENALTY,
        }
        + match host {
            Host::Ground => 0.0,
            // Thickest round the foot of a trunk, thinning out up it.
            Host::Bark => FOOT_BONUS - HEIGHT_PENALTY * rise(height),
            Host::Rock | Host::Built => (FOOT_BONUS - HEIGHT_PENALTY * rise(height)) * 0.5,
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
    grow_within(voxels, mats, soil, COLUMN_BOXES, out)
}

/// [`grow`], with at most `budget` boxes.
fn grow_within(voxels: &ColumnVoxels, mats: &Materials, soil: &Soil, budget: usize, out: &mut Vec<PlantBox>) {
    // Nothing grows on a column that is under water or not open ground (see `judge`).
    let grows = |c: &Column| matches!(c.cover, Cover::Land | Cover::Bank | Cover::Spring) && c.water_surface().is_none() && moisture(c) > 0.0;
    if !soil.columns.iter().any(grows) {
        return;
    }
    let mut patches: Vec<Patch> = Vec::new();
    for_each_site(
        voxels,
        mats,
        |v| host_of(v).is_some(),
        |site| {
            if let Some(p) = judge(site, soil).filter(|_| !indoors(voxels, site)) {
                patches.push(p);
            }
        },
    );
    // The most deserving first, in an order that does not depend on how they were found.
    patches.sort_unstable_by(|a, b| b.score.total_cmp(&a.score).then_with(|| (a.site.air.to_array(), a.site.toward as u8).cmp(&(b.site.air.to_array(), b.site.toward as u8))));
    patches.truncate(COLUMN_SITES);
    // A column that overspends gives up its least deserving faces, whole: what is left is
    // still unbroken colonies in the best places, where dropping boxes at random left specks.
    let start = out.len();
    let mut keep = patches.len();
    for _ in 0..4 {
        out.truncate(start);
        let mut raster = Raster::new();
        for p in &patches[..keep] {
            plant(p, soil.seed, &mut raster);
        }
        raster.finish(out, |_| (false, false));
        let boxes = out.len() - start;
        #[cfg(test)]
        if keep == patches.len() {
            tally::record(&patches, boxes);
        }
        if boxes <= budget {
            break;
        }
        keep = (keep as f32 * budget as f32 / boxes as f32 * 0.92) as usize;
    }
    #[cfg(test)]
    tally::kept(keep, patches.len());
    // If it still overspends, thin it evenly so no part of the column is left bare.
    if out.len() - start > budget {
        let mut mine: Vec<PlantBox> = out.drain(start..).collect();
        mine.sort_unstable_by_key(|b| hash3(soil.seed ^ 0xb0c5, b.lo[0], b.lo[1], b.lo[2]));
        mine.truncate(budget);
        out.append(&mut mine);
    }
}

/// How far up from a face a roof over it is looked for, in metres.
const ROOF_REACH: f32 = 8.0;

/// Whether a site is indoors: somewhere over it, past any beams, is a roof or the boards
/// of a floor or a deck. Moss and lichen want the rain and the light a roof keeps off. A
/// ruin has lost its roof, so its walls go as green as any wall out in the weather, and
/// under trees and overhangs of rock moss grows as it always has.
fn indoors(voxels: &ColumnVoxels, site: &Site) -> bool {
    site.canopy > 0
        && (1..=m(ROOF_REACH) as i32).any(|dy| {
            let v = voxels.voxel(site.air + IVec3::Y * dy);
            matches!(v, mat::THATCH | mat::SLATE | mat::ROOF_TILE | mat::PLANK | mat::PLANK_PALE | mat::PLANK_DARK | mat::PLANK_RED) || mat::ROOFING.contains(&v)
        })
}

/// Decides whether a site grows moss.
fn judge(site: &Site, soil: &Soil) -> Option<Patch> {
    judge_with(site, soil, GAP_CUT)
}

/// [`judge`], with the gaps cut where their noise is below `gap_cut`.
fn judge_with(site: &Site, soil: &Soil, gap_cut: f32) -> Option<Patch> {
    let host = host_of(site.host)?;
    let col = soil.column(site.air.x, site.air.z);
    if !matches!(col.cover, Cover::Land | Cover::Bank | Cover::Spring) || col.water_surface().is_some() {
        return None;
    }
    // A ceiling only in deep shade: the thin board of a signpost or the eave of a tower out
    // in the open stays bare.
    if site.toward == Toward::Above && site.canopy < UNDERSIDE_CANOPY {
        return None;
    }
    let shade = (site.canopy as f32 / 5.0).min(1.0);
    let wet = moisture(col);
    if wet == 0.0 {
        return None;
    }
    // Asked after the cheaper questions: most sites never get this far.
    if soil.taken.contains(&site.air) {
        return None;
    }
    let (x, y, z) = (site.air.x as f64, site.air.y as f64, site.air.z as f64);
    let gap = perlin3(soil.seed ^ 0x3057, x / m(GAP_SCALE) as f64, y / m(GAP_SCALE) as f64, z / m(GAP_SCALE) as f64);
    if gap < gap_cut {
        return None;
    }
    // Bigger colonies, with a noisy edge.
    let colony = perlin3(soil.seed ^ 0x3055, x / m(5.6) as f64, y / m(5.6) as f64, z / m(5.6) as f64)
        + 0.45 * perlin3(soil.seed ^ 0x3056, x / m(1.4) as f64, y / m(1.4) as f64, z / m(1.4) as f64);
    let score = appeal(host, site.toward, shade, wet, site.air.y as f32 - col.ground) + colony * 0.55;
    // Thinning toward the edge of a gap as toward the edge of a colony.
    let strength = (score * 2.5).min((gap - gap_cut) * 6.0).clamp(0.0, 1.0);
    (score > 0.0).then_some(Patch { site: *site, host, strength, score })
}

/// Plants one face. A colony's moss kind and shade come from a slow noise, so patches read
/// as one plant.
fn plant(p: &Patch, seed: u32, raster: &mut Raster) {
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
    let ceiling = site.toward == Toward::Above;
    // A ceiling carries only the flat kinds.
    if ceiling && matches!(moss, Moss::Cushion | Moss::Shaggy) {
        moss = Moss::Carpet;
    }
    let (main, tip) = moss.colours(shade);
    let face = match moss {
        Moss::Carpet => carpet(&mut rng, p.strength, main, tip),
        Moss::Cushion => cushion(&mut rng, p.strength, main, tip),
        Moss::Shaggy => shaggy(site.toward, &mut rng, p.strength, main, tip),
        Moss::Lichen => lichen(&mut rng, p.strength, main, tip),
    };
    lay(&mut site.plot(raster), &face, ceiling);
}

/// One cell of a face of moss.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Cell {
    Bare,
    /// Lying flat: half a cell, 0.05 m, thick.
    Skin(Voxel),
    /// Standing a full cell, 0.1 m, off the face.
    Raised(Voxel),
}

/// A face of moss, cell by cell, `[v][u]` in the face's own coordinates.
type Face = [[Cell; 4]; 4];

/// Most cells of a face raised to a full cell: enough to break up the plane, never a mound.
const RAISED_PER_FACE: usize = 2;

/// Puts a face of moss on its plot. On a ceiling nothing is raised: a cell standing down
/// from it would read as a drip.
fn lay(plot: &mut FacePlot, face: &Face, ceiling: bool) {
    for (v, row) in face.iter().enumerate() {
        for (u, &cell) in row.iter().enumerate() {
            let (u, v) = (u as i32, v as i32);
            match cell {
                Cell::Bare => {}
                Cell::Raised(voxel) if !ceiling => plot.fill(u, u + 1, v, v + 1, 0, 1, voxel),
                Cell::Skin(voxel) | Cell::Raised(voxel) => plot.skin(u, u + 1, v, v + 1, voxel),
            }
        }
    }
}

/// Raises up to `n` of a face's skin cells, chosen at random, in `voxel`, keeping to
/// [`RAISED_PER_FACE`] in all.
fn raise(face: &mut Face, rng: &mut Rng, n: i32, voxel: Voxel) {
    for _ in 0..n {
        let raised = face.iter().flatten().filter(|c| matches!(c, Cell::Raised(_))).count();
        let (u, v) = (rng.int(0, 3) as usize, rng.int(0, 3) as usize);
        if raised < RAISED_PER_FACE && matches!(face[v][u], Cell::Skin(_)) {
            face[v][u] = Cell::Raised(voxel);
        }
    }
}

/// A flat mat, ragged where the colony thins, with a lighter spot or two raised.
fn carpet(rng: &mut Rng, strength: f32, main: Voxel, tip: Voxel) -> Face {
    let solid = strength >= SOLID;
    let mut face = [[Cell::Bare; 4]; 4];
    // Where the colony thins, one lobe that shrinks with it: a rim, not a scatter of flakes.
    let (cu, cv) = (rng.range(0.5, 2.5), rng.range(0.5, 2.5));
    let reach = 1.0 + strength / SOLID * 2.2;
    for (v, row) in face.iter_mut().enumerate() {
        for (u, cell) in row.iter_mut().enumerate() {
            let d = ((u as f32 - cu).powi(2) + (v as f32 - cv).powi(2)).sqrt();
            if solid || d + rng.range(-0.3, 0.3) < reach {
                *cell = Cell::Skin(main);
            }
        }
    }
    // A raised cell breaks the mat round it into several boxes, so only now and then.
    let n = (rng.unit() < 0.2) as i32;
    raise(&mut face, rng, n, tip);
    face
}

/// Uneven pads: a few overlapping humps, the skin raised a cell where a hump is high, and
/// the highest crowns lighter, so neighbouring faces never line up into one even sheet.
fn cushion(rng: &mut Rng, strength: f32, main: Voxel, tip: Voxel) -> Face {
    let mut height = [[0.0f32; 4]; 4];
    let mounds = 1 + (strength > 0.45) as i32 + (rng.unit() < 0.35) as i32;
    for _ in 0..mounds {
        let (cu, cv) = (rng.range(-0.5, 3.5), rng.range(-0.5, 3.5));
        let r = rng.range(1.1, 2.4);
        let peak = rng.range(0.8, 1.4 + 1.2 * strength);
        for (v, row) in height.iter_mut().enumerate() {
            for (u, h) in row.iter_mut().enumerate() {
                let d2 = (u as f32 - cu).powi(2) + (v as f32 - cv).powi(2);
                let here = peak * (1.0 - d2 / (r * r)) + rng.range(-0.35, 0.35);
                *h = h.max(here);
            }
        }
    }
    let mut face = [[Cell::Bare; 4]; 4];
    // The highest few points of the humps are raised, the rest lies flat.
    let mut crowns: Vec<(f32, usize, usize)> = Vec::new();
    for (v, row) in height.iter().enumerate() {
        for (u, &h) in row.iter().enumerate() {
            if h >= 0.5 || strength >= SOLID {
                face[v][u] = Cell::Skin(main);
                if h >= 1.5 {
                    crowns.push((h, u, v));
                }
            }
        }
    }
    crowns.sort_unstable_by(|a, b| b.0.total_cmp(&a.0));
    let high = (rng.unit() < 0.6) as usize + (rng.unit() < 0.25) as usize;
    for &(_, u, v) in crowns.iter().take(high.min(RAISED_PER_FACE)) {
        face[v][u] = Cell::Raised(if rng.unit() < 0.6 { tip } else { main });
    }
    face
}

/// Shaggy moss: a band along the top of a wall with streaks running down from it, all
/// flat to the face. On a floor, a mat with a tuft or two.
fn shaggy(toward: Toward, rng: &mut Rng, strength: f32, main: Voxel, tip: Voxel) -> Face {
    let mut face = [[Cell::Bare; 4]; 4];
    for row in &mut face[2..] {
        row.fill(Cell::Skin(main));
    }
    if toward == Toward::Below {
        for _ in 0..rng.int(2, 4) {
            let (u, v) = (rng.int(0, 3) as usize, rng.int(0, 3) as usize);
            face[v][u] = Cell::Skin(if rng.unit() < 0.4 { tip } else { main });
        }
        let n = rng.int(1, 2);
        raise(&mut face, rng, n, tip);
        return face;
    }
    let strands = if strength > 0.5 { 3 } else { 2 };
    for _ in 0..strands {
        let u = rng.int(0, 3) as usize;
        let len = rng.int(2, 4);
        let colour = if rng.unit() < 0.5 { main } else { tip };
        let v = rng.int(0, 3);
        for row in (v - len).max(0)..=v {
            face[row as usize][u] = Cell::Skin(colour);
        }
    }
    let n = (rng.unit() < 0.4) as i32;
    raise(&mut face, rng, n, tip);
    face
}

/// A thin crust in scattered flat blotches, nothing raised.
fn lichen(rng: &mut Rng, strength: f32, main: Voxel, tip: Voxel) -> Face {
    let mut face = [[Cell::Bare; 4]; 4];
    let blotches = if strength > 0.5 { 3 } else { 2 };
    for i in 0..blotches {
        let (u, v) = (rng.int(0, 3), rng.int(0, 3));
        let (w, h) = (rng.int(1, 3), rng.int(1, 3));
        for row in &mut face[v as usize..(v + h).min(4) as usize] {
            for cell in &mut row[u as usize..(u + w).min(4) as usize] {
                if *cell == Cell::Bare {
                    *cell = Cell::Skin(if i == 0 { main } else { tip });
                }
            }
        }
    }
    face
}

#[cfg(test)]
pub mod tally {
    use super::*;
    use std::sync::Mutex;
    /// Patches by host and by top, side or underside; columns; boxes before the budget; columns over it.
    #[derive(Default, Debug, Clone)]
    pub struct Tally {
        pub patches: std::collections::BTreeMap<String, usize>,
        pub columns: usize,
        pub boxes: usize,
        pub over: usize,
        pub most: usize,
        pub kept: usize,
        pub judged: usize,
    }
    pub(super) fn kept(kept: usize, judged: usize) {
        if let Some(t) = TALLY.lock().unwrap().as_mut() {
            t.kept += kept;
            t.judged += judged;
        }
    }
    pub static TALLY: Mutex<Option<Tally>> = Mutex::new(None);
    pub(super) fn record(patches: &[Patch], boxes: usize) {
        if let Some(t) = TALLY.lock().unwrap().as_mut() {
            for p in patches {
                let way = match p.site.toward {
                    Toward::Below => "top",
                    Toward::Above => "under",
                    _ => "side",
                };
                *t.patches.entry(format!("{:?} {way}", p.host)).or_default() += 1;
            }
            t.columns += 1;
            t.boxes += boxes;
            t.over += (boxes > COLUMN_BOXES) as usize;
            t.most = t.most.max(boxes);
        }
    }
}

/// The share of columns of open, dry ground that end up with moss.
#[cfg(test)]
fn sunny_ground_coverage(biome: Biome, canopy: u32, cover: Cover, host: Voxel) -> f32 {
    let col = Column { ground: 40.0, water: None, cover, biome, temperature: 0.5, river: f32::MAX, clear: f32::MAX, worn: None };
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
    use crate::{World, continent::SimConfig, features::attach::Voxels, materials::build, world::GameWorld};

    fn column(biome: Biome) -> Column {
        Column { ground: 40.0, water: None, cover: Cover::Land, biome, temperature: 0.5, river: f32::MAX, clear: f32::MAX, worn: None }
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

    /// Counts what moss grows on around given places:
    /// `MOSS_AT="x,z;x,z" cargo test -p worldgen --release moss_census -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn moss_census() {
        let seed = std::env::var("MOSS_SEED").map_or(1, |s| s.parse().unwrap());
        let w = GameWorld::new(seed);
        let span: i32 = std::env::var("MOSS_SPAN").map_or(3, |s| s.parse().unwrap());
        let given = std::env::var("MOSS_AT").unwrap_or_else(|_| "27808,14368;38944,13600;48544,47008;43168,33952".into());
        for at in given.split(';').filter(|s| !s.is_empty()) {
            let (x, z) = at.split_once(',').unwrap();
            let (x, z): (i32, i32) = (x.parse().unwrap(), z.parse().unwrap());
            *tally::TALLY.lock().unwrap() = Some(Default::default());
            for cx in -span..=span {
                for cz in -span..=span {
                    w.generate_column(x.div_euclid(CHUNK) + cx, z.div_euclid(CHUNK) + cz);
                }
            }
            let t = tally::TALLY.lock().unwrap().take().unwrap();
            println!("{:?} at {x},{z}: {} columns, {} boxes a column, most {}, {} over budget, {}% of faces kept", w.column(x, z).biome, t.columns, t.boxes / t.columns.max(1), t.most, t.over, t.kept * 100 / t.judged.max(1));
            for (k, n) in &t.patches {
                println!("    {k}: {} a column", n / t.columns.max(1));
            }
        }
    }

    #[test]
    fn moss_is_thickest_at_the_foot_of_a_trunk_and_on_top_of_logs_and_boulders() {
        // Somewhere only middling damp and lightly shaded, where not everything is mossy.
        let faces = |biome: Biome, canopy: u32, host: Voxel, toward: Toward, y: i32| {
            let cols = vec![column(biome); voxel::PAD * voxel::PAD];
            (0..4000)
                .filter(|&i| {
                    let site = Site { air: IVec3::new(i * 3, y, i * 7 % 900), toward, host, canopy };
                    let soil = Soil { columns: &cols, x0: site.air.x, z0: site.air.z, seed: 13, taken: &HashSet::new() };
                    judge(&site, &soil).is_some()
                })
                .count()
        };
        let count = |host: Voxel, toward: Toward, y: i32| faces(Biome::Woodland, 1, host, toward, y);
        // The ground is at 40: a trunk's foot, and the same trunk 8 m up.
        let (foot, high) = (count(mat::BARK_OAK, Toward::PosX, 41), count(mat::BARK_OAK, Toward::PosX, 61));
        assert!(foot > high * 2, "{foot} faces at the foot, {high} high up");
        // In a dark, wet wood it still climbs.
        assert!(faces(Biome::OldGrowth, 5, mat::BARK_OAK, Toward::PosX, 61) > 400, "a trunk in old growth is bare above its foot");
        for host in [mat::BARK_OAK, mat::STONE] {
            let (top, side) = (count(host, Toward::Below, 43), count(host, Toward::PosX, 43));
            assert!(top as f32 > side as f32 * 1.25, "{host}: {top} on top, {side} on the side");
        }
    }

    #[test]
    fn a_column_over_its_budget_gives_up_whole_faces_and_keeps_the_rest_unbroken() {
        let (chunks, origin) = scene(64);
        let voxels = ColumnVoxels::new(&chunks, origin - 1, origin - 1);
        let cols = vec![column(Biome::OldGrowth); voxel::PAD * voxel::PAD];
        let soil = Soil { columns: &cols, x0: origin - 1, z0: origin - 1, seed: 5, taken: &HashSet::new() };
        let cells_of = |budget: usize| {
            let mut out = Vec::new();
            grow_within(&voxels, &build(), &soil, budget, &mut out);
            assert!(out.len() <= budget, "{} boxes of {budget}", out.len());
            let mut cells = HashSet::new();
            for b in &out {
                for x in b.lo[0]..b.hi[0] {
                    for y in b.lo[1]..b.hi[1] {
                        for z in b.lo[2]..b.hi[2] {
                            cells.insert((x, y, z, b.voxel, b.thin));
                        }
                    }
                }
            }
            (out.len(), cells)
        };
        let (all_boxes, all) = cells_of(usize::MAX);
        let (few_boxes, few) = cells_of(all_boxes / 3);
        assert!(!few.is_empty() && few.len() < all.len());
        // What is kept is exactly what would have grown there anyway.
        assert!(few.is_subset(&all), "moss changed where it was kept");
        // Whole faces go, so the boxes left are no smaller than before: no specks.
        assert!(few.len() as f32 / few_boxes as f32 >= all.len() as f32 / all_boxes as f32 * 0.8, "{} cells in {few_boxes} boxes, {} in {all_boxes}", few.len(), all.len());
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

    /// Cells of moss in a box, and whether they are skin.
    fn cells(b: &PlantBox) -> i32 {
        (0..3).map(|i| b.hi[i] - b.lo[i]).product()
    }

    #[test]
    fn moss_lies_flat_with_few_raised_cells() {
        for (biome, seed) in [(Biome::OldGrowth, 5), (Biome::Rainforest, 7), (Biome::Taiga, 9)] {
            let out = grown(biome, 64, seed);
            let (mut skin, mut raised) = (0, 0);
            for b in &out {
                let dims = [b.hi[0] - b.lo[0], b.hi[1] - b.lo[1], b.hi[2] - b.lo[2]];
                if b.thin != 0 {
                    // A skin: one face pulled in, one cell deep along it, drawn as half.
                    assert_eq!(b.thin.count_ones(), 1, "{b:?}");
                    let axis = b.thin.trailing_zeros() as usize / 2;
                    assert_eq!(dims[axis], 1, "{b:?} is a skin more than a cell deep");
                    skin += cells(b);
                } else {
                    // Raised: never more than one cell off its face.
                    assert_eq!(*dims.iter().min().unwrap(), 1, "{b:?} stands too tall");
                    raised += cells(b);
                }
            }
            assert!(skin > 0, "{biome:?} grew no skin");
            // Mostly a 0.05 m skin, with a few full cells breaking it up.
            let share = raised as f32 / (skin + raised) as f32;
            assert!(share < 0.15, "{biome:?}: {raised} raised cells of {}", skin + raised);
        }
    }

    #[test]
    fn faces_are_skin_with_at_most_a_few_cells_raised() {
        let mut raised_faces = 0;
        for i in 0..4000u32 {
            let mut rng = Rng::new(i);
            let strength = (i % 11) as f32 / 10.0;
            let toward = Toward::ALL[(i % 6) as usize];
            let (main, tip) = (mat::MOSS_CARPET, mat::MOSS_CARPET_LIT);
            let face = match i % 4 {
                0 => carpet(&mut rng, strength, main, tip),
                1 => cushion(&mut rng, strength, main, tip),
                2 => shaggy(toward, &mut rng, strength, main, tip),
                _ => lichen(&mut rng, strength, main, tip),
            };
            let raised = face.iter().flatten().filter(|c| matches!(c, Cell::Raised(_))).count();
            assert!(raised <= RAISED_PER_FACE, "{face:?}");
            if i % 4 == 3 {
                assert_eq!(raised, 0, "lichen is only a crust");
            }
            raised_faces += (raised > 0) as u32;
        }
        assert!(raised_faces > 200, "some unevenness: {raised_faces}");
    }

    #[test]
    fn ceilings_get_only_a_flat_skin_and_only_in_deep_shade() {
        let cols = vec![column(Biome::Rainforest); voxel::PAD * voxel::PAD];
        let at = |i: i32| IVec3::new(i * 3, 41 + i % 17, i * 7 % 900);
        let judge_at = |i: i32, toward: Toward, host: Voxel, canopy: u32| {
            let site = Site { air: at(i), toward, host, canopy };
            let soil = Soil { columns: &cols, x0: site.air.x, z0: site.air.z, seed: 13, taken: &HashSet::new() };
            judge(&site, &soil)
        };
        // A signboard or an eave out in the open: never.
        for canopy in 1..UNDERSIDE_CANOPY {
            for i in 0..3000 {
                assert!(judge_at(i, Toward::Above, mat::PLANK, canopy).is_none(), "moss under a ceiling {canopy} thick");
            }
        }
        // Deep under an overhang or a crown: some, but clearly less than on the walls.
        for host in [mat::STONE, mat::BARK_OAK, mat::MOSSY_STONE] {
            let (mut under, mut side) = (0, 0);
            for i in 0..6000 {
                under += judge_at(i, Toward::Above, host, 8).is_some() as u32;
                side += judge_at(i, Toward::PosX, host, 8).is_some() as u32;
            }
            assert!(under > 0, "{host}: no moss under a dark overhang");
            assert!((under as f32) < side as f32 * 0.5, "{host}: {under} under, {side} on the side");
        }
        // Whatever grows there is skin against the ceiling, pulled up toward it.
        let mut boxes = Vec::new();
        for i in 0..3000 {
            let Some(p) = judge_at(i, Toward::Above, mat::STONE, 8) else { continue };
            let mut raster = Raster::new();
            plant(&p, 13, &mut raster);
            raster.finish(&mut boxes, |_| (false, false));
            for b in boxes.drain(..) {
                assert_eq!(b.thin, 1 << 3, "{b:?} hangs");
                assert_eq!(b.hi[1], (p.site.air.y + 1) * CELLS, "{b:?} is not against the ceiling");
                assert_eq!(b.hi[1] - b.lo[1], 1, "{b:?}");
            }
        }
    }

    #[test]
    fn colonies_are_a_quarter_to_a_third_sparser_than_before() {
        // The same sites judged with and without the gaps, over the kinds of place moss grows.
        let (mut with, mut without) = (0, 0);
        for biome in [Biome::OldGrowth, Biome::Deciduous, Biome::Taiga, Biome::Woodland] {
            let cols = vec![column(biome); voxel::PAD * voxel::PAD];
            for i in 0..8000 {
                let (host, toward) = match i % 4 {
                    0 => (mat::GRASS_LUSH, Toward::Below),
                    1 => (mat::BARK_OAK, Toward::ALL[(i / 4 % 4 + 2) as usize]),
                    2 => (mat::STONE, Toward::ALL[(i / 4 % 4 + 2) as usize]),
                    _ => (mat::MOSSY_STONE, Toward::ALL[(i / 4 % 4 + 2) as usize]),
                };
                let site = Site { air: IVec3::new(i * 5, 30 + i % 23, i * 11 % 1300), toward, host, canopy: (i % 7) as u32 };
                let soil = Soil { columns: &cols, x0: site.air.x, z0: site.air.z, seed: 17, taken: &HashSet::new() };
                with += judge_with(&site, &soil, GAP_CUT).is_some() as u32;
                without += judge_with(&site, &soil, f32::NEG_INFINITY).is_some() as u32;
            }
        }
        let kept = with as f32 / without as f32;
        assert!((0.62..=0.80).contains(&kept), "kept {kept}: {with} of {without}");
    }
}
