//! The house kit: procedural huts, timber houses and stone houses in each region's way of
//! building, every one of them walked into through its door (see #201 and #203).
//!
//! Nobody draws the homes. [`build`] makes one from a [`Spec`] (region, step, age,
//! wealth, how many sleep there, a seed) and the [`Palette`] of the place it stands, so
//! its stone is the rock in the ground and its timber the trees round about. What it
//! makes is in landmark model space: the origin column is the middle of the site, `y = 0`
//! is the first layer above the ground, and footings go down below it. The door is always
//! on the +X side, so a caller turns the whole building to face its lane with
//! [`Building::turned`] (the turns of [`crate::features::shape::rotate`]).
//!
//! Besides its voxels and fine parts, a [`Building`] says where things go: its door; the
//! anchors where the regional components of #203 attach (porches, awnings, woodpiles);
//! and inside, its floor, a bed for each sleeper, the hearth, spots along the walls for
//! furniture, and the windows.
//!
//! # The rules, as data
//!
//! How each region builds is a plain text file, `assets/structures/kit/<region>.txt`
//! (`temperate`, `upland`, `northern`, `alpine`, `desert`, `wetland`, `rainforest`,
//! `oldgrowth`), compiled in like the structures beside it. `#` starts a comment.
//!
//! ```text
//! about: Temperate lowland: cob and wattle under thatch, and timber frames.
//! hut:
//!   footprint: rect 4, round 1      # a weighted list
//!   width: 10 13                    # a range: wealth chooses along it for sizes
//!   walls: cob 3, wattle 2 poor     # `rich` or `poor` tips a weight with wealth
//!   lean-to: 0.3 rich               # a chance, 0 to 1, tipped the same way
//! timber:
//!   ...
//! stone:
//!   ...
//! components:
//!   door: porch 2 rich, rain-butt 1, nothing 3
//! ```
//!
//! There is a section for each step, `hut:`, `timber:` and `stone:`, and every file has
//! all three. A line under one is `rule: value`. A weighted list is options separated by
//! commas, each a word, a weight (1 if left out) and `rich` or `poor` if wealth makes it
//! commoner (twice as common at the richest, never at the poorest) or rarer. A range is
//! one number, or the lowest and the highest; sizes take wealth's share of the way along
//! it, the rest the seed's. A chance is a number from 0 to 1, with `rich` or `poor` after
//! it to tip it the same way. Sizes are in voxels of 0.4 m. The rules, and what a file
//! leaves out (a plain timber cottage):
//!
//! | Rule | Value | Left out |
//! |---|---|---|
//! | `footprint` | list of `rect`, `l` (a wing at the back), `round` | rect |
//! | `width`, `length` | range: the short and the long side outside the walls (a round one is `width` across) | 12–15, 15–20 |
//! | `thick` | range: wall thickness, 1 or 2 | 1 |
//! | `walls` | list of `cob`, `wattle`, `drystone`, `log`, `frame`, `stone`, `mudbrick`, `plank`, `reed` | plank |
//! | `base` | range: courses of stone at the foot of the walls | 1 |
//! | `wall height` | range: floor to the top of the walls, at least 5 | 6–7 |
//! | `roof` | list of `thatch`, `reed`, `palm`, `turf`, `slate`, `tile`, `shingle`, `mud` | thatch |
//! | `roof shape` | list of `gable`, `hip`, `cone` (round buildings take it anyway), `flat` | gable |
//! | `pitch` | range: rise over run, 1 is 45 degrees | 1–1.2 |
//! | `overhang` | range: how far the roof reaches past the walls | 1–2 |
//! | `gable door` | chance the door is in a gable end, under the ridge | 0 |
//! | `floor` | list of `earth`, `flags`, `boards` | boards |
//! | `raised` | range: the floor's height above the ground | 0 |
//! | `under` | list of what holds a raised floor: `stilts`, `plinth`, `roots` | stilts |
//! | `veranda` | range: depth of the platform or step before the door | 1 |
//! | `smoke` | list of `hole` (a hearth in the middle) and `chimney` (a fireplace in a wall) | hole |
//! | `lean-to` | chance of an open shed against a gable end | 0 |
//! | `windows` | range: how many | 1–3 |
//! | `door width`, `door height` | ranges: the doorway, at least 2 by 5 | 3, 5 |
//! | `whitewash` | chance the walls are limewashed | 0 |
//! | `roof stones` | chance of rows of stones on battens holding the roof down | 0 |
//! | `courtyard` | chance of a walled yard before the door, its gate the way in | 0 |
//! | `parapet` | range: height of a flat roof's parapet | 1 |
//! | `round corners` | chance a rectangle's corners are cut, as cob and drystone are | 0 |
//!
//! Under `components:` each line is a kind of anchor (`door`, `eaves`, `gable`, `ridge`,
//! `wall`, `corner`, `roof`, `yard`, `window`) and a weighted list of the components that
//! go there, by name, with `nothing` for none. The kit only carries these weights for now;
//! the components come with #203.
//!
//! Whatever the rules say, the kit keeps a building walk-in: doorways at least two
//! voxels wide and five high, five voxels of headroom over every floor cell and under the
//! eaves before the door, and no room narrower than a person. It grows a plan past the
//! sizes the rules give until there is a bed for everyone (a round hut that would have to
//! grow too far is built square), and lowers a pitch that would lift the roof so far over
//! a wide room that moss would take the room for outdoors.
//!
//! # Houses saved from the editor
//!
//! Any of the kit's homes can be copied in the structure editor into a design of kind
//! `house` ([`copy`]), edited by hand and saved under `assets/structures/house/`. [`build`]
//! then chooses, by the seed, between the kit and the houses saved for the same region
//! and step, each by its weight (see [`crate::features::designs`]). A saved house keeps
//! the door, beds and the rest the kit gave it ([`Home`]), and its floor is found again by
//! walking in ([`walk`]) whenever it is built. With nothing saved, [`build`] is the kit.

