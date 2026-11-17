//! Place names: believable, specific names that come from what is actually there.
//!
//! Each continent is read once for the things worth naming (see [`survey`]): its
//! regions, the mountain ranges and faults that cross it, islands, lakes, rivers,
//! waterfalls, peaks and bays, and every landmark on its trails. Each gets a name made
//! from what the world knows about it: what kind of place it is, the forest or ground
//! it stands in, the water and landform beside it, and later who lives there.
//!
//! Names are a pure function of the world seed and the place, so they are the same on
//! every visit and for every player, and are stored with the continent in a
//! [`Gazetteer`] that maps, signposts and quests all read from.
//!
//! The land is divided into a few regions, each speaking one tongue chosen by its
//! climate, so names nearby sound like they belong together: Skarvatn and Haldvik in a
//! cold north, Llyn Caerith and Penwen in a wet green west. English descriptive names
//! ("Greywater Falls", "Bogwatch Ruin", "The Long Reach") are mixed in throughout.
//! The words themselves are data, in `words.txt`.

pub mod folk;
mod survey;
mod words;

#[cfg(test)]
mod tests;

pub use survey::survey;

use glam::Vec2;

use crate::{
    features::landmarks::Landmark,
    noise::{hash2, perlin2, unit},
    scale::{m, m64},
};
use words::{KindWords, Tongue, words};

/// Tries at a name before falling back to a plain one from the tongue.
const ATTEMPTS: i32 = 48;
/// Longest word put into a compound such as "Bogwatch".
const COMPOUND_MAX: usize = 6;
/// Nearer than this, two places never lean on the same word.
const SPACING: f32 = m(7000.0);
/// Most places on one continent that may lean on the same word of the surroundings or
/// the same adjective, however far apart.
const WORD_USES: usize = 3;
/// Bits of words that a made-up word of a tongue must not contain.
const UNFORTUNATE: &[&str] = &[
    "penis", "anus", "anal", "rape", "nazi", "cunt", "slut", "shit", "piss", "fag", "dick", "cock", "twat", "porn", "sex",
    "fuck", "butt", "poo", "turd", "wank", "tits",
];
/// How close a built place must be to a natural feature to borrow its name.
const NEAR: f32 = m(250.0);
/// Share of names that use a tongue's own patterns, where it has any for the kind.
const TONGUE_SHARE: f32 = 0.35;
/// Scale of the wobble in region borders, in voxels.
const REGION_WOBBLE: f64 = m64(4000.0);

/// Words that are never the part of a name lent to a neighbour.
const STOP: &[&str] = &[
    "The", "of", "Lake", "Mount", "River", "Isle", "Island", "Bay", "Falls", "Peak", "Water", "Mere", "Old", "Long", "Great",
    "Afon", "Llyn", "Moel", "Ynys", "Porth", "Lago", "Monte", "Rio", "Isla", "Cala", "Salto", "Sierra", "Motu", "Wadi", "Vale",
    "Shrine", "Ruins",
];

/// What kind of place something is, for choosing its words and for maps.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PlaceKind {
    /// A stretch of country with its own tongue.
    Region,
    /// A mountain range crossing the continent (see [`crate::structures`]).
    Range,
    Fault,
    Rift,
    Island,
    Lake,
    River,
    Bay,
    /// A long, narrow arm of the sea.
    Inlet,
    Peak,
    Falls,
    Landmark(Landmark),
    /// Settlements (see [`crate::settlements`]). A large village is named as a village.
    /// (Appended after them: a tor on the moor, see `features::tor`.)
    Hamlet,
    Town,
    /// A tor on the moor (see [`crate::features::tor`]).
    Tor,
    Village,
    GreatTown,
}

