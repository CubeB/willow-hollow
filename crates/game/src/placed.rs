//! Things set down in the world: a flower laid on a stone, a fire laid in its ring. Each is plain data (what it is, how many, where, which way it faces and
//! what state it is in), kept by the chunk column it stands in, so that only what is
//! near is looked at or drawn, and so that a column's things could be sent to another
//! player along with the column. They are saved with their world.
//!
//! Nothing here is terrain. A thing is drawn as a small model of boxes, its fine parts
//! a tenth of a metre, standing on whatever ground was there when it was set down.

use std::collections::HashMap;

use glam::{DVec2, DVec3, Vec3};
use voxel::CHUNK;
use worldgen::scale::{VOXELS_PER_METRE, m64};

use crate::{
    inventory::{self, Item, Shape, Stack},
    particles::{ParticleInstance, dither_edge, dither_since},
};

/// The states of a fire: laid and never lit, burning, and burnt down to its ashes.
pub const LAID: u8 = 0;
pub const BURNING: u8 = 1;
pub const BURNT: u8 = 2;
/// Put out before it had burned down, its sticks only half burnt.
pub const OUT: u8 = 3;

/// How far off the small things and the camps are still drawn, in metres.
const SMALL_SIGHT: f64 = 45.0;
const CAMP_SIGHT: f64 = 220.0;
/// The most boxes all the things in view may come to.
const MOST_BOXES: usize = 640;

#[derive(Clone, Debug, PartialEq)]
pub struct Thing {
    /// What it is, all of it, as a pack holds it: taken up, it goes back as it was.
    pub item: Item,
    pub count: u32,
    /// The middle of its foot, where it meets the ground, in world voxels.
    pub pos: DVec3,
    /// Which way it is turned about the vertical, radians.
    pub yaw: f32,
    /// What state it is in; only a fire has more than one.
    pub state: u8,
    /// When it came to that state, in the world's years (see [`Placed::pass`]).
    pub since: f64,
}

impl Thing {
    pub fn new(item: Item, count: u32, pos: DVec3, yaw: f32) -> Self {
        Self { item, count, pos, yaw, state: 0, since: 0.0 }
    }

    pub fn is_fire(&self) -> bool {
        self.item.id == "campfire"
    }

    /// Whether it is a fire, burning.
    pub fn burns(&self) -> bool {
        self.is_fire() && self.state == BURNING
    }

    /// How many sticks a fire not burning gives back when it is taken apart: all of
    /// them from one never lit, those not burnt through from one put out, and none from
    /// ashes.
    pub fn salvage(&self) -> u32 {
        match self.state {
            LAID => inventory::FIRE_STICKS,
            OUT => inventory::FIRE_STICKS - 1,
            _ => 0,
        }
    }

    /// How near, in voxels, the player must be to reach it.
    pub fn reach(&self) -> f64 {
        m64(if self.is_fire() { 2.0 } else { 1.3 })
    }

    /// The heart of a fire, where its flames are.
    pub fn heart(&self) -> DVec3 {
        self.pos + DVec3::new(0.0, m64(0.3), 0.0)
    }

}

/// Which thing, of all those set down: its column, and its place among that column's.
pub type Handle = ((i32, i32), usize);

#[derive(Default)]
pub struct Placed {
    columns: HashMap<(i32, i32), Vec<Thing>>,
    /// How long this world has been walked since the first thing was set down, in its
    /// own years: the clock its fires burn down by.
    years: f64,
    /// Changed since it was last saved.
    pub unsaved: bool,
}

fn column_of(pos: DVec3) -> (i32, i32) {
    ((pos.x.floor() as i32).div_euclid(CHUNK), (pos.z.floor() as i32).div_euclid(CHUNK))
}

impl Placed {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets `thing` down. A fire's state dates from now.
    pub fn put(&mut self, mut thing: Thing) {
        thing.since = self.years;
        self.columns.entry(column_of(thing.pos)).or_default().push(thing);
        self.unsaved = true;
    }

    /// Takes a thing up out of the world.
    pub fn take(&mut self, at: Handle) -> Option<Thing> {
        let column = self.columns.get_mut(&at.0)?;
        if at.1 >= column.len() {
            return None;
        }
        let thing = column.remove(at.1);
        if column.is_empty() {
            self.columns.remove(&at.0);
        }
        self.unsaved = true;
        Some(thing)
    }

