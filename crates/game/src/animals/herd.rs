//! A herd and the beasts in it: grazing, wandering, drifting slowly as a group, and
//! backing away from the player.
//!
//! Each beast goes its own way within reach of the herd's middle, which drifts slowly
//! over the herd's range, or mills about inside its pen. Every step is checked against
//! the ground read from the voxels (`ground.rs`): no beast walks into water deeper than
//! it wades, up a step higher than it climbs, or under anything lower than itself. The
//! wild keep their distance: within a kind's flight distance of the player the herd
//! turns and goes, the nearest at a run, and stops to look back once clear; after a
//! while with no one near they settle to grazing again. Nothing here ever comes at the
//! player. The kept only step out of the way.

use std::{collections::HashSet, sync::Arc};

use glam::{DVec2, DVec3};
use worldgen::{
    fauna::{Habits, Kind, RANGE, Site},
    noise::{hash2, unit},
    scale::{VOXELS_PER_METRE, m64},
};

use super::{
    ground::{Cell, Ground, ROOM},
    model::{Gait, Look, Pose, Role, Shape},
};

/// A pen, a coop's yard or a pasture: the fence line round it, and its gate.
#[derive(Clone, Debug, PartialEq)]
pub struct Pen {
    /// The fence, world voxel columns (x, z), corner to corner round it; the last
    /// corner joins the first.
    pub fence: Vec<DVec2>,
    /// Where the gate is: the side of the fence it is in (from corner k to the next) and
    /// how far along it, 0 to 1. It is shut: until someone drives the animals out
    /// through it, they treat it as fence.
    pub gate: Option<(usize, f64)>,
}

impl Pen {
    /// A rectangular pen about `middle`, `half` voxels from the middle to each side,
    /// turned `turn` radians, the gate in the middle of the side facing `gate_toward`
    /// (if given).
    pub fn rect(middle: DVec2, half: DVec2, turn: f64, gate_toward: Option<DVec2>) -> Pen {
        let (s, c) = turn.sin_cos();
        let corner = |x: f64, z: f64| middle + DVec2::new(x * c - z * s, x * s + z * c);
        let fence = vec![corner(-half.x, -half.y), corner(half.x, -half.y), corner(half.x, half.y), corner(-half.x, half.y)];
        let mut pen = Pen { fence, gate: None };
        if let Some(to) = gate_toward {
            let side = (0..4).min_by(|&a, &b| pen.side_middle(a).distance(to).total_cmp(&pen.side_middle(b).distance(to)));
            pen.gate = side.map(|k| (k, 0.5));
        }
        pen
    }

    fn side(&self, k: usize) -> (DVec2, DVec2) {
        (self.fence[k], self.fence[(k + 1) % self.fence.len()])
    }

    fn side_middle(&self, k: usize) -> DVec2 {
        let (a, b) = self.side(k);
        (a + b) * 0.5
    }

    /// Where the gate is.
    pub fn gate_at(&self) -> Option<DVec2> {
        self.gate.filter(|g| g.0 < self.fence.len()).map(|(k, t)| {
            let (a, b) = self.side(k);
            a + (b - a) * t
        })
    }

    /// Whether `p` is inside the fence.
    pub fn inside(&self, p: DVec2) -> bool {
        let n = self.fence.len();
        let mut inside = false;
        for k in 0..n {
            let (a, b) = self.side(k);
            if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
                inside = !inside;
            }
        }
        inside
    }

    /// How far `p` is from the nearest stretch of fence.
    pub fn clearance(&self, p: DVec2) -> f64 {
        (0..self.fence.len())
            .map(|k| {
                let (a, b) = self.side(k);
                let ab = b - a;
                let t = ((p - a).dot(ab) / ab.length_squared().max(1e-9)).clamp(0.0, 1.0);
                p.distance(a + ab * t)
            })
            .fold(f64::MAX, f64::min)
    }

    /// Inside, with at least `margin` to spare from the fence.
    pub fn holds(&self, p: DVec2, margin: f64) -> bool {
        self.inside(p) && self.clearance(p) >= margin
    }

    pub fn bounds(&self) -> (DVec2, DVec2) {
        let lo = self.fence.iter().fold(DVec2::MAX, |a, &b| a.min(b));
        let hi = self.fence.iter().fold(DVec2::MIN, |a, &b| a.max(b));
        (lo, hi)
    }

    pub fn middle(&self) -> DVec2 {
        self.fence.iter().fold(DVec2::ZERO, |a, &b| a + b) / self.fence.len().max(1) as f64
    }

    /// The pen's area, in square metres.
    pub fn area(&self) -> f64 {
        let n = self.fence.len();
        let twice: f64 = (0..n).map(|k| self.fence[k].perp_dot(self.fence[(k + 1) % n])).sum();
        twice.abs() * 0.5 / (VOXELS_PER_METRE as f64).powi(2)
    }
}

/// Where a herd may go.
#[derive(Clone, Debug, PartialEq)]
pub enum Fold {
    /// Anywhere within `reach` voxels of home, as far as the ground goes.
    Range { home: DVec2, reach: f64 },
    /// Inside a fence.
    Pen(Pen),
}

