//! The small things people leave about a place: barrels and crates by a door, a woodpile,
//! a chopping block with its axe, a kettle by the fire, offerings at a shrine. Each has an
//! intact form and a decayed one (a burst barrel, a crate stove in, a bench fallen
//! through, a woodpile gone to fungus), chosen by the site's [`History`].
//!
//! Props are mostly fine parts, 0.1 m boxes drawn like plants (see
//! [`voxel::mesh::PlantBox`]); only what is at least 0.4 m across and tall (a barrel, a
//! crate, a woodpile, a chopping block, a block of stone) has a voxel core, which is what
//! a player bumps into. Fine parts sit outside the faces of the voxels they dress, never
//! flush with one facing the same way, so no two surfaces fight.
//!
//! [`dress`] lays a few props fitting the kind of landmark about its pad: on the ground
//! or on a low step of the site's own stonework, never in its doors, stairs or other ways
//! through (see [`Shape::keep_clear`]), never touching each other, and never off the pad.
//! Props that would stand on a trail are dropped later, once the trails are down.

use glam::{IVec2, IVec3};
use voxel::{
    AIR, Voxel,
    mesh::{CELLS, PlantBox},
};

use super::{
    landmarks::{History, Landmark, Palette},
    parts::rotate_box,
    shape::{Rng, Shape, rotate},
};
use crate::materials as mat;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Prop {
    Barrel,
    Crate,
    Sack,
    RopeCoil,
    Pot,
    Bottle,
    Woodpile,
    ChoppingBlock,
    Bench,
    Hearth,
    Kettle,
    Fence,
    Rubble,
    PlankHeap,
    Offerings,
}

impl Prop {
    pub const ALL: [Prop; 15] = [
        Prop::Barrel,
        Prop::Crate,
        Prop::Sack,
        Prop::RopeCoil,
        Prop::Pot,
        Prop::Bottle,
        Prop::Woodpile,
        Prop::ChoppingBlock,
        Prop::Bench,
        Prop::Hearth,
        Prop::Kettle,
        Prop::Fence,
        Prop::Rubble,
        Prop::PlankHeap,
        Prop::Offerings,
    ];
}

/// A prop as built, in model space: its voxels, and its fine parts in cells.
#[derive(Clone, Debug)]
pub struct Placed {
    pub prop: Prop,
    pub decayed: bool,
    pub voxels: Vec<(IVec3, Voxel)>,
    pub fine: Vec<PlantBox>,
}

impl Placed {
    /// Every voxel it fills or draws into, sorted.
    pub fn cells(&self) -> Vec<IVec3> {
        let mut out: Vec<IVec3> = self.voxels.iter().map(|&(q, _)| q).collect();
        for b in &self.fine {
            let (lo, hi) = (IVec3::from_array(b.lo).div_euclid(IVec3::splat(CELLS)), (IVec3::from_array(b.hi) - 1).div_euclid(IVec3::splat(CELLS)));
            for y in lo.y..=hi.y {
                for z in lo.z..=hi.z {
                    for x in lo.x..=hi.x {
                        out.push(IVec3::new(x, y, z));
                    }
                }
            }
        }
        out.sort_unstable_by_key(|q| (q.y, q.x, q.z));
        out.dedup();
        out
    }

    /// The voxel columns it stands in, sorted.
    pub fn footprint(&self) -> Vec<IVec2> {
        let mut out: Vec<IVec2> = self.cells().iter().map(|q| IVec2::new(q.x, q.z)).collect();
        out.sort_unstable_by_key(|c| (c.x, c.y));
        out.dedup();
        out
    }

    /// Its lowest layer of voxels.
    pub fn base(&self) -> i32 {
        self.cells().first().map_or(0, |q| q.y)
    }

    /// Lifts it by `dy` voxels.
    fn lift(&mut self, dy: i32) {
        for (q, _) in &mut self.voxels {
            q.y += dy;
        }
        for b in &mut self.fine {
            b.lo[1] += dy * CELLS;
            b.hi[1] += dy * CELLS;
        }
    }
}

/// Lays one prop's parts: boxes in cells from the corner of the prop's own voxel 0, and
/// voxels from it, both turned with the prop.
struct Kit {
    at: IVec3,
    turns: u32,
    voxels: Vec<(IVec3, Voxel)>,
    fine: Vec<PlantBox>,
}

impl Kit {
    fn part(&mut self, lo: (i32, i32, i32), hi: (i32, i32, i32), v: Voxel, sway: bool) {
        let (lo, hi) = rotate_box(IVec3::from(lo), IVec3::from(hi), self.turns);
        debug_assert!(lo.cmplt(hi).all(), "empty prop part");
        let at = self.at * CELLS;
        self.fine.push(PlantBox { lo: (lo + at).to_array(), hi: (hi + at).to_array(), voxel: v, sway_lo: false, sway_hi: sway, thin: 0, season: 0 });
    }

    /// A box fixed in place.
    fn b(&mut self, lo: (i32, i32, i32), hi: (i32, i32, i32), v: Voxel) {
        self.part(lo, hi, v, false);
    }

