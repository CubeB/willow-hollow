//! Scene renderer. Each frame: shadow maps from the sun, then sky, far view, voxel
//! terrain and water into an HDR target, then bloom and a tonemap pass to whatever the
//! output is (window or screenshot texture).

use std::ops::Range;

use bytemuck::{Pod, Zeroable};
use glam::{DVec3, IVec3, Mat4, Vec2, Vec3, Vec4};
use voxel::{CHUNK, mesh::Vertex};
use worldgen::scale::{VOXELS_PER_METRE, m};
use wgpu::util::DeviceExt;

use crate::{
    arena::MeshSpan,
    camera::Camera,
    env::Environment,
    farview::{FarVertex, FarView},
    particles::ParticleInstance,
    gfx::{DEPTH_FORMAT, Gpu, create_depth},
    impostors::{self, BoxInstance, Impostors},
    profile::GpuTimer,
    terrain::{LEVELS, PLANT_REACH, Streamer},
};

const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Light shafts: how much lit air lies along the view, and the depth it was found for.
const SHAFTS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;
const SHADOW_SIZE: u32 = 2048;
/// How far a low sun moves (as a change in its direction) before the shadow maps follow
/// it: a 20 m tree's shadow then steps about 4 cm. A higher sun is followed every frame
/// (see `Renderer::cascade_matrix`).
const SHADOW_SUN_STEP: f32 = 0.002;
/// The sun height below which each shadow map is laid square to the sun rather than
/// over the ground, where it would be drawn out too long. The nearest holds the detail
/// of shadows close by and gives up the ground early. The farther ones keep to it for
/// longer: on the ground their texels do not move as the sun does, so the long shadows
/// of evening glide. Lower than this they are drawn out so far that shadows wash out.
const LOW_SUN: [f32; CASCADES] = [0.2, 0.11, 0.11];
/// The level the shadow maps are laid over comes in steps of this, and follows the low
/// ground near the camera once that is further than `GROUND_SLACK` from it.
const GROUND_STEP: f32 = m(16.0);
const GROUND_SLACK: f32 = m(12.0);
const CASCADES: usize = 3;
/// View-depth at which each shadow cascade ends.
const CASCADE_ENDS: [f32; CASCADES] = [m(32.0), m(128.0), m(512.0)];
/// The clouds' blocks (see cloud_trace in common.wgsl): how wide each cell of their grid,
/// how tall each block (cubes), how many blocks the tallest heap stands above the cloud
/// base and how many the thickest hangs below it, and how many cells before the grid's
/// pattern comes round again (a power of two).
const CLOUD_CELL: f32 = m(8.0);
const CLOUD_STEP: f32 = m(8.0);
const CLOUD_STEPS: f32 = 14.0;
pub const CLOUD_BELLY: f32 = 2.0;
const CLOUD_PERIOD: i32 = 2048;
/// A view ray looking for a cloud crosses the grid in squares of this many cells (two to
/// the `CLOUD_LEVEL`), going cell by cell only through a square that holds any cloud;
/// and the most squares it crosses.
const CLOUD_LEVEL: u32 = 3;
const CLOUD_COARSE: i32 = 1 << CLOUD_LEVEL;
/// How many levels the clouds' field has, each half the size of the last, down to one cell.
const CLOUD_LEVELS: u32 = CLOUD_PERIOD.trailing_zeros() + 1;
const CLOUD_MARCH: i32 = 64;
/// How many amounts of cover `cloud_edges` holds the field's level for.
const CLOUD_COVERS: usize = 33;
/// How far round the clouds' cycle of forming and clearing (`cloud_mix`) one stretch of
/// sky may be ahead of another, in radians: enough that no two a few kilometres apart
/// keep time.
const CLOUD_TURNS: f32 = 3.0 * std::f32::consts::TAU;

/// The clouds' field, four bytes for each cell of their grid, row by row: two patterns of
/// heaps (r and g), each high where cloud stands first as the cover grows (see
/// cloud_column in common.wgsl), and in b where each stretch of sky is in its cycle of
/// going from the one pattern to the other and back (see `cloud_mix`). So the clouds form
/// and clear as the cycle goes round, each stretch of sky in its own time: a heap of one
/// pattern grows out from its core as one of the other breaks up and goes. Each pattern
/// is of heaps a few hundred metres across with ragged edges, wrapping round at the
/// grid's edge. The field itself never changes, so it is worked out once here and not
/// for every pixel that looks for a cloud; only the cycle moves on.
///
/// Each coarser level holds, for each square of cells, the highest of each pattern in it
/// in r and g, the middle of their places in the cycle in b, and in a how far from that
/// the place of any cell in the square is, so a ray can tell that a square holds no cloud
/// without looking at its cells (see cloud_bound in common.wgsl).
fn cloud_field() -> (Vec<Vec<[u8; 4]>>, [f32; CLOUD_COVERS]) {
    use rayon::prelude::*;
    fn hash(x: i32, y: i32, z: i32) -> f32 {
        let mut h = (x as u32).wrapping_mul(0x8da6b343).wrapping_add((y as u32).wrapping_mul(0xd8163841)).wrapping_add((z as u32).wrapping_mul(0xcb1ab31f));
        h ^= h >> 15;
        h = h.wrapping_mul(0x2c1b3c6d);
        h ^= h >> 12;
        h = h.wrapping_mul(0x297a2d39);
        h ^= h >> 15;
        (h >> 8) as f32 / 16777216.0
    }
    // Smooth noise with a lattice point every `k` cells, wrapping with the grid.
    let lattice = |x: i32, y: i32, k: i32, salt: i32| {
        let n = CLOUD_PERIOD / k;
        let (lx, ly) = (x / k, y / k);
        let s = |f: f32| f * f * (3.0 - 2.0 * f);
        let (ux, uy) = (s(((x - lx * k) as f32 + 0.5) / k as f32), s(((y - ly * k) as f32 + 0.5) / k as f32));
        let (nx, ny) = ((lx + 1) % n, (ly + 1) % n);
        let a = hash(lx, ly, salt) + (hash(nx, ly, salt) - hash(lx, ly, salt)) * ux;
        let b = hash(lx, ny, salt) + (hash(nx, ny, salt) - hash(lx, ny, salt)) * ux;
        a + (b - a) * uy
    };
    // Smooth noise folded about its middle: round bumps with creases between them, as the
    // top of a heap of cumulus has.
    let billow = |x: i32, y: i32, k: i32, salt: i32| (lattice(x, y, k, salt) * 2.0 - 1.0).abs();
    // Where the heaps stand (half a kilometre across, gathered in broader stretches with
    // clearer sky between), then their bulges, then their bumps.
    let heaps = |x: i32, y: i32, salt: i32| {
        let mass = lattice(x, y, 256, salt) * 0.2 + lattice(x, y, 64, salt + 2) * 0.36 + lattice(x, y, 32, salt + 4) * 0.18;
        let bumps = billow(x, y, 16, salt + 6) * 0.15 + billow(x, y, 8, salt + 8) * 0.07;
        ((mass + bumps) * 255.0).round().clamp(0.0, 255.0) as u8
    };
    let n = CLOUD_PERIOD as usize;
    let field: Vec<[u8; 4]> = (0..CLOUD_PERIOD)
        .into_par_iter()
        .flat_map_iter(|y| {
            (0..CLOUD_PERIOD).map(move |x| {
                // The place in the cycle changes slowly across the sky, so the cells of a
                // heap form and clear nearly together and a whole cloud comes and goes.
                let place = lattice(x, y, 256, 51) * 0.6 + lattice(x, y, 128, 53) * 0.4;
                [heaps(x, y, 39), heaps(x, y, 61), (place * 255.0).round() as u8, 0]
            })
        })
        .collect();
    // For each amount of cover, the level that leaves that share of the field above it:
    // over a sample of cells at points all round the cycle, which the sky as a whole keeps
    // to near enough at any one time, its stretches being at every point of it.
    const BINS: usize = 1024;
    let mut counts = vec![0usize; BINS];
    let mut total = 0;
    for k in 0..8 {
        let churn = k as f32 * std::f32::consts::TAU / 8.0;
        for t in field.iter().step_by(7) {
            counts[((cloud_mix(*t, churn) * BINS as f32) as usize).min(BINS - 1)] += 1;
            total += 1;
        }
    }
    let edges = std::array::from_fn(|i| {
        let wanted = (i as f32 / (CLOUD_COVERS - 1) as f32 * total as f32) as usize;
        let mut above = 0;
        let mut bin = BINS - 1;
        while bin > 0 && above + counts[bin] <= wanted {
            above += counts[bin];
            bin -= 1;
        }
        if i == CLOUD_COVERS - 1 { -0.01 } else { (bin + 1) as f32 / BINS as f32 }
    });
    // Coarser levels: the highest of each pattern in the four cells under each, the middle
    // of their places in the cycle, and how far from it the furthest of them is.
    let mut levels = vec![field];
    for l in 1..CLOUD_LEVELS as usize {
        let (side, last) = (n >> l, &levels[l - 1]);
        let mut next = Vec::with_capacity(side * side);
        for y in 0..side {
            for x in 0..side {
                let four = [(0, 0), (1, 0), (0, 1), (1, 1)].map(|(dx, dy)| last[(2 * y + dy) * side * 2 + 2 * x + dx]);
                let top = |c: usize| four.iter().map(|t| t[c]).max().unwrap_or(0);
                let lo = four.iter().map(|t| t[2] as i32 - t[3] as i32).min().unwrap_or(0);
                let hi = four.iter().map(|t| t[2] as i32 + t[3] as i32).max().unwrap_or(0);
                let mid = (lo + hi + 1) / 2;
                next.push([top(0), top(1), mid.clamp(0, 255) as u8, (hi - mid).max(mid - lo).clamp(0, 255) as u8]);
            }
        }
        levels.push(next);
    }
    (levels, edges)
}

/// The value of the clouds' field in a cell of it (`texel`, as the texture holds it) when
/// the clouds' cycle has come round to `churn` radians: part of the way from the one
/// pattern of heaps to the other, as far as this stretch of sky has got. As cloud_mix in
/// common.wgsl.
fn cloud_mix(texel: [u8; 4], churn: f32) -> f32 {
    let [a, b, place] = [texel[0], texel[1], texel[2]].map(|v| v as f32 / 255.0);
    let t = 0.5 - 0.5 * (churn + place * CLOUD_TURNS).cos();
    a + (b - a) * t
}

/// The level of the clouds' field that this much cover (0..1) leaves its edge at.
fn cloud_edge(edges: &[f32; CLOUD_COVERS], cover: f32) -> f32 {
    let at = cover.clamp(0.0, 1.0) * (CLOUD_COVERS - 1) as f32;
    let i = (at as usize).min(CLOUD_COVERS - 2);
    edges[i] + (edges[i + 1] - edges[i]) * (at - i as f32)
}
/// Entries in the globals' tables of the colours small plants give the ground they stand
/// on from afar (see `wash` in terrain.wgsl, and `worldgen::features::plants::washes`).
const WASH_SLOTS: usize = 64;

/// Those tables: for the wash numbered `i`, its leaves' colour (linear) and its flowers',
/// each with its plant kind, which the seasons go by, in the whole part of `w`; in the
/// rest of `w`, half of how much of the ground's colour the plant takes the place of, and
/// half the flowers' share of it. Above the three bits of the flowers' kind is the row
/// of the table of seasons they keep (`plants::seasons`), 0 with none of their own.
fn wash_tables() -> ([[f32; 4]; WASH_SLOTS], [[f32; 4]; WASH_SLOTS]) {
    let mats = worldgen::materials::build();
    let all = worldgen::features::plants::washes();
    assert!(all.len() < WASH_SLOTS, "more washes than the globals hold");
    let linear = |c: [f32; 3]| c.map(|v| (v / 255.0).powf(2.2));
    let colour = |v: voxel::Voxel| mats.get(v).color.map(|c| c as f32);
    let (mut body, mut bloom) = ([[0.0; 4]; WASH_SLOTS], [[0.0; 4]; WASH_SLOTS]);
    for (i, w) in all.iter().enumerate() {
        let (a, b) = (colour(w.body.0), colour(w.body.1));
        let [r, g, bl] = linear(std::array::from_fn(|k| a[k] + (b[k] - a[k]) * w.body.2));
        body[i + 1] = [r, g, bl, mats.get(w.body.0).plant as f32 + w.cover.clamp(0.0, 1.0) * 0.5];
        let [r, g, bl] = linear(colour(w.bloom.0));
        let kind = mats.get(w.bloom.0).plant as u32 | (w.season as u32) << 3;
        bloom[i + 1] = [r, g, bl, kind as f32 + w.bloom.1.clamp(0.0, 1.0) * 0.5];
    }
    (body, bloom)
}

