//! Cloth that moves in the wind: ribbons tied by one end, and sheets tied at their
//! corners. (A string of prayer flags, on a line between two poles, is
//! `trails::delights::flags`.)
//!
//! Both are plant boxes in fine cells, a tenth of a metre ([`CELLS`] to a voxel), each of
//! which carries what the shader needs to move it (see [`Cloth`]): a length of ribbon how
//! far below its knot it hangs, a tile of a sheet how far each of its corners gives. Where
//! the wind takes them is worked out in the shader from that and from where they are
//! (`cloth_motion` in terrain.wgsl), so they cost nothing to keep moving. Like every fine
//! part they are not drawn past the plants' reach, and their shadows stand still, as a
//! prayer flag's do.

use glam::IVec3;
use voxel::{
    Voxel,
    mesh::{CELLS, Cloth, PlantBox, RIBBON_BENDS, SHEET_GIVE},
};

use super::shape::rotate;
use crate::scale::VOXELS_PER_METRE;

/// The longest ribbon, in cells (3 m): as far below its knot as a ribbon bends.
pub const RIBBON_LONGEST: i32 = RIBBON_BENDS;

/// A ribbon tied by one end and hanging from it, `length` cells of `colour` (at most
/// [`RIBBON_LONGEST`]) down from its knot. `knot` is the corner of the cell it is tied
/// at: the top of the ribbon, on the underside of a branch or a beam, at its low x and
/// low z side. It is two cells wide, along x or along z as `seed` has it, and one thick.
///
/// In still air it hangs straight down from the knot. In a breeze it streams out
/// downwind, further the harder it blows, with a wave running down it from the knot that
/// grows toward the free end, so the end flutters most. Each ribbon moves at its own
/// phase, which comes from where its knot is. Leave it room: in a storm it streams out at
/// a slant, up to its whole length from the knot, whichever way the wind blows.
pub fn ribbon(knot: IVec3, length: i32, colour: Voxel, seed: u32) -> Vec<PlantBox> {
    let length = length.clamp(1, RIBBON_LONGEST);
    let wide = if seed & 1 == 0 { IVec3::new(2, 0, 1) } else { IVec3::new(1, 0, 2) };
    // A cell at a time down from the knot, so that it can bend along its length; each
    // length knows how far below the knot it hangs, and only the last shows its foot.
    (0..length)
        .map(|k| {
            let top = knot - IVec3::new(0, k, 0);
            let lo = top - IVec3::Y;
            let hi = top + wide;
            PlantBox::ribbon(lo.to_array(), hi.to_array(), colour, k as u8, k == length - 1)
        })
        .collect()
}

/// A sheet of `colour` tied at its four corners: a sail shade across a courtyard, or an
/// awning over a door. `corners` are where it is tied, in fine cells (the corners of
/// cells, so a sheet from x = 0 to x = 40 is four metres across): the four corners of a
/// rectangle square to the axes, seen from above, in any order, each at its own height.
/// So a sheet can lie level, slope like an awning, or twist like a sail shade with two
/// corners high and two low. It is a cell thick, hanging below the line between its ties.
///
/// Its corners stay where they are tied. In still air its middle sags a little; in wind
/// the middle billows up and down, the more the bigger the sheet and the harder it
/// blows, and ripples run across it downwind, most toward its free edges.
///
/// It is cut into tiles that keep within one voxel each, a voxel square where the sheet
/// lies near level and narrower where it slopes, so that it steps by a cell at most from
/// one tile to the next. Each tile carries how far its corners give, so a sheet built in
/// a model that is then turned must be turned with [`turned`], as `parts::place_fine`
/// does; moved by whole voxels it stays as it is.
pub fn sheet(corners: [IVec3; 4], colour: Voxel) -> Vec<PlantBox> {
    let (x0, x1) = (corners.iter().map(|c| c.x).min().unwrap_or(0), corners.iter().map(|c| c.x).max().unwrap_or(0));
    let (z0, z1) = (corners.iter().map(|c| c.z).min().unwrap_or(0), corners.iter().map(|c| c.z).max().unwrap_or(0));
    if x1 <= x0 || z1 <= z0 {
        return Vec::new();
    }
    // The height of each corner of the rectangle: that of the tie nearest it.
    let tie = |x: i32, z: i32| corners.iter().min_by_key(|c| (c.x - x).abs() + (c.z - z).abs()).map_or(0, |c| c.y) as f32;
    let (h00, h10, h11, h01) = (tie(x0, z0), tie(x1, z0), tie(x1, z1), tie(x0, z1));
    let (w, d) = ((x1 - x0) as f32, (z1 - z0) as f32);
    let height = |x: f32, z: f32| {
        let (u, v) = ((x - x0 as f32) / w, (z - z0 as f32) / d);
        (h00 * (1.0 - u) + h10 * u) * (1.0 - v) + (h01 * (1.0 - u) + h11 * u) * v
    };
    // Tiles a voxel across where the sheet is near level, narrower along a steep slope.
    let step = |rise: f32, run: f32| match rise.abs() / run {
        s if s * CELLS as f32 <= 1.0 => CELLS,
        s if s * 2.0 <= 1.0 => 2,
        _ => 1,
    };
    let (sx, sz) = (step((h10 - h00).abs().max((h11 - h01).abs()), w), step((h01 - h00).abs().max((h11 - h10).abs()), d));
    let lines = |a: i32, b: i32, step: i32| {
        let mut v = vec![a];
        let mut k = (a.div_euclid(step) + 1) * step;
        while k < b {
            v.push(k);
            k += step;
        }
        v.push(b);
        v
    };
    let (xs, zs) = (lines(x0, x1, sx), lines(z0, z1, sz));
    let (nx, nz) = (xs.len() - 1, zs.len() - 1);
    // The top of each tile, where the sheet passes over its middle.
    let top: Vec<i32> = (0..nz)
        .flat_map(|j| (0..nx).map(move |i| (i, j)))
        .map(|(i, j)| height((xs[i] + xs[i + 1]) as f32 * 0.5, (zs[j] + zs[j + 1]) as f32 * 0.5).round() as i32)
        .collect();
    let at = |i: usize, j: usize| top[j * nx + i];
    let give = |x: i32, z: i32| sheet_give((x - x0) as f32 / w, (z - z0) as f32 / d);
    let span = sheet_span(w.min(d));
    let mut out = Vec::with_capacity(nx * nz);
    for j in 0..nz {
        for i in 0..nx {
            let y = at(i, j);
            // Up to meet a neighbour standing higher, so no light shows between them.
            let mut hi = y;
            for (di, dj) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                let (a, b) = (i as i32 + di, j as i32 + dj);
                if a >= 0 && b >= 0 && (a as usize) < nx && (b as usize) < nz {
                    hi = hi.max(at(a as usize, b as usize) - 1);
                }
            }
            let (xa, xb, za, zb) = (xs[i], xs[i + 1], zs[j], zs[j + 1]);
            let gives = [give(xa, za), give(xb, za), give(xb, zb), give(xa, zb)];
            out.push(PlantBox::sheet([xa, y - 1, za], [xb, hi, zb], colour, gives, span));
        }
    }
    out
}

