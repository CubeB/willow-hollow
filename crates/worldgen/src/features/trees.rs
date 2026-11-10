//! Procedural trees and other standing things, built as voxel templates.
//!
//! Each species is a function of a seed and a size, so a handful of variants per
//! species (built once, reused everywhere) gives forests that read as one kind of tree
//! without any two neighbours being the same.

use glam::{IVec3, Vec3};
use voxel::{Kind, Materials, Voxel};

use super::shape::{Rng, Shape, Template};
use crate::materials as mat;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Species {
    Oak,
    Birch,
    Maple,
    Cherry,
    Spruce,
    TallPine,
    Redwood,
    ElderOak,
    Jungle,
    Kapok,
    Acacia,
    Baobab,
    Willow,
    Palm,
    DeadSnag,
    Cactus,
    Bush,
    Fern,
    Boulder,
    MossyBoulder,
    FallenLog,
    Stump,
}

impl Species {
    pub const ALL: [Species; 22] = [
        Species::Oak,
        Species::Birch,
        Species::Maple,
        Species::Cherry,
        Species::Spruce,
        Species::TallPine,
        Species::Redwood,
        Species::ElderOak,
        Species::Jungle,
        Species::Kapok,
        Species::Acacia,
        Species::Baobab,
        Species::Willow,
        Species::Palm,
        Species::DeadSnag,
        Species::Cactus,
        Species::Bush,
        Species::Fern,
        Species::Boulder,
        Species::MossyBoulder,
        Species::FallenLog,
        Species::Stump,
    ];

    /// How many variants to pre-build. Big trees are costly to build and rarely
    /// side by side, so they get fewer.
    pub fn variants(self) -> usize {
        match self {
            Species::Redwood | Species::Kapok | Species::ElderOak | Species::TallPine => 5,
            Species::Boulder | Species::MossyBoulder | Species::Stump | Species::Cactus => 6,
            _ => 8,
        }
    }

    pub fn build(self, seed: u32) -> Template {
        let mut rng = Rng::new(seed ^ (self as u32).wrapping_mul(0x27d4_eb2f));
        let mut s = Shape::new();
        match self {
            Species::Oak => oak(&mut s, &mut rng, mat::LEAVES_OAK, 1.0),
            Species::Maple => {
                let leaves = if rng.unit() < 0.5 { mat::LEAVES_MAPLE } else { mat::LEAVES_MAPLE_RED };
                oak(&mut s, &mut rng, leaves, 0.95)
            }
            Species::Cherry => oak(&mut s, &mut rng, mat::LEAVES_CHERRY, 0.8),
            Species::Birch => birch(&mut s, &mut rng),
            Species::Spruce => spruce(&mut s, &mut rng, 1.0),
            Species::TallPine => spruce(&mut s, &mut rng, 1.9),
            Species::Redwood => redwood(&mut s, &mut rng),
            Species::ElderOak => elder_oak(&mut s, &mut rng),
            Species::Jungle => jungle(&mut s, &mut rng),
            Species::Kapok => kapok(&mut s, &mut rng),
            Species::Acacia => acacia(&mut s, &mut rng),
            Species::Baobab => baobab(&mut s, &mut rng),
            Species::Willow => willow(&mut s, &mut rng),
            Species::Palm => palm(&mut s, &mut rng),
            Species::DeadSnag => dead_snag(&mut s, &mut rng),
            Species::Cactus => cactus(&mut s, &mut rng),
            Species::Bush => bush(&mut s, &mut rng, mat::LEAVES_OAK),
            Species::Fern => bush(&mut s, &mut rng, mat::LEAVES_JUNGLE),
            Species::Boulder => boulder(&mut s, &mut rng, false),
            Species::MossyBoulder => boulder(&mut s, &mut rng, true),
            Species::FallenLog => fallen_log(&mut s, &mut rng),
            Species::Stump => stump(&mut s, &mut rng),
        }
        s.build()
    }
}

/// One box of a low-detail stand-in for a model, in the model's voxel coordinates.
/// `hi` is exclusive.
#[derive(Clone, Copy, Debug)]
pub struct ImpostorBox {
    pub lo: IVec3,
    pub hi: IVec3,
    pub voxel: Voxel,
}

