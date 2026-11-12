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
                let id = m.add(Material { color: [$r, $g, $b], kind: Kind::$kind, jitter: $jitter, emissive: false $(|| palette!(@$glow))?, plant: voxel::plant::NONE });
                debug_assert_eq!(id, $name);
            )*
            classify(&mut m);
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
    // Cliff strata and the scree at their feet (added last so existing ids keep their numbers).
    ROCK_PALE      = (172, 166, 154), Solid, 2;
    ROCK_BROWN     = (112, 102,  94), Solid, 2;
    SANDSTONE_RED  = (188, 112,  78), Solid, 2;
    SCREE          = (140, 134, 128), Solid, 3;
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
    IVY_STEM          = (100,  92,  64), Foliage, 2;
    IVY_WOOD          = (126, 116, 100), Foliage, 2;
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
    LIANA_STEM        = ( 96,  82,  62), Foliage, 2;
    // Ground of the rarer biomes.
    GRASS_MARSH    = ( 84, 126,  92), Solid, 2;
    GRASS_BIRCH    = (128, 166,  82), Solid, 2;
    GRASS_ALPINE   = (104, 156, 104), Solid, 2;
    BOG_MOSS       = (138, 100,  78), Solid, 2;
    GRASS_STEPPE   = (168, 166, 100), Solid, 2;
    DUNE_SAND      = (232, 180, 116), Solid, 1;
    RED_SAND       = (196, 118,  76), Solid, 2;
    RED_ROCK       = (172,  86,  58), Solid, 2;
    RED_ROCK_DARK  = (136,  66,  50), Solid, 2;
    MANGROVE_MUD   = ( 88,  80,  60), Solid, 1;
    ASH            = ( 74,  70,  70), Solid, 2;
    BASALT         = ( 52,  50,  56), Solid, 2;
    // Dark streaks in old climber wood, where the stems twist round each other.
    IVY_WOOD_DARK     = ( 98,  88,  78), Foliage, 2;
    LIANA_WOOD        = ( 78,  66,  52), Foliage, 2;
    // Tree detail: bark relief on trunks and the twigs on limbs (features::tree_detail).
    BARK_OAK_RIDGE    = (112,  88,  62), Foliage, 2;
    BARK_REDWOOD_RIDGE = (160,  92,  64), Foliage, 2;
    BARK_PINE_PLATE   = (106,  80,  60), Foliage, 2;
    BARK_BIRCH_MARK   = ( 72,  70,  66), Foliage, 1;
    BARK_BIRCH_PEEL   = (212, 186, 152), Foliage, 2;
    BARK_PALM_RING    = (128, 104,  74), Foliage, 2;
    TWIG              = ( 82,  64,  48), Foliage, 2;
    TWIG_PALE         = (164, 156, 144), Foliage, 2;
    // Climber leaves and stems that read as leaves and lines, not moss: ivy a dark,
    // bluish, glossy green well apart from moss's yellow greens, and little per-voxel
    // jitter, so a leaf is one colour and its neighbours hardly jump.
    IVY_GLOSS         = ( 42,  94,  52), Foliage, 1;
    IVY_GLOSS_LIT     = ( 56, 114,  60), Foliage, 1;
    IVY_GLOSS_NEW     = ( 70, 118,  64), Foliage, 1;
    CREEPER_GREEN     = ( 82, 122,  50), Foliage, 1;
    CREEPER_GREEN_DARK = ( 62, 102,  46), Foliage, 1;
    CREEPER_SCARLET   = (172,  56,  42), Foliage, 1;
    VINE_GREEN        = ( 64, 114,  56), Foliage, 1;
    IVY_TWIG          = ( 90,  80,  58), Foliage, 1;
    IVY_ROPE          = (114, 102,  84), Foliage, 1;
    IVY_ROPE_DARK     = ( 96,  86,  74), Foliage, 1;
    // Volcanic beaches: basalt ground to black sand by the surf.
    BLACK_SAND        = ( 46,  44,  48), Solid, 1;
    // The props left about landmarks (features::props): iron fittings and the rust they
    // turn to, rope new and rotten, fired clay, bottle glass and sackcloth.
    IRON              = ( 64,  62,  64), Solid, 1;
    RUST              = (126,  72,  46), Solid, 2;
    ROPE              = (172, 148, 102), Solid, 2;
    ROPE_ROTTEN       = (110,  98,  78), Solid, 2;
    CLAY              = (170, 100,  68), Solid, 1;
    GLASS_GREEN       = ( 62, 116,  90), Solid, 0;
    SACKCLOTH         = (180, 160, 120), Solid, 2;
    // Caves (features::cave): calcite flowstone, and every rock a cave shows in three
    // steps darker, for the light falling away from the entrance. The renderer has no
    // sky occlusion, so the dark is in the colour.
    FLOWSTONE         = (206, 192, 164), Solid, 2;
    CAVE1_STONE       = ( 95,  95,  93), Solid, 2;
    CAVE1_ROCK_DARK   = ( 71,  70,  73), Solid, 2;
    CAVE1_ROCK_PALE   = (127, 123, 114), Solid, 2;
    CAVE1_ROCK_BROWN  = ( 83,  75,  70), Solid, 2;
    CAVE1_SANDSTONE   = (151, 126,  90), Solid, 2;
    CAVE1_SANDSTONE_RED= (139,  83,  58), Solid, 2;
    CAVE1_SANDSTONE_DARK= (124,  98,  68), Solid, 2;
    CAVE1_GRAVEL      = (108, 104,  98), Solid, 2;
    CAVE1_FLOWSTONE   = (152, 142, 121), Solid, 2;
    CAVE2_STONE       = ( 70,  70,  69), Solid, 2;
    CAVE2_ROCK_DARK   = ( 53,  52,  54), Solid, 2;
    CAVE2_ROCK_PALE   = ( 95,  91,  85), Solid, 2;
    CAVE2_ROCK_BROWN  = ( 62,  56,  52), Solid, 2;
    CAVE2_SANDSTONE   = (112,  94,  67), Solid, 2;
    CAVE2_SANDSTONE_RED= (103,  62,  43), Solid, 2;
    CAVE2_SANDSTONE_DARK= ( 92,  73,  51), Solid, 2;
    CAVE2_GRAVEL      = ( 80,  77,  73), Solid, 2;
    CAVE2_FLOWSTONE   = (113, 106,  90), Solid, 2;
    CAVE3_STONE       = ( 51,  51,  50), Solid, 2;
    CAVE3_ROCK_DARK   = ( 38,  38,  39), Solid, 2;
    CAVE3_ROCK_PALE   = ( 69,  66,  62), Solid, 2;
    CAVE3_ROCK_BROWN  = ( 45,  41,  38), Solid, 2;
    CAVE3_SANDSTONE   = ( 82,  68,  49), Solid, 2;
    CAVE3_SANDSTONE_RED= ( 75,  45,  31), Solid, 2;
    CAVE3_SANDSTONE_DARK= ( 67,  53,  37), Solid, 2;
    CAVE3_GRAVEL      = ( 58,  56,  53), Solid, 2;
    CAVE3_FLOWSTONE   = ( 82,  77,  66), Solid, 2;
    // Evergreens (added last so existing ids keep their numbers): the leaves of trees and
    // shrubs that stay on through the winter, and their barks. Every leaf here is dark
    // and on the blue side of green, which is how the far view tells an evergreen from a
    // broadleaf by its colour alone (season.wgsl's `season_evergreen`).
    LEAVES_HOLLY      = ( 30,  78,  50), Foliage, 2;
    LEAVES_YEW        = ( 36,  74,  50), Foliage, 2;
    LEAVES_HOLM       = ( 70,  98,  68), Foliage, 3;
    NEEDLES_PINE      = ( 52,  98,  66), Foliage, 3;
    NEEDLES_JUNIPER   = ( 82, 114,  96), Foliage, 3;
    LEAVES_LAUREL     = ( 44,  98,  60), Foliage, 2;
    GORSE             = ( 62,  92,  56), Foliage, 3;
    BARK_HOLLY        = (126, 124, 112), Solid, 1;
    BARK_YEW          = (120,  76,  58), Solid, 2;
    BARK_SCOTS        = (164, 108,  76), Solid, 2;
    BARK_YEW_FLAKE    = (146,  92,  70), Foliage, 2;
    // Their fine pieces: holly berries and gorse flowers among the leaves.
    HOLLY_BERRY       = (204,  40,  36), Foliage, 1;
    GORSE_BLOOM       = (240, 204,  56), Foliage, 1;
    // Ground plants that stay green: fern fronds, wood-rush, and heather with the rust
    // of its spent flowers.
    FERN              = ( 46, 104,  58), Foliage, 2;
    FERN_TIP          = ( 70, 128,  70), Foliage, 2;
    WOODRUSH          = ( 50,  96,  56), Foliage, 3;
    HEATHER_GREEN     = ( 74,  92,  64), Foliage, 3;
    HEATHER_RUST      = (122,  82,  78), Foliage, 2;
}

