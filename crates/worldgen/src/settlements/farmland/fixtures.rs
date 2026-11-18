//! What stands about the fields (#64): haystacks in the meadows, scarecrows in the
//! corn, a shelter for the beasts on a pasture with its trough, and now and then a
//! dovecote, a well or a row of skeps by a barn or at a field's edge.
//!
//! - Haystacks stand a few together along a hay meadow's rows. They are stacked at the
//!   hay-making and stand whole through the autumn; in the winter they are cut into for
//!   fodder, and by the spring they are gone and the meadow grows again. In the north and
//!   the mountains the hay is built round a pole instead.
//! - A scarecrow stands in a field while it carries wheat, barley or rye, from the sowing
//!   to the harvest, so it moves with the field's rotation ([`rotation::crop`]).
//! - A pasture may have a field shelter, open all along its front (`BarnKind::Shelter`),
//!   with a trough beside it; or a trough down by the water where a stream runs by.
//! - A dovecote, a well, or a row of skeps whose bees work the flowers about. The skeps
//!   come and go with the settlement's own hives ([`Lives::hives`]), and their bees fly
//!   as the wild ones do (`GameWorld::hives_near`).
//!
//! Year 0 is today's world in every season (see `years::When::seen`): the hay is
//! stacked, and every field carries its own crop.
//!
//! Each stands on dry, worked ground inside one field, clear of its hedges and walls and
//! of every trail, building, river, quarry and terrace. The shelter's ground is levelled
//! as a barn's is. The rest sit on the ground as it lies, which is why they keep to
//! ground that hardly rises under them and well away from any levelling. The shelters,
//! being buildings, are stamped into the trails' network with the barns. The rest are
//! kept here, each part with the seasons or years it stands in and the ground it keeps
//! clear of crops and grass (the hay is mown about a stack), and are found by chunk
//! ([`Fixtures::at`]), so a chunk pays one lookup for them.

use std::collections::HashMap;

use glam::{IVec2, IVec3, Vec2};
use voxel::{
    CHUNK, Voxel,
    mesh::{CELLS, PlantBox},
};

use super::{
    Crop, Plot, farmstead,
    fields::worked,
    influence::{CROPS, EDGE, Farm},
    plot, rotation,
};
use crate::{
    continent::Continent,
    features::{
        fittings::{self, Fitted, Fitting, Hook},
        landmarks::Palette,
        nests::{self, Nest, NestKind},
        parts,
        shape::rotate,
    },
    noise::{hash2, unit},
    scale::m,
    settlements::{
        Age, Kind, Region,
        special::barn::{self, BarnKind, BarnSpec},
        years::Lives,
    },
    surface::Sampler,
    trails::{
        Pad, Stamp,
        build::{in_parallel, palette_at},
    },
};

/// What a fixture is.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum What {
    Haystack,
    Scarecrow,
    /// An open-fronted shelter for the beasts on a pasture.
    Shelter,
    /// A trough for the beasts to drink from, by a shelter or down by the water.
    Trough,
    Dovecote,
    Well,
    /// A row of skeps (or log hives, in the north and the old forests).
    Skeps,
}

/// When a part of a fixture stands.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shows {
    /// In every year and season.
    Always,
    /// A haystack whole: from the hay-making to the end of the autumn, and in year 0.
    Stacked,
    /// A haystack cut into for the winter's fodder.
    Cut,
    /// A scarecrow: from the spring to the autumn of a year its field carries grain.
    Grain,
    /// The hive at this place in a row: while its settlement keeps more hives than that.
    Hive(u8),
}

/// Something set out in a settlement's fields.
#[derive(Clone, Debug)]
pub struct Fixture {
    /// The settlement whose fields it is in, by its index in `Network::settlements` and
    /// [`Land::farms`](super::Land::farms).
    pub owner: usize,
    pub what: What,
    /// Where its model's foot stands, continent-local: a layer over the ground.
    pub at: IVec3,
    pub turns: u32,
    /// The columns it covers, continent-local, inclusive.
    pub area: (IVec2, IVec2),
    /// The way its front looks: a shelter's open side, a trough's or a hive's face.
    pub front: IVec2,
    /// The field it stands in.
    pub field: Plot,
    /// The water in a trough or a well, continent-local: where a beast or a person
    /// drinks or draws.
    pub water: Option<IVec3>,
    /// A row of skeps' colonies, continent-local, in the row's order: the `n`th is there
    /// while [`Shows::Hive`]`(n)` is.
    pub hives: Vec<Nest>,
}

/// A part of a fixture that comes and goes, and when.
#[derive(Clone)]
struct Part {
    fixture: u32,
    shows: Shows,
    stamp: Stamp,
    /// The colony in it, for a hive.
    nest: Option<Nest>,
    /// The columns nothing grows on while it stands, continent-local, inclusive: what it
    /// stands on, trodden, and for a haystack the meadow mown about it.
    clear: (IVec2, IVec2),
}

/// Every fixture of a continent's fields, filed by chunk. All but the shelters, which
/// are buildings and stamped into the network with the barns, are kept here.
#[derive(Clone, Default)]
pub struct Fixtures {
    pub list: Vec<Fixture>,
    parts: Vec<Part>,
    /// The parts touching each continent-local chunk column, a voxel beyond it included.
    index: HashMap<(i32, i32), Vec<u32>>,
}

impl Fixtures {
    /// What stands now of the fixtures about continent-local chunk column `(cx, cz)`, as
    /// `lives` has the year, each with the columns nothing grows on while it does;
    /// `seed` is the world's.
    pub fn at(&self, cx: i32, cz: i32, seed: u32, lives: &Lives) -> Vec<(&Stamp, (IVec2, IVec2))> {
        self.parts_at(cx, cz).filter(|p| shows(p.shows, &self.list[p.fixture as usize], seed, lives)).map(|p| (&p.stamp, p.clear)).collect()
    }

