//! What every settlement building is put together with: the region's look from the
//! house kit's rules, and a small builder of walls, floors, roofs, stairs, doorways and
//! windows that places fittings and props and says when one lands where it should not.
//!
//! The kit's own builder makes homes of one storey under a roof it lowers for a wide
//! room; these buildings have two storeys, open fronts and towers, so they are built here
//! with the kit's materials and its rules for each region, but their own plans.

use std::collections::{HashMap, HashSet};

use glam::{IVec2, IVec3};
use voxel::{
    AIR, Voxel,
    mesh::{CELLS, PlantBox},
};

use super::{Built, Guild, SpecialSpec, Use, Wheel};
use crate::{
    features::{
        fittings::{self, Fitting},
        landmarks::Palette,
        props::{self, Prop},
        shape::{Rng, rotate},
    },
    materials as mat,
    noise::{hash3, unit},
    settlements::{
        Region,
        kit::{
            self, Anchor, AnchorKind, Area, Building, Cover, Door, Form, Hearth, Inside, Opening, RoofShape, Spot, Step, Style, WallKind,
            rules::{self, FloorKind, Under},
        },
    },
};

/// How far footings go down into the ground.
pub(super) const FOOT: i32 = 4;
/// From one floor to the next: the floor's boards, then five layers a person stands in.
pub(super) const STOREY: i32 = 6;

/// How a region builds a settlement building: what the kit's rules choose for one of its
/// timber or stone houses, with what makes no sense for a public building left out.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct Look {
    pub walls: WallKind,
    pub cover: Cover,
    pub shape: RoofShape,
    pub pitch: f32,
    pub overhang: i32,
    /// Courses of stone at the foot of the walls.
    pub base: i32,
    pub floor: FloorKind,
    /// How far the floor stands above the ground, and on what.
    pub raised: i32,
    pub under: Under,
    pub whitewash: bool,
    pub parapet: i32,
    pub roof_stones: bool,
}

impl Look {
    pub fn of(region: Region, step: Step, wealth: f32, seed: u32) -> Look {
        let r = rules::rules(region).step(step);
        let mut rng = Rng::new(seed ^ 0x100c_5eed);
        let walls = *rules::choose(&r.walls, wealth, rng.unit()).unwrap_or(&WallKind::Plank);
        let mut cover = *rules::choose(&r.roof, wealth, rng.unit()).unwrap_or(&Cover::Thatch);
        let mut shape = *rules::choose(&r.roof_shape, wealth, rng.unit()).unwrap_or(&RoofShape::Gable);
        // A round roof is for a round building; these are square.
        if shape == RoofShape::Cone {
            shape = RoofShape::Hip;
        }
        // A flat roof is beaten earth, whatever the region thatches its pitched ones with.
        if shape == RoofShape::Flat {
            cover = Cover::Mud;
        }
        let pitch = r.pitch.at(rng.unit()).clamp(0.7, 1.4);
        let overhang = r.overhang.whole(rng.unit()).clamp(0, 4);
        let base = r.base.whole(rng.unit()).clamp(0, 4);
        let floor = *rules::choose(&r.floor, wealth, rng.unit()).unwrap_or(&FloorKind::Boards);
        let raised = r.raised.whole(rng.unit()).max(0);
        let under = *rules::choose(&r.under, wealth, rng.unit()).unwrap_or(&Under::Stilts);
        let whitewash = rng.unit() < r.whitewash.at(wealth);
        let parapet = r.parapet.whole(rng.unit()).clamp(1, 2);
        let roof_stones = rng.unit() < r.roof_stones.at(wealth);
        Look { walls, cover, shape, pitch, overhang, base, floor, raised, under, whitewash, parapet, roof_stones }
    }

    /// Whether the walls are of earth: the desert's mud brick, cob.
    pub fn earthen(&self) -> bool {
        matches!(self.walls, WallKind::Mudbrick | WallKind::Cob)
    }
}

/// A rectangle of walls a building stands in: `lo` and `hi` are the columns of its outer
/// walls (x, and z in `y`), inclusive; `floor` the layer of its floor's voxels and `plate`
/// the top course of its walls.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct Block {
    pub lo: IVec2,
    pub hi: IVec2,
    pub floor: i32,
    pub plate: i32,
}

impl Block {
    pub fn new(lo: (i32, i32), hi: (i32, i32), floor: i32, plate: i32) -> Block {
        Block { lo: IVec2::new(lo.0, lo.1), hi: IVec2::new(hi.0, hi.1), floor, plate }
    }

    pub fn has(&self, c: IVec2) -> bool {
        c.x >= self.lo.x && c.y >= self.lo.y && c.x <= self.hi.x && c.y <= self.hi.y
    }

    /// Whether a column is inside the walls, not in them.
    pub fn within(&self, c: IVec2) -> bool {
        c.x > self.lo.x && c.y > self.lo.y && c.x < self.hi.x && c.y < self.hi.y
    }

    pub fn ring(&self) -> Vec<IVec2> {
        self.columns().into_iter().filter(|&c| !self.within(c)).collect()
    }

    pub fn inner(&self) -> Vec<IVec2> {
        self.columns().into_iter().filter(|&c| self.within(c)).collect()
    }

    pub fn columns(&self) -> Vec<IVec2> {
        (self.lo.y..=self.hi.y).flat_map(|z| (self.lo.x..=self.hi.x).map(move |x| IVec2::new(x, z))).collect()
    }

    pub fn corner(&self, c: IVec2) -> bool {
        (c.x == self.lo.x || c.x == self.hi.x) && (c.y == self.lo.y || c.y == self.hi.y)
    }

    /// The feet of a person standing on its floor.
    pub fn feet(&self) -> i32 {
        self.floor + 1
    }
}

/// A voxel at column `c` and height `y`.
pub(super) fn v3(c: IVec2, y: i32) -> IVec3 {
    IVec3::new(c.x, y, c.y)
}

/// The quarter turns that face a fitting (built facing +Z) toward `facing`.
pub(super) fn turns_to(facing: IVec3) -> u32 {
    match (facing.x.signum(), facing.z.signum()) {
        (0, 1) => 0,
        (-1, 0) => 1,
        (0, -1) => 2,
        _ => 3,
    }
}