/// A few boxes standing in for a finished model: its trunk, then its crown cut into
/// layers. Worked out from the voxels themselves, so a change to how a species grows
/// changes its stand-in too.
pub fn impostor(t: &Template, mats: &Materials) -> Vec<ImpostorBox> {
    /// Tallest a layer of the crown may be.
    const LAYER: i32 = 14;
    const MAX_LAYERS: i32 = 6;
    let is_leaf = |v: Voxel| mats.get(v).kind == Kind::Foliage;
    // The box around most of these voxels: stray tips would make it far too wide.
    let boxed = |list: &[&(IVec3, Voxel)], lo_y: i32, hi_y: i32| -> Option<ImpostorBox> {
        let mut xs: Vec<i32> = list.iter().map(|(p, _)| p.x).collect();
        let mut zs: Vec<i32> = list.iter().map(|(p, _)| p.z).collect();
        xs.sort_unstable();
        zs.sort_unstable();
        let trim = list.len() / 8;
        let (first, last) = (*list.first()?, list.len() - 1 - trim);
        let mut counts: Vec<(Voxel, u32)> = Vec::new();
        for (_, v) in list {
            match counts.iter_mut().find(|c| c.0 == *v) {
                Some(c) => c.1 += 1,
                None => counts.push((*v, 1)),
            }
        }
        let voxel = counts.iter().max_by_key(|c| c.1).map_or(first.1, |c| c.0);
        Some(ImpostorBox {
            lo: IVec3::new(xs[trim], lo_y, zs[trim]),
            hi: IVec3::new(xs[last] + 1, hi_y, zs[last] + 1),
            voxel,
        })
    };
    let leaves: Vec<&(IVec3, Voxel)> = t.voxels.iter().filter(|(_, v)| is_leaf(*v)).collect();
    let wood: Vec<&(IVec3, Voxel)> = t.voxels.iter().filter(|(p, v)| !is_leaf(*v) && p.y >= 0).collect();
    // What the layers are cut from: a crown, or the whole of a tree with no leaves.
    let body = if leaves.is_empty() { &wood } else { &leaves };
    let Some(bottom) = body.iter().map(|(p, _)| p.y).min() else {
        return Vec::new();
    };
    let top = body.iter().map(|(p, _)| p.y).max().unwrap_or(bottom) + 1;

    let mut boxes = Vec::new();
    if !leaves.is_empty() && bottom > 0 {
        let stem: Vec<_> = wood.iter().copied().filter(|(p, _)| p.y <= bottom).collect();
        boxes.extend(boxed(&stem, 0, bottom));
    }
    let layers = ((top - bottom) / LAYER).clamp(1, MAX_LAYERS);
    for k in 0..layers {
        let lo_y = bottom + (top - bottom) * k / layers;
        let hi_y = bottom + (top - bottom) * (k + 1) / layers;
        let layer: Vec<_> = body.iter().copied().filter(|(p, _)| (lo_y..hi_y).contains(&p.y)).collect();
        boxes.extend(boxed(&layer, lo_y, hi_y));
    }
    boxes
}

/// A straight-ish trunk rooted a little below ground (so it does not float on a
/// slope), returning its top.
fn trunk(s: &mut Shape, rng: &mut Rng, height: f32, r_base: f32, r_top: f32, lean: f32, bark: Voxel) -> Vec3 {
    let (dir, _) = rng.heading();
    let top = Vec3::new(0.5, height, 0.5) + dir * lean;
    s.limb(Vec3::new(0.5, -3.0, 0.5), top, r_base, r_top, bark);
    top
}

