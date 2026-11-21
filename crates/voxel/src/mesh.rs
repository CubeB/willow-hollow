//! Greedy mesher with baked per-vertex ambient occlusion.

use bytemuck::{Pod, Zeroable};

use crate::{CHUNK, Kind, Materials, PAD, PaddedChunk, flags, plant};

/// One quad corner, packed into 8 bytes.
///
/// `a`: x (6 bits) | y (6) << 6 | z (6) << 12 | face (3) << 18 | ao (2) << 21
///      | quarter-voxel fractions of x, y, z (2 bits each) << 23, 25, 27
///      | one more bit per axis, << 29, 30, 31, read with that axis's quarter `q`:
///        - `q == 0`: which side of its voxel the corner is on (set for single leaf
///          voxels and plant boxes, so the voxel is `floor(position) - bit`);
///        - `q != 0`: a half step, for plant skins half a cell thick. The corner moves an
///          eighth of a voxel toward the nearer face of its voxel, so `q` 1, 2, 3 put it
///          at 1/8, 5/8 and 7/8 of the voxel. That combination was unused before, so no
///          bits were added; see [`half_quarter`].
/// `b`: r | g << 8 | b << 16 | flags << 24 (flags include a 3-bit plant kind << 29)
///      Broadleaf leaves (plant kinds `DECIDUOUS` and `AUTUMN`) give up the low two bits
///      of each colour channel to a 6-bit tree tag; see [`tag_trees`].
///
/// Water faces (those with [`flags::WATER`](crate::flags::WATER)) have no fractions or
/// corner bits, so `a` bits 23..=31 hold the current instead (see [`pack_current`]):
/// direction << 23 (5 bits) | speed << 28 (3 bits) | river << 31. `b` is laid out as for
/// any face, with plant kind 0. All nine bits are zero on still water. The two AO bits of
/// a water surface say instead where it meets a fall: bit 21 set at a corner at the foot
/// of one (foam), bit 22 at a corner on the lip of one, which the shader draws lower so
/// the surface bends over into the fall.
///
/// The sheet of falling water itself has face index 6 and its own layout (see
/// [`spill_vertex`]).
///
/// Positions are chunk-local, 0..=32. Face index: 0 +X, 1 -X, 2 +Y, 3 -Y, 4 +Z, 5 -Z.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, PartialEq, Eq)]
pub struct Vertex {
    pub a: u32,
    pub b: u32,
}

/// Quads as runs of four vertices, counter-clockwise seen from outside.
/// Index pattern per quad is (0, 1, 2, 0, 2, 3).
#[derive(Default)]
pub struct ChunkMesh {
    pub opaque: Vec<Vertex>,
    pub water: Vec<Vertex>,
    /// Opaque quads are grouped by face direction: direction `f` is quads
    /// `faces[f]..faces[f + 1]`. Lets a renderer skip whole directions that cannot be seen.
    /// Plants follow the terrain as a second set of six groups, `faces[6 + f]..`, so
    /// they can be left out with distance.
    pub faces: [u32; 13],
}

/// Cells along a voxel's edge in the finer grid plants are modelled on.
pub const CELLS: i32 = 4;

/// One small box of a plant, in world cells. `hi` is exclusive.
#[derive(Clone, Copy, Debug)]
pub struct PlantBox {
    pub lo: [i32; 3],
    pub hi: [i32; 3],
    pub voxel: crate::Voxel,
    /// Whether the bottom and top of the box move in the wind.
    pub sway_lo: bool,
    pub sway_hi: bool,
    /// Faces drawn half a cell in from where `lo` and `hi` put them, one bit per face
    /// index (bit 2 is the top, +Y): a box one cell thick with one of these is a skin
    /// 0.05 m thick, like moss flat on a stone. Only half steps that land at 1/8, 5/8 or
    /// 7/8 of a voxel can be drawn (see [`Vertex`]); a skin flush against the voxel's
    /// near or far face always does.
    pub thin: u8,
    /// 0, or the number (1..=127) of the season this box keeps: a row of a table the
    /// renderer holds, which says between which dates the box is there and when it is in
    /// bud, in flower and gone to seed (a flower's bloom, its leaves, a berry). A box with
    /// one is drawn as a [`plant::FLOWER`](crate::plant::FLOWER) whatever its material,
    /// and comes and goes a column at a time. See [`season_bits`]. A ribbon or a sheet
    /// keeps more of what it does here instead (see [`Cloth`]).
    pub season: u8,
    /// 0, or cloth, which moves all over in the wind. From 1 to 31, how far a line strung
    /// between two posts gives in the wind where this box hangs on it: 1 where it is tied
    /// fast to 31 in the middle of a long span. The box rides the line as it swings, and
    /// where it hangs free (`sway_lo` or `sway_hi`) it flaps as cloth does: a string of
    /// prayer flags. See [`flap_code`]. Its top two bits make it a ribbon or a sheet
    /// instead: see [`Cloth`].
    pub flap: u8,
}

/// What kind of cloth a plant box is, as its [`PlantBox::flap`] holds it (and for a ribbon
/// or a sheet, its [`PlantBox::season`], which cloth has no other use for). The top two
/// bits of `flap` say which: neither for a line, as prayer flags have always been written,
/// then a ribbon, then a sheet. [`PlantBox::ribbon`] and [`PlantBox::sheet`] make them;
/// worldgen's `features::cloth` makes whole ribbons and sheets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cloth {
    /// Not cloth.
    None,
    /// On a line strung between two posts, giving this much there (see [`flap_code`]).
    Line(u8),
    /// A length of a ribbon tied by one end: how many cells below the knot the top of
    /// this length hangs (up to 63), and whether its foot is the ribbon's free end.
    Ribbon { knot: u8, end: bool },
    /// A tile of a sheet tied at its corners: how far each corner of the tile gives, from
    /// 0 where the sheet is tied to [`SHEET_GIVE`], in the order (low x, low z), (high x,
    /// low z), (high x, high z), (low x, high z); and how big the whole sheet is, 0 to 3,
    /// for how far it billows.
    Sheet { gives: [u8; 4], span: u8 },
}

/// The top two bits of [`PlantBox::flap`], which say what kind of cloth a box is.
const CLOTH_BITS: u8 = 0xC0;
const RIBBON: u8 = 0x40;
const SHEET: u8 = 0x80;
/// How far a sheet gives where it is held least (see [`Cloth::Sheet`]).
pub const SHEET_GIVE: u8 = 7;
/// How far below its knot a ribbon bends, in cells (3 m): below that it would hang on as
/// it does there.
pub const RIBBON_BENDS: i32 = 30;
/// The plant kind a corner of a ribbon or a sheet carries, which the shader knows it by:
/// ground's. Ground is never foliage, so nothing else that moves in the wind has it, and
/// with [`flags::FOLIAGE`] and a code in its tag (see [`cloth_code`]) it can only be cloth.
/// terrain.wgsl draws it as no plant at all, so the seasons leave it alone.
pub const CLOTH_PLANT: u8 = plant::GROUND;

impl PlantBox {
    /// What kind of cloth this box is, if any.
    pub fn cloth(&self) -> Cloth {
        match self.flap & CLOTH_BITS {
            _ if self.flap == 0 => Cloth::None,
            RIBBON => Cloth::Ribbon { knot: self.flap & 63, end: self.season & 1 != 0 },
            SHEET => Cloth::Sheet {
                gives: [self.flap & 7, self.flap >> 3 & 7, self.season & 7, self.season >> 3 & 7],
                span: self.season >> 6,
            },
            _ => Cloth::Line(self.flap),
        }
    }

    /// This box made into the given cloth; with [`Cloth::None`], a plain box keeping no
    /// season.
    pub fn with_cloth(mut self, cloth: Cloth) -> Self {
        (self.flap, self.season) = match cloth {
            Cloth::None => (0, 0),
            Cloth::Line(give) => (give.clamp(1, 31), 0),
            Cloth::Ribbon { knot, end } => (RIBBON | knot.min(63), end as u8),
            Cloth::Sheet { gives, span } => {
                let g = gives.map(|g| g.min(SHEET_GIVE));
                (SHEET | g[0] | g[1] << 3, g[2] | g[3] << 3 | span.min(3) << 6)
            }
        };
        self
    }

    /// A length of a ribbon (see [`Cloth::Ribbon`]).
    pub fn ribbon(lo: [i32; 3], hi: [i32; 3], voxel: crate::Voxel, knot: u8, end: bool) -> Self {
        Self { lo, hi, voxel, sway_lo: false, sway_hi: false, thin: 0, season: 0, flap: 0 }.with_cloth(Cloth::Ribbon { knot, end })
    }

    /// A tile of a sheet (see [`Cloth::Sheet`]).
    pub fn sheet(lo: [i32; 3], hi: [i32; 3], voxel: crate::Voxel, gives: [u8; 4], span: u8) -> Self {
        Self { lo, hi, voxel, sway_lo: false, sway_hi: false, thin: 0, season: 0, flap: 0 }.with_cloth(Cloth::Sheet { gives, span })
    }
}

/// What a corner of a ribbon or a sheet carries to the shader, in the six bits a leaf
/// keeps its tree's tag in (see [`tag_bits`]), on a vertex with [`flags::FOLIAGE`] and
/// plant kind [`CLOTH_PLANT`]. A ribbon's has bit 5 clear and 1 more than how many cells
/// below the knot the corner hangs (`below`, up to [`RIBBON_BENDS`]), so it is never 0. A
/// sheet's has bit 5 set, how big the sheet is in bits 3 and 4, and how far the corner
/// gives in the low three (`corner`, numbered as [`Cloth::Sheet`] lists them).
/// `cloth_motion` in terrain.wgsl reads it back. Anything else carries 0.
pub fn cloth_code(cloth: Cloth, below: i32, corner: usize) -> u8 {
    match cloth {
        Cloth::Ribbon { .. } => 1 + below.clamp(0, RIBBON_BENDS) as u8,
        Cloth::Sheet { gives, span } => 32 | span.min(3) << 3 | gives[corner & 3].min(SHEET_GIVE),
        _ => 0,
    }
}

/// What a corner of a box on a line ([`PlantBox::flap`]) carries to the shader, in the six
/// bits a leaf keeps its tree's tag in (see [`tag_bits`]): the line's give in the low
/// five, and whether the corner is a free edge of cloth above them. `flap_motion` in
/// terrain.wgsl reads it back, on vertices with [`flags::FOLIAGE`] and no plant kind.
pub fn flap_code(give: u8, free: bool) -> u8 {
    give.clamp(1, 31) | (free as u8) << 5
}

/// The plant kind a box is drawn as: its material's, or a flower's if it keeps a season.
fn box_kind(b: &PlantBox, mats: &Materials) -> u8 {
    if b.flap != 0 {
        plant::NONE
    } else if b.season != 0 {
        plant::FLOWER
    } else {
        mats.get(b.voxel).plant
    }
}

/// A season's number (see [`PlantBox::season`]) spread over the low two bits of each
/// colour channel, where a broadleaf leaf carries its tree tag, and its seventh bit in
/// the third bit of blue, which a flower gives up as well ([`SEASON_HIGH`]): the shader
/// reads them back the same way (`season_row` in season.wgsl). The table filled its
/// first 64 rows with the flowers and the crops; the fruit of the trees came after.
pub fn season_bits(season: u8) -> u32 {
    let s = (season & 127) as u32;
    (s & 3) | (s & 12) << 6 | (s & 112) << 12
}

/// The bit of a flower's colour that holds the seventh bit of its season's number: the
/// third of blue, a change of 4/255 at most.
pub const SEASON_HIGH: u32 = 1 << 18;

/// The quarter and half-step bit that put a vertex `r` eighths of a voxel into its voxel,
/// for odd `r`, or `None` for 3/8, which the format cannot hold.
pub fn half_quarter(r: u32) -> Option<u32> {
    match r {
        1 => Some(1),
        5 => Some(2),
        7 => Some(3),
        _ => None,
    }
}

impl ChunkMesh {
    pub fn is_empty(&self) -> bool {
        self.opaque.is_empty() && self.water.is_empty()
    }
}

/// How the water in one column runs.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Current {
    /// Velocity across the ground, metres per second along x and along z.
    pub x: f32,
    pub z: f32,
    /// Part of a river, which runs even where it is slow, rather than a lake or the sea.
    pub river: bool,
}

/// Fastest current a water vertex can carry, metres per second. The water shader keeps
/// the same number.
pub const MAX_CURRENT: f32 = 2.0;
/// First bit of the current in a water vertex's `a`.
pub const CURRENT_SHIFT: u32 = 23;
/// Directions a current is rounded to, a full turn from +X toward +Z.
const CURRENT_DIRECTIONS: f32 = 32.0;
/// Highest speed step: speeds are 0..=7, spaced as `MAX_CURRENT * (step / 7)^1.5`, so
/// slow brooks, whose ripples are watched closest, get the finer steps.
const CURRENT_STEPS: f32 = 7.0;

