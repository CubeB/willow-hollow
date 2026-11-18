//! The player's own figure (#204): how they look, rolled by the same generator that makes
//! the people of the settlements (`people.rs`) and then changed part by part on the
//! figure screen (`screen.rs`), and kept in `figure.txt` beside the settings.
//!
//! Who the player is goes with them into every world, as the settings do, rather than
//! being kept with each world's save: a plain run begins a new world each time, and a
//! look kept per world would be lost to every new game. What is kept is the look itself,
//! not the seed it was rolled from, so a later change to the generator never changes a
//! figure someone has made.

mod preview;
mod screen;

pub use screen::{Screen, show};

use std::fmt::Write as _;

use glam::DVec3;
use worldgen::{
    names::folk::{self, Sex},
    noise::hash2,
    scale::m64,
    settlements::Region,
};

use crate::{
    character::{self, Beard, Build, Carried, Footwear, Garment, Hair, Headwear, Layer, Look, Rgb, WANDERER},
    people::{self, Age, Person, Trade},
};

/// How tall the figure may be made, in metres, and how broad against the player's body.
/// Both take in everyone the generator makes, children too.
pub const HEIGHT: std::ops::RangeInclusive<f32> = 0.9..=2.05;
pub const GIRTH: std::ops::RangeInclusive<f32> = 0.8..=1.35;
/// The longest name the screen takes.
pub const NAME_LENGTH: usize = 40;

/// From the soles to the hips of the player's own body, in metres: nine of its eighteen
/// cells.
const HIPS: f64 = 0.88;

/// What a roll of a new look is steered by: where the player comes from, and who they are.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Roll {
    pub seed: u32,
    /// The country whose dress and dyes they wear.
    pub region: Region,
    /// The tongue their people speak, which names them: the region's own if None.
    pub tongue: Option<&'static str>,
    pub age: Age,
    /// A woman or a man; either, as the seed has it, if None.
    pub sex: Option<Sex>,
    /// How well off the household they come from, 0 to 1: the brighter the dyes.
    pub wealth: f32,
}

impl Default for Roll {
    fn default() -> Self {
        Roll { seed: 1, region: Region::Temperate, tongue: None, age: Age::Adult, sex: None, wealth: 0.5 }
    }
}

impl Roll {
    /// The roll after this one, steered the same way.
    pub fn next(self) -> Roll {
        Roll { seed: hash2(self.seed, 0x52, 0x6f11), ..self }
    }

    /// A roll as `--figure-look` gives it: a seed, then the country and age if given.
    pub fn from_flag(text: &str) -> anyhow::Result<Roll> {
        let mut words = text.split(',').map(str::trim);
        let seed = words.next().unwrap_or("");
        let mut roll = Roll { seed: seed.parse().map_err(|_| anyhow::anyhow!("--figure-look takes a seed first, not '{seed}'"))?, ..Roll::default() };
        if let Some(w) = words.next() {
            roll.region = Region::ALL.into_iter().find(|r| r.word() == w).ok_or_else(|| anyhow::anyhow!("unknown region {w}"))?;
        }
        if let Some(w) = words.next() {
            roll.age = AGES.iter().map(|a| a.0).find(|a| a.word() == w).ok_or_else(|| anyhow::anyhow!("unknown age {w}: child, adult or elder"))?;
        }
        Ok(roll)
    }

    pub fn tongue(&self) -> &'static str {
        self.tongue.unwrap_or_else(|| folk::tongue_of(self.region))
    }

    /// Whom this roll makes: the generator's person from the first of the seed's own
    /// seeds that gives one of the sex asked for. The same roll makes the same person.
    pub fn person(&self) -> Person {
        let make = |k: i32| Person::in_tongue(hash2(self.seed, k, 0x0f16), self.region, self.tongue(), Trade::None, self.wealth, self.age);
        (0..32).map(make).find(|p| self.sex.is_none_or(|s| p.sex == s)).unwrap_or_else(|| make(0))
    }
}

