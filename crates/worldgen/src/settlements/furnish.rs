//! Furnishing a home or a workplace: what stands inside it (stage 2, #202).
//!
//! The village layout builds each building from the house kit and then asks here what
//! goes in it; what is set outside, in yards and lanes, is the layout's own business.
//!
//! [`furnish`] lays a bed for everyone who sleeps there on the kit's places for beds (a
//! cradle for a baby, children two to a bed), the fire and its pot on the hearth, the
//! fittings of the trade worked there, a table with stools or a bench, shelves of pots and
//! a chest, and glass in the windows, lit after dark where someone is at home. Wealth adds
//! more things, neglect breaks them, and an empty home keeps only a few broken ones.
//!
//! Everything is checked as it is set: it stands on the floor, clear of the walls and of
//! everything set before it, out of the way in from the door and of the fireplace, and
//! someone coming in at the door can still walk up to every bed, the hearth and each thing
//! set. What cannot be set without blocking the way is left out.

use std::{
    collections::{HashSet, VecDeque},
    ops::Range,
};

use glam::{IVec2, IVec3, Vec2};
use voxel::{
    AIR, Voxel,
    mesh::{CELLS, PlantBox},
};

use super::{
    Livelihood, Region,
    kit::{Building, Hearth, Spot},
};
use crate::{
    features::{
        fittings::{self, Fitting, Hook, Stand},
        landmarks::Palette,
        parts,
        props::{self, Prop},
        shape::{Rng, rotate},
    },
    materials as mat,
    noise::{hash3, unit},
};

/// Who lives in a building and what is done there.
#[derive(Clone, Debug)]
pub struct Household {
    pub region: Region,
    /// How many sleep there, children among them.
    pub sleepers: u8,
    pub children: u8,
    /// The trade worked there, if it is a workplace as well as (or instead of) a home.
    pub trade: Option<Livelihood>,
    /// 0 poor to 1 well off: how much is in it, and how fine.
    pub wealth: f32,
    /// 0 well kept to 1 run down: how much of it is worn or broken.
    pub neglect: f32,
    /// Whether anyone lives there now. An empty home keeps a little of what was left.
    pub lived_in: bool,
}

/// What is set in a building, in the building's own model space as the kit built it
/// (before it is turned), ready to stamp with it.
#[derive(Clone, Debug, Default)]
pub struct Furnished {
    pub voxels: Vec<(IVec3, Voxel)>,
    pub fine: Vec<PlantBox>,
    /// Each thing set, in the order it was set.
    pub items: Vec<Item>,
    /// Where each sleeper lays their head, in cells (1/[`CELLS`] of a voxel): one for
    /// everyone who sleeps there, children sharing a bed among them. Empty in an empty
    /// home.
    pub sleepers: Vec<IVec3>,
    /// Where the fire burns, in cells, when someone is at home to keep it.
    pub fire: Option<IVec3>,
}

/// One thing set in a building.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub what: What,
    pub decayed: bool,
    /// The voxels it stands in, `lo` to `hi` exclusive.
    pub lo: IVec3,
    pub hi: IVec3,
    /// Its parts among [`Furnished::voxels`] and [`Furnished::fine`].
    pub voxels: Range<usize>,
    pub fine: Range<usize>,
}

impl Furnished {
    /// Where each sleeper lies, in model cells (as [`Self::sleepers`]): their head, and the
    /// way along the bed their feet lie from it, a step across the floor. People are put to
    /// bed here (#204).
    pub fn lying(&self) -> Vec<(IVec3, IVec2)> {
        let bed = |i: &&Item| matches!(i.what, What::Fitting(Fitting::StrawPallet | Fitting::BoxBed | Fitting::RopeBed | Fitting::Cradle));
        self.sleepers
            .iter()
            .map(|&head| {
                let v = head.div_euclid(IVec3::splat(CELLS));
                let under = self.items.iter().filter(bed).find(|i| v.x >= i.lo.x && v.x < i.hi.x && v.z >= i.lo.z && v.z < i.hi.z);
                // Along the bed's length, toward its far end from the pillow.
                let toward = under.map_or(IVec2::X, |b| {
                    let (size, mid) = (b.hi - b.lo, (b.lo + b.hi) * CELLS);
                    if size.x >= size.z {
                        IVec2::new(if mid.x >= 2 * head.x { 1 } else { -1 }, 0)
                    } else {
                        IVec2::new(0, if mid.z >= 2 * head.z { 1 } else { -1 })
                    }
                });
                (head, toward)
            })
            .collect()
    }

    /// Where someone can sit: each stool and bench, as the voxel its sitter's feet are in
    /// on the floor beside the table (model space), and the way they face, toward the
    /// table where there is one. A stool's seat is 5 cells up, a bench's about as high.
    pub fn seats(&self) -> Vec<(IVec3, IVec2)> {
        let table = self.items.iter().find(|i| i.what == What::Fitting(Fitting::Table)).map(|t| (t.lo + t.hi).as_vec3() * 0.5);
        let mut out = Vec::new();
        for i in &self.items {
            if i.decayed || !matches!(i.what, What::Fitting(Fitting::Stool) | What::Bench) {
                continue;
            }
            let mid = (i.lo + i.hi).as_vec3() * 0.5;
            let toward = table.map_or(IVec2::ZERO, |t| {
                let d = t - mid;
                if d.x.abs() >= d.z.abs() { IVec2::new(d.x.signum() as i32, 0) } else { IVec2::new(0, d.z.signum() as i32) }
            });
            // A bench seats two, one at either end of it.
            let long = if (i.hi - i.lo).x >= (i.hi - i.lo).z { IVec3::X } else { IVec3::Z };
            let n = if i.what == What::Bench { 2 } else { 1 };
            for k in 0..n {
                let at = if n == 1 { i.lo } else { i.lo + long * (k * ((i.hi - i.lo).dot(long) - 1)) };
                out.push((IVec3::new(at.x, i.lo.y, at.z), toward));
            }
        }
        out
    }
}

/// What an [`Item`] is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum What {
    Fitting(Fitting),
    /// A plank bench at the table, for a household without stools enough.
    Bench,
    /// The fire in a fireplace and the pot or kettle on it.
    Fireplace,
    /// The glass in a window, and whether a light is behind it.
    Pane { lit: bool },
}

/// The clear height a person needs over the floor, in voxels, as the kit keeps it: what
/// is set lower than this in a column stops anyone walking there.
const HEAD: i32 = 5;
/// How many places for one thing are tried against the way through the room before it is
/// given up; each costs a walk over the floor.
const TRIES: usize = 32;
/// Columns kept clear: the way in from the door, and the floor before the fire.
const WAY_IN: u8 = 1;
const BY_FIRE: u8 = 2;
const DIRS: [IVec2; 4] = [IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y];

/// Furnishes `b` for the household `h`, of the stone and timber of `pal`, varied by
/// `seed`.
pub fn furnish(b: &Building, h: &Household, pal: &Palette, seed: u32) -> Furnished {
    let Some(mut room) = Room::new(b, pal, seed, h.lived_in) else { return Furnished::default() };
    let mut rng = Rng::new(seed ^ 0x6675_726e);
    let lived = h.lived_in;
    let neglect = h.neglect.clamp(0.0, 1.0);
    let wealth = h.wealth.clamp(0.0, 1.0);
    // Worn or broken: everything in an empty home, and in a lived-in one as it is let go.
    let worn = |rng: &mut Rng, k: f32| !lived || rng.unit() < neglect * k;
    // A store: kept, but nobody sleeps or works in it and no fire burns there. It is
    // stacked with what the place keeps by, and its windows are dark at night though none
    // is broken.
    let store = lived && h.sleepers == 0 && h.trade.is_none();
    let lit = lived && !store;

    room.beds(h, &mut rng);
    match b.inside.hearth {
        Some(hearth) if hearth.central => room.hut_hearth(hearth, lit, &mut rng),
        Some(hearth) => room.fireplace(hearth, lit, &mut rng),
        None => {}
    }
    // What the trade is worked with, and what any home has; an empty home keeps only some
    // of it.
    let mut things: Vec<Fitting> = h.trade.map_or(&[][..], trade_fittings).iter().map(|f| f.local(h.region)).collect();
    if h.trade.is_some() && h.sleepers == 0 && wealth > 0.5 {
        // A workshop and nothing else has room for the trade's first fitting twice.
        things.push(things[0]);
    }
    for (k, &f) in things.iter().enumerate() {
        if lived || rng.unit() < 0.5 {
            let decayed = worn(&mut rng, 0.6);
            if f.stand() == Stand::Hung {
                room.hang(f, decayed, seed ^ k as u32);
            } else {
                room.put(f, decayed, seed ^ (k as u32) << 4);
            }
        }
    }
    if !store && (lived || rng.unit() < 0.5) {
        let decayed = worn(&mut rng, 0.6);
        room.table(decayed, wealth, &mut rng);
    }
    let mut home = vec![(Fitting::Shelf, 0.6), (Fitting::Chest, 0.4)];
    if store {
        use Fitting::*;
        home = [Chest, BasketStack, Shelf, Chest, WithyBundles, BasketStack, WaterJar, Shelf].map(|f| (f, 0.0)).to_vec();
    }
    if wealth > 0.3 {
        home.push((Fitting::BasketStack, 0.0));
    }
    if wealth > 0.55 {
        home.push((Fitting::Shelf, 0.0));
    }
    if wealth > 0.7 {
        home.push((Fitting::Chest, 0.0));
    }
    for (k, &(f, empty)) in home.iter().enumerate() {
        if lived || rng.unit() < empty {
            let decayed = worn(&mut rng, 0.6);
            room.put(f.local(h.region), decayed, seed ^ 0x51e1 ^ (k as u32) << 8);
        }
    }
    // Herbs drying from the rafters: the forager's living, and now and then anyone's.
    if lit {
        let bunches = if h.trade == Some(Livelihood::Forager) { 2 } else { (rng.unit() < 0.3) as u32 };
        for k in 0..bunches {
            room.hang(Fitting::HerbBunches, false, seed ^ 0xb0c4 ^ k);
        }
    }
    room.panes(lived, lit, neglect, &mut rng);
    room.out
}

