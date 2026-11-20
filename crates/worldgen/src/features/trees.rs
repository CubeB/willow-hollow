//! Procedural trees and other standing things, built as voxel templates.
//!
//! Each species is a function of a seed and a size, so a handful of variants per
//! species (built once, reused everywhere) gives forests that read as one kind of tree
//! without any two neighbours being the same.

use glam::{IVec3, Vec3};
use voxel::{Kind, Materials, Voxel};

use super::shape::{Rng, Shape, Template};
use crate::{materials as mat, scale::OLD_VOXEL};

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
    // The evergreens come last, so the kinds before them keep their numbers, and with
    // them the shapes their variants are built from.
    Holly,
    Yew,
    HolmOak,
    ScotsPine,
    Juniper,
    HollyBush,
    Laurel,
    Gorse,
    // Added last so the kinds before keep their numbers.
    /// A willow cut back to a stout trunk for its poles, by a stream through a place's
    /// land (see `settlements::farmland::willows`).
    Pollard,
    /// A red mangrove of the tropical shallows, standing on its stilt roots (#69).
    Mangrove,
    /// A low shrub of the mangrove shallows and mud: a young mangrove, or a clump of the
    /// salt-loving bushes that grow up between them (#69).
    MangroveShrub,
}

impl Species {
    pub const ALL: [Species; 33] = [
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
        Species::Holly,
        Species::Yew,
        Species::HolmOak,
        Species::ScotsPine,
        Species::Juniper,
        Species::HollyBush,
        Species::Laurel,
        Species::Gorse,
        Species::Pollard,
        Species::Mangrove,
        Species::MangroveShrub,
    ];

    /// Whether it keeps its leaves through the winter and grows among broadleaves or on
    /// open ground: the kinds mixed into other woods as a minority.
    pub fn winter_green(self) -> bool {
        matches!(
            self,
            Species::Holly | Species::Yew | Species::HolmOak | Species::ScotsPine | Species::Juniper | Species::HollyBush | Species::Laurel | Species::Gorse
        )
    }

    /// How many variants to pre-build. Big trees are costly to build and rarely
    /// side by side, so they get fewer.
    pub fn variants(self) -> usize {
        match self {
            Species::Redwood | Species::Kapok | Species::ElderOak | Species::TallPine => 5,
            Species::Boulder | Species::MossyBoulder | Species::Stump | Species::Cactus => 6,
            // The trees most often met in stands of their own: enough that a short one, a
            // tall one, a sparse one and a full one all turn up in the same wood.
            Species::Oak | Species::Birch => 16,
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
            Species::Holly => holly(&mut s, &mut rng),
            Species::Yew => yew(&mut s, &mut rng),
            Species::HolmOak => holm_oak(&mut s, &mut rng),
            Species::ScotsPine => scots_pine(&mut s, &mut rng),
            Species::Juniper => juniper(&mut s, &mut rng),
            Species::HollyBush => holly_bush(&mut s, &mut rng),
            Species::Laurel => laurel(&mut s, &mut rng),
            Species::Gorse => gorse(&mut s, &mut rng),
            Species::Pollard => pollard(&mut s, &mut rng),
            Species::Mangrove => mangrove(&mut s, &mut rng),
            Species::MangroveShrub => mangrove_shrub(&mut s, &mut rng),
        }
        if let Some(wood) = self.bare_twigs() {
            twigs(&mut s, seed, wood);
        }
        s.build()
    }

