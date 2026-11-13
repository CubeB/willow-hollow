//! Small delights: the quiet surprises someone has left beside a trail. Wind chimes on
//! the brow of a hill, a wayside shelf with a candle burning where two ways meet, a log
//! seat turned to a view, prayer flags over a high pass, stones balanced by a ford, a
//! face carved in a block of stone where the trail leaves the wood for the open heath.
//!
//! Each stands where it does for a reason, and each reason is rare; of the places that
//! have one, only some are taken, and never two near each other (see [`SPACING`] and
//! [`APART`]), so that coming on one is an event. They are built like the markers of
//! [`super::roadside`], of the rock in the ground and the timber of the country, held
//! back until every trail is down, and dropped if a later trail runs over them. What
//! hangs (chime rods, flags) is of fine parts whose free end moves with the wind in the
//! terrain shader, as leaves do.
//!
//! Nothing here makes a sound yet. [`super::Network::delights`] lists every one with
//! its place, and `GameWorld::delights_near` finds those about the player: that is where
//! the chimes' sound belongs when there is one.

use glam::{IVec2, IVec3, Vec2, vec2};
use voxel::{Voxel, mesh::CELLS};

use super::{
    build::HIGH,
    roadside::{FOOTING, Model, Trail, turns_toward},
};
use crate::{
    biome::Biome,
    continent::Continent,
    features::{
        landmarks::Palette,
        parts,
        shape::{Rng, rotate},
    },
    materials as mat,
    noise::{hash2, unit},
    scale::m,
    surface::{Cover, Sampler},
};

/// What has been left beside the trail.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Delight {
    /// Wind chimes hung from a crooked post, on the brow of a hill where the wind
    /// catches them.
    Chimes,
    /// A little stone shelf under a roof with a candle burning and a flower laid on
    /// it, where two ways meet.
    Shelf,
    /// A log seat turned to face the view, where the land falls away beside the trail.
    ViewBench,
    /// A string of prayer flags over the trail between two poles, on a high pass.
    Flags,
    /// Stones balanced one on another, on the bank by a ford.
    BalancedStones,
    /// A face carved in a block of stone, watching the trail where it leaves one
    /// country for another: out of the wood onto the heath, off the moor into the trees.
    CarvedFace,
}

impl Delight {
    pub const ALL: [Delight; 6] =
        [Delight::Chimes, Delight::Shelf, Delight::ViewBench, Delight::Flags, Delight::BalancedStones, Delight::CarvedFace];

    /// Why one stands where it does, in a word or two.
    pub fn reason(self) -> &'static str {
        match self {
            Delight::Chimes => "the brow of a hill",
            Delight::Shelf => "a meeting of ways",
            Delight::ViewBench => "a view",
            Delight::Flags => "a high pass",
            Delight::BalancedStones => "a ford",
            Delight::CarvedFace => "the edge of a country",
        }
    }

    /// Of the places with this one's reason, the share that get it.
    fn chance(self) -> f32 {
        match self {
            Delight::Chimes => 0.25,
            Delight::Shelf => 0.2,
            Delight::ViewBench => 0.08,
            Delight::Flags => 0.5,
            Delight::BalancedStones => 0.1,
            Delight::CarvedFace => 0.22,
        }
    }
}

/// Fewest trail steps between two delights on one trail: about 640 m.
const SPACING: usize = 800;
/// No two delights stand nearer each other than this, on whatever trails.
pub(super) const APART: f32 = m(600.0);
/// No two of one kind stand nearer each other than this.
pub(super) const APART_SAME: f32 = m(2500.0);
/// Trail steps either side of a brow that must lie below it.
const BROW: usize = 60;
/// How far the trail must fall away either side of a brow.
const BROW_DROP: f32 = m(2.4);
/// Trail steps of one country either side of the line where a trail passes into another.
const EDGE: usize = 48;
/// Height of a flag pole above the ground, in cells.
const POLE: i32 = 28;

/// Cloth of the five colours flags are strung in, in their order.
const FLAG_COLOURS: [Voxel; 5] = [mat::CLOTH_BLUE, mat::CLOTH_PALE, mat::CLOTH_RED, mat::MOSS, mat::FLOWER_YELLOW];

