//! Settlements over the years (#207). A settlement is laid out once, as it stands in
//! year 0: today's world, the same in every season. From year 1 on, what stands on its
//! plots changes a little each season and a thing or two each year: fresh thatch fades,
//! woodpiles grow through the summer and are burnt down through the winter, hives come
//! and go, a home falls out of use and stands empty with its roof going, and one that
//! has stood empty a while is taken down and built again, its walls rising inside
//! scaffolding for a summer before it is roofed and lived in.
//!
//! Nothing here is simulated as the game runs, and nothing is saved. Each building's
//! year built and the year it fell out of use are worked out afresh from the seed and
//! the year, by replaying its settlement's years from the first ([`Lives::new`], a pure
//! function), and what a stamp looks like then from the [`Tag`] it was laid with
//! ([`Lives::view`]). The ground is the same in every year: a plot's pad, its garden's
//! bed and the paths stay where they were laid, so only what is stamped changes, and a
//! settlement never holds more homes than its land took when it was laid out.
//!
//! The player's hand comes in only as an input: the boons granted ([`Boons`]), kept by
//! the game beside its save, which bring what they add into view from the year granted
//! and change how a settlement fares from then on.

use std::{borrow::Cow, collections::HashMap};

use glam::{IVec2, IVec3};
use voxel::{
    AIR, Voxel,
    mesh::{CELLS, PlantBox},
};

use super::{Kind, Livelihood, Settlement, layout::Purpose};
use crate::{
    features::fittings::Fitting,
    materials as mat,
    noise::{hash2, hash3, unit},
    trails::{Network, Stamp},
};

/// A moment of the world's years: the year, counted from the one the world is first
/// seen in, and the season of it (0 spring, 1 summer, 2 autumn, 3 winter). Year 0 is
/// today's world, as laid out, in every season.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct When {
    pub year: i32,
    pub season: u8,
}

impl When {
    /// Today's world.
    pub const TODAY: When = When { year: 0, season: 0 };

    pub fn new(year: i32, season: u8) -> When {
        When { year, season: season.min(3) }
    }

    /// From the game's calendar: its year, and its date (0 to 1 from the spring
    /// equinox, a season each quarter).
    pub fn of(year: u32, date: f32) -> When {
        When::new(year.min(i32::MAX as u32) as i32, (date.rem_euclid(1.0) * 4.0) as u8)
    }

    /// Seasons since the spring of year 0.
    pub fn quarter(self) -> i32 {
        self.year * 4 + self.season as i32
    }

    /// The moment as far as what is built goes: every season of year 0 is the same.
    pub fn seen(self) -> When {
        if self.year <= 0 { When::TODAY } else { self }
    }
}

/// Something the player has done for a settlement that changes it from then on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Boon {
    /// A second well sunk: more water, so a home that falls empty is soon built again
    /// and no more than one stands empty at a time.
    SecondWell,
    /// The guild hall opened: a great town's guild house hung with its banners.
    GuildHall,
}

impl Boon {
    pub const ALL: [Boon; 2] = [Boon::SecondWell, Boon::GuildHall];

    /// Its name in the console and the save.
    pub fn name(self) -> &'static str {
        match self {
            Boon::SecondWell => "second-well",
            Boon::GuildHall => "guild-hall",
        }
    }

    pub fn by_name(name: &str) -> Option<Boon> {
        let name = name.trim().replace([' ', '_'], "-");
        Boon::ALL.into_iter().find(|b| b.name().eq_ignore_ascii_case(&name))
    }

    /// What it does, in a few words.
    pub fn about(self) -> &'static str {
        match self {
            Boon::SecondWell => "a second well on the green: empty homes are built again",
            Boon::GuildHall => "a great town's guild hall opened, its banners hung",
        }
    }
}

/// One boon granted: to which settlement (the continent it is on, by macro-cell, and its
/// index among that continent's settlements), and in which year.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Granted {
    pub cell: (i32, i32),
    pub settlement: u32,
    pub boon: Boon,
    pub year: i32,
}

/// Every boon granted in one world: the game keeps it beside the save, and it is all
/// of the player's hand that the settlements' years take in.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Boons {
    pub granted: Vec<Granted>,
}

impl Boons {
    /// Grants `g`, unless that settlement has that boon already; whether it did.
    pub fn grant(&mut self, g: Granted) -> bool {
        if self.granted.iter().any(|h| h.cell == g.cell && h.settlement == g.settlement && h.boon == g.boon) {
            return false;
        }
        self.granted.push(g);
        true
    }

    /// The year settlement `k` of continent `cell` was granted `boon`, if it was.
    pub fn year_of(&self, cell: (i32, i32), k: usize, boon: Boon) -> Option<i32> {
        self.granted.iter().find(|g| g.cell == cell && g.settlement as usize == k && g.boon == boon).map(|g| g.year)
    }

    /// One line a boon: `second-well 3 -1 12 4` (the boon, the continent, the
    /// settlement and the year).
    pub fn to_text(&self) -> String {
        self.granted.iter().map(|g| format!("{} {} {} {} {}\n", g.boon.name(), g.cell.0, g.cell.1, g.settlement, g.year)).collect()
    }

    /// As [`Self::to_text`] wrote it; lines it cannot read are passed over.
    pub fn from_text(text: &str) -> Boons {
        let mut boons = Boons::default();
        for line in text.lines() {
            let w: Vec<&str> = line.split_whitespace().collect();
            let [name, cx, cz, k, year] = w[..] else { continue };
            let (Some(boon), Ok(cx), Ok(cz), Ok(k), Ok(year)) = (Boon::by_name(name), cx.parse(), cz.parse(), k.parse(), year.parse()) else { continue };
            boons.grant(Granted { cell: (cx, cz), settlement: k, boon, year });
        }
        boons
    }
}

/// What a stamp is, for telling how it looks in a year (see [`Lives::view`]). Stamps
/// with none are the same in every year.
#[derive(Clone, Debug, PartialEq)]
pub enum Tag {
    /// A plot's building, its shell and what furnishes it.
    Building(Building),
    /// Something set out about a plot: in its yard, its garden or against its walls.
    Yard { plot: u32, what: Yard },
    /// Built only for a boon: unseen until it is granted.
    Boon { settlement: u32, boon: Boon },
}

