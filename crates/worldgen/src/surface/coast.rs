//! The coast: estuary sand bars and deltas where rivers meet the sea, and the kind of
//! shore everywhere else.
//!
//! What a shore is like is decided by three things. Exposure: how hard the waves hit,
//! from whether the coast faces into the prevailing wind with open water upwind, and
//! whether it stands out on a headland. Slope: steep ground meets the sea as cliffs,
//! gentle ground as beaches and flats. Climate and biome: cold coasts are stonier, hot
//! dry ones sandier, and swampy ones muddy. Exposed coasts get shingle and boulders,
//! sheltered bays long sandy strands, wide beaches dunes behind them, estuaries mud and
//! reed flats, and high steep ground sea cliffs (see `sea_cliff`).

use glam::{Vec2, vec2};

use crate::{
    biome::Biome,
    continent::Continent,
    noise::{hash2, perlin2, unit},
    scale::{OLD_VOXEL, m, m64},
};

use super::{Cover, MIN_RIVER_Q, funnel, node, sea_cliff, smoothstep};

/// Smallest discharge that builds a delta.
const DELTA_Q: f32 = 1500.0;

/// A small delta: a fan of low mud flats built out into shallow sea in front of a big,
/// slow river, split by a few distributary channels.
pub struct Delta {
    /// Where the river reaches the coast, and the way it flows out to sea.
    pos: Vec2,
    dir: Vec2,
    /// Width of the river at its mouth, in voxels.
    width: f32,
    /// How far the fan reaches out to sea at most.
    pub radius: f32,
    salt: u32,
}

impl Delta {
    /// The delta at the mouth of the river leaving node `i`, if it builds one: only big
    /// rivers whose last reach is already down at sea level, so slow and silty.
    pub fn at(c: &Continent, seed: u32, i: usize) -> Option<Delta> {
        let r = c.rcv[i] as usize;
        let q = c.discharge[i];
        if c.ocean[i] || !c.ocean[r] || q < DELTA_Q || c.water_level[i] > 3.0 {
            return None;
        }
        let (a, b) = (node(c, i), node(c, r));
        let width = Continent::river_width(q) * OLD_VOXEL * funnel(c, r, q);
        Some(Delta {
            pos: (a + b) * 0.5,
            dir: (b - a).try_normalize()?,
            width,
            radius: (width * 5.0).clamp(m(40.0), m(110.0)),
            salt: hash2(seed ^ 0xde17, i as i32, 1),
        })
    }

    /// Builds the delta into a sea-side column at `p` whose ground is `ground`. Returns
    /// the cover of new flats, or `None` where the delta leaves the column alone or cuts
    /// a channel through it (the sea then fills the channel).
    pub fn shape(&self, p: Vec2, wx: f64, wz: f64, ground: &mut f32) -> Option<Cover> {
        let v = p - self.pos;
        let d = v.length();
        let along = v.dot(self.dir);
        if d > self.radius * 1.2 || along < 0.0 || *ground < -m(5.0) {
            return None;
        }
        let side = self.dir.perp_dot(v);
        let angle = side.atan2(along);
        // The fan's edge, lobed.
        let lobes = perlin2(self.salt, angle as f64 * 2.5, 0.5) * 0.3;
        let reach = self.radius * (1.0 - (angle / 1.3).powi(2)).max(0.0) * (0.85 + lobes);
        if d > reach {
            return None;
        }
        let t = d / reach.max(1.0);
        // Distributaries: the main channel straight out and two that branch off it,
        // each wandering and narrowing towards the sea.
        for (k, a0) in [0.0f32, -0.6, 0.55].into_iter().enumerate() {
            let wander = perlin2(self.salt ^ (k as u32 + 1), d as f64 / m64(35.0), k as f64 * 3.1) * 0.35;
            let lateral = (angle - a0 - wander).abs() * d;
            let half = self.width * if k == 0 { 0.4 } else { 0.25 } * (1.0 - 0.4 * t);
            // Branches split off a little way out.
            let started = k == 0 || d > self.width * 1.5;
            if started && lateral < half {
                let depth = m(1.0) + m(0.8) * (1.0 - lateral / half);
                *ground = ground.min(-depth);
                return None;
            }
        }
        // Flats a little above the sea, fading into it at the fan's edge.
        let flat = m(0.3) + m(0.6) * (1.0 - t) + perlin2(self.salt ^ 0x77, wx / m64(15.0), wz / m64(15.0)) * m(0.3);
        let edge = smoothstep(0.82, 1.0, t);
        *ground = ground.max(flat + (*ground - flat) * edge);
        Some(if edge > 0.3 { Cover::Beach } else { Cover::Bank })
    }
}

