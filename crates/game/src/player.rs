//! First-person character: walking, stepping, jumping and swimming over voxel terrain.

use std::{collections::HashMap, sync::Arc};

use glam::{DVec3, Vec3};
use voxel::{CHUNK, Kind, PaddedChunk};
use worldgen::{World, world::GameWorld};

/// Answers "what is at this voxel?" for collision.
pub trait Voxels {
    fn kind(&mut self, x: i32, y: i32, z: i32) -> Kind;
}

/// Character size in voxels. Roughly Cube World's proportions: a little over two
/// terrain voxels tall.
pub const HEIGHT: f64 = 2.2;
pub const EYE_HEIGHT: f64 = 1.9;
const HALF_WIDTH: f64 = 0.32;

const WALK_SPEED: f32 = 5.2;
const SPRINT_SPEED: f32 = 8.6;
const SWIM_SPEED: f32 = 3.4;
const JUMP_SPEED: f32 = 8.8;
const GRAVITY: f32 = 27.0;
/// Ledges up to this high are walked up without jumping.
const STEP: f64 = 1.0;
/// Out of water a swimmer can haul up a taller bank.
const CLIMB_OUT: f64 = 2.0;
const SKIN: f64 = 1e-4;

/// Airspeed the glider settles at when flown level.
const GLIDE_TRIM: f32 = 13.0;
/// Sink rate at trim: with [`GLIDE_TRIM`] this is about 8 voxels forward per voxel down.
const GLIDE_SINK: f32 = 1.6;
/// How strongly diving gains speed and climbing loses it.
const GLIDE_GRAVITY: f32 = 14.0;
const GLIDE_DRAG: f32 = 0.55;
const GLIDE_MIN_SPEED: f32 = 5.0;
const GLIDE_MAX_SPEED: f32 = 34.0;
/// Steepest the glider follows the view, down and up, in radians.
const GLIDE_DIVE: f32 = 0.85;
const GLIDE_CLIMB: f32 = 0.30;

const BOAT_SPEED: f32 = 9.0;
const BOAT_FAST: f32 = 22.0;
/// From the middle of the hull to the bow, for running aground.
const BOAT_REACH: f64 = 1.6;
/// Sitting in a boat, the eye is this far above the water.
const BOAT_EYE: f64 = 1.5;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Walk,
    Glide,
    Boat,
}

#[derive(Default, Clone, Copy)]
pub struct Controls {
    /// Desired horizontal direction in world space (y ignored), zero if standing still.
    pub wish: Vec3,
    pub jump: bool,
    pub sprint: bool,
    /// Pressed this frame: open the glider if in the air, or fold it away.
    pub deploy: bool,
    /// Where the camera is looking; the glider follows it.
    pub look: Vec3,
    /// How fast the view is turning, radians per second.
    pub turn: f32,
}

pub struct Player {
    /// Centre of the feet.
    pub pos: DVec3,
    pub vel: Vec3,
    pub on_ground: bool,
    pub swimming: bool,
    /// How far the eye currently lags below its true height after stepping up, so
    /// steps read as a smooth rise rather than a jolt.
    step_lag: f32,
    pub mode: Mode,
    airspeed: f32,
    /// How far the glider is leaning into a turn, radians.
    pub bank: f32,
    /// Direction a boat's bow points, radians.
    pub heading: f32,
    /// Advances while afloat, for the rocking of the boat.
    swell: f32,
}

impl Player {
    pub fn new(pos: DVec3) -> Self {
        Self {
            pos,
            vel: Vec3::ZERO,
            on_ground: false,
            swimming: false,
            step_lag: 0.0,
            mode: Mode::Walk,
            airspeed: 0.0,
            bank: 0.0,
            heading: 0.0,
            swell: 0.0,
        }
    }

    pub fn eye(&self) -> DVec3 {
        if self.mode == Mode::Boat {
            let rock = (self.swell * 1.1).sin() as f64 * 0.06;
            return self.pos + DVec3::new(0.0, BOAT_EYE + rock, 0.0);
        }
        self.pos + DVec3::new(0.0, EYE_HEIGHT - self.step_lag as f64, 0.0)
    }