/// How many of the shadow cascades, nearest first, include small plants.
const PLANT_SHADOW_CASCADES: usize = 2;
/// A cached shadow map is redrawn early once the view has turned this far from it
/// (the cosine of about ten degrees).
const CASCADE_SWING: f32 = 0.985;
/// Or once the camera has travelled this share of the cascade's depth.
const CASCADE_DRIFT: f64 = 0.06;
/// Sun heights above which the far cascade can do with only the tops of things.
const HIGH_SUN: f32 = 0.4;
/// The faintest light (in the brightest of its channels) that terrain casts shadows in.
/// A full moon in a clear sky is some thirty times this.
const MIN_SHADOW_LIGHT: f32 = 0.005;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    cam_right: [f32; 4],
    cam_up: [f32; 4],
    cam_fwd: [f32; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    sky_zenith: [f32; 4],
    sky_horizon: [f32; 4],
    fog: [f32; 4],
    cam_wrapped: [f32; 4],
    mist: [f32; 4],
    cascades: [f32; 4],
    wind: [f32; 4],
    /// Nearest glowing particles (fireflies): camera-relative xyz, glow in w.
    lights: [[f32; 4]; 4],
    /// The nearest lanterns: camera-relative xyz, 1 in w (0 = unused).
    lamps: [[f32; 4]; 4],
    /// x: the date, 0..1 from the spring equinox. y: snow cover. z: ice on water. w: the
    /// summit snowline, metres.
    season: [f32; 4],
    /// x: how wet the ground is. y: how much more snow the hills hold this winter.
    weather: [f32; 4],
    /// rgb: the horizon toward the sun. w: how far into dusk or dawn the sky is.
    sky_glow: [f32; 4],
    /// xyz: toward the sun itself, by night too.
    sun_pos: [f32; 4],
    /// x: haze per voxel at sea level. y: the height it thins over. w: haziness, 0..1.
    air: [f32; 4],
    /// Each biome's ground, by biome id: x snow at sea level, y ice on water, z how much
    /// more snow the hills hold, w the summit snowline in metres (see ground.wgsl).
    snow: [[f32; 4]; SNOW_SLOTS],
    /// The colours small plants give the ground from afar (see `wash_tables`).
    wash_body: [[f32; 4]; WASH_SLOTS],
    wash_bloom: [[f32; 4]; WASH_SLOTS],
    /// The seasons of the plants that have their own: `plants::seasons`.
    flowering: [[f32; 4]; SEASON_SLOTS],
}

/// Rows in the table of seasons, which the shaders keep as 64.
const SEASON_SLOTS: usize = worldgen::features::plants::SEASON_SLOTS;
const _: () = assert!(SEASON_SLOTS == 64);

/// Entries in the globals' snow table: every biome, rounded up.
const SNOW_SLOTS: usize = 24;
const _: () = assert!(worldgen::biome::Biome::ALL.len() <= SNOW_SLOTS);

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Shadows {
    light_vp: [[[f32; 4]; 4]; CASCADES],
    /// How wide each cascade's texels are as the sun sees them, in voxels.
    texel: [f32; 4],
    /// How far along the sun's rays each cascade's depth runs, in voxels.
    span: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CascadeUniform {
    light_vp: [[f32; 4]; 4],
    season: [f32; 4],
    /// The seasons of the plants that have their own, as in [`Globals`].
    flowering: [[f32; 4]; SEASON_SLOTS],
}

/// What one cascade's shadow map was last drawn from.
#[derive(Clone, Copy)]
struct CascadeView {
    /// Camera-relative, for the camera as it was then.
    light_vp: Mat4,
    texel: f32,
    span: f32,
    camera: DVec3,
    fwd: Vec3,
}

/// The four lanterns and fires nearest the camera, relative to it, each with how
/// brightly it burns (1 for a lantern).
fn nearest_lamps(lamps: &[(DVec3, f32)], camera: DVec3) -> [[f32; 4]; 4] {
    let mut best = [([0.0f32; 4], f64::MAX); 4];
    for &(p, bright) in lamps {
        let d = p - camera;
        let d2 = d.length_squared();
        if d2 < best[3].1 {
            best[3] = ([d.x as f32, d.y as f32, d.z as f32, bright], d2);
            best.sort_by(|a, b| a.1.total_cmp(&b.1));
        }
    }
    best.map(|b| b.0)
}

/// The height in metres up to which no ground holds any snow, in any row of the globals'
/// snow table (x snow, z hill snow, w snowline: see snow_depth in ground.wgsl), so that the
/// terrain shader can skip looking for a voxel's biome there. With snow or hill snow in any
/// row, some may lie at any height, and it is far below the world.
fn snow_floor(snow: &[[f32; 4]]) -> f32 {
    if snow.iter().all(|row| row[0] <= 0.0 && row[2] <= 0.0 && !row[3].is_nan()) {
        snow.iter().map(|row| row[3]).fold(f32::INFINITY, f32::min)
    } else {
        -1.0e30
    }
}

/// The few brightest-and-nearest glowing particles, which light the ground around them.
fn nearest_glows(particles: &[ParticleInstance]) -> [[f32; 4]; 4] {
    let mut best = [([0.0f32; 4], f32::MAX); 4];
    for p in particles.iter().filter(|p| p.glow > 6.0) {
        let d = Vec3::from(p.pos).length_squared();
        if d < best[3].1 {
            best[3] = ([p.pos[0], p.pos[1], p.pos[2], p.glow], d);
            best.sort_by(|a, b| a.1.total_cmp(&b.1));
        }
    }
    best.map(|b| b.0)
}

/// Whether cascade `i` takes its turn this frame. The far ones cover so much that they
/// are redrawn in rotation, never two heavy ones in the same frame.
fn cascade_due(i: usize, frame: u64) -> bool {
    match i {
        0 => true,
        1 => frame % 2 == 1,
        _ => frame.is_multiple_of(4),
    }
}

/// Per-chunk (or per-continent) instance data.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Instance {
    /// Origin relative to the camera.
    offset: [f32; 3],
    /// Origin in world voxels (for stable per-voxel hashing).
    origin: [i32; 3],
    /// World voxels per cell of its mesh.
    scale: f32,
    /// The range of horizontal distances this chunk's ring is drawn in.
    band: [f32; 2],
    /// Terrain: a grid of the biomes over it, for the snow (see `GpuChunk::biomes`),
    /// and in the last word the skyline on the sun's side at its four corners (see
    /// `GpuChunk::skylines` and grid_skyline in ground.wgsl).
    biomes: [u32; 8],
    /// Terrain: the lie of the land at its four corners (see `GpuChunk::slopes`), the
    /// normal's x of each in the first word and its z in the second, as signed bytes.
    slopes: [u32; 2],
    /// Terrain: how far the land lies in a hollow or on a ridge at its four corners (see
    /// `GpuChunk::hollows`), a byte each, x fastest; 128 for level. Read only by the
    /// fragment shader, from the instances bound as storage.
    hollows: u32,
}
// The terrain's fragment shader reads instances as words (INSTANCE_WORDS in terrain.wgsl).
const _: () = assert!(size_of::<Instance>() == 20 * 4);

/// The camera's position for the far trees' whole-voxel placement; in the last of
/// `frac`, how far built things are seen (farbuilt::SIGHT).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ImpostorCamera {
    voxel: [i32; 4],
    frac: [f32; 4],
}

/// Most cubes of solid moving models drawn in one frame: room for the crows and their
/// nests as well as the boats, the figure and what is set down.
const PROP_CAPACITY: usize = 2048;

/// Everything one frame draws.
pub struct Scene<'a> {
    pub camera: &'a Camera,
    pub env: &'a Environment,
    pub terrain: &'a Streamer,
    pub far: &'a FarView,
    pub impostors: &'a Impostors,
    pub particles: &'a [ParticleInstance],
    /// Solid moving models (boats, the glider), drawn with the opaque scene.
    pub props: &'a [ParticleInstance],
    /// More of the same that are not seen but cast a shadow: the player's own figure,
    /// seen through its eyes.
    pub shades: &'a [ParticleInstance],
    pub wind: Vec3,
    /// Snow, ice and wetness on the ground, here and in each biome.
    pub ground: crate::season::Places,
    /// Lit lantern voxels near the camera (world voxels), which light the ground around them.
    /// Lanterns and fires: where each is, and how brightly it burns (1 for a lantern).
    pub lamps: &'a [(DVec3, f32)],
    /// Whether to draw light shafts (settings::Settings::shafts).
    pub shafts: bool,
}

#[derive(Default, Clone, Copy)]
pub struct DrawStats {
    pub chunks_drawn: u32,
    pub chunks_culled: u32,
    pub quads_drawn: u64,
    pub far_tiles: u32,
    pub impostor_boxes: u32,
    /// Boxes of what people have built sent to be drawn beyond the voxel rings.
    pub built_boxes: u32,
    pub shadow_draws: [u32; CASCADES],
}

/// A frame's lists of what to draw, kept from one frame to the next so that they are
/// not grown afresh each time.
#[derive(Default)]
struct DrawLists {
    instances: Vec<Instance>,
    opaque: Vec<ChunkDraw>,
    opaque_edge: Vec<ChunkDraw>,
    casters: [Vec<ChunkDraw>; CASCADES],
    /// With each its distance from the camera, squared, for sorting.
    water: Vec<(ChunkDraw, f32)>,
    impostors: Vec<ChunkDraw>,
    /// What people have built, a continent's squares after another's.
    built: Vec<ChunkDraw>,
    /// All of them together, as the indirect buffer holds them.
    draws: Vec<ChunkDraw>,
}

impl DrawLists {
    fn clear(&mut self) {
        self.instances.clear();
        self.opaque.clear();
        self.opaque_edge.clear();
        self.casters.iter_mut().for_each(Vec::clear);
        self.water.clear();
        self.impostors.clear();
        self.built.clear();
        self.draws.clear();
    }
}

/// One corner of a far-tree box as drawn (see the impostor pipeline): its corner and
/// outward normal on one side, the same on the opposite side, and the bare crown's colour.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BoxCorner {
    corner: [f32; 3],
    normal: [f32; 3],
    opposite: [f32; 3],
    opposite_normal: [f32; 3],
    bare: [u8; 4],
}

/// Sides drawn of each far-tree box: one of each opposite pair, the one facing the camera.
const BOX_SIDES: u32 = 3;

/// A chunk draw as the GPU reads it from the indirect buffer: index count, instance
/// count, first index, base vertex, first instance.
type ChunkDraw = [u32; 5];

fn chunk_draw(mesh: &MeshSpan, instance: u32) -> ChunkDraw {
    [mesh.quads * 6, 1, 0, mesh.first_vertex(), instance]
}

/// Adds draws for the face directions marked in `show`, merging neighbouring directions
/// into one draw.
fn push_faces(list: &mut Vec<ChunkDraw>, mesh: &MeshSpan, faces: &[u32], show: [bool; 6], instance: u32) {
    let mut f = 0;
    while f < 6 {
        if !show[f] {
            f += 1;
            continue;
        }
        let first = faces[f];
        while f < 6 && show[f] {
            f += 1;
        }
        let quads = faces[f] - first;
        if quads > 0 {
            list.push([quads * 6, 1, 0, mesh.first_vertex() + first * 4, instance]);
        }
    }
}

/// Issues part of this frame's chunk draws. `indirect` is None on GPUs that cannot set
/// the first instance from an indirect buffer.
fn draw_chunks(pass: &mut wgpu::RenderPass, indirect: Option<&wgpu::Buffer>, draws: &[ChunkDraw], list: Range<usize>) {
    if list.is_empty() {
        return;
    }
    match indirect {
        Some(buffer) => pass.multi_draw_indexed_indirect(buffer, (list.start * size_of::<ChunkDraw>()) as u64, list.len() as u32),
        None => {
            for [indices, count, _, base, instance] in &draws[list] {
                pass.draw_indexed(0..*indices, *base as i32, *instance..*instance + *count);
            }
        }
    }
}

fn indirect_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("chunk draws"),
        size: (capacity * size_of::<ChunkDraw>()) as u64,
        usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// A texture used as a render target and later sampled.
struct Target {
    view: wgpu::TextureView,
}

fn make_target(device: &wgpu::Device, label: &str, w: u32, h: u32, format: wgpu::TextureFormat) -> Target {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: w.max(1),
            height: h.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    Target {
        view: tex.create_view(&Default::default()),
    }
}

pub struct Renderer {
    width: u32,
    height: u32,
    /// The level of the clouds' field each amount of cover leaves its edge at.
    cloud_edges: [f32; CLOUD_COVERS],
    /// The colours small plants give the ground from afar (see `wash_tables`).
    wash: ([[f32; 4]; WASH_SLOTS], [[f32; 4]; WASH_SLOTS]),
    /// The table of seasons (`plants::seasons`), for the globals and the shadow passes.
    flowering: [[f32; 4]; SEASON_SLOTS],
    globals: wgpu::Buffer,
    shadows: wgpu::Buffer,
    scene_bind: wgpu::BindGroup,
    cascade_binds: Vec<(wgpu::Buffer, wgpu::BindGroup)>,
    shadow_views: Vec<wgpu::TextureView>,
    hdr: Target,
    depth: Target,
    bloom_a: Target,
    bloom_b: Target,
    /// The light caught by the air along each view ray, at a quarter of the size (see
    /// shafts.wgsl), and what the pass that lays it over the scene reads.
    shafts: Target,
    shafts_layout: wgpu::BindGroupLayout,
    shafts_bind: wgpu::BindGroup,
    march: wgpu::RenderPipeline,
    lay: wgpu::RenderPipeline,
    image_layout: wgpu::BindGroupLayout,
    depth_layout: wgpu::BindGroupLayout,
    depth_bind: wgpu::BindGroup,
    linear_sampler: wgpu::Sampler,
    sky: wgpu::RenderPipeline,
    /// The northern lights' own uniform, read by the sky alone (see aurora.rs), and its
    /// bind group.
    aurora: wgpu::Buffer,
    aurora_bind: wgpu::BindGroup,
    opaque: wgpu::RenderPipeline,
    opaque_inside: wgpu::RenderPipeline,
    water: wgpu::RenderPipeline,
    far: wgpu::RenderPipeline,
    impostor: wgpu::RenderPipeline,
    /// The same boxes for what people have built (farbuilt.rs).
    impostor_built: wgpu::RenderPipeline,
    impostor_cube: wgpu::Buffer,
    impostor_camera: wgpu::Buffer,
    impostor_bind: wgpu::BindGroup,
    shadow: wgpu::RenderPipeline,
    bright: wgpu::RenderPipeline,
    blur_h: wgpu::RenderPipeline,
    blur_v: wgpu::RenderPipeline,
    tonemap: wgpu::RenderPipeline,
    particles: wgpu::RenderPipeline,
    props: wgpu::RenderPipeline,
    prop_shadow: wgpu::RenderPipeline,
    prop_instances: wgpu::Buffer,
    cube: wgpu::Buffer,
    particle_instances: wgpu::Buffer,
    particle_capacity: usize,
    sea: wgpu::Buffer,
    instances: wgpu::Buffer,
    instance_capacity: usize,
    chunks_layout: wgpu::BindGroupLayout,
    chunks_bind: wgpu::BindGroup,
    quad_indices: wgpu::Buffer,
    quad_capacity: u32,
    indirect: wgpu::Buffer,
    indirect_capacity: usize,
    multi_draw: bool,
    cascade_views: [Option<CascadeView>; CASCADES],
    /// Where the shadow maps take the sun to be. It follows the real sun in small steps:
    /// the maps' texels are fixed in the world only while it holds still, and a sun
    /// that crept every frame would set every shadow's edge crawling.
    shadow_sun: Vec3,
    /// The level the shadow maps are laid over, in voxels above sea level.
    shadow_ground: Option<f32>,
    /// How much of their loaded reach the rings of voxel terrain are drawn out to, less
    /// the higher the camera is over the ground (see `band_scale`), eased toward that.
    band_scale: f32,
    lists: DrawLists,
    frame: u64,
    pub timer: Option<GpuTimer>,
    pub stats: DrawStats,
    start: std::time::Instant,
    /// Seconds to take the time as, in place of the real clock (see `screenshot`).
    pub clock: Option<f32>,
}

