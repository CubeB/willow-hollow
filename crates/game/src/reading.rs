//! Something on a wall read close to (#207): a settlement's notice board or a guild
//! house's, shown as a sheet of paper over the view until the player puts it down (E
//! again) or walks away.

/// A page to read: its heading, its lines (each with a heading of its own, or none) and
/// a word at its foot.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    pub title: String,
    pub lines: Vec<(String, String)>,
    pub foot: Option<String>,
}

impl Page {
    pub fn new(title: String) -> Self {
        Page { title, ..Default::default() }
    }

    /// The page as plain text, for the console and the log.
    pub fn text(&self) -> String {
        let mut out = self.title.clone();
        for (head, body) in &self.lines {
            out += "\n";
            if !head.is_empty() {
                out += &format!("{head}: ");
            }
            out += body;
        }
        if let Some(foot) = &self.foot {
            out += &format!("\n({foot})");
        }
        out
    }
}

/// How far from a board it can be read from, and how far the player can step from where
/// they read it before it is put down.
pub const REACH: f32 = 2.4;
pub const LEAVE: f32 = 2.5;

const PAPER: egui::Color32 = egui::Color32::from_rgb(234, 222, 194);
const INK: egui::Color32 = egui::Color32::from_rgb(52, 40, 30);
const FADED: egui::Color32 = egui::Color32::from_rgb(112, 96, 78);

/// Draws `page` in the middle of the screen, on paper.
pub fn show(ctx: &egui::Context, page: &Page) {
    let wide = (ctx.content_rect().width() * 0.9).min(560.0);
    egui::Area::new(egui::Id::unique("reading"))
        .anchor(egui::Align2::CENTER_CENTER, [0.0, -20.0])
        .interactable(false)
        .fade_in(false)
        .show(ctx, |ui| {
            ui.set_width(wide);
            egui::Frame::new()
                .fill(PAPER)
                .stroke(egui::Stroke::new(1.5, egui::Color32::from_rgb(150, 124, 92)))
                .corner_radius(3.0)
                .inner_margin(egui::Margin::symmetric(26, 20))
                .show(ui, |ui| {
                    ui.set_width(wide - 52.0);
                    ui.vertical_centered(|ui| ui.label(egui::RichText::new(&page.title).size(22.0).color(INK).strong()));
                    ui.add_space(10.0);
                    for (head, body) in &page.lines {
                        if !head.is_empty() {
                            ui.label(egui::RichText::new(head).size(17.0).color(INK).strong());
                        }
                        ui.label(egui::RichText::new(body).size(16.0).color(INK));
                        ui.add_space(9.0);
                    }
                    if let Some(foot) = &page.foot {
                        ui.vertical_centered(|ui| ui.label(egui::RichText::new(foot).size(13.0).color(FADED).italics()));
                    }
                });
        });
}
