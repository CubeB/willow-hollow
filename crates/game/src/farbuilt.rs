//! What people have built, seen from beyond the voxel rings: each landmark, bridge,
//! lantern and building the trails stamp into the world, as a few boxes drawn with the
//! far trees (see impostors.rs), so that a tower on a hill or a bridge over a river is
//! still there when the ground it stands on has become the far view.
//!
//! The boxes are made from the stamps' own voxels, gathered into the cells of the
//! coarsest voxel ring, on the same grid. A structure is therefore the same shape on
//! both sides of the ring's edge, and the edge passes across it without a seam: the ring
//! draws what lies inside its circle and the boxes what lies beyond it, pixel by pixel.
//!
//! A whole continent's are made at once, on a thread of their own, as soon as it has been
//! simulated: a continent has a thousand or so stamps, which come to some 7,200 boxes.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        mpsc::{Receiver, Sender, channel},
    },
    time::{Duration, Instant},
};

use glam::{DVec3, IVec3};
use voxel::{AIR, Kind, Materials, Voxel, mesh::CELLS, plant};
use worldgen::{World, macro_layout::MACRO_SIZE, scale::m, trails::Stamp, world::GameWorld};

use crate::{farview, gfx::Gpu, impostors::BoxInstance};

/// The edge of a cell, in voxels: a voxel of the coarsest ring.
pub const CELL: i32 = 1 << (crate::terrain::LEVELS - 1);
const _: () = assert!(CELL == 4, "BUILT_CELL in impostor.wgsl must match");

/// How far from the camera built things are drawn, in voxels, across the ground. A
/// tower on a hill reads against the sky from three kilometres, a little beyond the
/// far trees.
pub const SIGHT: f32 = m(3600.0);
/// They break up over the last of their sight, so that none blinks out at its edge (as
/// `BUILT_FADE` in impostor.wgsl).
pub const FADE: f32 = m(400.0);
const _: () = assert!(FADE == 1000.0, "BUILT_FADE in impostor.wgsl must match");
/// Continents are let go this much further off than they are made, so that moving back
/// and forth across the edge of sight does not make one again and again.
const SLACK: f32 = m(600.0);
/// The squares, in voxels, a continent's boxes are drawn by: those of a square are
/// culled together.
const SQUARE: i32 = 1024;
/// The most boxes kept of one continent. Today's come to some 7,200; with towns there
/// will be more, but nowhere near this.
const MAX_BOXES: usize = 1 << 17;
/// How far a point of light may spread beyond its cell at night, for culling: the
/// couple of pixels it is drawn at least (see impostor.wgsl) at the edge of sight.
const GLOW_SPREAD: i32 = m(16.0) as i32;
/// How often continents in sight not yet simulated are looked for again.
const LOOK_EVERY: Duration = Duration::from_millis(250);

/// Most cells along one edge of a box, so that its size fits a byte of voxels.
const MAX_RUN: i32 = 255 / CELL;

/// How close two materials' colours must be for their cells to be drawn as one box, as
/// the sum of the differences of their channels. The stones of a wall, each a little
/// different, are one grey from where these are seen; a timber door in it is not.
const SAME_COLOUR: i32 = 30;

/// Bits of a built box's flags, the last byte of `impostors::BoxInstance::tree` (see
/// impostor.wgsl).
pub mod flags {
    /// The cell of a lit lantern, a fire or a lit window: faintly lit by itself by day,
    /// and after dark a point of light at least a couple of pixels across, so that it
    /// neither vanishes nor twinkles.
    pub const GLOW: u8 = 1;
    /// Leaves, which snow lies on as it does on a crown.
    pub const FOLIAGE: u8 = 2;
    /// Under something else of the same stamp that keeps the sky off, a stone roof or a
    /// floor, and so in its shade, as the coarse ring has it out where the shadow maps
    /// have ended.
    pub const ROOFED: u8 = 4;
}

