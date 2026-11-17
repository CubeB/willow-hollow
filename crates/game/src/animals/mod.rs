//! Animals that walk the ground (#206): the wild herds out in the land, and the kept
//! animals in their pens, coop yards and pastures.
//!
//! Wild herds come from the world (`worldgen::fauna`): each square of land has a herd or
//! not by the seed, of the kind its country has, so the same place always has the same
//! herd, and it is stood up again each time the player comes back near. Kept animals are
//! handed over by whoever lays out a settlement, a pen at a time, with [`Herds::keep`].
//! Each herd's ground is read from the voxels on a thread of its own when the herd is
//! stood up (`ground.rs`); until it has arrived the herd is neither moved nor drawn,
//! which is never for long, since herds are stood up well beyond sight of them.
//!
//! They are drawn as props, near and far as the people are (`model.rs`), within a
//! budget of boxes of their own (see [`BUDGET`]), nearest first.

pub mod ground;
pub mod herd;
pub mod model;

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        mpsc::{Receiver, Sender, channel},
    },
};

use glam::{DVec2, DVec3};
use worldgen::{
    fauna::{self, Kind, NotReady, Site},
    noise::hash2,
    scale::{VOXELS_PER_METRE, m64},
    world::GameWorld,
};

pub use ground::Ground;
pub use herd::{Act, Fold, Herd, Moment, Pen};
pub use model::{Detail, Gait, Look, Pose, Role};

use crate::particles::ParticleInstance;

/// How near a wild herd's home comes before the herd is stood up, and how far off any
/// herd is let go again, in metres: beyond the furthest any animal is seen from.
pub const REACH: f32 = 300.0;
pub const GONE: f32 = 420.0;
/// The most boxes the animals take of the props' room in a frame. A herd of a dozen
/// near is some 300; past this, the furthest are drawn in a few boxes, and then not at
/// all, so a settlement's worth of pens beside its people never crowds out the rest.
pub const BUDGET: usize = 1536;
/// How often the squares about the player are looked over for herds, in seconds.
const LOOK_EVERY: f32 = 1.5;

/// Which herd: a wild one by its square, a kept one by whoever handed its pen over, or
/// one called up from the console.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Id {
    Wild((i32, i32)),
    Kept(u64),
    Called(u32),
}

enum Job {
    Square((i32, i32)),
    Ground(Id, (i32, i32, i32, i32)),
}

enum Answer {
    Square((i32, i32), Result<Option<Site>, NotReady>),
    Ground(Id, Arc<Ground>, f32),
}

/// The thread that decides squares and reads ground.
struct Worker {
    ask: Sender<Job>,
    answer: Receiver<Answer>,
}

impl Worker {
    fn new(world: Arc<GameWorld>) -> Self {
        let (ask, jobs) = channel::<Job>();
        let (reply, answer) = channel();
        std::thread::Builder::new()
            .name("animals".into())
            .spawn(move || {
                crate::threads::background();
                // Ends when the game drops `ask`.
                while let Ok(job) = jobs.recv() {
                    let out = match job {
                        Job::Square(sq) => Answer::Square(sq, fauna::site(&world, sq)),
                        Job::Ground(id, (x0, z0, w, d)) => {
                            let start = std::time::Instant::now();
                            let g = Ground::read(&world, x0, z0, w, d);
                            Answer::Ground(id, Arc::new(g), start.elapsed().as_secs_f32() * 1000.0)
                        }
                    };
                    if reply.send(out).is_err() {
                        break;
                    }
                }
            })
            .expect("animals thread");
        Worker { ask, answer }
    }
}