    /// A weed whose top moves in the wind.
    fn weed(&mut self, lo: (i32, i32, i32), hi: (i32, i32, i32), v: Voxel) {
        self.part(lo, hi, v, true);
    }

    fn vox(&mut self, x: i32, y: i32, z: i32, v: Voxel) {
        let r = rotate(IVec3::new(x, y, z), self.turns);
        self.voxels.push((self.at + r, v));
    }

    /// A mushroom with its stem foot at cell (x, y, z): a stem and a cap.
    fn mushroom(&mut self, x: i32, y: i32, z: i32, cap: Voxel) {
        self.b((x, y, z), (x + 1, y + 1, z + 1), mat::MUSHROOM_STEM);
        self.b((x - 1, y + 1, z - 1), (x + 2, y + 2, z + 2), cap);
    }
}

/// Whether a place is damp enough for rot to grow fungus and weeds to grow green.
fn damp(p: &Palette) -> bool {
    p.moss > 0.1
}

fn weed_colour(p: &Palette) -> Voxel {
    if damp(p) { mat::TUFT } else { mat::TUFT_DRY }
}

/// Builds a prop with its voxel 0 at `at` (its foot on layer `at.y`), turned by quarter
/// turns, intact or decayed, varied by `seed`.
pub fn build(prop: Prop, p: &Palette, at: IVec3, turns: u32, decayed: bool, seed: u32) -> Placed {
    let mut k = Kit { at, turns, voxels: Vec::new(), fine: Vec::new() };
    let mut rng = Rng::new(seed ^ 0x9a0b_5eed ^ (prop as u32).wrapping_mul(0x27d4_eb2f));
    match prop {
        Prop::Barrel => barrel(&mut k, &mut rng, p, decayed),
        Prop::Crate => crate_box(&mut k, &mut rng, p, decayed),
        Prop::Sack => sack(&mut k, &mut rng, decayed),
        Prop::RopeCoil => rope_coil(&mut k, &mut rng, decayed),
        Prop::Pot => pot(&mut k, &mut rng, decayed),
        Prop::Bottle => bottle(&mut k, &mut rng, decayed),
        Prop::Woodpile => woodpile(&mut k, &mut rng, p, decayed),
        Prop::ChoppingBlock => chopping_block(&mut k, &mut rng, p, decayed),
        Prop::Bench => bench(&mut k, &mut rng, p, decayed),
        Prop::Hearth => hearth(&mut k, &mut rng, p, decayed),
        Prop::Kettle => kettle(&mut k, decayed),
        Prop::Fence => fence(&mut k, &mut rng, p, decayed),
        Prop::Rubble => rubble(&mut k, &mut rng, p, decayed),
        Prop::PlankHeap => plank_heap(&mut k, &mut rng, p, decayed),
        Prop::Offerings => offerings(&mut k, &mut rng, p, decayed),
    }
    Placed { prop, decayed, voxels: k.voxels, fine: k.fine }
}

/// A barrel 0.8 m across and tall: a voxel core in a skin of staves bellied at the middle
/// between two iron hoops, with a lid. Decayed, it has burst: a ring of staves still
/// standing at odd heights, the rest and a rusted hoop lying in the grass.
fn barrel(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    if !decayed {
        k.vox(0, 0, 0, p.plank);
        k.vox(0, 1, 0, p.plank);
        k.b((-1, 0, -1), (5, 2, 5), p.plank);
        k.b((-1, 6, -1), (5, 8, 5), p.plank);
        for (y0, y1, v) in [(2, 3, mat::IRON), (3, 5, p.plank), (5, 6, mat::IRON)] {
            k.b((-2, y0, -1), (6, y1, 5), v);
            k.b((-1, y0, -2), (5, y1, 6), v);
        }
        k.b((0, 8, 0), (4, 9, 4), mat::PLANK_DARK);
        return;
    }
    k.b((-1, 0, -1), (5, 1, 5), mat::PLANK_DARK);
    let staves = [((-1, 0), (0, 2)), ((-1, 2), (0, 4)), ((0, -1), (2, 0)), ((2, -1), (4, 0)), ((4, 0), (5, 2)), ((4, 2), (5, 4)), ((0, 4), (2, 5)), ((2, 4), (4, 5))];
    for ((x0, z0), (x1, z1)) in staves {
        if rng.unit() < 0.55 {
            k.b((x0, 1, z0), (x1, rng.int(2, 7), z1), p.plank);
        }
    }
    k.b((6, 0, 0), (13, 1, 1), mat::PLANK_DARK);
    k.b((6, 0, 3), (11, 1, 4), p.plank);
    k.b((1, 0, 6), (2, 1, 13), mat::PLANK_DARK);
    // A hoop that has rolled off and rusted.
    for (lo, hi) in [((-7, 6), (-1, 7)), ((-7, 11), (-1, 12)), ((-7, 7), (-6, 11)), ((-2, 7), (-1, 11))] {
        k.b((lo.0, 0, lo.1), (hi.0, 1, hi.1), mat::RUST);
    }
}

