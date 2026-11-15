//! Structures drawn by hand in the structure editor (`game --structures`), the plain text
//! files they are kept in, and how the world chooses between them and the generator.
//!
//! A saved structure is built of roles, not materials: "stone", "timber", "roof" and the
//! rest are taken from the [`Palette`] of the place it stands, so one drawn shrine is
//! granite on the moor and sandstone in the desert, as the generator's own are. Anything
//! drawn in a named material (a lantern, iron, charcoal) is that material everywhere.
//!
//! The files live in the repository under `assets/structures/<kind>/`, one to a
//! structure, and are compiled into the game, so a world is the same on every machine
//! that has the same files. Each has a `weight`, how often it is chosen against the
//! others of its kind, and the generator has one too, in `<kind>/generator.txt`: none
//! when it is missing, so once a kind has a structure saved, its sites are built from
//! the saved ones only until the generator is given a weight again. A kind with nothing
//! saved is built by the generator as before.
//!
//! The format, in brief (the editor writes it, and it is meant to be read):
//!
//! ```text
//! kind: shrine
//! name: High gate
//! weight: 1
//! corner: -9 -10
//! key:
//!   S stone
//!   T timber
//!   a LANTERN
//! layer 0:
//!   ..SSS..
//!   ..SaS..
//! fine:
//!   0 20 0  4 24 4  timber
//! keep clear:
//!   -3 0 -10  3 8 2
//! ```
//!
//! Each `layer Y:` is one level seen from above, one character to a voxel: x runs
//! along a row (east), z down the rows (south), and the first character of the first
//! row is at `corner` (x then z). `.` is nothing, which leaves the world as it was;
//! layer 0 is the first above the ground and layers below it are footings. `fine` lists
//! the small parts drawn finer than voxels (a lamp's frame), in quarter voxels with the
//! top corner exclusive, and `keep clear` boxes of voxels that props are kept out of
//! (doors, stairs), both inclusive.

use std::{collections::HashMap, fmt::Write as _, sync::OnceLock};

use glam::IVec3;
use voxel::{AIR, Voxel, mesh::PlantBox};

use super::{
    landmarks::{self, Landmark, Palette},
    shape::Shape,
};
use crate::{
    materials as mat,
    noise::{hash2, hash3, unit},
};

/// A part of a structure that takes its material from the place it stands.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Role {
    Stone,
    DarkStone,
    /// Stone, dark stone and mossy stone mixed voxel by voxel, as the generator lays
    /// rough walling.
    Weathered,
    /// Mossy stone where the place has moss, dark stone where it is too dry.
    Mossy,
    Timber,
    Planks,
    Roof,
    /// The place's colour: cloth, painted posts.
    Accent,
}

impl Role {
    pub const ALL: [Role; 8] = [
        Role::Stone,
        Role::DarkStone,
        Role::Weathered,
        Role::Mossy,
        Role::Timber,
        Role::Planks,
        Role::Roof,
        Role::Accent,
    ];