/// What a building's stamp needs to be seen in another year.
#[derive(Clone, Debug, PartialEq)]
pub struct Building {
    /// Its plot, by index in `Network::plots`.
    pub plot: u32,
    /// A bit for each of the stamp's voxels, in its order, set for those of the
    /// furnishing rather than the shell.
    inside: Vec<u64>,
    /// The level it stands at (its plot's ground is a voxel below), the lowest voxel of
    /// its roof covering, and the columns inside its outer walls, inclusive.
    ground: i32,
    eaves: i32,
    walls: (IVec2, IVec2),
    /// The level of its floor (the voxel stood on), the top course of its walls, and which
    /// of the columns of `walls` its walls stand on, a bit each, row by row from
    /// `walls.0`: for its walls rising inside scaffolding as it is built again (see
    /// [`frame`]).
    floor: i32,
    plate: i32,
    wall: Vec<u64>,
    /// Its roof's thatch, if it is thatched: as it is new, before it weathers.
    pub thatch: Option<Voxel>,
}

/// The kinds of things set about a plot that change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Yard {
    /// A stack of logs against a wall.
    Woodpile,
    /// A hive, numbered among the plot's own from 0.
    Hive(u8),
    /// A washing line, and what hangs on it.
    Line,
    /// A garden's bed or its hurdles, the chopping block: what is dug or set into the
    /// ground and stays when the house is gone.
    Ground,
    /// Anything else of the household's.
    Other,
}

/// What the layout keeps for the years beside what it builds: in `Network::years`.
#[derive(Default)]
pub struct Record {
    /// The plot each of `Network::chimneys` rises from, in step with it.
    pub chimney_plot: Vec<u32>,
    /// Where each boon would add what it builds, by settlement.
    pub sites: Vec<Site>,
    /// The logging camps out in the woods, whose clearings grow by the year (see
    /// `settlements::logging`).
    pub camps: Vec<super::logging::Camp>,
    /// Tags for props held back until the lanes are worn, by their index among the
    /// network's held props, and what each prop held while dressing a plot is.
    pub(crate) held: HashMap<usize, Tag>,
    pub(crate) kinds: HashMap<usize, Fitting>,
}

/// Where a boon would build: the settlement, the boon, and the foot of what it adds,
/// continent-local.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Site {
    pub settlement: u32,
    pub boon: Boon,
    pub at: IVec3,
}

/// The order stamps sort their voxels in (see `Network::add_stamp_with`).
fn key(p: IVec3) -> (i32, i32, i32) {
    (p.y, p.z, p.x)
}

impl Tag {
    /// The tag of a plot's building, whose `voxels` are its shell's `shell` and then its
    /// furnishing's, and `room` the voxels of its floor (the kit's `Inside::floor`), all
    /// where they stand. Puts `voxels` in the order the stamp keeps them (as
    /// `Network::add_stamp_with` sorts them: stably, so to the same order), to mark which
    /// are the furnishing's.
    pub(crate) fn building(plot: usize, voxels: &mut Vec<(IVec3, Voxel)>, shell: usize, ground: i32, walls: (IVec2, IVec2), room: &[IVec3]) -> Tag {
        let mut order: Vec<u32> = (0..voxels.len() as u32).collect();
        order.sort_by_key(|&i| key(voxels[i as usize].0));
        let mut inside = vec![0u64; voxels.len().div_ceil(64)];
        for (j, &i) in order.iter().enumerate() {
            if i as usize >= shell {
                inside[j / 64] |= 1 << (j % 64);
            }
        }
        *voxels = order.iter().map(|&i| voxels[i as usize]).collect();
        let is_inside = |j: usize| inside[j / 64] >> (j % 64) & 1 != 0;
        let shell_of = || voxels.iter().enumerate().filter(|&(j, _)| !is_inside(j)).map(|(_, v)| *v);
        let top = shell_of().filter(|&(_, v)| v != AIR).map(|(p, _)| p.y).max().unwrap_or(ground);
        let eaves = shell_of().filter(|&(_, v)| mat::ROOFING.contains(&v)).map(|(p, _)| p.y).min().unwrap_or(top + 1);
        let mut counts = [0usize; 3];
        for (p, v) in shell_of() {
            if p.y >= eaves
                && let Some(i) = THATCHES.iter().position(|t| t[0] == v)
            {
                counts[i] += 1;
            }
        }
        let most = (0..3).max_by_key(|&i| (counts[i], std::cmp::Reverse(i))).filter(|&i| counts[i] > 0);
        let (floor, plate, wall) = frame(shell_of(), ground, walls, room, eaves);
        Tag::Building(Building { plot: plot as u32, inside, ground, eaves, walls, floor, plate, wall, thatch: most.map(|i| THATCHES[i][0]) })
    }
}