impl PlaceKind {
    /// The section of `words.txt` that names this kind.
    pub fn key(self) -> &'static str {
        match self {
            PlaceKind::Region => "region",
            PlaceKind::Range => "range",
            PlaceKind::Fault => "fault",
            PlaceKind::Rift => "rift",
            PlaceKind::Island => "island",
            PlaceKind::Lake => "lake",
            PlaceKind::River => "river",
            PlaceKind::Bay => "bay",
            PlaceKind::Inlet => "inlet",
            PlaceKind::Peak => "peak",
            PlaceKind::Falls => "falls",
            PlaceKind::Landmark(l) => match l {
                Landmark::Shrine => "shrine",
                Landmark::Lookout => "lookout",
                Landmark::Camp => "camp",
                Landmark::Ruins => "ruins",
                Landmark::Stones => "stones",
                Landmark::Cairn => "cairn",
                Landmark::Arch => "arch",
                Landmark::Hut => "hut",
                Landmark::Landing => "landing",
            },
            PlaceKind::Hamlet => "hamlet",
            PlaceKind::Town => "town",
            PlaceKind::Tor => "tor",
            PlaceKind::Village => "village",
            PlaceKind::GreatTown => "greattown",
        }
    }

    /// Land and water, rather than anything people built.
    pub fn is_natural(self) -> bool {
        !matches!(self, PlaceKind::Landmark(_)) && !self.is_settlement()
    }

    /// A hamlet, village or town.
    pub fn is_settlement(self) -> bool {
        matches!(self, PlaceKind::Hamlet | PlaceKind::Village | PlaceKind::Town | PlaceKind::GreatTown)
    }

    /// Whether things built beside it can take its name ("Greymere Landing").
    fn lends_name(self) -> bool {
        matches!(
            self,
            PlaceKind::Island
                | PlaceKind::Lake
                | PlaceKind::River
                | PlaceKind::Bay
                | PlaceKind::Inlet
                | PlaceKind::Peak
                | PlaceKind::Falls
                | PlaceKind::Tor
        ) || self.is_settlement()
    }

    /// Whether it can take the name of something beside it.
    fn borrows_name(self) -> bool {
        matches!(self, PlaceKind::Falls | PlaceKind::Landmark(_)) || self.is_settlement()
    }

    /// Whether people keep it, so it can be named after one of them ("Marren's Hut").
    fn kept(self) -> bool {
        matches!(self, PlaceKind::Landmark(Landmark::Hut | Landmark::Camp | Landmark::Landing)) || self.is_settlement()
    }

    /// Whether the place spans several macro-cells, so every one of them must give it
    /// the same name.
    fn shared(self) -> bool {
        matches!(self, PlaceKind::Range | PlaceKind::Fault | PlaceKind::Rift)
    }
}

/// One named place.
#[derive(Clone, Debug)]
pub struct Place {
    pub name: String,
    pub kind: PlaceKind,
    /// Continent-local voxel position the name belongs at: the landmark itself, the
    /// middle of a lake or island, a summit, a point part way along a river.
    pub at: Vec2,
    /// How far the place reaches from `at` (or from `line`), in voxels.
    pub radius: f32,
    /// The course of a long place such as a river or range, in continent-local voxels,
    /// from its mouth or one end; empty for everything else.
    pub line: Vec<Vec2>,
    /// How much it matters on a map; larger names show sooner. Landmarks use
    /// [`crate::trails::sites::importance`], and regions are largest at 30.
    pub importance: f32,
    /// Index in `Network::sites`, for a landmark.
    pub site: Option<usize>,
    /// Index in `Network::settlements`, for a hamlet, village or town.
    pub settlement: Option<usize>,
    /// The part of the name that can be lent to things beside it.
    core: Option<String>,
}

impl Place {
    /// Distance from a continent-local position to the edge of the place, 0 inside.
    pub fn distance(&self, p: Vec2) -> f32 {
        let mut d = self.at.distance(p);
        for pair in self.line.windows(2) {
            d = d.min(segment_distance(p, pair[0], pair[1]));
        }
        if let [only] = self.line.as_slice() {
            d = d.min(only.distance(p));
        }
        (d - self.radius).max(0.0)
    }
}

