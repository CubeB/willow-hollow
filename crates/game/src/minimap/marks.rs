//! Places worth walking to, found in a continent: waterfalls, lakes, peaks, tors, springs,
//! river mouths and confluences, and landmarks. The map shows each by its icon and name
//! once it has been visited. Bridges are not among
//! them: trails cross so many rivers that a bridge is something walked over on the way
//! to somewhere, not a place to go and find.
//!
//! The water features are chosen as `mapgen --poi` chooses them (falls by drop, lakes by
//! size, confluences by the size of the smaller river); those the place names already
//! pick out (lakes, falls, peaks, landmarks) come from the gazetteer, with their names.

use glam::{DVec2, Vec2};
use worldgen::{
    continent::Continent,
    macro_layout::MACRO_SIZE,
    names::PlaceKind,
    scale::m,
    surface::Spring,
};

/// What sort of place a mark stands for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Falls,
    Lake,
    Peak,
    Tor,
    Spring,
    Mouth,
    Confluence,
    Landmark,
    /// A hamlet, village or town.
    Settlement,
}

#[derive(Clone, Debug)]
pub struct Mark {
    pub kind: Kind,
    /// World voxels, x and z.
    pub at: DVec2,
    /// How near the player must come for it to count as visited, in voxels.
    pub reach: f32,
    pub name: Option<String>,
    /// Whether the map hints at it before it has been visited. A share of what people
    /// built is left unmarked, to come on unawares.
    pub hint: bool,
}

/// The share of landmarks the map gives no hint of.
pub const SURPRISES: f32 = 0.3;

/// Whether the landmark at continent-local column `(x, z)` of a world is one the map
/// keeps quiet about: picked by a hash of the seed and the place, so it is the same
/// every time the world is played, and owes nothing to the order sites are found in.
pub fn surprise(world_seed: u32, x: i32, z: i32) -> bool {
    let mut h = world_seed.wrapping_mul(0x9e37_79b9) ^ (x as u32).wrapping_mul(0x85eb_ca6b) ^ (z as u32).wrapping_mul(0xc2b2_ae35) ^ 0x51ed_270b;
    h = (h ^ (h >> 16)).wrapping_mul(0x7feb_352d);
    h = (h ^ (h >> 15)).wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    ((h >> 8) as f32 / (1u32 << 24) as f32) < SURPRISES
}

/// Fewest cells apart two river mouths, or two confluences, are picked.
const RIVER_SPACING: f32 = 48.0;
/// Least water a river must carry for its mouth to be marked.
const MIN_MOUTH: f32 = 1500.0;
/// Least water the smaller of two joining rivers must carry.
const MIN_TRIBUTARY: f32 = 500.0;
/// How near the player must come to a small place for it to count as visited.
const NEAR: f32 = m(45.0);

