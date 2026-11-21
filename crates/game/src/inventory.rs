//! The pack: what the player has gathered on their way, and the window that shows it.
//!
//! A pack is a few rows of slots, each holding a stack of one kind of thing. Things are
//! gathered with the interact key from what grows within reach. The pack remembers where
//! and when, so a picked flower stays gone, and grows back a year or two on. The window
//! shows the slots as a grid of small pictures drawn in squares, as everything here is:
//! drag one onto another to move, join or swap them, click one to read about it or let
//! it go. Seven more slots are the hotbar, always in view along the foot of the screen:
//! one of them is held, and what is in it is what is set down or laid when the player
//! uses their hands. A pack is kept with its world's save.
//!
//! Two things gathered make a third: resin dragged onto a sweet herb, or the herb onto
//! the resin, rolls them into sticks of incense (#40), to be lit at a shrine.

use std::collections::{HashMap, VecDeque};

use voxel::Voxel;
use worldgen::{features::plants::Flower, materials as mat};

/// Slots in a pack, and how many across the window shows them.
pub const SLOTS: usize = 24;
pub const ACROSS: usize = 6;
/// Slots in the hotbar. They come after the pack's own, so slot `SLOTS + k` is the
/// hotbar's `k`th.
pub const HOTBAR: usize = 7;
const ALL: usize = SLOTS + HOTBAR;
/// Most of one kind a slot holds.
pub const STACK: u32 = 99;
/// How many gathered spots are remembered; the oldest are forgotten first (and so grow
/// back early), by which time they are a long walk behind.
const REMEMBERED: usize = 20_000;

/// How a thing is drawn in its slot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    Flower,
    Spike,
    Sprig,
    Berries,
    Mushroom,
    Reed,
    Stick,
    Fire,
    Lantern,
    /// Beads of resin from a trunk (#40).
    Resin,
    /// Sticks of incense.
    Incense,
}

impl Shape {
    // There were tents once: a thing saved as one no longer reads, and is left out.
    const ALL: [(Shape, &'static str); 11] = [
        (Shape::Flower, "flower"),
        (Shape::Spike, "spike"),
        (Shape::Sprig, "sprig"),
        (Shape::Berries, "berries"),
        (Shape::Mushroom, "mushroom"),
        (Shape::Reed, "reed"),
        (Shape::Stick, "stick"),
        (Shape::Fire, "fire"),
        (Shape::Lantern, "lantern"),
        (Shape::Resin, "resin"),
        (Shape::Incense, "incense"),
    ];

    pub(crate) fn word(self) -> &'static str {
        Self::ALL.iter().find(|s| s.0 == self).expect("every shape has a word").1
    }

    pub(crate) fn from_word(word: &str) -> Option<Self> {
        Self::ALL.iter().find(|s| s.1 == word).map(|s| s.0)
    }

    /// The picture, eight squares each way: `c` the thing's own colour, `d` a darker
    /// shade of it, `l` a lighter, `y` a yellow heart, `g` and `h` stem and leaf greens,
    /// `w` a pale stalk, `b` brown.
    pub(crate) fn picture(self) -> [&'static str; 8] {
        match self {
            Shape::Flower => ["........", "..c..c..", ".cclcc..", "..cyyc..", ".ccycc..", "..c.gc..", "...gh...", "...g...."],
            Shape::Spike => ["...l....", "...c....", "..ccd...", "...cc...", "..dcc...", "...g.h..", "...gh...", "...g...."],
            Shape::Sprig => ["........", "..c..l..", ".cg.cg..", "..gcg...", ".c.g.c..", "..hgh...", "...g....", "...g...."],
            Shape::Berries => ["........", "...h....", "..hhg...", "....g...", "..lc.c..", ".ccdccd.", "..d..d..", "........"],
            Shape::Mushroom => ["........", "..cccc..", ".clccdc.", ".cccccd.", "...ww...", "...ww...", "...ww...", "..hwwh.."],
            Shape::Reed => ["...b....", "...b....", "...c....", "...c....", "...d..h.", "...g.h..", "..hgh...", "...g...."],
            Shape::Stick => ["........", "......l.", ".d...c..", "..c.c...", "...cd...", "..c..d..", ".c....d.", "........"],
            Shape::Fire => ["........", "...y....", "..yy.y..", "..yyyy..", ".cyyyyc.", ".dccccd.", "wdwddwdw", "........"],
            Shape::Lantern => ["...dd...", "..d..d..", "..dddd..", "..lyyc..", "..lyyc..", "..lyyc..", "..dddd..", "........"],
            Shape::Resin => ["........", "...l....", "..lcc...", "..ccd...", "...d..l.", ".....lc.", "....ccd.", ".....d.."],
            Shape::Incense => ["..l..l.l", "..c.c..c", "..cc..c.", "...c.c..", "...ccc..", "...bbb..", "...b.b..", "..b...b."],
        }
    }
}

/// A kind of thing that can be carried.
#[derive(Clone, PartialEq, Debug)]
pub struct Item {
    /// What it is called in a save, the same in every version of the game.
    pub id: String,
    pub name: String,
    /// A line about it.
    pub about: String,
    /// sRGB.
    pub color: [u8; 3],
    pub shape: Shape,
}

impl Item {
    pub fn new(id: &str, name: &str, about: &str, color: [u8; 3], shape: Shape) -> Self {
        Self { id: id.into(), name: name.into(), about: about.into(), color, shape }
    }
}

