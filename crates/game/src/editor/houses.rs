//! The homes of the house kit (`worldgen::settlements::kit`) in the structure editor: a
//! region's hut, timber house or stone house, a different one for each variant, for any
//! household. The kit's rules are read from their files under `assets/structures/kit`
//! and read again as they are saved, so a change to them shows without a rebuild; and any
//! home can be copied into a house of one's own to edit.

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

use glam::IVec3;
use voxel::{Voxel, mesh::PlantBox};
use worldgen::{
    features::{
        designs::{Design, Kind, Special},
        landmarks::Palette,
    },
    materials as mat,
    noise::{hash2, unit},
    settlements::{
        Region,
        kit::{
            self, Building, Rules, Spec, Step,
            components::{Attached, Dress},
        },
        special::{self, Built, Facts, SpecialSpec},
    },
};

/// Which home is shown: a region and a step, as indices into [`Region::ALL`] and
/// [`Step::ALL`].
pub type House = (usize, usize);

pub fn region_label(r: Region) -> &'static str {
    match r {
        Region::Temperate => "Temperate lowland",
        Region::Upland => "Upland moor",
        Region::Northern => "Northern forest",
        Region::Alpine => "Alpine",
        Region::Desert => "Desert and savanna",
        Region::Wetland => "Wetland",
        Region::Rainforest => "Rainforest",
        Region::OldGrowth => "Old growth",
    }
}

/// The home of `region` and `step`.
pub fn of(region: Region, step: Step) -> House {
    (Region::ALL.iter().position(|&r| r == region).unwrap_or(0), Step::ALL.iter().position(|&s| s == step).unwrap_or(0))
}

/// `house/<region>/<step>` on the command line: `house/wetland/hut`.
pub fn named(tail: Option<&str>) -> House {
    let mut words = tail.unwrap_or("").split('/');
    let region = words.next().and_then(|w| Region::ALL.iter().position(|r| r.word() == w)).unwrap_or(0);
    let step = words.next().and_then(|w| Step::ALL.iter().position(|s| s.word() == w)).unwrap_or(0);
    (region, step)
}

/// What variant `variant` asks of the kit: its age, wealth and household come from it.
fn variant_spec((region, step): House, variant: u32) -> Spec {
    let h = |k: i32| unit(hash2(variant ^ 0x4805_e5ed, k, 0));
    let step = Step::ALL[step];
    let most = step.max_sleepers();
    Spec { region: Region::ALL[region], step, age: h(1), wealth: h(2), sleepers: (1 + (h(3) * most as f32) as u8).min(most), seed: variant }
}

pub fn title(house: House, variant: u32) -> String {
    format!("Houses: {}, {}, variant {variant}", region_label(Region::ALL[house.0]), Step::ALL[house.1].label().to_lowercase())
}

/// The stone and timber of a region's own country, varied for the variant as a site's
/// palette is.
pub fn palette(region: Region, variant: u32) -> Palette {
    let biome = region.biome();
    Palette::local(biome, mat::bedrock(biome).0).varied(variant)
}

/// What variant `variant` asks of a settlement building's generator (`settlements::special`)
/// in `region`: its age and wealth, the side of a sawmill's stream (none every fourth
/// variant, which saws over a pit), and the facts a guild house's guilds come from.
/// Nothing for a well head, which has no generator: an empty plot. The gates, walls,
/// beacon and pavilion (`special::STRUCTURES`) are shown as the buildings are.
pub fn special(kind: Special, region: Region, variant: u32) -> Option<SpecialSpec> {
    if !special::MADE.contains(&kind) && !special::STRUCTURES.contains(&kind) {
        return None;
    }
    let h = |k: i32| unit(hash2(variant ^ 0x5a1d_57e5, k, 0));
    let stream = [None, Some(IVec3::Z), Some(IVec3::NEG_Z), Some(IVec3::NEG_X)][(variant % 4) as usize];
    let facts = Facts { willows: h(3) < 0.5, stone: h(4) < 0.5, trade: h(5) < 0.5, colour: h(6) < 0.4, water: h(7) < 0.6, ancient: h(8) < 0.25 };
    Some(SpecialSpec { which: kind, region, age: h(1), wealth: h(2), seed: variant, stream, facts })
}