    /// For a broadleaf that stands bare all winter, the twigs its bare crown shows.
    fn bare_twigs(self) -> Option<Voxel> {
        match self {
            Species::Oak | Species::Maple | Species::Cherry | Species::ElderOak | Species::Birch | Species::Willow | Species::Acacia => Some(mat::TWIG_FINE),
            _ => None,
        }
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

/// Two or three thinner branches forking from the end of a limb, up and out along
/// `out`, so a bare winter crown branches the way a real one does instead of ending in
/// stubs. Hidden inside the leaves in summer. `r` is the limb's end radius.
fn forks(s: &mut Shape, rng: &mut Rng, at: Vec3, out: Vec3, len: f32, r: f32, bark: Voxel) {
    let side = Vec3::new(-out.z, 0.0, out.x);
    for k in 0..rng.int(2, 3) {
        let turn = if k % 2 == 0 { 1.0 } else { -1.0 } * rng.range(0.35, 0.9);
        let dir = (out + side * turn + Vec3::Y * rng.range(0.5, 1.1)).normalize_or_zero();
        let tip = at + dir * len * rng.range(0.7, 1.15);
        s.limb(at, tip, r, (r * 0.75).max(0.32), bark);
    }
}

/// A mass of leaves (see [`Shape::blob`]) on the wood at `from`, noted as a crown for the
/// twigs of winter to fill.
fn leafy(s: &mut Shape, from: Vec3, centre: Vec3, radii: Vec3, leaves: Voxel, seed: u32, ragged: f32) {
    s.blob(centre, radii, leaves, seed, ragged);
    s.crown(from, centre, radii);
}

/// Sprigs grown into a crown for each unit of its size, in model units, and the fewest
/// any crown has, so a birch's small clusters are twiggy too.
const SPRIGS: f32 = 2.2;
const FEWEST_SPRIGS: f32 = 4.0;
/// Times a sprig forks, each fork shorter and thinner: once less in a small crown, which
/// would otherwise be a solid tangle.
const SPRIG_FORKS: i32 = 3;
const SMALL_CROWN: f32 = 2.5;

/// The fine twigs of a bare crown: from the wood each mass of leaves grows on, sprigs that
/// fork and fork again out to the edge of the leaves, so a broadleaf wood in winter is a
/// haze of fine twigs and not a few blocky limbs. They are kept only inside the tree's
/// own leaves, and pulled back a cell from where the leaves give onto the air: hidden
/// while the leaves are on, they show as each leaf falls, on the tree's own date, with
/// nothing more for the seasons to work out. From a random stream of their own, so the
/// voxels of the tree are as they were.
fn twigs(s: &mut Shape, seed: u32, wood: Voxel) {
    let mut rng = Rng::new(seed ^ 0x7c19_5e11);
    let crowns = s.crowns().to_vec();
    for c in &crowns {
        let size = (c.radii.x * c.radii.y * c.radii.z).cbrt();
        for _ in 0..(size * SPRIGS).max(FEWEST_SPRIGS).round() as i32 {
            // Toward somewhere near the edge of the leaves, mostly out and up.
            let (out, _) = rng.heading();
            let up = rng.range(-0.3, 0.95);
            let dir = out * (1.0 - up * up).sqrt() + Vec3::Y * up;
            let way = c.centre + dir * c.radii * rng.range(0.8, 1.0) - c.from;
            let forks = if size < SMALL_CROWN { SPRIG_FORKS - 1 } else { SPRIG_FORKS };
            sprig(s, &mut rng, c.from, way.normalize_or_zero(), way.length() * 0.5, forks, wood);
        }
    }
    s.finish_sticks(hides_twigs);
}

/// The leaves of the broadleaves, which hide the twigs of their bare crowns until they
/// fall.
pub fn hides_twigs(v: Voxel) -> bool {
    matches!(v, mat::LEAVES_OAK | mat::LEAVES_BIRCH | mat::LEAVES_WILLOW | mat::LEAVES_ACACIA | mat::LEAVES_CHERRY | mat::LEAVES_MAPLE | mat::LEAVES_MAPLE_RED)
}

/// One sprig of a bare crown from `at` along `dir`, forking `forks` times more.
fn sprig(s: &mut Shape, rng: &mut Rng, at: Vec3, dir: Vec3, len: f32, forks: i32, wood: Voxel) {
    let end = at + dir * len;
    // Stouter where it leaves the limb.
    s.stick(at, end, if forks == SPRIG_FORKS { 2 } else { 1 }, wood);
    if forks == 0 {
        return;
    }
    for k in 0..2 {
        let bend = Vec3::new(rng.range(-1.0, 1.0), rng.range(-0.5, 1.0), rng.range(-1.0, 1.0));
        let d = (dir + bend * 0.6).normalize_or_zero();
        let from = if k == 0 { end } else { at.lerp(end, rng.range(0.45, 0.8)) };
        let len = len * rng.range(0.5, 0.7);
        sprig(s, rng, from, d, len, forks - 1, wood);
    }
}

fn oak(s: &mut Shape, rng: &mut Rng, leaves: Voxel, scale: f32) {
    // A wood holds trees of every age: squat young ones with a few limbs beside tall old
    // ones with many. `size` is this tree against the middling one of its kind.
    let h = rng.range(10.5, 23.0) * scale;
    let size = (h / (16.0 * scale)).sqrt();
    // A tall tree's crown is carried higher, not wider: its neighbours are no further off.
    let spread = size.min(1.0) * scale;
    let r0 = rng.range(0.9, 1.5) * scale * size;
    let lean = rng.range(0.0, 1.5);
    let top = trunk(s, rng, h * 0.6, r0, 0.7, lean, mat::BARK_OAK);
    // Main limbs fan out from the top of the trunk, each ending in a mass of leaves. The
    // taller the tree the more of them, give or take.
    let limbs = (rng.int(3, 6) + ((size - 0.8) * 4.0) as i32).clamp(3, 8);
    let base_angle = rng.range(0.0, std::f32::consts::TAU);
    for i in 0..limbs {
        let angle = base_angle + i as f32 / limbs as f32 * std::f32::consts::TAU + rng.range(-0.3, 0.3);
        let out = Vec3::new(angle.cos(), 0.0, angle.sin());
        // Where there are many, they leave the trunk at different heights and not in a ring.
        let drop = if limbs > 5 && i % 2 == 1 { rng.range(0.12, 0.3) } else { rng.range(0.0, 0.15) };
        let start = top - Vec3::Y * h * drop;
        let end = start + out * rng.range(3.5, 6.2) * spread + Vec3::Y * rng.range(2.0, 5.0) * scale * size;
        s.limb(start, end, 0.65 * scale, 0.4, mat::BARK_OAK);
        let fork = rng.range(1.8, 2.8) * scale;
        forks(s, rng, end, out, fork, 0.4, mat::BARK_OAK);
        let r = rng.range(3.0, 4.4) * spread;
        leafy(s, end, end + Vec3::Y * 0.5, Vec3::new(r, r * 0.72, r), leaves, rng.next_u32(), 0.4);
    }
    let r = rng.range(3.8, 5.2) * spread;
    leafy(s, top, top + Vec3::Y * (h * 0.22), Vec3::new(r, r * 0.75, r), leaves, rng.next_u32(), 0.4);
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
        let fork = rng.range(2.5, 4.0);
        forks(s, rng, end, out, fork, 0.45, mat::BARK_OAK);
        let r = rng.range(6.5, 8.5);
        leafy(s, mid, mid + Vec3::Y * 2.0, Vec3::new(r, r * 0.6, r), mat::LEAVES_OAK, rng.next_u32(), 0.4);
        let r = rng.range(5.0, 7.0);
        leafy(s, end, end + Vec3::Y, Vec3::new(r, r * 0.65, r), mat::LEAVES_OAK, rng.next_u32(), 0.45);
    }
    let r = rng.range(7.0, 9.0);
    leafy(s, top, top + Vec3::Y * (h * 0.3), Vec3::new(r, r * 0.7, r), mat::LEAVES_OAK, rng.next_u32(), 0.4);
    // Two or three low limbs, heavy and reaching out level a few metres up before they turn
    // up into leaf, as an oak grown old in the open keeps them: what people sit under, and
    // tie ribbons to. Last, so the rest of the tree is as it was.
    for _ in 0..rng.int(2, 3) {
        let (out, _) = rng.heading();
        let start = Vec3::new(0.5, rng.range(5.0, 7.0), 0.5);
        let mid = start + out * rng.range(5.0, 7.0) + Vec3::Y * rng.range(0.0, 1.0);
        let end = mid + out * rng.range(3.5, 5.0) + Vec3::Y * rng.range(2.0, 3.5);
        s.limb(start, mid, 1.1, 0.75, mat::BARK_OAK);
        s.limb(mid, end, 0.75, 0.45, mat::BARK_OAK);
        let fork = rng.range(2.0, 3.0);
        forks(s, rng, end, out, fork, 0.4, mat::BARK_OAK);
        let r = rng.range(4.0, 5.5);
        leafy(s, end, end + Vec3::Y, Vec3::new(r, r * 0.6, r), mat::LEAVES_OAK, rng.next_u32(), 0.45);
    }
}

fn birch(s: &mut Shape, rng: &mut Rng) {
    // From saplings a little over head height in the crown to tall old poles.
    let h = rng.range(10.5, 23.0);
    let size = (h / 16.0).sqrt();
    let lean = rng.range(0.0, 2.0);
    let top = trunk(s, rng, h, 0.75 * size, 0.45, lean, mat::BARK_BIRCH);
    // A light, open crown: slender branches climb steeply out of the upper half of the
    // trunk, turning a golden angle each, every one carrying its own small cluster, so the
    // leaves hang from wood and a bare winter birch is a fan of thin branches.
    // A short one carries a few, a tall one many, and the crown starts higher or lower
    // up the trunk from tree to tree.
    let n = (rng.int(4, 7) + ((size - 0.8) * 6.0) as i32).clamp(4, 10);
    let low = rng.range(0.34, 0.52);
    let a0 = rng.range(0.0, std::f32::consts::TAU);
    for i in 0..n {
        let t = low + (0.9 - low) * i as f32 / (n - 1) as f32 + rng.range(-0.03, 0.03);
        let start = Vec3::new(0.5, 0.0, 0.5).lerp(top, t);
        let angle = a0 + i as f32 * 2.4 + rng.range(-0.4, 0.4);
        let out = Vec3::new(angle.cos(), 0.0, angle.sin());
        // Longer low down, shorter toward the top.
        let len = rng.range(2.6, 4.2) * (1.15 - 0.6 * (t - low));
        let end = start + out * len + Vec3::Y * len * rng.range(0.7, 1.1);
        s.limb(start, end, 0.4, 0.34, mat::BARK_BIRCH);
        if rng.unit() < 0.6 {
            let side = Vec3::new(-out.z, 0.0, out.x) * if rng.unit() < 0.5 { 1.0 } else { -1.0 };
            let from = start.lerp(end, 0.55);
            let tip = from + (out + side * 0.8 + Vec3::Y).normalize_or_zero() * rng.range(1.4, 2.2);
            s.limb(from, tip, 0.34, 0.34, mat::BARK_BIRCH);
            let r = rng.range(1.3, 1.8);
            leafy(s, tip, tip + Vec3::Y * 0.3, Vec3::new(r, r * 0.85, r), mat::LEAVES_BIRCH, rng.next_u32(), 0.55);
        }
        let r = rng.range(1.7, 2.5);
        leafy(s, end, end + Vec3::Y * 0.3, Vec3::new(r, r * 0.85, r), mat::LEAVES_BIRCH, rng.next_u32(), 0.55);
    }
    // The leader runs on a little past the trunk into the top cluster.
    let tip = top + Vec3::Y * 1.6 + Vec3::new(rng.range(-0.4, 0.4), 0.0, rng.range(-0.4, 0.4));
    s.limb(top, tip, 0.45, 0.34, mat::BARK_BIRCH);
    leafy(s, tip, tip, Vec3::new(2.0, 2.3, 2.0), mat::LEAVES_BIRCH, rng.next_u32(), 0.5);
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
    // Its lianas are grown as fine climbers (features::climber), not voxels.
}

fn kapok(s: &mut Shape, rng: &mut Rng) {
    // Rainforest giant with buttress roots, an emergent crown and wide limbs.
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
    // Its lianas are grown as fine climbers (features::climber), not voxels.
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
        leafy(s, end, end + Vec3::Y * 0.5, Vec3::new(r, 1.3, r), mat::LEAVES_ACACIA, rng.next_u32(), 0.45);
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
        leafy(s, end, end, Vec3::new(r, r * 0.8, r), mat::LEAVES_WILLOW, rng.next_u32(), 0.4);
    }
    let r = rng.range(4.0, 5.5);
    leafy(s, top, top + Vec3::Y * 2.0, Vec3::new(r, r * 0.7, r), mat::LEAVES_WILLOW, rng.next_u32(), 0.4);
    // The curtains.
    s.hang(mat::LEAVES_WILLOW, mat::LEAVES_WILLOW, rng, 0.55, (3, 8));
}