fn segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
    (a + ab * t).distance(p)
}

/// What is known about a place, for choosing its words.
#[derive(Clone, Debug, Default)]
pub struct Facts {
    /// What describes its surroundings, most telling first: keys of `[feature]` in
    /// `words.txt`, such as "bog", "maple", "lake" or "peak".
    pub tags: Vec<&'static str>,
    /// Who lives here. Once there are residents, a place people keep can be named
    /// after one of them; until then such places borrow a name from the tongue.
    pub residents: Vec<String>,
}

/// A place to be named and added to a [`Gazetteer`].
pub struct Request<'a> {
    pub kind: PlaceKind,
    pub at: Vec2,
    pub radius: f32,
    pub line: Vec<Vec2>,
    pub importance: f32,
    pub facts: &'a Facts,
    /// Anything stable that tells this place from others of its kind, such as its
    /// position; it varies the choices made.
    pub salt: u32,
}

/// A region's centre and tongue.
#[derive(Clone, Copy, Debug)]
struct Region {
    at: Vec2,
    /// Index into the tongues of `words.txt`.
    tongue: usize,
    /// Index of its own place in [`Gazetteer::places`].
    place: Option<usize>,
}

/// Every named place on one continent, with what is needed to name more of them
/// consistently (settlements, later).
#[derive(Default, Debug)]
pub struct Gazetteer {
    /// In the order they were named: regions, then structures, islands, lakes, rivers,
    /// falls, peaks, bays and landmarks.
    pub places: Vec<Place>,
    /// The continent's seed.
    seed: u32,
    regions: Vec<Region>,
    /// Place of each trail site, by site index.
    sites: Vec<Option<usize>>,
    /// Place of each settlement, by its index in `Network::settlements`.
    settlements: Vec<Option<usize>>,
    /// Words already in names, and where, so neighbours don't lean on the same one.
    used: Vec<(String, Vec2)>,
}

impl Gazetteer {
    pub fn new(seed: u32) -> Self {
        Self { seed, ..Default::default() }
    }

    /// The place of a landmark, by its index in `Network::sites`.
    pub fn site(&self, site: usize) -> Option<&Place> {
        self.places.get((*self.sites.get(site)?)?)
    }

    pub fn named(&self, name: &str) -> Option<&Place> {
        self.places.iter().find(|p| p.name == name)
    }

    /// Index in [`Self::places`] of the region a continent-local position lies in.
    pub fn region_at(&self, p: Vec2) -> Option<usize> {
        self.regions.get(self.region_index(p)?)?.place
    }

