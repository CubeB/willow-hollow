//! Building blocks for voxel models: limbs, leafy blobs and the like.

use std::collections::HashMap;

use glam::{IVec3, Vec3};
use voxel::{
    Voxel,
    mesh::{CELLS, PlantBox},
};

use super::attach::Raster;
use crate::{
    noise::{hash3, unit},
    scale::{BLOCK, OLD_VOXEL},
};

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
///
/// Models are laid out in 0.8 m blocks. [`Shape::set`] fills a whole block, while limbs
/// and blobs are rasterised at full voxel resolution, so organic shapes get smoother as
/// voxels get smaller.
pub struct Shape {
    voxels: HashMap<IVec3, Voxel>,
    /// Voxels per model unit.
    unit: f32,
    /// Whole voxels along a block's edge.
    block: i32,
    /// For models that lie along the ground: from the origin to the far end, in voxels.
    axis: Option<IVec3>,
    /// Fine parts drawn as plant boxes alongside the voxels (lantern frames, chains and
    /// the like), in model cells of 1/CELLS voxel. Only raw models carry them.
    fine: Vec<PlantBox>,
    /// Boxes of model voxels (inclusive) that must be left open to walk through: doors,
    /// stairs and the like. Props keep out of them.
    clear: Vec<(IVec3, IVec3)>,
    /// Fine sticks laid so far, in model cells, before they are made into boxes.
    sticks: Raster,
    /// The crowns of leaves laid so far, for what grows inside them.
    crowns: Vec<Crown>,
}

/// A mass of leaves on a tree, and the wood it grows from, in model units: where the fine
/// twigs of a bare winter crown spread from and how far.
#[derive(Clone, Copy, Debug)]
pub struct Crown {
    pub from: Vec3,
    pub centre: Vec3,
    pub radii: Vec3,
}

impl Default for Shape {
    fn default() -> Self {
        Self::new()
    }
}

impl Shape {
    /// A model laid out in 0.8 m blocks.
    pub fn new() -> Self {
        Self {
            voxels: HashMap::new(),
            unit: OLD_VOXEL,
            block: BLOCK,
            axis: None,
            fine: Vec::new(),
            clear: Vec::new(),
            sticks: Raster::new(),
            crowns: Vec::new(),
        }
    }

    /// A model laid out directly in voxels.
    pub fn raw() -> Self {
        Self {
            voxels: HashMap::new(),
            unit: 1.0,
            block: 1,
            axis: None,
            fine: Vec::new(),
            clear: Vec::new(),
            sticks: Raster::new(),
            crowns: Vec::new(),
        }
    }

    /// The voxels of the block at `p`.
    fn block_voxels(&self, p: IVec3) -> impl Iterator<Item = IVec3> + use<> {
        let b = self.block;
        (0..b).flat_map(move |y| (0..b).flat_map(move |z| (0..b).map(move |x| p * b + IVec3::new(x, y, z))))
    }

    /// Places a block, replacing whatever was there.
    pub fn set(&mut self, p: IVec3, v: Voxel) {
        for q in self.block_voxels(p) {
            self.voxels.insert(q, v);
        }
    }

    /// Places a block only where the model is still empty.
    pub fn fill(&mut self, p: IVec3, v: Voxel) {
        for q in self.block_voxels(p) {
            self.voxels.entry(q).or_insert(v);
        }
    }

    /// Replaces a single voxel, addressed as [`Shape::snapshot`] reports it.
    pub fn paint(&mut self, p: IVec3, v: Voxel) {
        self.voxels.insert(p, v);
    }

    pub fn get(&self, p: IVec3) -> Option<Voxel> {
        self.voxels.get(&p).copied()
    }

    /// Marks the model as lying along the ground, from its origin in direction `span`.
    pub fn lie_along(&mut self, span: Vec3) {
        self.axis = Some((span * self.unit).round().as_ivec3());
    }

