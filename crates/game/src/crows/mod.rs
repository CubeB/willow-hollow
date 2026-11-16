//! Crows: small groups that perch on the high points about the player, come down to feed
//! on the ground by the berries, go back to their rookery at dusk, and fly off when the
//! player comes too close.
//!
//! They are not the flocks that pass overhead (see `particles.rs`): each crow goes its
//! own way, choosing its own seat and taking off in its own time, and only the group's
//! choice of where to go next keeps them together, loosely, as crows are. Where they can
//! go comes from a map of the tops of things made on a worker thread (`survey.rs`,
//! `sites.rs`); the same map is what keeps a crow in the air out of the hills and the
//! trees. They are drawn as props (`model.rs`).

mod model;
pub mod sites;
pub mod survey;

use std::sync::Arc;

use glam::{DVec3, Vec3};
use worldgen::{noise, scale::VOXELS_PER_METRE};

use crate::particles::ParticleInstance;
use model::{Legs, Pose, Wings};
use sites::{Kind, Place, Sites};
use survey::Lookout;

/// What the crows need to know about the moment.
pub struct Around<'a> {
    pub camera: DVec3,
    /// Sun elevation (sine), negative at night.
    pub sun_elevation: f32,
    /// The hour of the day, 0 to 24: whether the light is going or coming.
    pub hour: f32,
    /// The time of year (season.rs), for the trees in leaf.
    pub date: f32,
    pub rain: f32,
    /// The height of the land at a place, roughly (`particles::land_under`), for what is
    /// beyond the crows' own map.
    pub land: &'a dyn Fn(f64, f64) -> Option<f64>,
}

/// Metres to voxels.
const fn v(metres: f32) -> f32 {
    metres * VOXELS_PER_METRE
}

/// How near the player comes before a crow on the ground takes off, in metres; one up
/// in a tree or on a rock lets the player a little nearer.
const FLEE_GROUND: f32 = 13.0;
const FLEE_PERCH: f32 = 9.0;
/// How fast a crow flies, in metres a second.
const SPEED: f32 = 9.0;
/// Groups about at once, at the most.
const MAX_GROUPS: usize = 3;
/// How far off, in metres, a group is let go of.
const GONE: f32 = 280.0;
/// How far off crows and nests are drawn, in metres.
const SIGHT: f32 = 300.0;
const NEST_SIGHT: f32 = 140.0;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum State {
    /// Sitting up on a crown, a rock, a roof or a nest.
    Perched,
    /// On the ground, walking about and pecking.
    Ground,
    /// On the way to a spot.
    Flying,
    /// Coming in to the spot, wings flared.
    Landing,
    /// Going away out of sight, not to come back.
    Leaving,
}

/// What a settled crow is doing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Act {
    Still,
    Look,
    Peck,
    Caw,
    Stretch,
    Shuffle,
    Walk,
}

/// A hop along the ground.
#[derive(Clone, Copy, Debug)]
struct Hop {
    from: DVec3,
    to: DVec3,
    t: f32,
}

const HOP_TIME: f32 = 0.28;

#[derive(Clone, Debug)]
struct Crow {
    /// Where its feet are (or would be, in the air), in voxels.
    pos: DVec3,
    /// Voxels a second.
    vel: Vec3,
    yaw: f32,
    pitch: f32,
    bank: f32,
    state: State,
    /// Where it is going, or sitting.
    spot: DVec3,
    /// Seconds until it takes off for `spot`, when it is to.
    going: Option<f32>,
    /// Seconds since it took off, or came in.
    air: f32,
    settled: f32,
    /// Through its wing beat, radians, and the wings as they are drawn this moment.
    flap: f32,
    wings: Wings,
    seed: f32,
    /// Its own speed, as a share of [`SPEED`].
    pace: f32,
    head: f32,
    head_to: f32,
    dip: f32,
    beak: f32,
    tail: f32,
    fan: f32,
    act: Act,
    act_t: f32,
    act_len: f32,
    next_act: f32,
    hop: Option<Hop>,
    hops: u32,
    yaw_to: f32,
}

impl Crow {
    fn new(pos: DVec3, state: State, seed: f32) -> Self {
        Crow {
            pos,
            vel: Vec3::ZERO,
            yaw: seed * 7.0,
            pitch: -0.3,
            bank: 0.0,
            state,
            spot: pos,
            going: None,
            air: 0.0,
            settled: 10.0,
            flap: seed,
            wings: Wings::Folded,
            seed,
            pace: 0.9 + 0.2 * noise::unit(noise::hash2(0xc40, (seed * 1000.0) as i32, 1)),
            head: 0.0,
            head_to: 0.0,
            dip: 0.0,
            beak: 0.0,
            tail: 0.15,
            fan: 0.0,
            act: Act::Still,
            act_t: 0.0,
            act_len: 0.0,
            next_act: seed.fract() * 3.0,
            hop: None,
            hops: 0,
            yaw_to: seed * 7.0,
        }
    }

    fn settled_state(&self) -> bool {
        matches!(self.state, State::Perched | State::Ground)
    }

    fn take_off(&mut self, spot: DVec3, away: Option<DVec3>) {
        let to = spot - self.pos;
        let flat = Vec3::new(to.x as f32, 0.0, to.z as f32).normalize_or(Vec3::new(self.yaw.sin(), 0.0, self.yaw.cos()));
        // Off with a spring and hard beats; startled, straight up and away first.
        let from = away.map(|a| {
            let d = self.pos - a;
            Vec3::new(d.x as f32, 0.0, d.z as f32).normalize_or(flat)
        });
        let push = from.map_or(flat * v(2.5), |f| f * v(3.5));
        self.vel = push + Vec3::Y * v(if away.is_some() { 4.2 } else { 3.2 });
        self.state = State::Flying;
        self.spot = spot;
        self.going = None;
        self.air = 0.0;
        self.hop = None;
        self.act = Act::Still;
        self.dip = 0.0;
        self.beak = 0.0;
    }

    fn land(&mut self) {
        self.pos = self.spot;
        self.vel = Vec3::ZERO;
        self.settled = 0.0;
        self.air = 0.0;
        self.act = Act::Still;
        self.next_act = 0.4 + self.seed.fract() * 1.5;
        self.yaw_to = self.yaw;
    }
}