/// A flower or a mushroom, picked.
pub fn of_flower(f: Flower) -> Item {
    let (about, shape) = match f {
        Flower::Pimpernel => ("It shuts before rain, so they say, and is seldom wrong.", Shape::Flower),
        Flower::Cinquefoil => ("Five petals and five leaves to a hand, low in the grass.", Shape::Flower),
        Flower::Stitchwort => ("White stars on stems too thin to hold them up alone.", Shape::Flower),
        Flower::Violet => ("Sweet for a breath, and then the scent is gone.", Shape::Flower),
        Flower::Speedwell => ("The colour of the sky an hour after rain.", Shape::Flower),
        Flower::Campion => ("Soft pink, and out from spring until the frost.", Shape::Flower),
        Flower::Heather => ("A sprig from the open moor. It keeps its colour dried.", Shape::Sprig),
        Flower::WaterLily => ("Lifted dripping from the still water.", Shape::Flower),
        Flower::FlyAgaric => ("Red and white-spotted. For looking at, not for eating.", Shape::Mushroom),
        Flower::PennyBun => ("Fat and brown as a loaf, and as good.", Shape::Mushroom),
        Flower::StGeorge => ("A firm cream cap, up in the grass when the year is young.", Shape::Mushroom),
        Flower::Chanterelle => ("Gold as an egg yolk, and smelling of apricots.", Shape::Mushroom),
        Flower::Bluebell => ("One bell from a wood that was blue with them.", Shape::Spike),
        Flower::Foxglove => ("A spire of speckled bells. The bees climb right inside.", Shape::Spike),
        Flower::Primrose => ("Pale as butter, and among the first of the year.", Shape::Flower),
        Flower::Daisy => ("It opens with the day and shuts at dusk.", Shape::Flower),
        Flower::Buttercup => ("Hold it under a chin and see the gold.", Shape::Flower),
        Flower::Poppy => ("Scarlet silk that drops its petals in a day.", Shape::Flower),
        Flower::Lupin => ("A tall spike over leaves like open hands.", Shape::Spike),
        Flower::CottonGrass => ("A white tuft from the wet moor, soft as down.", Shape::Sprig),
        Flower::MarshMarigold => ("Kingcups: gold cups from the edge of the water.", Shape::Flower),
        Flower::FlagIris => ("A yellow flag from among the sword leaves by the stream.", Shape::Flower),
        Flower::Thrift => ("A pink pompom from a cushion by the sea.", Shape::Sprig),
        Flower::Gentian => ("A blue so deep it looks cut from the evening sky.", Shape::Flower),
        Flower::Edelweiss => ("A grey felted star, found alone, high up. A thing to keep.", Shape::Flower),
        Flower::Orchid => ("Pale, with a dark lip, from the dim of the rainforest.", Shape::Spike),
        Flower::Heliconia => ("Red bracts tipped with gold, like a row of beaks.", Shape::Spike),
        Flower::DesertMarigold => ("Gold over grey wool, where little else will grow.", Shape::Flower),
        Flower::Thistle => ("A purple brush, taken with care.", Shape::Spike),
    };
    Item::new(f.id(), f.name(), about, f.color(), shape)
}

/// What can be gathered from voxel `v`, if anything: what the bushes and the reed beds
/// bear. (Flowers are not voxels: see [`of_flower`].)
pub fn of_voxel(v: Voxel, materials: &voxel::Materials) -> Option<Item> {
    let (id, name, about, shape) = match v {
        mat::GORSE_BLOOM => ("gorse-blossom", "Gorse blossom", "It smells of coconut, and was hard won from the thorns.", Shape::Sprig),
        mat::HOLLY_BERRY => ("holly-berries", "Holly berries", "For looking at, not for eating.", Shape::Berries),
        mat::IVY_BERRY => ("ivy-berries", "Ivy berries", "Ripe in the dead of winter, when the birds need them.", Shape::Berries),
        mat::REED_HEAD => ("reed-mace", "Reed mace", "A velvet head from the water's edge.", Shape::Reed),
        _ => return None,
    };
    Some(Item::new(id, name, about, materials.get(v).color, shape))
}

/// Dry wood for a fire, gathered from under the trees.
pub fn sticks() -> Item {
    Item::new("sticks", "Sticks", "Dry wood from under a tree. Three will lay a fire.", [150, 116, 80], Shape::Stick)
}

/// Resin from a wounded trunk (#40): a bead of it here and there on the pines and the
/// junipers, the acacias of the dry country and the great trees of the rainforest.
pub fn resin() -> Item {
    Item::new("resin", "Resin", "A bead of gum from a wounded trunk, sticky and sweet-smelling. Rolled with a sweet herb it makes incense.", [214, 150, 52], Shape::Resin)
}

/// Sticks of incense, made in the pack from resin and a sweet herb (#40), to be lit at
/// a shrine's burner.
pub fn incense() -> Item {
    Item::new("incense", "Incense", "Resin rolled with a sweet herb onto slips of wood. It is lit at a shrine.", [150, 84, 62], Shape::Incense)
}

/// The herbs sweet enough to make incense with: violets and heather and gorse blossom
/// in most country, marigolds where it is dry, as people lay at a shrine, and orchids in
/// the rainforest, where one of them is vanilla.
pub fn sweet(id: &str) -> bool {
    matches!(id, "violet" | "heather" | "gorse-blossom" | "desert_marigold" | "orchid")
}

/// How many sticks of incense one resin and one sweet herb make.
pub const INCENSE_MADE: u32 = 2;

/// The lantern every wanderer sets out with, to light the way after dark.
pub fn lantern() -> Item {
    Item::new("lantern", "Lantern", "Brass and glass about a candle: a warm light to walk by at night.", [176, 136, 72], Shape::Lantern)
}

/// What a wanderer sets out with, in the order it was first handed out: a pack is given
/// whatever of it came after what it already had (see `Pack::give_kit`). Kit 1 was a
/// tent, which there are no longer; 2 is the lantern.
const KIT: u8 = 2;

/// A fire laid in a ring of stones. It is never carried: it is what three sticks become.
pub fn campfire() -> Item {
    Item::new("campfire", "Campfire", "A ring of stones, and a fire laid in it.", [236, 132, 52], Shape::Fire)
}

/// How many sticks lay a fire.
pub const FIRE_STICKS: u32 = 3;

