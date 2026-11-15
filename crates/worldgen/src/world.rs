//! The real world: continents generated on demand and turned into voxel chunks.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, OnceLock, RwLock},
};

use glam::{IVec3, Vec3};

use voxel::{
    AIR, CHUNK, Materials, PAD, PaddedChunk, Voxel,
    mesh::{CELLS, Current, PlantBox},
};

use crate::{
    Cascade, ColumnOutput, SpringMark, Surf, World,
    continent::{self, Continent, NO_LAKE, SimConfig},
    features::{self, Library},
    macro_layout::{MACRO_SIZE, cell_desc, cell_of},
    materials as mat,
    noise::{hash3, perlin2, unit},
    scale::{BLOCK, m, m64},
    surface::{self, Column, Cover, Sampler, node},
    trails,
};

type Slot = Arc<OnceLock<Arc<Continent>>>;

pub struct GameWorld {
    seed: u32,
    materials: Materials,
    config: SimConfig,
    /// Tree and feature models, built on first use.
    library: OnceLock<Library>,
    /// Simulated macro-cells. Each slot is filled once, by whichever thread asks first;
    /// others asking for the same cell wait for it rather than simulating it again.
    continents: Mutex<HashMap<(i32, i32), Slot>>,
    /// Climbing plants grown lately, shared by the chunk columns they span.
    climbers: features::climber::PlantCache,
    /// Caves laid out lately, by world cell.
    caves: features::cave::Cache,
    /// Trees and other features placed lately.
    placed: features::PlaceCache,
    /// The moor's tors and stonework laid out lately, by world cell.
    tors: features::tor::Cache,
    /// The columns whose flower has been picked: nothing grows on them until it is
    /// back (see `pick`).
    picked: RwLock<HashSet<(i32, i32)>>,
}

impl GameWorld {
    pub fn new(seed: u32) -> Self {
        Self::with_config(seed, SimConfig::default())
    }

    pub fn with_config(seed: u32, config: SimConfig) -> Self {
        Self {
            seed,
            materials: mat::build(),
            config,
            library: OnceLock::new(),
            continents: Mutex::new(HashMap::new()),
            climbers: Default::default(),
            caves: Default::default(),
            placed: Default::default(),
            tors: Default::default(),
            picked: Default::default(),
        }
    }

    pub fn seed(&self) -> u32 {
        self.seed
    }

    pub fn library(&self) -> &Library {
        self.library.get_or_init(Library::build)
    }

    /// The tors and moor stonework of continent `c` that reach the world square from
    /// `(x0, z0)`, `span` voxels wide.
    pub fn tors_near(&self, c: &Continent, x0: i32, z0: i32, span: i32) -> Vec<Arc<features::tor::Tor>> {
        self.tors.near(self.seed, c, x0, z0, span)
    }

    /// The simulated macro-cell `(mx, mz)`, generating it if needed (about 3 seconds).
    pub fn continent(&self, mx: i32, mz: i32) -> Arc<Continent> {
        let slot = self
            .continents
            .lock()
            .expect("continent map poisoned")
            .entry((mx, mz))
            .or_default()
            .clone();
        slot.get_or_init(|| {
            let mut c = continent::generate(cell_desc(self.seed, mx, mz), self.config);
            c.trails = trails::build(&c, self.seed);
            c.trails.names = crate::names::survey(&c, self.seed);
            Arc::new(c)
        })
        .clone()
    }

    /// The macro-cell if it has already been simulated. Never blocks.
    pub fn continent_if_ready(&self, mx: i32, mz: i32) -> Option<Arc<Continent>> {
        let slot = self
            .continents
            .lock()
            .expect("continent map poisoned")
            .get(&(mx, mz))
            .cloned()?;
        slot.get().cloned()
    }

    /// The surface at one world voxel column. For spawn placement and tools; chunk
    /// generation samples in bulk instead.
    pub fn column(&self, x: i32, z: i32) -> Column {
        let (mx, mz) = cell_of(x, z);
        let c = self.continent(mx, mz);
        let (lx, lz) = (x - mx * MACRO_SIZE, z - mz * MACRO_SIZE);
        Sampler::new(&c, self.seed, lx, lz, 1).column(lx, lz)
    }

    /// The small delights left beside the trails (see [`trails::delights`]) within
    /// `reach` voxels of the column at `(x, z)`, each with the world voxel its foot
    /// stands in. Never blocks: nothing from a continent not yet simulated. This is how
    /// the game finds the wind chimes near the player, for when they have a sound.
    pub fn delights_near(&self, x: i32, z: i32, reach: i32) -> Vec<(trails::delights::Delight, IVec3)> {
        let mut out = Vec::new();
        let (lo, hi) = (cell_of(x - reach, z - reach), cell_of(x + reach, z + reach));
        for mz in lo.1..=hi.1 {
            for mx in lo.0..=hi.0 {
                let Some(c) = self.continent_if_ready(mx, mz) else { continue };
                let origin = IVec3::new(mx * MACRO_SIZE, 0, mz * MACRO_SIZE);
                out.extend(c.trails.delights.iter().map(|d| (d.kind, d.at + origin)).filter(|(_, at)| {
                    let (dx, dz) = ((at.x - x) as i64, (at.z - z) as i64);
                    dx * dx + dz * dz <= reach as i64 * reach as i64
                }));
            }
        }
        out
    }

    /// Picks what grows on the column at `(x, z)`: columns built from now on have nothing
    /// growing there, and [`Self::flowers_in`] no longer names it. Whoever shows the
    /// column must build it again to see it gone.
    pub fn pick(&self, x: i32, z: i32) {
        self.picked.write().expect("picked columns poisoned").insert((x, z));
    }

    /// Lets the column at `(x, z)` grow again what was picked from it.
    pub fn regrow(&self, x: i32, z: i32) {
        self.picked.write().expect("picked columns poisoned").remove(&(x, z));
    }

    /// The flowers and mushrooms growing on the columns of the square from `(x0, z0)`,
    /// `span` voxels each way, as the chunks have them, each with its column. For
    /// gathering; it leaves out what only the chunk knows (rock standing on the ground,
    /// the turf about a tor, a stone step), so now and then it names a flower that a
    /// boulder sits on. Nothing where the square crosses onto another continent, and
    /// nothing that has been picked.
    pub fn flowers_in(&self, x0: i32, z0: i32, span: i32) -> Vec<((i32, i32), features::plants::Flower)> {
        self.flowers_standing(x0, z0, span).into_iter().map(|(at, f)| ((at.x, at.z), f)).collect()
    }

