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
/// any face, with plant kind 0. All nine bits are zero on still water.
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
    /// near or far face always does. A box with bit 3 (its underside) is also drawn from
    /// below, which other plants are not.
    pub thin: u8,
}

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
/// bits 8..32: colour, 32..40: flags, 40..48: four 2-bit AO values.
type FaceKey = u64;

thread_local! {
    /// Face keys for one direction: `N` slices of `N * N` cells. Greedy merging zeroes
    /// every cell it consumes, so the buffer is all zero again after each direction.
    static FACE_MASK: std::cell::RefCell<Vec<FaceKey>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Meshes a chunk whose water is all still.
pub fn mesh_chunk(chunk: &PaddedChunk, mats: &Materials) -> ChunkMesh {
    mesh_chunk_with_currents(chunk, mats, &[])
}

/// Meshes a chunk, giving its water surface the current in each column. `currents` is
/// indexed `x * PAD + z` in padded coordinates, like the voxels, or empty if all the
/// water is still.
pub fn mesh_chunk_with_currents(chunk: &PaddedChunk, mats: &Materials, currents: &[Current]) -> ChunkMesh {
    mesh_chunk_under_sky(chunk, mats, currents, &[])
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
/// [`flags::SKY`], which is where snow settles.
pub fn mesh_chunk_under_sky(chunk: &PaddedChunk, mats: &Materials, currents: &[Current], covered: &[bool]) -> ChunkMesh {
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
                    let mut water_faces = w & !(ns | nw) & INTERIOR;

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
                        // Leaves that fall one by one stay one voxel each, so the shader can tell
                        // which voxel every corner of a face belongs to.
                        let single = m.plant == plant::DECIDUOUS || m.plant == plant::AUTUMN;
                        let key = 1
                            | (uniform && !single) as u64 * 4
                            | leaf_color_bits(m.color, m.plant) << 8
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
                    let running = face == 2
                        && water_faces != 0
                        && !currents.is_empty()
                        && (x - 1..=x + 1).any(|i| (z - 1..=z + 1).any(|j| currents[i * PAD + j].river));
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
                            | (if running { 8 } else { 4 })
                            | color_bits(m.color) << 8
                            | (fl as u64) << 32
                            | 0xFF << 40;
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
                        emit_quad(&mut mesh, key, face, [n_axis, u_axis, v_axis], s, u, v, w, h, currents);
                        u += w;
                    }
                }
            }
        }
        mesh.faces[6..].fill((mesh.opaque.len() / 4) as u32);
    });
    mesh
}

/// Whether the shader may drop a plant of this kind one voxel at a time (season.wgsl's
/// `season_gone`, and grass buried in snow), which it decides at each corner of a face
/// from the voxel the corner names.
fn droppable(kind: u8) -> bool {
    matches!(kind, plant::DECIDUOUS | plant::AUTUMN | plant::GRASS | plant::FLOWER)
}

/// A plant box cut at voxel boundaries, so that every corner of every face names the
/// same voxel. Otherwise the shader could drop some corners of a face and not others,
/// and the face would be torn into a long sliver toward the middle of the screen. Grass
/// is decided by column alone, so it is only cut across x and z.
fn split_at_voxels(b: &PlantBox, mats: &Materials, out: &mut Vec<PlantBox>) {
    let kind = mats.get(b.voxel).plant;
    if !droppable(kind) {
        out.push(*b);
        return;
    }
    let axes = if kind == plant::GRASS { [true, false, true] } else { [true; 3] };
    let start = out.len();
    out.push(*b);
    for axis in 0..3 {
        if !axes[axis] {
            continue;
        }
        let mut k = start;
        while k < out.len() {
            let p = out[k];
            // The first voxel boundary strictly inside the box.
            let cut = (p.lo[axis].div_euclid(CELLS) + 1) * CELLS;
            if cut < p.hi[axis] {
                let (mut a, mut c) = (p, p);
                a.hi[axis] = cut;
                c.lo[axis] = cut;
                // Each piece keeps the skin insets of the faces it still has.
                a.thin &= !(1 << (2 * axis));
                c.thin &= !(1 << (2 * axis + 1));
                out[k] = a;
                out.push(c);
            } else {
                k += 1;
            }
        }
    }
}

