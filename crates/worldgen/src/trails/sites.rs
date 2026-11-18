//! Where landmarks go: read off the continent map, so each kind stands somewhere it
//! makes sense. Lookouts crown hills, cairns mark passes, shrines sit at springs and
//! lake shores, camps by rivers, landings on the coast.

use crate::{
    biome::Biome,
    continent::{Continent, NO_LAKE},
    features::landmarks::Landmark,
    noise::{hash2, unit},
    scale::m,
    surface::MIN_RIVER_Q,
};

/// Fewest voxels between any two landmarks, and between a landmark and a village or
/// town it is not part of (half that from a hamlet).
pub(crate) const MIN_GAP: f32 = m(720.0);
/// Square kilometres of land per landmark.
const LAND_PER_SITE: f32 = 5.0;

fn weight(kind: Landmark) -> f32 {
    match kind {
        Landmark::Camp => 5.0,
        Landmark::Shrine => 4.0,
        Landmark::Lookout | Landmark::Ruins => 3.0,
        Landmark::Stones | Landmark::Hut | Landmark::Cairn | Landmark::Landing => 2.0,
        Landmark::Arch => 1.0,
    }
}

/// How much a site draws people, for the traffic on the trails between sites. A trip
/// between two sites counts the product of theirs. Settlements slot in above all of
/// these (a hamlet 6, a town 12, see [`crate::settlements::Kind::importance`]), and the
/// busiest trails between them become roads.
pub fn importance(kind: Landmark) -> f32 {
    match kind {
        Landmark::Landing => 3.0,
        Landmark::Camp => 2.5,
        Landmark::Shrine => 2.0,
        Landmark::Hut => 1.5,
        Landmark::Lookout | Landmark::Stones => 1.0,
        Landmark::Arch => 0.7,
        Landmark::Cairn => 0.5,
        // Ruins are forgotten: nobody makes the trip, and no trail leads to them.
        Landmark::Ruins => 0.0,
    }
}

/// Connected land, as a label per cell (0 for water). Trails only join sites with the
/// same label.
pub fn land_components(c: &Continent) -> Vec<u32> {
    let n = c.size;
    let passable = |i: usize| !c.ocean[i] && c.lake[i] == NO_LAKE;
    let mut label = vec![0u32; n * n];
    let (mut next, mut stack) = (0u32, Vec::new());
    for start in 0..n * n {
        if label[start] != 0 || !passable(start) {
            continue;
        }
        next += 1;
        label[start] = next;
        stack.push(start);
        while let Some(i) = stack.pop() {
            let (x, z) = ((i % n) as i32, (i / n) as i32);
            for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
                let (nx, nz) = (x + dx, z + dz);
                if nx < 0 || nz < 0 || nx >= n as i32 || nz >= n as i32 {
                    continue;
                }
                let j = nz as usize * n + nx as usize;
                if label[j] == 0 && passable(j) {
                    label[j] = next;
                    stack.push(j);
                }
            }
        }
    }
    label
}

