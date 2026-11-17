//! What a settlement is furnished and dressed with: the beds, tables, chests and hearths
//! inside its houses, the fittings of each trade in the workshops and yards (an anvil, a
//! net rack, a herb drying rack), and the wells, carts, hives and ricks of its yards and
//! lanes. Each has an intact form and a decayed one, as the props about a landmark do, and
//! is built of the place's own stone and timber from its [`Palette`].
//!
//! [`Fitting`] is a sibling of [`Prop`](super::props::Prop) rather than more of it. A prop
//! is laid loose about a landmark by `props::dress`, wherever there is room; a fitting is
//! set in a room or a yard by a settlement's plan, so as well as its parts it says how much
//! room it needs ([`Fitting::size`]), what it stands against ([`Stand`]), which way it
//! faces ([`FRONT`]), and where later work attaches to it ([`Hook`]): the ends of a washing
//! line, a stall's awning, a hearth's fire, a hive's door.
//!
//! Every fitting is built in model space, unturned: it stands in columns `0..w` by `0..d`
//! and layers `0..h` of its [`Fitting::size`], its back along `z = 0` (a wall it stands
//! against is at `z = -1`) and its front toward +Z. [`Fitted::place`] turns it and moves it
//! to where it stands. As with props, the parts are mostly fine boxes a tenth of a metre on
//! a side ([`PlantBox`]); only what is solid, boxy and at least a voxel each way (a chest,
//! a well head, a hay rick) has voxels, which is what a player bumps into. Fine parts never
//! lie flush with a face of another material facing the same way, so no two surfaces
//! fight; a test holds every fitting to that.

mod furniture;
mod trades;
mod yard;

use glam::{IVec2, IVec3, Vec3};
use voxel::{
    AIR, Voxel,
    mesh::{CELLS, PlantBox},
};

use super::{
    landmarks::Palette,
    parts,
    plants::Flower,
    props::{self, Prop},
    shape::{Rng, rotate},
};
use crate::{materials as mat, settlements::Region};

/// One kind of thing a settlement is furnished with.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Fitting {
    // Furniture.
    /// A straw mattress on the floor of a hut, with a bolster and a blanket.
    StrawPallet,
    /// A cupboard bed in a house: a box of boards with a roof and a curtain, for two.
    BoxBed,
    /// A bed of woven rope on a frame, for warm country, where it is carried out at night.
    RopeBed,
    Table,
    Stool,
    /// Standing shelves against a wall, with pots, jars and bowls on them.
    Shelf,
    Chest,
    Cradle,
    /// The hearth in the middle of a hut: a ring of stones, the fire, and a pot hung over
    /// it from a tripod.
    HutHearth,
    /// An upright loom leant against a wall, its warp held taut by weights.
    Loom,
    /// A loom pegged out flat on the ground, as weavers in the desert use.
    GroundLoom,
    // The trades.
    /// The smith's anvil on its block, with a quenching bucket.
    Anvil,
    /// The smith's forge: a stone hearth under a hood, with its bellows.
    Forge,
    Grindstone,
    /// A woodworker's bench with its tools hung behind it.
    Workbench,
    /// A sawbuck with a log on it, the saw left in the cut.
    Sawhorse,
    /// Long logs laid up on skids, waiting to be sawn.
    LogDeck,
    /// The forager's drying frame, hung with bunches of the plants of the country round.
    HerbRack,
    /// Bunches of plants hung to dry under the eaves, from a batten.
    HerbBunches,
    /// A hide stretched to dry on a frame of poles.
    HideFrame,
    /// A rack of antlers on a board between two posts.
    AntlerRack,
    /// Nets hung over a pole to dry and be mended.
    NetRack,
    /// Fish hung in pairs over the rails of a frame to dry.
    FishRack,
    /// Wicker funnels laid in a river to catch eels.
    EelTrap,
    /// Domed wicker pots for lobsters and crabs, on the shore.
    LobsterPot,
    /// The basket weaver's bundles of willow rods.
    WithyBundles,
    /// A long trough the weaver soaks willow in to make it supple.
    SoakingTrough,
    /// Baskets stacked, and a creel.
    BasketStack,
    // The yards and lanes.
    WaterTrough,
    Handcart,
    /// A barrel under the eaves catching the rain.
    WaterButt,
    /// The desert's water butt: a great clay jar on a ring of stones.
    WaterJar,
    /// Two posts and a line between them.
    WashingLine,
    /// A stone well head with a windlass and a little roof.
    StoneWell,
    /// A beam on a post with a weight at one end and a bucket at the other, to lift water
    /// from a well in dry country.
    Shaduf,
    /// A low ring of stones about the water of an oasis.
    OasisWell,
    /// A market stall: a frame of posts and a counter.
    MarketStall,
    /// A hen house on legs and its fenced run.
    ChickenCoop,
    /// A straw skep on a stand.
    Skep,
    /// A hollow log stood on end for bees, as in the northern forests.
    LogHive,
    /// A wooden box hive on legs.
    BoxHive,
    /// A thatched hay rick, roped down against the wind.
    HayRick,
    /// Hay built up round a pole, as in the north and the mountains.
    PoleRick,
    /// Split logs stacked between stakes against a wall, under a capping of boards.
    LogStack,
    /// A round stack of split logs with a capping of bark, as in the north.
    RoundStack,
    /// A wattle hurdle: rods woven between upright sails.
    Hurdle,
}

/// The three kinds of fitting, and the rows of the gallery.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Group {
    Furniture,
    Trade,
    Yard,
}

/// What a fitting stands against.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Stand {
    /// Its back against a wall: the wall is the row of columns at `z = -1`.
    Wall,
    /// On open floor or ground, with room on every side.
    Free,
    /// Hung by its top from a beam or the eaves: its highest layer is fixed to the
    /// underside of what it hangs from, and nothing is under it.
    Hung,
}

/// Where later work attaches to a fitting, in cells of its model space (points on the
/// grid of tenth-metre cells, so they turn as the fitting does).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Hook {
    /// A line strung from `from` to `to`, sagging `sag` cells in the middle: where washing
    /// hangs, from the cloth of `features::cloth` once there is some.
    Line { from: IVec3, to: IVec3, sag: i32 },
    /// The tops of a stall's back and front rails, from the back left corner `back` to the
    /// front right corner `front`: the rectangle an awning is stretched over, sloping
    /// down to the front.
    Awning { back: IVec3, front: IVec3 },
    /// Where a fire burns: the light at night and the smoke rise from here.
    Fire(IVec3),
    /// A hive's door, where its bees come and go.
    Hive(IVec3),
    /// Water to draw: the surface in a well, a trough or a butt.
    Water(IVec3),
    /// Two opposite corners of the floor of a run where animals are kept.
    Pen(IVec3, IVec3),
    /// Where someone lies down to sleep: the head end of a bed.
    Sleeper(IVec3),
}

