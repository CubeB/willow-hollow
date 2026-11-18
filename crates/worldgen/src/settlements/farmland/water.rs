//! The water people keep in the land about a settlement (#64): the paddies of warm wet
//! valleys (see [`super::paddy`]) and the ditches that drain the wet ground about an old
//! place (see [`super::ditch`]).
//!
//! Both change the ground itself, so both are decided in the surface sampler, column by
//! column and from the column alone, as a terrace's shelves are: every chunk that asks
//! finds the same water at the same level, and the chunks either side of a border agree.

use super::{ditch, paddy};
use crate::{continent::Continent, surface::Column};

/// How a column of a paddy or a ditch is shaped: its ground, and its water.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Shape {
    /// The column's new ground (its top voxel is at `floor(ground)`).
    pub ground: f32,
    /// The water standing on it, as [`Column::water`] has it, if any.
    pub water: Option<i32>,
}

/// Shapes the column at continent-local `(lx, lz)` of `c`, whose ground the land, the
/// trails and the terraces left as `col`, into a paddy or a ditch if it lies in one.
/// `seed` is the world's. Out in the wild, and in the land of a place that neither grows
/// rice nor drains its ground, this is one lookup.
pub fn shape(c: &Continent, seed: u32, lx: i32, lz: i32, col: &mut Column) {
    let Some(owner) = c.land.owner(lx, lz) else { return };
    let farm = &c.land.farms[owner];
    let shaped = if paddy::grows(farm) {
        paddy::shape(c, seed, lx, lz, col)
    } else if ditch::drains(farm) {
        ditch::cut(c, seed, lx, lz, owner, col)
    } else {
        None
    };
    if let Some(s) = shaped {
        col.ground = s.ground;
        col.water = s.water;
    }
}

#[cfg(test)]
mod tests {
    use crate::continent::SimConfig;

    /// Lists the settlements of a full-size world (seed from `SEED`, 1 if unset) that
    /// grow rice or drain their land, with where their paddies and ditched fields are, for
    /// aiming screenshots: `cargo test -p worldgen list_water -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn list_water() {
        let seed = std::env::var("SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
        let mut c = crate::continent::generate(crate::macro_layout::cell_desc(seed, 0, 0), SimConfig::default());
        c.trails = crate::trails::build(&c, seed);
        c.land = super::super::influence::build(&c, seed);
        for (k, f) in c.land.farms.iter().enumerate() {
            let (rice, drains) = (super::paddy::grows(f), super::ditch::drains(f));
            if !rice && !drains {
                continue;
            }
            let n = (f.radius + f.reach) as i32;
            let (mut paddy, mut ditched, mut first) = (0, 0, None);
            for x in (f.at.x as i32 - n..f.at.x as i32 + n).step_by(6) {
                for z in (f.at.y as i32 - n..f.at.y as i32 + n).step_by(6) {
                    let Some(found) = super::super::fields::locate(&c, seed, x, z) else { continue };
                    if found.plot.owner != k {
                        continue;
                    }
                    if found.plot.crop == super::super::Crop::Paddy {
                        paddy += 1;
                        first.get_or_insert((x, z));
                    } else if drains && found.q == found.p {
                        ditched += 1;
                        first.get_or_insert((x, z));
                    }
                }
            }
            // Where a way crosses one of its ditches, for a bridge.
            let bridges: Vec<(i32, i32)> = c
                .trails
                .points()
                .filter(|p| p.half_width > 0.0 && drains && c.land.owner(p.x as i32, p.z as i32) == Some(k))
                .filter(|p| super::ditch::crossing(&c, k, glam::vec2(p.x, p.z)))
                .map(|p| (p.x as i32, p.z as i32))
                .take(3)
                .collect();
            let s = &c.trails.settlements[k];
            println!("  bridges at {bridges:?}");
            println!(
                "{:?} {:?} {:?} at {},{} y {} warmth {:.2} {:?}: rice {rice} drains {drains}: {paddy} paddy and {ditched} ditched samples, first at {first:?}",
                s.age, s.kind, s.region, s.x, s.z, s.y, f.temperature, f.biome
            );
        }
    }
}
