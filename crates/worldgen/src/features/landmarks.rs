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
    parts, props,
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
    cuboid(s, v(lo.0, -8, lo.1), v(hi.0, -1, hi.1), |_| f());
}

/// A filled disc of radius `r` at height `y`.
fn disc(s: &mut Shape, r: f32, y: i32, mut f: impl FnMut(IVec3) -> Voxel) {
    let ri = r.ceil() as i32;
    for z in -ri..=ri {
        for x in -ri..=ri {
            if ((x * x + z * z) as f32).sqrt() <= r {
                let p = v(x, y, z);
                s.set(p, f(p));
            }
        }
    }
}

/// A lamp post at (x, z), in a style that varies by landmark, built in 0.4 m voxels.
fn lamp_post(s: &mut Shape, rng: &mut Rng, p: &Palette, x: i32, z: i32, height: i32) {
    let style = parts::LampStyle::pick(rng.next_u32());
    parts::paint_lantern(s, style, p, v(x, 0, z), height, rng.int(0, 3) as u32);
}

/// A loose stone or two lying about the site.
fn scatter(s: &mut Shape, rng: &mut Rng, p: &Palette, count: i32, reach: i32) {
    for _ in 0..count {
        let (x, z) = (rng.int(-reach, reach), rng.int(-reach, reach));
        let at = v(x, 0, z);
        if s.get(at).is_none() {
            let size = rng.int(1, 2);
            cuboid(s, at, at + v(size - 1, rng.int(0, 1), size - 1), |_| p.block(rng));
        }
    }
}

fn shrine(s: &mut Shape, rng: &mut Rng, p: &Palette) {
    footing(s, (-7, -7), (7, 7), || p.stone_dark);
    // A paved platform with a step up to the shrine itself.
    cuboid(s, v(-7, 0, -7), v(7, 0, 7), |_| p.stone);
    for x in -6..=6 {
        for z in -6..=6 {
            if (x + z) % 4 == 0 {
                s.set(v(x, 0, z), p.stone_dark);
            }
        }
    }
    match rng.int(0, 2) {
        // A stone niche: four pillars with capitals under a gabled roof, an altar between.
        0 => {
            cuboid(s, v(-5, 1, -5), v(5, 1, 5), |_| p.stone_dark);
            let h = rng.int(8, 10);
            for (cx, cz) in [(-5, -4), (4, -4), (-5, 3), (4, 3)] {
                cuboid(s, v(cx - 1, 2, cz - 1), v(cx + 2, 2, cz + 2), |_| p.stone);
                cuboid(s, v(cx, 3, cz), v(cx + 1, h, cz + 1), |_| p.stone_dark);
                cuboid(s, v(cx - 1, h + 1, cz - 1), v(cx + 2, h + 1, cz + 2), |_| p.stone);
            }
            // The roof: a 45 degree pitch two voxels thick, with a ridge cap.
            let eave = h + 2;
            for z in -7i32..=7 {
                let y = eave + (7 - z.abs()) / 1;
                for x in -8..=8 {
                    s.set(v(x, y, z), p.roof);
                    s.set(v(x, y - 1, z), p.roof);
                }
            }
            for x in -8..=8 {
                s.set(v(x, eave + 8, 0), p.wood);
            }
            // The altar, with a lamp on it.
            cuboid(s, v(-3, 2, -1), v(3, 3, 1), |_| p.stone);
            cuboid(s, v(-1, 4, 0), v(1, 4, 0), |_| p.stone_dark);
            s.set(v(0, 5, 0), mat::LANTERN);
        }
        // A timber gate in the accent colour before a small altar: two tall pillars, a
        // lower tie beam, and a top beam that curves up at its ends.
        1 => {
            let h = rng.int(13, 16);
            for x in [-6, 5] {
                cuboid(s, v(x - 1, 1, -2), v(x + 2, 1, 1), |_| p.stone_dark);
                cuboid(s, v(x, 2, -1), v(x + 1, h, 0), |_| p.accent);
            }
            cuboid(s, v(-5, h - 3, -1), v(5, h - 3, 0), |_| p.accent);
            for x in -9i32..=9 {
                let up = (x.abs() >= 8) as i32 + (x.abs() >= 9) as i32;
                cuboid(s, v(x, h + 1 + up, -1), v(x, h + 2 + up, 0), |_| p.wood);
            }
            cuboid(s, v(-7, h + 3, -1), v(7, h + 3, 0), |_| p.stone_dark);
            // The way through the gate to the altar.
            s.keep_clear(v(-3, 0, -10), v(3, 8, 2));
            // The altar beyond it.
            cuboid(s, v(-4, 1, 3), v(4, 1, 6), |_| p.stone);
            cuboid(s, v(-2, 2, 4), v(2, 3, 5), |_| p.stone_dark);
            s.set(v(0, 4, 5), mat::LANTERN);
        }
        // A tall tiered stone lantern on a stepped base.
        _ => {
            cuboid(s, v(-4, 1, -4), v(4, 1, 4), |_| p.stone_dark);
            cuboid(s, v(-2, 2, -2), v(2, 2, 2), |_| p.stone);
            parts::paint_lantern(s, parts::LampStyle::Stone, p, v(0, 3, 0), 8, 0);
        }
    }
    // Offerings at the foot, and stones round the edge.
    for (x, z) in [(-6, 6), (6, 6), (-6, -6)] {
        if rng.unit() < 0.6 {
            s.set(v(x, 1, z), p.block(rng));
        }
    }
    scatter(s, rng, p, 6, 9);
}