/// How a building's walls stand, from its shell's voxels (`shell`), the columns inside
/// its outer walls (`walls`, inclusive) and the voxels of its floor (`room`): the level of
/// its floor (`ground` with none), the top course of its walls, and which columns of
/// `walls` they stand on, a bit each (see [`Building::wall`]).
///
/// The walls stand on the columns outside the room with something solid at the floor's
/// level: a wall's foot, or a raised floor run out under it, and the doorway's threshold
/// with them, so that they close the room in. Their top is a voxel under the lowest of
/// the roof's covering over them, which sits on them. A wall of reeds is covered as a roof
/// is, so only covering as high as the roof over the room is taken for the roof's: a
/// voxel or two over the walls' top, as the roof rises from the walls inward.
fn frame(shell: impl Iterator<Item = (IVec3, Voxel)> + Clone, ground: i32, (lo, hi): (IVec2, IVec2), room: &[IVec3], eaves: i32) -> (i32, i32, Vec<u64>) {
    let (w, d) = ((hi.x - lo.x + 1).max(0) as usize, (hi.y - lo.y + 1).max(0) as usize);
    let at = |p: IVec3| (p.x >= lo.x && p.x <= hi.x && p.z >= lo.y && p.z <= hi.y).then(|| (p.z - lo.y) as usize * w + (p.x - lo.x) as usize);
    let floor = room.iter().map(|q| q.y).min().unwrap_or(ground);
    let mut indoors = vec![false; w * d];
    for &q in room {
        if let Some(i) = at(q) {
            indoors[i] = true;
        }
    }
    // The cheap tests first: most of a shell is its rooms' air and its walls' lower courses.
    let mut wall = vec![0u64; (w * d).div_ceil(64)];
    let mut over_room = i32::MAX;
    for (p, v) in shell.clone() {
        let Some(i) = at(p) else { continue };
        if indoors[i] {
            if p.y > floor && p.y < over_room && v != AIR && mat::ROOFING.contains(&v) {
                over_room = p.y;
            }
        } else if p.y == floor && v != AIR && !mat::ROOFING.contains(&v) {
            wall[i / 64] |= 1 << (i % 64);
        }
    }
    let on_wall = |i: usize| wall[i / 64] >> (i % 64) & 1 != 0;
    // Without a room to go by, the walls stand at least a person's height.
    let low = if over_room == i32::MAX { floor + 5 } else { over_room - 1 };
    let mut on_walls = i32::MAX;
    for (p, v) in shell {
        if p.y >= low
            && p.y < on_walls
            && let Some(i) = at(p)
            && on_wall(i)
            && v != AIR
            && mat::ROOFING.contains(&v)
        {
            on_walls = p.y;
        }
    }
    let plate = if on_walls < i32::MAX {
        on_walls - 1
    } else if over_room < i32::MAX {
        over_room - 2
    } else {
        eaves - 1
    };
    (floor, plate.max(floor + 3), wall)
}

/// Tags everything held back while plot `plot` was dressed: the props from `from` on.
pub(crate) fn tag_yard(net: &mut Network, from: usize, plot: usize) {
    let mut hives = 0u8;
    for n in from..net.held_props.len() {
        let what = match net.years.kinds.remove(&n) {
            Some(Fitting::LogStack | Fitting::RoundStack) => Yard::Woodpile,
            Some(Fitting::Skep | Fitting::LogHive | Fitting::BoxHive) => {
                hives += 1;
                Yard::Hive(hives - 1)
            }
            Some(Fitting::WashingLine) => Yard::Line,
            Some(Fitting::Hurdle) | None => Yard::Ground,
            Some(_) => Yard::Other,
        };
        net.years.held.insert(n, Tag::Yard { plot: plot as u32, what });
    }
}

/// Each thatch as it is new, then in three shades fresher, from just laid to nearly
/// weathered to it. A roof takes one of them all over, never a mix.
const THATCHES: [[Voxel; 4]; 3] = [
    [mat::THATCH, mat::THATCH_NEW, mat::THATCH_NEW_2, mat::THATCH_NEW_3],
    [mat::REED_THATCH, mat::REED_NEW, mat::REED_NEW_2, mat::REED_NEW_3],
    [mat::PALM_THATCH, mat::PALM_NEW, mat::PALM_NEW_2, mat::PALM_NEW_3],
];

/// Whether `v` is any thatch, of any age.
fn thatch(v: Voxel) -> bool {
    v == mat::THATCH_OLD || THATCHES.iter().any(|t| t.contains(&v))
}

/// The colour of a roof of `base` thatched `since` seasons ago: fresh for the season it
/// was laid, paler the next two, and then as thatch is.
fn shade(base: Voxel, since: i32) -> Voxel {
    let Some(t) = THATCHES.iter().find(|t| t[0] == base) else { return base };
    match since {
        0 => t[1],
        1 => t[2],
        2 | 3 => t[3],
        _ => t[0],
    }
}

/// What stands on a plot at a moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Standing and lived in, or kept: `rebuilt` 0 as it stood in year 0, and one more each
    /// time it has been built again; `thatched`, the season (see [`When::quarter`]) its
    /// roof was last thatched, if it has been since year 0.
    Standing { rebuilt: u32, thatched: Option<i32> },
    /// Fallen out of use in year `since`: nobody lives there, the fire is out, the
    /// windows are dark and the roof is going.
    Abandoned { since: i32 },
    /// Taken down to its pad, to be built again: the spring of the year it is.
    Cleared,
    /// Its walls rising inside scaffolding: the summer after.
    Scaffold,
}

impl State {
    /// Whether anyone lives or works there.
    pub fn standing(self) -> bool {
        matches!(self, State::Standing { .. })
    }
}

/// One building's life, as far as the moment it was asked for: when what stands there
/// now was finished (0 for what stood in year 0), and when it fell out of use if it has
/// since and stands empty.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Life {
    pub built: i32,
    pub fell: Option<i32>,
    pub state: State,
}

const TODAY: Life = Life { built: 0, fell: None, state: State::Standing { rebuilt: 0, thatched: None } };

/// What a settlement can hold: at most the homes its land took when it was laid out
/// (`homes`), of which its water lets no more than `empty` stand out of use at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capacity {
    pub homes: usize,
    pub empty: usize,
}

/// How each settlement of a continent stands at one moment: every plot's state, how
/// many hives each keeps, and the boons granted by then.
pub struct Lives {
    pub when: When,
    /// Each plot's life, by index in `Network::plots`; empty in year 0, when every one
    /// is as built.
    lives: Vec<Life>,
    /// Each settlement's hives, out of the four an apiary sets out; empty in year 0.
    hives: Vec<u8>,
    capacity: Vec<Capacity>,
    /// The boons granted on the continent by now, by settlement.
    granted: Vec<(u32, Boon)>,
    /// The settlement each plot belongs to; empty in year 0.
    settlement: Vec<u32>,
    seed: u32,
}

/// Salts for the replay's rolls.
const ORDER: u32 = 0x5e8a_0d3e;
const EVENT: u32 = 0x5e8a_e7e7;

/// Hives an apiary keeps at most and at least.
const HIVES: u8 = 4;
const FEWEST_HIVES: u8 = 2;