impl Hook {
    /// This hook turned and moved as [`Fitted::place`] does its fitting.
    fn place(self, at: IVec3, turns: u32) -> Hook {
        // Points turn about the middle of voxel column 0, as fine boxes do.
        let c = IVec3::new(CELLS / 2, 0, CELLS / 2);
        let pt = |q: IVec3| rotate(q - c, turns) + c + at * CELLS;
        match self {
            Hook::Line { from, to, sag } => Hook::Line { from: pt(from), to: pt(to), sag },
            Hook::Awning { back, front } => Hook::Awning { back: pt(back), front: pt(front) },
            Hook::Fire(q) => Hook::Fire(pt(q)),
            Hook::Hive(q) => Hook::Hive(pt(q)),
            Hook::Water(q) => Hook::Water(pt(q)),
            Hook::Pen(a, b) => Hook::Pen(pt(a), pt(b)),
            Hook::Sleeper(q) => Hook::Sleeper(pt(q)),
        }
    }
}

/// The way every fitting faces in model space: its front, where it is used from, is
/// toward +Z.
pub const FRONT: IVec2 = IVec2::new(0, 1);

/// The way a fitting faces once turned by `turns` quarter turns (see [`Fitted::place`]).
pub fn front(turns: u32) -> IVec2 {
    let r = rotate(IVec3::new(FRONT.x, 0, FRONT.y), turns);
    IVec2::new(r.x, r.z)
}

impl Fitting {
    pub const ALL: [Fitting; 46] = [
        Fitting::StrawPallet,
        Fitting::BoxBed,
        Fitting::RopeBed,
        Fitting::Table,
        Fitting::Stool,
        Fitting::Shelf,
        Fitting::Chest,
        Fitting::Cradle,
        Fitting::HutHearth,
        Fitting::Loom,
        Fitting::GroundLoom,
        Fitting::Anvil,
        Fitting::Forge,
        Fitting::Grindstone,
        Fitting::Workbench,
        Fitting::Sawhorse,
        Fitting::LogDeck,
        Fitting::HerbRack,
        Fitting::HerbBunches,
        Fitting::HideFrame,
        Fitting::AntlerRack,
        Fitting::NetRack,
        Fitting::FishRack,
        Fitting::EelTrap,
        Fitting::LobsterPot,
        Fitting::WithyBundles,
        Fitting::SoakingTrough,
        Fitting::BasketStack,
        Fitting::WaterTrough,
        Fitting::Handcart,
        Fitting::WaterButt,
        Fitting::WaterJar,
        Fitting::WashingLine,
        Fitting::StoneWell,
        Fitting::Shaduf,
        Fitting::OasisWell,
        Fitting::MarketStall,
        Fitting::ChickenCoop,
        Fitting::Skep,
        Fitting::LogHive,
        Fitting::BoxHive,
        Fitting::HayRick,
        Fitting::PoleRick,
        Fitting::LogStack,
        Fitting::RoundStack,
        Fitting::Hurdle,
    ];