/// How many treads make one turn of the stair, and the radii it is laid across.
const STAIR_STEPS_PER_TURN: i32 = 28;
const STAIR_RADII: [f32; 6] = [2.9, 3.7, 4.5, 5.3, 6.1, 6.9];

fn stair_angle(step: i32) -> f32 {
    0.45 + step as f32 * TAU / STAIR_STEPS_PER_TURN as f32
}

/// The cells one tread covers.
fn tread_cells(step: i32) -> Vec<(i32, i32)> {
    let a = stair_angle(step);
    let mut cells: Vec<(i32, i32)> = STAIR_RADII
        .iter()
        .flat_map(|&r| {
            let (x, z) = (a.cos() * r, a.sin() * r);
            [(x.round() as i32, z.round() as i32), ((x + a.sin() * 0.5).round() as i32, (z - a.cos() * 0.5).round() as i32)]
        })
        .collect();
    cells.sort_unstable();
    cells.dedup();
    cells
}

fn lookout(s: &mut Shape, rng: &mut Rng, p: &Palette) {
    const R: f32 = 10.8;
    let height = rng.int(28, 50);
    let timber = rng.unit() < 0.35;
    let wall = |rng: &mut Rng| if timber { p.plank } else { p.block(rng) };
    let ri = R.ceil() as i32 + 1;
    let thick = 2.6;

    footing(s, (-11, -11), (11, 11), || p.stone_dark);
    // A stepped plinth the tower stands on.
    disc(s, R + 1.8, 0, |_| p.stone_dark);
    for y in 0..=height + 6 {
        for z in -ri..=ri {
            for x in -ri..=ri {
                let d = ((x * x + z * z) as f32).sqrt();
                if d > R {
                    continue;
                }
                let at = v(x, y, z);
                let shell = d > R - thick;
                if y <= height {
                    if shell {
                        // Timber towers are open frames above the lower storey; stone ones get
                        // string courses and slit windows.
                        let open = timber && y > 6 && (x + z).rem_euclid(5) != 0 && y % 7 != 0;
                        let course = !timber && y % 9 == 8;
                        let a = (z as f32).atan2(x as f32).rem_euclid(TAU);
                        let slit = !timber && y % 9 >= 3 && y % 9 <= 5 && y > 8 && (a * 4.0 / TAU * 1.0).fract().abs() < 0.04 + 0.0;
                        s.set(at, if open || slit { AIR } else if course { p.stone_dark } else { wall(rng) });
                    } else {
                        s.set(at, AIR);
                    }
                }
            }
        }
    }
    // The newel the stair winds around.
    for y in 0..=height {
        cuboid(s, v(-1, y, -1), v(1, y, 1), |_| p.stone_dark);
    }
    // A door facing +X with a frame, and a step up to it.
    cuboid(s, v(7, 0, -2), v(12, 6, 2), |_| AIR);
    cuboid(s, v(9, 7, -3), v(11, 7, 3), |_| p.wood);
    for z in [-3, 3] {
        cuboid(s, v(9, 0, z), v(11, 6, z), |_| p.wood);
    }
    cuboid(s, v(12, 0, -2), v(13, 0, 2), |_| p.stone);
    // Nothing is left inside the tower, before its door, or where the stair comes out on
    // the top floor.
    s.keep_clear(v(-9, 0, -9), v(9, height, 9));
    s.keep_clear(v(7, 0, -4), v(18, 7, 4));
    for step in height - 3..height + 4 {
        for (x, z) in tread_cells(step) {
            s.keep_clear(v(x - 1, height + 1, z - 1), v(x + 1, height + 4, z + 1));
        }
    }
    // Treads, one voxel (0.4 m) above the last: climbable by walking.
    for step in 0..height {
        for (x, z) in tread_cells(step) {
            if x.abs() > 1 || z.abs() > 1 {
                s.set(v(x, step, z), p.plank);
            }
        }
    }
    // The top floor, open above the last few treads, with a crenellated parapet.
    for z in -ri..=ri {
        for x in -ri..=ri {
            let d = ((x * x + z * z) as f32).sqrt();
            if d > R + 1.0 {
                continue;
            }
            let angle = (z as f32).atan2(x as f32).rem_euclid(TAU);
            let last = stair_angle(height - 1).rem_euclid(TAU);
            let behind = (last - angle).rem_euclid(TAU);
            let stairwell = d < R - thick && d > 2.0 && behind < TAU * 6.0 / STAIR_STEPS_PER_TURN as f32 + 0.3;
            if !stairwell {
                s.set(v(x, height, z), p.plank);
            }
            if d > R - thick {
                // The balcony lip juts out a little over the wall below.
                s.set(v(x, height + 1, z), wall(rng));
                s.set(v(x, height + 2, z), wall(rng));
                if (x * 3 + z * 5).rem_euclid(4) < 2 {
                    s.set(v(x, height + 3, z), wall(rng));
                }
            }
        }
    }
    s.set(v(0, height + 1, 0), mat::LANTERN);
    cuboid(s, v(0, height + 2, 0), v(0, height + 5, 0), |_| p.wood);
    s.set(v(0, height + 6, 0), p.accent);
    // Clear the way up: six voxels of headroom over every tread.
    let cells: Vec<Vec<(i32, i32)>> = (0..height).map(tread_cells).collect();
    for step in 0..height {
        for &(x, z) in &cells[step as usize] {
            if x.abs() <= 1 && z.abs() <= 1 {
                continue;
            }
            for dy in 1..=6 {
                if step + dy <= height && !(step + dy < height && cells[(step + dy) as usize].contains(&(x, z))) {
                    s.set(v(x, step + dy, z), AIR);
                }
            }
        }
    }
}

