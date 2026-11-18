//! One settlement's people and the places of their days, made from the settlement as it
//! was laid out (`worldgen::settlements::layout`): a household for the beds of each home,
//! a bed for everyone, a place of work and a few haunts, and the lanes between them all.
//! Nothing here changes once it is made, and it is made the same every time.

use glam::{DVec3, IVec2, Vec2, Vec3};
use voxel::{Voxel, mesh::CELLS};
use worldgen::{
    continent::Continent,
    fauna,
    features::fittings::{self, Fitting},
    macro_layout::MACRO_SIZE,
    names::PlaceKind,
    noise::{hash2, unit},
    settlements::{
        Kind, Livelihood, Region, calendar,
        farmland::{self, Crop, rotation},
        layout::Purpose,
        special::Guild,
        years::{Boon, Lives},
    },
    surface::Sampler,
};

use super::{
    land::{self, Country, Keep},
    lanes::{Lanes, Walk, flat},
};
use crate::{
    animals::herd,
    character::Task,
    people::{self, Age, Person, Trade},
};

/// The longest way from a door, a well or the water's edge to a lane that is walked: a
/// plot set off the lanes has its own path, and nothing further is reached.
const SPUR: f32 = 60.0;

/// What a place of the settlement is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Where {
    /// Outside the door of a building, by its index in [`Town::plots`].
    Door(usize),
    Well,
    Stall(usize),
    Green,
    /// Under the village tree.
    Tree,
    Shrine,
    /// The water's edge.
    Water,
    // Out in the land (#64, #206): reached from the lanes by a way found across it (see
    // `land.rs`), and walked about in straight across.
    /// One of the settlement's fields, by its index in [`Town::fields`].
    Field(usize),
    /// The middle of the logging camp's yard.
    Camp,
    /// The quarry's yard, before its mouth.
    Quarry,
    /// Outside the gate of a fold its flock is kept in by night, by its index in
    /// [`Town::droves`].
    Pasture(usize),
}

impl Where {
    /// Whether it is out in the land, beyond the lanes.
    pub fn out(self) -> bool {
        matches!(self, Where::Field(_) | Where::Camp | Where::Quarry | Where::Pasture(_))
    }
}

/// A field worked by the settlement's farmers (#64): which it is, and where they stand
/// in it, each with the way they face, continent-local, feet level.
#[derive(Clone, Debug)]
pub struct Field {
    pub place: usize,
    pub plot: worldgen::settlements::farmland::Plot,
    /// What it carries the year the settlement was made for.
    pub crop: Crop,
    pub spots: Vec<(Vec3, Vec2)>,
}

/// Work out in the land at one place, at a camp or a quarry: where each thing is done,
/// for its workers to take in turn.
#[derive(Clone, Debug)]
pub struct Works {
    pub place: usize,
    pub spots: Vec<Spot>,
    /// Where they sit down to eat at midday.
    pub rest: Vec<Spot>,
    /// Where those who stay out there through the week sleep: under a camp's lean-to.
    pub beds: Vec<Spot>,
}

/// A place people go to: where they stand on reaching it, and the way to it from the
/// lanes.
#[derive(Clone, Debug)]
pub struct Place {
    pub what: Where,
    /// Where the way in ends, continent-local, feet level.
    pub at: Vec3,
    /// The lane node it is reached from, and the straight path from there to `at`.
    pub anchor: u32,
    pub spur: Vec<Vec3>,
    /// The way someone there faces: toward the well, the stall, the tree.
    pub face: Vec2,
}

/// A building of the settlement, as its people use it.
#[derive(Clone, Debug)]
pub struct Building {
    pub purpose: Purpose,
    /// Its door's place among [`Town::places`].
    pub place: usize,
    /// The feet in the doorway, and the way out of it.
    pub doorway: Vec3,
    pub out: Vec2,
    /// The columns of its walls and a column more all round, under the eaves, inclusive
    /// (its porch, steps and yard are walked through, so are left out); and its floor's
    /// level (the layer the feet are in), for telling whether someone is inside it.
    pub lo: IVec2,
    pub hi: IVec2,
    pub floor: i32,
    /// Whether its walls are round, within the ellipse that fills them.
    pub round: bool,
    pub beds: Vec<(Vec3, Vec2)>,
    pub seats: Vec<(Vec3, Vec2)>,
    pub yard: Option<(IVec2, IVec2)>,
    pub garden: Option<(IVec2, IVec2)>,
    /// What stands in its yard and garden: each fitting, its middle on the ground, its
    /// front, and half its size across the ground.
    pub things: Vec<(Fitting, Vec3, Vec2, Vec2)>,
}

impl Building {
    /// Whether feet at `p` (continent-local) are inside it, or under its eaves.
    pub fn holds(&self, p: Vec3) -> bool {
        self.within(Vec2::new(p.x, p.z), 1.0) && (p.y - self.floor as f32).abs() < 3.5
    }

    /// Whether ground point `p` (continent-local) is within its walls grown by `grow` all
    /// round (shrunk where `grow` is less than 0): within their rectangle, or for a round
    /// house within the ellipse in it, whose corners are open ground.
    pub fn within(&self, p: Vec2, grow: f32) -> bool {
        in_walls(self.lo, self.hi, self.round, p, grow)
    }
}

/// Somewhere to be, and what to do there.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Spot {
    /// The feet, continent-local; for someone in bed, the soles, with [`Spot::facing`]
    /// the way from the head to them.
    pub at: Vec3,
    /// A camera's yaw.
    pub facing: f32,
    pub task: Task,
    /// Indoors, in this building (by index in [`Town::plots`]): out of sight but from
    /// inside it.
    pub inside: Option<usize>,
    /// Sitting on a stool brought out, where there is no seat.
    pub stool: bool,
}

impl Spot {
    pub fn new(at: Vec3, toward: Vec2, task: Task) -> Self {
        Spot { at, facing: toward.y.atan2(toward.x), task, inside: None, stool: false }
    }
}

/// Where someone works, and at what.
#[derive(Clone, Copy, Debug)]
pub struct Job {
    pub place: usize,
    pub spot: Spot,
}

/// How someone's days go.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    /// Works at their trade, at home or at a special building.
    Worker,
    /// Keeps the house: water, sweeping, the garden.
    Homemaker,
    /// Keeps the tavern, which is their home.
    Innkeeper,
    /// Keeps a stall on the green.
    Seller,
    Child,
    Elder,
}

/// One of the settlement's people.
#[derive(Clone, Debug)]
pub struct Resident {
    /// Their number among the settlement's people as it is this year.
    pub n: usize,
    /// What they are known by in the save: their number in year 0, when everyone is as
    /// the settlement was laid out, or one of their own for a household come since (#207).
    pub id: usize,
    pub person: Person,
    /// Their home, by index in [`Town::plots`], which of its household they are, and
    /// their bed in it.
    pub home: usize,
    pub member: usize,
    pub bed: Spot,
    pub role: Role,
    /// Where they work, the first in the morning and the rest in turn through the day.
    pub jobs: Vec<Job>,
    /// Their stall, for a seller.
    pub stall: Option<usize>,
}

/// A market stall as it is dressed on market day (#207), from `calendar::Stall`: the
/// stall stands bare in the world, and its awning and goods are drawn by the game.
#[derive(Clone, Debug)]
pub struct Dress {
    /// The tops of its back and front rails, continent-local voxels: the back left corner,
    /// high, and the front right, low. None for a stall fallen into disrepair, which is
    /// left bare.
    pub awning: Option<(Vec3, Vec3)>,
    /// The way it faces, toward its customers.
    pub front: Vec2,
    /// Its counter's top, the low and the high corner, continent-local voxels.
    pub counter: (Vec3, Vec3),
    pub colour: Voxel,
    pub seed: u32,
    /// Who keeps it on market day, by their number among the residents.
    pub keeper: Option<usize>,
}