    /// The word for it in a file.
    pub fn word(self) -> &'static str {
        match self {
            Role::Stone => "stone",
            Role::DarkStone => "dark-stone",
            Role::Weathered => "weathered-stone",
            Role::Mossy => "mossy-stone",
            Role::Timber => "timber",
            Role::Planks => "planks",
            Role::Roof => "roof",
            Role::Accent => "accent",
        }
    }

    /// Its name in the editor.
    pub fn label(self) -> &'static str {
        match self {
            Role::Stone => "Stone",
            Role::DarkStone => "Dark stone",
            Role::Weathered => "Weathered stone",
            Role::Mossy => "Mossy stone",
            Role::Timber => "Timber",
            Role::Planks => "Planks",
            Role::Roof => "Roof",
            Role::Accent => "Accent colour",
        }
    }

    /// What it becomes, for the editor's tooltips.
    pub fn about(self) -> &'static str {
        match self {
            Role::Stone => "The rock in the ground where it stands: granite on the moor, sandstone in the desert",
            Role::DarkStone => "The stone found with that rock, for footings, courses and copings",
            Role::Weathered => "Stone, dark stone and moss mixed voxel by voxel, like old rough walling",
            Role::Mossy => "Mossy stone where the place has moss, dark stone where it is too dry",
            Role::Timber => "The bark of the trees round about: posts, beams, logs",
            Role::Planks => "Sawn boards in the place's timber",
            Role::Roof => "Thatch, slate, tile or boards, whichever the place roofs with",
            Role::Accent => "The place's colour: red, blue or pale cloth, painted gates",
        }
    }

    /// Its character in a file's key.
    fn symbol(self) -> char {
        match self {
            Role::Stone => 'S',
            Role::DarkStone => 'D',
            Role::Weathered => 'W',
            Role::Mossy => 'M',
            Role::Timber => 'T',
            Role::Planks => 'P',
            Role::Roof => 'R',
            Role::Accent => 'A',
        }
    }

    /// The material it is at voxel `at` of a site built of `p` with `seed`.
    pub fn resolve(self, p: &Palette, seed: u32, at: IVec3) -> Voxel {
        match self {
            Role::Stone => p.stone,
            Role::DarkStone => p.stone_dark,
            Role::Weathered => {
                // The same shares as Palette::block, from the voxel's place rather than
                // a stream, so an edit elsewhere never reshuffles the rest.
                let r = unit(hash3(seed ^ 0x57a7_e0e5, at.x, at.y, at.z));
                if r < p.moss * 0.6 {
                    mat::MOSSY_STONE
                } else if r < 0.3 + p.moss * 0.4 {
                    p.stone_dark
                } else {
                    p.stone
                }
            }
            Role::Mossy if p.moss >= 0.1 => mat::MOSSY_STONE,
            Role::Mossy => p.stone_dark,
            Role::Timber => p.wood,
            Role::Planks => p.plank,
            Role::Roof => p.roof,
            Role::Accent => p.accent,
        }
    }

    /// The material this role always is in `p`, for the roles that have one.
    fn plain(self, p: &Palette) -> Option<Voxel> {
        match self {
            Role::Stone => Some(p.stone),
            Role::DarkStone => Some(p.stone_dark),
            Role::Weathered => None,
            Role::Mossy => Some(mat::MOSSY_STONE),
            Role::Timber => Some(p.wood),
            Role::Planks => Some(p.plank),
            Role::Roof => Some(p.roof),
            Role::Accent => Some(p.accent),
        }
    }
}

/// What one voxel of a saved structure is.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Cell {
    /// Emptied: the ground or plants that were there are cleared away.
    Air,
    Role(Role),
    Material(Voxel),
}

impl Cell {
    /// The word for it in a file.
    pub fn word(self) -> &'static str {
        match self {
            Cell::Air => "air",
            Cell::Role(r) => r.word(),
            Cell::Material(v) => mat::name_of(v).unwrap_or("air"),
        }
    }

    /// The cell a file's word names: a role, `air`, or a material by its name in the code.
    pub fn parse(word: &str) -> Option<Cell> {
        if word.eq_ignore_ascii_case("air") {
            return Some(Cell::Air);
        }
        if let Some(r) = Role::ALL.into_iter().find(|r| r.word().eq_ignore_ascii_case(word)) {
            return Some(Cell::Role(r));
        }
        mat::by_name(word).map(Cell::Material)
    }

    pub fn resolve(self, p: &Palette, seed: u32, at: IVec3) -> Voxel {
        match self {
            Cell::Air => AIR,
            Cell::Role(r) => r.resolve(p, seed, at),
            Cell::Material(v) => v,
        }
    }
}

/// A part finer than a voxel, in model cells of a quarter voxel (see [`PlantBox`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Fine {
    pub lo: IVec3,
    /// Exclusive.
    pub hi: IVec3,
    pub cell: Cell,
    pub sway_lo: bool,
    pub sway_hi: bool,
    pub thin: u8,
    pub season: u8,
}

/// One saved structure.
#[derive(Clone, PartialEq, Debug)]
pub struct Design {
    pub kind: Landmark,
    pub name: String,
    /// How often it is chosen against the others of its kind and the generator.
    pub weight: u32,
    /// Anything worth remembering about it, such as what it was copied from.
    pub note: String,
    pub cells: HashMap<IVec3, Cell>,
    pub fine: Vec<Fine>,
    pub clear: Vec<(IVec3, IVec3)>,
}

/// The kinds of landmark a saved structure can stand for. Landings are left out: the
/// length of a pier follows the shore it is built from.
pub const KINDS: [Landmark; 7] = [
    Landmark::Shrine,
    Landmark::Lookout,
    Landmark::Camp,
    Landmark::Ruins,
    Landmark::Stones,
    Landmark::Cairn,
    Landmark::Arch,
];