    /// As [`Self::flowers_in`], each with the voxel its flower stands in: the one above
    /// the ground of its column.
    pub fn flowers_standing(&self, x0: i32, z0: i32, span: i32) -> Vec<(IVec3, features::plants::Flower)> {
        let (mx, mz) = cell_of(x0, z0);
        if span <= 0 || cell_of(x0 + span, z0 + span) != (mx, mz) {
            return Vec::new();
        }
        let Some(c) = self.continent_if_ready(mx, mz) else { return Vec::new() };
        let (lx0, lz0) = (x0 - mx * MACRO_SIZE - 1, z0 - mz * MACRO_SIZE - 1);
        let n = (span + 2) as usize;
        let sampler = Sampler::new(&c, self.seed, lx0, lz0, span + 2);
        let columns: Vec<Column> = (0..n * n).map(|i| sampler.column(lx0 + (i / n) as i32, lz0 + (i % n) as i32)).collect();
        let picked = self.picked.read().expect("picked columns poisoned");
        let mut out = Vec::new();
        for px in 1..n - 1 {
            for pz in 1..n - 1 {
                let col = &columns[px * n + pz];
                let (wx, wz) = (x0 - 1 + px as i32, z0 - 1 + pz as i32);
                if picked.contains(&(wx, wz)) {
                    continue;
                }
                let ground = |x: usize, z: usize| columns[x * n + z].ground;
                let steep = ((ground(px + 1, pz) - ground(px - 1, pz)) * 0.5).hypot((ground(px, pz + 1) - ground(px, pz - 1)) * 0.5);
                let roll = unit(hash3(self.seed ^ 0x5cf, wx, col.top(), wz));
                let patch = crate::surface::ground_patch(self.seed, wx as f64, wz as f64);
                let mut growing = surface_materials(col, steep, roll, patch).0;
                if growing == mat::ground_cover(col.biome) && col.cover == Cover::Land
                    && let Some(f) = features::flower(col.biome, wx, wz, self.seed, roll)
                {
                    growing = f;
                }
                if let Some(f) = features::plants::flower_at(col, growing, wx, wz, self.seed) {
                    out.push((IVec3::new(wx, col.top() + 1, wz), f));
                }
            }
        }
        out
    }

    /// A good place to begin: on the bank of a river in the origin continent, in a
    /// valley with high ground around it, looking at the water.
    pub fn spawn_point(&self) -> Spawn {
        let c = self.continent(0, 0);
        let n = c.size as i32;
        // Score river cells by how much the land rises around them.
        let reach = (24 * n / 1024).max(4);
        let stride = (reach / 4).max(1) as usize;
        let mut best: Option<(f32, usize)> = None;
        for i in (0..c.size * c.size).step_by(3) {
            let q = c.discharge[i];
            if c.ocean[i] || c.lake[i] != NO_LAKE || !(2500.0..=40_000.0).contains(&q) {
                continue;
            }
            let (x, z) = ((i % c.size) as i32, (i / c.size) as i32);
            if c.height[i] < 8.0 || x < reach || z < reach || x >= n - reach || z >= n - reach {
                continue;
            }
            let mut relief = 0.0f32;
            for dz in (-reach..=reach).step_by(stride) {
                for dx in (-reach..=reach).step_by(stride) {
                    let j = ((z + dz) * n + x + dx) as usize;
                    relief = relief.max(c.height[j] - c.height[i]);
                }
            }
            // Hills to look at, but not so steep the start is a gorge.
            let score = relief.min(160.0) + q.ln() * 4.0;
            if best.is_none_or(|b| score > b.0) {
                best = Some((score, i));
            }
        }
        let Some((_, cell)) = best else {
            // No suitable river: the middle of the continent will do.
            return Spawn {
                x: MACRO_SIZE / 2,
                z: MACRO_SIZE / 2,
                yaw: 0.0,
            };
        };

        // Walk outward from the river until there is dry, open ground just above it.
        let river = node(&c, cell);
        let sampler_at = |x: i32, z: i32| Sampler::new(&c, self.seed, x, z, 1).column(x, z);
        for ring in 0..40 {
            let dist = m(6.4) + ring as f32 * m(1.2);
            for step in 0..16 {
                let angle = step as f32 / 16.0 * std::f32::consts::TAU;
                let (x, z) = (
                    (river.x + angle.cos() * dist) as i32,
                    (river.y + angle.sin() * dist) as i32,
                );
                let col = sampler_at(x, z);
                if col.cover == Cover::Land && col.water_surface().is_none() {
                    let (dx, dz) = (river.x - x as f32, river.y - z as f32);
                    return Spawn {
                        x,
                        z,
                        yaw: dz.atan2(dx).to_degrees(),
                    };
                }
            }
        }
        Spawn {
            x: river.x as i32,
            z: river.y as i32,
            yaw: 0.0,
        }
    }
}

/// Where and which way a new player starts.
#[derive(Clone, Copy, Debug)]
pub struct Spawn {
    pub x: i32,
    pub z: i32,
    /// Degrees; 0 looks along +X.
    pub yaw: f32,
}

/// The overhanging rock over each padded column of a chunk column (see
/// [`surface::overhang`]), indexed like `columns`.
pub(crate) fn overhangs(seed: u32, sampler: &Sampler, columns: &[Column], (lx0, lz0): (i32, i32), (wx0, wz0): (i32, i32)) -> Vec<Option<(i32, i32)>> {
    use surface::overhang::{REACH, Spot, reach, slab};
    // Every column the pass looks at: the padded ones and a ring around them.
    let ring = REACH + 1;
    let n = PAD as i32 + 2 * ring;
    let spot = |c: &Column| Spot {
        top: c.top(),
        cliff: c.cover == Cover::Cliff,
        open: c.water_surface().is_none() && c.cover != Cover::Trail && c.clear > m(1.5),
        grown: matches!(c.cover, Cover::Land | Cover::Bank),
    };
    // The ring is sampled only where a slab is found to need it, which is seldom.
    let spots: Vec<std::cell::OnceCell<Spot>> = (0..n * n).map(|_| std::cell::OnceCell::new()).collect();
    let at = |wx: i32, wz: i32| {
        let (x, z) = (wx - wx0, wz - wz0);
        *spots[((x + ring) * n + z + ring) as usize].get_or_init(|| {
            if (0..PAD as i32).contains(&x) && (0..PAD as i32).contains(&z) {
                spot(&columns[x as usize * PAD + z as usize])
            } else {
                spot(&sampler.column(lx0 + x, lz0 + z))
            }
        })
    };
    let reaches: Vec<std::cell::OnceCell<i32>> = (0..n * n).map(|_| std::cell::OnceCell::new()).collect();
    let reach_at = |wx: i32, wz: i32| *reaches[((wx - wx0 + ring) * n + wz - wz0 + ring) as usize].get_or_init(|| reach(seed, wx, wz));
    let mut out = Vec::with_capacity(PAD * PAD);
    for px in 0..PAD as i32 {
        for pz in 0..PAD as i32 {
            out.push(slab(wx0 + px, wz0 + pz, &at, &reach_at));
        }
    }
    out
}