fn shader(device: &wgpu::Device, label: &str, source: String) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
}

fn quad_index_buffer(device: &wgpu::Device, quads: u32) -> wgpu::Buffer {
    let mut idx = Vec::with_capacity(quads as usize * 6);
    for q in 0..quads {
        let b = q * 4;
        idx.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
    }
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("quad indices"),
        contents: bytemuck::cast_slice(&idx),
        usage: wgpu::BufferUsages::INDEX,
    })
}

fn instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("instances"),
        size: (capacity * size_of::<Instance>()) as u64,
        // Storage too: the terrain's fragment shader reads a chunk's instance by its index.
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// The terrain's own bind group: the chunks' instances, which its fragment shader reads.
fn chunks_bind_group(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, instances: &wgpu::Buffer) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("chunks"),
        layout,
        entries: &[wgpu::BindGroupEntry { binding: 1, resource: instances.as_entire_binding() }],
    })
}

fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn texture_entry(binding: u32, sample_type: wgpu::TextureSampleType, dim: wgpu::TextureViewDimension) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: dim,
            multisampled: false,
        },
        count: None,
    }
}

fn fullscreen_triangle() -> wgpu::PrimitiveState {
    Default::default()
}

impl Renderer {
    /// `output_format` is the format of the texture the final image is written to.
    pub fn new(gpu: &Gpu, output_format: wgpu::TextureFormat, width: u32, height: u32) -> Self {
        let device = &gpu.device;
        // Shaders work in voxels but know how big one is.
        let common = format!(
            "const VOXELS_PER_METRE: f32 = {VOXELS_PER_METRE:?};\nconst CLOUD_CELL: f32 = {CLOUD_CELL:?};\nconst CLOUD_STEP: f32 = {CLOUD_STEP:?};\nconst CLOUD_STEPS: f32 = {CLOUD_STEPS:?};\nconst CLOUD_PERIOD: i32 = {CLOUD_PERIOD};\nconst CLOUD_MARCH: i32 = {CLOUD_MARCH};\nconst CLOUD_BELLY: f32 = {CLOUD_BELLY:?};\nconst CLOUD_LEVEL: i32 = {CLOUD_LEVEL};\nconst CLOUD_COARSE: i32 = {CLOUD_COARSE};\nconst CLOUD_LEVELS: i32 = {CLOUD_LEVELS};\nconst CLOUD_TURNS: f32 = {CLOUD_TURNS:?};\n{}",
            include_str!("shaders/common.wgsl")
        );

        // --- Uniforms and the scene bind group (globals, shadows) -----------------
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shadows = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadows"),
            size: size_of::<Shadows>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shadow_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow maps"),
            size: wgpu::Extent3d {
                width: SHADOW_SIZE,
                height: SHADOW_SIZE,
                depth_or_array_layers: CASCADES as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let shadow_array_view = shadow_tex.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let shadow_views = (0..CASCADES as u32)
            .map(|layer| {
                shadow_tex.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: layer,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let cloud_size = wgpu::Extent3d {
            width: CLOUD_PERIOD as u32,
            height: CLOUD_PERIOD as u32,
            depth_or_array_layers: 1,
        };
        let cloud_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cloud field"),
            size: cloud_size,
            mip_level_count: CLOUD_LEVELS,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let (cloud_levels, cloud_edges) = cloud_field();
        for (level, texels) in cloud_levels.iter().enumerate() {
            let side = CLOUD_PERIOD as u32 >> level;
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo { mip_level: level as u32, ..cloud_tex.as_image_copy() },
                bytemuck::cast_slice(texels),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(side * 4),
                    rows_per_image: Some(side),
                },
                wgpu::Extent3d { width: side, height: side, depth_or_array_layers: 1 },
            );
        }
        let cloud_view = cloud_tex.create_view(&Default::default());
        let scene_layout_bg = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT),
                uniform_entry(1, wgpu::ShaderStages::FRAGMENT),
                texture_entry(2, wgpu::TextureSampleType::Depth, wgpu::TextureViewDimension::D2Array),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                // The cloud field: read where the trees' light is worked out, too.
                wgpu::BindGroupLayoutEntry {
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ..texture_entry(4, wgpu::TextureSampleType::Float { filterable: false }, wgpu::TextureViewDimension::D2)
                },
            ],
        });
        let scene_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene"),
            layout: &scene_layout_bg,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: shadows.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&shadow_array_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&shadow_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&cloud_view),
                },
            ],
        });
        let depth_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene depth"),
            entries: &[texture_entry(0, wgpu::TextureSampleType::Depth, wgpu::TextureViewDimension::D2)],
        });
        let scene_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene"),
            bind_group_layouts: &[Some(&scene_layout_bg)],
            immediate_size: 0,
        });
        let water_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("water"),
            bind_group_layouts: &[Some(&scene_layout_bg), Some(&depth_layout)],
            immediate_size: 0,
        });

        // --- Shadow cascades -----------------------------------------------------
        let cascade_layout_bg = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cascade"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX)],
        });
        let cascade_binds = (0..CASCADES)
            .map(|_| {
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("cascade"),
                    size: size_of::<CascadeUniform>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("cascade"),
                    layout: &cascade_layout_bg,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    }],
                });
                (buffer, bind)
            })
            .collect();
        let shadow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow"),
            bind_group_layouts: &[Some(&cascade_layout_bg)],
            immediate_size: 0,
        });

        // --- Targets -------------------------------------------------------------
        let hdr = make_target(device, "hdr", width, height, HDR_FORMAT);
        let depth = Target {
            view: create_depth(device, width, height),
        };
        let bloom_a = make_target(device, "bloom a", width / 4, height / 4, HDR_FORMAT);
        let bloom_b = make_target(device, "bloom b", width / 4, height / 4, HDR_FORMAT);
        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let image_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("image"),
            entries: &[
                texture_entry(0, wgpu::TextureSampleType::Float { filterable: true }, wgpu::TextureViewDimension::D2),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                texture_entry(2, wgpu::TextureSampleType::Float { filterable: true }, wgpu::TextureViewDimension::D2),
                // The scene's depth, for the tonemap to find pinholes (see post.wgsl).
                texture_entry(3, wgpu::TextureSampleType::Depth, wgpu::TextureViewDimension::D2),
            ],
        });
        let depth_bind = Self::depth_bind_group(device, &depth_layout, &depth.view);
        let shafts = make_target(device, "shafts", width / 4, height / 4, SHAFTS_FORMAT);
        let shafts_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shafts"),
            entries: &[
                texture_entry(0, wgpu::TextureSampleType::Depth, wgpu::TextureViewDimension::D2),
                texture_entry(1, wgpu::TextureSampleType::Float { filterable: true }, wgpu::TextureViewDimension::D2),
            ],
        });
        let shafts_bind = Self::shafts_bind_group(device, &shafts_layout, &depth.view, &shafts.view);
        let lay_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shafts"),
            bind_group_layouts: &[Some(&scene_layout_bg), Some(&shafts_layout)],
            immediate_size: 0,
        });

        // --- Pipelines -----------------------------------------------------------
        let depth_state = |write: bool, compare: wgpu::CompareFunction| {
            Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(write),
                depth_compare: Some(compare),
                stencil: Default::default(),
                bias: Default::default(),
            })
        };
        let hdr_targets = |blend: Option<wgpu::BlendState>| {
            [Some(wgpu::ColorTargetState {
                format: HDR_FORMAT,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })]
        };

        // The northern lights have a uniform of their own, bound for the sky alone, so
        // they ask nothing of the scene's globals.
        let aurora = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("aurora"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let aurora_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("aurora"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::FRAGMENT)],
        });
        let aurora_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("aurora"),
            layout: &aurora_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: aurora.as_entire_binding() }],
        });
        let sky_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sky"),
            bind_group_layouts: &[Some(&scene_layout_bg), Some(&aurora_layout)],
            immediate_size: 0,
        });
        let sky_module = shader(device, "sky", format!("{common}\n{}", include_str!("shaders/sky.wgsl")));
        let sky = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky"),
            layout: Some(&sky_layout),
            vertex: wgpu::VertexState {
                module: &sky_module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: fullscreen_triangle(),
            // Drawn after the land, and only where none of it was (the depth still as it
            // was cleared, which is where the sky's one triangle lies): looking for the
            // clouds along a view ray is too much work to do behind a hillside.
            depth_stencil: depth_state(false, wgpu::CompareFunction::Equal),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &sky_module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &hdr_targets(None),
            }),
            multiview_mask: None,
            cache: None,
        });

        let plant_reach = format!("const PLANT_REACH: f32 = {PLANT_REACH:?};");
        let terrain_module = shader(device, "terrain", format!("{common}\n{plant_reach}\n{}\n{}\n{}", include_str!("shaders/season.wgsl"), include_str!("shaders/ground.wgsl"), include_str!("shaders/terrain.wgsl")));
        let vertex_attrs = wgpu::vertex_attr_array![0 => Uint32, 1 => Uint32];
        let instance_attrs = wgpu::vertex_attr_array![2 => Float32x3, 3 => Sint32x3, 4 => Float32, 5 => Float32x2, 6 => Uint32x4, 7 => Uint32x4, 8 => Uint32x2];
        let terrain_buffers = [
            Some(wgpu::VertexBufferLayout {
                array_stride: size_of::<Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &vertex_attrs,
            }),
            Some(wgpu::VertexBufferLayout {
                array_stride: size_of::<Instance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &instance_attrs,
            }),
        ];
        // The opaque terrain reads what is the same all over a chunk from its instance,
        // rather than having it handed down from every vertex (VOut in terrain.wgsl).
        let chunks_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("chunks"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let terrain_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("terrain"),
            bind_group_layouts: &[Some(&scene_layout_bg), Some(&chunks_layout)],
            immediate_size: 0,
        });
        let opaque = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("opaque"),
            layout: Some(&terrain_layout),
            vertex: wgpu::VertexState {
                module: &terrain_module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &terrain_buffers,
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: depth_state(true, wgpu::CompareFunction::Greater),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &terrain_module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &hdr_targets(None),
            }),
            multiview_mask: None,
            cache: None,
        });
        let opaque_inside = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("opaque inside"),
            layout: Some(&terrain_layout),
            vertex: wgpu::VertexState {
                module: &terrain_module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &terrain_buffers,
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: depth_state(true, wgpu::CompareFunction::Greater),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &terrain_module,
                entry_point: Some("fs_inside"),
                compilation_options: Default::default(),
                targets: &hdr_targets(None),
            }),
            multiview_mask: None,
            cache: None,
        });
        let water = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("water"),
            layout: Some(&water_layout),
            vertex: wgpu::VertexState {
                module: &terrain_module,
                entry_point: Some("vs_water"),
                compilation_options: Default::default(),
                buffers: &terrain_buffers,
            },
            primitive: Default::default(),
            depth_stencil: depth_state(false, wgpu::CompareFunction::Greater),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &terrain_module,
                entry_point: Some("fs_water"),
                compilation_options: Default::default(),
                targets: &hdr_targets(Some(wgpu::BlendState::ALPHA_BLENDING)),
            }),
            multiview_mask: None,
            cache: None,
        });

        let shadow_module = shader(device, "shadow", format!("{plant_reach}\n{}\n{}", include_str!("shaders/season.wgsl"), include_str!("shaders/shadow.wgsl")));
        let shadow = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow"),
            layout: Some(&shadow_layout),
            vertex: wgpu::VertexState {
                module: &shadow_module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &terrain_buffers,
            },
            // Both faces: thin leaf shells must cast from either side.
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
            }),
            multisample: Default::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });

        let far_module = shader(device, "far", format!("{common}\n{}\n{}\n{}", include_str!("shaders/season.wgsl"), include_str!("shaders/ground.wgsl"), include_str!("shaders/far.wgsl")));
        let far_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Unorm8x4, 2 => Snorm16x2, 4 => Unorm8x4, 5 => Uint8x4, 6 => Uint8x4];
        let far_instance_attrs = wgpu::vertex_attr_array![3 => Float32x3];
        let far = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("far"),
            layout: Some(&scene_layout),
            vertex: wgpu::VertexState {
                module: &far_module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[
                    Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<FarVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &far_attrs,
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<Instance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &far_instance_attrs,
                    }),
                ],
            },
            primitive: Default::default(),
            depth_stencil: depth_state(true, wgpu::CompareFunction::Greater),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &far_module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &hdr_targets(None),
            }),
            multiview_mask: None,
            cache: None,
        });
        // Far trees: instanced boxes, the box itself as four corners per side.
        let impostor_camera = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("impostor camera"),
            size: size_of::<ImpostorCamera>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let impostor_layout_bg = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("impostor camera"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX)],
        });
        let impostor_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("impostor camera"),
            layout: &impostor_layout_bg,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: impostor_camera.as_entire_binding(),
            }],
        });
        let impostor_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("impostor"),
            bind_group_layouts: &[Some(&scene_layout_bg), Some(&impostor_layout_bg)],
            immediate_size: 0,
        });
        let impostor_module = shader(device, "impostor", format!("{common}\n{}\n{}\n{}", include_str!("shaders/season.wgsl"), include_str!("shaders/ground.wgsl"), include_str!("shaders/impostor.wgsl")));
        let impostor_box_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 6 => Float32x3, 7 => Float32x3, 8 => Unorm8x4];
        let impostor_instance_attrs = wgpu::vertex_attr_array![2 => Sint32x3, 3 => Uint8x4, 4 => Unorm8x4, 5 => Uint8x4];
        let impostor = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("impostor"),
            layout: Some(&impostor_layout),
            vertex: wgpu::VertexState {
                module: &impostor_module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[
                    Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<BoxCorner>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &impostor_box_attrs,
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<BoxInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &impostor_instance_attrs,
                    }),
                ],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: depth_state(true, wgpu::CompareFunction::Greater),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &impostor_module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &hdr_targets(None),
            }),
            multiview_mask: None,
            cache: None,
        });
        // What people have built: the same boxes, cut off pixel by pixel where the voxel
        // rings take over, which the trees' pipeline need not pay for.
        let impostor_built = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("impostor built"),
            layout: Some(&impostor_layout),
            vertex: wgpu::VertexState {
                module: &impostor_module,
                entry_point: Some("vs_built"),
                compilation_options: Default::default(),
                buffers: &[
                    Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<BoxCorner>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &impostor_box_attrs,
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<BoxInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &impostor_instance_attrs,
                    }),
                ],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: depth_state(true, wgpu::CompareFunction::Greater),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &impostor_module,
                entry_point: Some("fs_built"),
                compilation_options: Default::default(),
                targets: &hdr_targets(None),
            }),
            multiview_mask: None,
            cache: None,
        });
        let sides: [(Vec3, Vec3, Vec3); 6] = [
            (Vec3::X, Vec3::Y, Vec3::Z),
            (Vec3::NEG_X, Vec3::Z, Vec3::Y),
            (Vec3::Y, Vec3::Z, Vec3::X),
            (Vec3::NEG_Y, Vec3::X, Vec3::Z),
            (Vec3::Z, Vec3::X, Vec3::Y),
            (Vec3::NEG_Z, Vec3::Y, Vec3::X),
        ];
        // Of each pair of opposite sides only the one facing the camera can be seen, so a
        // box is drawn as three sides, each vertex carrying its corner of both sides of the
        // pair and the shader taking one (impostor.wgsl). The corners are those of the six
        // sides as they were, in the same order, so each side's two triangles are too.
        // The bare crown's colour rides along as bytes, read as the box colours are.
        let [r, g, b] = impostors::BARE_CROWN;
        let mut box_corners: Vec<BoxCorner> = Vec::with_capacity(12);
        for pair in sides.chunks(2) {
            let corner = |(n, u, v): (Vec3, Vec3, Vec3), su: f32, sv: f32| (Vec3::splat(0.5) + (n + u * su + v * sv) * 0.5, n);
            for (su, sv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let (p, n) = corner(pair[0], su, sv);
                let (q, m) = corner(pair[1], su, sv);
                box_corners.push(BoxCorner {
                    corner: p.to_array(),
                    normal: n.to_array(),
                    opposite: q.to_array(),
                    opposite_normal: m.to_array(),
                    bare: [r, g, b, 255],
                });
            }
        }
        let impostor_cube = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("impostor box"),
            contents: bytemuck::cast_slice(&box_corners),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let sea_color = worldgen::materials::build().get(worldgen::materials::WATER).color;
        let sea_vertex = |x: f32, z: f32| FarVertex::water([x, crate::farview::SEA_LEVEL, z], sea_color, crate::farview::FLAG_WATER);
        const SEA_EXTENT: f32 = m(320_000.0);
        let sea = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("sea"),
            contents: bytemuck::cast_slice(&[
                sea_vertex(-SEA_EXTENT, -SEA_EXTENT),
                sea_vertex(-SEA_EXTENT, SEA_EXTENT),
                sea_vertex(SEA_EXTENT, SEA_EXTENT),
                sea_vertex(SEA_EXTENT, -SEA_EXTENT),
            ]),
            usage: wgpu::BufferUsages::VERTEX,
        });

        // Particles: a unit cube with normals, instanced.
        let particle_module = shader(device, "particles", format!("{common}\n{}", include_str!("shaders/particles.wgsl")));
        let mut cube_verts: Vec<[f32; 6]> = Vec::new();
        for (n, u, v) in [
            ([1.0f32, 0.0, 0.0], [0.0f32, 1.0, 0.0], [0.0f32, 0.0, 1.0]),
            ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
            ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
            ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
            ([0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]),
        ] {
            let (n, u, v) = (Vec3::from(n), Vec3::from(u), Vec3::from(v));
            let corner = |su: f32, sv: f32| {
                let p = n * 0.5 + u * su * 0.5 + v * sv * 0.5;
                [p.x, p.y, p.z, n.x, n.y, n.z]
            };
            let (a, b, c, d) = (corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0));
            cube_verts.extend_from_slice(&[a, b, c, a, c, d]);
        }
        let cube = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("cube"),
            contents: bytemuck::cast_slice(&cube_verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let cube_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
        let particle_attrs = wgpu::vertex_attr_array![2 => Float32x3, 3 => Float32, 4 => Float32x3, 5 => Float32, 6 => Float32x4, 7 => Float32x2, 8 => Float32];
        let particles = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("particles"),
            layout: Some(&scene_layout),
            vertex: wgpu::VertexState {
                module: &particle_module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[
                    Some(wgpu::VertexBufferLayout {
                        array_stride: 24,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &cube_attrs,
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<ParticleInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &particle_attrs,
                    }),
                ],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: depth_state(false, wgpu::CompareFunction::Greater),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &particle_module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &hdr_targets(Some(wgpu::BlendState::ALPHA_BLENDING)),
            }),
            multiview_mask: None,
            cache: None,
        });
        let props = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("props"),
            layout: Some(&scene_layout),
            vertex: wgpu::VertexState {
                module: &particle_module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[
                    Some(wgpu::VertexBufferLayout {
                        array_stride: 24,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &cube_attrs,
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<ParticleInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &particle_attrs,
                    }),
                ],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: depth_state(true, wgpu::CompareFunction::Greater),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &particle_module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &hdr_targets(None),
            }),
            multiview_mask: None,
            cache: None,
        });
        let prop_shadow_module = shader(device, "prop shadow", include_str!("shaders/prop_shadow.wgsl").to_string());
        let prop_shadow = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("prop shadow"),
            layout: Some(&shadow_layout),
            vertex: wgpu::VertexState {
                module: &prop_shadow_module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[
                    Some(wgpu::VertexBufferLayout {
                        array_stride: 24,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x3],
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<ParticleInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &wgpu::vertex_attr_array![2 => Float32x3, 3 => Float32, 4 => Float32x3, 7 => Float32x2],
                    }),
                ],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 },
            }),
            multisample: Default::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });
        let prop_instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("prop instances"),
            size: (PROP_CAPACITY * size_of::<ParticleInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // Room for every particle there can be, and a murmuration's birds besides.
        let particle_capacity = 10240;
        let particle_instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particle instances"),
            size: (particle_capacity * size_of::<ParticleInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Light shafts: marched at a quarter of the size, then added to the scene.
        let shafts_module = shader(device, "shafts", format!("{common}\n{}", include_str!("shaders/shafts.wgsl")));
        let shafts_pass = |label: &str, entry: &str, layout: &wgpu::PipelineLayout, format, blend| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &shafts_module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: fullscreen_triangle(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shafts_module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let march = shafts_pass("shafts march", "fs_march", &water_layout, SHAFTS_FORMAT, None);
        let adding = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let lay = shafts_pass(
            "shafts",
            "fs_lay",
            &lay_layout,
            HDR_FORMAT,
            Some(wgpu::BlendState {
                color: adding,
                alpha: adding,
            }),
        );

        // Post-processing: one layout, several entry points.
        let post_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("post"),
            bind_group_layouts: &[Some(&image_layout)],
            immediate_size: 0,
        });
        let post = |label: &str, entry: &str, defines: &str, format: wgpu::TextureFormat| {
            let module = shader(
                device,
                label,
                format!("{defines}\n{}", include_str!("shaders/post.wgsl")),
            );
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&post_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: fullscreen_triangle(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let no_dir = "const DIR: vec2<f32> = vec2<f32>(0.0, 0.0);\nconst ENCODE_SRGB: bool = false;";
        let bright = post("bright", "fs_bright", no_dir, HDR_FORMAT);
        let blur_h = post(
            "blur h",
            "fs_blur",
            "const DIR: vec2<f32> = vec2<f32>(1.0, 0.0);\nconst ENCODE_SRGB: bool = false;",
            HDR_FORMAT,
        );
        let blur_v = post(
            "blur v",
            "fs_blur",
            "const DIR: vec2<f32> = vec2<f32>(0.0, 1.0);\nconst ENCODE_SRGB: bool = false;",
            HDR_FORMAT,
        );
        let tonemap = post(
            "tonemap",
            "fs_tonemap",
            &format!(
                "const DIR: vec2<f32> = vec2<f32>(0.0, 0.0);\nconst ENCODE_SRGB: bool = {};",
                !output_format.is_srgb()
            ),
            output_format,
        );

        let instance_capacity = 4096;
        let instances = instance_buffer(device, instance_capacity);
        let chunks_bind = chunks_bind_group(device, &chunks_layout, &instances);
        let quad_capacity = 16384;
        Self {
            width,
            height,
            cloud_edges,
            wash: wash_tables(),
            flowering: worldgen::features::plants::seasons(),
            globals,
            shadows,
            scene_bind,
            cascade_binds,
            shadow_views,
            hdr,
            depth,
            bloom_a,
            bloom_b,
            shafts,
            shafts_layout,
            shafts_bind,
            march,
            lay,
            image_layout,
            depth_layout,
            depth_bind,
            linear_sampler,
            sky,
            aurora,
            aurora_bind,
            opaque,
            opaque_inside,
            water,
            far,
            impostor,
            impostor_built,
            impostor_cube,
            impostor_camera,
            impostor_bind,
            shadow,
            bright,
            blur_h,
            blur_v,
            tonemap,
            particles,
            props,
            prop_shadow,
            prop_instances,
            cube,
            particle_instances,
            particle_capacity,
            sea,
            instances,
            instance_capacity,
            chunks_layout,
            chunks_bind,
            quad_indices: quad_index_buffer(device, quad_capacity),
            quad_capacity,
            indirect: indirect_buffer(device, 4096),
            indirect_capacity: 4096,
            multi_draw: device.features().contains(wgpu::Features::INDIRECT_FIRST_INSTANCE),
            cascade_views: [None; CASCADES],
            shadow_sun: Vec3::ZERO,
            shadow_ground: None,
            band_scale: 1.0,
            lists: DrawLists::default(),
            frame: 0,
            timer: GpuTimer::new(gpu),
            stats: DrawStats::default(),
            start: std::time::Instant::now(),
            clock: None,
        }
    }

    fn depth_bind_group(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, view: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene depth"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            }],
        })
    }

    fn shafts_bind_group(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, depth: &wgpu::TextureView, marched: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shafts"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(marched),
                },
            ],
        })
    }

    fn image_bind_group(&self, device: &wgpu::Device, src: &wgpu::TextureView, extra: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("image"),
            layout: &self.image_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(src),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.linear_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(extra),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&self.depth.view),
                },
            ],
        })
    }

    pub fn resize(&mut self, gpu: &Gpu, width: u32, height: u32) {
        if width == 0 || height == 0 || (width, height) == (self.width, self.height) {
            return;
        }
        let device = &gpu.device;
        (self.width, self.height) = (width, height);
        self.hdr = make_target(device, "hdr", width, height, HDR_FORMAT);
        self.depth = Target {
            view: create_depth(device, width, height),
        };
        self.depth_bind = Self::depth_bind_group(device, &self.depth_layout, &self.depth.view);
        self.bloom_a = make_target(device, "bloom a", width / 4, height / 4, HDR_FORMAT);
        self.bloom_b = make_target(device, "bloom b", width / 4, height / 4, HDR_FORMAT);
        self.shafts = make_target(device, "shafts", width / 4, height / 4, SHAFTS_FORMAT);
        self.shafts_bind = Self::shafts_bind_group(device, &self.shafts_layout, &self.depth.view, &self.shafts.view);
    }

    /// Light view-projection for one cascade, in camera-relative space, the width of its
    /// texels as the sun sees them and the depth it spans, both in voxels. `ground` is
    /// the level to lay the map over, in voxels above sea level.
    fn cascade_matrix(camera: &Camera, aspect: f32, sun: Vec3, low_sun: f32, near: f32, far: f32, ground: f32) -> (Mat4, f32, f32) {
        // Bounding sphere of this slice of the view frustum.
        let fwd = camera.forward();
        let right = camera.right();
        let up = right.cross(fwd);
        let tan_y = (camera.fov_y * 0.5).tan();
        let tan_x = tan_y * aspect;
        let corner = |d: f32, sx: f32, sy: f32| fwd * d + right * (d * tan_x * sx) + up * (d * tan_y * sy);
        let corners = [
            corner(near, -1.0, -1.0),
            corner(near, 1.0, 1.0),
            corner(far, -1.0, -1.0),
            corner(far, 1.0, 1.0),
            corner(far, 1.0, -1.0),
            corner(far, -1.0, 1.0),
        ];
        let center = fwd * ((near + far) * 0.5);
        let radius = corners.iter().map(|c| (*c - center).length()).fold(0.0, f32::max) * 1.05;
        let texel = 2.0 * radius / SHADOW_SIZE as f32;
        // Casters well outside the slice (tall trees, hills toward the sun) still count.
        let reach = radius + m(320.0);
        // Depth 0..1 over [0, 2 * reach] along the sun's rays, toward the ground.
        let depth_range = reach * 2.0;

        if sun.y >= low_sun {
            // The map is laid over the ground, not square to the sun: a point is drawn
            // where the sun's ray through it meets a level plane, at that spot's own
            // world x and z. Texels are then squares of ground that never move, whatever
            // the sun does, so the sun can be followed every frame and a shadow glides
            // across them as it does across the ground. A map square to the sun turns
            // with it, and every turn slid all its texels to new places at once: every
            // shadow's edge twitched each time the maps caught up with the sun.
            let lean = Vec2::new(sun.x, sun.z) / sun.y;
            let level = (ground as f64 - camera.pos.y) as f32;
            // The slice's sphere, thrown onto the ground along the sun's rays, is longer
            // the lower the sun. The map grows to hold it in steps of a fifth, since each
            // change of size moves the texels.
            let grown = |lean: f32| 2f32.powf(((1.0 + lean * lean).sqrt().log2() * 4.0 - 0.001).ceil() / 4.0);
            let half = Vec2::new(grown(lean.x), grown(lean.y)) * radius;
            let cell = half * 2.0 / SHADOW_SIZE as f32;
            let snap = |at: f32, camera_at: f64, cell: f32| (((camera_at + at as f64) / cell as f64).floor() * cell as f64 - camera_at) as f32;
            let mid = Vec2::new(center.x, center.z) - lean * (center.y - level);
            let mid = Vec2::new(snap(mid.x, camera.pos.x, cell.x), snap(mid.y, camera.pos.z, cell.y));
            let matrix = Mat4::from_cols(
                Vec4::new(1.0 / half.x, 0.0, -sun.x / depth_range, 0.0),
                Vec4::new(-lean.x / half.x, -lean.y / half.y, -sun.y / depth_range, 0.0),
                Vec4::new(0.0, 1.0 / half.y, -sun.z / depth_range, 0.0),
                Vec4::new(
                    (lean.x * level - mid.x) / half.x,
                    (lean.y * level - mid.y) / half.y,
                    (reach + center.dot(sun)) / depth_range,
                    1.0,
                ),
            );
            // A texel of ground is narrower seen from the sun, the more along its rays it lies.
            let seen = (cell.x * (1.0 - sun.x * sun.x).sqrt()).max(cell.y * (1.0 - sun.z * sun.z).sqrt());
            return (matrix, seen, depth_range);
        }

        // Light view looking along the sun direction; snap the centre to texels so
        // the map does not shimmer as the camera moves. The texels are fixed in the
        // world, not to the camera: everything here is relative to the camera, so the
        // snapping has to allow for where the camera is.
        let light_up = if sun.y.abs() > 0.95 { Vec3::Z } else { Vec3::Y };
        let s = sun.cross(light_up).normalize();
        let u = s.cross(sun);
        let snap = |axis: Vec3| {
            let camera_at = camera.pos.dot(axis.as_dvec3());
            let at = camera_at + center.dot(axis) as f64;
            ((at / texel as f64).floor() * texel as f64 - camera_at) as f32
        };
        let snapped = s * snap(s) + u * snap(u) + sun * center.dot(sun);
        let eye = snapped + sun * reach;
        let view = Mat4::from_cols(
            Vec4::new(s.x, u.x, sun.x, 0.0),
            Vec4::new(s.y, u.y, sun.y, 0.0),
            Vec4::new(s.z, u.z, sun.z, 0.0),
            Vec4::new(-eye.dot(s), -eye.dot(u), -eye.dot(sun), 1.0),
        );
        let proj = Mat4::from_cols(
            Vec4::new(1.0 / radius, 0.0, 0.0, 0.0),
            Vec4::new(0.0, 1.0 / radius, 0.0, 0.0),
            Vec4::new(0.0, 0.0, -1.0 / depth_range, 0.0),
            Vec4::new(0.0, 0.0, 0.0, 1.0),
        );
        (proj * view, texel, depth_range)
    }

    /// Renders the scene and writes the tonemapped image to `target`.
    pub fn render(&mut self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, scene: &Scene) {
        self.draw(gpu, encoder, scene);
        self.finish(gpu, encoder, target);
    }

    /// Renders the scene into the renderer's own textures: all of a frame's work but
    /// the last pass, and none of it needing the window's next image. A game submits
    /// this before it waits for that image, so that the GPU is drawing while it waits.
    pub fn draw(&mut self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, scene: &Scene) {
        let Scene { camera, env, far, .. } = *scene;
        let device = &gpu.device;
        // Horizontal distances out to which each ring of voxel terrain is drawn: as far as
        // it is fully loaded, or less from high over the land.
        let target = band_scale(scene.terrain, camera.pos);
        // A screenshot is one frame, so it takes the reach at once.
        self.band_scale = if self.clock.is_some() { target } else { self.band_scale + (target - self.band_scale) * 0.05 };
        let bands = scene.terrain.bands().map(|b| b * self.band_scale);
        let voxel_radius = bands[LEVELS - 1];
        let aspect = self.width as f32 / self.height as f32;
        let view_proj = camera.view_proj(aspect);
        let tan = (camera.fov_y * 0.5).tan();
        let fwd = camera.forward();
        let right = camera.right();
        let up = right.cross(fwd);
        let time = self.clock.unwrap_or_else(|| self.start.elapsed().as_secs_f32());
        let v4 = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
        // Light shafts want the sun up and something in the air to catch it: they fade
        // out, and their passes are left out, as the sun sets and in thin dry air.
        let shafts = if scene.shafts && env.daylight {
            let air = ((env.haziness() - 0.1) / 0.2).max(env.mist * VOXELS_PER_METRE / 0.0004).clamp(0.0, 1.0);
            air * (env.sun_elevation / 0.06).clamp(0.0, 1.0)
        } else {
            0.0
        };

        // Shadow cascades.
        let mut light_vp = [[[0.0; 4]; 4]; CASCADES];
        let mut light = Vec::with_capacity(CASCADES);
        let mut texels = [0.0; 4];
        let mut spans = [0.0; 4];
        let mut fresh = [false; CASCADES];
        let mut near = 0.1;
        let frame = self.frame;
        self.frame += 1;
        // The nearest map, once it is square to a low sun, follows it in steps: its
        // texels are fixed in the world only while the sun holds still, and close by a
        // step is too small to see. The farther maps follow the sun every frame, or the
        // long shadows cast from far off would jump a metre at each step.
        if env.sun_dir.y >= LOW_SUN[0] || self.shadow_sun.distance(env.sun_dir) > SHADOW_SUN_STEP {
            self.shadow_sun = env.sun_dir;
        }
        let ground = match self.shadow_ground {
            Some(g) if (g - env.mist_floor).abs() <= GROUND_SLACK => g,
            _ => (env.mist_floor / GROUND_STEP).round() * GROUND_STEP,
        };
        self.shadow_ground = Some(ground);
        for (i, &far_end) in CASCADE_ENDS.iter().enumerate() {
            let sun = if i == 0 { self.shadow_sun } else { env.sun_dir };
            let (m, texel, span) = Self::cascade_matrix(camera, aspect, sun, LOW_SUN[i], near, far_end, ground);
            light.push(Frustum::new(m));
            near = far_end;
            fresh[i] = self.cascade_views[i].is_none_or(|v| {
                cascade_due(i, frame) || v.fwd.dot(fwd) < CASCADE_SWING || v.camera.distance(camera.pos) > far_end as f64 * CASCADE_DRIFT
            });
            if fresh[i] {
                self.cascade_views[i] = Some(CascadeView {
                    light_vp: m,
                    texel,
                    span,
                    camera: camera.pos,
                    fwd,
                });
                gpu.queue.write_buffer(
                    &self.cascade_binds[i].0,
                    0,
                    bytemuck::bytes_of(&CascadeUniform { light_vp: m.to_cols_array_2d(), season: [env.date, 0.0, 0.0, 0.0], flowering: self.flowering }),
                );
            }
            // A map drawn earlier is still right: it is fixed in the world, so only the
            // camera's movement since then needs allowing for.
            let view = self.cascade_views[i].expect("just set");
            let moved = (camera.pos - view.camera).as_vec3();
            light_vp[i] = (view.light_vp * Mat4::from_translation(moved)).to_cols_array_2d();
            texels[i] = view.texel;
            spans[i] = view.span;
        }
        gpu.queue
            .write_buffer(&self.shadows, 0, bytemuck::bytes_of(&Shadows { light_vp, texel: texels, span: spans }));

        let cam_wrapped = camera.pos.rem_euclid(glam::DVec3::splat(4096.0)).as_vec3();
        // Where the camera stands on the cloud grid as it drifts, wrapped with the grid's
        // pattern so the clouds never jump.
        let drift = time as f64 * m(4.8) as f64;
        let cloud_at = glam::DVec2::new(camera.pos.x + drift, camera.pos.z + drift * 0.35)
            .rem_euclid(glam::DVec2::splat(CLOUD_CELL as f64 * CLOUD_PERIOD as f64))
            .as_vec2();
        let mut globals = Globals {
            view_proj: view_proj.to_cols_array_2d(),
            cam_right: v4(right * tan * aspect, 0.0),
            // w: how tall a pixel is per voxel of distance, taking the screen as 1080 lines
            // (see distant in terrain.wgsl), the same for every pixel of the frame.
            cam_up: v4(up * tan, (up * tan).length() / fwd.length().max(1e-6) * 2.0 / 1080.0),
            cam_fwd: v4(fwd, 0.0),
            sun_dir: v4(env.sun_dir, time),
            sun_color: v4(env.sun_color, env.daylight as u8 as f32),
            sky_zenith: v4(env.sky_zenith, env.cloud_churn.rem_euclid(std::f32::consts::TAU)),
            sky_horizon: v4(env.sky_horizon, env.cloud_swell),
            fog: [env.fog_density, voxel_radius, env.clouds, time * m(4.8)],
            // w: how far the tree models reach, for the far view's forest height.
            cam_wrapped: [cam_wrapped.x, camera.pos.y as f32, cam_wrapped.z, impostors::REACH],
            // The mist is thickest up to a little over the low ground, and thins above.
            mist: [env.mist, m(22.0), env.mist_floor + m(5.0) - camera.pos.y as f32, cloud_edge(&self.cloud_edges, env.clouds)],
            cascades: [
                CASCADE_ENDS[0],
                CASCADE_ENDS[1],
                CASCADE_ENDS[2],
                env.cloud_base - camera.pos.y as f32,
            ],
            wind: v4(scene.wind, 0.0),
            lights: nearest_glows(scene.particles),
            lamps: nearest_lamps(scene.lamps, camera.pos),
            season: [env.date, scene.ground.here.snow, scene.ground.here.frozen, scene.ground.here.snowline],
            weather: [scene.ground.here.wet, scene.ground.here.highland, cloud_at.x, cloud_at.y],
            sky_glow: v4(env.sky_glow, env.glow),
            // w: how far the stars have wheeled round, radians: once a day, and once
            // more over the year, so each season has its own evening sky.
            sun_pos: v4(env.sun_pos, (env.time / 24.0 + env.date) * std::f32::consts::TAU),
            air: [env.haze, env.haze_height, shafts, env.haziness()],
            snow: std::array::from_fn(|k| {
                let g = scene.ground.biomes.get(k).unwrap_or(&scene.ground.here);
                [g.snow, g.frozen, g.highland, g.snowline]
            }),
            wash_body: self.wash.0,
            wash_bloom: self.wash.1,
            flowering: self.flowering,
        };
        globals.cam_fwd[3] = snow_floor(&globals.snow);
        gpu.queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));
        // The lights: drawn by the sky alone (see aurora.rs). Their curtains change
        // shape with the time, so they need no other number.
        gpu.queue.write_buffer(&self.aurora, 0, bytemuck::bytes_of(&[env.aurora, 0.0f32, 0.0, 0.0]));

        // Visible chunks, with their camera-relative offsets computed in f64.
        let frustum = Frustum::new(view_proj);
        // Only faces turned toward the sun can be the nearest thing it sees.
        let toward_sun = [
            env.sun_dir.x > -0.001,
            env.sun_dir.x < 0.001,
            env.sun_dir.y > -0.001,
            env.sun_dir.y < 0.001,
            env.sun_dir.z > -0.001,
            env.sun_dir.z < 0.001,
        ];
        // The far cascade is coarser than a voxel, so the tops of things are enough for
        // it until the sun gets low and the sides start to catch the light.
        let far_casters = if env.sun_dir.y > HIGH_SUN { [false, false, true, false, false, false] } else { toward_sun };
        // How far a shadow can reach across the ground, for finding what can cast into view.
        let sweep = -env.sun_dir * (m(120.0) / env.sun_dir.y.clamp(0.15, 1.0));
        let chunk_size = CHUNK as f32;
        // Last frame's lists, emptied, so that they keep the room they grew to.
        let mut lists = std::mem::take(&mut self.lists);
        lists.clear();
        let DrawLists { instances, opaque, opaque_edge, casters, water, impostors: impostor_draws, built: built_draws, draws } = &mut lists;
        // Which of a chunk's skylines the sun (or moon) is on the side of: toward
        // sunrise, or toward sunset (as skyline_light in common.wgsl chooses).
        let sun_side = if scene.env.sun_dir.x > 0.0 { 0 } else { 1 };
        let mut stats = DrawStats::default();
        let mut max_quads = 0;
        let lit = env.sun_color.max_element() > MIN_SHADOW_LIGHT;
        // A column is first tested whole, as one box from its lowest chunk to its highest:
        // if that is outside its ring, or neither seen nor casting a shadow into view,
        // then so is every chunk in it, each test below failing for the whole column's box
        // as it would for a box inside it.
        let mut columns_culled = 0;
        let columns = scene.terrain.columns().filter(|column| {
            let Some(first) = column.first() else { return false };
            let (low, high) = column.iter().fold((i32::MAX, i32::MIN), |(l, h), c| (l.min(c.pos.y), h.max(c.pos.y)));
            let scale = 1 << first.level;
            let band = [if first.level == 0 { 0.0 } else { bands[first.level as usize - 1] }, bands[first.level as usize]];
            let size = chunk_size * scale as f32;
            // Worked out as for the chunks themselves, so the box holds each of theirs.
            let offset_of = |y: i32| ((IVec3::new(first.pos.x, y, first.pos.z) * CHUNK * scale).as_dvec3() - camera.pos).as_vec3();
            let offset = offset_of(low);
            let far_corner = offset_of(high) + Vec3::splat(size);
            let nearest = Vec2::new(offset.x.max(-far_corner.x).max(0.0), offset.z.max(-far_corner.z).max(0.0)).length();
            let reach = Vec2::new(offset.x.abs().max(far_corner.x.abs()), offset.z.abs().max(far_corner.z.abs())).length();
            let shown = !(band[0] >= band[1] || reach < band[0] || nearest >= band[1])
                && (frustum.intersects_box(offset, far_corner)
                    || (lit
                        && frustum.intersects_swept_box(offset, far_corner, sweep)
                        && (0..CASCADES).any(|i| fresh[i] && light[i].intersects_box(offset, far_corner))));
            if !shown {
                columns_culled += column.len() as u32;
            }
            shown
        });
        for chunk in columns.flatten() {
            let scale = 1 << chunk.level;
            let band = [if chunk.level == 0 { 0.0 } else { bands[chunk.level as usize - 1] }, bands[chunk.level as usize]];
            let size = chunk_size * scale as f32;
            let origin = chunk.pos * CHUNK * scale;
            let offset = (origin.as_dvec3() - camera.pos).as_vec3();
            let far_corner = offset + Vec3::splat(size);
            // How far across the ground the chunk spans from the camera.
            let nearest = Vec2::new(offset.x.max(-far_corner.x).max(0.0), offset.z.max(-far_corner.z).max(0.0)).length();
            let reach = Vec2::new(offset.x.abs().max(far_corner.x.abs()), offset.z.abs().max(far_corner.z.abs())).length();
            // A ring draws only between its two circles.
            if band[0] >= band[1] || reach < band[0] || nearest >= band[1] {
                stats.chunks_culled += 1;
                continue;
            }
            let visible = frustum.intersects_box(offset, far_corner);
            let center = offset + Vec3::splat(size * 0.5);
            // Each cascade only draws what falls inside its own box of light, and only
            // if its shadow could land somewhere the camera can see.
            // The moon casts shadows as the sun does: it is the only light on the land at
            // night, and without them a wood was lit right through, trunk, crown and the
            // ground beneath, as flat and even as open ground, while the plants close by
            // (drawn into the maps as props) still cast theirs. Only in the moment either
            // side of the change from sun to moon, when neither gives any light, is there
            // nothing for a shadow to take away.
            let reaches_view = lit && frustum.intersects_swept_box(offset, far_corner, sweep);
            let casts: [bool; CASCADES] = std::array::from_fn(|i| fresh[i] && reaches_view && light[i].intersects_box(offset, far_corner));
            // Directions whose faces could be seen from the camera. Foliage sways, so
            // leave a little slack.
            let slack = m(1.0);
            let seen = [
                offset.x < slack,
                far_corner.x > -slack,
                offset.y < slack,
                far_corner.y > -slack,
                offset.z < slack,
                far_corner.z > -slack,
            ];
            if !visible && !casts.contains(&true) {
                stats.chunks_culled += 1;
                continue;
            }
            let index = instances.len() as u32;
            let mut biomes = chunk.biomes;
            biomes[7] = u32::from_le_bytes(chunk.skylines.map(|s| s[sun_side]));
            instances.push(Instance {
                offset: offset.to_array(),
                origin: origin.to_array(),
                scale: scale as f32,
                band,
                biomes,
                slopes: [0, 1].map(|k| u32::from_le_bytes(chunk.slopes.map(|s| s[k] as u8))),
                hollows: u32::from_le_bytes(chunk.hollows),
            });
            if let Some(mesh) = &chunk.opaque {
                max_quads = max_quads.max(mesh.quads);
                let (terrain, plants) = (&chunk.faces[..=6], &chunk.faces[6..]);
                // Plants are too small to matter from further away. They give out one by
                // one over the last of their reach (plant_reach in season.wgsl), which the
                // shaders know them for by an instance of their own with its scale negative.
                let near = chunk.level == 0 && nearest < PLANT_REACH * 0.98 && plants[6] > plants[0];
                let plant_index = instances.len() as u32;
                if near {
                    let mut own = instances[index as usize];
                    own.scale = -own.scale;
                    instances.push(own);
                }
                for (i, (list, _)) in casters.iter_mut().zip(casts).enumerate().filter(|(_, (_, casts))| *casts) {
                    push_faces(list, mesh, terrain, if i == CASCADES - 1 { far_casters } else { toward_sun }, index);
                    if near && i < PLANT_SHADOW_CASCADES {
                        push_faces(list, mesh, plants, toward_sun, plant_index);
                    }
                }
                if visible {
                    // Chunks entirely inside their ring never need trimming to it.
                    let inside = reach + m(1.0) < band[1] && nearest >= band[0];
                    let list: &mut Vec<ChunkDraw> = if inside { opaque } else { opaque_edge };
                    push_faces(list, mesh, terrain, seen, index);
                    if near {
                        push_faces(list, mesh, plants, seen, plant_index);
                    }
                }
            }
            if visible {
                stats.chunks_drawn += 1;
                if let Some(mesh) = &chunk.water {
                    water.push((chunk_draw(mesh, index), center.length_squared()));
                    stats.quads_drawn += mesh.quads as u64;
                    max_quads = max_quads.max(mesh.quads);
                }
            }
        }

        stats.chunks_culled += columns_culled;

        // Far view: one instance per continent (plus one for the sea), tiles culled
        // against the frustum.
        let mut far_tiles: Vec<(&wgpu::Buffer, usize, u32)> = Vec::new();
        let mut far_rivers: Vec<(&wgpu::Buffer, u32, u32)> = Vec::new();
        for cell in far.cells() {
            let offset = cell.offset(camera.pos);
            let index = instances.len() as u32;
            instances.push(Instance {
                offset: offset.to_array(),
                origin: [0; 3],
                scale: 1.0,
                band: [0.0; 2],
                biomes: [0; 8],
                slopes: [0; 2],
                hollows: 0x8080_8080,
            });
            for tile in &cell.tiles {
                if frustum.intersects_box(tile.lo + offset, tile.hi + offset) {
                    far_tiles.push((&tile.buffer, cell.stride, index));
                }
            }
            if let Some((buffer, quads)) = &cell.rivers {
                far_rivers.push((buffer, *quads, index));
                max_quads = max_quads.max(*quads);
            }
        }
        let sea_instance = instances.len() as u32;
        instances.push(Instance {
            offset: [0.0, -camera.pos.y as f32, 0.0],
            origin: [0; 3],
            scale: 1.0,
            band: [0.0; 2],
            biomes: [0; 8],
                slopes: [0; 2],
            hollows: 0x8080_8080,
        });
        stats.far_tiles = far_tiles.len() as u32;

        // Far trees, a square of them at a time.
        for tile in scene.impostors.tiles() {
            let lo = (tile.lo.as_dvec3() - camera.pos).as_vec3();
            let hi = (tile.hi.as_dvec3() - camera.pos).as_vec3();
            let reach = Vec2::new(lo.x.abs().max(hi.x.abs()), lo.z.abs().max(hi.z.abs())).length();
            // Squares wholly inside the voxel terrain are left to it.
            if reach + m(40.0) >= voxel_radius && frustum.intersects_box(lo, hi) {
                impostor_draws.push([BOX_SIDES * 6, tile.span.quads, 0, 0, tile.span.first()]);
                stats.impostor_boxes += tile.span.quads;
            }
        }

        // What people have built, a square at a time: those reaching beyond the voxel
        // rings and coming within sight. Each continent's are a buffer of their own.
        let mut built_spans: Vec<(&wgpu::Buffer, Range<usize>)> = Vec::new();
        for shown in scene.impostors.built.continents() {
            let start = built_draws.len();
            for square in &shown.squares {
                let lo = (square.lo.as_dvec3() - camera.pos).as_vec3();
                let hi = (square.hi.as_dvec3() - camera.pos).as_vec3();
                let nearest = Vec2::new(lo.x.max(-hi.x).max(0.0), lo.z.max(-hi.z).max(0.0)).length();
                let reach = Vec2::new(lo.x.abs().max(hi.x.abs()), lo.z.abs().max(hi.z.abs())).length();
                if reach >= voxel_radius && nearest < crate::farbuilt::SIGHT && frustum.intersects_box(lo, hi) {
                    built_draws.push([BOX_SIDES * 6, square.count, 0, 0, square.first]);
                    stats.built_boxes += square.count;
                }
            }
            if built_draws.len() > start {
                built_spans.push((&shown.buffer, start..built_draws.len()));
            }
        }

        // Translucent water blends correctly only when drawn far to near.
        water.sort_unstable_by(|a, b| b.1.total_cmp(&a.1));
        stats.shadow_draws = std::array::from_fn(|i| casters[i].len() as u32);
        stats.quads_drawn += opaque.iter().chain(opaque_edge.iter()).map(|d| d[0] as u64 / 6).sum::<u64>();
        // Nearest first, so that hidden surfaces are rejected instead of shaded.
        let distance_of = |d: &ChunkDraw| {
            let [x, y, z] = instances[d[4] as usize].offset;
            (Vec3::new(x, y, z) + Vec3::splat(chunk_size * 0.5)).length_squared()
        };
        opaque.sort_by_cached_key(|d| distance_of(d) as u32);
        self.stats = stats;

        // All of this frame's chunk draws, in one buffer the passes take slices of.
        draws.reserve(opaque.len() + opaque_edge.len() + water.len() + impostor_draws.len() + built_draws.len() + casters.iter().map(Vec::len).sum::<usize>());
        let mut append = |list: &mut dyn Iterator<Item = ChunkDraw>| {
            let start = draws.len();
            draws.extend(list);
            start..draws.len()
        };
        let shadow_lists: [Range<usize>; CASCADES] = std::array::from_fn(|i| append(&mut casters[i].iter().copied()));
        let opaque_list = append(&mut opaque.iter().copied());
        let edge_list = append(&mut opaque_edge.iter().copied());
        let water_list = append(&mut water.iter().map(|w| w.0));
        let impostor_list = append(&mut impostor_draws.iter().copied());
        let built_list = append(&mut built_draws.iter().copied());
        for (_, span) in &mut built_spans {
            *span = built_list.start + span.start..built_list.start + span.end;
        }
        if draws.len() > self.indirect_capacity {
            self.indirect_capacity = draws.len().next_power_of_two();
            self.indirect = indirect_buffer(device, self.indirect_capacity);
        }
        gpu.queue.write_buffer(&self.indirect, 0, bytemuck::cast_slice(&draws[..]));
        let indirect = self.multi_draw.then_some(&self.indirect);
        let chunk_vertices = scene.terrain.vertices();

        if instances.len() > self.instance_capacity {
            self.instance_capacity = instances.len().next_power_of_two();
            self.instances = instance_buffer(device, self.instance_capacity);
            self.chunks_bind = chunks_bind_group(device, &self.chunks_layout, &self.instances);
        }
        if max_quads > self.quad_capacity {
            self.quad_capacity = max_quads.next_power_of_two();
            self.quad_indices = quad_index_buffer(device, self.quad_capacity);
        }
        gpu.queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&instances[..]));
        let whole = camera.pos.floor();
        let (voxel, frac) = (whole.as_ivec3(), (camera.pos - whole).as_vec3());
        gpu.queue.write_buffer(
            &self.impostor_camera,
            0,
            bytemuck::bytes_of(&ImpostorCamera {
                voxel: [voxel.x, voxel.y, voxel.z, 0],
                frac: [frac.x, frac.y, frac.z, crate::farbuilt::SIGHT],
            }),
        );

        if let Some(t) = &self.timer {
            t.mark(encoder, 0);
        }

        // The moving models: those seen first, then those that only cast a shadow.
        let prop_count = scene.props.len().min(PROP_CAPACITY);
        let shade_count = scene.shades.len().min(PROP_CAPACITY - prop_count);
        if prop_count > 0 {
            gpu.queue.write_buffer(&self.prop_instances, 0, bytemuck::cast_slice(&scene.props[..prop_count]));
        }
        if shade_count > 0 {
            let at = (prop_count * size_of::<ParticleInstance>()) as u64;
            gpu.queue.write_buffer(&self.prop_instances, at, bytemuck::cast_slice(&scene.shades[..shade_count]));
        }

        // --- Shadow passes -------------------------------------------------------
        for (i, view) in self.shadow_views.iter().enumerate() {
            if !fresh[i] {
                if let Some(t) = &self.timer {
                    t.mark(encoder, i as u32 + 1);
                }
                continue;
            }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if !shadow_lists[i].is_empty() {
                pass.set_pipeline(&self.shadow);
                pass.set_bind_group(0, &self.cascade_binds[i].1, &[]);
                pass.set_index_buffer(self.quad_indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.set_vertex_buffer(0, chunk_vertices.slice(..));
                pass.set_vertex_buffer(1, self.instances.slice(..));
                draw_chunks(&mut pass, indirect, &draws, shadow_lists[i].clone());
            }
            // They are small and near: the far cascade would not show them.
            if i < PLANT_SHADOW_CASCADES && prop_count + shade_count > 0 {
                pass.set_pipeline(&self.prop_shadow);
                pass.set_bind_group(0, &self.cascade_binds[i].1, &[]);
                pass.set_vertex_buffer(0, self.cube.slice(..));
                pass.set_vertex_buffer(1, self.prop_instances.slice(..));
                pass.draw(0..36, 0..(prop_count + shade_count) as u32);
            }
            drop(pass);
            if let Some(t) = &self.timer {
                t.mark(encoder, i as u32 + 1);
            }
        }

        // --- Opaque scene --------------------------------------------------------
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.hdr.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.scene_bind, &[]);

            // Far view first: land tiles, then river ribbons and the sea over them.
            pass.set_vertex_buffer(1, self.instances.slice(..));
            pass.set_pipeline(&self.far);
            let mut bound_stride = 0;
            let mut grid_count = 0;
            for (buffer, stride, index) in &far_tiles {
                if *stride != bound_stride {
                    let (grid, count) = far.grid(*stride);
                    pass.set_index_buffer(grid.slice(..), wgpu::IndexFormat::Uint16);
                    (bound_stride, grid_count) = (*stride, count);
                }
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw_indexed(0..grid_count, 0, *index..*index + 1);
            }
            pass.set_index_buffer(self.quad_indices.slice(..), wgpu::IndexFormat::Uint32);
            for (buffer, quads, index) in &far_rivers {
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw_indexed(0..quads * 6, 0, *index..*index + 1);
            }
            pass.set_vertex_buffer(0, self.sea.slice(..));
            pass.draw_indexed(0..6, 0, sea_instance..sea_instance + 1);

            if let Some(t) = &self.timer {
                t.mark_in_pass(&mut pass, 4);
            }
            // Far trees stand on the far view, drawn with it.
            if !impostor_list.is_empty() {
                pass.set_pipeline(&self.impostor);
                pass.set_bind_group(1, &self.impostor_bind, &[]);
                pass.set_vertex_buffer(0, self.impostor_cube.slice(..));
                pass.set_vertex_buffer(1, scene.impostors.buffer().slice(..));
                draw_chunks(&mut pass, indirect, &draws, impostor_list.clone());
                pass.set_vertex_buffer(1, self.instances.slice(..));
            }
            if !built_spans.is_empty() {
                pass.set_pipeline(&self.impostor_built);
                pass.set_bind_group(1, &self.impostor_bind, &[]);
                pass.set_vertex_buffer(0, self.impostor_cube.slice(..));
                for (buffer, span) in &built_spans {
                    pass.set_vertex_buffer(1, buffer.slice(..));
                    draw_chunks(&mut pass, indirect, &draws, span.clone());
                }
                pass.set_vertex_buffer(1, self.instances.slice(..));
            }
            if let Some(t) = &self.timer {
                t.mark_in_pass(&mut pass, 5);
            }

            pass.set_pipeline(&self.opaque_inside);
            pass.set_bind_group(1, &self.chunks_bind, &[]);
            pass.set_vertex_buffer(0, chunk_vertices.slice(..));
            draw_chunks(&mut pass, indirect, &draws, opaque_list);
            pass.set_pipeline(&self.opaque);
            draw_chunks(&mut pass, indirect, &draws, edge_list);
            if let Some(t) = &self.timer {
                t.mark_in_pass(&mut pass, 6);
            }

            if prop_count > 0 {
                pass.set_pipeline(&self.props);
                pass.set_vertex_buffer(0, self.cube.slice(..));
                pass.set_vertex_buffer(1, self.prop_instances.slice(..));
                pass.draw(0..36, 0..prop_count as u32);
            }
            if let Some(t) = &self.timer {
                t.mark_in_pass(&mut pass, 7);
            }
            // The sky last, behind everything (see its pipeline).
            pass.set_pipeline(&self.sky);
            pass.set_bind_group(1, &self.aurora_bind, &[]);
            pass.draw(0..3, 0..1);
            if let Some(t) = &self.timer {
                t.mark_in_pass(&mut pass, 8);
            }
        }

        // --- Water, reading the depth of what lies beneath, then particles ---------
        let particle_count = scene.particles.len().min(self.particle_capacity);
        if particle_count > 0 {
            gpu.queue.write_buffer(&self.particle_instances, 0, bytemuck::cast_slice(&scene.particles[..particle_count]));
        }
        if !water_list.is_empty() || particle_count > 0 {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("water"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.hdr.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth.view,
                    // Read-only, so the same texture can be sampled in the shader.
                    depth_ops: None,
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.water);
            pass.set_bind_group(0, &self.scene_bind, &[]);
            pass.set_bind_group(1, &self.depth_bind, &[]);
            pass.set_index_buffer(self.quad_indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.set_vertex_buffer(0, chunk_vertices.slice(..));
            pass.set_vertex_buffer(1, self.instances.slice(..));
            draw_chunks(&mut pass, indirect, &draws, water_list.clone());
            if let Some(t) = &self.timer {
                t.mark_in_pass(&mut pass, 9);
            }
            if particle_count > 0 {
                pass.set_pipeline(&self.particles);
                pass.set_bind_group(0, &self.scene_bind, &[]);
                pass.set_vertex_buffer(0, self.cube.slice(..));
                pass.set_vertex_buffer(1, self.particle_instances.slice(..));
                pass.draw(0..36, 0..particle_count as u32);
            }
        }
        else if let Some(t) = &self.timer {
            t.mark(encoder, 9);
        }
        if let Some(t) = &self.timer {
            t.mark(encoder, 10);
        }

        // --- Light shafts ----------------------------------------------------------
        if shafts > 0.0 {
            let passes = [
                ("shafts march", &self.march, &self.depth_bind, &self.shafts.view, wgpu::LoadOp::Clear(wgpu::Color::BLACK)),
                ("shafts", &self.lay, &self.shafts_bind, &self.hdr.view, wgpu::LoadOp::Load),
            ];
            for (label, pipeline, bind, view, load) in passes {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some(label),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &self.scene_bind, &[]);
                pass.set_bind_group(1, bind, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        if let Some(t) = &self.timer {
            t.mark(encoder, 11);
        }

        // --- Bloom and tonemap ---------------------------------------------------
        // The third slot of each bind group must not be the pass's own target, so an
        // unused texture is bound there.
        let hdr_bind = self.image_bind_group(device, &self.hdr.view, &self.bloom_b.view);
        fullscreen(encoder, "bright", &self.bright, &hdr_bind, &self.bloom_a.view);
        let a_bind = self.image_bind_group(device, &self.bloom_a.view, &self.hdr.view);
        fullscreen(encoder, "blur h", &self.blur_h, &a_bind, &self.bloom_b.view);
        let b_bind = self.image_bind_group(device, &self.bloom_b.view, &self.hdr.view);
        fullscreen(encoder, "blur v", &self.blur_v, &b_bind, &self.bloom_a.view);
        if let Some(t) = &self.timer {
            t.mark(encoder, 12);
        }
        self.lists = lists;
    }

    /// Writes the tonemapped image of what [`Renderer::draw`] drew to `target`.
    pub fn finish(&mut self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        if let Some(t) = &self.timer {
            t.mark(encoder, 13);
        }
        let final_bind = self.image_bind_group(&gpu.device, &self.hdr.view, &self.bloom_a.view);
        fullscreen(encoder, "tonemap", &self.tonemap, &final_bind, target);
        if let Some(t) = &mut self.timer {
            t.mark(encoder, 14);
        }
    }
}

/// How much of their loaded reach to draw the rings of voxel terrain out to, with the
/// camera at `camera`: all of it on the ground, and from high over the land, where no
/// hill hides the land beyond, half (and the far view the rest). Up among the clouds
/// every chunk out to the edge was in sight, its voxels a pixel or two across, and the
/// land cost near twice what it did on the ground; from up there the far view's slopes
/// look the same. How high is over the tallest chunk with anything in it around the
/// camera's column, so a cliff top close by keeps the full reach.
fn band_scale(terrain: &Streamer, camera: DVec3) -> f32 {
    let chunks = columns_around(camera).filter_map(|(x, z)| terrain.column(0, x, z)).flatten();
    band_scale_over(chunks.filter(|c| c.level == 0 && c.opaque.is_some()).map(|c| c.pos), camera)
}

/// How far around the camera's column [`band_scale`] looks for the ground, in voxels.
const GROUND_AROUND: f64 = m(24.0) as f64;

/// The full-size columns, as `(x, z)` in columns, that [`band_scale_over`] can find the
/// ground in: a few, rather than every chunk loaded.
fn columns_around(camera: DVec3) -> impl Iterator<Item = (i32, i32)> {
    let size = CHUNK as f64;
    // A column counts if the camera is within GROUND_AROUND of it, so its low corner
    // lies from a column and that much below the camera to that much above it.
    let span = |c: f64| ((c - size - GROUND_AROUND) / size).floor() as i32..=((c + GROUND_AROUND) / size).floor() as i32;
    let xs = span(camera.x);
    span(camera.z).flat_map(move |z| xs.clone().map(move |x| (x, z)))
}

/// [`band_scale`] over the full-size chunks with anything in them at `chunks`, in chunks.
fn band_scale_over(chunks: impl Iterator<Item = IVec3>, camera: DVec3) -> f32 {
    let around = GROUND_AROUND;
    let mut ground = None::<f64>;
    for pos in chunks {
        let origin = (pos * CHUNK).as_dvec3();
        let size = CHUNK as f64;
        if origin.x - around <= camera.x && camera.x <= origin.x + size + around && origin.z - around <= camera.z && camera.z <= origin.z + size + around {
            ground = Some(ground.map_or(origin.y + size, |g: f64| g.max(origin.y + size)));
        }
    }
    // Not loaded yet under the camera: the reach as loaded.
    let Some(ground) = ground else { return 1.0 };
    let lift = (camera.y - ground) as f32;
    let t = ((lift - m(60.0)) / m(140.0)).clamp(0.0, 1.0);
    1.0 - 0.5 * t * t * (3.0 - 2.0 * t)
}

/// Draws one triangle over the whole of `target` with `pipeline`.
fn fullscreen(encoder: &mut wgpu::CommandEncoder, label: &str, pipeline: &wgpu::RenderPipeline, bind: &wgpu::BindGroup, target: &wgpu::TextureView) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bind, &[]);
    pass.draw(0..3, 0..1);
}

