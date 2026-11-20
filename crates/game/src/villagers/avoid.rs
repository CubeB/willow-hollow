//! Keeping out of each other's way (#206). Nothing here changes anyone's day: where
//! someone is still comes from their day and the clock alone, worked out afresh each
//! frame from where everyone is that moment, so that two looks at the same moment see the
//! same thing. A walker steps aside from the line their walk takes, round anyone at a stay
//! ahead of them (the legs of someone sitting and the length of someone lying taken in),
//! and to their right as they meet someone coming the other way, who steps to their own
//! right too; behind someone crossing their way; and, as everyone walks at one pace and
//! nobody overtakes, apart from anyone who falls in beside them or treads on their heels.
//! Each eases out as they come near and back again once by, as far as the room beside
//! the way allows: not into a
//! wall or onto what stands in a yard or on the green, not off the boards over water or
//! down a bank. Where there is no room (a doorway, a bridge, a gate) they keep to the
//! way as they always have.
//!
//! Only those walking in sight are moved, each looked at against those near them, found
//! in a small hash of squares; and the room beside a walk is measured a little at a time,
//! the first time someone on it has to step aside there.

use std::collections::{HashMap, HashSet};

use glam::{IVec2, Vec2, Vec3};
use worldgen::{features::fittings::Fitting, settlements::layout::Layout};

use super::{
    Placed, WALK_SIGHT,
    day::Plan,
    lanes::{Walk, flat},
    town::{Spot, Town},
};
use crate::character::{self, Task};

/// Half the breadth of someone across the ground, in voxels: a little over half their
/// shoulders.
pub const BODY: f32 = 0.6;
/// The gap kept between two people passing, besides, in voxels.
pub const GAP: f32 = 0.35;
/// The furthest anyone steps aside from their way, in voxels; and out in the land, where
/// hedges and ditches run beside the ways and only the ways were looked for, the most.
const FURTHEST: f32 = 2.0;
const OUT_IN_LAND: f32 = 0.7;
/// How far ahead a walker starts to step aside for someone still, and for someone coming
/// the other way (who comes on twice as fast), and how far past them they are back on
/// their way, in voxels along it.
const AHEAD: f32 = 5.0;
const MEETING: f32 = 9.0;
const PAST: f32 = 4.0;
const MET: f32 = 8.0;
/// How far aside a walker may be for each voxel they are from either end of their walk,
/// so that they set off from one spot and come to the next exactly; and for each voxel
/// they are from where the way narrows, so that they ease back onto it before.
const ENDS: f32 = 0.7;
const NARROWING: f32 = 0.5;
/// Over how many voxels from either end of their walk someone setting off or coming to a
/// spot goes from being as they were there to walking, for those passing them; and over
/// how many someone coming out of a door or going in comes to be there to be stepped
/// round, or stops being.
const SETTLE: f32 = 4.0;
const SHAPE: f32 = 1.5;
/// Two walking abreast or meeting share the stepping aside; so that they do not both step
/// the same way, those meeting keep to their right unless they are already well to the
/// left of each other (this far, in voxels), and of two side by side the one with the
/// lower number keeps to their right of the line the other walks.
const KEEP_RIGHT: f32 = 0.5;
const TIE: f32 = 0.15;
/// How far behind and ahead along a walk the way it goes is taken from, in voxels: for
/// which way a walker faces, and, far enough that a corner is turned over a few steps,
/// for which way they step aside and which way others see them going.
const TURN: f32 = 1.0;
const COURSE: f32 = 4.0;
/// How far apart the room beside a walk is measured along it, and in steps aside from it.
const ROOM_STEP: f32 = 1.0;
const PROBE: f32 = 0.5;
/// How far ahead along the way a walker looks at those about it, in voxels: past anyone
/// they step aside for.
const LOOK: f32 = MEETING + 3.0;
/// The side of the squares figures are filed in for finding those near, in voxels: more
/// than anyone is looked at from.
const SQUARE: f32 = 14.0;
/// The side of the squares what stands about a settlement is filed in, in columns.
const CELL: i32 = 16;
/// How long a bed is from its head, and half its breadth, in voxels.
const BED: f32 = 4.5;
const BED_HALF: f32 = 1.0;

/// The walks being drawn, by who is walking and from which of their stays.
pub type Routes = HashMap<(usize, usize), Route>;

