//! How well the grass grows on a column: the part of its colour that comes from where it
//! is, worked out once when the ground is made and carried to the shader with the
//! ground's own colour (see `wash_map` in voxel's mesh.rs and `sward` in terrain.wgsl).
//!
//! Two things are told. How wet it is: rain, a river or lake close by and the water
//! table under it make it lush and dark; steep ground that sheds its water, and a thin
//! skin of soil over rock, make it thin and yellow. And how worn it is: trodden short and
//! pale beside a path and round the places people gather. Both change only slowly over
//! the ground, so the grass changes in broad soft patches rather than voxel by voxel.
//! The shader adds what it knows better itself: which way a slope faces and how high it
//! is, the shade of trees, the season, and the clumps and swathes the grass grows in.

use super::{Column, Cover};
use crate::{continent::Continent, scale::m};

/// The code for a column the grass grows on in the ordinary way: neither wetter nor
/// drier than its country, and unworn. Ground with no code shows as this.
pub const PLAIN: u8 = 0;

/// The sward code of the column `col`, at continent-local voxel `(lx, lz)` of `c`. Its
/// low three bits are how wet the grass is, from -4 (parched) to 3 (lush), as a signed
/// three-bit number, so 0 is the grass of its country; the next three are how worn it
/// is, 0 to 7. `steep` is the ground's rise per voxel there, and `rocky` whether rock
/// shows through near it.
pub fn code(c: &Continent, col: &Column, lx: i32, lz: i32, steep: f32, rocky: bool) -> u8 {
    if !matches!(col.cover, Cover::Land | Cover::Bank) {
        return PLAIN;
    }
    let wet = wetness(c, col, lx, lz, steep, rocky);
    let worn = wear(col);
    let w = (wet * 3.5).round().clamp(-4.0, 3.0) as i8;
    let t = (worn * 7.0).round().clamp(0.0, 7.0) as u8;
    (w as u8 & 7) | t << 3
}

/// How wet the grass is, about -1 to 1.
fn wetness(c: &Continent, col: &Column, lx: i32, lz: i32, steep: f32, rocky: bool) -> f32 {
    // The continent's rain against its average, as the grid has it, smoothly between
    // the grid's points. Rain is what the country's own grass is coloured for, so only
    // the wetter and drier parts of a country show it, and halving or doubling it moves
    // the grass a third of the way.
    let rain = rain_at(c, lx, lz).max(0.05);
    let mut wet = (rain.log2() * 0.35).clamp(-0.4, 0.4);
    // A river's banks are lush for a few metres back from the water.
    if col.river < m(9.0) {
        let near = 1.0 - (col.river.max(0.0) / m(9.0));
        wet += near * near * 0.9;
    }
    // So is ground only a little above standing water, where the water table is high.
    if let Some(k) = col.water {
        let above = col.ground - (k - 1) as f32;
        if (0.0..m(2.5)).contains(&above) {
            wet += (1.0 - above / m(2.5)) * 0.6;
        }
    }
    if col.cover == Cover::Bank {
        wet += 0.25;
    }
    // Steep ground sheds its rain and holds only a thin soil, which dries first.
    wet -= smooth(0.3, 0.85, steep) * 0.55;
    if rocky {
        wet -= 0.5;
    }
    wet.clamp(-1.0, 1.0)
}

/// How worn the grass is, 0 to 1: trodden down along a path and round a landmark, for a
/// couple of metres out from its edge.
fn wear(col: &Column) -> f32 {
    if col.clear >= m(3.2) {
        return 0.0;
    }
    1.0 - smooth(0.0, m(3.2), col.clear)
}

/// The continent's rain at a continent-local voxel, between the grid's cell centres.
fn rain_at(c: &Continent, lx: i32, lz: i32) -> f32 {
    let cs = c.cell_size();
    let max = (c.size - 1) as f32;
    let (fx, fz) = (((lx as f32 + 0.5) / cs - 0.5).clamp(0.0, max), ((lz as f32 + 0.5) / cs - 0.5).clamp(0.0, max));
    let (x0, z0) = ((fx as usize).min(c.size - 2), (fz as usize).min(c.size - 2));
    let (tx, tz) = (fx - x0 as f32, fz - z0 as f32);
    let at = |x: usize, z: usize| c.rain[z * c.size + x];
    let a = at(x0, z0) + (at(x0 + 1, z0) - at(x0, z0)) * tx;
    let b = at(x0, z0 + 1) + (at(x0 + 1, z0 + 1) - at(x0, z0 + 1)) * tx;
    a + (b - a) * tz
}

fn smooth(lo: f32, hi: f32, x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(code: u8) -> (i8, u8) {
        (((code << 5) as i8) >> 5, code >> 3)
    }

    #[test]
    fn codes_round_trip() {
        for w in -4i8..=3 {
            for t in 0u8..=7 {
                let code = (w as u8 & 7) | t << 3;
                assert_eq!(decode(code), (w, t));
            }
        }
        assert_eq!(decode(PLAIN), (0, 0));
    }
}
