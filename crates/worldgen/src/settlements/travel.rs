//! People on the move (#205): lone campers beside the long trails, settlers' camps on the
//! roads between settlements, trade boats on the rivers between partners, and fishing
//! boats out from the ports.
//!
//! Nothing here is simulated, and nothing is saved. Where everyone is follows from the
//! world's seed, the day and the hour alone, worked out afresh whenever it is asked, so
//! that every player sees the same travellers in the same places (#36) and someone met on
//! one walk is met again on the next. A day's plan for each party ([`Plan`]) is found once
//! for the day, which is where the ground is looked at (for a dry, level spot to camp,
//! clear of the trail), and where the party is at any hour of it ([`Plan::at`]) is a
//! quick sum the game can do every frame.
//!
//! - A lone camper stops a week by one of the long stretches of trail, about one to each
//!   stretch, with a bedroll or a small tent and a fire: fishing if there is water by, or
//!   sketching, and asleep at night. The next week they are gone, and someone else may be
//!   by another stretch.
//! - Settlers, five to eight with a handcart or two, walk the trails from one settlement to
//!   another, along the lines the trails were laid as. They walk a set distance a day, and
//!   at night camp beside the road in an open spot, their carts drawn up by the fire.
//!   Where the country they come from keeps oxen or donkeys, one of those draws each cart,
//!   walking in its shafts with someone at its head (#206); elsewhere they pull the carts
//!   themselves.
//! - Trade boats row or sail between partner settlements on one river, from quay to quay
//!   (see [`super::quay`]), by day. Where the river falls or runs rough between them the
//!   crew land at a stage above it, carry their goods and then the boat along the path
//!   round it, and launch again from the stage below: a boat is never on the rough water
//!   itself. A voyage of more than a day ties up off the bank at evening, the crew camped
//!   ashore by their fire, and goes on in the morning. When the water turns rough the boat
//!   makes for the nearest bank, stage or quay it can tie up at, and waits there out the day.
//!   The world has no iron ore, so iron for the smithies comes this way: on some voyages a
//!   boat bound for a settlement with a smithy carries bars and blooms in its hold, and
//!   once it is in they wait stacked on the quay through the day after.
//! - Fishing boats put out from the ports and fishing settlements at dawn, fish their
//!   grounds through the morning and are back about midday; not in rough weather, and
//!   home at once if it turns rough while they are out.
//!
//! Distances walked and rowed are reckoned in the world's own hours: at the usual 40-minute
//! day, the settlers' pace of [`PACE`] is a walk of a little over a metre a second.

use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap},
};

use glam::{Vec2, Vec3, vec2};

use super::{
    Region,
    layout::Purpose,
    quay::{DRIFT, Portage, Quay, along_line, measure, water_ends},
};
use crate::{
    continent::Continent,
    fauna,
    macro_layout::MACRO_SIZE,
    noise::{fbm2, hash2, unit},
    scale::{m, m64},
    surface::{Column, Cover, Sampler},
    trails::Network,
};

/// Days in a week: a lone camper stays one.
pub const WEEK: i64 = 7;
/// How far settlers pulling handcarts walk in an hour of the world's day, in voxels.
pub const PACE: f32 = m(120.0);
/// When the settlers set off, stop to eat, go on, and stop for the night.
const SET_OFF: f32 = 8.0;
const HALT: f32 = 12.0;
const ON: f32 = 13.0;
const STOP: f32 = 17.5;
/// How long it takes them to walk off the road to their camp, or back to it, in hours.
const TURN_OFF: f32 = 0.25;
const WALK_HOURS: f32 = (HALT - SET_OFF) + (STOP - ON);
/// How far they walk in a day.
pub const DAY_WALK: f32 = PACE * WALK_HOURS;
/// The settlers' timetable comes round in spells of this many days: each party makes one
/// journey in a spell, or none.
const SPELL: i64 = 14;
/// The longest journey a party makes, in days, and the shortest road worth one.
const LONGEST_DAYS: i64 = SPELL - 2;
const SHORTEST_ROAD: f32 = m(500.0);
/// The shortest stretch of trail a lone camper stops by, and how far from any settlement.
const STRETCH: f32 = m(900.0);
const AWAY: f32 = m(200.0);
/// How likely a stretch is to have a camper by it in a week.
const CAMPER_CHANCE: f32 = 0.7;
/// How fast the boats go, in voxels an hour of the world's day: rowed, under sail, and
/// a fishing boat going out to its ground.
const ROW: f32 = m(150.0);
const SAIL: f32 = m(240.0);
/// The hours the trade boats are on the water.
const BOAT_DAY: (f32, f32) = (7.0, 19.0);
/// How long a boat takes to pull in from the middle of the river to a berth off the bank
/// or at a stage, or out again, in hours.
const TO_BANK: f32 = 0.3;
/// At a portage: how long landing at the first stage takes, and loading and launching from
/// the second, in hours; and how fast the crew carry their boat along the path, in voxels
/// an hour. The goods go round first, at a walk, there and back.
const LAND: f32 = 0.2;
const LAUNCH: f32 = 0.35;
const CARRY: f32 = m(50.0);
/// How far apart along a day's water the banks are looked for that a boat could make for
/// when the water turns rough.
const REFUGE: f32 = m(240.0);
/// Within this of a trail end, two trails are taken to meet.
const JOIN: f32 = m(8.0);

/// Which day of the world it is, counting the first day of year 0 as 0: the date as the
/// game keeps it (a fraction of the year, with the hour already counted in it) taken back
/// to the start of the day. A day turns at midnight.
pub fn day_of(year: u32, date: f32, hour: f32, year_days: f32) -> i64 {
    ((year as f64 + date as f64) * year_days.max(1.0) as f64 - hour as f64 / 24.0).round() as i64
}

/// About when the sun rises on `date` (a fraction of the year from the spring equinox):
/// earlier in summer, later in winter.
pub fn dawn(date: f32) -> f32 {
    6.0 - 1.4 * (date * std::f32::consts::TAU).sin()
}

/// A number from 0 to 1, the same for the same three.
fn roll(seed: u32, a: i64, b: i64) -> f32 {
    unit(hash2(seed, a as i32, b as i32))
}

/// What kind of travellers.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Kind {
    Camper,
    Settlers,
    RiverBoat,
    FishingBoat,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Camper, Kind::Settlers, Kind::RiverBoat, Kind::FishingBoat];

    /// One lower-case word for it, for the console.
    pub fn key(self) -> &'static str {
        match self {
            Kind::Camper => "camper",
            Kind::Settlers => "settlers",
            Kind::RiverBoat => "riverboat",
            Kind::FishingBoat => "fishingboat",
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Kind::Camper => "a lone camper",
            Kind::Settlers => "settlers",
            Kind::RiverBoat => "a trade boat",
            Kind::FishingBoat => "a fishing boat",
        }
    }

    /// The kind a word names, as [`Self::key`] gives it, or near enough.
    pub fn named(word: &str) -> Option<Kind> {
        let w = word.trim().to_ascii_lowercase().replace([' ', '-', '_', '\''], "");
        match w.as_str() {
            "camper" | "campers" | "camp" => Some(Kind::Camper),
            "settlers" | "settler" | "settlerscamp" => Some(Kind::Settlers),
            "riverboat" | "riverboats" | "tradeboat" | "boat" => Some(Kind::RiverBoat),
            "fishingboat" | "fishingboats" | "fishing" => Some(Kind::FishingBoat),
            _ => None,
        }
    }
}

/// What someone is doing, as far as how they are drawn goes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Doing {
    Walking,
    /// Walking between the shafts of a handcart.
    Pulling,
    Standing,
    /// By the fire, or in a boat.
    Sitting,
    Asleep,
    Fishing,
    Sketching,
    Rowing,
    /// Walking at the head of a beast drawing a cart, a hand on its halter.
    Leading,
    /// Walking with goods in their arms, round a portage.
    Laden,
    /// Walking under a boat held up over their heads, round a portage.
    Portaging,
    /// Hauling a boat along on rollers, or shoving it from behind.
    Hauling,
}

