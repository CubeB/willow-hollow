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

use super::{Cover, MIN_RIVER_Q, coast_ground, funnel, mouth_end, node, sea_cliff, smoothstep};

/// Smallest discharge that builds a delta.
const DELTA_Q: f32 = 400.0;

/// Farthest a delta's fan reaches from its apex, in voxels.
const DELTA_RADIUS: f32 = m(110.0);

/// Line segments a distributary's curve is flattened into.
const BRANCH_SEGS: usize = 6;

/// A distributary: a channel that leaves a delta's main channel and runs out across
/// the fan to the sea.
struct Branch {
    /// Its centreline in the delta's own frame: x out to sea along the main channel,
    /// y to the side.
    pts: [Vec2; BRANCH_SEGS + 1],
    /// Half its width where it leaves the main channel.
    half: f32,
}

impl Branch {
    /// A branch leaving the main channel `start` out from the apex, heading `a0` off
    /// the main channel's direction to begin with and `a1` by its end `length` on.
    fn new(start: f32, length: f32, side: f32, a0: f32, a1: f32, half: f32) -> Branch {
        let heading = |a: f32| vec2(a.cos(), side * a.sin());
        let p0 = vec2(start, 0.0);
        let p1 = p0 + heading(a0) * (length * 0.5);
        let p2 = p1 + heading(a1) * (length * 0.5);
        let mut pts = [Vec2::ZERO; BRANCH_SEGS + 1];
        for (k, pt) in pts.iter_mut().enumerate() {
            let t = k as f32 / BRANCH_SEGS as f32;
            *pt = p0 * ((1.0 - t) * (1.0 - t)) + p1 * (2.0 * (1.0 - t) * t) + p2 * (t * t);
        }
        Branch { pts, half }
    }

    /// How far `q` (in the delta's frame) is from the centreline, as a share of the
    /// channel's half-width there: under 1 inside the channel.
    fn across(&self, q: Vec2) -> f32 {
        let mut best = f32::MAX;
        for (k, w) in self.pts.windows(2).enumerate() {
            let ab = w[1] - w[0];
            let t = ((q - w[0]).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
            // Narrowing towards the sea.
            let half = self.half * (1.0 - 0.45 * (k as f32 + t) / BRANCH_SEGS as f32);
            best = best.min((q - (w[0] + ab * t)).length() / half);
        }
        best
    }
}

/// A small delta: a fan of low mud flats built out into shallow sea in front of a big,
/// slow river, split by a few distributary channels.
///
/// The fan opens from where the river's channel meets open water. The main channel runs
/// straight on through it, and one or two distributaries leave it on each side, each
/// heading out to sea at an angle away from the main channel and bending a little as
/// it goes. On one side the branch that leaves first always heads further off than the
/// one that leaves after it, so no two channels can meet again or cross, and none can
/// turn back to land.
pub struct Delta {
    /// The apex of the fan, where the river reaches open water, and the way it flows
    /// out to sea.
    pub(super) pos: Vec2,
    dir: Vec2,
    /// Width of the river at its mouth, in voxels.
    pub(super) width: f32,
    /// How far the fan reaches out to sea at most.
    pub radius: f32,
    branches: Vec<Branch>,
    salt: u32,
}

impl Delta {
    /// The delta at the mouth of the river leaving node `i`, if it builds one: only big
    /// rivers whose last reach is already down at sea level, so slow and silty, and
    /// only the biggest of any whose fans would run into each other.
    pub fn at(c: &Continent, seed: u32, i: usize) -> Option<Delta> {
        let mut delta = Self::fan(c, seed, i)?;
        // Mouths close enough for their fans to overlap. A node can sit several cells
        // from its own cell, and the apex some cells on from the node.
        let n = c.size as i32;
        let (x, z) = ((i % c.size) as i32, (i / c.size) as i32);
        let span = (2.0 * DELTA_RADIUS / c.cell_size()).ceil() as i32 + 10;
        // The channels other rivers run out to sea in nearby: where each starts and
        // ends, and half its width.
        let mut mouths = Vec::new();
        for nz in (z - span).max(1)..=(z + span).min(n - 2) {
            for nx in (x - span).max(1)..=(x + span).min(n - 2) {
                let j = (nz * n + nx) as usize;
                let r = c.rcv[j] as usize;
                if j == i || c.ocean[j] || !c.ocean[r] || c.discharge[j] < MIN_RIVER_Q {
                    continue;
                }
                let bigger = (c.discharge[j], i) > (c.discharge[i], j);
                if bigger
                    && let Some(other) = Self::fan(c, seed, j)
                    && (other.pos - delta.pos).length() < other.radius + delta.radius
                {
                    return None;
                }
                let half = Continent::river_width(c.discharge[j]) * OLD_VOXEL * funnel(c, r, c.discharge[j]) * 0.65;
                mouths.push(((node(c, j) + node(c, r)) * 0.5, mouth_end(c, j), half));
            }
        }
        delta.branch(c, &mouths);
        Some(delta)
    }