/// The player as they look.
#[derive(Clone, Debug, PartialEq)]
pub struct Figure {
    /// What the player is called, if they have said: it goes on the journal.
    pub name: String,
    pub look: Look,
    /// The hair's own colour before it greys, and how grey it has gone (0 to 1, white),
    /// which together make the look's hair.
    pub natural: Rgb,
    pub grey: f32,
    /// How the last roll was steered, so the screen opens as it was left.
    pub roll: Roll,
}

impl Default for Figure {
    /// The wanderer, as everyone begins.
    fn default() -> Self {
        Figure { name: String::new(), look: WANDERER, natural: WANDERER.hair, grey: 0.0, roll: Roll::default() }
    }
}

/// A look as the player wears it: the pack on their back, with the glider beside it, and
/// nothing in their hands, which are for the lantern, the glider's bar and what is held.
fn worn(look: Look) -> Look {
    Look { pack: true, carried: Carried::None, ..look }
}

impl Figure {
    /// The player as `roll` makes them.
    pub fn rolled(roll: &Roll) -> Figure {
        let p = roll.person();
        Figure { name: p.name.clone(), look: worn(p.look), natural: people::HAIR[p.hair].1, grey: p.grey, roll: *roll }
    }

    /// Becomes what `roll` makes, keeping the name the player gave (the name the roll
    /// would give is returned, for them to take if they like), and the height and build
    /// if `keep_build`.
    pub fn reroll(&mut self, roll: &Roll, keep_build: bool) -> String {
        let mut next = Figure::rolled(roll);
        let offered = std::mem::replace(&mut next.name, std::mem::take(&mut self.name));
        if keep_build {
            let b = next.look.build;
            next.look.build = Build::standing(self.look.build.metres(), self.look.build.girth, b.head, b.stoop);
        }
        *self = next;
        offered
    }

    /// Changes one part of the look, as the screen does. Whatever the choice, the look is
    /// one the figure can be drawn in: an index beyond its palette changes nothing.
    pub fn choose(&mut self, choice: Choice) {
        let l = &mut self.look;
        let cloth = |k: usize| people::CLOTH.get(k).map(|c| c.1);
        let set = |slot: &mut Rgb, c: Option<Rgb>| {
            if let Some(c) = c {
                *slot = c;
            }
        };
        match choice {
            Choice::Height(metres) => {
                let b = l.build;
                l.build = Build::standing(metres.clamp(*HEIGHT.start(), *HEIGHT.end()), b.girth, b.head, b.stoop);
            }
            Choice::Girth(g) => l.build.girth = g.clamp(*GIRTH.start(), *GIRTH.end()),
            Choice::Skin(k) => set(&mut l.skin, people::SKIN.get(k).copied()),
            Choice::Cut(cut) => l.cut = cut,
            Choice::HairColour(k) => {
                if let Some(&(_, c)) = people::HAIR.get(k) {
                    self.natural = c;
                    l.hair = Person::greyed(c, self.grey);
                }
            }
            Choice::Grey(g) => {
                self.grey = g.clamp(0.0, 1.0);
                l.hair = Person::greyed(self.natural, self.grey);
            }
            Choice::Beard(beard) => l.beard = beard,
            Choice::Garment(g) => l.garment = g,
            Choice::Top(k) => set(&mut l.top, cloth(k)),
            Choice::Bottom(k) => set(&mut l.bottom, cloth(k)),
            Choice::Layer(layer) => l.layer = layer,
            Choice::Over(k) => set(&mut l.over, cloth(k)),
            Choice::Scarf(on) => l.scarf = on,
            Choice::Accent(k) => set(&mut l.accent, cloth(k)),
            Choice::Rolled(on) => l.rolled = on,
            Choice::Belt(k) => set(&mut l.belt, belts().get(k).map(|c| c.1)),
            Choice::Headwear(h) => {
                // A straw hat is straw and a fur hat fur, until another colour is chosen;
                // a cloth one taken from either is the colour of what is worn over.
                let straw = l.hat == people::STRAW;
                let fur = people::FURS.contains(&l.hat);
                match h {
                    Headwear::StrawHat | Headwear::Veil if !straw => l.hat = people::STRAW,
                    Headwear::FurHat if !fur => l.hat = people::FURS[0],
                    Headwear::None | Headwear::StrawHat | Headwear::Veil | Headwear::FurHat => {}
                    _ if straw || fur => l.hat = l.over,
                    _ => {}
                }
                l.headwear = h;
            }
            Choice::Hat(k) => set(&mut l.hat, hats().get(k).map(|c| c.1)),
            Choice::Footwear(f) => l.footwear = f,
            Choice::Boots(k) => set(&mut l.boots, LEATHERS.get(k).map(|c| c.1)),
        }
    }

