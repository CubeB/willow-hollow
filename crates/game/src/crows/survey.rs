//! Making the map of [`sites`](super::sites) from the world, on a thread of its own.
//!
//! Every chunk column within reach of the player is generated once, its tops read off,
//! and then let go: the map keeps only a cell to every two by two columns, which is a few
//! kilobytes a column, where the voxels would be hundreds. The patches are kept while the
//! player is near and made again as the trees come into leaf or go bare.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        mpsc::{Receiver, Sender, channel},
    },
};

use glam::{DVec3, IVec3};
use voxel::{CHUNK, Kind, plant};
use worldgen::{
    World,
    features::trees::Species,
    macro_layout::{MACRO_SIZE, cell_of},
    scale::m,
    surface::Sampler,
    world::GameWorld,
};

use super::sites::{self, Grid, PATCH, Patch, ROOKERY_CELL, ROOKERY_SEARCH, Sites, Surface};
use crate::player::WorldVoxels;

/// How far round the player the map reaches, in metres.
pub const REACH: f32 = 100.0;
/// How far off a rookery is still looked for and gone back to, in metres.
pub const ROOKERY_SIGHT: f32 = 260.0;

/// Whether a bush of `species` bears berries that crows come down for.
fn berries(species: Species) -> bool {
    matches!(species, Species::Holly | Species::HollyBush | Species::Juniper | Species::Yew)
}

/// Whether a tree of `species` is one crows sit in the top of.
fn crown(species: Species) -> bool {
    crate::lod::stands_tall(species) && !matches!(species, Species::Palm | Species::Cactus | Species::DeadSnag)
}

/// What a patch is made from that stays the same all year, read once when its column
/// first comes into reach. A new time of year then only means deciding again which
/// broadleaf leaves are on, and never generating the column again.
struct Source {
    /// Where the look down each voxel column of the patch (x fastest) stops at any time
    /// of year: water, something solid, or evergreen leaves.
    floor: Vec<(i32, Surface)>,
    /// The broadleaf leaves above that floor in column `i`, top first:
    /// `leaves[start[i]..start[i + 1]]`, each its height and whether it is a maple's.
    start: Vec<u32>,
    leaves: Vec<(i32, bool)>,
    /// The tag of the tree whose leaf is the first broadleaf one met going down each
    /// column. The look down reads it there and keeps it for the leaves under it too.
    tags: Vec<u8>,
    /// The height of the top face of the land itself under each column.
    land: Vec<i32>,
    berries: Vec<IVec3>,
    tall: Vec<(IVec3, i32)>,
}

impl Source {
    /// Reads chunk column `col` out of `voxels`.
    fn read(world: &GameWorld, voxels: &mut WorldVoxels, col: (i32, i32)) -> Source {
        let mats = world.materials();
        let lib = world.library();
        let ceiling = voxels.ceiling(col);
        let inside = |p: IVec3| p.x.div_euclid(CHUNK) == col.0 && p.z.div_euclid(CHUNK) == col.1;
        let (mut bushes, mut tall) = (Vec::new(), Vec::new());
        for p in voxels.trees(col) {
            if !inside(p.pos) {
                continue;
            }
            if berries(p.species) {
                bushes.push(p.pos);
            } else if crown(p.species) && (p.top(lib) - p.pos.y) as f32 >= m(8.0) {
                tall.push((p.pos, p.top(lib) + 1));
            }
        }
        let n = (PATCH * PATCH) as usize;
        let (x0, z0) = (col.0 * PATCH, col.1 * PATCH);
        // The land, from one sampler for the whole patch: `world.column` makes a sampler
        // for every column it is asked about, which cost three times the generating.
        let (mx, mz) = cell_of(x0, z0);
        let continent = world.continent(mx, mz);
        let (lx0, lz0) = (x0 - mx * MACRO_SIZE, z0 - mz * MACRO_SIZE);
        let sampler = Sampler::new(&continent, world.seed(), lx0, lz0, PATCH);
        let land = (0..PATCH * PATCH).map(|i| sampler.column(lx0 + i % PATCH, lz0 + i / PATCH).top() + 1).collect();
        let mut s = Source { floor: Vec::with_capacity(n), start: Vec::with_capacity(n + 1), leaves: Vec::new(), tags: vec![0; n], land, berries: bushes, tall };
        for lz in 0..PATCH {
            for lx in 0..PATCH {
                let (x, z) = (x0 + lx, z0 + lz);
                let first = s.leaves.len();
                s.start.push(first as u32);
                let mut y = ceiling - 1;
                let mut floor = None;
                while floor.is_none() && y > ceiling - 1200 {
                    let mat = mats.get(voxels.voxel(x, y, z));
                    match mat.kind {
                        Kind::Air => {}
                        Kind::Water => floor = Some((y + 1, Surface::Water)),
                        Kind::Solid => floor = Some((y + 1, Surface::Rock)),
                        // A broadleaf crown is a seat only while it is in leaf.
                        Kind::Foliage if matches!(mat.plant, plant::DECIDUOUS | plant::AUTUMN) => {
                            if s.leaves.len() == first {
                                s.tags[(lz * PATCH + lx) as usize] = voxels.leaf_tag(x, y, z);
                            }
                            s.leaves.push((y, mat.plant == plant::AUTUMN));
                        }
                        Kind::Foliage => floor = Some((y + 1, Surface::Leaves)),
                    }
                    if floor.is_none() {
                        y -= 1;
                    }
                }
                s.floor.push(floor.unwrap_or((y, Surface::Rock)));
            }
        }
        s.start.push(s.leaves.len() as u32);
        s
    }

