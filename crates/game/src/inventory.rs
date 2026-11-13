//! The pack: what the player has gathered on their way, and the window that shows it.
//!
//! A pack is a few rows of slots, each holding a stack of one kind of thing. Things are
//! gathered with the interact key from what grows within reach; the world itself is not
//! changed by it, so each spot gives only once (the pack remembers where). The window
//! shows the slots as a grid of small pictures drawn in squares, as everything here is:
//! drag one onto another to move, join or swap them, click one to read about it or let
//! it go. A pack is kept with its world's save.

use std::collections::{HashSet, VecDeque};

use voxel::Voxel;
use worldgen::{features::plants::Flower, materials as mat};

/// Slots in a pack, and how many across the window shows them.
pub const SLOTS: usize = 24;
pub const ACROSS: usize = 6;
/// Most of one kind a slot holds.
pub const STACK: u32 = 99;
/// How many gathered spots are remembered; the oldest are forgotten first, by which
/// time they are a long walk behind.
const REMEMBERED: usize = 4000;

/// How a thing is drawn in its slot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    Flower,
    Spike,
    Sprig,
    Berries,
    Mushroom,
    Reed,
}

impl Shape {
    const ALL: [(Shape, &'static str); 6] = [
        (Shape::Flower, "flower"),
        (Shape::Spike, "spike"),
        (Shape::Sprig, "sprig"),
        (Shape::Berries, "berries"),
        (Shape::Mushroom, "mushroom"),
        (Shape::Reed, "reed"),
    ];

    fn word(self) -> &'static str {
        Self::ALL.iter().find(|s| s.0 == self).expect("every shape has a word").1
    }

    fn from_word(word: &str) -> Option<Self> {
        Self::ALL.iter().find(|s| s.1 == word).map(|s| s.0)
    }

