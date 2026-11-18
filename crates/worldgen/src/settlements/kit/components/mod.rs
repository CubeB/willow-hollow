//! The regional components (#203): what makes a region's homes its own, set at the
//! anchors the kit marks on every building ([`super::Anchor`]). A desert house gets a
//! cloth awning over its door and jars by it, a lowland cottage a porch and window boxes,
//! a northern house a woodpile stacked to the eaves and grass and flowers on its turf
//! roof, a marsh hut rope rails and a punt in the yard.
//!
//! Each [`Component`] is a small generator. Given an anchor (where, which way out, what
//! kind), the building's style and palette, how rich and how old the household is and a
//! seed (a [`Request`]), it lays voxels and fine parts in the building's model space
//! ([`Made`]). It works in the anchor's own frame: the anchor voxel is the origin, out
//! from the building is +X, up is +Y, and along the wall is Z, so the wall it stands
//! against is at x = -1. A corner's frame has both +X and +Z outward; a roof or a yard's,
//! +X across the ridge or away from the house.
//!
//! [`dress`] is how the kit uses them: each region's rules file says, for each kind of
//! anchor, which components go there and how often, tipped by wealth
//! ([`super::rules::Rules::components`]). The door, the windows, the gables, eaves,
//! ridge and roof each take one choice for the whole building, so its windows match; a
//! stretch of wall, a corner or a spot in the yard each take their own. A component
//! goes in only where it fits whole: its voxels where the building says nothing and
//! outside the ways in and the rooms it keeps clear, its fine parts out of anything solid
//! and anyone else's, and every part held by the building, the ground or the rest of it.
//! What does not fit is left out, never cut short. The anchors a component takes are
//! taken off the building, so what is set about it later (a village's log stacks and
//! water butts) goes elsewhere.
//!
//! Some of what each region's table in #203 asks for the kit builds itself, and is not
//! here: stilts, raised floors and root plinths; parapets and flat roofs; whitewash;
//! verandas and walled courtyards; stones on battens holding roofs down; and the deep
//! overhangs of the forests and the mountains.
//!
//! Every component is also a kind of saved structure (`kind: component`, in
//! `assets/structures/component/`), drawn in the anchor's frame in the structure editor.
//! A saved one is chosen against its generator by weight, as saved houses are against
//! the kit; one that does not fit where it is chosen gives way to the generator.
//!
//! [`public`] dresses a settlement's open spaces with the same things: sail shades over
//! lanes, awnings over stalls, lanterns hung from branches and boardwalks.

mod common;
mod dry;
mod green;
mod north;
pub mod public;
mod wet;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap, HashSet};

use glam::{IVec2, IVec3};
use voxel::{
    AIR, Voxel,
    mesh::{CELLS, PlantBox},
};

use super::{Anchor, AnchorKind, Building, Spec, Step, Style, footprint, rules::{Choice, Rules, choose}, walk};
use crate::{
    features::{
        cloth,
        designs::{self, Cell, Design, Role},
        fittings::{self, Fitting},
        landmarks::Palette,
        parts,
        shape::{Rng, rotate},
    },
    noise::{hash2, hash3, unit},
    settlements::Region,
};

