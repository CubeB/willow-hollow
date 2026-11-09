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

#[derive(Default, Clone, Copy)]
pub struct Controls {
    /// Desired horizontal direction in world space (y ignored), zero if standing still.
    pub wish: Vec3,
    pub jump: bool,
    pub sprint: bool,
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
}

impl Player {
    pub fn new(pos: DVec3) -> Self {
        Self {
            pos,
            vel: Vec3::ZERO,
            on_ground: false,
            swimming: false,
            step_lag: 0.0,
        }
    }

    pub fn eye(&self) -> DVec3 {
        self.pos + DVec3::new(0.0, EYE_HEIGHT - self.step_lag as f64, 0.0)
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
        // Chest-deep in water means swimming.
        let chest = (self.pos + DVec3::new(0.0, 1.2, 0.0)).floor().as_ivec3();
        self.swimming = v.kind(chest.x, chest.y, chest.z) == Kind::Water;
        let head = (self.pos + DVec3::new(0.0, EYE_HEIGHT, 0.0)).floor().as_ivec3();
        let head_under = v.kind(head.x, head.y, head.z) == Kind::Water;

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
        self.vel.x += (wish.x * speed - self.vel.x) * blend;
        self.vel.z += (wish.z * speed - self.vel.z) * blend;

        if self.swimming {
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

impl Voxels for WorldVoxels {
    fn kind(&mut self, x: i32, y: i32, z: i32) -> Kind {
        let col = (x.div_euclid(CHUNK), z.div_euclid(CHUNK));
        let world = &self.world;
        let chunks = self
            .columns
            .entry(col)
            .or_insert_with(|| world.generate_column(col.0, col.1));
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
        };
        for _ in 0..(8.0 * 60.0) as u32 {
            p.update(&mut world, c, 1.0 / 60.0);
            assert!(!Player::blocked(p.pos, &mut world), "inside solid at {:?}", p.pos);
        }
        assert!(p.pos.x > 30.0 && p.pos.x < 40.0);
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