/// What using the thing called `id` does, in a few words, for the line under its name.
pub fn use_of(id: &str) -> &'static str {
    match id {
        "sticks" => "right click to lay a fire",
        "lantern" => "lit while it is held; right click to shutter it",
        "incense" => "right click at a shrine's burner to light a stick",
        "resin" => "drag it onto a sweet herb in the pack to make incense",
        _ => "right click to set it down",
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Stack {
    pub item: Item,
    pub count: u32,
}

pub struct Pack {
    slots: Vec<Option<Stack>>,
    /// The columns of the world gathered from and not yet grown back, each with when it
    /// was (in `years`), and the order they were.
    gathered: HashMap<(i32, i32), f64>,
    order: VecDeque<(i32, i32)>,
    /// How long this world has been walked, in its own years: the clock that what was
    /// gathered grows back by.
    years: f64,
    /// Which slot of the hotbar is held, counting from 0.
    held: usize,
    /// How much of what a wanderer sets out with has been handed over: 1 the tent there
    /// once was, 2 the lantern.
    kit: u8,
    /// Changed since it was last saved.
    pub unsaved: bool,
}

impl Default for Pack {
    fn default() -> Self {
        Self::new()
    }
}

impl Pack {
    pub fn new() -> Self {
        Self { slots: vec![None; ALL], gathered: HashMap::new(), order: VecDeque::new(), years: 0.0, held: 0, kit: 0, unsaved: false }
    }

    /// Every slot: the pack's own, then the hotbar's.
    pub fn slots(&self) -> &[Option<Stack>] {
        &self.slots
    }

    /// The hotbar's slots.
    pub fn hotbar(&self) -> &[Option<Stack>] {
        &self.slots[SLOTS..]
    }

    /// Which slot of the hotbar is held, counting from 0.
    pub fn held(&self) -> usize {
        self.held
    }

    /// The slot, among all of them, that is held.
    pub fn held_slot(&self) -> usize {
        SLOTS + self.held
    }

    /// What is held, if the held slot is not empty.
    pub fn in_hand(&self) -> Option<&Stack> {
        self.slots[self.held_slot()].as_ref()
    }

    /// Holds slot `k` of the hotbar. Whether that is a change.
    pub fn hold(&mut self, k: usize) -> bool {
        let k = k.min(HOTBAR - 1);
        let changed = k != self.held;
        self.held = k;
        self.unsaved |= changed;
        changed
    }

    /// Holds the slot `by` along from the one held, round the end and back.
    pub fn cycle(&mut self, by: i32) -> bool {
        self.hold((self.held as i32 + by).rem_euclid(HOTBAR as i32) as usize)
    }

    /// Hands over what a wanderer sets out with, each thing once in the life of a pack:
    /// to a new pack, or to one from before there were lanterns.
    pub fn give_kit(&mut self) {
        if self.kit < 2 {
            // At the far end of the hotbar, out of the way of what is picked, if it is free.
            let last = ALL - 1;
            if self.slots[last].is_none() {
                self.slots[last] = Some(Stack { item: lantern(), count: 1 });
            } else {
                self.add(&lantern());
            }
        }
        if self.kit < KIT {
            self.kit = KIT;
            self.unsaved = true;
        }
    }

    /// How many of the thing called `id` are carried.
    pub fn count(&self, id: &str) -> u32 {
        self.slots.iter().flatten().filter(|s| s.item.id == id).map(|s| s.count).sum()
    }

    /// Puts one of `item` in: on a stack of its kind with room, else in the first empty
    /// slot of the hotbar, else of the pack. False if there is no room anywhere.
    pub fn add(&mut self, item: &Item) -> bool {
        // The hotbar is looked over first, then the pack.
        let order = || (SLOTS..ALL).chain(0..SLOTS);
        let stack = order().find(|&i| self.slots[i].as_ref().is_some_and(|s| s.item.id == item.id && s.count < STACK));
        let Some(at) = stack.or_else(|| order().find(|&i| self.slots[i].is_none())) else { return false };
        match &mut self.slots[at] {
            Some(s) => s.count += 1,
            empty => *empty = Some(Stack { item: item.clone(), count: 1 }),
        }
        self.unsaved = true;
        true
    }

    /// Puts `n` of `item` in, as far as there is room. How many would not go.
    pub fn add_many(&mut self, item: &Item, n: u32) -> u32 {
        (0..n).filter(|_| !self.add(item)).count() as u32
    }

    /// Takes `n` of the thing called `id` out, from wherever it is carried, the held
    /// slot first. False, and nothing taken, if there are not so many.
    pub fn spend(&mut self, id: &str, n: u32) -> bool {
        if self.count(id) < n {
            return false;
        }
        let mut left = n;
        for at in std::iter::once(self.held_slot()).chain(SLOTS..ALL).chain(0..SLOTS) {
            if let Some(s) = &self.slots[at]
                && s.item.id == id
            {
                let take = s.count.min(left);
                self.drop_from(at, take);
                left -= take;
            }
            if left == 0 {
                break;
            }
        }
        true
    }

    /// Lets go of `n` from slot `at`, or all there is.
    pub fn drop_from(&mut self, at: usize, n: u32) {
        let Some(slot) = self.slots.get_mut(at) else { return };
        if let Some(s) = slot {
            s.count = s.count.saturating_sub(n);
            if s.count == 0 {
                *slot = None;
            }
            self.unsaved = true;
        }
    }

    /// Moves what is in slot `from` to slot `to`: onto a stack of the same kind as far
    /// as it has room, else changing places with what is there. Resin onto a sweet herb,
    /// or a sweet herb onto resin, makes incense instead (see [`Self::blend`]).
    pub fn shift(&mut self, from: usize, to: usize) {
        if from == to || from >= ALL || to >= ALL || self.slots[from].is_none() {
            return;
        }
        if self.blend(from, to) {
            return;
        }
        let same = matches!((&self.slots[from], &self.slots[to]), (Some(a), Some(b)) if a.item.id == b.item.id);
        if same {
            let room = STACK - self.slots[to].as_ref().expect("checked").count;
            let moved = self.slots[from].as_ref().expect("checked").count.min(room);
            self.slots[to].as_mut().expect("checked").count += moved;
            self.drop_from(from, moved);
        } else {
            self.slots.swap(from, to);
        }
        self.unsaved = true;
    }

    /// Rolls one of the resin and one of the sweet herb in slots `from` and `to` (either
    /// way round) into [`INCENSE_MADE`] sticks of incense: into the slot dragged onto if
    /// that has been used up, else wherever they fit. Whether they were made; nothing is
    /// used if there is no room for them.
    pub fn blend(&mut self, from: usize, to: usize) -> bool {
        let (Some(Some(a)), Some(Some(b))) = (self.slots.get(from), self.slots.get(to)) else { return false };
        let (a, b) = (a.item.id.as_str(), b.item.id.as_str());
        if !(a == "resin" && sweet(b) || sweet(a) && b == "resin") {
            return false;
        }
        let before = self.slots.clone();
        self.drop_from(from, 1);
        self.drop_from(to, 1);
        if self.slots[to].is_none() {
            self.slots[to] = Some(Stack { item: incense(), count: 0 });
        }
        let made = match &mut self.slots[to] {
            Some(s) if s.item.id == "incense" => {
                s.count += INCENSE_MADE;
                true
            }
            _ => self.add_many(&incense(), INCENSE_MADE) == 0,
        };
        if !made {
            self.slots = before;
        }
        self.unsaved = true;
        made
    }

    /// Whether the column at `(x, z)` has been gathered from.
    pub fn gathered(&self, x: i32, z: i32) -> bool {
        self.gathered.contains_key(&(x, z))
    }

    /// Every column gathered from and not yet grown back.
    pub fn spots(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        self.order.iter().copied()
    }

    /// Remembers that the column at `(x, z)` has been gathered from, now. The spot
    /// forgotten to make room for it, if one was, which is free to grow back.
    pub fn mark(&mut self, x: i32, z: i32) -> Option<(i32, i32)> {
        self.mark_at(x, z, self.years)
    }

    fn mark_at(&mut self, x: i32, z: i32, when: f64) -> Option<(i32, i32)> {
        if self.gathered.insert((x, z), when).is_some() {
            return None;
        }
        self.unsaved = true;
        self.order.push_back((x, z));
        let old = (self.order.len() > REMEMBERED).then(|| self.order.pop_front()).flatten()?;
        self.gathered.remove(&old);
        Some(old)
    }

    /// How long after it is gathered the spot at `(x, z)` has grown back, in years: one
    /// to two, each spot its own.
    pub fn grows_back_after(x: i32, z: i32) -> f64 {
        1.0 + worldgen::noise::unit(worldgen::noise::hash3(0x9e0b, x, 0, z)) as f64
    }

    /// Moves the world's clock on by `years`, and takes out of the gathered spots those
    /// that have grown back by now, which it returns.
    pub fn pass(&mut self, years: f64) -> Vec<(i32, i32)> {
        if self.gathered.is_empty() {
            return Vec::new();
        }
        self.years += years;
        self.unsaved |= years > 0.0;
        let now = self.years;
        let back: Vec<(i32, i32)> = self.gathered.iter().filter(|&(&(x, z), &when)| now >= when + Self::grows_back_after(x, z)).map(|(at, _)| *at).collect();
        if !back.is_empty() {
            for at in &back {
                self.gathered.remove(at);
            }
            let gathered = &self.gathered;
            self.order.retain(|at| gathered.contains_key(at));
        }
        back
    }

    /// The pack as a save holds it: a line to a slot, each telling all there is to know
    /// of what is in it, so that a thing from a newer or older game still reads.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for (i, s) in self.slots.iter().enumerate() {
            if let Some(s) = s {
                // The hotbar's slots have their own word, so a pack from before there
                // was a hotbar reads as it always did.
                let slot = if i < SLOTS { format!("slot {i}") } else { format!("hot {}", i - SLOTS) };
                out += &format!("{slot} = {}\n", stack_text(s));
            }
        }
        if self.held != 0 {
            out += &format!("held = {}\n", self.held);
        }
        if self.kit > 0 {
            out += &format!("kit = {}\n", self.kit);
        }
        if !self.order.is_empty() {
            out += &format!("years = {}\n", self.years);
        }
        for chunk in self.order.iter().collect::<Vec<_>>().chunks(64) {
            let spots: Vec<String> = chunk.iter().map(|at| format!("{},{},{}", at.0, at.1, self.gathered[*at])).collect();
            out += &format!("gathered = {}\n", spots.join(";"));
        }
        out
    }

    /// Reads a pack back, leaving out any line that does not make sense.
    pub fn from_text(text: &str) -> Self {
        let mut pack = Self::new();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            let (key, value) = (key.trim(), value.trim());
            if key == "held" {
                pack.held = value.parse::<usize>().ok().filter(|k| *k < HOTBAR).unwrap_or(0);
            } else if key == "kit" {
                pack.kit = value.parse::<u8>().unwrap_or(0).min(KIT);
            } else if key == "years" {
                pack.years = value.parse::<f64>().ok().filter(|y| y.is_finite() && *y >= 0.0).unwrap_or(0.0);
            } else if key == "gathered" {
                // Each spot with when it was gathered; an older save has only the spot.
                for spot in value.split(';') {
                    let mut parts = spot.split(',').map(str::trim);
                    if let (Some(x), Some(z)) = (parts.next(), parts.next())
                        && let (Ok(x), Ok(z)) = (x.parse(), z.parse())
                    {
                        let when = parts.next().and_then(|w| w.parse::<f64>().ok()).filter(|w| w.is_finite()).unwrap_or(0.0);
                        pack.mark_at(x, z, when);
                    }
                }
            } else if let Some(at) = key
                .strip_prefix("slot ")
                .and_then(|n| n.trim().parse::<usize>().ok())
                .filter(|at| *at < SLOTS)
                .or_else(|| key.strip_prefix("hot ").and_then(|n| n.trim().parse::<usize>().ok()).filter(|k| *k < HOTBAR).map(|k| SLOTS + k))
                && let Some(stack) = stack_from_text(value)
            {
                pack.slots[at] = Some(stack);
            }
        }
        pack.unsaved = false;
        pack
    }
}

