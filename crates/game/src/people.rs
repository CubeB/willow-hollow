//! The people of the settlements (#204): who someone is, made whole from a seed.
//!
//! A person comes from a seed, the region they live in, their trade, how well off they
//! are and their age: a build, a skin tone, hair, clothes dyed in the region's colours
//! and the signs of their trade, all drawn on the player's jointed figure (see
//! [`character`]). Nothing is left to chance at run time, so someone is the same every
//! time they are made and for every player. A household shares a family name, and its
//! children take their skin and hair from their parents.

use glam::DVec3;
use worldgen::{
    names::folk::{self, Sex},
    noise::{hash2, unit},
    scale::m64,
    settlements::Region,
};

use crate::{
    character::{self, Beard, Build, Carried, Footwear, Garment, Hair, Headwear, Layer, Look, Pose, Rgb, blend, shade},
    particles::ParticleInstance,
};

/// What someone does for a living, which shows in what they wear and carry.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Trade {
    None,
    Woodcutter,
    Forager,
    Fisher,
    Hunter,
    Smith,
    Weaver,
    Beekeeper,
    Farmer,
    Herder,
    Mason,
    Innkeeper,
    Merchant,
    Traveller,
}

impl Trade {
    pub const ALL: [Trade; 14] = [
        Trade::None,
        Trade::Woodcutter,
        Trade::Forager,
        Trade::Fisher,
        Trade::Hunter,
        Trade::Smith,
        Trade::Weaver,
        Trade::Beekeeper,
        Trade::Farmer,
        Trade::Herder,
        Trade::Mason,
        Trade::Innkeeper,
        Trade::Merchant,
        Trade::Traveller,
    ];

    pub fn word(self) -> &'static str {
        match self {
            Trade::None => "none",
            Trade::Woodcutter => "woodcutter",
            Trade::Forager => "forager",
            Trade::Fisher => "fisher",
            Trade::Hunter => "hunter",
            Trade::Smith => "smith",
            Trade::Weaver => "weaver",
            Trade::Beekeeper => "beekeeper",
            Trade::Farmer => "farmer",
            Trade::Herder => "herder",
            Trade::Mason => "mason",
            Trade::Innkeeper => "innkeeper",
            Trade::Merchant => "merchant",
            Trade::Traveller => "traveller",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Age {
    Child,
    Adult,
    Elder,
}

impl Age {
    pub fn word(self) -> &'static str {
        match self {
            Age::Child => "child",
            Age::Adult => "adult",
            Age::Elder => "elder",
        }
    }
}

/// Skin tones, palest to darkest, taken from everywhere: anyone may be any of them,
/// wherever they live.
pub const SKIN: [Rgb; 12] = [
    [0xf2, 0xd8, 0xc4],
    [0xea, 0xc6, 0xa8],
    [0xe2, 0xb5, 0x92],
    [0xd9, 0xa3, 0x7a],
    [0xca, 0x95, 0x6a],
    [0xba, 0x83, 0x5a],
    [0xa6, 0x6f, 0x48],
    [0x92, 0x5e, 0x3a],
    [0x7c, 0x4c, 0x2e],
    // Not so dark as the darkest of all, which in shade would lose the face in black hair.
    [0x6a, 0x42, 0x29],
    [0x5c, 0x39, 0x23],
    [0x4f, 0x31, 0x1f],
];

/// Hair as it grows, before it greys.
pub const HAIR: [(&str, Rgb); 9] = [
    ("black", [0x1e, 0x19, 0x16]),
    ("dark brown", [0x38, 0x28, 0x1e]),
    ("brown", [0x5a, 0x3c, 0x26]),
    ("light brown", [0x84, 0x60, 0x3e]),
    ("auburn", [0x80, 0x3a, 0x20]),
    ("red", [0xa8, 0x4c, 0x26]),
    ("strawberry blond", [0xc0, 0x8a, 0x58]),
    ("blond", [0xd2, 0xb2, 0x74]),
    ("ash blond", [0xb8, 0xa8, 0x88]),
];
const GREY: Rgb = [0xa6, 0xa2, 0x9c];
const WHITE: Rgb = [0xe8, 0xe5, 0xde];

/// The cloths of the country: undyed linen and wool first, then the dyes.
pub const CLOTH: [(&str, Rgb); 18] = [
    ("linen", [0xe4, 0xda, 0xc2]),
    ("oatmeal wool", [0xc4, 0xb4, 0x96]),
    ("moorit wool", [0x74, 0x5a, 0x44]),
    ("grey wool", [0x8c, 0x88, 0x80]),
    ("black wool", [0x3e, 0x38, 0x34]),
    ("madder red", [0xa8, 0x36, 0x2a]),
    ("madder rose", [0xc4, 0x70, 0x60]),
    ("woad blue", [0x3a, 0x58, 0x88]),
    ("pale woad", [0x84, 0x9c, 0xb8]),
    ("weld yellow", [0xd2, 0xb0, 0x48]),
    ("woad and weld green", [0x5c, 0x76, 0x3c]),
    ("walnut brown", [0x5c, 0x42, 0x2e]),
    ("lichen purple", [0x76, 0x50, 0x6c]),
    ("indigo", [0x28, 0x36, 0x66]),
    ("saffron", [0xcc, 0x86, 0x2c]),
    ("annatto orange", [0xd0, 0x62, 0x28]),
    ("oak gall black", [0x30, 0x2c, 0x2c]),
    ("red earth", [0x96, 0x52, 0x30]),
];

/// The undyed cloths a region's people wear, and the dyes they have, by weight.
fn cloths(region: Region) -> (&'static [(usize, f32)], &'static [(usize, f32)]) {
    match region {
        Region::Temperate => (&[(0, 2.0), (1, 2.0), (2, 1.0), (3, 1.0)], &[(5, 3.0), (6, 1.0), (7, 3.0), (8, 1.0), (9, 2.0), (10, 3.0), (11, 2.0)]),
        Region::Upland => (&[(1, 2.0), (2, 2.0), (3, 3.0), (4, 1.0)], &[(7, 2.0), (12, 3.0), (5, 2.0), (10, 2.0), (11, 2.0), (17, 1.0)]),
        Region::Northern => (&[(1, 2.0), (2, 2.0), (3, 2.0), (4, 1.0)], &[(5, 3.0), (7, 3.0), (9, 1.0), (11, 2.0), (16, 2.0), (17, 1.0)]),
        Region::Alpine => (&[(0, 2.0), (1, 1.0), (4, 2.0)], &[(5, 4.0), (10, 3.0), (16, 2.0), (7, 1.0), (9, 1.0)]),
        Region::Desert => (&[(0, 4.0), (1, 2.0)], &[(13, 4.0), (14, 3.0), (5, 2.0), (17, 2.0), (6, 1.0)]),
        Region::Wetland => (&[(0, 2.0), (1, 2.0), (2, 2.0)], &[(10, 3.0), (11, 2.0), (9, 2.0), (7, 1.0), (17, 1.0)]),
        Region::Rainforest => (&[(0, 4.0), (1, 1.0)], &[(15, 3.0), (13, 2.0), (14, 2.0), (10, 2.0), (5, 2.0)]),
        Region::OldGrowth => (&[(1, 2.0), (2, 3.0), (3, 1.0)], &[(10, 3.0), (11, 3.0), (17, 2.0), (16, 1.0), (12, 1.0)]),
    }
}

/// Leather for boots and belts, and what hats are made of.
const LEATHERS: [Rgb; 4] = [[0x3b, 0x2d, 0x25], [0x5a, 0x40, 0x2c], [0x7a, 0x56, 0x36], [0x8d, 0x5c, 0x37]];
const STRAW: Rgb = [0xd6, 0xbe, 0x78];
const FURS: [Rgb; 3] = [[0x6a, 0x52, 0x3c], [0x9c, 0x8c, 0x78], [0x3a, 0x2e, 0x26]];
/// The smith's leather apron, the mason's dusty one, and oilskins yellow and dark.
const SMITHY: Rgb = [0x4e, 0x36, 0x26];
const DUSTY: Rgb = [0x9a, 0x94, 0x88];
const OILSKINS: [Rgb; 2] = [[0xc0, 0x9a, 0x38], [0x4c, 0x52, 0x34]];

/// One person.
#[derive(Clone, Debug, PartialEq)]
pub struct Person {
    pub seed: u32,
    pub name: String,
    pub sex: Sex,
    pub age: Age,
    pub trade: Trade,
    pub region: Region,
    pub wealth: f32,
    /// The skin tone, in [`SKIN`], and the hair's own colour before it greys, in
    /// [`HAIR`]: what their children take after.
    pub skin: usize,
    pub hair: usize,
    /// How far the hair has gone grey, 0 to 1 (white).
    pub grey: f32,
    /// What it all looks like, for drawing.
    pub look: Look,
}

/// The choices made for one side of a person, each a hash of their seed, so that one
/// side (their hair, say) can change without the rest changing with it.
struct Dice {
    seed: u32,
    n: i32,
}

impl Dice {
    fn new(seed: u32, side: u32) -> Self {
        Dice { seed: hash2(seed, side as i32, 0x0f01_4b5e), n: 0 }
    }

