//! Small plants: grasses, flowers, reeds, mushrooms and lily pads, modelled from
//! blocks a quarter of a voxel across that stand on top of the voxel grid. Each is
//! decided by its own column alone and stays inside it, so neighbouring chunks agree
//! without looking at each other. The boxes of one plant never overlap, because faces
//! sharing a plane flicker against each other.

use voxel::{
    Voxel,
    mesh::{CELLS, PlantBox},
};

use super::shape::Rng;
use crate::{
    biome::Biome,
    materials as mat,
    noise::{fbm2, hash3, perlin2, unit},
    scale::{BLOCK, m, m64},
    surface::{Column, Cover},
};

/// The kinds of grass, each with its own form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Grass {
    /// Short, dense and low.
    Lawn,
    /// Tall stems, many of them carrying seed heads.
    Meadow,
    /// Wide tapering blades, for wet and shaded ground.
    Broad,
    /// A tight dry clump whose tips splay outward.
    Bunch,
    /// A few short separate blades on nearly every block, for the cold and the heights.
    Alpine,
    /// Fine upright blades in a line, beside the water.
    Sedge,
}

impl Grass {
    /// The share of columns that grow it.
    fn share(self, sward: &Sward) -> f32 {
        let thickness = match self {
            // Short enough to carpet the ground, whatever the biome's usual density.
            Grass::Alpine => return ALPINE_SHARE,
            Grass::Lawn => 1.3,
            Grass::Meadow | Grass::Broad => 1.0,
            Grass::Bunch => 0.6,
            Grass::Sedge => 1.6,
        };
        sward.density * thickness
    }
}

/// The grass of one biome: its two colours, the share of columns that grow any, and
/// the two kinds that take turns across the land in drifts.
struct Sward {
    main: Voxel,
    other: Voxel,
    density: f32,
    common: Grass,
    drift: Grass,
}

fn sward(biome: Biome) -> Option<Sward> {
    let s = |main, other, density, common, drift| Sward { main, other, density, common, drift };
    Some(match biome {
        Biome::Grassland => s(mat::TUFT_MEADOW, mat::TUFT, 0.06, Grass::Meadow, Grass::Lawn),
        Biome::Deciduous => s(mat::TUFT, mat::TUFT_LUSH, 0.035, Grass::Lawn, Grass::Broad),
        Biome::Woodland => s(mat::TUFT, mat::TUFT_MEADOW, 0.04, Grass::Lawn, Grass::Bunch),
        Biome::OldGrowth => s(mat::TUFT_LUSH, mat::TUFT, 0.02, Grass::Broad, Grass::Lawn),
        Biome::Rainforest => s(mat::TUFT_LUSH, mat::TUFT, 0.05, Grass::Broad, Grass::Lawn),
        Biome::Tundra => s(mat::TUFT_ALPINE, mat::TUFT_DRY, 0.035, Grass::Alpine, Grass::Alpine),
        Biome::Taiga => s(mat::TUFT_ALPINE, mat::TUFT_DRY, 0.025, Grass::Alpine, Grass::Alpine),
        Biome::Heath => s(mat::TUFT_DRY, mat::TUFT, 0.025, Grass::Bunch, Grass::Lawn),
        Biome::Savanna => s(mat::TUFT_DRY, mat::TUFT_MEADOW, 0.04, Grass::Bunch, Grass::Meadow),
        Biome::Marsh => s(mat::TUFT_SEDGE, mat::REED, 0.06, Grass::Sedge, Grass::Broad),
        Biome::Bog => s(mat::TUFT_SEDGE, mat::TUFT_DRY, 0.03, Grass::Sedge, Grass::Alpine),
        Biome::BirchGrove => s(mat::TUFT, mat::TUFT_MEADOW, 0.04, Grass::Lawn, Grass::Meadow),
        Biome::AlpineMeadow => s(mat::TUFT_ALPINE, mat::TUFT_MEADOW, 0.05, Grass::Alpine, Grass::Meadow),
        Biome::Steppe => s(mat::TUFT_STRAW, mat::TUFT_DRY, 0.05, Grass::Bunch, Grass::Meadow),
        Biome::Mangrove => s(mat::TUFT_LUSH, mat::TUFT_SEDGE, 0.03, Grass::Broad, Grass::Sedge),
        // Rough, pale moor grass in tussocks, with shorter grazed grass between.
        Biome::Moor => s(mat::TUFT_DRY, mat::TUFT_STRAW, 0.07, Grass::Bunch, Grass::Lawn),
        _ => return None,
    })
}

/// Noise that varies over about `metres`, for things that grow in patches.
fn patch(seed: u32, salt: u32, x: i32, z: i32, metres: f64) -> f32 {
    perlin2(seed ^ salt, x as f64 / m64(metres), z as f64 / m64(metres))
}

/// How far from a river's edge sedge can take over from the local grass.
const SEDGE_REACH: f32 = m(3.2);

/// How much of the cold ground alpine grass covers.
const ALPINE_SHARE: f32 = 0.2;

/// Below this temperature, which already allows for altitude, only alpine grass grows.
const COLD: f32 = 0.32;

/// The grass that belongs at this column, if the biome grows any.
fn grass_at(col: &Column, seed: u32, x: i32, z: i32) -> Option<(Grass, Sward)> {
    let sward = sward(col.biome)?;
    let reach = SEDGE_REACH * (1.0 + patch(seed, 0x5ed6, x, z, 2.4));
    if col.river < reach {
        return Some((Grass::Sedge, sward));
    }
    if col.temperature + patch(seed, 0xc01d, x, z, 9.6) * 0.03 < COLD {
        return Some((Grass::Alpine, sward));
    }
    // A broad drift with a ragged edge.
    let drift = patch(seed, 0x6a55, x, z, 14.4) + patch(seed, 0x6a56, x, z, 3.6) * 0.3;
    let kind = if drift > 0.08 { sward.drift } else { sward.common };
    Some((kind, sward))
}

/// The small plants that stay green through the winter, and show against the snow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Wintergreen {
    /// A shuttlecock of fronds, in beds on the floor of a wood.
    Fern,
    /// Wood-rush: a dark tussock of broad arching blades, in shade.
    Rush,
    /// A low cushion of heather, rusty with last year's flowers.
    Heather,
}

/// The most of a wood's floor, in the thick of a bed, that grows any one wintergreen.
const BED_SHARE: f32 = 0.45;

/// The wintergreen that grows at this column, if one does. `a` is the column's roll of
/// the dice. Ferns and wood-rush grow in beds a few metres across, thickest where the
/// wood stands close overhead (the noise `features::choose` thins the trees by); heather
/// in broad drifts on open ground.
fn wintergreen_at(col: &Column, seed: u32, x: i32, z: i32, a: f32) -> Option<Wintergreen> {
    // How much of the ground each takes, over the whole biome: fern, wood-rush, heather.
    let (fern, rush, heather) = match col.biome {
        Biome::Deciduous => (0.035, 0.02, 0.0),
        Biome::OldGrowth => (0.06, 0.02, 0.0),
        Biome::BirchGrove => (0.03, 0.02, 0.008),
        Biome::Woodland => (0.02, 0.012, 0.0),
        Biome::Taiga => (0.015, 0.008, 0.012),
        Biome::Grassland => (0.0, 0.006, 0.0),
        Biome::Heath => (0.0, 0.0, 0.05),
        Biome::Bog => (0.0, 0.008, 0.03),
        Biome::Tundra => (0.0, 0.0, 0.015),
        Biome::Moor => (0.0, 0.004, 0.06),
        _ => return None,
    };
    // Most columns grow none, and are turned away before any noise is worked out.
    if a >= BED_SHARE {
        return None;
    }
    // A bed fills up from nothing at its edge to `BED_SHARE` at its heart, and beds take
    // up about a tenth of the ground, so the shares above come out over the whole.
    let bed = |salt: u32, metres: f64, share: f32| {
        let n = patch(seed, salt, x, z, metres) + patch(seed, salt ^ 0x77, x, z, metres / 4.0) * 0.25;
        (share * 12.0).min(1.0) * BED_SHARE * ((n - 0.22) / 0.2).clamp(0.0, 1.0)
    };
    if heather > 0.0 && a < bed(0x4ea7, 16.0, heather) {
        return Some(Wintergreen::Heather);
    }
    if fern == 0.0 && rush == 0.0 {
        return None;
    }
    let (ferns, rushes) = (bed(0xfe21, 9.0, fern), bed(0x2054, 6.0, rush));
    if a >= ferns.max(rushes) {
        return None;
    }
    let openness = fbm2(seed ^ 0xc1ea, x as f64 / m64(76.0), z as f64 / m64(76.0), 3, 2.0, 0.5);
    let shade = 0.25 + 0.75 * ((openness + 0.35) / 0.5).clamp(0.0, 1.0);
    if a < ferns * shade {
        Some(Wintergreen::Fern)
    } else if a < rushes * shade {
        Some(Wintergreen::Rush)
    } else {
        None
    }
}

/// The flowers, and the other things that can be picked where they grow: each kind the
/// small plants grow with a bloom (or a cap) on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Flower {
    // The small star of five petals and an eye, in the colour of its drift.
    Pimpernel,
    Cinquefoil,
    Stitchwort,
    Violet,
    Speedwell,
    Campion,
    /// Heather in flower, on the heath and the moor.
    Heather,
    /// A lily pad with a flower open on it.
    WaterLily,
    FlyAgaric,
    PennyBun,
    /// Nodding bells along one side of an arching stem, in carpets under broadleaves.
    Bluebell,
    /// A tall spike hung with bells down one side, at the edges of woods and on the moor.
    Foxglove,
    /// A low clump of pale yellow flowers in a rosette of leaves, in woods in spring.
    Primrose,
    /// A low white star with a yellow eye, over the grassland.
    Daisy,
    /// Glossy gold cups on a branching stem, in the meadows.
    Buttercup,
    /// A scarlet cup with a dark boss on a tall wiry stem, in the grassland.
    Poppy,
    /// A dense tapering spike over a hand of leaves, mostly blue-violet.
    Lupin,
    /// White tufts on slender stems over the bog and the wet moor.
    CottonGrass,
    /// Kingcups: gold flowers over a mound of round dark leaves, in the marsh and by streams.
    MarshMarigold,
    /// A fan of sword leaves and a gold flower with drooping falls, in the marsh and by streams.
    FlagIris,
    /// Pink pompoms over a tight cushion, on dunes and shingle.
    Thrift,
    /// Upright trumpets of deep blue, close to the ground in the alpine meadows.
    Gentian,
    /// A grey felted star, found alone here and there high up.
    Edelweiss,
    /// Pale flowers with a dark lip, alternating up a stem over broad leaves, in the rainforest.
    Orchid,
    /// Zigzag red bracts tipped with gold, among paddle leaves, in the rainforest.
    Heliconia,
    /// Gold heads over a grey woolly mound, scattered over the desert.
    DesertMarigold,
    /// A purple brush on a spiny stem, on the steppe.
    Thistle,
}

impl Flower {
    pub const ALL: &[Flower] = &[
        Flower::Pimpernel,
        Flower::Cinquefoil,
        Flower::Stitchwort,
        Flower::Violet,
        Flower::Speedwell,
        Flower::Campion,
        Flower::Heather,
        Flower::WaterLily,
        Flower::FlyAgaric,
        Flower::PennyBun,
        Flower::Bluebell,
        Flower::Foxglove,
        Flower::Primrose,
        Flower::Daisy,
        Flower::Buttercup,
        Flower::Poppy,
        Flower::Lupin,
        Flower::CottonGrass,
        Flower::MarshMarigold,
        Flower::FlagIris,
        Flower::Thrift,
        Flower::Gentian,
        Flower::Edelweiss,
        Flower::Orchid,
        Flower::Heliconia,
        Flower::DesertMarigold,
        Flower::Thistle,
    ];