/// A stack as a save holds it: how many, then all there is to know of the thing, so
/// that one from a newer or older game still reads.
pub fn stack_text(s: &Stack) -> String {
    let clean = |s: &str| s.replace(['|', '\n', '\r'], " ");
    let [r, g, b] = s.item.color;
    format!("{}|{}|{r},{g},{b}|{}|{}|{}", s.count, s.item.shape.word(), clean(&s.item.id), clean(&s.item.name), clean(&s.item.about))
}

/// Reads back what [`stack_text`] wrote, or nothing if it does not make sense.
pub fn stack_from_text(text: &str) -> Option<Stack> {
    let parts: Vec<&str> = text.splitn(6, '|').collect();
    let [count, shape, color, id, name, about] = parts[..] else { return None };
    let rgb: Vec<u8> = color.split(',').filter_map(|v| v.trim().parse().ok()).collect();
    let count = count.trim().parse::<u32>().ok().filter(|c| *c > 0)?.min(STACK);
    let item = Item::new(id.trim(), name.trim(), about.trim(), <[u8; 3]>::try_from(rgb).ok()?, Shape::from_word(shape.trim())?);
    (!item.id.is_empty()).then_some(Stack { item, count })
}

/// What the pack's window remembers from frame to frame.
#[derive(Default)]
pub struct View {
    selected: Option<usize>,
    dragging: Option<usize>,
}