impl Doing {
    pub fn word(self) -> &'static str {
        match self {
            Doing::Walking => "walking",
            Doing::Pulling => "pulling a cart",
            Doing::Standing => "standing about",
            Doing::Sitting => "sitting",
            Doing::Asleep => "asleep",
            Doing::Fishing => "fishing",
            Doing::Sketching => "sketching",
            Doing::Rowing => "rowing",
            Doing::Leading => "leading a beast",
            Doing::Laden => "carrying goods",
            Doing::Portaging => "carrying a boat",
            Doing::Hauling => "hauling a boat",
        }
    }
}

/// How the water has been for the boats today: calm, or rough since an hour of the day
/// (0 for since before anyone was up). Once it has turned the boats wait out the day where
/// they tied up, even if it calms again.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Water {
    pub rough_since: Option<f32>,
}

impl Water {
    pub const CALM: Water = Water { rough_since: None };
    /// Rough all day.
    pub const ROUGH: Water = Water { rough_since: Some(0.0) };

    pub fn rough_since(hour: f32) -> Water {
        Water { rough_since: Some(hour) }
    }

    /// Whether the water is rough at `hour`.
    pub fn rough_at(self, hour: f32) -> bool {
        self.rough_since.is_some_and(|t| hour >= t)
    }
}

/// How old someone is.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Who {
    Child,
    Adult,
    Elder,
}

/// What someone is about, which the game reads as their trade.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Work {
    /// Moving house, or on the road with a party.
    Settler,
    /// Taking goods to trade.
    Merchant,
    Camper,
    Boatman,
    Fisher,
}

/// One traveller as they are at a moment: who, where their feet are, which way they face
/// (a camera's yaw) and what they are doing.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Figure {
    pub seed: u32,
    pub who: Who,
    pub work: Work,
    pub feet: Vec3,
    pub facing: f32,
    pub doing: Doing,
}

/// A handcart, its middle on the ground, the way its shafts point, and what it carries,
/// by a seed.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Cart {
    pub at: Vec3,
    pub heading: f32,
    pub load: u32,
    /// The beast in its shafts drawing it, by index in [`Scene::beasts`]; None for a cart
    /// pulled by hand, or let down while its beast is tethered.
    pub beast: Option<usize>,
}

/// A beast of burden a party has with it: an ox or a donkey drawing a cart, or tethered by
/// the carts in camp.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Beast {
    pub kind: fauna::Kind,
    pub seed: u32,
    /// The ground under its middle, and the way it faces.
    pub feet: Vec3,
    pub facing: f32,
    pub walking: bool,
    /// Head down to the grass.
    pub grazing: bool,
}

/// A boat: the middle of its hull on the surface, the way its bow points, whether it is a
/// sailing boat, and a seed for its cargo.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Boat {
    pub at: Vec3,
    pub heading: f32,
    pub sail: bool,
    pub load: u32,
    /// Under way, rather than tied up or lying still.
    pub moving: bool,
    /// Out of the water, being carried round a portage: `at` is then the ground under its
    /// middle, and a rowing boat is held up over its crew's heads, a sailing boat hauled
    /// on rollers.
    pub ashore: bool,
    /// Its goods aboard; not while they are carried round a portage.
    pub laden: bool,
    /// Iron among its goods, bars and blooms for the smithy of the settlement it is bound
    /// for (#205), until it is landed on the quay there.
    pub iron: bool,
}

/// A fire as it is: burning up, or low, a few embers and a thread of smoke.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fire {
    Lit,
    Low,
}

/// A party of travellers as it is at one moment, everything continent-local.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    /// Which party, the same for it all its days.
    pub id: u64,
    pub kind: Kind,
    /// Whose country they come from, for how they dress.
    pub region: Region,
    /// The settlement they come from and the one they are going to, by index in
    /// `Network::settlements`, where they have one.
    pub from: Option<usize>,
    pub to: Option<usize>,
    pub people: Vec<Figure>,
    /// Their fire, where it burns on the ground.
    pub fire: Option<(Vec3, Fire)>,
    /// Small tents and bedrolls, each its middle on the ground and the way it lies.
    pub tents: Vec<(Vec3, f32)>,
    pub bedrolls: Vec<(Vec3, f32)>,
    pub carts: Vec<Cart>,
    pub beasts: Vec<Beast>,
    pub boats: Vec<Boat>,
    /// Goods set down on the ground, each pile's middle and the way it lies: a boat's
    /// cargo carried round a portage ahead of it.
    pub goods: Vec<(Vec3, f32)>,
    /// Iron landed on a quay for the smithy, waiting to be carted up to it (#205): the
    /// stack's middle on the boards and the way it lies.
    pub iron: Vec<(Vec3, f32)>,
    /// What they are about, in a few words.
    pub doing: &'static str,
    /// The middle of the party.
    pub at: Vec3,
}

impl Scene {
    fn new(id: u64, kind: Kind, region: Region) -> Self {
        Scene {
            id,
            kind,
            region,
            from: None,
            to: None,
            people: Vec::new(),
            fire: None,
            tents: Vec::new(),
            bedrolls: Vec::new(),
            carts: Vec::new(),
            beasts: Vec::new(),
            boats: Vec::new(),
            goods: Vec::new(),
            iron: Vec::new(),
            doing: "",
            at: Vec3::ZERO,
        }
    }
}

/// A way along the trails: the points of its line, the half-width the trail is worn to
/// at each, and how far along it each is.
#[derive(Clone, Debug)]
pub struct Way {
    pub line: Vec<Vec2>,
    pub widths: Vec<f32>,
    pub along: Vec<f32>,
}

impl Way {
    fn new(line: Vec<Vec2>, widths: Vec<f32>) -> Self {
        let mut along = Vec::with_capacity(line.len());
        let mut s = 0.0;
        for (k, p) in line.iter().enumerate() {
            if k > 0 {
                s += p.distance(line[k - 1]);
            }
            along.push(s);
        }
        Way { line, widths, along }
    }

    pub fn length(&self) -> f32 {
        self.along.last().copied().unwrap_or(0.0)
    }

    /// The point `s` along, the way the line runs there, and the trail's half-width.
    pub fn at(&self, s: f32) -> (Vec2, Vec2, f32) {
        let n = self.line.len();
        if n < 2 {
            return (self.line.first().copied().unwrap_or(Vec2::ZERO), Vec2::X, 0.0);
        }
        let s = s.clamp(0.0, self.along[n - 1]);
        let k = self.along.partition_point(|&a| a <= s).clamp(1, n - 1);
        let (a, b) = (self.line[k - 1], self.line[k]);
        let t = ((s - self.along[k - 1]) / (self.along[k] - self.along[k - 1]).max(1e-4)).clamp(0.0, 1.0);
        let w = self.widths[k - 1] + (self.widths[k] - self.widths[k - 1]) * t;
        (a + (b - a) * t, (b - a).normalize_or(Vec2::X), w.max(m(0.3)))
    }
}

/// A road settlers walk: from one settlement to another along the trails.
#[derive(Clone, Debug)]
pub struct Road {
    pub a: usize,
    pub b: usize,
    pub way: Way,
}

/// What a continent's travellers go by, worked out once from its trails and waterways.
pub struct Travel {
    /// The continent's seed and the world's.
    seed: u32,
    world_seed: u32,
    /// The roads between pairs of settlements, short enough to walk in a spell.
    pub roads: Vec<Road>,
    /// The long stretches of trail away from the settlements, which campers stop by.
    pub stretches: Vec<Way>,
    /// How many parties of settlers are on the roads, one journey a spell each at most.
    pub parties: usize,
    /// The region of each settlement, and where each is.
    regions: Vec<Region>,
    towns: Vec<(Vec2, f32, i32)>,
    /// Whether each settlement has a smithy, which the boats bring iron to (#205).
    smithies: Vec<bool>,
}

/// One stretch of the trail graph between two of its nodes.
#[derive(Clone, Copy)]
enum Path {
    /// Along a trail's line from one of its points to another.
    Along { line: u32, from: u32, to: u32 },
    /// Straight across, where two trails meet or one reaches a settlement's middle.
    Join,
}

struct Edge {
    a: u32,
    b: u32,
    path: Path,
    length: f32,
}

/// The trails as a graph: a node wherever one trail ends or another meets it, and at the
/// middle of each settlement.
struct Graph {
    pos: Vec<Vec2>,
    adj: Vec<Vec<(u32, u32)>>,
    edges: Vec<Edge>,
    town: Vec<u32>,
}

