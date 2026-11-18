//! Quarries (#203): where rock breaks the surface about a town or a large village of stone
//! country, or an ancient village, the place keeps a quarry out in its land, and a mason
//! dresses its stone beside it.
//!
//! The pit is cut into a hillside in steps, each a stride up and a stride in, so it is
//! climbed out of by them, its faces bare rock of the ground's own beds, its floor of
//! chips and rubble. It opens downhill through its mouth onto a yard levelled before it,
//! where the cut blocks stand stacked ready, a handcart by them, the spoil heaped at the
//! far end, and the mason's shed (see [`special::barn`]). A crane on the floor at the
//! back lifts a block from it, and a cart track runs from the yard to the nearest way.
//! An old place's quarry has been worked longer, and is a step deeper and wider.
//!
//! A quarry is sited as a barn is (see [`super::farmstead`]): out in the settlement's
//! land and off its fields, never near a trail, a river, a yard, a building's ground or
//! a barn, nor on ground anything else has been stamped on (a logging camp's, say), and
//! where cliffs or scree break the ground near, or the country is rock under a thin skin
//! and the hillside steep. Of the places that would do, the rockiest and steepest near a
//! way is worked.
//!
//! A quarry is not among a settlement's buildings (`Network::plots`), so it leaves its
//! count of walk-in buildings alone; quarries are kept in [`Land::quarries`](super::Land).

use glam::{IVec2, IVec3, Vec2, vec2};
use voxel::{Voxel, mesh::PlantBox};

use super::{farmstead::cover, influence::CROPS};
use crate::{
    biome::Biome,
    continent::Continent,
    features::{fittings::{self, Fitting}, parts, shape::rotate},
    materials as mat,
    noise::{hash2, hash3, unit},
    scale::m,
    settlements::{
        Age, Kind, Livelihood,
        special::barn::{self, BarnKind, BarnSpec},
    },
    surface::{Column, Cover, Sampler, local_rock},
    trails::{
        Pad, TrailPoint,
        build::{in_parallel, is_wet, palette_at, profile, sample, turns_toward},
    },
};

/// A settlement's quarry, as raised.
#[derive(Clone, Debug, PartialEq)]
pub struct Quarry {
    /// The settlement whose quarry it is, by index in `Network::settlements`.
    pub owner: usize,
    /// The middle of the pit's floor, continent-local: the layer over the floor.
    pub floor: IVec3,
    /// The way out of the pit through its mouth, along a row or column of the world.
    pub mouth: IVec2,
    /// The columns the pit is cut over, its steps and its mouth, and the yard before the
    /// mouth, continent-local and inclusive.
    pub pit: (IVec2, IVec2),
    pub yard: (IVec2, IVec2),
    /// How far the ground the pit was cut into rose over its floor, in voxels.
    pub depth: i32,
    /// The mason's shed: its model origin and turns, the columns of its walls and of all it
    /// covers, inclusive, and its doorway's floor voxel and the way out through it.
    pub origin: IVec3,
    pub turns: u32,
    pub walls: (IVec2, IVec2),
    pub footprint: (IVec2, IVec2),
    pub door: IVec3,
    pub facing: IVec3,
    /// The cart track from the far end of the yard to the way it meets.
    pub track: Vec<Vec2>,
}

impl Quarry {
    /// Whether a stamp spanning `lo` to `hi`, continent-local, is the mason's shed: as
    /// [`super::farmstead::Stead::stamped`] tells a barn's.
    pub fn stamped(&self, lo: IVec3, hi: IVec3) -> bool {
        let (lo, hi) = (IVec2::new(lo.x, lo.z), IVec2::new(hi.x, hi.z));
        let (a, b) = (self.footprint.0 - IVec2::splat(2), self.footprint.1 + IVec2::splat(2));
        lo.cmple(self.walls.0).all() && hi.cmpge(self.walls.1).all() && lo.cmpge(a).all() && hi.cmple(b).all()
    }

    /// Whether a stamp spanning `lo` to `hi` is this quarry's own: its shed's, or the
    /// one its pit and yard are cut and built by, which reaches over the middle of its
    /// floor and keeps to the ground it covers. Nothing climbs in either: the faces are
    /// freshly worked.
    pub fn worked(&self, lo: IVec3, hi: IVec3) -> bool {
        let (a, b) = self.covers();
        let (a, b) = (a - IVec2::splat(6), b + IVec2::splat(6));
        let (lo2, hi2) = (IVec2::new(lo.x, lo.z), IVec2::new(hi.x, hi.z));
        let floor = IVec2::new(self.floor.x, self.floor.z);
        self.stamped(lo, hi) || (lo2.cmpge(a).all() && hi2.cmple(b).all() && lo2.cmple(floor).all() && hi2.cmpge(floor).all())
    }

    /// The columns it covers, pit and yard, inclusive.
    pub fn covers(&self) -> (IVec2, IVec2) {
        (self.pit.0.min(self.yard.0), self.pit.1.max(self.yard.1))
    }