impl View {
    /// Picks out the thing in slot `at`.
    pub fn select(&mut self, at: usize) {
        self.selected = Some(at);
    }
}

const PANEL: egui::Color32 = egui::Color32::from_rgba_premultiplied(22, 27, 34, 238);
const SLOT: egui::Color32 = egui::Color32::from_rgb(44, 52, 62);
const SLOT_LIT: egui::Color32 = egui::Color32::from_rgb(60, 70, 82);
const EDGE: egui::Color32 = egui::Color32::from_rgb(86, 98, 112);
const PICKED: egui::Color32 = egui::Color32::from_rgb(236, 214, 150);
const INK: egui::Color32 = egui::Color32::from_rgb(240, 236, 226);
const FAINT: egui::Color32 = egui::Color32::from_rgb(160, 168, 176);

/// Draws a thing's picture in `rect`.
pub(crate) fn picture(painter: &egui::Painter, rect: egui::Rect, item: &Item, fade: f32) {
    let [r, g, b] = item.color;
    let tone = |by: f32| {
        let v = |c: u8| (c as f32 * by).round().clamp(0.0, 255.0) as u8;
        egui::Color32::from_rgb(v(r), v(g), v(b))
    };
    let side = (rect.width().min(rect.height()) / 8.0).floor().max(1.0);
    let corner = rect.center() - egui::vec2(side * 4.0, side * 4.0);
    for (row, line) in item.shape.picture().iter().enumerate() {
        for (col, ch) in line.chars().enumerate() {
            let color = match ch {
                'c' => tone(1.0),
                'd' => tone(0.74),
                'l' => tone(1.22),
                'y' => egui::Color32::from_rgb(246, 214, 92),
                'g' => egui::Color32::from_rgb(86, 138, 70),
                'h' => egui::Color32::from_rgb(112, 164, 84),
                'w' => egui::Color32::from_rgb(228, 218, 198),
                'b' => egui::Color32::from_rgb(112, 84, 54),
                _ => continue,
            };
            let at = corner + egui::vec2(col as f32 * side, row as f32 * side);
            painter.rect_filled(egui::Rect::from_min_size(at, egui::vec2(side, side)), 0.0, color.gamma_multiply(fade));
        }
    }
}

/// Draws slot `at` of `pack` in the pack's window, `side` across, and takes what the
/// mouse does to it. Its place goes in `rects`, for a drag to land on.
fn slot(ui: &mut egui::Ui, pack: &Pack, view: &mut View, at: usize, side: f32, rects: &mut Vec<(usize, egui::Rect)>) {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click_and_drag());
    rects.push((at, rect));
    let painter = ui.painter();
    let stack = pack.slots()[at].as_ref();
    let lit = response.hovered() || view.dragging.is_some_and(|_| ui.rect_contains_pointer(rect));
    painter.rect_filled(rect, 7.0, if lit { SLOT_LIT } else { SLOT });
    let edge = if view.selected == Some(at) && stack.is_some() { egui::Stroke::new(2.0, PICKED) } else { egui::Stroke::new(1.0, EDGE) };
    painter.rect_stroke(rect, 7.0, edge, egui::StrokeKind::Inside);
    if at >= SLOTS {
        // A hotbar slot shows the key that holds it, and the held one a bar under it.
        painter.text(rect.left_top() + egui::vec2(5.0, 3.0), egui::Align2::LEFT_TOP, at - SLOTS + 1, egui::FontId::proportional(11.0), FAINT);
        if at == pack.held_slot() {
            let bar = egui::Rect::from_min_max(rect.left_bottom() + egui::vec2(8.0, 3.0), rect.right_bottom() + egui::vec2(-8.0, 6.0));
            painter.rect_filled(bar, 1.5, PICKED);
        }
    }
    if let Some(s) = stack {
        // What is being dragged is drawn under the cursor, and faintly at home.
        let fade = if view.dragging == Some(at) { 0.3 } else { 1.0 };
        picture(painter, rect.shrink(9.0), &s.item, fade);
        count(painter, rect, s.count, 14.0);
        if response.drag_started() {
            view.dragging = Some(at);
        }
        if response.clicked() {
            view.selected = Some(at);
        }
        if view.dragging.is_none() {
            response.on_hover_text(&s.item.name);
        }
    } else if response.clicked() {
        view.selected = None;
    }
}

/// How many are in a slot, in its corner, if more than one.
fn count(painter: &egui::Painter, rect: egui::Rect, count: u32, size: f32) {
    if count > 1 {
        let corner = rect.right_bottom() - egui::vec2(6.0, 4.0);
        painter.text(corner + egui::vec2(1.0, 1.0), egui::Align2::RIGHT_BOTTOM, count, egui::FontId::proportional(size), egui::Color32::from_black_alpha(200));
        painter.text(corner, egui::Align2::RIGHT_BOTTOM, count, egui::FontId::proportional(size), INK);
    }
}