/// A few crows that keep together, loosely.
#[derive(Clone, Debug)]
struct Group {
    crows: Vec<Crow>,
    /// What kind of place it is at or making for, and its middle.
    kind: Kind,
    centre: DVec3,
    /// Seconds until it moves on.
    stay: f32,
    age: f32,
    leaving: bool,
    /// Never startled, nor moving on: crows set out for a screenshot.
    tame: bool,
}

impl Group {
    fn middle(&self) -> DVec3 {
        self.crows.iter().map(|c| c.pos).sum::<DVec3>() / self.crows.len().max(1) as f64
    }
}

/// Every crow about the player.
pub struct Crows {
    lookout: Option<Lookout>,
    look_timer: f32,
    sites: Option<Arc<Sites>>,
    groups: Vec<Group>,
    rng: u32,
    spawn_timer: f32,
    /// Whether any have been put out yet since the map came in.
    started: bool,
}

/// What the light is doing, as it matters to a crow.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Light {
    Day,
    /// The light is going: back to the rookery.
    Dusk,
    Night,
}

fn light(a: &Around) -> Light {
    if a.sun_elevation > 0.07 {
        Light::Day
    } else if a.hour > 12.0 && a.sun_elevation > -0.12 {
        Light::Dusk
    } else {
        Light::Night
    }
}

impl Crows {
    /// Crows for the game, with a worker thread making their map round the player.
    pub fn new(world: Arc<worldgen::world::GameWorld>) -> Self {
        let mut c = Self::without_lookout();
        c.lookout = Some(Lookout::new(world));
        c
    }

    /// Crows on a map handed to them (see [`Crows::set_sites`]): for a screenshot, or a test.
    pub fn without_lookout() -> Self {
        Self { lookout: None, look_timer: 0.0, sites: None, groups: Vec::new(), rng: 0x6c0f_f1e5, spawn_timer: 0.0, started: false }
    }

    pub fn set_sites(&mut self, sites: Sites) {
        self.sites = Some(Arc::new(sites));
    }

    fn unit(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        noise::unit(self.rng)
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }

    /// How many crows there are, and how many of them are in the air.
    #[cfg(test)]
    fn count(&self) -> (usize, usize) {
        let all = self.groups.iter().map(|g| g.crows.len()).sum();
        let flying = self.groups.iter().flat_map(|g| &g.crows).filter(|c| !c.settled_state()).count();
        (all, flying)
    }

    pub fn update(&mut self, dt: f32, a: &Around) {
        let dt = dt.min(0.1);
        if let Some(l) = &mut self.lookout {
            self.look_timer -= dt;
            if self.look_timer <= 0.0 {
                self.look_timer = 5.0;
                l.request(a.camera, a.date);
            }
            if let Some(s) = l.poll() {
                self.sites = Some(Arc::new(s));
            }
        }
        let Some(sites) = self.sites.clone() else { return };
        let light = light(a);

        // New groups now and then, by day; at dusk and through the night the rookery
        // has its crows.
        self.spawn_timer -= dt;
        if !self.started || self.spawn_timer <= 0.0 {
            self.spawn_timer = self.range(10.0, 25.0);
            let most = if a.rain > 0.5 { MAX_GROUPS - 1 } else { MAX_GROUPS };
            match light {
                Light::Day if self.groups.len() < most => {
                    let settled = !self.started || self.unit() < 0.5;
                    self.spawn(&sites, a, settled, None);
                    if !self.started && self.unit() < 0.6 {
                        self.spawn(&sites, a, true, None);
                    }
                }
                Light::Dusk | Light::Night if self.groups.iter().all(|g| g.kind != Kind::Roost) => {
                    let roost = sites.roosts().min_by(|p, q| p.centre.distance(a.camera).total_cmp(&q.centre.distance(a.camera))).cloned();
                    if let Some(r) = roost {
                        self.spawn(&sites, a, true, Some(r));
                    }
                }
                _ => {}
            }
            self.started = true;
        }

        for gi in 0..self.groups.len() {
            self.update_group(gi, dt, a, light, &sites);
        }
        let camera = a.camera;
        // Those gone out of sight are let go of, and a group once all of it has gone
        // (but not one still on its way in).
        for g in &mut self.groups {
            g.crows.retain(|c| c.state != State::Leaving || (c.pos.distance(camera) as f32) < v(GONE));
        }
        self.groups.retain(|g| !g.crows.is_empty() && (g.age < 40.0 || g.crows.iter().any(|c| (c.pos.distance(camera) as f32) < v(GONE))));
    }

    /// Puts out a new group: settled at a place a little way off, or flying in from out
    /// of sight to one; at `roost` if given.
    fn spawn(&mut self, sites: &Sites, a: &Around, settled: bool, roost: Option<Place>) {
        let n = 3 + (self.unit().powf(1.6) * 9.99) as usize;
        let place = match roost {
            Some(r) => Some(r),
            None => self.choose(sites, a.camera, a.camera, n, false, Light::Day, (v(40.0), v(110.0))),
        };
        let Some(place) = place else { return };
        let spots = self.spots(sites, &place, n);
        if spots.is_empty() {
            return;
        }
        let state = if place.kind.on_ground() { State::Ground } else { State::Perched };
        let mut crows = Vec::new();
        let (heading, far) = (self.range(0.0, std::f32::consts::TAU), v(self.range(190.0, 230.0)) as f64);
        for spot in spots {
            let seed = self.range(0.0, 1000.0);
            let mut c = Crow::new(spot, state, seed);
            if !settled {
                // From out of sight, high enough to clear whatever lies between.
                let off = DVec3::new(heading.cos() as f64, 0.0, heading.sin() as f64) * far
                    + DVec3::new(self.range(-1.0, 1.0) as f64, self.range(-0.5, 0.5) as f64, self.range(-1.0, 1.0) as f64) * v(5.0) as f64;
                let mut start = place.centre + off;
                let ground = (a.land)(start.x, start.z).unwrap_or(place.centre.y);
                start.y = (place.centre.y + v(25.0) as f64).max(ground + v(18.0) as f64) + off.y;
                c.pos = start;
                c.take_off(spot, None);
                c.vel = (spot - start).as_vec3().with_y(0.0).normalize_or(Vec3::X) * v(SPEED);
                c.air = 5.0;
            }
            crows.push(c);
        }
        let stay = self.range(15.0, 45.0);
        let at_night = place.kind == Kind::Roost;
        log::debug!("{} crows {} {:?} at {:.0} {:.0} {:.0}", crows.len(), if settled { "settled on" } else { "flying in to" }, place.kind, place.centre.x, place.centre.y, place.centre.z);
        self.groups.push(Group { crows, kind: place.kind, centre: place.centre, stay: if at_night { 1e9 } else { stay }, age: 0.0, leaving: false, tame: false });
    }

