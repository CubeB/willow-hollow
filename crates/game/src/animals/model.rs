//! Animals made of boxes (#206): who each is, made from a seed, and how it is drawn,
//! near and far, as the people are (#204).
//!
//! An animal is drawn as the figures and the crows are, as props: opaque boxes lit like
//! the terrain, casting shadows, and blocky as the world's plants and props are (#197).
//! A beast on four legs is a barrel of a body with a chest and a rump, a neck, a head
//! with its muzzle, ears and horns, four legs in two parts each and a tail; a bird is a
//! body with its breast and wings, a neck, a head with its bill and comb, a tail and two
//! legs. Sizes are in metres, of a grown female of the kind; the male and the young are
//! sized from her.
//!
//! In an animal's own frame x is to one side, y up from the ground under its middle, z
//! ahead, as a figure's is.

use glam::{DVec3, Vec3};
use worldgen::{
    fauna::Kind,
    noise::{hash2, unit},
    scale::{VOXELS_PER_METRE, m64},
};

use crate::particles::{ParticleInstance, dither_edge, dither_since};

/// Colours are sRGB.
pub type Rgb = [u8; 3];

/// The shape of body a kind has.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Shape {
    Fowl,
    Duck,
    Goose,
    Sheep,
    Goat,
    Cattle,
    Pig,
    Horse,
    Deer,
}

impl Shape {
    pub fn of(kind: Kind) -> Shape {
        use Kind::*;
        match kind {
            Chicken | Junglefowl => Shape::Fowl,
            Duck | WildDuck => Shape::Duck,
            Goose | WildGoose => Shape::Goose,
            Sheep | WildSheep => Shape::Sheep,
            Goat | WildGoat => Shape::Goat,
            Cattle | Ox | WildCattle => Shape::Cattle,
            Pig | Boar => Shape::Pig,
            Pony | Donkey | WildPony | WildAss => Shape::Horse,
            Reindeer | WildReindeer => Shape::Deer,
        }
    }

    pub fn bird(self) -> bool {
        matches!(self, Shape::Fowl | Shape::Duck | Shape::Goose)
    }
}

/// Who an animal is in its herd.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Role {
    Female,
    Male,
    Young,
}

/// Horns, tusks or antlers.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Horns {
    None,
    /// Short and straight up from the poll: a young beast's, or a polled breed's buds.
    Short,
    /// Curled round beside the head, as a ram's.
    Curl,
    /// Swept back over the neck in an arc, as a goat's and an ibex's.
    Sweep,
    /// Out to the sides and forward, as cattle's.
    Lyre,
    /// The boar's, curving up out of the jaw.
    Tusks,
    /// Branched, as a reindeer's.
    Antlers,
}

/// Markings over the body's own colour.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Marks {
    None,
    /// A pale patch on each flank, as a mouflon's.
    Saddle,
    /// A band round the body behind the shoulders, as a saddleback pig's.
    Belt,
    /// A dark line along the spine, as a dun's.
    Stripe,
    /// Patches of another colour on the flanks.
    Pied,
}

/// Everything that sets one animal apart from another.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub kind: Kind,
    pub shape: Shape,
    pub role: Role,
    /// Its size against a grown female of its kind.
    pub size: f32,
    /// The body: length, breadth and depth, and how far it stands off the ground, in
    /// metres; the withers raised this much above the back.
    pub length: f32,
    pub width: f32,
    pub depth: f32,
    pub leg: f32,
    pub hump: f32,
    /// The neck: its length and thickness, and how high it is held (radians above
    /// level) at ease.
    pub neck: f32,
    pub neck_width: f32,
    pub carriage: f32,
    /// The head: breadth, depth and length, and how far the nose is held down, radians.
    pub head: [f32; 3],
    pub nod: f32,
    pub ear: f32,
    /// Ears hanging down (1) or pricked up (0).
    pub lop: f32,
    pub tail: f32,
    pub horns: Horns,
    pub horn: f32,
    pub mane: bool,
    pub beard: bool,
    pub wool: bool,
    pub marks: Marks,
    pub coat: Rgb,
    pub belly: Rgb,
    /// The lower legs and feet.
    pub socks: Rgb,
    pub face: Rgb,
    pub muzzle: Rgb,
    /// Mane, tail tuft, beard and the markings.
    pub dark: Rgb,
    pub mark: Rgb,
    pub horn_colour: Rgb,
    /// Birds: the bill or beak, the comb and wattles if any, the wings, breast, tail and
    /// neck.
    pub bill: Rgb,
    pub comb: Option<Rgb>,
    pub wing: Rgb,
    pub breast: Rgb,
    pub tail_colour: Rgb,
    pub neck_colour: Rgb,
}

/// The choices made for one animal, each a hash of its seed.
struct Dice {
    seed: u32,
    n: i32,
}

impl Dice {
    fn new(seed: u32, side: u32) -> Self {
        Dice { seed: hash2(seed, side as i32, 0x0a71_3a15), n: 0 }
    }

