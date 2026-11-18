//! The settlement-influence field: how managed each spot of a continent is, from 0 where
//! the country is wild to 1 in a settlement's own yards (#201, #64).
//!
//! Every settlement has worked the land about it for as long as it has stood: the more
//! people, the older and the better off it is, the further its fields reach. How far a
//! spot is from a settlement is measured over the ground, the way people walk to work it,
//! so the land is worked up its valley before it is worked over the ridge, and a little
//! further along the roads; and what the ground is like decides how hard it is worked.
//! Gentle, fertile ground near fresh water is ploughed, a steep bank is only grazed, and
//! bog, bare rock and water are left as they are.
//!
//! The field is worked out once per continent, on a grid of cells [`CELL`] voxels across
//! kept only where some settlement reaches, so reading it in the wild costs one lookup.
//! Fields, hedges, trees and plants all read it, through [`Land::managed`] and
//! [`Land::land_use`].

use std::{cmp::Ordering, collections::BinaryHeap, collections::HashSet};

use glam::{Vec2, vec2};
use voxel::Voxel;

use crate::settlements::{Age, Kind, Region, Settlement};
use crate::{
    biome::Biome,
    continent::{Continent, NO_LAKE},
    features::landmarks::Palette,
    macro_layout::MACRO_SIZE,
    noise::fbm2,
    scale::{OLD_VOXEL, m, m64},
    surface::MIN_RIVER_Q,
};

/// Voxels along the edge of one cell of the field: 6.4 m, fine enough that the rings
/// follow the lie of the land, and coarse enough that a continent's worth is small.
pub const CELL: i32 = 16;
/// Cells along the edge of a tile. Tiles are kept only where a settlement reaches.
const TILE: i32 = 64;
/// Tiles along the edge of a continent.
const TILES: i32 = MACRO_SIZE / (CELL * TILE);
/// No settlement owns the cell.
const NOBODY: u8 = u8::MAX;

/// At or above this a spot is cropped: orchards and arable fields.
pub const CROPS: f32 = 0.70;
/// At or above this, pasture and hay meadow.
pub const PASTURE: f32 = 0.45;
/// At or above this, rough grazing and managed woods.
pub const ROUGH: f32 = 0.20;
/// Below this the country is wild: not even a stone of an old field edge is left.
pub const EDGE: f32 = 0.10;

/// The rings of land use, roughly from the middle of a settlement outward, broken up by
/// the ground (see #64).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Use {
    /// Inside a settlement's radius: yards, gardens and the houses, which the village
    /// layout lays out.
    Yard,
    /// Orchards and arable fields.
    Crops,
    /// Pasture and hay meadow.
    Pasture,
    /// Rough grazing and managed woods.
    Rough,
    /// The wild, with old field edges fading out into it.
    Wild,
}

impl Use {
    /// The ring a spot this managed (0 to 1) lies in, outside any settlement's radius.
    pub fn of(managed: f32) -> Use {
        match managed {
            m if m >= CROPS => Use::Crops,
            m if m >= PASTURE => Use::Pasture,
            m if m >= ROUGH => Use::Rough,
            _ => Use::Wild,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Use::Yard => "yards and gardens",
            Use::Crops => "orchards and crops",
            Use::Pasture => "pasture and hay meadow",
            Use::Rough => "rough grazing and managed woods",
            Use::Wild => "the wild",
        }
    }
}

/// What the fields about one settlement need to know of it: the facts that decide how
/// far its land reaches, and what it builds its walls and fences of.
#[derive(Clone, Debug)]
pub struct Farm {
    /// Its middle, in continent-local voxels.
    pub at: Vec2,
    /// Its own radius, inside which the village layout has the ground.
    pub radius: f32,
    /// How far beyond its radius its land reaches over easy ground, in voxels.
    pub reach: f32,
    pub kind: Kind,
    pub age: Age,
    pub region: Region,
    pub prosperity: f32,
    pub coast: bool,
    pub seed: u32,
    /// The biome at its middle, and how warm it is there (0 polar, 1 tropical).
    pub biome: Biome,
    pub temperature: f32,
    /// The stone of the place (see [`Palette::at`]): its walls are built of it.
    pub stone: Voxel,
    pub stone_dark: Voxel,
    /// The timber of the trees round about, for posts and rails.
    pub wood: Voxel,
}