impl Graph {
    fn of(net: &Network) -> Self {
        const BIN: f32 = 32.0;
        let lines = &net.lines;
        let key = |p: Vec2| ((p.x / BIN).floor() as i32, (p.y / BIN).floor() as i32);
        let mut bins: HashMap<(i32, i32), Vec<(u32, u32)>> = HashMap::new();
        for (l, line) in lines.iter().enumerate() {
            for (k, p) in line.iter().enumerate() {
                bins.entry(key(*p)).or_default().push((l as u32, k as u32));
            }
        }
        // The nearest point of another trail to `p`, within JOIN.
        let nearest_other = |p: Vec2, not: usize| -> Option<(u32, u32, f32)> {
            let (bx, bz) = key(p);
            let mut best: Option<(u32, u32, f32)> = None;
            for dz in -1..=1 {
                for dx in -1..=1 {
                    for &(l, k) in bins.get(&(bx + dx, bz + dz)).into_iter().flatten() {
                        if l as usize == not {
                            continue;
                        }
                        let d = lines[l as usize][k as usize].distance(p);
                        if d < JOIN && best.is_none_or(|b| d < b.2 || (d == b.2 && (l, k) < (b.0, b.1))) {
                            best = Some((l, k, d));
                        }
                    }
                }
            }
            best
        };
        let mut split: Vec<Vec<u32>> = lines.iter().map(|l| if l.len() >= 2 { vec![0, l.len() as u32 - 1] } else { Vec::new() }).collect();
        let mut joins = Vec::new();
        for (l, line) in lines.iter().enumerate() {
            if line.len() < 2 {
                continue;
            }
            for k in [0, line.len() - 1] {
                if let Some((l2, k2, d)) = nearest_other(line[k], l) {
                    split[l2 as usize].push(k2);
                    joins.push(((l as u32, k as u32), (l2, k2), d));
                }
            }
        }
        for s in &mut split {
            s.sort_unstable();
            s.dedup();
        }
        let mut id: HashMap<(u32, u32), u32> = HashMap::new();
        let mut pos = Vec::new();
        for (l, ks) in split.iter().enumerate() {
            for &k in ks {
                id.insert((l as u32, k), pos.len() as u32);
                pos.push(lines[l][k as usize]);
            }
        }
        let mut edges = Vec::new();
        for (l, ks) in split.iter().enumerate() {
            for w in ks.windows(2) {
                let line = &lines[l];
                let length: f32 = (w[0]..w[1]).map(|k| line[k as usize].distance(line[k as usize + 1])).sum();
                edges.push(Edge { a: id[&(l as u32, w[0])], b: id[&(l as u32, w[1])], path: Path::Along { line: l as u32, from: w[0], to: w[1] }, length });
            }
        }
        for (a, b, d) in joins {
            edges.push(Edge { a: id[&a], b: id[&b], path: Path::Join, length: d.max(0.1) });
        }
        // Each settlement's middle, joined to every trail that comes in to its green.
        let mut town = Vec::new();
        for s in &net.settlements {
            let t = pos.len() as u32;
            pos.push(s.at());
            town.push(t);
            let reach = s.kind.green() + JOIN;
            for (l, ks) in split.iter().enumerate() {
                for &k in ks {
                    let p = lines[l][k as usize];
                    let end = k == 0 || k as usize == lines[l].len() - 1;
                    if end && p.distance(s.at()) < reach {
                        edges.push(Edge { a: t, b: id[&(l as u32, k)], path: Path::Join, length: p.distance(s.at()).max(0.1) });
                    }
                }
            }
        }
        let mut adj = vec![Vec::new(); pos.len()];
        for (e, edge) in edges.iter().enumerate() {
            adj[edge.a as usize].push((edge.b, e as u32));
            adj[edge.b as usize].push((edge.a, e as u32));
        }
        Graph { pos, adj, edges, town }
    }

    /// The shortest way along the trails from node `from` to every other: the distance
    /// to each and the edge it is reached by.
    fn reach(&self, from: u32) -> (Vec<f32>, Vec<u32>) {
        let mut dist = vec![f32::INFINITY; self.pos.len()];
        let mut by = vec![u32::MAX; self.pos.len()];
        let mut heap = BinaryHeap::new();
        dist[from as usize] = 0.0;
        heap.push((Reverse(0.0f32.to_bits()), from));
        while let Some((Reverse(d), n)) = heap.pop() {
            let d = f32::from_bits(d);
            if d > dist[n as usize] {
                continue;
            }
            for &(to, e) in &self.adj[n as usize] {
                let nd = d + self.edges[e as usize].length;
                if nd < dist[to as usize] {
                    dist[to as usize] = nd;
                    by[to as usize] = e;
                    heap.push((Reverse(nd.to_bits()), to));
                }
            }
        }
        (dist, by)
    }

    /// The line of the way to node `to` found by [`Self::reach`] from `from`, with the
    /// half-width of the trail at each point.
    fn way(&self, net: &Network, by: &[u32], from: u32, to: u32) -> Way {
        // The edges from the far end back to the start, then each turned the right way.
        let mut steps = Vec::new();
        let mut n = to;
        while n != from {
            let e = by[n as usize];
            if e == u32::MAX {
                break;
            }
            let edge = &self.edges[e as usize];
            let back = if edge.b == n { edge.a } else { edge.b };
            steps.push((e, back, n));
            n = back;
        }
        steps.reverse();
        let mut line: Vec<Vec2> = vec![self.pos[from as usize]];
        let mut widths: Vec<f32> = vec![m(0.6)];
        for (e, a, b) in steps {
            let edge = &self.edges[e as usize];
            match edge.path {
                Path::Along { line: l, from: k0, to: k1 } => {
                    let pts = &net.lines[l as usize];
                    let ws = &net.line_widths[l as usize];
                    let forward = edge.a == a;
                    let ks: Vec<u32> = if forward { (k0..=k1).collect() } else { (k0..=k1).rev().collect() };
                    for k in ks {
                        line.push(pts[k as usize]);
                        widths.push(ws.get(k as usize).copied().unwrap_or(m(0.6)));
                    }
                }
                Path::Join => {
                    line.push(self.pos[b as usize]);
                    widths.push(*widths.last().unwrap_or(&m(0.6)));
                }
            }
        }
        // No two points on top of each other.
        let mut keep_line = vec![line[0]];
        let mut keep_widths = vec![widths[0]];
        for (p, w) in line.into_iter().zip(widths).skip(1) {
            if p.distance(*keep_line.last().expect("a point")) > 0.5 {
                keep_line.push(p);
                keep_widths.push(w);
            }
        }
        Way::new(keep_line, keep_widths)
    }
}

impl Travel {
    /// What the travellers of continent `c` go by: its roads and long stretches, from its
    /// trails. `world_seed` is the world's, for reading the ground the way it is grown.
    pub fn of(c: &Continent, world_seed: u32) -> Travel {
        let net = &c.trails;
        let graph = Graph::of(net);
        let n = net.settlements.len();
        let longest = DAY_WALK * LONGEST_DAYS as f32;
        let mut roads = Vec::new();
        for a in 0..n {
            let (dist, by) = graph.reach(graph.town[a]);
            for b in a + 1..n {
                let d = dist[graph.town[b] as usize];
                if d.is_finite() && (SHORTEST_ROAD..=longest).contains(&d) {
                    let way = graph.way(net, &by, graph.town[a], graph.town[b]);
                    roads.push(Road { a, b, way });
                }
            }
        }
        // The long stretches between nodes, well away from every settlement.
        let mut stretches = Vec::new();
        for edge in &graph.edges {
            let Path::Along { line, from, to } = edge.path else { continue };
            if edge.length < STRETCH {
                continue;
            }
            let pts: Vec<Vec2> = net.lines[line as usize][from as usize..=to as usize].to_vec();
            let ws: Vec<f32> = (from..=to).map(|k| net.line_widths[line as usize].get(k as usize).copied().unwrap_or(m(0.6))).collect();
            let mid = pts[pts.len() / 2];
            if net.settlements.iter().any(|s| s.at().distance(mid) < s.radius + AWAY) {
                continue;
            }
            stretches.push(Way::new(pts, ws));
        }
        Travel {
            seed: c.desc.seed,
            world_seed,
            roads,
            stretches,
            parties: (n / 3).clamp(2, 10),
            regions: net.settlements.iter().map(|s| s.region).collect(),
            towns: net.settlements.iter().map(|s| (s.at(), s.radius, s.y)).collect(),
            smithies: (0..n).map(|k| net.plots.iter().any(|p| p.settlement == k && p.purpose == Purpose::Smithy)).collect(),
        }
    }