    /// Its name, in plain English.
    pub fn name(self) -> &'static str {
        use Fitting::*;
        match self {
            StrawPallet => "Straw pallet",
            BoxBed => "Box bed",
            RopeBed => "Rope bed",
            Table => "Table",
            Stool => "Stool",
            Shelf => "Shelf of pots",
            Chest => "Chest",
            Cradle => "Cradle",
            HutHearth => "Hut hearth",
            Loom => "Loom",
            GroundLoom => "Ground loom",
            Anvil => "Anvil",
            Forge => "Forge",
            Grindstone => "Grindstone",
            Workbench => "Workbench",
            Sawhorse => "Sawhorse",
            LogDeck => "Log deck",
            HerbRack => "Herb drying rack",
            HerbBunches => "Herbs under the eaves",
            HideFrame => "Hide frame",
            AntlerRack => "Antler rack",
            NetRack => "Net rack",
            FishRack => "Fish drying rack",
            EelTrap => "Eel traps",
            LobsterPot => "Lobster pots",
            WithyBundles => "Withy bundles",
            SoakingTrough => "Soaking trough",
            BasketStack => "Baskets",
            WaterTrough => "Water trough",
            Handcart => "Handcart",
            WaterButt => "Water butt",
            WaterJar => "Water jar",
            WashingLine => "Washing line",
            StoneWell => "Stone well",
            Shaduf => "Shaduf",
            OasisWell => "Oasis well",
            MarketStall => "Market stall",
            ChickenCoop => "Chicken coop",
            Skep => "Skep",
            LogHive => "Log hive",
            BoxHive => "Box hive",
            HayRick => "Hay rick",
            PoleRick => "Pole rick",
            LogStack => "Log stack",
            RoundStack => "Round log stack",
            Hurdle => "Wattle hurdle",
        }
    }

    pub fn group(self) -> Group {
        use Fitting::*;
        match self {
            StrawPallet | BoxBed | RopeBed | Table | Stool | Shelf | Chest | Cradle | HutHearth | Loom | GroundLoom => Group::Furniture,
            Anvil | Forge | Grindstone | Workbench | Sawhorse | LogDeck | HerbRack | HerbBunches | HideFrame | AntlerRack | NetRack | FishRack
            | EelTrap | LobsterPot | WithyBundles | SoakingTrough | BasketStack => Group::Trade,
            _ => Group::Yard,
        }
    }

    /// The room it takes, in voxels: it stands in columns `0..x` by `0..z` and reaches no
    /// higher than layer `y - 1`, in both its forms, before it is turned.
    pub fn size(self) -> IVec3 {
        use Fitting::*;
        let (x, y, z) = match self {
            StrawPallet => (5, 1, 2),
            BoxBed => (6, 5, 3),
            RopeBed => (5, 2, 2),
            Table => (4, 3, 2),
            Stool => (1, 2, 1),
            Shelf => (3, 4, 1),
            Chest => (2, 2, 2),
            Cradle => (3, 3, 2),
            HutHearth => (5, 4, 5),
            Loom => (4, 5, 2),
            GroundLoom => (5, 1, 3),
            Anvil => (3, 4, 3),
            Forge => (5, 6, 3),
            Grindstone => (3, 3, 3),
            Workbench => (5, 4, 2),
            Sawhorse => (3, 3, 2),
            LogDeck => (8, 3, 3),
            HerbRack => (3, 4, 2),
            HerbBunches => (4, 2, 1),
            HideFrame => (5, 5, 2),
            AntlerRack => (4, 5, 2),
            NetRack => (6, 5, 2),
            FishRack => (5, 5, 2),
            EelTrap => (3, 1, 2),
            LobsterPot => (2, 3, 2),
            WithyBundles => (5, 2, 2),
            SoakingTrough => (6, 2, 2),
            BasketStack => (3, 2, 2),
            WaterTrough => (4, 2, 2),
            Handcart => (5, 3, 3),
            WaterButt => (3, 4, 3),
            WaterJar => (3, 3, 3),
            WashingLine => (10, 5, 1),
            StoneWell => (6, 7, 6),
            Shaduf => (10, 9, 3),
            OasisWell => (5, 2, 5),
            MarketStall => (6, 6, 3),
            ChickenCoop => (8, 5, 6),
            Skep => (2, 4, 2),
            LogHive => (3, 3, 3),
            BoxHive => (2, 4, 3),
            HayRick => (8, 8, 7),
            PoleRick => (5, 8, 5),
            LogStack => (5, 4, 2),
            RoundStack => (5, 5, 5),
            Hurdle => (5, 3, 1),
        };
        IVec3::new(x, y, z)
    }

    /// The room this fitting takes once [`Fitted::place`]d at `at` with `turns`: voxels
    /// `lo` to `hi`, `hi` exclusive. Turned, the room swings about column 0 as the parts
    /// do, so a plan can check it fits, and lay it against a wall, before building it.
    pub fn room(self, at: IVec3, turns: u32) -> (IVec3, IVec3) {
        let s = self.size();
        let (a, b) = (rotate(IVec3::ZERO, turns), rotate(IVec3::new(s.x - 1, 0, s.z - 1), turns));
        (a.min(b) + at, a.max(b) + at + IVec3::new(1, s.y, 1))
    }

    pub fn stand(self) -> Stand {
        use Fitting::*;
        match self {
            StrawPallet | BoxBed | Shelf | Chest | Loom | Forge | Workbench | WaterButt | WaterJar | LogStack => Stand::Wall,
            HerbBunches => Stand::Hung,
            _ => Stand::Free,
        }
    }

    /// How many sleep in it: what a house's beds say about who lives there. A cradle
    /// holds a baby, who is counted.
    pub fn sleeps(self) -> u32 {
        match self {
            Fitting::BoxBed => 2,
            Fitting::StrawPallet | Fitting::RopeBed | Fitting::Cradle => 1,
            _ => 0,
        }
    }

    /// The form this fitting takes in a region, where its people have their own: a round
    /// log stack in the north, a clay jar for a water butt in the desert. Most are the
    /// same everywhere.
    pub fn local(self, region: Region) -> Fitting {
        use Region::*;
        match (self, region) {
            (Fitting::LogStack, Northern | Alpine) => Fitting::RoundStack,
            (Fitting::HayRick, Northern | Alpine) => Fitting::PoleRick,
            (Fitting::Loom, Desert) => Fitting::GroundLoom,
            (Fitting::WaterButt, Desert) => Fitting::WaterJar,
            (Fitting::Skep, Northern | Alpine | OldGrowth | Rainforest) => Fitting::LogHive,
            (f, _) => f,
        }
    }
}

/// The plants a forager of each region hangs up to dry, when no list is given: what
/// flowers there, and in the old forests and the north the mushrooms too.
pub fn herbs_of(region: Region) -> &'static [Flower] {
    use Flower::*;
    match region {
        Region::Temperate => &[Daisy, Poppy, Foxglove, Primrose, Cinquefoil, Campion],
        Region::Upland => &[Heather, Foxglove, Cinquefoil, Thistle],
        Region::Northern => &[CottonGrass, Heather, PennyBun, Stitchwort],
        Region::Alpine => &[Edelweiss, Gentian, Lupin],
        Region::Desert => &[DesertMarigold, Thistle, Poppy],
        Region::Wetland => &[MarshMarigold, FlagIris, CottonGrass],
        Region::Rainforest => &[Orchid, Heliconia],
        Region::OldGrowth => &[PennyBun, FlyAgaric, Foxglove, Bluebell],
    }
}

/// The colour a flower keeps when it is hung up to dry: its own, a little faded.
pub fn dried(f: Flower) -> Voxel {
    match f.petals() {
        mat::BLOOM_RED | mat::BLOOM_POPPY | mat::MUSHROOM_RED => mat::DRIED_RED,
        mat::BLOOM_YELLOW | mat::BLOOM_GOLD | mat::BLOOM_PRIMROSE => mat::DRIED_GOLD,
        mat::BLOOM_WHITE | mat::BLOOM_FELT => mat::DRIED_WHITE,
        mat::BLOOM_BLUE | mat::BLOOM_BLUEBELL | mat::BLOOM_LUPIN | mat::BLOOM_GENTIAN => mat::DRIED_BLUE,
        mat::BLOOM_PINK | mat::BLOOM_ORCHID => mat::DRIED_PINK,
        mat::MUSHROOM_TAN => mat::DRIED_BROWN,
        _ => mat::DRIED_PURPLE,
    }
}

/// A fitting as built: its voxels, its fine parts in cells, and its hooks.
#[derive(Clone, Debug)]
pub struct Fitted {
    pub kind: Fitting,
    pub decayed: bool,
    /// Quarter turns from model space: it faces [`front`]`(turns)`.
    pub turns: u32,
    pub voxels: Vec<(IVec3, Voxel)>,
    pub fine: Vec<PlantBox>,
    pub hooks: Vec<Hook>,
}

impl Fitted {
    /// Every voxel it fills or draws into, sorted.
    pub fn cells(&self) -> Vec<IVec3> {
        let mut out: Vec<IVec3> = self.voxels.iter().map(|&(q, _)| q).collect();
        for b in &self.fine {
            let (lo, hi) = (IVec3::from_array(b.lo).div_euclid(IVec3::splat(CELLS)), (IVec3::from_array(b.hi) - 1).div_euclid(IVec3::splat(CELLS)));
            for y in lo.y..=hi.y {
                for z in lo.z..=hi.z {
                    for x in lo.x..=hi.x {
                        out.push(IVec3::new(x, y, z));
                    }
                }
            }
        }
        out.sort_unstable_by_key(|q| (q.y, q.x, q.z));
        out.dedup();
        out
    }