impl Farm {
    fn of(c: &Continent, seed: u32, s: &Settlement) -> Farm {
        let at = s.at();
        let ground = sample(c, at);
        let people = s.people as f32;
        let age = match s.age {
            Age::Young => 0.72,
            Age::Established => 1.0,
            Age::Ancient => 1.2,
        };
        let reach = (m(120.0) + m(20.0) * people.sqrt()) * age * (0.85 + 0.3 * s.prosperity);
        let (wx, wz) = (c.desc.mx * MACRO_SIZE + s.x, c.desc.mz * MACRO_SIZE + s.z);
        let pal = Palette::at(seed, ground.biome, wx, s.y - 1, wz);
        Farm {
            at,
            radius: s.radius,
            reach,
            kind: s.kind,
            age: s.age,
            region: s.region,
            prosperity: s.prosperity,
            coast: s.coast,
            seed: s.seed,
            biome: ground.biome,
            temperature: ground.temperature,
            stone: pal.stone,
            stone_dark: pal.stone_dark,
            wood: crate::features::timber(ground.biome).unwrap_or(crate::materials::WOOD_DEAD),
        }
    }

    /// Whether continent-local `p` is inside its radius, where the layout has the ground.
    pub fn holds(&self, p: Vec2) -> bool {
        self.at.distance_squared(p) < self.radius * self.radius
    }
}

/// One tile of the field: how managed each cell is (0 to 255), and whose land it is.
struct Tile {
    managed: [u8; (TILE * TILE) as usize],
    owner: [u8; (TILE * TILE) as usize],
}

/// The settlement-influence field of one continent.
#[derive(Default)]
pub struct Land {
    /// `TILES * TILES` tiles, row by row; `None` where no settlement reaches, and empty
    /// on a continent with no settlements.
    tiles: Vec<Option<Box<Tile>>>,
    /// One for each settlement, in the order of `Network::settlements`.
    pub farms: Vec<Farm>,
    /// The barns out in the fields, each with the settlement it serves (see
    /// [`super::farmstead`]); raised once the land is worked out.
    pub steads: Vec<super::farmstead::Stead>,
    /// The quarries, each with the settlement it serves (see [`super::quarry`]); cut once
    /// the barns are raised (#203).
    pub quarries: Vec<super::quarry::Quarry>,
    /// What stands about the fields: haystacks, scarecrows, shelters, troughs, wells,
    /// dovecotes and skeps (see [`super::fixtures`]); set out last (#64, #206).
    pub fixtures: super::fixtures::Fixtures,
}

impl Land {
    /// The cell `(i, j)`: how managed, 0 to 255, and whose.
    #[inline]
    fn cell(&self, i: i32, j: i32) -> (u8, u8) {
        if self.tiles.is_empty() || i < 0 || j < 0 || i >= TILES * TILE || j >= TILES * TILE {
            return (0, NOBODY);
        }
        match &self.tiles[((j / TILE) * TILES + i / TILE) as usize] {
            None => (0, NOBODY),
            Some(t) => {
                let k = ((j % TILE) * TILE + i % TILE) as usize;
                (t.managed[k], t.owner[k])
            }
        }
    }

