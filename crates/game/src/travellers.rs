//! Travellers on the roads and the water (#205): lone campers by the long trails, parties
//! of settlers walking between settlements, trade boats on the rivers between partners,
//! and fishing boats out from the ports, drawn about the player.
//!
//! Who is where comes from worldgen's timetables (`settlements::travel`), worked out from
//! the world's seed, the day and the hour: nothing is simulated here, so every player sees
//! the same travellers in the same places (#36). The one thing kept is which parties of
//! settlers the player has met, so that the map can show where they have got to.
//!
//! Each continent's timetable is made once, on a worker thread, the first time the player
//! is on it. The day's plans for the parties near the player are found on a worker too, as
//! the player moves and the days turn; where each party is this frame follows from its
//! plan and the hour, which is a quick sum.
//!
//! The people are drawn with `people::draw`, as near or as far as its own detail levels
//! say. The poses they need that the figures do not have yet (sitting on the ground,
//! fishing, sketching, rowing, asleep) are made here from the pose's parts, each marked
//! where a pose of the figures' own should be swapped in.

use std::{collections::HashMap, sync::Arc, thread::JoinHandle};

use glam::{DVec2, DVec3, Vec2, Vec3};
use worldgen::{
    continent::Continent,
    macro_layout::{MACRO_SIZE, cell_of},
    noise::{hash2, unit},
    scale::{m, m64},
    settlements::{
        Region,
        travel::{self, Boat, Cart, Doing, Figure, Fire, Kind, Plan, Scene, Travel, Who, Work},
    },
    world::GameWorld,
};

use crate::{
    character::{self, Gait, Pose},
    inventory,
    particles::ParticleInstance,
    people::{self, Age, Person, Trade},
    placed,
};

/// How far about the player the day's plans are found, and how far the player goes
/// before they are found again.
const PLAN_REACH: f32 = m(300.0);
const REPLAN: f64 = m64(70.0);
/// Beyond this nothing of a party is drawn: its people are gone from sight before it
/// (`character::SIGHT`), and its boats, carts and tents soon after.
const SEEN: f64 = m64(230.0);
/// Most parties drawn at once, nearest first.
const MOST: usize = 10;
/// Most boxes the travellers take of the props' buffer in a frame, nearest first: what a
/// party of eight settlers with their carts and fire takes close to, and a boat or two.
pub const BUDGET: usize = 640;
/// Within this a party of settlers counts as met, and goes on the map.
const MET: f64 = m64(60.0);
/// Fires burning further off than this throw no light the player would see by.
const LIGHT: f64 = m64(90.0);

/// The days and the hour, as the game keeps them.
#[derive(Clone, Copy, Debug)]
pub struct When {
    pub hour: f32,
    pub date: f32,
    pub year: u32,
    pub year_days: f32,
    /// How long a day lasts, in real minutes: how fast the walkers' legs go.
    pub day_minutes: f32,
}

impl When {
    pub fn day(&self) -> i64 {
        travel::day_of(self.year, self.date, self.hour, self.year_days)
    }
}

/// A continent's timetable: being made, or made.
enum Table {
    Making(JoinHandle<Travel>, Arc<Continent>),
    Ready(Arc<Travel>, Arc<Continent>),
}

/// Where and for which day a set of plans was found.
#[derive(Clone, Copy, PartialEq)]
struct Planned {
    cell: (i32, i32),
    day: i64,
    at: DVec2,
}

/// A party as it is this frame, and where its continent lies in the world.
pub struct Seen {
    pub scene: Scene,
    origin: DVec3,
    pub distance: f64,
}

impl Seen {
    fn world(&self, p: Vec3) -> DVec3 {
        self.origin + p.as_dvec3()
    }
}

/// One of the parties of settlers met: on which continent, and which.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Met {
    cell: (i32, i32),
    id: u64,
}

pub struct Travellers {
    tables: HashMap<(i32, i32), Table>,
    plans: Vec<Plan>,
    planned: Option<Planned>,
    planning: Option<(Planned, JoinHandle<Vec<Plan>>)>,
    /// This frame's parties near the player, nearest first.
    pub seen: Vec<Seen>,
    /// The people met with so far, by seed, made once each.
    people: HashMap<u32, Person>,
    met: Vec<Met>,
    /// The met have changed since they were last saved.
    pub unsaved: bool,
    when: When,
    /// How many boxes were drawn last frame.
    pub drawn: usize,
}

impl Travellers {
    pub fn new() -> Self {
        Self {
            tables: HashMap::new(),
            plans: Vec::new(),
            planned: None,
            planning: None,
            seen: Vec::new(),
            people: HashMap::new(),
            met: Vec::new(),
            unsaved: false,
            when: When { hour: 12.0, date: 0.0, year: 0, year_days: 24.0, day_minutes: 40.0 },
            drawn: 0,
        }
    }

    /// As saved: the parties of settlers met, a line each.
    pub fn from_text(text: &str) -> Self {
        let mut t = Self::new();
        for line in text.lines() {
            let w: Vec<&str> = line.split_whitespace().collect();
            if let ["met", mx, mz, id] = w.as_slice()
                && let (Ok(mx), Ok(mz), Ok(id)) = (mx.parse(), mz.parse(), id.parse())
            {
                t.met.push(Met { cell: (mx, mz), id });
            }
        }
        t
    }

