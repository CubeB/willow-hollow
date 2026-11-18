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
//! The people are drawn as the villagers are, near or far as they are from the camera, at
//! the tasks the figures have poses for (#204): sat on a log, a stool or the ground about
//! the fire, fishing, sketching, asleep on a bedroll, at the oars, pulling a handcart,
//! carrying goods or a boat round a portage. The oxen and donkeys that draw some settlers'
//! carts are drawn as the herds' beasts are (#206), walking in the shafts.

use std::{collections::HashMap, sync::Arc, thread::JoinHandle};

use glam::{DVec2, DVec3, Vec2, Vec3};
use worldgen::{
    continent::Continent,
    macro_layout::{MACRO_SIZE, cell_of},
    noise::{hash2, unit},
    scale::{m, m64},
    settlements::{
        Region,
        travel::{self, Beast, Boat, Cart, Doing, Figure, Fire, Kind, Plan, Scene, Travel, Water, Who, Work},
    },
    world::GameWorld,
};

use crate::{
    animals::model,
    character::{self, Carried, Gait, Look, Pose, Task},
    inventory,
    particles::{self, ParticleInstance},
    people::{Age, Person, Trade},
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
    /// The day the water turned rough and the hour it did, which holds the boats in to the
    /// end of that day; None while it has been calm all day.
    rough: Option<(i64, f32)>,
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
            rough: None,
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
        let water = self.water(calm);
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
        self.place(camera, water);
    }

    /// How the water has been for the boats today, `calm` as it is now: once it turns it
    /// stays rough for them to the day's end, and a day that dawns rough has been rough
    /// since before anyone was up.
    fn water(&mut self, calm: bool) -> Water {
        let day = self.when.day();
        match self.rough {
            Some((d, since)) if d == day => Water::rough_since(since),
            _ if calm => {
                self.rough = None;
                Water::CALM
            }
            Some((d, _)) if d == day - 1 => {
                self.rough = Some((day, 0.0));
                Water::ROUGH
            }
            _ => {
                self.rough = Some((day, self.when.hour));
                Water::rough_since(self.when.hour)
            }
        }
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
        // Rough, for a screenshot, since before the day began.
        self.rough = (!calm).then_some((want.day, 0.0));
        let water = if calm { Water::CALM } else { Water::ROUGH };
        self.place(camera, water);
    }

    /// Where everyone planned for is this moment, the nearest first, and who is met.
    fn place(&mut self, camera: DVec3, water: Water) {
        self.seen.clear();
        let Some(planned) = self.planned else { return };
        // Plans for another day are no use: they would put everyone where they were then.
        if planned.day != self.when.day() {
            return;
        }
        let origin = DVec3::new((planned.cell.0 * MACRO_SIZE) as f64, 0.0, (planned.cell.1 * MACRO_SIZE) as f64);
        for plan in &self.plans {
            let Some(scene) = plan.at(self.when.hour, self.when.date, water) else { continue };
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
const LEATHER: [f32; 3] = [0.16, 0.09, 0.05];
const ROPE: [f32; 3] = [0.45, 0.38, 0.24];
/// Iron for the smithies (#205): bar iron, blue-black from the hammer; a bloom from the
/// furnace, a rough grey lump; and the rust that comes up on either left out in the wet.
const BAR: [f32; 3] = [0.065, 0.066, 0.075];
const BLOOM: [f32; 3] = [0.105, 0.095, 0.088];
const RUSTED: [f32; 3] = [0.21, 0.085, 0.04];

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
            let beast = cart.beast.and_then(|b| scene.beasts.get(b)).map(|b| (beast_look(b), flat(cart.at).distance(flat(b.feet)) / m(1.0)));
            self.cart(s.world(cart.at), cart, pulled, beast);
        }
        for (k, b) in scene.beasts.iter().enumerate() {
            if self.full() {
                return;
            }
            self.beast(s.world(b.feet), b, scene.carts.iter().any(|c| c.beast == Some(k)));
        }
        for (k, &(at, heading)) in scene.goods.iter().enumerate() {
            if self.full() {
                return;
            }
            self.goods(s.world(at), heading, hash2(scene.id as u32, k as i32, 0x900d));
        }
        for (k, &(at, heading)) in scene.iron.iter().enumerate() {
            if self.full() {
                return;
            }
            self.iron_stack(s.world(at), heading, hash2(scene.id as u32, k as i32, 0x1e0b));
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
            // Sat in the boat, riding the water with it; everyone else is ashore.
            let boat = scene.boats.first().filter(|b| !b.ashore && flat(b.at).distance(flat(f.feet)) < m(2.4) && (b.at.y - f.feet.y).abs() < 0.5);
            let bob = boat.map_or(0.0, |b| bob(s.world(b.at), self.clock));
            self.figure(f, s.world(f.feet) + DVec3::new(0.0, bob, 0.0), k, scene.kind, scene.fire.is_some(), boat.is_some());
        }
    }

    /// A beast at `feet`: walking in the shafts of a cart (`in_shafts`, with its collar on)
    /// or tethered, grazing or standing.
    fn beast(&mut self, feet: DVec3, b: &Beast, in_shafts: bool) {
        let l = beast_look(b);
        let detail = model::detail(&l, feet.distance(self.cam));
        if detail == model::Detail::Gone {
            return;
        }
        let phase = unit(b.seed) * std::f32::consts::TAU;
        // A walk, its stride going round with the ground the party covers.
        let stride = ((l.leg + l.depth * 0.3) * 2.4).max(0.15);
        let swish = (self.clock * 0.7 + phase).sin();
        let pose = model::Pose {
            stride: if b.walking { (self.clock * self.pace / stride * std::f32::consts::TAU + phase).rem_euclid(std::f32::consts::TAU) } else { 0.0 },
            gait: model::Gait::Walk,
            reach: if b.walking { 1.0 } else { 0.0 },
            graze: if b.grazing { 0.8 + 0.2 * (self.clock * 0.5 + phase).sin() } else { 0.0 },
            ears: (self.clock * 1.3 + phase).sin(),
            tail: if swish > 0.6 { swish } else { 0.0 },
            ..Default::default()
        };
        model::draw_at(&l, detail, feet, b.facing, &pose, self.cam, self.out);
        if in_shafts && detail == model::Detail::Near {
            // A collar about the neck at the withers, for the shafts to bear on.
            let mut br = self.brush(feet, b.facing);
            let (len, w, d, leg) = (l.length, l.width, l.depth, l.leg);
            br.block([len * 0.4, leg + d * 0.72, 0.0], [0.12, d * 0.62, w + 0.06], LEATHER);
            br.block([len * 0.1, leg + d + 0.03, 0.0], [0.3, 0.06, w + 0.04], LEATHER);
        }
    }

    /// A boat's goods carried round a portage and set down on the bank: sacks, a cask and
    /// a crate, in a heap along `heading`.
    fn goods(&mut self, at: DVec3, heading: f32, salt: u32) {
        let mut b = self.brush(at, heading);
        b.put([-0.35, 0.17, 0.0], [0.5, 0.34, 0.4], shade(SACK, 0.9 + 0.2 * unit(salt)), 0.2, [0.0; 2]);
        b.put([0.2, 0.17, 0.12], [0.5, 0.34, 0.4], shade(SACK, 0.85 + 0.2 * unit(salt.rotate_left(7))), -0.3, [0.0; 2]);
        b.block([0.05, 0.48, 0.05], [0.4, 0.3, 0.38], shade(SACK, 1.05));
        b.block([0.55, 0.25, -0.35], [0.42, 0.5, 0.42], shade(WOOD_LIGHT, 0.85));
        b.block([0.55, 0.27, -0.35], [0.44, 0.05, 0.44], IRON);
        b.block([-0.45, 0.2, -0.45], [0.45, 0.4, 0.38], shade(WOOD, 1.3));
    }

    /// Iron landed on a quay for the smithy (#205), lying along `heading`: two bundles of
    /// bar iron tied with rope on timbers to keep them off the wet boards, the upper laid a
    /// little askew, and a bloom or two at the end of them.
    fn iron_stack(&mut self, at: DVec3, heading: f32, salt: u32) {
        let mut b = self.brush(at, heading);
        for x in [-0.32f32, 0.32] {
            b.block([x, 0.03, 0.0], [0.08, 0.06, 0.5], WOOD_DARK);
        }
        b.block([0.0, 0.12, 0.0], [0.95, 0.12, 0.42], BAR);
        b.put([0.02, 0.23, 0.01], [0.88, 0.1, 0.32], shade(BAR, 1.15), 0.05 * (unit(salt) - 0.5), [0.0; 2]);
        for x in [-0.28f32, 0.26] {
            b.block([x, 0.17, 0.0], [0.05, 0.2, 0.44], ROPE);
        }
        for k in 0..1 + salt % 2 {
            let z = if k == 0 { 0.1 } else { -0.13 };
            let r = unit(salt.rotate_left(5 + k));
            let rough = if r < 0.3 { RUSTED } else { BLOOM };
            b.put([0.68, 0.08, z], [0.22, 0.16, 0.2], rough, r * 1.4, [0.12 * (r - 0.5), 0.1]);
            b.put([0.7, 0.18, z + 0.02], [0.12, 0.07, 0.11], shade(BLOOM, 0.9), r * 2.3, [0.0; 2]);
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
    /// let down to the ground; and what it carries. With a beast in its shafts (its look,
    /// and how far ahead of the cart's middle it is, in metres), longer shafts, set wider,
    /// reach forward along the beast's flanks to its collar.
    fn cart(&mut self, at: DVec3, cart: &Cart, pulled: bool, beast: Option<(model::Look, f32)>) {
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
        match beast {
            Some((l, ahead)) => {
                // From the cart's front to the beast's collar, up its flanks.
                let (x0, x1) = (0.72f32, ahead + l.length * 0.4);
                let (y0, y1) = (0.72f32, l.leg + l.depth * 0.6);
                let long = x1 - x0;
                let rise = (y1 - y0).atan2(long);
                for side in [-1.0f32, 1.0] {
                    let z = (l.width * 0.5 + 0.07).max(0.32) * side;
                    b.put([(x0 + x1) / 2.0, (y0 + y1) / 2.0, z], [(long * long + (y1 - y0).powi(2)).sqrt(), 0.07, 0.07], WOOD, 0.0, [0.0, rise]);
                }
            }
            None => {
                let (rise, mid) = if pulled { (0.17f32, 0.74) } else { (-0.43f32, 0.36) };
                for side in [-1.0f32, 1.0] {
                    b.put([1.33, mid, 0.32 * side], [1.25, 0.07, 0.07], WOOD, 0.0, [0.0, rise]);
                }
            }
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

    /// A rowing boat's hull, the game's own without its mast, sitting on `at`; or turned
    /// over, its gunwales `at` and its keel up, as it is carried overhead.
    fn hull(&mut self, at: DVec3, heading: f32, over: bool) {
        let mut b = self.brush(at, heading);
        const O: f32 = 0.8;
        // Turned over about the top of the gunwales.
        let top = 0.75 * O;
        let y = |y: f32| if over { top - y } else { y };
        b.block([0.0, y(0.05 * O), 0.0], [4.6 * O, 0.3 * O, 1.5 * O], HULL);
        b.block([0.0, y(0.45 * O), 0.85 * O], [4.6 * O, 0.6 * O, 0.2 * O], WOOD_LIGHT);
        b.block([0.0, y(0.45 * O), -0.85 * O], [4.6 * O, 0.6 * O, 0.2 * O], WOOD_LIGHT);
        b.block([-2.3 * O, y(0.45 * O), 0.0], [0.2 * O, 0.6 * O, 1.9 * O], WOOD_LIGHT);
        b.block([2.65 * O, y(0.3 * O), 0.0], [0.9 * O, 0.5 * O, 1.1 * O], HULL);
        b.block([3.25 * O, y(0.5 * O), 0.0], [0.5 * O, 0.5 * O, 0.6 * O], WOOD_LIGHT);
        b.block([-1.0 * O, y(0.4 * O), 0.0], [0.5 * O, 0.12 * O, 1.5 * O], WOOD_LIGHT);
        b.block([1.1 * O, y(0.4 * O), 0.0], [0.5 * O, 0.12 * O, 1.5 * O], WOOD_LIGHT);
    }

    /// A boat: a sailing boat as the game's own, or a rowing boat, its oars working while
    /// it is under way; with its cargo, or a fishing boat's nets. Ashore on a portage, a
    /// rowing boat is held up over its crew's heads, turned over, and a sailing boat is
    /// hauled along on rollers, its mast down along it.
    fn boat(&mut self, at: DVec3, boat: &Boat, kind: Kind) {
        let clock = self.clock;
        if boat.ashore {
            if boat.sail {
                // Two rollers under it, turning as it goes, the hull on them and the mast
                // laid along the thwarts.
                let mut b = self.brush(at, boat.heading);
                for x in [-1.1f32, 1.0] {
                    b.put([x, 0.11, 0.0], [0.22, 0.22, 1.7], WOOD_LIGHT, 0.0, [0.0, clock * 1.5 * boat.moving as u8 as f32]);
                }
                self.hull(at + DVec3::new(0.0, m64(0.2), 0.0), boat.heading, false);
                let mut b = self.brush(at, boat.heading);
                b.block([0.3, 0.2 + 0.7, 0.0], [4.6, 0.12, 0.12], WOOD);
                b.block([0.3, 0.2 + 0.78, 0.0], [3.2, 0.1, 0.3], shade(CANVAS[0], 1.1));
            } else {
                self.hull(at + DVec3::new(0.0, m64(1.95), 0.0), boat.heading, true);
            }
            return;
        }
        let at = at + DVec3::new(0.0, bob(at, clock), 0.0);
        if boat.sail {
            crate::vehicles::draw_boat(at, boat.heading, clock, self.cam, self.out);
        } else {
            self.hull(at, boat.heading, false);
            let mut b = self.brush(at, boat.heading);
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
        if !boat.laden {
            return;
        }
        if kind == Kind::FishingBoat {
            b.block([-0.35, 0.3, 0.1], [0.8, 0.18, 0.75], NET);
            b.block([0.15, 0.36, -0.35], [0.4, 0.34, 0.4], WICKER);
        } else if boat.iron {
            // Iron for a smithy (#205), low between the thwarts as heavy goods are stowed:
            // two bundles of bar iron, tied, and a pair of blooms before them.
            for z in [-0.19f32, 0.19] {
                b.block([-0.15, 0.29, z], [0.8, 0.14, 0.22], BAR);
                b.block([-0.15, 0.4, z], [0.76, 0.09, 0.15], shade(BAR, 1.2));
                for x in [-0.45f32, 0.15] {
                    b.block([x, 0.33, z], [0.05, 0.2, 0.24], ROPE);
                }
            }
            for (k, z) in [(0u32, -0.17f32), (1, 0.17)] {
                let r = unit(h.rotate_left(3 + k));
                b.put([0.45, 0.3, z], [0.22, 0.16, 0.2], if r < 0.3 { RUSTED } else { BLOOM }, r * 1.4, [0.1 * (r - 0.5), 0.0]);
                b.put([0.47, 0.41, z], [0.12, 0.07, 0.11], shade(BLOOM, 0.9), r * 2.3, [0.0; 2]);
            }
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

    /// Draws one of a party at `feet`, the `k`th of them; `by_fire` when they are in camp,
    /// `afloat` when they are in a boat.
    fn figure(&mut self, f: &Figure, feet: DVec3, k: usize, kind: Kind, by_fire: bool, afloat: bool) {
        let (clock, pace) = (self.clock, self.pace);
        let phase = unit(f.seed) * 2.0;
        let look = self.person(f).look;
        let build = look.build;
        // Each at their own point of whatever they are at.
        let offset = unit(f.seed.rotate_left(5));
        let mut facing = f.facing;
        let mut feet = feet;
        // A step is about two thirds of a metre; children take shorter ones.
        let step = if f.who == Who::Child { 0.5 } else { 0.68 };
        let face = facing;
        let walking = move || gait(Doing::Walking, face, clock * pace / step + phase, pace);
        let (pose, task, look) = match f.doing {
            Doing::Walking => (walking(), None, look),
            Doing::Pulling => {
                // Between the shafts, a hand on each at the hip, leaning into the pull.
                let mut pose = walking();
                (pose.arm, pose.fore) = ([0.28; 2], [-0.4; 2]);
                pose.lean += 0.14;
                (pose, None, Look { carried: Carried::None, ..look })
            }
            Doing::Leading => {
                // At the beast's head, the near hand out on its halter.
                let mut pose = walking();
                (pose.arm[0], pose.fore[0]) = (-0.55, -0.45);
                (pose, None, look)
            }
            Doing::Laden => {
                // A sack or a cask held against the chest in both arms.
                let mut pose = walking();
                (pose.arm, pose.fore) = ([-0.35; 2], [-1.35; 2]);
                pose.lean -= 0.06;
                let mut b = self.brush(feet, facing);
                b.put([0.3, 1.08, 0.0], [0.34, 0.42, 0.44], shade(SACK, 0.9 + 0.2 * unit(f.seed)), 0.0, [0.0, 0.15]);
                (pose, None, Look { carried: Carried::None, ..look })
            }
            Doing::Portaging => {
                // Under the boat turned over on their shoulders, both hands up on its
                // gunwales.
                let mut pose = walking();
                (pose.arm, pose.fore) = ([-2.75; 2], [-0.25; 2]);
                (pose, None, Look { carried: Carried::None, ..look })
            }
            Doing::Hauling => {
                // The one ahead leaning into a rope over the shoulder, the one behind
                // shoving at the stern.
                let mut pose = walking();
                if k % 2 == 0 {
                    (pose.arm, pose.fore) = ([-0.3, 0.35], [-1.6, -0.3]);
                    pose.lean += 0.3;
                    let mut b = self.brush(feet, facing);
                    b.put([-1.0, 1.0, 0.18], [2.1, 0.04, 0.04], ROPE, 0.0, [0.0, 0.38]);
                } else {
                    (pose.arm, pose.fore) = ([-1.25; 2], [-0.2; 2]);
                    pose.lean += 0.4;
                }
                (pose, None, Look { carried: Carried::None, ..look })
            }
            Doing::Standing => {
                // Those standing about a camp talk among themselves.
                let task = if by_fire && k % 2 == 1 { Task::Talk } else { Task::Stand };
                (character::task_pose(task, &build, clock, offset), Some(task), look)
            }
            // In a boat, on its thwart, as the player sits in one.
            Doing::Sitting | Doing::Fishing | Doing::Rowing if afloat => {
                feet -= DVec3::new(0.0, m64(BOAT_SEAT), 0.0);
                match f.doing {
                    // In time with the oars, which are at the start of their swing at 0.
                    Doing::Rowing => (character::task_pose(Task::Row, &build, clock, 0.0), Some(Task::Row), look),
                    Doing::Fishing => {
                        // Sat turned to fish over the side, each over their own, the rod
                        // held out over the water.
                        facing += if k % 2 == 0 { 1.4 } else { -1.4 };
                        let mut pose = gait(Doing::Sitting, facing, 0.0, 0.0);
                        let rod = character::task_pose(Task::Fish, &build, clock, offset);
                        (pose.arm, pose.fore, pose.jiggle) = (rod.arm, rod.fore, rod.jiggle);
                        (pose, Some(Task::Fish), Look { carried: Task::Fish.carried(look.carried), ..look })
                    }
                    _ => (gait(Doing::Sitting, facing, 0.0, 0.0), Some(Task::Sit), Look { carried: Task::Sit.carried(look.carried), ..look }),
                }
            }
            // About a fire: some on a log or a stool, some on the ground.
            Doing::Sitting if by_fire && !(kind == Kind::Settlers && k % 2 == 1) => {
                self.seat(feet, facing, f.seed);
                // A stool is lower than a bench: the knees up a little, the shins
                // straight down, so the tallest keep their feet on the ground.
                let mut pose = character::task_pose(Task::Sit, &build, clock, offset);
                (pose.leg, pose.shin) = ([-1.72; 2], [1.72; 2]);
                (pose, Some(Task::Sit), Look { carried: Task::Sit.carried(look.carried), ..look })
            }
            Doing::Sitting => {
                // On a blanket folded on the ground, so a long coat or skirt lies on it
                // rather than in the ground; one leg out along it and the other drawn up,
                // rather than both reaching down into it as the figures' own pose has
                // them, sat up; a bow and quiver laid down, not leant back on.
                let mut b = self.brush(feet, facing);
                b.block([-0.05, BLANKET as f32 / 2.0, 0.0], [0.62, BLANKET as f32, 0.58], WOOL[(f.seed % 3) as usize]);
                feet += DVec3::new(0.0, m64(BLANKET), 0.0);
                let mut pose = character::task_pose(Task::SitLow, &build, clock, offset);
                (pose.leg, pose.shin, pose.lean) = ([-1.62, -1.8], [0.0, 0.35], 0.04);
                let carried = match Task::SitLow.carried(look.carried) {
                    Carried::Bow => Carried::None,
                    c => c,
                };
                (pose, Some(Task::SitLow), Look { carried, ..look })
            }
            Doing::Sketching => {
                self.seat(feet, facing, f.seed);
                (character::task_pose(Task::Sketch, &build, clock, offset), Some(Task::Sketch), Look { carried: Task::Sketch.carried(look.carried), ..look })
            }
            // Stood on the bank, the rod held out over the water.
            Doing::Fishing => (character::task_pose(Task::Fish, &build, clock, offset), Some(Task::Fish), Look { carried: Task::Fish.carried(look.carried), ..look }),
            // On their back on the bedroll, feet toward its foot: their back on its top and
            // their head down on the rolled end, as the figures lie a cell up off a bed.
            Doing::Asleep => {
                feet += DVec3::new(0.0, m64(SLEEPER), 0.0);
                (character::task_pose(Task::Lie, &build, clock, offset), Some(Task::Lie), Look { carried: Task::Lie.carried(look.carried), ..look })
            }
            // Only boatmen row.
            Doing::Rowing => (character::task_pose(Task::Stand, &build, clock, offset), Some(Task::Stand), look),
        };
        self.person_at(&look, feet, facing, &pose, task);
    }

    /// Draws someone as near or as far as they are: whole within `character::NEAR`, in a
    /// few boxes out to `character::SIGHT` (but for someone lying down, who is too low to
    /// make out from afar), dithering out over the last stretch.
    fn person_at(&mut self, look: &Look, feet: DVec3, facing: f32, pose: &Pose, task: Option<Task>) {
        let d = feet.distance(self.cam);
        let sight = m64(character::SIGHT as f64);
        if d >= sight {
            return;
        }
        let from = self.out.len();
        if d < m64(character::NEAR as f64) {
            match task {
                Some(t) => character::draw_at(look, feet, facing, pose, t, self.cam, self.out),
                None => character::draw(look, feet, facing, pose, None, self.cam, self.out),
            }
        } else if task != Some(Task::Lie) {
            character::draw_far(look, feet, facing, pose, self.cam, self.out);
        }
        particles::dither_since(self.out, from, particles::dither_edge(d, sight, m64(20.0)));
    }

    /// What someone sits on by a fire, under them at `feet`: the stool some carry, or a
    /// round of log stood on end, either a seat high (`character::SEAT`).
    fn seat(&mut self, feet: DVec3, facing: f32, seed: u32) {
        if seed & 0x100 == 0 {
            character::draw_stool(feet, facing, self.cam, self.out);
        } else {
            let mut b = self.brush(feet, facing);
            let high = character::SEAT;
            b.block([0.0, (high - 0.03) / 2.0, 0.0], [0.42, high - 0.03, 0.42], WOOD);
            b.block([0.0, high - 0.015, 0.0], [0.38, 0.03, 0.38], WOOD_LIGHT);
        }
    }
}

/// How far below a boat's waterline a figure sat in it has its feet: as the player sits
/// in one, on a thwart.
const BOAT_SEAT: f64 = 0.43;
/// How far over the ground a sleeper's feet are set, so their back lies on the bedroll's
/// top and their head rests on its rolled end, metres.
const SLEEPER: f64 = 0.21;
/// How thick the blanket is, folded over, that someone sits on on the ground, metres.
const BLANKET: f64 = 0.12;

/// Who a beast is, from its seed: an ox, or a jenny of a donkey, as the herds' are made.
fn beast_look(b: &Beast) -> model::Look {
    let role = if b.kind == worldgen::fauna::Kind::Ox { model::Role::Male } else { model::Role::Female };
    model::Look::new(b.kind, b.seed, role)
}

fn flat(p: Vec3) -> Vec2 {
    Vec2::new(p.x, p.z)
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
        // The portages, by the middle of each path, seen from the stage above.
        for run in &ways.runs {
            for p in &run.portages {
                let what = format!("the portage on the river from {} to {}", town(run.a), town(run.b));
                let (mid, _) = p.at(p.length() * 0.5);
                let at = origin + mid.as_dvec3();
                let first = p.path[0];
                let back = (flat(first) - flat(mid)).normalize_or(Vec2::X);
                let stand = (flat(first) + back * m(4.0)).as_dvec2() + DVec2::new(origin.x, origin.z);
                let to = DVec2::new(at.x, at.z) - stand;
                let doing = format!("a path of {:.0} m round the rough water, between two landing stages", p.length() / m(1.0));
                out.push(Listed { what, doing, at, stand, look: to.y.atan2(to.x) as f32 });
            }
        }
    } else {
        let t = Travel::of(&c, world.seed());
        let here = local(at, cell);
        for plan in t.near(&c, when.day(), here, f32::INFINITY) {
            if kind.is_some_and(|k| k != plan.kind()) {
                continue;
            }
            let Some(s) = plan.at(when.hour, when.date, if calm { Water::CALM } else { Water::ROUGH }) else { continue };
            let route = match (s.from, s.to) {
                (Some(a), Some(b)) => format!(", from {} to {}", town(a), town(b)),
                (Some(a), None) => format!(", of {}", town(a)),
                _ => String::new(),
            };
            let many = plan.party().unwrap_or(s.people.len());
            // Settlers with beasts in their carts' shafts say so, to be found by them (#206).
            let drawn = s.beasts.first().map_or(String::new(), |b| format!(", with {}", b.kind.word()));
            let what = format!("{}{}{drawn}{route}", s.kind.word(), if many > 2 { format!(", {many} of them") } else { String::new() });
            let p = origin + s.at.as_dvec3();
            // Where to stand to see them: on the trail by a camp, looking across it at the
            // fire with the tents beyond; ahead of walkers on the road, as they come on;
            // abreast of a boat under way, on the bank if it is near; at the foot of the
            // nearest quay for a boat tied up.
            let beyond = s.tents.first().or(s.bedrolls.first()).map(|t| flat(t.0));
            let stand = match (s.kind, s.fire, beyond) {
                (Kind::RiverBoat | Kind::FishingBoat, _, _) => match s.boats.iter().find(|b| b.moving && !b.ashore) {
                    Some(b) => (flat(b.at) + Vec2::from_angle(b.heading + 1.57) * m(12.0)).as_dvec2() + DVec2::new(origin.x, origin.z),
                    // Ashore with the crew, a little further from the water than they are.
                    None if s.people.iter().any(|f| flat(f.feet).distance(flat(s.at)) > m(2.5)) || s.boats.iter().any(|b| b.ashore) => {
                        let f = s.people.iter().max_by(|a, b| flat(a.feet).distance(flat(s.at)).total_cmp(&flat(b.feet).distance(flat(s.at)))).expect("a crew");
                        let away = (flat(f.feet) - flat(s.at)).normalize_or(Vec2::X);
                        (flat(f.feet) + away * m(6.0) + away.perp() * m(2.0)).as_dvec2() + DVec2::new(origin.x, origin.z)
                    }
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

    #[test]
    fn once_the_water_turns_it_stays_rough_to_the_end_of_the_day() {
        let mut t = Travellers::new();
        let at = |t: &mut Travellers, day: f32, hour: f32, calm: bool| {
            t.when = When { hour, date: (day + hour / 24.0) / 24.0, year: 0, year_days: 24.0, day_minutes: 40.0 };
            t.water(calm)
        };
        assert_eq!(at(&mut t, 2.0, 9.0, true), Water::CALM);
        assert_eq!(at(&mut t, 2.0, 11.0, false), Water::rough_since(11.0));
        // Calm again in the afternoon, but the boats stay in.
        assert_eq!(at(&mut t, 2.0, 15.0, true), Water::rough_since(11.0));
        // Still rough the next morning: rough since before they were up.
        assert_eq!(at(&mut t, 3.0, 6.0, false), Water::ROUGH);
        assert_eq!(at(&mut t, 4.0, 8.0, true), Water::CALM);
    }

    /// The lowest point of every box drawn, turned as the shader turns it.
    fn lowest(out: &[ParticleInstance]) -> f32 {
        let turn = |v: Vec3, yaw: f32, tilt: [f32; 2]| {
            let (sr, cr) = tilt[1].sin_cos();
            let a = Vec3::new(v.x * cr - v.y * sr, v.x * sr + v.y * cr, v.z);
            let (sp, cp) = tilt[0].sin_cos();
            let b = Vec3::new(a.x, a.y * cp - a.z * sp, a.y * sp + a.z * cp);
            let (s, c) = yaw.sin_cos();
            Vec3::new(b.x * c - b.z * s, b.y, b.x * s + b.z * c)
        };
        let mut low = f32::INFINITY;
        for p in out {
            let half = Vec3::from(p.size) * 0.5;
            for k in 0..8 {
                let corner = Vec3::new(if k & 1 == 0 { -half.x } else { half.x }, if k & 2 == 0 { -half.y } else { half.y }, if k & 4 == 0 { -half.z } else { half.z });
                low = low.min(p.pos[1] + turn(corner, p.yaw, p.tilt).y);
            }
        }
        low
    }

    fn figure(seed: u32, doing: Doing) -> Figure {
        Figure { seed, who: Who::Adult, work: Work::Settler, feet: Vec3::ZERO, facing: 0.4, doing }
    }

    #[test]
    fn every_traveller_keeps_to_its_boxes_and_out_of_the_ground() {
        let cam = DVec3::new(30.0, 12.0, -20.0);
        let mut people = HashMap::new();
        let all = [
            Doing::Walking,
            Doing::Pulling,
            Doing::Standing,
            Doing::Sitting,
            Doing::Asleep,
            Doing::Fishing,
            Doing::Sketching,
            Doing::Rowing,
            Doing::Leading,
            Doing::Laden,
            Doing::Portaging,
            Doing::Hauling,
        ];
        let mut most = 0;
        for seed in 0..40u32 {
            for doing in all {
                for (kind, by_fire, afloat) in [(Kind::Settlers, true, false), (Kind::Camper, false, false), (Kind::RiverBoat, false, true), (Kind::RiverBoat, true, false)] {
                    if afloat && !matches!(doing, Doing::Sitting | Doing::Fishing | Doing::Rowing) {
                        continue;
                    }
                    for clock in [0.0, 0.7, 1.9, 3.3] {
                        let mut out = Vec::new();
                        let mut d = Drawing { cam, clock, pace: 1.3, out: &mut out, start: 0, people: &mut people, region: Region::Temperate };
                        let f = figure(seed.wrapping_mul(0x9e37_79b9), doing);
                        d.figure(&f, DVec3::ZERO, seed as usize, kind, by_fire, afloat);
                        most = most.max(out.len());
                        // Into a boat no deeper than its thwart; on land, nowhere into the
                        // ground, but for a toe's last sliver as the foot rolls in a stride.
                        let floor = if afloat { -(BOAT_SEAT * m64(1.0)) as f32 } else { 0.0 } - m(0.04);
                        // The boxes are drawn about the camera. A fishing line hangs down
                        // into the water, below the bank, as it should.
                        let solid: Vec<ParticleInstance> = out.iter().filter(|b| b.size[0].max(b.size[2]) > 0.05).copied().collect();
                        let low = lowest(&solid) + cam.y as f32;
                        assert!(low >= floor, "{doing:?} (seed {seed}, at {clock}) reaches {} m into the ground", (floor - low) / m(1.0));
                    }
                }
            }
        }
        // Each figure in its own boxes, with no more than a seat, a sack or a rope besides.
        assert!(most <= crate::villagers::FIGURE_BOXES + 2, "{most} boxes to a traveller");
    }

    /// A party of settlers as big as they come, close to: eight of them, two carts each
    /// drawn by an ox, on the road, and in camp at night with their tents and bedrolls.
    fn biggest_party(camp: bool) -> Scene {
        let doing = |k: usize| match (camp, k) {
            (true, _) => if k % 3 == 1 { Doing::Standing } else { Doing::Sitting },
            (false, 0 | 6) => Doing::Leading,
            _ => Doing::Walking,
        };
        let people = (0..8).map(|k| Figure { feet: Vec3::new(k as f32 * m(1.5), 0.0, 0.0), ..figure(k as u32 * 77, doing(k)) }).collect();
        let beasts = (0..2).map(|k| Beast { kind: worldgen::fauna::Kind::Ox, seed: k, feet: Vec3::new(k as f32 * m(6.0), 0.0, m(1.0)), facing: 0.0, walking: !camp, grazing: false }).collect();
        let carts = (0..2).map(|k| Cart { at: Vec3::new(k as f32 * m(6.0) - m(2.0), 0.0, m(1.0)), heading: 0.0, load: k * 31, beast: (!camp).then_some(k as usize) }).collect();
        Scene {
            id: 2,
            kind: Kind::Settlers,
            region: Region::Temperate,
            from: None,
            to: None,
            people,
            fire: camp.then_some((Vec3::new(0.0, 0.0, m(3.0)), Fire::Lit)),
            tents: if camp { vec![(Vec3::new(0.0, 0.0, m(6.0)), 0.0), (Vec3::new(m(3.0), 0.0, m(6.0)), 0.0)] } else { Vec::new() },
            bedrolls: Vec::new(),
            carts,
            beasts,
            boats: Vec::new(),
            goods: Vec::new(),
            iron: Vec::new(),
            doing: "",
            at: Vec3::ZERO,
        }
    }

    #[test]
    fn the_biggest_party_close_to_is_drawn_within_the_budget() {
        let mut people = HashMap::new();
        for camp in [false, true] {
            let seen = Seen { scene: biggest_party(camp), origin: DVec3::ZERO, distance: 0.0 };
            let mut out = Vec::new();
            let mut d = Drawing { cam: DVec3::new(m64(4.0), m64(1.7), m64(-4.0)), clock: 1.1, pace: 1.3, out: &mut out, start: 0, people: &mut people, region: Region::Temperate };
            d.scene(&seen);
            println!("the biggest party {}: {} boxes", if camp { "in camp" } else { "on the road" }, out.len());
            assert!(out.len() < BUDGET, "{} boxes for one party, of {BUDGET}", out.len());
        }
    }
}
