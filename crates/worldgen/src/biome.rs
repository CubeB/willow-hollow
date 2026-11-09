//! Biome classification from climate.

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[repr(u8)]
pub enum Biome {
    Ocean,
    /// Permanent snow and bare rock above the tree line.
    Alpine,
    Tundra,
    /// Cold conifer forest.
    Taiga,
    /// Dry cool grass and heather.
    Heath,
    Grassland,
    /// Temperate broadleaf forest.
    Deciduous,
    /// Wet temperate forest of giant trees.
    OldGrowth,
    Desert,
    Savanna,
    /// Warm seasonal woodland.
    Woodland,
    Rainforest,
}

impl Biome {
    pub const ALL: [Biome; 12] = [
        Biome::Ocean,
        Biome::Alpine,
        Biome::Tundra,
        Biome::Taiga,
        Biome::Heath,
        Biome::Grassland,
        Biome::Deciduous,
        Biome::OldGrowth,
        Biome::Desert,
        Biome::Savanna,
        Biome::Woodland,
        Biome::Rainforest,
    ];

    pub fn from_u8(v: u8) -> Biome {
        Self::ALL[(v as usize).min(Self::ALL.len() - 1)]
    }

    /// Whittaker-style lookup.
    ///
    /// `temperature`: 0 = polar, 1 = tropical, already lowered for altitude.
    /// `rain`: relative rainfall, 1 = the continent's average.
    pub fn classify(temperature: f32, rain: f32) -> Biome {
        let (t, r) = (temperature, rain);
        if t < 0.10 {
            Biome::Alpine
        } else if t < 0.24 {
            if r < 0.55 { Biome::Tundra } else { Biome::Taiga }
        } else if t < 0.58 {
            match r {
                r if r < 0.40 => Biome::Heath,
                r if r < 0.85 => Biome::Grassland,
                r if r < 1.55 => Biome::Deciduous,
                _ => Biome::OldGrowth,
            }
        } else {
            match r {
                r if r < 0.35 => Biome::Desert,
                r if r < 0.80 => Biome::Savanna,
                r if r < 1.45 => Biome::Woodland,
                _ => Biome::Rainforest,
            }
        }
    }

    /// Representative colour for maps (sRGB).
    pub fn map_color(self) -> [u8; 3] {
        match self {
            Biome::Ocean => [40, 90, 150],
            Biome::Alpine => [236, 238, 242],
            Biome::Tundra => [158, 162, 132],
            Biome::Taiga => [62, 104, 82],
            Biome::Heath => [150, 146, 104],
            Biome::Grassland => [142, 176, 96],
            Biome::Deciduous => [88, 146, 72],
            Biome::OldGrowth => [48, 108, 64],
            Biome::Desert => [220, 196, 138],
            Biome::Savanna => [186, 176, 98],
            Biome::Woodland => [116, 156, 70],
            Biome::Rainforest => [34, 118, 58],
        }
    }
}