/// Packs a current (metres per second along x and z) into the bits of a water vertex's
/// `a` that it occupies: 23..=27 direction, 28..=30 speed, 31 river.
pub fn pack_current(x: f32, z: f32, river: bool) -> u32 {
    let speed = (x.hypot(z) / MAX_CURRENT).min(1.0);
    let step = (speed.powf(1.0 / 1.5) * CURRENT_STEPS).round() as u32;
    let dir = if step == 0 {
        0
    } else {
        ((z.atan2(x) / std::f32::consts::TAU * CURRENT_DIRECTIONS).round() as i32).rem_euclid(CURRENT_DIRECTIONS as i32) as u32
    };
    (dir | step << 5 | (river as u32) << 8) << CURRENT_SHIFT
}

/// The current a water vertex carries, as the shader reads it.
pub fn unpack_current(a: u32) -> Current {
    let bits = a >> CURRENT_SHIFT;
    let angle = (bits & 31) as f32 / CURRENT_DIRECTIONS * std::f32::consts::TAU;
    let step = ((bits >> 5) & 7) as f32 / CURRENT_STEPS;
    let speed = MAX_CURRENT * step * step.sqrt();
    Current {
        x: angle.cos() * speed,
        z: angle.sin() * speed,
        river: (bits >> 8) & 1 != 0,
    }
}

/// The current at one corner of a water face: the mean of the four columns around it, so
/// faces that share the corner agree and the current varies smoothly over the surface.
/// Dry columns count as still, which slows the water along its banks. `x` and `z` are
/// chunk-local corner coordinates, 0..=32.
fn corner_current(currents: &[Current], x: usize, z: usize) -> u32 {
    let (mut vx, mut vz, mut rivers) = (0.0, 0.0, 0);
    // The corner at local x lies between local voxels x - 1 and x: padded x and x + 1.
    for px in x..=x + 1 {
        for pz in z..=z + 1 {
            let c = currents[px * PAD + pz];
            vx += c.x;
            vz += c.z;
            rivers += c.river as u32;
        }
    }
    pack_current(vx * 0.25, vz * 0.25, rivers >= 2)
}

const N: usize = CHUNK as usize;

/// A visible face awaiting merging. Zero means "no face"; equal non-zero keys can merge.
/// bit 0: present, bit 1: water, bit 2: mergeable (uniform AO), bit 3: water surface
/// beside a river, which carries the current at each corner (and so never merges),
/// bits 8..32: colour, 32..40: flags, 40..48: four 2-bit AO values, 48..57: the colour
/// a plant gives the voxel from afar (see `mesh_chunk_under_sky`), 0 for none.
type FaceKey = u64;

