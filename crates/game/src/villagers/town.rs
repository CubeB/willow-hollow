//! One settlement's people and the places of their days, made from the settlement as it
//! was laid out (`worldgen::settlements::layout`): a household for the beds of each home,
//! a bed for everyone, a place of work and a few haunts, and the lanes between them all.
//! Nothing here changes once it is made, and it is made the same every time.

use glam::{DVec3, IVec2, Vec2, Vec3};
use worldgen::{
    continent::Continent,
    features::fittings::{self, Fitting},
    macro_layout::MACRO_SIZE,
    names::PlaceKind,
    noise::{hash2, unit},
    settlements::{Kind, Livelihood, layout::Purpose},
    surface::Sampler,
};

use super::lanes::{Lanes, Walk, flat};
use crate::{
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
    /// Their number in the settlement: what they are known by in the save.
    pub n: usize,
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

impl Town {
    /// Settlement `k` of continent `c` (whose cell is `cell`), with its people, for a world
    /// of `world_seed`.
    pub fn build(c: &Continent, world_seed: u32, cell: (i32, i32), k: usize) -> Option<Town> {
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
        for &(f, mid, front, half) in &things {
            let ground = |p: Vec3| Vec3::new(p.x, height(flat(p)), p.z);
            match f {
                Fitting::StoneWell | Fitting::Shaduf | Fitting::OasisWell => {
                    let at = mid + Vec3::new(front.x, 0.0, front.y) * (half.x.max(half.y) + 1.2);
                    if let Some(i) = place(Where::Well, Vec3::new(at.x, mid.y, at.z), &mut places) {
                        places[i].face = -front;
                        wells.push(i);
                    }
                }
                Fitting::MarketStall => {
                    let deep = half.x.min(half.y);
                    let at = mid + Vec3::new(front.x, 0.0, front.y) * (deep + 1.6);
                    if let Some(i) = place(Where::Stall(stalls.len()), Vec3::new(at.x, mid.y, at.z), &mut places) {
                        places[i].face = -front;
                        stalls.push(i);
                        let back = mid - Vec3::new(front.x, 0.0, front.y) * (deep * 0.35);
                        behind.push(Spot::new(ground(back).with_y(mid.y), front, Task::Talk));
                    }
                }
                _ => {}
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
        };
        town.tavern = (0..town.plots.len()).find(|&i| town.plots[i].purpose == Purpose::Tavern && door_of[i].is_some());
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
        town.people(c, tongue, s, &door_of);
        Some(town)
    }

    /// Everyone who lives there: a household for the beds of each home, each with a bed
    /// and a day's work.
    fn people(&mut self, c: &Continent, tongue: &str, s: &worldgen::settlements::Settlement, door_of: &[Option<usize>]) {
        let net = &c.trails;
        let mut n = 0;
        let crafts: Vec<Trade> = s.livelihoods.iter().map(|&l| trade_of(l)).collect();
        // Who works at the buildings nobody lives in: the sawmill and the granary.
        let wanting: Vec<usize> = (0..self.plots.len()).filter(|&i| matches!(self.plots[i].purpose, Purpose::Sawmill | Purpose::Granary) && door_of[i].is_some()).collect();
        let mut sellers = 0;
        let plots = &net.plots[net.layouts[self.k].plots.clone()];
        for (i, p) in plots.iter().enumerate() {
            if p.sleepers == 0 || door_of[i].is_none() {
                continue;
            }
            let seed = hash2(s.seed ^ 0x9e09_1e5, i as i32, 1);
            let h = |k: i32| unit(hash2(seed, k, 0x7a));
            let trade = match p.purpose {
                Purpose::Workplace(l) => trade_of(l),
                Purpose::Smithy => Trade::Smith,
                Purpose::Tavern => Trade::Innkeeper,
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
                let mut r = Resident { n, person, home: i, member: j, bed, role, jobs: Vec::new(), stall };
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
                r.jobs = self.jobs(&r, at);
                self.residents.push(r);
                n += 1;
            }
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
        let green = pa == pb && pa == self.green_place && a.inside.is_none() && b.inside.is_none();
        indoors || green
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
        }
    }
}