    /// How far `p` is from what it covers: 0 on it.
    pub fn distance(&self, p: Vec2) -> f32 {
        let (lo, hi) = self.covers();
        p.distance(p.clamp(lo.as_vec2(), (hi + IVec2::ONE).as_vec2()))
    }
}

/// How far each step of a pit rises, and how far in it goes, in voxels: a stride each
/// way, so the pit is climbed in and out of by its steps.
const RISE: i32 = 2;
const TREAD: i32 = 2;
/// How far out among the settlement's land a quarry is looked for, as a share of how far
/// its land reaches beyond the houses.
const OUT: (f32, f32) = (0.1, 1.0);
/// How far the far end of the yard is from the nearest way, to the edge of its walk.
const WAY: (f32, f32) = (m(4.0), m(70.0));
/// What the pit and the yard keep clear of: the edge of any trail or other building's
/// ground, further than a site's levelling reaches, as a barn's does.
const CLEAR: f32 = m(6.5);
/// And of the edge of a river.
const RIVER: f32 = m(10.0);
/// How far apart two quarries are, at the least.
const APART: f32 = m(120.0);
/// Places looked at about each settlement, and the most of them looked at closely.
const LOOKS: u32 = 128;
const TRIES: u32 = 48;
/// How far the yard runs out from the mouth, and how far across it either side of the
/// line out of the mouth.
const YARD: (i32, i32) = (22, 13);
/// How far out the rings the rock about a site is looked for on are.
const RINGS: [i32; 4] = [30, 44, 64, 90];

/// The pit of a quarry, in a frame of its own: `u` along the way out of the mouth, `v`
/// across it.
#[derive(Clone, Copy, Debug)]
struct Pit {
    /// The middle of the floor, and the way out of the mouth.
    at: IVec2,
    mouth: IVec2,
    /// The floor's half length along `u` and half width across.
    long: i32,
    wide: i32,
    /// How many steps up its sides.
    steps: i32,
    /// How far out from `at` it is cut: the mouth runs out to here.
    reach: i32,
    /// The floor's top layer.
    floor: i32,
}

impl Pit {
    fn across(&self) -> IVec2 {
        IVec2::new(-self.mouth.y, self.mouth.x)
    }

    fn local(&self, q: IVec2) -> (i32, i32) {
        let d = q - self.at;
        (d.dot(self.mouth), d.dot(self.across()))
    }

    fn world(&self, u: i32, v: i32) -> IVec2 {
        self.at + self.mouth * u + self.across() * v
    }

    /// The step the column `q` is cut to, 0 the floor and the mouth running out from it;
    /// None where it is not cut.
    fn step(&self, q: IVec2) -> Option<i32> {
        let (u, v) = self.local(q);
        if u > self.reach {
            return None;
        }
        let du = (-u - self.long).max(0);
        let dv = (v.abs() - self.wide).max(0);
        let s = ((((du * du + dv * dv) as f32).sqrt()) / TREAD as f32).ceil() as i32;
        (s <= self.steps).then_some(s)
    }

    /// Whether column `q` is in the yard, at `u` from `reach - 1` out along the mouth.
    fn in_yard(&self, q: IVec2) -> bool {
        let (u, v) = self.local(q);
        (self.reach - 1..=self.reach - 1 + YARD.0).contains(&u) && v.abs() <= YARD.1
    }

    /// The columns from `(u0, v0)` to `(u1, v1)` of its frame, continent-local and
    /// inclusive.
    fn rect(&self, (u0, v0): (i32, i32), (u1, v1): (i32, i32)) -> (IVec2, IVec2) {
        let (a, b) = (self.world(u0, v0), self.world(u1, v1));
        (a.min(b), a.max(b))
    }

    fn cut(&self) -> (IVec2, IVec2) {
        let s = self.steps * TREAD;
        self.rect((-self.long - s, -self.wide - s), (self.reach, self.wide + s))
    }

    fn yard(&self) -> (IVec2, IVec2) {
        self.rect((self.reach - 1, -YARD.1), (self.reach - 1 + YARD.0, YARD.1))
    }
}

/// A quarry chosen and built, before it is stamped into the world.
struct Chosen {
    quarry: Quarry,
    pads: Vec<Pad>,
    voxels: Vec<(IVec3, Voxel)>,
    fine: Vec<PlantBox>,
    shed: (Vec<(IVec3, Voxel)>, Vec<PlantBox>),
    track: Vec<TrailPoint>,
}

/// Finds every settlement of continent `c` that keeps a quarry a site for it, and raises
/// it: cuts its pit, levels its yard, stamps it and its shed into the trails' network
/// as the village's buildings are, lays its track, and keeps it in
/// [`Land::quarries`](super::Land). For a continent whose barns are raised; `seed` is the
/// world's.
pub fn raise(c: &mut Continent, seed: u32) {
    let farms: Vec<usize> = (0..c.land.farms.len()).collect();
    let chosen: Vec<Option<Chosen>> = in_parallel(&farms, |&k| choose(c, seed, k));
    let mut quarries: Vec<Quarry> = Vec::new();
    for q in chosen.into_iter().flatten() {
        // Two neighbours' quarries stand apart.
        let at = |q: &Quarry| vec2(q.floor.x as f32, q.floor.z as f32);
        if quarries.iter().any(|o| at(o).distance(at(&q.quarry)) < APART) {
            continue;
        }
        let Chosen { quarry, pads, voxels, fine, shed, track } = q;
        c.trails.pads.extend(pads);
        c.trails.add_stamp_with(voxels, fine);
        c.trails.add_stamp_with(shed.0, shed.1);
        c.trails.add_points(&track);
        quarries.push(quarry);
    }
    c.land.quarries = quarries;
}