    /// Where a group of `n` at `from` goes next: a place by its kind and how far it is,
    /// not too near the player at `camera`, and when `scared` further from the player
    /// than they are now.
    #[allow(clippy::too_many_arguments)]
    fn choose(&mut self, sites: &Sites, from: DVec3, camera: DVec3, n: usize, scared: bool, light: Light, (near, far): (f32, f32)) -> Option<Place> {
        let flat = |p: DVec3, q: DVec3| ((p.x - q.x).hypot(p.z - q.z)) as f32;
        let mut total = 0.0;
        let mut weights = Vec::with_capacity(sites.places.len());
        for p in &sites.places {
            let kind_weight = match (p.kind, light) {
                (Kind::Roost, Light::Dusk | Light::Night) => 50.0,
                (_, Light::Dusk | Light::Night) => 0.0,
                (Kind::Berries, _) => 3.0,
                (Kind::Field, _) => 1.4,
                (Kind::Tree, _) => 2.0,
                (Kind::Rock, _) => 1.7,
                (Kind::Roost, _) => 0.5,
            };
            let d = flat(p.centre, from);
            let to_player = flat(p.centre, camera);
            let ok = kind_weight > 0.0
                && (p.kind == Kind::Roost || (d >= near && d <= far))
                && to_player > v(if p.kind.on_ground() { FLEE_GROUND } else { FLEE_PERCH } + 8.0)
                && (!scared || to_player > v(35.0).max(flat(from, camera) + v(10.0)))
                && p.spots.len() >= n.min(3);
            let w = if ok { kind_weight / (1.0 + d / v(40.0)) } else { 0.0 };
            total += w;
            weights.push(w);
        }
        if total <= 0.0 {
            return None;
        }
        let mut pick = self.unit() * total;
        for (p, w) in sites.places.iter().zip(weights) {
            pick -= w;
            if w > 0.0 && pick <= 0.0 {
                return Some(p.clone());
            }
        }
        None
    }

    /// The spots for `n` crows at `place`: its own, then those of places of the same sort
    /// nearby, then any seat a short flight off. Fewer if there are not so many.
    fn spots(&mut self, sites: &Sites, place: &Place, n: usize) -> Vec<DVec3> {
        // Not one another crow is sitting in or making for.
        let taken: Vec<DVec3> = self.groups.iter().flat_map(|g| &g.crows).map(|c| c.spot).collect();
        let free = |s: &DVec3| taken.iter().all(|t| t.distance(*s) > 0.5);
        let mut spots: Vec<DVec3> = place.spots.iter().copied().filter(free).collect();
        if spots.len() < n {
            let mut others: Vec<&Place> = sites
                .places
                .iter()
                .filter(|p| p.centre != place.centre && p.kind.on_ground() == place.kind.on_ground() && p.centre.distance(place.centre) < v(25.0) as f64)
                .collect();
            others.sort_by(|p, q| p.centre.distance(place.centre).total_cmp(&q.centre.distance(place.centre)));
            for p in others {
                for s in &p.spots {
                    if free(s) && spots.iter().all(|o| o.distance(*s) > 1.0) {
                        spots.push(*s);
                    }
                }
                if spots.len() >= n {
                    break;
                }
            }
        }
        // Each gets one at random, so a group does not fill a tree from one side.
        for i in (1..spots.len()).rev() {
            let j = (self.unit() * (i + 1) as f32) as usize % (i + 1);
            spots.swap(i, j);
        }
        spots.truncate(n);
        spots
    }

