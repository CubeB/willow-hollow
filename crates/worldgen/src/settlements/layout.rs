//! How a settlement is laid out (stage 2, #202): its lanes, the plots along them, and the
//! homes, workplaces and yard things built on each, on real ground.
//!
//! A settlement's facts (its kind, age, prosperity, water and trades: see [`Settlement`])
//! say what is built there; the ground and the trails that arrive say where. The trails
//! coming in become its lanes. A young place keeps to one street, run on past the green;
//! an established one has a lane looping round its green as well, and a town two; an
//! ancient one has back lanes out to its fields too, sunk below the ground about them by
//! the feet and carts of centuries. Plots line the lanes and face across the green. Each
//! building stands on its own levelled ground (a few round pads at one level, which hold
//! a long plot more closely than one wide one), terraced into the slope where the ground
//! falls away, its door turned to its lane and a worn path from the door to the lane;
//! there is a garden behind a home and a yard beside a workplace, set out with its trade's
//! things. The water point, the market and room for the village tree are at the core.
//!
//! The special buildings of stage 3 (#203) come in where the kit's are built, in
//! `Town::build`: a building of the kit's shape, its door to +X, put in place of the
//! stand-in for a plot whose [`Purpose::special`] is true is levelled for, placed,
//! furnished and recorded as any other.
//!
//! Everything is decided once for the whole continent, after the trails are laid so the
//! lanes can follow those that arrive, and goes into the world as stamps, as the
//! landmarks do, so it reaches the coarse rings and the far view. Yard things are held
//! back with the landmarks' props until every way is worn, so none is left on a lane.

use std::{collections::HashSet, ops::Range};

use glam::{IVec2, IVec3, Vec2, Vec3, vec2};
use voxel::{
    AIR, Voxel,
    mesh::{CELLS, PlantBox},
};

use super::{
    Age, Kind, Livelihood, Reason, Region, Settlement,
    furnish::{self, Household},
    kit::{self, AnchorKind, Building, Spec, Step},
    special::{self, Built, Facts, SpecialSpec},
};
use crate::features::designs::Special;
use crate::{
    biome::Biome,
    continent::Continent,
    features::{
        cloth,
        fittings::{self, Fitted, Fitting, Hook},
        landmarks::{self, Palette},
        parts::{self, LampStyle},
        props::{self, Prop},
        shape::{Rng, rotate},
    },
    materials as mat,
    noise::{hash2, perlin2, unit},
    scale::m,
    surface::{Cover, Sampler},
    trails::{
        LanternCause, Network, Pad, TrailLantern, TrailPoint,
        build::{in_parallel, is_wet, palette_at, profile, sample, turns_toward},
        route,
    },
};

/// What a building on a plot is for. The special buildings of stage 3 (#203), the smithy,
/// tavern and the rest, will stand where their plots are; until then a larger house from
/// the kit stands in for each, and the plot keeps what it is for.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Purpose {
    /// Where a household lives.
    Home,
    /// A store hut or a woodshed, where nobody sleeps.
    Store,
    /// Where a trade is worked, with its things in the yard, and where the household
    /// working it lives.
    Workplace(Livelihood),
    /// The forge, with the smith living above it.
    Smithy,
    /// With beds for travellers, and the keeper's household.
    Tavern,
    /// Beside a stream where there is one, for its water wheel.
    Sawmill,
    Granary,
    MarketHall,
    GuildHouse,
}

impl Purpose {
    /// A word or two for it, for tools.
    pub fn word(self) -> &'static str {
        match self {
            Purpose::Home => "home",
            Purpose::Store => "store",
            Purpose::Workplace(l) => l.word(),
            Purpose::Smithy => "smithy",
            Purpose::Tavern => "tavern",
            Purpose::Sawmill => "sawmill",
            Purpose::Granary => "granary",
            Purpose::MarketHall => "market hall",
            Purpose::GuildHouse => "guild house",
        }
    }

    /// Whether stage 3 puts a building of its own on this plot in place of the kit's
    /// house that stands in for it.
    pub fn special(self) -> bool {
        self.which().is_some()
    }

    /// The building of its own that stage 3 puts on this plot (see [`special::build`]).
    pub fn which(self) -> Option<Special> {
        Some(match self {
            Purpose::Smithy => Special::Smithy,
            Purpose::Tavern => Special::Tavern,
            Purpose::Sawmill => Special::Sawmill,
            Purpose::Granary => Special::Granary,
            Purpose::MarketHall => Special::MarketHall,
            Purpose::GuildHouse => Special::GuildHouse,
            _ => return None,
        })
    }

    /// Whether a household lives there.
    pub fn lived_in(self) -> bool {
        matches!(self, Purpose::Home | Purpose::Workplace(_) | Purpose::Smithy | Purpose::Tavern)
    }
}

/// One walk-in building as it stands, kept for what comes later: people need homes and
/// workplaces, and the map draws buildings.
#[derive(Clone, Debug)]
pub struct Plot {
    /// The settlement it belongs to, by index in `Network::settlements`.
    pub settlement: usize,
    pub purpose: Purpose,
    /// The kit's building that stands there: a hut, a timber house or a stone house.
    pub step: Step,
    /// Where the kit's model origin stands, continent-local: the building's middle column,
    /// on the first layer above its levelled ground.
    pub origin: IVec3,
    /// The quarter turns its building was turned by (as `shape::rotate` turns), which took
    /// its door from +X as the kit builds it to `facing`.
    pub turns: u32,
    /// The voxel underfoot in its doorway, continent-local, and the way out through it.
    pub door: IVec3,
    pub facing: IVec2,
    /// Every column the building covers, its eaves and steps among them, continent-local
    /// and inclusive.
    pub lo: IVec2,
    pub hi: IVec2,
    /// The columns of its outer walls and all within them, continent-local and inclusive:
    /// what a walker goes round, where `lo` and `hi` take in its porch, steps and yard too.
    pub walls: (IVec2, IVec2),
    /// Whether its walls are round, within the ellipse that fills `walls`: its corners
    /// are open ground.
    pub round: bool,
    /// How many live there: its beds, but for a tavern's guests.
    pub sleepers: u8,
    /// The levelled pads it stands on with its garden and yard, as each one's middle and
    /// radius, all at one level: its ground is `origin.y - 1`.
    pub pads: Vec<(Vec2, f32)>,
    /// The bed of the garden behind it, if it has one: continent-local columns from the
    /// first corner to the second, inclusive, dug over, with hurdles a column outside.
    pub garden: Option<(IVec2, IVec2)>,
    /// The yard beside it, if it has one: continent-local columns, inclusive.
    pub yard: Option<(IVec2, IVec2)>,
    /// Where the hen coop stands in its yard, continent-local, if hens are kept there.
    pub coop: Option<IVec3>,
    /// Where each who sleeps there lies, for putting people to bed (#204): their head,
    /// continent-local in voxels (to the nearest cell), and the way along the bed their
    /// feet lie from it.
    pub beds: Vec<(Vec3, IVec2)>,
    /// Where someone can sit inside: the voxel a sitter's feet are in, continent-local,
    /// and the way they face, toward the table where there is one.
    pub seats: Vec<(IVec3, IVec2)>,
}

/// What a settlement is laid out with besides its buildings, for the stages after.
#[derive(Clone, Debug, Default)]
pub struct Layout {
    /// Its lanes as lines of points two voxels apart: the trails that arrive, from the
    /// edge of the green outward, then the streets, loops and back lanes laid for it.
    pub lanes: Vec<Vec<Vec2>>,
    /// Ground kept open beside the green for the village tree: its middle and radius.
    pub tree: Option<(Vec2, f32)>,
    /// The foot of each well head and market stall, continent-local.
    pub wells: Vec<IVec3>,
    pub stalls: Vec<IVec3>,
    /// Every fitting set out about its yards, gardens and green, as it was set: what it
    /// is, where its model's column 0 stands (continent-local) and its quarter turns. The
    /// people working there (#204) find their hives, anvils and racks here.
    pub things: Vec<(Fitting, IVec3, u32)>,
    /// The nearest open water seen from its middle, continent-local, if any is near: where
    /// its fishers go, and where water is fetched without a well.
    pub water: Option<Vec2>,
    /// Its buildings, by index in `Network::plots`.
    pub plots: Range<usize>,
    // What stands at its edges and in it besides its buildings (#203), none of it
    // counted among them (see `edges`).
    /// The gates on its main ways in, or an ancient town's gatehouses.
    pub gates: Vec<Gate>,
    /// An ancient town's old wall: the line of columns each piece still standing runs
    /// along, continent-local.
    pub wall: Vec<Vec<IVec2>>,
    /// The fishers' boat sheds, where each meets the water.
    pub sheds: Vec<Shed>,
    /// A coastal great town's harbour wall.
    pub harbour: Option<Harbour>,
    /// A great town's park.
    pub park: Option<Park>,
}

/// Lays out every settlement on the continent, in order. `first` is the index of the
/// first settlement's green among the network's pads, and `ways` every trail before it
/// was laid, as its points and the half-width its traffic gives it at each: the lanes
/// follow those that arrive.
pub(crate) fn lay_out(c: &Continent, seed: u32, net: &mut Network, first: usize, ways: &[(Vec<Vec2>, Vec<f32>)]) {
    for k in 0..net.settlements.len() {
        let mut town = Town::new(c, seed, net, first, k, ways);
        town.build(net);
        town.layout.water = town.water;
        let from = net.plots.len();
        net.plots.append(&mut town.plots);
        town.layout.plots = from..net.plots.len();
        net.layouts.push(town.layout);
    }
}

/// Least gap between the edges of two pads, in voxels: room for a terrace wall.
const PAD_GAP: f32 = 2.0;
/// Within this of each other, two pads a single voxel apart in height would tilt one
/// another's level ground as each blends into the land about it, so they are made level
/// with each other instead (see the pads in `Sampler::apply_trails`).
const PAD_BLEND: f32 = m(4.8);
/// How far a plot stands back from the worn edge of its lane, in voxels.
const SETBACK: f32 = 2.5;
/// Depth of a home's garden behind it, and of a workplace's or a home's yard beside it,
/// in voxels.
const GARDEN: i32 = 6;
const YARD: i32 = 7;
const HOME_YARD: i32 = 6;
/// The most a plot's ground may rise across it: it is terraced, but not out of a cliff.
const PLOT_RISE: f32 = m(3.2);
/// The most a plot's level may stand above or below its lane, in voxels, so the way up
/// to its door is a walk.
const PLOT_STEP: f32 = 4.0;
/// Half-width of the path worn from a door to its lane.
const DOOR_PATH: f32 = m(0.4);
/// Half-width of the lanes laid for a settlement, loops and streets.
const LANE: f32 = m(0.6);
/// Radius of the ground kept for the village tree.
const TREE_ROOM: f32 = m(6.0);

/// Flags on the columns about a settlement, for the small things set out last.
const WAY: u8 = 1;
const PLOT: u8 = 2;
const THING: u8 = 4;
const GREEN: u8 = 8;
const KEEP: u8 = 16;

/// What a lane is in the settlement.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Way {
    /// A trail that arrives; the busiest is the main lane.
    Arrival { main: bool },
    /// Laid on through a place with one way in, so its street runs on past the green.
    Street,
    /// Round the green.
    Loop,
    /// From the loop out to the fields.
    Back,
    /// The edge of the green, which houses face across.
    Green,
}

/// A lane for plots to line: points two voxels apart and the half-width worn at each.
struct Lane {
    pts: Vec<Vec2>,
    hw: Vec<f32>,
    way: Way,
}

/// The worn ways about a settlement, filed in squares, for keeping plots off them.
struct Worn {
    lo: Vec2,
    n: i32,
    bins: Vec<Vec<Point>>,
}

#[derive(Clone, Copy)]
struct Point {
    at: Vec2,
    /// Half-width worn there; a bridge's deck, which wears no ground, keeps a little room.
    hw: f32,
    y: f32,
    /// A path from a door, which no plot is laid along.
    door: bool,
}

const BIN: f32 = 16.0;
/// The widest any trail is worn, to search far enough about a point for its edge.
const WIDEST: f32 = m(1.6);

