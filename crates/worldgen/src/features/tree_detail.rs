//! Fine detail on trees, in 0.1 m pieces: bark relief on trunks, twigs on limbs and at
//! the edges of evergreen canopies, and leaf clusters that soften the outer faces of
//! foliage. It is laid over the voxel trees and keeps them reading as voxel trees.
//!
//! - **Bark** grows on the lower trunk only, where people look: ridges and plates that
//!   follow the species (furrowed oak, deep-ridged redwood, scaly pine, birch with dark
//!   dashes and peeling strips, ringed palm). Patterns come from world coordinates, so they
//!   run unbroken up a trunk and across voxels, and merge into long boxes.
//! - **Twigs** stick out of limbs, never trunks, so bare winter trees show them too, and
//!   out of evergreen foliage, whose leaves never fall from around them. Not palms: a
//!   palm has no branches, only fronds on a bare trunk.
//! - **Leaf clusters** hang under canopies and tuft their sides, in the leaf's own
//!   material, so they turn, fall and sway exactly as the leaves they sit on.
//! - **Fruit**: holly berries, wild cherries and acorns among the leaves, each keeping a
//!   row of the table of seasons (see [`seasons`]), so they come green, ripen and are
//!   gone in their own months without the world being built again.
//! - **Bare crowns**: a broadleaf's model carries fine twigs inside its leaves
//!   (`Template::fine`, grown by `trees::twigs`), hidden while the leaves are on and
//!   showing as each falls. [`bare_crowns`] lays them into the world.
//!
//! Every piece is decided from the air voxel it occupies and that voxel's neighbours,
//! which a chunk column has in its border, so neighbouring columns agree. Each column
//! has fixed budgets of sites and boxes, spent on the lowest faces first, as the plants'
//! draw distance limits how far away any of it is drawn.

use std::collections::HashSet;

use glam::IVec3;
use voxel::{
    AIR, CHUNK, Materials, PAD, Voxel,
    mesh::{CELLS, PlantBox},
};

use super::attach::{ColumnVoxels, FacePlot, Raster, Site, Toward, Voxels, for_each_site};
use super::plants::Season;
use super::shape::{FACES, ON_WOOD, Rng, boxes_meet, on_faces};
use super::trees::{Species, hides_twigs};
use super::{Library, Placement, parts::rotate_box};
use crate::{
    materials as mat,
    noise::{hash2, hash3, unit},
    scale::m,
    surface::Column,
};

/// Trunk faces given bark relief per chunk column, lowest first.
pub const BARK_SITES: usize = 220;
/// Twigs per chunk column.
pub const TWIG_SITES: usize = 48;
/// Most boxes of bark and limb twigs in one chunk column: at 5 quads a box, about 50 KB.
pub const WOOD_BOXES: usize = 320;
/// Foliage faces given a leaf cluster or a twig per chunk column, lowest first.
pub const LEAF_SITES: usize = 150;
/// Most boxes of leaf clusters and canopy twigs in one chunk column.
pub const LEAF_BOXES: usize = 400;

// Rows of the table of seasons the trees' fruit keeps, after the crops (55 to 63; see
// `plants::seasons`), 64 to 71.
/// Wild cherries: green from May, dark red from midsummer, taken by the birds by August.
pub const CHERRIES: u8 = 64;
/// Acorns, and a holm oak's: green from July, brown in September, all fallen before the
/// leaves are, so none hangs from a bare twig.
pub const ACORNS: u8 = 65;
/// Holly berries: green through the summer, red from October, and the last eaten in
/// February; a holly is without them through the spring.
pub const HOLLY: u8 = 66;

/// The seasons the trees' fruit keeps, each with its row of the table. In bud a fruit
/// is green and unripe, out it is ripe, and gone to seed it browns and falls.
pub fn seasons() -> [(u8, Season); 3] {
    let s = |start: f32, length: f32, bud: f32, seed: f32| Season { start, length, bud, seed };
    [(CHERRIES, s(0.27, 0.06, 0.10, 0.015)), (ACORNS, s(0.47, 0.08, 0.14, 0.005)), (HOLLY, s(0.56, 0.36, 0.26, 0.0))]
}

/// The row of the table of seasons a piece of fruit keeps, or 0 for anything else.
pub fn fruit_season(v: Voxel) -> u8 {
    match v {
        mat::CHERRY => CHERRIES,
        mat::ACORN => ACORNS,
        mat::HOLLY_BERRY => HOLLY,
        _ => 0,
    }
}

/// The season of a fruit that can be picked, if `v` is one: it can be taken only while
/// it is ripe, as the shader draws it.
pub fn fruit(v: Voxel) -> Option<Season> {
    let row = fruit_season(v);
    seasons().into_iter().find(|&(r, _)| r == row && row != 0).map(|(_, s)| s)
}

/// Share of the outer faces of limbs that grow a twig.
const LIMB_TWIGS: f32 = 0.08;
/// Share of the outer faces of evergreen foliage that show a twig.
const CANOPY_TWIGS: f32 = 0.05;

/// Whether a species has anything for this module to add, so a column without any can
/// skip the work.
pub fn has_detail(species: Species) -> bool {
    !matches!(species, Species::Boulder | Species::MossyBoulder | Species::Cactus | Species::FallenLog | Species::Stump)
}

/// How a species' bark is textured.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Bark {
    /// Vertical ridges `width` cells wide every `period` cells, cracked across into
    /// plates on a share `crack` of the gaps between them.
    Furrowed { ridge: Voxel, period: i32, width: i32, crack: f32 },
    /// Redwood: broad ridges in long straight runs, some standing out twice as far.
    Deep,
    /// Pine: scaly plates in staggered rows.
    Plates,
    /// Birch: dark dashes across the trunk, and strips peeling away.
    Peeling,
    /// Palm: the rings left by fallen fronds.
    Rings,
}

impl Bark {
    /// How far up the trunk the relief reaches, in voxels above the ground.
    fn reach(self) -> i32 {
        match self {
            Bark::Rings => m(7.0) as i32,
            Bark::Deep => m(9.0) as i32,
            _ => m(5.0) as i32,
        }
    }
}

/// The relief of a bark, or `None` for smooth bark (baobab) and anything that is not bark.
fn bark_of(v: Voxel) -> Option<Bark> {
    let furrowed = |ridge, period, width, crack| Bark::Furrowed { ridge, period, width, crack };
    Some(match v {
        mat::BARK_OAK => furrowed(mat::BARK_OAK_RIDGE, 3, 2, 0.3),
        mat::BARK_WILLOW => furrowed(mat::BARK_WILLOW, 3, 2, 0.15),
        mat::BARK_ACACIA => furrowed(mat::BARK_ACACIA, 4, 2, 0.35),
        mat::BARK_JUNGLE => furrowed(mat::BARK_JUNGLE, 5, 1, 0.05),
        mat::WOOD_DEAD => furrowed(mat::WOOD_DEAD, 4, 1, 0.2),
        mat::BARK_REDWOOD => Bark::Deep,
        mat::BARK_PINE => Bark::Plates,
        mat::BARK_BIRCH => Bark::Peeling,
        mat::BARK_PALM => Bark::Rings,
        // Yew flakes away in long plates; a pine's upper trunk in broad papery ones.
        mat::BARK_YEW => furrowed(mat::BARK_YEW_FLAKE, 4, 2, 0.4),
        mat::BARK_SCOTS => furrowed(mat::BARK_SCOTS, 4, 3, 0.5),
        _ => return None,
    })
}

