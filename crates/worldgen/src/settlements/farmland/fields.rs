//! The worked land about a settlement, cut into fields, each with its use (#64).
//!
//! Nothing lays the fields out whole. Which field a column is in is worked out from where
//! the column is, so every chunk that asks finds the same field there, and neighbouring
//! chunks agree at their borders without either seeing the other's fields. A field is
//! the ground nearest one of a set of seeded points, measured in a plane bent by a slow
//! noise, with height counted as distance too: so a field's edges bend round the hills
//! along the contours instead of running straight up them, and nothing about them shows a
//! grid. Where two columns side by side are in different fields, a boundary stands
//! between them (see [`super::bounds`]).
//!
//! An old settlement's fields are small, and its arable cut into strips along the
//! contour; a young one's are big ragged clearings run together from several, as are the
//! fields out in the pasture and the rough of any settlement.

use glam::{Vec2, vec2};

use super::influence::{CROPS, EDGE, Farm, Land, PASTURE, ROUGH};
use crate::{
    biome::Biome,
    continent::Continent,
    macro_layout::MACRO_SIZE,
    noise::{fbm2, hash3, perlin2, unit},
    scale::{OLD_VOXEL, m, m64},
    settlements::{Age, Region},
    surface::{Column, Cover},
};

/// What a field is used for.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Crop {
    /// Wheat, golden at harvest.
    Wheat,
    /// Barley, paler, and the grain of drier and cooler ground.
    Barley,
    /// Rye and oats: the hardy grain of the cold, tall and grey.
    Rye,
    /// Turnips and beet: rows of dark leaves.
    Roots,
    /// Flax, blue in flower, grown for linen.
    Flax,
    /// Vines in rows on posts, in warm country.
    Vines,
    /// Olive trees in rows, in warm country.
    Olives,
    /// Fruit trees in rows: apples and pears, white with blossom in spring.
    Orchard,
    /// Fallow, freshly ploughed.
    Ploughed,
    /// Fallow gone to wildflowers.
    Fallow,
    /// Hay meadow: long grass and flowers, cut for the winter.
    Hay,
    /// Pasture, grazed short.
    Pasture,
    /// Rough grazing: the common, gorse and the odd tree.
    Rough,
    /// A managed wood: coppiced stools and standards.
    Coppice,
    /// Rice standing in water: level paddies held by low earth bunds, on the flat wet
    /// floor of a warm valley (see [`super::paddy`]). Last, so the kinds before it keep
    /// their numbers.
    Paddy,
}

impl Crop {
    pub const ALL: [Crop; 15] = [
        Crop::Wheat,
        Crop::Barley,
        Crop::Rye,
        Crop::Roots,
        Crop::Flax,
        Crop::Vines,
        Crop::Olives,
        Crop::Orchard,
        Crop::Ploughed,
        Crop::Fallow,
        Crop::Hay,
        Crop::Pasture,
        Crop::Rough,
        Crop::Coppice,
        Crop::Paddy,
    ];

    /// Whether its ground is tilled: soil shows between its rows, and it is ploughed in
    /// winter.
    pub fn tilled(self) -> bool {
        matches!(self, Crop::Wheat | Crop::Barley | Crop::Rye | Crop::Roots | Crop::Flax | Crop::Vines | Crop::Ploughed)
    }

    /// Whether it is a grain or another crop sown in rows each year.
    pub fn arable(self) -> bool {
        matches!(self, Crop::Wheat | Crop::Barley | Crop::Rye | Crop::Roots | Crop::Flax)
    }

    /// Whether trees stand in it in rows (see [`super::orchard`]).
    pub fn trees(self) -> bool {
        matches!(self, Crop::Orchard | Crop::Olives)
    }

    pub fn word(self) -> &'static str {
        match self {
            Crop::Wheat => "wheat",
            Crop::Barley => "barley",
            Crop::Rye => "rye",
            Crop::Roots => "roots",
            Crop::Flax => "flax",
            Crop::Vines => "vines",
            Crop::Olives => "olives",
            Crop::Orchard => "orchard",
            Crop::Ploughed => "ploughed fallow",
            Crop::Fallow => "fallow",
            Crop::Hay => "hay meadow",
            Crop::Pasture => "pasture",
            Crop::Rough => "rough grazing",
            Crop::Coppice => "coppice",
            Crop::Paddy => "paddy",
        }
    }
}