fn camp(s: &mut Shape, rng: &mut Rng, p: &Palette) {
    // A ring of fire stones around a bed of embers.
    for k in 0..14 {
        let a = k as f32 / 14.0 * TAU;
        let (x, z) = ((a.cos() * 3.4).round() as i32, (a.sin() * 3.4).round() as i32);
        cuboid(s, v(x, 0, z), v(x, rng.int(0, 1), z), |_| p.block(rng));
    }
    disc(s, 2.5, -1, |_| mat::CHARCOAL);
    disc(s, 1.6, 0, |_| mat::CHARCOAL);
    s.set(v(0, 0, 0), mat::EMBER);
    s.set(v(1, 0, 0), mat::EMBER);
    s.set(v(0, 0, 1), mat::EMBER);
    if rng.unit() < 0.6 {
        s.set(v(-1, 0, 0), mat::EMBER);
    }
    // A tripod over it, and a pot.
    for (x, z) in [(-2, -1), (2, -1), (0, 2)] {
        for y in 1..=5 {
            let t = y as f32 / 6.0;
            s.set(v((x as f32 * (1.0 - t)).round() as i32, y, (z as f32 * (1.0 - t)).round() as i32), p.wood);
        }
    }
    cuboid(s, v(0, 4, 0), v(0, 4, 0), |_| p.stone_dark);
    // Log benches: round-ish logs two voxels thick.
    for (x0, z0, along_x) in [(-7, 6, true), (9, -4, false)] {
        for k in 0..rng.int(7, 10) {
            let q = if along_x { v(x0 + k, 0, z0) } else { v(x0, 0, z0 + k) };
            cuboid(s, q, q + v(0, 1, 0), |_| p.wood);
            let off = if along_x { v(0, 0, 1) } else { v(1, 0, 0) };
            s.set(q + off, p.wood);
            if rng.unit() < 0.2 {
                s.set(q + v(0, 2, 0), p.wood);
            }
        }
    }
    // A tent: an A-frame of cloth on a ridge pole, closed at the back, open at the front.
    let cloth = if rng.unit() < 0.5 { p.accent } else { mat::CLOTH_PALE };
    let (tx, tz, len) = (-12, -10, rng.int(10, 13));
    for k in 0..len {
        let z = tz + k;
        for y in 0..=5 {
            let w = 5 - y;
            s.set(v(tx - w, y, z), cloth);
            s.set(v(tx + w, y, z), cloth);
            if w == 0 {
                s.set(v(tx, y, z), cloth);
            }
        }
    }
    cuboid(s, v(tx - 5, 0, tz + len - 1), v(tx + 5, 0, tz + len - 1), |_| cloth);
    // The ridge pole sticks out at both ends, with a guy stake at each corner.
    cuboid(s, v(tx, 6, tz - 1), v(tx, 6, tz + len), |_| p.wood);
    for (x, z) in [(tx - 7, tz), (tx + 7, tz), (tx - 7, tz + len - 1), (tx + 7, tz + len - 1)] {
        cuboid(s, v(x, 0, z), v(x, 1, z), |_| p.wood);
    }
    // A bedroll inside.
    cuboid(s, v(tx - 1, 0, tz + 3), v(tx + 1, 0, tz + 6), |_| mat::CLOTH_BLUE);
    // The tent's mouth and the floor inside it.
    s.keep_clear(v(tx - 5, 0, tz - 5), v(tx + 5, 6, tz + len - 2));
    lamp_post(s, rng, p, 10, 10, 9);
}

