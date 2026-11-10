//! The voxel-resolution surface: turns a continent's coarse simulation grid into ground
//! and water heights for individual voxel columns.
//!
//! Terrain is interpolated from the grid and given fine detail. Rivers are not sampled
//! from the grid at all: they are smooth curves through the drainage network, carved
//! into the terrain, with water levels taken from the simulation's never-rising
//! profile. That is what keeps every stream continuous and flowing downhill at a
//! resolution 32 times finer than the simulation.

use glam::{Vec2, vec2};

use crate::{
    biome::Biome,
    continent::{Continent, NO_LAKE},
    macro_layout::MACRO_SIZE,
    noise::{fbm2, hash2, perlin2, unit},
    trails::{Pad, TrailPoint},
};

/// Smallest discharge that forms a visible stream.
pub const MIN_RIVER_Q: f32 = 24.0;
/// Line segments each river curve is flattened into for distance queries.
const SEGS: usize = 6;
/// Narrowest channel half-width, so the thinnest brooks are still unbroken water.
const MIN_HALF_WIDTH: f32 = 0.75;
/// Taken from the distance to a trail's centreline to give the distance to its edge.
const TRAIL_CLEARANCE: f32 = 1.5;

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// What the top of a column is, which decides its material.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cover {
    Land,
    /// Within a step or two of a river channel.
    Bank,
    RiverBed,
    LakeBed,
    Beach,
    SeaFloor,
    /// A trodden path.
    Trail,
}

#[derive(Clone, Copy, Debug)]
pub struct Column {
    /// Ground height; the top solid voxel is at `floor(ground)`.
    pub ground: f32,
    /// Water surface plane. Water fills the voxels above the ground up to `water - 1`,
    /// so a column holds water only if `ground < water - 1`.
    pub water: Option<i32>,
    pub cover: Cover,
    pub biome: Biome,
    /// 0 = polar, 1 = tropical.
    pub temperature: f32,
    /// Distance to the edge of the nearest river channel (negative inside it), or
    /// `f32::MAX` with no river nearby.
    pub river: f32,
    /// Distance to the edge of the nearest trail or landmark, or `f32::MAX` with none
    /// nearby. Nothing grows where this is small.
    pub clear: f32,
}

impl Column {
    pub fn top(&self) -> i32 {
        self.ground.floor() as i32
    }

    /// Water surface plane if this column actually holds water.
    pub fn water_surface(&self) -> Option<i32> {
        self.water.filter(|&k| self.top() < k - 1)
    }
}

/// One smooth stretch of river: a quadratic curve from the midpoint of one network
/// edge, past a node, to the midpoint of the next. Consecutive pieces share endpoints
/// and tangents, so a river is a continuous smooth line however the grid routed it.
pub struct Piece {
    /// The curve flattened into line segments, in continent-local voxels.
    pub pts: [Vec2; SEGS + 1],
    /// Channel width at the start and end.
    w0: f32,
    w1: f32,
    /// Water level at the start, the middle and the end.
    l0: f32,
    l1: f32,
    l2: f32,
    /// Length of the curve in voxels.
    length: f32,
    /// Bounding box grown by the farthest the piece can influence terrain.
    lo: Vec2,
    hi: Vec2,
}

impl Piece {
    fn new(p0: Vec2, p1: Vec2, p2: Vec2, widths: (f32, f32), levels: (f32, f32, f32)) -> Self {
        let mut pts = [Vec2::ZERO; SEGS + 1];
        let (mut lo, mut hi) = (Vec2::MAX, Vec2::MIN);
        for (k, pt) in pts.iter_mut().enumerate() {
            let t = k as f32 / SEGS as f32;
            *pt = p0 * ((1.0 - t) * (1.0 - t)) + p1 * (2.0 * (1.0 - t) * t) + p2 * (t * t);
            lo = lo.min(*pt);
            hi = hi.max(*pt);
        }
        let reach = Vec2::splat(reach(widths.0.max(widths.1) * 0.5));
        let length = pts.windows(2).map(|w| (w[1] - w[0]).length()).sum::<f32>().max(1.0);
        Piece {
            pts,
            length,
            w0: widths.0,
            w1: widths.1,
            l0: levels.0,
            l1: levels.1,
            l2: levels.2,
            lo: lo - reach,
            hi: hi + reach,
        }
    }