    pub fn to_text(&self) -> String {
        self.met.iter().map(|m| format!("met {} {} {}\n", m.cell.0, m.cell.1, m.id)).collect()
    }

    /// Looks for the travellers about `camera` as the game runs: the timetables and plans
    /// on worker threads, never waiting for them, and where everyone is at this moment.
    /// `calm` is whether the water is fit for boats.
    pub fn update(&mut self, world: &GameWorld, camera: DVec3, when: When, calm: bool) {
        self.when = when;
        let cell = cell_of(camera.x.floor() as i32, camera.z.floor() as i32);
        // The timetable of the continent the player is on.
        if !self.tables.contains_key(&cell)
            && let Some(c) = world.continent_if_ready(cell.0, cell.1)
        {
            let (seed, made) = (world.seed(), c.clone());
            self.tables.insert(cell, Table::Making(std::thread::spawn(move || Travel::of(&made, seed)), c));
        }
        if matches!(self.tables.get(&cell), Some(Table::Making(job, _)) if job.is_finished())
            && let Some(Table::Making(job, c)) = self.tables.remove(&cell)
        {
            match job.join() {
                Ok(t) => {
                    self.tables.insert(cell, Table::Ready(Arc::new(t), c));
                }
                Err(_) => log::warn!("the travellers' timetable for continent {cell:?} could not be made"),
            }
        }
        // The day's plans about the player, found again as the day turns or the player
        // moves on.
        let want = Planned { cell, day: when.day(), at: DVec2::new(camera.x, camera.z) };
        if let Some((p, job)) = self.planning.take() {
            if job.is_finished() {
                if let Ok(plans) = job.join() {
                    (self.plans, self.planned) = (plans, Some(p));
                }
            } else {
                self.planning = Some((p, job));
            }
        }
        let stale = self.planned.is_none_or(|p| p.cell != want.cell || p.day != want.day || p.at.distance(want.at) > REPLAN);
        if stale
            && self.planning.is_none()
            && let Some(Table::Ready(t, c)) = self.tables.get(&cell)
        {
            let (t, c) = (t.clone(), c.clone());
            let at = local(camera, cell);
            self.planning = Some((want, std::thread::spawn(move || t.near(&c, want.day, at, PLAN_REACH))));
        }
        self.place(camera, calm);
    }

    /// Looks for the travellers about `camera` at once, waiting for whatever it needs:
    /// for a screenshot.
    pub fn settle(&mut self, world: &GameWorld, camera: DVec3, when: When, calm: bool) {
        self.when = when;
        let cell = cell_of(camera.x.floor() as i32, camera.z.floor() as i32);
        let c = world.continent(cell.0, cell.1);
        let t = Arc::new(Travel::of(&c, world.seed()));
        let want = Planned { cell, day: when.day(), at: DVec2::new(camera.x, camera.z) };
        self.plans = t.near(&c, want.day, local(camera, cell), PLAN_REACH);
        self.planned = Some(want);
        self.tables.insert(cell, Table::Ready(t, c));
        self.place(camera, calm);
    }

    /// Where everyone planned for is this moment, the nearest first, and who is met.
    fn place(&mut self, camera: DVec3, calm: bool) {
        self.seen.clear();
        let Some(planned) = self.planned else { return };
        // Plans for another day are no use: they would put everyone where they were then.
        if planned.day != self.when.day() {
            return;
        }
        let origin = DVec3::new((planned.cell.0 * MACRO_SIZE) as f64, 0.0, (planned.cell.1 * MACRO_SIZE) as f64);
        for plan in &self.plans {
            let Some(scene) = plan.at(self.when.hour, self.when.date, calm) else { continue };
            let distance = (origin + scene.at.as_dvec3()).distance(camera);
            if distance < SEEN {
                self.seen.push(Seen { scene, origin, distance });
            }
        }
        self.seen.sort_by(|a, b| a.distance.total_cmp(&b.distance).then(a.scene.id.cmp(&b.scene.id)));
        self.seen.truncate(MOST);
        for s in &self.seen {
            let met = Met { cell: planned.cell, id: s.scene.id };
            if s.scene.kind == Kind::Settlers && s.distance < MET && !self.met.contains(&met) {
                log::info!("met {} on the road, {}", s.scene.kind.word(), s.scene.doing);
                self.met.push(met);
                self.unsaved = true;
            }
        }
    }

    /// Where each party of settlers met is now, in world voxels, for the map; those whose
    /// journey is over are forgotten.
    pub fn marks(&mut self) -> Vec<DVec2> {
        let (day, hour) = (self.when.day(), self.when.hour);
        let mut out = Vec::new();
        let tables = &self.tables;
        let before = self.met.len();
        self.met.retain(|m| match tables.get(&m.cell) {
            Some(Table::Ready(t, _)) => match t.settlers_where(m.id, day, hour) {
                Some(p) => {
                    out.push(DVec2::new((m.cell.0 * MACRO_SIZE) as f64 + p.x as f64, (m.cell.1 * MACRO_SIZE) as f64 + p.y as f64));
                    true
                }
                None => false,
            },
            // Not known yet: kept until it is.
            _ => true,
        });
        self.unsaved |= self.met.len() != before;
        out
    }

