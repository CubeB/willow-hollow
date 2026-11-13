//! The small things set beside a trail, by how much it is used: milestones and benches
//! along the main routes, waymark posts on faint footpaths, and cairns where a faint
//! trail crosses stony ground or fades out altogether, so the way can be found again.
//!
//! Each is a model a voxel or so across, mostly of fine parts, with a footing sunk two
//! voxels into the ground so it never floats where the ground beside the trail settles
//! a little lower than reckoned. Like a site's props they are held back until every
//! trail is down, and any that a later trail runs over is dropped (see
//! [`super::build`]).

use glam::{IVec2, IVec3, Vec2, vec2};
use voxel::{
    Voxel,
    mesh::{CELLS, PlantBox},
};

use super::{
    TrailPoint,
    build::{TrailClass, class_of},
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
    surface::{Column, Cover, Sampler, shoulder_weight},
};

/// What stands beside the trail.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Marker {
    /// A dressed stone a little under a metre tall, every half kilometre of a main route.
    Milestone,
    /// A plank seat on two stone blocks, now and then where a main route runs level.
    Bench,
    /// A short post with a painted band, along a faint footpath.
    Waymark,
    /// A small pile of stones, where a faint trail crosses stony ground or fades out.
    Cairn,
}

/// One marker as built, before it is placed: voxels and fine parts in model space, its
/// foot on layer 0 and model +X pointing at the trail (or, for a bench, along it).
pub struct Model {
    pub voxels: Vec<(IVec3, Voxel)>,
    pub fine: Vec<PlantBox>,
}

impl Model {
    pub(super) fn new() -> Self {
        Model { voxels: Vec::new(), fine: Vec::new() }
    }

    /// A fixed box, in cells from the corner of model voxel 0.
    pub(super) fn b(&mut self, lo: (i32, i32, i32), hi: (i32, i32, i32), v: Voxel) {
        self.part(lo, hi, v, false, false);
    }

    /// A box whose bottom (`sway_lo`) or top (`sway_hi`) moves in the wind.
    pub(super) fn part(&mut self, lo: (i32, i32, i32), hi: (i32, i32, i32), v: Voxel, sway_lo: bool, sway_hi: bool) {
        debug_assert!(lo.0 < hi.0 && lo.1 < hi.1 && lo.2 < hi.2, "empty marker part");
        self.fine.push(PlantBox { lo: [lo.0, lo.1, lo.2], hi: [hi.0, hi.1, hi.2], voxel: v, sway_lo, sway_hi, thin: 0 });
    }

    pub(super) fn vox(&mut self, x: i32, y: i32, z: i32, v: Voxel) {
        self.voxels.push((IVec3::new(x, y, z), v));
    }
}

/// Depth of every marker's footing below the ground it stands on, in voxels.
pub(super) const FOOTING: i32 = 2;