/// Defines the components: each its word in files, its name in the editor, the kinds of
/// anchor it goes at, and the most of it one building takes.
macro_rules! components {
    ($($(#[$m:meta])* $v:ident = $w:literal, $label:literal, [$($a:ident),+], $most:literal;)*) => {
        /// One regional component.
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
        pub enum Component { $($(#[$m])* $v),* }

        impl Component {
            pub const ALL: &'static [Component] = &[$(Component::$v),*];

            /// Its word in kit files and saved designs.
            pub fn word(self) -> &'static str {
                match self { $(Component::$v => $w),* }
            }

            /// Its name in the editor.
            pub fn label(self) -> &'static str {
                match self { $(Component::$v => $label),* }
            }

            /// The kinds of anchor it goes at.
            pub fn anchors(self) -> &'static [AnchorKind] {
                match self { $(Component::$v => &[$(AnchorKind::$a),+]),* }
            }

            /// The most of it one building takes.
            pub fn most(self) -> usize {
                match self { $(Component::$v => $most),* }
            }
        }
    };
}

components! {
    // The desert and the savanna.
    /// Cloth stretched over a door on two poles, or over a window on arms.
    Awning = "awning", "Awning", [Door, Window], 4;
    /// A sail of cloth on posts, two corners high and two low, over a yard or a terrace.
    SailShade = "sail-shade", "Sail shade", [Yard, RoofTop], 1;
    /// A flat roof lived on: a mat, cushions and jars under a shade.
    Terrace = "terrace", "Roof terrace", [RoofTop], 1;
    /// A screen of turned wood before a window, to see out of unseen.
    Lattice = "lattice", "Lattice screen", [Window], 4;
    /// Clay jars of water, by the door or against a wall.
    WaterJar = "water-jar", "Water jars", [Door, WallFace, Corner], 2;
    /// Palm fronds laid over poles: shade against a wall or in the yard.
    PalmShade = "palm-shade", "Palm-frond shade", [WallFace, Yard], 1;
    /// The door's jambs and lintel painted blue, green or ochre.
    DoorFrame = "door-frame", "Painted door frame", [Door], 1;
    /// A tower on a flat roof catching the wind down into the house: great towns only.
    WindCatcher = "wind-catcher", "Wind catcher", [RoofTop], 1;
    /// A beehive oven of clay.
    Oven = "oven", "Clay oven", [Yard], 1;
    /// Mats of dates and peppers drying on a flat roof.
    DryingMats = "drying-mats", "Drying mats", [RoofTop], 1;
    // Temperate lowland.
    /// A timber porch on two posts before the door.
    Porch = "porch", "Porch", [Door], 1;
    /// A box of flowers under a window, empty where nobody keeps it.
    WindowBox = "window-box", "Window box", [Window], 4;
    /// A butt catching the rain.
    RainButt = "rain-butt", "Rain butt", [Corner, Door, WallFace], 2;
    /// A dovecote high on a gable.
    Dovecote = "dovecote", "Dovecote", [Gable], 1;
    /// A rose climbing the wall, in flower in summer.
    ClimbingRose = "climbing-rose", "Climbing rose", [WallFace, Door], 2;
    /// Ivy up a wall to the eaves.
    Ivy = "ivy", "Ivy", [WallFace, Corner], 2;
    /// The kit's shutters painted, and hanging askew or gone where the house is let go.
    Shutters = "shutters", "Painted shutters", [Window], 6;
    /// Bunches of herbs drying under the eaves.
    Herbs = "herbs", "Herbs drying", [Eaves], 2;
    /// A bench against the wall.
    Bench = "bench", "Bench", [Door, WallFace], 1;
    /// Split logs stacked against a wall: to the eaves in the north and the mountains.
    Woodpile = "woodpile", "Woodpile", [WallFace, Door, Corner], 2;
    /// A hen coop in the yard.
    HenCoop = "hen-coop", "Hen coop", [Yard], 1;
    /// Hives in the yard.
    Hives = "hives", "Hives", [Yard], 1;
    // Moor and upland.
    /// Ropes over the thatch weighted with stones hanging at the eaves.
    ThatchWeights = "thatch-weights", "Thatch weights", [Eaves], 4;
    /// Cut peat stacked to dry.
    PeatStack = "peat-stack", "Peat stack", [WallFace, Door, Corner, Yard], 1;
    /// A porch walled at the sides against the wind.
    WindPorch = "wind-porch", "Porch against the wind", [Door], 1;
    /// A drystone wall sheltering the yard.
    LeeWall = "lee-wall", "Lee wall", [Yard], 1;
    /// A bench of stone and turf against the wall.
    TurfBench = "turf-bench", "Turf bench", [WallFace, Door], 1;
    /// A stone trough of water.
    WaterTrough = "water-trough", "Water trough", [Corner, Yard], 1;
    /// A small drystone fold for sheep.
    SheepFold = "sheep-fold", "Sheep fold", [Yard], 1;
    // Taiga and tundra.
    /// Grass and flowers growing on a turf roof.
    TurfFlowers = "turf-flowers", "Grass and flowers on the roof", [Ridge, RoofTop], 8;
    /// Carved boards under the gable's eaves, and a sun at its peak.
    CarvedBoards = "carved-boards", "Carved gable boards", [Gable], 2;
    /// A food store raised on posts out of the reach of mice.
    RaisedStore = "raised-store", "Store on posts", [Yard], 1;
    /// Hides hung across the doorway.
    HideCurtain = "hide-curtain", "Hides across the door", [Door], 1;
    /// A rack of fish drying.
    FishRack = "fish-rack", "Fish rack", [Yard], 1;
    /// A frame with a hide stretched on it.
    DryingRack = "drying-rack", "Hide frame", [Yard], 1;
    // The mountains.
    /// A timber balcony on the gable, hay drying over its rail.
    Balcony = "balcony", "Balcony with hay", [Gable], 1;
    /// A rack of hay drying on poles.
    HayRack = "hay-rack", "Hay rack", [Yard], 1;
    /// A small bell in a cote on the ridge.
    Bell = "bell", "Bell", [Ridge], 1;
    // Marsh, mangrove and coast.
    /// Planks on posts from the door out over wet ground.
    Boardwalk = "boardwalk", "Boardwalk", [Door], 1;
    /// Rope rails on posts either side of the way in.
    RopeRail = "rope-rail", "Rope rail", [Door], 1;
    /// A screen of bound reeds.
    ReedScreen = "reed-screen", "Reed screen", [WallFace, Yard], 2;
    /// A rack with nets hung on it.
    NetRack = "net-rack", "Net rack", [Yard, Door], 1;
    /// Nets hung to dry from the eaves.
    DryingNets = "drying-nets", "Nets drying", [Eaves], 2;
    /// Eel traps of woven withy.
    EelTraps = "eel-traps", "Eel traps", [WallFace, Yard], 1;
    /// A flat-bottomed punt and its pole.
    Punt = "punt", "Punt", [Yard], 1;
    /// A fence of driftwood stakes.
    DriftwoodFence = "driftwood-fence", "Driftwood fence", [Yard], 2;
    /// An old boat turned over for a shed.
    UpturnedBoat = "upturned-boat", "Upturned boat", [Yard], 1;
    /// Sheaves of cut reed stood to dry.
    ReedStack = "reed-stack", "Reed stack", [Yard, WallFace], 1;
    // Rainforest.
    /// A hammock slung between two posts.
    Hammock = "hammock", "Hammock", [Yard], 1;
    /// A woven screen hung over a window.
    WovenScreen = "woven-screen", "Woven screen", [Window], 4;
    /// Carved posts either side of the door.
    CarvedPosts = "carved-posts", "Carved door posts", [Door], 1;
    /// Baskets of trailing plants hung from the eaves.
    HangingBaskets = "hanging-baskets", "Hanging baskets", [Eaves], 3;
    /// Crossed horns at the ends of the ridge.
    Horns = "horns", "Ridge horns", [Ridge], 2;
    // Old-growth forest.
    /// Moss, ferns and mushrooms on the roof.
    MossyRoof = "mossy-roof", "Moss on the roof", [Ridge, RoofTop], 8;
    /// Lanterns hung under the eaves, lit.
    Lanterns = "lanterns", "Lanterns under the eaves", [Eaves], 4;
    /// Old logs with fungi growing on them.
    MushroomLogs = "mushroom-logs", "Mushroom logs", [Corner, Yard, WallFace], 1;
    // Anywhere rich enough.
    /// A cloth banner hung from a pole.
    Banner = "banner", "Banner", [Gable, Eaves], 1;
}

impl Component {
    /// The component a word names.
    pub fn by_word(w: &str) -> Option<Component> {
        Component::ALL.iter().copied().find(|c| c.word().eq_ignore_ascii_case(w))
    }

    pub fn takes(self, kind: AnchorKind) -> bool {
        self.anchors().contains(&kind)
    }

    /// The regions whose kit files (as compiled in) name it.
    pub fn regions(self) -> Vec<Region> {
        Region::ALL.into_iter().filter(|&r| super::rules::rules(r).components.iter().any(|(_, list)| list.iter().any(|c| c.what == self.word()))).collect()
    }
}

/// How the kit dresses a building beyond its rules.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Dress {
    /// The building stands in a great town: a desert house there may take a wind catcher.
    pub great_town: bool,
}

/// What a component is asked to make.
#[derive(Clone, Copy, Debug)]
pub struct Request<'a> {
    pub anchor: Anchor,
    pub region: Region,
    pub step: Step,
    pub style: Style,
    pub pal: &'a Palette,
    /// 0 poorest to 1 richest: the colour of cloth, how full a window box is.
    pub wealth: f32,
    /// 0 new to 1 old.
    pub age: f32,
    /// Let go: an awning sagging off a fallen pole, a shutter hanging, a window box empty.
    pub decayed: bool,
    /// The building stands in a great town (see [`Dress`]).
    pub great_town: bool,
    pub seed: u32,
}

/// What a component makes, in the building's model space.
#[derive(Clone, Debug, Default)]
pub struct Made {
    pub voxels: Vec<(IVec3, Voxel)>,
    /// In cells of 1/[`CELLS`] voxel.
    pub fine: Vec<PlantBox>,
    /// The building's own voxels painted another colour: a door's frame.
    pub paint: Vec<(IVec3, Voxel)>,
    /// The building's own fine parts (by index into [`Building::fine`]) in another colour,
    /// or with `None` taken away: shutters painted, or gone.
    pub refit: Vec<(usize, Option<Voxel>)>,
}