    fn unit(&mut self) -> f32 {
        self.n += 1;
        unit(hash2(self.seed, self.n, 0))
    }

    fn below(&mut self, n: usize) -> usize {
        ((self.unit() * n as f32) as usize).min(n - 1)
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }

    fn chance(&mut self, p: f32) -> bool {
        self.unit() < p
    }

    fn pick<T: Copy>(&mut self, weighted: &[(T, f32)]) -> T {
        let total: f32 = weighted.iter().map(|w| w.1).sum();
        let mut u = self.unit() * total;
        for &(t, w) in weighted {
            if u < w {
                return t;
            }
            u -= w;
        }
        weighted[weighted.len() - 1].0
    }
}

/// The sides of a person, each with its own dice.
const SEX: u32 = 1;
const KIN: u32 = 2;
const BUILD: u32 = 3;
const HAIRDO: u32 = 4;
const CLOTHES: u32 = 5;
const COLOURS: u32 = 6;

/// What a person is made from, beyond their seed.
struct Making<'a> {
    region: Region,
    tongue: &'a str,
    trade: Trade,
    wealth: f32,
    age: Age,
    sex: Option<Sex>,
    /// Whom they take after.
    parents: Option<[&'a Person; 2]>,
    /// Their household's family name, and a given name already taken in it.
    family: Option<&'a str>,
    taken: &'a [String],
}

impl Person {
    /// Someone of `region` making their living by `trade`, `wealth` well off (0 to 1),
    /// named in the tongue most spoken there.
    pub fn new(seed: u32, region: Region, trade: Trade, wealth: f32, age: Age) -> Person {
        Person::in_tongue(seed, region, folk::tongue_of(region), trade, wealth, age)
    }

    /// As [`Person::new`], named in `tongue` (a tongue of `words.txt`, as the
    /// gazetteer gives a settlement's).
    pub fn in_tongue(seed: u32, region: Region, tongue: &str, trade: Trade, wealth: f32, age: Age) -> Person {
        make(seed, &Making { region, tongue, trade, wealth, age, sex: None, parents: None, family: None, taken: &[] })
    }

    /// Their hair as it is now, greyed by the years.
    pub fn hair_now(base: usize, grey: f32) -> Rgb {
        let natural = HAIR[base].1;
        if grey <= 0.7 {
            blend(natural, GREY, grey / 0.7 * 0.85)
        } else {
            blend(blend(natural, GREY, 0.85), WHITE, (grey - 0.7) / 0.3)
        }
    }

