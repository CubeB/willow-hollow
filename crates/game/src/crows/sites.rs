//! Where crows can be: the high points they perch on, the open ground they feed on, and
//! the rookeries they go back to at dusk.
//!
//! All of it is read from a coarse map of the tops of things, a block of two by two
//! voxel columns to a cell, made on a worker thread (see `survey.rs`). The same map is
//! what a crow in the air keeps above, so that it never flies into a hill or a tree it
//! cannot see. Nothing here changes the world: a rookery's nests are drawn as models,
//! and where they are comes from the trees the world grows and a hash of their places.

use glam::{DVec3, IVec3, Vec2};
use worldgen::{
    noise::{hash2, hash3, unit},
    scale::m,
};

/// Voxel columns a cell of the map is across.
pub const BLOCK: i32 = 2;
/// Voxel columns a patch of the map is across: a chunk column, as the world makes them.
pub const PATCH: i32 = voxel::CHUNK;
const SIDE: i32 = PATCH / BLOCK;

/// What the top of a column is.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Surface {
    /// Not looked at yet.
    #[default]
    Unknown,
    /// Rock, earth, timber, a roof: anything solid.
    Rock,
    /// The leaves of a crown, or a bush, in leaf.
    Leaves,
    Water,
}

/// The top of a cell of the map.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Block {
    /// The height of the top face of the highest voxel among its columns.
    pub top: i32,
    pub surface: Surface,
    /// Which of its four columns is the highest: x in bit 0, z in bit 1.
    pub at: u8,
    /// How far that top stands above the land itself, in voxels: 0 for the ground, more
    /// for a boulder, a roof, a crown.
    pub lift: i32,
    /// Whether it is open, level ground: the land itself, its four columns at one height.
    pub ground: bool,
}

/// One chunk column of the map, and what grows there that matters to a crow.
#[derive(Clone, Debug)]
pub struct Patch {
    pub blocks: Vec<Block>,
    /// The foot of each bush bearing berries.
    pub berries: Vec<IVec3>,
    /// The foot of each tall tree, and the height of its top.
    pub tall: Vec<(IVec3, i32)>,
    /// The time of year it was made for: bare trees are not perched in.
    pub date: f32,
}

impl Patch {
    /// The map of chunk column `col`. `column(x, z)` is the top of the world column there
    /// (the height of the top face of its highest voxel) and what it is; `land(x, z)` the
    /// height of the top face of the land itself there, under anything standing on it.
    pub fn build(
        col: (i32, i32),
        mut column: impl FnMut(i32, i32) -> (i32, Surface),
        mut land: impl FnMut(i32, i32) -> i32,
        berries: Vec<IVec3>,
        tall: Vec<(IVec3, i32)>,
        date: f32,
    ) -> Patch {
        let mut blocks = Vec::with_capacity((SIDE * SIDE) as usize);
        for bz in 0..SIDE {
            for bx in 0..SIDE {
                let (x0, z0) = (col.0 * PATCH + bx * BLOCK, col.1 * PATCH + bz * BLOCK);
                let tops = [column(x0, z0), column(x0 + 1, z0), column(x0, z0 + 1), column(x0 + 1, z0 + 1)];
                let at = (0..4).max_by_key(|&i| (tops[i].0, -(i as i32))).unwrap_or(0);
                let (top, surface) = tops[at];
                let lift = top - land(x0 + (at & 1) as i32, z0 + (at >> 1) as i32);
                let level = tops.iter().all(|&(t, s)| t == top && s == Surface::Rock);
                blocks.push(Block { top, surface, at: at as u8, lift, ground: level && lift.abs() <= 1 });
            }
        }
        Patch { blocks, berries, tall, date }
    }
}

/// The map round a place, put together from patches.
#[derive(Clone, Debug, Default)]
pub struct Grid {
    /// The cell in its first corner, in cells from the world's origin.
    x0: i32,
    z0: i32,
    w: i32,
    h: i32,
    blocks: Vec<Block>,
}