/// Thickness of one rock layer, in voxels.
const STRATA_THICKNESS: i32 = 4;

/// The rock a cliff shows at height `y`: horizontal bands that change with elevation.
/// `tilt` shifts the bands a little from place to place so they gently dip, and
/// neighbouring columns with the same `tilt` agree on every layer.
pub fn strata(seed: u32, biome: Biome, y: i32, tilt: f32) -> Voxel {
    use crate::noise::{hash2, unit};
    let layer = |k: i32| unit(hash2(seed ^ 0x57a7, k, 3));
    let band = ((y as f32 + tilt) / STRATA_THICKNESS as f32).floor() as i32;
    // Some layers are as thick as two, so thickness varies.
    let k = if layer(band) < 0.4 { band - 1 } else { band };
    let r = unit(hash2(seed ^ 0x57a8, k, 7));
    // Red rock country is red through and through, in the rock its slopes show (`bedrock`).
    if biome == Biome::RedRock {
        return match r {
            ..0.40 => RED_ROCK,
            ..0.65 => SANDSTONE_RED,
            ..0.85 => RED_ROCK_DARK,
            _ => SANDSTONE,
        };
    }
    // The warm dry country (the dune sea with the rest of the desert) shows sandstone.
    let dry = matches!(biome, Biome::Desert | Biome::DuneSea | Biome::Savanna | Biome::Heath);
    if dry {
        match r {
            ..0.30 => SANDSTONE,
            ..0.55 => SANDSTONE_RED,
            ..0.80 => SANDSTONE_DARK,
            _ => ROCK_PALE,
        }
    } else {
        match r {
            ..0.30 => STONE,
            ..0.52 => ROCK_DARK,
            ..0.75 => ROCK_PALE,
            _ => ROCK_BROWN,
        }
    }
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
        Biome::Marsh => GRASS_MARSH,
        Biome::BirchGrove => GRASS_BIRCH,
        Biome::AlpineMeadow => GRASS_ALPINE,
        Biome::SnowyPine => SNOW,
        Biome::Bog => BOG_MOSS,
        Biome::Steppe => GRASS_STEPPE,
        Biome::DuneSea => DUNE_SAND,
        Biome::RedRock => RED_SAND,
        Biome::Mangrove => MANGROVE_MUD,
        Biome::Volcanic => ASH,
    }
}

