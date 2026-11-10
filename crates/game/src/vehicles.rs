//! Boats tied up at landings, and how boats and the glider are drawn.
//!
//! Boats are not part of the voxel world, since they move. Each landing starts with one
//! moored; a boat you have sailed stays wherever you left it for the rest of the session.

use std::{collections::HashSet, sync::Arc};

use glam::{DVec3, Vec3};
use worldgen::{macro_layout::MACRO_SIZE, world::GameWorld};

use crate::particles::ParticleInstance;

/// Farthest away boats are looked for and drawn.
const VIEW: f64 = 450.0;
/// How often the moorings around the player are looked up, in seconds.
const REFRESH: f32 = 1.0;

/// One moored boat's identity: its continent and which landing.
type MooringKey = (i32, i32, u32);

#[derive(Clone, Copy, Debug)]
pub struct Boat {
    /// On the water surface, at the middle of the hull.
    pub pos: DVec3,
    /// Direction the bow points, radians.
    pub heading: f32,
    mooring: Option<MooringKey>,
}

pub struct Fleet {
    world: Arc<GameWorld>,
    /// Moorings whose boat has been taken.
    taken: HashSet<MooringKey>,
    /// Boats left wherever the player went ashore.
    drifting: Vec<Boat>,
    /// Boats currently near the player.
    near: Vec<Boat>,
    timer: f32,
}

impl Fleet {
    pub fn new(world: Arc<GameWorld>) -> Self {
        Self {
            world,
            taken: HashSet::new(),
            drifting: Vec::new(),
            near: Vec::new(),
            timer: 0.0,
        }
    }

    pub fn update(&mut self, pos: DVec3, dt: f32) {
        self.timer -= dt;
        if self.timer > 0.0 {
            return;
        }
        self.timer = REFRESH;
        self.refresh(pos);
    }

    /// Rebuilds the list of boats near `pos`.
    pub fn refresh(&mut self, pos: DVec3) {
        self.near.clear();
        let cell = |v: f64| (v / MACRO_SIZE as f64).floor() as i32;
        let (cx, cz) = (cell(pos.x), cell(pos.z));
        for mz in cz - 1..=cz + 1 {
            for mx in cx - 1..=cx + 1 {
                let Some(c) = self.world.continent_if_ready(mx, mz) else { continue };
                let origin = DVec3::new((mx * MACRO_SIZE) as f64, 0.0, (mz * MACRO_SIZE) as f64);
                for (i, site) in c.trails.sites.iter().enumerate() {
                    let Some(m) = site.mooring else { continue };
                    let key = (mx, mz, i as u32);
                    let boat_pos = origin + m.pos.as_dvec3();
                    if !self.taken.contains(&key) && boat_pos.distance_squared(pos) < VIEW * VIEW {
                        self.near.push(Boat {
                            pos: boat_pos,
                            heading: m.heading,
                            mooring: Some(key),
                        });
                    }
                }
            }
        }
        self.near.extend(self.drifting.iter().filter(|b| b.pos.distance_squared(pos) < VIEW * VIEW));
    }