    /// A line about them, for logs: name, age, trade and what they wear.
    pub fn describe(&self) -> String {
        let l = &self.look;
        format!(
            "{} ({}, {}, {:.2} m, wealth {:.2}): {} hair{}, {}{}, {}, {}{}",
            self.name,
            match (self.age, self.sex) {
                (Age::Child, Sex::Female) => "girl",
                (Age::Child, Sex::Male) => "boy",
                (Age::Adult, Sex::Female) => "woman",
                (Age::Adult, Sex::Male) => "man",
                (Age::Elder, Sex::Female) => "old woman",
                (Age::Elder, Sex::Male) => "old man",
            },
            self.trade.word(),
            l.build.metres(),
            self.wealth,
            said(l.cut),
            match l.beard {
                Beard::None => String::new(),
                Beard::Moustache => " and a moustache".into(),
                b => format!(" and a {} beard", said(b)),
            },
            said(l.garment),
            if l.layer == Layer::None { String::new() } else { format!(" and {}", said(l.layer)) },
            if l.headwear == Headwear::None { "bareheaded".into() } else { said(l.headwear) },
            if l.footwear == Footwear::Bare { "barefoot".into() } else { said(l.footwear) },
            if l.carried == Carried::None { String::new() } else { format!(", carrying {}", said(l.carried)) },
        )
    }
}

/// A kind of thing as said, not as spelt in code: `StrawHat` is a straw hat.
fn said(what: impl std::fmt::Debug) -> String {
    let mut out = String::new();
    for c in format!("{what:?}").chars() {
        if c.is_uppercase() && !out.is_empty() {
            out.push(' ');
        }
        out.extend(c.to_lowercase());
    }
    out
}

/// Makes one person.
fn make(seed: u32, m: &Making) -> Person {
    let age = m.age;
    let trade = if age == Age::Child { Trade::None } else { m.trade };
    let wealth = m.wealth.clamp(0.0, 1.0);
    let region = m.region;
    let sex = m.sex.unwrap_or_else(|| if Dice::new(seed, SEX).chance(0.5) { Sex::Female } else { Sex::Male });
    let woman = sex == Sex::Female;

    // Skin and hair from the parents, or from anywhere; a child's skin falls between
    // theirs, its hair the colour of one or the other's.
    let mut kin = Dice::new(seed, KIN);
    let (skin, hair) = match m.parents {
        Some([a, b]) => {
            let (lo, hi) = (a.skin.min(b.skin), a.skin.max(b.skin));
            (lo + kin.below(hi - lo + 1), if kin.chance(0.5) { a.hair } else { b.hair })
        }
        None => {
            let skin = kin.below(SKIN.len());
            (skin, kin.pick(hair_by_skin(skin)))
        }
    };

    // The build: children smaller with bigger heads, the old a little shrunk and bent.
    let mut d = Dice::new(seed, BUILD);
    let parents_girth = m.parents.map(|[a, b]| (a.look.build.girth + b.look.build.girth) * 0.5);
    let (metres, girth, head, stoop, grey) = match age {
        Age::Child => {
            let girth = 0.84 + parents_girth.map_or(0.06, |g| (g - 0.86) * 0.35) + d.range(0.0, 0.06);
            (d.range(0.95, 1.4), girth, d.range(1.25, 1.36), 0.0, 0.0)
        }
        Age::Adult | Age::Elder => {
            let tall = if woman { d.range(1.52, 1.74) } else { d.range(1.62, 1.88) };
            let u = d.unit();
            let girth = 0.86 + 0.42 * u * u + if woman { 0.0 } else { 0.04 };
            let girth = parents_girth.map_or(girth, |g| girth * 0.6 + g * 0.4);
            if age == Age::Elder {
                // Many are white; the rest well on the way.
                let grey = if d.chance(0.5) { d.range(0.88, 1.0) } else { d.range(0.5, 0.88) };
                (tall - d.range(0.02, 0.08), girth, d.range(0.98, 1.03), d.range(0.05, 0.2), grey)
            } else {
                let grey = if d.chance(0.2) { d.range(0.05, 0.45) } else { 0.0 };
                (tall, girth, d.range(0.97, 1.03), 0.0, grey)
            }
        }
    };
    let build = Build::standing(metres, girth, head, stoop);

    // How the hair is worn, and the beard.
    let mut h = Dice::new(seed, HAIRDO);
    use Hair::*;
    let cut = h.pick(match (age, woman) {
        (Age::Child, true) => &[(Plaited, 30.0), (Shoulder, 25.0), (Long, 15.0), (Short, 15.0), (Bun, 5.0), (TopKnot, 10.0)],
        (Age::Child, false) => &[(Short, 40.0), (Cropped, 35.0), (Shoulder, 15.0), (TopKnot, 10.0)],
        (Age::Adult, true) => &[(Long, 20.0), (Plaited, 20.0), (Bun, 25.0), (Shoulder, 15.0), (Short, 8.0), (Cropped, 4.0), (TopKnot, 6.0), (Shaved, 1.0)],
        (Age::Adult, false) => &[(Short, 30.0), (Cropped, 22.0), (Shoulder, 10.0), (Long, 6.0), (Plaited, 3.0), (Bun, 2.0), (TopKnot, 7.0), (Shaved, 10.0), (Receding, 10.0)],
        (Age::Elder, true) => &[(Bun, 40.0), (Plaited, 15.0), (Short, 25.0), (Shoulder, 10.0), (Cropped, 10.0)],
        (Age::Elder, false) => &[(Receding, 35.0), (Shaved, 15.0), (Short, 25.0), (Cropped, 20.0), (Long, 5.0)],
    });
    let beard = match (age, woman) {
        (Age::Child, _) | (_, true) => Beard::None,
        (Age::Adult, false) => h.pick(&[(Beard::None, 35.0), (Beard::Moustache, 15.0), (Beard::Short, 30.0), (Beard::Full, 20.0)]),
        (Age::Elder, false) => h.pick(&[(Beard::None, 25.0), (Beard::Moustache, 15.0), (Beard::Short, 25.0), (Beard::Full, 35.0)]),
    };

    // What is worn, by the country and then by the trade.
    let mut c = Dice::new(seed, CLOTHES);
    let mut garment = c.pick(&garments(region, woman, age));
    let mut layer = c.pick(&layers(region, woman, age));
    let mut headwear = c.pick(&hats(region, woman, age));
    let mut footwear = c.pick(&shoes(region, age));
    let mut carried = Carried::None;
    let mut rolled = matches!(region, Region::Desert | Region::Rainforest) && c.chance(0.3);
    let mut pack = false;
    match trade {
        Trade::None if age == Age::Elder && c.chance(0.5) => carried = Carried::Stick,
        Trade::None => {}
        Trade::Woodcutter => {
            carried = Carried::Axe;
            rolled |= c.chance(0.4);
            if c.chance(0.5) {
                headwear = Headwear::Cap;
            }
        }
        Trade::Forager => carried = Carried::Basket,
        Trade::Fisher => {
            // The oilskin coat, to the knee, and its hat.
            garment = Garment::Smock;
            layer = Layer::None;
            if c.chance(0.7) {
                headwear = Headwear::Souwester;
            }
            footwear = Footwear::Boots;
            if c.chance(0.5) {
                carried = Carried::Rod;
            }
        }
        Trade::Hunter => {
            carried = Carried::Bow;
            if !matches!(headwear, Headwear::FurHat | Headwear::Wrap) {
                headwear = if c.chance(0.5) { Headwear::Hood } else { Headwear::Cap };
            }
        }
        Trade::Smith => {
            layer = Layer::Bib;
            rolled = true;
            carried = Carried::Hammer;
            if garment.skirted() {
                garment = Garment::Tunic;
            }
        }
        Trade::Weaver if woman => layer = Layer::Shawl,
        Trade::Weaver => {}
        Trade::Beekeeper => {
            headwear = Headwear::Veil;
            carried = Carried::Smoker;
        }
        Trade::Farmer => {
            carried = Carried::Hoe;
            rolled |= c.chance(0.4);
            if c.chance(0.7) {
                headwear = Headwear::StrawHat;
            }
        }
        Trade::Herder => {
            carried = Carried::Crook;
            if c.chance(0.6) {
                layer = Layer::Cloak;
            }
        }
        Trade::Mason => {
            layer = Layer::Apron;
            rolled = true;
            carried = Carried::Hammer;
        }
        Trade::Innkeeper => {
            layer = Layer::Bib;
            rolled |= c.chance(0.6);
            if c.chance(0.4) {
                carried = Carried::Jug;
            }
        }
        Trade::Merchant => {
            if !matches!(layer, Layer::Cloak | Layer::Waistcoat) {
                layer = if c.chance(0.5) { Layer::Waistcoat } else { Layer::Cloak };
            }
            carried = Carried::Purse;
        }
        Trade::Traveller => {
            pack = true;
            carried = Carried::Staff;
            // A pack and a cloak don't go together.
            if layer == Layer::Cloak {
                layer = Layer::None;
            }
            if footwear == Footwear::Bare {
                footwear = Footwear::Sandals;
            }
        }
    }
    // The cold keeps a scarf on; a shawl or cloak does instead.
    let cold = matches!(region, Region::Northern | Region::Alpine | Region::Upland);
    let scarf = !matches!(layer, Layer::Shawl | Layer::Cloak) && headwear != Headwear::Veil && c.chance(if cold { 0.55 } else { 0.18 });

    // The colours: the region's dyes, brighter where people are better off.
    let mut k = Dice::new(seed, COLOURS);
    let mut top = cloth(&mut k, region, wealth, 0.0);
    let mut bottom = if garment == Garment::Robe && k.chance(0.7) { shade(top, 0.95) } else { cloth(&mut k, region, wealth, -0.15) };
    let boots = LEATHERS[k.below(3)];
    let accent = cloth(&mut k, region, wealth, 0.6);
    let mut over = cloth(&mut k, region, wealth, 0.1);
    let mut hat = match headwear {
        Headwear::StrawHat | Headwear::Veil => shade(STRAW, k.range(0.85, 1.05)),
        Headwear::FurHat => FURS[k.below(FURS.len())],
        Headwear::Hood if layer == Layer::Cloak => over,
        _ => cloth(&mut k, region, wealth, 0.2),
    };
    // A plain smock is unbelted; a dress or robe has a sash.
    let belt = match garment {
        Garment::Smock => shade(top, 0.85),
        Garment::Dress | Garment::Robe if k.chance(0.6) => accent,
        _ => LEATHERS[1 + k.below(3)],
    };
    match trade {
        Trade::Smith => over = SMITHY,
        Trade::Mason => over = DUSTY,
        Trade::Innkeeper => over = shade(CLOTH[0].1, k.range(0.95, 1.02)),
        Trade::Fisher => {
            top = OILSKINS[k.below(2)];
            hat = top;
        }
        Trade::Hunter => {
            // Nothing bright to be seen by.
            top = blend(top, [0x5a, 0x5a, 0x3a], 0.6);
            over = blend(over, [0x4a, 0x48, 0x30], 0.6);
            if headwear == Headwear::Hood {
                hat = over;
            }
        }
        Trade::Merchant => {
            // The best of the dyes.
            top = cloth(&mut k, region, 1.0, 0.4);
            over = cloth(&mut k, region, 1.0, 0.4);
        }
        _ => {}
    }
    if layer == Layer::Waistcoat && over == top {
        bottom = shade(bottom, 0.9);
    }

    // The name: the household's family name, and a given name not already in it.
    let family = m.family.map_or_else(|| folk::family_name(seed, m.tongue), String::from);
    let mut given = folk::given_name(seed, m.tongue, sex);
    for attempt in 1..12 {
        if !m.taken.iter().any(|t| t == given) {
            break;
        }
        given = folk::given_name(hash2(seed, attempt, 0x6e), m.tongue, sex);
    }

    let look = Look {
        skin: SKIN[skin],
        hair: Person::hair_now(hair, grey),
        top,
        bottom,
        boots,
        accent,
        pack,
        build,
        cut,
        beard,
        garment,
        layer,
        headwear,
        footwear,
        carried,
        over,
        hat,
        belt,
        scarf,
        rolled,
    };
    Person { seed, name: folk::joined(given, &family), sex, age, trade, region, wealth, skin, hair, grey, look }
}

/// Hair colours by how dark the skin is: red and fair hair are seldom seen with dark
/// skin, though everything is seen somewhere.
fn hair_by_skin(skin: usize) -> &'static [(usize, f32)] {
    match skin {
        0..=3 => &[(0, 1.0), (1, 3.0), (2, 4.0), (3, 3.0), (4, 2.0), (5, 1.5), (6, 1.0), (7, 2.5), (8, 1.5)],
        4..=7 => &[(0, 3.0), (1, 4.0), (2, 3.0), (3, 1.0), (4, 1.0), (5, 0.3), (7, 0.3)],
        _ => &[(0, 7.0), (1, 3.0), (2, 1.0)],
    }
}