    pub fn get(&self, at: Handle) -> Option<&Thing> {
        self.columns.get(&at.0)?.get(at.1)
    }

    /// Puts the thing at `at` into `state`, from now.
    pub fn set_state(&mut self, at: Handle, state: u8) {
        let now = self.years;
        if let Some(thing) = self.columns.get_mut(&at.0).and_then(|c| c.get_mut(at.1)) {
            thing.state = state;
            thing.since = now;
            self.unsaved = true;
        }
    }

    /// Leaves only `count` of the thing at `at`.
    pub fn set_count(&mut self, at: Handle, count: u32) {
        if let Some(thing) = self.columns.get_mut(&at.0).and_then(|c| c.get_mut(at.1)) {
            thing.count = count;
            self.unsaved = true;
        }
    }

    /// Every thing within `radius` voxels of `pos`, across the ground.
    pub fn near(&self, pos: DVec3, radius: f64) -> impl Iterator<Item = (Handle, &Thing)> + '_ {
        let (lo, hi) = (column_of(pos - DVec3::splat(radius)), column_of(pos + DVec3::splat(radius)));
        // A few things in a wide view are quicker gone through than the view's columns.
        // Counted in 64 bits: the columns of a view wide enough to take in everything are
        // more than 32 hold.
        let all = (self.columns.len() as i64) < (hi.0 - lo.0 + 1) as i64 * (hi.1 - lo.1 + 1) as i64;
        let columns: Box<dyn Iterator<Item = (&(i32, i32), &Vec<Thing>)>> = if all {
            Box::new(self.columns.iter().filter(move |(c, _)| (lo.0..=hi.0).contains(&c.0) && (lo.1..=hi.1).contains(&c.1)))
        } else {
            Box::new((lo.1..=hi.1).flat_map(move |cz| (lo.0..=hi.0).map(move |cx| (cx, cz))).filter_map(|c| self.columns.get_key_value(&c)))
        };
        columns
            .flat_map(|(c, things)| things.iter().enumerate().map(move |(i, t)| ((*c, i), t)))
            .filter(move |(_, t)| (t.pos.x - pos.x).hypot(t.pos.z - pos.z) <= radius)
    }

    /// The thing within reach of someone whose hands are at `hands`: the one they are
    /// furthest inside the reach of, for its size, so that a flower laid by a fire can
    /// still be picked out.
    pub fn within_reach(&self, hands: DVec3) -> Option<Handle> {
        self.near(hands, m64(3.0))
            .filter(|(_, t)| (t.pos.y - hands.y).abs() < m64(2.0))
            .map(|(at, t)| (at, (t.pos.x - hands.x).hypot(t.pos.z - hands.z) / t.reach()))
            .filter(|(_, inside)| *inside <= 1.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(at, _)| at)
    }

    /// Moves the world's clock on by `years`, and lets the fires that have burned for
    /// `burn` years or more go out. Whether any did.
    pub fn pass(&mut self, years: f64, burn: f64) -> bool {
        if self.columns.is_empty() {
            return false;
        }
        self.years += years;
        let now = self.years;
        let mut out = false;
        for thing in self.columns.values_mut().flatten() {
            if thing.burns() && now - thing.since >= burn {
                thing.state = BURNT;
                thing.since = now;
                out = true;
            }
        }
        self.unsaved |= years > 0.0;
        out
    }

    /// The fires within `radius` voxels of `pos` that smoke, each with how much: 1 for
    /// one burning, and a little, less and less, for one that has gone out in the last
    /// `smoulder` years.
    pub fn smoking(&self, pos: DVec3, radius: f64, smoulder: f64) -> Vec<(DVec3, f32)> {
        self.near(pos, radius)
            .filter_map(|(_, t)| {
                if t.burns() {
                    return Some((t.heart(), 1.0));
                }
                let cold = (self.years - t.since) / smoulder.max(1e-9);
                (t.is_fire() && matches!(t.state, OUT | BURNT) && cold < 1.0).then(|| (t.heart(), 0.4 * (1.0 - cold as f32)))
            })
            .collect()
    }

    /// Where the player has camped: every fire of theirs that has ever burned.
    pub fn camps(&self) -> Vec<DVec2> {
        self.columns.values().flatten().filter(|t| t.is_fire() && t.state != LAID).map(|t| DVec2::new(t.pos.x, t.pos.z)).collect()
    }

    /// As a save holds them: a line to a thing.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        if self.columns.is_empty() {
            return out;
        }
        out += &format!("years = {}\n", self.years);
        let mut columns: Vec<_> = self.columns.iter().collect();
        columns.sort_by_key(|(c, _)| **c);
        for t in columns.into_iter().flat_map(|(_, things)| things) {
            let stack = inventory::stack_text(&Stack { item: t.item.clone(), count: t.count });
            out += &format!("thing = {:.3},{:.3},{:.3}|{:.3}|{}|{}|{stack}\n", t.pos.x, t.pos.y, t.pos.z, t.yaw, t.state, t.since);
        }
        out
    }

    /// Reads them back, leaving out any line that does not make sense, and any tent
    /// pitched when there were tents (its shape no longer reads).
    pub fn from_text(text: &str) -> Self {
        let mut placed = Self::new();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            let (key, value) = (key.trim(), value.trim());
            if key == "years" {
                placed.years = value.parse::<f64>().ok().filter(|y| y.is_finite() && *y >= 0.0).unwrap_or(0.0);
            } else if key == "thing" {
                let parts: Vec<&str> = value.splitn(5, '|').collect();
                let thing = (|| {
                    let [pos, yaw, state, since, stack] = parts[..] else { return None };
                    let p: Vec<f64> = pos.split(',').filter_map(|v| v.trim().parse().ok()).filter(|v: &f64| v.is_finite()).collect();
                    let [x, y, z] = p[..] else { return None };
                    let stack = inventory::stack_from_text(stack)?;
                    Some(Thing {
                        item: stack.item,
                        count: stack.count,
                        pos: DVec3::new(x, y, z),
                        yaw: yaw.trim().parse::<f32>().ok().filter(|v| v.is_finite())?,
                        state: state.trim().parse::<u8>().ok()?.min(OUT),
                        since: since.trim().parse::<f64>().ok().filter(|v| v.is_finite())?,
                    })
                })();
                if let Some(thing) = thing {
                    placed.columns.entry(column_of(thing.pos)).or_default().push(thing);
                }
            }
        }
        placed.unsaved = false;
        placed
    }

    /// Draws what is near enough to `cam` to be seen. `clock` is seconds, for the flames.
    pub fn draw(&self, cam: DVec3, clock: f32, out: &mut Vec<ParticleInstance>) {
        let limit = out.len() + MOST_BOXES;
        let mut near: Vec<(f64, &Thing)> = self.near(cam, m64(CAMP_SIGHT)).map(|(_, t)| (t.pos.distance(cam), t)).collect();
        // The nearest first, so that it is the far ones that go without if there are too many.
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (d, thing) in near {
            if out.len() >= limit {
                break;
            }
            if thing.is_fire() || d <= m64(SMALL_SIGHT) {
                let from = out.len();
                draw_one(thing, cam, clock, out);
                let sight = if thing.is_fire() { CAMP_SIGHT } else { SMALL_SIGHT };
                dither_since(out, from, dither_edge(d, m64(sight), m64(sight * 0.12)));
            }
        }
    }
}

