//! The places trails lead to: shrines, towers, camps, ruins and the like.
//!
//! Every landmark is generated from a seed and a palette taken from where it stands, so
//! a shrine in the desert is sandstone and a shrine in the redwoods is mossy rock and
//! red timber, and no two are quite the same. In model space the origin column is the
//! middle of the site and `y = 0` is the first layer above the ground.

use std::f32::consts::TAU;

use glam::IVec3;
use voxel::{AIR, Voxel};

use super::{
    parts,
    shape::{Rng, Shape},
};
use crate::{biome::Biome, materials as mat, scale::m};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Landmark {
    Shrine,
    Lookout,
    Camp,
    Ruins,
    Stones,
    Cairn,
    Arch,
    Hut,
    Landing,
}

impl Landmark {
    pub const ALL: [Landmark; 9] = [
        Landmark::Shrine,
        Landmark::Lookout,
        Landmark::Camp,
        Landmark::Ruins,
        Landmark::Stones,
        Landmark::Cairn,
        Landmark::Arch,
        Landmark::Hut,
        Landmark::Landing,
    ];

    /// Radius of the ground levelled for this landmark.
    pub fn radius(self) -> f32 {
        match self {
            Landmark::Shrine | Landmark::Cairn => m(4.0),
            Landmark::Lookout | Landmark::Camp | Landmark::Arch => m(6.4),
            Landmark::Hut => m(7.2),
            Landmark::Ruins | Landmark::Stones => m(10.4),
            Landmark::Landing => m(3.2),
        }
    }

    /// Colour used to mark this landmark on maps.
    pub fn map_color(self) -> [u8; 3] {
        match self {
            Landmark::Shrine => [240, 220, 120],
            Landmark::Lookout => [240, 120, 90],
            Landmark::Camp => [250, 160, 60],
            Landmark::Ruins => [190, 180, 170],
            Landmark::Stones => [170, 150, 220],
            Landmark::Cairn => [255, 255, 255],
            Landmark::Arch => [230, 150, 200],
            Landmark::Hut => [130, 220, 150],
            Landmark::Landing => [90, 220, 240],
        }
    }
}

/// Building materials in keeping with a place.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub stone: Voxel,
    pub stone_dark: Voxel,
    pub wood: Voxel,
    pub plank: Voxel,
    pub roof: Voxel,
    pub accent: Voxel,
    /// Share of exposed stonework that has gone green.
    pub moss: f32,
    /// The biome this palette suits, for choosing among its alternatives.
    pub biome: Biome,
}

impl Palette {
    pub fn of(biome: Biome) -> Self {
        use Biome::*;
        let stone = |wood, plank, roof, accent, moss| Palette {
            stone: mat::STONE,
            stone_dark: mat::ROCK_DARK,
            wood,
            plank,
            roof,
            accent,
            moss,
            biome,
        };
        match biome.parent() {
            Desert | Savanna => Palette {
                stone: mat::SANDSTONE,
                stone_dark: mat::SANDSTONE_DARK,
                wood: mat::BARK_ACACIA,
                plank: mat::PLANK_PALE,
                roof: mat::THATCH,
                accent: mat::CLOTH_RED,
                moss: 0.0,
                biome,
            },
            Alpine | Tundra | Ocean => stone(mat::BARK_PINE, mat::PLANK_DARK, mat::SLATE, mat::CLOTH_BLUE, 0.04),
            Taiga => stone(mat::BARK_PINE, mat::PLANK_DARK, mat::SLATE, mat::CLOTH_BLUE, 0.2),
            OldGrowth => stone(mat::BARK_REDWOOD, mat::PLANK_RED, mat::THATCH, mat::CLOTH_PALE, 0.6),
            Rainforest => stone(mat::BARK_JUNGLE, mat::PLANK_PALE, mat::THATCH, mat::CLOTH_RED, 0.55),
            Heath | Grassland => stone(mat::BARK_OAK, mat::PLANK, mat::THATCH, mat::CLOTH_BLUE, 0.14),
            Deciduous | Woodland => stone(mat::BARK_OAK, mat::PLANK, mat::ROOF_TILE, mat::CLOTH_RED, 0.3),
            other => unreachable!("{other:?} is not an original biome"),
        }
    }