    fn update_group(&mut self, gi: usize, dt: f32, a: &Around, light: Light, sites: &Sites) {
        let mut scared = false;
        {
            let g = &mut self.groups[gi];
            g.age += dt;
            if !g.tame {
                g.stay -= dt;
            }
            // When the light goes, home; when it comes, out again.
            if light == Light::Dusk && g.kind != Kind::Roost && !g.leaving && g.stay > 20.0 {
                g.stay = noise::unit(noise::hash2(0xd05c, gi as i32, g.age as i32)) * 20.0;
            }
            if light == Light::Day && g.kind == Kind::Roost && g.stay > 60.0 {
                g.stay = 5.0 + 50.0 * noise::unit(noise::hash2(0xda1, gi as i32, 3));
            }
        }
        // Each crow on its own; any that took fright puts the others near it up.
        let mut fright: Vec<DVec3> = Vec::new();
        let n = self.groups[gi].crows.len();
        let (mates_pos, mates_vel, flying) = {
            let g = &self.groups[gi];
            let fl: Vec<&Crow> = g.crows.iter().filter(|c| matches!(c.state, State::Flying | State::Leaving)).collect();
            let k = fl.len().max(1) as f64;
            (fl.iter().map(|c| c.pos).sum::<DVec3>() / k, fl.iter().map(|c| c.vel).sum::<Vec3>() / k as f32, fl.len())
        };
        for i in 0..n {
            let tame = self.groups[gi].tame;
            let r = (self.unit(), self.unit(), self.unit());
            let others: Vec<DVec3> = self.groups[gi].crows.iter().enumerate().filter(|&(j, c)| j != i && !c.settled_state()).map(|(_, c)| c.pos).collect();
            let c = &mut self.groups[gi].crows[i];
            if step(c, dt, a, sites, tame, r, &others, (mates_pos, mates_vel, flying)) {
                fright.push(c.pos);
            }
        }
        if !fright.is_empty() {
            scared = true;
            let g = &mut self.groups[gi];
            for c in g.crows.iter_mut().filter(|c| c.settled_state() && c.going.is_none()) {
                if fright.iter().any(|f| f.distance(c.pos) < v(18.0) as f64) {
                    let delay = 0.05 + 0.5 * noise::unit(noise::hash2(0xf1ee, (c.seed * 100.0) as i32, g.age as i32));
                    c.going = Some(delay);
                    c.spot = c.pos;
                }
            }
            g.stay = 0.0;
        }
        // Moving on: somewhere new for the whole group, each crow off in its own time.
        let g = &self.groups[gi];
        if g.stay > 0.0 || g.leaving {
            return;
        }
        let (from, n, age) = (g.middle(), g.crows.len(), g.age);
        let go_away = match light {
            Light::Day => !scared && age > 90.0 && self.unit() < 0.12,
            _ => false,
        };
        let range = if scared { (v(25.0), v(110.0)) } else { (v(12.0), v(90.0)) };
        let place = if go_away { None } else { self.choose(sites, from, a.camera, n, scared, light, range) };
        let Some(place) = place else {
            // Nowhere to go here: away out of sight, as crows do at dusk where there is
            // no rookery near.
            let from_player = (from - a.camera).with_y(0.0).normalize_or(DVec3::X);
            let away = from + from_player * v(450.0) as f64;
            let ground = (a.land)(away.x, away.z).unwrap_or(from.y);
            let target = away.with_y(from.y.max(ground) + v(40.0) as f64);
            let g = &mut self.groups[gi];
            g.leaving = true;
            for (k, c) in g.crows.iter_mut().enumerate() {
                c.spot = target + DVec3::new(k as f64 * 2.0, 0.0, (k % 3) as f64 * 3.0);
                if c.settled_state() {
                    c.going = Some(c.going.unwrap_or(0.0).max(k as f32 * 0.35));
                } else {
                    c.state = State::Leaving;
                }
            }
            return;
        };
        let spots = self.spots(sites, &place, n);
        let stay = match place.kind {
            Kind::Roost if light != Light::Day => 1e9,
            k if k.on_ground() => self.range(20.0, 60.0),
            _ => self.range(15.0, 45.0),
        };
        let delays: Vec<f32> = (0..n).map(|_| if scared { self.range(0.05, 0.6) } else { self.range(0.0, 3.0) }).collect();
        let g = &mut self.groups[gi];
        g.kind = place.kind;
        g.centre = place.centre;
        g.stay = stay;
        // Those there are no spots for stay with the nearest that has one.
        let mut spare = Vec::new();
        for (k, c) in g.crows.iter_mut().enumerate() {
            match spots.get(k) {
                Some(&s) => {
                    c.spot = s;
                    if c.settled_state() {
                        c.going = Some(c.going.unwrap_or(delays[k]));
                    } else if c.state != State::Leaving {
                        c.state = State::Flying;
                    }
                }
                None => spare.push(k),
            }
        }
        // A crow without a seat goes off on its own.
        for k in spare.into_iter().rev() {
            if g.crows[k].pos.distance(a.camera) > v(60.0) as f64 {
                g.crows.remove(k);
            } else {
                let c = &mut g.crows[k];
                c.spot = c.pos + DVec3::new(0.0, v(40.0) as f64, 0.0) + (c.pos - a.camera).with_y(0.0).normalize_or(DVec3::X) * v(400.0) as f64;
                c.state = State::Leaving;
            }
        }
    }

    /// Draws every crow and the nests near enough to `cam`.
    pub fn draw(&self, cam: DVec3, out: &mut Vec<ParticleInstance>) {
        if let Some(sites) = &self.sites {
            for n in &sites.nests {
                if n.pos.distance(cam) < v(NEST_SIGHT) as f64 {
                    model::draw_nest(n, cam, out);
                }
            }
        }
        for c in self.groups.iter().flat_map(|g| &g.crows) {
            if c.pos.distance(cam) < v(SIGHT) as f64 {
                model::draw(&pose(c), cam, out);
            }
        }
    }

    /// Writes where each crow is to the log, for aiming a screenshot.
    pub fn report(&self, camera: DVec3) {
        for g in &self.groups {
            let at = g.middle() - camera;
            let states: Vec<&str> = g.crows.iter().map(|c| match c.state {
                State::Perched => "perched",
                State::Ground => "ground",
                State::Flying => "flying",
                State::Landing => "landing",
                State::Leaving => "leaving",
            }).collect();
            log::info!("{} crows at {:?}, {:.0} {:.0} {:.0} from the camera: {}", g.crows.len(), g.kind, at.x, at.y, at.z, states.join(" "));
        }
    }

    /// Sets out a group for a screenshot, as `how` says: `ground` (feeding), `perch`,
    /// `fly` (passing), `land` (coming in) or `roost`, at the place of that sort nearest
    /// to `ahead`. They are never startled and stay where they are put.
    pub fn stage(&mut self, how: &str, ahead: DVec3, camera: DVec3) -> bool {
        let Some(sites) = self.sites.clone() else { return false };
        let wanted = |p: &Place| match how {
            "ground" | "land" => p.kind.on_ground(),
            "perch" => matches!(p.kind, Kind::Tree | Kind::Rock),
            "tree" => p.kind == Kind::Tree,
            "rock" => p.kind == Kind::Rock,
            "roost" => p.kind == Kind::Roost,
            _ => matches!(p.kind, Kind::Tree | Kind::Rock) || p.kind.on_ground(),
        };
        let Some(place) = sites.places.iter().filter(|p| wanted(p)).min_by(|p, q| p.centre.distance(ahead).total_cmp(&q.centre.distance(ahead))).cloned() else {
            log::warn!("no place for crows to {how} near {:.0} {:.0}", ahead.x, ahead.z);
            return false;
        };
        let n = if how == "roost" { place.spots.len().min(12) } else { 7 };
        let spots = self.spots(&sites, &place, n);
        log::info!("{} crows set out to {how} at {:.0} {:.0} {:.0}, {:?}", spots.len(), place.centre.x, place.centre.y, place.centre.z, place.kind);
        let state = if place.kind.on_ground() { State::Ground } else { State::Perched };
        let mut crows = Vec::new();
        for (k, spot) in spots.into_iter().enumerate() {
            let mut c = Crow::new(spot, state, 17.0 + k as f32 * 3.7);
            let to_cam = camera - spot;
            c.yaw = (-(to_cam.x as f32)).atan2(to_cam.z as f32) + (k as f32 * 2.1).sin() * 1.4;
            c.yaw_to = c.yaw;
            match how {
                "fly" => {
                    // Across the view, a few metres up, each its own way along.
                    let across = Vec3::new(-(to_cam.z as f32), 0.0, to_cam.x as f32).normalize_or(Vec3::X);
                    c.pos = spot + DVec3::new(0.0, v(3.0 + k as f32 * 0.7) as f64, 0.0) + (across * v(k as f32 * 1.6 - 5.0)).as_dvec3();
                    c.take_off(c.pos + (across * v(60.0)).as_dvec3(), None);
                    c.vel = across * v(SPEED) + Vec3::Y * v((k as f32 * 1.3).sin());
                    c.air = 3.0 + k as f32 * 0.37;
                    c.state = State::Leaving;
                }
                "land" => {
                    let back = Vec3::new(c.yaw.sin(), 0.0, -c.yaw.cos());
                    c.pos = spot + (back * v(1.8) + Vec3::Y * v(1.0)).as_dvec3();
                    c.take_off(spot, None);
                    c.state = State::Landing;
                    c.air = 0.6 + k as f32 * 0.3;
                    c.vel = -back * v(2.5) - Vec3::Y * v(1.0);
                }
                _ => {
                    c.next_act = k as f32 * 0.4;
                }
            }
            crows.push(c);
        }
        self.groups.push(Group { crows, kind: place.kind, centre: place.centre, stay: 1e9, age: 0.0, leaving: false, tame: true });
        self.started = true;
        self.spawn_timer = 1e9;
        true
    }
}