    /// The voxel columns it stands in, sorted.
    pub fn footprint(&self) -> Vec<IVec2> {
        let mut out: Vec<IVec2> = self.cells().iter().map(|q| IVec2::new(q.x, q.z)).collect();
        out.sort_unstable_by_key(|c| (c.x, c.y));
        out.dedup();
        out
    }

    /// The way it faces.
    pub fn facing(&self) -> IVec2 {
        front(self.turns)
    }

    /// This fitting, built in model space, turned by `turns` quarter turns about the
    /// middle of its voxel column 0 and moved so that column stands at `at` (its foot on
    /// layer `at.y`): as a landmark's props are placed.
    pub fn place(&self, at: IVec3, turns: u32) -> Fitted {
        Fitted {
            kind: self.kind,
            decayed: self.decayed,
            turns: (self.turns + turns) & 3,
            voxels: self.voxels.iter().map(|&(q, v)| (rotate(q, turns) + at, v)).collect(),
            fine: parts::place_fine(&self.fine, at, turns),
            hooks: self.hooks.iter().map(|h| h.place(at, turns)).collect(),
        }
    }
}

/// Lays one fitting's parts in model space: boxes in cells, voxels, and hooks.
pub(crate) struct Kit {
    voxels: Vec<(IVec3, Voxel)>,
    fine: Vec<PlantBox>,
    hooks: Vec<Hook>,
}

type C = (i32, i32, i32);

impl Kit {
    fn new() -> Self {
        Kit { voxels: Vec::new(), fine: Vec::new(), hooks: Vec::new() }
    }

    fn part(&mut self, lo: C, hi: C, v: Voxel, sway_lo: bool, sway_hi: bool) {
        let (lo, hi) = (IVec3::from(lo), IVec3::from(hi));
        // A box turned inside out is a slip in the design, and the tests stop on it.
        if cfg!(any(test, debug_assertions)) {
            assert!(lo.cmplt(hi).all(), "empty fitting part {lo} {hi}");
        }
        if !lo.cmplt(hi).all() {
            return;
        }
        self.fine.push(PlantBox { lo: lo.to_array(), hi: hi.to_array(), voxel: v, sway_lo, sway_hi, thin: 0, season: 0, flap: 0 });
    }

    /// A box fixed in place.
    fn b(&mut self, lo: C, hi: C, v: Voxel) {
        self.part(lo, hi, v, false, false);
    }

    /// A weed whose top moves in the wind.
    fn weed(&mut self, lo: C, hi: C, v: Voxel) {
        self.part(lo, hi, v, false, true);
    }

    /// Something hanging, whose foot moves in the wind.
    fn hang(&mut self, lo: C, hi: C, v: Voxel) {
        self.part(lo, hi, v, true, false);
    }

    fn vox(&mut self, x: i32, y: i32, z: i32, v: Voxel) {
        self.voxels.push((IVec3::new(x, y, z), v));
    }

    fn hook(&mut self, h: Hook) {
        self.hooks.push(h);
    }

    /// A pole `w` cells thick along the line from `a` to `b` (points in cells), laid as a
    /// staircase of boxes along whichever way it runs furthest.
    fn rod(&mut self, a: (f32, f32, f32), b: (f32, f32, f32), w: i32, v: Voxel) {
        let (a, b) = (Vec3::from(a), Vec3::from(b));
        let d = b - a;
        let axis = if d.x.abs() >= d.y.abs() && d.x.abs() >= d.z.abs() {
            0
        } else if d.y.abs() >= d.z.abs() {
            1
        } else {
            2
        };
        let n = d[axis].abs().ceil().max(1.0) as i32;
        let half = w as f32 / 2.0;
        let step = |s: i32| {
            let p = a + d * ((s as f32 + 0.5) / n as f32);
            let mut lo = IVec3::ZERO;
            for i in 0..3 {
                lo[i] = if i == axis { p[i].floor() as i32 } else { (p[i] - half).round() as i32 };
            }
            lo
        };
        let mut size = IVec3::splat(w);
        size[axis] = 1;
        let (o, q) = ((axis + 1) % 3, (axis + 2) % 3);
        let mut last = step(0);
        let (mut rl, mut rh) = (last, last + size);
        for s in 1..n {
            let lo = step(s);
            if lo[o] == last[o] && lo[q] == last[q] {
                rl = rl.min(lo);
                rh = rh.max(lo + size);
            } else {
                self.b(rl.into(), rh.into(), v);
                // The next run starts a step back, so it meets the last face to face, not
                // only along an edge; where the pole steps both ways at once a knee joins
                // the two.
                let mut back = lo;
                back[axis] = last[axis];
                if lo[o] != last[o] && lo[q] != last[q] {
                    let mut knee = back;
                    knee[q] = last[q];
                    let mut knee_hi = knee + size;
                    knee_hi[axis] = lo[axis].max(last[axis]) + 1;
                    knee[axis] = lo[axis].min(last[axis]);
                    self.b(knee.into(), knee_hi.into(), v);
                }
                rl = lo.min(back);
                rh = (lo + size).max(back + size);
            }
            last = lo;
        }
        self.b(rl.into(), rh.into(), v);
    }

    /// A disc or ring standing upright, facing along Z between `z0` and `z1`: centred on
    /// `(cx, cy)` in cells, `r` across from the middle and hollow inside `hole` (0 for a
    /// solid disc). A wheel, a grindstone.
    fn round_xy(&mut self, (cx, cy): (f32, f32), r: f32, hole: f32, (z0, z1): (i32, i32), v: Voxel) {
        for (y0, y1, x0, x1) in round_rows(cy, cx, r, hole) {
            self.b((x0, y0, z0), (x1, y1, z1), v);
        }
    }

    /// A disc or ring lying flat between layers of cells `y0` and `y1`, centred on
    /// `(cx, cz)`. A well head, a coil of straw.
    fn round_xz(&mut self, (cx, cz): (f32, f32), r: f32, hole: f32, (y0, y1): (i32, i32), v: Voxel) {
        for (z0, z1, x0, x1) in round_rows(cz, cx, r, hole) {
            self.b((x0, y0, z0), (x1, y1, z1), v);
        }
    }

    /// One of a landmark's props, built with its voxel 0 at cell `at` (a whole number of
    /// voxels from the origin when it has voxels).
    fn prop(&mut self, prop: Prop, p: &Palette, at: C, turns: u32, decayed: bool, seed: u32) {
        let at = IVec3::from(at);
        let placed = props::build(prop, p, IVec3::ZERO, turns, decayed, seed);
        debug_assert!(placed.voxels.is_empty() || at % CELLS == IVec3::ZERO, "{prop:?} has voxels and is set between them");
        let shift = at.div_euclid(IVec3::splat(CELLS));
        self.voxels.extend(placed.voxels.into_iter().map(|(q, v)| (q + shift, v)));
        self.fine.extend(placed.fine.into_iter().map(|b| PlantBox { lo: (IVec3::from_array(b.lo) + at).to_array(), hi: (IVec3::from_array(b.hi) + at).to_array(), ..b }));
    }
}

