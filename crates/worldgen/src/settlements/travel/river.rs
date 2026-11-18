//! The trade boats' voyages (#205): from one quay of a run to the other by day, round the
//! falls by their portages, tied up off the bank for the nights between, and making for
//! the nearest place to tie up when the water turns rough.
//!
//! A voyage is planned in voyage hours: the hours of each day the boat is on the water
//! ([`BOAT_DAY`]) one after another, the nights left out. Where the boat is at an hour of a
//! day is then a look along its parts; the ground is looked at only for where it ties up
//! for the night or out of the weather, once for the day, with the rest of the day's plan.

use super::*;

/// The hours of a day the trade boats are on the water.
const DAY: f32 = BOAT_DAY.1 - BOAT_DAY.0;
/// When the crew camped ashore turn in for the night, and are up again.
const BED: f32 = 21.5;
const UP: f32 = 6.5;

/// One part of a trade boat's voyage, from and to voyage hours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Part {
    from: f32,
    to: f32,
    what: What,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum What {
    /// Under way from `s.0` along the run's line to `s.1`, on water leg `leg` (see
    /// [`Voyage::legs`]).
    Water { s: (f32, f32), leg: usize },
    /// Pulling in from the line at `s` to tie up off the bank for the night, or putting
    /// out again in the morning (`out`).
    Bank { s: f32, out: bool, leg: usize },
    /// Round portage `k`, at one of its steps.
    Carry { k: usize, step: Step },
}

/// The steps of going round a portage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    /// Pulling in from the line to the stage it lands at.
    In,
    /// Tied up there, the goods handed out onto the bank.
    Land,
    /// The goods carried along the path to the other stage, and the crew walking back.
    Goods,
    /// The boat carried along the path (a rowing boat), or hauled on rollers (a sail).
    Haul,
    /// Afloat again at the other stage, the goods loaded back into it.
    Launch,
    /// Pulling out from that stage to the line.
    Out,
}

/// Where a stretch of water a boat goes along ends: at a quay (0 the one it set out from,
/// 1 the other), or at a portage's stage (0 the one it lands at, 1 the one it puts out
/// from again).
#[derive(Clone, Copy, Debug, PartialEq)]
enum End {
    Quay(usize),
    Stage(usize, usize),
}

/// A trade boat's voyage from one quay of its run to the other.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Voyage {
    parts: Vec<Part>,
    /// The stretches of water between the quays and the portages, the way it goes: from
    /// and to how far along the run's line, and what is at either end.
    legs: Vec<(f32, f32, End, End)>,
}