/// Builds a delight from the palette of the place, varied by `seed`: its foot on layer 0
/// and model +X toward the trail, except that a seat's front is model -Z and a string
/// of flags runs along +X from the pole at the origin to one `span` voxels away standing
/// `rise` voxels higher.
pub fn model(kind: Delight, p: &Palette, seed: u32, span: i32, rise: i32) -> Model {
    let mut k = Model::new();
    let mut rng = Rng::new(seed ^ 0x0de1_1675 ^ (kind as u32).wrapping_mul(0x2c1b_3c6d));
    let hang = |k: &mut Model, lo, hi, v| k.part(lo, hi, v, true, false);
    let swing = |k: &mut Model, lo, hi, v| k.part(lo, hi, v, true, true);
    match kind {
        Delight::Chimes => {
            // A post two metres tall with an arm out toward the trail on a brace, and
            // under the arm's end a round of wood on a cord with a rod at each corner,
            // each a different length, and the striker and its sail between them.
            k.vox(0, -FOOTING, 0, p.stone);
            k.b((1, -FOOTING * CELLS + CELLS, 1), (3, 20, 3), p.wood);
            k.b((3, 18, 1), (11, 19, 3), p.wood);
            k.b((3, 16, 1), (4, 18, 3), p.wood);
            k.b((4, 17, 1), (5, 18, 3), p.wood);
            k.b((7, 17, 1), (9, 18, 3), mat::ROPE);
            k.b((6, 16, 0), (10, 17, 4), p.plank);
            let mut lengths = [5, 6, 7, 9];
            for n in (1..4).rev() {
                lengths.swap(n, rng.int(0, n as i32) as usize);
            }
            for (n, (x, z)) in [(6, 0), (9, 0), (6, 3), (9, 3)].into_iter().enumerate() {
                let rod = if n % 2 == 0 { mat::PLANK_PALE } else { mat::WOOD_DEAD };
                hang(&mut k, (x, 16 - lengths[n], z), (x + 1, 16, z + 1), rod);
            }
            hang(&mut k, (7, 11, 1), (8, 16, 2), mat::ROPE);
            swing(&mut k, (7, 10, 1), (9, 11, 3), p.plank);
            swing(&mut k, (8, 7, 2), (9, 10, 3), mat::ROPE);
            swing(&mut k, (7, 4, 2), (9, 7, 3), p.accent);
        }
        Delight::Shelf => {
            // A pillar of stone 0.8 m high carrying a slab, and on the slab a niche of
            // three walls under a roof, open toward the trail: a candle alight in it, a
            // dish and a flower before it.
            for y in -FOOTING..=1 {
                k.vox(0, y, 0, if y == 0 && rng.unit() < 0.5 { p.stone_dark } else { p.stone });
            }
            k.b((-1, 8, -1), (5, 9, 5), p.stone_dark);
            k.b((0, 9, 0), (1, 13, 4), p.stone);
            k.b((1, 9, 0), (4, 13, 1), p.stone);
            k.b((1, 9, 3), (4, 13, 4), p.stone);
            k.b((-1, 13, -1), (5, 14, 5), p.roof);
            k.b((0, 14, 1), (4, 15, 3), p.roof);
            k.b((1, 9, 1), (2, 11, 2), mat::CLOTH_PALE);
            k.b((1, 11, 1), (2, 12, 2), mat::LANTERN);
            k.b((2, 9, 2), (4, 10, 3), mat::CLAY);
            let bloom = [mat::BLOOM_YELLOW, mat::BLOOM_WHITE, mat::BLOOM_RED, mat::BLOOM_BLUE][rng.int(0, 3) as usize];
            k.b((4, 9, 1), (5, 10, 2), bloom);
            if rng.unit() < 0.5 {
                k.b((4, 9, 3), (5, 10, 4), p.accent);
            }
            if p.moss > 0.1 {
                k.b((1, 15, 1), (3, 16, 3), mat::MOSS_CUSHION);
            }
        }
        Delight::ViewBench => {
            // Half a log on two stones, its flat side up and worn pale where people
            // sit. The view is in front, at model -Z; the trail behind.
            for x in [0, 2] {
                for y in -FOOTING..=0 {
                    k.vox(x, y, 0, if rng.unit() < 0.3 { p.stone_dark } else { p.stone });
                }
            }
            k.b((-1, 4, 0), (13, 5, 4), p.wood);
            k.b((-1, 5, 0), (13, 6, 4), p.plank);
        }
        Delight::Flags => {
            // Two poles, a cord between them that sags in the middle, and the flags
            // hung from it a hand apart: five colours over and over.
            let far = span * CELLS;
            let dy = rise * CELLS;
            k.vox(0, -FOOTING, 0, p.stone);
            k.vox(span, rise - FOOTING, 0, p.stone);
            k.b((1, -FOOTING * CELLS + CELLS, 1), (3, POLE, 3), p.wood);
            k.b((far + 1, dy - FOOTING * CELLS + CELLS, 1), (far + 3, dy + POLE, 3), p.wood);
            let (x0, x1) = (3, far + 1);
            let len = (x1 - x0) as f32;
            let sag = (len / 8.0).clamp(1.0, 3.0);
            let mut colour = rng.int(0, 4) as usize;
            let mut x = x0;
            while x < x1 {
                let w = 3.min(x1 - x);
                let t = (x - x0) as f32 / len + 0.5 * w as f32 / len;
                let y = POLE - 2 + (dy as f32 * t).round() as i32 - (sag * (1.0 - (2.0 * t - 1.0).powi(2))).round() as i32;
                k.b((x, y, 2), (x + w, y + 1, 3), mat::ROPE);
                if w == 3 {
                    hang(&mut k, (x, y - 3, 2), (x + 2, y, 3), FLAG_COLOURS[colour % 5]);
                    colour += 1;
                }
                x += w;
            }
        }
        Delight::BalancedStones => {
            // A flat stone, a small one on it, a wider one balanced on that, and so
            // on up to a pebble a metre from the ground; a second, shorter stack by it.
            k.vox(0, -FOOTING, 0, p.stone);
            let stone = |rng: &mut Rng| if rng.unit() < 0.4 { p.stone_dark } else { p.stone };
            let lean = rng.int(0, 1);
            let stack: [((i32, i32, i32), (i32, i32, i32)); 6] = [
                ((0, 0, 0), (4, 2, 4)),
                ((1, 2, 1), (3, 4, 3)),
                ((lean - 1, 4, 0), (lean + 3, 5, 3)),
                ((1, 5, 1), (3, 7, 2)),
                ((1 - lean, 7, 0), (4 - lean, 8, 3)),
                ((2, 8, 1), (3, 10, 2)),
            ];
            for (lo, hi) in stack {
                let v = stone(&mut rng);
                k.b(lo, hi, v);
            }
            if rng.unit() < 0.7 {
                for (lo, hi) in [((1, 0, 6), (4, 1, 9)), ((2, 1, 7), (3, 3, 8)), ((1, 3, 6), (4, 4, 8))] {
                    let v = stone(&mut rng);
                    k.b(lo, hi, v);
                }
            }
        }
        Delight::CarvedFace => {
            // A block of stone 0.8 m each way, its face toward the trail worked in
            // relief: a heavy brow, a long nose, eyes and a mouth cut dark beneath.
            for y in -FOOTING..=1 {
                for (x, z) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    k.vox(x, y, z, p.stone);
                }
            }
            // One row of the face to a line, top first: `#` stands proud by two cells,
            // `.` by one, `o` is cut dark, a space is the bare block.
            const FACE: [&[u8; 8]; 8] = [b" ...... ", b"########", b".oo##oo.", b"...##...", b"..####..", b"........", b".oooooo.", b"        "];
            let cut = if p.stone == mat::BASALT || p.stone == mat::ROCK_DARK { mat::CHARCOAL } else { mat::SLATE };
            for (row, line) in FACE.iter().enumerate() {
                let y = 7 - row as i32;
                // Runs of one sign become one box.
                let mut z = 0;
                while z < 8 {
                    let sign = line[z];
                    let mut end = z + 1;
                    while end < 8 && line[end] == sign {
                        end += 1;
                    }
                    let (depth, v) = match sign {
                        b'#' => (2, p.stone),
                        b'.' => (1, p.stone),
                        b'o' => (1, cut),
                        _ => (0, p.stone),
                    };
                    if depth > 0 {
                        k.b((8, y, z as i32), (8 + depth, y + 1, end as i32), v);
                    }
                    z = end;
                }
            }
            // The crown of the head, and what has grown on it.
            k.b((1, 8, 1), (7, 9, 7), p.stone);
            if p.moss > 0.1 {
                k.b((2, 9, 2), (5, 10, 6), mat::MOSS_CUSHION);
            } else if rng.unit() < 0.5 {
                k.b((3, 9, 2), (5, 10, 4), mat::LICHEN_GREY);
            }
        }
    }
    k
}