/// Takes out every small plant on a column of ground that something solid now stands
/// over within the plant's height: the plants are grown before the bridges, buildings
/// and trees are put down, and a reed on the bank under a bridge would otherwise stand
/// up through its deck. The whole plant goes, not only what pokes through, or a stalk
/// would be left ending at the underside of the planks. `columns` are the padded
/// columns from `wx0`, `wz0`; plants in a cave, below the ground, are left be.
fn headroom(out: &[(i32, PaddedChunk)], mats: &Materials, columns: &[Column], wx0: i32, wz0: i32, plants: &mut Vec<PlantBox>) {
    use features::attach::Voxels;
    if plants.is_empty() {
        return;
    }
    let voxels = features::attach::ColumnVoxels::new(out, wx0, wz0);
    let top = |x: i32, z: i32| columns[(x - wx0) as usize * PAD + (z - wz0) as usize].top();
    let solid = |p: IVec3| mats.get(voxels.voxel(p)).kind == voxel::Kind::Solid;
    let mut blocked: HashSet<(i32, i32)> = HashSet::new();
    for b in plants.iter() {
        let (lo, hi) = (IVec3::from_array(b.lo).div_euclid(IVec3::splat(CELLS)), (IVec3::from_array(b.hi) - 1).div_euclid(IVec3::splat(CELLS)));
        let ground = top(lo.x, lo.z);
        if lo.y < ground || blocked.contains(&(lo.x, lo.z)) {
            continue;
        }
        let over = (lo.x..=hi.x).any(|x| (lo.z..=hi.z).any(|z| (lo.y.max(ground + 1)..=hi.y).any(|y| solid(IVec3::new(x, y, z)))));
        if over {
            blocked.insert((lo.x, lo.z));
        }
    }
    if !blocked.is_empty() {
        plants.retain(|b| {
            let lo = IVec3::from_array(b.lo).div_euclid(IVec3::splat(CELLS));
            lo.y < top(lo.x, lo.z) || !blocked.contains(&(lo.x, lo.z))
        });
    }
}

/// The darker, weathered form of a cliff rock, for the streaks water leaves down a face.
fn stained(rock: Voxel) -> Voxel {
    match rock {
        mat::STONE | mat::ROCK_BROWN => mat::ROCK_DARK,
        mat::ROCK_PALE => mat::STONE,
        mat::ROCK_DARK => mat::SLATE,
        mat::SANDSTONE => mat::SANDSTONE_DARK,
        mat::SANDSTONE_RED => mat::RED_ROCK_DARK,
        mat::SANDSTONE_DARK => mat::ROCK_BROWN,
        other => other,
    }
}

/// Picks the voxel for the top of a column and for the layers just beneath it.
///
/// `roll` is per-voxel noise; `patch` varies smoothly over several voxels.
fn surface_materials(col: &Column, steepness: f32, roll: f32, patch: f32) -> (Voxel, Voxel) {
    let cover = mat::ground_cover(col.biome);
    match col.cover {
        // Beds change in broad patches: speckle shows through the water as static.
        Cover::RiverBed => (if patch < 0.62 { mat::GRAVEL } else { mat::SAND }, mat::GRAVEL),
        Cover::LakeBed => (if patch < 0.55 { mat::MUD } else { mat::SAND }, mat::MUD),
        Cover::Beach if col.biome == crate::biome::Biome::Volcanic => (mat::BLACK_SAND, mat::BLACK_SAND),
        Cover::SeaFloor | Cover::Beach => (mat::SAND, mat::SAND),
        Cover::Trail => trails::tread::surface(col, roll, patch),
        // Cliff faces take their colour from the rock layer at each height (see the fill
        // loop). The tops of ledges, which are nearly level, keep some soil.
        Cover::Cliff if steepness < 0.9 && roll < 0.55 => (cover, mat::STONE),
        Cover::Cliff => (mat::STONE, mat::STONE),
        Cover::Scree => (if roll < 0.8 { mat::SCREE } else { mat::GRAVEL }, mat::SCREE),
        // Coasts: shingle with a little sand between the stones, boulders, the dark wet
        // rock of a wave-cut shelf, and dune sand.
        Cover::Shingle => (if roll < 0.72 { mat::GRAVEL } else if roll < 0.9 { mat::SCREE } else { mat::SAND }, mat::GRAVEL),
        Cover::Boulders => (if patch < 0.6 { mat::STONE } else { mat::ROCK_DARK }, mat::STONE),
        Cover::Shelf => (if patch < 0.5 || roll < 0.2 { mat::ROCK_DARK } else { mat::STONE }, mat::STONE),
        Cover::Dune if matches!(col.biome, crate::biome::Biome::Desert | crate::biome::Biome::DuneSea) => (mat::DUNE_SAND, mat::DUNE_SAND),
        Cover::Dune if col.biome == crate::biome::Biome::Volcanic => (mat::BLACK_SAND, mat::BLACK_SAND),
        Cover::Dune => (mat::SAND, mat::SAND),
        // Wet stone and moss around a spring, with a little mud.
        Cover::Spring => (if roll < 0.4 { mat::MOSSY_STONE } else if roll < 0.6 { mat::STONE } else if roll < 0.88 { mat::MOSS } else { mat::MUD }, mat::STONE),
        // Banks take the ground of the land around them: grass with a little bare earth
        // in green country, sand in the desert, mud only where the land is mud anyway.
        // Flats barely above the sea are wet sand, the grass above them reaching down in
        // the odd tongue.
        Cover::Bank if col.biome.arid() => (cover, mat::subsoil(cover)),
        // The edge frays over a voxel or two whatever the slope, never into a band of speckle.
        Cover::Bank if crate::surface::flat_is_sand(col.ground + (roll - 0.5) * ((steepness + 0.1) * 2.5).min(0.6), patch) => (mat::SAND, mat::SAND),
        Cover::Bank if roll < 0.10 && col.river < m(1.4) && mat::subsoil(cover) == mat::DIRT => (mat::DIRT, mat::DIRT),
        Cover::Bank | Cover::Land => {
            // Ground too steep to hold soil shows bare rock.
            // Soil gives way to rock between about 35 and 50 degrees, raggedly.
            let rocky = steepness > 0.72 + roll * 0.45;
            if rocky {
                let (light, dark) = mat::bedrock(col.biome);
                (if roll < 0.25 { dark } else { light }, light)
            } else if col.temperature < 0.10 + (roll - 0.5) * 0.03 {
                (mat::SNOW, mat::STONE)
            } else {
                (cover, mat::subsoil(cover))
            }
        }
    }
}