fn ruins(s: &mut Shape, rng: &mut Rng, p: &Palette) {
    let broken = |rng: &mut Rng, h: i32| ((h as f32) * rng.range(0.25, 1.1)).round() as i32;
    match rng.int(0, 2) {
        // The shell of a keep with one fallen side, windows still in its walls.
        0 => {
            let (w, d, h) = (rng.int(12, 18), rng.int(10, 16), rng.int(10, 16));
            footing(s, (-w - 1, -d - 1), (w + 1, d + 1), || p.stone_dark);
            let wall = |s: &mut Shape, rng: &mut Rng, x: i32, z: i32, dx: i32, dz: i32| {
                let top = broken(rng, h);
                for k in 0..2 {
                    for y in 0..=top {
                        // Windows, three voxels wide, every eighth column above the base.
                        let window = (x + z).rem_euclid(8) < 2 && (6..=9).contains(&y);
                        let at = v(x + dx * k, y, z + dz * k);
                        s.set(at, if window { AIR } else { p.block(rng) });
                    }
                }
            };
            for x in -w..=w {
                wall(s, rng, x, -d, 0, 1);
                if rng.unit() < 0.45 {
                    wall(s, rng, x, d - 1, 0, 1);
                }
            }
            for z in -d..=d {
                wall(s, rng, -w, z, 1, 0);
                wall(s, rng, w - 1, z, 1, 0);
            }
            cuboid(s, v(-2, 0, -d), v(2, 6, -d + 1), |_| AIR);
            s.keep_clear(v(-3, 0, -d - 5), v(3, 8, -d + 5));
            cuboid(s, v(-3, 7, -d), v(3, 7, -d + 1), |_| p.stone_dark);
            // Fallen stones inside and out.
            scatter(s, rng, p, 24, w);
        }
        // A colonnade with some columns down.
        1 => {
            let n = rng.int(3, 5);
            for row in [-8, 8] {
                for k in -n..=n {
                    let x = k * 6;
                    let top = broken(rng, 16);
                    cuboid(s, v(x - 1, -8, row - 1), v(x + 2, 0, row + 2), |_| p.stone_dark);
                    for y in 1..=top.max(1) {
                        cuboid(s, v(x, y, row), v(x + 1, y, row + 1), |_| p.block(rng));
                    }
                    if top >= 15 && k < n {
                        cuboid(s, v(x - 1, 17, row - 1), v(x + 7, 18, row + 2), |_| p.stone_dark);
                    } else if top < 6 {
                        for j in 1..=6 {
                            if rng.unit() < 0.7 {
                                s.set(v(x, 0, row + j * row.signum()), p.block(rng));
                            }
                        }
                    }
                }
            }
        }
        // A ring of walling around a sunken floor.
        _ => {
            let r = rng.range(14.0, 20.0);
            let ri = r.ceil() as i32 + 2;
            for z in -ri..=ri {
                for x in -ri..=ri {
                    let d = ((x * x + z * z) as f32).sqrt();
                    if (d - r).abs() < 1.2 {
                        let top = broken(rng, 10);
                        for y in -4..=top {
                            s.set(v(x, y, z), p.block(rng));
                        }
                    } else if d < r - 1.2 {
                        s.set(v(x, -2, z), if rng.unit() < 0.5 { p.stone_dark } else { p.stone });
                    }
                }
            }
        }
    }
    // Scattered rubble.
    let rubble = rng.int(14, 26);
    scatter(s, rng, p, rubble, 20);
}

fn standing_stone(s: &mut Shape, rng: &mut Rng, p: &Palette, x: i32, z: i32, height: i32) {
    let (w, d) = (rng.int(1, 3), rng.int(1, 2));
    for y in -6..=height {
        let taper = (y > height * 2 / 3) as i32 + (y > height - 2) as i32;
        // Each layer is a little ragged, so the stone is not a clean prism.
        let lean = if y > height / 2 { rng.int(0, 1) } else { 0 };
        for dz in 0..=(d - taper).max(0) {
            for dx in 0..=(w - taper).max(0) {
                if y > 0 && (dx, dz) != (0, 0) && rng.unit() < 0.1 {
                    continue;
                }
                s.set(v(x + dx + lean, y, z + dz), p.block(rng));
            }
        }
    }
}

fn stones(s: &mut Shape, rng: &mut Rng, p: &Palette) {
    match rng.int(0, 2) {
        // A ring.
        0 => {
            let (n, r) = (rng.int(8, 12), rng.range(14.0, 21.0));
            for k in 0..n {
                let a = k as f32 / n as f32 * TAU + rng.range(-0.08, 0.08);
                let h = rng.int(6, 13);
                standing_stone(s, rng, p, (a.cos() * r) as i32, (a.sin() * r) as i32, h);
            }
            cuboid(s, v(-1, 0, -1), v(1, 0, 1), |_| p.stone_dark);
        }
        // An alignment dwindling away.
        1 => {
            let n = rng.int(5, 8);
            for k in 0..n {
                let z = rng.int(-2, 2);
                standing_stone(s, rng, p, -20 + k * 6, z, 16 - 2 * k);
            }
        }
        // A dolmen: uprights under a capstone.
        _ => {
            let h = rng.int(7, 10);
            for (x, z) in [(-6, -4), (6, -4), (-6, 4), (6, 4)] {
                standing_stone(s, rng, p, x, z, h);
            }
            for y in 0..3 {
                cuboid(s, v(-8 + y, h + 1 + y, -6 + y), v(9 - y, h + 1 + y, 7 - y), |_| p.stone_dark);
            }
        }
    }
    scatter(s, rng, p, 10, 14);
}

