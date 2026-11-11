//! Roots showing where the ground falls away from a tree: on a steep hillside, the lip
//! of a bank or the edge of a drop, the soil washes off the downhill side and leaves the
//! roots gripping it. On level ground they stay buried, so a tree there shows none.
//!
//! A root leaves the foot of the trunk as wood, thick and climbing a little way up the
//! trunk, runs down the slope lying on the ground and tapering, and ends in a strand of
//! 0.1 m pieces. The ground along each root is measured once, where the tree is placed,
//! so every chunk the root crosses draws it the same.

use glam::{IVec3, Vec2};
use voxel::{Voxel, mesh::{CELLS, PlantBox}};

use super::shape::Rng;
use super::trees::Species;
use super::{Library, rotate};
use crate::{materials as mat, noise::hash3};

/// Most roots one tree shows.
const MOST: usize = 4;
/// How far a root may run from the trunk, in voxels, wood and strand together.
const STEPS: usize = 14;
/// Slope (drop over run) under which the ground keeps a tree's roots buried.
const BURIED: f32 = 0.4;
/// Lowest the ground under a root may fall, below the tree's own ground: models reach
/// no deeper.
const DEEPEST: i32 = -7;

/// One root, in world voxels.
#[derive(Clone, Copy, Debug)]
struct Root {
    /// Where it leaves the trunk, across the ground.
    from: Vec2,
    dir: Vec2,
    /// Voxels it runs as wood, then all the voxels it runs.
    wood: u8,
    len: u8,
    /// Its radius where it leaves the trunk, in voxels.
    girth: f32,
    /// Height of the ground's top voxel a voxel at a time along it, from `from`, against
    /// the tree's own ground.
    ground: [i8; STEPS + 1],
}

impl Root {
    fn at(&self, t: f32) -> Vec2 {
        self.from + self.dir * t
    }

    fn ground_at(&self, t: f32) -> i32 {
        self.ground[(t.max(0.0) as usize).min(self.len as usize)] as i32
    }
}

/// The roots a placed tree shows, if any.
#[derive(Clone, Copy, Debug)]
pub struct Roots {
    list: [Option<Root>; MOST],
    bark: Voxel,
}

/// Whether a species shows roots: trees with a woody trunk. Palms hold the sand with a
/// mat of thin roots that never shows.
fn rooted(species: Species) -> bool {
    matches!(
        species,
        Species::Oak
            | Species::Birch
            | Species::Maple
            | Species::Cherry
            | Species::Spruce
            | Species::TallPine
            | Species::Redwood
            | Species::ElderOak
            | Species::Jungle
            | Species::Kapok
            | Species::Acacia
            | Species::Baobab
            | Species::Willow
            | Species::DeadSnag
    )
}

fn is_bark(v: Voxel) -> bool {
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
    )
}