    fn unit(&mut self) -> f32 {
        self.n += 1;
        unit(hash2(self.seed, self.n, 0))
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

pub(crate) fn shade(c: Rgb, by: f32) -> Rgb {
    c.map(|v| (v as f32 * by).round().clamp(0.0, 255.0) as u8)
}

pub(crate) fn blend(a: Rgb, b: Rgb, t: f32) -> Rgb {
    [0, 1, 2].map(|k| (a[k] as f32 + (b[k] as f32 - a[k] as f32) * t).round() as u8)
}

fn linear(c: Rgb) -> [f32; 4] {
    let l = |v: u8| (v as f32 / 255.0).powf(2.2);
    [l(c[0]), l(c[1]), l(c[2]), 1.0]
}

// Colours of hide, hair, wool and feather.
const WHITE: Rgb = [0xe6, 0xe2, 0xd6];
const CREAM: Rgb = [0xd8, 0xcc, 0xae];
const WOOL: Rgb = [0xdc, 0xd4, 0xbe];
const BLACK: Rgb = [0x26, 0x22, 0x20];
const GREY: Rgb = [0x84, 0x80, 0x7a];
const DARK_GREY: Rgb = [0x4e, 0x4a, 0x46];
const MOORIT: Rgb = [0x6c, 0x4a, 0x30];
const BROWN: Rgb = [0x5a, 0x3c, 0x26];
const RED: Rgb = [0x8e, 0x48, 0x26];
const GINGER: Rgb = [0xb0, 0x64, 0x30];
const DUN: Rgb = [0xb4, 0x96, 0x68];
const FAWN: Rgb = [0xa8, 0x82, 0x56];
const BAY: Rgb = [0x74, 0x40, 0x22];
const CHESTNUT: Rgb = [0x96, 0x52, 0x26];
const PINK: Rgb = [0xd8, 0xa4, 0x90];
const SKIN_PINK: Rgb = [0xc4, 0x88, 0x78];
const HOOF: Rgb = [0x3a, 0x32, 0x2c];
const HORN: Rgb = [0xa8, 0x9a, 0x80];
const DARK_HORN: Rgb = [0x5c, 0x52, 0x46];
const IVORY: Rgb = [0xe0, 0xd8, 0xc0];
const EYE: Rgb = [0x14, 0x12, 0x10];
const COMB: Rgb = [0xc4, 0x24, 0x1c];
const BEAK: Rgb = [0xd8, 0xb0, 0x50];
const ORANGE_BILL: Rgb = [0xe0, 0x8c, 0x2c];
const DRAKE_GREEN: Rgb = [0x1c, 0x4c, 0x30];
const MALLARD_HEN: Rgb = [0x8a, 0x6a, 0x48];
const GREYLAG: Rgb = [0x8a, 0x84, 0x76];
const SANDY: Rgb = [0xc0, 0xa8, 0x82];
const MEALY: Rgb = [0xc8, 0xa8, 0x78];

/// The sides of an animal, each with its own dice.
const BUILD: u32 = 1;
const COLOURS: u32 = 2;
const HEAD: u32 = 3;

impl Look {
    /// An animal of `kind`, as its seed makes it: `role` in its herd.
    pub fn new(kind: Kind, seed: u32, role: Role) -> Look {
        let shape = Shape::of(kind);
        let mut b = Dice::new(seed, BUILD);
        let mut c = Dice::new(seed, COLOURS);
        let mut h = Dice::new(seed, HEAD);
        let mut look = base(kind, shape);
        look.role = role;
        // Sized from a grown female: males bigger, the young well short of grown.
        let size = match role {
            Role::Female => b.range(0.92, 1.06),
            Role::Male => b.range(1.06, 1.18) * if kind == Kind::Boar || kind == Kind::WildCattle { 1.06 } else { 1.0 },
            Role::Young => b.range(0.5, 0.7),
        };
        let size = if kind == Kind::Ox { size.max(1.04) } else { size };
        look.size = size;
        colour(&mut look, kind, role, &mut c);
        horns(&mut look, kind, role, &mut h);
        // Each a little its own: broader or slighter, longer in the leg or the neck.
        look.width *= b.range(0.93, 1.07);
        look.depth *= b.range(0.95, 1.05);
        look.leg *= b.range(0.95, 1.05);
        look.neck *= b.range(0.94, 1.06);
        if role == Role::Young {
            // Long in the leg and big in the head for their size.
            look.leg *= 1.25;
            look.head = look.head.map(|v| v * 1.2);
            look.ear *= 1.15;
            look.beard = false;
            look.mane &= shape == Shape::Horse;
        }
        for v in [&mut look.length, &mut look.width, &mut look.depth, &mut look.leg, &mut look.hump, &mut look.neck, &mut look.neck_width, &mut look.ear, &mut look.tail] {
            *v *= size;
        }
        look.head = look.head.map(|v| v * size);
        look.horn *= size;
        look
    }

    /// From the ground to the top of the withers, standing, in metres.
    pub fn height(&self) -> f32 {
        self.leg + self.depth + self.hump
    }

    /// From nose to tail, roughly, in metres: how far apart herd mates stand.
    pub fn reach(&self) -> f32 {
        self.length + self.head[2] * 0.6 + if self.shape.bird() { 0.0 } else { self.neck * 0.3 }
    }

    /// A line about it, for logs.
    pub fn describe(&self) -> String {
        let role = match (self.role, self.shape) {
            (Role::Young, _) => "young",
            (Role::Male, Shape::Fowl) => "cock",
            (Role::Male, Shape::Duck) => "drake",
            (Role::Male, Shape::Goose) => "gander",
            (Role::Male, Shape::Sheep) => "ram",
            (Role::Male, Shape::Goat) => "billy",
            (Role::Male, Shape::Cattle) if self.kind == Kind::Ox => "ox",
            (Role::Male, Shape::Cattle) => "bull",
            (Role::Male, Shape::Pig) => "boar",
            (Role::Male, Shape::Horse) => "stallion",
            (Role::Male, Shape::Deer) => "bull",
            (Role::Female, Shape::Fowl | Shape::Duck) => "hen",
            (Role::Female, Shape::Goose) => "goose",
            (Role::Female, Shape::Sheep) => "ewe",
            (Role::Female, Shape::Goat) => "nanny",
            (Role::Female, Shape::Cattle) if self.kind == Kind::Ox => "ox",
            (Role::Female, Shape::Cattle | Shape::Deer) => "cow",
            (Role::Female, Shape::Pig) => "sow",
            (Role::Female, Shape::Horse) => "mare",
        };
        let hex = |c: Rgb| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
        format!(
            "{} {role}, {:.2} m at the withers, coat {}{}{}",
            self.kind.word(),
            self.height(),
            hex(self.coat),
            if self.marks == Marks::None { String::new() } else { format!(", {:?} marks {}", self.marks, hex(self.mark)).to_lowercase() },
            if self.horns == Horns::None { String::new() } else { format!(", {:?} horns {:.2} m", self.horns, self.horn).to_lowercase() },
        )
    }
}

/// A kind's grown female, in its commonest colours.
fn base(kind: Kind, shape: Shape) -> Look {
    let mut l = Look {
        kind,
        shape,
        role: Role::Female,
        size: 1.0,
        length: 1.0,
        width: 0.4,
        depth: 0.45,
        leg: 0.45,
        hump: 0.0,
        neck: 0.3,
        neck_width: 0.16,
        carriage: 0.5,
        head: [0.15, 0.16, 0.28],
        nod: 0.7,
        ear: 0.1,
        lop: 0.0,
        tail: 0.12,
        horns: Horns::None,
        horn: 0.0,
        mane: false,
        beard: false,
        wool: false,
        marks: Marks::None,
        coat: BROWN,
        belly: BROWN,
        socks: BROWN,
        face: BROWN,
        muzzle: BROWN,
        dark: BLACK,
        mark: BLACK,
        horn_colour: HORN,
        bill: BEAK,
        comb: None,
        wing: BROWN,
        breast: BROWN,
        tail_colour: BROWN,
        neck_colour: BROWN,
    };
    use Kind::*;
    let dims = |l: &mut Look, length, width, depth, leg, neck, neck_width, carriage, head: [f32; 3], nod, ear, tail| {
        (l.length, l.width, l.depth, l.leg, l.neck, l.neck_width, l.carriage, l.head, l.nod, l.ear, l.tail) = (length, width, depth, leg, neck, neck_width, carriage, head, nod, ear, tail);
    };
    match kind {
        Chicken => dims(&mut l, 0.3, 0.19, 0.2, 0.12, 0.1, 0.08, 0.0, [0.065, 0.075, 0.08], 0.0, 0.0, 0.12),
        Junglefowl => dims(&mut l, 0.28, 0.16, 0.18, 0.13, 0.1, 0.07, 0.0, [0.06, 0.07, 0.075], 0.0, 0.0, 0.16),
        Duck | WildDuck => dims(&mut l, 0.4, 0.2, 0.16, 0.07, 0.08, 0.075, 0.0, [0.07, 0.075, 0.09], 0.0, 0.0, 0.08),
        Goose | WildGoose => dims(&mut l, 0.6, 0.28, 0.25, 0.13, 0.26, 0.085, 0.0, [0.08, 0.09, 0.11], 0.0, 0.0, 0.1),
        Sheep => dims(&mut l, 0.95, 0.5, 0.48, 0.33, 0.22, 0.2, 0.5, [0.15, 0.17, 0.27], 0.8, 0.1, 0.12),
        WildSheep => dims(&mut l, 0.9, 0.33, 0.38, 0.45, 0.26, 0.15, 0.6, [0.13, 0.15, 0.25], 0.7, 0.08, 0.1),
        Goat => dims(&mut l, 0.8, 0.31, 0.38, 0.42, 0.28, 0.12, 0.75, [0.12, 0.14, 0.25], 0.65, 0.12, 0.1),
        WildGoat => dims(&mut l, 0.95, 0.36, 0.44, 0.47, 0.3, 0.15, 0.7, [0.14, 0.16, 0.27], 0.65, 0.1, 0.12),
        Cattle => dims(&mut l, 1.5, 0.62, 0.66, 0.62, 0.36, 0.34, 0.25, [0.26, 0.3, 0.5], 0.75, 0.15, 0.75),
        Ox => dims(&mut l, 1.75, 0.72, 0.78, 0.7, 0.4, 0.4, 0.2, [0.3, 0.34, 0.56], 0.75, 0.16, 0.85),
        WildCattle => dims(&mut l, 1.85, 0.66, 0.82, 0.78, 0.42, 0.42, 0.2, [0.28, 0.32, 0.58], 0.7, 0.15, 0.85),
        Pig => dims(&mut l, 1.05, 0.46, 0.48, 0.26, 0.18, 0.36, 0.0, [0.26, 0.26, 0.32], 0.35, 0.13, 0.12),
        Boar => dims(&mut l, 1.05, 0.36, 0.52, 0.34, 0.2, 0.34, 0.0, [0.22, 0.26, 0.42], 0.45, 0.1, 0.2),
        Pony | WildPony => dims(&mut l, 1.28, 0.46, 0.56, 0.68, 0.66, 0.22, 0.85, [0.18, 0.22, 0.5], 1.0, 0.12, 0.62),
        Donkey | WildAss => dims(&mut l, 1.1, 0.4, 0.5, 0.64, 0.5, 0.2, 0.7, [0.18, 0.22, 0.48], 0.9, 0.26, 0.48),
        Reindeer | WildReindeer => dims(&mut l, 1.15, 0.42, 0.52, 0.62, 0.42, 0.2, 0.6, [0.16, 0.18, 0.4], 0.75, 0.11, 0.08),
    }
    match kind {
        Pig => l.lop = 0.7,
        Boar => l.hump = 0.12,
        WildCattle => l.hump = 0.1,
        WildAss => (l.leg, l.length) = (0.72, 1.15),
        Pony => l.length = 1.25,
        Sheep => l.wool = true,
        Goat => l.lop = 0.2,
        _ => {}
    }
    l.mane = shape == Shape::Horse || shape == Shape::Deer;
    l
}

/// The coat, and for birds the plumage: the kept in the colours of their breeds, the
/// wild in their own.
fn colour(l: &mut Look, kind: Kind, role: Role, d: &mut Dice) {
    use Kind::*;
    let male = role == Role::Male;
    let set_all = |l: &mut Look, c: Rgb| {
        (l.coat, l.belly, l.socks, l.face, l.muzzle) = (c, c, c, c, c);
    };
    match kind {
        Chicken => {
            let (body, neck, tail) = d.pick(&[
                ((WHITE, WHITE, WHITE), 3.0),
                ((BLACK, BLACK, BLACK), 2.0),
                (([0xc8, 0x98, 0x58], [0xd4, 0xa8, 0x68], [0x9a, 0x6a, 0x38]), 3.0),
                (([0x8a, 0x4a, 0x26], [0xb0, 0x6a, 0x30], BLACK), 4.0),
                (([0x8c, 0x8a, 0x86], [0xa4, 0xa2, 0x9c], [0x5a, 0x58, 0x56]), 1.5),
                ((BROWN, [0xc0, 0x84, 0x3c], BLACK), 2.0),
            ]);
            set_hen(l, body, neck, tail);
            l.socks = if d.chance(0.6) { [0xd6, 0xb4, 0x48] } else { [0x6a, 0x6e, 0x70] };
            l.comb = Some(COMB);
            if male {
                // The cock: a red-gold cape and a dark tail when he is not all one colour.
                if body != WHITE && body != BLACK {
                    (l.neck_colour, l.wing, l.tail_colour) = ([0xc8, 0x6a, 0x24], [0x7a, 0x2a, 0x1a], [0x14, 0x24, 0x1c]);
                }
                l.tail *= 1.6;
            }
        }
        Junglefowl => {
            if male {
                set_hen(l, [0x5a, 0x22, 0x14], [0xd4, 0x7a, 0x26], [0x12, 0x26, 0x1e]);
                l.breast = [0x1a, 0x1a, 0x18];
                l.wing = [0x8a, 0x30, 0x18];
                l.tail *= 1.8;
            } else {
                set_hen(l, [0x7a, 0x5a, 0x3a], [0x9a, 0x7a, 0x4a], [0x5a, 0x40, 0x2a]);
            }
            l.comb = Some(COMB);
            l.socks = [0x5a, 0x5c, 0x5e];
        }
        Duck => {
            let (body, head) = d.pick(&[((WHITE, WHITE), 4.0), ((MALLARD_HEN, MALLARD_HEN), 2.0), (([0x9a, 0x86, 0x60], [0x6a, 0x5e, 0x44]), 2.0), ((BLACK, [0x1a, 0x2a, 0x22]), 1.0), ((CREAM, CREAM), 1.0)]);
            set_hen(l, body, head, shade(body, 0.85));
            if male && body == MALLARD_HEN {
                mallard_drake(l);
            }
            l.bill = if body == BLACK { [0x2a, 0x2a, 0x26] } else { ORANGE_BILL };
            l.socks = ORANGE_BILL;
        }
        WildDuck => {
            if male {
                set_hen(l, MALLARD_HEN, MALLARD_HEN, MALLARD_HEN);
                mallard_drake(l);
                l.bill = [0xc8, 0xb8, 0x40];
            } else {
                set_hen(l, MALLARD_HEN, [0x9a, 0x80, 0x5c], shade(MALLARD_HEN, 0.9));
                l.bill = [0xa8, 0x70, 0x30];
            }
            l.socks = ORANGE_BILL;
        }
        Goose => {
            let body = d.pick(&[(WHITE, 4.0), (GREYLAG, 3.0), (shade(GREYLAG, 1.12), 1.0)]);
            set_hen(l, body, if body == WHITE { WHITE } else { shade(body, 0.85) }, shade(body, 0.8));
            if body != WHITE {
                l.breast = blend(body, WHITE, 0.4);
            }
            l.bill = ORANGE_BILL;
            l.socks = ORANGE_BILL;
        }
        WildGoose => {
            set_hen(l, GREYLAG, shade(GREYLAG, 0.85), shade(GREYLAG, 0.75));
            l.breast = blend(GREYLAG, WHITE, 0.35);
            l.wing = shade(GREYLAG, 0.9);
            l.bill = ORANGE_BILL;
            l.socks = [0xd8, 0x98, 0x88];
        }
        Sheep => {
            let fleece = d.pick(&[(WOOL, 10.0), (CREAM, 3.0), (shade(BLACK, 1.3), 1.5), (MOORIT, 1.5), (GREY, 1.0)]);
            let face = d.pick(&[(blend(WOOL, WHITE, 0.4), 5.0), (BLACK, 3.0), (shade(MOORIT, 0.9), 1.0), ([0x9a, 0x8a, 0x76], 1.5)]);
            set_all(l, fleece);
            (l.face, l.muzzle, l.socks) = (face, shade(face, 0.9), face);
            l.belly = shade(fleece, 0.92);
            l.horn_colour = HORN;
        }
        WildSheep => {
            // The mouflon: short foxy brown, a pale saddle, white below and on the muzzle.
            let coat = shade([0x8a, 0x52, 0x2c], d.range(0.9, 1.08));
            set_all(l, coat);
            l.belly = CREAM;
            l.socks = blend(coat, CREAM, 0.6);
            l.muzzle = CREAM;
            l.face = shade(coat, 0.85);
            l.marks = if male { Marks::Saddle } else { Marks::None };
            l.mark = CREAM;
            l.dark = shade(coat, 0.55);
            l.horn_colour = [0x8a, 0x7a, 0x5c];
        }
        Goat => {
            let coat = d.pick(&[(WHITE, 4.0), (BLACK, 2.0), (BROWN, 2.0), (FAWN, 2.0), (GREY, 1.0), (CHESTNUT, 1.0)]);
            set_all(l, coat);
            if d.chance(0.35) {
                l.marks = Marks::Pied;
                l.mark = if coat == WHITE { d.pick(&[(BLACK, 1.0), (BROWN, 1.0)]) } else { WHITE };
            }
            if coat == BROWN && d.chance(0.5) {
                // Toggenburg's pale face stripes and legs.
                l.muzzle = CREAM;
                l.socks = CREAM;
            }
            l.dark = shade(coat, 0.7);
            l.beard = d.chance(0.6);
        }
        WildGoat => {
            let coat = shade([0x7a, 0x68, 0x50], d.range(0.9, 1.1));
            set_all(l, coat);
            l.belly = blend(coat, CREAM, 0.5);
            l.socks = shade(coat, 0.6);
            l.face = shade(coat, 0.85);
            l.marks = if male { Marks::Stripe } else { Marks::None };
            l.mark = shade(coat, 0.5);
            l.dark = shade(coat, 0.5);
            l.beard = male;
            l.horn_colour = DARK_HORN;
        }
        Cattle | Ox => {
            let coat = d.pick(&[(RED, 3.0), (BROWN, 2.0), (BLACK, 2.0), (DUN, 1.5), (shade(CREAM, 0.95), 1.0), (GINGER, 1.5), (FAWN, 1.0)]);
            set_all(l, coat);
            l.muzzle = blend(coat, PINK, 0.4);
            l.socks = shade(coat, 0.85);
            l.marks = d.pick(&[(Marks::None, 5.0), (Marks::Pied, 3.0), (Marks::Belt, 0.7)]);
            l.mark = if coat == shade(CREAM, 0.95) { d.pick(&[(BLACK, 1.0), (RED, 1.0)]) } else { WHITE };
            if l.marks == Marks::Pied && d.chance(0.5) {
                l.face = WHITE;
            }
            l.dark = shade(coat, 0.6);
            if kind == Ox {
                l.coat = if d.chance(0.5) { shade(CREAM, 0.95) } else { l.coat };
                (l.belly, l.face) = (l.coat, l.coat);
            }
        }
        WildCattle => {
            // The aurochs: the bulls black with a pale stripe down the back and a pale
            // ring round the muzzle, the cows red brown.
            let coat = if male { [0x22, 0x1c, 0x18] } else { shade([0x6a, 0x3a, 0x22], d.range(0.9, 1.1)) };
            set_all(l, coat);
            l.muzzle = [0xc8, 0xb8, 0x9c];
            l.marks = if male { Marks::Stripe } else { Marks::None };
            l.mark = [0xb4, 0xa0, 0x7a];
            l.dark = shade(coat, 0.7);
            l.horn_colour = [0xd4, 0xc8, 0xa8];
        }
        Pig => {
            let coat = d.pick(&[(PINK, 4.0), (BLACK, 2.0), (GINGER, 2.0), ([0x8a, 0x5a, 0x40], 1.0)]);
            set_all(l, coat);
            l.marks = match coat {
                BLACK => d.pick(&[(Marks::None, 1.0), (Marks::Belt, 2.0)]),
                PINK => d.pick(&[(Marks::None, 3.0), (Marks::Pied, 1.0)]),
                _ => Marks::None,
            };
            l.mark = if coat == BLACK { PINK } else { BLACK };
            l.muzzle = if coat == BLACK { [0x3a, 0x30, 0x2e] } else { SKIN_PINK };
            l.dark = shade(coat, 0.75);
            l.lop = if d.chance(0.6) { 0.8 } else { 0.0 };
        }
        Boar => {
            if role == Role::Young {
                // Humbugs: brown, striped pale along the back.
                set_all(l, [0x8a, 0x64, 0x3c]);
                l.marks = Marks::Stripe;
                l.mark = [0xc8, 0xaa, 0x7a];
            } else {
                let coat = shade([0x46, 0x3a, 0x30], d.range(0.85, 1.15));
                set_all(l, coat);
                l.socks = shade(coat, 0.7);
                l.mark = shade(coat, 0.6);
            }
            l.muzzle = [0x3a, 0x30, 0x2c];
            l.dark = shade(l.coat, 0.55);
            l.horn_colour = IVORY;
        }
        Pony => {
            let coat = d.pick(&[(BAY, 4.0), (CHESTNUT, 3.0), (GREY, 2.0), (BLACK, 1.5), (DUN, 1.5), (blend(GREY, WHITE, 0.5), 1.0)]);
            set_all(l, coat);
            l.dark = match coat {
                BAY | DUN => BLACK,
                CHESTNUT => shade(CHESTNUT, 0.85),
                c => shade(c, 0.7),
            };
            if coat == BAY || coat == DUN {
                l.socks = shade(BLACK, 1.2);
            }
            if d.chance(0.2) {
                l.marks = Marks::Pied;
                l.mark = WHITE;
            }
            if coat == DUN {
                l.marks = Marks::Stripe;
                l.mark = BLACK;
            }
            if d.chance(0.3) {
                l.face = blend(coat, WHITE, 0.85);
            }
        }
        Donkey => {
            let coat = d.pick(&[(GREY, 4.0), ([0x6a, 0x52, 0x40], 2.0), (DARK_GREY, 1.0), (blend(GREY, WHITE, 0.4), 1.0)]);
            set_all(l, coat);
            l.muzzle = CREAM;
            l.belly = blend(coat, CREAM, 0.5);
            l.marks = Marks::Stripe;
            l.mark = shade(coat, 0.5);
            l.dark = shade(coat, 0.5);
        }
        WildPony => {
            // Brown with a mealy muzzle and belly, dark legs, mane and tail.
            let coat = d.pick(&[(BAY, 3.0), (shade(BAY, 0.8), 1.0), (DUN, 2.0)]);
            set_all(l, coat);
            l.muzzle = MEALY;
            l.belly = blend(coat, MEALY, 0.6);
            l.socks = BLACK;
            l.dark = BLACK;
            if coat == DUN {
                l.marks = Marks::Stripe;
                l.mark = BLACK;
            }
        }
        WildAss => {
            set_all(l, shade(SANDY, d.range(0.92, 1.06)));
            l.belly = WHITE;
            l.socks = blend(SANDY, WHITE, 0.7);
            l.muzzle = WHITE;
            l.marks = Marks::Stripe;
            l.mark = [0x4a, 0x3a, 0x2c];
            l.dark = [0x4a, 0x3a, 0x2c];
        }
        Reindeer => {
            let coat = d.pick(&[([0x7a, 0x66, 0x52], 4.0), ([0x9a, 0x92, 0x86], 2.0), (WHITE, 1.0), ([0x4a, 0x3e, 0x34], 1.5)]);
            set_all(l, coat);
            l.belly = blend(coat, CREAM, 0.5);
            l.socks = shade(coat, 0.7);
            if d.chance(0.25) {
                l.marks = Marks::Pied;
                l.mark = WHITE;
            }
            l.dark = blend(coat, CREAM, 0.6);
            l.horn_colour = [0x8a, 0x76, 0x5c];
        }
        WildReindeer => {
            let coat = shade([0x6e, 0x5e, 0x4c], d.range(0.9, 1.1));
            set_all(l, coat);
            l.belly = CREAM;
            l.socks = shade(coat, 0.65);
            l.muzzle = shade(coat, 0.8);
            l.dark = blend(coat, CREAM, 0.7);
            l.horn_colour = [0x7e, 0x6a, 0x52];
        }
    }
    // Birds keep their colours in the plumage fields; give the beasts' eyes something.
    if !l.shape.bird() {
        l.neck_colour = l.coat;
    }
}

fn set_hen(l: &mut Look, body: Rgb, neck: Rgb, tail: Rgb) {
    (l.coat, l.breast, l.wing, l.tail_colour, l.neck_colour, l.face) = (body, shade(body, 1.04), shade(body, 0.92), tail, neck, neck);
}

fn mallard_drake(l: &mut Look) {
    l.coat = [0x9a, 0x96, 0x8c];
    l.wing = [0x7a, 0x74, 0x6a];
    l.breast = [0x6a, 0x3a, 0x2a];
    l.neck_colour = DRAKE_GREEN;
    l.face = DRAKE_GREEN;
    l.tail_colour = [0x2a, 0x2a, 0x28];
}

/// Horns, tusks and antlers, as the kind and the beast have them.
fn horns(l: &mut Look, kind: Kind, role: Role, d: &mut Dice) {
    use Kind::*;
    let (male, young) = (role == Role::Male, role == Role::Young);
    let (horns, length) = match kind {
        Sheep if male && d.chance(0.6) => (Horns::Curl, d.range(0.3, 0.45)),
        Sheep if !young && d.chance(0.15) => (Horns::Short, 0.08),
        WildSheep if male => (Horns::Curl, d.range(0.4, 0.55)),
        WildSheep if !young => (Horns::Short, d.range(0.06, 0.1)),
        Goat if young => (Horns::Short, 0.05),
        Goat if d.chance(0.7) => (Horns::Sweep, if male { d.range(0.3, 0.5) } else { d.range(0.15, 0.25) }),
        WildGoat if young => (Horns::Short, 0.08),
        WildGoat => (Horns::Sweep, if male { d.range(0.65, 0.9) } else { d.range(0.2, 0.3) }),
        Cattle if young => (Horns::None, 0.0),
        Cattle if d.chance(0.3) => (Horns::None, 0.0),
        Cattle => (Horns::Lyre, if male { d.range(0.3, 0.45) } else { d.range(0.2, 0.4) }),
        Ox => (Horns::Lyre, d.range(0.45, 0.7)),
        WildCattle if young => (Horns::Short, 0.1),
        WildCattle => (Horns::Lyre, if male { d.range(0.7, 0.85) } else { d.range(0.5, 0.65) }),
        Boar if male => (Horns::Tusks, 0.1),
        Reindeer | WildReindeer if young => (Horns::Short, 0.12),
        // Both reindeer wear antlers, the bulls far bigger.
        Reindeer | WildReindeer => (Horns::Antlers, if male { d.range(0.85, 1.1) } else { d.range(0.45, 0.6) }),
        _ => (Horns::None, 0.0),
    };
    l.horns = horns;
    l.horn = length;
}

/// How an animal is holding itself this moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    /// Through its stride, radians: once round for each pair of steps.
    pub stride: f32,
    pub gait: Gait,
    /// How far the legs reach: 0 standing, 1 at the gait's full stride.
    pub reach: f32,
    /// Head down to the grass: 0 up, 1 at the ground.
    pub graze: f32,
    /// The head held high, alert: 0 at ease, 1 up and staring.
    pub alert: f32,
    /// The head turned from the body's heading, radians, toward the beast's left as a
    /// camera's yaw turns.
    pub look: f32,
    /// An ear flicked (-1 to 1) and the tail swished.
    pub ears: f32,
    pub tail: f32,
    /// The body tipped nose down along the slope, radians.
    pub pitch: f32,
    /// Afloat: only birds swim, sitting low on the water with their legs out of sight.
    pub afloat: bool,
    /// A bird's wings out from its sides, 0 to 1, as it runs or flutters.
    pub flap: f32,
}

impl Default for Pose {
    fn default() -> Self {
        Pose { stride: 0.0, gait: Gait::Walk, reach: 0.0, graze: 0.0, alert: 0.0, look: 0.0, ears: 0.0, tail: 0.0, pitch: 0.0, afloat: false, flap: 0.0 }
    }
}

/// How the legs go.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Gait {
    Walk,
    Trot,
    Run,
}

