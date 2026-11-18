//! Where settlements stand, decided on the continent map before any landmark.
//!
//! Every spot of the map is scored as people choosing where to settle would: fresh water
//! first (a river's bend, where two rivers meet, a lake shore, a spring), then level,
//! fertile ground, a place to ford or bridge the river, a sheltered bay for boats, a pass.
//! Bog, cliffs, high bare mountain and open dune sea are left alone, and in the desert
//! only an oasis will do. Towns take the best of it, far apart; villages the next best,
//! a few kilometres apart; and hamlets grow up a short walk from each village and town.
//! Each is then settled on real ground at walking scale, as landmarks are (see
//! [`crate::trails::siting`]), on a levelled green that stage 2 (#202) builds about.

use glam::Vec2;

use super::{Age, Kind, Livelihood, Reason, Region, Settlement, Water};
use crate::{
    biome::Biome,
    continent::{Continent, NO_LAKE},
    macro_layout::CellKind,
    noise::{hash2, unit},
    scale::{VOXELS_PER_METRE, m},
    surface::{Cover, MIN_RIVER_Q, Sampler, fall_drop},
    trails::{Pad, siting::FETCH},
};

/// Least distance, in voxels, between two villages, or a village and a town.
pub const VILLAGE_GAP: f32 = m(3000.0);
/// Least distance, in voxels, between two towns.
pub const TOWN_GAP: f32 = m(8000.0);
/// How far a hamlet stands from the village or town it grew from.
pub const HAMLET_NEAR: f32 = m(600.0);
pub const HAMLET_FAR: f32 = m(2000.0);
/// Least distance, in voxels, between a hamlet and any other settlement.
const HAMLET_GAP: f32 = m(600.0);
/// Square kilometres of good land for each village, as landmarks have
/// [`crate::trails::sites`]' `LAND_PER_SITE`; and the most villages on one continent.
const LAND_PER_VILLAGE: f32 = 14.0;
const MOST_VILLAGES: usize = 9;
/// Good land a continent needs for a town, and for a second one.
const LAND_FOR_TOWN: f32 = 30.0;
const LAND_FOR_TWO_TOWNS: f32 = 70.0;
/// How often a continent has a great town, and the good land it needs.
const GREAT_TOWN_CHANCE: f32 = 1.0 / 3.0;
const LAND_FOR_GREAT_TOWN: f32 = 50.0;
/// Least water, as counted on the 1024 grid, a river must carry to take a boat.
const BOAT_Q: f32 = 250.0;
/// Steepest a river may fall, in rise over run, and still be calm enough for boats:
/// any steeper and it runs down a stair of cascades.
const CALM: f32 = 0.04;
/// How far about a settlement its livelihoods are read from the country.
const LIVING: f32 = m(600.0);
/// Most map spots tried for each settlement before giving up on it.
const TRIES: usize = 60;
/// How much a village's spot gains for each kilometre it lies from the nearest
/// settlement already placed, and over how many kilometres.
const SPREAD: f32 = 1.5;
const SPREAD_KM: f32 = 6.0;

/// The settlements of one continent, with the green each stands about and what the
/// trails need to know of them.
#[derive(Default)]
pub struct Placed {
    pub settlements: Vec<Settlement>,
    /// The levelled green at each one's middle, in the same order.
    pub greens: Vec<Pad>,
    /// The map cell each was chosen by, in the same order: on land, where its trails
    /// are routed from.
    pub cells: Vec<usize>,
    /// The nearest fresh water to each middle, if it is close enough to fetch from.
    pub water: Vec<Option<Vec2>>,
}

/// How a map spot suits a settlement.
#[derive(Clone, Copy, Debug)]
struct Spot {
    cell: usize,
    /// As a village or hamlet.
    score: f32,
    /// As a town; below zero, not fit for one.
    town: f32,
    reason: Reason,
    /// Within reach of the sea.
    sea: bool,
}

/// The continent map, read for settling.
struct Map<'a> {
    c: &'a Continent,
    n: usize,
    /// Grid cells per cell of the full 1024 grid.
    scale: f32,
    /// Voxels and simulation units per grid cell.
    cs: f32,
    sim: f32,
    /// The largest and second largest river flowing into each cell, as discharge, and
    /// the cell the largest comes from.
    first: Vec<f32>,
    second: Vec<f32>,
    up: Vec<u32>,
}

impl<'a> Map<'a> {
    fn new(c: &'a Continent) -> Self {
        let n = c.size;
        let (mut first, mut second, mut up) = (vec![0.0f32; n * n], vec![0.0f32; n * n], vec![u32::MAX; n * n]);
        for i in 0..n * n {
            let r = c.rcv[i] as usize;
            if r == i || c.ocean[i] || c.discharge[i] < MIN_RIVER_Q {
                continue;
            }
            let q = c.discharge[i];
            if q > first[r] {
                second[r] = first[r];
                first[r] = q;
                up[r] = i as u32;
            } else if q > second[r] {
                second[r] = q;
            }
        }
        Self { c, n, scale: n as f32 / 1024.0, cs: c.cell_size(), sim: c.sim_cell_size(), first, second, up }
    }

    fn at(&self, x: i32, z: i32) -> usize {
        let last = self.n as i32 - 1;
        (z.clamp(0, last) * self.n as i32 + x.clamp(0, last)) as usize
    }

    fn xz(&self, i: usize) -> (i32, i32) {
        ((i % self.n) as i32, (i / self.n) as i32)
    }

    /// Continent-local voxel position of a cell's middle.
    fn centre(&self, i: usize) -> Vec2 {
        let (x, z) = self.xz(i);
        Vec2::new((x as f32 + 0.5) * self.cs, (z as f32 + 0.5) * self.cs)
    }

    /// The cell under a continent-local voxel position.
    fn cell_of(&self, p: Vec2) -> usize {
        self.at((p.x / self.cs) as i32, (p.y / self.cs) as i32)
    }