/// A fitting as placed: where its model's column 0 went and how it was turned, to find
/// its parts again.
pub(super) struct Placed {
    pub at: IVec3,
    pub turns: u32,
}

impl Placed {
    /// Voxel `p` of the fitting's model, where it now is.
    pub fn at(&self, p: IVec3) -> IVec3 {
        rotate(p, self.turns) + self.at
    }
}

/// How a roof is laid over a block.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct Roof {
    pub shape: RoofShape,
    /// The ridge runs along z (gable ends at the low and high z walls), or else along x.
    pub along_z: bool,
    pub overhang: i32,
    pub pitch: f32,
    pub cover: Cover,
}

/// A settlement building being put together, in model space with its door to +X.
pub(super) struct Frame<'a> {
    pub spec: SpecialSpec,
    pub pal: &'a Palette,
    pub look: Look,
    /// Things left to fall apart: an old building of a poor place.
    pub neglect: bool,
    vox: HashMap<IVec3, Voxel>,
    pub fine: Vec<PlantBox>,
    /// The lowest layer of roof over each column, and the top of it.
    roof: HashMap<IVec2, (i32, i32)>,
    /// Every block of walls, for clearing their rooms.
    blocks: Vec<Block>,
    /// The voxels fittings and props take, to keep the next from landing in them.
    taken: HashSet<IVec3>,
    pub clear: Vec<(IVec3, IVec3)>,
    pub anchors: Vec<Anchor>,
    pub beds: Vec<Spot>,
    pub guest_beds: Vec<Spot>,
    pub furniture: Vec<Spot>,
    pub windows: Vec<Opening>,
    pub hearth: Option<Hearth>,
    pub door: Option<Door>,
    pub uses: Vec<(Use, IVec3)>,
    pub fire: Option<IVec3>,
    pub wheel: Option<Wheel>,
    pub guilds: Vec<Guild>,
    pub sign: Option<IVec3>,
    pub smoke: Option<IVec3>,
    pub faults: Vec<String>,
}

impl<'a> Frame<'a> {
    pub fn new(spec: &SpecialSpec, pal: &'a Palette) -> Self {
        // Public buildings are timber or stone, never huts; the richer and those that
        // hold a fire are stone.
        let step = match spec.which {
            crate::features::designs::Special::MarketHall | crate::features::designs::Special::GuildHouse => Step::Stone,
            crate::features::designs::Special::Smithy | crate::features::designs::Special::Tavern if spec.wealth > 0.75 => Step::Stone,
            _ => Step::Timber,
        };
        Frame {
            spec: *spec,
            pal,
            look: Look::of(spec.region, step, spec.wealth, spec.seed),
            neglect: spec.age > 0.8 && spec.wealth < 0.3,
            vox: HashMap::new(),
            fine: Vec::new(),
            roof: HashMap::new(),
            blocks: Vec::new(),
            taken: HashSet::new(),
            clear: Vec::new(),
            anchors: Vec::new(),
            beds: Vec::new(),
            guest_beds: Vec::new(),
            furniture: Vec::new(),
            windows: Vec::new(),
            hearth: None,
            door: None,
            uses: Vec::new(),
            fire: None,
            wheel: None,
            guilds: Vec::new(),
            sign: None,
            smoke: None,
            faults: Vec::new(),
        }
    }

    pub fn region(&self) -> Region {
        self.spec.region
    }

    // Noise and voxels.

    pub fn patch(&self, q: IVec3, size: i32, salt: u32) -> f32 {
        unit(hash3(self.spec.seed ^ salt, q.x.div_euclid(size), q.y.div_euclid(size), q.z.div_euclid(size)))
    }

    pub fn grain(&self, q: IVec3, salt: u32) -> f32 {
        unit(hash3(self.spec.seed ^ salt, q.x, q.y, q.z))
    }

    pub fn put(&mut self, q: IVec3, v: Voxel) {
        self.vox.insert(q, v);
    }

    pub fn get(&self, q: IVec3) -> Option<Voxel> {
        self.vox.get(&q).copied()
    }

    pub fn solid(&self, q: IVec3) -> bool {
        self.get(q).is_some_and(|v| v != AIR)
    }

    /// Fills an inclusive box.
    pub fn fill(&mut self, lo: IVec3, hi: IVec3, v: Voxel) {
        for y in lo.y.min(hi.y)..=lo.y.max(hi.y) {
            for z in lo.z.min(hi.z)..=lo.z.max(hi.z) {
                for x in lo.x.min(hi.x)..=lo.x.max(hi.x) {
                    self.put(IVec3::new(x, y, z), v);
                }
            }
        }
    }

    /// Empties an inclusive box, and keeps it clear of props.
    pub fn hollow(&mut self, lo: IVec3, hi: IVec3) {
        self.fill(lo, hi, AIR);
    }

    /// A fine part, fixed, from cell `lo` to `hi` (exclusive) of model space.
    pub fn bx(&mut self, lo: IVec3, hi: IVec3, v: Voxel) {
        let (a, b) = (lo.min(hi), lo.max(hi));
        if (b - a).min_element() > 0 {
            self.fine.push(PlantBox { lo: a.to_array(), hi: b.to_array(), voxel: v, sway_lo: false, sway_hi: false, thin: 0, season: 0, flap: 0 });
        }
    }

    // Materials.

    pub fn stonework(&self, q: IVec3) -> Voxel {
        let r = self.grain(q, 0x570e);
        if r < self.pal.moss * (0.3 + 0.5 * self.spec.age) {
            mat::MOSSY_STONE
        } else if r < 0.35 {
            self.pal.stone_dark
        } else {
            self.pal.stone
        }
    }

    pub fn floor_voxel(&self, q: IVec3) -> Voxel {
        match self.look.floor {
            FloorKind::Earth => mat::PATH_DIRT,
            FloorKind::Flags if self.patch(q, 2, 0xf1a6) < 0.5 => self.pal.stone,
            FloorKind::Flags => self.pal.stone_dark,
            FloorKind::Boards => self.pal.plank,
        }
    }