/// Tree wood: what bark and limb twigs grow on.
pub(super) fn is_wood(v: Voxel) -> bool {
    bark_of(v).is_some() || matches!(v, mat::BARK_BAOBAB | mat::BARK_HOLLY)
}

/// The twigs a wood grows.
fn twig_of(wood: Voxel) -> Voxel {
    match wood {
        mat::BARK_BIRCH | mat::WOOD_DEAD | mat::BARK_HOLLY => mat::TWIG_PALE,
        _ => mat::TWIG,
    }
}

/// Tree foliage: what leaf clusters grow on.
fn is_leaf(v: Voxel) -> bool {
    matches!(
        v,
        mat::LEAVES_OAK
            | mat::LEAVES_BIRCH
            | mat::NEEDLES
            | mat::NEEDLES_REDWOOD
            | mat::LEAVES_JUNGLE
            | mat::LEAVES_ACACIA
            | mat::LEAVES_WILLOW
            | mat::LEAVES_PALM
            | mat::LEAVES_MAPLE
            | mat::LEAVES_MAPLE_RED
            | mat::LEAVES_CHERRY
            | mat::LEAVES_BAOBAB
            | mat::LEAVES_HOLLY
            | mat::LEAVES_YEW
            | mat::LEAVES_HOLM
            | mat::NEEDLES_PINE
            | mat::NEEDLES_JUNIPER
            | mat::LEAVES_LAUREL
            | mat::GORSE
            | mat::LEAVES_MANGROVE
            | mat::LEAVES_MANGROVE_LIT
    )
}

/// Foliage that stays on all year, so a twig poking out of it never hangs in empty air.
fn is_evergreen(v: Voxel) -> bool {
    matches!(
        v,
        mat::NEEDLES
            | mat::NEEDLES_REDWOOD
            | mat::LEAVES_JUNGLE
            | mat::LEAVES_PALM
            | mat::LEAVES_BAOBAB
            | mat::LEAVES_HOLLY
            | mat::LEAVES_YEW
            | mat::LEAVES_HOLM
            | mat::NEEDLES_PINE
            | mat::NEEDLES_JUNIPER
            | mat::LEAVES_LAUREL
            | mat::GORSE
            | mat::LEAVES_MANGROVE
            | mat::LEAVES_MANGROVE_LIT
    )
}

/// The cell at world cell `(wu, wy)` of a trunk face: how far it stands out and what of,
/// or `None` where the bare trunk shows. `wu` runs across the face, `wy` up it.
fn bark_cell(bark: Bark, seed: u32, wu: i32, wy: i32) -> Option<(i32, Voxel)> {
    let roll = |salt: u32, a: i32, b: i32| unit(hash2(seed ^ salt, a, b));
    match bark {
        Bark::Furrowed { ridge, period, width, crack } => {
            // The ridges wander a cell sideways every 0.6 m or so.
            let shift = (hash2(seed ^ 0xba01, wy.div_euclid(6), period) & 1) as i32;
            let k = (wu + shift).rem_euclid(period);
            if k >= width {
                return None;
            }
            let plate = (wu + shift).div_euclid(period);
            if wy.rem_euclid(5) == 0 && roll(0xba02, plate, wy.div_euclid(5)) < crack {
                return None;
            }
            Some((1, ridge))
        }
        Bark::Deep => {
            let shift = (hash2(seed ^ 0xba03, wy.div_euclid(14), 0) & 1) as i32;
            let k = (wu + shift).rem_euclid(4);
            if k == 3 {
                return None;
            }
            let ridge = (wu + shift).div_euclid(4);
            if wy.rem_euclid(9) == 0 && roll(0xba04, ridge, wy.div_euclid(9)) < 0.12 {
                return None;
            }
            let deep = k == 1 && roll(0xba05, ridge, wy.div_euclid(18)) < 0.45;
            Some((if deep { 2 } else { 1 }, mat::BARK_REDWOOD_RIDGE))
        }
        Bark::Plates => {
            let row = wy.div_euclid(3);
            let off = (row & 1) * 2;
            let k = (wu + off).rem_euclid(4);
            if k == 0 || wy.rem_euclid(3) == 0 {
                return None;
            }
            (roll(0xba06, (wu + off).div_euclid(4), row) < 0.8).then_some((1, mat::BARK_PINE_PLATE))
        }
        Bark::Peeling => {
            // Lenticels: short dark dashes across the trunk.
            let sh = (hash2(seed ^ 0xba07, wy, 0) % 5) as i32;
            if (wu + sh).rem_euclid(5) < 3 && roll(0xba08, (wu + sh).div_euclid(5), wy) < 0.09 {
                return Some((1, mat::BARK_BIRCH_MARK));
            }
            // Strips peeling away, two cells tall, their free end curling out.
            let row = wy.div_euclid(2);
            let sh = (hash2(seed ^ 0xba09, row, 0) % 6) as i32;
            let k = (wu + sh).rem_euclid(6);
            if k < 4 && roll(0xba0a, (wu + sh).div_euclid(6), row) < 0.07 {
                let curl = k == 3 && wy.rem_euclid(2) == 1;
                return Some((if curl { 2 } else { 1 }, mat::BARK_BIRCH_PEEL));
            }
            None
        }
        Bark::Rings => {
            if wy.rem_euclid(3) != 0 {
                return None;
            }
            (roll(0xba0b, wu.div_euclid(4), wy) > 0.12).then_some((1, mat::BARK_PALM_RING))
        }
    }
}

/// Whether the wood at `host` is an upright trunk rather than a limb or a log: the same
/// bark carries on above and below it.
fn is_trunk(voxels: &ColumnVoxels, host: IVec3, bark: Voxel) -> bool {
    [-3, -1, 1, 3].iter().all(|&k| voxels.voxel(host + IVec3::Y * k) == bark)
}

/// What a chunk column's trees grow on, and what to keep clear of.
pub struct Grove<'a> {
    pub columns: &'a [Column],
    /// World position of the padded corner of `columns`.
    pub x0: i32,
    pub z0: i32,
    pub seed: u32,
    /// Air voxels already holding plants, climbers or moss.
    pub taken: &'a HashSet<IVec3>,
}