impl Grid {
    /// The map of the patches from `lo` to `hi` (chunk columns, inclusive), from those
    /// that are made.
    pub fn of<'a>(lo: (i32, i32), hi: (i32, i32), patch: impl Fn((i32, i32)) -> Option<&'a Patch>) -> Grid {
        let (w, h) = ((hi.0 - lo.0 + 1) * SIDE, (hi.1 - lo.1 + 1) * SIDE);
        let mut blocks = vec![Block::default(); (w * h) as usize];
        for pz in lo.1..=hi.1 {
            for px in lo.0..=hi.0 {
                let Some(p) = patch((px, pz)) else { continue };
                for bz in 0..SIDE {
                    let row = ((pz - lo.1) * SIDE + bz) * w + (px - lo.0) * SIDE;
                    blocks[row as usize..(row + SIDE) as usize].copy_from_slice(&p.blocks[(bz * SIDE) as usize..((bz + 1) * SIDE) as usize]);
                }
            }
        }
        Grid { x0: lo.0 * SIDE, z0: lo.1 * SIDE, w, h, blocks }
    }

    /// A map from a function of the cell, for tests: `w` by `h` cells from cell `(x0, z0)`.
    #[cfg(test)]
    pub fn from_fn(x0: i32, z0: i32, w: i32, h: i32, f: impl Fn(i32, i32) -> Block) -> Grid {
        let blocks = (0..w * h).map(|i| f(x0 + i % w, z0 + i / w)).collect();
        Grid { x0, z0, w, h, blocks }
    }

    /// The cell `(bx, bz)` (in cells from the world's origin), if it is on the map and
    /// has been looked at.
    pub fn block(&self, bx: i32, bz: i32) -> Option<Block> {
        let (i, j) = (bx - self.x0, bz - self.z0);
        if i < 0 || j < 0 || i >= self.w || j >= self.h {
            return None;
        }
        Some(self.blocks[(j * self.w + i) as usize]).filter(|b| b.surface != Surface::Unknown)
    }

    /// The cell that voxel column `(x, z)` is in.
    #[cfg(test)]
    pub fn at(&self, x: f64, z: f64) -> Option<Block> {
        self.block((x / BLOCK as f64).floor() as i32, (z / BLOCK as f64).floor() as i32)
    }

    /// The highest top of anything within a little of `(x, z)`, so that something kept
    /// above it clears the edges of what it passes over too. With `rock_only`, leaves are
    /// passed through. None where none of it has been looked at.
    pub fn top(&self, x: f64, z: f64, rock_only: bool) -> Option<f64> {
        self.top_within(x, z, 0.6, rock_only)
    }

    /// The top of the cell right under `(x, z)`: what something there stands on.
    pub fn under(&self, x: f64, z: f64, rock_only: bool) -> Option<f64> {
        self.top_within(x, z, 0.0, rock_only)
    }

    fn top_within(&self, x: f64, z: f64, r: f64, rock_only: bool) -> Option<f64> {
        let (a, b) = (((x - r) / BLOCK as f64).floor() as i32, ((x + r) / BLOCK as f64).floor() as i32);
        let (c, d) = (((z - r) / BLOCK as f64).floor() as i32, ((z + r) / BLOCK as f64).floor() as i32);
        let mut top: Option<f64> = None;
        for bz in c..=d {
            for bx in a..=b {
                if let Some(k) = self.block(bx, bz) {
                    let t = if rock_only && k.surface == Surface::Leaves { k.top - k.lift } else { k.top };
                    top = Some(top.map_or(t as f64, |v: f64| v.max(t as f64)));
                }
            }
        }
        top
    }

    /// The top of the highest column of cell `(bx, bz)`, where a crow would stand on it.
    fn seat(&self, bx: i32, bz: i32, b: Block) -> DVec3 {
        let (x, z) = (bx * BLOCK + (b.at & 1) as i32, bz * BLOCK + (b.at >> 1) as i32);
        DVec3::new(x as f64 + 0.5, b.top as f64, z as f64 + 0.5)
    }

    /// The middle of cell `(bx, bz)`: on level ground, where its four columns meet.
    fn middle(&self, bx: i32, bz: i32, b: Block) -> DVec3 {
        DVec3::new(((bx * BLOCK) as f64) + BLOCK as f64 * 0.5, b.top as f64, ((bz * BLOCK) as f64) + BLOCK as f64 * 0.5)
    }

    fn cells(&self) -> impl Iterator<Item = (i32, i32, Block)> + '_ {
        (0..self.w * self.h).map(move |i| (self.x0 + i % self.w, self.z0 + i / self.w, self.blocks[i as usize]))
    }

    /// Whether a crow could stand at `at` on the open ground: a level cell of the land,
    /// with ground of about the same height all round it.
    pub fn open_ground(&self, at: DVec3) -> Option<f64> {
        let (bx, bz) = ((at.x / BLOCK as f64).floor() as i32, (at.z / BLOCK as f64).floor() as i32);
        let b = self.block(bx, bz).filter(|b| b.ground)?;
        let level = [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().all(|(dx, dz)| self.block(bx + dx, bz + dz).is_some_and(|n| n.surface == Surface::Rock && (n.top - b.top).abs() <= 1));
        level.then_some(b.top as f64)
    }
}

