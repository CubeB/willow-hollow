//! Where exactly a landmark stands, once the map has said roughly where (see
//! [`super::sites`]). The map picks a cell for a reason (a hilltop, a saddle, a spring,
//! a river bank); this looks at the real ground about that cell and scores each spot
//! by the same reason at walking scale: the brow with the widest view, the low point
//! of the pass, a few paces from the spring, a sheltered nook backed by rising ground
//! with water near. The best spot that can take the site's level pad wins.

use glam::Vec2;

use super::Reason;
use crate::{
    continent::Continent,
    features::landmarks::Landmark,
    scale::m,
    surface::{Column, Cover, Sampler},
};

/// How far from the cell's middle a site may move, in map cells.
const SEARCH_CELLS: f32 = 2.5;
/// And no further than this, on the coarse grids tests use.
const SEARCH_MAX: f32 = m(64.0);
/// Spacing of the grid of spots looked at, in voxels.
const STEP: i32 = 10;
/// Most spots tried for a level pad, best first, before giving up.
const TRIES: usize = 48;

/// Where a landmark settled, and why.
#[derive(Clone, Copy, Debug)]
pub struct Settled {
    pub x: i32,
    pub z: i32,
    /// Mean ground height under the pad.
    pub y: f32,
    pub reason: Reason,
    /// The nearest fresh water (a stream, a lake or a spring), if any is close enough to
    /// fetch from.
    pub water: Option<Vec2>,
}

/// Farthest fresh water counts as near enough to fetch from.
pub const FETCH: f32 = m(70.0);

/// Whether a column holds fresh water: a stream, a river or a lake, not the sea.
fn fresh(col: &Column) -> bool {
    col.water_surface().is_some_and(|k| k > 0) && col.cover != Cover::SeaFloor
}

/// The steepest rise a pad may straddle: a lookout may stand on a brow and be terraced
/// into it; the rest want gentler ground.
fn max_rise(kind: Landmark) -> f32 {
    match kind {
        Landmark::Lookout | Landmark::Cairn => m(7.2),
        _ => m(5.6),
    }
}

/// The level pad a site needs at `(x, z)`: its mean height, or None if water, a river
/// bank, a cliff or too steep a slope is in the way.
fn probe(s: &Sampler, kind: Landmark, x: i32, z: i32) -> Option<f32> {
    let r = kind.radius() as i32 + 1;
    let (mut lo, mut hi, mut sum) = (f32::MAX, f32::MIN, 0.0);
    let d = r * 2 / 3;
    let rim = [(0, 0), (r, 0), (-r, 0), (0, r), (0, -r), (d, d), (-d, d), (d, -d), (-d, -d)];
    for (dx, dz) in rim {
        let col = s.column(x + dx, z + dz);
        if col.water_surface().is_some() || col.river < m(4.8) || !matches!(col.cover, Cover::Land | Cover::Beach) {
            return None;
        }
        lo = lo.min(col.ground);
        hi = hi.max(col.ground);
        sum += col.ground;
    }
    (hi - lo <= max_rise(kind)).then_some(sum / rim.len() as f32)
}

/// One spot of the grid.
struct Spot {
    p: Vec2,
    ground: f32,
    /// Dry land a site could stand on.
    land: bool,
    fresh: bool,
    /// Distance to the edge of the nearest river channel: a brook too narrow to fall
    /// on the grid still shows here.
    river: f32,
}