thread_local! {
    /// Face keys for one direction: `N` slices of `N * N` cells. Greedy merging zeroes
    /// every cell it consumes, so the buffer is all zero again after each direction.
    static FACE_MASK: std::cell::RefCell<Vec<FaceKey>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The sheets of falling water passing out through the bottom of a chunk, for the chunk
/// below it to carry on down. Meshing a column of chunks from the top down with one of
/// these, starting empty, makes a fall whole however many chunks it drops through.
#[derive(Default)]
pub struct Falls {
    /// Per padded column (`x * PAD + z`) and direction (see [`SPILL_DIRS`]): 0 for no
    /// sheet, else one more than how far below its lip, in voxels, the sheet is at the
    /// top of the chunk's lowest voxel. Empty for none at all.
    through: Vec<[u8; 4]>,
}

/// The ways a sheet of water can fall from a column: toward +X, -X, +Z, -Z.
const SPILL_DIRS: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
/// Furthest below its lip a sheet's vertices can say they are, in voxels.
const SPILL_DEEPEST: u32 = 57;

/// How far below the lip a row of a sheet is, as a vertex holds it: quarter voxels down
/// to two voxels, where the sheet curves most, then whole voxels.
fn spill_code(quarters: u32) -> u32 {
    if quarters <= 8 { quarters } else { (quarters / 4 + 6).min(63) }
}

/// One vertex of a sheet of falling water, which has its own layout in `a`:
/// x | y << 6 | z << 12 as for any vertex, face 6 << 18, the way it falls << 21 (an index
/// into [`SPILL_DIRS`]), how far below the lip it is << 23 (6 bits, [`spill_code`]), how
/// near the water it lands in << 29 (0..=3, 3 at the foot), and << 31 a bit set on the
/// vertices that close the sheet's open ends, which stay on the face of the column the
/// water left. The shader moves the rest out from that face along the curve of the fall,
/// and every vertex down by the quarter voxels in its code, so `y` here is the voxel
/// boundary at or above it.
fn spill_vertex(p: [usize; 3], dir: usize, code: u32, foot: u32, flat: bool, b: u32) -> Vertex {
    Vertex {
        a: p[0] as u32 | (p[1] as u32) << 6 | (p[2] as u32) << 12 | 6 << 18 | (dir as u32) << 21 | code << 23 | foot << 29 | (flat as u32) << 31,
        b,
    }
}

/// Adds the sheets of water falling from every open side of the chunk's water, in place
/// of flat side faces: each starts at the lip, the top of a run of water with air beside
/// it, curves out and down through that air, and ends on the first water or ground under
/// it. `falls` brings in the sheets that come down from the chunk above and takes out
/// those that go on into the chunk below.
fn add_spills(mesh: &mut ChunkMesh, chunk: &PaddedChunk, mats: &Materials, solid: &[[u64; PAD]; PAD], water: &[[u64; PAD]; PAD], falls: &mut Falls) {
    let entering = std::mem::take(&mut falls.through);
    if entering.is_empty() && water.iter().all(|row| row.iter().all(|&w| w == 0)) {
        return;
    }
    let top = PAD - 1;
    // One walk down a column's side: calls `each(y, below)` for every level a sheet
    // passes, with how many voxels below the lip the top of that level is.
    let walk = |exposed: u64, air: u64, enter: u8, each: &mut dyn FnMut(usize, u32)| {
        let (mut live, mut below) = (false, 0u32);
        for y in (0..PAD).rev() {
            if y == top && enter != 0 {
                (live, below) = (true, enter as u32 - 1);
            } else if live && air >> y & 1 != 0 {
                below = (below + 1).min(SPILL_DEEPEST);
            } else {
                (live, below) = (exposed >> y & 1 != 0, 0);
            }
            if live {
                each(y, below);
            }
        }
    };
    // Which levels of each column's four sides a sheet passes, border columns included:
    // a sheet is closed at an end only where the column beside it has none.
    let mut passes = vec![[0u64; 4]; PAD * PAD];
    let mut active = Vec::new();
    for x in 0..PAD {
        for z in 0..PAD {
            for (d, &(dx, dz)) in SPILL_DIRS.iter().enumerate() {
                let (nx, nz) = (x as i32 + dx, z as i32 + dz);
                if nx < 0 || nz < 0 || nx >= PAD as i32 || nz >= PAD as i32 {
                    continue;
                }
                let air = !(solid[nx as usize][nz as usize] | water[nx as usize][nz as usize]);
                let exposed = water[x][z] & air;
                let enter = entering.get(x * PAD + z).map_or(0, |e| e[d]);
                if exposed == 0 && enter == 0 {
                    continue;
                }
                let (mut levels, mut leaving) = (0u64, 0u8);
                walk(exposed, air, enter, &mut |y, below| {
                    levels |= 1 << y;
                    if y == 1 {
                        leaving = below as u8 + 1;
                    }
                });
                passes[x * PAD + z][d] = levels;
                if leaving != 0 {
                    if falls.through.is_empty() {
                        falls.through = vec![[0; 4]; PAD * PAD];
                    }
                    falls.through[x * PAD + z][d] = leaving;
                }
                if (1..=N).contains(&x) && (1..=N).contains(&z) {
                    active.push((x, z, d, exposed, air, enter));
                }
            }
        }
    }
    // A sheet that comes down past rock has no water of its own to take its colour from.
    let any_water = (1..mats.len()).map(|i| i as crate::Voxel).find(|&v| mats.get(v).kind == Kind::Water);
    let voxels = chunk.voxels();
    for (x, z, d, exposed, air, enter) in active {
        let (dx, dz) = SPILL_DIRS[d];
        let (nx, nz) = ((x as i32 + dx) as usize, (z as i32 + dz) as usize);
        let landing = water[nx][nz];
        let own = voxels[(x * PAD + z) * PAD..][..PAD].iter().rev().copied().find(|&v| mats.get(v).kind == Kind::Water);
        let Some(v) = own.or(any_water) else { continue };
        let m = mats.get(v);
        let b = color_bits(m.color) as u32 | ((flags::WATER | ((m.jitter << flags::JITTER_SHIFT) & flags::JITTER_MASK)) as u32) << 24;
        // The face of the column the water leaves, and the two ends of the sheet along
        // it, in local voxels.
        let along_x = dx == 0;
        let near = if along_x { z } else { x };
        let plane = if dx + dz > 0 { near } else { near - 1 };
        let ends = if along_x { [x - 1, x] } else { [z - 1, z] };
        let at = |end: usize, y: usize| if along_x { [ends[end], y, plane] } else { [plane, y, ends[end]] };
        walk(exposed, air, enter, &mut |y, below| {
            if !(1..=N).contains(&y) {
                return;
            }
            // Rows of the sheet across this level: (code, y, nearness to the foot).
            let foot = air >> (y - 1) & 1 == 0 && landing >> (y - 1) & 1 != 0;
            let first = if below == 0 { 1 } else { 0 };
            let steps = if below < 2 { 4 } else { 1 };
            let rows: Vec<(u32, usize, u32)> = (first..=steps)
                .map(|r| {
                    let quarters = below * 4 + r * (4 / steps);
                    let nearness = if !foot { 0 } else if steps == 4 { (3 + r).saturating_sub(4) } else { 3 * r };
                    (spill_code(quarters), if r == steps { y - 1 } else { y }, nearness)
                })
                .collect();
            // Open ends: air beside the landing column with no sheet of its own in it.
            let open = [-1i32, 1].map(|step| {
                let (sx, sz) = if along_x { (step, 0) } else { (0, step) };
                let (ax, az) = ((x as i32 + sx) as usize, (z as i32 + sz) as usize);
                let (bx, bz) = ((nx as i32 + sx) as usize, (nz as i32 + sz) as usize);
                (solid[bx][bz] | water[bx][bz]) >> y & 1 == 0 && passes[ax * PAD + az][d] >> y & 1 == 0
            });
            for pair in rows.windows(2) {
                let (hi, lo) = (pair[0], pair[1]);
                let corner = |end: usize, row: (u32, usize, u32), flat: bool| spill_vertex(at(end, row.1), d, row.0, row.2, flat, b);
                mesh.water.extend([corner(0, hi, false), corner(1, hi, false), corner(1, lo, false), corner(0, lo, false)]);
                for end in 0..2 {
                    if open[end] {
                        mesh.water.extend([corner(end, hi, true), corner(end, hi, false), corner(end, lo, false), corner(end, lo, true)]);
                    }
                }
            }
        });
    }
}

/// Where a corner of a water surface meets a fall, for the two AO bits of its vertex:
/// bit 0 at the foot of one (a column around the corner holds water a level higher),
/// bit 1 on the lip of one (a column around it is open air at this level). `x` and `z`
/// are chunk-local corner coordinates, `y` the padded level of the surface's voxel.
fn corner_spill(solid: &[[u64; PAD]; PAD], water: &[[u64; PAD]; PAD], x: usize, y: usize, z: usize) -> u32 {
    let mut bits = 0;
    for px in x..=x + 1 {
        for pz in z..=z + 1 {
            bits |= (water[px][pz] >> (y + 1) & 1) as u32;
            bits |= (((solid[px][pz] | water[px][pz]) >> y & 1) as u32 ^ 1) << 1;
        }
    }
    bits
}

/// Meshes a chunk whose water is all still.
pub fn mesh_chunk(chunk: &PaddedChunk, mats: &Materials) -> ChunkMesh {
    mesh_chunk_with_currents(chunk, mats, &[])
}

/// Meshes a chunk, giving its water surface the current in each column. `currents` is
/// indexed `x * PAD + z` in padded coordinates, like the voxels, or empty if all the
/// water is still.
pub fn mesh_chunk_with_currents(chunk: &PaddedChunk, mats: &Materials, currents: &[Current]) -> ChunkMesh {
    mesh_chunk_under_sky(chunk, mats, currents, &[], &[], &mut Falls::default())
}

/// Marks each padded column (`x * PAD + z`) in `covered` that holds anything opaque or
/// any water in this chunk, from its first interior voxel up. Meshing a column of chunks
/// from the top down, what the chunks above have marked is what covers the next one.
pub fn cover_columns(chunk: &PaddedChunk, mats: &Materials, covered: &mut [bool]) {
    let voxels = chunk.voxels();
    for (i, c) in covered.iter_mut().enumerate().take(PAD * PAD) {
        if !*c {
            *c = voxels[i * PAD + 1..(i + 1) * PAD].iter().any(|&v| roofs(mats, v));
        }
    }
}

/// Whether a voxel keeps snow off what is below it: anything but air, broadleaf leaves,
/// which are down by the time snow lies, and wood (bare branches, a plank), which snow
/// falls past.
fn roofs(mats: &Materials, v: crate::Voxel) -> bool {
    let m = mats.get(v);
    m.kind != Kind::Air && !matches!(m.plant, plant::DECIDUOUS | plant::AUTUMN | plant::VINE)
}

/// [`mesh_chunk_with_currents`], knowing which columns have something above the chunk
/// (`covered`, from [`cover_columns`] on the chunks above, or empty for none). Every
/// opaque face of a voxel with nothing above it, open to the sky, carries
/// [`flags::SKY`], which is where snow settles. `falls` carries falling water down a
/// column of chunks meshed from the top (see [`Falls`]).
///
/// `wash` (one per padded column, or empty) names the voxels that take the colour of a
/// plant standing on them when seen from afar: in its low six bits the padded y of the
/// column's voxel that does (0 for none), and above them which colour, 1..=511. Every face
/// of such a voxel carries [`flags::WASH`] and, where other faces keep fractions and
/// corner bits (`a` bits 23..=31), the colour's number. Bits 16..22 are the column's
/// sward code (see [`tag_bits`]), which every face of grassy ground in the column
/// carries in the low bits of its colour.
pub fn mesh_chunk_under_sky(chunk: &PaddedChunk, mats: &Materials, currents: &[Current], covered: &[bool], wash: &[u32], falls: &mut Falls) -> ChunkMesh {
    assert!(wash.is_empty() || wash.len() == PAD * PAD, "one wash per padded column");
    assert!(currents.is_empty() || currents.len() == PAD * PAD, "one current per padded column");
    assert!(covered.is_empty() || covered.len() == PAD * PAD, "one cover per padded column");
    const SOLID: u8 = Kind::Solid as u8;
    const WATER: u8 = Kind::Water as u8;
    /// Bits of the interior voxels (padded y 1..=N) in a column mask.
    const INTERIOR: u64 = ((1u64 << N) - 1) << 1;

    let mut mesh = ChunkMesh::default();

    // One bit per voxel along y for each (x, z) column, so face visibility in all six
    // directions is a handful of bit operations per column rather than a test per voxel.
    // Foliage is meshed exactly like solid rock; the difference is only for collision.
    let kind_of: Vec<u8> = (0..mats.len())
        .map(|i| match mats.get(i as crate::Voxel).kind {
            Kind::Foliage => SOLID,
            k => k as u8,
        })
        .collect();
    let mut solid = [[0u64; PAD]; PAD];
    let mut water = [[0u64; PAD]; PAD];
    // The foliage among the solid bits. Foliage sways, so it must not hide the faces of
    // what it stands on, or the sway would open holes in the ground.
    let mut leafy = [[0u64; PAD]; PAD];
    // What keeps the snow off the voxels under it (see `roofs`).
    let mut roof = [[0u64; PAD]; PAD];
    let roofing: Vec<bool> = (0..mats.len()).map(|i| roofs(mats, i as crate::Voxel)).collect();
    let voxels = chunk.voxels();
    for x in 0..PAD {
        for z in 0..PAD {
            let col = &voxels[(x * PAD + z) * PAD..][..PAD];
            let (mut s, mut w, mut l, mut r) = (0u64, 0u64, 0u64, 0u64);
            for (y, &v) in col.iter().enumerate() {
                if roofing[v as usize] {
                    r |= 1 << y;
                }
                match kind_of[v as usize] {
                    SOLID => {
                        s |= 1 << y;
                        if mats.get(v).kind == Kind::Foliage {
                            l |= 1 << y;
                        }
                    }
                    WATER => w |= 1 << y,
                    _ => {}
                }
            }
            solid[x][z] = s;
            water[x][z] = w;
            leafy[x][z] = l;
            roof[x][z] = r;
        }
    }
    let is_solid = |p: [usize; 3]| ((solid[p[0]][p[2]] >> p[1]) & 1) as u32;

    FACE_MASK.with_borrow_mut(|mask| {
        mask.resize(N * N * N, 0);
        for face in 0..6usize {
            mesh.faces[face] = (mesh.opaque.len() / 4) as u32;
            let n_axis = face / 2;
            let positive = face.is_multiple_of(2);
            // (u, v, n) is a cyclic permutation of (x, y, z), so U x V = N.
            let u_axis = (n_axis + 1) % 3;
            let v_axis = (n_axis + 2) % 3;
            // Which rows (v) and columns (u) of each slice hold faces, so the merge
            // pass only scans the occupied part.
            let mut used_rows = [0u32; N];
            let mut used_cols = [0u32; N];

            for x in 1..=N {
                for z in 1..=N {
                    let (s, w, l) = (solid[x][z], water[x][z], leafy[x][z]);
                    if s == 0 && w == 0 {
                        continue;
                    }
                    let roofed = !covered.is_empty() && covered[x * PAD + z];
                    let washed = if wash.is_empty() { 0 } else { wash[x * PAD + z] };
                    // Solid and water masks of the neighbour each voxel's face looks at.
                    let (ns, nw, nl) = match face {
                        0 => (solid[x + 1][z], water[x + 1][z], leafy[x + 1][z]),
                        1 => (solid[x - 1][z], water[x - 1][z], leafy[x - 1][z]),
                        2 => (s >> 1, w >> 1, l >> 1),
                        3 => (s << 1, w << 1, l << 1),
                        4 => (solid[x][z + 1], water[x][z + 1], leafy[x][z + 1]),
                        _ => (solid[x][z - 1], water[x][z - 1], leafy[x][z - 1]),
                    };
                    // Solid faces show against anything not solid, and against foliage;
                    // foliage only against what is not solid; water only against air.
                    let mut opaque_faces = ((s & !l & !(ns & !nl)) | (l & !ns)) & INTERIOR;
                    // Water shows its surface. Where its side stands open it falls, and
                    // the fall is drawn as a sheet (see `add_spills`), not as a flat face.
                    let mut water_faces = if face == 2 { w & !(ns | nw) & INTERIOR } else { 0 };

                    while opaque_faces != 0 {
                        let y = opaque_faces.trailing_zeros() as usize;
                        opaque_faces &= opaque_faces - 1;
                        let p = [x, y, z];
                        let mut q = p;
                        if positive {
                            q[n_axis] += 1;
                        } else {
                            q[n_axis] -= 1;
                        }
                        // Occluders in the layer just outside the face, around each corner.
                        // `su`/`sv` are 0 for the negative side, 2 for the positive side.
                        let corner = |su: usize, sv: usize| -> u64 {
                            let (mut a, mut b) = (q, q);
                            a[u_axis] = a[u_axis] + su - 1;
                            b[v_axis] = b[v_axis] + sv - 1;
                            let mut c = a;
                            c[v_axis] = b[v_axis];
                            let (s1, s2) = (is_solid(a), is_solid(b));
                            (if s1 + s2 == 2 { 0 } else { 3 - (s1 + s2 + is_solid(c)) }) as u64
                        };
                        let ao = [corner(0, 0), corner(2, 0), corner(2, 2), corner(0, 2)];
                        let uniform = ao[0] == ao[1] && ao[1] == ao[2] && ao[2] == ao[3];
                        let ao_bits = ao[0] | ao[1] << 2 | ao[2] << 4 | ao[3] << 6;
                        let m = mats.get(voxels[PaddedChunk::idx(x, y, z)]);
                        let mut fl = (m.jitter << flags::JITTER_SHIFT) & flags::JITTER_MASK;
                        if m.kind == Kind::Foliage {
                            fl |= flags::FOLIAGE;
                        }
                        if m.emissive {
                            fl |= flags::EMISSIVE;
                        }
                        fl |= m.plant << flags::PLANT_SHIFT;
                        if !roofed && roof[x][z] >> (y + 1) == 0 {
                            fl |= flags::SKY;
                        }
                        // Leaves that fall one by one, and any other plant the shader may drop a
                        // voxel at a time, stay one voxel each, so the shader can tell which
                        // voxel every corner of a face belongs to.
                        let single = droppable(m.plant);
                        let mut tint = 0u64;
                        if (washed >> 6) & 0x1FF != 0 && (washed & 63) as usize == y && m.kind == Kind::Solid && !m.emissive {
                            tint = ((washed >> 6) & 0x1FF) as u64;
                            fl |= flags::WASH;
                        }
                        let mut colour = leaf_color_bits(m.color, m.plant);
                        if m.plant == plant::GROUND {
                            colour |= tag_bits((washed >> 16) as u8) as u64;
                        }
                        let key = 1
                            | tint << 48
                            | (uniform && !single) as u64 * 4
                            | colour << 8
                            | (fl as u64) << 32
                            | ao_bits << 40;
                        let (slice, u, v) = (p[n_axis] - 1, p[u_axis] - 1, p[v_axis] - 1);
                        mask[(slice * N + v) * N + u] = key;
                        used_rows[slice] |= 1 << v;
                        used_cols[slice] |= 1 << u;
                    }

                    // The surface on and beside a river is meshed a voxel at a time, so each
                    // corner can carry the current where it stands and the flow turns
                    // smoothly from voxel to voxel. Merged faces never touch a river
                    // column, so their corners are still, as the averages beside them are.
                    let running = water_faces != 0
                        && !currents.is_empty()
                        && (x - 1..=x + 1).any(|i| (z - 1..=z + 1).any(|j| currents[i * PAD + j].river));
                    // So is the surface at the lip and at the foot of a fall, whose corners
                    // say which they are: the levels at which a column around this one is
                    // open air, or holds water a level higher.
                    let mut by_fall = 0u64;
                    if water_faces != 0 {
                        for i in x - 1..=x + 1 {
                            for j in z - 1..=z + 1 {
                                by_fall |= !(solid[i][j] | water[i][j]) | water[i][j] >> 1;
                            }
                        }
                    }
                    while water_faces != 0 {
                        let y = water_faces.trailing_zeros() as usize;
                        water_faces &= water_faces - 1;
                        let p = [x, y, z];
                        let m = mats.get(voxels[PaddedChunk::idx(x, y, z)]);
                        let fl = flags::WATER
                            | ((m.jitter << flags::JITTER_SHIFT) & flags::JITTER_MASK);
                        // Water has no AO and, unless it is running, always merges.
                        let key = 1
                            | 2
                            | (if running || by_fall >> y & 1 != 0 { 8 } else { 4 })
                            | color_bits(m.color) << 8
                            | (fl as u64) << 32;
                        let (slice, u, v) = (p[n_axis] - 1, p[u_axis] - 1, p[v_axis] - 1);
                        mask[(slice * N + v) * N + u] = key;
                        used_rows[slice] |= 1 << v;
                        used_cols[slice] |= 1 << u;
                    }
                }
            }

            // Greedy rectangle merge over each slice that has faces.
            for s in 0..N {
                if used_rows[s] == 0 {
                    continue;
                }
                let mask = &mut mask[s * N * N..][..N * N];
                let u_first = used_cols[s].trailing_zeros() as usize;
                let u_end = N - used_cols[s].leading_zeros() as usize;
                for v in 0..N {
                    if used_rows[s] >> v & 1 == 0 {
                        continue;
                    }
                    let mut u = u_first;
                    while u < u_end {
                        let key = mask[v * N + u];
                        if key == 0 {
                            u += 1;
                            continue;
                        }
                        let (mut w, mut h) = (1, 1);
                        if key & 4 != 0 {
                            while u + w < N && mask[v * N + u + w] == key {
                                w += 1;
                            }
                            'grow: while v + h < N {
                                for k in 0..w {
                                    if mask[(v + h) * N + u + k] != key {
                                        break 'grow;
                                    }
                                }
                                h += 1;
                            }
                        }
                        for dv in 0..h {
                            mask[(v + dv) * N + u..(v + dv) * N + u + w].fill(0);
                        }
                        emit_quad(&mut mesh, key, face, [n_axis, u_axis, v_axis], s, u, v, w, h, currents, &solid, &water);
                        u += w;
                    }
                }
            }
        }
        mesh.faces[6..].fill((mesh.opaque.len() / 4) as u32);
    });
    // After the surfaces, so that a fall is drawn over the water behind it.
    add_spills(&mut mesh, chunk, mats, &solid, &water, falls);
    mesh
}

