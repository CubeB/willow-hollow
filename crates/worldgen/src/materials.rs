//! The voxel palette: every block id the generator can emit.

use voxel::{Kind, Material, Materials, Voxel};

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
    GRASS      = ( 96, 150,  62), Solid, 2;
    DIRT       = (122,  88,  58), Solid, 2;
    STONE      = (128, 128, 126), Solid, 2;
    SAND       = (222, 204, 150), Solid, 1;
    GRAVEL     = (146, 140, 132), Solid, 3;
    SNOW       = (238, 242, 248), Solid, 1;
    WATER      = ( 46, 112, 168), Water, 0;
}
