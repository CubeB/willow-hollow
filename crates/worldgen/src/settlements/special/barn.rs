//! Barns out in the fields (#64): what a farm keeps at the edge of its fields away from
//! the village, built in its region's own way and walked into through its door as every
//! building is. They are outlying workplaces, and a settlement's count of buildings
//! leaves them out (#45).
//!
//! - A hay barn: a tall store with a cart door, its loft piled with hay up a ladder, the
//!   cart and the tools of the fields below, hay heaped in a corner and a rick outside.
//! - A byre: the cattle's house, a stall for each beast along its back wall with a manger
//!   of hay at its head, a loft of hay over them up a ladder, and a trough by the door.
//! - A field shed: in poorer or drier country, a small shed for the tools and the sacks,
//!   raised on stilts in the marsh and the rainforest.
//! - A mason's shed, by a quarry (#203): the banker stones the stone is dressed on, each
//!   with a stone half squared and the chisel and mallet laid by, a piece half carved,
//!   the grindstone the chisels are kept sharp on and the tools on the wall.
//! - A field shelter on a pasture (#64, #206): three walls and a roof, open all along its
//!   front so the beasts walk in out of the weather, with a manger of hay along the back
//!   wall and straw on its floor. It has no door to shut: the whole front is the way in.
//!
//! The walls, roof and floor are the region's, as the kit's rules choose them for a
//! timber house (see [`super::frame::Look`]): weatherboard and thatch in the lowlands,
//! stone and slate in the uplands, logs and turf in the north, a stone foot under logs
//! and stones on the shingle in the Alps, mud brick under a flat roof in the desert.
//!
//! As the other buildings here, a barn is built in model space with its door to +X; the
//! caller turns it with [`Barn::turned`].

use glam::{IVec2, IVec3};

use super::{
    Facts, SpecialSpec, Use,
    frame::{Block, Frame, Roof, v3},
};
use crate::{
    features::{designs::Special, fittings::Fitting, landmarks::Palette, props::Prop, shape::rotate},
    materials as mat,
    settlements::{
        Region,
        kit::{Area, Building, rules::{FloorKind, Under}},
    },
};

/// What an outlying farm building is for.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BarnKind {
    /// A store for the hay and the harvest, and the cart.
    Hay,
    /// The house of the cattle, with their hay over them.
    Byre,
    /// A small shed for the tools.
    Shed,
    /// A mason's shed by a quarry, where the stone is dressed.
    Mason,
    /// An open-fronted shelter for the beasts on a pasture (see `farmland::fixtures`).
    Shelter,
}

impl BarnKind {
    pub const ALL: [BarnKind; 5] = [BarnKind::Hay, BarnKind::Byre, BarnKind::Shed, BarnKind::Mason, BarnKind::Shelter];

    pub fn word(self) -> &'static str {
        match self {
            BarnKind::Hay => "hay barn",
            BarnKind::Byre => "byre",
            BarnKind::Shed => "field shed",
            BarnKind::Mason => "mason's shed",
            BarnKind::Shelter => "field shelter",
        }
    }
}

/// What barn to build.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BarnSpec {
    pub kind: BarnKind,
    pub region: Region,
    /// 0 new to 1 old, as [`SpecialSpec::age`].
    pub age: f32,
    /// 0 poorest to 1 richest: how big it is, and whether a rick stands by it.
    pub wealth: f32,
    pub seed: u32,
}

/// A barn as built, in model space with its door to +X.
#[derive(Clone, Debug)]
pub struct Barn {
    pub kind: BarnKind,
    pub building: Building,
    /// Where each beast of a byre stands in its stall: the voxel its feet are in, and the
    /// way it faces, toward its manger.
    pub stalls: Vec<(IVec3, IVec3)>,
    /// Where each thing in it is used from: the voxel stood on, inside the walls.
    pub uses: Vec<(Use, IVec3)>,
}