/// The field a column is in.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Plot {
    /// The field's own number: the same on every column of it, and different from every
    /// field about it.
    pub id: u32,
    pub crop: Crop,
    /// Which way its rows run, along the contour: a unit vector across the ground.
    pub rows: Vec2,
    /// How managed it is at its middle (see [`super::influence`]): which ring it is in.
    pub managed: f32,
    /// The settlement whose field it is, by its index in `Network::settlements` and
    /// [`Land::farms`].
    pub owner: usize,
    /// What holds its terraces up, if it is terraced (see [`super::terrace`]).
    pub terrace: Option<super::terrace::Bank>,
}

/// Spacing of the seeded points the fields are cut about, in voxels: a field of an
/// established village is some 26 m across before its rows are cut into strips.
const LATTICE: f32 = 64.0;
/// How far a voxel of height counts across the ground, so fields lie along the contours.
const CLIMB: f32 = 3.0;

/// The ground's height at continent-local `p`, smoothly from the continent's grid, in
/// voxels: the lie of the land a field follows, without the voxels' small relief.
fn height(c: &Continent, p: Vec2) -> f32 {
    let cs = c.cell_size();
    let max = (c.size - 1) as f32;
    let (fx, fz) = ((p.x / cs - 0.5).clamp(0.0, max), (p.y / cs - 0.5).clamp(0.0, max));
    let (x0, z0) = ((fx as usize).min(c.size - 2), (fz as usize).min(c.size - 2));
    let (tx, tz) = (fx - x0 as f32, fz - z0 as f32);
    let h = |x: usize, z: usize| c.height[z * c.size + x];
    let a = h(x0, z0) + (h(x0 + 1, z0) - h(x0, z0)) * tx;
    let b = h(x0, z0 + 1) + (h(x0 + 1, z0 + 1) - h(x0, z0 + 1)) * tx;
    (a + (b - a) * tz) * OLD_VOXEL
}

/// Which way the ground falls at `p`, as a unit vector (downhill), and how steeply, in
/// voxels of rise per voxel: measured across a few metres, so it turns smoothly.
pub(super) fn fall(c: &Continent, p: Vec2) -> (Vec2, f32) {
    let d = m(10.0);
    let g = vec2(height(c, p + vec2(d, 0.0)) - height(c, p - vec2(d, 0.0)), height(c, p + vec2(0.0, d)) - height(c, p - vec2(0.0, d))) / (2.0 * d);
    let slope = g.length();
    (if slope > 1e-4 { -g / slope } else { Vec2::X }, slope)
}

/// The seeded point of lattice cell `(i, j)`, in the bent plane.
fn site(key: u32, i: i32, j: i32) -> Vec2 {
    let h = hash3(key, i, j, 0x51e);
    let (u, v) = (unit(h), unit(h.rotate_left(16).wrapping_mul(0x2c1b_3c6d)));
    vec2((i as f32 + 0.15 + 0.7 * u) * LATTICE, (j as f32 + 0.15 + 0.7 * v) * LATTICE)
}

/// A roll of the dice for lattice cell `(i, j)`, with `salt` for each question asked of it.
pub(super) fn roll(key: u32, i: i32, j: i32, salt: i32) -> f32 {
    unit(hash3(key ^ 0xf1e1d, i, j, salt))
}

/// How likely a field of `farm` whose middle is managed `managed` is to run together with
/// its neighbour: a young place's clearings are big and ragged, an old one's fields small,
/// and out in the pasture and the rough the fields are bigger whatever the age.
fn merge_chance(farm: &Farm, managed: f32) -> f32 {
    let age: f32 = match farm.age {
        Age::Young => 0.62,
        Age::Established => 0.3,
        Age::Ancient => 0.12,
    };
    let out: f32 = if managed < ROUGH {
        1.0
    } else if managed < PASTURE {
        0.45
    } else if managed < CROPS {
        0.2
    } else {
        0.0
    };
    (age + out).min(1.0)
}