/// A kind's word in files and folder names.
pub fn kind_word(kind: Landmark) -> &'static str {
    match kind {
        Landmark::Shrine => "shrine",
        Landmark::Lookout => "lookout",
        Landmark::Camp => "camp",
        Landmark::Ruins => "ruins",
        Landmark::Stones => "stones",
        Landmark::Cairn => "cairn",
        Landmark::Arch => "arch",
        Landmark::Hut => "hut",
        Landmark::Landing => "landing",
    }
}

fn kind_by_word(word: &str) -> Option<Landmark> {
    KINDS.into_iter().find(|&k| kind_word(k).eq_ignore_ascii_case(word))
}

impl Design {
    pub fn new(kind: Landmark, name: &str) -> Self {
        Design {
            kind,
            name: name.to_string(),
            weight: 1,
            note: String::new(),
            cells: HashMap::new(),
            fine: Vec::new(),
            clear: Vec::new(),
        }
    }

    /// The generator's variant `seed` of `kind`, as the editor shows it for a place with
    /// palette `p` (already varied for the site, as [`landmarks::build_site`] varies it),
    /// turned into roles so it can be drawn over and saved.
    pub fn copy_of(kind: Landmark, p: &Palette, seed: u32, name: &str) -> Self {
        let mut shape = landmarks::generate(kind, p, seed, 0);
        // The same model again with every role in a material of its own, which says which
        // role each voxel came from even where the place uses one material for two of
        // them (planks for both the roof and the floor, say). Building it the second way
        // can turn out a little differently where the generator weighs materials, so it
        // is only believed where the two agree.
        let tags = Palette {
            stone: mat::CAVE3_STONE,
            stone_dark: mat::CAVE3_ROCK_DARK,
            wood: mat::CAVE3_ROCK_BROWN,
            plank: mat::CAVE3_SANDSTONE,
            roof: mat::CAVE3_SANDSTONE_RED,
            accent: mat::CAVE3_GRAVEL,
            ..*p
        };
        let mut tagged = landmarks::generate(kind, &tags, seed, 0);
        let hint: HashMap<IVec3, Voxel> = tagged.snapshot().into_iter().filter_map(|q| Some((q, tagged.get(q)?))).collect();
        let cell = |v: Voxel, tag: Option<Voxel>| -> Cell {
            if v == AIR {
                return Cell::Air;
            }
            if v == mat::MOSSY_STONE {
                return Cell::Role(Role::Mossy);
            }
            if let Some(t) = tag {
                if let Some(r) = Role::ALL.into_iter().find(|r| r.plain(&tags) == Some(t) && r.plain(p) == Some(v)) {
                    return Cell::Role(r);
                }
                if t == v {
                    return Cell::Material(v);
                }
            }
            match Role::ALL.into_iter().find(|r| r.plain(p) == Some(v)) {
                Some(r) => Cell::Role(r),
                None => Cell::Material(v),
            }
        };
        let mut d = Design::new(kind, name);
        d.note = format!("copied from the generator's {} number {seed}", kind_word(kind));
        for q in shape.snapshot() {
            if let Some(v) = shape.get(q) {
                d.cells.insert(q, cell(v, hint.get(&q).copied()));
            }
        }
        d.clear = shape.clear_boxes().to_vec();
        let (fine, fine_tags) = (shape.take_fine(), tagged.take_fine());
        let same = fine.len() == fine_tags.len();
        for (k, b) in fine.iter().enumerate() {
            let tag = same.then(|| fine_tags[k]).filter(|t| t.lo == b.lo && t.hi == b.hi).map(|t| t.voxel);
            d.fine.push(Fine {
                lo: IVec3::from_array(b.lo),
                hi: IVec3::from_array(b.hi),
                cell: cell(b.voxel, tag),
                sway_lo: b.sway_lo,
                sway_hi: b.sway_hi,
                thin: b.thin,
                season: b.season,
            });
        }
        d
    }