/// Every herd about the player.
pub struct Herds {
    world: Arc<GameWorld>,
    herds: Vec<(Id, Herd)>,
    /// The squares looked at, with the herd each has; and those asked about.
    squares: HashMap<(i32, i32), Option<Site>>,
    asked: HashSet<(i32, i32)>,
    worker: Option<Worker>,
    /// Decide and read everything there and then, on this thread: for screenshots.
    at_once: bool,
    /// Look for wild herds at all.
    pub wild: bool,
    /// Draw every animal as from far off.
    pub far: bool,
    /// Pens set out to be seen, with a hurdle fence drawn round them.
    fenced: HashSet<Id>,
    look_in: f32,
    player: Option<DVec3>,
    called: u32,
    /// The herds named in the log last, to name them again only when they change.
    named: Vec<Id>,
    report_in: f32,
}

impl Herds {
    pub fn new(world: Arc<GameWorld>) -> Self {
        Herds {
            world,
            herds: Vec::new(),
            squares: HashMap::new(),
            asked: HashSet::new(),
            worker: None,
            at_once: false,
            wild: true,
            far: false,
            fenced: HashSet::new(),
            look_in: 0.0,
            player: None,
            called: 0,
            named: Vec::new(),
            report_in: 0.0,
        }
    }

    /// Herds for a screenshot: every square decided and all ground read at once, and the
    /// wild herds only if `wild`.
    pub fn at_once(world: Arc<GameWorld>, wild: bool) -> Self {
        Herds { at_once: true, wild, ..Herds::new(world) }
    }

    fn worker(&mut self) -> &Worker {
        let world = self.world.clone();
        self.worker.get_or_insert_with(|| Worker::new(world))
    }

    /// Asks for a herd's ground, or reads it there and then.
    fn want_ground(&mut self, id: Id) {
        let Some(wanted) = self.herds.iter().find(|h| h.0 == id).map(|h| h.1.ground_wanted()) else { return };
        if self.at_once {
            let g = Arc::new(Ground::read(&self.world, wanted.0, wanted.1, wanted.2, wanted.3));
            self.landed(id, g, 0.0);
        } else {
            let _ = self.worker().ask.send(Job::Ground(id, wanted));
        }
    }

    fn landed(&mut self, id: Id, g: Arc<Ground>, ms: f32) {
        if let Some((_, h)) = self.herds.iter_mut().find(|h| h.0 == id) {
            h.stand_up(&g);
            log::debug!(target: "game::animals", "{} stood up, {} of them, on {}x{} columns of ground read in {ms:.0} ms", h.kind.word(), h.beasts.len(), g.w, g.d);
            h.ground = Some(g);
        }
    }

    fn add(&mut self, id: Id, herd: Herd) {
        log::debug!(target: "game::animals", "{} x{} at {:.0},{:.0}: {}", herd.kind.word(), herd.count, herd.focus.x, herd.focus.y, herd.reason);
        self.herds.push((id, herd));
        self.want_ground(id);
    }

    /// Moves every herd on by `dt` seconds, with the player at `player`: stands up the
    /// wild herds coming into reach and lets go those far behind.
    pub fn update(&mut self, dt: f32, player: DVec3) {
        self.player = Some(player);
        if let Some(w) = &self.worker {
            let answers: Vec<Answer> = w.answer.try_iter().collect();
            for a in answers {
                match a {
                    Answer::Square(sq, Ok(site)) => {
                        self.asked.remove(&sq);
                        self.squares.insert(sq, site);
                    }
                    // Its continent is not made yet: asked again on a later look.
                    Answer::Square(sq, Err(NotReady)) => {
                        self.asked.remove(&sq);
                    }
                    Answer::Ground(id, g, ms) => self.landed(id, g, ms),
                }
            }
        }
        self.look_in -= dt;
        if self.look_in <= 0.0 {
            self.look_in = LOOK_EVERY;
            self.look_round(player);
        }
        let flat = |p: DVec3| DVec2::new(p.x, p.z);
        let gone = m64(GONE as f64);
        let before = self.herds.len();
        self.herds.retain(|(id, h)| {
            let keep = flat(h.at()).distance(flat(player)) < gone;
            if !keep {
                log::debug!(target: "game::animals", "{} at {:.0},{:.0} let go ({id:?})", h.kind.word(), h.focus.x, h.focus.y);
            }
            keep
        });
        if self.herds.len() != before {
            let ids: HashSet<Id> = self.herds.iter().map(|h| h.0).collect();
            self.fenced.retain(|id| ids.contains(id));
        }
        for (_, h) in &mut self.herds {
            if let Some(g) = h.ground.clone() {
                h.update(dt, &Moment { ground: &g, player: Some(player) });
            }
        }
        self.report_in -= dt;
        if self.report_in <= 0.0 {
            self.report_in = 10.0;
            let ids: Vec<Id> = self.herds.iter().map(|h| h.0).collect();
            if ids != self.named {
                self.named = ids;
                self.report(player, log::Level::Debug);
            }
        }
    }