/// The field lattice cell `(i, j)` belongs to once fields are run together: its own, or
/// a neighbour's. Decided by the cell that is kept, so every column agrees.
fn merged(land: &Land, key: u32, i: i32, j: i32) -> (i32, i32) {
    let chance = |i: i32, j: i32| {
        let s = site(key, i, j);
        let managed = land.managed_at(s);
        let Some(owner) = land.owner(s.x as i32, s.y as i32) else { return 1.0 };
        merge_chance(&land.farms[owner], managed)
    };
    let mut ci = i;
    let even = i & !1;
    if even != i && roll(key, even, j, 1) < chance(even, j) {
        ci = even;
    }
    let mut cj = j;
    let even = j & !1;
    if even != j && roll(key, ci, even, 2) < chance(ci, even) {
        cj = even;
    }
    (ci, cj)
}

/// The use of a field whose middle is at continent-local `at`, managed `managed`, of
/// `farm`: from its ring, the climate and the region, and a roll of the dice `a`.
pub(super) fn crop_for(c: &Continent, farm: &Farm, at: Vec2, managed: f32, a: f32) -> Crop {
    use Crop::*;
    let cs = c.cell_size();
    let max = c.size as i32 - 1;
    let cell = |v: f32| ((v / cs) as i32).clamp(0, max) as usize;
    let i = cell(at.y) * c.size + cell(at.x);
    let temperature = c.temperature[i];
    let biome = Biome::from_u8(c.biome[i]);
    // Is there water to hand: a stream or a lake shore close by.
    let wet = (-1i32..=1).any(|dz| {
        (-1i32..=1).any(|dx| {
            let (x, z) = ((cell(at.x) as i32 + dx).clamp(0, max) as usize, (cell(at.y) as i32 + dz).clamp(0, max) as usize);
            let k = z * c.size + x;
            !c.ocean[k] && (c.discharge[k] >= crate::surface::MIN_RIVER_Q * 2.0 || c.lake[k] != crate::continent::NO_LAKE)
        })
    });
    let warm = temperature > 0.6;
    let cold = temperature < 0.34 || matches!(farm.region, Region::Northern | Region::Alpine);
    let dry = farm.region == Region::Desert || biome.arid() || matches!(biome, Biome::Savanna | Biome::Steppe);
    let upland = farm.region == Region::Upland;
    let wooded = crate::features::canopy_tree(biome).is_some() && !matches!(biome, Biome::Steppe | Biome::Moor | Biome::Heath | Biome::Savanna | Biome::Desert);
    // The flat wet floor of a warm valley is levelled into paddies before anything else
    // is grown on it; what is left of the dice chooses among the rest.
    let mut a = a;
    if managed >= CROPS && super::paddy::valley(c, farm, at, biome, wet) {
        if a < super::paddy::SHARE {
            return Paddy;
        }
        a = (a - super::paddy::SHARE) / (1.0 - super::paddy::SHARE);
    }
    let list: &[(Crop, f32)] = if managed >= CROPS {
        if dry && !wet {
            &[(Barley, 3.0), (Wheat, 1.5), (Olives, 2.0), (Ploughed, 2.5), (Fallow, 0.6), (Pasture, 1.0)]
        } else if dry {
            &[(Barley, 2.5), (Wheat, 2.5), (Olives, 2.0), (Roots, 1.2), (Vines, 1.0), (Ploughed, 1.5), (Fallow, 0.5)]
        } else if warm {
            &[(Wheat, 3.0), (Barley, 1.5), (Vines, 2.5), (Olives, 2.0), (Orchard, 0.8), (Ploughed, 1.0), (Fallow, 1.0)]
        } else if cold {
            &[(Rye, 4.0), (Barley, 2.0), (Roots, 2.5), (Ploughed, 1.0), (Hay, 1.6), (Fallow, 0.8)]
        } else if upland {
            &[(Barley, 2.5), (Rye, 1.5), (Roots, 1.5), (Hay, 2.0), (Pasture, 1.2), (Ploughed, 1.0)]
        } else {
            &[(Wheat, 4.0), (Barley, 2.5), (Roots, 2.0), (Flax, 1.0), (Orchard, 1.4), (Ploughed, 1.2), (Fallow, 1.2), (Hay, 0.6)]
        }
    } else if managed >= PASTURE {
        if warm || dry {
            &[(Pasture, 5.0), (Hay, 1.5), (Olives, 1.0), (Fallow, 0.6)]
        } else {
            &[(Pasture, 5.0), (Hay, 3.5), (Orchard, 0.4), (Fallow, 0.6)]
        }
    } else if wooded {
        &[(Rough, 5.0), (Coppice, 4.0)]
    } else {
        &[(Rough, 6.0), (Coppice, 0.8)]
    };
    let total: f32 = list.iter().map(|w| w.1).sum();
    let mut r = a * total;
    for &(crop, w) in list {
        if r < w {
            return crop;
        }
        r -= w;
    }
    list[list.len() - 1].0
}

