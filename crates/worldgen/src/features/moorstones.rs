//! The old stonework of the moor, in the same granite as its tors: lone standing stones,
//! rows of small stones leading off toward a tor, circles, the rings of low stones where
//! round huts stood (sometimes a few together inside the wall of a pound), and granite
//! crosses beside the old ways.
//!
//! Each tor cell (see [`super::tor`]) may hold one or two of them, laid out with its tor
//! and from the same sampler, so they cross chunk borders whole like the tor does. They
//! keep to open, fairly level moor, clear of the tor and its clitter, of water, and of the
//! trails (but for the crosses, which stand right beside them).

use std::f32::consts::{PI, TAU};

use glam::{IVec3, Vec2, Vec3, vec2};

use super::{
    shape::Rng,
    tor::{self, Block, CELL, Part, Tor},
};
use crate::{
    biome::Biome,
    continent::Continent,
    macro_layout::MACRO_SIZE,
    noise::{hash2, unit},
    scale::m,
    surface::{Column, Sampler},
};

/// The kinds of stonework.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Work {
    Row,
    Circle,
    Huts,
    Standing,
    Cross,
}

/// The share of moor cells that hold each kind, tried in this order.
const SHARES: [(Work, f32); 4] = [(Work::Row, 0.09), (Work::Circle, 0.06), (Work::Huts, 0.12), (Work::Standing, 0.1)];
/// The share of moor cells crossed by a trail that set a cross beside it.
const CROSS_SHARE: f32 = 0.3;
/// How far from the cell's tor any stonework keeps.
const TOR_CLEARANCE: f32 = m(45.0);

/// Adds the stonework of world cell `(gx, gz)` to `tor`, its tor (empty if it has none).
pub fn lay_out(seed: u32, c: &Continent, gx: i32, gz: i32, mut tor: Tor) -> Tor {
    let h = hash2(seed ^ 0x3005, gx, gz);
    let roll = unit(h);
    let work = {
        let mut r = roll;
        SHARES.iter().find_map(|&(w, share)| {
            if r < share {
                return Some(w);
            }
            r -= share;
            None
        })
    };
    let cross = unit(hash2(seed ^ 0x3006, gx, gz)) < CROSS_SHARE;
    if work.is_none() && !cross {
        return tor;
    }
    let (ox, oz) = (c.desc.mx * MACRO_SIZE, c.desc.mz * MACRO_SIZE);
    let (lx0, lz0) = (gx * CELL - ox, gz * CELL - oz);
    let margin = tor::REACH + 64;
    if lx0 < margin || lz0 < margin || lx0 + CELL > MACRO_SIZE - margin || lz0 + CELL > MACRO_SIZE - margin {
        return tor;
    }
    // Only on the moor: the grid cell at the middle of the cell must be moor.
    let cs = c.cell_size();
    let n = c.size as i32;
    let grid = |lx: i32, lz: i32| (((lz as f32 / cs) as i32).clamp(0, n - 1) * n + ((lx as f32 / cs) as i32).clamp(0, n - 1)) as usize;
    if c.biome[grid(lx0 + CELL / 2, lz0 + CELL / 2)] != Biome::Moor as u8 {
        return tor;
    }
    let s = Sampler::new(c, seed, lx0 - 8, lz0 - 8, CELL + 16);
    let col = |x: i32, z: i32| s.column(x - ox, z - oz);
    let mut rng = Rng::new(h.rotate_left(5));
    let o = tor.origin;
    let tor_at = (!tor.is_empty()).then(|| vec2(o.x as f32, o.z as f32));
    // Somewhere inside the cell, clear of its tor, on open level moor.
    let inset = m(30.0);
    let spot = |rng: &mut Rng, radius: f32, want: &dyn Fn(&Column) -> bool| -> Option<(i32, i32)> {
        for _ in 0..24 {
            let x = gx * CELL + rng.range(inset, CELL as f32 - inset) as i32;
            let z = gz * CELL + rng.range(inset, CELL as f32 - inset) as i32;
            if tor_at.is_some_and(|t| t.distance(vec2(x as f32, z as f32)) < TOR_CLEARANCE + radius) {
                continue;
            }
            if c.biome[grid(x - ox, z - oz)] != Biome::Moor as u8 {
                continue;
            }
            let k = col(x, z);
            if !want(&k) {
                continue;
            }
            // Level enough over the whole of it.
            let r = radius.max(m(1.0)) as i32;
            let around = [(r, 0), (-r, 0), (0, r), (0, -r)].map(|(dx, dz)| col(x + dx, z + dz));
            let (lo, hi) = around.iter().fold((k.ground, k.ground), |(lo, hi), a| (lo.min(a.ground), hi.max(a.ground)));
            if hi - lo > radius * 0.25 + m(0.8) || around.iter().any(|a| !tor::open(a)) {
                continue;
            }
            return Some((x, z));
        }
        None
    };
    let away = |k: &Column| tor::open(k) && k.clear > m(8.0);
    let mut stones = Vec::new();
    let mut works = Vec::new();
    match work {
        Some(Work::Row) => {
            if let Some((x, z)) = spot(&mut rng, m(6.0), &away) {
                // Toward the nearest tor of any size, if one is near; else any way.
                let toward = nearest_tor(seed, c, gx, gz, vec2(x as f32, z as f32));
                row(&mut rng, &col, &mut stones, x, z, toward);
                works.push((Work::Row, x, z));
            }
        }
        Some(Work::Circle) => {
            if let Some((x, z)) = spot(&mut rng, m(14.0), &away) {
                circle(&mut rng, &col, &mut stones, x, z);
                works.push((Work::Circle, x, z));
            }
        }
        Some(Work::Huts) => {
            if let Some((x, z)) = spot(&mut rng, m(18.0), &away) {
                huts(&mut rng, &col, &mut stones, x, z);
                works.push((Work::Huts, x, z));
            }
        }
        Some(Work::Standing) => {
            if let Some((x, z)) = spot(&mut rng, m(2.0), &away) {
                let tall = m(rng.range(1.8, 3.6));
                standing(&mut rng, &col, &mut stones, x, z, tall);
                works.push((Work::Standing, x, z));
            }
        }
        _ => {}
    }
    if cross
        && let Some((x, z)) = spot(&mut rng, m(1.0), &|k: &Column| tor::open(k) && (m(1.2)..m(3.5)).contains(&k.clear))
    {
        granite_cross(&mut rng, &col, &mut stones, x, z);
        works.push((Work::Cross, x, z));
    }
    for (w, x, z) in works {
        tor.works.push((w, IVec3::new(x, col(x, z).top(), z)));
    }
    for (c, e, yaw, part_round) in stones {
        let local = c - o.as_vec3();
        tor.push(Block::new(local, e, yaw, part_round, e.y * 0.3, e.y * 0.15, 0.35, Part::Worked));
    }
    tor
}