impl Fold {
    /// The point nearest `p` that the herd's middle may go to.
    fn keep(&self, p: DVec2, margin: f64) -> DVec2 {
        match self {
            Fold::Range { home, reach } => {
                let off = p - *home;
                if off.length() > *reach { *home + off.normalize_or_zero() * *reach } else { p }
            }
            Fold::Pen(pen) => {
                if pen.holds(p, margin) {
                    return p;
                }
                // In toward the middle until it is well inside.
                let m = pen.middle();
                (1..=10).map(|k| p + (m - p) * (k as f64 / 10.0)).find(|&q| pen.holds(q, margin)).unwrap_or(m)
            }
        }
    }

    pub fn middle(&self) -> DVec2 {
        match self {
            Fold::Range { home, .. } => *home,
            Fold::Pen(pen) => pen.middle(),
        }
    }
}

/// What a beast is about.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Act {
    /// Head down, cropping the grass.
    Graze,
    /// Standing, looking about.
    Stand,
    /// Going to a spot near the herd's middle.
    Walk,
    /// Head up, watching the player.
    Watch,
    /// Going away from the player.
    Flee,
}

/// One animal.
#[derive(Clone, Debug)]
pub struct Beast {
    pub look: Look,
    /// Where its feet are, in world voxels; the height eases to the ground under it.
    pub pos: DVec3,
    /// Which way it faces, as a camera's yaw.
    pub facing: f32,
    pub pose: Pose,
    pub act: Act,
    /// How fast it is going, metres a second.
    pub speed: f32,
    goal: Option<DVec2>,
    timer: f32,
    clock: f32,
    dice: u32,
    /// Its own pace, as a share of its kind's.
    pace: f32,
}

impl Beast {
    fn new(look: Look, pos: DVec3, facing: f32, seed: u32) -> Self {
        let mut b = Beast {
            look,
            pos,
            facing,
            pose: Pose::default(),
            act: Act::Graze,
            speed: 0.0,
            goal: None,
            timer: 0.0,
            clock: 0.0,
            dice: seed | 1,
            pace: 0.0,
        };
        b.pace = b.range(0.85, 1.15);
        b.clock = b.range(0.0, 60.0);
        b.timer = b.range(1.0, 10.0);
        b.pose.graze = if b.unit() < 0.7 { 1.0 } else { 0.0 };
        if b.pose.graze < 0.5 {
            b.act = Act::Stand;
        }
        b
    }

    fn unit(&mut self) -> f32 {
        // A xorshift: each beast its own dice, the same each time for the same steps.
        let mut x = self.dice;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.dice = x;
        unit(x)
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }

    fn flat(&self) -> DVec2 {
        DVec2::new(self.pos.x, self.pos.z)
    }
}

/// How the moment looks to a herd.
pub struct Moment<'a> {
    pub ground: &'a Ground,
    /// Where the player is, if anywhere near.
    pub player: Option<DVec3>,
}

/// A herd of one kind.
pub struct Herd {
    pub kind: Kind,
    pub habits: Habits,
    pub fold: Fold,
    pub beasts: Vec<Beast>,
    /// The ground it walks on, once read.
    pub ground: Option<Arc<Ground>>,
    /// Where the herd is drifting about, and which way.
    pub focus: DVec2,
    drift: f32,
    /// Seconds since it was last frightened.
    pub calm: f32,
    /// How many to stand up, and who they are, from this.
    pub count: usize,
    pub seed: u32,
    /// The height of home, for where the first are stood.
    pub home_y: f64,
    /// Why it is here, for the log.
    pub reason: String,
    dice: u32,
}

/// Metres to voxels, for positions.
fn v64(metres: f32) -> f64 {
    m64(metres as f64)
}

impl Herd {
    fn new(kind: Kind, count: usize, fold: Fold, home_y: f64, seed: u32, reason: String) -> Herd {
        let focus = fold.middle();
        Herd {
            kind,
            habits: kind.habits(),
            fold,
            beasts: Vec::new(),
            ground: None,
            focus,
            drift: unit(hash2(seed, 9, 9)) * std::f32::consts::TAU,
            calm: 1000.0,
            count,
            seed,
            home_y,
            reason,
            dice: seed | 1,
        }
    }

    /// The wild herd of a site, at home.
    pub fn wild(site: &Site) -> Herd {
        let home = DVec2::new(site.home.x as f64 + 0.5, site.home.z as f64 + 0.5);
        Herd::new(site.kind, site.count as usize, Fold::Range { home, reach: v64(RANGE) }, site.home.y as f64, site.seed, site.reason.clone())
    }

    /// `count` kept animals of `kind` inside `pen`, whose ground is about `y`.
    pub fn kept(kind: Kind, count: usize, pen: Pen, y: f64, seed: u32, reason: String) -> Herd {
        Herd::new(kind, count, Fold::Pen(pen), y, seed, reason)
    }

    /// `count` of `kind` loose about `at`, as wild ones range about their home.
    pub fn loose(kind: Kind, count: usize, at: DVec3, seed: u32, reason: String) -> Herd {
        let home = DVec2::new(at.x, at.z);
        Herd::new(kind, count, Fold::Range { home, reach: v64(RANGE * 0.6) }, at.y, seed, reason)
    }