    /// Whether no settlement reaches anywhere in the square of continent-local columns
    /// from `(lx0, lz0)`, `span` wide: the quick way for a chunk out in the wild to know
    /// it has nothing to do.
    pub fn wild_square(&self, lx0: i32, lz0: i32, span: i32) -> bool {
        if self.tiles.is_empty() {
            return true;
        }
        let tile = |v: i32| (v.div_euclid(CELL) - 1).div_euclid(TILE).clamp(0, TILES - 1);
        let tile_hi = |v: i32| (v.div_euclid(CELL) + 1).div_euclid(TILE).clamp(0, TILES - 1);
        for tj in tile(lz0)..=tile_hi(lz0 + span) {
            for ti in tile(lx0)..=tile_hi(lx0 + span) {
                if self.tiles[(tj * TILES + ti) as usize].is_some() {
                    return false;
                }
            }
        }
        true
    }

    /// How managed the ground at continent-local column `(lx, lz)` is: 0 in the wild,
    /// 1 inside a settlement's radius, and between, the rings of [`Use`]. Smooth from
    /// column to column. In the wild this is a single lookup.
    pub fn managed(&self, lx: i32, lz: i32) -> f32 {
        self.managed_at(vec2(lx as f32 + 0.5, lz as f32 + 0.5))
    }

    /// [`Self::managed`] at any continent-local point.
    pub fn managed_at(&self, p: Vec2) -> f32 {
        if self.tiles.is_empty() {
            return 0.0;
        }
        let (fx, fz) = (p.x / CELL as f32 - 0.5, p.y / CELL as f32 - 0.5);
        let (i, j) = (fx.floor() as i32, fz.floor() as i32);
        // Out in the wild all four cells are in a tile that was never made.
        if i >= 0 && j >= 0 && i < TILES * TILE - 1 && j < TILES * TILE - 1 {
            let (a, b) = (((j / TILE) * TILES + i / TILE) as usize, (((j + 1) / TILE) * TILES + (i + 1) / TILE) as usize);
            if a == b && self.tiles[a].is_none() {
                return 0.0;
            }
        }
        let (tx, tz) = (fx - i as f32, fz - j as f32);
        let v = |i: i32, j: i32| self.cell(i, j).0 as f32;
        let top = v(i, j) + (v(i + 1, j) - v(i, j)) * tx;
        let bottom = v(i, j + 1) + (v(i + 1, j + 1) - v(i, j + 1)) * tx;
        (top + (bottom - top) * tz) / 255.0
    }

    /// The settlement whose land continent-local column `(lx, lz)` is, by its index in
    /// `Network::settlements` (and [`Self::farms`]), if any.
    pub fn owner(&self, lx: i32, lz: i32) -> Option<usize> {
        let (i, j) = (lx.div_euclid(CELL), lz.div_euclid(CELL));
        let (_, o) = self.cell(i, j);
        (o != NOBODY).then_some(o as usize)
    }

    /// Whether continent-local column `(lx, lz)` is inside a settlement's radius, among
    /// its yards and houses.
    pub fn yard(&self, lx: i32, lz: i32) -> bool {
        let p = vec2(lx as f32 + 0.5, lz as f32 + 0.5);
        self.owner(lx, lz).is_some_and(|o| self.farms[o].holds(p))
    }

    /// The ring of land use at continent-local column `(lx, lz)`.
    pub fn land_use(&self, lx: i32, lz: i32) -> Use {
        if self.yard(lx, lz) { Use::Yard } else { Use::of(self.managed(lx, lz)) }
    }

    /// How much of the continent any settlement reaches, in square voxels: for tests and
    /// tools.
    pub fn reached(&self) -> usize {
        self.tiles.iter().flatten().map(|t| t.managed.iter().filter(|&&v| v > 0).count()).sum::<usize>() * (CELL * CELL) as usize
    }
}

/// The ground at a point, from the continent's grid: what decides how it can be worked.
struct Ground {
    /// Steepness of the smooth land, in voxels of rise per voxel.
    slope: f32,
    biome: Biome,
    temperature: f32,
    /// Under the sea or a lake.
    water: bool,
    /// The most water running through a grid cell about it (see `Continent::discharge`).
    river: f32,
}

