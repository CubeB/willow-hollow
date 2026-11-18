//! Ditches (#64): about an old settlement on marshy or wet ground, generations of digging
//! have drained the land. Straight ditches holding water are cut on a grid of the
//! place's own, square to one another, and the fields are the long plots between them: a
//! fen's ditched fields, where the ditch is the field's edge and no hedge is needed. Where
//! a lane crosses a ditch it goes over on a plank bridge.
//!
//! A stretch of ditch runs between two crossings of the grid wherever the plot on either
//! side of it is drained, so a ditch never stops in the middle of a field: at the edge of
//! the drained land it runs on to the next crossing and ends there. Its water stands a
//! voxel below the lie of the land, two voxels deep, and steps down by a voxel now and
//! then as the land falls. The spoil was thrown up beside it: the ground along a ditch is
//! never so low that its water could spill, so the ditch stands between low banks where
//! the land dips. A ditch stops short of a way, of a building's ground and of a river's
//! channel, its end banked up.
//!
//! Like a paddy's, a ditch is decided from the column alone (see [`super::water`]).

use glam::{Vec2, vec2};
use voxel::{
    Voxel,
    mesh::{CELLS, PlantBox},
};

use super::{
    fields::{Found, Plot, crop_for, roll},
    influence::{Farm, PASTURE, ROUGH, sample},
    paddy,
    water::Shape,
};
use crate::{
    biome::Biome,
    continent::Continent,
    macro_layout::MACRO_SIZE,
    noise::{hash2, hash3, unit},
    scale::m,
    settlements::{Age, Region},
    surface::{Column, Cover, MIN_RIVER_Q},
};

/// How far apart the ditches are, each way, in voxels: the long side of a field and its
/// short side.
pub const SPACE: (f32, f32) = (72.0, 46.0);
/// Half the width of a ditch's water, in voxels.
const HALF: f32 = 1.0;
/// Half the width of a ditch with its banks.
const BANK: f32 = 2.3;
/// How deep the water in a ditch is, in voxels.
pub const DEPTH: i32 = 2;
/// How far a ditch keeps from a way's line, from a river's channel and from the houses.
const WAY: f32 = m(0.6);
const RIVER: f32 = m(3.0);
const HOUSES: f32 = m(4.0);
/// How long the bank is that closes a ditch's end where it stops short, in voxels.
const END: f32 = 1.5;
/// The steepest ground drained by ditches, in voxels of rise per voxel.
const FLAT: f32 = 0.05;

/// Whether `farm` drains its wet ground: an ancient place, or an established one in the
/// marshes; never one that grows rice, whose wet floors are paddies.
pub fn drains(farm: &Farm) -> bool {
    !paddy::grows(farm)
        && match farm.age {
            Age::Ancient => true,
            Age::Established => farm.region == Region::Wetland,
            Age::Young => false,
        }
}

/// The two ways `farm`'s ditches run, as unit vectors: the first along the long side of
/// its fields.
fn frame(farm: &Farm) -> (Vec2, Vec2) {
    let u = Vec2::from_angle(unit(hash2(farm.seed, 0xd17c, 3)) * std::f32::consts::FRAC_PI_2);
    (u, vec2(-u.y, u.x))
}

/// Continent-local `p` on `farm`'s grid of ditches, in voxels along each way.
fn local(farm: &Farm, p: Vec2) -> Vec2 {
    let (u, v) = frame(farm);
    let d = p - farm.at;
    vec2(d.dot(u), d.dot(v))
}

/// The continent-local point `(s, t)` plots across `farm`'s grid.
fn point(farm: &Farm, s: f32, t: f32) -> Vec2 {
    let (u, v) = frame(farm);
    farm.at + u * (s * SPACE.0) + v * (t * SPACE.1)
}