impl Barn {
    /// Turned by quarter turns about the vertical through the origin, as
    /// [`Building::turned`] turns the building.
    pub fn turned(&self, turns: u32) -> Barn {
        let r = |q: IVec3| rotate(q, turns);
        Barn {
            kind: self.kind,
            building: self.building.turned(turns),
            stalls: self.stalls.iter().map(|&(q, d)| (r(q), r(d))).collect(),
            uses: self.uses.iter().map(|&(u, q)| (u, r(q))).collect(),
        }
    }
}

/// The most columns across a barn takes, x and z, its eaves and the rick beside it
/// included.
pub const LARGEST: (i32, i32) = (24, 40);

/// Builds the barn `spec` asks for, of the materials of palette `pal`.
pub fn build(spec: &BarnSpec, pal: &Palette) -> Barn {
    make(spec, pal).0
}

/// [`build`], and whatever was put where it should not have been (see `special::make`).
pub(crate) fn make(spec: &BarnSpec, pal: &Palette) -> (Barn, Vec<String>) {
    // The frame chooses a timber building's look for anything but the market hall, the
    // guild house, and a rich smithy or tavern: a barn takes the granary's place there,
    // a farm's store as the granary is the town's.
    let as_special = SpecialSpec { which: Special::Granary, region: spec.region, age: spec.age, wealth: spec.wealth, seed: spec.seed, stream: None, facts: Facts::default() };
    let f = Frame::new(&as_special, pal);
    let mut stalls = Vec::new();
    let (built, faults) = match spec.kind {
        BarnKind::Hay => hay(f, spec),
        BarnKind::Byre => byre(f, spec, &mut stalls),
        BarnKind::Shed => shed(f),
        BarnKind::Mason => mason(f),
        BarnKind::Shelter => shelter(f),
    };
    (Barn { kind: spec.kind, building: built.building, stalls, uses: built.uses }, faults)
}

/// The roof every barn has: the region's, its ridge running the length of the barn.
fn roof(f: &mut Frame, b: &Block) {
    let r = Roof { shape: f.look.shape, along_z: true, overhang: f.look.overhang, pitch: f.look.pitch.max(0.9), cover: f.look.cover };
    f.roof(b, &r);
}

/// A barn's floor: beaten earth where the region would board a home's.
fn earth_floor(f: &mut Frame) {
    if f.look.floor == FloorKind::Boards {
        f.look.floor = FloorKind::Earth;
    }
}

/// A wide door's two leaves standing open inside against the +X wall of `b`, either side
/// of a doorway from column `z0`, `width` wide and `height` high.
fn two_leaves(f: &mut Frame, b: &Block, z0: i32, width: i32, height: i32) {
    f.door_leaf(b, z0, width, height);
    let x = b.hi.x * 4;
    let (y0, y1) = (b.feet() * 4, (b.feet() + height.min(5)) * 4 - 1);
    let leaf = (width.min(3) * 4 - 1).max(3);
    let board = if f.neglect { mat::PLANK_DARK } else { f.pal.plank };
    f.bx(IVec3::new(x - 1, y0, z0 * 4 - leaf), IVec3::new(x, y1, z0 * 4), board);
    f.bx(IVec3::new(x - 2, y0 + 3, z0 * 4 - leaf + 1), IVec3::new(x - 1, y0 + 4, z0 * 4 - 1), f.pal.wood);
    f.bx(IVec3::new(x - 2, y1 - 4, z0 * 4 - leaf + 1), IVec3::new(x - 1, y1 - 3, z0 * 4 - 1), f.pal.wood);
}

