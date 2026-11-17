//! The animals people keep and their wild kin (#206): what kinds there are, how each
//! lives, which a settlement keeps, and where the wild herds are.
//!
//! Herds are not part of the voxel world. Nothing here is read by chunk generation, so
//! it leaves every world's ground (and the bench checksum) as it was. A wild herd's
//! place, kind and size come from the seed and a hash of the square of land it belongs
//! to, so the same place always has the same herd, for every player, and it is there
//! again when the player comes back. The game (`game::animals`) stands them up, moves
//! them about and draws them.
//!
//! Wild animals are shy and never hostile, the boar and the wild cattle included: they
//! keep their distance and back away from the player (this settles #47 for them).

use glam::{IVec3, Vec2};

use crate::{
    biome::Biome,
    continent::Continent,
    macro_layout::{MACRO_SIZE, cell_of},
    noise::{hash2, unit},
    scale::m,
    settlements::{Kind as Place, Region, Settlement, Water},
    surface::{Column, Cover, Sampler},
    world::GameWorld,
};

/// Every kind of animal: those people keep, then their wild kin in the same order.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Kind {
    Chicken,
    Sheep,
    Goat,
    Cattle,
    /// Cattle kept to draw a plough or a cart: big, horned, always grown.
    Ox,
    Pig,
    Duck,
    Goose,
    Pony,
    Donkey,
    Reindeer,
    Junglefowl,
    WildSheep,
    WildGoat,
    WildCattle,
    Boar,
    WildDuck,
    WildGoose,
    WildPony,
    WildAss,
    WildReindeer,
}

impl Kind {
    pub const KEPT: [Kind; 11] = [
        Kind::Chicken,
        Kind::Sheep,
        Kind::Goat,
        Kind::Cattle,
        Kind::Ox,
        Kind::Pig,
        Kind::Duck,
        Kind::Goose,
        Kind::Pony,
        Kind::Donkey,
        Kind::Reindeer,
    ];
    pub const WILD: [Kind; 10] = [
        Kind::Junglefowl,
        Kind::WildSheep,
        Kind::WildGoat,
        Kind::WildCattle,
        Kind::Boar,
        Kind::WildDuck,
        Kind::WildGoose,
        Kind::WildPony,
        Kind::WildAss,
        Kind::WildReindeer,
    ];
    pub const ALL: [Kind; 21] = [
        Kind::Chicken,
        Kind::Sheep,
        Kind::Goat,
        Kind::Cattle,
        Kind::Ox,
        Kind::Pig,
        Kind::Duck,
        Kind::Goose,
        Kind::Pony,
        Kind::Donkey,
        Kind::Reindeer,
        Kind::Junglefowl,
        Kind::WildSheep,
        Kind::WildGoat,
        Kind::WildCattle,
        Kind::Boar,
        Kind::WildDuck,
        Kind::WildGoose,
        Kind::WildPony,
        Kind::WildAss,
        Kind::WildReindeer,
    ];

    pub fn wild(self) -> bool {
        Kind::WILD.contains(&self)
    }

    /// The wild kin of a kept kind, or the kept kind a wild one was tamed into.
    pub fn kin(self) -> Kind {
        use Kind::*;
        match self {
            Chicken => Junglefowl,
            Sheep => WildSheep,
            Goat => WildGoat,
            Cattle | Ox => WildCattle,
            Pig => Boar,
            Duck => WildDuck,
            Goose => WildGoose,
            Pony => WildPony,
            Donkey => WildAss,
            Reindeer => WildReindeer,
            Junglefowl => Chicken,
            WildSheep => Sheep,
            WildGoat => Goat,
            WildCattle => Cattle,
            Boar => Pig,
            WildDuck => Duck,
            WildGoose => Goose,
            WildPony => Pony,
            WildAss => Donkey,
            WildReindeer => Reindeer,
        }
    }