/// Bare rock where the ground is too steep for soil: the usual rock and a darker one.
pub fn bedrock(biome: Biome) -> (Voxel, Voxel) {
    match biome {
        Biome::RedRock => (RED_ROCK, RED_ROCK_DARK),
        Biome::DuneSea | Biome::Desert => (SANDSTONE, SANDSTONE_DARK),
        Biome::Volcanic => (BASALT, ROCK_DARK),
        _ => (STONE, ROCK_DARK),
    }
}

/// What lies a little beneath a biome's ground cover.
pub fn subsoil(cover: Voxel) -> Voxel {
    match cover {
        DESERT_SAND | DUNE_SAND => cover,
        RED_SAND => RED_ROCK,
        ASH => BASALT,
        MANGROVE_MUD | BOG_MOSS => MUD,
        _ => DIRT,
    }
}

/// The rock `v` as it looks `tier` steps (0 to 3) into the dark of a cave: itself at
/// the entrance, then the darker forms of the nearest strata rock.
pub fn cave_rock(v: Voxel, tier: u8) -> Voxel {
    if tier == 0 {
        return v;
    }
    let k = match v {
        ROCK_DARK | SLATE | BASALT => 1,
        ROCK_PALE => 2,
        ROCK_BROWN => 3,
        SANDSTONE => 4,
        SANDSTONE_RED | RED_ROCK | RED_ROCK_DARK => 5,
        SANDSTONE_DARK => 6,
        GRAVEL | SCREE => 7,
        FLOWSTONE => 8,
        _ => 0,
    };
    // The tiers are registered in order, nine rocks to a tier.
    CAVE1_STONE + (tier.min(3) as Voxel - 1) * 9 + k
}

