//! The house kit's rules: one text file per region under `assets/structures/kit/`, read
//! here. The format is described in the kit's own documentation ([`super`]).

use std::sync::OnceLock;

use super::{AnchorKind, Step};
use crate::settlements::Region;

/// Defines a kind of choice in the files: an enum whose arms each have a word.
macro_rules! words {
    ($(#[$m:meta])* $name:ident { $($(#[$vm:meta])* $v:ident = $w:literal),* $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        pub enum $name { $($(#[$vm])* $v),* }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$v),*];

            /// Its word in a kit file.
            pub fn word(self) -> &'static str {
                match self { $($name::$v => $w),* }
            }

            /// The option a kit file's word names.
            pub fn by_word(w: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|x| x.word().eq_ignore_ascii_case(w))
            }
        }
    };
}

words! {
    /// The shape of a building seen from above.
    Form {
        Rect = "rect",
        /// A main block with a wing at its back, under a roof of two ridges.
        L = "l",
        Round = "round",
    }
}

words! {
    /// What the walls are made of, and how.
    WallKind {
        /// Earth and straw built up in lifts, thick, with soft corners.
        Cob = "cob",
        /// Panels of daub on woven rods between posts.
        Wattle = "wattle",
        /// Stone laid without mortar: the place's rock, rough and mossy where it is damp.
        Drystone = "drystone",
        /// Logs laid one on another, their ends crossing at the corners.
        Log = "log",
        /// A frame of posts, rails and plates, its panels filled and limewashed.
        Frame = "frame",
        /// Coursed stone of the place's rock, with darker quoins at the corners.
        Stone = "stone",
        /// Sun-dried brick under a render of mud.
        Mudbrick = "mudbrick",
        /// Upright boards between posts.
        Plank = "plank",
        /// Panels of bound reed between posts.
        Reed = "reed",
    }
}

words! {
    /// What a roof is covered with.
    Cover {
        Thatch = "thatch",
        Reed = "reed",
        Palm = "palm",
        Turf = "turf",
        Slate = "slate",
        Tile = "tile",
        Shingle = "shingle",
        /// Packed earth on a flat roof.
        Mud = "mud",
    }
}

words! {
    /// The shape of a roof.
    RoofShape {
        /// Two slopes meeting at a ridge, with a gable wall at each end.
        Gable = "gable",
        /// Sloping on every side.
        Hip = "hip",
        /// A cone over a round building.
        Cone = "cone",
        /// Flat, with a parapet round it.
        Flat = "flat",
    }
}

words! {
    /// What a building's floor is.
    FloorKind {
        Earth = "earth",
        Flags = "flags",
        Boards = "boards",
    }
}

words! {
    /// What holds a raised floor up.
    Under {
        /// Posts, with the ground open beneath.
        Stilts = "stilts",
        /// A solid podium of stone.
        Plinth = "plinth",
        /// A podium of great roots, as the old forest's halls stand on.
        Roots = "roots",
    }
}

words! {
    /// Where the smoke of the fire goes out.
    Smoke {
        /// A hearth in the middle of the floor, the smoke out through a capped hole at the
        /// top of the roof.
        Hole = "hole",
        /// A fireplace built into a wall, with a chimney stack.
        Chimney = "chimney",
    }
}

/// Which way wealth tips a weight or a chance.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bias {
    None,
    /// Commoner the richer the household: nothing at the poorest, twice as common at the
    /// richest.
    Rich,
    /// The other way about.
    Poor,
}

impl Bias {
    /// How much a weight or a chance counts for at `wealth`, 0 poorest to 1 richest.
    pub fn scale(self, wealth: f32) -> f32 {
        match self {
            Bias::None => 1.0,
            Bias::Rich => 2.0 * wealth,
            Bias::Poor => 2.0 * (1.0 - wealth),
        }
    }
}

/// One option of a weighted list.
#[derive(Clone, PartialEq, Debug)]
pub struct Choice<T> {
    pub what: T,
    pub weight: f32,
    pub bias: Bias,
}

/// The option `roll` (0..1) falls on, each weighted as wealth tips it. When wealth leaves
/// nothing any weight, the first option.
pub fn choose<T>(list: &[Choice<T>], wealth: f32, roll: f32) -> Option<&T> {
    let weight = |c: &Choice<T>| (c.weight * c.bias.scale(wealth)).max(0.0);
    let total: f32 = list.iter().map(weight).sum();
    if total <= 0.0 {
        return list.first().map(|c| &c.what);
    }
    let mut r = roll.clamp(0.0, 0.999_999) * total;
    for c in list {
        if r < weight(c) {
            return Some(&c.what);
        }
        r -= weight(c);
    }
    list.last().map(|c| &c.what)
}