/// Appends plants to a chunk's mesh. `origin` is the chunk's corner in world cells, and
/// every box must start inside the chunk; any box reaching outside what a vertex can
/// hold is left out.
pub fn add_plants<'a>(mesh: &mut ChunkMesh, boxes: impl Iterator<Item = &'a PlantBox>, origin: [i32; 3], mats: &Materials) {
    let reach = 63 * CELLS;
    let mut cut = Vec::new();
    for b in boxes {
        if (0..3).all(|a| b.lo[a] >= origin[a] && b.hi[a] <= origin[a] + reach && b.lo[a] < b.hi[a]) {
            split_at_voxels(b, mats, &mut cut);
        }
    }
    for face in 0..6usize {
        mesh.faces[6 + face] = (mesh.opaque.len() / 4) as u32;
        let n_axis = face / 2;
        let positive = face.is_multiple_of(2);
        let (u_axis, v_axis) = ((n_axis + 1) % 3, (n_axis + 2) % 3);
        for b in &cut {
            // Undersides are never seen from where a person stands, except a skin on a
            // ceiling, which is nothing but its underside.
            if face == 3 && b.thin & 1 << 3 == 0 {
                continue;
            }
            let m = mats.get(b.voxel);
            let n = if positive { b.hi[n_axis] } else { b.lo[n_axis] };
            let corners = [
                (b.lo[u_axis], b.lo[v_axis]),
                (b.hi[u_axis], b.lo[v_axis]),
                (b.hi[u_axis], b.hi[v_axis]),
                (b.lo[u_axis], b.hi[v_axis]),
            ];
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
                let ao = if !at_top && !b.sway_lo { 1 } else { 3 };
                let sways = if at_top { b.sway_hi } else { b.sway_lo };
                // Plants are taken to stand in the open, where snow settles on them.
                let fl = ((m.jitter << flags::JITTER_SHIFT) & flags::JITTER_MASK)
                    | flags::SKY
                    | if sways { flags::FOLIAGE } else { 0 }
                    | m.plant << flags::PLANT_SHIFT;
                // A corner on the box's far edge, exactly on a voxel boundary, belongs to the
                // voxel before it. With a quarter set, the same bit is a half step instead.
                let mut corner = 0u32;
                for axis in 0..3 {
                    corner |= (((frac[axis] == 0 && cell[axis] == b.hi[axis]) || half[axis]) as u32) << axis;
                }
                Vertex {
                    a: p[0] | p[1] << 6 | p[2] << 12 | (face as u32) << 18 | ao << 21 | frac[0] << 23 | frac[1] << 25 | frac[2] << 27 | corner << 29,
                    b: leaf_color_bits(m.color, m.plant) as u32 | (fl as u32) << 24,
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
/// filled in by [`tag_trees`].
#[inline]
fn leaf_color_bits(c: [u8; 3], kind: u8) -> u64 {
    let bits = color_bits(c);
    if kind == plant::DECIDUOUS || kind == plant::AUTUMN { bits & !(TAG_BITS as u64) } else { bits }
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
        let a = first.a;
        let mut voxel = [0i32; 3];
        for (axis, v) in voxel.iter_mut().enumerate() {
            let p = (a >> (6 * axis)) & 63;
            let quarter = (a >> (23 + 2 * axis)) & 3;
            // With no quarter, the corner bit says the corner is on the far side of its voxel.
            let back = if quarter == 0 { (a >> (29 + axis)) & 1 } else { 0 };
            *v = origin[axis] + p as i32 - back as i32;
        }
        let t = (tag(voxel) & 63) as u32;
        let bits = (t & 3) | (t >> 2 & 3) << 8 | (t >> 4 & 3) << 16;
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
) {
    let [n_axis, u_axis, v_axis] = axes;
    let positive = face.is_multiple_of(2);
    let n = s + positive as usize;
    let ao_bits = (key >> 40) & 0xFF;
    let ao = |i: usize| ((ao_bits >> (i * 2)) & 3) as u32;
    let b = ((key >> 8) & 0xFFFF_FFFF) as u32;

    // Corners counter-clockwise seen from +N, matching the AO corner order.
    let corners = [(u, v), (u + w, v), (u + w, v + h), (u, v + h)];
    let vertex = |i: usize| {
        let mut p = [0u32; 3];
        p[n_axis] = n as u32;
        p[u_axis] = corners[i].0 as u32;
        p[v_axis] = corners[i].1 as u32;
        // A single leaf voxel says which side of it each corner is on, so the shader can
        // name the voxel from any corner.
        let kind = ((key >> 32) as u8 & flags::PLANT_MASK) >> flags::PLANT_SHIFT;
        let mut corner = 0u32;
        if kind == plant::DECIDUOUS || kind == plant::AUTUMN {
            corner |= ((p[n_axis] as usize > s) as u32) << n_axis;
            corner |= ((corners[i].0 > u) as u32) << u_axis;
            corner |= ((corners[i].1 > v) as u32) << v_axis;
        }
        // Running water keeps its current where other faces keep fractions and corners.
        let current = if key & 8 != 0 { corner_current(currents, p[0] as usize, p[2] as usize) } else { 0 };
        Vertex {
            a: p[0] | p[1] << 6 | p[2] << 12 | (face as u32) << 18 | ao(i) << 21 | corner << 29 | current,
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
        };
        add_plants(&mut mesh, [plant].iter(), chunk, &m);
        assert_eq!(mesh.opaque.len(), (6 + 5) * 4, "bottom face is skipped");
        assert_eq!(mesh.faces[6], 6);
        assert_eq!(mesh.faces[12], 11);
        assert_eq!(mesh.faces[9], mesh.faces[10], "no downward group");
        for v in &mesh.opaque[6 * 4..] {
            let fine = |shift: u32, frac: u32| ((v.a >> shift) & 63) * 4 + ((v.a >> frac) & 3);
            assert!((9..=10).contains(&fine(0, 23)));
            assert!((5..=11).contains(&fine(6, 25)));
            assert!((2..=3).contains(&fine(12, 27)));
            let top = fine(6, 25) == 11;
            assert_eq!((v.b >> 24) as u8 & flags::FOLIAGE != 0, top, "only the free end sways");
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
            PlantBox { lo: [3, 3, 6], hi: [9, 6, 11], voxel: flower, sway_lo: false, sway_hi: true, thin: 0 },
            PlantBox { lo: [7, 2, 2], hi: [10, 9, 5], voxel: grass, sway_lo: false, sway_hi: true, thin: 0 },
            PlantBox { lo: [-2, 2, 2], hi: [2, 3, 3], voxel: grass, sway_lo: false, sway_hi: true, thin: 0 },
        ];
        let mut mesh = ChunkMesh::default();
        add_plants(&mut mesh, boxes.iter(), chunk, &m);
        assert!(!mesh.opaque.is_empty());
        for quad in mesh.opaque.chunks_exact(4) {
            let kind = (quad[0].b >> 29) as u8 & 7;
            let named: Vec<[i32; 3]> = quad.iter().map(named_voxel).collect();
            for v in &named {
                assert_eq!(v[0], named[0][0], "{named:?}");
                assert_eq!(v[2], named[0][2], "{named:?}");
                if kind == plant::FLOWER {
                    assert_eq!(v[1], named[0][1], "{named:?}");
                }
            }
        }
        // A stone box is never cut.
        let mut mesh = ChunkMesh::default();
        let b = PlantBox { lo: [3, 3, 6], hi: [9, 6, 11], voxel: stone, sway_lo: false, sway_hi: false, thin: 0 };
        add_plants(&mut mesh, [b].iter(), chunk, &m);
        assert_eq!(mesh.opaque.len(), 5 * 4);
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
            };
            let mut mesh = ChunkMesh::default();
            add_plants(&mut mesh, [b].iter(), chunk, &m);
            // A ceiling skin is seen from below, so it keeps its underside.
            let quads = mesh.opaque.len() / 4;
            assert_eq!(quads, if face == 3 { 6 } else { 5 }, "face {face}");
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
    fn only_ceiling_skins_are_drawn_from_below() {
        let (m, stone, _) = mats();
        let skin = |thin| PlantBox { lo: [0, 7, 0], hi: [4, 8, 4], voxel: stone, sway_lo: false, sway_hi: false, thin };
        let mut mesh = ChunkMesh::default();
        add_plants(&mut mesh, [skin(1 << 3)].iter(), [0, 0, 0], &m);
        assert_eq!(mesh.opaque.len() / 4, 6);
        assert_eq!(mesh.faces[9] + 1, mesh.faces[10], "one downward quad");
        let mut mesh = ChunkMesh::default();
        add_plants(&mut mesh, [skin(1 << 2)].iter(), [0, 0, 0], &m);
        assert_eq!(mesh.opaque.len() / 4, 5);
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