/// Where a used trail climbs steeply, the edge of each step of the voxel ground (a trail
/// column standing above the next trail column along) is a log or a stone laid across
/// the way, so the climb reads as a flight of rough steps. The columns at one height on
/// a levelled trail run across it, so each edge is a row. Returns the step and which way
/// is down from it. `columns` is a square `n` wide. Needs all four neighbours, so only for
/// the columns inside its outermost ring.
#[allow(clippy::too_many_arguments)]
pub(crate) fn step_nose(
    columns: &[Column],
    n: usize,
    px: usize,
    pz: usize,
    steep: f32,
    patch: f32,
    seed: u32,
    wx0: i32,
    wz0: i32,
) -> Option<trails::tread::StepAt> {
    let col = &columns[px * n + pz];
    let w = col.worn.as_ref()?;
    if steep < trails::tread::STEP_GRADE || px == 0 || pz == 0 || px == n - 1 || pz == n - 1 {
        return None;
    }
    let top = col.top();
    // Down the trail first, so a step at a bend faces the way the trail runs.
    let mut sides = [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)];
    if w.dir.y.abs() > w.dir.x.abs() {
        sides.rotate_left(2);
    }
    let down = sides.into_iter().find(|&(dx, dz)| {
        let next = &columns[(px as i32 + dx) as usize * n + (pz as i32 + dz) as usize];
        next.cover == Cover::Trail && next.top() < top
    })?;
    let step = trails::tread::step(col, w, patch, seed, wx0 + px as i32, wz0 + pz as i32)?;
    // The log ends on a side across the way where the step's edge does not go on.
    let (ax, az) = (down.1.abs(), down.0.abs());
    let open = [-1i32, 1].map(|s| {
        let next = &columns[(px as i32 + ax * s) as usize * n + (pz as i32 + az * s) as usize];
        next.cover != Cover::Trail || next.top() != top
    });
    Some((step, down, open))
}

/// The log step whose riser stands at the side of trail column `(px, pz)`, given the
/// step at each column (`nose`, as [`step_nose`] finds them). A log is laid in the
/// corner at the foot of the riser, on the column below, rather than along the lip of
/// the one above, so it sits between the two treads the way a log set into a path does
/// instead of perching on the edge of the step. Returns the log, the way from this
/// column to the riser, and where the log ends across the way (as the step's).
pub(crate) fn step_foot(n: usize, px: usize, pz: usize, nose: impl Fn(usize, usize) -> Option<trails::tread::StepAt>) -> Option<trails::tread::StepAt> {
    for (dx, dz) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
        let (x, z) = (px as i32 + dx, pz as i32 + dz);
        if x < 0 || z < 0 || x >= n as i32 || z >= n as i32 {
            continue;
        }
        if let Some((step @ trails::tread::Step::Log(_), down, open)) = nose(x as usize, z as usize)
            && down == (-dx, -dz)
        {
            return Some((step, (dx, dz), open));
        }
    }
    None
}

/// Sees each whole climbing plant a chunk column grows, before it is cut to the column.
pub(crate) type Inspect<'a> = dyn FnMut(&features::climber::Root, &features::attach::Raster) + 'a;

/// What growing climbers on a chunk column needs to know about it.
struct ClimberRun<'a> {
    voxels: &'a features::attach::ColumnVoxels<'a>,
    continent: &'a Continent,
    chunk: (i32, i32),
    local: (i32, i32),
    macro_origin: (i32, i32),
    columns: &'a [Column],
    wx0: i32,
    wz0: i32,
}

impl GameWorld {
    /// Ivy and its relatives for one chunk column. Climbers can start beyond the column
    /// and read the world up to their own reach from there, so this looks further out
    /// than the column does. Adds the air voxels they fill to `taken`.
    fn grow_climbers(&self, run: &ClimberRun, plants: &mut Vec<PlantBox>, taken: &mut HashSet<IVec3>, inspect: &mut Inspect) {
        use features::climber::{self, MARGIN, REACH};
        if !run.columns.iter().any(|c| climber::place_chance(c) > 0.0) {
            return;
        }
        let lib = self.library();
        let (lx0, lz0) = run.local;
        // Features up to MAX_REACH from anything a plant might touch.
        let margin = MARGIN + features::MAX_REACH + 1;
        let sampler = Sampler::new(run.continent, self.seed, lx0 - margin, lz0 - margin, PAD as i32 + 2 * margin);
        let placements = features::place_cached(&self.placed, self.seed, lib, &sampler, lx0 - MARGIN, lz0 - MARGIN, PAD as i32 + 2 * MARGIN, run.macro_origin);
        // Landmarks anywhere a plant here could reach, visited once each and applied in
        // a fixed order, so overlaps resolve the same way every time.
        let (mcx, mcz) = (run.macro_origin.0.div_euclid(CHUNK), run.macro_origin.1.div_euclid(CHUNK));
        let (local_cx, local_cz) = (run.chunk.0 - mcx, run.chunk.1 - mcz);
        let reach_chunks = (MARGIN + PAD as i32).div_euclid(CHUNK) + 1;
        let mut found: Vec<&crate::trails::Stamp> = Vec::new();
        for dz in -reach_chunks..=reach_chunks {
            for dx in -reach_chunks..=reach_chunks {
                for s in run.continent.trails.stamps_at(local_cx + dx, local_cz + dz) {
                    if !found.iter().any(|&f| std::ptr::eq(f, s)) {
                        found.push(s);
                    }
                }
            }
        }
        found.sort_unstable_by_key(|s| (s.lo.y, s.lo.x, s.lo.z, s.hi.y, s.hi.x, s.hi.z, s.voxels.len()));
        let origin = IVec3::new(run.macro_origin.0, 0, run.macro_origin.1);
        let mut stamped: HashMap<IVec3, Voxel> = HashMap::new();
        for s in &found {
            stamped.extend(s.voxels.iter().map(|&(p, v)| (p + origin, v)));
        }
        let world = climber::Surroundings::new(lib, &self.materials, &placements, &sampler, run.macro_origin, &stamped);
        let (mox, moz) = run.macro_origin;
        let edge = REACH + 2;
        let finder = climber::RootFinder {
            world: &world,
            mats: &self.materials,
            seed: self.seed,
            // Only roots whose whole plant stays inside this macro-cell, where every
            // column reading it sees the same continent.
            x0: (run.wx0 - REACH).max(mox + edge),
            z0: (run.wz0 - REACH).max(moz + edge),
            x1: (run.wx0 + PAD as i32 + REACH).min(mox + MACRO_SIZE - edge),
            z1: (run.wz0 + PAD as i32 + REACH).min(moz + MACRO_SIZE - edge),
        };
        let mut roots = Vec::new();
        finder.trees(&mut roots);
        finder.stamps(found.iter().map(|s| (s.voxels.as_slice(), origin)), &mut roots);
        finder.ground(&mut roots);
        finder.rims(&mut roots);
        let before = plants.len();
        climber::grow_column(roots, run.chunk, run.voxels, &world, &self.materials, self.seed, &self.climbers, plants, inspect);
        taken.extend(plants[before..].iter().map(|p| IVec3::from_array(p.lo).div_euclid(IVec3::splat(CELLS))));
    }
}