    /// This palette with its choices varied by `seed`: each place draws its roof, timber,
    /// accent colour and stonework from what suits the biome, so neighbouring structures
    /// belong together without being copies of one another.
    pub fn varied(mut self, seed: u32) -> Self {
        use Biome::*;
        let h = seed.wrapping_mul(0x9e37_79b1) ^ 0x5bd1_e995;
        let pick = |list: &[Voxel], k: u32| list[((h >> (k * 4)) as usize) % list.len()];
        let (roofs, planks): (&[Voxel], &[Voxel]) = match self.biome.parent() {
            Desert | Savanna => (&[mat::THATCH, mat::THATCH, mat::PLANK_PALE], &[mat::PLANK_PALE, mat::PLANK]),
            Alpine | Tundra | Ocean | Taiga => (&[mat::SLATE, mat::SLATE, mat::PLANK_DARK], &[mat::PLANK_DARK, mat::PLANK]),
            OldGrowth | Rainforest => (&[mat::THATCH, mat::THATCH, mat::PLANK_RED], &[mat::PLANK_RED, mat::PLANK_PALE]),
            Heath | Grassland => (&[mat::THATCH, mat::THATCH, mat::SLATE], &[mat::PLANK, mat::PLANK_PALE]),
            // Deciduous and Woodland; every newer biome has one of the parents above.
            _ => (&[mat::ROOF_TILE, mat::ROOF_TILE, mat::THATCH, mat::SLATE], &[mat::PLANK, mat::PLANK_PALE]),
        };
        self.roof = pick(roofs, 0);
        self.plank = pick(planks, 1);
        self.accent = pick(&[mat::CLOTH_RED, mat::CLOTH_BLUE, mat::CLOTH_PALE, self.accent], 2);
        // Where the forest has pale bark the timber is pale too.
        if matches!(self.biome, Deciduous | Woodland | Heath | Grassland) && (h >> 12) % 4 == 0 {
            self.wood = mat::BARK_BIRCH;
        }
        // Some places are built mostly of the darker rock, never in the desert.
        if !matches!(self.biome, Desert | Savanna) && (h >> 16) % 5 == 0 {
            std::mem::swap(&mut self.stone, &mut self.stone_dark);
        }
        self
    }

    /// A block of weathered stonework.
    pub fn block(&self, rng: &mut Rng) -> Voxel {
        let r = rng.unit();
        if r < self.moss * 0.6 {
            mat::MOSSY_STONE
        } else if r < 0.3 + self.moss * 0.4 {
            self.stone_dark
        } else {
            self.stone
        }
    }
}

fn v(x: i32, y: i32, z: i32) -> IVec3 {
    IVec3::new(x, y, z)
}

/// Fills an inclusive box, asking `f` for each voxel.
fn cuboid(s: &mut Shape, lo: IVec3, hi: IVec3, mut f: impl FnMut(IVec3) -> Voxel) {
    for y in lo.y..=hi.y {
        for z in lo.z..=hi.z {
            for x in lo.x..=hi.x {
                let p = v(x, y, z);
                s.set(p, f(p));
            }
        }
    }
}

/// A foundation running down into the ground, so nothing floats on uneven land.
fn footing(s: &mut Shape, lo: (i32, i32), hi: (i32, i32), mut f: impl FnMut() -> Voxel) {
    cuboid(s, v(lo.0, -4, lo.1), v(hi.0, -1, hi.1), |_| f());
}

/// A lamp post at block (x, z), in a style that varies by landmark, built in 0.4 m voxels.
fn lamp_post(s: &mut Shape, rng: &mut Rng, p: &Palette, x: i32, z: i32, height: i32) {
    let style = parts::LampStyle::pick(rng.next_u32());
    let b = crate::scale::BLOCK;
    parts::paint_lantern(s, style, p, v(x * b, 0, z * b), height * b, rng.int(0, 3) as u32);
}