impl Made {
    pub fn is_empty(&self) -> bool {
        self.voxels.is_empty() && self.fine.is_empty() && self.paint.is_empty() && self.refit.is_empty()
    }
}

/// A component set on a building by [`dress`]: what, where, and the saved design it was
/// built from, if not the generator.
#[derive(Clone, PartialEq, Debug)]
pub struct Attached {
    pub component: Component,
    pub anchor: Anchor,
    pub design: Option<String>,
}

/// The kinds of anchor in the order a building is dressed: the door first, as what a
/// house is seen by, and the yard last, as what has room for anything.
const ORDER: [AnchorKind; 9] = [
    AnchorKind::Door,
    AnchorKind::Window,
    AnchorKind::Gable,
    AnchorKind::Eaves,
    AnchorKind::Ridge,
    AnchorKind::RoofTop,
    AnchorKind::WallFace,
    AnchorKind::Corner,
    AnchorKind::Yard,
];

/// Whether one choice goes for every anchor of the kind on a building, so its windows
/// match and its eaves hang the same thing; the rest choose anchor by anchor.
fn one_for_all(kind: AnchorKind) -> bool {
    !matches!(kind, AnchorKind::WallFace | AnchorKind::Corner | AnchorKind::Yard)
}

/// The kit's components for building `b`, made for `spec` by `rules`, set on it: its
/// voxels and fine parts grow by theirs, the anchors they take are taken off it, and its
/// footprint covers them. Returns what went where.
pub fn dress(b: &mut Building, spec: &Spec, pal: &Palette, rules: &Rules, how: &Dress) -> Vec<Attached> {
    let bare = b.clone();
    let mut site = Site::new(b);
    let mut done: Vec<(Attached, Made)> = Vec::new();
    let mut count: BTreeMap<Component, usize> = BTreeMap::new();
    let seed = spec.seed;
    for (k, &kind) in ORDER.iter().enumerate() {
        let options: Vec<Choice<Option<Component>>> = rules
            .components(kind)
            .iter()
            .filter_map(|c| {
                let what = match Component::by_word(&c.what) {
                    Some(Component::WindCatcher) if !how.great_town => return None,
                    Some(w) if w.takes(kind) => Some(w),
                    Some(_) => return None,
                    None => None,
                };
                Some(Choice { what, weight: c.weight, bias: c.bias })
            })
            .collect();
        let anchors: Vec<Anchor> = bare.anchors.iter().filter(|a| a.kind == kind).copied().collect();
        if options.is_empty() || anchors.is_empty() {
            continue;
        }
        // Which component goes at which anchor.
        let mut plan: Vec<(Component, Anchor, usize)> = Vec::new();
        if one_for_all(kind) {
            // A door takes two choices, so an awning can have jars beside it.
            let rolls = if kind == AnchorKind::Door { 2 } else { 1 };
            let mut chosen: Vec<Component> = Vec::new();
            for r in 0..rolls {
                if let Some(Some(c)) = choose(&options, spec.wealth, unit(hash2(seed ^ 0x0c0e, k as i32, r))).copied()
                    && !chosen.contains(&c)
                {
                    chosen.push(c);
                }
            }
            for c in chosen {
                for (i, a) in anchors_for(c, &anchors, b).into_iter().enumerate() {
                    plan.push((c, a, i));
                }
            }
        } else {
            for (i, a) in anchors.iter().enumerate() {
                if let Some(Some(c)) = choose(&options, spec.wealth, unit(hash2(seed ^ 0x05e7, k as i32, i as i32))).copied() {
                    plan.push((c, *a, i));
                }
            }
        }
        for (c, a, i) in plan {
            if count.get(&c).copied().unwrap_or(0) >= c.most() {
                continue;
            }
            let s = hash2(seed ^ 0x2c0_3e47, c as i32 * 16 + k as i32, i as i32);
            let req = Request {
                anchor: a,
                region: spec.region,
                step: spec.step,
                style: b.style,
                pal,
                wealth: spec.wealth,
                age: spec.age,
                decayed: unit(s ^ 0xdeca) < (spec.age - 0.35) * 1.4,
                great_town: how.great_town,
                seed: s,
            };
            if let Some((made, design)) = attach(c, &req, b, &site) {
                site.take(&made);
                *count.entry(c).or_default() += 1;
                done.push((Attached { component: c, anchor: a, design }, made));
            }
        }
    }
    apply(b, &done);
    // The kit keeps every home walked into; should something set before the door still
    // stand in the way, the home keeps only what has no voxels to block it.
    if !done.is_empty() && !walk(b).ok() && walk(&bare).ok() {
        *b = bare;
        done.retain(|(_, m)| m.voxels.is_empty());
        apply(b, &done);
    }
    done.into_iter().map(|(a, _)| a).collect()
}

/// The anchors of one kind a component chosen for them all goes at, in order: horns at
/// the two ends of the ridge, a bell in its middle, anything else at each.
fn anchors_for(c: Component, anchors: &[Anchor], b: &Building) -> Vec<Anchor> {
    let along = |a: &Anchor| if ridge_along_x(b) { a.at.x } else { a.at.z };
    let mut sorted = anchors.to_vec();
    sorted.sort_by_key(|a| (along(a), a.at.y));
    match c {
        Component::Horns if sorted.len() > 1 => vec![sorted[0], sorted[sorted.len() - 1]],
        Component::Bell => sorted.get(sorted.len() / 2).copied().into_iter().collect(),
        _ => anchors.to_vec(),
    }
}

/// Whether the main ridge runs along x: two ridge anchors in a line along x say so, and
/// otherwise the longer side of the walls.
fn ridge_along_x(b: &Building) -> bool {
    let ridge: Vec<&Anchor> = b.anchors.iter().filter(|a| a.kind == AnchorKind::Ridge).collect();
    for (i, a) in ridge.iter().enumerate() {
        for c in &ridge[i + 1..] {
            if a.at.z == c.at.z && a.at.x != c.at.x {
                return true;
            }
            if a.at.x == c.at.x && a.at.z != c.at.z {
                return false;
            }
        }
    }
    b.walls.hi.x - b.walls.lo.x >= b.walls.hi.y - b.walls.lo.y
}

/// The quarter turns of an anchor's frame: +X out of the building, along its normal (for
/// a corner, +X and +Z both out). On a roof +X goes across the ridge, and in the yard
/// away from the house.
pub fn turns_for(a: &Anchor, b: &Building) -> u32 {
    let n = a.normal;
    let want = if n.x != 0 || n.z != 0 {
        IVec3::new(n.x.signum(), 0, n.z.signum())
    } else if matches!(a.kind, AnchorKind::Ridge | AnchorKind::RoofTop) {
        if ridge_along_x(b) { IVec3::Z } else { IVec3::X }
    } else {
        let mid = (b.walls.lo + b.walls.hi).as_vec2() * 0.5;
        let d = IVec2::new(a.at.x, a.at.z).as_vec2() - mid;
        if d.x.abs() >= d.y.abs() { IVec3::X * d.x.signum() as i32 } else { IVec3::Z * d.y.signum() as i32 }
    };
    (0..4)
        .find(|&t| {
            let (x, z) = (rotate(IVec3::X, t), rotate(IVec3::Z, t));
            if want.x != 0 && want.z != 0 { x + z == want } else { x == want }
        })
        .unwrap_or(0)
}