    /// The kind as said of a group of them: "chickens", "wild boar".
    pub fn word(self) -> &'static str {
        use Kind::*;
        match self {
            Chicken => "chickens",
            Sheep => "sheep",
            Goat => "goats",
            Cattle => "cattle",
            Ox => "oxen",
            Pig => "pigs",
            Duck => "ducks",
            Goose => "geese",
            Pony => "ponies",
            Donkey => "donkeys",
            Reindeer => "reindeer",
            Junglefowl => "junglefowl",
            WildSheep => "wild sheep",
            WildGoat => "wild goats",
            WildCattle => "wild cattle",
            Boar => "wild boar",
            WildDuck => "wild ducks",
            WildGoose => "wild geese",
            WildPony => "wild ponies",
            WildAss => "wild asses",
            WildReindeer => "wild reindeer",
        }
    }

    /// One word for the kind, for flags and the console: `wild-boar`, `oxen`.
    pub fn key(self) -> String {
        self.word().replace(' ', "-")
    }

    /// The kind a word names: its key or its word, one of them or many, with a space, a
    /// dash or nothing between (`chicken`, `hens`, `wildboar`, `wild-goats`).
    pub fn named(word: &str) -> Option<Kind> {
        let w: String = word.to_ascii_lowercase().chars().filter(|c| c.is_ascii_alphabetic()).collect();
        let alias = match w.as_str() {
            "hen" | "hens" | "chicken" | "fowl" => Some(Kind::Chicken),
            "cow" | "cows" | "bull" | "bulls" => Some(Kind::Cattle),
            "ox" => Some(Kind::Ox),
            "pig" | "swine" => Some(Kind::Pig),
            "boar" => Some(Kind::Boar),
            "aurochs" => Some(Kind::WildCattle),
            "ibex" => Some(Kind::WildGoat),
            "mouflon" => Some(Kind::WildSheep),
            "mallard" | "mallards" => Some(Kind::WildDuck),
            "greylag" | "greylags" => Some(Kind::WildGoose),
            "goose" => Some(Kind::Goose),
            "duck" => Some(Kind::Duck),
            "goat" => Some(Kind::Goat),
            "pony" => Some(Kind::Pony),
            "donkey" | "ass" | "asses" => Some(Kind::Donkey),
            "wildass" => Some(Kind::WildAss),
            "deer" => Some(Kind::Reindeer),
            _ => None,
        };
        alias.or_else(|| {
            Kind::ALL.into_iter().find(|k| {
                let key: String = k.word().chars().filter(|c| c.is_ascii_alphabetic()).collect();
                key == w || key.trim_end_matches('s') == w.trim_end_matches('s')
            })
        })
    }

    /// How it lives.
    pub fn habits(self) -> Habits {
        use Kind::*;
        let h = |flight, spread, herd, climb, wade, height, walk, run| Habits { flight, spread, herd, climb, wade, swims: false, height, walk, run };
        let swimmer = |flight, spread, herd, walk, run| Habits { flight, spread, herd, climb: 1, wade: 0, swims: true, height: 2, walk, run };
        match self {
            Chicken => h(2.5, 3.5, (5, 12), 1, 0, 2, 0.45, 2.5),
            Junglefowl => h(12.0, 5.0, (3, 7), 1, 0, 2, 0.5, 3.5),
            Sheep => h(4.0, 6.0, (6, 14), 1, 1, 3, 0.65, 3.5),
            WildSheep => h(30.0, 8.0, (4, 9), 2, 1, 3, 0.75, 5.0),
            Goat => h(2.5, 6.0, (4, 10), 2, 1, 3, 0.65, 3.5),
            WildGoat => h(35.0, 9.0, (3, 8), 3, 1, 3, 0.75, 5.0),
            Cattle => h(3.0, 8.0, (3, 8), 1, 2, 4, 0.8, 3.0),
            Ox => h(2.0, 6.0, (2, 4), 1, 2, 5, 0.8, 2.5),
            WildCattle => h(40.0, 12.0, (4, 10), 1, 2, 5, 0.9, 6.0),
            Pig => h(2.5, 5.0, (3, 7), 1, 1, 3, 0.6, 3.0),
            Boar => h(28.0, 7.0, (3, 8), 1, 1, 3, 0.7, 6.0),
            Duck => swimmer(3.0, 3.5, (5, 12), 0.4, 2.0),
            WildDuck => swimmer(18.0, 5.0, (4, 10), 0.4, 2.5),
            Goose => swimmer(3.0, 4.5, (4, 10), 0.5, 2.5),
            WildGoose => swimmer(25.0, 6.0, (4, 9), 0.5, 3.0),
            Pony => h(2.0, 8.0, (3, 8), 1, 2, 4, 0.9, 5.0),
            WildPony => h(25.0, 12.0, (4, 9), 1, 2, 4, 1.0, 7.0),
            Donkey => h(1.5, 6.0, (2, 5), 1, 1, 4, 0.8, 3.5),
            WildAss => h(35.0, 12.0, (3, 7), 1, 1, 4, 1.0, 7.0),
            Reindeer => h(4.0, 10.0, (6, 14), 1, 2, 4, 0.9, 5.0),
            WildReindeer => h(45.0, 14.0, (6, 14), 1, 2, 4, 1.0, 8.0),
        }
    }
}