/// One settlement, with its people.
#[derive(Clone, Debug)]
pub struct Town {
    /// The continent it is on and its index there.
    pub cell: (i32, i32),
    pub k: usize,
    pub name: String,
    pub kind: Kind,
    /// The continent's corner in the world, in voxels: everything here is continent-local.
    pub origin: DVec3,
    /// The middle of its green, feet level, and how far it reaches.
    pub centre: Vec3,
    pub radius: f32,
    pub green: f32,
    pub lanes: Lanes,
    pub places: Vec<Place>,
    pub plots: Vec<Building>,
    pub residents: Vec<Resident>,
    /// How far it is to walk from each place to each other, in voxels.
    pub dist: Vec<Vec<f32>>,
    pub wells: Vec<usize>,
    pub stalls: Vec<usize>,
    /// Where each stall's keeper stands, behind it, facing its customers.
    pub behind: Vec<Spot>,
    pub green_place: usize,
    pub tavern: Option<usize>,
    pub tree: Option<usize>,
    pub shrine: Option<usize>,
    pub water: Option<usize>,
    /// What people say there of where they are (see talk.rs).
    pub near: Option<String>,
    pub river: Option<String>,
    // #207: its market days, its festivals and its boards.
    /// The settlement's seed and region: its market days, and what its stalls sell.
    pub seed: u32,
    pub region: Region,
    /// Each of `stalls` as it is dressed on market day, in the same order.
    pub market: Vec<Dress>,
    /// Where the village tree's ribbons are tied, and the spare places under its limbs a
    /// festival hangs its lanterns from (`Network::tree_knots`), continent-local voxels.
    pub knots: Vec<Vec3>,
    pub spare: Vec<Vec3>,
    /// Where a festival's lanterns stand along the lanes: the foot of each post, and the
    /// way its arm reaches out over the lane.
    pub posts: Vec<(Vec3, Vec2)>,
    /// The well heads: what each is, its middle on the ground, the way it faces and half
    /// its size across.
    pub well_heads: Vec<(Fitting, Vec3, Vec2, Vec2)>,
    /// The outside of the door of each building that stands this year: where its wall
    /// meets the open air, at the level of its floor, and the way out.
    pub doors: Vec<(Vec3, Vec2)>,
    /// The boards to read: the building each is in, the middle of its face and the way
    /// it faces.
    pub boards: Vec<(usize, Vec3, Vec2)>,
    /// Its guild house, if it has one, and the guilds that share it.
    pub guilds: Option<(usize, Vec<Guild>)>,
    /// The named places about it, nearest first, with what kind each is: what its
    /// people's notices speak of.
    pub about: Vec<(String, PlaceKind)>,
    /// The other settlements of its continent within [`NEIGHBOURS`], nearest first: what
    /// its notices send letters to, its merchants keep the market days of and its
    /// wayfarers chart.
    pub neighbours: Vec<Neighbour>,
    // Out in the land (#64, #206).
    /// How far from its middle the furthest of its places is, beyond the lanes as well.
    pub extent: f32,
    pub fields: Vec<Field>,
    pub camp: Option<Works>,
    pub quarry: Option<Works>,
    /// The flocks and herds driven out of their folds each morning and home at evening.
    pub droves: Vec<Drove>,
    /// The hens of its yards, fed each morning.
    pub hens: Vec<Hens>,
}

/// A yard's hens (#206), and who feeds them of a morning, where.
#[derive(Clone, Debug)]
pub struct Hens {
    /// Their pen's key (see `fauna::Kept`), which says when they are fed.
    pub key: u64,
    pub home: usize,
    pub spot: Spot,
    pub feeder: Option<usize>,
}

/// A pasture whose flock or herd is folded by night and driven out to graze by day
/// (#206), as the settlement's people know it: the herd's own day (`animals::herd`),
/// in continent-local voxels, and who drives it.
#[derive(Clone, Debug)]
pub struct Drove {
    pub place: usize,
    pub herd: crate::animals::herd::Drove,
    /// Who drives it, by their number among the residents.
    pub herder: Option<usize>,
}

/// How far about a settlement its neighbours are counted: the country a guild house's
/// wayfarers know (#207).
pub const NEIGHBOURS: f32 = worldgen::scale::m(4000.0);

/// Another settlement near one (#207).
#[derive(Clone, Debug)]
pub struct Neighbour {
    /// Its index in `Network::settlements`.
    pub k: usize,
    pub name: String,
    pub kind: Kind,
    pub seed: u32,
    /// Its middle, continent-local, and how far it is.
    pub at: Vec2,
    pub far: f32,
}

/// What a trade of the settlement is to the person working it.
pub fn trade_of(l: Livelihood) -> Trade {
    match l {
        Livelihood::Woodcutter | Livelihood::Sawmill | Livelihood::LoggingCamp => Trade::Woodcutter,
        Livelihood::Weaver | Livelihood::Fletcher => Trade::Weaver,
        Livelihood::Fisher | Livelihood::Port => Trade::Fisher,
        Livelihood::Hunter => Trade::Hunter,
        Livelihood::Apiary => Trade::Beekeeper,
        Livelihood::Quarry => Trade::Mason,
        Livelihood::Forager => Trade::Forager,
        Livelihood::Smithy => Trade::Smith,
    }
}

/// A stall facing `front` as the layout recorded it, in continent-local voxels.
fn dress_of(m: &calendar::Stall, front: Vec2) -> Dress {
    let voxels = |q: glam::IVec3| q.as_vec3() / CELLS as f32;
    Dress { awning: m.awning.map(|(a, b)| (voxels(a), voxels(b))), front, counter: (voxels(m.counter.0), voxels(m.counter.1)), colour: m.colour, seed: m.seed, keeper: None }
}

/// [`Building::within`], for a building whose walls and eaves cover `lo` to `hi`.
fn in_walls(lo: IVec2, hi: IVec2, round: bool, p: Vec2, grow: f32) -> bool {
    // From the first wall column's edge to the last's.
    let (lo, hi) = (lo.as_vec2() + 1.0, hi.as_vec2());
    let (mid, half) = ((lo + hi) * 0.5, (hi - lo) * 0.5 + grow);
    if half.min_element() <= 0.0 {
        return false;
    }
    let d = (p - mid).abs();
    if round { (d / half).length_squared() < 1.0 } else { d.cmplt(half).all() }
}

/// The first point out of plot `p`'s walls from `from` going `out`, across the ground, as
/// [`Building::within`] tells them.
fn outside(p: &worldgen::settlements::layout::Plot, from: Vec2, out: Vec2) -> Vec2 {
    let (lo, hi) = (p.walls.0 - IVec2::ONE, p.walls.1 + IVec2::ONE);
    let mut at = from;
    for _ in 0..24 {
        if !in_walls(lo, hi, p.round, at, 0.0) {
            break;
        }
        at += out * 0.25;
    }
    at
}

/// Where the village tree stands, for its haunt: the foot of its trunk where one was
/// planted (`Network::village_trees`), else the middle of the ground kept for it; and
/// the reach of that ground.
fn tree_foot(c: &Continent, k: usize) -> Option<(Vec2, f32)> {
    let (kept, r) = c.trails.layouts.get(k).and_then(|l| l.tree)?;
    let trunk = c.trails.village_trees.get(k).copied().flatten().map(|t| Vec2::new(t.x as f32 + 0.5, t.z as f32 + 0.5));
    Some((trunk.unwrap_or(kept), r))
}

/// What a settlement works out in the land, its places among the rest.
struct OutInLand {
    fields: Vec<Field>,
    camp: Option<Works>,
    quarry: Option<Works>,
    droves: Vec<Drove>,
}

/// The longest walk from a herder's home to the fold, in hours: they must be there as
/// its gate is opened, up no earlier than a quarter to five.
const HERDER_WALK: f32 = 2.4;

/// The most who go out to the logging camp and the quarry each day.
const CAMP_HANDS: usize = 4;
const QUARRY_HANDS: usize = 3;

/// How many have gone out to each of a settlement's works in the land so far, as its
/// people are made.
#[derive(Default)]
struct Hands {
    fields: Vec<usize>,
    camp: usize,
    quarry: usize,
}

/// The most fields a settlement's farmers work at once, by its size.
fn fields_worked(kind: Kind) -> usize {
    match kind {
        Kind::Hamlet => 2,
        Kind::Village => 3,
        Kind::LargeVillage | Kind::Town => 4,
        Kind::GreatTown => 5,
    }
}