    /// A distance in voxels, in whole cells of this grid, at least one.
    fn cells(&self, d: f32) -> i32 {
        ((d / self.cs).round() as i32).max(1)
    }

    /// Discharge as it would be counted on the 1024 grid.
    fn q(&self, i: usize) -> f32 {
        self.c.discharge[i] / (self.scale * self.scale)
    }

    fn river(&self, i: usize) -> bool {
        !self.c.ocean[i] && self.c.lake[i] == NO_LAKE && self.c.discharge[i] >= MIN_RIVER_Q
    }

    /// Where a stream comes out of the ground: a river cell no river flows into.
    fn spring(&self, i: usize) -> bool {
        self.river(i) && self.up[i] == u32::MAX
    }

    /// Where a river turns sharply, between a little way up it and a little way down.
    fn bend(&self, i: usize) -> bool {
        let (mut a, mut b) = (i, i);
        for _ in 0..3 {
            if self.up[a] != u32::MAX {
                a = self.up[a] as usize;
            }
            b = self.c.rcv[b] as usize;
        }
        if a == i || b == i {
            return false;
        }
        let (p, q, r) = (self.centre(a), self.centre(i), self.centre(b));
        (q - p).normalize_or_zero().dot((r - q).normalize_or_zero()) < 0.3
    }

    /// Where two real rivers join, not a brook into a river.
    fn confluence(&self, i: usize) -> bool {
        let second = self.second[i] / (self.scale * self.scale);
        second >= 250.0 && self.second[i] >= self.first[i] * 0.3
    }

