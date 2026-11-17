//! Builds a home from its rules: the plan and its layout, then footings and floor, walls,
//! roof, hearth and chimney, the way in, and where everything goes.

use std::collections::BTreeMap;

use glam::{IVec2, IVec3, Vec2};
use voxel::{
    AIR, Voxel,
    mesh::{CELLS, PlantBox},
};

use super::{
    Anchor, AnchorKind, Area, Building, Door, Hearth, Inside, Opening, Spec, Spot, Step, Style,
    plan::{DIRS, HearthPlan, Layout, Params, Plan, Spot2, Wants},
    rules::{Cover, FloorKind, Form, RoofShape, Smoke, StepRules, Under, WallKind, choose},
};
use crate::{
    features::{
        landmarks::Palette,
        shape::{Rng, Shape},
    },
    materials as mat,
    noise::{hash3, unit},
};

/// How deep footings go below the ground.
const FOOT: i32 = 4;
/// The clear height a person needs, in voxels: someone 1.76 m tall is 4.4 voxels.
pub(super) const HEADROOM: i32 = 5;
/// The most the roof stands above the feet anywhere inside, so that moss and lichen,
/// which look 8 m up for a roof (features::moss), know every room is indoors even under
/// the cap of a smoke hole.
const ROOF_REACH: i32 = 15;
/// How deep a walled yard before a door is, at least and at most.
const YARD: (i32, i32) = (6, 9);

fn v3(q: IVec2, y: i32) -> IVec3 {
    IVec3::new(q.x, y, q.y)
}

fn flat(n: IVec2) -> IVec3 {
    IVec3::new(n.x, 0, n.y)
}

/// Everything decided about a building before it is laid out in voxels.
struct House<'a> {
    pal: &'a Palette,
    seed: u32,
    age: f32,
    wealth: f32,
    step: Step,
    plan: Plan,
    layout: Layout,
    walls: WallKind,
    cover: Cover,
    shape: RoofShape,
    thick: i32,
    /// The layer of the floor voxels; feet are a layer above.
    floor_y: i32,
    /// The top layer of the walls.
    plate: i32,
    base: i32,
    pitch: f32,
    overhang: i32,
    door_h: i32,
    floor: FloorKind,
    under: Under,
    veranda: i32,
    whitewash: bool,
    roof_stones: bool,
    courtyard: bool,
    parapet: i32,
    /// How much the ridge sags at the middle, in voxels.
    sag: f32,
    /// Each roof column's lowest and highest voxel, by (x, z).
    roof: BTreeMap<(i32, i32), (i32, i32)>,
    s: Shape,
    anchors: Vec<Anchor>,
    clear: Vec<(IVec3, IVec3)>,
    smoke: IVec3,
    hearth_at: Option<Hearth>,
    /// The x of a walled yard's gate, if there is one.
    gate: Option<i32>,
}

pub(super) fn make(spec: &Spec, pal: &Palette, r: &StepRules) -> Building {
    let age = spec.age.clamp(0.0, 1.0);
    let wealth = spec.wealth.clamp(0.0, 1.0);
    let seed = spec.seed ^ (spec.region as u32 + 1).wrapping_mul(0x9e37_79b9) ^ (spec.step as u32 + 1).wrapping_mul(0x85eb_ca6b);
    let mut rng = Rng::new(seed);
    let mut form = *choose(&r.footprint, wealth, rng.unit()).unwrap_or(&Form::Rect);
    let walls = *choose(&r.walls, wealth, rng.unit()).unwrap_or(&WallKind::Plank);
    let mut cover = *choose(&r.roof, wealth, rng.unit()).unwrap_or(&Cover::Thatch);
    let mut shape = *choose(&r.roof_shape, wealth, rng.unit()).unwrap_or(&RoofShape::Gable);
    let courtyard = rng.unit() < r.courtyard.at(wealth);
    let raised = r.raised.whole(rng.unit()).clamp(0, 10);
    // A round building takes a cone or a flat roof, and a yard only goes before a
    // rectangle on the ground.
    if form == Form::Round {
        shape = if shape == RoofShape::Flat { RoofShape::Flat } else { RoofShape::Cone };
    } else if shape == RoofShape::Cone {
        shape = RoofShape::Hip;
    }
    let courtyard = courtyard && raised == 0 && form != Form::Round;
    if courtyard && form == Form::L {
        form = Form::Rect;
    }
    if shape == RoofShape::Flat {
        cover = Cover::Mud;
    } else if cover == Cover::Mud {
        cover = Cover::Thatch;
    }
    let thick = r.thick.whole(rng.unit()).clamp(1, 2);
    let wall_h = r.wall_height.whole(rng.unit()).max(HEADROOM);
    let base = r.base.whole(rng.unit()).clamp(0, wall_h);
    let pitch = r.pitch.at(rng.unit()).clamp(0.3, 2.5);
    let overhang = if shape == RoofShape::Flat { 0 } else { r.overhang.whole(rng.unit()).clamp(0, 4) };
    let along_x = shape != RoofShape::Flat && rng.unit() < r.gable_door.at(wealth);
    let mut floor = *choose(&r.floor, wealth, rng.unit()).unwrap_or(&FloorKind::Boards);
    let under = *choose(&r.under, wealth, rng.unit()).unwrap_or(&Under::Stilts);
    if raised > 0 && under != Under::Plinth {
        floor = FloorKind::Boards;
    }
    let veranda = r.veranda.whole(rng.unit()).clamp(1, 4);
    let smoke = if form == Form::Round { Smoke::Hole } else { *choose(&r.smoke, wealth, rng.unit()).unwrap_or(&Smoke::Hole) };
    let lean_to = rng.unit() < r.lean_to.at(wealth) && shape == RoofShape::Gable && raised == 0 && !courtyard;
    let windows = r.windows.whole(wealth * 0.6 + rng.unit() * 0.4).max(0);
    let door_w = r.door_width.whole(rng.unit()).clamp(2, 4);
    let door_h = r.door_height.whole(rng.unit()).clamp(HEADROOM, wall_h);
    let whitewash = rng.unit() < r.whitewash.at(wealth);
    let roof_stones = rng.unit() < r.roof_stones.at(wealth) && matches!(shape, RoofShape::Gable | RoofShape::Hip);
    let parapet = r.parapet.whole(rng.unit()).clamp(1, 3);
    let chamfer = rng.unit() < r.round_corners.at(wealth);
    let door_shift = if along_x || form == Form::Round { 0.0 } else { rng.range(-0.7, 0.7) };
    let wing = (rng.range(0.7, 1.0), rng.range(0.5, 0.9), rng.unit() < 0.5);
    let furniture = 1 + (wealth * 3.0).round() as i32 + (spec.step != Step::Hut) as i32;
    let size = (wealth * 0.8 + rng.unit() * 0.4 - 0.2).clamp(0.0, 1.0);
    let sag = if age > 0.5 { (age - 0.5) * 4.0 } else { 0.0 };
    let gables = match shape {
        RoofShape::Gable if along_x => [Some(IVec2::NEG_X), Some(IVec2::X)],
        RoofShape::Gable => [Some(IVec2::Y), Some(IVec2::NEG_Y)],
        _ => [None, None],
    };
    let sleepers = spec.sleepers.min(spec.step.max_sleepers()) as i32;
    let mut width = r.width.whole(size).max(2 * thick + 4);
    let mut length = r.length.whole(size).max(width);
    // Grow the plan until everyone asked for has a bed; a plan that never fits (rules
    // asking for something very small) sleeps as many as it can.
    let mut found = None;
    for attempt in 0..16 {
        // A round hut that has grown half as big again and still cannot sleep everyone is
        // built square instead, as a curved wall wastes the floor beds go on.
        if attempt == 6 && form == Form::Round {
            form = Form::Rect;
            width = r.width.whole(size).max(2 * thick + 4);
            length = r.length.whole(size).max(width);
            if shape == RoofShape::Cone {
                shape = RoofShape::Hip;
            }
        }
        let params = Params { form, width, length, along_x, thick, door_width: door_w, door_shift, wing, chamfer };
        let wants = Wants { chimney: smoke == Smoke::Chimney, gables, windows, sleepers: if attempt < 14 { sleepers } else { 0 }, furniture, lean_to, seed };
        if let Some(plan) = Plan::new(&params)
            && let Some(layout) = Layout::new(&plan, &wants)
        {
            found = Some((plan, layout));
            break;
        }
        length += 2;
        if attempt % 2 == 1 || form == Form::Round {
            width += 1;
        }
    }
    let (plan, layout) = found.expect("a plan that sleeps nobody always fits");
    let floor_y = raised;
    let plate = floor_y + wall_h;
    let mut h = House {
        pal,
        seed,
        age,
        wealth,
        step: spec.step,
        walls,
        cover,
        shape,
        thick,
        floor_y,
        plate,
        base,
        pitch,
        overhang,
        door_h,
        floor,
        under,
        veranda,
        whitewash,
        roof_stones,
        courtyard,
        parapet,
        sag,
        roof: BTreeMap::new(),
        s: Shape::raw(),
        anchors: Vec::new(),
        clear: Vec::new(),
        smoke: IVec3::ZERO,
        hearth_at: None,
        gate: None,
        plan,
        layout,
    };
    // Keep the roof low enough over every room that a roof is near overhead (see
    // ROOF_REACH): wide buildings take a gentler pitch.
    let deepest = h.deepest();
    if deepest > 0.0 {
        let most = (ROOF_REACH - (plate + 1 - (floor_y + 1)) - 1) as f32 / deepest;
        h.pitch = h.pitch.min(most.max(0.3));
    }
    h.build()
}

