//! How each river's water comes and goes through the year, from the climate of the
//! ground it drains (#142). The year is drawn, not generated: the water stands where it
//! always does, and this only tells the shader how full a stream should look in spring
//! and how low by the end of summer (the game's `season::stream`).

/// A river's year. The number rides to the shader in two bits of each water vertex and of
/// each bank voxel beside it (see `voxel::mesh::Current::regime`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Regime {
    /// The same all year: lakes, the sea, and rivers of the wet tropics, which have no
    /// melt and no summer drought.
    Steady = 0,
    /// Dry country (heath, steppe, desert, the drier savanna): a little fuller after the
    /// winter rain, and very low by late summer.
    Dry = 1,
    /// Fed by rain in a temperate climate: full in early spring, low in late summer.
    Rain = 2,
    /// Fed by snow: swells most, and latest, as the snow on the hills melts.
    Snow = 3,
}

/// The share of a river's water that has to come from snow country for it to swell with
/// the melt: a third, so a lowland river with a range in its catchment counts.
const SNOW_FED: f32 = 0.3;
/// The share from hot country above which a river follows the tropics' year.
const HOT_FED: f32 = 0.5;
/// Below this mean rainfall over its catchment (1 the continent's average) a river runs
/// in dry country.
const DRY_CATCHMENT: f32 = 0.6;

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Each cell's [`Regime`] as u8, from every cell whose water passes through it, each
/// counted by the rain that falls there: so a lowland river fed from a snowy range swells
/// with the melt, and a river crossing a desert from wet hills is not a desert stream.
/// Temperatures are the biomes' (`Biome::classify`): below 0.24 is snow country, above
/// 0.58 hot.
pub fn regimes(rcv: &[u32], ocean: &[bool], rain: &[f32], temperature: &[f32]) -> Vec<u8> {
    let n = rcv.len();
    // Per cell: rain from snow country, rain from hot country, all the rain, and how many
    // cells it fell on, summed down the drainage.
    let mut sums = vec![[0.0f32; 4]; n];
    let mut donors = vec![0u32; n];
    for i in 0..n {
        if ocean[i] {
            continue;
        }
        let r = rcv[i] as usize;
        if r != i && !ocean[r] {
            donors[r] += 1;
        }
        let (w, t) = (rain[i].max(0.0), temperature[i]);
        sums[i] = [w * (1.0 - smoothstep(0.18, 0.30, t)), w * smoothstep(0.52, 0.64, t), w, 1.0];
    }
    // From the divides down: a cell is passed on once all its donors have been.
    let mut ready: Vec<usize> = (0..n).filter(|&i| !ocean[i] && donors[i] == 0).collect();
    while let Some(i) = ready.pop() {
        let r = rcv[i] as usize;
        if r == i || ocean[r] {
            continue;
        }
        let s = sums[i];
        for (into, v) in sums[r].iter_mut().zip(s) {
            *into += v;
        }
        donors[r] -= 1;
        if donors[r] == 0 {
            ready.push(r);
        }
    }
    sums.iter()
        .zip(ocean)
        .map(|(&[snow, hot, water, cells], &sea)| {
            if sea || water <= 0.0 {
                return Regime::Steady as u8;
            }
            let wet = water / cells;
            let regime = if snow / water >= SNOW_FED {
                Regime::Snow
            } else if hot / water >= HOT_FED {
                if wet < DRY_CATCHMENT { Regime::Dry } else { Regime::Steady }
            } else if wet < DRY_CATCHMENT {
                Regime::Dry
            } else {
                Regime::Rain
            };
            regime as u8
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A river carries the year of the ground it drains: snow melt from a cold head runs
    /// on down past warm lowlands, until enough rain has joined it there to outweigh it.
    #[test]
    fn a_river_keeps_the_year_of_its_catchment() {
        // A line of cells draining toward the sea at the end: 0 -> 1 -> ... -> 7 (sea).
        let n = 8;
        let rcv: Vec<u32> = (0..n as u32).map(|i| (i + 1).min(n as u32 - 1)).collect();
        let mut ocean = vec![false; n];
        ocean[n - 1] = true;
        let rain = vec![1.0; n];
        // Two cold cells at the head, then temperate.
        let temperature: Vec<f32> = (0..n).map(|i| if i < 2 { 0.1 } else { 0.4 }).collect();
        let r = regimes(&rcv, &ocean, &rain, &temperature);
        assert_eq!(r[0], Regime::Snow as u8);
        assert_eq!(r[5], Regime::Snow as u8, "2 of 6 cells' water is melt");
        assert_eq!(r[6], Regime::Rain as u8, "2 of 7 is too little");
        assert_eq!(r[7], Regime::Steady as u8, "the sea");
        // The same river in dry country, and in the wet tropics.
        let dry = regimes(&rcv, &ocean, &vec![0.3; n], &vec![0.4; n]);
        assert_eq!(dry[6], Regime::Dry as u8);
        let tropics = regimes(&rcv, &ocean, &vec![1.5; n], &vec![0.9; n]);
        assert_eq!(tropics[6], Regime::Steady as u8);
    }
}