/// A crate 0.4 or 0.8 m on a side: a voxel core with corner posts, a rim round the lid and
/// a batten across each side. Decayed, it is stove in: the floor, a post or two, and a
/// side lying flat beside it.
fn crate_box(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    let n = if rng.unit() < 0.35 { 2 } else { 1 };
    let s = n * CELLS;
    let posts = [(-1, -1), (s - 1, -1), (-1, s - 1), (s - 1, s - 1)];
    if !decayed {
        for y in 0..n {
            for z in 0..n {
                for x in 0..n {
                    k.vox(x, y, z, p.plank);
                }
            }
        }
        for (x, z) in posts {
            k.b((x, 0, z), (x + 2, s, z + 2), p.wood);
        }
        k.b((-1, s, -1), (s + 1, s + 1, 1), p.wood);
        k.b((-1, s, s - 1), (s + 1, s + 1, s + 1), p.wood);
        k.b((-1, s, 1), (1, s + 1, s - 1), p.wood);
        k.b((s - 1, s, 1), (s + 1, s + 1, s - 1), p.wood);
        let h = s / 2;
        k.b((-1, h, 1), (0, h + 1, s - 1), p.wood);
        k.b((s, h, 1), (s + 1, h + 1, s - 1), p.wood);
        k.b((1, h, -1), (s - 1, h + 1, 0), p.wood);
        k.b((1, h, s), (s - 1, h + 1, s + 1), p.wood);
        // A small crate stacked on a big one.
        if n == 2 && rng.unit() < 0.5 {
            k.b((1, s + 1, 1), (5, s + 4, 5), p.plank);
            k.b((1, s + 4, 1), (5, s + 5, 2), p.wood);
            k.b((1, s + 4, 4), (5, s + 5, 5), p.wood);
        }
        return;
    }
    k.b((0, 0, 0), (s, 1, s), mat::PLANK_DARK);
    for (i, (x, z)) in posts.into_iter().enumerate() {
        if i == 0 || rng.unit() < 0.4 {
            k.b((x, 0, z), (x + 2, rng.int(2, s), z + 2), mat::PLANK_DARK);
        }
    }
    k.b((1, 1, 1), (s - 1, 2, 2), p.plank);
    k.b((s + 1, 0, 0), (2 * s + 1, 1, s), mat::PLANK_DARK);
    k.b((s + 1, 1, s / 2), (2 * s + 1, 2, s / 2 + 1), p.wood);
}

/// A sack of grain tied at the neck, sometimes a second lying by it. Decayed, it has
/// slumped and split, and the grain has run out.
fn sack(k: &mut Kit, rng: &mut Rng, decayed: bool) {
    if !decayed {
        k.b((0, 0, 0), (4, 3, 3), mat::SACKCLOTH);
        k.b((1, 3, 1), (3, 4, 2), mat::SACKCLOTH);
        k.b((1, 4, 1), (2, 5, 2), mat::ROPE);
        if rng.unit() < 0.5 {
            k.b((0, 0, 4), (5, 2, 7), mat::SACKCLOTH);
            k.b((5, 0, 5), (6, 1, 6), mat::ROPE);
        }
        return;
    }
    k.b((0, 0, 0), (5, 1, 4), mat::SACKCLOTH);
    k.b((1, 1, 1), (3, 2, 3), mat::SACKCLOTH);
    k.b((5, 0, 1), (8, 1, 3), mat::THATCH);
}

/// A coil of rope, two turns high with its end trailing. Decayed, a broken ring gone dark.
fn rope_coil(k: &mut Kit, rng: &mut Rng, decayed: bool) {
    let ring = [((0, 0), (4, 1)), ((0, 3), (4, 4)), ((0, 1), (1, 3)), ((3, 1), (4, 3))];
    if !decayed {
        for y in 0..2 {
            for (lo, hi) in ring {
                k.b((lo.0, y, lo.1), (hi.0, y + 1, hi.1), mat::ROPE);
            }
        }
        k.b((4, 0, 2), (7 + rng.int(0, 2), 1, 3), mat::ROPE);
        return;
    }
    let gap = rng.int(0, 3) as usize;
    for (i, (lo, hi)) in ring.into_iter().enumerate() {
        if i != gap {
            k.b((lo.0, 0, lo.1), (hi.0, 1, hi.1), mat::ROPE_ROTTEN);
        }
    }
}

/// A clay pot or a taller jar. Decayed, broken: its foot and a piece of wall, and shards.
fn pot(k: &mut Kit, rng: &mut Rng, decayed: bool) {
    if !decayed {
        let tall = rng.unit() < 0.4;
        let h = if tall { 4 } else { 3 };
        k.b((0, 0, 0), (3, h, 3), mat::CLAY);
        k.b((1, h, 1), (2, h + 1, 2), mat::CLAY);
        if rng.unit() < 0.5 {
            k.b((4, 0, 1), (6, 2, 3), mat::CLAY);
        }
        return;
    }
    k.b((0, 0, 0), (3, 1, 3), mat::CLAY);
    k.b((0, 1, 0), (1, 2, 3), mat::CLAY);
    k.b((4, 0, 1), (5, 1, 2), mat::CLAY);
    k.b((1, 0, 4), (3, 1, 5), mat::CLAY);
}

