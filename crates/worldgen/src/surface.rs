//! The voxel-resolution surface: turns a continent's coarse simulation grid into ground
//! and water heights for individual voxel columns.
//!
//! Terrain is interpolated from the grid and given fine detail. Rivers are not sampled
//! from the grid at all: they are smooth curves through the drainage network, carved
//! into the terrain, with water levels taken from the simulation's never-rising
//! profile. That is what keeps every stream continuous and flowing downhill at a
//! resolution 32 times finer than the simulation.

use glam::{Vec2, vec2};

mod cliff;
mod coast;
mod current;
mod ravine;
mod sea_cliff;

pub use cliff::strata_tilt;

use crate::{
    biome::Biome,
    continent::{Continent, NO_LAKE},
    macro_layout::MACRO_SIZE,
    noise::{fbm2, hash2, perlin2, unit},
    scale::{BLOCK, OLD_VOXEL, m, m64},
    trails::{Pad, TrailPoint},
};

/// Smallest discharge that forms a visible stream.
pub const MIN_RIVER_Q: f32 = 24.0;
/// Line segments each river curve is flattened into for distance queries.
const SEGS: usize = 6;
/// Narrowest channel half-width, so the thinnest brooks are still unbroken water.
const MIN_HALF_WIDTH: f32 = m(0.6);
/// How far, in cells of the 1024 grid, the largest rivers swing sideways to meander.
const SINUOSITY_CELLS: f32 = 4.0;
/// Taken from the distance to a trail's centreline to give the distance to its edge.
const TRAIL_CLEARANCE: f32 = m(1.2);
/// Distance from a river channel within which cliffs, waterfalls and fault ravines
/// take the river into account, so none of them can dig a valley floor out beside it.
const RIVER_GUARD: f32 = m(80.0);
/// Distance from a river channel within which a cliff's foot must stand clear above
/// the water (fading out over the second half).
const CLIFF_GUARD: f32 = m(48.0);

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
    /// Sheer rock face or ledge.
    Cliff,
    /// Loose stone at the foot of a cliff.
    Scree,
    // Coasts (see `coast`), added after the others so existing variants keep their order.
    /// A pebble and cobble beach on an exposed coast.
    Shingle,
    /// Big rounded stones on the most exposed shores and under sea cliffs.
    Boulders,
    /// The wave-cut rock platform at the foot of a sea cliff, partly awash.
    Shelf,
    /// Wind-blown sand heaped up behind a wide beach.
    Dune,
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
    /// Water level at the start, the middle and the end, in simulation units.
    l0: f32,
    l1: f32,
    l2: f32,
    /// Water level just upstream of the start: higher than `l0` where the piece starts
    /// at the foot of a waterfall.
    lip: f32,
    /// Network nodes the piece starts beside, passes and ends beside.
    nodes: [usize; 3],
    /// Depth of the gorge the river runs in at the start, middle and end, in voxels
    /// (0 for none). Filled in by the sampler, which knows the world seed.
    gorge: [f32; 3],
    /// Length of the curve in voxels.
    length: f32,
    /// Bounding box grown by the farthest the piece can influence terrain.
    lo: Vec2,
    hi: Vec2,
}

impl Piece {
    fn new(p0: Vec2, p1: Vec2, p2: Vec2, widths: (f32, f32), levels: (f32, f32, f32), lip: f32, nodes: [usize; 3]) -> Self {
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
            lip: lip.max(levels.0),
            nodes,
            gorge: [0.0; 3],
            lo: lo - reach,
            hi: hi + reach,
        }
    }

    /// Distance from `p` to the curve, the curve parameter (0..1) of the nearest point,
    /// and that point.
    fn nearest(&self, p: Vec2) -> (f32, f32, Vec2) {
        let (mut best, mut best_t, mut best_q) = (f32::MAX, 0.0, self.pts[0]);
        for k in 0..SEGS {
            let (a, b) = (self.pts[k], self.pts[k + 1]);
            let ab = b - a;
            let s = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
            let q = a + ab * s;
            let d = (p - q).length_squared();
            if d < best {
                (best, best_t, best_q) = (d, (k as f32 + s) / SEGS as f32, q);
            }
        }
        (best.sqrt(), best_t, best_q)
    }

    /// Whether `p` is within `margin` of the area the piece can touch.
    fn near(&self, p: Vec2, margin: f32) -> bool {
        p.x >= self.lo.x - margin && p.x <= self.hi.x + margin && p.y >= self.lo.y - margin && p.y <= self.hi.y + margin
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

    /// Simulated water level a distance `back` (voxels) upstream of curve parameter `t`.
    /// Above the start of the piece that is the level at the top of any waterfall there.
    fn level_above(&self, t: f32, back: f32) -> f32 {
        let u = t - back / self.length;
        if u < 0.0 { self.lip } else { self.level(u) }
    }

    /// Depth of the gorge at curve parameter `t`, in voxels.
    fn gorge_depth(&self, t: f32) -> f32 {
        let [a, b, c] = self.gorge;
        if t < 0.5 { a + (b - a) * (t * 2.0) } else { b + (c - b) * (t * 2.0 - 1.0) }
    }

    /// Height of the waterfall at the start of the piece, in simulation units.
    fn plunge(&self) -> f32 {
        self.lip - self.l0
    }
}

/// How far from the channel edge a river's banks are reshaped.
fn bank_width(half_width: f32) -> f32 {
    m(2.4) + 2.0 * half_width
}

/// Farthest from its centreline a river touches the terrain.
fn reach(half_width: f32) -> f32 {
    let hw = half_width.max(MIN_HALF_WIDTH);
    hw + bank_width(hw) + m(0.8)
}

/// Quantises a river's water level into flat pools separated by drops.
///
/// Only some integer levels are allowed, chosen by hash; water sits at the highest
/// allowed level at or below its simulated level. Because this is a non-decreasing
/// function of the level alone, the result still never rises downstream. On a gentle
/// river the allowed levels are far apart along its length, giving long still pools
/// and the odd weir; on a steep one they come quickly, giving cascades. Missing levels
/// become taller falls, more often at altitude. Takes simulation units and returns
/// voxels, so pools keep the same height however fine the voxels are.
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
    k.max(0) * BLOCK
}