/// How likely something is, 0 to 1, as wealth tips it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Chance {
    pub p: f32,
    pub bias: Bias,
}

impl Chance {
    pub const NEVER: Chance = Chance { p: 0.0, bias: Bias::None };

    pub fn at(self, wealth: f32) -> f32 {
        (self.p * self.bias.scale(wealth)).clamp(0.0, 1.0)
    }
}

/// A range of numbers, lowest to highest.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Span {
    pub lo: f32,
    pub hi: f32,
}

impl Span {
    pub const fn one(v: f32) -> Span {
        Span { lo: v, hi: v }
    }

    /// The value `t` (0..1) of the way along.
    pub fn at(self, t: f32) -> f32 {
        self.lo + (self.hi - self.lo) * t.clamp(0.0, 1.0)
    }

    /// A whole number `t` of the way along, rounded.
    pub fn whole(self, t: f32) -> i32 {
        self.at(t).round() as i32
    }
}

/// How one step (hut, timber house or stone house) is built in a region. Sizes are in
/// voxels (0.4 m); where a range is given, wealth chooses along it for sizes and the seed
/// for the rest.
#[derive(Clone, PartialEq, Debug)]
pub struct StepRules {
    pub footprint: Vec<Choice<Form>>,
    /// The short side outside the walls, or a round building's width.
    pub width: Span,
    /// The long side outside the walls.
    pub length: Span,
    /// How thick the walls are.
    pub thick: Span,
    pub walls: Vec<Choice<WallKind>>,
    /// Courses of stone at the foot of the walls, above the floor.
    pub base: Span,
    /// From the floor to the top of the walls, at least 5 so a person can stand at a wall.
    pub wall_height: Span,
    pub roof: Vec<Choice<Cover>>,
    pub roof_shape: Vec<Choice<RoofShape>>,
    /// Rise over run: 1 is 45 degrees.
    pub pitch: Span,
    /// How far the roof reaches out past the walls.
    pub overhang: Span,
    /// The chance the door is in a gable end, under the ridge, rather than in a long side.
    pub gable_door: Chance,
    pub floor: Vec<Choice<FloorKind>>,
    /// How far the floor is above the ground.
    pub raised: Span,
    pub under: Vec<Choice<Under>>,
    /// How deep the platform before the door is (a doorstep at least).
    pub veranda: Span,
    pub smoke: Vec<Choice<Smoke>>,
    pub lean_to: Chance,
    pub windows: Span,
    pub door_width: Span,
    pub door_height: Span,
    /// The chance the walls are limewashed white.
    pub whitewash: Chance,
    /// The chance of rows of stones on battens holding a roof down.
    pub roof_stones: Chance,
    /// The chance of a walled yard before the door, its gate the way in.
    pub courtyard: Chance,
    /// How high a flat roof's parapet stands.
    pub parapet: Span,
    /// The chance a rectangular building's corners are cut off, as cob and drystone
    /// corners are rounded.
    pub round_corners: Chance,
}

impl Default for StepRules {
    /// What a file leaves out: a plain timber cottage.
    fn default() -> Self {
        fn one<T>(what: T) -> Vec<Choice<T>> {
            vec![Choice { what, weight: 1.0, bias: Bias::None }]
        }
        StepRules {
            footprint: one(Form::Rect),
            width: Span { lo: 12.0, hi: 15.0 },
            length: Span { lo: 15.0, hi: 20.0 },
            thick: Span::one(1.0),
            walls: one(WallKind::Plank),
            base: Span::one(1.0),
            wall_height: Span { lo: 6.0, hi: 7.0 },
            roof: one(Cover::Thatch),
            roof_shape: one(RoofShape::Gable),
            pitch: Span { lo: 1.0, hi: 1.2 },
            overhang: Span { lo: 1.0, hi: 2.0 },
            gable_door: Chance::NEVER,
            floor: one(FloorKind::Boards),
            raised: Span::one(0.0),
            under: one(Under::Stilts),
            veranda: Span::one(1.0),
            smoke: one(Smoke::Hole),
            lean_to: Chance::NEVER,
            windows: Span { lo: 1.0, hi: 3.0 },
            door_width: Span::one(3.0),
            door_height: Span::one(5.0),
            whitewash: Chance::NEVER,
            roof_stones: Chance::NEVER,
            courtyard: Chance::NEVER,
            parapet: Span::one(1.0),
            round_corners: Chance::NEVER,
        }
    }
}

/// A region's whole kit: its three steps, and the components its buildings take at their
/// anchors.
#[derive(Clone, PartialEq, Debug)]
pub struct Rules {
    pub region: Region,
    /// A line about the region's way of building.
    pub about: String,
    /// The hut, the timber house and the stone house, in [`Step::ALL`] order.
    pub steps: [StepRules; 3],
    /// For each kind of anchor, the components that can go there and how often, by name
    /// (`nothing` for none): each a [`super::components::Component`] that goes at that
    /// kind of anchor.
    pub components: Vec<(AnchorKind, Vec<Choice<String>>)>,
}

