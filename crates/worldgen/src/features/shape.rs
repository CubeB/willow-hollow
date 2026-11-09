//! Building blocks for voxel models: limbs, leafy blobs and the like.

use std::collections::HashMap;

use glam::{IVec3, Vec3};
use voxel::Voxel;

use crate::noise::{hash3, unit};

/// A small deterministic random stream for shaping one model.
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Self {
        Self(seed.wrapping_mul(0x9e37_79b9) | 1)
    }

    pub fn next_u32(&mut self) -> u32 {
        // xorshift32
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }

    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f32 {
        unit(self.next_u32())
    }

    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }

    pub fn int(&mut self, lo: i32, hi: i32) -> i32 {
        lo + (self.unit() * (hi - lo + 1) as f32) as i32
    }

    /// A horizontal unit vector at a random angle, and that angle.
    pub fn heading(&mut self) -> (Vec3, f32) {
        let a = self.range(0.0, std::f32::consts::TAU);
        (Vec3::new(a.cos(), 0.0, a.sin()), a)
    }
}

/// A voxel model under construction, in its own coordinates: the origin is where it
/// meets the ground, +Y up.
#[derive(Default)]
pub struct Shape {
    voxels: HashMap<IVec3, Voxel>,
}

impl Shape {
    pub fn new() -> Self {
        Self::default()
    }

    /// Places a voxel, replacing whatever was there.
    pub fn set(&mut self, p: IVec3, v: Voxel) {
        self.voxels.insert(p, v);
    }

    /// Places a voxel only where the model is still empty.
    pub fn fill(&mut self, p: IVec3, v: Voxel) {
        self.voxels.entry(p).or_insert(v);
    }

    pub fn get(&self, p: IVec3) -> Option<Voxel> {
        self.voxels.get(&p).copied()
    }

    /// A tapered limb from `a` (radius `ra`) to `b` (radius `rb`): trunks, branches,
    /// roots. Replaces anything already there, so wood always wins over leaves.
    pub fn limb(&mut self, a: Vec3, b: Vec3, ra: f32, rb: f32, v: Voxel) {
        let r = ra.max(rb) + 1.0;
        let lo = (a.min(b) - Vec3::splat(r)).floor().as_ivec3();
        let hi = (a.max(b) + Vec3::splat(r)).ceil().as_ivec3();
        let ab = b - a;
        let len2 = ab.length_squared().max(1e-6);
        for y in lo.y..=hi.y {
            for z in lo.z..=hi.z {
                for x in lo.x..=hi.x {
                    let p = Vec3::new(x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5);
                    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
                    let radius = ra + (rb - ra) * t;
                    if (p - (a + ab * t)).length_squared() <= radius * radius {
                        self.set(IVec3::new(x, y, z), v);
                    }
                }
            }
        }
    }

    /// A mass of foliage: an ellipsoid whose surface is roughened voxel by voxel so
    /// it does not read as a geometric solid. `ragged` (0..1) is how deep the
    /// roughness bites. Only fills empty space.
    pub fn blob(&mut self, center: Vec3, radii: Vec3, v: Voxel, seed: u32, ragged: f32) {
        let lo = (center - radii).floor().as_ivec3();
        let hi = (center + radii).ceil().as_ivec3();
        for y in lo.y..=hi.y {
            for z in lo.z..=hi.z {
                for x in lo.x..=hi.x {
                    let p = Vec3::new(x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5);
                    let d = ((p - center) / radii).length();
                    let edge = 1.0 - ragged * unit(hash3(seed, x, y, z));
                    if d <= edge {
                        self.fill(IVec3::new(x, y, z), v);
                    }
                }
            }
        }
    }

    /// Strands hanging from the underside of everything made of `from`: willow
    /// curtains, jungle vines. Each exposed underside grows a strand with probability
    /// `chance`, between `min` and `max` voxels long.
    pub fn hang(&mut self, from: Voxel, strand: Voxel, rng: &mut Rng, chance: f32, len: (i32, i32)) {
        let mut tips: Vec<IVec3> = self
            .voxels
            .iter()
            .filter(|&(p, &v)| v == from && !self.voxels.contains_key(&(*p - IVec3::Y)))
            .map(|(p, _)| *p)
            .collect();
        // HashMap order is arbitrary; sort so the same seed always gives the same tree.
        tips.sort_unstable_by_key(|p| (p.x, p.y, p.z));
        for tip in tips {
            if rng.unit() < chance {
                for k in 1..=rng.int(len.0, len.1) {
                    let p = tip - IVec3::Y * k;
                    if p.y < 1 || self.voxels.contains_key(&p) {
                        break;
                    }
                    self.set(p, strand);
                }
            }
        }
    }

    /// Positions of every voxel, in a fixed order.
    pub fn snapshot(&self) -> Vec<IVec3> {
        let mut v: Vec<IVec3> = self.voxels.keys().copied().collect();
        v.sort_unstable_by_key(|p| (p.y, p.x, p.z));
        v
    }

    /// Removes air voxels used to carve hollows.
    pub fn drop_air(&mut self) {
        self.voxels.retain(|_, v| *v != voxel::AIR);
    }

    /// Finishes the model: voxels sorted for deterministic stamping, with bounds.
    pub fn build(mut self) -> Template {
        self.drop_air();
        let mut voxels: Vec<(IVec3, Voxel)> = self.voxels.into_iter().collect();
        voxels.sort_unstable_by_key(|(p, _)| (p.y, p.x, p.z));
        let (mut lo, mut hi) = (IVec3::MAX, IVec3::MIN);
        for (p, _) in &voxels {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        // How far it reaches from its own vertical axis, whichever way it is turned.
        let reach = lo.x.abs().max(hi.x.abs()).max(lo.z.abs()).max(hi.z.abs());
        Template {
            voxels,
            reach,
            top: hi.y,
        }
    }
}

/// A finished voxel model, ready to be stamped into the world.
pub struct Template {
    pub voxels: Vec<(IVec3, Voxel)>,
    /// Largest horizontal distance from the origin to any voxel.
    pub reach: i32,
    /// Highest voxel, relative to the origin.
    pub top: i32,
}

/// Turns a model-space offset by quarter turns about the vertical axis.
pub fn rotate(p: IVec3, quarter_turns: u32) -> IVec3 {
    match quarter_turns & 3 {
        0 => p,
        1 => IVec3::new(-p.z, p.y, p.x),
        2 => IVec3::new(-p.x, p.y, -p.z),
        _ => IVec3::new(p.z, p.y, -p.x),
    }
}