/// How a kind lives, as far as the game needs to know to move it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Habits {
    /// How near the player may come before it backs away, in metres. Kept animals are
    /// used to people, and only step aside from someone walking into them.
    pub flight: f32,
    /// How far from the middle of its herd it grazes, in metres.
    pub spread: f32,
    /// How many make a wild herd, or a pen's worth of the kept: fewest and most.
    pub herd: (u8, u8),
    /// The highest step up or down it takes from one voxel column to the next, in
    /// voxels: goats and wild sheep go up crags that others cannot.
    pub climb: i32,
    /// The deepest water it wades into, in voxels.
    pub wade: i32,
    /// Goes out on water of any depth, swimming.
    pub swims: bool,
    /// The headroom it needs, in voxels.
    pub height: i32,
    /// How fast it walks and runs, metres a second.
    pub walk: f32,
    pub run: f32,
}

/// What a wild kind's country must have beyond the right biome.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Needs {
    Nothing,
    /// Tors, crags or scree near: rocky upland.
    Rocks,
    /// Cliffs about it: crags.
    Crags,
    /// Wood of its own kind all round, not just the edge of it.
    DeepWood,
    /// Still water near, a lake or a pool in the wet ground, to swim on.
    StillWater,
}

/// One wild kind, and the country it lives in.
#[derive(Clone, Copy, Debug)]
pub struct Wild {
    pub kind: Kind,
    pub biomes: &'static [Biome],
    pub needs: Needs,
    /// How often it is the one met, against the others whose country this also is.
    pub weight: f32,
    /// The country, in words, for the log.
    pub country: &'static str,
}

/// Where water birds live: the wet ground anywhere, and the lakes of any country that is
/// not too dry, too cold or too high for them.
const WATERSIDE: &[Biome] = &[
    Biome::Marsh,
    Biome::Bog,
    Biome::Mangrove,
    Biome::Grassland,
    Biome::Deciduous,
    Biome::OldGrowth,
    Biome::BirchGrove,
    Biome::Taiga,
    Biome::Tundra,
    Biome::Heath,
    Biome::Moor,
    Biome::Steppe,
    Biome::Woodland,
    Biome::Savanna,
    Biome::Rainforest,
];