/// What a trade is worked with indoors, in the order it is set.
fn trade_fittings(t: Livelihood) -> &'static [Fitting] {
    use Fitting::*;
    match t {
        Livelihood::Woodcutter => &[Workbench, Sawhorse],
        Livelihood::Sawmill | Livelihood::LoggingCamp => &[Workbench, Grindstone],
        Livelihood::Weaver => &[Loom, WithyBundles, BasketStack],
        Livelihood::Fletcher => &[Workbench, WithyBundles],
        Livelihood::Fisher => &[NetRack, EelTrap, BasketStack],
        Livelihood::Port => &[NetRack, LobsterPot, BasketStack],
        Livelihood::Hunter => &[HideFrame, AntlerRack],
        Livelihood::Apiary => &[Shelf, BasketStack],
        Livelihood::Quarry => &[Grindstone, Workbench],
        Livelihood::Forager => &[HerbRack, HerbBunches],
        Livelihood::Smithy => &[Forge, Anvil, Grindstone],
    }
}

/// The turns that face a fitting (its front toward +Z as built) toward `n`.
fn turns_to(n: IVec3) -> u32 {
    (0..4).find(|&t| rotate(IVec3::Z, t) == n).unwrap_or(0)
}

/// Every voxel the parts fill or draw into.
fn cells_of(voxels: &[(IVec3, Voxel)], fine: &[PlantBox]) -> Vec<IVec3> {
    let mut out: Vec<IVec3> = voxels.iter().map(|&(q, _)| q).collect();
    for b in fine {
        let (lo, hi) = (IVec3::from_array(b.lo).div_euclid(IVec3::splat(CELLS)), (IVec3::from_array(b.hi) - 1).div_euclid(IVec3::splat(CELLS)));
        for y in lo.y..=hi.y {
            for z in lo.z..=hi.z {
                for x in lo.x..=hi.x {
                    out.push(IVec3::new(x, y, z));
                }
            }
        }
    }
    out.sort_unstable_by_key(|q| (q.y, q.x, q.z));
    out.dedup();
    out
}

fn fixed(lo: (i32, i32, i32), hi: (i32, i32, i32), voxel: Voxel) -> PlantBox {
    PlantBox { lo: [lo.0, lo.1, lo.2], hi: [hi.0, hi.1, hi.2], voxel, sway_lo: false, sway_hi: false, thin: 0, season: 0, flap: 0 }
}

/// Fine parts turned by `turns` about the cell `m` and moved so that `m` lands on `to`.
fn turn_about(parts: &[PlantBox], m: IVec3, turns: u32, to: IVec3) -> Vec<PlantBox> {
    parts
        .iter()
        .map(|b| {
            let (a, c) = (rotate(IVec3::from_array(b.lo) - m, turns), rotate(IVec3::from_array(b.hi) - m, turns));
            PlantBox { lo: (a.min(c) + to).to_array(), hi: (a.max(c) + to).to_array(), ..*b }
        })
        .collect()
}

/// Something built and moved to where it stands, ready to set.
struct Parts {
    voxels: Vec<(IVec3, Voxel)>,
    fine: Vec<PlantBox>,
    hooks: Vec<Hook>,
}

/// A place a thing could stand: the turns that face it, its voxel 0 (as
/// [`fittings::Fitted::place`] takes it), the box it takes, the columns before it where
/// it is used from, and how good a place it is (higher first).
#[derive(Clone, Debug)]
struct Place {
    turns: u32,
    at: IVec3,
    lo: IVec3,
    hi: IVec3,
    front: Vec<IVec2>,
    score: f32,
}

/// The room being furnished: what is in the way, what has been set, and the way through.
struct Room<'a> {
    b: &'a Building,
    pal: &'a Palette,
    seed: u32,
    /// Whether anyone lives here: only then does anyone sleep in the beds or keep the fire.
    lived: bool,
    /// The layer a person's feet are in.
    feet: i32,
    /// The columns of the grid below, from `lo`, `size` of them.
    lo: IVec2,
    size: IVec2,
    /// Floor with the height of a person clear over it.
    floor: Vec<bool>,
    /// [`WAY_IN`] and [`BY_FIRE`], where nothing else stands.
    keep: Vec<u8>,
    /// Inside a window, where nothing tall stands against the wall.
    window: Vec<bool>,
    /// The kit's places for furniture, and its places for beds that no bed took: where
    /// things go first.
    spots: Vec<bool>,
    /// How many cells of what has been set are in each column below a person's head.
    blocked: Vec<u16>,
    solid: HashSet<IVec3>,
    /// The building's own fine parts, in cells: the kerb of a hearth, sills, shutters.
    fixed: Vec<(IVec3, IVec3)>,
    taken: HashSet<IVec3>,
    /// The floor just inside the doorway, where the way in starts.
    entries: Vec<IVec2>,
    /// Each a set of columns one of which must stay reached from the door: the hearth's
    /// sides, and the front of each thing set.
    targets: Vec<Vec<IVec2>>,
    /// While the beds are made, the front of each place for a bed, held reached like a
    /// target until a bed is made there (empty when none is held).
    held: Vec<Vec<IVec2>>,
    /// While the beds are made, which places for beds have a bed still to be made on
    /// them, standing in the way as if it were there.
    waiting: Vec<bool>,
    middle: Vec2,
    door: Vec2,
    out: Furnished,
}