/// Works out the roots of a tree standing at `pos` (its origin, on the first voxel above
/// the ground). `ground` gives the top of the ground at a world column, or None over
/// water. None where the ground is too level for any to show.
pub fn find(lib: &Library, species: Species, variant: usize, turns: u32, pos: IVec3, seed: u32, ground: impl Fn(i32, i32) -> Option<i32>) -> Option<Roots> {
    if !rooted(species) {
        return None;
    }
    // The foot of the trunk: where it stands and how thick it is.
    let t = lib.get(species, variant);
    let row = t.voxels.partition_point(|(p, _)| p.y < 0);
    let (mut sum, mut n, mut bark) = (Vec2::ZERO, 0, None);
    for &(p, v) in t.voxels[row..].iter().take_while(|(p, _)| p.y == 0) {
        if is_bark(v) {
            let q = rotate(p, turns);
            sum += Vec2::new(q.x as f32 + 0.5, q.z as f32 + 0.5);
            n += 1;
            bark = bark.or(Some(v));
        }
    }
    // Roots wear the trunk's own bark, so a birch's are pale like its trunk.
    let bark = bark?;
    let centre = Vec2::new(pos.x as f32, pos.z as f32) + sum / n as f32;
    let radius = (n as f32 / std::f32::consts::PI).sqrt();
    let base = pos.y - 1;
    let ground_at = |p: Vec2| ground(p.x.floor() as i32, p.y.floor() as i32).map(|g| g - base);

    // How steeply the ground falls away from the trunk each way round.
    const WAYS: usize = 16;
    let heading = |a: f32| Vec2::new(a.cos(), a.sin());
    let run = radius + 4.0;
    let slope = |a: f32| ground_at(centre + heading(a) * run).map_or(0.0, |g| -g as f32 / run);
    let (mut down, mut steepest) = (0.0, 0.0);
    for k in 0..WAYS {
        let a = k as f32 / WAYS as f32 * std::f32::consts::TAU;
        let s = slope(a);
        if s > steepest {
            (down, steepest) = (a, s);
        }
    }
    if steepest < BURIED {
        return None;
    }

    let mut rng = Rng::new(hash3(seed ^ 0x7007, pos.x, pos.y, pos.z));
    // Steeper ground bares more of them.
    let want = (1 + ((steepest - BURIED) / 0.15) as i32 + rng.int(0, 1)).min(MOST as i32) as usize;
    let mut list = [None; MOST];
    let mut found = 0;
    // Downhill first, then fanning out to either side.
    for spread in [0.0f32, 0.75, -0.75, 1.35, -1.35, 1.9, -1.9] {
        if found == want {
            break;
        }
        let a = down + spread + rng.range(-0.25, 0.25);
        if slope(a) < BURIED * 0.6 {
            continue;
        }
        let dir = heading(a);
        let from = centre + dir * (radius - 0.5).max(0.5);
        let mut root = Root { from, dir, wood: 0, len: 0, girth: (0.55 + radius * 0.2).min(1.5), ground: [0; STEPS + 1] };
        let mut len = 0;
        let mut last = 0;
        // Within the model's reach, or a chunk the root runs into would not know of it.
        let most = ((t.reach as f32 - radius - 2.0).max(0.0) as usize).min(STEPS);
        for k in 0..=most {
            let Some(g) = ground_at(root.at(k as f32)) else { break };
            // A root follows the ground down, but stops at a rise, a sheer drop or water.
            if k > 0 && (g > last || g < last - 2) || g < DEEPEST {
                break;
            }
            root.ground[k] = g as i8;
            last = g;
            len = k;
        }
        let reach = (3.0 + radius * 0.7) as usize + rng.int(0, 2) as usize;
        let wood = reach.min(len.saturating_sub(2));
        if wood < 2 {
            continue;
        }
        root.wood = wood as u8;
        root.len = (wood + 2 + rng.int(0, 1) as usize).min(len) as u8;
        list[found] = Some(root);
        found += 1;
    }
    (found > 0).then_some(Roots { list, bark })
}

impl Roots {
    /// The wooden voxels of the roots, in world positions.
    pub fn voxels(&self, pos: IVec3, mut set: impl FnMut(IVec3, Voxel)) {
        let base = pos.y - 1;
        for root in self.list.iter().flatten() {
            let wood = root.wood as f32;
            let mut t = 0.0;
            while t <= wood {
                let u = t / wood;
                // Thick and climbing the trunk at the foot, thinning to a voxel at the end.
                let r = root.girth + (0.5 - root.girth) * u;
                let lift = (1.0 - u).powi(2) * root.girth * 1.6;
                let at = root.at(t);
                let y = (base + root.ground_at(t) + 1) as f32 + r * 0.35 + lift;
                let c = glam::Vec3::new(at.x, y, at.y);
                let lo = (c - glam::Vec3::splat(r)).floor().as_ivec3();
                let hi = (c + glam::Vec3::splat(r)).floor().as_ivec3();
                set(c.floor().as_ivec3(), self.bark);
                for vy in lo.y..=hi.y {
                    for vz in lo.z..=hi.z {
                        for vx in lo.x..=hi.x {
                            let p = glam::Vec3::new(vx as f32 + 0.5, vy as f32 + 0.5, vz as f32 + 0.5);
                            if (p - c).length_squared() <= r * r {
                                set(IVec3::new(vx, vy, vz), self.bark);
                            }
                        }
                    }
                }
                t += 0.5;
            }
        }
    }