impl Lives {
    /// Today's world: everything as built, and no boons.
    pub fn today(net: &Network) -> Lives {
        Lives::new(net, 0, (0, 0), When::TODAY, &Boons::default())
    }

    /// How the settlements of `net` (continent `cell` of the world of `seed`) stand at
    /// `when`, with `boons` granted. A pure function of these: the settlements' years
    /// are replayed from the first, each the same every time.
    pub fn new(net: &Network, seed: u32, cell: (i32, i32), when: When, boons: &Boons) -> Lives {
        let when = when.seen();
        let granted: Vec<(u32, Boon)> = boons.granted.iter().filter(|g| g.cell == cell && g.year <= when.year).map(|g| (g.settlement, g.boon)).collect();
        let homes_of = |k: usize| -> Vec<usize> { net.layouts.get(k).map_or(Vec::new(), |l| l.plots.clone().filter(|&i| churns(net.plots[i].purpose)).collect()) };
        let capacity: Vec<Capacity> = (0..net.settlements.len())
            .map(|k| {
                let homes = homes_of(k).len();
                let well = boons.year_of(cell, k, Boon::SecondWell).is_some_and(|y| y <= when.year);
                Capacity { homes, empty: slack(&net.settlements[k], homes, well) }
            })
            .collect();
        let mut lives = Lives { when, lives: Vec::new(), hives: Vec::new(), capacity, granted, settlement: Vec::new(), seed };
        if when.year <= 0 {
            return lives;
        }
        // Which plots have thatched roofs, from their buildings' tags.
        let mut thatched = vec![false; net.plots.len()];
        for s in net.stamps() {
            if let Some(Tag::Building(b)) = s.tag.as_deref()
                && let Some(t) = thatched.get_mut(b.plot as usize)
            {
                *t = b.thatch.is_some();
            }
        }
        lives.lives = vec![TODAY; net.plots.len()];
        lives.settlement = net.plots.iter().map(|p| p.settlement as u32).collect();
        lives.hives = vec![HIVES; net.settlements.len()];
        for k in 0..net.settlements.len() {
            let homes = homes_of(k);
            if homes.is_empty() {
                continue;
            }
            let s = &net.settlements[k];
            let apiary = net.layouts[k].plots.clone().any(|i| net.plots[i].purpose == Purpose::Workplace(Livelihood::Apiary));
            let well = boons.year_of(cell, k, Boon::SecondWell);
            let thatch: Vec<bool> = homes.iter().map(|&i| thatched[i]).collect();
            let (recs, hives) = replay(seed, k, s, &thatch, apiary, well, when.year);
            lives.hives[k] = hives;
            for (&i, r) in homes.iter().zip(&recs) {
                lives.lives[i] = r.at(when);
            }
        }
        lives
    }

    /// The life of plot `plot` (by index in `Network::plots`).
    pub fn life(&self, plot: usize) -> Life {
        self.lives.get(plot).copied().unwrap_or(TODAY)
    }

    pub fn state(&self, plot: usize) -> State {
        self.life(plot).state
    }

    /// Whether anyone lives or works in plot `plot`'s building now.
    pub fn standing(&self, plot: usize) -> bool {
        self.state(plot).standing()
    }

    /// How many times plot `plot`'s building has been built again since year 0: a new
    /// household each time.
    pub fn generation(&self, plot: usize) -> u32 {
        match self.state(plot) {
            State::Standing { rebuilt, .. } => rebuilt,
            _ => 0,
        }
    }

    /// What settlement `k` can hold.
    pub fn capacity(&self, k: usize) -> Capacity {
        self.capacity.get(k).copied().unwrap_or(Capacity { homes: 0, empty: 0 })
    }

    /// How many hives each apiary of settlement `k` keeps now.
    pub fn hives(&self, k: usize) -> u8 {
        self.hives.get(k).copied().unwrap_or(HIVES)
    }

    /// Whether settlement `k` has been granted `boon` by now.
    pub fn granted(&self, k: usize, boon: Boon) -> bool {
        self.granted.iter().any(|&(s, b)| s as usize == k && b == boon)
    }

    /// Whether nothing differs from today's world: year 0, and no boon granted.
    pub fn is_today(&self) -> bool {
        self.lives.is_empty() && self.granted.is_empty()
    }

    /// Whether smoke rises from chimney `i` of `net` (see `Network::chimneys`): whether
    /// its home is lived in.
    pub fn smokes(&self, net: &Network, i: usize) -> bool {
        net.years.chimney_plot.get(i).is_none_or(|&p| self.standing(p as usize))
    }