pub mod rules;
pub mod walk;

mod copy;
mod make;
mod plan;
#[cfg(test)]
mod tests;

use glam::{IVec2, IVec3};
use voxel::{Voxel, mesh::PlantBox};

use crate::{
    features::{designs, landmarks::Palette, parts, shape::rotate},
    settlements::Region,
};
pub use copy::{Home, copy, from_design};
pub use rules::{Cover, Form, RoofShape, Rules, WallKind};
pub use walk::{Walk, walk};

/// How far up the ladder of building a home is.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Step {
    Hut,
    Timber,
    Stone,
}

impl Step {
    pub const ALL: [Step; 3] = [Step::Hut, Step::Timber, Step::Stone];

    /// Its word in kit files and tools.
    pub fn word(self) -> &'static str {
        match self {
            Step::Hut => "hut",
            Step::Timber => "timber",
            Step::Stone => "stone",
        }
    }

    /// Its name in the editor.
    pub fn label(self) -> &'static str {
        match self {
            Step::Hut => "Hut",
            Step::Timber => "Timber house",
            Step::Stone => "Stone house",
        }
    }

    /// The most a home of this step sleeps; a [`Spec`] asking for more gets this many.
    pub fn max_sleepers(self) -> u8 {
        match self {
            Step::Hut => 4,
            Step::Timber => 8,
            Step::Stone => 10,
        }
    }
}

/// What to build.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Spec {
    pub region: Region,
    pub step: Step,
    /// 0 new to 1 old: weathered timber, patched and sagging roofs, render flaking.
    pub age: f32,
    /// 0 poorest to 1 richest: size and finish.
    pub wealth: f32,
    /// How many sleep there: a bed each, up to [`Step::max_sleepers`]. 0 for a store.
    pub sleepers: u8,
    pub seed: u32,
}

/// The kinds of place a regional component attaches to.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum AnchorKind {
    /// Before the door, on the ground or the step: a porch, an awning, a water jar.
    Door,
    /// Under the edge of the roof: herbs drying, hay, nets.
    Eaves,
    /// High on a gable end: a dovecote, carved boards.
    Gable,
    /// Along the ridge: finials, ridge decoration.
    Ridge,
    /// On a stretch of wall: a woodpile, a bench, a trellis.
    WallFace,
    /// Outside a corner, on the ground: a rain butt, a peat stack.
    Corner,
    /// On top: the smoke's way out, or a flat roof's terrace.
    RoofTop,
    /// On the ground about the building, clear of it: a hive, a hen coop, a sail shade.
    Yard,
    /// Outside a window: a window box, shutters, a lattice.
    Window,
}

impl AnchorKind {
    pub const ALL: [AnchorKind; 9] = [
        AnchorKind::Door,
        AnchorKind::Eaves,
        AnchorKind::Gable,
        AnchorKind::Ridge,
        AnchorKind::WallFace,
        AnchorKind::Corner,
        AnchorKind::RoofTop,
        AnchorKind::Yard,
        AnchorKind::Window,
    ];

    /// Its word in kit files.
    pub fn word(self) -> &'static str {
        match self {
            AnchorKind::Door => "door",
            AnchorKind::Eaves => "eaves",
            AnchorKind::Gable => "gable",
            AnchorKind::Ridge => "ridge",
            AnchorKind::WallFace => "wall",
            AnchorKind::Corner => "corner",
            AnchorKind::RoofTop => "roof",
            AnchorKind::Yard => "yard",
            AnchorKind::Window => "window",
        }
    }
}