    /// The structure built for a site of palette `p` (varied for the site) and `seed`,
    /// ready for [`landmarks::finish`] to lay props about.
    pub fn shape(&self, p: &Palette, seed: u32) -> Shape {
        let mut s = Shape::raw();
        let mut cells: Vec<(&IVec3, &Cell)> = self.cells.iter().collect();
        cells.sort_unstable_by_key(|(q, _)| (q.y, q.x, q.z));
        for (&q, c) in cells {
            s.paint(q, c.resolve(p, seed, q));
        }
        for f in &self.fine {
            let voxel = match f.cell {
                Cell::Air => continue,
                c => c.resolve(p, seed, f.lo.div_euclid(IVec3::splat(voxel::mesh::CELLS))),
            };
            s.add_fine(PlantBox {
                lo: f.lo.to_array(),
                hi: f.hi.to_array(),
                voxel,
                sway_lo: f.sway_lo,
                sway_hi: f.sway_hi,
                thin: f.thin,
                season: f.season,
            });
        }
        for &(lo, hi) in &self.clear {
            s.keep_clear(lo, hi);
        }
        s
    }

    /// The smallest box (inclusive) holding every cell, if there are any.
    pub fn bounds(&self) -> Option<(IVec3, IVec3)> {
        let mut it = self.cells.keys();
        let first = *it.next()?;
        Some(it.fold((first, first), |(lo, hi), &q| (lo.min(q), hi.max(q))))
    }

    /// The file a structure of this name is saved as, under the structures folder.
    pub fn file_name(&self) -> String {
        format!("{}/{}.txt", kind_word(self.kind), slug(&self.name))
    }