/// Raises sand bars on the bed of a tidal river channel. `bed` is the channel bed and
/// `across` runs from 0 mid-stream to 1 at the channel edge. Some bars break the surface
/// as low sand islands.
pub fn sand_bar(seed: u32, wx: f64, wz: f64, across: f32, bed: f32) -> f32 {
    if across > 0.85 {
        return bed;
    }
    let b = perlin2(seed ^ 0xba55, wx / m64(16.0), wz / m64(16.0));
    let lift = smoothstep(0.15, 0.45, b) * (1.0 - smoothstep(0.6, 0.85, across));
    bed + (m(0.5) - bed) * lift
}

/// Farthest from a river mouth that its estuary flats reach, in voxels.
const MOUTH_REACH: f32 = m(120.0);

/// Where a river reaches the sea: near it the shore is estuary mud and reeds rather than
/// beach or cliff.
pub struct Mouth {
    pos: Vec2,
    /// Fully estuary within `inner` of the mouth, fading out by `outer`.
    inner: f32,
    outer: f32,
    /// Brooks make no estuary; big rivers make a full one.
    strength: f32,
}

impl Mouth {
    fn at(c: &Continent, i: usize) -> Option<Mouth> {
        let r = c.rcv[i] as usize;
        let q = c.discharge[i];
        if c.ocean[i] || !c.ocean[r] || q < MIN_RIVER_Q * 4.0 {
            return None;
        }
        let width = Continent::river_width(q) * OLD_VOXEL * funnel(c, r, q);
        let inner = (width * 2.5).clamp(m(20.0), MOUTH_REACH / 2.2);
        Some(Mouth {
            pos: (node(c, i) + node(c, r)) * 0.5,
            inner,
            outer: inner * 2.2,
            strength: smoothstep(MIN_RIVER_Q * 4.0, 600.0, q),
        })
    }

    fn weight(&self, p: Vec2) -> f32 {
        (1.0 - smoothstep(self.inner, self.outer, (p - self.pos).length())) * self.strength
    }
}

/// The river mouths whose estuaries can reach the area from `lo` to `hi` (continent-local
/// voxels). Gathered from far enough out that every column sees the same mouths whatever
/// area it is sampled with.
pub fn mouths(c: &Continent, lo: Vec2, hi: Vec2) -> Vec<Mouth> {
    let cs = c.cell_size();
    // A river node can sit several cells from its own cell (see `node`).
    let pad = MOUTH_REACH + cs * (10.0 * (c.size as f32 / 1024.0)).max(8.0);
    let cell = |v: f32| ((v / cs).floor() as i32).clamp(1, c.size as i32 - 2) as usize;
    let mut out = Vec::new();
    for z in cell(lo.y - pad)..=cell(hi.y + pad) {
        for x in cell(lo.x - pad)..=cell(hi.x + pad) {
            if let Some(mouth) = Mouth::at(c, z * c.size + x) {
                let near = mouth.pos.clamp(lo, hi);
                if (mouth.pos - near).length() < mouth.outer {
                    out.push(mouth);
                }
            }
        }
    }
    out
}

