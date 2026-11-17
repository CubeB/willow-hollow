//! A building seen from above: the columns inside its outer walls, which are wall and
//! which room, the doorway, and where the hearth, the windows, the beds and the furniture
//! go. Everything here is in columns (x, and z in a vector's `y`).

use std::collections::VecDeque;

use glam::IVec2;

use super::rules::Form;
use crate::noise::{hash2, unit};

/// The four ways along the ground.
pub(super) const DIRS: [IVec2; 4] = [IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y];

/// A set of columns over a box.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct Mask {
    pub lo: IVec2,
    pub hi: IVec2,
    bits: Vec<bool>,
}

impl Mask {
    pub fn new(lo: IVec2, hi: IVec2) -> Self {
        let n = ((hi.x - lo.x + 1).max(0) * (hi.y - lo.y + 1).max(0)) as usize;
        Mask { lo, hi, bits: vec![false; n] }
    }

    fn index(&self, q: IVec2) -> Option<usize> {
        if q.x < self.lo.x || q.y < self.lo.y || q.x > self.hi.x || q.y > self.hi.y {
            return None;
        }
        Some(((q.y - self.lo.y) * (self.hi.x - self.lo.x + 1) + (q.x - self.lo.x)) as usize)
    }

    pub fn has(&self, q: IVec2) -> bool {
        self.index(q).is_some_and(|i| self.bits[i])
    }

    pub fn set(&mut self, q: IVec2, v: bool) {
        if let Some(i) = self.index(q) {
            self.bits[i] = v;
        }
    }

    /// Every column in the set, row by row.
    pub fn cells(&self) -> Vec<IVec2> {
        let mut out = Vec::new();
        for z in self.lo.y..=self.hi.y {
            for x in self.lo.x..=self.hi.x {
                if self.has(IVec2::new(x, z)) {
                    out.push(IVec2::new(x, z));
                }
            }
        }
        out
    }

    pub fn count(&self) -> usize {
        self.bits.iter().filter(|b| **b).count()
    }
}

/// A rectangular block of a plan, roofed by one ridge.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Section {
    /// The outer wall line, inclusive.
    pub lo: IVec2,
    pub hi: IVec2,
    /// Whether its ridge runs along x (its gable ends then face -X and +X).
    pub along_x: bool,
}

/// What a plan is made from.
#[derive(Clone, Copy, Debug)]
pub(super) struct Params {
    pub form: Form,
    /// The short side, or a round building's width.
    pub width: i32,
    pub length: i32,
    /// The main ridge runs along x, so the door at +X is in a gable end.
    pub along_x: bool,
    pub thick: i32,
    pub door_width: i32,
    /// Where along its wall the door goes, -1 to 1, 0 in the middle.
    pub door_shift: f32,
    /// The wing of an L: its width and how far it reaches out, as shares of the width,
    /// and which side it is on.
    pub wing: (f32, f32, bool),
    pub chamfer: bool,
}

/// A building's plan.
#[derive(Clone, Debug)]
pub(super) struct Plan {
    pub form: Form,
    /// Every column inside the outer wall line, the walls included.
    pub outer: Mask,
    pub wall: Mask,
    pub room: Mask,
    /// The rectangular blocks, main first; none for a round building.
    pub sections: Vec<Section>,
    /// A round building's radius to the outside of its wall.
    pub radius: f32,
    pub thick: i32,
    /// The wall columns of the doorway, cut through the +X wall.
    pub door: Vec<IVec2>,
    /// The doorway's width, from z to z inclusive.
    pub door_z: (i32, i32),
    /// The column just outside the doorway at each of its z, in order.
    pub step: Vec<IVec2>,
}