/// A willow pollarded by a stream, as the people of a place keep them for their poles: a
/// stout trunk cut back at head height, its head knobbled from the cutting, and a crown of
/// straight young poles standing up out of it in their narrow leaves.
fn pollard(s: &mut Shape, rng: &mut Rng) {
    let h = rng.range(2.2, 3.0);
    let lean = rng.range(0.0, 0.4);
    let top = trunk(s, rng, h, 0.75, 0.6, lean, mat::BARK_WILLOW);
    // The head the poles are cut back to every few years.
    s.blob(top, Vec3::new(1.0, 0.6, 1.0), mat::BARK_WILLOW, rng.next_u32(), 0.35);
    let poles = rng.int(8, 12);
    for k in 0..poles {
        let angle = k as f32 / poles as f32 * std::f32::consts::TAU + rng.range(-0.3, 0.3);
        let out = Vec3::new(angle.cos(), 0.0, angle.sin());
        let from = top + out * rng.range(0.3, 0.8);
        let len = rng.range(2.5, 4.5);
        let end = from + out * (len * rng.range(0.15, 0.35)) + Vec3::Y * len;
        s.limb(from, end, 0.2, 0.12, mat::BARK_WILLOW);
        s.blob(from.lerp(end, 0.7), Vec3::new(0.8, len * 0.35, 0.8), mat::LEAVES_WILLOW, rng.next_u32(), 0.45);
    }
}