/// Whether settlement `k` keeps a quarry: one of stone country (it lives by it in part)
/// that is a large village or bigger, or an ancient village.
fn keeps(c: &Continent, k: usize) -> bool {
    let s = &c.trails.settlements[k];
    s.has(Livelihood::Quarry) && (s.kind >= Kind::LargeVillage || (s.age == Age::Ancient && s.kind >= Kind::Village))
}

/// How far `p` is from the columns `lo` to `hi`, inclusive: 0 within them.
fn off((lo, hi): (IVec2, IVec2), p: Vec2) -> f32 {
    p.distance(p.clamp(lo.as_vec2(), (hi + IVec2::ONE).as_vec2()))
}

/// The quarry of settlement `k` of `c`, on the best of the sites looked at.
fn choose(c: &Continent, seed: u32, k: usize) -> Option<Chosen> {
    if !keeps(c, k) {
        return None;
    }
    let land = &c.land;
    let farm = &land.farms[k];
    let old = farm.age == Age::Ancient;
    let steps = if farm.kind >= Kind::Town { 3 } else { 2 } + old as i32;
    let (long, wide) = (3 + steps, 4 + steps);
    let s = steps * TREAD;
    let reach = ((((long + s) * (long + s) + (wide + s) * (wide + s)) as f32).sqrt()).ceil() as i32 + 1;
    let room = (reach + YARD.0) as f32;
    let turn0 = unit(hash2(farm.seed ^ 0x9a44, k as i32, 1)) * std::f32::consts::TAU;
    let mut best: Option<(f32, Pit, Vec<TrailPoint>)> = None;
    let mut tried = 0;
    for i in 0..LOOKS {
        if tried >= TRIES {
            break;
        }
        let u = unit(hash2(farm.seed ^ 0x9a45, i as i32, 2));
        let angle = turn0 + i as f32 * 2.399_963;
        let p = farm.at + Vec2::from_angle(angle) * (farm.radius + farm.reach * (OUT.0 + (OUT.1 - OUT.0) * u));
        let (lx, lz) = (p.x.floor() as i32, p.y.floor() as i32);
        if land.owner(lx, lz).is_some_and(|o| o != k) || land.managed(lx, lz) >= CROPS || land.yard(lx, lz) {
            continue;
        }
        // Well away from every settlement's houses and buildings, and from the barns.
        if land.farms.iter().any(|f| f.at.distance(p) < f.radius + m(30.0) + room) {
            continue;
        }
        if c.trails.plots.iter().any(|pl| vec2(pl.origin.x as f32, pl.origin.z as f32).distance(p) < m(30.0) + room) {
            continue;
        }
        if land.steads.iter().any(|b| off(b.footprint, p) < room + m(8.0)) {
            continue;
        }
        let far = room + WAY.1 + m(4.0);
        let (points, _) = c.trails.gather(p - Vec2::splat(far), p + Vec2::splat(far));
        if !points.iter().any(|t| t.half_width > 0.0) {
            continue;
        }
        tried += 1;
        if let Some((score, pit, track)) = site(c, seed, k, p.floor().as_ivec2(), (long, wide, steps, reach), &points)
            && best.as_ref().is_none_or(|b| score > b.0)
        {
            best = Some((score, pit, track));
        }
    }
    let (_, pit, track) = best?;
    Some(build(c, seed, k, pit, track))
}

/// Whether column `col` at `q` will do for a quarry's pit or yard: dry ground of the
/// land or loose stone, clear of every trail, building, river and yard.
fn sound(c: &Continent, col: &Column, q: IVec2) -> bool {
    !is_wet(col) && col.water_surface().is_none() && col.river >= RIVER && col.clear >= CLEAR && !c.land.yard(q.x, q.y) && matches!(col.cover, Cover::Land | Cover::Scree)
}