fn shrine(s: &mut Shape, rng: &mut Rng, p: &Palette) {
    footing(s, (-2, -2), (2, 2), || p.stone_dark);
    match rng.int(0, 2) {
        // A stone niche: two posts carrying a slab, with a lamp on the altar between.
        0 => {
            let h = rng.int(3, 4);
            cuboid(s, v(-2, 0, -2), v(2, 0, 2), |_| p.stone);
            for x in [-2, 2] {
                cuboid(s, v(x, 1, -1), v(x, h, 1), |_| p.stone_dark);
            }
            cuboid(s, v(-3, h + 1, -2), v(3, h + 1, 2), |_| p.roof);
            // The roof's upper layer sits inside its edge, leaving a one-voxel lip all round.
            let bk = crate::scale::BLOCK;
            let top = (h + 1) * bk + bk - 1;
            for x in -3 * bk..=3 * bk + bk - 1 {
                for z in -2 * bk..=2 * bk + bk - 1 {
                    if x == -3 * bk || x == 3 * bk + bk - 1 || z == -2 * bk || z == 2 * bk + bk - 1 {
                        s.paint(v(x, top, z), AIR);
                    }
                }
            }
            cuboid(s, v(-1, 1, 0), v(1, 1, 0), |_| p.stone);
            s.set(v(0, 2, 0), mat::LANTERN);
        }
        // A timber gate in the accent colour before a small altar.
        1 => {
            let h = rng.int(5, 7);
            for x in [-3, 3] {
                cuboid(s, v(x, -3, -2), v(x, h, -2), |_| p.accent);
            }
            cuboid(s, v(-4, h + 1, -2), v(4, h + 1, -2), |_| p.wood);
            cuboid(s, v(-3, h - 1, -2), v(3, h - 1, -2), |_| p.accent);
            cuboid(s, v(-1, 0, 1), v(1, 1, 2), |_| p.stone);
            s.set(v(0, 2, 2), mat::LANTERN);
        }
        // A tall stone lantern, tiered.
        _ => {
            cuboid(s, v(-1, 0, -1), v(1, 0, 1), |_| p.stone_dark);
            cuboid(s, v(0, 1, 0), v(0, 3, 0), |_| p.stone);
            cuboid(s, v(-1, 4, -1), v(1, 4, 1), |_| p.stone);
            s.set(v(0, 5, 0), mat::LANTERN);
            cuboid(s, v(-2, 6, -2), v(2, 6, 2), |_| p.roof);
            cuboid(s, v(-1, 7, -1), v(1, 7, 1), |_| p.roof);
            s.set(v(0, 8, 0), p.stone);
        }
    }
    // Offerings at the foot.
    for (x, z) in [(-3, 3), (3, 3), (-3, -3)] {
        if rng.unit() < 0.6 {
            s.set(v(x, 0, z), p.block(rng));
        }
    }
}

/// How high the treads of a spiral stair climb per step, and how many fill a turn.
const STAIR_STEPS_PER_TURN: i32 = 16;

fn stair_angle(step: i32) -> f32 {
    0.45 + step as f32 * TAU / STAIR_STEPS_PER_TURN as f32
}