impl Grove<'_> {
    /// Height of the air voxel `p` above the ground of its column.
    fn above_ground(&self, p: IVec3) -> i32 {
        let (px, pz) = ((p.x - self.x0) as usize, (p.z - self.z0) as usize);
        p.y - self.columns[px * PAD + pz].top()
    }

    fn rng(&self, site: &Site, salt: u32) -> Rng {
        Rng::new(hash3(self.seed ^ salt, site.air.x, site.air.y * 7 + site.toward as i32, site.air.z))
    }
}

/// One face to grow on, with the order it is wanted in: lowest first.
struct Job {
    site: Site,
    key: i32,
}

/// Grows the detail on the trees of a chunk column, whose voxels `voxels` reads.
pub fn grow(voxels: &ColumnVoxels, mats: &Materials, grove: &Grove, out: &mut Vec<PlantBox>) {
    let seed = grove.seed;
    // Bark on lower trunks, twigs on limbs.
    let (mut bark, mut limb_twigs) = (Vec::new(), Vec::new());
    for_each_site(voxels, mats, is_wood, |site| {
        if grove.taken.contains(&site.air) {
            return;
        }
        let h = grove.above_ground(site.air);
        let host = site.air + site.toward.step();
        if site.toward.is_side() && is_trunk(voxels, host, site.host) {
            let Some(relief) = bark_of(site.host) else { return };
            // A ragged top, so the relief does not stop in a level line.
            let ragged = (hash2(seed ^ 0xba10, site.air.x, site.air.z) % 5) as i32;
            if h >= 1 && h <= relief.reach() + ragged {
                bark.push(Job { site: *site, key: h });
            }
        } else if h >= m(3.0) as i32 && site.toward != Toward::Above && site.host != mat::BARK_PALM {
            let roll = unit(hash3(seed ^ 0xba11, site.air.x, site.air.y * 7 + site.toward as i32, site.air.z));
            if roll < LIMB_TWIGS {
                limb_twigs.push(Job { site: *site, key: (roll * 1e6) as i32 });
            }
        }
    });
    bark.sort_by_key(|j| j.key);
    bark.truncate(BARK_SITES);
    limb_twigs.sort_by_key(|j| j.key);
    limb_twigs.truncate(TWIG_SITES);

    let mut wood = Raster::new();
    for job in &bark {
        let site = &job.site;
        let relief = bark_of(site.host).expect("bark sites are on bark");
        let across = match site.toward {
            Toward::PosX | Toward::NegX => site.air.z,
            _ => site.air.x,
        } * CELLS;
        let up = site.air.y * CELLS;
        let mut plot = site.plot(&mut wood);
        for v in 0..CELLS {
            for u in 0..CELLS {
                if let Some((d, voxel)) = bark_cell(relief, seed, across + u, up + v) {
                    plot.fill(u, u + 1, v, v + 1, 0, d, voxel);
                }
            }
        }
    }
    for job in &limb_twigs {
        let mut rng = grove.rng(&job.site, 0xba12);
        twig(&mut job.site.plot(&mut wood), &mut rng, twig_of(job.site.host));
    }
    let used: HashSet<IVec3> = wood.voxels().collect();
    let start = out.len();
    wood.finish(out, |_| (false, false));
    if out.len() - start > WOOD_BOXES {
        // Thin evenly, so no part of the column is left bare.
        let mut mine: Vec<PlantBox> = out.drain(start..).collect();
        mine.sort_unstable_by_key(|b| (hash3(seed ^ 0xba13, b.lo[0], b.lo[1], b.lo[2]), b.lo, b.hi, b.voxel));
        mine.truncate(WOOD_BOXES);
        out.append(&mut mine);
    }

    // Leaf clusters, and twigs in evergreen foliage.
    let mut leaves = Vec::new();
    leaf_sites(voxels, mats, |site| {
        if grove.taken.contains(&site.air) || used.contains(&site.air) {
            return;
        }
        let h = grove.above_ground(site.air);
        // The tops of canopies are only seen from above, except on bushes.
        if site.toward == Toward::Below && h > m(3.0) as i32 {
            return;
        }
        let roll = unit(hash3(seed ^ 0xba14, site.air.x, site.air.y, site.air.z));
        let chance = match site.toward {
            Toward::Above => 0.5,
            Toward::Below => 0.25,
            _ => 0.4,
        };
        if roll < chance {
            leaves.push(Job { site, key: h + (hash3(seed ^ 0xba15, site.air.x, site.air.y, site.air.z) % 6) as i32 });
        }
    });
    leaves.sort_by_key(|j| j.key);
    leaves.truncate(LEAF_SITES);
    let mut count = 0;
    let mut piece = Vec::new();
    for job in &leaves {
        let site = &job.site;
        let mut rng = grove.rng(site, 0xba16);
        // Each voxel's pieces are finished on their own, so no box spans two voxels: a
        // falling leaf drops whole voxels at a time, and every corner of a box must agree.
        let mut raster = Raster::new();
        let mut plot = site.plot(&mut raster);
        if is_evergreen(site.host) && site.host != mat::LEAVES_PALM && site.toward != Toward::Below && rng.unit() < CANOPY_TWIGS {
            twig(&mut plot, &mut rng, mat::TWIG);
        } else {
            // Laid first, as a cell once taken is kept: the leaves then gather round them.
            let tree = grove.above_ground(site.air) > m(2.5) as i32;
            fruit_of(&mut plot, &mut grove.rng(site, 0xba17), site.host, site.toward, tree);
            garnish(&mut plot, &mut rng, site.host);
            cluster(&mut plot, &mut rng, site.toward == Toward::Above, site.host);
        }
        piece.clear();
        raster.finish(&mut piece, |_| (true, true));
        for b in &mut piece {
            b.season = fruit_season(b.voxel);
        }
        if count + piece.len() > LEAF_BOXES {
            break;
        }
        count += piece.len();
        out.append(&mut piece);
    }
}

/// A twig: a stick 0.1 m thick standing 0.2 to 0.4 m out of the face, often forked.
fn twig(plot: &mut FacePlot, rng: &mut Rng, wood: Voxel) {
    let (u, v) = (rng.int(1, 2), rng.int(1, 2));
    let len = rng.int(2, 4);
    plot.fill(u, u + 1, v, v + 1, 0, len, wood);
    if rng.unit() < 0.6 {
        let at = rng.int(1, len - 1);
        let side = if rng.unit() < 0.5 { 1 } else { -1 };
        let (u2, v2) = if rng.unit() < 0.5 { (u + side, v) } else { (u, v + side) };
        plot.fill(u2, u2 + 1, v2, v2 + 1, at, at + 2, wood);
    }
}