/// Whether the quarry would stand about `at`, with a floor `long` by `wide` either way
/// and `steps` up its sides, cut out to `reach`: if so, how well it would do (the rockier
/// and steeper, and the nearer the way, the better), its pit, and its track's points.
fn site(c: &Continent, seed: u32, k: usize, at: IVec2, (long, wide, steps, reach): (i32, i32, i32, i32), points: &[TrailPoint]) -> Option<(f32, Pit, Vec<TrailPoint>)> {
    let half = (reach + YARD.0 + 3).max(RINGS[3] + 2);
    let lo = at - IVec2::splat(half);
    let sampler = Sampler::new(c, seed, lo.x, lo.y, 2 * half + 1);
    let col = |q: IVec2| sampler.column(q.x, q.y);
    // The ground over the pit, every other column, and the lowest and highest of it.
    let (mut low, mut high) = (i32::MAX, i32::MIN);
    let r = reach + 2;
    for dz in (-r..=r).step_by(2) {
        for dx in (-r..=r).step_by(2) {
            if dx * dx + dz * dz > r * r {
                continue;
            }
            let q = at + IVec2::new(dx, dz);
            let cl = col(q);
            if !sound(c, &cl, q) {
                return None;
            }
            low = low.min(cl.top());
            high = high.max(cl.top());
        }
    }
    let relief = high - low;
    if relief < 5 || relief > steps * RISE + 10 {
        return None;
    }
    // It opens downhill: out the side whose edge is lowest.
    let mouth = [IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y].into_iter().min_by_key(|&a| col(at + a * reach).top()).expect("four sides");
    let pit = Pit { at, mouth, long, wide, steps, reach, floor: low };
    // The yard before the mouth: sound, and near enough the floor's level to be levelled
    // to it.
    let (ylo, yhi) = pit.yard();
    for z in (ylo.y..=yhi.y).step_by(2) {
        for x in (ylo.x..=yhi.x).step_by(2) {
            let q = IVec2::new(x, z);
            let cl = col(q);
            if !sound(c, &cl, q) || !(-6..=4).contains(&(cl.top() - low)) {
                return None;
            }
        }
    }
    // Nothing else is stamped on the ground.
    let (clo, chi) = pit.cut();
    let (alo, ahi) = (clo.min(ylo) - IVec2::splat(4), chi.max(yhi) + IVec2::splat(4));
    let chunk = voxel::CHUNK;
    for cz in alo.y.div_euclid(chunk)..=ahi.y.div_euclid(chunk) {
        for cx in alo.x.div_euclid(chunk)..=ahi.x.div_euclid(chunk) {
            if c.trails.stamps_at(cx, cz).any(|t| t.hi.x >= alo.x && t.lo.x <= ahi.x && t.hi.z >= alo.y && t.lo.z <= ahi.y) {
                return None;
            }
        }
    }
    // Rock breaking the ground near: cliffs and scree on rings about the pit, or a
    // hillside steep enough that the rock is near under it, as steep ground is counted
    // stone country (`place`'s livelihoods), less steep in country of bare rock.
    let mut rock = 0;
    for r in RINGS {
        for a in 0..24 {
            let q = at + (Vec2::from_angle(a as f32 / 24.0 * std::f32::consts::TAU) * r as f32).round().as_ivec2();
            rock += matches!(col(q).cover, Cover::Cliff | Cover::Scree) as i32;
        }
    }
    let rocky = matches!(col(at).biome, Biome::Moor | Biome::RedRock | Biome::Volcanic | Biome::Alpine);
    if rock == 0 && relief < if rocky { 6 } else { 8 } {
        return None;
    }
    let track = track(c, seed, k, &pit, points)?;
    let long_way = track.len() as f32 * 2.0;
    let score = rock as f32 * 3.0 + relief as f32 + if rocky { 6.0 } else { 0.0 } - long_way / m(10.0);
    Some((score, pit, track))
}

/// The cart track from the far end of the yard of `pit` to the nearest way among
/// `points`, out ahead of the pit: dry, off any river, yard, building or barn, and the
/// height of the ground along it, drawn to the yard's at its start and the way's at
/// its end. None if there is no such way near enough.
fn track(c: &Continent, seed: u32, k: usize, pit: &Pit, points: &[TrailPoint]) -> Option<Vec<TrailPoint>> {
    let start = pit.world(pit.reach - 1 + YARD.0, 0).as_vec2() + 0.5;
    let (d, way) = points
        .iter()
        .filter(|t| t.half_width > 0.0)
        .map(|t| (vec2(t.x, t.z).distance(start) - t.half_width, t))
        .min_by(|a, b| a.0.total_cmp(&b.0))?;
    let to = vec2(way.x, way.z);
    let ahead = (to - pit.at.as_vec2()).dot(pit.mouth.as_vec2());
    if !(WAY.0..=WAY.1).contains(&d) || ahead < (pit.reach + YARD.0) as f32 {
        return None;
    }
    // To the edge of the way's walk.
    let end = to + (start - to).normalize_or_zero() * way.half_width * 0.5;
    let n = (start.distance(end) / 2.0).ceil().max(2.0) as usize;
    let pts: Vec<Vec2> = (0..=n).map(|i| start.lerp(end, i as f32 / n as f32)).collect();
    let cols = sample(c, seed, &pts);
    let land = &c.land;
    for (p, col) in pts.iter().zip(&cols) {
        let q = p.floor().as_ivec2();
        if is_wet(col) || col.river < m(2.4) || land.yard(q.x, q.y) || land.owner(q.x, q.y).is_some_and(|o| o != k) {
            return None;
        }
        if land.farms.iter().any(|f| f.at.distance(*p) < f.radius) || land.steads.iter().any(|b| off(b.footprint, *p) < m(1.6)) {
            return None;
        }
        if c.trails.plots.iter().any(|pl| off((pl.lo, pl.hi), *p) < m(1.6)) || p.distance(pit.at.as_vec2()) < (pit.reach + 2) as f32 {
            return None;
        }
    }
    let heights = profile(&cols, &[(0, pit.floor as f32 + 0.5), (n, way.y)]);
    Some(pts.iter().zip(heights).map(|(p, y)| TrailPoint { x: p.x, z: p.y, y, half_width: m(0.6) }).collect())
}