    /// Name of the tongue spoken at a continent-local position, for tools.
    pub fn tongue_at(&self, p: Vec2) -> &'static str {
        let w = words();
        w.tongues.get(self.tongue_index(p)).map_or("", |t| t.name.as_str())
    }

    /// Which region a position lies in: the nearest centre, by a distance that wobbles
    /// so borders wander instead of running straight.
    fn region_index(&self, p: Vec2) -> Option<usize> {
        let (x, z) = (p.x as f64 / REGION_WOBBLE, p.y as f64 / REGION_WOBBLE);
        self.regions
            .iter()
            .enumerate()
            .map(|(k, r)| {
                let wobble = 1.0 + 0.3 * perlin2(self.seed ^ (k as u32).wrapping_mul(0x632b_e5ab), x, z);
                (k, r.at.distance(p) * wobble)
            })
            .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))
            .map(|(k, _)| k)
    }

    fn tongue_index(&self, p: Vec2) -> usize {
        self.region_index(p).map_or(0, |k| self.regions[k].tongue)
    }

    /// Divides the land into regions around `centres`, each speaking a tongue named in
    /// `words.txt`. Their own names are added later, as places of kind Region.
    pub(crate) fn set_regions(&mut self, centres: &[(Vec2, &str)]) {
        let w = words();
        self.regions = centres
            .iter()
            .map(|&(at, tongue)| Region { at, tongue: w.tongue_named(tongue), place: None })
            .collect();
    }

    /// Names a place and adds it, returning its index in [`Self::places`].
    pub fn add(&mut self, req: Request) -> usize {
        let (name, core, words) = self.compose(&req);
        self.used.extend(words.into_iter().map(|w| (w, req.at)));
        self.places.push(Place {
            name,
            kind: req.kind,
            at: req.at,
            radius: req.radius,
            line: req.line,
            importance: req.importance,
            site: None,
            settlement: None,
            core,
        });
        self.places.len() - 1
    }

    /// [`Self::add`] for the landmark at `site` in `Network::sites`.
    pub(crate) fn add_site(&mut self, site: usize, req: Request) -> usize {
        let i = self.add(req);
        self.places[i].site = Some(site);
        if self.sites.len() <= site {
            self.sites.resize(site + 1, None);
        }
        self.sites[site] = Some(i);
        i
    }

    /// The place of a settlement, by its index in `Network::settlements`.
    pub fn settlement(&self, k: usize) -> Option<&Place> {
        self.places.get((*self.settlements.get(k)?)?)
    }

    /// [`Self::add`] for the settlement at `k` in `Network::settlements`.
    pub(crate) fn add_settlement(&mut self, k: usize, req: Request) -> usize {
        let i = self.add(req);
        self.places[i].settlement = Some(k);
        if self.settlements.len() <= k {
            self.settlements.resize(k + 1, None);
        }
        self.settlements[k] = Some(i);
        i
    }

    /// `n` given names of the tongue spoken at continent-local `at`, chosen by `salt`:
    /// the people of a place, for naming it after one of them.
    pub fn given_names(&self, at: Vec2, salt: u32, n: usize) -> Vec<String> {
        let tongue = &words().tongues[self.tongue_index(at)];
        let mut out: Vec<String> = Vec::new();
        for k in 0..n as i32 * 4 {
            if out.len() >= n {
                break;
            }
            if let Some(name) = pick(&tongue.given, hash2(salt ^ self.seed, k, 0x9e)).map(String::from)
                && !out.contains(&name)
            {
                out.push(name);
            }
        }
        out
    }

    /// The name of the nearest natural feature close enough to lend it.
    fn near_core(&self, req: &Request) -> Option<&str> {
        if !req.kind.borrows_name() {
            return None;
        }
        self.places
            .iter()
            .filter(|p| p.kind.lends_name() && p.kind != req.kind)
            .filter_map(|p| Some((p.core.as_deref()?, p.distance(req.at))))
            .filter(|&(_, d)| d < NEAR)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(core, _)| core)
    }

    /// A name for `req` that no other place here has, with the part of it that can be
    /// lent to neighbours, and the words in it that neighbours should not reuse.
    fn compose(&self, req: &Request) -> (String, Option<String>, Vec<String>) {
        let w = words();
        static NONE: KindWords = KindWords { pattern: Vec::new(), kind: Vec::new(), role: Vec::new(), adj: Vec::new() };
        let kind = w.kinds.get(req.kind.key()).unwrap_or(&NONE);
        let shared = req.kind.shared();
        // A place spanning several cells is named the same from each: by its own salt
        // alone, in a tongue that salt picks.
        let (base, tongue) = if shared {
            (req.salt, (hash2(req.salt, 7, 7) as usize) % w.tongues.len())
        } else {
            (req.salt ^ self.seed.rotate_left(13), self.tongue_index(req.at))
        };
        let tongue = &w.tongues[tongue];
        let own: &[String] = &kind.pattern;
        let theirs: &[String] = tongue.patterns.get(req.kind.key()).map(|v| v.as_slice()).unwrap_or(&[]);
        let near = self.near_core(req);

        for attempt in 0..ATTEMPTS {
            let r = |k: i32| hash2(base, attempt, k);
            let use_theirs = !theirs.is_empty() && (own.is_empty() || unit(r(1)) < TONGUE_SHARE);
            let pattern = pick(if use_theirs { theirs } else { own }, r(2)).unwrap_or("{root}");
            let (root, stem) = make_root(tongue, r(3));
            let low = root.to_lowercase();
            let word = tag_word(&req.facts.tags, r(4), r(5));
            let kind_word = pick(&kind.kind, r(6));
            let role = pick(&kind.role, r(7));
            let adj = pick(&kind.adj, r(8));
            if UNFORTUNATE.iter().any(|bad| low.contains(bad)) {
                continue;
            }
            let owner = if req.facts.residents.is_empty() {
                req.kind.kept().then(|| pick(&tongue.given, r(9))).flatten()
            } else {
                pick(&req.facts.residents, r(9))
            };
            let value = |slot: &str| -> Option<String> {
                match slot {
                    "root" => Some(root.clone()),
                    "low" => Some(low.clone()),
                    "stem" => Some(stem.to_string()),
                    "word" => word.map(String::from),
                    "type" => kind_word.map(String::from),
                    "role" => role.map(String::from),
                    "adj" => adj.map(String::from),
                    "near" => near.map(String::from),
                    "owner" => owner.map(String::from),
                    _ => None,
                }
            };
            let Some((name, slots)) = fill(pattern, value) else { continue };
            // A name that says its opening twice, as "Caercaerdur" or "Caer Caerwen": the
            // word just before the root, joined to it or not, must not begin as it does.
            let echoes = |slot: &str, letters: usize| {
                pattern.split_once(slot).is_some_and(|(before, _)| {
                    let lead = before.split_whitespace().last().unwrap_or("").to_lowercase();
                    lead.len() >= letters && low.len() >= letters && lead.as_bytes()[..letters] == low.as_bytes()[..letters]
                })
            };
            if echoes("{low}", 2) || echoes("{root}", 3) {
                continue;
            }
            let has = |s: &str| slots.iter().any(|x| x == s);
            // A role and a type saying the same thing, as in "Watchwatch".
            if let (Some(role), Some(t)) = (role, kind_word)
                && has("role")
                && has("type")
                && t.to_lowercase().contains(role)
            {
                continue;
            }
            if name.is_empty() || self.places.iter().any(|p| p.name == name) {
                continue;
            }
            // Roots are never reused in one continent, and other words not nearby,
            // unless they were lent on purpose through {near}.
            let mut words = Vec::new();
            if has("root") || has("low") {
                if !shared && self.used.iter().any(|(u, _)| *u == root) {
                    continue;
                }
                words.push(root.clone());
            }
            let crowded = |w: &str| {
                !shared && {
                    let uses = self.used.iter().filter(|(u, _)| u == w);
                    uses.clone().any(|(_, p)| p.distance(req.at) < SPACING) || uses.count() >= WORD_USES
                }
            };
            if has("stem") {
                if crowded(stem) {
                    continue;
                }
                words.push(stem.to_string());
            }
            if has("word")
                && let Some(word) = word
            {
                if crowded(word) {
                    continue;
                }
                words.push(word.to_string());
            }
            if has("adj")
                && let Some(adj) = adj
            {
                // "The Old ..." is a pattern of its own: only so many of those nearby.
                let tag = format!("adj:{adj}");
                if crowded(&tag) {
                    continue;
                }
                words.push(tag);
            }
            let core = core_of(&name, &root);
            // The telling part of a name ("Roundring" of "Roundring Stones") is never
            // repeated on one continent, unless it was lent on purpose through {near}.
            if !shared
                && !has("near")
                && let Some(c) = &core
                && self.places.iter().any(|p| p.core.as_ref() == Some(c) && !p.kind.shared())
            {
                continue;
            }
            return (name, core, words);
        }

        // Every pattern was taken: a plain name from the tongue, which has hundreds.
        let suffix = pick(&kind.kind, hash2(base, -1, 6)).map_or(String::new(), |t| format!(" {t}"));
        for k in 0..2000 {
            let (root, _) = make_root(tongue, hash2(base ^ 0x00fa_11ba, k, 0));
            let low = root.to_lowercase();
            if UNFORTUNATE.iter().any(|bad| low.contains(bad)) {
                continue;
            }
            let name = format!("{root}{suffix}");
            if !self.places.iter().any(|p| p.name == name) && !self.used.iter().any(|(u, _)| *u == root) {
                return (name, Some(root.clone()), vec![root]);
            }
        }
        let name = format!("{}{suffix} {}", tongue.first.first().map_or("Place", String::as_str), self.places.len());
        (name, None, Vec::new())
    }
}