    /// The fires burning or smouldering among the travellers, and how much each smokes,
    /// for the smoke.
    pub fn smoking(&self) -> Vec<(DVec3, f32)> {
        self.seen
            .iter()
            .filter_map(|s| {
                let (at, fire) = s.scene.fire?;
                Some((s.world(at) + DVec3::new(0.0, m64(0.3), 0.0), if fire == Fire::Lit { 1.0 } else { 0.3 }))
            })
            .collect()
    }

    /// The hearts of the fires burning near enough to light the ground about them.
    pub fn burning(&self, camera: DVec3) -> Vec<DVec3> {
        self.seen
            .iter()
            .filter_map(|s| {
                let (at, fire) = s.scene.fire?;
                let heart = s.world(at) + DVec3::new(0.0, m64(0.3), 0.0);
                (fire == Fire::Lit && heart.distance(camera) < LIGHT).then_some(heart)
            })
            .collect()
    }

    /// Draws everyone near, nearest first, up to [`BUDGET`] boxes in all. `clock` is in
    /// seconds, for the legs, the oars and the flames.
    pub fn draw(&mut self, cam: DVec3, clock: f32, out: &mut Vec<ParticleInstance>) {
        let start = out.len();
        let pace = travel::PACE / m(1.0) / (self.when.day_minutes.max(0.1) * 60.0 / 24.0);
        let seen = std::mem::take(&mut self.seen);
        for s in &seen {
            if out.len() - start >= BUDGET {
                break;
            }
            let mut d = Drawing { cam, clock, pace, out, start, people: &mut self.people, region: s.scene.region };
            d.scene(s);
        }
        self.seen = seen;
        self.drawn = out.len() - start;
        // People met are kept only while there is some chance of meeting them again.
        if self.people.len() > 256 {
            self.people.clear();
        }
    }
}

/// Whether the water is calm enough for the boats to go out under `sky`: not in a gale,
/// nor in heavy rain or snow.
pub fn calm(sky: &crate::weather::Sky) -> bool {
    sky.storm < 0.35 && sky.rain < 0.6 && sky.snow < 0.6
}

/// `p` in the continent of `cell`.
fn local(p: DVec3, cell: (i32, i32)) -> Vec2 {
    Vec2::new((p.x - (cell.0 * MACRO_SIZE) as f64) as f32, (p.z - (cell.1 * MACRO_SIZE) as f64) as f32)
}

/// One of the figures' cells, in voxels: a figure is eighteen of them tall.
const CELL: f32 = m(1.76) / 18.0;
/// How high the hips are over the feet standing, in the figures' cells.
const HIPS: f32 = 9.0;

/// Colours, linear.
const WOOD: [f32; 3] = [0.21, 0.14, 0.085];
const WOOD_DARK: [f32; 3] = [0.12, 0.08, 0.05];
const WOOD_LIGHT: [f32; 3] = [0.42, 0.28, 0.15];
const HULL: [f32; 3] = [0.21, 0.13, 0.07];
const CANVAS: [[f32; 3]; 3] = [[0.55, 0.48, 0.36], [0.33, 0.38, 0.27], [0.48, 0.28, 0.20]];
const SACK: [f32; 3] = [0.62, 0.55, 0.42];
const IRON: [f32; 3] = [0.08, 0.08, 0.08];
const WOOL: [[f32; 3]; 3] = [[0.40, 0.22, 0.14], [0.22, 0.26, 0.34], [0.35, 0.34, 0.22]];
const NET: [f32; 3] = [0.20, 0.24, 0.22];
const WICKER: [f32; 3] = [0.50, 0.38, 0.20];
const PAPER: [f32; 3] = [0.85, 0.82, 0.72];

/// Boxes set out in the frame of something standing at `base` (camera-relative, voxels)
/// and turned to `heading`: x ahead, y up and z across, in metres.
struct Brush<'a> {
    out: &'a mut Vec<ParticleInstance>,
    base: Vec3,
    heading: f32,
}

impl Brush<'_> {
    fn put(&mut self, at: [f32; 3], size: [f32; 3], color: [f32; 3], turn: f32, tilt: [f32; 2]) {
        let (s, c) = self.heading.sin_cos();
        let p = Vec3::from(at) * m(1.0);
        self.out.push(ParticleInstance {
            pos: (self.base + Vec3::new(p.x * c - p.z * s, p.y, p.x * s + p.z * c)).into(),
            yaw: self.heading + turn,
            size: (Vec3::from(size) * m(1.0)).into(),
            glow: 0.0,
            color: [color[0], color[1], color[2], 1.0],
            tilt,
            dissolve: 0.0,
        });
    }

    fn block(&mut self, at: [f32; 3], size: [f32; 3], color: [f32; 3]) {
        self.put(at, size, color, 0.0, [0.0; 2]);
    }
}