    /// The figure as a few lines of `key = value`, for `figure.txt`.
    pub fn to_text(&self) -> String {
        let l = &self.look;
        let b = l.build;
        let hex = |c: Rgb| format!("{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
        let r = &self.roll;
        let mut t = String::from("# How the player looks (#204), as the figure screen left it. A line the game\n# cannot read is the wanderer's own.\n");
        let name: String = self.name.chars().filter(|c| !c.is_control()).collect();
        let _ = writeln!(t, "name = {}", name.trim());
        let _ = writeln!(t, "build = {} {} {} {}", b.scale, b.girth, b.head, b.stoop);
        for (key, c) in [("skin", l.skin), ("hair", l.hair), ("natural", self.natural), ("top", l.top), ("bottom", l.bottom), ("boots", l.boots)] {
            let _ = writeln!(t, "{key} = {}", hex(c));
        }
        for (key, c) in [("accent", l.accent), ("over", l.over), ("hat", l.hat), ("belt", l.belt)] {
            let _ = writeln!(t, "{key} = {}", hex(c));
        }
        let _ = writeln!(t, "grey = {}", self.grey);
        let _ = writeln!(t, "cut = {}", word(l.cut));
        let _ = writeln!(t, "beard = {}", word(l.beard));
        let _ = writeln!(t, "garment = {}", word(l.garment));
        let _ = writeln!(t, "layer = {}", word(l.layer));
        let _ = writeln!(t, "headwear = {}", word(l.headwear));
        let _ = writeln!(t, "footwear = {}", word(l.footwear));
        let _ = writeln!(t, "scarf = {}", l.scarf);
        let _ = writeln!(t, "rolled = {}", l.rolled);
        let _ = writeln!(t, "roll = {} {} {} {} {} {}", r.seed, r.region.word(), r.tongue.unwrap_or("-"), r.age.word(), sex_word(r.sex), r.wealth);
        t
    }

    /// Reads a figure back, keeping the wanderer's own for anything missing or that
    /// cannot be read, so a damaged file never stops the game: an empty one is the
    /// wanderer.
    pub fn from_text(text: &str) -> Figure {
        let mut f = Figure::default();
        let mut natural = None;
        for line in text.lines().map(str::trim).filter(|l| !l.starts_with('#')) {
            let Some((key, value)) = line.split_once('=') else { continue };
            let value = value.trim();
            let l = &mut f.look;
            let colour = |slot: &mut Rgb| {
                if let Some(c) = rgb(value) {
                    *slot = c;
                }
            };
            match key.trim() {
                "name" => f.name = value.chars().take(NAME_LENGTH).collect(),
                "build" => {
                    let v: Vec<f32> = value.split_whitespace().filter_map(|w| w.parse().ok()).filter(|v: &f32| v.is_finite()).collect();
                    if let [scale, girth, head, stoop] = v[..] {
                        // Held to what the screen could have made, for a hand-edited file.
                        let b = Build { scale: 1.0, girth: girth.clamp(*GIRTH.start(), *GIRTH.end()), head: head.clamp(0.9, 1.4), stoop: stoop.clamp(0.0, 0.3) };
                        let tall = Build { scale, ..b }.metres();
                        l.build = if HEIGHT.contains(&tall) { Build { scale, ..b } } else { Build::standing(tall.clamp(*HEIGHT.start(), *HEIGHT.end()), b.girth, b.head, b.stoop) };
                    }
                }
                "skin" => colour(&mut l.skin),
                "hair" => colour(&mut l.hair),
                "natural" => natural = rgb(value),
                "top" => colour(&mut l.top),
                "bottom" => colour(&mut l.bottom),
                "boots" => colour(&mut l.boots),
                "accent" => colour(&mut l.accent),
                "over" => colour(&mut l.over),
                "hat" => colour(&mut l.hat),
                "belt" => colour(&mut l.belt),
                "grey" => f.grey = value.parse::<f32>().ok().filter(|g| g.is_finite()).map_or(f.grey, |g| g.clamp(0.0, 1.0)),
                "cut" => l.cut = named(&CUTS, value).unwrap_or(l.cut),
                "beard" => l.beard = named(&BEARDS, value).unwrap_or(l.beard),
                "garment" => l.garment = named(&GARMENTS, value).unwrap_or(l.garment),
                "layer" => l.layer = named(&LAYERS, value).unwrap_or(l.layer),
                "headwear" => l.headwear = named(&HEADWEAR, value).unwrap_or(l.headwear),
                "footwear" => l.footwear = named(&FOOTWEAR, value).unwrap_or(l.footwear),
                "scarf" => l.scarf = value.parse().unwrap_or(l.scarf),
                "rolled" => l.rolled = value.parse().unwrap_or(l.rolled),
                "roll" => f.roll = roll_from(value).unwrap_or(f.roll),
                _ => {}
            }
        }
        // A file without the hair's own colour has it as the hair shows.
        f.natural = natural.unwrap_or(f.look.hair);
        f
    }
}

fn rgb(hex: &str) -> Option<Rgb> {
    let n = u32::from_str_radix(hex.trim_start_matches('#'), 16).ok().filter(|_| hex.trim_start_matches('#').len() == 6)?;
    Some([(n >> 16) as u8, (n >> 8) as u8, n as u8])
}

fn roll_from(value: &str) -> Option<Roll> {
    let w: Vec<&str> = value.split_whitespace().collect();
    let [seed, region, tongue, age, sex, wealth] = w[..] else { return None };
    Some(Roll {
        seed: seed.parse().ok()?,
        region: Region::ALL.into_iter().find(|r| r.word() == region)?,
        tongue: folk::tongues().into_iter().find(|t| *t == tongue),
        age: AGES.iter().find(|a| a.0.word() == age)?.0,
        sex: SEXES.iter().map(|s| s.0).find(|s| sex_word(*s) == sex)?,
        wealth: wealth.parse::<f32>().ok().filter(|w| w.is_finite())?.clamp(0.0, 1.0),
    })
}

fn sex_word(sex: Option<Sex>) -> &'static str {
    match sex {
        Some(Sex::Female) => "woman",
        Some(Sex::Male) => "man",
        None => "either",
    }
}

