//! Settlements: hamlets, villages and towns, where people live (see #45 for the plan).
//!
//! A settlement is decided on the continent map, before the landmarks and trails, from a
//! few seeded facts (its kind, age, prosperity, water and livelihoods); everything built
//! there follows from those facts and from the country it stands in.

pub mod furnish;
pub mod kit;
pub mod place;

use glam::Vec2;

use crate::{biome::Biome, names::PlaceKind, scale::m};

/// The building tradition of a stretch of country: how its people build their huts and
/// houses, and which regional components (awnings, turf roofs, stilts) they add. One
/// region covers several biomes; a settlement takes the region of the biome at its core.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Region {
    /// Temperate lowland: cob and thatch, timber frames.
    Temperate,
    /// Moor and upland: low drystone, turf and slate.
    Upland,
    /// Taiga and tundra: log houses with turf roofs.
    Northern,
    /// High mountain country: steep shingle roofs held down with stones.
    Alpine,
    /// Desert and savanna: mud brick, flat roofs, awnings and sail shades.
    Desert,
    /// Marsh and mangrove: stilts, reeds and boardwalks.
    Wetland,
    /// Rainforest: raised timber floors under deep palm thatch.
    Rainforest,
    /// Old-growth forest: long plank halls on root plinths.
    OldGrowth,
}

impl Region {
    pub const ALL: [Region; 8] = [
        Region::Temperate,
        Region::Upland,
        Region::Northern,
        Region::Alpine,
        Region::Desert,
        Region::Wetland,
        Region::Rainforest,
        Region::OldGrowth,
    ];

    /// The region a biome's people build in.
    pub fn of(biome: Biome) -> Self {
        use Biome::*;
        match biome {
            Desert | Savanna | DuneSea | RedRock => Region::Desert,
            Heath | Moor | Steppe | Volcanic => Region::Upland,
            Taiga | SnowyPine | Bog | Tundra => Region::Northern,
            Alpine | AlpineMeadow => Region::Alpine,
            Marsh | Mangrove => Region::Wetland,
            Rainforest => Region::Rainforest,
            OldGrowth => Region::OldGrowth,
            Grassland | Deciduous | BirchGrove | Woodland | Ocean => Region::Temperate,
        }
    }

    /// A short lower-case word for the region, for file names and tools.
    pub fn word(self) -> &'static str {
        match self {
            Region::Temperate => "temperate",
            Region::Upland => "upland",
            Region::Northern => "northern",
            Region::Alpine => "alpine",
            Region::Desert => "desert",
            Region::Wetland => "wetland",
            Region::Rainforest => "rainforest",
            Region::OldGrowth => "oldgrowth",
        }
    }

    /// A biome typical of the region, for building a palette where there is no real
    /// ground to read (the structure editor, tests).
    pub fn biome(self) -> Biome {
        match self {
            Region::Temperate => Biome::Deciduous,
            Region::Upland => Biome::Moor,
            Region::Northern => Biome::Taiga,
            Region::Alpine => Biome::AlpineMeadow,
            Region::Desert => Biome::Savanna,
            Region::Wetland => Biome::Marsh,
            Region::Rainforest => Biome::Rainforest,
            Region::OldGrowth => Biome::OldGrowth,
        }
    }
}

/// How big a settlement is. Only walk-in buildings count: wells, hives, stalls, gates,
/// walls and jetties do not (see #45).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Kind {
    /// A few huts or a house and a store, a short walk from a village or town.
    Hamlet,
    Village,
    /// A village with a smithy and a tavern.
    LargeVillage,
    /// Stone houses about a market square, gates on the main roads.
    Town,
    /// A guild house, a park, a market hall and an old stone core.
    GreatTown,
}

impl Kind {
    pub const ALL: [Kind; 5] = [Kind::Hamlet, Kind::Village, Kind::LargeVillage, Kind::Town, Kind::GreatTown];

    /// Fewest and most walk-in buildings.
    pub fn buildings(self) -> (u8, u8) {
        match self {
            Kind::Hamlet => (2, 4),
            Kind::Village => (5, 12),
            Kind::LargeVillage => (13, 18),
            Kind::Town => (19, 22),
            Kind::GreatTown => (23, 25),
        }
    }

    /// Fewest and most people living there.
    pub fn people(self) -> (u8, u8) {
        match self {
            Kind::Hamlet => (4, 10),
            Kind::Village => (12, 40),
            Kind::LargeVillage => (40, 60),
            Kind::Town => (55, 75),
            Kind::GreatTown => (70, 90),
        }
    }

    /// How much it draws people, for the traffic on the trails, on the scale of
    /// [`crate::trails::sites::importance`]: well above any landmark, so the busiest
    /// ways between settlements are worn into tracks and roads.
    pub fn importance(self) -> f32 {
        match self {
            Kind::Hamlet => 6.0,
            Kind::Village => 8.0,
            Kind::LargeVillage => 10.0,
            Kind::Town => 12.0,
            Kind::GreatTown => 16.0,
        }
    }