    /// Stamp `s` as it stands now: as it was laid (borrowed), changed, or None where
    /// nothing of it stands. The same for every chunk that reads it, so chunk borders
    /// agree in every year.
    pub fn view<'a>(&self, s: &'a Stamp) -> Option<Cow<'a, Stamp>> {
        let Some(tag) = s.tag.as_deref() else { return Some(Cow::Borrowed(s)) };
        match tag {
            Tag::Boon { settlement, boon } => self.granted(*settlement as usize, *boon).then_some(Cow::Borrowed(s)),
            _ if self.lives.is_empty() => Some(Cow::Borrowed(s)),
            Tag::Building(b) => self.building(s, b),
            Tag::Yard { plot, what } => self.yard(s, *plot as usize, *what),
        }
    }

    /// Whether anything of stamp `s` stands now, as [`Self::view`] has it, told without
    /// making it.
    pub fn shows(&self, s: &Stamp) -> bool {
        match s.tag.as_deref() {
            None => true,
            Some(Tag::Boon { settlement, boon }) => self.granted(*settlement as usize, *boon),
            Some(_) if self.lives.is_empty() => true,
            Some(Tag::Building(b)) => self.state(b.plot as usize) != State::Cleared,
            Some(Tag::Yard { plot, what }) => match (self.state(*plot as usize), what) {
                (State::Cleared | State::Scaffold, Yard::Ground) => true,
                (State::Cleared | State::Scaffold, _) | (State::Abandoned { .. }, Yard::Hive(_)) => false,
                (State::Standing { .. }, Yard::Hive(n)) => *n < self.hives(self.settlement_of(*plot as usize)),
                _ => true,
            },
        }
    }

    /// If stamp `s` is a home going up again in scaffolding, the box its walls and the
    /// scaffolding about them stand in, continent-local: walls this new have had no time
    /// for ivy to climb them or moss to take on the floor laid open to the sky.
    pub fn rising(&self, s: &Stamp) -> Option<(IVec3, IVec3)> {
        let Some(Tag::Building(b)) = s.tag.as_deref() else { return None };
        if self.lives.is_empty() || self.state(b.plot as usize) != State::Scaffold {
            return None;
        }
        let (lo, hi) = (b.walls.0 - IVec2::ONE, b.walls.1 + IVec2::ONE);
        Some((IVec3::new(lo.x, s.lo.y, lo.y), IVec3::new(hi.x, s.hi.y, hi.y)))
    }

    fn building<'a>(&self, s: &'a Stamp, b: &Building) -> Option<Cow<'a, Stamp>> {
        let inside = |j: usize| b.inside.get(j / 64).is_some_and(|w| w >> (j % 64) & 1 != 0);
        let seed = hash2(self.seed ^ 0x0b5e, b.plot as i32, 7);
        match self.state(b.plot as usize) {
            State::Standing { thatched: None, .. } => Some(Cow::Borrowed(s)),
            State::Standing { thatched: Some(q), .. } => {
                let Some(base) = b.thatch else { return Some(Cow::Borrowed(s)) };
                // Thatched all over at once: the whole roof one colour.
                let new = shade(base, self.when.quarter() - q);
                let mut out = s.clone();
                for (j, (p, v)) in out.voxels.iter_mut().enumerate() {
                    if !inside(j) && p.y >= b.eaves && thatch(*v) {
                        *v = new;
                    }
                }
                Some(Cow::Owned(out))
            }
            State::Abandoned { .. } => {
                let holes = holes(b, seed);
                let mut out = s.clone();
                for (j, (p, v)) in out.voxels.iter_mut().enumerate() {
                    let roof = !inside(j) && p.y >= b.eaves && mat::ROOFING.contains(v);
                    *v = match *v {
                        _ if roof && holes.iter().any(|&(c, r)| (IVec2::new(p.x, p.z).as_vec2() + 0.5 - c).length_squared() < r * r) => AIR,
                        // The weather has the whole roof alike.
                        v if roof && thatch(v) => mat::THATCH_OLD,
                        mat::WINDOW_LIT => mat::WINDOW_DARK,
                        // The fire long out.
                        mat::EMBER => mat::CHARCOAL,
                        v => v,
                    };
                }
                // Nothing left hanging out, no light behind the panes, and the weeds
                // coming up about it.
                out.fine.retain(|f| f.flap == 0);
                for f in &mut out.fine {
                    f.voxel = match f.voxel {
                        mat::WINDOW_LIT => mat::WINDOW_DARK,
                        mat::EMBER => mat::CHARCOAL,
                        v => v,
                    };
                }
                out.fine.extend(weeds(s, b, seed));
                Some(Cow::Owned(out))
            }
            State::Cleared => None,
            State::Scaffold => Some(Cow::Owned(rising(s, b, inside))),
        }
    }

    fn yard<'a>(&self, s: &'a Stamp, plot: usize, what: Yard) -> Option<Cow<'a, Stamp>> {
        match (self.state(plot), what) {
            // Rebuilding: only what is in the ground stays, its greens gone over.
            (State::Cleared | State::Scaffold, Yard::Ground) => Some(Cow::Owned(Stamp { lo: s.lo, hi: s.hi, voxels: s.voxels.clone(), fine: Vec::new(), tag: None })),
            (State::Cleared | State::Scaffold, _) => None,
            // The bees gone with the household.
            (State::Abandoned { .. }, Yard::Hive(_)) => None,
            (State::Abandoned { .. }, Yard::Woodpile) => Some(stack(s, 1)),
            (State::Abandoned { .. }, Yard::Line) => Some(Cow::Owned(Stamp { lo: s.lo, hi: s.hi, voxels: s.voxels.clone(), fine: s.fine.iter().filter(|f| f.flap == 0).copied().collect(), tag: None })),
            (State::Standing { .. }, Yard::Hive(n)) => (n < self.hives(self.settlement_of(plot))).then_some(Cow::Borrowed(s)),
            // Stacked through the summer and autumn, burnt down through the winter and
            // spring.
            (State::Standing { .. }, Yard::Woodpile) => Some(stack(s, [1, 2, 3, 2][self.when.season as usize])),
            _ => Some(Cow::Borrowed(s)),
        }
    }

    /// The settlement plot `plot` belongs to, as its hives are counted.
    fn settlement_of(&self, plot: usize) -> usize {
        self.settlement.get(plot).copied().unwrap_or(0) as usize
    }
}

/// Whether a plot's building comes and goes with the years: the homes, and the
/// workplaces their households live over. What a settlement has only one of (its
/// tavern, smithy, mill or hall) stands through them all.
fn churns(p: Purpose) -> bool {
    matches!(p, Purpose::Home | Purpose::Workplace(_))
}

/// How many of a settlement's `homes` its water lets stand out of use at once: one
/// where there is fresh water near, more on wells or the sea alone and in a big place,
/// and only one again with a second well.
fn slack(s: &Settlement, homes: usize, well: bool) -> usize {
    let n = if well {
        1
    } else if s.water.fresh() {
        1 + homes / 10
    } else {
        2 + homes / 8
    };
    n.min(homes)
}

/// How many things change in a settlement in `year`: one in a hamlet or a village, one
/// or two in a large village, two in a town.
fn events(kind: Kind, seed: u32, k: usize, year: i32) -> usize {
    match kind {
        Kind::Hamlet | Kind::Village => 1,
        Kind::LargeVillage => 1 + (unit(hash3(seed ^ EVENT, k as i32, year, 99)) < 0.5) as usize,
        Kind::Town | Kind::GreatTown => 2,
    }
}