/// How a kind of thing is written in the file: its name in the code, in lower case, so
/// that the words the screen shows can change without a saved figure changing.
fn word(what: impl std::fmt::Debug) -> String {
    format!("{what:?}").to_lowercase()
}

fn named<T: Copy + std::fmt::Debug>(list: &[(T, &str)], value: &str) -> Option<T> {
    list.iter().map(|e| e.0).find(|t| word(t) == value)
}

/// Each part's kinds, with what the screen calls them.
pub const CUTS: [(Hair, &str); 9] = [
    (Hair::Cropped, "cropped"),
    (Hair::Short, "short"),
    (Hair::Shoulder, "to the shoulder"),
    (Hair::Long, "long"),
    (Hair::Plaited, "plaited"),
    (Hair::Bun, "in a bun"),
    (Hair::TopKnot, "a top-knot"),
    (Hair::Shaved, "shaved"),
    (Hair::Receding, "receding"),
];
pub const BEARDS: [(Beard, &str); 4] = [(Beard::None, "none"), (Beard::Moustache, "a moustache"), (Beard::Short, "short"), (Beard::Full, "full")];
pub const GARMENTS: [(Garment, &str); 4] = [(Garment::Tunic, "tunic and trousers"), (Garment::Smock, "smock"), (Garment::Dress, "dress"), (Garment::Robe, "robe")];
pub const LAYERS: [(Layer, &str); 6] = [
    (Layer::None, "nothing over"),
    (Layer::Apron, "an apron"),
    (Layer::Bib, "a bib apron"),
    (Layer::Waistcoat, "a waistcoat"),
    (Layer::Cloak, "a cloak"),
    (Layer::Shawl, "a shawl"),
];
pub const HEADWEAR: [(Headwear, &str); 9] = [
    (Headwear::None, "bareheaded"),
    (Headwear::StrawHat, "a straw hat"),
    (Headwear::Cap, "a cap"),
    (Headwear::Hood, "a hood"),
    (Headwear::Headscarf, "a headscarf"),
    (Headwear::Wrap, "a head wrap"),
    (Headwear::FurHat, "a fur hat"),
    (Headwear::Veil, "a beekeeper's veil"),
    (Headwear::Souwester, "a sou'wester"),
];
pub const FOOTWEAR: [(Footwear, &str); 4] = [(Footwear::Boots, "boots"), (Footwear::Shoes, "shoes"), (Footwear::Sandals, "sandals"), (Footwear::Bare, "barefoot")];
pub const AGES: [(Age, &str); 3] = [(Age::Child, "a child"), (Age::Adult, "grown"), (Age::Elder, "an elder")];
pub const SEXES: [(Option<Sex>, &str); 3] = [(None, "either"), (Some(Sex::Female), "a woman"), (Some(Sex::Male), "a man")];