/// One box of what people have built, in world voxels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FarBox {
    pub lo: IVec3,
    pub size: IVec3,
    pub color: [u8; 3],
    /// The plant kind of its material (voxel::plant): wood keeps its colour under snow.
    pub plant: u8,
    pub flags: u8,
}

/// The boxes of one stamp, whose continent's origin corner is at world voxel `origin`:
/// its solid cells merged into boxes, then a glow point for each cell anything glows in.
///
/// Each cell is what the coarse ring makes it (see `lod::generate_with`): the last of the
/// stamp's voxels written into it, unless a flame was, and empty if that was air or
/// water. Fine parts are left out of the shape, as there, but a flame among them still
/// glows.
pub fn stamp_boxes(stamp: &Stamp, origin: IVec3, mats: &Materials, out: &mut Vec<FarBox>) {
    let cell_of = |p: IVec3| (p + origin).div_euclid(IVec3::splat(CELL));
    let (lo, hi) = (cell_of(stamp.lo), cell_of(stamp.hi));
    let dims = hi - lo + IVec3::ONE;
    let index = |c: IVec3| {
        let r = c - lo;
        ((r.y * dims.z + r.z) * dims.x + r.x) as usize
    };
    let mut grid = vec![AIR; (dims.x * dims.y * dims.z) as usize];
    let mut glows: Vec<(IVec3, Voxel)> = Vec::new();
    for &(p, v) in &stamp.voxels {
        let c = cell_of(p);
        let cell = &mut grid[index(c)];
        // A flame keeps its cell, as in the coarse ring.
        if !mats.get(*cell).emissive || mats.get(v).emissive {
            *cell = v;
        }
        if mats.get(v).emissive && !glows.iter().any(|g| g.0 == c) {
            glows.push((c, v));
        }
    }
    for b in stamp.fine.iter().filter(|b| mats.get(b.voxel).emissive) {
        let c = cell_of(IVec3::from_array(b.lo).div_euclid(IVec3::splat(CELLS)));
        if !glows.iter().any(|g| g.0 == c) {
            glows.push((c, b.voxel));
        }
    }
    // Which cells have something above them that keeps the sky off, as the voxel mesher
    // judges it for the coarse ring (`roofs` in voxel's mesh.rs): not wood, which snow
    // falls past, nor leaves that are down by the time it does.
    let mut roofed = vec![false; grid.len()];
    for z in 0..dims.z {
        for x in 0..dims.x {
            let mut over = false;
            for y in (0..dims.y).rev() {
                let i = index(lo + IVec3::new(x, y, z));
                roofed[i] = over;
                let m = mats.get(grid[i]);
                over |= m.kind != Kind::Air && !matches!(m.plant, plant::DECIDUOUS | plant::AUTUMN | plant::VINE);
            }
        }
    }
    for v in &mut grid {
        if !mats.get(*v).kind.is_opaque() {
            *v = AIR;
        }
    }
    // Each cell anything glows in is its glow point's alone, by day as well as by night.
    for &(c, _) in &glows {
        grid[index(c)] = AIR;
    }
    merge(&grid, &roofed, dims, mats, |c, size, color, v, under| {
        let m = mats.get(v);
        let foliage = if m.kind == Kind::Foliage { flags::FOLIAGE } else { 0 };
        out.push(FarBox {
            lo: (lo + c) * CELL,
            size: size * CELL,
            color,
            plant: m.plant,
            flags: foliage | if under { flags::ROOFED } else { 0 },
        });
    });
    for (c, v) in glows {
        let under = if roofed[index(c)] { flags::ROOFED } else { 0 };
        out.push(FarBox { lo: c * CELL, size: IVec3::splat(CELL), color: mats.get(v).color, plant: 0, flags: flags::GLOW | under });
    }
}