    /// The patch of chunk column `col` as it is on `date`.
    fn patch(&self, col: (i32, i32), date: f32) -> Patch {
        let at = |x: i32, z: i32| ((z - col.1 * PATCH) * PATCH + (x - col.0 * PATCH)) as usize;
        let column = |x: i32, z: i32| {
            let i = at(x, z);
            let leaves = &self.leaves[self.start[i] as usize..self.start[i + 1] as usize];
            // The first leaf still on, going down, is the top: bare ones are looked through.
            match leaves.iter().find(|&&(y, maple)| crate::season::leaf(date, IVec3::new(x, y, z), maple, self.tags[i]).keep) {
                Some(&(y, _)) => (y + 1, Surface::Leaves),
                None => self.floor[i],
            }
        };
        Patch::build(col, column, |x, z| self.land[at(x, z)], self.berries.clone(), self.tall.clone(), date)
    }
}

/// Reads the patch of chunk column `col` as it is on `date` out of `voxels`, the way the
/// survey did before it kept its [`Source`]s: what the tests hold them to.
#[cfg(test)]
fn build(world: &GameWorld, voxels: &mut WorldVoxels, col: (i32, i32), date: f32) -> Patch {
    let mats = world.materials();
    let lib = world.library();
    let ceiling = voxels.ceiling(col);
    let inside = |p: IVec3| p.x.div_euclid(CHUNK) == col.0 && p.z.div_euclid(CHUNK) == col.1;
    let (mut bushes, mut tall) = (Vec::new(), Vec::new());
    for p in voxels.trees(col) {
        if !inside(p.pos) {
            continue;
        }
        if berries(p.species) {
            bushes.push(p.pos);
        } else if crown(p.species) && (p.top(lib) - p.pos.y) as f32 >= m(8.0) {
            tall.push((p.pos, p.top(lib) + 1));
        }
    }
    let column = |x: i32, z: i32| {
        let mut tag = None;
        let mut y = ceiling - 1;
        while y > ceiling - 1200 {
            let v = voxels.voxel(x, y, z);
            let mat = mats.get(v);
            match mat.kind {
                Kind::Air => {}
                Kind::Water => return (y + 1, Surface::Water),
                Kind::Solid => return (y + 1, Surface::Rock),
                Kind::Foliage => {
                    let bare = matches!(mat.plant, plant::DECIDUOUS | plant::AUTUMN) && {
                        let tag = *tag.get_or_insert_with(|| voxels.leaf_tag(x, y, z));
                        !crate::season::leaf(date, IVec3::new(x, y, z), mat.plant == plant::AUTUMN, tag).keep
                    };
                    if !bare {
                        return (y + 1, Surface::Leaves);
                    }
                }
            }
            y -= 1;
        }
        (y, Surface::Rock)
    };
    Patch::build(col, column, |x, z| world.column(x, z).top() + 1, bushes, tall, date)
}

/// Whether a patch made on `made` is out of date on `date`.
fn stale(made: f32, date: f32) -> bool {
    let d = (made - date).rem_euclid(1.0);
    d.min(1.0 - d) > 0.01
}

/// The map round `camera` and the places on it, from the patches held in `held` (made
/// as they are needed, each kept with what it was made from) and the rookeries found so
/// far in `rookeries`.
pub struct Survey {
    world: Arc<GameWorld>,
    held: HashMap<(i32, i32), (Option<Source>, Patch)>,
    /// What each square's rookery is, once looked for: its trees and nests.
    rookeries: HashMap<(i32, i32), Option<(Vec<(IVec3, i32)>, Vec<sites::Nest>)>>,
    /// Makes every patch afresh from new voxels, as the survey once did: the reference
    /// the tests hold the kept sources to.
    #[cfg(test)]
    afresh: bool,
}