/// A settlement building of the generator, of the stone and timber of its region's
/// country, as variant `variant` has it.
pub fn build_special(kind: Special, region: Region, variant: u32) -> Option<Built> {
    special::build(&special(kind, region, variant)?, &palette(region, variant))
}

/// The settlement building on view as a structure of one's own, named `name`.
pub fn copy_special(kind: Special, region: Region, variant: u32, name: &str) -> Option<Design> {
    special::copy(&special(kind, region, variant)?, &palette(region, variant), name)
}

/// What a settlement building has that a home has not, in a line: its guest beds, its
/// fire, its wheel or its guilds.
pub fn special_notes(b: &Built) -> String {
    let mut out = vec![format!("{} beds, {} places for furniture, {} windows", b.building.inside.beds.len(), b.building.inside.furniture.len(), b.building.inside.windows.len())];
    if !b.guest_beds.is_empty() {
        out.push(format!("{} of them let to travellers", b.guest_beds.len()));
    }
    if b.fire.is_some() {
        out.push("a fire always alight".into());
    }
    if let Some(w) = b.wheel {
        out.push(format!("a water wheel {} voxels across on the {} side", w.radius * 2 + 1, side_word(w.side)));
    }
    if !b.guilds.is_empty() {
        out.push(format!("the guilds of {}", b.guilds.iter().map(|g| g.name()).collect::<Vec<_>>().join(", ")));
    }
    let (w, d) = (b.building.footprint.hi - b.building.footprint.lo + 1).into();
    out.push(format!("{w} by {d} voxels in all"));
    out.join("; ")
}

/// A side of a building as built, door at +X, in words.
fn side_word(d: IVec3) -> &'static str {
    match (d.x.signum(), d.z.signum()) {
        (1, _) => "front",
        (-1, _) => "back",
        (_, 1) => "right",
        _ => "left",
    }
}

/// A household set by hand: each of its age, wealth and number sleeping there, or `None`
/// while the variant says.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Household {
    pub age: Option<f32>,
    pub wealth: Option<f32>,
    pub sleepers: Option<u8>,
}

/// When a file was last saved and how long it was then: two saves within one tick of the
/// clock are still told apart unless they are the same length.
type Stamp = (SystemTime, u64);

fn stamp(path: &Path) -> Option<Stamp> {
    std::fs::metadata(path).and_then(|m| Ok((m.modified()?, m.len()))).ok()
}

/// One region's rules as last read from its file.
struct KitFile {
    path: PathBuf,
    /// The file as it was read, to notice it saved again.
    stamp: Option<Stamp>,
    /// The rules last read from it without a fault, if any have been.
    rules: Option<Rules>,
    /// What was wrong with it when it was last read, naming the line.
    error: Option<String>,
}

impl KitFile {
    fn read(&mut self, region: Region) {
        self.stamp = stamp(&self.path);
        match std::fs::read_to_string(&self.path) {
            Ok(text) => match Rules::parse(region, &text) {
                Ok(r) => {
                    self.rules = Some(r);
                    self.error = None;
                }
                // The last rules that read are kept to build with until it reads again.
                Err(e) => self.error = Some(e),
            },
            Err(_) => {
                self.rules = None;
                self.error = None;
            }
        }
    }
}

/// The kit's rules as the editor builds with them: each region's from its file when the
/// editor runs from the repository, and otherwise those compiled in. The world always
/// builds with those compiled in.
pub struct Kit {
    files: Vec<KitFile>,
    checked: Instant,
}

/// How often the kit's files are looked at for a change.
const EVERY: Duration = Duration::from_millis(500);

impl Kit {
    pub fn new(dir: &Path) -> Self {
        let files = Region::ALL.iter().map(|r| KitFile { path: dir.join("kit").join(format!("{}.txt", r.word())), stamp: None, rules: None, error: None }).collect();
        let mut kit = Kit { files, checked: Instant::now() };
        kit.read_all();
        kit
    }

    pub fn read_all(&mut self) {
        for (k, f) in self.files.iter_mut().enumerate() {
            f.read(Region::ALL[k]);
        }
        self.checked = Instant::now();
    }

    /// Reads again any file saved since it was last read, looking no more than twice a
    /// second. Returns whether any was.
    pub fn poll(&mut self) -> bool {
        if self.checked.elapsed() < EVERY {
            return false;
        }
        self.checked = Instant::now();
        let mut changed = false;
        for (k, f) in self.files.iter_mut().enumerate() {
            if stamp(&f.path) != f.stamp {
                f.read(Region::ALL[k]);
                changed = true;
            }
        }
        changed
    }