/// Tells the seasons what each plant material is (see `voxel::plant`). A new plant
/// material belongs in one of these lists, or the seasons will leave it alone.
fn classify(m: &mut Materials) {
    use voxel::plant as pk;
    // Moss and lichen are left out: they keep their colour all year.
    let kinds: [(&[Voxel], u8); 10] = [
        (&[LEAVES_OAK, LEAVES_BIRCH, LEAVES_WILLOW, LEAVES_ACACIA, LEAVES_CHERRY], pk::DECIDUOUS),
        // Virginia creeper turns and drops like a broadleaf tree.
        (&[CREEPER_LEAF_DARK, CREEPER_LEAF, CREEPER_LEAF_RED, CREEPER_GREEN, CREEPER_GREEN_DARK, CREEPER_SCARLET], pk::DECIDUOUS),
        // Ivy is evergreen; vines and lianas keep their leaves.
        (&[IVY_LEAF_DARK, IVY_LEAF, IVY_LEAF_PALE, IVY_LEAF_RED, VINE_LEAF, IVY_GLOSS, IVY_GLOSS_LIT, IVY_GLOSS_NEW, VINE_GREEN], pk::VINE),
        (&[NEEDLES, NEEDLES_REDWOOD, LEAVES_JUNGLE, LEAVES_PALM, LEAVES_BAOBAB], pk::EVERGREEN),
        // Holly, yew and the rest keep their leaves and carry snow as the conifers do; so
        // do the ferns, wood-rush and heather under them, which the snow does not bury.
        (
            &[
                LEAVES_HOLLY, LEAVES_YEW, LEAVES_HOLM, NEEDLES_PINE, NEEDLES_JUNIPER, LEAVES_LAUREL, GORSE, HOLLY_BERRY, GORSE_BLOOM, FERN,
                FERN_TIP, WOODRUSH, HEATHER_GREEN, HEATHER_RUST,
            ],
            pk::EVERGREEN,
        ),
        (&[LEAVES_MAPLE, LEAVES_MAPLE_RED], pk::AUTUMN),
        (
            &[
                TUFT, TUFT_MEADOW, TUFT_LUSH, TUFT_DRY, TUFT_STRAW, TUFT_ALPINE, TUFT_SEDGE, SEED_HEAD, REED, REED_HEAD,
            ],
            pk::GRASS,
        ),
        (
            &[GRASS, GRASS_LUSH, GRASS_MEADOW, GRASS_DRY, GRASS_SAVANNA, GRASS_WOODLAND, GRASS_JUNGLE, GRASS_TAIGA, GRASS_TUNDRA],
            pk::GROUND,
        ),
        (
            &[BLOOM_RED, BLOOM_YELLOW, BLOOM_WHITE, BLOOM_PURPLE, BLOOM_BLUE, BLOOM_PINK, MUSHROOM_STEM, MUSHROOM_RED, MUSHROOM_TAN, VINE_FLOWER],
            pk::FLOWER,
        ),
        (
            &[
                VINE, BARK_OAK, BARK_BIRCH, BARK_PINE, BARK_REDWOOD, BARK_JUNGLE, BARK_PALM, BARK_WILLOW, BARK_ACACIA, BARK_BAOBAB,
                WOOD_DEAD, PLANK, PLANK_PALE, PLANK_DARK, PLANK_RED, IVY_STEM, IVY_WOOD, LIANA_STEM, IVY_WOOD_DARK, LIANA_WOOD,
                // Bark relief and twigs are wood: they keep their colour, and winter trees show them.
                BARK_OAK_RIDGE, BARK_REDWOOD_RIDGE, BARK_PINE_PLATE, BARK_BIRCH_MARK, BARK_BIRCH_PEEL, BARK_PALM_RING, TWIG, TWIG_PALE,
                IVY_TWIG, IVY_ROPE, IVY_ROPE_DARK,
                BARK_HOLLY, BARK_YEW, BARK_SCOTS, BARK_YEW_FLAKE,
            ],
            pk::VINE,
        ),
    ];
    for (voxels, kind) in kinds {
        for &v in voxels {
            m.set_plant(v, kind);
        }
    }
}