/// What the screen calls each region: where the player comes from.
pub fn country(region: Region) -> &'static str {
    match region {
        Region::Temperate => "the lowlands",
        Region::Upland => "the uplands",
        Region::Northern => "the north",
        Region::Alpine => "the mountains",
        Region::Desert => "the desert",
        Region::Wetland => "the marshes",
        Region::Rainforest => "the rainforest",
        Region::OldGrowth => "the old forests",
    }
}

/// Leathers, for boots and shoes.
pub const LEATHERS: [(&str, Rgb); 4] =
    [("dark leather", people::LEATHERS[0]), ("brown leather", people::LEATHERS[1]), ("tan leather", people::LEATHERS[2]), ("saddle leather", people::LEATHERS[3])];

/// A belt of leather, or a sash of any cloth.
pub fn belts() -> Vec<(&'static str, Rgb)> {
    LEATHERS.iter().copied().chain(people::CLOTH).collect()
}

/// A hat of any cloth, of straw, or of fur.
pub fn hats() -> Vec<(&'static str, Rgb)> {
    let furs = [("brown fur", people::FURS[0]), ("grey fur", people::FURS[1]), ("dark fur", people::FURS[2])];
    people::CLOTH.into_iter().chain([("straw", people::STRAW)]).chain(furs).collect()
}

/// One change to one part of the look: everything the screen offers is one of these.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Choice {
    /// Metres from the soles to the crown, within [`HEIGHT`].
    Height(f32),
    /// Breadth against the player's body, within [`GIRTH`].
    Girth(f32),
    /// A tone of [`people::SKIN`].
    Skin(usize),
    Cut(Hair),
    /// A colour of [`people::HAIR`], before it greys.
    HairColour(usize),
    /// How grey, 0 to 1.
    Grey(f32),
    Beard(Beard),
    Garment(Garment),
    /// The garment's cloth, the trousers' or skirt's, what is over it, and the scarf's,
    /// each of [`people::CLOTH`].
    Top(usize),
    Bottom(usize),
    Layer(Layer),
    Over(usize),
    Scarf(bool),
    Accent(usize),
    /// Sleeves rolled to the elbow.
    Rolled(bool),
    /// Of [`belts`].
    Belt(usize),
    Headwear(Headwear),
    /// Of [`hats`].
    Hat(usize),
    Footwear(Footwear),
    /// Of [`LEATHERS`].
    Boots(usize),
}