/// A fine box from cell `lo` to `hi`, continent-local, not swaying.
fn bx(lo: IVec3, hi: IVec3, v: Voxel) -> PlantBox {
    let (a, b) = (lo.min(hi), lo.max(hi));
    PlantBox { lo: a.to_array(), hi: b.to_array(), voxel: v, sway_lo: false, sway_hi: false, thin: 0, season: 0, flap: 0 }
}

/// Builds settlement `k`'s quarry on `pit`, with its cart track `track`.
fn build(c: &Continent, seed: u32, k: usize, mut pit: Pit, track: Vec<TrailPoint>) -> Chosen {
    let farm = &c.land.farms[k];
    let s = &c.trails.settlements[k];
    let r = pit.reach + 2;
    let lo = pit.at - IVec2::splat(r + YARD.0 + 2);
    let sampler = Sampler::new(c, seed, lo.x, lo.y, 2 * (r + YARD.0 + 2) + 1);
    // The floor at the lowest of the ground it is cut over, every column of it.
    let disc: Vec<IVec2> = (-r..=r).flat_map(|dz| (-r..=r).map(move |dx| IVec2::new(dx, dz))).filter(|d| d.length_squared() <= r * r).map(|d| pit.at + d).collect();
    let cols: Vec<Column> = disc.iter().map(|q| sampler.column(q.x, q.y)).collect();
    pit.floor = disc.iter().zip(&cols).filter(|(q, _)| (*q - pit.at).length_squared() <= pit.reach * pit.reach).map(|(_, c)| c.top()).min().unwrap_or(pit.floor);
    let f = pit.floor;
    let side = if hash2(farm.seed ^ 0x9a46, k as i32, 3) % 2 == 0 { 1 } else { -1 };
    let roll = |q: IVec3, salt: u32| unit(hash3(farm.seed ^ salt, q.x, q.y, q.z));
    let mut voxels: Vec<(IVec3, Voxel)> = Vec::new();
    let mut fine: Vec<PlantBox> = Vec::new();
    // The pit: each column cut down to its step, or where the ground stands lower than
    // its step, built back as it was; the floor laid with chips and rubble. The pit's
    // pad has lowered everything within it to the floor.
    let mut depth = 0;
    for (&q, col) in disc.iter().zip(&cols) {
        if pit.in_yard(q) {
            continue;
        }
        let g = col.top();
        depth = depth.max(g - f);
        let rock = |y: i32| local_rock(seed, col.biome, q.x, y, q.y);
        let natural = if col.cover == Cover::Scree { mat::SCREE } else { mat::ground_cover(col.biome) };
        let (h, cut) = match pit.step(q) {
            Some(0) => {
                let at = IVec3::new(q.x, f, q.y);
                let r = roll(at, 0x9a47);
                voxels.push((at, if r < 0.45 { mat::GRAVEL } else if r < 0.8 { rock(f) } else { mat::SCREE }));
                continue;
            }
            Some(s) => (g.min(f + RISE * s), g > f + RISE * s),
            None => (g, false),
        };
        for y in f + 1..=h {
            let at = IVec3::new(q.x, y, q.y);
            let v = if y < h {
                rock(y)
            } else if !cut {
                natural
            } else if roll(at, 0x9a48) < 0.3 {
                mat::GRAVEL
            } else {
                rock(y)
            };
            voxels.push((at, v));
        }
    }
    let local = local_rock(seed, farm.biome, pit.at.x, f, pit.at.y);
    let (dressed, dressed_dark) = mat::masonry(local);
    let up = |q: IVec2, y: i32| IVec3::new(q.x, y, q.y);
    // A heap of spoil, `high` voxels at its middle at `q`, on ground at `y`.
    let heap = |voxels: &mut Vec<(IVec3, Voxel)>, at: IVec2, y: i32, high: i32| {
        for dz in -3..=3 {
            for dx in -3..=3 {
                let h = high - ((dx * dx + dz * dz) as f32).sqrt().round() as i32;
                for k in 1..=h {
                    let q = up(at + IVec2::new(dx, dz), y + k);
                    let r = roll(q, 0x9a49);
                    voxels.push((q, if r < 0.45 { mat::SCREE } else if r < 0.8 { mat::GRAVEL } else { local }));
                }
            }
        }
    };
    // On the floor: spoil heaped in a back corner, a few blocks lying cut, and the crane
    // at the back lifting another.
    heap(&mut voxels, pit.world(-pit.long + 2, -side * (pit.wide - 2)), f, 3);
    for (u, v) in [(-pit.long + 3, side * (pit.wide - 2)), (-pit.long + 3, side * (pit.wide - 4))] {
        let q = pit.world(u, v);
        voxels.push((up(q, f + 1), dressed));
        voxels.push((up(q + pit.mouth, f + 1), dressed));
    }
    crane(&mut fine, &pit, f, dressed, farm.wood, s.seed);
    // The yard: blocks stacked ready in rows, a handcart by them, and the spoil heaped at
    // its far end.
    for (i, u) in [pit.reach + 2, pit.reach + 5, pit.reach + 8].into_iter().enumerate() {
        let layers = 2 + (hash2(farm.seed ^ 0x9a4a, i as i32, 0) % 2) as i32;
        for y in 1..=layers {
            for (du, dv) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let q = pit.world(u + du, -side * (5 + dv));
                voxels.push((up(q, f + y), if (y + du) % 2 == 0 { dressed } else { dressed_dark }));
            }
        }
    }
    let pal = palette_at(c, seed, farm.biome, s.x, s.y - 1, s.z).varied(farm.seed ^ 0x9a57);
    let cart = fittings::build(Fitting::Handcart.local(farm.region), &pal, farm.seed ^ 0x9a4b, false).place(up(pit.world(pit.reach + 12, -side * 6), f + 1), turns_toward(pit.mouth.as_vec2()));
    voxels.extend(cart.voxels);
    fine.extend(cart.fine);
    heap(&mut voxels, pit.world(pit.reach + YARD.0 - 4, -side * 9), f, 3);
    // The mason's shed on the other side of the yard, its door to the line out of the
    // mouth.
    let spec = BarnSpec {
        kind: BarnKind::Mason,
        region: farm.region,
        age: (match farm.age {
            Age::Young => 0.2,
            Age::Established => 0.45,
            Age::Ancient => 0.7,
        } + unit(hash2(farm.seed ^ 0x9a4c, k as i32, 4)) * 0.2),
        wealth: (farm.prosperity * 0.8 + unit(hash2(farm.seed ^ 0x9a4d, k as i32, 5)) * 0.2).clamp(0.0, 1.0),
        seed: farm.seed ^ 0x9a4e,
    };
    let shed = barn::build(&spec, &pal);
    let centre = pit.world(pit.reach + 8, side * 7);
    let turns = turns_toward((-pit.across() * side).as_vec2());
    let origin = IVec3::new(centre.x, f + 1, centre.y);
    let turned = |(lo, hi): (IVec2, IVec2)| {
        let (a, b) = (rotate(IVec3::new(lo.x, 0, lo.y), turns), rotate(IVec3::new(hi.x, 0, hi.y), turns));
        (centre + IVec2::new(a.x.min(b.x), a.z.min(b.z)), centre + IVec2::new(a.x.max(b.x), a.z.max(b.z)))
    };
    let b = &shed.building;
    let shed_voxels: Vec<(IVec3, Voxel)> = b.voxels.iter().map(|&(q, v)| (origin + rotate(q, turns), v)).collect();
    let shed_fine = parts::place_fine(&b.fine, origin, turns);
    let facing = rotate(b.door.facing, turns);
    // The pads: the pit's walled, all of it at the floor; the yard's levelled to it and
    // trodden about its middle.
    let mut pads = vec![Pad { walled: true, ..Pad::plain(pit.at.x as f32 + 0.5, pit.at.y as f32 + 0.5, pit.reach as f32 + 0.5, f as f32) }];
    pads.extend(cover(pit.yard()).into_iter().map(|(at, r)| {
        let mut pad = Pad::plain(at.x, at.y, r, f as f32);
        pad.wear = 0.35;
        pad.hub = r * 0.5;
        pad
    }));
    let quarry = Quarry {
        owner: k,
        floor: IVec3::new(pit.at.x, f + 1, pit.at.y),
        mouth: pit.mouth,
        pit: pit.cut(),
        yard: pit.yard(),
        depth,
        origin,
        turns,
        walls: turned((b.walls.lo, b.walls.hi)),
        footprint: turned((b.footprint.lo, b.footprint.hi)),
        door: origin + rotate(b.door.floor, turns),
        facing,
        track: track.iter().map(|t| vec2(t.x, t.z)).collect(),
    };
    Chosen { quarry, pads, voxels, fine, shed: (shed_voxels, shed_fine), track }
}

