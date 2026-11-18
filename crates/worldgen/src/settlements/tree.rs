//! The village tree: the big old tree near a village, where people leave small things
//! among its roots, tie ribbons to its lower branches, and have worn a path round it.
//!
//! [`plant_all`] plants one for each settlement on the ground its layout keeps beside the
//! green, and dresses it. [`village_tree`] finds one where none was planted: the nearest
//! giant to the village's middle, or failing one the stoutest tree near. [`dress`] works out what the village has added to it, by how long
//! the village has stood: offerings and ribbons, as fine parts to stamp with the rest of
//! the village, and the [`WornRing`] of trodden ground round the trunk. The tree itself
//! is left as it grew, so it stands where it always would have, roots and all, and a
//! village that comes to it finds it as it is.
//!
//! Everything is decided from where the tree stands and the world's seed, so it comes out
//! the same however often it is asked for, whichever chunk draws it.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
};

use glam::{IVec2, IVec3, Vec2};
use voxel::{
    Voxel,
    mesh::{CELLS, PlantBox},
};

use crate::{
    continent::Continent,
    features::{self, Library, Placement, cloth, landmarks::Palette, plants::Flower, roots, shape::rotate, trees::Species},
    macro_layout::MACRO_SIZE,
    materials as mat,
    noise::{hash2, hash3, perlin2, unit},
    scale::m,
    surface::{Cover, Sampler, Worn},
    settlements::Age,
    trails::{Network, Stamp},
};

/// Plants a tree for every settlement on the ground its layout kept for one beside the
/// green, of the country's biggest kind, in a form whose crown keeps clear of the roofs
/// about it where one will; or where the layout kept none, takes the stoutest tree left
/// standing within the settlement. Then dresses each as the place's age has it: its
/// ribbons and offerings are stamped, and its worn ring and where it stands kept in `net`.
pub(crate) fn plant_all(c: &Continent, seed: u32, net: &mut Network) {
    let lib = Library::shared();
    // Each planted before any is looked for, so that each is grown with all the others in
    // the ground as the world will have them.
    let chosen: Vec<Option<features::Planted>> = (0..net.settlements.len())
        .map(|k| {
            let (at, _) = net.layouts.get(k)?.tree?;
            let l = &net.layouts[k];
            let room = net.plots[l.plots.clone()].iter().map(|p| gap(at, p.lo, p.hi)).fold(f32::MAX, f32::min);
            plant(c, seed, net, lib, at, room)
        })
        .collect();
    net.planted = chosen.iter().flatten().copied().collect();
    let origin = (c.desc.mx * MACRO_SIZE, c.desc.mz * MACRO_SIZE);
    let ks: Vec<usize> = (0..net.settlements.len()).collect();
    let net_now = &*net;
    let dressed = crate::trails::build::in_parallel(&ks, |&k| {
        let s = &net_now.settlements[k];
        let tree = match chosen[k] {
            Some(t) => {
                let margin = features::MAX_REACH + 1;
                let sampler = Sampler::with_network(c, seed, t.x - margin, t.z - margin, 2 * margin + 1, net_now);
                let at = (origin.0 + t.x, origin.1 + t.z);
                features::place(seed, lib, &sampler, t.x, t.z, 1, origin).into_iter().find(|p| (p.pos.x, p.pos.z) == at && p.species == t.species)
            }
            None => village_tree(c, seed, net_now, lib, Vec2::new(s.x as f32 + 0.5, s.z as f32 + 0.5), s.radius),
        }?;
        let age = match s.age {
            Age::Young => 0.1,
            Age::Established => 0.45,
            Age::Ancient => 0.8,
        } + 0.2 * unit(hash2(s.seed ^ 0x7ee1, 1, 2));
        Some((tree.pos - IVec3::new(origin.0, 0, origin.1), dress_at(c, seed, net_now, lib, &tree, age)))
    });
    net.village_trees = vec![None; net.settlements.len()];
    net.tree_knots = vec![Default::default(); net.settlements.len()];
    for (k, d) in dressed.into_iter().enumerate() {
        let Some((at, d)) = d else { continue };
        net.village_trees[k] = Some(at);
        net.tree_knots[k] = (d.knots, d.spare);
        net.add_stamp_with(Vec::new(), d.fine);
        net.rings.push(d.ring);
    }
}

/// How far continent-local `p` is from the columns `lo..=hi`, in voxels.
fn gap(p: Vec2, lo: IVec2, hi: IVec2) -> f32 {
    let (lo, hi) = (lo.as_vec2(), hi.as_vec2() + 1.0);
    let d = (lo - p).max(p - hi).max(Vec2::ZERO);
    d.length()
}