    /// Looks over the squares about the player for wild herds, and stands up those
    /// whose homes have come into reach.
    fn look_round(&mut self, player: DVec3) {
        if !self.wild {
            return;
        }
        let seed = self.world.seed();
        let reach = m64(REACH as f64);
        let (x, z) = (player.x.floor() as i32, player.z.floor() as i32);
        let r = reach as i32 + fauna::SQUARE;
        let (lo, hi) = (fauna::square_of(x - r, z - r), fauna::square_of(x + r, z + r));
        for sz in lo.1..=hi.1 {
            for sx in lo.0..=hi.0 {
                let sq = (sx, sz);
                if self.squares.contains_key(&sq) || self.asked.contains(&sq) {
                    continue;
                }
                // Most squares have no herd by the dice alone, and need no more thought.
                if fauna::home_of(seed, sq).is_none() {
                    self.squares.insert(sq, None);
                } else if self.at_once {
                    if let Ok(site) = fauna::site(&self.world, sq) {
                        self.squares.insert(sq, site);
                    }
                } else {
                    self.asked.insert(sq);
                    let _ = self.worker().ask.send(Job::Square(sq));
                }
            }
        }
        let here = DVec2::new(player.x, player.z);
        let mut new = Vec::new();
        for (sq, site) in &self.squares {
            let Some(site) = site else { continue };
            let home = DVec2::new(site.home.x as f64, site.home.z as f64);
            let id = Id::Wild(*sq);
            if home.distance(here) < reach && !self.herds.iter().any(|h| h.0 == id) {
                new.push((id, Herd::wild(site)));
            }
        }
        // In the same order every time, whatever order the map keeps them in.
        new.sort_by_key(|(id, _)| match id {
            Id::Wild(sq) => *sq,
            _ => (0, 0),
        });
        for (id, h) in new {
            self.add(id, h);
        }
        // Forget the squares long left behind, so the map stays small.
        let far = (8 * fauna::SQUARE) as f64;
        self.squares.retain(|sq, _| {
            let mid = DVec2::new((sq.0 as f64 + 0.5) * fauna::SQUARE as f64, (sq.1 as f64 + 0.5) * fauna::SQUARE as f64);
            mid.distance(here) < far
        });
    }

    /// Stands `count` kept animals of `kind` inside `pen` (a pen, a coop's yard or a
    /// pasture), to graze and mill about in it, treating its fence and its shut gate
    /// as a wall. `key` names the pen: asking again with the same key does nothing, so
    /// whoever lays out the settlements can hand over the pens about the player every
    /// second or so. Herds far out of sight are let go, and stood up again (the same
    /// animals) the next time their pen is handed over near. False when the pen is
    /// beyond reach of the player and nothing was done.
    pub fn keep(&mut self, key: u64, kind: Kind, count: usize, pen: Pen) -> bool {
        let id = Id::Kept(key);
        if self.herds.iter().any(|h| h.0 == id) {
            return true;
        }
        let Some(player) = self.player else { return false };
        let mid = pen.middle();
        if mid.distance(DVec2::new(player.x, player.z)) > m64(REACH as f64) || pen.fence.len() < 3 {
            return false;
        }
        let seed = hash2(self.world.seed() ^ 0x4e7a, key as i32, (key >> 32) as i32);
        let reason = format!("kept in a pen of {:.0} square metres", pen.area());
        self.add(id, Herd::kept(kind, count.max(1), pen, player.y, seed, reason));
        true
    }