    /// Distance from `p` to the curve, and the curve parameter (0..1) of the nearest point.
    fn nearest(&self, p: Vec2) -> (f32, f32) {
        let (mut best, mut best_t) = (f32::MAX, 0.0);
        for k in 0..SEGS {
            let (a, b) = (self.pts[k], self.pts[k + 1]);
            let ab = b - a;
            let s = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
            let d = (p - (a + ab * s)).length_squared();
            if d < best {
                (best, best_t) = (d, (k as f32 + s) / SEGS as f32);
            }
        }
        (best.sqrt(), best_t)
    }

    /// Channel half-width at curve parameter `t` (0..1).
    pub fn half_width(&self, t: f32) -> f32 {
        ((self.w0 + (self.w1 - self.w0) * t) * 0.5).max(MIN_HALF_WIDTH)
    }

    /// Simulated water level at curve parameter `t`, before pooling.
    pub fn level(&self, t: f32) -> f32 {
        if t < 0.5 {
            self.l0 + (self.l1 - self.l0) * (t * 2.0)
        } else {
            self.l1 + (self.l2 - self.l1) * (t * 2.0 - 1.0)
        }
    }
}

/// How far from the channel edge a river's banks are reshaped.
fn bank_width(half_width: f32) -> f32 {
    3.0 + 2.0 * half_width
}

/// Farthest from its centreline a river touches the terrain.
fn reach(half_width: f32) -> f32 {
    let hw = half_width.max(MIN_HALF_WIDTH);
    hw + bank_width(hw) + 1.0
}

/// Quantises a river's water level into flat pools separated by drops.
///
/// Only some integer levels are allowed, chosen by hash; water sits at the highest
/// allowed level at or below its simulated level. Because this is a non-decreasing
/// function of the level alone, the result still never rises downstream. On a gentle
/// river the allowed levels are far apart along its length, giving long still pools
/// and the odd weir; on a steep one they come quickly, giving cascades. Missing levels
/// become taller falls, more often at altitude.
pub fn pool_level(seed: u32, level: f32) -> i32 {
    let mut k = level.floor() as i32;
    while k > 0 {
        let keep = match k {
            ..24 => 0.80,
            24..120 => 0.60,
            _ => 0.45,
        };
        if unit(hash2(seed ^ 0x9001, k, 11)) < keep {
            break;
        }
        k -= 1;
    }
    k.max(0)
}

/// Samples the surface for the voxel columns of one chunk (or any small area).
pub struct Sampler<'a> {
    c: &'a Continent,
    world_seed: u32,
    pieces: Vec<Piece>,
    trail_points: Vec<TrailPoint>,
    pads: Vec<Pad>,
}

impl<'a> Sampler<'a> {
    /// Prepares to sample columns in the square starting at continent-local voxel
    /// `(lx0, lz0)` and `span` voxels wide.
    pub fn new(c: &'a Continent, world_seed: u32, lx0: i32, lz0: i32, span: i32) -> Self {
        let cs = c.cell_size();
        let widest = Continent::river_width(c.max_discharge.max(MIN_RIVER_Q));
        // A piece lies within a cell of its node, so gather nodes that close plus the
        // farthest a river can reach.
        let margin = reach(widest * 0.5) + cs * 1.5;
        let cell = |v: f32| ((v / cs).floor() as i32).clamp(1, c.size as i32 - 2) as usize;
        let (x0, x1) = (cell(lx0 as f32 - margin), cell((lx0 + span) as f32 + margin));
        let (z0, z1) = (cell(lz0 as f32 - margin), cell((lz0 + span) as f32 + margin));

        let mut pieces = Vec::new();
        for z in z0..=z1 {
            for x in x0..=x1 {
                add_pieces(c, z * c.size + x, &mut pieces);
            }
        }
        let (lo, hi) = (
            vec2(lx0 as f32, lz0 as f32),
            vec2((lx0 + span) as f32, (lz0 + span) as f32),
        );
        pieces.retain(|p| p.lo.x <= hi.x && p.hi.x >= lo.x && p.lo.y <= hi.y && p.hi.y >= lo.y);
        let (trail_points, pads) = c.trails.gather(lo, hi);
        Self {
            c,
            world_seed,
            pieces,
            trail_points,
            pads,
        }
    }