/// The tree a village plants at continent-local `at`, whose crown should keep within
/// `room` voxels of its trunk: of the country's biggest kind, in the form that keeps
/// within it, or the smallest if none does, turned as the seed has it.
fn plant(c: &Continent, seed: u32, net: &Network, lib: &Library, at: Vec2, room: f32) -> Option<features::Planted> {
    let (x, z) = (at.x.floor() as i32, at.y.floor() as i32);
    let col = Sampler::with_network(c, seed, x, z, 1, net).column(x, z);
    let salt = hash2(seed ^ 0x9a17, x, z);
    let species = features::village_species(col.biome, unit(salt))?;
    let n = species.variants();
    let first = (salt >> 8) as usize % n;
    let reach = |v: usize| lib.get(species, v).reach as f32;
    let variant = (0..n).map(|i| (first + i) % n).find(|&v| reach(v) <= room + m(1.0)).unwrap_or_else(|| (0..n).min_by(|&a, &b| reach(a).total_cmp(&reach(b))).unwrap_or(0));
    Some(features::Planted { x, z, species, variant, turns: (salt >> 20) % 4 })
}

/// How far from a village's middle its tree may stand.
pub const REACH: f32 = m(400.0);
/// Ribbons on the tree of a village just founded, and of the oldest.
const RIBBONS_YOUNG: f32 = 4.0;
const RIBBONS_ANCIENT: f32 = 20.0;
/// Highest over the ground below that a ribbon is tied, in voxels: as high as someone can
/// reach from a friend's shoulders, or throw a weighted end over a limb.
const TIE_HIGHEST: i32 = m(8.0) as i32;
/// Where a tree has nothing that low to tie to, ribbons go round the trunk at about head
/// height instead, this far over its own ground.
const GIRDLE: i32 = m(2.0) as i32;
/// Lowest a ribbon's free end hangs over the ground, in cells: clear of a grown head.
const END_CLEAR: i32 = m(1.2) as i32 * CELLS;
/// The most spare places under a tree's limbs kept for a festival (#207), and the room
/// a lantern hung at one needs below it, in cells.
const SPARE: usize = 12;
const LANTERN_ROOM: i32 = 12;
/// The colours the ribbons are dyed: madder red, woad blue, weld yellow, and undyed.
const DYES: [Voxel; 4] = [mat::CLOTH_RED, mat::CLOTH_BLUE, mat::CLOTH_YELLOW, mat::CLOTH_PALE];
/// The flowers left in season: each comes and goes with its own months.
const POSIES: [Flower; 7] = [Flower::Primrose, Flower::Bluebell, Flower::Daisy, Flower::Campion, Flower::Buttercup, Flower::Poppy, Flower::Heather];

/// The tree of a village whose middle is at continent-local `core`: the nearest giant
/// within `reach` voxels of it, or if there is none, the tree there with the stoutest
/// trunk (the nearer of two alike). As it stands in the world, roots and all. `net` is
/// the continent's trails and sites as they will be: a tree in a site's clearing is felled,
/// and only one left standing will do. None where nothing grows that near.
///
/// It reads the ground over the whole square, a few tens of milliseconds: for building a
/// continent, not for every frame.
pub fn village_tree(c: &Continent, seed: u32, net: &Network, lib: &Library, core: Vec2, reach: f32) -> Option<Placement> {
    let origin = (c.desc.mx * MACRO_SIZE, c.desc.mz * MACRO_SIZE);
    let r = reach.ceil() as i32;
    let (lx0, lz0, span) = (core.x.floor() as i32 - r, core.y.floor() as i32 - r, 2 * r + 1);
    let margin = features::MAX_REACH + 1;
    let sampler = Sampler::with_network(c, seed, lx0 - margin, lz0 - margin, span + 2 * margin, net);
    let away = |p: &Placement| Vec2::new((p.pos.x - origin.0) as f32 + 0.5, (p.pos.z - origin.1) as f32 + 0.5).distance(core);
    // Ties go the same way every time.
    let nearer = |a: &Placement, b: &Placement| away(a).total_cmp(&away(b)).then((a.pos.x, a.pos.z).cmp(&(b.pos.x, b.pos.z)));
    let giants = features::place_giants(seed, lib, &sampler, lx0, lz0, span, origin);
    let chosen = match giants.iter().filter(|p| away(p) <= reach).min_by(|a, b| nearer(a, b)) {
        Some(giant) => *giant,
        None => {
            let trees = features::place_trees(seed, lib, &sampler, lx0, lz0, span, origin);
            let living = trees.iter().filter(|p| away(p) <= reach && p.lie.is_none() && !matches!(p.species, Species::Stump | Species::DeadSnag));
            let girth = |p: &Placement| roots::foot(lib, p.species, p.variant, p.turns).map_or(0.0, |f| f.1);
            *living.max_by(|a, b| girth(a).total_cmp(&girth(b)).then(nearer(b, a)))?
        }
    };
    // As the world has it, with the roots that the quick placing leaves out.
    let (lx, lz) = (chosen.pos.x - origin.0, chosen.pos.z - origin.1);
    let sampler = Sampler::with_network(c, seed, lx - margin, lz - margin, 2 * margin + 1, net);
    features::place(seed, lib, &sampler, lx, lz, 1, origin).into_iter().find(|p| p.pos == chosen.pos && p.species == chosen.species)
}