    /// Lets go of the animals of the pen `key`.
    pub fn release(&mut self, key: u64) {
        self.herds.retain(|h| h.0 != Id::Kept(key));
        self.fenced.remove(&Id::Kept(key));
    }

    /// Where the player is taken to be until the first update: what [`Self::keep`]
    /// measures its reach from.
    pub fn set_player(&mut self, player: DVec3) {
        self.player = Some(player);
    }

    /// A herd of `count` of `kind` (as many as a herd of them has, if not given) about
    /// `at`, from the console or a flag. Wild ones keep their distance as any wild herd
    /// does; kept ones wander loose.
    pub fn call(&mut self, kind: Kind, count: Option<usize>, at: DVec3) -> usize {
        self.called += 1;
        let h = kind.habits();
        let n = count.unwrap_or(((h.herd.0 as usize + h.herd.1 as usize) / 2).max(2)).clamp(1, 40);
        let seed = hash2(self.world.seed() ^ 0xca11, at.x.floor() as i32, at.z.floor() as i32 ^ self.called as i32);
        self.add(Id::Called(self.called), Herd::loose(kind, n, at, seed, "called up".into()));
        n
    }

    /// A pen of `kind` with as many in it as one holds, about `at`, its gate toward
    /// `from` and a hurdle fence drawn round it: what kept animals look like before the
    /// settlements hand over their own pens.
    pub fn pen_ahead(&mut self, kind: Kind, at: DVec2, from: DVec2) -> (Pen, usize) {
        let (n, each) = pen_size(kind);
        let side = (n as f64 * each).sqrt() * VOXELS_PER_METRE as f64;
        let to = from - at;
        let turn = to.y.atan2(to.x);
        let half = DVec2::new(side * 0.6, side * 0.45).max(DVec2::splat(m64(1.6)));
        let pen = Pen::rect(at, half, turn, Some(from));
        self.called += 1;
        let key = (1u64 << 63) | self.called as u64;
        if self.keep(key, kind, n, pen.clone()) {
            self.fenced.insert(Id::Kept(key));
        }
        (pen, n)
    }

    pub fn have_sites(&self) -> bool {
        !self.squares.is_empty()
    }

    pub fn herds(&self) -> impl Iterator<Item = &Herd> {
        self.herds.iter().map(|h| &h.1)
    }

    /// Draws every animal in sight of `cam`, nearest first, as near or far as each is,
    /// within [`BUDGET`] boxes; and the fences of the pens set out to be seen.
    pub fn draw(&self, cam: DVec3, out: &mut Vec<ParticleInstance>) {
        let mut seen: Vec<(f64, usize, usize, Detail)> = Vec::new();
        for (i, (_, h)) in self.herds.iter().enumerate() {
            if h.ground.is_none() {
                continue;
            }
            for (j, b) in h.beasts.iter().enumerate() {
                let d = b.pos.distance(cam);
                let detail = match model::detail(&b.look, d) {
                    Detail::Near if self.far => Detail::Far,
                    x => x,
                };
                if detail != Detail::Gone {
                    seen.push((d, i, j, detail));
                }
            }
        }
        seen.sort_by(|a, b| a.0.total_cmp(&b.0));
        let start = out.len();
        for (_, i, j, detail) in seen {
            let used = out.len() - start;
            let detail = if detail == Detail::Near && used + model::NEAR_MOST > BUDGET { Detail::Far } else { detail };
            if used + model::FAR_MOST > BUDGET {
                break;
            }
            let b = &self.herds[i].1.beasts[j];
            model::draw_at(&b.look, detail, b.pos, b.facing, &b.pose, cam, out);
        }
        for (id, h) in &self.herds {
            if let (true, Fold::Pen(pen), Some(g)) = (self.fenced.contains(id), &h.fold, &h.ground) {
                draw_fence(pen, g, cam, out);
            }
        }
    }