/// What a place is, to a crow.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// The crown of a tall tree.
    Tree,
    /// The top of a tor, a roof, a tower, a great boulder.
    Rock,
    /// The ground about a bush in berry.
    Berries,
    /// Open ground near the trees.
    Field,
    /// A rookery: its nests and the crowns of its trees.
    Roost,
}

impl Kind {
    pub fn on_ground(self) -> bool {
        matches!(self, Kind::Berries | Kind::Field)
    }
}

/// Somewhere a group of crows can go, and the spots in it where each can stand.
#[derive(Clone, Debug)]
pub struct Place {
    pub kind: Kind,
    pub centre: DVec3,
    pub spots: Vec<DVec3>,
}

/// How far above the land a crown stands, at the least, for crows to perch on it.
const TALL_CROWN: f32 = 7.0;
/// And the top of a rock or a roof.
const HIGH_ROCK: f32 = 2.4;
/// The most spots a crow can sit in on one high point.
const SEATS: usize = 6;

/// The high points on the map that crows perch on: the crowns of tall trees, tor tops,
/// roofs. Each is the highest cell for a couple of cells round, standing well up from the
/// land, with the cells about it near as high as seats for the rest of a group.
pub fn high_points(grid: &Grid) -> Vec<Place> {
    let mut out = Vec::new();
    for (bx, bz, b) in grid.cells() {
        let kind = match b.surface {
            Surface::Leaves if b.lift as f32 >= m(TALL_CROWN) => Kind::Tree,
            Surface::Rock if b.lift as f32 >= m(HIGH_ROCK) => Kind::Rock,
            _ => continue,
        };
        // The highest about it; of two as high, the first.
        let me = (bz, bx);
        let highest = (-2..=2).all(|dz: i32| {
            (-2..=2).all(|dx: i32| {
                (dx == 0 && dz == 0)
                    || grid.block(bx + dx, bz + dz).is_none_or(|n| n.top < b.top || (n.top == b.top && (bz + dz, bx + dx) > me))
            })
        });
        if !highest {
            continue;
        }
        let mut seats = vec![(b.top, grid.seat(bx, bz, b))];
        for dz in -2..=2i32 {
            for dx in -2..=2i32 {
                if (dx, dz) == (0, 0) || dx * dx + dz * dz > 5 {
                    continue;
                }
                if let Some(n) = grid.block(bx + dx, bz + dz)
                    && n.surface == b.surface
                    && n.top >= b.top - 2
                    && n.lift >= b.lift - 3
                {
                    seats.push((n.top, grid.seat(bx + dx, bz + dz, n)));
                }
            }
        }
        seats.sort_by_key(|s| -s.0);
        seats.truncate(SEATS);
        let spots: Vec<DVec3> = seats.into_iter().map(|s| s.1).collect();
        out.push(Place { kind, centre: spots[0], spots });
    }
    out
}