fn cairn(s: &mut Shape, rng: &mut Rng, p: &Palette) {
    let h = rng.int(8, 16);
    footing(s, (-4, -4), (4, 4), || p.stone_dark);
    for y in 0..h {
        let r = 5.6 * (1.0 - y as f32 / h as f32) + 0.8;
        let ri = r.ceil() as i32;
        for z in -ri..=ri {
            for x in -ri..=ri {
                let wobble = rng.range(-0.5, 0.5);
                if ((x * x + z * z) as f32).sqrt() <= r + wobble && rng.unit() < 0.9 {
                    s.set(v(x, y, z), p.block(rng));
                }
            }
        }
        s.set(v(0, y, 0), p.block(rng));
    }
    if rng.unit() < 0.6 {
        // A prayer flag on a pole, flying in strips.
        cuboid(s, v(0, h, 0), v(0, h + 9, 0), |_| p.wood);
        for x in 1..=6 {
            s.set(v(x, h + 9, 0), p.accent);
            s.set(v(x, h + 8, 0), if x % 2 == 0 { mat::CLOTH_PALE } else { p.accent });
            s.set(v(x, h + 7, 0), if x >= 5 { AIR } else { p.accent });
        }
    }
    scatter(s, rng, p, 8, 8);
}

fn arch(s: &mut Shape, rng: &mut Rng, p: &Palette) {
    let (r, thick) = (rng.range(9.0, 14.0), rng.int(3, 5));
    let ri = r.ceil() as i32 + 5;
    let leg = rng.int(2, 6);
    let band = rng.range(3.0, 4.5);
    for z in 0..thick {
        for x in -ri..=ri {
            for y in -6..=ri + leg + 2 {
                let lift = (y - leg).max(0) as f32;
                let d = ((x * x) as f32 + lift * lift).sqrt();
                // The underside is a little ragged where the rock has weathered.
                if d >= r && d <= r + band && !(y > leg && rng.unit() < 0.05) {
                    s.set(v(x, y, z), p.block(rng));
                }
            }
        }
    }
    // A keystone at the crown, a shade darker.
    let top = leg + (r + band).floor() as i32 - 1;
    cuboid(s, v(-1, top - 3, 0), v(1, top, thick - 1), |_| p.stone_dark);
    if rng.unit() < 0.5 {
        // A lamp hung on a short chain from the crown.
        let under = leg + r.floor() as i32;
        cuboid(s, v(0, under - 2, 0), v(0, under - 1, 0), |_| p.wood);
        s.set(v(0, under - 3, 0), mat::LANTERN);
    }
}

fn hut(s: &mut Shape, rng: &mut Rng, p: &Palette) {
    let (w, d) = (rng.int(6, 8), rng.int(8, 10));
    let (h, stone_walls) = (rng.int(7, 9), rng.unit() < 0.4);
    footing(s, (-w - 1, -d - 1), (w + 1, d + 1), || p.stone_dark);
    let wall = |rng: &mut Rng| if stone_walls { p.block(rng) } else { p.plank };
    // A stone sill, a plank floor, and walls one voxel thick with corner posts.
    for z in -d..=d {
        for x in -w..=w {
            let edge = x.abs() == w || z.abs() == d;
            s.set(v(x, -1, z), p.plank);
            if edge {
                s.set(v(x, 0, z), p.stone_dark);
            }
        }
    }
    for y in 1..h {
        for z in -d..=d {
            for x in -w..=w {
                let edge = x.abs() == w || z.abs() == d;
                let corner = x.abs() >= w - 1 && z.abs() >= d - 1 && edge;
                if corner {
                    s.set(v(x, y, z), p.wood);
                } else if edge {
                    // Timber walls get a horizontal band every few boards.
                    let band = !stone_walls && (y == 4 || y == h - 1);
                    s.set(v(x, y, z), if band { p.wood } else { wall(rng) });
                } else {
                    s.set(v(x, y, z), AIR);
                }
            }
        }
    }
    // A door on the +Z end with a frame and a step, and a lit window each side.
    cuboid(s, v(-1, 1, d), v(1, 5, d), |_| AIR);
    cuboid(s, v(-2, 1, d), v(-2, 6, d), |_| p.wood);
    cuboid(s, v(2, 1, d), v(2, 6, d), |_| p.wood);
    cuboid(s, v(-2, 6, d), v(2, 6, d), |_| p.wood);
    cuboid(s, v(-2, 0, d + 1), v(2, 0, d + 2), |_| p.stone);
    s.keep_clear(v(-3, 0, d - 4), v(3, 7, d + 5));
    for x in [-w, w] {
        cuboid(s, v(x, 3, -1), v(x, 5, 1), |_| p.wood);
        s.set(v(x, 4, 0), mat::LANTERN);
    }
    // A gabled roof in voxels: a 45 degree pitch two voxels thick, with an overhang at the
    // eaves and a timber ridge cap.
    let eave = h;
    let half = w + 2;
    let height_at = |x: i32| eave + half - x.abs();
    for x in -half..=half {
        let top = height_at(x);
        for z in -d - 2..=d + 2 {
            s.set(v(x, top, z), p.roof);
            s.set(v(x, top - 1, z), p.roof);
        }
    }
    for z in -d - 2..=d + 2 {
        s.set(v(0, height_at(0) + 1, z), p.wood);
    }
    // Gable ends: the wall carried up under the roof.
    for x in -w..=w {
        for y in eave..height_at(x) - 1 {
            for z in [-d, d] {
                s.set(v(x, y, z), if stone_walls { p.stone } else { p.plank });
            }
        }
    }
    // Chimney.
    let (cx, cz) = (w - 3, -d + 2);
    cuboid(s, v(cx, 1, cz), v(cx + 1, h + w + 4, cz + 1), |_| p.stone_dark);
    cuboid(s, v(cx - 1, h + w + 5, cz - 1), v(cx + 2, h + w + 5, cz + 2), |_| p.stone);
    lamp_post(s, rng, p, 4, d + 6, 9);
    scatter(s, rng, p, 6, d + 6);
}

