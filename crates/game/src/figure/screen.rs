//! The figure screen (#204), reached from the settings menu: the player rolls a look
//! with the generator that makes the people of the settlements, steered by the country
//! and the people they come from, their age and their build, then changes it part by
//! part while the figure turns beside it. Every control is one of the [`Choice`]s, so
//! whatever is chosen is a look the figure can be drawn in.

use worldgen::{names::folk, settlements::Region};

use super::{AGES, BEARDS, CUTS, Choice, FOOTWEAR, Figure, GARMENTS, GIRTH, HEADWEAR, HEIGHT, LAYERS, LEATHERS, NAME_LENGTH, SEXES, belts, country, hats, preview};
use crate::{
    character::{Footwear, Garment, Headwear, Layer, Rgb},
    people,
};

/// The picture of the figure, in points.
const PREVIEW: egui::Vec2 = egui::vec2(230.0, 400.0);
/// How fast the figure turns while it is not being turned by hand, radians a second.
const TURNING: f32 = 0.45;
const BACKDROP: egui::Color32 = egui::Color32::from_rgb(40, 47, 56);

/// The screen while it is open: the figure being made, and what it was when it opened.
pub struct Screen {
    pub figure: Figure,
    was: Figure,
    /// A roll keeps the height and build chosen.
    keep_build: bool,
    /// The name the last roll would have given, for the player to take if they like.
    offered: String,
    /// How far the figure has turned, and how long it has stood there breathing.
    turn: f32,
    clock: f32,
    texture: Option<egui::TextureHandle>,
}

impl Screen {
    pub fn new(figure: &Figure) -> Self {
        Screen { figure: figure.clone(), was: figure.clone(), keep_build: false, offered: String::new(), turn: -0.5, clock: 0.0, texture: None }
    }

    /// The figure turned this far from facing the viewer, for a screenshot.
    pub fn turned(mut self, turn: f32) -> Self {
        self.turn = turn;
        self
    }

    /// Whether anything has been changed since it opened.
    pub fn changed(&self) -> bool {
        self.figure != self.was
    }
}

/// Draws the screen; true once the player is done with it.
pub fn show(ctx: &egui::Context, s: &mut Screen) -> bool {
    let dt = ctx.input(|i| i.stable_dt).clamp(0.0, 0.1);
    s.clock += dt;
    let mut done = false;
    let mut picks = Vec::new();
    // It opens over the settings menu at once, with nothing to fade in from.
    let window = egui::Window::new("Your figure").anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).resizable(false).collapsible(false).fade_in(false);
    window.show(ctx, |ui| {
        // Spaces between the columns rather than lines, which would stand as tall as the
        // screen and push the buttons off it.
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| figure(ui, s, dt));
            ui.add_space(14.0);
            ui.vertical(|ui| {
                ui.set_width(330.0);
                who(ui, s);
                ui.add_space(6.0);
                body(ui, &s.figure, &mut picks);
            });
            ui.add_space(14.0);
            ui.vertical(|ui| {
                ui.set_width(420.0);
                clothes(ui, &s.figure, &mut picks);
            });
        });
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Reset to the wanderer").clicked() {
                s.figure = Figure { name: std::mem::take(&mut s.figure.name), roll: s.figure.roll, ..Figure::default() };
            }
            if ui.add_enabled(s.changed(), egui::Button::new("Undo changes")).clicked() {
                s.figure = s.was.clone();
            }
            done = ui.button("Done").clicked();
            ui.label(egui::RichText::new("Esc keeps what is chosen too.").small().weak());
        });
    });
    for choice in picks {
        s.figure.choose(choice);
    }
    done
}