/// A loft of boards at layer `y` over the inner columns of `b` from its back wall to
/// column `x1`, and hay heaped on it a layer or two deep wherever the roof leaves room,
/// but for the columns at `keep` (the ladder's head).
fn loft(f: &mut Frame, b: &Block, y: i32, x1: i32, keep: &[IVec2]) {
    for c in b.inner() {
        if c.x > x1 {
            continue;
        }
        f.put(v3(c, y), f.pal.plank);
        if keep.contains(&c) {
            continue;
        }
        let high = 1 + (f.grain(v3(c, y), 0x10f7) < 0.55) as i32;
        for k in 1..=high {
            if f.roof_low(c).is_some_and(|low| y + k < low) {
                f.put(v3(c, y + k), if f.grain(v3(c, y + k), 0x4a7) < 0.15 { mat::DRIED_GOLD } else { mat::HAY });
            }
        }
    }
}

/// A ladder in column `c` up to a loft at layer `top` from the floor at `feet`, its rails
/// against the loft's edge on the -X side of the column, rungs every few cells, and a
/// hand's height over the loft to step off by.
fn ladder(f: &mut Frame, c: IVec2, feet: i32, top: i32) {
    let wood = f.pal.wood;
    let (x, z) = (c.x * 4, c.y * 4);
    let (y0, y1) = (feet * 4, (top + 2) * 4);
    for zr in [z, z + 3] {
        f.bx(IVec3::new(x, y0, zr), IVec3::new(x + 1, y1, zr + 1), wood);
    }
    let mut y = y0 + 2;
    while y < y1 - 1 {
        f.bx(IVec3::new(x, y, z + 1), IVec3::new(x + 1, y + 1, z + 3), wood);
        y += 3;
    }
}

/// The tools of the fields hung on a wall: a rake, a pitchfork, a spade and a scythe on a
/// rail `len` columns long from column `from`, at layer `y`. The wall's inner face is at
/// cell `face` across it, along x where `along_x`, else along z; `inward` is the way into
/// the room across it (+1 or -1).
fn tools(f: &mut Frame, along_x: bool, face: i32, inward: i32, from: i32, len: i32, y: i32) {
    let (wood, iron) = (f.pal.wood, mat::IRON);
    let yc = y * 4 + 3;
    // A box from `a0..a1` along the wall and `y0..y1` up, `d` cells out from its face.
    let mut put = |a0: i32, a1: i32, y0: i32, y1: i32, d: i32, v| {
        let (c0, c1) = if inward > 0 { (face + d, face + d + 1) } else { (face - d - 1, face - d) };
        if along_x {
            f.bx(IVec3::new(a0, y0, c0), IVec3::new(a1, y1, c1), v);
        } else {
            f.bx(IVec3::new(c0, y0, a0), IVec3::new(c1, y1, a1), v);
        }
    };
    put(from * 4, (from + len) * 4, yc, yc + 1, 0, wood);
    for k in 0..len {
        let a = from * 4 + 1 + k * 4;
        match k % 4 {
            // A rake, its head up.
            0 => {
                put(a + 1, a + 2, yc - 9, yc + 2, 1, wood);
                put(a - 1, a + 4, yc + 2, yc + 3, 1, wood);
            }
            // A pitchfork, its tines up.
            1 => {
                put(a + 1, a + 2, yc - 9, yc + 1, 1, wood);
                put(a, a + 3, yc + 1, yc + 2, 1, iron);
                put(a, a + 1, yc + 2, yc + 4, 1, iron);
                put(a + 2, a + 3, yc + 2, yc + 4, 1, iron);
            }
            // A spade, its blade down.
            2 => {
                put(a + 1, a + 2, yc - 5, yc + 2, 1, wood);
                put(a, a + 3, yc - 8, yc - 5, 1, iron);
            }
            // A scythe: the long snath, and the blade along the top.
            _ => {
                put(a, a + 1, yc - 10, yc + 2, 1, wood);
                put(a, a + 4, yc + 2, yc + 3, 2, iron);
            }
        }
    }
}