fn lookout(s: &mut Shape, rng: &mut Rng, p: &Palette) {
    const R: f32 = 5.4;
    let height = rng.int(14, 26);
    let timber = rng.unit() < 0.35;
    let wall = |rng: &mut Rng| if timber { p.plank } else { p.block(rng) };
    let (ri, r_out) = (R.ceil() as i32 + 1, R);

    footing(s, (-5, -5), (5, 5), || p.stone_dark);
    for y in 0..=height + 3 {
        for z in -ri..=ri {
            for x in -ri..=ri {
                let d = ((x * x + z * z) as f32).sqrt();
                if d > r_out {
                    continue;
                }
                let at = v(x, y, z);
                let shell = d > r_out - 1.3;
                if y <= height {
                    if shell {
                        // Timber towers are open frames above the lower storey.
                        let open = timber && y > 3 && (x + z).rem_euclid(3) != 0 && y % 4 != 0;
                        s.set(at, if open { AIR } else { wall(rng) });
                    } else {
                        s.set(at, AIR);
                    }
                }
            }
        }
    }
    // Central newel the stair winds around.
    cuboid(s, v(0, 0, 0), v(0, height, 0), |_| p.stone_dark);
    // A door facing +X.
    cuboid(s, v(4, 0, -1), v(6, 2, 1), |_| AIR);
    // Treads, each one voxel above the last, two wide.
    for step in 0..height {
        let a = stair_angle(step);
        for r in [1.8f32, 2.6, 3.4] {
            let (x, z) = ((a.cos() * r).round() as i32, (a.sin() * r).round() as i32);
            if (x, z) != (0, 0) {
                s.set(v(x, step, z), p.plank);
            }
        }
    }
    // The top floor, open above the last few treads, with a parapet round it.
    for z in -ri..=ri {
        for x in -ri..=ri {
            let d = ((x * x + z * z) as f32).sqrt();
            if d > r_out {
                continue;
            }
            let angle = (z as f32).atan2(x as f32).rem_euclid(TAU);
            let last = stair_angle(height - 1).rem_euclid(TAU);
            let behind = (last - angle).rem_euclid(TAU);
            let stairwell = d < r_out - 1.3 && d > 1.0 && behind < TAU * 5.0 / STAIR_STEPS_PER_TURN as f32 + 0.3;
            if !stairwell {
                s.set(v(x, height, z), p.plank);
            }
            if d > r_out - 1.3 {
                s.set(v(x, height + 1, z), wall(rng));
                if (x * 3 + z * 7).rem_euclid(4) < 2 {
                    s.set(v(x, height + 2, z), wall(rng));
                }
            }
        }
    }
    s.set(v(0, height + 1, 0), mat::LANTERN);
    s.set(v(0, height + 2, 0), p.accent);
    // Clear the way up: at least three voxels of air over every tread.
    for step in 0..height {
        let a = stair_angle(step);
        for r in [1.8f32, 2.6, 3.4] {
            let (x, z) = ((a.cos() * r).round() as i32, (a.sin() * r).round() as i32);
            for dy in 1..=3 {
                if (x, z) != (0, 0) && step + dy <= height {
                    let above = v(x, step + dy, z);
                    let tread = (0..height).any(|t| {
                        t == step + dy && [1.8f32, 2.6, 3.4].iter().any(|r| {
                            let a = stair_angle(t);
                            ((a.cos() * r).round() as i32, (a.sin() * r).round() as i32) == (x, z)
                        })
                    });
                    if !tread {
                        s.set(above, AIR);
                    }
                }
            }
        }
    }
}

fn camp(s: &mut Shape, rng: &mut Rng, p: &Palette) {
    // Fire ring around a bed of embers.
    for (x, z) in [(-2, -1), (-2, 0), (-2, 1), (2, -1), (2, 0), (2, 1), (-1, -2), (0, -2), (1, -2), (-1, 2), (0, 2), (1, 2)] {
        s.set(v(x, 0, z), p.block(rng));
    }
    cuboid(s, v(-1, -1, -1), v(1, -1, 1), |_| mat::CHARCOAL);
    s.set(v(0, 0, 0), mat::EMBER);
    if rng.unit() < 0.6 {
        s.set(v(1, 0, 0), mat::EMBER);
        s.set(v(0, 0, -1), mat::CHARCOAL);
    }
    // Log benches.
    for (x0, z0, along_x) in [(-3, 4, true), (5, -2, false)] {
        for k in 0..rng.int(3, 5) {
            s.set(if along_x { v(x0 + k, 0, z0) } else { v(x0, 0, z0 + k) }, p.wood);
        }
    }
    // A tent: an A-frame of cloth, open at the ends.
    let cloth = if rng.unit() < 0.5 { p.accent } else { mat::CLOTH_PALE };
    let (tx, tz, len) = (-6, -5, rng.int(4, 6));
    for k in 0..len {
        let z = tz + k;
        for y in 0..=2 {
            let w = 2 - y;
            s.set(v(tx - w, y, z), cloth);
            s.set(v(tx + w, y, z), cloth);
        }
    }
    let lamp_height = rng.int(2, 3);
    lamp_post(s, rng, p, 5, 5, lamp_height);
    if rng.unit() < 0.5 {
        cuboid(s, v(4, 0, 2), v(4, 1, 3), |_| p.plank);
    }
}