impl Gait {
    /// Where each leg is in the stride, as a share of it: the near fore, the off fore,
    /// the near hind and the off hind.
    fn timing(self) -> [f32; 4] {
        match self {
            // A walk sets each foot down in turn, hind then fore on one side, then the other.
            Gait::Walk => [0.25, 0.75, 0.0, 0.5],
            // A trot moves the legs in diagonal pairs.
            Gait::Trot => [0.0, 0.5, 0.5, 0.0],
            // A gallop: the hinds close together, then the fores.
            Gait::Run => [0.5, 0.6, 0.0, 0.1],
        }
    }

    /// How far a leg swings at full reach, and how far the lower leg folds as it comes
    /// forward, radians.
    fn swing(self) -> (f32, f32) {
        match self {
            Gait::Walk => (0.32, 0.55),
            Gait::Trot => (0.48, 1.0),
            Gait::Run => (0.72, 1.3),
        }
    }
}

/// Where a figure stands and which way it faces, for setting its parts in the world.
struct Frame {
    /// The feet, from the camera.
    base: Vec3,
    yaw: f32,
    /// The body tipped along the slope.
    pitch: f32,
    /// One metre, in voxels.
    s: f32,
}

/// A box of a part: its low corner and size, in metres from the part's joint.
type Piece = ([f32; 3], [f32; 3], Rgb);