    /// The fan the river leaving node `i` would build, without its distributaries.
    fn fan(c: &Continent, seed: u32, i: usize) -> Option<Delta> {
        let r = c.rcv[i] as usize;
        let q = c.discharge[i];
        if c.ocean[i] || !c.ocean[r] || q < DELTA_Q || c.water_level[i] > 3.0 {
            return None;
        }
        let (a, b) = (node(c, i), node(c, r));
        let exit = (a + b) * 0.5;
        let dir = (b - a).try_normalize()?;
        // The coast the simulation drew is rarely where the river crosses it: follow
        // the channel out to the first open water, so the fan stands in the sea rather
        // than half of it lying buried under the land beside the last reach.
        let run = (mouth_end(c, i) - exit).length();
        let step = c.cell_size() / 8.0;
        let pos = (0..=(run / step) as usize).map(|k| exit + dir * (k as f32 * step)).find(|&p| coast_ground(c, p).is_some_and(|g| g < -m(0.4)))?;
        let width = Continent::river_width(q) * OLD_VOXEL * funnel(c, r, q);
        Some(Delta {
            pos,
            dir,
            width,
            radius: (width * 5.0).clamp(m(40.0), DELTA_RADIUS),
            branches: Vec::new(),
            salt: hash2(seed ^ 0xde17, i as i32, 1),
        })
    }

    /// Splits the main channel into distributaries, none of which may run into one of
    /// the `mouths` of other rivers (the ends of each one's channel and half its width).
    fn branch(&mut self, c: &Continent, mouths: &[(Vec2, Vec2, f32)]) {
        let roll = |k: i32| unit(hash2(self.salt ^ 0xb2a, k, 3));
        let reach = self.radius * 1.25;
        let mut branches = Vec::new();
        for (n, side) in [1.0f32, -1.0].into_iter().enumerate() {
            let k = n as i32 * 8;
            // The first to leave swings well out to the side.
            let start = self.radius * (0.05 + 0.12 * roll(k));
            // It bends as it goes, one way or the other.
            let a0 = 0.5 + 0.3 * roll(k + 1);
            let a1 = if a0 > 0.65 { 0.5 } else { 0.68 } + 0.12 * roll(k + 2);
            let half = (self.width * 0.3).clamp(m(0.8), m(4.5));
            branches.push(Branch::new(start, reach - start, side, a0, a1, half));
            // Bigger deltas split again further out, the second branch keeping
            // between the first and the main channel.
            if self.width > m(8.0) && roll(k + 3) < 0.65 {
                let start = self.radius * (0.3 + 0.15 * roll(k + 4));
                let a0 = 0.2 + 0.22 * roll(k + 5);
                let a1 = if a0 > 0.31 { 0.2 } else { 0.34 } + 0.08 * roll(k + 6);
                let half = (self.width * 0.22).clamp(m(0.8), m(3.5));
                branches.push(Branch::new(start, reach - start, side, a0, a1, half));
            }
        }
        // A branch must cross open water all the way: one that would run into the
        // shore beside the mouth, an islet, or another river's channel is not cut at all.
        branches.retain(|b| {
            b.pts.windows(2).flat_map(|w| [w[0], (w[0] + w[1]) * 0.5, w[1]]).all(|q| {
                let p = self.place(q);
                let clear = mouths.iter().all(|&(from, to, half)| {
                    let run = to - from;
                    let t = ((p - from).dot(run) / run.length_squared().max(1e-6)).clamp(0.0, 1.0);
                    (p - (from + run * t)).length() > half + b.half + m(4.0)
                });
                q.length() > self.radius * 1.2 || (clear && coast_ground(c, p).is_some_and(|g| g < 0.0))
            })
        });
        self.branches = branches;
    }