    /// The material of a wall of `b` at voxel `q`, of the region's wall kind.
    pub fn wall_voxel(&self, q: IVec3, b: &Block) -> Voxel {
        let pal = self.pal;
        let course = q.y - b.floor;
        if q.y < 0 || course <= 0 {
            return pal.stone_dark;
        }
        let c = IVec2::new(q.x, q.z);
        let gable = q.y > b.plate;
        if course <= self.look.base && !gable {
            return if course == 1 { pal.stone_dark } else { self.stonework(q) };
        }
        let corner = b.corner(c);
        let along = if c.x == b.lo.x || c.x == b.hi.x { (c.y - b.lo.y).min(b.hi.y - c.y) } else { (c.x - b.lo.x).min(b.hi.x - c.x) };
        let post = |every: i32| corner || along % every == 0;
        let flaked = self.patch(q, 2, 0xf1a4) < self.spec.age * 0.45;
        let limewash = |bare: Voxel| if self.look.whitewash && !flaked { mat::WHITEWASH } else { bare };
        let top = q.y == b.plate;
        // The beam a floor above is carried on shows in a framed wall.
        let joist = (course - 1) % STOREY == STOREY - 1;
        match self.look.walls {
            WallKind::Cob => limewash(mat::COB),
            WallKind::Mudbrick => {
                if self.look.whitewash && !flaked {
                    mat::WHITEWASH
                } else if self.patch(q, 2, 0xb41c) < self.spec.age * 0.4 {
                    mat::MUD_BRICK
                } else {
                    mat::MUD_PLASTER
                }
            }
            WallKind::Drystone => self.stonework(q),
            WallKind::Stone => {
                // Quoins: long and short stones turning each corner, a course at a time.
                if corner || (along == 1 && course % 2 == 0) {
                    pal.stone_dark
                } else if pal.moss > 0.2 && self.patch(q, 2, 0x3055) < self.spec.age * 0.25 {
                    mat::MOSSY_STONE
                } else {
                    pal.stone
                }
            }
            WallKind::Log => {
                // Log ends cross at the corners, a darker grain.
                if corner && course % 2 == 0 { mat::PLANK_DARK } else { pal.wood }
            }
            WallKind::Frame => {
                if gable {
                    if post(4) { pal.wood } else { pal.plank }
                } else if post(if self.spec.wealth > 0.6 { 2 } else { 3 }) || top || joist || course == 1 || course == 3 {
                    pal.wood
                } else {
                    limewash(mat::DAUB)
                }
            }
            WallKind::Wattle => {
                if post(3) || top {
                    pal.wood
                } else if self.patch(q, 2, 0x3a77) < self.spec.age * 0.35 {
                    mat::WATTLE
                } else {
                    limewash(mat::DAUB)
                }
            }
            WallKind::Plank => {
                if post(4) || top || course == 1 || joist { pal.wood } else { pal.plank }
            }
            WallKind::Reed => {
                if post(3) || top || course == 1 || joist { pal.wood } else { mat::REED_THATCH }
            }
        }
    }

    /// The material of an inside wall between rooms.
    pub fn partition_voxel(&self) -> Voxel {
        if self.look.earthen() { mat::MUD_PLASTER } else { self.pal.plank }
    }

    pub fn cover_voxel(&self, q: IVec3, cover: Cover) -> Voxel {
        let (new, old) = match cover {
            Cover::Thatch => (mat::THATCH, mat::THATCH_OLD),
            Cover::Reed => (mat::REED_THATCH, mat::THATCH_OLD),
            Cover::Palm => (mat::PALM_THATCH, mat::THATCH_OLD),
            Cover::Turf => (mat::ROOF_TURF, mat::ROOF_TURF),
            Cover::Slate => (mat::SLATE, mat::SLATE),
            Cover::Tile => (mat::ROOF_TILE, mat::ROOF_TILE_OLD),
            Cover::Shingle => (mat::SHINGLE, mat::SHINGLE_OLD),
            Cover::Mud => (mat::ROOF_MUD, mat::ROOF_MUD),
        };
        let worn = self.patch(q, 3, 0x0d1d) < self.spec.age * 0.8;
        let mended = self.spec.age > 0.65 && self.patch(q + IVec3::ONE, 4, 0x9a7c) < 0.15;
        if worn && !mended { old } else { new }
    }

    // Walls, floors and roofs.

    /// The walls of a block from their footings (or its raised floor) to the plate, its
    /// floor, and what holds a raised floor up.
    pub fn shell(&mut self, b: &Block) {
        self.blocks.push(*b);
        let start = if b.floor > 0 { b.floor + 1 } else { -FOOT };
        for c in b.ring() {
            for y in start..=b.plate {
                let at = v3(c, y);
                let v = self.wall_voxel(at, b);
                self.put(at, v);
            }
        }
        self.floor(b);
    }

    /// Counts a block whose walls were raised by hand as rooms to clear.
    pub fn register(&mut self, b: &Block) {
        self.blocks.push(*b);
    }

    /// A block's floor, and under a raised one its stilts, plinth or roots.
    pub fn floor(&mut self, b: &Block) {
        let fy = b.floor;
        for c in b.inner() {
            let at = v3(c, fy);
            let v = self.floor_voxel(at);
            self.put(at, v);
        }
        if fy <= 0 {
            return;
        }
        let edge = if self.look.under == Under::Plinth { self.pal.stone_dark } else { self.pal.plank };
        for c in b.ring() {
            self.put(v3(c, fy), edge);
        }
        self.under(b.lo, b.hi, fy);
    }

    /// What holds up a floor at layer `fy` over columns `lo` to `hi`: a plinth of stone,
    /// or posts at the corners and every few columns, on stone pads.
    pub fn under(&mut self, lo: IVec2, hi: IVec2, fy: i32) {
        let plinth = self.look.under == Under::Plinth;
        let roots = self.look.under == Under::Roots;
        for z in lo.y..=hi.y {
            for x in lo.x..=hi.x {
                let c = IVec2::new(x, z);
                let edge_x = x == lo.x || x == hi.x;
                let edge_z = z == lo.y || z == hi.y;
                if plinth {
                    for y in -2..fy {
                        let at = v3(c, y);
                        let v = if edge_x || edge_z { self.stonework(at) } else { self.pal.stone_dark };
                        self.put(at, v);
                    }
                    continue;
                }
                let every = if roots { 4 } else { 3 };
                let along_x = (x - lo.x) % every == 0 || x == hi.x;
                let along_z = (z - lo.y) % every == 0 || z == hi.y;
                if !(along_x && along_z) {
                    continue;
                }
                for y in -FOOT..fy {
                    let v = if y < 0 { self.pal.stone_dark } else { self.pal.wood };
                    self.put(v3(c, y), v);
                }
                // Old roots spread out from the foot of each post.
                if roots {
                    for d in [IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y] {
                        let n = c + d;
                        if !self.solid(v3(n, 0)) {
                            self.put(v3(n, 0), self.pal.wood);
                        }
                    }
                }
            }
        }
    }