/// One stone: world middle, half extents, turn, and plan rounding.
type Stone = (Vec3, Vec3, f32, f32);

/// The world position of the nearest tor's summit to `p`, within a few cells.
fn nearest_tor(seed: u32, c: &Continent, gx: i32, gz: i32, p: Vec2) -> Option<Vec2> {
    let (ox, oz) = (c.desc.mx * MACRO_SIZE, c.desc.mz * MACRO_SIZE);
    let mut best: Option<(f32, Vec2)> = None;
    for dz in -2..=2 {
        for dx in -2..=2 {
            if let Some(s) = tor::site(seed, c, gx + dx, gz + dz) {
                let at = s.at + vec2(ox as f32, oz as f32);
                let d = at.distance(p);
                if d > m(40.0) && d < m(600.0) && best.is_none_or(|b| d < b.0) {
                    best = Some((d, at));
                }
            }
        }
    }
    best.map(|b| b.1)
}

/// A stone set upright in the ground at `(x, z)`, `tall` voxels out of it.
#[allow(clippy::too_many_arguments)]
fn set_stone(col: &impl Fn(i32, i32) -> Column, out: &mut Vec<Stone>, x: f32, z: f32, e: Vec2, tall: f32, yaw: f32, round: f32) -> bool {
    let k = col(x.floor() as i32, z.floor() as i32);
    if !tor::open(&k) || k.clear < m(1.0) {
        return false;
    }
    // Bedded in: a fifth of it below the turf.
    let half = tall * 0.6;
    let foot = k.ground.floor() + 1.0 - tall * 0.2;
    out.push((Vec3::new(x, foot + half, z), Vec3::new(e.x, half, e.y), yaw, round));
    true
}

