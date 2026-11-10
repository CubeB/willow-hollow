//! Growing on surfaces: fine pieces attached to any face of the voxels that features
//! and landmarks have already stamped, rather than standing on top of a column.
//!
//! This is the shared layer under moss, ivy, vines and bark. It has three parts:
//!
//! - [`Voxels`]: read access to the finished world. [`ColumnVoxels`] covers one generated
//!   chunk column and its one-voxel border, which is all that anything deciding one face
//!   at a time needs.
//! - [`for_each_site`]: visits every air voxel in a chunk column that touches something
//!   solid, once per face touched, with what is known about that spot: the [`Site`].
//! - [`Raster`] and [`FacePlot`]: lay pieces out on a face in that face's own terms, then
//!   turn the lot into merged, non-overlapping [`PlantBox`]es.
//!
//! **Agreeing across chunk borders.** A piece lives in the air voxel it occupies, so each
//! piece is decided by exactly one chunk column, the one that owns that air voxel. The
//! face it grows on may belong to a neighbour, but that neighbour's voxels are in the
//! border. Moss needs nothing more. A plant spanning many columns (ivy) is grown from
//! its root in one run and cut to each column with [`Raster::keep_column`].
//!
//! To add a new surface growth (#59 bark, say): call `for_each_site`, decide from the
//! [`Site`], plot boxes through [`Site::plot`], and finish with [`Raster::finish`].

use std::collections::HashMap;

use glam::IVec3;
use voxel::{
    AIR, CHUNK, Kind, Materials, PAD, PaddedChunk, Voxel,
    mesh::{CELLS, PlantBox},
};

/// Read access to generated voxels in world coordinates.
pub trait Voxels {
    fn voxel(&self, p: IVec3) -> Voxel;
}

/// The direction from an air voxel to the voxel it grows on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Toward {
    /// The host is underneath: growth on its top face.
    Below,
    /// The host is overhead: growth on its underside.
    Above,
    PosX,
    NegX,
    PosZ,
    NegZ,
}

impl Toward {
    pub const ALL: [Toward; 6] = [Toward::Below, Toward::Above, Toward::PosX, Toward::NegX, Toward::PosZ, Toward::NegZ];

    pub fn step(self) -> IVec3 {
        match self {
            Toward::Below => IVec3::NEG_Y,
            Toward::Above => IVec3::Y,
            Toward::PosX => IVec3::X,
            Toward::NegX => IVec3::NEG_X,
            Toward::PosZ => IVec3::Z,
            Toward::NegZ => IVec3::NEG_Z,
        }
    }

    /// Whether the face is a wall, as opposed to a floor or ceiling.
    pub fn is_side(self) -> bool {
        !matches!(self, Toward::Below | Toward::Above)
    }
}

/// One generated chunk column plus its one-voxel border, in world coordinates.
pub struct ColumnVoxels<'a> {
    chunks: &'a [(i32, PaddedChunk)],
    cy_min: i32,
    /// World position of padded (0, 0) horizontally.
    x0: i32,
    z0: i32,
}

impl<'a> ColumnVoxels<'a> {
    /// `chunks` are lowest first with none skipped, and `x0`, `z0` the world position of
    /// the border's corner.
    pub fn new(chunks: &'a [(i32, PaddedChunk)], x0: i32, z0: i32) -> Self {
        let cy_min = chunks.first().map_or(0, |c| c.0);
        Self { chunks, cy_min, x0, z0 }
    }

    /// Whether world voxel height `y` has a chunk here, so a box starting there can be
    /// drawn.
    pub fn has_height(&self, y: i32) -> bool {
        let cy = y.div_euclid(CHUNK);
        cy >= self.cy_min && ((cy - self.cy_min) as usize) < self.chunks.len()
    }
}

impl Voxels for ColumnVoxels<'_> {
    fn voxel(&self, p: IVec3) -> Voxel {
        let (px, pz) = (p.x - self.x0, p.z - self.z0);
        debug_assert!((0..PAD as i32).contains(&px) && (0..PAD as i32).contains(&pz), "{p} is outside this column");
        let cy = p.y.div_euclid(CHUNK);
        if cy < self.cy_min {
            // Chunks start under the lowest ground: below them is solid rock.
            return crate::materials::STONE;
        }
        match self.chunks.get((cy - self.cy_min) as usize) {
            Some((_, chunk)) => chunk.get(px as usize, (p.y.rem_euclid(CHUNK) + 1) as usize, pz as usize),
            None => AIR,
        }
    }
}