    /// A floor of boards at layer `y` over the inside of `b`: the floor of a storey above.
    pub fn slab(&mut self, b: &Block, y: i32) {
        for c in b.inner() {
            self.put(v3(c, y), self.pal.plank);
        }
    }

    /// Lays a roof over `b`. Over the room every layer of it is at least a storey above
    /// the floor; its eaves stop short wherever they would hang lower than a person
    /// standing on the floor, so none closes the way in.
    pub fn roof(&mut self, b: &Block, r: &Roof) {
        let oh = if r.shape == RoofShape::Flat { 0 } else { r.overhang };
        let (lo, hi) = (b.lo - IVec2::splat(oh), b.hi + IVec2::splat(oh));
        let mut low: HashMap<IVec2, i32> = HashMap::new();
        let centre = (b.lo + b.hi).as_vec2() * 0.5;
        let radius = (b.hi - b.lo).as_vec2().max_element() * 0.5 + oh as f32;
        for z in lo.y..=hi.y {
            for x in lo.x..=hi.x {
                let c = IVec2::new(x, z);
                let (dx, dz) = ((x - lo.x).min(hi.x - x), (z - lo.y).min(hi.y - z));
                let d = match r.shape {
                    RoofShape::Gable if r.along_z => dx as f32,
                    RoofShape::Gable => dz as f32,
                    RoofShape::Hip => dx.min(dz) as f32,
                    RoofShape::Flat => 0.0,
                    RoofShape::Cone => {
                        let d = radius + 0.5 - (c.as_vec2() - centre).length();
                        if d < 0.0 {
                            continue;
                        }
                        d.floor()
                    }
                };
                let y = b.plate + 1 + (r.pitch * (d - oh as f32)).floor() as i32;
                // Over the walls never lower than their top; the eaves droop past them.
                let over = d >= oh as f32;
                if y < b.floor + STOREY && !over {
                    continue;
                }
                low.insert(c, if over { y.max(b.plate + 1) } else { y });
            }
        }
        // Each column of roof reaches up to meet the next one in, so no light shows
        // between them however steep.
        let dirs = [IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y];
        let mut cols: Vec<(IVec2, i32)> = low.iter().map(|(&c, &y)| (c, y)).collect();
        cols.sort_unstable_by_key(|(c, _)| (c.x, c.y));
        for (c, y) in cols {
            let top = dirs.iter().filter_map(|d| low.get(&(c + *d))).map(|n| n - 1).fold(y, i32::max);
            for k in y..=top {
                let at = v3(c, k);
                // A turf roof is grass over boards.
                let v = if r.cover == Cover::Turf && k < top { self.pal.plank } else { self.cover_voxel(at, r.cover) };
                self.put(at, v);
            }
            // Rows of stones hold a shingle roof down against the wind.
            if self.look.roof_stones && r.shape != RoofShape::Flat && (c.x + c.y).rem_euclid(3) == 0 && (if r.along_z { c.x } else { c.y }).rem_euclid(4) == 1 {
                self.put(v3(c, top + 1), self.pal.stone_dark);
            }
            self.roof.insert(c, (y, top));
            // Gable ends: the wall carried up under the roof.
            if b.has(c) && !b.within(c) && y - 1 > b.plate {
                for k in b.plate + 1..y {
                    let at = v3(c, k);
                    if self.get(at) != Some(AIR) {
                        let v = self.wall_voxel(at, b);
                        self.put(at, v);
                    }
                }
            }
        }
        if r.shape == RoofShape::Flat {
            // A parapet round a flat roof, its corners a course higher.
            for c in b.ring() {
                let up = self.look.parapet + if b.corner(c) { 1 } else { 0 };
                for k in 1..=up {
                    let at = v3(c, b.plate + 1 + k);
                    let v = self.wall_voxel(v3(c, b.plate), b);
                    self.put(at, v);
                }
            }
        }
    }

    /// The lowest layer of roof over a column, if it has one.
    pub fn roof_low(&self, c: IVec2) -> Option<i32> {
        self.roof.get(&c).map(|r| r.0)
    }

    /// The top of the roof over a column, if it has one.
    pub fn roof_top(&self, c: IVec2) -> Option<i32> {
        self.roof.get(&c).map(|r| r.1)
    }

    /// A doorway through the +X wall of `b`, `width` wide from column `z0` and `height`
    /// high, with its door leaf standing open inside against the wall; and the door.
    pub fn front_door(&mut self, b: &Block, z0: i32, width: i32, height: i32) {
        let x = b.hi.x;
        self.hollow(IVec3::new(x, b.feet(), z0), IVec3::new(x, b.floor + height, z0 + width - 1));
        let v = self.floor_voxel(IVec3::new(x, b.floor, z0));
        for z in z0..z0 + width {
            self.put(IVec3::new(x, b.floor, z), v);
        }
        self.door = Some(Door { floor: IVec3::new(x, b.floor, z0 + (width - 1) / 2), facing: IVec3::X, width, height });
        // The way in, kept clear of props: the doorway and the ground before it.
        self.clear.push((IVec3::new(x - 2, b.feet(), z0), IVec3::new(x + 3, b.floor + height, z0 + width - 1)));
        self.anchors.push(Anchor { kind: AnchorKind::Door, at: IVec3::new(x + 1, b.feet().min(0).max(0), z0 + width / 2), normal: IVec3::X });
    }