/// Settlement `k`'s work out in the land (#64, #206): a few of its fields near, its
/// logging camp and its quarry, and the folds of its pastures; each a place reached
/// from the lane nearest by a way found across the land (`land.rs`), added to `places`.
/// `walls` and `things` are its buildings' and its yards' things, kept clear of.
#[allow(clippy::too_many_arguments)]
fn out_in_land(
    c: &Continent,
    seed: u32,
    cell: (i32, i32),
    k: usize,
    lives: &Lives,
    lanes: &Lanes,
    reached: &[bool],
    walls: &[(IVec2, IVec2, bool)],
    things: &[(Fitting, Vec3, Vec2, Vec2)],
    places: &mut Vec<Place>,
) -> OutInLand {
    let started = std::time::Instant::now();
    let net = &c.trails;
    let s = &net.settlements[k];
    let origin = (cell.0 * MACRO_SIZE, cell.1 * MACRO_SIZE);
    let centre = s.at();
    let camp = net.years.camps.iter().find(|q| q.owner == k);
    let quarry = c.land.quarries.iter().find(|q| q.owner == k);
    let herds: Vec<herd::Drove> = fauna::kept_about(c, seed, k)
        .iter()
        .filter_map(|p| match herd::tending(p) {
            Some(herd::Tend::Drove(d)) => Some(d),
            _ => None,
        })
        .collect();
    let local = |p: glam::DVec2| Vec2::new((p.x - origin.0 as f64) as f32, (p.y - origin.1 as f64) as f32);

    // What the ways keep clear of: the buildings and what stands about them, the barns
    // out in the fields, the camp's works, the quarry's shed and pit, and the folds.
    let mut keep: Vec<Keep> = walls.iter().map(|&(lo, hi, round)| Keep { lo, hi, round }).collect();
    keep.extend(things.iter().map(|t| Keep::rect((flat(t.1) - t.3).floor().as_ivec2(), ((flat(t.1) + t.3).ceil() - 1.0).as_ivec2())));
    keep.extend(c.land.steads.iter().filter(|b| b.owner == k).map(|b| Keep::rect(b.footprint.0, b.footprint.1)));
    if let Some(q) = camp {
        keep.extend(land::camp_keep(q));
    }
    if let Some(q) = quarry {
        keep.push(Keep::rect(q.walls.0 - IVec2::ONE, q.walls.1 + IVec2::ONE));
        keep.push(Keep::rect(q.pit.0, q.pit.1));
    }
    for d in &herds {
        let (lo, hi) = d.fold.bounds();
        keep.push(Keep::rect(local(lo).floor().as_ivec2() - IVec2::ONE, local(hi).floor().as_ivec2() + IVec2::ONE));
    }
    let within = land::Within::new(centre, s.radius + 4.0, &net.layouts[k].lanes);
    let mut country = Country::new(c, seed, lives.when, origin, keep, Some(within));

    // The way to `to` from whichever lane node near it it is shortest from (the nearest
    // may be shut in by the houses or the hedges), as a place's path in.
    let way_to = |country: &mut Country, places: &mut Vec<Place>, what: Where, to: Vec3, face: Vec2| -> Option<usize> {
        let far = |i: usize| flat(lanes.nodes[i]).distance(flat(to));
        let nearest = (0..lanes.nodes.len()).filter(|&i| reached[i]).map(far).fold(f32::INFINITY, f32::min);
        // Only from those out towards its edge: from further in, the way out is along
        // the lanes anyway.
        let outer = |i: usize| flat(lanes.nodes[i]).distance(centre) >= s.radius * 0.5;
        let from: Vec<usize> = (0..lanes.nodes.len()).filter(|&i| reached[i] && far(i) <= nearest + worldgen::scale::m(60.0) && (outer(i) || far(i) <= nearest)).collect();
        let points: Vec<Vec2> = from.iter().map(|&i| flat(lanes.nodes[i])).collect();
        let (i, mut spur) = country.way(&points, flat(to))?;
        let anchor = from[i];
        spur.remove(0);
        match spur.last_mut() {
            Some(last) => *last = to,
            None => spur.push(to),
        }
        places.push(Place { what, at: to, anchor: anchor as u32, spur, face });
        Some(places.len() - 1)
    };

    // The fields nearest, ring by ring out from the houses: each well inside one of the
    // settlement's own, sown or planted or cut for hay, with room in it for a few to work.
    let mut fields: Vec<Field> = Vec::new();
    let turn0 = unit(hash2(s.seed ^ 0xf1e1, k as i32, 0)) * std::f32::consts::TAU;
    let want = fields_worked(s.kind);
    'rings: for ring in 0..8 {
        let r = s.radius + worldgen::scale::m(10.0) + ring as f32 * worldgen::scale::m(9.0);
        let n = 16 + ring * 2;
        for i in 0..n {
            if fields.len() >= want {
                break 'rings;
            }
            let p = centre + Vec2::from_angle(turn0 + i as f32 * std::f32::consts::TAU / n as f32) * r;
            let field_at = |q: Vec2| farmland::plot(c, seed, q.x.floor() as i32, q.y.floor() as i32);
            let Some(plot) = field_at(p) else { continue };
            let crop = rotation::crop(&plot, seed, lives.when);
            if plot.owner != k || fields.iter().any(|f| f.plot.id == plot.id) || !(crop.tilled() || crop.trees() || crop == Crop::Hay) || !country.open(p) {
                continue;
            }
            let y = country.feet(p);
            let (rows, across) = (plot.rows, plot.rows.perp());
            let mut spots = Vec::new();
            for (u, v) in [(-2.5, -1.0), (2.5, 1.0), (-5.5, 1.2), (5.5, -1.2), (0.0, 2.6), (0.0, -2.6)] {
                let q = p + rows * worldgen::scale::m(u) + across * worldgen::scale::m(v);
                let here = country.feet(q);
                if field_at(q).is_some_and(|f| f.id == plot.id) && country.open(q) && (here - y).abs() <= 2.0 {
                    spots.push((Vec3::new(q.x, here, q.y), if u < 0.0 { rows } else { -rows }));
                }
            }
            if spots.len() < 2 {
                continue;
            }
            let Some(place) = way_to(&mut country, places, Where::Field(fields.len()), Vec3::new(p.x, y, p.y), rows) else { continue };
            fields.push(Field { place, plot, crop, spots });
        }
    }

    // The logging camp: the middle of its yard, between the lean-to and the saw pit.
    let camp = camp.and_then(|q| {
        let pt = |x: f32, z: f32| land::in_camp(q, x, z);
        let dir = |x: f32, z: f32| (pt(x, z) - pt(0.0, 0.0)).normalize_or(Vec2::X);
        let y = q.at.y as f32;
        let at = |x: f32, z: f32, up: f32| {
            let p = pt(x, z);
            Vec3::new(p.x, y + up, p.y)
        };
        let place = way_to(&mut country, places, Where::Camp, at(-3.0, 2.0, 0.0), dir(1.0, 0.0))?;
        let spots = vec![
            // At the chopping block, on the log over the saw pit, and at the logs waiting
            // to be sawn.
            Spot::new(at(-0.4, -2.0, 0.0), dir(-1.0, 0.0), Task::Chop),
            Spot::new(at(6.0, 1.0, 3.0), dir(-1.0, 0.0), Task::Saw),
            Spot::new(at(7.0, 3.6, 0.0), dir(0.0, 1.0), Task::Carry),
        ];
        let rest = vec![Spot::new(at(-5.0, -1.0, 0.0), dir(1.0, 0.2), Task::SitLow), Spot::new(at(-5.0, 1.4, 0.0), dir(1.0, -0.2), Task::SitLow)];
        // Under the lean-to, heads to its back, for those who stay out at the camp
        // through the week.
        let beds = [-4.0, -1.5, 1.0, 3.5].map(|z| Spot { inside: None, ..Spot::new(at(-12.0, z, 0.0), dir(1.0, 0.0), Task::Lie) }).to_vec();
        Some(Works { place, spots, rest, beds })
    });

    // The quarry: its yard before the mouth, the face at the back of the pit, and the
    // mason's shed.
    let quarry = quarry.and_then(|q| {
        let yard = (q.yard.0.as_vec2() + q.yard.1.as_vec2() + 1.0) * 0.5;
        let floor = Vec2::new(q.floor.x as f32 + 0.5, q.floor.z as f32 + 0.5);
        let y = q.floor.y as f32;
        let out = q.mouth.as_vec2().normalize_or((yard - floor).normalize_or(Vec2::X));
        let place = way_to(&mut country, places, Where::Quarry, Vec3::new(yard.x, y, yard.y), -out)?;
        let door = Vec2::new(q.door.x as f32 + 0.5, q.door.z as f32 + 0.5);
        let from_door = Vec2::new(q.facing.x as f32, q.facing.z as f32).normalize_or(Vec2::X);
        let shed = door + from_door * 2.5 + from_door.perp() * 1.6;
        let v = |p: Vec2, y: f32| Vec3::new(p.x, y, p.y);
        let spots = vec![
            Spot::new(v(floor - out * 2.0, y), -out, Task::Hammer),
            Spot::new(v(floor + out * 2.5 + out.perp() * 1.5, y), out, Task::Stone),
            Spot::new(v(shed, q.door.y as f32 + 1.0), -from_door, Task::Hammer),
        ];
        let side = out.perp() * 3.0;
        let rest = vec![Spot::new(v(yard + side, y), -out, Task::SitLow), Spot::new(v(yard - side, y), -out, Task::SitLow)];
        let beds = Vec::new();
        Some(Works { place, spots, rest, beds })
    });

    // The folds: their herders come and go by the gate.
    let mut droves = Vec::new();
    for d in herds {
        let (gate, out) = (local(d.gate), d.out.as_vec2());
        let p = gate + out * 2.0 + out.perp() * 1.4;
        let to = Vec3::new(p.x, country.feet(p), p.y);
        if let Some(place) = way_to(&mut country, places, Where::Pasture(droves.len()), to, -out.perp()) {
            droves.push(Drove { place, herd: d, herder: None });
        }
    }
    log::debug!(
        target: "game::villagers",
        "{}'s ways out to {} fields, {} camp, {} quarry and {} folds found in {:.0} ms, {} columns looked at",
        net.names.settlement(k).map_or("?", |p| p.name.as_str()),
        fields.len(),
        camp.iter().count(),
        quarry.iter().count(),
        droves.len(),
        started.elapsed().as_secs_f32() * 1000.0,
        country.looked
    );
    OutInLand { fields, camp, quarry, droves }
}

impl Town {
    /// Settlement `k` of continent `c` (whose cell is `cell`), with its people, for a world
    /// of `world_seed`.
    #[cfg(test)]
    pub fn build(c: &Continent, world_seed: u32, cell: (i32, i32), k: usize) -> Option<Town> {
        Self::build_in(c, world_seed, cell, k, &Lives::today(&c.trails))
    }