/// One home's years as far as the replay has gone.
#[derive(Clone, Copy, Debug)]
struct Rec {
    rebuilt: u32,
    /// The year what stands was finished, or is to be.
    built: i32,
    fell: Option<i32>,
    /// The season its roof was last thatched (see [`When::quarter`]).
    thatched: Option<i32>,
    /// The year it was last taken down and built again, if it has been.
    works: Option<i32>,
}

impl Rec {
    fn at(&self, when: When) -> Life {
        let state = match (self.works, self.fell) {
            (Some(y), _) if y == when.year && when.season == 0 => State::Cleared,
            (Some(y), _) if y == when.year && when.season == 1 => State::Scaffold,
            (_, Some(since)) => State::Abandoned { since },
            _ => State::Standing { rebuilt: self.rebuilt, thatched: self.thatched },
        };
        Life { built: self.built, fell: self.fell, state }
    }
}

/// Settlement `k`'s years replayed from the first to `year`: the record of each of its
/// homes, in the order of `thatch` (which says which have thatched roofs), and its
/// apiaries' hives at the end. `well` is the year a second well was sunk, if one was.
///
/// Each year a thing or two changes (see [`events`]), each to a different home: a roof
/// is thatched afresh; a home that has stood empty since before the year is taken down
/// in spring and built again (scaffolding through the summer, lived in from autumn); or,
/// while fewer stand empty than the water lets, one falls out of use; or, in a place
/// that keeps bees, a hive is lost or a swarm taken. The homes are taken in an order the
/// seed sets, so that each comes round in turn.
fn replay(seed: u32, k: usize, s: &Settlement, thatch: &[bool], apiary: bool, well: Option<i32>, year: i32) -> (Vec<Rec>, u8) {
    let n = thatch.len();
    let mut recs = vec![Rec { rebuilt: 0, built: 0, fell: None, thatched: None, works: None }; n];
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&i| hash3(seed ^ ORDER, k as i32, i as i32, 0));
    let (mut falls, mut thatches) = (0usize, 0usize);
    let mut hives = HIVES;
    // The next home in the order from `from` that `ok` takes; the turn goes on from it.
    let next = |from: &mut usize, ok: &dyn Fn(usize) -> bool| -> Option<usize> {
        let c = (0..n).map(|d| (*from + d) % n).find(|&c| ok(order[c]))?;
        *from = (c + 1) % n;
        Some(order[c])
    };
    for y in 1..=year {
        let sunk = well.is_some_and(|w| w <= y);
        let empty = slack(s, n, sunk);
        let mut touched = vec![false; n];
        // The hives change once in a year at most, so that two changes never undo each
        // other.
        let mut hived = false;
        for j in 0..events(s.kind, seed, k, y) {
            let apiary = apiary && !hived;
            let r = unit(hash3(seed ^ EVENT, k as i32, y, j as i32));
            let out = recs.iter().filter(|r| r.fell.is_some() || r.works == Some(y)).count();
            let (recs_now, touched_now) = (&recs, &touched);
            // The home that has stood empty longest, of those empty since before now.
            let ready = order.iter().copied().filter(|&i| !touched_now[i] && recs_now[i].fell.is_some_and(|f| f < y)).min_by_key(|&i| recs_now[i].fell);
            let standing = |i: usize| !touched_now[i] && recs_now[i].fell.is_none() && recs_now[i].works.is_none_or(|w| w < y);
            let thatchable = |i: usize| standing(i) && thatch[i] && recs_now[i].thatched.is_none_or(|q| y * 4 - q >= 8);
            // Once built again, a home stands a few years before it can fall empty.
            let fallable = |i: usize| standing(i) && (recs_now[i].rebuilt == 0 || y - recs_now[i].built >= 3);
            enum Change {
                Thatch(usize),
                Rebuild(usize),
                Fall(usize),
                Hive,
                Nothing,
            }
            let change = if apiary && r < 0.12 {
                Change::Hive
            } else if let Some(i) = (r < 0.4).then(|| next(&mut thatches, &thatchable)).flatten() {
                Change::Thatch(i)
            } else if let Some(i) = ready.filter(|_| sunk || out >= empty || r > 0.75) {
                Change::Rebuild(i)
            } else if let Some(i) = (out < empty).then(|| next(&mut falls, &fallable)).flatten() {
                Change::Fall(i)
            } else if let Some(i) = ready {
                // Whatever else can change, so that every year something does.
                Change::Rebuild(i)
            } else if let Some(i) = next(&mut thatches, &thatchable) {
                Change::Thatch(i)
            } else if let Some(i) = (out < empty).then(|| next(&mut falls, &standing)).flatten() {
                Change::Fall(i)
            } else if apiary {
                Change::Hive
            } else {
                Change::Nothing
            };
            match change {
                Change::Thatch(i) => {
                    recs[i].thatched = Some(y * 4);
                    touched[i] = true;
                }
                Change::Rebuild(i) => {
                    let rebuilt = recs[i].rebuilt + 1;
                    recs[i] = Rec { rebuilt, built: y, fell: None, thatched: thatch[i].then_some(y * 4 + 2), works: Some(y) };
                    touched[i] = true;
                }
                Change::Fall(i) => {
                    recs[i].fell = Some(y);
                    touched[i] = true;
                }
                Change::Hive => {
                    hived = true;
                    hives = match hives {
                        h if h <= FEWEST_HIVES => h + 1,
                        h if h >= HIVES => h - 1,
                        h if r < 0.06 => h - 1,
                        h => h + 1,
                    };
                }
                Change::Nothing => {}
            }
        }
    }
    (recs, hives)
}

/// Where an empty home's roof has fallen in: a hole or two, a few voxels across, at
/// places its seed chooses inside its walls, each going through the whole covering.
fn holes(b: &Building, seed: u32) -> Vec<(glam::Vec2, f32)> {
    let (lo, hi) = b.walls;
    let size = (hi - lo + IVec2::ONE).as_vec2();
    (0..1 + (seed % 2) as i32)
        .map(|i| {
            let h = hash2(seed, i, 0x401e);
            let at = lo.as_vec2() + size * glam::Vec2::new(0.25 + 0.5 * unit(h), 0.25 + 0.5 * unit(h.rotate_left(13)));
            (at, 1.2 + 1.3 * unit(h.rotate_left(7)) + size.min_element() * 0.08)
        })
        .collect()
}