/// Turns `v` about the side-to-side axis: positive tips what points ahead down, and
/// swings what hangs down back.
fn tip(v: Vec3, p: f32) -> Vec3 {
    let (s, c) = p.sin_cos();
    Vec3::new(v.x, v.y * c - v.z * s, v.y * s + v.z * c)
}

/// Turns `v` about the upright, as a part's own `turn` does.
fn spin(v: Vec3, t: f32) -> Vec3 {
    let (s, c) = t.sin_cos();
    Vec3::new(v.x * c - v.z * s, v.y, v.x * s + v.z * c)
}

impl Frame {
    fn new(feet: DVec3, cam: DVec3, facing: f32, pitch: f32) -> Self {
        // The animal looks along its own z; a camera at yaw 0 looks along x.
        Frame { base: (feet - cam).as_vec3(), yaw: facing - std::f32::consts::FRAC_PI_2, pitch, s: VOXELS_PER_METRE }
    }

    /// Boxes of a part hung from `joint` (metres, in the animal's frame), the part
    /// tipped by `pitch` and turned by `turn` about the upright through its joint, each
    /// box first rolled by `roll` about the part's own z. The whole tips with the body.
    fn part(&self, out: &mut Vec<ParticleInstance>, joint: Vec3, pitch: f32, turn: f32, roll: f32, pieces: &[Piece]) {
        let joint = tip(joint, self.pitch);
        let pitch = pitch + self.pitch;
        let yaw = self.yaw;
        let (by, bx) = yaw.sin_cos();
        let (sy, cy) = (yaw + turn).sin_cos();
        for (lo, size, color) in pieces {
            let (lo, size) = (Vec3::from(*lo), Vec3::from(*size));
            let mut mid = lo + size * 0.5;
            if roll != 0.0 {
                let (sr, cr) = roll.sin_cos();
                mid = Vec3::new(mid.x * cr - mid.y * sr, mid.x * sr + mid.y * cr, mid.z);
            }
            let tipped = tip(mid, pitch);
            let at = Vec3::new(joint.x * bx - joint.z * by, joint.y, joint.x * by + joint.z * bx)
                + Vec3::new(tipped.x * cy - tipped.z * sy, tipped.y, tipped.x * sy + tipped.z * cy);
            out.push(ParticleInstance {
                pos: (self.base + at * self.s).into(),
                yaw: yaw + turn,
                size: (size * self.s).into(),
                glow: 0.0,
                color: linear(*color),
                tilt: [pitch, roll],
                dissolve: 0.0,
            });
        }
    }
}