    /// Its name, in plain English.
    pub fn name(self) -> &'static str {
        match self {
            Flower::Pimpernel => "Scarlet pimpernel",
            Flower::Cinquefoil => "Cinquefoil",
            Flower::Stitchwort => "Stitchwort",
            Flower::Violet => "Violet",
            Flower::Speedwell => "Speedwell",
            Flower::Campion => "Campion",
            Flower::Heather => "Heather",
            Flower::WaterLily => "Water lily",
            Flower::FlyAgaric => "Fly agaric",
            Flower::PennyBun => "Penny bun",
            Flower::Bluebell => "Bluebell",
            Flower::Foxglove => "Foxglove",
            Flower::Primrose => "Primrose",
            Flower::Daisy => "Daisy",
            Flower::Buttercup => "Buttercup",
            Flower::Poppy => "Poppy",
            Flower::Lupin => "Lupin",
            Flower::CottonGrass => "Cotton grass",
            Flower::MarshMarigold => "Marsh marigold",
            Flower::FlagIris => "Flag iris",
            Flower::Thrift => "Thrift",
            Flower::Gentian => "Gentian",
            Flower::Edelweiss => "Edelweiss",
            Flower::Orchid => "Orchid",
            Flower::Heliconia => "Heliconia",
            Flower::DesertMarigold => "Desert marigold",
            Flower::Thistle => "Thistle",
        }
    }

    /// A key that never changes, for save files.
    pub fn id(self) -> &'static str {
        match self {
            Flower::Pimpernel => "pimpernel",
            Flower::Cinquefoil => "cinquefoil",
            Flower::Stitchwort => "stitchwort",
            Flower::Violet => "violet",
            Flower::Speedwell => "speedwell",
            Flower::Campion => "campion",
            Flower::Heather => "heather",
            Flower::WaterLily => "water_lily",
            Flower::FlyAgaric => "fly_agaric",
            Flower::PennyBun => "penny_bun",
            Flower::Bluebell => "bluebell",
            Flower::Foxglove => "foxglove",
            Flower::Primrose => "primrose",
            Flower::Daisy => "daisy",
            Flower::Buttercup => "buttercup",
            Flower::Poppy => "poppy",
            Flower::Lupin => "lupin",
            Flower::CottonGrass => "cotton_grass",
            Flower::MarshMarigold => "marsh_marigold",
            Flower::FlagIris => "flag_iris",
            Flower::Thrift => "thrift",
            Flower::Gentian => "gentian",
            Flower::Edelweiss => "edelweiss",
            Flower::Orchid => "orchid",
            Flower::Heliconia => "heliconia",
            Flower::DesertMarigold => "desert_marigold",
            Flower::Thistle => "thistle",
        }
    }

    /// The material of its bloom (or cap): the colour it shows.
    pub fn petals(self) -> Voxel {
        match self {
            Flower::Pimpernel => mat::BLOOM_RED,
            Flower::Cinquefoil => mat::BLOOM_YELLOW,
            Flower::Stitchwort => mat::BLOOM_WHITE,
            Flower::WaterLily => mat::BLOOM_PINK,
            Flower::Violet | Flower::Heather => mat::BLOOM_PURPLE,
            Flower::Speedwell => mat::BLOOM_BLUE,
            Flower::Campion => mat::BLOOM_PINK,
            Flower::FlyAgaric => mat::MUSHROOM_RED,
            Flower::PennyBun => mat::MUSHROOM_TAN,
            Flower::Bluebell => mat::BLOOM_BLUEBELL,
            Flower::Foxglove => mat::BLOOM_FOXGLOVE,
            Flower::Primrose => mat::BLOOM_PRIMROSE,
            Flower::Daisy | Flower::CottonGrass => mat::BLOOM_WHITE,
            Flower::Buttercup | Flower::MarshMarigold | Flower::FlagIris | Flower::DesertMarigold => mat::BLOOM_GOLD,
            Flower::Poppy => mat::BLOOM_POPPY,
            Flower::Lupin => mat::BLOOM_LUPIN,
            Flower::Thrift => mat::BLOOM_PINK,
            Flower::Gentian => mat::BLOOM_GENTIAN,
            Flower::Edelweiss => mat::BLOOM_FELT,
            Flower::Orchid => mat::BLOOM_ORCHID,
            Flower::Heliconia => mat::BLOOM_RED,
            Flower::Thistle => mat::BLOOM_THISTLE,
        }
    }

    /// The colour of its bloom, sRGB: the colour of [`Flower::petals`] in the palette.
    pub fn color(self) -> [u8; 3] {
        match self.petals() {
            mat::BLOOM_RED => [222, 70, 64],
            mat::BLOOM_YELLOW => [244, 214, 80],
            mat::BLOOM_WHITE => [244, 244, 234],
            mat::BLOOM_PURPLE => [170, 110, 204],
            mat::BLOOM_BLUE => [108, 140, 230],
            mat::BLOOM_PINK => [242, 160, 198],
            mat::MUSHROOM_RED => [196, 56, 48],
            mat::MUSHROOM_TAN => [178, 142, 100],
            mat::BLOOM_BLUEBELL => [124, 116, 240],
            mat::BLOOM_FOXGLOVE => [198, 92, 168],
            mat::BLOOM_PRIMROSE => [244, 230, 112],
            mat::BLOOM_GOLD => [250, 196, 36],
            mat::BLOOM_POPPY => [222, 46, 34],
            mat::BLOOM_LUPIN => [120, 98, 214],
            mat::BLOOM_GENTIAN => [38, 66, 200],
            mat::BLOOM_FELT => [232, 232, 218],
            mat::BLOOM_ORCHID => [242, 200, 230],
            _ => [170, 84, 178],
        }
    }
}

/// The names of the months, January first.
pub const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/// How far into the calendar year the spring equinox falls (the 20th of March), which is
/// where the game's date starts from: a date is a fraction of the year from then, 0.25
/// midsummer, 0.5 the autumn equinox and 0.75 midwinter.
const EQUINOX: f32 = 79.0 / 365.0;

/// The date on which month `m` begins, 1 for January (and 13 for January again).
pub fn month_start(m: u8) -> f32 {
    ((m as f32 - 1.0) / 12.0 - EQUINOX).rem_euclid(1.0)
}

/// The month a date falls in, 1 for January.
pub fn month_of(date: f32) -> u8 {
    (((date + EQUINOX).rem_euclid(1.0) * 12.0) as u8).min(11) + 1
}

/// Over how much of the year the plants of one kind come into flower, one after another,
/// and go over again: each column has its own place in it. season.wgsl keeps the same
/// number (`SEASON_STAGGER`).
pub const STAGGER: f32 = 0.05;

/// What one part of a plant is doing on a date.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Not there at all.
    Gone,
    /// There, but green and shut.
    Bud,
    /// Out: in flower, or in leaf.
    Bloom,
    /// Over: a brown head where the flower was.
    Seed,
}

/// The year one part of a plant keeps: a flower's bloom, or its leaves. The renderer
/// holds every one in a table (see [`seasons`]), and a plant's boxes name theirs by its
/// row ([`voxel::mesh::PlantBox::season`]), so the shader and [`Season::phase`] agree
/// about every plant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Season {
    /// The date the earliest plants come out. The latest follow [`STAGGER`] after.
    pub start: f32,
    /// How long each plant is out for. 1 or more for all year.
    pub length: f32,
    /// How long before it comes out a plant shows a green bud.
    pub bud: f32,
    /// How long after it goes over a plant keeps a seed head.
    pub seed: f32,
}

/// The shader's `season_hash`, for the column at `(x, z)`: where in the [`STAGGER`] the
/// plant standing on it falls.
fn stagger(x: i32, z: i32) -> f32 {
    let mut h = (x as u32).wrapping_mul(374761393).wrapping_add((z as u32).wrapping_mul(2246822519));
    h = (h ^ (h >> 13)).wrapping_mul(1274126177);
    h ^= h >> 16;
    (h & 0xff_ffff) as f32 / 16_777_216.0
}

impl Season {
    const ALWAYS: Season = Season { start: 0.0, length: 1.0, bud: 0.0, seed: 0.0 };

    /// What the plant on the column at `(x, z)` is doing on `date`. `season_phase` in
    /// season.wgsl is the same sum.
    pub fn phase(&self, date: f32, x: i32, z: i32) -> Phase {
        if self.length >= 1.0 {
            return Phase::Bloom;
        }
        let u = (date - self.start - STAGGER * stagger(x, z) + self.bud).rem_euclid(1.0);
        if u < self.bud {
            Phase::Bud
        } else if u < self.bud + self.length {
            Phase::Bloom
        } else if u < self.bud + self.length + self.seed {
            Phase::Seed
        } else {
            Phase::Gone
        }
    }

    /// As the shader's table holds it.
    fn row(&self) -> [f32; 4] {
        [self.start, self.length, self.bud, self.seed]
    }
}

/// Rows in the table of seasons: what a [`voxel::mesh::PlantBox::season`] can name.
pub const SEASON_SLOTS: usize = 64;

/// The table of seasons, by the number a plant's box carries: `[start, length, bud,
/// seed]` of a [`Season`]. Row 0 is unused (a box with no season of its own); then come
/// the blooms of the flowers in the order of [`Flower::ALL`], then their leaves.
pub fn seasons() -> [[f32; 4]; SEASON_SLOTS] {
    let mut rows = [[0.0; 4]; SEASON_SLOTS];
    for &f in Flower::ALL {
        rows[f.bloom_code() as usize] = f.bloom().row();
        rows[f.leaf_code() as usize] = f.leaves().row();
    }
    rows
}

impl Flower {
    /// The first and last month it is in flower (a mushroom, up), 1 for January: true to
    /// the plant, in the temperate north. `(1, 12)` for the few that flower all year.
    pub fn months(self) -> (u8, u8) {
        match self {
            Flower::Primrose => (2, 5),
            Flower::Violet | Flower::MarshMarigold => (3, 5),
            Flower::Daisy => (3, 10),
            Flower::Bluebell => (4, 5),
            Flower::Stitchwort => (4, 6),
            Flower::Speedwell | Flower::Thrift => (4, 7),
            Flower::Campion => (4, 10),
            Flower::Lupin | Flower::CottonGrass | Flower::FlagIris => (5, 7),
            Flower::Buttercup => (5, 8),
            Flower::Pimpernel => (5, 9),
            Flower::Foxglove | Flower::Poppy | Flower::Gentian => (6, 8),
            Flower::Cinquefoil | Flower::WaterLily => (6, 9),
            Flower::Edelweiss | Flower::Thistle => (7, 9),
            Flower::Heather => (8, 9),
            Flower::PennyBun => (8, 10),
            Flower::FlyAgaric => (9, 11),
            // The desert flowers whenever it has rained, and the rainforest has no winter.
            Flower::DesertMarigold => (3, 11),
            Flower::Orchid | Flower::Heliconia => (1, 12),
        }
    }

    /// Whether it flowers the whole year round.
    pub fn all_year(self) -> bool {
        self.months() == (1, 12)
    }

    /// Its flowering season: the dates (fractions of the year from the spring equinox)
    /// between which it is to be found in flower. The end is before the start for one
    /// that flowers across the equinox, as a primrose does; `(0.0, 1.0)` is all year.
    pub fn season(self) -> (f32, f32) {
        if self.all_year() {
            return (0.0, 1.0);
        }
        let (first, last) = self.months();
        (month_start(first), month_start(last + 1))
    }

    /// How much of the year its season is.
    fn span(self) -> f32 {
        let (start, end) = self.season();
        if self.all_year() { 1.0 } else { (end - start).rem_euclid(1.0) }
    }

    /// Whether `date` is in its flowering season. A plant here and there is not out yet
    /// at the start of it, or is already over at the end: see [`Flower::blooming_at`].
    pub fn in_bloom(self, date: f32) -> bool {
        self.all_year() || (date - self.season().0).rem_euclid(1.0) < self.span()
    }

    /// What the plant of this kind on the column at `(x, z)` shows on `date`: each comes
    /// out and goes over on its own day, as the game draws it.
    pub fn phase_at(self, date: f32, x: i32, z: i32) -> Phase {
        self.bloom().phase(date, x, z)
    }

    /// Whether the plant of this kind on the column at `(x, z)` is in flower on `date`.
    pub fn blooming_at(self, date: f32, x: i32, z: i32) -> bool {
        self.phase_at(date, x, z) == Phase::Bloom
    }

    /// The year its bloom keeps.
    pub fn bloom(self) -> Season {
        if self.all_year() {
            return Season::ALWAYS;
        }
        let length = self.span() - STAGGER;
        let (bud, seed) = match self {
            // A mushroom comes up overnight and rots away.
            Flower::FlyAgaric | Flower::PennyBun => (0.0, 0.0),
            // Spent heather stays on the plant, rusty, until it buds again.
            Flower::Heather => (0.04, 1.0 - 0.04 - length),
            // The white heads are the seed, and blow away; a lily sinks; these are small.
            Flower::CottonGrass | Flower::WaterLily | Flower::Daisy | Flower::Primrose => (0.03, 0.0),
            _ => (0.03, 0.045),
        };
        Season { start: self.season().0, length, bud, seed }
    }

    /// The year its stems and leaves keep: up before the buds, and gone by winter. A
    /// bluebell's die back soon after it flowers; a primrose's stay.
    pub fn leaves(self) -> Season {
        if self.all_year() || matches!(self, Flower::Primrose | Flower::DesertMarigold) {
            return Season::ALWAYS;
        }
        // A mushroom's stem goes with its cap.
        if matches!(self, Flower::FlyAgaric | Flower::PennyBun) {
            return self.bloom();
        }
        let (first, end) = self.season();
        let start = (first - 0.08).rem_euclid(1.0);
        let since = |date: f32| (date - start).rem_euclid(1.0);
        let after = since(end + 0.10);
        let length = if self == Flower::Bluebell { after } else { after.max(since(0.68)) };
        Season { start, length, bud: 0.0, seed: 0.0 }
    }

    /// The row of [`seasons`] its bloom keeps.
    pub fn bloom_code(self) -> u8 {
        self as u8 + 1
    }

    /// The row of [`seasons`] its stems and leaves keep.
    pub fn leaf_code(self) -> u8 {
        self as u8 + 1 + Flower::ALL.len() as u8
    }
}

/// The flower a drift of flowering ground grows, from the colour the drift carries
/// (`features::flower`) and the biome. `which` (see [`swathe`]) picks between the flowers
/// one colour stands for, in broad swathes, so neighbouring drifts can differ: and so
/// that a biome has something in flower from spring to autumn, each kind in its season.
fn species(biome: Biome, surface: Voxel, which: f32) -> Option<Flower> {
    use Flower as F;
    let alt = which > 0.0;
    let generic = match surface {
        mat::FLOWER_RED => F::Pimpernel,
        mat::FLOWER_YELLOW => F::Cinquefoil,
        mat::FLOWER_WHITE => F::Stitchwort,
        mat::FLOWER_PURPLE => F::Violet,
        mat::FLOWER_BLUE => F::Speedwell,
        mat::FLOWER_PINK => F::Campion,
        _ => return None,
    };
    Some(match (biome, surface) {
        (Biome::Grassland, mat::FLOWER_YELLOW) => if alt { F::Cinquefoil } else { F::Buttercup },
        (Biome::Grassland, mat::FLOWER_WHITE) => if alt { F::Stitchwort } else { F::Daisy },
        // Scarlet pimpernel is a weed of the same ground as the poppy.
        (Biome::Grassland, mat::FLOWER_RED) => if which > 0.3 { F::Pimpernel } else { F::Poppy },
        (Biome::Grassland | Biome::AlpineMeadow, mat::FLOWER_PURPLE) => if alt { F::Violet } else { F::Lupin },
        (Biome::Deciduous | Biome::Woodland, mat::FLOWER_BLUE) => F::Bluebell,
        // Dog violets in the birches, out before the bluebells.
        (Biome::BirchGrove, mat::FLOWER_BLUE) => if which < -0.2 { F::Violet } else { F::Bluebell },
        (Biome::Deciduous | Biome::Woodland, mat::FLOWER_YELLOW) => F::Primrose,
        // Stitchwort in spring; red campion from then until the frost, and ragged robin,
        // its cousin, in the marsh.
        (Biome::Deciduous | Biome::Woodland | Biome::BirchGrove | Biome::Marsh, mat::FLOWER_WHITE) => if alt { F::Campion } else { F::Stitchwort },
        (Biome::AlpineMeadow, mat::FLOWER_BLUE) => F::Gentian,
        (Biome::Marsh, mat::FLOWER_YELLOW) => if alt { F::FlagIris } else { F::MarshMarigold },
        // Violets on the heath and the moor in spring, before the tormentil.
        (Biome::Heath | Biome::Moor, mat::FLOWER_YELLOW) => if alt { F::Violet } else { F::Cinquefoil },
        (Biome::Moor, mat::FLOWER_WHITE) => F::CottonGrass,
        // Kingcups in the wettest of the bog in spring, cotton grass in early summer, and
        // tormentil on its drier edges until autumn.
        (Biome::Bog, mat::FLOWER_WHITE) => {
            if which > 0.15 {
                F::MarshMarigold
            } else if which < -0.15 {
                F::Cinquefoil
            } else {
                F::CottonGrass
            }
        }
        // The far north's short summer: gentians and cinquefoil after the violets.
        (Biome::Tundra, mat::FLOWER_PURPLE) => if alt { F::Gentian } else { F::Violet },
        (Biome::Tundra, mat::FLOWER_WHITE) => if alt { F::Cinquefoil } else { F::Stitchwort },
        (Biome::Rainforest, mat::FLOWER_RED) => F::Heliconia,
        (Biome::Rainforest, mat::FLOWER_PINK) => F::Orchid,
        (Biome::Steppe, mat::FLOWER_PURPLE) => if alt { F::Violet } else { F::Thistle },
        (Biome::Desert | Biome::RedRock, mat::FLOWER_YELLOW) => F::DesertMarigold,
        // Dry country: marigolds whenever it rains.
        (Biome::Savanna, mat::FLOWER_YELLOW) => if alt { F::DesertMarigold } else { F::Cinquefoil },
        _ => generic,
    })
}