/// What shows among the leaves of some kinds: the red berries of a holly, the yellow
/// flowers of gorse, each 0.1 m. Nothing, and no roll of the dice, for any other leaf.
/// One set out from the face has a leaf under it to sit on, or it would hang in the air
/// wherever the tufts laid after it happened to miss.
fn garnish(plot: &mut FacePlot, rng: &mut Rng, leaf: Voxel) {
    let (piece, chance, most) = match leaf {
        mat::LEAVES_HOLLY => (mat::HOLLY_BERRY, 0.3, 2),
        mat::GORSE => (mat::GORSE_BLOOM, 0.6, 3),
        _ => return,
    };
    if rng.unit() >= chance {
        return;
    }
    for _ in 0..rng.int(1, most) {
        let (u, v, d) = (rng.int(0, 3), rng.int(0, 3), rng.int(0, 1));
        plot.fill(u, u + 1, v, v + 1, d, d + 1, piece);
        plot.fill(u, u + 1, v, v + 1, 0, d, leaf);
    }
}

/// The fruit of a broadleaf among its leaves, 0.1 m pieces: wild cherries in twos and
/// threes hanging under a cherry, and acorns on an oak or a holm oak high enough to be a
/// tree and not a bush. Sat on a leaf as [`garnish`] sets its berries, and rolled with
/// a die of its own, so the leaves about them grow as they did before there was fruit.
fn fruit_of(plot: &mut FacePlot, rng: &mut Rng, leaf: Voxel, toward: Toward, tree: bool) {
    let (piece, chance, most) = match leaf {
        // Hanging under the boughs; few on top, where they are only seen from above.
        mat::LEAVES_CHERRY => (mat::CHERRY, if toward == Toward::Below { 0.1 } else { 0.35 }, 3),
        mat::LEAVES_OAK | mat::LEAVES_HOLM if tree => (mat::ACORN, 0.22, 2),
        _ => return,
    };
    if rng.unit() >= chance {
        return;
    }
    for _ in 0..rng.int(1, most) {
        let (u, v, d) = (rng.int(0, 3), rng.int(0, 3), rng.int(0, 1));
        plot.fill(u, u + 1, v, v + 1, d, d + 1, piece);
        plot.fill(u, u + 1, v, v + 1, 0, d, leaf);
    }
}

/// A few tufts of leaves, hanging deeper from the underside of a canopy.
fn cluster(plot: &mut FacePlot, rng: &mut Rng, hanging: bool, leaf: Voxel) {
    for _ in 0..rng.int(2, 3) {
        let (u, v) = (rng.int(0, 3), rng.int(0, 3));
        let (w, h) = (rng.int(1, 2), rng.int(1, 2));
        let depth = if hanging {
            rng.int(1, 3)
        } else if rng.unit() < 0.3 {
            2
        } else {
            1
        };
        plot.fill(u, u + w, v, v + h, 0, depth, leaf);
    }
}

/// Bits of one chunk's interior heights in a column mask.
const INTERIOR: u64 = ((1u64 << CHUNK) - 1) << 1;

/// Visits each air voxel of a chunk column on the outside of tree foliage, once, as a
/// site on the leaf it touches: the leaf overhead if there is one, then the sides, then
/// below. Air shut in on the far side, inside a canopy, is passed over.
fn leaf_sites(voxels: &ColumnVoxels, mats: &Materials, mut visit: impl FnMut(Site)) {
    const ORDER: [Toward; 6] = [Toward::Above, Toward::PosX, Toward::NegX, Toward::PosZ, Toward::NegZ, Toward::Below];
    let n = CHUNK as usize;
    let IVec3 { x: x0, z: z0, .. } = voxels.corner();
    for ((cy, chunk), masks) in voxels.chunks().iter().zip(voxels.masks(mats)) {
        let mut leaf = [[0u64; PAD]; PAD];
        let mut air = [[0u64; PAD]; PAD];
        let mut any = false;
        for x in 0..PAD {
            for z in 0..PAD {
                let (solid, opaque, open) = masks[x][z];
                air[x][z] = open;
                // Leaves are foliage: opaque but not solid. Only those need looking at.
                let mut bits = opaque & !solid;
                while bits != 0 {
                    let y = bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    if is_leaf(chunk.get(x, y, z)) {
                        leaf[x][z] |= 1 << y;
                        any = true;
                    }
                }
            }
        }
        if !any {
            continue;
        }
        let bit = |mask: &[[u64; PAD]; PAD], x: usize, y: usize, z: usize| mask[x][z] >> y & 1 == 1;
        for x in 1..=n {
            for z in 1..=n {
                let l = &leaf;
                let around = (l[x][z] << 1) | (l[x][z] >> 1) | l[x + 1][z] | l[x - 1][z] | l[x][z + 1] | l[x][z - 1];
                let mut bits = air[x][z] & INTERIOR & around;
                while bits != 0 {
                    let y = bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    for toward in ORDER {
                        let s = toward.step();
                        let (hx, hy, hz) = ((x as i32 + s.x) as usize, (y as i32 + s.y) as usize, (z as i32 + s.z) as usize);
                        if !bit(&leaf, hx, hy, hz) {
                            continue;
                        }
                        let (ox, oy, oz) = ((x as i32 - s.x) as usize, (y as i32 - s.y) as usize, (z as i32 - s.z) as usize);
                        if bit(&air, ox, oy, oz) {
                            let air_pos = IVec3::new(x0 + x as i32, cy * CHUNK + y as i32 - 1, z0 + z as i32);
                            visit(Site { air: air_pos, toward, host: chunk.get(hx, hy, hz), canopy: 0 });
                        }
                        break;
                    }
                }
            }
        }
    }
}