/// An air voxel touching a solid voxel, and what is known about the spot.
#[derive(Clone, Copy, Debug)]
pub struct Site {
    /// The air voxel the growth occupies.
    pub air: IVec3,
    /// Where the host is, from `air`.
    pub toward: Toward,
    /// The voxel grown on.
    pub host: Voxel,
    /// Opaque voxels overhead in this column: 0 is open sky. Stands in for shade, since
    /// no light data exists during generation.
    pub canopy: u32,
}

impl Site {
    /// World cell of the air voxel's corner.
    pub fn origin(&self) -> IVec3 {
        self.air * CELLS
    }

    /// A plot for laying out pieces on this face.
    pub fn plot<'a>(&self, raster: &'a mut Raster) -> FacePlot<'a> {
        FacePlot { origin: self.origin(), toward: self.toward, raster }
    }
}

/// Bits of one chunk's interior heights in a column mask.
const INTERIOR: u64 = ((1u64 << CHUNK) - 1) << 1;

/// Visits every site of a chunk column, in a fixed order: by column, then by height and
/// face. `host_ok` says which voxels can carry growth at all, and is the cheap way to
/// skip a column's worth of things that can't.
pub fn for_each_site(voxels: &ColumnVoxels, mats: &Materials, host_ok: impl Fn(Voxel) -> bool, mut visit: impl FnMut(&Site)) {
    let (solid_bit, opaque_bit): (Vec<bool>, Vec<bool>) = (0..mats.len())
        .map(|i| {
            let kind = mats.get(i as Voxel).kind;
            (kind == Kind::Solid, kind.is_opaque())
        })
        .unzip();
    let chunks = voxels.chunks;
    // Solid (host) and opaque (shade) bits per chunk and column, borders included.
    let masks: Vec<[[(u64, u64, u64); PAD]; PAD]> = chunks
        .iter()
        .map(|(_, chunk)| {
            let mut m = [[(0u64, 0u64, 0u64); PAD]; PAD];
            for (x, row) in m.iter_mut().enumerate() {
                for (z, cell) in row.iter_mut().enumerate() {
                    for y in 0..PAD {
                        let v = chunk.get(x, y, z);
                        if solid_bit[v as usize] {
                            cell.0 |= 1 << y;
                        }
                        if opaque_bit[v as usize] {
                            cell.1 |= 1 << y;
                        }
                        if v == AIR {
                            cell.2 |= 1 << y;
                        }
                    }
                }
            }
            m
        })
        .collect();
    let n = CHUNK as usize;
    for x in 1..=n {
        for z in 1..=n {
            // Opaque voxels in the chunks above the one being scanned.
            let mut above = 0u32;
            for c in (0..chunks.len()).rev() {
                let (cy, chunk) = &chunks[c];
                let m = &masks[c];
                let air = m[x][z].2 & INTERIOR;
                if air != 0 {
                    let solid = m[x][z].0;
                    let around = (solid << 1) | (solid >> 1) | m[x + 1][z].0 | m[x - 1][z].0 | m[x][z + 1].0 | m[x][z - 1].0;
                    let mut bits = air & around;
                    while bits != 0 {
                        let y = bits.trailing_zeros() as usize;
                        bits &= bits - 1;
                        // Opaque voxels higher up this same chunk, interior only.
                        let over = m[x][z].1 & INTERIOR & !((2u64 << y) - 1);
                        let canopy = above + over.count_ones();
                        let air_pos = IVec3::new(voxels.x0 + x as i32, cy * CHUNK + y as i32 - 1, voxels.z0 + z as i32);
                        for toward in Toward::ALL {
                            let step = toward.step();
                            let (hx, hy, hz) = (
                                (x as i32 + step.x) as usize,
                                (y as i32 + step.y) as usize,
                                (z as i32 + step.z) as usize,
                            );
                            let host = chunk.get(hx, hy, hz);
                            if solid_bit[host as usize] && host_ok(host) {
                                visit(&Site { air: air_pos, toward, host, canopy });
                            }
                        }
                    }
                }
                above += (m[x][z].1 & INTERIOR).count_ones();
            }
        }
    }
}

/// Lays out boxes on one face of a [`Site`], in the face's own coordinates: `u` and `v`
/// run across the face and `d` straight out from it, each in cells from 0. On a wall `v`
/// runs up the wall. All of it stays inside the air voxel.
pub struct FacePlot<'a> {
    origin: IVec3,
    toward: Toward,
    raster: &'a mut Raster,
}