fn oak(s: &mut Shape, rng: &mut Rng, leaves: Voxel, scale: f32) {
    let h = rng.range(12.0, 20.0) * scale;
    let r0 = rng.range(0.9, 1.5) * scale;
    let lean = rng.range(0.0, 1.5);
    let top = trunk(s, rng, h * 0.6, r0, 0.7, lean, mat::BARK_OAK);
    // Main limbs fan out from the top of the trunk, each ending in a mass of leaves.
    let limbs = rng.int(4, 6);
    let base_angle = rng.range(0.0, std::f32::consts::TAU);
    for i in 0..limbs {
        let angle = base_angle + i as f32 / limbs as f32 * std::f32::consts::TAU + rng.range(-0.3, 0.3);
        let out = Vec3::new(angle.cos(), 0.0, angle.sin());
        let start = top - Vec3::Y * rng.range(0.0, h * 0.15);
        let end = start + out * rng.range(3.5, 6.5) * scale + Vec3::Y * rng.range(2.0, 5.0) * scale;
        s.limb(start, end, 0.65 * scale, 0.4, mat::BARK_OAK);
        let r = rng.range(3.0, 4.4) * scale;
        s.blob(end + Vec3::Y * 0.5, Vec3::new(r, r * 0.72, r), leaves, rng.next_u32(), 0.4);
    }
    let r = rng.range(3.8, 5.2) * scale;
    s.blob(top + Vec3::Y * (h * 0.22), Vec3::new(r, r * 0.75, r), leaves, rng.next_u32(), 0.4);
}

fn elder_oak(s: &mut Shape, rng: &mut Rng) {
    // An ancient giant: huge trunk, limbs like trunks, a crown you could live under.
    let h = rng.range(34.0, 46.0);
    let r0 = rng.range(2.6, 3.4);
    let top = trunk(s, rng, h * 0.5, r0, 1.8, 0.0, mat::BARK_OAK);
    // Root flare.
    for _ in 0..rng.int(4, 6) {
        let (dir, _) = rng.heading();
        s.limb(Vec3::new(0.5, 1.5, 0.5), Vec3::new(0.5, -2.0, 0.5) + dir * rng.range(4.0, 6.0), 1.4, 0.6, mat::BARK_OAK);
    }
    let limbs = rng.int(5, 7);
    for i in 0..limbs {
        let angle = i as f32 / limbs as f32 * std::f32::consts::TAU + rng.range(-0.3, 0.3);
        let out = Vec3::new(angle.cos(), 0.0, angle.sin());
        let start = top - Vec3::Y * rng.range(0.0, 4.0);
        let mid = start + out * rng.range(6.0, 9.0) + Vec3::Y * rng.range(3.0, 6.0);
        let end = mid + out * rng.range(4.0, 7.0) + Vec3::Y * rng.range(1.0, 4.0);
        s.limb(start, mid, 1.5, 1.0, mat::BARK_OAK);
        s.limb(mid, end, 1.0, 0.5, mat::BARK_OAK);
        let r = rng.range(6.5, 8.5);
        s.blob(mid + Vec3::Y * 2.0, Vec3::new(r, r * 0.6, r), mat::LEAVES_OAK, rng.next_u32(), 0.4);
        let r = rng.range(5.0, 7.0);
        s.blob(end + Vec3::Y, Vec3::new(r, r * 0.65, r), mat::LEAVES_OAK, rng.next_u32(), 0.45);
    }
    let r = rng.range(7.0, 9.0);
    s.blob(top + Vec3::Y * (h * 0.3), Vec3::new(r, r * 0.7, r), mat::LEAVES_OAK, rng.next_u32(), 0.4);
}

fn birch(s: &mut Shape, rng: &mut Rng) {
    let h = rng.range(13.0, 20.0);
    let lean = rng.range(0.0, 2.0);
    let top = trunk(s, rng, h, 0.75, 0.45, lean, mat::BARK_BIRCH);
    // Light, open crown of small clusters up the top half of the trunk.
    for i in 0..rng.int(4, 6) {
        let t = 0.5 + i as f32 * 0.1 + rng.range(-0.05, 0.05);
        let (dir, _) = rng.heading();
        let c = Vec3::new(0.5, 0.0, 0.5).lerp(top, t) + dir * rng.range(0.5, 2.0);
        let r = rng.range(1.8, 2.8);
        s.blob(c, Vec3::new(r, r * 0.9, r), mat::LEAVES_BIRCH, rng.next_u32(), 0.55);
    }
    s.blob(top + Vec3::Y, Vec3::new(2.2, 2.4, 2.2), mat::LEAVES_BIRCH, rng.next_u32(), 0.5);
}