impl Voyage {
    /// The voyage down a run of `len` (or up it) with `portages`, at `speed` voxels an hour
    /// on the water. On the water until there is just time to pull in to the bank by dusk,
    /// then on in the morning; never landing at a portage too late to land by dusk, nor
    /// carrying round it unless that and the launch are done the same day; never putting
    /// out from a stage with less than a little of the day left.
    pub(super) fn plan(len: f32, portages: &[Portage], down: bool, speed: f32) -> Voyage {
        let mut legs = Vec::new();
        let order: Vec<usize> = if down { (0..portages.len()).collect() } else { (0..portages.len()).rev().collect() };
        let (mut s, mut start) = (if down { 0.0 } else { len }, End::Quay(0));
        for &k in &order {
            let p = &portages[k];
            let (stop, go) = if down { (p.above, p.below) } else { (p.below, p.above) };
            legs.push((s, stop, start, End::Stage(k, 0)));
            (s, start) = (go, End::Stage(k, 1));
        }
        legs.push((s, if down { len } else { 0.0 }, start, End::Quay(1)));
        // The end of the day voyage hour `t` falls in: at a day's turn, the next one.
        let dusk = |t: f32| ((t / DAY).floor() + 1.0) * DAY;
        let mut parts = Vec::new();
        let mut t = 0.0;
        for (leg, &(s0, s1, _, to)) in legs.iter().enumerate() {
            let landing = TO_BANK + LAND;
            let after = if matches!(to, End::Stage(..)) { landing } else { 0.0 };
            let mut s = s0;
            loop {
                let left = (s1 - s).abs() / speed;
                let end = dusk(t);
                if t + left + after <= end + 1e-3 {
                    parts.push(Part { from: t, to: t + left, what: What::Water { s: (s, s1), leg } });
                    t += left;
                    break;
                }
                // As far as they get with time left to pull in to the bank by dusk.
                let stop = end - TO_BANK;
                if stop > t {
                    let mid = s + (s1 - s).signum() * (stop - t) * speed;
                    parts.push(Part { from: t, to: stop, what: What::Water { s: (s, mid), leg } });
                    s = mid;
                }
                parts.push(Part { from: stop.max(t), to: end, what: What::Bank { s, out: false, leg } });
                parts.push(Part { from: end, to: end + TO_BANK, what: What::Bank { s, out: true, leg } });
                t = end + TO_BANK;
            }
            let End::Stage(k, _) = to else { continue };
            let long = portages[k].length();
            let mut step = |t: &mut f32, hours: f32, step: Step| {
                parts.push(Part { from: *t, to: *t + hours, what: What::Carry { k, step } });
                *t += hours;
            };
            step(&mut t, TO_BANK, Step::In);
            step(&mut t, LAND, Step::Land);
            // The goods round and the boat after them the same day, or they wait at the
            // stage for the morning.
            let (goods, haul) = (2.0 * long / PACE, long / CARRY);
            if t + goods + haul + LAUNCH > dusk(t) + 1e-3 && goods + haul + LAUNCH < DAY {
                t = dusk(t);
            }
            step(&mut t, goods, Step::Goods);
            step(&mut t, haul, Step::Haul);
            step(&mut t, LAUNCH, Step::Launch);
            if t + TO_BANK + 0.25 > dusk(t) {
                t = dusk(t);
            }
            step(&mut t, TO_BANK, Step::Out);
        }
        Voyage { parts, legs }
    }

    /// How many days the voyage takes, the last whatever part of it is needed.
    pub(super) fn days(&self) -> i64 {
        let hours = self.parts.last().map_or(0.0, |p| p.to);
        ((hours / DAY - 1e-4).ceil() as i64).max(1)
    }

    /// The last part begun by voyage hour `t` (begun before it, for `before`, as for how
    /// things stand at the end of a day), by index; None before the first.
    fn part_at(&self, t: f32, before: bool) -> Option<usize> {
        let n = self.parts.partition_point(|p| if before { p.from < t } else { p.from <= t });
        n.checked_sub(1)
    }
}

/// Where a trade boat ties up off the bank, and the crew's camp ashore by it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Mooring {
    /// How far along the run's line it pulls in from.
    s: f32,
    berth: Vec3,
    camp: Option<Camp>,
}

/// A crew's camp ashore: their fire, and where each of the two sits by it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Camp {
    fire: Vec3,
    seats: [Vec3; 2],
}

/// Where a trade boat is and what its crew are about at one moment.
struct Moment {
    boat: Boat,
    /// The crew, each where they stand, the way they face and what they are doing; None
    /// for both sat in the boat.
    crew: Option<[(Vec3, f32, Doing); 2]>,
    /// A pile of the boat's goods on the bank.
    goods: Option<(Vec3, f32)>,
    /// Tied up for the day or the night with the crew camped ashore.
    camp: Option<Camp>,
    /// Tied up at a quay.
    quay: bool,
    doing: &'static str,
}