    /// Every party out on `day` whose day's doings come within `reach` of `at` (all of
    /// them for an infinite reach), as plans for the day, in a fixed order.
    pub fn near(&self, c: &Continent, day: i64, at: Vec2, reach: f32) -> Vec<Plan> {
        let mut out = Vec::new();
        let within = |p: Vec2, r: f32| p.distance(at) < reach + r;
        // Lone campers, by the stretches they stop at this week.
        let week = day.div_euclid(WEEK);
        for k in 0..self.stretches.len() {
            if let Some((s, side)) = self.camper_spot(week, k) {
                let (p, _, _) = self.stretches[k].at(s);
                if within(p, m(60.0)) {
                    out.extend(self.camper(c, week, k, s, side).map(Plan::Camper));
                }
            }
        }
        // Settlers, by where the day's walk takes them.
        let spell = day.div_euclid(SPELL);
        for k in 0..self.parties {
            let Some(j) = self.journey(spell, k) else { continue };
            let today = day - j.start;
            if !(0..j.days).contains(&today) {
                continue;
            }
            let road = &self.roads[j.road];
            let (s0, s1) = (DAY_WALK * today as f32 - m(260.0), DAY_WALK * (today + 1) as f32);
            let mut near = false;
            let mut s = s0.max(0.0);
            while s <= s1.min(road.way.length()) + 1.0 {
                if within(road.way.at(if j.back { road.way.length() - s } else { s }).0, m(40.0)) {
                    near = true;
                    break;
                }
                s += m(40.0);
            }
            if near {
                out.push(Plan::Settlers(self.settlers(c, &j, today)));
            }
        }
        // The river boats.
        let ways = &c.trails.waterways;
        for (r, run) in ways.runs.iter().enumerate() {
            if run.line.iter().step_by(4).any(|p| within(vec2(p.x, p.z), m(60.0))) {
                for b in 0..boats_on(run.length(), self.seed, r) {
                    out.push(Plan::RiverBoat(self.river_boat(c, r, b, day)));
                }
            }
        }
        // The fishing boats.
        for (g, ground) in ways.grounds.iter().enumerate() {
            let quay = &ways.quays[ground.quay];
            if within(vec2(quay.berth.x, quay.berth.z), m(60.0)) || within(vec2(ground.at.x, ground.at.z), m(60.0)) {
                out.push(Plan::Fishing(self.fishing_boat(c, g, day)));
            }
        }
        out
    }

    /// Where along stretch `k` a camper stops in `week`, and on which side, if one does.
    fn camper_spot(&self, week: i64, k: usize) -> Option<(f32, f32)> {
        let h = |n: i64| roll(self.seed ^ 0x0ca3_9e55, week, k as i64 * 16 + n);
        if h(0) >= CAMPER_CHANCE {
            return None;
        }
        let len = self.stretches[k].length();
        Some((len * (0.15 + 0.7 * h(1)), if h(2) < 0.5 { 1.0 } else { -1.0 }))
    }

    /// The camper by stretch `k` in `week`, `s` along it on `side`.
    fn camper(&self, c: &Continent, week: i64, k: usize, s: f32, side: f32) -> Option<CamperPlan> {
        let way = &self.stretches[k];
        let seed = hash2(self.seed ^ 0x0ca3_9e56, week as i32, k as i32);
        let spot = self.camp_spot(c, way, (s - m(60.0), s + m(60.0)), side, m(1.6), &[])?;
        let sampler = Sampler::new(c, self.world_seed, spot.fire.x as i32 - 48, spot.fire.z as i32 - 48, 97);
        // Fishing if there is water a short walk off, sketching if not.
        let fire = vec2(spot.fire.x, spot.fire.z);
        let mut water: Option<(Vec2, Vec2)> = None;
        for a in 0..16 {
            let n = Vec2::from_angle(a as f32 / 16.0 * std::f32::consts::TAU);
            let mut last = fire;
            let mut d = m(2.0);
            while d < m(18.0) {
                let p = fire + n * d;
                if sampler.column(p.x.floor() as i32, p.y.floor() as i32).water_surface().is_some() {
                    if water.is_none_or(|w| w.0.distance(fire) > last.distance(fire)) {
                        water = Some((last, n));
                    }
                    break;
                }
                last = p;
                d += 1.0;
            }
        }
        let ground = |p: Vec2| (sampler.column(p.x.floor() as i32, p.y.floor() as i32).top() + 1) as f32;
        let (seat, facing, fishing) = match water {
            Some((bank, n)) => (Vec3::new(bank.x, ground(bank), bank.y), n.y.atan2(n.x), true),
            None => {
                // Sat a little way off from the fire, looking out away from the trail.
                let p = fire + spot.out * m(2.6) + spot.along * m(1.2);
                (Vec3::new(p.x, ground(p), p.y), spot.out.y.atan2(spot.out.x), false)
            }
        };
        let bed = fire + spot.out * m(1.4) - spot.along * m(1.0);
        Some(CamperPlan {
            id: (1u64 << 60) | ((week as u64 & 0xffff_ffff) << 20) | k as u64,
            region: self.region_near(fire),
            seed,
            spot,
            bed: Vec3::new(bed.x, ground(bed), bed.y),
            tent: unit(seed.rotate_left(7)) < 0.45,
            seat,
            facing,
            fishing,
        })
    }

    /// The region of the settlement nearest `p`, whose people a traveller there would be.
    fn region_near(&self, p: Vec2) -> Region {
        self.towns
            .iter()
            .zip(&self.regions)
            .min_by(|a, b| a.0.0.distance_squared(p).total_cmp(&b.0.0.distance_squared(p)))
            .map_or(Region::Temperate, |(_, r)| *r)
    }

    /// A spot to camp beside way `way` between `range` along it (looked for from its far
    /// end back), on `side` first: dry, level enough and off the trail, its middle `off`
    /// beyond the trail's edge, out of the trees where there is a clearing to be had. None
    /// only where there is nowhere at all. `ends` are where beasts are tethered, so far
    /// along the row of carts from its middle either way (see [`SettlersPlan::tethers`]):
    /// their ground is looked at too.
    fn camp_spot(&self, c: &Continent, way: &Way, range: (f32, f32), side: f32, off: f32, ends: &[f32]) -> Option<Spot> {
        let (lo, hi) = (range.0.max(0.0), range.1.min(way.length()));
        if hi < lo {
            return None;
        }
        // One sampler for the whole stretch searched.
        let (mut a, mut b) = (Vec2::MAX, Vec2::MIN);
        let mut s = lo;
        while s <= hi + m(16.0) {
            let p = way.at(s.min(hi)).0;
            (a, b) = (a.min(p), b.max(p));
            s += m(16.0);
        }
        let pad = off + m(14.0);
        let (x0, z0) = ((a.x - pad).floor() as i32, (a.y - pad).floor() as i32);
        let span = ((b - a).max_element() + 2.0 * pad).ceil() as i32 + 2;
        let sampler = Sampler::new(c, self.world_seed, x0, z0, span);
        let col = |p: Vec2| sampler.column(p.x.floor() as i32, p.y.floor() as i32);
        let wx = |p: Vec2| (c.desc.mx as f64 * MACRO_SIZE as f64 + p.x as f64, c.desc.mz as f64 * MACRO_SIZE as f64 + p.y as f64);
        // In a clearing and on level ground first; then anywhere level; then anywhere
        // they could lay a bedroll.
        for (open, rough) in [(true, 1), (false, 1), (false, 2)] {
            let mut s = hi;
            while s >= lo {
                for side in [side, -side] {
                    let (p, dir, hw) = way.at(s);
                    let out = vec2(-dir.y, dir.x) * side;
                    let centre = p + out * (hw + off);
                    if self.towns.iter().any(|t| t.0.distance(centre) < t.1 + m(30.0)) {
                        continue;
                    }
                    // The fire's ground, and the ground the tents, carts and bedrolls
                    // stand on about it.
                    let mut probe = vec![centre, centre + dir * m(2.5), centre - dir * m(2.5), centre + out * m(3.4), centre - out * (off - m(0.8)).max(m(0.6))];
                    probe.extend(ends.iter().map(|&e| centre - out * CART_ROW + dir * e));
                    let cols: Vec<Column> = probe.iter().map(|&q| col(q)).collect();
                    let dry = cols.iter().all(|k| k.water_surface().is_none() && k.river > m(1.0) && matches!(k.cover, Cover::Land | Cover::Beach | Cover::Bank));
                    // The fire well off any trail, not just this one, and nothing on one.
                    let off_trail = cols[0].clear > off - m(0.8) && cols.iter().all(|k| k.clear > m(0.4));
                    let (top, bottom) = cols.iter().fold((i32::MIN, i32::MAX), |(t, b), k| (t.max(k.top()), b.min(k.top())));
                    if !(dry && off_trail && top - bottom <= rough) {
                        continue;
                    }
                    let (x, z) = wx(centre);
                    if open && wooded(self.world_seed, x, z) > 0.15 {
                        continue;
                    }
                    let y = |k: usize| (cols[k].top() + 1) as f32;
                    let tethers = [0, 1].map(|k| if k < ends.len() { y(5 + k) } else { f32::NAN });
                    return Some(Spot { fire: Vec3::new(centre.x, y(0), centre.y), out, along: dir, by: s, beside: p, beyond: y(3), near: y(4), tethers });
                }
                s -= m(12.0);
            }
        }
        None
    }

