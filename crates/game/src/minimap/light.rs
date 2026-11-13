//! The map's light: one fixed sun, in the afternoon and to the west-south-west, and the
//! shadows it throws across the model, worked out once for each model as it is built
//! and kept in its corners. Nothing here is done again while the map is drawn.
//!
//! The sun stands where it does for the way the large map is first looked at, from a
//! little west of south: the sides of things turned to the eye are lit, and shadows fall
//! to the right, across the ground and clear of what throws them.
//!
//! The model is taken as a field of heights, one to a column: the ground, with whatever
//! stands on it (trees, buildings, rocks) raised on top. A point is in shadow if, going
//! from it toward the sun, the field rises above the line to the sun; how nearly it does
//! gives a soft edge, wider the further off the thing casting it is.

/// The way to the sun, in the model's own measure: east, up and south. Afternoon, from
/// the west-south-west, a little under half way up the sky. relief.wgsl has the same.
pub const SUN: [f32; 3] = [-0.66, 0.68, 0.32];

/// How far a shadow is followed, in columns: past this nothing is tall enough to matter.
const REACH: f32 = 140.0;
/// How wide a shadow's soft edge is for each column of distance from what casts it, as
/// a part of the sun's climb over that column.
const SOFT: f32 = 0.22;

/// A field of heights to cast shadows from.
pub struct Field<'a> {
    pub n: usize,
    /// The top of what stands in each column, in blocks; columns with nothing in them
    /// are very low.
    pub heights: &'a [f32],
    /// The highest of them.
    pub highest: f32,
    /// How much the sun climbs for each column gone toward it, in blocks, and the way
    /// toward it across the columns, a unit.
    climb: f32,
    way: [f32; 2],
}

impl<'a> Field<'a> {
    /// `lift` raises the sun, for models whose heights are drawn taller than life: a range
    /// drawn four times its height would otherwise lay its shadow across the whole map.
    pub fn new(n: usize, heights: &'a [f32], lift: f32) -> Self {
        debug_assert_eq!(heights.len(), n * n);
        let flat = SUN[0].hypot(SUN[2]);
        let highest = heights.iter().copied().fold(f32::MIN, f32::max);
        Self { n, heights, highest, climb: SUN[1] / flat * lift.max(1.0), way: [SUN[0] / flat, SUN[2] / flat] }
    }

    /// How much of the sun reaches the point `(x, z)` columns from the model's corner and
    /// `y` blocks up: 1 in the open, 0 in full shadow.
    pub fn lit(&self, x: f32, z: f32, y: f32) -> f32 {
        self.past(x, z, y, [i32::MAX, i32::MAX, i32::MIN, i32::MIN])
    }

    /// As [`Self::lit`], for a point on something standing in the columns from
    /// `own[0], own[1]` to `own[2], own[3]`, which do not shade it: a tree's crown is
    /// not in the shadow of the tree.
    pub fn past(&self, x: f32, z: f32, y: f32, own: [i32; 4]) -> f32 {
        let mut seen = 1.0f32;
        // Half a column at a time, from a quarter column out.
        let mut t = 0.25;
        let n = self.n as f32;
        while t < REACH {
            let line = y + t * self.climb;
            let edge = t * self.climb * SOFT;
            if line - edge > self.highest {
                break;
            }
            let (px, pz) = (x + self.way[0] * t, z + self.way[1] * t);
            if px < 0.0 || pz < 0.0 || px >= n || pz >= n {
                break;
            }
            let (cx, cz) = (px as i32, pz as i32);
            if cx >= own[0] && cz >= own[1] && cx <= own[2] && cz <= own[3] {
                t += 0.5;
                continue;
            }
            let over = line - self.heights[cz as usize * self.n + cx as usize];
            if over <= 0.0 {
                return 0.0;
            }
            seen = seen.min(over / edge);
            t += 0.5;
        }
        seen.clamp(0.0, 1.0)
    }

    /// How much of the sun reaches the top of each column of ground, given their tops in
    /// blocks (which may lie under what stands on them); columns with no ground get 1.
    pub fn ground(&self, tops: &[f32]) -> Vec<f32> {
        debug_assert_eq!(tops.len(), self.n * self.n);
        let n = self.n;
        (0..n * n)
            .map(|i| if tops[i] > f32::MIN { self.lit((i % n) as f32 + 0.5, (i / n) as f32 + 0.5, tops[i]) } else { 1.0 })
            .collect()
    }
}

/// How much brighter or darker than level ground the top of a column is for the way the
/// land about it slopes: `east` and `south` are how much it rises a column that way, in
/// blocks. 1 on the level, more on a slope facing the sun, less on one facing away.
pub fn slope(east: f32, south: f32) -> f32 {
    let (nx, ny, nz) = (-east, 1.0, -south);
    let len = (nx * nx + ny * ny + nz * nz).sqrt();
    let facing = (nx * SUN[0] + ny * SUN[1] + nz * SUN[2]) / len;
    (facing / SUN[1]).clamp(0.45, 1.35)
}

