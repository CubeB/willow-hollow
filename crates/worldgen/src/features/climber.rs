//! Climbing plants that are grown, not stamped: a stem starts at the base of something,
//! creeps upward in 0.1 m steps, forks and fans out, and a second pass then dresses the
//! finished skeleton in leaves.
//!
//! The generator takes parameters ([`Climber`]), so common ivy, Virginia creeper,
//! flowering vines and hanging lianas are the same code with different settings.
//!
//! A plant is one deterministic run from its root, seeded from where it is, that only
//! ever looks inside [`REACH`] of its root. Every chunk column near the root runs it and
//! keeps the part inside itself, so all of them agree without talking to each other.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use glam::{IVec3, Vec3};
use voxel::{Kind, Materials, Voxel, mesh::CELLS};

use super::{
    Library, Placement,
    attach::{ColumnVoxels, Raster, Voxels},
    shape::Rng,
    trees::Species,
};
use crate::{
    biome::Biome,
    materials as mat,
    noise::{hash3, unit},
    surface::{Column, Cover, Sampler},
};

/// How far a plant can spread from its root, horizontally, in voxels.
pub const REACH: i32 = 28;
/// How far a chunk column looks for plant roots: plants within `REACH` can reach it, and
/// each of those looks a further `REACH` beyond its own root.
pub const MARGIN: i32 = 2 * REACH;

const CELL: i32 = CELLS;
/// The six steps a stem can take.
const STEPS: [IVec3; 6] = [IVec3::X, IVec3::NEG_X, IVec3::Y, IVec3::NEG_Y, IVec3::Z, IVec3::NEG_Z];

/// Where a solid neighbour holds a clinging stem up: walls and the floor, not a ceiling.
const BRACES: [IVec3; 5] = [IVec3::X, IVec3::NEG_X, IVec3::NEG_Y, IVec3::Z, IVec3::NEG_Z];

/// Fewest cells a fork keeps: shorter ones are stubs that read as stray dashes.
const MIN_BRANCH: u32 = 6;

/// Stems may leave a surface for this many cells at a time, to round a corner, reach
/// the far side of a wall top or follow a trunk that steps in by a voxel.
const AIR_GAP: u32 = 4;

/// The time of year as a share of it, from the start of spring: 0.75 is the start of
/// autumn. Fixed in mid autumn until the calendar of #65 exists; flowers and berries are
/// then shown or hidden per cluster against it, with no remeshing of the plant.
pub const SEASON: f32 = 0.82;

/// Part of the year, as start and end shares of it from the start of spring. It may run
/// over the turn of the year.
#[derive(Clone, Copy, Debug)]
pub struct Span(pub f32, pub f32);

impl Span {
    /// Whether `t` falls in the span once it is pushed later by `delay`.
    pub fn holds(self, t: f32, delay: f32) -> bool {
        let (a, b) = ((self.0 + delay).fract(), (self.1 + delay).fract());
        if a <= b { t >= a && t < b } else { t >= a || t < b }
    }
}

/// Settings for one kind of climber.
#[derive(Clone, Copy, Debug)]
pub struct Climber {
    pub name: &'static str,
    pub stem: Voxel,
    /// Stem material where it has thickened into wood, and the darker streaks in it.
    pub wood: Voxel,
    pub streak: Voxel,
    /// Leaves: dark and glossy, ordinary, pale new growth.
    pub leaves: [Voxel; 3],
    /// A share of leaves turned to this colour, if any.
    pub autumn: Option<(Voxel, f32)>,
    /// Flowers on mature plants, and when they are open.
    pub flowers: Option<(Voxel, Span)>,
    /// Berries: unripe and when, then ripe and when.
    pub berries: Option<(Voxel, Span, Voxel, Span)>,
    /// Pull straight up (negative to hang down).
    pub up: f32,
    /// Keeping to the previous direction.
    pub inertia: f32,
    pub wander: f32,
    /// Attraction to staying on a surface.
    pub stick: f32,
    /// Leaning toward the sky and the sun.
    pub light: f32,
    /// True for plants that hang free instead of clinging.
    pub hanging: bool,
    /// Total stem length in cells for a young and an ancient plant.
    pub stem_budget: (u32, u32),
    /// Highest a plant reaches above its root, in cells.
    pub max_height: i32,
    /// How often a tip forks, as a chance per cell after a long stretch.
    pub branch_rate: f32,
    pub max_generations: u8,
    /// Cells between leaves along a stem.
    pub leaf_gap: u32,
    /// The outline of one leaf of a clinging plant, as cells across (u) and from the
    /// stalk toward the tip (v). Every leaf has the same size.
    pub leaf: &'static [(i32, i32)],
    /// Share of possible leaves actually placed.
    pub leaf_density: f32,
    /// Leaves bunched in clusters along the stem rather than spread evenly.
    pub clusters: bool,
    /// Boxes at most for one plant.
    pub box_budget: usize,
}

/// Ivy's leaf: two basal lobes either side of the stalk, a broad middle and a pointed
/// tip, 0.3 m across.
const IVY_LEAF_SHAPE: [(i32, i32); 6] = [(-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1), (0, 2)];
/// Virginia creeper's: leaflets fanning out from the stalk like a hand.
const CREEPER_LEAF_SHAPE: [(i32, i32); 6] = [(0, 0), (-1, 1), (0, 1), (1, 1), (-1, 2), (1, 2)];
/// A plain pointed oval.
const VINE_LEAF_SHAPE: [(i32, i32); 5] = [(0, 0), (-1, 1), (0, 1), (1, 1), (0, 2)];

pub const IVY: Climber = Climber {
    name: "common ivy",
    stem: mat::IVY_TWIG,
    wood: mat::IVY_ROPE,
    streak: mat::IVY_ROPE_DARK,
    leaves: [mat::IVY_GLOSS, mat::IVY_GLOSS_LIT, mat::IVY_GLOSS_NEW],
    autumn: None,
    flowers: Some((mat::IVY_FLOWER, Span(0.72, 0.92))),
    berries: Some((mat::IVY_BERRY_GREEN, Span(0.92, 0.12), mat::IVY_BERRY, Span(0.12, 0.3))),
    up: 1.0,
    inertia: 1.3,
    wander: 0.55,
    stick: 1.8,
    light: 0.25,
    hanging: false,
    stem_budget: (90, 900),
    max_height: 180,
    branch_rate: 0.09,
    max_generations: 4,
    leaf_gap: 3,
    leaf: &IVY_LEAF_SHAPE,
    leaf_density: 0.85,
    clusters: false,
    box_budget: 900,
};

pub const VIRGINIA_CREEPER: Climber = Climber {
    name: "virginia creeper",
    stem: mat::IVY_TWIG,
    wood: mat::IVY_ROPE,
    streak: mat::IVY_ROPE_DARK,
    leaves: [mat::CREEPER_GREEN_DARK, mat::CREEPER_GREEN, mat::CREEPER_GREEN],
    autumn: Some((mat::CREEPER_SCARLET, 0.6)),
    flowers: None,
    berries: Some((mat::IVY_BERRY_GREEN, Span(0.55, 0.72), mat::IVY_BERRY, Span(0.72, 0.9))),
    up: 1.15,
    inertia: 1.1,
    wander: 0.8,
    stick: 1.6,
    light: 0.45,
    hanging: false,
    stem_budget: (70, 650),
    max_height: 160,
    branch_rate: 0.12,
    max_generations: 4,
    leaf_gap: 4,
    leaf: &CREEPER_LEAF_SHAPE,
    leaf_density: 0.7,
    clusters: false,
    box_budget: 700,
};

/// Climbing rose and honeysuckle: fewer stems, showy flowers, near people.
pub const FLOWERING_VINE: Climber = Climber {
    name: "flowering vine",
    stem: mat::IVY_TWIG,
    wood: mat::IVY_ROPE,
    streak: mat::IVY_ROPE_DARK,
    leaves: [mat::VINE_GREEN, mat::VINE_GREEN, mat::VINE_GREEN],
    autumn: None,
    flowers: Some((mat::VINE_FLOWER, Span(0.25, 0.85))),
    berries: None,
    up: 1.0,
    inertia: 1.0,
    wander: 0.7,
    stick: 1.6,
    light: 0.5,
    hanging: false,
    stem_budget: (50, 400),
    max_height: 100,
    branch_rate: 0.1,
    max_generations: 3,
    leaf_gap: 3,
    leaf: &VINE_LEAF_SHAPE,
    leaf_density: 0.6,
    clusters: false,
    box_budget: 450,
};

/// Lianas hanging from canopies and cliff edges in wet country: they fall free instead
/// of climbing.
pub const LIANA: Climber = Climber {
    name: "liana",
    stem: mat::LIANA_STEM,
    wood: mat::LIANA_WOOD,
    streak: mat::LIANA_WOOD,
    leaves: [mat::VINE_LEAF, mat::IVY_LEAF, mat::VINE_LEAF],
    autumn: None,
    flowers: None,
    berries: None,
    up: -2.0,
    inertia: 1.5,
    wander: 0.3,
    stick: 0.0,
    light: 0.0,
    hanging: true,
    stem_budget: (120, 700),
    max_height: 0,
    branch_rate: 0.015,
    max_generations: 2,
    leaf_gap: 1,
    leaf: &VINE_LEAF_SHAPE,
    leaf_density: 0.9,
    clusters: true,
    box_budget: 320,
};

/// A low creeper sprawling over fallen logs and boulders on a wet forest floor: hardly
/// climbs, keeps close to what it covers.
pub const TRAILING_CREEPER: Climber = Climber {
    name: "trailing creeper",
    stem: mat::IVY_TWIG,
    wood: mat::IVY_ROPE,
    streak: mat::IVY_ROPE_DARK,
    leaves: [mat::CREEPER_GREEN_DARK, mat::VINE_GREEN, mat::VINE_GREEN],
    autumn: None,
    flowers: None,
    berries: None,
    up: 0.15,
    inertia: 1.0,
    wander: 0.9,
    stick: 2.4,
    light: 0.2,
    hanging: false,
    stem_budget: (60, 380),
    max_height: 40,
    branch_rate: 0.14,
    max_generations: 3,
    leaf_gap: 3,
    leaf: &CREEPER_LEAF_SHAPE,
    leaf_density: 0.85,
    clusters: false,
    box_budget: 400,
};

/// Where a plant starts: a cell just off a surface, with the way out of the surface and
/// how far along in life the plant is (0 young, 1 ancient).
#[derive(Clone, Copy, Debug)]
pub struct Root {
    pub cell: IVec3,
    pub out: IVec3,
    pub age: f32,
    pub seed: u32,
}

/// Solid things and water stop a stem; leaves do not.
fn solid(v: Voxel, mats: &Materials) -> bool {
    matches!(mats.get(v).kind, Kind::Solid | Kind::Water)
}