    /// Writes the herds near `player` to the log at `level`: what each is, how far off
    /// and which way, and why it is there.
    pub fn report(&self, player: DVec3, level: log::Level) {
        if !log::log_enabled!(target: "game::animals", level) {
            return;
        }
        if self.herds.is_empty() {
            log::log!(target: "game::animals", level, "no herds near");
            return;
        }
        let lines: Vec<String> = self.herds.iter().map(|(_, h)| describe(h, player)).collect();
        log::log!(target: "game::animals", level, "herds near: {}", lines.join("; "));
    }

    /// A line for each herd near `player`, nearest first, for the console.
    pub fn listed(&self, player: DVec3) -> Vec<String> {
        let mut herds: Vec<&Herd> = self.herds().collect();
        herds.sort_by(|a, b| a.at().distance(player).total_cmp(&b.at().distance(player)));
        herds.into_iter().map(|h| describe(h, player)).collect()
    }
}

/// A herd in words: how many of what, how far which way, doing what, and why there.
fn describe(h: &Herd, player: DVec3) -> String {
    let at = h.at();
    let off = DVec2::new(at.x - player.x, at.z - player.z);
    let metres = off.length() / m64(1.0);
    // North is toward -z, east toward +x, as the map has them.
    let points = ["east", "south-east", "south", "south-west", "west", "north-west", "north", "north-east"];
    let way = points[((off.y.atan2(off.x).to_degrees() + 360.0 + 22.5) / 45.0) as usize % 8];
    let fleeing = h.beasts.iter().filter(|b| b.act == Act::Flee).count();
    let grazing = h.beasts.iter().filter(|b| b.act == Act::Graze).count();
    let doing = if h.ground.is_none() {
        "not stood up yet".to_string()
    } else if fleeing > 0 {
        format!("{fleeing} going off")
    } else {
        format!("{grazing} grazing")
    };
    format!("{} x{} {metres:.0} m {way} at {:.0},{:.0}, {doing}: {}", h.kind.word(), h.beasts.len().max(if h.ground.is_none() { h.count } else { 0 }), at.x, at.z, h.reason)
}

/// How many a demonstration pen of `kind` holds, and the square metres each is given.
fn pen_size(kind: Kind) -> (usize, f64) {
    use Kind::*;
    match kind {
        Chicken => (8, 1.6),
        Duck | Goose => (6, 2.5),
        Sheep | Goat => (6, 5.0),
        Pig => (4, 5.0),
        Cattle => (4, 14.0),
        Ox => (2, 16.0),
        Pony | Donkey => (3, 16.0),
        Reindeer => (5, 10.0),
        _ => (4, 8.0),
    }
}

/// A hurdle fence along a pen's line: a post every couple of metres and two rails, the
/// gate a little paler.
fn draw_fence(pen: &Pen, g: &Ground, cam: DVec3, out: &mut Vec<ParticleInstance>) {
    let middle = pen.middle();
    if DVec2::new(cam.x, cam.z).distance(middle) > m64(90.0) {
        return;
    }
    let wood = [0.16f32, 0.1, 0.055, 1.0];
    let pale = [0.3f32, 0.22, 0.13, 1.0];
    let gate = pen.gate_at();
    let n = pen.fence.len();
    let floor = |p: DVec2| g.at(p).map_or(cam.y, |c| c.floor as f64);
    let mut push = |at: DVec3, size: [f32; 3], yaw: f32, color: [f32; 4]| {
        out.push(ParticleInstance { pos: (at - cam).as_vec3().into(), yaw, size, glow: 0.0, color, tilt: [0.0; 2], dissolve: 0.0 });
    };
    let v = VOXELS_PER_METRE;
    for k in 0..n {
        let (a, b) = (pen.fence[k], pen.fence[(k + 1) % n]);
        let len = a.distance(b);
        let dir = (b - a) / len.max(1e-9);
        let yaw = -(dir.y.atan2(dir.x) as f32) + std::f32::consts::FRAC_PI_2;
        let posts = (len / m64(2.0)).ceil().max(1.0) as usize;
        for p in 0..posts {
            let at = a + dir * (len * p as f64 / posts as f64);
            push(DVec3::new(at.x, floor(at) + m64(0.5), at.y), [0.1 * v, 1.0 * v, 0.1 * v], yaw, wood);
        }
        // The rails, a stretch between each pair of posts, following the ground.
        for p in 0..posts {
            let (s, e) = (a + dir * (len * p as f64 / posts as f64), a + dir * (len * (p + 1) as f64 / posts as f64));
            let mid = (s + e) * 0.5;
            let on_gate = gate.is_some_and(|q| q.distance(mid) < len / posts as f64 * 0.6);
            let colour = if on_gate { pale } else { wood };
            let span = (s.distance(e)) as f32;
            for h in [0.4, 0.85] {
                push(DVec3::new(mid.x, floor(mid) + m64(h), mid.y), [0.05 * v, 0.08 * v, span], yaw, colour);
            }
        }
    }
}