    /// The square of ground it needs read: its corner and width each way, in voxels.
    pub fn ground_wanted(&self) -> (i32, i32, i32, i32) {
        let (lo, hi) = match &self.fold {
            Fold::Range { home, reach } => {
                let r = reach + v64(self.habits.spread + 6.0);
                (*home - DVec2::splat(r), *home + DVec2::splat(r))
            }
            Fold::Pen(pen) => {
                let (lo, hi) = pen.bounds();
                (lo - DVec2::splat(3.0), hi + DVec2::splat(3.0))
            }
        };
        let (x0, z0) = (lo.x.floor() as i32, lo.y.floor() as i32);
        (x0, z0, hi.x.ceil() as i32 - x0 + 1, hi.y.ceil() as i32 - z0 + 1)
    }

    /// Where its middle is.
    pub fn centre(&self) -> DVec2 {
        if self.beasts.is_empty() {
            return self.focus;
        }
        self.beasts.iter().map(Beast::flat).sum::<DVec2>() / self.beasts.len() as f64
    }

    /// Where it is, with its height.
    pub fn at(&self) -> DVec3 {
        let c = self.centre();
        let y = if self.beasts.is_empty() { self.home_y } else { self.beasts.iter().map(|b| b.pos.y).sum::<f64>() / self.beasts.len() as f64 };
        DVec3::new(c.x, y, c.y)
    }

    fn unit(&mut self) -> f32 {
        let mut x = self.dice;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.dice = x;
        unit(x)
    }

    /// Who the `k`th of the herd is: a male leading most herds, some young among the
    /// beasts, and the rest females; the oxen all grown.
    fn role(&self, k: usize) -> Role {
        use Kind::*;
        let u = unit(hash2(self.seed, k as i32, 0x5011));
        let birds = Shape::of(self.kind).bird();
        if self.kind == Ox {
            return Role::Male;
        }
        if matches!(self.kind, Duck | WildDuck | Goose | WildGoose) {
            return if u < 0.45 { Role::Male } else { Role::Female };
        }
        let male = match self.kind {
            Chicken | Junglefowl => 0.85,
            k if k.wild() => 0.75,
            Reindeer => 0.4,
            _ => 0.25,
        };
        if k == 0 {
            return if u < male { Role::Male } else { Role::Female };
        }
        let young = if birds { 0.0 } else if self.kind.wild() { 0.28 } else { 0.18 };
        if k >= 2 && u < young { Role::Young } else { Role::Female }
    }

    /// Whether a beast can stand on a cell at all.
    fn stands(&self, look: &Look, c: Cell) -> bool {
        let h = &self.habits;
        let need = if look.role == Role::Young { (h.height - 1).max(1) } else { h.height };
        if h.swims && c.water > 0 {
            // Afloat, it needs only air over the water.
            return c.room as i32 >= (c.water as i32 + 1).min(ROOM as i32);
        }
        c.water as i32 <= h.wade && c.room as i32 >= need
    }

    /// Whether a beast can step from one cell to the next: no higher than it climbs, no
    /// deeper than it wades, and with the room it needs.
    fn passes(&self, look: &Look, from: Cell, to: Cell) -> bool {
        let h = &self.habits;
        let level = |c: Cell| if h.swims && c.water > 0 { c.surface() } else { c.floor as i32 };
        self.stands(look, to) && (level(to) - level(from)).abs() <= h.climb
    }

    /// Whether the middle of a beast may be at `p`: inside its pen, if it has one.
    fn allowed(&self, look: &Look, p: DVec2) -> bool {
        match &self.fold {
            Fold::Range { .. } => true,
            Fold::Pen(pen) => pen.holds(p, v64(look.width * 0.5 + look.length * 0.25 + 0.1)),
        }
    }