impl Travel {
    /// The trade boat `b` of run `r` on `day`.
    pub(super) fn river_boat(&self, c: &Continent, r: usize, b: usize, day: i64) -> RiverPlan {
        let run = &c.trails.waterways.runs[r];
        let h = hash2(self.seed ^ 0x0b0a_7e55, r as i32, b as i32);
        let sail = b == 1 || (b == 0 && unit(h) < 0.3);
        let speed = if sail { SAIL } else { ROW };
        let len = run.length();
        // Down to the far quay, a day there, back up, a day at home: as many days on the
        // water as a rowing boat needs, so that the boats of a run keep the same round,
        // the second half a round behind the first, never tied up at one quay together.
        let trip = [true, false].map(|down| Voyage::plan(len, &run.portages, down, ROW).days()).into_iter().max().unwrap_or(1);
        let cycle = 2 * (trip + 1);
        let first = hash2(self.seed ^ 0x0b0a_7e55, r as i32, 0);
        let phase = (first.rotate_left(9) % cycle as u32) as i64 + b as i64 * (trip + 1);
        let t = (day + phase).rem_euclid(cycle);
        let (leg, down) = if t < trip {
            (Leg::Going(t), true)
        } else if t == trip {
            (Leg::Moored, true)
        } else if t < 2 * trip + 1 {
            (Leg::Going(t - trip - 1), false)
        } else {
            (Leg::Moored, false)
        };
        let quays = &c.trails.waterways.quays;
        let mut plan = RiverPlan {
            id: (3u64 << 60) | ((r as u64) << 8) | b as u64,
            region: self.regions[run.a],
            from: if down { run.a } else { run.b },
            to: if down { run.b } else { run.a },
            seed: h,
            sail,
            speed,
            leg,
            down,
            line: run.line.clone(),
            along: run.along.clone(),
            berths: (quays[run.from].berth, quays[run.from].heading, quays[run.to].berth, quays[run.to].heading),
            portages: run.portages.clone(),
            voyage: Voyage::plan(len, &run.portages, down, speed),
            nights: [None, None],
            refuges: Vec::new(),
            stages: vec![[None, None]; run.portages.len()],
        };
        if let Leg::Going(d) = leg {
            plan.look_ashore(c, self, d);
        }
        plan
    }
}

impl RiverPlan {
    /// Where it ties up on day `day` of its voyage: for the night before and the night
    /// after, and wherever it could make for along the day's water if it turns rough; and
    /// the crew's camps by the stages of the portages.
    fn look_ashore(&mut self, c: &Continent, travel: &Travel, day: i64) {
        let (start, end) = (day as f32 * DAY, (day + 1) as f32 * DAY);
        let mut stages = std::mem::take(&mut self.stages);
        for (k, camps) in stages.iter_mut().enumerate() {
            for i in 0..2 {
                camps[i] = self.stage_camp(c, travel, k, i);
            }
        }
        self.stages = stages;
        for (n, t) in [start, end].into_iter().enumerate() {
            let Some(i) = self.voyage.part_at(t, true) else { continue };
            if let What::Bank { s, leg, .. } = self.voyage.parts[i].what {
                let (p, _) = along_line(&self.line, &self.along, s);
                self.nights[n] = Some(self.bank_spot(c, travel, s, leg).unwrap_or(Mooring { s, berth: p, camp: None }));
            }
        }
        // Somewhere to tie up every so far along the water they are on today.
        for part in &self.voyage.parts {
            let What::Water { s: (a, b), leg } = part.what else { continue };
            if part.to <= start || part.from >= end || part.to <= part.from {
                continue;
            }
            let at = |t: f32| a + (b - a) * ((t.clamp(start, end) - part.from) / (part.to - part.from)).clamp(0.0, 1.0);
            let (s0, s1) = (at(part.from), at(part.to));
            let n = ((s1 - s0).abs() / REFUGE).ceil() as usize;
            for j in 0..n {
                let s = s0 + (s1 - s0) * (j as f32 + 0.5) / n as f32;
                if let Some(m) = self.bank_spot(c, travel, s, leg) {
                    self.refuges.push((leg, m));
                }
            }
        }
    }

    /// The berth at stage `which` of portage `k` (0 above the rough water, 1 below), the
    /// foot of its jetty on the bank, and the way inland there.
    fn stage(&self, k: usize, which: usize) -> (Vec3, Vec3, Vec2) {
        let p = &self.portages[k];
        let berth = p.berths[which].0;
        let foot = if which == 0 { p.path[0] } else { p.path[p.path.len() - 1] };
        let n = (flat(foot) - flat(berth)).normalize_or_zero();
        (berth, foot, n)
    }

    /// Which of portage `k`'s stages (0 above, 1 below) is stage `which` of it the way the
    /// boat is going (0 the one it lands at).
    fn stage_of(&self, which: usize) -> usize {
        if self.down { which } else { 1 - which }
    }