/// Drawing one party.
struct Drawing<'a> {
    cam: DVec3,
    clock: f32,
    /// How fast the walkers go on the screen, in metres a second.
    pace: f32,
    out: &'a mut Vec<ParticleInstance>,
    start: usize,
    people: &'a mut HashMap<u32, Person>,
    region: Region,
}

impl Drawing<'_> {
    fn full(&self) -> bool {
        self.out.len() - self.start >= BUDGET
    }

    fn brush(&mut self, at: DVec3, heading: f32) -> Brush<'_> {
        Brush { base: (at - self.cam).as_vec3(), heading, out: self.out }
    }

    fn scene(&mut self, s: &Seen) {
        let scene = &s.scene;
        // The fire first: it is what is seen of a camp from furthest off, by night.
        if let Some((at, fire)) = scene.fire {
            let mut thing = placed::Thing::new(inventory::campfire(), 1, s.world(at), unit(hash2(scene.id as u32, 3, 9)) * 6.0);
            thing.state = if fire == Fire::Lit { placed::BURNING } else { placed::OUT };
            placed::draw_one(&thing, self.cam, self.clock, self.out);
            if fire == Fire::Low {
                // Banked for the night: no flame, but the embers under the sticks still
                // glow, slowly, so a camp can be found in the dark.
                let breath = 0.5 + 0.5 * (self.clock * 0.9 + unit(scene.id as u32) * 6.0).sin();
                let mut b = self.brush(s.world(at), 0.4);
                b.block([0.0, 0.05, 0.0], [0.34, 0.05, 0.34], [0.85, 0.16, 0.02]);
                b.out.last_mut().expect("just drawn").glow = 0.45 + 0.35 * breath;
            }
        }
        for &(at, heading) in &scene.tents {
            if self.full() {
                return;
            }
            self.tent(s.world(at), heading, hash2(scene.id as u32, at.x as i32, at.z as i32));
        }
        for &(at, heading) in &scene.bedrolls {
            if self.full() {
                return;
            }
            self.bedroll(s.world(at), heading, hash2(scene.id as u32, at.x as i32, 7));
        }
        for cart in &scene.carts {
            if self.full() {
                return;
            }
            let pulled = scene.people.iter().any(|f| f.doing == Doing::Pulling);
            self.cart(s.world(cart.at), cart, pulled);
        }
        for boat in &scene.boats {
            if self.full() {
                return;
            }
            self.boat(s.world(boat.at), boat, scene.kind);
        }
        for (k, f) in scene.people.iter().enumerate() {
            if self.full() {
                return;
            }
            let bob = scene.boats.first().map_or(0.0, |b| bob(s.world(b.at), self.clock));
            self.figure(f, s.world(f.feet) + DVec3::new(0.0, bob, 0.0), k, scene.kind, scene.fire.is_some());
        }
    }

    /// A small tent, its ridge along `heading`, in stepped canvas.
    fn tent(&mut self, at: DVec3, heading: f32, salt: u32) {
        let canvas = CANVAS[(salt % 3) as usize];
        let mut b = self.brush(at, heading);
        // A ridge tent two metres long, a metre high and a metre and a half across at
        // the foot: its ends filled in steps under the slope, then the two sides laid
        // over them, leaning in to meet under the ridge pole.
        let (half, high) = (0.75f32, 1.0f32);
        for k in 0..4 {
            let (lo, hi) = (k as f32 * high / 4.0, (k + 1) as f32 * high / 4.0);
            let wide = 2.0 * half * (1.0 - hi / high) + 0.08;
            b.block([0.0, (lo + hi) / 2.0, 0.0], [1.96, hi - lo, wide], shade(canvas, 0.82));
        }
        let (slope, lean) = ((half * half + high * high).sqrt(), half.atan2(high));
        for side in [-1.0f32, 1.0] {
            // A tilt about the length leans the top of a side toward -z for a positive
            // angle, so each side leans away from its own.
            b.put([0.0, high / 2.0, side * half / 2.0], [2.0, slope, 0.05], canvas, 0.0, [-side * lean, 0.0]);
        }
        b.block([0.0, high + 0.02, 0.0], [2.2, 0.07, 0.07], WOOD);
        // The way in, dark, at the end toward the fire.
        b.block([1.0, 0.3, 0.0], [0.04, 0.55, 0.45], [0.04, 0.035, 0.03]);
    }

    /// A bedroll laid out, its foot toward `heading`, rolled up a little at its head.
    fn bedroll(&mut self, at: DVec3, heading: f32, salt: u32) {
        let wool = WOOL[(salt % 3) as usize];
        let mut b = self.brush(at, heading);
        b.block([0.0, 0.05, 0.0], [1.9, 0.1, 0.72], wool);
        b.block([-0.85, 0.12, 0.0], [0.26, 0.2, 0.72], shade(wool, 1.2));
    }

    /// A handcart: a bed on two wheels, its shafts ahead, held up by whoever pulls it or
    /// let down to the ground; and what it carries.
    fn cart(&mut self, at: DVec3, cart: &Cart, pulled: bool) {
        let mut b = self.brush(at, cart.heading);
        b.block([0.0, 0.62, 0.0], [1.5, 0.12, 1.0], WOOD);
        for side in [-1.0f32, 1.0] {
            b.block([0.0, 0.79, 0.48 * side], [1.5, 0.22, 0.06], WOOD_LIGHT);
            b.block([0.0, 0.38, 0.58 * side], [0.76, 0.76, 0.08], WOOD_DARK);
            b.block([0.0, 0.38, 0.64 * side], [0.14, 0.14, 0.05], IRON);
        }
        b.block([-0.74, 0.79, 0.0], [0.06, 0.22, 1.0], WOOD_LIGHT);
        b.block([0.0, 0.38, 0.0], [0.1, 0.1, 1.2], WOOD_DARK);
        // The shafts: up at the puller's hands, or resting on the ground.
        let (rise, mid) = if pulled { (0.17f32, 0.74) } else { (-0.43f32, 0.36) };
        for side in [-1.0f32, 1.0] {
            b.put([1.33, mid, 0.32 * side], [1.25, 0.07, 0.07], WOOD, 0.0, [0.0, rise]);
        }
        // The load: a covered bundle, or sacks and a barrel or two.
        let h = cart.load;
        if unit(h) < 0.35 {
            b.block([-0.05, 0.86, 0.0], [1.3, 0.36, 0.86], CANVAS[(h % 3) as usize]);
            b.block([-0.05, 1.05, 0.0], [1.34, 0.03, 0.04], WOOD_DARK);
        } else {
            let n = 2 + (h >> 4) % 3;
            for k in 0..n {
                let x = -0.5 + k as f32 * 0.45;
                let z = if k % 2 == 0 { -0.2 } else { 0.22 };
                if (h >> (8 + k)) & 1 == 0 {
                    b.block([x, 0.93, z], [0.44, 0.5, 0.44], shade(WOOD_LIGHT, 0.8));
                    b.block([x, 0.93, z], [0.46, 0.06, 0.46], IRON);
                } else {
                    b.put([x, 0.86, z], [0.5, 0.34, 0.4], shade(SACK, 0.9 + 0.2 * unit(h.rotate_left(k))), 0.3 * k as f32, [0.0; 2]);
                }
            }
        }
    }

    /// A boat: a sailing boat as the game's own, or a rowing boat, its oars working while
    /// it is under way; with its cargo, or a fishing boat's nets.
    fn boat(&mut self, at: DVec3, boat: &Boat, kind: Kind) {
        let clock = self.clock;
        let at = at + DVec3::new(0.0, bob(at, clock), 0.0);
        if boat.sail {
            crate::vehicles::draw_boat(at, boat.heading, clock, self.cam, self.out);
        } else {
            // The game's hull, without its mast.
            let mut b = self.brush(at, boat.heading);
            const O: f32 = 0.8;
            b.block([0.0, 0.05 * O, 0.0], [4.6 * O, 0.3 * O, 1.5 * O], HULL);
            b.block([0.0, 0.45 * O, 0.85 * O], [4.6 * O, 0.6 * O, 0.2 * O], WOOD_LIGHT);
            b.block([0.0, 0.45 * O, -0.85 * O], [4.6 * O, 0.6 * O, 0.2 * O], WOOD_LIGHT);
            b.block([-2.3 * O, 0.45 * O, 0.0], [0.2 * O, 0.6 * O, 1.9 * O], WOOD_LIGHT);
            b.block([2.65 * O, 0.3 * O, 0.0], [0.9 * O, 0.5 * O, 1.1 * O], HULL);
            b.block([3.25 * O, 0.5 * O, 0.0], [0.5 * O, 0.5 * O, 0.6 * O], WOOD_LIGHT);
            b.block([-1.0 * O, 0.4 * O, 0.0], [0.5 * O, 0.12 * O, 1.5 * O], WOOD_LIGHT);
            b.block([1.1 * O, 0.4 * O, 0.0], [0.5 * O, 0.12 * O, 1.5 * O], WOOD_LIGHT);
            // The oars, swept fore and aft and dipped as they pull; shipped along the
            // sides when the boat lies still.
            for side in [-1.0f32, 1.0] {
                if boat.moving {
                    let stroke = (clock * 2.4).sin();
                    let dip = (clock * 2.4).cos().max(0.0) * 0.25;
                    b.put([0.75 + 0.35 * stroke, 0.45, 1.25 * side], [0.08, 0.06, 2.0], WOOD_LIGHT, 0.45 * stroke * side, [(0.3 + dip) * side, 0.0]);
                } else {
                    b.block([0.1, 0.62, 0.66 * side], [2.2, 0.05, 0.08], WOOD_LIGHT);
                }
            }
        }
        // The cargo between the thwarts, or a fishing boat's nets and baskets.
        let mut b = self.brush(at, boat.heading);
        let h = boat.load;
        if kind == Kind::FishingBoat {
            b.block([-0.35, 0.3, 0.1], [0.8, 0.18, 0.75], NET);
            b.block([0.15, 0.36, -0.35], [0.4, 0.34, 0.4], WICKER);
        } else {
            let n = 3 + h % 3;
            for k in 0..n {
                let x = -0.9 + (k / 2) as f32 * 0.55;
                let z = if k % 2 == 0 { -0.3 } else { 0.3 };
                if (h >> (4 + k)) & 1 == 0 {
                    b.block([x, 0.48, z], [0.42, 0.52, 0.42], shade(WOOD_LIGHT, 0.85));
                    b.block([x, 0.5, z], [0.44, 0.05, 0.44], IRON);
                } else {
                    b.put([x, 0.38, z], [0.5, 0.3, 0.38], shade(SACK, 0.9 + 0.2 * unit(h.rotate_left(k))), 0.4 * k as f32, [0.0; 2]);
                }
            }
        }
    }

    /// The person a figure is, made the first time they are drawn.
    fn person(&mut self, f: &Figure) -> &Person {
        let region = self.region;
        self.people.entry(f.seed).or_insert_with(|| {
            let age = match f.who {
                Who::Child => Age::Child,
                Who::Adult => Age::Adult,
                Who::Elder => Age::Elder,
            };
            let (trade, wealth) = match (f.who, f.work) {
                (Who::Child, _) => (Trade::None, 0.35),
                (_, Work::Settler) => (Trade::Traveller, 0.3),
                (_, Work::Merchant) => (Trade::Merchant, 0.55),
                (_, Work::Camper) => (Trade::Traveller, 0.45),
                (_, Work::Boatman) => (Trade::Merchant, 0.4),
                (_, Work::Fisher) => (Trade::Fisher, 0.3),
            };
            Person::new(f.seed, region, trade, wealth, age)
        })
    }

    /// Draws one of a party at `feet`, the `k`th of them; `by_fire` when they are in camp.
    fn figure(&mut self, f: &Figure, feet: DVec3, k: usize, kind: Kind, by_fire: bool) {
        let (clock, pace, cam) = (self.clock, self.pace, self.cam);
        let phase = unit(f.seed) * 2.0;
        let afloat = matches!(kind, Kind::RiverBoat | Kind::FishingBoat);
        let mut facing = f.facing;
        let mut feet = feet;
        let pose = match f.doing {
            Doing::Walking | Doing::Pulling => {
                // A step is about two thirds of a metre; children take shorter ones.
                let step = if f.who == Who::Child { 0.5 } else { 0.68 };
                gait(Doing::Walking, facing, clock * pace / step + phase, pace)
            }
            Doing::Standing => gait(Doing::Standing, facing, 0.0, 0.0),
            // In a boat, on its thwart, as the player sits in one.
            Doing::Sitting | Doing::Fishing | Doing::Rowing if afloat => {
                feet -= DVec3::new(0.0, m64(0.43), 0.0);
                let mut pose = gait(Doing::Sitting, facing, 0.0, 0.0);
                if f.doing == Doing::Rowing {
                    // TODO(P): a rowing pose of the figures' own, when there is one.
                    let pull = (clock * 2.4).sin();
                    pose.arm = [-1.25 + 0.5 * pull; 2];
                    pose.fore = [-0.35 + 0.25 * pull; 2];
                    pose.lean = 0.25 * pull;
                }
                if f.doing == Doing::Fishing {
                    // Turned to fish over the side, each over their own.
                    facing += if k % 2 == 0 { 1.4 } else { -1.4 };
                    pose.arm = [-1.1, -0.95];
                    pose.fore = [-0.4, -0.5];
                }
                pose
            }
            // About a fire, on a log.
            Doing::Sitting if by_fire => self.seated(feet, facing),
            Doing::Sitting => {
                // TODO(P): a pose of the figures' own for sitting on the ground. Until
                // then: the bench pose, lowered to the ground, legs out before them.
                Pose { leg: [-1.5, -1.4], shin: [0.1, 0.25], arm: [-0.3, -0.2], fore: [-0.6, -0.5], lift: -HIPS * CELL + m(0.12), ..Default::default() }
            }
            Doing::Sketching => {
                // TODO(P): a sketching pose. Until then: sat on a log, a sketchbook held
                // up in one hand and the other at work on it.
                let mut pose = self.seated(feet, facing);
                pose.arm = [-0.9, -1.15];
                pose.fore = [-1.3, -1.1];
                pose.head = -0.15;
                let ahead = DVec3::new(facing.cos() as f64, 0.0, facing.sin() as f64);
                let mut b = self.brush(feet + ahead * m64(0.38), facing);
                b.put([0.0, 0.88, 0.0], [0.04, 0.26, 0.2], PAPER, 0.0, [0.0, -0.5]);
                pose
            }
            Doing::Fishing => {
                // TODO(P): a fishing pose. Until then: stood on the bank, both hands on
                // a rod held out over the water, its line down into it.
                let pose = Pose { arm: [-1.05, -0.85], fore: [-0.45, -0.6], ..Default::default() };
                self.rod(feet, facing);
                pose
            }
            Doing::Asleep => {
                // TODO(P): a sleeping pose. Until then: the figure stood as if asleep
                // and laid down on its back, its feet where it stood (see `lie_down`).
                let pose = Pose { arm: [0.05, -0.05], fore: [-0.15; 2], ..Default::default() };
                let person = self.person(f).clone();
                let from = self.out.len();
                people::draw(&person, feet, facing, &pose, cam, self.out);
                lie_down(&mut self.out[from..], (feet - cam).as_vec3(), facing);
                return;
            }
            // Only boatmen row.
            Doing::Rowing => gait(Doing::Sitting, facing, 0.0, 0.0),
        };
        let person = self.person(f).clone();
        people::draw(&person, feet, facing, &pose, cam, self.out);
    }

    /// Sat on a log about a fire: the bench pose, lowered so the feet are on the ground,
    /// and the log under them.
    fn seated(&mut self, feet: DVec3, facing: f32) -> Pose {
        // TODO(P): a pose of the figures' own for sitting on a log or a stool, when
        // there is one; this is the one the player sits on a bench in.
        let mut pose = gait(Doing::Sitting, facing, 0.0, 0.0);
        pose.lift = -4.0 * CELL;
        let mut b = self.brush(feet, facing);
        b.put([-0.05, 0.2, 0.0], [0.36, 0.4, 0.7], WOOD, 0.0, [0.0; 2]);
        pose
    }

    /// A fishing rod held out before someone standing at `feet` facing `facing`, its line
    /// hanging down from its tip.
    fn rod(&mut self, feet: DVec3, facing: f32) {
        let mut b = self.brush(feet, facing);
        let (long, up) = (2.6f32, 0.5f32);
        let (hx, hy) = (0.45, 0.95);
        let (dx, dy) = (up.cos(), up.sin());
        b.put([hx + dx * long * 0.5, hy + dy * long * 0.5, 0.0], [long, 0.04, 0.04], WOOD_DARK, 0.0, [0.0, up]);
        let (tx, ty) = (hx + dx * long, hy + dy * long);
        b.block([tx, ty * 0.5 - 0.3, 0.0], [0.015, ty + 0.6, 0.015], PAPER);
    }
}