    fn file(&self, region: Region) -> &KitFile {
        &self.files[Region::ALL.iter().position(|&r| r == region).unwrap()]
    }

    /// The rules to build `region`'s homes with.
    pub fn rules(&self, region: Region) -> &Rules {
        self.file(region).rules.as_ref().unwrap_or_else(|| kit::rules::rules(region))
    }

    /// Says where `region`'s rules come from, and what is wrong with its file if anything
    /// is. Returns whether the files were read again.
    pub fn ui(&mut self, ui: &mut egui::Ui, region: Region) -> bool {
        let f = self.file(region);
        let name = format!("kit/{}.txt", region.word());
        ui.label(egui::RichText::new("The kit's rules").strong());
        match (&f.error, &f.rules) {
            (Some(e), last) => {
                ui.label(egui::RichText::new(format!("{name}, {e}")).color(egui::Color32::from_rgb(240, 120, 100)));
                ui.small(if last.is_some() { "Until it reads, the homes here are built with the rules last read from it." } else { "Until it reads, the homes here are built with the rules compiled into the game." });
            }
            (None, Some(_)) => {
                ui.small(format!("Read from {name}, and read again whenever it is saved."));
            }
            (None, None) => {
                ui.small(format!("There is no {name} here, so the homes here are built with the rules compiled into the game."));
            }
        }
        // Whatever the editor builds with, the world builds with what was compiled in.
        if f.rules.as_ref().is_some_and(|r| r != kit::rules::rules(region)) {
            ui.small(egui::RichText::new("These differ from the rules compiled into this game. Only the editor builds with them: the world takes them up when the game is built again.").color(egui::Color32::from_rgb(230, 190, 110)));
        } else {
            ui.small("The world builds with the rules compiled into the game, and takes up a change to the files when the game is built again.");
        }
        let again = ui.button("Read the kit files again").on_hover_text("They are read again whenever they are saved, too").clicked();
        if again {
            self.read_all();
        }
        again
    }
}

/// The Houses view: the household asked of the kit, and its rules.
pub struct Houses {
    pub household: Household,
    pub kit: Kit,
    /// The homes are shown with their region's components on them (porches, awnings,
    /// woodpiles), as the world builds them, or bare, as the kit makes them first.
    pub components: bool,
}

impl Houses {
    pub fn new(dir: &Path) -> Self {
        Houses { household: Household::default(), kit: Kit::new(dir), components: true }
    }

    /// What the kit is asked for: the variant's household, with whatever is set by hand.
    pub fn spec(&self, house: House, variant: u32) -> Spec {
        let s = variant_spec(house, variant);
        let h = self.household;
        Spec { age: h.age.unwrap_or(s.age), wealth: h.wealth.unwrap_or(s.wealth), sleepers: h.sleepers.unwrap_or(s.sleepers).min(s.step.max_sleepers()), ..s }
    }

    /// The kit's own home, of the stone and timber of its region's country, and its voxels
    /// and fine parts to draw.
    pub fn build(&self, house: House, variant: u32) -> (Building, Vec<(IVec3, Voxel)>, Vec<PlantBox>) {
        let (b, _) = self.dressed(house, variant);
        let (voxels, fine) = (b.voxels.clone(), b.fine.clone());
        (b, voxels, fine)
    }

    /// The kit's own home with its components or bare, as [`Houses::components`] says,
    /// and the components set on it.
    fn dressed(&self, house: House, variant: u32) -> (Building, Vec<Attached>) {
        let s = self.spec(house, variant);
        let (pal, rules) = (palette(s.region, variant), self.kit.rules(s.region));
        if self.components { kit::generate_dressed(&s, &pal, rules, &Dress::default()) } else { (kit::generate_bare(&s, &pal, rules), Vec::new()) }
    }

    /// The home on view as a house of one's own, named `name`.
    pub fn copy(&self, house: House, variant: u32, name: &str) -> Design {
        let s = self.spec(house, variant);
        kit::copy(&s, &palette(s.region, variant), self.kit.rules(s.region), Kind::House(s.region, s.step), name)
    }