/// Finds where a landmark can stand near a map cell, for a reason. None when no spot
/// near it can take a level pad.
pub fn settle(c: &Continent, seed: u32, kind: Landmark, cell: usize) -> Option<Settled> {
    let cs = c.cell_size();
    let n = c.size;
    let centre = Vec2::new(((cell % n) as f32 + 0.5) * cs, ((cell / n) as f32 + 0.5) * cs);
    let reach = (cs * SEARCH_CELLS).min(SEARCH_MAX) as i32 / STEP * STEP;
    let side = reach / STEP * 2 + 1;
    let (x0, z0) = (centre.x as i32 - reach, centre.y as i32 - reach);
    let pad = kind.radius() as i32 + 2;
    let sampler = Sampler::new(c, seed, x0 - pad, z0 - pad, reach * 2 + pad * 2 + 1);

    let mut spots = Vec::with_capacity((side * side) as usize);
    for j in 0..side {
        for i in 0..side {
            let (x, z) = (x0 + i * STEP, z0 + j * STEP);
            let col = sampler.column(x, z);
            spots.push(Spot {
                p: Vec2::new(x as f32 + 0.5, z as f32 + 0.5),
                ground: col.ground,
                land: col.water_surface().is_none() && col.river >= m(4.8) && matches!(col.cover, Cover::Land | Cover::Beach),
                fresh: fresh(&col),
                river: col.river,
            });
        }
    }
    let at = |i: i32, j: i32| -> Option<&Spot> {
        (i >= 0 && j >= 0 && i < side && j < side).then(|| &spots[(j * side + i) as usize])
    };
    // Fresh water: the wet spots of the grid, and the springs and stream heads near.
    // A spot within a few voxels of a channel's edge stands for a brook between spots.
    let mut water: Vec<(Vec2, bool)> = spots.iter().filter(|s| s.fresh || s.river < STEP as f32 * 0.6).map(|s| (s.p, false)).collect();
    water.extend(sampler.springs().iter().map(|s| (s.pos, true)));
    water.extend(sampler.heads().iter().map(|&h| (h, true)));
    let nearest_water = |p: Vec2| -> Option<(f32, Vec2, bool)> {
        water
            .iter()
            .map(|&(w, spring)| (w.distance(p), w, spring))
            .min_by(|a, b| a.0.total_cmp(&b.0))
    };
    // How many wet spots lie close about a point of water: few for a brook.
    let wet_about = |w: Vec2| water.iter().filter(|&&(q, spring)| !spring && q.distance_squared(w) < m(7.0).powi(2)).count();
    let ring_mean = |i: i32, j: i32, r0: f32, r1: f32| -> Option<f32> {
        let k = (r1 / STEP as f32).ceil() as i32;
        let (mut sum, mut count) = (0.0, 0);
        for dj in -k..=k {
            for di in -k..=k {
                let d = ((di * di + dj * dj) as f32).sqrt() * STEP as f32;
                if d < r0 || d > r1 {
                    continue;
                }
                if let Some(s) = at(i + di, j + dj) {
                    sum += s.ground;
                    count += 1;
                }
            }
        }
        (count > 4).then(|| sum / count as f32)
    };
    let ground_at = |i: i32, j: i32, dx: f32, dz: f32| -> Option<f32> {
        at(i + (dx / STEP as f32).round() as i32, j + (dz / STEP as f32).round() as i32).map(|s| s.ground)
    };

    // Score every spot that could take a pad, by the reason its kind wants.
    let mut scored: Vec<(f32, i32, i32, Reason)> = Vec::new();
    let rpad = (kind.radius() / STEP as f32).ceil() as i32;
    for j in 0..side {
        for i in 0..side {
            let s = &spots[(j * side + i) as usize];
            if !s.land || s.p.distance(centre) > reach as f32 {
                continue;
            }
            // Rough levelness over the pad, from the grid.
            let (mut lo, mut hi) = (s.ground, s.ground);
            let mut dry = true;
            for dj in -rpad..=rpad {
                for di in -rpad..=rpad {
                    if let Some(q) = at(i + di, j + dj) {
                        lo = lo.min(q.ground);
                        hi = hi.max(q.ground);
                        dry &= q.land;
                    }
                }
            }
            if !dry || hi - lo > max_rise(kind) * 1.2 {
                continue;
            }
            let rough = (hi - lo - m(2.0)).max(0.0);
            let off = s.p.distance(centre) / reach as f32;
            let here = s.ground;
            let prominence = ring_mean(i, j, m(15.0), m(45.0)).map_or(0.0, |mean| here - mean);
            let water_near = nearest_water(s.p);
            let dw = water_near.map_or(f32::MAX, |w| w.0);
            let (score, reason) = match kind {
                Landmark::Lookout | Landmark::Arch => (prominence * 2.0 - rough - off * 6.0, Reason::View),
                Landmark::Stones | Landmark::Ruins => {
                    let open = prominence.clamp(-m(4.0), m(6.0));
                    let reason = if prominence > m(2.0) { Reason::View } else { Reason::Open };
                    (open - rough * 2.0 - off * 6.0, reason)
                }
                Landmark::Cairn => {
                    // The low point between two higher shoulders, with the ground falling
                    // away along the way through.
                    let d = m(25.0);
                    let mut saddle = f32::MIN;
                    let h = std::f32::consts::FRAC_1_SQRT_2;
                    for (ax, az) in [(1.0, 0.0), (h, h), (0.0, 1.0), (-h, h)] {
                        let side_a = ground_at(i, j, ax * d, az * d);
                        let side_b = ground_at(i, j, -ax * d, -az * d);
                        let way_a = ground_at(i, j, -az * d, ax * d);
                        let way_b = ground_at(i, j, az * d, -ax * d);
                        if let (Some(a), Some(b), Some(c), Some(e)) = (side_a, side_b, way_a, way_b) {
                            saddle = saddle.max((a.min(b) - here).min(here - c.max(e)));
                        }
                    }
                    let reason = if saddle > 0.0 { Reason::Pass } else { Reason::View };
                    (saddle.max(-m(10.0)) * 2.0 + prominence * 0.3 - rough - off * 6.0, reason)
                }
                Landmark::Shrine => match water_near {
                    Some((d, _, spring)) if d < m(40.0) => {
                        let fit = -(d - m(9.0)).abs() * 0.5;
                        let reason = if spring { Reason::Spring } else { Reason::Water };
                        (fit + if spring { m(16.0) } else { 0.0 } - rough * 2.0 - off * 4.0, reason)
                    }
                    _ => (prominence - rough * 2.0 - off * 6.0 - m(20.0), if prominence > m(2.0) { Reason::View } else { Reason::Open }),
                },
                Landmark::Camp => match water_near {
                    Some((d, w, spring)) if d < m(50.0) => {
                        let fit = -(d - m(12.0)).abs() * 0.5;
                        let ford = !spring && wet_about(w) <= 4;
                        let reason = if ford { Reason::Ford } else { Reason::Water };
                        (fit + if ford { m(8.0) } else { 0.0 } - rough * 3.0 - off * 4.0, reason)
                    }
                    _ => (-rough * 3.0 - off * 6.0 - m(20.0), Reason::Open),
                },
                Landmark::Hut => {
                    // Backed by rising ground on one side, and water within a walk.
                    let d = m(22.0);
                    let mut shelter = 0.0f32;
                    for k in 0..8 {
                        let a = k as f32 * std::f32::consts::FRAC_PI_4;
                        if let Some(g) = ground_at(i, j, a.cos() * d, a.sin() * d) {
                            shelter = shelter.max((g - here).min(m(8.0)));
                        }
                    }
                    let fetch = if dw < FETCH { m(6.0) - (dw - m(20.0)).max(0.0) * 0.1 } else { 0.0 };
                    let reason = if shelter > m(2.5) {
                        Reason::Shelter
                    } else if dw < FETCH {
                        Reason::Water
                    } else {
                        Reason::Open
                    };
                    (shelter + fetch - rough * 2.0 - off * 5.0, reason)
                }
                Landmark::Landing => (0.0, Reason::Shore),
            };
            scored.push((score, i, j, reason));
        }
    }
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then((a.2, a.1).cmp(&(b.2, b.1))));
    for &(_, i, j, reason) in scored.iter().take(TRIES) {
        let s = &spots[(j * side + i) as usize];
        let (x, z) = (s.p.x as i32, s.p.y as i32);
        if let Some(y) = probe(&sampler, kind, x, z) {
            let water = nearest_water(s.p).filter(|w| w.0 < FETCH).map(|w| w.1);
            return Some(Settled { x, z, y, reason, water });
        }
    }
    // Nothing scored stood level: the first good ground out from the middle, as before.
    for ring in 0..12 {
        let r = ring as f32 * cs * 0.25;
        let steps = if ring == 0 { 1 } else { 8 + ring * 2 };
        for k in 0..steps {
            let a = k as f32 / steps as f32 * std::f32::consts::TAU;
            let (x, z) = ((centre.x + a.cos() * r) as i32, (centre.y + a.sin() * r) as i32);
            if !sampler.covers(x, z, pad) {
                continue;
            }
            if let Some(y) = probe(&sampler, kind, x, z) {
                let water = nearest_water(Vec2::new(x as f32 + 0.5, z as f32 + 0.5)).filter(|w| w.0 < FETCH).map(|w| w.1);
                return Some(Settled { x, z, y, reason: Reason::Open, water });
            }
        }
    }
    None
}