    /// A tapered limb from `a` (radius `ra`) to `b` (radius `rb`): trunks, branches,
    /// roots. Replaces anything already there, so wood always wins over leaves.
    pub fn limb(&mut self, a: Vec3, b: Vec3, ra: f32, rb: f32, v: Voxel) {
        let (a, b, ra, rb) = (a * self.unit, b * self.unit, ra * self.unit, rb * self.unit);
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
                        self.voxels.insert(IVec3::new(x, y, z), v);
                    }
                }
            }
        }
    }

    /// A mass of foliage: an ellipsoid whose surface is roughened voxel by voxel so
    /// it does not read as a geometric solid. `ragged` (0..1) is how deep the
    /// roughness bites. Only fills empty space.
    pub fn blob(&mut self, center: Vec3, radii: Vec3, v: Voxel, seed: u32, ragged: f32) {
        let (center, radii) = (center * self.unit, radii * self.unit);
        let lo = (center - radii).floor().as_ivec3();
        let hi = (center + radii).ceil().as_ivec3();
        for y in lo.y..=hi.y {
            for z in lo.z..=hi.z {
                for x in lo.x..=hi.x {
                    let p = Vec3::new(x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5);
                    let d = ((p - center) / radii).length();
                    let edge = 1.0 - ragged * unit(hash3(seed, x, y, z));
                    if d <= edge {
                        self.voxels.entry(IVec3::new(x, y, z)).or_insert(v);
                    }
                }
            }
        }
    }

    /// Strands hanging from the underside of everything made of `from`: willow
    /// curtains, jungle vines. Each exposed underside grows a strand with probability
    /// `chance`, between `min` and `max` blocks long.
    pub fn hang(&mut self, from: Voxel, strand: Voxel, rng: &mut Rng, chance: f32, len: (i32, i32)) {
        let mut tips: Vec<IVec3> = self
            .voxels
            .iter()
            .filter(|&(p, &v)| v == from && !self.voxels.contains_key(&(*p - IVec3::Y)))
            .map(|(p, _)| *p)
            .collect();
        // HashMap order is arbitrary; sort so the same seed always gives the same tree.
        tips.sort_unstable_by_key(|p| (p.x, p.y, p.z));
        // Finer voxels expose more undersides; the same number of strands should hang.
        let chance = chance / (self.block * self.block) as f32;
        for tip in tips {
            if rng.unit() < chance {
                for k in 1..=rng.int(len.0, len.1) * self.block {
                    let p = tip - IVec3::Y * k;
                    if p.y < 1 || self.voxels.contains_key(&p) {
                        break;
                    }
                    self.voxels.insert(p, strand);
                }
            }
        }
    }

    /// Notes a crown of leaves at `centre`, of `radii`, growing from the wood at `from`, all
    /// in model units, for the twigs to fill once the voxels are laid (see [`Shape::stick`]).
    /// Nothing is drawn, nor any random number taken, so the tree is as it was.
    pub fn crown(&mut self, from: Vec3, centre: Vec3, radii: Vec3) {
        self.crowns.push(Crown { from, centre, radii });
    }

    /// The crowns noted so far, in the order they were.
    pub fn crowns(&self) -> &[Crown] {
        &self.crowns
    }

    /// A straight stick of fine cells from `a` to `b` (model units), `thick` cells across:
    /// a twig finer than a voxel. Where it crosses another, the first laid keeps the cell.
    pub fn stick(&mut self, a: Vec3, b: Vec3, thick: i32, v: Voxel) {
        let k = self.unit * CELLS as f32;
        let (a, b) = (a * k, b * k);
        let d = b - a;
        // A cell at a time along the way it mostly goes, so a slanting stick is laid in
        // runs and merges into a few boxes a voxel; where it steps aside it steps one way
        // at a time, so each cell meets the next face to face and no stick is seen as a
        // string of beads.
        let steps = d.abs().max_element().ceil().max(1.0) as i32;
        let lo = IVec3::splat(thick / 2);
        let mut last = a.floor().as_ivec3();
        for i in 0..=steps {
            let p = (a + d * (i as f32 / steps as f32)).floor().as_ivec3();
            let mut at = last;
            for axis in 0..3 {
                at[axis] = p[axis];
                for y in 0..thick {
                    for z in 0..thick {
                        for x in 0..thick {
                            self.sticks.set(at - lo + IVec3::new(x, y, z), v);
                        }
                    }
                }
            }
            last = p;
        }
    }

    /// Makes the sticks laid so far into fine parts, a voxel's worth at a time, keeping
    /// only those in voxels of the model that `keep` takes, and pulled back a cell from
    /// any side of one that gives onto empty space.
    pub fn finish_sticks(&mut self, keep: impl Fn(Voxel) -> bool) {
        let mut sticks = std::mem::take(&mut self.sticks);
        let open: HashMap<IVec3, [bool; 6]> = sticks
            .voxels()
            .filter(|&at| self.get(at).is_some_and(&keep))
            .map(|at| (at, FACES.map(|f| self.get(at + f).is_some())))
            .collect();
        sticks.retain_voxels(|at| open.contains_key(&at));
        // They move in the wind as the leaves round them do, which is the same sway at the
        // same place, so the leaves never part from round them and show them.
        let mut merged = Vec::new();
        sticks.finish(&mut merged, |_| (true, true));
        // Back into a box a voxel, as merging ran some on through the next: each is kept or
        // hidden with its own leaf.
        let mut boxes = Vec::new();
        for m in merged {
            let (lo, hi) = (IVec3::from_array(m.lo), IVec3::from_array(m.hi));
            let (first, last) = (lo.div_euclid(IVec3::splat(CELLS)), (hi - 1).div_euclid(IVec3::splat(CELLS)));
            for y in first.y..=last.y {
                for z in first.z..=last.z {
                    for x in first.x..=last.x {
                        let base = IVec3::new(x, y, z) * CELLS;
                        let (l, h) = (lo.max(base), hi.min(base + CELLS));
                        boxes.push(PlantBox { lo: l.to_array(), hi: h.to_array(), ..m });
                    }
                }
            }
        }
        for mut b in boxes {
            let at = IVec3::from_array(b.lo).div_euclid(IVec3::splat(CELLS));
            let sides = open[&at];
            let base = at * CELLS;
            for (face, &reach) in sides.iter().enumerate() {
                let axis = face / 2;
                if reach {
                    continue;
                }
                if face % 2 == 0 && b.hi[axis] == base[axis] + CELLS {
                    b.hi[axis] -= 1;
                } else if face % 2 == 1 && b.lo[axis] == base[axis] {
                    b.lo[axis] += 1;
                }
            }
            if (0..3).all(|a| b.lo[a] < b.hi[a]) {
                self.fine.push(b);
            }
        }
    }

    /// Positions of every voxel, in a fixed order.
    pub fn snapshot(&self) -> Vec<IVec3> {
        let mut v: Vec<IVec3> = self.voxels.keys().copied().collect();
        v.sort_unstable_by_key(|p| (p.y, p.x, p.z));
        v
    }

    /// Adds a fine part, in model cells.
    pub fn add_fine(&mut self, part: PlantBox) {
        self.fine.push(part);
    }

    /// Takes the fine parts added so far, in the order they were added.
    pub fn take_fine(&mut self) -> Vec<PlantBox> {
        std::mem::take(&mut self.fine)
    }

    /// Marks an inclusive box of voxels as a way through, to be kept clear of props.
    pub fn keep_clear(&mut self, lo: IVec3, hi: IVec3) {
        self.clear.push((lo.min(hi), lo.max(hi)));
    }

    /// Whether voxel `p` is in a way through marked by [`Shape::keep_clear`].
    pub fn must_stay_clear(&self, p: IVec3) -> bool {
        self.clear.iter().any(|&(lo, hi)| p.cmpge(lo).all() && p.cmple(hi).all())
    }

    /// The ways through marked by [`Shape::keep_clear`], as inclusive boxes.
    pub fn clear_boxes(&self) -> &[(IVec3, IVec3)] {
        &self.clear
    }

    /// Removes air voxels used to carve hollows.
    pub fn drop_air(&mut self) {
        self.voxels.retain(|_, v| *v != voxel::AIR);
    }

    /// Every voxel including air, sorted, for models that carve their own interiors.
    pub fn into_voxels(self) -> Vec<(IVec3, Voxel)> {
        let mut voxels: Vec<(IVec3, Voxel)> = self.voxels.into_iter().collect();
        voxels.sort_unstable_by_key(|(p, _)| (p.y, p.x, p.z));
        voxels
    }

    /// Finishes the model: voxels sorted for deterministic stamping, with bounds.
    pub fn build(mut self) -> Template {
        self.drop_air();
        let (block, axis) = (self.block, self.axis);
        let mut voxels: Vec<(IVec3, Voxel)> = self.voxels.into_iter().collect();
        voxels.sort_unstable_by_key(|(p, _)| (p.y, p.x, p.z));
        let (mut lo, mut hi) = (IVec3::MAX, IVec3::MIN);
        for (p, _) in &voxels {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        // How far it reaches from its own vertical axis, whichever way it is turned.
        let reach = lo.x.abs().max(hi.x.abs()).max(lo.z.abs()).max(hi.z.abs());
        let base = voxels
            .iter()
            .filter(|(p, _)| p.y <= block)
            .map(|(p, _)| p.x.abs().max(p.z.abs()))
            .max()
            .unwrap_or(0);
        Template {
            voxels,
            reach,
            base,
            top: hi.y,
            axis,
            fine: self.fine,
        }
    }
}