impl World for GameWorld {
    fn materials(&self) -> &Materials {
        &self.materials
    }

    fn generate_column(&self, cx: i32, cz: i32) -> ColumnOutput {
        self.generate_column_with(cx, cz, &mut |_, _| {})
    }
}

impl GameWorld {
    /// The colour the plant growing on a column gives its ground from afar (see
    /// [`features::plants::wash_at`]), for whoever draws the ground without generating
    /// it: `surface` is the column's top voxel, `(x, z)` where it is in the world.
    pub fn wash_at(&self, col: &Column, surface: Voxel, x: i32, z: i32) -> u8 {
        let mut growing = surface;
        if surface == mat::ground_cover(col.biome) && col.cover == Cover::Land {
            let roll = unit(hash3(self.seed ^ 0x5cf, x, col.top(), z));
            if let Some(f) = features::flower(col.biome, x, z, self.seed, roll) {
                growing = f;
            }
        }
        features::plants::wash_at(col, growing, x, z, self.seed)
    }

    /// [`World::generate_column`] without the small plants: the same voxels, currents,
    /// trees and so on, and `plants` left empty. Growing the plants is most of the work
    /// of a column, so this is several times quicker, for whoever only needs the shape
    /// of the world: collision, or terrain too far off for its plants to be drawn. (The
    /// topmost chunks may be missing where only plants reached into them.)
    pub fn generate_column_bare(&self, cx: i32, cz: i32) -> ColumnOutput {
        self.generate(cx, cz, false, &mut |_, _| {})
    }

    /// [`World::generate_column`], showing `inspect` every climbing plant whole.
    pub(crate) fn generate_column_with(&self, cx: i32, cz: i32, inspect: &mut Inspect) -> ColumnOutput {
        self.generate(cx, cz, true, inspect)
    }