/// A cell's shore, from the coarse grid alone.
#[derive(Clone, Copy, Default)]
struct ShoreCell {
    /// How hard waves hit, 0 (sheltered) to 1 (a windward headland).
    exposure: f32,
    /// How squarely the coast faces into the prevailing wind across open water, 0 to 1.
    onshore: f32,
    /// How enclosed by land the water is, 0 (open coast) to 1 (deep in a bay).
    shelter: f32,
    /// The highest ground in the cell and its neighbours, in voxels.
    hinter: f32,
}

impl ShoreCell {
    fn compute(c: &Continent, x: i32, z: i32) -> Self {
        let n = c.size as i32;
        let cs = c.cell_size();
        let height = |x: i32, z: i32| c.height[(z.clamp(0, n - 1) * n + x.clamp(0, n - 1)) as usize];
        // Off the grid is the deep sea between continents.
        let sea = |q: Vec2| {
            let (x, z) = (q.x.floor() as i32, q.y.floor() as i32);
            x < 0 || z < 0 || x >= n || z >= n || c.ocean[(z * n + x) as usize]
        };
        // Which way the ground falls towards the sea, over a few cells so it turns smoothly.
        let fall = vec2(height(x - 2, z) - height(x + 2, z), height(x, z - 2) - height(x, z + 2));
        let seaward = fall.normalize_or_zero();
        let wind = vec2(c.desc.wind.cos(), c.desc.wind.sin());
        // Waves run before the wind, so the coast they break on faces back up it.
        let facing = seaward.dot(-wind).max(0.0);
        let centre = vec2(x as f32 + 0.5, z as f32 + 0.5);
        // Fetch: how much open water lies upwind for the wind to raise waves over.
        let step = m(40.0) / cs;
        let fetch = (1..=8).filter(|&k| sea(centre - wind * (step * k as f32))).count() as f32 / 8.0;
        // Openness: sea all round on a headland, little of it in a bay.
        let ring = |r: f32| {
            (0..12)
                .filter(|&k| {
                    let a = k as f32 * std::f32::consts::TAU / 12.0;
                    sea(centre + vec2(a.cos(), a.sin()) * r)
                })
                .count() as f32
                / 12.0
        };
        let open = 0.5 * (ring(m(110.0) / cs) + ring(m(220.0) / cs));
        let onshore = facing * (0.25 + 0.75 * fetch);
        let mut hinter = 0.0f32;
        for dz in -1..=1 {
            for dx in -1..=1 {
                hinter = hinter.max(height(x + dx, z + dz));
            }
        }
        ShoreCell {
            exposure: (0.65 * onshore + 0.35 * smoothstep(0.45, 0.8, open)).min(1.0),
            onshore,
            shelter: 1.0 - smoothstep(0.25, 0.55, open),
            hinter: hinter * OLD_VOXEL,
        }
    }
}

/// The shore of every grid cell an area's columns blend between, worked out once per
/// sampler. A cell outside it is worked out when asked for, so a column's shore never
/// depends on the area it was sampled with.
pub struct Shores {
    x0: i32,
    z0: i32,
    w: i32,
    h: i32,
    cells: Vec<ShoreCell>,
}

impl Shores {
    /// The shores for columns from `lo` to `hi` (continent-local voxels). Empty if no sea
    /// is near.
    pub fn new(c: &Continent, lo: Vec2, hi: Vec2) -> Self {
        let n = c.size as i32;
        let cs = c.cell_size();
        // The cell-centre grid corners a column blends between, and one more each way.
        let corner = |v: f32| ((v / cs - 0.5).floor() as i32).clamp(0, n - 1);
        let (x0, z0) = ((corner(lo.x) - 1).max(0), (corner(lo.y) - 1).max(0));
        let (x1, z1) = ((corner(hi.x) + 2).min(n - 1), (corner(hi.y) + 2).min(n - 1));
        let any_sea = (z0..=z1).any(|z| (x0..=x1).any(|x| c.ocean[(z * n + x) as usize]));
        if !any_sea {
            return Shores { x0, z0, w: 0, h: 0, cells: Vec::new() };
        }
        let (w, h) = (x1 - x0 + 1, z1 - z0 + 1);
        let cells = (0..h)
            .flat_map(|dz| (0..w).map(move |dx| (x0 + dx, z0 + dz)))
            .map(|(x, z)| ShoreCell::compute(c, x, z))
            .collect();
        Shores { x0, z0, w, h, cells }
    }