/// How likely an arable field of `farm` is to be cut into strips along the contour.
fn strip_chance(farm: &Farm) -> f32 {
    match farm.age {
        Age::Young => 0.05,
        Age::Established => 0.3,
        Age::Ancient => 0.7,
    }
}

/// Whether a column's ground can be worked at all: dry land, not a trail or a landmark's
/// pad, not too cold, and not bog. Cliffs, scree, beaches and water are left wild.
pub fn worked(col: &Column) -> bool {
    matches!(col.cover, Cover::Land | Cover::Bank)
        && col.water_surface().is_none()
        && col.clear >= m(0.6)
        && col.temperature >= 0.12
        && !matches!(col.biome, Biome::Bog | Biome::Mangrove | Biome::DuneSea | Biome::Alpine | Biome::SnowyPine)
}

/// The field continent-local column `(lx, lz)` of continent `c` is in, if any: none in the
/// wild, inside a settlement's radius (the layout's), or past the last of its fields.
/// It says nothing of the column itself: whether its ground can be worked is
/// [`worked`]'s to say. `seed` is the world's. In the wild this is one lookup.
pub fn plot(c: &Continent, seed: u32, lx: i32, lz: i32) -> Option<Plot> {
    locate(c, seed, lx, lz).map(|f| f.plot)
}

/// Where a column lies among the fields: its field, and what the paddies, the ditches and
/// the names need to know of how the field was cut.
#[derive(Clone, Copy, Debug)]
pub struct Found {
    pub plot: Plot,
    /// The field's own number before it was cut into strips: the same on every strip of
    /// it, and different from every other field's.
    pub field: u32,
    /// The column's middle, continent-local; where it lies in the plane the fields are
    /// cut in; and the field's middle there.
    pub p: Vec2,
    pub q: Vec2,
    pub site: Vec2,
    /// Roughly how far in from the nearest edge of its field the column lies, in voxels:
    /// small along every edge, and along each seam where two fields were run together.
    pub edge: f32,
}

/// How far the plane the fields are cut in is bent at world column `(wx, wz)`: a slow
/// noise and a quicker one, so no edge runs straight for long. `seed` is the world's.
pub fn bend(seed: u32, wx: f64, wz: f64) -> Vec2 {
    vec2(
        fbm2(seed ^ 0xf1e2, wx / m64(70.0), wz / m64(70.0), 2, 2.0, 0.5),
        fbm2(seed ^ 0xf1e3, wx / m64(70.0), wz / m64(70.0), 2, 2.0, 0.5),
    ) * m(11.0)
        + vec2(perlin2(seed ^ 0xf1e4, wx / m64(13.0), wz / m64(13.0)), perlin2(seed ^ 0xf1e5, wx / m64(13.0), wz / m64(13.0))) * m(2.2)
}