impl Survey {
    pub fn new(world: Arc<GameWorld>) -> Self {
        Self {
            world,
            held: HashMap::new(),
            rookeries: HashMap::new(),
            #[cfg(test)]
            afresh: false,
        }
    }

    /// Makes sure the patch of `col` is held as it is about `date`. A column new to the
    /// survey is read from `voxels`. One already held only has its broadleaf crowns
    /// looked at again, once the date has moved on.
    fn ensure(&mut self, voxels: &mut WorldVoxels, col: (i32, i32), date: f32) {
        #[cfg(test)]
        if self.afresh {
            if self.held.get(&col).is_none_or(|(_, p)| stale(p.date, date)) {
                let mut fresh = WorldVoxels::new(self.world.clone());
                let p = build(&self.world, &mut fresh, col, date);
                self.held.insert(col, (None, p));
            }
            return;
        }
        match self.held.get_mut(&col) {
            Some((Some(source), patch)) => {
                if stale(patch.date, date) {
                    *patch = source.patch(col, date);
                }
            }
            _ => {
                // Only this column's own voxels are read, so the others can go: voxels
                // take a hundred times the room of what is kept of them.
                voxels.trim(DVec3::new((col.0 * CHUNK + CHUNK / 2) as f64, 0.0, (col.1 * CHUNK + CHUNK / 2) as f64));
                let source = Source::read(&self.world, voxels, col);
                let patch = source.patch(col, date);
                self.held.insert(col, (Some(source), patch));
            }
        }
    }

    /// Looks round `camera` on `date`.
    pub fn look(&mut self, camera: DVec3, date: f32) -> Sites {
        let t = std::time::Instant::now();
        let reach = (m(REACH) / CHUNK as f32).ceil() as i32;
        let (cx, cz) = ((camera.x / CHUNK as f64).floor() as i32, (camera.z / CHUNK as f64).floor() as i32);
        let made = self.held.len();
        // One set of voxels for the whole look, kept to the few columns being read.
        let mut voxels = WorldVoxels::new(self.world.clone());
        for z in cz - reach..=cz + reach {
            for x in cx - reach..=cx + reach {
                self.ensure(&mut voxels, (x, z), date);
            }
        }
        // The rookeries in sight: each looked for once, from the patches round its point.
        let seed = self.world.seed();
        let sight = m(ROOKERY_SIGHT) as f64;
        let (rx, rz) = ((camera.x / ROOKERY_CELL as f64).floor() as i32, (camera.z / ROOKERY_CELL as f64).floor() as i32);
        let mut nests = Vec::new();
        let mut rookery_trees = Vec::new();
        for z in rz - 1..=rz + 1 {
            for x in rx - 1..=rx + 1 {
                let Some(anchor) = sites::rookery_anchor(seed, (x, z)) else { continue };
                if ((anchor.0 as f64 - camera.x).hypot(anchor.1 as f64 - camera.z)) > sight {
                    continue;
                }
                if !self.rookeries.contains_key(&(x, z)) {
                    let found = self.find_rookery(&mut voxels, anchor, date);
                    self.rookeries.insert((x, z), found);
                }
                if let Some(Some((trees, n))) = self.rookeries.get(&(x, z)) {
                    nests.extend_from_slice(n);
                    rookery_trees.push(trees.clone());
                }
            }
        }
        drop(voxels);
        // Let go of what is well out of reach.
        let keep = reach + 3;
        self.held.retain(|&(x, z), _| (x - cx).abs() <= keep && (z - cz).abs() <= keep);
        self.rookeries.retain(|&(x, z), _| (x - rx).abs() <= 2 && (z - rz).abs() <= 2);

        let grid = Grid::of((cx - reach, cz - reach), (cx + reach, cz + reach), |c| self.held.get(&c).map(|(_, p)| p));
        let berries: Vec<IVec3> = (cz - reach..=cz + reach)
            .flat_map(|z| (cx - reach..=cx + reach).map(move |x| (x, z)))
            .filter_map(|c| self.held.get(&c))
            .flat_map(|(_, p)| p.berries.iter().copied())
            .collect();
        let highs = sites::high_points(&grid);
        let mut places = sites::feeding_grounds(&grid, &berries, &highs);
        for trees in &rookery_trees {
            let mine: Vec<sites::Nest> = nests.iter().copied().filter(|n| trees.iter().any(|(f, _)| (n.pos.x - f.x as f64).hypot(n.pos.z - f.z as f64) < m(6.0) as f64)).collect();
            if let Some(r) = sites::roost(trees, &mine, &highs) {
                places.push(r);
            }
        }
        places.extend(highs);
        let kinds = |k: sites::Kind| places.iter().filter(|p| p.kind == k).count();
        log::debug!(
            "crows' map round {:.0} {:.0}: {} patches made in {:.0} ms; {} crowns, {} rocks, {} berry bushes, {} fields, {} roosts, {} nests",
            camera.x,
            camera.z,
            self.held.len().saturating_sub(made),
            t.elapsed().as_secs_f64() * 1000.0,
            kinds(sites::Kind::Tree),
            kinds(sites::Kind::Rock),
            kinds(sites::Kind::Berries),
            kinds(sites::Kind::Field),
            kinds(sites::Kind::Roost),
            nests.len(),
        );
        Sites { grid, places, nests }
    }