/// Ground spots near `at`, within `reach` voxels of it, and no nearer than `near`: level
/// cells of open land at about the height of `at`, `apart` cells from one another.
fn ground_about(grid: &Grid, at: DVec3, near: f64, reach: f64, most: usize) -> Vec<DVec3> {
    let (cx, cz) = ((at.x / BLOCK as f64).floor() as i32, (at.z / BLOCK as f64).floor() as i32);
    let r = (reach / BLOCK as f64).ceil() as i32;
    let mut spots: Vec<DVec3> = Vec::new();
    // Outward from the middle, so the nearest are kept when there are many.
    let mut ring: Vec<(i32, i32)> = (-r..=r).flat_map(|dz| (-r..=r).map(move |dx| (dx, dz))).collect();
    ring.sort_by_key(|&(dx, dz)| (dx * dx + dz * dz, hash2(0xc20, cx + dx, cz + dz)));
    for (dx, dz) in ring {
        let Some(b) = grid.block(cx + dx, cz + dz) else { continue };
        let p = grid.middle(cx + dx, cz + dz, b);
        let d = Vec2::new((p.x - at.x) as f32, (p.z - at.z) as f32).length() as f64;
        if d < near || d > reach || (p.y - at.y).abs() > 3.0 || grid.open_ground(p).is_none() {
            continue;
        }
        if spots.iter().all(|s| s.distance(p) >= BLOCK as f64 * 1.2) {
            spots.push(p);
            if spots.len() >= most {
                break;
            }
        }
    }
    spots
}

/// The places crows feed: the open ground round each bush in berry, and open ground not
/// far from tall trees, where crows walk about looking for what they can find.
pub fn feeding_grounds(grid: &Grid, berries: &[IVec3], trees: &[Place]) -> Vec<Place> {
    let mut out = Vec::new();
    for &bush in berries {
        let foot = DVec3::new(bush.x as f64 + 0.5, bush.y as f64, bush.z as f64 + 0.5);
        let spots = ground_about(grid, foot, m(1.1) as f64, m(4.5) as f64, 10);
        if spots.len() >= 3 {
            out.push(Place { kind: Kind::Berries, centre: foot, spots });
        }
    }
    // Fields: a few patches of open ground, each a short flight from a crown.
    let step = (m(9.0) as i32 / BLOCK).max(1);
    let mut fields = Vec::new();
    for (bx, bz, b) in grid.cells() {
        if bx.rem_euclid(step) != 0 || bz.rem_euclid(step) != 0 || !b.ground {
            continue;
        }
        let at = grid.middle(bx, bz, b);
        let near_trees = trees.iter().any(|t| {
            let d = Vec2::new((t.centre.x - at.x) as f32, (t.centre.z - at.z) as f32).length();
            d > m(6.0) && d < m(35.0)
        });
        if !near_trees || berries.iter().any(|f| (f.as_dvec3() - at).length() < m(8.0) as f64) {
            continue;
        }
        let spots = ground_about(grid, at, 0.0, m(4.0) as f64, 10);
        if spots.len() >= 6 {
            fields.push(Place { kind: Kind::Field, centre: at, spots });
        }
    }
    // Not so many that the berries are never chosen.
    fields.sort_by_key(|f| hash2(0xf1e1, f.centre.x as i32, f.centre.z as i32));
    fields.truncate(40);
    out.extend(fields);
    out
}

/// A stick nest in the crown of a rookery tree.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Nest {
    /// The middle of its underside, on the branch it is built on.
    pub pos: DVec3,
    pub yaw: f32,
    /// Across, in metres.
    pub size: f32,
    pub salt: u32,
}

impl Nest {
    /// Its height, in metres.
    pub fn height(&self) -> f32 {
        self.size * 0.42
    }

    /// Where a crow sits on its rim.
    pub fn seat(&self) -> DVec3 {
        self.pos + DVec3::new(0.0, m(self.height()) as f64 * 0.9, 0.0)
    }
}

