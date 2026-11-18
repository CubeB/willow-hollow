//! Trails and landmarks: the trace of people on the land.
//!
//! After a continent has been simulated, sites are chosen from its map and joined by a
//! sparse network of paths (following the RoadWeaver method: best-first routing with
//! quadratic height cost, simplified, relaxed and splined, with a slope-clamped height
//! profile). The result is stored with the continent and consulted in two ways: the
//! surface sampler levels and paves the ground along trails, and chunk generation
//! stamps the structures (landmarks, bridges, lanterns) that overlap each column.

pub mod build;
pub mod delights;
pub mod roadside;
pub mod route;
pub mod sites;
pub mod siting;
pub mod tread;

use std::collections::HashMap;

use glam::{IVec3, Vec2, Vec3};
use voxel::{CHUNK, Voxel, mesh::{CELLS, PlantBox}};

pub use build::build;

use crate::{features::landmarks::Landmark, scale::m};

/// Size of the tiles trail points are filed under, in voxels.
const TILE: i32 = 64;
/// Farthest from its centreline a trail or pad touches the ground.
pub const REACH: f32 = m(9.6);

/// One 0.8 m step of a trail, in continent-local voxels.
#[derive(Clone, Copy, Debug)]
pub struct TrailPoint {
    pub x: f32,
    pub z: f32,
    /// Height of the trail's surface.
    pub y: f32,
    /// Half-width of the paved strip. Zero or less where a bridge carries the trail:
    /// the ground is left alone, but trees are still kept out of the way.
    pub half_width: f32,
}

/// How far beyond the edge of its pad a landmark's clearing reaches: trees are thinned
/// and felled out to here, and the surface sampler gathers pads this far out.
pub const CLEARING: f32 = m(16.0);

/// How much further than [`REACH`] pads are gathered, for their clearings.
fn p_reach() -> f32 {
    (CLEARING - REACH).max(0.0)
}

/// A levelled circle of ground under a landmark.
#[derive(Clone, Copy, Debug)]
pub struct Pad {
    pub x: f32,
    pub z: f32,
    pub radius: f32,
    pub y: f32,
    /// Cut into a slope as a terrace, held by a dry-stone wall where the ground beside
    /// it stands well above or below it (see [`build`]'s walls).
    pub walled: bool,
    /// How trodden the ground is, 0 (grassed over) to 1 (bare where people walk).
    pub wear: f32,
    /// How far the place has gone to ruin, from its [`History`](crate::features::landmarks::History):
    /// a kept site has its trees felled back, a forgotten one fills in with young growth.
    pub neglect: f32,
    /// The way in: the direction from the middle to the door, radians in the x-z plane
    /// (atan2 of z over x), and how far out the door's step is. No door: NaN.
    pub front: f32,
    pub door: f32,
    /// The radius of the worn way round the building, from where each path arrives to
    /// the door.
    pub ring: f32,
    /// A worn patch about the middle (a camp's fire, a cairn's foot), its radius.
    pub hub: f32,
    /// Directions the paths come in from, as `front`.
    pub arrivals: [f32; 4],
    pub arrival_count: u8,
    /// A garden's bed, worked bare: the continent-local columns from the first corner to
    /// the second, inclusive. None on a pad with no garden.
    pub garden: Option<(glam::IVec2, glam::IVec2)>,
}

impl Pad {
    /// A plain levelled pad, before its history and paths are known.
    pub fn plain(x: f32, z: f32, radius: f32, y: f32) -> Self {
        Pad {
            x,
            z,
            radius,
            y,
            walled: false,
            wear: 0.0,
            neglect: 0.0,
            front: f32::NAN,
            door: 0.0,
            ring: 0.0,
            hub: 0.0,
            arrivals: [0.0; 4],
            arrival_count: 0,
            garden: None,
        }
    }