    /// The nearest boat within `reach`, by index into the boats near the player.
    pub fn nearest(&self, pos: DVec3, reach: f64) -> Option<usize> {
        self.near
            .iter()
            .enumerate()
            .map(|(i, b)| (i, b.pos.distance_squared(pos)))
            .filter(|&(_, d)| d < reach * reach)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    /// Takes a boat out of the world so the player can sail it.
    pub fn take(&mut self, index: usize) -> Boat {
        let boat = self.near.swap_remove(index);
        match boat.mooring {
            Some(key) => {
                self.taken.insert(key);
            }
            None => self.drifting.retain(|b| b.pos != boat.pos),
        }
        boat
    }

    /// Leaves a boat afloat.
    pub fn leave(&mut self, pos: DVec3, heading: f32) {
        let boat = Boat {
            pos,
            heading,
            mooring: None,
        };
        self.drifting.push(boat);
        self.near.push(boat);
    }

    pub fn boats(&self) -> &[Boat] {
        &self.near
    }
}

/// `(centre, size, colour)` in the model's own frame: +X forward, +Y up.
type Part = ([f32; 3], [f32; 3], [f32; 3]);

const HULL: [f32; 3] = [0.21, 0.13, 0.07];
const HULL_LIGHT: [f32; 3] = [0.42, 0.28, 0.15];
const MAST: [f32; 3] = [0.20, 0.14, 0.09];
const SAIL: [f32; 3] = [0.82, 0.76, 0.62];

const BOAT: [Part; 11] = [
    // Floor and sides.
    ([0.0, 0.05, 0.0], [4.6, 0.3, 1.5], HULL),
    ([0.0, 0.45, 0.85], [4.6, 0.6, 0.2], HULL_LIGHT),
    ([0.0, 0.45, -0.85], [4.6, 0.6, 0.2], HULL_LIGHT),
    ([-2.3, 0.45, 0.0], [0.2, 0.6, 1.9], HULL_LIGHT),
    // A blunt bow in two steps.
    ([2.65, 0.3, 0.0], [0.9, 0.5, 1.1], HULL),
    ([3.25, 0.5, 0.0], [0.5, 0.5, 0.6], HULL_LIGHT),
    // Thwarts.
    ([-1.0, 0.4, 0.0], [0.5, 0.12, 1.5], HULL_LIGHT),
    ([1.1, 0.4, 0.0], [0.5, 0.12, 1.5], HULL_LIGHT),
    // Mast, yard and sail.
    ([0.4, 2.6, 0.0], [0.16, 4.6, 0.16], MAST),
    ([0.4, 4.5, 0.0], [0.12, 0.12, 3.2], MAST),
    ([0.32, 3.0, 0.0], [0.05, 2.8, 2.9], SAIL),
];

const CANOPY: [f32; 3] = [0.72, 0.30, 0.24];
const CANOPY_LIGHT: [f32; 3] = [0.85, 0.74, 0.58];
const FRAME: [f32; 3] = [0.20, 0.15, 0.11];

const GLIDER: [Part; 7] = [
    // Wing, a broad shallow V, leading edge first so a strip shows at the top of the view.
    ([1.25, 1.05, 0.0], [0.5, 0.06, 7.4], FRAME),
    ([0.65, 1.0, 1.9], [1.1, 0.04, 3.6], CANOPY),
    ([0.65, 1.0, -1.9], [1.1, 0.04, 3.6], CANOPY),
    ([-0.3, 1.0, 1.9], [0.8, 0.04, 3.0], CANOPY_LIGHT),
    ([-0.3, 1.0, -1.9], [0.8, 0.04, 3.0], CANOPY_LIGHT),
    // Struts.
    ([0.6, 0.45, 1.2], [0.06, 1.1, 0.06], FRAME),
    ([0.6, 0.45, -1.2], [0.06, 1.1, 0.06], FRAME),
];

fn push_parts(parts: &[Part], origin: DVec3, heading: f32, cam: DVec3, out: &mut Vec<ParticleInstance>) {
    let (s, c) = heading.sin_cos();
    let base = (origin - cam).as_vec3();
    for (center, size, color) in parts {
        let p = Vec3::from(*center);
        let turned = Vec3::new(p.x * c - p.z * s, p.y, p.x * s + p.z * c);
        out.push(ParticleInstance {
            pos: (base + turned).into(),
            yaw: heading,
            size: *size,
            glow: 0.0,
            color: [color[0], color[1], color[2], 1.0],
        });
    }
}

/// Draws a boat with its hull rocking gently on the water.
pub fn draw_boat(pos: DVec3, heading: f32, time: f32, cam: DVec3, out: &mut Vec<ParticleInstance>) {
    let bob = (time * 1.1 + pos.x as f32 * 0.3).sin() * 0.05;
    push_parts(&BOAT, pos + DVec3::new(0.0, bob as f64, 0.0), heading, cam, out);
}

/// Draws the wing above someone gliding, with feet at `pos`.
pub fn draw_glider(pos: DVec3, heading: f32, cam: DVec3, out: &mut Vec<ParticleInstance>) {
    push_parts(&GLIDER, pos + DVec3::new(0.0, crate::player::EYE_HEIGHT, 0.0), heading, cam, out);
}