/// How high up a mangrove's stilts the tide leaves them dark and wet, in voxels above
/// the mud: about as deep as the water it grows in comes (see `features::mangrove_stand`).
const MANGROVE_TIDE: i32 = 3;

/// A root a voxel thick along `curve` (in blocks, from 0 to 1), each voxel sharing a face
/// with the one before: drawn as a limb, a root that thin breaks up into a string of
/// beads touching at their corners.
fn root_line(s: &mut Shape, curve: impl Fn(f32) -> Vec3, v: Voxel) {
    const STEPS: i32 = 64;
    let at = |k: i32| (curve(k as f32 / STEPS as f32) * OLD_VOXEL).floor().as_ivec3();
    let mut p = at(0);
    s.paint(p, v);
    for k in 1..=STEPS {
        let q = at(k);
        // Out first and down last, so a root plunging into the mud steps the way it bends.
        while p != q {
            if p.x != q.x {
                p.x += (q.x - p.x).signum();
            } else if p.z != q.z {
                p.z += (q.z - p.z).signum();
            } else {
                p.y += (q.y - p.y).signum();
            }
            s.paint(p, v);
        }
    }
}

fn mangrove(s: &mut Shape, rng: &mut Rng) {
    // A red mangrove: a short trunk held up off the mud on a ring of stilt roots, which
    // leave it well above the water and arch out and down, and over it a low, dense dome
    // of small dark leaves, wider than the tree is tall. Squat: the stilts are a low base
    // standing in the water, not legs the tree is held up on.
    let h = rng.range(5.5, 8.0);
    // Where the roots leave the trunk: just above the highest water it stands in.
    let knee = rng.range(1.8, 2.3);
    let lean = rng.range(0.0, 0.6);
    let (dir, _) = rng.heading();
    let axis = Vec3::new(0.5, 0.0, 0.5);
    let top = axis + Vec3::Y * (h * 0.62) + dir * lean;
    // Below the knee the trunk tapers away into the roots: there is no foot to it.
    let foot = axis + Vec3::Y * knee;
    s.limb(foot, top, 0.5, 0.42, mat::BARK_MANGROVE);
    // The stilts, in a ring, each from its own height on the trunk (a few from higher up,
    // as the tree has grown) arching out and plunging into the mud, some forking once on
    // the way down. Quadratic curves drawn in short straight pieces.
    let n = rng.int(6, 8);
    let a0 = rng.range(0.0, std::f32::consts::TAU);
    for i in 0..n {
        let angle = a0 + i as f32 / n as f32 * std::f32::consts::TAU + rng.range(-0.25, 0.25);
        let out = Vec3::new(angle.cos(), 0.0, angle.sin());
        let high = if rng.unit() < 0.3 { rng.range(0.6, 1.6) } else { rng.range(-0.3, 0.4) };
        let from = axis.lerp(top, (knee + high) / (h * 0.62)) + out * 0.3;
        let reach = rng.range(2.4, 3.6) + high * 0.5;
        let end = axis + out * reach - Vec3::Y * 1.5;
        let bend = from + out * (reach * rng.range(0.55, 0.7)) + Vec3::Y * rng.range(0.3, 0.7);
        let arch = |t: f32| from * (1.0 - t) * (1.0 - t) + bend * 2.0 * t * (1.0 - t) + end * t * t;
        // A voxel thick from inside the trunk, so where the roots leave it there is no
        // knot of them; the finer roots dropping from the arches are `features::mangrove`.
        root_line(s, arch, mat::BARK_MANGROVE);
        if rng.unit() < 0.35 {
            // A second foot, off the arch on its way down, landing further out.
            let at = arch(rng.range(0.55, 0.7));
            let side = Vec3::new(-out.z, 0.0, out.x) * rng.range(-0.6, 0.6);
            let land = axis + (out + side) * (reach + rng.range(0.8, 1.4)) - Vec3::Y * 1.5;
            let mid = at.lerp(land, 0.5) + Vec3::Y * 0.4;
            root_line(s, |t| at * (1.0 - t) * (1.0 - t) + mid * 2.0 * t * (1.0 - t) + land * t * t, mat::BARK_MANGROVE);
        }
    }
    // A few stout limbs, nearly level, carry the dome out wide.
    let limbs = rng.int(4, 6);
    let b0 = rng.range(0.0, std::f32::consts::TAU);
    let spread = rng.range(2.3, 3.0);
    for i in 0..limbs {
        let angle = b0 + i as f32 / limbs as f32 * std::f32::consts::TAU + rng.range(-0.3, 0.3);
        let out = Vec3::new(angle.cos(), 0.0, angle.sin());
        let start = top - Vec3::Y * rng.range(0.0, 0.8);
        let end = start + out * spread * rng.range(0.8, 1.1) + Vec3::Y * rng.range(0.8, 1.6);
        s.limb(start, end, 0.36, 0.26, mat::BARK_MANGROVE);
        let r = rng.range(2.1, 2.8);
        s.blob(end + Vec3::Y * 0.4, Vec3::new(r, r * 0.6, r), mat::LEAVES_MANGROVE, rng.next_u32(), 0.3);
    }
    let r = rng.range(3.0, 3.7);
    s.blob(top + Vec3::Y * 1.4, Vec3::new(r, (h - h * 0.62).max(2.4) * 0.75, r), mat::LEAVES_MANGROVE, rng.next_u32(), 0.3);
    // The gloss: here and there a leaf on the top of the dome catches the light. And the
    // stilts are dark with the wet as far as the tide comes up them.
    for p in s.snapshot() {
        match s.get(p) {
            Some(mat::LEAVES_MANGROVE) if s.get(p + IVec3::Y).is_none() && rng.unit() < 0.22 => s.paint(p, mat::LEAVES_MANGROVE_LIT),
            Some(mat::BARK_MANGROVE) if p.y < MANGROVE_TIDE => s.paint(p, mat::BARK_MANGROVE_WET),
            _ => {}
        }
    }
}