/// The figure itself, turning; dragged, it turns by hand.
fn figure(ui: &mut egui::Ui, s: &mut Screen, dt: f32) {
    let (rect, response) = ui.allocate_exact_size(PREVIEW, egui::Sense::drag());
    if response.dragged() {
        s.turn += response.drag_delta().x * 0.012;
    } else {
        s.turn += dt * TURNING;
    }
    // A pixel of the picture for each of the screen's, but no finer than one and a half
    // to a point: it is drawn afresh each frame, about 2 ms at one to a point and 5 at
    // one and a half, and on a screen scaled twice as fine it would take 8 ms for a
    // figure no clearer to see.
    let ppp = ui.ctx().pixels_per_point().min(1.5);
    let px = [(PREVIEW.x * ppp).round() as usize, (PREVIEW.y * ppp).round() as usize];
    let (image, soles, metre) = preview::picture(&s.figure.look, s.turn, s.clock, px);
    match &mut s.texture {
        Some(t) => t.set(image, egui::TextureOptions::LINEAR),
        None => s.texture = Some(ui.ctx().load_texture("figure", image, egui::TextureOptions::LINEAR)),
    }
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, BACKDROP);
    // The ground it stands on, and its shadow there.
    let soles = rect.min + soles.to_vec2() / ppp;
    let metre = metre / ppp;
    painter.add(egui::Shape::ellipse_filled(soles, egui::vec2(metre * 0.9, metre * 0.16), egui::Color32::from_rgba_premultiplied(30, 36, 43, 255)));
    painter.add(egui::Shape::ellipse_filled(soles, egui::vec2(metre * 0.3, metre * 0.07), egui::Color32::from_black_alpha(90)));
    if let Some(t) = &s.texture {
        painter.image(t.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    }
    painter.text(rect.left_bottom() + egui::vec2(8.0, -8.0), egui::Align2::LEFT_BOTTOM, format!("{:.2} m", s.figure.look.build.metres()), egui::FontId::proportional(13.0), egui::Color32::from_gray(170));
    response.on_hover_text("Drag to turn");
}

/// Who they are: their name, and a roll of a new look, steered.
fn who(ui: &mut egui::Ui, s: &mut Screen) {
    egui::Grid::new("who").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        ui.label("Name");
        ui.add(egui::TextEdit::singleline(&mut s.figure.name).char_limit(NAME_LENGTH).hint_text("none given").desired_width(200.0));
        ui.end_row();
        let offered = s.offered.clone();
        if !offered.is_empty() && offered != s.figure.name {
            ui.label("");
            if ui.small_button(format!("Call me {offered}")).on_hover_text("The name the roll gave them").clicked() {
                s.figure.name = offered;
            }
            ui.end_row();
        }

        let mut roll = s.figure.roll;
        let mut again = false;
        ui.label("");
        ui.horizontal(|ui| {
            again = ui.button("Roll").on_hover_text("A new look, from the people of the settlements").clicked();
            ui.checkbox(&mut s.keep_build, "keep my height and build");
        });
        ui.end_row();

        ui.label("From");
        egui::ComboBox::from_id_salt("region").selected_text(country(roll.region)).width(150.0).show_ui(ui, |ui| {
            for r in Region::ALL {
                ui.selectable_value(&mut roll.region, r, country(r));
            }
        });
        ui.end_row();

        ui.label("Their people");
        egui::ComboBox::from_id_salt("tongue").selected_text(tongue_word(roll.tongue)).width(150.0).show_ui(ui, |ui| {
            ui.selectable_value(&mut roll.tongue, None, tongue_word(None));
            for t in folk::tongues() {
                ui.selectable_value(&mut roll.tongue, Some(t), tongue_word(Some(t)));
            }
        });
        ui.end_row();

        ui.label("Age");
        ui.horizontal(|ui| {
            for (age, word) in AGES {
                ui.radio_value(&mut roll.age, age, word);
            }
        });
        ui.end_row();

        ui.label("Who");
        ui.horizontal(|ui| {
            for (sex, word) in SEXES {
                ui.radio_value(&mut roll.sex, sex, word);
            }
        });
        ui.end_row();

        ui.label("Dyes");
        ui.horizontal(|ui| {
            ui.small("plain");
            ui.add(egui::Slider::new(&mut roll.wealth, 0.0..=1.0).show_value(false));
            ui.small("bright");
        });
        ui.end_row();

        // A change of how it is steered rolls again from the same seed, to show what the
        // change does; the button goes on to the next seed.
        if again {
            roll = roll.next();
        }
        if again || roll != s.figure.roll {
            s.offered = s.figure.reroll(&roll, s.keep_build);
        }
    });
}

