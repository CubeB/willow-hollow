//! Greedy mesher with baked per-vertex ambient occlusion.

use bytemuck::{Pod, Zeroable};

use crate::{CHUNK, Kind, Materials, PAD, PaddedChunk, flags};

/// One quad corner, packed into 8 bytes.
///
/// `a`: x (6 bits) | y (6) << 6 | z (6) << 12 | face (3) << 18 | ao (2) << 21
///      | quarter-voxel fractions of x, y, z (2 bits each) << 23, 25, 27
/// `b`: r | g << 8 | b << 16 | flags << 24
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
}

impl ChunkMesh {
    pub fn is_empty(&self) -> bool {
        self.opaque.is_empty() && self.water.is_empty()
    }
}

const N: usize = CHUNK as usize;

/// A visible face awaiting merging. Zero means "no face"; equal non-zero keys can merge.
/// bit 0: present, bit 1: water, bit 2: mergeable (uniform AO),
/// bits 8..32: colour, 32..40: flags, 40..48: four 2-bit AO values.
type FaceKey = u64;

thread_local! {
    /// Face keys for one direction: `N` slices of `N * N` cells. Greedy merging zeroes
    /// every cell it consumes, so the buffer is all zero again after each direction.
    static FACE_MASK: std::cell::RefCell<Vec<FaceKey>> = const { std::cell::RefCell::new(Vec::new()) };
}

pub fn mesh_chunk(chunk: &PaddedChunk, mats: &Materials) -> ChunkMesh {
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
    let voxels = chunk.voxels();
    for x in 0..PAD {
        for z in 0..PAD {
            let col = &voxels[(x * PAD + z) * PAD..][..PAD];
            let (mut s, mut w, mut l) = (0u64, 0u64, 0u64);
            for (y, &v) in col.iter().enumerate() {
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
                        let key = 1
                            | (uniform as u64) << 2
                            | color_bits(m.color) << 8
                            | (fl as u64) << 32
                            | ao_bits << 40;
                        let (slice, u, v) = (p[n_axis] - 1, p[u_axis] - 1, p[v_axis] - 1);
                        mask[(slice * N + v) * N + u] = key;
                        used_rows[slice] |= 1 << v;
                        used_cols[slice] |= 1 << u;
                    }

                    while water_faces != 0 {
                        let y = water_faces.trailing_zeros() as usize;
                        water_faces &= water_faces - 1;
                        let p = [x, y, z];
                        let m = mats.get(voxels[PaddedChunk::idx(x, y, z)]);
                        let fl = flags::WATER
                            | ((m.jitter << flags::JITTER_SHIFT) & flags::JITTER_MASK);
                        // Water has no AO and always merges.
                        let key =
                            1 | 2 | 4 | color_bits(m.color) << 8 | (fl as u64) << 32 | 0xFF << 40;
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
                        emit_quad(&mut mesh, key, face, [n_axis, u_axis, v_axis], s, u, v, w, h);
                        u += w;
                    }
                }
            }
        }
        mesh.faces[6..].fill((mesh.opaque.len() / 4) as u32);
    });
    mesh
}

/// Appends plants to a chunk's mesh. `origin` is the chunk's corner in world cells, and
/// every box must start inside the chunk.
pub fn add_plants<'a>(mesh: &mut ChunkMesh, boxes: impl Iterator<Item = &'a PlantBox> + Clone, origin: [i32; 3], mats: &Materials) {
    for face in 0..6usize {
        mesh.faces[6 + face] = (mesh.opaque.len() / 4) as u32;
        // Undersides are never seen from where a person stands.
        if face == 3 {
            continue;
        }
        let n_axis = face / 2;
        let positive = face.is_multiple_of(2);
        let (u_axis, v_axis) = ((n_axis + 1) % 3, (n_axis + 2) % 3);
        for b in boxes.clone() {
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
                for axis in 0..3 {
                    let local = cell[axis] - origin[axis];
                    p[axis] = local.div_euclid(CELLS) as u32;
                    frac[axis] = local.rem_euclid(CELLS) as u32;
                }
                // Darker where a rooted stem meets the ground.
                let ao = if !at_top && !b.sway_lo { 1 } else { 3 };
                let sways = if at_top { b.sway_hi } else { b.sway_lo };
                let fl = ((m.jitter << flags::JITTER_SHIFT) & flags::JITTER_MASK) | if sways { flags::FOLIAGE } else { 0 };
                Vertex {
                    a: p[0] | p[1] << 6 | p[2] << 12 | (face as u32) << 18 | ao << 21 | frac[0] << 23 | frac[1] << 25 | frac[2] << 27,
                    b: color_bits(m.color) as u32 | (fl as u32) << 24,
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
        Vertex {
            a: p[0] | p[1] << 6 | p[2] << 12 | (face as u32) << 18 | ao(i) << 21,
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
        });
        let water = m.add(Material {
            color: [40, 90, 200],
            kind: Kind::Water,
            jitter: 0,
            emissive: false,
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

    #[test]
    fn ground_keeps_its_face_under_foliage() {
        let (mut m, stone, _) = mats();
        let leaf = m.add(Material {
            color: [40, 120, 40],
            kind: Kind::Foliage,
            jitter: 0,
            emissive: false,
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
}