/// Every choice the screen offers, the sliders at their ends and middles.
#[cfg(test)]
pub fn every_choice() -> Vec<Choice> {
    let mut all = Vec::new();
    let (h, g) = ((*HEIGHT.start(), *HEIGHT.end()), (*GIRTH.start(), *GIRTH.end()));
    all.extend([h.0, (h.0 + h.1) * 0.5, h.1].map(Choice::Height));
    all.extend([g.0, (g.0 + g.1) * 0.5, g.1].map(Choice::Girth));
    all.extend((0..people::SKIN.len()).map(Choice::Skin));
    all.extend(CUTS.map(|c| Choice::Cut(c.0)));
    all.extend((0..people::HAIR.len()).map(Choice::HairColour));
    all.extend([0.0, 0.5, 1.0].map(Choice::Grey));
    all.extend(BEARDS.map(|c| Choice::Beard(c.0)));
    all.extend(GARMENTS.map(|c| Choice::Garment(c.0)));
    for k in 0..people::CLOTH.len() {
        all.extend([Choice::Top(k), Choice::Bottom(k), Choice::Over(k), Choice::Accent(k)]);
    }
    all.extend(LAYERS.map(|c| Choice::Layer(c.0)));
    all.extend([Choice::Scarf(false), Choice::Scarf(true), Choice::Rolled(false), Choice::Rolled(true)]);
    all.extend((0..belts().len()).map(Choice::Belt));
    all.extend(HEADWEAR.map(|c| Choice::Headwear(c.0)));
    all.extend((0..hats().len()).map(Choice::Hat));
    all.extend(FOOTWEAR.map(|c| Choice::Footwear(c.0)));
    all.extend((0..LEATHERS.len()).map(Choice::Boots));
    all
}