fn ruins(s: &mut Shape, rng: &mut Rng, p: &Palette) {
    let broken = |rng: &mut Rng, h: i32| ((h as f32) * rng.range(0.25, 1.1)).round() as i32;
    match rng.int(0, 2) {
        // The shell of a keep with one fallen side.
        0 => {
            let (w, d, h) = (rng.int(6, 9), rng.int(5, 8), rng.int(5, 8));
            footing(s, (-w, -d), (w, d), || p.stone_dark);
            let wall = |s: &mut Shape, rng: &mut Rng, x: i32, z: i32| {
                let top = broken(rng, h);
                cuboid(s, v(x, 0, z), v(x, top, z), |_| p.stone);
                for y in 0..=top {
                    s.set(v(x, y, z), p.block(rng));
                }
            };
            for x in -w..=w {
                wall(s, rng, x, -d);
                if rng.unit() < 0.45 {
                    wall(s, rng, x, d);
                }
            }
            for z in -d..=d {
                wall(s, rng, -w, z);
                wall(s, rng, w, z);
            }
            cuboid(s, v(-1, 0, -d), v(1, 3, -d), |_| AIR);
        }
        // A colonnade with some columns down.
        1 => {
            let n = rng.int(3, 5);
            for row in [-4, 4] {
                for k in -n..=n {
                    let x = k * 3;
                    let top = broken(rng, 8);
                    cuboid(s, v(x, -3, row), v(x, top, row), |_| p.stone);
                    for y in 0..=top {
                        s.set(v(x, y, row), p.block(rng));
                    }
                    if top >= 8 && k < n {
                        cuboid(s, v(x, 9, row), v(x + 3, 9, row), |_| p.stone_dark);
                    } else if top < 3 {
                        for j in 1..=4 {
                            s.set(v(x, 0, row + j * row.signum()), p.block(rng));
                        }
                    }
                }
            }
        }
        // A ring of walling around a sunken floor.
        _ => {
            let r = rng.range(7.0, 10.0);
            let ri = r.ceil() as i32 + 1;
            for z in -ri..=ri {
                for x in -ri..=ri {
                    let d = ((x * x + z * z) as f32).sqrt();
                    if (d - r).abs() < 0.8 {
                        let top = broken(rng, 5);
                        for y in -2..=top {
                            s.set(v(x, y, z), p.block(rng));
                        }
                    } else if d < r - 0.8 {
                        s.set(v(x, -1, z), if rng.unit() < 0.5 { p.stone_dark } else { p.stone });
                    }
                }
            }
        }
    }
    // Scattered blocks.
    for _ in 0..rng.int(5, 12) {
        s.fill(v(rng.int(-11, 11), 0, rng.int(-11, 11)), p.block(rng));
    }
}

fn standing_stone(s: &mut Shape, rng: &mut Rng, p: &Palette, x: i32, z: i32, height: i32) {
    let (w, d) = (rng.int(0, 1), rng.int(0, 1));
    for y in -3..=height {
        let taper = (y > height * 2 / 3) as i32;
        for dz in 0..=(d - taper).max(0) {
            for dx in 0..=(w - taper).max(0) {
                s.set(v(x + dx, y, z + dz), p.block(rng));
            }
        }
    }
}

fn stones(s: &mut Shape, rng: &mut Rng, p: &Palette) {
    match rng.int(0, 2) {
        // A ring.
        0 => {
            let (n, r) = (rng.int(7, 12), rng.range(7.5, 11.0));
            for k in 0..n {
                let a = k as f32 / n as f32 * TAU + rng.range(-0.08, 0.08);
                let h = rng.int(3, 7);
                standing_stone(s, rng, p, (a.cos() * r) as i32, (a.sin() * r) as i32, h);
            }
            s.set(v(0, 0, 0), p.stone_dark);
        }
        // An alignment dwindling away.
        1 => {
            let n = rng.int(5, 8);
            for k in 0..n {
                let z = rng.int(-1, 1);
                standing_stone(s, rng, p, -10 + k * 3, z, 8 - k);
            }
        }
        // A dolmen: uprights under a capstone.
        _ => {
            let h = rng.int(3, 5);
            for (x, z) in [(-3, -2), (3, -2), (-3, 2), (3, 2)] {
                standing_stone(s, rng, p, x, z, h);
            }
            cuboid(s, v(-4, h + 1, -3), v(4, h + 2, 3), |_| p.stone_dark);
        }
    }
}