/// The marks of continent `c`, in world voxels.
pub fn find(c: &Continent, world_seed: u32) -> Vec<Mark> {
    let origin = DVec2::new(c.desc.mx as f64, c.desc.mz as f64) * MACRO_SIZE as f64;
    let world = |p: Vec2| origin + p.as_dvec2();
    let cs = c.cell_size();
    let n = c.size;
    let centre = |i: usize| Vec2::new((i % n) as f32 + 0.5, (i / n) as f32 + 0.5) * cs;
    let mut marks = Vec::new();

    for p in &c.trails.names.places {
        let (kind, reach) = match p.kind {
            PlaceKind::Falls => (Kind::Falls, NEAR),
            // From the shore, from as far as the journal knows a lake by name: the
            // radius is the lake's as a circle of the same area.
            PlaceKind::Lake => (Kind::Lake, p.radius * 1.1 + crate::journal::LAKE_REACH),
            PlaceKind::Peak => (Kind::Peak, p.radius.max(NEAR)),
            // A tor is reached by standing among its piles.
            PlaceKind::Tor => (Kind::Tor, NEAR),
            _ => continue,
        };
        marks.push(Mark { kind, at: world(p.at), reach, name: Some(p.name.clone()), hint: true });
    }
    for (k, s) in c.trails.sites.iter().enumerate() {
        let at = Vec2::new(s.x as f32 + 0.5, s.z as f32 + 0.5);
        let name = c.trails.names.site(k).map(|p| p.name.clone());
        // Continent-local, so a site's luck does not change with where its continent lies.
        let hint = !surprise(world_seed, s.x, s.z);
        marks.push(Mark { kind: Kind::Landmark, at: world(at), reach: NEAR, name, hint });
    }
    // Settlements are visited by walking onto their green, and never kept quiet about:
    // everyone in the country knows where the villages are.
    for (k, s) in c.trails.settlements.iter().enumerate() {
        let name = c.trails.names.settlement(k).map(|p| p.name.clone());
        marks.push(Mark { kind: Kind::Settlement, at: world(s.at()), reach: s.radius.max(NEAR), name, hint: true });
    }

    let land: Vec<usize> = (0..n * n).filter(|&i| !c.ocean[i]).collect();
    let mouths = land
        .iter()
        .filter(|&&i| c.ocean[c.rcv[i] as usize] && c.discharge[i] >= MIN_MOUTH)
        .map(|&i| (c.discharge[i], i))
        .collect();
    for i in spaced(n, mouths, 8, RIVER_SPACING) {
        marks.push(Mark { kind: Kind::Mouth, at: world(centre(i)), reach: NEAR * 2.0, name: None, hint: true });
    }
    // Where the second-largest river into a cell is itself a real river.
    let (mut first, mut second) = (vec![0.0f32; n * n], vec![0.0f32; n * n]);
    for &i in &land {
        let (r, q) = (c.rcv[i] as usize, c.discharge[i]);
        if q > first[r] {
            second[r] = first[r];
            first[r] = q;
        } else if q > second[r] {
            second[r] = q;
        }
    }
    let joins = land.iter().filter(|&&i| second[i] >= MIN_TRIBUTARY).map(|&i| (second[i], i)).collect();
    for i in spaced(n, joins, 8, RIVER_SPACING) {
        marks.push(Mark { kind: Kind::Confluence, at: world(centre(i)), reach: NEAR * 1.5, name: None, hint: true });
    }
    // The springs that well up into a pool of some size, a few to a continent.
    let springs: Vec<(f32, usize, Vec2)> = land
        .iter()
        .filter(|&&i| Spring::head(c, i).is_some())
        .filter_map(|&i| Spring::at(c, world_seed, i).map(|s| (s.radius, i, s.pos)))
        .filter(|s| s.0 >= 2.0)
        .collect();
    let at_of: std::collections::HashMap<usize, Vec2> = springs.iter().map(|s| (s.1, s.2)).collect();
    for i in spaced(n, springs.iter().map(|s| (s.0, s.1)).collect(), 10, 64.0) {
        marks.push(Mark { kind: Kind::Spring, at: world(at_of[&i]), reach: NEAR, name: None, hint: true });
    }
    marks
}

/// The highest-scoring cells, at most `count`, each at least `spacing` cells from the
/// others picked. Ties go to the lower index, so the choice is stable.
fn spaced(n: usize, mut v: Vec<(f32, usize)>, count: usize, spacing: f32) -> Vec<usize> {
    v.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut out: Vec<usize> = Vec::new();
    for (_, i) in v {
        let far = out.iter().all(|&j| {
            let dx = (i % n) as f32 - (j % n) as f32;
            let dz = (i / n) as f32 - (j / n) as f32;
            dx.hypot(dz) > spacing
        });
        if far {
            out.push(i);
            if out.len() == count {
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// About the share of landmarks asked for is left unmarked, the same ones each time
    /// a world is played, and other ones in another world.
    #[test]
    fn a_share_of_landmarks_is_left_unmarked_the_same_each_time() {
        let sites: Vec<(i32, i32)> = (0..4000).map(|k| ((k * 7919) % 65536, (k * 104_729) % 65536)).collect();
        let quiet = |seed: u32| sites.iter().map(|&(x, z)| surprise(seed, x, z)).collect::<Vec<bool>>();
        let one = quiet(1);
        let share = one.iter().filter(|&&q| q).count() as f32 / sites.len() as f32;
        assert!((share - SURPRISES).abs() < 0.03, "{share}");
        assert_eq!(one, quiet(1));
        let differ = one.iter().zip(quiet(2)).filter(|(a, b)| **a != *b).count();
        assert!(differ > sites.len() / 5, "{differ}");
    }

    #[test]
    fn spaced_keeps_the_best_and_drops_their_neighbours() {
        let n = 100;
        let v = vec![(5.0, 10 * n + 10), (9.0, 10 * n + 12), (1.0, 80 * n + 80), (3.0, 50 * n + 50)];
        let got = spaced(n, v, 8, 10.0);
        assert_eq!(got, vec![10 * n + 12, 50 * n + 50, 80 * n + 80]);
        let v = vec![(5.0, 1), (9.0, 50), (1.0, 99)];
        assert_eq!(spaced(n, v, 2, 10.0).len(), 2);
    }
}