/// Covers the solid cells of `grid` (y slowest, then z, then x) with boxes, greedily:
/// from each cell not yet covered, as far along x as cells of much the same colour run,
/// then along z by whole rows, then up by whole layers, never joining a cell with
/// something above it (`roofed`) to one without. Each box is given to `emit` as its low
/// cell, its size in cells, the mean colour of its cells, the material of its first and
/// whether it is roofed.
fn merge(grid: &[Voxel], roofed: &[bool], dims: IVec3, mats: &Materials, mut emit: impl FnMut(IVec3, IVec3, [u8; 3], Voxel, bool)) {
    let index = |x: i32, y: i32, z: i32| ((y * dims.z + z) * dims.x + x) as usize;
    let mut used = vec![false; grid.len()];
    for y in 0..dims.y {
        for z in 0..dims.z {
            for x in 0..dims.x {
                let first = grid[index(x, y, z)];
                if first == AIR || used[index(x, y, z)] {
                    continue;
                }
                let m = mats.get(first);
                let under = roofed[index(x, y, z)];
                let like = |i: usize| {
                    let v = grid[i];
                    if v == AIR || used[i] || roofed[i] != under {
                        return false;
                    }
                    let o = mats.get(v);
                    let gap: i32 = (0..3).map(|k| (o.color[k] as i32 - m.color[k] as i32).abs()).sum();
                    o.kind == m.kind && o.plant == m.plant && o.emissive == m.emissive && gap <= SAME_COLOUR
                };
                let mut w = 1;
                while x + w < dims.x && w < MAX_RUN && like(index(x + w, y, z)) {
                    w += 1;
                }
                let mut d = 1;
                while z + d < dims.z && d < MAX_RUN && (0..w).all(|k| like(index(x + k, y, z + d))) {
                    d += 1;
                }
                let mut h = 1;
                while y + h < dims.y && h < MAX_RUN && (0..d).all(|j| (0..w).all(|k| like(index(x + k, y + h, z + j)))) {
                    h += 1;
                }
                let mut sum = [0u32; 3];
                for j in 0..h {
                    for i in 0..d {
                        for k in 0..w {
                            let at = index(x + k, y + j, z + i);
                            used[at] = true;
                            let c = mats.get(grid[at]).color;
                            (0..3).for_each(|n| sum[n] += c[n] as u32);
                        }
                    }
                }
                let n = (w * d * h) as u32;
                emit(IVec3::new(x, y, z), IVec3::new(w, h, d), sum.map(|s| ((s + n / 2) / n) as u8), first, under);
            }
        }
    }
}

/// The world voxel a stamp is filed under for the far boxes: the middle of its bounds,
/// so that each is drawn by one square of land however many it spans.
pub fn stamp_middle(stamp: &Stamp, origin: IVec3) -> IVec3 {
    (stamp.lo + stamp.hi).div_euclid(IVec3::splat(2)) + origin
}

/// The boxes of one square of a continent, which are drawn or culled together.
pub struct Square {
    /// Where they start in the continent's buffer, and how many there are.
    pub first: u32,
    pub count: u32,
    /// Bounds in world voxels, with room for the points of light to spread at night.
    pub lo: IVec3,
    pub hi: IVec3,
}

/// One continent's built things on the GPU.
pub struct Shown {
    pub buffer: wgpu::Buffer,
    pub squares: Vec<Square>,
}

/// What the thread sends back: a continent's boxes, by square.
struct Made {
    cell: (i32, i32),
    boxes: Vec<BoxInstance>,
    squares: Vec<Square>,
}

/// The built things of the continents in sight of the camera.
pub struct FarBuilt {
    world: Arc<GameWorld>,
    requests: Sender<(i32, i32)>,
    results: Receiver<Made>,
    /// The continents made, by macro-cell: None where there is nothing to draw.
    shown: HashMap<(i32, i32), Option<Shown>>,
    /// Asked of the thread and not back yet.
    pending: HashSet<(i32, i32)>,
    /// When continents in sight were last looked for.
    looked: Option<Instant>,
    /// When a continent last arrived.
    progressed: Instant,
    /// Boxes held, for statistics.
    pub boxes: u64,
}