/// The boxes of a disc or ring rasterised in rows, `(row0, row1, col0, col1)`, rows
/// that come out the same merged: centred on `(row, col)`, `r` across from the middle,
/// hollow inside `hole`.
fn round_rows(row: f32, col: f32, r: f32, hole: f32) -> Vec<(i32, i32, i32, i32)> {
    let mut spans: Vec<(i32, i32, i32, i32)> = Vec::new();
    let mut push = |y: i32, x0: i32, x1: i32| {
        if x1 <= x0 {
            return;
        }
        // Grow the box above when this row is the same as its last.
        if let Some(s) = spans.iter_mut().rev().find(|s| s.1 == y && s.2 == x0 && s.3 == x1) {
            s.1 = y + 1;
        } else {
            spans.push((y, y + 1, x0, x1));
        }
    };
    for y in (row - r).floor() as i32..(row + r).ceil() as i32 {
        let dy = y as f32 + 0.5 - row;
        if dy.abs() >= r {
            continue;
        }
        let out = (r * r - dy * dy).sqrt();
        let (o0, o1) = ((col - out).round() as i32, (col + out).round() as i32);
        if dy.abs() < hole {
            let inn = (hole * hole - dy * dy).sqrt();
            let (i0, i1) = ((col - inn).round() as i32, (col + inn).round() as i32);
            if i1 > i0 {
                push(y, o0, i0);
                push(y, i1, o1);
                continue;
            }
        }
        push(y, o0, o1);
    }
    spans
}

/// Whether a place is damp enough for rot to grow fungus and weeds to grow green.
fn damp(p: &Palette) -> bool {
    p.moss > 0.1
}

fn weed_colour(p: &Palette) -> Voxel {
    if damp(p) { mat::TUFT } else { mat::TUFT_DRY }
}

fn region(p: &Palette) -> Region {
    Region::of(p.biome)
}

/// Builds a fitting in model space from the palette of its place, intact or decayed,
/// varied by `seed`. The forager's racks and bunches are hung with the plants of the
/// region ([`herbs_of`]); [`build_with`] chooses them.
pub fn build(kind: Fitting, p: &Palette, seed: u32, decayed: bool) -> Fitted {
    build_with(kind, p, seed, decayed, herbs_of(region(p)))
}

/// As [`build`], with the forager's racks and bunches hung with `herbs` (the plants that
/// flower about the place, from `plants::flower_at`), or the region's own if it is empty.
pub fn build_with(kind: Fitting, p: &Palette, seed: u32, decayed: bool, herbs: &[Flower]) -> Fitted {
    let herbs = if herbs.is_empty() { herbs_of(region(p)) } else { herbs };
    let mut k = Kit::new();
    let mut rng = Rng::new(seed ^ 0x0f17_7e25 ^ (kind as u32).wrapping_mul(0x2545_f491));
    use Fitting::*;
    let r = &mut rng;
    match kind {
        StrawPallet => furniture::straw_pallet(&mut k, r, p, decayed),
        BoxBed => furniture::box_bed(&mut k, r, p, decayed),
        RopeBed => furniture::rope_bed(&mut k, r, p, decayed),
        Table => furniture::table(&mut k, r, p, decayed),
        Stool => furniture::stool(&mut k, r, p, decayed),
        Shelf => furniture::shelf(&mut k, r, p, decayed),
        Chest => furniture::chest(&mut k, r, p, decayed),
        Cradle => furniture::cradle(&mut k, r, p, decayed),
        HutHearth => furniture::hut_hearth(&mut k, r, p, decayed, seed),
        Loom => furniture::loom(&mut k, r, p, decayed),
        GroundLoom => furniture::ground_loom(&mut k, r, p, decayed),
        Anvil => trades::anvil(&mut k, r, p, decayed),
        Forge => trades::forge(&mut k, r, p, decayed),
        Grindstone => trades::grindstone(&mut k, r, p, decayed),
        Workbench => trades::workbench(&mut k, r, p, decayed),
        Sawhorse => trades::sawhorse(&mut k, r, p, decayed),
        LogDeck => trades::log_deck(&mut k, r, p, decayed),
        HerbRack => trades::herb_rack(&mut k, r, p, decayed, herbs),
        HerbBunches => trades::herb_bunches(&mut k, r, p, decayed, herbs),
        HideFrame => trades::hide_frame(&mut k, r, p, decayed),
        AntlerRack => trades::antler_rack(&mut k, r, p, decayed),
        NetRack => trades::net_rack(&mut k, r, p, decayed),
        FishRack => trades::fish_rack(&mut k, r, p, decayed),
        EelTrap => trades::eel_trap(&mut k, r, p, decayed),
        LobsterPot => trades::lobster_pot(&mut k, r, p, decayed),
        WithyBundles => trades::withy_bundles(&mut k, r, p, decayed),
        SoakingTrough => trades::soaking_trough(&mut k, r, p, decayed),
        BasketStack => trades::basket_stack(&mut k, r, p, decayed),
        WaterTrough => yard::water_trough(&mut k, r, p, decayed),
        Handcart => yard::handcart(&mut k, r, p, decayed),
        WaterButt => yard::water_butt(&mut k, r, p, decayed),
        WaterJar => yard::water_jar(&mut k, r, p, decayed),
        WashingLine => yard::washing_line(&mut k, r, p, decayed),
        StoneWell => yard::stone_well(&mut k, r, p, decayed),
        Shaduf => yard::shaduf(&mut k, r, p, decayed),
        OasisWell => yard::oasis_well(&mut k, r, p, decayed),
        MarketStall => yard::market_stall(&mut k, r, p, decayed),
        ChickenCoop => yard::chicken_coop(&mut k, r, p, decayed),
        Skep => yard::skep(&mut k, r, p, decayed),
        LogHive => yard::log_hive(&mut k, r, p, decayed),
        BoxHive => yard::box_hive(&mut k, r, p, decayed),
        HayRick => yard::hay_rick(&mut k, r, p, decayed),
        PoleRick => yard::pole_rick(&mut k, r, p, decayed),
        LogStack => yard::log_stack(&mut k, r, p, decayed),
        RoundStack => yard::round_stack(&mut k, r, p, decayed),
        Hurdle => yard::hurdle(&mut k, r, p, decayed),
    }
    Fitted { kind, decayed, turns: 0, voxels: k.voxels, fine: k.fine, hooks: k.hooks }
}