/// A pier running out along +X from the shore at the origin, `length` voxels long.
fn landing(s: &mut Shape, rng: &mut Rng, p: &Palette, length: i32) {
    for x in 0..length {
        // Boards across the pier, alternating in tone, with a gap now and then.
        let board = if x % 4 < 3 { if (x / 4) % 2 == 0 { p.plank } else { mat::PLANK_DARK } } else { AIR };
        for z in -3i32..=3 {
            if board != AIR || z.abs() == 3 {
                s.set(v(x, 0, z), if board != AIR { board } else { p.wood });
            }
        }
    }
    // Posts down to the bed with caps, and rails between them.
    for x in (0..length).step_by(8) {
        for z in [-4, 4] {
            cuboid(s, v(x, -18, z), v(x + 1, 4, z + 1), |_| p.wood);
            cuboid(s, v(x, 5, z), v(x + 1, 5, z + 1), |_| p.stone_dark);
        }
    }
    for x in 0..length {
        for z in [-4, 4] {
            s.set(v(x, 3, z), p.wood);
        }
    }
    // Rails open at the middle of the pier's end, where boats tie up.
    cuboid(s, v(length - 1, 1, -3), v(length, 3, 3), |_| p.wood);
    // Stone steps up from the shore.
    cuboid(s, v(-3, -3, -3), v(-1, -1, 3), |_| p.stone_dark);
    cuboid(s, v(-2, 0, -3), v(-1, 0, 3), |_| p.stone);
    lamp_post(s, rng, p, length - 3, 2, 8);
    // The boards on the -Z side of the middle, the steps up and the end where boats tie
    // up stay open; props go along the +Z side.
    s.keep_clear(v(-3, 0, -3), v(length + 1, 6, 0));
    s.keep_clear(v(length - 5, 0, -4), v(length + 1, 6, 4));
}

/// How long a site has stood and how well it is looked after, both 0 to 1.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct History {
    pub age: f32,
    pub care: f32,
}

impl History {
    /// Taken from the kind of site and its seed: ruins and standing stones are old and
    /// forgotten, camps new and kept, the rest somewhere between.
    pub fn of(kind: Landmark, seed: u32) -> Self {
        let mut rng = Rng::new(seed ^ 0x4157_0e1d ^ (kind as u32).wrapping_mul(0x85eb_ca6b));
        let ((a0, a1), (c0, c1)) = match kind {
            Landmark::Ruins => ((0.8, 1.0), (0.0, 0.15)),
            Landmark::Stones => ((0.9, 1.0), (0.0, 0.1)),
            Landmark::Cairn | Landmark::Arch => ((0.5, 0.9), (0.1, 0.5)),
            Landmark::Camp => ((0.0, 0.15), (0.8, 1.0)),
            _ => ((0.2, 0.7), (0.4, 0.95)),
        };
        History { age: rng.range(a0, a1), care: rng.range(c0, c1) }
    }

    /// How far the site has gone to ruin.
    pub fn neglect(self) -> f32 {
        self.age * (1.0 - self.care)
    }
}