/// How a boat at `at` rides the water at `clock`, as the game's own boats do.
fn bob(at: DVec3, clock: f32) -> f64 {
    ((clock * 1.1 + at.x as f32 * 0.3).sin() * 0.05 * worldgen::scale::OLD_VOXEL) as f64
}

/// A pose from the figures' own gait: walking at `speed` metres a second, `stride` steps
/// along, or standing, or sitting on a bench.
fn gait(doing: Doing, facing: f32, stride: f32, speed: f32) -> Pose {
    let mut g = Gait::new(facing);
    let doing = match doing {
        Doing::Walking | Doing::Pulling => character::Doing::Afoot(speed.max(0.4)),
        Doing::Sitting => character::Doing::Sitting,
        _ => character::Doing::Afoot(0.0),
    };
    // With no time passing, the gait takes up the pose at once.
    g.update(doing, stride, None, false, None, 0.0, 0.0);
    g.pose()
}

/// Lays a figure drawn standing at `feet` (camera-relative) facing `facing` down on its
/// back, its feet where they were: every box turned back a quarter about the line across
/// the feet, and raised so the back is on the ground, as a figure's own boxes are turned
/// about that line when its legs swing.
fn lie_down(boxes: &mut [ParticleInstance], feet: Vec3, facing: f32) {
    let yaw = facing - std::f32::consts::FRAC_PI_2;
    let (across, ahead) = (Vec3::new(yaw.cos(), 0.0, yaw.sin()), Vec3::new(-yaw.sin(), 0.0, yaw.cos()));
    let back = -std::f32::consts::FRAC_PI_2;
    let (sb, cb) = back.sin_cos();
    let lift = m(0.22);
    for b in boxes {
        let v = Vec3::from(b.pos) - feet;
        let (x, y, z) = (v.dot(across), v.y, v.dot(ahead));
        let (y2, z2) = (y * cb - z * sb, y * sb + z * cb);
        b.pos = (feet + across * x + Vec3::Y * (y2 + lift) + ahead * z2).into();
        b.tilt[0] += back;
    }
}