/// A walk being drawn, with the room beside it.
#[derive(Clone, Debug)]
pub struct Route {
    pub walk: Walk,
    /// The building it goes about in, from room to room, or None out of doors.
    inside: Option<usize>,
    /// How far there is to step aside to the right of it and to the left (looking along
    /// it), every [`ROOM_STEP`] along it; each measured the first time it is needed (none
    /// yet below 0).
    room: Vec<[f32; 2]>,
}

/// How far along a walk the room beside it is looked at from any point, in voxels.
const REACH: f32 = FURTHEST / NARROWING + BODY;

impl Route {
    pub fn new(walk: Walk, inside: Option<usize>) -> Self {
        Route { walk, inside, room: Vec::new() }
    }

    /// Measures the room beside the walk in `town` from `from` to `to` voxels along it,
    /// where it has not been: as much as [`Route::room_at`] looks at there.
    pub(super) fn measure(&mut self, town: &Town, from: f32, to: f32) {
        let n = (self.walk.length() / ROOM_STEP).ceil() as usize + 1;
        if self.room.len() != n {
            self.room = vec![[-1.0; 2]; n];
        }
        let middle = flat(town.centre);
        let k0 = ((from / ROOM_STEP).floor() as isize - 1).clamp(0, n as isize - 1) as usize;
        let k1 = ((to / ROOM_STEP).ceil() as isize + 2).clamp(0, n as isize - 1) as usize;
        for k in k0..=k1 {
            if self.room[k][0] >= 0.0 {
                continue;
            }
            let d = k as f32 * ROOM_STEP;
            let base = flat(self.walk.at(d).0);
            let side = self.walk.heading(d, COURSE).perp();
            let most = if self.inside.is_none() && base.distance(middle) > town.radius + 8.0 { OUT_IN_LAND } else { FURTHEST };
            self.room[k] = [1.0, -1.0].map(|sign| {
                let mut free = 0.0;
                let mut off = PROBE;
                while off <= most + 1e-3 {
                    if town.blocked(base + side * sign * off, side * sign, base, self.inside) {
                        break;
                    }
                    free = off;
                    off += PROBE;
                }
                free
            });
        }
    }

    /// The room to the right and the left `d` along the walk, where measured: what there
    /// is within a body's breadth along it, and less where the way narrows within a few
    /// steps, so that nobody stepping aside has to step back all at once.
    pub(super) fn room_at(&self, d: f32) -> [f32; 2] {
        let n = self.room.len();
        let mut room = [if n == 0 { 0.0 } else { FURTHEST }; 2];
        let k0 = ((d - REACH) / ROOM_STEP).floor().clamp(0.0, n as f32) as usize;
        let k1 = ((d + REACH) / ROOM_STEP).ceil().clamp(0.0, n as f32 - 1.0) as usize;
        for k in k0..=k1.min(n.saturating_sub(1)) {
            let off = ((k as f32 * ROOM_STEP - d).abs() - BODY).max(0.0) * NARROWING;
            for j in 0..2 {
                room[j] = room[j].min(self.room[k][j].max(0.0) + off);
            }
        }
        room
    }
}

/// What stands about a settlement that nobody steps aside onto, filed by squares: its
/// buildings, what stands in its yards and on its green, the posts of its gates, and the
/// ground beside its ways that is water or a bank.
#[derive(Clone, Debug, Default)]
pub struct Ground {
    /// Each thing's corners across the ground.
    boxes: Vec<(Vec2, Vec2)>,
    /// Each gate's corners, the middle of its way through, the way across that and half
    /// its breadth.
    gates: Vec<(Vec2, Vec2, Vec2, Vec2, f32)>,
    /// What reaches into each square of [`CELL`] columns.
    cells: HashMap<(i32, i32), Vec<Item>>,
    /// The columns beside the ways that are water, or a step up or down from them.
    rough: HashSet<(i32, i32)>,
}

#[derive(Clone, Copy, Debug)]
enum Item {
    Walls(u32),
    Box(u32),
    Gate(u32),
}

