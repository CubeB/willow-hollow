//! Parts for built things, modelled directly in 0.4 m voxels (not the 0.8 m blocks the
//! older landmark code lays out in): lantern posts in four styles, and the trims that
//! stop a block structure reading as a heap of cubes.
//!
//! Every part takes its colours from a [`Palette`], so a lantern in the desert is
//! sandstone and acacia and one in the redwoods is mossy rock and red timber.

use glam::IVec3;
use voxel::Voxel;

use super::{landmarks::Palette, shape::{Rng, Shape, rotate}};
use crate::materials as mat;

/// How a lantern is built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LampStyle {
    /// A post with a caged lamp on top, under a small roof and finial.
    Post,
    /// A post with a crook arm and a lamp hanging from a chain.
    Crook,
    /// A stone lantern: tiered base, lit box, a roof.
    Stone,
    /// A crossbar carrying two hanging lamps.
    Pair,
}

impl LampStyle {
    pub const ALL: [LampStyle; 4] = [LampStyle::Post, LampStyle::Crook, LampStyle::Stone, LampStyle::Pair];

    /// A style from a hash, favouring the plain post.
    pub fn pick(hash: u32) -> Self {
        match hash % 8 {
            0..=2 => LampStyle::Post,
            3..=4 => LampStyle::Crook,
            5 => LampStyle::Stone,
            _ => LampStyle::Pair,
        }
    }
}

/// The voxels of a lantern post `height` voxels tall to the lamp, standing on the ground
/// with its footing below. The origin column is the post, the first layer above the
/// ground is `y = 0`, and the arm of a crook points along +X unless `turns` says otherwise.
pub fn lantern(style: LampStyle, p: &Palette, height: i32, turns: u32) -> Vec<(IVec3, Voxel)> {
    let mut s = Shape::raw();
    paint_lantern(&mut s, style, p, IVec3::ZERO, height, turns);
    s.into_voxels()
}

/// Puts a lantern into a shape, with its foot at voxel `at` (the first layer above the
/// ground).
pub fn paint_lantern(s: &mut Shape, style: LampStyle, p: &Palette, at: IVec3, height: i32, turns: u32) {
    let h = height.max(5);
    let mut put = |x: i32, y: i32, z: i32, v: Voxel| {
        let q = rotate(IVec3::new(x, y, z), turns);
        s.paint(at + IVec3::new(q.x, y, q.z), v);
    };
    // A footing running down into the ground, so nothing floats on uneven land.
    for y in -3..0 {
        put(0, y, 0, p.stone_dark);
    }
    // Every style stands on a plus-shaped stone base.
    for (x, z) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
        put(x, 0, z, p.stone_dark);
    }
    match style {
        LampStyle::Post => {
            put(0, 1, 0, p.stone);
            for y in 2..h {
                put(0, y, 0, p.wood);
            }
            // The lamp, caged: posts at the corners, light in the middle.
            put(0, h, 0, mat::LANTERN);
            for (x, z) in [(1, 1), (-1, 1), (1, -1), (-1, -1)] {
                put(x, h, z, p.wood);
            }
            put(0, h - 1, 0, p.wood);
            // A little roof over the cage, tapering to a finial.
            for x in -1..=1 {
                for z in -1..=1 {
                    put(x, h + 1, z, p.roof);
                }
            }
            put(0, h + 2, 0, p.roof);
            put(0, h + 3, 0, p.accent);
        }
        LampStyle::Crook => {
            put(0, 1, 0, p.stone);
            for y in 2..=h {
                put(0, y, 0, p.wood);
            }
            put(0, h + 1, 0, p.roof);
            // The arm, with a brace under it.
            put(1, h, 0, p.wood);
            put(2, h, 0, p.wood);
            put(1, h - 1, 0, p.wood);
            // A chain, the lamp, and a foot under it.
            put(2, h - 1, 0, p.wood);
            put(2, h - 2, 0, mat::LANTERN);
            put(2, h - 3, 0, p.stone_dark);
        }
        LampStyle::Stone => {
            // A tiered stone lantern.
            for y in 1..=2 {
                put(0, y, 0, p.stone);
            }
            for (x, z) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
                put(x, 3, z, p.stone_dark);
            }
            put(0, 4, 0, mat::LANTERN);
            for (x, z) in [(1, 1), (-1, 1), (1, -1), (-1, -1)] {
                put(x, 4, z, p.stone);
            }
            for x in -1..=1 {
                for z in -1..=1 {
                    put(x, 5, z, p.roof);
                }
            }
            put(0, 6, 0, p.roof);
            put(0, 7, 0, p.stone);
        }
        LampStyle::Pair => {
            put(0, 1, 0, p.stone);
            for y in 2..=h {
                put(0, y, 0, p.wood);
            }
            put(0, h + 1, 0, p.accent);
            for x in -2..=2 {
                put(x, h, 0, p.wood);
            }
            for x in [-2, 2] {
                put(x, h - 1, 0, p.wood);
                put(x, h - 2, 0, mat::LANTERN);
                put(x, h - 3, 0, p.stone_dark);
            }
        }
    }
}

/// Breaks up the hard outline of blocky stonework: some of the voxels standing out at the
/// corners and top edges of a stone block are chipped away, a few more worn down to the
/// darker rock, and rubble (a loose voxel) lies at the foot of some walls. Only stone is
/// touched, never timber, so stairs, floors and doors keep their shape.
pub fn weather_stone(s: &mut Shape, rng: &mut Rng, p: &Palette, chip: f32) {
    let stone = [p.stone, p.stone_dark, mat::MOSSY_STONE, mat::SANDSTONE, mat::SANDSTONE_DARK, mat::SLATE];
    let is_stone = |v: Option<Voxel>| v.is_some_and(|v| stone.contains(&v));
    let at = |s: &Shape, q: IVec3| s.get(q).filter(|v| *v != voxel::AIR);
    let mut chips = Vec::new();
    let mut worn = Vec::new();
    for q in s.snapshot() {
        let v = s.get(q);
        if !is_stone(v) {
            continue;
        }
        let open = |d: IVec3| at(s, q + d).is_none();
        let top = open(IVec3::Y);
        // Exposed on top and at least one side: an edge or a corner.
        let sides = [IVec3::X, IVec3::NEG_X, IVec3::Z, IVec3::NEG_Z].iter().filter(|d| open(**d)).count();
        if top && sides >= 2 && rng.unit() < chip {
            chips.push(q);
        } else if (top || sides >= 1) && rng.unit() < chip * 0.5 {
            worn.push(q);
        }
    }
    for q in chips {
        s.paint(q, voxel::AIR);
    }
    for q in worn {
        let darker = if s.get(q) == Some(p.stone_dark) { p.stone } else { p.stone_dark };
        s.paint(q, darker);
    }
}