    /// The structure as a file.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "# A structure for Willow Hollow, made in the structure editor (game --structures).");
        let _ = writeln!(out, "# Plain text, and fine to change by hand: docs/DEVELOPMENT.md says how it reads.");
        let _ = writeln!(out, "kind: {}", kind_word(self.kind));
        let _ = writeln!(out, "name: {}", one_line(&self.name));
        let _ = writeln!(out, "weight: {}", self.weight);
        if !self.note.trim().is_empty() {
            let _ = writeln!(out, "note: {}", one_line(&self.note));
        }
        // Symbols for the materials, in order of their names so a file changes little
        // when a material is added.
        let mut used: Vec<Voxel> = self.cells.values().filter_map(|c| if let Cell::Material(v) = c { Some(*v) } else { None }).collect();
        used.sort_unstable_by_key(|&v| mat::name_of(v).unwrap_or(""));
        used.dedup();
        let mut symbols: HashMap<Cell, char> = HashMap::new();
        symbols.insert(Cell::Air, '_');
        for r in Role::ALL {
            symbols.insert(Cell::Role(r), r.symbol());
        }
        let mut spare = MATERIAL_SYMBOLS.chars();
        for &v in &used {
            // Past the last spare symbol a material is written as dark stone; there are
            // over a hundred, and no structure comes near using that many materials.
            let c = spare.next().unwrap_or('D');
            symbols.insert(Cell::Material(v), c);
        }
        let Some((lo, hi)) = self.bounds() else {
            let _ = writeln!(out, "corner: 0 0");
            return out + &self.tail();
        };
        let _ = writeln!(out, "corner: {} {}", lo.x, lo.z);
        let _ = writeln!(out, "key:");
        // Air and the roles first, then the materials, listing only what is used.
        let present: std::collections::HashSet<Cell> = self.cells.values().copied().collect();
        let listed = [Cell::Air].into_iter().chain(Role::ALL.map(Cell::Role)).chain(used.iter().map(|&v| Cell::Material(v)));
        for c in listed.filter(|c| present.contains(c)) {
            let _ = writeln!(out, "  {} {}", symbols[&c], c.word());
        }
        for y in lo.y..=hi.y {
            if !(lo.z..=hi.z).any(|z| (lo.x..=hi.x).any(|x| self.cells.contains_key(&IVec3::new(x, y, z)))) {
                continue;
            }
            let _ = writeln!(out, "layer {y}:");
            for z in lo.z..=hi.z {
                out.push_str("  ");
                for x in lo.x..=hi.x {
                    out.push(self.cells.get(&IVec3::new(x, y, z)).map_or('.', |c| symbols[c]));
                }
                out.push('\n');
            }
        }
        out + &self.tail()
    }

    /// The fine parts and the ways through, the end of a file.
    fn tail(&self) -> String {
        let mut out = String::new();
        if !self.fine.is_empty() {
            let _ = writeln!(out, "fine:");
            for f in &self.fine {
                let _ = write!(out, "  {} {} {}  {} {} {}  {}", f.lo.x, f.lo.y, f.lo.z, f.hi.x, f.hi.y, f.hi.z, f.cell.word());
                if f.sway_lo {
                    out.push_str(" sway-low");
                }
                if f.sway_hi {
                    out.push_str(" sway-high");
                }
                if f.thin != 0 {
                    let _ = write!(out, " thin {}", f.thin);
                }
                if f.season != 0 {
                    let _ = write!(out, " season {}", f.season);
                }
                out.push('\n');
            }
        }
        if !self.clear.is_empty() {
            let _ = writeln!(out, "keep clear:");
            for (lo, hi) in &self.clear {
                let _ = writeln!(out, "  {} {} {}  {} {} {}", lo.x, lo.y, lo.z, hi.x, hi.y, hi.z);
            }
        }
        out
    }

    /// Reads a structure from its file. The error says which line is wrong and why.
    pub fn parse(text: &str) -> Result<Design, String> {
        enum Section {
            Head,
            Key,
            Layer(i32, i32),
            Fine,
            Clear,
        }
        let mut d = Design::new(Landmark::Shrine, "");
        let mut kind = None;
        let mut corner = (0, 0);
        let mut key: HashMap<char, Cell> = HashMap::new();
        let mut section = Section::Head;
        for (n, raw) in text.lines().enumerate() {
            let at = |msg: String| format!("line {}: {msg}", n + 1);
            let line = raw.split('#').next().unwrap_or("").trim_end();
            if line.trim().is_empty() {
                continue;
            }
            if !line.starts_with([' ', '\t']) {
                let (head, value) = line.split_once(':').ok_or_else(|| at(format!("expected a heading such as `layer 0:`, found `{}`", line.trim())))?;
                let value = value.trim();
                let head = head.trim().to_ascii_lowercase();
                section = Section::Head;
                match head.as_str() {
                    "kind" => kind = Some(kind_by_word(value).ok_or_else(|| at(format!("`{value}` is not a kind of structure that can be saved (one of {})", KINDS.map(kind_word).join(", "))))?),
                    "name" => d.name = value.to_string(),
                    "note" => d.note = value.to_string(),
                    "weight" => d.weight = value.parse().map_err(|_| at(format!("the weight should be a whole number, not `{value}`")))?,
                    "corner" => {
                        let v = numbers(value).filter(|v| v.len() == 2).ok_or_else(|| at("the corner should be two whole numbers, x and z".into()))?;
                        corner = (v[0], v[1]);
                    }
                    "key" => section = Section::Key,
                    "fine" => section = Section::Fine,
                    "keep clear" => section = Section::Clear,
                    h if h.starts_with("layer") => {
                        let y = h["layer".len()..].trim().parse().map_err(|_| at(format!("a layer needs its height, as in `layer 0:`, not `{h}`")))?;
                        section = Section::Layer(y, 0);
                    }
                    _ => return Err(at(format!("`{head}` is not a heading this format has"))),
                }
                continue;
            }
            let body = line.trim();
            match &mut section {
                Section::Head => return Err(at("an indented line belongs under a heading such as `key:` or `layer 0:`".into())),
                Section::Key => {
                    let mut chars = body.chars();
                    let symbol = chars.next().unwrap();
                    let word = chars.as_str().trim();
                    if symbol == '.' {
                        return Err(at("`.` always means nothing, and cannot be given a material".into()));
                    }
                    let cell = Cell::parse(word).ok_or_else(|| at(format!("`{word}` is not a role (such as stone or timber) or a material")))?;
                    key.insert(symbol, cell);
                }
                Section::Layer(y, row) => {
                    for (i, ch) in body.chars().enumerate() {
                        if ch == '.' || ch == ' ' {
                            continue;
                        }
                        let cell = *key.get(&ch).ok_or_else(|| at(format!("`{ch}` is not in the key")))?;
                        d.cells.insert(IVec3::new(corner.0 + i as i32, *y, corner.1 + *row), cell);
                    }
                    *row += 1;
                }
                Section::Fine => {
                    let mut words = body.split_whitespace();
                    let v: Option<Vec<i32>> = words.by_ref().take(6).map(|w| w.parse().ok()).collect();
                    let v = v.filter(|v| v.len() == 6).ok_or_else(|| at("a fine part starts with six whole numbers, its two corners".into()))?;
                    let word = words.next().ok_or_else(|| at("a fine part needs its material after its corners".into()))?;
                    let cell = Cell::parse(word).ok_or_else(|| at(format!("`{word}` is not a role or a material")))?;
                    let mut f = Fine { lo: IVec3::new(v[0], v[1], v[2]), hi: IVec3::new(v[3], v[4], v[5]), cell, sway_lo: false, sway_hi: false, thin: 0, season: 0 };
                    while let Some(w) = words.next() {
                        let mut number = || words.next().and_then(|w| w.parse().ok()).ok_or_else(|| at(format!("`{w}` needs a number after it")));
                        match w {
                            "sway-low" => f.sway_lo = true,
                            "sway-high" => f.sway_hi = true,
                            "thin" => f.thin = number()?,
                            "season" => f.season = number()?,
                            _ => return Err(at(format!("`{w}` is not something a fine part can have"))),
                        }
                    }
                    if !f.lo.cmplt(f.hi).all() {
                        return Err(at("a fine part's second corner must be beyond its first on every axis".into()));
                    }
                    d.fine.push(f);
                }
                Section::Clear => {
                    let v = numbers(body).filter(|v| v.len() == 6).ok_or_else(|| at("a box to keep clear is six whole numbers, its two corners".into()))?;
                    d.clear.push((IVec3::new(v[0], v[1], v[2]).min(IVec3::new(v[3], v[4], v[5])), IVec3::new(v[0], v[1], v[2]).max(IVec3::new(v[3], v[4], v[5]))));
                }
            }
        }
        d.kind = kind.ok_or("the file does not say its kind, as in `kind: shrine`")?;
        if d.name.is_empty() {
            d.name = "unnamed".into();
        }
        Ok(d)
    }
}