    /// The camp ashore by stage `which` of portage `k`, for a night there.
    fn stage_camp(&self, c: &Continent, travel: &Travel, k: usize, which: usize) -> Option<Camp> {
        let (_, foot, n) = self.stage(k, which);
        let pad = m(24.0);
        let sampler = Sampler::new(c, travel.world_seed, (foot.x - pad).floor() as i32, (foot.z - pad).floor() as i32, (2.0 * pad) as i32 + 2);
        // Beside the path, not on it: a little up the bank and to the side away from the
        // rough water.
        let p = &self.portages[k];
        let (_, down_path) = p.at(if which == 0 { m(2.0) } else { p.length() - m(2.0) });
        let along = Vec2::from_angle(down_path) * if which == 0 { -1.0 } else { 1.0 };
        for open in [true, false] {
            for back in [m(1.0), m(3.0), m(5.0)] {
                let edge = flat(foot) + along * back;
                if let Some(camp) = camp_ashore(c, travel, &sampler, edge, n, open) {
                    return Some(camp);
                }
            }
        }
        None
    }

    /// Somewhere to tie up off the bank about `s` along the line, on water leg `leg`, with
    /// a camp ashore by it: looked for a little up and down the river from there, and on
    /// either bank. None where there is nowhere.
    fn bank_spot(&self, c: &Continent, travel: &Travel, s: f32, leg: usize) -> Option<Mooring> {
        let (a, b, _, _) = self.voyage.legs[leg];
        let (lo, hi) = (a.min(b) + m(8.0), a.max(b) - m(8.0));
        let (p, _) = along_line(&self.line, &self.along, s);
        let pad = m(48.0) + m(40.0);
        let sampler = Sampler::new(c, travel.world_seed, (p.x - pad).floor() as i32, (p.z - pad).floor() as i32, (2.0 * pad) as i32 + 2);
        let col = |q: Vec2| sampler.column(q.x.floor() as i32, q.y.floor() as i32);
        let first = if hash2(self.seed, s as i32, 0xba4c) & 1 == 0 { 1.0 } else { -1.0 };
        for open in [true, false] {
            for j in 0..13u32 {
                let at = s + m(8.0) * j.div_ceil(2) as f32 * if j % 2 == 1 { 1.0 } else { -1.0 };
                if at < lo || at > hi {
                    continue;
                }
                let (q, dir) = along_line(&self.line, &self.along, at);
                let mid = flat(q);
                if col(mid).water_surface().is_none() {
                    continue;
                }
                for side in [first, -first] {
                    let n = Vec2::from_angle(dir).perp() * side;
                    let Some(near) = water_ends(&sampler, mid, n, m(30.0)) else { continue };
                    let berth = mid + n * (near - m(1.3)).max(0.0);
                    if col(berth).water_surface().is_none() {
                        continue;
                    }
                    if let Some(camp) = camp_ashore(c, travel, &sampler, mid + n * near, n, open) {
                        return Some(Mooring { s: at, berth: Vec3::new(berth.x, q.y, berth.y), camp: Some(camp) });
                    }
                }
            }
        }
        None
    }

    /// The way the boat points going on from `dir`, the way down the line.
    fn ahead(&self, dir: f32) -> f32 {
        if self.down { dir } else { dir + std::f32::consts::PI }
    }

    /// The quay it set out from (0) or is bound for (1), its berth and heading there.
    fn quay(&self, which: usize) -> (Vec3, f32) {
        let (first, second) = ((self.berths.0, self.berths.1), (self.berths.2, self.berths.3));
        if (which == 0) == self.down { first } else { second }
    }

    fn boat(&self, at: Vec3, heading: f32, moving: bool) -> Boat {
        Boat { at, heading, sail: self.sail, load: self.seed, moving, ashore: false, laden: true }
    }

    fn still(&self, boat: Boat, doing: &'static str) -> Moment {
        Moment { boat, crew: None, goods: None, camp: None, quay: false, doing }
    }

    /// Tied up at quay `which`.
    fn at_quay(&self, which: usize, doing: &'static str) -> Moment {
        let (at, heading) = self.quay(which);
        Moment { quay: true, ..self.still(self.boat(at, heading, false), doing) }
    }