/// How far a sheet gives at `(u, v)` across it, each 0 to 1, from 0 at its corners to
/// [`SHEET_GIVE`] in its middle: mostly a dome over the whole sheet, and some of it held
/// least anywhere away from the corners, so its free edges move too.
fn sheet_give(u: f32, v: f32) -> u8 {
    let (a, b) = ((u * std::f32::consts::PI).sin().max(0.0), (v * std::f32::consts::PI).sin().max(0.0));
    let e = 0.7 * a * b + 0.3 * (1.0 - (1.0 - a) * (1.0 - b));
    (e * SHEET_GIVE as f32).round().clamp(0.0, SHEET_GIVE as f32) as u8
}

/// How big a sheet is, for how far it billows: 0 for one under a metre and a half across
/// its narrower way, an awning, to 3 for one over five metres, a great sail shade.
fn sheet_span(narrow_cells: f32) -> u8 {
    let metres = narrow_cells / (VOXELS_PER_METRE * CELLS as f32);
    match metres {
        m if m < 1.5 => 0,
        m if m < 3.0 => 1,
        m if m < 5.0 => 2,
        _ => 3,
    }
}

/// The corners of a sheet's tile as [`Cloth::Sheet`] lists them, as which way along x and
/// along z each lies from the middle.
const CORNERS: [(i32, i32); 4] = [(-1, -1), (1, -1), (1, 1), (-1, 1)];