    /// Whether the ground at continent-local `p` (inside the pad) is worn bare: along
    /// the way from each path's arrival round to the door, in front of the door, and
    /// about the hub, ragged at the edges and broken up the less the place is used.
    pub fn worn(&self, seed: u32, p: Vec2, lx: i32, lz: i32) -> bool {
        // A garden is dug over, so nothing wild grows in it.
        if let Some((lo, hi)) = self.garden
            && (lo.x..=hi.x).contains(&lx)
            && (lo.y..=hi.y).contains(&lz)
        {
            return true;
        }
        if self.wear <= 0.0 {
            return false;
        }
        let rel = p - Vec2::new(self.x, self.z);
        let r = rel.length();
        // Patchy where little used: a slow noise decides which stretches still show.
        let patch = crate::noise::perlin2(seed ^ 0x3017, p.x as f64 / m(2.4) as f64, p.y as f64 / m(2.4) as f64) * 0.5 + 0.5;
        if patch > 0.25 + self.wear {
            return false;
        }
        let ragged = (crate::noise::unit(crate::noise::hash2(seed ^ 0x3018, lx, lz)) - 0.5) * m(0.5);
        let half = m(0.3) + m(0.45) * self.wear + ragged;
        if r < self.hub + ragged {
            return true;
        }
        if self.front.is_nan() {
            return false;
        }
        let theta = rel.y.atan2(rel.x);
        let diff = |a: f32, b: f32| (a - b + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        let ray = |a: f32| Vec2::new(a.cos(), a.sin());
        // Before the door and out from it to the ring.
        let door = ray(self.front) * self.door;
        if (p - Vec2::new(self.x, self.z) - door).length() < m(0.9) + m(1.0) * self.wear + ragged {
            return true;
        }
        let along = rel.dot(ray(self.front));
        let across = (rel - ray(self.front) * along).length();
        if along > self.door - m(0.4) && along < self.ring + half && across < half {
            return true;
        }
        // Round the ring from each arrival to the door, and in from the edge to the ring.
        for &a in &self.arrivals[..self.arrival_count as usize] {
            let (to_front, here) = (diff(self.front, a), diff(theta, a));
            let on_arc = if to_front >= 0.0 { (0.0..=to_front).contains(&here) } else { (to_front..=0.0).contains(&here) };
            if on_arc && (r - self.ring).abs() < half {
                return true;
            }
            let along = rel.dot(ray(a));
            if along > self.ring - half && (rel - ray(a) * along).length() < half {
                return true;
            }
        }
        false
    }
}

/// Why a landmark stands where it does.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Reason {
    /// On a rise, with the land falling away below.
    View,
    /// Beside a spring.
    Spring,
    /// Near a stream or a lake shore.
    Water,
    /// Where a narrow stream can be waded.
    Ford,
    /// In the low gap of a ridge.
    Pass,
    /// Backed by rising ground.
    Shelter,
    /// On the coast, by open water.
    Shore,
    /// Nothing better near: the first good level ground.
    Open,
}

/// Voxels to write into the world, in continent-local coordinates.
#[derive(Clone)]
pub struct Stamp {
    pub lo: IVec3,
    pub hi: IVec3,
    pub voxels: Vec<(IVec3, Voxel)>,
    /// Fine parts (lantern frames, chains and the like) in continent-local cells of
    /// 1/CELLS voxel. `lo` and `hi` cover these too.
    pub fine: Vec<PlantBox>,
    /// What it is, if it changes over the years (#207): what reads it sees it through
    /// `settlements::years::Lives::view`. None for what is the same in every year.
    pub tag: Option<Box<crate::settlements::years::Tag>>,
}