fn pick<S: AsRef<str>>(list: &[S], h: u32) -> Option<&str> {
    (!list.is_empty()).then(|| list[h as usize % list.len()].as_ref())
}

/// A word of the tongue, and the first part it was built from.
fn make_root(t: &Tongue, h: u32) -> (String, &str) {
    let first = pick(&t.first, h).unwrap_or("Al");
    let second = pick(&t.second, hash2(h, 1, 0)).unwrap_or("");
    let vowel = |c: char| "aeiouy".contains(c.to_ascii_lowercase());
    let (end, start) = (first.chars().last(), second.chars().next());
    // "Skar" and "rud" make Skarud, "Ka" and "ola" make Kala: no doubled letters, and
    // no two vowels run together at the join.
    let second = match (end, start) {
        (Some(a), Some(b)) if a.eq_ignore_ascii_case(&b) || (vowel(a) && vowel(b)) => &second[1..],
        _ => second,
    };
    (format!("{first}{second}"), first)
}

/// A word for the surroundings, favouring the most telling tags.
fn tag_word(tags: &[&str], a: u32, b: u32) -> Option<&'static str> {
    if tags.is_empty() {
        return None;
    }
    let u = unit(a);
    let tag = tags[((u * u * tags.len() as f32) as usize).min(tags.len() - 1)];
    pick(words().features.get(tag)?, b)
}