/// One delight chosen beside a trail and placed: absolute voxels and fine parts in
/// continent-local coordinates.
pub struct Placed {
    pub kind: Delight,
    pub at: Vec2,
    /// Where model voxel 0 is: on the column it stands in, one above the ground.
    pub origin: IVec3,
    pub voxels: Vec<(IVec3, Voxel)>,
    pub fine: Vec<voxel::mesh::PlantBox>,
    /// The columns where it stands on the ground.
    pub footprint: Vec<IVec2>,
}

/// A delight standing in the world.
#[derive(Clone, Debug)]
pub struct Standing {
    pub kind: Delight,
    /// The continent-local voxel its model's foot stands in, one above the ground.
    pub at: IVec3,
    /// The columns where it stands on the ground.
    pub footprint: Vec<IVec2>,
}

/// What a trail offers at step `i`, if anything: the kind of delight its reason calls
/// for, and the side of the trail the reason lies on (0 for either).
fn reason_at(c: &Continent, seed: u32, t: &Trail, fords: &[(usize, usize)], junctions: &[Vec2], i: usize) -> Option<(Delight, f32)> {
    // One roll for each place a reason holds, not for each step of it.
    let lucky = |kind: Delight, place: i32| unit(hash2(c.desc.seed ^ 0x0de1_1678 ^ t.salt, place, kind as i32)) < kind.chance();
    let col = &t.cols[i];
    if let Some(&(s, _)) = fords.iter().find(|&&(s, e)| i + 4 == s || i == e + 4) {
        return lucky(Delight::BalancedStones, s as i32).then_some((Delight::BalancedStones, 0.0));
    }
    // The top of a rise, the trail falling away both ways.
    let h = t.heights[i];
    if i >= BROW && i + BROW < t.pts.len() {
        let (before, after) = (&t.heights[i - BROW..i], &t.heights[i + 1..=i + BROW]);
        let top = before.iter().all(|&y| y < h) && after.iter().all(|&y| y <= h);
        let low = |ys: &[f32]| ys.iter().copied().fold(f32::MAX, f32::min);
        if top && h - low(before) >= BROW_DROP && h - low(after) >= BROW_DROP {
            let high = col.ground > HIGH.0 || matches!(col.biome, Biome::Alpine | Biome::Tundra | Biome::AlpineMeadow | Biome::SnowyPine);
            let kind = if high { Delight::Flags } else { Delight::Chimes };
            return lucky(kind, i as i32).then_some((kind, 0.0));
        }
    }
    if let Some(j) = junctions.iter().find(|j| (m(3.6)..m(6.0)).contains(&j.distance(t.pts[i]))) {
        return lucky(Delight::Shelf, hash2(7, j.x as i32, j.y as i32) as i32).then_some((Delight::Shelf, 0.0));
    }
    // Where the trail leaves one country for another (out of the wood onto the heath,
    // off the moor into the trees), each a good way deep on its side of the line.
    if i >= EDGE && i + EDGE < t.pts.len() && t.cols[i - 1].biome != col.biome {
        let whole = |from: usize, biome: Biome| (from..from + EDGE).step_by(8).all(|k| t.cols[k].biome == biome);
        if whole(i - EDGE, t.cols[i - 1].biome) && whole(i, col.biome) {
            return lucky(Delight::CarvedFace, i as i32).then_some((Delight::CarvedFace, 0.0));
        }
    }
    if !i.is_multiple_of(8) {
        return None;
    }
    let tangent = (t.pts[i + 4] - t.pts[i - 4]).normalize_or_zero();
    let normal = vec2(-tangent.y, tangent.x);
    let bare = |q: Vec2| {
        let (x, z) = (q.x.floor() as i32, q.y.floor() as i32);
        Sampler::new(c, seed, x, z, 1).column(x, z)
    };
    // A view: the land falling well away on one side, and nothing standing in front.
    if lucky(Delight::ViewBench, (i / 64) as i32) {
        for side in [1.0, -1.0] {
            let at = |d: f32| bare(t.pts[i] + normal * side * d).ground;
            if at(m(16.0)) <= h && h - at(m(40.0)) >= m(7.0) && h - at(m(80.0)) >= m(14.0) {
                return Some((Delight::ViewBench, side));
            }
        }
    }
    None
}