    /// Tied up at `m`, the crew camped ashore if there is a camp, the boat pointing on.
    fn moored(&self, m: &Mooring, doing: &'static str) -> Moment {
        let (_, dir) = along_line(&self.line, &self.along, m.s);
        Moment { camp: m.camp, ..self.still(self.boat(m.berth, self.ahead(dir), false), doing) }
    }

    /// The night's mooring about voyage hour `t` at the turn of a day.
    fn night(&self, t: f32, day: i64) -> Option<&Mooring> {
        let start = day as f32 * DAY;
        self.nights[if (t - start).abs() < 1e-3 { 0 } else { 1 }].as_ref()
    }

    /// Tied up at stage `which` (the way it goes) of portage `k`, the goods aboard or not.
    fn at_stage(&self, k: usize, which: usize, laden: bool, doing: &'static str) -> Moment {
        let i = self.stage_of(which);
        let (berth, _, _) = self.stage(k, i);
        let heading = self.ahead(self.portages[k].berths[i].1);
        Moment { camp: self.stages[k][i], ..self.still(Boat { laden, ..self.boat(berth, heading, false) }, doing) }
    }

    /// Where it is at voyage hour `t` of `day`, `before` for how things stand at the end
    /// of the day (or before its start), as it goes when the water is calm.
    fn moment(&self, t: f32, before: bool, day: i64) -> Moment {
        let Some(i) = self.voyage.part_at(t, before) else { return self.at_quay(0, "tied up at the quay") };
        let part = self.voyage.parts[i];
        let f = if part.to > part.from { ((t - part.from) / (part.to - part.from)).clamp(0.0, 1.0) } else { 1.0 };
        let done = t >= part.to;
        let line = |s: f32| along_line(&self.line, &self.along, s);
        match part.what {
            What::Water { s: (a, b), .. } => {
                if done && i + 1 == self.voyage.parts.len() {
                    return self.at_quay(1, "tied up at the quay");
                }
                let (p, dir) = line(a + (b - a) * f);
                let doing = if self.down { "carrying goods down the river" } else { "carrying goods up the river" };
                self.still(self.boat(p, self.ahead(dir), !done), doing)
            }
            What::Bank { s, out, .. } => {
                let (p, dir) = line(s);
                let mooring = self.night(if out { part.from } else { part.to }, day).copied().unwrap_or(Mooring { s, berth: p, camp: None });
                if !out && done {
                    return self.moored(&mooring, "tied up by the bank for the night");
                }
                let (from, to) = if out { (mooring.berth, p) } else { (p, mooring.berth) };
                let doing = if out { "putting out from the bank" } else { "pulling in to the bank for the night" };
                self.still(self.boat(from.lerp(to, f), self.ahead(dir), !done), doing)
            }
            What::Carry { k, step } => self.carrying(k, step, f, done),
        }
    }