    /// A door leaf of planks standing open against the inside of the +X wall of `b`,
    /// beside a doorway from `z0`, `width` wide and `height` high.
    pub fn door_leaf(&mut self, b: &Block, z0: i32, width: i32, height: i32) {
        let x = b.hi.x * CELLS;
        let (y0, y1) = (b.feet() * CELLS, (b.feet() + height.min(5)) * CELLS - 1);
        let side = z0 + width;
        let board = if self.neglect { mat::PLANK_DARK } else { self.pal.plank };
        self.bx(IVec3::new(x - 1, y0, side * CELLS), IVec3::new(x, y1, (side + width.min(3)) * CELLS - 1), board);
        self.bx(IVec3::new(x - 2, y0 + 3, side * CELLS + 1), IVec3::new(x - 1, y0 + 4, (side + width.min(3)) * CELLS - 2), self.pal.wood);
        self.bx(IVec3::new(x - 2, y1 - 4, side * CELLS + 1), IVec3::new(x - 1, y1 - 3, (side + width.min(3)) * CELLS - 2), self.pal.wood);
    }

    /// A window through the wall at column `c` facing out along `normal`, `wide` columns
    /// along the wall from `c` and `high` layers from `y`, and what its region puts at a
    /// window: shutters, a lattice, a screen of reeds.
    pub fn window(&mut self, c: IVec2, y: i32, wide: i32, high: i32, normal: IVec3) {
        let along = IVec3::new(normal.z.abs(), 0, normal.x.abs());
        let lo = v3(c, y);
        let hi = lo + along * (wide - 1) + IVec3::Y * (high - 1);
        self.hollow(lo, hi);
        self.windows.push(Opening { lo, hi, normal });
        self.anchors.push(Anchor { kind: AnchorKind::Window, at: lo + normal, normal });
        // The outer face of the wall, in cells, and the opening's ends along it.
        let face = |q: IVec3| -> i32 { if normal.x + normal.z > 0 { (q.x * normal.x.abs() + q.z * normal.z.abs() + 1) * CELLS } else { (q.x * normal.x.abs() + q.z * normal.z.abs()) * CELLS } };
        let f = face(lo);
        let out = normal.x + normal.z;
        let (a0, a1) = ((lo.x * along.x + lo.z * along.z) * CELLS, (hi.x * along.x + hi.z * along.z + 1) * CELLS);
        let (y0, y1) = (y * CELLS, (y + high) * CELLS);
        let plane = |s: &mut Self, depth: (i32, i32), a: (i32, i32), ys: (i32, i32), v: Voxel| {
            let (d0, d1) = (depth.0.min(depth.1), depth.0.max(depth.1));
            if along.x == 1 {
                s.bx(IVec3::new(a.0, ys.0, d0), IVec3::new(a.1, ys.1, d1), v);
            } else {
                s.bx(IVec3::new(d0, ys.0, a.0), IVec3::new(d1, ys.1, a.1), v);
            }
        };
        match self.region() {
            // Lattice screens set in the opening, to let the air through and keep the sun out.
            Region::Desert => {
                let mid = f - out * 2;
                let wood = if self.spec.wealth > 0.5 { mat::PLANK_RED } else { self.pal.plank };
                let mut k = a0 + 1;
                while k < a1 - 1 {
                    plane(self, (mid, mid + out), (k, k + 1), (y0, y1), wood);
                    k += 3;
                }
                let mut k = y0 + 1;
                while k < y1 - 1 {
                    plane(self, (mid - out, mid), (a0, a1), (k, k + 1), wood);
                    k += 3;
                }
            }
            // Screens of woven reed, propped half open.
            Region::Wetland | Region::Rainforest => {
                plane(self, (f, f + out), (a0, a1), (y1 - 3, y1), mat::REED_THATCH);
            }
            // Shutters standing open either side, and a sill.
            _ => {
                let board = if self.neglect { mat::PLANK_DARK } else { self.pal.plank };
                let w = ((a1 - a0) / 2).max(3);
                plane(self, (f, f + out), (a0 - w, a0), (y0, y1), board);
                plane(self, (f, f + out), (a1, a1 + w), (y0, y1), board);
                plane(self, (f - out, f + out * 2), (a0, a1), (y0 - 1, y0), self.pal.wood);
            }
        }
    }

    /// An inside wall from `lo` to `hi`, inclusive.
    pub fn partition(&mut self, lo: IVec3, hi: IVec3) {
        let v = self.partition_voxel();
        self.fill(lo, hi, v);
    }

    /// A flight of stairs two columns wide, up one storey from the floor at layer `fy`.
    /// `start` is the column a person stands in to begin the climb, `dir` the way up and
    /// `side` the way to the flight's second column. It takes eight columns along `dir`:
    /// two on the floor and five treads under the opening in the floor above, which is
    /// over all seven, and the eighth is the floor above, which is left as it is.
    ///
    /// A person two columns wide and five tall rises a layer at each tread, and their
    /// head clears the floor above all the way up: the opening is over the column they
    /// rise from as well as the treads.
    pub fn stairs(&mut self, start: IVec2, dir: IVec2, side: IVec2, fy: i32) {
        let tread = self.pal.plank;
        let string = self.pal.wood;
        for t in 0..7 {
            let c = start + dir * t;
            for w in 0..2 {
                let col = c + side * w;
                for y in fy + 1..fy + t {
                    // The treads on a solid flight, the strings showing along the side.
                    let v = if y == fy + t - 1 { tread } else { string };
                    self.put(v3(col, y), v);
                }
                self.put(v3(col, fy + STOREY), AIR);
            }
        }
        // A rail along the open side of the opening above.
        let rail = self.pal.wood;
        let open = start + side * 2;
        for t in 0..7 {
            let c = open + dir * t;
            if self.solid(v3(c, fy + STOREY)) {
                let x = c.x * CELLS + if side.x != 0 { if side.x > 0 { 0 } else { CELLS - 1 } } else { 1 };
                let z = c.y * CELLS + if side.y != 0 { if side.y > 0 { 0 } else { CELLS - 1 } } else { 1 };
                let top = (fy + STOREY + 1) * CELLS;
                if t % 3 == 0 {
                    self.bx(IVec3::new(x, top, z), IVec3::new(x + 1, top + 4, z + 1), rail);
                }
                let (lo, hi) = if dir.x != 0 { (IVec3::new(c.x * CELLS, top + 4, z), IVec3::new((c.x + 1) * CELLS, top + 5, z + 1)) } else { (IVec3::new(x, top + 4, c.y * CELLS), IVec3::new(x + 1, top + 5, (c.y + 1) * CELLS)) };
                self.bx(lo, hi, rail);
            }
        }
    }