/// Builds a marker from the palette of the country it stands in, varied by `seed`.
pub fn model(kind: Marker, p: &Palette, seed: u32) -> Model {
    let mut k = Model::new();
    let mut rng = Rng::new(seed ^ 0x3a7e_51de ^ (kind as u32).wrapping_mul(0x2c1b_3c6d));
    match kind {
        Marker::Milestone => {
            // A block two voxels tall with a ridged cap and a dark carved plate on the
            // face toward the trail; lichen on some.
            for y in -FOOTING..=1 {
                k.vox(0, y, 0, p.stone);
            }
            k.b((0, 8, 1), (4, 9, 3), p.stone);
            k.b((4, 3, 1), (5, 6, 3), mat::SLATE);
            if p.moss > 0.1 && rng.unit() < 0.6 {
                k.b((-1, 1, 0), (0, 3, 2), mat::LICHEN_GREY);
            }
        }
        Marker::Bench => {
            // Two stone blocks 0.8 m apart, a plank seat 1.2 m long across them.
            for x in [0, 2] {
                for y in -FOOTING..=0 {
                    k.vox(x, y, 0, p.stone);
                }
            }
            k.b((0, 4, 0), (12, 5, 4), p.plank);
            if rng.unit() < 0.5 {
                // A board for a back, on two short posts behind the seat (the trail is
                // in front of it, at -Z).
                k.b((1, 5, 3), (2, 9, 4), p.wood);
                k.b((10, 5, 3), (11, 9, 4), p.wood);
                k.b((0, 8, 4), (12, 10, 5), p.plank);
            }
        }
        Marker::Waymark => {
            // A post 0.2 m square standing 1.2 m, banded near the top in the local
            // colour, sunk into the ground.
            // Packed round with a stone below ground, which also gives it a voxel to stand
            // among the stamps by.
            k.vox(0, -FOOTING, 0, p.stone);
            k.b((1, -FOOTING * CELLS + CELLS, 1), (3, 8, 3), mat::PLANK_DARK);
            k.b((1, 8, 1), (3, 10, 3), p.accent);
            k.b((1, 10, 1), (3, 12, 3), mat::PLANK_DARK);
        }
        Marker::Cairn => {
            // A core of stone with stones piled about its foot and smaller ones on top,
            // a metre or so high, now and then taller.
            let tall = rng.unit() < 0.3;
            let top = if tall { 1 } else { 0 };
            for y in -FOOTING..=top {
                k.vox(0, y, 0, if rng.unit() < 0.3 { p.stone_dark } else { p.stone });
            }
            let stone = |rng: &mut Rng| if rng.unit() < 0.35 { p.stone_dark } else { p.stone };
            // Round the foot: a stone on each of three or four sides.
            let sides: [((i32, i32, i32), (i32, i32, i32)); 4] =
                [((-2, 0, 0), (0, 2, 3)), ((4, 0, 1), (6, 2, 4)), ((1, 0, -2), (4, 2, 0)), ((0, 0, 4), (3, 2, 6))];
            let skip = rng.int(0, 5) as usize;
            for (n, &(lo, hi)) in sides.iter().enumerate() {
                if n != skip {
                    let v = stone(&mut rng);
                    k.b(lo, hi, v);
                }
            }
            // On top: a slab, a smaller stone and a capstone, each a little askew.
            let y = (top + 1) * CELLS;
            let v = stone(&mut rng);
            k.b((0, y, 0), (4, y + 2, 4), v);
            let (sx, sz) = (rng.int(0, 1), rng.int(0, 1));
            let v = stone(&mut rng);
            k.b((sx, y + 2, sz), (sx + 3, y + 4, sz + 3), v);
            let (cx, cz) = (rng.int(1, 2), rng.int(1, 2));
            let v = stone(&mut rng);
            k.b((cx, y + 4, cz), (cx + 1, y + 6, cz + 1), v);
        }
    }
    k
}

/// One marker chosen beside a trail and placed: absolute voxels and fine parts in
/// continent-local coordinates, and the columns it stands in.
pub struct Placed {
    pub kind: Marker,
    pub at: Vec2,
    /// Where model voxel 0 is: on the column it stands in, one above the ground.
    pub origin: IVec3,
    pub voxels: Vec<(IVec3, Voxel)>,
    pub fine: Vec<PlantBox>,
    pub footprint: Vec<IVec2>,
}

/// Trail steps between milestones: about 800 m.
const MILE: usize = 1000;
/// Trail steps between benches, before jitter.
const BENCH_GAP: usize = 1700;
/// Trail steps between waymarks, before jitter.
const WAYMARK_GAP: usize = 220;
/// Trail steps between cairns over stony ground, before jitter.
const CAIRN_GAP: usize = 180;
/// Fewest trail steps between any two markers on one trail.
const MIN_GAP: usize = 30;

/// Whether a trail crosses stony ground at a column of the bare land: rock, scree, the
/// high bare country, or anywhere as high as trails have thinned to mountain paths.
pub(super) fn stony(col: &Column) -> bool {
    matches!(col.cover, Cover::Cliff | Cover::Scree | Cover::Shingle | Cover::Boulders)
        || matches!(col.biome, Biome::Alpine | Biome::Tundra | Biome::AlpineMeadow | Biome::Volcanic | Biome::RedRock)
        || col.ground > super::build::HIGH.1
}

/// Everything about one laid trail that the markers are chosen from.
pub struct Trail<'a> {
    pub pts: &'a [Vec2],
    /// Half-width its traffic calls for at each point.
    pub traffic: &'a [f32],
    /// Half-width after narrowing over high and steep ground.
    pub widths: &'a [f32],
    /// Half-width actually worn: zero in the gaps of a trace.
    pub worn: &'a [f32],
    pub heights: &'a [f32],
    /// The bare land under each point.
    pub cols: &'a [Column],
    pub carried: &'a [(usize, usize)],
    pub salt: u32,
}