/// Every fitting laid out on plain ground to be looked at, for the structure editor: each
/// intact with its decayed form beside it, in rows by [`Group`] (or only `group`'s), all
/// facing +Z.
pub struct Gallery {
    pub voxels: Vec<(IVec3, Voxel)>,
    pub fine: Vec<PlantBox>,
    /// The room each fitting takes, in voxels, `lo` to `hi` exclusive, and which it is.
    pub spaces: Vec<(IVec3, IVec3, Fitting, bool)>,
}

/// Lays out every fitting, or only `group`'s, built from `p`, as [`Gallery`] says.
pub fn gallery(p: &Palette, seed: u32, group: Option<Group>) -> Gallery {
    // Pairs wrap onto a new row past this many columns.
    const ROW: i32 = 56;
    let mut g = Gallery { voxels: Vec::new(), fine: Vec::new(), spaces: Vec::new() };
    let (mut x, mut z, mut depth) = (0, 0, 0);
    let mut last = None;
    for (n, kind) in Fitting::ALL.into_iter().enumerate().filter(|(_, f)| group.is_none_or(|g| f.group() == g)) {
        let s = kind.size();
        let wide = 2 * s.x + 1;
        if x > 0 && (x + wide > ROW || last != Some(kind.group())) {
            // Rows run back from the first, away from a camera in front.
            z -= depth + 3;
            x = 0;
            depth = 0;
        }
        last = Some(kind.group());
        for (i, decayed) in [false, true].into_iter().enumerate() {
            let fitted = build(kind, p, seed.wrapping_mul(0x9e37_79b9) ^ n as u32, decayed);
            // Hung things hang from a beam on two posts, at about head height.
            let lift = if kind.stand() == Stand::Hung { 4 } else { 0 };
            let at = IVec3::new(x + i as i32 * (s.x + 1), lift, z - s.z);
            let placed = fitted.place(at, 0);
            if lift > 0 {
                let top = (at.y + s.y) * CELLS;
                let (x0, x1, z0) = (at.x * CELLS, (at.x + s.x) * CELLS, at.z * CELLS);
                for px in [x0 - 1, x1] {
                    g.fine.push(post((px, 0, z0), (px + 1, top + 1, z0 + 2), p.wood));
                }
                g.fine.push(post((x0 - 1, top, z0), (x1 + 1, top + 1, z0 + 2), p.wood));
            }
            g.voxels.extend(placed.voxels);
            g.fine.extend(placed.fine);
            g.spaces.push((IVec3::new(at.x, 0, at.z), IVec3::new(at.x + s.x, at.y + s.y, at.z + s.z), kind, decayed));
        }
        x += wide + 3;
        depth = depth.max(s.z);
    }
    // Centred on the origin.
    let (mut lo, mut hi) = (IVec3::MAX, IVec3::MIN);
    for &(a, b, ..) in &g.spaces {
        lo = lo.min(a);
        hi = hi.max(b);
    }
    let shift = IVec3::new(-(lo.x + hi.x) / 2, 0, -(lo.z + hi.z) / 2);
    for (q, _) in &mut g.voxels {
        *q += shift;
    }
    for b in &mut g.fine {
        for i in [0, 2] {
            b.lo[i] += shift[i] * CELLS;
            b.hi[i] += shift[i] * CELLS;
        }
    }
    for (a, b, ..) in &mut g.spaces {
        *a += shift;
        *b += shift;
    }
    g
}

fn post(lo: C, hi: C, v: Voxel) -> PlantBox {
    PlantBox { lo: [lo.0, lo.1, lo.2], hi: [hi.0, hi.1, hi.2], voxel: v, sway_lo: false, sway_hi: false, thin: 0, season: 0, flap: 0 }
}