/// The ground trodden bare round a village tree: a path round the trunk a little way out,
/// and at the foot, where people kneel to leave things, bare in patches. Continent-local,
/// in voxels. The trunk and the roots flaring from it are never worn.
#[derive(Clone, Copy, Debug)]
pub struct WornRing {
    /// The middle of the trunk.
    pub x: f32,
    pub z: f32,
    /// Out to here from the middle is the trunk and its flaring foot.
    pub trunk: f32,
    /// Out to here the foot is worn in patches.
    pub foot: f32,
    /// The path round goes this far out.
    pub ring: f32,
    /// How trodden it is, 0 (hardly) to 1 (bare all the way round).
    pub wear: f32,
}

impl WornRing {
    /// Whether the ground at continent-local column `(lx, lz)`, whose middle is `p`, is
    /// worn bare. Patchy where little used, ragged at the edges, as a site's pad is.
    pub fn worn(&self, seed: u32, p: Vec2, lx: i32, lz: i32) -> bool {
        let r = (p - Vec2::new(self.x, self.z)).length();
        if r < self.trunk || r > self.reach() {
            return false;
        }
        let ragged = (unit(hash2(seed ^ 0x3018, lx, lz)) - 0.5) * m(0.5);
        let patch = perlin2(seed ^ 0x3017, p.x as f64 / m(2.4) as f64, p.y as f64 / m(2.4) as f64) * 0.5 + 0.5;
        let half = m(0.3) + m(0.45) * self.wear + ragged;
        if (r - self.ring).abs() < half {
            return patch <= 0.35 + self.wear;
        }
        r < self.foot + ragged && patch <= 0.1 + 0.3 * self.wear
    }

    /// As a trail's wear, if the ring wears column `(lx, lz)` (middle `p`), whose cover
    /// was `under` and the land's slope there `slope`: worn as a well-used footpath is,
    /// into whatever ground it was. Only land, banks and scree wear. For the surface to
    /// set in place of the column's own, with its cover made [`Cover::Trail`], as it does
    /// for a pad's worn ground.
    pub fn wear(&self, seed: u32, p: Vec2, lx: i32, lz: i32, under: Cover, slope: f32) -> Option<Worn> {
        if !matches!(under, Cover::Land | Cover::Bank | Cover::Scree) || !self.worn(seed, p, lx, lz) {
            return None;
        }
        let half_width = m(0.6);
        Some(Worn { under, slope, half_width, offset: half_width * 0.5, dir: Vec2::X, point: hash2(seed ^ 0x7a12, lx, lz) })
    }

    /// Furthest from the middle of the trunk that it wears anything.
    pub fn reach(&self) -> f32 {
        self.ring + m(1.0)
    }
}

/// What a village has added to its tree.
pub struct Dressing {
    /// Offerings among the roots and ribbons in the branches: fine parts in continent-local
    /// cells of 1/[`CELLS`] voxel, to stamp as a stamp's fine parts are.
    pub fine: Vec<PlantBox>,
    /// The ground trodden bare round it.
    pub ring: WornRing,
    /// Where each ribbon is tied: the corner of its top cell, continent-local cells.
    pub knots: Vec<IVec3>,
    /// Where more could be tied or hung on a festival (#207), as `knots` are, with room
    /// below each for a lantern: the next lowest places under its limbs, apart from the
    /// ribbons and from one another. Nothing is built at them.
    pub spare: Vec<IVec3>,
}

impl Dressing {
    /// The fine parts as a stamp of their own, with its bounds.
    pub fn stamp(&self) -> Stamp {
        let cells = IVec3::splat(CELLS);
        let (mut lo, mut hi) = (IVec3::splat(i32::MAX), IVec3::splat(i32::MIN));
        for b in &self.fine {
            lo = lo.min(IVec3::from_array(b.lo).div_euclid(cells));
            hi = hi.max((IVec3::from_array(b.hi) - 1).div_euclid(cells));
        }
        Stamp { lo, hi, voxels: Vec::new(), fine: self.fine.clone() }
    }
}