    /// The column at continent-local voxel `(lx, lz)`.
    pub fn column(&self, lx: i32, lz: i32) -> Column {
        let c = self.c;
        let cs = c.cell_size();
        let p = vec2(lx as f32 + 0.5, lz as f32 + 0.5);
        // Position in the grid of cell centres.
        let max = (c.size - 1) as f32;
        let (fx, fz) = ((p.x / cs - 0.5).clamp(0.0, max), (p.y / cs - 0.5).clamp(0.0, max));
        let (x0, z0) = ((fx as usize).min(c.size - 2), (fz as usize).min(c.size - 2));
        let corners = [
            z0 * c.size + x0,
            z0 * c.size + x0 + 1,
            (z0 + 1) * c.size + x0,
            (z0 + 1) * c.size + x0 + 1,
        ];
        let (tx, tz) = (fx - x0 as f32, fz - z0 as f32);
        let bilinear = |f: &[f32]| {
            let a = f[corners[0]] + (f[corners[1]] - f[corners[0]]) * tx;
            let b = f[corners[2]] + (f[corners[3]] - f[corners[2]]) * tx;
            a + (b - a) * tz
        };
        let lake = corners.iter().map(|&i| c.lake[i]).fold(NO_LAKE, f32::max);
        let near_sea = corners.iter().any(|&i| c.ocean[i]);

        // World coordinates, so detail and borders line up across continents.
        let wx = (c.desc.mx as i64 * MACRO_SIZE as i64 + lx as i64) as f64;
        let wz = (c.desc.mz as i64 * MACRO_SIZE as i64 + lz as i64) as f64;
        let seed = self.world_seed;

        let base = bicubic(c, fx, fz);
        // Fine relief the simulation grid cannot hold. Faded out at the waterline so
        // shores stay clean.
        let detail = fbm2(seed ^ 0xd1, wx / 56.0, wz / 56.0, 3, 2.0, 0.5) * 2.6
            + perlin2(seed ^ 0xd2, wx / 17.0, wz / 17.0) * 1.0
            + perlin2(seed ^ 0xd3, wx / 7.0, wz / 7.0) * 0.55;
        let mut ground = base + detail * ((base - 0.5).abs() / 4.0).clamp(0.12, 1.0);
        let mut water: Option<i32> = None;
        let mut cover = Cover::Land;
        let mut river = f32::MAX;

        // Standing water this column could be under: it must not be given river banks.
        let lake_level = (lake != NO_LAKE).then(|| pool_level(c.desc.seed, lake));
        let submerged = (near_sea && ground < 0.0)
            || lake_level.is_some_and(|k| ground < k as f32 - 1.0);

        // The river whose channel edge is nearest.
        // (signed distance to the channel edge, half-width, water level there, water
        // level a couple of voxels upstream)
        let mut nearest: Option<(f32, f32, f32, f32)> = None;
        for piece in &self.pieces {
            if p.x < piece.lo.x || p.x > piece.hi.x || p.y < piece.lo.y || p.y > piece.hi.y {
                continue;
            }
            let (dist, t) = piece.nearest(p);
            let hw = piece.half_width(t);
            let sd = dist - hw;
            if nearest.is_none_or(|n| sd < n.0) {
                let upstream = (t - 2.0 / piece.length).max(0.0);
                nearest = Some((sd, hw, piece.level(t), piece.level(upstream)));
            }
        }
        if let Some((sd, hw, level, upstream_level)) = nearest {
            river = sd;
            let k = pool_level(c.desc.seed, level);
            if sd < 0.0 {
                // In the channel: a parabolic bed, deepest mid-stream.
                let depth = (0.6 + 0.44 * hw).clamp(1.0, 7.0);
                let across = 1.0 + sd / hw; // 0 at the centre, 1 at the edge
                let bed = k as f32 - 1.01 - depth * (1.0 - across * across);
                ground = if submerged { ground.min(bed) } else { bed };
                water = Some(k);
                cover = Cover::RiverBed;
            } else if !submerged && sd < bank_width(hw) {
                // Banks meet the water flush with its surface, then rise (or fall) to
                // the surrounding terrain. Where the land is lower than the river this
                // builds the levee that keeps the water in.
                // Beside a drop the bank holds the level of the pool above it for a
                // couple of voxels, so the upper pool cannot spill around the lip.
                let bank = pool_level(c.desc.seed, upstream_level) as f32 - 0.5;
                ground = bank + (ground - bank) * smoothstep(0.0, bank_width(hw), sd);
                if sd < 1.6 {
                    cover = Cover::Bank;
                }
            }
        }

        if let Some(k) = lake_level
            && ground < k as f32 - 1.0
        {
            water = Some(water.map_or(k, |w| w.max(k)));
            if cover != Cover::RiverBed {
                cover = Cover::LakeBed;
            }
        }
        if ground < -1.0 {
            water = Some(water.map_or(0, |w| w.max(0)));
            if cover == Cover::Land {
                cover = Cover::SeaFloor;
            }
        } else if near_sea && ground < 3.0 && cover == Cover::Land {
            cover = Cover::Beach;
        }

        let clear = self.apply_trails(p, lx, lz, &mut ground, &mut cover, river, water, lake_level, near_sea);

        // Biome from the nearest cell, with the lookup position wobbled so borders
        // interlock and dither instead of following the grid.
        let wobble = |s: u32| {
            perlin2(seed ^ s, wx / 44.0, wz / 44.0) * 0.55
                + (unit(hash2(seed ^ s, lx, lz)) - 0.5) * 0.16
        };
        let bx = ((p.x / cs + wobble(0xb1)) as i32).clamp(0, c.size as i32 - 1) as usize;
        let bz = ((p.y / cs + wobble(0xb2)) as i32).clamp(0, c.size as i32 - 1) as usize;
        Column {
            ground,
            water,
            cover,
            biome: Biome::from_u8(c.biome[bz * c.size + bx]),
            temperature: bilinear(&c.temperature),
            river,
            clear,
        }
    }