/// Ages a built site by its history: rotten wood, lamps gone out, and on neglected sites
/// erosion from the top down. Only voxels with nothing above them fall, roofs before
/// wood before stone and corners last, worse in some sections of the site than others,
/// and what falls lands on the pad beside it as rubble. Lookouts are left whole, so
/// their stair and lamp stay as built; nothing here makes a material the site lacked,
/// so desert sites stay sandstone.
fn decay(s: &mut Shape, kind: Landmark, p: &Palette, seed: u32, h: History) {
    if kind == Landmark::Lookout {
        return;
    }
    let neglect = h.neglect();
    let mut rng = Rng::new(seed ^ 0xdeca_7001);
    // Rot and dead lamps.
    for q in s.snapshot() {
        let Some(v) = s.get(q) else { continue };
        let swap = match v {
            _ if v == p.wood && rng.unit() < neglect * 0.6 => mat::WOOD_DEAD,
            _ if v == p.plank && rng.unit() < neglect * 0.6 => mat::PLANK_DARK,
            mat::LANTERN | mat::EMBER if h.care < 0.3 => mat::CHARCOAL,
            _ => continue,
        };
        s.paint(q, swap);
    }
    if neglect < 0.25 {
        return;
    }
    let roof = [p.roof, mat::THATCH];
    let wood = [p.wood, p.plank, mat::WOOD_DEAD, mat::PLANK_DARK];
    let solid = |s: &Shape, q: IVec3| s.get(q).is_some_and(|v| v != AIR);
    let section = |q: IVec3| 0.3 + 1.2 * crate::noise::unit(crate::noise::hash3(seed ^ 0x5ec7, q.x.div_euclid(4), 0, q.z.div_euclid(4)));
    let rounds = (neglect * 6.0).ceil() as i32;
    for _ in 0..rounds {
        let mut fall = Vec::new();
        for q in s.snapshot() {
            if q.y < 1 || !solid(s, q) || solid(s, q + IVec3::Y) {
                continue;
            }
            let v = s.get(q).unwrap();
            let class = if roof.contains(&v) {
                1.0
            } else if wood.contains(&v) {
                0.6
            } else if v == mat::LANTERN || v == mat::CHARCOAL {
                0.0
            } else {
                0.3
            };
            // A corner is open on both an x side and a z side; it holds out longest.
            let open = |d: IVec3| !solid(s, q + d);
            let corner = (open(IVec3::X) || open(IVec3::NEG_X)) && (open(IVec3::Z) || open(IVec3::NEG_Z));
            let chance = neglect * 0.35 * class * section(q) * if corner { 0.25 } else { 1.0 };
            if rng.unit() < chance {
                fall.push((q, v));
            }
        }
        for (q, piece) in fall {
            s.paint(q, AIR);
            if rng.unit() < 0.5 {
                let at = v(q.x + rng.int(-1, 1), 0, q.z + rng.int(-1, 1));
                if !solid(s, at) {
                    s.paint(at, piece);
                }
            }
        }
    }
}

/// Builds a landmark. `length` only matters for landings: the length of the pier.
pub fn build(kind: Landmark, palette: &Palette, seed: u32, length: i32) -> Vec<(IVec3, Voxel)> {
    build_parts(kind, palette, seed, length).0
}

/// [`build`], with the fine parts (0.1 m boxes of its lamp posts and props, in model
/// cells). Props are merged in, every one of them: [`build_site`] keeps them apart, so
/// those on a trail can be left out.
pub fn build_parts(kind: Landmark, palette: &Palette, seed: u32, length: i32) -> (Vec<(IVec3, Voxel)>, Vec<voxel::mesh::PlantBox>) {
    let (mut voxels, mut fine, props) = build_site(kind, palette, seed, length);
    for prop in props {
        voxels.extend(prop.voxels);
        fine.extend(prop.fine);
    }
    (voxels, fine)
}