/// Characters for materials in a file's key: none of them a role's, `.`, `_` or `#`.
const MATERIAL_SYMBOLS: &str = "abcdefghijklmnopqrstuvwxyz0123456789!$%&*+=?@^~<>()[]{}/|;,'\"`-\
    ÀÁÂÃÄÅÆÇÈÉÊËÌÍÎÏÐÑÒÓÔÕÖØÙÚÛÜÝÞàáâãäåæçèéêëìíîïðñòóôõöøùúûüýþÿ";

fn numbers(s: &str) -> Option<Vec<i32>> {
    s.split_whitespace().map(|w| w.parse().ok()).collect()
}

fn one_line(s: &str) -> String {
    s.replace(['\n', '\r', '#'], " ")
}

/// A name made safe for a file name: lower case letters, digits and dashes.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    // generator.txt is the generator's own weight, not a structure.
    match out.as_str() {
        "" => "structure".into(),
        "generator" => "generator-structure".into(),
        _ => out,
    }
}

/// How often the generator's own variants are chosen for a kind against its saved
/// structures, from a `generator.txt`: its `weight:` line.
pub fn parse_generator(text: &str) -> Result<u32, String> {
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("");
        if let Some((head, value)) = line.split_once(':')
            && head.trim().eq_ignore_ascii_case("weight")
        {
            return value.trim().parse().map_err(|_| format!("the weight should be a whole number, not `{}`", value.trim()));
        }
    }
    Err("no `weight:` line".into())
}

/// A `generator.txt` giving the generator `weight`.
pub fn generator_text(weight: u32) -> String {
    format!("# How often the generator's own variants of this kind are chosen against the structures saved here.\n# 0 means only the saved ones are built.\nweight: {weight}\n")
}

/// The saved structures of one kind, and the generator's share against them.
#[derive(Clone, Default, Debug)]
pub struct Choices {
    pub generator: Option<u32>,
    pub designs: Vec<Design>,
}

impl Choices {
    /// The generator's weight: as its file says, or none once a structure is saved.
    pub fn generator_weight(&self) -> u32 {
        self.generator.unwrap_or(if self.designs.is_empty() { 1 } else { 0 })
    }

    /// The saved structure a site of this kind with `seed` is built from, or None for
    /// the generator. Each is chosen in proportion to its weight.
    pub fn choose(&self, kind: Landmark, seed: u32) -> Option<&Design> {
        let total: u64 = self.generator_weight() as u64 + self.designs.iter().map(|d| d.weight as u64).sum::<u64>();
        if total == 0 || self.designs.is_empty() {
            return None;
        }
        let mut r = hash2(seed ^ 0xde51_9e5f, kind as i32, 0) as u64 % total;
        for d in &self.designs {
            if r < d.weight as u64 {
                return Some(d);
            }
            r -= d.weight as u64;
        }
        None
    }
}