/// Ranked places for each kind of landmark, best first, with how many of each to place.
pub fn candidates(c: &Continent, comp: &[u32]) -> Vec<(Landmark, usize, Vec<usize>)> {
    let n = c.size as i32;
    let scale = (n as f32 / 1024.0).max(0.25);
    // Every height and score below is in simulation units.
    let cs = c.sim_cell_size();
    let stride = (n / 256).max(1) as usize;
    let at = |x: i32, z: i32| (z.clamp(0, n - 1) * n + x.clamp(0, n - 1)) as usize;
    let h = |x: i32, z: i32| c.height[at(x, z)];
    let radius = |r: f32| ((r * scale).round() as i32).max(1);
    let (r_hill, r_saddle, r_near) = (radius(12.0), radius(6.0), radius(3.0));

    let land = comp.iter().filter(|&&l| l != 0).count();
    let total = (land as f32 * (cs / 1000.0).powi(2) / LAND_PER_SITE).clamp(0.0, 90.0);
    let sum: f32 = Landmark::ALL.iter().map(|&k| weight(k)).sum();

    let mut ranked: Vec<(Landmark, Vec<(f32, usize)>)> = Landmark::ALL.iter().map(|&k| (k, Vec::new())).collect();
    let margin = (r_hill + 1).min(n / 4);
    for z in (margin..n - margin).step_by(stride) {
        for x in (margin..n - margin).step_by(stride) {
            let i = at(x, z);
            if comp[i] == 0 || c.height[i] < 2.0 {
                continue;
            }
            let here = c.height[i];
            let q = c.discharge[i];
            let steep = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .iter()
                .map(|&(dx, dz)| (h(x + dx, z + dz) - here).abs())
                .fold(0.0f32, f32::max)
                / cs;
            let ring = |r: i32| {
                [(r, 0), (-r, 0), (0, r), (0, -r), (r, r), (r, -r), (-r, r), (-r, -r)].map(|(dx, dz)| at(x + dx, z + dz))
            };
            let relief = here - ring(r_hill).iter().map(|&j| c.height[j]).sum::<f32>() / 8.0;
            let rng = |salt: u32| unit(hash2(c.desc.seed ^ salt, x, z));
            let biome = Biome::from_u8(c.biome[i]);
            let is_river = q >= MIN_RIVER_Q;
            let river_near = ring(r_near).iter().map(|&j| c.discharge[j]).fold(0.0f32, f32::max);
            let lake_near = ring(r_near).iter().any(|&j| c.lake[j] != NO_LAKE);
            let sea_near = ring(radius(2.0)).iter().any(|&j| c.ocean[j]);
            let drop = if c.ocean[c.rcv[i] as usize] {
                0.0
            } else {
                c.water_level[i] - c.water_level[c.rcv[i] as usize]
            };

            let mut offer = |kind: Landmark, score: f32| ranked[kind as usize].1.push((score, i));

            if !is_river && here > 25.0 && relief > 6.0 && steep < 0.5 {
                offer(Landmark::Lookout, relief / (1.0 + steep * 2.0) + rng(1) * 8.0);
            }
            if here > 40.0 && !is_river {
                let (e, w, s, n2) = (
                    h(x + r_saddle, z),
                    h(x - r_saddle, z),
                    h(x, z + r_saddle),
                    h(x, z - r_saddle),
                );
                let ew_low = e.max(w) < here && s.min(n2) > here;
                let ns_low = s.max(n2) < here && e.min(w) > here;
                if (ew_low || ns_low) && steep < 0.7 {
                    offer(Landmark::Cairn, 40.0 + here * 0.1 + rng(2) * 10.0);
                }
            }
            if steep < 0.35 && !is_river {
                if drop > 6.0 || river_near > 0.0 && lake_near {
                    offer(Landmark::Shrine, 70.0 + rng(3) * 30.0);
                } else if lake_near {
                    offer(Landmark::Shrine, 50.0 + rng(3) * 30.0);
                } else if here > 30.0 && (MIN_RIVER_Q..120.0).contains(&river_near) {
                    offer(Landmark::Shrine, 20.0 + rng(3) * 30.0);
                }
            }
            if steep < 0.2 && !is_river && (100.0..9000.0).contains(&river_near) {
                offer(Landmark::Camp, 30.0 + rng(4) * 30.0 - here * 0.02);
            }
            if steep < 0.12 && !is_river && (10.0..140.0).contains(&here) && relief >= 0.0 && biome != Biome::Alpine {
                offer(Landmark::Ruins, rng(5) * 40.0 + relief * 0.5);
            }
            if steep < 0.15
                && !is_river
                && matches!(biome, Biome::Grassland | Biome::Heath | Biome::Tundra | Biome::Savanna)
            {
                offer(Landmark::Stones, rng(6) * 50.0 + relief.max(0.0) * 0.6);
            }
            // Huts are no longer built: a house among the trees sat badly with a world
            // where nobody lives. The kind is kept so journals that name one still read.
            if relief > 4.0
                && steep < 0.6
                && !is_river
                && matches!(biome, Biome::Desert | Biome::Savanna | Biome::Heath | Biome::Grassland | Biome::Tundra)
            {
                offer(Landmark::Arch, rng(8) * 50.0 + relief);
            }
            if here < 10.0 && steep < 0.18 && sea_near {
                offer(Landmark::Landing, 30.0 + rng(9) * 30.0 + if river_near > 400.0 { 25.0 } else { 0.0 });
            }
        }
    }
    ranked
        .into_iter()
        .map(|(kind, mut list)| {
            list.sort_unstable_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
            let quota = (total * weight(kind) / sum).ceil() as usize;
            let quota = if kind == Landmark::Landing { quota.max(2) } else { quota };
            (kind, quota, list.into_iter().map(|(_, i)| i).collect())
        })
        .collect()
}

/// Whether a position is far enough from all the ones in `placed`.
pub fn spaced(placed: &[(f32, f32)], x: f32, z: f32) -> bool {
    placed.iter().all(|&(px, pz)| (px - x).hypot(pz - z) >= MIN_GAP)
}