/// View frustum in camera-relative space. With an infinite reverse-Z projection there
/// is no far plane, so five planes.
struct Frustum {
    planes: [Vec4; 5],
}

impl Frustum {
    fn new(view_proj: Mat4) -> Self {
        let (r0, r1, r2, r3) = (view_proj.row(0), view_proj.row(1), view_proj.row(2), view_proj.row(3));
        Self {
            planes: [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r3 - r2],
        }
    }

    /// Whether a box could touch the frustum at any point as it slides along `sweep`.
    fn intersects_swept_box(&self, min: Vec3, max: Vec3, sweep: Vec3) -> bool {
        self.planes.iter().all(|p| {
            let n = p.truncate();
            let v = Vec3::select(n.cmpge(Vec3::ZERO), max, min);
            n.dot(v) + p.w + n.dot(sweep).max(0.0) >= 0.0
        })
    }

    fn intersects_box(&self, min: Vec3, max: Vec3) -> bool {
        self.planes.iter().all(|p| {
            // The box corner farthest along the plane normal.
            let v = Vec3::select(p.truncate().cmpge(Vec3::ZERO), max, min);
            p.truncate().dot(v) + p.w >= 0.0
        })
    }
}

/// Renders one frame headless and saves it as a PNG.
#[allow(clippy::too_many_arguments)]
pub fn screenshot(
    gpu: &Gpu,
    path: &std::path::Path,
    size: (u32, u32),
    camera: &Camera,
    env: &Environment,
    streamer: &mut Streamer,
    far: &mut FarView,
    impostors: &mut Impostors,
    particles: &[ParticleInstance],
    props: &[ParticleInstance],
    wind: Vec3,
    ground: crate::season::Places,
    lamps: &[(DVec3, f32)],
    // Drawn over each finished frame, as the map is in the game; with its text or not.
    hud: &mut dyn FnMut(&mut wgpu::CommandEncoder, &wgpu::TextureView, bool),
) -> anyhow::Result<()> {
    let (width, height) = size;
    // VOXEL_NO_SHAFTS leaves the light shafts out, to see what they add.
    let shafts = std::env::var_os("VOXEL_NO_SHAFTS").is_none();
    // VOXEL_CLOUD_AGE=M shows the clouds as they will be M minutes on in steady weather,
    // having formed and cleared: a few shots a few minutes apart show them changing.
    let mut aged = env.clone();
    if let Some(minutes) = std::env::var("VOXEL_CLOUD_AGE").ok().and_then(|v| v.parse::<f32>().ok()) {
        aged.cloud_churn += minutes * 60.0 * crate::weather::CHURN;
    }
    let env = &aged;
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut renderer = Renderer::new(gpu, format, width, height);
    // VOXEL_CLOCK=S draws the shot as at S seconds on the renderer's clock. Otherwise it
    // is however long loading took, which sways the leaves and moves the water by a
    // different amount each run; two shots to compare need the same S.
    renderer.clock = std::env::var("VOXEL_CLOCK").ok().and_then(|v| v.parse::<f32>().ok());
    streamer.wait_idle(gpu, camera.pos);
    far.wait_ready(gpu, camera.pos);
    impostors.wait_idle(gpu, camera.pos);

    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("screenshot"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    // Rows in a texture-to-buffer copy must be padded to a multiple of 256 bytes.
    let row = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("screenshot readback"),
        size: (row * height) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    let scene = Scene {
        camera,
        env,
        terrain: streamer,
        far,
        impostors,
        particles,
        props,
        shades: &[],
        wind,
        ground,
        lamps,
        shafts,
    };
    renderer.render(gpu, &mut encoder, &view, &scene);
    hud(&mut encoder, &view, true);
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit([encoder.finish()]);

    buffer.slice(..).map_async(wgpu::MapMode::Read, |r| {
        r.expect("screenshot buffer map failed");
    });
    gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
    let data = buffer.slice(..).get_mapped_range()?;
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        let start = (y * row) as usize;
        pixels.extend_from_slice(&data[start..start + (width * 4) as usize]);
    }
    drop(data);
    buffer.unmap();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    image::save_buffer(path, &pixels, width, height, image::ColorType::Rgba8)?;
    log::info!(
        "screenshot {} ({} chunks drawn, {} quads, shadow draws {:?}, multi-draw {}, arena {} MiB, meshes {} MiB, far trees {}/{} boxes, built {}/{} boxes, voxels to {:.0})",
        path.display(),
        renderer.stats.chunks_drawn,
        renderer.stats.quads_drawn,
        renderer.stats.shadow_draws,
        renderer.multi_draw,
        streamer.stats.arena_bytes >> 20,
        streamer.stats.mesh_bytes >> 20,
        renderer.stats.impostor_boxes,
        impostors.boxes,
        renderer.stats.built_boxes,
        impostors.built.boxes,
        streamer.bands()[LEVELS - 1]
    );
    // The frames that follow, as a game would draw them a sixtieth of a second apart, each
    // saved beside the first: VOXEL_SEQUENCE=N, with VOXEL_SEQUENCE_MOVES naming what
    // moves between them (any of sun, wind, walk, turn, shuttle). For finding flicker.
    if let Some(n) = std::env::var("VOXEL_SEQUENCE").ok().and_then(|s| s.parse::<usize>().ok()) {
        let moves = std::env::var("VOXEL_SEQUENCE_MOVES").unwrap_or_default();
        let has = |what: &str| moves.split(',').any(|m| m == what);
        let clock = renderer.clock.unwrap_or_else(|| renderer.start.elapsed().as_secs_f32());
        renderer.clock = Some(clock);
        let ahead = (camera.forward() * Vec3::new(1.0, 0.0, 1.0)).normalize_or_zero();
        for k in 1..=n {
            let dt = k as f32 / 60.0;
            if has("wind") {
                renderer.clock = Some(clock + dt);
            }
            let mut env = env.clone();
            if has("sun") {
                let later = Environment::at(env.time + dt / (40.0 * 60.0) * 24.0, env.date);
                (env.sun_dir, env.sun_pos, env.sun_elevation) = (later.sun_dir, later.sun_pos, later.sun_elevation);
            }
            let mut camera = Camera { pos: camera.pos, ..*camera };
            if has("walk") {
                camera.pos += (ahead * m(1.5) * dt).as_dvec3();
            }
            if has("turn") {
                camera.yaw += 0.6 * dt;
            }
            // Away and back again: every other frame is drawn from where the first was.
            if has("shuttle") && k % 2 == 1 {
                camera.pos += (ahead * m(1.5) * dt).as_dvec3();
                camera.yaw += 0.6 * dt;
            }
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            let scene = Scene { camera: &camera, env: &env, terrain: streamer, far, impostors, particles, props, shades: &[], lamps, wind, ground, shafts };
            renderer.render(gpu, &mut encoder, &view, &scene);
            encoder.copy_texture_to_buffer(
                target.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(height) },
                },
                wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            );
            gpu.queue.submit([encoder.finish()]);
            buffer.slice(..).map_async(wgpu::MapMode::Read, |r| {
                r.expect("screenshot buffer map failed");
            });
            gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
            let data = buffer.slice(..).get_mapped_range()?;
            let mut pixels = Vec::with_capacity((width * height * 4) as usize);
            for y in 0..height {
                let start = (y * row) as usize;
                pixels.extend_from_slice(&data[start..start + (width * 4) as usize]);
            }
            drop(data);
            buffer.unmap();
            image::save_buffer(path.with_extension(format!("{k:03}.png")), &pixels, width, height, image::ColorType::Rgba8)?;
        }
    }
    // Headless frame timing without a window: VOXEL_TIME_FRAMES=N renders the same
    // view N more times, waiting for the GPU after each, and logs how long they took.
    if let Some(n) = std::env::var("VOXEL_TIME_FRAMES").ok().and_then(|s| s.parse::<usize>().ok()) {
        let mut times = Vec::with_capacity(n);
        for _ in 0..n {
            let t = std::time::Instant::now();
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            let scene = Scene { camera, env, terrain: streamer, far, impostors, particles, props, shades: &[], lamps, wind, ground, shafts };
            renderer.render(gpu, &mut encoder, &view, &scene);
            hud(&mut encoder, &view, false);
            gpu.queue.submit([encoder.finish()]);
            gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
            times.push(t.elapsed().as_secs_f32() * 1000.0);
        }
        times.sort_by(f32::total_cmp);
        if !times.is_empty() {
            log::info!("headless frames: median {:.2} ms, p95 {:.2} ms over {n}", times[n / 2], times[n * 95 / 100]);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Looking for the ground only in the columns around the camera finds what walking
    /// every loaded chunk found, wherever in its column the camera is.
    #[test]
    fn band_scale_finds_the_ground_in_the_columns_around_the_camera() {
        // A rough patch of land, chunk tops from 0 to 7 chunks up, some columns empty.
        let mut columns = std::collections::HashMap::new();
        for x in -6i32..6 {
            for z in -6..6 {
                let h = (x * 7 + z * 13).rem_euclid(9);
                if h < 8 {
                    columns.insert((x, z), (0..=h).map(|y| IVec3::new(x, y, z)).collect::<Vec<_>>());
                }
            }
        }
        let size = CHUNK as f64;
        let mut tried = 0;
        for i in 0..400 {
            // Across columns and on and near their edges, at heights either side of the reach.
            let f = |k: i32| (k as f64 * 0.37).rem_euclid(1.0);
            let x = (i % 20 - 10) as f64 * size * 0.25 + [0.0, 0.5, size - 0.5, f(i)][i as usize % 4];
            let z = (i / 20 - 10) as f64 * size * 0.25 + [0.0, size - 0.5, 0.5, f(i * 3)][(i / 4) as usize % 4];
            let y = 8.0 * size + m(40.0 + (i % 7) as f32 * 30.0) as f64;
            let camera = DVec3::new(x, y, z);
            let all = band_scale_over(columns.values().flatten().copied(), camera);
            let near = band_scale_over(columns_around(camera).filter_map(|c| columns.get(&c)).flatten().copied(), camera);
            assert_eq!(all, near, "camera at {camera}");
            tried += (all < 1.0) as u32;
        }
        assert!(tried > 50, "only {tried} cameras high enough to matter");
    }

    #[test]
    fn the_snow_floor_is_below_every_snowline_and_gone_once_snow_lies() {
        // snow_depth in ground.wgsl, which the terrain shader skips below the floor.
        let depth = |alt: f32, row: [f32; 4]| {
            let lowland = 0.34 + (1.0 - 0.34) * ((alt - 35.0) / 75.0).clamp(0.0, 1.0);
            row[0] * (1.0 + alt.max(0.0) * 0.01) * lowland + row[2] * (alt - 25.0).max(0.0) * 0.003 + (alt - row[3]).max(0.0) * 0.008
        };
        let summer = [[0.0, 0.0, 0.0, 900.0], [0.0, 0.2, 0.0, 640.0], [0.0, 0.0, 0.0, 2000.0]];
        let floor = snow_floor(&summer);
        assert_eq!(floor, 640.0);
        for alt in [-50.0, 0.0, 300.0, 639.9, 640.0] {
            assert!(summer.iter().all(|&row| depth(alt, row) <= 0.0), "snow at {alt} m below the floor");
        }
        assert!(depth(641.0, summer[1]) > 0.0);
        // Any snow lying, low or on the hills, and it can be anywhere.
        for row in [[0.1, 0.0, 0.0, 900.0], [0.0, 0.0, 0.3, 900.0]] {
            assert!(snow_floor(&[summer[0], row]) < -1.0e29);
        }
    }

    #[test]
    fn the_cloud_field_gives_each_cover_its_share_of_the_sky() {
        let (levels, edges) = cloud_field();
        let field = &levels[0];
        assert_eq!(levels.len(), CLOUD_LEVELS as usize);
        assert_eq!(levels.last().map(Vec::len), Some(1));
        // Wherever the clouds' cycle has come to, as they form in one place and clear in
        // another, the sky as a whole keeps near the cover asked for.
        for churn in [0.0f32, 1.0, 2.5, 4.4] {
            for cover in [0.1f32, 0.3, 0.6, 0.9] {
                let edge = cloud_edge(&edges, cover);
                let share = field.iter().step_by(5).filter(|&&t| cloud_mix(t, churn) > edge).count() as f32 / (field.len() / 5) as f32;
                assert!((share - cover).abs() < 0.05, "cover {cover} at {churn}: {share} of the sky");
            }
        }
        // A sky all cloud leaves no cell out, and more cover never means less cloud.
        assert!(cloud_edge(&edges, 1.0) < 0.0);
        assert!(edges.windows(2).all(|w| w[1] <= w[0]));
        // Each coarser cell bounds the field in the cells under it, wherever the cycle is
        // (as cloud_bound in common.wgsl), to the one at the top.
        let side = CLOUD_PERIOD as usize;
        let bound = |t: [u8; 4], churn: f32| {
            let top = t[0].max(t[1]) as f32 / 255.0;
            (cloud_mix(t, churn) + (t[3] as f32 / 255.0 * CLOUD_TURNS * 0.5).min(1.0) * top).min(top)
        };
        for l in [1, CLOUD_LEVEL as usize, 6] {
            let (coarse, k) = (&levels[l], 1usize << l);
            for (sx, sy) in [(0, 0), (3, 7), (40, 11), (100, 90)].map(|(x, y)| (x % (side >> l), y % (side >> l))) {
                for churn in [0.0f32, 1.3, 3.0, 5.1] {
                    let highest = (0..k).flat_map(|y| (0..k).map(move |x| (x, y))).map(|(x, y)| cloud_mix(field[(sy * k + y) * side + sx * k + x], churn)).fold(0.0, f32::max);
                    assert!(bound(coarse[sy * (side >> l) + sx], churn) >= highest - 1e-5, "level {l} square {sx},{sy} at {churn}");
                }
            }
        }
        let top = levels.last().map(|l| l[0][0].max(l[0][1]));
        assert_eq!(top, field.iter().map(|t| t[0].max(t[1])).max());
    }

    #[test]
    fn every_wash_has_a_place_in_the_globals() {
        let (body, bloom) = wash_tables();
        let n = worldgen::features::plants::washes().len();
        assert!(n > 10 && n < WASH_SLOTS);
        assert_eq!(body[0], [0.0; 4]);
        for i in 1..=n {
            // A plant kind in the whole part, and half a share in the rest.
            assert!(body[i][3].fract() > 0.0 && body[i][3].fract() <= 0.5, "{:?}", body[i]);
            assert!(bloom[i][3].fract() <= 0.5);
            // The flowers' season rides above their kind, and both come back out whole.
            let w = &worldgen::features::plants::washes()[i - 1];
            assert_eq!((bloom[i][3] as u32) >> 3, w.season as u32);
            assert!((bloom[i][3] as u32 & 7) < 8 && (w.season as usize) < SEASON_SLOTS);
            assert!(body[i][..3].iter().all(|c| (0.0..=1.0).contains(c)));
        }
    }
}