    /// Steps aboard a boat floating at `pos`.
    pub fn board(&mut self, pos: DVec3, heading: f32) {
        self.mode = Mode::Boat;
        self.pos = pos;
        self.vel = Vec3::ZERO;
        self.heading = heading;
        self.on_ground = false;
        self.swimming = false;
    }

    /// Steps out of the boat, onto dry land if any is within reach and otherwise into
    /// the water. Returns where the boat was left, and its heading.
    pub fn leave_boat(&mut self, v: &mut impl Voxels) -> (DVec3, f32) {
        let boat = (self.pos, self.heading);
        self.mode = Mode::Walk;
        self.vel = Vec3::ZERO;
        for radius in [1.5f64, 2.5, 3.5, 4.5] {
            for k in 0..16 {
                let a = k as f64 / 16.0 * std::f64::consts::TAU;
                let spot = DVec3::new(self.pos.x + a.cos() * radius, 0.0, self.pos.z + a.sin() * radius);
                let (x, z) = (spot.x.floor() as i32, spot.z.floor() as i32);
                let base = self.pos.y.round() as i32;
                for y in (base - 2..=base + 3).rev() {
                    let clear = |dy: i32, v: &mut dyn FnMut(i32) -> Kind| v(dy) == Kind::Air;
                    let mut kind = |dy: i32| v.kind(x, y + dy, z);
                    if kind(-1) == Kind::Solid && clear(0, &mut kind) && clear(1, &mut kind) && clear(2, &mut kind) {
                        self.pos = DVec3::new(x as f64 + 0.5, y as f64, z as f64 + 0.5);
                        return boat;
                    }
                }
            }
        }
        boat
    }

    /// Height of the water plane at a column, searching down from about `around`.
    /// `None` if the first thing below is solid ground, or the water is out of reach.
    fn water_plane(v: &mut impl Voxels, p: DVec3, around: f64) -> Option<f64> {
        let (x, z) = (p.x.floor() as i32, p.z.floor() as i32);
        let mut top = around.round() as i32;
        // Starting inside a taller body of water, rise to its real surface first.
        while v.kind(x, top, z) == Kind::Water && top < around.round() as i32 + 64 {
            top += 1;
        }
        for y in (top - 30..=top).rev() {
            match v.kind(x, y, z) {
                Kind::Water => return Some(y as f64 + 1.0),
                Kind::Solid => return None,
                _ => {}
            }
        }
        None
    }