/// The ground at continent-local point `p`, read off the continent's grid.
fn sample(c: &Continent, p: Vec2) -> Ground {
    let cs = c.cell_size();
    let max = (c.size - 1) as f32;
    let (fx, fz) = ((p.x / cs - 0.5).clamp(0.0, max), (p.y / cs - 0.5).clamp(0.0, max));
    let (x0, z0) = ((fx as usize).min(c.size - 2), (fz as usize).min(c.size - 2));
    let (tx, tz) = (fx - x0 as f32, fz - z0 as f32);
    let at = |x: usize, z: usize| z * c.size + x;
    let bilinear = |f: &[f32]| {
        let a = f[at(x0, z0)] + (f[at(x0 + 1, z0)] - f[at(x0, z0)]) * tx;
        let b = f[at(x0, z0 + 1)] + (f[at(x0 + 1, z0 + 1)] - f[at(x0, z0 + 1)]) * tx;
        a + (b - a) * tz
    };
    let h = |x: usize, z: usize| c.height[at(x, z)];
    let gx = (h(x0 + 1, z0) - h(x0, z0)) * (1.0 - tz) + (h(x0 + 1, z0 + 1) - h(x0, z0 + 1)) * tz;
    let gz = (h(x0, z0 + 1) - h(x0, z0)) * (1.0 - tx) + (h(x0 + 1, z0 + 1) - h(x0 + 1, z0)) * tx;
    let nearest = at((fx.round() as usize).min(c.size - 1), (fz.round() as usize).min(c.size - 1));
    let height = bilinear(&c.height);
    let water = c.ocean[nearest] || (c.lake[nearest] != NO_LAKE && c.lake[nearest] > height + 0.3);
    let mut river = 0.0f32;
    for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        let i = at(x0 + dx, z0 + dz);
        if !c.ocean[i] {
            river = river.max(c.discharge[i]);
        }
    }
    Ground {
        slope: gx.hypot(gz) * OLD_VOXEL / cs,
        biome: Biome::from_u8(c.biome[nearest]),
        temperature: bilinear(&c.temperature),
        water,
        river,
    }
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How well the ground suits working it, 0 (not at all) to 1: gentle, warm enough, not
/// bog or bare rock, and near fresh water. `wet` is whether a stream or lake is close.
fn suits(g: &Ground, wet: bool) -> f32 {
    use Biome::*;
    let ground: f32 = match g.biome {
        Ocean | Alpine | SnowyPine | DuneSea => 0.0,
        Bog | Mangrove => 0.25,
        Marsh => 0.4,
        Tundra | Volcanic => 0.45,
        Desert | RedRock => 0.3,
        Moor | Heath | Steppe | AlpineMeadow => 0.65,
        Taiga | OldGrowth | Rainforest => 0.75,
        Savanna => 0.8,
        Grassland | Deciduous | Woodland | BirchGrove => 1.0,
    };
    let gentle = 1.0 - smoothstep(0.16, 0.62, g.slope);
    let warm = smoothstep(0.12, 0.2, g.temperature);
    // Water makes the dry country green where it runs, and helps anywhere.
    let water = if wet { if g.biome.arid() || g.biome == Savanna { 0.45 } else { 0.12 } } else { 0.0 };
    ((ground + water).min(1.0) * gentle * warm).clamp(0.0, 1.0)
}

/// A cell waiting to be reached, nearest first.
#[derive(PartialEq)]
struct Step(f32, usize);

impl Eq for Step {}