/// Lays the twigs of the bare crowns of `placements` (see `Template::fine`) that fall in
/// the chunk column whose voxels `voxels` reads: each box is inside one voxel, and is
/// drawn by the column that voxel is in, only where the world still holds a leaf that
/// falls there (something built may have cleared it), and pulled back a cell from any
/// side of that leaf open to the air. So it is never seen in leaf, nor stands in a face
/// of the leaf it shares a plane with. And only while what it grows from is drawn too,
/// back to the wood, so no piece of twig is left hanging in the air in winter.
pub fn bare_crowns(lib: &Library, placements: &[Placement], voxels: &ColumnVoxels, out: &mut Vec<PlantBox>) {
    let IVec3 { x: x0, z: z0, .. } = voxels.corner();
    let (x1, z1) = (x0 + 1, z0 + 1);
    let (x2, z2) = (x0 + CHUNK, z0 + CHUNK);
    // What this column can read: its own voxels and a ring round them. A twig further off
    // is taken to be as its tree was made; the column it is in may find otherwise, but
    // only for what lies in the world beyond this one's ring.
    let seen = |q: IVec3| (x0..=x2 + 1).contains(&q.x) && (z0..=z2 + 1).contains(&q.z);
    // A leaf's face is drawn where it gives onto the air and against another tree's leaf
    // or wood, so a twig a cell back from the air is shut in. Against anything else (the
    // ground, which is drawn sloping, a wall, bracken) the face is left out while what is
    // there may be drawn short of it, and a twig in that leaf could be seen: none goes there.
    let holds = |v: Voxel| is_leaf(v) || is_wood(v);
    let cells = IVec3::splat(CELLS);
    // Each twig as it is drawn, or `None` where it is not, so those growing from it are not.
    let mut placed: Vec<Option<(IVec3, IVec3)>> = Vec::new();
    for p in placements {
        let t = lib.get(p.species, p.variant);
        if t.fine.is_empty() || p.lie.is_some() {
            continue;
        }
        if p.pos.x + t.reach < x1 || p.pos.x - t.reach > x2 || p.pos.z + t.reach < z1 || p.pos.z - t.reach > z2 {
            continue;
        }
        placed.clear();
        for (i, b) in t.fine.iter().enumerate() {
            let from = t.fine_from.get(i).copied().unwrap_or(ON_WOOD);
            let parent = (from != ON_WOOD).then(|| placed[from as usize]);
            if parent == Some(None) {
                placed.push(None);
                continue;
            }
            let (lo, hi) = rotate_box(IVec3::from_array(b.lo), IVec3::from_array(b.hi), p.turns);
            let (mut lo, mut hi) = (lo + p.pos * cells, hi + p.pos * cells);
            let q = lo.div_euclid(cells);
            if !seen(q) {
                placed.push(Some((lo, hi)));
                continue;
            }
            let around = FACES.map(|step| seen(q + step).then(|| voxels.voxel(q + step)));
            if !hides_twigs(voxels.voxel(q)) || around.iter().flatten().any(|&v| v != AIR && !holds(v)) {
                placed.push(None);
                continue;
            }
            let base = q * cells;
            for (face, &next) in around.iter().enumerate() {
                let axis = face / 2;
                let touches = if face % 2 == 0 { hi[axis] == base[axis] + CELLS } else { lo[axis] == base[axis] };
                if touches && next == Some(AIR) {
                    if face % 2 == 0 {
                        hi[axis] -= 1;
                    } else {
                        lo[axis] += 1;
                    }
                }
            }
            // Still against what it grows from, once both are pulled back from the air.
            let held = match parent {
                Some(Some(up)) => boxes_meet((lo, hi), up),
                _ => on_faces(lo, hi).any(|f| !seen(q + f) || is_wood(voxels.voxel(q + f))),
            };
            if !lo.cmplt(hi).all() || !held {
                placed.push(None);
                continue;
            }
            placed.push(Some((lo, hi)));
            if q.x >= x1 && q.x <= x2 && q.z >= z1 && q.z <= z2 && voxels.has_height(q.y) {
                out.push(PlantBox { lo: lo.to_array(), hi: hi.to_array(), ..*b });
            }
        }
    }
}