fn spruce(s: &mut Shape, rng: &mut Rng, scale: f32) {
    let h = rng.range(15.0, 26.0) * scale;
    let bare = h * rng.range(0.18, 0.3);
    let r_base = rng.range(0.8, 1.2) * scale.sqrt();
    s.limb(Vec3::new(0.5, -3.0, 0.5), Vec3::new(0.5, h, 0.5), r_base, 0.35, mat::BARK_PINE);
    // Tiers of needles: a cone, pinched between whorls so the branches show as layers.
    let r_max = rng.range(3.2, 4.6) * scale.sqrt();
    let tier = rng.range(2.6, 3.6);
    let phase = rng.range(0.0, tier);
    let mut y = bare;
    while y < h - 1.0 {
        let frac = (y - bare) / (h - bare);
        let saw = ((y + phase) / tier).fract();
        let r = (r_max * (1.0 - frac).powf(0.85) + 0.5) * (0.55 + 0.45 * saw);
        s.blob(Vec3::new(0.5, y + 0.5, 0.5), Vec3::new(r, 0.9, r), mat::NEEDLES, rng.next_u32(), 0.35);
        y += 1.0;
    }
    s.limb(Vec3::new(0.5, h - 1.0, 0.5), Vec3::new(0.5, h + 1.5, 0.5), 0.6, 0.3, mat::NEEDLES);
}

fn redwood(s: &mut Shape, rng: &mut Rng) {
    // The towering tree: a massive fluted column with the crown far overhead.
    let h = rng.range(72.0, 118.0);
    let r0 = rng.range(3.2, 4.4);
    let axis = Vec3::new(0.5, 0.0, 0.5);
    // Trunk in sections so it can taper non-linearly and flare at the foot.
    let mut prev = (axis - Vec3::Y * 4.0, r0 * 1.5);
    for i in 1..=8 {
        let t = i as f32 / 8.0;
        let y = h * 0.92 * t;
        let r = 0.9 + (r0 - 0.9) * (1.0 - t).powf(1.4);
        s.limb(prev.0, axis + Vec3::Y * y, prev.1, r, mat::BARK_REDWOOD);
        prev = (axis + Vec3::Y * y, r);
    }
    // Buttressed foot.
    for _ in 0..rng.int(5, 8) {
        let (dir, _) = rng.heading();
        s.limb(axis + Vec3::Y * 2.0, axis - Vec3::Y * 2.0 + dir * rng.range(r0 + 1.0, r0 + 3.5), r0 * 0.5, 0.8, mat::BARK_REDWOOD);
    }
    // Whorls of short, slightly drooping branches up the top half, shorter near the top.
    let crown_base = h * rng.range(0.42, 0.52);
    let mut y = crown_base;
    while y < h * 0.95 {
        let frac = (y - crown_base) / (h - crown_base);
        let count = rng.int(3, 5);
        let base_angle = rng.range(0.0, std::f32::consts::TAU);
        for i in 0..count {
            let angle = base_angle + i as f32 / count as f32 * std::f32::consts::TAU + rng.range(-0.25, 0.25);
            let out = Vec3::new(angle.cos(), 0.0, angle.sin());
            let len = (9.0 - 6.0 * frac) * rng.range(0.8, 1.15);
            let start = axis + Vec3::Y * y;
            let end = start + out * len - Vec3::Y * rng.range(0.5, 2.0);
            s.limb(start, end, 0.8, 0.4, mat::BARK_REDWOOD);
            let r = (3.2 - 1.4 * frac) * rng.range(0.85, 1.15);
            s.blob(start.lerp(end, 0.6), Vec3::new(r, 1.3, r), mat::NEEDLES_REDWOOD, rng.next_u32(), 0.4);
            s.blob(end, Vec3::new(r * 0.9, 1.4, r * 0.9), mat::NEEDLES_REDWOOD, rng.next_u32(), 0.4);
        }
        y += rng.range(4.0, 6.0);
    }
    s.blob(axis + Vec3::Y * (h * 0.95), Vec3::new(2.4, 4.0, 2.4), mat::NEEDLES_REDWOOD, rng.next_u32(), 0.4);
}