    /// At step `step` of going round portage `k`, `f` of the way through it.
    fn carrying(&self, k: usize, step: Step, f: f32, done: bool) -> Moment {
        let p = &self.portages[k];
        let long = p.length();
        let (land, launch) = (self.stage_of(0), self.stage_of(1));
        let (land_berth, land_foot, land_in) = self.stage(k, land);
        let (launch_berth, launch_foot, launch_in) = self.stage(k, launch);
        // Along the path the way they go, `x` from the stage they landed at.
        let path = |x: f32| {
            let (q, dir) = p.at(if self.down { x } else { long - x });
            (q, self.ahead(dir))
        };
        let pile = |foot: Vec3, n: Vec2| {
            let q = flat(foot) + n * m(0.9);
            (Vec3::new(q.x, foot.y, q.y), n.y.atan2(n.x) + std::f32::consts::FRAC_PI_2)
        };
        // Stood at the stage's foot, side by side, facing the boat.
        let by = |foot: Vec3, n: Vec2| {
            let across = n.perp();
            let face = (-n).y.atan2(-n.x);
            [0, 1].map(|j| {
                let q = flat(foot) + across * if j == 0 { m(0.5) } else { -m(0.5) };
                (Vec3::new(q.x, foot.y, q.y), face, Doing::Standing)
            })
        };
        let stop = if self.down { p.above } else { p.below };
        let go = if self.down { p.below } else { p.above };
        let line = |s: f32| along_line(&self.line, &self.along, s);
        match step {
            Step::In => {
                let (q, dir) = line(stop);
                self.still(self.boat(q.lerp(land_berth, f), self.ahead(dir), !done), "pulling in to a landing stage")
            }
            Step::Land => {
                if done {
                    return self.at_stage(k, 0, false, "camped by the landing stage, to carry round in the morning");
                }
                let mut mo = self.at_stage(k, 0, f < 0.5, "landing, to carry round the rough water");
                mo.camp = None;
                mo.crew = Some(by(land_foot, land_in));
                mo.goods = (f >= 0.5).then(|| pile(land_foot, land_in));
                mo
            }
            Step::Goods => {
                let mut mo = self.at_stage(k, 0, false, "carrying their goods round the rough water");
                mo.camp = None;
                let there = f < 0.5;
                let x = if there { long * f * 2.0 } else { long * (1.0 - (f - 0.5) * 2.0) };
                mo.crew = Some([0, 1].map(|j| {
                    let (q, dir) = path((x - j as f32 * m(1.6) * if there { 1.0 } else { -1.0 }).clamp(0.0, long));
                    (q, if there { dir } else { dir + std::f32::consts::PI }, if there { Doing::Laden } else { Doing::Walking })
                }));
                if !there {
                    mo.doing = "walking back for the boat";
                    mo.goods = Some(pile(launch_foot, launch_in));
                }
                mo
            }
            Step::Haul => {
                let (q, dir) = path(long * f);
                let boat = Boat { ashore: true, laden: false, ..self.boat(q, dir, !done) };
                let crew = [0, 1].map(|j| {
                    let off = if self.sail {
                        if j == 0 { m(2.1) } else { -m(2.1) }
                    } else if j == 0 {
                        m(0.95)
                    } else {
                        -m(0.95)
                    };
                    let (c, _) = path((long * f + off).clamp(0.0, long));
                    (c, dir, if self.sail { Doing::Hauling } else { Doing::Portaging })
                });
                let doing = if self.sail { "hauling the boat round the rough water on rollers" } else { "carrying the boat round the rough water" };
                Moment { boat, crew: Some(crew), goods: Some(pile(launch_foot, launch_in)), camp: None, quay: false, doing }
            }
            Step::Launch => {
                if done {
                    return self.at_stage(k, 1, true, "camped by the landing stage for the night");
                }
                let mut mo = self.at_stage(k, 1, f >= 0.5, "launching again, the goods loaded back in");
                mo.camp = None;
                mo.crew = Some(by(launch_foot, launch_in));
                mo.goods = (f < 0.5).then(|| pile(launch_foot, launch_in));
                mo
            }
            Step::Out => {
                let (q, dir) = line(go);
                self.still(self.boat(launch_berth.lerp(q, f), self.ahead(dir), !done), "putting out from a landing stage")
            }
        }
    }