/// Component `c` made for `req` on `b` as it stands in `site`: from a saved design when
/// the seed picks one and it fits, and otherwise by its generator. None if it does not
/// fit there.
fn attach(c: Component, req: &Request, b: &Building, site: &Site) -> Option<(Made, Option<String>)> {
    if let Some(d) = designs::pick_component(c, req.seed)
        && let Some(m) = saved(d, req, b, site)
    {
        return Some((m, Some(d.name.clone())));
    }
    generate(c, req, b, site).map(|m| (m, None))
}

/// The generator's own component `c` for `req` on `b`, if it fits.
fn generate(c: Component, req: &Request, b: &Building, site: &Site) -> Option<Made> {
    let mut cx = Cx::new(req, b, site);
    build_one(c, &mut cx)?;
    let made = cx.finish();
    (!made.is_empty() && site.fits(&made, req.anchor.kind == AnchorKind::Door)).then_some(made)
}

/// Runs a component's generator in `cx`. None where it cannot go at all.
fn build_one(c: Component, cx: &mut Cx) -> Option<()> {
    use Component::*;
    match c {
        Awning => dry::awning(cx),
        SailShade => dry::sail_shade(cx),
        Terrace => dry::terrace(cx),
        Lattice => dry::lattice(cx),
        WaterJar => common::against_wall(cx, Fitting::WaterJar, 2),
        PalmShade => dry::palm_shade(cx),
        DoorFrame => dry::door_frame(cx),
        WindCatcher => dry::wind_catcher(cx),
        Oven => dry::oven(cx),
        DryingMats => dry::drying_mats(cx),
        Porch => green::porch(cx, false),
        WindowBox => green::window_box(cx),
        RainButt => common::against_wall(cx, Fitting::WaterButt, 1),
        Dovecote => green::dovecote(cx),
        ClimbingRose => green::climber(cx, true),
        Ivy => green::climber(cx, false),
        Shutters => green::shutters(cx),
        Herbs => common::hung(cx, Fitting::HerbBunches),
        Bench => common::bench(cx),
        Woodpile => common::woodpile(cx),
        HenCoop => common::in_yard(cx, &[Fitting::ChickenCoop]),
        Hives => common::in_yard(cx, &[Fitting::Skep, Fitting::Skep, Fitting::Skep]),
        ThatchWeights => green::thatch_weights(cx),
        PeatStack => green::peat_stack(cx),
        WindPorch => green::porch(cx, true),
        LeeWall => green::lee_wall(cx),
        TurfBench => green::turf_bench(cx),
        WaterTrough => common::in_yard(cx, &[Fitting::WaterTrough]),
        SheepFold => green::sheep_fold(cx),
        TurfFlowers => common::roof_growth(cx, true),
        CarvedBoards => north::carved_boards(cx),
        RaisedStore => north::raised_store(cx),
        HideCurtain => north::hide_curtain(cx),
        FishRack => common::in_yard(cx, &[Fitting::FishRack]),
        DryingRack => common::in_yard(cx, &[Fitting::HideFrame]),
        Balcony => north::balcony(cx),
        HayRack => common::in_yard(cx, &[Fitting::HayRick]),
        Bell => north::bell(cx),
        Boardwalk => wet::boardwalk(cx),
        RopeRail => wet::rope_rail(cx),
        ReedScreen => wet::reed_screen(cx),
        NetRack => common::in_yard(cx, &[Fitting::NetRack]),
        DryingNets => wet::drying_nets(cx),
        EelTraps => common::in_yard(cx, &[Fitting::EelTrap, Fitting::EelTrap]),
        Punt => wet::punt(cx),
        DriftwoodFence => wet::driftwood_fence(cx),
        UpturnedBoat => wet::upturned_boat(cx),
        ReedStack => wet::reed_stack(cx),
        Hammock => wet::hammock(cx),
        WovenScreen => wet::woven_screen(cx),
        CarvedPosts => wet::carved_posts(cx),
        HangingBaskets => wet::hanging(cx, false),
        Horns => north::horns(cx),
        MossyRoof => common::roof_growth(cx, false),
        Lanterns => wet::hanging(cx, true),
        MushroomLogs => wet::mushroom_logs(cx),
        Banner => common::banner(cx),
    }
}

/// A saved design of a component built for `req`: drawn in the anchor's frame, it is
/// turned and moved to the anchor. None if it does not fit there.
fn saved(d: &Design, req: &Request, b: &Building, site: &Site) -> Option<Made> {
    let t = turns_for(&req.anchor, b);
    let mut s = d.shape(req.pal, req.seed);
    let fine = s.take_fine();
    let voxels = s.into_voxels();
    let made = Made {
        voxels: voxels.into_iter().filter(|(_, v)| *v != AIR).map(|(q, v)| (rotate(q, t) + req.anchor.at, v)).collect(),
        fine: parts::place_fine(&fine, req.anchor.at, t),
        ..Made::default()
    };
    (!made.is_empty() && site.fits(&made, req.anchor.kind == AnchorKind::Door)).then_some(made)
}

/// Sets what was made on the building.
fn apply(b: &mut Building, done: &[(Attached, Made)]) {
    let paint: HashMap<IVec3, Voxel> = done.iter().flat_map(|(_, m)| m.paint.iter().copied()).collect();
    for (q, v) in &mut b.voxels {
        if let Some(&p) = paint.get(q) {
            *v = p;
        }
    }
    let mut keep = vec![true; b.fine.len()];
    for &(i, v) in done.iter().flat_map(|(_, m)| &m.refit) {
        match v {
            Some(v) => b.fine[i].voxel = v,
            None => keep[i] = false,
        }
    }
    let mut k = 0;
    b.fine.retain(|_| {
        k += 1;
        keep[k - 1]
    });
    for (_, m) in done {
        b.voxels.extend(m.voxels.iter().copied());
        b.fine.extend(m.fine.iter().copied());
    }
    b.voxels.sort_unstable_by_key(|(q, _)| (q.y, q.x, q.z));
    // The anchors taken, and any a component now stands in, are no longer free.
    let solid: HashSet<IVec3> = b.voxels.iter().filter(|(_, v)| *v != AIR).map(|(q, _)| *q).collect();
    b.anchors.retain(|a| !solid.contains(&a.at) && !done.iter().any(|(d, _)| d.anchor == *a));
    b.footprint = footprint(b.walls, &b.voxels, &b.fine);
}