impl<'a> Room<'a> {
    fn new(b: &'a Building, pal: &'a Palette, seed: u32, lived: bool) -> Option<Room<'a>> {
        let fy = b.inside.floor.first()?.y;
        let feet = fy + 1;
        let lo = b.walls.lo - IVec2::ONE;
        let size = b.walls.hi + IVec2::ONE - lo + IVec2::ONE;
        let n = (size.x * size.y) as usize;
        let solid: HashSet<IVec3> = b.voxels.iter().filter(|(_, v)| *v != AIR).map(|&(q, _)| q).collect();
        let mut room = Room {
            b,
            pal,
            seed,
            lived,
            feet,
            lo,
            size,
            floor: vec![false; n],
            keep: vec![0; n],
            window: vec![false; n],
            spots: vec![false; n],
            blocked: vec![0; n],
            solid,
            fixed: b.fine.iter().map(|p| (IVec3::from_array(p.lo), IVec3::from_array(p.hi))).collect(),
            taken: HashSet::new(),
            entries: Vec::new(),
            targets: Vec::new(),
            held: Vec::new(),
            waiting: Vec::new(),
            middle: Vec2::ZERO,
            door: Vec2::ZERO,
            out: Furnished::default(),
        };
        let mut sum = Vec2::ZERO;
        for f in &b.inside.floor {
            let q = IVec2::new(f.x, f.z);
            sum += q.as_vec2();
            if let Some(i) = room.index(q)
                && (feet..feet + HEAD).all(|y| !room.solid.contains(&IVec3::new(q.x, y, q.y)))
            {
                room.floor[i] = true;
            }
        }
        room.middle = sum / b.inside.floor.len() as f32;
        // The doorway's columns, and the floor just inside it at each.
        let w = b.door.width.max(1);
        let z0 = b.door.floor.z - (w - 1) / 2;
        let (z1, mut inner) = (z0 + w - 1, i32::MAX);
        for z in z0..=z1 {
            let x = b.inside.floor.iter().filter(|f| f.z == z).map(|f| f.x).max();
            if let Some(x) = x {
                room.entries.push(IVec2::new(x, z));
                inner = inner.min(x);
            }
        }
        if room.entries.is_empty() {
            return None;
        }
        room.door = IVec2::new(inner, (z0 + z1) / 2).as_vec2();
        // The way in, as the kit keeps it: three columns into the room, a column wider
        // than the doorway each side.
        let reach_x = room.entries.iter().map(|e| e.x).max().unwrap_or(inner);
        for z in z0 - 1..=z1 + 1 {
            for x in inner - 2..=reach_x {
                if let Some(i) = room.index(IVec2::new(x, z)) {
                    room.keep[i] |= WAY_IN;
                }
            }
        }
        if let Some(h) = b.inside.hearth {
            let at = IVec2::new(h.at.x, h.at.z);
            if h.central {
                for z in at.y - 1..=at.y + 2 {
                    for x in at.x - 1..=at.x + 2 {
                        room.mark(IVec2::new(x, z), BY_FIRE);
                    }
                }
                // Nobody walks through the fire, or its ashes: the beds are made round it.
                for q in [at, at + IVec2::X, at + IVec2::Y, at + IVec2::ONE] {
                    if let Some(k) = room.index(q) {
                        room.blocked[k] += 1;
                    }
                }
            } else {
                let (n, t) = (IVec2::new(h.facing.x, h.facing.z), IVec2::new(h.facing.z, h.facing.x));
                for k in -1..=1 {
                    for d in 1..=2 {
                        room.mark(at + t * k + n * d, BY_FIRE);
                    }
                }
            }
            room.targets.push(hearth_sides(h));
        }
        for o in &b.inside.windows {
            let n = IVec2::new(o.normal.x, o.normal.z);
            for z in o.lo.z..=o.hi.z {
                for x in o.lo.x..=o.hi.x {
                    if let Some(i) = room.index(IVec2::new(x, z) - n)
                        && room.floor[i]
                    {
                        room.window[i] = true;
                    }
                }
            }
        }
        for s in &b.inside.furniture {
            for c in s.cells() {
                room.mark_spot(IVec2::new(c.x, c.z));
            }
        }
        // Anything the way in cannot reach to begin with is not held against what is set.
        room.forget_unreached();
        Some(room)
    }

    /// Stops holding what is set to leaving a way to anything not reached now.
    fn forget_unreached(&mut self) {
        let reached = self.reach();
        let mut targets = std::mem::take(&mut self.targets);
        targets.retain(|t| t.iter().any(|q| self.index(*q).is_some_and(|i| reached[i])));
        self.targets = targets;
        let mut held = std::mem::take(&mut self.held);
        for t in &mut held {
            if !t.iter().any(|q| self.index(*q).is_some_and(|i| reached[i])) {
                t.clear();
            }
        }
        self.held = held;
    }

    fn index(&self, q: IVec2) -> Option<usize> {
        let r = q - self.lo;
        (r.x >= 0 && r.y >= 0 && r.x < self.size.x && r.y < self.size.y).then(|| (r.y * self.size.x + r.x) as usize)
    }

    fn is_floor(&self, q: IVec2) -> bool {
        self.index(q).is_some_and(|i| self.floor[i])
    }

    fn mark(&mut self, q: IVec2, bit: u8) {
        if let Some(i) = self.index(q) {
            self.keep[i] |= bit;
        }
    }

    fn mark_spot(&mut self, q: IVec2) {
        if let Some(i) = self.index(q) {
            self.spots[i] = true;
        }
    }

    /// Which floor columns someone coming in at the door stands over, walking among what
    /// has been set: a body two columns square with a person's height clear over it.
    fn reach(&self) -> Vec<bool> {
        let n = self.floor.len();
        let open = |q: IVec2| self.index(q).is_some_and(|i| self.floor[i] && self.blocked[i] == 0);
        let fits = |q: IVec2| open(q) && open(q + IVec2::X) && open(q + IVec2::Y) && open(q + IVec2::ONE);
        let (mut seen, mut reached) = (vec![false; n], vec![false; n]);
        let mut queue = VecDeque::new();
        for &e in &self.entries {
            for o in [IVec2::ZERO, IVec2::NEG_X, IVec2::NEG_Y, IVec2::NEG_ONE] {
                let p = e + o;
                if fits(p) {
                    let i = self.index(p).unwrap();
                    if !seen[i] {
                        seen[i] = true;
                        queue.push_back(p);
                    }
                }
            }
        }
        while let Some(q) = queue.pop_front() {
            for d in [IVec2::ZERO, IVec2::X, IVec2::Y, IVec2::ONE] {
                reached[self.index(q + d).unwrap()] = true;
            }
            for d in DIRS {
                let p = q + d;
                if fits(p) {
                    let i = self.index(p).unwrap();
                    if !seen[i] {
                        seen[i] = true;
                        queue.push_back(p);
                    }
                }
            }
        }
        reached
    }

    /// Whether every bed, the hearth and each thing set can still be walked up to.
    fn way_through(&self) -> bool {
        let reached = self.reach();
        let held = self.held.iter().filter(|t| !t.is_empty());
        self.targets.iter().chain(held).all(|t| t.iter().any(|q| self.index(*q).is_some_and(|i| reached[i])))
    }

    /// Whether voxel `c` is free to set something in: over the floor, not in a wall or in
    /// anything set before, and below a person's head not where `keep` says to keep clear.
    fn free(&self, c: IVec3, keep: u8) -> bool {
        let q = IVec2::new(c.x, c.z);
        let Some(i) = self.index(q) else { return false };
        c.y >= self.feet
            && self.floor[i]
            && (c.y >= self.feet + HEAD || self.keep[i] & keep == 0)
            && !self.solid.contains(&c)
            && !self.taken.contains(&c)
    }

    fn box_free(&self, lo: IVec3, hi: IVec3, keep: u8) -> bool {
        (lo.y..hi.y).all(|y| (lo.z..hi.z).all(|z| (lo.x..hi.x).all(|x| self.free(IVec3::new(x, y, z), keep))))
    }

    /// Whether `parts` keep out of the building's own fine parts. They may share a voxel,
    /// as the fire in a hut does with the kerb round it, but not a cell.
    fn clear_of_fixed(&self, parts: &Parts) -> bool {
        let fine = parts.fine.iter().map(|b| (IVec3::from_array(b.lo), IVec3::from_array(b.hi)));
        let voxels = parts.voxels.iter().map(|&(q, _)| (q * CELLS, (q + IVec3::ONE) * CELLS));
        fine.chain(voxels).all(|(lo, hi)| !self.fixed.iter().any(|&(a, b)| lo.cmplt(b).all() && a.cmplt(hi).all()))
    }

    fn take(&mut self, cells: &[IVec3], on: bool) {
        for &c in cells {
            if c.y >= self.feet
                && c.y < self.feet + HEAD
                && let Some(i) = self.index(IVec2::new(c.x, c.z))
            {
                if on {
                    self.blocked[i] += 1;
                } else {
                    self.blocked[i] -= 1;
                }
            }
            if on {
                self.taken.insert(c);
            } else {
                self.taken.remove(&c);
            }
        }
    }

    /// Sets `parts` if they are free of everything and leave the way through: `front` is
    /// where the thing is used from, one column of which must stay reached. With `force`
    /// it is set even if it blocks the way (a bed the kit planned for), but only held to
    /// staying reached if it is now.
    fn set(&mut self, what: What, decayed: bool, parts: Parts, (lo, hi): (IVec3, IVec3), front: Vec<IVec2>, keep: u8, force: bool) -> bool {
        let cells = cells_of(&parts.voxels, &parts.fine);
        if !cells.iter().all(|&c| self.free(c, keep)) || !self.clear_of_fixed(&parts) {
            return false;
        }
        self.take(&cells, true);
        let has_front = !front.is_empty();
        if has_front {
            self.targets.push(front);
        }
        if !self.way_through() {
            if has_front {
                self.targets.pop();
            }
            if !force {
                self.take(&cells, false);
                return false;
            }
            // Kept to only what it does not stop.
            self.forget_unreached();
        }
        self.commit(what, decayed, parts, lo, hi);
        true
    }

    fn commit(&mut self, what: What, decayed: bool, parts: Parts, lo: IVec3, hi: IVec3) {
        let (v0, f0) = (self.out.voxels.len(), self.out.fine.len());
        self.out.voxels.extend(parts.voxels);
        self.out.fine.extend(parts.fine);
        for h in parts.hooks {
            match h {
                // A broken bed is still slept in by those who have nothing better.
                Hook::Sleeper(q) if self.lived => self.out.sleepers.push(q),
                Hook::Fire(q) if self.lived && !decayed => self.out.fire = Some(q),
                _ => {}
            }
        }
        self.out.items.push(Item { what, decayed, lo, hi, voxels: v0..self.out.voxels.len(), fine: f0..self.out.fine.len() });
    }

    /// `f` built for this room and placed at `p`.
    fn build(&self, f: Fitting, decayed: bool, seed: u32, p: &Place) -> Parts {
        let built = fittings::build(f, self.pal, seed, decayed).place(p.at, p.turns);
        Parts { voxels: built.voxels, fine: built.fine, hooks: built.hooks }
    }

    /// A small random number for `q`, the same whatever order places are looked at in.
    fn jitter(&self, q: IVec2, salt: i32) -> f32 {
        unit(hash3(self.seed ^ 0x7a1c_e5e7, q.x, q.y, salt))
    }

    /// The box a thing `s` big takes with its back along the columns from `q` in the
    /// positive direction across `n`, facing `n`; and the place.
    fn place_at(&self, s: IVec3, q: IVec2, n: IVec2, along: IVec2, score: f32) -> Place {
        let cols = |i: i32, j: i32| q + along * i + n * j;
        let (mut lo, mut hi) = (IVec2::MAX, IVec2::MIN);
        for (i, j) in [(0, 0), (s.x - 1, 0), (0, s.z - 1), (s.x - 1, s.z - 1)] {
            lo = lo.min(cols(i, j));
            hi = hi.max(cols(i, j));
        }
        let turns = turns_to(IVec3::new(n.x, 0, n.y));
        let (a, b) = (rotate(IVec3::ZERO, turns), rotate(IVec3::new(s.x - 1, 0, s.z - 1), turns));
        let lo3 = IVec3::new(lo.x, self.feet, lo.y);
        Place {
            turns,
            at: lo3 - a.min(b),
            lo: lo3,
            hi: IVec3::new(hi.x + 1, self.feet + s.y, hi.y + 1),
            front: (0..s.x).map(|i| cols(i, s.z)).collect(),
            score,
        }
    }

    /// Places with the back of a thing `s` big against a wall over most of its length,
    /// facing into the room: on the kit's places for furniture first, then farthest from
    /// the door. Nothing tall stands across a window.
    fn against_walls(&self, s: IVec3) -> Vec<Place> {
        let mut out = Vec::new();
        for f in &self.b.inside.floor {
            let q = IVec2::new(f.x, f.z);
            for n in DIRS {
                if self.is_floor(q - n) || !self.is_floor(q) {
                    continue;
                }
                // Along the wall the positive way, so each box is looked at once.
                let along = if n.x != 0 { IVec2::Y } else { IVec2::X };
                if !(0..s.x).all(|i| (0..s.z).all(|j| self.is_floor(q + along * i + n * j))) {
                    continue;
                }
                let backed = (0..s.x).filter(|&i| !self.is_floor(q + along * i - n)).count() as i32;
                if backed * 2 <= s.x {
                    continue;
                }
                if s.y >= 3 && (0..s.x).any(|i| self.index(q + along * i).is_some_and(|k| self.window[k])) {
                    continue;
                }
                let on_spots = (0..s.x).filter(|&i| self.index(q + along * i).is_some_and(|k| self.spots[k])).count() as f32 / s.x as f32;
                let far = (q.as_vec2() - self.door).length();
                let score = 3.0 * on_spots + 0.15 * far + 2.0 * self.jitter(q, n.x * 3 + n.y);
                out.push(self.place_at(s, q, n, along, score));
            }
        }
        sort(&mut out);
        out
    }

    /// Places for a thing `s` big standing free on the floor, along either way, nearest
    /// the middle of the room first.
    fn in_the_open(&self, s: IVec3) -> Vec<Place> {
        let mut out = Vec::new();
        for f in &self.b.inside.floor {
            let q = IVec2::new(f.x, f.z);
            for (n, along) in [(IVec2::Y, IVec2::X), (IVec2::NEG_X, IVec2::Y)] {
                let mut p = self.place_at(s, q, n, along, 0.0);
                if !(p.lo.x..p.hi.x).all(|x| (p.lo.z..p.hi.z).all(|z| self.is_floor(IVec2::new(x, z)))) {
                    continue;
                }
                let mid = IVec2::new(p.lo.x + p.hi.x, p.lo.z + p.hi.z).as_vec2() * 0.5;
                p.score = -(mid - self.middle).length() + 1.5 * self.jitter(q, 7 + n.x);
                // Used from any side.
                p.front = ring(p.lo, p.hi);
                out.push(p);
            }
        }
        sort(&mut out);
        out
    }

    /// Sets fitting `f` against a wall if it can, or else, if it need not stand against
    /// one, out on the floor. Returns the place it took.
    fn put(&mut self, f: Fitting, decayed: bool, seed: u32) -> Option<Place> {
        let s = f.size();
        let mut places = self.against_walls(s);
        if f.stand() == Stand::Free {
            places.extend(self.in_the_open(s));
        }
        self.put_in(f, decayed, seed, places, BY_FIRE | WAY_IN)
    }

    fn put_in(&mut self, f: Fitting, decayed: bool, seed: u32, places: Vec<Place>, keep: u8) -> Option<Place> {
        let mut tries = 0;
        for p in places {
            if !self.box_free(p.lo, p.hi, keep) {
                continue;
            }
            let parts = self.build(f, decayed, seed, &p);
            if self.set(What::Fitting(f), decayed, parts, (p.lo, p.hi), p.front.clone(), keep, false) {
                return Some(p);
            }
            tries += 1;
            if tries >= TRIES {
                break;
            }
        }
        None
    }

    /// A bed for everyone who sleeps here, on the kit's places for beds: box beds for two
    /// in houses that can afford them, straw pallets or, in warm country, rope beds for
    /// one; a cradle for a baby, and children two to a bed. An empty home keeps a broken
    /// bed or two.
    fn beds(&mut self, h: &Household, rng: &mut Rng) {
        let spots = self.b.inside.beds.clone();
        let warm = matches!(h.region, Region::Desert | Region::Rainforest);
        let fireplace = self.b.inside.hearth.is_some_and(|x| !x.central);
        let wealth = h.wealth.clamp(0.0, 1.0);
        let box_beds = !warm && if fireplace { wealth >= 0.35 } else { wealth >= 0.8 };
        let single = if warm && wealth >= 0.3 { Fitting::RopeBed } else { Fitting::StrawPallet }.local(h.region);
        let mut used = vec![false; spots.len()];
        let n = h.sleepers as usize;
        // Which place each bed is made on, and who sleeps in it: 2 for a pair, 1 alone, 0 a
        // baby in a cradle. An empty home keeps a broken bed or two.
        let plan: Vec<(usize, u8)> = if h.lived_in {
            self.sleeping(h, &spots, box_beds, rng)
        } else {
            [0.75, 0.35].into_iter().enumerate().take(spots.len()).filter(|&(_, chance)| rng.unit() < chance).map(|(i, _)| (i, 1)).collect()
        };
        let claimed: Vec<usize> = plan.iter().map(|&(i, _)| i).collect();
        // Until it is made, the bed on each place claimed stands in the way, and the front
        // of its place is held reached: a bigger bed made first must leave room to walk up
        // to it.
        let reached = self.reach();
        self.waiting = vec![false; spots.len()];
        self.held = vec![Vec::new(); spots.len()];
        for &i in &claimed {
            let s = spots[i];
            let front: Vec<IVec2> = (0..s.len).map(|k| s.at + s.along * k + s.facing * s.wide).map(|c| IVec2::new(c.x, c.z)).collect();
            if front.iter().any(|q| self.index(*q).is_some_and(|k| reached[k])) {
                self.held[i] = front;
            }
            self.wait_for(i, true);
        }
        let decayed = !h.lived_in || (h.neglect > 0.7 && rng.unit() < (h.neglect - 0.7) * 2.0);
        for &(i, g) in &plan {
            let seed = rng.next_u32();
            used[i] = match g {
                0 => self.cradle(&spots, i, decayed, seed) || self.bed(&spots, i, &claimed, single, decayed, seed, 1),
                2 if box_beds => self.bed(&spots, i, &claimed, Fitting::BoxBed, decayed, seed, 2) || self.bed(&spots, i, &claimed, single, decayed, seed, 2),
                _ if box_beds && !h.lived_in => self.bed(&spots, i, &claimed, Fitting::BoxBed, decayed, seed, 1) || self.bed(&spots, i, &claimed, single, decayed, seed, 1),
                g => self.bed(&spots, i, &claimed, single, decayed, seed, g),
            };
            self.wait_for(i, false);
            self.held[i].clear();
        }
        if h.lived_in {
            // Anyone left over shares a bed already made.
            let beds: Vec<IVec3> = self.out.sleepers.clone();
            let mut k = 0;
            while self.out.sleepers.len() < n && !beds.is_empty() {
                let head = beds[k % beds.len()];
                self.out.sleepers.push(head + IVec3::new(1, 0, 1));
                k += 1;
            }
        }
        // The beds are made, and each holds the way to its own front; places for beds no bed
        // took are places for furniture.
        self.held.clear();
        for (s, _) in spots.iter().zip(&used).filter(|(_, u)| !**u) {
            for c in s.cells() {
                self.mark_spot(IVec2::new(c.x, c.z));
            }
        }
    }

    /// Who sleeps in which bed, as (place, how many): pairs in box beds where the house
    /// has them, otherwise each alone, children two to a bed half the time, the youngest
    /// in a cradle more often than not; two to a bed where there are too few places.
    fn sleeping(&mut self, h: &Household, spots: &[Spot], box_beds: bool, rng: &mut Rng) -> Vec<(usize, u8)> {
        {
            let n = h.sleepers as usize;
            let kids = (h.children as usize).min(n);
            let adults = n - kids;
            let cradle = kids > 0 && rng.unit() < 0.65;
            let mut groups: Vec<u8> = Vec::new();
            if box_beds {
                groups.extend(std::iter::repeat_n(2, adults / 2));
                groups.extend(std::iter::repeat_n(1, adults % 2));
            } else {
                groups.extend(std::iter::repeat_n(1, adults));
            }
            let in_beds = kids - cradle as usize;
            if box_beds || rng.unit() < 0.5 {
                groups.extend(std::iter::repeat_n(2, in_beds / 2));
                groups.extend(std::iter::repeat_n(1, in_beds % 2));
            } else {
                groups.extend(std::iter::repeat_n(1, in_beds));
            }
            if cradle {
                groups.push(0);
            }
            // More to sleep than places for beds: two to a bed where it can be.
            while groups.len() > spots.len() {
                let ones: Vec<usize> = groups.iter().enumerate().filter(|(_, g)| **g == 1).map(|(i, _)| i).take(2).collect();
                if ones.len() == 2 {
                    groups.remove(ones[1]);
                    groups[ones[0]] = 2;
                } else if let Some(i) = groups.iter().position(|g| *g == 0) {
                    groups.remove(i);
                } else {
                    break;
                }
            }
            groups.truncate(spots.len());
            // Each on a place in the kit's order or, with places to spare (box beds sleep
            // two), every other one, so that a bed bigger than its place has room to reach
            // past it.
            if !groups.is_empty() && groups.len() * 2 <= spots.len() {
                groups.iter().enumerate().map(|(k, &g)| (k * spots.len() / groups.len(), g)).collect()
            } else {
                groups.into_iter().enumerate().collect()
            }
        }
    }

    /// Puts the bed still to be made on bed place `i` in the way, or takes it out.
    fn wait_for(&mut self, i: usize, on: bool) {
        if self.waiting.get(i) != Some(&!on) {
            return;
        }
        self.waiting[i] = on;
        let cols: Vec<IVec2> = self.b.inside.beds[i].cells().map(|c| IVec2::new(c.x, c.z)).collect();
        for q in cols {
            if let Some(k) = self.index(q) {
                if on {
                    self.blocked[k] += 1;
                } else {
                    self.blocked[k] -= 1;
                }
            }
        }
    }

    /// A bed of `kind` on bed place `i`, reaching past it along the wall and out into the
    /// room as far as a bigger bed needs, but never onto the place of another bed, one of
    /// `claimed`. `sleeps` is how many lie in it: a bed for one shared by two children
    /// takes both.
    #[allow(clippy::too_many_arguments)]
    fn bed(&mut self, spots: &[Spot], i: usize, claimed: &[usize], kind: Fitting, decayed: bool, seed: u32, sleeps: u8) -> bool {
        let spot = spots[i];
        let others: Vec<IVec2> = claimed
            .iter()
            .filter(|&&k| k != i && k < spots.len())
            .flat_map(|&k| spots[k].cells().map(|c| IVec2::new(c.x, c.z)).collect::<Vec<_>>())
            .collect();
        // Its own place is its own to fill, and its front its own to block: the bed's front
        // is held instead.
        let own = self.held.get_mut(i).map(std::mem::take).unwrap_or_default();
        let waited = self.waiting.get(i) == Some(&true);
        self.wait_for(i, false);
        let made = self.bed_on(&spot, &others, kind, decayed, seed, sleeps);
        if !made {
            if let Some(h) = self.held.get_mut(i) {
                *h = own;
            }
            if waited {
                self.wait_for(i, true);
            }
        }
        made
    }

    fn bed_on(&mut self, spot: &Spot, others: &[IVec2], kind: Fitting, decayed: bool, seed: u32, sleeps: u8) -> bool {
        let spot = *spot;
        let s = kind.size();
        let tall = s.y >= 3;
        for shift in 0..=(s.x - spot.len).max(0) {
            let p = self.on_spot(&spot, s, shift);
            let cols: Vec<IVec2> = (p.lo.x..p.hi.x).flat_map(|x| (p.lo.z..p.hi.z).map(move |z| IVec2::new(x, z))).collect();
            if cols.iter().any(|c| others.contains(c)) || (tall && cols.iter().any(|c| self.index(*c).is_some_and(|k| self.window[k]))) {
                continue;
            }
            if !self.box_free(p.lo, p.hi, WAY_IN | BY_FIRE) {
                continue;
            }
            let mut parts = self.build(kind, decayed, seed, &p);
            if decayed {
                // Whoever sleeps in a broken bed lays their head where they would in a whole one.
                parts.hooks = self.build(kind, false, seed, &p).hooks;
            }
            if sleeps == 2 && kind.sleeps() == 1 {
                // Two children top to tail.
                let tail = parts.hooks.iter().find_map(|h| if let Hook::Sleeper(q) = h { Some(*q) } else { None });
                if let Some(head) = tail {
                    let foot = IVec3::new(p.lo.x + p.hi.x, 0, p.lo.z + p.hi.z) * CELLS - head * IVec3::new(1, 0, 1);
                    parts.hooks.push(Hook::Sleeper(IVec3::new(foot.x, head.y, foot.z)));
                }
            }
            // The kit planned a bed of its own size here, so one that fits its place is set
            // even if something set before would block it.
            let force = s.x <= spot.len && s.z <= spot.wide;
            if self.set(What::Fitting(kind), decayed, parts, (p.lo, p.hi), p.front.clone(), WAY_IN | BY_FIRE, force) {
                return true;
            }
        }
        false
    }

    /// A cradle at one end of bed place `i`.
    fn cradle(&mut self, spots: &[Spot], i: usize, decayed: bool, seed: u32) -> bool {
        let spot = spots[i];
        let s = Fitting::Cradle.size();
        let waited = self.waiting.get(i) == Some(&true);
        self.wait_for(i, false);
        for shift in [0, -(spot.len - s.x)] {
            let p = self.on_spot(&spot, s, shift);
            if !self.box_free(p.lo, p.hi, WAY_IN | BY_FIRE) {
                continue;
            }
            let mut parts = self.build(Fitting::Cradle, decayed, seed, &p);
            parts.hooks = self.build(Fitting::Cradle, false, seed, &p).hooks;
            if self.set(What::Fitting(Fitting::Cradle), decayed, parts, (p.lo, p.hi), p.front.clone(), WAY_IN | BY_FIRE, false) {
                return true;
            }
        }
        if waited {
            self.wait_for(i, true);
        }
        false
    }

    /// The place for a thing `s` big on bed place `spot`, starting `shift` columns before
    /// it along the wall, its back to the wall.
    fn on_spot(&self, spot: &Spot, s: IVec3, shift: i32) -> Place {
        let (along, n) = (IVec2::new(spot.along.x, spot.along.z), IVec2::new(spot.facing.x, spot.facing.z));
        let start = IVec2::new(spot.at.x, spot.at.z) - along * shift;
        let mut p = self.place_at(s, start, n, along, 0.0);
        p.front = (0..s.x).map(|i| start + along * i + n * s.z).collect();
        p
    }

    /// The hearth in the middle of a hut: its fire and the pot hung over it on the hearth
    /// stone the kit laid, within the kit's own kerb of stones (the fitting's ring is left
    /// off). It is turned whichever way leaves the way through; if none does, the posts
    /// and the pot are left off too.
    fn hut_hearth(&mut self, hearth: Hearth, lived: bool, rng: &mut Rng) {
        let decayed = !lived;
        let f = fittings::build(Fitting::HutHearth, self.pal, self.seed ^ 0x4ea7, decayed);
        let parts: Vec<PlantBox> = f.fine.into_iter().filter(|p| p.voxel != self.pal.stone && p.voxel != self.pal.stone_dark).collect();
        // The middle of the fitting's room, set on the middle of the hearth stone.
        let m = IVec3::new(10, 0, 10);
        let centre = IVec3::new((hearth.at.x + 1) * CELLS, self.feet * CELLS, (hearth.at.z + 1) * CELLS);
        let fire = vec![Hook::Fire(centre + IVec3::new(0, 3, 0))];
        let first = rng.int(0, 3) as u32;
        for k in 0..4 {
            let turns = (first + k) & 3;
            let fine = turn_about(&parts, m, turns, centre);
            let (lo, hi) = bounds(&fine);
            let p = Parts { voxels: Vec::new(), fine, hooks: fire.clone() };
            if self.set(What::Fitting(Fitting::HutHearth), decayed, p, (lo, hi), Vec::new(), WAY_IN, false) {
                return;
            }
        }
        let fire_only: Vec<PlantBox> = parts
            .iter()
            .filter(|p| p.voxel != mat::IRON && (0..3).step_by(2).all(|a| p.lo[a] >= m[a] - CELLS && p.hi[a] <= m[a] + CELLS))
            .copied()
            .collect();
        let fine = turn_about(&fire_only, m, 0, centre);
        let (lo, hi) = bounds(&fine);
        let p = Parts { voxels: Vec::new(), fine, hooks: fire };
        self.set(What::Fitting(Fitting::HutHearth), decayed, p, (lo, hi), Vec::new(), WAY_IN, true);
    }

    /// A fire in a fireplace, on a bed of ash between two logs, with a pot hung over it on
    /// a chain or a kettle on the embers; or, where nobody lives, cold charcoal and maybe a
    /// rusted pot. Built in the firebox's own frame, its back toward the chimney and its
    /// front toward the room.
    fn fireplace(&mut self, hearth: Hearth, lived: bool, rng: &mut Rng) {
        let mut fine = vec![fixed((0, 0, 0), (4, 1, 4), mat::ASH)];
        let mut b = |lo, hi, v| fine.push(fixed(lo, hi, v));
        if lived {
            b((0, 1, 0), (4, 2, 1), self.pal.wood);
            b((1, 1, 1), (3, 2, 3), mat::EMBER);
            let vessel = rng.int(0, 2);
            if vessel != 1 {
                // Flames licking up from the embers.
                b((1, 2, 1), (2, 4, 2), mat::EMBER);
                b((2, 2, 2), (3, 3, 3), mat::EMBER);
            }
            match vessel {
                0 => {
                    b((1, 4, 1), (3, 7, 3), mat::IRON);
                    b((2, 7, 2), (3, 12, 3), mat::IRON);
                }
                1 => {
                    b((1, 2, 1), (3, 4, 3), mat::IRON);
                    b((3, 3, 2), (4, 4, 3), mat::IRON);
                    b((2, 4, 2), (3, 5, 3), mat::IRON);
                }
                _ => {}
            }
        } else {
            b((1, 1, 1), (3, 2, 2), mat::CHARCOAL);
            if rng.unit() < 0.5 {
                b((1, 1, 2), (3, 3, 4), mat::RUST);
            }
        }
        let fine = parts::place_fine(&fine, hearth.at, turns_to(hearth.facing));
        let hooks = if lived { vec![Hook::Fire(hearth.at * CELLS + IVec3::new(CELLS / 2, 1, CELLS / 2))] } else { Vec::new() };
        let (lo, hi) = (hearth.at, hearth.at + IVec3::new(1, 3, 1));
        self.commit(What::Fireplace, !lived, Parts { voxels: Vec::new(), fine, hooks }, lo, hi);
    }

    /// A table, with stools about it or a bench along it.
    fn table(&mut self, decayed: bool, wealth: f32, rng: &mut Rng) {
        let s = Fitting::Table.size();
        let mut places = self.in_the_open(s);
        places.extend(self.against_walls(s));
        let Some(t) = self.put_in(Fitting::Table, decayed, self.seed ^ 0x7ab1e, places, BY_FIRE | WAY_IN) else { return };
        // The table's long sides, as rows of columns along it, either side.
        let long_x = t.hi.x - t.lo.x > t.hi.z - t.lo.z;
        let sides: Vec<Vec<IVec2>> = if long_x {
            [t.lo.z - 1, t.hi.z].iter().map(|&z| (t.lo.x..t.hi.x).map(|x| IVec2::new(x, z)).collect()).collect()
        } else {
            [t.lo.x - 1, t.hi.x].iter().map(|&x| (t.lo.z..t.hi.z).map(|z| IVec2::new(x, z)).collect()).collect()
        };
        if wealth < 0.35 && !decayed {
            // A bench along one side, and a stool at the other if there is room.
            for k in 0..2 {
                for start in [1, 0] {
                    let cols: Vec<IVec2> = sides[k].iter().skip(start).take(3).copied().collect();
                    if cols.len() == 3 && self.bench(&cols, long_x, rng.next_u32()) {
                        let at = sides[1 - k][1];
                        self.stool(at, decayed, rng.next_u32());
                        return;
                    }
                }
            }
        }
        let want = 2 + (wealth * 2.5) as usize;
        let mut set = 0;
        for k in [1, 2, 0, 3] {
            for side in &sides {
                if set < want && k < side.len() && self.stool(side[k], decayed || rng.unit() < 0.15, rng.next_u32()) {
                    set += 1;
                }
            }
        }
    }

    fn stool(&mut self, q: IVec2, decayed: bool, seed: u32) -> bool {
        let p = self.place_at(Fitting::Stool.size(), q, IVec2::Y, IVec2::X, 0.0);
        if !self.box_free(p.lo, p.hi, BY_FIRE | WAY_IN) {
            return false;
        }
        let parts = self.build(Fitting::Stool, decayed, seed, &p);
        self.set(What::Fitting(Fitting::Stool), decayed, parts, (p.lo, p.hi), ring(p.lo, p.hi), BY_FIRE | WAY_IN, false)
    }

    /// A plank bench along the three columns `cols`, which run along x if `along_x`.
    fn bench(&mut self, cols: &[IVec2], along_x: bool, seed: u32) -> bool {
        let lo = cols.iter().fold(IVec2::MAX, |a, c| a.min(*c));
        let (n, along) = if along_x { (IVec2::Y, IVec2::X) } else { (IVec2::NEG_X, IVec2::Y) };
        let p = self.place_at(IVec3::new(3, 2, 1), lo, n, along, 0.0);
        if !self.box_free(p.lo, p.hi, BY_FIRE | WAY_IN) {
            return false;
        }
        let built = props::build(Prop::Bench, self.pal, IVec3::ZERO, 0, false, seed);
        let voxels = built.voxels.iter().map(|&(q, v)| (rotate(q, p.turns) + p.at, v)).collect();
        let fine = parts::place_fine(&built.fine, p.at, p.turns);
        let parts = Parts { voxels, fine, hooks: Vec::new() };
        self.set(What::Bench, false, parts, (p.lo, p.hi), ring(p.lo, p.hi), BY_FIRE | WAY_IN, false)
    }

    /// Hangs `f` from a beam or the underside of the roof, high enough to walk under, or
    /// over something already set.
    fn hang(&mut self, f: Fitting, decayed: bool, seed: u32) {
        let s = f.size();
        let ceiling = |q: IVec2| (self.feet..self.feet + 24).find(|&y| self.solid.contains(&IVec3::new(q.x, y, q.y)));
        let mut places = Vec::new();
        for fl in &self.b.inside.floor {
            let q = IVec2::new(fl.x, fl.z);
            for (n, along) in [(IVec2::Y, IVec2::X), (IVec2::NEG_X, IVec2::Y)] {
                let cols: Vec<IVec2> = (0..s.x).map(|i| q + along * i).collect();
                let Some(top) = ceiling(q) else { continue };
                if !cols.iter().all(|&c| self.is_floor(c) && ceiling(c) == Some(top)) || top - s.y < self.feet + 3 {
                    continue;
                }
                let mut p = self.place_at(s, q, n, along, 0.0);
                let lift = top - s.y - self.feet;
                p.at.y += lift;
                p.lo.y += lift;
                p.hi.y += lift;
                // Over what is set already, or high, rather than where people walk.
                let over = cols.iter().filter(|c| self.index(**c).is_some_and(|i| self.blocked[i] > 0)).count() as f32;
                p.score = over + (top - self.feet) as f32 * 0.3 + 2.0 * self.jitter(q, 31 + n.x);
                p.front = Vec::new();
                places.push(p);
            }
        }
        sort(&mut places);
        self.put_in(f, decayed, seed, places, WAY_IN);
    }

    /// Glass in every window: lit where someone is at home (most of them), dark where
    /// nobody is, and broken in a home left long empty. Each pane is four lights between a
    /// cross of glazing bars, a cell in from the outside of the wall.
    /// Glass in every window: lit in most where `lit`, broken now and then where the home
    /// is not `kept`.
    fn panes(&mut self, kept: bool, lit: bool, neglect: f32, rng: &mut Rng) {
        let lived = lit;
        let c = CELLS;
        let windows = self.b.inside.windows.clone();
        let mut any_lit = false;
        for (k, o) in windows.iter().enumerate() {
            let last = k + 1 == windows.len();
            let lit = lived && (rng.unit() < 0.9 - 0.35 * neglect || (last && !any_lit));
            any_lit |= lit;
            let a = if o.normal.x != 0 { 0 } else { 2 };
            let across = 2 - a;
            let outer = if o.normal[a] > 0 { o.hi[a] } else { o.lo[a] };
            let (p0, p1) = if o.normal[a] > 0 { (outer * c + 2, outer * c + 3) } else { (outer * c + 1, outer * c + 2) };
            let (w0, w1) = (o.lo[across] * c, (o.hi[across] + 1) * c);
            let (y0, y1) = (o.lo.y * c, (o.hi.y + 1) * c);
            let (wm, ym) = ((w0 + w1) / 2, (y0 + y1) / 2);
            let glass = if lit { mat::WINDOW_LIT } else { mat::WINDOW_DARK };
            let mut fine = Vec::new();
            let mut push = |(u0, u1): (i32, i32), (v0, v1): (i32, i32), voxel: Voxel| {
                let (mut lo, mut hi) = ([0; 3], [0; 3]);
                (lo[a], hi[a]) = (p0, p1);
                (lo[across], hi[across]) = (u0, u1);
                (lo[1], hi[1]) = (v0, v1);
                fine.push(PlantBox { lo, hi, voxel, sway_lo: false, sway_hi: false, thin: 0, season: 0, flap: 0 });
            };
            let broken = if kept { 0.0 } else { 0.2 + 0.5 * neglect };
            for (u, v) in [((w0, wm), (y0, ym)), ((wm + 1, w1), (y0, ym)), ((w0, wm), (ym + 1, y1)), ((wm + 1, w1), (ym + 1, y1))] {
                if rng.unit() >= broken {
                    push(u, v, glass);
                }
            }
            push((wm, wm + 1), (y0, y1), self.pal.wood);
            push((w0, wm), (ym, ym + 1), self.pal.wood);
            push((wm + 1, w1), (ym, ym + 1), self.pal.wood);
            let (mut lo, mut hi) = (o.lo, o.hi + IVec3::ONE);
            lo[a] = outer;
            hi[a] = outer + 1;
            self.commit(What::Pane { lit }, !kept, Parts { voxels: Vec::new(), fine, hooks: Vec::new() }, lo, hi);
        }
    }
}