/// Which of a colour's flowers the drifts round `(x, z)` grow: noise over about a hundred
/// metres, for [`species`].
fn swathe(seed: u32, x: i32, z: i32) -> f32 {
    patch(seed, 0xa17e, x, z, 96.0)
}

/// What grows on one column, decided before any of it is built, so that [`grow`] and
/// [`flower_at`] cannot disagree.
enum Growth {
    Grass(Grass, Sward),
    /// A dry tussock: marram on the dunes, or a clump on a dry stream's bank.
    Tussock,
    Reeds,
    /// A lily pad with no flower.
    Lily,
    Wintergreen(Wintergreen),
    Flower(Flower),
}

/// Whether a column is in the temperate band where kingcups and flag irises line the water.
fn temperate(col: &Column) -> bool {
    (0.36..0.72).contains(&col.temperature) && !col.biome.arid() && !matches!(col.biome, Biome::Savanna | Biome::Rainforest | Biome::Mangrove)
}

/// What grows at this column, if anything. `a` is the column's roll of the dice.
fn decide(col: &Column, surface: Voxel, x: i32, z: i32, seed: u32, a: f32) -> Option<Growth> {
    match col.cover {
        // Dry streams have no reeds, only a tussock now and then along the banks.
        Cover::Bank if col.biome.arid() => {
            (col.water_surface().is_none() && a < 0.035 && patch(seed, 0x4eef, x, z, 6.0) > 0.1).then_some(Growth::Tussock)
        }
        // Elsewhere reeds stand in clumps and stretches along the water, with open bank
        // between them where the grass comes down to the edge.
        Cover::Bank if a < 0.25 && patch(seed, 0x4eed, x, z, 4.8) > 0.3 => Some(Growth::Reeds),
        Cover::Bank if col.clear >= m(0.8) && col.water_surface().is_none() => {
            if surface != mat::ground_cover(col.biome) {
                return None;
            }
            // Kingcups and flag irises in stretches along temperate streams, between the reeds.
            if a < 0.08 && temperate(col) && patch(seed, 0xb10e, x, z, 7.2) > 0.2 {
                let iris = patch(seed, 0xb10f, x, z, 20.0) > 0.0;
                return Some(Growth::Flower(if iris { Flower::FlagIris } else { Flower::MarshMarigold }));
            }
            let (kind, sward) = grass_at(col, seed, x, z)?;
            (a < kind.share(&sward)).then_some(Growth::Grass(kind, sward))
        }
        Cover::LakeBed => {
            let k = col.water_surface()?;
            if k - col.top() >= 2 * BLOCK && a < 0.10 && patch(seed, 0x11e5, x, z, 4.8) > 0.2 {
                let bloom = unit(hash3(seed ^ 0x11e6, x, 0, z)) < 0.2;
                Some(if bloom { Growth::Flower(Flower::WaterLily) } else { Growth::Lily })
            } else {
                None
            }
        }
        Cover::Land if col.clear >= m(0.8) && col.water_surface().is_none() => {
            // Flowering heather ground carries the heather itself, not a flower.
            if surface == mat::HEATHER {
                return Some(Growth::Flower(Flower::Heather));
            }
            if matches!(surface, mat::FLOWER_RED | mat::FLOWER_YELLOW | mat::FLOWER_WHITE | mat::FLOWER_PURPLE | mat::FLOWER_BLUE | mat::FLOWER_PINK) {
                return species(col.biome, surface, swathe(seed, x, z)).map(Growth::Flower);
            }
            let forest = matches!(col.biome, Biome::OldGrowth | Biome::Rainforest | Biome::Taiga);
            if forest && a < 0.012 && patch(seed, 0x3051, x, z, 6.4) > 0.15 {
                let red = unit(hash3(seed ^ 0x3052, x, 0, z)) < 0.5;
                return Some(Growth::Flower(if red { Flower::FlyAgaric } else { Flower::PennyBun }));
            }
            if surface != mat::ground_cover(col.biome) {
                return None;
            }
            // Foxgloves stand in loose groups at the edges of woods and on the moor, and
            // edelweiss is found alone, high up where the grass is short.
            let wooded = matches!(col.biome, Biome::Deciduous | Biome::Woodland | Biome::OldGrowth | Biome::BirchGrove | Biome::Moor);
            if wooded && a < 0.03 && patch(seed, 0xf0c5, x, z, 12.0) > 0.38 {
                return Some(Growth::Flower(Flower::Foxglove));
            }
            if matches!(col.biome, Biome::AlpineMeadow | Biome::Tundra) && a < 0.0025 {
                return Some(Growth::Flower(Flower::Edelweiss));
            }
            if let Some(kind) = wintergreen_at(col, seed, x, z, a) {
                return Some(Growth::Wintergreen(kind));
            }
            let (kind, sward) = grass_at(col, seed, x, z)?;
            (a < kind.share(&sward)).then_some(Growth::Grass(kind, sward))
        }
        // Marram grass binding the dunes: sparse dry clumps on the seaward face, thicker
        // further up and behind, where the sand is higher and sheltered from the spray.
        // Thrift grows in pink patches among it, out of the tropics.
        Cover::Dune if col.water_surface().is_none() && col.biome != Biome::Volcanic => {
            if a < 0.03 && col.temperature < 0.75 && patch(seed, 0x7417, x, z, 6.0) > 0.3 {
                return Some(Growth::Flower(Flower::Thrift));
            }
            let landward = ((col.ground - m(2.0)) / m(3.0)).clamp(0.0, 1.0);
            (a < 0.03 + 0.12 * landward && patch(seed, 0x3a77, x, z, 4.0) > -0.25).then_some(Growth::Tussock)
        }
        // Thrift in cushions on the shingle above the tide.
        Cover::Shingle if col.water_surface().is_none() && col.temperature < 0.75 && !col.biome.arid() => {
            (a < 0.05 && patch(seed, 0x7418, x, z, 6.0) > 0.15).then_some(Growth::Flower(Flower::Thrift))
        }
        _ => None,
    }
}

/// The roll of the dice for the column at `(x, z)`.
fn column_hash(seed: u32, x: i32, z: i32) -> u32 {
    hash3(seed ^ 0x91a7, x, 0, z)
}

/// The flower (or mushroom) that [`grow`] builds on the column at world position
/// `(x, z)` for the same arguments, if it builds one. Where things grow does not depend on
/// the season, so neither does this, though a flower can be out of bloom: ask
/// [`Flower::blooming_at`].
pub fn flower_at(col: &Column, surface: Voxel, x: i32, z: i32, seed: u32) -> Option<Flower> {
    match decide(col, surface, x, z, seed, unit(column_hash(seed, x, z)))? {
        Growth::Flower(f) => Some(f),
        _ => None,
    }
}

/// The colour a plant gives the ground it stands on, seen from too far off for the plant
/// itself to be drawn: the ground under it is drawn in the plant's colours instead, so a
/// moor's heather and a meadow's flowers still show as drifts of colour across a far
/// hillside, where each stood. (See `wash` in the game's terrain shader.)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wash {
    /// The leaves and stems: two materials, and the share of the second.
    pub body: (Voxel, Voxel, f32),
    /// The flowers or heads, and how much of the plant's colour is theirs (0 with none).
    /// The seasons take them away as they do the flowers themselves.
    pub bloom: (Voxel, f32),
    /// The row of [`seasons`] the flowers keep, or 0 for heads with no season of their own.
    pub season: u8,
    /// How much of the ground's colour gives way to the plant's.
    pub cover: f32,
}

fn wash_of(growth: &Growth) -> Option<Wash> {
    let season = match *growth {
        Growth::Flower(f) => f.bloom_code(),
        _ => 0,
    };
    let w = |body: (Voxel, Voxel, f32), bloom: (Voxel, f32), cover: f32| Some(Wash { body, bloom, season, cover });
    let none = (voxel::AIR, 0.0);
    match *growth {
        Growth::Grass(kind, ref sward) => match kind {
            Grass::Lawn => w((sward.main, sward.other, 0.3), none, 0.7),
            Grass::Meadow => w((sward.main, sward.other, 0.3), (mat::SEED_HEAD, 0.3), 0.6),
            Grass::Broad => w((mat::TUFT_LUSH, mat::TUFT, 0.3), none, 0.7),
            Grass::Bunch => w((mat::TUFT_STRAW, mat::TUFT_DRY, 0.4), none, 0.7),
            Grass::Alpine => w((sward.main, sward.other, 0.25), none, 0.45),
            Grass::Sedge => w((mat::TUFT_SEDGE, mat::TUFT_LUSH, 0.25), none, 0.65),
        },
        Growth::Tussock => w((mat::TUFT_STRAW, mat::TUFT_DRY, 0.4), none, 0.7),
        Growth::Reeds => w((mat::REED, mat::REED, 0.0), (mat::REED_HEAD, 0.25), 0.8),
        Growth::Lily => None,
        Growth::Wintergreen(Wintergreen::Fern) => w((mat::FERN, mat::FERN_TIP, 0.3), none, 0.85),
        Growth::Wintergreen(Wintergreen::Rush) => w((mat::WOODRUSH, mat::WOODRUSH, 0.0), none, 0.8),
        Growth::Wintergreen(Wintergreen::Heather) => w((mat::HEATHER_GREEN, mat::HEATHER_RUST, 0.3), none, 0.9),
        Growth::Flower(f) => match f {
            Flower::WaterLily | Flower::FlyAgaric | Flower::PennyBun => None,
            Flower::Heather => w((mat::HEATHER_GREEN, mat::HEATHER_RUST, 0.15), (mat::BLOOM_PURPLE, 0.4), 0.9),
            Flower::CottonGrass => w((mat::TUFT_SEDGE, mat::TUFT_DRY, 0.3), (mat::BLOOM_WHITE, 0.35), 0.5),
            Flower::Thrift => w((mat::TUFT_ALPINE, mat::TUFT, 0.3), (mat::BLOOM_PINK, 0.45), 0.7),
            Flower::DesertMarigold => w((mat::TUFT_ALPINE, mat::TUFT_DRY, 0.5), (mat::BLOOM_GOLD, 0.4), 0.6),
            Flower::Foxglove | Flower::Lupin | Flower::Poppy | Flower::Thistle | Flower::Heliconia | Flower::Bluebell | Flower::FlagIris => {
                w((mat::TUFT_LUSH, mat::TUFT, 0.3), (f.petals(), 0.6), 0.6)
            }
            _ => w((mat::TUFT_LUSH, mat::TUFT, 0.3), (f.petals(), 0.5), 0.5),
        },
    }
}

/// Every wash there is. A column's is named by its place in this list plus one (see
/// [`wash_at`]), 0 being none.
pub fn washes() -> &'static [Wash] {
    static ALL: std::sync::OnceLock<Vec<Wash>> = std::sync::OnceLock::new();
    ALL.get_or_init(|| {
        let mut all: Vec<Wash> = Vec::new();
        let mut add = |g: Growth| {
            if let Some(w) = wash_of(&g)
                && !all.contains(&w)
            {
                all.push(w);
            }
        };
        for biome in Biome::ALL {
            if let Some(s) = sward(biome) {
                for kind in [s.common, s.drift, Grass::Sedge, Grass::Alpine] {
                    add(Growth::Grass(kind, sward(biome).expect("the biome has a sward")));
                }
            }
        }
        add(Growth::Tussock);
        add(Growth::Reeds);
        for kind in [Wintergreen::Fern, Wintergreen::Rush, Wintergreen::Heather] {
            add(Growth::Wintergreen(kind));
        }
        for &f in Flower::ALL {
            add(Growth::Flower(f));
        }
        assert!(all.len() < 256, "a wash is named in a byte");
        all
    })
}

/// The wash of whatever [`grow`] builds on the column at `(x, z)` for the same arguments:
/// its place in [`washes`] plus one, or 0 where nothing grows or what does is too slight to
/// colour the ground. Works out what grows there without building it, which is most of
/// the work.
pub fn wash_at(col: &Column, surface: Voxel, x: i32, z: i32, seed: u32) -> u8 {
    let Some(growth) = decide(col, surface, x, z, seed, unit(column_hash(seed, x, z))) else {
        return 0;
    };
    let Some(wash) = wash_of(&growth) else {
        return 0;
    };
    washes().iter().position(|w| *w == wash).map_or(0, |i| i as u8 + 1)
}