/// Where the end of a part `length` long, pointing ahead from `joint`, has got to,
/// tipped by `pitch` and turned by `turn`.
fn reach_from(joint: Vec3, pitch: f32, turn: f32, length: f32) -> Vec3 {
    joint + spin(tip(Vec3::new(0.0, 0.0, length), pitch), turn)
}

/// Where the foot of a leg `length` long hung from `joint` has got to.
fn down_from(joint: Vec3, pitch: f32, length: f32) -> Vec3 {
    joint + tip(Vec3::new(0.0, -length, 0.0), pitch)
}

/// Most boxes an animal is drawn with near, and far.
pub const NEAR_MOST: usize = 32;
pub const FAR_MOST: usize = 6;

/// The swing of leg `k` (near fore, off fore, near hind, off hind) at this moment of
/// the stride, and how far its lower part folds.
fn leg_swing(pose: &Pose, k: usize) -> (f32, f32) {
    let (amp, fold) = pose.gait.swing();
    let phase = pose.stride + std::f32::consts::TAU * pose.gait.timing()[k];
    let (s, c) = phase.sin_cos();
    (amp * pose.reach * s, fold * pose.reach * (-c).max(0.0))
}

/// The neck's angle above level, the head's nose-down tip and the neck's length, from
/// the pose: held up at ease or higher when alert, or reaching down so the muzzle is at
/// the grass. A neck too short to reach it is stretched as far as it must be: drawn in
/// boxes, a longer neck reads better than a muzzle cropping the air.
fn carriage(l: &Look, pose: &Pose, joint_y: f32) -> (f32, f32, f32) {
    let [_, hh, hl] = l.head;
    const NOD: f32 = 1.4;
    const STEEPEST: f32 = 1.25;
    // How far the muzzle's lowest edge hangs below the poll, the head tipped down.
    let hang = hh * 0.5 * NOD.cos() + hl * NOD.sin();
    let drop = joint_y - hang - 0.015;
    let (down, length) = if drop > l.neck * STEEPEST.sin() { (-STEEPEST, drop / STEEPEST.sin()) } else { (-(drop / l.neck).clamp(-1.0, 1.0).asin(), l.neck) };
    let up = l.carriage + 0.35 * pose.alert;
    let neck = up + (down - up) * pose.graze;
    let nod = l.nod * (1.0 - 0.45 * pose.alert) + (NOD - l.nod) * pose.graze;
    (neck, nod, l.neck + (length - l.neck) * pose.graze)
}