    fn cell(&self, c: &Continent, i: usize) -> ShoreCell {
        let (x, z) = ((i % c.size) as i32, (i / c.size) as i32);
        let (dx, dz) = (x - self.x0, z - self.z0);
        if dx >= 0 && dz >= 0 && dx < self.w && dz < self.h {
            self.cells[(dz * self.w + dx) as usize]
        } else {
            ShoreCell::compute(c, x, z)
        }
    }

    /// The shore at `p`, a column whose grid corners are `corners` (in the order the
    /// sampler blends them) at blend weights `tx`, `tz`.
    pub fn at(&self, c: &Continent, corners: &[usize; 4], tx: f32, tz: f32, p: Vec2, mouths: &[Mouth]) -> Shore {
        let weights = [(1.0 - tx) * (1.0 - tz), tx * (1.0 - tz), (1.0 - tx) * tz, tx * tz];
        let cells = corners.map(|i| self.cell(c, i));
        let blend = |f: fn(&ShoreCell) -> f32| cells.iter().zip(weights).map(|(s, w)| f(s) * w).sum::<f32>();
        let biome = corners
            .iter()
            .filter(|&&i| !c.ocean[i])
            .max_by(|&&a, &&b| c.height[a].total_cmp(&c.height[b]))
            .map_or(Biome::Ocean, |&i| Biome::from_u8(c.biome[i]));
        Shore {
            exposure: blend(|s| s.exposure),
            onshore: blend(|s| s.onshore),
            shelter: blend(|s| s.shelter),
            hinter: blend(|s| s.hinter),
            seaness: corners.iter().zip(weights).map(|(&i, w)| if c.ocean[i] { w } else { 0.0 }).sum(),
            mouth: mouths.iter().map(|m| m.weight(p)).fold(0.0, f32::max),
            temperature: corners.iter().zip(weights).map(|(&i, w)| c.temperature[i] * w).sum(),
            biome,
            wind: c.desc.wind,
            strand: Strand::Sand,
            dune: 0.0,
        }
    }
}

/// What a beach is made of.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Strand {
    Sand,
    Shingle,
    Boulders,
    /// Estuary and swamp coast: mud flats with reeds.
    Mud,
}

/// The shore at one column: everything that decides what kind of coast it is.
#[derive(Clone, Copy, Debug)]
pub struct Shore {
    /// How hard waves hit, 0 to 1.
    pub exposure: f32,
    /// How squarely the coast faces the prevailing wind over open water, 0 to 1.
    pub onshore: f32,
    /// How deep in a bay, 0 to 1.
    pub shelter: f32,
    /// The highest ground nearby, in voxels.
    pub hinter: f32,
    /// The share of the surrounding grid that is sea. Every shore effect fades out with
    /// it, so the coast blends into the inland ground where the sampler stops treating
    /// columns as coastal.
    pub seaness: f32,
    /// Nearness to a river mouth, 0 to 1.
    pub mouth: f32,
    pub temperature: f32,
    /// The biome of the land behind the shore.
    pub biome: Biome,
    /// Direction the prevailing wind blows toward, radians.
    wind: f32,
    strand: Strand,
    /// Height added by dunes, in voxels.
    dune: f32,
}

impl Shore {
    /// How far every shore effect is faded in, 0 to 1.
    pub fn fade(&self) -> f32 {
        smoothstep(0.0, 0.15, self.seaness)
    }