fn shade(c: [f32; 3], by: f32) -> [f32; 3] {
    [(c[0] * by).min(1.0), (c[1] * by).min(1.0), (c[2] * by).min(1.0)]
}

/// One traveller or quay as the console lists them.
pub struct Listed {
    pub what: String,
    pub doing: String,
    pub at: DVec3,
    /// Somewhere on dry land a little way off to stand and see them from, and which way
    /// to look from there.
    pub stand: DVec2,
    pub look: f32,
}

/// The travellers (or with `quays`, the quays) nearest `at` on its continent at `when`,
/// of `kind` or any, up to `n`, nearest first: for the console. Waits for the continent.
pub fn listing(world: &GameWorld, at: DVec3, when: When, calm: bool, kind: Option<Kind>, quays: bool, n: usize) -> Vec<Listed> {
    let cell = cell_of(at.x.floor() as i32, at.z.floor() as i32);
    let c = world.continent(cell.0, cell.1);
    let origin = DVec3::new((cell.0 * MACRO_SIZE) as f64, 0.0, (cell.1 * MACRO_SIZE) as f64);
    let names = &c.trails.names;
    let town = |k: usize| names.settlement(k).map_or_else(|| format!("settlement {k}"), |p| p.name.clone());
    let mut out = Vec::new();
    if quays {
        let ways = &c.trails.waterways;
        for (k, q) in ways.quays.iter().enumerate() {
            let river = q.serves == worldgen::settlements::quay::Serves::River;
            let what = format!("the {} quay of {}{}", if river { "river" } else { "fishing" }, town(q.settlement), if q.landing.is_some() { " (its landing)" } else { "" });
            let boats = ways.grounds_of(k).count() + ways.runs.iter().filter(|r| r.from == k || r.to == k).count();
            let foot = origin + DVec3::new(q.foot.x as f64, q.level as f64, q.foot.y as f64);
            let (stand, look) = quay_stand(q, origin);
            out.push(Listed { what, doing: format!("{boats} boats work from here"), at: foot, stand, look });
        }
    } else {
        let t = Travel::of(&c, world.seed());
        let here = local(at, cell);
        for plan in t.near(&c, when.day(), here, f32::INFINITY) {
            if kind.is_some_and(|k| k != plan.kind()) {
                continue;
            }
            let Some(s) = plan.at(when.hour, when.date, calm) else { continue };
            let route = match (s.from, s.to) {
                (Some(a), Some(b)) => format!(", from {} to {}", town(a), town(b)),
                (Some(a), None) => format!(", of {}", town(a)),
                _ => String::new(),
            };
            let many = plan.party().unwrap_or(s.people.len());
            let what = format!("{}{}{route}", s.kind.word(), if many > 2 { format!(", {many} of them") } else { String::new() });
            let p = origin + s.at.as_dvec3();
            let flat = |v: Vec3| Vec2::new(v.x, v.z);
            // Where to stand to see them: on the trail by a camp, looking across it at the
            // fire with the tents beyond; ahead of walkers on the road, as they come on;
            // abreast of a boat under way, on the bank if it is near; at the foot of the
            // nearest quay for a boat tied up.
            let beyond = s.tents.first().or(s.bedrolls.first()).map(|t| flat(t.0));
            let stand = match (s.kind, s.fire, beyond) {
                (Kind::RiverBoat | Kind::FishingBoat, _, _) => match s.boats.iter().find(|b| b.moving) {
                    Some(b) => (flat(b.at) + Vec2::from_angle(b.heading + 1.57) * m(12.0)).as_dvec2() + DVec2::new(origin.x, origin.z),
                    None => {
                        let quays = &c.trails.waterways.quays;
                        let boat = flat(s.at);
                        let q = quays.iter().min_by(|a, b| a.head.distance(boat).total_cmp(&b.head.distance(boat))).expect("a boat has a quay");
                        quay_stand(q, origin).0
                    }
                },
                (_, Some((fire, _)), Some(tent)) => {
                    let out = (tent - flat(fire)).normalize_or(Vec2::X);
                    (flat(fire) - out * m(8.0)).as_dvec2() + DVec2::new(origin.x, origin.z)
                }
                _ => {
                    let lead = s.people.first().map_or(0.0, |f| f.facing);
                    (flat(s.at) + Vec2::from_angle(lead) * m(11.0) + Vec2::from_angle(lead + 1.57) * m(2.5)).as_dvec2() + DVec2::new(origin.x, origin.z)
                }
            };
            let to = DVec2::new(p.x, p.z) - stand;
            out.push(Listed { what, doing: s.doing.to_string(), at: p, stand, look: to.y.atan2(to.x) as f32 });
        }
    }
    out.sort_by(|a, b| a.at.distance(at).total_cmp(&b.at.distance(at)));
    out.truncate(n);
    out
}

