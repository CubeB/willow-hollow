//! Roots showing where the ground falls away from a tree: on a steep hillside, the lip
//! of a bank or the edge of a drop, the soil washes off the downhill side and leaves the
//! roots gripping it. On level ground they stay buried, so a tree there shows none.
//!
//! A root leaves the foot of the trunk as wood, thick and climbing a little way up the
//! trunk, runs down the slope lying on the ground and tapering, and ends in a strand of
//! 0.1 m pieces. The ground along each root is measured once, where the tree is placed,
//! so every chunk the root crosses draws it the same.
//!
//! A big old tree is different: its weight is carried out into the ground on buttresses,
//! whatever the lie of the land. A ring of them flares from its foot, tall where they
//! leave the trunk and sinking as they run out over the ground, in and out of the soil,
//! so the foot of a redwood or an ancient oak is a thing to walk round and step over. The
//! bigger the trunk the taller, longer and more of them.
//!
//! A large tree short of that (a big oak or maple, a yew, a tall pine) has a smaller
//! flare: a few short roots that leave the trunk a little way up it and step down into the
//! ground within a metre or two, among any bared on a slope. Under both, the trunk swells
//! as it meets the ground, a collar that is widest at the soil and follows it down a
//! slope, so the tree stands anchored rather than stuck in. A trunk of ordinary size keeps
//! to the roots above.

use glam::{IVec3, Vec2};
use voxel::{Voxel, mesh::{CELLS, PlantBox}};

use super::shape::Rng;
use super::trees::Species;
use super::{Library, rotate};
use crate::{materials as mat, noise::hash3};

/// Most roots one tree shows.
const MOST: usize = 8;
/// How far a root may run from the trunk, in voxels, wood and strand together.
const STEPS: usize = 28;
/// Radius of trunk, in voxels at the ground, from which a tree stands on buttresses: a
/// little over a metre across. Every young or middling tree is narrower than this.
const BUTTRESSED: f32 = 2.9;
/// Radius of trunk from which a tree's foot flares: the bigger half of the oaks and
/// maples, the yews and the tall pines, but no birch, spruce or holly.
const FLARED: f32 = 2.2;
/// Directions round the trunk the ground is measured in, for its collar.
const COLLAR_WAYS: usize = 16;
/// The radius at which buttresses are as big as they get: a redwood's.
const MIGHTY: f32 = 7.5;
/// Most roots a tree of ordinary size bares on a slope, and how far they may run.
const SLOPE_MOST: usize = 4;
const SLOPE_STEPS: usize = 14;
/// Slope (drop over run) under which the ground keeps a tree's roots buried.
const BURIED: f32 = 0.4;
/// Lowest the ground under a root may fall, below the tree's own ground: models reach
/// no deeper.
const DEEPEST: i32 = -7;

/// One root, in world voxels.
#[derive(Clone, Copy, Debug)]
struct Root {
    /// Where it leaves the trunk, across the ground.
    from: Vec2,
    dir: Vec2,
    /// Voxels it runs as wood, then all the voxels it runs.
    wood: u8,
    len: u8,
    /// Its radius where it leaves the trunk, in voxels.
    girth: f32,
    /// Height of the ground's top voxel a voxel at a time along it, from `from`, against
    /// the tree's own ground.
    ground: [i8; STEPS + 1],
    /// How far it turns aside per voxel of its run, in radians: a buttress wanders.
    bend: f32,
    /// A buttress or a flare only (0 for a root bared on a slope): how high it stands
    /// against the trunk, in voxels above the ground, and where along it the soil starts to
    /// come over it and let it go (a phase, in radians).
    fin: f32,
    dip: f32,
    /// Whether it goes under the soil and comes up again on its way out, as a long
    /// buttress does; a short flare only sinks into the ground at its end.
    waves: bool,
}

impl Root {
    fn at(&self, t: f32) -> Vec2 {
        if self.bend == 0.0 {
            return self.from + self.dir * t;
        }
        // Along a gentle arc: the chord to a point `t` along it turns half as far as the
        // root has by then.
        let (s, c) = (self.bend * t * 0.5).sin_cos();
        self.from + Vec2::new(self.dir.x * c - self.dir.y * s, self.dir.x * s + self.dir.y * c) * t
    }

    fn ground_at(&self, t: f32) -> i32 {
        self.ground[(t.max(0.0) as usize).min(self.len as usize)] as i32
    }
}