/// Where to stand the player's figure so that it meets what the player's own body does:
/// its hands on the glider's bar as it hangs from it (`hanging`, 0 to 1 as the wing
/// opens), its hips on the thwart when `seated` in the boat. The eye, and the body the
/// world knows, are the player's own whatever the figure looks like; only a figure taller
/// or shorter than the wanderer is moved.
pub fn fitted(look: &Look, feet: DVec3, facing: f32, hanging: f32, seated: bool) -> DVec3 {
    let scale = look.build.scale;
    if scale == 1.0 {
        return feet;
    }
    let short = (1.0 - scale) as f64;
    if seated {
        return feet + DVec3::Y * m64(HIPS) * short;
    }
    if hanging > 0.0 {
        let (ahead, up) = character::grip();
        let (s, c) = facing.sin_cos();
        return feet + (DVec3::new(c as f64, 0.0, s as f64) * ahead as f64 + DVec3::Y * up as f64) * short * hanging.min(1.0) as f64;
    }
    feet
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rolls() -> Vec<Roll> {
        let mut out = Vec::new();
        for (k, region) in Region::ALL.into_iter().enumerate() {
            for (age, _) in AGES {
                out.push(Roll { seed: 7 + k as u32, region, age, wealth: k as f32 / 7.0, ..Roll::default() });
            }
        }
        out
    }

    #[test]
    fn the_wanderer_is_the_default_and_an_empty_file_reads_as_the_wanderer() {
        assert_eq!(Figure::default().look, WANDERER);
        assert_eq!(Figure::from_text("").look, WANDERER);
        assert_eq!(Figure::from_text(""), Figure::default());
        assert_eq!(Figure::from_text(&Figure::default().to_text()), Figure::default());
    }

    #[test]
    fn a_figure_survives_being_written_and_read_back() {
        for roll in rolls() {
            let mut f = Figure::rolled(&roll);
            assert_eq!(Figure::from_text(&f.to_text()), f, "{}", f.to_text());
            f.name = "Tam of the Ford".into();
            f.choose(Choice::Height(1.83));
            f.choose(Choice::Grey(0.37));
            f.choose(Choice::Headwear(Headwear::FurHat));
            assert_eq!(Figure::from_text(&f.to_text()), f, "{}", f.to_text());
        }
        // Steered every way, the roll comes back as it went.
        for tongue in folk::tongues() {
            for (sex, _) in SEXES {
                let f = Figure::rolled(&Roll { tongue: Some(tongue), sex, ..Roll::default() });
                assert_eq!(Figure::from_text(&f.to_text()).roll, f.roll);
            }
        }
    }

    #[test]
    fn a_damaged_file_keeps_the_wanderers_own_for_what_it_cannot_read() {
        let f = Figure::from_text("build = 4 1 1\nskin = 12345g\ncut = mohican\nroll = 1 nowhere\nscarf = perhaps\ngrey = NaN\nnonsense\nhair = 1e1e1e");
        let w = Figure::default();
        assert_eq!((f.look.build, f.look.skin, f.look.cut, f.roll, f.look.scarf, f.grey), (w.look.build, w.look.skin, w.look.cut, w.roll, w.look.scarf, w.grey));
        assert_eq!(f.look.hair, [0x1e; 3]);
        // A figure taller than any the screen makes is held to the tallest.
        let tall = Figure::from_text("build = 3 1 1 0");
        assert!((tall.look.build.metres() - HEIGHT.end()).abs() < 1e-4, "{}", tall.look.build.metres());
    }

    #[test]
    fn a_roll_is_the_same_from_the_same_seed_and_steered_as_asked() {
        for roll in rolls() {
            assert_eq!(Figure::rolled(&roll), Figure::rolled(&roll));
            assert_ne!(Figure::rolled(&roll).look, Figure::rolled(&roll.next()).look);
            let p = roll.person();
            assert_eq!((p.age, p.region), (roll.age, roll.region));
            // The player's own pack, and their hands free.
            let f = Figure::rolled(&roll);
            assert!(f.look.pack && f.look.carried == Carried::None);
            assert_eq!(f.look.hair, Person::greyed(f.natural, f.grey));
        }
        for (sex, _) in SEXES.into_iter().skip(1) {
            let mut roll = Roll { sex, ..Roll::default() };
            for _ in 0..20 {
                assert_eq!(Some(roll.person().sex), sex);
                roll = roll.next();
            }
        }
        // The tongue names them.
        let named = |tongue| Figure::rolled(&Roll { tongue: Some(tongue), ..Roll::default() }).name;
        assert_ne!(named("north"), named("dune"));
    }

    #[test]
    fn a_reroll_keeps_the_name_given_and_the_build_if_asked() {
        let mut f = Figure::rolled(&Roll::default());
        f.name = "Marren".into();
        f.choose(Choice::Height(1.95));
        f.choose(Choice::Girth(1.2));
        let offered = f.reroll(&Roll::default().next(), true);
        assert_eq!(f.name, "Marren");
        assert_eq!(offered, Roll::default().next().person().name);
        assert!((f.look.build.metres() - 1.95).abs() < 1e-4 && f.look.build.girth == 1.2);
        f.reroll(&Roll::default().next().next(), false);
        assert!((f.look.build.metres() - 1.95).abs() > 1e-3, "the roll's own height");
    }

    /// The boxes of a look, standing and at full stride, near and far.
    fn drawn(look: &Look) -> (usize, usize, f32) {
        let pose = character::Gait::striding(0.3, 4.0).pose();
        let mut near = Vec::new();
        character::draw(look, DVec3::ZERO, 0.4, &pose, Some(0.0), DVec3::new(3.0, 2.0, 1.0), &mut near);
        let mut standing = Vec::new();
        character::draw(look, DVec3::ZERO, 0.4, &Default::default(), Some(0.0), DVec3::ZERO, &mut standing);
        let mut far = Vec::new();
        character::draw_far(look, DVec3::ZERO, 0.4, &pose, DVec3::new(3.0, 2.0, 1.0), &mut far);
        for b in near.iter().chain(&far).chain(&standing) {
            assert!(b.pos.iter().chain(&b.size).chain(&b.color).all(|v| v.is_finite()), "{look:?}");
        }
        let low = standing.iter().map(|b| b.pos[1] - b.size[1] * 0.5).fold(f32::MAX, f32::min);
        (near.len(), far.len(), low)
    }

    #[test]
    fn every_choice_on_the_screen_makes_a_look_that_can_be_drawn() {
        let mut bases = vec![Figure::default()];
        bases.extend(rolls().iter().step_by(3).map(Figure::rolled));
        let mut most = 0;
        for base in &bases {
            for choice in every_choice() {
                let mut f = base.clone();
                f.choose(choice);
                let (near, far, low) = drawn(&f.look);
                most = most.max(near);
                // A villager's budget, and the four boxes of the furled glider.
                assert!(near <= 49 && far <= 8, "{choice:?}: {near} near, {far} far");
                assert!(low.abs() < 1e-3, "{choice:?}: the feet are at {low}");
                assert!(f.look.pack && f.look.carried == Carried::None);
                let tall = f.look.build.metres();
                assert!(tall > HEIGHT.start() - 1e-3 && tall < HEIGHT.end() + 1e-3, "{choice:?}: {tall} m");
                assert_eq!(Figure::from_text(&f.to_text()), f, "{choice:?} does not survive saving");
            }
        }
        // And every kind of every part is among them.
        let all = every_choice();
        assert!(Hair::ALL.iter().all(|h| all.contains(&Choice::Cut(*h))));
        assert!(Beard::ALL.iter().all(|h| all.contains(&Choice::Beard(*h))));
        assert!(Garment::ALL.iter().all(|h| all.contains(&Choice::Garment(*h))));
        assert!(Layer::ALL.iter().all(|h| all.contains(&Choice::Layer(*h))));
        assert!(Headwear::ALL.iter().all(|h| all.contains(&Choice::Headwear(*h))));
        assert!(Footwear::ALL.iter().all(|h| all.contains(&Choice::Footwear(*h))));
        println!("{} choices on {} figures, at most {most} boxes", all.len(), bases.len());
    }

    #[test]
    fn a_choice_beyond_its_palette_changes_nothing() {
        let f = Figure::rolled(&Roll::default());
        for choice in [Choice::Skin(99), Choice::HairColour(99), Choice::Top(99), Choice::Belt(99), Choice::Hat(99), Choice::Boots(99)] {
            let mut g = f.clone();
            g.choose(choice);
            assert_eq!(g, f, "{choice:?}");
        }
    }

    #[test]
    fn a_figure_of_another_height_holds_the_glider_and_sits_on_the_thwart() {
        let mut f = Figure::default();
        f.choose(Choice::Height(1.55));
        let facing = 0.7;
        let hands = |look: &Look, feet: DVec3| {
            let mut out = Vec::new();
            character::draw(look, feet, facing, &character::Gait::gliding(facing).pose(), None, DVec3::ZERO, &mut out);
            // The two highest boxes of skin are the hands, up on the bar.
            let l = |v: u8| (v as f32 / 255.0).powf(2.2);
            let skin = [l(look.skin[0]), l(look.skin[1]), l(look.skin[2]), 1.0];
            let mut bare: Vec<glam::Vec3> = out.iter().filter(|b| b.color == skin).map(|b| glam::Vec3::from(b.pos)).collect();
            bare.sort_by(|a, b| b.y.total_cmp(&a.y));
            (bare[0] + bare[1]) * 0.5
        };
        let wanderer = hands(&WANDERER, DVec3::ZERO);
        let shorter = hands(&f.look, fitted(&f.look, DVec3::ZERO, facing, 1.0, false));
        assert!(wanderer.distance(shorter) < 0.05, "the hands at {shorter}, the bar at {wanderer}");
        // The wanderer stands where they always have.
        assert_eq!(fitted(&WANDERER, DVec3::new(1.0, 2.0, 3.0), facing, 1.0, true), DVec3::new(1.0, 2.0, 3.0));
        assert!(fitted(&f.look, DVec3::ZERO, facing, 0.0, true).y > 0.0, "a shorter figure is raised to the thwart");
    }
}