fn jungle(s: &mut Shape, rng: &mut Rng) {
    let h = rng.range(20.0, 32.0);
    let lean = rng.range(0.5, 2.5);
    let top = trunk(s, rng, h, 0.95, 0.6, lean, mat::BARK_JUNGLE);
    // An umbrella: a few near-horizontal limbs under a flattened canopy.
    for _ in 0..rng.int(3, 5) {
        let (dir, _) = rng.heading();
        let end = top + dir * rng.range(3.0, 5.5) + Vec3::Y * rng.range(0.5, 1.5);
        s.limb(top - Vec3::Y, end, 0.5, 0.3, mat::BARK_JUNGLE);
        let r = rng.range(3.0, 4.2);
        s.blob(end, Vec3::new(r, r * 0.5, r), mat::LEAVES_JUNGLE, rng.next_u32(), 0.45);
    }
    let r = rng.range(4.5, 6.0);
    s.blob(top + Vec3::Y * 1.5, Vec3::new(r, r * 0.45, r), mat::LEAVES_JUNGLE, rng.next_u32(), 0.4);
    s.hang(mat::LEAVES_JUNGLE, mat::VINE, rng, 0.08, (3, 9));
}

fn kapok(s: &mut Shape, rng: &mut Rng) {
    // Rainforest giant with buttress roots, an emergent crown, and hanging lianas.
    let h = rng.range(46.0, 68.0);
    let r0 = rng.range(2.2, 3.0);
    let axis = Vec3::new(0.5, 0.0, 0.5);
    s.limb(axis - Vec3::Y * 3.0, axis + Vec3::Y * h, r0, 1.4, mat::BARK_JUNGLE);
    for _ in 0..rng.int(4, 6) {
        let (dir, _) = rng.heading();
        let foot = axis - Vec3::Y * 2.0 + dir * rng.range(5.0, 8.0);
        s.limb(axis + Vec3::Y * rng.range(7.0, 11.0), foot, 0.9, 0.9, mat::BARK_JUNGLE);
    }
    let limbs = rng.int(5, 7);
    for i in 0..limbs {
        let angle = i as f32 / limbs as f32 * std::f32::consts::TAU + rng.range(-0.3, 0.3);
        let out = Vec3::new(angle.cos(), 0.0, angle.sin());
        let start = axis + Vec3::Y * (h - rng.range(0.0, 5.0));
        let end = start + out * rng.range(9.0, 14.0) + Vec3::Y * rng.range(1.0, 3.0);
        s.limb(start, end, 1.1, 0.5, mat::BARK_JUNGLE);
        let r = rng.range(5.0, 7.0);
        s.blob(end, Vec3::new(r, r * 0.45, r), mat::LEAVES_JUNGLE, rng.next_u32(), 0.4);
        s.blob(start.lerp(end, 0.5) + Vec3::Y, Vec3::new(r * 0.8, 2.5, r * 0.8), mat::LEAVES_JUNGLE, rng.next_u32(), 0.45);
    }
    let r = rng.range(6.0, 8.0);
    s.blob(axis + Vec3::Y * (h + 1.5), Vec3::new(r, 3.0, r), mat::LEAVES_JUNGLE, rng.next_u32(), 0.4);
    s.hang(mat::LEAVES_JUNGLE, mat::VINE, rng, 0.05, (6, 22));
}

fn acacia(s: &mut Shape, rng: &mut Rng) {
    // The savanna silhouette: a leaning trunk forking into a flat spreading top.
    let h = rng.range(7.0, 11.0);
    let (lean_dir, _) = rng.heading();
    let base = Vec3::new(0.5, -3.0, 0.5);
    let fork = Vec3::new(0.5, h, 0.5) + lean_dir * rng.range(1.5, 3.5);
    s.limb(base, fork, 0.9, 0.6, mat::BARK_ACACIA);
    for _ in 0..rng.int(3, 4) {
        let (dir, _) = rng.heading();
        let end = fork + dir * rng.range(3.0, 6.0) + Vec3::Y * rng.range(2.0, 3.5);
        s.limb(fork, end, 0.55, 0.3, mat::BARK_ACACIA);
        let r = rng.range(4.5, 6.5);
        s.blob(end + Vec3::Y * 0.5, Vec3::new(r, 1.3, r), mat::LEAVES_ACACIA, rng.next_u32(), 0.45);
    }
}