impl Plan {
    pub fn new(p: &Params) -> Option<Plan> {
        let mut sections = Vec::new();
        let mut radius = 0.0;
        let (lo, hi);
        if p.form == Form::Round {
            radius = p.width as f32 * 0.5;
            let r = radius.ceil() as i32;
            (lo, hi) = (IVec2::splat(-r), IVec2::splat(r));
        } else {
            let (xe, ze) = if p.along_x { (p.length, p.width) } else { (p.width, p.length) };
            let (x0, z0) = (-xe / 2, -ze / 2);
            let main = Section { lo: IVec2::new(x0, z0), hi: IVec2::new(x0 + xe - 1, z0 + ze - 1), along_x: p.along_x };
            sections.push(main);
            if p.form == Form::L {
                let least = 2 * p.thick + 3;
                let ww = ((p.width as f32 * p.wing.0).round() as i32).max(least);
                let wl = ((p.width as f32 * p.wing.1).round() as i32).max(least - p.thick);
                if p.along_x {
                    // A wing at the back end, reaching out to one side, its ridge across.
                    let ww = ww.min(xe - 4);
                    let (zl, zh) = if p.wing.2 { (main.lo.y, main.hi.y + wl) } else { (main.lo.y - wl, main.hi.y) };
                    sections.push(Section { lo: IVec2::new(main.lo.x, zl), hi: IVec2::new(main.lo.x + ww - 1, zh), along_x: false });
                } else {
                    // A wing out of the back wall, at one end.
                    let ww = ww.min(ze);
                    let (zl, zh) = if p.wing.2 { (main.hi.y - ww + 1, main.hi.y) } else { (main.lo.y, main.lo.y + ww - 1) };
                    sections.push(Section { lo: IVec2::new(main.lo.x - wl, zl), hi: IVec2::new(main.hi.x, zh), along_x: true });
                }
            }
            lo = sections.iter().fold(IVec2::MAX, |a, s| a.min(s.lo));
            hi = sections.iter().fold(IVec2::MIN, |a, s| a.max(s.hi));
        }
        let mut outer = Mask::new(lo, hi);
        for z in lo.y..=hi.y {
            for x in lo.x..=hi.x {
                let q = IVec2::new(x, z);
                let inside = if p.form == Form::Round {
                    let (fx, fz) = (x as f32 + 0.5, z as f32 + 0.5);
                    fx * fx + fz * fz <= radius * radius
                } else {
                    sections.iter().any(|s| q.cmpge(s.lo).all() && q.cmple(s.hi).all())
                };
                outer.set(q, inside);
            }
        }
        if p.chamfer && p.form != Form::Round {
            // Cut each outside corner, as cob and drystone corners are rounded.
            let was = outer.clone();
            for q in was.cells() {
                let open = |d: IVec2| !was.has(q + d);
                if (open(IVec2::X) || open(IVec2::NEG_X)) && (open(IVec2::Y) || open(IVec2::NEG_Y)) {
                    outer.set(q, false);
                }
            }
        }
        // The wall is every column within `thick` of the outside, the room the rest; then
        // any corner of the room too tight for a person to stand in is walled up.
        let mut wall = Mask::new(lo, hi);
        let mut room = Mask::new(lo, hi);
        for q in outer.cells() {
            let t = p.thick;
            let edge = (-t..=t).any(|dz| (-t..=t).any(|dx| !outer.has(q + IVec2::new(dx, dz))));
            if edge { wall.set(q, true) } else { room.set(q, true) }
        }
        let open = |m: &Mask, q: IVec2| [IVec2::ZERO, IVec2::X, IVec2::Y, IVec2::ONE].iter().all(|d| m.has(q + *d));
        let was = room.clone();
        for q in was.cells() {
            let fits = [IVec2::ZERO, IVec2::NEG_X, IVec2::NEG_Y, IVec2::NEG_ONE].iter().any(|d| open(&was, q + *d));
            if !fits {
                room.set(q, false);
                wall.set(q, true);
            }
        }
        if room.count() < 4 {
            return None;
        }
        // The doorway, through the +X wall: where every z of it and the one either side has
        // room behind, nearest the place `door_shift` asks for.
        let room_x = |z: i32| (lo.x..=hi.x).rev().find(|&x| room.has(IVec2::new(x, z)));
        let outer_x = |z: i32| (lo.x..=hi.x).rev().find(|&x| outer.has(IVec2::new(x, z)));
        let w = p.door_width.max(2);
        let fits = |z0: i32| {
            (z0 - 1..=z0 + w).all(|z| room_x(z).is_some())
                && (z0..z0 + w).all(|z| outer_x(z) == outer_x(z0))
                && {
                    let r: Vec<i32> = (z0..z0 + w).filter_map(room_x).collect();
                    r.iter().max().unwrap() - r.iter().min().unwrap() <= 1
                }
        };
        let candidates: Vec<i32> = (lo.y..=hi.y - w + 1).filter(|&z0| fits(z0)).collect();
        let (&first, &last) = (candidates.first()?, candidates.last()?);
        let middle = (first + last) as f32 * 0.5;
        let want = middle + p.door_shift.clamp(-1.0, 1.0) * (last - first) as f32 * 0.5;
        let z0 = *candidates.iter().min_by(|a, b| (**a as f32 - want).abs().total_cmp(&(**b as f32 - want).abs()))?;
        let mut door = Vec::new();
        let mut step = Vec::new();
        for z in z0..z0 + w {
            let (rx, ox) = (room_x(z)?, outer_x(z)?);
            for x in rx + 1..=ox {
                door.push(IVec2::new(x, z));
            }
            step.push(IVec2::new(ox + 1, z));
        }
        Some(Plan { form: p.form, outer, wall, room, sections, radius, thick: p.thick, door, door_z: (z0, z0 + w - 1), step })
    }