impl Rules {
    pub fn step(&self, step: Step) -> &StepRules {
        &self.steps[step as usize]
    }

    /// The components that can go at an anchor of this kind, and their weights.
    pub fn components(&self, kind: AnchorKind) -> &[Choice<String>] {
        self.components.iter().find(|(k, _)| *k == kind).map_or(&[], |(_, c)| c.as_slice())
    }

    fn fallback(region: Region) -> Self {
        Rules { region, about: String::new(), steps: [StepRules::default(), StepRules::default(), StepRules::default()], components: Vec::new() }
    }

    /// Reads a region's kit file. The error says which line is wrong and why.
    pub fn parse(region: Region, text: &str) -> Result<Rules, String> {
        enum Section {
            Head,
            Step(usize),
            Components,
        }
        let mut rules = Rules::fallback(region);
        let mut seen = [false; 3];
        let mut section = Section::Head;
        for (n, raw) in text.lines().enumerate() {
            let at = |msg: String| format!("line {}: {msg}", n + 1);
            let line = raw.split('#').next().unwrap_or("").trim_end();
            if line.trim().is_empty() {
                continue;
            }
            let (head, value) = line.split_once(':').ok_or_else(|| at(format!("expected `name: value`, found `{}`", line.trim())))?;
            let (head, value) = (head.trim().to_ascii_lowercase(), value.trim());
            if !line.starts_with([' ', '\t']) {
                section = match head.as_str() {
                    "about" => {
                        rules.about = value.to_string();
                        Section::Head
                    }
                    "components" => Section::Components,
                    word => match Step::ALL.iter().position(|s| s.word() == word) {
                        Some(k) => {
                            seen[k] = true;
                            Section::Step(k)
                        }
                        None => return Err(at(format!("`{word}` is not a heading a kit file has (about, hut, timber, stone, components)"))),
                    },
                };
                if !value.is_empty() && !matches!(section, Section::Head) {
                    return Err(at(format!("`{head}:` heads a section; its lines go under it, indented")));
                }
                continue;
            }
            match section {
                Section::Head => return Err(at("an indented line belongs under a heading such as `hut:`".into())),
                Section::Step(k) => set(&mut rules.steps[k], &head, value).map_err(at)?,
                Section::Components => {
                    let kind = AnchorKind::ALL
                        .into_iter()
                        .find(|a| a.word() == head)
                        .ok_or_else(|| at(format!("`{head}` is not a kind of anchor ({})", AnchorKind::ALL.map(|a| a.word()).join(", "))))?;
                    // Only a component that goes at this kind of anchor, or nothing.
                    let fits = |w: &str| w == "nothing" || super::components::Component::by_word(w).is_some_and(|c| c.takes(kind));
                    let list = choices(value, |w| fits(w).then(|| w.to_string())).map_err(at)?;
                    rules.components.retain(|(a, _)| *a != kind);
                    rules.components.push((kind, list));
                }
            }
        }
        if let Some(k) = seen.iter().position(|s| !s) {
            return Err(format!("the file has no `{}:` section", Step::ALL[k].word()));
        }
        Ok(rules)
    }
}

/// Sets one rule of a step from its line.
fn set(r: &mut StepRules, key: &str, value: &str) -> Result<(), String> {
    match key {
        "footprint" => r.footprint = choices(value, Form::by_word)?,
        "width" => r.width = span(value)?,
        "length" => r.length = span(value)?,
        "thick" => r.thick = span(value)?,
        "walls" => r.walls = choices(value, WallKind::by_word)?,
        "base" => r.base = span(value)?,
        "wall height" => r.wall_height = span(value)?,
        "roof" => r.roof = choices(value, Cover::by_word)?,
        "roof shape" => r.roof_shape = choices(value, RoofShape::by_word)?,
        "pitch" => r.pitch = span(value)?,
        "overhang" => r.overhang = span(value)?,
        "gable door" => r.gable_door = chance(value)?,
        "floor" => r.floor = choices(value, FloorKind::by_word)?,
        "raised" => r.raised = span(value)?,
        "under" => r.under = choices(value, Under::by_word)?,
        "veranda" => r.veranda = span(value)?,
        "smoke" => r.smoke = choices(value, Smoke::by_word)?,
        "lean-to" => r.lean_to = chance(value)?,
        "windows" => r.windows = span(value)?,
        "door width" => r.door_width = span(value)?,
        "door height" => r.door_height = span(value)?,
        "whitewash" => r.whitewash = chance(value)?,
        "roof stones" => r.roof_stones = chance(value)?,
        "courtyard" => r.courtyard = chance(value)?,
        "parapet" => r.parapet = span(value)?,
        "round corners" => r.round_corners = chance(value)?,
        _ => return Err(format!("`{key}` is not a rule the kit has")),
    }
    Ok(())
}