    /// Steps up to the door of a raised floor: a platform `deep` columns out before the
    /// doorway (from column `z0`, `width` wide), on posts, and a flight down to the ground
    /// a layer at a time, each tread a column.
    pub fn front_steps(&mut self, b: &Block, z0: i32, width: i32, deep: i32) {
        let fy = b.floor;
        if fy <= 0 {
            return;
        }
        let x0 = b.hi.x + 1;
        let (za, zb) = (z0 - 1, z0 + width);
        for x in x0..x0 + deep {
            for z in za..=zb {
                self.put(IVec3::new(x, fy, z), self.pal.plank);
            }
        }
        self.under(IVec2::new(x0, za), IVec2::new(x0 + deep - 1, zb), fy);
        for k in 1..=fy {
            let x = x0 + deep - 1 + k;
            for z in z0..z0 + width {
                for y in -1..=fy - k {
                    let v = if y == fy - k { self.pal.plank } else { self.pal.wood };
                    self.put(IVec3::new(x, y, z), v);
                }
                for y in fy - k + 1..fy - k + 6 {
                    self.put(IVec3::new(x, y, z), AIR);
                }
            }
        }
        // Rails along the sides of the platform.
        let rail = self.pal.wood;
        let top = (fy + 1) * CELLS;
        for z in [za, zb] {
            let zc = if z == za { za * CELLS } else { (zb + 1) * CELLS - 1 };
            self.bx(IVec3::new(x0 * CELLS, top + 4, zc), IVec3::new((x0 + deep) * CELLS, top + 5, zc + 1), rail);
            for x in x0..x0 + deep {
                self.bx(IVec3::new(x * CELLS + 1, top, zc), IVec3::new(x * CELLS + 2, top + 4, zc + 1), rail);
            }
        }
        for y in fy + 1..fy + 6 {
            for x in x0..x0 + deep {
                for z in za..=zb {
                    if self.get(IVec3::new(x, y, z)).is_none() {
                        self.put(IVec3::new(x, y, z), AIR);
                    }
                }
            }
        }
    }

    /// A fire that is always alight: a voxel of embers at `at` (so it glows at night,
    /// lights what is about it, and the game's smoke rises from it), logs burning on it
    /// where nothing else is laid on it already.
    pub fn fire(&mut self, at: IVec3, logs: bool) {
        self.put(at, mat::EMBER);
        self.fire = Some(at);
        if !logs {
            return;
        }
        let c = at * CELLS;
        let top = c.y + CELLS;
        self.bx(IVec3::new(c.x - 1, top, c.z + 1), IVec3::new(c.x + 5, top + 1, c.z + 2), mat::CHARCOAL);
        self.bx(IVec3::new(c.x + 1, top, c.z - 1), IVec3::new(c.x + 2, top + 1, c.z + 5), self.pal.wood);
        self.bx(IVec3::new(c.x + 1, top + 1, c.z + 1), IVec3::new(c.x + 3, top + 2, c.z + 3), mat::EMBER);
    }

    /// A sign hanging from a bracket on the +X wall of `b`, beside the door, at column
    /// `z`: a board painted with the colours given, on both faces.
    pub fn hanging_sign(&mut self, b: &Block, z: i32, y: i32, paint: (Voxel, Voxel)) {
        let face = (b.hi.x + 1) * CELLS;
        let (zc, yc) = (z * CELLS + 1, y * CELLS);
        let iron = mat::IRON;
        // The bracket: an arm out from the wall and a strut under it.
        self.bx(IVec3::new(face, yc + 8, zc), IVec3::new(face + 12, yc + 9, zc + 1), iron);
        self.bx(IVec3::new(face, yc + 5, zc), IVec3::new(face + 1, yc + 9, zc + 1), iron);
        self.bx(IVec3::new(face + 1, yc + 6, zc), IVec3::new(face + 4, yc + 7, zc + 1), iron);
        // Two short chains and the board.
        for x in [face + 4, face + 10] {
            self.bx(IVec3::new(x, yc + 6, zc), IVec3::new(x + 1, yc + 8, zc + 1), iron);
        }
        let board = if self.neglect { mat::PLANK_DARK } else { self.pal.plank };
        self.bx(IVec3::new(face + 3, yc, zc), IVec3::new(face + 12, yc + 6, zc + 1), board);
        // Painted on both faces: a field and a device on it.
        for (z0, z1) in [(zc - 1, zc), (zc + 1, zc + 2)] {
            self.bx(IVec3::new(face + 4, yc + 1, z0), IVec3::new(face + 11, yc + 5, z1), paint.0);
            self.bx(IVec3::new(face + 6, yc + 2, if z0 < zc { z0 - 1 } else { z1 }), IVec3::new(face + 9, yc + 4, if z0 < zc { z0 } else { z1 + 1 }), paint.1);
        }
        self.sign = Some(IVec3::new(b.hi.x + 2, y, z));
    }