/// Draws a beast on four legs standing with its feet about `feet`, facing `facing` (a
/// camera's yaw), for a camera at `cam`.
fn draw_beast(l: &Look, feet: DVec3, facing: f32, pose: &Pose, cam: DVec3, out: &mut Vec<ParticleInstance>) {
    let frame = Frame::new(feet, cam, facing, pose.pitch);
    let (len, w, d, leg) = (l.length, l.width, l.depth, l.leg);
    // A trot and a gallop lift the body a little at each push off.
    let bob = match pose.gait {
        Gait::Walk => 0.0,
        Gait::Trot | Gait::Run => (pose.stride * 2.0).sin().abs() * 0.04 * pose.reach * l.size,
    };
    let top = leg + d + bob;
    let mut part = |joint: Vec3, pitch: f32, turn: f32, roll: f32, pieces: &[Piece]| frame.part(out, joint, pitch, turn, roll, pieces);
    let wool = l.wool;

    // The barrel, its belly below, the chest and withers at the front and the rump
    // behind; a woolly sheep is one deep fleece.
    let mut body: Vec<Piece> = Vec::with_capacity(8);
    let belly = d * 0.28;
    body.push(([-w * 0.5, leg + bob + belly, -len * 0.5], [w, d - belly, len * 0.98], l.coat));
    body.push(([-w * 0.46, leg + bob, -len * 0.4], [w * 0.92, belly + 0.01, len * 0.8], l.belly));
    if !wool {
        let chest = len * 0.32;
        body.push(([-w * 0.47, leg + bob + belly * 0.4, len * 0.5 - chest], [w * 0.94, d - belly * 0.4 + l.hump, chest], l.coat));
        body.push(([-w * 0.48, leg + bob + belly * 0.6, -len * 0.52], [w * 0.96, d * 0.9 - belly * 0.6, len * 0.26], shade(l.coat, 0.97)));
    }
    match l.marks {
        Marks::None => {}
        Marks::Saddle => body.push(([-w * 0.52, leg + bob + d * 0.4, -len * 0.15], [w * 1.04, d * 0.35, len * 0.35], l.mark)),
        Marks::Belt => body.push(([-w * 0.52, leg + bob + belly * 0.5, len * 0.05], [w * 1.04, d - belly * 0.5 + 0.01, len * 0.22], l.mark)),
        Marks::Stripe => body.push(([-w * 0.1, top - 0.005, -len * 0.5], [w * 0.2, 0.025, len * 0.95], l.mark)),
        Marks::Pied => body.extend([
            ([-w * 0.52, leg + bob + belly, -len * 0.3], [w * 1.04, d * 0.55, len * 0.28], l.mark),
            ([-w * 0.52, leg + bob + d * 0.35, len * 0.12], [w * 0.4, d * 0.5, len * 0.22], l.mark),
        ]),
    }
    part(Vec3::ZERO, 0.0, 0.0, 0.0, &body);

    // Four legs, each an upper part in the coat and a lower in the socks, the lower
    // folding back as the leg comes forward.
    let lw = (w * 0.22).clamp(0.05, 0.13) * if wool { 0.75 } else { 1.0 };
    let upper = leg * 0.55;
    for k in 0..4 {
        let side = if k % 2 == 0 { 1.0 } else { -1.0 };
        let fore = k < 2;
        let z = if fore { len * 0.5 - lw * 1.4 } else { -len * 0.5 + lw * 1.4 };
        let hip = Vec3::new(side * (w * 0.5 - lw * 0.6), leg + bob + 0.02, z);
        let (swing, fold) = leg_swing(pose, k);
        let thigh = if fore { l.coat } else { shade(l.coat, 0.95) };
        part(hip, swing, 0.0, 0.0, &[([-lw * 0.5, -upper, -lw * 0.55], [lw, upper + 0.04, lw * 1.1], thigh)]);
        let knee = down_from(hip, swing, upper);
        let fold = if fore { fold } else { -fold * 0.5 };
        let lower = leg - upper + bob;
        part(knee, swing + fold, 0.0, 0.0, &[([-lw * 0.42, -lower, -lw * 0.45], [lw * 0.84, lower, lw * 0.9], l.socks)]);
    }

    // The neck from the top of the chest, and the head from the end of it; head down,
    // the neck comes from lower on the chest.
    let root = if l.shape == Shape::Pig { 0.55 - 0.15 * pose.graze } else { 0.78 - 0.25 * pose.graze };
    let neck_root = Vec3::new(0.0, leg + bob + d * root, len * 0.42);
    let (rise, nod, neck_len) = carriage(l, pose, neck_root.y);
    let (neck_turn, head_turn) = (pose.look * 0.45, pose.look);
    let nw = l.neck_width;
    let mut neck: Vec<Piece> = vec![([-nw * 0.5, -nw * 0.5, -nw * 0.2], [nw, nw, neck_len + nw * 0.3], l.coat)];
    if l.mane {
        let mane = if l.shape == Shape::Deer { l.dark } else { l.dark };
        if l.shape == Shape::Deer {
            // The pale ruff hanging under a reindeer's neck.
            neck.push(([-nw * 0.55, -nw * 0.8, 0.0], [nw * 1.1, nw * 0.5, neck_len * 0.8], mane));
        } else {
            neck.push(([-nw * 0.18, nw * 0.5, -nw * 0.1], [nw * 0.36, nw * 0.35, neck_len + nw * 0.2], mane));
        }
    }
    part(neck_root, -rise, neck_turn, 0.0, &neck);
    let poll = reach_from(neck_root, -rise, neck_turn, neck_len);
    let [hw, hh, hl] = l.head;
    let skull = hl * 0.55;
    let snout = if l.shape == Shape::Pig { hw * 0.75 } else { hw * 0.8 };
    let mut head: Vec<Piece> = vec![
        ([-hw * 0.5, -hh * 0.5, -hh * 0.2], [hw, hh, skull + hh * 0.2], l.face),
        ([-snout * 0.5, -hh * 0.5, skull], [snout, hh * 0.72, hl - skull], l.muzzle),
        // The eyes, one on each side.
        ([-hw * 0.5 - 0.012, hh * 0.08, skull * 0.55], [0.024, hh * 0.18, hh * 0.18], EYE),
        ([hw * 0.5 - 0.012, hh * 0.08, skull * 0.55], [0.024, hh * 0.18, hh * 0.18], EYE),
    ];
    if l.shape == Shape::Pig {
        // The flat disc of the snout.
        head.push(([-snout * 0.45, -hh * 0.45, hl - 0.01], [snout * 0.9, hh * 0.6, 0.03], shade(l.muzzle, 0.8)));
    }
    if l.beard {
        head.push(([-hw * 0.18, -hh * 0.5 - hl * 0.25, skull * 0.9], [hw * 0.36, hl * 0.28, hl * 0.2], l.dark));
    }
    horns_on(l, &mut head, hw, hh, skull);
    part(poll, nod, head_turn, 0.0, &head);
    // The ears, at the back of the skull, pricked up or hanging, flicking now and then.
    let e = l.ear;
    for side in [1.0f32, -1.0] {
        let flick = pose.ears * if side > 0.0 { 1.0 } else { -0.4 };
        let base = poll + spin(tip(Vec3::new(side * hw * 0.45, hh * 0.4, hh * 0.15), nod), head_turn);
        let roll = side * (0.5 + l.lop * 1.4 + flick * 0.4);
        part(base, nod * (1.0 - l.lop) - 0.2, head_turn + side * 0.3, roll, &[([-e * 0.2, 0.0, -e * 0.15], [e * 0.4, e, e * 0.3], shade(l.face, 0.92))]);
    }

    // The tail, hung from the top of the rump, swishing.
    if l.tail > 0.0 {
        let root = Vec3::new(0.0, top - d * 0.12, -len * 0.5);
        let t = l.tail;
        let tw = (w * 0.12).clamp(0.03, 0.08);
        let (lift, colour) = match l.shape {
            Shape::Pig => (-2.2, l.coat),
            Shape::Goat | Shape::Deer => (-2.5, l.coat),
            Shape::Sheep => (0.25, l.coat),
            _ => (0.12, l.coat),
        };
        let mut tail: Vec<Piece> = vec![([-tw * 0.5, -t, -tw * 0.5], [tw, t, tw], colour)];
        match l.shape {
            // The long hair of a horse's tail; a tuft at the end of a cow's.
            Shape::Horse => tail[0] = ([-tw * 0.9, -t, -tw * 0.9], [tw * 1.8, t, tw * 1.6], l.dark),
            Shape::Cattle => tail.push(([-tw * 0.9, -t - 0.08, -tw * 0.9], [tw * 1.8, 0.14, tw * 1.8], l.dark)),
            _ => {}
        }
        part(root, lift, pose.tail * 0.5, 0.0, &tail);
    }
}

/// Horns, tusks or antlers on a head whose skull is `skull` long.
fn horns_on(l: &Look, head: &mut Vec<Piece>, hw: f32, hh: f32, skull: f32) {
    let (h, c) = (l.horn, l.horn_colour);
    let tip_c = shade(c, 0.7);
    let t = (h * 0.16).clamp(0.025, 0.08);
    let top = hh * 0.5;
    match l.horns {
        Horns::None => {}
        Horns::Short => {
            for side in [1.0f32, -1.0] {
                head.push(([side * hw * 0.25 - t * 0.5, top, 0.0], [t, h.max(0.04), t], c));
            }
        }
        Horns::Curl => {
            // Back from the poll and round down beside the cheek.
            for side in [1.0f32, -1.0] {
                let x = side * (hw * 0.5 + t * 0.4) - t * 0.5;
                head.extend([
                    ([x, top - t * 0.5, -h * 0.25], [t * 1.1, t * 1.4, h * 0.4], c),
                    ([x + side * t * 0.3, -hh * 0.3, -h * 0.3], [t * 1.1, top + hh * 0.3, t * 1.4], c),
                    ([x + side * t * 0.4, -hh * 0.4, -h * 0.3], [t, t * 1.1, h * 0.35], tip_c),
                ]);
            }
        }
        Horns::Sweep => {
            // Up and back over the neck.
            for side in [1.0f32, -1.0] {
                let x = side * hw * 0.22 - t * 0.5;
                head.extend([([x, top, -t * 0.5], [t, h * 0.45, t * 1.2], c), ([x + side * t * 0.2, top + h * 0.35, -h * 0.5], [t * 0.85, t * 1.1, h * 0.55], tip_c)]);
            }
        }
        Horns::Lyre => {
            // Out to the sides from the poll, then forward and up.
            for side in [1.0f32, -1.0] {
                let out = hw * 0.5 + h * 0.45;
                head.extend([
                    ([if side > 0.0 { hw * 0.3 } else { -out }, top - t, -t * 0.5], [out - hw * 0.3, t, t], c),
                    ([side * out - t * 0.5, top - t, -t * 0.5], [t, h * 0.5, t], tip_c),
                ]);
            }
        }
        Horns::Tusks => {
            for side in [1.0f32, -1.0] {
                head.push(([side * hw * 0.32 - 0.012, -hh * 0.2, skull + 0.06], [0.024, 0.07, 0.03], c));
            }
        }
        Horns::Antlers => {
            // A beam up and back from each side of the poll, a brow tine forward over
            // the face and a tine up from the beam's middle; the tops palmate.
            for side in [1.0f32, -1.0] {
                let x = side * (hw * 0.35 + h * 0.2) - t * 0.5;
                head.extend([
                    ([side * hw * 0.3 - t * 0.5, top, -t], [t, h * 0.55, t], c),
                    ([x, top + h * 0.5, -h * 0.45], [t, t, h * 0.5], c),
                    ([x, top + h * 0.5, -h * 0.45], [t * 1.2, h * 0.45, t * 1.6], tip_c),
                    ([side * hw * 0.3 - t * 0.5, top + h * 0.12, 0.0], [t * 0.8, t * 0.8, skull * 0.9], c),
                ]);
            }
        }
    }
}