/// Who lives where in the wild (#206).
pub const WILD: [Wild; 10] = [
    Wild { kind: Kind::Junglefowl, biomes: &[Biome::Rainforest, Biome::Woodland, Biome::Mangrove], needs: Needs::Nothing, weight: 1.0, country: "rainforest and warm wood edges" },
    Wild { kind: Kind::WildSheep, biomes: &[Biome::Moor, Biome::Heath, Biome::AlpineMeadow, Biome::Tundra], needs: Needs::Rocks, weight: 1.0, country: "rocky upland, around tors" },
    Wild { kind: Kind::WildGoat, biomes: &[Biome::Alpine, Biome::AlpineMeadow, Biome::RedRock, Biome::SnowyPine], needs: Needs::Crags, weight: 1.4, country: "crags and alpine cliffs" },
    Wild { kind: Kind::WildCattle, biomes: &[Biome::Grassland, Biome::Steppe, Biome::Savanna], needs: Needs::Nothing, weight: 1.0, country: "open grassland and steppe" },
    Wild { kind: Kind::Boar, biomes: &[Biome::Deciduous, Biome::OldGrowth], needs: Needs::DeepWood, weight: 1.0, country: "deep broadleaf and old forest" },
    Wild { kind: Kind::WildDuck, biomes: WATERSIDE, needs: Needs::StillWater, weight: 1.6, country: "lakes and wetlands" },
    Wild { kind: Kind::WildGoose, biomes: WATERSIDE, needs: Needs::StillWater, weight: 1.0, country: "lakes and wetlands" },
    Wild { kind: Kind::WildPony, biomes: &[Biome::Moor, Biome::Heath], needs: Needs::Nothing, weight: 1.2, country: "moor" },
    Wild { kind: Kind::WildAss, biomes: &[Biome::Steppe, Biome::Desert, Biome::Savanna], needs: Needs::Nothing, weight: 0.8, country: "dry steppe" },
    Wild { kind: Kind::WildReindeer, biomes: &[Biome::Tundra], needs: Needs::Nothing, weight: 1.0, country: "tundra" },
];

/// The wild entry for a kind.
pub fn wild(kind: Kind) -> Option<&'static Wild> {
    WILD.iter().find(|w| w.kind == kind)
}

/// The animals a settlement keeps: chickens everywhere, and the rest by its country, its
/// water and how big it is (#206). Oxen only where there is a plough or a cart to draw.
pub fn kept_by(s: &Settlement) -> Vec<Kind> {
    use Kind::*;
    let mut out = vec![Chicken];
    out.extend(match s.region {
        Region::Temperate => &[Sheep, Cattle, Pig][..],
        Region::Upland => &[Sheep, Pony, Cattle][..],
        Region::Northern => &[Reindeer, Sheep][..],
        Region::Alpine => &[Goat, Sheep][..],
        Region::Desert => &[Goat, Donkey][..],
        Region::Wetland => &[Duck, Goose, Cattle][..],
        Region::Rainforest => &[Pig, Duck][..],
        Region::OldGrowth => &[Pig, Goat][..],
    });
    if matches!(s.water, Water::River | Water::Lake) {
        out.extend([Duck, Goose]);
    }
    if !matches!(s.kind, Place::Hamlet) && matches!(s.region, Region::Temperate | Region::Upland | Region::Wetland) {
        out.push(Ox);
    }
    if !matches!(s.kind, Place::Hamlet) && matches!(s.region, Region::Desert | Region::Alpine) {
        out.push(Donkey);
    }
    let mut seen = Vec::new();
    out.retain(|k| {
        let first = !seen.contains(k);
        seen.push(*k);
        first
    });
    out
}

/// The side of the square of land each wild herd belongs to, in voxels: one herd at most
/// to a square.
pub const SQUARE: i32 = m(256.0) as i32;
/// The share of squares that have a herd, where the country suits one. With the
/// player seeing a couple of hundred metres either way, a walk through such country
/// meets a herd every kilometre or so: now and then, not all the time.
pub const CHANCE: f32 = 0.3;
/// How far a wild herd wanders from its home, in metres.
pub const RANGE: f32 = 45.0;
/// How far beyond a settlement's edge wild herds keep, in metres, and beyond the edge
/// of their own range.
pub const SHY: f32 = 80.0;
/// How far round its home a herd's country is looked over, in metres.
const LOOK: f32 = 70.0;
/// How far apart the points it is looked over at are, in voxels.
const STEP: i32 = 6;

/// One wild herd: what it is, where it lives, how many and why there.
#[derive(Clone, Debug, PartialEq)]
pub struct Site {
    pub kind: Kind,
    /// Where it lives, a world voxel: the one its feet are in, above the ground.
    pub home: IVec3,
    pub count: u8,
    /// Who each of them is comes from this.
    pub seed: u32,
    /// The square it belongs to (see [`SQUARE`]).
    pub square: (i32, i32),
    /// Why it lives here, in words, for the log.
    pub reason: String,
}

/// A continent not simulated yet, so a herd's place cannot be decided.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NotReady;