impl FarBuilt {
    pub fn new(world: Arc<GameWorld>) -> Self {
        let (requests, asked) = channel::<(i32, i32)>();
        let (sender, results) = channel();
        let maker = world.clone();
        // One thread of its own: a continent takes a few milliseconds, and is made once.
        std::thread::Builder::new()
            .name("far-built".into())
            .spawn(move || {
                crate::threads::background();
                for cell in asked {
                    if sender.send(make(&maker, cell)).is_err() {
                        break;
                    }
                }
            })
            .expect("far-built thread");
        Self {
            world,
            requests,
            results,
            shown: HashMap::new(),
            pending: HashSet::new(),
            looked: None,
            progressed: Instant::now(),
            boxes: 0,
        }
    }

    pub fn continents(&self) -> impl Iterator<Item = &Shown> {
        self.shown.values().flatten()
    }

    /// A line on how far loading has got, for the log.
    pub fn progress(&self) -> String {
        format!("built things: {} continents, {} boxes, {} being made", self.shown.len(), self.boxes, self.pending.len())
    }

    /// The continents whose land comes within `reach` of the camera, across the ground.
    fn in_sight(camera: DVec3, reach: f32) -> Vec<(i32, i32)> {
        let size = MACRO_SIZE as f64;
        let here = ((camera.x / size).floor() as i32, (camera.z / size).floor() as i32);
        let gap = |p: f64, cell: i32| (cell as f64 * size - p).max(p - (cell as f64 + 1.0) * size).max(0.0);
        let mut out = Vec::new();
        for dz in -1..=1 {
            for dx in -1..=1 {
                let c = (here.0 + dx, here.1 + dz);
                if gap(camera.x, c.0).hypot(gap(camera.z, c.1)) <= reach as f64 {
                    out.push(c);
                }
            }
        }
        out
    }

    /// Call once per frame: lets go of continents out of sight, asks for those come into
    /// it once they have been simulated, and puts up those made.
    pub fn update(&mut self, gpu: &Gpu, camera: DVec3) {
        let keep = Self::in_sight(camera, SIGHT + SLACK);
        let mut dropped = 0;
        self.shown.retain(|c, shown| {
            let kept = keep.contains(c);
            if !kept && let Some(s) = shown {
                dropped += s.squares.iter().map(|q| q.count as u64).sum::<u64>();
            }
            kept
        });
        self.boxes = self.boxes.saturating_sub(dropped);
        if self.looked.is_none_or(|t| t.elapsed() >= LOOK_EVERY) {
            self.looked = Some(Instant::now());
            for c in Self::in_sight(camera, SIGHT) {
                if !self.shown.contains_key(&c) && !self.pending.contains(&c) && self.world.continent_if_ready(c.0, c.1).is_some() {
                    self.pending.insert(c);
                    let _ = self.requests.send(c);
                }
            }
        }
        while let Ok(made) = self.results.try_recv() {
            self.pending.remove(&made.cell);
            self.progressed = Instant::now();
            if !keep.contains(&made.cell) {
                continue;
            }
            let shown = if made.boxes.is_empty() {
                None
            } else {
                // Some hundreds of kilobytes, once for each continent come into sight.
                gpu.try_vertex_buffer("built boxes", bytemuck::cast_slice(&made.boxes)).map(|buffer| Shown { buffer, squares: made.squares })
            };
            if shown.is_some() {
                self.boxes += made.boxes.len() as u64;
            }
            self.shown.insert(made.cell, shown);
        }
    }

