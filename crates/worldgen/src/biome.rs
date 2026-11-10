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
    /// Flat, waterlogged temperate ground beside big rivers and lakes: sedge, reeds and
    /// willows.
    Marsh,
    /// Cool forest given over almost wholly to birch.
    BirchGrove,
    /// Flowering grass just below the snow, above the trees.
    AlpineMeadow,
    /// Dense spruce standing in snow on wet cold slopes.
    SnowyPine,
    /// Flat, sodden boreal ground of moss and drowned trees.
    Bog,
    /// Dry temperate grass with lone trees.
    Steppe,
    /// Open sand in the driest hot country, heaped into dunes.
    DuneSea,
    /// Red sandstone uplands in hot dry country.
    RedRock,
    /// Mud flats and mangrove at warm, wet, low coasts and river mouths.
    Mangrove,
    /// Black ash and basalt on volcanic islands.
    Volcanic,
}

/// What, beyond climate, decides a biome at a spot.
#[derive(Clone, Copy, Debug)]
pub struct Site {
    /// 0 = polar, 1 = tropical, already lowered for altitude.
    pub temperature: f32,
    /// Relative rainfall, 1 = the continent's average.
    pub rain: f32,
    /// Height above sea level in simulation units (0.8 m).
    pub height: f32,
    /// Rise over run.
    pub slope: f32,
    /// How waterlogged the ground is, 0..1: high on flat land beside big rivers and lakes.
    pub wet: f32,
    /// 1 at the coast, falling to 0 a little way inland.
    pub coast: f32,
    /// Slow noise, -1..1, that hands stretches of country to one sub-biome or another.
    pub variety: f32,
    /// 0..1, how much the spot belongs to a volcano.
    pub volcanic: f32,
}

impl Biome {
    pub const ALL: [Biome; 22] = [
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
        Biome::Marsh,
        Biome::BirchGrove,
        Biome::AlpineMeadow,
        Biome::SnowyPine,
        Biome::Bog,
        Biome::Steppe,
        Biome::DuneSea,
        Biome::RedRock,
        Biome::Mangrove,
        Biome::Volcanic,
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

    /// Picks the biome for a spot: the climate decides the broad type, and the ground
    /// and a slow noise then choose among the kinds of country that climate allows.
    pub fn select(site: &Site) -> Biome {
        let Site { temperature: t, rain: r, height: h, slope, wet, coast, variety: v, volcanic } = *site;
        let base = Self::classify(t, r);
        if base == Biome::Alpine {
            return base;
        }
        if volcanic > 0.5 {
            return Biome::Volcanic;
        }
        let flat = slope < 0.045;
        match base {
            // Warm, wet, low coasts and the mouths of rivers.
            Biome::Woodland | Biome::Rainforest | Biome::Savanna
                if t > 0.62 && r > 0.7 && h < 7.0 && slope < 0.04 && (coast > 0.3 || wet > 0.6) =>
            {
                Biome::Mangrove
            }
            // Flowering grass from the tree line up to the snow.
            _ if t < 0.155 && h > 100.0 && r > 0.45 && slope < 0.5 => Biome::AlpineMeadow,
            Biome::Tundra | Biome::Taiga if flat && (wet > 0.3 || (r > 1.3 && slope < 0.02)) && t > 0.14 => Biome::Bog,
            Biome::Taiga if r > 1.05 && v > 0.05 => Biome::SnowyPine,
            Biome::Grassland | Biome::Deciduous | Biome::OldGrowth | Biome::Heath | Biome::Woodland
                if flat && wet > 0.4 && h < 120.0 =>
            {
                Biome::Marsh
            }
            Biome::Taiga | Biome::Deciduous | Biome::Heath if (0.22..0.40).contains(&t) && r > 0.5 && v > 0.32 => {
                Biome::BirchGrove
            }
            Biome::Heath | Biome::Grassland if t > 0.40 && r < 0.62 && v < -0.1 => Biome::Steppe,
            Biome::Desert | Biome::Savanna if h > 70.0 && v < -0.05 && r < 0.6 => Biome::RedRock,
            Biome::Desert if h < 80.0 && v > -0.1 => Biome::DuneSea,
            b => b,
        }
    }

    /// The original biome a newer one is closest to, for things that have not yet been
    /// given their own variant for it.
    pub fn parent(self) -> Biome {
        match self {
            Biome::Marsh => Biome::Grassland,
            Biome::BirchGrove => Biome::Deciduous,
            Biome::AlpineMeadow => Biome::Tundra,
            Biome::SnowyPine => Biome::Taiga,
            Biome::Bog => Biome::Taiga,
            Biome::Steppe => Biome::Heath,
            Biome::DuneSea | Biome::RedRock => Biome::Desert,
            Biome::Mangrove => Biome::Rainforest,
            Biome::Volcanic => Biome::Tundra,
            b => b,
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
            Biome::Marsh => [92, 150, 128],
            Biome::BirchGrove => [150, 190, 100],
            Biome::AlpineMeadow => [176, 150, 200],
            Biome::SnowyPine => [150, 180, 176],
            Biome::Bog => [140, 98, 84],
            Biome::Steppe => [204, 190, 128],
            Biome::DuneSea => [240, 170, 96],
            Biome::RedRock => [180, 86, 58],
            Biome::Mangrove => [60, 96, 70],
            Biome::Volcanic => [62, 58, 62],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(temperature: f32, rain: f32, height: f32, slope: f32) -> Site {
        Site { temperature, rain, height, slope, wet: 0.0, coast: 0.0, variety: 0.0, volcanic: 0.0 }
    }

    #[test]
    fn every_new_biome_has_ground_that_selects_it() {
        let cases = [
            (Site { wet: 0.8, ..site(0.45, 1.0, 20.0, 0.01) }, Biome::Marsh),
            (Site { variety: 0.6, ..site(0.30, 1.0, 60.0, 0.1) }, Biome::BirchGrove),
            (site(0.13, 0.9, 400.0, 0.2), Biome::AlpineMeadow),
            (Site { variety: 0.4, ..site(0.20, 1.3, 200.0, 0.2) }, Biome::SnowyPine),
            (Site { wet: 0.6, ..site(0.18, 0.9, 30.0, 0.01) }, Biome::Bog),
            (Site { variety: -0.5, ..site(0.50, 0.3, 40.0, 0.05) }, Biome::Steppe),
            (Site { variety: 0.5, ..site(0.80, 0.1, 30.0, 0.02) }, Biome::DuneSea),
            (Site { variety: -0.5, ..site(0.80, 0.2, 150.0, 0.2) }, Biome::RedRock),
            (Site { coast: 1.0, ..site(0.80, 1.6, 2.0, 0.01) }, Biome::Mangrove),
            (Site { volcanic: 1.0, ..site(0.60, 1.0, 100.0, 0.2) }, Biome::Volcanic),
        ];
        for (s, want) in cases {
            assert_eq!(Biome::select(&s), want, "{s:?}");
        }
        // Climate alone still decides ordinary country.
        assert_eq!(Biome::select(&site(0.45, 1.0, 50.0, 0.1)), Biome::Deciduous);
        assert_eq!(Biome::select(&site(0.05, 1.0, 900.0, 0.3)), Biome::Alpine);
    }

    #[test]
    fn biomes_keep_their_ids_and_new_ones_fall_back_to_old() {
        for (i, b) in Biome::ALL.iter().enumerate() {
            assert_eq!(*b as usize, i);
            assert_eq!(Biome::from_u8(i as u8), *b);
            assert!(Biome::ALL[..12].contains(&b.parent()));
        }
        assert_eq!(Biome::Rainforest as u8, 11);
    }
}