    fn sail(&mut self, v: &mut impl Voxels, controls: Controls, dt: f32) {
        self.swell += dt;
        let wish = Vec3::new(controls.wish.x, 0.0, controls.wish.z).normalize_or_zero();
        let speed = if controls.sprint { BOAT_FAST } else { BOAT_SPEED };
        // A boat answers slowly and carries its way.
        let blend = 1.0 - (-1.2 * dt).exp();
        self.vel.x += (wish.x * speed - self.vel.x) * blend;
        self.vel.z += (wish.z * speed - self.vel.z) * blend;
        self.vel.y = 0.0;
        if self.vel.length_squared() > 0.09 {
            let want = self.vel.z.atan2(self.vel.x);
            let turn = (want - self.heading + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
            self.heading += turn * (1.0 - (-2.5 * dt).exp());
        }
        // One axis at a time, so a boat slides along a shore instead of stopping dead.
        let plane = self.pos.y;
        for axis in [0usize, 2] {
            let delta = (self.vel[axis] * dt) as f64;
            if delta == 0.0 {
                continue;
            }
            let mut next = self.pos;
            next[axis] += delta;
            let mut bow = next;
            bow[axis] += delta.signum() * BOAT_REACH;
            // Water can be followed down any drop, but only up a low step.
            if Self::water_plane(v, bow, plane).is_some_and(|w| w <= plane + 1.01) {
                self.pos = next;
            } else {
                self.vel[axis] = 0.0;
            }
        }
        if let Some(w) = Self::water_plane(v, self.pos, plane) {
            self.pos.y += (w - self.pos.y) * (1.0 - ((-6.0 * dt) as f64).exp());
        }
    }

    /// Glider flight: a calm exchange of height for speed. Looking down dives and gains
    /// speed; looking up spends it. Lift only ever comes out of speed, so however it is
    /// flown the glider comes down in the end.
    fn glide(&mut self, controls: Controls, dt: f32) {
        let look = controls.look.normalize_or_zero();
        let pitch = look.y.asin().clamp(-GLIDE_DIVE, GLIDE_CLIMB);
        let heading = Vec3::new(look.x, 0.0, look.z).normalize_or_zero();
        let accel = -GLIDE_GRAVITY * pitch.sin() - (self.airspeed - GLIDE_TRIM) * GLIDE_DRAG;
        self.airspeed = (self.airspeed + accel * dt).clamp(GLIDE_MIN_SPEED, GLIDE_MAX_SPEED);
        // Below trim the wing starts to lose its grip and sink rate climbs quickly.
        let slow = ((GLIDE_TRIM - self.airspeed) / 3.0).max(0.0);
        let sink = GLIDE_SINK + slow * slow;
        let target = (heading * pitch.cos() + Vec3::Y * pitch.sin()) * self.airspeed - Vec3::Y * sink;
        self.vel += (target - self.vel) * (1.0 - (-3.0 * dt).exp());
    }

    fn blocked(pos: DVec3, v: &mut impl Voxels) -> bool {
        let lo = (pos - DVec3::new(HALF_WIDTH, 0.0, HALF_WIDTH)).floor().as_ivec3();
        let hi = (pos + DVec3::new(HALF_WIDTH - SKIN, HEIGHT - SKIN, HALF_WIDTH - SKIN))
            .floor()
            .as_ivec3();
        for y in lo.y..=hi.y {
            for z in lo.z..=hi.z {
                for x in lo.x..=hi.x {
                    if v.kind(x, y, z) == Kind::Solid {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Moves along one axis as far as possible. Returns true if something was hit.
    fn slide(&mut self, axis: usize, delta: f64, v: &mut impl Voxels) -> bool {
        let mut step = DVec3::ZERO;
        step[axis] = delta;
        if !Self::blocked(self.pos + step, v) {
            self.pos += step;
            return false;
        }
        // Close in on the obstacle by bisection.
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..10 {
            let mid = (lo + hi) * 0.5;
            if Self::blocked(self.pos + step * mid, v) {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        self.pos += step * lo;
        true
    }

    /// Horizontal move on one axis, walking up low ledges.
    fn walk(&mut self, axis: usize, delta: f64, v: &mut impl Voxels) {
        if delta == 0.0 {
            return;
        }
        let before = self.pos;
        if !self.slide(axis, delta, v) {
            return;
        }
        let max_rise = if self.swimming {
            CLIMB_OUT
        } else if self.on_ground {
            STEP
        } else {
            return;
        };
        // Try the same move from higher up, lowest rise first.
        let mut rise = 0.5;
        while rise <= max_rise + 1e-9 {
            let lifted = before + DVec3::new(0.0, rise + 2.0 * SKIN, 0.0);
            let mut moved = lifted;
            moved[axis] += delta;
            if !Self::blocked(lifted, v) && !Self::blocked(moved, v) {
                self.pos = moved;
                self.step_lag = (self.step_lag + rise as f32).min(1.5);
                if self.swimming {
                    self.vel.y = self.vel.y.max(0.0);
                }
                return;
            }
            rise += 0.5;
        }
        self.vel[axis] = 0.0;
    }

    pub fn update(&mut self, v: &mut impl Voxels, controls: Controls, dt: f32) {
        if self.mode == Mode::Boat {
            self.sail(v, controls, dt);
            return;
        }
        // Chest-deep in water means swimming.
        let chest = (self.pos + DVec3::new(0.0, 1.2, 0.0)).floor().as_ivec3();
        self.swimming = v.kind(chest.x, chest.y, chest.z) == Kind::Water;
        let head = (self.pos + DVec3::new(0.0, EYE_HEIGHT, 0.0)).floor().as_ivec3();
        let head_under = v.kind(head.x, head.y, head.z) == Kind::Water;

        if controls.deploy {
            if self.mode == Mode::Glide {
                self.mode = Mode::Walk;
            } else if !self.on_ground && !self.swimming {
                self.mode = Mode::Glide;
                self.airspeed = Vec3::new(self.vel.x, 0.0, self.vel.z).length().max(9.0);
            }
        }
        if self.swimming {
            self.mode = Mode::Walk;
        }
        let gliding = self.mode == Mode::Glide;
        // Lean into turns; the camera rolls with it.
        let lean = if gliding { (controls.turn * 0.35).clamp(-0.5, 0.5) } else { 0.0 };
        self.bank += (lean - self.bank) * (1.0 - (-4.0 * dt).exp());

        let wish = Vec3::new(controls.wish.x, 0.0, controls.wish.z).normalize_or_zero();
        let speed = if self.swimming {
            SWIM_SPEED
        } else if controls.sprint {
            SPRINT_SPEED
        } else {
            WALK_SPEED
        };
        // Quick to respond on the ground, slower in air and water.
        let response = if self.swimming {
            4.0
        } else if self.on_ground {
            14.0
        } else {
            3.0
        };
        let blend = 1.0 - (-response * dt).exp();
        if !gliding {
            self.vel.x += (wish.x * speed - self.vel.x) * blend;
            self.vel.z += (wish.z * speed - self.vel.z) * blend;
        }

        if gliding {
            self.glide(controls, dt);
        } else if self.swimming {
            // Float with the head just clear of the surface; Space swims upward.
            let target = if controls.jump {
                3.6
            } else if head_under {
                1.2
            } else {
                -1.0
            };
            self.vel.y += (target - self.vel.y) * (1.0 - (-5.0 * dt).exp());
        } else {
            if controls.jump && self.on_ground {
                self.vel.y = JUMP_SPEED;
            }
            self.vel.y = (self.vel.y - GRAVITY * dt).max(-60.0);
        }

        // Small sub-steps so fast movement cannot pass through a one-voxel wall.
        let travel = (self.vel * dt).as_dvec3();
        let steps = (travel.abs().max_element() / 0.4).ceil().max(1.0) as u32;
        let part = travel / steps as f64;
        for _ in 0..steps {
            self.walk(0, part.x, v);
            self.walk(2, part.z, v);
            let falling = part.y < 0.0;
            if self.slide(1, part.y, v) {
                self.on_ground = falling;
                self.vel.y = 0.0;
            } else if part.y != 0.0 {
                self.on_ground = false;
            }
        }
        if self.on_ground && self.mode == Mode::Glide {
            self.mode = Mode::Walk;
        }
        self.step_lag *= (-12.0 * dt).exp();
    }
}

/// Voxel lookups for collision, generated on demand around the player.
///
/// Collision needs only the handful of chunk columns the player is touching, and a
/// column takes well under a millisecond to generate, so this regenerates them rather
/// than keeping voxel data for every chunk that has been meshed.
pub struct WorldVoxels {
    world: Arc<GameWorld>,
    columns: HashMap<(i32, i32), Vec<(i32, PaddedChunk)>>,
}

impl WorldVoxels {
    pub fn new(world: Arc<GameWorld>) -> Self {
        Self {
            world,
            columns: HashMap::new(),
        }
    }

    /// Drops columns far from `pos`. Call occasionally.
    pub fn trim(&mut self, pos: DVec3) {
        if self.columns.len() > 64 {
            let (cx, cz) = (
                (pos.x / CHUNK as f64).floor() as i32,
                (pos.z / CHUNK as f64).floor() as i32,
            );
            self.columns
                .retain(|&(x, z), _| (x - cx).abs() <= 2 && (z - cz).abs() <= 2);
        }
    }
}

impl WorldVoxels {
    /// The voxel id at a world position (air outside generated chunks).
    pub fn voxel(&mut self, x: i32, y: i32, z: i32) -> voxel::Voxel {
        let col = (x.div_euclid(CHUNK), z.div_euclid(CHUNK));
        let world = &self.world;
        let chunks = self
            .columns
            .entry(col)
            .or_insert_with(|| world.generate_column(col.0, col.1).chunks);
        let cy = y.div_euclid(CHUNK);
        match chunks.iter().find(|(c, _)| *c == cy) {
            Some((_, chunk)) => chunk.get(
                x.rem_euclid(CHUNK) as usize + 1,
                y.rem_euclid(CHUNK) as usize + 1,
                z.rem_euclid(CHUNK) as usize + 1,
            ),
            None if chunks.first().is_some_and(|(lowest, _)| cy < *lowest) => worldgen::materials::STONE,
            None => voxel::AIR,
        }
    }
}

impl Voxels for WorldVoxels {
    fn kind(&mut self, x: i32, y: i32, z: i32) -> Kind {
        let col = (x.div_euclid(CHUNK), z.div_euclid(CHUNK));
        let world = &self.world;
        let chunks = self
            .columns
            .entry(col)
            .or_insert_with(|| world.generate_column(col.0, col.1).chunks);
        let cy = y.div_euclid(CHUNK);
        match chunks.iter().find(|(c, _)| *c == cy) {
            Some((_, chunk)) => {
                let v = chunk.get(
                    x.rem_euclid(CHUNK) as usize + 1,
                    y.rem_euclid(CHUNK) as usize + 1,
                    z.rem_euclid(CHUNK) as usize + 1,
                );
                world.materials().get(v).kind
            }
            // Columns hold every chunk from just under the ground to the top of
            // anything on it: outside that range it is rock below and air above.
            None if chunks.first().is_some_and(|(lowest, _)| cy < *lowest) => Kind::Solid,
            None => Kind::Air,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test world described by a function.
    struct Fake<F: Fn(i32, i32, i32) -> Kind>(F);

    impl<F: Fn(i32, i32, i32) -> Kind> Voxels for Fake<F> {
        fn kind(&mut self, x: i32, y: i32, z: i32) -> Kind {
            (self.0)(x, y, z)
        }
    }

    fn run(p: &mut Player, v: &mut impl Voxels, c: Controls, seconds: f32) {
        for _ in 0..(seconds * 120.0) as u32 {
            p.update(v, c, 1.0 / 120.0);
        }
    }

    fn forward() -> Controls {
        Controls {
            wish: Vec3::X,
            ..Default::default()
        }
    }

    #[test]
    fn falls_and_lands_on_the_floor() {
        // Solid at y < 10, so the floor surface is y = 10.
        let mut world = Fake(|_, y, _| if y < 10 { Kind::Solid } else { Kind::Air });
        let mut p = Player::new(DVec3::new(0.5, 20.0, 0.5));
        run(&mut p, &mut world, Controls::default(), 3.0);
        assert!(p.on_ground);
        assert!((p.pos.y - 10.0).abs() < 1e-3, "resting at {}", p.pos.y);
    }

    #[test]
    fn walks_at_walking_speed_and_sprints_faster() {
        let mut world = Fake(|_, y, _| if y < 10 { Kind::Solid } else { Kind::Air });
        let mut p = Player::new(DVec3::new(0.5, 10.0, 0.5));
        run(&mut p, &mut world, forward(), 4.0);
        let walked = p.pos.x;
        assert!((walked - 4.0 * WALK_SPEED as f64).abs() < 1.5, "walked {walked}");

        let mut p = Player::new(DVec3::new(0.5, 10.0, 0.5));
        let sprint = Controls {
            sprint: true,
            ..forward()
        };
        run(&mut p, &mut world, sprint, 4.0);
        assert!(p.pos.x > walked * 1.4);
    }

    #[test]
    fn steps_up_one_voxel_but_not_two() {
        // Floor rises by `rise` voxels at x >= 5.
        for (rise, should_climb) in [(1, true), (2, false)] {
            let mut world = Fake(move |x, y, _| {
                let top = if x >= 5 { 10 + rise } else { 10 };
                if y < top { Kind::Solid } else { Kind::Air }
            });
            let mut p = Player::new(DVec3::new(0.5, 10.0, 0.5));
            run(&mut p, &mut world, forward(), 3.0);
            if should_climb {
                assert!(p.pos.x > 8.0, "did not climb a {rise}-voxel step");
                assert!((p.pos.y - 11.0).abs() < 1e-2);
            } else {
                assert!(p.pos.x < 5.0, "walked through a {rise}-voxel wall");
                assert!((p.pos.y - 10.0).abs() < 1e-2);
            }
        }
    }

    #[test]
    fn jumping_clears_a_two_voxel_wall_only_with_a_jump() {
        let mut world = Fake(|x, y, _| {
            let top = if x >= 5 { 11 } else { 10 };
            if y < top { Kind::Solid } else { Kind::Air }
        });
        // Jump height is v^2 / 2g: enough for one voxel, not two.
        let height = JUMP_SPEED * JUMP_SPEED / (2.0 * GRAVITY);
        assert!(height > 1.1 && height < 2.0, "jump height {height}");
        let mut p = Player::new(DVec3::new(0.5, 10.0, 0.5));
        let jump = Controls {
            jump: true,
            ..forward()
        };
        run(&mut p, &mut world, jump, 3.0);
        assert!(p.pos.x > 6.0);
    }

    #[test]
    fn never_ends_up_inside_solid() {
        // Bumpy ground and a wall, crossed at a sprint with constant jumping.
        let ground = |x: i32, z: i32| 10 + ((x * 7 + z * 13).rem_euclid(5) == 0) as i32;
        let mut world = Fake(move |x, y, z| {
            if x >= 40 || y < ground(x, z) { Kind::Solid } else { Kind::Air }
        });
        let mut p = Player::new(DVec3::new(0.5, 12.0, 0.5));
        let c = Controls {
            wish: Vec3::new(1.0, 0.0, 0.3),
            jump: true,
            sprint: true,
            ..Default::default()
        };
        for _ in 0..(8.0 * 60.0) as u32 {
            p.update(&mut world, c, 1.0 / 60.0);
            assert!(!Player::blocked(p.pos, &mut world), "inside solid at {:?}", p.pos);
        }
        assert!(p.pos.x > 30.0 && p.pos.x < 40.0);
    }

    fn open_air(_: i32, _: i32, _: i32) -> Kind {
        Kind::Air
    }

    fn gliding_at(look: Vec3, height: f64) -> (Player, Controls) {
        let mut p = Player::new(DVec3::new(0.5, height, 0.5));
        let mut world = Fake(open_air);
        p.update(&mut world, Controls::default(), 1.0 / 120.0);
        let open = Controls {
            deploy: true,
            look,
            ..Default::default()
        };
        p.update(&mut world, open, 1.0 / 120.0);
        assert_eq!(p.mode, Mode::Glide);
        let flying = Controls { look, ..Default::default() };
        (p, flying)
    }

    #[test]
    fn the_glider_opens_only_in_the_air() {
        let mut world = Fake(|_, y, _| if y < 10 { Kind::Solid } else { Kind::Air });
        let mut p = Player::new(DVec3::new(0.5, 10.0, 0.5));
        run(&mut p, &mut world, Controls::default(), 0.5);
        assert!(p.on_ground);
        let open = Controls {
            deploy: true,
            look: Vec3::X,
            ..Default::default()
        };
        p.update(&mut world, open, 1.0 / 120.0);
        assert_eq!(p.mode, Mode::Walk, "opened standing on the ground");
    }

    #[test]
    fn gliding_trades_height_for_distance() {
        let mut world = Fake(open_air);
        let (mut p, flying) = gliding_at(Vec3::X, 1000.0);
        run(&mut p, &mut world, flying, 30.0);
        let (run_x, drop) = (p.pos.x, 1000.0 - p.pos.y);
        let ratio = run_x / drop;
        println!("glide ratio {ratio:.1}, sink {:.2}", drop / 30.0);
        assert!((6.0..12.0).contains(&ratio), "glide ratio {ratio}");
    }

    #[test]
    fn no_way_of_flying_gains_height_for_long() {
        let mut world = Fake(open_air);
        for pitch in [-0.8f32, -0.3, 0.0, 0.1, 0.2, 0.3, 0.6, 1.2] {
            let look = Vec3::new(pitch.cos(), pitch.sin(), 0.0);
            let (mut p, flying) = gliding_at(look, 1000.0);
            // Let the starting speed bleed off, then watch height.
            run(&mut p, &mut world, flying, 20.0);
            let before = p.pos.y;
            run(&mut p, &mut world, flying, 20.0);
            assert!(p.pos.y < before - 1.0, "pitch {pitch}: climbed from {before} to {}", p.pos.y);
        }
    }

    #[test]
    fn the_glider_lands_and_folds_away() {
        let mut world = Fake(|_, y, _| if y < 10 { Kind::Solid } else { Kind::Air });
        let mut p = Player::new(DVec3::new(0.5, 40.0, 0.5));
        p.update(&mut world, Controls::default(), 1.0 / 120.0);
        let flying = Controls { look: Vec3::X, ..Default::default() };
        p.update(&mut world, Controls { deploy: true, ..flying }, 1.0 / 120.0);
        assert_eq!(p.mode, Mode::Glide);
        run(&mut p, &mut world, flying, 60.0);
        assert_eq!(p.mode, Mode::Walk);
        assert!(p.on_ground);
    }

    /// Open water with a plane at y = 10, solid from x = 40 on, with a deck across it.
    fn harbour(x: i32, y: i32, _z: i32) -> Kind {
        if x >= 40 || y < 3 || ((20..=24).contains(&x) && y == 11) {
            Kind::Solid
        } else if y < 10 {
            Kind::Water
        } else {
            Kind::Air
        }
    }

    #[test]
    fn a_boat_floats_sails_under_bridges_and_runs_aground() {
        let mut world = Fake(harbour);
        let mut p = Player::new(DVec3::ZERO);
        p.board(DVec3::new(0.5, 10.0, 0.5), 0.0);
        run(&mut p, &mut world, forward(), 20.0);
        assert!((p.pos.y - 10.0).abs() < 0.2, "riding at {}", p.pos.y);
        assert!(p.pos.x > 30.0, "only reached {}", p.pos.x);
        assert!(p.pos.x < 40.0 - BOAT_REACH + 0.1, "sailed into the land at {}", p.pos.x);
    }

    #[test]
    fn a_boat_follows_a_river_down_a_step_but_not_up_a_cliff() {
        // Upper pool (plane 12) for x < 20, lower pool (plane 10) after, a wall at x = 40.
        let world_fn = |x: i32, y: i32, _: i32| {
            let plane = if x < 20 { 12 } else { 10 };
            if x >= 40 || y < 3 {
                Kind::Solid
            } else if y < plane {
                Kind::Water
            } else {
                Kind::Air
            }
        };
        let mut world = Fake(world_fn);
        let mut p = Player::new(DVec3::ZERO);
        p.board(DVec3::new(0.5, 12.0, 0.5), 0.0);
        run(&mut p, &mut world, forward(), 12.0);
        assert!(p.pos.x > 22.0, "stuck above the step at {}", p.pos.x);
        assert!((p.pos.y - 10.0).abs() < 0.3, "riding at {}", p.pos.y);
        let back = Controls { wish: -Vec3::X, ..Default::default() };
        run(&mut p, &mut world, back, 12.0);
        assert!(p.pos.x > 18.0, "went up a two-voxel step to {}", p.pos.x);
    }

    #[test]
    fn stepping_out_of_a_boat_finds_the_shore() {
        let mut world = Fake(|x, y, _| {
            if x >= 3 {
                if y < 10 { Kind::Solid } else { Kind::Air }
            } else if y < 3 {
                Kind::Solid
            } else if y < 10 {
                Kind::Water
            } else {
                Kind::Air
            }
        });
        let mut p = Player::new(DVec3::ZERO);
        p.board(DVec3::new(0.5, 10.0, 0.5), 0.0);
        let (left, _) = p.leave_boat(&mut world);
        assert_eq!(p.mode, Mode::Walk);
        assert!((left.x - 0.5).abs() < 1e-6);
        assert!(p.pos.x >= 3.0, "not ashore: {:?}", p.pos);
        assert!(!Player::blocked(p.pos, &mut world));
    }

    #[test]
    fn floats_in_water_and_climbs_out_onto_the_bank() {
        // A pool: bed at y = 5, water up to y = 9 (surface plane at 10), with a bank
        // whose top is flush with the water surface from x = 12 onward.
        let mut world = Fake(|x, y, _| {
            if x >= 12 {
                if y < 10 { Kind::Solid } else { Kind::Air }
            } else if y < 5 {
                Kind::Solid
            } else if y < 10 {
                Kind::Water
            } else {
                Kind::Air
            }
        });
        let mut p = Player::new(DVec3::new(2.5, 12.0, 0.5));
        run(&mut p, &mut world, Controls::default(), 4.0);
        assert!(p.swimming, "should be swimming, at y = {}", p.pos.y);
        let eye = p.pos.y + EYE_HEIGHT;
        assert!(eye > 9.6 && eye < 11.0, "floating with eye at {eye}");

        run(&mut p, &mut world, forward(), 6.0);
        assert!(p.pos.x > 13.0, "stuck in the water at x = {}", p.pos.x);
        assert!(!p.swimming && (p.pos.y - 10.0).abs() < 1e-2);
    }
}