const WOOD: [f32; 3] = [0.21, 0.14, 0.085];
const CHARRED: [f32; 3] = [0.035, 0.03, 0.028];
const STEM: [f32; 3] = [0.10, 0.25, 0.07];
const LEAF: [f32; 3] = [0.15, 0.33, 0.10];
const CREAM: [f32; 3] = [0.70, 0.64, 0.50];

fn linear(c: [u8; 3]) -> [f32; 3] {
    c.map(|v| (v as f32 / 255.0).powf(2.2))
}

fn shade(c: [f32; 3], by: f32) -> [f32; 3] {
    c.map(|v| v * by)
}

/// A number from 0 to 1 that is always the same for the same `k`.
fn chance(k: u32) -> f32 {
    worldgen::noise::unit(worldgen::noise::hash3(0x51ab, k as i32, 7, 3))
}

/// Lays the boxes of one model: each is given in metres in the model's own frame, which
/// stands at `base` (from the camera, in voxels) turned `yaw` about the vertical.
struct Brush<'a> {
    out: &'a mut Vec<ParticleInstance>,
    base: Vec3,
    yaw: f32,
}

impl Brush<'_> {
    /// A box with its middle at `at`, turned `turn` about the vertical within the model
    /// and tilted (pitched about its own x, rolled about its own z) before that.
    fn tilted(&mut self, at: [f32; 3], size: [f32; 3], color: [f32; 3], turn: f32, tilt: [f32; 2], glow: f32) {
        let (s, c) = self.yaw.sin_cos();
        let p = Vec3::from(at) * VOXELS_PER_METRE;
        let turned = Vec3::new(p.x * c - p.z * s, p.y, p.x * s + p.z * c);
        self.out.push(ParticleInstance {
            pos: (self.base + turned).into(),
            yaw: self.yaw + turn,
            size: (Vec3::from(size) * VOXELS_PER_METRE).into(),
            glow,
            color: [color[0], color[1], color[2], 1.0],
            tilt,
            dissolve: 0.0,
        });
    }

    fn piece(&mut self, at: [f32; 3], size: [f32; 3], color: [f32; 3]) {
        self.tilted(at, size, color, 0.0, [0.0; 2], 0.0);
    }

    /// The same brush, moved `by` metres across the model's ground and turned `turn`.
    fn moved(&mut self, by: [f32; 2], turn: f32) -> Brush<'_> {
        let (s, c) = self.yaw.sin_cos();
        let p = Vec3::new(by[0], 0.0, by[1]) * VOXELS_PER_METRE;
        Brush { base: self.base + Vec3::new(p.x * c - p.z * s, 0.0, p.x * s + p.z * c), yaw: self.yaw + turn, out: self.out }
    }
}