fn mangrove_shrub(s: &mut Shape, rng: &mut Rng) {
    // Knee to waist high, dark and dense, in one to three low mounds. Half of them are
    // young mangroves, the mound held up off the mud on a few stilts of their own; the
    // rest a clump of the bushes that grow between the trees, on short stems. Either way
    // the leaves keep clear of the shallow water they may stand in.
    let r = rng.range(1.0, 1.6);
    let stilted = rng.unit() < 0.5;
    let lift = r * 0.55 + if stilted { rng.range(0.9, 1.3) } else { 0.6 };
    let centre = Vec3::new(0.5, lift, 0.5);
    s.blob(centre, Vec3::new(r, r * 0.55, r), mat::LEAVES_MANGROVE, rng.next_u32(), 0.4);
    for _ in 0..rng.int(0, 2) {
        let (dir, _) = rng.heading();
        let r2 = r * rng.range(0.55, 0.8);
        s.blob(centre + dir * r * rng.range(0.7, 1.0) - Vec3::Y * rng.range(0.0, 0.3), Vec3::new(r2, r2 * 0.6, r2), mat::LEAVES_MANGROVE, rng.next_u32(), 0.45);
    }
    if !stilted {
        for _ in 0..rng.int(2, 3) {
            let (dir, _) = rng.heading();
            let base = Vec3::new(0.5, 0.0, 0.5) + dir * rng.range(0.0, r * 0.5);
            root_line(s, |t| base + Vec3::Y * (lift * t - 1.0 * (1.0 - t)), mat::BARK_MANGROVE_WET);
        }
    } else {
        let a0 = rng.range(0.0, std::f32::consts::TAU);
        let n = rng.int(3, 4);
        for i in 0..n {
            let angle = a0 + i as f32 / n as f32 * std::f32::consts::TAU + rng.range(-0.3, 0.3);
            let out = Vec3::new(angle.cos(), 0.0, angle.sin());
            let from = centre - Vec3::Y * 0.2;
            let end = Vec3::new(0.5, -1.0, 0.5) + out * rng.range(0.9, 1.4);
            let bend = from.lerp(end, 0.5) + out * 0.3 + Vec3::Y * 0.3;
            root_line(s, |t| from * (1.0 - t) * (1.0 - t) + bend * 2.0 * t * (1.0 - t) + end * t * t, mat::BARK_MANGROVE_WET);
        }
    }
    for p in s.snapshot() {
        if s.get(p) == Some(mat::LEAVES_MANGROVE) && s.get(p + IVec3::Y).is_none() && rng.unit() < 0.18 {
            s.paint(p, mat::LEAVES_MANGROVE_LIT);
        }
    }
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
    // Mossy boulders are the darker, older rock of damp ground. Their moss is grown as
    // fine pieces (features::moss), not painted voxels, which sat on top as single blocks.
    let dark = if mossy { 0.6 } else { 0.3 };
    let rock = if rng.unit() < dark { mat::ROCK_DARK } else { mat::STONE };
    // Sunk into the ground a little, like something that has sat there a long time.
    let c = Vec3::new(0.5, r * 0.4, 0.5);
    s.blob(c, Vec3::new(r * rng.range(0.8, 1.2), r * 0.8, r * rng.range(0.8, 1.2)), rock, rng.next_u32(), 0.3);
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
    // Moss on logs is grown as fine pieces (features::moss), not painted voxels.
    s.drop_air();
}

fn stump(s: &mut Shape, rng: &mut Rng) {
    let h = rng.range(1.0, 2.5);
    s.limb(Vec3::new(0.5, -2.0, 0.5), Vec3::new(0.5, h, 0.5), rng.range(1.0, 1.6), 1.0, mat::BARK_OAK);
}

// --- Evergreens: trees and shrubs that keep their leaves through the winter. ---