    /// Where it is at `hour` of `day` of its voyage once the water turned rough at `since`:
    /// making for the nearest place it can tie up along the water it is on, or going
    /// round a portage it had begun, and staying there. None where it goes on as in calm
    /// water: already tied up for the night, or the water turned only after dark.
    fn sheltering(&self, day: i64, since: f32, hour: f32) -> Option<Moment> {
        if since >= BOAT_DAY.1 {
            return None;
        }
        let start = day as f32 * DAY;
        let t0 = start + (since - BOAT_DAY.0).clamp(0.0, DAY);
        let t = start + (hour - BOAT_DAY.0).clamp(0.0, DAY);
        let before = since <= BOAT_DAY.0;
        let weather = "tied up, waiting for calmer water";
        let Some(i) = self.voyage.part_at(t0, before) else { return Some(self.at_quay(0, "kept in by the weather")) };
        let part = self.voyage.parts[i];
        let carried = |k: usize, step: Step| self.voyage.parts.iter().find(|p| p.what == What::Carry { k, step }).map_or(part.to, |p| p.to);
        match part.what {
            What::Water { s: (a, b), leg } if t0 < part.to => {
                let f = ((t0 - part.from) / (part.to - part.from).max(1e-4)).clamp(0.0, 1.0);
                let now = a + (b - a) * f;
                let (s0, s1, e0, e1) = self.voyage.legs[leg];
                // The nearest end of the water it is on, or bank along it.
                let mut best: (f32, Option<Mooring>, Option<End>) = (s0, None, Some(e0));
                for (s, end) in [(s0, e0), (s1, e1)] {
                    if (s - now).abs() < (best.0 - now).abs() {
                        best = (s, None, Some(end));
                    }
                }
                for (l, r) in &self.refuges {
                    if *l == leg && (r.s - now).abs() < (best.0 - now).abs() {
                        best = (r.s, Some(*r), None);
                    }
                }
                let row = (best.0 - now).abs() / self.speed;
                let tied = || match (best.1, best.2) {
                    (Some(r), _) => self.moored(&r, weather),
                    (_, Some(End::Quay(q))) => self.at_quay(q, weather),
                    (_, Some(End::Stage(k, w))) => self.at_stage(k, w, true, weather),
                    _ => self.at_quay(0, weather),
                };
                if t >= t0 + row + TO_BANK {
                    return Some(tied());
                }
                let line = |s: f32| along_line(&self.line, &self.along, s);
                let (q, dir) = line(now + (best.0 - now) * ((t - t0) / row.max(1e-4)).clamp(0.0, 1.0));
                let back = (best.0 - now) * (s1 - s0) < 0.0;
                let heading = self.ahead(dir) + if back { std::f32::consts::PI } else { 0.0 };
                if t < t0 + row {
                    return Some(self.still(self.boat(q, heading, true), "making for the bank as the weather turns"));
                }
                // Pulling in to it.
                let to = tied().boat.at;
                let f = ((t - t0 - row) / TO_BANK).clamp(0.0, 1.0);
                Some(self.still(self.boat(q.lerp(to, f), heading, true), "making for the bank as the weather turns"))
            }
            // Rough since before they were up, or turned as they were putting out in the
            // morning: they stay where they lay for the night.
            What::Bank { out: true, .. } => {
                let r = self.night(start, day).copied()?;
                Some(self.moored(&r, weather))
            }
            What::Bank { out: false, .. } if before => {
                let r = self.night(start, day).copied()?;
                Some(self.moored(&r, weather))
            }
            What::Carry { k, step: Step::In | Step::Land } => {
                let landed = carried(k, Step::Land);
                (t >= landed).then(|| self.at_stage(k, 0, false, weather))
            }
            What::Carry { k, step: Step::Goods | Step::Haul | Step::Launch } => {
                let launched = carried(k, Step::Launch);
                (t >= launched).then(|| self.at_stage(k, 1, true, weather))
            }
            What::Carry { k, step: Step::Out } => Some(self.at_stage(k, 1, true, weather)),
            _ => None,
        }
    }