/// The swelling of a big trunk where it meets the ground: from `radius` out to `radius +
/// swell` round `centre`, standing `height` voxels up the trunk at the bark and sinking
/// into the soil at its edge, concave like the foot of a real trunk.
#[derive(Clone, Copy, Debug)]
struct Collar {
    centre: Vec2,
    radius: f32,
    swell: f32,
    height: f32,
    /// The ground's top at the collar's edge, a way at a time round from +x, against the
    /// tree's own ground: the collar follows it down a slope.
    ground: [i8; COLLAR_WAYS],
}

impl Collar {
    /// The top of the collar over the column whose middle is `p`, and the ground there,
    /// against the tree's own ground; None outside it.
    fn at(&self, p: Vec2) -> Option<(f32, f32)> {
        let rel = p - self.centre;
        let t = (rel.length() - self.radius) / self.swell;
        if t >= 1.0 {
            return None;
        }
        let t = t.max(0.0);
        // The ground between the two ways measured either side, and between the trunk's
        // own ground at the bark and the collar's edge.
        let a = rel.y.atan2(rel.x).rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU * COLLAR_WAYS as f32;
        let (i, f) = (a.floor() as usize % COLLAR_WAYS, a.fract());
        let edge = self.ground[i] as f32 * (1.0 - f) + self.ground[(i + 1) % COLLAR_WAYS] as f32 * f;
        Some((self.height * (1.0 - t) * (1.0 - t), edge * t))
    }
}

/// The roots a placed tree shows, if any.
#[derive(Clone, Copy, Debug)]
pub struct Roots {
    list: [Option<Root>; MOST],
    bark: Voxel,
    /// Buttresses and flares only: how far they spread from the tree, in voxels each way
    /// across the ground, past its model's own reach (0 for roots, which keep within the
    /// model).
    spread: u8,
    /// A large tree's collar, which it has whenever it stands on buttresses or flares (the
    /// roots in the list with a `fin`, which [`Roots::voxels`] draws their own way and with
    /// no strands).
    collar: Option<Collar>,
}

/// Whether a species shows roots: trees with a woody trunk. Palms hold the sand with a
/// mat of thin roots that never shows.
fn rooted(species: Species) -> bool {
    matches!(
        species,
        Species::Oak
            | Species::Birch
            | Species::Maple
            | Species::Cherry
            | Species::Spruce
            | Species::TallPine
            | Species::Redwood
            | Species::ElderOak
            | Species::Jungle
            | Species::Kapok
            | Species::Acacia
            | Species::Baobab
            | Species::Willow
            | Species::DeadSnag
            | Species::Holly
            | Species::Yew
            | Species::HolmOak
            | Species::ScotsPine
    )
}

/// Whether a voxel of a tree's model is its bark, the wood of its trunk and limbs.
pub fn is_bark(v: Voxel) -> bool {
    matches!(
        v,
        mat::BARK_OAK
            | mat::BARK_BIRCH
            | mat::BARK_PINE
            | mat::BARK_REDWOOD
            | mat::BARK_JUNGLE
            | mat::BARK_WILLOW
            | mat::BARK_ACACIA
            | mat::BARK_BAOBAB
            | mat::WOOD_DEAD
            | mat::BARK_HOLLY
            | mat::BARK_YEW
    )
}

/// The foot of a tree's trunk, from its model turned by `turns`: the middle of the bark
/// at the ground, against the model's origin; how far round it is, in voxels; and the bark.
/// None for a model with no bark there.
pub fn foot(lib: &Library, species: Species, variant: usize, turns: u32) -> Option<(Vec2, f32, Voxel)> {
    let t = lib.get(species, variant);
    let row = t.voxels.partition_point(|(p, _)| p.y < 0);
    let (mut sum, mut n, mut bark) = (Vec2::ZERO, 0, None);
    for &(p, v) in t.voxels[row..].iter().take_while(|(p, _)| p.y == 0) {
        if is_bark(v) {
            let q = rotate(p, turns);
            sum += Vec2::new(q.x as f32 + 0.5, q.z as f32 + 0.5);
            n += 1;
            bark = bark.or(Some(v));
        }
    }
    let bark = bark?;
    Some((sum / n as f32, (n as f32 / std::f32::consts::PI).sqrt(), bark))
}