/// A finished voxel model, ready to be stamped into the world.
pub struct Template {
    pub voxels: Vec<(IVec3, Voxel)>,
    /// Largest horizontal distance from the origin to any voxel.
    pub reach: i32,
    /// Largest horizontal distance to any voxel near the ground: what it stands on.
    pub base: i32,
    /// Highest voxel, relative to the origin.
    pub top: i32,
    /// For models that lie along the ground: from the origin to the far end, in voxels.
    pub axis: Option<IVec3>,
    /// Fine parts in model cells: on a broadleaf, the twigs of its bare crown, each box
    /// inside one voxel of its leaves, so they are hidden while the leaves are on and show
    /// as each leaf falls (see `trees::twigs`).
    pub fine: Vec<PlantBox>,
}

/// The six neighbours of a voxel, in the order of a box's faces: +x, -x, +y, -y, +z, -z.
pub const FACES: [IVec3; 6] = [IVec3::X, IVec3::NEG_X, IVec3::Y, IVec3::NEG_Y, IVec3::Z, IVec3::NEG_Z];

/// Turns a model-space offset by quarter turns about the vertical axis.
pub fn rotate(p: IVec3, quarter_turns: u32) -> IVec3 {
    match quarter_turns & 3 {
        0 => p,
        1 => IVec3::new(-p.z, p.y, p.x),
        2 => IVec3::new(-p.x, p.y, -p.z),
        _ => IVec3::new(p.z, p.y, -p.x),
    }
}