/// Shows the pack's window, and does to `pack` what is done in it. What was let go of
/// in it comes back, for the caller to set down in the world.
pub fn show(ctx: &egui::Context, pack: &mut Pack, view: &mut View) -> Vec<Stack> {
    let side = 58.0;
    let gap = 7.0;
    let mut rects = Vec::with_capacity(ALL);
    let mut let_go = Vec::new();
    egui::Area::new(egui::Id::unique("pack")).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).fade_in(false).show(ctx, |ui| {
        egui::Frame::new().fill(PANEL).stroke(egui::Stroke::new(1.0, EDGE)).corner_radius(10).inner_margin(egui::Margin::same(18)).show(ui, |ui| {
            let width = HOTBAR as f32 * side + (HOTBAR - 1) as f32 * gap;
            ui.set_width(width);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Pack").size(22.0).color(INK));
                let carried: u32 = pack.slots().iter().flatten().map(|s| s.count).sum();
                let kinds = pack.slots().iter().flatten().count();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(format!("{carried} things in {kinds} of {ALL} slots")).size(13.0).color(FAINT));
                });
            });
            ui.add_space(10.0);
            // The pack's rows sit in the middle, over the hotbar's one wider row.
            ui.horizontal(|ui| {
                ui.add_space((HOTBAR - ACROSS) as f32 * (side + gap) / 2.0);
                egui::Grid::new("pack slots").spacing([gap, gap]).show(ui, |ui| {
                    for at in 0..SLOTS {
                        slot(ui, pack, view, at, side, &mut rects);
                        if at % ACROSS == ACROSS - 1 {
                            ui.end_row();
                        }
                    }
                });
            });
            ui.add_space(12.0);
            ui.label(egui::RichText::new("Hotbar").size(13.0).color(FAINT));
            ui.add_space(2.0);
            egui::Grid::new("hotbar slots").spacing([gap, gap]).show(ui, |ui| {
                for at in SLOTS..ALL {
                    slot(ui, pack, view, at, side, &mut rects);
                }
            });
            ui.add_space(12.0);
            // The thing picked out: what it is, and letting it go.
            let chosen = view.selected.and_then(|at| Some((at, pack.slots()[at].clone()?)));
            ui.allocate_ui(egui::vec2(width, 62.0), |ui| match chosen {
                Some((at, s)) => {
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(34.0, 34.0), egui::Sense::hover());
                        picture(ui.painter(), rect, &s.item, 1.0);
                        ui.label(egui::RichText::new(format!("{}  \u{d7} {}", s.item.name, s.count)).size(16.0).color(INK));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Set all down").clicked() {
                                pack.drop_from(at, u32::MAX);
                                let_go.push(s.clone());
                            }
                            if ui.button("Set one down").clicked() {
                                pack.drop_from(at, 1);
                                let_go.push(Stack { item: s.item.clone(), count: 1 });
                            }
                        });
                    });
                    ui.label(egui::RichText::new(&s.item.about).size(13.0).color(FAINT));
                }
                None => {
                    let hint = if pack.slots().iter().all(Option::is_none) {
                        "Nothing yet. Press E by a flower or a mushroom to gather it."
                    } else {
                        "Click a thing to read about it. Drag it to another slot, or down to the hotbar."
                    };
                    ui.label(egui::RichText::new(hint).size(13.0).color(FAINT));
                }
            });
            ui.add_space(4.0);
            ui.label(egui::RichText::new("I or Esc to close").size(12.0).color(FAINT));
        });
    });

    // The drag: the picture rides the cursor, and lands in whichever slot it is let go over.
    if let Some(from) = view.dragging {
        let pointer = ctx.pointer_interact_pos();
        if let (Some(p), Some(s)) = (pointer, pack.slots()[from].as_ref()) {
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("pack drag")));
            picture(&painter, egui::Rect::from_center_size(p, egui::vec2(side - 18.0, side - 18.0)), &s.item, 1.0);
        }
        if ctx.input(|i| i.pointer.any_released() || !i.pointer.any_down()) {
            if let Some(&(to, _)) = pointer.and_then(|p| rects.iter().find(|r| r.1.contains(p))) {
                pack.shift(from, to);
                view.selected = Some(to);
            }
            view.dragging = None;
        }
    }
    let_go
}