/// Chooses and builds the delights beside one trail. `fords` are the runs of its steps
/// that cross water on stepping stones; `taken` are places already occupied (lanterns,
/// signposts, markers and delights on trails laid before this one), kept clear of.
pub fn choose(c: &Continent, seed: u32, t: &Trail, fords: &[(usize, usize)], junctions: &[Vec2], taken: &[Vec2]) -> Vec<Placed> {
    let n = t.pts.len();
    let mut out: Vec<Placed> = Vec::new();
    let mut next = 30;
    for i in 30..n.saturating_sub(30) {
        if i < next || t.worn[i] <= 0.0 || t.carried(i) {
            continue;
        }
        let Some((kind, side)) = reason_at(c, seed, t, fords, junctions, i) else { continue };
        if let Some(placed) = place(c, seed, t, i, kind, side, taken) {
            out.push(placed);
            next = i + SPACING;
        }
    }
    out
}

/// Builds a delight of `kind` beside step `i`: on the side its reason lies, or on
/// whichever has room when `side` is 0. `None` if it cannot stand there.
fn place(c: &Continent, seed: u32, t: &Trail, i: usize, kind: Delight, side: f32, taken: &[Vec2]) -> Option<Placed> {
    let tangent = (t.pts[i + 4] - t.pts[i - 4]).normalize_or_zero();
    if tangent == Vec2::ZERO {
        return None;
    }
    let normal = vec2(-tangent.y, tangent.x);
    let pick = hash2(t.salt ^ 0x0de1_1676, i as i32, kind as i32);
    let first = if side != 0.0 {
        side
    } else if pick & 1 == 0 {
        1.0
    } else {
        -1.0
    };
    let sides: &[f32] = if side != 0.0 || kind == Delight::Flags { &[first] } else { &[first, -first] };
    let edge = t.widths[i].max(t.worn[i]);
    let off = match kind {
        Delight::Chimes => m(1.3),
        Delight::Shelf => m(0.7),
        Delight::ViewBench => m(0.9),
        Delight::Flags => 0.0,
        Delight::BalancedStones => m(0.9),
        Delight::CarvedFace => m(0.6),
    };
    for &side in sides {
        // The model's +X faces the trail; a seat's front faces away from it, to the
        // view; flags run across the way from one pole to the other.
        let (q, turns, span) = match kind {
            Delight::Flags => {
                let turns = turns_toward(normal);
                let axis = rotate(IVec3::X, turns);
                let axis = vec2(axis.x as f32, axis.z as f32);
                // Wider where the trail runs askew to the string.
                let half = ((edge + 1.2) / axis.dot(normal).abs().max(0.7)).ceil() as i32;
                if half > 6 {
                    return None;
                }
                (t.pts[i].floor() + 0.5 - axis * half as f32, turns, 2 * half)
            }
            Delight::ViewBench => (t.pts[i] + normal * side * (edge + off), (turns_toward(normal * side) + 1) % 4, 0),
            _ => (t.pts[i] + normal * side * (edge + off), turns_toward(-normal * side), 0),
        };
        if taken.iter().any(|o| o.distance_squared(q) < m(3.2).powi(2)) {
            continue;
        }
        let (x, z) = (q.x.floor() as i32, q.y.floor() as i32);
        let s = Sampler::new(c, seed, x - 14, z - 14, 29);
        // The ground that will be at a column once the trail is down, if it can be stood on.
        let ground = |cx: i32, cz: i32| -> Option<i32> {
            let bare = s.column(cx, cz);
            let dry = bare.water_surface().is_none() && bare.cover != Cover::Cliff && bare.river >= m(0.8);
            dry.then(|| t.ground_at(&bare, vec2(cx as f32 + 0.5, cz as f32 + 0.5), i).floor() as i32)
        };
        let Some(g) = ground(x, z) else { continue };
        let rise = if kind == Delight::Flags {
            let far = rotate(IVec3::new(span, 0, 0), turns);
            let Some(other) = ground(x + far.x, z + far.z) else { continue };
            if (other - g).abs() > 3 {
                continue;
            }
            other - g
        } else {
            0
        };
        let palette = super::build::palette_at(c, seed, s.column(x, z).biome, x, g, z).varied(hash2(seed ^ 0x3a81, x >> 6, z >> 6));
        let built = model(kind, &palette, pick, span, rise);
        let origin = IVec3::new(x, g + 1, z);
        let voxels: Vec<(IVec3, Voxel)> = built.voxels.iter().map(|&(p, v)| (origin + rotate(p, turns), v)).collect();
        let fine = parts::place_fine(&built.fine, origin, turns);
        // Where it stands: its voxels, and the parts that come down to the ground.
        let mut blocks: Vec<IVec2> = voxels.iter().map(|(p, _)| IVec2::new(p.x, p.z)).collect();
        blocks.sort_unstable_by_key(|c| (c.x, c.y));
        blocks.dedup();
        let mut footprint = blocks.clone();
        for b in fine.iter().filter(|b| b.lo[1] <= origin.y * CELLS) {
            let (lo, hi) = (IVec3::from_array(b.lo).div_euclid(IVec3::splat(CELLS)), (IVec3::from_array(b.hi) - 1).div_euclid(IVec3::splat(CELLS)));
            for zz in lo.z..=hi.z {
                for xx in lo.x..=hi.x {
                    footprint.push(IVec2::new(xx, zz));
                }
            }
        }
        footprint.sort_unstable_by_key(|c| (c.x, c.y));
        footprint.dedup();
        if t.on_way(&footprint, i) {
            continue;
        }
        // Level ground under all of it, though a block with its sunk footing can stand
        // where the ground is a voxel lower; the far pole of a string of flags stands
        // on its own ground.
        let far = rotate(IVec3::new(span, 0, 0), turns);
        let level = footprint.iter().all(|c| {
            let want = if kind == Delight::Flags && (c.x - x - far.x).abs() <= 1 && (c.y - z - far.z).abs() <= 1 { g + rise } else { g };
            ground(c.x, c.y).is_some_and(|y| y == want || (y == want - 1 && kind != Delight::ViewBench && blocks.contains(c)))
        });
        if !level {
            continue;
        }
        return Some(Placed { kind, at: q, origin, voxels, fine, footprint });
    }
    None
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::{continent::SimConfig, macro_layout::MACRO_SIZE, world::GameWorld};

    #[test]
    fn every_delight_is_repeatable_and_stands_on_a_sunk_footing() {
        for biome in [Biome::Grassland, Biome::Desert, Biome::Moor, Biome::Rainforest] {
            let p = Palette::at(5, biome, 100, 60, 100);
            for kind in Delight::ALL {
                for seed in 0..20 {
                    let (span, rise) = if kind == Delight::Flags { (4 + seed as i32 % 8, seed as i32 % 5 - 2) } else { (0, 0) };
                    let a = model(kind, &p, seed, span, rise);
                    let b = model(kind, &p, seed, span, rise);
                    assert_eq!(a.voxels, b.voxels);
                    let key = |m: &Model| m.fine.iter().map(|b| (b.lo, b.hi, b.voxel, b.sway_lo, b.sway_hi)).collect::<Vec<_>>();
                    assert_eq!(key(&a), key(&b));
                    assert!(a.voxels.iter().any(|(q, _)| q.y == -FOOTING), "{kind:?} has no footing");
                    assert!(a.fine.iter().all(|b| b.lo.iter().zip(&b.hi).all(|(l, h)| l < h)), "{kind:?} has an empty part");
                    // No part lies inside another's face: none overlap.
                    for (n, x) in a.fine.iter().enumerate() {
                        for y in &a.fine[n + 1..] {
                            let apart = (0..3).any(|k| x.hi[k] <= y.lo[k] || y.hi[k] <= x.lo[k]);
                            assert!(apart, "{kind:?} {seed}: {x:?} overlaps {y:?}");
                        }
                        // Nor inside one of its voxels.
                        for (q, _) in &a.voxels {
                            let (lo, hi) = (*q * CELLS, (*q + 1) * CELLS);
                            let apart = (0..3).any(|k| x.hi[k] <= lo[k] || hi[k] <= x.lo[k]);
                            assert!(apart, "{kind:?} {seed}: {x:?} is inside the voxel at {q:?}");
                        }
                    }
                    // Built of the place: no grey stone on the moor or in the desert.
                    if matches!(biome, Biome::Desert | Biome::Moor) {
                        assert!(a.voxels.iter().all(|&(_, v)| v != mat::STONE));
                    }
                }
            }
        }
        // Only what hangs moves: the chimes' rods and the flags.
        for kind in Delight::ALL {
            let moves = model(kind, &Palette::of(Biome::Grassland), 3, 6, 0).fine.iter().filter(|b| b.sway_lo || b.sway_hi).count();
            assert_eq!(moves > 0, matches!(kind, Delight::Chimes | Delight::Flags), "{kind:?}");
        }
    }

    #[test]
    fn delights_are_rare_placed_for_a_reason_and_keep_off_the_way() {
        let w = GameWorld::with_config(1, SimConfig { size: 1024 });
        let c = w.continent(0, 0);
        let net = &c.trails;
        let mut kinds: HashMap<Delight, usize> = HashMap::new();
        for d in &net.delights {
            *kinds.entry(d.kind).or_default() += 1;
        }
        let steps: usize = net.lines.iter().map(|l| l.len() * 8).sum();
        let km = steps as f32 * 0.8 / 1000.0;
        println!("{} delights on {km:.0} km of trail: {kinds:?}", net.delights.len());
        assert!(net.delights.len() >= 6, "hardly any delights: {kinds:?}");
        assert!(kinds.len() >= 4, "too few kinds of delight: {kinds:?}");
        // A surprise: fewer than one to every two kilometres of trail, and far fewer
        // than the markers.
        assert!((net.delights.len() as f32) < km / 2.0, "{} delights on {km:.0} km of trail", net.delights.len());
        assert!(net.delights.len() * 8 < net.markers.len(), "{} delights to {} markers", net.delights.len(), net.markers.len());
        let walked: Vec<_> = net.points().filter(|p| p.half_width > 0.0).collect();
        for (n, d) in net.delights.iter().enumerate() {
            let at = vec2(d.at.x as f32 + 0.5, d.at.z as f32 + 0.5);
            // Beside a trail, and never on it.
            let near = walked.iter().map(|p| vec2(p.x, p.z).distance(at) - p.half_width).fold(f32::MAX, f32::min);
            assert!(near < m(4.0), "{:?} at {:?} is {near:.1} voxels from any trail", d.kind, d.at);
            for foot in &d.footprint {
                let f = foot.as_vec2() + 0.5;
                assert!(walked.iter().all(|p| vec2(p.x, p.z).distance(f) >= p.half_width + 0.5), "{:?} at {:?} stands on the trail", d.kind, d.at);
            }
            // Well apart, and further from another of its own kind.
            for e in &net.delights[n + 1..] {
                let gap = at.distance(vec2(e.at.x as f32 + 0.5, e.at.z as f32 + 0.5));
                assert!(gap >= APART, "{:?} and {:?} stand {gap:.0} voxels apart", d.kind, e.kind);
                assert!(d.kind != e.kind || gap >= APART_SAME, "two {:?} stand {gap:.0} voxels apart", d.kind);
            }
            // Each for its reason.
            match d.kind {
                Delight::BalancedStones => {
                    let ford = net.crossings.iter().any(|x| x.kind == super::super::CrossingKind::Stones && vec2(x.x as f32, x.z as f32).distance(at) < m(16.0));
                    assert!(ford, "balanced stones at {:?} with no ford near", d.at);
                }
                Delight::Shelf => {
                    assert!(net.junctions.iter().any(|j| j.distance(at) < m(12.0)), "a shelf at {:?} with no meeting of ways", d.at);
                }
                _ => {}
            }
        }
        // The same again from the same seed.
        let again = GameWorld::with_config(1, SimConfig { size: 1024 });
        let other = again.continent(0, 0);
        let key = |n: &super::super::Network| n.delights.iter().map(|d| (d.kind, d.at)).collect::<Vec<_>>();
        assert_eq!(key(net), key(&other.trails));
        // And the game can find the chimes about a place.
        if let Some(chime) = net.delights.iter().find(|d| d.kind == Delight::Chimes) {
            let (x, z) = (c.desc.mx * MACRO_SIZE + chime.at.x, c.desc.mz * MACRO_SIZE + chime.at.z);
            let found = w.delights_near(x + 20, z - 30, 64);
            assert!(found.iter().any(|&(kind, at)| kind == Delight::Chimes && at == IVec3::new(x, chime.at.y, z)), "{found:?}");
            assert!(w.delights_near(x + 400, z, 64).iter().all(|&(_, at)| at.x != x));
        }
    }
}
