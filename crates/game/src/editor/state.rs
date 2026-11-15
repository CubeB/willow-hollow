//! What the structure editor holds and shows: the structures on disk, the one being
//! looked at or drawn, the tools, and the panels round the view.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use glam::{IVec3, Vec3};
use voxel::{AIR, Materials, Voxel, mesh::PlantBox};
use worldgen::{
    biome::Biome,
    features::{
        designs::{self, Cell, Design, Role},
        landmarks::{self, Landmark, Palette},
        parts::{self, LampStyle},
    },
    materials as mat,
};

use super::view::{self, Grid, Orbit, Vertex};

/// Places to see a structure in: the palette of each, as a site there would have it.
pub const PLACES: [(&str, Biome, Voxel); 9] = [
    ("Broadleaf wood, grey stone", Biome::Deciduous, mat::STONE),
    ("Moor, granite", Biome::Moor, mat::GRANITE),
    ("Grassland, pale rock", Biome::Grassland, mat::ROCK_PALE),
    ("Pine forest, dark rock", Biome::Taiga, mat::ROCK_DARK),
    ("Redwoods, brown rock", Biome::OldGrowth, mat::ROCK_BROWN),
    ("Jungle, grey stone", Biome::Rainforest, mat::STONE),
    ("Desert, sandstone", Biome::Desert, mat::SANDSTONE),
    ("Red rock country", Biome::RedRock, mat::RED_ROCK),
    ("Volcanic isle, basalt", Biome::Volcanic, mat::BASALT),
];

/// The colour of the plain ground the structure stands on.
const GROUND: [u8; 3] = [118, 150, 84];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Build,
    Paint,
    Erase,
    Pick,
    Box,
}

impl Tool {
    const ALL: [(Tool, &'static str, &'static str, &'static str); 5] = [
        (Tool::Build, "Build", "B", "Adds a voxel against the face under the cursor, or on the ground"),
        (Tool::Paint, "Paint", "P", "Changes the material of the voxel under the cursor"),
        (Tool::Erase, "Erase", "E", "Takes away the voxel under the cursor (right click always does)"),
        (Tool::Pick, "Pick", "I", "Takes the material of the voxel under the cursor to build with (middle click always does)"),
        (Tool::Box, "Box", "X", "Click two corners to fill the box between them; right click two corners to empty it"),
    ];
}

/// What is shown.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Showing {
    /// The generator's own variants of a kind (an index into [`designs::KINDS`]).
    Generator(usize),
    /// A saved structure: the kind, and which of its files.
    Saved(usize, usize),
    /// Things that cannot be saved yet, to look at.
    Landing,
    Lantern(usize),
    Signpost,
}

type Edit = Vec<(IVec3, Option<Cell>, Option<Cell>)>;

/// One structure file.
pub struct Entry {
    pub path: PathBuf,
    pub design: Design,
    /// Changed since it was last saved or read.
    pub dirty: bool,
    /// Whether there is a file for it yet.
    pub on_disk: bool,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
}

/// The files of one kind.
pub struct KindFiles {
    pub kind: Landmark,
    /// The generator's weight, if it has a generator.txt.
    pub generator: Option<u32>,
    pub entries: Vec<Entry>,
}

impl KindFiles {
    fn generator_weight(&self) -> u32 {
        self.generator.unwrap_or(if self.entries.iter().any(|e| e.on_disk) { 0 } else { 1 })
    }
}

/// Where the cursor points: the voxel under it, if any, and where a voxel built there
/// would go.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Hit {
    pub cell: Option<IVec3>,
    pub place: IVec3,
}

/// The model as drawn, rebuilt whenever what is shown changes.
#[derive(Default)]
struct Model {
    /// What is drawn, by place.
    at: HashMap<IVec3, Voxel>,
    pick: Option<Grid>,
    lo: IVec3,
    hi: IVec3,
}

/// Settings that start the editor somewhere particular, for screenshots and for coming
/// back to a structure.
#[derive(Clone, Default, Debug)]
pub struct Options {
    /// The structures folder, if not the repository's own.
    pub dir: Option<PathBuf>,
    /// What to show first: a kind (`shrine`), a saved structure (`shrine/high-gate`),
    /// `landing`, `lantern`, `lantern/N` or `signpost`.
    pub show: Option<String>,
    pub variant: Option<u32>,
    /// Cut away everything above this layer.
    pub layer: Option<i32>,
    /// Start drawing on a copy of the variant shown.
    pub copy: bool,
    /// Camera: yaw and pitch in degrees, distance in voxels.
    pub orbit: Option<(f32, f32, Option<f32>)>,
    /// Pretend the cursor is here (in points), for screenshots.
    pub hover: Option<(f32, f32)>,
    pub place: Option<usize>,
    pub screenshot: Option<PathBuf>,
    pub size: (u32, u32),
}

pub struct Editor {
    pub dir: PathBuf,
    pub kinds: Vec<KindFiles>,
    /// Files that could not be read, and why.
    pub problems: Vec<(PathBuf, String)>,
    pub showing: Showing,
    pub variant: u32,
    pub place: usize,
    pub tool: Tool,
    pub brush: Cell,
    pub mirror_x: bool,
    pub mirror_z: bool,
    /// The layer everything above which is cut away, when cutting.
    pub cut: Option<i32>,
    pub show_props: bool,
    pub show_air: bool,
    pub orbit: Orbit,
    /// The part of the window the view takes, in points.
    pub viewport: egui::Rect,
    pub fake_pointer: Option<egui::Pos2>,
    model: Model,
    /// The model needs rebuilding, and the camera framing on it if `reframe`.
    stale: bool,
    reframe: bool,
    /// First corner of a box being drawn, and whether it fills or empties.
    anchor: Option<(IVec3, bool)>,
    hover: Option<Hit>,
    message: String,
    copy_name: String,
    confirm_delete: bool,
    filter: String,
    show_help: bool,
    mats: Materials,
    /// The window was asked to close with changes not saved, and is asking what to do.
    pub closing: bool,
    /// Done: the window may close.
    pub quit: bool,
}

