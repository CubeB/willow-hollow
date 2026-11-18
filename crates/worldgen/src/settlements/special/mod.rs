//! The buildings a settlement has besides its homes (#203): the tavern, the smithy, the
//! sawmill, the granary, the market hall, the guild house and the fishers' boat shed.
//! Each is built in its region's own way, from the same rules the house kit builds homes
//! by, and furnished: walked into through its door, as every home is.
//!
//! [`build`] makes one from a [`SpecialSpec`] and the [`Palette`] of the place it stands.
//! What it makes is a [`kit::Building`] in landmark model space, as the kit's homes are:
//! the origin column is the middle of the site, `y = 0` is the first layer above the
//! ground (the ground's surface is `y = -1`), footings go down to `y = -4`, the rooms and
//! doorway are cleared with AIR, and the door is on the +X side. A caller turns it to its
//! lane with [`Built::turned`]. Besides the building, a [`Built`] says what the layout
//! needs of it: the beds let to travellers, where the bar, the forge and the stalls are
//! used from, the water wheel, and the guilds whose banners hang on a guild house.
//!
//! Two of them do two jobs, so a settlement's cap of buildings leaves room for homes:
//! the smith lives above the forge, and the fisher in the boat shed. The tavern's keeper
//! lives over the common room, beside the guests.
//!
//! The well head and the gate are not buildings anyone goes into, and are not made here.

mod boatshed;
mod frame;
mod granary;
mod guild;
mod market;
mod sawmill;
mod smithy;
mod tavern;
#[cfg(test)]
mod tests;

use glam::{IVec3, Vec3};
use voxel::{AIR, Voxel};

use super::{Age, Livelihood, Region, Settlement, kit::Spot};
use crate::{
    features::{
        designs::{self, Cell, Design, Fine, Kind, Role, Special},
        landmarks::Palette,
        shape::rotate,
    },
    materials as mat,
};
use voxel::mesh::CELLS;

pub use super::kit::Building;

/// What to build.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SpecialSpec {
    pub which: Special,
    pub region: Region,
    /// 0 new to 1 old: weathered timber, worn roofs, render flaking; and, with little
    /// wealth, things left to fall apart.
    pub age: f32,
    /// 0 poorest to 1 richest: size and finish. A tavern's wealth is how many it sleeps.
    pub wealth: f32,
    pub seed: u32,
    /// The side the stream runs past, as built (before the building is turned): `+Z`,
    /// `-Z` or `-X` (the back; `+X` is the door, and is taken as `+Z`). A sawmill with a
    /// stream has a water wheel on that side; without one it saws by hand over a pit.
    /// Turn a direction in the world back to model space with
    /// `rotate(d, (4 - turns) & 3)`. Nothing else looks at it.
    pub stream: Option<IVec3>,
    /// What the guild house's guilds are chosen from. Nothing else looks at it.
    pub facts: Facts,
}

impl SpecialSpec {
    /// A spec with nothing set but the building, its region and seed: middling age and
    /// wealth, no stream, and the facts of nowhere in particular.
    pub fn new(which: Special, region: Region, seed: u32) -> Self {
        SpecialSpec { which, region, age: 0.4, wealth: 0.5, seed, stream: None, facts: Facts::default() }
    }
}

/// What a settlement is like, for choosing the guilds that share its guild house.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Facts {
    /// Willows grow about it: it has basket weavers.
    pub willows: bool,
    /// It stands on stone country, where rock breaks the surface: it has a quarry.
    pub stone: bool,
    /// It trades far: on the coast, with a port, or with partners along the roads.
    pub trade: bool,
    /// It is old or prosperous enough to keep painters.
    pub colour: bool,
}

impl Facts {
    /// The facts of a settlement as stage 1 found it.
    pub fn of(s: &Settlement) -> Facts {
        Facts {
            willows: s.has(Livelihood::Weaver),
            stone: s.has(Livelihood::Quarry),
            trade: s.coast || s.port.is_some() || s.has(Livelihood::Port) || s.partners.len() >= 2,
            colour: s.age == Age::Ancient || s.prosperity > 0.6,
        }
    }
}

/// The guilds a guild house is shared by (#203).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Guild {
    /// Where willows grow (#50).
    Weavers,
    /// On stone country (#38).
    Masons,
    /// #51.
    Painters,
    /// #52.
    Merchants,
    /// Anywhere.
    Wayfarers,
}

impl Guild {
    pub const ALL: [Guild; 5] = [Guild::Weavers, Guild::Masons, Guild::Painters, Guild::Merchants, Guild::Wayfarers];