/// The `wash` [`mesh_chunk_under_sky`] takes for the chunk at height `cy` (in chunks), from
/// each padded column's ground height (in world voxels), colour number (0 for none) and
/// sward code (0 for grass as its country grows it). Only ground standing open to the air
/// is washed: none under a trunk, a rock or water. The sward goes with the column
/// whatever stands on it.
pub fn wash_map(chunk: &PaddedChunk, mats: &Materials, cy: i32, columns: &[(i32, u8, u8)]) -> Vec<u32> {
    if columns.iter().all(|c| c.1 == 0 && c.2 == 0) {
        return Vec::new();
    }
    let voxels = chunk.voxels();
    columns
        .iter()
        .enumerate()
        .map(|(i, &(top, code, sward))| {
            let sward = ((sward & 63) as u32) << 16;
            let y = top - cy * CHUNK + 1;
            if code == 0 || !(1..=CHUNK).contains(&y) {
                return sward;
            }
            let above = mats.get(voxels[i * PAD + y as usize + 1]).kind;
            sward | if above == Kind::Air { y as u32 | (code as u32) << 6 } else { 0 }
        })
        .collect()
}

/// Whether the shader may drop a plant of this kind one voxel at a time (season.wgsl's
/// `season_gone`, and grass buried in snow), which it decides at each corner of a face
/// from the voxel the corner names.
fn droppable(kind: u8) -> bool {
    matches!(kind, plant::DECIDUOUS | plant::AUTUMN | plant::GRASS | plant::FLOWER)
}

/// A plant box as it is drawn: whole, or one of the pieces it was cut into.
struct Piece {
    b: PlantBox,
    /// Faces that are not drawn, one bit per face index.
    hidden: u8,
}

/// A plant box cut at voxel boundaries, so that every corner of every face names the
/// same voxel. Otherwise the shader could drop some corners of a face and not others,
/// and the face would be torn into a long sliver toward the middle of the screen.
fn split_at_voxels(b: &PlantBox, mats: &Materials, out: &mut Vec<Piece>) {
    let kind = box_kind(b, mats);
    let start = out.len();
    out.push(Piece { b: *b, hidden: 0 });
    // Every small plant goes a whole column at a time where it gives out with distance
    // (see `plant_reach` in season.wgsl), so every box is cut where it crosses from one
    // column into the next; only those the seasons drop need cutting across their height.
    let whole = !droppable(kind);
    // Grass and flowers go a whole column at a time (see `vs` in terrain.wgsl), so the
    // pieces of one cut across its height are always there together, and the faces
    // where they meet are never seen. Leaves go voxel by voxel.
    let together = matches!(kind, plant::GRASS | plant::FLOWER);
    for axis in 0..3 {
        if whole && axis == 1 {
            continue;
        }
        let mut k = start;
        while k < out.len() {
            let p = out[k].b;
            // The first voxel boundary strictly inside the box.
            let cut = (p.lo[axis].div_euclid(CELLS) + 1) * CELLS;
            if cut < p.hi[axis] {
                let (mut a, mut c) = (p, p);
                a.hi[axis] = cut;
                c.lo[axis] = cut;
                // A sheet's tile cut across gives at the cut as the sheet does there.
                if let Cloth::Sheet { gives: [g0, g1, g2, g3], span } = p.cloth()
                    && axis != 1
                {
                    let f = (cut - p.lo[axis]) as f32 / (p.hi[axis] - p.lo[axis]) as f32;
                    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * f).round() as u8;
                    let (ga, gc) = if axis == 0 {
                        let (lo, hi) = (mix(g0, g1), mix(g3, g2));
                        ([g0, lo, hi, g3], [lo, g1, g2, hi])
                    } else {
                        let (lo, hi) = (mix(g0, g3), mix(g1, g2));
                        ([g0, g1, hi, lo], [lo, hi, g2, g3])
                    };
                    a = a.with_cloth(Cloth::Sheet { gives: ga, span });
                    c = c.with_cloth(Cloth::Sheet { gives: gc, span });
                }
                // Each piece keeps the skin insets of the faces it still has.
                a.thin &= !(1 << (2 * axis));
                c.thin &= !(1 << (2 * axis + 1));
                let (mut hide_a, mut hide_c) = (out[k].hidden, out[k].hidden);
                if axis == 1 {
                    // Where the box bends in the wind, the pieces must still meet: the
                    // one that held still at its fixed end bends, and the rest ride on it.
                    let moves = p.sway_lo || p.sway_hi;
                    a.sway_hi = moves;
                    c.sway_lo = moves;
                    if together {
                        hide_a |= 1 << 2;
                        hide_c |= 1 << 3;
                    }
                } else if whole {
                    // These pieces only ever part at the very end of the plants' reach,
                    // too far off to see between them.
                    hide_a |= 1 << (2 * axis);
                    hide_c |= 1 << (2 * axis + 1);
                }
                out[k] = Piece { b: a, hidden: hide_a };
                out.push(Piece { b: c, hidden: hide_c });
            } else {
                k += 1;
            }
        }
    }
}

/// For each padded column of a chunk (`[x][z]`), a bit per padded height where the voxel
/// is opaque and there all year: what a plant's face against it can never be seen past.
fn lasting(chunk: &PaddedChunk, mats: &Materials) -> [[u64; PAD]; PAD] {
    let hides: Vec<bool> = (0..mats.len())
        .map(|i| {
            let m = mats.get(i as crate::Voxel);
            m.kind.is_opaque() && !droppable(m.plant)
        })
        .collect();
    let mut bits = [[0u64; PAD]; PAD];
    // Voxels are stored a column at a time, in this same order.
    for (cell, column) in bits.iter_mut().flatten().zip(chunk.voxels().as_chunks::<PAD>().0) {
        for (y, &v) in column.iter().enumerate() {
            *cell |= (hides[v as usize] as u64) << y;
        }
    }
    bits
}

/// Whether face `face` of a box lies wholly against (or inside) voxels of `lasting`,
/// where nothing can see it. `origin` is the chunk's corner in world cells.
fn buried(b: &PlantBox, face: usize, origin: [i32; 3], lasting: &[[u64; PAD]; PAD]) -> bool {
    let axis = face / 2;
    // A face drawn in from the side of its box has the rest of its cell in front of it,
    // and one that moves in the wind up and down does not stay against anything.
    if b.thin >> face & 1 != 0 || (axis == 1 && if face == 2 { b.sway_hi } else { b.sway_lo }) {
        return false;
    }
    // The voxels holding the cells just outside the face, as padded chunk coordinates.
    let mut range = [(0i32, 0i32); 3];
    for a in 0..3 {
        let (lo, hi) = match (a == axis, face.is_multiple_of(2)) {
            (true, true) => (b.hi[a], b.hi[a]),
            (true, false) => (b.lo[a] - 1, b.lo[a] - 1),
            (false, _) => (b.lo[a], b.hi[a] - 1),
        };
        range[a] = ((lo - origin[a]).div_euclid(CELLS) + 1, (hi - origin[a]).div_euclid(CELLS) + 1);
        if range[a].0 < 0 || range[a].1 >= PAD as i32 {
            return false;
        }
    }
    let rows = (u64::MAX >> (63 - range[1].1)) & (u64::MAX << range[1].0);
    (range[0].0..=range[0].1).all(|x| (range[2].0..=range[2].1).all(|z| lasting[x as usize][z as usize] & rows == rows))
}

/// Appends plants to a chunk's mesh. `origin` is the chunk's corner in world cells, and
/// every box must start inside the chunk; any box reaching outside what a vertex can
/// hold is left out. Faces lying against the chunk's own lasting voxels (the ground a
/// tuft stands on) are left out too. `covered` is as for [`mesh_chunk_under_sky`]: a
/// plant with anything over it, in this chunk or one above, carries no [`flags::SKY`],
/// so no snow lies on the moss inside a hut or the grass under a bridge.
pub fn add_plants<'a>(
    mesh: &mut ChunkMesh,
    boxes: impl Iterator<Item = &'a PlantBox>,
    chunk: &PaddedChunk,
    origin: [i32; 3],
    mats: &Materials,
    covered: &[bool],
) {
    let reach = 63 * CELLS;
    let mut cut = Vec::new();
    for b in boxes {
        if (0..3).all(|a| b.lo[a] >= origin[a] && b.hi[a] <= origin[a] + reach && b.lo[a] < b.hi[a]) {
            split_at_voxels(b, mats, &mut cut);
        }
    }
    if !cut.is_empty() {
        let lasting = lasting(chunk, mats);
        for piece in &mut cut {
            let woven = match piece.b.cloth() {
                // A length of ribbon shows its top only at the knot and its foot only at the
                // free end: between, the next length covers them.
                Cloth::Ribbon { knot, end } => {
                    piece.hidden |= ((knot > 0) as u8) << 2 | (!end as u8) << 3;
                    true
                }
                Cloth::Sheet { .. } => true,
                _ => false,
            };
            for face in 0..6 {
                // Cloth moving all over leaves whatever it lay against.
                if !woven && buried(&piece.b, face, origin, &lasting) {
                    piece.hidden |= 1 << face;
                }
            }
        }
    }
    // Which pieces stand open to the sky: nothing that keeps snow off (see `roofs`) in
    // the voxels over the top of the piece, here or in the chunks above.
    let open: Vec<bool> = if cut.is_empty() {
        Vec::new()
    } else {
        let roofing: Vec<bool> = (0..mats.len()).map(|i| roofs(mats, i as crate::Voxel)).collect();
        let voxels = chunk.voxels();
        cut.iter()
            .map(|piece| {
                let b = &piece.b;
                let at = |a: usize, c: i32| ((c - origin[a]).div_euclid(CELLS) + 1).clamp(0, PAD as i32 - 1) as usize;
                let (x, z) = (at(0, b.lo[0]), at(2, b.lo[2]));
                if !covered.is_empty() && covered[x * PAD + z] {
                    return false;
                }
                let top = at(1, b.hi[1] - 1);
                !voxels[(x * PAD + z) * PAD..][..PAD][top + 1..].iter().any(|&v| roofing[v as usize])
            })
            .collect()
    };
    for face in 0..6usize {
        mesh.faces[6 + face] = (mesh.opaque.len() / 4) as u32;
        let n_axis = face / 2;
        let positive = face.is_multiple_of(2);
        let (u_axis, v_axis) = ((n_axis + 1) % 3, (n_axis + 2) % 3);
        for (piece, &open) in cut.iter().zip(&open) {
            if piece.hidden >> face & 1 != 0 {
                continue;
            }
            let b = &piece.b;
            let m = mats.get(b.voxel);
            let kind = box_kind(b, mats);
            let n = if positive { b.hi[n_axis] } else { b.lo[n_axis] };
            let corners = [
                (b.lo[u_axis], b.lo[v_axis]),
                (b.hi[u_axis], b.lo[v_axis]),
                (b.hi[u_axis], b.hi[v_axis]),
                (b.lo[u_axis], b.hi[v_axis]),
            ];
            // Anything else that sways with no plant kind (a chime's rod, a rope's end)
            // keeps the low bits of its colour clear, so as not to be taken for a box on
            // a line.
            let mut color = leaf_color_bits(m.color, kind) as u32;
            if kind == plant::NONE && (b.flap != 0 || b.sway_lo || b.sway_hi) {
                color &= !TAG_BITS;
            }
            // A ribbon or a sheet moves its own way, and is marked out by a plant kind of
            // its own (CLOTH_PLANT).
            let cloth = b.cloth();
            let woven = matches!(cloth, Cloth::Ribbon { .. } | Cloth::Sheet { .. });
            let drawn = if woven { CLOTH_PLANT } else { kind };
            let vertex = |i: usize| {
                let mut cell = [0i32; 3];
                cell[n_axis] = n;
                cell[u_axis] = corners[i].0;
                cell[v_axis] = corners[i].1;
                let at_top = cell[1] == b.hi[1];
                let mut p = [0u32; 3];
                let mut frac = [0u32; 3];
                // Half steps, per axis: a skin's face pulled half a cell in.
                let mut half = [false; 3];
                for axis in 0..3 {
                    let inset = (cell[axis] == b.hi[axis] && b.thin & 1 << (2 * axis) != 0) as i32
                        - (cell[axis] == b.lo[axis] && b.thin & 1 << (2 * axis + 1) != 0) as i32;
                    // In eighths of a voxel: half cells.
                    let local = 2 * (cell[axis] - origin[axis]) - inset;
                    let r = local.rem_euclid(2 * CELLS) as u32;
                    p[axis] = local.div_euclid(2 * CELLS) as u32;
                    if r % 2 == 0 {
                        frac[axis] = r / 2;
                    } else if let Some(q) = half_quarter(r) {
                        frac[axis] = q;
                        half[axis] = true;
                    } else {
                        // 3/8 has no encoding: draw the face where the box puts it.
                        debug_assert!(false, "{b:?} has a half step at 3/8 of a voxel");
                        frac[axis] = ((r as i32 + inset) / 2) as u32;
                    }
                }
                // Darker where a rooted stem meets the ground.
                let ao = if !at_top && !b.sway_lo && !woven { 1 } else { 3 };
                let sways = if at_top { b.sway_hi } else { b.sway_lo };
                // A box on a line moves all over, with the line, and so does other cloth.
                let fl = ((m.jitter << flags::JITTER_SHIFT) & flags::JITTER_MASK)
                    | if open { flags::SKY } else { 0 }
                    | if sways || b.flap != 0 { flags::FOLIAGE } else { 0 }
                    // Embers on a hearth and glass in a window shine as a whole voxel of
                    // them would. Not on what sways: with FOLIAGE it would read as WASH.
                    | if m.emissive && !(sways || b.flap != 0) { flags::EMISSIVE } else { 0 }
                    | drawn << flags::PLANT_SHIFT;
                let code = match cloth {
                    Cloth::None => season_bits(b.season),
                    Cloth::Line(_) => tag_bits(flap_code(b.flap, sways)),
                    Cloth::Ribbon { knot, .. } => tag_bits(cloth_code(cloth, knot as i32 + b.hi[1] - cell[1], 0)),
                    Cloth::Sheet { .. } => {
                        let corner = match (cell[0] == b.hi[0], cell[2] == b.hi[2]) {
                            (false, false) => 0,
                            (true, false) => 1,
                            (true, true) => 2,
                            (false, true) => 3,
                        };
                        tag_bits(cloth_code(cloth, 0, corner))
                    }
                };
                // A corner on the box's far edge, exactly on a voxel boundary, belongs to the
                // voxel before it. With a quarter set, the same bit is a half step instead.
                let mut corner = 0u32;
                for axis in 0..3 {
                    corner |= (((frac[axis] == 0 && cell[axis] == b.hi[axis]) || half[axis]) as u32) << axis;
                }
                Vertex {
                    a: p[0] | p[1] << 6 | p[2] << 12 | (face as u32) << 18 | ao << 21 | frac[0] << 23 | frac[1] << 25 | frac[2] << 27 | corner << 29,
                    b: color | code | (fl as u32) << 24,
                }
            };
            let order = if positive { [0, 1, 2, 3] } else { [0, 3, 2, 1] };
            mesh.opaque.extend(order.map(vertex));
        }
    }
    mesh.faces[12] = (mesh.opaque.len() / 4) as u32;
}