/// Draws one thing, wherever it is.
pub fn draw_one(thing: &Thing, cam: DVec3, clock: f32, out: &mut Vec<ParticleInstance>) {
    let mut brush = Brush { out, base: (thing.pos - cam).as_vec3(), yaw: thing.yaw };
    if thing.is_fire() {
        fire(&mut brush, thing, clock);
    } else {
        // Up to three of a kind are shown, fanned out where they lie.
        let n = thing.count.clamp(1, 3);
        for k in 0..n {
            let fan = k as f32 - (n - 1) as f32 * 0.5;
            let mut one = brush.moved([fan * 0.09, k as f32 * 0.03], fan * 0.6);
            small(&mut one, &thing.item);
        }
    }
}

/// A fire in its ring of stones: laid ready, burning, put out, or burnt down to ash.
fn fire(b: &mut Brush, thing: &Thing, clock: f32) {
    let salt = (thing.pos.x.floor() as i32 as u32).wrapping_mul(31) ^ (thing.pos.z.floor() as i32 as u32).wrapping_mul(17);
    // The ring: nine stones, no two alike.
    for k in 0..9u32 {
        let (r1, r2, r3) = (chance(salt ^ (k * 3)), chance(salt ^ (k * 3 + 1)), chance(salt ^ (k * 3 + 2)));
        let a = (k as f32 + 0.3 * r1) * std::f32::consts::TAU / 9.0;
        let tall = 0.11 + 0.07 * r2;
        let grey = shade([0.30, 0.29, 0.27], 0.75 + 0.45 * r3);
        b.tilted([0.44 * a.cos(), tall / 2.0, 0.44 * a.sin()], [0.2 + 0.06 * r3, tall, 0.15 + 0.05 * r1], grey, a * 1.7 + r2, [0.0; 2], 0.0);
    }
    let burning = thing.state == BURNING;
    // The bed: bare earth, embers, wet black char, or ash.
    let bed = match thing.state {
        LAID => [0.09, 0.07, 0.05],
        BURNING => [0.05, 0.04, 0.035],
        OUT => [0.03, 0.028, 0.026],
        _ => [0.20, 0.19, 0.18],
    };
    b.tilted([0.0, 0.02, 0.0], [0.6, 0.04, 0.6], bed, 0.4, [0.0; 2], 0.0);
    if thing.state == BURNT {
        // What is left of the sticks, fallen in.
        for k in 0..3 {
            let a = k as f32 * 2.2 + 0.4;
            b.tilted([0.07 * a.cos(), 0.06, 0.07 * a.sin()], [0.06, 0.05, 0.36], CHARRED, a, [0.0; 2], 0.0);
        }
        return;
    }
    // Three sticks stood together over a little kindling; put out, still standing but
    // blackened, the kindling gone to char.
    let wood = match thing.state {
        BURNING => shade(WOOD, 0.35),
        OUT => shade(WOOD, 0.2),
        _ => WOOD,
    };
    let (long, lean) = (0.52f32, 0.72f32);
    for k in 0..3 {
        let a = k as f32 * std::f32::consts::TAU / 3.0 + 0.3;
        let out = 0.5 * long * lean.sin();
        b.tilted([out * a.cos(), 0.5 * long * lean.cos() + 0.04, out * a.sin()], [0.07, long, 0.07], wood, a + std::f32::consts::FRAC_PI_2, [lean, 0.0], 0.0);
    }
    for (turn, y) in [(0.5, 0.06), (2.0, 0.1)] {
        b.tilted([0.0, y, 0.0], [0.05, 0.05, 0.42], shade(wood, 0.85), turn, [0.0; 2], 0.0);
    }
    if !burning {
        return;
    }
    // Embers that breathe, and tongues of flame that leap and fall back.
    let breath = 0.5 + 0.5 * (clock * 2.3).sin();
    b.tilted([0.0, 0.07, 0.0], [0.36, 0.05, 0.36], [1.0, 0.26, 0.04], 0.9, [0.0; 2], 1.0 + 0.5 * breath);
    for k in 0..6 {
        let f = k as f32;
        let leap = 0.5 + 0.5 * ((clock * (9.0 + f * 1.3) + f * 2.1).sin() * 0.6 + (clock * (17.0 + f) + f).sin() * 0.4);
        let (a, r) = (f * std::f32::consts::TAU / 5.0, if k == 5 { 0.0 } else { 0.1 });
        let tall = if k == 5 { 0.34 + 0.3 * leap } else { 0.16 + 0.24 * leap };
        let (color, glow) = if k == 5 { ([1.0, 0.50, 0.10], 1.6) } else { ([1.0, 0.26, 0.03], 1.25) };
        b.tilted([r * a.cos(), 0.09 + tall / 2.0, r * a.sin()], [0.1, tall, 0.1], color, clock * 0.8 + f, [0.0; 2], glow + 0.5 * leap);
    }
}