    pub fn name(self) -> &'static str {
        match self {
            Guild::Weavers => "Weavers",
            Guild::Masons => "Masons and Carvers",
            Guild::Painters => "Painters",
            Guild::Merchants => "Merchants",
            Guild::Wayfarers => "Wayfarers and Cartographers",
        }
    }

    /// The colours of its banner: the field and the stripe across it.
    pub fn colours(self) -> (Voxel, Voxel) {
        match self {
            Guild::Weavers => (mat::CLOTH_BLUE, mat::CLOTH_PALE),
            Guild::Masons => (mat::CLOTH_PALE, mat::CLOTH_RED),
            Guild::Painters => (mat::CLOTH_RED, mat::CLOTH_YELLOW),
            Guild::Merchants => (mat::CLOTH_YELLOW, mat::CLOTH_BLUE),
            Guild::Wayfarers => (mat::CLOTH_GREEN, mat::CLOTH_YELLOW),
        }
    }

    /// Whether a place with these facts has the guild. Wayfarers and cartographers are
    /// everywhere roads are.
    pub fn suits(self, f: &Facts) -> bool {
        match self {
            Guild::Weavers => f.willows,
            Guild::Masons => f.stone,
            Guild::Painters => f.colour,
            Guild::Merchants => f.trade,
            Guild::Wayfarers => true,
        }
    }
}

/// The two or three guilds that share a guild house: those the place suits, in an order
/// the seed gives, three of them in a richer town. Where too few suit, merchants (every
/// great town trades) and then painters make up two.
pub fn guilds(facts: &Facts, wealth: f32, seed: u32) -> Vec<Guild> {
    let mut suits: Vec<Guild> = Guild::ALL.into_iter().filter(|g| g.suits(facts)).collect();
    for fill in [Guild::Merchants, Guild::Painters] {
        if suits.len() < 2 && !suits.contains(&fill) {
            suits.push(fill);
        }
    }
    suits.sort_by_key(|&g| crate::noise::hash2(seed ^ 0x6a11_d5e5, g as i32, 0));
    suits.truncate(if wealth > 0.45 { 3 } else { 2 });
    suits.sort_by_key(|&g| g as u8);
    suits
}

/// What a place in a building is used for, to be walked to.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Use {
    /// The front of a tavern's counter, where drink is bought.
    Bar,
    /// Before a fire to sit by.
    Hearth,
    /// Before the smith's forge.
    Forge,
    /// Beside the anvil.
    Anvil,
    /// Beside the saw: the frame saw's carriage, or the edge of the saw pit.
    Saw,
    /// Before a market stall.
    Stall,
    /// Among the bins and sacks of a granary.
    Store,
    /// At a table: a guild's, or the long one in its hall.
    Table,
    /// Beside the boat on its slipway.
    Boat,
}

/// A water wheel turning a saw.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Wheel {
    /// The voxel at the middle of the axle, on the wheel's outer face.
    pub hub: IVec3,
    /// The way its axle runs, out of the building: the side the stream is on.
    pub side: IVec3,
    /// Its radius in voxels, to the tips of its paddles.
    pub radius: i32,
}

/// A building of [`build`], and what the layout and the game need to know of it.
#[derive(Clone, Debug)]
pub struct Built {
    pub which: Special,
    pub building: Building,
    /// The beds let to travellers, a tavern's (also among `building.inside.beds`, after
    /// the household's own).
    pub guest_beds: Vec<Spot>,
    /// Where each thing in it is used from: the voxel stood on, inside the walls.
    pub uses: Vec<(Use, IVec3)>,
    /// The voxel of embers that glows in the smith's forge (or the tavern's hearth): it
    /// lights the room at night and the game's smoke rises from it.
    pub fire: Option<IVec3>,
    pub wheel: Option<Wheel>,
    /// The guilds of a guild house, whose banners hang over its door.
    pub guilds: Vec<Guild>,
    /// The voxel the board of a hanging sign hangs in, over a tavern's door.
    pub sign: Option<IVec3>,
}

impl Built {
    /// Turned by quarter turns about the vertical through the origin, as
    /// [`Building::turned`] turns the building.
    pub fn turned(&self, turns: u32) -> Built {
        let r = |q: IVec3| rotate(q, turns);
        let spot = |s: &Spot| Spot { at: r(s.at), along: r(s.along), facing: r(s.facing), ..*s };
        Built {
            which: self.which,
            building: self.building.turned(turns),
            guest_beds: self.guest_beds.iter().map(spot).collect(),
            uses: self.uses.iter().map(|&(u, q)| (u, r(q))).collect(),
            fire: self.fire.map(r),
            wheel: self.wheel.map(|w| Wheel { hub: r(w.hub), side: r(w.side), ..w }),
            guilds: self.guilds.clone(),
            sign: self.sign.map(r),
        }
    }

    /// Where a traveller lies in each guest bed once the building is turned by `turns`
    /// and its origin set at voxel `origin`: the middle of the bed, on the floor under it,
    /// in the same voxels as `origin` (continent-local, for `trails::Network::guest_beds`).
    pub fn guest_beds_at(&self, origin: IVec3, turns: u32) -> Vec<Vec3> {
        let r = |q: IVec3| rotate(q, turns);
        self.guest_beds.iter().map(|s| lying(&Spot { at: r(s.at), along: r(s.along), facing: r(s.facing), ..*s }) + origin.as_vec3()).collect()
    }
}