/// Weeds come up about an empty home: tufts on about a third of the ground its stamp
/// covers outside its walls, on the level of its plot.
fn weeds(s: &Stamp, b: &Building, seed: u32) -> Vec<PlantBox> {
    let (lo, hi) = b.walls;
    let y = b.ground * CELLS;
    let mut out = Vec::new();
    for z in s.lo.z..=s.hi.z {
        for x in s.lo.x..=s.hi.x {
            let h = hash2(seed ^ 0x7eed, x, z);
            if (lo.x..=hi.x).contains(&x) && (lo.y..=hi.y).contains(&z) || unit(h) > 0.35 {
                continue;
            }
            let tall = 3 + (h >> 8) as i32 % 5;
            let (cx, cz) = (x * CELLS + 1 + (h >> 16) as i32 % 2, z * CELLS + 1 + (h >> 20) as i32 % 2);
            let voxel = if h & 1 == 0 { mat::TUFT_DRY } else { mat::TUFT_MEADOW };
            out.push(PlantBox { lo: [cx, y, cz], hi: [cx + 2, y + tall, cz + 2], voxel, sway_lo: false, sway_hi: true, thin: 0, season: 0, flap: 0 });
        }
    }
    out
}

/// The highest course of a building's walls laid while it rises in scaffolding: three
/// quarters of the way up them, never their top, and high enough that the windows show
/// as gaps in them and the doorway as a gap from the floor up.
fn rising_to(b: &Building) -> i32 {
    let h = b.plate - b.floor;
    b.floor + ((h * 3 + 2) / 4).min(h - 1).max(3)
}

/// Which side of a building's walls a column is on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    Wall,
    Within,
    Out,
}

/// Where a building's walls stand, over the columns inside its outer walls and one more
/// all round: each column one of its walls', within them, or out of doors, which is what
/// can be reached from the edge without crossing a wall.
struct Lie {
    lo: IVec2,
    w: i32,
    d: i32,
    of: Vec<Side>,
}

impl Lie {
    fn new(b: &Building) -> Lie {
        let (lo, hi) = (b.walls.0 - IVec2::ONE, b.walls.1 + IVec2::ONE);
        let (w, d) = ((hi.x - lo.x + 1).max(0), (hi.y - lo.y + 1).max(0));
        let inner = b.walls.1.x - b.walls.0.x + 1;
        let mut of = vec![Side::Within; (w * d) as usize];
        for z in 1..d - 1 {
            for x in 1..w - 1 {
                let i = ((z - 1) * inner + x - 1) as usize;
                if b.wall.get(i / 64).is_some_and(|m| m >> (i % 64) & 1 != 0) {
                    of[(z * w + x) as usize] = Side::Wall;
                }
            }
        }
        let mut todo: Vec<(i32, i32)> = (0..w).flat_map(|x| [(x, 0), (x, d - 1)]).chain((0..d).flat_map(|z| [(0, z), (w - 1, z)])).collect();
        while let Some((x, z)) = todo.pop() {
            if x < 0 || z < 0 || x >= w || z >= d || of[(z * w + x) as usize] != Side::Within {
                continue;
            }
            of[(z * w + x) as usize] = Side::Out;
            todo.extend([(x + 1, z), (x - 1, z), (x, z + 1), (x, z - 1)]);
        }
        Lie { lo, w, d, of }
    }

    fn at(&self, x: i32, z: i32) -> Side {
        let (x, z) = (x - self.lo.x, z - self.lo.y);
        if x < 0 || z < 0 || x >= self.w || z >= self.d { Side::Out } else { self.of[(z * self.w + x) as usize] }
    }

    /// Whether column `(x, z)` is beside one of the walls', on a side or at a corner.
    fn beside(&self, x: i32, z: i32) -> bool {
        (-1..=1).any(|dz| (-1..=1).any(|dx| self.at(x + dx, z + dz) == Side::Wall))
    }
}

/// A building rising again inside scaffolding, the summer after it was taken down: its
/// walls three quarters up in their own material, the doorway and the windows open in
/// them, and its floor laid; no roof yet, nothing inside and nothing hung. A wall of reeds
/// is its frame woven with wattle, the reeds to be tied on after. Out of doors, what
/// stands up from the ground (its steps, a yard's wall) and what is built onto the walls
/// (the crossed ends of a log wall's logs) are as high as the walls, and nothing of the
/// eaves is left in the air: no roof comes down so low beside its walls. `inside` says
/// which of the stamp's voxels are the furnishing's.
fn rising(s: &Stamp, b: &Building, inside: impl Fn(usize) -> bool) -> Stamp {
    let top = rising_to(b);
    let lie = Lie::new(b);
    // How high each column out of doors stands solid from the ground, as the voxels are
    // gone through from the lowest up.
    let sw = s.hi.x - s.lo.x + 1;
    let mut stands = vec![i32::MIN; (sw * (s.hi.z - s.lo.z + 1)).max(0) as usize];
    let mut voxels = Vec::with_capacity(s.voxels.len());
    for (j, &(p, v)) in s.voxels.iter().enumerate() {
        if inside(j) {
            continue;
        }
        let roofing = v != AIR && mat::ROOFING.contains(&v);
        let kept = match lie.at(p.x, p.z) {
            _ if v == AIR => Some(v),
            Side::Wall if p.y > top => None,
            // A wall's reeds, whose frame stands woven ready for them.
            Side::Wall if roofing => (p.y > b.floor).then_some(mat::WATTLE),
            Side::Wall => Some(v),
            Side::Within => (p.y <= b.floor && !roofing).then_some(v),
            Side::Out => {
                let c = ((p.z - s.lo.z) * sw + p.x - s.lo.x) as usize;
                let grounded = !roofing && (p.y <= b.ground || stands[c] == p.y - 1);
                if grounded {
                    stands[c] = p.y;
                }
                (!roofing && (p.y <= b.floor || p.y <= top && (grounded || lie.beside(p.x, p.z)))).then_some(v)
            }
        };
        if let Some(v) = kept {
            voxels.push((p, v));
        }
    }
    // The scaffolding stands a voxel out from the walls, which may be a voxel past what
    // the stamp covered (a flat roof with no eaves): the chunks a voxel beyond a stamp
    // read it too, so it is drawn whole.
    let (lo, hi) = (b.walls.0 - IVec2::ONE, b.walls.1 + IVec2::ONE);
    let lo = IVec3::new(s.lo.x.min(lo.x), s.lo.y, s.lo.z.min(lo.y));
    let hi = IVec3::new(s.hi.x.max(hi.x), s.hi.y, s.hi.z.max(hi.y));
    Stamp { lo, hi, voxels, fine: scaffold(s, b, top, &lie), tag: None }
}