    /// About how far from its middle the settlement's buildings reach, in voxels.
    pub fn radius(self) -> f32 {
        match self {
            Kind::Hamlet => m(30.0),
            Kind::Village => m(60.0),
            Kind::LargeVillage => m(80.0),
            Kind::Town => m(100.0),
            Kind::GreatTown => m(120.0),
        }
    }

    /// Radius of the levelled, worn green at its middle, which the buildings will stand
    /// about (stage 2, #202).
    pub fn green(self) -> f32 {
        match self {
            Kind::Hamlet => m(6.0),
            Kind::Village => m(9.0),
            Kind::LargeVillage => m(10.4),
            Kind::Town => m(12.0),
            Kind::GreatTown => m(13.6),
        }
    }

    /// The kind as it is said: "large village".
    pub fn word(self) -> &'static str {
        match self {
            Kind::Hamlet => "hamlet",
            Kind::Village => "village",
            Kind::LargeVillage => "large village",
            Kind::Town => "town",
            Kind::GreatTown => "great town",
        }
    }

    /// One lower-case word for the kind, for tools and console commands.
    pub fn key(self) -> &'static str {
        match self {
            Kind::Hamlet => "hamlet",
            Kind::Village => "village",
            Kind::LargeVillage => "largevillage",
            Kind::Town => "town",
            Kind::GreatTown => "greattown",
        }
    }

    /// The kind a word names, as [`Self::key`] or [`Self::word`] gives it.
    pub fn named(word: &str) -> Option<Kind> {
        let w = word.trim().to_ascii_lowercase().replace([' ', '-', '_'], "");
        Kind::ALL.into_iter().find(|k| k.key() == w)
    }

    /// How the gazetteer files its name: villages of either size share their words, and
    /// a great town has grander ones than a town.
    pub fn place_kind(self) -> PlaceKind {
        match self {
            Kind::Hamlet => PlaceKind::Hamlet,
            Kind::Village | Kind::LargeVillage => PlaceKind::Village,
            Kind::Town => PlaceKind::Town,
            Kind::GreatTown => PlaceKind::GreatTown,
        }
    }

    /// Colour used to mark it on maps.
    pub fn map_color(self) -> [u8; 3] {
        match self {
            Kind::Hamlet => [250, 236, 200],
            Kind::Village => [250, 200, 80],
            Kind::LargeVillage => [250, 160, 50],
            Kind::Town => [235, 80, 50],
            Kind::GreatTown => [200, 30, 120],
        }
    }
}

/// How long a settlement has stood.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Age {
    /// Founded within a lifetime: timber still bright, few graves.
    Young,
    Established,
    /// Founded long ago: an old stone core, and walls if it is a great town.
    Ancient,
}

impl Age {
    pub fn word(self) -> &'static str {
        match self {
            Age::Young => "young",
            Age::Established => "established",
            Age::Ancient => "ancient",
        }
    }
}

/// The water a settlement grew up by. Wells are counted apart: a place by the sea, or
/// one whose stream runs too far off to fetch from, digs them (see [`Settlement::wells`]).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Water {
    River,
    Lake,
    /// The coast, with no fresh water within reach: its wells give the drinking water.
    Sea,
    Spring,
    /// No open water near at all: wells alone.
    Wells,
}

impl Water {
    pub fn word(self) -> &'static str {
        match self {
            Water::River => "river",
            Water::Lake => "lake",
            Water::Sea => "sea",
            Water::Spring => "spring",
            Water::Wells => "wells",
        }
    }

    /// Whether it is fresh water to drink from.
    pub fn fresh(self) -> bool {
        matches!(self, Water::River | Water::Lake | Water::Spring)
    }
}

/// What people do for a living, read from the country about a settlement (see the
/// table in #45).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Livelihood {
    /// Woodland near.
    Woodcutter,
    /// Woodland near and more than 15 buildings: a sawmill, with a water wheel on a stream.
    Sawmill,
    LoggingCamp,
    /// Willows near: baskets.
    Weaver,
    /// Willows near: arrows (#50).
    Fletcher,
    /// A river, a lake or the sea.
    Fisher,
    /// A town on the coast.
    Port,
    /// Dry, open country: hide frames, antler racks.
    Hunter,
    /// Meadow or heath: hives (#172).
    Apiary,
    /// Rock outcrops: quarry and mason.
    Quarry,
    /// Every village: the region's own plants hung to dry.
    Forager,
    /// Large villages and up, with iron brought in by trade.
    Smithy,
}

impl Livelihood {
    pub const ALL: [Livelihood; 12] = [
        Livelihood::Woodcutter,
        Livelihood::Sawmill,
        Livelihood::LoggingCamp,
        Livelihood::Weaver,
        Livelihood::Fletcher,
        Livelihood::Fisher,
        Livelihood::Port,
        Livelihood::Hunter,
        Livelihood::Apiary,
        Livelihood::Quarry,
        Livelihood::Forager,
        Livelihood::Smithy,
    ];