/// One step of the grown skeleton.
#[derive(Clone, Copy, Debug)]
pub struct Node {
    pub cell: IVec3,
    /// Direction of growth into this cell.
    pub dir: IVec3,
    /// From the surface it clings to, or zero if hanging free.
    pub out: IVec3,
    /// The node it grew from, or `u32::MAX` for the root.
    pub parent: u32,
    /// 0 for the main stem, one more for each fork since.
    pub generation: u8,
    /// How much plant hangs off this node: this plus everything grown beyond it.
    pub load: u32,
}

/// The grown stems of one plant, in the order they were laid down.
pub struct Skeleton {
    pub nodes: Vec<Node>,
}

/// Chooses for a plant as it grows, reading the world through a small cache: a plant
/// asks about the same voxels over and over, and reading them from features is the
/// expensive part.
struct Sensing<'a, V: Voxels> {
    world: &'a V,
    mats: &'a Materials,
    cache: HashMap<IVec3, bool>,
}

impl<V: Voxels> Sensing<'_, V> {
    fn solid_voxel(&mut self, p: IVec3) -> bool {
        match self.cache.get(&p) {
            Some(&s) => s,
            None => {
                let s = solid(self.world.voxel(p), self.mats);
                self.cache.insert(p, s);
                s
            }
        }
    }

    /// Whether a cell is free of rock, wood and water.
    fn air(&mut self, cell: IVec3) -> bool {
        !self.solid_voxel(cell.div_euclid(IVec3::splat(CELL)))
    }

    /// The direction from a solid neighbouring cell to this one, or zero. Unit steps only,
    /// so this picks the steady nearest solid surface.
    fn surface_out(&mut self, cell: IVec3) -> IVec3 {
        for s in STEPS {
            if !self.air(cell + s) {
                return -s;
            }
        }
        IVec3::ZERO
    }

    /// Whether a cell rests against a wall or lies on a floor, as opposed to clinging only
    /// to a ceiling or floating free.
    fn braced(&mut self, cell: IVec3) -> bool {
        BRACES.iter().any(|&s| !self.air(cell + s))
    }

    /// How open the sky is straight overhead, 0..1.
    fn sky(&mut self, cell: IVec3) -> f32 {
        let at = cell.div_euclid(IVec3::splat(CELL));
        let mut hits = 0;
        for k in 1..=10 {
            // Leaves shade a little, solid things more.
            let v = self.world.voxel(at + IVec3::Y * k * 2);
            match self.mats.get(v).kind {
                Kind::Solid => hits += 3,
                Kind::Foliage => hits += 1,
                _ => {}
            }
        }
        (1.0 - hits as f32 / 8.0).clamp(0.0, 1.0)
    }
}

/// A growing tip.
struct Tip {
    node: u32,
    dir: Vec3,
    generation: u8,
    /// Cells since this tip last forked.
    since: u32,
    /// Cells it can still grow; zero once it has stopped.
    life: u32,
    /// Cells off a surface.
    air: u32,
    /// How open the sky last looked here, 0..1. Checked every few steps: it is the
    /// costliest thing a tip does and hardly changes.
    sky: f32,
}

/// How much a generation's forking is held back: later ones fork less.
const fn fork_rate(generation: u8) -> f32 {
    match generation {
        0 => 1.0,
        1 => 0.8,
        2 => 0.55,
        _ => 0.35,
    }
}

/// Grows a plant's stems from its root. Each round gives every living tip one step, so
/// they lengthen together; the run stops at the budget, at the plant's height and reach
/// limits, or when every tip has died.
pub fn grow_stems(params: &Climber, root: &Root, world: &impl Voxels, mats: &Materials) -> Skeleton {
    let mut rng = Rng::new(root.seed);
    let mut sense = Sensing { world, mats, cache: HashMap::new() };
    let budget = params.stem_budget.0 + ((params.stem_budget.1 - params.stem_budget.0) as f32 * root.age) as u32;
    let mut nodes = vec![Node { cell: root.cell, dir: IVec3::Y, out: root.out, parent: u32::MAX, generation: 0, load: 0 }];
    let mut occupied: HashSet<IVec3> = HashSet::from([root.cell]);
    let first = if params.hanging {
        Vec3::NEG_Y
    } else if root.out == IVec3::Y {
        // Starting on open ground: creep off in some direction until something upright.
        rng.heading().0
    } else {
        Vec3::Y
    };
    let mut tips = vec![Tip { node: 0, dir: first, generation: 0, since: 0, life: budget, air: 0, sky: 0.5 }];
    let reach = REACH * CELL;
    let top = root.cell.y + params.max_height;
    let bottom = root.cell.y - if params.hanging { 700 } else { 8 };

    let mut steps = 0u32;
    while steps < budget && !tips.is_empty() {
        for i in 0..tips.len() {
            if steps >= budget {
                break;
            }
            steps += 1;
            let tip = &tips[i];
            let (c, dir, generation, since, air, node_index) = (nodes[tip.node as usize].cell, tip.dir, tip.generation, tip.since, tip.air, tip.node);
            let out_here = nodes[node_index as usize].out;
            // A hanging stem that has come down onto something rests there.
            if params.hanging && !sense.air(c - IVec3::Y) {
                tips[i].life = 0;
                continue;
            }
            let sky =if since % 6 == 0 { sense.sky(c) } else { tip.sky };
            let shade = 1.0 - sky;

            // The direction the plant would like to go.
            // Later generations spread sideways more than they climb.
            let up = Vec3::Y * params.up * (1.0 - 0.3 * shade) / (1.0 + generation as f32 * 0.7);
            let (h, _) = rng.heading();
            let wander = Vec3::new(h.x, (rng.unit() - 0.5) * 0.4, h.z) * params.wander;
            // The sun tracks across the +Z side of the sky.
            let light = Vec3::new(0.0, 0.2, 1.0) * params.light * (0.4 + sky);
            // Off the surface, the pull back to it grows with every cell.
            let back = -out_here.as_vec3() * params.stick * air as f32;
            let desired = up + dir * params.inertia + wander + light + back;

            // Pick the best free neighbouring cell.
            let mut best: Option<(f32, IVec3, IVec3, bool)> = None;
            for &step in &STEPS {
                let next = c + step;
                if !sense.air(next) || occupied.contains(&next) {
                    continue;
                }
                if next.y > top || next.y < bottom {
                    continue;
                }
                if (next.x - root.cell.x).abs() > reach || (next.z - root.cell.z).abs() > reach {
                    continue;
                }
                let out = sense.surface_out(next);
                let attached = out != IVec3::ZERO;
                if !params.hanging && !attached && air + 1 > AIR_GAP {
                    continue;
                }
                // No doubling straight back, unless that is the way back to a surface.
                if step.as_vec3().dot(dir) < -0.85 && !(attached && air > 0) {
                    continue;
                }
                let mut score = desired.dot(step.as_vec3());
                if attached {
                    score += params.stick;
                    // Clinging plants avoid the undersides of things: a stem may cross one
                    // to reach another surface, but should not wander about under it.
                    if !params.hanging && !sense.braced(next) {
                        score -= 0.8;
                    }
                }
                // Tips avoid running over each other, so the plant keeps a fan shape.
                let crowded = STEPS.iter().filter(|s| occupied.contains(&(next + **s))).count() as f32;
                score -= (crowded - 1.0).max(0.0) * 0.9;
                score += rng.unit() * 0.25;
                if best.is_none_or(|b| score > b.0) {
                    best = Some((score, next, out, attached));
                }
            }
            let Some((_, next, out, attached)) = best else {
                tips[i].life = 0;
                continue;
            };

            // Lay down the step.
            let step = next - c;
            let id = nodes.len() as u32;
            let out_final = if attached { out } else { out_here };
            nodes.push(Node {
                cell: next,
                dir: step,
                out: if params.hanging && !attached { IVec3::ZERO } else { out_final },
                parent: node_index,
                generation,
                load: 0,
            });
            occupied.insert(next);
            let new_dir = (dir * 0.92 + step.as_vec3() * 0.08).normalize_or(Vec3::Y);
            let t = &mut tips[i];
            t.node = id;
            t.dir = new_dir;
            t.since = since + 1;
            t.life = t.life.saturating_sub(1);
            t.air = if attached { 0 } else { air + 1 };
            t.sky = sky;

            // Dying: running out of life, or a chance that grows in deep shade.
            let die_chance = 0.002 + shade * 0.004 + generation as f32 * 0.005;
            if t.life == 0 || rng.unit() < die_chance {
                t.life = 0;
                continue;
            }

            // Forking: more likely the longer since the last fork.
            let chance = params.branch_rate * (since as f32 / 20.0).clamp(0.2, 3.0) * fork_rate(generation);
            if generation < params.max_generations && since >= 4 && rng.unit() < chance {
                let surface = if out_final == IVec3::ZERO { IVec3::Z } else { out_final };
                let side = surface.as_vec3().cross(new_dir).normalize_or(Vec3::X);
                let sign = if rng.unit() < 0.5 { 1.0 } else { -1.0 };
                let branch_dir = (new_dir * 0.5 + side * sign).normalize_or(new_dir);
                let life = (tips[i].life as f32 * 0.55) as u32 + 8;
                tips[i].since = 0;
                tips.push(Tip {
                    node: id,
                    dir: branch_dir,
                    generation: generation + 1,
                    since: 0,
                    life,
                    air: 0,
                    sky,
                });
            }
        }
        tips.retain(|t| t.life > 0);
    }

    let mut nodes = prune(params, nodes, &mut sense);
    // Each node's load is itself plus everything grown beyond it.
    for k in (1..nodes.len()).rev() {
        nodes[k].load += 1;
        let parent = nodes[k].parent as usize;
        let carried = nodes[k].load;
        nodes[parent].load += carried;
    }
    nodes[0].load += 1;
    Skeleton { nodes }
}