    /// The columns a grown beast of the herd can walk to from about `from`, no further
    /// than `radius` voxels from it: so that none is stood up on a ledge or a rock it
    /// could never get down from, or across water from the rest.
    fn reachable(&self, ground: &Ground, from: DVec2, radius: f64) -> HashSet<(i32, i32)> {
        let look = Look::new(self.kind, self.seed, Role::Female);
        let (fx, fz) = (from.x.floor() as i32, from.y.floor() as i32);
        let r = radius.ceil() as i32;
        let within = |x: i32, z: i32| ((x - fx).pow(2) + (z - fz).pow(2)) as f64 <= radius * radius;
        // From the nearest column to `from` that it can stand on.
        let mut start = None;
        'rings: for ring in 0..=r.min(24) {
            for dz in -ring..=ring {
                for dx in -ring..=ring {
                    if dx.abs().max(dz.abs()) != ring {
                        continue;
                    }
                    let (x, z) = (fx + dx, fz + dz);
                    if ground.cell(x, z).is_some_and(|c| self.stands(&look, c)) {
                        start = Some((x, z));
                        break 'rings;
                    }
                }
            }
        }
        let mut seen = HashSet::new();
        let Some(start) = start else { return seen };
        let mut queue = std::collections::VecDeque::from([start]);
        seen.insert(start);
        while let Some((x, z)) = queue.pop_front() {
            let Some(c) = ground.cell(x, z) else { continue };
            for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (nx, nz) = (x + dx, z + dz);
                if seen.contains(&(nx, nz)) || !within(nx, nz) {
                    continue;
                }
                if ground.cell(nx, nz).is_some_and(|n| self.passes(&look, c, n)) {
                    seen.insert((nx, nz));
                    queue.push_back((nx, nz));
                }
            }
        }
        seen
    }

    /// Stands the herd up about its focus, on ground each can stand on and walk from
    /// to the rest.
    pub fn stand_up(&mut self, ground: &Ground) {
        self.beasts.clear();
        let spread = v64(self.habits.spread);
        let home_floor = ground.at(self.focus).map_or(self.home_y as i32, |c| c.floor as i32);
        let open = match &self.fold {
            Fold::Range { .. } => self.reachable(ground, self.focus, spread * 1.2 + 4.0),
            Fold::Pen(pen) => {
                let (lo, hi) = pen.bounds();
                self.reachable(ground, pen.middle(), (hi - lo).length() * 0.5 + 2.0)
            }
        };
        for k in 0..self.count {
            let seed = hash2(self.seed, k as i32, 0xbea5);
            let look = Look::new(self.kind, seed, self.role(k));
            let mut placed = None;
            for _ in 0..40 {
                let (a, r) = (self.unit() as f64 * std::f64::consts::TAU, self.unit().sqrt() as f64 * spread);
                let p = match &self.fold {
                    Fold::Range { .. } => self.focus + DVec2::new(a.cos(), a.sin()) * r,
                    Fold::Pen(pen) => {
                        let (lo, hi) = pen.bounds();
                        lo + (hi - lo) * DVec2::new(self.unit() as f64, self.unit() as f64)
                    }
                };
                let Some(c) = ground.at(p) else { continue };
                if !open.contains(&(p.x.floor() as i32, p.y.floor() as i32)) {
                    continue;
                }
                // Not up on a stone or a stump: the ground about it is level with it.
                let level = [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)]
                    .iter()
                    .all(|&(dx, dz)| ground.at(p + DVec2::new(dx, dz)).is_some_and(|n| self.passes(&look, c, n)));
                if self.stands(&look, c) && level && self.allowed(&look, p) && (c.floor as i32 - home_floor).abs() <= 12 {
                    placed = Some((p, c));
                    break;
                }
            }
            let Some((p, c)) = placed else { continue };
            let y = if self.habits.swims && c.water > 0 { c.surface() } else { c.floor as i32 };
            let facing = self.unit() * std::f32::consts::TAU;
            self.beasts.push(Beast::new(look, DVec3::new(p.x, y as f64, p.y), facing, seed));
        }
    }

    /// Moves the herd on by `dt` seconds.
    pub fn update(&mut self, dt: f32, m: &Moment) {
        if dt <= 0.0 || self.beasts.is_empty() {
            return;
        }
        let ground = m.ground;
        let h = self.habits;
        let wild = self.kind.wild();
        let flight = v64(h.flight);
        self.calm += dt;
        // The nearest any of them is to the player.
        let near = m.player.map(|p| {
            let d = self.beasts.iter().map(|b| b.pos.distance(p)).fold(f64::MAX, f64::min);
            (p, d)
        });
        if let Some((p, d)) = near
            && wild
            && d < flight
        {
            // Off, all of them: the herd's middle moves away from the player, a good
            // way beyond the distance it keeps; out of its range if it must, rather
            // than be cornered at its edge, as far as its ground was read.
            self.calm = 0.0;
            let centre = self.centre();
            let away = (centre - DVec2::new(p.x, p.z)).normalize_or(DVec2::new(self.drift.cos() as f64, self.drift.sin() as f64));
            self.focus = keep_on(ground, centre + away * flight * 1.2, self.focus);
        }
        // At peace, the middle drifts slowly over the range, turning back from its edge
        // and from ground it cannot go on; driven out of it, it drifts back.
        if self.calm > 12.0 {
            self.drift += (self.unit() - 0.5) * 0.6 * dt;
            let pace = v64(0.07) * dt as f64;
            let home = self.fold.keep(self.focus, v64(1.0));
            if home.distance(self.focus) > 1.0 {
                let back = home - self.focus;
                self.drift = back.y.atan2(back.x) as f32;
            }
            let next = self.focus + DVec2::new(self.drift.cos() as f64, self.drift.sin() as f64) * pace;
            let kept = if home.distance(self.focus) > 1.0 { next } else { self.fold.keep(next, v64(1.0)) };
            let footing = ground.at(next).is_some_and(|c| c.water == 0 || h.swims);
            if kept.distance(next) > 1e-6 || !footing {
                self.drift += std::f32::consts::PI * (0.5 + self.unit() * 0.5);
            } else {
                self.focus = next;
            }
        }
        let others: Vec<DVec2> = self.beasts.iter().map(Beast::flat).collect();
        for k in 0..self.beasts.len() {
            let mut b = self.beasts[k].clone();
            self.think(&mut b, dt, near, ground);
            self.go(&mut b, dt, ground);
            // Keep from crowding the rest: a step aside from any too close.
            let mine = b.flat();
            let room = v64(b.look.reach() * 0.45);
            for (j, &o) in others.iter().enumerate() {
                let d = mine.distance(o);
                if j != k && d < room && d > 1e-6 {
                    let push = (mine - o) / d * (room - d) * (2.0 * dt as f64).min(0.5);
                    self.nudge(&mut b, push, ground);
                }
            }
            self.pose(&mut b, dt, ground, near.map(|n| n.0));
            self.beasts[k] = b;
        }
    }

    /// Decides what a beast is about.
    fn think(&mut self, b: &mut Beast, dt: f32, near: Option<(DVec3, f64)>, ground: &Ground) {
        let h = self.habits;
        let wild = self.kind.wild();
        let flight = v64(h.flight);
        let mine = b.flat();
        b.timer -= dt;
        b.clock += dt;
        let threat = near.map(|(p, _)| (p, b.pos.distance(p)));
        if wild {
            match threat {
                // Frightened, or the herd is: away.
                Some((_, d)) if d < flight || (self.calm < 1.0 && d < flight * 2.0) => {
                    if b.act != Act::Flee {
                        b.act = Act::Flee;
                        b.timer = b.range(3.0, 6.0);
                    }
                }
                // Near enough to be watched, heads up.
                Some((_, d)) if d < flight * 1.7 && b.act != Act::Flee => {
                    if b.act != Act::Watch {
                        b.act = Act::Watch;
                        b.goal = None;
                    }
                    b.timer = b.timer.max(b.range(2.0, 5.0));
                }
                _ => {}
            }
        } else if let Some((p, d)) = threat
            && d < flight
            && b.act != Act::Walk
        {
            // Kept, and used to people: out of the way, a few steps.
            let away = (mine - DVec2::new(p.x, p.z)).normalize_or(DVec2::X);
            b.goal = Some(self.fold.keep(mine + away * flight * 1.5, v64(b.look.width)));
            b.act = Act::Walk;
            b.timer = 4.0;
        }
        match b.act {
            Act::Flee => {
                let clear = threat.is_none_or(|(_, d)| d > flight * 1.35);
                if clear && b.timer <= 0.0 {
                    b.act = Act::Watch;
                    b.timer = b.range(3.0, 8.0);
                }
            }
            Act::Watch => {
                if b.timer <= 0.0 && self.calm > 8.0 && threat.is_none_or(|(_, d)| d > flight * 1.7) {
                    b.act = Act::Stand;
                    b.timer = b.range(1.0, 4.0);
                }
            }
            Act::Walk => {
                let arrived = b.goal.is_none_or(|g| g.distance(mine) < v64(0.3));
                if arrived || b.timer <= -20.0 {
                    b.goal = None;
                    b.act = Act::Graze;
                    b.timer = b.range(3.0, 12.0);
                }
            }
            Act::Graze | Act::Stand => {
                if b.timer <= 0.0 {
                    let spread = v64(h.spread);
                    let off = mine.distance(self.focus);
                    let u = b.unit();
                    if off > spread || u < 0.4 {
                        b.goal = Some(self.spot_for(b, ground, threat.map(|t| t.0)));
                        b.act = Act::Walk;
                        b.timer = 0.0;
                    } else if u < 0.8 {
                        b.act = Act::Graze;
                        b.timer = b.range(4.0, 14.0);
                    } else {
                        b.act = Act::Stand;
                        b.timer = b.range(2.0, 6.0);
                    }
                }
            }
        }
    }

    /// A spot near the herd's middle for a beast to go to: on water, mostly, for
    /// those that swim.
    fn spot_for(&mut self, b: &mut Beast, ground: &Ground, player: Option<DVec3>) -> DVec2 {
        let spread = v64(self.habits.spread);
        let shy = v64(self.habits.flight * 1.5).max(v64(3.0));
        let mut best = None;
        for _ in 0..8 {
            let (a, r) = (b.unit() as f64 * std::f64::consts::TAU, b.unit().sqrt() as f64 * spread);
            let p = self.fold.keep(self.focus + DVec2::new(a.cos(), a.sin()) * r, v64(b.look.width));
            let Some(c) = ground.at(p) else { continue };
            // Never toward the player: nowhere it would have to pass near them.
            let near_player = player.is_some_and(|q| {
                let q = DVec2::new(q.x, q.z);
                let (from, ab) = (b.flat(), p - b.flat());
                let t = ((q - from).dot(ab) / ab.length_squared().max(1e-9)).clamp(0.0, 1.0);
                q.distance(from + ab * t) < shy.min(q.distance(from))
            });
            if !self.stands(&b.look, c) || !self.allowed(&b.look, p) || near_player {
                continue;
            }
            best = Some(p);
            let wet = c.water > 0;
            if !self.habits.swims || wet == (b.unit() < 0.7) {
                break;
            }
        }
        best.unwrap_or(self.focus)
    }

    /// Moves a beast as its act has it: turning toward where it is going, and stepping
    /// only where the ground lets it.
    fn go(&mut self, b: &mut Beast, dt: f32, ground: &Ground) {
        let h = self.habits;
        let mine = b.flat();
        let (heading, want) = match b.act {
            Act::Flee => {
                // Away from the player and toward where the herd is going.
                let toward = (self.focus - mine).normalize_or_zero();
                let speed = if self.calm < 2.5 { h.run } else { (h.walk + h.run) * 0.5 };
                let dir = if toward.length() > 0.5 { toward } else { DVec2::new(b.facing.cos() as f64, b.facing.sin() as f64) };
                let far = mine.distance(self.focus) < v64(h.spread * 0.5);
                (dir.y.atan2(dir.x) as f32, if far { h.walk } else { speed })
            }
            Act::Walk => match b.goal {
                Some(g) if g.distance(mine) > v64(0.2) => {
                    let d = g - mine;
                    (d.y.atan2(d.x) as f32, h.walk * b.pace)
                }
                _ => (b.facing, 0.0),
            },
            _ => (b.facing, 0.0),
        };
        let pace = want * b.pace.min(1.05);
        // Speed up and slow down over a moment, not at once.
        let ease = 1.0 - (-dt * 3.0).exp();
        b.speed += (pace - b.speed) * ease;
        let pi = std::f32::consts::PI;
        let turn = (heading - b.facing + pi).rem_euclid(2.0 * pi) - pi;
        let rate = if b.act == Act::Flee { 4.0 } else { 1.8 };
        b.facing += turn.clamp(-rate * dt, rate * dt);
        // Turning on the spot more than it walks.
        let along = (1.0 - turn.abs() / 1.6).max(0.0);
        let mut left = m64((b.speed * along * dt) as f64);
        let mut stuck = false;
        while left > 1e-6 && !stuck {
            let step = left.min(0.5);
            left -= step;
            stuck = true;
            for swerve in [0.0f32, 0.5, -0.5, 1.0, -1.0, 1.6, -1.6] {
                let yaw = b.facing + swerve;
                let dir = DVec2::new(yaw.cos() as f64, yaw.sin() as f64);
                if self.can_move(b, dir * step, ground) {
                    b.pos.x += dir.x * step;
                    b.pos.z += dir.y * step;
                    if swerve != 0.0 {
                        b.facing += swerve.signum() * (rate * dt).min(swerve.abs());
                    }
                    stuck = false;
                    break;
                }
            }
        }
        if stuck {
            b.speed *= 0.5;
            if b.act == Act::Walk {
                b.goal = None;
            }
        }
        // The stride goes round with the ground covered.
        let gone = b.speed * along * dt;
        let stride = stride_length(&b.look, b.speed, h);
        b.pose.stride = (b.pose.stride + gone / stride * std::f32::consts::TAU).rem_euclid(std::f32::consts::TAU);
    }

    /// Whether a beast may move by `by` from where it is: the ground under its middle
    /// and under its nose both passable, and inside its pen.
    fn can_move(&self, b: &Beast, by: DVec2, ground: &Ground) -> bool {
        let mine = b.flat();
        let to = mine + by;
        let Some(here) = ground.at(mine) else { return false };
        let Some(there) = ground.at(to) else { return false };
        if !self.passes(&b.look, here, there) || !self.allowed(&b.look, to) {
            return false;
        }
        let ahead = by.normalize_or_zero() * v64(b.look.reach() * 0.45);
        ground.at(to + ahead).is_some_and(|nose| self.passes(&b.look, there, nose))
    }

    /// Pushes a beast aside by `push`, if the ground lets it go there.
    fn nudge(&self, b: &mut Beast, push: DVec2, ground: &Ground) {
        if self.can_move(b, push, ground) {
            b.pos.x += push.x;
            b.pos.z += push.y;
        }
    }

    /// How a beast holds itself this moment: its head, ears and tail, its gait and its
    /// height on the ground.
    fn pose(&self, b: &mut Beast, dt: f32, ground: &Ground, player: Option<DVec3>) {
        let h = self.habits;
        let ease = |rate: f32| 1.0 - (-dt * rate).exp();
        let (graze, alert) = match b.act {
            Act::Graze => (1.0, 0.0),
            Act::Walk if b.speed < h.walk * 1.2 => (0.25, 0.0),
            Act::Watch => (0.0, 1.0),
            Act::Flee => (0.0, 0.6),
            _ => (0.0, 0.0),
        };
        // A bird pecks: the head goes down and up again, not down and stays.
        let graze = if b.look.shape.bird() && graze > 0.5 { (b.clock * 2.3 + b.pace * 9.0).sin().max(0.0) } else { graze };
        b.pose.graze += (graze - b.pose.graze) * ease(if b.look.shape.bird() { 14.0 } else { 2.5 });
        b.pose.alert += (alert - b.pose.alert) * ease(4.0);
        // The head turned: toward whoever it is watching, or looking about.
        let pi = std::f32::consts::PI;
        let look = match (b.act, player) {
            (Act::Watch, Some(p)) => {
                let to = DVec2::new(p.x - b.pos.x, p.z - b.pos.z);
                (((to.y.atan2(to.x) as f32) - b.facing + pi).rem_euclid(2.0 * pi) - pi).clamp(-1.3, 1.3)
            }
            (Act::Stand, _) => (b.clock * 0.45 + b.pace * 5.0).sin() * 0.7,
            _ => 0.0,
        };
        b.pose.look += (look - b.pose.look) * ease(3.0);
        // The tail swishes in slow bursts, and an ear flicks now and then.
        let swish = (b.clock * 0.29 + b.pace * 3.0).sin();
        b.pose.tail = (b.clock * 2.6).sin() * swish.max(0.0);
        let flick = (b.clock * 0.37 + b.pace * 7.0).fract();
        b.pose.ears = if flick < 0.05 { (flick * 20.0 * std::f32::consts::PI).sin() } else { 0.0 };
        // The gait, from how fast it goes.
        let speed = b.speed;
        let (gait, reach) = if speed < h.walk * 1.4 {
            (Gait::Walk, (speed / h.walk.max(0.1)).min(1.0))
        } else if speed < h.run * 0.75 {
            (Gait::Trot, 1.0)
        } else {
            (Gait::Run, 1.0)
        };
        b.pose.gait = gait;
        b.pose.reach = if speed < 0.05 { 0.0 } else { reach };
        b.pose.flap = if b.look.shape.bird() && speed > h.walk * 2.0 { 0.6 + 0.4 * (b.clock * 20.0).sin().abs() } else { 0.0 };
        // Down to the ground under it, or up onto the water, and tipped along the slope.
        let mine = b.flat();
        if let Some(c) = ground.at(mine) {
            let afloat = h.swims && c.water > 0;
            let y = if afloat { c.surface() } else { c.floor as i32 } as f64;
            b.pos.y += (y - b.pos.y) * ease(8.0) as f64;
            b.pose.afloat = afloat;
        }
        // Measured a voxel beyond its feet each way, so that a single step under it
        // tips it a little, and the legs take up some of any slope.
        let half = v64(b.look.length * 0.5) + 1.0;
        let dir = DVec2::new(b.facing.cos() as f64, b.facing.sin() as f64);
        let at = |p: DVec2| ground.at(p).map(|c| c.floor as f64);
        let pitch = match (at(mine + dir * half), at(mine - dir * half)) {
            (Some(front), Some(back)) if !b.pose.afloat => ((back - front) / (2.0 * half)).atan().clamp(-0.5, 0.5) as f32 * 0.7,
            _ => 0.0,
        };
        b.pose.pitch += (pitch - b.pose.pitch) * ease(5.0);
    }
}