    /// [`Self::build`] as the settlement stands in a year (#207), `lives` saying which of
    /// its homes are lived in then and by which household.
    pub fn build_in(c: &Continent, world_seed: u32, cell: (i32, i32), k: usize, lives: &Lives) -> Option<Town> {
        let net = &c.trails;
        let s = net.settlements.get(k)?;
        let layout = net.layouts.get(k)?;
        let names = &net.names;
        let name = names.settlement(k).map_or_else(|| "this place".to_string(), |p| p.name.clone());
        let tongue = names.tongue_at(s.at());
        let reach = s.radius + 40.0;
        let lo = s.at() - reach;
        let sam = Sampler::new(c, world_seed, lo.x.floor() as i32, lo.y.floor() as i32, (2.0 * reach).ceil() as i32 + 2);
        let height = |p: Vec2| {
            let col = sam.column(p.x.floor() as i32, p.y.floor() as i32);
            col.water_surface().unwrap_or(col.top() + 1).max(col.top() + 1) as f32
        };
        let centre = Vec3::new(s.x as f32 + 0.5, s.y as f32, s.z as f32 + 0.5);
        let green = s.kind.green() * 2.0;
        let lanes = Lanes::new(&layout.lanes, centre, green, &mut |p| height(p));
        let reached = lanes.reach(0);
        let mut places: Vec<Place> = Vec::new();
        // The walls of every building, which a path in keeps clear of.
        let walls: Vec<(IVec2, IVec2, bool)> = net.plots[layout.plots.clone()].iter().map(|p| (p.walls.0 - IVec2::ONE, p.walls.1 + IVec2::ONE, p.round)).collect();
        let clear = |a: Vec2, b: Vec2| {
            let n = ((a.distance(b) / 0.5).ceil() as usize).max(1);
            (1..n).all(|k| !walls.iter().any(|&(lo, hi, round)| in_walls(lo, hi, round, a.lerp(b, k as f32 / n as f32), 0.0)))
        };
        // A path in from the nearest lane it can come straight from without going through a
        // house (from the nearest of all where none can); on the green, from its middle.
        let place = |what: Where, at: Vec3, places: &mut Vec<Place>| -> Option<usize> {
            let on_green = flat(at).distance(flat(centre)) < green * 0.5;
            let (anchor, d) = if on_green { (0, flat(at).distance(flat(centre))) } else { lanes.nearest_where(at, &reached, SPUR, |n| clear(flat(n), flat(at)))? };
            if d > SPUR {
                return None;
            }
            let from = lanes.nodes[anchor as usize];
            let steps = (d / 2.0).ceil().max(1.0) as usize;
            let spur = (1..=steps)
                .map(|i| {
                    let p = from.lerp(at, i as f32 / steps as f32);
                    // The ground along it, but at the ends their own level.
                    let y = if i == steps { at.y } else { height(flat(p)).max(from.y.min(at.y) - 2.0) };
                    Vec3::new(p.x, y, p.z)
                })
                .collect();
            let face = (flat(at) - flat(from)).normalize_or(Vec2::X);
            places.push(Place { what, at, anchor, spur, face });
            Some(places.len() - 1)
        };
        let green_place = place(Where::Green, centre, &mut places)?;

        // The buildings: each one's door, and what is about it.
        let plots = &net.plots[layout.plots.clone()];
        let things: Vec<(Fitting, Vec3, Vec2, Vec2)> = layout
            .things
            .iter()
            .map(|&(f, at, turns)| {
                let (a, b) = f.room(at, turns);
                let mid = (a + b).as_vec3() * 0.5;
                (f, Vec3::new(mid.x, at.y as f32, mid.z), fittings::front(turns).as_vec2(), Vec2::new((b.x - a.x) as f32, (b.z - a.z) as f32) * 0.5)
            })
            .collect();
        let mut buildings: Vec<Building> = Vec::new();
        let mut door_of: Vec<Option<usize>> = Vec::new();
        for p in plots {
            let out = p.facing.as_vec2().normalize_or(Vec2::X);
            let doorway = Vec3::new(p.door.x as f32 + 0.5, p.door.y as f32 + 1.0, p.door.z as f32 + 0.5);
            // Two paces out from the outside of its walls: a thick wall's doorway, or a
            // round one's, is a step or two in from where the wall meets the open air.
            let face = outside(p, flat(doorway), out);
            let porch = Vec3::new(face.x, 0.0, face.y) + Vec3::new(out.x, 0.0, out.y) * 2.0;
            let porch = Vec3::new(porch.x, p.origin.y as f32, porch.z);
            let at = place(Where::Door(buildings.len()), porch, &mut places);
            if let Some(i) = at {
                places[i].face = -out;
            }
            door_of.push(at);
            let mine: Vec<(Fitting, Vec3, Vec2, Vec2)> =
                things.iter().filter(|t| p.pads.iter().any(|&(c, r)| c.distance(flat(t.1)) < r) && !matches!(t.0, Fitting::Hurdle)).copied().collect();
            buildings.push(Building {
                purpose: p.purpose,
                place: at.unwrap_or(green_place),
                doorway,
                out,
                lo: p.walls.0 - IVec2::ONE,
                hi: p.walls.1 + IVec2::ONE,
                floor: p.door.y + 1,
                round: p.round,
                beds: p.beds.iter().map(|&(h, d)| (h, d.as_vec2())).collect(),
                seats: p.seats.iter().map(|&(q, d)| (Vec3::new(q.x as f32 + 0.5, q.y as f32, q.z as f32 + 0.5), d.as_vec2())).collect(),
                yard: p.yard,
                garden: p.garden,
                things: mine,
            });
        }

        // The water point, the stalls, the tree, the shrine and the water's edge.
        let mut wells = Vec::new();
        let mut stalls = Vec::new();
        let mut behind = Vec::new();
        let mut market = Vec::new();
        let mut well_heads = Vec::new();
        // The stalls on the green are the layout's, in its order (#207).
        let mut nth_stall = 0;
        for &(f, mid, front, half) in &things {
            let ground = |p: Vec3| Vec3::new(p.x, height(flat(p)), p.z);
            match f {
                Fitting::StoneWell | Fitting::Shaduf | Fitting::OasisWell => {
                    well_heads.push((f, mid, front, half));
                    let at = mid + Vec3::new(front.x, 0.0, front.y) * (half.x.max(half.y) + 1.2);
                    if let Some(i) = place(Where::Well, Vec3::new(at.x, mid.y, at.z), &mut places) {
                        places[i].face = -front;
                        wells.push(i);
                    }
                }
                Fitting::MarketStall => {
                    let dress = layout.market.get(nth_stall).map(|m| dress_of(m, front));
                    nth_stall += 1;
                    let deep = half.x.min(half.y);
                    let at = mid + Vec3::new(front.x, 0.0, front.y) * (deep + 1.6);
                    if let Some(i) = place(Where::Stall(stalls.len()), Vec3::new(at.x, mid.y, at.z), &mut places) {
                        places[i].face = -front;
                        stalls.push(i);
                        let back = mid - Vec3::new(front.x, 0.0, front.y) * (deep * 0.35);
                        behind.push(Spot::new(ground(back).with_y(mid.y), front, Task::Talk));
                        market.push(dress.unwrap_or(Dress { awning: None, front, counter: (mid, mid), colour: worldgen::materials::CLOTH_PALE, seed: 0, keeper: None }));
                    }
                }
                _ => {}
            }
        }
        // A second well, once the player has had one sunk (#207), fetched from as the
        // first is.
        if lives.granted(k, Boon::SecondWell)
            && let Some(site) = net.years.sites.iter().find(|w| w.settlement as usize == k && w.boon == Boon::SecondWell)
        {
            let well = Vec2::new(site.at.x as f32 + 1.0, site.at.z as f32 + 1.0);
            let toward = (flat(centre) - well).normalize_or(Vec2::X);
            let at = well + toward * 2.5;
            if let Some(i) = place(Where::Well, Vec3::new(at.x, site.at.y as f32, at.y), &mut places) {
                places[i].face = -toward;
                wells.push(i);
            }
        }
        let tree = tree_foot(c, k).and_then(|(at, r)| {
            let toward = (flat(centre) - at).normalize_or(Vec2::X);
            let p = at + toward * (r * 0.75);
            let i = place(Where::Tree, Vec3::new(p.x, height(p), p.y), &mut places)?;
            places[i].face = -toward;
            Some(i)
        });
        let shrine = s.shrine.and_then(|i| net.sites.get(i)).and_then(|site| {
            let at = Vec2::new(site.x as f32 + 0.5, site.z as f32 + 0.5);
            let toward = (flat(centre) - at).normalize_or(Vec2::X);
            let p = at + toward * 6.0;
            let i = place(Where::Shrine, Vec3::new(p.x, height(p), p.y), &mut places)?;
            places[i].face = -toward;
            Some(i)
        });
        let water = layout.water.and_then(|w| {
            // The last dry ground on the way from the green to the water: the first found
            // going back from the water toward the green, a step at a time.
            let from = flat(centre);
            let steps = w.distance(from).ceil() as usize;
            let p = (0..=steps).map(|i| w.lerp(from, i as f32 / steps.max(1) as f32)).find(|p| {
                let col = sam.column(p.x.floor() as i32, p.y.floor() as i32);
                col.water_surface().is_none() && col.river > 0.0
            })?;
            let i = place(Where::Water, Vec3::new(p.x, height(p), p.y), &mut places)?;
            places[i].face = (w - p).normalize_or(Vec2::X);
            Some(i)
        });

        // Out in the land (#64, #206): its fields, its logging camp and its quarry, and
        // the folds of its flocks, each reached by a way found across the land once.
        let out = out_in_land(c, world_seed, cell, k, lives, &lanes, &reached, &walls, &things, &mut places);

        // How far it is from every place to every other.
        let mut dist = vec![vec![0.0f32; places.len()]; places.len()];
        let spur_len = |p: &Place| Walk::new(std::iter::once(lanes.nodes[p.anchor as usize]).chain(p.spur.iter().copied()).collect()).length();
        let spurs: Vec<f32> = places.iter().map(spur_len).collect();
        let mut from_anchor: Vec<(u32, Vec<f32>)> = Vec::new();
        for (i, p) in places.iter().enumerate() {
            let d = match from_anchor.iter().find(|(a, _)| *a == p.anchor) {
                Some((_, d)) => d.clone(),
                None => {
                    let d = lanes.distances(p.anchor);
                    from_anchor.push((p.anchor, d.clone()));
                    d
                }
            };
            for (j, q) in places.iter().enumerate() {
                if i != j {
                    dist[i][j] = spurs[i] + d[q.anchor as usize] + spurs[j];
                }
            }
        }

        let mut town = Town {
            cell,
            k,
            name,
            kind: s.kind,
            origin: DVec3::new((cell.0 * MACRO_SIZE) as f64, 0.0, (cell.1 * MACRO_SIZE) as f64),
            centre,
            radius: s.radius,
            green,
            lanes,
            places,
            plots: buildings,
            residents: Vec::new(),
            dist,
            wells,
            stalls,
            behind,
            green_place,
            tavern: None,
            tree,
            shrine,
            water,
            near: None,
            river: None,
            seed: s.seed,
            region: s.region,
            market,
            knots: Vec::new(),
            spare: Vec::new(),
            posts: Vec::new(),
            well_heads,
            doors: Vec::new(),
            boards: Vec::new(),
            guilds: None,
            about: Vec::new(),
            neighbours: Vec::new(),
            extent: 0.0,
            fields: out.fields,
            camp: out.camp,
            quarry: out.quarry,
            droves: out.droves,
            hens: Vec::new(),
        };
        town.extent = town.places.iter().map(|p| flat(p.at).distance(flat(centre))).fold(town.radius, f32::max);
        town.tavern = (0..town.plots.len()).find(|&i| town.plots[i].purpose == Purpose::Tavern && door_of[i].is_some());
        town.dressing(c, k, lives, &things, &mut |p| height(p));
        town.near = net
            .settlements
            .iter()
            .enumerate()
            .filter(|&(j, _)| j != k)
            .min_by(|a, b| a.1.at().distance(s.at()).total_cmp(&b.1.at().distance(s.at())))
            .and_then(|(j, _)| names.settlement(j))
            .map(|p| p.name.clone());
        town.river = names
            .places
            .iter()
            .filter(|p| matches!(p.kind, PlaceKind::River | PlaceKind::Lake))
            .map(|p| (p, p.distance(s.at())))
            .filter(|(_, d)| *d < s.radius + 160.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(p, _)| p.name.clone());
        town.people(c, tongue, s, &door_of, lives);
        town.keepers();
        town.feeders(c, world_seed);
        Some(town)
    }