/// The colour the flowers and heather round `(x, z)` give the land seen from beyond the
/// voxels, where a stretch of ground is all one colour: the wash of what grows there (its
/// place in [`washes`] plus one, as [`wash_at`] gives it) and how much of the ground's
/// colour it takes the place of. It follows the drifts the flowers grow in
/// ([`super::flower_drift`]) and the moor's beds of heather, so a far moor is patched
/// with heather as a near one is, and the far view colours it by the date as the near
/// ground is coloured.
pub fn far_wash(biome: Biome, x: i32, z: i32, seed: u32) -> Option<(u8, f32)> {
    let (growth, share) = match super::flower_drift(biome, x, z, seed) {
        Some((mat::HEATHER, share)) => (Growth::Flower(Flower::Heather), share),
        Some((surface, share)) => (Growth::Flower(species(biome, surface, swathe(seed, x, z))?), share),
        None => {
            // Heather out of flower, in its beds (see `wintergreen_at`).
            let heather = match biome {
                Biome::Heath => 0.05,
                Biome::Bog => 0.03,
                Biome::Moor => 0.06,
                _ => return None,
            };
            let n = patch(seed, 0x4ea7, x, z, 16.0);
            let share = (heather * 12.0f32).min(1.0) * BED_SHARE * ((n - 0.22) / 0.2).clamp(0.0, 1.0);
            (Growth::Wintergreen(Wintergreen::Heather), share)
        }
    };
    let w = wash_of(&growth)?;
    let code = washes().iter().position(|v| *v == w)? as u8 + 1;
    Some((code, (share * 2.0).min(1.0) * w.cover))
}

/// Lays out boxes in cells relative to the corner of one voxel.
struct Plot<'a> {
    origin: [i32; 3],
    out: &'a mut Vec<PlantBox>,
    /// The flower being built, if it is one: its blooms and its leaves are given the
    /// seasons they keep (see [`seasons`]).
    flower: Option<Flower>,
}

/// Whether a material is part of a flower's bloom (or a mushroom), which comes and goes
/// with its flowering season.
fn is_bloom(voxel: Voxel) -> bool {
    matches!(
        voxel,
        mat::BLOOM_RED
            | mat::BLOOM_YELLOW
            | mat::BLOOM_WHITE
            | mat::BLOOM_PURPLE
            | mat::BLOOM_BLUE
            | mat::BLOOM_PINK
            | mat::MUSHROOM_STEM
            | mat::MUSHROOM_RED
            | mat::MUSHROOM_TAN
            | mat::BLOOM_BLUEBELL
            | mat::BLOOM_FOXGLOVE
            | mat::BLOOM_PRIMROSE
            | mat::BLOOM_GOLD
            | mat::BLOOM_POPPY
            | mat::BLOOM_EYE
            | mat::BLOOM_LUPIN
            | mat::BLOOM_GENTIAN
            | mat::BLOOM_FELT
            | mat::BLOOM_ORCHID
            | mat::BLOOM_ORCHID_LIP
            | mat::BLOOM_THISTLE
            // The eye of a cinquefoil.
            | mat::REED_HEAD
    )
}

impl<'a> Plot<'a> {
    /// A plot for a plant with its corner at `origin`, in cells.
    fn new(origin: [i32; 3], out: &'a mut Vec<PlantBox>) -> Self {
        Self { origin, out, flower: None }
    }

    /// The season a box of `voxel` keeps, as a row of [`seasons`]: 0 for anything that
    /// is not part of a flower.
    fn season(&self, voxel: Voxel) -> u8 {
        let Some(f) = self.flower else { return 0 };
        // The six small flowers of the drifts have a blade of grass for a stem.
        let small = matches!(f, Flower::Pimpernel | Flower::Cinquefoil | Flower::Stitchwort | Flower::Violet | Flower::Speedwell | Flower::Campion);
        if is_bloom(voxel) {
            f.bloom_code()
        } else if matches!(voxel, mat::STEM | mat::LEAF_GLOSS | mat::LEAF_GREY) || (small && matches!(voxel, mat::TUFT_LUSH | mat::TUFT)) {
            f.leaf_code()
        } else {
            0
        }
    }

    /// A box from `lo` up to, but not including, `hi`. Its ends sway with the wind if
    /// their flags say so: a stem is rooted at the bottom and free at the top.
    fn add(&mut self, lo: [i32; 3], hi: [i32; 3], voxel: Voxel, sway: (bool, bool)) {
        let at = |p: [i32; 3]| [self.origin[0] + p[0], self.origin[1] + p[1], self.origin[2] + p[2]];
        self.out.push(PlantBox {
            lo: at(lo),
            hi: at(hi),
            voxel,
            sway_lo: sway.0,
            sway_hi: sway.1,
            thin: 0,
            season: self.season(voxel),
        });
    }

    /// A vertical stem one cell square, rooted on the ground.
    fn stem(&mut self, x: i32, z: i32, height: i32, voxel: Voxel) {
        self.add([x, 0, z], [x + 1, height, z + 1], voxel, (false, true));
    }

    /// A single cell at `(x, y, z)`, free to sway.
    fn dot(&mut self, x: i32, y: i32, z: i32, voxel: Voxel) {
        self.add([x, y, z], [x + 1, y + 1, z + 1], voxel, (true, true));
    }

    /// A single cell lying on the ground.
    fn low(&mut self, x: i32, z: i32, voxel: Voxel) {
        self.add([x, 0, z], [x + 1, 1, z + 1], voxel, (false, true));
    }

    /// Four leaves lying round the edge of the footprint, turning one way round it like a
    /// pinwheel, which leave the middle four cells open for the plant's stems.
    fn rosette(&mut self, height: i32, voxel: Voxel) {
        for (lo, hi) in [([0, 0], [2, 1]), ([3, 0], [4, 2]), ([2, 3], [4, 4]), ([0, 2], [1, 4])] {
            self.add([lo[0], 0, lo[1]], [hi[0], height, hi[1]], voxel, (false, true));
        }
    }
}

/// The four directions across the ground.
const SIDES: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

/// The four 2x2 quarters of a voxel's footprint, by their low corners.
const QUARTERS: [(i32, i32); 4] = [(0, 0), (2, 0), (0, 2), (2, 2)];

/// The quarters in random order.
fn shuffled_quarters(rng: &mut Rng) -> [(i32, i32); 4] {
    let mut q = QUARTERS;
    for i in 0..3 {
        q.swap(i, rng.int(i as i32, 3) as usize);
    }
    q
}

fn lawn(p: &mut Plot, rng: &mut Rng, main: Voxel, other: Voxel) {
    for (qx, qz) in QUARTERS {
        if rng.unit() < 0.2 {
            continue;
        }
        let colour = if rng.unit() < 0.3 { other } else { main };
        let height = rng.int(1, 2);
        // A low mat over half the quarter, and sometimes a taller blade in the other half.
        let across = rng.unit() < 0.5;
        let (w, d) = if across { (2, 1) } else { (1, 2) };
        p.add([qx, 0, qz], [qx + w, height, qz + d], colour, (false, true));
        if rng.unit() < 0.5 {
            let along = rng.int(0, 1);
            let (x, z) = if across { (qx + along, qz + 1) } else { (qx + 1, qz + along) };
            p.stem(x, z, rng.int(2, 3), colour);
        }
    }
}

fn meadow(p: &mut Plot, rng: &mut Rng, main: Voxel, other: Voxel) {
    let quarters = shuffled_quarters(rng);
    let stems = rng.int(3, 4) as usize;
    for (i, &(qx, qz)) in quarters[..stems].iter().enumerate() {
        let (x, z) = (qx + rng.int(0, 1), qz + rng.int(0, 1));
        let height = rng.int(5, 9);
        p.stem(x, z, height, if i % 3 == 2 { other } else { main });
        if i < 2 && rng.unit() < 0.75 {
            p.add([x, height, z], [x + 1, height + 3, z + 1], mat::SEED_HEAD, (true, true));
        }
    }
}

fn broad(p: &mut Plot, rng: &mut Rng) {
    let quarters = shuffled_quarters(rng);
    let blades = rng.int(3, 4) as usize;
    for (i, &(qx, qz)) in quarters[..blades].iter().enumerate() {
        let colour = if i % 3 == 2 { mat::TUFT } else { mat::TUFT_LUSH };
        let height = rng.int(3, 6);
        let across = rng.unit() < 0.5;
        let shift = rng.int(0, 1);
        // Two cells wide and one thick, narrowing to a single cell at the tip.
        let (x, z, w, d) = if across { (qx, qz + shift, 2, 1) } else { (qx + shift, qz, 1, 2) };
        p.add([x, 0, z], [x + w, height, z + d], colour, (false, true));
        let along = rng.int(0, 1);
        let (tx, tz) = if across { (x + along, z) } else { (x, z + along) };
        p.add([tx, height, tz], [tx + 1, height + rng.int(1, 2), tz + 1], colour, (true, true));
    }
}

fn bunch(p: &mut Plot, rng: &mut Rng) {
    for (x, z) in [(1, 1), (2, 1), (1, 2), (2, 2)] {
        if rng.unit() < 0.15 {
            continue;
        }
        let colour = if rng.unit() < 0.4 { mat::TUFT_DRY } else { mat::TUFT_STRAW };
        let height = rng.int(3, 6);
        p.stem(x, z, height, colour);
        // The tip leans out over the edge of the clump.
        if rng.unit() < 0.7 {
            let outward = if rng.unit() < 0.5 {
                (if x == 1 { 0 } else { 3 }, z)
            } else {
                (x, if z == 1 { 0 } else { 3 })
            };
            p.add([outward.0, height, outward.1], [outward.0 + 1, height + rng.int(1, 3), outward.1 + 1], colour, (true, true));
        }
    }
}

fn alpine(p: &mut Plot, rng: &mut Rng, main: Voxel, other: Voxel) {
    // One colour to a block, and each blade alone in its own quarter of the voxel.
    let colour = if rng.unit() < 0.25 { other } else { main };
    let quarters = shuffled_quarters(rng);
    let blades = rng.int(3, 4) as usize;
    for &(qx, qz) in &quarters[..blades] {
        p.stem(qx + rng.int(0, 1), qz + rng.int(0, 1), rng.int(2, 3), colour);
    }
}

fn sedge(p: &mut Plot, rng: &mut Rng) {
    let along_x = rng.unit() < 0.5;
    let line = rng.int(1, 2);
    for i in 0..CELLS {
        let off = line + rng.int(0, 1);
        let (x, z) = if along_x { (i, off) } else { (off, i) };
        let height = rng.int(4, 9);
        p.stem(x, z, height, if rng.unit() < 0.25 { mat::TUFT_LUSH } else { mat::TUFT_SEDGE });
        if rng.unit() < 0.12 {
            p.add([x, height, z], [x + 1, height + 2, z + 1], mat::REED_HEAD, (true, true));
        }
    }
}

fn grass(p: &mut Plot, rng: &mut Rng, kind: Grass, sward: &Sward) {
    match kind {
        Grass::Lawn => lawn(p, rng, sward.main, sward.other),
        Grass::Meadow => meadow(p, rng, sward.main, sward.other),
        Grass::Broad => broad(p, rng),
        Grass::Bunch => bunch(p, rng),
        Grass::Alpine => alpine(p, rng, sward.main, sward.other),
        Grass::Sedge => sedge(p, rng),
    }
}

fn flower(p: &mut Plot, rng: &mut Rng, petals: Voxel) {
    let (x, z) = (rng.int(1, CELLS - 2), rng.int(1, CELLS - 2));
    let height = rng.int(3, 5);
    p.stem(x, z, height, mat::TUFT_LUSH);
    // A leaf partway up.
    let side = if rng.unit() < 0.5 { 1 } else { -1 };
    p.add([x + side, 1, z], [x + side + 1, 2, z + 1], mat::TUFT, (false, true));
    // Petals in a cross, with a contrasting eye on top.
    p.add([x - 1, height, z], [x + 2, height + 1, z + 1], petals, (true, true));
    p.add([x, height, z - 1], [x + 1, height + 1, z], petals, (true, true));
    p.add([x, height, z + 1], [x + 1, height + 1, z + 2], petals, (true, true));
    let eye = if petals == mat::BLOOM_YELLOW { mat::REED_HEAD } else { mat::BLOOM_YELLOW };
    p.add([x, height + 1, z], [x + 1, height + 2, z + 1], eye, (true, true));
}

fn fern(p: &mut Plot, rng: &mut Rng) {
    // Four fronds rise from the crown in the middle, and each arches over to the side
    // next round from the last, where its paler tip hangs: a shuttlecock, seen from above
    // a pinwheel.
    for (x, z, tx, tz) in [(1, 1, 0, 1), (2, 1, 2, 0), (2, 2, 3, 2), (1, 2, 1, 3)] {
        if rng.unit() < 0.1 {
            continue;
        }
        let height = rng.int(3, 6);
        p.stem(x, z, height, mat::FERN);
        let droop = rng.int(1, 2).min(height - 2);
        p.add([tx, height - droop, tz], [tx + 1, height, tz + 1], mat::FERN_TIP, (true, true));
    }
    // A young frond or two low in the corners.
    for (x, z) in [(0, 0), (3, 0), (0, 3), (3, 3)] {
        if rng.unit() < 0.35 {
            p.add([x, 0, z], [x + 1, rng.int(1, 2), z + 1], mat::FERN_TIP, (false, true));
        }
    }
}

fn rush(p: &mut Plot, rng: &mut Rng) {
    // A rosette: broad dark blades, two cells wide, lying low around the edge and turning
    // one way round it, with a stem or two standing up out of the open middle.
    for (lo, hi) in [([0, 0], [2, 1]), ([3, 0], [4, 2]), ([2, 3], [4, 4]), ([0, 2], [1, 4])] {
        if rng.unit() < 0.2 {
            continue;
        }
        p.add([lo[0], 0, lo[1]], [hi[0], rng.int(1, 3), hi[1]], mat::WOODRUSH, (false, true));
    }
    let middle = shuffled_quarters(rng);
    for &(qx, qz) in &middle[..rng.int(1, 2) as usize] {
        p.stem(1 + qx / 2, 1 + qz / 2, rng.int(3, 5), mat::WOODRUSH);
    }
}

/// A cushion of heather: woody and green all year, with sprigs standing out of it that
/// are purple in flower (where the ground says it is `flowering`) and rust once spent.
fn heather(p: &mut Plot, rng: &mut Rng, flowering: bool) {
    for (qx, qz) in QUARTERS {
        if rng.unit() < 0.15 {
            continue;
        }
        let height = rng.int(1, 3);
        // Most fill their quarter; some only half of it, so the edge is ragged.
        let (w, d) = match rng.int(0, 5) {
            0 => (1, 2),
            1 => (2, 1),
            _ => (2, 2),
        };
        p.add([qx, 0, qz], [qx + w, height, qz + d], mat::HEATHER_GREEN, (false, true));
        if rng.unit() < 0.75 {
            let (x, z) = (qx + rng.int(0, w - 1), qz + rng.int(0, d - 1));
            let sprig = if flowering && rng.unit() < 0.65 { mat::BLOOM_PURPLE } else { mat::HEATHER_RUST };
            p.add([x, height, z], [x + 1, height + rng.int(1, 2), z + 1], sprig, (true, true));
        }
    }
}