/// `b`, with what it carries turned by `turns` quarter turns as `parts::rotate_box` turns
/// its box: a tile of a sheet knows how far each of its corners gives, and they turn with
/// it. Anything else is as it was. Its box is left to the caller to turn.
pub fn turned(b: &PlantBox, turns: u32) -> PlantBox {
    let Cloth::Sheet { gives, span } = b.cloth() else { return *b };
    let mut out = [0u8; 4];
    for (k, &(x, z)) in CORNERS.iter().enumerate() {
        let r = rotate(IVec3::new(x, 0, z), turns);
        let to = CORNERS.iter().position(|&c| c == (r.x, r.z)).unwrap_or(k);
        out[to] = gives[k];
    }
    b.with_cloth(Cloth::Sheet { gives: out, span })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::{features::parts::place_fine, materials as mat};

    #[test]
    fn a_ribbon_hangs_from_its_knot_a_cell_at_a_time() {
        let knot = IVec3::new(5, 40, -3);
        let r = ribbon(knot, 12, mat::CLOTH_RED, 7);
        assert_eq!(r.len(), 12);
        for (k, b) in r.iter().enumerate() {
            assert_eq!(b.hi[1], knot.y - k as i32, "each length hangs from the one above");
            assert_eq!(b.hi[1] - b.lo[1], 1);
            assert_eq!(b.cloth(), Cloth::Ribbon { knot: k as u8, end: k == 11 });
            assert_eq!((b.lo[0], b.lo[2]), (knot.x, knot.z));
            assert_eq!((b.hi[0] - b.lo[0]) * (b.hi[2] - b.lo[2]), 2, "two cells wide and one thick");
        }
        // The other seed turns it the other way; no ribbon is longer than bends.
        assert_ne!(ribbon(knot, 3, mat::CLOTH_RED, 8)[0].hi, r[0].hi);
        assert_eq!(ribbon(knot, 99, mat::CLOTH_RED, 0).len(), RIBBON_LONGEST as usize);
    }

    /// How far a sheet's boxes give at each corner of each, by where the corner is.
    fn gives_at(boxes: &[PlantBox]) -> HashMap<(i32, i32), Vec<u8>> {
        let mut at: HashMap<(i32, i32), Vec<u8>> = HashMap::new();
        for b in boxes {
            let Cloth::Sheet { gives, .. } = b.cloth() else { panic!("{b:?} is not a sheet") };
            for (k, &(x, z)) in CORNERS.iter().enumerate() {
                let p = (if x < 0 { b.lo[0] } else { b.hi[0] }, if z < 0 { b.lo[2] } else { b.hi[2] });
                at.entry(p).or_default().push(gives[k]);
            }
        }
        at
    }

    #[test]
    fn a_sheet_is_held_at_its_corners_and_whole_between_them() {
        // A sail shade four metres by three, two corners high and two low.
        let corners = [IVec3::new(-20, 26, 8), IVec3::new(20, 21, 8), IVec3::new(20, 26, 38), IVec3::new(-20, 21, 38)];
        let s = sheet(corners, mat::CLOTH_PALE);
        assert!(!s.is_empty());
        let at = gives_at(&s);
        for c in corners {
            assert_eq!(at[&(c.x, c.z)], vec![0], "tied fast at {c}");
        }
        // Where tiles meet, they agree on how far the sheet gives there, so it never tears.
        for (p, g) in &at {
            assert!(g.iter().all(|&v| v == g[0]), "the tiles at {p:?} disagree: {g:?}");
        }
        assert_eq!(at.values().map(|g| g[0]).max(), Some(SHEET_GIVE), "the middle gives most");
        // Each tile keeps within a voxel, and steps by a cell at most from the next.
        let mut seen = 0;
        for b in &s {
            for axis in [0, 2] {
                assert_eq!(b.lo[axis].div_euclid(CELLS), (b.hi[axis] - 1).div_euclid(CELLS), "{b:?} crosses a voxel");
            }
            for n in &s {
                let side = (b.hi[0] == n.lo[0] && b.lo[2] == n.lo[2]) || (b.hi[2] == n.lo[2] && b.lo[0] == n.lo[0]);
                if side {
                    seen += 1;
                    assert!(b.lo[1] <= n.hi[1] && n.lo[1] <= b.hi[1], "light shows between {b:?} and {n:?}");
                }
            }
        }
        assert!(seen > 0);
        // It covers the rectangle between the ties exactly once.
        let area: i32 = s.iter().map(|b| (b.hi[0] - b.lo[0]) * (b.hi[2] - b.lo[2])).sum();
        assert_eq!(area, 40 * 30);
    }

    #[test]
    fn a_turned_sheet_gives_where_it_did() {
        let corners = [IVec3::new(0, 20, 0), IVec3::new(24, 18, 0), IVec3::new(24, 20, 16), IVec3::new(0, 18, 16)];
        let s = sheet(corners, mat::CLOTH_PALE);
        let here = gives_at(&s);
        for turns in 0..4 {
            let placed = place_fine(&s, IVec3::new(3, 0, -2), turns);
            let there = gives_at(&placed);
            for (&(x, z), g) in &here {
                // Where that corner of the sheet went.
                let c = IVec3::new(CELLS / 2, 0, CELLS / 2);
                let r = rotate(IVec3::new(x, 0, z) - c, turns) + c + IVec3::new(3, 0, -2) * CELLS;
                assert_eq!(&there[&(r.x, r.z)], g, "turned {turns}: the corner from ({x}, {z})");
            }
        }
    }

    #[test]
    fn small_sheets_billow_less_than_great_ones() {
        assert!(sheet_span(12.0) < sheet_span(25.0) && sheet_span(25.0) < sheet_span(40.0) && sheet_span(40.0) < sheet_span(80.0));
        assert_eq!(sheet_give(0.0, 0.0), 0);
        assert_eq!(sheet_give(1.0, 1.0), 0);
        assert_eq!(sheet_give(0.5, 0.5), SHEET_GIVE);
        assert!(sheet_give(0.5, 0.0) > 0 && sheet_give(0.5, 0.0) < sheet_give(0.5, 0.5), "a free edge gives, less than the middle");
    }
}
