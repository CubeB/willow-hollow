//! Pollarded willows (#64): along the streams through a place's land stand willows in a
//! row, a little way back from the water, cut back every few years to a stout trunk for
//! their poles. A row keeps to one bank of a stretch, or to both, and now and then one
//! has died or been grubbed out, so it has its gaps; and a stretch here and there has
//! none.
//!
//! They are trees the world grows, as a village's own tree is (see
//! [`crate::features::Planted`]): planted where the stream runs once the land is worked
//! out, so the trees' layers grow them, nothing grows under them, and a logging camp that
//! reaches them fells them with the rest.

use glam::{Vec2, vec2};

use super::influence::{Farm, ROUGH};
use crate::{
    biome::Biome,
    continent::Continent,
    features::{Planted, trees::Species},
    noise::{hash3, unit},
    scale::m,
    settlements::Region,
    surface::{Column, Cover, MIN_RIVER_Q, Sampler},
};

/// How far apart the willows of a row stand, in voxels.
const SPACING: f32 = m(7.0);
/// How far back from the water's edge a willow stands, in voxels.
const BANK: f32 = m(1.6);
/// The most water a stream may carry and still be lined with willows: a wide river is
/// not.
const STREAM: f32 = MIN_RIVER_Q * 30.0;

/// Whether the people of `farm` keep willows by their streams: in the green, wet country
/// of the lowlands and the hills, neither cold nor hot.
fn keeps(farm: &Farm) -> bool {
    matches!(farm.region, Region::Temperate | Region::Wetland | Region::Upland)
        && (0.25..0.75).contains(&farm.temperature)
        && !farm.biome.arid()
        && !matches!(farm.biome, Biome::Savanna | Biome::Steppe | Biome::Tundra | Biome::Taiga | Biome::Rainforest | Biome::Mangrove)
}

/// The willows of continent `c`, whose land is worked out, along the streams through
/// every place's land that keeps them. `seed` is the world's.
pub fn plant(c: &Continent, seed: u32) -> Vec<Planted> {
    let land = &c.land;
    let mut out: Vec<Planted> = Vec::new();
    if land.farms.is_empty() {
        return out;
    }
    let (n, cs) = (c.size, c.cell_size());
    let middle = |k: usize| vec2(((k % n) as f32 + 0.5) * cs, ((k / n) as f32 + 0.5) * cs);
    for k in 0..n * n {
        let r = c.rcv[k] as usize;
        if c.ocean[k] || r == k || c.ocean[r] || !(MIN_RIVER_Q..=STREAM).contains(&c.discharge[k]) {
            continue;
        }
        // The stretch of the stream from this cell of the grid down to the next.
        let (a, b) = (middle(k), middle(r));
        let mid = (a + b) * 0.5;
        let Some(owner) = land.owner(mid.x as i32, mid.y as i32) else { continue };
        if land.managed_at(mid) < ROUGH + 0.05 || !keeps(&land.farms[owner]) {
            continue;
        }
        let (i, j) = ((k % n) as i32, (k / n) as i32);
        let sides: &[f32] = match unit(hash3(seed ^ 0x9011, i, j, 0)) {
            r if r < 0.35 => &[1.0, -1.0],
            r if r < 0.65 => &[1.0],
            r if r < 0.9 => &[-1.0],
            _ => continue,
        };
        let len = a.distance(b);
        let along = (b - a) / len;
        let across = vec2(-along.y, along.x);
        let lo = a.min(b) - 48.0;
        let sampler = Sampler::new(c, seed, lo.x as i32, lo.y as i32, ((a - b).abs().max_element() + 96.0) as i32);
        let col = |p: Vec2| sampler.column(p.x.floor() as i32, p.y.floor() as i32);
        let back = Continent::river_width(c.discharge[k]) * crate::scale::OLD_VOXEL * 0.5 + BANK;
        let phase = unit(hash3(seed ^ 0x9012, i, j, 1)) * SPACING;
        for (s, &side) in sides.iter().enumerate() {
            for step in 0..((len - phase) / SPACING).ceil().max(0.0) as i32 {
                let t = phase + step as f32 * SPACING;
                let roll = hash3(seed ^ 0x9013, i * 4 + s as i32, j, step);
                if unit(roll) > 0.85 {
                    continue;
                }
                // Where the stream's bank lies about here: the channel winds about the
                // line of the grid, so the row follows its own water, not the line.
                let p0 = a + along * t + across * (side * back);
                let (r0, r1) = (col(p0).river, col(p0 + across * (side * 2.0)).river);
                if r0 > m(12.0) || r1 <= r0 {
                    continue;
                }
                let p = p0 - across * (side * (r0 - BANK));
                let here = col(p);
                if !stands(c, &sampler, &here, p) {
                    continue;
                }
                let at = (p.x.floor() as i32, p.y.floor() as i32);
                let room = |x: i32, z: i32| apart(at, (x, z), SPACING * 0.6);
                // Clear of the row's last, of another stream's, and of a village's tree.
                if !out.iter().chain(c.trails.planted.iter().filter(|t| t.species != Species::Pollard)).all(|t| room(t.x, t.z)) {
                    continue;
                }
                out.push(Planted { x: at.0, z: at.1, species: Species::Pollard, variant: (roll >> 8) as usize % Species::Pollard.variants(), turns: (roll >> 20) % 4 });
            }
        }
    }
    out
}