    /// What a region sets beside a door: on the ground outside the +X wall of `b`, clear of
    /// the doorway from `z0`, `width` wide (and of any steps before it).
    pub fn porch(&mut self, b: &Block, z0: i32, width: i32) {
        // Clear of the posts or roots a raised floor stands on, and of those under any
        // platform before the door.
        let x = b.hi.x + if b.floor > 0 { 2 } else { 1 };
        let y = 0;
        let (left, right) = (z0 - 1 - 2, z0 + width + 2);
        let decayed = self.neglect;
        match self.region() {
            Region::Desert => {
                // An awning of cloth over the door on two poles, and water jars by it; the
                // doorway framed in colour on the whitewash.
                let colour = [mat::CLOTH_BLUE, mat::CLOTH_RED, mat::CLOTH_YELLOW][(self.spec.seed % 3) as usize];
                let frame = if self.spec.seed % 2 == 0 { mat::CLOTH_BLUE } else { mat::PLANK_RED };
                for z in z0 - 1..=z0 + width {
                    for yy in b.feet()..=b.floor + 6 {
                        let q = IVec3::new(b.hi.x, yy, z);
                        if self.solid(q) && (z == z0 - 1 || z == z0 + width || yy == b.floor + 6) {
                            self.put(q, frame);
                        }
                    }
                }
                let face = (b.hi.x + 1) * CELLS;
                let (za, zb) = ((z0 - 2) * CELLS, (z0 + width + 2) * CELLS);
                let (high, low) = ((b.floor + 7) * CELLS, (b.floor + 6) * CELLS + 1);
                let out = face + 3 * CELLS;
                self.fine.extend(crate::features::cloth::sheet([IVec3::new(face, high, za), IVec3::new(face, high, zb), IVec3::new(out, low, za), IVec3::new(out, low, zb)], colour));
                for z in [za, zb - 1] {
                    self.bx(IVec3::new(out - 1, 0, z), IVec3::new(out, low, z + 1), self.pal.wood);
                }
                self.fit(Fitting::WaterJar, IVec3::new(x, y, right + 1), IVec3::X, decayed);
            }
            Region::Temperate | Region::Upland => {
                // Along the wall, its back to it: turned so it reaches toward -Z from here.
                self.prop(Prop::Bench, IVec3::new(x, y, right + 3), 3, false);
                self.fit(Fitting::WaterButt, IVec3::new(x, y, left - 2), IVec3::X, decayed);
            }
            Region::Northern | Region::Alpine => {
                self.fit(Fitting::LogStack, IVec3::new(x, y, right + 1), IVec3::X, decayed);
            }
            Region::Wetland => {
                self.fit(Fitting::EelTrap, IVec3::new(x, y, right + 1), IVec3::X, decayed);
            }
            Region::Rainforest => {
                self.fit(Fitting::BasketStack, IVec3::new(x, y, right + 1), IVec3::X, decayed);
            }
            Region::OldGrowth => {
                self.fit(Fitting::LogStack, IVec3::new(x, y, right + 1), IVec3::X, decayed);
            }
        }
    }

    /// A chimney stack of stone over columns `lo` to `hi` (x and z, inclusive), from layer
    /// `from` to two layers above the roof over it, with its flue left open at `flue`
    /// all the way up from layer `flue_from`. The smoke comes out of the top.
    pub fn chimney(&mut self, lo: IVec2, hi: IVec2, from: i32, flue: IVec2, flue_from: i32) {
        let mut top = from + 2;
        for z in lo.y..=hi.y {
            for x in lo.x..=hi.x {
                if let Some(t) = self.roof_top(IVec2::new(x, z)) {
                    top = top.max(t + 2);
                }
            }
        }
        // Mud brick in the desert, the local stone elsewhere.
        let earthen = self.look.earthen() && self.region() == Region::Desert;
        for z in lo.y..=hi.y {
            for x in lo.x..=hi.x {
                for y in from..=top {
                    let at = IVec3::new(x, y, z);
                    let v = if earthen { mat::MUD_PLASTER } else if y == top { self.pal.stone_dark } else { self.stonework(at) };
                    self.put(at, v);
                }
            }
        }
        for y in flue_from..=top {
            self.put(v3(flue, y), AIR);
        }
        self.smoke = Some(v3(flue, top + 1));
    }

    // Fittings and props.

    /// Places a fitting (in its region's own form) with the near corner of the room it
    /// takes at `lo`, its lowest x and z, its foot on layer `lo.y`, facing `facing`.
    pub fn fit(&mut self, kind: Fitting, lo: IVec3, facing: IVec3, decayed: bool) -> Placed {
        let kind = kind.local(self.region());
        let turns = turns_to(facing);
        let s = kind.size();
        let (a, b) = (rotate(IVec3::ZERO, turns), rotate(IVec3::new(s.x - 1, 0, s.z - 1), turns));
        let at = lo - a.min(b);
        let seed = self.spec.seed ^ hash3(0xf177_1a6, lo.x, lo.y, lo.z);
        let fitted = fittings::build(kind, self.pal, seed, decayed).place(at, turns);
        self.take(&fitted.voxels, &fitted.fine, kind.name());
        Placed { at, turns }
    }

    /// Places a prop with its voxel 0 at `at`, turned by `turns`.
    pub fn prop(&mut self, prop: Prop, at: IVec3, turns: u32, decayed: bool) {
        let seed = self.spec.seed ^ hash3(0x960b_5, at.x, at.y, at.z);
        let p = props::build(prop, self.pal, at, turns, decayed, seed);
        self.take(&p.voxels, &p.fine, &format!("{prop:?}"));
    }

    /// Lays a fitting's or a prop's parts, noting any that land in what is built already
    /// or in another fitting.
    fn take(&mut self, voxels: &[(IVec3, Voxel)], fine: &[PlantBox], what: &str) {
        let own: HashSet<IVec3> = voxels.iter().map(|&(q, _)| q).collect();
        let mut cells: HashSet<IVec3> = own.clone();
        for b in fine {
            let (lo, hi) = (IVec3::from_array(b.lo).div_euclid(IVec3::splat(CELLS)), (IVec3::from_array(b.hi) - 1).div_euclid(IVec3::splat(CELLS)));
            for y in lo.y..=hi.y {
                for z in lo.z..=hi.z {
                    for x in lo.x..=hi.x {
                        cells.insert(IVec3::new(x, y, z));
                    }
                }
            }
        }
        let mut sorted: Vec<IVec3> = cells.iter().copied().collect();
        sorted.sort_unstable_by_key(|q| (q.y, q.x, q.z));
        if let Some(q) = sorted.iter().find(|q| self.solid(**q)) {
            self.faults.push(format!("the {what} is built into what stands at {q}"));
        } else if let Some(q) = sorted.iter().find(|q| self.taken.contains(q)) {
            self.faults.push(format!("the {what} is built into another fitting at {q}"));
        }
        for &(q, v) in voxels {
            self.put(q, v);
        }
        self.fine.extend_from_slice(fine);
        self.taken.extend(cells);
    }