/// [`plot`], with how the field was cut (see [`Found`]).
pub fn locate(c: &Continent, seed: u32, lx: i32, lz: i32) -> Option<Found> {
    let land = &c.land;
    let managed = land.managed(lx, lz);
    if managed < EDGE {
        return None;
    }
    let p = vec2(lx as f32 + 0.5, lz as f32 + 0.5);
    let owner = land.owner(lx, lz)?;
    if land.farms[owner].holds(p) {
        return None;
    }
    // Between the ditches of drained land the fields are the ditches' own.
    if let Some(found) = super::ditch::field(c, seed, p, owner) {
        return Some(found);
    }
    let key = c.desc.seed ^ 0x6f1e_1d5;
    let q = p + bend(seed, (c.desc.mx * MACRO_SIZE + lx) as f64, (c.desc.mz * MACRO_SIZE + lz) as f64);
    let here = height(c, p);
    let (gi, gj) = ((q.x / LATTICE).floor() as i32, (q.y / LATTICE).floor() as i32);
    let mut best = (f32::MAX, 0, 0);
    // How far off the next nearest seeded point is, for how near the column is to an edge.
    let mut second = f32::MAX;
    for dj in -1..=1 {
        for di in -1..=1 {
            let (i, j) = (gi + di, gj + dj);
            let s = site(key, i, j);
            let rise = (height(c, s) - here) * CLIMB;
            let d = s.distance_squared(q) + rise * rise;
            if d < best.0 {
                second = best.0;
                best = (d, i, j);
            } else if d < second {
                second = d;
            }
        }
    }
    let (i, j) = merged(land, key, best.1, best.2);
    let s = site(key, i, j);
    let at_site = land.managed_at(s);
    // The field takes its use, its owner and its ring from its middle; one whose middle
    // lies inside a yard or out in the wild is the column's own.
    let (managed, farm_index) = match land.owner(s.x as i32, s.y as i32) {
        Some(o) if at_site >= EDGE && !land.farms[o].holds(s) => (at_site, o),
        _ => (managed, owner),
    };
    let farm = &land.farms[farm_index];
    let mut crop = crop_for(c, farm, s, managed, roll(key, i, j, 3));
    let (down, slope) = fall(c, s);
    let along = if slope > 0.03 {
        vec2(-down.y, down.x)
    } else {
        Vec2::from_angle(roll(key, i, j, 5) * std::f32::consts::PI)
    };
    let mut id = hash3(key, i, j, 0x1d);
    let field = id | 1;
    // An old place's arable lies in strips along the contour, each of its own crop.
    if crop.arable() && roll(key, i, j, 6) < strip_chance(farm) {
        let across = vec2(-along.y, along.x);
        let width = m(4.0) + m(3.5) * roll(key, i, j, 7);
        let k = (((q - s).dot(across)) / width + roll(key, i, j, 8)).floor() as i32;
        id = hash3(id, k, 0, 0x57);
        crop = match unit(hash3(id, 1, 2, 3)) {
            r if r < 0.12 => Crop::Ploughed,
            r if r < 0.3 && crop != Crop::Rye => Crop::Roots,
            r if r < 0.45 && crop == Crop::Wheat => Crop::Barley,
            _ => crop,
        };
    }
    let terrace = super::terrace::of(farm, crop, managed, slope);
    let edge = (second.sqrt() - best.0.sqrt()) * 0.5;
    Some(Found { plot: Plot { id: id | 1, crop, rows: along, managed, owner: farm_index, terrace }, field, p, q, site: s, edge })
}

/// The fields of a square of columns, worked out together for a chunk: the field each
/// column is in, if its ground is worked.
pub struct Patch {
    /// Its corner, in continent-local columns, and its width.
    pub x0: i32,
    pub z0: i32,
    pub n: usize,
    /// Row by row in x, as the chunk's columns are kept: `x * n + z`. Empty where no
    /// settlement reaches the square.
    plots: Vec<Option<Plot>>,
    /// How each column lies by a lane hedged through the fields, as `plots`; empty where
    /// none runs (see [`super::lanes`]).
    lanes: Vec<Option<super::lanes::Lane>>,
}