/// `to`, if it is on the ground; else `fallback`.
fn keep_on(ground: &Ground, to: DVec2, fallback: DVec2) -> DVec2 {
    if ground.at(to).is_some() {
        return to;
    }
    // Pulled in toward the middle of what is known until it is.
    let m = ground.middle();
    (1..=10).map(|k| to + (m - to) * (k as f64 / 10.0)).find(|&q| ground.at(q).is_some()).unwrap_or(fallback)
}

/// How far a beast goes in one round of its stride at `speed` metres a second.
fn stride_length(l: &Look, speed: f32, h: Habits) -> f32 {
    let leg = if l.shape.bird() { l.leg * 5.0 } else { l.leg + l.depth * 0.3 };
    let by = if speed < h.walk * 1.4 {
        2.4
    } else if speed < h.run * 0.75 {
        3.2
    } else {
        4.8
    };
    (leg * by).max(0.15)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Level ground, `side` voxels across about the origin, at height 10, with what
    /// `f` makes of any column.
    fn ground(side: i32, f: impl Fn(i32, i32) -> Option<Cell>) -> Ground {
        Ground::from_fn(-side / 2, -side / 2, side, side, |x, z| f(x, z).unwrap_or(Cell::open(10)))
    }

    fn site(kind: Kind, n: u8) -> Site {
        Site { kind, home: glam::IVec3::new(0, 10, 0), count: n, seed: 77, square: (0, 0), reason: "a test".into() }
    }

    fn run(herd: &mut Herd, g: &Ground, player: Option<DVec3>, seconds: f32, mut each: impl FnMut(&Herd)) {
        let dt = 1.0 / 30.0;
        for _ in 0..(seconds / dt) as u32 {
            herd.update(dt, &Moment { ground: g, player });
            each(herd);
        }
    }

    #[test]
    fn a_wild_herd_backs_away_from_a_player_who_comes_close() {
        for kind in Kind::WILD {
            let g = ground(400, |_, _| None);
            let mut herd = Herd::wild(&site(kind, 6));
            herd.stand_up(&g);
            assert!(!herd.beasts.is_empty(), "{}", kind.word());
            run(&mut herd, &g, None, 5.0, |_| {});
            let start = herd.centre();
            // The player walks up to within a few metres of the middle.
            let flight = v64(kind.habits().flight);
            let player = DVec3::new(start.x + flight * 0.4, 10.0, start.y);
            let before = herd.beasts.iter().map(|b| b.pos.distance(player)).fold(f64::MAX, f64::min);
            run(&mut herd, &g, Some(player), 6.0, |_| {});
            let after = herd.beasts.iter().map(|b| b.pos.distance(player)).fold(f64::MAX, f64::min);
            assert!(after > before + v64(3.0), "{}: nearest {:.1} m, then {:.1} m", kind.word(), before / v64(1.0), after / v64(1.0));
            assert!(herd.beasts.iter().all(|b| b.pos.distance(player) > before.min(flight * 0.4)), "{}: one came nearer", kind.word());
            // Left alone, they settle to grazing again.
            run(&mut herd, &g, None, 60.0, |_| {});
            assert!(herd.beasts.iter().any(|b| matches!(b.act, Act::Graze | Act::Stand | Act::Walk)), "{} never settle", kind.word());
        }
    }

    #[test]
    fn nothing_comes_at_the_player() {
        // Whatever it is, however near the player stands, none of them moves closer.
        for kind in Kind::ALL {
            let g = ground(300, |_, _| None);
            let mut herd = Herd::loose(kind, 6, DVec3::new(0.0, 10.0, 0.0), 5, String::new());
            herd.stand_up(&g);
            let player = DVec3::new(v64(2.0), 10.0, 0.0);
            let mut was: Vec<f64> = herd.beasts.iter().map(|b| b.pos.distance(player)).collect();
            let mut closer = 0.0f64;
            run(&mut herd, &g, Some(player), 20.0, |h| {
                for (k, b) in h.beasts.iter().enumerate() {
                    let d = b.pos.distance(player);
                    if d < v64(1.5) {
                        closer = closer.max(was[k] - d);
                    }
                    was[k] = d;
                }
            });
            assert!(closer < v64(0.3), "{} came {closer} voxels nearer", kind.word());
        }
    }

    #[test]
    fn herds_keep_out_of_deep_water_off_cliffs_and_away_from_trunks() {
        // A lake to the east, a crag to the north standing 5 voxels up, and a trunk.
        let lake = |x: i32, _z: i32| x > 20;
        let crag = |_x: i32, z: i32| z < -15;
        let trunk = |x: i32, z: i32| (x + 8).abs() <= 1 && (z - 6).abs() <= 1;
        let g = ground(200, |x, z| {
            if lake(x, z) {
                Some(Cell { floor: 6, water: 4, room: 12 })
            } else if crag(x, z) {
                Some(Cell::open(15))
            } else if trunk(x, z) {
                Some(Cell::open(40))
            } else {
                None
            }
        });
        for kind in Kind::ALL {
            let swims = kind.habits().swims;
            let mut herd = Herd::loose(kind, 8, DVec3::new(0.0, 10.0, 0.0), 3, String::new());
            herd.stand_up(&g);
            // A player coming from the west and south drives them toward all of it.
            let mut t = 0.0;
            let dt = 1.0 / 30.0;
            for _ in 0..(90.0 / dt) as u32 {
                t += dt;
                let player = DVec3::new(-60.0 + (t as f64 * 3.0) % 80.0, 10.0, 30.0 - (t as f64 * 1.5) % 40.0);
                herd.update(dt, &Moment { ground: &g, player: Some(player) });
                for b in &herd.beasts {
                    let (x, z) = (b.pos.x.floor() as i32, b.pos.z.floor() as i32);
                    assert!(!trunk(x, z), "{} walked into a trunk", kind.word());
                    assert!(!crag(x, z) || lake(x, z) || kind.habits().climb >= 5, "{} went up a crag", kind.word());
                    assert!(swims || !lake(x, z), "{} waded into deep water at {x} {z}", kind.word());
                }
            }
        }
    }

    #[test]
    fn kept_animals_stay_inside_their_pen() {
        for kind in Kind::KEPT {
            let g = ground(200, |x, z| (x > 10 && z > 0).then_some(Cell { floor: 9, water: 1, room: 12 }));
            let pen = Pen::rect(DVec2::new(4.0, 2.0), DVec2::new(v64(4.0), v64(3.0)), 0.4, Some(DVec2::new(40.0, 0.0)));
            let mut herd = Herd::kept(kind, 6, pen.clone(), 10.0, 11, String::new());
            herd.stand_up(&g);
            assert!(herd.beasts.len() >= 4, "only {} {} stood in the pen", herd.beasts.len(), kind.word());
            let mut t = 0.0f64;
            run(&mut herd, &g, None, 1.0, |_| {});
            let dt = 1.0 / 30.0;
            for _ in 0..(120.0 / dt) as u32 {
                t += dt as f64;
                // Someone walking round and through it.
                let player = DVec3::new(4.0 + (t * 0.7).cos() * 12.0, 10.0, 2.0 + (t * 0.7).sin() * 9.0);
                herd.update(dt, &Moment { ground: &g, player: Some(player) });
                for b in &herd.beasts {
                    assert!(pen.inside(b.flat()), "a {} got out of its pen at {:?}", kind.word(), b.pos);
                }
            }
            // They do move about in it.
            assert!(herd.beasts.iter().any(|b| b.pose.stride != 0.0), "{} never moved", kind.word());
        }
    }

    #[test]
    fn a_pen_knows_its_inside_and_its_gate() {
        let pen = Pen::rect(DVec2::ZERO, DVec2::new(10.0, 5.0), 0.0, Some(DVec2::new(0.0, 30.0)));
        assert!(pen.inside(DVec2::new(9.0, 4.0)) && !pen.inside(DVec2::new(11.0, 0.0)));
        assert!(pen.holds(DVec2::ZERO, 4.0) && !pen.holds(DVec2::new(9.0, 0.0), 2.0));
        let gate = pen.gate_at().unwrap();
        assert!((gate - DVec2::new(0.0, 5.0)).length() < 1e-9, "{gate}");
        assert!((pen.area() - 200.0 / 6.25).abs() < 1e-9);
    }
}