    pub(super) fn at(&self, hour: f32, water: Water) -> Scene {
        let mut scene = Scene::new(self.id, Kind::RiverBoat, self.region);
        scene.from = Some(self.from);
        scene.to = Some(self.to);
        let now = match self.leg {
            Leg::Moored => self.at_quay(1, "tied up at the quay"),
            Leg::Going(day) => {
                let start = day as f32 * DAY;
                let t = start + (hour - BOAT_DAY.0).clamp(0.0, DAY);
                let before = !(BOAT_DAY.0..BOAT_DAY.1).contains(&hour);
                let rough = water.rough_since.filter(|&since| hour >= since);
                rough.and_then(|since| self.sheltering(day, since, hour)).unwrap_or_else(|| self.moment(t, before, day))
            }
        };
        let at = now.boat.at;
        scene.boats.push(now.boat);
        scene.goods.extend(now.goods);
        let asleep = !(UP..BED).contains(&hour);
        let heading = now.boat.heading;
        if let Some(camp) = now.camp {
            // Ashore about their fire, and asleep beside it at night, their feet to it.
            let lit = (BOAT_DAY.1..23.0).contains(&hour) || (5.5..BOAT_DAY.0).contains(&hour) || hour < 1.0;
            scene.fire = Some((camp.fire, if lit { Fire::Lit } else { Fire::Low }));
            for (k, seat) in camp.seats.iter().enumerate() {
                let facing = toward(flat(*seat), flat(camp.fire));
                if asleep {
                    let bed = flat(*seat) - Vec2::from_angle(facing) * BEDROLL_BACK;
                    scene.bedrolls.push((Vec3::new(bed.x, seat.y, bed.y), facing));
                }
                let doing = if asleep { Doing::Asleep } else { Doing::Sitting };
                scene.people.push(Figure { seed: hash2(self.seed, k as i32, 0xc4e3), who: Who::Adult, work: Work::Boatman, feet: *seat, facing, doing });
            }
        } else if let Some(crew) = now.crew {
            for (k, (feet, facing, doing)) in crew.into_iter().enumerate() {
                scene.people.push(Figure { seed: hash2(self.seed, k as i32, 0xc4e3), who: Who::Adult, work: Work::Boatman, feet, facing, doing });
            }
        } else if !(now.quay && asleep) {
            // A crew of two: one at the oars, or at the sheet, and one at the stern; gone
            // home from the quay for the night.
            let along = Vec2::from_angle(heading);
            for (k, off) in [(0, m(0.85)), (1, -m(0.8))] {
                let p = flat(at) + along * off;
                let doing = if now.boat.moving && k == 0 && !self.sail { Doing::Rowing } else { Doing::Sitting };
                let facing = if doing == Doing::Rowing { heading + std::f32::consts::PI } else { heading };
                scene.people.push(Figure { seed: hash2(self.seed, k, 0xc4e3), who: Who::Adult, work: Work::Boatman, feet: Vec3::new(p.x, at.y, p.y), facing, doing });
            }
        }
        scene.doing = now.doing;
        scene.at = at;
        scene
    }
}

/// A camp for a boat's crew ashore, back from the water's edge at `edge` along `n`
/// (inland): dry, level, off every trail and away from the settlements; in the open first
/// if `open`. None where there is no room.
fn camp_ashore(c: &Continent, travel: &Travel, sampler: &Sampler, edge: Vec2, n: Vec2, open: bool) -> Option<Camp> {
    let col = |p: Vec2| sampler.column(p.x.floor() as i32, p.y.floor() as i32);
    let across = n.perp();
    for back in [m(3.5), m(5.0), m(6.5)] {
        let fire = edge + n * back;
        if travel.towns.iter().any(|t| t.0.distance(fire) < t.1 + m(20.0)) {
            return None;
        }
        let seats = [fire + across * m(1.7), fire - across * m(1.7)];
        let probe = [fire, seats[0], seats[1], seats[0] + across * (2.0 * BEDROLL_BACK), seats[1] - across * (2.0 * BEDROLL_BACK)];
        let cols = probe.map(col);
        let dry = cols.iter().all(|k| k.water_surface().is_none() && k.river > m(0.6) && matches!(k.cover, Cover::Land | Cover::Beach | Cover::Bank));
        let off_trail = cols[0].clear > m(1.4) && cols.iter().all(|k| k.clear > m(0.4));
        let (top, bottom) = cols.iter().fold((i32::MIN, i32::MAX), |(t, b), k| (t.max(k.top()), b.min(k.top())));
        if !(dry && off_trail && top - bottom <= 1) {
            continue;
        }
        let (x, z) = (c.desc.mx as f64 * MACRO_SIZE as f64 + fire.x as f64, c.desc.mz as f64 * MACRO_SIZE as f64 + fire.y as f64);
        if open && wooded(travel.world_seed, x, z) > 0.15 {
            continue;
        }
        let y = |k: usize| (cols[k].top() + 1) as f32;
        return Some(Camp { fire: Vec3::new(fire.x, y(0), fire.y), seats: [Vec3::new(seats[0].x, y(1), seats[0].y), Vec3::new(seats[1].x, y(2), seats[1].y)] });
    }
    None
}