    /// The fine strands the roots end in, in world cells.
    pub fn strands(&self, pos: IVec3, out: &mut Vec<PlantBox>) {
        let base = pos.y - 1;
        let wood = self.bark;
        let step = 1.0 / CELLS as f32;
        for root in self.list.iter().flatten() {
            let (from, to) = (root.wood as f32 + 0.25, root.len as f32 + 0.5);
            let mut prev: Option<PlantBox> = None;
            let mut t = from;
            while t < to {
                let u = (t - from) / (to - from);
                let thick = if u < 0.45 { 2 } else { 1 };
                let at = root.at(t) * CELLS as f32;
                let (x, z) = (at.x.floor() as i32, at.y.floor() as i32);
                let y = (base + root.ground_at(t) + 1) * CELLS;
                let here = PlantBox { lo: [x, y, z], hi: [x + thick, y + thick, z + thick], voxel: wood, sway_lo: false, sway_hi: false, thin: 0 };
                match prev {
                    // Running on along x or z at the same height: one longer box.
                    Some(ref mut b) if b.lo[1] == y && b.hi[1] == y + thick && b.lo[2] == z && b.hi[2] == z + thick && (b.hi[0] == x || b.lo[0] == x + thick || (b.lo[0]..b.hi[0]).contains(&x)) => {
                        b.lo[0] = b.lo[0].min(x);
                        b.hi[0] = b.hi[0].max(x + thick);
                    }
                    Some(ref mut b) if b.lo[1] == y && b.hi[1] == y + thick && b.lo[0] == x && b.hi[0] == x + thick && (b.hi[2] == z || b.lo[2] == z + thick || (b.lo[2]..b.hi[2]).contains(&z)) => {
                        b.lo[2] = b.lo[2].min(z);
                        b.hi[2] = b.hi[2].max(z + thick);
                    }
                    _ => {
                        if let Some(b) = prev.take() {
                            // Over a step down, the strand drapes down its face.
                            if b.lo[1] > y {
                                out.push(PlantBox { lo: [x, y, z], hi: [x + 1, b.lo[1], z + 1], ..b });
                            }
                            out.push(b);
                        }
                        prev = Some(here);
                    }
                }
                t += step;
            }
            out.extend(prev);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use voxel::{AIR, CHUNK};

    use super::*;
    use crate::{
        World,
        continent::SimConfig,
        features::attach::{ColumnVoxels, Voxels},
        world::GameWorld,
    };

    #[test]
    fn roots_bare_on_slopes_only_and_agree_across_chunks() {
        let w = GameWorld::with_config(11, SimConfig { size: 256 });
        let centre = crate::macro_layout::MACRO_SIZE / CHUNK / 2;
        let mut outputs = HashMap::new();
        let (mut rooted_trees, mut trees, mut checked) = (0, 0, 0);
        let mut seen = HashSet::new();
        'search: for cx in (centre - 200..centre + 200).step_by(7) {
            for cz in (centre - 200..centre + 200).step_by(7) {
                let out = w.generate_column(cx, cz);
                for p in &out.trees {
                    if !rooted(p.species) || !seen.insert(p.pos) {
                        continue;
                    }
                    trees += 1;
                    let Some(roots) = &p.roots else { continue };
                    rooted_trees += 1;
                    // The ground really falls away from a rooted tree.
                    let g = |x: i32, z: i32| w.column(x, z).top();
                    let lowest = (-6..=6).flat_map(|dx| (-6..=6).map(move |dz| (dx, dz))).map(|(dx, dz)| g(p.pos.x + dx, p.pos.z + dz)).min().unwrap();
                    assert!(p.pos.y - 1 - lowest >= 2, "{:?} at {} shows roots on level ground", p.species, p.pos);
                    // Every root voxel is there in the column that holds it, whichever
                    // chunk column the tree stands in.
                    let mut voxels = Vec::new();
                    roots.voxels(p.pos, |q, v| voxels.push((q, v)));
                    for (q, _) in voxels.into_iter().step_by(3) {
                        let (ox, oz) = (q.x.div_euclid(CHUNK), q.z.div_euclid(CHUNK));
                        let held = outputs.entry((ox, oz)).or_insert_with(|| w.generate_column(ox, oz));
                        let got = ColumnVoxels::new(&held.chunks, ox * CHUNK - 1, oz * CHUNK - 1).voxel(q);
                        assert_ne!(got, AIR, "root voxel of {:?} at {} missing at {q}", p.species, p.pos);
                        checked += 1;
                    }
                    if rooted_trees >= 12 {
                        break 'search;
                    }
                }
            }
        }
        assert!(rooted_trees > 0 && checked > 0, "no tree showed roots");
        assert!(rooted_trees * 3 < trees, "{rooted_trees} of {trees} trees show roots");
    }
}