    /// The colonies in the hives about continent-local chunk column `(cx, cz)` that are
    /// kept now, continent-local.
    pub fn hives_at(&self, cx: i32, cz: i32, seed: u32, lives: &Lives) -> Vec<Nest> {
        self.parts_at(cx, cz).filter(|p| p.nest.is_some() && shows(p.shows, &self.list[p.fixture as usize], seed, lives)).filter_map(|p| p.nest).collect()
    }

    /// Settlement `k`'s fixtures of kind `what`: where the beasts of a pasture find their
    /// shelter and their trough.
    pub fn of(&self, k: usize, what: What) -> impl Iterator<Item = &Fixture> {
        self.list.iter().filter(move |f| f.owner == k && f.what == what)
    }

    fn parts_at(&self, cx: i32, cz: i32) -> impl Iterator<Item = &Part> {
        self.index.get(&(cx, cz)).into_iter().flatten().map(|&i| &self.parts[i as usize])
    }

    fn add(&mut self, fixture: u32, shows: Shows, voxels: Vec<(IVec3, Voxel)>, fine: Vec<PlantBox>, nest: Option<Nest>, mown: i32) {
        let Some(stamp) = stamp(voxels, fine) else { return };
        let id = self.parts.len() as u32;
        let clear = grow((IVec2::new(stamp.lo.x, stamp.lo.z), IVec2::new(stamp.hi.x, stamp.hi.z)), mown);
        // The one-voxel chunk border means a part also touches chunks a voxel beyond.
        let (lo, hi) = union(clear, (IVec2::new(stamp.lo.x, stamp.lo.z), IVec2::new(stamp.hi.x, stamp.hi.z)));
        for cz in (lo.y - 1).div_euclid(CHUNK)..=(hi.y + 1).div_euclid(CHUNK) {
            for cx in (lo.x - 1).div_euclid(CHUNK)..=(hi.x + 1).div_euclid(CHUNK) {
                self.index.entry((cx, cz)).or_default().push(id);
            }
        }
        self.parts.push(Part { fixture, shows, stamp, nest, clear });
    }
}

/// Whether a part that `s` says when of stands for fixture `f` now, as `lives` has the
/// year; `seed` is the world's.
pub fn shows(s: Shows, f: &Fixture, seed: u32, lives: &Lives) -> bool {
    let when = lives.when.seen();
    let today = when.year <= 0;
    match s {
        Shows::Always => true,
        Shows::Stacked => today || matches!(when.season, 1 | 2),
        Shows::Cut => !today && when.season == 3,
        Shows::Grain => (today || when.season != 3) && grain(rotation::crop(&f.field, seed, when)),
        Shows::Hive(n) => n < lives.hives(f.owner),
    }
}

fn grain(crop: Crop) -> bool {
    matches!(crop, Crop::Wheat | Crop::Barley | Crop::Rye)
}

/// Whether a field of `crop` carries grain in some year of its rotation.
fn ever_grain(crop: Crop) -> bool {
    matches!(crop, Crop::Wheat | Crop::Barley | Crop::Rye | Crop::Roots | Crop::Flax | Crop::Ploughed)
}