/// Reads a structures folder's files, given as (path under the folder, text) pairs, into
/// the choices for each kind. Paths are `<kind>/<name>.txt`; anything else, and any file
/// that cannot be read, goes into the list of problems with what is wrong with it.
pub fn gather<'a>(files: impl IntoIterator<Item = (&'a str, &'a str)>) -> (HashMap<Landmark, Choices>, Vec<(String, String)>) {
    let mut out: HashMap<Landmark, Choices> = HashMap::new();
    let mut problems = Vec::new();
    let mut files: Vec<(&str, &str)> = files.into_iter().collect();
    // In a fixed order, so the world does not depend on how a folder was listed.
    files.sort_unstable();
    for (path, text) in files {
        let path = path.replace('\\', "/");
        let Some((folder, file)) = path.split_once('/') else { continue };
        let Some(stem) = file.strip_suffix(".txt") else { continue };
        let Some(kind) = kind_by_word(folder) else {
            problems.push((path.clone(), format!("`{folder}` is not a kind of structure that can be saved")));
            continue;
        };
        if stem.eq_ignore_ascii_case("generator") {
            match parse_generator(text) {
                Ok(w) => out.entry(kind).or_default().generator = Some(w),
                Err(e) => problems.push((path.clone(), e)),
            }
            continue;
        }
        match Design::parse(text) {
            Ok(d) if d.kind != kind => problems.push((path.clone(), format!("it says it is a {}, but is in the {folder} folder", kind_word(d.kind)))),
            Ok(d) => out.entry(kind).or_default().designs.push(d),
            Err(e) => problems.push((path.clone(), e)),
        }
    }
    (out, problems)
}

mod saved {
    // FILES: every structure file under assets/structures, gathered by build.rs.
    include!(concat!(env!("OUT_DIR"), "/saved_structures.rs"));
}

/// The structures compiled into this build, by kind.
pub fn library() -> &'static HashMap<Landmark, Choices> {
    static LIBRARY: OnceLock<HashMap<Landmark, Choices>> = OnceLock::new();
    LIBRARY.get_or_init(|| gather(saved::FILES.iter().copied()).0)
}

/// Files under assets/structures this build could not read, with why.
pub fn problems() -> Vec<(String, String)> {
    gather(saved::FILES.iter().copied()).1
}