impl Ground {
    /// What stands about `town`, laid out as `layout`, with its fittings `things` (as
    /// `Town::build_in` reads them), its village tree's trunk if one was planted, and the
    /// columns found `rough` beside its ways.
    pub fn new(town: &Town, layout: &Layout, things: &[(Fitting, Vec3, Vec2, Vec2)], trunk: Option<Vec2>, rough: HashSet<(i32, i32)>) -> Ground {
        let mut g = Ground { rough, ..Default::default() };
        let file = |cells: &mut HashMap<(i32, i32), Vec<Item>>, lo: Vec2, hi: Vec2, item: Item| {
            let (lo, hi) = ((lo - BODY - 1.0).floor().as_ivec2().div_euclid(IVec2::splat(CELL)), (hi + BODY + 1.0).floor().as_ivec2().div_euclid(IVec2::splat(CELL)));
            for z in lo.y..=hi.y {
                for x in lo.x..=hi.x {
                    cells.entry((x, z)).or_default().push(item);
                }
            }
        };
        for (i, b) in town.plots.iter().enumerate() {
            file(&mut g.cells, b.lo.as_vec2(), b.hi.as_vec2() + 1.0, Item::Walls(i as u32));
        }
        let mut boxes: Vec<(Vec2, Vec2)> = things.iter().map(|t| (flat(t.1) - t.3, flat(t.1) + t.3)).collect();
        boxes.extend(trunk.map(|t| (t - 1.0, t + 1.0)));
        // A festival's lantern posts, and an ancient town's old wall.
        boxes.extend(town.posts.iter().map(|p| (flat(p.0) - 0.5, flat(p.0) + 0.5)));
        boxes.extend(layout.wall.iter().flatten().map(|c| (c.as_vec2(), c.as_vec2() + 1.0)));
        if let Some((pond, r)) = layout.park.as_ref().and_then(|p| p.pond) {
            let at = Vec2::new(pond.x as f32 + 0.5, pond.z as f32 + 0.5);
            boxes.push((at - r as f32, at + r as f32));
        }
        for (i, &(lo, hi)) in boxes.iter().enumerate() {
            file(&mut g.cells, lo, hi, Item::Box(i as u32));
        }
        g.boxes = boxes;
        for (i, gate) in layout.gates.iter().enumerate() {
            let (lo, hi) = (gate.lo.as_vec2(), gate.hi.as_vec2() + 1.0);
            let mid = Vec2::new(gate.origin.x as f32 + 0.5, gate.origin.z as f32 + 0.5);
            let across = gate.out.as_vec2().normalize_or(Vec2::X).perp();
            g.gates.push((lo, hi, mid, across, gate.passage as f32 * 0.5));
            file(&mut g.cells, lo, hi, Item::Gate(i as u32));
        }
        g
    }
}

/// The columns beside `ways` (lines of points across the ground, continent-local) that
/// nobody steps aside onto: water with no boards over it, or a step of more than a voxel
/// up or down from the way. `ground` gives the feet's level on a column and whether it is
/// such water; each column is looked at once.
pub fn rough_beside(ways: &[Vec<Vec2>], ground: &mut dyn FnMut(Vec2) -> (f32, bool)) -> HashSet<(i32, i32)> {
    let mut seen: HashMap<(i32, i32), (f32, bool)> = HashMap::new();
    let mut look = |p: Vec2| -> (f32, bool) {
        let c = (p.x.floor() as i32, p.y.floor() as i32);
        *seen.entry(c).or_insert_with(|| ground(Vec2::new(c.0 as f32 + 0.5, c.1 as f32 + 0.5)))
    };
    let mut rough = HashSet::new();
    for way in ways {
        for w in way.windows(2) {
            let along = (w[1] - w[0]).normalize_or(Vec2::X);
            let n = (w[0].distance(w[1]) / ROOM_STEP).ceil().max(1.0) as usize;
            for k in 0..n {
                let p = w[0].lerp(w[1], k as f32 / n as f32);
                let (y, _) = look(p);
                for off in [1.0, 2.0, 3.0] {
                    for sign in [1.0, -1.0] {
                        let q = p + along.perp() * sign * off;
                        let (feet, wet) = look(q);
                        if wet || (feet - y).abs() > 1.2 {
                            rough.insert((q.x.floor() as i32, q.y.floor() as i32));
                        }
                    }
                }
            }
        }
    }
    rough
}