/// The same as `v`, gone grey and soft with age: what wood and cloth turn to in a
/// decayed fitting.
fn aged(v: Voxel) -> Voxel {
    match v {
        mat::PLANK | mat::PLANK_RED | mat::PLANK_PALE => mat::PLANK_DARK,
        mat::CLOTH_RED | mat::CLOTH_BLUE | mat::CLOTH_GREEN | mat::CLOTH_YELLOW | mat::CLOTH_PALE => mat::SACKCLOTH,
        mat::ROPE => mat::ROPE_ROTTEN,
        mat::IRON => mat::RUST,
        mat::HAY | mat::THATCH => mat::HAY_ROTTEN,
        mat::WICKER | mat::WITHY => mat::ROPE_ROTTEN,
        mat::PLANK_DARK => mat::PLANK_DARK,
        v if v == AIR => AIR,
        _ => mat::WOOD_DEAD,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::biome::Biome;
    use std::collections::HashMap;

    type Key = (Vec<(IVec3, Voxel)>, Vec<([i32; 3], [i32; 3], Voxel, bool, bool)>, Vec<Hook>);

    fn key(f: &Fitted) -> Key {
        (f.voxels.clone(), f.fine.iter().map(|b| (b.lo, b.hi, b.voxel, b.sway_lo, b.sway_hi)).collect(), f.hooks.clone())
    }

    /// Every palette a fitting can be built from: each biome's, varied, and each built
    /// of each kind of rock.
    fn palettes() -> Vec<Palette> {
        let rocks = [mat::STONE, mat::GRANITE, mat::SANDSTONE, mat::RED_ROCK, mat::BASALT, mat::ROCK_PALE];
        let mut out = Vec::new();
        for (i, biome) in Biome::ALL.into_iter().enumerate() {
            out.push(Palette::of(biome));
            out.push(Palette::local(biome, rocks[i % rocks.len()]).varied(i as u32 * 7 + 1));
        }
        out
    }

    /// The boxes of a fitting in cells, its voxels among them.
    fn boxes(f: &Fitted) -> Vec<(IVec3, IVec3, Voxel)> {
        let mut out: Vec<(IVec3, IVec3, Voxel)> = f.voxels.iter().map(|&(q, v)| (q * CELLS, (q + 1) * CELLS, v)).collect();
        out.extend(f.fine.iter().map(|b| (IVec3::from_array(b.lo), IVec3::from_array(b.hi), b.voxel)));
        out
    }

    /// Whether two boxes share some of a face: they meet or overlap on every axis, and
    /// overlap on at least two.
    fn touching(a: (IVec3, IVec3), b: (IVec3, IVec3)) -> bool {
        let over = |i: usize| a.1[i].min(b.1[i]) - a.0[i].max(b.0[i]);
        (0..3).all(|i| over(i) >= 0) && (0..3).filter(|&i| over(i) > 0).count() >= 2
    }

    /// What is wrong, gathered a couple to a fitting, so that one run shows all of it.
    #[derive(Default)]
    struct Faults(Vec<String>, HashMap<(Fitting, bool), usize>);

    impl Faults {
        fn add(&mut self, kind: Fitting, decayed: bool, what: String) {
            let n = self.1.entry((kind, decayed)).or_default();
            if *n < 2 {
                self.0.push(format!("{kind:?} (decayed {decayed}): {what}"));
            }
            *n += 1;
        }

        fn none(self) {
            assert!(self.0.is_empty(), "\n{}", self.0.join("\n"));
        }
    }

    #[test]
    fn every_fitting_builds_in_every_palette_and_keeps_to_its_room() {
        let mut faults = Faults::default();
        for p in palettes() {
            for kind in Fitting::ALL {
                let s = kind.size();
                for seed in 0..6 {
                    for decayed in [false, true] {
                        let f = build(kind, &p, seed, decayed);
                        assert!(!f.fine.is_empty() || !f.voxels.is_empty(), "{kind:?} is empty");
                        for b in &f.fine {
                            assert!((0..3).all(|i| b.lo[i] < b.hi[i]), "{kind:?}: empty part {b:?}");
                            assert_ne!(b.voxel, AIR, "{kind:?}: a part of air");
                        }
                        for q in f.cells() {
                            if !(q.cmpge(IVec3::ZERO).all() && q.cmplt(s).all()) {
                                faults.add(kind, decayed, format!("seed {seed} reaches {q} outside its room {s} in {:?}", p.biome));
                            }
                        }
                        // Nothing a fitting is built of is grey stone where the rock is not.
                        if p.stone != mat::STONE && p.stone_dark != mat::STONE && boxes(&f).iter().any(|b| b.2 == mat::STONE) {
                            faults.add(kind, decayed, format!("grey stone in {:?}", p.biome));
                        }
                    }
                }
            }
        }
        faults.none();
    }

    #[test]
    fn fittings_are_deterministic_and_decay() {
        let p = Palette::of(Biome::Deciduous);
        for kind in Fitting::ALL {
            for seed in [0, 5, 77] {
                let a = build(kind, &p, seed, false);
                assert_eq!(key(&a), key(&build(kind, &p, seed, false)), "{kind:?} is not deterministic");
                assert_ne!(key(&a), key(&build(kind, &p, seed, true)), "{kind:?} does not decay");
            }
        }
    }

    #[test]
    fn nothing_floats() {
        // Every part is joined, face to face through the others, to the ground; or, for a
        // fitting that hangs, to the beam it hangs from.
        let mut faults = Faults::default();
        for p in [Palette::of(Biome::Deciduous), Palette::of(Biome::Desert), Palette::of(Biome::Taiga), Palette::of(Biome::Moor)] {
            for kind in Fitting::ALL {
                let top = kind.size().y * CELLS;
                for seed in 0..8 {
                    for decayed in [false, true] {
                        let f = build(kind, &p, seed, decayed);
                        let parts = boxes(&f);
                        let hung = kind.stand() == Stand::Hung;
                        let mut held: Vec<bool> = parts.iter().map(|b| if hung { b.1.y == top } else { b.0.y == 0 }).collect();
                        assert!(held.iter().any(|&h| h), "{kind:?} touches neither the ground nor its beam");
                        if !hung {
                            assert!(parts.iter().all(|b| b.0.y >= 0), "{kind:?} reaches into the ground");
                        }
                        let mut grew = true;
                        while grew {
                            grew = false;
                            for i in 0..parts.len() {
                                if !held[i] && (0..parts.len()).any(|j| held[j] && touching((parts[i].0, parts[i].1), (parts[j].0, parts[j].1))) {
                                    held[i] = true;
                                    grew = true;
                                }
                            }
                        }
                        for (i, b) in parts.iter().enumerate() {
                            if !held[i] {
                                faults.add(kind, decayed, format!("seed {seed}: {} to {} of {:?} floats", b.0, b.1, mat::name_of(b.2)));
                            }
                        }
                    }
                }
            }
        }
        faults.none();
    }

    #[test]
    fn no_two_surfaces_fight() {
        // Two boxes of different stuff never put a face in the same place facing the same
        // way, where the two would flicker through each other. Undersides on the ground
        // are never seen, so those may.
        let mut faults = Faults::default();
        for p in [Palette::of(Biome::Deciduous), Palette::of(Biome::Desert), Palette::of(Biome::Taiga)] {
            for kind in Fitting::ALL {
                let grounded = kind.stand() != Stand::Hung;
                for seed in 0..6 {
                    for decayed in [false, true] {
                        let parts = boxes(&build(kind, &p, seed, decayed));
                        for (i, a) in parts.iter().enumerate() {
                            for b in &parts[i + 1..] {
                                if a.2 == b.2 {
                                    continue;
                                }
                                for axis in 0..3 {
                                    let (u, w) = ((axis + 1) % 3, (axis + 2) % 3);
                                    let over = |i: usize| a.1[i].min(b.1[i]) - a.0[i].max(b.0[i]);
                                    if over(u) <= 0 || over(w) <= 0 {
                                        continue;
                                    }
                                    for (lo, fa, fb) in [(true, a.0[axis], b.0[axis]), (false, a.1[axis], b.1[axis])] {
                                        if fa == fb && !(grounded && lo && axis == 1 && fa == 0) {
                                            let (na, nb) = (mat::name_of(a.2).unwrap_or("?"), mat::name_of(b.2).unwrap_or("?"));
                                            faults.add(kind, decayed, format!("seed {seed}: {na} {}..{} and {nb} {}..{} share a face", a.0, a.1, b.0, b.1));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        faults.none();
    }

    #[test]
    fn placing_turns_parts_and_hooks_together() {
        let p = Palette::of(Biome::Deciduous);
        for kind in [Fitting::WashingLine, Fitting::MarketStall, Fitting::HutHearth, Fitting::Skep] {
            let f = build(kind, &p, 3, false);
            assert!(!f.hooks.is_empty(), "{kind:?} has no hooks");
            for turns in 0..4 {
                let at = IVec3::new(10, 5, -7);
                let placed = f.place(at, turns);
                assert_eq!(placed.facing(), front(turns));
                assert_eq!(placed.footprint().len(), f.footprint().len());
                // A hook keeps its place among the parts: as far from the first fine box's
                // corner as before, once both are turned.
                let c = IVec3::new(CELLS / 2, 0, CELLS / 2);
                let corner = |b: &PlantBox| IVec3::from_array(b.lo);
                let (Hook::Fire(h) | Hook::Hive(h) | Hook::Line { from: h, .. } | Hook::Awning { back: h, .. }) = f.hooks[0] else { continue };
                let (Hook::Fire(g) | Hook::Hive(g) | Hook::Line { from: g, .. } | Hook::Awning { back: g, .. }) = placed.hooks[0] else { panic!() };
                assert_eq!(g, rotate(h - c, turns) + c + at * CELLS);
                let (lo, hi) = parts::rotate_box(corner(&f.fine[0]), IVec3::from_array(f.fine[0].hi), turns);
                assert_eq!(placed.fine[0].lo, (lo + at * CELLS).to_array());
                assert_eq!(placed.fine[0].hi, (hi + at * CELLS).to_array());
            }
        }
        assert_eq!(front(0), IVec2::new(0, 1));
        assert_eq!(front(1), IVec2::new(-1, 0));
        // Every fitting, however turned, keeps to the room it says it takes.
        for kind in Fitting::ALL {
            let f = build(kind, &p, 1, false);
            for turns in 0..4 {
                let at = IVec3::new(-3, 2, 6);
                let (lo, hi) = kind.room(at, turns);
                assert_eq!((hi - lo).x * (hi - lo).z, kind.size().x * kind.size().z);
                for q in f.place(at, turns).cells() {
                    assert!(q.cmpge(lo).all() && q.cmplt(hi).all(), "{kind:?} turned {turns}: {q} outside {lo}..{hi}");
                }
            }
        }
    }

    #[test]
    fn beds_say_who_sleeps_in_them_and_hooks_are_where_they_should_be() {
        let p = Palette::of(Biome::Deciduous);
        for kind in Fitting::ALL {
            let f = build(kind, &p, 1, false);
            let sleepers = f.hooks.iter().filter(|h| matches!(h, Hook::Sleeper(_))).count();
            assert_eq!(sleepers > 0, kind.sleeps() > 0, "{kind:?}");
            let has = |m: fn(&Hook) -> bool| f.hooks.iter().any(m);
            match kind {
                Fitting::WashingLine => assert!(has(|h| matches!(h, Hook::Line { .. }))),
                Fitting::MarketStall => assert!(has(|h| matches!(h, Hook::Awning { .. }))),
                Fitting::HutHearth | Fitting::Forge => assert!(has(|h| matches!(h, Hook::Fire(_)))),
                Fitting::Skep | Fitting::LogHive | Fitting::BoxHive => assert!(has(|h| matches!(h, Hook::Hive(_)))),
                Fitting::StoneWell | Fitting::Shaduf | Fitting::OasisWell => assert!(has(|h| matches!(h, Hook::Water(_)))),
                Fitting::ChickenCoop => assert!(has(|h| matches!(h, Hook::Pen(..)))),
                _ => {}
            }
            // Every hook is within the fitting's room.
            let s = kind.size() * CELLS;
            for h in &f.hooks {
                let points = match *h {
                    Hook::Line { from, to, .. } => vec![from, to],
                    Hook::Awning { back, front } => vec![back, front],
                    Hook::Pen(a, b) => vec![a, b],
                    Hook::Fire(q) | Hook::Hive(q) | Hook::Water(q) | Hook::Sleeper(q) => vec![q],
                };
                for q in points {
                    assert!(q.cmpge(IVec3::ZERO).all() && q.cmple(s).all(), "{kind:?}: {h:?} outside");
                }
            }
        }
    }

    #[test]
    fn the_forager_hangs_what_she_is_given() {
        let p = Palette::of(Biome::Moor);
        for kind in [Fitting::HerbRack, Fitting::HerbBunches] {
            let heather = build_with(kind, &p, 2, false, &[Flower::Heather]);
            assert!(heather.fine.iter().any(|b| b.voxel == mat::DRIED_PURPLE), "{kind:?} hangs no heather");
            assert!(heather.fine.iter().all(|b| b.voxel != mat::DRIED_GOLD), "{kind:?} hangs what it was not given");
            let poppies = build_with(kind, &p, 2, false, &[Flower::Poppy]);
            assert!(poppies.fine.iter().any(|b| b.voxel == mat::DRIED_RED), "{kind:?} hangs no poppies");
        }
        // Dried flowers keep their colour all year: none is a plant the seasons take.
        let mats = mat::build();
        for &f in Flower::ALL {
            assert_eq!(mats.get(dried(f)).plant, voxel::plant::NONE, "{f:?}");
        }
    }

    #[test]
    fn regions_have_their_own_forms() {
        assert_eq!(Fitting::LogStack.local(Region::Northern), Fitting::RoundStack);
        assert_eq!(Fitting::WaterButt.local(Region::Desert), Fitting::WaterJar);
        assert_eq!(Fitting::Table.local(Region::Desert), Fitting::Table);
        for r in Region::ALL {
            assert!(!herbs_of(r).is_empty());
            for f in Fitting::ALL {
                // A regional form can go where what it replaces was planned to: one that
                // stands free can also stand against a wall, but not the other way about.
                let (was, is) = (f.stand(), f.local(r).stand());
                assert!(was == is || (was == Stand::Wall && is == Stand::Free), "{f:?} in {r:?}");
            }
        }
    }

    #[test]
    fn the_gallery_has_every_fitting_and_none_overlap() {
        let g = gallery(&Palette::of(Biome::Deciduous), 1, None);
        assert_eq!(g.spaces.len(), Fitting::ALL.len() * 2);
        for (i, a) in g.spaces.iter().enumerate() {
            for b in &g.spaces[i + 1..] {
                let apart = (a.1.x <= b.0.x || b.1.x <= a.0.x) || (a.1.z <= b.0.z || b.1.z <= a.0.z);
                assert!(apart, "{:?} and {:?} overlap in the gallery", a.2, b.2);
            }
        }
        let mut seen: HashMap<IVec3, Voxel> = HashMap::new();
        for &(q, v) in &g.voxels {
            assert!(seen.insert(q, v).is_none(), "two voxels at {q}");
        }
        let one = gallery(&Palette::of(Biome::Deciduous), 1, Some(Group::Yard));
        assert_eq!(one.spaces.len(), 2 * Fitting::ALL.iter().filter(|f| f.group() == Group::Yard).count());
    }
}