/// A crane standing on the floor of `pit` (its top layer `f`) at the middle of the back:
/// a post braced to a jib reaching out toward the mouth, and from its end a rope down to
/// a block of `stone` hung a hand over the floor.
fn crane(fine: &mut Vec<PlantBox>, pit: &Pit, f: i32, stone: Voxel, wood: Voxel, seed: u32) {
    let foot = pit.world(-pit.long + 1, 0);
    let base = IVec3::new(foot.x * 4, (f + 1) * 4, foot.y * 4);
    let (m, a) = (pit.mouth, pit.across());
    // A point `along` cells out toward the mouth and `out` across from the middle of the
    // post's column, `y` cells up.
    let at = |along: i32, across: i32, y: i32| base + IVec3::new(2 + m.x * along + a.x * across, y, 2 + m.y * along + a.y * across);
    let top = 40 + (seed % 3) as i32 * 2;
    // The post, and the timbers it stands on crossed under it.
    fine.push(bx(at(-1, -1, 0), at(1, 1, top + 2), wood));
    fine.push(bx(at(-4, -1, 0), at(5, 1, 2), wood));
    fine.push(bx(at(-1, -4, 0), at(1, 5, 2), wood));
    // The jib, and the brace under it in steps up from the post.
    fine.push(bx(at(1, -1, top), at(17, 1, top + 2), wood));
    for i in 0..5 {
        fine.push(bx(at(1 + 2 * i, 0, top - 12 + 2 * i), at(3 + 2 * i, 1, top - 9 + 2 * i), wood));
    }
    // The rope from the end of the jib, the hook, and the block on it.
    let block = (f + 2) * 4 - 1;
    fine.push(bx(at(15, 0, block + 4), at(16, 1, top), mat::ROPE));
    fine.push(bx(at(15, 0, block + 3), at(16, 1, block + 4), mat::IRON));
    fine.push(bx(at(13, -2, block), at(18, 2, block + 3), stone));
    // A winch at the post's foot to haul on.
    fine.push(bx(at(1, -3, 5), at(3, 3, 7), wood));
}