impl Town {
    /// Whether someone may not stand with their middle at `p` (continent-local), stepping
    /// aside toward `toward` from `from` on their way: in building `inside` within its
    /// walls and off its beds, out of doors clear of what stands about the settlement (see
    /// [`Ground`]), but for what their way itself goes over.
    pub fn blocked(&self, p: Vec2, toward: Vec2, from: Vec2, inside: Option<usize>) -> bool {
        if let Some(i) = inside {
            let Some(b) = self.plots.get(i) else { return true };
            // A wall's breadth and half a body within the walls.
            return !b.within(p, -(1.0 + BODY)) || b.beds.iter().any(|&(head, way)| along_from(p, flat(head), flat(head) + way * BED) < BED_HALF + BODY);
        }
        let g = &self.ground;
        let c = IVec2::new(p.x.floor() as i32, p.y.floor() as i32).div_euclid(IVec2::splat(CELL));
        for item in g.cells.get(&(c.x, c.y)).into_iter().flatten() {
            let hit = match *item {
                Item::Walls(i) => self.plots[i as usize].within(p, BODY),
                Item::Box(i) => {
                    let (lo, hi) = g.boxes[i as usize];
                    let on = |p: Vec2| p.cmpgt(lo - BODY).all() && p.cmplt(hi + BODY).all();
                    on(p) && !on(from)
                }
                Item::Gate(i) => {
                    let (lo, hi, mid, across, half) = g.gates[i as usize];
                    p.cmpgt(lo).all() && p.cmplt(hi).all() && (p - mid).dot(across).abs() > half - BODY
                }
            };
            if hit {
                return true;
            }
        }
        // The ground under them, and under the edge of them toward where they step.
        let edge = p + toward * BODY;
        g.rough.contains(&(p.x.floor() as i32, p.y.floor() as i32)) || g.rough.contains(&(edge.x.floor() as i32, edge.y.floor() as i32))
    }
}

/// How far `p` is from the line from `a` to `b`, across the ground.
fn along_from(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
    (a + ab * t).distance(p)
}

/// Where someone is in their day as the frame found them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum State {
    /// At their `n`th stay.
    At(usize),
    /// On the way from their `n`th stay to the next, this far along the walk, in voxels.
    Going(usize, f32),
    /// Held where they are: stopped by the player to talk.
    Held,
}

/// Someone as the others step round them: where they stay, as the line their body lies
/// along across the ground (out a body's breadth from it), and for someone walking where
/// they are and which way they go (taken over a few steps of their walk), how far under
/// way they are (from 0 at either end of their walk, where they are as they were at the
/// stay, to 1) and how far they are there to be stepped round at all (coming out of a
/// door, from 0 at the doorway); and the level of their feet and the building they are
/// in. Someone coming to their spot or setting off from it is stepped round both as they
/// go and as they will be or were there, the one giving way to the other as they come.
#[derive(Clone, Copy, Debug)]
struct Body {
    n: usize,
    spot: (Vec2, Vec2),
    at: Vec2,
    heading: Vec2,
    moving: f32,
    present: f32,
    y: f32,
    inside: Option<usize>,
}

/// What [`keep_off`] keeps between frames, so as not to allocate afresh each one.
#[derive(Default)]
pub struct Scratch {
    bodies: Vec<Body>,
    /// Each body's square and its index among them, in order.
    filed: Vec<((i32, i32), u32)>,
    near: Vec<u32>,
    still: Vec<Still>,
}

/// The line someone at `spot` lies along across the ground and how far out from it they
/// reach, for someone `lying` voxels long abed: the legs out before them seated, and the
/// length of them lying.
pub(super) fn outline(spot: &Spot, lying: f32) -> (Vec2, Vec2, f32) {
    let at = flat(spot.at);
    let way = Vec2::from_angle(spot.facing);
    match spot.task {
        Task::Lie => (at, at + way * lying * 0.92, BODY),
        // On a stool or a seat, the knees and feet out before them.
        Task::Sit | Task::Milk | Task::Sketch => (at - way * 0.1, at + way * 0.8, BODY),
        // On the ground, the legs stretched out.
        Task::SitLow | Task::Row => (at, at + way * 1.8, BODY),
        // Bent to work, a tool swung before them.
        Task::Chop | Task::Hoe | Task::Hammer | Task::Sweep | Task::Reap | Task::Saw | Task::Tend | Task::Sow | Task::Feed => (at, at + way * 0.6, BODY),
        _ => (at, at, BODY),
    }
}