/// The columns about a box of voxels, on every side.
fn ring(lo: IVec3, hi: IVec3) -> Vec<IVec2> {
    let mut out = Vec::new();
    for x in lo.x..hi.x {
        out.push(IVec2::new(x, lo.z - 1));
        out.push(IVec2::new(x, hi.z));
    }
    for z in lo.z..hi.z {
        out.push(IVec2::new(lo.x - 1, z));
        out.push(IVec2::new(hi.x, z));
    }
    out
}

/// The columns beside a hearth from which it is tended: round a hearth in the floor, or
/// before a fireplace.
fn hearth_sides(h: Hearth) -> Vec<IVec2> {
    let at = IVec2::new(h.at.x, h.at.z);
    if h.central {
        [(-1, 0), (-1, 1), (2, 0), (2, 1), (0, -1), (1, -1), (0, 2), (1, 2)].iter().map(|&(x, z)| at + IVec2::new(x, z)).collect()
    } else {
        let (n, t) = (IVec2::new(h.facing.x, h.facing.z), IVec2::new(h.facing.z, h.facing.x));
        vec![at + n, at + n + t, at + n - t]
    }
}

/// The voxels fine parts draw into, `lo` to `hi` exclusive.
fn bounds(fine: &[PlantBox]) -> (IVec3, IVec3) {
    let cells = cells_of(&[], fine);
    let lo = cells.iter().fold(IVec3::MAX, |a, c| a.min(*c));
    let hi = cells.iter().fold(IVec3::MIN, |a, c| a.max(*c + IVec3::ONE));
    if cells.is_empty() { (IVec3::ZERO, IVec3::ZERO) } else { (lo, hi) }
}