impl FacePlot<'_> {
    /// Fills the cells `u0..u1`, `v0..v1`, `d0..d1`, leaving any already taken. Anything
    /// outside the voxel is ignored. Swaying is decided when the raster is finished.
    pub fn fill(&mut self, u0: i32, u1: i32, v0: i32, v1: i32, d0: i32, d1: i32, voxel: Voxel) {
        for d in d0.max(0)..d1.min(CELLS) {
            for v in v0.max(0)..v1.min(CELLS) {
                for u in u0.max(0)..u1.min(CELLS) {
                    let c = match self.toward {
                        Toward::Below => [u, d, v],
                        Toward::Above => [u, CELLS - 1 - d, v],
                        Toward::PosX => [CELLS - 1 - d, v, u],
                        Toward::NegX => [d, v, u],
                        Toward::PosZ => [u, v, CELLS - 1 - d],
                        Toward::NegZ => [u, v, d],
                    };
                    self.raster.set(self.origin + IVec3::from_array(c), voxel);
                }
            }
        }
    }
}

/// Cells of a voxel, each holding the plant material there, or air.
type Grid = [Voxel; (CELLS * CELLS * CELLS) as usize];

fn grid_index(p: IVec3) -> usize {
    (p.x + p.z * CELLS + p.y * CELLS * CELLS) as usize
}

/// A set of plant cells in world cell coordinates, turned into merged boxes at the end.
/// Whatever writes a cell first keeps it, so overlapping growths share space cleanly,
/// and the result never has two boxes in the same place.
#[derive(Default)]
pub struct Raster {
    grids: HashMap<IVec3, Grid>,
}

impl Raster {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.grids.is_empty()
    }

    /// Claims a cell for `voxel`, unless it already holds something. Returns whether it
    /// was free.
    pub fn set(&mut self, cell: IVec3, voxel: Voxel) -> bool {
        let at = cell.div_euclid(IVec3::splat(CELLS));
        let local = cell - at * CELLS;
        let grid = self.grids.entry(at).or_insert([AIR; (CELLS * CELLS * CELLS) as usize]);
        let slot = &mut grid[grid_index(local)];
        if *slot == AIR {
            *slot = voxel;
            true
        } else {
            false
        }
    }

    /// Whether a cell holds a plant.
    pub fn taken(&self, cell: IVec3) -> bool {
        let at = cell.div_euclid(IVec3::splat(CELLS));
        self.grids.get(&at).is_some_and(|g| g[grid_index(cell - at * CELLS)] != AIR)
    }

    /// Air voxels (in world voxel coordinates) holding any plant cell.
    pub fn voxels(&self) -> impl Iterator<Item = IVec3> + '_ {
        self.grids.keys().copied()
    }

    /// Keeps only the chunk column at `(cx, cz)` of the heights a chunk exists for: how a
    /// plant grown once for many columns is clipped to each.
    pub fn keep_column(&mut self, cx: i32, cz: i32) {
        self.grids.retain(|at, _| at.x.div_euclid(CHUNK) == cx && at.z.div_euclid(CHUNK) == cz);
    }

    /// Drops every cell in voxels `keep` rejects.
    pub fn retain_voxels(&mut self, keep: impl Fn(IVec3) -> bool) {
        self.grids.retain(|at, _| keep(*at));
    }

    /// Turns the cells into merged boxes, appended to `out`. `sway` gives the wind flags
    /// for a box's material: (bottom moves, top moves).
    pub fn finish(self, out: &mut Vec<PlantBox>, sway: impl Fn(Voxel) -> (bool, bool)) {
        let mut boxes = Vec::new();
        let mut keys: Vec<IVec3> = self.grids.keys().copied().collect();
        keys.sort_unstable_by_key(|p| (p.y, p.x, p.z));
        for at in keys {
            let mut grid = self.grids[&at];
            for y in 0..CELLS {
                for z in 0..CELLS {
                    for x in 0..CELLS {
                        let voxel = grid[grid_index(IVec3::new(x, y, z))];
                        if voxel == AIR {
                            continue;
                        }
                        let (mut w, mut d, mut h) = (1, 1, 1);
                        let same = |g: &Grid, p: IVec3| g[grid_index(p)] == voxel;
                        while x + w < CELLS && same(&grid, IVec3::new(x + w, y, z)) {
                            w += 1;
                        }
                        'depth: while z + d < CELLS {
                            for ix in x..x + w {
                                if !same(&grid, IVec3::new(ix, y, z + d)) {
                                    break 'depth;
                                }
                            }
                            d += 1;
                        }
                        'height: while y + h < CELLS {
                            for iz in z..z + d {
                                for ix in x..x + w {
                                    if !same(&grid, IVec3::new(ix, y + h, iz)) {
                                        break 'height;
                                    }
                                }
                            }
                            h += 1;
                        }
                        for iy in y..y + h {
                            for iz in z..z + d {
                                for ix in x..x + w {
                                    grid[grid_index(IVec3::new(ix, iy, iz))] = AIR;
                                }
                            }
                        }
                        let lo = at * CELLS + IVec3::new(x, y, z);
                        let hi = lo + IVec3::new(w, h, d);
                        let (sway_lo, sway_hi) = sway(voxel);
                        boxes.push(PlantBox { lo: lo.to_array(), hi: hi.to_array(), voxel, sway_lo, sway_hi });
                    }
                }
            }
        }
        merge_boxes(&mut boxes);
        out.append(&mut boxes);
    }
}