/// Where to stand to see quay `q` and the boats tied up at it, and the yaw to look along:
/// back on the bank from the jetty's foot and a little aside, looking out along it. A
/// landing's foot can be where its boats lie, so the foot itself will not do.
fn quay_stand(q: &worldgen::settlements::quay::Quay, origin: DVec3) -> (DVec2, f32) {
    let foot = DVec2::new(origin.x + q.foot.x as f64, origin.z + q.foot.y as f64);
    let head = DVec2::new(origin.x + q.head.x as f64, origin.z + q.head.y as f64);
    let back = (foot - head).try_normalize().unwrap_or(-DVec2::from_angle(q.heading as f64));
    let stand = foot + back * m64(9.0) + back.perp() * m64(4.0);
    let to = head - stand;
    (stand, to.y.atan2(to.x) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_settlers_met_are_saved_and_read_back() {
        let mut t = Travellers::new();
        t.met = vec![Met { cell: (0, 0), id: (2u64 << 60) | 5 }, Met { cell: (-1, 3), id: 7 }];
        assert_eq!(Travellers::from_text(&t.to_text()).met, t.met);
        // What cannot be read is passed over, not guessed at.
        assert!(Travellers::from_text("met x 0 1\nnonsense\nmet 0 0\n").met.is_empty());
    }

    #[test]
    fn boats_keep_in_when_it_blows_or_pours() {
        let sky = |storm: f32, rain: f32| crate::weather::Sky { storm, rain, ..crate::weather::Weather::new(None).now };
        assert!(calm(&sky(0.0, 0.0)));
        assert!(!calm(&sky(0.6, 0.0)));
        assert!(!calm(&sky(0.0, 0.9)));
    }
}