/// A stamp of `voxels` and `fine`, continent-local, in one order and with its bounds,
/// as `Network::add_stamp_with` makes one.
fn stamp(mut voxels: Vec<(IVec3, Voxel)>, fine: Vec<PlantBox>) -> Option<Stamp> {
    voxels.sort_by_key(|&(p, _)| (p.y, p.z, p.x));
    let first = match (voxels.first(), fine.first()) {
        (Some(&(p, _)), _) => p,
        (None, Some(b)) => IVec3::from_array(b.lo).div_euclid(IVec3::splat(CELLS)),
        (None, None) => return None,
    };
    let (mut lo, mut hi) = (first, first);
    for (p, _) in &voxels {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    for b in &fine {
        lo = lo.min(IVec3::from_array(b.lo).div_euclid(IVec3::splat(CELLS)));
        hi = hi.max((IVec3::from_array(b.hi) - 1).div_euclid(IVec3::splat(CELLS)));
    }
    Some(Stamp { lo, hi, voxels, fine, tag: None })
}

/// How many of each a settlement sets out, by its size: a hamlet a stack and a
/// scarecrow, a great town's wide fields many.
struct Want {
    hay: usize,
    crows: usize,
    shelters: usize,
    troughs: usize,
    extras: usize,
}

fn wanted(farm: &Farm) -> Want {
    let n: usize = match farm.kind {
        Kind::Hamlet => 1,
        Kind::Village => 2,
        Kind::LargeVillage => 3,
        Kind::Town => 4,
        Kind::GreatTown => 5,
    };
    let extras = match farm.age {
        Age::Young => n / 2,
        Age::Established => n.div_ceil(2),
        Age::Ancient => n.div_ceil(2) + 1,
    };
    Want { hay: n, crows: n + 1, shelters: n.div_ceil(2), troughs: 1, extras }
}

/// Places looked at in each settlement's fields.
const LOOKS: u32 = 320;
/// How far out among its fields to look, as a share of how far they reach beyond the
/// houses.
const OUT: (f32, f32) = (0.04, 0.85);
/// What a fixture set on the ground as it lies keeps clear of: a trail's edge, and any
/// pad, past where a pad's levelling would tilt the ground under it.
const CLEAR: f32 = m(3.0);
/// And a river's edge.
const RIVER: f32 = m(4.0);
/// Columns of its own field round a fixture, between it and the field's hedge or wall.
const MARGIN: i32 = 2;
/// How far apart the fixtures raised here keep, beyond what they cover: the shelters'
/// levelled ground reaches further (see `Chosen::room`).
const APART: i32 = 3;
/// How far a trough by the water stands from the stream's edge.
const BY_WATER: (f32, f32) = (m(4.0), m(16.0));

/// A fixture chosen for a site, before it is raised.
struct Chosen {
    /// What it was chosen with, which may stand close together (a shelter and its
    /// trough).
    group: u32,
    fixture: Fixture,
    /// Columns nothing else raised here may come into, inclusive.
    room: (IVec2, IVec2),
    pads: Vec<Pad>,
    parts: Vec<(Shows, Vec<(IVec3, Voxel)>, Vec<PlantBox>, Option<Nest>)>,
}

/// Finds the fixtures of every settlement of continent `c` their sites, and raises them:
/// stamps what stays into the trails' network, levels the shelters' ground, and keeps
/// them all in [`Land::fixtures`](super::Land). For a continent whose trails, land,
/// barns, camps and quarries are built; `seed` is the world's.
pub fn raise(c: &mut Continent, seed: u32) {
    let farms: Vec<usize> = (0..c.land.farms.len()).collect();
    let chosen: Vec<Vec<Chosen>> = in_parallel(&farms, |&k| choose(c, seed, k));
    let mut out = Fixtures::default();
    let mut taken: Vec<(u32, (IVec2, IVec2))> = Vec::new();
    for ch in chosen.into_iter().flatten() {
        // Each settlement's keep apart; two neighbours' may not have.
        if taken.iter().any(|&(g, r)| g != ch.group && meets(r, ch.room)) {
            continue;
        }
        taken.push((ch.group, ch.room));
        c.trails.pads.extend(ch.pads);
        let id = out.list.len() as u32;
        // The hay is mown about a haystack, and the grass kept down or trodden about a
        // trough, a well, a dovecote and the skeps; a scarecrow stands in the corn.
        let mown = match ch.fixture.what {
            What::Haystack => 2,
            What::Scarecrow => 0,
            _ => 1,
        };
        for (shows, voxels, fine, nest) in ch.parts {
            if ch.fixture.what == What::Shelter {
                c.trails.add_stamp_with(voxels, fine);
            } else {
                out.add(id, shows, voxels, fine, nest, mown);
            }
        }
        out.list.push(ch.fixture);
    }
    c.land.fixtures = out;
}

fn meets(a: (IVec2, IVec2), b: (IVec2, IVec2)) -> bool {
    a.0.cmple(b.1).all() && b.0.cmple(a.1).all()
}

fn grow((lo, hi): (IVec2, IVec2), by: i32) -> (IVec2, IVec2) {
    (lo - IVec2::splat(by), hi + IVec2::splat(by))
}

/// The columns a fitting `kind` placed at `at` turned `turns` stands on, inclusive.
fn area_of(kind: Fitting, at: IVec3, turns: u32) -> (IVec2, IVec2) {
    let (lo, hi) = kind.room(at, turns);
    (IVec2::new(lo.x, lo.z), IVec2::new(hi.x - 1, hi.z - 1))
}

fn union(a: (IVec2, IVec2), b: (IVec2, IVec2)) -> (IVec2, IVec2) {
    (a.0.min(b.0), a.1.max(b.1))
}

/// Where the water of fitting `f`, placed in the world, is, from its hooks.
fn water(f: &Fitted) -> Option<IVec3> {
    f.hooks.iter().find_map(|h| if let Hook::Water(q) = h { Some(q.div_euclid(IVec3::splat(CELLS))) } else { None })
}

/// The fixtures of settlement `k` of `c`, each on its site.
fn choose(c: &Continent, seed: u32, k: usize) -> Vec<Chosen> {
    let land = &c.land;
    let farm = &land.farms[k];
    let mut want = wanted(farm);
    let s = &c.trails.settlements[k];
    // Built of what the place's houses are, as its barns are.
    let pal = palette_at(c, seed, farm.biome, s.x, s.y - 1, s.z).varied(farm.seed ^ 0xba57);
    let mut out: Vec<Chosen> = Vec::new();
    let mut fields: Vec<(u32, What)> = Vec::new();
    let roll = |salt: u32, a: i32, b: i32| unit(hash2(farm.seed ^ salt, a, b));
    // By the barns first, what keeps by a farmstead.
    for (n, st) in land.steads.iter().filter(|st| st.owner == k).enumerate() {
        if want.extras == 0 {
            break;
        }
        if roll(0xf1e0, n as i32, 0) < 0.4 {
            continue;
        }
        let (lo, hi) = st.footprint;
        let mid = (lo + hi) / 2;
        let side = (hash2(farm.seed ^ 0xf1e1, n as i32, 1) % 4) as i32;
        for t in 0..4 {
            let d = rotate(IVec3::X, ((side + t) % 4) as u32);
            let d = IVec2::new(d.x, d.z);
            let reach = if d.x != 0 { (hi.x - lo.x) / 2 } else { (hi.y - lo.y) / 2 };
            let p = mid + d * (reach + 6);
            let group = out.len() as u32 + (k as u32) * 4096;
            if let Some(ch) = extra(c, seed, farm, k, &pal, p.as_vec2(), d, group, &out) {
                fields.push((ch.fixture.field.id, ch.fixture.what));
                out.push(ch);
                want.extras -= 1;
                break;
            }
        }
    }
    // Then about the fields.
    let turn0 = roll(0xf1e2, k as i32, 0) * std::f32::consts::TAU;
    let mut water_tries = 12;
    for i in 0..LOOKS {
        if want.hay + want.crows + want.shelters + want.troughs + want.extras == 0 {
            break;
        }
        let u = roll(0xf1e3, i as i32, 2);
        let p = farm.at + Vec2::from_angle(turn0 + i as f32 * 2.399_963) * (farm.radius + farm.reach * (OUT.0 + (OUT.1 - OUT.0) * u));
        let (lx, lz) = (p.x.floor() as i32, p.y.floor() as i32);
        let managed = land.managed(lx, lz);
        if land.owner(lx, lz) != Some(k) || !(EDGE + 0.05..=CROPS + 0.2).contains(&managed) || land.yard(lx, lz) {
            continue;
        }
        if land.farms.iter().any(|f| f.at.distance(p) < f.radius + m(20.0)) || land.quarries.iter().any(|q| q.distance(p) < m(16.0)) {
            continue;
        }
        let Some(field) = plot(c, seed, lx, lz) else { continue };
        if field.terrace.is_some() {
            continue;
        }
        let used = |what: What| fields.iter().any(|&(id, w)| id == field.id && w == what);
        let group = out.len() as u32 + (k as u32) * 4096;
        // What was chosen here, and which of the wants it meets: a group of haystacks
        // counts once, and a shelter's own trough is not the one by the water.
        let (got, met): (Vec<Chosen>, &mut usize) = match field.crop {
            Crop::Hay if want.hay > 0 && !used(What::Haystack) => (haystacks(c, seed, farm, k, &pal, p, &field, group, &out), &mut want.hay),
            crop if ever_grain(crop) && want.crows > 0 && !used(What::Scarecrow) => (scarecrow(c, seed, farm, k, &pal, p, &field, group, &out).into_iter().collect(), &mut want.crows),
            Crop::Pasture | Crop::Rough if want.shelters > 0 && !used(What::Shelter) => (shelter(c, seed, farm, k, &pal, p, &field, group, &out), &mut want.shelters),
            Crop::Pasture | Crop::Rough if want.troughs > 0 && water_tries > 0 && !used(What::Trough) => {
                water_tries -= 1;
                (by_water(c, seed, farm, k, &pal, p, &field, group, &out).into_iter().collect(), &mut want.troughs)
            }
            Crop::Orchard | Crop::Vines | Crop::Olives | Crop::Coppice => continue,
            _ if want.extras > 0 && near_edge(c, seed, p, field.id) => {
                let d = rotate(IVec3::X, hash2(farm.seed ^ 0xf1e4, i as i32, 3) % 4);
                (extra(c, seed, farm, k, &pal, p, IVec2::new(d.x, d.z), group, &out).into_iter().collect(), &mut want.extras)
            }
            _ => continue,
        };
        if !got.is_empty() {
            *met -= 1;
        }
        for ch in got {
            fields.push((ch.fixture.field.id, ch.fixture.what));
            out.push(ch);
        }
    }
    out
}

/// Whether continent-local point `p` in field `id` is near its edge: another field, or
/// none, a few metres off one way or another.
fn near_edge(c: &Continent, seed: u32, p: Vec2, id: u32) -> bool {
    let d = m(6.0);
    [(d, 0.0), (-d, 0.0), (0.0, d), (0.0, -d)].iter().any(|&(dx, dz)| plot(c, seed, (p.x + dx).floor() as i32, (p.y + dz).floor() as i32).is_none_or(|q| q.id != id))
}

/// The level to stand something covering columns `lo` to `hi` (continent-local,
/// inclusive) on, set on the ground as it lies: the top of the lowest ground under it,
/// if the columns are a site for it. They must be dry, worked ground of field `id` with
/// [`MARGIN`] columns more of it round them, clear of trails, pads and rivers, rising no
/// more than `rise` across, and out of the room of what is already `chosen`.
fn ground(c: &Continent, seed: u32, (lo, hi): (IVec2, IVec2), id: u32, rise: i32, chosen: &[Chosen]) -> Option<i32> {
    if chosen.iter().any(|o| meets(o.room, grow((lo, hi), APART))) {
        return None;
    }
    let (a, b) = grow((lo, hi), MARGIN);
    let span = (b - a).max_element() + 1;
    let sampler = Sampler::new(c, seed, a.x, a.y, span);
    let (mut low, mut high) = (i32::MAX, i32::MIN);
    for z in a.y..=b.y {
        for x in a.x..=b.x {
            let inside = x >= lo.x && x <= hi.x && z >= lo.y && z <= hi.y;
            // The ring round it only has to be the same field, and every other column of
            // it will tell: a hedge or a wall is wider than one.
            if !inside && (x + z) % 2 != 0 {
                continue;
            }
            if c.land.yard(x, z) || plot(c, seed, x, z).is_none_or(|p| p.id != id) {
                return None;
            }
            if !inside {
                continue;
            }
            let col = sampler.column(x, z);
            if !worked(&col) || col.clear < CLEAR || col.river < RIVER {
                return None;
            }
            low = low.min(col.top());
            high = high.max(col.top());
        }
    }
    (high >= low && high - low <= rise).then_some(low)
}

/// What a fixture of `farm` (settlement `k`) is like, before it has its parts.
#[allow(clippy::too_many_arguments)]
fn fixture(k: usize, what: What, at: IVec3, turns: u32, area: (IVec2, IVec2), front: IVec2, field: &Plot) -> Fixture {
    Fixture { owner: k, what, at, turns, area, front, field: *field, water: None, hives: Vec::new() }
}

/// A few haystacks along the rows of hay meadow `field` from `p`.
#[allow(clippy::too_many_arguments)]
fn haystacks(c: &Continent, seed: u32, farm: &Farm, k: usize, pal: &Palette, p: Vec2, field: &Plot, group: u32, chosen: &[Chosen]) -> Vec<Chosen> {
    // In the north and the mountains the hay is built round a pole.
    let kind = if matches!(farm.region, Region::Northern | Region::Alpine) { Fitting::PoleRick } else { Fitting::Haystack };
    let at0 = p.floor().as_ivec2();
    let count = 1 + (unit(hash2(farm.seed ^ 0xf1f0, at0.x, at0.y)) * 3.0) as i32;
    let step = kind.size().x as f32 + 4.0;
    let mut out: Vec<Chosen> = Vec::new();
    for j in 0..count {
        let q = (p + field.rows * step * j as f32).floor().as_ivec2();
        let h = hash2(farm.seed ^ 0xf1f1, q.x, q.y);
        let turns = h % 4;
        let at = IVec3::new(q.x, 0, q.y);
        let area = area_of(kind, at, turns);
        let Some(y) = ground(c, seed, area, field.id, 2, chosen).filter(|_| !out.iter().any(|o| meets(o.room, grow(area, APART)))) else { continue };
        let at = IVec3::new(q.x, y + 1, q.y);
        let (whole, cut) = (fittings::build(kind, pal, h, false).place(at, turns), fittings::build(kind, pal, h, true).place(at, turns));
        out.push(Chosen {
            group,
            fixture: fixture(k, What::Haystack, at, turns, area, fittings::front(turns), field),
            room: area,
            pads: Vec::new(),
            parts: vec![(Shows::Stacked, whole.voxels, whole.fine, None), (Shows::Cut, cut.voxels, cut.fine, None)],
        });
    }
    out
}

/// A scarecrow at `p` in `field`, a field that carries grain in some years.
#[allow(clippy::too_many_arguments)]
fn scarecrow(c: &Continent, seed: u32, farm: &Farm, k: usize, pal: &Palette, p: Vec2, field: &Plot, group: u32, chosen: &[Chosen]) -> Option<Chosen> {
    let q = p.floor().as_ivec2();
    let h = hash2(farm.seed ^ 0xf1f2, q.x, q.y);
    let turns = h % 4;
    let at = IVec3::new(q.x, 0, q.y);
    let area = area_of(Fitting::Scarecrow, at, turns);
    let y = ground(c, seed, area, field.id, 2, chosen)?;
    let at = IVec3::new(q.x, y + 1, q.y);
    let f = fittings::build(Fitting::Scarecrow, pal, h, false).place(at, turns);
    Some(Chosen {
        group,
        fixture: fixture(k, What::Scarecrow, at, turns, area, fittings::front(turns), field),
        room: area,
        pads: Vec::new(),
        parts: vec![(Shows::Grain, f.voxels, f.fine, None)],
    })
}

/// A field shelter on pasture `field` near `p`, on ground levelled for it as a barn's
/// is, with a trough on the levelled ground beside it.
#[allow(clippy::too_many_arguments)]
fn shelter(c: &Continent, seed: u32, farm: &Farm, k: usize, pal: &Palette, p: Vec2, field: &Plot, group: u32, chosen: &[Chosen]) -> Vec<Chosen> {
    let origin = p.floor().as_ivec2();
    let h = hash2(farm.seed ^ 0xf1f3, origin.x, origin.y);
    let turns = h % 4;
    let r = |salt: u32| unit(hash2(farm.seed ^ salt, origin.x, origin.y));
    let spec = BarnSpec {
        kind: BarnKind::Shelter,
        region: farm.region,
        age: (match farm.age {
            Age::Young => 0.15,
            Age::Established => 0.45,
            Age::Ancient => 0.75,
        } + r(0xf1f4) * 0.25)
            .min(1.0),
        wealth: (farm.prosperity * 0.8 + r(0xf1f5) * 0.2).clamp(0.0, 1.0),
        seed: h,
    };
    let built = barn::build(&spec, pal);
    let b = &built.building;
    // The trough stands along the shelter's side, its end at the line of the open front,
    // on the same levelled ground.
    let trough_q = IVec3::new(b.walls.hi.x - 3, 0, b.footprint.hi.y + 2);
    let trough_model = Fitting::WaterTrough.room(trough_q, 0);
    let model = union((b.footprint.lo, b.footprint.hi), (IVec2::new(trough_model.0.x, trough_model.0.z), IVec2::new(trough_model.1.x - 1, trough_model.1.z - 1)));
    let turned = |(lo, hi): (IVec2, IVec2)| {
        let (a, b) = (rotate(IVec3::new(lo.x, 0, lo.y), turns), rotate(IVec3::new(hi.x, 0, hi.y), turns));
        (origin + IVec2::new(a.x.min(b.x), a.z.min(b.z)), origin + IVec2::new(a.x.max(b.x), a.z.max(b.z)))
    };
    let site = turned(model);
    // Its levelled ground reaches a little past what stands on it, and the levelling
    // blends out beyond that; nothing set on the ground as it lies comes so near.
    let room = grow(site, m(7.0) as i32);
    if chosen.iter().any(|o| meets(o.room, room)) {
        return Vec::new();
    }
    // All on the one pasture, its corners and middle.
    let (lo, hi) = grow(site, 1);
    let mid = (lo + hi) / 2;
    if [lo, hi, IVec2::new(lo.x, hi.y), IVec2::new(hi.x, lo.y), mid].iter().any(|q| plot(c, seed, q.x, q.y).is_none_or(|p| p.id != field.id)) {
        return Vec::new();
    }
    let Some(y) = farmstead::site(c, seed, site) else { return Vec::new() };
    let at = IVec3::new(origin.x, y + 1, origin.y);
    let open = rotate(IVec3::X, turns);
    let pads: Vec<Pad> = farmstead::cover(site)
        .into_iter()
        .map(|(q, r)| {
            let mut pad = Pad::plain(q.x, q.y, r, y as f32);
            // Trodden before the open front, where the beasts go in and out.
            pad.wear = 0.3;
            let step = (at + open * (b.walls.hi.x + 3)).as_vec3();
            let d = glam::vec2(step.x + 0.5, step.z + 0.5) - q;
            pad.front = d.y.atan2(d.x);
            pad.door = d.length();
            pad
        })
        .collect();
    let voxels: Vec<(IVec3, Voxel)> = b.voxels.iter().map(|&(q, v)| (at + rotate(q, turns), v)).collect();
    let fine = parts::place_fine(&b.fine, at, turns);
    let front = IVec2::new(open.x, open.z);
    let shed = Chosen {
        group,
        fixture: fixture(k, What::Shelter, at, turns, turned((b.walls.lo, b.walls.hi)), front, field),
        room,
        pads,
        parts: vec![(Shows::Always, voxels, fine, None)],
    };
    let tat = at + rotate(trough_q, turns);
    let t = fittings::build(Fitting::WaterTrough, pal, h ^ 0x7a0, false).place(tat, turns);
    let mut trough = fixture(k, What::Trough, tat, turns, area_of(Fitting::WaterTrough, tat, turns), fittings::front(turns), field);
    trough.water = water(&t);
    let trough = Chosen { group, fixture: trough, room, pads: Vec::new(), parts: vec![(Shows::Always, t.voxels, t.fine, None)] };
    vec![shed, trough]
}

/// A trough on pasture `field` at `p`, if a stream runs by a short way off: the beasts
/// drink there, and it is filled from the stream.
#[allow(clippy::too_many_arguments)]
fn by_water(c: &Continent, seed: u32, farm: &Farm, k: usize, pal: &Palette, p: Vec2, field: &Plot, group: u32, chosen: &[Chosen]) -> Option<Chosen> {
    let q = p.floor().as_ivec2();
    let col = Sampler::new(c, seed, q.x, q.y, 1).column(q.x, q.y);
    if !(BY_WATER.0..=BY_WATER.1).contains(&col.river) {
        return None;
    }
    let h = hash2(farm.seed ^ 0xf1f6, q.x, q.y);
    let turns = h % 4;
    let at = IVec3::new(q.x, 0, q.y);
    let area = area_of(Fitting::WaterTrough, at, turns);
    let y = ground(c, seed, area, field.id, 1, chosen)?;
    let at = IVec3::new(q.x, y + 1, q.y);
    let t = fittings::build(Fitting::WaterTrough, pal, h, false).place(at, turns);
    let mut f = fixture(k, What::Trough, at, turns, area, fittings::front(turns), field);
    f.water = water(&t);
    Some(Chosen { group, fixture: f, room: area, pads: Vec::new(), parts: vec![(Shows::Always, t.voxels, t.fine, None)] })
}

/// What a settlement of `farm` may set by a barn or at a field's edge.
fn extras_of(farm: &Farm) -> Vec<What> {
    let mut out = vec![What::Well];
    if nests::lives_in(NestKind::Bees, farm.biome) && farm.region != Region::Desert {
        out.extend([What::Skeps, What::Skeps]);
    }
    if matches!(farm.region, Region::Temperate | Region::Upland | Region::Wetland | Region::Desert) {
        out.push(What::Dovecote);
    }
    out
}

/// A dovecote, a well or a row of skeps about `p`, its front toward `d`.
#[allow(clippy::too_many_arguments)]
fn extra(c: &Continent, seed: u32, farm: &Farm, k: usize, pal: &Palette, p: Vec2, d: IVec2, group: u32, chosen: &[Chosen]) -> Option<Chosen> {
    let q = p.floor().as_ivec2();
    let field = plot(c, seed, q.x, q.y)?;
    if field.terrace.is_some() || matches!(field.crop, Crop::Orchard | Crop::Vines | Crop::Olives | Crop::Coppice) {
        return None;
    }
    let h = hash2(farm.seed ^ 0xf1f7, q.x, q.y);
    let kinds = extras_of(farm);
    let what = kinds[(h % kinds.len() as u32) as usize];
    // Fittings face +Z; turned so they face `d`.
    let turns = (0..4).find(|&t| fittings::front(t) == d).unwrap_or(0);
    let (kind, count) = match what {
        What::Dovecote => (Fitting::Dovecote, 1),
        What::Skeps => (Fitting::Skep.local(farm.region), 3 + (h >> 8) as i32 % 2),
        _ => (Fitting::StoneWell, 1),
    };
    // A row of hives stands along the way they face, a column between each.
    let gap = kind.size().x + 1;
    let along = rotate(IVec3::X, turns);
    let at = IVec3::new(q.x, 0, q.y);
    let area = (0..count).map(|i| area_of(kind, at + along * gap * i, turns)).reduce(union)?;
    let y = ground(c, seed, area, field.id, 1, chosen)?;
    let at = IVec3::new(q.x, y + 1, q.y);
    let mut f = fixture(k, what, at, turns, area, d, &field);
    let mut parts = Vec::new();
    for i in 0..count {
        let built = fittings::build(kind, pal, h ^ i as u32, false).place(at + along * gap * i, turns);
        if what == What::Skeps {
            let nest = nests::in_hive(&built.voxels, &built.fine);
            f.hives.extend(nest);
            parts.push((Shows::Hive(i as u8), built.voxels, built.fine, nest));
        } else {
            f.water = water(&built);
            parts.push((Shows::Always, built.voxels, built.fine, None));
        }
    }
    Some(Chosen { group, fixture: f, room: area, pads: Vec::new(), parts })
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use voxel::PAD;

    use super::*;
    use crate::{
        World,
        continent::SimConfig,
        settlements::years::{Boons, When},
        world::GameWorld,
    };

    fn world() -> GameWorld {
        GameWorld::with_config(3, SimConfig { size: 256 })
    }

    #[test]
    fn fixtures_stand_in_their_fields_off_trails_rivers_and_buildings() {
        let w = world();
        let c = w.continent(0, 0);
        let fx = &c.land.fixtures;
        let kinds: HashSet<What> = fx.list.iter().map(|f| f.what).collect();
        for what in [What::Haystack, What::Scarecrow, What::Shelter, What::Trough] {
            assert!(kinds.contains(&what), "no {what:?} on the test continent: {kinds:?}");
        }
        let sampler = |lo: IVec2, hi: IVec2| Sampler::new(&c, w.seed(), lo.x - 1, lo.y - 1, (hi - lo).max_element() + 3);
        for f in &fx.list {
            let (lo, hi) = f.area;
            // Off the trails, clear of the edge of every one. (The ground a shelter's
            // beasts tread before it is its own.)
            let (points, _) = c.trails.gather(lo.as_vec2() - Vec2::splat(8.0), hi.as_vec2() + Vec2::splat(8.0));
            for t in points.iter().filter(|t| t.half_width > 0.0) {
                let p = Vec2::new(t.x, t.z).clamp(lo.as_vec2(), (hi + IVec2::ONE).as_vec2());
                assert!(p.distance(Vec2::new(t.x, t.z)) - t.half_width > m(1.5), "a trail at {},{} by the {:?} at {lo}", t.x, t.z, f.what);
            }
            let s = sampler(lo, hi);
            for z in lo.y..=hi.y {
                for x in lo.x..=hi.x {
                    let col = s.column(x, z);
                    assert!(col.water_surface().is_none(), "{:?} at {x},{z} is in water", f.what);
                    assert!(col.river > m(3.0), "{:?} at {x},{z} is in a river", f.what);
                    assert!(!c.land.yard(x, z), "{:?} at {x},{z} is in a yard", f.what);
                    if f.what != What::Shelter && f.what != What::Trough {
                        assert_eq!(plot(&c, w.seed(), x, z).map(|p| p.id), Some(f.field.id), "{:?} at {x},{z} is off its field", f.what);
                    }
                }
            }
            for st in &c.land.steads {
                assert!(!meets(st.footprint, f.area), "{:?} at {:?} is in a barn", f.what, f.area);
            }
            for q in &c.land.quarries {
                assert!(!meets(q.covers(), f.area), "{:?} at {:?} is in a quarry", f.what, f.area);
            }
            assert!(f.field.terrace.is_none());
            for g in &fx.list {
                if !std::ptr::eq(f, g) && !(f.what == What::Trough || g.what == What::Trough) {
                    assert!(!meets(f.area, g.area), "{:?} and {:?} stand in each other", f.what, g.what);
                }
            }
        }
    }

    #[test]
    fn fixtures_are_the_same_every_time() {
        let a = world();
        let b = world();
        let (ca, cb) = (a.continent(0, 0), b.continent(0, 0));
        let key = |c: &Continent| c.land.fixtures.list.iter().map(|f| (f.owner, f.what, f.at, f.turns, f.area)).collect::<Vec<_>>();
        assert_eq!(key(&ca), key(&cb));
        let boxes = |p: &Part| p.stamp.fine.iter().map(|b| (b.lo, b.hi, b.voxel)).collect::<Vec<_>>();
        assert_eq!(ca.land.fixtures.parts.len(), cb.land.fixtures.parts.len());
        assert!(ca.land.fixtures.parts.iter().zip(&cb.land.fixtures.parts).all(|(p, q)| p.stamp.voxels == q.stamp.voxels && boxes(p) == boxes(q)));
    }

    /// The haystacks are stacked through the summer and the autumn, cut into in the
    /// winter, and gone in the spring; in year 0 they stand.
    #[test]
    fn haystacks_keep_the_seasons() {
        let w = world();
        let c = w.continent(0, 0);
        let f = c.land.fixtures.list.iter().position(|f| f.what == What::Haystack).expect("a haystack");
        let parts: Vec<&Part> = c.land.fixtures.parts.iter().filter(|p| p.fixture == f as u32).collect();
        let at = |year: i32, season: u8| {
            let lives = Lives::new(&c.trails, w.seed(), (0, 0), When::new(year, season), &Boons::default());
            parts.iter().filter(|p| shows(p.shows, &c.land.fixtures.list[f], w.seed(), &lives)).map(|p| p.shows).collect::<Vec<_>>()
        };
        assert_eq!(at(0, 0), vec![Shows::Stacked]);
        assert_eq!(at(0, 3), vec![Shows::Stacked]);
        assert_eq!(at(2, 0), Vec::<Shows>::new());
        assert_eq!(at(2, 1), vec![Shows::Stacked]);
        assert_eq!(at(2, 2), vec![Shows::Stacked]);
        assert_eq!(at(2, 3), vec![Shows::Cut]);
    }

    /// A scarecrow stands in years its field carries grain, and not in others.
    #[test]
    fn scarecrows_follow_the_rotation() {
        let w = world();
        let c = w.continent(0, 0);
        let crows: Vec<&Fixture> = c.land.fixtures.list.iter().filter(|f| f.what == What::Scarecrow).collect();
        assert!(!crows.is_empty());
        let (mut stood, mut gone) = (0, 0);
        for year in 1..7 {
            let lives = Lives::new(&c.trails, w.seed(), (0, 0), When::new(year, 1), &Boons::default());
            for f in &crows {
                let up = shows(Shows::Grain, f, w.seed(), &lives);
                assert_eq!(up, grain(rotation::crop(&f.field, w.seed(), When::new(year, 1))));
                if up { stood += 1 } else { gone += 1 }
            }
        }
        assert!(stood > 0 && gone > 0, "{stood} scarecrows stood and {gone} were taken down");
    }

    /// What is out in the fields is built the same by every chunk that has a piece of it.
    #[test]
    fn fixtures_agree_across_chunks() {
        let w = world();
        let c = w.continent(0, 0);
        let mut checked = 0;
        for f in c.land.fixtures.list.iter().filter(|f| matches!(f.what, What::Haystack | What::Shelter)).take(6) {
            let (lo, hi) = f.area;
            let (cx, cz) = (lo.x.div_euclid(CHUNK), lo.y.div_euclid(CHUNK));
            let (cx1, cz1) = (hi.x.div_euclid(CHUNK), hi.y.div_euclid(CHUNK));
            // A fixture standing over a border is checked across it; any other, against
            // its neighbour to the east.
            let (ex, ez) = if cx1 > cx { (cx + 1, cz) } else if cz1 > cz { (cx, cz + 1) } else { (cx + 1, cz) };
            let a = w.generate_column(cx, cz);
            let b = w.generate_column(ex, ez);
            for (cy, ca) in &a.chunks {
                let Some((_, cb)) = b.chunks.iter().find(|(y, _)| y == cy) else { continue };
                for i in 0..PAD {
                    for y in 0..PAD {
                        if ex > cx {
                            assert_eq!(ca.get(PAD - 2, y, i), cb.get(0, y, i));
                            assert_eq!(ca.get(PAD - 1, y, i), cb.get(1, y, i));
                        } else {
                            assert_eq!(ca.get(i, y, PAD - 2), cb.get(i, y, 0));
                            assert_eq!(ca.get(i, y, PAD - 1), cb.get(i, y, 1));
                        }
                    }
                }
            }
            // And a haystack is there, whole in year 0, in the chunk that has its foot.
            if f.what == What::Haystack {
                let id = c.land.fixtures.list.iter().position(|g| std::ptr::eq(f, g)).unwrap() as u32;
                let part = c.land.fixtures.parts.iter().find(|p| p.fixture == id && p.shows == Shows::Stacked).unwrap();
                let (q, v) = part.stamp.voxels.iter().copied().find(|(q, _)| q.x.div_euclid(CHUNK) == cx && q.z.div_euclid(CHUNK) == cz).unwrap();
                let (cy, ly) = (q.y.div_euclid(CHUNK), q.y.rem_euclid(CHUNK) as usize + 1);
                let ch = &a.chunks.iter().find(|(y, _)| *y == cy).expect("its chunk").1;
                assert_eq!(ch.get((q.x - cx * CHUNK) as usize + 1, ly, (q.z - cz * CHUNK) as usize + 1), v, "the haystack at {:?} is not built", f.at);
            }
            checked += 1;
        }
        assert!(checked >= 2, "only {checked} fixtures checked");
    }

    /// The bees of the hives people keep, in a village or out by the fields, are found
    /// where the hives stand, and are gone with a hive that is lost.
    #[test]
    fn hives_near_finds_the_kept_hives_and_not_lost_ones() {
        use crate::settlements::years::{Tag, Yard};
        let w = world();
        let c = w.continent(0, 0);
        // Every kept hive's colony today, with whether it is a village's.
        let mut hives: Vec<(Nest, Option<&Stamp>, Option<(usize, u8)>)> = Vec::new();
        for s in c.trails.stamps() {
            if matches!(s.tag.as_deref(), Some(Tag::Yard { what: Yard::Hive(_), .. }))
                && let Some(n) = nests::in_hive(&s.voxels, &s.fine)
            {
                hives.push((n, Some(s), None));
            }
        }
        for p in &c.land.fixtures.parts {
            if let (Some(n), Shows::Hive(i)) = (p.nest, p.shows) {
                hives.push((n, None, Some((c.land.fixtures.list[p.fixture as usize].owner, i))));
            }
        }
        assert!(!hives.is_empty(), "no kept hives on the test continent");
        for (n, _, _) in &hives {
            let found = w.hives_near(n.door.x, n.door.z, 6);
            assert!(found.contains(n), "the hive at {} is not found: {found:?}", n.door);
        }
        // A year some are lost in: each lost one is not found, and each kept one is.
        let mut lost = 0;
        for year in 1..40 {
            w.set_now(When::new(year, 1), &Boons::default());
            let lives = w.lives(0, 0, &c);
            for (n, stamp, field) in &hives {
                let kept = match (stamp, field) {
                    (Some(s), _) => lives.shows(s),
                    (_, Some((k, i))) => *i < lives.hives(*k),
                    _ => unreachable!(),
                };
                assert_eq!(w.hives_near(n.door.x, n.door.z, 6).contains(n), kept, "year {year}: the hive at {}", n.door);
                lost += !kept as u32;
            }
            if lost > 0 {
                break;
            }
        }
        assert!(lost > 0, "no hive is ever lost");
    }

    /// Every fixture of seed 1's main continent, for finding them in the game.
    #[test]
    #[ignore]
    fn list_fixtures() {
        let seed = std::env::var("SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
        let w = GameWorld::new(seed);
        let (x, z) = std::env::var("AT").ok().and_then(|s| s.split_once(',').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))).unwrap_or((34336, 17380));
        let (mx, mz) = crate::macro_layout::cell_of(x, z);
        let c = w.continent(mx, mz);
        let (ox, oz) = (mx * crate::macro_layout::MACRO_SIZE, mz * crate::macro_layout::MACRO_SIZE);
        for f in &c.land.fixtures.list {
            let s = &c.trails.settlements[f.owner];
            println!(
                "{:?} of settlement {} ({:?}) at {} {} {}, turned {}, in a field of {:?}{}",
                f.what,
                f.owner,
                s.kind,
                f.at.x + ox,
                f.at.y,
                f.at.z + oz,
                f.turns,
                f.field.crop,
                if f.hives.is_empty() { String::new() } else { format!(", {} hives", f.hives.len()) }
            );
        }
        // And the gardens that keep hens.
        for (k, layout) in c.trails.layouts.iter().enumerate() {
            for p in &c.trails.plots[layout.plots.clone()] {
                let Some((glo, ghi)) = p.garden else { continue };
                if let Some(&(_, at, turns)) = layout.things.iter().find(|&&(f, at, _)| f == Fitting::ChickenCoop && (glo.x..=ghi.x).contains(&at.x) && (glo.y..=ghi.y).contains(&at.z)) {
                    let o = IVec2::new(ox, oz);
                    let yard = p.coop.map_or("none".to_string(), |y| format!("{} {}", y.x + ox, y.z + oz));
                    println!("HenRun of settlement {k} at {} {} {}, turned {turns}, garden {}..{}, the yard's coop {yard}", at.x + ox, at.y, at.z + oz, glo + o, ghi + o);
                }
            }
        }
    }
}