    /// Shapes the ground of a coastal column, whose natural ground is `ground` and whose
    /// simulated slope is `slope` (voxels per voxel): sea cliffs and their rock shelves,
    /// shingle berms, boulders, flattened bays, dunes and mud flats. Returns the cover of
    /// rock it leaves, if any; [`Shore::cover`] gives the cover of anything else.
    pub fn shape(&mut self, seed: u32, slope: f32, wx: f64, wz: f64, ground: &mut f32) -> Option<Cover> {
        match sea_cliff::apply(seed, self, slope, wx, wz, ground) {
            sea_cliff::Part::Face => return Some(Cover::Cliff),
            sea_cliff::Part::Shelf => return Some(Cover::Shelf),
            sea_cliff::Part::Foot => {
                // Blocks fallen from the face lie along its foot.
                *ground += boulder(seed, wx, wz);
                return Some(Cover::Boulders);
            }
            sea_cliff::Part::None => {}
        }
        let fade = self.fade();
        let (stony, mud) = self.scores(seed, slope, wx, wz);
        let jitter = (unit(hash2(seed ^ 0x5b1f, wx as i32 >> 1, wz as i32 >> 1)) - 0.5) * 0.12;
        self.strand = if mud + jitter > 0.5 {
            Strand::Mud
        } else if stony + jitter > 0.95 {
            Strand::Boulders
        } else if stony + jitter > 0.6 {
            Strand::Shingle
        } else {
            Strand::Sand
        };

        // Shaping follows the scores rather than the strand chosen, so the ground stays
        // continuous where one kind of beach gives way to another.
        let g = *ground;
        let stone_w = smoothstep(0.5, 0.7, stony);
        let mud_w = smoothstep(0.35, 0.65, mud) * fade;
        let sand_w = (1.0 - stone_w) * (1.0 - mud_w);
        // Sheltered bays: a flatter beach, so a longer, wider strand.
        const TOP: f32 = m(4.0);
        if g > 0.0 && g < TOP {
            let a = 1.0 - 0.5 * self.shelter * sand_w * fade;
            *ground = g * (a + (1.0 - a) * g / TOP);
        }
        // Waves throw shingle up into a storm ridge along the top of the beach.
        let berm = 1.0 - ((g - m(1.8)) / m(1.1)).powi(2);
        if berm > 0.0 {
            *ground += m(0.55) * berm * berm * stone_w * fade;
        }
        // Boulders on the most exposed shores, from the waterline up the beach.
        let rocks = smoothstep(0.85, 1.0, stony) * fade;
        if rocks > 0.0 {
            let band = smoothstep(-m(1.5), -m(0.5), g) * (1.0 - smoothstep(m(2.5), m(3.6), g));
            *ground += boulder(seed, wx, wz) * rocks * band;
        }
        // Estuary flats: low mud just above the sea, cut by winding creeks.
        if mud_w > 0.0 {
            let band = smoothstep(-m(1.4), -m(0.4), g) * (1.0 - smoothstep(m(2.6), m(3.8), g));
            let creek = perlin2(seed ^ 0x30d2, wx / m64(34.0), wz / m64(34.0)).abs() < 0.05;
            let flat = if creek {
                -m(0.9)
            } else {
                m(0.35) + g.max(0.0) * 0.12 + perlin2(seed ^ 0x30d1, wx / m64(14.0), wz / m64(14.0)) * m(0.25)
            };
            *ground += (flat - *ground) * mud_w * band;
        }
        // Dunes heaped up behind wide beaches by the onshore wind.
        self.dune = self.dune_height(seed, slope, wx, wz, g) * sand_w * fade;
        *ground += self.dune;
        None
    }

    /// How stony and how muddy this shore is, from about 0 to 1 and beyond: the stony
    /// score picks shingle and boulders, the muddy one estuary flats.
    fn scores(&self, seed: u32, slope: f32, wx: f64, wz: f64) -> (f32, f32) {
        let cold = 1.0 - smoothstep(0.12, 0.32, self.temperature);
        let dry = matches!(self.biome, Biome::Desert | Biome::DuneSea | Biome::Savanna | Biome::RedRock);
        let swamp = matches!(self.biome, Biome::Mangrove | Biome::Marsh | Biome::Bog);
        let mut stony = self.exposure * 0.95 + cold * 0.3 + smoothstep(0.15, 0.45, slope) * 0.35 - self.shelter * 0.4
            + perlin2(seed ^ 0x5b1e, wx / m64(90.0), wz / m64(90.0)) * 0.3;
        if dry {
            stony -= 0.25;
        }
        let gentle = 1.0 - smoothstep(0.1, 0.3, slope);
        let mud = self.mouth * gentle + if swamp { 0.75 * gentle * (1.0 - self.exposure) } else { 0.0 };
        (stony * (1.0 - self.mouth), mud)
    }