    /// The journey party `k` makes in `spell`, if it makes one.
    fn journey(&self, spell: i64, k: usize) -> Option<Journey> {
        if self.roads.is_empty() {
            return None;
        }
        let h = |n: i64| hash2(self.seed ^ 0x5e77_1e55, spell as i32, k as i32 * 16 + n as i32);
        // Not every party is on the road every spell.
        if unit(h(0)) < 0.25 {
            return None;
        }
        let road = h(1) as usize % self.roads.len();
        let days = (self.roads[road].way.length() / DAY_WALK).ceil().max(1.0) as i64;
        if days > LONGEST_DAYS {
            return None;
        }
        let start = spell * SPELL + (h(2) % (SPELL - days) as u32) as i64;
        Some(Journey { spell, party: k, road, back: h(3) & 1 == 1, start, days, seed: h(4) })
    }

    /// The plan for day `today` (counting from 0) of journey `j`.
    fn settlers(&self, c: &Continent, j: &Journey, today: i64) -> SettlersPlan {
        let road = &self.roads[j.road];
        let len = road.way.length();
        // The road the way they walk it.
        let way = if j.back {
            let mut line = road.way.line.clone();
            let mut widths = road.way.widths.clone();
            line.reverse();
            widths.reverse();
            Way::new(line, widths)
        } else {
            road.way.clone()
        };
        let (from, to) = if j.back { (road.b, road.a) } else { (road.a, road.b) };
        let mut members = Vec::new();
        let moving = unit(j.seed.rotate_left(3)) < 0.55;
        let size = 5 + (unit(j.seed.rotate_left(9)) * 3.999) as u32;
        for n in 0..size {
            let h = hash2(j.seed, n as i32, 0x9e0);
            let who = match n {
                0 | 1 => Who::Adult,
                _ if moving && unit(h) < 0.4 => Who::Child,
                _ if unit(h.rotate_left(5)) < 0.15 => Who::Elder,
                _ => Who::Adult,
            };
            members.push((h, who, if moving { Work::Settler } else { Work::Merchant }));
        }
        let carts = 1 + (unit(j.seed.rotate_left(13)) < 0.5) as usize;
        // Their carts drawn by the beasts their own country keeps for it, where it keeps any.
        let kept = c.trails.settlements.get(from).map(fauna::kept_by).unwrap_or_default();
        let beast = [fauna::Kind::Ox, fauna::Kind::Donkey].into_iter().find(|k| kept.contains(k));
        let ends = tethers(carts, beast);
        let camp = |day: i64| -> Option<Spot> {
            if day < 0 || day >= j.days - 1 {
                return None;
            }
            let target = (DAY_WALK * (day + 1) as f32).min(len);
            let side = if hash2(j.seed, day as i32, 5) & 1 == 0 { 1.0 } else { -1.0 };
            // Where the day's walk ends if there is a spot; if not, they stop earlier.
            self.camp_spot(c, &way, (target - m(240.0), target), side, m(5.0), &ends).or_else(|| self.camp_spot(c, &way, (target - m(700.0), target - m(240.0)), side, m(5.0), &ends))
        };
        let morning = camp(today - 1);
        let evening = camp(today);
        let s0 = morning.as_ref().map_or(0.0, |c| c.by);
        let s1 = evening.as_ref().map_or(if today == j.days - 1 { len } else { (DAY_WALK * (today + 1) as f32).min(len) }, |c| c.by);
        // The stretch walked today, with the height of the trail underfoot.
        let mut pts = Vec::new();
        let mut s = s0;
        loop {
            let (p, _, _) = way.at(s);
            pts.push(Vec3::new(p.x, feet_on_trail(&c.trails, p).unwrap_or(f32::NAN), p.y));
            if s >= s1 {
                break;
            }
            s = (s + m(4.0)).min(s1);
        }
        fill_heights(&mut pts, self.towns[from].2 as f32);
        let along = measure(&pts);
        SettlersPlan {
            id: (2u64 << 60) | ((j.spell as u64 & 0xffff_ffff) << 8) | j.party as u64,
            region: self.regions[from],
            from,
            to,
            seed: j.seed,
            members,
            carts,
            beast,
            morning,
            evening,
            pts,
            along,
            moving,
        }
    }

    /// Where party `id` of settlers is at `hour` of `day`, roughly (on the road, as they
    /// walk it, without looking where they camp), and whether they are still on their
    /// way: for marking them on the map from afar. None once their journey is over, or
    /// for an id that is not a party of settlers.
    pub fn settlers_where(&self, id: u64, day: i64, hour: f32) -> Option<Vec2> {
        if id >> 60 != 2 {
            return None;
        }
        let (spell, party) = (((id >> 8) & 0xffff_ffff) as u32 as i32 as i64, (id & 0xff) as usize);
        let j = self.journey(spell, party)?;
        let today = day - j.start;
        if !(0..j.days).contains(&today) || party >= self.parties {
            return None;
        }
        let road = &self.roads[j.road];
        let s = (DAY_WALK * (today as f32 + walked(hour))).min(road.way.length());
        Some(road.way.at(if j.back { road.way.length() - s } else { s }).0)
    }

    /// The fishing boat that works ground `g`, on `day`.
    fn fishing_boat(&self, c: &Continent, g: usize, day: i64) -> FishingPlan {
        let ways = &c.trails.waterways;
        let ground = &ways.grounds[g];
        let quay = &ways.quays[ground.quay];
        let h = hash2(self.seed ^ 0x0f15_4b0a, g as i32, day as i32);
        // The boats of one quay each tie up at their own place: the first alongside the
        // jetty, the next on its other side, the third off its end.
        let k = ways.grounds[..g].iter().filter(|o| o.quay == ground.quay).count();
        let (out, across) = (Vec2::from_angle(quay.heading), Vec2::from_angle(quay.heading + std::f32::consts::FRAC_PI_2));
        let slot = match k {
            0 => Vec2::ZERO,
            1 => -across * m(3.0),
            _ => out * m(4.4) - across * m(1.5),
        };
        let berth = quay.berth + Vec3::new(slot.x, 0.0, slot.y);
        FishingPlan {
            id: (4u64 << 60) | g as u64,
            region: self.regions[quay.settlement],
            home: quay.settlement,
            seed: hash2(self.seed ^ 0x0f15_4b0b, g as i32, 0),
            berth: (berth, quay.heading),
            ground: ground.at,
            // Each boat its own hours, a little, from day to day.
            late: unit(h) * 0.6,
            home_by: 11.8 + unit(h.rotate_left(8)) * 0.9,
            sail: unit(hash2(self.seed, g as i32, 0x5a11)) < 0.4,
        }
    }
}

/// How many trade boats work a run of `length`: a rowing boat, and on a long one a
/// sailing boat too.
fn boats_on(length: f32, seed: u32, r: usize) -> usize {
    if length > m(1200.0) || unit(hash2(seed ^ 0x0b0a_7e56, r as i32, 0)) < 0.35 { 2 } else { 1 }
}