fn standing(rng: &mut Rng, col: &impl Fn(i32, i32) -> Column, out: &mut Vec<Stone>, x: i32, z: i32, tall: f32) {
    let e = vec2(m(rng.range(0.4, 0.65)), m(rng.range(0.22, 0.38)));
    set_stone(col, out, x as f32 + 0.5, z as f32 + 0.5, e, tall, rng.range(0.0, PI), e.y * 0.6);
}

/// A row of small stones, single or double, leading away from a standing stone at its
/// head toward `toward`.
fn row(rng: &mut Rng, col: &impl Fn(i32, i32) -> Column, out: &mut Vec<Stone>, x: i32, z: i32, toward: Option<Vec2>) {
    let start = vec2(x as f32 + 0.5, z as f32 + 0.5);
    let dir = toward.map_or_else(
        || {
            let a = rng.range(0.0, TAU);
            vec2(a.cos(), a.sin())
        },
        |t| (t - start).normalize_or(Vec2::X),
    );
    let yaw = dir.y.atan2(dir.x);
    let side = vec2(-dir.y, dir.x);
    let count = rng.int(10, 26);
    let spacing = m(rng.range(1.3, 2.1));
    let double = rng.unit() < 0.35;
    let width = m(rng.range(1.2, 1.8));
    // The blocking stone at its head.
    let tall = m(rng.range(1.4, 2.4));
    standing(rng, col, out, x, z, tall);
    let mut missed = 0;
    for k in 1..=count {
        let wander = (k as f32 * 0.35).sin() * m(0.4);
        let at = start + dir * (spacing * k as f32 + m(1.0)) + side * wander;
        for lane in 0..if double { 2 } else { 1 } {
            let p = at + side * width * lane as f32;
            let tall = m(rng.range(0.35, 0.9));
            let e = vec2(m(rng.range(0.2, 0.32)), m(rng.range(0.15, 0.25)));
            // The odd stone is gone, taken for a wall or a gatepost.
            if rng.unit() < 0.12 {
                continue;
            }
            if !set_stone(col, out, p.x, p.y, e, tall, yaw + rng.range(-0.3, 0.3), e.y * 0.5) {
                missed += 1;
            }
        }
        if missed > 3 {
            break;
        }
    }
}

/// A ring of upright stones.
fn circle(rng: &mut Rng, col: &impl Fn(i32, i32) -> Column, out: &mut Vec<Stone>, x: i32, z: i32) {
    let centre = vec2(x as f32 + 0.5, z as f32 + 0.5);
    let r = m(rng.range(6.0, 13.0));
    let count = ((TAU * r / m(rng.range(2.4, 3.4))) as i32).clamp(10, 26);
    let turn = rng.range(0.0, TAU);
    for k in 0..count {
        let a = turn + k as f32 / count as f32 * TAU + rng.range(-0.05, 0.05);
        let p = centre + vec2(a.cos(), a.sin()) * r;
        let roll = rng.unit();
        let e = vec2(m(rng.range(0.35, 0.55)), m(rng.range(0.22, 0.32)));
        if roll < 0.12 {
            continue;
        }
        if roll < 0.24 {
            // Fallen, lying in the grass.
            set_stone(col, out, p.x, p.y, vec2(m(rng.range(0.6, 0.9)), e.x), m(0.55), a, e.y * 0.6);
            continue;
        }
        set_stone(col, out, p.x, p.y, e, m(rng.range(0.8, 1.6)), a + PI * 0.5, e.y * 0.6);
    }
}

/// A few rings of low stones where round huts stood, inside the wall of a pound when there
/// are enough of them.
fn huts(rng: &mut Rng, col: &impl Fn(i32, i32) -> Column, out: &mut Vec<Stone>, x: i32, z: i32) {
    let centre = vec2(x as f32 + 0.5, z as f32 + 0.5);
    let count = rng.int(1, 5);
    let mut rings: Vec<(Vec2, f32)> = Vec::new();
    for _ in 0..count * 4 {
        if rings.len() >= count as usize {
            break;
        }
        let a = rng.range(0.0, TAU);
        let p = if rings.is_empty() { centre } else { centre + vec2(a.cos(), a.sin()) * m(rng.range(5.0, 13.0)) };
        let r = m(rng.range(2.4, 4.0));
        if rings.iter().any(|&(q, s)| q.distance(p) < r + s + m(1.5)) {
            continue;
        }
        rings.push((p, r));
        let (tall, door) = (m(rng.range(0.3, 0.6)), rng.range(0.0, TAU));
        ring_wall(rng, col, out, p, r, tall, Some(door));
    }
    if rings.len() >= 3 {
        let r = rings.iter().map(|&(q, s)| q.distance(centre) + s).fold(0.0f32, f32::max) + m(rng.range(4.0, 7.0));
        let (tall, door) = (m(rng.range(0.4, 0.75)), rng.range(0.0, TAU));
        ring_wall(rng, col, out, centre, r, tall, Some(door));
    }
}