/// Whether plot `(a, b)` of the grid of the settlement `owner` is drained: well inside
/// the land it works and off the houses, flat, and marshy or the wet floor of a valley.
fn drained(c: &Continent, owner: usize, a: i32, b: i32) -> bool {
    let land = &c.land;
    let farm = &land.farms[owner];
    let span = MACRO_SIZE as f32;
    for (s, t) in [(0.5, 0.5), (0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
        let q = point(farm, a as f32 + s, b as f32 + t);
        if q.x < 0.0 || q.y < 0.0 || q.x >= span || q.y >= span {
            return false;
        }
        if land.owner(q.x as i32, q.y as i32) != Some(owner) || land.managed_at(q) < ROUGH || farm.at.distance(q) < farm.radius + m(8.0) {
            return false;
        }
    }
    let mid = point(farm, a as f32 + 0.5, b as f32 + 0.5);
    if land.managed_at(mid) < PASTURE {
        return false;
    }
    let g = sample(c, mid);
    if g.water || g.slope > FLAT {
        return false;
    }
    let marsh = matches!(g.biome, Biome::Marsh | Biome::Bog | Biome::Mangrove) || farm.region == Region::Wetland;
    let floor = g.river >= MIN_RIVER_Q && g.slope < 0.03;
    (marsh || floor) && unit(hash2(farm.seed ^ 0xd17c, a, b)) < 0.85
}

/// The field between the ditches that continent-local `p` lies in, of the settlement
/// `owner` whose land it is, if the ground there is drained (see [`super::fields::locate`]).
pub(super) fn field(c: &Continent, _seed: u32, p: Vec2, owner: usize) -> Option<Found> {
    let farm = &c.land.farms[owner];
    if !drains(farm) {
        return None;
    }
    let w = local(farm, p);
    let (a, b) = ((w.x / SPACE.0).floor() as i32, (w.y / SPACE.1).floor() as i32);
    if !drained(c, owner, a, b) {
        return None;
    }
    let site = point(farm, a as f32 + 0.5, b as f32 + 0.5);
    let managed = c.land.managed_at(site);
    let key = c.desc.seed ^ 0xd17c_4e5;
    let crop = crop_for(c, farm, site, managed, roll(key, a, b, owner as i32));
    let id = hash3(key, a, b, owner as i32) | 1;
    let edge = (w.x - a as f32 * SPACE.0).min((a + 1) as f32 * SPACE.0 - w.x).min(w.y - b as f32 * SPACE.1).min((b + 1) as f32 * SPACE.1 - w.y);
    let plot = Plot { id, crop, rows: frame(farm).0, managed, owner, terrace: None };
    Some(Found { plot, field: id, p, q: p, site, edge })
}

/// How far continent-local `p` is from the water's middle of the nearest ditch of the
/// settlement `owner`, if one is within a ditch and its banks of it.
fn nearest(c: &Continent, owner: usize, p: Vec2) -> Option<f32> {
    let farm = &c.land.farms[owner];
    let w = local(farm, p);
    let (au, bv) = ((w.x / SPACE.0).round(), (w.y / SPACE.1).round());
    let (du, dv) = ((w.x - au * SPACE.0).abs(), (w.y - bv * SPACE.1).abs());
    if du >= BANK && dv >= BANK {
        return None;
    }
    let mut best = f32::MAX;
    // The stretches of the ditch along a line of the grid near enough, each between two
    // crossings, and each there if the plot either side of it is drained.
    let stretches = |at: f32, space: f32| ((at - BANK) / space).floor() as i32..=((at + BANK) / space).floor() as i32;
    let beyond = |at: f32, k: i32, space: f32| (k as f32 * space - at).max(at - (k + 1) as f32 * space).max(0.0);
    if du < BANK {
        let a = au as i32;
        for b in stretches(w.y, SPACE.1) {
            if drained(c, owner, a - 1, b) || drained(c, owner, a, b) {
                best = best.min(du.hypot(beyond(w.y, b, SPACE.1)));
            }
        }
    }
    if dv < BANK {
        let b = bv as i32;
        for a in stretches(w.x, SPACE.0) {
            if drained(c, owner, a, b - 1) || drained(c, owner, a, b) {
                best = best.min(dv.hypot(beyond(w.x, a, SPACE.0)));
            }
        }
    }
    (best < BANK).then_some(best)
}

/// How the column at continent-local `(lx, lz)` of `c`, in the land of the settlement
/// `owner` and whose ground is `col`, lies by a ditch, if it does: cut down under the
/// ditch's water, or banked up beside it. `seed` is the world's.
pub(super) fn cut(c: &Continent, seed: u32, lx: i32, lz: i32, owner: usize, col: &Column) -> Option<Shape> {
    if col.cover != Cover::Land {
        return None;
    }
    let farm = &c.land.farms[owner];
    let p = vec2(lx as f32 + 0.5, lz as f32 + 0.5);
    let dist = nearest(c, owner, p)?;
    let keep = (col.clear - WAY).min(col.river - RIVER).min(farm.at.distance(p) - farm.radius - HOUSES);
    if keep < 0.0 {
        return None;
    }
    // The top voxel of the water: a voxel under the lie of the land.
    let level = crate::surface::natural_at(c, seed, p, false).floor() as i32 - 1;
    if dist < HALF && keep >= END {
        return Some(Shape { ground: (level - DEPTH) as f32 + 0.5, water: Some(level + 1) });
    }
    // The bank: never lower than the water beside it, which can stand a voxel higher
    // than this column's own level.
    (col.top() < level + 1).then_some(Shape { ground: (level + 1) as f32 + 0.5, water: None })
}

/// Whether column `col` is a ditch's water (and not a paddy's, which is a voxel deep).
pub fn ditched(col: &Column) -> bool {
    paddy::flooded(col) && col.water_surface().is_some_and(|k| k - 1 - col.top() == DEPTH)
}

/// Whether the column at continent-local `(lx, lz)` of `c`, whose ground is `col`, is a
/// way where it crosses a ditch: laid with the planks of a bridge.
pub fn bridged(c: &Continent, lx: i32, lz: i32, col: &Column) -> bool {
    if col.cover != Cover::Trail {
        return false;
    }
    let Some(owner) = c.land.owner(lx, lz) else { return false };
    if !drains(&c.land.farms[owner]) {
        return false;
    }
    nearest(c, owner, vec2(lx as f32 + 0.5, lz as f32 + 0.5)).is_some_and(|d| d < HALF + 0.5)
}

/// Whether continent-local `p` lies on a ditch of the settlement `owner`: for finding
/// where the ways cross them.
#[cfg(test)]
pub(super) fn crossing(c: &Continent, owner: usize, p: Vec2) -> bool {
    nearest(c, owner, p).is_some_and(|d| d < HALF)
}

/// The rails of a plank bridge on the column at world `(x, z)`, whose deck is its top
/// voxel `top`: on each side `open` says the way's edge is on (+x, -x, +z, -z), a post and
/// a rail along it, as small boxes into `out`. `wood` is the timber of the place.
pub fn rails(x: i32, top: i32, z: i32, open: [bool; 4], wood: Voxel, out: &mut Vec<PlantBox>) {
    let origin = [x * CELLS, (top + 1) * CELLS, z * CELLS];
    let mut add = |lo: [i32; 3], hi: [i32; 3], voxel: Voxel| {
        out.push(PlantBox {
            lo: [origin[0] + lo[0], origin[1] + lo[1], origin[2] + lo[2]],
            hi: [origin[0] + hi[0], origin[1] + hi[1], origin[2] + hi[2]],
            voxel,
            sway_lo: false,
            sway_hi: false,
            thin: 0,
            season: 0,
            flap: 0,
        });
    };
    for (k, &side) in open.iter().enumerate() {
        if !side {
            continue;
        }
        // The edge of the column the side is on, a cell in from it.
        let (x0, x1, z0, z1) = match k {
            0 => (CELLS - 1, CELLS, 0, CELLS),
            1 => (0, 1, 0, CELLS),
            2 => (0, CELLS, CELLS - 1, CELLS),
            _ => (0, CELLS, 0, 1),
        };
        // A post at its middle, and the rail along it at hand height.
        let (mx, mz) = if k < 2 { (x0, CELLS / 2 - 1) } else { (CELLS / 2 - 1, z0) };
        add([mx, 0, mz], [mx + 1, 7, mz + 1], wood);
        add([x0, 6, z0], [x1, 7, z1], wood);
    }
}

#[cfg(test)]
mod tests {
    use voxel::{CHUNK, PAD};

    use super::*;
    use crate::{World, continent::SimConfig, surface::Sampler, world::GameWorld};

    /// A world with drained land, and its ditched plots' middles.
    pub(crate) fn drained_land() -> Option<(GameWorld, Vec<(usize, Vec2)>)> {
        for seed in 1..40 {
            let w = GameWorld::with_config(seed, SimConfig { size: 256 });
            let c = w.continent(0, 0);
            let mut out = Vec::new();
            for (k, f) in c.land.farms.iter().enumerate().filter(|(_, f)| drains(f)) {
                let n = ((f.radius + f.reach) / SPACE.1).ceil() as i32 + 1;
                for a in -n..=n {
                    for b in -n..=n {
                        if drained(&c, k, a, b) {
                            out.push((k, point(f, a as f32 + 0.5, b as f32 + 0.5)));
                        }
                    }
                }
            }
            if out.len() >= 4 {
                return Some((w, out));
            }
        }
        None
    }

    /// Ditches run straight, hold their water between banks no lower than it, step down a
    /// voxel at most from one column to the next, and the chunks either side of a border
    /// agree on them.
    #[test]
    fn ditches_are_straight_and_hold_their_water() {
        let (w, plots) = drained_land().expect("no world among the first seeds drains its land");
        let c = w.continent(0, 0);
        let (mut water, mut held) = (0, 0);
        let mut chunks = Vec::new();
        for &(owner, mid) in plots.iter().take(10) {
            let farm = &c.land.farms[owner];
            // Along the edge of the plot: a straight line of the grid.
            let corner = mid - frame(farm).0 * (SPACE.0 * 0.5) - frame(farm).1 * (SPACE.1 * 0.5);
            let (x0, z0) = (corner.x as i32 - 40, corner.y as i32 - 40);
            let n = 80;
            let sampler = Sampler::new(&c, w.seed(), x0 - 1, z0 - 1, n + 2);
            for i in 0..n {
                for j in 0..n {
                    let (x, z) = (x0 + i, z0 + j);
                    let col = sampler.column(x, z);
                    if !ditched(&col) {
                        continue;
                    }
                    water += 1;
                    // On a line of the grid: straight, wherever it is.
                    let o = c.land.owner(x, z).unwrap();
                    let l = local(&c.land.farms[o], vec2(x as f32 + 0.5, z as f32 + 0.5));
                    let off = |v: f32, s: f32| (v - (v / s).round() * s).abs();
                    assert!(off(l.x, SPACE.0) < HALF || off(l.y, SPACE.1) < HALF, "ditch water off the grid at {x},{z}");
                    let k = col.water_surface().unwrap();
                    for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                        let next = sampler.column(x + dx, z + dz);
                        match next.water_surface() {
                            Some(nk) => assert!((nk - k).abs() <= 1, "a ditch steps {} at {x},{z}", nk - k),
                            None => {
                                assert!(next.top() >= k - 1, "the ditch at {x},{z} spills over {},{}", x + dx, z + dz);
                                held += 1;
                            }
                        }
                    }
                    chunks.push((x.div_euclid(CHUNK), z.div_euclid(CHUNK)));
                }
            }
        }
        println!("{water} columns of ditch water, {held} faces held by banks");
        assert!(water > 100 && held > 50, "only {water} columns of ditch");
        chunks.sort();
        chunks.dedup();
        for &(cx, cz) in chunks.iter().take(6) {
            let a = w.generate_column(cx, cz);
            let b = w.generate_column(cx, cz + 1);
            for (cy, ca) in &a.chunks {
                let Some((_, cb)) = b.chunks.iter().find(|(y, _)| y == cy) else { continue };
                for x in 0..PAD {
                    for y in 0..PAD {
                        assert_eq!(ca.get(x, y, PAD - 2), cb.get(x, y, 0), "chunks {cx},{cz} and south differ at x {x} y {y} of {cy}");
                        assert_eq!(ca.get(x, y, PAD - 1), cb.get(x, y, 1), "chunks {cx},{cz} and south differ at x {x} y {y} of {cy}");
                    }
                }
            }
        }
    }

    /// Where a way crosses a ditch it is laid with planks, and rails stand along it.
    #[test]
    fn lanes_cross_ditches_on_plank_bridges() {
        let (w, _) = drained_land().expect("no world among the first seeds drains its land");
        let c = w.continent(0, 0);
        let mut bridges = 0;
        for (k, f) in c.land.farms.iter().enumerate().filter(|(_, f)| drains(f)) {
            let n = (f.radius + f.reach) as i32;
            let (x0, z0) = (f.at.x as i32 - n, f.at.y as i32 - n);
            let sampler = Sampler::new(&c, w.seed(), x0, z0, 2 * n);
            for x in (x0..x0 + 2 * n).step_by(2) {
                for z in (z0..z0 + 2 * n).step_by(2) {
                    if c.land.owner(x, z) != Some(k) {
                        continue;
                    }
                    let col = sampler.column(x, z);
                    if bridged(&c, x, z, &col) {
                        bridges += 1;
                        let (cx, cz) = (x.div_euclid(CHUNK), z.div_euclid(CHUNK));
                        if bridges <= 3 {
                            let out = w.generate_column(cx, cz);
                            let (cy, ly) = (col.top().div_euclid(CHUNK), col.top().rem_euclid(CHUNK));
                            let deck = out.chunks.iter().find(|(y, _)| *y == cy).map(|(_, ch)| ch.get((x - cx * CHUNK + 1) as usize, ly as usize + 1, (z - cz * CHUNK + 1) as usize));
                            assert_eq!(deck, Some(crate::materials::PLANK), "no deck at {x},{z}");
                        }
                    }
                }
            }
        }
        println!("{bridges} columns of plank bridge");
        assert!(bridges > 0, "no lane crosses a ditch");
    }
}