/// `rich` or `poor` at the end of an option or a chance, or nothing.
fn bias(word: Option<&str>) -> Result<Bias, String> {
    match word {
        None => Ok(Bias::None),
        Some("rich") => Ok(Bias::Rich),
        Some("poor") => Ok(Bias::Poor),
        Some(w) => Err(format!("`{w}` should be `rich`, `poor` or nothing")),
    }
}

/// A weighted list: `cob 3, wattle 2 poor, frame rich`. A weight left out is 1.
fn choices<T>(value: &str, mut what: impl FnMut(&str) -> Option<T>) -> Result<Vec<Choice<T>>, String> {
    let mut out = Vec::new();
    for item in value.split(',') {
        let mut words = item.split_whitespace();
        let Some(word) = words.next() else { return Err("an empty option in a list".into()) };
        let what = what(word).ok_or_else(|| format!("`{word}` is not one of the options here"))?;
        let mut rest: Vec<&str> = words.collect();
        let weight = match rest.first().map(|w| w.parse::<f32>()) {
            Some(Ok(w)) => {
                rest.remove(0);
                w
            }
            _ => 1.0,
        };
        if rest.len() > 1 {
            return Err(format!("`{}` has too much after it", word));
        }
        if !(weight >= 0.0) {
            return Err(format!("`{word}` has a weight below nothing"));
        }
        out.push(Choice { what, weight, bias: bias(rest.first().copied())? });
    }
    Ok(out)
}

/// One number, or the lowest and the highest.
fn span(value: &str) -> Result<Span, String> {
    let v: Option<Vec<f32>> = value.split_whitespace().map(|w| w.parse().ok()).collect();
    match v.as_deref() {
        Some(&[a]) => Ok(Span::one(a)),
        Some(&[a, b]) if a <= b => Ok(Span { lo: a, hi: b }),
        Some(&[_, _]) => Err("a range goes from its lowest to its highest".into()),
        _ => Err(format!("expected one number or two, not `{value}`")),
    }
}

/// A chance, 0 to 1, and `rich` or `poor` after it if wealth tips it.
fn chance(value: &str) -> Result<Chance, String> {
    let mut words = value.split_whitespace();
    let p: f32 = words.next().and_then(|w| w.parse().ok()).ok_or_else(|| format!("expected a chance from 0 to 1, not `{value}`"))?;
    if !(0.0..=1.0).contains(&p) {
        return Err(format!("a chance is from 0 to 1, not {p}"));
    }
    let b = bias(words.next())?;
    if words.next().is_some() {
        return Err("too much after the chance".into());
    }
    Ok(Chance { p, bias: b })
}

mod compiled {
    // KIT: every file under assets/structures/kit, gathered by build.rs.
    include!(concat!(env!("OUT_DIR"), "/kit_rules.rs"));
}

/// Reads every region's file from (path, text) pairs: the rules for each region in
/// [`Region::ALL`] order, and what could not be read. A region whose file is missing or
/// cannot be read is built by the plain defaults.
pub fn gather<'a>(files: impl IntoIterator<Item = (&'a str, &'a str)>) -> (Vec<Rules>, Vec<(String, String)>) {
    let files: Vec<(&str, &str)> = files.into_iter().collect();
    let mut problems = Vec::new();
    let rules = Region::ALL
        .iter()
        .map(|&region| {
            let name = format!("kit/{}.txt", region.word());
            match files.iter().find(|(p, _)| p.replace('\\', "/") == name) {
                Some((_, text)) => Rules::parse(region, text).unwrap_or_else(|e| {
                    problems.push((name.clone(), e));
                    Rules::fallback(region)
                }),
                None => {
                    problems.push((name.clone(), "there is no such file".into()));
                    Rules::fallback(region)
                }
            }
        })
        .collect();
    for (path, _) in &files {
        let known = Region::ALL.iter().any(|r| path.replace('\\', "/") == format!("kit/{}.txt", r.word()));
        if !known {
            problems.push((path.to_string(), "not the name of a region".into()));
        }
    }
    (rules, problems)
}

/// The rules compiled into this build for `region`.
pub fn rules(region: Region) -> &'static Rules {
    static RULES: OnceLock<Vec<Rules>> = OnceLock::new();
    let all = RULES.get_or_init(|| gather(compiled::KIT.iter().copied()).0);
    &all[Region::ALL.iter().position(|&r| r == region).unwrap()]
}

/// Kit files this build could not read, with why.
pub fn problems() -> Vec<(String, String)> {
    gather(compiled::KIT.iter().copied()).1
}