impl Editor {
    pub fn new(opts: &Options) -> Self {
        let dir = opts.dir.clone().unwrap_or_else(default_dir);
        let mut e = Editor {
            dir,
            kinds: Vec::new(),
            problems: Vec::new(),
            showing: Showing::Generator(0),
            variant: opts.variant.unwrap_or(1),
            place: opts.place.unwrap_or(0).min(PLACES.len() - 1),
            tool: Tool::Build,
            brush: Cell::Role(Role::Stone),
            mirror_x: false,
            mirror_z: false,
            cut: opts.layer,
            show_props: true,
            show_air: false,
            orbit: Orbit { target: Vec3::new(0.0, 6.0, 0.0), yaw: 0.8, pitch: 0.5, distance: 60.0 },
            viewport: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)),
            fake_pointer: opts.hover.map(|(x, y)| egui::pos2(x, y)),
            model: Model::default(),
            stale: true,
            reframe: true,
            anchor: None,
            hover: None,
            message: String::new(),
            copy_name: String::new(),
            confirm_delete: false,
            filter: String::new(),
            show_help: true,
            mats: mat::build(),
            closing: false,
            quit: false,
        };
        e.reload();
        if let Some(show) = &opts.show {
            e.show_named(show);
        }
        if opts.copy {
            e.copy_variant();
        }
        if let Some((yaw, pitch, distance)) = opts.orbit {
            e.reframe_now();
            e.orbit.yaw = yaw.to_radians();
            e.orbit.pitch = pitch.to_radians();
            if let Some(d) = distance {
                e.orbit.distance = d;
            }
        }
        e
    }

    /// Reads every structure file again, keeping none of the unsaved changes.
    pub fn reload(&mut self) {
        let (kinds, problems) = read_dir(&self.dir);
        self.kinds = kinds;
        self.problems = problems;
        if let Showing::Saved(k, i) = self.showing
            && i >= self.kinds[k].entries.len()
        {
            self.showing = Showing::Generator(k);
        }
        self.stale = true;
    }

    fn show_named(&mut self, name: &str) {
        let (head, tail) = name.split_once('/').map_or((name, None), |(a, b)| (a, Some(b)));
        match head {
            "landing" => self.select(Showing::Landing),
            "signpost" => self.select(Showing::Signpost),
            "lantern" | "lanterns" => self.select(Showing::Lantern(tail.and_then(|t| t.parse().ok()).unwrap_or(0).min(LampStyle::ALL.len() - 1))),
            _ => {
                let Some(k) = designs::KINDS.iter().position(|&kind| designs::kind_word(kind) == head) else {
                    self.message = format!("There is no kind of structure called {head}");
                    return;
                };
                let file = tail.and_then(|t| self.kinds[k].entries.iter().position(|e| e.path.file_stem().is_some_and(|s| s == t)));
                self.select(match file {
                    Some(i) => Showing::Saved(k, i),
                    None => Showing::Generator(k),
                });
            }
        }
    }

    fn select(&mut self, s: Showing) {
        if self.showing != s {
            self.showing = s;
            self.anchor = None;
            self.confirm_delete = false;
            self.stale = true;
            self.reframe = true;
        }
    }

    fn kind(&self) -> Option<Landmark> {
        match self.showing {
            Showing::Generator(k) | Showing::Saved(k, _) => Some(designs::KINDS[k]),
            Showing::Landing => Some(Landmark::Landing),
            _ => None,
        }
    }

    fn entry(&self) -> Option<&Entry> {
        match self.showing {
            Showing::Saved(k, i) => self.kinds[k].entries.get(i),
            _ => None,
        }
    }

    fn entry_mut(&mut self) -> Option<&mut Entry> {
        match self.showing {
            Showing::Saved(k, i) => self.kinds[k].entries.get_mut(i),
            _ => None,
        }
    }

    fn palette(&self) -> Palette {
        let (_, biome, rock) = PLACES[self.place];
        Palette::local(biome, rock)
    }

    /// The name shown for what is on view.
    pub fn title(&self) -> String {
        match self.showing {
            Showing::Generator(k) => format!("{}: the generator's own, variant {}", kind_label(designs::KINDS[k]), self.variant),
            Showing::Saved(k, i) => format!("{}: {}", kind_label(designs::KINDS[k]), self.kinds[k].entries[i].design.name),
            Showing::Landing => format!("Landing, variant {}", self.variant),
            Showing::Lantern(s) => format!("Trail lantern: {:?}, variant {}", LampStyle::ALL[s], self.variant),
            Showing::Signpost => "Signpost".into(),
        }
    }

    /// Rebuilds the model if anything shown has changed, and hands the view what to draw.
    pub fn prepare(&mut self, gpu: &crate::gfx::Gpu, view: &mut view::View) {
        if self.stale {
            let (opaque, glass) = self.rebuild();
            view.load(gpu, &opaque, &glass);
            self.stale = false;
        }
        if self.reframe {
            self.reframe_now();
        }
        view.load_overlay(gpu, &self.overlay());
    }

    fn reframe_now(&mut self) {
        self.reframe = false;
        let (lo, hi) = (self.model.lo.as_vec3(), self.model.hi.as_vec3() + Vec3::ONE);
        let lo = Vec3::new(lo.x, lo.y.max(0.0), lo.z);
        let size = (hi - lo).max(Vec3::splat(4.0));
        self.orbit.target = (lo + hi) * 0.5;
        let reach = self.reach() as f32;
        self.orbit.distance = size.length().max(reach * 2.2) * 1.25 + 8.0;
        self.orbit.pitch = 0.45;
        self.orbit.yaw = match self.kind().and_then(worldgen::trails::build::front) {
            // From a little to one side of the front, so the way in is seen.
            Some(front) => front as f32 * std::f32::consts::FRAC_PI_2 + 0.6,
            None => 0.8,
        };
    }

    /// Builds the model shown, and the meshes of its solid and see-through parts.
    fn rebuild(&mut self) -> (Vec<Vertex>, Vec<Vertex>) {
        let local = self.palette();
        let seed = self.variant;
        let (mut voxels, mut fine, props) = match self.showing {
            Showing::Generator(k) => landmarks::build_site(designs::KINDS[k], &local, seed, 0),
            Showing::Saved(k, i) => {
                let p = local.varied(seed);
                let d = &self.kinds[k].entries[i].design;
                landmarks::finish(d.kind, &p, seed, 0, d.shape(&p, seed))
            }
            Showing::Landing => landmarks::build_site(Landmark::Landing, &local, seed, 12),
            Showing::Lantern(s) => {
                let (v, f) = landmarks::trail_lantern_styled(&local, 3, seed, LampStyle::ALL[s], 0, parts::GATE_HALF);
                (v, f, Vec::new())
            }
            Showing::Signpost => (landmarks::signpost(&local.varied(seed), &[0.4, 2.3, 4.4]), Vec::new(), Vec::new()),
        };
        // Only the structure itself can be picked and drawn over; props are laid round
        // it by the game, whatever it is.
        let mut pickable: Vec<IVec3> = voxels.iter().filter(|(_, v)| *v != AIR || self.show_air).map(|&(q, _)| q).collect();
        if self.show_props {
            for prop in props {
                voxels.extend(prop.voxels);
                fine.extend(prop.fine);
            }
        }
        let cut = self.cut;
        let shown = |q: &IVec3| cut.is_none_or(|l| q.y <= l);
        let drawn: Vec<(IVec3, Voxel)> = voxels.iter().copied().filter(|(q, v)| *v != AIR && shown(q)).collect();
        let air: Vec<IVec3> = voxels.iter().filter(|(q, v)| *v == AIR && shown(q)).map(|&(q, _)| q).collect();
        let fine: Vec<PlantBox> = fine.into_iter().filter(|b| shown(&IVec3::new(0, b.lo[1].div_euclid(voxel::mesh::CELLS), 0))).collect();
        pickable.retain(|q| shown(q) && (cut.is_some() || q.y >= 0));
        let (mut lo, mut hi) = (IVec3::splat(-2), IVec3::splat(2));
        for (q, _) in &voxels {
            lo = lo.min(*q);
            hi = hi.max(*q);
        }
        self.model = Model {
            at: voxels.iter().copied().collect(),
            pick: Some(Grid::new(pickable.iter().copied())),
            lo,
            hi,
        };
        let solid = Grid::new(drawn.iter().map(|&(q, _)| q));
        let mut opaque = Vec::new();
        view::voxels(&mut opaque, &drawn, &solid, &self.mats);
        view::fine(&mut opaque, &fine, &self.mats);
        let reach = self.reach();
        if cut.is_none_or(|l| l >= 0) {
            // A hair below the layer 0 floor, so the bottoms of voxels there never show
            // through it.
            view::plane(&mut opaque, -0.003, reach, GROUND, 1.0, view::FACE);
        }
        let mut glass = Vec::new();
        if self.show_air {
            for q in air {
                let lo = q.as_vec3() + Vec3::splat(0.08);
                view::cuboid(&mut glass, lo, lo + Vec3::splat(0.84), [120, 170, 230], 0.12, view::OUTLINE, false);
            }
        }
        (opaque, glass)
    }

    /// How far the ground reaches each way from the middle.
    fn reach(&self) -> i32 {
        let site = self.kind().map_or(8.0, |k| k.radius()) as i32;
        let extent = self.model.lo.abs().max(self.model.hi.abs());
        (site + 6).max(extent.x.max(extent.z) + 4)
    }

    /// What is drawn over the model each frame: the cursor's voxel, the layer being
    /// worked on, the levelled ground and the way in.
    fn overlay(&self) -> Vec<Vertex> {
        let mut out = Vec::new();
        let reach = self.reach();
        if let Some(l) = self.cut {
            view::plane(&mut out, l as f32 + 0.004, reach, [250, 250, 250], 0.1, view::GRID);
        }
        // The edge of the levelled ground a site gets.
        if let Some(kind) = self.kind() {
            let r = kind.radius();
            let y = if self.cut.is_some_and(|l| l < 0) { self.cut.unwrap() as f32 + 0.01 } else { 0.01 };
            for k in 0..96 {
                let (a0, a1) = (k as f32 / 96.0 * std::f32::consts::TAU, (k + 1) as f32 / 96.0 * std::f32::consts::TAU);
                let p0 = Vec3::new(a0.cos() * r, y, a0.sin() * r);
                let p1 = Vec3::new(a1.cos() * r, y, a1.sin() * r);
                if k % 2 == 0 {
                    view::arrow(&mut out, p0, (p1 - p0).normalize(), (p1 - p0).length(), [255, 255, 255], 0.55);
                }
            }
            if let Some(front) = worldgen::trails::build::front(kind) {
                let dir = worldgen::features::shape::rotate(IVec3::X, front).as_vec3();
                view::arrow(&mut out, dir * (r + 6.0) + Vec3::Y * y, -dir, 5.0, [255, 214, 90], 0.85);
            }
        }
        let editing = self.entry().is_some();
        if let Some(hit) = self.hover {
            if let Some(q) = hit.cell {
                let lo = q.as_vec3() - Vec3::splat(0.02);
                let red = editing && self.tool == Tool::Erase;
                view::cuboid(&mut out, lo, lo + Vec3::splat(1.04), if red { [255, 80, 60] } else { [255, 255, 255] }, 0.08, view::OUTLINE, false);
            }
            if editing && matches!(self.tool, Tool::Build | Tool::Box) && self.allowed(hit.place) {
                let rgb = self.brush_color();
                let lo = hit.place.as_vec3() + Vec3::splat(0.03);
                view::cuboid(&mut out, lo, lo + Vec3::splat(0.94), rgb, 0.5, view::OUTLINE, false);
            }
        }
        if let Some((a, fill)) = self.anchor {
            let b = self.hover.map_or(a, |h| if fill { h.place } else { h.cell.unwrap_or(h.place) });
            let (lo, hi) = (a.min(b).as_vec3() - Vec3::splat(0.03), a.max(b).as_vec3() + Vec3::splat(1.03));
            view::cuboid(&mut out, lo, hi, if fill { self.brush_color() } else { [255, 80, 60] }, 0.15, view::OUTLINE, false);
        }
        out
    }

    fn brush_color(&self) -> [u8; 3] {
        match self.brush {
            Cell::Air => [120, 170, 230],
            c => self.mats.get(c.resolve(&self.palette().varied(self.variant), self.variant, IVec3::ZERO)).color,
        }
    }

    /// Whether a voxel can be built at `q`: not above the cut.
    fn allowed(&self, q: IVec3) -> bool {
        self.cut.is_none_or(|l| q.y <= l)
    }

    /// What the cursor at `pos` (in points) points at.
    fn hit(&self, pos: egui::Pos2) -> Option<Hit> {
        let r = self.viewport;
        let (o, d) = self.orbit.ray(pos.x - r.min.x, pos.y - r.min.y, r.width(), r.height());
        let cell = self.model.pick.as_ref().and_then(|g| g.cast(o, d, 1e5));
        let floor = self.cut.unwrap_or(0) as f32;
        let reach = self.reach() as f32;
        let plane = (d.y.abs() > 1e-6).then(|| (floor - o.y) / d.y).filter(|&t| t > 0.0).filter(|&t| {
            let p = o + d * t;
            p.x.abs() <= reach && p.z.abs() <= reach
        });
        match (cell, plane) {
            (Some((q, n, t)), p) if p.is_none_or(|pt| t <= pt) => Some(Hit { cell: Some(q), place: q + n }),
            (_, Some(t)) => {
                let p = o + d * t;
                Some(Hit { cell: None, place: IVec3::new(p.x.floor() as i32, floor as i32, p.z.floor() as i32) })
            }
            _ => None,
        }
    }

    /// The places one click changes: `q`, and its mirror images.
    fn mirrored(&self, q: IVec3) -> Vec<IVec3> {
        let mut out = vec![q];
        if self.mirror_x {
            out.push(IVec3::new(-q.x, q.y, q.z));
        }
        if self.mirror_z {
            for p in out.clone() {
                out.push(IVec3::new(p.x, p.y, -p.z));
            }
        }
        out.sort_unstable_by_key(|p| (p.x, p.y, p.z));
        out.dedup();
        out
    }

    /// Makes a change to the structure being drawn, as one step to undo.
    fn change(&mut self, to: Vec<(IVec3, Option<Cell>)>) {
        let Some(entry) = self.entry_mut() else { return };
        let mut edit = Edit::new();
        for (q, after) in to {
            let before = entry.design.cells.get(&q).copied();
            if before != after && !edit.iter().any(|(p, _, _)| *p == q) {
                edit.push((q, before, after));
            }
        }
        if edit.is_empty() {
            return;
        }
        apply(&mut entry.design, &edit, false);
        entry.undo.push(edit);
        entry.redo.clear();
        entry.dirty = true;
        self.stale = true;
    }

    pub fn undo(&mut self) {
        if let Some(entry) = self.entry_mut()
            && let Some(edit) = entry.undo.pop()
        {
            apply(&mut entry.design, &edit, true);
            entry.redo.push(edit);
            entry.dirty = true;
            self.stale = true;
        }
    }

    pub fn redo(&mut self) {
        if let Some(entry) = self.entry_mut()
            && let Some(edit) = entry.redo.pop()
        {
            apply(&mut entry.design, &edit, false);
            entry.undo.push(edit);
            entry.dirty = true;
            self.stale = true;
        }
    }

    /// Acts on a click in the view.
    fn click(&mut self, hit: Hit, button: egui::PointerButton) {
        use egui::PointerButton::*;
        if self.entry().is_none() {
            if matches!(self.showing, Showing::Generator(_)) {
                self.message = "This is the generator's own. Copy this variant (on the right) to draw on it.".into();
            }
            return;
        }
        let pick = button == Middle || (button == Primary && self.tool == Tool::Pick);
        if pick {
            if let Some(q) = hit.cell
                && let Some(c) = self.entry().and_then(|e| e.design.cells.get(&q).copied())
            {
                self.brush = c;
                self.message = format!("Building with {}", cell_label(c));
                self.tool = if self.tool == Tool::Pick { Tool::Build } else { self.tool };
            }
            return;
        }
        if self.tool == Tool::Box {
            let fill = button == Primary;
            let corner = if fill { Some(hit.place) } else { hit.cell.or(Some(hit.place)) };
            let Some(corner) = corner.filter(|q| self.allowed(*q)) else { return };
            match self.anchor {
                Some((a, f)) if f == fill => {
                    self.anchor = None;
                    let (lo, hi) = (a.min(corner), a.max(corner));
                    let mut to = Vec::new();
                    for y in lo.y..=hi.y {
                        for z in lo.z..=hi.z {
                            for x in lo.x..=hi.x {
                                for q in self.mirrored(IVec3::new(x, y, z)) {
                                    to.push((q, fill.then_some(self.brush)));
                                }
                            }
                        }
                    }
                    self.change(to);
                }
                _ => {
                    self.anchor = Some((corner, fill));
                    self.message = "Now click the opposite corner. Esc to stop.".into();
                }
            }
            return;
        }
        let erase = button == Secondary || self.tool == Tool::Erase;
        if erase {
            if let Some(q) = hit.cell {
                let to = self.mirrored(q).into_iter().map(|q| (q, None)).collect();
                self.change(to);
            }
        } else if self.tool == Tool::Paint {
            if let Some(q) = hit.cell {
                let brush = self.brush;
                let to = self.mirrored(q).into_iter().filter(|q| self.entry().is_some_and(|e| e.design.cells.contains_key(q))).map(|q| (q, Some(brush))).collect();
                self.change(to);
            }
        } else if self.allowed(hit.place) {
            let brush = self.brush;
            let to = self.mirrored(hit.place).into_iter().map(|q| (q, Some(brush))).collect();
            self.change(to);
        } else {
            self.message = "That is above the cut: raise the layer (Page Up) to build higher.".into();
        }
    }

    /// Starts a saved structure from the variant on view.
    pub fn copy_variant(&mut self) {
        let Showing::Generator(k) = self.showing else { return };
        let kind = designs::KINDS[k];
        let name = if self.copy_name.trim().is_empty() { format!("{} {}", kind_label(kind), self.variant) } else { self.copy_name.trim().to_string() };
        let palette = self.palette().varied(self.variant);
        let design = Design::copy_of(kind, &palette, self.variant, &name);
        self.add_entry(k, design);
        self.message = format!("Copied variant {} as \u{201c}{name}\u{201d}. Draw on it, then save (Ctrl+S).", self.variant);
        self.copy_name.clear();
    }

    fn add_entry(&mut self, k: usize, design: Design) {
        let folder = self.dir.join(designs::kind_word(design.kind));
        let stem = designs::slug(&design.name);
        let mut path = folder.join(format!("{stem}.txt"));
        let mut n = 2;
        while path.exists() || self.kinds[k].entries.iter().any(|e| e.path == path) {
            path = folder.join(format!("{stem}-{n}.txt"));
            n += 1;
        }
        self.kinds[k].entries.push(Entry { path, design, dirty: true, on_disk: false, undo: Vec::new(), redo: Vec::new() });
        let i = self.kinds[k].entries.len() - 1;
        self.select(Showing::Saved(k, i));
    }

    pub fn save(&mut self) {
        if let Showing::Saved(k, i) = self.showing {
            self.message = self.save_entry(k, i);
        }
    }

    /// Whether anything has changed since it was saved.
    pub fn unsaved(&self) -> bool {
        self.kinds.iter().any(|k| k.entries.iter().any(|e| e.dirty))
    }

    fn save_all(&mut self) {
        for k in 0..self.kinds.len() {
            for i in 0..self.kinds[k].entries.len() {
                if self.kinds[k].entries[i].dirty {
                    self.message = self.save_entry(k, i);
                }
            }
        }
    }

    /// Writes one structure's file, and says how that went.
    fn save_entry(&mut self, k: usize, i: usize) -> String {
        let entry = &mut self.kinds[k].entries[i];
        let text = entry.design.to_text();
        let result = entry.path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&entry.path, &text));
        match result {
            Ok(()) => {
                entry.dirty = false;
                entry.on_disk = true;
                // Read back, to be sure what was written is what is drawn.
                match Design::parse(&text) {
                    Ok(back) if back == entry.design => format!("Saved {}. Build the game again to see it in the world.", short(&entry.path)),
                    Ok(_) => format!("Saved {}, but it reads back differently: please report this.", short(&entry.path)),
                    Err(e) => format!("Saved {}, but it does not read back: {e}", short(&entry.path)),
                }
            }
            Err(e) => format!("Could not save {}: {e}", entry.path.display()),
        }
    }

    fn revert(&mut self) {
        let Some(entry) = self.entry_mut() else { return };
        if !entry.on_disk {
            return;
        }
        match std::fs::read_to_string(&entry.path).map_err(|e| e.to_string()).and_then(|t| Design::parse(&t)) {
            Ok(d) => {
                entry.design = d;
                entry.dirty = false;
                entry.undo.clear();
                entry.redo.clear();
                self.stale = true;
                self.message = "Back to how it was saved.".into();
            }
            Err(e) => self.message = format!("Could not read it again: {e}"),
        }
    }

    fn delete(&mut self) {
        let Showing::Saved(k, i) = self.showing else { return };
        let entry = &self.kinds[k].entries[i];
        if entry.on_disk
            && let Err(e) = std::fs::remove_file(&entry.path)
        {
            self.message = format!("Could not delete {}: {e}", entry.path.display());
            return;
        }
        self.message = format!("Deleted \u{201c}{}\u{201d}.", entry.design.name);
        self.kinds[k].entries.remove(i);
        self.select(Showing::Generator(k));
    }

    fn set_generator_weight(&mut self, k: usize, weight: u32) {
        let path = self.dir.join(designs::kind_word(designs::KINDS[k])).join("generator.txt");
        let result = std::fs::create_dir_all(path.parent().unwrap()).and_then(|_| std::fs::write(&path, designs::generator_text(weight)));
        match result {
            Ok(()) => {
                self.kinds[k].generator = Some(weight);
                self.message = format!("Saved the generator's weight in {}.", short(&path));
            }
            Err(e) => self.message = format!("Could not save {}: {e}", path.display()),
        }
    }

    /// Draws the panels and handles the mouse and keys. Called once a frame.
    pub fn ui(&mut self, ctx: &egui::Context) {
        self.keys(ctx);
        ctx.root_ui(|ui| {
            egui::Panel::top("structures top").show(ui, |ui| self.top_bar(ui));
            egui::Panel::bottom("structures status").show(ui, |ui| self.status_bar(ui));
            egui::Panel::left("structures list").exact_size(250.0).show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.list(ui));
            });
            egui::Panel::right("structures tools").exact_size(300.0).show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.tools(ui));
            });
            egui::CentralPanel::no_frame().show(ui, |ui| self.viewport_ui(ui));
        });
        if self.closing {
            egui::Window::new("Changes not saved").anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).collapsible(false).resizable(false).show(ctx, |ui| {
                ui.label("Some structures have changes that are not saved.");
                ui.horizontal(|ui| {
                    if ui.button("Save them and close").clicked() {
                        self.save_all();
                        self.quit = !self.unsaved();
                    }
                    if ui.button("Close without saving").clicked() {
                        self.quit = true;
                    }
                    if ui.button("Keep working").clicked() {
                        self.closing = false;
                    }
                });
            });
        }
    }

    fn keys(&mut self, ctx: &egui::Context) {
        use egui::{Key, Modifiers};
        let (redo, undo, save) = ctx.input_mut(|i| {
            let redo = i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z) || i.consume_key(Modifiers::COMMAND, Key::Y);
            (redo, i.consume_key(Modifiers::COMMAND, Key::Z), i.consume_key(Modifiers::COMMAND, Key::S))
        });
        if redo {
            self.redo();
        }
        if undo {
            self.undo();
        }
        if save {
            self.save();
        }
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let pressed: Vec<Key> = ctx.input(|i| {
            if i.modifiers.command || i.modifiers.alt {
                return Vec::new();
            }
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key { key, pressed: true, .. } => Some(*key),
                    _ => None,
                })
                .collect()
        });
        for key in pressed {
            match key {
                Key::B => self.tool = Tool::Build,
                Key::P => self.tool = Tool::Paint,
                Key::E => self.tool = Tool::Erase,
                Key::I => self.tool = Tool::Pick,
                Key::X => self.tool = Tool::Box,
                Key::ArrowLeft => self.step_variant(-1),
                Key::ArrowRight => self.step_variant(1),
                Key::PageUp => self.step_cut(1),
                Key::PageDown => self.step_cut(-1),
                Key::L => self.set_cut(if self.cut.is_some() { None } else { Some(self.model.hi.y) }),
                Key::F => self.reframe = true,
                Key::H | Key::F1 => self.show_help = !self.show_help,
                Key::Escape => self.anchor = None,
                Key::Num0 => self.brush = Cell::Air,
                k => {
                    let numbers = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8];
                    if let Some(n) = numbers.iter().position(|&d| d == k) {
                        self.brush = Cell::Role(Role::ALL[n]);
                    }
                }
            }
        }
    }

    fn step_variant(&mut self, by: i32) {
        self.variant = (self.variant as i64 + by as i64).rem_euclid(1 << 16) as u32;
        self.stale = true;
    }

    fn set_cut(&mut self, cut: Option<i32>) {
        if self.cut != cut {
            self.cut = cut;
            self.anchor = None;
            self.stale = true;
        }
    }

    fn step_cut(&mut self, by: i32) {
        let (lo, hi) = (self.model.lo.y, self.model.hi.y + 1);
        let now = self.cut.unwrap_or(if by > 0 { lo - 1 } else { hi + 1 });
        self.set_cut(Some((now + by).clamp(lo, hi)));
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Structures");
            ui.separator();
            ui.label(egui::RichText::new(self.title()).strong());
            if let Some(e) = self.entry() {
                if e.dirty {
                    ui.label(egui::RichText::new("(not saved)").color(egui::Color32::from_rgb(230, 160, 60)));
                }
                let (can_undo, can_redo) = (!e.undo.is_empty(), !e.redo.is_empty());
                ui.separator();
                if ui.add_enabled(can_undo, egui::Button::new("Undo")).on_hover_text("Ctrl+Z").clicked() {
                    self.undo();
                }
                if ui.add_enabled(can_redo, egui::Button::new("Redo")).on_hover_text("Ctrl+Y or Ctrl+Shift+Z").clicked() {
                    self.redo();
                }
                if ui.button("Save").on_hover_text("Ctrl+S").clicked() {
                    self.save();
                }
            }
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            match self.hover {
                Some(hit) => {
                    let q = hit.cell.unwrap_or(hit.place);
                    let what = match (hit.cell, self.entry()) {
                        (Some(q), Some(e)) => e.design.cells.get(&q).map_or("nothing".into(), |c| cell_label(*c)),
                        (Some(q), None) => self.model.at.get(&q).and_then(|&v| mat::name_of(v)).unwrap_or("air").to_ascii_lowercase().replace('_', " "),
                        (None, _) => "the ground".into(),
                    };
                    ui.label(format!("Layer {}   x {}  z {}   {what}", q.y, q.x, q.z));
                }
                None => {
                    ui.label("Drag to turn, wheel to zoom, Shift+drag to move the view");
                }
            }
            if !self.message.is_empty() {
                ui.separator();
                ui.label(&self.message);
            }
        });
    }

    fn list(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("Landmarks").strong());
        ui.small("Each has the generator's own variants, and any you have saved.");
        let mut chosen = None;
        for (k, files) in self.kinds.iter().enumerate() {
            let open = matches!(self.showing, Showing::Generator(j) | Showing::Saved(j, _) if j == k);
            egui::CollapsingHeader::new(kind_label(files.kind)).id_salt(k).open(open.then_some(true)).show(ui, |ui| {
                if ui.selectable_label(self.showing == Showing::Generator(k), format!("The generator's own  (weight {})", files.generator_weight())).clicked() {
                    chosen = Some(Showing::Generator(k));
                }
                for (i, e) in files.entries.iter().enumerate() {
                    let star = if e.dirty { " (not saved)" } else { "" };
                    let text = format!("{}  (weight {}){star}", e.design.name, e.design.weight);
                    if ui.selectable_label(self.showing == Showing::Saved(k, i), text).on_hover_text(short(&e.path)).clicked() {
                        chosen = Some(Showing::Saved(k, i));
                    }
                }
            });
        }
        ui.add_space(8.0);
        ui.label(egui::RichText::new("To look at").strong());
        ui.small("These follow the shore or the trail and cannot be saved yet.");
        if ui.selectable_label(self.showing == Showing::Landing, "Landing").clicked() {
            chosen = Some(Showing::Landing);
        }
        for (s, style) in LampStyle::ALL.iter().enumerate() {
            if ui.selectable_label(self.showing == Showing::Lantern(s), format!("Trail lantern: {style:?}")).clicked() {
                chosen = Some(Showing::Lantern(s));
            }
        }
        if ui.selectable_label(self.showing == Showing::Signpost, "Signpost").clicked() {
            chosen = Some(Showing::Signpost);
        }
        if !self.problems.is_empty() {
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Files that could not be read").strong().color(egui::Color32::from_rgb(230, 110, 90)));
            for (path, why) in &self.problems {
                ui.small(format!("{}: {why}", short(path)));
            }
        }
        ui.add_space(8.0);
        if ui.button("Read the files again").on_hover_text("Throws away changes not yet saved").clicked() {
            self.reload();
        }
        ui.small(format!("Folder: {}", self.dir.display()));
        if let Some(s) = chosen {
            self.select(s);
        }
    }

    fn tools(&mut self, ui: &mut egui::Ui) {
        match self.showing {
            Showing::Generator(k) => self.generator_tools(ui, k),
            Showing::Saved(..) => self.design_tools(ui),
            _ => {
                ui.label("Only to look at: the game builds these to fit the shore or the trail.");
                self.preview(ui);
            }
        }
        ui.add_space(10.0);
        egui::CollapsingHeader::new("How to use").default_open(self.show_help).show(ui, help);
    }

    /// Choosing the variant and the place it is seen in.
    fn preview(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Variant");
            if ui.button("<").on_hover_text("Left arrow").clicked() {
                self.step_variant(-1);
            }
            let mut v = self.variant;
            if ui.add(egui::DragValue::new(&mut v).range(0..=65535)).changed() {
                self.variant = v;
                self.stale = true;
            }
            if ui.button(">").on_hover_text("Right arrow").clicked() {
                self.step_variant(1);
            }
            if ui.button("Any").on_hover_text("A variant at random").clicked() {
                self.variant = (self.variant.wrapping_mul(2654435761).wrapping_add(12345) >> 8) % 65536;
                self.stale = true;
            }
        });
        let mut place = self.place;
        egui::ComboBox::from_label("Seen in").selected_text(PLACES[place].0).show_ui(ui, |ui| {
            for (k, p) in PLACES.iter().enumerate() {
                ui.selectable_value(&mut place, k, p.0);
            }
        });
        if place != self.place {
            self.place = place;
            self.stale = true;
        }
        if ui.checkbox(&mut self.show_props, "Show props").on_hover_text("Barrels, crates, woodpiles: the game lays them round every structure of the kind").changed() {
            self.stale = true;
        }
        if ui.checkbox(&mut self.show_air, "Show carved air").on_hover_text("Voxels that clear the ground or plants away, drawn as blue boxes").changed() {
            self.stale = true;
        }
    }

    fn layers(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("Layers").strong());
        let mut cutting = self.cut.is_some();
        if ui.checkbox(&mut cutting, "Cut away everything above a layer").on_hover_text("L, then Page Up and Page Down").changed() {
            self.set_cut(if cutting { Some(self.model.hi.y.min(4)) } else { None });
        }
        if let Some(mut l) = self.cut {
            let (lo, hi) = (self.model.lo.y, self.model.hi.y + 1);
            if ui.add(egui::Slider::new(&mut l, lo..=hi).text("layer")).changed() {
                self.set_cut(Some(l));
            }
            ui.small("You build on the grid of this layer. 0 is the first layer above the ground; below it are footings in the ground.");
        }
    }

    fn generator_tools(&mut self, ui: &mut egui::Ui, k: usize) {
        let kind = designs::KINDS[k];
        ui.label(egui::RichText::new(format!("{}: the generator's own", kind_label(kind))).strong());
        ui.small("Each variant is what a different site makes. Step through them to find one to start from.");
        ui.add_space(4.0);
        self.preview(ui);
        ui.add_space(6.0);
        self.layers(ui);
        ui.add_space(8.0);
        ui.label(egui::RichText::new("Make your own").strong());
        ui.horizontal(|ui| {
            ui.label("Name");
            ui.add(egui::TextEdit::singleline(&mut self.copy_name).hint_text(format!("{} {}", kind_label(kind), self.variant)).desired_width(170.0));
        });
        if ui.button("Copy this variant to edit").on_hover_text("Starts a structure of your own from what is shown").clicked() {
            self.copy_variant();
        }
        if ui.button("Start from nothing").clicked() {
            let name = if self.copy_name.trim().is_empty() { format!("New {}", kind_label(kind).to_lowercase()) } else { self.copy_name.trim().to_string() };
            self.add_entry(k, Design::new(kind, &name));
            self.copy_name.clear();
        }
        ui.add_space(8.0);
        ui.label(egui::RichText::new("How often in the world").strong());
        let mut w = self.kinds[k].generator_weight();
        ui.horizontal(|ui| {
            ui.label("Weight");
            if ui.add(egui::DragValue::new(&mut w).range(0..=100)).changed() {
                self.set_generator_weight(k, w);
            }
        });
        ui.small("Each site of this kind is built from the generator or one of your saved ones, chosen in proportion to their weights. Once one is saved, the generator's weight is 0 until you change it here.");
    }

    fn design_tools(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette().varied(self.variant);
        if let Some(entry) = self.entry_mut() {
            ui.horizontal(|ui| {
                ui.label("Name");
                if ui.text_edit_singleline(&mut entry.design.name).changed() {
                    entry.dirty = true;
                }
            });
            ui.small(format!("{}{}", short(&entry.path), if entry.on_disk { "" } else { " (not saved yet)" }));
            ui.horizontal(|ui| {
                ui.label("Weight");
                if ui.add(egui::DragValue::new(&mut entry.design.weight).range(0..=100)).on_hover_text("How often it is chosen against the others of its kind").changed() {
                    entry.dirty = true;
                }
            });
        }
        ui.add_space(6.0);
        ui.label(egui::RichText::new("Tool").strong());
        ui.horizontal_wrapped(|ui| {
            for (tool, name, key, about) in Tool::ALL {
                if ui.selectable_label(self.tool == tool, format!("{name} ({key})")).on_hover_text(about).clicked() {
                    self.tool = tool;
                    self.anchor = None;
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("Mirror");
            ui.checkbox(&mut self.mirror_x, "left\u{2013}right").on_hover_text("Every change is made on the other side of x = 0 too");
            ui.checkbox(&mut self.mirror_z, "front\u{2013}back").on_hover_text("Every change is made on the other side of z = 0 too");
        });
        ui.add_space(6.0);
        ui.label(egui::RichText::new("Build with").strong());
        ui.small("These take their material from the place, as the generator's do: the stone of the rock there, the timber of its trees.");
        let mut pick = None;
        let swatch = |ui: &mut egui::Ui, selected: bool, rgb: [u8; 3], text: &str, about: &str| -> bool {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 3.0, egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
                if selected {
                    ui.painter().rect_stroke(rect.expand(2.0), 4.0, egui::Stroke::new(2.0, egui::Color32::WHITE), egui::StrokeKind::Outside);
                }
                ui.selectable_label(selected, text).on_hover_text(about).clicked()
            })
            .inner
        };
        for (n, r) in Role::ALL.into_iter().enumerate() {
            let rgb = self.mats.get(r.resolve(&palette, self.variant, IVec3::ZERO)).color;
            if swatch(ui, self.brush == Cell::Role(r), rgb, &format!("{}  ({})", r.label(), n + 1), r.about()) {
                pick = Some(Cell::Role(r));
            }
        }
        if swatch(ui, self.brush == Cell::Air, [120, 170, 230], "Air  (0)", "Clears away the ground or plants where it is: hollows, doorways dug into a bank") {
            pick = Some(Cell::Air);
        }
        egui::CollapsingHeader::new("Other materials").show(ui, |ui| {
            ui.small("These are the same everywhere.");
            ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("Search").desired_width(200.0));
            let filter = self.filter.to_ascii_lowercase();
            egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                for &(name, v) in mat::NAMES {
                    let label = name.to_ascii_lowercase().replace('_', " ");
                    if !filter.is_empty() && !label.contains(&filter) {
                        continue;
                    }
                    if swatch(ui, self.brush == Cell::Material(v), self.mats.get(v).color, &label, name) {
                        pick = Some(Cell::Material(v));
                    }
                }
            });
        });
        if let Some(c) = pick {
            self.brush = c;
            if self.tool == Tool::Pick || self.tool == Tool::Erase {
                self.tool = Tool::Build;
            }
        }
        ui.add_space(6.0);
        self.layers(ui);
        ui.add_space(6.0);
        ui.label(egui::RichText::new("See it as").strong());
        ui.small("The variant and the place change only how it is drawn here: the roles take each place's materials.");
        self.preview(ui);
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Save").on_hover_text("Ctrl+S").clicked() {
                self.save();
            }
            let on_disk = self.entry().is_some_and(|e| e.on_disk);
            if ui.add_enabled(on_disk, egui::Button::new("Revert")).on_hover_text("Back to how it was last saved").clicked() {
                self.revert();
            }
            if self.confirm_delete {
                if ui.button(egui::RichText::new("Really delete").color(egui::Color32::from_rgb(240, 110, 90))).clicked() {
                    self.delete();
                    self.confirm_delete = false;
                }
            } else if ui.button("Delete\u{2026}").clicked() {
                self.confirm_delete = true;
            }
        });
    }

    fn viewport_ui(&mut self, ui: &mut egui::Ui) {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
        self.viewport = rect;
        let shift = ui.input(|i| i.modifiers.shift);
        let delta = response.drag_delta();
        if response.dragged_by(egui::PointerButton::Middle) || (shift && response.dragged()) {
            self.orbit.pan(delta.x, delta.y, rect.height());
        } else if response.dragged() {
            self.orbit.turn(delta.x, delta.y);
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta().y);
            if scroll != 0.0 {
                self.orbit.zoom(scroll / 40.0);
            }
        }
        let pointer = response.hover_pos().or(self.fake_pointer);
        self.hover = if response.dragged() { None } else { pointer.and_then(|p| self.hit(p)) };
        if let Some(hit) = self.hover {
            for button in [egui::PointerButton::Primary, egui::PointerButton::Secondary, egui::PointerButton::Middle] {
                if response.clicked_by(button) {
                    self.click(hit, button);
                }
            }
        }
        // Words in the view: the way in, and the levelled ground's edge.
        let painter = ui.painter_at(rect);
        let label = |at: Vec3, text: &str| {
            if let Some((x, y)) = self.orbit.project(at, rect.width(), rect.height()) {
                let at = egui::pos2(rect.min.x + x, rect.min.y + y);
                painter.text(at + egui::vec2(1.0, 1.0), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(14.0), egui::Color32::from_black_alpha(160));
                painter.text(at, egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(14.0), egui::Color32::WHITE);
            }
        };
        if let Some(kind) = self.kind() {
            let r = kind.radius();
            match worldgen::trails::build::front(kind) {
                Some(front) => {
                    let dir = worldgen::features::shape::rotate(IVec3::X, front).as_vec3();
                    label(dir * (r + 5.0) + Vec3::Y * 1.5, "The trail comes in here");
                }
                None => label(Vec3::new(0.0, 0.0, r + 2.5), "Turned any way round in the world"),
            }
            let corner = r * std::f32::consts::FRAC_1_SQRT_2;
            label(Vec3::new(-corner, 0.0, -corner), "Edge of the levelled ground");
        }
        if self.entry().is_some_and(|e| e.design.cells.is_empty()) {
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, "Click on the ground to lay the first voxel", egui::FontId::proportional(18.0), egui::Color32::WHITE);
        }
    }
}