impl Trail<'_> {
    pub(super) fn carried(&self, i: usize) -> bool {
        self.carried.iter().any(|&(a, b)| (a.saturating_sub(6)..=b + 6).contains(&i))
    }

    /// A faint stretch: a trace by its traffic, or a busier trail thinned to a
    /// mountain path.
    fn faint(&self, i: usize) -> bool {
        class_of(self.widths[i]) == TrailClass::Trace || (self.widths[i] < self.traffic[i] * 0.7 && self.widths[i] < m(0.6))
    }

    fn main(&self, i: usize) -> bool {
        matches!(class_of(self.traffic[i]), TrailClass::Track | TrailClass::Road) && !self.faint(i)
    }

    /// The jittered gap after a marker of some kind at step `i`.
    fn gap(&self, base: usize, kind: Marker, i: usize) -> usize {
        (base as f32 * (0.75 + 0.5 * unit(hash2(self.salt ^ 0x3a7f ^ kind as u32, i as i32, 5)))) as usize
    }

    /// Height of the ground that will be at `q` once the trail is down: the bare
    /// ground drawn toward the nearest point of this trail as its shoulder draws it.
    pub(super) fn ground_at(&self, bare: &Column, q: Vec2, i: usize) -> f32 {
        let (lo, hi) = (i.saturating_sub(24), (i + 25).min(self.pts.len()));
        let nearest = (lo..hi)
            .filter(|&k| self.worn[k] > 0.0)
            .min_by(|&a, &b| self.pts[a].distance_squared(q).total_cmp(&self.pts[b].distance_squared(q)));
        let Some(k) = nearest else { return bare.ground };
        let t = TrailPoint { x: self.pts[k].x, z: self.pts[k].y, y: self.heights[k], half_width: self.worn[k] };
        let d = self.pts[k].distance(q);
        bare.ground + (t.y - bare.ground) * shoulder_weight(&t, bare.ground, d)
    }

    /// Whether any column in `foot` is on the walked way of this trail near step `i`.
    pub(super) fn on_way(&self, foot: &[IVec2], i: usize) -> bool {
        let (lo, hi) = (i.saturating_sub(40), (i + 40).min(self.pts.len()));
        foot.iter().any(|c| {
            let c = c.as_vec2() + 0.5;
            (lo..hi).any(|k| self.pts[k].distance_squared(c) < (self.widths[k] + 0.5).powi(2))
        })
    }
}

/// Chooses and builds the markers beside one trail. `taken` are places already marked
/// (lanterns, and markers on trails laid before this one), kept clear of.
pub fn choose(c: &Continent, seed: u32, t: &Trail, taken: &[Vec2]) -> Vec<Placed> {
    let n = t.pts.len();
    let mut out: Vec<Placed> = Vec::new();
    if n < 40 {
        return out;
    }
    // Each kind's count starts part way through its gap, so short trails get some.
    let start = |base: usize, kind: Marker| (unit(hash2(t.salt ^ 0x51a7, kind as i32, 9)) * base as f32) as usize;
    let mut since_mile = start(MILE, Marker::Milestone);
    let mut since_bench = start(BENCH_GAP, Marker::Bench);
    let mut since_way = start(WAYMARK_GAP, Marker::Waymark);
    let mut since_cairn = start(CAIRN_GAP, Marker::Cairn);
    let (mut bench_gap, mut way_gap, mut cairn_gap) = (BENCH_GAP, WAYMARK_GAP, CAIRN_GAP);
    let mut last = 0usize;
    for i in 12..n - 12 {
        since_mile += 1;
        since_bench += 1;
        since_way += 1;
        since_cairn += 1;
        if i < last + MIN_GAP || t.carried(i) {
            continue;
        }
        let col = &t.cols[i];
        let rough = stony(col);
        // A trace fading out here: mark where it goes on.
        let fades = t.worn[i] > 0.0 && t.worn[i + 1] == 0.0;
        let kind = if t.main(i) && since_mile >= MILE {
            Marker::Milestone
        } else if t.main(i) && since_bench >= bench_gap && level(t, i) {
            Marker::Bench
        } else if t.faint(i) && (fades || since_cairn >= cairn_gap) && rough {
            Marker::Cairn
        } else if t.faint(i) && !rough && (fades || since_way >= way_gap) {
            Marker::Waymark
        } else {
            continue;
        };
        if col.water_surface().is_some() || col.river < m(4.8) {
            continue;
        }
        let Some(placed) = place(c, seed, t, i, kind, taken, &out) else { continue };
        match kind {
            Marker::Milestone => since_mile = 0,
            Marker::Bench => {
                since_bench = 0;
                bench_gap = t.gap(BENCH_GAP, kind, i);
            }
            Marker::Waymark => {
                since_way = 0;
                way_gap = t.gap(WAYMARK_GAP, kind, i);
            }
            Marker::Cairn => {
                since_cairn = 0;
                cairn_gap = t.gap(CAIRN_GAP, kind, i);
            }
        }
        last = i;
        out.push(placed);
    }
    out
}