/// Coalesces boxes with the same cross-section and material that touch end to end,
/// within one chunk, so a patch costs a few large boxes and not one per voxel.
pub fn merge_boxes(boxes: &mut Vec<PlantBox>) {
    let chunk_of = |c: i32| c.div_euclid(CHUNK * CELLS);
    for axis in [0, 2, 1] {
        let (a1, a2) = ((axis + 1) % 3, (axis + 2) % 3);
        boxes.sort_unstable_by_key(|b| (b.lo[a1], b.lo[a2], b.hi[a1], b.hi[a2], b.voxel, b.sway_lo, b.sway_hi, b.lo[axis]));
        let mut merged: Vec<PlantBox> = Vec::with_capacity(boxes.len());
        for &b in boxes.iter() {
            if let Some(last) = merged.last_mut()
                && last.lo[a1] == b.lo[a1]
                && last.lo[a2] == b.lo[a2]
                && last.hi[a1] == b.hi[a1]
                && last.hi[a2] == b.hi[a2]
                && last.voxel == b.voxel
                && last.sway_lo == b.sway_lo
                && last.sway_hi == b.sway_hi
                && last.hi[axis] == b.lo[axis]
                && chunk_of(last.lo[axis]) == chunk_of(b.hi[axis] - 1)
            {
                last.hi[axis] = b.hi[axis];
            } else {
                merged.push(b);
            }
        }
        *boxes = merged;
    }
}

#[cfg(test)]
pub mod tests_support {
    //! A small hand-built world to grow things on.

    use std::collections::HashMap;

    use glam::IVec3;
    use voxel::Voxel;

    use super::Voxels;

    /// Voxels in a map, optionally on a flat floor: everything below height `floor` is
    /// `floor_voxel`.
    pub struct Blocks {
        pub map: HashMap<IVec3, Voxel>,
        pub floor: i32,
        pub floor_voxel: Voxel,
    }

    impl Voxels for Blocks {
        fn voxel(&self, p: IVec3) -> Voxel {
            if let Some(v) = self.map.get(&p) {
                return *v;
            }
            if p.y < self.floor { self.floor_voxel } else { voxel::AIR }
        }
    }
}

#[cfg(test)]
mod tests {
    use voxel::Material;

    use super::*;

    fn palette() -> (Materials, Voxel, Voxel) {
        let mut m = Materials::new();
        let stone = m.add(Material { color: [120, 120, 120], kind: Kind::Solid, jitter: 0, emissive: false });
        let leaf = m.add(Material { color: [40, 120, 40], kind: Kind::Foliage, jitter: 0, emissive: false });
        (m, stone, leaf)
    }

    fn column_of(voxels: &[(IVec3, Voxel)]) -> Vec<(i32, PaddedChunk)> {
        let mut chunk = PaddedChunk::new();
        for &(p, v) in voxels {
            chunk.set(p.x as usize + 1, p.y as usize + 1, p.z as usize + 1, v);
        }
        vec![(0, chunk)]
    }

    #[test]
    fn a_lone_block_has_six_faces_of_sites_and_nothing_else() {
        let (mats, stone, _) = palette();
        let chunks = column_of(&[(IVec3::new(5, 5, 5), stone)]);
        let view = ColumnVoxels::new(&chunks, -1, -1);
        let mut sites = Vec::new();
        for_each_site(&view, &mats, |_| true, |s| sites.push(*s));
        assert_eq!(sites.len(), 6);
        for s in &sites {
            assert_eq!(s.air + s.toward.step(), IVec3::new(5, 5, 5));
            assert_eq!(s.host, stone);
        }
    }