#[inline]
fn color_bits(c: [u8; 3]) -> u64 {
    c[0] as u64 | (c[1] as u64) << 8 | (c[2] as u64) << 16
}

/// The low two bits of each colour channel of a broadleaf leaf, which hold its tree tag.
const TAG_BITS: u32 = 0x03_0303;

/// A colour as packed into a vertex: a broadleaf leaf's with its tree tag cleared, to be
/// filled in by [`tag_trees`], and a flower's with the same bits and [`SEASON_HIGH`]
/// cleared for the number of its season (0, the common year of flowers, unless a
/// [`PlantBox`] names one).
#[inline]
fn leaf_color_bits(c: [u8; 3], kind: u8) -> u64 {
    let bits = color_bits(c);
    match kind {
        plant::FLOWER => bits & !((TAG_BITS | SEASON_HIGH) as u64),
        plant::DECIDUOUS | plant::AUTUMN | plant::GRASS | plant::GROUND => bits & !(TAG_BITS as u64),
        _ => bits,
    }
}

/// Six bits as a vertex carries them in the low two bits of each colour channel: a leaf
/// its tree's tag (see [`tag_trees`]), and grass its sward code (worldgen's
/// `surface::sward`: how wet and how worn the grass is where it grows), which `sward` in
/// terrain.wgsl reads back. Grassy ground has it from [`mesh_chunk_under_sky`], tufts
/// from [`tag_sward`].
pub fn tag_bits(code: u8) -> u32 {
    let t = (code & 63) as u32;
    (t & 3) | (t >> 2 & 3) << 8 | (t >> 4 & 3) << 16
}

/// Gives every grass tuft in a mesh the sward code of the column it stands in, as
/// [`tag_trees`] does a leaf its tree's tag. `code` is asked with the voxel each face
/// belongs to, in the chunk's own voxels counted from `origin`.
pub fn tag_sward(mesh: &mut ChunkMesh, origin: [i32; 3], mut code: impl FnMut([i32; 3]) -> u8) {
    let plants = mesh.faces[6] as usize * 4;
    for quad in mesh.opaque[plants..].chunks_exact_mut(4) {
        if (quad[0].b >> 29) & 7 != plant::GRASS as u32 {
            continue;
        }
        let bits = tag_bits(code(face_voxel(quad[0].a, origin)));
        for v in quad {
            v.b = (v.b & !TAG_BITS) | bits;
        }
    }
}

/// The voxel a face belongs to, from the first corner of its quad: the one the shader
/// names (see `vertex_corner` in terrain.wgsl).
fn face_voxel(a: u32, origin: [i32; 3]) -> [i32; 3] {
    let mut voxel = [0i32; 3];
    for (axis, v) in voxel.iter_mut().enumerate() {
        let p = (a >> (6 * axis)) & 63;
        let quarter = (a >> (23 + 2 * axis)) & 3;
        // With no quarter, the corner bit says the corner is on the far side of its voxel.
        let back = if quarter == 0 { (a >> (29 + axis)) & 1 } else { 0 };
        *v = origin[axis] + p as i32 - back as i32;
    }
    voxel
}