    fn generate(&self, cx: i32, cz: i32, planted: bool, inspect: &mut Inspect) -> ColumnOutput {
        // A chunk never straddles macro-cells. Its one-voxel border reaches into the
        // uniform deep sea between them, or onto a range held to the same height on
        // both sides of the border (see `continent::border`).
        let (mx, mz) = cell_of(cx * CHUNK, cz * CHUNK);
        let continent = self.continent(mx, mz);
        let (wx0, wz0) = (cx * CHUNK - 1, cz * CHUNK - 1);
        let (lx0, lz0) = (wx0 - mx * MACRO_SIZE, wz0 - mz * MACRO_SIZE);
        // Features up to MAX_REACH away can lean into this column, so sample that far.
        let margin = features::MAX_REACH + 1;
        let sampler = Sampler::new(&continent, self.seed, lx0 - margin, lz0 - margin, PAD as i32 + 2 * margin);

        // The columns of the padded square, and one more all round it: the border columns
        // are the next chunk column's own, and must come out here as they do there, which
        // takes their neighbours on every side (for the slope, and for a trail's steps).
        const WIDE: usize = PAD + 2;
        let wide: Vec<Column> = (0..WIDE * WIDE).map(|i| sampler.column(lx0 - 1 + (i / WIDE) as i32, lz0 - 1 + (i % WIDE) as i32)).collect();
        let mut columns = Vec::with_capacity(PAD * PAD);
        let (mut lo, mut hi) = (i32::MAX, i32::MIN);
        for px in 0..PAD {
            for pz in 0..PAD {
                let col = wide[(px + 1) * WIDE + pz + 1];
                let top = col.top();
                lo = lo.min(top);
                hi = hi.max(top).max(col.water_surface().unwrap_or(i32::MIN));
                columns.push(col);
            }
        }

        // Tors and the moor's old stonework: nothing grows under their rock, and short
        // turf grows at their feet.
        let tors = self.tors.near(self.seed, &continent, wx0, wz0, PAD as i32);
        let mut rock = Vec::new();
        for t in &tors {
            t.raster(wx0, wz0, PAD as i32, |q, v| rock.push((q, v)));
        }
        // The columns with rock standing just above their ground.
        let mut rocky = vec![false; if rock.is_empty() { 0 } else { PAD * PAD }];
        for &(q, v) in &rock {
            lo = lo.min(q.y);
            hi = hi.max(q.y);
            let i = (q.x - wx0) as usize * PAD + (q.z - wz0) as usize;
            if v != AIR && (0..=2).contains(&(q.y - columns[i].top())) {
                rocky[i] = true;
            }
        }

        // Surface and subsurface voxel per column.
        let mut skins = Vec::with_capacity(PAD * PAD);
        let mut plants = Vec::new();
        let mut wash = Vec::with_capacity(PAD * PAD);
        // The columns here whose flower has been picked.
        let picked: Vec<(i32, i32)> = {
            let all = self.picked.read().expect("picked columns poisoned");
            all.iter().copied().filter(|&(x, z)| (wx0..wx0 + PAD as i32).contains(&x) && (wz0..wz0 + PAD as i32).contains(&z)).collect()
        };
        for px in 0..PAD {
            for pz in 0..PAD {
                let top = columns[px * PAD + pz].top();
                // Slope of the ground here, in voxels of rise per voxel.
                let ground = |x: usize, z: usize| wide[x * WIDE + z].ground;
                let gx = (ground(px + 2, pz + 1) - ground(px, pz + 1)) * 0.5;
                let gz = (ground(px + 1, pz + 2) - ground(px + 1, pz)) * 0.5;
                let steep = gx.hypot(gz);
                let roll = unit(hash3(self.seed ^ 0x5cf, wx0 + px as i32, top, wz0 + pz as i32));
                let (wx, wz) = ((wx0 + px as i32) as f64, (wz0 + pz as i32) as f64);
                let patch = crate::surface::ground_patch(self.seed, wx, wz);
                let col = &columns[px * PAD + pz];
                let mut skin = surface_materials(col, steep, roll, patch);
                let (cx_, cz_) = (wx as i32, wz as i32);
                if !tors.is_empty() && col.cover == Cover::Land && skin.0 == mat::ground_cover(col.biome) && tors.iter().any(|t| t.turf(cx_, cz_)) {
                    skin.0 = mat::TURF;
                }
                let rocky = rocky.get(px * PAD + pz).copied().unwrap_or(false);
                let step = step_nose(&wide, WIDE, px + 1, pz + 1, steep, patch, self.seed, wx0 - 1, wz0 - 1);
                if let Some((trails::tread::Step::Stone(v), _, _)) = step {
                    skin.0 = v;
                }
                let mut growing = skin.0;
                if skin.0 == mat::ground_cover(col.biome) && col.cover == Cover::Land
                    && let Some(f) = features::flower(col.biome, wx as i32, wz as i32, self.seed, roll)
                {
                    growing = f;
                }
                // The border columns belong to neighbouring chunk columns.
                let own = !rocky && (1..=CHUNK as usize).contains(&px) && (1..=CHUNK as usize).contains(&pz) && !picked.contains(&(wx as i32, wz as i32));
                wash.push((top, if own { features::plants::wash_at(col, growing, wx as i32, wz as i32, self.seed) } else { 0 }));
                if planted && !rocky && (1..=CHUNK as usize).contains(&px) && (1..=CHUNK as usize).contains(&pz) && !picked.contains(&(wx as i32, wz as i32)) {
                    let grown = plants.len();
                    features::plants::grow(col, growing, wx as i32, wz as i32, self.seed, &mut plants);
                    if plants.len() == grown {
                        features::aquatic::grow(col, wx as i32, wz as i32, self.seed, &mut plants);
                    }
                    if plants.len() == grown {
                        features::pebbles::scatter(col, growing, wx as i32, wz as i32, self.seed, &mut plants);
                    }
                    // A log step lies at the foot of its riser, on this column if the
                    // riser stands beside it.
                    let nose_at = |x: usize, z: usize| {
                        let c = &wide[x * WIDE + z];
                        if c.worn.is_none() || x == 0 || z == 0 || x == WIDE - 1 || z == WIDE - 1 {
                            return None;
                        }
                        let gx = (ground(x + 1, z) - ground(x - 1, z)) * 0.5;
                        let gz = (ground(x, z + 1) - ground(x, z - 1)) * 0.5;
                        let (nx, nz) = ((wx0 - 1 + x as i32) as f64, (wz0 - 1 + z as i32) as f64);
                        let patch = crate::surface::ground_patch(self.seed, nx, nz);
                        step_nose(&wide, WIDE, x, z, gx.hypot(gz), patch, self.seed, wx0 - 1, wz0 - 1)
                    };
                    let foot = step_foot(WIDE, px + 1, pz + 1, nose_at);
                    trails::tread::dress(col, foot, wx as i32, wz as i32, self.seed, &mut plants);
                }
                skins.push(skin);
            }
        }

        let lib = self.library();
        let placements = features::place_cached(
            &self.placed,
            self.seed,
            lib,
            &sampler,
            lx0,
            lz0,
            PAD as i32,
            (mx * MACRO_SIZE, mz * MACRO_SIZE),
        );
        // Only the features whose model, or buttresses, can touch this column.
        let mut placements: Vec<_> = placements
            .into_iter()
            .filter(|p| {
                let reach = p.reach(lib);
                p.pos.x + reach >= wx0
                    && p.pos.x - reach < wx0 + PAD as i32
                    && p.pos.z + reach >= wz0
                    && p.pos.z - reach < wz0 + PAD as i32
            })
            .collect();
        if !tors.is_empty() {
            placements.retain(|p| !tors.iter().any(|t| t.crowds(p.pos)));
        }
        for p in &placements {
            hi = hi.max(p.top(lib));
            lo = lo.min(p.bottom());
        }
        // Landmarks, bridges and lanterns, whose voxels are continent-local.
        let local_chunk = (cx - mx * (MACRO_SIZE / CHUNK), cz - mz * (MACRO_SIZE / CHUNK));
        let stamps: Vec<_> = continent.trails.stamps_at(local_chunk.0, local_chunk.1).collect();
        for s in &stamps {
            hi = hi.max(s.hi.y);
            lo = lo.min(s.lo.y);
        }
        hi = plants.iter().fold(hi, |top, p| top.max((p.lo[1]).div_euclid(CELLS)));
        // Lips of rock jutting out over the ground in front of some faces.
        let overhangs = overhangs(self.seed, &sampler, &columns, (lx0, lz0), (wx0, wz0));
        hi = overhangs.iter().flatten().fold(hi, |top, s| top.max(s.1));
        let caves = features::cave::Carve::new(&self.caves, self.seed, &continent, wx0, wz0);
        lo = lo.min(caves.lowest().unwrap_or(lo));

        // Only chunks that can hold a visible face: from just under the lowest ground
        // to the highest ground or water or tree top.
        let cy_min = (lo - 2).div_euclid(CHUNK);
        let cy_max = hi.div_euclid(CHUNK);
        let mut out = Vec::new();
        for cy in cy_min..=cy_max {
            let base = cy * CHUNK - 1; // world y of padded y = 0
            let mut chunk = PaddedChunk::new();
            for px in 0..PAD {
                for pz in 0..PAD {
                    let col = &columns[px * PAD + pz];
                    let (surface, under) = skins[px * PAD + pz];
                    let top = col.top();
                    let water_top = col.water_surface().map_or(i32::MIN, |k| k - 1);
                    let (wx, wz) = (wx0 + px as i32, wz0 + pz as i32);
                    let rock = (col.cover == Cover::Cliff).then(|| {
                        let (fx, fz) = (wx as f64, wz as f64);
                        let tilt = surface::strata_tilt(self.seed, fx, fz);
                        // Weathering streaks hang down from the lip where water runs
                        // over it: narrow in plan, so long and vertical on the face. Only
                        // a few, and only where water gathers.
                        let streak = perlin2(self.seed ^ 0x57b1, fx / m64(2.6), fz / m64(2.6));
                        let wet = perlin2(self.seed ^ 0x57b3, fx / m64(30.0), fz / m64(30.0));
                        let run = (perlin2(self.seed ^ 0x57b2, fx / m64(9.0), fz / m64(9.0)) * 0.5 + 0.5).clamp(0.0, 1.0);
                        let stain = if streak > 0.4 && wet > 0.05 { m(1.5) + m(9.0) * run } else { 0.0 };
                        (tilt, stain)
                    });
                    // Layer edges are ragged: some voxels take the layer above or below.
                    let rock_at = |y: i32, (tilt, stain): (f32, f32)| {
                        let h = hash3(self.seed ^ 0x57a9, wx, y, wz);
                        let jitter = if unit(h) < 0.25 { (h % 3) as f32 - 1.0 } else { 0.0 };
                        // Beds of uneven thickness (see `strata_height`); some carry a
                        // thin bed of other rock in their upper half.
                        let s = surface::strata_height(self.seed, y as f32 + jitter, tilt);
                        let band = (s / 4.0).floor() as i32;
                        let mut ly = s.floor() as i32;
                        if s - band as f32 * 4.0 >= 2.0 && unit(hash3(self.seed ^ 0x57b0, band, 5, 0)) < 0.28 {
                            ly += 4 * 37;
                        }
                        let rock = mat::strata(self.seed, col.biome, ly, 0.0);
                        if (top - y) as f32 >= stain { rock } else { stained(rock) }
                    };
                    let slab = overhangs.get(px * PAD + pz).copied().flatten();
                    let slab_rock = slab.map(|_| rock.unwrap_or((surface::strata_tilt(self.seed, wx as f64, wz as f64), 0.0)));
                    for (py, v) in chunk.column_mut(px, pz).iter_mut().enumerate() {
                        let y = base + py as i32;
                        *v = if let (Some((a, b)), Some(r)) = (slab, slab_rock)
                            && (a..=b).contains(&y)
                        {
                            rock_at(y, r)
                        } else if y > top {
                            if y <= water_top { mat::WATER } else { AIR }
                        } else if y == top {
                            match rock {
                                Some(r) if surface == mat::STONE => rock_at(y, r),
                                _ => surface,
                            }
                        } else if let Some(r) = rock
                            && under == mat::STONE
                        {
                            // All the way down: a face can be far taller than the soil.
                            rock_at(y, r)
                        } else if y >= top - 3 * BLOCK {
                            under
                        } else {
                            mat::STONE
                        };
                    }
                }
            }
            out.push((cy, chunk));
        }
        caves.apply(self.seed, &sampler, &mut out, cy_min, &columns, (wx0, wz0), (lx0, lz0), &mut plants, &mut placements);

        // Which way the water runs in each column, for its surface and what floats on it.
        let mut currents = Vec::new();
        for (i, col) in columns.iter().enumerate() {
            if col.water_surface().is_none() {
                continue;
            }
            let (px, pz) = ((i / PAD) as i32, (i % PAD) as i32);
            if let Some(v) = sampler.current(lx0 + px, lz0 + pz, col) {
                if currents.is_empty() {
                    currents = vec![Current::default(); PAD * PAD];
                }
                currents[i] = Current { x: v.x, z: v.y, river: true };
            }
        }

        // Cascade lips: water columns whose neighbour's water surface is lower.
        let mut cascades = Vec::new();
        for px in 1..PAD - 1 {
            for pz in 1..PAD - 1 {
                let Some(k) = columns[px * PAD + pz].water_surface() else { continue };
                let (mut total, mut dir) = (0.0f32, Vec3::ZERO);
                for (dx, dz) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                    let n = &columns[(px as i32 + dx) as usize * PAD + (pz as i32 + dz) as usize];
                    if let Some(nk) = n.water_surface()
                        && nk < k
                    {
                        let drop = (k - nk) as f32;
                        total += drop;
                        dir += Vec3::new(dx as f32, 0.0, dz as f32) * drop;
                    }
                }
                if total > 0.0 {
                    // A river goes over the lip the way it runs, however the pool below
                    // happens to meet the voxels around it.
                    let mut dir = dir.normalize_or_zero();
                    let current = currents.get(px * PAD + pz).map_or(Vec3::ZERO, |c| Vec3::new(c.x, 0.0, c.z));
                    let downstream = current.normalize_or_zero();
                    if downstream.dot(dir) > 0.0 {
                        dir = downstream;
                    }
                    cascades.push(Cascade {
                        pos: Vec3::new((wx0 + px as i32) as f32 + 0.5, k as f32, (wz0 + pz as i32) as f32 + 0.5),
                        dir,
                        drop: total,
                        speed: current.length(),
                    });
                }
            }
        }