/// Cuts what would dangle from a clinging plant: stems that end under a ceiling or out
/// in the air, tips that run down to the bottom edge of something and stop there like
/// an icicle, and forks too short to read as anything but a stray dash. A stem may still
/// turn under an edge or cross a gap, as long as it goes on to another surface. Hanging
/// plants are meant to dangle and are left alone. Loads are left for the caller.
fn prune<V: Voxels>(params: &Climber, nodes: Vec<Node>, sense: &mut Sensing<'_, V>) -> Vec<Node> {
    let n = nodes.len();
    if params.hanging || n < 2 {
        return nodes;
    }
    let braced: Vec<bool> = nodes.iter().map(|node| sense.braced(node.cell)).collect();
    // Children come after their parents, so a backward sweep sees a node's whole stem
    // beyond it and a forward one sees its parent settled. Each cut can leave a new end
    // that needs cutting in turn, so sweep until nothing changes.
    let mut alive = vec![true; n];
    let mut count = n;
    loop {
        let mut children = vec![0u32; n];
        for k in 1..n {
            if alive[k] {
                children[nodes[k].parent as usize] += 1;
            }
        }
        // Tips that came down to an edge with nothing under them lose the run down.
        for k in 1..n {
            let below = nodes[k].cell - IVec3::Y;
            if !alive[k] || children[k] != 0 || !sense.air(below) || sense.braced(below) {
                continue;
            }
            let mut m = k;
            while m != 0 && nodes[m].dir == IVec3::NEG_Y && (m == k || children[m] == 1) {
                alive[m] = false;
                m = nodes[m].parent as usize;
            }
        }
        // A node stays if it is braced or leads on to one that is; and how many cells
        // stay from each node on, to find the stubs.
        let mut leads = vec![false; n];
        let mut beyond = vec![0u32; n];
        leads[0] = true;
        for k in (1..n).rev() {
            if !alive[k] {
                continue;
            }
            leads[k] |= braced[k];
            if leads[k] {
                let p = nodes[k].parent as usize;
                leads[p] = true;
                beyond[k] += 1;
                beyond[p] += beyond[k];
            }
        }
        for k in 1..n {
            let p = nodes[k].parent as usize;
            let fork = nodes[p].generation != nodes[k].generation;
            alive[k] = alive[k] && leads[k] && alive[p] && !(fork && beyond[k] < MIN_BRANCH);
        }
        let now = alive.iter().filter(|a| **a).count();
        if now == count {
            break;
        }
        count = now;
    }
    let mut index = vec![u32::MAX; n];
    let mut out = Vec::with_capacity(count);
    for k in 0..n {
        if !alive[k] {
            continue;
        }
        let mut node = nodes[k];
        if k > 0 {
            node.parent = index[node.parent as usize];
        }
        node.load = 0;
        index[k] = out.len() as u32;
        out.push(node);
    }
    out
}

/// How thick a stem is at a node, in cells, from the share of the plant it carries
/// (`load` of `total`): the woody rope at the base of an old plant, thinning upward.
fn thickness(load: u32, total: u32, age: f32) -> i32 {
    let share = load as f32 / total.max(1) as f32;
    let mut t = 1;
    if load > 50 && share > 0.3 {
        t = 2;
    }
    if load > 200 && share > 0.6 {
        t = 3;
    }
    if load > 500 && share > 0.85 && age > 0.75 {
        t = 4;
    }
    t.min(1 + (age * 4.0) as i32)
}

/// Turns a skeleton into stem cells. Everything goes in air only.
pub fn raster_stems(params: &Climber, root: &Root, skel: &Skeleton, world: &impl Voxels, mats: &Materials, raster: &mut Raster) {
    let mut sense = Sensing { world, mats, cache: HashMap::new() };
    let total = skel.nodes.first().map_or(1, |n| n.load);
    let phase = unit(root.seed) * std::f32::consts::TAU;
    let twist = (root.seed % 7) as i32;
    for node in &skel.nodes {
        let mut t = thickness(node.load, total, root.age);
        // Hanging cords stay thin.
        if params.hanging {
            t = t.min(2);
        }
        let wood = t >= 2;
        let out = if node.out == IVec3::ZERO { IVec3::Z } else { node.out };
        let side = perpendicular(out, node.dir);
        // Old wood is several stems twisted together: it pinches and swells, and the
        // rope drifts from side to side around the line it grew along.
        let along = (node.cell.y + node.cell.x + node.cell.z) as f32;
        let (mut width, mut shift) = (t, 0);
        if t >= 3 {
            width = t - ((along * 0.31 + phase).sin() > 0.55) as i32;
            shift = ((along * 0.17 + phase).sin() * 1.3).round() as i32;
        }
        for a in 0..width {
            for b in 0..width {
                // Round off the outer corners of a thick rope.
                if width >= 3 && a >= 1 && (b == 0 || b == width - 1) && (a + b) % 2 == 0 {
                    continue;
                }
                let cell = node.cell + out * a + side * (b + shift);
                if !sense.air(cell) {
                    continue;
                }
                let voxel = if !wood {
                    params.stem
                } else if width >= 3 && b == (node.cell.y.div_euclid(5) + twist).rem_euclid(width) {
                    // One darker strand winding slowly round an old rope: long runs, so
                    // the rope reads as one line and not as dashes.
                    params.streak
                } else {
                    params.wood
                };
                raster.set(cell, voxel);
            }
        }
    }
}

/// An axis perpendicular to both given axes (which may be parallel).
fn perpendicular(a: IVec3, b: IVec3) -> IVec3 {
    let c = a.cross(b);
    if c != IVec3::ZERO {
        return c.signum();
    }
    if a.x == 0 && a.y == 0 { IVec3::X } else { IVec3::Z }
}

/// A lobed leaf outline, as cells across (u) and along (v) the leaf, for `size` 0 to 2.
fn leaf_shape(size: u32, variant: u32) -> &'static [(i32, i32)] {
    const SMALL_A: [(i32, i32); 2] = [(0, 0), (0, 1)];
    const SMALL_B: [(i32, i32); 2] = [(0, 0), (1, 0)];
    const MID_A: [(i32, i32); 4] = [(0, 0), (-1, 0), (0, 1), (1, 0)];
    const MID_B: [(i32, i32); 4] = [(0, 0), (1, 0), (0, 1), (1, 1)];
    const BIG_A: [(i32, i32); 7] = [(0, -1), (-1, 0), (0, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];
    const BIG_B: [(i32, i32); 7] = [(-1, -1), (0, -1), (1, -1), (0, 0), (-1, 1), (0, 1), (1, 1)];
    match (size, variant % 2) {
        (0, 0) => &SMALL_A,
        (0, _) => &SMALL_B,
        (1, 0) => &MID_A,
        (1, _) => &MID_B,
        (_, 0) => &BIG_A,
        _ => &BIG_B,
    }
}

/// The second pass: walks each stem and dresses it in leaves. Separate from the stems so
/// it can be tuned or dropped on its own. `leaves` is the share of leaves to place.
pub fn raster_leaves(params: &Climber, root: &Root, skel: &Skeleton, world: &impl Voxels, mats: &Materials, raster: &mut Raster, leaves: f32) {
    if leaves <= 0.0 {
        return;
    }
    if params.hanging {
        hanging_leaves(params, root, skel, world, mats, raster, leaves);
    } else {
        clinging_leaves(params, root, skel, world, mats, raster, leaves);
    }
}

/// One way to lay a leaf: where its stalk end sits, and the directions across it and
/// from stalk to tip.
#[derive(Clone, Copy)]
struct Lay {
    origin: IVec3,
    across: IVec3,
    down: IVec3,
}

impl Lay {
    fn at(self, (u, v): (i32, i32)) -> IVec3 {
        self.origin + self.across * u + self.down * v
    }
}

/// Where a leaf off `node` may go, best first, on side `sign` of the stem by preference.
/// Leaves lie flat one cell off the surface, over the stem's own layer, clear of the
/// stem itself so it shows as a line between them. On a wall they hang tip down whichever
/// way the stem runs; on the ground they point away from the stem. Nothing grows off a
/// ceiling or from a stem out in the air.
fn leaf_lays(node: &Node, sign: i32) -> Vec<Lay> {
    let out = node.out;
    if out == IVec3::ZERO || out == IVec3::NEG_Y {
        return Vec::new();
    }
    let lift = node.cell + out;
    if out == IVec3::Y {
        let along = if node.dir.y == 0 && node.dir != out { node.dir } else { IVec3::X };
        let side = perpendicular(out, along);
        return [sign, -sign].map(|s| Lay { origin: lift + side * s, across: along, down: side * s }).to_vec();
    }
    let across = perpendicular(out, IVec3::Y);
    if node.dir.y != 0 {
        // Up a stem: beside it, the basal lobes level with the cell above.
        [2 * sign, -2 * sign, sign, -sign].map(|k| Lay { origin: lift + across * k + IVec3::Y, across, down: IVec3::NEG_Y }).to_vec()
    } else {
        // Along a stem: below it or above it, a cell clear either way.
        let below = Lay { origin: lift - IVec3::Y * 2, across, down: IVec3::NEG_Y };
        let above = Lay { origin: lift + IVec3::Y * 4, across, down: IVec3::NEG_Y };
        if sign > 0 { vec![below, above] } else { vec![above, below] }
    }
}

/// Leaves on a clinging plant, one shape ([`Climber::leaf`]) of one size, every
/// `leaf_gap` cells along each stem and on alternate sides of it, so they overlap down
/// the stem like shingles. A leaf goes in whole or not at all: every cell of it held a
/// cell off the surface it grows on, so none overhangs an edge, and none touching
/// another leaf, so the surface and stem show between them. Colours come in patches
/// over the plant, not leaf by leaf.
fn clinging_leaves(params: &Climber, root: &Root, skel: &Skeleton, world: &impl Voxels, mats: &Materials, raster: &mut Raster, leaves: f32) {
    let mut sense = Sensing { world, mats, cache: HashMap::new() };
    let mut rng = Rng::new(root.seed ^ 0x1ea7);
    let nodes = &skel.nodes;
    let total = nodes.first().map_or(1, |n| n.load);
    // Distance from the root along the stems, and the fork each node's branch began at.
    let mut dist = vec![0u32; nodes.len()];
    let mut branch = vec![0u32; nodes.len()];
    for k in 1..nodes.len() {
        let p = nodes[k].parent as usize;
        dist[k] = dist[p] + 1;
        branch[k] = if nodes[p].generation != nodes[k].generation { k as u32 } else { branch[p] };
    }
    let gap = params.leaf_gap.max(1);
    let mut placed: HashSet<IVec3> = HashSet::new();
    let mut cells = Vec::with_capacity(params.leaf.len());
    for (k, node) in nodes.iter().enumerate() {
        // Bare tips where it is still growing.
        if !dist[k].is_multiple_of(gap) || node.load < 3 {
            continue;
        }
        // Bare wood at the base of old plants.
        let t = thickness(node.load, total, root.age);
        if (t >= 3 && rng.unit() < 0.85) || (t == 2 && rng.unit() < 0.4) {
            continue;
        }
        if rng.unit() > params.leaf_density * leaves {
            continue;
        }
        let sign = if (dist[k] / gap).is_multiple_of(2) { 1 } else { -1 };
        let out = node.out;
        let mut fits = |c: IVec3| {
            sense.air(c)
                && !raster.taken(c)
                && sense.air(c - out)
                && !sense.air(c - out * 2)
                && !(-1..=1).any(|x| (-1..=1).any(|y| (-1..=1).any(|z| placed.contains(&(c + IVec3::new(x, y, z))))))
        };
        let Some(lay) = leaf_lays(node, sign).into_iter().find(|&lay| params.leaf.iter().all(|&uv| fits(lay.at(uv)))) else {
            continue;
        };
        cells.clear();
        cells.extend(params.leaf.iter().map(|&uv| lay.at(uv)));
        let colour = patch_colour(params, root, node, branch[k]);
        for &c in &cells {
            raster.set(c, colour);
            placed.insert(c);
        }
    }
}