fn reeds(p: &mut Plot, rng: &mut Rng) {
    // Each stem has a quarter of the voxel to itself, room for a head around it.
    let mut quarters = [(0, 0), (2, 0), (0, 2), (2, 2)];
    let stems = rng.int(2, 3) as usize;
    for i in 0..stems {
        quarters.swap(i, rng.int(i as i32, 3) as usize);
    }
    for &(qx, qz) in &quarters[..stems] {
        let (x, z) = (qx + rng.int(0, 1), qz + rng.int(0, 1));
        let height = rng.int(10, 20);
        if rng.unit() < 0.5 {
            // The stem stops under the head and a spike carries on above it.
            let (below, above) = (height - 5, height - 1);
            p.stem(x, z, below, mat::REED);
            p.add([qx, below, qz], [qx + 2, above, qz + 2], mat::REED_HEAD, (true, true));
            p.add([x, above, z], [x + 1, height, z + 1], mat::REED, (true, true));
        } else {
            p.stem(x, z, height, mat::REED);
        }
    }
}

fn mushroom(p: &mut Plot, rng: &mut Rng, cap: Voxel) {
    let (x, z) = (rng.int(1, CELLS - 2), rng.int(1, CELLS - 2));
    p.add([x, 0, z], [x + 1, 2, z + 1], mat::MUSHROOM_STEM, (false, false));
    p.add([x - 1, 2, z - 1], [x + 2, 3, z + 2], cap, (false, false));
    p.add([x, 3, z], [x + 1, 4, z + 1], cap, (false, false));
}

/// A pad floating on the surface plane `y` (in cells, relative to the plot), with a
/// flower open on it if `bloom`.
fn lily(p: &mut Plot, rng: &mut Rng, y: i32, bloom: bool) {
    let top = y;
    let size = rng.int(3, CELLS);
    let (x, z) = (rng.int(0, CELLS - size), rng.int(0, CELLS - size));
    p.add([x, top - 1, z], [x + size, top, z + size - 1], mat::LILY, (false, false));
    // The other row is shorter, leaving the notch a lily pad has.
    p.add([x, top - 1, z + size - 1], [x + size - 1, top, z + size], mat::LILY, (false, false));
    if bloom {
        p.add([x + 1, top, z + 1], [x + 2, top + 1, z + 2], mat::BLOOM_PINK, (false, false));
    }
}

/// A cell of the middle four, where a plant's stem stands clear of a rosette.
fn middle(rng: &mut Rng) -> (i32, i32) {
    (rng.int(1, 2), rng.int(1, 2))
}

fn bluebell(p: &mut Plot, rng: &mut Rng) {
    // Strap leaves lying along two edges.
    let s = rng.int(0, 1);
    p.add([s, 0, 0], [s + 3, 1, 1], mat::LEAF_GLOSS, (false, true));
    let s = rng.int(0, 1);
    p.add([s, 0, 3], [s + 3, 1, 4], mat::LEAF_GLOSS, (false, true));
    // Two or three stems, each arching over at the top to one side, where the bells hang
    // one under another from the crook.
    for (i, (x, z, dx, dz)) in [(1, 1, -1, 0), (2, 2, 1, 0), (1, 2, 0, 1)].into_iter().enumerate() {
        if i == 2 && rng.unit() < 0.5 {
            continue;
        }
        let height = rng.int(5, 7);
        p.stem(x, z, height, mat::STEM);
        let (bx, bz) = (x + dx, z + dz);
        // The crook itself is the first bell, the next hangs under it, and a third lower
        // down the side of the stem on the taller ones.
        p.add([bx, height - 2, bz], [bx + 1, height, bz + 1], mat::BLOOM_BLUEBELL, (true, true));
        if height == 7 {
            p.dot(bx, height - 4, bz, mat::BLOOM_BLUEBELL);
        }
    }
}

fn foxglove(p: &mut Plot, rng: &mut Rng) {
    p.rosette(rng.int(1, 2), mat::LEAF_GLOSS);
    let (x, z) = middle(rng);
    let height = rng.int(9, 13);
    p.stem(x, z, height, mat::STEM);
    // Bells all down one side of the spike, from below its green tip, and every other
    // one on the side next round; the lowest are open and paler at the mouth.
    let first = rng.int(0, 3) as usize;
    let (dx, dz) = SIDES[first];
    let (ex, ez) = SIDES[(first + 2) % 4];
    let lowest = (height - 7).max(3);
    for y in lowest..height - 2 {
        let colour = if y == lowest { mat::BLOOM_ORCHID } else { mat::BLOOM_FOXGLOVE };
        p.dot(x + dx, y, z + dz, colour);
        if (y - lowest) % 2 == 1 {
            p.dot(x + ex, y, z + ez, mat::BLOOM_FOXGLOVE);
        }
    }
}

fn primrose(p: &mut Plot, rng: &mut Rng) {
    p.rosette(1, mat::LEAF_GLOSS);
    // Flowers on short stalks in the open middle, a clump of pale yellow.
    for (i, (x, z)) in [(1, 1), (2, 1), (1, 2), (2, 2)].into_iter().enumerate() {
        if i > 0 && rng.unit() < 0.25 {
            continue;
        }
        let stalk = rng.int(1, 2);
        p.stem(x, z, stalk, mat::STEM);
        p.dot(x, stalk, z, mat::BLOOM_PRIMROSE);
    }
}

fn daisy(p: &mut Plot, rng: &mut Rng) {
    let (x, z) = middle(rng);
    let height = rng.int(2, 3);
    p.stem(x, z, height, mat::STEM);
    // A flat white star at the top with its yellow eye in the middle of it.
    p.dot(x, height, z, mat::BLOOM_GOLD);
    for (dx, dz) in SIDES {
        p.dot(x + dx, height, z + dz, mat::BLOOM_WHITE);
    }
    // Leaves lying under it, and a second, smaller head low beside it.
    p.low(x - 1, z, mat::LEAF_GLOSS);
    p.low(x + 1, z, mat::LEAF_GLOSS);
    let (cx, cz) = (x + if rng.unit() < 0.5 { 1 } else { -1 }, z + if rng.unit() < 0.5 { 1 } else { -1 });
    if rng.unit() < 0.6 {
        p.low(cx, cz, mat::STEM);
        p.dot(cx, 1, cz, mat::BLOOM_WHITE);
    }
}

fn buttercup(p: &mut Plot, rng: &mut Rng) {
    // Cut leaves on the ground at two corners.
    p.add([0, 0, 0], [2, 1, 1], mat::LEAF_GLOSS, (false, true));
    p.add([2, 0, 3], [4, 1, 4], mat::LEAF_GLOSS, (false, true));
    let (x, z) = middle(rng);
    let height = rng.int(5, 7);
    p.stem(x, z, height, mat::STEM);
    p.dot(x, height, z, mat::BLOOM_GOLD);
    // Side shoots, each ending in a cup, lower down on two sides.
    let first = rng.int(0, 3) as usize;
    let (dx, dz) = SIDES[first];
    p.dot(x + dx, height - 3, z + dz, mat::STEM);
    p.dot(x + dx, height - 2, z + dz, mat::BLOOM_GOLD);
    if height >= 6 {
        let (ex, ez) = SIDES[(first + 2) % 4];
        p.dot(x + ex, height - 5, z + ez, mat::STEM);
        p.dot(x + ex, height - 4, z + ez, mat::BLOOM_GOLD);
    }
}

fn poppy(p: &mut Plot, rng: &mut Rng) {
    let quarters = shuffled_quarters(rng);
    // A wiry stem with a scarlet cup on it, and a dark boss in the middle of the cup.
    let (qx, qz) = quarters[0];
    let (x, z) = (qx + rng.int(0, 1), qz + rng.int(0, 1));
    let height = rng.int(6, 8);
    p.stem(x, z, height, mat::STEM);
    p.add([qx, height, qz], [qx + 2, height + 1, qz + 2], mat::BLOOM_POPPY, (true, true));
    p.dot(x, height + 1, z, mat::BLOOM_EYE);
    // A second, lower: open, or a nodding bud.
    let (qx, qz) = quarters[1];
    let (x, z) = (qx + rng.int(0, 1), qz + rng.int(0, 1));
    let height = rng.int(4, 5);
    p.stem(x, z, height, mat::STEM);
    if rng.unit() < 0.5 {
        p.add([qx, height, qz], [qx + 2, height + 1, qz + 2], mat::BLOOM_POPPY, (true, true));
    } else {
        p.dot(x, height, z, mat::LEAF_GREY);
    }
    // Grey-green cut leaves low in another quarter.
    let (qx, qz) = quarters[2];
    p.add([qx, 0, qz], [qx + 2, 1, qz + 1], mat::LEAF_GREY, (false, true));
}

fn lupin(p: &mut Plot, rng: &mut Rng) {
    let (x, z) = middle(rng);
    let stalk = rng.int(2, 3);
    p.stem(x, z, stalk, mat::STEM);
    // A hand of leaves round the stalk.
    for (dx, dz) in SIDES {
        p.dot(x + dx, 1, z + dz, mat::LEAF_GLOSS);
    }
    let colour = match rng.unit() {
        u if u < 0.2 => mat::BLOOM_PINK,
        u if u < 0.28 => mat::BLOOM_WHITE,
        _ => mat::BLOOM_LUPIN,
    };
    // The spike, thick below and tapering to its tip.
    let (qx, qz) = (x - rng.int(0, 1), z - rng.int(0, 1));
    let thick = rng.int(3, 4);
    p.add([qx, stalk, qz], [qx + 2, stalk + thick, qz + 2], colour, (true, true));
    p.add([x, stalk + thick, z], [x + 1, stalk + thick + 2, z + 1], colour, (true, true));
}

fn cotton_grass(p: &mut Plot, rng: &mut Rng) {
    let quarters = shuffled_quarters(rng);
    let stems = rng.int(3, 4) as usize;
    for (i, &(qx, qz)) in quarters[..stems].iter().enumerate() {
        let (x, z) = (qx + rng.int(0, 1), qz + rng.int(0, 1));
        let height = rng.int(4, 7);
        p.stem(x, z, height, mat::TUFT_SEDGE);
        if i == 0 || rng.unit() < 0.85 {
            p.add([x, height, z], [x + 1, height + 2, z + 1], mat::BLOOM_WHITE, (true, true));
        }
    }
}

fn marsh_marigold(p: &mut Plot, rng: &mut Rng) {
    // Round glossy leaves held up on stalks in a mound, gold cups sitting over some.
    let quarters = shuffled_quarters(rng);
    let leaves = rng.int(3, 4) as usize;
    for (i, &(qx, qz)) in quarters[..leaves].iter().enumerate() {
        let (x, z) = (qx + rng.int(0, 1), qz + rng.int(0, 1));
        let up = rng.int(1, 2);
        p.stem(x, z, up, mat::STEM);
        p.add([qx, up, qz], [qx + 2, up + 1, qz + 2], mat::LEAF_GLOSS, (true, true));
        if i < 2 {
            p.add([x, up + 1, z], [x + 1, up + 2, z + 1], mat::BLOOM_GOLD, (true, true));
        }
    }
}

fn flag_iris(p: &mut Plot, rng: &mut Rng) {
    // A fan of sword leaves in a row, and the flower stem among them.
    let row = rng.int(1, 2);
    let flip = rng.unit() < 0.5;
    let at = |i: i32| if flip { 3 - i } else { i };
    for (i, (lo, hi)) in [(0, (5, 6)), (1, (7, 9)), (3, (5, 7))] {
        let height = rng.int(lo, hi);
        p.stem(at(i), row, height, mat::TUFT_SEDGE);
    }
    let x = at(2);
    let height = rng.int(8, 10);
    p.stem(x, row, height, mat::STEM);
    // The standard upright on top and the falls drooping either side.
    p.dot(x, height, row, mat::BLOOM_GOLD);
    p.dot(x, height - 1, row - 1, mat::BLOOM_GOLD);
    p.dot(x, height - 1, row + 1, mat::BLOOM_GOLD);
}

fn thrift(p: &mut Plot, rng: &mut Rng) {
    // A tight cushion, domed in the middle, with pink heads on wiry stalks at its corners.
    let (s, t) = (rng.int(0, 1), rng.int(0, 1));
    p.add([s, 0, t], [s + 3, 1, t + 3], mat::TUFT_ALPINE, (false, false));
    p.add([s + 1, 1, t + 1], [s + 2, 2, t + 2], mat::TUFT_ALPINE, (false, true));
    for (i, (cx, cz)) in [(0, 0), (2, 0), (0, 2), (2, 2)].into_iter().enumerate() {
        if i > 0 && rng.unit() < 0.3 {
            continue;
        }
        let top = rng.int(2, 4);
        p.add([s + cx, 1, t + cz], [s + cx + 1, top, t + cz + 1], mat::STEM, (false, true));
        p.dot(s + cx, top, t + cz, mat::BLOOM_PINK);
    }
}

fn gentian(p: &mut Plot, rng: &mut Rng) {
    p.rosette(1, mat::LEAF_GLOSS);
    // Upright trumpets of deep blue, close to the ground.
    let mut shown = 0;
    for (x, z) in [(1, 1), (2, 1), (1, 2), (2, 2)] {
        if shown == 3 || (shown > 0 && rng.unit() < 0.4) {
            continue;
        }
        shown += 1;
        p.low(x, z, mat::STEM);
        p.add([x, 1, z], [x + 1, 3, z + 1], mat::BLOOM_GENTIAN, (true, true));
    }
}

fn edelweiss(p: &mut Plot, rng: &mut Rng) {
    let (x, z) = middle(rng);
    let height = 3;
    // Felted grey leaves at the foot, a grey stem, and a star of felted bracts at the top
    // round a cluster of pale heads, the corner bracts held a little lower.
    for (dx, dz) in SIDES {
        p.low(x + dx, z + dz, mat::LEAF_GREY);
    }
    p.stem(x, z, height, mat::LEAF_GREY);
    p.dot(x, height, z, mat::BLOOM_PRIMROSE);
    for (dx, dz) in SIDES {
        p.dot(x + dx, height, z + dz, mat::BLOOM_FELT);
    }
    for (dx, dz) in [(1, 1), (-1, 1), (1, -1), (-1, -1)] {
        if rng.unit() < 0.75 {
            p.dot(x + dx, height - 1, z + dz, mat::BLOOM_FELT);
        }
    }
}

