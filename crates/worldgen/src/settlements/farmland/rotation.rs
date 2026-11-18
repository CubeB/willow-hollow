//! Crop rotation (#207): from year 1, a field that grows grain, roots or flax, or lies
//! ploughed, moves on round a rotation of three once a year, each field in a season of
//! its own, so that across a settlement's fields one or another is changing in every
//! season. In year 0 every field carries its own crop, as laid out. A field's own crop
//! still says what grows at its edges and what stands in it (`orchard`, `bounds`): only
//! what it carries changes.

use super::{
    Far, crops,
    fields::{Crop, Plot},
};
use crate::{
    continent::Continent,
    materials as mat,
    noise::hash2,
    settlements::years::When,
    surface::Column,
};

/// The three crops a field of `own` goes round, its own first: grain after roots or a
/// year's rest, each different from the one before.
fn cycle(own: Crop) -> Option<[Crop; 3]> {
    use Crop::*;
    Some(match own {
        Wheat => [Wheat, Barley, Ploughed],
        Barley => [Barley, Roots, Fallow],
        Rye => [Rye, Roots, Fallow],
        Roots => [Roots, Barley, Ploughed],
        Flax => [Flax, Wheat, Roots],
        Ploughed => [Ploughed, Wheat, Barley],
        // Fallow gone to wildflowers has its scrub (see `orchard`), and the meadows,
        // pastures, woods and trees stay as they are.
        _ => return None,
    })
}

/// The crop field `plot` carries at `when`, `seed` being the world's.
pub fn crop(plot: &Plot, seed: u32, when: When) -> Crop {
    let when = when.seen();
    let Some(cycle) = cycle(plot.crop).filter(|_| when.year > 0) else { return plot.crop };
    // The season of the year it turns in, its own.
    let season = (hash2(seed ^ 0x0c4b_7e11, plot.id as i32, 0) % 4) as i32;
    let since = (when.year - 1) * 4 + when.season as i32 - season;
    if since < 0 {
        return plot.crop;
    }
    cycle[(since / 4 + 1) as usize % 3]
}

/// `plot` with the crop it carries at `when`.
pub fn rotated(plot: Plot, seed: u32, when: When) -> Plot {
    Plot { crop: crop(&plot, seed, when), ..plot }
}

/// [`super::far`] at `when`: a field's ground with the crop it carries then, and
/// whatever is between fields as it is.
pub fn far(c: &Continent, seed: u32, col: &Column, lx: i32, lz: i32, step: i32, when: When) -> Option<Far> {
    let own = super::far(c, seed, col, lx, lz, step)?;
    if when.seen().year <= 0 {
        return Some(own);
    }
    let p = super::plot(c, seed, lx, lz)?;
    let now = crop(&p, seed, when);
    let ground = |crop: Crop| crops::ground(crop, col).unwrap_or_else(|| mat::ground_cover(col.biome));
    // A boundary between fields is drawn as it is; a field's own ground takes its crop.
    if now == p.crop || own.surface != ground(p.crop) || own.wash != crops::wash(p.crop) || own.wear != crops::grazed(p.crop) {
        return Some(own);
    }
    Some(Far { surface: ground(now), wash: crops::wash(now), wear: crops::grazed(now) })
}

#[cfg(test)]
mod tests {
    use glam::Vec2;

    use super::*;

    fn field(crop: Crop, id: u32) -> Plot {
        Plot { id, crop, rows: Vec2::X, managed: 1.0, owner: 0, terrace: None }
    }

    #[test]
    fn fields_keep_their_own_crop_in_year_zero() {
        for crop in Crop::ALL {
            for season in 0..4 {
                assert_eq!(super::crop(&field(crop, 7), 1, When::new(0, season)), crop);
            }
        }
    }

    #[test]
    fn arable_fields_turn_once_a_year_each_in_its_own_season() {
        for id in 0..40 {
            let p = field(Crop::Wheat, id);
            let seq: Vec<Crop> = (4..4 + 4 * 6).map(|q| super::crop(&p, 1, When::new(q / 4, (q % 4) as u8))).collect();
            let turns = seq.windows(2).filter(|w| w[0] != w[1]).count();
            // Six years from year 1: a turn in each but perhaps the first's.
            assert!((5..=6).contains(&turns), "field {id} turned {turns} times: {seq:?}");
            // Never the same crop two years running.
            for y in 1..6 {
                let (a, b) = (super::crop(&p, 1, When::new(y, 3)), super::crop(&p, 1, When::new(y + 1, 3)));
                assert_ne!(a, b, "field {id} kept {a:?} from year {y} to {}", y + 1);
            }
        }
        // Grass and trees stay put.
        for crop in [Crop::Pasture, Crop::Hay, Crop::Orchard, Crop::Vines, Crop::Coppice, Crop::Fallow] {
            assert_eq!(super::crop(&field(crop, 3), 1, When::new(5, 2)), crop);
        }
    }

    #[test]
    fn fields_turn_in_different_seasons() {
        let seasons: std::collections::HashSet<i32> = (0..40)
            .map(|id| {
                let p = field(Crop::Barley, id);
                (0..4).find(|&s| super::crop(&p, 1, When::new(1, s as u8)) != Crop::Barley).unwrap_or(4)
            })
            .collect();
        assert!(seasons.len() >= 3, "fields all turn together: {seasons:?}");
    }
}