    /// The x just outside the doorway.
    pub fn front(&self) -> i32 {
        self.step.iter().map(|q| q.x).max().unwrap()
    }

    /// The x of the room's edge behind the doorway.
    pub fn door_inner(&self) -> i32 {
        self.door.iter().map(|q| q.x).min().unwrap() - 1
    }

    /// The ways out of a wall column straight to the outside.
    pub fn outward(&self, q: IVec2) -> impl Iterator<Item = IVec2> + '_ {
        DIRS.into_iter().filter(move |d| self.outer.has(q) && !self.outer.has(q + *d))
    }

    /// Whether wall column `q` has the outside straight beyond it at `n`, room `thick`
    /// columns in, and wall between: a stretch of plain wall.
    pub fn plain_wall(&self, q: IVec2, n: IVec2) -> bool {
        self.wall.has(q) && !self.outer.has(q + n) && (1..self.thick).all(|k| self.wall.has(q - n * k)) && self.room.has(q - n * self.thick)
    }
}

/// A place along a wall for a bed or furniture, in columns (see [`super::Spot`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Spot2 {
    pub at: IVec2,
    pub along: IVec2,
    pub facing: IVec2,
    pub len: i32,
    pub wide: i32,
}

impl Spot2 {
    pub fn cells(&self) -> impl Iterator<Item = IVec2> + '_ {
        (0..self.len).flat_map(move |i| (0..self.wide).map(move |j| self.at + self.along * i + self.facing * j))
    }

    /// The columns just in front of it, where someone stands to use it.
    fn front(&self) -> impl Iterator<Item = IVec2> + '_ {
        (0..self.len).map(move |i| self.at + self.along * i + self.facing * self.wide)
    }
}

/// A fireplace in a wall and its chimney stack.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct Fireplace {
    /// The opening the fire burns in: the innermost wall column of the stack's middle.
    pub firebox: IVec2,
    /// Into the room.
    pub normal: IVec2,
    /// Every column of the stack, from the inside of the wall to one past its outside.
    pub stack: Vec<IVec2>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) enum HearthPlan {
    /// A hearth two by two in the floor, from this corner toward +X and +Z.
    Central(IVec2),
    Fireplace(Fireplace),
}

/// A window: the wall columns it is cut through.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct Window2 {
    /// The two outermost columns, in order along the wall.
    pub face: [IVec2; 2],
    /// Out of the building.
    pub normal: IVec2,
    /// Every column it goes through.
    pub through: Vec<IVec2>,
}

/// An open shed against a wall: the wall columns it stands against and the way out.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct LeanTo {
    pub along: Vec<IVec2>,
    pub normal: IVec2,
}