/// Works out the roots of a tree standing at `pos` (its origin, on the first voxel above
/// the ground). `ground` gives the top of the ground at a world column, or None over
/// water. `room` is how far from `pos` the roots of a big tree may spread: as far as the
/// chunks around look for the trees that reach into them. None where the ground is too
/// level for any to show.
pub fn find(lib: &Library, species: Species, variant: usize, turns: u32, pos: IVec3, seed: u32, room: i32, ground: impl Fn(i32, i32) -> Option<i32>) -> Option<Roots> {
    if !rooted(species) {
        return None;
    }
    // The foot of the trunk: where it stands and how thick it is. Roots wear the trunk's
    // own bark, so a birch's are pale like its trunk.
    let (offset, radius, bark) = foot(lib, species, variant, turns)?;
    let centre = Vec2::new(pos.x as f32, pos.z as f32) + offset;
    let base = pos.y - 1;
    let ground_at = |p: Vec2| ground(p.x.floor() as i32, p.y.floor() as i32).map(|g| g - base);
    if radius >= BUTTRESSED {
        return buttresses(room, centre, radius, bark, pos, seed, ground_at);
    }
    let flared = radius >= FLARED;

    // How steeply the ground falls away from the trunk each way round.
    const WAYS: usize = 16;
    let heading = |a: f32| Vec2::new(a.cos(), a.sin());
    let run = radius + 4.0;
    let slope = |a: f32| ground_at(centre + heading(a) * run).map_or(0.0, |g| -g as f32 / run);
    let (mut down, mut steepest) = (0.0, 0.0);
    for k in 0..WAYS {
        let a = k as f32 / WAYS as f32 * std::f32::consts::TAU;
        let s = slope(a);
        if s > steepest {
            (down, steepest) = (a, s);
        }
    }
    if steepest < BURIED && !flared {
        return None;
    }

    let mut rng = Rng::new(hash3(seed ^ 0x7007, pos.x, pos.y, pos.z));
    // Steeper ground bares more of them.
    let want = if steepest < BURIED { 0 } else { (1 + ((steepest - BURIED) / 0.15) as i32 + rng.int(0, 1)).min(SLOPE_MOST as i32) as usize };
    let mut list = [None; MOST];
    let mut found = 0;
    // Downhill first, then fanning out to either side.
    for spread in [0.0f32, 0.75, -0.75, 1.35, -1.35, 1.9, -1.9] {
        if found == want {
            break;
        }
        let a = down + spread + rng.range(-0.25, 0.25);
        if slope(a) < BURIED * 0.6 {
            continue;
        }
        let dir = heading(a);
        let from = centre + dir * (radius - 0.5).max(0.5);
        let mut root = Root { from, dir, wood: 0, len: 0, girth: (0.55 + radius * 0.2).min(1.5), ground: [0; STEPS + 1], bend: 0.0, fin: 0.0, dip: 0.0, waves: false };
        let mut len = 0;
        let mut last = 0;
        // Within the model's reach, or a chunk the root runs into would not know of it.
        let most = ((lib.get(species, variant).reach as f32 - radius - 2.0).max(0.0) as usize).min(SLOPE_STEPS);
        for k in 0..=most {
            let Some(g) = ground_at(root.at(k as f32)) else { break };
            // A root follows the ground down, but stops at a rise, a sheer drop or water.
            if k > 0 && (g > last || g < last - 2) || g < DEEPEST {
                break;
            }
            root.ground[k] = g as i8;
            last = g;
            len = k;
        }
        let reach = (3.0 + radius * 0.7) as usize + rng.int(0, 2) as usize;
        let wood = reach.min(len.saturating_sub(2));
        if wood < 2 {
            continue;
        }
        root.wood = wood as u8;
        root.len = (wood + 2 + rng.int(0, 1) as usize).min(len) as u8;
        list[found] = Some(root);
        found += 1;
    }
    if !flared {
        return (found > 0).then_some(Roots { list, bark, spread: 0, collar: None });
    }

    // A large tree's flare: a few short roots round the foot, between those bared down
    // the slope, if any, and the collar under them all.
    use std::f32::consts::TAU;
    let bared: Vec<Vec2> = list.iter().flatten().map(|r: &Root| r.dir).collect();
    let mut rng = Rng::new(hash3(seed ^ 0xf1a4, pos.x, pos.y, pos.z));
    let count = 3 + rng.int(0, 2) as usize;
    let a0 = rng.range(0.0, TAU);
    for k in 0..count {
        let a = a0 + (k as f32 + rng.range(-0.3, 0.3)) / count as f32 * TAU;
        let dir = Vec2::new(a.cos(), a.sin());
        let girth = 0.55 + rng.range(0.0, 0.3);
        let fin = radius * rng.range(0.7, 1.0) + 1.0;
        let length = radius * rng.range(0.8, 1.2) + 2.0;
        let bend = rng.range(-0.08, 0.08);
        // Not where a root already runs down the slope, nor past the slots there are.
        if found == MOST || bared.iter().any(|b| b.dot(dir) > 0.85) {
            continue;
        }
        let from = centre + dir * (radius - 0.8).max(0.5);
        let mut root = Root { from, dir, wood: 0, len: 0, girth, ground: [0; STEPS + 1], bend, fin, dip: 0.0, waves: false };
        let most = (room as f32 - from.distance(Vec2::new(pos.x as f32, pos.z as f32)) - girth - 1.5).clamp(0.0, STEPS as f32);
        let len = lay(&mut root, length.min(most) as usize, &ground_at);
        if len < 2 {
            continue;
        }
        (root.wood, root.len) = (len as u8, len as u8);
        list[found] = Some(root);
        found += 1;
    }
    let collar = collar(centre, radius, radius * 0.8 + 1.2, radius * 0.45 + 0.8, &ground_at);
    Some(Roots { list, bark, spread: spread(&list, &collar, pos), collar: Some(collar) })
}