/// A landmark's voxels and fine parts, and apart from them the props laid about it,
/// each in its intact or decayed form by the site's [`History`].
pub fn build_site(kind: Landmark, palette: &Palette, seed: u32, length: i32) -> (Vec<(IVec3, Voxel)>, Vec<voxel::mesh::PlantBox>, Vec<props::Placed>) {
    let palette = &palette.varied(seed);
    let history = History::of(kind, seed);
    let mut rng = Rng::new(seed ^ (kind as u32).wrapping_mul(0x9e37_79b9));
    let mut s = Shape::raw();
    match kind {
        Landmark::Shrine => shrine(&mut s, &mut rng, palette),
        Landmark::Lookout => lookout(&mut s, &mut rng, palette),
        Landmark::Camp => camp(&mut s, &mut rng, palette),
        Landmark::Ruins => ruins(&mut s, &mut rng, palette),
        Landmark::Stones => stones(&mut s, &mut rng, palette),
        Landmark::Cairn => cairn(&mut s, &mut rng, palette),
        Landmark::Arch => arch(&mut s, &mut rng, palette),
        Landmark::Hut => hut(&mut s, &mut rng, palette),
        Landmark::Landing => landing(&mut s, &mut rng, palette, length * crate::scale::BLOCK),
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
    decay(&mut s, kind, palette, seed, history);
    let props = props::dress(kind, palette, seed, history, &s, length * crate::scale::BLOCK);
    let fine = s.take_fine();
    (s.into_voxels(), fine, props)
}

/// A lamp on a post beside a trail, in the style `hash` picks.
pub fn trail_lantern(palette: &Palette, height: i32, hash: u32) -> Vec<(IVec3, Voxel)> {
    trail_lantern_parts(palette, height, hash).0
}

/// [`trail_lantern`], with its fine parts in model cells.
pub fn trail_lantern_parts(palette: &Palette, height: i32, hash: u32) -> (Vec<(IVec3, Voxel)>, Vec<voxel::mesh::PlantBox>) {
    trail_lantern_styled(palette, height, hash, parts::LampStyle::pick_trail(hash >> 3), hash >> 7 & 3, parts::GATE_HALF)
}

/// [`trail_lantern_parts`] in a given style and turned a given number of quarter turns.
pub fn trail_lantern_styled(
    palette: &Palette,
    height: i32,
    hash: u32,
    style: parts::LampStyle,
    turns: u32,
    gate_half: i32,
) -> (Vec<(IVec3, Voxel)>, Vec<voxel::mesh::PlantBox>) {
    let palette = palette.varied(hash);
    parts::lantern_parts_spanning(style, &palette, height * crate::scale::BLOCK, turns, gate_half)
}

/// A signpost: a post with a cap, and one board per direction pointing along it, each
/// board a voxel thick with a pointed end and a bracket under it. Each direction is in radians.
pub fn signpost(palette: &Palette, directions: &[f32]) -> Vec<(IVec3, Voxel)> {
    let mut s = Shape::raw();
    for y in -6..=9 {
        s.set(v(0, y, 0), palette.wood);
    }
    for (x, z) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
        s.set(v(x, -1, z), palette.stone_dark);
    }
    s.set(v(0, 10, 0), palette.roof);
    for (k, &a) in directions.iter().enumerate().take(3) {
        let y = 8 - k as i32 * 2;
        for r in 1..=6 {
            let (x, z) = (a.cos() * r as f32, a.sin() * r as f32);
            let at = v(x.round() as i32, y, z.round() as i32);
            s.set(at, palette.plank);
            if r <= 5 {
                s.set(at + v(0, 1, 0), palette.plank);
            }
        }
        // A bracket under the board.
        let (bx, bz) = ((a.cos() * 1.0).round() as i32, (a.sin() * 1.0).round() as i32);
        s.set(v(bx, y - 1, bz), palette.wood);
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
                    assert!(a.iter().any(|&(q, _)| q.y == -3));
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

    #[test]
    fn old_ruins_crumble_and_kept_camps_do_not() {
        let palette = Palette::of(Biome::Deciduous);
        let count = |kind, seed, decayed: bool| {
            let palette = &palette.varied(seed);
            let mut rng = Rng::new(seed ^ (kind as u32).wrapping_mul(0x9e37_79b9));
            let mut s = Shape::raw();
            match kind {
                Landmark::Ruins => ruins(&mut s, &mut rng, palette),
                _ => camp(&mut s, &mut rng, palette),
            }
            if decayed {
                decay(&mut s, kind, palette, seed, History::of(kind, seed));
            }
            s.into_voxels().iter().filter(|&&(q, v)| v != AIR && q.y >= 1).count()
        };
        for seed in 0..12 {
            let h = History::of(Landmark::Ruins, seed);
            assert!(h.neglect() > 0.6, "{h:?}");
            assert!(count(Landmark::Ruins, seed, true) < count(Landmark::Ruins, seed, false), "seed {seed}: ruins kept everything");
            assert!(History::of(Landmark::Camp, seed).neglect() < 0.05);
            assert_eq!(count(Landmark::Camp, seed, true), count(Landmark::Camp, seed, false), "seed {seed}: a kept camp decayed");
            // A kept camp's fire still burns.
            assert!(build(Landmark::Camp, &palette, seed, 0).iter().any(|&(_, v)| v == mat::EMBER));
        }
    }

    /// Follows the stair from the door to the top floor, as a player would.
    #[test]
    fn lookout_stairs_can_be_climbed() {
        let palette = Palette::of(Biome::Grassland);
        for seed in 0..12 {
            let model: HashMap<IVec3, Voxel> = build(Landmark::Lookout, &palette, seed, 0).into_iter().collect();
            let solid = |p: IVec3| model.get(&p).is_some_and(|&v| v != AIR);
            let height = (0..80).rev().find(|&y| model.get(&v(0, y, 0)) == Some(&mat::LANTERN)).unwrap() - 1;
            let outer = |step: i32| tread_cells(step).into_iter().filter(|c| c.0.abs() > 1 || c.1.abs() > 1).collect::<Vec<_>>();
            for step in 0..height {
                let cells = outer(step);
                assert!(cells.len() >= 5, "seed {seed}: tread {step} is too narrow: {cells:?}");
                for &(x, z) in &cells {
                    assert!(solid(v(x, step, z)), "seed {seed}: tread {step} missing at {x},{z}");
                    for dy in 1..=5 {
                        if step + dy < height {
                            let next_tread = outer(step + dy).contains(&(x, z));
                            assert!(next_tread || !solid(v(x, step + dy, z)), "seed {seed}: no headroom above tread {step} at {x},{z}");
                        }
                    }
                }
                if step > 0 {
                    let prev = outer(step - 1);
                    let reaches = cells.iter().any(|a| prev.iter().any(|b| (a.0 - b.0).abs() <= 1 && (a.1 - b.1).abs() <= 1));
                    assert!(reaches, "seed {seed}: treads {} and {step} are not adjacent", step - 1);
                }
            }
            // The door is open at ground level.
            assert!(!solid(v(9, 1, 0)) && !solid(v(10, 3, 0)));
        }
    }
}