#[cfg(test)]
mod tests {
    use voxel::{AIR, CHUNK, PAD};

    use super::*;
    use crate::{World, continent::SimConfig, world::GameWorld};

    fn world(seed: u32) -> GameWorld {
        GameWorld::with_config(seed, SimConfig { size: 256 })
    }

    /// Small worlds with a quarry or more.
    const SEEDS: [u32; 4] = [2, 6, 7, 8];

    /// Quarries are cut where a settlement that lives by stone would keep one, out in its
    /// land off every trail, river, yard, building and barn, their floors open and their
    /// faces standing, and their sheds walked into through their doors; none of them is
    /// among the settlements' buildings.
    #[test]
    fn quarries_stand_off_trails_rivers_and_plots_and_are_walked_into() {
        let mut found = 0;
        for seed in SEEDS {
            let w = world(seed);
            let c = w.continent(0, 0);
            for q in &c.land.quarries {
                found += 1;
                let s = &c.trails.settlements[q.owner];
                assert!(s.has(Livelihood::Quarry), "seed {seed}: a quarry of a place that does not live by stone");
                let farm = &c.land.farms[q.owner];
                let (lo, hi) = q.covers();
                let mid = (lo + hi).as_vec2() * 0.5;
                assert!(farm.at.distance(mid) > farm.radius + m(20.0), "seed {seed}: a quarry at {mid} is in among the houses");
                // Off the trails, its own track but: the nearest trail's walk well clear.
                let (points, _) = c.trails.gather(lo.as_vec2() - 8.0, hi.as_vec2() + 8.0);
                for t in points.iter().filter(|t| t.half_width > 0.0 && !q.track.contains(&vec2(t.x, t.z))) {
                    assert!(q.distance(vec2(t.x, t.z)) - t.half_width > m(4.8), "seed {seed}: a trail at {},{} by the quarry at {mid}", t.x, t.z);
                }
                // Off the rivers and out of the yards.
                let span = (hi - lo).max_element() + 1;
                let sampler = Sampler::new(&c, w.seed(), lo.x, lo.y, span);
                for z in (lo.y..=hi.y).step_by(2) {
                    for x in (lo.x..=hi.x).step_by(2) {
                        let col = sampler.column(x, z);
                        assert!(col.water_surface().is_none() && col.river > m(5.0), "seed {seed}: water at {x},{z} in the quarry at {mid}");
                        assert!(!c.land.yard(x, z), "seed {seed}: a yard at {x},{z} in the quarry at {mid}");
                    }
                }
                // Clear of the buildings and the barns, and not one of them.
                for p in &c.trails.plots {
                    assert!(q.distance(vec2(p.origin.x as f32, p.origin.z as f32)) > m(30.0), "seed {seed}: the quarry at {mid} by a building at {}", p.origin);
                }
                for b in &c.land.steads {
                    assert!(off(b.footprint, mid) > m(10.0) && q.distance(((b.walls.0 + b.walls.1).as_vec2()) * 0.5) > m(4.0), "seed {seed}: the quarry at {mid} by a barn");
                }
                // The pit is cut: its floor open over the middle, its back face standing.
                let get = |q: IVec3| {
                    let (cx, cz) = (q.x.div_euclid(CHUNK), q.z.div_euclid(CHUNK));
                    let column = w.generate_column(cx, cz);
                    let (cy, ly) = (q.y.div_euclid(CHUNK), q.y.rem_euclid(CHUNK));
                    column.chunks.iter().find(|(k, _)| *k == cy).map(|(_, ch)| ch.get((q.x - cx * CHUNK + 1) as usize, ly as usize + 1, (q.z - cz * CHUNK + 1) as usize))
                };
                for y in 0..4 {
                    assert_eq!(get(q.floor + IVec3::Y * y), Some(AIR), "seed {seed}: the floor of the quarry at {mid} is filled at {}", q.floor + IVec3::Y * y);
                }
                assert!(get(q.floor - IVec3::Y).is_some_and(|v| v != AIR), "seed {seed}: no floor under the quarry at {mid}");
                assert!(q.depth >= 5, "seed {seed}: the quarry at {mid} is cut only {} deep", q.depth);
                // The shed's stamp is known for its own, and walked into through its door.
                for y in 1..=4 {
                    assert_eq!(get(q.door + IVec3::Y * y), Some(AIR), "seed {seed}: the mason's shed at {} has its door blocked", q.door);
                }
                let mut near: Vec<&crate::trails::Stamp> = Vec::new();
                for cz in q.footprint.0.y.div_euclid(CHUNK) - 1..=q.footprint.1.y.div_euclid(CHUNK) + 1 {
                    for cx in q.footprint.0.x.div_euclid(CHUNK) - 1..=q.footprint.1.x.div_euclid(CHUNK) + 1 {
                        for t in c.trails.stamps_at(cx, cz) {
                            if !near.iter().any(|&n| std::ptr::eq(n, t)) {
                                near.push(t);
                            }
                        }
                    }
                }
                assert_eq!(near.iter().filter(|t| q.stamped(t.lo, t.hi)).count(), 1, "seed {seed}: the mason's shed at {} is not its own stamp", q.origin);
                // The yard and the shed stand off the pit, the shed in the yard.
                assert!(q.footprint.0.cmpge(q.yard.0 - IVec2::splat(2)).all() && q.footprint.1.cmple(q.yard.1 + IVec2::splat(2)).all(), "seed {seed}: the shed stands out of its yard");
            }
        }
        assert!(found >= 3, "only {found} quarries on the test worlds");
    }

