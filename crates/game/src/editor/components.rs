//! The regional components (`worldgen::settlements::kit::components`) in the structure
//! editor: each on a stand, a stretch of a kit home it goes on cut out about its anchor,
//! turned so the anchor is at the origin and out from the house is +X (east), as a
//! component's design is drawn; and galleries of a region's components side by side, each
//! new and let go.

use glam::{IVec3, Vec3};
use voxel::{AIR, Voxel, mesh::PlantBox};
use worldgen::{
    features::designs::Design,
    settlements::{
        Region,
        kit::components::{self, Component},
    },
};

use super::houses;

/// How far along the wall and into the house a stand keeps of its home.
const REACH: i32 = 6;

/// The galleries of components: every one, or a region's. The word is the one
/// `--structure components/WORD` takes.
pub fn galleries() -> Vec<(String, &'static str, Option<Region>)> {
    let mut out = vec![("Every component".to_string(), "all", None)];
    out.extend(Region::ALL.into_iter().map(|r| (houses::region_label(r).to_string(), r.word(), Some(r))));
    out
}

/// The components of a gallery, in the order they are laid out.
pub fn of_gallery(region: Option<Region>) -> Vec<Component> {
    Component::ALL.iter().copied().filter(|c| region.is_none_or(|r| c.regions().contains(&r))).collect()
}

/// A component as the editor draws it.
pub struct Shown {
    pub voxels: Vec<(IVec3, Voxel)>,
    pub fine: Vec<PlantBox>,
    /// A design fits where it was drawn on some home; the generator's own always does.
    pub fits: bool,
    /// The layer of the ground under the stand: below the anchor's for a raised door.
    pub ground: i32,
}

/// Component `c` for `variant` on its stand: the generator's own, or `design` where it
/// fits on a home. A design that fits nowhere is drawn on the stand the generator's own
/// would have, so what is in the way shows.
pub fn shown(c: Component, variant: u32, design: Option<&Design>) -> Shown {
    if let Some(s) = components::stand(c, variant, design, houses::palette) {
        let (voxels, fine) = s.parts(REACH);
        let ground = s.ground;
        return Shown { voxels, fine, fits: true, ground };
    }
    let Some(d) = design else { return Shown { voxels: Vec::new(), fine: Vec::new(), fits: false, ground: 0 } };
    let s = components::stand(c, variant, None, houses::palette);
    let pal = houses::palette(s.as_ref().map_or(Region::Temperate, |s| s.region), variant);
    let bare = s.as_ref().map(|s| components::Stand { made: components::Made::default(), ..s.clone() });
    let (mut voxels, mut fine) = bare.as_ref().map(|s| s.parts(REACH)).unwrap_or_default();
    let mut shape = d.shape(&pal, variant);
    fine.extend(shape.take_fine());
    voxels.extend(shape.into_voxels().into_iter().filter(|(_, v)| *v != AIR));
    let ground = bare.as_ref().map_or(0, |s| s.ground);
    Shown { voxels, fine, fits: false, ground }
}

/// The generator's own component `c` for `variant` as a design to edit, named `name`.
pub fn copy(c: Component, variant: u32, name: &str) -> Option<Design> {
    components::copy(c, variant, name, houses::palette)
}