    /// Who feeds the hens of each yard (#206): whoever keeps the house, an elder or a
    /// child of it, or failing them one whose work is about it.
    fn feeders(&mut self, c: &Continent, world_seed: u32) {
        let corner = Vec2::new(self.origin.x as f32, self.origin.z as f32);
        let v2 = |v: Vec2| glam::DVec2::new(v.x as f64, v.y as f64);
        for p in fauna::kept_about(c, world_seed, self.k) {
            let Some(herd::Tend::Fed(key, _)) = herd::tending(&p) else { continue };
            let home = (key & 0xfff) as usize;
            let Some(b) = self.plots.get(home) else { continue };
            let (at, face, _) = herd::feeder(v2(p.middle), v2(p.half), v2(p.gate_toward));
            let at = Vec2::new(at.x as f32, at.y as f32) - corner;
            if self.walled(at) {
                continue;
            }
            let rank = |r: &Resident| match r.role {
                Role::Homemaker => 0,
                Role::Elder => 1,
                Role::Child => 2,
                Role::Worker if !self.works_out(r) => 3,
                _ => 9,
            };
            let feeder = self.residents.iter().filter(|r| r.home == home && rank(r) < 9).min_by_key(|r| (rank(r), r.member)).map(|r| r.n);
            let y = self.places[b.place].at.y;
            self.hens.push(Hens { key, home, spot: Spot::new(Vec3::new(at.x, y, at.y), face.as_vec2(), Task::Feed), feeder });
        }
    }

    /// What its festivals and boards need of it (#207): its tree's knots, posts for
    /// lanterns along its lanes, where its boards are and its guilds, and the named places
    /// about it. `things` is every fitting set out about it, as `build` reads them.
    fn dressing(&mut self, c: &Continent, k: usize, lives: &Lives, things: &[(Fitting, Vec3, Vec2, Vec2)], height: &mut dyn FnMut(Vec2) -> f32) {
        let net = &c.trails;
        let layout = &net.layouts[k];
        let voxels = |q: &glam::IVec3| q.as_vec3() / CELLS as f32;
        if let Some((knots, spare)) = net.tree_knots.get(k) {
            self.knots = knots.iter().map(voxels).collect();
            self.spare = spare.iter().map(voxels).collect();
        }
        for (i, p) in layout.plots.clone().zip(&net.plots[layout.plots.clone()]) {
            // S8 (#207): no garland on the door of a home empty or not yet built again.
            if !lives.standing(i) {
                continue;
            }
            let out = p.facing.as_vec2().normalize_or(Vec2::X);
            let face = outside(p, Vec2::new(p.door.x as f32 + 0.5, p.door.z as f32 + 0.5), out);
            self.doors.push((Vec3::new(face.x, p.door.y as f32 + 1.0, face.y), out));
        }
        let first = layout.plots.start;
        for b in net.boards.iter().filter(|b| layout.plots.contains(&b.plot)) {
            self.boards.push((b.plot - first, b.at, Vec2::new(b.facing.x as f32, b.facing.z as f32)));
        }
        self.guilds = net.guild_halls.iter().find(|g| layout.plots.contains(&g.0)).map(|g| (g.0 - first, g.1.clone()));
        let at = flat(self.centre);
        let mut about: Vec<(f32, String, PlaceKind)> = net
            .names
            .places
            .iter()
            .filter(|p| !matches!(p.kind, PlaceKind::Region | PlaceKind::Range | PlaceKind::Rift | PlaceKind::Fault | PlaceKind::Island) && p.settlement != Some(k))
            .map(|p| (p.distance(at), p.name.clone(), p.kind))
            .filter(|p| p.0 < worldgen::scale::m(2500.0))
            .collect();
        about.sort_by(|a, b| a.0.total_cmp(&b.0));
        self.about = about.into_iter().take(12).map(|p| (p.1, p.2)).collect();
        for (j, s) in net.settlements.iter().enumerate() {
            let far = s.at().distance(at);
            if j != k && far < NEIGHBOURS
                && let Some(name) = net.names.settlement(j)
            {
                self.neighbours.push(Neighbour { k: j, name: name.name.clone(), kind: s.kind, seed: s.seed, at: s.at(), far });
            }
        }
        self.neighbours.sort_by(|a, b| a.far.total_cmp(&b.far));

        // A lantern on a post every ten metres or so along each lane, either side in
        // turn, from the edge of the green to the edge of the settlement; clear of the
        // buildings, the yard things and the lanes themselves.
        const EVERY: f32 = worldgen::scale::m(10.0);
        const MOST: usize = 28;
        let ways: Vec<Vec2> = layout.lanes.iter().flatten().copied().collect();
        for (n, lane) in layout.lanes.iter().enumerate() {
            let mut side = if n % 2 == 0 { 1.0 } else { -1.0 };
            let mut since = EVERY * 0.5;
            for i in 1..lane.len().saturating_sub(1) {
                since += lane[i].distance(lane[i - 1]);
                let from_middle = lane[i].distance(at);
                if since < EVERY || from_middle < self.green * 0.5 + 4.0 || from_middle > self.radius || self.posts.len() >= MOST {
                    continue;
                }
                let along = (lane[i + 1] - lane[i - 1]).normalize_or(Vec2::X);
                let out = along.perp() * side;
                let foot = lane[i] + out * 3.0;
                let clear = !self.walled(foot)
                    && ways.iter().all(|w| w.distance(foot) > 2.2)
                    && things.iter().all(|t| flat(t.1).distance(foot) > t.3.max_element() + 1.5)
                    && self.posts.iter().all(|q| flat(q.0).distance(foot) > EVERY * 0.6);
                if !clear {
                    continue;
                }
                self.posts.push((Vec3::new(foot.x, height(foot), foot.y), -out));
                since = 0.0;
                side = -side;
            }
        }
    }