/// [`dress`] for a tree on continent `c`, reading its ground, trails and sites (`net`) and
/// taking the colours of the country it stands in.
pub fn dress_at(c: &Continent, seed: u32, net: &Network, lib: &Library, tree: &Placement, age: f32) -> Dressing {
    let origin = (c.desc.mx * MACRO_SIZE, c.desc.mz * MACRO_SIZE);
    let (lx, lz) = (tree.pos.x - origin.0, tree.pos.z - origin.1);
    let r = tree.reach(lib) + 2;
    let sampler = Sampler::with_network(c, seed, lx - r, lz - r, 2 * r + 1, net);
    let biome = sampler.column(lx, lz).biome;
    let palette = Palette::at(seed, biome, tree.pos.x, tree.pos.y, tree.pos.z);
    // The same columns are asked about many times over.
    let known: RefCell<HashMap<(i32, i32), Option<i32>>> = RefCell::default();
    let ground = |x: i32, z: i32| {
        *known.borrow_mut().entry((x, z)).or_insert_with(|| {
            let col = sampler.column(x, z);
            col.water_surface().is_none().then_some(col.top())
        })
    };
    dress(lib, tree, origin, ground, &palette, age, seed)
}

/// What a village `age` old (0 just founded, 1 as old as any) has added to its tree,
/// `tree`, placed in the world as [`village_tree`] gives it; `origin` is where its
/// continent starts, in world voxels, and `ground` the top of the ground at a
/// continent-local column, or None over water. The offerings take their stone from
/// `palette`. More ribbons the older the village, from four to about twenty, and more
/// offerings, and the path round is the more worn.
pub fn dress(lib: &Library, tree: &Placement, origin: (i32, i32), ground: impl Fn(i32, i32) -> Option<i32>, palette: &Palette, age: f32, seed: u32) -> Dressing {
    let age = age.clamp(0.0, 1.0);
    let shift = IVec3::new(origin.0, 0, origin.1);
    let at = tree.pos - shift;
    let salt = hash3(seed ^ 0x7ee0, tree.pos.x, tree.pos.y, tree.pos.z);
    // The trunk, and its foot with whatever flares from it.
    let (offset, radius) = roots::foot(lib, tree.species, tree.variant, tree.turns).map_or((Vec2::splat(1.0), 1.0), |f| (f.0, f.1));
    let centre = Vec2::new(at.x as f32, at.z as f32) + offset;
    let foot = tree.roots.and_then(|r| r.collar()).map_or(radius + 0.5, |c| c.1);
    // Everything of the tree, model and roots, in continent-local voxels.
    let mut rooted = HashSet::new();
    if let Some(r) = &tree.roots {
        r.voxels(tree.pos, |q, _| {
            rooted.insert(q - shift);
        });
    }
    let model = |q: IVec3| features::model_voxel(lib, tree, q + shift);
    let taken = |q: IVec3| rooted.contains(&q) || model(q).is_some();

    let wear = 0.35 + 0.65 * age;
    let ring = WornRing { x: centre.x, z: centre.y, trunk: foot, foot: foot + m(1.2), ring: foot + m(2.2), wear };
    let mut fine = Vec::new();
    offerings(&ring, at.y - 1, &ground, &taken, palette, age, seed, salt, &mut fine);
    let (knots, spare) = ribbons(lib, tree, shift, &ground, &taken, centre, radius, age, salt, &mut fine);
    Dressing { fine, ring, knots, spare }
}

/// Lays the offerings at the foot of the tree, on the trodden ground between its roots:
/// bowls of grain or berries, candles burnt down, little carved figures, stones piled one
/// on another, and flowers in season.
#[allow(clippy::too_many_arguments)]
fn offerings(
    ring: &WornRing,
    base: i32,
    ground: &impl Fn(i32, i32) -> Option<i32>,
    taken: &impl Fn(IVec3) -> bool,
    palette: &Palette,
    age: f32,
    seed: u32,
    salt: u32,
    out: &mut Vec<PlantBox>,
) {
    let want = (3.0 + 13.0 * age).round() as usize;
    let centre = Vec2::new(ring.x, ring.z);
    let r = ring.foot.ceil() as i32 + 1;
    // Worn columns just outside the trunk's foot, standing clear, nestled against a root
    // or the foot itself first.
    let mut spots = Vec::new();
    for z in centre.y.floor() as i32 - r..=centre.y.floor() as i32 + r {
        for x in centre.x.floor() as i32 - r..=centre.x.floor() as i32 + r {
            let p = Vec2::new(x as f32 + 0.5, z as f32 + 0.5);
            let Some(g) = ground(x, z) else { continue };
            if (g - base).abs() > 3 || !ring.worn(seed, p, x, z) || p.distance(centre) > ring.foot + 1.0 {
                continue;
            }
            if taken(IVec3::new(x, g + 1, z)) || taken(IVec3::new(x, g + 2, z)) {
                continue;
            }
            let nestled = [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|&(dx, dz)| taken(IVec3::new(x + dx, g + 1, z + dz)));
            let roll = unit(hash3(salt ^ 0x0ffe, x, g, z));
            spots.push((roll - nestled as u8 as f32, x, g, z));
        }
    }
    spots.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut laid: Vec<(i32, i32)> = Vec::new();
    for &(_, x, g, z) in &spots {
        if laid.len() == want {
            break;
        }
        // A little apart, so each can be seen for what it is.
        if laid.iter().any(|&(lx, lz)| (lx - x).abs() < 2 && (lz - z).abs() < 2) {
            continue;
        }
        laid.push((x, z));
        let h = hash3(salt ^ 0x0ff1, x, g, z);
        let corner = IVec3::new(x, g + 1, z) * CELLS;
        offering(h, corner, palette, out);
    }
}