/// Where a boat is tied up.
#[derive(Clone, Copy, Debug)]
pub struct Mooring {
    /// Continent-local position on the water surface.
    pub pos: Vec3,
    /// Direction the bow points, radians.
    pub heading: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Site {
    pub kind: Landmark,
    /// Continent-local voxel column at the middle of the site.
    pub x: i32,
    pub z: i32,
    /// First layer above the levelled ground.
    pub y: i32,
    pub mooring: Option<Mooring>,
    /// What brought it here.
    pub reason: Reason,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CrossingKind {
    Stones,
    Log,
    Plank,
    Arch,
}

/// A place where a trail crosses water.
#[derive(Clone, Copy, Debug)]
pub struct Crossing {
    pub kind: CrossingKind,
    pub x: i32,
    pub z: i32,
    /// Water columns spanned.
    pub span: i32,
}

/// Why a lantern stands where it does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LanternCause {
    /// At the head of a bridge.
    Bridge,
    /// Where trails meet.
    Junction,
    /// On the approach to a landmark.
    Approach,
    /// Somewhere along an otherwise plain stretch.
    Plain,
    /// Along the main lane of a town or a large village.
    Lane,
}

/// A lantern beside a trail.
#[derive(Clone, Debug)]
pub struct TrailLantern {
    pub at: Vec2,
    /// The trail it lights, by the salt it was laid with.
    pub trail: u32,
    /// Its step along that trail.
    pub step: usize,
    pub cause: LanternCause,
    pub style: crate::features::parts::LampStyle,
    /// Whether anyone has lit it: most stand cold.
    pub lit: bool,
    /// The columns where it stands on the ground: its footprint in its lowest layers.
    pub footprint: Vec<glam::IVec2>,
}

/// Everything people have added to one continent.
#[derive(Default)]
pub struct Network {
    pub sites: Vec<Site>,
    pub crossings: Vec<Crossing>,
    /// Links between sites that no route was found for, so the network is split there.
    pub unlinked: usize,
    /// Where one trail meets another.
    pub junctions: Vec<Vec2>,
    /// Each trail as a line, for maps.
    pub lines: Vec<Vec<Vec2>>,
    /// The half-width its traffic gives each trail at each point of `lines`.
    pub line_widths: Vec<Vec<f32>>,
    /// Every lantern set beside a trail, in the order they were placed.
    pub lanterns: Vec<TrailLantern>,
    /// The name of every place on the continent, landmarks and natural features alike.
    /// Filled in once the network itself is built (see [`crate::names::survey`]).
    pub names: crate::names::Gazetteer,
    tiles: HashMap<(i32, i32), Vec<TrailPoint>>,
    pub(crate) pads: Vec<Pad>,
    stamps: Vec<Stamp>,
    /// Lanterns chosen but not yet built, waiting until every trail is down so none
    /// is built on a trail laid after it: (its index in `lanterns`, voxels, fine parts).
    pub(crate) held_lanterns: Vec<(usize, Vec<(IVec3, Voxel)>, Vec<PlantBox>)>,
    /// Props laid about sites, held in the same way: voxels, fine parts, and the world
    /// columns they stand in.
    pub(crate) held_props: Vec<(Vec<(IVec3, Voxel)>, Vec<PlantBox>, Vec<glam::IVec2>)>,
    /// Stamps overlapping each chunk column, by continent-local chunk coordinates.
    stamp_index: HashMap<(i32, i32), Vec<u32>>,
    /// Every milestone, bench, waymark and cairn set beside a trail, and where: the
    /// continent-local voxel its model's foot stands in, one above the ground.
    pub markers: Vec<(roadside::Marker, IVec3)>,
    /// Markers chosen but not yet built, held like the props for the same reason.
    held_markers: Vec<roadside::Placed>,
    /// Every small delight left beside a trail (wind chimes, prayer flags, a seat at a
    /// view and the like), and where. Sound for the chimes starts from here.
    pub delights: Vec<delights::Standing>,
    /// Delights chosen but not yet built, held like the markers.
    held_delights: Vec<delights::Placed>,
    /// Every hamlet, village and town, placed before the landmarks: towns first, then
    /// villages, then hamlets. Each stands about a green, a pad after the landmarks' own
    /// (so [`Self::gather`] finds it), and is a node of the trails as a landmark is.
    pub settlements: Vec<crate::settlements::Settlement>,
    /// Where smoke leaves every lived-in home with a fire: the top of each chimney or
    /// smoke hole, in continent-local voxels. The game sends chimney smoke up from here.
    pub chimneys: Vec<Vec3>,
    /// Every walk-in building of every settlement, in the order they were laid out:
    /// settlement by settlement, each one's in the order its plots were chosen.
    pub plots: Vec<crate::settlements::layout::Plot>,
    /// How each settlement is laid out about its buildings, by index in `settlements`.
    pub layouts: Vec<crate::settlements::layout::Layout>,
    /// Where each settlement's tree stands, by index in `settlements`: the foot of its
    /// trunk, continent-local, or None where it has none.
    pub village_trees: Vec<Option<IVec3>>,
    /// Where each settlement's tree has its ribbons tied, and the spare places under its
    /// limbs a festival hangs its lanterns from (#207; see
    /// [`crate::settlements::tree::Dressing`]), by index in `settlements`: continent-local
    /// cells.
    pub tree_knots: Vec<(Vec<IVec3>, Vec<IVec3>)>,
    /// Every tree planted by hand, for [`crate::features::place`] to grow.
    pub planted: Vec<crate::features::Planted>,
    /// The ground trodden bare round each village tree.
    pub rings: Vec<crate::settlements::tree::WornRing>,
    /// Where a traveller may sleep the night: the middle of each guest bed of every
    /// tavern, on its boards, in continent-local voxels (see
    /// [`crate::settlements::special::Built::guest_beds_at`]).
    pub guest_beds: Vec<Vec3>,
    /// The notice boards on the walls of the taverns and guild houses (#207), the
    /// player's to read: by index in `plots`, and where.
    pub boards: Vec<crate::settlements::special::Board>,
    /// The guilds that share each guild house (#207), by its index in `plots`.
    pub guild_halls: Vec<(usize, Vec<crate::settlements::special::Guild>)>,
    /// The quays the boats of the settlements tie up at, the river runs between partners'
    /// quays and the fishing grounds off the ports (#205).
    pub waterways: crate::settlements::quay::Waterways,
    /// What the settlements keep for how they change over the years (#207).
    pub years: crate::settlements::years::Record,
}

impl Network {
    pub(crate) fn add_points(&mut self, points: &[TrailPoint]) {
        for p in points {
            let key = ((p.x as i32).div_euclid(TILE), (p.z as i32).div_euclid(TILE));
            self.tiles.entry(key).or_default().push(*p);
        }
    }