    /// The region, step, variant and household to show, and what the variant made.
    /// Returns the home to show if it changed, and whether the variant or the household
    /// did.
    pub fn tools(&mut self, ui: &mut egui::Ui, house: House, variant: &mut u32, built: Option<&Building>) -> (Option<House>, bool) {
        let mut next = house;
        ui.label(egui::RichText::new("Houses").strong());
        ui.small("The house kit's homes, in the stone and timber of their own country. Each variant is another household: older or newer, poorer or richer, more or fewer in it. Set the household by hand below to step through the variants of one.");
        ui.add_space(4.0);
        egui::ComboBox::from_label("Region").selected_text(region_label(Region::ALL[next.0])).show_ui(ui, |ui| {
            for (k, r) in Region::ALL.iter().enumerate() {
                ui.selectable_value(&mut next.0, k, region_label(*r));
            }
        });
        ui.horizontal(|ui| {
            for (k, s) in Step::ALL.iter().enumerate() {
                ui.selectable_value(&mut next.1, k, s.label());
            }
        });
        let was = *variant;
        ui.horizontal(|ui| {
            ui.label("Variant");
            if ui.button("<").on_hover_text("Left arrow").clicked() {
                *variant = variant.wrapping_sub(1) % 65536;
            }
            ui.add(egui::DragValue::new(variant).range(0..=65535));
            if ui.button(">").on_hover_text("Right arrow").clicked() {
                *variant = (*variant + 1) % 65536;
            }
        });
        let household = self.household_ui(ui, house, *variant);
        if let Some(b) = built {
            ui.small(format!("{}; {} beds, {} places for furniture, {} windows", b.style.describe(), b.inside.beds.len(), b.inside.furniture.len(), b.inside.windows.len()));
        }
        let about = &self.kit.rules(Region::ALL[house.0]).about;
        if !about.is_empty() {
            ui.small(about);
        }
        let components = self.components_ui(ui, house, *variant);
        ui.add_space(6.0);
        let reread = self.kit.ui(ui, Region::ALL[house.0]);
        ui.add_space(6.0);
        ((next != house).then_some(next), *variant != was || household || reread || components)
    }

    /// Whether the home is shown with its components, and which went where. Returns
    /// whether that was changed.
    fn components_ui(&mut self, ui: &mut egui::Ui, house: House, variant: u32) -> bool {
        ui.add_space(4.0);
        let changed = ui.checkbox(&mut self.components, "Components").on_hover_text("The region's porches, awnings, woodpiles and the rest, set at the kit's anchors as the world sets them. Off, the home is shown as the kit makes it first").changed();
        if self.components {
            let (_, attached) = self.dressed(house, variant);
            let mut said: Vec<String> = Vec::new();
            for a in &attached {
                let one = match &a.design {
                    Some(name) => format!("{} (\u{201c}{name}\u{201d}) at the {}", a.component.label().to_lowercase(), a.anchor.kind.word()),
                    None => format!("{} at the {}", a.component.label().to_lowercase(), a.anchor.kind.word()),
                };
                if !said.contains(&one) {
                    said.push(one);
                }
            }
            ui.small(if said.is_empty() { "None went on this one: what its kit file chose did not fit, or it chose nothing.".to_string() } else { format!("On it: {}.", said.join("; ")) });
        }
        changed
    }

    /// Age, wealth and how many sleep there: as the variant has them until set by hand.
    /// Returns whether any changed.
    fn household_ui(&mut self, ui: &mut egui::Ui, house: House, variant: u32) -> bool {
        let from = variant_spec(house, variant);
        let now = self.spec(house, variant);
        let h = &mut self.household;
        let mut changed = false;
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Household").strong());
        let (mut age, mut wealth, mut sleepers) = (now.age, now.wealth, now.sleepers);
        // Shown to two places but never rounded, which egui would count as a change, and
        // only a value moved counts as set by hand.
        let two = |v: f64, _| format!("{v:.2}");
        ui.add(egui::Slider::new(&mut age, 0.0..=1.0).custom_formatter(two).text("age")).on_hover_text("0 newly built, 1 old: weathered timber, sagging roofs, render flaking");
        ui.add(egui::Slider::new(&mut wealth, 0.0..=1.0).custom_formatter(two).text("wealth")).on_hover_text("0 poorest, 1 richest: size and finish");
        ui.add(egui::Slider::new(&mut sleepers, 0..=from.step.max_sleepers()).text("sleeping there")).on_hover_text("A bed for each; none for a store");
        if age != now.age {
            h.age = Some(age);
            changed = true;
        }
        if wealth != now.wealth {
            h.wealth = Some(wealth);
            changed = true;
        }
        if sleepers != now.sleepers {
            h.sleepers = Some(sleepers);
            changed = true;
        }
        ui.horizontal(|ui| {
            let by_hand = *h != Household::default();
            ui.small(if by_hand { "Set by hand, for every variant." } else { "As the variant has it." });
            if by_hand && ui.small_button("As the variant has it").clicked() {
                *h = Household::default();
                changed = true;
            }
        });
        changed
    }
}