/// A gallery of animals on a screenshot (`--animals`), to see what the models make.
#[derive(Clone, Debug)]
pub struct Gallery {
    /// The kinds shown: one kind shows its male, its females and its young; several
    /// show one of each, the kept beside their wild kin.
    pub kinds: Vec<Kind>,
    /// How far ahead the first row stands, in metres.
    pub ahead: f64,
    /// Turned this far (radians) from side on to the camera.
    pub turn: f32,
    pub far: bool,
    /// Walking, trotting or running in place; grazing; or alert.
    pub gait: Option<Gait>,
    pub graze: bool,
    pub alert: bool,
}

impl Default for Gallery {
    fn default() -> Self {
        Gallery { kinds: Kind::ALL.to_vec(), ahead: 4.0, turn: 0.0, far: false, gait: None, graze: false, alert: false }
    }
}

/// An animal stood somewhere for a still: who, where, facing which way, and how.
pub type Standing = (Look, DVec3, f32, Pose);

/// Stands a gallery in rows before a camera at `eye` looking along `yaw`, each on the
/// ground `ground` gives, side on to the camera; `clock` seconds into its idling, for
/// motion between two stills. Who each is goes to the log.
pub fn stand_gallery(g: &Gallery, seed: u32, eye: DVec3, yaw: f32, clock: f32, ground: &dyn Fn(i32, i32) -> f64) -> Vec<Standing> {
    // One kind: a row of its own, the male first, females of every colour, the young.
    let looks: Vec<Vec<Look>> = if g.kinds.len() == 1 {
        let k = g.kinds[0];
        let mut row = vec![Look::new(k, hash2(seed, 0, 1), Role::Male)];
        row.extend((1..6).map(|i| Look::new(k, hash2(seed, i, 1), Role::Female)));
        row.extend((6..8).map(|i| Look::new(k, hash2(seed, i, 1), Role::Young)));
        vec![row]
    } else {
        // Rows by size, the small at the front.
        let mut rows: Vec<Vec<Look>> = vec![Vec::new(); 3];
        for (i, &k) in g.kinds.iter().enumerate() {
            let l = Look::new(k, hash2(seed, i as i32, 2), if i % 3 == 1 { Role::Male } else { Role::Female });
            let row = match model::Shape::of(k) {
                s if s.bird() => 0,
                model::Shape::Sheep | model::Shape::Goat | model::Shape::Pig => 1,
                _ => 2,
            };
            rows[row].push(l);
        }
        rows.retain(|r| !r.is_empty());
        rows
    };
    let (ahead, right) = (DVec3::new(yaw.cos() as f64, 0.0, yaw.sin() as f64), DVec3::new(-yaw.sin() as f64, 0.0, yaw.cos() as f64));
    let mut out = Vec::new();
    let mut on = g.ahead;
    log::info!("animals: {} rows", looks.len());
    for row in looks {
        let gap = row.iter().map(|l| l.width.max(0.3) as f64 + 0.5).fold(0.0, f64::max);
        let longest = row.iter().map(|l| l.reach() as f64).fold(0.0, f64::max);
        let n = row.len();
        for (i, l) in row.into_iter().enumerate() {
            let across = (i as f64 - (n as f64 - 1.0) * 0.5) * (longest * 0.75 + gap).max(gap * 2.0);
            let at = eye + ahead * m64(on) + right * m64(across);
            let feet = DVec3::new(at.x, ground(at.x.floor() as i32, at.z.floor() as i32), at.z);
            // Side on, nose to the camera's right.
            let facing = yaw + std::f32::consts::FRAC_PI_2 + g.turn;
            let k = i as f32;
            let pose = Pose {
                stride: clock * 4.0 + k * 0.9,
                gait: g.gait.unwrap_or(Gait::Walk),
                reach: if g.gait.is_some() { 1.0 } else { 0.0 },
                graze: if g.graze { 1.0 } else { 0.0 },
                alert: if g.alert { 1.0 } else { 0.0 },
                look: if g.alert { -0.7 } else { ((k * 1.3 + clock * 0.5).sin()) * 0.25 },
                ears: (clock * 3.0 + k).sin() * (clock * 0.4 + k).sin().max(0.0),
                tail: (clock * 2.6 + k).sin(),
                flap: if g.gait == Some(Gait::Run) { 0.8 } else { 0.0 },
                ..Default::default()
            };
            let mut boxes = Vec::new();
            model::draw(&l, feet, facing, &pose, eye, &mut boxes);
            let near = boxes.len();
            boxes.clear();
            model::draw_far(&l, feet, facing, &pose, eye, &mut boxes);
            log::info!("  {near:>2} boxes near, {} far: {}", boxes.len(), l.describe());
            out.push((l, feet, facing, pose));
        }
        on += longest * 1.1 + 1.2;
    }
    out
}