/// The floor under a crow at `(x, z)`: the top of what is there, or where the map has
/// not been made, well above the land as the continent has it.
fn floor(sites: &Sites, a: &Around, x: f64, z: f64, rock_only: bool) -> Option<f64> {
    match sites.grid.top(x, z, rock_only) {
        Some(t) => Some(t),
        None => (a.land)(x, z).map(|l| l + v(10.0) as f64),
    }
}

/// Moves one crow on by `dt`. True if it took fright at the player.
#[allow(clippy::too_many_arguments)]
fn step(c: &mut Crow, dt: f32, a: &Around, sites: &Sites, tame: bool, r: (f32, f32, f32), others: &[DVec3], mates: (DVec3, Vec3, usize)) -> bool {
    let mut frightened = false;
    match c.state {
        State::Perched | State::Ground => {
            c.settled += dt;
            let near = (c.pos.distance(a.camera) as f32) < v(if c.state == State::Ground { FLEE_GROUND } else { FLEE_PERCH });
            if near && !tame {
                c.take_off(c.spot, Some(a.camera));
                // Its spot is the player's now: the group finds it another.
                c.spot = c.pos + DVec3::new(0.0, v(6.0) as f64, 0.0);
                frightened = true;
            } else if let Some(t) = &mut c.going {
                *t -= dt;
                if *t <= 0.0 {
                    if c.spot.distance(c.pos) < 0.5 {
                        c.going = None;
                    } else {
                        let spot = c.spot;
                        c.take_off(spot, None);
                    }
                }
            }
            if c.settled_state() {
                idle(c, dt, sites, r);
            }
        }
        State::Flying | State::Leaving => fly(c, dt, a, sites, others, mates),
        State::Landing => come_in(c, dt, sites),
    }
    animate(c, dt);
    frightened
}