/// A leaf's colour, the same over patches of a plant: turned leaves a branch at a time,
/// fresh growth at the very tips, and two close greens over areas a metre or two across.
fn patch_colour(params: &Climber, root: &Root, node: &Node, branch: u32) -> Voxel {
    if let Some((voxel, share)) = params.autumn
        && unit(hash3(root.seed ^ 0xa07, branch as i32, 0, 0)) < share
    {
        return voxel;
    }
    if node.load < 6 {
        return params.leaves[2];
    }
    let patch = node.cell.div_euclid(IVec3::splat(12));
    if unit(hash3(root.seed ^ 0x5ade, patch.x, patch.y, patch.z)) < 0.55 { params.leaves[0] } else { params.leaves[1] }
}

/// Leaves on a hanging cord: bunched in clusters along it, at alternating nodes.
fn hanging_leaves(params: &Climber, root: &Root, skel: &Skeleton, world: &impl Voxels, mats: &Materials, raster: &mut Raster, leaves: f32) {
    let mut sense = Sensing { world, mats, cache: HashMap::new() };
    let mut rng = Rng::new(root.seed ^ 0x1ea7);
    let longest = skel.nodes.iter().map(|n| n.load).max().unwrap_or(1).max(1);
    for (i, node) in skel.nodes.iter().enumerate() {
        // Bare wood at the base of old plants, and bare tips where it is still growing.
        let t = thickness(node.load, longest, root.age);
        if !(i as u32).is_multiple_of(params.leaf_gap) || node.load < 2 {
            continue;
        }
        // In clusters: a few nodes in leaf, then a bare stretch of cord.
        if params.clusters && !hash3(root.seed, i as i32 / 6, 0, 0).is_multiple_of(2) {
            continue;
        }
        if (t >= 3 && rng.unit() < 0.85) || (t == 2 && rng.unit() < 0.4) {
            continue;
        }
        if rng.unit() > params.leaf_density * leaves {
            continue;
        }
        let out = if node.out == IVec3::ZERO {
            IVec3::new(if rng.unit() < 0.5 { 1 } else { -1 }, 0, 0)
        } else {
            node.out
        };
        let side = perpendicular(out, node.dir);
        let sign = if (i as u32 / params.leaf_gap).is_multiple_of(2) { 1 } else { -1 };
        // Lean a leaf out from the stem, sitting one cell off the surface.
        let centre = node.cell + side * sign + out;
        // How far along: bigger towards the base, fresh and small at the tips.
        let along = 1.0 - (node.load as f32 / longest as f32).clamp(0.0, 1.0);
        let size = if along < 0.3 { 2 } else if along < 0.7 { 1 } else { 0 };
        let down = if node.dir.y.abs() > 0 { node.dir } else { perpendicular(out, side) * -1 };
        let colour = leaf_colour(params, along, &mut rng);
        for &(u, v) in leaf_shape(size, rng.next_u32()) {
            // In the plane of the surface, the leaf's tip hanging down from its stalk.
            let cell = centre + side * u + down * v;
            if sense.air(cell) {
                raster.set(cell, colour);
            }
        }
    }
}

fn leaf_colour(params: &Climber, along: f32, rng: &mut Rng) -> Voxel {
    if let Some((voxel, share)) = params.autumn
        && rng.unit() < share
    {
        return voxel;
    }
    if along > 0.75 && rng.unit() < 0.7 {
        return params.leaves[2];
    }
    if rng.unit() < 0.4 { params.leaves[0] } else { params.leaves[1] }
}

/// What a flower or berry cluster shows at time of year `season`, or nothing. Each
/// cluster has its own `threshold` (0..1), which delays it a little, so a plant comes
/// into flower and fruit a cluster at a time.
pub fn cluster_at(params: &Climber, season: f32, threshold: f32) -> Option<Voxel> {
    let delay = threshold * 0.08;
    if let Some((voxel, open)) = params.flowers
        && open.holds(season, delay)
    {
        return Some(voxel);
    }
    let (green, unripe, ripe, ripened) = params.berries?;
    if unripe.holds(season, delay) {
        Some(green)
    } else if ripened.holds(season, delay) {
        Some(ripe)
    } else {
        None
    }
}

/// Flowers and berries, placed high on mature plants in the light, where real ivy stops
/// climbing and flowers. Each cluster is a few tiny cubes with its own threshold.
pub fn raster_fruit(params: &Climber, root: &Root, skel: &Skeleton, world: &impl Voxels, mats: &Materials, raster: &mut Raster) {
    if root.age < 0.45 || (params.flowers.is_none() && params.berries.is_none()) {
        return;
    }
    let mut sense = Sensing { world, mats, cache: HashMap::new() };
    let mut rng = Rng::new(root.seed ^ 0xf01e);
    let top = skel.nodes.iter().map(|n| n.cell.y).max().unwrap_or(root.cell.y);
    let high = root.cell.y + ((top - root.cell.y) as f32 * 0.55) as i32;
    for (i, node) in skel.nodes.iter().enumerate() {
        // Nothing hangs off a ceiling or out in the air.
        if i % 11 != 0 || node.cell.y < high || node.out == IVec3::ZERO || node.out == IVec3::NEG_Y {
            continue;
        }
        if rng.unit() > 0.5 || sense.sky(node.cell) < 0.3 {
            continue;
        }
        let Some(voxel) = cluster_at(params, SEASON, rng.unit()) else { continue };
        let side = perpendicular(node.out, node.dir);
        let across = side.cross(node.out);
        let cells = [(0, 0), (1, 0), (0, 1)].map(|(a, b)| node.cell + node.out + side * a + across * b);
        // Whole clusters over the surface only, never a stray cube or two.
        let out = node.out;
        if !cells.iter().all(|&c| sense.air(c) && !raster.taken(c) && !sense.air(c - out * 2)) {
            continue;
        }
        for cell in cells {
            raster.set(cell, voxel);
        }
    }
}

/// Which climber suits a place, or `None` if nothing should grow there. `warm` is for
/// flowering vines.
pub fn suited(biome: Biome, warm: bool) -> Option<Climber> {
    match biome {
        Biome::Rainforest => Some(if warm { FLOWERING_VINE } else { IVY }),
        Biome::OldGrowth | Biome::Deciduous => Some(IVY),
        Biome::Woodland => Some(if warm { VIRGINIA_CREEPER } else { IVY }),
        Biome::Taiga | Biome::Heath | Biome::Grassland => Some(VIRGINIA_CREEPER),
        Biome::Savanna => Some(FLOWERING_VINE),
        _ => None,
    }
}

/// Whether a host material can carry a climber at all.
pub fn climbable(v: Voxel) -> bool {
    matches!(
        v,
        mat::BARK_OAK
            | mat::BARK_BIRCH
            | mat::BARK_PINE
            | mat::BARK_REDWOOD
            | mat::BARK_JUNGLE
            | mat::BARK_WILLOW
            | mat::BARK_ACACIA
            | mat::BARK_BAOBAB
            | mat::WOOD_DEAD
            | mat::STONE
            | mat::ROCK_DARK
            | mat::MOSSY_STONE
            | mat::SANDSTONE
            | mat::SANDSTONE_DARK
            | mat::PLANK
            | mat::PLANK_PALE
            | mat::PLANK_DARK
            | mat::PLANK_RED
    )
}

/// How likely a tree of this species is to carry ivy: mostly old broadleaves.
fn tree_chance(species: Species) -> f32 {
    match species {
        Species::ElderOak | Species::Kapok | Species::Redwood => 0.45,
        Species::Oak | Species::Maple | Species::Willow | Species::Baobab => 0.22,
        Species::Jungle | Species::Cherry | Species::DeadSnag => 0.14,
        Species::Birch | Species::Acacia => 0.08,
        Species::Spruce | Species::TallPine | Species::Palm => 0.03,
        Species::Boulder | Species::MossyBoulder | Species::FallenLog | Species::Stump => 0.14,
        _ => 0.0,
    }
}

/// A big old tree or standing thing, as far as ivy is concerned: how old it reads.
fn host_age(species: Species) -> f32 {
    match species {
        Species::ElderOak | Species::Redwood | Species::Kapok => 1.0,
        Species::Oak | Species::Maple | Species::Willow | Species::Baobab => 0.65,
        Species::Jungle | Species::Cherry | Species::Birch => 0.45,
        Species::Boulder | Species::MossyBoulder | Species::FallenLog | Species::Stump => 0.35,
        _ => 0.3,
    }
}

/// A place to grow: what a chunk needs to find roots near it and read the world around
/// them. Everything looked at is within [`MARGIN`] of the chunk, and what a plant sees is
/// the same whichever chunk asks.
pub struct Surroundings<'a> {
    pub lib: &'a Library,
    pub mats: &'a Materials,
    pub placements: &'a [Placement],
    pub sampler: &'a Sampler<'a>,
    /// Where continent-local coordinates start, in world voxels.
    pub origin: (i32, i32),
    pub stamps: &'a HashMap<IVec3, Voxel>,
    /// Columns already read: reading one is the slow part of a voxel query.
    columns: RefCell<HashMap<(i32, i32), Column>>,
    /// Which placements can reach each square of [`BUCKET`] voxels, in placement order,
    /// with the heights they span.
    buckets: Buckets,
}

/// Placement indices with the heights they span, by square.
type Buckets = HashMap<(i32, i32), Vec<(u32, i32, i32)>>;

/// Side of the squares placements are filed under for voxel lookups.
const BUCKET: i32 = 8;

impl<'a> Surroundings<'a> {
    pub fn new(lib: &'a Library, mats: &'a Materials, placements: &'a [Placement], sampler: &'a Sampler<'a>, origin: (i32, i32), stamps: &'a HashMap<IVec3, Voxel>) -> Self {
        let mut buckets = Buckets::new();
        for (i, p) in placements.iter().enumerate() {
            let reach = lib.get(p.species, p.variant).reach;
            let (lo, hi) = (p.bottom(), p.top(lib));
            for bz in (p.pos.z - reach).div_euclid(BUCKET)..=(p.pos.z + reach).div_euclid(BUCKET) {
                for bx in (p.pos.x - reach).div_euclid(BUCKET)..=(p.pos.x + reach).div_euclid(BUCKET) {
                    buckets.entry((bx, bz)).or_default().push((i as u32, lo, hi));
                }
            }
        }
        Self { lib, mats, placements, sampler, origin, stamps, columns: RefCell::default(), buckets }
    }

    fn column(&self, x: i32, z: i32) -> Column {
        *self.columns.borrow_mut().entry((x, z)).or_insert_with(|| self.sampler.column(x - self.origin.0, z - self.origin.1))
    }
}