/// How far apart (voxels) the rookeries are chosen from: one at most to a square this
/// size, round the trees nearest a point in it.
pub const ROOKERY_CELL: i32 = m(300.0) as i32;
/// How far from that point the trees of a rookery are looked for.
pub const ROOKERY_SEARCH: f32 = 45.0;
/// Trees within this many metres of one another stand in one rookery.
const ROOKERY_SPREAD: f32 = 14.0;
/// How tall, from foot to top, a tree must be to have rooks' nests in it.
const ROOKERY_TREE: f32 = 9.0;

/// The point the rookery of square `cell` is looked for round, if it has one at all.
pub fn rookery_anchor(seed: u32, cell: (i32, i32)) -> Option<(i32, i32)> {
    let h = |k: i32| unit(hash3(seed ^ 0x7007, cell.0, k, cell.1));
    (h(0) < 0.85).then(|| (cell.0 * ROOKERY_CELL + ((0.15 + 0.7 * h(1)) * ROOKERY_CELL as f32) as i32, cell.1 * ROOKERY_CELL + ((0.15 + 0.7 * h(2)) * ROOKERY_CELL as f32) as i32))
}

/// The trees of the rookery round `anchor`, out of `tall` (each tree's foot and the
/// height of its top), which must hold every tall tree within [`ROOKERY_SEARCH`] metres
/// of it and [`ROOKERY_SPREAD`] beyond. A rookery is a cluster of tall trees, four at the
/// least: the one with the most tall trees round it, and those trees. None if there is
/// no such cluster.
pub fn rookery_trees(anchor: (i32, i32), tall: &[(IVec3, i32)]) -> Vec<(IVec3, i32)> {
    let tall: Vec<(IVec3, i32)> = tall.iter().copied().filter(|(foot, top)| (top - foot.y) as f32 >= m(ROOKERY_TREE)).collect();
    let flat = |a: IVec3, b: IVec3| (((a.x - b.x) as f32).powi(2) + ((a.z - b.z) as f32).powi(2)).sqrt();
    let spread = m(ROOKERY_SPREAD);
    let anchor = IVec3::new(anchor.0, 0, anchor.1);
    let best = tall
        .iter()
        .filter(|(foot, _)| flat(*foot, anchor) <= m(ROOKERY_SEARCH))
        .map(|&(foot, top)| {
            let n = tall.iter().filter(|(o, _)| *o != foot && flat(*o, foot) <= spread).count();
            (n, std::cmp::Reverse(hash3(0x2007, foot.x, foot.y, foot.z)), foot, top)
        })
        .max_by_key(|&(n, h, _, _)| (n, h));
    let Some((n, _, foot, top)) = best.filter(|b| b.0 >= 3) else { return Vec::new() };
    let mut trees: Vec<(IVec3, i32)> = tall.iter().copied().filter(|(o, _)| *o != foot && flat(*o, foot) <= spread).collect();
    trees.sort_by(|a, b| flat(a.0, foot).total_cmp(&flat(b.0, foot)));
    trees.truncate(n.min(6));
    trees.insert(0, (foot, top));
    trees
}