/// A hay barn: the door to the cart in the long +X wall, a loft over the back, the cart
/// and the tools on the floor, and the rick outside the -Z end.
fn hay(mut f: Frame, spec: &BarnSpec) -> (super::Built, Vec<String>) {
    earth_floor(&mut f);
    let half = 6 + (spec.wealth * 3.0) as i32;
    let fy = 0;
    let b = Block::new((-7, -half), (6, half), fy, fy + 9);
    let feet = b.feet();
    f.shell(&b);
    roof(&mut f, &b);
    // The loft over the back four columns, up its ladder near the +Z end.
    let up = fy + 6;
    let at = IVec2::new(-2, half - 3);
    loft(&mut f, &b, up, -3, &[IVec2::new(-3, at.y), IVec2::new(-3, at.y + 1)]);
    ladder(&mut f, at, feet, up);
    // The cart door, wide and tall, its leaves standing open.
    let dz = -2;
    f.front_door(&b, dz, 4, 6);
    two_leaves(&mut f, &b, dz, 4, 6);
    // The cart, the bench with the tools over it, hay heaped under the loft and a sack
    // or two of seed.
    f.fit(Fitting::Handcart, IVec3::new(0, feet, -half + 1), IVec3::Z, false);
    f.fit(Fitting::Workbench, IVec3::new(0, feet, half - 2), IVec3::NEG_Z, false);
    tools(&mut f, true, half * 4, -1, 0, 5, feet + 3);
    for z in -half + 1..=-half + 3 {
        for x in -6..=-4 {
            let high = 1 + (f.grain(IVec3::new(x, 0, z), 0x4a9) * 2.5) as i32;
            for y in feet..feet + high {
                f.put(IVec3::new(x, y, z), mat::HAY);
            }
        }
    }
    f.prop(Prop::Sack, IVec3::new(-6, feet, 1), 0, false);
    f.prop(Prop::Sack, IVec3::new(-6, feet, 3), 0, false);
    f.uses.push((Use::Store, IVec3::new(2, fy, 0)));
    // Vents high in the gable ends.
    f.window(IVec2::new(-1, b.lo.y), up + 1, 2, 1, IVec3::NEG_Z);
    f.window(IVec2::new(-1, b.hi.y), up + 1, 2, 1, IVec3::Z);
    // A rick outside, where the farm is well enough off to have more hay than its barn
    // holds; in the desert nothing grows to make one.
    if spec.wealth > 0.35 && spec.region != Region::Desert {
        f.fit(Fitting::HayRick, IVec3::new(b.lo.x, 0, b.lo.y - 10), IVec3::Z, false);
    }
    f.finish(Area { lo: b.lo, hi: b.hi }, fy)
}

/// A byre: a stall for each beast along the back wall, its manger at its head, the loft
/// over them, and an aisle from the door to muck out along.
fn byre(mut f: Frame, spec: &BarnSpec, stalls: &mut Vec<(IVec3, IVec3)>) -> (super::Built, Vec<String>) {
    // Flags underfoot, to sluice down.
    f.look.floor = FloorKind::Flags;
    let half = 5 + (spec.wealth * 3.0) as i32;
    let fy = 0;
    let b = Block::new((-6, -half), (4, half), fy, fy + 8);
    let feet = b.feet();
    f.shell(&b);
    roof(&mut f, &b);
    let up = fy + 6;
    // The stalls: boards between them, three columns wide, the manger along the wall at
    // their heads with hay in it.
    let z0 = -half + 1;
    let count = (2 * half - 2) / 4;
    for k in 0..=count {
        let z = z0 + 4 * k;
        for x in -4..=-2 {
            for y in feet..feet + 3 {
                f.put(IVec3::new(x, y, z), f.pal.plank);
            }
        }
    }
    for z in z0..z0 + 4 * count + 1 {
        f.put(IVec3::new(-5, feet, z), f.pal.wood);
        f.put(IVec3::new(-5, feet + 1, z), mat::HAY);
    }
    for k in 0..count {
        stalls.push((IVec3::new(-3, feet, z0 + 4 * k + 2), IVec3::NEG_X));
    }
    // The loft over the stalls, its ladder at the far end of the aisle.
    let at = IVec2::new(-1, half - 1);
    loft(&mut f, &b, up, -2, &[IVec2::new(-2, at.y), IVec2::new(-2, at.y - 1)]);
    ladder(&mut f, at, feet, up);
    let dz = -1;
    f.front_door(&b, dz, 3, 5);
    f.door_leaf(&b, dz, 3, 5);
    // A fork and a spade by the door, a window to see the beasts by, and the trough
    // outside.
    tools(&mut f, false, (b.hi.x) * 4, -1, -half + 1, 2, feet + 2);
    f.window(IVec2::new(b.hi.x, half - 2), feet + 2, 1, 1, IVec3::X);
    f.fit(Fitting::WaterTrough, IVec3::new(b.hi.x + 2, 0, 3), IVec3::X, false);
    f.uses.push((Use::Store, IVec3::new(1, fy, 0)));
    f.finish(Area { lo: b.lo, hi: b.hi }, fy)
}