/// A corked bottle or two standing. Decayed, empties lying in the grass.
fn bottle(k: &mut Kit, rng: &mut Rng, decayed: bool) {
    if !decayed {
        k.b((1, 0, 1), (2, 2, 2), mat::GLASS_GREEN);
        k.b((1, 2, 1), (2, 3, 2), mat::PLANK_PALE);
        if rng.unit() < 0.6 {
            k.b((3, 0, 2), (4, 2, 3), mat::GLASS_GREEN);
            k.b((3, 2, 2), (4, 3, 3), mat::PLANK_PALE);
        }
        return;
    }
    k.b((0, 0, 1), (3, 1, 2), mat::GLASS_GREEN);
    if rng.unit() < 0.7 {
        k.b((2, 0, 3), (3, 1, 6), mat::GLASS_GREEN);
    }
}

/// A woodpile two or three voxels long against its stakes, log ends showing on both
/// faces and a row of logs along the top. Decayed, it has sunk to one course of grey
/// wood, logs have rolled off, and where it is damp fungus grows on it.
fn woodpile(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    let len = rng.int(2, 3);
    let h = if decayed { 1 } else { rng.int(1, 2) };
    let (core, ends, cut) = if decayed { (mat::WOOD_DEAD, mat::WOOD_DEAD, mat::PLANK_DARK) } else { (p.wood, mat::PLANK_PALE, p.plank) };
    for x in 0..len {
        for y in 0..h {
            k.vox(x, y, 0, core);
        }
    }
    let skip = if decayed { 0.5 } else { 0.2 };
    for i in 0..2 * len {
        for j in 0..2 * h {
            for (z0, z1) in [(-1, 0), (CELLS, CELLS + 1)] {
                if rng.unit() >= skip {
                    k.b((2 * i, 2 * j, z0), (2 * i + 2, 2 * j + 2, z1), if (i + j) % 2 == 0 { ends } else { cut });
                }
            }
        }
    }
    let top = h * CELLS;
    if !decayed {
        k.b((0, top, 1), (len * CELLS, top + 2, 3), p.wood);
        for (x0, x1) in [(-1, 0), (len * CELLS, len * CELLS + 1)] {
            for (z0, z1) in [(-1, 0), (CELLS, CELLS + 1)] {
                k.b((x0, 0, z0), (x1, top + 2, z1), p.wood);
            }
        }
        return;
    }
    k.b((len * CELLS + 1, 0, 0), (len * CELLS + 3, 2, 4), mat::WOOD_DEAD);
    k.b((1, 0, CELLS + 2), (7, 2, CELLS + 4), mat::WOOD_DEAD);
    if damp(p) {
        for i in 0..len {
            if rng.unit() < 0.7 {
                k.mushroom(i * CELLS + 2, top, 2, if rng.unit() < 0.3 { mat::MUSHROOM_RED } else { mat::MUSHROOM_TAN });
            }
        }
    }
}

/// A chopping block: a sawn round with its bark, an axe sunk in the top, split logs and
/// chips about it. Decayed, the block is grey and cracked, and the axe head lies rusting
/// beside it with what is left of the handle.
fn chopping_block(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    let (bark, top) = if decayed { (mat::WOOD_DEAD, mat::PLANK_DARK) } else { (p.wood, mat::PLANK_PALE) };
    k.vox(0, 0, 0, bark);
    k.b((-1, 0, 0), (5, 4, 4), bark);
    k.b((0, 0, -1), (4, 4, 5), bark);
    k.b((0, 4, 0), (4, 5, 4), top);
    if !decayed {
        k.b((1, 5, 1), (3, 7, 2), mat::IRON);
        k.b((3, 6, 1), (9, 7, 2), p.wood);
        k.b((6, 0, -1), (8, 1, 2), mat::PLANK_PALE);
        k.b((5, 0, 4), (6, 2, 6), mat::PLANK_PALE);
        for _ in 0..3 {
            let (x, z) = (rng.int(-4, 7), rng.int(6, 8));
            k.b((x, 0, z), (x + 1, 1, z + 1), mat::PLANK_PALE);
        }
        return;
    }
    k.b((1, 5, 1), (2, 6, 3), mat::PLANK_DARK);
    k.b((6, 0, 0), (8, 1, 1), mat::RUST);
    if rng.unit() < 0.6 {
        k.b((8, 0, 0), (12, 1, 1), mat::PLANK_DARK);
    }
    if damp(p) {
        k.b((-2, 2, 1), (-1, 3, 3), mat::MUSHROOM_TAN);
    }
}

/// A plank bench 1.2 m long on four legs with a stretcher. Decayed, it has fallen
/// through: the legs stand at broken heights and the seat lies in two halves below.
fn bench(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    let legs = [(1, 0), (10, 0), (1, 2), (10, 2)];
    if !decayed {
        for (x, z) in legs {
            k.b((x, 0, z), (x + 1, 4, z + 1), p.wood);
        }
        k.b((2, 1, 1), (10, 2, 2), p.wood);
        k.b((0, 4, 0), (12, 5, 3), p.plank);
        return;
    }
    for (x, z) in legs {
        if rng.unit() < 0.75 {
            k.b((x, 0, z), (x + 1, rng.int(1, 3), z + 1), mat::PLANK_DARK);
        }
    }
    k.b((-1, 0, 4), (5, 1, 7), mat::PLANK_DARK);
    k.b((6, 0, 4), (13, 1, 7), p.plank);
}