fn cairn(s: &mut Shape, rng: &mut Rng, p: &Palette) {
    let h = rng.int(4, 8);
    footing(s, (-2, -2), (2, 2), || p.stone_dark);
    for y in 0..h {
        let r = (2.8 * (1.0 - y as f32 / h as f32)).ceil() as i32;
        for z in -r..=r {
            for x in -r..=r {
                if x * x + z * z <= r * r + 1 && rng.unit() < 0.9 {
                    s.set(v(x, y, z), p.block(rng));
                }
            }
        }
        s.set(v(0, y, 0), p.block(rng));
    }
    if rng.unit() < 0.6 {
        // A prayer flag on a pole.
        cuboid(s, v(0, h, 0), v(0, h + 4, 0), |_| p.wood);
        for x in 1..=3 {
            s.set(v(x, h + 4, 0), p.accent);
            s.set(v(x, h + 3, 0), if x == 3 { AIR } else { p.accent });
        }
    }
}

fn arch(s: &mut Shape, rng: &mut Rng, p: &Palette) {
    let (r, thick) = (rng.range(4.5, 7.0), rng.int(1, 2));
    let ri = r.ceil() as i32 + 3;
    let leg = rng.int(1, 3);
    for z in 0..thick {
        for x in -ri..=ri {
            for y in -3..=ri + leg {
                let lift = (y - leg).max(0) as f32;
                let d = ((x * x) as f32 + lift * lift).sqrt();
                if d >= r && d <= r + 2.0 {
                    s.set(v(x, y, z), p.block(rng));
                }
            }
        }
    }
    if rng.unit() < 0.5 {
        // A lamp hung at the crown.
        let top = leg + r.floor() as i32 - 1;
        s.set(v(0, top, 0), p.wood);
        s.set(v(0, top - 1, 0), mat::LANTERN);
    }
}

fn hut(s: &mut Shape, rng: &mut Rng, p: &Palette) {
    let (w, d) = (rng.int(3, 4), rng.int(4, 5));
    let (h, stone_walls) = (rng.int(3, 4), rng.unit() < 0.4);
    footing(s, (-w, -d), (w, d), || p.stone_dark);
    let wall = |rng: &mut Rng| if stone_walls { p.block(rng) } else { p.plank };
    cuboid(s, v(-w, -1, -d), v(w, -1, d), |_| p.plank);
    for y in 0..h {
        for z in -d..=d {
            for x in -w..=w {
                let edge = x.abs() == w || z.abs() == d;
                let corner = x.abs() == w && z.abs() == d;
                let at = v(x, y, z);
                if corner {
                    s.set(at, p.wood);
                } else if edge {
                    s.set(at, wall(rng));
                } else {
                    s.set(at, AIR);
                }
            }
        }
    }
    // Door on the +Z end, and a lit window each side.
    cuboid(s, v(0, 0, d), v(0, 2, d), |_| AIR);
    for x in [-w, w] {
        s.set(v(x, 1, 0), mat::LANTERN);
    }
    // A gabled roof built in voxels, not blocks: a 45 degree pitch, two voxels thick, with
    // an overhang at the eaves and a timber ridge cap. The walls are blocks 2 voxels wide
    // and centred on x = 0.5, so the roof is symmetric about that line.
    let b = crate::scale::BLOCK;
    let eave = h * b;
    let half = w * b + 1 + 2;
    let height_at = |x: i32| eave + half - ((x as f32 - 0.5).abs().ceil() as i32 - 0).max(0) - 1;
    for x in -half..=half + 1 {
        let top = height_at(x);
        for z in -(d + 1) * b..=(d + 1) * b + b - 1 {
            s.paint(v(x, top, z), p.roof);
            s.paint(v(x, top - 1, z), p.roof);
        }
    }
    for z in -(d + 1) * b..=(d + 1) * b + b - 1 {
        for x in [0, 1] {
            s.paint(v(x, height_at(x) + 1, z), p.wood);
        }
    }
    // Gable ends: the wall carried up under the roof.
    for x in -(w * b)..=w * b + 1 {
        for y in eave..height_at(x) - 1 {
            for z in [-d * b, -d * b + 1, d * b, d * b + 1] {
                s.paint(v(x, y, z), if stone_walls { p.stone } else { p.plank });
            }
        }
    }
    // Chimney.
    let (cx, cz) = (w - 1, -d + 1);
    cuboid(s, v(cx, 0, cz), v(cx, h + w + 2, cz), |_| p.stone_dark);
    lamp_post(s, rng, p, 2, d + 3, 2);
}

