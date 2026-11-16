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
use worldgen::{World, features::trees::Species, scale::m, world::GameWorld};

use super::sites::{self, Grid, Patch, ROOKERY_CELL, ROOKERY_SEARCH, Sites, Surface};
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

/// Reads the patch of chunk column `col` as it is on `date` out of `voxels`.
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
                    // A broadleaf crown that is bare at this time of year is no seat.
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

/// The map round `camera` and the places on it, from the patches held in `held` (made
/// as they are needed) and the rookeries found so far in `rookeries`.
pub struct Survey {
    world: Arc<GameWorld>,
    held: HashMap<(i32, i32), Patch>,
    /// What each square's rookery is, once looked for: its trees and nests.
    rookeries: HashMap<(i32, i32), Option<(Vec<(IVec3, i32)>, Vec<sites::Nest>)>>,
}

impl Survey {
    pub fn new(world: Arc<GameWorld>) -> Self {
        Self { world, held: HashMap::new(), rookeries: HashMap::new() }
    }

    fn ensure(&mut self, col: (i32, i32), date: f32) {
        let stale = self.held.get(&col).is_none_or(|p| {
            let d = (p.date - date).rem_euclid(1.0);
            d.min(1.0 - d) > 0.01
        });
        if stale {
            let mut voxels = WorldVoxels::new(self.world.clone());
            let p = build(&self.world, &mut voxels, col, date);
            self.held.insert(col, p);
        }
    }

    /// Looks round `camera` on `date`.
    pub fn look(&mut self, camera: DVec3, date: f32) -> Sites {
        let t = std::time::Instant::now();
        let reach = (m(REACH) / CHUNK as f32).ceil() as i32;
        let (cx, cz) = ((camera.x / CHUNK as f64).floor() as i32, (camera.z / CHUNK as f64).floor() as i32);
        let made = self.held.len();
        for z in cz - reach..=cz + reach {
            for x in cx - reach..=cx + reach {
                self.ensure((x, z), date);
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
                    let found = self.find_rookery(anchor, date);
                    self.rookeries.insert((x, z), found);
                }
                if let Some(Some((trees, n))) = self.rookeries.get(&(x, z)) {
                    nests.extend_from_slice(n);
                    rookery_trees.push(trees.clone());
                }
            }
        }
        // Let go of what is well out of reach.
        let keep = reach + 3;
        self.held.retain(|&(x, z), _| (x - cx).abs() <= keep && (z - cz).abs() <= keep);
        self.rookeries.retain(|&(x, z), _| (x - rx).abs() <= 2 && (z - rz).abs() <= 2);

        let grid = Grid::of((cx - reach, cz - reach), (cx + reach, cz + reach), |c| self.held.get(&c));
        let berries: Vec<IVec3> = (cz - reach..=cz + reach)
            .flat_map(|z| (cx - reach..=cx + reach).map(move |x| (x, z)))
            .filter_map(|c| self.held.get(&c))
            .flat_map(|p| p.berries.iter().copied())
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
    fn find_rookery(&mut self, anchor: (i32, i32), date: f32) -> Option<(Vec<(IVec3, i32)>, Vec<sites::Nest>)> {
        let r = ((m(ROOKERY_SEARCH) + m(16.0)) / CHUNK as f32).ceil() as i32;
        let (ax, az) = (anchor.0.div_euclid(CHUNK), anchor.1.div_euclid(CHUNK));
        let mut tall = Vec::new();
        for z in az - r..=az + r {
            for x in ax - r..=ax + r {
                self.ensure((x, z), date);
                tall.extend(self.held[&(x, z)].tall.iter().copied());
            }
        }
        let trees = sites::rookery_trees(anchor, &tall);
        if trees.is_empty() {
            return None;
        }
        let mats = self.world.materials();
        let mut voxels = WorldVoxels::new(self.world.clone());
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