/// A field shed: the bench and tools along the back wall, sacks, and what the region sets
/// by a door; up steps on stilts where the ground is wet.
fn shed(mut f: Frame) -> (super::Built, Vec<String>) {
    let raised = matches!(f.region(), Region::Wetland | Region::Rainforest);
    let fy = if raised { 3 } else { 0 };
    if raised {
        f.look.under = Under::Stilts;
        f.look.floor = FloorKind::Boards;
    } else {
        earth_floor(&mut f);
    }
    let b = Block::new((-4, -3), (3, 3), fy, fy + 6);
    let feet = b.feet();
    f.shell(&b);
    roof(&mut f, &b);
    let dz = -1;
    f.front_door(&b, dz, 2, 5);
    f.door_leaf(&b, dz, 2, 5);
    f.front_steps(&b, dz, 2, 1);
    f.fit(Fitting::Workbench, IVec3::new(-3, feet, -2), IVec3::X, false);
    tools(&mut f, false, -3 * 4, 1, -2, 4, feet + 3);
    f.prop(Prop::Sack, IVec3::new(1, feet, 1), 0, false);
    f.prop(Prop::Sack, IVec3::new(1, feet, -2), 0, false);
    f.window(IVec2::new(0, b.lo.y), feet + 2, 1, 1, IVec3::NEG_Z);
    f.uses.push((Use::Store, IVec3::new(0, fy, 0)));
    f.porch(&b, dz, 2);
    f.finish(Area { lo: b.lo, hi: b.hi }, fy)
}

/// A field shelter: a long low shed of the region's walls and roof with the whole of its
/// +X side open between the corner posts, a manger of hay along the back wall for the
/// beasts to pull at, and straw strewn on the beaten earth. A person stands up in it.
fn shelter(mut f: Frame) -> (super::Built, Vec<String>) {
    earth_floor(&mut f);
    let fy = 0;
    let b = Block::new((-2, -4), (2, 4), fy, fy + 6);
    let feet = b.feet();
    f.shell(&b);
    roof(&mut f, &b);
    // The way in is the middle of the front, and the rest of the front is open as well,
    // down to the floor, under the plate that carries the roof.
    f.front_door(&b, -1, 3, 5);
    let floor = f.floor_voxel(IVec3::new(b.hi.x, fy, 0));
    for z in b.lo.y + 1..b.hi.y {
        f.hollow(IVec3::new(b.hi.x, feet, z), IVec3::new(b.hi.x, b.plate - 1, z));
        f.put(IVec3::new(b.hi.x, fy, z), floor);
    }
    // The manger along the back wall, hay heaped in it, and a rail over it.
    let wood = f.pal.wood;
    for z in b.lo.y + 1..b.hi.y {
        f.put(IVec3::new(b.lo.x + 1, feet, z), wood);
        f.put(IVec3::new(b.lo.x + 1, feet + 1, z), mat::HAY);
    }
    let (x, y) = ((b.lo.x + 2) * 4, (feet + 2) * 4 + 2);
    f.bx(IVec3::new(x, y, (b.lo.y + 1) * 4), IVec3::new(x + 1, y + 1, b.hi.y * 4), wood);
    // Straw trodden into the floor, in wisps.
    for z in b.lo.y + 1..b.hi.y {
        for xc in b.lo.x + 2..b.hi.x {
            let q = IVec3::new(xc, 0, z);
            if f.grain(q, 0x57a3) < 0.45 {
                let (cx, cz) = (xc * 4 + (f.grain(q, 0x57a4) * 2.0) as i32, z * 4 + (f.grain(q, 0x57a5) * 2.0) as i32);
                f.bx(IVec3::new(cx, feet * 4, cz), IVec3::new(cx + 2, feet * 4 + 1, cz + 2), mat::HAY);
            }
        }
    }
    f.finish(Area { lo: b.lo, hi: b.hi }, fy)
}