    /// Blocks until every continent in sight that has been simulated is shown (for
    /// screenshots). If nothing arrives for [`crate::terrain::wait_limit`], says so and
    /// exits the process with a failure, rather than hang.
    pub fn wait_idle(&mut self, gpu: &Gpu, camera: DVec3) {
        let (began, limit) = (Instant::now(), crate::terrain::wait_limit());
        self.progressed = began;
        loop {
            self.looked = None;
            self.update(gpu, camera);
            let missing = Self::in_sight(camera, SIGHT)
                .into_iter()
                .any(|c| !self.shown.contains_key(&c) && self.world.continent_if_ready(c.0, c.1).is_some());
            if !missing && self.pending.is_empty() {
                break;
            }
            if self.progressed.elapsed() > limit {
                log::error!("hung waiting for the built things: nothing arrived for {:.0} s; {}", self.progressed.elapsed().as_secs_f32(), self.progress());
                std::process::exit(3);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        log::info!("stage: built things loaded after {:.2} s, {}", began.elapsed().as_secs_f32(), self.progress());
    }
}

/// Makes the boxes of every stamp on continent `cell`, which has been simulated, with
/// what lights them where each stamp stands: the skylines, the snow and the fold of the
/// hills.
fn make(world: &GameWorld, cell: (i32, i32)) -> Made {
    let began = Instant::now();
    let continent = world.continent(cell.0, cell.1);
    // Not a pool thread, so it may work them out itself (see `farview::Skylines`).
    let skylines = farview::Skylines::get(&continent);
    let origin = IVec3::new(cell.0 * MACRO_SIZE, 0, cell.1 * MACRO_SIZE);
    let mats = world.materials();
    let mut placed: Vec<((i32, i32), BoxInstance)> = Vec::new();
    let mut boxes = Vec::new();
    for stamp in continent.trails.stamps() {
        boxes.clear();
        stamp_boxes(stamp, origin, mats, &mut boxes);
        if placed.len() + boxes.len() > MAX_BOXES {
            log::warn!("continent {cell:?} has more built boxes than the {MAX_BOXES} kept");
            break;
        }
        let middle = stamp_middle(stamp, IVec3::ZERO);
        let (lx, lz) = (middle.x as f32 + 0.5, middle.z as f32 + 0.5);
        // The ground where it stands, as the voxel terrain has it there.
        let (x, z) = (middle.x + origin.x, middle.z + origin.z);
        let biome = farview::biome_at(world, x, z);
        let [rise, set] = skylines.blended(lx, lz);
        let hollow = farview::hollow_at(world, x, z);
        let square = (middle.x.div_euclid(SQUARE), middle.z.div_euclid(SQUARE));
        for b in &boxes {
            placed.push((
                square,
                BoxInstance {
                    lo: b.lo.to_array(),
                    size: b.size.min(IVec3::splat(255)).as_uvec3().to_array().map(|s| s as u8),
                    biome,
                    color: b.color,
                    plant: b.plant,
                    // Where a tree's carry its tag, these carry how deep in a fold of
                    // the hills they stand (farview::hollow_at).
                    tree: [hollow, rise, set, b.flags],
                },
            ));
        }
    }
    // A stable sort, so each square's boxes keep the order they were laid in.
    placed.sort_by_key(|(square, _)| *square);
    let mut out = Made { cell, boxes: Vec::with_capacity(placed.len()), squares: Vec::new() };
    for run in placed.chunk_by(|a, b| a.0 == b.0) {
        let first = out.boxes.len() as u32;
        let (mut lo, mut hi) = (IVec3::MAX, IVec3::MIN);
        for (_, b) in run {
            let at = IVec3::from_array(b.lo);
            let spread = if b.tree[3] & flags::GLOW != 0 { GLOW_SPREAD } else { 0 };
            lo = lo.min(at - IVec3::new(spread, 0, spread));
            hi = hi.max(at + IVec3::new(b.size[0] as i32, b.size[1] as i32, b.size[2] as i32) + IVec3::splat(spread));
            out.boxes.push(*b);
        }
        out.squares.push(Square { first, count: run.len() as u32, lo, hi });
    }
    log::info!(
        "built things of continent {cell:?}: {} boxes in {} squares, made in {:.1} ms",
        out.boxes.len(),
        out.squares.len(),
        began.elapsed().as_secs_f32() * 1000.0
    );
    out
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use worldgen::{World, world::GameWorld};

    /// Every solid cell of a stamp is covered by exactly one box, and no box covers an
    /// empty cell but for a flame's: the boxes are the shape the coarse ring draws.
    #[test]
    fn boxes_cover_the_cells_of_each_stamp_once() {
        let world = GameWorld::new(1);
        let mats = world.materials();
        let continent = world.continent(0, 0);
        // Not at the world's origin, so the cells are those of world positions.
        let origin = IVec3::new(65536, 0, -65536);
        let (mut stamps, mut cell_count, mut boxes_made, mut glows) = (0, 0, 0, 0);
        let mut by_square: HashMap<(i32, i32), usize> = HashMap::new();
        for stamp in continent.trails.stamps() {
            let mut cells: HashMap<IVec3, Voxel> = HashMap::new();
            for &(p, v) in &stamp.voxels {
                let old = cells.entry((p + origin).div_euclid(IVec3::splat(CELL))).or_insert(v);
                if !mats.get(*old).emissive || mats.get(v).emissive {
                    *old = v;
                }
            }
            cells.retain(|_, v| mats.get(*v).kind.is_opaque());
            let mut boxes = Vec::new();
            stamp_boxes(stamp, origin, mats, &mut boxes);
            let mut covered: HashMap<IVec3, u32> = HashMap::new();
            for b in &boxes {
                assert_eq!(b.lo.rem_euclid(IVec3::splat(CELL)), IVec3::ZERO);
                assert_eq!(b.size % CELL, IVec3::ZERO);
                assert!(b.size.min_element() > 0 && b.size.max_element() <= 255);
                if b.flags & flags::GLOW != 0 {
                    glows += 1;
                    assert_eq!(b.size, IVec3::splat(CELL));
                    // A flame among the fine parts may glow in a cell otherwise empty.
                    cells.entry(b.lo.div_euclid(IVec3::splat(CELL))).or_insert(AIR);
                }
                for x in 0..b.size.x / CELL {
                    for y in 0..b.size.y / CELL {
                        for z in 0..b.size.z / CELL {
                            *covered.entry(b.lo.div_euclid(IVec3::splat(CELL)) + IVec3::new(x, y, z)).or_default() += 1;
                        }
                    }
                }
            }
            assert!(covered.values().all(|&n| n == 1), "a cell covered twice");
            assert_eq!(covered.len(), cells.len(), "boxes cover {} cells of {}", covered.len(), cells.len());
            assert!(cells.keys().all(|c| covered.contains_key(c)));
            let m = stamp_middle(stamp, origin);
            *by_square.entry((m.x.div_euclid(256), m.z.div_euclid(256))).or_default() += boxes.len();
            stamps += 1;
            cell_count += cells.len();
            boxes_made += boxes.len();
        }
        let busiest = by_square.values().max().copied().unwrap_or(0);
        println!("{stamps} stamps, {cell_count} cells, {boxes_made} boxes ({glows} glowing); at most {busiest} in a square of 256");
        assert!(stamps > 100 && boxes_made < cell_count && glows > 0);
    }

    /// The boxes stand in the very cells the coarsest ring fills with the same stamps,
    /// so that its edge passes across a structure without a step: every cell of a box
    /// over a lookout is solid in the ring's own chunks.
    #[test]
    fn boxes_fill_the_cells_the_coarse_ring_does() {
        use voxel::CHUNK;
        use worldgen::features::landmarks::Landmark;
        let world = GameWorld::new(1);
        let continent = world.continent(0, 0);
        let models = crate::lod::Models::default();
        let level = (crate::terrain::LEVELS - 1) as u8;
        let (mut cells, mut missing) = (0, 0);
        for site in continent.trails.sites.iter().filter(|s| s.kind == Landmark::Lookout).take(3) {
            let mut boxes = Vec::new();
            for stamp in continent.trails.stamps().filter(|s| (s.lo.x..=s.hi.x).contains(&site.x) && (s.lo.z..=s.hi.z).contains(&site.z)) {
                stamp_boxes(stamp, IVec3::ZERO, world.materials(), &mut boxes);
            }
            let mut columns: HashMap<(i32, i32), Vec<(i32, voxel::PaddedChunk)>> = HashMap::new();
            for b in boxes.iter().filter(|b| b.flags & flags::GLOW == 0) {
                for x in 0..b.size.x / CELL {
                    for y in 0..b.size.y / CELL {
                        for z in 0..b.size.z / CELL {
                            let c = b.lo / CELL + IVec3::new(x, y, z);
                            let (cx, cz) = (c.x.div_euclid(CHUNK), c.z.div_euclid(CHUNK));
                            let chunks = columns.entry((cx, cz)).or_insert_with(|| crate::lod::generate(&world, &models, level, cx, cz).0);
                            let cy = c.y.div_euclid(CHUNK);
                            let local = c - IVec3::new(cx, cy, cz) * CHUNK + IVec3::ONE;
                            let solid = chunks
                                .iter()
                                .find(|(y, _)| *y == cy)
                                .is_some_and(|(_, chunk)| chunk.get(local.x as usize, local.y as usize, local.z as usize) != AIR);
                            cells += 1;
                            missing += !solid as usize;
                        }
                    }
                }
            }
        }
        println!("{missing} of {cells} cells of the boxes empty in the coarse ring");
        assert!(cells > 100 && missing * 50 <= cells, "{missing} of {cells}");
    }

    /// Prints, for each lookout and the widest bridge of world 1, where to stand 0.5, 1.5
    /// and 3 km off to see it without a hill in the way, for screenshots. Run with
    /// `cargo test -p game views_of_the_lookouts -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn views_of_the_lookouts() {
        use worldgen::features::landmarks::Landmark;
        let world = GameWorld::new(1);
        let continent = world.continent(0, 0);
        let mut places: Vec<(i32, i32, i32)> = continent.trails.sites.iter().filter(|s| s.kind == Landmark::Lookout).map(|s| (s.x, s.z, s.y)).collect();
        if let Some(c) = continent.trails.crossings.iter().max_by_key(|c| c.span) {
            places.push((c.x, c.z, world.column(c.x, c.z).top()));
        }
        for (sx, sz, sy) in places {
            let top = continent
                .trails
                .stamps()
                .filter(|s| (s.lo.x..=s.hi.x).contains(&sx) && (s.lo.z..=s.hi.z).contains(&sz))
                .map(|s| s.hi.y)
                .max()
                .unwrap_or(sy);
            println!("{sx},{sz} top {top} in {:?}", world.column(sx, sz).biome);
            let site = glam::IVec2::new(sx, sz);
            let target = glam::Vec3::new(site.x as f32, top as f32, site.y as f32);
            for metres in [500.0, 1500.0, 3000.0] {
                let mut best: Option<(f32, f32, glam::Vec3)> = None;
                for k in 0..24 {
                    let yaw = k as f32 * std::f32::consts::TAU / 24.0;
                    let dir = glam::Vec2::new(yaw.cos(), yaw.sin());
                    let at = glam::Vec2::new(target.x, target.z) - dir * m(metres);
                    let eye = world.column(at.x as i32, at.y as i32).top() as f32 + m(40.0);
                    let from = glam::Vec3::new(at.x, eye, at.y);
                    // The least height the line of sight clears the ground by, past the
                    // first hundred metres.
                    let clear = (8..100)
                        .map(|i| {
                            let p = from.lerp(target, i as f32 / 100.0);
                            p.y - world.column(p.x as i32, p.z as i32).top() as f32
                        })
                        .fold(f32::MAX, f32::min);
                    if best.is_none_or(|b| clear > b.0) {
                        best = Some((clear, yaw, from));
                    }
                }
                let (clear, yaw, from) = best.unwrap();
                let pitch = ((target.y - from.y) / m(metres)).atan().to_degrees();
                println!(
                    "  {metres} m from {:.0},{:.0},{:.0} look {:.0},{:.1} clears by {clear:.0}",
                    from.x,
                    from.z,
                    m(40.0),
                    yaw.to_degrees(),
                    pitch
                );
            }
        }
    }
}