/// The square of land a world voxel column is in.
pub fn square_of(x: i32, z: i32) -> (i32, i32) {
    (x.div_euclid(SQUARE), z.div_euclid(SQUARE))
}

/// Where the herd of a square would live, if it has one at all: the dice alone, before
/// the country is looked at. Far enough in from the square's edges that no two herds'
/// ranges meet.
pub fn home_of(seed: u32, sq: (i32, i32)) -> Option<(i32, i32)> {
    let h = hash2(seed ^ 0xfa0a, sq.0, sq.1);
    if unit(h) >= CHANCE {
        return None;
    }
    let pad = m(RANGE + 12.0) as i32;
    let span = (SQUARE - 2 * pad) as f32;
    Some((sq.0 * SQUARE + pad + (unit(hash2(h, 1, 0)) * span) as i32, sq.1 * SQUARE + pad + (unit(hash2(h, 2, 0)) * span) as i32))
}

/// Whether a column is ground an animal can stand on: dry, and neither cliff nor rock.
fn standable(c: &Column) -> bool {
    c.water_surface().is_none() && matches!(c.cover, Cover::Land | Cover::Bank | Cover::Trail | Cover::Beach | Cover::Dune | Cover::Shingle)
}

/// Whether a column holds still water a bird can swim on: a lake, or a pool in wet
/// ground; not a river, which carries them off, nor the sea.
fn still_water(c: &Column) -> bool {
    c.water_surface().is_some_and(|w| w - 1 - c.top() >= 1) && match c.cover {
        Cover::LakeBed => true,
        Cover::Land | Cover::Bank => matches!(c.biome, Biome::Marsh | Biome::Bog | Biome::Mangrove),
        _ => false,
    }
}

/// The wild herd of square `sq`, if it has one, as the world has it: its kind from the
/// country where it lives, its home moved onto ground it can stand on (beside the water,
/// for water birds), and none at all within reach of a settlement.
pub fn site(world: &GameWorld, sq: (i32, i32)) -> Result<Option<Site>, NotReady> {
    let seed = world.seed();
    let Some(first) = home_of(seed, sq) else { return Ok(None) };
    let (mx, mz) = cell_of(first.0, first.1);
    let c = world.continent_if_ready(mx, mz).ok_or(NotReady)?;
    Ok(site_in(world, &c, sq, first))
}