/// Draws the hotbar along the foot of the screen: its seven slots, the held one picked
/// out, and over it `word` (what has just come to hand, and how much of it to show, 0
/// to 1) if there is one.
pub fn hotbar(ctx: &egui::Context, pack: &Pack, word: Option<(&str, f32)>) {
    let (side, gap, pad) = (46.0, 5.0, 6.0);
    let width = HOTBAR as f32 * side + (HOTBAR - 1) as f32 * gap + 2.0 * pad;
    let screen = ctx.content_rect();
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("hotbar")));
    let corner = egui::pos2((screen.center().x - width / 2.0).round(), screen.bottom() - side - 2.0 * pad - 10.0);
    let panel = egui::Rect::from_min_size(corner, egui::vec2(width, side + 2.0 * pad));
    painter.rect_filled(panel, 9.0, egui::Color32::from_rgba_premultiplied(18, 22, 28, 190));
    painter.rect_stroke(panel, 9.0, egui::Stroke::new(1.0, EDGE.gamma_multiply(0.7)), egui::StrokeKind::Inside);
    for (k, stack) in pack.hotbar().iter().enumerate() {
        let rect = egui::Rect::from_min_size(corner + egui::vec2(pad + k as f32 * (side + gap), pad), egui::vec2(side, side));
        let held = k == pack.held();
        painter.rect_filled(rect, 6.0, if held { SLOT_LIT } else { SLOT.gamma_multiply(0.85) });
        let edge = if held { egui::Stroke::new(2.0, PICKED) } else { egui::Stroke::new(1.0, EDGE.gamma_multiply(0.8)) };
        painter.rect_stroke(rect, 6.0, edge, egui::StrokeKind::Inside);
        painter.text(rect.left_top() + egui::vec2(4.0, 2.0), egui::Align2::LEFT_TOP, k + 1, egui::FontId::proportional(10.0), if held { PICKED } else { FAINT });
        if let Some(s) = stack {
            picture(&painter, rect.shrink(7.0), &s.item, 1.0);
            count(&painter, rect, s.count, 12.0);
        }
    }
    if let Some((text, shown)) = word {
        let at = egui::pos2(screen.center().x, panel.top() - 8.0);
        let font = egui::FontId::proportional(16.0);
        painter.text(at + egui::vec2(1.0, 1.0), egui::Align2::CENTER_BOTTOM, text, font.clone(), egui::Color32::from_black_alpha(190).gamma_multiply(shown));
        painter.text(at, egui::Align2::CENTER_BOTTOM, text, font, INK.gamma_multiply(shown));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn thing(id: &str) -> Item {
        Item::new(id, "A thing", "About it.", [200, 100, 50], Shape::Flower)
    }

    #[test]
    fn resin_dropped_on_a_sweet_herb_makes_incense_and_on_anything_else_does_not() {
        let at = |pack: &Pack, id: &str| pack.slots.iter().position(|s| s.as_ref().is_some_and(|s| s.item.id == id));
        let mut pack = Pack::new();
        pack.add(&resin());
        pack.add(&resin());
        pack.add(&thing("violet"));
        pack.add(&thing("poppy"));
        let (r, v, p) = (at(&pack, "resin").unwrap(), at(&pack, "violet").unwrap(), at(&pack, "poppy").unwrap());
        // Not with a poppy: the two only change places.
        pack.shift(r, p);
        assert_eq!(pack.count("incense"), 0);
        assert_eq!((pack.count("resin"), pack.count("poppy")), (2, 1));
        let r = at(&pack, "resin").unwrap();
        // With the violet: the one violet and one bead of resin are gone, and in the
        // violet's place is incense.
        pack.shift(r, v);
        assert_eq!((pack.count("resin"), pack.count("violet"), pack.count("incense")), (1, 0, INCENSE_MADE));
        assert_eq!(at(&pack, "incense"), Some(v));
        // The other way round makes it too, and it joins the incense made before.
        pack.add(&thing("heather"));
        let (h, r) = (at(&pack, "heather").unwrap(), at(&pack, "resin").unwrap());
        pack.shift(h, r);
        assert_eq!((pack.count("resin"), pack.count("heather"), pack.count("incense")), (0, 0, 2 * INCENSE_MADE));
        assert!(use_of("incense").contains("shrine") && use_of("resin").contains("herb"));
    }

    #[test]
    fn things_stack_by_kind_and_a_full_pack_takes_no_more() {
        let mut pack = Pack::new();
        for _ in 0..STACK + 3 {
            assert!(pack.add(&thing("poppy")));
        }
        assert!(pack.add(&thing("heather")));
        // The hotbar fills first.
        let held: Vec<(String, u32)> = pack.hotbar().iter().flatten().map(|s| (s.item.id.clone(), s.count)).collect();
        assert_eq!(held, [("poppy".into(), STACK), ("poppy".into(), 3), ("heather".into(), 1)]);
        assert!(pack.slots()[..SLOTS].iter().all(Option::is_none));
        assert_eq!(pack.count("poppy"), STACK + 3);
        // Fill every slot with something different: then only what stacks goes in.
        for k in 0..ALL {
            pack.add(&thing(&format!("kind {k}")));
        }
        assert_eq!(pack.slots()[0].as_ref().map(|s| s.item.id.as_str()), Some("kind 4"), "the pack fills after the hotbar");
        assert!(pack.slots().iter().all(Option::is_some));
        assert!(!pack.add(&thing("one more kind")));
        assert!(pack.add(&thing("heather")));
    }

    #[test]
    fn dragging_moves_joins_and_swaps() {
        let mut pack = Pack::new();
        for _ in 0..STACK + 5 {
            pack.add(&thing("poppy"));
        }
        pack.add(&thing("heather"));
        // Out of the hotbar, where they went, and into the pack's first slots.
        for k in 0..3 {
            pack.shift(SLOTS + k, k);
        }
        assert!(pack.hotbar().iter().all(Option::is_none));
        // To an empty slot.
        pack.shift(2, 10);
        assert!(pack.slots()[2].is_none() && pack.slots()[10].as_ref().is_some_and(|s| s.item.id == "heather"));
        // Onto a full stack of its own kind: nothing moves. The other way, it tops up.
        pack.shift(1, 0);
        assert_eq!((pack.slots()[0].as_ref().map(|s| s.count), pack.slots()[1].as_ref().map(|s| s.count)), (Some(STACK), Some(5)));
        pack.drop_from(0, 3);
        pack.shift(1, 0);
        assert_eq!((pack.slots()[0].as_ref().map(|s| s.count), pack.slots()[1].as_ref().map(|s| s.count)), (Some(STACK), Some(2)));
        // Onto another kind: they change places.
        pack.shift(10, 1);
        assert!(pack.slots()[1].as_ref().is_some_and(|s| s.item.id == "heather") && pack.slots()[10].as_ref().is_some_and(|s| s.item.id == "poppy"));
        // Letting the last go empties the slot.
        pack.drop_from(1, 1);
        assert!(pack.slots()[1].is_none());
        assert_eq!(pack.count("poppy"), STACK + 2);
    }

    #[test]
    fn a_pack_reads_back_as_it_was_saved_and_shrugs_off_nonsense() {
        let mut pack = Pack::new();
        pack.add(&Item::new("reed-mace", "Reed mace", "A velvet head | from the water's edge.", [112, 84, 54], Shape::Reed));
        for _ in 0..7 {
            pack.add(&thing("poppy"));
        }
        pack.shift(SLOTS, 0);
        pack.shift(SLOTS + 1, 17);
        for k in 0..150 {
            pack.mark(k * 3 - 100, -k);
        }
        let text = pack.to_text();
        let back = Pack::from_text(&format!("{text}slot 99 = 4|flower|1,2,3|x|X|\nslot 3 = lots|flower|1,2,3|x|X|\nrubbish\nslot 4 = 2|wheel|1,2,3|x|X|\n"));
        assert!(!back.unsaved);
        assert_eq!(back.slots().iter().flatten().count(), 2);
        assert_eq!(back.slots()[17].as_ref().map(|s| (s.item.id.as_str(), s.count)), Some(("poppy", 7)));
        let reed = back.slots()[0].as_ref().expect("the reed");
        assert_eq!((reed.item.shape, reed.item.color, reed.item.about.as_str()), (Shape::Reed, [112, 84, 54], "A velvet head   from the water's edge."));
        assert!(back.gathered(-100, 0) && back.gathered(347, -149) && !back.gathered(5, 5));
        assert_eq!(back.to_text(), text.replace(" | ", "   "));
        // What is in the hotbar, and which slot of it is held, come back too.
        let mut pack = Pack::new();
        pack.add(&sticks());
        pack.add(&thing("poppy"));
        pack.shift(SLOTS + 1, SLOTS + 5);
        pack.hold(5);
        let back = Pack::from_text(&pack.to_text());
        assert_eq!(back.to_text(), pack.to_text());
        assert_eq!((back.count("sticks"), back.held(), back.in_hand().map(|s| s.item.id.as_str())), (1, 5, Some("poppy")));
        assert_eq!(back.hotbar()[0].as_ref().map(|s| s.item.clone()), Some(sticks()));
        // A pack from before there was a hotbar reads as it did, and is given its lantern.
        let mut old = Pack::from_text("slot 3 = 4|flower|1,2,3|x|X|about\nslot 30 = 4|flower|1,2,3|y|Y|about\n");
        assert_eq!((old.count("x"), old.count("y"), old.count("lantern")), (4, 0, 0));
        old.give_kit();
        assert_eq!(old.hotbar()[HOTBAR - 1].as_ref().map(|s| s.item.clone()), Some(lantern()));
        // One from when there were tents reads without its tent, and is given its
        // lantern, once.
        let mut tented = Pack::from_text("hot 0 = 1|tent|204,182,136|tent|Tent|Canvas.\nhot 1 = 5|stick|150,116,80|sticks|Sticks|Dry wood.\nkit = 1\n");
        assert_eq!((tented.count("tent"), tented.count("sticks"), tented.slots().iter().flatten().count()), (0, 5, 1));
        tented.give_kit();
        tented.give_kit();
        assert_eq!((tented.count("tent"), tented.count("lantern")), (0, 1));
        assert_eq!(Pack::from_text(&tented.to_text()).to_text(), tented.to_text());
        // A save from before spots had dates still reads.
        assert!(Pack::from_text("gathered = 4,5;6,-7\n").gathered(6, -7));
    }

    #[test]
    fn the_held_slot_goes_round_and_things_are_spent_from_wherever_they_are() {
        let mut pack = Pack::new();
        assert!(!pack.hold(0) && pack.hold(6) && pack.cycle(1));
        assert_eq!(pack.held(), 0);
        assert!(pack.cycle(-1));
        assert_eq!(pack.held(), 6);
        for _ in 0..5 {
            pack.add(&sticks());
        }
        pack.shift(SLOTS, 3);
        pack.add(&sticks());
        assert!(!pack.spend("sticks", 7));
        assert_eq!(pack.count("sticks"), 6);
        assert!(pack.spend("sticks", FIRE_STICKS));
        assert_eq!(pack.count("sticks"), 3);
        assert!(pack.spend("sticks", 3));
        assert!(pack.slots().iter().all(Option::is_none));
        assert_eq!(pack.add_many(&sticks(), 4), 0);
    }

    #[test]
    fn what_was_gathered_grows_back_in_a_year_or_two() {
        let mut pack = Pack::new();
        assert!(pack.pass(5.0).is_empty());
        for k in 0..200 {
            pack.mark(k, 7 - k);
        }
        // Nothing within the year; all of it within two; each spot in its own time.
        assert!(pack.pass(0.99).is_empty());
        let mut back = 0;
        let mut seen = Vec::new();
        for _ in 0..21 {
            let now = pack.pass(0.05);
            back += now.len();
            seen.push(now.len());
            assert!(now.iter().all(|&(x, z)| !pack.gathered(x, z)));
        }
        assert_eq!(back, 200, "{seen:?}");
        assert!(seen.iter().filter(|n| **n > 0).count() > 10, "{seen:?}");
        assert_eq!(pack.spots().count(), 0);
        // The clock and the dates come back from a save.
        pack.mark(1, 2);
        pack.pass(0.5);
        let mut again = Pack::from_text(&pack.to_text());
        assert!(again.gathered(1, 2));
        assert_eq!(again.pass(Pack::grows_back_after(1, 2) - 0.5 + 1e-6), vec![(1, 2)]);
    }

    #[test]
    fn the_world_tells_which_flowers_stand_where_the_same_each_time() {
        let world = worldgen::world::GameWorld::with_config(3, worldgen::continent::SimConfig { size: 256 });
        let c = world.continent(0, 0);
        // Squares of land across the continent: somewhere among them flowers grow.
        let cs = c.cell_size() as i32;
        let mut found = Vec::new();
        for i in (0..c.size * c.size).step_by(97) {
            if !c.ocean[i] && c.height[i] > 4.0 {
                let (x, z) = ((i % c.size) as i32 * cs, (i / c.size) as i32 * cs);
                let here = world.flowers_in(x, z, 24);
                assert_eq!(here, world.flowers_in(x, z, 24));
                assert!(here.iter().all(|((fx, fz), _)| (x..x + 24).contains(fx) && (z..z + 24).contains(fz)));
                // A smaller square inside it names the same flowers on the columns they share.
                let inner = world.flowers_in(x + 5, z + 7, 10);
                assert!(inner.iter().all(|f| here.contains(f)), "at {x},{z}");
                found.extend(here);
            }
            if found.len() > 200 {
                break;
            }
        }
        assert!(found.len() > 200, "only {} flowers found", found.len());
        let kinds: HashSet<&str> = found.iter().map(|f| f.1.id()).collect();
        assert!(kinds.len() >= 2, "{kinds:?}");
    }

    #[test]
    fn only_so_many_gathered_spots_are_remembered() {
        let mut pack = Pack::new();
        for k in 0..REMEMBERED as i32 + 10 {
            pack.mark(k, 0);
        }
        assert!(!pack.gathered(0, 0) && !pack.gathered(9, 0) && pack.gathered(10, 0) && pack.gathered(REMEMBERED as i32 + 9, 0));
    }

    #[test]
    fn the_flowers_of_the_ground_can_be_gathered_and_the_ground_cannot() {
        let mats = mat::build();
        let berries = of_voxel(mat::HOLLY_BERRY, &mats).expect("holly berries");
        assert_eq!((berries.id.as_str(), berries.color), ("holly-berries", mats.get(mat::HOLLY_BERRY).color));
        // Every flower makes a thing of its own, with something to say for itself.
        let things: Vec<Item> = Flower::ALL.iter().map(|f| of_flower(*f)).collect();
        let ids: HashSet<&str> = things.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids.len(), Flower::ALL.len());
        assert!(things.iter().all(|t| !t.name.is_empty() && t.about.len() > 10 && !t.about.contains('|')));
        assert!(of_voxel(mat::GRASS, &mats).is_none() && of_voxel(mat::STONE, &mats).is_none() && of_voxel(voxel::AIR, &mats).is_none());
        // Every picture is eight squares each way.
        for (shape, word) in Shape::ALL {
            assert!(shape.picture().iter().all(|l| l.chars().count() == 8), "{word}");
            assert_eq!(Shape::from_word(word), Some(shape));
        }
    }
}