fn orchid(p: &mut Plot, rng: &mut Rng) {
    // Two broad leaves lying flat, and a stem with flowers turned alternately either way,
    // each a pale face over its dark lip.
    for z in [0, 3] {
        let x = rng.int(0, 2);
        p.add([x, 0, z], [x + 2, 1, z + 1], mat::LEAF_GLOSS, (false, true));
    }
    let (x, z) = middle(rng);
    let height = rng.int(6, 8);
    p.stem(x, z, height, mat::STEM);
    let mut side = if rng.unit() < 0.5 { 1 } else { -1 };
    let mut y = height - 1;
    while y >= 3 {
        p.dot(x + side, y, z, mat::BLOOM_ORCHID);
        p.dot(x + side, y - 1, z, mat::BLOOM_ORCHID_LIP);
        side = -side;
        y -= 2;
    }
}

fn heliconia(p: &mut Plot, rng: &mut Rng) {
    // Paddle leaves standing up along two edges.
    for z in [0, 3] {
        let x = rng.int(0, 2);
        p.add([x, 0, z], [x + 2, rng.int(6, 9), z + 1], mat::LEAF_GLOSS, (false, true));
    }
    // The flower stalk, with red bracts alternating either side of it down from the top,
    // each tipped with gold.
    let (x, z) = middle(rng);
    let height = rng.int(7, 9);
    p.stem(x, z, height, mat::STEM);
    let mut side = if rng.unit() < 0.5 { 1 } else { -1 };
    let mut top = height;
    while top - 2 >= 2 {
        p.add([x + side, top - 2, z], [x + side + 1, top, z + 1], mat::BLOOM_RED, (true, true));
        p.dot(x + side, top, z, mat::BLOOM_GOLD);
        side = -side;
        top -= 2;
    }
}

fn desert_marigold(p: &mut Plot, rng: &mut Rng) {
    // A grey woolly mound with gold heads on stalks round it and one sitting on top.
    p.add([1, 0, 1], [3, 1, 3], mat::LEAF_GREY, (false, false));
    p.dot(rng.int(1, 2), 1, rng.int(1, 2), mat::BLOOM_GOLD);
    for (i, (x, z)) in [(0, 1), (2, 0), (3, 2), (1, 3)].into_iter().enumerate() {
        if i > 0 && rng.unit() < 0.3 {
            continue;
        }
        let stalk = rng.int(1, 2);
        p.stem(x, z, stalk, mat::LEAF_GREY);
        p.dot(x, stalk, z, mat::BLOOM_GOLD);
    }
}

fn thistle(p: &mut Plot, rng: &mut Rng) {
    let quarters = shuffled_quarters(rng);
    let (qx, qz) = quarters[0];
    let (x, z) = (qx + rng.int(0, 1), qz + rng.int(0, 1));
    let height = rng.int(6, 8);
    p.stem(x, z, height, mat::STEM);
    // Spiny leaves standing out from the stem, turning outward from the middle.
    let out = (if x < 2 { -1 } else { 1 }, if z < 2 { -1 } else { 1 });
    let ok = |v: i32| (0..CELLS).contains(&v);
    if ok(x + out.0) {
        p.dot(x + out.0, 1, z, mat::LEAF_GREY);
    }
    if ok(z + out.1) {
        p.dot(x, 3, z + out.1, mat::LEAF_GREY);
    }
    // The head: a bulb of grey bracts and the purple brush on top.
    p.add([qx, height, qz], [qx + 2, height + 1, qz + 2], mat::LEAF_GREY, (true, true));
    p.add([qx, height + 1, qz], [qx + 2, height + 2, qz + 2], mat::BLOOM_THISTLE, (true, true));
    // A smaller head, lower, in the opposite quarter.
    if rng.unit() < 0.6 {
        let (qx, qz) = (2 - qx, 2 - qz);
        let (x, z) = (qx + rng.int(0, 1), qz + rng.int(0, 1));
        let height = rng.int(4, 5);
        p.stem(x, z, height, mat::STEM);
        p.dot(x, height, z, mat::LEAF_GREY);
        p.dot(x, height + 1, z, mat::BLOOM_THISTLE);
    }
}

/// Builds a flower of the kind given.
fn build_flower(p: &mut Plot, rng: &mut Rng, kind: Flower) {
    match kind {
        Flower::Pimpernel | Flower::Cinquefoil | Flower::Stitchwort | Flower::Violet | Flower::Speedwell | Flower::Campion => {
            flower(p, rng, kind.petals())
        }
        Flower::Heather => heather(p, rng, true),
        Flower::FlyAgaric | Flower::PennyBun => mushroom(p, rng, kind.petals()),
        // Built by `grow`, which knows where the water is.
        Flower::WaterLily => {}
        Flower::Bluebell => bluebell(p, rng),
        Flower::Foxglove => foxglove(p, rng),
        Flower::Primrose => primrose(p, rng),
        Flower::Daisy => daisy(p, rng),
        Flower::Buttercup => buttercup(p, rng),
        Flower::Poppy => poppy(p, rng),
        Flower::Lupin => lupin(p, rng),
        Flower::CottonGrass => cotton_grass(p, rng),
        Flower::MarshMarigold => marsh_marigold(p, rng),
        Flower::FlagIris => flag_iris(p, rng),
        Flower::Thrift => thrift(p, rng),
        Flower::Gentian => gentian(p, rng),
        Flower::Edelweiss => edelweiss(p, rng),
        Flower::Orchid => orchid(p, rng),
        Flower::Heliconia => heliconia(p, rng),
        Flower::DesertMarigold => desert_marigold(p, rng),
        Flower::Thistle => thistle(p, rng),
    }
}