/// Draws a gallery's animals, as near or far as each is from `cam`; with `far`, all as
/// from far off.
pub fn draw_gallery(standing: &[Standing], far: bool, cam: DVec3, out: &mut Vec<ParticleInstance>) {
    for (l, feet, facing, pose) in standing {
        let d = feet.distance(cam);
        let detail = match model::detail(l, d) {
            Detail::Near if far => Detail::Far,
            x => x,
        };
        model::draw_at(l, detail, *feet, *facing, pose, cam, out);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use super::*;

    fn world() -> Arc<GameWorld> {
        static WORLD: OnceLock<Arc<GameWorld>> = OnceLock::new();
        WORLD
            .get_or_init(|| {
                let w = Arc::new(GameWorld::new(1));
                w.continent(0, 0);
                w
            })
            .clone()
    }

    /// A place on continent (0, 0) of world 1 with a wild herd near.
    fn herd_country() -> (DVec3, Site) {
        let w = world();
        for sz in 20..60 {
            for sx in 20..60 {
                if let Ok(Some(s)) = fauna::site(&w, (sx, sz)) {
                    return (DVec3::new(s.home.x as f64, s.home.y as f64, s.home.z as f64), s);
                }
            }
        }
        panic!("no herd anywhere near the middle of continent 0, 0");
    }

    #[test]
    fn herds_come_back_the_same_when_the_player_returns() {
        let (home, site) = herd_country();
        let mut herds = Herds::at_once(world(), true);
        let near = home + DVec3::new(m64(60.0), 0.0, 0.0);
        herds.update(1.0 / 30.0, near);
        let found = |h: &Herds| -> Vec<(Kind, usize, Vec<Look>)> {
            let mut v: Vec<_> = h.herds().map(|h| (h.kind, h.beasts.len(), h.beasts.iter().map(|b| b.look).collect())).collect();
            v.sort_by_key(|x| (x.0 as u8, x.1));
            v
        };
        let first = found(&herds);
        assert!(first.iter().any(|f| f.0 == site.kind), "the herd of {} is not there", site.kind.word());
        let starts: Vec<DVec3> = herds.herds().flat_map(|h| h.beasts.iter().map(|b| b.pos)).collect();
        // Away over the hills, and back.
        herds.update(1.0 / 30.0, near + DVec3::new(m64(3000.0), 0.0, 0.0));
        assert_eq!(herds.herds().count(), 0, "the herds were not let go");
        herds.update(1.0 / 30.0, near);
        herds.look_in = 0.0;
        herds.update(1.0 / 30.0, near);
        assert_eq!(found(&herds), first, "a different herd when the player came back");
        let again: Vec<DVec3> = herds.herds().flat_map(|h| h.beasts.iter().map(|b| b.pos)).collect();
        assert_eq!(starts.len(), again.len());
        // And another world's herds stood up the same way from the start.
        let mut other = Herds::at_once(world(), true);
        other.update(1.0 / 30.0, near);
        assert_eq!(found(&other), first);
    }

    #[test]
    fn a_herd_stands_on_ground_it_can_stand_on() {
        let (home, _) = herd_country();
        let mut herds = Herds::at_once(world(), true);
        herds.update(1.0 / 30.0, home + DVec3::new(m64(200.0), 0.0, 0.0));
        for h in herds.herds() {
            let g = h.ground.as_ref().expect("read at once");
            assert!(!h.beasts.is_empty(), "no {} could stand", h.kind.word());
            for b in &h.beasts {
                let c = g.at(DVec2::new(b.pos.x, b.pos.z)).unwrap();
                assert!(c.water as i32 <= h.habits.wade || h.habits.swims, "{} stood in water {} deep", h.kind.word(), c.water);
                assert!(c.room as i32 >= h.habits.height.min(c.room as i32), "{}", h.kind.word());
            }
        }
    }

    #[test]
    fn the_animals_keep_to_their_budget_however_many_there_are() {
        let mut herds = Herds::at_once(world(), false);
        herds.set_player(DVec3::ZERO);
        let g = Arc::new(Ground::from_fn(-400, -400, 800, 800, |_, _| ground::Cell::open(0)));
        for (k, kind) in Kind::ALL.into_iter().cycle().take(40).enumerate() {
            let at = DVec3::new((k % 8) as f64 * 30.0 - 120.0, 0.0, (k / 8) as f64 * 30.0 - 60.0);
            herds.herds.push((Id::Called(k as u32), Herd::loose(kind, 12, at, k as u32, String::new())));
            let h = &mut herds.herds.last_mut().unwrap().1;
            h.stand_up(&g);
            h.ground = Some(g.clone());
        }
        let mut out = Vec::new();
        herds.draw(DVec3::new(0.0, 4.0, 0.0), &mut out);
        assert!(out.len() <= BUDGET, "{} boxes", out.len());
        assert!(out.len() > BUDGET / 2, "only {} boxes for {} animals", out.len(), herds.herds().map(|h| h.beasts.len()).sum::<usize>());
    }

    #[test]
    fn a_pen_handed_over_is_stood_up_once() {
        let mut herds = Herds::at_once(world(), false);
        let (home, _) = herd_country();
        herds.set_player(home);
        let pen = Pen::rect(DVec2::new(home.x + 20.0, home.z), DVec2::new(10.0, 8.0), 0.0, None);
        assert!(herds.keep(9, Kind::Sheep, 5, pen.clone()));
        assert!(herds.keep(9, Kind::Sheep, 5, pen.clone()));
        assert_eq!(herds.herds().count(), 1);
        // Too far off to bother with.
        let far = Pen::rect(DVec2::new(home.x + 5000.0, home.z), DVec2::new(10.0, 8.0), 0.0, None);
        assert!(!herds.keep(10, Kind::Sheep, 5, far));
        herds.release(9);
        assert_eq!(herds.herds().count(), 0);
    }
}