    /// A point of the delta's own frame (x out to sea, y to the side) in
    /// continent-local voxels.
    fn place(&self, q: Vec2) -> Vec2 {
        self.pos + self.dir * q.x + self.dir.perp() * q.y
    }

    /// The centrelines of the delta's channels, in continent-local voxels: the main
    /// channel first, then each distributary from where it leaves the main channel.
    #[cfg(test)]
    pub fn channels(&self) -> Vec<Vec<Vec2>> {
        let main = vec![self.pos, self.place(vec2(self.radius * 1.25, 0.0))];
        std::iter::once(main).chain(self.branches.iter().map(|b| b.pts.iter().map(|&q| self.place(q)).collect())).collect()
    }

    /// The way the river flows out to sea.
    #[cfg(test)]
    pub fn seaward(&self) -> Vec2 {
        self.dir
    }

    /// Builds the delta into a sea-side column at `p` whose ground is `ground`. Returns
    /// the cover of new flats, or `None` where the delta leaves the column alone or cuts
    /// a channel through it (the sea then fills the channel).
    pub fn shape(&self, p: Vec2, wx: f64, wz: f64, ground: &mut f32) -> Option<Cover> {
        let v = p - self.pos;
        let d = v.length();
        let along = v.dot(self.dir);
        if d > self.radius * 1.2 || along < 0.0 || *ground < -m(6.0) {
            return None;
        }
        // The fan is built on the shallows and slopes away where the sea deepens,
        // rather than ending in a wall over deep water.
        let shelf = smoothstep(-m(6.0), -m(2.5), *ground);
        let side = self.dir.perp_dot(v);
        // Channels: the main one straight out, narrowing towards the sea, and the
        // distributaries that leave it. They run on past the edge of the fan, so
        // whatever shoals lie beyond it cannot close them off.
        let main = side.abs() / (self.width * 0.5 * (1.0 - 0.4 * (d / self.radius).min(1.0)));
        let q = vec2(along, side);
        let across = self.branches.iter().map(|b| b.across(q)).fold(main, f32::min);
        if across < 1.0 {
            let depth = m(1.0) + m(0.8) * (1.0 - across);
            *ground = ground.min(-depth);
            return None;
        }
        let angle = side.atan2(along);
        // The fan's edge, lobed.
        let lobes = perlin2(self.salt, angle as f64 * 2.5, 0.5) * 0.3;
        let reach = self.radius * (1.0 - (angle / 1.3).powi(2)).max(0.0) * (0.85 + lobes);
        if d > reach {
            return None;
        }
        let t = d / reach.max(1.0);
        // Flats a little above the sea, fading into it at the fan's edge.
        let flat = m(0.2) + m(0.3) * (1.0 - t) + perlin2(self.salt ^ 0x77, wx / m64(15.0), wz / m64(15.0)) * m(0.2);
        let edge = smoothstep(0.82, 1.0, t).max(1.0 - shelf);
        *ground = ground.max(flat + (*ground - flat) * edge);
        Some(if edge > 0.3 { Cover::Beach } else { Cover::Bank })
    }
}

/// The deltas whose fans can reach the area from `lo` to `hi` (continent-local voxels).
/// Gathered from far enough out that every column sees the same deltas whatever area
/// it is sampled with.
pub fn deltas(c: &Continent, seed: u32, lo: Vec2, hi: Vec2) -> Vec<Delta> {
    let cs = c.cell_size();
    // A river node can sit several cells from its own cell (see `node`), and the fan's
    // apex as far again along the mouth's channel.
    let pad = DELTA_RADIUS * 1.2 + cs * ((10.0 * (c.size as f32 / 1024.0)).max(8.0) + 5.0);
    let cell = |v: f32| ((v / cs).floor() as i32).clamp(1, c.size as i32 - 2) as usize;
    let mut out = Vec::new();
    for z in cell(lo.y - pad)..=cell(hi.y + pad) {
        for x in cell(lo.x - pad)..=cell(hi.x + pad) {
            let i = z * c.size + x;
            // Cheap checks first: almost no cell is a big river's mouth.
            if c.ocean[i] || !c.ocean[c.rcv[i] as usize] || c.discharge[i] < DELTA_Q {
                continue;
            }
            if let Some(delta) = Delta::at(c, seed, i)
                && (delta.pos - delta.pos.clamp(lo, hi)).length() < delta.radius * 1.2
            {
                out.push(delta);
            }
        }
    }
    out
}

/// Raises sand bars on the bed of a tidal river channel. `bed` is the channel bed and
/// `across` runs from 0 mid-stream to 1 at the channel edge. Some bars break the surface
/// as low sand islands, flush with the water or a voxel proud of it.
pub fn sand_bar(seed: u32, wx: f64, wz: f64, across: f32, bed: f32) -> f32 {
    if across > 0.85 {
        return bed;
    }
    let b = perlin2(seed ^ 0xba55, wx / m64(16.0), wz / m64(16.0));
    let lift = smoothstep(0.15, 0.45, b) * (1.0 - smoothstep(0.6, 0.85, across));
    // The crest is level with the water; only its highest middle stands a voxel above.
    let crest = if b > 0.4 { 0.5 } else { -0.5 };
    bed + (crest - bed) * lift
}

/// Farthest from a river mouth that its estuary flats reach, in voxels.
const MOUTH_REACH: f32 = m(120.0);

/// How far beyond a mouth's estuary the aimless creeks of swamp coasts keep clear of it,
/// as a multiple of the estuary's reach.
const MOUTH_CLEAR: f32 = 1.6;

/// Where a river reaches the sea: near it the shore is estuary mud and reeds rather than
/// beach or cliff.
pub struct Mouth {
    pos: Vec2,
    /// The way the river flows out to sea.
    dir: Vec2,
    /// The stretch of its channel either side of the coast.
    run: (Vec2, Vec2),
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
            dir: (node(c, r) - node(c, i)).try_normalize()?,
            run: (node(c, i), mouth_end(c, i)),
            inner,
            outer: inner * 2.2,
            strength: smoothstep(MIN_RIVER_Q * 4.0, 600.0, q),
        })
    }

    fn weight(&self, p: Vec2) -> f32 {
        (1.0 - smoothstep(self.inner, self.outer, (p - self.pos).length())) * self.strength
    }

    /// Farthest from the mouth that it changes the shore.
    fn reach(&self) -> f32 {
        let run = (self.run.0 - self.pos).length().max((self.run.1 - self.pos).length());
        (self.outer * MOUTH_CLEAR).max(run + m(16.0))
    }

    /// How far `p` is from the river's channel where it runs out to sea.
    fn clearance(&self, p: Vec2) -> f32 {
        let run = self.run.1 - self.run.0;
        let t = ((p - self.run.0).dot(run) / run.length_squared().max(1e-6)).clamp(0.0, 1.0);
        (p - (self.run.0 + run * t)).length()
    }

    /// How near `p` is to the mouth, 0 to 1, out to well beyond its estuary.
    fn nearness(&self, p: Vec2) -> f32 {
        1.0 - smoothstep(self.outer, self.outer * MOUTH_CLEAR, (p - self.pos).length())
    }
}