/// Where the nests of a rookery are built, in its `trees`: high in each crown, on a bough
/// with nothing solid over it, one to three to a tree. `solid(x, y, z)` says whether a
/// voxel is solid (wood, not leaves).
pub fn place_nests(seed: u32, trees: &[(IVec3, i32)], mut solid: impl FnMut(i32, i32, i32) -> bool) -> Vec<Nest> {
    let mut nests: Vec<Nest> = Vec::new();
    for (i, &(foot, top)) in trees.iter().enumerate() {
        let roll = |k: i32| unit(hash3(seed ^ 0x2e57, foot.x, k, foot.z));
        let want = if i == 0 { 3 } else { 1 + (roll(0) * 2.99) as usize };
        let low = foot.y + ((top - foot.y) as f32 * 0.55) as i32;
        let mut built = 0;
        for k in 0..40 {
            if built >= want {
                break;
            }
            let (a, r) = (roll(1 + k * 3) * std::f32::consts::TAU, roll(2 + k * 3) * 4.0);
            let (x, z) = (foot.x + (a.cos() * r).round() as i32, foot.z + (a.sin() * r).round() as i32);
            // The highest bough in that column, high in the crown.
            let Some(y) = (low..=top).rev().find(|&y| solid(x, y, z) && !solid(x, y + 1, z) && !solid(x, y + 2, z)) else { continue };
            let pos = DVec3::new(x as f64 + 0.5, y as f64 + 1.0, z as f64 + 0.5);
            if nests.iter().any(|n| n.pos.distance(pos) < m(1.6) as f64) {
                continue;
            }
            let salt = hash3(seed ^ 0x9e57, x, y, z);
            nests.push(Nest { pos, yaw: unit(salt) * std::f32::consts::TAU, size: 0.55 + 0.3 * unit(salt >> 7), salt });
            built += 1;
        }
    }
    nests
}

/// The roost of a rookery: its nests, and the crowns of its trees on the map.
pub fn roost(trees: &[(IVec3, i32)], nests: &[Nest], highs: &[Place]) -> Option<Place> {
    let mut spots: Vec<DVec3> = nests.iter().map(Nest::seat).collect();
    for h in highs.iter().filter(|h| h.kind == Kind::Tree) {
        if trees.iter().any(|(foot, _)| Vec2::new((h.centre.x - foot.x as f64) as f32, (h.centre.z - foot.z as f64) as f32).length() < m(4.0)) {
            spots.extend(h.spots.iter().copied());
        }
    }
    if spots.is_empty() {
        return None;
    }
    let n = trees.len().max(1) as f64;
    let middle = trees.iter().map(|(f, t)| DVec3::new(f.x as f64 + 0.5, *t as f64, f.z as f64 + 0.5)).sum::<DVec3>() / n;
    Some(Place { kind: Kind::Roost, centre: middle, spots })
}

/// Everything a crow needs to know about the country round the player.
#[derive(Clone, Debug, Default)]
pub struct Sites {
    pub grid: Grid,
    pub places: Vec<Place>,
    pub nests: Vec<Nest>,
}