/// How much of a day's walk the settlers have done by `hour`.
fn walked(hour: f32) -> f32 {
    let walking = if hour < SET_OFF {
        0.0
    } else if hour < HALT {
        hour - SET_OFF
    } else if hour < ON {
        HALT - SET_OFF
    } else if hour < STOP {
        HALT - SET_OFF + hour - ON
    } else {
        WALK_HOURS
    };
    walking / WALK_HOURS
}

/// How thickly the trees grow at world `(x, z)` against how thick they could: the slow
/// openness that thins the forests into clearings, as the trees are grown
/// (`features::choose`). Under 0 nothing but the giants grows there.
fn wooded(world_seed: u32, x: f64, z: f64) -> f32 {
    let openness = fbm2(world_seed ^ 0xc1ea, x / m64(76.0), z / m64(76.0), 3, 2.0, 0.5);
    ((openness + 0.35) / 0.5).clamp(0.0, 1.0)
}

/// Where someone's feet are on the trail at `p`: on the top of the trail's surface.
fn feet_on_trail(net: &Network, p: Vec2) -> Option<f32> {
    let (pts, _) = net.gather(p, p);
    pts.iter()
        .map(|t| (vec2(t.x, t.z).distance_squared(p), t.y))
        .filter(|(d, _)| *d < m(3.0) * m(3.0))
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, y)| y.floor() + 1.0)
}

/// Fills in the heights a line has none for, from the nearest that it has, or `or`.
fn fill_heights(pts: &mut [Vec3], or: f32) {
    let mut last = pts.iter().find(|p| p.y.is_finite()).map_or(or, |p| p.y);
    for p in pts.iter_mut() {
        if p.y.is_finite() {
            last = p.y;
        } else {
            p.y = last;
        }
    }
}

/// A spot to camp: the fire's place on the ground, the way out from the trail to it and
/// the way along the trail, how far along the way it is and the trail point beside it,
/// and the height of the ground beyond the fire (where the tents go) and between it and
/// the trail (where the carts are drawn up).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    pub fire: Vec3,
    pub out: Vec2,
    pub along: Vec2,
    pub by: f32,
    pub beside: Vec2,
    pub beyond: f32,
    pub near: f32,
    /// The height of the ground where each beast is tethered, NaN where there is none.
    pub tethers: [f32; 2],
}

/// One party's journey in a spell.
struct Journey {
    spell: i64,
    party: usize,
    road: usize,
    /// Walked from the road's second settlement to its first.
    back: bool,
    start: i64,
    days: i64,
    seed: u32,
}

/// Where a boat is in its days.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Leg {
    /// This many whole days into a trip.
    Going(i64),
    Moored,
}

/// A party's plan for a day: everything about where they will be that needed the ground
/// looked at, so where they are at any hour of it is a quick sum.
#[derive(Clone, Debug)]
pub enum Plan {
    Camper(CamperPlan),
    Settlers(SettlersPlan),
    RiverBoat(RiverPlan),
    Fishing(FishingPlan),
}

#[derive(Clone, Debug)]
pub struct CamperPlan {
    pub id: u64,
    region: Region,
    seed: u32,
    spot: Spot,
    bed: Vec3,
    tent: bool,
    seat: Vec3,
    facing: f32,
    fishing: bool,
}

#[derive(Clone, Debug)]
pub struct SettlersPlan {
    pub id: u64,
    region: Region,
    from: usize,
    to: usize,
    seed: u32,
    members: Vec<(u32, Who, Work)>,
    carts: usize,
    /// What draws the carts, if anything does but the settlers themselves.
    beast: Option<fauna::Kind>,
    morning: Option<Spot>,
    evening: Option<Spot>,
    pts: Vec<Vec3>,
    along: Vec<f32>,
    moving: bool,
}

#[derive(Clone, Debug)]
pub struct RiverPlan {
    pub id: u64,
    region: Region,
    from: usize,
    to: usize,
    seed: u32,
    sail: bool,
    /// How fast it goes on the water, in voxels an hour.
    speed: f32,
    leg: Leg,
    down: bool,
    line: Vec<Vec3>,
    along: Vec<f32>,
    /// The first quay's berth and heading, and the second's.
    berths: (Vec3, f32, Vec3, f32),
    /// What it carries on this voyage, by a seed: the same every day of the voyage and
    /// the day it lies at the quay it came to. Whether that is iron for the smithy there,
    /// and where on that quay the iron is stacked once it is landed (#205).
    cargo: u32,
    iron: bool,
    stack: Option<(Vec3, f32)>,
    portages: Vec<Portage>,
    /// The voyage it is on, the way it goes.
    voyage: river::Voyage,
    /// Where it ties up off the bank for the night before the day planned for and the
    /// night after, when it does.
    nights: [Option<river::Mooring>; 2],
    /// Where it could tie up along the day's water if the water turns rough, each with the
    /// water leg it is on.
    refuges: Vec<(usize, river::Mooring)>,
    /// The crew's camps by each portage's stages, above and below, for a night there.
    stages: Vec<[Option<river::Camp>; 2]>,
}

#[derive(Clone, Debug)]
pub struct FishingPlan {
    pub id: u64,
    region: Region,
    home: usize,
    seed: u32,
    berth: (Vec3, f32),
    ground: Vec3,
    /// How long after dawn they put out, and the hour they are back at the quay.
    late: f32,
    home_by: f32,
    sail: bool,
}

impl Plan {
    pub fn id(&self) -> u64 {
        match self {
            Plan::Camper(p) => p.id,
            Plan::Settlers(p) => p.id,
            Plan::RiverBoat(p) => p.id,
            Plan::Fishing(p) => p.id,
        }
    }

    pub fn kind(&self) -> Kind {
        match self {
            Plan::Camper(_) => Kind::Camper,
            Plan::Settlers(_) => Kind::Settlers,
            Plan::RiverBoat(_) => Kind::RiverBoat,
            Plan::Fishing(_) => Kind::FishingBoat,
        }
    }

    /// How many settlers are on the road together, those asleep in the tents too; None
    /// for the others, who are all in sight.
    pub fn party(&self) -> Option<usize> {
        match self {
            Plan::Settlers(p) => Some(p.members.len()),
            _ => None,
        }
    }

    /// Where the party is and what they are doing at `hour` of the day planned for, on
    /// `date` (for the dawn), the boats keeping in or making for shelter as the `water`
    /// is. None while they are not out at all: settlers before they set off and once they
    /// are in.
    pub fn at(&self, hour: f32, date: f32, water: Water) -> Option<Scene> {
        match self {
            Plan::Camper(p) => Some(p.at(hour)),
            Plan::Settlers(p) => p.at(hour),
            Plan::RiverBoat(p) => Some(p.at(hour, water)),
            Plan::Fishing(p) => Some(p.at(hour, date, water)),
        }
    }
}

/// Facing from `a` toward `b`, as a camera's yaw.
fn toward(a: Vec2, b: Vec2) -> f32 {
    let d = b - a;
    d.y.atan2(d.x)
}

fn flat(p: Vec3) -> Vec2 {
    vec2(p.x, p.z)
}

impl CamperPlan {
    fn at(&self, hour: f32) -> Scene {
        let mut scene = Scene::new(self.id, Kind::Camper, self.region);
        let fire = self.spot.fire;
        // Asleep at night, and, out under the sky, for an hour after the midday meal.
        let asleep = !(6.5..21.5).contains(&hour) || (!self.tent && (13.0..14.0).contains(&hour));
        let lit = (6.0..9.0).contains(&hour) || (18.0..23.0).contains(&hour);
        scene.fire = Some((fire, if lit { Fire::Lit } else { Fire::Low }));
        let bed_turn = self.spot.along.y.atan2(self.spot.along.x);
        if self.tent {
            let t = flat(self.bed) + self.spot.out * m(0.8);
            scene.tents.push((Vec3::new(t.x, self.bed.y, t.y), bed_turn));
        } else {
            scene.bedrolls.push((self.bed, bed_turn));
        }
        let by_fire = flat(fire) - self.spot.along * m(1.1);
        let by_fire = Vec3::new(by_fire.x, fire.y, by_fire.y);
        let (feet, facing, doing) = if asleep {
            // Lying along the bedroll, feet at its far end.
            let feet = flat(self.bed) + Vec2::from_angle(bed_turn) * BEDROLL_BACK;
            (Vec3::new(feet.x, self.bed.y, feet.y), bed_turn, Doing::Asleep)
        } else if (9.0..12.0).contains(&hour) || (14.0..18.0).contains(&hour) {
            (self.seat, self.facing, if self.fishing { Doing::Fishing } else { Doing::Sketching })
        } else {
            (by_fire, toward(flat(by_fire), flat(fire)), Doing::Sitting)
        };
        // Asleep in a tent, they are not seen.
        if !(asleep && self.tent) {
            scene.people.push(Figure { seed: self.seed, who: Who::Adult, work: Work::Camper, feet, facing, doing });
        }
        scene.doing = doing.word();
        scene.at = fire;
        scene
    }
}