fn site_in(world: &GameWorld, c: &Continent, sq: (i32, i32), first: (i32, i32)) -> Option<Site> {
    let seed = world.seed();
    let h = hash2(seed ^ 0xfa0b, sq.0, sq.1);
    let origin = (c.desc.mx * MACRO_SIZE, c.desc.mz * MACRO_SIZE);
    let (lx, lz) = (first.0 - origin.0, first.1 - origin.1);
    let look = m(LOOK) as i32;
    // Kept clear of the continent's edges, where the land is the sea's.
    if lx - look < 0 || lz - look < 0 || lx + look >= MACRO_SIZE || lz + look >= MACRO_SIZE {
        return None;
    }
    // A quick look at the continent's own map first: most squares are sea, or country
    // that no wild kin lives in, and need no closer look.
    let cs = c.cell_size();
    let grid = |x: i32, z: i32| {
        let (gx, gz) = (((x as f32) / cs) as usize, ((z as f32) / cs) as usize);
        let i = gz.min(c.size - 1) * c.size + gx.min(c.size - 1);
        (Biome::from_u8(c.biome[i]), c.ocean[i])
    };
    let lives = |b: Biome| WILD.iter().any(|w| w.biomes.contains(&b));
    let corners = [(0, 0), (-look, -look), (look, -look), (-look, look), (look, look)];
    if !corners.iter().any(|&(dx, dz)| {
        let (b, sea) = grid(lx + dx, lz + dz);
        !sea && lives(b)
    }) {
        return None;
    }

    let sampler = Sampler::new(c, seed, lx - look, lz - look, 2 * look + 1);
    let n = 2 * look / STEP + 1;
    let at = |i: i32, j: i32| (-look + i * STEP, -look + j * STEP);
    let mut cols = Vec::with_capacity((n * n) as usize);
    for j in 0..n {
        for i in 0..n {
            let (dx, dz) = at(i, j);
            cols.push(sampler.column(lx + dx, lz + dz));
        }
    }
    let col = |i: i32, j: i32| &cols[(j * n + i) as usize];
    let middle = n / 2;
    let biome = col(middle, middle).biome;
    // What the country round about has.
    let (mut crags, mut wood, mut water) = (0, 0, None::<(i32, i32, i32)>);
    let same_wood = matches!(biome, Biome::Deciduous | Biome::OldGrowth);
    for j in 0..n {
        for i in 0..n {
            let c = col(i, j);
            if matches!(c.cover, Cover::Cliff | Cover::Scree) {
                crags += 1;
            }
            if same_wood && c.biome == biome {
                wood += 1;
            }
            let d = (i - middle).pow(2) + (j - middle).pow(2);
            if still_water(c) && water.is_none_or(|w| d < w.2) {
                water = Some((i, j, d));
            }
        }
    }
    // Laying out the tors is work, so only where a kind that looks for them could live.
    let tors = if WILD.iter().any(|w| w.needs == Needs::Rocks && w.biomes.contains(&biome)) {
        world.tors_near(c, first.0 - look, first.1 - look, 2 * look + 1).len()
    } else {
        0
    };
    let met = |needs: Needs| match needs {
        Needs::Nothing => true,
        // Tors, or a good few crags, about the place.
        Needs::Rocks => tors > 0 || crags >= 6,
        Needs::Crags => crags >= 12,
        // Nearly all the country round is the same wood.
        Needs::DeepWood => wood * 10 >= n * n * 9,
        Needs::StillWater => water.is_some(),
    };
    let choices: Vec<&Wild> = WILD.iter().filter(|w| w.biomes.contains(&biome) && met(w.needs)).collect();
    if choices.is_empty() {
        return None;
    }
    let total: f32 = choices.iter().map(|w| w.weight).sum();
    let mut u = unit(hash2(h, 3, 0)) * total;
    let mut wild = choices[choices.len() - 1];
    for w in &choices {
        if u < w.weight {
            wild = w;
            break;
        }
        u -= w.weight;
    }

    // Home: beside the water for the water birds, and ground to stand on for everyone,
    // the nearest to where the dice put it.
    let towards = match (wild.needs, water) {
        (Needs::StillWater, Some((i, j, _))) => (i, j),
        _ => (middle, middle),
    };
    let flat = |i: i32, j: i32| {
        let g = col(i, j).ground;
        [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().all(|&(di, dj)| {
            let (a, b) = (i + di, j + dj);
            (0..n).contains(&a) && (0..n).contains(&b) && (col(a, b).ground - g).abs() <= STEP as f32 * 0.8
        })
    };
    let mut best: Option<(i32, i32, i32)> = None;
    for j in 0..n {
        for i in 0..n {
            let d = (i - towards.0).pow(2) + (j - towards.1).pow(2);
            if standable(col(i, j)) && wild.biomes.contains(&col(i, j).biome) && flat(i, j) && best.is_none_or(|b| d < b.2) {
                best = Some((i, j, d));
            }
        }
    }
    let (bi, bj, d) = best?;
    // Water birds live on the shore: no further from the water than a short walk.
    if wild.needs == Needs::StillWater && d > 4 {
        return None;
    }
    let (dx, dz) = at(bi, bj);
    let home = (lx + dx, lz + dz);
    let home_col = col(bi, bj);

    // Shy of people: none near a settlement, nor near enough to wander into one.
    let me = Vec2::new(home.0 as f32, home.1 as f32);
    let mut nearest: Option<(usize, f32)> = None;
    for (k, s) in c.trails.settlements.iter().enumerate() {
        let gap = me.distance(s.at()) - s.radius;
        if gap < m(SHY + RANGE) {
            return None;
        }
        if nearest.is_none_or(|n| gap < n.1) {
            nearest = Some((k, gap));
        }
    }

    let habits = wild.kind.habits();
    let count = habits.herd.0 + (unit(hash2(h, 4, 0)) * (habits.herd.1 - habits.herd.0 + 1) as f32) as u8;
    let count = count.min(habits.herd.1);
    let biome_word = format!("{:?}", home_col.biome).to_lowercase();
    let mut reason = format!("{} of the {} ({biome_word}", wild.kind.word(), wild.country);
    match wild.needs {
        Needs::Rocks if tors > 0 => reason += &format!(", {tors} tor{} near", if tors == 1 { "" } else { "s" }),
        Needs::Rocks | Needs::Crags => reason += &format!(", crags about: {crags} of {} places looked at", n * n),
        Needs::DeepWood => reason += ", the same wood all round",
        Needs::StillWater => reason += ", by still water",
        Needs::Nothing => {}
    }
    reason += ")";
    if let Some((k, gap)) = nearest {
        let name = c.trails.names.settlement(k).map_or("a settlement", |p| p.name.as_str());
        reason += &format!(", {:.0} m beyond the edge of {name}", gap / m(1.0));
    }
    Some(Site {
        kind: wild.kind,
        home: IVec3::new(origin.0 + home.0, home_col.top() + 1, origin.1 + home.1),
        count,
        seed: hash2(h, 5, 0),
        square: sq,
        reason,
    })
}

/// The wild herds whose homes are within `reach` voxels of the column at `(x, z)`,
/// nearest first. Never blocks: none from a continent not simulated yet.
pub fn sites_near(world: &GameWorld, x: i32, z: i32, reach: i32) -> Vec<Site> {
    let (lo, hi) = (square_of(x - reach, z - reach), square_of(x + reach, z + reach));
    let mut out = Vec::new();
    for sz in lo.1..=hi.1 {
        for sx in lo.0..=hi.0 {
            if let Ok(Some(s)) = site(world, (sx, sz)) {
                let (dx, dz) = ((s.home.x - x) as i64, (s.home.z - z) as i64);
                if dx * dx + dz * dz <= reach as i64 * reach as i64 {
                    out.push(s);
                }
            }
        }
    }
    out.sort_by_key(|s| ((s.home.x - x) as i64).pow(2) + ((s.home.z - z) as i64).pow(2));
    out
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use super::*;

    fn world() -> &'static GameWorld {
        static WORLD: OnceLock<GameWorld> = OnceLock::new();
        WORLD.get_or_init(|| {
            let w = GameWorld::new(1);
            w.continent(0, 0);
            w
        })
    }

    /// Every herd on a stretch of continent (0, 0): a square `side` squares across from
    /// square `from`.
    fn herds(from: (i32, i32), side: i32) -> Vec<Site> {
        let mut out = Vec::new();
        for sz in from.1..from.1 + side {
            for sx in from.0..from.0 + side {
                if let Some(s) = site(world(), (sx, sz)).expect("continent 0, 0 is ready") {
                    out.push(s);
                }
            }
        }
        out
    }

    #[test]
    fn every_kind_has_a_kin_a_name_and_habits() {
        for k in Kind::ALL {
            assert_eq!(k.kin().kin(), if k == Kind::Ox { Kind::Cattle } else { k }, "{k:?}");
            assert_ne!(k.wild(), k.kin().wild());
            assert_eq!(Kind::named(&k.key()), Some(k), "{}", k.key());
            assert_eq!(Kind::named(k.word()), Some(k), "{}", k.word());
            let h = k.habits();
            assert!(h.herd.0 >= 2 && h.herd.0 <= h.herd.1 && h.walk < h.run);
            // The wild keep their distance; the kept only step aside.
            assert!(if k.wild() { h.flight >= 10.0 } else { h.flight <= 5.0 }, "{k:?}");
        }
        assert_eq!(Kind::named("hens"), Some(Kind::Chicken));
        assert_eq!(Kind::named("wild boar"), Some(Kind::Boar));
        assert_eq!(Kind::named("wildgoats"), Some(Kind::WildGoat));
        // Every wild kind lives somewhere.
        for k in Kind::WILD {
            assert!(wild(k).is_some_and(|w| !w.biomes.is_empty()), "{k:?}");
        }
    }

    #[test]
    fn herds_are_placed_the_same_every_time() {
        let a = herds((40, 20), 12);
        let again = herds((40, 20), 12);
        assert_eq!(a, again);
        // And by another way of asking.
        let (x, z) = (46 * SQUARE, 26 * SQUARE);
        let near = sites_near(world(), x, z, 3 * SQUARE);
        assert_eq!(near, sites_near(world(), x, z, 3 * SQUARE));
        for s in &near {
            assert!(a.contains(s) || s.square.0 < 40 || s.square.1 < 20 || s.square.0 >= 52 || s.square.1 >= 32);
        }
    }

    #[test]
    fn herds_keep_off_water_and_cliffs_and_away_from_settlements() {
        let c = world().continent(0, 0);
        let all = herds((20, 10), 50);
        println!("{} herds in {} squares", all.len(), 50 * 50);
        assert!(!all.is_empty());
        let mut kinds = std::collections::BTreeMap::new();
        for s in &all {
            *kinds.entry(s.kind.word()).or_insert(0) += 1;
            let col = world().column(s.home.x, s.home.z);
            assert!(standable(&col), "{} at {} {} stands on {:?}", s.kind.word(), s.home.x, s.home.z, col.cover);
            assert_eq!(s.home.y, col.top() + 1);
            let wild = wild(s.kind).unwrap();
            assert!(wild.biomes.contains(&col.biome), "{} in {:?}: {}", s.kind.word(), col.biome, s.reason);
            for t in &c.trails.settlements {
                let d = Vec2::new(s.home.x as f32, s.home.z as f32).distance(t.at()) - t.radius;
                assert!(d >= m(SHY + RANGE), "{} only {:.0} m from a settlement's edge", s.kind.word(), d / m(1.0));
            }
            assert!((wild.kind.habits().herd.0..=wild.kind.habits().herd.1).contains(&s.count));
            assert!(!s.reason.is_empty());
        }
        println!("{kinds:?}");
    }

    #[test]
    fn water_birds_live_beside_still_water() {
        let mut birds = 0;
        for s in herds((20, 10), 50).iter().filter(|s| s.kind.habits().swims) {
            birds += 1;
            let r = m(4.0) as i32;
            let wet = (-r..=r).any(|dz| (-r..=r).any(|dx| still_water(&world().column(s.home.x + dx * 2, s.home.z + dz * 2))));
            assert!(wet, "{} at {} {} with no water by: {}", s.kind.word(), s.home.x, s.home.z, s.reason);
        }
        println!("{birds} herds of water birds");
    }

    #[test]
    fn a_settlement_keeps_chickens_and_the_animals_of_its_country() {
        let c = world().continent(0, 0);
        assert!(!c.trails.settlements.is_empty());
        for s in &c.trails.settlements {
            let kept = kept_by(s);
            assert_eq!(kept[0], Kind::Chicken);
            assert!(kept.iter().all(|k| !k.wild()));
            assert!(kept.len() >= 2);
        }
    }

    /// Prints where the herds are on a continent, to aim screenshots at them: world 1's
    /// continent (0, 0), or FAUNA_SEED's continent FAUNA_CONTINENT=MX,MZ.
    /// `cargo test --release -p worldgen --lib fauna::tests::print -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn print() {
        let seed = std::env::var("FAUNA_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
        let (mx, mz) = std::env::var("FAUNA_CONTINENT")
            .ok()
            .and_then(|v| v.split_once(',').and_then(|(a, b)| Some((a.trim().parse().ok()?, b.trim().parse().ok()?))))
            .unwrap_or((0, 0));
        let w = GameWorld::new(seed);
        w.continent(mx, mz);
        let n = MACRO_SIZE / SQUARE;
        let mut all = 0;
        for sz in mz * n..(mz + 1) * n {
            for sx in mx * n..(mx + 1) * n {
                if let Ok(Some(s)) = site(&w, (sx, sz)) {
                    println!("herd {} x{} at {},{} (y {}): {}", s.kind.key(), s.count, s.home.x, s.home.z, s.home.y, s.reason);
                    all += 1;
                }
            }
        }
        println!("{all} herds on world {seed}'s continent {mx}, {mz}");
    }
}