/// A stack of leaf masses up a stem, each as wide as `radius` says for its share of the
/// way up (0 at the foot of the crown, 1 at its tip) and nudged off the stem a little, so
/// the crown is one dense body with a lumpy outline rather than a turned shape.
#[allow(clippy::too_many_arguments)]
fn column_crown(s: &mut Shape, rng: &mut Rng, foot: Vec3, tip: Vec3, step: f32, wobble: f32, leaves: Voxel, ragged: f32, radius: impl Fn(f32) -> f32) {
    let n = ((tip.y - foot.y) / step).ceil().max(1.0) as i32;
    for i in 0..=n {
        let t = i as f32 / n as f32;
        let r = radius(t);
        let off = Vec3::new(rng.range(-wobble, wobble), 0.0, rng.range(-wobble, wobble)) * (1.0 - t);
        s.blob(foot.lerp(tip, t) + off, Vec3::new(r, step * 1.1, r), leaves, rng.next_u32(), ragged);
    }
}

fn holly(s: &mut Shape, rng: &mut Rng) {
    // An understorey tree: a smooth grey stem lost inside a dense dark crown that comes
    // nearly to the ground, widest low down and drawn up to a point.
    let h = rng.range(7.0, 11.5);
    let lean = rng.range(0.0, 0.8);
    let top = trunk(s, rng, h * 0.9, 0.5, 0.3, lean, mat::BARK_HOLLY);
    let skirt = rng.range(1.0, 1.8);
    let r = rng.range(1.9, 2.7);
    let foot = Vec3::new(0.5, skirt, 0.5).lerp(top, skirt / h);
    column_crown(s, rng, foot, top + Vec3::Y * (h * 0.1), 0.9, 0.5, mat::LEAVES_HOLLY, 0.45, |t| {
        // Quickly out to full width, then a long taper.
        let rise = (t / 0.22).min(1.0);
        r * (0.55 + 0.45 * rise) * (1.0 - 0.8 * ((t - 0.22).max(0.0) / 0.78).powf(1.3)) + 0.25
    });
    // A bough or two pushing out of the outline low down.
    for _ in 0..rng.int(1, 3) {
        let (dir, _) = rng.heading();
        let at = foot.lerp(top, rng.range(0.1, 0.45));
        let end = at + dir * (r * rng.range(0.8, 1.1)) + Vec3::Y * rng.range(-0.3, 0.4);
        s.limb(at, end, 0.34, 0.3, mat::BARK_HOLLY);
        let b = rng.range(1.0, 1.5);
        s.blob(end, Vec3::new(b, b * 0.8, b), mat::LEAVES_HOLLY, rng.next_u32(), 0.5);
    }
}

fn yew(s: &mut Shape, rng: &mut Rng) {
    // Old, dark and squat: a short trunk of several fused stems, and a crown broader than
    // it is tall, in heavy layers that sweep down almost to the ground at their tips.
    let h = rng.range(7.5, 10.5);
    let fork = Vec3::new(0.5, h * rng.range(0.3, 0.38), 0.5);
    let stems = rng.int(2, 4);
    let a0 = rng.range(0.0, std::f32::consts::TAU);
    let mut tops = Vec::new();
    for i in 0..stems {
        let angle = a0 + i as f32 / stems as f32 * std::f32::consts::TAU + rng.range(-0.3, 0.3);
        let out = Vec3::new(angle.cos(), 0.0, angle.sin());
        // The stems stand shoulder to shoulder at the foot, making one fluted bole.
        let base = Vec3::new(0.5, -3.0, 0.5) + out * 0.7;
        let top = fork + out * rng.range(1.2, 2.2) + Vec3::Y * rng.range(0.0, 1.5);
        s.limb(base, top, rng.range(0.85, 1.15), 0.6, mat::BARK_YEW);
        tops.push((top, out));
    }
    // The skirt: long level boughs, each a wide flat mass drooping at the tip.
    let boughs = rng.int(6, 8);
    for i in 0..boughs {
        let angle = a0 + 0.4 + i as f32 / boughs as f32 * std::f32::consts::TAU + rng.range(-0.3, 0.3);
        let out = Vec3::new(angle.cos(), 0.0, angle.sin());
        let start = fork + Vec3::Y * rng.range(-0.5, 1.0);
        let len = rng.range(4.4, 6.0);
        let end = start + out * len - Vec3::Y * rng.range(0.5, 1.6);
        s.limb(start, end, 0.6, 0.35, mat::BARK_YEW);
        let r = rng.range(2.8, 3.6);
        s.blob(start.lerp(end, 0.55) + Vec3::Y * 0.4, Vec3::new(r, 1.1, r), mat::LEAVES_YEW, rng.next_u32(), 0.4);
        s.blob(end, Vec3::new(r * 0.85, 1.2, r * 0.85), mat::LEAVES_YEW, rng.next_u32(), 0.45);
    }
    // Over it, the stems carry on up into a low dome with a flat top, never a point.
    for (top, out) in tops {
        let end = top + out * rng.range(1.5, 2.5) + Vec3::Y * rng.range(1.6, 2.6);
        s.limb(top, end, 0.6, 0.35, mat::BARK_YEW);
        let r = rng.range(3.0, 4.0);
        s.blob(end, Vec3::new(r, r * 0.5, r), mat::LEAVES_YEW, rng.next_u32(), 0.4);
    }
    let r = rng.range(3.8, 4.8);
    s.blob(fork + Vec3::Y * (h * 0.36), Vec3::new(r, r * 0.55, r), mat::LEAVES_YEW, rng.next_u32(), 0.4);
    let r = rng.range(2.6, 3.4);
    s.blob(Vec3::new(0.5, h - 1.2, 0.5), Vec3::new(r, 1.3, r), mat::LEAVES_YEW, rng.next_u32(), 0.45);
}