/// Samples the surface for the voxel columns of one chunk (or any small area).
pub struct Sampler<'a> {
    c: &'a Continent,
    world_seed: u32,
    pieces: Vec<Piece>,
    /// Waterfalls whose cliff can reach the area.
    falls: Vec<Fall>,
    /// River deltas that can reach the area.
    deltas: Vec<coast::Delta>,
    /// River mouths whose estuaries can reach the area, and the shores of its grid cells.
    mouths: Vec<coast::Mouth>,
    shores: coast::Shores,
    trail_points: Vec<TrailPoint>,
    pads: Vec<Pad>,
}

impl<'a> Sampler<'a> {
    /// Prepares to sample columns in the square starting at continent-local voxel
    /// `(lx0, lz0)` and `span` voxels wide.
    pub fn new(c: &'a Continent, world_seed: u32, lx0: i32, lz0: i32, span: i32) -> Self {
        let cs = c.cell_size();
        let widest = Continent::river_width(c.max_discharge.max(MIN_RIVER_Q)) * OLD_VOXEL;
        // A piece lies within a cell of its node, so gather nodes that close plus the
        // farthest a river can reach, or be looked for by the cliffs and ravines beside it.
        let margin = reach(widest * 0.5) + cs * (3.0 + SINUOSITY_CELLS * 1.5) + RIVER_GUARD;
        let cell = |v: f32| ((v / cs).floor() as i32).clamp(1, c.size as i32 - 2) as usize;
        let (x0, x1) = (cell(lx0 as f32 - margin), cell((lx0 + span) as f32 + margin));
        let (z0, z1) = (cell(lz0 as f32 - margin), cell((lz0 + span) as f32 + margin));

        let mut falls = Vec::new();
        let mut deltas = Vec::new();
        let mut pieces = Vec::new();
        for z in z0..=z1 {
            for x in x0..=x1 {
                add_pieces(c, z * c.size + x, &mut pieces);
                if let Some(f) = Fall::at(c, world_seed, z * c.size + x) {
                    falls.push(f);
                }
                if let Some(d) = coast::Delta::at(c, world_seed, z * c.size + x) {
                    deltas.push(d);
                }
            }
        }
        let (lo, hi) = (
            vec2(lx0 as f32, lz0 as f32),
            vec2((lx0 + span) as f32, (lz0 + span) as f32),
        );
        // Gorges: their walls and rims reach further from the river than its banks.
        let mut depths = std::collections::HashMap::new();
        for p in &mut pieces {
            let mut depth = |j: usize| *depths.entry(j).or_insert_with(|| ravine::gorge_depth(c, world_seed, j));
            let (a, m, b) = (depth(p.nodes[0]), depth(p.nodes[1]), depth(p.nodes[2]));
            // The ends are shared with the neighbouring pieces, so depth is continuous.
            p.gorge = [(a + m) * 0.5, m, (m + b) * 0.5];
            let deepest = p.gorge.iter().copied().fold(0.0, f32::max);
            if deepest > 0.0 {
                let r = Vec2::splat(ravine::gorge_reach(deepest));
                p.lo -= r;
                p.hi += r;
            }
        }
        // Keep the pieces whose banks reach the area, and those far enough off for the
        // cliffs and ravines in it to need to know where the water is.
        let (glo, ghi) = (lo - RIVER_GUARD, hi + RIVER_GUARD);
        pieces.retain(|p| p.lo.x <= ghi.x && p.hi.x >= glo.x && p.lo.y <= ghi.y && p.hi.y >= glo.y);
        falls.retain(|f| {
            let r = Vec2::splat(f.radius);
            let (a, b) = (f.pos - r, f.pos + r);
            a.x <= hi.x && b.x >= lo.x && a.y <= hi.y && b.y >= lo.y
        });
        let (trail_points, pads) = c.trails.gather(lo, hi);
        Self {
            c,
            world_seed,
            pieces,
            falls,
            deltas,
            mouths: coast::mouths(c, lo, hi),
            shores: coast::Shores::new(c, lo, hi),
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

        let (mut ground, slope) = natural_ground(c, seed, fx, fz, wx, wz);
        let mut water: Option<i32> = None;
        let mut cover = Cover::Land;
        // The nearest river within RIVER_GUARD, looked up only by the rare columns a
        // cliff or ravine touches.
        let beside_cell = std::cell::OnceCell::new();
        let beside = || *beside_cell.get_or_init(|| self.river_beside(p));
        if lake == NO_LAKE && !near_sea {
            let slope = slope * OLD_VOXEL / cs;
            let natural = ground;
            // A large fault step stands as a sheer face over its talus, in place of the
            // grid's ramp. Beside a river it eases back to the ramp the river was routed
            // over, so the step neither dams the channel nor drops the bank into it.
            let scarp = c.scarp(lx, lz).and_then(|s| {
                let guard = beside().map_or(1.0, |(sd, _)| smoothstep(CLIFF_GUARD * 0.5, CLIFF_GUARD, sd));
                (guard > 0.0).then_some((s, guard))
            });
            // A cliff's face stands on the valley side, its foot clear above the river.
            let mut part = match scarp {
                Some((s, guard)) => {
                    ground += s.lift * guard;
                    if s.face && guard > 0.5 {
                        cliff::Part::Face(1.0)
                    } else if unit(hash2(seed ^ 0x5ca3, lx >> 1, lz >> 1)) < s.scree * guard {
                        cliff::Part::Foot(1.0)
                    } else {
                        cliff::Part::None
                    }
                }
                None => cliff::apply(seed, slope, wx, wz, &mut ground, |base| match beside() {
                    Some((sd, level)) => {
                        let far = smoothstep(CLIFF_GUARD * 0.5, CLIFF_GUARD, sd);
                        far.max(smoothstep(level + m(2.0), level + m(6.0), base))
                    }
                    None => 1.0,
                }),
            };
            for f in &self.falls {
                let d = (p - f.pos).length();
                if d < f.radius {
                    let fall = f.cut(d, slope, wx, wz, &mut ground);
                    if part == cliff::Part::None {
                        part = fall;
                    }
                }
            }
            // Whatever the cliffs and falls did, the ground beside a river is not cut
            // below its water: the valley floor stays a floor. Further out the limit
            // falls away at 45 degrees, so it never makes a step of its own.
            if ground < natural
                && let Some((sd, level)) = beside()
            {
                let floor = level + m(1.0) - (sd - CLIFF_GUARD * 0.5).max(0.0);
                if ground < floor {
                    ground = floor.min(natural);
                    if !matches!(part, cliff::Part::Face(_)) {
                        part = cliff::Part::None;
                    }
                }
            }
            match part {
                cliff::Part::Face(_) => cover = Cover::Cliff,
                cliff::Part::Foot(p) if unit(hash2(seed ^ 0x5c4e, lx >> 1, lz >> 1)) < p => cover = Cover::Scree,
                _ => {}
            }
        }
        // Coasts: sea cliffs, shingle, boulders, dunes and estuary flats.
        let mut shore = (near_sea && lake == NO_LAKE).then(|| self.shores.at(c, &corners, tx, tz, p, &self.mouths));
        if let Some(s) = &mut shore
            && let Some(rock) = s.shape(seed, slope * OLD_VOXEL / cs, wx, wz, &mut ground)
        {
            cover = rock;
        }
        let mut river = f32::MAX;

        // Standing water this column could be under: it must not be given river banks.
        let lake_level = (lake != NO_LAKE).then(|| pool_level(c.desc.seed, lake));
        let submerged = (near_sea && ground < 0.0)
            || lake_level.is_some_and(|k| ground < k as f32 - 1.0);

        // The river whose channel edge is nearest.
        // (signed distance to the channel edge, half-width, water level there, water
        // level a couple of voxels upstream, extra depth of a plunge pool, gorge depth,
        // how far a mouth has shoaled towards the sea floor, nearest point on the
        // centreline)
        let mut nearest: Option<(f32, f32, f32, f32, f32, f32, f32, Vec2)> = None;
        for piece in &self.pieces {
            if !piece.near(p, 0.0) {
                continue;
            }
            let (dist, t, q) = piece.nearest(p);
            let hw = piece.half_width(t);
            let sd = dist - hw;
            if nearest.is_none_or(|n| sd < n.0) {
                // Under a waterfall the bed is scoured into a pool.
                let plunge = piece.plunge() * OLD_VOXEL;
                let pool = if plunge > 0.0 {
                    (plunge * 0.3).min(m(3.0)) * (1.0 - smoothstep(0.0, m(3.0) + hw * 2.0, t * piece.length))
                } else {
                    0.0
                };
                // Out at sea a river mouth shoals to the sea floor rather than ending in a
                // deep rounded hole.
                let seaward = if piece.nodes[1] == piece.nodes[2] { smoothstep(0.2, 0.95, t) } else { 0.0 };
                nearest = Some((sd, hw, piece.level(t), piece.level_above(t, m(1.6)), pool, piece.gorge_depth(t), seaward, q));
            }
        }
        if let Some((sd, hw, level, upstream_level, pool, mut gorge, seaward, centre)) = nearest {
            if gorge > 0.0 && sd >= 0.0 {
                // A gorge is a river cut down into high ground, so its walls stand no
                // higher than the land at the edge of its reach. Where the valley is
                // broad and low the gorge dies away instead of raising embankments
                // either side of the water.
                let away = (p - centre).normalize_or_zero();
                let q = p + away * (ravine::gorge_reach(gorge) - sd).max(0.0);
                let (gx, gz) = ((q.x / cs - 0.5).clamp(0.0, max), (q.y / cs - 0.5).clamp(0.0, max));
                let (outer, _) = natural_ground(c, seed, gx, gz, wx + (q.x - p.x) as f64, wz + (q.y - p.y) as f64);
                gorge = gorge.min(outer - level * OLD_VOXEL).max(0.0);
            }
            river = sd;
            let k = pool_level(c.desc.seed, level);
            if sd < 0.0 {
                // In the channel: a parabolic bed, deepest mid-stream.
                let depth = (m(0.48) + 0.44 * hw).clamp(m(0.8), m(5.6)) + pool;
                let across = 1.0 + sd / hw; // 0 at the centre, 1 at the edge
                let mut bed = k as f32 - 1.01 - depth * (1.0 - across * across);
                bed += (ground.min(k as f32 - 1.6) - bed).max(0.0) * seaward;
                cover = Cover::RiverBed;
                if k == 0 {
                    // A tidal reach: a sandy bed, with bars in the wider channels.
                    cover = Cover::SeaFloor;
                    if hw > m(2.5) && !submerged {
                        bed = coast::sand_bar(seed, wx, wz, across, bed);
                        if bed >= -1.0 {
                            cover = Cover::Beach;
                        }
                    }
                }
                ground = if submerged { ground.min(bed) } else { bed };
                water = Some(k);
            } else if !submerged && (sd < bank_width(hw) || (gorge > 0.0 && sd < ravine::gorge_reach(gorge))) {
                // Banks meet the water flush with its surface, then rise (or fall) to
                // the surrounding terrain. Where the land is lower than the river this
                // builds the levee that keeps the water in.
                // Beside a drop the bank holds the level of the pool above it for a
                // couple of voxels, so the upper pool cannot spill around the lip.
                let bank = pool_level(c.desc.seed, upstream_level) as f32 - 0.5;
                let natural = ground;
                ground = bank + (ground - bank) * smoothstep(0.0, bank_width(hw), sd);
                if sd < m(1.28) && seaward < 0.3 {
                    cover = Cover::Bank;
                }
                if gorge > 0.0 {
                    // In a gorge the banks are a strip of gravel and boulders under
                    // sheer walls.
                    let (g, part) = ravine::gorge_wall(seed, wx, wz, sd, hw, bank, level * OLD_VOXEL, gorge, natural);
                    let w = smoothstep(m(1.5), m(5.0), gorge);
                    ground += (g - ground) * w;
                    if w > 0.5 {
                        match part {
                            cliff::Part::Face(_) => cover = Cover::Cliff,
                            cliff::Part::Foot(_) => cover = Cover::Scree,
                            cliff::Part::None if cover == Cover::Cliff || cover == Cover::Scree => cover = Cover::Land,
                            cliff::Part::None => {}
                        }
                    }
                }
            }
        }

        // Deltas build out into the sea in front of big slow rivers.
        if near_sea && cover != Cover::RiverBed && cover != Cover::SeaFloor {
            for d in &self.deltas {
                if let Some(flat) = d.shape(p, wx, wz, &mut ground) {
                    cover = flat;
                }
            }
        }

        // Dry fault ravines, kept well clear of water. `river` is only known within a
        // few metres of a channel, so the distance comes from the wider search.
        if lake == NO_LAKE && !near_sea && matches!(cover, Cover::Land | Cover::Cliff | Cover::Scree) {
            let fade = || beside().map_or(1.0, |(sd, _)| smoothstep(RIVER_GUARD * 0.5, RIVER_GUARD, sd));
            match ravine::dry(seed, wx, wz, fade, &mut ground) {
                cliff::Part::Face(_) => cover = Cover::Cliff,
                cliff::Part::Foot(_) => cover = Cover::Scree,
                cliff::Part::None => {}
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
        } else if near_sea && cover == Cover::Land {
            match &shore {
                Some(s) => cover = s.cover(ground).unwrap_or(cover),
                None if ground < m(2.4) => cover = Cover::Beach,
                None => {}
            }
        }

        let clear = self.apply_trails(p, lx, lz, &mut ground, &mut cover, river, water, lake_level, near_sea);

        // Biome from the nearest cell, with the lookup position wobbled so borders
        // interlock and dither instead of following the grid.
        let wobble = |s: u32| {
            perlin2(seed ^ s, wx / m64(35.2), wz / m64(35.2)) * 0.55
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

    /// The river channel whose edge is nearest `p`, if one is within `RIVER_GUARD`:
    /// the distance to its edge, and its smooth water level there in voxels.
    fn river_beside(&self, p: Vec2) -> Option<(f32, f32)> {
        let mut best: Option<(f32, f32)> = None;
        for piece in &self.pieces {
            if !piece.near(p, RIVER_GUARD) {
                continue;
            }
            let (dist, t, _) = piece.nearest(p);
            let sd = dist - piece.half_width(t);
            if sd < RIVER_GUARD && best.is_none_or(|b| sd < b.0) {
                best = Some((sd, piece.level(t) * OLD_VOXEL));
            }
        }
        best
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
        let guard = if river == f32::MAX { 1.0 } else { smoothstep(m(1.6), m(4.8), river) };
        let mut clear = f32::MAX;

        // Trail points lie 0.8 m apart, so the nearest point is close enough to the
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
            let shoulder = (rise * 1.3).clamp(m(1.6), m(6.4));
            if d < t.half_width + shoulder {
                let w = (1.0 - smoothstep(t.half_width, t.half_width + shoulder, d)) * guard;
                *ground += (t.y - *ground) * w;
                let ragged = (unit(hash2(self.world_seed ^ 0x7a11, lx, lz)) - 0.5) * m(0.72);
                if d < t.half_width + ragged && guard > 0.5 && matches!(*cover, Cover::Land | Cover::Beach | Cover::Bank | Cover::Scree | Cover::Cliff) {
                    *cover = Cover::Trail;
                }
            }
        }

        // Landmarks last, so a trail passing through one cannot tilt its level ground.
        for pad in &self.pads {
            let d = (p - vec2(pad.x, pad.z)).length() - pad.radius;
            clear = clear.min(d);
            if dry && d < m(4.8) {
                let w = (1.0 - smoothstep(0.0, m(4.8), d)) * guard;
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
    let width = |j: usize| Continent::river_width(c.discharge[j]) * OLD_VOXEL * funnel(c, j, c.discharge[j]);
    let (wi, li) = (width(i), river_level(c, i));
    // Sea level is 0; the last piece of a river runs down to it.
    let lr = river_level(c, r);
    // Above a waterfall the pool holds its level right to the lip.
    let exit_level = if fall_drop(c, i) > 0.0 { li } else { (li + lr) * 0.5 };

    let size = c.size as isize;
    let mut fed = false;
    for dz in -1..=1isize {
        for dx in -1..=1isize {
            let d = (i as isize + dz * size + dx) as usize;
            if d != i && c.rcv[d] as usize == i && is_river(d) {
                fed = true;
                let pd = node(c, d);
                // Below a waterfall the pool starts at the foot of the fall.
                let ld = river_level(c, d);
                let (start, lip) = if fall_drop(c, d) > 0.0 { (li, ld) } else { ((ld + li) * 0.5, (ld + li) * 0.5) };
                out.push(Piece::new((pd + pi) * 0.5, pi, exit, (width(d), wi), (start, li, exit_level), lip, [d, i, r]));
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
            li,
            [i, i, r],
        ));
    }
    if c.ocean[r] {
        // A mouth: carry the channel on into the sea, opening out as it goes.
        let open = Continent::river_width(c.discharge[i]) * OLD_VOXEL * funnel(c, r, c.discharge[i]);
        out.push(Piece::new(
            exit,
            (exit + pr) * 0.5,
            pr,
            (wi, open * 1.3),
            (exit_level, exit_level * 0.5, 0.0),
            exit_level,
            [i, r, r],
        ));
    }
}

/// Simulated level at which the sea's rise and fall reaches up a river: below it the
/// river is tidal, and its water stands at sea level.
const TIDAL: f32 = 2.0;

/// The water level a river keeps at node `j`, in simulation units. Near the coast the
/// simulated level eases down to sea level, so a river meets the sea flush rather than
/// over a lip. The mapping only ever lowers levels and never reverses their order, so
/// rivers still never flow uphill.
pub(crate) fn river_level(c: &Continent, j: usize) -> f32 {
    if c.ocean[j] {
        return 0.0;
    }
    let l = c.water_level[j];
    if l < TIDAL {
        0.0
    } else if l < 2.0 * TIDAL {
        (l - TIDAL) * 2.0
    } else {
        l
    }
}

/// How much wider than usual a river carrying `discharge` is at node `j`: big rivers
/// open into a funnel-shaped estuary over the last few metres of their fall.
fn funnel(c: &Continent, j: usize, discharge: f32) -> f32 {
    let low = 1.0 - smoothstep(0.0, 8.0, river_level(c, j));
    1.0 + 1.8 * low * smoothstep(150.0, 1500.0, discharge)
}

/// Smallest drop, in simulation units, that a river can make as one waterfall.
const FALL_MIN: f32 = 7.0;
/// Chance that a drop that large is a waterfall rather than a run of cascades.
const FALL_CHANCE: f32 = 0.4;

/// How far the water drops, in simulation units, if the river leaving node `i` goes
/// over a waterfall on its way to the next node, or 0.
///
/// Where the simulated river falls steeply between two nodes it normally runs down a
/// series of pools. Some of those reaches instead hold the upper pool's level to the
/// midpoint between the nodes and drop the whole height there at once.
pub(crate) fn fall_drop(c: &Continent, i: usize) -> f32 {
    let candidate = |i: usize| {
        let r = c.rcv[i] as usize;
        if r == i || c.ocean[i] || c.ocean[r] || c.lake[i] != NO_LAKE || c.lake[r] != NO_LAKE || c.discharge[i] < MIN_RIVER_Q {
            return 0.0;
        }
        let drop = c.water_level[i] - c.water_level[r];
        let (x, z) = ((i % c.size) as i32, (i / c.size) as i32);
        if drop < FALL_MIN || unit(hash2(c.desc.seed ^ 0xfa11, x, z)) > FALL_CHANCE {
            return 0.0;
        }
        drop
    };
    // Never two in a row: the river needs a pool between them.
    if candidate(c.rcv[i] as usize) > 0.0 {
        return 0.0;
    }
    candidate(i)
}

/// The cliff a waterfall pours over: a face across the valley at the lip, which
/// follows the valley sides downstream as a short gorge and fades out within `radius`.
struct Fall {
    pos: Vec2,
    radius: f32,
    /// Natural ground height at the lip, in voxels.
    level: f32,
    /// Height of the drop, in voxels.
    height: f32,
    salt: u32,
    seed: u32,
}

impl Fall {
    fn at(c: &Continent, world_seed: u32, i: usize) -> Option<Fall> {
        if fall_drop(c, i) <= 0.0 {
            return None;
        }
        let r = c.rcv[i] as usize;
        let top = pool_level(c.desc.seed, river_level(c, i));
        let bottom = pool_level(c.desc.seed, river_level(c, r));
        let height = (top - bottom) as f32;
        if height < m(3.0) {
            return None;
        }
        let pos = (node(c, i) + node(c, r)) * 0.5;
        let cs = c.cell_size();
        let (level, _) = bicubic_slope(c, pos.x / cs - 0.5, pos.y / cs - 0.5);
        Some(Fall {
            pos,
            radius: m(24.0) + height * 1.5,
            level: level * OLD_VOXEL,
            height,
            salt: hash2(world_seed ^ 0xfa12, i as i32, 3),
            seed: world_seed,
        })
    }

    /// Cuts the face into a column `dist` voxels from the lip.
    fn cut(&self, dist: f32, slope: f32, wx: f64, wz: f64, ground: &mut f32) -> cliff::Part {
        // Only on sloping ground: on the flat the lift would raise a round plateau.
        let strength = (1.0 - smoothstep(self.radius * 0.5, self.radius, dist)) * smoothstep(0.2, 0.45, slope);
        let half = self.height * 0.5;
        let face = cliff::Face {
            x: *ground - self.level,
            lo: -half * 1.5,
            hi: half * 1.6,
            height: self.height,
            ledge: 0.0,
            split: 0.5,
            salt: self.salt,
            seed: self.seed,
        };
        face.cut(strength, slope.max(0.35), wx, wz, ground)
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
        p += sinuosity(c, i, size);
    }
    p * c.cell_size()
}

/// A sideways swing, in cells, that turns the long straight runs the router leaves on
/// big rivers into meanders.
///
/// Routing on a grid follows one slope direction for as long as a valley does, so a
/// large river can run dead straight for hundreds of metres. This pushes each node
/// across the local direction of flow by low-frequency noise sampled at its position.
/// Nodes along one reach sample neighbouring points, so the swing is smooth along the
/// river, and a node's position stays a function of its own cell alone, so tributaries
/// and the river they join still meet. Water levels belong to the nodes and are
/// untouched, which keeps every river flowing downhill. `size` is 0 for brooks and 1
/// for the largest rivers.
fn sinuosity(c: &Continent, i: usize, size: f32) -> Vec2 {
    if size <= 0.0 {
        return Vec2::ZERO;
    }
    let (x, z) = (i % c.size, i / c.size);
    let cell = |j: usize| vec2((j % c.size) as f32, (j / c.size) as f32);
    // Direction of flow over the next couple of cells, so it turns gently.
    let r = c.rcv[i] as usize;
    let r2 = c.rcv[r] as usize;
    let ahead = cell(r2) - cell(i);
    let Some(dir) = ahead.try_normalize() else { return Vec2::ZERO };
    let side = vec2(-dir.y, dir.x);
    // Wavelength about 7 cells at the 1024 grid, scaled so it is the same distance
    // on the ground at any grid size.
    let k = 1024.0 / c.size as f64 / 7.0;
    let wave = perlin2(c.desc.seed ^ 0x3ea3, x as f64 * k, z as f64 * k)
        + 0.5 * perlin2(c.desc.seed ^ 0x3ea4, x as f64 * k * 2.3, z as f64 * k * 2.3);
    side * wave * SINUOSITY_CELLS * size * (c.size as f32 / 1024.0)
}


/// Ground height in voxels before rivers, cliffs or trails touch it, at position
/// `(fx, fz)` in the cell-centre grid and world voxel `(wx, wz)`; and the simulated
/// slope there in height units per cell.
fn natural_ground(c: &Continent, seed: u32, fx: f32, fz: f32, wx: f64, wz: f64) -> (f32, f32) {
    let (base, slope) = bicubic_slope(c, fx, fz);
    let base = base * OLD_VOXEL;
    // Fine relief the simulation grid cannot hold. Faded out at the waterline so
    // shores stay clean.
    let detail = fbm2(seed ^ 0xd1, wx / m64(44.8), wz / m64(44.8), 3, 2.0, 0.5) * m(2.08)
        + perlin2(seed ^ 0xd2, wx / m64(13.6), wz / m64(13.6)) * m(0.8)
        + perlin2(seed ^ 0xd3, wx / m64(5.6), wz / m64(5.6)) * m(0.44);
    (base + detail * ((base - m(0.4)).abs() / m(3.2)).clamp(0.12, 1.0), slope)
}

/// Catmull-Rom interpolation of terrain height at a position in the cell-centre grid,
/// with the steepness of that smooth surface (height units per cell). The slope comes
/// from the same 16 samples, so it costs almost nothing extra.
fn bicubic_slope(c: &Continent, fx: f32, fz: f32) -> (f32, f32) {
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
    let derivatives = |t: f32| {
        let t2 = t * t;
        [
            -1.5 * t2 + 2.0 * t - 0.5,
            4.5 * t2 - 5.0 * t,
            -4.5 * t2 + 4.0 * t + 0.5,
            1.5 * t2 - t,
        ]
    };
    let (wx, wz) = (weights(tx), weights(tz));
    let dx = derivatives(tx);
    let dz = derivatives(tz);
    let max = c.size as isize - 1;
    let (mut sum, mut gx, mut gz) = (0.0, 0.0, 0.0);
    for j in 0..4 {
        let z = (z1 - 1 + j as isize).clamp(0, max) as usize;
        let (mut row, mut row_dx) = (0.0, 0.0);
        for i in 0..4 {
            let x = (x1 - 1 + i as isize).clamp(0, max) as usize;
            let h = c.height[z * c.size + x];
            row += h * wx[i];
            row_dx += h * dx[i];
        }
        sum += row * wz[j];
        gx += row_dx * wz[j];
        gz += row * dz[j];
    }
    (sum, gx.hypot(gz))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{continent::{SimConfig, generate}, macro_layout::cell_desc};

    /// Longest dead-straight stretch (in voxels) of any large river: how far the node
    /// chain runs while staying within `tol` voxels of one straight line.
    fn longest_straight(c: &Continent, min_q: f32, tol: f32) -> f32 {
        let mut best = 0.0f32;
        for i in 0..c.size * c.size {
            if c.ocean[i] || c.lake[i] != NO_LAKE || c.discharge[i] < min_q {
                continue;
            }
            // Only start where no larger river feeds in from upstream along the chain.
            let mut chain = vec![node(c, i)];
            let mut j = i;
            for _ in 0..400 {
                let r = c.rcv[j] as usize;
                if r == j || c.ocean[r] || c.lake[r] != NO_LAKE {
                    break;
                }
                chain.push(node(c, r));
                j = r;
            }
            let mut a = 0;
            for b in 2..chain.len() {
                // Shrink from the start until every point is within tol of chord a..b.
                loop {
                    let (pa, pb) = (chain[a], chain[b]);
                    let d = (pb - pa).normalize_or_zero();
                    let ok = chain[a..=b].iter().all(|q| {
                        let v = *q - pa;
                        (v.x * d.y - v.y * d.x).abs() <= tol
                    });
                    if ok || a + 2 > b {
                        break;
                    }
                    a += 1;
                }
                best = best.max((chain[b] - chain[a]).length());
            }
        }
        best
    }

    #[test]
    fn rivers_do_not_run_straight_for_long() {
        let c = generate(cell_desc(7, 0, 0), SimConfig { size: 1024 });
        let straight = longest_straight(&c, 4000.0, m(5.0));
        eprintln!("longest straight reach: {straight} voxels = {} m", straight / m(1.0));
        assert!(straight < m(190.0), "straight reach {straight}");
    }

    #[test]
    fn cliffs_stand_on_some_steep_ground_with_scree_and_never_on_gentle_ground() {
        let c = generate(cell_desc(7, 0, 0), SimConfig { size: 1024 });
        let cs = c.cell_size();
        let slope = |i: usize| {
            let gx = (c.height[i + 1] - c.height[i - 1]) * OLD_VOXEL / (2.0 * cs);
            let gz = (c.height[i + c.size] - c.height[i - c.size]) * OLD_VOXEL / (2.0 * cs);
            gx.hypot(gz)
        };
        let mut sites: Vec<(f32, usize)> = Vec::new();
        for z in (8..c.size - 8).step_by(6) {
            for x in (8..c.size - 8).step_by(6) {
                let i = z * c.size + x;
                if !c.ocean[i] && c.height[i] * OLD_VOXEL > m(10.0) {
                    sites.push((slope(i), i));
                }
            }
        }
        sites.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        let count = |i: usize| {
            let (lx, lz) = ((i % c.size) as f32 * cs, (i / c.size) as f32 * cs);
            let (lx, lz) = (lx as i32, lz as i32);
            let s = Sampler::new(&c, 7, lx - 32, lz - 32, 64);
            let (mut cliff, mut scree) = (0, 0);
            for dz in -32..32 {
                for dx in -32..32 {
                    let col = s.column(lx + dx, lz + dz);
                    match col.cover {
                        Cover::Cliff => {
                            assert!(col.ground > m(5.0), "cliff down at the shore");
                            cliff += 1;
                        }
                        Cover::Scree => scree += 1,
                        _ => {}
                    }
                }
            }
            (cliff, scree)
        };
        let steep: Vec<(i32, i32)> = sites.iter().take(300).map(|s| count(s.1)).collect();
        let cliffed = steep.iter().filter(|s| s.0 > 0).count();
        let scree: i32 = steep.iter().map(|s| s.1).sum();
        eprintln!("{cliffed} of {} steep sites have a cliff; {scree} scree columns", steep.len());
        // Cliffs are features: some of the steepest ground has them, most does not.
        assert!(cliffed >= 15, "too few cliffs: {cliffed}");
        assert!(cliffed * 2 < steep.len(), "cliffs cover most steep ground: {cliffed}");
        assert!(scree > 100, "no scree at the foot of cliffs: {scree}");
        let gentle: Vec<(f32, (i32, i32))> = sites.iter().rev().take(300).map(|s| (s.0, count(s.1))).filter(|g| g.1 != (0, 0)).collect();
        // Waterfalls and fault ravines cut through gentle ground, but only here and there.
        assert!(gentle.len() <= 15, "cliffs or scree on gentle ground: {gentle:?}");
    }

    /// Prints how much of the land and of the steep land is cliff, and where the most
    /// cliff is: `cargo test --release -p worldgen cliff_survey -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn cliff_survey() {
        let c = generate(cell_desc(7, 0, 0), SimConfig { size: 1024 });
        const STRIDE: i32 = 16;
        const TILE: i32 = 1024;
        let (mut land, mut steep, mut cliff, mut steep_cliff, mut scree) = (0, 0, 0, 0, 0);
        let mut tiles: Vec<(i32, i32, i32)> = Vec::new();
        let mut faces: Vec<(f32, i32, i32, i32, i32)> = Vec::new();
        let span = MACRO_SIZE;
        for tz in (0..span).step_by(TILE as usize) {
            for tx in (0..span).step_by(TILE as usize) {
                let s = Sampler::new(&c, 7, tx, tz, TILE);
                let mut here = 0;
                for z in (tz..tz + TILE).step_by(STRIDE as usize) {
                    for x in (tx..tx + TILE).step_by(STRIDE as usize) {
                        let i = (z / 64) as usize * c.size + (x / 64) as usize;
                        if c.ocean[i] {
                            continue;
                        }
                        let col = s.column(x, z);
                        let (_, sl) = bicubic_slope(&c, x as f32 / 64.0 - 0.5, z as f32 / 64.0 - 0.5);
                        let is_steep = sl * OLD_VOXEL / 64.0 > 0.55 && col.ground > m(8.0);
                        land += 1;
                        steep += is_steep as i32;
                        match col.cover {
                            Cover::Cliff => {
                                cliff += 1;
                                steep_cliff += is_steep as i32;
                                here += 1;
                                // The drop in front of this face, and which way it looks.
                                let mut best = (0.0f32, 0, 0);
                                for (dx, dz) in [(8, 0), (-8, 0), (0, 8), (0, -8), (6, 6), (-6, 6), (6, -6), (-6, -6)] {
                                    let d = col.ground - s.column(x + dx, z + dz).ground;
                                    if d > best.0 {
                                        best = (d, dx, dz);
                                    }
                                }
                                faces.push((best.0, x, z, best.1, best.2));
                            }
                            Cover::Scree => scree += 1,
                            _ => {}
                        }
                    }
                }
                if here > 0 {
                    tiles.push((here, tx + TILE / 2, tz + TILE / 2));
                }
            }
        }
        eprintln!(
            "land {land}, steep {steep} ({:.1}%), cliff {cliff} ({:.2}% of land, {:.1}% of steep), scree {scree}",
            steep as f32 / land as f32 * 100.0,
            cliff as f32 / land as f32 * 100.0,
            steep_cliff as f32 / steep.max(1) as f32 * 100.0
        );
        tiles.sort_by_key(|t| -t.0);
        for t in tiles.iter().take(8) {
            eprintln!("  tile {},{}: {}", t.1, t.2, t.0);
        }
        // Gorges: river nodes with one, and dry ravine floors.
        let mut gorges: Vec<(f32, usize)> = (0..c.size * c.size)
            .map(|i| (ravine::gorge_depth(&c, 7, i), i))
            .filter(|g| g.0 > m(4.0))
            .collect();
        gorges.sort_by(|a, b| b.0.total_cmp(&a.0));
        eprintln!("{} gorge nodes deeper than 4 m", gorges.len());
        for &(d, i) in gorges.iter().step_by((gorges.len() / 8).max(1)).take(8) {
            let (a, b) = (node(&c, i), node(&c, c.rcv[i] as usize));
            let dir = (b - a).normalize();
            eprintln!("  gorge {:.0} m at {:.0},{:.0}  downstream {:.2},{:.2}  q {:.0}", d / m(1.0), a.x, a.y, dir.x, dir.y, c.discharge[i]);
        }
        let mut dry = Vec::new();
        for z in (0..MACRO_SIZE).step_by(64) {
            for x in (0..MACRO_SIZE).step_by(64) {
                let i = (z / 64) as usize * c.size + (x / 64) as usize;
                if c.ocean[i] || c.lake[i] != NO_LAKE {
                    continue;
                }
                let mut g = c.height[i] * OLD_VOXEL;
                if ravine::dry(7, x as f64, z as f64, || 1.0, &mut g) == cliff::Part::Foot(1.0) {
                    dry.push((x, z));
                }
            }
        }
        eprintln!("{} dry ravine floor samples: {:?}", dry.len(), dry.iter().step_by((dry.len() / 8).max(1)).collect::<Vec<_>>());
        // Trails that run into a cliff (the trail is levelled through it).
        let mut c = c;
        c.trails = crate::trails::build(&c, 7);
        let (mut on_trail, mut trail_cliff) = (0, Vec::new());
        for line in &c.trails.lines {
            for p in line.iter().step_by(4) {
                let (x, z) = (p.x as i32, p.y as i32);
                let s = Sampler::new(&c, 7, x - 6, z - 6, 13);
                on_trail += 1;
                let near = (-6..=6).step_by(3).any(|dz| (-6..=6).step_by(3).any(|dx| s.column(x + dx, z + dz).cover == Cover::Cliff));
                if near {
                    trail_cliff.push((x, z));
                }
            }
        }
        eprintln!("trail points {on_trail}, near a cliff {}: {:?}", trail_cliff.len(), &trail_cliff[..trail_cliff.len().min(6)]);
        // Waterfalls over a lip: how many, how tall, and the biggest few.
        let mut falls: Vec<(f32, usize)> = (0..c.size * c.size)
            .filter_map(|i| Fall::at(&c, 7, i).map(|f| (f.height, i)))
            .collect();
        falls.sort_by(|a, b| b.0.total_cmp(&a.0));
        let fp = |p: f32| falls.get((falls.len() as f32 * p) as usize).map_or(0.0, |f| f.0 / m(1.0));
        eprintln!("{} waterfalls: p10 {:.1} m, median {:.1} m, p90 {:.1} m", falls.len(), fp(0.1), fp(0.5), fp(0.9));
        for &(h, i) in falls.iter().step_by((falls.len() / 8).max(1)).take(8) {
            let r = c.rcv[i] as usize;
            let (a, b) = (node(&c, i), node(&c, r));
            let p = (a + b) * 0.5;
            let d = (b - a).normalize();
            eprintln!(
                "  fall {:.0} m at {:.0},{:.0}  downstream {:.2},{:.2}  q {:.0}",
                h / m(1.0),
                p.x,
                p.y,
                d.x,
                d.y,
                c.discharge[i]
            );
        }
        // Face heights, and the tallest faces at least 300 m apart.
        faces.sort_by(|a, b| b.0.total_cmp(&a.0));
        let pct = |p: f32| faces.get((faces.len() as f32 * p) as usize).map_or(0.0, |f| f.0 / m(1.0));
        eprintln!("face drop over 3.2 m: p10 {:.1} m, median {:.1} m, p90 {:.1} m", pct(0.1), pct(0.5), pct(0.9));
        let mut picked: Vec<(f32, i32, i32, i32, i32)> = Vec::new();
        for f in &faces {
            if picked.iter().all(|p| ((p.1 - f.1) as f32).hypot((p.2 - f.2) as f32) > m(300.0)) {
                picked.push(*f);
                // Yaw of the direction from the drop back to the face: where to look from.
                let yaw = (-(f.4 as f32)).atan2(-(f.3 as f32)).to_degrees();
                eprintln!("  face {:.0} m at {},{}  drop toward {},{}  look yaw {yaw:.0}", f.0 / m(1.0), f.1, f.2, f.3, f.4);
                if picked.len() == 12 {
                    break;
                }
            }
        }
    }

    /// Samples the land just beyond the banks of every `step`th river node, out to
    /// 20 m from the channel, and measures how far each column has been dug below
    /// both its natural height and the river's water: the valley floor carved out
    /// beside the water. Returns (the deepest as (depth, x, z, cover), samples dug
    /// deeper than 2 m, samples). With `unguarded`, also collects the samples that
    /// cliffs and fault ravines would dig that deep if they ignored the river, as
    /// they used to.
    fn bank_cuts(seed: u32, step: usize, mut unguarded: Option<&mut Vec<(f32, i32, i32, Cover)>>) -> (Vec<(f32, i32, i32, Cover)>, usize, usize) {
        let c = generate(cell_desc(seed, 0, 0), SimConfig { size: 1024 });
        let cs = c.cell_size();
        let (mut worst, mut deep, mut total) = (Vec::new(), 0, 0);
        for i in (0..c.size * c.size).step_by(step) {
            if c.ocean[i] || c.lake[i] != NO_LAKE || c.discharge[i] < MIN_RIVER_Q {
                continue;
            }
            let r = c.rcv[i] as usize;
            if r == i || c.ocean[r] || c.lake[r] != NO_LAKE {
                continue;
            }
            // Smooth water level between the two nodes, in voxels; the lower one, so a
            // drop between them does not count as digging.
            let level = river_level(&c, i).min(river_level(&c, r)) * OLD_VOXEL;
            if level < m(4.0) {
                continue;
            }
            let (a, b) = (node(&c, i), node(&c, r));
            let mid = (a + b) * 0.5;
            let Some(dir) = (b - a).try_normalize() else { continue };
            let side = vec2(-dir.y, dir.x);
            let hw = Continent::river_width(c.discharge[i]) * OLD_VOXEL * 0.5;
            let s = Sampler::new(&c, seed, mid.x as i32 - 96, mid.y as i32 - 96, 192);
            for sign in [-1.0f32, 1.0] {
                for k in 0..10 {
                    let sd = bank_width(hw) + m(1.0) + k as f32 * m(2.0);
                    if sd > m(20.0) {
                        break;
                    }
                    let q = mid + side * sign * (hw + sd);
                    let (lx, lz) = (q.x as i32, q.y as i32);
                    let col = s.column(lx, lz);
                    // Only the land beside this river: not another channel's banks, a
                    // lake, or a trail cut into the slope.
                    if col.river < sd - m(1.0) || col.water_surface().is_some() || col.clear < m(6.0) {
                        continue;
                    }
                    let (fx, fz) = ((lx as f32 + 0.5) / cs - 0.5, (lz as f32 + 0.5) / cs - 0.5);
                    let (natural, slope) = natural_ground(&c, seed, fx, fz, lx as f64, lz as f64);
                    let dug = natural.min(level) - col.ground;
                    total += 1;
                    if dug > m(2.0) {
                        deep += 1;
                        worst.push((dug, lx, lz, col.cover));
                    }
                    if let Some(unguarded) = unguarded.as_mut() {
                        // What cliffs and fault ravines that ignore the river would do here.
                        let mut g = natural;
                        let mut part = cliff::apply(seed, slope * OLD_VOXEL / cs, lx as f64, lz as f64, &mut g, |_| 1.0);
                        if let cliff::Part::Face(_) | cliff::Part::Foot(_) = ravine::dry(seed, lx as f64, lz as f64, || 1.0, &mut g) {
                            part = cliff::Part::Foot(1.0);
                        }
                        let dug = natural.min(level) - g;
                        if dug > m(2.0) {
                            let cover = if part == cliff::Part::None { Cover::Land } else { Cover::Cliff };
                            unguarded.push((dug, lx, lz, cover));
                        }
                    }
                }
            }
        }
        worst.sort_by(|a, b| b.0.total_cmp(&a.0));
        (worst, deep, total)
    }

    #[test]
    fn ground_beside_rivers_is_not_dug_below_the_water() {
        let (worst, deep, total) = bank_cuts(7, 5, None);
        eprintln!("{deep} of {total} samples beside rivers dug over 2 m below the water; worst {:?}", worst.first());
        assert!(total > 2000, "too few samples: {total}");
        // Cliffs, falls, gorges and ravines all leave the valley floor alone. A few
        // samples land where two rivers' banks meet, or on a trail's shoulder.
        assert!(deep * 500 <= total, "{deep} of {total} dug below the river: {:?}", &worst[..worst.len().min(8)]);
        assert!(worst.first().is_none_or(|w| w.0 < m(6.0)), "dug {:?}", worst.first());
    }

    /// Prints the deepest cuts beside rivers:
    /// `cargo test --release -p worldgen bank_cut_survey -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bank_cut_survey() {
        for seed in [1, 3, 7] {
            let mut unguarded = Vec::new();
            let (worst, deep, total) = bank_cuts(seed, 2, Some(&mut unguarded));
            unguarded.sort_by(|a, b| b.0.total_cmp(&a.0));
            eprintln!(
                "seed {seed}: {deep} of {total} samples beside rivers dug over 2 m below the water ({} if cliffs and ravines ignored the river)",
                unguarded.len()
            );
            for (name, list) in [("now", &worst), ("unguarded", &unguarded)] {
                let mut picked: Vec<(f32, i32, i32, Cover)> = Vec::new();
                for w in list {
                    if picked.iter().all(|p| ((p.1 - w.1) as f32).hypot((p.2 - w.2) as f32) > m(200.0)) {
                        picked.push(*w);
                        eprintln!("  {name}: dug {:.1} m at {},{} ({:?})", w.0 / m(1.0), w.1, w.2, w.3);
                        if picked.len() == 8 {
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Prints column cost on steep, flat and gorge ground:
    /// `cargo test --release -p worldgen column_cost -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn column_cost() {
        let c = generate(cell_desc(7, 0, 0), SimConfig { size: 1024 });
        for (name, (cx, cz)) in [("mountain", (22016, 40064)), ("lowland", (30000, 15000)), ("coast gorge", (26240, 45824))] {
            let (mut best, mut sum) = (f64::MAX, 0.0);
            for _ in 0..3 {
                let t0 = std::time::Instant::now();
                let s = Sampler::new(&c, 7, cx - 256, cz - 256, 512);
                for dz in 0..512 {
                    for dx in 0..512 {
                        sum += s.column(cx - 256 + dx, cz - 256 + dz).ground;
                    }
                }
                best = best.min(t0.elapsed().as_nanos() as f64 / 262144.0);
            }
            eprintln!("{name}: {best:.0} ns/column ({sum})");
        }
    }
}