    /// Levels the ground under landmarks and along trails. Returns the distance to the
    /// nearest such thing.
    ///
    /// Water is never disturbed: only dry columns change, the ground right beside a
    /// channel is left exactly as the river made it, and nothing can sink below a
    /// neighbouring water surface. That keeps water contained by construction.
    #[allow(clippy::too_many_arguments)]
    fn apply_trails(
        &self,
        p: Vec2,
        lx: i32,
        lz: i32,
        ground: &mut f32,
        cover: &mut Cover,
        river: f32,
        water: Option<i32>,
        lake_level: Option<i32>,
        near_sea: bool,
    ) -> f32 {
        if self.trail_points.is_empty() && self.pads.is_empty() {
            return f32::MAX;
        }
        let top = ground.floor() as i32;
        let dry = water.is_none_or(|k| top >= k - 1);
        let guard = if river == f32::MAX { 1.0 } else { smoothstep(2.0, 6.0, river) };
        let mut clear = f32::MAX;

        // Trail points lie one voxel apart, so the nearest point is close enough to the
        // nearest point on the curve.
        let mut any = f32::MAX;
        let mut paved: Option<(f32, &TrailPoint)> = None;
        for t in &self.trail_points {
            let d2 = (t.x - p.x).powi(2) + (t.z - p.y).powi(2);
            any = any.min(d2);
            if t.half_width > 0.0 && paved.is_none_or(|b| d2 < b.0) {
                paved = Some((d2, t));
            }
        }
        if any != f32::MAX {
            clear = clear.min(any.sqrt() - TRAIL_CLEARANCE);
        }

        if let Some((d2, t)) = paved
            && dry
        {
            let d = d2.sqrt();
            let rise = (t.y - *ground).abs();
            let shoulder = (rise * 1.3).clamp(2.0, 8.0);
            if d < t.half_width + shoulder {
                let w = (1.0 - smoothstep(t.half_width, t.half_width + shoulder, d)) * guard;
                *ground += (t.y - *ground) * w;
                let ragged = (unit(hash2(self.world_seed ^ 0x7a11, lx, lz)) - 0.5) * 0.9;
                if d < t.half_width + ragged && guard > 0.5 && matches!(*cover, Cover::Land | Cover::Beach | Cover::Bank) {
                    *cover = Cover::Trail;
                }
            }
        }

        // Landmarks last, so a trail passing through one cannot tilt its level ground.
        for pad in &self.pads {
            let d = (p - vec2(pad.x, pad.z)).length() - pad.radius;
            clear = clear.min(d);
            if dry && d < 6.0 {
                let w = (1.0 - smoothstep(0.0, 6.0, d)) * guard;
                *ground += (pad.y - *ground) * w;
            }
        }

        if dry {
            // Whatever was done above, dry ground stays dry and keeps holding its water in.
            if let Some(k) = lake_level
                && top >= k - 1
            {
                *ground = ground.max(k as f32 - 0.5);
            }
            if near_sea && top >= 0 {
                *ground = ground.max(0.5);
            }
        }
        clear
    }
}