        // Surf: sea water beside rock that stands out of it, thinned to a few per column.
        const MAX_SURF: usize = 8;
        let mut surf = Vec::new();
        'surf: for px in (1..PAD - 1).step_by(3) {
            for pz in (1..PAD - 1).step_by(3) {
                if columns[px * PAD + pz].water_surface() != Some(0) {
                    continue;
                }
                let mut dir = Vec3::ZERO;
                for (dx, dz) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1), (2, 0), (-2, 0), (0, 2), (0, -2)] {
                    let (x, z) = (px as i32 + dx, pz as i32 + dz);
                    if x < 0 || z < 0 || x >= PAD as i32 || z >= PAD as i32 {
                        continue;
                    }
                    let n = &columns[x as usize * PAD + z as usize];
                    if matches!(n.cover, Cover::Shelf | Cover::Boulders | Cover::Cliff) && n.water_surface().is_none() {
                        dir += Vec3::new(dx as f32, 0.0, dz as f32);
                    }
                }
                if dir != Vec3::ZERO {
                    surf.push(Surf {
                        pos: Vec3::new((wx0 + px as i32) as f32 + 0.5, 0.0, (wz0 + pz as i32) as f32 + 0.5),
                        dir: dir.normalize(),
                    });
                    if surf.len() >= MAX_SURF {
                        break 'surf;
                    }
                }
            }
        }

        // Springs whose pool centre lies inside this column (not its padded border).
        let springs = sampler
            .springs()
            .iter()
            .filter(|s| {
                let (x, z) = (s.pos.x.floor() as i32 - lx0, s.pos.y.floor() as i32 - lz0);
                (1..=CHUNK).contains(&x) && (1..=CHUNK).contains(&z)
            })
            .map(|s| SpringMark {
                pos: Vec3::new(s.pos.x + (wx0 - lx0) as f32, s.water as f32, s.pos.y + (wz0 - lz0) as f32),
                radius: s.radius,
            })
            .chain(sampler.heads().iter().filter_map(|h| {
                // A stream with no pool still bubbles up from the voxel of water at its
                // head, if that column holds the stream's water.
                let (x, z) = (h.x.floor() as i32 - lx0, h.y.floor() as i32 - lz0);
                if !(1..=CHUNK).contains(&x) || !(1..=CHUNK).contains(&z) {
                    return None;
                }
                let k = columns[x as usize * PAD + z as usize].water_surface()?;
                Some(SpringMark { pos: Vec3::new(h.x + (wx0 - lx0) as f32, k as f32, h.y + (wz0 - lz0) as f32), radius: 0.5 })
            }))
            .collect();

        let mut put = |q: IVec3, v: Voxel, force: bool| {
            let (px, pz) = (q.x - wx0, q.z - wz0);
            // Height above the bottom of the lowest chunk (not its padded border).
            let py = q.y - cy_min * CHUNK;
            if px < 0 || pz < 0 || py < 0 || px >= PAD as i32 || pz >= PAD as i32 {
                return;
            }
            let (ci, ly) = ((py / CHUNK) as usize, py.rem_euclid(CHUNK) as usize);
            // Chunk `ci` holds padded y 0..PAD; rows 0 and 33 are the borders
            // of neighbouring chunks and are written there too.
            let mut write = |chunk_index: usize, padded_y: usize| {
                if let Some((_, chunk)) = out.get_mut(chunk_index) {
                    let (ux, uz) = (px as usize, pz as usize);
                    if force || chunk.get(ux, padded_y, uz) == AIR {
                        chunk.set(ux, padded_y, uz, v);
                    }
                }
            };
            // World y maps to padded y + 1 in its own chunk...
            write(ci, ly + 1);
            // ...and to the borders of the chunks below and above.
            if ly == 0 && ci > 0 {
                write(ci - 1, PAD - 1);
            }
            if ly == CHUNK as usize - 1 {
                write(ci + 1, 0);
            }
        };
        for p in &placements {
            features::stamp(lib, &self.materials, p, &mut put);
        }
        for &(q, v) in &rock {
            put(q, v, true);
        }
        // Built things go last: they clear whatever grew or stood in their way.
        let origin = IVec3::new(mx * MACRO_SIZE, 0, mz * MACRO_SIZE);
        for s in &stamps {
            for &(p, v) in &s.voxels {
                put(p + origin, v, true);
            }
        }
        headroom(&out, &self.materials, &columns, wx0, wz0, &mut plants);
        if !planted {
            // Whatever the caves grew goes with the rest of the plants.
            plants.clear();
            return ColumnOutput {
                chunks: out,
                cascades,
                springs,
                surf,
                currents,
                plants,
                wash,
                trees: placements,
            };
        }
        // Their fine parts, each box grown by the chunk column its corner is in, so a part
        // straddling a border is drawn once. Before the moss, which keeps off them.
        let cells = IVec3::splat(CELLS);
        for s in &stamps {
            for b in &s.fine {
                let (lo, hi) = (IVec3::from_array(b.lo) + origin * cells, IVec3::from_array(b.hi) + origin * cells);
                let q = lo.div_euclid(cells);
                if (wx0 + 1..=wx0 + CHUNK).contains(&q.x) && (wz0 + 1..=wz0 + CHUNK).contains(&q.z) {
                    plants.push(PlantBox { lo: lo.to_array(), hi: hi.to_array(), ..*b });
                }
            }
        }
        // The fine strands roots end in, drawn by the chunk column each box's corner is in.
        let mut strands = Vec::new();
        for p in &placements {
            if let Some(roots) = &p.roots {
                strands.clear();
                roots.strands(p.pos, &mut strands);
                for b in &strands {
                    let q = IVec3::from_array(b.lo).div_euclid(cells);
                    if (wx0 + 1..=wx0 + CHUNK).contains(&q.x) && (wz0 + 1..=wz0 + CHUNK).contains(&q.z) {
                        plants.push(*b);
                    }
                }
            }
        }
        // Now the world's shape is final, things can grow on it.
        let voxels = features::attach::ColumnVoxels::new(&out, wx0, wz0);
        let mut standing: HashSet<IVec3> = plants.iter().map(|p| IVec3::from_array(p.lo).div_euclid(IVec3::splat(CELLS))).collect();
        self.grow_climbers(
            &ClimberRun {
                voxels: &voxels,
                continent: &continent,
                chunk: (cx, cz),
                local: (lx0, lz0),
                macro_origin: (mx * MACRO_SIZE, mz * MACRO_SIZE),
                columns: &columns,
                wx0,
                wz0,
            },
            &mut plants,
            &mut standing,
            inspect,
        );
        let soil = features::moss::Soil {
            columns: &columns,
            x0: wx0,
            z0: wz0,
            seed: self.seed,
            taken: &standing,
        };
        let before_moss = plants.len();
        features::moss::grow(&voxels, &self.materials, &soil, &mut plants);
        // Bark, twigs and leaf clusters, clear of the moss and climbers already there.
        if placements.iter().any(|p| features::tree_detail::has_detail(p.species)) {
            features::tree_detail::occupied(&plants[before_moss..], &mut standing);
            let grove = features::tree_detail::Grove {
                columns: &columns,
                x0: wx0,
                z0: wz0,
                seed: self.seed,
                taken: &standing,
            };
            features::tree_detail::grow(&voxels, &self.materials, &grove, &mut plants);
        }
        // What the bushes, the ivy and the reeds bore on a picked column has been taken.
        if !picked.is_empty() {
            plants.retain(|p| !(features::borne(p.voxel) && picked.contains(&(p.lo[0].div_euclid(CELLS), p.lo[2].div_euclid(CELLS)))));
        }
        ColumnOutput {
            chunks: out,
            cascades,
            springs,
            surf,
            currents,
            plants,
            wash,
            trees: placements,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::continent::SimConfig;

    #[test]
    fn every_column_grows_a_climbing_plant_the_same_way() {
        // A plant near a chunk border is grown by each column it reaches, each reading
        // the world through its own wider window. They must all grow it identically, or
        // stems would break off at the border. That goes for the ones winding round a
        // trunk as much as any: a stem that goes round the back of a tree comes out
        // again in whichever column the front of it is in.
        let w = GameWorld::with_config(5, SimConfig { size: 256 });
        let centre = MACRO_SIZE / CHUNK / 2;
        let mut seen: HashMap<IVec3, Vec<(IVec3, Voxel)>> = HashMap::new();
        let (mut shared, mut winding) = (0, 0);
        'search: for cz in (centre - 200..centre + 200).step_by(5) {
            for cx in (centre - 200..centre + 200).step_by(5) {
                for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let mut grown = Vec::new();
                    // Each column grows its plants itself, not from the shared cache.
                    w.climbers.clear();
                    w.generate_column_with(cx + dx, cz + dz, &mut |root, raster| grown.push((root.cell, root.wind, raster.cells())));
                    for (root, wind, cells) in grown {
                        match seen.get(&root) {
                            Some(before) => {
                                assert_eq!(before, &cells, "plant at {root} grew differently seen from column {},{}", cx + dx, cz + dz);
                                shared += 1;
                                // One worth the name: wound round, and big enough to show it.
                                winding += (wind.abs() > 0.3 && cells.len() > 400) as u32;
                            }
                            None => {
                                seen.insert(root, cells);
                            }
                        }
                    }
                }
                if shared >= 12 && winding >= 4 {
                    break 'search;
                }
            }
        }
        assert!(shared >= 3, "only {shared} plants were seen from two columns");
        assert!(winding >= 2, "only {winding} winding plants were seen from two columns");
    }
}