    /// Rise over run at a cell, steepest to its four neighbours.
    fn steep(&self, x: i32, z: i32) -> f32 {
        let h = &self.c.height;
        let here = h[self.at(x, z)];
        [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().map(|&(dx, dz)| (h[self.at(x + dx, z + dz)] - here).abs()).fold(0.0f32, f32::max) / self.sim
    }

    /// Rise over run across the ground about a cell, `r` cells out each way.
    fn broad(&self, x: i32, z: i32, r: i32) -> f32 {
        let h = &self.c.height;
        let here = h[self.at(x, z)];
        let ring = [(r, 0), (-r, 0), (0, r), (0, -r), (r, r), (r, -r), (-r, r), (-r, -r)];
        ring.iter().map(|&(dx, dz)| (h[self.at(x + dx, z + dz)] - here).abs()).sum::<f32>() / 8.0 / (r as f32 * self.sim)
    }

    /// Ground people could live on: dry land that is not bog, cliff, bare mountain or
    /// open dune sea.
    fn habitable(&self, i: usize) -> bool {
        let c = self.c;
        if c.ocean[i] || c.lake[i] != NO_LAKE || c.height[i] < 1.0 {
            return false;
        }
        let biome = Biome::from_u8(c.biome[i]);
        if fertility(biome) <= 0.0 {
            return false;
        }
        let (x, z) = self.xz(i);
        self.steep(x, z) < 0.3
    }

    /// Scores the spot at cell `i` for a settlement, or None if nobody would live there.
    fn survey(&self, i: usize, islands: bool, seed: u32) -> Option<Spot> {
        let c = self.c;
        if !self.habitable(i) || c.discharge[i] >= MIN_RIVER_Q {
            return None;
        }
        let (x, z) = self.xz(i);
        let here = c.height[i];
        let biome = Biome::from_u8(c.biome[i]);
        let (steep, broad) = (self.steep(x, z), self.broad(x, z, self.cells(m(80.0))));
        if steep > 0.25 || broad > 0.18 {
            return None;
        }
        // The water about it: within a short walk, and the sea a little further.
        let near = self.cells(m(80.0));
        let (mut river_q, mut bend, mut join, mut spring, mut lake) = (0.0f32, false, false, false, false);
        for dz in -near..=near {
            for dx in -near..=near {
                if dx * dx + dz * dz > near * near {
                    continue;
                }
                let j = self.at(x + dx, z + dz);
                if c.lake[j] != NO_LAKE && !c.ocean[j] {
                    lake = true;
                }
                if !self.river(j) {
                    continue;
                }
                river_q = river_q.max(self.q(j));
                bend |= self.q(j) >= 200.0 && self.bend(j);
                join |= self.confluence(j);
                spring |= self.spring(j);
            }
        }
        let coastal = self.cells(m(150.0));
        let sea = (-coastal..=coastal).any(|dz| (-coastal..=coastal).any(|dx| c.ocean[self.at(x + dx, z + dz)]));
        // A bay: sea close by, but land on most of the way round further out.
        let out = self.cells(m(420.0));
        let open = (0..16)
            .filter(|&k| {
                let a = k as f32 * std::f32::consts::TAU / 16.0;
                c.ocean[self.at(x + (a.cos() * out as f32).round() as i32, z + (a.sin() * out as f32).round() as i32)]
            })
            .count();
        let bay = sea && (1..=5).contains(&open);
        if islands && !sea {
            return None;
        }
        let fresh = river_q > 0.0 || lake || spring;
        let desert = matches!(biome, Biome::Desert | Biome::RedRock);
        if desert && !fresh {
            return None;
        }
        // The low gap of a ridge, with higher ground either side.
        let r = self.cells(m(150.0));
        let h = |dx: i32, dz: i32| c.height[self.at(x + dx, z + dz)];
        let pass = here > 40.0 && {
            let (e, w, s, n) = (h(r, 0), h(-r, 0), h(0, r), h(0, -r));
            (e.max(w) < here && s.min(n) > here + 4.0) || (s.max(n) < here && e.min(w) > here + 4.0)
        };
        let ford = (40.0..900.0).contains(&river_q);
        let bridge = (900.0..6000.0).contains(&river_q);

        let water = if join {
            34.0
        } else if bend {
            30.0
        } else if river_q > 0.0 {
            24.0
        } else if lake {
            28.0
        } else if spring {
            22.0
        } else {
            0.0
        };
        let fertile = fertility(biome) * 20.0 * (1.0 - (c.rain[i] - 1.0).abs().min(1.0) * 0.4);
        let level = (1.0 - broad / 0.18) * 12.0 + (1.0 - steep / 0.25) * 4.0;
        let high = (here - 80.0).max(0.0) * 0.08;
        let rng = unit(hash2(seed ^ 0x5e71, x, z));
        // The coast draws a village only a little: its people farm and fish the rivers as
        // much as the sea, and a bay matters most to a town's port.
        let score = water
            + fertile
            + level
            + if ford || bridge { 6.0 } else { 0.0 }
            + if bay { 6.0 } else if sea { 2.0 } else { 0.0 }
            + if pass { 6.0 } else { 0.0 }
            - high
            + rng * 12.0;
        // A town wants a big river to bridge, where rivers meet, or a bay for a port, on
        // broad level ground low down.
        let town = if (fresh || sea) && !islands {
            let big = if river_q > 100.0 { (river_q / 100.0).ln() * 6.0 } else { 0.0 };
            score + big + if bridge { 10.0 } else { 0.0 } + if join { 10.0 } else { 0.0 } + if bay { 16.0 } else if sea { 4.0 } else { 0.0 } + level - high
        } else {
            -1.0
        };
        let reason = if desert {
            Reason::Oasis
        } else if join {
            Reason::Confluence
        } else if bend {
            Reason::Bend
        } else if ford || bridge {
            Reason::Ford
        } else if river_q > 0.0 {
            Reason::River
        } else if lake {
            Reason::LakeShore
        } else if spring {
            Reason::Spring
        } else if bay {
            Reason::Bay
        } else if sea {
            Reason::Shore
        } else if pass {
            Reason::Pass
        } else {
            Reason::Open
        };
        Some(Spot { cell: i, score, town, reason, sea })
    }

    /// Square kilometres of ground people could live on.
    fn habitable_km2(&self, comp: &[u32]) -> f32 {
        let stride = 2;
        let mut count = 0usize;
        for z in (0..self.n).step_by(stride) {
            for x in (0..self.n).step_by(stride) {
                let i = z * self.n + x;
                if comp[i] != 0 && self.habitable(i) {
                    count += 1;
                }
            }
        }
        let km = self.cs / VOXELS_PER_METRE / 1000.0;
        count as f32 * (stride * stride) as f32 * km * km
    }

    /// Shares of the country within `reach` of `p` that are each kind of ground.
    fn country(&self, p: Vec2, reach: f32) -> Country {
        let c = self.c;
        let (cx, cz) = ((p.x / self.cs) as i32, (p.y / self.cs) as i32);
        let r = self.cells(reach);
        let step = (r / 12).max(1);
        let mut k = Country::default();
        let mut count = 0.0;
        for dz in (-r..=r).step_by(step as usize) {
            for dx in (-r..=r).step_by(step as usize) {
                if dx * dx + dz * dz > r * r {
                    continue;
                }
                let (x, z) = (cx + dx, cz + dz);
                let i = self.at(x, z);
                if c.ocean[i] {
                    continue;
                }
                count += 1.0;
                let biome = Biome::from_u8(c.biome[i]);
                use Biome::*;
                if matches!(biome, Deciduous | Woodland | BirchGrove | Taiga | SnowyPine | OldGrowth | Rainforest) {
                    k.wood += 1.0;
                }
                if matches!(biome, Marsh | Mangrove) {
                    k.willow += 1.0;
                }
                if matches!(biome, Steppe | Savanna | Desert | RedRock | Tundra | DuneSea) || (biome == Grassland && c.rain[i] < 0.8) {
                    k.dry += 1.0;
                }
                if matches!(biome, Grassland | Heath | Moor | AlpineMeadow) {
                    k.meadow += 1.0;
                }
                if matches!(biome, Moor | RedRock | Volcanic | Alpine) || self.steep(x, z) > 0.6 {
                    k.rock += 1.0;
                }
            }
        }
        if count > 0.0 {
            for v in [&mut k.wood, &mut k.willow, &mut k.dry, &mut k.meadow, &mut k.rock] {
                *v /= count;
            }
        }
        k
    }

    /// The biggest river or lake water within `reach` of `p`, as discharge on the 1024
    /// grid, and its cell.
    fn water_near(&self, p: Vec2, reach: f32) -> Option<(f32, usize)> {
        let (cx, cz) = ((p.x / self.cs) as i32, (p.y / self.cs) as i32);
        let r = self.cells(reach);
        let mut best: Option<(f32, usize)> = None;
        for dz in -r..=r {
            for dx in -r..=r {
                if dx * dx + dz * dz > r * r {
                    continue;
                }
                let j = self.at(cx + dx, cz + dz);
                if self.c.ocean[j] || (self.c.discharge[j] < MIN_RIVER_Q && self.c.lake[j] == NO_LAKE) {
                    continue;
                }
                let q = self.q(j);
                if best.is_none_or(|b| q > b.0) {
                    best = Some((q, j));
                }
            }
        }
        best
    }

    /// Whether the river from cell `i` to the one it drains into is calm: no waterfall
    /// and no stair of cascades.
    fn calm(&self, i: usize) -> bool {
        let c = self.c;
        let r = c.rcv[i] as usize;
        if c.lake[i] != NO_LAKE && c.lake[r] != NO_LAKE {
            return true;
        }
        if fall_drop(c, i) > 0.0 {
            return false;
        }
        let run = self.centre(i).distance(self.centre(r)) / self.cs * self.sim;
        (c.water_level[i] - c.water_level[r]) / run.max(1.0) <= CALM
    }
}

/// What the country about a settlement is made of, as shares of the land.
#[derive(Default, Debug)]
struct Country {
    wood: f32,
    willow: f32,
    dry: f32,
    meadow: f32,
    rock: f32,
}

/// How good a biome's ground is to live on and grow food in; 0 where nobody settles.
fn fertility(biome: Biome) -> f32 {
    match biome {
        Biome::Grassland => 1.0,
        Biome::Deciduous | Biome::Woodland => 0.85,
        Biome::BirchGrove | Biome::Steppe => 0.7,
        Biome::OldGrowth => 0.6,
        Biome::Savanna | Biome::Heath | Biome::Rainforest => 0.55,
        Biome::Taiga | Biome::Marsh => 0.5,
        Biome::Moor | Biome::Volcanic => 0.4,
        Biome::Mangrove | Biome::AlpineMeadow => 0.35,
        Biome::Tundra => 0.3,
        Biome::RedRock | Biome::SnowyPine => 0.2,
        Biome::Desert => 0.15,
        Biome::Bog | Biome::DuneSea | Biome::Alpine | Biome::Ocean => 0.0,
    }
}

/// Where a settlement settled on real ground.
struct Settled {
    x: i32,
    z: i32,
    /// Mean ground height under the green.
    y: f32,
    biome: Biome,
    /// The nearest fresh water within [`FETCH`], and what it is.
    water: Option<(Vec2, Water)>,
}

/// Spacing of the grid of spots looked at, in voxels.
const STEP: i32 = 12;
/// How far from its map cell's middle a settlement may move.
const SEARCH: f32 = m(80.0);
/// Steepest rise its green may straddle.
const RISE: f32 = m(2.8);

/// Finds the spot near map cell `cell` where a settlement's green can lie level, as near
/// fresh water as is comfortable and no nearer: a short walk from the stream, clear of
/// its banks. None when no spot near can take the green.
fn settle(c: &Continent, seed: u32, kind: Kind, cell: usize) -> Option<Settled> {
    let cs = c.cell_size();
    let n = c.size;
    let centre = Vec2::new(((cell % n) as f32 + 0.5) * cs, ((cell / n) as f32 + 0.5) * cs);
    let reach = SEARCH as i32 / STEP * STEP;
    let side = reach / STEP * 2 + 1;
    let (x0, z0) = (centre.x as i32 - reach, centre.y as i32 - reach);
    let green = kind.green();
    let pad = green as i32 + 2;
    let sampler = Sampler::new(c, seed, x0 - pad, z0 - pad, reach * 2 + pad * 2 + 1);
    struct Spot {
        p: Vec2,
        ground: f32,
        land: bool,
    }
    let mut spots = Vec::with_capacity((side * side) as usize);
    let mut water: Vec<(Vec2, Water)> = Vec::new();
    for j in 0..side {
        for i in 0..side {
            let (x, z) = (x0 + i * STEP, z0 + j * STEP);
            let col = sampler.column(x, z);
            let p = Vec2::new(x as f32 + 0.5, z as f32 + 0.5);
            let wet = col.water_surface().is_some_and(|k| k > 0);
            if wet && col.cover == Cover::LakeBed {
                water.push((p, Water::Lake));
            } else if (wet && col.cover != Cover::SeaFloor) || col.river < STEP as f32 * 0.6 {
                water.push((p, Water::River));
            }
            spots.push(Spot {
                p,
                ground: col.ground,
                land: col.water_surface().is_none() && col.river >= m(4.8) && matches!(col.cover, Cover::Land | Cover::Beach),
            });
        }
    }
    water.extend(sampler.springs().iter().map(|s| (s.pos, Water::Spring)));
    water.extend(sampler.heads().iter().map(|&h| (h, Water::Spring)));
    let nearest_water = |p: Vec2| -> Option<(f32, Vec2, Water)> {
        water.iter().map(|&(w, kind)| (w.distance(p), w, kind)).min_by(|a, b| a.0.total_cmp(&b.0))
    };
    let at = |i: i32, j: i32| -> Option<&Spot> { (i >= 0 && j >= 0 && i < side && j < side).then(|| &spots[(j * side + i) as usize]) };
    let r = (green / STEP as f32).ceil() as i32;
    let mut scored: Vec<(f32, i32, i32)> = Vec::new();
    for j in 0..side {
        for i in 0..side {
            let s = &spots[(j * side + i) as usize];
            if !s.land || s.p.distance(centre) > reach as f32 {
                continue;
            }
            let (mut lo, mut hi, mut dry) = (s.ground, s.ground, true);
            for dj in -r..=r {
                for di in -r..=r {
                    if let Some(q) = at(i + di, j + dj) {
                        lo = lo.min(q.ground);
                        hi = hi.max(q.ground);
                        dry &= q.land;
                    }
                }
            }
            if !dry || hi - lo > RISE * 1.3 {
                continue;
            }
            let rough = (hi - lo - m(1.0)).max(0.0);
            let off = s.p.distance(centre) / reach as f32;
            // Close enough to fetch water from, but back from the bank, out of the floods.
            let fetch = nearest_water(s.p).map_or(0.0, |(d, _, _)| if d < FETCH { m(8.0) - (d - m(30.0)).abs() * 0.2 } else { 0.0 });
            scored.push((fetch - rough * 3.0 - off * m(3.0), i, j));
        }
    }
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then((a.2, a.1).cmp(&(b.2, b.1))));
    let probe = |x: i32, z: i32| -> Option<f32> {
        let rr = green as i32 + 1;
        let d = rr * 2 / 3;
        let rim = [(0, 0), (rr, 0), (-rr, 0), (0, rr), (0, -rr), (d, d), (-d, d), (d, -d), (-d, -d)];
        let (mut lo, mut hi, mut sum) = (f32::MAX, f32::MIN, 0.0);
        for (dx, dz) in rim {
            let col = sampler.column(x + dx, z + dz);
            if col.water_surface().is_some() || col.river < m(4.8) || !matches!(col.cover, Cover::Land | Cover::Beach) {
                return None;
            }
            lo = lo.min(col.ground);
            hi = hi.max(col.ground);
            sum += col.ground;
        }
        (hi - lo <= RISE).then_some(sum / rim.len() as f32)
    };
    for &(_, i, j) in scored.iter().take(24) {
        let s = &spots[(j * side + i) as usize];
        let (x, z) = (s.p.x as i32, s.p.y as i32);
        if let Some(y) = probe(x, z) {
            let water = nearest_water(s.p).filter(|w| w.0 < FETCH).map(|w| (w.1, w.2));
            return Some(Settled { x, z, y, biome: sampler.column(x, z).biome, water });
        }
    }
    None
}