impl Sites {
    pub fn roosts(&self) -> impl Iterator<Item = &Place> {
        self.places.iter().filter(|p| p.kind == Kind::Roost)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A little country: level land at height 10, an oak's crown 9 m up round (40, 40),
    /// a tor of rock 3 m high round (100, 40), and a bush in berry at (60, 80).
    fn country(x: i32, z: i32) -> (i32, Surface) {
        let crown = (x - 40).pow(2) + (z - 40).pow(2) <= 16;
        let tor = (95..=104).contains(&x) && (36..=44).contains(&z);
        if crown {
            (10 + m(9.0) as i32 - ((x - 40).pow(2) + (z - 40).pow(2)) / 6, Surface::Leaves)
        } else if tor {
            (10 + m(3.0) as i32 + ((x == 99) as i32), Surface::Rock)
        } else {
            (10, Surface::Rock)
        }
    }

    fn grid() -> Grid {
        let patches: Vec<((i32, i32), Patch)> = (0..5 * 5)
            .map(|i| {
                let col = (i % 5, i / 5);
                (col, Patch::build(col, country, |_, _| 10, vec![IVec3::new(60, 10, 80)], vec![], 0.3))
            })
            .collect();
        Grid::of((0, 0), (4, 4), |c| patches.iter().find(|p| p.0 == c).map(|p| &p.1))
    }

    #[test]
    fn perches_are_the_tops_of_crowns_and_tors_and_never_the_open_ground() {
        let g = grid();
        let highs = high_points(&g);
        let trees: Vec<_> = highs.iter().filter(|p| p.kind == Kind::Tree).collect();
        let rocks: Vec<_> = highs.iter().filter(|p| p.kind == Kind::Rock).collect();
        assert_eq!((trees.len(), rocks.len()), (1, 1), "{highs:?}");
        assert!(trees[0].centre.distance(DVec3::new(40.5, 0.0, 40.5).with_y(trees[0].centre.y)) < 3.0);
        assert!(rocks[0].centre.x > 94.0 && rocks[0].centre.x < 105.0);
        for p in &highs {
            assert!(p.spots.len() >= 3, "only {} seats on {:?}", p.spots.len(), p.kind);
            for s in &p.spots {
                // Each is on the top face of something: what is under it is solid or
                // leaves, and the voxel it stands in is clear.
                let (top, surface) = country(s.x.floor() as i32, s.z.floor() as i32);
                assert_eq!(s.y, top as f64, "{s} is not on the top of its column");
                assert!(matches!(surface, Surface::Rock | Surface::Leaves));
            }
        }
        // Nothing perches on the plain land.
        assert!(highs.iter().all(|p| p.spots.iter().all(|s| s.y > 10.0 + m(2.0) as f64)));
    }

    #[test]
    fn crows_feed_on_level_ground_by_the_berries_and_near_the_trees() {
        let g = grid();
        let highs = high_points(&g);
        let food = feeding_grounds(&g, &[IVec3::new(60, 10, 80)], &highs);
        let berries: Vec<_> = food.iter().filter(|p| p.kind == Kind::Berries).collect();
        assert_eq!(berries.len(), 1);
        assert!(food.iter().any(|p| p.kind == Kind::Field));
        for p in &food {
            for s in &p.spots {
                assert_eq!(s.y, 10.0, "{s} is not on the ground");
                // Not on the tor, nor under the crown.
                assert!(!(93.0..106.0).contains(&s.x) || !(34.0..46.0).contains(&s.z));
                assert!(Vec2::new(s.x as f32 - 40.5, s.z as f32 - 40.5).length() > 5.0);
            }
        }
        for s in &berries[0].spots {
            let d = Vec2::new(s.x as f32 - 60.5, s.z as f32 - 80.5).length();
            assert!(d >= m(1.0) && d <= m(4.6), "{d} from the bush");
        }
    }

    #[test]
    fn a_rookery_is_a_cluster_of_tall_trees_with_nests_on_their_boughs() {
        // Six tall trees close together and two standing off on their own.
        let mut tall: Vec<(IVec3, i32)> = (0..6).map(|i| (IVec3::new(100 + (i % 3) * 12, 10, 100 + (i / 3) * 12), 10 + m(16.0) as i32)).collect();
        tall.push((IVec3::new(160, 10, 100), 10 + m(16.0) as i32));
        tall.push((IVec3::new(100, 10, 160), 10 + m(16.0) as i32));
        let trees = rookery_trees((110, 105), &tall);
        assert_eq!(trees.len(), 6, "{trees:?}");
        assert!(trees.iter().all(|(f, _)| f.x < 130 && f.z < 120));
        // Always the same ones.
        assert_eq!(trees, rookery_trees((110, 105), &tall));
        // Two lone trees are not a rookery.
        assert!(rookery_trees((160, 100), &tall[6..]).is_empty());
        // Boughs: every tree has a branch layer at two thirds of its height.
        let bough = |x: i32, y: i32, z: i32| {
            trees.iter().any(|(f, t)| {
                let up = (f.y + (t - f.y) * 2 / 3, (x - f.x).abs() <= 3 && (z - f.z).abs() <= 3);
                (y == up.0 && up.1) || (x == f.x && z == f.z && y >= f.y && y < up.0)
            })
        };
        let nests = place_nests(5, &trees, bough);
        assert!(nests.len() >= trees.len(), "{} nests in {} trees", nests.len(), trees.len());
        for n in &nests {
            let (x, y, z) = (n.pos.x.floor() as i32, n.pos.y.floor() as i32, n.pos.z.floor() as i32);
            assert!(bough(x, y - 1, z) && !bough(x, y, z), "a nest at {} is not on a bough", n.pos);
        }
        assert_eq!(nests, place_nests(5, &trees, bough));
    }
}