/// Adds every air voxel the boxes reach into to `into`.
pub fn occupied(boxes: &[PlantBox], into: &mut HashSet<IVec3>) {
    for b in boxes {
        let lo = IVec3::from_array(b.lo).div_euclid(IVec3::splat(CELLS));
        let hi = (IVec3::from_array(b.hi) - 1).div_euclid(IVec3::splat(CELLS));
        for y in lo.y..=hi.y {
            for z in lo.z..=hi.z {
                for x in lo.x..=hi.x {
                    into.insert(IVec3::new(x, y, z));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use voxel::{Kind, PaddedChunk, plant};

    use super::*;
    use crate::{
        World,
        biome::Biome,
        continent::SimConfig,
        materials::build,
        surface::Cover,
        world::GameWorld,
    };

    const GROUND: i32 = 7;

    /// A chunk column at world `(origin, 0, origin)` holding one tree: a 3x3 trunk of
    /// `bark` rooted below ground, a bare limb, and a canopy of `leaves` around the top.
    fn scene(origin: i32, bark: Voxel, leaves: Voxel) -> Vec<(i32, PaddedChunk)> {
        let p = |x: i32, y: i32, z: i32| IVec3::new(origin + x, y, origin + z);
        let mut map = HashMap::new();
        for x in -1..CHUNK + 1 {
            for z in -1..CHUNK + 1 {
                for y in 0..=GROUND {
                    map.insert(p(x, y, z), if y == GROUND { mat::GRASS_LUSH } else { mat::DIRT });
                }
            }
        }
        for y in 2..28 {
            for x in 8..11 {
                for z in 8..11 {
                    map.insert(p(x, y, z), bark);
                }
            }
        }
        // Thin limbs reaching out east and south, bare for most of their length.
        for k in 11..31 {
            map.insert(p(k, 18, 9), bark);
            map.insert(p(9, 15, k), bark);
        }
        for x in 2..18 {
            for z in 2..18 {
                for y in 22..30 {
                    let d = IVec3::new(x - 9, y - 26, z - 9);
                    if d.length_squared() <= 30 && map.get(&p(x, y, z)).is_none() {
                        map.insert(p(x, y, z), leaves);
                    }
                }
            }
        }
        let mut chunk = PaddedChunk::new();
        for (q, v) in map {
            let (px, py, pz) = (q.x - origin + 1, q.y + 1, q.z - origin + 1);
            if (0..PAD as i32).contains(&px) && (0..PAD as i32).contains(&py) && (0..PAD as i32).contains(&pz) {
                chunk.set(px as usize, py as usize, pz as usize, v);
            }
        }
        vec![(0, chunk)]
    }

    fn grown(origin: i32, bark: Voxel, leaves: Voxel, seed: u32) -> Vec<PlantBox> {
        let chunks = scene(origin, bark, leaves);
        let voxels = ColumnVoxels::new(&chunks, origin - 1, origin - 1);
        let col = Column { ground: GROUND as f32, water: None, cover: Cover::Land, biome: Biome::Deciduous, temperature: 0.5, river: f32::MAX, clear: f32::MAX, worn: None };
        let cols = vec![col; PAD * PAD];
        let grove = Grove { columns: &cols, x0: origin - 1, z0: origin - 1, seed, taken: &HashSet::new() };
        let mut out = Vec::new();
        grow(&voxels, &build(), &grove, &mut out);
        out
    }

    fn key(b: &PlantBox) -> ([i32; 3], [i32; 3], Voxel, bool) {
        (b.lo, b.hi, b.voxel, b.sway_hi)
    }

    #[test]
    fn the_fruit_ripens_in_its_own_months_and_is_gone_before_the_leaves() {
        use crate::features::plants::{Phase, month_start};
        let mid = |m: u8| (month_start(m) + 1.0 / 24.0).rem_euclid(1.0);
        let ripe = |v: Voxel, m: u8| (0..400).filter(|&k| fruit(v).unwrap().phase(mid(m), k * 7, 1000 - k * 3) == Phase::Bloom).count() as f32 / 400.0;
        let there = |v: Voxel, date: f32| (0..400).filter(|&k| fruit(v).unwrap().phase(date, k * 7, 1000 - k * 3) != Phase::Gone).count();
        // Cherries in July, acorns in September and October, holly berries in winter.
        assert!(ripe(mat::CHERRY, 7) > 0.8 && ripe(mat::CHERRY, 5) == 0.0 && there(mat::CHERRY, mid(9)) == 0);
        assert!(ripe(mat::ACORN, 9) > 0.3 && ripe(mat::ACORN, 10) > 0.3 && ripe(mat::ACORN, 8) == 0.0 && there(mat::ACORN, mid(4)) == 0);
        assert!(ripe(mat::HOLLY_BERRY, 12) > 0.9 && ripe(mat::HOLLY_BERRY, 1) > 0.9 && there(mat::HOLLY_BERRY, mid(4)) == 0);
        // Nothing hangs on into the leaf fall, from a twig the leaves have left (see
        // season_leaf in season.wgsl: the first leaf of the earliest tree goes at 0.602).
        for v in [mat::CHERRY, mat::ACORN] {
            assert_eq!(there(v, 0.606), 0, "{v} still hangs as the leaves fall");
        }
        // Nothing but fruit has a season of its own.
        assert!(fruit(mat::GORSE_BLOOM).is_none() && fruit(mat::LEAVES_OAK).is_none());
    }

    #[test]
    fn berries_and_blooms_never_hang_in_the_air() {
        for (leaf, piece) in [(mat::LEAVES_HOLLY, mat::HOLLY_BERRY), (mat::GORSE, mat::GORSE_BLOOM), (mat::LEAVES_CHERRY, mat::CHERRY), (mat::LEAVES_OAK, mat::ACORN)] {
            let mut seen = 0;
            for seed in 0..4000 {
                for toward in [Toward::NegX, Toward::PosZ, Toward::Above, Toward::Below] {
                    let site = Site { air: IVec3::new(3, 9, -2), toward, host: leaf, canopy: 0 };
                    let mut raster = Raster::new();
                    let mut plot = site.plot(&mut raster);
                    let mut rng = Rng::new(seed);
                    fruit_of(&mut plot, &mut rng, leaf, toward, true);
                    garnish(&mut plot, &mut rng, leaf);
                    // Not the tufts: they are what used to hold a berry up only by chance.
                    let mut boxes = Vec::new();
                    raster.finish(&mut boxes, |_| (true, true));
                    let step = toward.step();
                    let low = site.air * CELLS;
                    let filled = |c: IVec3| boxes.iter().any(|b| (0..3).all(|k| (b.lo[k]..b.hi[k]).contains(&c[k])));
                    for b in boxes.iter().filter(|b| b.voxel == piece) {
                        for x in b.lo[0]..b.hi[0] {
                            for y in b.lo[1]..b.hi[1] {
                                for z in b.lo[2]..b.hi[2] {
                                    seen += 1;
                                    // The next cell toward the host: in the host itself, or filled.
                                    let next = IVec3::new(x, y, z) + step;
                                    let inside = (0..3).all(|k| (low[k]..low[k] + CELLS).contains(&next[k]));
                                    assert!(!inside || filled(next), "seed {seed} {toward:?}: a piece at {x},{y},{z} with nothing behind it");
                                }
                            }
                        }
                    }
                }
            }
            assert!(seen > 500, "only {seen} pieces looked at");
        }
    }

    #[test]
    fn the_same_inputs_grow_the_same_detail() {
        let a = grown(64, mat::BARK_OAK, mat::LEAVES_OAK, 9);
        let b = grown(64, mat::BARK_OAK, mat::LEAVES_OAK, 9);
        assert!(!a.is_empty());
        assert_eq!(a.iter().map(key).collect::<Vec<_>>(), b.iter().map(key).collect::<Vec<_>>());
    }

    #[test]
    fn each_species_shows_its_own_bark() {
        let has = |boxes: &[PlantBox], v: Voxel| boxes.iter().any(|b| b.voxel == v);
        let oak = grown(64, mat::BARK_OAK, mat::LEAVES_OAK, 3);
        assert!(has(&oak, mat::BARK_OAK_RIDGE), "oak has no furrows");
        let redwood = grown(64, mat::BARK_REDWOOD, mat::NEEDLES_REDWOOD, 3);
        assert!(has(&redwood, mat::BARK_REDWOOD_RIDGE), "redwood has no ridges");
        let pine = grown(64, mat::BARK_PINE, mat::NEEDLES, 3);
        assert!(has(&pine, mat::BARK_PINE_PLATE), "pine has no plates");
        let birch = grown(64, mat::BARK_BIRCH, mat::LEAVES_BIRCH, 3);
        assert!(has(&birch, mat::BARK_BIRCH_MARK) && has(&birch, mat::BARK_BIRCH_PEEL), "birch has no marks or peeling strips");
        let palm = grown(64, mat::BARK_PALM, mat::LEAVES_PALM, 3);
        assert!(has(&palm, mat::BARK_PALM_RING), "palm has no rings");
        for (boxes, leaf) in [(&oak, mat::LEAVES_OAK), (&birch, mat::LEAVES_BIRCH), (&redwood, mat::NEEDLES_REDWOOD)] {
            assert!(has(boxes, leaf), "no leaf clusters of {leaf}");
        }
        assert!(has(&oak, mat::TWIG), "the bare limb grew no twigs");
        // Bark relief stays on the lower trunk.
        for b in oak.iter().filter(|b| b.voxel == mat::BARK_OAK_RIDGE) {
            assert!(b.lo[1].div_euclid(CELLS) <= GROUND + m(5.0) as i32 + 5, "{b:?} is too high up");
        }
    }

    #[test]
    fn budgets_hold_and_every_piece_is_fine() {
        for (bark, leaves) in [(mat::BARK_OAK, mat::LEAVES_OAK), (mat::BARK_REDWOOD, mat::NEEDLES_REDWOOD), (mat::BARK_BIRCH, mat::LEAVES_BIRCH)] {
            let out = grown(64, bark, leaves, 7);
            let (leafy, woody): (Vec<&PlantBox>, Vec<&PlantBox>) = out.iter().partition(|b| b.sway_hi);
            assert!(woody.len() <= WOOD_BOXES, "{}", woody.len());
            assert!(leafy.len() <= LEAF_BOXES, "{}", leafy.len());
            for b in &out {
                let dims = [b.hi[0] - b.lo[0], b.hi[1] - b.lo[1], b.hi[2] - b.lo[2]];
                assert!(dims.iter().min().unwrap() < &CELLS, "{b:?} is a full voxel");
                assert!(dims.iter().all(|&d| d > 0));
            }
            // A leaf cluster never spans two voxels, so it falls whole.
            for b in &leafy {
                let lo = IVec3::from_array(b.lo).div_euclid(IVec3::splat(CELLS));
                let hi = (IVec3::from_array(b.hi) - 1).div_euclid(IVec3::splat(CELLS));
                assert_eq!(lo, hi, "{b:?} spans voxels");
            }
        }
    }

    #[test]
    fn new_materials_are_wood_and_clusters_are_their_leaves() {
        let mats = build();
        for v in [
            mat::BARK_OAK_RIDGE,
            mat::BARK_REDWOOD_RIDGE,
            mat::BARK_PINE_PLATE,
            mat::BARK_BIRCH_MARK,
            mat::BARK_BIRCH_PEEL,
            mat::BARK_PALM_RING,
            mat::TWIG,
            mat::TWIG_PALE,
            mat::TWIG_FINE,
        ] {
            assert_eq!(mats.get(v).plant, voxel::plant::VINE, "{v} must keep its colour all year");
        }
        let out = grown(64, mat::BARK_OAK, mat::LEAVES_MAPLE, 4);
        for b in out.iter().filter(|b| b.sway_hi) {
            assert!(b.voxel == mat::LEAVES_MAPLE || b.voxel == mat::TWIG, "{b:?}");
            if b.voxel == mat::LEAVES_MAPLE {
                assert_eq!(mats.get(b.voxel).plant, voxel::plant::AUTUMN);
            }
        }
        // Deciduous foliage grows no twigs that would hang in the air once it has fallen.
        assert!(!out.iter().any(|b| b.sway_hi && b.voxel == mat::TWIG));
    }

    #[test]
    fn palms_grow_no_twigs() {
        for seed in 0..4 {
            let palm = grown(64, mat::BARK_PALM, mat::LEAVES_PALM, seed);
            assert!(palm.iter().any(|b| b.voxel == mat::LEAVES_PALM), "no frond clusters");
            assert!(!palm.iter().any(|b| b.voxel == mat::TWIG || b.voxel == mat::TWIG_PALE), "a palm grew a twig");
        }
    }

    #[test]
    fn no_template_paints_whole_voxels_of_moss() {
        let lib = crate::features::Library::build();
        for species in Species::ALL {
            for variant in 0..species.variants() {
                assert!(!lib.get(species, variant).voxels.iter().any(|(_, v)| *v == mat::MOSS), "{species:?} {variant} paints moss");
            }
        }
    }

    #[test]
    fn neighbouring_columns_agree_and_every_piece_is_attached() {
        // In a real world, each piece lies in its own column, in air, next to wood or
        // leaves that the voxels of the owning columns really hold; and a column grows the
        // same detail every time.
        let w = GameWorld::with_config(11, SimConfig { size: 256 });
        let mats = build();
        let detail = |p: &PlantBox| {
            matches!(
                p.voxel,
                mat::BARK_OAK_RIDGE
                    | mat::BARK_REDWOOD_RIDGE
                    | mat::BARK_PINE_PLATE
                    | mat::BARK_BIRCH_MARK
                    | mat::BARK_BIRCH_PEEL
                    | mat::BARK_PALM_RING
                    | mat::TWIG
                    | mat::TWIG_PALE
                    | mat::TWIG_FINE
                    | mat::BARK_YEW_FLAKE
                    | mat::HOLLY_BERRY
                    | mat::GORSE_BLOOM
                    | mat::CHERRY
                    | mat::ACORN
            ) || is_leaf(p.voxel)
        };
        let (centre, stride) = (crate::macro_layout::MACRO_SIZE / CHUNK / 2, 3);
        let (mut compared, mut crowns) = (0, 0);
        'columns: for cx in (centre - 300..centre + 300).step_by(stride) {
            for cz in (centre - 300..centre + 300).step_by(stride) {
                let here = w.generate_column(cx, cz);
                if !here.plants.iter().any(detail) {
                    continue;
                }
                let pieces = |o: &crate::ColumnOutput| o.plants.iter().filter(|p| detail(p)).map(key).collect::<Vec<_>>();
                assert_eq!(pieces(&here), pieces(&w.generate_column(cx, cz)), "column {cx},{cz} changed between runs");
                let east = w.generate_column(cx + 1, cz);
                let south = w.generate_column(cx, cz + 1);
                let west = w.generate_column(cx - 1, cz);
                let north = w.generate_column(cx, cz - 1);
                let world_voxel = |q: IVec3| -> Voxel {
                    let lookup = |o: &crate::ColumnOutput, ox: i32, oz: i32| ColumnVoxels::new(&o.chunks, ox * CHUNK - 1, oz * CHUNK - 1).voxel(q);
                    if q.x >= (cx + 1) * CHUNK {
                        lookup(&east, cx + 1, cz)
                    } else if q.x < cx * CHUNK {
                        lookup(&west, cx - 1, cz)
                    } else if q.z >= (cz + 1) * CHUNK {
                        lookup(&south, cx, cz + 1)
                    } else if q.z < cz * CHUNK {
                        lookup(&north, cx, cz - 1)
                    } else {
                        lookup(&here, cx, cz)
                    }
                };
                for b in here.plants.iter().filter(|p| detail(p)) {
                    let span = CHUNK * CELLS;
                    assert_eq!((b.lo[0].div_euclid(span), (b.hi[0] - 1).div_euclid(span)), (cx, cx), "{b:?} is outside its column");
                    assert_eq!((b.lo[2].div_euclid(span), (b.hi[2] - 1).div_euclid(span)), (cz, cz), "{b:?} is outside its column");
                    let cell = IVec3::from_array(b.lo).div_euclid(IVec3::splat(CELLS));
                    if b.voxel == mat::TWIG_FINE {
                        // A twig of a bare crown, in the leaf that hides it till it falls.
                        let hi = (IVec3::from_array(b.hi) - 1).div_euclid(IVec3::splat(CELLS));
                        assert_eq!(cell, hi, "{b:?} spans voxels");
                        assert!(hides_twigs(world_voxel(cell)), "{b:?} is not in a leaf");
                        crowns += 1;
                        continue;
                    }
                    assert_eq!(world_voxel(cell), AIR, "{b:?} is not in air");
                    let touching = Toward::ALL.iter().any(|t| {
                        let v = world_voxel(cell + t.step());
                        is_wood(v) || is_leaf(v) || mats.get(v).kind == Kind::Solid
                    });
                    assert!(touching, "{b:?} floats");
                }
                compared += 1;
                if compared >= 6 {
                    break 'columns;
                }
            }
        }
        assert!(compared > 0, "found no columns with tree detail to compare");
        assert!(crowns > 0, "found no twigs of bare crowns");
    }

    #[test]
    fn bare_crowns_stay_inside_the_leaves_that_fall() {
        // Every twig of a bare crown is in one voxel of its own tree's falling leaves, and
        // keeps a cell back from any side of it open to the air, where in summer it would
        // show through or flicker in the face of the leaf.
        let lib = crate::features::Library::build();
        let mats = build();
        for species in Species::ALL {
            for variant in 0..species.variants() {
                let t = lib.get(species, variant);
                let model = |p: IVec3| t.voxels.binary_search_by_key(&(p.y, p.x, p.z), |(q, _)| (q.y, q.x, q.z)).ok().map(|i| t.voxels[i].1);
                let falls = |v: Voxel| matches!(mats.get(v).plant, plant::DECIDUOUS | plant::AUTUMN);
                let leafy = t.voxels.iter().any(|&(_, v)| hides_twigs(v));
                if matches!(species, Species::Oak | Species::Birch | Species::Maple | Species::ElderOak) {
                    assert!(t.fine.len() > 100, "{species:?} {variant} has a bare crown of {} twigs", t.fine.len());
                }
                if !leafy {
                    assert!(t.fine.is_empty(), "{species:?} {variant} has twigs with no leaves to hide them");
                }
                for b in &t.fine {
                    assert_eq!(b.voxel, mat::TWIG_FINE);
                    let (lo, hi) = (IVec3::from_array(b.lo), IVec3::from_array(b.hi));
                    let at = lo.div_euclid(IVec3::splat(CELLS));
                    assert_eq!(at, (hi - 1).div_euclid(IVec3::splat(CELLS)), "{species:?} {variant}: {b:?} spans voxels");
                    let leaf = model(at).expect("a twig in the air");
                    assert!(hides_twigs(leaf) && falls(leaf) && mats.get(leaf).kind == Kind::Foliage, "{species:?} {variant}: {b:?} is in {leaf}");
                    for (face, step) in FACES.iter().enumerate() {
                        let axis = face / 2;
                        let base = at[axis] * CELLS;
                        let touches = if face % 2 == 0 { hi[axis] == base + CELLS } else { lo[axis] == base };
                        assert!(!touches || model(at + *step).is_some(), "{species:?} {variant}: {b:?} reaches the open");
                    }
                }
                // Each grows from the wood or from one before it, which it meets face to face.
                assert_eq!(t.fine_from.len(), t.fine.len());
                let span = |b: &PlantBox| (IVec3::from_array(b.lo), IVec3::from_array(b.hi));
                for (i, (b, &from)) in t.fine.iter().zip(&t.fine_from).enumerate() {
                    if from == ON_WOOD {
                        assert!(touches_wood(span(b), |q| model(q).is_some_and(is_wood)), "{species:?} {variant}: {b:?} is not on the wood");
                    } else {
                        assert!((from as usize) < i && meets(span(b), span(&t.fine[from as usize])), "{species:?} {variant}: {b:?} is not on the twig it grows from");
                    }
                }
            }
        }
    }

    /// Whether two boxes of cells share some of a face, worked out cell by cell.
    fn meets(a: (IVec3, IVec3), b: (IVec3, IVec3)) -> bool {
        let inside = |c: IVec3, (lo, hi): (IVec3, IVec3)| c.cmpge(lo).all() && c.cmplt(hi).all();
        for y in a.0.y..a.1.y {
            for z in a.0.z..a.1.z {
                for x in a.0.x..a.1.x {
                    let c = IVec3::new(x, y, z);
                    if FACES.iter().any(|&f| inside(c + f, b)) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Whether a box of cells has a cell against a voxel that `wood` takes.
    fn touches_wood((lo, hi): (IVec3, IVec3), wood: impl Fn(IVec3) -> bool) -> bool {
        for y in lo.y..hi.y {
            for z in lo.z..hi.z {
                for x in lo.x..hi.x {
                    let c = IVec3::new(x, y, z);
                    if FACES.iter().any(|&f| wood((c + f).div_euclid(IVec3::splat(CELLS)))) {
                        return true;
                    }
                }
            }
        }
        false
    }

    #[test]
    fn no_twig_of_a_bare_crown_hangs_in_the_air() {
        // In the world, each column drawing its own: every twig of a bare crown meets the
        // wood, or a twig that does, and so on back, across the edges of the columns.
        let w = GameWorld::with_config(11, SimConfig { size: 256 });
        let centre = crate::macro_layout::MACRO_SIZE / CHUNK / 2;
        let twigs = |o: &crate::ColumnOutput| o.plants.iter().filter(|p| p.voxel == mat::TWIG_FINE).count();
        let mut columns: Vec<(usize, i32, i32)> = Vec::new();
        for cx in (centre - 300..centre + 300).step_by(20) {
            for cz in (centre - 300..centre + 300).step_by(20) {
                let n = twigs(&w.generate_column(cx, cz));
                if n > 0 {
                    columns.push((n, cx, cz));
                }
            }
        }
        columns.sort_unstable_by(|a, b| b.cmp(a));
        assert!(columns.len() >= 4, "found too few columns with bare crowns: {columns:?}");
        let (mut checked, mut held) = (0, 0);
        for &(_, cx, cz) in columns.iter().take(6) {
            let mut outputs = HashMap::new();
            for dx in -1..=1 {
                for dz in -1..=1 {
                    outputs.insert((cx + dx, cz + dz), w.generate_column(cx + dx, cz + dz));
                }
            }
            let world_voxel = |q: IVec3| -> Voxel {
                let (ox, oz) = (q.x.div_euclid(CHUNK), q.z.div_euclid(CHUNK));
                match outputs.get(&(ox, oz)) {
                    Some(o) => ColumnVoxels::new(&o.chunks, ox * CHUNK - 1, oz * CHUNK - 1).voxel(q),
                    None => AIR,
                }
            };
            let span = |b: &PlantBox| (IVec3::from_array(b.lo), IVec3::from_array(b.hi));
            let boxes: Vec<((IVec3, IVec3), bool)> = outputs
                .iter()
                .flat_map(|(&(ox, oz), o)| o.plants.iter().filter(|p| p.voxel == mat::TWIG_FINE).map(move |b| (span(b), (ox, oz) == (cx, cz))))
                .collect();
            let mut near: HashMap<IVec3, Vec<usize>> = HashMap::new();
            for (i, &(b, _)) in boxes.iter().enumerate() {
                near.entry(b.0.div_euclid(IVec3::splat(CELLS))).or_default().push(i);
            }
            let mut reached: Vec<bool> = boxes.iter().map(|&(b, _)| touches_wood(b, |q| is_wood(world_voxel(q)))).collect();
            let mut queue: Vec<usize> = (0..boxes.len()).filter(|&i| reached[i]).collect();
            while let Some(i) = queue.pop() {
                let at = boxes[i].0.0.div_euclid(IVec3::splat(CELLS));
                for step in [IVec3::ZERO].into_iter().chain(FACES) {
                    for &j in near.get(&(at + step)).into_iter().flatten() {
                        if !reached[j] && meets(boxes[i].0, boxes[j].0) {
                            reached[j] = true;
                            queue.push(j);
                        }
                    }
                }
            }
            for (&(b, own), &r) in boxes.iter().zip(&reached) {
                if own {
                    assert!(r, "the twig {b:?} in column {cx},{cz} hangs in the air");
                    held += 1;
                }
            }
            checked += 1;
        }
        assert!(checked > 0 && held > 0);
    }
}