/// A building as components see it while it is dressed, in model space: what it says of
/// each voxel, what earlier components took, the boxes kept clear, and every fine part.
pub(crate) struct Site {
    /// The building's voxels, air where it clears the ground.
    said: HashMap<IVec3, Voxel>,
    /// Voxels components have taken.
    taken: HashSet<IVec3>,
    clear: Vec<(IVec3, IVec3)>,
    /// The building's fine parts and the components', by every voxel they reach into,
    /// each with its index in the building's (`usize::MAX` for a component's).
    fine: HashMap<IVec3, Vec<([i32; 3], [i32; 3], usize)>>,
}

impl Site {
    pub(crate) fn new(b: &Building) -> Self {
        let mut s = Site { said: b.voxels.iter().copied().collect(), taken: HashSet::new(), clear: b.clear.clone(), fine: HashMap::new() };
        for (i, p) in b.fine.iter().enumerate() {
            s.add_fine(p, i);
        }
        s
    }

    /// An empty site: open ground and nothing on it, for things set on their own.
    pub(crate) fn open() -> Self {
        Site { said: HashMap::new(), taken: HashSet::new(), clear: Vec::new(), fine: HashMap::new() }
    }

    fn add_fine(&mut self, p: &PlantBox, i: usize) {
        for q in cells_of(p.lo, p.hi) {
            self.fine.entry(q).or_default().push((p.lo, p.hi, i));
        }
    }

    /// Whether voxel `q` is solid: the building's, a component's, or the ground below
    /// `y = 0` wherever the building says nothing.
    pub(crate) fn solid(&self, q: IVec3) -> bool {
        self.taken.contains(&q) || self.said.get(&q).map_or(q.y < 0, |&v| v != AIR)
    }

    /// Whether the building says anything of `q`: a voxel of its own, or air it clears.
    fn said(&self, q: IVec3) -> bool {
        self.said.contains_key(&q)
    }

    /// The index of the box kept clear that holds `q`, if any.
    fn in_clear(&self, q: IVec3) -> Option<usize> {
        self.clear.iter().position(|(lo, hi)| q.cmpge(*lo).all() && q.cmple(*hi).all())
    }

    /// Whether a component's voxel can go at `q`: nothing is there or cleared there, and
    /// it is not kept clear.
    pub(crate) fn free(&self, q: IVec3) -> bool {
        !self.taken.contains(&q) && !self.said(q) && self.in_clear(q).is_none()
    }

    /// Whether a box of cells overlaps a fine part already there, but for the building's
    /// parts listed in `gone`.
    pub(crate) fn clash(&self, lo: [i32; 3], hi: [i32; 3], gone: &[usize]) -> bool {
        cells_of(lo, hi).any(|q| self.fine.get(&q).is_some_and(|v| v.iter().any(|(a, b, k)| (0..3).all(|i| lo[i] < b[i] && a[i] < hi[i]) && !gone.contains(k))))
    }

    /// Whether a box of cells touches, face, edge or corner, a fine part already there.
    fn touches(&self, lo: [i32; 3], hi: [i32; 3]) -> bool {
        let (wlo, whi) = (lo.map(|v| v - 1), hi.map(|v| v + 1));
        cells_of(wlo, whi).any(|q| self.fine.get(&q).is_some_and(|v| v.iter().any(|(a, b, _)| (0..3).all(|i| lo[i] <= b[i] && a[i] <= hi[i]))))
    }

    /// Takes what a component made, so the next keeps out of it.
    fn take(&mut self, m: &Made) {
        self.taken.extend(m.voxels.iter().map(|(q, _)| *q));
        for p in &m.fine {
            self.add_fine(p, usize::MAX);
        }
    }

    /// Whether what a component made fits whole: its voxels where nothing is and nothing
    /// is kept clear, its fine parts out of anything solid, out of the boxes kept clear
    /// (but for the way in, for what dresses a door: cloth and rails never stop anyone)
    /// and out of every other fine part; and all of it held, by the building, the ground
    /// or the rest of it.
    pub(crate) fn fits(&self, m: &Made, door: bool) -> bool {
        let own: HashSet<IVec3> = m.voxels.iter().map(|(q, _)| *q).collect();
        if own.len() != m.voxels.len() {
            return false;
        }
        let gone: Vec<usize> = m.refit.iter().filter(|(_, v)| v.is_none()).map(|(i, _)| *i).collect();
        for &q in &own {
            let c = q * CELLS;
            if !self.free(q) || self.clash(c.to_array(), (c + CELLS).to_array(), &gone) {
                return false;
            }
        }
        for p in &m.fine {
            if !(0..3).all(|i| p.lo[i] < p.hi[i]) || self.clash(p.lo, p.hi, &gone) {
                return false;
            }
            // A fine part may wrap a voxel of its own (a fitting's jar round its core).
            for q in cells_of(p.lo, p.hi) {
                if self.solid(q) || (!door && self.in_clear(q).is_some()) {
                    return false;
                }
            }
        }
        if m.paint.iter().any(|(q, _)| self.said.get(q).is_none_or(|&v| v == AIR)) {
            return false;
        }
        self.held(m, &own)
    }

    /// Whether everything made is held: each voxel by the ground, the building or an
    /// earlier component through voxels of its own, and each fine part by touching
    /// something solid or a fine part that is.
    fn held(&self, m: &Made, own: &HashSet<IVec3>) -> bool {
        const D6: [IVec3; 6] = [IVec3::X, IVec3::NEG_X, IVec3::Y, IVec3::NEG_Y, IVec3::Z, IVec3::NEG_Z];
        let mut held: HashSet<IVec3> = own.iter().copied().filter(|&q| D6.iter().any(|&d| !own.contains(&(q + d)) && self.solid(q + d))).collect();
        let mut queue: Vec<IVec3> = held.iter().copied().collect();
        while let Some(q) = queue.pop() {
            for d in D6 {
                if own.contains(&(q + d)) && held.insert(q + d) {
                    queue.push(q + d);
                }
            }
        }
        if held.len() != own.len() {
            return false;
        }
        let solid = |q: IVec3| own.contains(&q) || self.solid(q);
        let on_voxel = |p: &PlantBox| {
            let lo = IVec3::from(p.lo.map(|v| (v - 1).div_euclid(CELLS)));
            let hi = IVec3::from(p.hi.map(|v| v.div_euclid(CELLS)));
            (lo.y..=hi.y).any(|y| (lo.z..=hi.z).any(|z| (lo.x..=hi.x).any(|x| solid(IVec3::new(x, y, z)))))
        };
        let mut fixed: Vec<bool> = m.fine.iter().map(|p| on_voxel(p) || self.touches(p.lo, p.hi)).collect();
        let touch = |a: &PlantBox, b: &PlantBox| (0..3).all(|i| a.lo[i] <= b.hi[i] && b.lo[i] <= a.hi[i]);
        loop {
            let mut more = false;
            for i in 0..m.fine.len() {
                if !fixed[i] && (0..m.fine.len()).any(|j| fixed[j] && touch(&m.fine[i], &m.fine[j])) {
                    fixed[i] = true;
                    more = true;
                }
            }
            if !more {
                break;
            }
        }
        fixed.into_iter().all(|f| f)
    }
}