/// A mason's tools hung on a rail along a wall, as [`tools`] hangs a farm's: a pair of
/// chisels, a mallet, a square and a straightedge in turn, on a rail `len` columns long
/// from column `from` at layer `y`, the wall's inner face at cell `face` across it.
fn mason_tools(f: &mut Frame, along_x: bool, face: i32, inward: i32, from: i32, len: i32, y: i32) {
    let (wood, iron, board) = (f.pal.wood, mat::IRON, f.pal.plank);
    let yc = y * 4 + 3;
    let mut put = |a0: i32, a1: i32, y0: i32, y1: i32, d: i32, v| {
        let (c0, c1) = if inward > 0 { (face + d, face + d + 1) } else { (face - d - 1, face - d) };
        if along_x {
            f.bx(IVec3::new(a0, y0, c0), IVec3::new(a1, y1, c1), v);
        } else {
            f.bx(IVec3::new(c0, y0, a0), IVec3::new(c1, y1, a1), v);
        }
    };
    put(from * 4, (from + len) * 4, yc, yc + 1, 0, wood);
    for k in 0..len {
        let a = from * 4 + 1 + k * 4;
        match k % 4 {
            // Two chisels, their blades down.
            0 => {
                for d in [0, 2] {
                    put(a + d, a + d + 1, yc - 3, yc + 1, 1, wood);
                    put(a + d, a + d + 1, yc - 6, yc - 3, 1, iron);
                }
            }
            // A mallet, its round head down.
            1 => {
                put(a + 1, a + 2, yc - 3, yc + 1, 1, wood);
                put(a, a + 3, yc - 6, yc - 3, 1, wood);
                put(a, a + 3, yc - 6, yc - 3, 2, wood);
            }
            // A square of iron, hung by its corner.
            2 => {
                put(a, a + 1, yc - 6, yc + 1, 1, iron);
                put(a, a + 4, yc - 6, yc - 5, 1, iron);
            }
            // A straightedge.
            _ => put(a + 1, a + 2, yc - 10, yc + 2, 1, board),
        }
    }
}