/// Measures the ground under `root` a voxel at a time, out to `want` voxels from where it
/// leaves the trunk, and how far it can run: over a hump of the ground or down a bank, but
/// not up a wall, over a drop or into water. Down a hillside it follows the ground a long
/// way: every voxel of it lies on its own column's ground, which that column's chunks hold.
fn lay(root: &mut Root, want: usize, ground_at: &impl Fn(Vec2) -> Option<i32>) -> usize {
    let (mut last, mut len) = (0, 0);
    for s in 0..=want {
        let Some(g) = ground_at(root.at(s as f32)) else { break };
        if s > 0 && (g > last + 1 || g < last - 2) || !(-10..=4).contains(&g) {
            break;
        }
        root.ground[s] = g as i8;
        last = g;
        len = s;
    }
    len
}

/// How far a tree's buttresses or flares and its collar spread from its `pos`, each way
/// across the ground (see [`Roots::spread`]).
fn spread(list: &[Option<Root>], collar: &Collar, pos: IVec3) -> u8 {
    let spread = list.iter().flatten().filter(|r| r.fin > 0.0).map(|r: &Root| {
        let end = r.at(r.len as f32);
        (end.x - pos.x as f32).abs().max((end.y - pos.z as f32).abs()) + r.girth + 1.5
    });
    let off = (collar.centre - Vec2::new(pos.x as f32, pos.z as f32)).abs().max_element();
    spread.fold(off + collar.radius + collar.swell + 1.0, f32::max).ceil() as u8
}

/// The collar of a trunk `radius` voxels round `centre`, standing `height` up it and
/// `swell` out from it, with the ground measured round its edge.
fn collar(centre: Vec2, radius: f32, height: f32, swell: f32, ground_at: &impl Fn(Vec2) -> Option<i32>) -> Collar {
    let mut ground = [0i8; COLLAR_WAYS];
    for (k, g) in ground.iter_mut().enumerate() {
        let a = k as f32 / COLLAR_WAYS as f32 * std::f32::consts::TAU;
        // Over water, or off the edge of a drop, it stays at the tree's own ground.
        *g = ground_at(centre + Vec2::new(a.cos(), a.sin()) * (radius + swell)).map_or(0, |g| g.clamp(-8, 4)) as i8;
    }
    Collar { centre, radius, swell, height, ground }
}