/// Every voxel a box of cells reaches into.
fn cells_of(lo: [i32; 3], hi: [i32; 3]) -> impl Iterator<Item = IVec3> {
    let a = IVec3::from(lo).div_euclid(IVec3::splat(CELLS));
    let b = (IVec3::from(hi) - 1).div_euclid(IVec3::splat(CELLS));
    (a.y..=b.y).flat_map(move |y| (a.z..=b.z).flat_map(move |z| (a.x..=b.x).map(move |x| IVec3::new(x, y, z))))
}

/// A component being made: what it is asked for, the building and the site it goes on,
/// and its parts so far, in the anchor's own frame (see the module's documentation).
pub(crate) struct Cx<'a> {
    pub r: &'a Request<'a>,
    pub b: &'a Building,
    site: &'a Site,
    t: u32,
    pub rng: Rng,
    vox: Vec<(IVec3, Voxel)>,
    mine: HashSet<IVec3>,
    fine: Vec<PlantBox>,
    paint: Vec<(IVec3, Voxel)>,
    refit: Vec<(usize, Option<Voxel>)>,
}

impl<'a> Cx<'a> {
    fn new(r: &'a Request<'a>, b: &'a Building, site: &'a Site) -> Self {
        Cx { r, b, site, t: turns_for(&r.anchor, b), rng: Rng::new(r.seed), vox: Vec::new(), mine: HashSet::new(), fine: Vec::new(), paint: Vec::new(), refit: Vec::new() }
    }

    /// A voxel of the frame in the building's model space.
    pub fn model(&self, l: IVec3) -> IVec3 {
        rotate(l, self.t) + self.r.anchor.at
    }