/// A mason's shed by a quarry: two banker stones along the back wall, waist high, each
/// with a block on it squared halfway and the rough of it still on top, a chisel and a
/// mallet laid by and the chips swept off it into the corner; a piece half carved
/// standing in the front corner, the grindstone and the tools on the wall. Its floor is
/// flagged, and its door wide for a block on a barrow.
fn mason(mut f: Frame) -> (super::Built, Vec<String>) {
    f.look.floor = FloorKind::Flags;
    let fy = 0;
    let half = 4;
    let b = Block::new((-5, -half), (4, half), fy, fy + 7);
    let feet = b.feet();
    f.shell(&b);
    roof(&mut f, &b);
    let dz = -1;
    f.front_door(&b, dz, 3, 5);
    f.door_leaf(&b, dz, 3, 5);
    f.front_steps(&b, dz, 3, 1);
    let (stone, dark, wood, iron) = (f.pal.stone, f.pal.stone_dark, f.pal.wood, mat::IRON);
    // The bankers, two columns long each against the back wall, and what is on them.
    for z0 in [-half + 1, half - 2] {
        for z in z0..z0 + 2 {
            f.put(IVec3::new(-4, feet, z), dark);
            f.put(IVec3::new(-4, feet + 1, z), stone);
        }
        let (x, top, z) = (-4 * 4, (feet + 2) * 4, z0 * 4);
        // The block, squared on its faces, the rough of it still standing on top.
        f.bx(IVec3::new(x, top, z), IVec3::new(x + 3, top + 3, z + 4), stone);
        f.bx(IVec3::new(x, top + 3, z + 1), IVec3::new(x + 2, top + 4, z + 4), dark);
        // A chisel, its blade toward the block, and a mallet by it.
        f.bx(IVec3::new(x + 1, top, z + 5), IVec3::new(x + 3, top + 1, z + 6), iron);
        f.bx(IVec3::new(x + 3, top, z + 5), IVec3::new(x + 4, top + 1, z + 6), wood);
        f.bx(IVec3::new(x, top, z + 6), IVec3::new(x + 2, top + 2, z + 8), wood);
        f.bx(IVec3::new(x + 2, top, z + 7), IVec3::new(x + 4, top + 1, z + 8), wood);
        f.uses.push((Use::Table, IVec3::new(-3, fy, z0)));
    }
    // The chips swept off the bankers into the corner between them.
    for (k, (dx, dz)) in [(0, 0), (1, 1), (0, 2), (2, 0), (1, 3)].into_iter().enumerate() {
        let q = IVec3::new(-4 * 4 + dx, feet * 4, -4 + dz);
        let v = if k % 2 == 0 { mat::GRAVEL } else { dark };
        f.bx(q, q + IVec3::new(1, 1 + (k % 2) as i32, 1), v);
    }
    // A piece half carved in the front corner: a rough block, a moulding cut along one
    // side of its top and the rest still rough.
    let corner = IVec3::new(3, feet, half - 1);
    f.put(corner, dark);
    let (cx, cy, cz) = (corner.x * 4, (feet + 1) * 4, corner.z * 4);
    f.bx(IVec3::new(cx, cy, cz), IVec3::new(cx + 4, cy + 1, cz + 4), stone);
    f.bx(IVec3::new(cx + 1, cy + 1, cz), IVec3::new(cx + 3, cy + 3, cz + 2), stone);
    f.bx(IVec3::new(cx, cy + 1, cz + 2), IVec3::new(cx + 4, cy + 3, cz + 4), dark);
    // The grindstone, and the tools on the +Z wall.
    f.fit(Fitting::Grindstone, IVec3::new(-1, feet, -half + 1), IVec3::NEG_X, false);
    mason_tools(&mut f, true, half * 4, -1, -1, 4, feet + 3);
    f.window(IVec2::new(0, b.lo.y), feet + 3, 2, 1, IVec3::NEG_Z);
    f.window(IVec2::new(-2, b.hi.y), feet + 3, 2, 1, IVec3::Z);
    f.finish(Area { lo: b.lo, hi: b.hi }, fy)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::settlements::kit;

    fn every(seeds: u32) -> Vec<BarnSpec> {
        let mut out = Vec::new();
        for kind in BarnKind::ALL {
            for region in Region::ALL {
                for seed in 0..seeds {
                    let h = |k: u32| crate::noise::unit(crate::noise::hash2(seed ^ 0xba4, kind as i32 * 8 + region as i32, k as i32));
                    out.push(BarnSpec { kind, region, age: h(1), wealth: h(2), seed: seed * 7919 + 17 });
                }
            }
        }
        out
    }

    fn palette(region: Region, seed: u32) -> Palette {
        let biome = region.biome();
        Palette::local(biome, mat::bedrock(biome).0).varied(seed)
    }

    /// Every barn of every kind and region is walked into through its door, to where its
    /// things are used and to every stall, nothing in it is built into anything else, and
    /// it keeps to the room [`LARGEST`] allows it.
    #[test]
    fn every_barn_is_walked_into_and_nothing_is_built_into_anything() {
        let mut wrong = Vec::new();
        for spec in every(6) {
            let (barn, faults) = make(&spec, &palette(spec.region, spec.seed));
            let b = &barn.building;
            let name = format!("{} of {} (seed {}, wealth {:.2})", spec.kind.word(), spec.region.word(), spec.seed, spec.wealth);
            let w = kit::walk(b);
            wrong.extend(w.problems.iter().map(|p| format!("{name}: {p}")));
            wrong.extend(faults.iter().map(|p| format!("{name}: {p}")));
            let floor: HashSet<IVec3> = w.floor.iter().copied().collect();
            if floor.len() < 12 {
                wrong.push(format!("{name}: only {} voxels of floor walked on", floor.len()));
            }
            for (u, q) in &barn.uses {
                if !floor.contains(q) {
                    wrong.push(format!("{name}: nobody gets to the {u:?} at {q}"));
                }
            }
            for (q, _) in &barn.stalls {
                if !floor.contains(&(*q - IVec3::Y)) {
                    wrong.push(format!("{name}: nobody gets into the stall at {q}"));
                }
            }
            let size = b.footprint.hi - b.footprint.lo + IVec2::ONE;
            if size.x > LARGEST.0 || size.y > LARGEST.1 {
                wrong.push(format!("{name}: {size} across, over {LARGEST:?}"));
            }
        }
        assert!(wrong.is_empty(), "{} things wrong:\n{}", wrong.len(), wrong.join("\n"));
    }

    #[test]
    fn a_byre_has_its_stalls_and_every_barn_its_hay_or_tools() {
        for spec in every(3) {
            let barn = build(&spec, &palette(spec.region, spec.seed));
            let has = |v| barn.building.voxels.iter().any(|&(_, w)| w == v);
            match spec.kind {
                BarnKind::Byre => assert!(barn.stalls.len() >= 2, "{spec:?}"),
                BarnKind::Hay => assert!(has(mat::HAY), "{spec:?}"),
                BarnKind::Shed => assert!(barn.building.fine.iter().any(|b| b.voxel == mat::IRON), "{spec:?}: no tools"),
                BarnKind::Mason => {
                    assert!(barn.building.fine.iter().any(|b| b.voxel == mat::IRON), "{spec:?}: no chisels");
                    assert_eq!(barn.uses.iter().filter(|u| u.0 == Use::Table).count(), 2, "{spec:?}: not two bankers");
                }
                BarnKind::Shelter => {
                    assert!(has(mat::HAY), "{spec:?}: no hay in the manger");
                    // Open all along its front: no wall between the corner posts at a
                    // person's height.
                    let w = barn.building.walls;
                    let shut = barn.building.voxels.iter().filter(|&&(q, v)| q.x == w.hi.x && q.z > w.lo.y && q.z < w.hi.y && (1..=4).contains(&q.y) && v != voxel::AIR).count();
                    assert_eq!(shut, 0, "{spec:?}: its front is not open");
                }
            }
            // The same spec builds the same barn.
            assert!(build(&spec, &palette(spec.region, spec.seed)).building == barn.building);
        }
    }

    #[test]
    fn barns_are_built_in_their_regions_way() {
        let look = |r: Region| build(&BarnSpec { kind: BarnKind::Hay, region: r, age: 0.5, wealth: 0.5, seed: 3 }, &palette(r, 3)).building.style;
        let (north, desert) = (look(Region::Northern), look(Region::Desert));
        assert_ne!(north.walls, desert.walls);
        assert_ne!(north.roof, desert.roof);
    }
}