/// A ring of hearth stones 1.2 m across round an ash bed, charred logs on it. Decayed,
/// the logs are gone, stones have rolled out of the ring and weeds come up through it.
fn hearth(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    for i in 0..8 {
        let a = i as f32 / 8.0 * std::f32::consts::TAU;
        let out = if decayed && rng.unit() < 0.3 { 2.5 } else { 0.0 };
        let (x, z) = ((6.0 + (4.5 + out) * a.cos()).round() as i32, (6.0 + (4.5 + out) * a.sin()).round() as i32);
        let v = if i % 2 == 0 { p.stone } else { p.stone_dark };
        k.b((x - 1, 0, z - 1), (x + 1, rng.int(1, 2), z + 1), v);
    }
    k.b((4, 0, 4), (8, 1, 8), mat::ASH);
    if !decayed {
        k.b((3, 1, 5), (9, 2, 6), mat::CHARCOAL);
        k.b((5, 1, 3), (6, 2, 9), mat::CHARCOAL);
        k.b((6, 1, 6), (7, 2, 7), mat::WOOD_DEAD);
        return;
    }
    k.b((5, 1, 6), (7, 2, 7), mat::CHARCOAL);
    let weed = weed_colour(p);
    for (x, z) in [(4, 5), (7, 4), (6, 7)] {
        if rng.unit() < 0.8 {
            k.weed((x, 1, z), (x + 1, rng.int(3, 5), z + 1), weed);
        }
    }
}

/// An iron kettle with a spout and a bail handle. Decayed, tipped over and rusted, its
/// lid lying apart.
fn kettle(k: &mut Kit, decayed: bool) {
    if !decayed {
        k.b((0, 0, 0), (3, 2, 3), mat::IRON);
        k.b((3, 1, 1), (4, 2, 2), mat::IRON);
        k.b((1, 2, 0), (2, 4, 1), mat::IRON);
        k.b((1, 2, 2), (2, 4, 3), mat::IRON);
        k.b((1, 4, 0), (2, 5, 3), mat::IRON);
        return;
    }
    k.b((0, 0, 0), (2, 3, 3), mat::RUST);
    k.b((2, 0, 1), (3, 1, 2), mat::RUST);
    k.b((4, 0, 0), (6, 1, 2), mat::RUST);
}

/// A section of post and rail fence 1.2 m long. Decayed, a post is down, the others are
/// broken off, and a rail lies in the grass.
fn fence(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    let posts = [0, 6, 11];
    if !decayed {
        for x in posts {
            k.b((x, 0, 1), (x + 1, 10, 2), p.wood);
        }
        k.b((0, 4, 2), (12, 5, 3), p.plank);
        k.b((0, 8, 2), (12, 9, 3), p.plank);
        return;
    }
    let down = rng.int(0, 2) as usize;
    for (i, x) in posts.into_iter().enumerate() {
        if i != down {
            k.b((x, 0, 1), (x + 1, rng.int(4, 9), 2), mat::WOOD_DEAD);
        }
    }
    k.b((0, 3, 2), (6, 4, 3), mat::PLANK_DARK);
    k.b((1, 0, 4), (12, 1, 5), mat::PLANK_DARK);
}

/// Fallen stone: a block or two and broken pieces round them. Intact, the same stone
/// dressed and stacked to build with.
fn rubble(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    if !decayed {
        k.vox(0, 0, 0, p.stone);
        k.vox(1, 0, 0, p.stone_dark);
        k.b((1, CELLS, 1), (7, CELLS + 2, 3), p.stone);
        k.b((0, 0, 5), (4, 2, 7), p.stone_dark);
        return;
    }
    let mut blocks = vec![(0, 0)];
    if rng.unit() < 0.6 {
        blocks.push(if rng.unit() < 0.5 { (1, 0) } else { (0, 1) });
    }
    for &(x, z) in &blocks {
        k.vox(x, 0, z, p.block(rng));
    }
    for _ in 0..rng.int(5, 8) {
        let (x, z) = (rng.int(-3, 9), rng.int(-3, 9));
        // Pieces only beside the blocks, never in them.
        if blocks.iter().any(|&(bx, bz)| x.div_euclid(CELLS) == bx && z.div_euclid(CELLS) == bz) {
            continue;
        }
        let (w, d) = (rng.int(1, 3), rng.int(1, 2));
        if blocks.iter().any(|&(bx, bz)| (x + w - 1).div_euclid(CELLS) == bx && (z + d - 1).div_euclid(CELLS) == bz) {
            continue;
        }
        let v = p.block(rng);
        k.b((x, 0, z), (x + w, rng.int(1, 2), z + d), v);
    }
}