/// How far across the way to ease a change of side over, for someone coming across it
/// `goes` fast (the sine of the angle their way makes with the walker's): from `least`,
/// wider the faster they come across, so that it takes them about as long however fast;
/// but growing by no more than two thirds again for each tenth faster, so that someone
/// turning, as they do at a corner of their walk or going back on themselves, does not
/// swing it all at once.
fn easing(goes: f32, least: f32) -> f32 {
    let goes = goes.abs();
    (goes * 5.0).max(least).min(least * (goes * 5.0).exp())
}

/// Eases from 0 to 1 as `x` goes from 0 to 1.
fn ramp(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// How much to step aside for someone `rel` along the way ahead (behind, below 0): wholly
/// while within `reach` of being alongside them, easing in over `ahead` before and out
/// over `past` after.
fn bump(rel: f32, reach: f32, ahead: f32, past: f32) -> f32 {
    if rel > reach {
        1.0 - ramp((rel - reach) / ahead)
    } else if rel < -reach {
        1.0 - ramp((-rel - reach) / past)
    } else {
        1.0
    }
}

fn square(p: Vec2) -> (i32, i32) {
    ((p.x / SQUARE).floor() as i32, (p.y / SQUARE).floor() as i32)
}

/// Steps everyone walking in sight of `eye` in `town` aside from those near them, as
/// they are placed in `figs` (by resident, with where they are in their day by `plans`),
/// along their `routes`: moves their feet off the way and turns them the way they go.
pub fn keep_off(town: &Town, plans: &[Plan], eye: Vec3, routes: &mut Routes, figs: &mut [Option<(Placed, State)>], scratch: &mut Scratch) {
    let sight = worldgen::scale::m(WALK_SIGHT);
    let Scratch { bodies, filed, near, still: stills } = scratch;
    bodies.clear();
    filed.clear();
    let eye2 = flat(eye);
    for (n, fig) in figs.iter().enumerate() {
        let Some((p, state)) = fig else { continue };
        if flat(p.0).distance(eye2) > sight + SQUARE * 2.0 {
            continue;
        }
        let Some(r) = town.residents.get(n) else { continue };
        let lying = character::lying_length(&r.person.look.build);
        let here = flat(p.0);
        let still = |a: Vec2, b: Vec2| Body { n, spot: (a, b), at: here, heading: Vec2::ZERO, moving: 0.0, present: 1.0, y: p.0.y, inside: p.6 };
        let body = match *state {
            State::At(i) => {
                let Some(s) = plans[n].stays.get(i) else { continue };
                let (a, b, _) = outline(&s.spot, lying);
                still(a, b)
            }
            State::Held => still(here, here),
            State::Going(i, d) => {
                let Some(route) = routes.get(&(n, i)) else { continue };
                let length = route.walk.length();
                // Near either end, they are much as they were at the stay or will be at
                // the next: seated still, or abed; or, coming out of a door or going in,
                // only just there to be stepped round. Halfway along a short walk they
                // are wholly walking.
                let off = d.min(length - d);
                let Some(s) = plans[n].stays.get(if d < length * 0.5 { i } else { i + 1 }) else { continue };
                let (moving, present, spot) = match s.spot.inside == p.6 {
                    true => {
                        let o = outline(&s.spot, lying);
                        (ramp(off / SETTLE.min(length * 0.5).max(1e-3)), 1.0, (o.0, o.1))
                    }
                    false => (1.0, ramp(off / SHAPE.min(length * 0.5).max(1e-3)), (here, here)),
                };
                Body { n, spot, at: here, heading: route.walk.heading(d, COURSE), moving, present, y: p.0.y, inside: p.6 }
            }
        };
        if body.present > 0.0 {
            filed.push((square(body.at), bodies.len() as u32));
            bodies.push(body);
        }
    }
    filed.sort_unstable();

    for (n, fig) in figs.iter_mut().enumerate() {
        let Some((p, State::Going(i, d))) = fig else { continue };
        let d = *d;
        let Some(route) = routes.get_mut(&(n, *i)) else { continue };
        let heading = route.walk.heading(d, TURN);
        p.1 = heading.y.atan2(heading.x);
        let length = route.walk.length();
        let ends = |d: f32| d.min(length - d).max(0.0) * ENDS;
        if ends(d) <= 0.0 {
            continue;
        }
        // Those near enough to matter.
        let here = flat(p.0);
        let reach = LOOK + BODY * 2.0 + GAP + 1.0;
        near.clear();
        let (sx, sz) = square(here);
        for dz in -1..=1 {
            for dx in -1..=1 {
                let key = (sx + dx, sz + dz);
                let from = filed.partition_point(|f| f.0 < key);
                for f in filed[from..].iter().take_while(|f| f.0 == key) {
                    let b = &bodies[f.1 as usize];
                    let far = b.at.distance(here).min(along_from(here, b.spot.0, b.spot.1));
                    if b.n != n && b.inside == p.6 && (b.y - p.0.y).abs() < 3.0 && far < reach {
                        near.push(f.1);
                    }
                }
            }
        }
        if near.is_empty() {
            continue;
        }
        // As much of the room beside the way as is looked at from here.
        route.measure(town, d - MET - 3.0 - REACH, d + LOOK + REACH);
        let route = &*route;
        let side = |d: f32| route.walk.heading(d, COURSE).perp();
        let s = aside(route, n, d, near, bodies, stills).clamp(-ends(d), ends(d));
        if s == 0.0 {
            continue;
        }
        let way = side(d) * s;
        // Turned the way the stepping aside takes them: toward where they will be a step on.
        let on = (d + 0.6).min(length);
        let next = flat(route.walk.at(on).0) + side(on) * aside(route, n, on, near, bodies, stills).clamp(-ends(on), ends(on));
        let go = next - (here + way);
        if go.length_squared() > 1e-4 {
            p.1 = go.y.atan2(go.x);
        }
        p.0 += Vec3::new(way.x, 0.0, way.y);
    }
}

/// Where the line from `a` to `b` across the ground is beside `walk`, from `from` to `to`
/// along it, as seen from `d` along it: for each point the walk passes them (see
/// [`Walk::passing`]), how far along it they are from `d` (beyond the end of the walk, as
/// far again as they are beyond it), how far along it that is, how far to the right of it
/// they are (to the left, below 0), how far they reach along it and across it, and the way
/// it goes there. Each moves on as the walker does, and comes into the stretch looked at
/// or goes out of it too far off to matter.
fn beside(walk: &Walk, d: f32, from: f32, to: f32, a: Vec2, b: Vec2, mut each: impl FnMut(f32, f32, f32, f32, f32, Vec2)) {
    let c = (a + b) * 0.5;
    let half = (b - a) * 0.5;
    let length = walk.length();
    walk.passing(c, from, to, |dc, foot| {
        let t = walk.heading(dc, COURSE);
        // Off either end, as far on as they are.
        let past = (c - foot).dot(t);
        let beyond = if dc >= length - 1e-3 { past.max(0.0) } else if dc <= 1e-3 { past.min(0.0) } else { 0.0 };
        each(dc - d + beyond, dc, (c - foot).dot(t.perp()), half.dot(t).abs(), half.dot(t.perp()).abs(), t);
    });
}

/// Someone still, or where someone coming to their spot or setting off will be or was, as
/// a walker sees them: where along the way they are, from how far to the left of the way
/// to how far to the right a walker has to keep clear of them, how much they are there to
/// be kept clear of (by how still they are and how near along the way), and how deep they
/// stand in it.
#[derive(Clone, Copy, Debug)]
struct Still {
    dc: f32,
    lo: f32,
    hi: f32,
    weight: f32,
    depth: f32,
}

/// How far resident `n`, `d` along `route`, steps aside from it for the bodies `near` (by
/// index in `bodies`): to the right of it above 0, to the left below, as far as the room
/// beside it allows. `still` is for keeping those still in, between calls.
fn aside(route: &Route, n: usize, d: f32, near: &[u32], bodies: &[Body], still: &mut Vec<Still>) -> f32 {
    let walk = &route.walk;
    let (from, to) = (d - MET - 3.0, d + LOOK);
    // Room for both and the gap between.
    let clear = BODY * 2.0 + GAP;
    // How deep someone stands in the way, from `lo` to the left of it to `hi` to the right:
    // the stepping aside grows from nothing as they come into it, over `soft`.
    let depth = |lo: f32, hi: f32, soft: f32| ramp((-lo).min(hi) / soft);
    // The most each way any of them asks for: one to the right and one to the left of the
    // way ask for no more than to go between them.
    let (mut right, mut left) = (0.0f32, 0.0f32);
    let mut ask = |s: f32| {
        if s > 0.0 {
            right = right.max(s);
        } else {
            left = left.min(s);
        }
    };

    // Those still, or as they will be or were at their spot, beside each part of the way
    // that passes them.
    still.clear();
    for &k in near {
        let o = &bodies[k as usize];
        let there = (1.0 - o.moving) * o.present;
        if there <= 0.0 {
            continue;
        }
        beside(walk, d, from, to, o.spot.0, o.spot.1, |rel, dc, lat, h_along, h_across, _| {
            let weight = there * bump(rel, h_along + clear, AHEAD, PAST);
            let (lo, hi) = (lat - h_across - clear, lat + h_across + clear);
            if weight > 0.0 {
                still.push(Still { dc, lo, hi, weight, depth: depth(lo, hi, 0.3) });
            }
        });
    }
    // Round each in the way on the side away from them, unless only the other side has
    // room; dead ahead, on the side the way turns toward beyond them, cutting the corner as
    // anyone would, or else to the right. Only where they are settles which side, so that
    // it holds as the walker comes up to them; where two stand too close together to go
    // between, the walker may be asked round each a different way, and go between all the
    // same.
    for o in still.iter() {
        if o.depth <= 0.0 {
            continue;
        }
        let (lo, hi) = (o.lo, o.hi);
        let lat = (lo + hi) * 0.5;
        let room = route.room_at(o.dc);
        let short = |need: f32, room: f32| (need - room).max(0.0);
        let (r_short, l_short) = (short(hi, room[0]), short(-lo, room[1]));
        let turn = walk.heading(o.dc + COURSE, COURSE).dot(walk.heading(o.dc - COURSE, COURSE).perp());
        let mut to_right = if lat.abs() > TIE { lat < 0.0 } else { turn > -0.2 };
        if to_right && l_short + 0.05 < r_short {
            to_right = false;
        } else if !to_right && r_short + 0.05 < l_short {
            to_right = true;
        }
        ask(o.weight * o.depth * if to_right { hi } else { lo });
    }

    // Someone walking: as they meet, each to their right of the other; abreast, apart;
    // crossing, behind them. Which side changes as they come across the way, eased over
    // more of it the faster they come across, so that nobody leaps from one side to the
    // other.
    for &k in near {
        let o = &bodies[k as usize];
        let moving = o.moving * o.present;
        if moving <= 0.0 {
            continue;
        }
        beside(walk, d, from, to, o.at, o.at, |rel, _, lat, _, _, t| {
            let (lo, hi) = (lat - clear, lat + clear);
            let goes = o.heading.dot(t.perp());
            let soft = easing(goes, 0.3);
            let deep = depth(lo, hi, soft);
            if deep <= 0.0 {
                return;
            }
            let cos = o.heading.dot(t);
            let meeting = ramp((-cos - 0.1) / 0.6);
            let following = ramp((cos - 0.1) / 0.6);
            let crossing = 1.0 - meeting - following;
            if meeting > 0.0 {
                // Both step to their right, half each, unless the other is already well
                // over to the right, when both step left; easing through the turn between.
                let w = bump(rel, clear, MEETING, MET);
                let by = ramp((lat - KEEP_RIGHT).abs() / soft);
                let s = if lat < KEEP_RIGHT { ((clear + lat) * 0.5).max(0.0) } else { -((clear - lat) * 0.5).max(0.0) };
                ask(moving * meeting * w * by * deep * s);
            }
            if following > 0.0 {
                // Going the same way at the same pace, nobody overtakes: those abreast, or
                // near enough to tread on each other's heels, step apart, half each.
                let w = 1.0 - ramp((rel.abs() - BODY * 2.0) / 0.6);
                let tie = if n < o.n { TIE } else { -TIE };
                let lean = lat + tie;
                let by = ramp(lean.abs() / easing(goes, TIE));
                let s = if lean > 0.0 { -((clear - lat) * 0.5).max(0.0) } else { ((clear + lat) * 0.5).max(0.0) };
                ask(moving * following * w * by * deep * s);
            }
            if crossing > 0.0 {
                // Behind them, the side they come from, while they are still near the way.
                let w = bump(rel, clear, AHEAD, PAST);
                let (s, back) = if goes > 0.0 { (lo, -lat) } else { (hi, lat) };
                let near_way = ramp((clear + 3.0 - back) / 3.0);
                ask(moving * crossing * w * near_way * deep * s);
            }
        });
    }

    let room = route.room_at(d);
    (right + left).clamp(-room[1], room[0])
}
