//! The ground a herd walks on, read from the voxels.
//!
//! For each voxel column of a herd's range: the voxel its feet stand in, how deep any
//! water is over it, and how much room there is above before anything solid or leafy.
//! A tree's trunk, a wall or a boulder stands up out of the ground as a column whose
//! floor is far above its neighbours', so a beast that only steps up as high as it can
//! climb never walks through one; a bush is a column with no room above it. Read once
//! when a herd is stood up, on a thread of its own, and kept as four bytes a column.

use glam::DVec2;
use voxel::{CHUNK, Kind, PAD, PaddedChunk};
use worldgen::{World, world::GameWorld};

/// One voxel column of a herd's ground.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    /// The voxel the feet stand in: the first above the ground, or above the bed under
    /// water.
    pub floor: i16,
    /// How deep the water stands over the floor, in voxels.
    pub water: u8,
    /// How many voxels of air or water there are from the floor up before anything
    /// solid or leafy, at most [`ROOM`].
    pub room: u8,
}

/// The most headroom a cell records: more than the tallest beast needs.
pub const ROOM: u8 = 12;

impl Cell {
    pub const fn open(floor: i16) -> Cell {
        Cell { floor, water: 0, room: ROOM }
    }

    /// Where a swimmer floats: the top of the water.
    pub fn surface(self) -> i32 {
        self.floor as i32 + self.water as i32
    }
}

/// The ground of a square of columns.
pub struct Ground {
    /// The square's corner, a world voxel column, and its width each way.
    pub x0: i32,
    pub z0: i32,
    pub w: i32,
    pub d: i32,
    cells: Vec<Cell>,
}

impl Ground {
    /// Ground made by `f` for each column, for tests.
    #[cfg(test)]
    pub fn from_fn(x0: i32, z0: i32, w: i32, d: i32, f: impl Fn(i32, i32) -> Cell) -> Self {
        let cells = (0..w * d).map(|i| f(x0 + i % w, z0 + i / w)).collect();
        Ground { x0, z0, w, d, cells }
    }

    pub fn cell(&self, x: i32, z: i32) -> Option<Cell> {
        let (i, j) = (x - self.x0, z - self.z0);
        ((0..self.w).contains(&i) && (0..self.d).contains(&j)).then(|| self.cells[(j * self.w + i) as usize])
    }

    /// The cell a point is over.
    pub fn at(&self, p: DVec2) -> Option<Cell> {
        self.cell(p.x.floor() as i32, p.y.floor() as i32)
    }

    pub fn middle(&self) -> DVec2 {
        DVec2::new(self.x0 as f64 + self.w as f64 * 0.5, self.z0 as f64 + self.d as f64 * 0.5)
    }

    /// Reads the square from `(x0, z0)`, `w` by `d` columns, from the voxels of the
    /// world: every chunk column it reaches is generated (without its small plants),
    /// read, and let go. A few hundred milliseconds for a herd's range: call it off the
    /// main thread.
    pub fn read(world: &GameWorld, x0: i32, z0: i32, w: i32, d: i32) -> Self {
        let mut cells = vec![Cell::open(0); (w * d) as usize];
        let mats = world.materials();
        for cz in z0.div_euclid(CHUNK)..=(z0 + d - 1).div_euclid(CHUNK) {
            for cx in x0.div_euclid(CHUNK)..=(x0 + w - 1).div_euclid(CHUNK) {
                let out = world.generate_column_bare(cx, cz);
                let chunks = &out.chunks;
                for lz in 0..CHUNK {
                    for lx in 0..CHUNK {
                        let (x, z) = (cx * CHUNK + lx, cz * CHUNK + lz);
                        let (i, j) = (x - x0, z - z0);
                        if !(0..w).contains(&i) || !(0..d).contains(&j) {
                            continue;
                        }
                        // Where the land's surface is, from the column as it was made; what
                        // stands on it or was cut into it is found from there.
                        let top = out.wash[((lx + 1) as usize) * PAD + (lz + 1) as usize].0;
                        let kind = |y: i32| mats.get(voxel_in(chunks, x, y, z)).kind;
                        cells[(j * w + i) as usize] = scan(&kind, top + 1);
                    }
                }
            }
        }
        Ground { x0, z0, w, d, cells }
    }
}

/// What is at height `y` of a column's chunks: rock below them, air above.
fn voxel_in(chunks: &[(i32, PaddedChunk)], x: i32, y: i32, z: i32) -> voxel::Voxel {
    let cy = y.div_euclid(CHUNK);
    match chunks.iter().find(|(c, _)| *c == cy) {
        Some((_, chunk)) => chunk.get(x.rem_euclid(CHUNK) as usize + 1, y.rem_euclid(CHUNK) as usize + 1, z.rem_euclid(CHUNK) as usize + 1),
        None if chunks.first().is_some_and(|(lowest, _)| cy < *lowest) => worldgen::materials::STONE,
        None => voxel::AIR,
    }
}

/// The cell of a column whose land's first voxel above the surface is `start`.
pub fn scan(kind: &dyn Fn(i32) -> Kind, start: i32) -> Cell {
    let mut y = start;
    if kind(y) == Kind::Solid {
        // Something stands on the ground here (a trunk, a wall, a stone): its top is
        // the floor, and the climb up to it is what keeps a beast off it.
        let mut k = 0;
        while kind(y) == Kind::Solid && k < 64 {
            y += 1;
            k += 1;
        }
    } else {
        // The ground is lower than the land's surface: a floor dug or levelled into it.
        let mut k = 0;
        while kind(y - 1) != Kind::Solid && k < 64 {
            y -= 1;
            k += 1;
        }
    }
    let floor = y;
    let mut water = 0u8;
    while kind(y) == Kind::Water && water < 250 {
        water += 1;
        y += 1;
    }
    let mut room = 0u8;
    while room < ROOM && matches!(kind(floor + room as i32), Kind::Air | Kind::Water) {
        room += 1;
    }
    Cell { floor: floor.clamp(i16::MIN as i32, i16::MAX as i32) as i16, water, room }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trunk_rises_out_of_the_ground_and_a_pond_has_depth() {
        // Ground at 10, a trunk standing 30 high, water 3 deep over a bed at 7.
        let trunk = |y: i32| if y < 41 { Kind::Solid } else { Kind::Air };
        assert_eq!(scan(&trunk, 11).floor, 41);
        let pond = |y: i32| match y {
            ..8 => Kind::Solid,
            8..11 => Kind::Water,
            _ => Kind::Air,
        };
        assert_eq!(scan(&pond, 8), Cell { floor: 8, water: 3, room: ROOM });
        // Under a branch two voxels up.
        let branch = |y: i32| match y {
            ..11 => Kind::Solid,
            13 => Kind::Solid,
            _ => Kind::Air,
        };
        assert_eq!(scan(&branch, 11), Cell { floor: 11, water: 0, room: 2 });
        let bush = |y: i32| match y {
            ..11 => Kind::Solid,
            11..14 => Kind::Foliage,
            _ => Kind::Air,
        };
        assert_eq!(scan(&bush, 11).room, 0);
    }
}