/// The household the home on view is shown furnished for ("Furnished"): the sleepers of
/// `s`, some of them children, its wealth, worn as it is old, a trade more often than not,
/// and now and then nobody at home, as `variant` has them.
pub fn household(s: &Spec, variant: u32) -> worldgen::settlements::furnish::Household {
    use worldgen::settlements::Livelihood;
    let h = |k: i32| unit(hash2(variant ^ 0x4805_e5ed, k, 1));
    let n = Livelihood::ALL.len();
    let trade = (h(1) < 0.65).then(|| Livelihood::ALL[((h(2) * n as f32) as usize).min(n - 1)]);
    let children = (h(3) * s.sleepers as f32 * 0.6) as u8;
    worldgen::settlements::furnish::Household { region: s.region, sleepers: s.sleepers, children, trade, wealth: s.wealth, neglect: s.age * 0.7, lived_in: h(4) > 0.12 }
}

/// What that household keeps in the home `b`, as the village would furnish it, and a
/// line saying who lives there.
pub fn furnish(s: &Spec, variant: u32, b: &Building) -> (Vec<(IVec3, Voxel)>, Vec<PlantBox>, String) {
    let h = household(s, variant);
    let f = worldgen::settlements::furnish::furnish(b, &h, &palette(s.region, variant), variant);
    let who = if h.lived_in {
        let trade = h.trade.map_or(String::new(), |t| format!(", a {} at work", t.word()));
        format!("Lived in: {} sleeping, {} of them children{trade}; {} things set, {} windows lit", h.sleepers, h.children, f.items.len(), f.items.iter().filter(|i| matches!(i.what, worldgen::settlements::furnish::What::Pane { lit: true })).count())
    } else {
        format!("Empty, {} things left behind", f.items.iter().filter(|i| !matches!(i.what, worldgen::settlements::furnish::What::Pane { .. })).count())
    };
    (f.voxels, f.fine, who)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A kit file saved while the editor is open is read again: a fault in it is named
    /// and the last rules that read are kept, and once put right it is read again.
    #[test]
    fn a_saved_kit_file_is_read_again() {
        let dir = std::env::temp_dir().join(format!("willow-hollow-kit-test-{}", std::process::id()));
        let file = dir.join("kit").join("temperate.txt");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let save = |text: &str| {
            // Long enough for the clock to tick on, as a person saving would.
            std::thread::sleep(Duration::from_millis(30));
            std::fs::write(&file, text).unwrap();
        };
        save("about: first\nhut:\ntimber:\nstone:\n");
        let mut kit = Kit::new(&dir);
        assert_eq!(kit.rules(Region::Temperate).about, "first");
        // A region with no file of its own is built as the game was.
        assert_eq!(kit.rules(Region::Desert), kit::rules::rules(Region::Desert));
        let look = |kit: &mut Kit| {
            kit.checked = Instant::now() - EVERY;
            kit.poll()
        };
        assert!(!look(&mut kit), "nothing saved, nothing read");
        save("about: second\nhut:\n  colour: red\ntimber:\nstone:\n");
        assert!(look(&mut kit));
        let error = kit.file(Region::Temperate).error.clone().unwrap_or_default();
        assert!(error.starts_with("line 3:"), "{error}");
        assert_eq!(kit.rules(Region::Temperate).about, "first");
        save("about: third, put right\nhut:\n  width: 9 11\ntimber:\nstone:\n");
        assert!(look(&mut kit));
        assert!(kit.file(Region::Temperate).error.is_none());
        assert_eq!(kit.rules(Region::Temperate).about, "third, put right");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