impl Worn {
    fn new(lo: Vec2, hi: Vec2) -> Self {
        let n = ((hi - lo).max_element() / BIN).ceil() as i32 + 1;
        Worn { lo, n, bins: vec![Vec::new(); (n * n) as usize] }
    }

    fn bin(&self, p: Vec2) -> (i32, i32) {
        (((p.x - self.lo.x) / BIN).floor() as i32, ((p.y - self.lo.y) / BIN).floor() as i32)
    }

    fn add(&mut self, p: Point) {
        let (i, j) = self.bin(p.at);
        if (0..self.n).contains(&i) && (0..self.n).contains(&j) {
            self.bins[(j * self.n + i) as usize].push(p);
        }
    }

    /// Every point within `r` of `p`, until `f` says to stop; whether it did.
    fn any(&self, p: Vec2, r: f32, mut f: impl FnMut(&Point) -> bool) -> bool {
        let (a, b) = (self.bin(p - Vec2::splat(r)), self.bin(p + Vec2::splat(r)));
        for j in a.1.max(0)..=b.1.min(self.n - 1) {
            for i in a.0.max(0)..=b.0.min(self.n - 1) {
                for q in &self.bins[(j * self.n + i) as usize] {
                    if q.at.distance_squared(p) <= r * r && f(q) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Whether a disc of radius `r` at `p` keeps `gap` clear of every worn strip.
    fn clear(&self, p: Vec2, r: f32, gap: f32) -> bool {
        !self.any(p, r + gap + WIDEST, |q| q.at.distance(p) < r + gap + q.hw)
    }

    /// The nearest point of a lane (not a door's path) within `r` of `p`.
    fn nearest(&self, p: Vec2, r: f32) -> Option<Point> {
        let mut best: Option<(f32, Point)> = None;
        self.any(p, r, |q| {
            let d = q.at.distance_squared(p);
            if !q.door && best.is_none_or(|b| d < b.0 || (d == b.0 && (q.at.x, q.at.y) < (b.1.at.x, b.1.at.y))) {
                best = Some((d, *q));
            }
            false
        });
        best.map(|b| b.1)
    }
}

/// A disc of ground already taken: a pad and its level, or ground kept open.
#[derive(Clone, Copy)]
struct Disc {
    at: Vec2,
    r: f32,
    y: Option<f32>,
    /// The building whose plot it is, by index in `Town::raised`.
    owner: Option<usize>,
}

/// The columns about a settlement and what is on each.
struct Grid {
    x0: i32,
    z0: i32,
    n: i32,
    cells: Vec<u8>,
}

impl Grid {
    fn new(lo: Vec2, hi: Vec2) -> Self {
        let (x0, z0) = (lo.x.floor() as i32, lo.y.floor() as i32);
        let n = ((hi - lo).max_element().ceil() as i32) + 2;
        Grid { x0, z0, n, cells: vec![0; (n * n) as usize] }
    }

    fn get(&self, c: IVec2) -> u8 {
        let (i, j) = (c.x - self.x0, c.y - self.z0);
        if (0..self.n).contains(&i) && (0..self.n).contains(&j) { self.cells[(j * self.n + i) as usize] } else { u8::MAX }
    }

    fn mark(&mut self, c: IVec2, f: u8) {
        let (i, j) = (c.x - self.x0, c.y - self.z0);
        if (0..self.n).contains(&i) && (0..self.n).contains(&j) {
            self.cells[(j * self.n + i) as usize] |= f;
        }
    }

    fn disc(&mut self, at: Vec2, r: f32, f: u8) {
        for z in (at.y - r).floor() as i32..=(at.y + r).ceil() as i32 {
            for x in (at.x - r).floor() as i32..=(at.x + r).ceil() as i32 {
                if vec2(x as f32 + 0.5, z as f32 + 0.5).distance(at) < r {
                    self.mark(IVec2::new(x, z), f);
                }
            }
        }
    }

    /// Whether any of `cols` has any of the flags in `mask`, or lies off the grid.
    fn any(&self, cols: &[IVec2], mask: u8) -> bool {
        cols.iter().any(|&c| self.get(c) & mask != 0)
    }
}

/// A building wanted in a settlement, before it has a plot.
#[derive(Clone, Copy, Debug)]
struct Want {
    purpose: Purpose,
    step: Step,
    /// Who lives there, and the beds it is built with: a tavern has beds for travellers.
    residents: u8,
    beds: u8,
    wealth: f32,
    age: f32,
    seed: u32,
}

/// What stands on a plot, in the building's model space with its door to +X, before it
/// is turned to its lane: the building, a garden behind it and a yard beside it.
struct Lot {
    b: Building,
    /// The garden's bed, columns inclusive; its hurdles stand a column outside it.
    garden: Option<(IVec2, IVec2)>,
    yard: Option<(IVec2, IVec2)>,
    /// The columns about it all, inclusive.
    rect: (IVec2, IVec2),
    /// The pads it is levelled on, in model space: the building's with a column to spare
    /// all round, and the garden's and the yard's, each covered by a disc or a few.
    discs: Vec<(Vec2, f32)>,
}

impl Lot {
    /// `yard` is which side it is on (+Z or -Z) and how deep.
    fn new(b: Building, garden: bool, yard: Option<(bool, i32)>) -> Self {
        let (lo, hi) = (b.footprint.lo, b.footprint.hi);
        let garden = garden.then(|| {
            let mid = (b.walls.lo.y + b.walls.hi.y).div_euclid(2);
            let half = ((b.walls.hi.y - b.walls.lo.y) / 2).clamp(3, 5);
            (IVec2::new(lo.x - 1 - GARDEN, mid - half), IVec2::new(lo.x - 2, mid + half))
        });
        let yard = yard.map(|(plus, deep)| {
            let (x0, x1) = (lo.x, hi.x.min(lo.x + 13).max(lo.x + 9));
            if plus { (IVec2::new(x0, hi.y + 2), IVec2::new(x1, hi.y + 1 + deep)) } else { (IVec2::new(x0, lo.y - 1 - deep), IVec2::new(x1, lo.y - 2)) }
        });
        let mut discs = cover((lo, hi), 1.0);
        let (mut rlo, mut rhi) = (lo, hi);
        if let Some((a, b)) = garden {
            discs.extend(cover((a - IVec2::ONE, b + IVec2::ONE), 0.75));
            rlo = rlo.min(a - IVec2::ONE);
            rhi = rhi.max(b + IVec2::ONE);
        }
        if let Some((a, b)) = yard {
            discs.extend(cover((a, b), 0.75));
            rlo = rlo.min(a);
            rhi = rhi.max(b);
        }
        Lot { b, garden, yard, rect: (rlo - IVec2::ONE, rhi + IVec2::ONE), discs }
    }

    /// Whether model column `c` is on the plot's level ground, a little inside its pads.
    fn level(&self, c: IVec2) -> bool {
        let p = c.as_vec2() + 0.5;
        self.discs.iter().any(|&(at, r)| at.distance(p) < r - 0.5)
    }
}

/// Discs covering columns `lo..=hi` with `margin` to spare beyond their corners: pieces
/// about as long as they are wide along the longer side, so a long plot is not levelled
/// as one wide round.
fn cover((lo, hi): (IVec2, IVec2), margin: f32) -> Vec<(Vec2, f32)> {
    let size = (hi - lo + IVec2::ONE).as_vec2();
    let long_x = size.x >= size.y;
    let (long, short) = if long_x { (size.x, size.y) } else { (size.y, size.x) };
    let n = (long / short).round().max(1.0) as i32;
    let piece = long / n as f32;
    (0..n)
        .map(|i| {
            let along = (i as f32 + 0.5) * piece;
            let (at, half) = if long_x {
                (vec2(lo.x as f32 + along, lo.y as f32 + size.y * 0.5), vec2(piece, short) * 0.5)
            } else {
                (vec2(lo.x as f32 + size.x * 0.5, lo.y as f32 + along), vec2(short, piece) * 0.5)
            };
            (at, half.length() + margin)
        })
        .collect()
}

/// A point of a model's ground (in voxel units, a column's middle at +0.5) once the model
/// is turned by `turns`, as `shape::rotate` turns its voxels.
fn turn_point(p: Vec2, turns: u32) -> Vec2 {
    let mut q = p - 0.5;
    for _ in 0..turns & 3 {
        q = vec2(-q.y, q.x);
    }
    q + 0.5
}

/// A plot chosen for a lot: where its model origin goes, its turns, and its pads.
#[derive(Clone, Debug)]
struct Spot {
    origin: IVec2,
    turns: u32,
    discs: Vec<(Vec2, f32)>,
    y: f32,
    /// The lane it faces, and the point on it its door looks to.
    lane: usize,
    toward: Vec2,
}

/// A building raised, kept while the settlement is laid out, for its yard and path.
struct Raised {
    want: Want,
    lot: Lot,
    spot: Spot,
    origin: IVec3,
}

/// Columns `lo..=hi` of a model turned by `turns`, as columns inclusive.
fn turn_rect((lo, hi): (IVec2, IVec2), turns: u32) -> (IVec2, IVec2) {
    let (a, b) = (rotate(IVec3::new(lo.x, 0, lo.y), turns), rotate(IVec3::new(hi.x, 0, hi.y), turns));
    (IVec2::new(a.x.min(b.x), a.z.min(b.z)), IVec2::new(a.x.max(b.x), a.z.max(b.z)))
}

fn flat(q: IVec3) -> IVec2 {
    IVec2::new(q.x, q.z)
}

/// The column of model voxel column `q` once its model is turned by `turns` and its
/// origin set at `o`.
fn world(o: IVec3, q: IVec2, turns: u32) -> IVec2 {
    flat(o + rotate(IVec3::new(q.x, 0, q.y), turns))
}

/// Quarter turns that face a fitting's front (+Z as built) along `d`.
fn facing(d: Vec2) -> u32 {
    (0..4).max_by(|&a, &b| fittings::front(a).as_vec2().dot(d).total_cmp(&fittings::front(b).as_vec2().dot(d))).unwrap_or(0)
}

/// Cloth colours hung out to dry and stretched over stalls.
const CLOTHS: [Voxel; 6] = [mat::CLOTH_PALE, mat::CLOTH_PALE, mat::CLOTH_BLUE, mat::CLOTH_RED, mat::CLOTH_GREEN, mat::CLOTH_YELLOW];

/// One settlement being laid out.
struct Town<'a> {
    c: &'a Continent,
    seed: u32,
    k: usize,
    s: Settlement,
    centre: Vec2,
    green: Pad,
    pal: Palette,
    /// The ground as it was when the trails were down, before any plot.
    sam: Sampler<'a>,
    worn: Worn,
    discs: Vec<Disc>,
    grid: Grid,
    lanes: Vec<Lane>,
    rng: Rng,
    /// 0 well kept to 1 run down: how much of what stands about is worn or broken.
    neglect: f32,
    /// The nearest open water seen from the middle, and the way to the woods.
    water: Option<Vec2>,
    wood: Vec2,
    raised: Vec<Raised>,
    plots: Vec<Plot>,
    layout: Layout,
    /// Stage 3's edges (#203) between keeping their ground and building them.
    edge: edges::Edges,
}

impl<'a> Town<'a> {
    fn new(c: &'a Continent, seed: u32, net: &Network, first: usize, k: usize, ways: &[(Vec<Vec2>, Vec<f32>)]) -> Self {
        let s = net.settlements[k].clone();
        let green = net.pads[first + k];
        let centre = vec2(green.x, green.z);
        let reach = s.radius + m(24.0);
        let (lo, hi) = (centre - reach, centre + reach);
        let sam = Sampler::with_network(c, seed, lo.x.floor() as i32, lo.y.floor() as i32, (2.0 * reach).ceil() as i32 + 2, net);
        let biome = sam.column(s.x, s.z).biome;
        let pal = palette_at(c, seed, biome, s.x, s.y - 1, s.z).varied(s.seed);
        let mut worn = Worn::new(lo, hi);
        let mut grid = Grid::new(lo, hi);
        let (points, pads) = net.gather(lo, hi);
        for p in &points {
            let hw = if p.half_width > 0.0 { p.half_width } else { m(1.2) };
            worn.add(Point { at: vec2(p.x, p.z), hw, y: p.y, door: false });
            grid.disc(vec2(p.x, p.z), hw + 1.0, WAY);
        }
        let mut discs = Vec::new();
        for p in &pads {
            discs.push(Disc { at: vec2(p.x, p.z), r: p.radius, y: Some(p.y), owner: None });
            grid.disc(vec2(p.x, p.z), p.radius, if vec2(p.x, p.z) == centre { GREEN } else { PLOT });
        }
        let neglect = ((1.0 - s.prosperity) * 0.6 + if s.age == Age::Ancient { 0.1 } else { 0.0 }).clamp(0.0, 0.8);
        let mut town = Town {
            c,
            seed,
            k,
            centre,
            green,
            pal,
            sam,
            worn,
            discs,
            grid,
            lanes: Vec::new(),
            rng: Rng::new(s.seed ^ 0x1a70_07a5),
            neglect,
            water: None,
            wood: Vec2::ZERO,
            raised: Vec::new(),
            plots: Vec::new(),
            layout: Layout::default(),
            edge: edges::Edges::default(),
            s,
        };
        town.arrivals(ways);
        town.look_about();
        town
    }

    /// The trails that come within the settlement, as lanes running out from its middle.
    fn arrivals(&mut self, ways: &[(Vec<Vec2>, Vec<f32>)]) {
        let (r, inner) = (self.s.radius + 4.0, self.green.radius - 2.0);
        let mut found: Vec<Lane> = Vec::new();
        for (pts, widths) in ways {
            let mut run: Vec<usize> = Vec::new();
            let flush = |run: &mut Vec<usize>, found: &mut Vec<Lane>| {
                if run.len() >= 6 {
                    let mut idx = std::mem::take(run);
                    if pts[idx[0]].distance(self.centre) > pts[idx[idx.len() - 1]].distance(self.centre) {
                        idx.reverse();
                    }
                    let lane_pts: Vec<Vec2> = idx.iter().map(|&i| pts[i]).collect();
                    let hw = idx
                        .iter()
                        .map(|&i| self.worn.nearest(pts[i], 3.0).map_or(widths[i.min(widths.len() - 1)], |q| q.hw))
                        .collect();
                    found.push(Lane { pts: lane_pts, hw, way: Way::Arrival { main: false } });
                }
                run.clear();
            };
            for (i, p) in pts.iter().enumerate() {
                let d = p.distance(self.centre);
                if d <= r && d >= inner {
                    run.push(i);
                } else {
                    flush(&mut run, &mut found);
                }
            }
            flush(&mut run, &mut found);
        }
        // The busiest that comes in to the green is the main lane.
        let reaches = |l: &Lane| l.pts[0].distance(self.centre) < self.green.radius + 6.0;
        if let Some(main) = (0..found.len()).filter(|&i| reaches(&found[i])).max_by(|&a, &b| found[a].hw[0].total_cmp(&found[b].hw[0]).then(b.cmp(&a))) {
            found[main].way = Way::Arrival { main: true };
            found.swap(0, main);
        }
        for l in &found {
            self.layout.lanes.push(l.pts.clone());
        }
        self.lanes = found;
        // The green's edge, which houses face across: drawn round so that its points'
        // normals point outward.
        let gr = self.green.radius + 0.5;
        let n = ((std::f32::consts::TAU * gr) / 2.0).ceil() as usize;
        let pts: Vec<Vec2> = (0..n).map(|i| self.centre + Vec2::from_angle(-(i as f32) / n as f32 * std::f32::consts::TAU) * gr).collect();
        self.lanes.push(Lane { hw: vec![0.5; pts.len()], pts, way: Way::Green });
    }

    /// Where the nearest water lies, and which way the woods are.
    fn look_about(&mut self) {
        let r = self.s.radius;
        let mut best: Option<(f32, Vec2)> = None;
        for ring in 1..=5 {
            let d = r * ring as f32 / 5.0 + m(4.0);
            for a in 0..24 {
                let p = self.centre + Vec2::from_angle(a as f32 / 24.0 * std::f32::consts::TAU) * d;
                let col = self.sam.column(p.x.floor() as i32, p.y.floor() as i32);
                if (col.water_surface().is_some() || col.river < 0.0) && best.is_none_or(|b| d < b.0) {
                    best = Some((d, p));
                }
            }
            if best.is_some() {
                break;
            }
        }
        self.water = best.map(|b| b.1);
        let cs = self.c.cell_size();
        let n = self.c.size as i32;
        let mut wood = Vec2::ZERO;
        for a in 0..16 {
            let dir = Vec2::from_angle(a as f32 / 16.0 * std::f32::consts::TAU);
            for f in [1.2, 1.7, 2.3] {
                let p = self.centre + dir * r * f;
                let (x, z) = ((p.x / cs) as i32, (p.y / cs) as i32);
                if x < 0 || z < 0 || x >= n || z >= n {
                    continue;
                }
                use Biome::*;
                let b = Biome::from_u8(self.c.biome[(z * n + x) as usize]);
                if matches!(b, Deciduous | Woodland | BirchGrove | Taiga | SnowyPine | OldGrowth | Rainforest) {
                    wood += dir;
                }
            }
        }
        self.wood = wood.normalize_or_zero();
    }

    fn build(&mut self, net: &mut Network) {
        self.keep_for_tree();
        // Stage 3 (#203): a great town's park kept round the tree, before the lanes go
        // round it.
        self.keep_for_park();
        self.lay_lanes(net);
        // Stage 3 (#203): the gates' ground kept from every plot.
        self.keep_for_gates(net);
        let mut wants = self.wants();
        // Stage 3 (#203): the fishers' workplace a boat shed, where there is water.
        self.choose_shed(&mut wants);
        let shed = self.edge.shed;
        // The kit's buildings are most of the work, and each stands alone, so they are
        // built side by side.
        // The tavern, the smithy and the rest are stage 3's own (see [`special`]), each
        // furnished as it is built; the homes and workplaces are the kit's.
        let (region, pal) = (self.s.region, &self.pal);
        let facts = Facts::of(&self.s);
        // A great town's homes may take what only a great town's do (a wind catcher).
        let how = kit::components::Dress { great_town: self.s.kind == Kind::GreatTown };
        // A home as `kit::build_dressed` builds it, and where a hen coop set beside it
        // stands, if one was (model space).
        let home = |spec: &Spec| match crate::features::designs::pick_house(spec.region, spec.step, spec.seed, spec.sleepers).and_then(|d| kit::from_design(d, pal, spec.seed)) {
            Some(b) => (b, None),
            None => {
                let (b, attached) = kit::generate_dressed(spec, pal, kit::rules::rules(region), &how);
                let coop = attached.iter().find(|a| a.component == kit::components::Component::HenCoop).map(|a| a.anchor.at + a.anchor.normal * 2);
                (b, coop)
            }
        };
        let kits = in_parallel(&wants, |w| {
            let which = w.purpose.which().or((Some(w.seed) == shed).then_some(Special::BoatShed));
            let own = which.and_then(|which| special::build(&SpecialSpec { which, region, age: w.age, wealth: w.wealth, seed: w.seed, stream: None, facts }, pal));
            match own {
                Some(own) => (own.building.clone(), Some(own), None),
                None => {
                    let (b, coop) = home(&Spec { region, step: w.step, age: w.age, wealth: w.wealth, sleepers: w.beds, seed: w.seed });
                    (b, None, coop)
                }
            }
        });
        for (want, (built, own, coop)) in wants.into_iter().zip(kits) {
            self.raise(net, want, built, own, coop);
        }
        self.door_paths(net);
        // The ground as it is now, every plot levelled and every way worn.
        let reach = self.s.radius + m(24.0);
        let lo = self.centre - reach;
        let fin = Sampler::with_network(self.c, self.seed, lo.x.floor() as i32, lo.y.floor() as i32, (2.0 * reach).ceil() as i32 + 2, net);
        let raised = std::mem::take(&mut self.raised);
        for (k, r) in raised.iter().enumerate() {
            self.dress(net, r, k);
        }
        self.raised = raised;
        // Stage 3 (#203): the gates, the old wall, the water side and the park, built
        // now every plot and path is down.
        self.raise_gates(net, &fin);
        self.raise_wall(net, &fin);
        self.harbour(net, &fin);
        self.park(net, &fin);
        self.wells(net, &fin);
        self.stalls(net);
        self.lanterns(net);
    }

    /// Ground kept open beside the green for the village tree, on the side away from
    /// the main way in where it can be.
    fn keep_for_tree(&mut self) {
        let away = self.lanes.first().filter(|l| matches!(l.way, Way::Arrival { .. })).map_or(self.rng.unit() * std::f32::consts::TAU, |l| {
            let d = l.pts[(l.pts.len() / 3).min(6)] - self.centre;
            d.y.atan2(d.x) + std::f32::consts::PI
        });
        let r = TREE_ROOM;
        for j in 0..16 {
            let turn = (j as f32 / 2.0).ceil() * if j % 2 == 0 { 1.0 } else { -1.0 } * 0.4;
            let at = self.centre + Vec2::from_angle(away + turn) * (self.green.radius + r + 3.0);
            if at.distance(self.centre) + r > self.s.radius || !self.worn.clear(at, r, 2.0) || self.discs.iter().any(|d| d.at.distance(at) < d.r + r + 1.0) {
                continue;
            }
            if self.probe(at, r).is_none() {
                continue;
            }
            self.discs.push(Disc { at, r, y: None, owner: None });
            self.grid.disc(at, r, KEEP);
            self.layout.tree = Some((at, r));
            return;
        }
    }

    /// The outward way of the main lane where it leaves the green, or one by the seed.
    fn main_way(&mut self) -> Vec2 {
        match self.lanes.first() {
            Some(l) if matches!(l.way, Way::Arrival { main: true }) => (l.pts[(l.pts.len() - 1).min(5)] - self.centre).normalize_or(Vec2::X),
            _ => Vec2::from_angle(unit(self.s.seed) * std::f32::consts::TAU),
        }
    }

    /// The lanes laid for the place: the street on past the green, the loops round it
    /// and the sunken back lanes, as its age and size have them.
    fn lay_lanes(&mut self, net: &mut Network) {
        let (gr, r) = (self.green.radius, self.s.radius);
        let main = self.main_way();
        let tau = std::f32::consts::TAU;
        let salt = self.s.seed ^ 0x1a9e;
        // The street: on from the far side of the green, wherever no trail leaves that way.
        let back = -main;
        let outward = |l: &Lane| (l.pts[l.pts.len() - 1] - l.pts[0]).normalize_or_zero();
        let taken = self.lanes.iter().any(|l| matches!(l.way, Way::Arrival { .. }) && l.pts[0].distance(self.centre) < gr + 6.0 && outward(l).dot(back) > 0.5);
        if !taken {
            let len = (r - gr) * 0.85;
            let side = vec2(-back.y, back.x);
            let control: Vec<Vec2> = (0..=4)
                .map(|i| {
                    let f = i as f32 / 4.0;
                    let wobble = if i == 0 { 0.0 } else { perlin2(salt, f as f64 * 2.0, 0.5) * 0.15 * len };
                    self.centre + back * (gr + 1.0 + len * f) + side * wobble
                })
                .collect();
            let hw = self.lanes.first().filter(|l| matches!(l.way, Way::Arrival { .. })).map_or(LANE, |l| (l.hw[0] * 0.8).max(LANE));
            self.lay(net, route::resample(&control), hw, 0.0, Way::Street);
        }
        if self.s.age == Age::Young || self.s.kind == Kind::Hamlet {
            return;
        }
        // Loops round the green: one in a village, an inner and an outer in a town.
        let rings: &[f32] = if self.s.kind >= Kind::Town { &[0.4, 0.75] } else { &[0.45] };
        let mut loop_r = gr;
        for (n, &f) in rings.iter().enumerate() {
            let r0 = gr + (r - gr) * f;
            loop_r = loop_r.max(r0);
            let steps = (tau * r0 / 2.0).ceil() as usize;
            let pts: Vec<Vec2> = (0..=steps)
                .map(|i| {
                    let a = i as f32 / steps as f32 * tau;
                    let (x, z) = (a.cos() as f64 * 1.4, a.sin() as f64 * 1.4);
                    let wobble = perlin2(salt ^ ((n as u32 + 1) * 0x77), x + 3.1, z - 1.7);
                    self.centre + Vec2::from_angle(a) * r0 * (1.0 + 0.14 * wobble)
                })
                .collect();
            self.lay(net, pts, LANE, 0.0, Way::Loop);
        }
        // Back lanes out to the fields, in an ancient place, worn down into the ground.
        if self.s.age == Age::Ancient {
            let arrivals: Vec<Vec2> = self.lanes.iter().filter(|l| matches!(l.way, Way::Arrival { .. })).map(|l| (l.pts[l.pts.len() - 1] - self.centre).normalize_or_zero()).collect();
            let want = if self.s.kind >= Kind::Town { 3 } else { 2 };
            let mut dirs: Vec<Vec2> = Vec::new();
            let start = unit(salt) * tau;
            for j in 0..12 {
                let d = Vec2::from_angle(start + j as f32 / 12.0 * tau);
                if dirs.len() < want && arrivals.iter().chain(&dirs).all(|a| a.dot(d) < 0.8) {
                    dirs.push(d);
                }
            }
            for d in dirs {
                let side = vec2(-d.y, d.x);
                let len = r * 0.95 - loop_r;
                let control: Vec<Vec2> = (0..=4)
                    .map(|i| {
                        let f = i as f32 / 4.0;
                        let wobble = if i == 0 { 0.0 } else { perlin2(salt ^ 0x5e5e, (d.x + f) as f64 * 1.7, d.y as f64) * 0.2 * len };
                        self.centre + d * (loop_r + len * f) + side * wobble
                    })
                    .collect();
                self.lay(net, route::resample(&control), m(0.55), 3.0, Way::Back);
            }
        }
    }

    /// Lays a lane along `pts` (about two voxels apart): worn `hw` either side, its
    /// height the ground's smoothed and, for a sunken lane, `sink` voxels below it but at
    /// its ends. It keeps out of water, off pads and kept ground, and inside the
    /// settlement; where those break it, it is laid in pieces.
    fn lay(&mut self, net: &mut Network, pts: Vec<Vec2>, hw: f32, sink: f32, way: Way) {
        if pts.len() < 10 {
            return;
        }
        let cols = sample(self.c, self.seed, &pts);
        let ok: Vec<bool> = pts
            .iter()
            .zip(&cols)
            .map(|(p, col)| {
                !is_wet(col)
                    && col.river >= m(2.4)
                    && matches!(col.cover, Cover::Land | Cover::Beach | Cover::Trail | Cover::Scree | Cover::Dune | Cover::Shingle)
                    && p.distance(self.centre) <= self.s.radius - 2.0
                    // Onto the green, which the ways in cross, but off every other pad.
                    && self.discs.iter().all(|d| d.at == self.centre || d.at.distance(*p) >= d.r + hw + 2.0)
            })
            .collect();
        let mut i = 0;
        while i < pts.len() {
            if !ok[i] {
                i += 1;
                continue;
            }
            let a = i;
            while i < pts.len() && ok[i] {
                i += 1;
            }
            if i - a < 10 {
                continue;
            }
            let (run, cols) = (&pts[a..i], &cols[a..i]);
            // Drawn to the height of any way it meets, so the two join without a step.
            let pins: Vec<(usize, f32)> = run.iter().enumerate().filter_map(|(j, p)| self.worn.nearest(*p, 4.0).map(|q| (j, q.y))).collect();
            let mut heights = profile(cols, &pins);
            if sink > 0.0 {
                let n = heights.len();
                for (j, h) in heights.iter_mut().enumerate() {
                    *h -= sink * (j.min(n - 1 - j) as f32 / 8.0).min(1.0);
                }
            }
            let points: Vec<TrailPoint> = run.iter().zip(&heights).map(|(p, &y)| TrailPoint { x: p.x, z: p.y, y, half_width: hw }).collect();
            net.add_points(&points);
            net.lines.push(run.iter().step_by(8).copied().collect());
            net.line_widths.push(vec![hw; run.len().div_ceil(8)]);
            for p in &points {
                self.worn.add(Point { at: vec2(p.x, p.z), hw, y: p.y, door: false });
                self.grid.disc(vec2(p.x, p.z), hw + 1.0, WAY);
            }
            self.layout.lanes.push(run.to_vec());
            self.lanes.push(Lane { pts: run.to_vec(), hw: vec![hw; run.len()], way });
        }
    }

    /// The buildings the settlement has, from its facts: the special buildings its kind
    /// calls for, a workplace for each trade, a store, and homes for the rest, each with
    /// its household, in the order their plots are chosen.
    fn wants(&mut self) -> Vec<Want> {
        let s = &self.s;
        let n = s.buildings as usize;
        let kind = s.kind;
        let mut specials: Vec<Purpose> = Vec::new();
        if kind >= Kind::GreatTown {
            specials.extend([Purpose::MarketHall, Purpose::GuildHouse]);
        }
        if kind >= Kind::LargeVillage {
            specials.extend([Purpose::Tavern, Purpose::Smithy]);
        }
        if kind >= Kind::LargeVillage && s.has(Livelihood::Sawmill) {
            specials.push(Purpose::Sawmill);
        }
        if kind >= Kind::Town {
            specials.push(Purpose::Granary);
        }
        // A workplace for each trade worked in the place; the weaver and the fletcher
        // share one, both working willow. A village is "a fisher or a hunter".
        let mut trades: Vec<Livelihood> = Vec::new();
        for l in [Livelihood::Fisher, Livelihood::Woodcutter, Livelihood::Forager, Livelihood::Hunter, Livelihood::Weaver, Livelihood::Apiary] {
            let has = s.has(l) || (l == Livelihood::Weaver && s.has(Livelihood::Fletcher));
            if has && !(l == Livelihood::Hunter && kind <= Kind::Village && s.has(Livelihood::Fisher) && n < 9) {
                trades.push(l);
            }
        }
        let homes_least = match kind {
            Kind::Hamlet => 1,
            Kind::Village => (n / 3).max(2),
            _ => n / 2,
        };
        let room = n.saturating_sub(homes_least);
        specials.truncate(room);
        if kind == Kind::Hamlet && !trades.is_empty() {
            // A hamlet works one trade of those about it.
            let one = trades[(hash2(s.seed, 0x7ade, 1) % trades.len() as u32) as usize];
            trades = vec![one];
        }
        trades.truncate(room - specials.len());
        let store = match kind {
            Kind::Hamlet => n >= 3 && unit(hash2(s.seed, 0x570e, 1)) < 0.6,
            Kind::Village | Kind::LargeVillage => n >= 9,
            _ => false,
        } && specials.len() + trades.len() + 1 < n;
        let homes = n - specials.len() - trades.len() - store as usize;
        // The steps of the homes, from the place's age, size and wealth: a young village
        // mostly huts, an established one timber houses among them, an ancient or big
        // one an old stone core.
        let p = s.prosperity;
        let (stone, timber) = match (s.age, kind) {
            (Age::Young, Kind::Hamlet | Kind::Village) => (0.0, 0.1 + 0.15 * p),
            (Age::Young, _) => (0.0, 0.35 + 0.2 * p),
            (Age::Established, Kind::Hamlet) => (0.0, 0.3),
            (Age::Established, Kind::Village) => (0.0, 0.25 + 0.2 * p),
            (Age::Established, Kind::LargeVillage) => (0.0, 0.55),
            (Age::Established, _) => (0.15, 0.45),
            (Age::Ancient, Kind::Hamlet) => (0.0, 0.35),
            (Age::Ancient, Kind::Village) => (0.12, 0.35),
            (Age::Ancient, Kind::LargeVillage) => (0.2, 0.45),
            (Age::Ancient, Kind::Town) => (0.3, 0.4),
            (Age::Ancient, Kind::GreatTown) => (0.35, 0.4),
        };
        let stone_homes = (homes as f32 * stone).round() as usize;
        let timber_homes = ((homes as f32 * timber).round() as usize).min(homes - stone_homes);
        let grand = s.age == Age::Ancient || kind >= Kind::Town;
        let mut list: Vec<(Purpose, Step)> = Vec::new();
        for &sp in &specials {
            let step = match sp {
                Purpose::Sawmill | Purpose::Granary => Step::Timber,
                _ if grand => Step::Stone,
                _ => Step::Timber,
            };
            list.push((sp, step));
        }
        for &l in &trades {
            let step = if self.rng.unit() < timber * 0.6 { Step::Timber } else { Step::Hut };
            list.push((Purpose::Workplace(l), step));
        }
        for h in 0..homes {
            list.push((Purpose::Home, if h < stone_homes { Step::Stone } else if h < stone_homes + timber_homes { Step::Timber } else { Step::Hut }));
        }
        if store {
            list.push((Purpose::Store, Step::Hut));
        }
        // Room in the beds for everyone: huts become timber houses until there is.
        let cap = |p: Purpose, st: Step| match p {
            Purpose::Smithy | Purpose::Tavern => 4,
            p if p.lived_in() => st.max_sleepers(),
            _ => 0,
        };
        let people = s.people as usize;
        for i in 0..list.len() {
            if list.iter().map(|&(p, st)| cap(p, st) as usize).sum::<usize>() >= people {
                break;
            }
            if list[i].1 == Step::Hut && list[i].0.lived_in() {
                list[i].1 = Step::Timber;
            }
        }
        // Everyone a bed: one to each household first, then round in turn.
        let mut residents = vec![0u8; list.len()];
        let mut left = people;
        while left > 0 {
            let mut gave = false;
            for (i, &(p, st)) in list.iter().enumerate() {
                if left > 0 && residents[i] < cap(p, st) {
                    residents[i] += 1;
                    left -= 1;
                    gave = true;
                }
            }
            if !gave {
                break;
            }
        }
        let base_age = match s.age {
            Age::Young => 0.12,
            Age::Established => 0.4,
            Age::Ancient => 0.65,
        };
        let mut out = Vec::with_capacity(list.len());
        for (i, &(purpose, step)) in list.iter().enumerate() {
            let h = |salt: i32| unit(hash2(self.s.seed ^ 0x6b17_9e3d, i as i32, salt));
            let grand_one = purpose.special() || step == Step::Stone;
            let wealth = (p * 0.7 + 0.3 * h(1) + if grand_one { 0.25 } else { 0.0 }).clamp(0.0, 1.0);
            let age = (base_age + (h(2) - 0.5) * 0.3 + if step == Step::Stone { 0.15 } else { 0.0 }).clamp(0.0, 1.0);
            let beds = match purpose {
                Purpose::Tavern => (residents[i] + 4).min(step.max_sleepers()),
                _ => residents[i],
            };
            out.push(Want { purpose, step, residents: residents[i], beds, wealth, age, seed: hash2(self.s.seed ^ 0x6b17_9e3d, i as i32, 7) });
        }
        out
    }

    /// Probes the ground of a disc: its mean height and lowest and highest, or None
    /// where any of it is wet, by a river's bank, or not ground to build on.
    fn probe(&self, at: Vec2, r: f32) -> Option<(f32, f32, f32)> {
        let (mut lo, mut hi, mut sum, mut n) = (f32::MAX, f32::MIN, 0.0, 0.0);
        let mut look = |p: Vec2| -> bool {
            let col = self.sam.column(p.x.floor() as i32, p.y.floor() as i32);
            if col.water_surface().is_some() || col.river < m(3.2) || !matches!(col.cover, Cover::Land | Cover::Beach | Cover::Trail | Cover::Scree | Cover::Dune | Cover::Shingle) {
                return false;
            }
            lo = lo.min(col.ground);
            hi = hi.max(col.ground);
            sum += col.ground;
            n += 1.0;
            true
        };
        if !look(at) {
            return None;
        }
        for (count, f) in [(6, 0.5), (12, 1.0)] {
            for k in 0..count {
                if !look(at + Vec2::from_angle(k as f32 / count as f32 * std::f32::consts::TAU) * (r - 0.5) * f) {
                    return None;
                }
            }
        }
        Some((sum / n, lo, hi))
    }

    /// Whether a disc keeps clear of every pad and kept ground, and of every worn way.
    fn room_for(&self, at: Vec2, r: f32) -> bool {
        self.discs.iter().all(|d| d.at.distance(at) >= d.r + r + PAD_GAP) && self.worn.clear(at, r, 1.0)
    }

    /// The level of a new pad at `at` beside the pads about it: as wanted, or made level
    /// with a neighbour it would otherwise stand a single voxel above or below.
    fn level_with(&self, at: Vec2, r: f32, mut y: f32) -> Option<f32> {
        for _ in 0..3 {
            let clash = self.discs.iter().find_map(|d| {
                let dy = d.y?;
                let near = at.distance(d.at) - r - d.r < PAD_BLEND;
                let diff = (y - dy).abs();
                (near && diff > 0.25 && diff < 1.75).then_some(dy)
            });
            match clash {
                None => return Some(y),
                Some(dy) if dy.fract() == 0.0 => y = dy,
                Some(_) => return None,
            }
        }
        None
    }

    /// What it costs to put `want` at `at` on `lane`: lower is likelier. Homes gather
    /// toward the middle, the stone core nearest it; each trade goes where it belongs.
    fn cost(&self, want: &Want, at: Vec2, lane: &Lane) -> f32 {
        let rel = at - self.centre;
        let r = self.s.radius;
        let d = rel.length() / r;
        let dir = rel.normalize_or_zero();
        let main = lane.way == Way::Arrival { main: true };
        let by_water = |w: Option<Vec2>| w.map_or(1.0 - d, |w| w.distance(at) / r * 1.5 + d * 0.2);
        let mut c = match want.purpose {
            Purpose::Home => match want.step {
                Step::Stone => d * 1.6,
                Step::Timber => d * 1.15,
                Step::Hut => d,
            },
            Purpose::Store => 0.3 + d * 0.5,
            Purpose::Workplace(l) => match l {
                Livelihood::Fisher | Livelihood::Weaver | Livelihood::Fletcher => by_water(self.water),
                // Toward the woods it works, at the edge.
                Livelihood::Woodcutter => 1.0 - d * 0.6 - dir.dot(self.wood) * 0.5,
                // At the edge facing the wild.
                Livelihood::Forager => 1.0 - d * 0.8 - dir.dot(self.wood) * 0.3,
                // At the edge, out toward the open country.
                Livelihood::Hunter => 1.0 - d * 0.8 + dir.dot(self.wood) * 0.3,
                // At the sunny edge, the south, which is +z, facing the open ground.
                Livelihood::Apiary => 1.0 - d * 0.6 - dir.y * 0.6,
                _ => d,
            },
            Purpose::Smithy => (d - 0.45).abs() - if main { 0.2 } else { 0.0 },
            Purpose::Tavern => d * 0.8 - if main { 0.4 } else { 0.0 },
            Purpose::Sawmill => by_water(self.water),
            Purpose::Granary => 1.0 - d,
            Purpose::MarketHall => d - if lane.way == Way::Green { 0.6 } else { 0.0 },
            Purpose::GuildHouse => d - if lane.way == Way::Green || main { 0.3 } else { 0.0 },
        };
        // Established places gather their homes about the green.
        if want.purpose == Purpose::Home && lane.way == Way::Green && self.s.age != Age::Young {
            c -= 0.15;
        }
        c + unit(hash2(self.s.seed ^ 0x51ab, at.x as i32, at.y as i32)) * 0.12
    }

    /// Finds a plot for `lot`: along the lanes, facing them, the cheapest that has
    /// room and ground to level. `pass` relaxes what is asked: a young place may build
    /// off its one street, steeper ground is taken, and at the last anywhere in the
    /// settlement with a path out to the nearest lane.
    fn find(&self, lot: &Lot, want: &Want, pass: u32) -> Option<Spot> {
        let (gr, big) = (self.green.radius, self.s.radius);
        let young = self.s.age == Age::Young && pass == 0;
        // Each a cost, the lane's point it would face, the way out from the lane to it,
        // the lane's worn half-width, the least the plot stands back, and the lane.
        let mut cands: Vec<(f32, Vec2, Vec2, f32, f32, usize)> = Vec::new();
        for (li, lane) in self.lanes.iter().enumerate() {
            let one_street = matches!(lane.way, Way::Arrival { main: true } | Way::Street | Way::Green);
            if (young && !one_street) || (lane.way == Way::Back && pass == 0) {
                continue;
            }
            let n = lane.pts.len();
            for i in (1..n.saturating_sub(1)).step_by(2) {
                let p = lane.pts[i];
                let t = (lane.pts[(i + 2).min(n - 1)] - lane.pts[i.saturating_sub(2)]).normalize_or_zero();
                for side in [-1.0, 1.0] {
                    let nrm = vec2(-t.y, t.x) * side;
                    let at = p + nrm * (lane.hw[i] + SETBACK + 6.0);
                    let d = at.distance(self.centre);
                    if d > big || d < gr + PAD_GAP || (lane.way == Way::Green && nrm.dot(p - self.centre) < 0.0) {
                        continue;
                    }
                    cands.push((self.cost(want, at, lane), p, nrm, lane.hw[i], 0.0, li));
                }
            }
        }
        if pass >= 2 {
            // Anywhere with room, facing the nearest lane, its door's path to find it.
            let span = big as i32;
            let off_lanes = Lane { pts: Vec::new(), hw: Vec::new(), way: Way::Loop };
            for dz in (-span..=span).step_by(6) {
                for dx in (-span..=span).step_by(6) {
                    let at = self.centre + vec2(dx as f32, dz as f32);
                    let d = at.distance(self.centre);
                    if d > big || d < gr + PAD_GAP {
                        continue;
                    }
                    let Some(q) = self.worn.nearest(at, m(16.0)) else { continue };
                    let nrm = (at - q.at).normalize_or_zero();
                    cands.push((self.cost(want, at, &off_lanes) + 1.0, q.at, nrm, q.hw, at.distance(q.at), usize::MAX));
                }
            }
        }
        cands.sort_by(|a, b| a.0.total_cmp(&b.0).then((a.1.x, a.1.y, a.2.x, a.2.y).partial_cmp(&(b.1.x, b.1.y, b.2.x, b.2.y)).unwrap_or(std::cmp::Ordering::Equal)));
        let rise = if pass == 0 { PLOT_RISE } else { PLOT_RISE * 1.5 };
        let step = if pass == 0 { PLOT_STEP } else { PLOT_STEP + 2.0 };
        for (_, toward, nrm, hw, back, lane) in cands {
            if nrm == Vec2::ZERO {
                continue;
            }
            // Its door turned to the lane, and its front, wherever that lies as turned,
            // stood back from the lane's worn edge.
            let turns = turns_toward(-nrm);
            let (tlo, thi) = turn_rect(lot.rect, turns);
            let half = (thi - tlo + IVec2::ONE).as_vec2() * 0.5;
            let off = tlo.as_vec2() + half;
            let reach = half.x * nrm.x.abs() + half.y * nrm.y.abs();
            let at = toward + nrm * (hw + SETBACK + reach).max(back);
            let origin = (at - off).round().as_ivec2();
            let discs: Vec<(Vec2, f32)> = lot.discs.iter().map(|&(c, r)| (origin.as_vec2() + turn_point(c, turns), r)).collect();
            let inside = |&(c, r): &(Vec2, f32)| {
                let d = c.distance(self.centre);
                d + r <= big && d - r >= gr + PAD_GAP
            };
            if !discs.iter().all(inside) || !discs.iter().all(|&(c, r)| self.room_for(c, r)) {
                continue;
            }
            // Stage 3 (#203): a boat shed's open back on the water.
            if !self.backs_on_water(lot, want, origin, turns) {
                continue;
            }
            // A path from a door off the lanes must not cross another plot to reach one.
            if lane == usize::MAX && self.discs.iter().any(|d| crosses(at, toward, d.at, d.r)) {
                continue;
            }
            let (mut sum, mut lo, mut hi, mut dry) = (0.0, f32::MAX, f32::MIN, true);
            for &(c, r) in &discs {
                let Some((mean, a, b)) = self.probe(c, r) else {
                    dry = false;
                    break;
                };
                sum += mean;
                lo = lo.min(a);
                hi = hi.max(b);
            }
            if !dry || hi - lo > rise {
                continue;
            }
            let mean = sum / discs.len() as f32;
            let lane_y = self.worn.nearest(toward, 6.0).map_or(mean, |q| q.y).round();
            let mut y = mean.round().clamp(lane_y - step, lane_y + step);
            if hi - y > 8.0 || y - lo > 8.0 {
                continue;
            }
            // Level with the pads about it, every one of its own at the one height.
            for &(c, r) in discs.iter().chain(&discs) {
                y = self.level_with(c, r, y).unwrap_or(f32::NAN);
            }
            if y.is_nan() || discs.iter().any(|&(c, r)| self.level_with(c, r, y) != Some(y)) {
                continue;
            }
            return Some(Spot { origin, turns, discs, y, lane, toward });
        }
        None
    }

    /// Builds what `want` asks for and finds it a plot, smaller and plainer if it must
    /// be; then levels its pad, furnishes it, stamps it into the world and records it.
    fn raise(&mut self, net: &mut Network, want: Want, built: Building, own: Option<Built>, coop: Option<IVec3>) {
        let (mut own, mut coop) = (own, coop);
        let spec = |step: Step, beds: u8| Spec { region: self.s.region, step, age: want.age, wealth: want.wealth, sleepers: beds, seed: want.seed };
        let garden = match want.purpose {
            Purpose::Home => unit(want.seed ^ 0x9a2d) < 0.85,
            Purpose::Workplace(Livelihood::Forager | Livelihood::Apiary) => true,
            _ => false,
        };
        let yard = match want.purpose {
            Purpose::Home => (unit(want.seed ^ 0x7a2d) < 0.6).then_some(HOME_YARD),
            Purpose::Store | Purpose::MarketHall | Purpose::GuildHouse => None,
            _ => Some(YARD),
        }
        .map(|deep| (unit(want.seed ^ 0x51de) < 0.5, deep));
        let mut want = want;
        let mut chosen = None;
        for pass in 0..4 {
            let lot = match pass {
                0 | 1 => Lot::new(built.clone(), garden, yard),
                2 => Lot::new(built.clone(), false, yard.filter(|_| !matches!(want.purpose, Purpose::Home))),
                _ => {
                    // At the last, a hut with nothing set about it: there was no room for
                    // anything bigger.
                    (own, coop) = (None, None);
                    want.step = Step::Hut;
                    want.beds = want.beds.min(Step::Hut.max_sleepers());
                    want.residents = want.residents.min(want.beds);
                    Lot::new(kit::generate_bare(&spec(Step::Hut, want.beds), &self.pal, kit::rules::rules(self.s.region)), false, None)
                }
            };
            if let Some(spot) = self.find(&lot, &want, pass) {
                chosen = Some((lot, spot));
                break;
            }
        }
        // Where there is no room even for a hut it goes unbuilt; the tests see none do.
        let Some((mut lot, spot)) = chosen else { return };
        let origin = IVec3::new(spot.origin.x, spot.y as i32 + 1, spot.origin.y);
        // A sawmill near water turns a wheel on the side toward it, where the ground the
        // wheel's pit takes is free; elsewhere it saws by hand over a pit.
        if want.purpose == Purpose::Sawmill
            && let (Some(w), Some(_)) = (self.water, &own)
        {
            let d = w - flat(origin).as_vec2();
            if d.length() < m(60.0) {
                let toward = if d.x.abs() > d.y.abs() { IVec3::new(d.x.signum() as i32, 0, 0) } else { IVec3::new(0, 0, d.y.signum() as i32) };
                let spec = SpecialSpec { which: Special::Sawmill, region: self.s.region, age: want.age, wealth: want.wealth, seed: want.seed, stream: Some(rotate(toward, (4 - spot.turns) & 3)), facts: Facts::of(&self.s) };
                if let Some(wheeled) = special::build(&spec, &self.pal) {
                    let (a, b) = turn_rect((wheeled.building.footprint.lo, wheeled.building.footprint.hi), spot.turns);
                    let (a, b) = (a + flat(origin), b + flat(origin));
                    let (old_a, old_b) = turn_rect((lot.b.footprint.lo, lot.b.footprint.hi), spot.turns);
                    let (old_a, old_b) = (old_a + flat(origin), old_b + flat(origin));
                    let more: Vec<IVec2> = (a.y..=b.y).flat_map(|z| (a.x..=b.x).map(move |x| IVec2::new(x, z))).filter(|c| !(c.cmpge(old_a).all() && c.cmple(old_b).all())).collect();
                    if !self.grid.any(&more, WAY | PLOT | THING | KEEP) {
                        lot.b = wheeled.building.clone();
                        own = Some(wheeled);
                    }
                }
            }
        }
        let (t, b) = (spot.turns, &lot.b);
        let garden = lot.garden.map(|g| {
            let (a, b) = turn_rect(g, t);
            (a + flat(origin), b + flat(origin))
        });
        let yard = lot.yard.map(|g| {
            let (a, b) = turn_rect(g, t);
            (a + flat(origin), b + flat(origin))
        });
        let owner = Some(self.raised.len());
        for &(at, r) in &spot.discs {
            let mut pad = Pad::plain(at.x, at.y, r, spot.y);
            pad.walled = true;
            pad.neglect = self.neglect * 0.5;
            pad.garden = garden;
            net.pads.push(pad);
            self.discs.push(Disc { at, r, y: Some(spot.y), owner });
            self.grid.disc(at, r, PLOT);
        }
        // Who lives there, and what is in it.
        let children = want.residents.saturating_sub(2);
        let household = Household {
            region: self.s.region,
            sleepers: want.residents,
            children,
            trade: match want.purpose {
                Purpose::Workplace(l) => Some(l),
                Purpose::Smithy => Some(Livelihood::Smithy),
                Purpose::Sawmill => Some(Livelihood::Sawmill),
                _ => None,
            },
            wealth: want.wealth,
            neglect: self.neglect,
            // A store is kept, though nobody lives in it (`furnish` stocks it).
            lived_in: want.residents > 0 || want.purpose == Purpose::Store,
        };
        // Stage 3's own buildings come furnished, their beds and fires among them.
        let inside = if own.is_some() { furnish::Furnished::default() } else { furnish::furnish(b, &household, &self.pal, want.seed) };
        let (beds, seats) = match &own {
            Some(o) => (own_beds(o, origin, t), o.seats.iter().map(|&(q, d)| (origin + rotate(q, t), flat(rotate(d, t)))).collect()),
            None => rests(&inside, origin, t),
        };
        let mut voxels: Vec<(IVec3, Voxel)> = b.voxels.iter().chain(&inside.voxels).map(|&(q, v)| (origin + rotate(q, t), v)).collect();
        voxels.shrink_to_fit();
        let mut fine = parts::place_fine(&b.fine, origin, t);
        fine.extend(parts::place_fine(&inside.fine, origin, t));
        net.add_stamp_with(voxels, fine);
        if let Some(own) = &own {
            net.guest_beds.extend(own.guest_beds_at(origin, t));
        }
        if inside.fire.is_some() || own.as_ref().is_some_and(|o| o.fire.is_some()) {
            let q = origin + rotate(b.smoke, t);
            net.chimneys.push(Vec3::new(q.x as f32 + 0.5, q.y as f32, q.z as f32 + 0.5));
        }
        let (lo, hi) = turn_rect((b.footprint.lo, b.footprint.hi), t);
        let walls = turn_rect((b.walls.lo, b.walls.hi), t);
        self.plots.push(Plot {
            settlement: self.k,
            purpose: want.purpose,
            step: want.step,
            origin,
            turns: t,
            door: origin + rotate(b.door.floor, t),
            facing: flat(rotate(b.door.facing, t)),
            lo: lo + flat(origin),
            hi: hi + flat(origin),
            walls: (walls.0 + flat(origin), walls.1 + flat(origin)),
            round: b.style.form == kit::Form::Round,
            // Those who live in one of stage 3's own: its beds, but for a tavern's guests.
            sleepers: own.as_ref().map_or(want.residents, |o| b.inside.beds.len().saturating_sub(o.guest_beds.len()) as u8),
            pads: spot.discs.clone(),
            garden,
            yard,
            // A coop set beside the home as it was built; the yard's, if any, is found
            // as it is dressed.
            coop: coop.map(|q| origin + rotate(q, t)),
            beds,
            seats,
        });
        self.raised.push(Raised { want, lot, spot, origin });
    }

    /// A worn path from each door out to its lane.
    fn door_paths(&mut self, net: &mut Network) {
        for i in 0..self.raised.len() {
            let Raised { lot, spot, origin, .. } = &self.raised[i];
            let (b, t) = (&lot.b, spot.turns);
            let step = world(*origin, IVec2::new(b.footprint.hi.x + 1, b.door.floor.z), t).as_vec2() + 0.5;
            let out = flat(rotate(b.door.facing, t)).as_vec2();
            let start = step + out * 2.0;
            // Points of the lane it faces, nearest first; for a plot off the lanes, the
            // point of the nearest lane it was turned to face.
            let targets: Vec<Vec2> = if spot.lane == usize::MAX {
                vec![spot.toward]
            } else {
                let mut pts = self.lanes[spot.lane].pts.clone();
                pts.sort_by(|a, b| a.distance_squared(start).total_cmp(&b.distance_squared(start)));
                pts.truncate(12);
                pts
            };
            // The first it can reach without crossing another plot.
            let own = |d: &Disc| d.owner == Some(i) || d.y.is_none() || d.at == self.centre;
            let clear = |pts: &[Vec2]| !pts.iter().any(|p| self.discs.iter().any(|d| !own(d) && d.at.distance(*p) < d.r - 0.5));
            let Some((to, pts)) = targets.iter().map(|&to| (to, route::resample(&[step, start, to]))).find(|(to, pts)| to.distance(start) >= 1.0 && clear(pts)) else {
                continue;
            };
            let to_y = self.worn.nearest(to, 3.0).map_or(self.green.y, |q| q.y);
            let pad_y = spot.y;
            let on_pad = |p: &Vec2| spot.discs.iter().any(|&(c, r)| c.distance(*p) < r);
            let out_len = pts.iter().filter(|p| !on_pad(p)).count().max(1) as f32;
            let mut k = 0.0;
            let points: Vec<TrailPoint> = pts
                .iter()
                .map(|p| {
                    let y = if on_pad(p) {
                        pad_y
                    } else {
                        k += 1.0;
                        pad_y + (to_y - pad_y) * (k / out_len)
                    };
                    TrailPoint { x: p.x, z: p.y, y, half_width: DOOR_PATH }
                })
                .collect();
            net.add_points(&points);
            for p in &points {
                self.worn.add(Point { at: vec2(p.x, p.z), hw: DOOR_PATH, y: p.y, door: true });
                self.grid.disc(vec2(p.x, p.z), DOOR_PATH + 1.0, WAY);
            }
        }
    }

    /// A fitting built of the place's materials, worn or broken as its neglect has it.
    fn fitting(&mut self, kind: Fitting, seed: u32) -> Fitted {
        let decayed = self.rng.unit() < self.neglect * 0.5;
        let kind = kind.local(self.s.region);
        fittings::build_with(kind, &self.pal, seed, decayed, fittings::herbs_of(self.s.region))
    }

    /// Holds a fitting set at `at` (continent-local) with `turns`, with any cloth hung on
    /// it, back with the props until the lanes are worn.
    fn hold(&mut self, net: &mut Network, f: &Fitted, at: IVec3, turns: u32, extra: Vec<PlantBox>) {
        let placed = f.place(at, turns);
        let foot = placed.footprint();
        let mut fine = placed.fine;
        fine.extend(parts::place_fine(&extra, at, turns));
        for &c in &foot {
            self.grid.mark(c, THING);
        }
        net.held_props.push((placed.voxels, fine, foot));
        self.layout.things.push((f.kind, at, turns));
    }

    /// Sets out a plot's garden, its trade's things in its yard, and what stands
    /// against its walls.
    fn dress(&mut self, net: &mut Network, r: &Raised, k: usize) {
        let (want, t, origin) = (r.want, r.spot.turns, r.origin);
        let b = &r.lot.b;
        // What the building takes in its model space: its solid voxels at the height of
        // things set about it, and the ways in that must stay clear.
        let mut solid: HashSet<IVec2> = HashSet::new();
        for (q, v) in &b.voxels {
            if *v != AIR && (0..6).contains(&q.y) {
                solid.insert(flat(*q));
            }
        }
        for &(lo, hi) in &b.clear {
            if lo.y <= 5 && hi.y >= 0 {
                for z in lo.z..=hi.z {
                    for x in lo.x..=hi.x {
                        solid.insert(IVec2::new(x, z));
                    }
                }
            }
        }
        // The way out of the door, kept clear for a few steps.
        let (door_z, front) = (b.door.floor.z, b.footprint.hi.x);
        for x in front - 1..=front + 4 {
            for z in door_z - 2..=door_z + 2 {
                solid.insert(IVec2::new(x, z));
            }
        }
        let (garden, yard) = (r.lot.garden, r.lot.yard);
        let anchors: Vec<(AnchorKind, IVec3, IVec3)> = b.anchors.iter().map(|a| (a.kind, a.at, a.normal)).collect();
        let mut taken: Vec<(IVec2, IVec2)> = Vec::new();
        let seed = want.seed;
        if let Some(g) = garden {
            taken.push((g.0 - IVec2::ONE, g.1 + IVec2::ONE));
            self.garden(net, g, origin, t, seed);
        }
        let region = self.s.region;
        // Things of the trade in the yard.
        let mut yard_things: Vec<Fitting> = match want.purpose {
            Purpose::Workplace(l) => match l {
                Livelihood::Woodcutter => vec![Fitting::Sawhorse, Fitting::LogDeck],
                Livelihood::Forager => vec![Fitting::HerbRack, Fitting::HerbRack],
                Livelihood::Fisher => {
                    let mut v = vec![Fitting::NetRack, Fitting::FishRack, Fitting::EelTrap];
                    if self.s.coast {
                        v.push(Fitting::LobsterPot);
                    }
                    v
                }
                Livelihood::Hunter => vec![Fitting::HideFrame, Fitting::AntlerRack, Fitting::HideFrame],
                Livelihood::Weaver | Livelihood::Fletcher => vec![Fitting::SoakingTrough, Fitting::WithyBundles, Fitting::BasketStack],
                Livelihood::Apiary => {
                    let hive = if region == Region::Temperate && self.s.prosperity > 0.7 { Fitting::BoxHive } else { Fitting::Skep };
                    vec![hive; 4]
                }
                _ => Vec::new(),
            },
            Purpose::Smithy => vec![Fitting::Anvil, Fitting::Grindstone],
            Purpose::Sawmill => vec![Fitting::LogDeck, Fitting::Sawhorse, Fitting::LogDeck],
            Purpose::Granary => vec![Fitting::HayRick, Fitting::Handcart],
            Purpose::Tavern => vec![Fitting::WaterTrough, Fitting::Handcart],
            Purpose::Home => {
                let pick = [Fitting::ChickenCoop, Fitting::WashingLine, Fitting::Handcart, Fitting::WashingLine, Fitting::WaterTrough, Fitting::HayRick];
                let mut v = vec![pick[(seed % pick.len() as u32) as usize]];
                if unit(seed ^ 0x2c) < 0.5 {
                    v.push(pick[((seed >> 8) % pick.len() as u32) as usize]);
                }
                v
            }
            _ => Vec::new(),
        };
        // Bees are kept at the sunny edge, a hive or two even by a home with a garden.
        if want.purpose == Purpose::Home && self.s.has(Livelihood::Apiary) && unit(seed ^ 0xbee) < 0.25 {
            yard_things.push(Fitting::Skep);
        }
        let mut areas: Vec<(IVec2, IVec2)> = Vec::new();
        if let Some(y) = yard {
            areas.push(y);
        }
        for (n, kind) in yard_things.into_iter().enumerate() {
            let s = hash2(seed ^ 0x7a2d, n as i32, 3);
            let Some((at, ft)) = areas.iter().find_map(|&a| self.room_in(kind.local(region), a, &r.lot, &solid, &taken, origin, t, s)) else { continue };
            let (lo, hi) = kind.local(region).room(at, ft);
            taken.push((flat(lo), flat(hi) - IVec2::ONE));
            let f = self.fitting(kind, s);
            if kind == Fitting::ChickenCoop && !f.decayed {
                self.plots[k].coop = Some(origin + rotate(at, t));
            }
            let extra = if kind == Fitting::WashingLine && !f.decayed { laundry(&f, s) } else { Vec::new() };
            self.hold(net, &f, origin + rotate(at, t), (ft + t) & 3, extra);
        }
        if want.purpose == Purpose::Workplace(Livelihood::Woodcutter) {
            // The chopping block, by the log deck.
            let at = r.lot.yard.map(|(a, b)| IVec2::new((a.x + b.x) / 2, (a.y + b.y) / 2));
            if let Some(at) = at
                && !taken.iter().any(|&(lo, hi)| at.cmpge(lo).all() && at.cmple(hi).all())
            {
                let w = world(origin, at, t);
                if self.grid.get(w) & (WAY | THING) == 0 {
                    let decayed = self.rng.unit() < self.neglect * 0.5;
                    let p = props::build(Prop::ChoppingBlock, &self.pal, IVec3::new(w.x, origin.y, w.y), t, decayed, seed);
                    let foot = p.footprint();
                    for &c in &foot {
                        self.grid.mark(c, THING);
                    }
                    net.held_props.push((p.voxels, p.fine, foot));
                }
            }
        }
        // Against the walls: a log stack along one, a butt at a corner catching the rain,
        // and the forager's bunches drying under the eaves.
        let walls: Vec<(Fitting, AnchorKind, f32)> = vec![
            (Fitting::LogStack, AnchorKind::WallFace, if matches!(region, Region::Northern | Region::Alpine) || want.purpose == Purpose::Workplace(Livelihood::Woodcutter) { 0.9 } else { 0.45 }),
            (Fitting::LogStack, AnchorKind::WallFace, if want.purpose == Purpose::Workplace(Livelihood::Woodcutter) { 0.9 } else { 0.0 }),
            (Fitting::WaterButt, AnchorKind::Corner, 0.5),
            (Fitting::HerbBunches, AnchorKind::Eaves, if want.purpose == Purpose::Workplace(Livelihood::Forager) { 1.0 } else { 0.0 }),
            (Fitting::HerbBunches, AnchorKind::Eaves, if want.purpose == Purpose::Workplace(Livelihood::Forager) { 1.0 } else { 0.0 }),
        ];
        let mut used: Vec<IVec3> = Vec::new();
        for (n, (kind, anchor, chance)) in walls.into_iter().enumerate() {
            let s = hash2(seed ^ 0x3a11, n as i32, 5);
            if unit(s) >= chance || want.purpose == Purpose::Store && kind != Fitting::LogStack {
                continue;
            }
            let kind = kind.local(region);
            let mut spots: Vec<&(AnchorKind, IVec3, IVec3)> = anchors.iter().filter(|a| a.0 == anchor && !used.contains(&a.1)).collect();
            spots.sort_by_key(|a| hash2(s, a.1.x, a.1.z));
            for &&(_, at, normal) in &spots {
                // Its back to the wall, its front facing out.
                let n2 = if normal.x != 0 && normal.z != 0 { IVec2::new(normal.x, 0) } else { flat(normal) };
                let ft = facing(n2.as_vec2());
                let size = kind.size();
                let at = if kind.stand() == fittings::Stand::Hung { IVec3::new(at.x, at.y - size.y + 1, at.z) } else { IVec3::new(at.x, 0, at.z) };
                let (lo, hi) = kind.room(at, ft);
                let cols: Vec<IVec2> = (lo.z..hi.z).flat_map(|z| (lo.x..hi.x).map(move |x| IVec2::new(x, z))).collect();
                let hung = kind.stand() == fittings::Stand::Hung;
                let blocked = cols.iter().any(|c| {
                    !(hung || r.lot.level(*c))
                        || (!hung && solid.contains(c))
                        || taken.iter().any(|&(a, b)| c.cmpge(a).all() && c.cmple(b).all())
                });
                // Hung things keep clear of the walls' solid voxels at their own height.
                let blocked = blocked
                    || (hung && b.voxels.iter().any(|(q, v)| *v != AIR && q.y >= lo.y && q.y < hi.y && cols.contains(&flat(*q))));
                let world_cols: Vec<IVec2> = cols.iter().map(|&c| world(origin, c, t)).collect();
                if blocked || self.grid.any(&world_cols, WAY | THING) {
                    continue;
                }
                used.push(at);
                taken.push((flat(lo), flat(hi) - IVec2::ONE));
                let f = self.fitting(kind, s);
                self.hold(net, &f, origin + rotate(at, t), (ft + t) & 3, Vec::new());
                break;
            }
        }
    }

    /// Room in `area` (model columns, inclusive) of a plot for a fitting of `kind`: where
    /// it keeps off the building and its ways in, what is set out already, and any way
    /// worn across the plot. Tried from a corner the seed chooses.
    #[allow(clippy::too_many_arguments)]
    fn room_in(&self, kind: Fitting, area: (IVec2, IVec2), lot: &Lot, solid: &HashSet<IVec2>, taken: &[(IVec2, IVec2)], origin: IVec3, t: u32, seed: u32) -> Option<(IVec3, u32)> {
        let (lo, hi) = area;
        if hi.x < lo.x || hi.y < lo.y {
            return None;
        }
        let flip = (seed & 1 != 0, seed & 2 != 0);
        let turns = [(seed >> 2) & 3, ((seed >> 2) + 1) & 3, ((seed >> 2) + 2) & 3, ((seed >> 2) + 3) & 3];
        for ft in turns {
            for j in 0..=(hi.y - lo.y) {
                for i in 0..=(hi.x - lo.x) {
                    let x = if flip.0 { hi.x - i } else { lo.x + i };
                    let z = if flip.1 { hi.y - j } else { lo.y + j };
                    let (a, b) = kind.room(IVec3::new(x, 0, z), ft);
                    let (a, b) = (flat(a), flat(b) - IVec2::ONE);
                    if !(a.cmpge(lo).all() && b.cmple(hi).all()) {
                        continue;
                    }
                    if taken.iter().any(|&(p, q)| a.cmple(q).all() && b.cmpge(p).all()) {
                        continue;
                    }
                    let cols: Vec<IVec2> = (a.y..=b.y).flat_map(|z| (a.x..=b.x).map(move |x| IVec2::new(x, z))).collect();
                    if cols.iter().any(|c| solid.contains(c) || !lot.level(*c)) {
                        continue;
                    }
                    let world_cols: Vec<IVec2> = cols.iter().map(|&c| world(origin, c, t)).collect();
                    if self.grid.any(&world_cols, WAY | THING) {
                        continue;
                    }
                    return Some((IVec3::new(x, 0, z), ft));
                }
            }
        }
        None
    }

    /// A home's garden: its bed dug over in ridges and furrows with greens growing along
    /// the ridges, and wattle hurdles round it but on the side toward the house.
    fn garden(&mut self, net: &mut Network, (lo, hi): (IVec2, IVec2), origin: IVec3, t: u32, seed: u32) {
        let mut voxels = Vec::new();
        let mut fine = Vec::new();
        let mut foot = Vec::new();
        let top = origin.y - 1;
        let crop = if unit(seed ^ 0xc409) < 0.5 { mat::TUFT_LUSH } else { mat::TUFT };
        for z in lo.y..=hi.y {
            for x in lo.x..=hi.x {
                let w = world(origin, IVec2::new(x, z), t);
                let ridge = (x - lo.x) % 2 == 0;
                voxels.push((IVec3::new(w.x, top, w.y), if ridge { mat::DIRT } else { mat::MUD }));
                foot.push(w);
                // Greens along the ridges, a few gone to the pot already.
                if ridge && (z - lo.y) % 2 == 0 && unit(hash2(seed ^ 0x9e3, x, z)) < 0.8 {
                    let (cx, cy, cz) = (w.x * CELLS, (top + 1) * CELLS, w.y * CELLS);
                    let tall = unit(hash2(seed ^ 0x9e4, x, z));
                    let (wide, high) = if crop == mat::TUFT { (2, 5 + (tall * 3.0) as i32) } else { (5, 3 + (tall * 2.0) as i32) };
                    let a = (CELLS - wide) / 2;
                    fine.push(PlantBox { lo: [cx + a, cy, cz + a], hi: [cx + a + wide, cy + high, cz + a + wide], voxel: crop, sway_lo: false, sway_hi: true, thin: 0, season: 0, flap: 0 });
                }
            }
        }
        for &c in &foot {
            self.grid.mark(c, THING);
        }
        net.held_props.push((voxels, fine, foot));
        // Hurdles along the back and the two sides.
        let mut runs: Vec<(IVec3, u32)> = Vec::new();
        let mut z = lo.y - 1;
        while z + 4 <= hi.y + 1 {
            runs.push((IVec3::new(lo.x - 1, 0, z), 1));
            z += 5;
        }
        for side in [lo.y - 1, hi.y + 1] {
            let mut x = lo.x;
            while x + 4 <= hi.x + 1 {
                runs.push((IVec3::new(x, 0, side), 0));
                x += 5;
            }
        }
        for (n, (at, ft)) in runs.into_iter().enumerate() {
            let s = hash2(seed ^ 0x40d1, n as i32, 1);
            let f = self.fitting(Fitting::Hurdle, s);
            self.hold(net, &f, origin + rotate(at, t), (ft + t) & 3, Vec::new());
        }
    }

    /// The water point: the settlement's wells, the first on its green and any more
    /// beside its lanes. Where water is near there are none but in a big town, and the
    /// worn way down to the water serves.
    fn wells(&mut self, net: &mut Network, fin: &Sampler) {
        if self.s.wells == 0 {
            return;
        }
        let kind = match self.s.region {
            Region::Desert if self.s.reason == Reason::Oasis => Fitting::OasisWell,
            Region::Desert => Fitting::Shaduf,
            _ => Fitting::StoneWell,
        };
        let mut left = self.s.wells as usize;
        let seed = self.s.seed ^ 0x3e11;
        // On the green, off the ways across it.
        let gy = self.green.y as i32 + 1;
        'green: for f in [0.5, 0.65, 0.35] {
            for j in 0..24 {
                let a = unit(seed) * std::f32::consts::TAU + j as f32 / 24.0 * std::f32::consts::TAU;
                let p = self.centre + Vec2::from_angle(a) * self.green.radius * f;
                let ft = facing(self.centre - p);
                if self.set_fitting(net, kind, IVec3::new(p.x.floor() as i32, gy, p.y.floor() as i32), ft, hash2(seed, j, 1), GREEN, None) {
                    left -= 1;
                    break 'green;
                }
            }
        }
        // The rest beside the lanes further out, each on level ground.
        let lanes: Vec<(Vec<Vec2>, f32)> = self.lanes.iter().filter(|l| matches!(l.way, Way::Loop | Way::Arrival { .. } | Way::Street)).map(|l| (l.pts.clone(), l.hw[0])).collect();
        let mut tries = 0u32;
        'more: for (pts, hw) in &lanes {
            for i in (12..pts.len().saturating_sub(4)).step_by(9) {
                if left == 0 {
                    break 'more;
                }
                tries += 1;
                let t = (pts[(i + 2).min(pts.len() - 1)] - pts[i - 2]).normalize_or_zero();
                let side = if tries.is_multiple_of(2) { 1.0 } else { -1.0 };
                let nrm = vec2(-t.y, t.x) * side;
                // Its middle far enough off the lane that all of it stands clear of the
                // worn ground.
                let size = kind.size();
                let p = pts[i] + nrm * (hw + 2.5 + size.x.max(size.z) as f32 * 0.75);
                if self.layout.wells.iter().any(|w| vec2(w.x as f32, w.z as f32).distance(p) < m(40.0)) {
                    continue;
                }
                let ft = facing(-nrm);
                let (lo, hi) = kind.room(IVec3::ZERO, ft);
                let mid = (lo + hi).as_vec3() * 0.5;
                let at = IVec2::new((p.x - mid.x).floor() as i32, (p.y - mid.z).floor() as i32);
                let (lo, hi) = kind.room(IVec3::new(at.x, 0, at.y), ft);
                let tops: Vec<i32> = (lo.z..hi.z).flat_map(|z| (lo.x..hi.x).map(move |x| (x, z))).map(|(x, z)| fin.column(x, z).top()).collect();
                let (a, b) = (tops.iter().min().copied().unwrap_or(0), tops.iter().max().copied().unwrap_or(0));
                if b - a > 1 || (lo.z..hi.z).any(|z| (lo.x..hi.x).any(|x| fin.column(x, z).water_surface().is_some())) {
                    continue;
                }
                // Set into the higher side of a slight slope rather than standing proud
                // of the lower.
                if self.set_fitting(net, kind, IVec3::new(at.x, a + 1, at.y), ft, hash2(seed, i as i32, 2), PLOT | GREEN | KEEP, None) {
                    left -= 1;
                }
            }
        }
    }

    /// Sets a fitting with its column 0 at `at` (continent-local, its foot on `at.y`) if
    /// its room and a column round it are free of ways, things and the flags in `avoid`;
    /// whether it did. `wells` and `stalls` record it in the layout.
    #[allow(clippy::too_many_arguments)]
    fn set_fitting(&mut self, net: &mut Network, kind: Fitting, at: IVec3, ft: u32, seed: u32, avoid: u8, cloth: Option<Voxel>) -> bool {
        let (lo, hi) = kind.room(at, ft);
        let cols: Vec<IVec2> = (lo.z - 1..=hi.z).flat_map(|z| (lo.x - 1..=hi.x).map(move |x| IVec2::new(x, z))).collect();
        let need = if avoid == GREEN { WAY | THING | PLOT | KEEP } else { WAY | THING | avoid };
        if self.grid.any(&cols, need) {
            return false;
        }
        // On the green, wholly on it.
        if avoid == GREEN && cols.iter().any(|&c| self.grid.get(c) & GREEN == 0) {
            return false;
        }
        let f = self.fitting(kind, seed);
        let extra = match (kind, cloth) {
            (Fitting::MarketStall, Some(colour)) if !f.decayed => awning(&f, colour),
            _ => Vec::new(),
        };
        self.hold(net, &f, at, ft, extra);
        match kind {
            Fitting::MarketStall => self.layout.stalls.push(at),
            _ => self.layout.wells.push(at),
        }
        true
    }

    /// Market stalls about the green, facing in across it: a couple in a village, more
    /// in a town.
    fn stalls(&mut self, net: &mut Network) {
        let want = match self.s.kind {
            Kind::Hamlet => 0,
            Kind::Village => 2,
            Kind::LargeVillage => 3,
            Kind::Town => 5,
            Kind::GreatTown => 7,
        };
        let seed = self.s.seed ^ 0x57a1;
        let y = self.green.y as i32 + 1;
        let mut set = 0;
        let start = unit(seed) * std::f32::consts::TAU;
        for f in [0.68, 0.5] {
            for j in 0..36 {
                if set >= want {
                    return;
                }
                let a = start + j as f32 / 36.0 * std::f32::consts::TAU;
                let p = self.centre + Vec2::from_angle(a) * self.green.radius * f;
                let ft = facing(self.centre - p);
                let colour = CLOTHS[(hash2(seed, j, 3) % CLOTHS.len() as u32) as usize];
                if self.set_fitting(net, Fitting::MarketStall, IVec3::new(p.x.floor() as i32, y, p.y.floor() as i32), ft, hash2(seed, j, 2), GREEN, Some(colour)) {
                    set += 1;
                }
            }
        }
    }

    /// Lanterns along the main lanes of a large village or a town, every dozen metres or
    /// so either side in turn, most of them unlit, as by the trails.
    fn lanterns(&mut self, net: &mut Network) {
        if self.s.kind < Kind::LargeVillage {
            return;
        }
        let style = match (self.s.kind, self.s.age) {
            (Kind::GreatTown, Age::Ancient) => LampStyle::Stone,
            (Kind::LargeVillage, _) => LampStyle::Crook,
            _ => LampStyle::Post,
        };
        let salt = 0x1a47_0000 ^ ((self.k as u32) << 4);
        let lanes: Vec<(Vec<Vec2>, Vec<f32>)> = self
            .lanes
            .iter()
            .filter(|l| matches!(l.way, Way::Arrival { .. } | Way::Street) || (l.way == Way::Loop && self.s.kind >= Kind::Town))
            .map(|l| (l.pts.clone(), l.hw.clone()))
            .collect();
        for (n, (pts, hw)) in lanes.iter().enumerate() {
            let trail = salt + n as u32;
            let mut side = if n % 2 == 0 { 1.0 } else { -1.0 };
            let mut since = 20usize;
            for i in 4..pts.len().saturating_sub(4) {
                since += 1;
                if since < 15 || pts[i].distance(self.centre) < self.green.radius + 6.0 {
                    continue;
                }
                let t = (pts[i + 4] - pts[i - 4]).normalize_or_zero();
                let nrm = vec2(-t.y, t.x) * side;
                let q = pts[i] + nrm * (hw[i] + m(1.0));
                let (x, z) = (q.x.floor() as i32, q.y.floor() as i32);
                let Some(lane) = self.worn.nearest(pts[i], 3.0) else { continue };
                let y = lane.y.floor() as i32 + 1;
                let turns = turns_toward(-nrm);
                let h = hash2(trail, x, z);
                let lit = unit(h ^ 0x11e7) < 0.3;
                let height = 3 + (unit(h) * 2.0) as i32;
                let (voxels, fine) = landmarks::trail_lantern_styled(&self.pal, height, h, style, turns, parts::GATE_HALF, lit);
                let mut footprint: Vec<IVec2> = voxels.iter().filter(|(p, _)| (0..=2).contains(&p.y)).map(|(p, _)| IVec2::new(x + p.x, z + p.z)).collect();
                footprint.insert(0, IVec2::new(x, z));
                let around: Vec<IVec2> = footprint.iter().flat_map(|c| [*c, *c + IVec2::X, *c - IVec2::X, *c + IVec2::Y, *c - IVec2::Y]).collect();
                if self.grid.any(&around, PLOT | THING | KEEP) || self.grid.any(&footprint, WAY) {
                    continue;
                }
                if net.lanterns.iter().any(|l| l.at.distance_squared(q) < m(9.0).powi(2)) {
                    continue;
                }
                for &c in &footprint {
                    self.grid.mark(c, THING);
                }
                let at = IVec3::new(x, y, z);
                let voxels = voxels.into_iter().map(|(p, v)| (p + at, v)).collect();
                net.held_lanterns.push((net.lanterns.len(), voxels, parts::place_fine(&fine, at, 0)));
                net.lanterns.push(TrailLantern { at: q, trail, step: i, cause: LanternCause::Lane, style, lit, footprint });
                since = 0;
                side = -side;
            }
        }
    }
}

/// Where people lie and sit in a furnished building whose model origin stands at `origin`,
/// turned by `turns`: the beds and seats of [`furnish::Furnished`], continent-local. This
/// is the one place a plot's beds are found (#204): a building furnished another way (a
/// special building of stage 3) fills `Plot::beds` and `Plot::seats` from its own here.
fn rests(inside: &furnish::Furnished, origin: IVec3, turns: u32) -> (Vec<(Vec3, IVec2)>, Vec<(IVec3, IVec2)>) {
    let way = |d: IVec2| flat(rotate(IVec3::new(d.x, 0, d.y), turns));
    // Cells turn about the middle of model voxel 0, as fine parts do.
    let c = IVec3::new(CELLS / 2, 0, CELLS / 2);
    let beds = inside.lying().into_iter().map(|(head, toward)| ((rotate(head - c, turns) + c + origin * CELLS).as_vec3() / CELLS as f32, way(toward))).collect();
    let seats = inside.seats().into_iter().map(|(at, toward)| (origin + rotate(at, turns), way(toward))).collect();
    (beds, seats)
}

/// Where the household of one of stage 3's own buildings lies, as [`rests`] gives it:
/// each of its beds but the guests', the head at the bed's first end, on top of it.
fn own_beds(own: &Built, origin: IVec3, turns: u32) -> Vec<(Vec3, IVec2)> {
    let beds = &own.building.inside.beds;
    let theirs = beds.len().saturating_sub(own.guest_beds.len());
    beds[..theirs]
        .iter()
        .map(|s| {
            let (at, along, facing) = (rotate(s.at, turns), rotate(s.along, turns), rotate(s.facing, turns));
            let head = (origin + at).as_vec3() + Vec3::new(0.5, 1.0, 0.5) + facing.as_vec3() * ((s.wide - 1) as f32 * 0.5);
            (head, flat(along))
        })
        .collect()
}

/// Whether the way from `a` to `b` passes within `r` of `c`.
fn crosses(a: Vec2, b: Vec2, c: Vec2, r: f32) -> bool {
    let ab = b - a;
    let t = ((c - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
    (a + ab * t).distance(c) < r
}

/// Washing pegged out on a washing line: shirts and sheets in the fitting's model cells,
/// hung from its line, their free edges fluttering in the wind as flags do.
fn laundry(f: &Fitted, seed: u32) -> Vec<PlantBox> {
    let Some(&Hook::Line { from, to, sag }) = f.hooks.iter().find(|h| matches!(h, Hook::Line { .. })) else { return Vec::new() };
    let mut rng = Rng::new(seed ^ 0x1a0d);
    let (x0, x1) = (from.x.min(to.x), from.x.max(to.x));
    // The line sags from either post to its lowest in the middle.
    let mid = (x0 + x1) as f32 * 0.5;
    let line_y = |x: f32| from.y as f32 - sag as f32 * (1.0 - ((x - mid) / (mid - x0 as f32)).abs()) - 0.5;
    let mut out = Vec::new();
    let mut x = x0 + 2 + rng.int(0, 2);
    while x < x1 - 4 {
        // A shirt, a sheet or a smaller cloth, each its own colour.
        let (wide, drop) = match rng.int(0, 3) {
            0 => (5, 6),
            1 => (rng.int(9, 12), rng.int(8, 10)),
            _ => (4, 4),
        };
        if x + wide > x1 - 1 {
            break;
        }
        if rng.unit() < 0.75 {
            let top = (x..x + wide).map(|c| line_y(c as f32 + 0.5).floor() as i32).min().unwrap_or(from.y);
            let colour = CLOTHS[(rng.next_u32() % CLOTHS.len() as u32) as usize];
            if top - drop >= 2 {
                out.push(PlantBox { lo: [x, top - drop, from.z], hi: [x + wide, top, from.z + 1], voxel: colour, sway_lo: true, sway_hi: false, thin: 0, season: 0, flap: 1 });
            }
        }
        x += wide + rng.int(1, 3);
    }
    out
}

/// An awning of cloth stretched over a market stall's rails, in its model cells.
fn awning(f: &Fitted, colour: Voxel) -> Vec<PlantBox> {
    let Some(&Hook::Awning { back, front }) = f.hooks.iter().find(|h| matches!(h, Hook::Awning { .. })) else { return Vec::new() };
    let corners = [back, IVec3::new(front.x, back.y, back.z), front, IVec3::new(back.x, front.y, front.z)];
    cloth::sheet(corners, colour)
}

// Stage 3's edges (#203): the gates and old walls, the water side and the park.
mod edges;
mod gates;
mod park;
mod port;
pub use edges::{Gate, Harbour, Park, Shed};
#[cfg(test)]
mod edge_tests;

#[cfg(test)]
mod tests;