/// One offering, its lowest corner at `corner` (cells) on the ground: which, and how it
/// lies, from the hash `h`. Each fits within its column, a voxel square.
fn offering(h: u32, corner: IVec3, palette: &Palette, out: &mut Vec<PlantBox>) {
    // Turned a quarter or not, and set anywhere in the column it fits.
    let turned = h & 0x100 != 0;
    let mut parts: Vec<([i32; 3], [i32; 3], Voxel, u8)> = Vec::new();
    let mut part = |lo: [i32; 3], hi: [i32; 3], v: Voxel| parts.push((lo, hi, v, 0));
    match h % 10 {
        // A bowl of grain, berries or honey.
        0 | 1 => {
            part([0, 0, 0], [3, 1, 3], mat::CLAY);
            part([0, 1, 0], [3, 2, 1], mat::CLAY);
            part([0, 1, 2], [3, 2, 3], mat::CLAY);
            part([0, 1, 1], [1, 2, 2], mat::CLAY);
            part([2, 1, 1], [3, 2, 2], mat::CLAY);
            let filling = [mat::TUFT_STRAW, mat::HOLLY_BERRY, mat::BLOOM_GOLD][(h >> 4) as usize % 3];
            part([1, 1, 1], [2, 2, 2], filling);
        }
        // Candles burnt down to different lengths, their wicks black: lit for a night's
        // vigil, not left burning.
        2..=4 => {
            for (k, (x, z)) in [(0, 0), (2, 1), (1, 3)].into_iter().enumerate().take(1 + (h >> 4) as usize % 3) {
                let tall = 2 + (h >> (8 + k)) as i32 % 2;
                part([x, 0, z], [x + 1, tall, z + 1], mat::CLOTH_PALE);
                part([x, tall, z], [x + 1, tall + 1, z + 1], mat::CHARCOAL);
            }
        }
        // A little carved figure, arms out.
        5 | 6 => {
            let wood = if h & 0x40 != 0 { mat::PLANK_PALE } else { mat::PLANK };
            part([0, 0, 0], [3, 2, 2], wood);
            part([0, 2, 0], [3, 3, 1], wood);
            part([1, 3, 0], [2, 4, 1], wood);
        }
        // Stones piled one on another, each a little smaller and set a little off.
        7 | 8 => {
            part([0, 0, 0], [3, 1, 3], palette.stone);
            part([1, 1, 0], [3, 2, 2], palette.stone_dark);
            part([1, 2, 1], [2, 3, 2], palette.stone);
            if h & 0x80 != 0 {
                part([1, 3, 1], [2, 4, 2], palette.stone_dark);
            }
        }
        // A posy laid down, there only while its flower is out.
        _ => {
            let flower = POSIES[(h >> 4) as usize % POSIES.len()];
            let season = flower.bloom_code();
            parts.push(([0, 0, 1], [3, 1, 2], mat::STEM, season));
            parts.push(([3, 0, 0], [4, 1, 3], flower.petals(), season));
            parts.push(([2, 0, 0], [3, 1, 1], flower.petals(), season));
        }
    }
    let (w, d) = parts.iter().fold((0, 0), |(w, d), p| (w.max(p.1[0]), d.max(p.1[2])));
    let (w, d) = if turned { (d, w) } else { (w, d) };
    let shift = IVec3::new((h >> 12) as i32 % (CELLS - w + 1), 0, (h >> 16) as i32 % (CELLS - d + 1));
    for (lo, hi, voxel, season) in parts {
        let (mut lo, mut hi) = (IVec3::from_array(lo), IVec3::from_array(hi));
        if turned {
            (lo, hi) = (IVec3::new(lo.z, lo.y, lo.x), IVec3::new(hi.z, hi.y, hi.x));
        }
        let (lo, hi) = (corner + shift + lo, corner + shift + hi);
        out.push(PlantBox { lo: lo.to_array(), hi: hi.to_array(), voxel, sway_lo: false, sway_hi: false, thin: 0, season, flap: 0 });
    }
}