/// A ring of low stones packed side by side, with a gap for a doorway facing `door`.
fn ring_wall(rng: &mut Rng, col: &impl Fn(i32, i32) -> Column, out: &mut Vec<Stone>, centre: Vec2, r: f32, tall: f32, door: Option<f32>) {
    let count = ((TAU * r / m(0.85)) as i32).max(8);
    for k in 0..count {
        let a = k as f32 / count as f32 * TAU;
        if door.is_some_and(|d| ((a - d + PI).rem_euclid(TAU) - PI).abs() < m(0.7) / r) {
            continue;
        }
        if rng.unit() < 0.1 {
            continue;
        }
        let p = centre + vec2(a.cos(), a.sin()) * (r + rng.range(-0.4, 0.4));
        let e = vec2(m(rng.range(0.3, 0.45)), m(rng.range(0.25, 0.35)));
        set_stone(col, out, p.x, p.y, e, tall * rng.range(0.7, 1.3), a + PI * 0.5, e.y * 0.7);
    }
}

/// A granite cross: a squared shaft with short arms, beside an old way.
fn granite_cross(rng: &mut Rng, col: &impl Fn(i32, i32) -> Column, out: &mut Vec<Stone>, x: i32, z: i32) {
    let k = col(x, z);
    let (px, pz) = (x as f32 + 0.5, z as f32 + 0.5);
    let yaw = rng.range(0.0, PI);
    let tall = m(rng.range(1.6, 2.4));
    let (w, d) = (m(rng.range(0.22, 0.3)), m(rng.range(0.15, 0.2)));
    let foot = k.ground.floor() + 1.0 - m(0.3);
    let shaft = tall + m(0.3);
    out.push((Vec3::new(px, foot + shaft * 0.5, pz), Vec3::new(w, shaft * 0.5, d), yaw, 0.3));
    let arm_y = foot + shaft - m(0.55);
    let (cos, sin) = (yaw.cos(), yaw.sin());
    let span = m(rng.range(0.42, 0.55));
    for dir in [-1.0f32, 1.0] {
        let off = vec2(cos, sin) * (w + span * 0.5) * dir;
        out.push((Vec3::new(px + off.x, arm_y, pz + off.y), Vec3::new(span * 0.5 + 0.3, m(0.2), d), yaw, 0.2));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::Cover;

    fn flat(_: i32, _: i32) -> Column {
        Column { ground: 40.0, water: None, cover: Cover::Land, biome: Biome::Moor, temperature: 0.35, river: f32::MAX, clear: f32::MAX, worn: None }
    }

    #[test]
    fn stones_stand_bedded_in_the_ground() {
        let mut rng = Rng::new(9);
        let mut out = Vec::new();
        row(&mut rng, &flat, &mut out, 100, 100, Some(vec2(300.0, 100.0)));
        circle(&mut rng, &flat, &mut out, 400, 100);
        huts(&mut rng, &flat, &mut out, 800, 100);
        assert!(out.len() > 30);
        let mut cross = Vec::new();
        granite_cross(&mut rng, &flat, &mut cross, 1200, 100);
        assert_eq!(cross.len(), 3);
        // The shaft is bedded in, and the arms stand well up it.
        assert!(cross[0].0.y - cross[0].1.y < 41.0 && cross[1].0.y > 44.0);
        for (c, e, _, _) in &out {
            // The top voxel of the ground is 40: every stone reaches down into it.
            assert!(c.y - e.y < 41.0, "a stone floats: {c} {e}");
            assert!(c.y + e.y > 41.0, "a stone is buried: {c} {e}");
        }
        // The row runs toward the tor.
        let along = out.iter().take(12).map(|(c, _, _, _)| c.x).fold(0.0f32, f32::max);
        assert!(along > 120.0);
    }
}