    /// The picture, eight squares each way: `c` the thing's own colour, `d` a darker
    /// shade of it, `l` a lighter, `y` a yellow heart, `g` and `h` stem and leaf greens,
    /// `w` a pale stalk, `b` brown.
    fn picture(self) -> [&'static str; 8] {
        match self {
            Shape::Flower => ["........", "..c..c..", ".cclcc..", "..cyyc..", ".ccycc..", "..c.gc..", "...gh...", "...g...."],
            Shape::Spike => ["...l....", "...c....", "..ccd...", "...cc...", "..dcc...", "...g.h..", "...gh...", "...g...."],
            Shape::Sprig => ["........", "..c..l..", ".cg.cg..", "..gcg...", ".c.g.c..", "..hgh...", "...g....", "...g...."],
            Shape::Berries => ["........", "...h....", "..hhg...", "....g...", "..lc.c..", ".ccdccd.", "..d..d..", "........"],
            Shape::Mushroom => ["........", "..cccc..", ".clccdc.", ".cccccd.", "...ww...", "...ww...", "...ww...", "..hwwh.."],
            Shape::Reed => ["...b....", "...b....", "...c....", "...c....", "...d..h.", "...g.h..", "..hgh...", "...g...."],
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

#[derive(Clone, PartialEq, Debug)]
pub struct Stack {
    pub item: Item,
    pub count: u32,
}

pub struct Pack {
    slots: Vec<Option<Stack>>,
    /// The columns of the world already gathered from, and the order they were.
    gathered: HashSet<(i32, i32)>,
    order: VecDeque<(i32, i32)>,
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
        Self { slots: vec![None; SLOTS], gathered: HashSet::new(), order: VecDeque::new(), unsaved: false }
    }

    pub fn slots(&self) -> &[Option<Stack>] {
        &self.slots
    }

    /// How many of the thing called `id` are carried.
    pub fn count(&self, id: &str) -> u32 {
        self.slots.iter().flatten().filter(|s| s.item.id == id).map(|s| s.count).sum()
    }

    /// Puts one of `item` in: on a stack of its kind with room, else in the first empty
    /// slot. False if the pack is full.
    pub fn add(&mut self, item: &Item) -> bool {
        let stack = self.slots.iter().position(|s| s.as_ref().is_some_and(|s| s.item.id == item.id && s.count < STACK));
        let Some(at) = stack.or_else(|| self.slots.iter().position(Option::is_none)) else { return false };
        match &mut self.slots[at] {
            Some(s) => s.count += 1,
            empty => *empty = Some(Stack { item: item.clone(), count: 1 }),
        }
        self.unsaved = true;
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
    /// as it has room, else changing places with what is there.
    pub fn shift(&mut self, from: usize, to: usize) {
        if from == to || from >= SLOTS || to >= SLOTS || self.slots[from].is_none() {
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

    /// Whether the column at `(x, z)` has been gathered from.
    pub fn gathered(&self, x: i32, z: i32) -> bool {
        self.gathered.contains(&(x, z))
    }

    /// Remembers that the column at `(x, z)` has been gathered from.
    pub fn mark(&mut self, x: i32, z: i32) {
        if self.gathered.insert((x, z)) {
            self.order.push_back((x, z));
            if self.order.len() > REMEMBERED
                && let Some(old) = self.order.pop_front()
            {
                self.gathered.remove(&old);
            }
            self.unsaved = true;
        }
    }

    /// The pack as a save holds it: a line to a slot, each telling all there is to know
    /// of what is in it, so that a thing from a newer or older game still reads.
    pub fn to_text(&self) -> String {
        let clean = |s: &str| s.replace(['|', '\n', '\r'], " ");
        let mut out = String::new();
        for (i, s) in self.slots.iter().enumerate() {
            if let Some(s) = s {
                let [r, g, b] = s.item.color;
                out += &format!("slot {i} = {}|{}|{r},{g},{b}|{}|{}|{}\n", s.count, s.item.shape.word(), clean(&s.item.id), clean(&s.item.name), clean(&s.item.about));
            }
        }
        for chunk in self.order.iter().collect::<Vec<_>>().chunks(64) {
            let spots: Vec<String> = chunk.iter().map(|(x, z)| format!("{x},{z}")).collect();
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
            if key == "gathered" {
                for spot in value.split(';') {
                    if let Some((x, z)) = spot.split_once(',')
                        && let (Ok(x), Ok(z)) = (x.trim().parse(), z.trim().parse())
                    {
                        pack.mark(x, z);
                    }
                }
            } else if let Some(at) = key.strip_prefix("slot ").and_then(|n| n.trim().parse::<usize>().ok()) {
                let parts: Vec<&str> = value.splitn(6, '|').collect();
                let stack = (|| {
                    let [count, shape, color, id, name, about] = parts[..] else { return None };
                    let rgb: Vec<u8> = color.split(',').filter_map(|v| v.trim().parse().ok()).collect();
                    let count = count.trim().parse::<u32>().ok().filter(|c| *c > 0)?.min(STACK);
                    let item = Item::new(id.trim(), name.trim(), about.trim(), <[u8; 3]>::try_from(rgb).ok()?, Shape::from_word(shape.trim())?);
                    (!item.id.is_empty()).then_some(Stack { item, count })
                })();
                if at < SLOTS && stack.is_some() {
                    pack.slots[at] = stack;
                }
            }
        }
        pack.unsaved = false;
        pack
    }
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
fn picture(painter: &egui::Painter, rect: egui::Rect, item: &Item, fade: f32) {
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

/// Shows the pack's window, and does to `pack` what is done in it.
pub fn show(ctx: &egui::Context, pack: &mut Pack, view: &mut View) {
    let side = 58.0;
    let gap = 7.0;
    let mut rects = Vec::with_capacity(SLOTS);
    egui::Area::new(egui::Id::unique("pack")).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).fade_in(false).show(ctx, |ui| {
        egui::Frame::new().fill(PANEL).stroke(egui::Stroke::new(1.0, EDGE)).corner_radius(10).inner_margin(egui::Margin::same(18)).show(ui, |ui| {
            let width = ACROSS as f32 * side + (ACROSS - 1) as f32 * gap;
            ui.set_width(width);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Pack").size(22.0).color(INK));
                let carried: u32 = pack.slots().iter().flatten().map(|s| s.count).sum();
                let kinds = pack.slots().iter().flatten().count();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(format!("{carried} things in {kinds} of {SLOTS} slots")).size(13.0).color(FAINT));
                });
            });
            ui.add_space(10.0);
            egui::Grid::new("pack slots").spacing([gap, gap]).show(ui, |ui| {
                for at in 0..SLOTS {
                    let (rect, response) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click_and_drag());
                    rects.push(rect);
                    let painter = ui.painter();
                    let stack = pack.slots()[at].as_ref();
                    let lit = response.hovered() || view.dragging.is_some_and(|_| ui.rect_contains_pointer(rect));
                    painter.rect_filled(rect, 7.0, if lit { SLOT_LIT } else { SLOT });
                    let edge = if view.selected == Some(at) && stack.is_some() { egui::Stroke::new(2.0, PICKED) } else { egui::Stroke::new(1.0, EDGE) };
                    painter.rect_stroke(rect, 7.0, edge, egui::StrokeKind::Inside);
                    if let Some(s) = stack {
                        // What is being dragged is drawn under the cursor, and faintly at home.
                        let fade = if view.dragging == Some(at) { 0.3 } else { 1.0 };
                        picture(painter, rect.shrink(9.0), &s.item, fade);
                        if s.count > 1 {
                            let corner = rect.right_bottom() - egui::vec2(6.0, 4.0);
                            painter.text(corner + egui::vec2(1.0, 1.0), egui::Align2::RIGHT_BOTTOM, s.count, egui::FontId::proportional(14.0), egui::Color32::from_black_alpha(200));
                            painter.text(corner, egui::Align2::RIGHT_BOTTOM, s.count, egui::FontId::proportional(14.0), INK);
                        }
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
                    if at % ACROSS == ACROSS - 1 {
                        ui.end_row();
                    }
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
                            if ui.button("Let all go").clicked() {
                                pack.drop_from(at, u32::MAX);
                            }
                            if ui.button("Let one go").clicked() {
                                pack.drop_from(at, 1);
                            }
                        });
                    });
                    ui.label(egui::RichText::new(&s.item.about).size(13.0).color(FAINT));
                }
                None => {
                    let hint = if pack.slots().iter().all(Option::is_none) {
                        "Nothing yet. Press E by a flower or a mushroom to gather it."
                    } else {
                        "Click a thing to read about it. Drag it to another slot to move it."
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
            if let Some(to) = pointer.and_then(|p| rects.iter().position(|r| r.contains(p))) {
                pack.shift(from, to);
                view.selected = Some(to);
            }
            view.dragging = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thing(id: &str) -> Item {
        Item::new(id, "A thing", "About it.", [200, 100, 50], Shape::Flower)
    }

    #[test]
    fn things_stack_by_kind_and_a_full_pack_takes_no_more() {
        let mut pack = Pack::new();
        for _ in 0..STACK + 3 {
            assert!(pack.add(&thing("poppy")));
        }
        assert!(pack.add(&thing("heather")));
        let held: Vec<(String, u32)> = pack.slots().iter().flatten().map(|s| (s.item.id.clone(), s.count)).collect();
        assert_eq!(held, [("poppy".into(), STACK), ("poppy".into(), 3), ("heather".into(), 1)]);
        assert_eq!(pack.count("poppy"), STACK + 3);
        // Fill every slot with something different: then only what stacks goes in.
        for k in 0..SLOTS {
            pack.add(&thing(&format!("kind {k}")));
        }
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
        pack.shift(1, 17);
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