/// Where everything goes in a plan.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct Layout {
    pub hearth: HearthPlan,
    pub beds: Vec<Spot2>,
    pub furniture: Vec<Spot2>,
    pub windows: Vec<Window2>,
    pub lean_to: Option<LeanTo>,
    /// The way in from the doorway, kept clear of furniture.
    pub corridor: Mask,
}

/// What a layout is asked for.
#[derive(Clone, Copy, Debug)]
pub(super) struct Wants {
    pub chimney: bool,
    /// Faces a chimney or a lean-to goes on by preference (gable ends), as outward ways.
    pub gables: [Option<IVec2>; 2],
    pub windows: i32,
    pub sleepers: i32,
    pub furniture: i32,
    pub lean_to: bool,
    pub seed: u32,
}

pub(super) const BED_LEN: i32 = 5;
pub(super) const BED_WIDE: i32 = 2;

impl Layout {
    /// Lays out a plan, or None if it cannot sleep everyone asked for.
    pub fn new(plan: &Plan, w: &Wants) -> Option<Layout> {
        let (lo, hi) = (plan.outer.lo, plan.outer.hi);
        let jitter = |q: IVec2, salt: u32| unit(hash2(w.seed ^ salt, q.x, q.y));
        // The way in: three columns into the room behind the doorway, a column wider than it
        // each side.
        let mut corridor = Mask::new(lo, hi);
        let inner = plan.door_inner();
        for z in plan.door_z.0 - 1..=plan.door_z.1 + 1 {
            for x in inner - 2..=inner {
                if plan.room.has(IVec2::new(x, z)) {
                    corridor.set(IVec2::new(x, z), true);
                }
            }
        }
        let mut taken = corridor.clone();
        // The hearth: a fireplace on a plain stretch of wall away from the door, at the
        // middle of a gable end if it can; or else in the middle of the floor.
        let mut hearth = None;
        if w.chimney {
            let mut best: Option<(f32, Fireplace)> = None;
            for q in plan.wall.cells() {
                for n in plan.outward(q).collect::<Vec<_>>() {
                    if n == IVec2::X {
                        continue;
                    }
                    let t = IVec2::new(n.y, n.x);
                    if ![q - t, q, q + t].iter().all(|&c| plan.plain_wall(c, n)) {
                        continue;
                    }
                    let front = q - n * plan.thick;
                    if [front - t, front, front + t].iter().any(|c| corridor.has(*c)) {
                        continue;
                    }
                    // Toward the middle of the face, and on a gable end if there is one.
                    let along = |c: IVec2| c.x * t.x + c.y * t.y;
                    let run: Vec<i32> = plan.wall.cells().into_iter().filter(|&c| plan.plain_wall(c, n) && (c - q).dot(n) == 0).map(along).collect();
                    let mid = (*run.iter().min().unwrap() + *run.iter().max().unwrap()) as f32 * 0.5;
                    let gable = w.gables.contains(&Some(n));
                    let score = if gable { 100.0 } else { 0.0 } - (along(q) as f32 - mid).abs() + jitter(q, 0xf12e) * 0.5;
                    if best.as_ref().is_none_or(|(s, _)| score > *s) {
                        let mut stack = Vec::new();
                        for k in -1..=1 {
                            for d in -(plan.thick - 1)..=1 {
                                stack.push(q + t * k + n * d);
                            }
                        }
                        best = Some((score, Fireplace { firebox: q - n * (plan.thick - 1), normal: -n, stack }));
                    }
                }
            }
            if let Some((_, f)) = best {
                let front = f.firebox + f.normal;
                let t = IVec2::new(f.normal.y, f.normal.x);
                for k in -1..=1 {
                    for d in 0..2 {
                        taken.set(front + t * k + f.normal * d, true);
                    }
                }
                hearth = Some(HearthPlan::Fireplace(f));
            }
        }
        let hearth = match hearth {
            Some(h) => h,
            None => {
                let room = plan.room.cells();
                let centre = room.iter().fold(glam::Vec2::ZERO, |a, q| a + q.as_vec2()) / room.len() as f32;
                let block = |q: IVec2| [IVec2::ZERO, IVec2::X, IVec2::Y, IVec2::ONE].map(|d| q + d);
                let at = room
                    .iter()
                    .copied()
                    .filter(|&q| block(q).iter().all(|c| plan.room.has(*c) && !corridor.has(*c)))
                    .min_by(|a, b| {
                        let d = |q: IVec2| (q.as_vec2() + glam::Vec2::splat(0.5) - centre).length_squared();
                        d(*a).total_cmp(&d(*b)).then((a.y, a.x).cmp(&(b.y, b.x)))
                    })?;
                for z in at.y - 1..=at.y + 2 {
                    for x in at.x - 1..=at.x + 2 {
                        taken.set(IVec2::new(x, z), true);
                    }
                }
                HearthPlan::Central(at)
            }
        };
        let stack: Vec<IVec2> = match &hearth {
            HearthPlan::Fireplace(f) => f.stack.clone(),
            HearthPlan::Central(_) => Vec::new(),
        };
        // A lean-to against a gable end that has no chimney, clear of its corners.
        let mut lean_to = None;
        if w.lean_to && plan.form == Form::Rect {
            let faces: Vec<IVec2> = w.gables.iter().flatten().copied().filter(|&n| n != IVec2::X && !stack.iter().any(|&c| !plan.outer.has(c + n) && plan.outer.has(c))).collect();
            if let Some(&n) = faces.get((w.seed % 7) as usize % faces.len().max(1)) {
                let t = IVec2::new(n.y.abs(), n.x.abs());
                let mut along: Vec<IVec2> = plan.wall.cells().into_iter().filter(|&c| plan.plain_wall(c, n)).collect();
                along.sort_by_key(|c| c.dot(t));
                if along.len() > 6 {
                    along = along[2..along.len() - 2].to_vec();
                    lean_to = Some(LeanTo { along, normal: n });
                }
            }
        }
        // Windows, two columns wide, on plain wall away from corners, the chimney, the
        // doorway (and the door standing open against the wall beside it) and the lean-to.
        let mut windows: Vec<Window2> = Vec::new();
        let mut spots: Vec<(IVec2, IVec2, IVec2)> = Vec::new();
        for q in plan.wall.cells() {
            for n in plan.outward(q).collect::<Vec<_>>() {
                let t = IVec2::new(n.y.abs(), n.x.abs());
                if ![q - t, q, q + t, q + t * 2].iter().all(|&c| plan.plain_wall(c, n)) {
                    continue;
                }
                let near = |c: IVec2| stack.iter().any(|s| (*s - c).abs().max_element() <= 1);
                if near(q) || near(q + t) {
                    continue;
                }
                let w_door = plan.door_z.1 - plan.door_z.0 + 1;
                if n == IVec2::X && q.y + 1 >= plan.door_z.0 - w_door - 1 && q.y <= plan.door_z.1 + 1 {
                    continue;
                }
                if lean_to.as_ref().is_some_and(|l| l.normal == n) {
                    continue;
                }
                spots.push((q, n, t));
            }
        }
        spots.sort_by_key(|(q, n, _)| hash2(w.seed ^ 0x3d1e, q.x * 4 + n.x, q.y * 4 + n.y));
        for (q, n, t) in spots {
            if windows.len() as i32 >= w.windows {
                break;
            }
            if windows.iter().any(|o| (o.face[0] - q).abs().max_element() < 5) {
                continue;
            }
            let through = (0..plan.thick).flat_map(|k| [q - n * k, q + t - n * k]).collect();
            windows.push(Window2 { face: [q, q + t], normal: n, through });
        }
        // Beds, then furniture, along the walls: each where someone can still walk up to
        // it and to everything already placed, the farthest from the door first.
        let mut layout = Layout { hearth, beds: Vec::new(), furniture: Vec::new(), windows, lean_to, corridor };
        let fill = |len: i32, wide: i32, want: i32, layout: &mut Layout, taken: &mut Mask, beds: bool| -> i32 {
            let mut options = Vec::new();
            for q in plan.room.cells() {
                for n in DIRS {
                    if !plan.wall.has(q - n) {
                        continue;
                    }
                    for t in [IVec2::new(n.y, n.x), IVec2::new(-n.y, -n.x)] {
                        let s = Spot2 { at: q, along: t, facing: n, len, wide };
                        // Against the wall along most of its length; a round wall curves
                        // away from the ends of a bed, so there a bed only touches it.
                        let backed = (0..len).filter(|&i| plan.wall.has(q + t * i - n)).count() as i32;
                        let enough = if plan.form == Form::Round { backed * 3 >= len } else { backed * 2 > len };
                        if s.cells().all(|c| plan.room.has(c) && !taken.has(c)) && enough {
                            let far = (q.x - plan.front()).abs() + (q.y - (plan.door_z.0 + plan.door_z.1) / 2).abs();
                            options.push((far as f32 + jitter(q, 0xbed5 ^ (n.x * 3 + n.y + 7) as u32 ^ (t.x * 5 + t.y) as u32) * 3.0, s));
                        }
                    }
                }
            }
            options.sort_by(|a, b| b.0.total_cmp(&a.0).then((a.1.at.y, a.1.at.x).cmp(&(b.1.at.y, b.1.at.x))));
            let mut placed = 0;
            for (_, s) in options {
                if placed >= want {
                    break;
                }
                if s.cells().any(|c| taken.has(c)) {
                    continue;
                }
                let list = if beds { &mut layout.beds } else { &mut layout.furniture };
                list.push(s);
                if layout.reachable(plan) {
                    for c in s.cells() {
                        taken.set(c, true);
                    }
                    placed += 1;
                } else {
                    let list = if beds { &mut layout.beds } else { &mut layout.furniture };
                    list.pop();
                }
            }
            placed
        };
        if fill(BED_LEN, BED_WIDE, w.sleepers, &mut layout, &mut taken, true) < w.sleepers {
            return None;
        }
        fill(3, 1, w.furniture, &mut layout, &mut taken, false);
        Some(layout)
    }