fn baobab(s: &mut Shape, rng: &mut Rng) {
    let h = rng.range(11.0, 16.0);
    let r0 = rng.range(2.8, 3.8);
    let axis = Vec3::new(0.5, 0.0, 0.5);
    // A bottle of a trunk: fattest a little way up.
    s.limb(axis - Vec3::Y * 3.0, axis + Vec3::Y * (h * 0.35), r0 * 0.9, r0, mat::BARK_BAOBAB);
    s.limb(axis + Vec3::Y * (h * 0.35), axis + Vec3::Y * h, r0, r0 * 0.55, mat::BARK_BAOBAB);
    for _ in 0..rng.int(5, 8) {
        let (dir, _) = rng.heading();
        let start = axis + Vec3::Y * (h - rng.range(0.0, 2.0));
        let end = start + dir * rng.range(3.0, 6.0) + Vec3::Y * rng.range(2.0, 5.0);
        s.limb(start, end, 1.0, 0.4, mat::BARK_BAOBAB);
        let r = rng.range(1.6, 2.4);
        s.blob(end, Vec3::new(r, r * 0.8, r), mat::LEAVES_BAOBAB, rng.next_u32(), 0.5);
    }
}

fn willow(s: &mut Shape, rng: &mut Rng) {
    let h = rng.range(7.0, 10.0);
    let lean = rng.range(0.5, 2.0);
    let top = trunk(s, rng, h, 1.1, 0.7, lean, mat::BARK_WILLOW);
    for _ in 0..rng.int(4, 6) {
        let (dir, _) = rng.heading();
        let end = top + dir * rng.range(3.0, 5.0) + Vec3::Y * rng.range(1.0, 3.0);
        s.limb(top, end, 0.5, 0.3, mat::BARK_WILLOW);
        let r = rng.range(2.5, 3.5);
        s.blob(end, Vec3::new(r, r * 0.8, r), mat::LEAVES_WILLOW, rng.next_u32(), 0.4);
    }
    let r = rng.range(4.0, 5.5);
    s.blob(top + Vec3::Y * 2.0, Vec3::new(r, r * 0.7, r), mat::LEAVES_WILLOW, rng.next_u32(), 0.4);
    // The curtains.
    s.hang(mat::LEAVES_WILLOW, mat::LEAVES_WILLOW, rng, 0.55, (3, 8));
}

fn palm(s: &mut Shape, rng: &mut Rng) {
    // A curved trunk, drawn in bending sections, with fronds arching from the top.
    let h = rng.range(9.0, 15.0);
    let (lean, _) = rng.heading();
    let bend = rng.range(1.5, 4.0);
    let mut prev = Vec3::new(0.5, -3.0, 0.5);
    let mut top = prev;
    for i in 1..=6 {
        let t = i as f32 / 6.0;
        top = Vec3::new(0.5, -3.0 + (h + 3.0) * t, 0.5) + lean * (bend * t * t);
        s.limb(prev, top, 0.7, 0.6, mat::BARK_PALM);
        prev = top;
    }
    let fronds = rng.int(7, 10);
    for i in 0..fronds {
        let angle = i as f32 / fronds as f32 * std::f32::consts::TAU + rng.range(-0.2, 0.2);
        let out = Vec3::new(angle.cos(), 0.0, angle.sin());
        let len = rng.range(5.0, 7.5);
        // Each frond rises then droops: two segments.
        let mid = top + out * (len * 0.5) + Vec3::Y * 1.5;
        let end = top + out * len - Vec3::Y * rng.range(1.0, 2.5);
        s.limb(top, mid, 0.8, 0.7, mat::LEAVES_PALM);
        s.limb(mid, end, 0.7, 0.4, mat::LEAVES_PALM);
    }
    s.blob(top + Vec3::Y, Vec3::new(1.6, 1.4, 1.6), mat::LEAVES_PALM, rng.next_u32(), 0.3);
}