impl House<'_> {
    fn feet(&self) -> i32 {
        self.floor_y + 1
    }

    /// A value 0..1 steady over blocks of `size` voxels, for patches of weathering.
    fn patch(&self, q: IVec3, size: i32, salt: u32) -> f32 {
        unit(hash3(self.seed ^ salt, q.x.div_euclid(size), q.y.div_euclid(size), q.z.div_euclid(size)))
    }

    fn grain(&self, q: IVec3, salt: u32) -> f32 {
        unit(hash3(self.seed ^ salt, q.x, q.y, q.z))
    }

    fn put(&mut self, q: IVec3, v: Voxel) {
        self.s.paint(q, v);
    }

    fn empty(&self, q: IVec3) -> bool {
        self.s.get(q).is_none()
    }

    fn build(mut self) -> Building {
        self.roof_field();
        self.underfloor();
        self.floors();
        self.walls();
        self.doorway();
        self.windows();
        self.roof_voxels();
        self.rooms();
        self.hearth();
        self.approach();
        self.yard();
        self.lean_to();
        self.log_ends();
        self.weather();
        self.finish()
    }

    // The plan's shape, for the roof.

    /// How far in from the eaves the ridge is: the most a roof rises, in runs.
    fn deepest(&self) -> f32 {
        match self.shape {
            RoofShape::Flat => 0.0,
            RoofShape::Cone => self.plan.radius - 0.5,
            RoofShape::Gable => self.plan.sections.iter().map(|s| (if s.along_x { s.hi.y - s.lo.y } else { s.hi.x - s.lo.x }) as f32 * 0.5).fold(0.0, f32::max),
            RoofShape::Hip => self.plan.sections.iter().map(|s| (s.hi - s.lo).min_element() as f32 * 0.5).fold(0.0, f32::max),
        }
    }

    /// The top layer of the roof over column `q`, before anything is cut into it, and how
    /// far in from the eaves it is (below nought out past the walls).
    fn roof_top(&self, q: IVec2) -> Option<(i32, f32)> {
        let t = 2;
        let oh = self.overhang;
        let rise = |d: f32| (self.pitch * d).round() as i32;
        match self.shape {
            RoofShape::Flat => self.plan.outer.has(q).then_some((self.plate + 1, 0.0)),
            RoofShape::Cone => {
                let d = self.plan.radius - 0.5 - (q.as_vec2() + Vec2::splat(0.5)).length();
                (d >= -(oh as f32) - 0.5).then(|| (self.plate + t + rise(d), d))
            }
            RoofShape::Gable | RoofShape::Hip => {
                let mut best: Option<(i32, f32)> = None;
                for s in &self.plan.sections {
                    if (q.x < s.lo.x - oh || q.x > s.hi.x + oh) || (q.y < s.lo.y - oh || q.y > s.hi.y + oh) {
                        continue;
                    }
                    let across = if s.along_x { (q.y - s.lo.y).min(s.hi.y - q.y) } else { (q.x - s.lo.x).min(s.hi.x - q.x) };
                    let lengthwise = if s.along_x { (q.x - s.lo.x).min(s.hi.x - q.x) } else { (q.y - s.lo.y).min(s.hi.y - q.y) };
                    let d = if self.shape == RoofShape::Hip { across.min(lengthwise) } else { across } as f32;
                    let mut top = self.plate + t + rise(d);
                    if self.sag > 0.0 && self.shape == RoofShape::Gable && d > 0.0 {
                        // An old ridge dips toward its middle; the eaves hold their line.
                        let (a, b) = if s.along_x { (s.lo.x, s.hi.x) } else { (s.lo.y, s.hi.y) };
                        let along = if s.along_x { q.x } else { q.y };
                        let f = ((along - a) as f32 / (b - a).max(1) as f32).clamp(0.0, 1.0);
                        let ridge = (if s.along_x { s.hi.y - s.lo.y } else { s.hi.x - s.lo.x }) as f32 * 0.5;
                        top -= (self.sag * 4.0 * f * (1.0 - f) * (d / ridge.max(1.0)).min(1.0)).round() as i32;
                    }
                    if best.is_none_or(|(b, _)| top > b) {
                        best = Some((top, d));
                    }
                }
                best
            }
        }
    }

    /// Works out each roof column's span of voxels: thick enough to be watertight between
    /// neighbours however steep, never down into a room or onto the ground, and lifted
    /// over the way in so a person walks under the eaves to the door.
    fn roof_field(&mut self) {
        let (lo, hi) = (self.plan.outer.lo - IVec2::splat(self.overhang + 1), self.plan.outer.hi + IVec2::splat(self.overhang + 1));
        let mut tops: BTreeMap<(i32, i32), i32> = BTreeMap::new();
        for z in lo.y..=hi.y {
            for x in lo.x..=hi.x {
                if let Some((top, _)) = self.roof_top(IVec2::new(x, z)) {
                    tops.insert((x, z), top);
                }
            }
        }
        let thick = if self.shape == RoofShape::Flat { 1 } else { 2 };
        let low = if self.shape == RoofShape::Flat { self.plate + 1 } else { 2.max(self.floor_y + 2) };
        for (&(x, z), &top) in &tops {
            let q = IVec2::new(x, z);
            let neighbours = DIRS.iter().filter_map(|d| tops.get(&(x + d.x, z + d.y))).min().copied().unwrap_or(top);
            let mut bottom = (top - thick + 1).min(neighbours);
            if self.plan.outer.has(q) {
                bottom = bottom.max(self.plate + 1);
            }
            bottom = bottom.max(low);
            if bottom <= top {
                self.roof.insert((x, z), (bottom, top));
            }
        }
        // Over the way in, the eaves rise to give a person headroom: an eyebrow over the door.
        let need = self.feet() + HEADROOM;
        let front = self.plan.front();
        let end = self.approach_end().max(front + self.overhang + 1);
        for z in self.plan.door_z.0 - 1..=self.plan.door_z.1 + 1 {
            for x in front..=end {
                let Some(&(bottom, top)) = self.roof.get(&(x, z)) else { continue };
                if bottom >= need {
                    continue;
                }
                let inner = self.roof.get(&(x - 1, z)).map_or(need, |r| r.0);
                self.roof.insert((x, z), (need, top.max(need + thick - 1).max(inner)));
            }
        }
    }

    /// The furthest x a person walks over to the door: the step or veranda, and the stairs.
    fn approach_end(&self) -> i32 {
        self.plan.front() + self.veranda - 1 + self.floor_y.max(0) + 1
    }

    fn roof_bottom(&self, q: IVec2) -> Option<i32> {
        self.roof.get(&(q.x, q.y)).map(|r| r.0)
    }

    // Below the floor.

    fn veranda_cells(&self) -> Vec<IVec2> {
        let front = self.plan.front();
        let (z0, z1) = self.plan.door_z;
        let (zl, zh) = if self.floor_y > 0 {
            let outer_z: Vec<i32> = self.plan.outer.cells().iter().filter(|q| q.x == front - 1).map(|q| q.y).collect();
            let (a, b) = (*outer_z.iter().min().unwrap_or(&z0), *outer_z.iter().max().unwrap_or(&z1));
            ((z0 - 2).max(a), (z1 + 2).min(b))
        } else {
            (z0, z1)
        };
        let mut out = Vec::new();
        for z in zl..=zh {
            for x in front..front + self.veranda {
                out.push(IVec2::new(x, z));
            }
        }
        out
    }

    /// Footings, and for a raised floor what holds it up: posts, a stone podium or roots.
    fn underfloor(&mut self) {
        let pal = *self.pal;
        let fy = self.floor_y;
        if fy <= 0 {
            for q in self.plan.room.cells().into_iter().chain(self.plan.door.clone()) {
                for y in -2..0 {
                    self.put(v3(q, y), mat::DIRT);
                }
            }
            return;
        }
        let mut area: Vec<IVec2> = self.plan.outer.cells();
        area.extend(self.veranda_cells());
        let set: std::collections::HashSet<IVec2> = area.iter().copied().collect();
        match self.under {
            Under::Stilts => {
                // Posts at the corners and every few columns round the edge, and a few under
                // the middle; a beam under the walls.
                let mut posts = Vec::new();
                for &q in &area {
                    let rim = DIRS.iter().any(|d| !set.contains(&(q + *d)));
                    let corner = rim && {
                        let open = |d: IVec2| !set.contains(&(q + d));
                        (open(IVec2::X) || open(IVec2::NEG_X)) && (open(IVec2::Y) || open(IVec2::NEG_Y))
                    };
                    if corner || (rim && (q.x + q.y).rem_euclid(3) == 0) || (q.x.rem_euclid(4) == 0 && q.y.rem_euclid(4) == 0) {
                        posts.push(q);
                    }
                    if rim || self.plan.wall.has(q) {
                        self.put(v3(q, fy - 1), pal.wood);
                    }
                }
                for q in posts {
                    for y in -FOOT..fy {
                        self.put(v3(q, y), pal.wood);
                    }
                }
            }
            Under::Plinth | Under::Roots => {
                let roots = self.under == Under::Roots;
                for &q in &area {
                    for y in -FOOT..fy {
                        let at = v3(q, y);
                        let v = if roots {
                            if self.grain(at, 0x2007) < 0.3 { mat::BARK_REDWOOD_RIDGE } else { pal.wood }
                        } else if y == fy - 1 {
                            pal.stone_dark
                        } else {
                            self.stonework(at)
                        };
                        self.put(at, v);
                    }
                }
                if roots {
                    // Buttress roots flaring out from the corners and along the sides, down
                    // into the ground.
                    let mut feet = Vec::new();
                    for &q in &area {
                        for d in DIRS {
                            if !set.contains(&(q + d)) && (q.x * 3 + q.y * 5).rem_euclid(4) == 0 {
                                feet.push((q, d));
                            }
                        }
                    }
                    for (q, d) in feet {
                        for k in 1..=fy + 1 {
                            let c = q + d * k;
                            let (z0, z1) = self.plan.door_z;
                            if c.x >= self.plan.front() && c.y >= z0 - 1 && c.y <= z1 + 1 {
                                break;
                            }
                            for y in -FOOT..fy - k {
                                self.put(v3(c, y), pal.wood);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Rough stone of the place: its rock, the darker stone with it, moss where it is damp.
    fn stonework(&self, q: IVec3) -> Voxel {
        let r = self.grain(q, 0x570e);
        if r < self.pal.moss * (0.3 + 0.5 * self.age) {
            mat::MOSSY_STONE
        } else if r < 0.35 {
            self.pal.stone_dark
        } else {
            self.pal.stone
        }
    }

    fn floor_voxel(&self, q: IVec2) -> Voxel {
        let at = v3(q, self.floor_y);
        match self.floor {
            FloorKind::Earth => mat::PATH_DIRT,
            FloorKind::Flags => {
                if self.patch(at, 2, 0xf1a6) < 0.5 { self.pal.stone } else { self.pal.stone_dark }
            }
            FloorKind::Boards => self.pal.plank,
        }
    }

    fn floors(&mut self) {
        let fy = self.floor_y;
        for q in self.plan.room.cells() {
            let v = self.floor_voxel(q);
            self.put(v3(q, fy), v);
        }
        // A raised floor runs out under the walls to the edge of the platform.
        if fy > 0 {
            for q in self.plan.wall.cells() {
                let v = if self.under == Under::Plinth { self.pal.stone_dark } else { self.pal.plank };
                self.put(v3(q, fy), v);
            }
        }
    }

    // The walls.

    /// Whether wall column `q` is a post of a framed wall: at a corner, or every few
    /// columns along a face.
    fn post(&self, q: IVec2, every: i32) -> bool {
        let out: Vec<IVec2> = self.plan.outward(q).collect();
        let Some(first) = out.first() else { return false };
        let corner = out.iter().any(|d| d.x != 0) && out.iter().any(|d| d.y != 0);
        let along = if first.x != 0 { q.y } else { q.x };
        corner || along.rem_euclid(every) == 0
    }

    /// The material of the wall at `q`, a voxel of a wall column.
    fn wall_voxel(&self, q: IVec3) -> Voxel {
        let pal = self.pal;
        let col = IVec2::new(q.x, q.z);
        let course = q.y - self.floor_y;
        if q.y < 0 || (self.floor_y == 0 && q.y == 0) {
            return pal.stone_dark;
        }
        let outside = self.plan.outward(col).next().is_some();
        let gable = q.y > self.plate;
        if course >= 1 && course <= self.base && !gable {
            return if course == 1 { pal.stone_dark } else { self.stonework(q) };
        }
        let flaked = self.patch(q, 2, 0xf1a4) < self.age * 0.45;
        let limewash = |bare: Voxel| if self.whitewash && !flaked { mat::WHITEWASH } else { bare };
        let timber = self.pal.wood;
        let top = q.y == self.plate;
        match self.walls {
            WallKind::Cob => limewash(mat::COB),
            WallKind::Mudbrick => {
                if self.whitewash && !flaked {
                    mat::WHITEWASH
                } else if self.patch(q, 2, 0xb41c) < self.age * 0.4 {
                    mat::MUD_BRICK
                } else {
                    mat::MUD_PLASTER
                }
            }
            WallKind::Drystone | WallKind::Stone if !outside => {
                if self.walls == WallKind::Stone { pal.stone } else { self.stonework(q) }
            }
            WallKind::Drystone => self.stonework(q),
            WallKind::Stone => {
                // Quoins: long and short stones turn each corner, a course at a time.
                let out: Vec<IVec2> = self.plan.outward(col).collect();
                let corner = out.iter().any(|d| d.x != 0) && out.iter().any(|d| d.y != 0);
                let beside = DIRS.iter().any(|&d| {
                    let c = col + d;
                    let o: Vec<IVec2> = self.plan.outward(c).collect();
                    o.iter().any(|d| d.x != 0) && o.iter().any(|d| d.y != 0)
                });
                if corner || (beside && course % 2 == 0) {
                    pal.stone_dark
                } else if self.pal.moss > 0.2 && self.patch(q, 2, 0x3055) < self.age * 0.25 {
                    mat::MOSSY_STONE
                } else {
                    pal.stone
                }
            }
            WallKind::Log => timber,
            WallKind::Frame => {
                let close = self.wealth > 0.7;
                let rail = course == (self.plate - self.floor_y) / 2 + 1;
                if gable {
                    if self.post(col, 4) { timber } else { pal.plank }
                } else if !outside {
                    limewash(mat::DAUB)
                } else if self.post(col, if close { 2 } else { 3 }) || top || rail || course == 1 {
                    timber
                } else {
                    limewash(mat::DAUB)
                }
            }
            WallKind::Wattle => {
                if self.post(col, 3) || top {
                    timber
                } else if self.patch(q, 2, 0x3a77) < self.age * 0.35 {
                    mat::WATTLE
                } else {
                    limewash(mat::DAUB)
                }
            }
            WallKind::Plank => {
                if self.post(col, 4) || top || course == 1 { timber } else { pal.plank }
            }
            WallKind::Reed => {
                if self.post(col, 3) || top || course == 1 { timber } else { mat::REED_THATCH }
            }
        }
    }

    fn walls(&mut self) {
        let start = if self.floor_y > 0 { self.floor_y + 1 } else { -FOOT };
        for q in self.plan.wall.cells() {
            let top = self.roof_bottom(q).map_or(self.plate, |b| (b - 1).max(self.plate));
            for y in start..=top {
                let at = v3(q, y);
                let v = self.wall_voxel(at);
                self.put(at, v);
            }
        }
        // A flat roof's parapet, its corners standing a course higher.
        if self.shape == RoofShape::Flat {
            for q in self.plan.wall.cells() {
                if self.plan.outward(q).next().is_none() {
                    continue;
                }
                let out: Vec<IVec2> = self.plan.outward(q).collect();
                let corner = out.iter().any(|d| d.x != 0) && out.iter().any(|d| d.y != 0);
                for k in 1..=self.parapet + corner as i32 {
                    let at = v3(q, self.plate + 1 + k);
                    let v = self.wall_voxel(v3(q, self.plate));
                    self.put(at, v);
                }
            }
        }
    }

    /// Whether the walls are of stone, which take stone lintels and jambs.
    fn stony(&self) -> bool {
        matches!(self.walls, WallKind::Stone | WallKind::Drystone)
    }

    fn doorway(&mut self) {
        let (fy, feet, dh) = (self.floor_y, self.feet(), self.door_h);
        let threshold = if fy > 0 && self.under != Under::Plinth { self.pal.plank } else { self.pal.stone_dark };
        let lintel = if self.stony() { self.pal.stone_dark } else { self.pal.wood };
        for q in self.plan.door.clone() {
            self.put(v3(q, fy), threshold);
            for y in feet..=fy + dh {
                self.put(v3(q, y), AIR);
            }
            if fy + dh + 1 <= self.plate {
                self.put(v3(q, fy + dh + 1), lintel);
            }
        }
        // Jambs either side.
        let xs: Vec<i32> = self.plan.door.iter().map(|q| q.x).collect();
        let (x0, x1) = (*xs.iter().min().unwrap(), *xs.iter().max().unwrap());
        for z in [self.plan.door_z.0 - 1, self.plan.door_z.1 + 1] {
            for x in x0..=x1 {
                let q = IVec2::new(x, z);
                if self.plan.wall.has(q) {
                    for y in feet..=(fy + dh + 1).min(self.plate) {
                        self.put(v3(q, y), lintel);
                    }
                }
            }
        }
    }

    fn windows(&mut self) {
        let lo = self.floor_y + 3;
        let lintel = if self.stony() { self.pal.stone_dark } else { self.pal.wood };
        for w in self.layout.windows.clone() {
            for &q in &w.through {
                for y in lo..=lo + 1 {
                    self.put(v3(q, y), AIR);
                }
                if lo + 2 <= self.plate {
                    self.put(v3(q, lo + 2), lintel);
                }
            }
        }
    }

    /// The roof's covering, weathered with age into patches of older material, and patched
    /// with new where it is old.
    fn cover_voxel(&self, q: IVec3) -> Voxel {
        let (new, old) = match self.cover {
            Cover::Thatch => (mat::THATCH, mat::THATCH_OLD),
            Cover::Reed => (mat::REED_THATCH, mat::THATCH_OLD),
            Cover::Palm => (mat::PALM_THATCH, mat::THATCH_OLD),
            Cover::Turf => (mat::ROOF_TURF, mat::ROOF_TURF),
            Cover::Slate => (mat::SLATE, mat::SLATE),
            Cover::Tile => (mat::ROOF_TILE, mat::ROOF_TILE_OLD),
            Cover::Shingle => (mat::SHINGLE, mat::SHINGLE_OLD),
            Cover::Mud => (mat::ROOF_MUD, mat::ROOF_MUD),
        };
        let worn = self.patch(q, 3, 0x0d1d) < self.age * 0.8;
        let mended = self.age > 0.65 && self.patch(q + IVec3::splat(1), 4, 0x9a7c) < 0.15;
        if worn && !mended { old } else { new }
    }

    fn ridge_voxel(&self) -> Option<Voxel> {
        Some(match self.cover {
            Cover::Thatch | Cover::Reed | Cover::Palm => self.cover_voxel(IVec3::ZERO),
            Cover::Slate => self.pal.stone_dark,
            Cover::Tile => mat::ROOF_TILE_OLD,
            Cover::Shingle => self.pal.wood,
            Cover::Turf | Cover::Mud => return None,
        })
    }

    fn roof_voxels(&mut self) {
        let roof: Vec<((i32, i32), (i32, i32))> = self.roof.iter().map(|(k, v)| (*k, *v)).collect();
        for &((x, z), (bottom, top)) in &roof {
            for y in bottom..=top {
                let at = IVec3::new(x, y, z);
                // A turf roof is grass over boards.
                let v = if self.cover == Cover::Turf && y < top { self.pal.plank } else { self.cover_voxel(at) };
                self.put(at, v);
            }
        }
        if self.shape == RoofShape::Flat {
            return;
        }
        // A ridge: a roll of thatch, ridge tiles or a ridge board along the top.
        let tops: BTreeMap<(i32, i32), i32> = roof.iter().map(|&(k, (_, t))| (k, t)).collect();
        let ridge: Vec<(i32, i32, i32)> = tops
            .iter()
            .filter(|&(&(x, z), &t)| {
                let higher = DIRS.iter().filter(|d| tops.get(&(x + d.x, z + d.y)).is_some_and(|&n| n > t)).count();
                let level = DIRS.iter().filter(|d| tops.get(&(x + d.x, z + d.y)).is_some_and(|&n| n == t)).count();
                higher == 0 && level <= 2 && self.roof_top(IVec2::new(x, z)).is_some_and(|(_, d)| d >= self.deepest() - 1.0)
            })
            .map(|(&(x, z), &t)| (x, z, t))
            .collect();
        if let Some(cap) = self.ridge_voxel() {
            for &(x, z, t) in &ridge {
                self.put(IVec3::new(x, t + 1, z), cap);
            }
        }
        for (k, &(x, z, t)) in ridge.iter().enumerate() {
            if k % 4 == 2 {
                let up = if self.ridge_voxel().is_some() { 2 } else { 1 };
                self.anchors.push(Anchor { kind: AnchorKind::Ridge, at: IVec3::new(x, t + up, z), normal: IVec3::Y });
            }
        }
        // Rows of stones on battens, across each slope, to hold the roof down in the wind.
        if self.roof_stones {
            let deepest = self.deepest();
            for &((x, z), (_, t)) in &roof {
                let q = IVec2::new(x, z);
                let Some((_, d)) = self.roof_top(q) else { continue };
                let row = d.round() as i32;
                if row >= 1 && (row - 1) % 3 == 0 && (d as f32) < deepest - 1.0 && !ridge.iter().any(|r| (r.0, r.1) == (x, z)) {
                    self.put(IVec3::new(x, t + 1, z), self.pal.wood);
                    if (x * 7 + z * 13).rem_euclid(3) == 0 {
                        let v = if self.grain(IVec3::new(x, t, z), 0x5703) < 0.5 { self.pal.stone } else { self.pal.stone_dark };
                        self.put(IVec3::new(x, t + 2, z), v);
                    }
                }
            }
        }
    }

    /// Hollows the rooms out to the roof, and puts tie beams across a house's open roof.
    fn rooms(&mut self) {
        let feet = self.feet();
        for q in self.plan.room.cells() {
            let top = self.roof_bottom(q).map_or(self.plate, |b| b - 1);
            for y in feet..=top {
                self.put(v3(q, y), AIR);
            }
        }
        let beam_y = self.plate + 1;
        if self.step == Step::Hut || self.shape == RoofShape::Cone {
            return;
        }
        if self.shape == RoofShape::Flat {
            // Vigas: the beams of a flat roof, their ends standing out through the walls.
            if self.plate < feet + HEADROOM {
                return;
            }
            let along_x = (self.plan.outer.hi.x - self.plan.outer.lo.x) > (self.plan.outer.hi.y - self.plan.outer.lo.y);
            for q in self.plan.outer.cells() {
                let line = if along_x { q.x } else { q.y };
                if line.rem_euclid(3) != 1 {
                    continue;
                }
                if self.plan.room.has(q) || self.plan.wall.has(q) {
                    self.put(v3(q, self.plate), self.pal.wood);
                }
                // Their ends stand out of the walls, though not over the way in.
                for d in self.plan.outward(q).collect::<Vec<_>>() {
                    let end = q + d;
                    let (z0, z1) = self.plan.door_z;
                    if (d.x != 0) == along_x && !(end.x >= self.plan.front() && end.y >= z0 - 1 && end.y <= z1 + 1) {
                        self.put(v3(end, self.plate), self.pal.wood);
                    }
                }
            }
            return;
        }
        for s in self.plan.sections.clone() {
            for q in self.plan.room.cells() {
                if !(q.cmpge(s.lo).all() && q.cmple(s.hi).all()) {
                    continue;
                }
                let line = if s.along_x { q.x } else { q.y };
                if line.rem_euclid(4) != 2 || self.roof_bottom(q).is_none_or(|b| b <= beam_y) {
                    continue;
                }
                self.put(v3(q, beam_y), self.pal.wood);
            }
        }
    }

    /// The hearth: a fireplace and its chimney stack, or a hearth in the floor under a
    /// capped smoke hole.
    fn hearth(&mut self) {
        let (fy, feet) = (self.floor_y, self.feet());
        match self.layout.hearth.clone() {
            HearthPlan::Fireplace(f) => {
                let local = |h: &Self, c: IVec2| {
                    let mut t = h.plate;
                    for d in [IVec2::ZERO, IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y] {
                        if let Some(r) = h.roof.get(&(c.x + d.x, c.y + d.y)) {
                            t = t.max(r.1);
                        }
                    }
                    t
                };
                let mut top = f.stack.iter().map(|&c| local(self, c)).max().unwrap_or(self.plate) + 2;
                if self.shape == RoofShape::Flat {
                    top = top.max(self.plate + self.parapet + 3);
                }
                let start = if fy > 0 && self.under == Under::Stilts { fy + 1 } else { -FOOT };
                for &c in &f.stack {
                    let corner = f.stack.iter().filter(|&&o| (o - c).abs().max_element() == 1 && (o - c).abs().min_element() == 0).count() <= 2;
                    for y in start..=top {
                        let at = v3(c, y);
                        let v = if y == top || (corner && y % 2 == 0) { self.pal.stone_dark } else { self.pal.stone };
                        self.put(at, v);
                    }
                }
                // The fire's opening over the hearth stone, blackened with soot all round
                // (which no moss takes to), and a mantel beam over it.
                let t = IVec2::new(f.normal.y, f.normal.x);
                self.put(v3(f.firebox, fy), mat::CHARCOAL);
                for y in feet..feet + 3 {
                    self.put(v3(f.firebox, y), AIR);
                    for c in [f.firebox - f.normal, f.firebox + t, f.firebox - t] {
                        self.put(v3(c, y), mat::CHARCOAL);
                    }
                }
                for k in [-1, 1] {
                    self.put(v3(f.firebox + t * k, feet + 3), self.pal.wood);
                }
                self.put(v3(f.firebox, feet + 3), mat::CHARCOAL);
                // Soot at the top of the flue, in the middle of the stack.
                let flue = f.firebox - f.normal * (self.thick - 1);
                self.put(v3(flue, top), mat::CHARCOAL);
                self.smoke = v3(flue, top + 1);
                self.hearth_at = Some(Hearth { at: v3(f.firebox, feet), facing: flat(f.normal), central: false });
            }
            HearthPlan::Central(at) => {
                for d in [IVec2::ZERO, IVec2::X, IVec2::Y, IVec2::ONE] {
                    self.put(v3(at + d, fy), self.pal.stone_dark);
                }
                // A kerb of stones round it, low enough to step over.
                let c = CELLS;
                let (x0, z0, y0) = (at.x * c, at.y * c, feet * c);
                let kerb = [((x0 - 1, z0 - 1), (x0 + 2 * c + 1, z0)), ((x0 - 1, z0 + 2 * c), (x0 + 2 * c + 1, z0 + 2 * c + 1)), ((x0 - 1, z0), (x0, z0 + 2 * c)), ((x0 + 2 * c, z0), (x0 + 2 * c + 1, z0 + 2 * c))];
                for ((ax, az), (bx, bz)) in kerb {
                    self.s.add_fine(fixed([ax, y0, az], [bx, y0 + 1, bz], self.pal.stone));
                }
                self.vent(at + IVec2::ONE);
                self.hearth_at = Some(Hearth { at: v3(at, feet), facing: IVec3::Y, central: true });
            }
        }
    }

    /// A smoke hole at the top of the roof nearest `near`, under a cap held up at its
    /// corners, so the smoke goes out through the slots between and the rain and snow
    /// stay out.
    fn vent(&mut self, near: IVec2) {
        let near = near.as_vec2();
        let best = self
            .roof
            .iter()
            .filter(|(k, _)| self.plan.room.has(IVec2::new(k.0, k.1)))
            .max_by(|a, b| {
                let score = |(&(x, z), &(_, t)): (&(i32, i32), &(i32, i32))| t as f32 * 4.0 - (IVec2::new(x, z).as_vec2() - near).length();
                score(*a).total_cmp(&score(*b)).then(b.0.cmp(a.0))
            })
            .map(|(&k, &r)| (k, r));
        let Some(((x, z), _)) = best else { return };
        let c = IVec2::new(x, z);
        let around = |d: (i32, i32)| (x + d.0, z + d.1);
        let mut top = 0;
        for dz in -1..=1 {
            for dx in -1..=1 {
                if let Some(r) = self.roof.get(&around((dx, dz))) {
                    top = top.max(r.1);
                }
            }
        }
        let cap = self.cover_voxel(v3(c, top));
        let bottom = self.roof[&(x, z)].0;
        for y in bottom..=top + 1 {
            self.put(v3(c, y), AIR);
        }
        for dz in -1..=1i32 {
            for dx in -1..=1i32 {
                let q = c + IVec2::new(dx, dz);
                if dx != 0 && dz != 0 {
                    let from = self.roof.get(&(q.x, q.y)).map_or(top, |r| r.1) + 1;
                    for y in from..=top + 1 {
                        self.put(v3(q, y), cap);
                    }
                } else if (dx, dz) != (0, 0) && self.s.get(v3(q, top + 1)).is_some_and(|v| v != AIR) && self.roof.get(&(q.x, q.y)).is_some_and(|r| r.1 <= top) {
                    // The slots between the corners stay open.
                    self.put(v3(q, top + 1), AIR);
                }
                self.put(v3(q, top + 2), cap);
            }
        }
        self.smoke = v3(c, top + 3);
    }

    /// The step or veranda before the door, the stairs down from a raised one, and the way
    /// in kept clear.
    fn approach(&mut self) {
        let pal = *self.pal;
        let fy = self.floor_y;
        let (z0, z1) = self.plan.door_z;
        let plinth = fy > 0 && self.under != Under::Stilts;
        let deck = if fy <= 0 || self.under == Under::Plinth { pal.stone } else { pal.plank };
        let veranda = self.veranda_cells();
        for &q in &veranda {
            self.put(v3(q, fy), deck);
            if fy <= 0 {
                self.put(v3(q, -1), pal.stone_dark);
            }
        }
        if fy > 0 && !plinth {
            // Posts under the veranda's outer corners and a beam under its edge.
            let xs = veranda.iter().map(|q| q.x).max().unwrap();
            let zs: Vec<i32> = veranda.iter().map(|q| q.y).collect();
            let (zl, zh) = (*zs.iter().min().unwrap(), *zs.iter().max().unwrap());
            for z in zl..=zh {
                self.put(IVec3::new(xs, fy - 1, z), pal.wood);
            }
            for z in [zl, zh] {
                for y in -FOOT..fy - 1 {
                    self.put(IVec3::new(xs, y, z), pal.wood);
                }
            }
        }
        // Stairs, a voxel down for each voxel out.
        let x1 = self.plan.front() + self.veranda - 1;
        for k in 1..=fy {
            let (x, y) = (x1 + k, fy - k);
            for z in z0..=z1 {
                if plinth {
                    for yy in -FOOT..=y {
                        let at = IVec3::new(x, yy, z);
                        let v = if yy == y { pal.stone_dark } else { self.stonework(at) };
                        self.put(at, v);
                    }
                } else {
                    self.put(IVec3::new(x, y, z), pal.plank);
                    self.put(IVec3::new(x, y - 1, z), pal.wood);
                }
            }
        }
        // Clear head height over every column walked to the door.
        for z in z0 - 1..=z1 + 1 {
            for x in self.plan.front()..=self.approach_end() {
                let surface = (-1..=fy).rev().find(|&y| self.s.get(IVec3::new(x, y, z)).is_some_and(|v| v != AIR)).unwrap_or(-1);
                for y in surface + 1..=surface + HEADROOM {
                    if self.empty(IVec3::new(x, y, z)) {
                        self.put(IVec3::new(x, y, z), AIR);
                    }
                }
            }
        }
        let (front, end) = (self.plan.front(), self.approach_end());
        self.clear.push((IVec3::new(self.plan.door_inner() - 2, 0.min(fy), z0 - 1), IVec3::new(end + 1, fy + self.door_h, z1 + 1)));
        let mid = (z0 + z1).div_euclid(2);
        self.anchors.push(Anchor { kind: AnchorKind::Door, at: IVec3::new(front, fy + 1, mid), normal: IVec3::X });
    }

    /// A walled yard before the door, its gate the way in.
    fn yard(&mut self) {
        if !self.courtyard {
            return;
        }
        let pal = *self.pal;
        let s = self.plan.sections[0];
        let front = self.plan.front();
        let depth = YARD.0 + (self.grain(IVec3::ZERO, 0x7a2d) * (YARD.1 - YARD.0 + 1) as f32) as i32;
        let (gx, zl, zh) = (front + depth, s.lo.y, s.hi.y);
        let (z0, z1) = self.plan.door_z;
        let mut wall = Vec::new();
        for x in front..=gx {
            wall.push(IVec2::new(x, zl));
            wall.push(IVec2::new(x, zh));
        }
        for z in zl + 1..zh {
            wall.push(IVec2::new(gx, z));
        }
        for q in wall {
            let gate = q.x == gx && q.y >= z0 && q.y <= z1;
            let post = q.x == gx && (q.y == z0 - 1 || q.y == z1 + 1);
            let top = if post { 6 } else { 4 };
            for y in -FOOT..=top {
                let at = v3(q, y);
                if gate {
                    if y == -1 {
                        self.put(at, pal.stone_dark);
                    } else if (0..=5).contains(&y) {
                        self.put(at, AIR);
                    }
                    continue;
                }
                let v = if y == top { pal.stone_dark } else if y < 0 { pal.stone_dark } else { self.wall_voxel(IVec3::new(q.x, y.max(self.base + 1).min(self.plate - 1), q.y)) };
                self.put(at, v);
            }
        }
        for z in z0 - 1..=z1 + 1 {
            self.put(IVec3::new(gx, 6, z), pal.wood);
        }
        // The yard's floor of trodden earth, cleared of whatever grew there.
        for z in zl + 1..zh {
            for x in front..gx {
                if self.empty(IVec3::new(x, -1, z)) {
                    self.put(IVec3::new(x, -1, z), mat::PATH_DIRT);
                }
                for y in 0..=5 {
                    if self.empty(IVec3::new(x, y, z)) {
                        self.put(IVec3::new(x, y, z), AIR);
                    }
                }
            }
        }
        self.clear.push((IVec3::new(front, 0, z0 - 1), IVec3::new(gx + 2, 5, z1 + 1)));
        for (x, z) in [(front + 2, zl + 2), (front + 2, zh - 2), (gx - 2, zl + 2), (gx - 2, zh - 2)] {
            if z < z0 - 1 || z > z1 + 1 {
                self.anchors.push(Anchor { kind: AnchorKind::Yard, at: IVec3::new(x, 0, z), normal: IVec3::Y });
            }
        }
        self.gate = Some(gx);
    }

    /// An open shed against a gable end: posts and a roof of one slope.
    fn lean_to(&mut self) {
        let Some(l) = self.layout.lean_to.clone() else { return };
        let pal = *self.pal;
        let depth = 3 + (self.grain(IVec3::ONE, 0x1ea7) * 2.0) as i32;
        let n = l.normal;
        let t = IVec2::new(n.y.abs(), n.x.abs());
        let (first, last) = (l.along[0], *l.along.last().unwrap());
        let high = self.plate - 1;
        let roof_y = |k: i32| high - (k - 1) / 2;
        let cover = if self.cover == Cover::Turf { pal.plank } else { self.cover_voxel(IVec3::ZERO) };
        for a in std::iter::once(first - t).chain(l.along.iter().copied()).chain(std::iter::once(last + t)) {
            for k in 1..=depth + 1 {
                let q = a + n * k;
                self.put(v3(q, roof_y(k)), cover);
                // Where the slope steps down, a second layer keeps it whole.
                if k > 1 && roof_y(k) < roof_y(k - 1) {
                    self.put(v3(q, roof_y(k - 1)), cover);
                }
            }
        }
        for a in [first, last] {
            let q = a + n * depth;
            for y in -FOOT..roof_y(depth) {
                self.put(v3(q, y), pal.wood);
            }
        }
        let mid = l.along[l.along.len() / 2] + n * 2;
        self.anchors.push(Anchor { kind: AnchorKind::Yard, at: v3(mid, 0), normal: IVec3::Y });
    }

    /// Log ends crossing at the corners, a course one way and the next the other.
    fn log_ends(&mut self) {
        if self.walls != WallKind::Log || self.plan.form == Form::Round {
            return;
        }
        let start = if self.floor_y > 0 { self.floor_y + 1 } else { 0 };
        for q in self.plan.wall.cells() {
            let out: Vec<IVec2> = self.plan.outward(q).collect();
            let (Some(&dx), Some(&dz)) = (out.iter().find(|d| d.x != 0), out.iter().find(|d| d.y != 0)) else { continue };
            for y in start..=self.plate {
                let d = if (y - start) % 2 == 0 { dx } else { dz };
                let at = v3(q + d, y);
                if self.empty(at) {
                    self.put(at, self.pal.plank);
                }
            }
        }
    }

    /// Grey, weathered timber on an old building.
    fn weather(&mut self) {
        if self.age <= 0.0 {
            return;
        }
        let (wood, plank) = (self.pal.wood, self.pal.plank);
        for q in self.s.snapshot() {
            let Some(v) = self.s.get(q) else { continue };
            if v == wood && self.patch(q, 2, 0x3ead) < self.age * 0.3 {
                self.put(q, mat::WOOD_DEAD);
            } else if v == plank && plank != mat::PLANK_DARK && self.patch(q, 2, 0x91a2) < self.age * 0.35 {
                self.put(q, mat::PLANK_DARK);
            }
        }
    }

    fn solid(&self, q: IVec3) -> bool {
        self.s.get(q).is_some_and(|v| v != AIR)
    }

    /// Whether wall column `q` shows a plain face to the outside toward `n` at layers
    /// `ys`, for something fixed against it.
    fn face(&self, q: IVec2, n: IVec2, ys: std::ops::RangeInclusive<i32>) -> bool {
        self.plan.outer.has(q) && !self.plan.outer.has(q + n) && ys.clone().all(|y| self.solid(v3(q, y)) && !self.solid(v3(q + n, y)))
    }

    /// The door leaf and the shutters; then the anchors, what is inside, and the rest.
    fn finish(mut self) -> Building {
        let pal = *self.pal;
        let (fy, feet) = (self.floor_y, self.feet());
        let (z0, z1) = self.plan.door_z;
        let front = self.plan.front();
        let w = z1 - z0 + 1;
        // The door leaf, standing open outward against the wall beside the doorway on
        // whichever side has a plain stretch of wall for it, short of the eaves.
        let eaves = (z0 - w..=z1 + w).filter_map(|z| self.roof_bottom(IVec2::new(front, z))).min().unwrap_or(i32::MAX);
        let leaf_top = (fy + self.door_h).min(eaves - 1);
        let fits = |h: &Self, zs: std::ops::RangeInclusive<i32>| zs.clone().all(|z| h.face(IVec2::new(front - 1, z), IVec2::X, feet..=leaf_top));
        let side = if fits(&self, z0 - w..=z0 - 1) {
            Some(z0 - w)
        } else if fits(&self, z1 + 1..=z1 + w) {
            Some(z1 + 1)
        } else {
            None
        };
        if let Some(za) = side
            && leaf_top >= feet + 3
        {
            let c = CELLS;
            let (x, y0, y1) = (front * c, feet * c, (leaf_top + 1) * c);
            self.s.add_fine(fixed([x, y0, za * c], [x + 1, y1, (za + w) * c], pal.plank));
            // Ledges across its back, standing a cell proud of it.
            for y in [y0 + c / 2, y1 - c] {
                self.s.add_fine(fixed([x + 1, y, za * c + 1], [x + 2, y + 1, (za + w) * c - 1], pal.wood));
            }
        }
        // Shutters either side of each window, folded back against the wall, and a sill.
        let lo = fy + 3;
        for win in self.layout.windows.clone() {
            let (n, [a, b]) = (win.normal, win.face);
            let t = b - a;
            for q in [a - t, b + t] {
                if self.face(q, n, lo..=lo + 1) {
                    self.s.add_fine(on_face(v3(q, lo), n, (0, CELLS), (0, 2 * CELLS), 1, pal.plank));
                }
            }
            for q in [a, b] {
                if self.face(q, n, lo - 1..=lo - 1) {
                    self.s.add_fine(on_face(v3(q, lo - 1), n, (0, CELLS), (CELLS - 1, CELLS), 2, pal.wood));
                }
            }
        }
        // Anchors for the regional components (#203).
        let mut anchors = std::mem::take(&mut self.anchors);
        let leaf_zs = z0 - w - 1..=z1 + w + 1;
        for q in self.plan.wall.cells() {
            let out: Vec<IVec2> = self.plan.outward(q).collect();
            if let (Some(&dx), Some(&dz)) = (out.iter().find(|d| d.x != 0), out.iter().find(|d| d.y != 0)) {
                anchors.push(Anchor { kind: AnchorKind::Corner, at: v3(q + dx + dz, 0), normal: flat(dx + dz) });
            }
            for n in out {
                if !self.plan.plain_wall(q, n) || (n == IVec2::X && leaf_zs.contains(&q.y)) {
                    continue;
                }
                let along = if n.x != 0 { q.y } else { q.x };
                if along.rem_euclid(6) == 3 {
                    anchors.push(Anchor { kind: AnchorKind::WallFace, at: v3(q + n, 0), normal: flat(n) });
                }
                if along.rem_euclid(6) == 0
                    && let Some(b) = self.roof_bottom(q + n)
                {
                    anchors.push(Anchor { kind: AnchorKind::Eaves, at: v3(q + n, b - 1), normal: flat(n) });
                }
            }
        }
        if self.shape == RoofShape::Gable {
            for n in [IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y] {
                let main = self.plan.sections[0];
                if (n.x != 0) != main.along_x {
                    continue;
                }
                let mut face: Vec<IVec2> = self.plan.wall.cells().into_iter().filter(|&q| self.plan.plain_wall(q, n)).collect();
                face.sort_by_key(|q| (q.x * n.y.abs() + q.y * n.x.abs(), q.x, q.y));
                if let Some(&q) = face.get(face.len() / 2) {
                    anchors.push(Anchor { kind: AnchorKind::Gable, at: v3(q + n, self.plate + 2), normal: flat(n) });
                }
            }
        }
        anchors.push(Anchor { kind: AnchorKind::RoofTop, at: self.smoke, normal: IVec3::Y });
        if self.shape == RoofShape::Flat {
            let room = self.plan.room.cells();
            if let Some(&q) = room.get(room.len() / 2) {
                anchors.push(Anchor { kind: AnchorKind::RoofTop, at: v3(q, self.plate + 2), normal: IVec3::Y });
            }
        }
        for win in &self.layout.windows {
            anchors.push(Anchor { kind: AnchorKind::Window, at: v3(win.face[0] + win.normal, lo), normal: flat(win.normal) });
        }
        let (olo, ohi) = (self.plan.outer.lo, self.plan.outer.hi);
        let away = self.overhang + 3;
        let mid = (olo + ohi).div_euclid(IVec2::splat(2));
        for at in [IVec2::new(olo.x - away, mid.y), IVec2::new(mid.x, olo.y - away), IVec2::new(mid.x, ohi.y + away)] {
            anchors.push(Anchor { kind: AnchorKind::Yard, at: v3(at, 0), normal: IVec3::Y });
        }
        anchors.retain(|a| !self.solid(a.at));
        // Inside.
        let room = self.plan.room.cells();
        let spot = |s: &Spot2| Spot { at: v3(s.at, feet), along: flat(s.along), facing: flat(s.facing), len: s.len, wide: s.wide };
        let inside = Inside {
            floor: room.iter().map(|&q| v3(q, fy)).collect(),
            beds: self.layout.beds.iter().map(spot).collect(),
            hearth: self.hearth_at,
            furniture: self.layout.furniture.iter().map(spot).collect(),
            windows: self
                .layout
                .windows
                .iter()
                .map(|win| {
                    let (a, b) = win.through.iter().fold((IVec2::MAX, IVec2::MIN), |(a, b), q| (a.min(*q), b.max(*q)));
                    Opening { lo: v3(a, lo), hi: v3(b, lo + 1), normal: flat(win.normal) }
                })
                .collect(),
        };
        let (rlo, rhi) = room.iter().fold((IVec2::MAX, IVec2::MIN), |(a, b), q| (a.min(*q), b.max(*q)));
        let mut clear = std::mem::take(&mut self.clear);
        clear.push((v3(rlo, feet), v3(rhi, feet + HEADROOM - 1)));
        let mid_z = (z0 + z1).div_euclid(2);
        let door = match self.gate {
            Some(gx) => Door { floor: IVec3::new(gx, -1, mid_z), facing: IVec3::X, width: w, height: 6 },
            None => Door { floor: IVec3::new(front - 1, fy, mid_z), facing: IVec3::X, width: w, height: self.door_h },
        };
        let style = Style { form: self.plan.form, walls: self.walls, roof: self.cover, shape: self.shape, raised: fy };
        let walls = Area { lo: olo, hi: ohi };
        let smoke = self.smoke;
        let fine = self.s.take_fine();
        let voxels = self.s.into_voxels();
        let (mut flo, mut fhi) = (olo, ohi);
        for (q, v) in &voxels {
            if *v != AIR && q.y >= 0 {
                flo = flo.min(IVec2::new(q.x, q.z));
                fhi = fhi.max(IVec2::new(q.x, q.z));
            }
        }
        Building { voxels, fine, footprint: Area { lo: flo, hi: fhi }, walls, door, anchors, inside, clear, smoke, style }
    }
}

/// A fine part fixed in place, in model cells.
fn fixed(lo: [i32; 3], hi: [i32; 3], voxel: Voxel) -> PlantBox {
    PlantBox { lo, hi, voxel, sway_lo: false, sway_hi: false, thin: 0, season: 0, flap: 0 }
}

/// A box of cells on the outside of voxel `q`'s face toward `n`: `along` cells along the
/// face (from the voxel's corner toward +X or +Z), `up` cells from its foot, `out` cells
/// out from the face.
fn on_face(q: IVec3, n: IVec2, along: (i32, i32), up: (i32, i32), out: i32, voxel: Voxel) -> PlantBox {
    let c = CELLS;
    let base = q * c;
    let (y0, y1) = (base.y + up.0, base.y + up.1);
    let (lo, hi) = match (n.x, n.y) {
        (1, _) => ([base.x + c, y0, base.z + along.0], [base.x + c + out, y1, base.z + along.1]),
        (-1, _) => ([base.x - out, y0, base.z + along.0], [base.x, y1, base.z + along.1]),
        (_, 1) => ([base.x + along.0, y0, base.z + c], [base.x + along.1, y1, base.z + c + out]),
        _ => ([base.x + along.0, y0, base.z - out], [base.x + along.1, y1, base.z]),
    };
    fixed(lo, hi, voxel)
}