    /// Whether a person can walk from the doorway to the front of every bed and piece of
    /// furniture and to the hearth, among them all: a body two columns square.
    pub fn reachable(&self, plan: &Plan) -> bool {
        let mut blocked = Mask::new(plan.outer.lo, plan.outer.hi);
        for s in self.beds.iter().chain(&self.furniture) {
            for c in s.cells() {
                blocked.set(c, true);
            }
        }
        if let HearthPlan::Central(at) = self.hearth {
            for d in [IVec2::ZERO, IVec2::X, IVec2::Y, IVec2::ONE] {
                blocked.set(at + d, true);
            }
        }
        let walk = |q: IVec2| (plan.room.has(q) || plan.door.contains(&q)) && !blocked.has(q);
        let fits = |q: IVec2| [IVec2::ZERO, IVec2::X, IVec2::Y, IVec2::ONE].iter().all(|d| walk(q + *d));
        let mut seen = Mask::new(plan.outer.lo - IVec2::ONE, plan.outer.hi);
        let mut reached = Mask::new(plan.outer.lo, plan.outer.hi);
        let mut queue = VecDeque::new();
        for &d in &plan.door {
            for o in [IVec2::ZERO, IVec2::NEG_X, IVec2::NEG_Y, IVec2::NEG_ONE] {
                if fits(d + o) && !seen.has(d + o) {
                    seen.set(d + o, true);
                    queue.push_back(d + o);
                }
            }
        }
        while let Some(q) = queue.pop_front() {
            for d in [IVec2::ZERO, IVec2::X, IVec2::Y, IVec2::ONE] {
                reached.set(q + d, true);
            }
            for d in DIRS {
                let n = q + d;
                if fits(n) && !seen.has(n) {
                    seen.set(n, true);
                    queue.push_back(n);
                }
            }
        }
        let spots_ok = self.beds.iter().chain(&self.furniture).all(|s| s.front().any(|c| reached.has(c)));
        let hearth_ok = match &self.hearth {
            HearthPlan::Central(at) => [IVec2::NEG_X, IVec2::new(2, 0), IVec2::NEG_Y, IVec2::new(0, 2)].iter().any(|d| reached.has(*at + *d)),
            HearthPlan::Fireplace(f) => reached.has(f.firebox + f.normal),
        };
        spots_ok && hearth_ok
    }
}