    pub fn pal(&self) -> &'a Palette {
        self.r.pal
    }

    pub fn decayed(&self) -> bool {
        self.r.decayed
    }

    pub fn wealth(&self) -> f32 {
        self.r.wealth
    }

    /// The layer of the frame where the ground is: model `y = 0`.
    pub fn ground(&self) -> i32 {
        -self.r.anchor.at.y
    }

    /// Whether voxel `l` is solid: the building, the ground, a component, or this one.
    pub fn solid(&self, l: IVec3) -> bool {
        self.mine.contains(&l) || self.site.solid(self.model(l))
    }

    pub fn open(&self, l: IVec3) -> bool {
        !self.solid(l)
    }

    /// Whether voxel `l` is solid in the building itself (not the ground, nor anything
    /// set about it).
    pub fn wall(&self, l: IVec3) -> bool {
        self.site.said.get(&self.model(l)).is_some_and(|&v| v != AIR)
    }

    /// The building's own voxel at `l`, if it has one.
    pub fn material(&self, l: IVec3) -> Option<Voxel> {
        self.site.said.get(&self.model(l)).copied().filter(|&v| v != AIR)
    }

    /// What the house's roof is covered with near the anchor, as weathered as it is
    /// there, so what is roofed to match it matches it.
    pub fn roofing(&self) -> Voxel {
        use crate::materials as mat;
        for y in 0..14 {
            for x in -2..4 {
                for z in -2..=2 {
                    if let Some(v) = self.material(IVec3::new(x, y, z))
                        && mat::ROOFING.contains(&v)
                    {
                        return v;
                    }
                }
            }
        }
        match self.r.style.roof {
            super::Cover::Thatch => mat::THATCH,
            super::Cover::Reed => mat::REED_THATCH,
            super::Cover::Palm => mat::PALM_THATCH,
            super::Cover::Turf => mat::ROOF_TURF,
            super::Cover::Slate => mat::SLATE,
            super::Cover::Tile => mat::ROOF_TILE,
            super::Cover::Shingle => mat::SHINGLE,
            super::Cover::Mud => mat::ROOF_MUD,
        }
    }

    /// Whether a voxel of this component can go at `l`.
    pub fn free(&self, l: IVec3) -> bool {
        !self.mine.contains(&l) && self.site.free(self.model(l))
    }

    /// Lays a voxel of the component.
    pub fn put(&mut self, l: IVec3, v: Voxel) {
        if self.mine.insert(l) {
            self.vox.push((l, v));
        }
    }

    /// Lays a voxel where one can go, and says whether it did.
    pub fn put_free(&mut self, l: IVec3, v: Voxel) -> bool {
        let ok = self.free(l);
        if ok {
            self.put(l, v);
        }
        ok
    }

    /// A fine part fixed in place, `lo` to `hi` (exclusive) in cells of the frame.
    pub fn part(&mut self, lo: IVec3, hi: IVec3, v: Voxel) {
        self.plant(PlantBox { lo: lo.min(hi).to_array(), hi: lo.max(hi).to_array(), voxel: v, sway_lo: false, sway_hi: false, thin: 0, season: 0, flap: 0 });
    }

    /// A fine part of a plant: its top sways, and it shows in the season `season` names
    /// (0 always).
    pub fn growth(&mut self, lo: IVec3, hi: IVec3, v: Voxel, season: u8) {
        self.plant(PlantBox { lo: lo.min(hi).to_array(), hi: lo.max(hi).to_array(), voxel: v, sway_lo: false, sway_hi: true, thin: 0, season, flap: 0 });
    }

    pub fn plant(&mut self, b: PlantBox) {
        if (0..3).all(|i| b.lo[i] < b.hi[i]) {
            self.fine.push(b);
        }
    }

    /// Fine parts already made in cells of the frame.
    pub fn plants(&mut self, parts: impl IntoIterator<Item = PlantBox>) {
        for p in parts {
            self.plant(p);
        }
    }

    /// Whether a box of cells of the frame is open: no solid voxel in it, no fine part of
    /// anyone else's, none of this one's.
    pub fn room(&self, lo: IVec3, hi: IVec3) -> bool {
        let (a, b) = parts::rotate_box(lo, hi, self.t);
        let at = self.r.anchor.at * CELLS;
        let (a, b) = ((a + at).to_array(), (b + at).to_array());
        let mine = self.fine.iter().any(|p| (0..3).all(|i| p.lo[i] < hi[i] && lo[i] < p.hi[i]));
        let gone: Vec<usize> = self.refit.iter().filter(|(_, v)| v.is_none()).map(|(i, _)| *i).collect();
        !mine && !self.site.clash(a, b, &gone) && cells_of(lo.to_array(), hi.to_array()).all(|q| self.open(q))
    }

    /// Paints a voxel of the building.
    pub fn paint(&mut self, l: IVec3, v: Voxel) {
        if self.wall(l) {
            self.paint.push((self.model(l), v));
        }
    }

    /// The building's fine parts lying wholly within a box of cells of the frame, by index.
    pub fn parts_within(&self, lo: IVec3, hi: IVec3) -> Vec<usize> {
        let (a, b) = parts::rotate_box(lo, hi, self.t);
        let at = self.r.anchor.at * CELLS;
        let (a, b) = (a + at, b + at);
        self.b.fine.iter().enumerate().filter(|(_, p)| IVec3::from(p.lo).cmpge(a).all() && IVec3::from(p.hi).cmple(b).all()).map(|(i, _)| i).collect()
    }

    /// Recolours one of the building's fine parts, or with `None` takes it away.
    pub fn refit(&mut self, i: usize, v: Option<Voxel>) {
        self.refit.push((i, v));
    }

    /// A ribbon of cloth hung from `knot`, in cells of the frame (see [`cloth::ribbon`]).
    pub fn ribbon(&mut self, knot: IVec3, length: i32, colour: Voxel, across_z: bool) {
        let parts = cloth::ribbon(knot, length, colour, across_z as u32);
        self.plants(parts);
    }

    /// A sheet of cloth tied at four corners, in cells of the frame (see [`cloth::sheet`]).
    pub fn sheet(&mut self, corners: [IVec3; 4], colour: Voxel) {
        let parts = cloth::sheet(corners, colour);
        self.plants(parts);
    }

    /// A fitting built for the place and set with its voxel column 0 at `at`, turned by
    /// `turns` within the frame (see [`fittings::Fitted::place`]).
    pub fn fitting(&mut self, kind: Fitting, at: IVec3, turns: u32) {
        let kind = kind.local(self.r.region);
        let f = fittings::build_with(kind, self.r.pal, self.r.seed, self.r.decayed, fittings::herbs_of(self.r.region)).place(at, turns);
        for (q, v) in f.voxels {
            if v != AIR {
                self.put(q, v);
            }
        }
        self.plants(f.fine);
    }

    /// Where something stands in column (x, z) of the frame, looking down from layer
    /// `from` no further than `depth` layers: the open layer over the first solid one.
    pub fn stands(&self, x: i32, z: i32, from: i32, depth: i32) -> Option<i32> {
        (from - depth..=from).rev().find(|&y| self.open(IVec3::new(x, y, z)) && self.solid(IVec3::new(x, y - 1, z)))
    }

    /// The layer every column of a patch of the frame stands at alike, if they do.
    pub fn level(&self, xs: std::ops::RangeInclusive<i32>, zs: std::ops::RangeInclusive<i32>, from: i32, depth: i32) -> Option<i32> {
        let mut at = None;
        for x in xs {
            for z in zs.clone() {
                let y = self.stands(x, z, from, depth)?;
                if at.is_some_and(|a| a != y) {
                    return None;
                }
                at = Some(y);
            }
        }
        at
    }

    /// The doorway behind a door's anchor: its first and last z in the frame, and how
    /// many layers it is open above the feet.
    pub fn doorway(&self) -> Option<(i32, i32, i32)> {
        let open = |z: i32| self.open(IVec3::new(-1, 0, z)) && self.open(IVec3::new(-1, 1, z));
        if !open(0) {
            return None;
        }
        let z0 = (-6..=0).rev().take_while(|&z| open(z)).last()?;
        let z1 = (0..=6).take_while(|&z| open(z)).last()?;
        let h = (0..14).take_while(|&y| self.open(IVec3::new(-1, y, 0))).count() as i32;
        (h >= 4 && z1 - z0 <= 8).then_some((z0, z1, h))
    }

    /// The window behind a window's anchor: its first and last z in the frame, and its
    /// height. Its sill is the layer below the anchor.
    pub fn window(&self) -> Option<(i32, i32, i32)> {
        let open = |z: i32| self.open(IVec3::new(-1, 0, z));
        if !open(0) {
            return None;
        }
        let z0 = (-4..=0).rev().take_while(|&z| open(z)).last()?;
        let z1 = (0..=4).take_while(|&z| open(z)).last()?;
        let h = (0..4).take_while(|&y| self.open(IVec3::new(-1, y, 0))).count() as i32;
        Some((z0, z1, h))
    }

    /// How far out from the wall the roof reaches over the anchor's layer, along row `z`
    /// of the frame: the last column with roof straight above within a few layers, and
    /// the layer of its underside.
    pub fn eaves_edge(&self, z: i32) -> Option<(i32, i32)> {
        let under = |x: i32| (-3..=2).find(|&y| self.wall(IVec3::new(x, y, z)) && self.open(IVec3::new(x, y - 1, z)));
        let mut edge = None;
        for x in 0..6 {
            match under(x) {
                Some(y) => edge = Some((x, y)),
                None => break,
            }
        }
        edge
    }

    /// A colour of cloth for the household: bright dyes for the rich, undyed for the
    /// poor, faded where it is let go.
    pub fn cloth(&mut self) -> Voxel {
        use crate::materials as mat;
        if self.r.decayed {
            return if self.rng.unit() < 0.5 { mat::CLOTH_PALE } else { mat::SACKCLOTH };
        }
        let rich = self.r.wealth + self.rng.range(-0.2, 0.2);
        if rich < 0.3 {
            mat::SACKCLOTH
        } else if rich < 0.5 {
            mat::CLOTH_PALE
        } else {
            [mat::CLOTH_RED, mat::CLOTH_BLUE, mat::CLOTH_YELLOW, mat::CLOTH_GREEN, mat::CLOTH_RED][(self.rng.next_u32() % 5) as usize]
        }
    }

    /// What was made, in the building's model space.
    fn finish(self) -> Made {
        let (at, t) = (self.r.anchor.at, self.t);
        Made {
            voxels: self.vox.into_iter().map(|(q, v)| (rotate(q, t) + at, v)).collect(),
            fine: parts::place_fine(&self.fine, at, t),
            paint: self.paint,
            refit: self.refit,
        }
    }
}

/// A component shown on its own in the editor: on a stretch of a bare kit home of a
/// region that has it, turned so the anchor is at the origin facing +X, as a design of
/// the component is drawn.
#[derive(Clone, Debug)]
pub struct Stand {
    pub component: Component,
    /// The home about the anchor, turned and moved.
    pub building: Building,
    pub anchor: Anchor,
    /// The layer the ground is under, below the anchor's (0 for an anchor on the ground,
    /// lower for a raised floor's door).
    pub ground: i32,
    pub region: Region,
    pub spec: Spec,
    /// The component, in the same space as the home: the design's own frame.
    pub made: Made,
}