/// Whether the trail runs level enough at step `i` for a bench.
fn level(t: &Trail, i: usize) -> bool {
    (t.heights[i + 8] - t.heights[i - 8]).abs() < 1.0
}

/// Builds a marker of `kind` beside step `i`, on whichever side has room, or `None`.
fn place(c: &Continent, seed: u32, t: &Trail, i: usize, kind: Marker, taken: &[Vec2], placed: &[Placed]) -> Option<Placed> {
    let tangent = (t.pts[i + 4] - t.pts[i - 4]).normalize_or_zero();
    if tangent == Vec2::ZERO {
        return None;
    }
    let normal = vec2(-tangent.y, tangent.x);
    let pick = hash2(t.salt ^ 0x3a80, i as i32, kind as i32);
    let first = if pick & 1 == 0 { 1.0 } else { -1.0 };
    // How far off the worn edge it stands.
    let off = match kind {
        Marker::Milestone => m(0.6),
        Marker::Bench => m(1.0),
        Marker::Waymark => m(0.5),
        Marker::Cairn => m(0.7),
    };
    for side in [first, -first] {
        let q = t.pts[i] + normal * side * (t.widths[i].max(t.worn[i]) + off);
        let clear = m(5.0);
        if taken.iter().chain(placed.iter().map(|p| &p.at)).any(|o| o.distance_squared(q) < clear * clear) {
            continue;
        }
        let (x, z) = (q.x.floor() as i32, q.y.floor() as i32);
        let bare = Sampler::new(c, seed, x, z, 1).column(x, z);
        if bare.water_surface().is_some() || bare.cover == Cover::Cliff || bare.river < m(4.8) {
            continue;
        }
        let ground = t.ground_at(&bare, q, i).floor() as i32;
        // The model's +X faces the trail; a bench's runs along it, the trail in front
        // of its seat at model -Z.
        let turns = match kind {
            Marker::Bench => turns_toward(tangent * side),
            _ => turns_toward(-normal * side),
        };
        let biome = bare.biome;
        let palette = super::build::palette_at(c, seed, biome, x, ground, z).varied(hash2(seed ^ 0x3a81, x >> 6, z >> 6));
        let m = model(kind, &palette, pick);
        let origin = IVec3::new(x, ground + 1, z);
        let voxels: Vec<(IVec3, Voxel)> = m.voxels.iter().map(|&(p, v)| (origin + rotate(p, turns), v)).collect();
        let fine = parts::place_fine(&m.fine, origin, turns);
        let mut footprint: Vec<IVec2> = voxels.iter().map(|(p, _)| IVec2::new(p.x, p.z)).collect();
        for b in &fine {
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
        // A bench wants level ground under both its blocks.
        if kind == Marker::Bench {
            let far = rotate(IVec3::new(2, 0, 0), turns);
            let (fx, fz) = (x + far.x, z + far.z);
            let other = Sampler::new(c, seed, fx, fz, 1).column(fx, fz);
            if other.water_surface().is_some() || t.ground_at(&other, vec2(fx as f32 + 0.5, fz as f32 + 0.5), i).floor() as i32 != ground {
                continue;
            }
        }
        return Some(Placed { kind, at: q, origin, voxels, fine, footprint });
    }
    None
}

/// Quarter turns that best point a model's +X along `d`.
pub(super) fn turns_toward(d: Vec2) -> u32 {
    let axes = [vec2(1.0, 0.0), vec2(0.0, 1.0), vec2(-1.0, 0.0), vec2(0.0, -1.0)];
    (0..4).max_by(|&a, &b| axes[a].dot(d).total_cmp(&axes[b].dot(d))).unwrap() as u32
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::{continent::SimConfig, world::GameWorld};

    fn world() -> GameWorld {
        GameWorld::with_config(3, SimConfig { size: 256 })
    }

    #[test]
    fn models_stand_on_a_sunk_footing() {
        let p = Palette::of(Biome::Grassland);
        for kind in [Marker::Milestone, Marker::Bench, Marker::Waymark, Marker::Cairn] {
            for seed in 0..20 {
                let m = model(kind, &p, seed);
                let lowest = m.voxels.iter().map(|(q, _)| q.y * CELLS).chain(m.fine.iter().map(|b| b.lo[1])).min().unwrap();
                assert_eq!(lowest, -FOOTING * CELLS, "{kind:?} footing");
                let top = m.voxels.iter().map(|(q, _)| (q.y + 1) * CELLS).chain(m.fine.iter().map(|b| b.hi[1])).max().unwrap();
                // Between half a metre and a metre and a half above the ground.
                assert!((5..=15).contains(&top), "{kind:?} stands {top} cells tall");
                for b in &m.fine {
                    assert!((0..3).all(|a| b.lo[a] < b.hi[a]), "{kind:?} empty part {b:?}");
                }
            }
        }
    }

    #[test]
    fn markers_follow_the_traffic_and_keep_off_the_way() {
        let w = world();
        let c = w.continent(0, 0);
        let net = &c.trails;
        let mut kinds: HashMap<Marker, usize> = HashMap::new();
        for (k, _) in &net.markers {
            *kinds.entry(*k).or_insert(0) += 1;
        }
        println!("markers: {kinds:?}");
        assert!(kinds.get(&Marker::Milestone).copied().unwrap_or(0) > 0, "no milestones");
        assert!(kinds.get(&Marker::Cairn).copied().unwrap_or(0) + kinds.get(&Marker::Waymark).copied().unwrap_or(0) > 0, "no marks on faint trails");
        // Each stands by the trail it marks (beside where it is worn, or where it has
        // faded out), never on the worn way, and on a stretch of the right traffic: main
        // routes for milestones and benches, faint ones for waymarks and cairns.
        let all: Vec<_> = net.points().collect();
        let nearest = |q: glam::Vec2, paved: bool| {
            *all.iter()
                .filter(|p| !paved || p.half_width > 0.0)
                .min_by(|a, b| glam::vec2(a.x, a.z).distance(q).total_cmp(&glam::vec2(b.x, b.z).distance(q)))
                .unwrap()
        };
        for &(kind, at) in &net.markers {
            let q = glam::vec2(at.x as f32 + 0.5, at.z as f32 + 0.5);
            let near = nearest(q, false);
            let d = glam::vec2(near.x, near.z).distance(q);
            assert!(d < m(3.2), "{kind:?} at {at} is {d} from its trail");
            let paved = nearest(q, true);
            let d = glam::vec2(paved.x, paved.z).distance(q);
            assert!(d > paved.half_width, "{kind:?} at {at} stands on the trail");
            if near.half_width > 0.0 {
                match kind {
                    Marker::Milestone | Marker::Bench => assert!(near.half_width >= m(0.5), "{kind:?} on a trail {} wide", near.half_width),
                    Marker::Waymark | Marker::Cairn => assert!(near.half_width < m(0.8), "{kind:?} on a trail {} wide", near.half_width),
                }
            }
        }
    }

    #[test]
    fn markers_stand_on_the_ground_the_world_makes() {
        // The ground a marker was set on is the ground the world generates there (or a
        // voxel off, which its footing covers), and it is the same every time.
        let (a, b) = (world(), world());
        let (ca, cb) = (a.continent(0, 0), b.continent(0, 0));
        assert_eq!(ca.trails.markers, cb.trails.markers);
        let (mut off, mut far) = (0, Vec::new());
        for &(kind, at) in &ca.trails.markers {
            let top = a.column(at.x, at.z).top();
            let diff = at.y - 1 - top;
            off += (diff != 0) as usize;
            if diff.abs() > 1 {
                far.push((kind, at, diff));
            }
        }
        let n = ca.trails.markers.len();
        println!("{off} of {n} markers a voxel or more off; further: {far:?}");
        assert!(far.len() * 100 <= n, "{} of {n} markers set more than a voxel off", far.len());
        assert!(off * 5 <= n, "{off} of {n} markers set a voxel off");
    }
}