    fn add_stamp(&mut self, voxels: Vec<(IVec3, Voxel)>) {
        self.add_stamp_with(voxels, Vec::new());
    }

    /// A stamp with fine parts, in continent-local cells.
    pub(crate) fn add_stamp_with(&mut self, mut voxels: Vec<(IVec3, Voxel)>, fine: Vec<PlantBox>) {
        // Models are built in hash maps, so their voxels come in a different order each
        // run. The coarse rings and the far boxes keep whichever voxel of a cell comes
        // last, so put them in one order; a stable sort leaves any voxel written twice at
        // one place in the order its builder meant.
        voxels.sort_by_key(|&(p, _)| (p.y, p.z, p.x));
        // Bounds from the voxels, or for a model of fine parts alone, from those.
        let first = match (voxels.first(), fine.first()) {
            (Some(&(p, _)), _) => p,
            (None, Some(b)) => IVec3::from_array(b.lo).div_euclid(IVec3::splat(CELLS)),
            (None, None) => return,
        };
        let (mut lo, mut hi) = (first, first);
        for (p, _) in &voxels {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        for b in &fine {
            lo = lo.min(IVec3::from_array(b.lo).div_euclid(IVec3::splat(CELLS)));
            hi = hi.max((IVec3::from_array(b.hi) - 1).div_euclid(IVec3::splat(CELLS)));
        }
        let id = self.stamps.len() as u32;
        // The one-voxel chunk border means a stamp also touches chunks a voxel beyond.
        for cz in (lo.z - 1).div_euclid(CHUNK)..=(hi.z + 1).div_euclid(CHUNK) {
            for cx in (lo.x - 1).div_euclid(CHUNK)..=(hi.x + 1).div_euclid(CHUNK) {
                self.stamp_index.entry((cx, cz)).or_default().push(id);
            }
        }
        self.stamps.push(Stamp { lo, hi, voxels, fine, tag: None });
    }

    /// [`Self::add_stamp_with`] for what changes over the years (#207), with its `tag`.
    pub(crate) fn add_stamp_tagged(&mut self, voxels: Vec<(IVec3, Voxel)>, fine: Vec<PlantBox>, tag: crate::settlements::years::Tag) {
        let n = self.stamps.len();
        self.add_stamp_with(voxels, fine);
        if let Some(s) = self.stamps.get_mut(n) {
            s.tag = Some(Box::new(tag));
        }
    }

    /// Trail points and pads that can affect the square from `lo` to `hi`.
    pub fn gather(&self, lo: Vec2, hi: Vec2) -> (Vec<TrailPoint>, Vec<Pad>) {
        let (lo, hi) = (lo - Vec2::splat(REACH), hi + Vec2::splat(REACH));
        let mut points = Vec::new();
        if !self.tiles.is_empty() {
            for tz in (lo.y as i32).div_euclid(TILE)..=(hi.y as i32).div_euclid(TILE) {
                for tx in (lo.x as i32).div_euclid(TILE)..=(hi.x as i32).div_euclid(TILE) {
                    if let Some(tile) = self.tiles.get(&(tx, tz)) {
                        points.extend(tile.iter().filter(|p| p.x >= lo.x && p.x <= hi.x && p.z >= lo.y && p.z <= hi.y));
                    }
                }
            }
        }
        // Pads further, out to the reach of their clearings.
        let reach = p_reach();
        let pads = self
            .pads
            .iter()
            .filter(|p| p.x + p.radius + reach >= lo.x && p.x - p.radius - reach <= hi.x && p.z + p.radius + reach >= lo.y && p.z - p.radius - reach <= hi.y)
            .copied()
            .collect();
        (points, pads)
    }

    /// Stamps overlapping a chunk column, by continent-local chunk coordinates.
    pub fn stamps_at(&self, cx: i32, cz: i32) -> impl Iterator<Item = &Stamp> {
        self.stamp_index
            .get(&(cx, cz))
            .into_iter()
            .flatten()
            .map(|&id| &self.stamps[id as usize])
    }

    pub fn stamp_count(&self) -> usize {
        self.stamps.len()
    }

    /// Every stamp, in the order they were laid: for what is drawn of them from afar.
    pub fn stamps(&self) -> impl Iterator<Item = &Stamp> {
        self.stamps.iter()
    }

    /// Every trail point; for tests and tools.
    pub fn points(&self) -> impl Iterator<Item = &TrailPoint> {
        self.tiles.values().flatten()
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod site_tests;