/// A pier running out along +X from the shore at the origin, `length` long.
fn landing(s: &mut Shape, rng: &mut Rng, p: &Palette, length: i32) {
    for x in 0..length {
        cuboid(s, v(x, 0, -1), v(x, 0, 1), |_| if x % 5 == 4 { p.wood } else { p.plank });
        if x % 4 == 1 {
            for z in [-2, 2] {
                cuboid(s, v(x, -9, z), v(x, 1, z), |_| p.wood);
            }
        }
    }
    cuboid(s, v(-1, -3, -1), v(-1, -1, 1), |_| p.stone_dark);
    lamp_post(s, rng, p, length - 1, 2, 3);
    if rng.unit() < 0.6 {
        cuboid(s, v(length - 3, 1, -1), v(length - 2, 1, -1), |_| p.plank);
    }
}

/// Builds a landmark. `length` only matters for landings: the length of the pier.
pub fn build(kind: Landmark, palette: &Palette, seed: u32, length: i32) -> Vec<(IVec3, Voxel)> {
    let palette = &palette.varied(seed);
    let mut rng = Rng::new(seed ^ (kind as u32).wrapping_mul(0x9e37_79b9));
    let mut s = Shape::new();
    match kind {
        Landmark::Shrine => shrine(&mut s, &mut rng, palette),
        Landmark::Lookout => lookout(&mut s, &mut rng, palette),
        Landmark::Camp => camp(&mut s, &mut rng, palette),
        Landmark::Ruins => ruins(&mut s, &mut rng, palette),
        Landmark::Stones => stones(&mut s, &mut rng, palette),
        Landmark::Cairn => cairn(&mut s, &mut rng, palette),
        Landmark::Arch => arch(&mut s, &mut rng, palette),
        Landmark::Hut => hut(&mut s, &mut rng, palette),
        Landmark::Landing => landing(&mut s, &mut rng, palette, length),
    }
    // Break up the hard block outline of the stonework.
    let chip = match kind {
        Landmark::Ruins | Landmark::Stones => 0.22,
        Landmark::Cairn => 0.3,
        Landmark::Arch | Landmark::Shrine => 0.16,
        Landmark::Lookout | Landmark::Hut => 0.08,
        _ => 0.0,
    };
    if chip > 0.0 {
        parts::weather_stone(&mut s, &mut rng, palette, chip);
    }
    s.into_voxels()
}

/// A lamp on a post beside a trail, in the style `hash` picks.
pub fn trail_lantern(palette: &Palette, height: i32, hash: u32) -> Vec<(IVec3, Voxel)> {
    let palette = palette.varied(hash);
    parts::lantern(parts::LampStyle::pick(hash >> 3), &palette, height * crate::scale::BLOCK, hash >> 7 & 3)
}