    /// The rookery round `anchor`, if there is one: its trees, and nests built in them.
    fn find_rookery(&mut self, voxels: &mut WorldVoxels, anchor: (i32, i32), date: f32) -> Option<(Vec<(IVec3, i32)>, Vec<sites::Nest>)> {
        let r = ((m(ROOKERY_SEARCH) + m(16.0)) / CHUNK as f32).ceil() as i32;
        let (ax, az) = (anchor.0.div_euclid(CHUNK), anchor.1.div_euclid(CHUNK));
        let mut tall = Vec::new();
        for z in az - r..=az + r {
            for x in ax - r..=ax + r {
                self.ensure(voxels, (x, z), date);
                tall.extend(self.held[&(x, z)].1.tall.iter().copied());
            }
        }
        let trees = sites::rookery_trees(anchor, &tall);
        if trees.is_empty() {
            return None;
        }
        let mats = self.world.materials();
        // The nests are found in the voxels round the anchor: keep those just read.
        voxels.trim(DVec3::new(anchor.0 as f64, 0.0, anchor.1 as f64));
        let nests = sites::place_nests(self.world.seed(), &trees, |x, y, z| mats.get(voxels.voxel(x, y, z)).kind == Kind::Solid);
        log::info!("a rookery of {} trees and {} nests at {} {}", trees.len(), nests.len(), trees[0].0.x, trees[0].0.z);
        Some((trees, nests))
    }
}

/// Runs a [`Survey`] on its own thread: making the patches means generating chunk
/// columns, which would stall a frame.
pub struct Lookout {
    ask: Sender<(DVec3, f32)>,
    answer: Receiver<Sites>,
    busy: bool,
}

impl Lookout {
    pub fn new(world: Arc<GameWorld>) -> Self {
        let (ask, questions) = channel::<(DVec3, f32)>();
        let (reply, answer) = channel();
        std::thread::Builder::new()
            .name("crows".into())
            .spawn(move || {
                let mut survey = Survey::new(world);
                // Ends when the game drops `ask`.
                while let Ok((mut camera, mut date)) = questions.recv() {
                    // Only the latest question matters.
                    while let Ok(q) = questions.try_recv() {
                        (camera, date) = q;
                    }
                    if reply.send(survey.look(camera, date)).is_err() {
                        break;
                    }
                }
            })
            .expect("crows thread");
        Self { ask, answer, busy: false }
    }

    pub fn request(&mut self, camera: DVec3, date: f32) {
        if !self.busy {
            self.busy = self.ask.send((camera, date)).is_ok();
        }
    }

    pub fn poll(&mut self) -> Option<Sites> {
        let found = self.answer.try_recv().ok()?;
        self.busy = false;
        Some(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The map made from each column's source, read once and then turned with the
    /// seasons, is the one the survey made when it generated every patch afresh: by the
    /// rookery at Wenna's Lodge in leaf, then a little way on in winter, then in autumn.
    #[test]
    fn kept_sources_make_the_same_map_as_generating_afresh() {
        let world = Arc::new(GameWorld::new(1));
        let (mut kept, mut afresh) = (Survey::new(world.clone()), Survey::new(world));
        afresh.afresh = true;
        let here = DVec3::new(26416.0, 0.0, 28358.0);
        let on = here + DVec3::new(3.0 * CHUNK as f64, 0.0, -CHUNK as f64);
        for (camera, date) in [(here, 0.55), (on, 0.95), (on, 0.70)] {
            let (a, b) = (kept.look(camera, date), afresh.look(camera, date));
            assert_eq!(format!("{a:?}"), format!("{b:?}"), "the maps differ round {camera} on {date}");
            assert!(!a.nests.is_empty() && !a.places.is_empty(), "nothing for the crows round {camera}");
        }
        // The time of year shows: some crowns in leaf in summer are bare in winter.
        let leafy = |date: f32| -> usize {
            kept.held.iter().map(|(&c, (s, _))| s.as_ref().unwrap().patch(c, date).blocks.iter().filter(|b| b.surface == Surface::Leaves).count()).sum()
        };
        let (summer, winter) = (leafy(0.55), leafy(0.95));
        assert!(winter < summer, "{winter} leafy cells in winter against {summer} in summer");
    }
}