/// The middle of a bed, on the floor under it, in the bed's own voxels.
pub fn lying(s: &Spot) -> Vec3 {
    let mid = |n: i32| (n - 1) as f32 * 0.5;
    s.at.as_vec3() + Vec3::new(0.5, 0.0, 0.5) + s.along.as_vec3() * mid(s.len) + s.facing.as_vec3() * mid(s.wide)
}

/// The settlement buildings [`build`] makes: all of them but the well head and the gate.
pub const MADE: [Special; 7] = [Special::Tavern, Special::Smithy, Special::Sawmill, Special::Granary, Special::MarketHall, Special::GuildHouse, Special::BoatShed];

/// The most voxels across a building may take, x and z, its eaves, steps and wheel
/// included: 12 by 16 metres, and 16 by 20 for a market hall.
pub fn largest(which: Special) -> (i32, i32) {
    match which {
        Special::MarketHall => (40, 50),
        _ => (30, 40),
    }
}

/// Builds the settlement building `spec` asks for, of the materials of palette `pal`
/// (taken as it is: vary it for the place first, as landmarks do, if wanted). None for
/// the well head and the gate, which are not made here.
pub fn build(spec: &SpecialSpec, pal: &Palette) -> Option<Built> {
    make(spec, pal).map(|(b, _)| b)
}

/// [`build`], and whatever the generator put where it should not have: a fitting into a
/// wall, two things into one place. The tests hold every building to none.
pub(crate) fn make(spec: &SpecialSpec, pal: &Palette) -> Option<(Built, Vec<String>)> {
    let f = frame::Frame::new(spec, pal);
    Some(match spec.which {
        Special::Tavern => tavern::build(f),
        Special::Smithy => smithy::build(f),
        Special::Sawmill => sawmill::build(f),
        Special::Granary => granary::build(f),
        Special::MarketHall => market::build(f),
        Special::GuildHouse => guild::build(f),
        Special::BoatShed => boatshed::build(f),
        Special::WellHead | Special::Gate => return None,
    })
}

/// The building `spec` asks for as a structure of one's own to edit, named `name`, of kind
/// `Special`, keeping its door, beds and hearth (see [`crate::settlements::kit::Home`])
/// so it is still walked into once edited. As the kit copies a home: every voxel a
/// palette role where it came from one, and its own material where not.
pub fn copy(spec: &SpecialSpec, pal: &Palette, name: &str) -> Option<Design> {
    let b = build(spec, pal)?.building;
    // The same building again with every role in a material of its own, to say which
    // role each voxel came from where the place uses one material for two.
    let tags = designs::tag_palette(pal);
    let t = build(spec, &tags)?.building;
    let same = b.voxels.len() == t.voxels.len() && b.voxels.iter().zip(&t.voxels).all(|(a, c)| a.0 == c.0);
    let cell = |v: Voxel, tag: Option<Voxel>, at: IVec3| -> Cell {
        if v == AIR {
            return Cell::Air;
        }
        let gives = |r: Role| r.plain(pal) == Some(v) && r.resolve(pal, spec.seed, at) == v;
        if let Some(t) = tag {
            if let Some(r) = Role::ALL.into_iter().find(|&r| r.plain(&tags) == Some(t) && gives(r)) {
                return Cell::Role(r);
            }
            if t == v {
                return Cell::Material(v);
            }
        }
        match Role::ALL.into_iter().find(|&r| gives(r)) {
            Some(r) => Cell::Role(r),
            None => Cell::Material(v),
        }
    };
    let mut d = Design::new(Kind::Special(spec.which), name);
    d.note = format!("copied from the {} of {}, variant {} (age {:.2}, wealth {:.2})", spec.which.label().to_lowercase(), spec.region.word(), spec.seed, spec.age, spec.wealth);
    for (k, &(q, v)) in b.voxels.iter().enumerate() {
        d.cells.insert(q, cell(v, same.then(|| t.voxels[k].1), q));
    }
    let same_fine = b.fine.len() == t.fine.len();
    for (k, f) in b.fine.iter().enumerate() {
        let tag = same_fine.then(|| t.fine[k]).filter(|g| g.lo == f.lo && g.hi == f.hi).map(|g| g.voxel);
        let lo = IVec3::from_array(f.lo);
        d.fine.push(Fine {
            lo,
            hi: IVec3::from_array(f.hi),
            cell: cell(f.voxel, tag, lo.div_euclid(IVec3::splat(CELLS))),
            sway_lo: f.sway_lo,
            sway_hi: f.sway_hi,
            thin: f.thin,
            season: f.season,
            flap: f.flap,
        });
    }
    d.clear = b.clear.clone();
    d.home = Some(super::kit::Home::of(&b));
    Some(d)
}