impl PartialOrd for Step {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Step {
    fn cmp(&self, other: &Self) -> Ordering {
        other.0.total_cmp(&self.0).then_with(|| other.1.cmp(&self.1))
    }
}

/// Works out the settlement-influence field of continent `c`, whose trails and
/// settlements are already built. `seed` is the world's.
pub fn build(c: &Continent, seed: u32) -> Land {
    let net = &c.trails;
    if net.settlements.is_empty() {
        return Land::default();
    }
    let farms: Vec<Farm> = net.settlements.iter().map(|s| Farm::of(c, seed, s)).collect();
    // The cells the trails run through: the way to a field is easier along a road.
    let trail: HashSet<(i32, i32)> =
        net.points().filter(|p| p.half_width > 0.0).map(|p| ((p.x as i32).div_euclid(CELL), (p.z as i32).div_euclid(CELL))).collect();
    let mut tiles: Vec<Option<Box<Tile>>> = (0..TILES * TILES).map(|_| None).collect();
    let (ox, oz) = ((c.desc.mx * MACRO_SIZE) as f64, (c.desc.mz * MACRO_SIZE) as f64);
    for (index, farm) in farms.iter().enumerate().take(NOBODY as usize) {
        let far = farm.radius + farm.reach * 1.25;
        let half = (far / CELL as f32).ceil() as i32 + 1;
        let n = (2 * half + 1) as usize;
        let (ci, cj) = ((farm.at.x / CELL as f32).floor() as i32, (farm.at.y / CELL as f32).floor() as i32);
        let centre = |i: i32, j: i32| vec2((i as f32 + 0.5) * CELL as f32, (j as f32 + 0.5) * CELL as f32);
        // The cost of crossing each cell of the window, per voxel, and how well it suits
        // working; water is not crossed at all.
        let mut toll = vec![f32::INFINITY; n * n];
        let mut suit = vec![0.0f32; n * n];
        for wj in 0..n {
            for wi in 0..n {
                let (i, j) = (ci - half + wi as i32, cj - half + wj as i32);
                let p = centre(i, j);
                if p.x < 0.0 || p.y < 0.0 || p.x >= MACRO_SIZE as f32 || p.y >= MACRO_SIZE as f32 {
                    continue;
                }
                let g = sample(c, p);
                if g.water {
                    continue;
                }
                let wet = g.river >= MIN_RIVER_Q * 2.0;
                let k = wj * n + wi;
                suit[k] = suits(&g, wet);
                // Climbing is slow work, and a big river is in the way.
                let mut t = 1.0 + 10.0 * g.slope * g.slope;
                if g.river >= MIN_RIVER_Q * 60.0 {
                    t *= 2.5;
                }
                if trail.contains(&(i, j)) {
                    t *= 0.55;
                }
                toll[k] = t;
            }
        }
        // The way from the middle to every cell, over the ground.
        let mut cost = vec![f32::INFINITY; n * n];
        let start = half as usize * n + half as usize;
        cost[start] = 0.0;
        let mut queue = BinaryHeap::new();
        queue.push(Step(0.0, start));
        while let Some(Step(d, k)) = queue.pop() {
            if d > cost[k] || d > far {
                continue;
            }
            let (wi, wj) = ((k % n) as i32, (k / n) as i32);
            for (di, dj) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
                let (a, b) = (wi + di, wj + dj);
                if a < 0 || b < 0 || a >= n as i32 || b >= n as i32 {
                    continue;
                }
                let next = b as usize * n + a as usize;
                let len = if di != 0 && dj != 0 { std::f32::consts::SQRT_2 } else { 1.0 };
                let step = len * CELL as f32 * 0.5 * (toll[k].min(1e6) + toll[next]);
                if step.is_finite() && d + step < cost[next] {
                    cost[next] = d + step;
                    queue.push(Step(d + step, next));
                }
            }
        }
        for wj in 0..n {
            for wi in 0..n {
                let k = wj * n + wi;
                let d = cost[k];
                if !d.is_finite() || d > far {
                    continue;
                }
                let (i, j) = (ci - half + wi as i32, cj - half + wj as i32);
                let p = centre(i, j);
                // A road carries the work further, but never past the farthest anyone
                // walks out to a field.
                if farm.at.distance(p) > far - CELL as f32 {
                    continue;
                }
                let inside = farm.at.distance(p) < farm.radius;
                let managed = if inside {
                    1.0
                } else {
                    // The rings fray: a slow noise moves each a little in or out.
                    let fray = fbm2(seed ^ 0x1a2d, (ox + p.x as f64) / m64(90.0), (oz + p.y as f64) / m64(90.0), 2, 2.0, 0.5) * 0.09;
                    let near = 1.0 - (d - farm.radius).max(0.0) / farm.reach + fray;
                    // Poor ground is worked less hard than its distance alone would have it.
                    (near * (0.25 + 0.75 * suit[k])).clamp(0.0, 0.99)
                };
                let v = (managed * 255.0).round() as u8;
                if v == 0 {
                    continue;
                }
                let t = tiles[((j / TILE) * TILES + i / TILE) as usize].get_or_insert_with(|| {
                    Box::new(Tile { managed: [0; (TILE * TILE) as usize], owner: [NOBODY; (TILE * TILE) as usize] })
                });
                let kk = ((j % TILE) * TILE + i % TILE) as usize;
                if v > t.managed[kk] {
                    t.managed[kk] = v;
                    t.owner[kk] = index as u8;
                }
            }
        }
    }
    Land { tiles, farms, steads: Vec::new(), quarries: Vec::new(), fixtures: Default::default() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{continent::SimConfig, world::GameWorld};

    #[test]
    fn the_rings_fall_away_to_the_wild_and_the_yards_are_whole() {
        let w = GameWorld::with_config(3, SimConfig { size: 256 });
        let c = w.continent(0, 0);
        let land = &c.land;
        assert!(!land.farms.is_empty(), "the test continent has no settlements");
        assert!(land.reached() > 0);
        for f in &land.farms {
            // The middle is a yard, wholly managed.
            let (x, z) = (f.at.x as i32, f.at.y as i32);
            assert_eq!(land.land_use(x, z), Use::Yard);
            assert!(land.managed(x, z) > 0.95);
            // Well beyond the farthest it reaches, nothing at all, in every direction.
            for a in 0..16 {
                let dir = Vec2::from_angle(a as f32 / 16.0 * std::f32::consts::TAU);
                let p = f.at + dir * (f.radius + f.reach * 1.25 + CELL as f32 * 3.0);
                let near_another = land.farms.iter().any(|g| g.at.distance(p) < g.radius + g.reach * 1.25 + CELL as f32 * 3.0);
                if !near_another {
                    assert_eq!(land.managed(p.x as i32, p.y as i32), 0.0, "still managed at {p} beyond {:?}", f.at);
                    assert_eq!(land.land_use(p.x as i32, p.y as i32), Use::Wild);
                }
            }
        }
        // Far from every settlement the country is wild, and the whole square of it says so.
        let lonely = (0..64).flat_map(|i| (0..64).map(move |j| (i * 1024 + 512, j * 1024 + 512))).find(|&(x, z)| {
            land.farms.iter().all(|f| f.at.distance(vec2(x as f32, z as f32)) > f.radius + f.reach * 1.25 + 2048.0)
        });
        let (x, z) = lonely.expect("somewhere far from every settlement");
        assert_eq!(land.managed(x, z), 0.0);
        assert!(land.wild_square(x - 512, z - 512, 1024));
    }

    #[test]
    fn the_field_is_the_same_each_time_it_is_built() {
        let a = GameWorld::with_config(5, SimConfig { size: 256 }).continent(0, 0);
        let b = GameWorld::with_config(5, SimConfig { size: 256 }).continent(0, 0);
        assert_eq!(a.land.reached(), b.land.reached());
        for f in &a.land.farms {
            for k in 0..200 {
                let p = f.at + Vec2::from_angle(k as f32 * 0.37) * (k as f32 * 4.0);
                let (x, z) = (p.x as i32, p.y as i32);
                assert_eq!(a.land.managed(x, z), b.land.managed(x, z));
                assert_eq!(a.land.owner(x, z), b.land.owner(x, z));
            }
        }
    }
}