/// A cloth of the region: undyed, or dyed with one of its dyes, the more often and the
/// brighter for being better off (`more` makes dye likelier still).
fn cloth(d: &mut Dice, region: Region, wealth: f32, more: f32) -> Rgb {
    let (undyed, dyes) = cloths(region);
    let c = if d.chance((0.22 + 0.6 * wealth + more).clamp(0.05, 0.95)) { CLOTH[d.pick(dyes)].1 } else { CLOTH[d.pick(undyed)].1 };
    // Fewer dips in the vat, and washed and worn longer: a poor household's colours are
    // dull and gone brownish.
    let y = (c[0] as f32 * 0.3 + c[1] as f32 * 0.59 + c[2] as f32 * 0.11).round() as u8;
    let dull = blend([y; 3], [0x7a, 0x6a, 0x58], 0.35);
    shade(blend(c, dull, (1.0 - wealth) * 0.5), d.range(0.92, 1.06))
}

fn garments(region: Region, woman: bool, age: Age) -> Vec<(Garment, f32)> {
    use Garment::*;
    let (tunic, smock, dress, robe) = match region {
        Region::Temperate => (25.0, 15.0, 60.0, 0.0),
        Region::Upland => (35.0, 15.0, 50.0, 0.0),
        Region::Northern => (55.0, 5.0, 40.0, 0.0),
        Region::Alpine => (25.0, 5.0, 70.0, 0.0),
        Region::Desert => (20.0, 0.0, 20.0, 60.0),
        Region::Wetland => (25.0, 45.0, 30.0, 0.0),
        Region::Rainforest => (45.0, 10.0, 35.0, 10.0),
        Region::OldGrowth => (40.0, 10.0, 30.0, 20.0),
    };
    if woman {
        vec![(Tunic, tunic * 0.4), (Smock, smock), (Dress, dress), (Robe, robe)]
    } else if age == Age::Child {
        vec![(Tunic, tunic + dress * 0.5), (Smock, smock + 10.0), (Robe, robe * 0.5)]
    } else {
        vec![(Tunic, tunic + dress), (Smock, smock + 10.0), (Robe, robe)]
    }
}