    /// The quarries are the same however often the continent is made.
    #[test]
    fn quarries_are_the_same_every_time() {
        for seed in SEEDS {
            assert_eq!(world(seed).continent(0, 0).land.quarries, world(seed).continent(0, 0).land.quarries);
        }
    }

    /// A quarry's pit is the same seen from either chunk of a border through it.
    #[test]
    fn quarries_agree_across_chunk_borders() {
        let mut compared = 0;
        for seed in SEEDS {
            let w = world(seed);
            let c = w.continent(0, 0);
            for q in &c.land.quarries {
                let (lo, hi) = q.pit;
                let (cx0, cz0) = (lo.x.div_euclid(CHUNK), lo.y.div_euclid(CHUNK));
                let pairs = [((cx0, cz0), (cx0 + 1, cz0), true), ((cx0, cz0), (cx0, cz0 + 1), false)];
                for (a0, b0, along_x) in pairs {
                    if along_x && (hi.x.div_euclid(CHUNK) == cx0) || !along_x && (hi.y.div_euclid(CHUNK) == cz0) {
                        continue;
                    }
                    let (a, b) = (w.generate_column_bare(a0.0, a0.1).chunks, w.generate_column_bare(b0.0, b0.1).chunks);
                    for (cy, ca) in &a {
                        let Some((_, cb)) = b.iter().find(|e| e.0 == *cy) else { continue };
                        for (i, j) in [(PAD - 2, 0), (PAD - 1, 1)] {
                            for y in 0..PAD {
                                for o in 0..PAD {
                                    let (va, vb) = if along_x { (ca.get(i, y, o), cb.get(j, y, o)) } else { (ca.get(o, y, i), cb.get(o, y, j)) };
                                    assert_eq!(va, vb, "seed {seed}: chunks {a0:?} and {b0:?} differ at {i} y {y} o {o} cy {cy}");
                                }
                            }
                        }
                    }
                    compared += 1;
                }
            }
        }
        assert!(compared > 0, "no quarry straddles a chunk border");
    }

    /// Lists the quarries of a full-size world (seed from `SEED`, 1 if unset), for aiming
    /// screenshots: `cargo test -p worldgen list_quarries -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn list_quarries() {
        let seed = std::env::var("SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
        let mut c = crate::continent::generate(crate::macro_layout::cell_desc(seed, 0, 0), SimConfig::default());
        c.trails = crate::trails::build(&c, seed);
        c.land = super::super::influence::build(&c, seed);
        super::super::farmstead::raise(&mut c, seed);
        let keepers = (0..c.land.farms.len()).filter(|&k| keeps(&c, k)).count();
        for s in c.trails.settlements.iter().filter(|s| s.has(Livelihood::Quarry)) {
            println!("lives by stone: {:?} {:?} {:?} at {},{}", s.age, s.kind, s.region, s.x, s.z);
        }
        let start = std::time::Instant::now();
        raise(&mut c, seed);
        println!("{} quarries for {keepers} settlements that would keep one, in {:.1} ms", c.land.quarries.len(), start.elapsed().as_secs_f64() * 1000.0);
        for q in &c.land.quarries {
            let home = &c.trails.settlements[q.owner];
            println!("quarry of {:?} {:?} {:?} at {},{}: floor {} mouth {} depth {} shed door {} facing {} track of {}", home.age, home.kind, home.region, home.x, home.z, q.floor, q.mouth, q.depth, q.door, q.facing, q.track.len());
        }
    }
}