impl Voxels for Surroundings<'_> {
    fn voxel(&self, p: IVec3) -> Voxel {
        // Built things go last in the world and replace whatever was there, air included.
        if let Some(&v) = self.stamps.get(&p) {
            return v;
        }
        let mut leaf = None;
        let near = self.buckets.get(&(p.x.div_euclid(BUCKET), p.z.div_euclid(BUCKET)));
        for &(i, lo, hi) in near.map_or(&[][..], |v| v.as_slice()) {
            if p.y < lo || p.y > hi {
                continue;
            }
            if let Some(v) = super::model_voxel(self.lib, &self.placements[i as usize], p) {
                if self.mats.get(v).kind == Kind::Solid {
                    return v;
                }
                leaf = leaf.or(Some(v));
            }
        }
        let col = self.column(p.x, p.z);
        if p.y <= col.top() {
            return if p.y == col.top() { mat::ground_cover(col.biome) } else { mat::STONE };
        }
        // Leaves only fill what would otherwise be air.
        if col.water_surface().is_some_and(|k| p.y < k) {
            return mat::WATER;
        }
        leaf.unwrap_or(voxel::AIR)
    }
}

/// A chance from a hash of a grid cell.
fn hashed_chance(seed: u32, a: i32, b: i32, salt: u32) -> f32 {
    unit(hash3(seed ^ salt, a, 0, b))
}

/// Trees that carry climbers. For each suitable tree, the cell just off the base of its
/// trunk on a chosen side, found by walking in from outside the trunk.
pub fn tree_root(placement: &Placement, world: &impl Voxels, mats: &Materials, seed: u32, chance_scale: f32) -> Option<Root> {
    let h = hash3(seed ^ 0xa55e, placement.pos.x, placement.pos.y, placement.pos.z);
    let mut rng = Rng::new(h);
    if rng.unit() >= tree_chance(placement.species) * chance_scale {
        return None;
    }
    // Which way round the trunk: mostly the shady -Z side.
    let (dx, dz) = match (rng.unit() * 4.0) as u32 {
        0 | 1 => (0, -1),
        2 => (-1, 0),
        _ => (1, 0),
    };
    // Walk inward from outside the trunk at this height until it hits wood.
    let y = placement.pos.y + 1;
    let out = IVec3::new(dx, 0, dz);
    for dist in (0..16).rev() {
        let probe = IVec3::new(placement.pos.x + dx * dist, y, placement.pos.z + dz * dist);
        if solid(world.voxel(probe), mats) {
            let air = probe + out;
            if solid(world.voxel(air), mats) || !climbable(world.voxel(probe)) {
                return None;
            }
            return Some(Root { cell: cell_beside(air, out), out, age: host_age(placement.species) * rng.range(0.55, 1.0), seed: h });
        }
    }
    None
}

/// How likely a tree is to carry lianas: the big rainforest trees, a few on the others.
fn liana_chance(species: Species) -> f32 {
    match species {
        Species::Kapok => 0.9,
        Species::Jungle => 0.55,
        Species::ElderOak | Species::Baobab => 0.15,
        _ => 0.0,
    }
}

/// The underside of whatever is overhead at `(x, z)`, going up from `y`: the last air
/// voxel below it, if there is open air at `y` and something before `top`.
pub fn underside(world: &impl Voxels, mats: &Materials, x: i32, y: i32, z: i32, top: i32) -> Option<IVec3> {
    let open = |v: Voxel| v == voxel::AIR || mats.get(v).kind == Kind::Air;
    if !open(world.voxel(IVec3::new(x, y, z))) {
        return None;
    }
    (y..top).find(|&k| !open(world.voxel(IVec3::new(x, k + 1, z)))).map(|k| IVec3::new(x, k, z))
}

/// Roots of lianas hanging from under a tree's crown, a few per tree.
pub fn canopy_roots(placement: &Placement, lib: &Library, world: &impl Voxels, mats: &Materials, seed: u32, chance_scale: f32) -> Vec<Root> {
    let mut roots = Vec::new();
    let chance = liana_chance(placement.species) * chance_scale;
    if chance <= 0.0 {
        return roots;
    }
    let pos = placement.pos;
    let h = hash3(seed ^ 0x11a7, pos.x, pos.y, pos.z);
    let mut rng = Rng::new(h);
    let reach = lib.get(placement.species, placement.variant).reach.clamp(3, REACH - 2);
    let top = placement.top(lib);
    let tries = if placement.species == Species::Kapok { 7u32 } else { 4 };
    for k in 0..tries {
        if rng.unit() >= chance * 0.6 {
            continue;
        }
        let (dir, _) = rng.heading();
        let dist = rng.range(2.0, reach as f32);
        let (x, z) = (pos.x + (dir.x * dist).round() as i32, pos.z + (dir.z * dist).round() as i32);
        let Some(air) = underside(world, mats, x, pos.y + 3, z, top) else { continue };
        // Only where there is a real drop beneath.
        if air.y - pos.y < 6 {
            continue;
        }
        let cell = air * CELL + IVec3::new(CELL / 2, CELL - 1, CELL / 2);
        roots.push(Root { cell, out: IVec3::NEG_Y, age: rng.range(0.4, 1.0), seed: h ^ k.wrapping_mul(0x9e37_79b9) });
    }
    roots
}

/// How likely cliff rims in a biome are to have lianas hanging over them.
fn rim_chance(biome: Biome) -> f32 {
    match biome {
        Biome::Rainforest => 0.7,
        Biome::OldGrowth | Biome::Deciduous => 0.12,
        _ => 0.0,
    }
}

/// Where a liana hangs over a drop beside the column at `(x, z)` with ground at `top`:
/// the air voxel over the edge, and the way out from the cliff face, if the ground
/// falls away by at least 6 voxels on some side. `top_at` gives other columns' tops.
pub fn rim_root(world: &impl Voxels, mats: &Materials, x: i32, z: i32, top: i32, top_at: impl Fn(i32, i32) -> i32) -> Option<(IVec3, IVec3)> {
    [IVec3::X, IVec3::NEG_X, IVec3::Z, IVec3::NEG_Z].into_iter().find_map(|d| {
        let air = IVec3::new(x + d.x, top, z + d.z);
        (top_at(air.x, air.z) <= top - 6 && !solid(world.voxel(air), mats)).then_some((air, d))
    })
}

/// The cell of air voxel `air` right against the surface it grows out from, at the foot
/// of the voxel and in the middle along the surface.
fn cell_beside(air: IVec3, out: IVec3) -> IVec3 {
    let along = |d: i32| match d {
        1 => 0,
        -1 => CELL - 1,
        _ => CELL / 2,
    };
    air * CELL + IVec3::new(along(out.x), 0, along(out.z))
}

/// Roots along the foot of a built thing: walks from sampled voxels of `stamp` down to
/// the ground on an open side.
pub fn stamp_roots(voxels: &[(IVec3, Voxel)], world: &impl Voxels, mats: &Materials, seed: u32) -> Vec<Root> {
    let mut roots = Vec::new();
    if voxels.len() < 40 {
        return roots;
    }
    let attempts = (voxels.len() / 300).clamp(1, 10);
    for k in 0..attempts {
        let h = hash3(seed ^ 0x57a3, voxels[0].0.x, voxels[0].0.z, k as i32);
        let mut rng = Rng::new(h);
        let (p, v) = voxels[(rng.next_u32() as usize) % voxels.len()];
        if v == voxel::AIR || !climbable(v) {
            continue;
        }
        let side = [IVec3::X, IVec3::NEG_X, IVec3::Z, IVec3::NEG_Z][(rng.next_u32() % 4) as usize];
        let mut air = p + side;
        if solid(world.voxel(air), mats) {
            continue;
        }
        // Down the open side of the wall to the ground.
        let mut depth = 0;
        while depth < 48 && !solid(world.voxel(air - IVec3::Y), mats) {
            air -= IVec3::Y;
            depth += 1;
        }
        if depth >= 48 || !solid(world.voxel(air - side), mats) {
            continue;
        }
        roots.push(Root { cell: cell_beside(air, side), out: side, age: rng.range(0.3, 1.0), seed: h });
    }
    roots
}

/// Scores a place for climbers: broad leaves in damp, sheltered places. Cold, high and
/// very dry country gets nothing.
pub fn place_chance(col: &Column) -> f32 {
    if col.temperature < 0.18 {
        return 0.0;
    }
    match col.biome {
        Biome::Rainforest => 1.0,
        Biome::OldGrowth => 0.9,
        Biome::Deciduous => 0.7,
        Biome::Woodland => 0.5,
        Biome::Taiga => 0.2,
        Biome::Heath | Biome::Grassland => 0.12,
        Biome::Savanna => 0.05,
        _ => 0.0,
    }
}

/// Leaf density for a plant's age: the plant carries more of its full complement of
/// leaves as it matures, then keeps them.
fn leaves_at(age: f32) -> f32 {
    (0.5 + age).clamp(0.0, 1.0)
}

/// Grows a plant and rasterises it whole: stems first so they win their cells, then
/// leaves, then flowers. The leaf pass thins until the plant fits its box budget.
pub fn plant(params: &Climber, root: &Root, world: &impl Voxels, mats: &Materials) -> Raster {
    let skeleton = grow_stems(params, root, world, mats);
    let mut stems = Raster::new();
    raster_stems(params, root, &skeleton, world, mats, &mut stems);
    let base_boxes = box_count(&stems);
    let mut leaves = leaves_at(root.age);
    // Thin the leaves, and only the leaves, until the plant fits.
    for _ in 0..6 {
        let mut full = Raster::new();
        full.absorb_clone(&stems);
        raster_leaves(params, root, &skeleton, world, mats, &mut full, leaves);
        raster_fruit(params, root, &skeleton, world, mats, &mut full);
        if base_boxes >= params.box_budget {
            break;
        }
        if box_count(&full) <= params.box_budget {
            return full;
        }
        leaves *= 0.7;
    }
    stems
}

fn box_count(raster: &Raster) -> usize {
    let mut copy = Raster::new();
    copy.absorb_clone(raster);
    let mut boxes = Vec::new();
    copy.finish(&mut boxes, |_| (false, false));
    boxes.len()
}

/// Most boxes of climbers in one chunk column: at 5 quads each, about 240 KB of vertices.
pub const COLUMN_BOXES: usize = 1500;

/// Finds plant roots near a chunk column.
pub struct RootFinder<'a> {
    pub world: &'a Surroundings<'a>,
    pub mats: &'a Materials,
    pub seed: u32,
    /// The horizontal area roots must lie in to be able to reach this column.
    pub x0: i32,
    pub z0: i32,
    pub x1: i32,
    pub z1: i32,
}