/// What a settlement lives by, from the country about it.
fn livelihoods(map: &Map, p: Vec2, kind: Kind, coast: bool, buildings: u8, roll: impl Fn(u32) -> f32) -> Vec<Livelihood> {
    let k = map.country(p, LIVING);
    let mut out = Vec::new();
    let near = map.water_near(p, m(300.0));
    // A river big enough to fish, or a lake.
    let fishing = near.is_some_and(|(q, j)| q >= 60.0 || map.c.lake[j] != NO_LAKE);
    // Willows grow along the rivers of all but the hottest, driest country, enough of
    // them to cut for baskets and arrows only along a broad river close by.
    let river = map.water_near(p, m(150.0)).is_some_and(|(q, _)| q >= 1500.0);
    let warm_bank = river && {
        let i = map.cell_of(p);
        let biome = Biome::from_u8(map.c.biome[i]);
        map.c.temperature[i] > 0.3 && !matches!(biome, Biome::Desert | Biome::Savanna | Biome::DuneSea)
    };
    let willows = k.willow >= 0.05 || warm_bank;
    for l in Livelihood::ALL {
        let has = match l {
            Livelihood::Woodcutter => k.wood >= 0.25,
            Livelihood::Sawmill | Livelihood::LoggingCamp => k.wood >= 0.25 && buildings > 15,
            // Baskets are woven in nearly every place where the willows grow thick, and in
            // about half of those with a few to hand.
            Livelihood::Weaver => willows && (k.willow >= 0.15 || roll(7) < 0.45),
            // Arrows are made in fewer, and only where there is game in the woods or on the
            // open country to shoot (#50).
            Livelihood::Fletcher => willows && kind >= Kind::Village && (k.dry >= 0.15 || k.wood >= 0.4) && roll(8) < 0.6,
            Livelihood::Fisher => fishing || coast,
            Livelihood::Port => coast && kind >= Kind::Town,
            Livelihood::Hunter => k.dry >= 0.3,
            Livelihood::Apiary => k.meadow >= 0.2,
            Livelihood::Quarry => k.rock >= 0.06,
            Livelihood::Forager => true,
            Livelihood::Smithy => kind >= Kind::LargeVillage,
        };
        if has {
            out.push(l);
        }
    }
    out
}

