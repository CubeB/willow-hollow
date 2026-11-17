//! The homes of the house kit (`worldgen::settlements::kit`) in the structure editor: a
//! region's hut, timber house or stone house, a different one for each variant. They are
//! only to look at; their rules are the files under `assets/structures/kit`.

use glam::IVec3;
use voxel::{Voxel, mesh::PlantBox};
use worldgen::{
    features::landmarks::Palette,
    materials as mat,
    noise::{hash2, unit},
    settlements::{
        Region,
        kit::{self, Building, Spec, Step},
    },
};

/// Which home is shown: a region and a step, as indices into [`Region::ALL`] and
/// [`Step::ALL`].
pub type House = (usize, usize);

fn region_label(r: Region) -> &'static str {
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

/// `house/<region>/<step>` on the command line: `house/wetland/hut`.
pub fn named(tail: Option<&str>) -> House {
    let mut words = tail.unwrap_or("").split('/');
    let region = words.next().and_then(|w| Region::ALL.iter().position(|r| r.word() == w)).unwrap_or(0);
    let step = words.next().and_then(|w| Step::ALL.iter().position(|s| s.word() == w)).unwrap_or(0);
    (region, step)
}

/// What variant `variant` asks of the kit: its age, wealth and household come from it.
pub fn spec((region, step): House, variant: u32) -> Spec {
    let h = |k: i32| unit(hash2(variant ^ 0x4805_e5ed, k, 0));
    let step = Step::ALL[step];
    let most = step.max_sleepers();
    Spec { region: Region::ALL[region], step, age: h(1), wealth: h(2), sleepers: (1 + (h(3) * most as f32) as u8).min(most), seed: variant }
}

pub fn title(house: House, variant: u32) -> String {
    format!("Houses: {}, {}, variant {variant}", region_label(Region::ALL[house.0]), Step::ALL[house.1].label().to_lowercase())
}

/// The home, of the stone and timber of its region's country, and its voxels and fine
/// parts to draw.
pub fn build(house: House, variant: u32) -> (Building, Vec<(IVec3, Voxel)>, Vec<PlantBox>) {
    let s = spec(house, variant);
    let biome = s.region.biome();
    let pal = Palette::local(biome, mat::bedrock(biome).0).varied(variant);
    let b = kit::build(&s, &pal);
    let (voxels, fine) = (b.voxels.clone(), b.fine.clone());
    (b, voxels, fine)
}

/// The region, step and variant to show, and what the variant made. Returns the home to
/// show if it changed, and whether the variant did.
pub fn tools(ui: &mut egui::Ui, house: House, variant: &mut u32, built: Option<&Building>) -> (Option<House>, bool) {
    let mut next = house;
    ui.label(egui::RichText::new("Houses").strong());
    ui.small("The house kit's homes, in the stone and timber of their own country. Each variant is another household: older or newer, poorer or richer, more or fewer in it. The rules are in assets/structures/kit, one file to a region.");
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
    let s = spec(house, *variant);
    ui.small(format!("Age {:.2}, wealth {:.2}, {} sleeping there", s.age, s.wealth, s.sleepers));
    if let Some(b) = built {
        ui.small(format!("{}; {} beds, {} places for furniture, {} windows", b.style.describe(), b.inside.beds.len(), b.inside.furniture.len(), b.inside.windows.len()));
    }
    let about = &kit::rules::rules(Region::ALL[house.0]).about;
    if !about.is_empty() {
        ui.small(about);
    }
    ((next != house).then_some(next), *variant != was)
}