/// Fills a pattern's slots, returning the name and which slots it used, or None if a
/// slot has no value or a compound would get a word too long for it.
fn fill(pattern: &str, value: impl Fn(&str) -> Option<String>) -> Option<(String, Vec<String>)> {
    let (mut out, mut slots, mut rest) = (String::new(), Vec::new(), pattern);
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let close = after.find('}')?;
        let slot = &after[..close];
        let v = value(slot)?;
        let tail = &after[close + 1..];
        // Written straight against letters or another slot: a compound.
        let compound = tail.chars().next().is_some_and(|c| c.is_alphabetic() || c == '{');
        if compound && (v.len() > COMPOUND_MAX || v.contains(' ')) {
            return None;
        }
        out.push_str(&v);
        slots.push(slot.to_string());
        rest = tail;
    }
    out.push_str(rest);
    let mut name = out.split_whitespace().collect::<Vec<_>>().join(" ");
    // Lower-case words of a tongue can open a name ("{low}"); capitalise it.
    if let Some(c) = name.chars().next()
        && c.is_lowercase()
    {
        name = c.to_uppercase().collect::<String>() + &name[c.len_utf8()..];
    }
    Some((name, slots))
}

/// The part of a name that can be lent: the tongue's word if it stands alone in the
/// name, else the first word that says something.
fn core_of(name: &str, root: &str) -> Option<String> {
    let tokens: Vec<&str> = name.split(' ').collect();
    if tokens.contains(&root) {
        return Some(root.to_string());
    }
    tokens
        .into_iter()
        .find(|t| t.len() > 2 && !STOP.contains(t) && !t.contains('\''))
        .map(String::from)
}