    /// A bed for one, lying `along` a wall from `at` (the voxel by the wall at one end, at
    /// a person's feet) and reaching `facing` into the room: a straw pallet, or in warm
    /// country a rope bed.
    pub fn bed(&mut self, at: IVec3, along: IVec3, facing: IVec3, guest: bool) {
        let kind = if matches!(self.region(), Region::Desert | Region::Rainforest) { Fitting::RopeBed } else { Fitting::StrawPallet };
        // The pallet lies along its model x with its back to model z = 0: turned so its
        // back is to the wall and its length runs along it.
        let s = kind.size();
        let cells: Vec<IVec3> = (0..s.x).flat_map(|i| (0..s.z).map(move |j| at + along * i + facing * j)).collect();
        let lo = cells.iter().fold(cells[0], |a, &c| a.min(c));
        self.fit(kind, lo, facing, false);
        let spot = Spot { at, along, facing, len: 5, wide: 2 };
        if guest {
            self.guest_beds.push(spot);
        } else {
            self.beds.push(spot);
        }
    }

    /// A place for furniture along a wall, as the kit gives a home.
    pub fn furniture_spot(&mut self, at: IVec3, along: IVec3, facing: IVec3, len: i32, wide: i32) {
        self.furniture.push(Spot { at, along, facing, len, wide });
    }

    // Finishing.

    /// Greys timber and darkens boards with age, as the kit weathers a home.
    fn weather(&mut self) {
        if self.spec.age <= 0.0 {
            return;
        }
        let (wood, plank) = (self.pal.wood, self.pal.plank);
        let mut qs: Vec<IVec3> = self.vox.keys().copied().collect();
        qs.sort_unstable_by_key(|q| (q.y, q.x, q.z));
        for q in qs {
            let v = self.vox[&q];
            if v == wood && self.patch(q, 2, 0x3ead) < self.spec.age * 0.3 {
                self.put(q, mat::WOOD_DEAD);
            } else if v == plank && plank != mat::PLANK_DARK && self.patch(q, 2, 0x91a2) < self.spec.age * 0.35 {
                self.put(q, mat::PLANK_DARK);
            }
        }
    }

    /// Clears every room, and under every roof, of whatever the ground grew there: what
    /// nothing was built in is stamped as air.
    fn clear_rooms(&mut self) {
        let mut cols: Vec<(IVec2, i32)> = self.roof.iter().map(|(&c, &(low, _))| (c, low)).collect();
        cols.sort_unstable_by_key(|(c, _)| (c.x, c.y));
        let blocks = self.blocks.clone();
        for (c, low) in cols {
            let room = blocks.iter().find(|b| b.within(c));
            let from = match room {
                Some(b) => {
                    for y in 0..b.floor {
                        if self.get(v3(c, y)).is_none() {
                            self.put(v3(c, y), AIR);
                        }
                    }
                    b.feet()
                }
                None => 0,
            };
            for y in from..low {
                if self.get(v3(c, y)).is_none() {
                    self.put(v3(c, y), AIR);
                }
            }
        }
    }

    /// The building, and what was found wrong with it.
    pub fn finish(mut self, walls: Area, style_raised: i32) -> (Built, Vec<String>) {
        self.weather();
        self.clear_rooms();
        let mut voxels: Vec<(IVec3, Voxel)> = self.vox.iter().map(|(&q, &v)| (q, v)).collect();
        voxels.sort_unstable_by_key(|(q, _)| (q.y, q.x, q.z));
        // Every column it covers above the ground: its walls, eaves, steps and wheel, and
        // the fine parts about it.
        let (mut lo, mut hi) = (walls.lo, walls.hi);
        for (q, v) in &voxels {
            if *v != AIR && q.y >= 0 {
                lo = lo.min(IVec2::new(q.x, q.z));
                hi = hi.max(IVec2::new(q.x, q.z));
            }
        }
        for b in &self.fine {
            let (a, c) = (IVec3::from_array(b.lo).div_euclid(IVec3::splat(CELLS)), (IVec3::from_array(b.hi) - 1).div_euclid(IVec3::splat(CELLS)));
            lo = lo.min(IVec2::new(a.x, a.z));
            hi = hi.max(IVec2::new(c.x, c.z));
        }
        let door = self.door.unwrap_or(Door { floor: IVec3::new(walls.hi.x, 0, 0), facing: IVec3::X, width: 3, height: 5 });
        let mut beds = self.beds.clone();
        beds.extend(self.guest_beds.iter().copied());
        // Smoke rises from the chimney, or from the middle of the roof.
        let mid = (walls.lo + walls.hi) / 2;
        let smoke = self.smoke.unwrap_or_else(|| v3(mid, self.roof_top(mid).unwrap_or(STOREY) + 1));
        self.anchors.push(Anchor { kind: AnchorKind::RoofTop, at: smoke, normal: IVec3::Y });
        for (c, n) in [(IVec2::new(walls.lo.x - 1, walls.lo.y - 1), IVec3::new(-1, 0, -1)), (IVec2::new(walls.hi.x + 1, walls.lo.y - 1), IVec3::new(1, 0, -1)), (IVec2::new(walls.lo.x - 1, walls.hi.y + 1), IVec3::new(-1, 0, 1)), (IVec2::new(walls.hi.x + 1, walls.hi.y + 1), IVec3::new(1, 0, 1))] {
            if !self.solid(v3(c, 0)) {
                self.anchors.push(Anchor { kind: AnchorKind::Corner, at: v3(c, 0), normal: n });
            }
        }
        let mut b = Building {
            voxels,
            fine: std::mem::take(&mut self.fine),
            footprint: Area { lo, hi },
            walls,
            door,
            anchors: std::mem::take(&mut self.anchors),
            inside: Inside { floor: Vec::new(), beds, hearth: self.hearth, furniture: std::mem::take(&mut self.furniture), windows: std::mem::take(&mut self.windows) },
            clear: std::mem::take(&mut self.clear),
            smoke,
            style: Style { form: Form::Rect, walls: self.look.walls, roof: self.look.cover, shape: self.look.shape, raised: style_raised },
        };
        b.inside.floor = kit::walk(&b).floor;
        let built = Built {
            which: self.spec.which,
            building: b,
            guest_beds: self.guest_beds,
            uses: self.uses,
            fire: self.fire,
            wheel: self.wheel,
            guilds: self.guilds,
            sign: self.sign,
        };
        (built, self.faults)
    }
}