fn layers(region: Region, woman: bool, age: Age) -> Vec<(Layer, f32)> {
    use Layer::*;
    let (none, apron, waistcoat, cloak, shawl) = match region {
        Region::Temperate => (40.0, 18.0, 15.0, 12.0, 15.0),
        Region::Upland => (25.0, 10.0, 10.0, 25.0, 30.0),
        Region::Northern => (25.0, 10.0, 10.0, 35.0, 20.0),
        Region::Alpine => (25.0, 25.0, 30.0, 10.0, 10.0),
        Region::Desert => (60.0, 5.0, 10.0, 5.0, 20.0),
        Region::Wetland => (55.0, 20.0, 10.0, 5.0, 10.0),
        Region::Rainforest => (75.0, 5.0, 5.0, 0.0, 15.0),
        Region::OldGrowth => (30.0, 10.0, 10.0, 40.0, 10.0),
    };
    match age {
        Age::Child => vec![(None, 80.0), (Apron, 8.0), (Cloak, cloak * 0.4)],
        _ if woman => vec![(None, none), (Apron, apron * 1.5), (Waistcoat, waistcoat * 0.5), (Cloak, cloak), (Shawl, shawl * 1.6)],
        _ => vec![(None, none), (Apron, apron * 0.5), (Waistcoat, waistcoat * 1.4), (Cloak, cloak), (Shawl, shawl * 0.2)],
    }
}

fn hats(region: Region, woman: bool, age: Age) -> Vec<(Headwear, f32)> {
    use Headwear::*;
    let mut w: Vec<(Headwear, f32)> = match region {
        Region::Temperate => vec![(None, 45.0), (StrawHat, 15.0), (Cap, 15.0), (Hood, 10.0), (Headscarf, 15.0)],
        Region::Upland => vec![(None, 35.0), (Cap, 25.0), (Hood, 25.0), (Headscarf, 15.0)],
        Region::Northern => vec![(None, 25.0), (FurHat, 40.0), (Hood, 25.0), (Cap, 10.0)],
        Region::Alpine => vec![(None, 30.0), (Cap, 30.0), (FurHat, 15.0), (Headscarf, 15.0), (Hood, 10.0)],
        Region::Desert => vec![(None, 20.0), (Wrap, 35.0), (Headscarf, 40.0), (StrawHat, 5.0)],
        Region::Wetland => vec![(None, 45.0), (StrawHat, 35.0), (Cap, 10.0), (Headscarf, 10.0)],
        Region::Rainforest => vec![(None, 60.0), (Headscarf, 15.0), (StrawHat, 15.0), (Wrap, 10.0)],
        Region::OldGrowth => vec![(None, 40.0), (Hood, 35.0), (Cap, 15.0), (Headscarf, 10.0)],
    };
    for (h, weight) in &mut w {
        let h = *h;
        // Headscarves are mostly women's, wraps and caps mostly men's; children go
        // bare-headed more often.
        *weight *= match (h, woman) {
            (Headscarf, false) => 0.15,
            (Wrap | Cap, true) => 0.3,
            _ => 1.0,
        };
        if age == Age::Child && h != None {
            *weight *= 0.4;
        }
    }
    w
}

fn shoes(region: Region, age: Age) -> Vec<(Footwear, f32)> {
    use Footwear::*;
    let (boots, shoes, sandals, bare) = match region {
        Region::Desert => (5.0, 20.0, 50.0, 25.0),
        Region::Rainforest => (0.0, 5.0, 40.0, 55.0),
        Region::Wetland => (35.0, 30.0, 0.0, 35.0),
        Region::Northern | Region::Alpine | Region::Upland => (80.0, 20.0, 0.0, 0.0),
        Region::Temperate | Region::OldGrowth => (50.0, 45.0, 0.0, 5.0),
    };
    let bare = if age == Age::Child { bare * 2.0 + 10.0 } else { bare };
    vec![(Boots, boots), (Shoes, shoes), (Sandals, sandals), (Bare, bare)]
}

/// The shapes a household takes.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Shape {
    LoneElder,
    Single,
    Couple,
    Family,
    /// Parents, children and a grandparent.
    Generations,
}