/// The river mouths whose estuaries can reach the area from `lo` to `hi` (continent-local
/// voxels). Gathered from far enough out that every column sees the same mouths whatever
/// area it is sampled with.
pub fn mouths(c: &Continent, lo: Vec2, hi: Vec2) -> Vec<Mouth> {
    let cs = c.cell_size();
    // A river node can sit several cells from its own cell (see `node`).
    let pad = MOUTH_REACH * MOUTH_CLEAR + cs * (10.0 * (c.size as f32 / 1024.0)).max(8.0);
    let cell = |v: f32| ((v / cs).floor() as i32).clamp(1, c.size as i32 - 2) as usize;
    let mut out = Vec::new();
    for z in cell(lo.y - pad)..=cell(hi.y + pad) {
        for x in cell(lo.x - pad)..=cell(hi.x + pad) {
            if let Some(mouth) = Mouth::at(c, z * c.size + x) {
                let near = mouth.pos.clamp(lo, hi);
                if (mouth.pos - near).length() < mouth.reach() {
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
        // The mouth whose estuary this is, and how clearly it is that one's rather than
        // the next one's.
        let (mut mouth, mut second, mut creek, mut own) = (0.0f32, 0.0f32, Vec2::ZERO, usize::MAX);
        for (k, m) in mouths.iter().enumerate() {
            let w = m.weight(p);
            if w > mouth {
                second = mouth;
                mouth = w;
                creek = vec2((p - m.pos).dot(m.dir), m.dir.perp_dot(p - m.pos));
                own = k;
            } else {
                second = second.max(w);
            }
        }
        // Its creeks stop short of any other river's channel rather than crossing it.
        let rival = mouths.iter().enumerate().filter(|&(k, _)| k != own).map(|(_, m)| m.clearance(p)).fold(f32::MAX, f32::min);
        Shore {
            exposure: blend(|s| s.exposure),
            onshore: blend(|s| s.onshore),
            shelter: blend(|s| s.shelter),
            hinter: blend(|s| s.hinter),
            seaness: corners.iter().zip(weights).map(|(&i, w)| if c.ocean[i] { w } else { 0.0 }).sum(),
            mouth,
            mouth_near: mouths.iter().map(|m| m.nearness(p)).fold(0.0, f32::max),
            mouth_lead: (mouth - second).min(smoothstep(m(6.0), m(16.0), rival)),
            creek,
            temperature: corners.iter().zip(weights).map(|(&i, w)| c.temperature[i] * w).sum(),
            biome,
            wind: c.desc.wind,
            strand: Strand::Sand,
            dune: 0.0,
            rock: false,
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
    /// Nearness to any river mouth at all, however small, out to beyond its estuary.
    mouth_near: f32,
    /// How much nearer that mouth is than the next nearest, 0 to 1; 0 beside another
    /// river's channel.
    mouth_lead: f32,
    /// Where the column lies from that mouth, in voxels: how far out along the way its
    /// river runs to sea, and how far to the side.
    creek: Vec2,
    pub temperature: f32,
    /// The biome of the land behind the shore.
    pub biome: Biome,
    /// Direction the prevailing wind blows toward, radians.
    wind: f32,
    strand: Strand,
    /// Height added by dunes, in voxels.
    dune: f32,
    /// Whether the column is the top of a boulder: bare rock, with sand between.
    rock: bool,
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
                // Blocks fallen from the face lie along its foot, on sand.
                let b = boulder(seed, wx, wz);
                *ground += b;
                return Some(if b > 0.0 { Cover::Boulders } else { Cover::Beach });
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
            let b = boulder(seed, wx, wz) * rocks * band;
            self.rock = b > 0.25;
            *ground += b;
        }
        // Estuary flats: low mud just above the sea, cut by creeks.
        if mud_w > 0.0 {
            let band = smoothstep(-m(2.4), -m(0.4), g) * (1.0 - smoothstep(m(2.6), m(3.8), g));
            let mut flat = m(0.2) + g.max(0.0) * 0.12 + perlin2(seed ^ 0x30d1, wx / m64(14.0), wz / m64(14.0)) * m(0.15);
            flat += (-m(0.9) - flat) * self.creek_depth(seed, wx, wz);
            *ground += (flat - *ground) * mud_w * band;
        }
        // Dunes heaped up behind wide beaches by the onshore wind.
        self.dune = self.dune_height(seed, slope, wx, wz, g) * sand_w * fade;
        *ground += self.dune;
        None
    }

    /// How far a tidal creek is cut into the mud flats at this column, 0 to 1.
    ///
    /// Beside a river mouth the creeks drain the way the river does: each runs out to
    /// sea alongside it, swaying a little, and since each keeps its own distance to the
    /// side of the river none can loop, turn back or cross another. Swamp coasts away
    /// from any river mouth keep aimless winding creeks.
    fn creek_depth(&self, seed: u32, wx: f64, wz: f64) -> f32 {
        let winding = if perlin2(seed ^ 0x30d2, wx / m64(34.0), wz / m64(34.0)).abs() < 0.05 {
            1.0 - self.mouth_near
        } else {
            0.0
        };
        if self.mouth <= 0.1 || self.mouth_lead <= 0.0 {
            return winding;
        }
        let (along, side) = (self.creek.x as f64, self.creek.y as f64);
        let sway = perlin2(seed ^ 0x30d3, along / m64(45.0), 0.5) as f64 * m64(8.0);
        // One creek at most in each strip of flats beside the river.
        let strip = (side + sway) / m64(32.0);
        let h = hash2(seed ^ 0x30d4, strip.floor() as i32, 0);
        let off = ((strip - strip.floor()) as f32 - (0.25 + 0.5 * unit(h))).abs() * m(32.0);
        let draining = if unit(h.rotate_left(20)) < 0.75 && off < m(0.7) + m(0.7) * unit(h.rotate_left(11)) {
            // Shoaling out where the next mouth's creeks take over.
            smoothstep(0.1, 0.35, self.mouth) * smoothstep(0.0, 0.25, self.mouth_lead)
        } else {
            0.0
        };
        winding.max(draining)
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
            // Between the boulders, ordinary sand.
            Strand::Boulders => (ground < m(3.4)).then_some(if self.rock { Cover::Boulders } else { Cover::Beach }),
            Strand::Sand if ground < m(2.4) + m(1.2) * self.shelter => Some(Cover::Beach),
            Strand::Sand => (self.dune > m(0.35)).then_some(Cover::Dune),
        }
    }
}

/// A boulder's height above the ground at world column `(wx, wz)`, or 0: rounded stones
/// 1.8 to 3.4 m across and up to 2 m high, at most one to each 10-voxel square so they never run together.
pub fn boulder(seed: u32, wx: f64, wz: f64) -> f32 {
    const CELL: i32 = 10;
    let (x, z) = (wx.floor() as i32, wz.floor() as i32);
    let (cx, cz) = (x.div_euclid(CELL), z.div_euclid(CELL));
    let h = hash2(seed ^ 0xb01d, cx, cz);
    if unit(h) > 0.6 {
        return 0.0;
    }
    let r = 2.2 + 2.0 * unit(h.rotate_left(5));
    let span = CELL as f32 - 2.0 * r;
    let (ox, oz) = (r + span * unit(h.rotate_left(10)), r + span * unit(h.rotate_left(15)));
    let (dx, dz) = ((x - cx * CELL) as f32 + 0.5 - ox, (z - cz * CELL) as f32 + 0.5 - oz);
    let d2 = (dx * dx + dz * dz) / (r * r);
    if d2 >= 1.0 { 0.0 } else { (m(0.9) + m(1.1) * unit(h.rotate_left(20))) * (1.0 - d2).powf(0.3) }
}