/// Makes `edit`'s changes, or takes them back.
fn apply(d: &mut Design, edit: &Edit, back: bool) {
    for &(q, before, after) in edit {
        match if back { before } else { after } {
            Some(c) => d.cells.insert(q, c),
            None => d.cells.remove(&q),
        };
    }
}

/// How the editor's panels give a kind.
pub fn kind_label(kind: Landmark) -> &'static str {
    match kind {
        Landmark::Shrine => "Shrine",
        Landmark::Lookout => "Lookout tower",
        Landmark::Camp => "Camp",
        Landmark::Ruins => "Ruins",
        Landmark::Stones => "Standing stones",
        Landmark::Cairn => "Cairn",
        Landmark::Arch => "Arch",
        Landmark::Hut => "Hut",
        Landmark::Landing => "Landing",
    }
}

fn cell_label(c: Cell) -> String {
    match c {
        Cell::Air => "air".into(),
        Cell::Role(r) => r.label().to_lowercase(),
        Cell::Material(v) => mat::name_of(v).unwrap_or("?").to_ascii_lowercase().replace('_', " "),
    }
}

/// A path as far down as the structures folder: `shrine/high-gate.txt`.
fn short(path: &Path) -> String {
    let parts: Vec<String> = path.components().rev().take(2).map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    parts.into_iter().rev().collect::<Vec<_>>().join("/")
}