impl Stand {
    /// The home with the component set on it, the home cut down to what lies within
    /// `reach` voxels of the anchor along the wall and into the house, to show the
    /// component on; the component whole.
    pub fn parts(&self, reach: i32) -> (Vec<(IVec3, Voxel)>, Vec<PlantBox>) {
        let mut b = self.building.clone();
        let keep = |q: IVec3| q.x >= -reach && q.z.abs() <= reach;
        // Painted and refitted as the whole home is, before it is cut down.
        apply(&mut b, &[(Attached { component: self.component, anchor: self.anchor, design: None }, Made { voxels: Vec::new(), fine: Vec::new(), ..self.made.clone() })]);
        let mut voxels: Vec<(IVec3, Voxel)> = b.voxels.into_iter().filter(|&(q, v)| v != AIR && keep(q)).collect();
        let mut fine: Vec<PlantBox> = b.fine.into_iter().filter(|p| keep(IVec3::from(p.lo).div_euclid(IVec3::splat(CELLS)))).collect();
        voxels.extend(self.made.voxels.iter().copied());
        fine.extend(self.made.fine.iter().copied());
        (voxels, fine)
    }
}

/// Where component `c` is shown for `variant`: on the first of the bare kit homes the
/// variant leads to that takes it, built by its generator, or from `design` if given.
pub fn stand(c: Component, variant: u32, design: Option<&Design>, pal_of: impl Fn(Region, u32) -> Palette) -> Option<Stand> {
    stand_aged(c, variant, design, false, pal_of)
}

/// [`stand`], new or let go (`decayed`): an awning sagging off a fallen pole, shutters
/// hanging, a window box empty.
pub fn stand_aged(c: Component, variant: u32, design: Option<&Design>, decayed: bool, pal_of: impl Fn(Region, u32) -> Palette) -> Option<Stand> {
    let mut regions = c.regions();
    if regions.is_empty() {
        regions.push(Region::Temperate);
    }
    for k in 0..48u32 {
        let v = variant.wrapping_add(k * 7919);
        let region = regions[(v as usize + k as usize) % regions.len()];
        let step = Step::ALL[(v % 3) as usize];
        let h = |n: i32| unit(hash3(v, n, c as i32, 0x57a));
        let spec = Spec { region, step, age: h(1) * 0.6, wealth: 0.55 + h(2) * 0.45, sleepers: 2, seed: v };
        let pal = pal_of(region, v);
        let b = super::generate_bare(&spec, &pal, super::rules::rules(region));
        let site = Site::new(&b);
        for a in b.anchors.iter().filter(|a| c.takes(a.kind)) {
            let age = if decayed { 0.8 + spec.age * 0.2 } else { spec.age };
            let req = Request { anchor: *a, region, step, style: b.style, pal: &pal, wealth: spec.wealth, age, decayed, great_town: true, seed: hash2(v, c as i32, 0x57a) };
            let made = match design {
                Some(d) => saved(d, &req, &b, &site),
                None => generate(c, &req, &b, &site),
            };
            if let Some(made) = made {
                let u = (4 - turns_for(a, &b)) & 3;
                let shift = -rotate(a.at, u);
                let building = moved(&b.turned(u), shift);
                let made = Made {
                    voxels: made.voxels.iter().map(|&(q, v)| (rotate(q, u) + shift, v)).collect(),
                    fine: parts::place_fine(&made.fine, shift, u),
                    paint: made.paint.iter().map(|&(q, v)| (rotate(q, u) + shift, v)).collect(),
                    refit: made.refit,
                };
                let anchor = Anchor { at: IVec3::ZERO, normal: rotate(a.normal, u), kind: a.kind };
                return Some(Stand { component: c, building, anchor, ground: -a.at.y, region, spec, made });
            }
        }
    }
    None
}

/// A building moved by whole voxels.
fn moved(b: &Building, d: IVec3) -> Building {
    let d2 = IVec2::new(d.x, d.z);
    let mut m = b.clone();
    for (q, _) in &mut m.voxels {
        *q += d;
    }
    m.fine = parts::place_fine(&b.fine, d, 0);
    m.footprint = super::Area { lo: b.footprint.lo + d2, hi: b.footprint.hi + d2 };
    m.walls = super::Area { lo: b.walls.lo + d2, hi: b.walls.hi + d2 };
    m.door.floor += d;
    for a in &mut m.anchors {
        a.at += d;
    }
    for q in &mut m.inside.floor {
        *q += d;
    }
    for s in m.inside.beds.iter_mut().chain(&mut m.inside.furniture) {
        s.at += d;
    }
    if let Some(h) = &mut m.inside.hearth {
        h.at += d;
    }
    for w in &mut m.inside.windows {
        w.lo += d;
        w.hi += d;
    }
    for (lo, hi) in &mut m.clear {
        *lo += d;
        *hi += d;
    }
    m.smoke += d;
    m
}

/// The component shown for `variant` as a design of its own to edit, named `name`: drawn
/// in its anchor's frame, its stone and timber as roles so it takes the materials of
/// wherever it is set.
pub fn copy(c: Component, variant: u32, name: &str, pal_of: impl Fn(Region, u32) -> Palette) -> Option<Design> {
    let s = stand(c, variant, None, &pal_of)?;
    let pal = pal_of(s.region, s.spec.seed);
    // The same again of a palette with a material for each role, which says which role
    // each voxel came from where the place uses one material for two.
    let tags = designs::tag_palette(&pal);
    let t = stand(c, variant, None, |_, _| tags)?;
    let same = s.made.voxels.len() == t.made.voxels.len() && s.made.voxels.iter().zip(&t.made.voxels).all(|(a, b)| a.0 == b.0);
    let cell = |v: Voxel, tag: Option<Voxel>| -> Cell {
        if let Some(t) = tag
            && let Some(r) = Role::ALL.into_iter().find(|&r| r.plain(&tags) == Some(t) && r.plain(&pal) == Some(v))
        {
            return Cell::Role(r);
        }
        match Role::ALL.into_iter().find(|&r| r.plain(&pal) == Some(v)) {
            Some(r) if tag.is_none() => Cell::Role(r),
            _ => Cell::Material(v),
        }
    };
    let mut d = Design::new(designs::Kind::Component(c), name);
    d.note = format!("copied from the generator's {}, variant {variant}, as it goes on a {} {}", c.word(), s.region.word(), s.spec.step.word());
    for (k, &(q, v)) in s.made.voxels.iter().enumerate() {
        d.cells.insert(q, cell(v, same.then(|| t.made.voxels[k].1)));
    }
    let same_fine = s.made.fine.len() == t.made.fine.len();
    for (k, f) in s.made.fine.iter().enumerate() {
        let tag = same_fine.then(|| t.made.fine[k]).filter(|g| g.lo == f.lo && g.hi == f.hi).map(|g| g.voxel);
        d.fine.push(designs::Fine::of(f, cell(f.voxel, tag)));
    }
    Some(d)
}