/// Whether columns `a` and `b` are `least` or more apart. Worked out in 64 bits: two
/// trees at opposite ends of a continent are far enough apart that the square of the
/// distance between them does not fit in 32, and wrapped round to a small number or a
/// negative one, which made them neighbours.
fn apart(a: (i32, i32), b: (i32, i32), least: f32) -> bool {
    let (dx, dz) = ((a.0 - b.0) as i64, (a.1 - b.1) as i64);
    (dx * dx + dz * dz) as f32 >= least * least
}

/// Whether a willow can stand on column `col`, at continent-local `p`: dry, level ground a
/// little way back from the water, in a place's land but not its yards, and off its ways.
fn stands(c: &Continent, sampler: &Sampler, col: &Column, p: Vec2) -> bool {
    let (x, z) = (p.x.floor() as i32, p.y.floor() as i32);
    if !matches!(col.cover, Cover::Land | Cover::Bank) || col.water_surface().is_some() || col.clear < m(2.4) {
        return false;
    }
    if !(m(0.6)..=m(3.0)).contains(&col.river) || c.land.managed(x, z) < ROUGH || c.land.yard(x, z) {
        return false;
    }
    let rise = (sampler.column(x + 1, z).ground - sampler.column(x - 1, z).ground)
        .abs()
        .max((sampler.column(x, z + 1).ground - sampler.column(x, z - 1).ground).abs());
    rise <= 1.6
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{continent::SimConfig, world::GameWorld};

    /// A tree across the continent leaves a willow its room: the distance between them
    /// once overflowed, and a village's tree some 18 km to the east or west could keep a
    /// willow from its stream.
    #[test]
    fn a_tree_far_across_the_continent_is_not_in_a_willows_way() {
        for far in [46_341, 50_000, 65_535] {
            assert!(apart((0, 0), (far, 3), SPACING * 0.6), "{far} voxels east");
            assert!(apart((far, 20), (0, 0), SPACING * 0.6), "{far} voxels west");
            assert!(apart((7, far), (0, 0), SPACING * 0.6), "{far} voxels south");
        }
        assert!(!apart((100, 100), (103, 102), SPACING * 0.6));
        assert!(apart((100, 100), (100, 100 + SPACING as i32), SPACING * 0.6));
    }

    /// Willows stand in rows a little way back from the streams through the land a place
    /// works, each a row's spacing or more from the next, and the same every time.
    #[test]
    fn willows_stand_in_rows_along_the_streams() {
        let (mut all, mut neighboured) = (0, 0);
        for seed in 1..4 {
            let w = GameWorld::with_config(seed, SimConfig { size: 256 });
            let c = w.continent(0, 0);
            let willows: Vec<&Planted> = c.trails.planted.iter().filter(|t| t.species == Species::Pollard).collect();
            assert_eq!(plant(&c, w.seed()).len(), willows.len(), "seed {seed}: planted again, the willows differ");
            for t in &willows {
                all += 1;
                let col = Sampler::new(&c, w.seed(), t.x - 2, t.z - 2, 5).column(t.x, t.z);
                assert!(col.water_surface().is_none() && (m(0.6)..=m(3.0)).contains(&col.river), "a willow at {},{} {} from the water", t.x, t.z, col.river);
                assert!(c.land.managed(t.x, t.z) >= ROUGH && !c.land.yard(t.x, t.z));
                let owner = c.land.owner(t.x, t.z).expect("a willow in nobody's land");
                assert!(keeps(&c.land.farms[owner]) || c.land.managed(t.x, t.z) < 0.6);
                // In a row: the next along stands about a row's spacing off.
                let next = willows.iter().filter(|u| (u.x, u.z) != (t.x, t.z)).map(|u| (((u.x - t.x).pow(2) + (u.z - t.z).pow(2)) as f32).sqrt()).fold(f32::MAX, f32::min);
                assert!(next >= SPACING * 0.6 - 1.0, "willows at {},{} only {next} apart", t.x, t.z);
                neighboured += (next < SPACING * 1.6) as u32;
            }
        }
        println!("{all} willows, {neighboured} with a neighbour in their row");
        assert!(all > 30, "only {all} willows");
        assert!(neighboured * 2 > all, "only {neighboured} of {all} willows stand in a row");
    }

    /// Lists the willows of a full-size world (seed from `SEED`, 1 if unset), a few for
    /// each place that keeps them, with the names of the fields about them, for aiming
    /// screenshots: `cargo test -p worldgen list_willows -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn list_willows() {
        let seed = std::env::var("SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
        let mut c = crate::continent::generate(crate::macro_layout::cell_desc(seed, 0, 0), SimConfig::default());
        c.trails = crate::trails::build(&c, seed);
        c.land = super::super::influence::build(&c, seed);
        let begun = std::time::Instant::now();
        let willows = plant(&c, seed);
        println!("{} willows, planted in {:?}", willows.len(), begun.elapsed());
        let mut shown = std::collections::HashMap::new();
        for t in &willows {
            let Some(owner) = c.land.owner(t.x, t.z) else { continue };
            let n = shown.entry(owner).or_insert(0);
            *n += 1;
            if *n > 3 {
                continue;
            }
            let s = &c.trails.settlements[owner];
            let names: Vec<_> = (0..6)
                .filter_map(|a| super::super::fieldnames::field_name_at(&c, vec2(t.x as f32, t.z as f32) + Vec2::from_angle(a as f32 * 1.05) * 30.0))
                .collect();
            println!("willow at {},{} of {:?} {:?} at {},{}: fields {names:?}", t.x, t.z, s.age, s.kind, s.x, s.z);
        }
    }
}
