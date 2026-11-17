//! Settlements: hamlets, villages and towns, where people live (see #45 for the plan).
//!
//! A settlement is decided on the continent map, before the landmarks and trails, from a
//! few seeded facts (its kind, age, prosperity, water and livelihoods); everything built
//! there follows from those facts and from the country it stands in.

use crate::biome::Biome;

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