impl RootFinder<'_> {
    fn within(&self, x: i32, z: i32) -> bool {
        x >= self.x0 && x < self.x1 && z >= self.z0 && z < self.z1
    }

    fn chance(&self, x: i32, z: i32) -> f32 {
        place_chance(&self.world.column(x, z))
    }

    /// Roots at the foot of trees, boulders and logs near here.
    pub fn trees(&self, out: &mut Vec<(Root, Climber)>) {
        for p in self.world.placements {
            if !self.within(p.pos.x, p.pos.z) {
                continue;
            }
            let chance = self.chance(p.pos.x, p.pos.z);
            if chance <= 0.0 {
                continue;
            }
            let col = self.world.column(p.pos.x, p.pos.z);
            if let Some(root) = tree_root(p, self.world, self.mats, self.seed, chance) {
                let low = matches!(p.species, Species::FallenLog | Species::Boulder | Species::MossyBoulder | Species::Stump);
                let wet = matches!(col.biome, Biome::Rainforest | Biome::OldGrowth | Biome::Deciduous);
                if low && wet {
                    out.push((root, TRAILING_CREEPER));
                } else if let Some(params) = suited(col.biome, col.temperature > 0.6) {
                    out.push((root, params));
                }
            }
            if col.biome == Biome::Rainforest {
                for root in canopy_roots(p, self.world.lib, self.world, self.mats, self.seed, chance) {
                    let (x, z) = (root.cell.x.div_euclid(CELL), root.cell.z.div_euclid(CELL));
                    if self.within(x, z) {
                        out.push((root, LIANA));
                    }
                }
            }
        }
    }

    /// Roots along the foot of built things, given each stamp's voxels and the offset
    /// from its coordinates to the world's.
    pub fn stamps<'b>(&self, stamps: impl Iterator<Item = (&'b [(IVec3, Voxel)], IVec3)>, out: &mut Vec<(Root, Climber)>) {
        for (voxels, offset) in stamps {
            let world: Vec<(IVec3, Voxel)> = voxels.iter().map(|&(p, v)| (p + offset, v)).collect();
            for root in stamp_roots(&world, self.world, self.mats, self.seed) {
                let (x, z) = (root.cell.x.div_euclid(CELL), root.cell.z.div_euclid(CELL));
                if !self.within(x, z) || unit(root.seed) >= self.chance(x, z) {
                    continue;
                }
                let col = self.world.column(x, z);
                // Flowers around where people are.
                let flowers = unit(root.seed.rotate_left(9)) < 0.3;
                if let Some(params) = suited(col.biome, col.temperature > 0.5) {
                    out.push((root, if flowers { FLOWERING_VINE } else { params }));
                }
            }
        }
    }

    /// Lianas hanging over the rims of cliffs and steep banks in wet country.
    pub fn rims(&self, out: &mut Vec<(Root, Climber)>) {
        let cell = 8;
        for gz in self.z0.div_euclid(cell)..=self.z1.div_euclid(cell) {
            for gx in self.x0.div_euclid(cell)..=self.x1.div_euclid(cell) {
                let (x, z) = (
                    gx * cell + (hashed_chance(self.seed, gx, gz, 0x3a1) * cell as f32) as i32,
                    gz * cell + (hashed_chance(self.seed, gx, gz, 0x3a2) * cell as f32) as i32,
                );
                if !self.within(x, z) {
                    continue;
                }
                let col = self.world.column(x, z);
                let chance = rim_chance(col.biome) * place_chance(&col);
                if chance <= 0.0 || col.cover != Cover::Land || col.water_surface().is_some() {
                    continue;
                }
                let h = hash3(self.seed ^ 0x51e, x, col.top(), z);
                if unit(h) >= chance {
                    continue;
                }
                let Some((air, d)) = rim_root(self.world, self.mats, x, z, col.top(), |x, z| self.world.column(x, z).top()) else { continue };
                if !self.within(air.x, air.z) {
                    continue;
                }
                let mut cell = cell_beside(air, d);
                cell.y = air.y * CELL + CELL - 1;
                out.push((Root { cell, out: d, age: unit(h.rotate_left(13)) * 0.6 + 0.4, seed: h }, LIANA));
            }
        }
    }

    /// Roots at the foot of steep ground, and a few creeping across the woodland floor.
    pub fn ground(&self, out: &mut Vec<(Root, Climber)>) {
        let cell = 16;
        for gz in self.z0.div_euclid(cell)..=self.z1.div_euclid(cell) {
            for gx in self.x0.div_euclid(cell)..=self.x1.div_euclid(cell) {
                let (x, z) = (
                    gx * cell + (hashed_chance(self.seed, gx, gz, 0x71) * cell as f32) as i32,
                    gz * cell + (hashed_chance(self.seed, gx, gz, 0x72) * cell as f32) as i32,
                );
                if !self.within(x, z) {
                    continue;
                }
                let col = self.world.column(x, z);
                let chance = place_chance(&col);
                if chance <= 0.0 || col.cover != Cover::Land || col.water_surface().is_some() {
                    continue;
                }
                let air = IVec3::new(x, col.top() + 1, z);
                let Some(params) = suited(col.biome, col.temperature > 0.6) else { continue };
                let h = hash3(self.seed ^ 0xc11f, x, air.y, z);
                // A wall of earth or rock beside this column?
                let wall = [IVec3::X, IVec3::NEG_X, IVec3::Z, IVec3::NEG_Z].into_iter().find(|&d| {
                    let high = self.world.column(x + d.x, z + d.z).top();
                    high >= air.y + 4 && solid(self.world.voxel(air + d), self.mats)
                });
                if let Some(d) = wall {
                    if unit(h) < 0.5 * chance {
                        out.push((Root { cell: cell_beside(air, -d), out: -d, age: unit(h.rotate_left(7)) * 0.6 + 0.2, seed: h }, params));
                    }
                    continue;
                }
                // Otherwise a low carpet, in the shade of woods only.
                if unit(h) < 0.1 * chance && column_shade(self.world, air, self.mats) {
                    let root = Root { cell: cell_beside(air, IVec3::Y), out: IVec3::Y, age: unit(h.rotate_left(11)) * 0.3 + 0.1, seed: h };
                    out.push((root, params));
                }
            }
        }
    }
}

/// Whether leaves shade a column, for woodland-floor plants.
fn column_shade(world: &impl Voxels, air: IVec3, mats: &Materials) -> bool {
    (2..28).any(|k| mats.get(world.voxel(air + IVec3::Y * k)).kind == Kind::Foliage)
}

/// Whole plants grown recently, by root cell. A plant spans several chunk columns and
/// each needs it whole to clip from, so it is grown once and shared. A plant depends only
/// on its root and the world around it, so whether it comes from here or is grown again
/// makes no difference to the result.
#[derive(Default)]
pub struct PlantCache {
    plants: Mutex<HashMap<IVec3, Arc<Raster>>>,
}

impl PlantCache {
    /// Most plants kept: a few chunk columns' worth around where the world is loading.
    const SIZE: usize = 768;

    /// Forgets every plant.
    pub fn clear(&self) {
        self.plants.lock().unwrap().clear();
    }

    fn get_or_grow(&self, root: &Root, grow: impl FnOnce() -> Raster) -> Arc<Raster> {
        if let Some(r) = self.plants.lock().unwrap().get(&root.cell) {
            return r.clone();
        }
        // Grown outside the lock; two threads may grow the same plant, to the same result.
        let raster = Arc::new(grow());
        let mut plants = self.plants.lock().unwrap();
        if plants.len() >= Self::SIZE {
            plants.clear();
        }
        plants.insert(root.cell, raster.clone());
        raster
    }
}

/// Grows all the climbers overlapping one chunk column: each whole plant grown from its
/// root and clipped to this column, so every column it touches gets the matching part.
/// `inspect` sees each whole plant before it is clipped.
#[allow(clippy::too_many_arguments)]
pub fn grow_column(
    mut roots: Vec<(Root, Climber)>,
    column: (i32, i32),
    voxels: &ColumnVoxels,
    world: &Surroundings,
    mats: &Materials,
    seed: u32,
    cache: &PlantCache,
    out: &mut Vec<voxel::mesh::PlantBox>,
    inspect: &mut dyn FnMut(&Root, &Raster),
) {
    roots.sort_unstable_by_key(|(r, _)| (r.cell.y, r.cell.x, r.cell.z));
    roots.dedup_by_key(|(r, _)| r.cell);
    let mut combined = Raster::new();
    for (root, params) in &roots {
        let whole = cache.get_or_grow(root, || plant(params, root, world, mats));
        inspect(root, &whole);
        let mut raster = whole.column_copy(column.0, column.1);
        raster.retain_voxels(|at| voxels.has_height(at.y));
        combined.absorb(raster);
    }
    let start = out.len();
    combined.finish(out, sway);
    if out.len() - start > COLUMN_BOXES {
        let mut mine: Vec<_> = out.drain(start..).collect();
        mine.sort_unstable_by_key(|b| hash3(seed ^ 0x1eaf, b.lo[0], b.lo[1], b.lo[2]));
        mine.truncate(COLUMN_BOXES);
        out.append(&mut mine);
    }
}