    #[test]
    fn leaves_are_no_host_but_do_shade() {
        let (mats, stone, leaf) = palette();
        let chunks = column_of(&[(IVec3::new(5, 5, 5), stone), (IVec3::new(5, 10, 5), leaf), (IVec3::new(5, 11, 5), leaf)]);
        let view = ColumnVoxels::new(&chunks, -1, -1);
        let mut sites = Vec::new();
        for_each_site(&view, &mats, |_| true, |s| sites.push(*s));
        assert!(sites.iter().all(|s| s.host == stone), "only solid voxels are hosts");
        let top = sites.iter().find(|s| s.toward == Toward::Below).expect("site on top of the block");
        assert_eq!(top.canopy, 2, "two leaves overhead");
        let side = sites.iter().find(|s| s.toward == Toward::PosX).expect("site on the side");
        assert_eq!(side.air.x, 4);
        assert_eq!(side.canopy, 0, "the air beside the block has nothing overhead");
    }

    #[test]
    fn plots_stay_on_their_face_and_inside_their_voxel() {
        let air = IVec3::new(2, 3, 4);
        for toward in Toward::ALL {
            let site = Site { air, toward, host: 1, canopy: 0 };
            let mut raster = Raster::new();
            site.plot(&mut raster).fill(0, 9, 0, 9, 0, 1, 7);
            let mut boxes = Vec::new();
            raster.finish(&mut boxes, |_| (false, false));
            assert_eq!(boxes.len(), 1, "{toward:?}: a whole face is one box");
            let b = boxes[0];
            // Full face, one cell thick, flush against the host.
            let dims: Vec<i32> = (0..3).map(|i| b.hi[i] - b.lo[i]).collect();
            assert_eq!(dims.iter().filter(|&&d| d == 1).count(), 1, "{toward:?}: {b:?}");
            for axis in 0..3 {
                let cell = air[axis] * CELLS;
                assert!(b.lo[axis] >= cell && b.hi[axis] <= cell + CELLS, "{toward:?}: {b:?}");
                let host_side = toward.step()[axis];
                if host_side > 0 {
                    assert_eq!(b.hi[axis], cell + CELLS, "{toward:?}: flush against the host");
                } else if host_side < 0 {
                    assert_eq!(b.lo[axis], cell, "{toward:?}: flush against the host");
                }
            }
        }
    }

    #[test]
    fn first_writer_keeps_a_cell_and_overlaps_are_impossible() {
        let mut raster = Raster::new();
        assert!(raster.set(IVec3::new(1, 2, 3), 5));
        assert!(!raster.set(IVec3::new(1, 2, 3), 6), "the cell is taken");
        raster.set(IVec3::new(2, 2, 3), 5);
        let mut boxes = Vec::new();
        raster.finish(&mut boxes, |_| (false, false));
        assert_eq!(boxes.len(), 1);
        assert_eq!(boxes[0].voxel, 5);
    }

    #[test]
    fn adjacent_faces_merge_but_stop_at_a_chunk_border() {
        let mut raster = Raster::new();
        // A full-height plate on a wall spanning two chunks vertically.
        let top = CHUNK * CELLS;
        for y in top - 8..top + 8 {
            for x in 0..4 {
                raster.set(IVec3::new(x, y, 0), 3);
            }
        }
        let mut boxes = Vec::new();
        raster.finish(&mut boxes, |_| (false, false));
        assert_eq!(boxes.len(), 2, "one box per chunk: {boxes:?}");
        assert!(boxes.iter().all(|b| (b.lo[1]).div_euclid(top) == (b.hi[1] - 1).div_euclid(top)));
    }

    #[test]
    fn keep_column_clips_to_one_chunk_column() {
        let mut raster = Raster::new();
        raster.set(IVec3::new(0, 0, 0), 1);
        raster.set(IVec3::new(CHUNK * CELLS, 0, 0), 2);
        raster.keep_column(1, 0);
        let mut boxes = Vec::new();
        raster.finish(&mut boxes, |_| (false, false));
        assert_eq!(boxes.len(), 1);
        assert_eq!(boxes[0].voxel, 2);
    }

    #[test]
    fn nothing_above_or_below_the_chunks_breaks_a_read() {
        let (_, stone, _) = palette();
        let chunks = column_of(&[(IVec3::new(5, 5, 5), stone)]);
        let view = ColumnVoxels::new(&chunks, -1, -1);
        assert_eq!(view.voxel(IVec3::new(5, 5, 5)), stone);
        assert_eq!(view.voxel(IVec3::new(5, 200, 5)), AIR);
        assert_ne!(view.voxel(IVec3::new(5, -200, 5)), AIR, "underground is rock");
        assert!(view.has_height(31) && !view.has_height(32) && !view.has_height(-1));
    }
}