fn sort(places: &mut [Place]) {
    places.sort_by(|a, b| b.score.total_cmp(&a.score).then((a.lo.z, a.lo.x, a.turns).cmp(&(b.lo.z, b.lo.x, b.turns))));
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::{
        noise::hash2,
        settlements::kit::{
            self, Step,
            tests::{SEEDS, Site, name, palette, spec, walked},
        },
    };

    /// A household for a home, varied by the seed: the kit's own sleepers for the first
    /// few, then anything from nobody to more than it has beds for; every trade in turn;
    /// now and then nobody at all.
    fn household(s: &kit::Spec, b: &Building) -> Household {
        let h = |k: i32| unit(hash2(s.seed ^ 0x4005e, k, s.step as i32 * 8 + s.region as i32));
        let beds = b.inside.beds.len() as u8;
        let sleepers = if s.seed < 4 || beds == 0 { s.sleepers.min(beds) } else { (h(1) * (beds as f32 + 3.0)) as u8 };
        let children = (h(2) * (sleepers as f32 * 0.7)) as u8;
        let trade = (s.seed % 3 != 1).then(|| Livelihood::ALL[(s.seed as usize / 3 + s.region as usize + s.step as usize) % Livelihood::ALL.len()]);
        Household { region: s.region, sleepers, children, trade, wealth: s.wealth, neglect: s.age * h(3), lived_in: s.seed % 5 != 4 }
    }

    /// Every home of every region and step, furnished: the kit's own spread of seeds, or
    /// as many as `FURNISH_SEEDS` says, for a longer look.
    fn every_furnished() -> impl Iterator<Item = (kit::Spec, Building, Household, Furnished)> {
        let seeds = std::env::var("FURNISH_SEEDS").ok().and_then(|v| v.parse().ok()).unwrap_or(SEEDS);
        Region::ALL.into_iter().flat_map(move |region| {
            let pal = palette(region);
            Step::ALL.into_iter().flat_map(move |step| {
                (0..seeds).map(move |seed| {
                    let s = spec(region, step, seed);
                    let b = kit::build(&s, &pal);
                    let h = household(&s, &b);
                    let f = furnish(&b, &h, &pal, seed ^ 0x9e37);
                    (s, b, h, f)
                })
            })
        })
    }

    fn item_cells(f: &Furnished, it: &Item) -> Vec<IVec3> {
        cells_of(&f.voxels[it.voxels.clone()], &f.fine[it.fine.clone()])
    }

    #[test]
    fn everyone_has_a_bed() {
        for (s, _, h, f) in every_furnished() {
            let want = if h.lived_in { h.sleepers as usize } else { 0 };
            assert_eq!(f.sleepers.len(), want, "{}: {h:?}", name(&s));
            let beds = f.items.iter().filter(|i| matches!(i.what, What::Fitting(k) if k.sleeps() > 0)).count();
            assert!(!h.lived_in || h.sleepers == 0 || beds > 0, "{}: nowhere to sleep", name(&s));
        }
    }

    #[test]
    fn nothing_is_set_in_the_walls_or_in_anything_else() {
        for (s, b, _, f) in every_furnished() {
            let solid: HashSet<IVec3> = b.voxels.iter().filter(|(_, v)| *v != AIR).map(|&(q, _)| q).collect();
            let mut whose: HashMap<IVec3, usize> = HashMap::new();
            for (k, it) in f.items.iter().enumerate() {
                for c in item_cells(&f, it) {
                    assert!(!solid.contains(&c), "{}: {:?} is in the building at {c}", name(&s), it.what);
                    if let Some(&o) = whose.get(&c) {
                        panic!("{}: {:?} and {:?} both fill {c}", name(&s), f.items[o].what, it.what);
                    }
                    whose.insert(c, k);
                }
                for p in &f.fine[it.fine.clone()] {
                    let (lo, hi) = (IVec3::from_array(p.lo), IVec3::from_array(p.hi));
                    assert!(lo.cmplt(hi).all(), "{}: {:?} has an empty part", name(&s), it.what);
                    for q in &b.fine {
                        let (a, c) = (IVec3::from_array(q.lo), IVec3::from_array(q.hi));
                        assert!(!(lo.cmplt(c).all() && a.cmplt(hi).all()), "{}: {:?} cuts into the building's {:?}", name(&s), it.what, q);
                    }
                }
            }
        }
    }

    /// Someone coming in at the door can still walk up to every bed, the hearth and
    /// everything set on the floor, as the kit's own check walks, with everything set
    /// standing solid in the way.
    #[test]
    fn people_walk_in_to_every_bed_and_the_hearth() {
        for (s, b, _, f) in every_furnished() {
            let mut site = Site::new(&b);
            for it in &f.items {
                for c in item_cells(&f, it) {
                    site.at.insert(c, mat::PLANK);
                }
            }
            let floor = walked(&b, &site);
            let fy = b.inside.floor[0].y;
            let reached = |cols: &[IVec2]| cols.iter().any(|q| floor.contains(&IVec3::new(q.x, fy, q.y)));
            if let Some(h) = b.inside.hearth {
                assert!(reached(&hearth_sides(h)), "{}: nobody can get to the hearth", name(&s));
            }
            for it in &f.items {
                // What hangs overhead is reached from under it, or over what it hangs over.
                let hung = matches!(it.what, What::Fitting(k) if k.stand() == Stand::Hung);
                if hung || matches!(it.what, What::Pane { .. } | What::Fireplace | What::Fitting(Fitting::HutHearth)) {
                    continue;
                }
                assert!(reached(&ring(it.lo, it.hi)), "{}: nobody can get to the {:?} at {}..{}", name(&s), it.what, it.lo, it.hi);
            }
        }
    }

    #[test]
    fn lights_burn_where_someone_is_home() {
        for (s, b, h, f) in every_furnished() {
            let lit = f.items.iter().filter(|i| i.what == What::Pane { lit: true }).count();
            let panes = f.items.iter().filter(|i| matches!(i.what, What::Pane { .. })).count();
            assert_eq!(panes, b.inside.windows.len(), "{}: a window without glass", name(&s));
            let store = h.lived_in && h.sleepers == 0 && h.trade.is_none();
            if store {
                // Kept, but nobody there at night and no fire in it.
                assert_eq!(lit, 0, "{}: a light in a store", name(&s));
                assert!(f.fire.is_none() && f.sleepers.is_empty(), "{}: someone in a store", name(&s));
            } else if h.lived_in {
                assert!(panes == 0 || lit > 0, "{}: every window dark in a lived-in home", name(&s));
                assert_eq!(f.fire.is_some(), b.inside.hearth.is_some(), "{}: the fire", name(&s));
            } else {
                assert_eq!(lit, 0, "{}: a light in an empty home", name(&s));
                assert!(f.fire.is_none() && f.sleepers.is_empty(), "{}: someone in an empty home", name(&s));
                assert!(f.items.iter().all(|i| i.decayed), "{}: something kept whole in an empty home", name(&s));
            }
        }
    }

    #[test]
    fn a_trade_is_worked_where_it_is_kept() {
        let (mut worked, mut homes) = (0, 0);
        for (_, _, h, f) in every_furnished() {
            let Some(t) = h.trade.filter(|_| h.lived_in) else { continue };
            homes += 1;
            let tools: Vec<Fitting> = trade_fittings(t).iter().map(|k| k.local(h.region)).collect();
            if f.items.iter().any(|i| matches!(i.what, What::Fitting(k) if tools.contains(&k))) {
                worked += 1;
            }
        }
        // A hut with beds for all has no room for a forge; most homes have room for
        // something of their trade.
        assert!(worked * 10 >= homes * 8, "the trade was set in only {worked} of {homes} workplaces");
    }

    /// Every kind of bed is made somewhere, and the fittings of the trades are set: a
    /// count of everything set, printed with --nocapture.
    #[test]
    fn every_kind_of_bed_is_made_somewhere() {
        let mut count: HashMap<String, usize> = HashMap::new();
        for (_, _, _, f) in every_furnished() {
            for it in &f.items {
                let name = match it.what {
                    What::Fitting(k) => format!("{k:?}"),
                    w => format!("{w:?}"),
                };
                *count.entry(if it.decayed { format!("{name} (decayed)") } else { name }).or_default() += 1;
            }
        }
        let mut all: Vec<_> = count.iter().collect();
        all.sort();
        println!("{all:?}");
        for bed in [Fitting::StrawPallet, Fitting::BoxBed, Fitting::RopeBed, Fitting::Cradle, Fitting::Loom, Fitting::Forge, Fitting::NetRack] {
            assert!(count.get(&format!("{bed:?}")).is_some_and(|&n| n > 0), "no {bed:?} anywhere");
        }
    }

    /// Prints a furnished home's plan at the height of the feet, for looking into a
    /// failure: `FURNISH_SHOW="oldgrowth stone 16" cargo test ... furnish::tests::show --
    /// --ignored --nocapture`. Each thing set is a letter (listed below the plan), `#` is
    /// the building, `,` floor walked to, `.` floor not, `D` the door's floor.
    #[test]
    #[ignore]
    fn show() {
        let Ok(want) = std::env::var("FURNISH_SHOW") else { return };
        let words: Vec<&str> = want.split_whitespace().collect();
        let (region, step) = (Region::ALL.into_iter().find(|r| r.word() == words[0]).unwrap(), Step::ALL.into_iter().find(|s| s.word() == words[1]).unwrap());
        let s = spec(region, step, words[2].parse().unwrap());
        let pal = palette(region);
        let b = kit::build(&s, &pal);
        let h = household(&s, &b);
        let f = furnish(&b, &h, &pal, s.seed ^ 0x9e37);
        let mut site = Site::new(&b);
        let mut whose: HashMap<IVec2, usize> = HashMap::new();
        let fy = b.inside.floor[0].y;
        for (k, it) in f.items.iter().enumerate() {
            for c in item_cells(&f, it) {
                site.at.insert(c, mat::PLANK);
                if c.y > fy && c.y <= fy + HEAD {
                    whose.entry(IVec2::new(c.x, c.z)).or_insert(k);
                }
            }
        }
        let walked = walked(&b, &site);
        println!("{}: {h:?}", name(&s));
        let (lo, hi) = (b.walls.lo - IVec2::ONE, b.walls.hi + IVec2::ONE);
        for z in lo.y..=hi.y {
            let row: String = (lo.x..=hi.x)
                .map(|x| {
                    let q = IVec3::new(x, fy, z);
                    if let Some(&k) = whose.get(&IVec2::new(x, z)) {
                        (b'a' + (k % 26) as u8) as char
                    } else if q == b.door.floor {
                        'D'
                    } else if site.solid(q + IVec3::Y) {
                        '#'
                    } else if walked.contains(&q) {
                        ','
                    } else if b.inside.floor.contains(&q) {
                        '.'
                    } else {
                        ' '
                    }
                })
                .collect();
            println!("{z:4} {row}");
        }
        for (k, it) in f.items.iter().enumerate() {
            println!("{} {:?}{} {}..{}", (b'a' + (k % 26) as u8) as char, it.what, if it.decayed { " (decayed)" } else { "" }, it.lo, it.hi);
        }
    }

    #[test]
    fn the_same_household_is_furnished_the_same() {
        for (s, b, h, f) in every_furnished().step_by(7) {
            let again = furnish(&b, &h, &palette(s.region), s.seed ^ 0x9e37);
            assert_eq!(format!("{f:?}"), format!("{again:?}"), "{}", name(&s));
        }
    }
}