impl Patch {
    /// The fields of the `n`-wide square of continent-local columns from `(x0, z0)`,
    /// whose ground is `columns` (`x * n + z`).
    pub fn new(c: &Continent, seed: u32, x0: i32, z0: i32, n: usize, columns: &[Column]) -> Patch {
        let mut patch = Patch { x0, z0, n, plots: Vec::new(), lanes: Vec::new() };
        if c.land.wild_square(x0, z0, n as i32) {
            return patch;
        }
        patch.plots = (0..n * n)
            .map(|i| {
                let col = &columns[i];
                // A paddy's water is worked ground; a ditch's is not.
                let flooded = super::paddy::flooded(col);
                if !worked(col) && !flooded {
                    return None;
                }
                plot(c, seed, x0 + (i / n) as i32, z0 + (i % n) as i32).filter(|p| !flooded || p.crop == Crop::Paddy)
            })
            .collect();
        // The lanes hedged through the fields, where any field here is of a place that
        // hedges them and a lane runs by; only the columns near a way are looked at.
        let hedged = |p: &Plot| super::lanes::hedged(&c.land.farms[p.owner], p.managed);
        if patch.plots.iter().flatten().any(hedged)
            && let Some(lanes) = super::lanes::Lanes::near(c, x0, z0, n)
        {
            patch.lanes = (0..n * n)
                .map(|i| {
                    patch.plots[i].as_ref().filter(|p| hedged(p))?;
                    if columns[i].clear >= super::lanes::LOOK {
                        return None;
                    }
                    lanes.at(seed, x0 + (i / n) as i32, z0 + (i % n) as i32)
                })
                .collect();
            // Nothing is sown on a verge.
            for (plot, lane) in patch.plots.iter_mut().zip(&patch.lanes) {
                if *lane == Some(super::lanes::Lane::Verge) {
                    *plot = None;
                }
            }
        }
        patch
    }

    /// How the column `(i, j)` of the square lies by a lane hedged through the fields, if
    /// it does.
    pub fn lane(&self, i: usize, j: usize) -> Option<super::lanes::Lane> {
        if self.lanes.is_empty() || i >= self.n || j >= self.n {
            return None;
        }
        self.lanes[i * self.n + j]
    }

    /// Whether no field reaches the square at all.
    pub fn is_empty(&self) -> bool {
        self.plots.is_empty()
    }

    /// The field of the column `(i, j)` of the square, if it is in one.
    pub fn at(&self, i: usize, j: usize) -> Option<&Plot> {
        if self.plots.is_empty() || i >= self.n || j >= self.n {
            return None;
        }
        self.plots[i * self.n + j].as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{continent::SimConfig, world::GameWorld};

    #[test]
    fn fields_are_the_same_wherever_they_are_asked_about() {
        let w = GameWorld::with_config(3, SimConfig { size: 256 });
        let c = w.continent(0, 0);
        let again = GameWorld::with_config(3, SimConfig { size: 256 }).continent(0, 0);
        let mut seen = std::collections::HashSet::new();
        let mut worked = 0;
        for f in &c.land.farms {
            for k in 0..400 {
                let p = f.at + Vec2::from_angle(k as f32 * 0.61) * (f.radius + k as f32 * 2.0);
                let (x, z) = (p.x as i32, p.y as i32);
                let a = plot(&c, w.seed(), x, z);
                assert_eq!(a, plot(&again, w.seed(), x, z));
                if let Some(a) = a {
                    worked += 1;
                    seen.insert(a.crop);
                    assert!(a.managed >= EDGE && a.managed < 1.0);
                    assert!(!c.land.yard(x, z), "a field inside a yard at {x},{z}");
                }
            }
        }
        assert!(worked > 100, "only {worked} columns in fields");
        assert!(seen.len() >= 4, "only {seen:?} grown");
    }

    #[test]
    fn neighbouring_columns_mostly_share_a_field() {
        // Fields are tens of metres across: two columns side by side are in the same one
        // far more often than not, and boundaries are lines, not speckle.
        let w = GameWorld::with_config(3, SimConfig { size: 256 });
        let c = w.continent(0, 0);
        let (mut same, mut all) = (0, 0);
        for f in &c.land.farms {
            for k in 0..300 {
                let p = f.at + Vec2::from_angle(k as f32 * 1.3) * (f.radius + 20.0 + k as f32);
                let (x, z) = (p.x as i32, p.y as i32);
                if let (Some(a), Some(b)) = (plot(&c, w.seed(), x, z), plot(&c, w.seed(), x + 1, z)) {
                    all += 1;
                    same += (a.id == b.id) as u32;
                }
            }
        }
        assert!(all > 100);
        assert!(same as f32 > all as f32 * 0.85, "{same} of {all} side by side share a field");
    }
}