/// A place a component attaches: `at` is the empty voxel against the surface where it
/// starts, and `normal` points away from the building (straight up on a roof, diagonally
/// out at a corner).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Anchor {
    pub kind: AnchorKind,
    pub at: IVec3,
    pub normal: IVec3,
}

/// A place for something on the floor against a wall: a bed, a chest, a table. It covers
/// `len` voxels along `along` and `wide` voxels along `facing` from `at`, the voxel by
/// the wall at one end, at the height of a person's feet (the layer above the floor).
/// `facing` points away from the wall, into the room.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Spot {
    pub at: IVec3,
    pub along: IVec3,
    pub facing: IVec3,
    pub len: i32,
    pub wide: i32,
}

impl Spot {
    /// Every voxel it covers, at the height of the feet.
    pub fn cells(&self) -> impl Iterator<Item = IVec3> + '_ {
        (0..self.len).flat_map(move |i| (0..self.wide).map(move |j| self.at + self.along * i + self.facing * j))
    }
}

/// Where the fire is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Hearth {
    /// The voxel the fire burns in, above the hearth stone. A central hearth is two by
    /// two from here, toward +X and +Z.
    pub at: IVec3,
    /// Into the room from a fireplace; straight up from a central hearth.
    pub facing: IVec3,
    /// In the middle of the floor, the smoke going out through the roof; or else in a
    /// fireplace in a wall, under a chimney.
    pub central: bool,
}

/// A window: the opening through the wall, an inclusive box of voxels.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Opening {
    pub lo: IVec3,
    pub hi: IVec3,
    /// Out of the building.
    pub normal: IVec3,
}

/// The way in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Door {
    /// The voxel underfoot in the doorway, at the middle of its width (toward -Z when the
    /// width is even). For a house behind a walled yard, the yard's gate.
    pub floor: IVec3,
    /// The way out through it: +X as built.
    pub facing: IVec3,
    pub width: i32,
    /// Clear height above the floor voxel.
    pub height: i32,
}

/// What is inside.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Inside {
    /// Every floor voxel of the room, the voxel stood on (a person's feet are in the layer
    /// above).
    pub floor: Vec<IVec3>,
    /// A bed for each sleeper, two voxels wide and five long, along a wall (in a round
    /// hut, where the wall curves away from its ends, touching it). Nothing is built
    /// there: the beds themselves are furniture's to make.
    pub beds: Vec<Spot>,
    pub hearth: Option<Hearth>,
    /// Other places along the walls for furniture.
    pub furniture: Vec<Spot>,
    pub windows: Vec<Opening>,
}

/// An area of columns, x and z (in `y`), inclusive.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Area {
    pub lo: IVec2,
    pub hi: IVec2,
}

impl Area {
    fn turned(self, turns: u32) -> Area {
        let a = rotate(IVec3::new(self.lo.x, 0, self.lo.y), turns);
        let b = rotate(IVec3::new(self.hi.x, 0, self.hi.y), turns);
        Area { lo: IVec2::new(a.x.min(b.x), a.z.min(b.z)), hi: IVec2::new(a.x.max(b.x), a.z.max(b.z)) }
    }
}

/// What the kit chose for a building, to say what it is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Style {
    pub form: Form,
    pub walls: WallKind,
    pub roof: Cover,
    pub shape: RoofShape,
    /// How far the floor stands above the ground, in voxels.
    pub raised: i32,
}

impl Style {
    /// A few words for it: "cob walls, thatched gable roof".
    pub fn describe(&self) -> String {
        let shape = match self.shape {
            RoofShape::Gable => "gabled",
            RoofShape::Hip => "hipped",
            RoofShape::Cone => "conical",
            RoofShape::Flat => "flat",
        };
        let raised = if self.raised > 0 { format!(", raised {:.1} m", self.raised as f32 * 0.4) } else { String::new() };
        format!("{} {} walls, {shape} {} roof{raised}", self.form.word(), self.walls.word(), self.roof.word())
    }
}