/// Room each walker takes along the road; the room a handcart takes behind its puller,
/// and how far behind them its middle is.
const WALKER: f32 = m(1.5);
const CART: f32 = m(2.8);
const CART_BEHIND: f32 = m(2.0);
/// Half a cart's length, its bed alone; how far its shafts reach out ahead of its middle.
const CART_HALF: f32 = m(0.75);
const SHAFTS: f32 = m(1.95);
/// How far ahead of a beast's body its leader walks, at its head, and how far out to the
/// side; how far from the fire toward the road the row of carts stands in camp.
const AT_HEAD: f32 = m(0.6);
const BESIDE: f32 = m(0.75);
const CART_ROW: f32 = m(2.8);

/// About how long a beast's body is, nose and tail aside, for the room it takes in the
/// file and in camp: the model drawn is its own size, near enough this.
pub fn beast_length(kind: fauna::Kind) -> f32 {
    match kind {
        fauna::Kind::Ox | fauna::Kind::Cattle => m(1.85),
        fauna::Kind::Donkey => m(1.15),
        _ => m(1.3),
    }
}

/// Where the beasts of a party with `carts` carts drawn by `beast` are tethered in camp: so
/// far along the row of carts from its middle, one beyond each end of it. None without a
/// beast.
fn tethers(carts: usize, beast: Option<fauna::Kind>) -> Vec<f32> {
    let Some(kind) = beast else { return Vec::new() };
    let half = beast_length(kind) * 0.5;
    let first = -m(1.5) * (carts - 1) as f32;
    let last = first + m(3.0) * (carts - 1) as f32;
    let mut out = vec![first - CART_HALF - m(0.5) - half];
    if carts > 1 {
        out.push(last + SHAFTS + m(0.5) + half);
    }
    out
}
/// How far a sleeper's bedroll reaches back from their feet to its middle.
pub const BEDROLL_BACK: f32 = m(0.85);

impl SettlersPlan {
    /// The order they walk in, from the front: each one's place behind the leader, the
    /// cart each puller pulls or leader leads (by who it is, its middle's place and its
    /// beast's), and the length of the whole file. The first walks at the front with a
    /// cart; with two, the last grown-up has the other near the back.
    fn file(&self) -> (Vec<f32>, Vec<(usize, f32, Option<f32>)>, f32) {
        let n = self.members.len();
        let second = (2..n).rev().find(|&k| self.members[k].1 == Who::Adult).unwrap_or(1);
        let pullers: &[usize] = if self.carts > 1 { &[0, second] } else { &[0] };
        let mut places = Vec::with_capacity(n);
        let mut carts = Vec::new();
        let mut back = 0.0;
        for k in 0..n {
            places.push(back);
            if pullers.contains(&k) {
                match self.beast {
                    // At its head, the beast behind them and its cart behind that, with
                    // the same room after it as after a handcart.
                    Some(kind) => {
                        let long = beast_length(kind);
                        let beast = back + AT_HEAD + long * 0.5;
                        let cart = beast + long * 0.5 + m(0.3) + CART_HALF;
                        carts.push((k, cart, Some(beast)));
                        back = cart + CART_HALF + (CART - CART_BEHIND - CART_HALF);
                    }
                    None => {
                        carts.push((k, back + CART_BEHIND, None));
                        back += CART;
                    }
                }
            }
            back += WALKER;
        }
        (places, carts, back)
    }

    /// Their camp about `spot`: the carts drawn up between the fire and the road and
    /// the tents beyond it, put in `scene`, and the place each sits about the fire,
    /// facing in.
    fn camp(&self, spot: &Spot, scene: &mut Scene) -> Vec<(Vec3, f32)> {
        let fire = flat(spot.fire);
        let at = |p: Vec2, y: f32| Vec3::new(p.x, y, p.y);
        let turn = spot.along.y.atan2(spot.along.x);
        for c in 0..self.carts {
            let p = fire - spot.out * CART_ROW + spot.along * (c as f32 * m(3.0) - m(1.5) * (self.carts - 1) as f32);
            scene.carts.push(Cart { at: at(p, spot.near), heading: turn, load: hash2(self.seed, c as i32, 0xca27), beast: None });
        }
        // The beasts tethered beyond the ends of the row of carts, facing out from it.
        if let Some(kind) = self.beast {
            for (c, &e) in tethers(self.carts, self.beast).iter().enumerate() {
                let p = fire - spot.out * CART_ROW + spot.along * e;
                let y = if spot.tethers[c].is_finite() { spot.tethers[c] } else { spot.near };
                let facing = turn + if e < 0.0 { std::f32::consts::PI } else { 0.0 };
                scene.beasts.push(Beast { kind, seed: hash2(self.seed, c as i32, 0xbea5), feet: at(p, y), facing, walking: false, grazing: false });
            }
        }
        let tents = 1 + (self.members.len() > 6) as usize;
        for t in 0..tents {
            let p = fire + spot.out * m(3.4) + spot.along * (t as f32 * m(3.0) - m(1.5) * (tents - 1) as f32);
            scene.tents.push((at(p, spot.beyond), turn));
        }
        let n = self.members.len();
        let turn0 = unit(self.seed) * std::f32::consts::TAU;
        (0..n)
            .map(|k| {
                let a = (k as f32 + 0.5) / n as f32 * std::f32::consts::TAU + turn0;
                let p = fire + Vec2::from_angle(a) * m(1.7 + 0.3 * ((k % 2) as f32));
                (at(p, spot.fire.y), toward(p, fire))
            })
            .collect()
    }