/// Ties the ribbons on: each from the underside of a limb of the tree's own model, the
/// lowest there are, never two side by side, in the four dyes. A tree with no limb low
/// enough has them tied round its trunk instead, hanging down the bark. Returns where each
/// is tied, and the spare places under the limbs left over (see [`Dressing::spare`]).
#[allow(clippy::too_many_arguments)]
fn ribbons(
    lib: &Library,
    tree: &Placement,
    shift: IVec3,
    ground: &impl Fn(i32, i32) -> Option<i32>,
    taken: &impl Fn(IVec3) -> bool,
    centre: Vec2,
    radius: f32,
    age: f32,
    salt: u32,
    out: &mut Vec<PlantBox>,
) -> (Vec<IVec3>, Vec<IVec3>) {
    let want = (RIBBONS_YOUNG + (RIBBONS_ANCIENT - RIBBONS_YOUNG) * age).round() as usize;
    let t = lib.get(tree.species, tree.variant);
    let at = tree.pos - shift;
    // How many cells a ribbon tied at the corner of cell `knot` can hang, its column clear
    // of the tree all the way down and its end clear of the ground.
    let room = |knot: IVec3| -> i32 {
        let (vx, vz) = (knot.x.div_euclid(CELLS), knot.z.div_euclid(CELLS));
        let Some(g) = ground(vx, vz) else { return 0 };
        let lowest = (g + 1) * CELLS + END_CLEAR;
        let mut y = knot.y.div_euclid(CELLS) - 1;
        while y * CELLS >= lowest && !taken(IVec3::new(vx, y, vz)) {
            y -= 1;
        }
        (knot.y - lowest).min((knot.y.div_euclid(CELLS) - 1 - y) * CELLS)
    };

    // Under the limbs: wood with air under it, out from the trunk, low enough to reach.
    let mut under = Vec::new();
    for &(p, v) in &t.voxels {
        if !roots::is_bark(v) {
            continue;
        }
        let q = at + rotate(p, tree.turns);
        if Vec2::new(q.x as f32 + 0.5, q.z as f32 + 0.5).distance(centre) < radius + 2.0 || taken(q - IVec3::Y) {
            continue;
        }
        let Some(g) = ground(q.x, q.z) else { continue };
        if q.y - g > TIE_HIGHEST {
            continue;
        }
        let h = hash3(salt ^ 0x41b0, q.x, q.y, q.z);
        under.push(((q.y - g) as f32 + unit(h) * 6.0, q, h));
    }
    under.sort_by(|a, b| a.0.total_cmp(&b.0).then((a.1.x, a.1.y, a.1.z).cmp(&(b.1.x, b.1.y, b.1.z))));
    let mut knots: Vec<IVec3> = Vec::new();
    let tie = |knot: IVec3, along_x: bool, h: u32, out: &mut Vec<PlantBox>, knots: &mut Vec<IVec3>| {
        let room = room(knot);
        let length = (12 + (h >> 8) as i32 % 19).min(room).min(cloth::RIBBON_LONGEST);
        if length < 8 {
            return;
        }
        let dye = DYES[(h >> 4) as usize % DYES.len()];
        out.extend(cloth::ribbon(knot, length, dye, if along_x { 0 } else { 1 }));
        knots.push(knot);
    };
    for &(_, q, h) in &under {
        if knots.len() == want {
            break;
        }
        if knots.iter().any(|k| (k.div_euclid(IVec3::splat(CELLS)) - q).as_vec3().length() < 1.2) {
            continue;
        }
        // Two cells wide along x or z, anywhere under the voxel it fits.
        let along_x = h & 1 == 0;
        let (ox, oz) = if along_x { ((h >> 2) as i32 % 3, (h >> 5) as i32 % 4) } else { ((h >> 2) as i32 % 4, (h >> 5) as i32 % 3) };
        tie(IVec3::new(q.x * CELLS + ox, q.y * CELLS, q.z * CELLS + oz), along_x, h, out, &mut knots);
    }
    if !knots.is_empty() {
        // The places left under the limbs, for a festival's lanterns: room for one to
        // hang below each, the lowest first, never beside a ribbon or another.
        let apart = |k: IVec3, q: IVec3| (k.div_euclid(IVec3::splat(CELLS)) - q).as_vec3().length() < 1.5;
        let mut spare: Vec<IVec3> = Vec::new();
        for &(_, q, _) in &under {
            if spare.len() == SPARE {
                break;
            }
            if knots.iter().chain(&spare).any(|&k| apart(k, q)) {
                continue;
            }
            let knot = IVec3::new(q.x * CELLS + CELLS / 2, q.y * CELLS, q.z * CELLS + CELLS / 2);
            if room(knot) >= LANTERN_ROOM {
                spare.push(knot);
            }
        }
        return (knots, spare);
    }

    // Round the trunk: from each side in turn, walk in to the bark at about head height and
    // tie on there, the ribbon flat against it.
    let g = at.y - 1;
    let y = g + GIRDLE;
    let sides = [(IVec3::X, false), (IVec3::NEG_X, false), (IVec3::Z, true), (IVec3::NEG_Z, true)];
    for k in 0..want {
        let (out_dir, along_x) = sides[k % 4];
        let across = IVec3::new(out_dir.z, 0, out_dir.x);
        let off = [0, -1, 1][(k / 4) % 3] * (radius * 0.5).round() as i32;
        let from = IVec3::new(centre.x.floor() as i32, y, centre.y.floor() as i32) + across * off;
        let Some(d) = (0..16).rev().find(|&d| roots::is_bark(features::model_voxel(lib, tree, from + out_dir * d + shift).unwrap_or(voxel::AIR))) else {
            continue;
        };
        let face = from + out_dir * d;
        if taken(face + out_dir) || taken(face + out_dir + IVec3::Y) {
            continue;
        }
        // The cell against the face, at the top of the air beside it.
        let side = |c: i32, o: i32| if o > 0 { (c + 1) * CELLS } else if o < 0 { c * CELLS - 1 } else { c * CELLS + 1 };
        let knot = IVec3::new(side(face.x, out_dir.x), (y + 1) * CELLS, side(face.z, out_dir.z));
        let h = hash3(salt ^ 0x41b1, knot.x, knot.y, knot.z);
        tie(knot, along_x, h, out, &mut knots);
    }
    (knots, Vec::new())
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use super::*;
    use crate::world::GameWorld;

    fn world() -> &'static GameWorld {
        static WORLD: OnceLock<GameWorld> = OnceLock::new();
        WORLD.get_or_init(|| GameWorld::new(1))
    }

    /// The village tree for a village by the elder oak in the grassland east of the start.
    fn tree() -> Placement {
        let w = world();
        let c = w.continent(0, 0);
        village_tree(&c, w.seed(), &c.trails, w.library(), Vec2::new(49300.0, 31800.0), REACH).expect("a tree near")
    }

    #[test]
    fn the_village_tree_is_the_nearest_giant_and_stands_in_the_world() {
        let w = world();
        let c = w.continent(0, 0);
        let p = tree();
        assert!(matches!(p.species, Species::ElderOak | Species::Redwood | Species::Kapok | Species::Baobab | Species::TallPine), "{:?}", p.species);
        assert!(Vec2::new(p.pos.x as f32, p.pos.z as f32).distance(Vec2::new(49300.0, 31800.0)) <= REACH);
        // Asked again, the same tree, roots and all.
        let again = tree();
        assert_eq!((again.pos, again.species, again.variant, again.turns), (p.pos, p.species, p.variant, p.turns));
        assert!(p.roots.is_some_and(|r| r.flared()), "a giant's foot flares");
        // It stands in the world where it was found.
        let out = w.generate_column_bare(p.pos.x.div_euclid(voxel::CHUNK), p.pos.z.div_euclid(voxel::CHUNK));
        assert!(out.trees.iter().any(|q| q.pos == p.pos && q.species == p.species), "the tree is not in the world");
        // No giant stands nearer the village's middle.
        let near = village_tree(&c, w.seed(), &c.trails, w.library(), Vec2::new(p.pos.x as f32, p.pos.z as f32), m(2.0));
        assert_eq!(near.map(|q| q.pos), Some(p.pos));
    }

    #[test]
    fn a_village_dresses_its_tree_more_the_older_it_is_and_the_same_every_time() {
        let w = world();
        let c = w.continent(0, 0);
        let p = tree();
        let young = dress_at(&c, w.seed(), &c.trails, w.library(), &p, 0.0);
        let old = dress_at(&c, w.seed(), &c.trails, w.library(), &p, 1.0);
        let again = dress_at(&c, w.seed(), &c.trails, w.library(), &p, 1.0);
        let parts = |d: &Dressing| d.fine.iter().map(|b| (b.lo, b.hi, b.voxel, b.season, b.flap)).collect::<Vec<_>>();
        assert_eq!(parts(&old), parts(&again));
        assert_eq!(old.knots, again.knots);
        assert!((3..=5).contains(&young.knots.len()), "{} ribbons on a young village's tree", young.knots.len());
        assert!(old.knots.len() >= 16, "only {} ribbons on an ancient village's tree", old.knots.len());
        assert!(old.fine.len() > young.fine.len());
        // Every dye is there on the old tree.
        for dye in DYES {
            assert!(old.fine.iter().any(|b| b.voxel == dye), "no ribbon dyed {dye:?}");
        }
        // Flowers come and go with their months; nothing else does.
        assert!(old.fine.iter().all(|b| b.season == 0 || b.cloth() != voxel::mesh::Cloth::None || POSIES.iter().any(|f| f.bloom_code() == b.season)));
    }

    #[test]
    fn ribbons_hang_from_the_wood_of_the_tree_and_nothing_floats() {
        let w = world();
        let c = w.continent(0, 0);
        let lib = w.library();
        let p = tree();
        let d = dress_at(&c, w.seed(), &c.trails, lib, &p, 1.0);
        let cells = IVec3::splat(CELLS);
        let wood = |q: IVec3| features::model_voxel(lib, &p, q).is_some_and(roots::is_bark);
        let mut rooted = HashSet::new();
        if let Some(r) = &p.roots {
            r.voxels(p.pos, |q, _| {
                rooted.insert(q);
            });
        }
        // Each knot is tied to the underside of a limb of the tree's own model, or round
        // its trunk, against the bark.
        for k in &d.knots {
            let over = k.div_euclid(cells);
            let beside = [IVec3::X, IVec3::NEG_X, IVec3::Z, IVec3::NEG_Z].iter().any(|&s| wood((*k - IVec3::Y + s).div_euclid(cells)));
            assert!(wood(over) || beside, "the ribbon tied at {k} hangs from nothing");
        }
        assert!(d.knots.iter().all(|k| wood(k.div_euclid(cells))), "an elder oak's ribbons hang from its limbs");
        // Each length of ribbon hangs in the air, above the ground, from the one above it.
        let world_column = |x: i32, z: i32| w.column(x, z).top();
        let mut by_cell: HashSet<IVec3> = HashSet::new();
        for b in d.fine.iter().filter(|b| b.cloth() != voxel::mesh::Cloth::None) {
            let lo = IVec3::from_array(b.lo);
            by_cell.insert(lo);
            let v = lo.div_euclid(cells);
            assert!(features::model_voxel(lib, &p, v).is_none() && !rooted.contains(&v), "ribbon inside the tree at {v}");
            assert!(v.y > world_column(v.x, v.z) + 1, "ribbon at {v} touches the ground");
        }
        for b in d.fine.iter().filter(|b| b.cloth() != voxel::mesh::Cloth::None) {
            let top = IVec3::from_array(b.lo) + IVec3::Y;
            assert!(by_cell.contains(&top) || d.knots.iter().any(|k| *k == top), "a length of ribbon at {:?} hangs from nothing", b.lo);
        }
        // Every offering stands on the ground or on another part of itself, on worn ground,
        // in the clear.
        let offerings: Vec<_> = d.fine.iter().filter(|b| b.cloth() == voxel::mesh::Cloth::None).collect();
        assert!(offerings.len() >= 16, "only {} parts of offerings", offerings.len());
        for b in &offerings {
            let lo = IVec3::from_array(b.lo);
            let v = lo.div_euclid(cells);
            let ground = world_column(v.x, v.z);
            let on_ground = lo.y == (ground + 1) * CELLS;
            let on_part = offerings.iter().any(|o| {
                o.hi[1] == lo.y && o.lo[0] < b.hi[0] && o.hi[0] > b.lo[0] && o.lo[2] < b.hi[2] && o.hi[2] > b.lo[2]
            });
            assert!(on_ground || on_part, "an offering at {lo} floats");
            assert!(features::model_voxel(lib, &p, v).is_none() && !rooted.contains(&v), "an offering inside the tree at {v}");
            assert!(d.ring.worn(w.seed(), Vec2::new(v.x as f32 + 0.5, v.z as f32 + 0.5), v.x, v.z), "an offering at {v} on unworn ground");
        }
    }

    #[test]
    fn the_worn_ring_goes_round_the_trunk_and_never_under_it() {
        let w = world();
        let c = w.continent(0, 0);
        let p = tree();
        let d = dress_at(&c, w.seed(), &c.trails, w.library(), &p, 1.0);
        let ring = d.ring;
        let (cx, cz) = (ring.x.floor() as i32, ring.z.floor() as i32);
        let (mut worn, mut round) = (0, [false; 8]);
        let r = ring.reach().ceil() as i32 + 1;
        for z in cz - r..=cz + r {
            for x in cx - r..=cx + r {
                let q = Vec2::new(x as f32 + 0.5, z as f32 + 0.5);
                let rel = q - Vec2::new(ring.x, ring.z);
                if !ring.worn(w.seed(), q, x, z) {
                    continue;
                }
                assert!(rel.length() >= ring.trunk, "worn inside the trunk at {x},{z}");
                worn += 1;
                if (rel.length() - ring.ring).abs() < 1.0 {
                    round[((rel.y.atan2(rel.x) + std::f32::consts::PI) / std::f32::consts::TAU * 8.0) as usize % 8] = true;
                }
            }
        }
        assert!(worn > 40, "only {worn} columns worn");
        assert!(round.iter().all(|&r| r), "the path does not go all the way round: {round:?}");
        // The tree's own column stays as it was, so the tree is still chosen there.
        let at = Vec2::new(p.pos.x as f32 + 0.5, p.pos.z as f32 + 0.5);
        assert!(!ring.worn(w.seed(), at, p.pos.x, p.pos.z));
    }
}