/// A stack of planks on two bearers, the courses staggered. Decayed, splintered planks
/// scattered dark and soft, fungus on them where it is damp.
fn plank_heap(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    if !decayed {
        k.b((1, 0, 0), (2, 1, 6), p.wood);
        k.b((10, 0, 0), (11, 1, 6), p.wood);
        for l in 0..rng.int(2, 3) {
            for i in 0..3 {
                let v = if (l + i) % 2 == 0 { p.plank } else { mat::PLANK_PALE };
                k.b((l % 2, 1 + l, 2 * i), (12 + l % 2, 2 + l, 2 * i + 2), v);
            }
        }
        return;
    }
    for row in 0..4 {
        let len = rng.int(4, 11);
        let x = rng.int(0, 12 - len);
        let v = if row % 2 == 0 { mat::PLANK_DARK } else { mat::WOOD_DEAD };
        k.b((x, 0, row * 3), (x + len, 1, row * 3 + 2), v);
    }
    k.b((5, 1, 1), (7, 2, 9), mat::PLANK_DARK);
    if damp(p) {
        k.mushroom(rng.int(1, 10), 1, 6, mat::MUSHROOM_TAN);
    }
}

/// Offerings laid on a cloth: a bowl, candles alight and flowers. Decayed, the cloth
/// faded and torn, the bowl broken, the candles burnt down and the flowers dry.
fn offerings(k: &mut Kit, rng: &mut Rng, p: &Palette, decayed: bool) {
    let flowers = [mat::BLOOM_RED, mat::BLOOM_YELLOW, mat::BLOOM_WHITE, mat::BLOOM_BLUE];
    if !decayed {
        k.b((0, 0, 0), (7, 1, 4), p.accent);
        k.b((1, 1, 1), (3, 2, 3), mat::CLAY);
        k.b((4, 1, 1), (5, 3, 2), mat::CLOTH_PALE);
        k.b((4, 3, 1), (5, 4, 2), mat::LANTERN);
        if rng.unit() < 0.6 {
            k.b((5, 1, 2), (6, 2, 3), mat::CLOTH_PALE);
            k.b((5, 2, 2), (6, 3, 3), mat::LANTERN);
        }
        let f = flowers[rng.int(0, 3) as usize];
        k.b((1, 2, 1), (3, 3, 3), f);
        k.b((6, 1, 0), (7, 2, 1), f);
        return;
    }
    k.b((0, 0, 0), (4, 1, 3), mat::SACKCLOTH);
    k.b((1, 1, 1), (2, 2, 2), mat::CLOTH_PALE);
    k.b((1, 2, 1), (2, 3, 2), mat::CHARCOAL);
    k.b((5, 0, 1), (7, 1, 2), mat::CLAY);
    k.b((2, 1, 2), (4, 2, 3), mat::TUFT_DRY);
}

/// What to lay about a site: (prop, nearest and farthest from the middle in voxels,
/// how many, the chance of each).
type Plan = &'static [(Prop, f32, f32, u32, f32)];

fn plan(kind: Landmark) -> Plan {
    use Prop::*;
    match kind {
        Landmark::Camp => &[
            (Kettle, 4.3, 5.6, 1, 0.9),
            (Woodpile, 5.0, 13.0, 1, 1.0),
            (Crate, 6.0, 13.0, 2, 0.8),
            (Sack, 6.0, 13.0, 2, 0.6),
            (Barrel, 6.0, 13.0, 1, 0.6),
            (ChoppingBlock, 6.0, 13.0, 1, 0.7),
            (RopeCoil, 6.0, 13.0, 1, 0.5),
        ],
        Landmark::Shrine => &[(Offerings, 2.0, 7.0, 2, 1.0), (Pot, 2.0, 8.0, 2, 0.8), (Bench, 6.0, 8.0, 1, 0.4)],
        Landmark::Lookout => &[
            (Barrel, 11.0, 14.5, 2, 0.9),
            (Crate, 11.0, 14.5, 2, 0.9),
            (Woodpile, 12.0, 14.5, 1, 0.7),
            (Sack, 11.0, 14.5, 1, 0.6),
            (RopeCoil, 11.0, 14.5, 1, 0.5),
        ],
        Landmark::Ruins => &[
            (Rubble, 4.0, 24.0, 4, 1.0),
            (PlankHeap, 4.0, 22.0, 1, 0.8),
            (Barrel, 4.0, 22.0, 1, 0.6),
            (Pot, 4.0, 22.0, 2, 0.8),
            (Bottle, 4.0, 22.0, 2, 0.6),
            (Hearth, 4.0, 18.0, 1, 0.5),
            (Fence, 6.0, 22.0, 1, 0.5),
            (Crate, 4.0, 22.0, 1, 0.5),
        ],
        Landmark::Hut => &[
            (Woodpile, 8.0, 15.0, 1, 1.0),
            (Barrel, 8.0, 15.0, 1, 0.8),
            (ChoppingBlock, 9.0, 15.0, 1, 0.8),
            (Bench, 9.0, 15.0, 1, 0.6),
            (Crate, 8.0, 15.0, 1, 0.6),
            (Sack, 8.0, 15.0, 1, 0.5),
            (Fence, 11.0, 15.5, 1, 0.5),
            (Pot, 2.0, 15.0, 1, 0.5),
        ],
        Landmark::Stones => &[(Offerings, 2.0, 6.0, 1, 0.8), (Rubble, 8.0, 22.0, 2, 0.8), (Pot, 3.0, 20.0, 1, 0.5)],
        Landmark::Cairn => &[(Offerings, 6.0, 8.5, 1, 0.8), (Pot, 6.0, 8.5, 1, 0.4)],
        Landmark::Arch => &[(Rubble, 4.0, 13.0, 1, 0.7), (Offerings, 3.0, 12.0, 1, 0.4)],
        Landmark::Landing => &[(Crate, 0.0, 0.0, 1, 0.8), (RopeCoil, 0.0, 0.0, 1, 0.8), (Barrel, 0.0, 0.0, 1, 0.5)],
    }
}

