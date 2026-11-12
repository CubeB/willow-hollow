//! Biome classification from climate.

use crate::continent::climate::LAPSE;

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
    /// Open, rolling upland of rough grass, heather and peat over granite, its summits
    /// crowned with tors: high, cool and wet ground of low relief.
    Moor,
}

/// The climate and ground a moor takes: temperature, least rainfall, least broad height
/// (in simulation units) and steepest broad slope, and the least of the slow variety noise.
const MOOR_COOL: f32 = 0.26;
const MOOR_WARM: f32 = 0.44;
const MOOR_RAIN: f32 = 0.75;
const MOOR_HEIGHT: f32 = 45.0;
const MOOR_SLOPE: f32 = 0.22;
const MOOR_VARIETY: f32 = -0.35;

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
    /// 0..1, how sheltered the nearby water is: 1 in bays, estuaries and lagoons,
    /// 0 on open coasts and small islands facing the open sea.
    pub shelter: f32,
    /// Slow noise, -1..1, that hands stretches of country to one sub-biome or another.
    pub variety: f32,
    /// 0..1, how much the spot belongs to a volcano.
    pub volcanic: f32,
    /// The mean height and mean slope of the country for a few hundred metres around:
    /// what tells a broad upland of low relief from a ridge between deep valleys.
    pub broad_height: f32,
    pub broad_slope: f32,
}

impl Biome {
    pub const ALL: [Biome; 23] = [
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
        Biome::Moor,
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
        let Site { temperature: t, rain: r, height: h, slope, wet, coast, shelter, variety: v, volcanic, broad_height, broad_slope } = *site;
        let base = Self::classify(t, r);
        if base == Biome::Alpine {
            return base;
        }
        if volcanic > 0.5 {
            return Biome::Volcanic;
        }
        // How waterlogged the ground is: wet ground near big water that is also flat
        // enough to hold it, more so under heavy rain.
        let sodden = wet * (1.0 - slope / 0.08).clamp(0.0, 1.0) * (0.75 + 0.25 * r.min(2.0));
        // Some stretches of country are given over to wetland more than others.
        let wetland = 0.22 - 0.06 * v;
        // The tree line wanders a little from place to place.
        let treeline = 0.19 + 0.02 * v;
        // Cold because it is high, not because it is far north: only there is there a
        // tree line with meadow above it rather than tundra all the way down.
        let alpine = h > 100.0 && t + h * LAPSE > treeline + 0.06;
        // Moor: the cool, wet temperate uplands where the ground lies open and rolling
        // rather than steep, a plateau above the lowland woods and below the mountains.
        // Its valleys are moor too, all but the steepest of their sides.
        let moor = (MOOR_COOL..MOOR_WARM).contains(&t)
            && r > MOOR_RAIN
            && broad_height > MOOR_HEIGHT
            && broad_slope < MOOR_SLOPE
            && h > MOOR_HEIGHT * 0.7
            && slope < MOOR_SLOPE * 2.0
            && v > MOOR_VARIETY;
        match base {
            // Warm, wet, low ground at sheltered coasts (bays, lagoons, estuaries) and
            // the mouths of big rivers; open coasts facing the sea stay beach and forest.
            Biome::Woodland | Biome::Rainforest | Biome::Savanna
                if t > 0.62
                    && r > 0.8
                    && h < 6.0
                    && slope < 0.03
                    && coast > 0.12
                    && ((coast > 0.3 && shelter > 0.5) || wet > 0.7) =>
            {
                Biome::Mangrove
            }
            // Flowering grass from the tree line up to the snow.
            _ if alpine && t < treeline && r > 0.4 && slope < 0.8 => Biome::AlpineMeadow,
            Biome::Tundra | Biome::Taiga
                if t > 0.13 && (sodden > wetland * 0.9 || (r > 1.3 && slope < 0.015)) =>
            {
                Biome::Bog
            }
            Biome::Taiga if r > 1.05 && v > 0.05 => Biome::SnowyPine,
            Biome::Grassland | Biome::Deciduous | Biome::OldGrowth | Biome::Heath | Biome::Woodland
                if sodden > wetland && h < 150.0 && !moor =>
            {
                Biome::Marsh
            }
            Biome::Taiga | Biome::Deciduous | Biome::Heath if (0.22..0.40).contains(&t) && r > 0.5 && v > 0.32 => {
                Biome::BirchGrove
            }
            // Its waterlogged hollows are blanket bog and valley mire.
            Biome::Grassland | Biome::Deciduous if moor => {
                if sodden > wetland * 0.8 { Biome::Bog } else { Biome::Moor }
            }
            Biome::Heath | Biome::Grassland if t > 0.40 && r < 0.62 && v < -0.1 => Biome::Steppe,
            Biome::Desert | Biome::Savanna if h > 70.0 && v < -0.05 && r < 0.6 => Biome::RedRock,
            Biome::Desert if h < 80.0 && v > -0.1 => Biome::DuneSea,
            b => b,
        }
    }

    /// Dry country, whose streams run between banks of bare sand or rock with no reeds.
    pub fn arid(self) -> bool {
        matches!(self, Biome::Desert | Biome::DuneSea | Biome::RedRock | Biome::Volcanic)
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
            Biome::Moor => Biome::Heath,
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
            Biome::Moor => [132, 118, 100],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(temperature: f32, rain: f32, height: f32, slope: f32) -> Site {
        Site {
            temperature,
            rain,
            height,
            slope,
            wet: 0.0,
            coast: 0.0,
            shelter: 0.0,
            variety: 0.0,
            volcanic: 0.0,
            broad_height: height,
            broad_slope: slope,
        }
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
            (Site { coast: 1.0, shelter: 1.0, ..site(0.80, 1.6, 2.0, 0.01) }, Biome::Mangrove),
            (Site { coast: 0.5, wet: 0.9, ..site(0.80, 1.6, 2.0, 0.01) }, Biome::Mangrove),
            (Site { volcanic: 1.0, ..site(0.60, 1.0, 100.0, 0.2) }, Biome::Volcanic),
            (site(0.36, 1.1, 80.0, 0.1), Biome::Moor),
            (Site { wet: 0.9, ..site(0.36, 1.1, 80.0, 0.01) }, Biome::Bog),
        ];
        for (s, want) in cases {
            assert_eq!(Biome::select(&s), want, "{s:?}");
        }
        // An open coast facing the sea, or a floodplain far inland, is no mangrove.
        assert_ne!(Biome::select(&Site { coast: 1.0, ..site(0.80, 1.6, 2.0, 0.01) }), Biome::Mangrove);
        assert_ne!(Biome::select(&Site { wet: 0.9, ..site(0.80, 1.6, 2.0, 0.01) }), Biome::Mangrove);
        // Cold uplands in the far north are tundra or taiga, not meadow under a tree line.
        assert_ne!(Biome::select(&site(0.11, 0.9, 105.0, 0.2)), Biome::AlpineMeadow);
        // Climate alone still decides ordinary country.
        assert_eq!(Biome::select(&site(0.45, 1.0, 50.0, 0.1)), Biome::Deciduous);
        assert_eq!(Biome::select(&site(0.05, 1.0, 900.0, 0.3)), Biome::Alpine);
        // Steep upland, or low ground, in the same climate is forest, not moor.
        assert_eq!(Biome::select(&site(0.36, 1.1, 80.0, 0.5)), Biome::Deciduous);
        assert_eq!(Biome::select(&site(0.36, 1.1, 20.0, 0.1)), Biome::Deciduous);
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