/// The repository's structures folder, where the game this was built from reads them.
pub fn default_dir() -> PathBuf {
    let built = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/structures");
    match built.canonicalize() {
        // Without the \\?\ Windows puts in front of a canonical path, for showing.
        Ok(p) => PathBuf::from(p.to_string_lossy().trim_start_matches(r"\\?\")),
        Err(_) => PathBuf::from("assets/structures"),
    }
}

/// Reads every kind's folder.
fn read_dir(dir: &Path) -> (Vec<KindFiles>, Vec<(PathBuf, String)>) {
    let mut problems = Vec::new();
    let mut kinds = Vec::new();
    for kind in designs::KINDS {
        let folder = dir.join(designs::kind_word(kind));
        let mut files: Vec<PathBuf> = std::fs::read_dir(&folder).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "txt")).collect();
        files.sort();
        let mut k = KindFiles { kind, generator: None, entries: Vec::new() };
        for path in files {
            let text = match std::fs::read_to_string(&path) {
                Ok(t) => t,
                Err(e) => {
                    problems.push((path, e.to_string()));
                    continue;
                }
            };
            if path.file_stem().is_some_and(|s| s.eq_ignore_ascii_case("generator")) {
                match designs::parse_generator(&text) {
                    Ok(w) => k.generator = Some(w),
                    Err(e) => problems.push((path, e)),
                }
                continue;
            }
            match Design::parse(&text) {
                Ok(d) if d.kind == kind => k.entries.push(Entry { path, design: d, dirty: false, on_disk: true, undo: Vec::new(), redo: Vec::new() }),
                Ok(d) => problems.push((path, format!("it says it is a {}, but is in the {} folder", designs::kind_word(d.kind), designs::kind_word(kind)))),
                Err(e) => problems.push((path, e)),
            }
        }
        kinds.push(k);
    }
    (kinds, problems)
}

/// The controls, as the panel lists them.
pub const CONTROLS: [(&str, &str); 14] = [
    ("Drag", "turn the view"),
    ("Shift+drag or middle drag", "move the view"),
    ("Wheel", "zoom"),
    ("F", "frame the structure again"),
    ("Left click", "use the tool"),
    ("Right click", "erase (in Box, empty a box)"),
    ("Middle click", "pick up the material clicked"),
    ("B P E I X", "build, paint, erase, pick, box"),
    ("1 to 8, 0", "stone ... accent, air"),
    ("L, Page Up/Down", "cut away above a layer, and move the cut"),
    ("Left/Right arrows", "the variant before or after"),
    ("Ctrl+Z, Ctrl+Y", "undo, redo"),
    ("Ctrl+S", "save"),
    ("Esc", "let go of a box's first corner"),
];

fn help(ui: &mut egui::Ui) {
    for (keys, what) in CONTROLS {
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(keys).strong());
            ui.label(what);
        });
    }
    ui.add_space(4.0);
    ui.small("Saved structures go in the repository, under assets/structures. Build the game again and the world uses them: see docs/DEVELOPMENT.md.");
}