/// Draws a bird with its feet at `feet` (or afloat with its body on the water there).
fn draw_bird(l: &Look, feet: DVec3, facing: f32, pose: &Pose, cam: DVec3, out: &mut Vec<ParticleInstance>) {
    let frame = Frame::new(feet, cam, facing, pose.pitch);
    let (len, w, d) = (l.length, l.width, l.depth);
    // Afloat, the water comes a third of the way up the body and the legs are under it.
    // Pecking, a bird bends its legs a little and tips forward.
    let crouch = if pose.afloat { 0.0 } else { l.leg * 0.3 * pose.graze };
    let leg = if pose.afloat { -d * 0.35 } else { l.leg - crouch };
    let mut part = |joint: Vec3, pitch: f32, turn: f32, roll: f32, pieces: &[Piece]| frame.part(out, joint, pitch, turn, roll, pieces);
    // A hen holds her body tipped tail up; a duck and a goose level.
    let tilt = match l.shape {
        Shape::Fowl => -0.22,
        _ => -0.05,
    } + pose.graze * 0.45;
    let middle = Vec3::new(0.0, leg + d * 0.5, 0.0);
    part(
        middle,
        tilt,
        0.0,
        0.0,
        &[
            ([-w * 0.5, -d * 0.5, -len * 0.5], [w, d, len], l.coat),
            ([-w * 0.46, -d * 0.55, len * 0.02], [w * 0.92, d * 0.62, len * 0.46], l.breast),
        ],
    );
    // Wings at the sides, out a little as it runs.
    for side in [1.0f32, -1.0] {
        part(
            middle + Vec3::new(side * w * 0.5, d * 0.2, -len * 0.05),
            tilt,
            0.0,
            side * (0.1 + pose.flap * 0.9),
            &[([-0.015, -d * 0.62, -len * 0.42], [0.03, d * 0.62, len * 0.68], l.wing)],
        );
    }
    // The tail, raised at the back: a cock's tall and arched.
    let tail_root = middle + tip(Vec3::new(0.0, d * 0.3, -len * 0.45), tilt);
    let t = l.tail;
    let up = match l.shape {
        Shape::Fowl => -1.0,
        _ => -0.35,
    };
    let mut tail: Vec<Piece> = vec![([-w * 0.3, -0.02, -t * 0.9], [w * 0.6, t * 0.35, t], l.tail_colour)];
    if l.shape == Shape::Fowl && l.role == Role::Male {
        tail.push(([-0.02, 0.0, -t * 0.7], [0.04, t * 0.9, t * 0.5], shade(l.tail_colour, 1.2)));
    }
    part(tail_root, up, pose.tail * 0.2, 0.0, &tail);
    // The neck up from the front of the body, and the head on it; pecking, the neck
    // tips over to bring the bill to the ground.
    let neck_root = middle + tip(Vec3::new(0.0, d * 0.25, len * 0.38), tilt);
    let nw = l.neck_width;
    let n = l.neck;
    let lean = match l.shape {
        Shape::Goose => 0.15,
        Shape::Duck => 0.4,
        _ => 0.2,
    };
    let [hw, hh, hl] = l.head;
    // How far over the neck must go for the tip of the bill to reach the ground: the
    // bend at which the head, tipped with it, comes lowest to the ground, found by halves.
    let tip_of_bill = |bend: f32| {
        let head = bend * 0.55 - lean * 0.3;
        neck_root.y + n * bend.cos() + tip(Vec3::new(0.0, hh * 0.1, hl), head).y
    };
    let (mut lo, mut hi) = (lean, 2.6f32);
    if pose.graze > 0.0 && tip_of_bill(hi) < 0.02 {
        for _ in 0..12 {
            let mid = (lo + hi) * 0.5;
            if tip_of_bill(mid) > 0.02 { lo = mid } else { hi = mid }
        }
    }
    let down = hi;
    let bend = lean * (1.0 - pose.alert) + (down - lean) * pose.graze;
    part(neck_root, bend, pose.look * 0.4, 0.0, &[([-nw * 0.5, -nw * 0.3, -nw * 0.5], [nw, n + nw * 0.3, nw], l.neck_colour)]);
    let crown = neck_root + spin(tip(Vec3::new(0.0, n, 0.0), bend), pose.look * 0.4);
    let bill = match l.shape {
        Shape::Fowl => ([-hw * 0.2, hh * 0.15, hl * 0.6], [hw * 0.4, hh * 0.3, hl * 0.4]),
        _ => ([-hw * 0.32, hh * 0.05, hl * 0.6], [hw * 0.64, hh * 0.28, hl * 0.75]),
    };
    let mut head: Vec<Piece> = vec![
        ([-hw * 0.5, 0.0, -hl * 0.4], [hw, hh, hl], l.face),
        (bill.0, bill.1, l.bill),
        ([-hw * 0.5 - 0.006, hh * 0.5, hl * 0.2], [0.012, 0.018, 0.018], EYE),
        ([hw * 0.5 - 0.006, hh * 0.5, hl * 0.2], [0.012, 0.018, 0.018], EYE),
    ];
    if let Some(comb) = l.comb {
        let tall = if l.role == Role::Male { 0.055 } else { 0.025 } * l.size;
        head.extend([([-0.01, hh, -hl * 0.2], [0.02, tall, hl * 0.75], comb), ([-0.012, -0.035 * l.size, hl * 0.45], [0.024, 0.04 * l.size, 0.03], comb)]);
    }
    part(crown, bend * 0.55 - lean * 0.3, pose.look, 0.0, &head);
    if !pose.afloat {
        // Two legs, stepping in turn, each with its foot.
        for (k, side) in [1.0f32, -1.0].into_iter().enumerate() {
            let hip = Vec3::new(side * w * 0.22, leg + 0.01, len * 0.02 + crouch * 0.5);
            let (s, _) = (pose.stride + std::f32::consts::PI * k as f32).sin_cos();
            let swing = s * 0.6 * pose.reach;
            let lw = (w * 0.12).max(0.02);
            let foot = if l.shape == Shape::Fowl { lw * 2.4 } else { lw * 3.2 };
            part(hip, swing, 0.0, 0.0, &[([-lw * 0.5, -leg - 0.01, -lw * 0.5], [lw, leg + 0.02, lw], l.socks), ([-foot * 0.5, -leg, -lw * 0.2], [foot, lw * 0.5, foot * 1.1], l.socks)]);
        }
    }
}

/// Draws an animal near, in all its parts.
pub fn draw(l: &Look, feet: DVec3, facing: f32, pose: &Pose, cam: DVec3, out: &mut Vec<ParticleInstance>) {
    if l.shape.bird() {
        draw_bird(l, feet, facing, pose, cam, out);
    } else {
        draw_beast(l, feet, facing, pose, cam, out);
    }
}

/// Draws an animal far off, in [`FAR_MOST`] boxes or fewer, coloured as the near one:
/// the body, the neck and head, the legs fore and hind as a pair each, and its horns,
/// antlers or tail, whichever shows most from a distance.
pub fn draw_far(l: &Look, feet: DVec3, facing: f32, pose: &Pose, cam: DVec3, out: &mut Vec<ParticleInstance>) {
    let frame = Frame::new(feet, cam, facing, pose.pitch);
    let mut part = |joint: Vec3, pitch: f32, turn: f32, pieces: &[Piece]| frame.part(out, joint, pitch, turn, 0.0, pieces);
    let (len, w, d) = (l.length, l.width, l.depth);
    if l.shape.bird() {
        let leg = if pose.afloat { -d * 0.35 } else { l.leg };
        part(Vec3::new(0.0, leg + d * 0.5, 0.0), 0.0, 0.0, &[([-w * 0.5, -d * 0.5, -len * 0.5], [w, d, len], l.coat)]);
        let root = Vec3::new(0.0, leg + d * 0.7, len * 0.38);
        let bend = 0.2 + 1.6 * pose.graze;
        part(root, bend, 0.0, &[([-l.neck_width * 0.6, 0.0, -l.neck_width * 0.6], [l.neck_width * 1.2, l.neck + l.head[1], l.neck_width * 1.2], l.neck_colour)]);
        if !pose.afloat {
            part(Vec3::new(0.0, leg, 0.0), 0.0, 0.0, &[([-w * 0.25, -leg, -0.02], [w * 0.5, leg, 0.04], l.socks)]);
        }
        return;
    }
    let leg = l.leg;
    part(Vec3::ZERO, 0.0, 0.0, &[([-w * 0.5, leg, -len * 0.5], [w, d + l.hump * 0.5, len], l.coat)]);
    for fore in [true, false] {
        let (a, _) = leg_swing(pose, if fore { 0 } else { 2 });
        let z = if fore { len * 0.38 } else { -len * 0.38 };
        let lw = (w * 0.22).clamp(0.05, 0.13);
        part(Vec3::new(0.0, leg + 0.02, z), a * 0.6, 0.0, &[([-w * 0.42, -leg - 0.02, -lw * 0.5], [w * 0.84, leg + 0.04, lw], l.socks)]);
    }
    let root = Vec3::new(0.0, leg + d * (0.78 - 0.25 * pose.graze), len * 0.42);
    let (rise, nod, neck_len) = carriage(l, pose, root.y);
    let nw = l.neck_width;
    part(root, -rise, pose.look * 0.45, &[([-nw * 0.5, -nw * 0.5, -nw * 0.2], [nw, nw, neck_len + nw * 0.3], l.coat)]);
    let poll = reach_from(root, -rise, pose.look * 0.45, neck_len);
    let [hw, hh, hl] = l.head;
    let mut head: Vec<Piece> = vec![([-hw * 0.5, -hh * 0.5, -hh * 0.2], [hw, hh, hl], l.face)];
    match l.horns {
        Horns::Antlers | Horns::Lyre | Horns::Curl | Horns::Sweep => {
            let span = if l.horns == Horns::Lyre { hw + l.horn } else { hw + l.horn * 0.5 };
            head.push(([-span * 0.5, hh * 0.5, -l.horn * 0.2], [span, l.horn * 0.55, (l.horn * 0.3).max(0.05)], l.horn_colour));
        }
        _ => {}
    }
    part(poll, nod, pose.look, &head);
    if !matches!(l.horns, Horns::Antlers | Horns::Lyre | Horns::Curl | Horns::Sweep) && l.tail > 0.3 {
        part(Vec3::new(0.0, leg + d * 0.88, -len * 0.5), 0.12, pose.tail * 0.5, &[([-0.05, -l.tail, -0.05], [0.1, l.tail, 0.1], l.dark)]);
    }
}