/// A signpost: one arm per direction, each direction in radians.
pub fn signpost(palette: &Palette, directions: &[f32]) -> Vec<(IVec3, Voxel)> {
    let mut s = Shape::new();
    cuboid(&mut s, v(0, -3, 0), v(0, 3, 0), |_| palette.wood);
    for (k, &a) in directions.iter().enumerate().take(3) {
        let y = 3 - k as i32;
        for r in 1..=2 {
            let (x, z) = (a.cos() * r as f32, a.sin() * r as f32);
            s.set(v(x.round() as i32, y, z.round() as i32), palette.plank);
        }
    }
    s.into_voxels()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn landmarks_differ_by_seed_and_are_repeatable() {
        let palette = Palette::of(Biome::Deciduous);
        for kind in Landmark::ALL {
            let a = build(kind, &palette, 7, 12);
            assert!(!a.is_empty(), "{kind:?} is empty");
            assert_eq!(a, build(kind, &palette, 7, 12), "{kind:?} is not deterministic");
            let differs = (8..40).any(|seed| build(kind, &palette, seed, 12) != a);
            assert!(differs, "{kind:?} never varies");
        }
    }

    #[test]
    fn every_lantern_style_is_lit_deterministic_and_stands_on_its_footing() {
        for biome in [Biome::Desert, Biome::Deciduous, Biome::Taiga, Biome::OldGrowth] {
            let p = Palette::of(biome);
            for style in parts::LampStyle::ALL {
                for turns in 0..4 {
                    let a = parts::lantern(style, &p, 8, turns);
                    assert_eq!(a, parts::lantern(style, &p, 8, turns));
                    let lights = a.iter().filter(|&&(_, v)| v == mat::LANTERN).count();
                    assert!(lights == 1 || (style == parts::LampStyle::Pair && lights == 2), "{style:?}: {lights} lights");
                    // Rooted below the ground, and in one piece: every voxel touches another.
                    assert!(a.iter().any(|&(q, _)| q == v(0, -3, 0)));
                    let set: std::collections::HashSet<_> = a.iter().map(|&(q, _)| q).collect();
                    for &(q, _) in &a {
                        let touches = [IVec3::X, IVec3::NEG_X, IVec3::Y, IVec3::NEG_Y, IVec3::Z, IVec3::NEG_Z]
                            .iter()
                            .any(|d| set.contains(&(q + *d)));
                        assert!(touches, "{style:?} turns {turns}: {q:?} floats");
                    }
                }
            }
        }
        // Trail lanterns differ in style along a trail.
        let p = Palette::of(Biome::Deciduous);
        let styles: std::collections::HashSet<_> = (0..64u32).map(|h| trail_lantern(&p, 3, h.wrapping_mul(2654435761))).collect::<Vec<_>>().into_iter().map(|l| l.len()).collect();
        assert!(styles.len() >= 3, "lanterns all look alike");
    }

    #[test]
    fn palettes_vary_by_place_but_stay_in_the_biome() {
        let roofs: std::collections::HashSet<_> = (0..40).map(|s| Palette::of(Biome::Deciduous).varied(s).roof).collect();
        assert!(roofs.len() >= 2);
        for seed in 0..40 {
            let d = Palette::of(Biome::Desert).varied(seed);
            assert_eq!((d.stone, d.stone_dark), (mat::SANDSTONE, mat::SANDSTONE_DARK));
        }
    }

    #[test]
    fn materials_follow_the_biome() {
        let uses = |biome, voxel| {
            (0..24).any(|seed| {
                build(Landmark::Ruins, &Palette::of(biome), seed, 0)
                    .iter()
                    .any(|&(_, v)| v == voxel)
            })
        };
        assert!(uses(Biome::Desert, mat::SANDSTONE));
        assert!(!uses(Biome::Desert, mat::STONE));
        assert!(uses(Biome::OldGrowth, mat::MOSSY_STONE));
    }

    /// Follows the stair from the door to the top floor, as a player would.
    #[test]
    fn lookout_stairs_can_be_climbed() {
        let palette = Palette::of(Biome::Grassland);
        for seed in 0..12 {
            // Blocks, addressed by one corner voxel each.
            let unit = IVec3::splat(crate::scale::BLOCK);
            let model: HashMap<IVec3, Voxel> = build(Landmark::Lookout, &palette, seed, 0)
                .into_iter()
                .filter(|(p, _)| p.rem_euclid(unit) == IVec3::ZERO)
                .map(|(p, v)| (p.div_euclid(unit), v))
                .collect();
            let solid = |p: IVec3| model.get(&p).is_some_and(|&v| v != AIR);
            let height = (0..40).rev().find(|&y| model.get(&v(0, y, 0)) == Some(&mat::LANTERN)).unwrap() - 1;
            let tread = |step: i32| {
                let a = stair_angle(step);
                v((a.cos() * 2.6).round() as i32, step, (a.sin() * 2.6).round() as i32)
            };
            for step in 0..height {
                let t = tread(step);
                assert!(solid(t), "seed {seed}: tread {step} missing at {t:?}");
                for dy in 1..=3 {
                    if step + dy < height {
                        let above = t + IVec3::Y * dy;
                        let is_next_tread = (step + 1..height).any(|s| tread(s) == above);
                        assert!(is_next_tread || !solid(above), "seed {seed}: no headroom above tread {step} at {above:?}");
                    }
                }
                if step > 0 {
                    let (a, b) = (tread(step - 1), t);
                    let gap = (a.x - b.x).abs().max((a.z - b.z).abs());
                    assert!(gap <= 2, "seed {seed}: treads {} and {step} too far apart", step - 1);
                }
            }
        }
    }
}