/// Supplies kept up on a lookout's top floor.
const LOOKOUT_TOP: Plan = &[(Prop::Crate, 3.0, 7.5, 1, 0.8), (Prop::Barrel, 3.0, 7.5, 1, 0.6), (Prop::Sack, 3.0, 7.5, 1, 0.5)];

/// Whether voxel `q` of the built site is taken by something solid.
fn solid(s: &Shape, q: IVec3) -> bool {
    s.get(q).is_some_and(|v| v != AIR)
}

/// The chance a prop at a site with this history is in its decayed form: none at a site
/// that is kept, all at an old and forgotten one.
pub fn decay_chance(h: History) -> f32 {
    ((h.neglect() - 0.15) * 2.0).clamp(0.0, 1.0)
}

/// Lays props fitting the kind of landmark about the built site `s`. `length` is the
/// pier's length in voxels for a landing, otherwise unused.
pub fn dress(kind: Landmark, p: &Palette, seed: u32, h: History, s: &Shape, length: i32) -> Vec<Placed> {
    let mut rng = Rng::new(seed ^ 0x9e0b_51e5 ^ (kind as u32).wrapping_mul(0x7feb_352d));
    let decay = decay_chance(h);
    let radius = kind.radius() - 1.5;
    let mut taken: std::collections::HashSet<IVec2> = std::collections::HashSet::new();
    let mut out = Vec::new();
    // Where each plan stands: the ground and the site's low steps, or a lookout's top floor.
    let mut plans: Vec<(Plan, i32, i32)> = vec![(plan(kind), 0, 2)];
    if kind == Landmark::Landing {
        plans[0].1 = 1;
        plans[0].2 = 1;
    }
    if kind == Landmark::Lookout {
        if let Some(lamp) = (0..80).rev().find(|&y| s.get(IVec3::new(0, y, 0)) == Some(mat::LANTERN)) {
            plans.push((LOOKOUT_TOP, lamp, lamp));
        }
    }
    for (list, ylo, yhi) in plans {
        for &(prop, r0, r1, count, chance) in list {
            for _ in 0..count {
                if rng.unit() >= chance {
                    continue;
                }
                for _attempt in 0..24 {
                    let (x, z) = if kind == Landmark::Landing {
                        (rng.int(2, (length - 6).max(2)), rng.int(2, 3))
                    } else {
                        let (a, r) = (rng.range(0.0, std::f32::consts::TAU), rng.range(r0, r1));
                        ((a.cos() * r).round() as i32, (a.sin() * r).round() as i32)
                    };
                    let turns = rng.int(0, 3) as u32;
                    let decayed = rng.unit() < decay;
                    let pseed = rng.next_u32();
                    let mut placed = build(prop, p, IVec3::new(x, 0, z), turns, decayed, pseed);
                    let foot = placed.footprint();
                    if kind != Landmark::Landing && foot.iter().any(|c| c.as_vec2().length() > radius) {
                        continue;
                    }
                    if foot.iter().any(|c| (-1..=1).any(|dz| (-1..=1).any(|dx| taken.contains(&(*c + IVec2::new(dx, dz)))))) {
                        continue;
                    }
                    let cells = placed.cells();
                    let fits = |y: i32| {
                        let supported = foot.iter().all(|c| {
                            let below = IVec3::new(c.x, y - 1, c.y);
                            solid(s, below) || (y == 0 && s.get(below).is_none())
                        });
                        supported && cells.iter().all(|q| {
                            let q = *q + IVec3::Y * y;
                            // The voxel and the one above it open: nothing is wedged under an overhang.
                            !solid(s, q) && !solid(s, q + IVec3::Y) && !s.must_stay_clear(q)
                        })
                    };
                    let Some(y) = (ylo..=yhi).find(|&y| fits(y)) else { continue };
                    placed.lift(y);
                    taken.extend(foot);
                    out.push(placed);
                    break;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{biome::Biome, features::landmarks};

    fn key(p: &Placed) -> (Prop, bool, Vec<(IVec3, Voxel)>, Vec<([i32; 3], [i32; 3], Voxel)>) {
        (p.prop, p.decayed, p.voxels.clone(), p.fine.iter().map(|b| (b.lo, b.hi, b.voxel)).collect())
    }

    #[test]
    fn every_prop_has_two_forms_and_is_deterministic() {
        let p = Palette::of(Biome::Deciduous);
        for prop in Prop::ALL {
            for turns in 0..4 {
                let a = build(prop, &p, IVec3::ZERO, turns, false, 5);
                let b = build(prop, &p, IVec3::ZERO, turns, true, 5);
                assert!(!a.fine.is_empty() && !b.fine.is_empty(), "{prop:?} is empty");
                assert_eq!(key(&a), key(&build(prop, &p, IVec3::ZERO, turns, false, 5)), "{prop:?} is not deterministic");
                assert_ne!(key(&a), key(&b), "{prop:?} does not decay");
                // Standing on the ground, never below it.
                for q in [a.cells(), b.cells()].concat() {
                    assert!(q.y >= 0, "{prop:?} reaches underground");
                }
                assert_eq!(a.base(), 0, "{prop:?} floats");
                for f in a.fine.iter().chain(&b.fine) {
                    assert!((0..3).all(|i| f.lo[i] < f.hi[i]), "{prop:?}: empty part {f:?}");
                }
            }
        }
    }

    #[test]
    fn props_are_deterministic_and_stay_on_their_pad() {
        for biome in [Biome::Deciduous, Biome::Desert, Biome::Taiga, Biome::OldGrowth] {
            let palette = Palette::of(biome);
            for kind in Landmark::ALL {
                let mut total = 0;
                for seed in 0..10 {
                    let (voxels, _, props) = landmarks::build_site(kind, &palette, seed, 12);
                    let (_, _, again) = landmarks::build_site(kind, &palette, seed, 12);
                    assert_eq!(props.iter().map(key).collect::<Vec<_>>(), again.iter().map(key).collect::<Vec<_>>(), "{kind:?} {seed}");
                    total += props.len();
                    let built: std::collections::HashMap<IVec3, Voxel> = voxels.into_iter().collect();
                    for prop in &props {
                        for c in prop.footprint() {
                            if kind != Landmark::Landing {
                                assert!(c.as_vec2().length() <= kind.radius() - 1.5, "{kind:?} {seed}: {:?} off the pad at {c}", prop.prop);
                            }
                        }
                        for q in prop.cells() {
                            assert!(built.get(&q).is_none_or(|&v| v == AIR), "{kind:?} {seed}: {:?} inside the build at {q}", prop.prop);
                        }
                    }
                }
                assert!(total > 0, "{kind:?} in {biome:?} has no props");
            }
        }
    }

    #[test]
    fn kept_sites_have_kept_props_and_ruins_decayed_ones() {
        let palette = Palette::of(Biome::Deciduous);
        for seed in 0..12 {
            let (_, _, camp) = landmarks::build_site(Landmark::Camp, &palette, seed, 0);
            assert!(camp.iter().all(|p| !p.decayed), "seed {seed}: a kept camp has decayed props");
            assert!(camp.iter().any(|p| p.prop == Prop::Woodpile), "seed {seed}: a camp has no woodpile");
            let (_, _, ruins) = landmarks::build_site(Landmark::Ruins, &palette, seed, 0);
            assert!(ruins.iter().all(|p| p.decayed), "seed {seed}: ruins have kept props");
            assert!(ruins.iter().any(|p| p.prop == Prop::Rubble), "seed {seed}: ruins have no rubble");
        }
    }

    #[test]
    fn props_keep_off_doors_and_the_lookout_stair() {
        let palette = Palette::of(Biome::Grassland);
        for seed in 0..16 {
            let (_, _, props) = landmarks::build_site(Landmark::Lookout, &palette, seed, 0);
            for prop in &props {
                for q in prop.cells() {
                    // Not inside the tower below its top floor, and not before its door.
                    let ground = prop.base() < 20;
                    assert!(!(ground && q.x.abs() <= 9 && q.z.abs() <= 9), "seed {seed}: {:?} in the tower at {q}", prop.prop);
                    assert!(!(ground && (7..=18).contains(&q.x) && (-4..=4).contains(&q.z)), "seed {seed}: {:?} at the door", prop.prop);
                }
            }
            let (_, _, hut) = landmarks::build_site(Landmark::Hut, &palette, seed, 0);
            // The hut's door is on +Z, 8 to 10 voxels out: the way in and out stays open.
            for prop in &hut {
                for q in prop.cells() {
                    assert!(!(q.x.abs() <= 3 && (6..=13).contains(&q.z)), "seed {seed}: {:?} in the hut door at {q}", prop.prop);
                }
            }
        }
    }

    #[test]
    fn desert_props_are_not_grey_stone() {
        for biome in [Biome::Desert, Biome::Savanna] {
            for kind in Landmark::ALL {
                for seed in 0..12 {
                    let (_, _, props) = landmarks::build_site(kind, &Palette::of(biome), seed, 12);
                    for prop in &props {
                        assert!(prop.voxels.iter().all(|&(_, v)| v != mat::STONE), "{kind:?} {seed}: {:?}", prop.prop);
                        assert!(prop.fine.iter().all(|b| b.voxel != mat::STONE), "{kind:?} {seed}: {:?}", prop.prop);
                    }
                }
            }
        }
    }
}
