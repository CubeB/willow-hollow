//! The voxel palette: every block id the generator can emit.

use voxel::{Kind, Material, Materials, Voxel};

use crate::biome::Biome;

macro_rules! palette {
    ($( $name:ident = ($r:expr, $g:expr, $b:expr), $kind:ident, $jitter:expr; )*) => {
        /// Voxel ids, in the order they are registered by [`build`].
        #[allow(non_camel_case_types, dead_code, clippy::upper_case_acronyms)]
        #[repr(u16)]
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        enum Id { AIR = 0, $( $name, )* }

        $( pub const $name: Voxel = Id::$name as Voxel; )*

        pub fn build() -> Materials {
            let mut m = Materials::new();
            $(
                let id = m.add(Material { color: [$r, $g, $b], kind: Kind::$kind, jitter: $jitter });
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