/// A household: who lives under one roof, sharing a family name, the children taking
/// after their parents. The first adult works at `trade`; the rest as suits it.
pub fn household(seed: u32, region: Region, tongue: &str, wealth: f32, trade: Trade) -> Vec<Person> {
    make_household(seed, region, tongue, wealth, trade, None)
}

/// A household of exactly `size`, as [`household`] makes one: the beds of a home in a
/// settlement are made for so many (#204). One alone is an elder or a single adult, two
/// a couple or a parent and child, more a family, with a grandparent in some.
pub fn household_of(seed: u32, region: Region, tongue: &str, wealth: f32, trade: Trade, size: usize) -> Vec<Person> {
    make_household(seed, region, tongue, wealth, trade, Some(size))
}

fn make_household(seed: u32, region: Region, tongue: &str, wealth: f32, trade: Trade, size: Option<usize>) -> Vec<Person> {
    if size == Some(0) {
        return Vec::new();
    }
    let mut d = Dice::new(seed, 40);
    let family = folk::family_name(seed, tongue);
    let picked = d.pick(&[(Shape::LoneElder, 0.08), (Shape::Single, 0.1), (Shape::Couple, 0.14), (Shape::Family, 0.5), (Shape::Generations, 0.18)]);
    // The shape picked, as near as a household of the size asked for can come to it.
    let shape = match size {
        None => picked,
        Some(1) if picked == Shape::LoneElder => Shape::LoneElder,
        Some(1) => Shape::Single,
        Some(2) if picked == Shape::Single => Shape::Single,
        Some(2) => Shape::Couple,
        Some(n) if n >= 4 && picked == Shape::Generations => Shape::Generations,
        Some(_) => Shape::Family,
    };
    let mut people: Vec<Person> = Vec::new();
    let mut taken: Vec<String> = Vec::new();
    let mut n = 0;
    let mut next = |d: &mut Dice| {
        n += 1;
        (hash2(seed, n, 0x0a5e), (wealth + d.range(-0.05, 0.05)).clamp(0.0, 1.0))
    };
    let add = |people: &mut Vec<Person>, taken: &mut Vec<String>, seed: u32, wealth: f32, age: Age, trade: Trade, sex: Option<Sex>, parents: Option<[&Person; 2]>| {
        let p = make(seed, &Making { region, tongue, trade, wealth, age, sex, parents, family: Some(family.as_str()), taken: &taken[..] });
        taken.push(p.name.split(' ').next().unwrap_or("").to_string());
        people.push(p);
    };

    if shape == Shape::LoneElder {
        let (s, w) = next(&mut d);
        add(&mut people, &mut taken, s, w, Age::Elder, if d.chance(0.15) { trade } else { Trade::None }, None, None);
        return people;
    }
    // Someone outside the household, whom one of its own takes after.
    let stranger = |s: u32, age: Age| make(s, &Making { region, tongue, trade: Trade::None, wealth, age, sex: None, parents: None, family: None, taken: &[] });
    let first_sex = if d.chance(0.5) { Sex::Female } else { Sex::Male };
    let other_sex = if first_sex == Sex::Female { Sex::Male } else { Sex::Female };
    // The first adult, a child of the elder who lives with them, if one does.
    if shape == Shape::Generations {
        let (s, w) = next(&mut d);
        add(&mut people, &mut taken, s, w, Age::Elder, if d.chance(0.15) { trade } else { Trade::None }, None, None);
        let (s, w) = next(&mut d);
        let other = stranger(hash2(s, 1, 0x0b), Age::Elder);
        let elder = people[0].clone();
        add(&mut people, &mut taken, s, w, Age::Adult, trade, Some(first_sex), Some([&elder, &other]));
    } else {
        let (s, w) = next(&mut d);
        add(&mut people, &mut taken, s, w, Age::Adult, trade, Some(first_sex), None);
    }
    let first = people.len() - 1;
    // Their partner, from elsewhere, working at the same or at something to go with it.
    let partner = if shape == Shape::Single {
        None
    } else {
        let (s, w) = next(&mut d);
        let work = d.pick(&[(trade, 3.0), (Trade::None, 3.0), (Trade::Forager, 1.0), (Trade::Weaver, 1.5), (Trade::Farmer, 1.5)]);
        add(&mut people, &mut taken, s, w, Age::Adult, work, Some(other_sex), None);
        Some(people.len() - 1)
    };
    let children = match (size, shape) {
        (Some(n), _) => n.saturating_sub(people.len()),
        (None, Shape::Family | Shape::Generations) => 1 + d.below(4),
        (None, Shape::Single) if d.chance(0.5) => 1 + d.below(2),
        _ => 0,
    };
    if children > 0 {
        let a = people[first].clone();
        // A parent no longer in the household still gave the children their looks.
        let b = match partner {
            Some(k) => people[k].clone(),
            None => stranger(hash2(seed, 99, 0x0c), Age::Adult),
        };
        for _ in 0..children {
            let (s, w) = next(&mut d);
            add(&mut people, &mut taken, s, w, Age::Child, Trade::None, None, Some([&a, &b]));
        }
    }
    people
}

/// People to stand in a gallery: a household side by side first, with children, then
/// every trade at every age (or all of the age `only`), as many as `n` in all; and how
/// many the household is.
pub fn gallery(seed: u32, region: Region, n: usize, only: Option<Age>) -> (Vec<Person>, usize) {
    let tongue = folk::tongue_of(region);
    // Each region's own people, not the same ones dressed otherwise.
    let seed = hash2(seed, region as i32, 0x9a10);
    // The first household with two parents and a child, to see them side by side.
    let mut out = (0..64)
        .map(|k| household(hash2(seed, k, 0x9a11), region, tongue, 0.5, Trade::ALL[1 + k as usize % (Trade::ALL.len() - 1)]))
        .find(|h| h.iter().filter(|p| p.age == Age::Adult).count() >= 2 && h.iter().any(|p| p.age == Age::Child))
        .unwrap_or_default();
    out.truncate(n);
    let household = out.len();
    let mut k = 0;
    while out.len() < n {
        let age = only.unwrap_or(match k % 7 {
            2 => Age::Child,
            5 => Age::Elder,
            _ => Age::Adult,
        });
        let trade = if age == Age::Adult { Trade::ALL[1 + (k * 5) % (Trade::ALL.len() - 1)] } else { Trade::None };
        let wealth = ((k as f32 * 0.37) % 1.0) * 0.9 + 0.05;
        out.push(Person::new(hash2(seed, k as i32, 0x9a12), region, trade, wealth, age));
        k += 1;
    }
    (out, household)
}