/// The buttresses of a big tree, whose trunk at the ground is `radius` voxels round
/// `centre`; no part of a buttress spreads more than `room` from the tree's `pos` (see
/// [`Roots::spread`]).
fn buttresses(room: i32, centre: Vec2, radius: f32, bark: Voxel, pos: IVec3, seed: u32, ground_at: impl Fn(Vec2) -> Option<i32>) -> Option<Roots> {
    use std::f32::consts::TAU;
    let size = ((radius - BUTTRESSED) / (MIGHTY - BUTTRESSED)).clamp(0.0, 1.0);
    let mut rng = Rng::new(hash3(seed ^ 0xb077, pos.x, pos.y, pos.z));
    let count = (4 + (size * 3.0).round() as i32 + rng.int(0, 1)).min(MOST as i32) as usize;
    let a0 = rng.range(0.0, TAU);
    let mut list = [None; MOST];
    let mut found = 0;
    for k in 0..count {
        // Round the trunk, but not evenly: real ones crowd to one side and leave a gap.
        let a = a0 + (k as f32 + rng.range(-0.3, 0.3)) / count as f32 * TAU;
        let dir = Vec2::new(a.cos(), a.sin());
        // From inside the bark, so no seam shows where the buttress meets the trunk.
        let inset = (radius - 1.0).max(0.5);
        let from = centre + dir * inset;
        // The widest a buttress gets, either side of its line, and so the room it needs.
        let girth = 0.9 + 1.1 * size + rng.range(0.0, 0.3);
        let most = (room as f32 - from.distance(Vec2::new(pos.x as f32, pos.z as f32)) - girth - 1.5).clamp(0.0, STEPS as f32);
        let want = (radius * rng.range(1.3, 2.0) + 4.0 + size * 3.0).min(most) as usize;
        let mut root = Root {
            from,
            dir,
            wood: 0,
            len: 0,
            girth,
            ground: [0; STEPS + 1],
            bend: rng.range(-0.05, 0.05),
            fin: radius * rng.range(0.7, 1.1) + 1.5,
            dip: rng.range(0.0, TAU),
            waves: true,
        };
        let len = lay(&mut root, want, &ground_at);
        if len < 3 {
            continue;
        }
        root.wood = len as u8;
        root.len = len as u8;
        list[found] = Some(root);
        found += 1;
    }
    let collar = collar(centre, radius, (radius * 0.9 + 2.0).min(9.0), (radius * 0.5 + 1.5).min(6.0), &ground_at);
    Some(Roots { list, bark, spread: spread(&list, &collar, pos), collar: Some(collar) })
}

/// One buttress's voxels. Seen across it, it is a ridge rounded over the top, as high as
/// it stands and as broad as its girth; along it, a plate high up the trunk that falls
/// away fast and runs on low over the ground. Past the middle of it the soil comes over
/// it and lets it go again every few metres, until it goes under for good; a short flare
/// just sinks into the soil toward its end. It is built down into the top of the ground a
/// voxel, so where the ground falls away beside it no daylight shows under its edge.
fn buttress_voxels(root: &Root, base: i32, bark: Voxel, set: &mut impl FnMut(IVec3, Voxel)) {
    let len = root.len as f32;
    let mut t = 0.0;
    while t <= len {
        let u = t / len;
        let fin = root.fin * (1.0 - u).powf(1.6);
        let ridge = (root.girth * 0.9 + 0.8) * (1.0 - 0.5 * u);
        let sunk = if root.waves {
            let wave = 0.5 + 0.5 * (t * 0.9 + root.dip).sin();
            (smooth(0.4, 0.7, u) * wave).max(smooth(0.9, 1.0, u))
        } else {
            smooth(0.55, 1.0, u)
        };
        let high = fin.max(ridge) * (1.0 - sunk) - 0.8 * sunk;
        if high >= 0.5 {
            let half = 0.5 + root.girth * (1.0 - u).powf(0.7);
            let c = root.at(t);
            let floor = base + root.ground_at(t);
            for vz in (c.y - half).floor() as i32..=(c.y + half).floor() as i32 {
                for vx in (c.x - half).floor() as i32..=(c.x + half).floor() as i32 {
                    let d = Vec2::new(vx as f32 + 0.5, vz as f32 + 0.5).distance(c) / half;
                    if d > 1.0 {
                        continue;
                    }
                    let stands = (high * (1.0 - d * d).sqrt()).round() as i32;
                    if stands < 1 {
                        continue;
                    }
                    for vy in floor - 1..=floor + stands {
                        set(IVec3::new(vx, vy, vz), bark);
                    }
                }
            }
        }
        t += 0.4;
    }
}

/// A collar's voxels: each column of it from a little into the ground up to where the
/// swelling trunk stands over it. It rises into the roots that leave the trunk along
/// `ways` and dips between them, so the foot is lobed rather than a plinth. Built two
/// voxels down, so on a slope, where the ground is only measured round the edge, no
/// daylight shows under it.
fn collar_voxels(c: &Collar, base: i32, bark: Voxel, ways: &[Vec2], set: &mut impl FnMut(IVec3, Voxel)) {
    let r = c.radius + c.swell;
    for vz in (c.centre.y - r).floor() as i32..=(c.centre.y + r).floor() as i32 {
        for vx in (c.centre.x - r).floor() as i32..=(c.centre.x + r).floor() as i32 {
            let p = Vec2::new(vx as f32 + 0.5, vz as f32 + 0.5);
            // Within the trunk, the trunk is there already.
            if p.distance(c.centre) < c.radius - 1.0 {
                continue;
            }
            let Some((top, floor)) = c.at(p) else { continue };
            let toward = (p - c.centre).normalize_or_zero();
            let lobe = ways.iter().map(|w| w.dot(toward).max(0.0).powi(4)).fold(0.0f32, f32::max);
            let top = top * if ways.is_empty() { 0.7 } else { 0.45 + 0.55 * lobe };
            let (top, floor) = (base + top.round() as i32, base + floor.round() as i32);
            if top <= floor {
                continue;
            }
            for vy in floor - 2..=top {
                set(IVec3::new(vx, vy, vz), bark);
            }
        }
    }
}