/// A home built by the kit, in landmark model space with its door to +X.
#[derive(Clone, Debug)]
pub struct Building {
    /// Every voxel, air included where the building clears the ground or plants away
    /// (its rooms, its doorway), sorted.
    pub voxels: Vec<(IVec3, Voxel)>,
    /// Parts finer than a voxel, in model cells of 1/[`voxel::mesh::CELLS`] voxel: door
    /// leaves, shutters, sills, the kerb of a hearth.
    pub fine: Vec<PlantBox>,
    /// Every column it covers above the ground, its eaves, steps and yard included.
    pub footprint: Area,
    /// The columns inside its outer walls.
    pub walls: Area,
    pub door: Door,
    pub anchors: Vec<Anchor>,
    pub inside: Inside,
    /// Boxes of voxels (inclusive) to keep clear of props: the way in, the rooms.
    pub clear: Vec<(IVec3, IVec3)>,
    /// Where the smoke comes out: above the chimney or the smoke hole.
    pub smoke: IVec3,
    pub style: Style,
}

/// Builds the home `spec` asks for, of the materials of palette `pal` (taken as it is:
/// vary it for the place first, as landmarks do, if wanted). This is what the world
/// builds: the kit's own, or a house saved for its region and step when the seed picks
/// one (see [`build_with`]).
pub fn build(spec: &Spec, pal: &Palette) -> Building {
    build_with(spec, pal, rules::rules(spec.region))
}

/// [`build`] with `rules` in place of the rules compiled in for the region: the structure
/// editor's, read from the kit's files as they are saved. A house saved for the region
/// and step is chosen by the seed against the kit, each by its weight, and only for a
/// household it has a bed for each of; it is built as it was drawn, whatever the age and
/// wealth asked for.
pub fn build_with(spec: &Spec, pal: &Palette, rules: &Rules) -> Building {
    designs::pick_house(spec.region, spec.step, spec.seed, spec.sleepers)
        .and_then(|d| from_design(d, pal, spec.seed))
        .unwrap_or_else(|| generate(spec, pal, rules))
}

/// The kit's own home for `spec` by `rules`, never a saved house.
pub fn generate(spec: &Spec, pal: &Palette, rules: &Rules) -> Building {
    make::make(spec, pal, rules.step(spec.step))
}

impl PartialEq for Building {
    fn eq(&self, o: &Self) -> bool {
        // Fine parts compare field by field, as PlantBox has no equality of its own.
        let same = |a: &PlantBox, b: &PlantBox| {
            (a.lo, a.hi, a.voxel, a.sway_lo, a.sway_hi, a.thin, a.season, a.flap) == (b.lo, b.hi, b.voxel, b.sway_lo, b.sway_hi, b.thin, b.season, b.flap)
        };
        self.voxels == o.voxels
            && self.fine.len() == o.fine.len()
            && self.fine.iter().zip(&o.fine).all(|(a, b)| same(a, b))
            && (self.footprint, self.walls, self.door, self.smoke, self.style) == (o.footprint, o.walls, o.door, o.smoke, o.style)
            && (&self.anchors, &self.inside, &self.clear) == (&o.anchors, &o.inside, &o.clear)
    }
}

impl Building {
    /// The building turned by quarter turns about the vertical through the origin, as
    /// [`rotate`] turns: one turn takes its door from +X to +Z.
    pub fn turned(&self, turns: u32) -> Building {
        let r = |q: IVec3| rotate(q, turns);
        let spot = |s: &Spot| Spot { at: r(s.at), along: r(s.along), facing: r(s.facing), ..*s };
        let boxed = |lo: IVec3, hi: IVec3| {
            let (a, b) = (r(lo), r(hi));
            (a.min(b), a.max(b))
        };
        let mut voxels: Vec<(IVec3, Voxel)> = self.voxels.iter().map(|&(q, v)| (r(q), v)).collect();
        voxels.sort_unstable_by_key(|(q, _)| (q.y, q.x, q.z));
        Building {
            voxels,
            fine: parts::place_fine(&self.fine, IVec3::ZERO, turns),
            footprint: self.footprint.turned(turns),
            walls: self.walls.turned(turns),
            door: Door { floor: r(self.door.floor), facing: r(self.door.facing), ..self.door },
            anchors: self.anchors.iter().map(|a| Anchor { at: r(a.at), normal: r(a.normal), ..*a }).collect(),
            inside: Inside {
                floor: self.inside.floor.iter().map(|&q| r(q)).collect(),
                beds: self.inside.beds.iter().map(spot).collect(),
                hearth: self.inside.hearth.map(|h| Hearth { at: r(h.at), facing: r(h.facing), ..h }),
                furniture: self.inside.furniture.iter().map(spot).collect(),
                windows: self
                    .inside
                    .windows
                    .iter()
                    .map(|w| {
                        let (lo, hi) = boxed(w.lo, w.hi);
                        Opening { lo, hi, normal: r(w.normal) }
                    })
                    .collect(),
            },
            clear: self.clear.iter().map(|&(lo, hi)| boxed(lo, hi)).collect(),
            smoke: r(self.smoke),
            style: self.style,
        }
    }
}