/// A small thing lying where it was put: a flower on its stem, a mushroom, a stick.
fn small(b: &mut Brush, item: &Item) {
    let c = linear(item.color);
    match item.shape {
        Shape::Flower => {
            b.piece([0.0, 0.03, -0.05], [0.04, 0.04, 0.3], STEM);
            b.piece([0.0, 0.04, 0.14], [0.16, 0.05, 0.16], c);
            b.piece([0.0, 0.07, 0.14], [0.06, 0.04, 0.06], [0.83, 0.67, 0.11]);
        }
        Shape::Spike => {
            b.piece([0.0, 0.03, -0.1], [0.04, 0.04, 0.26], STEM);
            b.piece([0.0, 0.045, 0.14], [0.09, 0.09, 0.26], c);
            b.piece([0.05, 0.03, -0.12], [0.1, 0.03, 0.06], LEAF);
        }
        Shape::Sprig => {
            b.piece([0.0, 0.03, 0.0], [0.04, 0.04, 0.34], STEM);
            for (x, z) in [(0.04, 0.14), (-0.04, 0.06), (0.03, -0.02)] {
                b.piece([x, 0.05, z], [0.08, 0.07, 0.08], c);
            }
        }
        Shape::Berries => {
            b.piece([0.0, 0.02, -0.09], [0.14, 0.03, 0.09], LEAF);
            for (x, y, z) in [(-0.05, 0.04, 0.0), (0.05, 0.04, 0.01), (0.0, 0.045, 0.08), (0.0, 0.1, 0.03)] {
                b.piece([x, y, z], [0.08, 0.08, 0.08], c);
            }
        }
        Shape::Mushroom => {
            b.piece([0.0, 0.07, 0.0], [0.07, 0.14, 0.07], CREAM);
            b.piece([0.0, 0.17, 0.0], [0.2, 0.07, 0.2], c);
            b.piece([0.0, 0.215, 0.0], [0.12, 0.03, 0.12], shade(c, 1.1));
        }
        Shape::Reed => {
            b.piece([0.0, 0.03, -0.12], [0.04, 0.04, 0.46], STEM);
            b.piece([0.0, 0.04, 0.16], [0.08, 0.08, 0.22], c);
        }
        Shape::Stick => {
            b.piece([0.0, 0.035, 0.0], [0.06, 0.06, 0.56], c);
            b.piece([0.05, 0.035, 0.12], [0.12, 0.04, 0.04], shade(c, 0.8));
        }
        // Not things that lie about, but a save may say anything: a bundle.
        Shape::Fire | Shape::Lantern => b.piece([0.0, 0.1, 0.0], [0.3, 0.2, 0.5], c),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flower(n: u32, x: f64, z: f64) -> Thing {
        Thing::new(Item::new("poppy", "Poppy", "Scarlet silk.", [222, 70, 64], Shape::Flower), n, DVec3::new(x, 12.0, z), 0.5)
    }

    fn fire(x: f64, z: f64) -> Thing {
        Thing::new(inventory::campfire(), 1, DVec3::new(x, 12.0, z), 0.0)
    }

    #[test]
    fn things_are_found_where_they_were_put_and_taken_up_again() {
        let mut placed = Placed::new();
        placed.put(flower(3, 10.5, -4.5));
        placed.put(fire(14.0, -4.0));
        placed.put(flower(1, 900.0, 900.0));
        assert_eq!(placed.near(DVec3::new(10.0, 12.0, -4.0), 20.0).count(), 2);
        assert_eq!(placed.near(DVec3::new(10.0, 12.0, -4.0), 2000.0).count(), 3);
        // The nearer of two within reach is the one to hand; out of reach, nothing is.
        let at = placed.within_reach(DVec3::new(10.0, 12.0, -4.0)).expect("the flower is within reach");
        assert_eq!(placed.get(at).map(|t| t.item.id.as_str()), Some("poppy"));
        let at = placed.within_reach(DVec3::new(15.0, 12.0, -4.0)).expect("the fire is within reach");
        assert!(placed.get(at).is_some_and(Thing::is_fire));
        assert!(placed.within_reach(DVec3::new(40.0, 12.0, -4.0)).is_none());
        assert!(placed.within_reach(DVec3::new(10.0, 40.0, -4.0)).is_none(), "it is far below");
        let taken = placed.take(at).expect("it was there");
        assert!(taken.is_fire() && placed.near(DVec3::new(10.0, 12.0, -4.0), 20.0).count() == 1);
        assert!(placed.take(((99, 99), 0)).is_none());
    }

    #[test]
    fn a_fire_burns_for_its_time_and_a_camp_is_remembered() {
        let mut placed = Placed::new();
        placed.put(fire(1.0, 2.0));
        placed.put(fire(50.0, 2.0));
        let at = placed.within_reach(DVec3::new(1.0, 12.0, 2.0)).unwrap();
        assert!(placed.camps().is_empty(), "a fire never lit is no camp");
        assert!(!placed.pass(0.5, 0.01), "a laid fire does not burn down");
        placed.set_state(at, BURNING);
        assert!(placed.get(at).is_some_and(Thing::burns));
        assert!(!placed.pass(0.009, 0.01));
        assert!(placed.pass(0.002, 0.01));
        assert_eq!(placed.get(at).map(|t| t.state), Some(BURNT));
        assert_eq!(placed.camps(), vec![DVec2::new(1.0, 2.0)]);
    }

    #[test]
    fn a_fire_put_out_smoulders_a_while_and_gives_back_what_is_left_of_it() {
        let mut placed = Placed::new();
        placed.put(fire(1.0, 2.0));
        let at = placed.within_reach(DVec3::new(1.0, 12.0, 2.0)).unwrap();
        assert_eq!(placed.get(at).map(Thing::salvage), Some(inventory::FIRE_STICKS));
        assert!(placed.smoking(DVec3::ZERO, 100.0, 0.01).is_empty(), "a laid fire does not smoke");
        placed.set_state(at, BURNING);
        assert_eq!(placed.smoking(DVec3::ZERO, 100.0, 0.01)[0].1, 1.0);
        placed.pass(0.001, 1.0);
        placed.set_state(at, OUT);
        assert!(!placed.get(at).unwrap().burns() && placed.camps().len() == 1);
        assert_eq!(placed.get(at).map(Thing::salvage), Some(inventory::FIRE_STICKS - 1));
        let wisp = placed.smoking(DVec3::ZERO, 100.0, 0.01)[0].1;
        assert!(wisp > 0.0 && wisp < 1.0);
        // It is still put out when it is read back, and in time it is cold.
        assert_eq!(Placed::from_text(&placed.to_text()).get(at).map(|t| t.state), Some(OUT));
        placed.pass(0.02, 1.0);
        assert!(placed.smoking(DVec3::ZERO, 100.0, 0.01).is_empty());
        placed.set_state(at, BURNT);
        assert_eq!(placed.get(at).map(Thing::salvage), Some(0));
    }

    #[test]
    fn what_is_set_down_reads_back_as_it_was_saved_and_shrugs_off_nonsense() {
        let mut placed = Placed::new();
        placed.put(flower(7, 10.25, -4.5));
        placed.put(fire(-300.0, 77.125));
        placed.put(Thing::new(inventory::sticks(), 2, DVec3::new(-296.0, 12.0, 80.0), 2.5));
        placed.pass(0.25, 1.0);
        let at = placed.within_reach(DVec3::new(-300.0, 12.0, 77.0)).unwrap();
        placed.set_state(at, BURNING);
        let text = placed.to_text();
        // A tent pitched when there were tents is quietly left out.
        let tent = "thing = -296.000,12.000,80.000|2.500|0|0|1|tent|204,182,136|tent|Tent|Canvas, two poles and a bedroll.\n";
        let back = Placed::from_text(&format!("{text}thing = 1,2|0|0|0|1|flower|1,2,3|x|X|\nthing = 1,2,3|0|9|0|0|flower|1,2,3|x|X|\nrubbish\n{tent}"));
        assert!(!back.unsaved);
        assert_eq!(back.to_text(), text);
        let all: Vec<&Thing> = back.near(DVec3::ZERO, 1e6).map(|(_, t)| t).collect();
        assert_eq!(all.len(), 3);
        assert!(all.iter().any(|t| t.item.id == "poppy" && t.count == 7 && t.yaw == 0.5));
        assert!(all.iter().any(|t| t.burns() && t.since == 0.25));
        assert!(Placed::from_text("").to_text().is_empty());
    }

    #[test]
    fn every_kind_of_thing_has_a_model_and_only_a_burning_fire_glows() {
        let cam = DVec3::new(0.0, 14.0, 0.0);
        let mut shapes = vec![inventory::sticks(), inventory::campfire()];
        for (shape, id) in [(Shape::Flower, "a"), (Shape::Spike, "b"), (Shape::Sprig, "c"), (Shape::Berries, "d"), (Shape::Mushroom, "e"), (Shape::Reed, "f")] {
            shapes.push(Item::new(id, "A thing", "About it.", [200, 100, 50], shape));
        }
        for item in shapes {
            let mut placed = Placed::new();
            placed.put(Thing::new(item.clone(), 2, DVec3::new(3.0, 12.0, 3.0), 1.0));
            let mut out = Vec::new();
            placed.draw(cam, 1.0, &mut out);
            assert!(!out.is_empty(), "{}", item.id);
            assert!(out.iter().all(|b| b.glow == 0.0 && b.size.iter().all(|s| *s > 0.0)), "{}", item.id);
            // Nothing of it is under the ground it stands on, or far from where it was put.
            assert!(out.iter().all(|b| b.pos[1] > -2.0 - 0.01 && Vec3::from(b.pos).distance(Vec3::new(3.0, -2.0, 3.0)) < 6.0), "{}", item.id);
        }
        let mut placed = Placed::new();
        placed.put(fire(3.0, 3.0));
        let at = placed.within_reach(DVec3::new(3.0, 12.0, 3.0)).unwrap();
        placed.set_state(at, BURNING);
        let (mut lit, mut far) = (Vec::new(), Vec::new());
        placed.draw(cam, 1.0, &mut lit);
        assert!(lit.iter().any(|b| b.glow > 2.0));
        // A small thing far off is not drawn; a camp still is.
        placed.put(flower(1, 300.0, 3.0));
        placed.draw(DVec3::new(300.0 + m64(60.0), 14.0, 3.0), 1.0, &mut far);
        assert!(!far.is_empty() && far.len() == lit.len());
    }
}