fn tongue_word(t: Option<&str>) -> String {
    match t {
        None => "as the country's".into(),
        Some(t) => {
            let mut c = t.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
        }
    }
}

/// The body: height and build, skin and hair.
fn body(ui: &mut egui::Ui, f: &Figure, picks: &mut Vec<Choice>) {
    let l = &f.look;
    egui::Grid::new("body").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        ui.label("Height");
        let mut tall = l.build.metres();
        // Shown to the centimetre, but not rounded to it: a slider rounds what it holds
        // each frame, which would change a rolled height just by looking at it.
        if ui.add(egui::Slider::new(&mut tall, HEIGHT).custom_formatter(|v, _| format!("{v:.2}")).suffix(" m")).changed() {
            picks.push(Choice::Height(tall));
        }
        ui.end_row();

        ui.label("Build");
        ui.horizontal(|ui| {
            let mut girth = l.build.girth;
            ui.small("slight");
            if ui.add(egui::Slider::new(&mut girth, GIRTH).show_value(false)).changed() {
                picks.push(Choice::Girth(girth));
            }
            ui.small("broad");
        });
        ui.end_row();

        ui.label("Skin");
        let skins: Vec<(&str, Rgb)> = people::SKIN.iter().map(|c| ("", *c)).collect();
        if let Some(k) = swatches(ui, &skins, l.skin) {
            picks.push(Choice::Skin(k));
        }
        ui.end_row();

        ui.label("Hair");
        ui.vertical(|ui| {
            if let Some(cut) = combo(ui, "cut", &CUTS, l.cut) {
                picks.push(Choice::Cut(cut));
            }
            if let Some(k) = swatches(ui, &people::HAIR, f.natural) {
                picks.push(Choice::HairColour(k));
            }
            let mut grey = f.grey;
            if ui.add(egui::Slider::new(&mut grey, 0.0..=1.0).show_value(false).text("grey")).changed() {
                picks.push(Choice::Grey(grey));
            }
        });
        ui.end_row();

        ui.label("Beard");
        if let Some(beard) = combo(ui, "beard", &BEARDS, l.beard) {
            picks.push(Choice::Beard(beard));
        }
        ui.end_row();
    });
}

/// What they wear, and its colours.
fn clothes(ui: &mut egui::Ui, f: &Figure, picks: &mut Vec<Choice>) {
    let l = &f.look;
    egui::Grid::new("clothes").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        ui.label("Wearing");
        ui.vertical(|ui| {
            if let Some(g) = combo(ui, "garment", &GARMENTS, l.garment) {
                picks.push(Choice::Garment(g));
            }
            if let Some(k) = swatches(ui, &people::CLOTH, l.top) {
                picks.push(Choice::Top(k));
            }
        });
        ui.end_row();

        ui.label(if matches!(l.garment, Garment::Dress | Garment::Robe) { "Skirt" } else { "Trousers" });
        if let Some(k) = swatches(ui, &people::CLOTH, l.bottom) {
            picks.push(Choice::Bottom(k));
        }
        ui.end_row();

        ui.label("Over it");
        ui.vertical(|ui| {
            if let Some(layer) = combo(ui, "layer", &LAYERS, l.layer) {
                picks.push(Choice::Layer(layer));
            }
            if l.layer != Layer::None
                && let Some(k) = swatches(ui, &people::CLOTH, l.over)
            {
                picks.push(Choice::Over(k));
            }
        });
        ui.end_row();

        ui.label("Scarf");
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                let (mut scarf, mut rolled) = (l.scarf, l.rolled);
                if ui.checkbox(&mut scarf, "a scarf").changed() {
                    picks.push(Choice::Scarf(scarf));
                }
                if ui.checkbox(&mut rolled, "sleeves rolled").changed() {
                    picks.push(Choice::Rolled(rolled));
                }
            });
            // The scarf's colour ties a plait too.
            if (l.scarf || l.cut == crate::character::Hair::Plaited)
                && let Some(k) = swatches(ui, &people::CLOTH, l.accent)
            {
                picks.push(Choice::Accent(k));
            }
        });
        ui.end_row();

        ui.label("Belt");
        if let Some(k) = swatches(ui, &belts(), l.belt) {
            picks.push(Choice::Belt(k));
        }
        ui.end_row();

        ui.label("On the head");
        ui.vertical(|ui| {
            if let Some(h) = combo(ui, "headwear", &HEADWEAR, l.headwear) {
                picks.push(Choice::Headwear(h));
            }
            if l.headwear != Headwear::None
                && let Some(k) = swatches(ui, &hats(), l.hat)
            {
                picks.push(Choice::Hat(k));
            }
        });
        ui.end_row();

        ui.label("On the feet");
        ui.vertical(|ui| {
            if let Some(foot) = combo(ui, "footwear", &FOOTWEAR, l.footwear) {
                picks.push(Choice::Footwear(foot));
            }
            if l.footwear != Footwear::Bare
                && let Some(k) = swatches(ui, &LEATHERS, l.boots)
            {
                picks.push(Choice::Boots(k));
            }
        });
        ui.end_row();
    });
}