    /// Who keeps each stall on market day (#207): its seller, or else someone of the
    /// settlement's crafts with something to sell, a trade not at another stall first,
    /// or failing them someone who keeps a garden.
    fn keepers(&mut self) {
        let sells = |t: Trade| {
            matches!(t, Trade::Woodcutter | Trade::Forager | Trade::Fisher | Trade::Hunter | Trade::Smith | Trade::Weaver | Trade::Beekeeper | Trade::Farmer | Trade::Herder | Trade::Mason)
        };
        let mut taken: Vec<usize> = Vec::new();
        for s in 0..self.market.len() {
            let seller = self.residents.iter().find(|r| r.stall == Some(s)).map(|r| r.n);
            let free = |r: &&Resident| !taken.contains(&r.n) && r.stall.is_none();
            let shown: Vec<Trade> = taken.iter().map(|&n| self.residents[n].person.trade).collect();
            let crafter = || {
                let mut c: Vec<&Resident> = self.residents.iter().filter(|r| r.role == Role::Worker && sells(r.person.trade)).filter(free).filter(|r| !self.stays_out(r)).collect();
                c.sort_by_key(|r| (shown.contains(&r.person.trade), hash2(self.seed ^ 0x6ee9, s as i32, r.n as i32)));
                c.first().map(|r| r.n)
            };
            let gardener = || self.residents.iter().filter(|r| r.role == Role::Homemaker).filter(free).min_by_key(|r| hash2(self.seed ^ 0x6eea, s as i32, r.n as i32)).map(|r| r.n);
            let keeper = seller.or_else(crafter).or_else(gardener);
            if let Some(n) = keeper {
                taken.push(n);
            }
            self.market[s].keeper = keeper;
        }
    }

    /// Whether day `day` is its market day (#207).
    pub fn market_on(&self, day: i64) -> bool {
        !self.market.is_empty() && calendar::is_market_day_of(self.kind, self.seed, day)
    }

    /// The stall resident `n` keeps on market day, if any.
    pub fn keeps(&self, n: usize) -> Option<usize> {
        self.market.iter().position(|m| m.keeper == Some(n))
    }

    /// Everyone who lives there: a household for the beds of each home, each with a bed
    /// and a day's work.
    fn people(&mut self, c: &Continent, tongue: &str, s: &worldgen::settlements::Settlement, door_of: &[Option<usize>], lives: &Lives) {
        let net = &c.trails;
        let mut n = 0;
        let crafts: Vec<Trade> = s.livelihoods.iter().map(|&l| trade_of(l)).collect();
        // Who works at the buildings nobody lives in: the sawmill and the granary.
        let wanting: Vec<usize> = (0..self.plots.len()).filter(|&i| matches!(self.plots[i].purpose, Purpose::Sawmill | Purpose::Granary) && door_of[i].is_some()).collect();
        let mut sellers = 0;
        let plots = &net.plots[net.layouts[self.k].plots.clone()];
        // How each would be numbered in year 0, which is what the save knows them by.
        let (first, mut known) = (net.layouts[self.k].plots.start, 0);
        // A household of herders for each fold (#206): of the homes with someone else to
        // stay in, the nearest it, if near enough to be there as the gate is opened
        // without rising in the dark.
        let mut herding: Vec<(usize, usize)> = Vec::new();
        for (d, drove) in self.droves.iter().enumerate() {
            let fold = (drove.place, Spot::new(self.places[drove.place].at, self.places[drove.place].face, Task::Stand));
            let far = |i: usize| self.walk_length((self.plots[i].place, &self.inside_spot(i, 0, Task::Stand)), (fold.0, &fold.1));
            let home = (0..plots.len())
                .filter(|&i| plots[i].purpose == Purpose::Home && plots[i].sleepers >= 2 && door_of[i].is_some() && lives.standing(first + i) && !herding.iter().any(|h| h.0 == i))
                .map(|i| (i, far(i)))
                .filter(|&(_, f)| f / super::day::PACE <= HERDER_WALK)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((i, _)) = home {
                herding.push((i, d));
            }
        }
        // Where the settlement works a quarry and no mason keeps a yard among its houses,
        // a household or two of masons, who walk out to it.
        let mut quarrying: Vec<usize> = Vec::new();
        if self.quarry.is_some() && !plots.iter().any(|p| p.purpose == Purpose::Workplace(Livelihood::Quarry)) {
            let mut homes: Vec<usize> = (0..plots.len())
                .filter(|&i| plots[i].purpose == Purpose::Home && plots[i].sleepers >= 2 && door_of[i].is_some() && lives.standing(first + i) && !herding.iter().any(|h| h.0 == i))
                .collect();
            homes.sort_by_key(|&i| hash2(s.seed ^ 0x4a50, i as i32, 0));
            quarrying.extend(homes.into_iter().take(2));
        }
        let mut hands = Hands::default();
        for (i, p) in plots.iter().enumerate() {
            if p.sleepers == 0 || door_of[i].is_none() {
                continue;
            }
            // Over the years (#207): nobody lives in a home standing empty, being taken down
            // or rising again, and a home built again has a new household.
            let (stands, rebuilt) = (lives.standing(first + i), lives.generation(first + i));
            if !stands {
                known += p.sleepers as usize;
                continue;
            }
            let seed = hash2(s.seed ^ 0x9e09_1e5, i as i32, 1);
            let seed = if rebuilt == 0 { seed } else { hash2(seed, rebuilt as i32, 0x5e8a) };
            let h = |k: i32| unit(hash2(seed, k, 0x7a));
            let herds = herding.iter().find(|h| h.0 == i).map(|h| h.1);
            let trade = match p.purpose {
                Purpose::Workplace(l) => trade_of(l),
                Purpose::Smithy => Trade::Smith,
                Purpose::Tavern => Trade::Innkeeper,
                _ if herds.is_some() => Trade::Herder,
                _ if quarrying.contains(&i) => Trade::Mason,
                _ => {
                    let u = h(1);
                    if u < 0.32 {
                        Trade::Farmer
                    } else if u < 0.47 && !self.stalls.is_empty() {
                        Trade::Merchant
                    } else if u < 0.75 && !crafts.is_empty() {
                        crafts[(h(2) * crafts.len() as f32) as usize % crafts.len()]
                    } else {
                        Trade::None
                    }
                }
            };
            let wealth = (s.prosperity * 0.7 + 0.3 * h(3) + if p.step == worldgen::settlements::kit::Step::Stone { 0.2 } else { 0.0 }).clamp(0.0, 1.0);
            let household = people::household_of(seed, s.region, tongue, wealth, trade, p.sleepers as usize);
            let mut made: Vec<(Resident, usize)> = Vec::new();
            for (j, person) in household.into_iter().enumerate() {
                let bed = self.bed(i, j);
                let role = match (person.age, person.trade) {
                    (Age::Child, _) => Role::Child,
                    (Age::Elder, _) => Role::Elder,
                    (_, Trade::Innkeeper) => Role::Innkeeper,
                    (_, Trade::Merchant) if sellers < self.stalls.len() => {
                        sellers += 1;
                        Role::Seller
                    }
                    (_, Trade::None | Trade::Merchant) => Role::Homemaker,
                    _ => Role::Worker,
                };
                let stall = (role == Role::Seller).then_some(sellers - 1);
                // A new household is known apart from the one before it.
                let id = known + j + rebuilt as usize * (1 << 20);
                let mut r = Resident { n, id, person, home: i, member: j, bed, role, jobs: Vec::new(), stall };
                // Someone of a craft with no workplace of their own goes to one of its own
                // kind, or to the sawmill or granary.
                let mut at = i;
                if role == Role::Worker && !matches!(p.purpose, Purpose::Workplace(_) | Purpose::Smithy) {
                    // Only a workplace with a way to its door.
                    let own = (0..self.plots.len()).find(|&w| {
                        let b = &self.plots[w];
                        door_of[w].is_some() && (matches!(b.purpose, Purpose::Workplace(l) if trade_of(l) == r.person.trade) || (b.purpose == Purpose::Smithy && r.person.trade == Trade::Smith))
                    });
                    at = match own {
                        Some(w) => w,
                        None if !wanting.is_empty() && h(4 + j as i32) < 0.5 => wanting[n % wanting.len()],
                        None => i,
                    };
                }
                // Out in the land (#64, #206): the herder of the fold, or a farmer, a
                // logger or a mason with work out there to go to.
                match herds.filter(|&d| role == Role::Worker && r.person.trade == Trade::Herder && self.droves[d].herder.is_none()) {
                    Some(d) => {
                        self.droves[d].herder = Some(n);
                        let place = self.droves[d].place;
                        r.jobs = vec![Job { place, spot: Spot::new(self.places[place].at, self.places[place].face, Task::Stand) }];
                    }
                    None => r.jobs = self.land_jobs(&r, &mut hands).unwrap_or_else(|| self.jobs(&r, at)),
                }
                made.push((r, at));
                n += 1;
            }
            // Somebody stays at home in every household: failing anyone else, the last of
            // those who would go out to the land keeps to the work about the house.
            let home_kept = made.iter().any(|(r, _)| r.role != Role::Child && !self.works_out(r));
            if !home_kept && let Some(k) = made.iter().rposition(|(r, _)| self.works_out(r) && self.drives(r.n).is_none()) {
                let (r, at) = &made[k];
                let jobs = self.jobs(r, *at);
                made[k].0.jobs = jobs;
            }
            self.residents.extend(made.into_iter().map(|(r, _)| r));
            known += p.sleepers as usize;
        }
    }