    pub fn word(self) -> &'static str {
        match self {
            Livelihood::Woodcutter => "woodcutter",
            Livelihood::Sawmill => "sawmill",
            Livelihood::LoggingCamp => "logging camp",
            Livelihood::Weaver => "weaver",
            Livelihood::Fletcher => "fletcher",
            Livelihood::Fisher => "fishers",
            Livelihood::Port => "port",
            Livelihood::Hunter => "hunters",
            Livelihood::Apiary => "apiary",
            Livelihood::Quarry => "quarry",
            Livelihood::Forager => "forager",
            Livelihood::Smithy => "smithy",
        }
    }
}

/// Why a settlement stands where it does: the best thing about its spot on the map.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Reason {
    /// On the inside of a river's bend.
    Bend,
    /// Where two rivers meet.
    Confluence,
    /// Where a river can be waded or bridged.
    Ford,
    /// Beside a river.
    River,
    LakeShore,
    Spring,
    /// In the desert, by the only water for miles.
    Oasis,
    /// A sheltered bay, for boats.
    Bay,
    /// On the coast.
    Shore,
    /// In the low gap of a ridge.
    Pass,
    /// Good level ground and nothing better.
    Open,
}

/// One settlement: a few seeded facts, the way a landmark has a `History`. Everything
/// built there (stage 2 on) follows from these and from the country it stands in.
#[derive(Clone, Debug)]
pub struct Settlement {
    pub kind: Kind,
    pub age: Age,
    /// The year it was founded. Year 0 is the world's first year, so every settlement
    /// standing when the world begins was founded in a year before it, a negative one.
    pub founded: i32,
    /// How well it does, 0 (scraping by) to 1 (thriving).
    pub prosperity: f32,
    /// How its people build: the region of the biome at its middle.
    pub region: Region,
    /// Whether the sea is close by.
    pub coast: bool,
    pub water: Water,
    /// How many wells it has dug: about one for every 30 people where fresh water is
    /// further than [`crate::trails::siting::FETCH`] from its middle, and one or two in a
    /// big town by a river, in squares away from it.
    pub wells: u8,
    /// What its people live by, in the order of [`Livelihood::ALL`].
    pub livelihoods: Vec<Livelihood>,
    /// Walk-in buildings and people, within the ranges of its kind.
    pub buildings: u8,
    pub people: u8,
    /// The middle of its green, in continent-local voxels, and the first layer above
    /// the levelled ground there.
    pub x: i32,
    pub z: i32,
    pub y: i32,
    /// About how far its buildings reach from the middle, in voxels.
    pub radius: f32,
    /// What everything about it is chosen by.
    pub seed: u32,
    pub reason: Reason,
    /// The village or town a hamlet grew from, by index in `Network::settlements`.
    pub parent: Option<usize>,
    /// Settlements on the same river with no falls or cascades between, which trade by
    /// boat (#205), by index in `Network::settlements`.
    pub partners: Vec<usize>,
    /// The map cell of the river it stands by, if a boat could use it.
    pub river: Option<usize>,
    /// Landmarks that fell inside it and became part of it, by index in `Network::sites`:
    /// a landing on the coast is its port, and a shrine its shrine.
    pub port: Option<usize>,
    pub shrine: Option<usize>,
}

impl Settlement {
    /// The middle, in continent-local voxels.
    pub fn at(&self) -> Vec2 {
        Vec2::new(self.x as f32 + 0.5, self.z as f32 + 0.5)
    }

    pub fn has(&self, l: Livelihood) -> bool {
        self.livelihoods.contains(&l)
    }

    /// Whether a landmark is part of it.
    pub fn absorbed(&self, site: usize) -> bool {
        self.port == Some(site) || self.shrine == Some(site)
    }

    /// Its key facts in a line, for tools and the console: "24 people in 9 buildings,
    /// established (founded in -212), by a river, temperate; woodcutter, fishers".
    pub fn summary(&self) -> String {
        let water = match self.water {
            Water::Wells => "no water near".to_string(),
            w => format!("by {} {}", if w == Water::Sea { "the" } else { "a" }, w.word()),
        };
        let wells = match self.wells {
            0 => String::new(),
            1 => ", a well".to_string(),
            n => format!(", {n} wells"),
        };
        let trades: Vec<&str> = self.livelihoods.iter().map(|l| l.word()).collect();
        format!(
            "{} people in {} buildings, {} (founded in {}), {water}{wells}, {}, prosperity {:.2}; {}",
            self.people,
            self.buildings,
            self.age.word(),
            self.founded,
            self.region.word(),
            self.prosperity,
            trades.join(", ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_region_reads_back_from_its_own_biome() {
        for r in Region::ALL {
            assert_eq!(Region::of(r.biome()), r, "{r:?}");
        }
    }
}