/// How open to the sky the corner of a column's top is, from whether the two columns
/// beside the corner and the one across it stand higher: the darkening that gathers at
/// the foot of a step, as on any block world. 1 in the open, down to a little over half.
pub fn corner(side_a: bool, side_b: bool, across: bool) -> f32 {
    let shut = if side_a && side_b { 3 } else { side_a as u8 + side_b as u8 + across as u8 };
    1.0 - 0.15 * shut as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(n: usize) -> Vec<f32> {
        vec![0.0; n * n]
    }

    #[test]
    fn the_sun_is_a_unit_and_stands_in_the_south_west() {
        let len = (SUN[0] * SUN[0] + SUN[1] * SUN[1] + SUN[2] * SUN[2]).sqrt();
        assert!((len - 1.0).abs() < 0.01, "{len}");
        assert!(SUN[0] < 0.0 && SUN[2] > 0.0 && SUN[1] > 0.5);
    }

    #[test]
    fn level_ground_is_all_in_the_sun() {
        let heights = flat(32);
        let f = Field::new(32, &heights, 1.0);
        assert!(f.ground(&heights).iter().all(|&v| v == 1.0));
    }

    /// A tower throws its shadow away from the sun, to the north-east, as long as its
    /// height and the sun's own height make it, and nowhere else.
    #[test]
    fn a_tower_shadows_the_ground_away_from_the_sun_and_no_further_than_its_height_reaches() {
        let n = 64;
        let mut heights = flat(n);
        heights[32 * n + 32] = 10.0;
        let f = Field::new(n, &heights, 1.0);
        let ground = flat(n);
        let lit = f.ground(&ground);
        let at = |x: usize, z: usize| lit[z * n + x];
        // Its own ground is under it; the ground just north-east of it is dark.
        assert_eq!(at(32, 32), 0.0);
        assert!(at(34, 31) < 0.3, "{}", at(34, 31));
        // Toward the sun, and to either side, the ground is in the open.
        for (x, z) in [(30, 33), (28, 34), (32, 36), (28, 29), (36, 36)] {
            assert_eq!(at(x, z), 1.0, "{x} {z}");
        }
        // The shadow is 10 blocks of height over the sun's climb long: about 10.8 columns.
        let flat_len = SUN[0].hypot(SUN[2]);
        let long = 10.0 * flat_len / SUN[1];
        let along = |d: f32| (32.5 - SUN[0] / flat_len * d, 32.5 - SUN[2] / flat_len * d);
        let (x, z) = along(long - 2.0);
        assert!(at(x as usize, z as usize) < 0.6, "inside the shadow's end: {}", at(x as usize, z as usize));
        let (x, z) = along(long + 6.0);
        assert_eq!(at(x as usize, z as usize), 1.0, "past the shadow's end");
        // Some of it is neither dark nor light: the edge is soft.
        assert!(lit.iter().any(|&v| v > 0.05 && v < 0.95));
        // The top of the tower itself is lit.
        assert_eq!(f.lit(32.5, 32.5, 10.0), 1.0);
    }

    #[test]
    fn a_valley_behind_a_ridge_is_dark_and_the_ridge_is_not() {
        let n = 48;
        // A ridge running north-west to south-east across the way to the sun.
        let heights: Vec<f32> = (0..n * n).map(|i| if (-1..1).contains(&((i % n) as i32 - (i / n) as i32)) { 6.0 } else { 0.0 }).collect();
        let f = Field::new(n, &heights, 1.0);
        let lit = f.ground(&heights);
        assert_eq!(lit[25 * n + 25], 1.0, "the ridge's own top");
        assert!(lit[21 * n + 27] < 0.2, "behind the ridge: {}", lit[21 * n + 27]);
        assert_eq!(lit[28 * n + 20], 1.0, "before the ridge");
        // With the sun lifted, as for a model drawn taller than life, the shadow is shorter.
        let lifted = Field::new(n, &heights, 3.0).ground(&heights);
        let dark = |v: &[f32]| v.iter().filter(|&&l| l < 0.5).count();
        assert!(dark(&lifted) * 2 < dark(&lit), "{} against {}", dark(&lifted), dark(&lit));
    }

    #[test]
    fn slopes_facing_the_sun_are_brighter_and_corners_under_steps_darker() {
        assert_eq!(slope(0.0, 0.0), 1.0);
        // Rising to the east and north faces the south-west.
        assert!(slope(0.5, -0.5) > 1.1);
        assert!(slope(-0.5, 0.5) < 0.8);
        assert_eq!(corner(false, false, false), 1.0);
        assert!(corner(true, false, false) < 1.0);
        assert!(corner(true, true, false) < corner(true, false, true));
        assert_eq!(corner(true, true, false), corner(true, true, true));
        assert!(corner(true, true, true) > 0.5);
    }
}