/// Sway flags for finished boxes: lianas swing, everything else barely moves.
pub fn sway(voxel: Voxel) -> (bool, bool) {
    if voxel == mat::LIANA_STEM { (true, false) } else { (false, false) }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use voxel::mesh::PlantBox;

    use super::*;
    use crate::{features::attach::tests_support::Blocks, materials::build};

    /// A straight wall 2 voxels thick along Z at x = 0..=1, on flat ground.
    fn wall(height: i32) -> (Blocks, Materials) {
        let mats = build();
        let mut map = HashMap::new();
        for x in 0..2 {
            for z in -20..20 {
                for y in 0..height {
                    map.insert(IVec3::new(x, y, z), mat::STONE);
                }
            }
        }
        (Blocks { map, floor: 0, floor_voxel: mat::STONE }, mats)
    }

    fn root_on_wall() -> Root {
        // On the -X face of the wall, one voxel above the ground, near the foot.
        Root { cell: IVec3::new(-1, 0, 0) * CELLS + IVec3::new(CELLS - 1, 0, 2), out: IVec3::NEG_X, age: 0.7, seed: 8123 }
    }

    #[test]
    fn stems_start_at_the_root_and_climb() {
        let (world, mats) = wall(40);
        let skel = grow_stems(&IVY, &root_on_wall(), &world, &mats);
        assert!(skel.nodes.len() > 40, "only {} nodes", skel.nodes.len());
        assert_eq!(skel.nodes[0].cell, root_on_wall().cell);
        let tallest = skel.nodes.iter().map(|n| n.cell.y).max().unwrap();
        assert!(tallest > root_on_wall().cell.y + 20 * CELLS, "only reached {tallest}");
    }

    #[test]
    fn stems_stay_in_air_and_never_float_beyond_the_gap() {
        let (world, mats) = wall(16);
        let skel = grow_stems(&IVY, &root_on_wall(), &world, &mats);
        let mut sense = Sensing { world: &world, mats: &mats, cache: HashMap::new() };
        // Cells off a surface in a row along each stem, back to its last hold.
        let mut runs = vec![0u32; skel.nodes.len()];
        for (k, n) in skel.nodes.iter().enumerate() {
            assert!(sense.air(n.cell), "{:?} is inside rock", n.cell);
            if sense.surface_out(n.cell) == IVec3::ZERO {
                runs[k] = if k == 0 { 1 } else { runs[n.parent as usize] + 1 };
                assert!(runs[k] <= AIR_GAP, "stem floated {} cells at {:?}", runs[k], n.cell);
            }
        }
    }

    #[test]
    fn a_stem_goes_over_the_top_of_a_wall() {
        let (world, mats) = wall(12);
        let skel = grow_stems(&IVY, &root_on_wall(), &world, &mats);
        let top = 12 * CELLS;
        let over = skel.nodes.iter().any(|n| n.cell.y >= top && n.cell.x >= 0);
        assert!(over, "no stem crossed the wall top; highest {:?}", skel.nodes.iter().map(|n| n.cell.y).max());
    }

    #[test]
    fn stems_fork_and_fan_out() {
        let (world, mats) = wall(60);
        let skel = grow_stems(&IVY, &root_on_wall(), &world, &mats);
        let generations = skel.nodes.iter().map(|n| n.generation).max().unwrap();
        assert!(generations >= 2, "only {generations} generations of forking");
        let (lo, hi) = (
            skel.nodes.iter().map(|n| n.cell.z).min().unwrap(),
            skel.nodes.iter().map(|n| n.cell.z).max().unwrap(),
        );
        assert!(hi - lo >= 8 * CELLS, "only spread {} cells across the wall", hi - lo);
    }

    #[test]
    fn an_old_plant_has_a_thick_base_and_thin_ends() {
        let (world, mats) = wall(70);
        let root = Root { age: 1.0, ..root_on_wall() };
        let skel = grow_stems(&IVY, &root, &world, &mats);
        let base = skel.nodes[0].load;
        assert!(thickness(base, base, root.age) >= 3, "base carries only {base}");
        let tip = skel.nodes.iter().map(|n| thickness(n.load, base, root.age)).min().unwrap();
        assert_eq!(tip, 1, "the ends should be single cells");
    }

    #[test]
    fn growing_is_deterministic() {
        let (world, mats) = wall(30);
        let root = root_on_wall();
        let (a, b) = (grow_stems(&IVY, &root, &world, &mats), grow_stems(&IVY, &root, &world, &mats));
        assert_eq!(a.nodes.len(), b.nodes.len());
        assert!(a.nodes.iter().zip(&b.nodes).all(|(p, q)| p.cell == q.cell));
    }

    fn rastered(params: &Climber, root: &Root, world: &Blocks, mats: &Materials) -> Vec<PlantBox> {
        let raster = plant(params, root, world, mats);
        let mut out = Vec::new();
        raster.finish(&mut out, sway);
        out
    }

    #[test]
    fn every_cell_is_air_and_boxes_never_overlap() {
        let (world, mats) = wall(24);
        let boxes = rastered(&IVY, &root_on_wall(), &world, &mats);
        assert!(!boxes.is_empty());
        let check = |c: IVec3| {
            let at = c.div_euclid(IVec3::splat(CELLS));
            assert!(!solid(world.voxel(at), &mats), "{c} is inside {at}");
        };
        for b in &boxes {
            check(IVec3::from_array(b.lo));
            check(IVec3::new(b.hi[0] - 1, b.hi[1] - 1, b.hi[2] - 1));
        }
        for (i, a) in boxes.iter().enumerate() {
            for b in &boxes[i + 1..] {
                let overlap = (0..3).all(|axis| a.lo[axis] < b.hi[axis] && b.lo[axis] < a.hi[axis]);
                assert!(!overlap, "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn the_piece_budget_holds() {
        let (world, mats) = wall(90);
        for params in [IVY, VIRGINIA_CREEPER, FLOWERING_VINE, LIANA, TRAILING_CREEPER] {
            let root = Root { age: 1.0, ..root_on_wall() };
            let boxes = rastered(&params, &root, &world, &mats);
            assert!(boxes.len() <= params.box_budget, "{}: {} boxes", params.name, boxes.len());
        }
    }

    #[test]
    fn two_species_differ_through_parameters_alone() {
        let (world, mats) = wall(60);
        let root = Root { age: 0.9, ..root_on_wall() };
        let ivy = rastered(&IVY, &root, &world, &mats);
        let creeper = rastered(&VIRGINIA_CREEPER, &root, &world, &mats);
        let colours = |boxes: &[PlantBox]| boxes.iter().map(|b| b.voxel).collect::<HashSet<_>>();
        assert!(!ivy.is_empty() && !creeper.is_empty());
        assert_ne!(colours(&ivy), colours(&creeper));
        assert!(colours(&ivy).contains(&mat::IVY_GLOSS) || colours(&ivy).contains(&mat::IVY_GLOSS_LIT));
        assert!(colours(&creeper).contains(&mat::CREEPER_GREEN) || colours(&creeper).contains(&mat::CREEPER_GREEN_DARK));
    }

    /// A wall with an eave: a slab at height `height` jutting `jut` voxels out over the -X
    /// face, like a tower roof or a signboard over its post.
    fn eaved_wall(height: i32, jut: i32) -> (Blocks, Materials) {
        let (mut world, mats) = wall(height);
        for x in -jut..2 {
            for z in -20..20 {
                world.map.insert(IVec3::new(x, height, z), mat::PLANK);
            }
        }
        (world, mats)
    }

    /// Plant cells whose leaf colour is one of the climber's.
    fn leaf_cells(params: &Climber, raster: &Raster) -> Vec<IVec3> {
        let autumn = params.autumn.map(|a| a.0);
        raster.cells().into_iter().filter(|(_, v)| params.leaves.contains(v) || Some(*v) == autumn).map(|(c, _)| c).collect()
    }

    #[test]
    fn nothing_dangles_under_an_eave() {
        let (world, mats) = eaved_wall(14, 4);
        let mut sense = Sensing { world: &world, mats: &mats, cache: HashMap::new() };
        for seed in 0..12 {
            for params in [IVY, VIRGINIA_CREEPER, FLOWERING_VINE] {
                let root = Root { age: 0.4 + seed as f32 * 0.05, seed: 9000 + seed, ..root_on_wall() };
                let skel = grow_stems(&params, &root, &world, &mats);
                let mut children = vec![0; skel.nodes.len()];
                for n in &skel.nodes[1..] {
                    children[n.parent as usize] += 1;
                }
                for (k, n) in skel.nodes.iter().enumerate() {
                    if children[k] > 0 {
                        continue;
                    }
                    // Every stem ends on a wall or the floor, never under the eave or in the air,
                    assert!(sense.braced(n.cell), "{}: a stem ends hanging at {:?}", params.name, n.cell);
                    // and never by running down off a bottom edge.
                    let below = n.cell - IVec3::Y;
                    let off_edge = sense.air(below) && !sense.braced(below);
                    assert!(!(n.dir == IVec3::NEG_Y && off_edge), "{}: a strand hangs down to {:?}", params.name, n.cell);
                }
                // Every leaf lies a cell off a wall or the ground, none off a ceiling.
                let raster = plant(&params, &root, &world, &mats);
                for c in leaf_cells(&params, &raster) {
                    let held = BRACES.iter().any(|&d| sense.air(c + d) && !sense.air(c + d * 2));
                    assert!(held, "{}: a leaf hangs at {c:?}", params.name);
                }
            }
        }
    }

    #[test]
    fn leaves_are_whole_and_kept_apart() {
        let (world, mats) = wall(60);
        for params in [IVY, VIRGINIA_CREEPER] {
            let root = Root { age: 0.8, ..root_on_wall() };
            let raster = plant(&params, &root, &world, &mats);
            let cells: HashSet<IVec3> = leaf_cells(&params, &raster).into_iter().collect();
            // Group touching leaf cells, corners included: each group is one whole leaf.
            let mut seen = HashSet::new();
            let mut groups = 0;
            for &start in &cells {
                if !seen.insert(start) {
                    continue;
                }
                let (mut stack, mut size) = (vec![start], 0);
                while let Some(c) = stack.pop() {
                    size += 1;
                    for x in -1..=1 {
                        for y in -1..=1 {
                            for z in -1..=1 {
                                let n = c + IVec3::new(x, y, z);
                                if cells.contains(&n) && seen.insert(n) {
                                    stack.push(n);
                                }
                            }
                        }
                    }
                }
                assert_eq!(size, params.leaf.len(), "{}: leaves touch or one is broken, near {start:?}", params.name);
                groups += 1;
            }
            assert!(groups >= 12, "{}: only {groups} leaves", params.name);
        }
    }

    #[test]
    fn young_plants_stay_small_and_old_ones_big() {
        let (world, mats) = wall(70);
        let young = grow_stems(&IVY, &Root { age: 0.05, ..root_on_wall() }, &world, &mats);
        let old = grow_stems(&IVY, &Root { age: 1.0, ..root_on_wall() }, &world, &mats);
        assert!(old.nodes.len() > 4 * young.nodes.len(), "old {} young {}", old.nodes.len(), young.nodes.len());
    }

    #[test]
    fn leaves_are_a_separate_pass_that_can_be_dropped() {
        let (world, mats) = wall(40);
        let root = root_on_wall();
        let skel = grow_stems(&IVY, &root, &world, &mats);
        let mut with = Raster::new();
        raster_stems(&IVY, &root, &skel, &world, &mats, &mut with);
        raster_leaves(&IVY, &root, &skel, &world, &mats, &mut with, 1.0);
        let mut without = Raster::new();
        raster_stems(&IVY, &root, &skel, &world, &mats, &mut without);
        raster_leaves(&IVY, &root, &skel, &world, &mats, &mut without, 0.0);
        let (mut a, mut b) = (Vec::new(), Vec::new());
        with.finish(&mut a, |_| (false, false));
        without.finish(&mut b, |_| (false, false));
        let leaves = |v: &[PlantBox]| v.iter().filter(|p| IVY.leaves.contains(&p.voxel)).count();
        assert!(leaves(&a) > 0, "no leaves placed");
        assert_eq!(leaves(&b), 0, "leaves weren't dropped");
    }

    #[test]
    fn a_liana_hangs_down_from_where_it_starts() {
        let (world, mats) = wall(90);
        let root = Root { cell: IVec3::new(-3, 45, 0) * CELLS, out: IVec3::ZERO, age: 0.8, seed: 44 };
        let skel = grow_stems(&LIANA, &root, &world, &mats);
        let lowest = skel.nodes.iter().map(|n| n.cell.y).min().unwrap();
        assert!(lowest < root.cell.y - 40, "only fell to {lowest} from {}", root.cell.y);
        assert!(skel.nodes.iter().all(|n| n.cell.y <= root.cell.y + 1), "lianas should not climb");
    }

    /// Finds climbers in a real world and prints where to point a camera at them:
    /// `CLIMBER_AT=seed,x,z cargo test -p worldgen --release find_climbers -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn find_climbers() {
        use crate::world::GameWorld;
        let spec = std::env::var("CLIMBER_AT").unwrap_or("1,38944,13600".into());
        let v: Vec<i32> = spec.split(',').map(|s| s.parse().unwrap()).collect();
        let w = GameWorld::new(v[0] as u32);
        let (cx0, cz0) = (v[1].div_euclid(voxel::CHUNK), v[2].div_euclid(voxel::CHUNK));
        let mut plants: HashMap<IVec3, (IVec3, usize, i32)> = HashMap::new();
        let mut slowest = 0.0f32;
        for cz in cz0 - 3..=cz0 + 3 {
            for cx in cx0 - 3..=cx0 + 3 {
                let t = std::time::Instant::now();
                w.generate_column_with(cx, cz, &mut |root, raster| {
                    let cells = raster.cells();
                    let rise = cells.iter().map(|c| c.0.y).max().unwrap_or(0) - root.cell.y;
                    plants.insert(root.cell, (root.out, cells.len(), rise));
                });
                slowest = slowest.max(t.elapsed().as_secs_f32() * 1000.0);
            }
        }
        let mut list: Vec<_> = plants.into_iter().collect();
        list.sort_by_key(|(_, (_, n, _))| std::cmp::Reverse(*n));
        println!("{} plants, slowest column {slowest:.1} ms", list.len());
        for (cell, (out, n, rise)) in list.iter().take(12) {
            let at = *cell / CELLS;
            // Stand back along the way out from the surface and look at it, up a little.
            let back = if *out == IVec3::Y || *out == IVec3::ZERO { IVec3::new(-1, 0, -1) } else { *out };
            let eye = at + back * 7;
            let yaw = ((-back.z) as f32).atan2((-back.x) as f32).to_degrees();
            println!("{n:5} cells, rises {:3} voxels, root {at} out {out}: --over {},{},4 --look {yaw:.0},20", rise / CELLS, eye.x, eye.z);
        }
    }

    /// Counts climber pieces around a few places in a real world, for comparing changes:
    /// `CLIMBER_STATS=seed,x,z;x,z cargo test -p worldgen --release climber_stats -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn climber_stats() {
        use crate::world::GameWorld;
        let spec = std::env::var("CLIMBER_STATS").unwrap_or("1,38997,13564;26456,28228;40115,22603".into());
        let mut parts = spec.split(';');
        let first: Vec<i32> = parts.next().unwrap().split(',').map(|s| s.parse().unwrap()).collect();
        let w = GameWorld::new(first[0] as u32);
        let mut spots = vec![(first[1], first[2])];
        spots.extend(parts.map(|p| {
            let v: Vec<i32> = p.split(',').map(|s| s.parse().unwrap()).collect();
            (v[0], v[1])
        }));
        let mats = build();
        let (mut plants, mut boxes, mut cells, mut leaf_cells, mut lone) = (0, 0, 0, 0, 0);
        let mut faces = 0usize;
        for (x, z) in spots {
            let (cx0, cz0) = (x.div_euclid(voxel::CHUNK), z.div_euclid(voxel::CHUNK));
            let mut seen: HashMap<IVec3, ()> = HashMap::new();
            for cz in cz0 - 2..=cz0 + 2 {
                for cx in cx0 - 2..=cx0 + 2 {
                    w.generate_column_with(cx, cz, &mut |root, raster| {
                        if seen.insert(root.cell, ()).is_some() {
                            return;
                        }
                        plants += 1;
                        let all = raster.cells();
                        let set: HashSet<IVec3> = all.iter().map(|c| c.0).collect();
                        cells += all.len();
                        for (c, v) in &all {
                            if mats.get(*v).kind == Kind::Foliage && !matches!(*v, mat::IVY_STEM | mat::IVY_WOOD | mat::IVY_WOOD_DARK | mat::IVY_TWIG | mat::IVY_ROPE | mat::IVY_ROPE_DARK | mat::LIANA_STEM | mat::LIANA_WOOD) {
                                leaf_cells += 1;
                            }
                            // A cell with no plant cell beside it: speckle.
                            if STEPS.iter().all(|s| !set.contains(&(*c + *s))) {
                                lone += 1;
                            }
                        }
                        let mut out = Vec::new();
                        let mut copy = Raster::new();
                        copy.absorb_clone(raster);
                        copy.finish(&mut out, |_| (false, false));
                        boxes += out.len();
                        faces += out.len() * 6;
                    });
                }
            }
        }
        println!("plants {plants} cells {cells} leaf cells {leaf_cells} lone cells {lone} boxes {boxes} quads<= {faces}");
    }

    /// Prints rainforest spots near the spawn of a seed, for finding lianas:
    /// `BIOME_SEED=1 cargo test -p worldgen --release find_rainforest -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn find_rainforest() {
        use crate::world::GameWorld;
        let seed = std::env::var("BIOME_SEED").map_or(1, |s| s.parse().unwrap());
        let w = GameWorld::new(seed);
        let spawn = w.spawn_point();
        let (sx, sz) = (spawn.x, spawn.z);
        let mut found = 0;
        for r in (0..6000).step_by(96) {
            for k in 0..16 {
                let a = k as f32 / 16.0 * std::f32::consts::TAU;
                let (x, z) = (sx + (a.cos() * r as f32) as i32, sz + (a.sin() * r as f32) as i32);
                let col = w.column(x, z);
                if col.biome == Biome::Rainforest && col.cover == Cover::Land {
                    println!("rainforest at {x},{z} ground {} temperature {:.2}", col.top(), col.temperature);
                    found += 1;
                    if found >= 12 {
                        return;
                    }
                }
            }
        }
    }

    #[test]
    fn ivy_flowers_in_autumn_and_fruits_over_winter() {
        let at = |season| cluster_at(&IVY, season, 0.0);
        assert_eq!(at(0.8), Some(mat::IVY_FLOWER));
        assert_eq!(at(0.97), Some(mat::IVY_BERRY_GREEN));
        assert_eq!(at(0.05), Some(mat::IVY_BERRY_GREEN), "the berry span runs over the new year");
        assert_eq!(at(0.2), Some(mat::IVY_BERRY));
        assert_eq!(at(0.5), None, "nothing in summer");
        // A cluster's own threshold holds it back a little.
        assert_eq!(cluster_at(&IVY, 0.73, 0.0), Some(mat::IVY_FLOWER));
        assert_eq!(cluster_at(&IVY, 0.73, 1.0), None);
        assert_eq!(cluster_at(&LIANA, 0.8, 0.0), None);
    }

    #[test]
    fn a_liana_starts_under_a_crown_and_falls_clear() {
        // A trunk with a slab of leaves over it at height 20.
        let mats = build();
        let mut map = HashMap::new();
        for y in 0..24 {
            map.insert(IVec3::new(0, y, 0), mat::BARK_JUNGLE);
        }
        for x in -6..=6 {
            for z in -6..=6 {
                map.insert(IVec3::new(x, 20, z), mat::LEAVES_JUNGLE);
            }
        }
        let world = Blocks { map, floor: 0, floor_voxel: mat::STONE };
        assert_eq!(underside(&world, &mats, 4, 3, 2, 30), Some(IVec3::new(4, 19, 2)));
        assert_eq!(underside(&world, &mats, 0, 3, 0, 30), None, "inside the trunk");
        assert_eq!(underside(&world, &mats, 12, 3, 0, 30), None, "nothing overhead");
        let air = IVec3::new(4, 19, 2);
        let root = Root { cell: air * CELLS + IVec3::new(5, 9, 5), out: IVec3::NEG_Y, age: 0.9, seed: 3 };
        let skel = grow_stems(&LIANA, &root, &world, &mats);
        let lowest = skel.nodes.iter().map(|n| n.cell.y).min().unwrap();
        assert!(lowest < 8 * CELLS, "hung only to {lowest}");
        let boxes = rastered(&LIANA, &root, &world, &mats);
        assert!(boxes.iter().any(|b| b.voxel == mat::LIANA_STEM));
        assert!(boxes.len() <= LIANA.box_budget);
    }

    #[test]
    fn a_liana_hangs_over_a_cliff_rim() {
        // Ground at 30 for x < 0, a drop to 0 beyond.
        let mats = build();
        let mut map = HashMap::new();
        for x in -6..0 {
            for z in -6..6 {
                for y in 0..=30 {
                    map.insert(IVec3::new(x, y, z), mat::STONE);
                }
            }
        }
        let world = Blocks { map, floor: 0, floor_voxel: mat::STONE };
        let top_at = |x: i32, _z: i32| if x < 0 { 30 } else { 0 };
        let (air, d) = rim_root(&world, &mats, -1, 0, 30, top_at).expect("no rim found");
        assert_eq!((air, d), (IVec3::new(0, 30, 0), IVec3::X));
        assert_eq!(rim_root(&world, &mats, -3, 0, 30, top_at), None, "away from the edge");
        let mut cell = cell_beside(air, d);
        cell.y = air.y * CELLS + CELLS - 1;
        let root = Root { cell, out: d, age: 1.0, seed: 77 };
        let skel = grow_stems(&LIANA, &root, &world, &mats);
        let lowest = skel.nodes.iter().map(|n| n.cell.y).min().unwrap();
        assert!(lowest < 20 * CELLS, "hung only to {lowest}");
        // Down the face, not out into the air.
        let far = skel.nodes.iter().map(|n| n.cell.x).max().unwrap();
        assert!(far < 4 * CELLS, "strayed out to {far}, lowest {lowest}, {} nodes", skel.nodes.len());
    }

    #[test]
    fn a_trailing_creeper_stays_low_over_a_log() {
        // A log 2 voxels high and long along X.
        let mats = build();
        let mut map = HashMap::new();
        for x in -10..10 {
            for y in 0..2 {
                for z in 0..2 {
                    map.insert(IVec3::new(x, y, z), mat::BARK_OAK);
                }
            }
        }
        let world = Blocks { map, floor: 0, floor_voxel: mat::STONE };
        let root = Root { cell: cell_beside(IVec3::new(0, 0, -1), IVec3::NEG_Z), out: IVec3::NEG_Z, age: 1.0, seed: 5 };
        let skel = grow_stems(&TRAILING_CREEPER, &root, &world, &mats);
        let highest = skel.nodes.iter().map(|n| n.cell.y).max().unwrap();
        assert!(skel.nodes.len() > 100, "only {} nodes", skel.nodes.len());
        assert!(highest <= root.cell.y + TRAILING_CREEPER.max_height);
        assert!(highest >= 2 * CELLS, "never got onto the log");
        let boxes = rastered(&TRAILING_CREEPER, &root, &world, &mats);
        assert!(boxes.len() <= TRAILING_CREEPER.box_budget);
    }

    #[test]
    fn dry_cold_places_get_no_climbers_and_damp_woods_do() {
        let col = |biome, temp: f32| Column { ground: 40.0, water: None, cover: crate::surface::Cover::Land, biome, temperature: temp, river: f32::MAX, clear: f32::MAX };
        assert_eq!(place_chance(&col(Biome::Desert, 0.8)), 0.0);
        assert_eq!(place_chance(&col(Biome::Deciduous, 0.1)), 0.0);
        assert!(place_chance(&col(Biome::Rainforest, 0.8)) > place_chance(&col(Biome::Heath, 0.5)));
    }
}