/// Adds the river pieces that pass node `i`.
pub fn add_pieces(c: &Continent, i: usize, out: &mut Vec<Piece>) {
    let is_river = |j: usize| !c.ocean[j] && c.discharge[j] >= MIN_RIVER_Q;
    if !is_river(i) {
        return;
    }
    let r = c.rcv[i] as usize;
    let (pi, pr) = (node(c, i), node(c, r));
    let exit = (pi + pr) * 0.5;
    let (wi, li) = (Continent::river_width(c.discharge[i]), c.water_level[i]);
    // Sea level is 0; the last piece of a river runs down to it.
    let lr = if c.ocean[r] { 0.0 } else { c.water_level[r] };
    let exit_level = (li + lr) * 0.5;

    let size = c.size as isize;
    let mut fed = false;
    for dz in -1..=1isize {
        for dx in -1..=1isize {
            let d = (i as isize + dz * size + dx) as usize;
            if d != i && c.rcv[d] as usize == i && is_river(d) {
                fed = true;
                let pd = node(c, d);
                out.push(Piece::new(
                    (pd + pi) * 0.5,
                    pi,
                    exit,
                    (Continent::river_width(c.discharge[d]), wi),
                    ((c.water_level[d] + li) * 0.5, li, exit_level),
                ));
            }
        }
    }
    if !fed {
        // A source: the stream starts at the node and widens from nothing.
        out.push(Piece::new(
            pi,
            (pi + exit) * 0.5,
            exit,
            (0.0, wi),
            (li, (li + exit_level) * 0.5, exit_level),
        ));
    }
    if c.ocean[r] {
        // A mouth: carry the channel on into the sea, widening as it goes.
        out.push(Piece::new(
            exit,
            (exit + pr) * 0.5,
            pr,
            (wi, wi * 1.25),
            (exit_level, exit_level * 0.5, 0.0),
        ));
    }
}

/// Where the river network's node for cell `i` sits, in continent-local voxels.
///
/// The simulation routes water between cell centres, in only 8 directions, which
/// leaves long dead-straight reaches. Land nodes are therefore moved: a little at
/// random, and further by a smooth drift shared with their neighbours, which bends
/// straight runs into meanders. Larger rivers, which have broad valley floors to
/// wander across, drift furthest.
pub(crate) fn node(c: &Continent, i: usize) -> Vec2 {
    let (x, z) = (i % c.size, i / c.size);
    let mut p = vec2(x as f32 + 0.5, z as f32 + 0.5);
    if !c.ocean[i] && c.lake[i] == NO_LAKE {
        let seed = c.desc.seed;
        let h = hash2(seed ^ 0x90de, x as i32, z as i32);
        p += vec2(unit(h) - 0.5, unit(h.rotate_left(16)) - 0.5) * 0.5;
        let size = smoothstep(40.0, 4000.0, c.discharge[i]);
        let drift = 0.35 + 1.25 * size;
        let (nx, nz) = (x as f64 / 4.5, z as f64 / 4.5);
        p += vec2(perlin2(seed ^ 0x3ea1, nx, nz), perlin2(seed ^ 0x3ea2, nx, nz)) * drift;
    }
    p * c.cell_size()
}

/// Catmull-Rom interpolation of terrain height at a position in the cell-centre grid.
fn bicubic(c: &Continent, fx: f32, fz: f32) -> f32 {
    let (x1, z1) = (fx.floor() as isize, fz.floor() as isize);
    let (tx, tz) = (fx - x1 as f32, fz - z1 as f32);
    let weights = |t: f32| {
        let (t2, t3) = (t * t, t * t * t);
        [
            -0.5 * t3 + t2 - 0.5 * t,
            1.5 * t3 - 2.5 * t2 + 1.0,
            -1.5 * t3 + 2.0 * t2 + 0.5 * t,
            0.5 * t3 - 0.5 * t2,
        ]
    };
    let (wx, wz) = (weights(tx), weights(tz));
    let max = c.size as isize - 1;
    let mut sum = 0.0;
    for (j, wzj) in wz.iter().enumerate() {
        let z = (z1 - 1 + j as isize).clamp(0, max) as usize;
        let mut row = 0.0;
        for (i, wxi) in wx.iter().enumerate() {
            let x = (x1 - 1 + i as isize).clamp(0, max) as usize;
            row += c.height[z * c.size + x] * wxi;
        }
        sum += row * wzj;
    }
    sum
}