    /// The bed of the `j`th of a household in building `i`, or the floor inside its door.
    fn bed(&self, i: usize, j: usize) -> Spot {
        let b = &self.plots[i];
        let (at, facing) = match b.beds.get(j) {
            Some(&(head, toward)) => (head, toward),
            None => (b.doorway - Vec3::new(b.out.x, 0.0, b.out.y) * 2.0, -b.out),
        };
        Spot { at, facing: facing.y.atan2(facing.x), task: Task::Lie, inside: Some(i), stool: false }
    }

    /// Where someone sits or stands outside the door of building `i`: to one side of it,
    /// facing out, `k` choosing the side.
    pub fn by_door(&self, i: usize, k: u32, task: Task) -> Spot {
        let b = &self.plots[i];
        let side = Vec2::new(-b.out.y, b.out.x) * if k % 2 == 0 { 1.0 } else { -1.0 };
        let far = 2.2 + ((k / 2) % 3) as f32 * 1.5;
        let at = self.places[b.place].at + Vec3::new(side.x, 0.0, side.y) * far - Vec3::new(b.out.x, 0.0, b.out.y) * 0.8;
        let toward = (b.out + side * 0.4).normalize_or(b.out);
        let mut s = Spot::new(at, toward, task);
        s.stool = task == Task::Sit;
        s
    }

    /// The jobs of someone working at building `at`: their trade's things in its yard,
    /// or by its door.
    fn jobs(&self, r: &Resident, at: usize) -> Vec<Job> {
        let b = &self.plots[at];
        let place = b.place;
        let thing = |kinds: &[Fitting]| b.things.iter().filter(|t| kinds.contains(&t.0)).copied().collect::<Vec<_>>();
        // Before a thing, facing it.
        let before = |t: &(Fitting, Vec3, Vec2, Vec2), task: Task, gap: f32| {
            let (_, mid, front, half) = *t;
            let d = half.x.max(half.y).min(2.5) + gap;
            Spot::new(mid + Vec3::new(front.x, 0.0, front.y) * d, -front, task)
        };
        let job = |spot: Spot| Job { place, spot };
        let door = |k: u32, task: Task| job(self.by_door(at, k, task));
        let mut out = Vec::new();
        match r.role {
            Role::Innkeeper => {
                out.push(Job { place, spot: Spot { inside: Some(at), ..self.inside_spot(at, b.seats.len(), Task::Stand) } });
                out.push(door(r.n as u32, Task::Sweep));
            }
            Role::Seller | Role::Homemaker | Role::Child | Role::Elder => {
                // Sweeping and the garden; the household's yard things are its own.
                out.push(door(r.n as u32, Task::Sweep));
                if let Some(g) = self.plots[r.home].garden {
                    out.push(Job { place: self.plots[r.home].place, spot: self.in_bed(g, r.n as u32, Task::Hoe) });
                }
            }
            Role::Worker => match r.person.trade {
                Trade::Woodcutter => {
                    // At the chopping block in the middle of the yard, then the log deck.
                    if let Some((a, c)) = b.yard {
                        let block = Vec3::new((a.x + c.x) as f32 * 0.5 + 0.5, self.places[place].at.y, (a.y + c.y) as f32 * 0.5 + 0.5);
                        let toward = (flat(block) - flat(self.places[place].at)).normalize_or(Vec2::X);
                        out.push(job(Spot::new(block - Vec3::new(toward.x, 0.0, toward.y) * 1.7, toward, Task::Chop)));
                    }
                    for t in thing(&[Fitting::LogDeck, Fitting::Sawhorse]).iter().take(1) {
                        out.push(job(before(t, Task::Carry, 1.2)));
                    }
                }
                Trade::Smith => {
                    for t in thing(&[Fitting::Anvil]).iter().take(1) {
                        out.push(job(before(t, Task::Hammer, 0.9)));
                    }
                    for t in thing(&[Fitting::Grindstone]).iter().take(1) {
                        out.push(job(before(t, Task::Tend, 1.0)));
                    }
                }
                Trade::Fisher => {
                    if let Some(w) = self.water {
                        // Each fisher has their own place along the bank.
                        let toward = self.places[w].face;
                        let along = toward.perp() * (((r.n % 5) as f32 - 2.0) * 2.5);
                        let at = self.places[w].at + Vec3::new(along.x, 0.0, along.y);
                        out.push(Job { place: w, spot: Spot::new(at, toward, Task::Fish) });
                    }
                    for t in thing(&[Fitting::NetRack, Fitting::FishRack]).iter().take(1) {
                        let mut s = before(t, Task::Sit, 1.3);
                        s.stool = true;
                        out.push(job(s));
                    }
                }
                Trade::Hunter => out.extend(thing(&[Fitting::HideFrame]).iter().take(2).map(|t| job(before(t, Task::Tend, 1.0)))),
                Trade::Weaver => {
                    for t in thing(&[Fitting::WithyBundles, Fitting::BasketStack]).iter().take(1) {
                        let mut s = before(t, Task::Sit, 1.4);
                        s.stool = true;
                        out.push(job(s));
                    }
                    out.extend(thing(&[Fitting::SoakingTrough]).iter().take(1).map(|t| job(before(t, Task::Tend, 1.0))));
                }
                Trade::Beekeeper => out.extend(thing(&[Fitting::Skep, Fitting::BoxHive]).iter().take(4).map(|t| job(before(t, Task::Tend, 1.1)))),
                Trade::Forager => {
                    out.extend(thing(&[Fitting::HerbRack]).iter().take(2).map(|t| job(before(t, Task::Tend, 1.0))));
                    if let Some(g) = b.garden {
                        out.push(job(self.in_bed(g, r.n as u32, Task::Hoe)));
                    }
                }
                Trade::Farmer => {
                    if let Some(g) = self.plots[r.home].garden {
                        out.push(Job { place: self.plots[r.home].place, spot: self.in_bed(g, r.n as u32, Task::Hoe) });
                    }
                }
                Trade::Mason => out.push(door(r.n as u32 + 1, Task::Hammer)),
                _ => {}
            },
        }
        // Two of a household at the same work take its tasks in turn, or work side by
        // side at its one task.
        if r.member % 2 == 1 && r.role == Role::Worker {
            if out.len() > 1 {
                out.rotate_left(1);
            } else if let Some(j) = out.first_mut().filter(|j| j.spot.inside.is_none() && j.spot.task != Task::Fish) {
                let side = Vec2::from_angle(j.spot.facing).perp() * 2.0;
                j.spot.at += Vec3::new(side.x, 0.0, side.y);
            }
        }
        // Nothing done outdoors stands within walls (a rack set against a house is let
        // be, rather than worked from inside it).
        out.retain(|j| j.spot.inside.is_some() || !self.walled(flat(j.spot.at)));
        // Whatever else, the work by their own door.
        if out.is_empty() {
            out.push(door(r.n as u32, if r.role == Role::Worker { Task::Carry } else { Task::Sweep }));
        }
        out
    }

    /// Whether `p` is within the walls of any building, or less than a step from them.
    fn walled(&self, p: Vec2) -> bool {
        self.plots.iter().any(|b| b.within(p, 0.5))
    }

    /// A place to stand in the garden bed `g`, among the rows, by `k`.
    fn in_bed(&self, (a, b): (IVec2, IVec2), k: u32, task: Task) -> Spot {
        let span = (b - a + IVec2::ONE).max(IVec2::ONE);
        let u = unit(hash2(k, a.x, a.y));
        let p = Vec2::new(a.x as f32 + 0.5 + (u * span.x as f32).floor().min(span.x as f32 - 1.0), (a.y + b.y) as f32 * 0.5 + 0.5);
        let y = self.plots.iter().find(|x| x.garden == Some((a, b))).map_or(self.centre.y, |x| x.floor as f32);
        Spot::new(Vec3::new(p.x + 0.5, y, p.y), Vec2::new(0.0, if k % 2 == 0 { 1.0 } else { -1.0 }), task)
    }

    /// A place inside building `i`: its `k`th seat, sitting, or the floor inside the door
    /// beyond the seats, standing.
    pub fn inside_spot(&self, i: usize, k: usize, standing: Task) -> Spot {
        let b = &self.plots[i];
        match b.seats.get(k) {
            Some(&(at, toward)) => Spot { at, facing: toward.y.atan2(toward.x), task: Task::Sit, inside: Some(i), stool: false },
            None => {
                // Standing in rows of three from the door inward, a long pace apart, and
                // kept inside the walls.
                let over = (k - b.seats.len()) as f32;
                let side = Vec2::new(-b.out.y, b.out.x) * (((over as u32 % 3) as f32) - 1.0) * 2.2;
                let at = b.doorway - Vec3::new(b.out.x, 0.0, b.out.y) * (2.5 + (over / 3.0).floor() * 2.2) + Vec3::new(side.x, 0.0, side.y);
                let (lo, hi) = (b.lo.as_vec2() + 1.5, b.hi.as_vec2() - 0.5);
                let at = Vec3::new(at.x.clamp(lo.x, hi.x.max(lo.x)), at.y, at.z.clamp(lo.y, hi.y.max(lo.y)));
                Spot { at, facing: (-b.out.y).atan2(-b.out.x) + (over * 2.4).sin() * 1.2, task: standing, inside: Some(i), stool: false }
            }
        }
    }