/// A settled crow's own business: looking about, pecking, walking, cawing.
fn idle(c: &mut Crow, dt: f32, sites: &Sites, r: (f32, f32, f32)) {
    let ground = c.state == State::Ground;
    c.act_t += dt;
    if c.act_t >= c.act_len {
        c.next_act -= dt;
        if c.act != Act::Still {
            c.act = Act::Still;
            c.next_act = 0.3 + 2.2 * r.0;
        }
        if c.next_act <= 0.0 && c.hop.is_none() {
            let roll = r.1;
            c.act = if ground {
                match roll {
                    x if x < 0.42 => Act::Peck,
                    x if x < 0.74 => Act::Walk,
                    x if x < 0.88 => Act::Look,
                    _ => Act::Caw,
                }
            } else {
                match roll {
                    x if x < 0.4 => Act::Look,
                    x if x < 0.6 => Act::Shuffle,
                    x if x < 0.78 => Act::Caw,
                    x if x < 0.9 => Act::Stretch,
                    _ => Act::Still,
                }
            };
            c.act_t = 0.0;
            c.act_len = match c.act {
                Act::Peck => 0.6 + 0.9 * r.2,
                // Until the last hop is done (see below).
                Act::Walk => 5.0,
                Act::Look => 0.8 + 1.6 * r.2,
                Act::Caw => 1.1 + 0.6 * r.2,
                Act::Stretch => 0.7,
                Act::Shuffle => 0.5,
                Act::Still => 1.0,
            };
            match c.act {
                Act::Look => c.head_to = (r.2 - 0.5) * 2.6,
                Act::Shuffle => c.yaw_to = c.yaw + (r.2 - 0.5) * 2.0,
                Act::Walk => {
                    c.hops = 1 + (r.2 * 3.0) as u32;
                    // Mostly on the way it faces, now and then turning.
                    c.yaw_to = c.yaw + (r.0 - 0.5) * 2.2;
                }
                _ => {}
            }
        }
    }
    // Hopping along: a short arc to a spot of level ground beside it.
    if ground && c.hop.is_none() && c.hops > 0 && c.act == Act::Walk {
        c.hops -= 1;
        let dir = Vec3::new(-c.yaw_to.sin(), 0.0, c.yaw_to.cos());
        let to = c.pos + (dir * v(0.28 + 0.2 * r.1)).as_dvec3();
        // Not far from where the group came down, and only onto ground as high.
        if let Some(y) = sites.grid.open_ground(to).filter(|y| (y - c.pos.y).abs() <= 1.0)
            && to.distance(c.spot) < v(5.0) as f64
        {
            c.hop = Some(Hop { from: c.pos, to: to.with_y(y), t: 0.0 });
        } else {
            c.yaw_to += 2.0;
        }
        if c.hops == 0 {
            c.act_len = c.act_t + HOP_TIME;
        }
    }
    if let Some(h) = &mut c.hop {
        h.t += dt / HOP_TIME;
        let t = h.t.min(1.0) as f64;
        let lift = (std::f64::consts::PI * t).sin() * v(0.12) as f64;
        c.pos = h.from.lerp(h.to, t) + DVec3::new(0.0, lift, 0.0);
        if h.t >= 1.0 {
            c.pos = h.to;
            c.hop = None;
        }
    }
    let off = (c.yaw_to - c.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    c.yaw += off * (1.0 - (-6.0 * dt).exp());
}

/// On the way to its spot: a climb off, a level flight, a long glide down to it, kept
/// above what it passes over, and loosely with the others of its group in the air.
fn fly(c: &mut Crow, dt: f32, a: &Around, sites: &Sites, others: &[DVec3], mates: (DVec3, Vec3, usize)) {
    c.air += dt;
    let leaving = c.state == State::Leaving;
    let to = c.spot - c.pos;
    let dh = (to.x.hypot(to.z)) as f32;
    let dir = Vec3::new(to.x as f32, 0.0, to.z as f32) / dh.max(1e-3);
    let mut speed = v(SPEED) * c.pace;
    if !leaving && dh < v(12.0) {
        speed *= (dh / v(12.0)).max(0.3);
    }
    // Down the glide path to the spot, and in the first moments up off the perch.
    let mut aim = c.spot.y + (dh * 0.35).min(v(16.0)) as f64;
    if c.air < 0.8 {
        aim = aim.max(c.pos.y + v(2.0) as f64);
    }
    // Clear of what lies ahead, but for the last few metres in, where it has to come
    // down among the leaves to its seat.
    let final_in = !leaving && dh < v(4.0);
    let flat_vel = Vec3::new(c.vel.x, 0.0, c.vel.z);
    for t in [0.0f32, 0.35, 0.7, 1.2] {
        let p = c.pos + (flat_vel * t).as_dvec3();
        if let Some(f) = floor(sites, a, p.x, p.z, final_in) {
            aim = aim.max(f + if final_in { 0.3 } else { v(1.4) as f64 });
        }
    }
    let climb = (((aim - c.pos.y) as f32) * 1.4).clamp(-v(4.0), v(4.5));
    let mut want = dir * speed + Vec3::Y * climb;
    // Keeping loosely with the others in the air: their way, and room between.
    if mates.2 >= 2 {
        want += (mates.1 - c.vel).with_y(0.0) * 0.25;
        let to_mates = (mates.0 - c.pos).as_vec3().with_y(0.0);
        if to_mates.length() > v(10.0) {
            want += to_mates.normalize() * v(1.0);
        }
    }
    for o in others {
        let d = (c.pos - *o).as_vec3();
        let l = d.length();
        if l < v(1.6) && l > 1e-3 {
            want += d / l * v(2.0) * (1.0 - l / v(1.6));
        }
    }
    let s = c.seed as f64;
    want += Vec3::new(noise::perlin2(0xc41, s, c.air as f64 * 0.5), 0.0, noise::perlin2(0xc42, s, c.air as f64 * 0.5)) * v(1.2);
    c.vel += (want - c.vel) * (1.0 - (-3.0 * dt).exp());
    // Where the map has been made it is a hard floor; beyond, the land is only roughly
    // known, and is kept above by the aim alone.
    let mut next = c.pos + (c.vel * dt).as_dvec3();
    if let Some(f) = sites.grid.under(next.x, next.z, final_in)
        && next.y < f
    {
        next.y = f;
        c.vel.y = c.vel.y.max(0.0);
    }
    c.pos = next;
    if !leaving && c.pos.distance(c.spot) < v(3.0) as f64 {
        c.state = State::Landing;
        c.air = 0.0;
    }
    // Long gone from view, or lost: it is where it was going.
    if !leaving && c.air > 40.0 && c.pos.distance(a.camera) > v(70.0) as f64 {
        c.state = State::Landing;
        c.pos = c.spot;
    }
}

/// Coming in: slowing hard, wings flared, legs down, onto the spot.
fn come_in(c: &mut Crow, dt: f32, sites: &Sites) {
    c.air += dt;
    let to = (c.spot - c.pos).as_vec3();
    let d = to.length();
    let want = to / d.max(1e-3) * (d * 2.2).clamp(v(0.5), v(4.5));
    c.vel += (want - c.vel) * (1.0 - (-6.0 * dt).exp());
    let mut next = c.pos + (c.vel * dt).as_dvec3();
    if let Some(f) = sites.grid.under(next.x, next.z, true)
        && next.y < f
        && f <= c.spot.y + 0.01
    {
        next.y = f;
    }
    c.pos = next;
    if d < v(0.08) || c.air > 2.5 {
        c.land();
        let ground = sites.grid.open_ground(c.spot).is_some_and(|y| (y - c.spot.y).abs() < 0.01);
        c.state = if ground { State::Ground } else { State::Perched };
    }
}

/// The heading, lean and wings for the moment.
fn animate(c: &mut Crow, dt: f32) {
    let tau = std::f32::consts::TAU;
    let ease = |x: &mut f32, to: f32, rate: f32| *x += (to - *x) * (1.0 - (-rate * dt).exp());
    match c.state {
        State::Flying | State::Leaving | State::Landing => {
            let landing = c.state == State::Landing;
            let flat = c.vel.x.hypot(c.vel.z);
            if flat > v(if landing { 0.8 } else { 0.3 }) {
                let want = (-c.vel.x).atan2(c.vel.z);
                let off = (want - c.yaw + std::f32::consts::PI).rem_euclid(tau) - std::f32::consts::PI;
                let turn = off * (1.0 - (-5.0 * dt).exp());
                c.yaw += turn;
                ease(&mut c.bank, (turn / dt.max(1e-4) * 0.35).clamp(-0.8, 0.8), 4.0);
            }
            let speed = c.vel.length().max(1e-3);
            let climbing = c.vel.y > v(0.8);
            if landing {
                let close = (1.0 - (c.pos.distance(c.spot) as f32 / v(3.0))).clamp(0.0, 1.0);
                ease(&mut c.pitch, -0.45 - 0.6 * close, 6.0);
                ease(&mut c.bank, 0.0, 6.0);
                // Flared: wings up and spread wide, slow deep beats to hold it.
                c.flap += 3.4 * tau * dt;
                let s = c.flap.sin();
                c.wings = Wings::Open { raise: 0.55 + 0.7 * s, hand: 0.35 * (c.flap - 0.9).sin(), sweep: 0.02 };
                c.fan = 1.0;
                ease(&mut c.tail, 0.45, 8.0);
            } else {
                ease(&mut c.pitch, (-(c.vel.y / speed).clamp(-0.7, 0.7).asin() * 0.7).clamp(-0.5, 0.45), 4.0);
                let s = c.seed as f64;
                let rest = noise::perlin2(0xc43, s, c.air as f64 * 0.45) > 0.15;
                let gliding = !climbing && c.air > 1.2 && (c.vel.y < -v(0.7) || rest);
                if gliding {
                    // On held wings, the fingers spread, the hand a little swept.
                    let held = Wings::Open { raise: 0.1, hand: -0.06, sweep: 0.22 };
                    c.wings = blend(c.wings, held, 1.0 - (-8.0 * dt).exp());
                    c.fan = 0.4;
                } else {
                    // Rowing: steady deep beats, the hand trailing the arm, quicker off
                    // the ground and in a climb.
                    let (rate, depth) = if c.air < 0.7 { (5.2, 0.95) } else if climbing { (4.8, 0.85) } else { (4.1, 0.7) };
                    c.flap += rate * tau * dt;
                    let s = c.flap.sin();
                    c.wings = Wings::Open { raise: 0.15 + depth * s, hand: 0.4 * (c.flap - 0.9).sin() - 0.05, sweep: 0.15 + 0.2 * (0.5 - 0.5 * s) };
                    c.fan = if c.air < 0.7 { 0.8 } else { 0.2 };
                }
                ease(&mut c.tail, 0.05, 6.0);
            }
            ease(&mut c.head, 0.0, 4.0);
            ease(&mut c.dip, -0.1, 6.0);
            ease(&mut c.beak, 0.0, 10.0);
        }
        State::Perched | State::Ground => {
            ease(&mut c.bank, 0.0, 8.0);
            let t = c.act_t;
            let (mut pitch, mut dip, mut beak, mut tail) = (-0.3, 0.0, 0.0, 0.15);
            match c.act {
                Act::Peck => {
                    // Quick stabs down at the ground, two or three a second.
                    let s = (t * 2.6 * tau).sin().max(0.0);
                    pitch = 0.3;
                    dip = 0.45 + 0.75 * s;
                    tail = -0.15;
                }
                Act::Look => {
                    ease(&mut c.head, c.head_to, 7.0);
                    dip = -0.2;
                }
                Act::Caw => {
                    // A bow forward with each call, the beak open, the tail flicked.
                    let s = (t * 2.8 * tau).sin().max(0.0);
                    pitch = 0.1 + 0.3 * s;
                    dip = -0.2 + 0.25 * s;
                    beak = 0.45 * s;
                    tail = 0.15 - 0.4 * s;
                }
                Act::Walk => {
                    pitch = -0.15;
                    if c.hop.is_some() {
                        tail = -0.1;
                    }
                }
                _ => {}
            }
            if c.act != Act::Look {
                ease(&mut c.head, 0.0, 3.0);
            }
            ease(&mut c.pitch, pitch, 10.0);
            ease(&mut c.dip, dip, 14.0);
            ease(&mut c.beak, beak, 18.0);
            ease(&mut c.tail, tail, 9.0);
            c.fan += (0.0 - c.fan) * (1.0 - (-5.0 * dt).exp());
            // Wings: folding away once down; held up a moment in a stretch or on a hop.
            c.wings = if c.settled < 0.35 {
                let k = c.settled / 0.35;
                Wings::Open { raise: 1.15 * (1.0 - k) + 0.2, hand: -0.4 * k, sweep: 0.6 * k }
            } else if c.act == Act::Stretch {
                Wings::Open { raise: 0.9 + 0.1 * (t * 9.0).sin(), hand: 0.25, sweep: 0.1 }
            } else if c.hop.is_some() {
                Wings::Open { raise: 0.5, hand: -0.3, sweep: 0.5 }
            } else {
                Wings::Folded
            };
        }
    }
}

fn blend(from: Wings, to: Wings, k: f32) -> Wings {
    match (from, to) {
        (Wings::Open { raise: a, hand: b, sweep: c }, Wings::Open { raise: x, hand: y, sweep: z }) => {
            Wings::Open { raise: a + (x - a) * k, hand: b + (y - b) * k, sweep: c + (z - c) * k }
        }
        _ => to,
    }
}

fn pose(c: &Crow) -> Pose {
    let legs = match c.state {
        State::Perched | State::Ground => Legs::Standing,
        State::Landing => Legs::Reaching,
        _ if c.air < 0.25 => Legs::Reaching,
        _ => Legs::Tucked,
    };
    Pose {
        feet: c.pos,
        yaw: c.yaw,
        pitch: c.pitch,
        bank: c.bank,
        wings: c.wings,
        head_turn: c.head,
        head_dip: c.dip,
        beak: c.beak,
        fan: c.fan,
        tail: c.tail,
        legs,
    }
}

#[cfg(test)]
mod tests {
    use super::sites::{Block, Grid, Surface};
    use super::*;

    /// Rolling land with a ridge of rock, a few tall crowns and a bush, mapped.
    fn country() -> Sites {
        let crowns = [(60, 60), (75, 64), (66, 80), (150, 40)];
        let land = |x: i32, z: i32| 20 + ((x as f32 * 0.03).sin() * 6.0 + (z as f32 * 0.02).cos() * 5.0) as i32;
        let grid = Grid::from_fn(0, 0, 120, 120, |bx, bz| {
            let (x, z) = (bx * 2, bz * 2);
            let ground = land(x, z);
            for (cx, cz) in crowns {
                let d2 = (x - cx).pow(2) + (z - cz).pow(2);
                if d2 <= 25 {
                    let top = ground + v(10.0) as i32 - d2 / 8;
                    return Block { top, surface: Surface::Leaves, at: 0, lift: top - ground, ground: false };
                }
            }
            if (180..196).contains(&x) && (100..180).contains(&z) {
                let top = ground + v(4.0) as i32 + (z % 7 == 0) as i32;
                return Block { top, surface: Surface::Rock, at: 0, lift: top - ground, ground: false };
            }
            let level = [(x + 2, z), (x, z + 2), (x - 2, z), (x, z - 2)].iter().all(|&(a, b)| land(a, b) == ground);
            Block { top: ground, surface: Surface::Rock, at: 0, lift: 0, ground: level }
        });
        let highs = sites::high_points(&grid);
        let mut places = sites::feeding_grounds(&grid, &[glam::IVec3::new(100, land(100, 120), 120)], &highs);
        let trees: Vec<(glam::IVec3, i32)> = crowns[..3].iter().map(|&(x, z)| (glam::IVec3::new(x, land(x, z), z), land(x, z) + v(10.0) as i32)).collect();
        places.extend(sites::roost(&trees, &[], &highs));
        places.extend(highs);
        Sites { grid, places, nests: Vec::new() }
    }

    fn around<'a>(camera: DVec3, sun: f32, hour: f32) -> Around<'a> {
        Around { camera, sun_elevation: sun, hour, date: 0.3, rain: 0.0, land: &|_, _| Some(30.0) }
    }

    /// Every crow is on top of things or above them, never in them.
    fn check(crows: &Crows, sites: &Sites, when: f32) {
        for c in crows.groups.iter().flat_map(|g| &g.crows) {
            assert!(c.pos.is_finite() && c.vel.is_finite());
            let rock = sites.grid.under(c.pos.x, c.pos.z, true);
            match c.state {
                State::Perched | State::Ground => {
                    if c.hop.is_none() {
                        let b = sites.grid.at(c.pos.x, c.pos.z).expect("on the map");
                        assert!(c.pos.y >= b.top as f64 - 1e-6 || c.pos.y >= (b.top - b.lift) as f64 && b.surface == Surface::Leaves, "{:?} crow under the top at {} ({}) after {when} s", c.state, c.pos, b.top);
                    }
                }
                State::Flying | State::Leaving => {
                    if let Some(t) = sites.grid.under(c.pos.x, c.pos.z, (c.spot - c.pos).with_y(0.0).length() < v(4.5) as f64) {
                        assert!(c.pos.y >= t - 1e-6, "flying crow in the ground at {} (top {t}) after {when} s", c.pos);
                    }
                }
                State::Landing => {
                    if let Some(t) = rock {
                        assert!(c.pos.y >= t.min(c.spot.y) - 0.5, "landing crow in rock at {} (top {t}) after {when} s", c.pos);
                    }
                }
            }
        }
    }

    #[test]
    fn crows_come_and_go_and_never_end_up_in_the_ground() {
        let sites = country();
        assert!(sites.places.iter().any(|p| p.kind == Kind::Tree) && sites.places.iter().any(|p| p.kind == Kind::Rock));
        assert!(sites.places.iter().any(|p| p.kind.on_ground()));
        let mut crows = Crows::without_lookout();
        crows.set_sites(sites.clone());
        let mut seen = std::collections::HashSet::new();
        let mut most_flying = 0;
        for step in 0..30 * 600 {
            let t = step as f32 / 30.0;
            // The player walks a slow circle through the country, putting them up.
            let camera = DVec3::new(120.0 + 70.0 * (t * 0.02).cos() as f64, 30.0, 120.0 + 70.0 * (t * 0.02).sin() as f64);
            crows.update(1.0 / 30.0, &around(camera, 0.5, 10.0));
            check(&crows, &sites, t);
            for c in crows.groups.iter().flat_map(|g| &g.crows) {
                seen.insert(c.state);
            }
            most_flying = most_flying.max(crows.count().1);
        }
        for s in [State::Perched, State::Ground, State::Flying, State::Landing] {
            assert!(seen.contains(&s), "never {s:?}");
        }
        assert!(most_flying >= 3);
    }

    #[test]
    fn a_crow_on_the_ground_takes_off_when_the_player_comes_near_and_lands_further_off() {
        let sites = country();
        let mut crows = Crows::without_lookout();
        crows.set_sites(sites.clone());
        let feed = sites.places.iter().find(|p| p.kind.on_ground()).unwrap().centre;
        let far = feed + DVec3::new(v(60.0) as f64, 0.0, 0.0);
        assert!(crows.stage("ground", feed, far));
        crows.groups[0].tame = false;
        for _ in 0..30 * 5 {
            crows.update(1.0 / 30.0, &around(far, 0.5, 10.0));
        }
        assert!(crows.groups[0].crows.iter().all(|c| c.state == State::Ground), "they stay put with nobody near");
        // The player walks up to them.
        let near = feed + DVec3::new(v(4.0) as f64, v(1.6) as f64, 0.0);
        for _ in 0..30 {
            crows.update(1.0 / 30.0, &around(near, 0.5, 10.0));
        }
        let group: Vec<&Crow> = crows.groups.iter().flat_map(|g| &g.crows).collect();
        assert!(group.iter().all(|c| !c.settled_state()), "all up within a second");
        for step in 0..30 * 40 {
            crows.update(1.0 / 30.0, &around(near, 0.5, 10.0));
            check(&crows, &sites, step as f32 / 30.0);
        }
        // Down again, out of the player's way, and loosely together.
        for g in &crows.groups {
            let middle = g.middle();
            for c in g.crows.iter().filter(|c| c.settled_state()) {
                assert!(c.pos.distance(near) > v(FLEE_PERCH) as f64, "settled {} m from the player", c.pos.distance(near) / v(1.0) as f64);
                assert!(c.pos.distance(middle) < v(30.0) as f64, "strayed {} m from its group", c.pos.distance(middle) / v(1.0) as f64);
            }
        }
    }

    #[test]
    fn at_dusk_they_go_back_to_the_rookery() {
        let sites = country();
        let roost: Vec<DVec3> = sites.roosts().flat_map(|r| r.spots.iter().copied()).collect();
        assert!(!roost.is_empty());
        let mut crows = Crows::without_lookout();
        crows.set_sites(sites.clone());
        let camera = DVec3::new(30.0, 30.0, 200.0);
        for _ in 0..30 * 20 {
            crows.update(1.0 / 30.0, &around(camera, 0.5, 15.0));
        }
        assert!(crows.count().0 > 0);
        for step in 0..30 * 120 {
            crows.update(1.0 / 30.0, &around(camera, 0.0, 19.5));
            check(&crows, &sites, step as f32 / 30.0);
        }
        let all: Vec<&Crow> = crows.groups.iter().flat_map(|g| &g.crows).collect();
        assert!(!all.is_empty());
        let home = all.iter().filter(|c| c.settled_state() && roost.iter().any(|r| r.distance(c.pos) < 0.01)).count();
        assert!(home * 3 >= all.len() * 2, "{home} of {} at the rookery", all.len());
    }
}