/// A settlement chosen and settled, before its facts are worked out.
struct Chosen {
    kind: Kind,
    spot: Spot,
    settled: Settled,
    parent: Option<usize>,
}

/// Whether `p` is at least `gap` from every settlement chosen so far.
fn far_from(chosen: &[Chosen], p: Vec2, gap: f32) -> bool {
    chosen.iter().all(|o| Vec2::new(o.settled.x as f32, o.settled.z as f32).distance(p) >= gap)
}

/// Places every settlement on a continent: towns, then villages, then the hamlets about
/// them. `comp` labels connected land as [`crate::trails::sites::land_components`] does.
pub fn place(c: &Continent, world_seed: u32, comp: &[u32]) -> Placed {
    let map = Map::new(c);
    let n = map.n as i32;
    let islands = c.desc.kind == CellKind::Islands;
    let seed = c.desc.seed;
    let roll = |salt: i32| unit(hash2(seed ^ 0x5e77_1e, salt, 3));
    let land = map.habitable_km2(comp);
    let stride = (n / 256).max(1) as usize;
    let margin = map.cells(m(500.0)).min(n / 4);
    let mut spots: Vec<Spot> = Vec::new();
    for z in (margin..n - margin).step_by(stride) {
        for x in (margin..n - margin).step_by(stride) {
            let i = map.at(x, z);
            if comp[i] == 0 {
                continue;
            }
            if let Some(s) = map.survey(i, islands, seed) {
                spots.push(s);
            }
        }
    }

    // How many of each: as many villages as the good land holds, up to nine; a town or
    // two on a continent, none on islands; and now and then a great town.
    let villages = ((land / LAND_PER_VILLAGE * (0.85 + 0.3 * roll(1))).round() as usize).min(MOST_VILLAGES);
    let towns = if islands || land < LAND_FOR_TOWN {
        0
    } else if land >= LAND_FOR_TWO_TOWNS {
        2
    } else {
        1
    };
    let great = towns > 0 && land >= LAND_FOR_GREAT_TOWN && roll(3) < GREAT_TOWN_CHANCE;

    let mut chosen: Vec<Chosen> = Vec::new();
    let try_place =|chosen: &mut Vec<Chosen>, list: &[Spot], kind: Kind, parent: Option<usize>, ok: &dyn Fn(&[Chosen], Vec2) -> bool| -> bool {
        let mut tried = 0;
        for s in list {
            if tried >= TRIES {
                break;
            }
            if !ok(chosen, map.centre(s.cell)) {
                continue;
            }
            tried += 1;
            let Some(settled) = settle(c, world_seed, kind, s.cell) else { continue };
            if !ok(chosen, Vec2::new(settled.x as f32, settled.z as f32)) {
                continue;
            }
            chosen.push(Chosen { kind, spot: *s, settled, parent });
            return true;
        }
        false
    };

    let mut by_town: Vec<Spot> = spots.iter().filter(|s| s.town >= 0.0).copied().collect();
    by_town.sort_by(|a, b| b.town.total_cmp(&a.town).then(a.cell.cmp(&b.cell)));
    // The biggest rivers are biggest at their mouths, so the best ground for a town is
    // nearly always by the sea. Once one town has its port, the next stands inland where
    // it can, where rivers meet or a big one is crossed, so that not every town is a port.
    let inland: Vec<Spot> = by_town.iter().filter(|s| !s.sea).copied().collect();
    for k in 0..towns {
        let kind = if great && k == 0 { Kind::GreatTown } else { Kind::Town };
        let apart = |chosen: &[Chosen], p: Vec2| far_from(chosen, p, TOWN_GAP);
        let port = chosen.iter().any(|c| c.kind >= Kind::Town && c.spot.sea);
        if !(port && try_place(&mut chosen, &inland, kind, None, &apart)) {
            try_place(&mut chosen, &by_town, kind, None, &apart);
        }
    }
    // Each village in turn on the best ground left, favouring ground well away from the
    // settlements already placed, so that they spread over the land rather than
    // gathering along the richest stretch of it.
    for _ in 0..villages {
        let mut ranked: Vec<(f32, Spot)> = spots
            .iter()
            .map(|s| {
                let p = map.centre(s.cell);
                let apart = chosen.iter().map(|o| Vec2::new(o.settled.x as f32, o.settled.z as f32).distance(p)).fold(f32::MAX, f32::min);
                (s.score + (apart / m(1000.0)).min(SPREAD_KM) * SPREAD, *s)
            })
            .collect();
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cell.cmp(&b.1.cell)));
        let list: Vec<Spot> = ranked.into_iter().map(|r| r.1).collect();
        if !try_place(&mut chosen, &list, Kind::Village, None, &|chosen, p| far_from(chosen, p, VILLAGE_GAP)) {
            break;
        }
    }
    // The best placed and luckiest few villages have grown large: about a third of them
    // on a continent, none out on the islands.
    let towns_placed = chosen.iter().filter(|s| s.kind >= Kind::Town).count();
    let mut village_rank: Vec<usize> = (towns_placed..chosen.len()).collect();
    village_rank.sort_by(|&a, &b| {
        let key = |k: usize| chosen[k].spot.score + roll(100 + k as i32) * 10.0;
        key(b).total_cmp(&key(a)).then(a.cmp(&b))
    });
    if !islands {
        let large = (village_rank.len() as f32 * 0.3).round() as usize;
        for &k in village_rank.iter().take(large) {
            chosen[k].kind = Kind::LargeVillage;
        }
    }

    // Hamlets: one to three about each village and town, a short walk out, on the best
    // ground there that keeps clear of every other settlement.
    let parents = chosen.len();
    for p in 0..parents {
        let home = Vec2::new(chosen[p].settled.x as f32, chosen[p].settled.z as f32);
        let h = hash2(seed ^ 0x4a31, p as i32, chosen[p].settled.x);
        let count = if chosen[p].kind >= Kind::Town { 2 + h % 2 } else { 1 + h % 3 };
        let mut near: Vec<Spot> = spots
            .iter()
            .filter(|s| {
                let d = map.centre(s.cell).distance(home);
                d >= HAMLET_NEAR && d <= HAMLET_FAR
            })
            .copied()
            .collect();
        // Nearer is likelier: the best ground within a kilometre or so.
        let key = |s: &Spot| s.score - (map.centre(s.cell).distance(home) - m(1000.0)).max(0.0) / m(100.0) * 2.0;
        near.sort_by(|a, b| key(b).total_cmp(&key(a)).then(a.cell.cmp(&b.cell)));
        for _ in 0..count {
            let ok = |chosen: &[Chosen], q: Vec2| {
                let d = q.distance(home);
                d >= HAMLET_NEAR && d <= HAMLET_FAR && far_from(chosen, q, HAMLET_GAP)
            };
            if !try_place(&mut chosen, &near, Kind::Hamlet, Some(p), &ok) {
                break;
            }
        }
    }

    // The facts of each, from its spot, the ground it settled on and its luck.
    let mut placed = Placed::default();
    let best = chosen.iter().map(|s| s.spot.score).fold(f32::MIN, f32::max);
    let worst = chosen.iter().map(|s| s.spot.score).fold(f32::MAX, f32::min);
    for s in &chosen {
        let (x, z) = (s.settled.x, s.settled.z);
        let p = Vec2::new(x as f32 + 0.5, z as f32 + 0.5);
        let own = hash2(seed ^ 0x5e77_1ed, x, z);
        let r = |salt: u32| unit(hash2(own, salt as i32, 5));
        let kind = s.kind;
        let good = if best > worst { (s.spot.score - worst) / (best - worst) } else { 0.5 };
        let prosperity = (0.2 + 0.45 * good + 0.35 * r(1)).clamp(0.05, 0.98);
        let (b0, b1) = kind.buildings();
        let built = (0.65 * prosperity + 0.35 * r(2)).clamp(0.0, 1.0);
        let buildings = b0 + ((b1 - b0) as f32 * built).round() as u8;
        let (p0, p1) = kind.people();
        let full = (built + (r(3) - 0.5) * 0.3).clamp(0.0, 1.0);
        let people = p0 + ((p1 - p0) as f32 * full).round() as u8;
        // Towns are mostly old, hamlets mostly new.
        let (young, old) = match kind {
            Kind::Hamlet => (0.5, 0.1),
            Kind::Village => (0.25, 0.25),
            Kind::LargeVillage => (0.15, 0.35),
            Kind::Town => (0.05, 0.5),
            Kind::GreatTown => (0.0, 0.7),
        };
        let a = r(4);
        let age = if a < young {
            Age::Young
        } else if a >= 1.0 - old {
            Age::Ancient
        } else {
            Age::Established
        };
        let (newest, oldest) = match age {
            Age::Young => (5, 60),
            Age::Established => (60, 300),
            Age::Ancient => (300, 1000),
        };
        let mut founded = -(newest + ((oldest - newest) as f32 * r(5)) as i32);
        // A hamlet grew from its village, so it is never the older.
        if let Some(parent) = s.parent
            && let Some(home) = placed.settlements.get(parent)
        {
            founded = founded.max(home.founded + 5).min(-1);
        }
        let coast = {
            let i = map.cell_of(p);
            let (cx, cz) = map.xz(i);
            let r = map.cells(m(150.0));
            (-r..=r).any(|dz| (-r..=r).any(|dx| c.ocean[map.at(cx + dx, cz + dz)]))
        };
        let water = match s.settled.water {
            Some((_, w)) => w,
            None if coast => Water::Sea,
            None => Water::Wells,
        };
        let wells = if !water.fresh() {
            ((people as f32 / 30.0).round() as u8).max(1)
        } else if kind >= Kind::Town && matches!(water, Water::River | Water::Lake) {
            1 + (r(6) < 0.5) as u8
        } else {
            0
        };
        let livelihoods = livelihoods(&map, p, kind, coast, buildings, r);
        let river = map.water_near(p, m(80.0)).filter(|&(q, _)| q >= BOAT_Q).map(|(_, j)| j);
        placed.settlements.push(Settlement {
            kind,
            age,
            founded,
            prosperity,
            region: Region::of(s.settled.biome),
            coast,
            water,
            wells,
            livelihoods,
            buildings,
            people,
            x,
            z,
            y: s.settled.y.round() as i32 + 1,
            radius: kind.radius(),
            seed: own,
            reason: s.spot.reason,
            parent: s.parent,
            partners: Vec::new(),
            river,
            port: None,
            shrine: None,
        });
        // A green kept by the people about it: trodden bare across the middle where the
        // ways in meet, the more so the busier the place.
        let mut green = Pad::plain(p.x, p.y, kind.green(), s.settled.y.round());
        green.wear = (0.35 + 0.1 * kind as u8 as f32 + 0.2 * prosperity).min(1.0);
        green.hub = kind.green() * 0.3;
        placed.greens.push(green);
        placed.cells.push(s.spot.cell);
        placed.water.push(s.settled.water.map(|w| w.0));
    }
    partners(&map, &mut placed.settlements);
    placed
}