    /// The height dunes add here, in voxels, for natural ground `g`.
    fn dune_height(&self, seed: u32, slope: f32, wx: f64, wz: f64, g: f32) -> f32 {
        let zone = smoothstep(m(1.4), m(3.0), g) * (1.0 - smoothstep(m(7.0), m(12.0), g));
        if zone <= 0.0 {
            return 0.0;
        }
        let sandy = matches!(self.biome, Biome::Desert | Biome::DuneSea);
        let wide = 1.0 - smoothstep(0.08, 0.25, slope);
        let warm = smoothstep(0.14, 0.28, self.temperature);
        let supply = (0.3 + 0.7 * self.onshore + if sandy { 0.5 } else { 0.0 }).min(1.0);
        let patch = smoothstep(-0.3, 0.1, perlin2(seed ^ 0xd0e1, wx / m64(220.0), wz / m64(220.0)));
        let w = zone * wide * warm * supply * patch * (1.0 - self.mouth);
        if w <= 0.0 {
            return 0.0;
        }
        // Ridges lie across the wind: the noise changes quickly along it and slowly across.
        let (cw, sw) = (self.wind.cos() as f64, self.wind.sin() as f64);
        let along = (wx * cw + wz * sw) / m64(26.0);
        let across = (wz * cw - wx * sw) / m64(80.0);
        let n = perlin2(seed ^ 0xd0e2, along, across) + 0.3 * perlin2(seed ^ 0xd0e3, wx / m64(9.0), wz / m64(9.0));
        let crest = (1.0 - n.abs() * 1.8).max(0.0);
        m(3.4) * w * (0.3 + 0.7 * crest * crest * (3.0 - 2.0 * crest))
    }

    /// The cover of a dry coastal column whose final ground is `ground`, or `None` for
    /// ordinary land.
    pub fn cover(&self, ground: f32) -> Option<Cover> {
        match self.strand {
            Strand::Mud => (ground < m(1.6)).then_some(Cover::Bank),
            Strand::Shingle => (ground < m(3.0)).then_some(Cover::Shingle),
            Strand::Boulders => (ground < m(3.4)).then_some(Cover::Boulders),
            Strand::Sand if ground < m(2.4) + m(1.2) * self.shelter => Some(Cover::Beach),
            Strand::Sand => (self.dune > m(0.35)).then_some(Cover::Dune),
        }
    }
}

/// A boulder's height above the ground at world column `(wx, wz)`, or 0: rounded stones
/// up to 2 m across, at most one to each 6-voxel square so they never run together.
pub fn boulder(seed: u32, wx: f64, wz: f64) -> f32 {
    const CELL: i32 = 6;
    let (x, z) = (wx.floor() as i32, wz.floor() as i32);
    let (cx, cz) = (x.div_euclid(CELL), z.div_euclid(CELL));
    let h = hash2(seed ^ 0xb01d, cx, cz);
    if unit(h) > 0.6 {
        return 0.0;
    }
    let r = 1.4 + 1.0 * unit(h.rotate_left(5));
    let span = CELL as f32 - 2.0 * r;
    let (ox, oz) = (r + span * unit(h.rotate_left(10)), r + span * unit(h.rotate_left(15)));
    let (dx, dz) = ((x - cx * CELL) as f32 + 0.5 - ox, (z - cz * CELL) as f32 + 0.5 - oz);
    let d2 = (dx * dx + dz * dz) / (r * r);
    if d2 >= 1.0 { 0.0 } else { (m(0.35) + m(0.6) * unit(h.rotate_left(20))) * (1.0 - d2).sqrt() }
}