/// A gallery laid out in rows from the front back, each component new and then let go
/// to its right: their voxels and fine parts, the room each takes on the ground, and a
/// label over each pair.
#[allow(clippy::type_complexity)]
pub fn gallery(region: Option<Region>, variant: u32) -> (Vec<(IVec3, Voxel)>, Vec<PlantBox>, Vec<(IVec3, IVec3)>, Vec<(Vec3, String)>) {
    // Pairs wrap onto a new row past this many columns.
    const ROW: i32 = 90;
    let (mut voxels, mut fine, mut spaces, mut labels) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let (mut x, mut z, mut depth) = (0, 0, 0);
    for c in of_gallery(region) {
        // Each stood on the ground, a raised door's as high as its floor.
        let pair: Vec<(Vec<(IVec3, Voxel)>, Vec<PlantBox>)> = [false, true]
            .into_iter()
            .filter_map(|decayed| components::stand_aged(c, variant, None, decayed, houses::palette))
            .map(|s| {
                let (v, f) = s.parts(REACH);
                let up = IVec3::Y * -s.ground;
                (v.into_iter().map(|(q, v)| (q + up, v)).collect(), f.into_iter().map(|p| moved(p, up)).collect())
            })
            .collect();
        if pair.is_empty() {
            continue;
        }
        let bounds: Vec<(IVec3, IVec3)> = pair.iter().map(|(v, f)| extent(v, f)).collect();
        let wide: i32 = bounds.iter().map(|(lo, hi)| hi.x - lo.x + 1).sum::<i32>() + 3;
        let deep = bounds.iter().map(|(lo, hi)| hi.z - lo.z + 1).max().unwrap_or(1);
        if x > 0 && x + wide > ROW {
            x = 0;
            z += depth + 6;
            depth = 0;
        }
        let mut at = x;
        let mut top = 0;
        for ((v, f), (lo, hi)) in pair.into_iter().zip(&bounds) {
            let d = IVec3::new(at - lo.x, 0, z - lo.z);
            voxels.extend(v.into_iter().map(|(q, v)| (q + d, v)));
            fine.extend(f.into_iter().map(|p| moved(p, d)));
            spaces.push((IVec3::new(lo.x, 0, lo.z) + d, IVec3::new(hi.x + 1, 1, hi.z + 1) + d));
            top = top.max(hi.y);
            at += hi.x - lo.x + 4;
        }
        labels.push((Vec3::new(x as f32 + wide as f32 * 0.5, top as f32 + 3.0, z as f32 + deep as f32 * 0.5), c.label().to_string()));
        x += wide + 6;
        depth = depth.max(deep);
    }
    (voxels, fine, spaces, labels)
}

/// The voxels a component's parts cover, inclusive.
fn extent(voxels: &[(IVec3, Voxel)], fine: &[PlantBox]) -> (IVec3, IVec3) {
    let (mut lo, mut hi) = (IVec3::MAX, IVec3::MIN);
    for (q, _) in voxels {
        lo = lo.min(*q);
        hi = hi.max(*q);
    }
    let cells = IVec3::splat(voxel::mesh::CELLS);
    for p in fine {
        lo = lo.min(IVec3::from(p.lo).div_euclid(cells));
        hi = hi.max((IVec3::from(p.hi) - 1).div_euclid(cells));
    }
    if lo.x > hi.x { (IVec3::ZERO, IVec3::ZERO) } else { (lo, hi) }
}

fn moved(p: PlantBox, d: IVec3) -> PlantBox {
    let c = d * voxel::mesh::CELLS;
    PlantBox { lo: (IVec3::from(p.lo) + c).to_array(), hi: (IVec3::from(p.hi) + c).to_array(), ..p }
}

/// What the panel says of a component: where it goes, in which regions, and the most a
/// home takes.
pub fn about(c: Component) -> String {
    let anchors: Vec<&str> = c.anchors().iter().map(|a| a.word()).collect();
    let regions: Vec<String> = c.regions().into_iter().map(|r| houses::region_label(r).to_lowercase()).collect();
    format!(
        "Goes at {} anchors, in {} homes; at most {} to a home. Its weight against what else goes there is in each region's kit file.",
        anchors.join(", "),
        if regions.is_empty() { "no region's".to_string() } else { regions.join(", ") },
        c.most()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every component is shown on a stand, and every gallery has something in it.
    #[test]
    fn every_component_has_a_stand() {
        for &c in Component::ALL {
            let s = shown(c, 1, None);
            assert!(s.fits && !(s.voxels.is_empty() && s.fine.is_empty()), "{} has a stand", c.word());
        }
        for (_, _, region) in galleries() {
            let (voxels, fine, spaces, labels) = gallery(region, 1);
            assert!(!voxels.is_empty() && !fine.is_empty() && !spaces.is_empty() && !labels.is_empty());
        }
    }

    /// Lists what goes on the kit's homes of one region in the Houses view, variant by
    /// variant, to find one to look at: `HOUSES_OF=temperate cargo test ...
    /// editor::components::tests::which -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn which() {
        let Ok(want) = std::env::var("HOUSES_OF") else { return };
        let region = Region::ALL.iter().position(|r| r.word() == want).expect("a region");
        let houses = houses::Houses::new(&super::super::state::default_dir());
        for step in 0..3 {
            for variant in 0..40 {
                let s = houses.spec((region, step), variant);
                let (_, attached) = worldgen::settlements::kit::generate_dressed(&s, &houses::palette(s.region, variant), houses.kit.rules(s.region), &components::Dress::default());
                let words: Vec<&str> = attached.iter().map(|a| a.component.word()).collect();
                println!("{} variant {variant}: {}", s.step.word(), words.join(" "));
            }
        }
    }
}