/// Where each of `count` people stand in a gallery, in rows facing the camera: metres
/// across (to the camera's right) and ahead of it. The household has the first row to
/// itself; each row after stands between the gaps of the one before, to be seen.
pub fn gallery_places(count: usize, household: usize, ahead: f64) -> Vec<(f64, f64)> {
    const ACROSS: f64 = 0.9;
    const ROW: usize = 7;
    let mut places = Vec::new();
    let mut row = 0;
    let mut k = 0;
    while k < count {
        let n = if row == 0 && household > 0 { household.min(count) } else { ROW.min(count - k) };
        let stagger = if row % 2 == 1 { 0.5 * ACROSS } else { 0.0 };
        for i in 0..n {
            places.push(((i as f64 - (n as f64 - 1.0) * 0.5) * ACROSS + stagger, ahead + row as f64 * 1.4));
        }
        k += n;
        row += 1;
    }
    places
}

/// A gallery of people on a screenshot (`--people`), to see what the generator makes.
#[derive(Clone, Debug)]
pub struct Gallery {
    pub count: usize,
    /// Whose people they are: the region the camera is in if not given.
    pub region: Option<Region>,
    /// How far ahead of the camera the first row stands, in metres.
    pub ahead: f64,
    /// Caught mid stride at this many metres a second; standing at none.
    pub stride: f32,
    /// Turned this far (radians) from facing the camera, to be seen from the side.
    pub turn: f32,
    /// Drawn as they are from far off, in a few boxes.
    pub far: bool,
    /// All of this age after the household, rather than every age.
    pub only: Option<Age>,
}

impl Default for Gallery {
    fn default() -> Self {
        Gallery { count: 12, region: None, ahead: 3.5, stride: 0.0, turn: 0.0, far: false, only: None }
    }
}

/// Someone stood somewhere: who, where their feet are, which way they face (a camera's
/// yaw), and how they hold themselves.
pub type Standing = (Person, DVec3, f32, Pose);

/// Stands a gallery in rows before a camera at `eye` looking along `yaw` (radians), each
/// on the ground where `ground` says it is at a column, facing the camera. Who each is
/// goes to the log.
pub fn stand_gallery(g: &Gallery, region: Region, seed: u32, eye: DVec3, yaw: f32, ground: &dyn Fn(i32, i32) -> f64) -> Vec<Standing> {
    let (people, household) = gallery(seed, region, g.count, g.only);
    let places = gallery_places(people.len(), household, g.ahead);
    let (ahead, right) = (DVec3::new(yaw.cos() as f64, 0.0, yaw.sin() as f64), DVec3::new(-yaw.sin() as f64, 0.0, yaw.cos() as f64));
    log::info!("{} people of the {} country, the first {household} a household:", people.len(), region.word());
    let mut out = Vec::new();
    for (k, (p, (across, on))) in people.into_iter().zip(places).enumerate() {
        let at = eye + ahead * m64(on) + right * m64(across);
        let feet = DVec3::new(at.x, ground(at.x.floor() as i32, at.z.floor() as i32), at.z);
        let to = eye - feet;
        let facing = to.z.atan2(to.x) as f32 + g.turn;
        let pose = if g.stride > 0.0 {
            character::Gait::striding(facing, g.stride).pose()
        } else {
            // At ease, each looking a little their own way.
            Pose { fore: [-0.1; 2], head: ((k * 7 % 5) as f32 - 2.0) * 0.12, ..Default::default() }
        };
        let mut boxes = Vec::new();
        character::draw(&p.look, feet, facing, &pose, None, eye, &mut boxes);
        let near = boxes.len();
        boxes.clear();
        character::draw_far(&p.look, feet, facing, &pose, eye, &mut boxes);
        log::info!("  {near:>2} boxes near, {} far: {}", boxes.len(), p.describe());
        out.push((p, feet, facing, pose));
    }
    out
}

/// Draws those stood about, as near or far as each is from `cam`; with `far`, all as
/// from far off.
pub fn draw_all(standing: &[Standing], far: bool, cam: DVec3, out: &mut Vec<ParticleInstance>) {
    for (p, feet, facing, pose) in standing {
        if far {
            character::draw_far(&p.look, *feet, *facing, pose, cam, out);
        } else {
            draw(p, *feet, *facing, pose, cam, out);
        }
    }
}