fn holm_oak(s: &mut Shape, rng: &mut Rng) {
    // The evergreen oak: a short stout trunk under one tight, rounded head of grey-green,
    // dense to the edge, where the summer oak is a loose gathering of separate masses.
    let h = rng.range(11.0, 16.0);
    let r0 = rng.range(0.95, 1.35);
    let lean = rng.range(0.0, 1.0);
    let top = trunk(s, rng, h * 0.34, r0, 0.85, lean, mat::BARK_OAK);
    let r = h * rng.range(0.36, 0.42);
    let heart = top + Vec3::Y * (r * 0.78);
    // Heavy limbs rising steeply through the head, which hides all but their feet.
    let limbs = rng.int(3, 5);
    let a0 = rng.range(0.0, std::f32::consts::TAU);
    for i in 0..limbs {
        let angle = a0 + i as f32 / limbs as f32 * std::f32::consts::TAU + rng.range(-0.3, 0.3);
        let out = Vec3::new(angle.cos(), 0.0, angle.sin());
        let end = top + out * (r * rng.range(0.5, 0.7)) + Vec3::Y * (r * rng.range(0.6, 1.0));
        s.limb(top - Vec3::Y * rng.range(0.0, 1.0), end, 0.7, 0.4, mat::BARK_OAK);
        // The head billows: a smaller dome on the shoulder over each limb.
        let b = r * rng.range(0.5, 0.62);
        let at = heart + out * (r * rng.range(0.55, 0.7)) + Vec3::Y * (r * rng.range(-0.3, 0.25));
        s.blob(at, Vec3::new(b, b * 0.85, b), mat::LEAVES_HOLM, rng.next_u32(), 0.3);
    }
    s.blob(heart, Vec3::new(r, r * 0.82, r), mat::LEAVES_HOLM, rng.next_u32(), 0.3);
}

fn scots_pine(s: &mut Shape, rng: &mut Rng) {
    // A tall bare trunk, grey and plated below and flaking to orange above, carrying a
    // flat, open top of a few level plates of needles on crooked limbs.
    let h = rng.range(19.0, 29.0);
    let r0 = rng.range(0.85, 1.2);
    let (lean_dir, _) = rng.heading();
    let lean = rng.range(0.0, 1.5);
    let axis = |y: f32| Vec3::new(0.5, y, 0.5) + lean_dir * lean * (y / h).max(0.0).powi(2);
    let turn = h * rng.range(0.4, 0.52);
    let r_turn = r0 * 0.78;
    s.limb(axis(-3.0), axis(turn), r0, r_turn, mat::BARK_PINE);
    let mut prev = (axis(turn), r_turn);
    for i in 1..=3 {
        let y = turn + (h - turn) * i as f32 / 3.0;
        let r = r_turn + (0.45 - r_turn) * i as f32 / 3.0;
        s.limb(prev.0, axis(y), prev.1, r, mat::BARK_SCOTS);
        prev = (axis(y), r);
    }
    // The stubs of branches it shaded out on the way up.
    for _ in 0..rng.int(1, 3) {
        let (dir, _) = rng.heading();
        let at = axis(h * rng.range(0.35, 0.6));
        s.limb(at, at + dir * rng.range(1.2, 2.2) + Vec3::Y * rng.range(0.0, 0.6), 0.36, 0.3, mat::WOOD_DEAD);
    }
    let limbs = rng.int(4, 6);
    let a0 = rng.range(0.0, std::f32::consts::TAU);
    for i in 0..limbs {
        let t = 0.64 + 0.3 * i as f32 / (limbs - 1) as f32 + rng.range(-0.02, 0.02);
        let angle = a0 + i as f32 * 2.4 + rng.range(-0.4, 0.4);
        let out = Vec3::new(angle.cos(), 0.0, angle.sin());
        let start = axis(h * t);
        // Longest low in the crown, each kinked upward at its elbow.
        let len = rng.range(3.0, 4.6) * (1.2 - 0.55 * (t - 0.64) / 0.3);
        let elbow = start + out * (len * 0.55) + Vec3::Y * rng.range(0.2, 0.8);
        let end = elbow + out * (len * 0.45) + Vec3::Y * rng.range(0.8, 1.8);
        s.limb(start, elbow, 0.5, 0.4, mat::BARK_SCOTS);
        s.limb(elbow, end, 0.4, 0.32, mat::BARK_SCOTS);
        let r = rng.range(2.4, 3.2);
        s.blob(end + Vec3::Y * 0.5, Vec3::new(r, rng.range(0.9, 1.25), r), mat::NEEDLES_PINE, rng.next_u32(), 0.45);
    }
    let r = rng.range(3.0, 4.2);
    s.blob(axis(h) + Vec3::Y * 0.6, Vec3::new(r, 1.4, r), mat::NEEDLES_PINE, rng.next_u32(), 0.45);
}

fn juniper(s: &mut Shape, rng: &mut Rng) {
    // A narrow blue-grey flame on open ground, often two or three spires from one root.
    let spires = if rng.unit() < 0.45 { 1 } else { rng.int(2, 3) };
    let a0 = rng.range(0.0, std::f32::consts::TAU);
    for i in 0..spires {
        let h = rng.range(4.0, 8.0) * if i == 0 { 1.0 } else { rng.range(0.5, 0.8) };
        let angle = a0 + i as f32 * 2.3;
        let out = Vec3::new(angle.cos(), 0.0, angle.sin());
        let foot = Vec3::new(0.5, 0.0, 0.5) + out * if i == 0 { 0.0 } else { rng.range(0.9, 1.4) };
        let tip = foot + Vec3::Y * h + out * rng.range(0.0, 0.5);
        s.limb(foot - Vec3::Y * 3.0, foot.lerp(tip, 0.6), 0.34, 0.3, mat::BARK_YEW);
        let r = rng.range(1.1, 1.5) * (0.75 + 0.04 * h);
        column_crown(s, rng, foot + Vec3::Y * 0.5, tip, 0.8, 0.25, mat::NEEDLES_JUNIPER, 0.3, |t| {
            r * (0.6 + 0.4 * (t / 0.3).min(1.0)) * (1.0 - 0.85 * ((t - 0.3).max(0.0) / 0.7).powf(1.6)) + 0.2
        });
    }
}