fn dead_snag(s: &mut Shape, rng: &mut Rng) {
    let h = rng.range(8.0, 16.0);
    let lean = rng.range(0.0, 1.5);
    let top = trunk(s, rng, h, 1.0, 0.4, lean, mat::WOOD_DEAD);
    for _ in 0..rng.int(2, 4) {
        let (dir, _) = rng.heading();
        let start = top - Vec3::Y * rng.range(1.0, h * 0.5);
        s.limb(start, start + dir * rng.range(2.0, 4.0) + Vec3::Y * rng.range(0.5, 2.5), 0.45, 0.25, mat::WOOD_DEAD);
    }
}

fn cactus(s: &mut Shape, rng: &mut Rng) {
    let h = rng.int(3, 7);
    for y in -1..=h {
        s.set(IVec3::new(0, y, 0), mat::CACTUS);
    }
    for _ in 0..rng.int(0, 2) {
        let (dir, _) = rng.heading();
        let d = IVec3::new(dir.x.round() as i32, 0, dir.z.round() as i32);
        let d = if d == IVec3::ZERO { IVec3::X } else { d };
        let y0 = rng.int(1, (h - 2).max(1));
        s.set(IVec3::new(d.x, y0, d.z), mat::CACTUS);
        for y in y0..=(y0 + rng.int(1, 3)).min(h) {
            s.set(IVec3::new(d.x * 2, y, d.z * 2), mat::CACTUS);
        }
    }
}

fn bush(s: &mut Shape, rng: &mut Rng, leaves: Voxel) {
    let r = rng.range(1.4, 2.6);
    s.blob(Vec3::new(0.5, r * 0.5, 0.5), Vec3::new(r, r * 0.7, r), leaves, rng.next_u32(), 0.5);
    if rng.unit() < 0.5 {
        let (dir, _) = rng.heading();
        let r2 = rng.range(1.0, 1.8);
        s.blob(Vec3::new(0.5, r2 * 0.5, 0.5) + dir * r, Vec3::new(r2, r2 * 0.7, r2), leaves, rng.next_u32(), 0.5);
    }
}

fn boulder(s: &mut Shape, rng: &mut Rng, mossy: bool) {
    let r = rng.range(1.3, 3.2);
    let rock = if rng.unit() < 0.3 { mat::ROCK_DARK } else { mat::STONE };
    // Sunk into the ground a little, like something that has sat there a long time.
    let c = Vec3::new(0.5, r * 0.4, 0.5);
    s.blob(c, Vec3::new(r * rng.range(0.8, 1.2), r * 0.8, r * rng.range(0.8, 1.2)), rock, rng.next_u32(), 0.3);
    if mossy {
        // Moss on the upper faces.
        let tops: Vec<IVec3> = s
            .snapshot()
            .into_iter()
            .filter(|p| s.get(*p + IVec3::Y).is_none() && p.y >= 0)
            .collect();
        for p in tops {
            if rng.unit() < 0.7 {
                s.paint(p, mat::MOSS);
            }
        }
    }
}

fn fallen_log(s: &mut Shape, rng: &mut Rng) {
    let (dir, _) = rng.heading();
    let len = rng.range(6.0, 10.0);
    let r = rng.range(0.8, 1.3);
    let a = Vec3::new(0.5, r, 0.5);
    let b = a + dir * len;
    s.limb(a, b, r, r * 0.8, mat::BARK_OAK);
    s.lie_along(dir * len);
    // Hollow a stretch of the big ones.
    if r > 1.1 && rng.unit() < 0.5 {
        s.limb(a + dir * 1.5, b - dir * 1.5, r * 0.45, r * 0.4, voxel::AIR);
    }
    if rng.unit() < 0.6 {
        let tops: Vec<IVec3> = s
            .snapshot()
            .into_iter()
            .filter(|p| s.get(*p + IVec3::Y).is_none() && s.get(*p) != Some(voxel::AIR))
            .collect();
        for p in tops {
            if rng.unit() < 0.45 {
                s.paint(p, mat::MOSS);
            }
        }
    }
    s.drop_air();
}

fn stump(s: &mut Shape, rng: &mut Rng) {
    let h = rng.range(1.0, 2.5);
    s.limb(Vec3::new(0.5, -2.0, 0.5), Vec3::new(0.5, h, 0.5), rng.range(1.0, 1.6), 1.0, mat::BARK_OAK);
}