/// Adds whatever grows on the column at world position `(x, z)`. `surface` is its top
/// voxel, or the flower the ground carries there.
pub fn grow(col: &Column, surface: Voxel, x: i32, z: i32, seed: u32, out: &mut Vec<PlantBox>) {
    let h = column_hash(seed, x, z);
    let Some(growth) = decide(col, surface, x, z, seed, unit(h)) else {
        return;
    };
    let y = col.top() + 1;
    let mut rng = Rng::new(h.rotate_left(13));
    let mut plot = Plot::new([x * CELLS, y * CELLS, z * CELLS], out);
    if let Growth::Flower(kind) = growth {
        plot.flower = Some(kind);
    }
    let p = &mut plot;
    match growth {
        Growth::Grass(kind, sward) => grass(p, &mut rng, kind, &sward),
        Growth::Tussock => bunch(p, &mut rng),
        Growth::Reeds => reeds(p, &mut rng),
        Growth::Lily | Growth::Flower(Flower::WaterLily) => {
            let k = col.water_surface().unwrap_or(y);
            lily(p, &mut rng, (k - y) * CELLS, matches!(growth, Growth::Flower(_)));
        }
        Growth::Wintergreen(Wintergreen::Fern) => fern(p, &mut rng),
        Growth::Wintergreen(Wintergreen::Rush) => rush(p, &mut rng),
        Growth::Wintergreen(Wintergreen::Heather) => heather(p, &mut rng, false),
        Growth::Flower(kind) => build_flower(p, &mut rng, kind),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(cover: Cover, biome: Biome) -> Column {
        Column {
            ground: 40.0,
            water: None,
            cover,
            biome,
            temperature: 0.5,
            river: f32::MAX,
            clear: f32::MAX,
            worn: None,
        }
    }

    fn grown(col: &Column, surface: Voxel) -> Vec<PlantBox> {
        let mut out = Vec::new();
        for x in 0..300 {
            for z in 0..300 {
                grow(col, surface, x, z, 5, &mut out);
            }
        }
        out
    }

    #[test]
    fn marram_grows_on_dunes_and_thickest_landward() {
        // In the tropics, where no thrift grows among it.
        let mut low = column(Cover::Dune, Biome::Ocean);
        low.ground = m(2.0);
        low.temperature = 0.9;
        let mut high = column(Cover::Dune, Biome::Ocean);
        high.ground = m(5.0);
        high.temperature = 0.9;
        let (seaward, landward) = (grown(&low, mat::SAND), grown(&high, mat::SAND));
        assert!(!seaward.is_empty() && landward.len() > 2 * seaward.len());
        assert!(landward.iter().all(|b| matches!(b.voxel, mat::TUFT_DRY | mat::TUFT_STRAW) && b.thin == 0));
    }

    #[test]
    fn meadows_grow_tufts_and_bare_ground_grows_nothing() {
        let meadow = column(Cover::Land, Biome::Grassland);
        assert!(grown(&meadow, mat::GRASS_MEADOW).iter().any(|p| p.voxel == mat::TUFT_MEADOW));
        assert!(grown(&column(Cover::Land, Biome::Desert), mat::DESERT_SAND).is_empty());
        assert!(grown(&column(Cover::Trail, Biome::Grassland), mat::PATH_DIRT).is_empty());
    }

    #[test]
    fn nothing_grows_close_to_a_trail() {
        let mut col = column(Cover::Land, Biome::Grassland);
        col.clear = 0.0;
        assert!(grown(&col, mat::GRASS_MEADOW).is_empty());
    }

    #[test]
    fn every_kind_of_plant_grows_and_stays_in_its_own_column() {
        let banks = column(Cover::Bank, Biome::Deciduous);
        let mut lake = column(Cover::LakeBed, Biome::Deciduous);
        lake.water = Some(40 + 8 * BLOCK);
        let forest = column(Cover::Land, Biome::OldGrowth);
        let mut all = grown(&banks, mat::GRAVEL);
        all.extend(grown(&lake, mat::MUD));
        all.extend(grown(&forest, mat::GRASS_LUSH));
        all.extend(grown(&column(Cover::Land, Biome::Grassland), mat::FLOWER_BLUE));
        for wanted in [mat::REED, mat::REED_HEAD, mat::LILY, mat::MUSHROOM_STEM, mat::BLOOM_BLUE] {
            assert!(all.iter().any(|p| p.voxel == wanted), "never grew {wanted}");
        }
        for b in &all {
            for axis in [0, 2] {
                assert_eq!(b.lo[axis].div_euclid(CELLS), (b.hi[axis] - 1).div_euclid(CELLS), "{b:?} leaves its column");
            }
            assert!((0..3).all(|axis| b.lo[axis] < b.hi[axis]), "{b:?} is empty");
        }
    }

    #[test]
    fn the_boxes_of_a_plant_never_overlap() {
        let mut lake = column(Cover::LakeBed, Biome::Deciduous);
        lake.water = Some(40 + 8 * BLOCK);
        let kinds = [
            (column(Cover::Bank, Biome::Deciduous), mat::GRAVEL),
            (lake, mat::MUD),
            (column(Cover::Land, Biome::OldGrowth), mat::GRASS_LUSH),
            (column(Cover::Land, Biome::Grassland), mat::GRASS_MEADOW),
            (column(Cover::Land, Biome::Grassland), mat::FLOWER_YELLOW),
            (column(Cover::Land, Biome::Heath), mat::ground_cover(Biome::Heath)),
            (column(Cover::Land, Biome::Heath), mat::HEATHER),
            (column(Cover::Land, Biome::Deciduous), mat::ground_cover(Biome::Deciduous)),
            (column(Cover::Land, Biome::Bog), mat::ground_cover(Biome::Bog)),
            (column(Cover::Land, Biome::Tundra), mat::ground_cover(Biome::Tundra)),
            (column(Cover::Land, Biome::Savanna), mat::ground_cover(Biome::Savanna)),
        ];
        let mut plants = 0;
        for (col, surface) in &kinds {
            for x in 0..300 {
                for z in 0..300 {
                    let mut boxes = Vec::new();
                    grow(col, *surface, x, z, 5, &mut boxes);
                    plants += !boxes.is_empty() as u32;
                    for (i, a) in boxes.iter().enumerate() {
                        for b in &boxes[i + 1..] {
                            let overlap = (0..3).all(|axis| a.lo[axis] < b.hi[axis] && b.lo[axis] < a.hi[axis]);
                            assert!(!overlap, "{a:?} overlaps {b:?}");
                        }
                    }
                }
            }
        }
        assert!(plants > 100);
    }

    #[test]
    fn no_plant_is_a_whole_voxel_block() {
        let mut all = grown(&column(Cover::Land, Biome::Grassland), mat::GRASS_MEADOW);
        all.extend(grown(&column(Cover::Land, Biome::Grassland), mat::FLOWER_RED));
        all.extend(grown(&column(Cover::Bank, Biome::Deciduous), mat::GRAVEL));
        assert!(!all.is_empty());
        for b in &all {
            assert!(b.hi[0] - b.lo[0] < CELLS && b.hi[2] - b.lo[2] < CELLS, "{b:?} is a voxel wide");
        }
    }

    #[test]
    fn wintergreens_grow_in_their_places_and_keep_their_leaves() {
        let mats = mat::build();
        let wood = grown(&column(Cover::Land, Biome::Deciduous), mat::ground_cover(Biome::Deciduous));
        let heath = grown(&column(Cover::Land, Biome::Heath), mat::ground_cover(Biome::Heath));
        let count = |boxes: &[PlantBox], v: Voxel| boxes.iter().filter(|b| b.voxel == v).count();
        assert!(count(&wood, mat::FERN) > 100 && count(&wood, mat::WOODRUSH) > 50, "a wood grows ferns and wood-rush");
        assert_eq!(count(&wood, mat::HEATHER_GREEN), 0, "no heather under trees");
        assert!(count(&heath, mat::HEATHER_GREEN) > 100 && count(&heath, mat::FERN) == 0, "a heath grows heather, not ferns");
        // They are a small part of what grows, so the summer floor is still grass.
        assert!(count(&wood, mat::FERN) + count(&wood, mat::WOODRUSH) < wood.len() / 2);
        // Heather in flower is purple over the flowering ground, and green under it all year.
        let drift = grown(&column(Cover::Land, Biome::Heath), mat::HEATHER);
        assert!(count(&drift, mat::BLOOM_PURPLE) > 100 && count(&drift, mat::HEATHER_GREEN) > count(&drift, mat::BLOOM_PURPLE));
        for v in [mat::FERN, mat::FERN_TIP, mat::WOODRUSH, mat::HEATHER_GREEN, mat::HEATHER_RUST] {
            assert_eq!(mats.get(v).plant, voxel::plant::EVERGREEN, "{v} must stay through the winter");
        }
        for b in wood.iter().chain(&heath).chain(&drift) {
            assert!(b.hi[0] - b.lo[0] < CELLS && b.hi[2] - b.lo[2] < CELLS, "{b:?} is a voxel wide");
            for axis in [0, 2] {
                assert_eq!(b.lo[axis].div_euclid(CELLS), (b.hi[axis] - 1).div_euclid(CELLS), "{b:?} leaves its column");
            }
        }
    }

    const ALL_GRASSES: [Grass; 6] = [Grass::Lawn, Grass::Meadow, Grass::Broad, Grass::Bunch, Grass::Alpine, Grass::Sedge];

    fn model(kind: Grass, salt: u32) -> Vec<PlantBox> {
        let mut out = Vec::new();
        let mut plot = Plot::new([0, 0, 0], &mut out);
        grass(&mut plot, &mut Rng::new(hash3(77, salt as i32, 0, 3)), kind, &sward(Biome::Grassland).unwrap());
        out
    }

    #[test]
    fn every_grass_is_well_formed() {
        for kind in ALL_GRASSES {
            for salt in 0..2000 {
                let boxes = model(kind, salt);
                for (i, a) in boxes.iter().enumerate() {
                    assert!((0..3).all(|axis| a.lo[axis] < a.hi[axis]), "{kind:?}: {a:?} is empty");
                    for axis in [0, 2] {
                        assert!(a.lo[axis] >= 0 && a.hi[axis] <= CELLS, "{kind:?}: {a:?} leaves its column");
                    }
                    assert!(a.hi[0] - a.lo[0] < CELLS && a.hi[2] - a.lo[2] < CELLS, "{kind:?}: {a:?} is a voxel wide");
                    for b in &boxes[i + 1..] {
                        let overlap = (0..3).all(|axis| a.lo[axis] < b.hi[axis] && b.lo[axis] < a.hi[axis]);
                        assert!(!overlap, "{kind:?}: {a:?} overlaps {b:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_grasses_differ_in_shape() {
        let height = |kind| (0..500).map(|s| model(kind, s).iter().map(|b| b.hi[1]).max().unwrap_or(0) as f32).sum::<f32>() / 500.0;
        assert!(height(Grass::Meadow) > height(Grass::Lawn) * 2.0);
        assert!(height(Grass::Sedge) > height(Grass::Bunch));
        assert!((0..500).all(|s| model(Grass::Alpine, s).iter().all(|b| b.hi[1] <= 3)), "alpine grass grows low");
        for salt in 0..500 {
            let blades = model(Grass::Alpine, salt);
            assert!((3..=4).contains(&blades.len()));
            assert!(blades.iter().all(|b| b.hi[0] - b.lo[0] == 1 && b.hi[2] - b.lo[2] == 1), "one microvoxel across");
        }
        for salt in 0..200 {
            assert!(model(Grass::Broad, salt).iter().any(|b| b.hi[0] - b.lo[0] == 2 || b.hi[2] - b.lo[2] == 2));
            assert!(model(Grass::Sedge, salt).iter().all(|b| b.hi[0] - b.lo[0] == 1 && b.hi[2] - b.lo[2] == 1));
        }
    }

    #[test]
    fn grass_kinds_form_drifts_and_sedge_keeps_to_the_water() {
        let dry = column(Cover::Land, Biome::Grassland);
        let at = |x, z| grass_at(&dry, 5, x, z).unwrap().0;
        let (mut lawn, mut meadow, mut neighbours_agree, mut total) = (0, 0, 0, 0);
        for x in 0..1500 {
            for z in 0..1500 {
                match at(x, z) {
                    Grass::Lawn => lawn += 1,
                    Grass::Meadow => meadow += 1,
                    other => panic!("{other:?} in a dry meadow"),
                }
                if x % 7 == 0 && z % 7 == 0 {
                    total += 1;
                    neighbours_agree += (at(x, z) == at(x + 1, z)) as u32;
                }
            }
        }
        assert!(lawn > 100_000 && meadow > 100_000, "lawn {lawn}, meadow {meadow}");
        assert!(neighbours_agree as f32 > total as f32 * 0.9, "{neighbours_agree} of {total}");

        let mut wet = column(Cover::Land, Biome::Grassland);
        wet.river = 1.0;
        assert_eq!(grass_at(&wet, 5, 10, 10).unwrap().0, Grass::Sedge);
    }

    #[test]
    fn the_cold_and_the_heights_grow_alpine_grass() {
        // A warm-climate biome high enough up to be cold.
        let mut high = column(Cover::Land, Biome::Grassland);
        high.temperature = 0.26;
        assert_eq!(grass_at(&high, 5, 40, 40).unwrap().0, Grass::Alpine);

        let tundra = column(Cover::Land, Biome::Tundra);
        assert!(grown(&tundra, mat::ground_cover(Biome::Tundra)).iter().any(|p| p.voxel == mat::TUFT_ALPINE));
    }

    #[test]
    fn a_wash_is_given_wherever_a_plant_that_shows_from_afar_grows() {
        for (biome, surface) in [(Biome::Moor, mat::HEATHER), (Biome::Moor, mat::GRASS_MOOR), (Biome::Grassland, mat::FLOWER_RED), (Biome::Deciduous, mat::GRASS)] {
            let col = column(Cover::Land, biome);
            let surface = if surface == mat::GRASS_MOOR || surface == mat::GRASS { mat::ground_cover(biome) } else { surface };
            let (mut washed, mut bare) = (0, 0);
            for x in 0..60 {
                for z in 0..60 {
                    let mut out = Vec::new();
                    grow(&col, surface, x, z, 7, &mut out);
                    let code = wash_at(&col, surface, x, z, 7);
                    assert!(usize::from(code) <= washes().len());
                    // Everything these grow shows from afar, so a wash goes with every plant.
                    // (Now and then a heather comes up with no cushions at all.)
                    assert!(code != 0 || out.is_empty(), "{biome:?} at {x},{z}");
                    if code != 0 { washed += 1 } else { bare += 1 }
                }
            }
            assert!(washed > 20, "{biome:?}: {washed} washed, {bare} bare");
        }
        // Flowering heather and heather out of flower are told apart, and each wash is listed once.
        let all = washes();
        assert!(all.iter().any(|w| w.bloom.0 == mat::BLOOM_PURPLE && w.body.0 == mat::HEATHER_GREEN));
        assert!(all.iter().any(|w| w.bloom.1 == 0.0 && w.body.0 == mat::HEATHER_GREEN));
        for (i, w) in all.iter().enumerate() {
            assert!(!all[..i].contains(w));
            assert!(w.cover > 0.0 && w.cover <= 1.0 && (0.0..=1.0).contains(&w.bloom.1));
        }
        // Lily pads lie on water and mushrooms are too few: neither colours the ground.
        assert!(wash_of(&Growth::Lily).is_none() && wash_of(&Growth::Flower(Flower::FlyAgaric)).is_none());
    }

    #[test]
    fn far_land_takes_the_colour_of_its_drifts() {
        let (mut tinted, mut heather) = (0, 0);
        for x in 0..80 {
            for z in 0..80 {
                if let Some((code, share)) = far_wash(Biome::Moor, x * 40, z * 40, 3) {
                    assert!((0.0..=1.0).contains(&share));
                    let wash = washes()[code as usize - 1];
                    if share > 0.05 {
                        tinted += 1;
                        // Heather that flowers, which the far view turns purple in its season.
                        if wash.season == Flower::Heather.bloom_code() {
                            assert_eq!(wash.bloom.0, mat::BLOOM_PURPLE);
                            heather += 1;
                        }
                    }
                }
            }
        }
        assert!(tinted > 300 && tinted < 5000, "{tinted} of 6400 cells tinted");
        assert!(heather > 50, "{heather} cells of flowering heather");
        // Bare lava grows nothing to colour it.
        assert!(far_wash(Biome::Volcanic, 100, 100, 3).is_none());
    }

    #[test]
    fn the_same_column_always_grows_the_same_plant() {
        let col = column(Cover::Land, Biome::Grassland);
        let (a, b) = (grown(&col, mat::GRASS_MEADOW), grown(&col, mat::GRASS_MEADOW));
        assert_eq!(a.len(), b.len());
        assert!(a.iter().zip(&b).all(|(p, q)| p.lo == q.lo && p.hi == q.hi && p.voxel == q.voxel));
    }

    /// Builds one flower of a kind on its own, for looking at its shape.
    fn flower_model(kind: Flower, salt: u32) -> Vec<PlantBox> {
        let mut out = Vec::new();
        let mut plot = Plot::new([0, 0, 0], &mut out);
        plot.flower = Some(kind);
        build_flower(&mut plot, &mut Rng::new(hash3(91, salt as i32, 0, 7)), kind);
        out
    }

    #[test]
    fn every_flower_is_well_formed_and_none_is_a_single_voxel() {
        for &kind in Flower::ALL {
            if kind == Flower::WaterLily {
                continue;
            }
            for salt in 0..2000 {
                let boxes = flower_model(kind, salt);
                assert!(kind == Flower::Heather || boxes.len() >= 3, "{kind:?} is too plain: {boxes:?}");
                if kind != Flower::Heather {
                    // Lupins come in pink and white as well.
                    let bloom = |v| v == kind.petals() || (kind == Flower::Lupin && matches!(v, mat::BLOOM_PINK | mat::BLOOM_WHITE));
                    assert!(boxes.iter().any(|b| bloom(b.voxel)), "{kind:?} grew no bloom");
                }
                for (i, a) in boxes.iter().enumerate() {
                    assert!((0..3).all(|axis| a.lo[axis] < a.hi[axis]), "{kind:?}: {a:?} is empty");
                    assert!(a.lo[1] >= 0, "{kind:?}: {a:?} is under the ground");
                    for axis in [0, 2] {
                        assert!(a.lo[axis] >= 0 && a.hi[axis] <= CELLS, "{kind:?}: {a:?} leaves its column");
                    }
                    assert!(a.hi[0] - a.lo[0] < CELLS && a.hi[2] - a.lo[2] < CELLS, "{kind:?}: {a:?} is a voxel wide");
                    assert_eq!(a.thin, 0);
                    for b in &boxes[i + 1..] {
                        let overlap = (0..3).all(|axis| a.lo[axis] < b.hi[axis] && b.lo[axis] < a.hi[axis]);
                        assert!(!overlap, "{kind:?}: {a:?} overlaps {b:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_new_flowers_differ_in_shape() {
        let height = |kind| (0..300).map(|s| flower_model(kind, s).iter().map(|b| b.hi[1]).max().unwrap() as f32).sum::<f32>() / 300.0;
        assert!(height(Flower::Foxglove) > 2.0 * height(Flower::Primrose));
        assert!(height(Flower::Daisy) < height(Flower::Poppy));
        assert!(height(Flower::Gentian) <= 3.0 && height(Flower::Edelweiss) <= 4.0, "the alpine flowers keep low");
        assert!(height(Flower::FlagIris) > height(Flower::MarshMarigold) * 2.0);
    }

    /// The kinds of flower growing over a stretch of one sort of ground.
    fn census(col: &Column, surface: Voxel) -> std::collections::HashMap<Flower, u32> {
        let mut found = std::collections::HashMap::new();
        for x in 0..300 {
            for z in 0..300 {
                if let Some(f) = flower_at(col, surface, x, z, 5) {
                    *found.entry(f).or_insert(0) += 1;
                }
            }
        }
        found
    }

    #[test]
    fn each_flower_grows_where_it_belongs_and_not_elsewhere() {
        let land = |biome| column(Cover::Land, biome);
        let has = |col: &Column, surface, kind: Flower| census(col, surface).contains_key(&kind);
        // Drifts of colour name the flower by the biome.
        assert!(has(&land(Biome::Deciduous), mat::FLOWER_BLUE, Flower::Bluebell));
        assert!(!has(&land(Biome::Grassland), mat::FLOWER_BLUE, Flower::Bluebell), "no bluebells out in the open");
        assert!(has(&land(Biome::Grassland), mat::FLOWER_RED, Flower::Poppy));
        assert!(has(&land(Biome::AlpineMeadow), mat::FLOWER_BLUE, Flower::Gentian));
        assert!(has(&land(Biome::Bog), mat::FLOWER_WHITE, Flower::CottonGrass));
        assert!(has(&land(Biome::Rainforest), mat::FLOWER_PINK, Flower::Orchid));
        assert!(has(&land(Biome::Desert), mat::FLOWER_YELLOW, Flower::DesertMarigold));
        assert!(has(&land(Biome::Steppe), mat::FLOWER_PURPLE, Flower::Thistle));
        // Over the plain ground: foxgloves by woods, edelweiss high up, and neither elsewhere.
        let wood = census(&land(Biome::Deciduous), mat::ground_cover(Biome::Deciduous));
        let foxgloves = wood.get(&Flower::Foxglove).copied().unwrap_or(0);
        assert!(foxgloves > 50 && foxgloves < 90_000 / 100, "{foxgloves} foxgloves");
        assert!(!has(&land(Biome::Steppe), mat::ground_cover(Biome::Steppe), Flower::Foxglove));
        let alps = census(&land(Biome::AlpineMeadow), mat::ground_cover(Biome::AlpineMeadow));
        let edelweiss = alps.get(&Flower::Edelweiss).copied().unwrap_or(0);
        assert!(edelweiss > 20 && edelweiss < 90_000 / 200, "edelweiss is a rare find: {edelweiss}");
        assert!(!has(&land(Biome::Grassland), mat::ground_cover(Biome::Grassland), Flower::Edelweiss));
        // By temperate water, kingcups and irises; not by tropical water.
        let bank = column(Cover::Bank, Biome::Deciduous);
        let banks = census(&bank, mat::ground_cover(Biome::Deciduous));
        assert!(banks.contains_key(&Flower::MarshMarigold) && banks.contains_key(&Flower::FlagIris), "{banks:?}");
        let mut hot = column(Cover::Bank, Biome::Rainforest);
        hot.temperature = 0.9;
        assert!(census(&hot, mat::ground_cover(Biome::Rainforest)).is_empty());
        // Thrift on the dunes and shingle of cool coasts, not hot ones.
        assert!(has(&column(Cover::Dune, Biome::Ocean), mat::SAND, Flower::Thrift));
        assert!(has(&column(Cover::Shingle, Biome::Ocean), mat::GRAVEL, Flower::Thrift));
        let mut tropic = column(Cover::Dune, Biome::Ocean);
        tropic.temperature = 0.9;
        assert!(!has(&tropic, mat::SAND, Flower::Thrift));
    }

    #[test]
    fn flower_at_names_what_grows() {
        let mut lake = column(Cover::LakeBed, Biome::Deciduous);
        lake.water = Some(40 + 8 * BLOCK);
        let places = [
            (column(Cover::Land, Biome::Grassland), mat::FLOWER_YELLOW),
            (column(Cover::Land, Biome::Grassland), mat::FLOWER_WHITE),
            (column(Cover::Land, Biome::Deciduous), mat::FLOWER_BLUE),
            (column(Cover::Land, Biome::Deciduous), mat::ground_cover(Biome::Deciduous)),
            (column(Cover::Land, Biome::OldGrowth), mat::ground_cover(Biome::OldGrowth)),
            (column(Cover::Land, Biome::Heath), mat::HEATHER),
            (column(Cover::Land, Biome::Heath), mat::ground_cover(Biome::Heath)),
            (column(Cover::Bank, Biome::Deciduous), mat::ground_cover(Biome::Deciduous)),
            (column(Cover::Dune, Biome::Ocean), mat::SAND),
            (lake, mat::MUD),
        ];
        // Every bloom but heather's purple, which its sprigs show only on flowering ground.
        let blooms: Vec<Voxel> = Flower::ALL.iter().map(|f| f.petals()).filter(|&v| v != mat::BLOOM_PURPLE).collect();
        let mut seen = std::collections::HashSet::new();
        for (col, surface) in &places {
            for x in 0..200 {
                for z in 0..200 {
                    let mut boxes = Vec::new();
                    grow(col, *surface, x, z, 5, &mut boxes);
                    match flower_at(col, *surface, x, z, 5) {
                        Some(f) => {
                            seen.insert(f);
                            // Heather can have every sprig gone to rust.
                            assert!(f == Flower::Heather || boxes.iter().any(|b| b.voxel == f.petals()), "{f:?} at {x},{z} grew {boxes:?}");
                        }
                        None => assert!(boxes.iter().all(|b| !blooms.contains(&b.voxel)), "a flower grew at {x},{z} unnamed: {boxes:?}"),
                    }
                }
            }
        }
        for f in [Flower::WaterLily, Flower::FlyAgaric, Flower::PennyBun, Flower::Heather, Flower::Bluebell, Flower::Foxglove, Flower::Thrift, Flower::MarshMarigold] {
            assert!(seen.contains(&f), "never found {f:?}");
        }
    }

    /// The middle of month `m`, as a date.
    fn mid(m: u8) -> f32 {
        (month_start(m) + 1.0 / 24.0).rem_euclid(1.0)
    }

    #[test]
    fn the_calendar_puts_the_months_round_the_equinox() {
        // The year starts at the spring equinox, late in March; midsummer is in June,
        // the autumn equinox in September and midwinter in December.
        assert_eq!((month_of(0.0), month_of(0.25), month_of(0.5), month_of(0.75)), (3, 6, 9, 12));
        for m in 1..=12 {
            assert_eq!(month_of(mid(m)), m);
            assert_eq!(month_of(month_start(m) + 0.001), m);
        }
        assert!((month_start(13) - month_start(1)).abs() < 1e-6);
    }

    #[test]
    fn every_flower_has_a_season_of_its_own() {
        for (i, &f) in Flower::ALL.iter().enumerate() {
            assert_eq!(f as usize, i, "Flower::ALL is out of order at {f:?}");
            let (first, last) = f.months();
            assert!((1..=12).contains(&first) && (1..=12).contains(&last), "{f:?}");
            let (start, end) = f.season();
            assert!((0.0..=1.0).contains(&start) && (0.0..=1.0).contains(&end), "{f:?}");
            let months = (last as i32 - first as i32).rem_euclid(12) + 1;
            if f.all_year() {
                assert!((0..40).all(|k| f.in_bloom(k as f32 / 40.0) && f.blooming_at(k as f32 / 40.0, k, 3)));
                continue;
            }
            // Out for two months at the least, and not for a month at the least.
            assert!((2..=11).contains(&months), "{f:?} flowers for {months} months");
            // In flower in the middle of each of its months, and not in the months either side.
            for k in 0..months {
                assert!(f.in_bloom(mid(((first as i32 - 1 + k) % 12 + 1) as u8)), "{f:?} in month {k} of its season");
            }
            assert!(!f.in_bloom(mid(((first as i32 + 10) % 12 + 1) as u8)), "{f:?} the month before");
            assert!(!f.in_bloom(mid((last % 12) + 1)), "{f:?} the month after");
            // The bloom is there for a part of the year only (heather keeps its spent
            // flowers), is out for longer than it is in bud, and fits in the year.
            let b = f.bloom();
            assert!(b.length > 0.1 && b.length > b.bud + 0.05, "{f:?}: {b:?}");
            assert!(b.bud + b.length + b.seed <= 1.0 + 1e-6, "{f:?}: {b:?}");
            // Its leaves are up before it buds and stay until it is over.
            let l = f.leaves();
            if l.length < 1.0 {
                let before = (b.start - b.bud - l.start).rem_euclid(1.0);
                assert!(before < 0.2 && before + b.bud + b.length <= l.length + 1e-6, "{f:?}: {l:?} against {b:?}");
            }
        }
        // True to the plants: the early ones early, the late ones late.
        let month = |f: Flower| f.months();
        assert!(month(Flower::Primrose).0 <= 2, "primroses at winter's end");
        assert_eq!(month(Flower::Bluebell), (4, 5), "bluebells before the canopy closes");
        assert!(month(Flower::Poppy).0 >= 6 && month(Flower::Foxglove).0 >= 6 && month(Flower::Buttercup).1 >= 7, "summer flowers");
        assert_eq!(month(Flower::Heather), (8, 9), "heather in late summer");
        assert!(month(Flower::Gentian).0 >= 6 && month(Flower::Gentian).1 <= 8, "the short alpine summer");
        assert!(month(Flower::FlyAgaric).0 >= 9, "toadstools in autumn");
        // The seasons differ: no one window is shared by more than a few kinds.
        let windows: std::collections::HashSet<(u8, u8)> = Flower::ALL.iter().map(|f| f.months()).collect();
        assert!(windows.len() >= 14, "only {} different seasons", windows.len());
    }

    #[test]
    fn each_plant_comes_out_on_its_own_day_within_the_season() {
        for &f in Flower::ALL {
            if f.all_year() {
                continue;
            }
            let (start, _) = f.season();
            let out = |date: f32| (0..400).filter(|&k| f.blooming_at(date, k * 7, 1000 - k * 3)).count();
            // None the day before the season, some a few days in, all once the stagger is past.
            assert_eq!(out((start - 0.004).rem_euclid(1.0)), 0, "{f:?} out before its season");
            let early = out((start + STAGGER * 0.4).rem_euclid(1.0));
            assert!(early > 80 && early < 320, "{f:?}: {early} of 400 out early in the season");
            assert_eq!(out((start + STAGGER + 0.01).rem_euclid(1.0)), 400, "{f:?} not all out");
            // Never out outside its season, and never gone in the middle of it.
            for k in 0..200 {
                let date = k as f32 / 200.0;
                for c in 0..20 {
                    let phase = f.phase_at(date, c * 13, c * 5);
                    assert!(f.in_bloom(date) || phase != Phase::Bloom, "{f:?} out of season at {date}");
                }
            }
        }
        // The same plant, asked twice, gives the same answer, and neighbours differ.
        let f = Flower::Bluebell;
        let date = f.season().0 + STAGGER * 0.5;
        let differ = (0..200).filter(|&x| f.blooming_at(date, x, 9) != f.blooming_at(date, x + 1, 9)).count();
        assert!(differ > 50, "{differ}");
    }

    /// Every kind of flower the drifts of a biome can grow, and what stands among them,
    /// away from water and the shore. Nothing for a biome with no drifts.
    fn flowers_of(biome: Biome) -> Vec<Flower> {
        let Some((surfaces, _)) = super::super::flower_kinds(biome) else { return Vec::new() };
        let mut found = Vec::new();
        for &surface in surfaces {
            for which in [-0.5, -0.17, 0.1, 0.17, 0.5] {
                let f = if surface == mat::HEATHER { Flower::Heather } else { species(biome, surface, which).expect("a drift names a flower") };
                if !found.contains(&f) {
                    found.push(f);
                }
            }
        }
        if matches!(biome, Biome::Deciduous | Biome::Woodland | Biome::BirchGrove | Biome::Moor) {
            found.push(Flower::Foxglove);
        }
        found
    }

    #[test]
    fn something_is_in_flower_in_every_month_from_spring_to_autumn() {
        let mut biomes = 0;
        let mut placed = std::collections::HashSet::new();
        for biome in Biome::ALL {
            let flowers = flowers_of(biome);
            if flowers.is_empty() {
                continue;
            }
            biomes += 1;
            placed.extend(flowers.iter().copied());
            // What the drifts are said to grow, they do grow.
            let col = column(Cover::Land, biome);
            for &f in &flowers {
                let surfaces = [mat::FLOWER_RED, mat::FLOWER_YELLOW, mat::FLOWER_WHITE, mat::FLOWER_PURPLE, mat::FLOWER_BLUE, mat::FLOWER_PINK, mat::HEATHER, mat::ground_cover(biome)];
                let grows = surfaces.iter().any(|&s| (0..90_000).any(|k| flower_at(&col, s, k % 300 * 7, k / 300 * 7, 5) == Some(f)));
                assert!(grows, "{f:?} never grows in {biome:?}");
            }
            // From the first of spring to the autumn equinox there is always something
            // out (in the far north and on the heights, through their short summer).
            let months = if matches!(biome, Biome::Tundra | Biome::AlpineMeadow) { 4..=9 } else { 3..=9 };
            for m in months {
                assert!(flowers.iter().any(|f| f.in_bloom(mid(m))), "nothing in flower in {biome:?} in {}: it grows {flowers:?}", MONTHS[m as usize - 1]);
            }
            // And never one season for all of them, but where there is no winter.
            let kinds: std::collections::HashSet<(u8, u8)> = flowers.iter().map(|f| f.months()).collect();
            assert!(kinds.len() >= 2 || matches!(biome, Biome::Desert | Biome::RedRock | Biome::Rainforest), "{biome:?} has one season: {flowers:?}");
        }
        assert!(biomes >= 13, "only {biomes} biomes grow flowers");
        // Every kind is to be found somewhere: in the drifts, or in its own place.
        let elsewhere = [Flower::WaterLily, Flower::FlyAgaric, Flower::PennyBun, Flower::Thrift, Flower::Edelweiss];
        for &f in Flower::ALL {
            assert!(placed.contains(&f) || elsewhere.contains(&f), "{f:?} grows nowhere");
        }
        // Over the whole world there is something to find in every month but the dead of
        // winter: primroses from February, toadstools into November.
        for m in 2..=11 {
            assert!(Flower::ALL.iter().any(|f| !f.all_year() && f.in_bloom(mid(m))), "nothing in {}", MONTHS[m as usize - 1]);
        }
    }

    #[test]
    fn a_flower_marks_its_blooms_and_leaves_with_their_seasons() {
        let rows = seasons();
        assert_eq!(rows[0], [0.0; 4]);
        assert!(Flower::ALL.len() * 2 < SEASON_SLOTS);
        let mats = mat::build();
        for &kind in Flower::ALL {
            assert_eq!(rows[kind.bloom_code() as usize], kind.bloom().row());
            assert_eq!(rows[kind.leaf_code() as usize], kind.leaves().row());
            if kind == Flower::WaterLily {
                continue;
            }
            for salt in 0..200 {
                let boxes = flower_model(kind, salt);
                for b in &boxes {
                    let plant = mats.get(b.voxel).plant;
                    if is_bloom(b.voxel) {
                        assert_eq!(b.season, kind.bloom_code(), "{kind:?}: {b:?}");
                    } else if plant == voxel::plant::FLOWER {
                        // No part of a flower is left to the common year of flowers.
                        assert_eq!(b.season, kind.leaf_code(), "{kind:?}: {b:?}");
                    } else {
                        // Its grass and its evergreen parts keep to the year of their kind.
                        assert!(b.season == 0 || b.season == kind.leaf_code(), "{kind:?}: {b:?}");
                    }
                }
                assert!(kind == Flower::Heather || boxes.iter().any(|b| b.season == kind.bloom_code()), "{kind:?} has no bloom");
            }
        }
        // Grass and the other small plants have no season of their own.
        assert!(grown(&column(Cover::Land, Biome::Grassland), mat::GRASS_MEADOW).iter().all(|b| b.season == 0));
        assert!(grown(&column(Cover::Land, Biome::Heath), mat::ground_cover(Biome::Heath)).iter().all(|b| b.season == 0));
        // A water lily's flower keeps its season, and its pad none.
        let mut lake = column(Cover::LakeBed, Biome::Deciduous);
        lake.water = Some(40 + 8 * BLOCK);
        let lilies = grown(&lake, mat::MUD);
        assert!(lilies.iter().any(|b| b.voxel == mat::BLOOM_PINK && b.season == Flower::WaterLily.bloom_code()));
        assert!(lilies.iter().all(|b| b.voxel != mat::LILY || b.season == 0));
    }

    #[test]
    fn flowers_have_names_keys_and_colours_and_die_back_in_winter() {
        let mats = mat::build();
        let mut ids = std::collections::HashSet::new();
        for &f in Flower::ALL {
            assert!(!f.name().is_empty());
            assert!(f.id().chars().all(|c| c.is_ascii_lowercase() || c == '_'), "{}", f.id());
            assert!(ids.insert(f.id()), "{} twice", f.id());
            assert_eq!(f.color(), mats.get(f.petals()).color, "{f:?}");
            assert_eq!(mats.get(f.petals()).plant, voxel::plant::FLOWER, "{f:?} must fade in winter");
        }
        for v in [mat::STEM, mat::LEAF_GLOSS, mat::LEAF_GREY, mat::BLOOM_EYE, mat::BLOOM_ORCHID_LIP] {
            assert_eq!(mats.get(v).plant, voxel::plant::FLOWER);
        }
    }
}