/// How near an animal is drawn whole, and how far it is drawn at all, in metres, by its
/// size: a cow is seen across a field a hen is lost in.
pub fn distances(l: &Look) -> (f32, f32) {
    match l.shape {
        Shape::Fowl | Shape::Duck => (22.0, 90.0),
        Shape::Goose => (28.0, 110.0),
        Shape::Sheep | Shape::Goat | Shape::Pig => (38.0, 180.0),
        Shape::Cattle | Shape::Horse | Shape::Deer => (50.0, 240.0),
    }
}

/// How far over the last stretch of sight an animal dithers out, in metres.
const FADE: f64 = 20.0;

/// Which drawing an animal at `d` voxels from the camera gets: whole, a few boxes, or
/// none at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Detail {
    Near,
    Far,
    Gone,
}

pub fn detail(l: &Look, d: f64) -> Detail {
    let (near, sight) = distances(l);
    if d >= m64(sight as f64) {
        Detail::Gone
    } else if d < m64(near as f64) {
        Detail::Near
    } else {
        Detail::Far
    }
}

/// Draws an animal at `detail`, dithering out near the edge of its sight.
pub fn draw_at(l: &Look, detail: Detail, feet: DVec3, facing: f32, pose: &Pose, cam: DVec3, out: &mut Vec<ParticleInstance>) {
    let from = out.len();
    match detail {
        Detail::Near => draw(l, feet, facing, pose, cam, out),
        Detail::Far => draw_far(l, feet, facing, pose, cam, out),
        Detail::Gone => return,
    }
    let sight = m64(distances(l).1 as f64);
    dither_since(out, from, dither_edge(feet.distance(cam), sight, m64(FADE)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(l: &Look, far: bool, pose: &Pose) -> Vec<ParticleInstance> {
        let mut out = Vec::new();
        if far {
            draw_far(l, DVec3::ZERO, 0.4, pose, DVec3::new(3.0, 2.0, 1.0), &mut out);
        } else {
            draw(l, DVec3::ZERO, 0.4, pose, DVec3::new(3.0, 2.0, 1.0), &mut out);
        }
        assert!(out.iter().all(|b| b.pos.iter().chain(&b.size).all(|v| v.is_finite())), "a box is not where it can be drawn: {}", l.describe());
        assert!(out.iter().all(|b| b.size.iter().all(|&v| v > 0.0)), "a box with no size: {}", l.describe());
        out
    }

    fn poses() -> Vec<Pose> {
        let mut out = vec![Pose::default()];
        for gait in [Gait::Walk, Gait::Trot, Gait::Run] {
            for k in 0..4 {
                out.push(Pose { stride: k as f32 * 1.7, gait, reach: 1.0, look: 0.6, ears: 1.0, tail: -1.0, pitch: 0.2, flap: 1.0, ..Default::default() });
            }
        }
        out.push(Pose { graze: 1.0, ..Default::default() });
        out.push(Pose { alert: 1.0, look: -1.2, ..Default::default() });
        out.push(Pose { afloat: true, ..Default::default() });
        out
    }

    #[test]
    fn every_kind_and_variant_draws_within_its_budget() {
        let mut most = (0, 0);
        for kind in Kind::ALL {
            for seed in 0..60u32 {
                for role in [Role::Female, Role::Male, Role::Young] {
                    let l = Look::new(kind, seed, role);
                    for pose in poses() {
                        let near = count(&l, false, &pose).len();
                        let far = count(&l, true, &pose).len();
                        assert!(near <= NEAR_MOST, "{near} boxes: {}", l.describe());
                        assert!(far <= FAR_MOST && far >= 2, "{far} boxes far: {}", l.describe());
                        most = (most.0.max(near), most.1.max(far));
                    }
                }
            }
        }
        println!("at most {} boxes near and {} far", most.0, most.1);
    }

    #[test]
    fn the_same_seed_makes_the_same_animal_and_others_differ() {
        for kind in Kind::ALL {
            assert_eq!(Look::new(kind, 7, Role::Female), Look::new(kind, 7, Role::Female));
            let herd: Vec<Look> = (0..12).map(|s| Look::new(kind, s, Role::Female)).collect();
            let alike = (0..12).flat_map(|i| (i + 1..12).map(move |j| (i, j))).filter(|&(i, j)| herd[i] == herd[j]).count();
            assert_eq!(alike, 0, "{} look alike", kind.word());
        }
    }

    #[test]
    fn animals_stand_on_their_feet_and_are_their_size() {
        let mut wrong = Vec::new();
        for kind in Kind::ALL {
            let l = Look::new(kind, 3, Role::Female);
            let mut out = Vec::new();
            // The camera is at the feet, so heights are from the ground.
            draw(&l, DVec3::ZERO, 0.4, &Pose::default(), DVec3::ZERO, &mut out);
            let low = out.iter().map(|b| b.pos[1] - b.size[1] * 0.5).fold(f32::MAX, f32::min) / VOXELS_PER_METRE;
            if low.abs() >= 0.06 {
                wrong.push(format!("{} stands {low} m off the ground", kind.word()));
            }
            let high = out.iter().map(|b| b.pos[1] + b.size[1] * 0.5).fold(f32::MIN, f32::max) / VOXELS_PER_METRE;
            let (lo, hi) = match Shape::of(kind) {
                Shape::Fowl | Shape::Duck => (0.25, 0.7),
                Shape::Goose => (0.5, 1.0),
                Shape::Sheep | Shape::Goat | Shape::Pig => (0.6, 1.4),
                _ => (1.0, 2.6),
            };
            if !(lo..hi).contains(&high) {
                wrong.push(format!("{} is {high} m tall", kind.word()));
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("; "));
    }

    #[test]
    fn grazing_brings_the_muzzle_down_to_the_grass() {
        let mut wrong = Vec::new();
        for kind in Kind::ALL {
            let l = Look::new(kind, 5, Role::Female);
            let boxes = |pose: &Pose| {
                let mut out = Vec::new();
                draw(&l, DVec3::ZERO, 0.0, pose, DVec3::ZERO, &mut out);
                out
            };
            let (up, down) = (boxes(&Pose::default()), boxes(&Pose { graze: 1.0, ..Default::default() }));
            assert_eq!(up.len(), down.len());
            // Of the boxes that come down (the head's: the legs stay where they are), the
            // lowest edge, tipped as each box is, is the muzzle's or the bill's.
            let low_edge = |b: &ParticleInstance| (b.pos[1] - b.size[1] * 0.5 * b.tilt[0].cos().abs() - b.size[2] * 0.5 * b.tilt[0].sin().abs()) / VOXELS_PER_METRE;
            let fell: Vec<usize> = (0..up.len()).filter(|&k| (up[k].pos[1] - down[k].pos[1]) / VOXELS_PER_METRE > 0.05).collect();
            if fell.is_empty() {
                wrong.push(format!("{}: the head does not come down", kind.word()));
                continue;
            }
            let low = fell.iter().map(|&k| low_edge(&down[k])).fold(f32::MAX, f32::min);
            if !(-0.03..0.06).contains(&low) {
                wrong.push(format!("{} grazes with its muzzle {low} m up", kind.word()));
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("; "));
    }

    #[test]
    fn far_off_is_a_few_boxes_and_beyond_sight_none() {
        let l = Look::new(Kind::WildCattle, 2, Role::Male);
        let at = |metres: f64| {
            let mut out = Vec::new();
            let feet = DVec3::new(m64(metres), 0.0, 0.0);
            draw_at(&l, detail(&l, feet.length()), feet, 0.0, &Pose::default(), DVec3::ZERO, &mut out);
            out
        };
        assert!(at(10.0).len() > 15);
        assert!((2..=FAR_MOST).contains(&at(120.0).len()));
        assert!(at(235.0).iter().all(|b| b.dissolve > 0.0), "dithering out near the edge of sight");
        assert!(at(260.0).is_empty());
    }
}