    fn at(&self, hour: f32) -> Option<Scene> {
        let mut scene = Scene::new(self.id, Kind::Settlers, self.region);
        scene.from = Some(self.from);
        scene.to = Some(self.to);
        let morning = hour < SET_OFF;
        let evening = hour >= STOP;
        // Not yet set out on the first morning; gone in once there on the last evening.
        let camp = if morning { self.morning.as_ref() } else if evening { self.evening.as_ref() } else { None };
        if (morning || evening) && camp.is_none() {
            return None;
        }
        let len = self.along.last().copied().unwrap_or(0.0);
        let (places, carts, file) = self.file();
        let walk = |s: f32| along_line(&self.pts, &self.along, s);
        // How far the back of the file is along today's stretch: the walking hours done
        // of the day's, the halt to eat not counted.
        let room = (len - file).max(0.0);
        let tail = room * walked(hour);
        let halted = (HALT..ON).contains(&hour);
        let on_road: Vec<(Vec3, f32)> = places
            .iter()
            .enumerate()
            .map(|(k, &back)| {
                let (p, dir) = walk(tail + file - back);
                // Not quite in single file: each a little to one side or the other.
                let side = Vec2::from_angle(dir + std::f32::consts::FRAC_PI_2) * if k % 2 == 0 { m(0.25) } else { -m(0.25) };
                (Vec3::new(p.x + side.x, p.y, p.z + side.y), dir)
            })
            .collect();
        // A beast in the shafts walks the road ahead of its cart, which is turned toward it.
        let road_beasts: Vec<Beast> = carts
            .iter()
            .enumerate()
            .filter_map(|(c, &(_, _, beast))| {
                let (p, dir) = walk(tail + file - beast?);
                Some(Beast { kind: self.beast?, seed: hash2(self.seed, c as i32, 0xbea5), feet: p, facing: dir, walking: !halted, grazing: false })
            })
            .collect();
        let road_carts: Vec<Cart> = carts
            .iter()
            .enumerate()
            .map(|(c, &(_, back, beast))| {
                let (p, dir) = walk(tail + file - back);
                let heading = match beast {
                    Some(_) => toward(flat(p), flat(road_beasts[c].feet)),
                    None => dir,
                };
                Cart { at: p, heading, load: hash2(self.seed, c as i32, 0xca27), beast: beast.map(|_| c) }
            })
            .collect();
        let Some(spot) = camp else {
            for (k, &(seed, who, work)) in self.members.iter().enumerate() {
                let (mut feet, facing) = on_road[k];
                let pulls = carts.iter().find(|c| c.0 == k);
                let leads = pulls.is_some_and(|c| c.2.is_some());
                let doing = if halted {
                    if k % 2 == 0 { Doing::Sitting } else { Doing::Standing }
                } else if leads {
                    Doing::Leading
                } else if pulls.is_some() {
                    Doing::Pulling
                } else {
                    Doing::Walking
                };
                if leads && !halted {
                    // Beside the beast's head, on its left, rather than in its way.
                    let (p, dir) = walk(tail + file - places[k]);
                    let side = Vec2::from_angle(dir + std::f32::consts::FRAC_PI_2) * BESIDE;
                    feet = Vec3::new(p.x + side.x, p.y, p.z + side.y);
                }
                scene.people.push(Figure { seed, who, work, feet, facing, doing });
            }
            scene.carts = road_carts;
            scene.beasts = road_beasts;
            scene.doing = if halted {
                "stopped by the road to eat"
            } else if self.moving {
                "on the road, moving house"
            } else {
                "on the road with goods to trade"
            };
            scene.at = on_road.first().map_or(Vec3::ZERO, |p| p.0);
            return Some(scene);
        };
        // In camp, or walking between it and the road: off it in the evening, back on
        // it in the morning.
        let ring = self.camp(spot, &mut scene);
        let blend = if morning { (SET_OFF - hour) / TURN_OFF } else { (hour - STOP) / TURN_OFF }.clamp(0.0, 1.0);
        if blend < 1.0 {
            for (c, cart) in scene.carts.iter_mut().enumerate() {
                let road = road_carts[c];
                cart.at = road.at.lerp(cart.at, blend);
                cart.heading = road.heading;
            }
            // Led off the road to where they are tethered, or back to the shafts.
            for (c, beast) in scene.beasts.iter_mut().enumerate() {
                let Some(road) = road_beasts.get(c) else { continue };
                let to = beast.feet;
                beast.feet = road.feet.lerp(to, blend);
                let way = if morning { flat(road.feet) - flat(to) } else { flat(to) - flat(road.feet) };
                beast.facing = if way.length() > 0.5 { way.y.atan2(way.x) } else { road.facing };
                beast.walking = true;
            }
        } else {
            // Cropping the grass about them in the evening light; still by night.
            for beast in &mut scene.beasts {
                beast.grazing = (STOP..21.0).contains(&hour) || (5.5..SET_OFF).contains(&hour);
            }
        }
        scene.fire = Some((spot.fire, if (5.5..SET_OFF).contains(&hour) || (STOP..22.0).contains(&hour) { Fire::Lit } else { Fire::Low }));
        let asleep = !(5.5..21.5).contains(&hour);
        for (k, &(seed, who, work)) in self.members.iter().enumerate() {
            let (sit, facing) = ring[k];
            let (road, dir) = on_road[k];
            if blend < 1.0 {
                let feet = road.lerp(sit, blend);
                // Facing the way they are going: off the road toward the fire in the
                // evening, and back to the road in the morning.
                let way = if morning { flat(road) - flat(sit) } else { flat(sit) - flat(road) };
                let facing = if way.length() > 0.5 { way.y.atan2(way.x) } else { dir };
                scene.people.push(Figure { seed, who, work, feet, facing, doing: Doing::Walking });
                continue;
            }
            let doing = if asleep {
                Doing::Asleep
            } else if k % 3 == 1 {
                Doing::Standing
            } else {
                Doing::Sitting
            };
            if doing == Doing::Asleep {
                // The children and every third grown-up sleep in the tents, out of sight;
                // the rest under the sky, on bedrolls, their feet to the fire.
                if who == Who::Child || k % 3 == 2 {
                    continue;
                }
                let bed = flat(sit) - Vec2::from_angle(facing) * BEDROLL_BACK;
                scene.bedrolls.push((Vec3::new(bed.x, sit.y, bed.y), facing));
            }
            scene.people.push(Figure { seed, who, work, feet: sit, facing, doing });
        }
        scene.doing = if blend < 1.0 {
            if morning { "breaking camp" } else { "making camp" }
        } else if asleep {
            "asleep in camp"
        } else {
            "in camp, about the fire"
        };
        scene.at = spot.fire;
        Some(scene)
    }
}

impl FishingPlan {
    fn at(&self, hour: f32, date: f32, water: Water) -> Scene {
        let mut scene = Scene::new(self.id, Kind::FishingBoat, self.region);
        scene.from = Some(self.home);
        let (berth, ground) = (self.berth.0, self.ground);
        let leave = dawn(date) + self.late;
        let reach = vec2(ground.x - berth.x, ground.z - berth.z).length();
        // Out by mid-morning, back about midday: as fast as that needs, under oar or sail.
        let speed = if self.sail { SAIL } else { ROW };
        let trip = (reach / speed).clamp(0.5, 1.6);
        let home = self.home_by;
        let rough = water.rough_since.filter(|&since| hour >= since);
        // Caught out by the weather: home at once from wherever they were, as fast as
        // they came out.
        if let Some(since) = rough
            && (leave..home).contains(&since)
        {
            let calm = self.at(since, date, Water::CALM);
            let was = calm.boats[0].at;
            let back = (flat(was).distance(flat(berth)) / (reach / trip).max(1.0)).max(0.05);
            let way = flat(berth) - flat(was);
            let (at, heading, moving, doing) = if hour < since + back {
                (was.lerp(berth, (hour - since) / back), way.y.atan2(way.x), true, "making for home as the weather turns")
            } else {
                (berth, self.berth.1, false, "in out of the weather")
            };
            return self.crewed(scene, at, heading, moving, doing);
        }
        let (arrive, haul) = (leave + trip, home - trip);
        let out_way = flat(ground) - flat(berth);
        let (at, heading, moving, doing) = if rough.is_some() || hour < leave || hour >= home {
            (berth, self.berth.1, false, if rough.is_some() && hour < home { "kept in by the weather" } else if hour < leave { "tied up, waiting for dawn" } else { "back in, the catch landed" })
        } else if hour < arrive {
            let t = (hour - leave) / trip;
            (berth.lerp(ground, t), out_way.y.atan2(out_way.x), true, "putting out to the fishing grounds")
        } else if hour < haul {
            // Drifting slowly out about the ground while the lines are out, and back to
            // where they came in by the time they make for home.
            let t = (hour - arrive) / (haul - arrive).max(0.1);
            let a = t * 2.5 + unit(self.seed) * 6.0;
            let p = flat(ground) + Vec2::from_angle(a) * DRIFT * (t * std::f32::consts::PI).sin();
            (Vec3::new(p.x, ground.y, p.y), a + std::f32::consts::FRAC_PI_2, false, "fishing")
        } else {
            let t = (hour - haul) / trip;
            (ground.lerp(berth, t), (-out_way.y).atan2(-out_way.x), true, "coming home with the catch")
        };
        self.crewed(scene, at, heading, moving, doing)
    }

    /// `scene` with the boat at `at`, pointing `heading`, and its crew aboard.
    fn crewed(&self, mut scene: Scene, at: Vec3, heading: f32, moving: bool, doing: &'static str) -> Scene {
        scene.boats.push(Boat { at, heading, sail: self.sail, load: self.seed, moving, ashore: false, laden: true, iron: false });
        let along = Vec2::from_angle(heading);
        for (k, off) in [(0, m(0.85)), (1, -m(0.8))] {
            let p = flat(at) + along * off;
            let what = if moving && k == 0 && !self.sail {
                Doing::Rowing
            } else if doing == "fishing" {
                Doing::Fishing
            } else {
                Doing::Sitting
            };
            scene.people.push(Figure { seed: hash2(self.seed, k, 0xf15e), who: Who::Adult, work: Work::Fisher, feet: Vec3::new(p.x, at.y, p.y), facing: if what == Doing::Rowing { heading + std::f32::consts::PI } else { heading }, doing: what });
        }
        scene.doing = doing;
        scene.at = at;
        scene
    }
}

mod river;
#[cfg(test)]
mod tests;