fn smooth(lo: f32, hi: f32, x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Roots {
    /// How far from the tree's `pos` any of it lies, across the ground, in voxels: 0 for
    /// roots, which keep within the reach of the tree's own model.
    pub fn spread(&self) -> i32 {
        self.spread as i32
    }

    /// Whether the tree's foot flares: a large tree's, on a collar, with buttresses or a
    /// few short flaring roots.
    pub fn flared(&self) -> bool {
        self.collar.is_some()
    }

    /// A flaring foot's collar: the middle of the trunk, in world voxels across the ground,
    /// and how far out from it the collar comes.
    pub fn collar(&self) -> Option<(Vec2, f32)> {
        self.collar.map(|c| (c.centre, c.radius + c.swell))
    }

    /// The wooden voxels of the roots, in world positions.
    pub fn voxels(&self, pos: IVec3, mut set: impl FnMut(IVec3, Voxel)) {
        let base = pos.y - 1;
        if let Some(c) = &self.collar {
            let mut ways = [Vec2::ZERO; MOST];
            let mut n = 0;
            for root in self.list.iter().flatten().filter(|r| r.fin > 0.0) {
                ways[n] = root.dir;
                n += 1;
            }
            collar_voxels(c, base, self.bark, &ways[..n], &mut set);
        }
        for root in self.list.iter().flatten() {
            if root.fin > 0.0 {
                buttress_voxels(root, base, self.bark, &mut set);
                continue;
            }
            let wood = root.wood as f32;
            let mut t = 0.0;
            while t <= wood {
                let u = t / wood;
                // Thick and climbing the trunk at the foot, thinning to a voxel at the end.
                let r = root.girth + (0.5 - root.girth) * u;
                let lift = (1.0 - u).powi(2) * root.girth * 1.6;
                let at = root.at(t);
                let y = (base + root.ground_at(t) + 1) as f32 + r * 0.35 + lift;
                let c = glam::Vec3::new(at.x, y, at.y);
                let lo = (c - glam::Vec3::splat(r)).floor().as_ivec3();
                let hi = (c + glam::Vec3::splat(r)).floor().as_ivec3();
                set(c.floor().as_ivec3(), self.bark);
                for vy in lo.y..=hi.y {
                    for vz in lo.z..=hi.z {
                        for vx in lo.x..=hi.x {
                            let p = glam::Vec3::new(vx as f32 + 0.5, vy as f32 + 0.5, vz as f32 + 0.5);
                            if (p - c).length_squared() <= r * r {
                                set(IVec3::new(vx, vy, vz), self.bark);
                            }
                        }
                    }
                }
                t += 0.5;
            }
        }
    }

    /// The fine strands the roots end in, in world cells. Buttresses and flares have none:
    /// they go into the ground whole.
    pub fn strands(&self, pos: IVec3, out: &mut Vec<PlantBox>) {
        let base = pos.y - 1;
        let wood = self.bark;
        let step = 1.0 / CELLS as f32;
        for root in self.list.iter().flatten().filter(|r| r.fin == 0.0) {
            let (from, to) = (root.wood as f32 + 0.25, root.len as f32 + 0.5);
            let mut prev: Option<PlantBox> = None;
            let mut t = from;
            while t < to {
                let u = (t - from) / (to - from);
                let thick = if u < 0.45 { 2 } else { 1 };
                let at = root.at(t) * CELLS as f32;
                let (x, z) = (at.x.floor() as i32, at.y.floor() as i32);
                let y = (base + root.ground_at(t) + 1) * CELLS;
                let here = PlantBox { lo: [x, y, z], hi: [x + thick, y + thick, z + thick], voxel: wood, sway_lo: false, sway_hi: false, thin: 0, season: 0, flap: 0 };
                match prev {
                    // Running on along x or z at the same height: one longer box.
                    Some(ref mut b) if b.lo[1] == y && b.hi[1] == y + thick && b.lo[2] == z && b.hi[2] == z + thick && (b.hi[0] == x || b.lo[0] == x + thick || (b.lo[0]..b.hi[0]).contains(&x)) => {
                        b.lo[0] = b.lo[0].min(x);
                        b.hi[0] = b.hi[0].max(x + thick);
                    }
                    Some(ref mut b) if b.lo[1] == y && b.hi[1] == y + thick && b.lo[0] == x && b.hi[0] == x + thick && (b.hi[2] == z || b.lo[2] == z + thick || (b.lo[2]..b.hi[2]).contains(&z)) => {
                        b.lo[2] = b.lo[2].min(z);
                        b.hi[2] = b.hi[2].max(z + thick);
                    }
                    _ => {
                        if let Some(b) = prev.take() {
                            // Over a step down, the strand drapes down its face.
                            if b.lo[1] > y {
                                out.push(PlantBox { lo: [x, y, z], hi: [x + 1, b.lo[1], z + 1], ..b });
                            }
                            out.push(b);
                        }
                        prev = Some(here);
                    }
                }
                t += step;
            }
            out.extend(prev);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use voxel::{AIR, CHUNK};

    use super::*;
    use crate::{
        World,
        continent::SimConfig,
        features::attach::{ColumnVoxels, Voxels},
        world::GameWorld,
    };

    #[test]
    fn big_trees_stand_on_buttresses_that_spread_over_the_ground() {
        // A redwood in world 1's old-growth wood, near the river below the start.
        let w = GameWorld::new(1);
        let at = IVec3::new(38154, 76, 26748);
        let out = w.generate_column(at.x.div_euclid(CHUNK), at.z.div_euclid(CHUNK));
        let p = out.trees.iter().find(|p| p.pos == at).expect("the redwood is where it was");
        assert_eq!(p.species, Species::Redwood);
        let roots = p.roots.expect("a redwood always shows its buttresses");
        assert!(roots.flared());
        assert!(roots.list.iter().flatten().count() >= 5, "a ring of them");
        // They reach well out from the trunk, and stand above the ground out there, then
        // go under it.
        let mut out_there = HashSet::new();
        roots.voxels(p.pos, |q, _| {
            let d = ((q.x - at.x) as f32).hypot((q.z - at.z) as f32);
            if d > 12.0 && q.y > w.column(q.x, q.z).top() {
                out_there.insert((q.x, q.z));
            }
        });
        assert!(out_there.len() > 20, "only {} columns of buttress show beyond 5 m of the trunk", out_there.len());
        // And none of a tree of ordinary size: they keep the roots they had.
        let small: Vec<_> = out.trees.iter().filter(|p| matches!(p.species, Species::Birch | Species::Spruce | Species::Holly)).collect();
        assert!(small.iter().all(|p| p.roots.is_none_or(|r| !r.flared())));
    }

    #[test]
    fn roots_bare_on_slopes_only_and_agree_across_chunks() {
        let w = GameWorld::with_config(11, SimConfig { size: 256 });
        let centre = crate::macro_layout::MACRO_SIZE / CHUNK / 2;
        let mut outputs = HashMap::new();
        let (mut rooted_trees, mut trees, mut checked) = (0, 0, 0);
        let mut seen = HashSet::new();
        'search: for cx in (centre - 200..centre + 200).step_by(7) {
            for cz in (centre - 200..centre + 200).step_by(7) {
                let out = w.generate_column(cx, cz);
                for p in &out.trees {
                    if !rooted(p.species) || !seen.insert(p.pos) {
                        continue;
                    }
                    trees += 1;
                    let Some(roots) = &p.roots else { continue };
                    rooted_trees += 1;
                    // The ground really falls away from a tree of ordinary size that
                    // shows roots. A big one stands on buttresses anywhere.
                    if !roots.flared() {
                        let g = |x: i32, z: i32| w.column(x, z).top();
                        let lowest = (-6..=6).flat_map(|dx| (-6..=6).map(move |dz| (dx, dz))).map(|(dx, dz)| g(p.pos.x + dx, p.pos.z + dz)).min().unwrap();
                        assert!(p.pos.y - 1 - lowest >= 2, "{:?} at {} shows roots on level ground", p.species, p.pos);
                    } else {
                        trees -= 1;
                        rooted_trees -= 1;
                    }
                    // Every root voxel is there in the column that holds it, whichever
                    // chunk column the tree stands in.
                    let mut voxels = Vec::new();
                    roots.voxels(p.pos, |q, v| voxels.push((q, v)));
                    for (q, _) in voxels.into_iter().step_by(3) {
                        let (ox, oz) = (q.x.div_euclid(CHUNK), q.z.div_euclid(CHUNK));
                        let held = outputs.entry((ox, oz)).or_insert_with(|| w.generate_column(ox, oz));
                        let got = ColumnVoxels::new(&held.chunks, ox * CHUNK - 1, oz * CHUNK - 1).voxel(q);
                        assert_ne!(got, AIR, "root voxel of {:?} at {} missing at {q}", p.species, p.pos);
                        checked += 1;
                    }
                    if rooted_trees >= 12 {
                        break 'search;
                    }
                }
            }
        }
        assert!(rooted_trees > 0 && checked > 0, "no tree showed roots");
        assert!(rooted_trees * 3 < trees, "{rooted_trees} of {trees} trees show roots");
    }

    #[test]
    fn large_trees_flare_at_the_foot_and_the_flares_agree_across_chunks() {
        let w = GameWorld::with_config(11, SimConfig { size: 256 });
        let centre = crate::macro_layout::MACRO_SIZE / CHUNK / 2;
        let mut outputs = HashMap::new();
        let mut seen = HashSet::new();
        let (mut flared, mut crossing, mut checked) = (0, 0, 0);
        'search: for cx in (centre - 200..centre + 200).step_by(5) {
            for cz in (centre - 200..centre + 200).step_by(5) {
                let out = w.generate_column(cx, cz);
                for p in &out.trees {
                    let Some(roots) = p.roots.filter(|r| r.collar.is_some()) else { continue };
                    if !seen.insert(p.pos) {
                        continue;
                    }
                    flared += 1;
                    let mut voxels = Vec::new();
                    roots.voxels(p.pos, |q, _| voxels.push(q));
                    // It stands up out of the ground, not only under it.
                    assert!(voxels.iter().any(|q| q.y > w.column(q.x, q.z).top()), "the flare of {:?} at {} is all underground", p.species, p.pos);
                    // Every voxel is there in the chunk column that holds it, whichever
                    // column the tree stands in.
                    let home = (p.pos.x.div_euclid(CHUNK), p.pos.z.div_euclid(CHUNK));
                    let mut crosses = false;
                    for q in voxels {
                        let (ox, oz) = (q.x.div_euclid(CHUNK), q.z.div_euclid(CHUNK));
                        crosses |= (ox, oz) != home;
                        let held = outputs.entry((ox, oz)).or_insert_with(|| w.generate_column(ox, oz));
                        let got = ColumnVoxels::new(&held.chunks, ox * CHUNK - 1, oz * CHUNK - 1).voxel(q);
                        assert_ne!(got, AIR, "flare voxel of {:?} at {} missing at {q}", p.species, p.pos);
                        checked += 1;
                    }
                    crossing += crosses as u32;
                    if crossing >= 6 && flared >= 10 {
                        break 'search;
                    }
                }
            }
        }
        assert!(crossing >= 6 && flared >= 10, "only {flared} trees flared, {crossing} of them across a chunk border ({checked} voxels)");
    }

    #[test]
    fn a_collar_follows_the_ground_down_a_slope() {
        // A trunk three voxels round on ground falling a voxel for every two toward +x.
        let ground_at = |p: Vec2| Some(-(p.x / 2.0).floor() as i32);
        let c = collar(Vec2::new(0.5, 0.5), 3.0, 3.6, 2.0, &ground_at);
        let mut lowest: HashMap<(i32, i32), i32> = HashMap::new();
        collar_voxels(&c, 100, mat::BARK_OAK, &[], &mut |q, _| {
            let l = lowest.entry((q.x, q.z)).or_insert(q.y);
            *l = (*l).min(q.y);
        });
        assert!(!lowest.is_empty());
        // Every column of it reaches down into the ground under it, downhill or up.
        for (&(x, z), &low) in &lowest {
            let g = 100 + ground_at(Vec2::new(x as f32 + 0.5, z as f32 + 0.5)).unwrap();
            assert!(low <= g, "the collar at {x},{z} stops at {low}, over the ground at {g}");
        }
        // And it reaches further down on the downhill side than up.
        assert!(lowest[&(4, 0)] < lowest[&(-3, 0)]);
    }
}