/// A list to choose one kind from; the kind chosen, if it changed.
fn combo<T: Copy + PartialEq>(ui: &mut egui::Ui, id: &str, list: &[(T, &str)], now: T) -> Option<T> {
    let mut pick = now;
    let shown = list.iter().find(|e| e.0 == now).map_or("", |e| e.1);
    egui::ComboBox::from_id_salt(id).selected_text(shown).width(170.0).show_ui(ui, |ui| {
        for &(t, word) in list {
            ui.selectable_value(&mut pick, t, word);
        }
    });
    (pick != now).then_some(pick)
}

/// A row of colours to choose from, the one worn ringed; a colour worn that is not among
/// them (a roll's, faded or bright) first, as it is. The one clicked, if one was.
fn swatches(ui: &mut egui::Ui, palette: &[(&str, Rgb)], now: Rgb) -> Option<usize> {
    let mut picked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(3.0, 3.0);
        if !palette.iter().any(|(_, c)| *c == now) {
            swatch(ui, now, true).on_hover_text("as it is");
        }
        for (k, (name, c)) in palette.iter().enumerate() {
            let r = swatch(ui, *c, *c == now);
            let r = if name.is_empty() { r } else { r.on_hover_text(*name) };
            if r.clicked() {
                picked = Some(k);
            }
        }
    });
    picked
}

fn swatch(ui: &mut egui::Ui, c: Rgb, chosen: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::click());
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0, egui::Color32::from_rgb(c[0], c[1], c[2]));
    let ring = if chosen {
        egui::Stroke::new(2.0, egui::Color32::from_rgb(236, 214, 150))
    } else if response.hovered() {
        egui::Stroke::new(1.0, egui::Color32::from_gray(200))
    } else {
        egui::Stroke::new(1.0, egui::Color32::from_gray(70))
    };
    painter.rect_stroke(rect, 3.0, ring, egui::StrokeKind::Outside);
    response
}

#[cfg(test)]
mod tests {
    use super::super::Roll;
    use super::*;

    #[test]
    fn looking_at_the_screen_changes_nothing() {
        let mut figures = vec![Figure::default()];
        for (seed, region) in [(7, Region::Desert), (12, Region::Northern), (5, Region::Wetland), (3, Region::Rainforest)] {
            for (age, _) in AGES {
                figures.push(Figure::rolled(&Roll { seed, region, age, ..Roll::default() }));
            }
        }
        let ctx = egui::Context::default();
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 720.0))), ..Default::default() };
        for f in figures {
            let mut s = Screen::new(&f);
            for _ in 0..3 {
                let mut done = false;
                ctx.run_pass(input.clone(), |ctx| done = show(ctx, &mut s)).drop_without_applying_deltas();
                assert!(!done);
            }
            assert!(!s.changed(), "the screen changed {:?} into {:?}", f.look, s.figure.look);
            assert!(s.texture.is_some(), "the figure was drawn");
        }
    }
}