/// The saved structure a site of `kind` with `seed` is built from, or None when the
/// generator builds it.
pub fn pick(kind: Landmark, seed: u32) -> Option<&'static Design> {
    library().get(&kind)?.choose(kind, seed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::biome::Biome;

    #[test]
    fn a_copy_survives_its_file_and_builds_what_the_generator_built() {
        for kind in KINDS {
            for seed in [3, 11] {
                let p = Palette::of(Biome::Deciduous).varied(seed);
                let d = Design::copy_of(kind, &p, seed, "Test copy");
                assert!(!d.cells.is_empty(), "{kind:?} copied nothing");
                let text = d.to_text();
                let back = Design::parse(&text).unwrap_or_else(|e| panic!("{kind:?}: {e}\n{text}"));
                assert_eq!(back, d, "{kind:?} changed on the way through its file");
                // Built again for the same place, the copy is the generator's model.
                let mut want: Vec<(IVec3, Voxel)> = landmarks::generate(kind, &p, seed, 0).into_voxels();
                let mut got = d.shape(&p, seed).into_voxels();
                want.sort_unstable_by_key(|(q, _)| (q.x, q.y, q.z));
                got.sort_unstable_by_key(|(q, _)| (q.x, q.y, q.z));
                let differ = want.iter().zip(&got).filter(|(a, b)| a != b).count();
                assert_eq!(want.len(), got.len(), "{kind:?}");
                assert_eq!(differ, 0, "{kind:?}: {differ} voxels differ");
            }
        }
    }

    #[test]
    fn roles_follow_the_place() {
        let mut d = Design::new(Landmark::Shrine, "Two stones");
        d.cells.insert(IVec3::new(0, 0, 0), Cell::Role(Role::Stone));
        d.cells.insert(IVec3::new(0, 1, 0), Cell::Role(Role::Roof));
        d.cells.insert(IVec3::new(0, 2, 0), Cell::Material(mat::LANTERN));
        let desert = Palette::local(Biome::Desert, mat::SANDSTONE);
        let moor = Palette::local(Biome::Heath, mat::GRANITE);
        let at = |p: &Palette, y| d.shape(p, 1).get(IVec3::new(0, y, 0)).unwrap();
        assert_eq!(at(&desert, 0), mat::SANDSTONE);
        assert_eq!(at(&moor, 0), mat::GRANITE);
        assert_eq!(at(&desert, 1), desert.roof);
        assert_eq!(at(&moor, 2), mat::LANTERN);
    }

    #[test]
    fn hand_written_files_read() {
        let text = "kind: cairn\nname: Little cairn   # a note\nweight: 3\ncorner: -1 -1\nkey:\n  S stone\n  x LANTERN\n  _ air\nlayer -1:\n  _S_\nlayer 0:\n  .S.\n  SxS\n";
        let d = Design::parse(text).unwrap();
        assert_eq!((d.kind, d.name.as_str(), d.weight), (Landmark::Cairn, "Little cairn", 3));
        assert_eq!(d.cells.get(&IVec3::new(0, 0, 0)), Some(&Cell::Material(mat::LANTERN)));
        assert_eq!(d.cells.get(&IVec3::new(-1, -1, -1)), Some(&Cell::Air));
        assert_eq!(d.cells.get(&IVec3::new(0, 0, -1)), Some(&Cell::Role(Role::Stone)));
        assert_eq!(d.cells.len(), 7);
        let err = Design::parse("kind: cairn\nkey:\n  S stone\nlayer 0:\n  SQ\n").unwrap_err();
        assert!(err.starts_with("line 5"), "{err}");
        assert!(Design::parse("kind: landing\n").is_err());
    }

    #[test]
    fn weights_share_out_the_sites() {
        let design = |name: &str, weight| Design { weight, ..Design::new(Landmark::Shrine, name) };
        let mut c = Choices { generator: None, designs: vec![design("a", 1), design("b", 3)] };
        let count = |c: &Choices, name: &str| (0..4000).filter(|&s| c.choose(Landmark::Shrine, s).is_some_and(|d| d.name == name)).count();
        // With no generator file, saved structures take every site.
        assert!((0..400).all(|s| c.choose(Landmark::Shrine, s).is_some()));
        let (a, b) = (count(&c, "a"), count(&c, "b"));
        assert!((800..1200).contains(&a) && (2800..3200).contains(&b), "{a} {b}");
        c.generator = Some(4);
        let generator = (0..4000).filter(|&s| c.choose(Landmark::Shrine, s).is_none()).count();
        assert!((1800..2200).contains(&generator), "{generator}");
        assert_eq!(Choices::default().choose(Landmark::Shrine, 5), None);
    }

    /// A saved structure is what the world builds at a site it is picked for: the hook
    /// [`landmarks::build_placed`] uses, with its props laid about it.
    #[test]
    fn a_picked_structure_is_what_gets_built() {
        let mut d = Design::new(Landmark::Arch, "Post");
        for y in -2..6 {
            d.cells.insert(IVec3::new(0, y, 0), Cell::Role(Role::Timber));
        }
        d.cells.insert(IVec3::new(0, 6, 0), Cell::Material(mat::LANTERN));
        let choices = Choices { generator: None, designs: vec![d] };
        let palette = Palette::of(Biome::Taiga);
        let seed = 9;
        let picked = choices.choose(Landmark::Arch, seed).expect("the only structure, with no generator");
        let p = palette.varied(seed);
        let (voxels, _, _) = landmarks::finish(Landmark::Arch, &p, seed, 0, picked.shape(&p, seed));
        let at: HashMap<IVec3, Voxel> = voxels.into_iter().collect();
        assert_eq!(at.get(&IVec3::new(0, 3, 0)), Some(&p.wood));
        assert_eq!(at.get(&IVec3::new(0, 6, 0)), Some(&mat::LANTERN));
    }

    /// Every file in assets/structures reads, so a slip in a hand edit is caught here and
    /// not by a structure silently going missing from the world.
    #[test]
    fn every_saved_structure_reads() {
        let problems = problems();
        assert!(problems.is_empty(), "{problems:?}");
    }
}