/// Settlements on the same calm river: following the water down from each one's river
/// finds any settlement further down it, unless a waterfall or a stair of cascades lies
/// between.
fn partners(map: &Map, list: &mut [Settlement]) {
    let c = map.c;
    let mut on: std::collections::HashMap<usize, Vec<usize>> = std::collections::HashMap::new();
    for (k, s) in list.iter().enumerate() {
        if let Some(r) = s.river {
            on.entry(r).or_default().push(k);
        }
    }
    let mut pairs = Vec::new();
    for (k, s) in list.iter().enumerate() {
        let Some(mut i) = s.river else { continue };
        // Any others by the very same stretch, then any further down.
        pairs.extend(on.get(&i).into_iter().flatten().filter(|&&o| o != k).map(|&o| (k, o)));
        for _ in 0..map.n * 4 {
            let r = c.rcv[i] as usize;
            if r == i || c.ocean[r] || !map.calm(i) {
                break;
            }
            i = r;
            for &other in on.get(&i).into_iter().flatten() {
                if other != k {
                    pairs.push((k, other));
                }
            }
        }
    }
    for (a, b) in pairs {
        for (x, y) in [(a, b), (b, a)] {
            if !list[x].partners.contains(&y) {
                list[x].partners.push(y);
            }
        }
    }
    for s in list.iter_mut() {
        s.partners.sort_unstable();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{
        continent::SimConfig,
        features::landmarks::Landmark,
        trails::{build::{TrailClass, class_of}, sites::land_components},
        world::GameWorld,
    };

    /// The continents of a few small worlds, as the tests see them.
    fn continents() -> Vec<(u32, Arc<Continent>)> {
        (1..=4).map(|seed| (seed, GameWorld::with_config(seed, SimConfig { size: 256 }).continent(0, 0))).collect()
    }

    fn is_town(k: Kind) -> bool {
        k >= Kind::Town
    }

    #[test]
    fn settlements_are_the_same_each_time() {
        let key = |c: &Continent| {
            let names = &c.trails.names;
            c.trails
                .settlements
                .iter()
                .enumerate()
                .map(|(k, s)| (s.kind, s.x, s.z, s.y, s.founded, s.people, s.livelihoods.clone(), s.partners.clone(), names.settlement(k).map(|p| p.name.clone())))
                .collect::<Vec<_>>()
        };
        let a = GameWorld::with_config(3, SimConfig { size: 256 }).continent(0, 0);
        let b = GameWorld::with_config(3, SimConfig { size: 256 }).continent(0, 0);
        assert!(!a.trails.settlements.is_empty());
        assert_eq!(key(&a), key(&b));
    }

    #[test]
    fn settlements_keep_their_distances_and_numbers() {
        for (seed, c) in continents() {
            let list = &c.trails.settlements;
            let islands = c.desc.kind == CellKind::Islands;
            let count = |f: &dyn Fn(Kind) -> bool| list.iter().filter(|s| f(s.kind)).count();
            let villages = count(&|k| matches!(k, Kind::Village | Kind::LargeVillage));
            let towns = count(&is_town);
            println!("seed {seed}: {} settlements, {villages} villages, {towns} towns", list.len());
            assert!(villages <= MOST_VILLAGES, "seed {seed}: {villages} villages");
            assert!(towns <= 2 && count(&|k| k == Kind::GreatTown) <= 1, "seed {seed}: {towns} towns");
            if islands {
                assert_eq!(towns + count(&|k| k == Kind::LargeVillage), 0, "seed {seed}: islands have only hamlets and villages");
            }
            for (i, a) in list.iter().enumerate() {
                assert!((a.kind.buildings().0..=a.kind.buildings().1).contains(&a.buildings), "seed {seed}: {a:?}");
                assert!((a.kind.people().0..=a.kind.people().1).contains(&a.people), "seed {seed}: {a:?}");
                assert_eq!(a.parent.is_some(), a.kind == Kind::Hamlet, "seed {seed}: only hamlets have a parent");
                for b in &list[i + 1..] {
                    let d = a.at().distance(b.at());
                    if a.kind != Kind::Hamlet && b.kind != Kind::Hamlet {
                        assert!(d >= VILLAGE_GAP, "seed {seed}: {:?} and {:?} {:.0} m apart", a.kind, b.kind, d / VOXELS_PER_METRE as f32);
                    }
                    if is_town(a.kind) && is_town(b.kind) {
                        assert!(d >= TOWN_GAP, "seed {seed}: towns {:.0} m apart", d / VOXELS_PER_METRE as f32);
                    }
                    if a.kind == Kind::Hamlet || b.kind == Kind::Hamlet {
                        assert!(d >= HAMLET_GAP, "seed {seed}: a hamlet {:.0} m from another settlement", d / VOXELS_PER_METRE as f32);
                    }
                }
            }
            for (p, parent) in list.iter().enumerate().filter(|(_, s)| s.kind != Kind::Hamlet) {
                let hamlets = list.iter().filter(|s| s.parent == Some(p)).count();
                assert!(hamlets <= 3, "seed {seed}: {:?} has {hamlets} hamlets", parent.kind);
            }
        }
    }

    #[test]
    fn hamlets_stand_a_short_walk_from_their_village() {
        for (seed, c) in continents() {
            let list = &c.trails.settlements;
            for s in list.iter().filter(|s| s.kind == Kind::Hamlet) {
                let parent = &list[s.parent.expect("a hamlet has a parent")];
                assert_ne!(parent.kind, Kind::Hamlet);
                let d = s.at().distance(parent.at());
                assert!((HAMLET_NEAR..=HAMLET_FAR).contains(&d), "seed {seed}: a hamlet {:.0} m from its parent", d / VOXELS_PER_METRE as f32);
                assert!(s.founded > parent.founded, "seed {seed}: a hamlet older than its parent");
            }
        }
    }

    #[test]
    fn every_settlement_has_fresh_water_close_or_wells() {
        for (seed, c) in continents() {
            // The water each was settled by is kept only in what `place` gives back, so
            // place them again (on the finished ground, so a step or two from where the
            // continent has them).
            let placed = place(&c, seed, &land_components(&c));
            for (s, water) in placed.settlements.iter().zip(&placed.water) {
                if s.water.fresh() {
                    let w = water.unwrap_or_else(|| panic!("seed {seed}: {:?} by a {:?} with none found", s.kind, s.water));
                    let d = w.distance(s.at());
                    assert!(d <= FETCH, "seed {seed}: water {:.0} m from the middle", d / VOXELS_PER_METRE as f32);
                } else {
                    assert!(s.wells > 0, "seed {seed}: {:?} with no water and no well", s.kind);
                }
            }
        }
    }

    #[test]
    fn partners_share_one_river_with_no_falls_between() {
        let mut pairs = 0;
        for (seed, c) in continents() {
            let list = &c.trails.settlements;
            for (a, s) in list.iter().enumerate() {
                for &b in &s.partners {
                    assert!(list[b].partners.contains(&a), "seed {seed}: partners go both ways");
                    let (from, to) = (s.river.unwrap(), list[b].river.unwrap());
                    // One lies downstream of the other, and the water between runs calm.
                    let calm_to = |mut i: usize, end: usize| {
                        for _ in 0..c.size * 4 {
                            if i == end {
                                return true;
                            }
                            let r = c.rcv[i] as usize;
                            if r == i || c.ocean[r] || fall_drop(&c, i) > 0.0 {
                                return false;
                            }
                            i = r;
                        }
                        false
                    };
                    assert!(calm_to(from, to) || calm_to(to, from), "seed {seed}: settlements {a} and {b} are not on one calm river");
                    pairs += 1;
                }
            }
        }
        println!("{} pairs of river partners", pairs / 2);
    }

    #[test]
    fn landmarks_keep_out_of_settlements_unless_taken_in() {
        let mut taken = 0;
        for (seed, c) in continents() {
            for (k, site) in c.trails.sites.iter().enumerate() {
                let p = Vec2::new(site.x as f32 + 0.5, site.z as f32 + 0.5);
                let hosts: Vec<&Settlement> = c.trails.settlements.iter().filter(|s| s.absorbed(k)).collect();
                assert!(hosts.len() <= 1, "seed {seed}: landmark {k} taken in twice");
                for s in &c.trails.settlements {
                    let d = s.at().distance(p);
                    if s.absorbed(k) {
                        taken += 1;
                        assert!(d >= s.kind.green() && d <= s.radius + m(150.0), "seed {seed}: {:?} {:.0} m from its settlement", site.kind, d / VOXELS_PER_METRE as f32);
                        assert!(
                            match site.kind {
                                Landmark::Landing => s.coast && s.port == Some(k),
                                Landmark::Shrine => s.shrine == Some(k),
                                _ => false,
                            },
                            "seed {seed}: a {:?} taken in",
                            site.kind
                        );
                    } else {
                        assert!(d >= s.radius, "seed {seed}: a {:?} {:.0} m inside a {:?}", site.kind, d / VOXELS_PER_METRE as f32, s.kind);
                    }
                }
            }
        }
        println!("{taken} landmarks taken in");
    }

    #[test]
    fn the_way_between_two_towns_is_a_road() {
        let mut checked = 0;
        for (seed, c) in continents() {
            if c.trails.settlements.iter().filter(|s| is_town(s.kind)).count() < 2 {
                continue;
            }
            let road: f32 = c
                .trails
                .lines
                .iter()
                .zip(&c.trails.line_widths)
                .flat_map(|(line, widths)| line.windows(2).zip(widths).filter(|(_, w)| class_of(**w) == TrailClass::Road).map(|(p, _)| p[0].distance(p[1])))
                .sum();
            println!("seed {seed}: {:.1} km of road", road / VOXELS_PER_METRE as f32 / 1000.0);
            assert!(road > m(500.0), "seed {seed}: two towns and no road between them");
            checked += 1;
        }
        assert!(checked > 0, "no continent with two towns to check");
    }
}