    /// Whether a walk from `a` at place `pa` to `b` at `pb` goes straight across: from one
    /// place in a house to another, or from spot to spot of the open green.
    fn across(&self, pa: usize, a: &Spot, pb: usize, b: &Spot) -> bool {
        let indoors = a.inside.is_some() && a.inside == b.inside;
        let open = pa == pb && a.inside.is_none() && b.inside.is_none();
        // Out in the land, about a field, the camp, the quarry or the fold, as on the
        // green.
        indoors || (open && (pa == self.green_place || self.places[pa].what.out()))
    }

    /// The way someone walks from `a` to `b` (each a spot at a place): out of a door if
    /// indoors, along the lanes, and in at the other end.
    pub fn walk(&self, (pa, a): (usize, &Spot), (pb, b): (usize, &Spot)) -> Walk {
        if self.across(pa, a, pb, b) {
            return Walk::new(vec![a.at, b.at]);
        }
        let mut pts = Vec::new();
        let (qa, qb) = (&self.places[pa], &self.places[pb]);
        let door = |p: &Place| match p.what {
            Where::Door(i) => Some(self.plots[i].doorway),
            _ => None,
        };
        match a.inside {
            Some(_) => pts.extend(door(qa)),
            None => pts.extend(self.last_steps(pa, a).into_iter().rev()),
        }
        pts.push(qa.at);
        if pa != pb {
            pts.extend(qa.spur.iter().rev().skip(1));
            if let Some(route) = self.lanes.route(qa.anchor, qb.anchor) {
                pts.extend(route.iter().map(|&n| self.lanes.nodes[n as usize]));
            }
            pts.extend(qb.spur.iter());
        }
        match b.inside {
            Some(_) => pts.extend(door(qb)),
            None => pts.extend(self.last_steps(pb, b)),
        }
        Walk::new(pts)
    }

    /// The last steps from place `p` to `s` there, outdoors, after its own `at`: straight
    /// to it, or round the building whose door it is when that stands in the way, as it
    /// does of a garden behind.
    pub fn last_steps(&self, p: usize, s: &Spot) -> Vec<Vec3> {
        let q = &self.places[p];
        let mut out = match q.what {
            Where::Door(i) => self.round(i, q.at, s.at),
            _ => Vec::new(),
        };
        out.push(s.at);
        out
    }

    /// The corners to go by from `from` to `to` round building `i`, a step clear of its
    /// walls, on the side `to` is; none if it is not in the way.
    fn round(&self, i: usize, from: Vec3, to: Vec3) -> Vec<Vec3> {
        let b = &self.plots[i];
        // Its walls, within the eaves and steps.
        let (lo, hi) = (b.lo.as_vec2() + 1.0, b.hi.as_vec2());
        let crosses = |a: Vec2, c: Vec2| (1..16).any(|k| b.within(a.lerp(c, k as f32 / 16.0), 0.0));
        if !crosses(flat(from), flat(to)) {
            return Vec::new();
        }
        let mid = (lo + hi) * 0.5;
        let half = (hi - lo) * 0.5 + 1.2;
        let (out, side) = (b.out, b.out.perp());
        let sign = if (flat(to) - mid).dot(side) >= 0.0 { 1.0 } else { -1.0 };
        let corner = |ahead: f32| mid + out * ahead * half.dot(out.abs()) + side * sign * half.dot(side.abs());
        let (front, back) = (corner(1.0), corner(-1.0));
        let mut steps = vec![Vec3::new(front.x, from.y, front.y)];
        if crosses(front, flat(to)) {
            steps.push(Vec3::new(back.x, to.y, back.y));
        }
        steps
    }

    /// How far the last steps to `s` at `p` are, from the place's `at`.
    fn last_length(&self, p: usize, s: &Spot) -> f32 {
        let mut d = 0.0;
        let mut last = self.places[p].at;
        for step in self.last_steps(p, s) {
            d += step.distance(last);
            last = step;
        }
        d
    }

    /// How far it is to walk from `a` to `b`, as [`Self::walk`] goes, without finding the
    /// way: from the distances between places.
    pub fn walk_length(&self, (pa, a): (usize, &Spot), (pb, b): (usize, &Spot)) -> f32 {
        let end = |p: usize, s: &Spot| match (s.inside, self.places[p].what) {
            (Some(_), Where::Door(i)) => self.plots[i].doorway.distance(self.places[p].at),
            (Some(_), _) => 0.0,
            (None, _) => self.last_length(p, s),
        };
        if self.across(pa, a, pb, b) {
            return a.at.distance(b.at);
        }
        end(pa, a) + self.dist[pa][pb] + end(pb, b)
    }

    /// The building feet at `p` (continent-local) are inside, if any.
    pub fn inside(&self, p: Vec3) -> Option<usize> {
        self.plots.iter().position(|b| b.holds(p))
    }

    /// A word for a place, for the console.
    pub fn place_word(&self, p: usize) -> String {
        match self.places[p].what {
            Where::Door(i) => self.plots[i].purpose.word().to_string(),
            Where::Well => "the well".into(),
            Where::Stall(_) => "the market".into(),
            Where::Green => "the green".into(),
            Where::Tree => "the village tree".into(),
            Where::Shrine => "the shrine".into(),
            Where::Water => "the water".into(),
            Where::Field(f) => format!("the {} field", self.fields[f].crop.word()),
            Where::Camp => "the logging camp".into(),
            Where::Quarry => "the quarry".into(),
            Where::Pasture(_) => "the pasture".into(),
        }
    }

    /// Whether resident `r`'s work is out in the land.
    pub fn works_out(&self, r: &Resident) -> bool {
        r.jobs.first().is_some_and(|j| self.places[j.place].what.out())
    }

    /// How long resident `r`'s walk out to their work in the land takes, in hours.
    pub fn walk_out(&self, r: &Resident) -> f32 {
        let home = self.plots[r.home].place;
        let inside = self.inside_spot(r.home, r.member, Task::Stand);
        r.jobs.first().map_or(f32::MAX, |j| self.walk_length((home, &inside), (j.place, &j.spot)) / super::day::PACE)
    }

    /// Whether resident `r` works at a logging camp too far out to be there and back
    /// each day, and so stays out there through the working week, sleeping under its
    /// lean-to, and comes home for the day of rest.
    pub fn stays_out(&self, r: &Resident) -> bool {
        r.jobs.len() > 3 && r.jobs.first().is_some_and(|j| self.places[j.place].what == Where::Camp) && self.walk_out(r) > super::day::FURTHEST_OUT
    }

    /// The fold resident `n` drives, if any.
    pub fn drives(&self, n: usize) -> Option<usize> {
        self.droves.iter().position(|d| d.herder == Some(n))
    }

    /// The jobs of someone of `r`'s trade out in the land, if their settlement works
    /// there and has room for another: a field, the logging camp or the quarry, each
    /// with two things to do in turn and somewhere to sit down at midday.
    fn land_jobs(&self, r: &Resident, hands: &mut Hands) -> Option<Vec<Job>> {
        if r.role != Role::Worker {
            return None;
        }
        let at_works = |w: &Works, m: usize| -> Vec<Job> {
            let n = w.spots.len();
            // More hands than things to do: the next works beside the first.
            let beside = |s: Spot| {
                if m < n {
                    return s;
                }
                let side = Vec2::from_angle(s.facing).perp() * 1.8;
                Spot { at: s.at + Vec3::new(side.x, 0.0, side.y), ..s }
            };
            let job = |spot: Spot| Job { place: w.place, spot };
            let mut jobs = vec![job(beside(w.spots[m % n])), job(beside(w.spots[(m + 1) % n])), job(w.rest[m % w.rest.len()])];
            jobs.extend(w.beds.get(m % w.beds.len().max(1)).map(|&b| job(b)));
            jobs
        };
        match r.person.trade {
            Trade::Farmer if !self.fields.is_empty() => {
                let f = hash2(self.seed ^ 0xfa53, r.home as i32, r.member as i32) as usize % self.fields.len();
                hands.fields.resize(self.fields.len(), 0);
                let m = hands.fields[f];
                hands.fields[f] += 1;
                let field = &self.fields[f];
                let spot = |k: usize| {
                    let (at, face) = field.spots[k % field.spots.len()];
                    Job { place: field.place, spot: Spot::new(at, face, Task::Hoe) }
                };
                let p = &self.places[field.place];
                let side = p.face.perp() * (m as f32 * 1.6 - 0.8);
                let rest = Job { place: field.place, spot: Spot::new(p.at + Vec3::new(side.x, 0.0, side.y), p.face, Task::SitLow) };
                Some(vec![spot(2 * m), spot(2 * m + 1), rest])
            }
            Trade::Woodcutter if hands.camp < CAMP_HANDS => {
                let w = self.camp.as_ref()?;
                hands.camp += 1;
                Some(at_works(w, hands.camp - 1))
            }
            Trade::Mason if hands.quarry < QUARRY_HANDS => {
                let w = self.quarry.as_ref()?;
                hands.quarry += 1;
                Some(at_works(w, hands.quarry - 1))
            }
            _ => None,
        }
    }
}