/// Draws someone standing at `feet` facing `facing` (a camera's yaw), near or far as
/// they are from `cam`.
pub fn draw(person: &Person, feet: DVec3, facing: f32, pose: &Pose, cam: DVec3, out: &mut Vec<ParticleInstance>) {
    character::draw_seen(&person.look, feet, facing, pose, cam, out);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(look: &Look, far: bool, pose: &Pose) -> usize {
        let mut out = Vec::new();
        if far {
            character::draw_far(look, DVec3::ZERO, 0.4, pose, DVec3::new(3.0, 2.0, 1.0), &mut out);
        } else {
            character::draw(look, DVec3::ZERO, 0.4, pose, None, DVec3::new(3.0, 2.0, 1.0), &mut out);
        }
        assert!(out.iter().all(|b| b.pos.iter().chain(&b.size).all(|v| v.is_finite())), "a box is not where it can be drawn");
        out.len()
    }

    #[test]
    fn the_same_seed_makes_the_same_person() {
        for (k, region) in Region::ALL.into_iter().enumerate() {
            for trade in Trade::ALL {
                for age in [Age::Child, Age::Adult, Age::Elder] {
                    let seed = hash2(k as u32, trade as i32, age as i32);
                    assert_eq!(Person::new(seed, region, trade, 0.4, age), Person::new(seed, region, trade, 0.4, age));
                }
            }
            assert_eq!(household(77, region, "old", 0.5, Trade::Smith), household(77, region, "old", 0.5, Trade::Smith));
        }
        // And differs from seed to seed: no two of a village look alike.
        let village: Vec<Person> = (0..40).map(|s| Person::new(s, Region::Temperate, Trade::ALL[s as usize % 14], 0.5, Age::Adult)).collect();
        for (i, a) in village.iter().enumerate() {
            for b in &village[i + 1..] {
                assert_ne!(a.look, b.look, "{} and {} look alike", a.name, b.name);
            }
        }
    }

    #[test]
    fn a_household_shares_a_name_and_its_children_take_after_their_parents() {
        let mut families = 0;
        for seed in 0..200 {
            let region = Region::ALL[seed as usize % 8];
            let h = household(seed, region, folk::tongues()[seed as usize % 6], 0.5, Trade::Farmer);
            assert!(!h.is_empty());
            let family: Vec<&str> = h.iter().map(|p| p.name.split_once(' ').unwrap().1).collect();
            assert!(family.iter().all(|f| *f == family[0]), "{family:?}");
            let given: std::collections::HashSet<&str> = h.iter().map(|p| p.name.split(' ').next().unwrap()).collect();
            assert_eq!(given.len(), h.len(), "two of a household share a given name: {:?}", h.iter().map(|p| &p.name).collect::<Vec<_>>());
            let adults: Vec<&Person> = h.iter().filter(|p| p.age == Age::Adult).collect();
            let children: Vec<&Person> = h.iter().filter(|p| p.age == Age::Child).collect();
            if adults.len() == 2 && !children.is_empty() {
                families += 1;
                let (lo, hi) = (adults[0].skin.min(adults[1].skin), adults[0].skin.max(adults[1].skin));
                for c in children {
                    assert!((lo..=hi).contains(&c.skin), "{}: skin {} not between {lo} and {hi}", c.name, c.skin);
                    assert!(c.hair == adults[0].hair || c.hair == adults[1].hair, "{}: hair from neither parent", c.name);
                    assert_eq!(c.trade, Trade::None);
                }
            }
        }
        assert!(families > 40, "only {families} households of two parents and children in 200");
    }

    #[test]
    fn every_option_draws_and_keeps_to_its_budget() {
        let base = Person::new(5, Region::Temperate, Trade::None, 0.5, Age::Adult).look;
        let poses = [
            Pose::default(),
            character::Gait::striding(0.3, 1.3).pose(),
            character::Gait::striding(0.3, 6.9).pose(),
            character::Gait::gliding(0.0).pose(),
        ];
        let builds = [Build::PLAYER, Build::standing(1.1, 0.85, 1.36, 0.0), Build::standing(1.62, 1.28, 1.0, 0.3)];
        let mut most = 0;
        let mut checked = 0;
        for cut in Hair::ALL {
            for beard in Beard::ALL {
                for garment in Garment::ALL {
                    for layer in Layer::ALL {
                        for headwear in Headwear::ALL {
                            for (j, footwear) in Footwear::ALL.into_iter().enumerate() {
                                for (i, carried) in Carried::ALL.into_iter().enumerate() {
                                    let look = Look {
                                        cut,
                                        beard,
                                        garment,
                                        layer,
                                        headwear,
                                        footwear,
                                        carried,
                                        build: builds[(i + j) % 3],
                                        pack: (i + j) % 2 == 0,
                                        scarf: (i * 3 + j) % 4 == 0,
                                        rolled: i % 3 == 0,
                                        ..base
                                    };
                                    let pose = &poses[(i + j * 3) % poses.len()];
                                    let near = count(&look, false, pose);
                                    most = most.max(near);
                                    assert!(near <= 45, "{near} boxes: {look:?}");
                                    assert!(count(&look, true, pose) <= 8, "far: {look:?}");
                                    checked += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
        println!("{checked} looks drawn, at most {most} boxes");
    }

    #[test]
    fn generated_people_keep_to_the_budget_and_have_proportions() {
        let (mut fewest, mut most) = (usize::MAX, 0);
        for seed in 0..600u32 {
            let region = Region::ALL[seed as usize % 8];
            let age = [Age::Child, Age::Adult, Age::Adult, Age::Elder][seed as usize % 4];
            let p = Person::new(seed, region, Trade::ALL[seed as usize % 14], (seed % 10) as f32 / 9.0, age);
            let n = count(&p.look, false, &Pose::default());
            (fewest, most) = (fewest.min(n), most.max(n));
            assert!((21..=45).contains(&n), "{n}: {}", p.describe());
            let tall = p.look.build.metres();
            let expect = match age {
                Age::Child => 0.9..1.45,
                _ => 1.4..1.9,
            };
            assert!(expect.contains(&tall), "{} is {tall} m", p.describe());
            // A child's head is bigger against its body than a grown-up's.
            assert_eq!(p.look.build.head > 1.2, age == Age::Child);
            assert!(age == Age::Elder || p.look.build.stoop == 0.0);
        }
        println!("generated people are drawn in {fewest} to {most} boxes");
    }

    #[test]
    fn a_figure_far_off_is_a_few_boxes_and_beyond_sight_none() {
        let p = Person::new(9, Region::Northern, Trade::Herder, 0.5, Age::Adult);
        let at = |d: f64| {
            let mut out = Vec::new();
            draw(&p, DVec3::new(m64(d), 0.0, 0.0), 0.0, &Pose::default(), DVec3::ZERO, &mut out);
            out
        };
        assert!(at(10.0).len() > 20);
        assert!((5..=8).contains(&at(100.0).len()));
        assert!(at(195.0).iter().all(|b| b.dissolve > 0.0), "dithering out near the edge of sight");
        assert!(at(250.0).is_empty());
    }

    #[test]
    fn a_gallery_has_its_household_children_and_elders() {
        for region in Region::ALL {
            let (people, household) = gallery(1, region, 24, None);
            assert_eq!(people.len(), 24);
            assert!(people.iter().any(|p| p.age == Age::Child) && people.iter().any(|p| p.age == Age::Elder));
            assert!(household >= 3 && people[..household].iter().any(|p| p.age == Age::Child));
            let places = gallery_places(people.len(), household, 3.5);
            assert_eq!(places.len(), people.len());
            for p in people.iter().take(6) {
                println!("{:<10} {}", region.word(), p.describe());
            }
            let (elders, household) = gallery(1, region, 12, Some(Age::Elder));
            assert!(elders[household..].iter().all(|p| p.age == Age::Elder));
        }
    }
}