fn holly_bush(s: &mut Shape, rng: &mut Rng) {
    // A dense dark mound, taller than it is wide.
    let r = rng.range(1.2, 1.9);
    let h = r * rng.range(1.25, 1.6);
    s.limb(Vec3::new(0.5, -1.0, 0.5), Vec3::new(0.5, h, 0.5), 0.32, 0.3, mat::BARK_HOLLY);
    s.blob(Vec3::new(0.5, h * 0.8, 0.5), Vec3::new(r, h, r), mat::LEAVES_HOLLY, rng.next_u32(), 0.45);
    if rng.unit() < 0.6 {
        let (dir, _) = rng.heading();
        let r2 = r * rng.range(0.55, 0.75);
        s.blob(Vec3::new(0.5, r2 * 0.8, 0.5) + dir * r * 0.9, Vec3::new(r2, r2 * 1.1, r2), mat::LEAVES_HOLLY, rng.next_u32(), 0.5);
    }
}

fn laurel(s: &mut Shape, rng: &mut Rng) {
    // Broad and low, in glossy overlapping domes: a thicket more than a bush.
    let r = rng.range(1.8, 2.6);
    s.blob(Vec3::new(0.5, r * 0.45, 0.5), Vec3::new(r, r * 0.62, r), mat::LEAVES_LAUREL, rng.next_u32(), 0.35);
    let a0 = rng.range(0.0, std::f32::consts::TAU);
    for i in 0..rng.int(2, 3) {
        let angle = a0 + i as f32 * 2.2 + rng.range(-0.3, 0.3);
        let out = Vec3::new(angle.cos(), 0.0, angle.sin());
        let r2 = r * rng.range(0.6, 0.85);
        s.blob(Vec3::new(0.5, r2 * 0.45, 0.5) + out * r * rng.range(0.7, 1.0), Vec3::new(r2, r2 * 0.65, r2), mat::LEAVES_LAUREL, rng.next_u32(), 0.35);
    }
}

fn gorse(s: &mut Shape, rng: &mut Rng) {
    // A ragged thicket of spiny shoots: several small upright masses, open and prickly at
    // the edge. Its flowers are grown as fine pieces (features::tree_detail), not voxels.
    let a0 = rng.range(0.0, std::f32::consts::TAU);
    let n = rng.int(3, 5);
    for i in 0..n {
        let angle = a0 + i as f32 / n as f32 * std::f32::consts::TAU + rng.range(-0.5, 0.5);
        let out = Vec3::new(angle.cos(), 0.0, angle.sin()) * if i == 0 { 0.0 } else { rng.range(0.8, 1.7) };
        let r = rng.range(0.8, 1.3);
        let h = r * rng.range(1.0, 1.5);
        s.blob(Vec3::new(0.5, h * 0.7, 0.5) + out, Vec3::new(r, h, r), mat::GORSE, rng.next_u32(), 0.65);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oaks_and_birches_come_short_and_tall() {
        for species in [Species::Oak, Species::Birch] {
            let models: Vec<Template> = (0..species.variants()).map(|i| species.build(0x51_7e3d ^ (i as u32).wrapping_mul(0x9e37_79b9))).collect();
            let tops: Vec<i32> = models.iter().map(|t| t.top).collect();
            let reach: Vec<i32> = models.iter().map(|t| t.reach).collect();
            let (short, tall) = (*tops.iter().min().unwrap(), *tops.iter().max().unwrap());
            assert!(tall * 10 >= short * 16, "{species:?} only spans {short} to {tall} voxels in height");
            // Chunks only look this far for trees: wider, and a crown is cut off at a border.
            let widest = *reach.iter().max().unwrap();
            assert!(widest <= super::super::TREES.reach, "{species:?} reaches {widest}");
        }
    }

    #[test]
    fn mangroves_stand_on_stilts_above_the_water() {
        let lib = super::super::Library::build();
        for variant in 0..Species::Mangrove.variants() {
            let t = lib.get(Species::Mangrove, variant);
            assert!(t.reach <= super::super::TREES.reach, "mangrove {variant} reaches {}", t.reach);
            // Over the deepest water it stands in, nothing but its roots: no trunk in the
            // middle and no leaves down to it.
            let deepest = super::super::MANGROVE_DEEPEST.ceil() as i32;
            let low: Vec<_> = t.voxels.iter().filter(|(p, _)| (0..deepest).contains(&p.y)).collect();
            assert!(low.iter().all(|(_, v)| matches!(*v, mat::BARK_MANGROVE | mat::BARK_MANGROVE_WET)), "mangrove {variant} has leaves near the water");
            assert!(!low.iter().any(|(p, _)| p.x.abs() <= 1 && p.z.abs() <= 1), "mangrove {variant} has a foot");
            // And those roots come down in a ring round it, on every side.
            let sides = low.iter().fold([false; 4], |mut s, (p, _)| {
                s[(p.x > 0) as usize * 2 + (p.z > 0) as usize] = true;
                s
            });
            assert!(sides.iter().all(|s| *s), "mangrove {variant} stands on one side only");
            assert!(low.iter().any(|(p, _)| p.x.abs().max(p.z.abs()) >= 5), "mangrove {variant}'s roots keep to the trunk");
        }
    }
}
