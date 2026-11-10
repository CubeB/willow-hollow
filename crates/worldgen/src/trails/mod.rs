//! Trails and landmarks: the trace of people on the land.
//!
//! After a continent has been simulated, sites are chosen from its map and joined by a
//! sparse network of paths (following the RoadWeaver method: best-first routing with
//! quadratic height cost, simplified, relaxed and splined, with a slope-clamped height
//! profile). The result is stored with the continent and consulted in two ways: the
//! surface sampler levels and paves the ground along trails, and chunk generation
//! stamps the structures (landmarks, bridges, lanterns) that overlap each column.

pub mod build;
pub mod route;
pub mod sites;

use std::collections::HashMap;

use glam::{IVec3, Vec2, Vec3};
use voxel::{CHUNK, Voxel};

pub use build::build;

use crate::features::landmarks::Landmark;

/// Size of the tiles trail points are filed under, in voxels.
const TILE: i32 = 64;
/// Farthest from its centreline a trail or pad touches the ground.
pub const REACH: f32 = 12.0;

/// One voxel-length step of a trail, in continent-local voxels.
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

/// A levelled circle of ground under a landmark.
#[derive(Clone, Copy, Debug)]
pub struct Pad {
    pub x: f32,
    pub z: f32,
    pub radius: f32,
    pub y: f32,
}

/// Voxels to write into the world, in continent-local coordinates.
pub struct Stamp {
    pub lo: IVec3,
    pub hi: IVec3,
    pub voxels: Vec<(IVec3, Voxel)>,
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

/// Everything people have added to one continent.
#[derive(Default)]
pub struct Network {
    pub sites: Vec<Site>,
    pub crossings: Vec<Crossing>,
    /// Where one trail meets another.
    pub junctions: Vec<Vec2>,
    /// Each trail as a line, for maps.
    pub lines: Vec<Vec<Vec2>>,
    tiles: HashMap<(i32, i32), Vec<TrailPoint>>,
    pads: Vec<Pad>,
    stamps: Vec<Stamp>,
    /// Stamps overlapping each chunk column, by continent-local chunk coordinates.
    stamp_index: HashMap<(i32, i32), Vec<u32>>,
}

impl Network {
    fn add_points(&mut self, points: &[TrailPoint]) {
        for p in points {
            let key = ((p.x as i32).div_euclid(TILE), (p.z as i32).div_euclid(TILE));
            self.tiles.entry(key).or_default().push(*p);
        }
    }

    fn add_stamp(&mut self, voxels: Vec<(IVec3, Voxel)>) {
        let Some(&(first, _)) = voxels.first() else { return };
        let (mut lo, mut hi) = (first, first);
        for (p, _) in &voxels {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        let id = self.stamps.len() as u32;
        // The one-voxel chunk border means a stamp also touches chunks a voxel beyond.
        for cz in (lo.z - 1).div_euclid(CHUNK)..=(hi.z + 1).div_euclid(CHUNK) {
            for cx in (lo.x - 1).div_euclid(CHUNK)..=(hi.x + 1).div_euclid(CHUNK) {
                self.stamp_index.entry((cx, cz)).or_default().push(id);
            }
        }
        self.stamps.push(Stamp { lo, hi, voxels });
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
        let pads = self
            .pads
            .iter()
            .filter(|p| p.x + p.radius >= lo.x && p.x - p.radius <= hi.x && p.z + p.radius >= lo.y && p.z - p.radius <= hi.y)
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

    /// Every trail point; for tests and tools.
    pub fn points(&self) -> impl Iterator<Item = &TrailPoint> {
        self.tiles.values().flatten()
    }
}

#[cfg(test)]
mod tests;