/// The scaffolding about walls rising to `top`: poles a voxel out from the walls every
/// few voxels, standing past the walls to where the roof will be, and boards between
/// them to stand on every few voxels up. About a rectangle they stand in straight runs
/// with a pole at each corner; about a round hut, or one whose corners are cut or that
/// has a wing, they follow its walls round.
fn scaffold(s: &Stamp, b: &Building, top: i32, lie: &Lie) -> Vec<PlantBox> {
    let part = |lo: [i32; 3], hi: [i32; 3], voxel: Voxel| PlantBox { lo, hi, voxel, sway_lo: false, sway_hi: false, thin: 0, season: 0, flap: 0 };
    // Poles stand a good way past the walls: on a hut whose thatch comes down near the
    // ground the walls are low, and poles no taller than them would read as a fence.
    let tall = (b.plate + 3).min(s.hi.y).max(top + 2);
    let (y0, y1) = (b.ground * CELLS, tall * CELLS);
    let pole = |x: i32, z: i32| part([x * CELLS + 1, y0, z * CELLS + 1], [x * CELLS + 3, y1, z * CELLS + 3], mat::BARK_PINE);
    // And one up by the poles' tops, for those who will set the roof's timbers.
    let mut levels: Vec<i32> = (b.ground + 2..tall - 2).step_by(3).collect();
    if levels.last().is_none_or(|&l| tall - 1 > l + 2) {
        levels.push(tall - 1);
    }
    let (lo, hi) = b.walls;
    let walled = |x: i32, z: i32| lie.at(x, z) == Side::Wall;
    let boxed = (lo.x..=hi.x).all(|x| walled(x, lo.y) && walled(x, hi.y)) && (lo.y..=hi.y).all(|z| walled(lo.x, z) && walled(hi.x, z));
    let mut out = Vec::new();
    // A rectangle, or walls that do not close (which no home has).
    if boxed || !lie.of.contains(&Side::Within) {
        let (x0, x1, z0, z1) = (lo.x - 1, hi.x + 1, lo.y - 1, hi.y + 1);
        let along = |a: i32, b: i32| -> Vec<i32> {
            let mut v: Vec<i32> = (a..=b).step_by(3).collect();
            if v.last() != Some(&b) {
                v.push(b);
            }
            v
        };
        for x in along(x0, x1) {
            out.extend([pole(x, z0), pole(x, z1)]);
        }
        for z in along(z0, z1).into_iter().filter(|&z| z != z0 && z != z1) {
            out.extend([pole(x0, z), pole(x1, z)]);
        }
        for h in levels {
            let y = h * CELLS;
            for x in x0..=x1 {
                for z in [z0, z1] {
                    out.push(part([x * CELLS, y, z * CELLS + 1], [x * CELLS + CELLS, y + 1, z * CELLS + 3], mat::PLANK_PALE));
                }
            }
            for z in z0 + 1..z1 {
                for x in [x0, x1] {
                    out.push(part([x * CELLS + 1, y, z * CELLS], [x * CELLS + 3, y + 1, z * CELLS + CELLS], mat::PLANK_PALE));
                }
            }
        }
        return out;
    }
    // Round the walls: every column out of doors beside one of them, taken round in turn.
    let mut ring: Vec<IVec2> = Vec::new();
    for z in lo.y - 1..=hi.y + 1 {
        for x in lo.x - 1..=hi.x + 1 {
            if lie.at(x, z) == Side::Out && lie.beside(x, z) {
                ring.push(IVec2::new(x, z));
            }
        }
    }
    let mid = (lo + hi + IVec2::ONE).as_vec2() * 0.5;
    let angle = |c: &IVec2| {
        let v = c.as_vec2() + 0.5 - mid;
        v.y.atan2(v.x)
    };
    ring.sort_by(|a, b| angle(a).total_cmp(&angle(b)));
    let n = ring.len();
    let poles = ((n + 1) / 3).max(4).min(n);
    for i in 0..poles {
        let c = ring[i * n / poles];
        out.push(pole(c.x, c.y));
    }
    for h in levels {
        let y = h * CELLS;
        for c in &ring {
            out.push(part([c.x * CELLS, y, c.y * CELLS], [c.x * CELLS + CELLS, y + 1, c.y * CELLS + CELLS], mat::PLANK_PALE));
        }
    }
    out
}

/// A woodpile with `thirds` thirds of its courses left (3 for all of it): what is above
/// them burnt, or not yet cut, and its cap with them.
fn stack(s: &Stamp, thirds: i32) -> Cow<'_, Stamp> {
    let floor = s.lo.y;
    let full = s.voxels.iter().filter(|(_, v)| *v != AIR).map(|(p, _)| p.y - floor + 1).max().unwrap_or(0);
    let keep = ((full * thirds + 2) / 3).max(1);
    if keep >= full {
        return Cow::Borrowed(s);
    }
    let top = floor + keep;
    Cow::Owned(Stamp {
        lo: s.lo,
        hi: s.hi,
        voxels: s.voxels.iter().filter(|(p, _)| p.y < top).copied().collect(),
        fine: s.fine.iter().filter(|f| f.lo[1] < top * CELLS).copied().collect(),
        tag: None,
    })
}

#[cfg(test)]
mod tests;