/// Gives every broadleaf leaf face in a mesh the tag of the tree it grows on, 1..=63 (0
/// for none), in the low two bits of each of its colour channels: red holds bits 0..2 of
/// the tag, green 2..4 and blue 4..6. A change of 3/255 at most, far below the colour's
/// own jitter. The seasons drop and turn each tree's leaves on its own date from it.
///
/// `tag` is asked once per face, with the voxel the face belongs to, in the chunk's own
/// voxels counted from `origin` (the voxel at the chunk's corner): the same voxel the
/// shader names from the face's corners.
pub fn tag_trees(mesh: &mut ChunkMesh, origin: [i32; 3], mut tag: impl FnMut([i32; 3]) -> u8) {
    for quad in mesh.opaque.chunks_exact_mut(4) {
        let first = quad[0];
        let kind = (first.b >> 29) & 7;
        if kind != plant::DECIDUOUS as u32 && kind != plant::AUTUMN as u32 {
            continue;
        }
        let bits = tag_bits(tag(face_voxel(first.a, origin)));
        for v in quad {
            v.b = (v.b & !TAG_BITS) | bits;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_quad(
    mesh: &mut ChunkMesh,
    key: FaceKey,
    face: usize,
    axes: [usize; 3],
    s: usize,
    u: usize,
    v: usize,
    w: usize,
    h: usize,
    currents: &[Current],
    solid: &[[u64; PAD]; PAD],
    water: &[[u64; PAD]; PAD],
) {
    let [n_axis, u_axis, v_axis] = axes;
    let positive = face.is_multiple_of(2);
    let n = s + positive as usize;
    let ao_bits = (key >> 40) & 0xFF;
    let ao = |i: usize| ((ao_bits >> (i * 2)) & 3) as u32;
    let b = ((key >> 8) & 0xFFFF_FFFF) as u32;

    // Corners counter-clockwise seen from +N, matching the AO corner order.
    let corners = [(u, v), (u + w, v), (u + w, v + h), (u, v + h)];
    let place = |i: usize| {
        let mut p = [0u32; 3];
        p[n_axis] = n as u32;
        p[u_axis] = corners[i].0 as u32;
        p[v_axis] = corners[i].1 as u32;
        p
    };
    // Where a corner of a water surface meets a fall (only surfaces are kept apart).
    let spill = |i: usize| {
        let p = place(i);
        corner_spill(solid, water, p[0] as usize, s + 1, p[2] as usize)
    };
    let vertex = |i: usize| {
        let p = place(i);
        // A single leaf (or other droppable plant) voxel says which side of it each corner
        // is on, so the shader can name the voxel from any corner.
        let kind = ((key >> 32) as u8 & flags::PLANT_MASK) >> flags::PLANT_SHIFT;
        let mut corner = 0u32;
        if key & 2 == 0 && droppable(kind) {
            corner |= ((p[n_axis] as usize > s) as u32) << n_axis;
            corner |= ((corners[i].0 > u) as u32) << u_axis;
            corner |= ((corners[i].1 > v) as u32) << v_axis;
        }
        // Running water keeps its current where other faces keep fractions and corners,
        // and water by a fall says so where other faces keep AO.
        let (mut current, mut shade) = (0, ao(i));
        if key & 8 != 0 {
            if !currents.is_empty() {
                current = corner_current(currents, p[0] as usize, p[2] as usize);
            }
            shade = spill(i);
        }
        let wash = ((key >> 48) & 0x1FF) as u32;
        Vertex {
            a: p[0] | p[1] << 6 | p[2] << 12 | (face as u32) << 18 | shade << 21 | corner << 29 | current | wash << 23,
            b,
        }
    };
    // Negative faces are seen from the other side, so reverse the winding.
    let mut order = if positive { [0, 1, 2, 3] } else { [0, 3, 2, 1] };
    // The index pattern splits along the first-to-third diagonal. Put that diagonal
    // between the brighter pair of corners so a dark corner stays in its own triangle.
    if ao(order[1]) + ao(order[3]) > ao(order[0]) + ao(order[2]) {
        order.rotate_left(1);
    }
    // A surface with one corner on a lip folds down across the corners beside it.
    if key & 8 != 0 && spill(order[0]) >> 1 != spill(order[2]) >> 1 && spill(order[1]) >> 1 == spill(order[3]) >> 1 {
        order.rotate_left(1);
    }
    let out = if key & 2 != 0 {
        &mut mesh.water
    } else {
        &mut mesh.opaque
    };
    out.extend(order.iter().map(|&i| vertex(i)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Material, Voxel};

    fn mats() -> (Materials, Voxel, Voxel) {
        let mut m = Materials::new();
        let stone = m.add(Material {
            color: [100, 100, 100],
            kind: Kind::Solid,
            jitter: 1,
            emissive: false,
            plant: 0,
        });
        let water = m.add(Material {
            color: [40, 90, 200],
            kind: Kind::Water,
            jitter: 0,
            emissive: false,
            plant: 0,
        });
        (m, stone, water)
    }

    fn pos(v: &Vertex) -> [u32; 3] {
        [v.a & 63, (v.a >> 6) & 63, (v.a >> 12) & 63]
    }

    #[test]
    fn empty_and_full_chunks_have_no_faces() {
        let (m, stone, _) = mats();
        assert!(mesh_chunk(&PaddedChunk::new(), &m).is_empty());
        let mut full = PaddedChunk::new();
        for x in 0..PAD {
            for z in 0..PAD {
                full.column_mut(x, z).fill(stone);
            }
        }
        assert!(mesh_chunk(&full, &m).is_empty());
    }

    #[test]
    fn single_voxel_is_six_quads() {
        let (m, stone, _) = mats();
        let mut c = PaddedChunk::new();
        c.set(5, 6, 7, stone);
        let mesh = mesh_chunk(&c, &m);
        assert_eq!(mesh.opaque.len(), 6 * 4);
        for v in &mesh.opaque {
            let p = pos(v);
            assert!(p[0] == 4 || p[0] == 5);
            assert!(p[1] == 5 || p[1] == 6);
            assert!(p[2] == 6 || p[2] == 7);
            assert_eq!((v.a >> 21) & 3, 3, "an isolated voxel is unoccluded");
        }
    }

    #[test]
    fn plant_boxes_land_on_quarter_voxels_in_their_own_groups() {
        let (m, stone, _) = mats();
        let mut c = PaddedChunk::new();
        c.set(5, 6, 7, stone);
        let mut mesh = mesh_chunk(&c, &m);
        let chunk = [32 * CELLS, 0, 0];
        let plant = PlantBox {
            lo: [chunk[0] + 9, 5, 2],
            hi: [chunk[0] + 10, 11, 3],
            voxel: stone,
            sway_lo: false,
            sway_hi: true,
            thin: 0,
            season: 0,
            flap: 0,
        };
        add_plants(&mut mesh, [plant].iter(), &c, chunk, &m, &[]);
        assert_eq!(mesh.opaque.len(), (6 + 6) * 4, "all six faces, the bottom too");
        assert_eq!(mesh.faces[6], 6);
        assert_eq!(mesh.faces[12], 12);
        assert_eq!(mesh.faces[9] + 1, mesh.faces[10], "one downward quad");
        for v in &mesh.opaque[6 * 4..] {
            let fine = |shift: u32, frac: u32| ((v.a >> shift) & 63) * 4 + ((v.a >> frac) & 3);
            assert!((9..=10).contains(&fine(0, 23)));
            assert!((5..=11).contains(&fine(6, 25)));
            assert!((2..=3).contains(&fine(12, 27)));
            let top = fine(6, 25) == 11;
            assert_eq!((v.b >> 24) as u8 & flags::FOLIAGE != 0, top, "only the free end sways");
        }
    }

    /// The bits `season_tag` in season.wgsl reads from a vertex.
    fn tag_of(b: u32) -> u8 {
        ((b & 3) | (b >> 6 & 12) | (b >> 12 & 48)) as u8
    }

    /// The number `season_row` in season.wgsl reads from a flower's vertex.
    fn row_of(b: u32) -> u8 {
        tag_of(b) | (b >> 12 & 64) as u8
    }

    #[test]
    fn a_flower_carries_every_row_of_the_table_and_nothing_else_in_its_colour() {
        let (m, stone, _) = mats();
        let c = PaddedChunk::new();
        for season in [1u8, 9, 63, 64, 65, 100, 127] {
            let b = PlantBox { lo: [9, 5, 2], hi: [10, 6, 3], voxel: stone, sway_lo: false, sway_hi: false, thin: 0, season, flap: 0 };
            let mut mesh = ChunkMesh::default();
            add_plants(&mut mesh, [b].iter(), &c, [0, 0, 0], &m, &[]);
            assert!(!mesh.opaque.is_empty());
            for v in &mesh.opaque {
                assert_eq!((v.b >> 24) as u8 >> flags::PLANT_SHIFT, plant::FLOWER);
                assert_eq!(row_of(v.b), season, "row {season}");
            }
        }
    }

    #[test]
    fn a_flag_on_a_line_moves_all_over_and_flaps_at_its_free_edge() {
        let (m, stone, _) = mats();
        let c = PaddedChunk::new();
        let chunk = [0, 0, 0];
        // A flag hanging from a line, and a rod hanging from a post (not on a line).
        let flag = PlantBox { lo: [9, 5, 2], hi: [11, 8, 3], voxel: stone, sway_lo: true, sway_hi: false, thin: 0, season: 0, flap: 17 };
        let rod = PlantBox { lo: [20, 5, 2], hi: [21, 8, 3], voxel: stone, sway_lo: true, sway_hi: false, thin: 0, season: 0, flap: 0 };
        let mut mesh = ChunkMesh::default();
        add_plants(&mut mesh, [flag, rod].iter(), &c, chunk, &m, &[]);
        assert!(!mesh.opaque.is_empty());
        for v in &mesh.opaque {
            let x = ((v.a & 63) * 4 + ((v.a >> 23) & 3)) as i32;
            let y = (((v.a >> 6) & 63) * 4 + ((v.a >> 25) & 3)) as i32;
            let fl = (v.b >> 24) as u8;
            assert_eq!(fl >> flags::PLANT_SHIFT, 0, "no plant kind");
            if x < 16 {
                assert!(fl & flags::FOLIAGE != 0, "every corner of a flag moves");
                assert_eq!(tag_of(v.b), flap_code(17, y == 5));
            } else {
                assert_eq!(fl & flags::FOLIAGE != 0, y == 5, "only a rod's free end sways");
                assert_eq!(tag_of(v.b), 0, "a rod is not taken for a flag");
            }
        }
    }

    #[test]
    fn cloth_reads_back_as_it_was_made_and_a_line_as_it_always_was() {
        let (_, stone, _) = mats();
        let plain = PlantBox { lo: [0; 3], hi: [1; 3], voxel: stone, sway_lo: false, sway_hi: false, thin: 0, season: 9, flap: 0 };
        assert_eq!(plain.cloth(), Cloth::None);
        for give in 1..=31 {
            assert_eq!(PlantBox { flap: give, season: 0, ..plain }.cloth(), Cloth::Line(give));
        }
        for cloth in [
            Cloth::Ribbon { knot: 0, end: false },
            Cloth::Ribbon { knot: 63, end: true },
            Cloth::Sheet { gives: [0, 7, 3, 5], span: 3 },
            Cloth::Sheet { gives: [7, 0, 0, 1], span: 0 },
        ] {
            assert_eq!(plain.with_cloth(cloth).cloth(), cloth);
        }
        // Every code a ribbon or a sheet carries is one, and they never meet.
        let ribbon = |below| cloth_code(Cloth::Ribbon { knot: 0, end: false }, below, 0);
        assert_eq!((ribbon(-3), ribbon(0), ribbon(12), ribbon(99)), (1, 1, 13, 1 + RIBBON_BENDS as u8));
        assert!(ribbon(99) < 32);
        let sheet = cloth_code(Cloth::Sheet { gives: [0, 7, 2, 4], span: 2 }, 0, 1);
        assert_eq!(sheet, 32 | 2 << 3 | 7);
        assert_eq!(cloth_code(Cloth::Line(5), 3, 1), 0);
    }

    /// Each corner of a mesh's cloth: where it is in cells, its plant kind and its code.
    fn cloth_corners(mesh: &ChunkMesh) -> Vec<([i32; 3], u8, u8)> {
        mesh.opaque
            .iter()
            .map(|v| {
                let fine = |shift: u32, frac: u32| (((v.a >> shift) & 63) * 4 + ((v.a >> frac) & 3)) as i32;
                let fl = (v.b >> 24) as u8;
                assert!(fl & flags::FOLIAGE != 0, "all cloth moves");
                assert_eq!((v.a >> 21) & 3, 3, "cloth hangs free, unshaded");
                ([fine(0, 23), fine(6, 25), fine(12, 27)], fl >> flags::PLANT_SHIFT, tag_of(v.b))
            })
            .collect()
    }

    #[test]
    fn a_ribbon_carries_how_far_each_corner_hangs_below_its_knot() {
        let (m, stone, _) = mats();
        let c = PaddedChunk::new();
        // Two lengths of a ribbon tied at cell height 30, the second its free end.
        let a = PlantBox::ribbon([5, 29, 2], [7, 30, 3], stone, 0, false);
        let b = PlantBox::ribbon([5, 28, 2], [7, 29, 3], stone, 1, true);
        let mut mesh = ChunkMesh::default();
        add_plants(&mut mesh, [a, b].iter(), &c, [0, 0, 0], &m, &[]);
        let corners = cloth_corners(&mesh);
        // Four sides each, the knot's top and the free end's foot; none between.
        assert_eq!(corners.len(), (4 + 4 + 2) * 4);
        for (p, kind, code) in corners {
            assert_eq!(kind, CLOTH_PLANT);
            assert_eq!(code as i32, 1 + 30 - p[1], "at {p:?}");
        }
    }

    #[test]
    fn a_sheet_carries_how_far_each_corner_gives_even_where_it_is_cut() {
        let (m, stone, _) = mats();
        let c = PaddedChunk::new();
        // A tile across a voxel's edge (cells 6 to 10 along x), cut there in two.
        let tile = PlantBox::sheet([6, 20, 0], [10, 21, 4], stone, [0, 4, 6, 2], 1);
        let mut mesh = ChunkMesh::default();
        add_plants(&mut mesh, [tile].iter(), &c, [0, 0, 0], &m, &[]);
        let corners = cloth_corners(&mesh);
        assert!(!corners.is_empty());
        for (p, kind, code) in corners {
            assert_eq!(kind, CLOTH_PLANT);
            assert_eq!(code & !7, 32 | 1 << 3);
            let want = match (p[0], p[2]) {
                (6, 0) => 0,
                (10, 0) => 4,
                (10, 4) => 6,
                (6, 4) => 2,
                // At the cut, half way along each edge.
                (8, 0) => 2,
                (8, 4) => 4,
                other => panic!("a corner at {other:?}"),
            };
            assert_eq!(code & 7, want, "at {p:?}");
        }
    }

    /// A vertex's position in eighths of a voxel, decoded the way the shaders do it.
    fn eighths(v: &Vertex) -> [i32; 3] {
        let p = pos(v);
        std::array::from_fn(|axis| {
            let q = (v.a >> (23 + 2 * axis)) & 3;
            let bit = (v.a >> (29 + axis)) & 1;
            let nudge = match (bit, q) {
                (1, 1) => -1,
                (1, 2 | 3) => 1,
                _ => 0,
            };
            (p[axis] * 8 + q * 2) as i32 + nudge
        })
    }

    /// The voxel a vertex says it belongs to, as the shaders work it out.
    fn named_voxel(v: &Vertex) -> [i32; 3] {
        let e = eighths(v);
        std::array::from_fn(|axis| {
            let q = (v.a >> (23 + 2 * axis)) & 3;
            let bit = ((v.a >> (29 + axis)) & 1) as i32;
            e[axis].div_euclid(8) - if q == 0 { bit } else { 0 }
        })
    }

    #[test]
    fn every_corner_of_a_droppable_plant_face_names_one_voxel() {
        let (mut m, _, _) = mats();
        let flower = m.add(Material { color: [200, 60, 200], kind: Kind::Solid, jitter: 0, emissive: false, plant: plant::FLOWER });
        let grass = m.add(Material { color: [60, 160, 60], kind: Kind::Solid, jitter: 0, emissive: false, plant: plant::GRASS });
        let stone = m.add(Material { color: [100, 100, 100], kind: Kind::Solid, jitter: 0, emissive: false, plant: plant::NONE });
        let chunk = [0, 0, 0];
        // Boxes straddling voxel boundaries on every axis, and one reaching out of the chunk.
        let boxes = [
            PlantBox { lo: [3, 3, 6], hi: [9, 6, 11], voxel: flower, sway_lo: false, sway_hi: true, thin: 0, season: 0, flap: 0 },
            PlantBox { lo: [7, 2, 2], hi: [10, 9, 5], voxel: grass, sway_lo: false, sway_hi: true, thin: 0, season: 0, flap: 0 },
            PlantBox { lo: [-2, 2, 2], hi: [2, 3, 3], voxel: grass, sway_lo: false, sway_hi: true, thin: 0, season: 0, flap: 0 },
        ];
        let mut mesh = ChunkMesh::default();
        add_plants(&mut mesh, boxes.iter(), &PaddedChunk::new(), chunk, &m, &[]);
        assert!(!mesh.opaque.is_empty());
        for quad in mesh.opaque.chunks_exact(4) {
            let named: Vec<[i32; 3]> = quad.iter().map(named_voxel).collect();
            for v in &named {
                assert_eq!(v[0], named[0][0], "{named:?}");
                assert_eq!(v[2], named[0][2], "{named:?}");
                assert_eq!(v[1], named[0][1], "{named:?}");
            }
        }
        // A stone box is cut only where it crosses from one column into the next (three
        // columns along x here and two along z), and the faces between its pieces are not
        // drawn: every corner of a face names one column, for giving out with distance.
        let mut mesh = ChunkMesh::default();
        let b = PlantBox { lo: [3, 3, 6], hi: [9, 6, 11], voxel: stone, sway_lo: false, sway_hi: false, thin: 0, season: 0, flap: 0 };
        add_plants(&mut mesh, [b].iter(), &PaddedChunk::new(), chunk, &m, &[]);
        assert_eq!(mesh.opaque.len(), (2 * 6 + 2 * 2 + 2 * 3) * 4);
        for quad in mesh.opaque.chunks_exact(4) {
            let named: Vec<[i32; 3]> = quad.iter().map(named_voxel).collect();
            assert!(named.iter().all(|v| v[0] == named[0][0] && v[2] == named[0][2]), "{named:?}");
        }
    }

    #[test]
    fn washed_ground_carries_its_colour_on_every_face_and_nothing_else_does() {
        let (m, stone, _) = mats();
        // A floor two deep with one block standing on it.
        let mut chunk = PaddedChunk::new();
        for x in 0..PAD {
            for z in 0..PAD {
                chunk.set(x, 1, z, stone);
                chunk.set(x, 2, z, stone);
            }
        }
        chunk.set(9, 3, 9, stone);
        // The ground's height in each column, with a wash on three of them: one in the
        // open, one under the block, and one a level too low to be the top.
        let mut columns = vec![(1, 0u8, 0u8); PAD * PAD];
        columns[5 * PAD + 5] = (1, 7, 0);
        columns[9 * PAD + 9] = (1, 9, 0);
        columns[12 * PAD + 12] = (0, 11, 0);
        let wash = wash_map(&chunk, &m, 0, &columns);
        assert_eq!(wash[5 * PAD + 5], 2 | 7 << 6);
        assert_eq!(wash[9 * PAD + 9], 0, "nothing grows under a block");
        let mesh = mesh_chunk_under_sky(&chunk, &m, &[], &[], &wash, &mut Falls::default());
        let washed: Vec<&[Vertex]> = mesh.opaque.chunks_exact(4).filter(|q| (q[0].b >> 24) as u8 & flags::WASH == flags::WASH).collect();
        // The top of the one washed voxel, alone: its sides are against its neighbours.
        assert_eq!(washed.len(), 1);
        for v in washed[0] {
            assert_eq!(v.a >> 23, 7, "the colour's number, where other faces keep fractions");
            assert_eq!((v.a >> 18) & 7, 2);
        }
        let xs: Vec<u32> = washed[0].iter().map(|v| v.a & 63).collect();
        assert_eq!((xs.iter().min(), xs.iter().max()), (Some(&4), Some(&5)), "one voxel wide: it never merges with bare ground");
        // With no wash at all the map is empty, and the floor merges as it always did.
        assert!(wash_map(&chunk, &m, 0, &vec![(1, 0u8, 0u8); PAD * PAD]).is_empty());
    }

    #[test]
    fn skins_are_half_a_cell_thick_on_every_side() {
        let (m, stone, _) = mats();
        let chunk = [32 * CELLS, 0, 0];
        // A skin on each of the six faces of the voxel at (3, 4, 5) of the chunk: the boxes
        // fill the air voxels around it, against it.
        let v = [3 * CELLS, 4 * CELLS, 5 * CELLS];
        for face in 0..6usize {
            let axis = face / 2;
            let positive = face.is_multiple_of(2);
            let (mut lo, mut hi) = ([v[0] + 1, v[1] + 1, v[2] + 1], [v[0] + 3, v[1] + 3, v[2] + 3]);
            // In the neighbouring voxel, one cell thick against the shared face; the skin's
            // outer face, away from the stone, is the one pulled in.
            if positive {
                lo[axis] = v[axis] + CELLS;
                hi[axis] = lo[axis] + 1;
            } else {
                hi[axis] = v[axis];
                lo[axis] = hi[axis] - 1;
            }
            let outer = if positive { 2 * axis } else { 2 * axis + 1 };
            let b = PlantBox {
                lo: [lo[0] + chunk[0], lo[1], lo[2]],
                hi: [hi[0] + chunk[0], hi[1], hi[2]],
                voxel: stone,
                sway_lo: false,
                sway_hi: false,
                thin: 1 << outer,
                season: 0,
                flap: 0,
            };
            let mut mesh = ChunkMesh::default();
            add_plants(&mut mesh, [b].iter(), &PaddedChunk::new(), chunk, &m, &[]);
            assert_eq!(mesh.opaque.len() / 4, 6, "face {face}");
            let host = if positive { v[axis] + CELLS } else { v[axis] };
            let mut depths = std::collections::BTreeSet::new();
            for vert in &mesh.opaque {
                let e = eighths(vert);
                depths.insert((e[axis] - 2 * host).abs());
                // Across the face, corners stay on whole cells.
                for other in (0..3).filter(|&o| o != axis) {
                    assert_eq!(e[other] % 2, 0, "face {face}: {e:?}");
                }
                // Every corner names the air voxel the skin is in.
                let mut air = [3, 4, 5];
                air[axis] += if positive { 1 } else { -1 };
                assert_eq!(named_voxel(vert), air, "face {face}: {e:?}");
            }
            assert_eq!(depths.into_iter().collect::<Vec<_>>(), vec![0, 1], "face {face}: half a cell from the stone");
        }
    }

    #[test]
    fn every_plant_box_is_drawn_from_below() {
        let (m, stone, _) = mats();
        let skin = |thin| PlantBox { lo: [0, 7, 0], hi: [4, 8, 4], voxel: stone, sway_lo: false, sway_hi: false, thin, season: 0, flap: 0 };
        for thin in [0, 1 << 2, 1 << 3] {
            let mut mesh = ChunkMesh::default();
            add_plants(&mut mesh, [skin(thin)].iter(), &PaddedChunk::new(), [0, 0, 0], &m, &[]);
            assert_eq!(mesh.opaque.len() / 4, 6);
            assert_eq!(mesh.faces[9] + 1, mesh.faces[10], "one downward quad");
        }
    }

    #[test]
    fn faces_nothing_can_see_are_left_out() {
        let (mut m, stone, _) = mats();
        let grass = m.add(Material { color: [60, 160, 60], kind: Kind::Solid, jitter: 0, emissive: false, plant: plant::GRASS });
        let leaf = m.add(Material { color: [60, 160, 60], kind: Kind::Foliage, jitter: 0, emissive: false, plant: plant::DECIDUOUS });
        let quads = |c: &PaddedChunk, b: PlantBox| {
            let mut mesh = ChunkMesh::default();
            add_plants(&mut mesh, [b].iter(), c, [0, 0, 0], &m, &[]);
            (0..6).map(|f| mesh.faces[7 + f] - mesh.faces[6 + f]).collect::<Vec<_>>()
        };
        // A blade standing on the voxel at (2, 0, 2) of the chunk, two and a bit voxels tall.
        let blade = PlantBox { lo: [11, 4, 9], hi: [12, 13, 10], voxel: grass, sway_lo: false, sway_hi: true, thin: 0, season: 0, flap: 0 };
        let mut c = PaddedChunk::new();
        assert_eq!(quads(&c, blade), [3, 3, 1, 1, 3, 3], "in the air: a top, a bottom, and the sides of three pieces");
        c.set(3, 1, 3, stone);
        assert_eq!(quads(&c, blade), [3, 3, 1, 0, 3, 3], "the bottom is against the stone");
        // Stone beside its lowest voxel hides that piece's side.
        c.set(4, 2, 3, stone);
        assert_eq!(quads(&c, blade), [2, 3, 1, 0, 3, 3]);
        // Leaves that fall hide nothing.
        let mut c = PaddedChunk::new();
        c.set(3, 1, 3, leaf);
        assert_eq!(quads(&c, blade), [3, 3, 1, 1, 3, 3]);
        // A skin's inset face is not against what the box is against.
        let mut c = PaddedChunk::new();
        c.set(3, 3, 3, stone);
        let skin = PlantBox { lo: [8, 7, 8], hi: [12, 8, 12], voxel: stone, sway_lo: false, sway_hi: false, thin: 1 << 2, season: 0, flap: 0 };
        assert_eq!(quads(&c, skin), [1; 6]);
        assert_eq!(quads(&c, PlantBox { thin: 0, ..skin }), [1, 1, 0, 1, 1, 1]);
    }

    #[test]
    fn a_cut_blade_bends_in_one_piece() {
        let (mut m, _, _) = mats();
        let grass = m.add(Material { color: [60, 160, 60], kind: Kind::Solid, jitter: 0, emissive: false, plant: plant::GRASS });
        let blade = PlantBox { lo: [9, 4, 9], hi: [10, 13, 10], voxel: grass, sway_lo: false, sway_hi: true, thin: 0, season: 0, flap: 0 };
        let mut mesh = ChunkMesh::default();
        add_plants(&mut mesh, [blade].iter(), &PaddedChunk::new(), [0, 0, 0], &m, &[]);
        for v in &mesh.opaque {
            let y = eighths(v)[1];
            assert_eq!((v.b >> 24) as u8 & flags::FOLIAGE != 0, y > 8, "all but the foot moves, at {y} eighths");
        }
    }

    #[test]
    fn ground_keeps_its_face_under_foliage() {
        let (mut m, stone, _) = mats();
        let leaf = m.add(Material {
            color: [40, 120, 40],
            kind: Kind::Foliage,
            jitter: 0,
            emissive: false,
            plant: 0,
        });
        let mut c = PaddedChunk::new();
        c.set(5, 6, 7, stone);
        c.set(5, 7, 7, leaf);
        let mesh = mesh_chunk(&c, &m);
        // Stone keeps all six faces; the leaf loses only the one against the stone.
        assert_eq!(mesh.opaque.len(), (6 + 5) * 4);
    }

    #[test]
    fn opaque_quads_are_grouped_by_face() {
        let (m, stone, _) = mats();
        let mut c = PaddedChunk::new();
        c.set(5, 6, 7, stone);
        c.set(9, 9, 9, stone);
        let mesh = mesh_chunk(&c, &m);
        assert_eq!(mesh.faces[..7], [0, 2, 4, 6, 8, 10, 12]);
        assert_eq!(mesh.faces[12], 12, "no plants were added");
        for face in 0..6 {
            for v in &mesh.opaque[mesh.faces[face] as usize * 4..mesh.faces[face + 1] as usize * 4] {
                assert_eq!((v.a >> 18) & 7, face as u32);
            }
        }
    }

    #[test]
    fn flat_floor_merges_to_one_top_quad() {
        let (m, stone, _) = mats();
        let mut c = PaddedChunk::new();
        // Floor through the border too, so there are no side faces or edge occlusion.
        for x in 0..PAD {
            for z in 0..PAD {
                c.column_mut(x, z)[..11].fill(stone);
            }
        }
        let mesh = mesh_chunk(&c, &m);
        assert_eq!(mesh.opaque.len(), 4);
        for v in &mesh.opaque {
            assert_eq!(pos(v)[1], 10);
            assert_eq!((v.a >> 18) & 7, 2);
        }
    }

    #[test]
    fn quads_wind_counter_clockwise_from_outside() {
        let (m, stone, _) = mats();
        let mut c = PaddedChunk::new();
        c.set(10, 10, 10, stone);
        let mesh = mesh_chunk(&c, &m);
        let normals = [
            [1, 0, 0],
            [-1, 0, 0],
            [0, 1, 0],
            [0, -1, 0],
            [0, 0, 1],
            [0, 0, -1],
        ];
        for quad in mesh.opaque.chunks(4) {
            let p: Vec<[i32; 3]> = quad.iter().map(|v| pos(v).map(|c| c as i32)).collect();
            let e1: Vec<i32> = (0..3).map(|i| p[1][i] - p[0][i]).collect();
            let e2: Vec<i32> = (0..3).map(|i| p[2][i] - p[0][i]).collect();
            let cross = [
                e1[1] * e2[2] - e1[2] * e2[1],
                e1[2] * e2[0] - e1[0] * e2[2],
                e1[0] * e2[1] - e1[1] * e2[0],
            ];
            let face = ((quad[0].a >> 18) & 7) as usize;
            assert_eq!(cross, normals[face], "face {face}");
        }
    }

    #[test]
    fn step_darkens_the_inside_corner() {
        let (m, stone, _) = mats();
        let mut c = PaddedChunk::new();
        for x in 0..PAD {
            for z in 0..PAD {
                let top = if x >= 17 { 12 } else { 11 };
                c.column_mut(x, z)[..top].fill(stone);
            }
        }
        let mesh = mesh_chunk(&c, &m);
        // Lower floor's top faces next to the wall (local x = 15..16) must carry AO < 3
        // on the wall-side edge and full light on the open edge.
        let mut saw_dark = false;
        for v in &mesh.opaque {
            let p = pos(v);
            let face = (v.a >> 18) & 7;
            let ao = (v.a >> 21) & 3;
            if face == 2 && p[1] == 10 {
                if p[0] == 16 {
                    assert!(ao < 3, "vertex against the wall should be occluded");
                    saw_dark = true;
                }
                if p[0] <= 14 {
                    assert_eq!(ao, 3);
                }
            }
        }
        assert!(saw_dark);
    }

    /// Timing check, not a correctness test: `cargo test -p voxel -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bench_rolling_terrain() {
        let (m, stone, _) = mats();
        let mut c = PaddedChunk::new();
        for x in 0..PAD {
            for z in 0..PAD {
                let h = 16.0 + 6.0 * ((x as f32 * 0.3).sin() + (z as f32 * 0.23).cos());
                c.column_mut(x, z)[..h as usize].fill(stone);
            }
        }
        let t = std::time::Instant::now();
        let mut quads = 0;
        for _ in 0..200 {
            quads = mesh_chunk(&c, &m).opaque.len() / 4;
        }
        println!("{quads} quads, {:.3} ms per chunk", t.elapsed().as_secs_f64() * 5.0);
    }

    #[test]
    fn water_surface_goes_to_the_water_mesh() {
        let (m, stone, water) = mats();
        let mut c = PaddedChunk::new();
        for x in 0..PAD {
            for z in 0..PAD {
                let col = c.column_mut(x, z);
                col[..5].fill(stone);
                col[5..9].fill(water);
            }
        }
        let mesh = mesh_chunk(&c, &m);
        assert_eq!(mesh.water.len(), 4, "one merged surface quad");
        // The lake bed under water is still drawn (it is visible through the water).
        assert_eq!(mesh.opaque.len(), 4);
        assert_eq!((mesh.water[0].b >> 24) as u8 & flags::WATER, flags::WATER);
    }

    #[test]
    fn currents_survive_packing() {
        assert_eq!(pack_current(0.0, 0.0, false), 0, "still water carries no bits");
        assert_eq!(unpack_current(pack_current(0.0, 0.0, true)), Current { x: 0.0, z: 0.0, river: true });
        for (x, z) in [(1.0, 0.0), (0.0, -0.5), (-1.3, 0.7), (0.3, 0.2), (1.5, -1.5), (-0.15, -0.25)] {
            let c = unpack_current(pack_current(x, z, true));
            assert!(c.river);
            let speed = f32::hypot(x, z).min(MAX_CURRENT);
            let back = c.x.hypot(c.z);
            assert!((back - speed).abs() < 0.2 * speed.max(0.5), "speed {speed} came back as {back}");
            let turn = (c.z.atan2(c.x) - z.atan2(x)).rem_euclid(std::f32::consts::TAU);
            let turn = turn.min(std::f32::consts::TAU - turn);
            assert!(turn <= std::f32::consts::PI / 32.0 + 1e-4, "({x}, {z}) turned {turn} rad");
        }
        let bits = pack_current(-2.0, 1.0, true);
        assert_eq!(bits & ((1 << CURRENT_SHIFT) - 1), 0, "only bits 23..=31 are used");
    }

    /// A lake 6 voxels deep with a river running +X along z = 10..=12 (padded).
    fn river_chunk() -> (Materials, PaddedChunk, Vec<Current>) {
        let (m, stone, water) = mats();
        let mut c = PaddedChunk::new();
        let mut currents = vec![Current::default(); PAD * PAD];
        for x in 0..PAD {
            for z in 0..PAD {
                let col = c.column_mut(x, z);
                col[..5].fill(stone);
                col[5..11].fill(water);
                if (10..=12).contains(&z) {
                    currents[x * PAD + z] = Current { x: 1.2, z: 0.0, river: true };
                }
            }
        }
        (m, c, currents)
    }

    #[test]
    fn still_water_merges_and_carries_no_current() {
        let (m, c, _) = river_chunk();
        let still = vec![Current::default(); PAD * PAD];
        for mesh in [mesh_chunk(&c, &m), mesh_chunk_with_currents(&c, &m, &still)] {
            assert_eq!(mesh.water.len(), 4);
            assert!(mesh.water.iter().all(|v| v.a >> CURRENT_SHIFT == 0));
        }
    }

    #[test]
    fn river_surface_carries_its_current_smoothly() {
        let (m, c, currents) = river_chunk();
        let mesh = mesh_chunk_with_currents(&c, &m, &currents);
        // Padded z 9..=13 lie beside or on the river: 5 rows of 32 single voxels. The
        // rest of the lake merges into a quad either side.
        assert_eq!(mesh.water.len() / 4, 5 * 32 + 2);
        let mut at: std::collections::HashMap<[u32; 3], u32> = Default::default();
        for v in &mesh.water {
            let bits = v.a >> CURRENT_SHIFT;
            assert_eq!(*at.entry(pos(v)).or_insert(bits), bits, "faces disagree at corner {:?}", pos(v));
            let cur = unpack_current(v.a);
            // Local z is padded z - 1. Corners at local z 10 and 11 sit wholly in the river.
            match pos(v)[2] {
                10 | 11 => {
                    assert!(cur.river);
                    assert!((cur.x - 1.2).abs() < 0.2 && cur.z.abs() < 1e-3, "{cur:?}");
                }
                9 | 12 => {
                    assert!(cur.river, "half river, half lake counts as river");
                    assert!(cur.x > 0.3 && cur.x < 0.9, "the bank side runs slower: {cur:?}");
                }
                _ => assert_eq!(cur, Current::default()),
            }
        }
    }

    /// The chunk at `origin` (in chunks) of a world given voxel by voxel, border and all.
    fn chunk_of(world: impl Fn(i32, i32, i32) -> Voxel, origin: [i32; 3]) -> PaddedChunk {
        let mut c = PaddedChunk::new();
        for x in 0..PAD {
            for z in 0..PAD {
                for y in 0..PAD {
                    let at = |i: usize, p: usize| origin[i] * CHUNK + p as i32 - 1;
                    c.set(x, y, z, world(at(0, x), at(1, y), at(2, z)));
                }
            }
        }
        c
    }

    /// The parts of a vertex of a sheet of falling water: way, code, foot, flat.
    fn sheet(v: &Vertex) -> (u32, u32, u32, bool) {
        assert_eq!(v.a >> 18 & 7, 6);
        (v.a >> 21 & 3, v.a >> 23 & 63, v.a >> 29 & 3, v.a >> 31 != 0)
    }

    fn is_sheet(v: &Vertex) -> bool {
        v.a >> 18 & 7 == 6
    }

    #[test]
    fn water_falls_from_the_lip_to_the_water_below() {
        let (m, stone, water) = mats();
        // A pool ten voxels up, stepping down two voxels toward +X.
        let world = |x: i32, y: i32, _z: i32| match y {
            ..4 => stone,
            4..10 if x < 16 => water,
            4..8 => water,
            _ => crate::AIR,
        };
        let mesh = mesh_chunk(&chunk_of(world, [0; 3]), &m);
        assert!(mesh.water.iter().all(|v| matches!(v.a >> 18 & 7, 2 | 6)), "only surfaces and sheets");
        let sheets: Vec<&Vertex> = mesh.water.iter().filter(|v| is_sheet(v)).collect();
        // Three quarter-voxel rows down the first voxel (the lip is a quarter down), four
        // down the second, in each of 32 columns; no ends to close.
        assert_eq!(sheets.len() / 4, 32 * 7);
        for v in &sheets {
            let (way, code, foot, flat) = sheet(v);
            assert_eq!((way, flat), (0, false));
            assert_eq!(pos(v)[0], 16, "on the face the water leaves");
            // Code 1 is the lip, a quarter below the upper surface; 8 the foot, on the
            // lower one. `y` is the voxel boundary at or above the row.
            assert!((1..=8).contains(&code));
            assert_eq!(pos(v)[1], 10 - code / 4, "code {code}");
            assert_eq!(foot, code.saturating_sub(5), "the last three rows near the foot");
        }
        for code in 1..=8 {
            assert!(sheets.iter().any(|v| sheet(v).1 == code), "no row {code}");
        }
        // The surfaces say where they meet the fall: the upper bends over at its lip, the
        // lower foams at the foot, and neither a voxel further off.
        for v in mesh.water.iter().filter(|v| !is_sheet(v)) {
            let [x, y, _] = pos(v);
            let marks = v.a >> 21 & 3;
            match (y, x) {
                (10, 16) => assert_eq!(marks, 2, "lip"),
                (8, 16) => assert_eq!(marks, 1, "foot"),
                _ => assert_eq!(marks, 0, "at {:?}", pos(v)),
            }
        }
        // Away from the fall the surfaces still merge: a row of single voxels each side.
        assert_eq!(mesh.water.iter().filter(|v| !is_sheet(v)).count() / 4, 2 * 32 + 2);
    }

    #[test]
    fn a_fall_carries_on_through_the_chunk_below() {
        let (m, stone, water) = mats();
        // A stream on a cliff top (surface the top of voxel 43) over a pool far below
        // (the top of voxel 11): the fall passes out of one chunk into the next.
        let world = |x: i32, y: i32, _z: i32| match (x < 16, y) {
            (true, ..40) | (false, ..10) => stone,
            (true, 40..44) | (false, 10..12) => water,
            _ => crate::AIR,
        };
        let mut falls = Falls::default();
        let upper = mesh_chunk_under_sky(&chunk_of(world, [0, 1, 0]), &m, &[], &[], &[], &mut falls);
        let lower = mesh_chunk_under_sky(&chunk_of(world, [0, 0, 0]), &m, &[], &[], &[], &mut falls);
        assert!(falls.through.is_empty(), "the fall ended in the pool");
        // Each row of the sheet, by its height in the world, with its code and foot.
        let mut rows = std::collections::BTreeMap::new();
        for (base, mesh) in [(32, &upper), (0, &lower)] {
            for v in mesh.water.iter().filter(|v| is_sheet(v)) {
                let (_, code, foot, _) = sheet(v);
                let quarters = if code <= 8 { code & 3 } else { 0 };
                let height = (base + pos(v)[1]) * 4 - quarters;
                assert_eq!(*rows.entry(height).or_insert((code, foot)), (code, foot), "rows differ at {height}");
            }
        }
        // From the lip, a quarter under the top of voxel 43, to the pool at the top of 11.
        let heights: Vec<u32> = rows.keys().copied().collect();
        let expect: Vec<u32> = (12..=43).map(|y| y * 4).chain((168..=175).filter(|q| q % 4 != 0)).collect();
        let mut expect = expect;
        expect.sort();
        assert_eq!(heights, expect);
        assert_eq!(rows[&(12 * 4)], (spill_code(32 * 4), 3), "the foot");
        assert_eq!(rows[&(13 * 4)], (spill_code(31 * 4), 0));
        assert_eq!(rows[&(32 * 4)].0, spill_code(12 * 4), "where the chunks meet");
        assert!(lower.water.iter().filter(|v| is_sheet(v)).count() / 4 == 32 * 20);
        // Meshed on its own the lower chunk knows nothing of the fall.
        let alone = mesh_chunk(&chunk_of(world, [0, 0, 0]), &m);
        assert!(!alone.water.iter().any(is_sheet));
    }

    #[test]
    fn falls_agree_across_chunk_borders_and_close_their_open_ends() {
        let (m, stone, water) = mats();
        // A step running across the border between two chunks, and stopping short of the
        // far side of the second, where the upper pool ends against a bank.
        let world = |x: i32, y: i32, z: i32| match y {
            ..4 => stone,
            4..10 if z < 20 && x < 50 => water,
            4..10 if z < 20 => stone,
            4..8 => water,
            _ => crate::AIR,
        };
        let west = mesh_chunk(&chunk_of(world, [0; 3]), &m);
        let east = mesh_chunk(&chunk_of(world, [1, 0, 0]), &m);
        // Surface corners on the shared border carry the same marks from either side.
        let marks = |mesh: &ChunkMesh, x: u32| {
            let mut at = std::collections::BTreeMap::new();
            for v in mesh.water.iter().filter(|v| !is_sheet(v) && pos(v)[0] == x) {
                let bits = v.a >> 21 & 3;
                assert_eq!(*at.entry((pos(v)[1], pos(v)[2])).or_insert(bits), bits);
            }
            at
        };
        let (a, b) = (marks(&west, 32), marks(&east, 0));
        assert_eq!(a, b);
        assert_eq!((a[&(10, 20)], a[&(8, 20)], a[&(10, 19)], a[&(8, 21)]), (2, 1, 0, 0));
        // So do the rows of the sheet where the two halves of it meet.
        let rows = |mesh: &ChunkMesh, x: u32| {
            let mut at: Vec<(u32, u32, u32)> = mesh.water.iter().filter(|v| is_sheet(v) && pos(v)[0] == x).map(|v| (pos(v)[1], pos(v)[2], sheet(v).1)).collect();
            at.sort();
            at.dedup();
            at
        };
        assert_eq!(rows(&west, 32), rows(&east, 0));
        assert_eq!(rows(&west, 32).len(), 8);
        // No end is closed at the border; the one end that stands open, where the pool
        // stops and the lower water runs on past, is.
        assert!(!west.water.iter().any(|v| is_sheet(v) && sheet(v).3));
        let caps: Vec<&Vertex> = east.water.iter().filter(|v| is_sheet(v) && sheet(v).3).collect();
        assert_eq!(caps.len(), 7 * 2, "two corners on the face in each quad of the end");
        assert!(caps.iter().all(|v| pos(v)[0] == 50 - 32 && pos(v)[2] == 20));
    }

    #[test]
    fn single_leaf_voxels_name_their_voxel_from_every_corner() {
        let (mut m, _, _) = mats();
        let leaf = m.add(Material {
            color: [60, 130, 50],
            kind: Kind::Foliage,
            jitter: 0,
            emissive: false,
            plant: plant::DECIDUOUS,
        });
        let mut c = PaddedChunk::new();
        // A row of identical leaves, which would merge into one long quad if they could.
        for x in 4..9 {
            c.set(x, 7, 7, leaf);
        }
        let mesh = mesh_chunk(&c, &m);
        assert_eq!(mesh.opaque.len() / 4, 5 * 4 + 2, "every face of every leaf stays its own quad");
        for quad in mesh.opaque.chunks(4) {
            let voxels: Vec<_> = quad
                .iter()
                .map(|v| {
                    let bits = [(v.a >> 29) & 1, (v.a >> 30) & 1, (v.a >> 31) & 1];
                    let p = pos(v);
                    [p[0] - bits[0], p[1] - bits[1], p[2] - bits[2]]
                })
                .collect();
            assert!(voxels.iter().all(|v| *v == voxels[0]), "corners disagree: {voxels:?}");
            assert_eq!(voxels[0][1], 6);
            assert_eq!(voxels[0][2], 6);
            assert!((3..8).contains(&voxels[0][0]));
            assert_eq!((quad[0].b >> 24) as u8 >> flags::PLANT_SHIFT, plant::DECIDUOUS);
        }
    }
}
