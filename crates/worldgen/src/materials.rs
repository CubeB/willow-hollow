//! The voxel palette: every block id the generator can emit.

use voxel::{Kind, Material, Materials, Voxel};

use crate::biome::Biome;

macro_rules! palette {
    (@glow) => { true };
    ($( $name:ident = ($r:expr, $g:expr, $b:expr), $kind:ident, $jitter:expr $(, $glow:ident)?; )*) => {
        /// Voxel ids, in the order they are registered by [`build`].
        #[allow(non_camel_case_types, dead_code, clippy::upper_case_acronyms)]
        #[repr(u16)]
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        enum Id { AIR = 0, $( $name, )* }

        $( pub const $name: Voxel = Id::$name as Voxel; )*

        pub fn build() -> Materials {
            let mut m = Materials::new();
            $(
                let id = m.add(Material { color: [$r, $g, $b], kind: Kind::$kind, jitter: $jitter, emissive: false $(|| palette!(@$glow))? });
                debug_assert_eq!(id, $name);
            )*
            m
        }
    };
}

palette! {
    // Ground cover, one per climate.
    GRASS          = ( 96, 150,  62), Solid, 2;
    GRASS_LUSH     = ( 66, 132,  58), Solid, 2;
    GRASS_MEADOW   = (134, 172,  78), Solid, 2;
    GRASS_DRY      = (156, 150,  94), Solid, 2;
    GRASS_SAVANNA  = (188, 172,  96), Solid, 2;
    GRASS_WOODLAND = (118, 152,  68), Solid, 2;
    GRASS_JUNGLE   = ( 52, 128,  56), Solid, 2;
    GRASS_TAIGA    = ( 86, 120,  80), Solid, 2;
    GRASS_TUNDRA   = (146, 148, 108), Solid, 2;
    // Earth and rock.
    DIRT           = (122,  88,  58), Solid, 2;
    MUD            = ( 98,  80,  60), Solid, 1;
    STONE          = (128, 128, 126), Solid, 2;
    ROCK_DARK      = ( 96,  94,  98), Solid, 2;
    SAND           = (222, 204, 150), Solid, 1;
    DESERT_SAND    = (226, 196, 138), Solid, 1;
    GRAVEL         = (146, 140, 132), Solid, 2;
    SNOW           = (238, 242, 248), Solid, 1;
    WATER          = ( 46, 112, 168), Water, 0;
    // Wood.
    BARK_OAK       = ( 94,  72,  50), Solid, 2;
    BARK_BIRCH     = (228, 226, 216), Solid, 3;
    BARK_PINE      = ( 86,  62,  46), Solid, 2;
    BARK_REDWOOD   = (138,  74,  50), Solid, 2;
    BARK_JUNGLE    = (108,  94,  74), Solid, 2;
    BARK_PALM      = (152, 126,  90), Solid, 2;
    BARK_WILLOW    = (104,  90,  66), Solid, 2;
    BARK_ACACIA    = (120,  96,  70), Solid, 2;
    BARK_BAOBAB    = (150, 124, 100), Solid, 1;
    WOOD_DEAD      = (156, 146, 130), Solid, 2;
    // Foliage: walked through, not into.
    LEAVES_OAK     = ( 66, 130,  48), Foliage, 3;
    LEAVES_BIRCH   = (124, 172,  62), Foliage, 3;
    NEEDLES        = ( 40,  90,  60), Foliage, 3;
    NEEDLES_REDWOOD= ( 50, 104,  60), Foliage, 3;
    LEAVES_JUNGLE  = ( 42, 122,  52), Foliage, 3;
    LEAVES_ACACIA  = (112, 142,  62), Foliage, 3;
    LEAVES_WILLOW  = (120, 162,  76), Foliage, 3;
    LEAVES_PALM    = ( 76, 142,  62), Foliage, 3;
    LEAVES_MAPLE   = (216, 122,  42), Foliage, 3;
    LEAVES_MAPLE_RED = (192,  62,  46), Foliage, 3;
    LEAVES_CHERRY  = (242, 174, 198), Foliage, 3;
    LEAVES_BAOBAB  = ( 98, 136,  58), Foliage, 3;
    VINE           = ( 54, 112,  50), Foliage, 2;
    MOSS           = ( 84, 124,  54), Solid, 2;
    CACTUS         = ( 78, 134,  70), Solid, 2;
    // Flowering ground: grass tops carrying a drift of colour.
    FLOWER_RED     = (206,  78,  72), Solid, 2;
    FLOWER_YELLOW  = (232, 206,  86), Solid, 2;
    FLOWER_WHITE   = (236, 236, 226), Solid, 1;
    FLOWER_PURPLE  = (156, 104, 188), Solid, 2;
    FLOWER_BLUE    = (104, 132, 214), Solid, 2;
    FLOWER_PINK    = (232, 158, 190), Solid, 2;
    HEATHER        = (164, 112, 160), Solid, 2;
    // Trodden ground.
    PATH_DIRT      = (142, 112,  78), Solid, 2;
    PATH_GRAVEL    = (164, 154, 138), Solid, 2;
    PATH_SAND      = (206, 178, 128), Solid, 1;
    PATH_DARK      = (104,  86,  64), Solid, 2;
    // Things people built.
    SANDSTONE      = (204, 170, 122), Solid, 2;
    SANDSTONE_DARK = (168, 132,  92), Solid, 2;
    MOSSY_STONE    = (106, 128,  92), Solid, 2;
    PLANK          = (160, 124,  82), Solid, 2;
    PLANK_PALE     = (190, 160, 116), Solid, 2;
    PLANK_DARK     = (102,  78,  58), Solid, 2;
    PLANK_RED      = (146,  84,  60), Solid, 2;
    THATCH         = (182, 156,  92), Solid, 3;
    SLATE          = ( 84,  90, 104), Solid, 2;
    ROOF_TILE      = (164,  84,  62), Solid, 2;
    CLOTH_RED      = (178,  62,  58), Solid, 1;
    CLOTH_BLUE     = ( 66,  98, 164), Solid, 1;
    CLOTH_PALE     = (216, 204, 176), Solid, 1;
    CHARCOAL       = ( 52,  48,  48), Solid, 1;
    LANTERN        = (255, 198, 112), Solid, 0, glow;
    EMBER          = (255, 122,  46), Solid, 1, glow;
    // Small plants: walked through, like leaves.
    TUFT           = (104, 160,  66), Foliage, 3;
    TUFT_MEADOW    = (146, 184,  84), Foliage, 3;
    TUFT_LUSH      = ( 60, 142,  62), Foliage, 3;
    TUFT_DRY       = (172, 160,  98), Foliage, 3;
    TUFT_STRAW     = (200, 184, 118), Foliage, 3;
    TUFT_ALPINE    = (132, 150,  98), Foliage, 3;
    TUFT_SEDGE     = (112, 158, 102), Foliage, 3;
    SEED_HEAD      = (210, 194, 136), Foliage, 2;
    BLOOM_RED      = (222,  70,  64), Foliage, 2;
    BLOOM_YELLOW   = (244, 214,  80), Foliage, 2;
    BLOOM_WHITE    = (244, 244, 234), Foliage, 1;
    BLOOM_PURPLE   = (170, 110, 204), Foliage, 2;
    BLOOM_BLUE     = (108, 140, 230), Foliage, 2;
    BLOOM_PINK     = (242, 160, 198), Foliage, 2;
    REED           = (120, 148,  72), Foliage, 3;
    REED_HEAD      = (112,  84,  54), Foliage, 2;
    MUSHROOM_STEM  = (226, 216, 196), Foliage, 1;
    MUSHROOM_RED   = (196,  56,  48), Foliage, 2;
    MUSHROOM_TAN   = (178, 142, 100), Foliage, 2;
    LILY           = ( 70, 138,  64), Foliage, 2;
    // Moss and lichen: fine pieces on trunks, stone and shaded ground.
    MOSS_CUSHION      = ( 74, 118,  52), Foliage, 3;
    MOSS_CUSHION_LIT  = (110, 152,  66), Foliage, 3;
    MOSS_CUSHION_DEEP = ( 58,  98,  52), Foliage, 3;
    MOSS_CARPET       = ( 92, 132,  56), Foliage, 3;
    MOSS_CARPET_LIT   = (124, 158,  70), Foliage, 3;
    MOSS_CARPET_DEEP  = ( 68, 110,  48), Foliage, 3;
    MOSS_SHAG         = ( 84, 112,  58), Foliage, 3;
    MOSS_SHAG_TIP     = (132, 154,  84), Foliage, 3;
    LICHEN_YELLOW     = (176, 176,  84), Foliage, 2;
    LICHEN_GREY       = (156, 164, 142), Foliage, 2;
    // Climbers: ivy, Virginia creeper, flowering vines and lianas.
    IVY_STEM          = (122, 116,  70), Foliage, 2;
    IVY_WOOD          = (124, 106,  76), Foliage, 2;
    IVY_LEAF_DARK     = ( 48, 110,  52), Foliage, 3;
    IVY_LEAF          = ( 72, 136,  58), Foliage, 3;
    IVY_LEAF_PALE     = (136, 180,  80), Foliage, 3;
    IVY_LEAF_RED      = (176,  74,  52), Foliage, 3;
    IVY_FLOWER        = (206, 208, 120), Foliage, 2;
    IVY_BERRY_GREEN   = (110, 134,  74), Foliage, 1;
    IVY_BERRY         = ( 40,  40,  66), Foliage, 1;
    CREEPER_LEAF_DARK = ( 60, 104,  48), Foliage, 3;
    CREEPER_LEAF      = ( 92, 132,  54), Foliage, 3;
    CREEPER_LEAF_RED  = (186,  60,  46), Foliage, 3;
    VINE_LEAF         = ( 72, 128,  58), Foliage, 3;
    VINE_FLOWER       = (232, 142, 170), Foliage, 2;
    LIANA_STEM        = (116, 100,  66), Foliage, 2;
}

/// The ground cover a biome grows.
pub fn ground_cover(biome: Biome) -> Voxel {
    match biome {
        Biome::Ocean => SAND,
        Biome::Alpine => SNOW,
        Biome::Tundra => GRASS_TUNDRA,
        Biome::Taiga => GRASS_TAIGA,
        Biome::Heath => GRASS_DRY,
        Biome::Grassland => GRASS_MEADOW,
        Biome::Deciduous => GRASS,
        Biome::OldGrowth => GRASS_LUSH,
        Biome::Desert => DESERT_SAND,
        Biome::Savanna => GRASS_SAVANNA,
        Biome::Woodland => GRASS_WOODLAND,
        Biome::Rainforest => GRASS_JUNGLE,
    }
}
