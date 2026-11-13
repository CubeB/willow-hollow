mod arena;
mod audio;
mod camera;
mod character;
mod env;
mod farview;
mod gfx;
mod impostors;
mod lod;
mod minimap;
mod overlay;
mod particles;
mod player;
mod profile;
mod render;
mod season;
mod settings;
mod weather;
mod terrain;
mod vehicles;

use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{Receiver, Sender, channel},
    },
    time::Instant,
};

use anyhow::{Context, Result, bail};
use audio::{Audio, Ground};
use glam::DVec3;
use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, DeviceId, ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{CursorGrabMode, Window, WindowId},
};
use worldgen::{
    World,
    scale::{OLD_VOXEL, m, m64},
    world::GameWorld,
};

use camera::{Camera, FlyController};
use env::Environment;
use farview::FarView;
use gfx::{Gpu, WindowSurface};
use impostors::Impostors;
use minimap::Minimap;
use overlay::Overlay;
use particles::{Particles, Surroundings};
use player::{Controls, Mode, Player, WorldVoxels};
use profile::CpuTimer;
use render::Renderer;
use settings::{Overrides, Progress, Settings, Store, Travel};
use terrain::Streamer;
use vehicles::Fleet;

struct Options {
    /// Choices that take precedence over the saved settings, for this run.
    overrides: Overrides,
    /// Camera start: world position, then yaw and pitch in degrees.
    pos: DVec3,
    /// If set, start at this (x, z) and this far above the ground or water there.
    over: Option<(i32, i32, f64)>,
    /// Start flying instead of on foot. Implied by --pos and --over.
    fly: bool,
    /// On a screenshot, stand the player's figure at the start and look from behind it,
    /// or from this many degrees round it.
    third: Option<f32>,
    /// What the figure on a screenshot is doing: running at this speed, or gliding with
    /// the wing this far open.
    figure_speed: f32,
    figure_glide: Option<f32>,
    /// How far the figure on a screenshot has swung its furled glider up from its back.
    figure_furl: f32,
    look: (f32, f32),
    /// Time of day in hours, if not the one left off at.
    time: Option<f32>,
    /// Time of year, 0 to 1 from the spring equinox, if not the one left off at.
    date: Option<f32>,
    /// Weather to hold for this run (clear, overcast, rain, snow, storm, fog).
    weather: Option<weather::Forced>,
    /// Start from the spawn point, ignoring any saved position.
    fresh: bool,
    /// Render one frame to this PNG without opening a window, then exit.
    screenshot: Option<PathBuf>,
    /// Write the map of the continent around the start to this PNG, then exit.
    map: Option<PathBuf>,
    audio: bool,
    size: (u32, u32),
    /// Exit after this many frames (smoke tests).
    frames: Option<u64>,
    /// Fly straight ahead at this many metres a second, to measure streaming.
    drift: Option<f32>,
    /// Turn while drifting, so as to fly a circle of this radius in metres.
    circle: Option<f32>,
    /// Seconds of ambient life (leaves, birds, spray) simulated before a screenshot.
    ambient: f32,
    /// Draw the map on a screenshot: the corner map, or the large map this many voxels
    /// across.
    shot_map: Option<Option<f64>>,
    /// Where the player is taken to have walked before a screenshot, world x and z.
    walked: Vec<glam::DVec2>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            overrides: Overrides::default(),
            pos: DVec3::new(0.0, 90.0, 0.0),
            over: None,
            fly: false,
            third: None,
            figure_speed: 4.16,
            figure_glide: None,
            figure_furl: 0.0,
            look: (35.0, -18.0),
            time: None,
            date: None,
            weather: None,
            fresh: false,
            screenshot: None,
            map: None,
            audio: true,
            size: (1280, 720),
            frames: None,
            drift: None,
            circle: None,
            ambient: 8.0,
            shot_map: None,
            walked: Vec::new(),
        }
    }
}

const USAGE: &str = "\
usage: game [options]
Options hold for one run. Settings and your place in each world are saved between runs.
  --seed N              play world N, from where you left off (default: a new random
                        world each run, starting on foot at its spawn point)
  --fresh               start from the spawn point instead of where you left off
  --pos X,Y,Z           camera start position in voxels
  --over X,Z,H          start H voxels above the ground (or water) at X,Z
  --fly                 start in free-fly mode (F toggles in game)
  --third               on a screenshot, show the player's figure from behind (V in game)
  --figure DEG          as --third, with the figure turned DEG degrees from the view
  --figure-speed V      the figure runs at V metres a second (default 4.16)
  --figure-glide T      the figure glides, its wing T of the way open (0-1)
  --figure-furl T       the figure has its furled glider T of the way up from its back (0-1)
  --look YAW,PITCH      camera direction in degrees
  --time H              time of day, 0-24
  --season S            spring, summer, autumn or winter
  --date F              time of year, 0-1 from the spring equinox
  --weather W           hold the weather: clear, overcast, rain, snow, storm or fog
  --radius N            chunk load radius
  --no-audio            run without sound
  --screenshot FILE     render one frame to a PNG without a window, then exit
  --map FILE            write a map of the continent around the start to a PNG, then exit
  --minimap             draw the corner map on a screenshot
  --bigmap KM           draw the large map, KM across, on a screenshot
  --walked X,Z;X,Z..    on a screenshot's map, as if walked from X,Z through the rest
                        to the start
  --ambient S           seconds of ambient life before a screenshot (default 8)
  --size WxH            screenshot size, or window size with --frames (default 1280x720)
  --day-minutes M       real minutes per day, 0 to freeze time
  --frames N            exit after N frames, logging where the time went
  --drift V             fly straight ahead at V metres a second
  --circle R            with --drift, fly a circle of radius R metres instead
  --no-vsync            uncapped frame rate";

fn list<T: std::str::FromStr>(s: &str, sep: char, n: usize) -> Result<Vec<T>> {
    let v: Vec<T> = s
        .split(sep)
        .map(|p| p.trim().parse::<T>().ok())
        .collect::<Option<_>>()
        .with_context(|| format!("could not parse '{s}'"))?;
    if v.len() != n {
        bail!("expected {n} values separated by '{sep}' in '{s}'");
    }
    Ok(v)
}

fn parse_args() -> Result<Options> {
    let mut o = Options::default();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = || args.next().with_context(|| format!("{a} needs a value"));
        match a.as_str() {
            "--seed" => o.overrides.seed = Some(value()?.parse()?),
            "--fresh" => o.fresh = true,
            "--pos" => {
                let v = list::<f64>(&value()?, ',', 3)?;
                o.pos = DVec3::new(v[0], v[1], v[2]);
                o.over = None;
                o.fly = true;
                o.overrides.placed = true;
            }
            "--fly" => o.fly = true,
            "--third" => o.third = Some(0.0),
            "--figure" => o.third = Some(value()?.parse()?),
            "--figure-speed" => o.figure_speed = value()?.parse()?,
            "--figure-glide" => o.figure_glide = Some(value()?.parse()?),
            "--figure-furl" => o.figure_furl = value()?.parse()?,
            "--over" => {
                let v = list::<f64>(&value()?, ',', 3)?;
                o.over = Some((v[0] as i32, v[1] as i32, v[2]));
                o.fly = true;
                o.overrides.placed = true;
            }
            "--look" => {
                let v = list::<f32>(&value()?, ',', 2)?;
                o.look = (v[0], v[1]);
            }
            "--time" => o.time = Some(value()?.parse()?),
            "--ambient" => o.ambient = value()?.parse::<f32>()?.clamp(0.0, 600.0),
            "--date" => o.date = Some(value()?.parse::<f32>()?.rem_euclid(1.0)),
            "--weather" => {
                let name = value()?;
                o.weather = Some(weather::Forced::named(&name).ok_or_else(|| anyhow::anyhow!("unknown weather {name}"))?);
            }
            "--season" => {
                let name = value()?;
                o.date = Some(season::named(&name).ok_or_else(|| anyhow::anyhow!("unknown season {name}"))?);
            }
            "--radius" => o.overrides.radius = Some(value()?.parse::<i32>()?.clamp(1, 64)),
            "--no-audio" => o.audio = false,
            "--screenshot" => o.screenshot = Some(value()?.into()),
            "--map" => o.map = Some(value()?.into()),
            "--minimap" => o.shot_map = Some(None),
            "--bigmap" => o.shot_map = Some(Some(m64(value()?.parse::<f64>()? * 1000.0))),
            "--walked" => {
                for p in value()?.split(';') {
                    let v = list::<f64>(p, ',', 2)?;
                    o.walked.push(glam::DVec2::new(v[0], v[1]));
                }
            }
            "--size" => {
                let v = list::<u32>(&value()?, 'x', 2)?;
                o.size = (v[0].max(1), v[1].max(1));
            }
            "--day-minutes" => o.overrides.day_minutes = Some(value()?.parse()?),
            "--frames" => o.frames = Some(value()?.parse()?),
            "--drift" => o.drift = Some(value()?.parse()?),
            "--circle" => o.circle = Some(value()?.parse::<f32>()?.max(1.0)),
            "--no-vsync" => o.overrides.vsync = Some(false),
            "--help" | "-h" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => bail!("unknown argument '{other}'\n{USAGE}"),
        }
    }
    // A plain run is a new game: a random world, on foot at its spawn point. Runs that
    // only look at a place (screenshots, maps, smoke tests, --pos/--over) keep the
    // saved seed so they stay repeatable.
    if o.overrides.seed.is_none() && !o.overrides.placed && o.screenshot.is_none() && o.map.is_none() && o.frames.is_none() {
        use std::hash::{BuildHasher, Hasher};
        let seed = std::collections::hash_map::RandomState::new().build_hasher().finish() as u32;
        o.overrides.seed = Some(seed);
        o.overrides.random = true;
        o.fresh = true;
    }
    Ok(o)
}

/// Height of the walkable or swimmable surface at a column.
fn surface_height(world: &GameWorld, x: i32, z: i32) -> f64 {
    let col = world.column(x, z);
    col.water_surface().unwrap_or(col.top() + 1).max(col.top() + 1) as f64
}

/// The starting view: an explicit position if one was given, else where the player
/// left off, else the spawn point.
fn make_camera(opts: &Options, world: &GameWorld, saved: Option<&Progress>) -> Camera {
    if let Some(p) = saved {
        let mut pos = p.pos;
        if p.travel != Travel::Flying {
            if p.travel == Travel::Walking {
                // In case the terrain here has changed since the save was made.
                pos.y = pos.y.max(surface_height(world, pos.x.floor() as i32, pos.z.floor() as i32));
            }
            pos.y += player::EYE_HEIGHT;
        }
        let mut camera = Camera::new(pos, 0.0, 0.0);
        (camera.yaw, camera.pitch) = (p.yaw, p.pitch);
        return camera;
    }
    if let Some((x, z, h)) = opts.over {
        let pos = DVec3::new(x as f64 + 0.5, surface_height(world, x, z) + h, z as f64 + 0.5);
        return Camera::new(pos, opts.look.0, opts.look.1);
    }
    if opts.fly {
        return Camera::new(opts.pos, opts.look.0, opts.look.1);
    }
    let spawn = world.spawn_point();
    let feet = surface_height(world, spawn.x, spawn.z);
    let pos = DVec3::new(spawn.x as f64 + 0.5, feet + player::EYE_HEIGHT, spawn.z as f64 + 0.5);
    Camera::new(pos, spawn.yaw, -4.0)
}

fn fullscreen(on: bool) -> Option<winit::window::Fullscreen> {
    on.then_some(winit::window::Fullscreen::Borderless(None))
}

/// How close a boat must be to climb aboard.
const BOARD_REACH: f64 = m64(4.8);

/// Voxel positions with their linear colours.
type Spots = Vec<(DVec3, [f32; 3])>;
type Leaves = Vec<particles::LeafSite>;

/// Which shape of leaf a tree's foliage sheds; plants and vines shed none, and nor do
/// holly, yew and the other winter evergreens, which are left out here.
fn leaf_shape(v: voxel::Voxel) -> Option<particles::LeafShape> {
    use particles::LeafShape::*;
    use worldgen::materials as mat;
    Some(match v {
        mat::NEEDLES | mat::NEEDLES_REDWOOD => Needle,
        mat::LEAVES_WILLOW | mat::LEAVES_ACACIA | mat::LEAVES_PALM => Slip,
        mat::LEAVES_MAPLE | mat::LEAVES_MAPLE_RED => Autumn,
        mat::LEAVES_CHERRY => Petal,
        mat::LEAVES_OAK | mat::LEAVES_BIRCH | mat::LEAVES_JUNGLE | mat::LEAVES_BAOBAB => Broad,
        _ => return None,
    })
}

/// Samples voxels around the camera for leaves (to shed) and flowers (for butterflies),
/// and reads the biome. Cheap enough to do every couple of seconds.
fn scan_surroundings(
    world: &GameWorld,
    voxels: &mut WorldVoxels,
    camera: DVec3,
    salt: u32,
) -> Scan {
    use worldgen::materials as mat;
    let (cx, cz) = (camera.x.floor() as i32, camera.z.floor() as i32);
    let biome = world.column(cx, cz).biome;
    let (mut leaves, mut flowers) = (Vec::new(), Vec::new());
    let mats = world.materials();
    let to_linear = |c: [u8; 3]| c.map(|v| (v as f32 / 255.0).powf(2.2));
    let mut rng = salt.wrapping_mul(0x9e37_79b9) ^ (cx as u32).wrapping_mul(31) ^ (cz as u32).wrapping_mul(17) | 1;
    let half = (30.0 * OLD_VOXEL) as i32;
    let span = 2 * half as u32 + 1;
    let (under, above) = ((40.0 * OLD_VOXEL) as i32, (40.0 * OLD_VOXEL) as i32);
    // Enough looks to find most of the trees in reach, not just a few of them.
    for _ in 0..1100 {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        let x = cx + (rng % span) as i32 - half;
        let z = cz + ((rng >> 8) % span) as i32 - half;
        let top = camera.y as i32 + above;
        let mut below = voxel::AIR;
        for y in ((camera.y as i32 - under)..=top).rev() {
            let v = voxels.voxel(x, y, z);
            let kind = mats.get(v).kind;
            if kind == voxel::Kind::Foliage && mats.get(below).kind == voxel::Kind::Air {
                if let Some(shape) = leaf_shape(v) {
                    // Leaves fall from under the canopy as well as off its top: find its
                    // underside, where a falling leaf comes into view.
                    let mut under = y;
                    while under > y - 40 && mats.get(voxels.voxel(x, under - 1, z)).kind == voxel::Kind::Foliage {
                        under -= 1;
                    }
                    let m = mats.get(v);
                    let open = mats.get(voxels.voxel(x, under - 1, z)).kind == voxel::Kind::Air;
                    leaves.push(particles::LeafSite {
                        pos: DVec3::new(x as f64, y as f64, z as f64),
                        under: open.then_some(under as f64),
                        color: to_linear(m.color),
                        shape,
                        plant: m.plant,
                        tag: voxels.leaf_tag(x, y, z),
                    });
                }
                break;
            }
            if kind == voxel::Kind::Solid {
                if matches!(
                    v,
                    mat::FLOWER_RED | mat::FLOWER_YELLOW | mat::FLOWER_WHITE | mat::FLOWER_PURPLE
                        | mat::FLOWER_BLUE | mat::FLOWER_PINK | mat::HEATHER
                ) {
                    flowers.push((DVec3::new(x as f64, y as f64, z as f64), to_linear(mats.get(v).color)));
                }
                break;
            }
            below = v;
        }
    }
    (biome, leaves, flowers, low_ground(world, cx, cz))
}

/// The lowest ground or water within a short walk: where mist gathers. Standing in a
/// valley that is underfoot; on a hill it is the valley below.
fn low_ground(world: &GameWorld, cx: i32, cz: i32) -> f32 {
    let mut low = surface_height(world, cx, cz);
    for reach in [m(50.0) as i32, m(140.0) as i32] {
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            low = low.min(surface_height(world, cx + dx * reach, cz + dz * reach));
        }
    }
    low as f32
}

/// The biome, leaves and flowers round the camera, and the low ground near it.
type Scan = (worldgen::biome::Biome, Leaves, Spots, f32);

/// Runs [`scan_surroundings`] on its own thread. The scan can touch dozens of chunk
/// columns that aren't cached, which would stall a frame.
struct Scanner {
    ask: Sender<(DVec3, u32)>,
    answer: Receiver<Scan>,
    busy: bool,
}

impl Scanner {
    fn new(world: Arc<GameWorld>) -> Self {
        let (ask, questions) = channel::<(DVec3, u32)>();
        let (reply, answer) = channel();
        std::thread::Builder::new()
            .name("scanner".into())
            .spawn(move || {
                let mut voxels = WorldVoxels::new(world.clone());
                // Ends when the game drops `ask`.
                while let Ok((camera, salt)) = questions.recv() {
                    voxels.trim(camera);
                    let found = scan_surroundings(&world, &mut voxels, camera, salt);
                    if reply.send(found).is_err() {
                        break;
                    }
                }
            })
            .expect("scanner thread");
        Self { ask, answer, busy: false }
    }

    fn request(&mut self, camera: DVec3, salt: u32) {
        if !self.busy {
            self.busy = self.ask.send((camera, salt)).is_ok();
        }
    }

    fn poll(&mut self) -> Option<Scan> {
        let found = self.answer.try_recv().ok()?;
        self.busy = false;
        Some(found)
    }
}

struct Running {
    window: Arc<Window>,
    gpu: Gpu,
    surface: WindowSurface,
    overlay: Overlay,
    renderer: Renderer,
    streamer: Streamer,
    far: FarView,
    impostors: Impostors,
    env: Environment,
    particles: Particles,
    /// Leaf and flower voxels near the player, refreshed now and then.
    leaves: Leaves,
    flowers: Vec<(DVec3, [f32; 3])>,
    scan_timer: f32,
    scanner: Scanner,
    biome: worldgen::biome::Biome,
    /// How warm the biome underfoot is, eased as the player crosses into another.
    warmth: f32,
    /// The haze of the air here (season::humidity), eased as the biome changes.
    humidity: f32,
    /// The low ground mist lies on near the camera, eased toward the last scan's answer.
    /// None until the first scan is in.
    mist_floor: Option<f32>,
    mist_target: f32,
    weather: weather::Weather,
    audio: Option<Audio>,
    /// The settings in force.
    settings: Settings,
    /// The settings as they stand on disk.
    saved: Settings,
    /// Where to save, if anywhere: smoke tests leave no trace.
    store: Option<Store>,
    save_timer: f32,
    show_settings: bool,
    quit: bool,
    /// Distance walked since the last footstep, and how many there have been.
    stride: f32,
    steps: u32,
    /// How far out the glider is: 0 a rod on the back, to [`GLIDER_UP`] swung overhead,
    /// to 1 spread and flying.
    glider: f32,
    last_fall_speed: f32,
    was_grounded: bool,
    was_swimming: bool,
    fleet: Fleet,
    /// Solid models to draw this frame.
    props: Vec<particles::ParticleInstance>,
    /// As `props`, but casting shadows only: the player's figure, seen through its eyes.
    shades: Vec<particles::ParticleInstance>,
    /// Glider key pressed since the last frame.
    deploy: bool,
    /// Space was pressed: opens the glider too, but only well clear of the ground.
    deploy_clear: bool,
    last_yaw: f32,
    /// The view is from behind the player's figure, not through its eyes.
    third: bool,
    gait: character::Gait,
    world: Arc<GameWorld>,
    camera: Camera,
    /// On foot, or flying free of collision.
    walking: bool,
    player: Player,
    voxels: WorldVoxels,
    /// How shut in by rock the eye has grown used to being (`player::enclosure`).
    shut_in: f32,
    fly: FlyController,
    keys: HashSet<KeyCode>,
    /// Whether the mouse is currently captured for looking around.
    grabbed: bool,
    /// Whether it should be: grabbing can fail until the window has focus, so it is
    /// retried every frame while this is set.
    want_grab: bool,
    /// Raw mouse motion has arrived at least once; until then, cursor position deltas
    /// (with the cursor re-centred each frame) drive the look instead.
    raw_motion_seen: bool,
    last_cursor: Option<(f64, f64)>,
    /// Where the cursor was last seen, for dragging the map.
    last_cursor_seen: Option<(f64, f64)>,
    last_frame: Instant,
    start: Instant,
    frame: u64,
    frame_ms: f32,
    loaded: bool,
    cpu: CpuTimer,
    /// Every frame's length in milliseconds, kept only when measuring.
    frame_log: Option<Vec<f32>>,
    drift: Option<f32>,
    circle: Option<f32>,
    map: Minimap,
    /// The large map is being dragged with the mouse, from this cursor position.
    drag: Option<(f64, f64)>,
    /// Where the cursor was when the map was last turned by it, while it is.
    turning: Option<(f64, f64)>,
}

/// How much of bringing the glider out is swinging it up from the back.
const GLIDER_UP: f32 = 0.4;
/// Distance between footsteps.
const STRIDE: f32 = m(1.7);
/// How long each frame may spend handing finished terrain to the GPU.
const UPLOAD_MS: f32 = 3.0;
/// Frames left out of the measurements, while the first terrain loads in.
const SETTLE_FRAMES: u64 = 120;
/// Seconds between saves, so a crash loses little.
const AUTOSAVE: f32 = 30.0;

struct App {
    opts: Options,
    run: Option<Running>,
}

impl Running {
    fn new(el: &ActiveEventLoop, opts: &Options) -> Result<Self> {
        let began = Instant::now();
        let store = if opts.frames.is_some() { None } else { Store::find() };
        let saved = store.as_ref().map_or_else(Settings::default, Store::settings);
        let settings = opts.overrides.applied_to(saved);
        let progress = store
            .as_ref()
            .filter(|_| opts.overrides.keeps_position() && !opts.fresh)
            .and_then(|s| s.progress(settings.seed));

        // Simulating the continent takes seconds, so it starts now, alongside the window
        // and the GPU, instead of waiting its turn after them.
        log::info!("world seed {0} (run with --seed {0} to come back to it)", settings.seed);
        let world = Arc::new(GameWorld::new(settings.seed));
        let first_column = opts.over.map(|(x, z, _)| (x, z)).or(progress.map(|p| (p.pos.x.floor() as i32, p.pos.z.floor() as i32)));
        let warming = world.clone();
        std::thread::spawn(move || {
            match first_column {
                Some((x, z)) => drop(warming.column(x, z)),
                None => drop(warming.spawn_point()),
            }
            warming.library();
        });

        let attrs = Window::default_attributes().with_title("Willow Hollow").with_fullscreen(fullscreen(settings.fullscreen));
        // Measurements always use the size asked for, not the player's window.
        let size = if opts.frames.is_some() { Some(opts.size) } else { settings.window };
        let attrs = match size {
            Some((w, h)) => attrs.with_inner_size(winit::dpi::PhysicalSize::new(w, h)),
            None => attrs.with_inner_size(winit::dpi::LogicalSize::new(1280, 720)),
        };
        let window = Arc::new(el.create_window(attrs)?);
        let (gpu, surface) = Gpu::for_window(window.clone(), settings.vsync)?;
        log::info!("startup: window and GPU after {:.2} s", began.elapsed().as_secs_f32());
        let overlay = Overlay::new(&window, &gpu, surface.config.format);
        let renderer = Renderer::new(
            &gpu,
            surface.config.format,
            surface.config.width,
            surface.config.height,
        );
        log::info!("startup: pipelines after {:.2} s", began.elapsed().as_secs_f32());
        let mut camera = make_camera(opts, &world, progress.as_ref());
        log::info!("startup: world after {:.2} s", began.elapsed().as_secs_f32());
        camera.fov_y = settings.fov.to_radians();
        camera.sensitivity = settings.sensitivity;
        let feet = camera.pos - DVec3::new(0.0, player::EYE_HEIGHT, 0.0);
        let mut player = Player::new(feet);
        if let Some(p) = progress.filter(|p| p.travel == Travel::Sailing) {
            player.board(p.pos, p.heading);
        }
        let flying = opts.fly || progress.is_some_and(|p| p.travel == Travel::Flying);
        let time = opts.time.or(progress.map(|p| p.time)).unwrap_or(10.0);
        let date = opts.date.or(progress.map(|p| p.date)).unwrap_or(season::DEFAULT_DATE);
        // What has been seen of this world is kept with the rest of its save; a visit to
        // a place given by hand keeps nothing.
        let map_dir = store.as_ref().filter(|_| opts.overrides.keeps_position()).map(|s| s.map_dir(settings.seed));
        let mut map = Minimap::new(&gpu, surface.config.format, world.clone(), map_dir);
        map.corner = settings.minimap;
        Ok(Self {
            map,
            drag: None,
            turning: None,
            window,
            overlay,
            renderer,
            last_yaw: camera.yaw,
            third: false,
            gait: character::Gait::new(camera.yaw),
            camera,
            walking: !flying,
            player,
            voxels: WorldVoxels::new(world.clone()),
            shut_in: 0.0,
            far: FarView::new(&gpu, world.clone()),
            impostors: Impostors::new(&gpu, world.clone()),
            streamer: Streamer::new(&gpu, world.clone(), settings.radius),
            env: Environment::at(time, date),
            particles: Particles::new(),
            leaves: Vec::new(),
            flowers: Vec::new(),
            scan_timer: 0.0,
            scanner: Scanner::new(world.clone()),
            biome: worldgen::biome::Biome::Grassland,
            warmth: season::warmth(worldgen::biome::Biome::Grassland),
            humidity: season::humidity(worldgen::biome::Biome::Grassland),
            mist_floor: None,
            mist_target: 0.0,
            weather: weather::Weather::new(opts.weather),
            audio: if opts.audio { Audio::start(settings.volume) } else { None },
            settings,
            saved,
            store,
            save_timer: AUTOSAVE,
            show_settings: false,
            quit: false,
            stride: 0.0,
            steps: 0,
            glider: 0.0,
            last_fall_speed: 0.0,
            was_grounded: false,
            was_swimming: false,
            fleet: Fleet::new(world.clone()),
            props: Vec::new(),
            shades: Vec::new(),
            deploy: false,
            deploy_clear: false,
            world,
            fly: FlyController { speed: m(16.0) },
            keys: HashSet::new(),
            grabbed: false,
            // Measurements must not be disturbed by whatever the mouse is doing.
            want_grab: opts.frames.is_none(),
            raw_motion_seen: false,
            last_cursor: None,
            last_cursor_seen: None,
            last_frame: Instant::now(),
            start: Instant::now(),
            frame: 0,
            frame_ms: 16.0,
            loaded: false,
            cpu: CpuTimer::new(),
            frame_log: opts.frames.map(|n| Vec::with_capacity(n as usize)),
            drift: opts.drift,
            circle: opts.circle,
            gpu,
            surface,
        })
    }

    /// Captures or releases the mouse. Capturing can fail (no focus yet); the caller
    /// keeps `want_grab` set and tries again later.
    fn set_grab(&mut self, grab: bool) {
        if grab {
            if !self.window.has_focus() {
                return;
            }
            let ok = self
                .window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| self.window.set_cursor_grab(CursorGrabMode::Confined))
                .is_ok();
            if !ok {
                return;
            }
            self.last_cursor = None;
            self.centre_cursor();
        } else {
            let _ = self.window.set_cursor_grab(CursorGrabMode::None);
            self.keys.clear();
        }
        self.window.set_cursor_visible(!grab);
        self.grabbed = grab;
    }

    /// Opens or closes the settings menu, which needs the mouse.
    fn set_menu(&mut self, open: bool) {
        self.show_settings = open;
        self.want_grab = !open;
        self.set_grab(!open);
        if !open {
            self.save_timer = 0.0;
        }
    }

    /// Opens or closes the large map, which needs the mouse to move it.
    fn set_map(&mut self, open: bool) {
        self.map.set_open(open);
        self.drag = None;
        self.turning = None;
        // Where the cursor was before it was let go of is not where it is now.
        self.last_cursor_seen = None;
        self.want_grab = !open;
        self.set_grab(!open);
    }

    /// Puts into effect whatever changed in the settings since `was`.
    fn apply_settings(&mut self, was: Settings) {
        let now = self.settings;
        if now.radius != was.radius {
            self.streamer.set_radius(now.radius);
        }
        self.camera.fov_y = now.fov.to_radians();
        self.camera.sensitivity = now.sensitivity;
        if now.volume != was.volume
            && let Some(audio) = &self.audio
        {
            audio.set_volume(now.volume);
        }
        if now.vsync != was.vsync {
            self.surface.set_vsync(&self.gpu, now.vsync);
        }
        if now.fullscreen != was.fullscreen {
            self.window.set_fullscreen(fullscreen(now.fullscreen));
        }
        if now.window != was.window
            && let Some((w, h)) = now.window
        {
            let _ = self.window.request_inner_size(winit::dpi::PhysicalSize::new(w, h));
        }
    }

    fn progress(&self) -> Progress {
        let flying = !self.walking;
        Progress {
            pos: if flying { self.camera.pos } else { self.player.pos },
            yaw: self.camera.yaw,
            pitch: self.camera.pitch,
            time: self.env.time,
            date: self.env.date,
            travel: match (flying, self.player.mode) {
                (true, _) => Travel::Flying,
                (false, Mode::Boat) => Travel::Sailing,
                (false, _) => Travel::Walking,
            },
            heading: self.player.heading,
        }
    }

    /// Writes the settings, if changed, and where the player is.
    fn persist(&mut self, overrides: &Overrides) {
        let Some(store) = &self.store else { return };
        let keep = overrides.worth_keeping(self.settings, self.saved);
        if keep != self.saved && store.write_settings(keep) {
            self.saved = keep;
        }
        if overrides.keeps_position() {
            store.write_progress(self.settings.seed, self.progress());
        }
        self.map.save(false);
    }

    fn centre_cursor(&mut self) {
        let size = self.window.inner_size();
        let centre = winit::dpi::PhysicalPosition::new(size.width as f64 / 2.0, size.height as f64 / 2.0);
        if self.window.set_cursor_position(centre).is_ok() {
            self.last_cursor = Some((centre.x, centre.y));
        }
    }

    /// Mouse look from cursor positions, used only if raw motion events never arrive.
    fn cursor_moved(&mut self, x: f64, y: f64) {
        if !self.grabbed || self.raw_motion_seen {
            return;
        }
        if let Some((lx, ly)) = self.last_cursor {
            let (dx, dy) = (x - lx, y - ly);
            if dx != 0.0 || dy != 0.0 {
                self.camera.add_look(dx as f32, dy as f32);
            }
        }
        self.last_cursor = Some((x, y));
        // Keep the cursor away from the window edges.
        let size = self.window.inner_size();
        let (cx, cy) = (size.width as f64 / 2.0, size.height as f64 / 2.0);
        if (x - cx).abs() > cx * 0.5 || (y - cy).abs() > cy * 0.5 {
            self.centre_cursor();
        }
    }

    fn toggle_flying(&mut self) {
        self.walking = !self.walking;
        if self.walking {
            // Drop in wherever the camera is.
            let feet = self.camera.pos - DVec3::new(0.0, player::EYE_HEIGHT, 0.0);
            self.player = Player::new(feet);
        }
    }

    /// Boards the nearest boat, or goes ashore from the one being sailed.
    fn interact(&mut self) {
        if !self.walking {
            return;
        }
        if self.player.mode == Mode::Boat {
            let (pos, heading) = self.player.leave_boat(&mut self.voxels);
            self.fleet.leave(pos, heading);
            self.camera.pos = self.player.eye();
        } else if let Some(i) = self.fleet.nearest(self.player.pos, BOARD_REACH) {
            let boat = self.fleet.take(i);
            self.player.board(boat.pos, boat.heading);
        }
    }

    /// Counts the steps taken, one each stride, playing a footstep for each and a
    /// heavier one on landing. The figure's legs keep time by the same count.
    fn update_footsteps(&mut self, dt: f32) {
        let on_foot = self.walking && self.player.mode == Mode::Walk && !self.player.swimming;
        let grounded = on_foot && self.player.on_ground;
        if grounded {
            let landed = !self.was_grounded && self.last_fall_speed < -m(6.0);
            if landed {
                self.stride = 0.0;
            }
            self.stride += glam::Vec2::new(self.player.vel.x, self.player.vel.z).length() * dt;
            let stepped = self.stride >= STRIDE;
            if stepped {
                self.stride -= STRIDE;
                self.steps = self.steps.wrapping_add(1);
            }
            if let Some(audio) = &self.audio
                && (landed || stepped)
            {
                let (x, y, z) = (self.player.pos.x.floor() as i32, self.player.pos.y.floor() as i32, self.player.pos.z.floor() as i32);
                let feet_in_water = self.world.materials().get(self.voxels.voxel(x, y, z)).kind == voxel::Kind::Water;
                let ground = if feet_in_water { Ground::Shallows } else { audio::ground_of(self.voxels.voxel(x, y - 1, z)) };
                if landed {
                    audio.step(ground, (-self.last_fall_speed / m(16.0)).min(1.0));
                }
                if stepped {
                    audio.step(ground, 0.7);
                }
            }
        }
        self.was_grounded = grounded;
        self.last_fall_speed = self.player.vel.y;
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.surface.resize(&self.gpu, width, height);
        self.renderer.resize(&self.gpu, width, height);
    }

    fn redraw(&mut self) {
        if self.gpu.has_failed() {
            // There is nothing left to draw with. Saving still works, so leave cleanly.
            log::error!("the graphics device failed, closing the game");
            self.quit = true;
            return;
        }
        if self.want_grab && !self.grabbed {
            self.set_grab(true);
        }
        let now = Instant::now();
        let raw_dt = (now - self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.frame_ms += (raw_dt * 1000.0 - self.frame_ms) * 0.05;
        let dt = raw_dt.min(0.1);
        if let Some(log) = &mut self.frame_log
            && self.frame > SETTLE_FRAMES
        {
            log.push(raw_dt * 1000.0);
        }
        if self.frame == SETTLE_FRAMES {
            self.cpu.reset_worst();
        }
        self.cpu.start();
        if let Some(speed) = self.drift {
            if let Some(radius) = self.circle {
                self.camera.yaw = (self.camera.yaw + speed / radius * dt).rem_euclid(std::f32::consts::TAU);
            }
            let ahead = self.camera.forward() * glam::Vec3::new(1.0, 0.0, 1.0);
            self.camera.pos += (ahead.normalize_or_zero() * m(speed) * dt).as_dvec3();
        }

        if self.walking {
            let fwd = self.camera.forward();
            let fwd = glam::Vec3::new(fwd.x, 0.0, fwd.z).normalize_or_zero();
            let right = self.camera.right();
            let axis = |pos: KeyCode, neg: KeyCode| {
                self.keys.contains(&pos) as i32 as f32 - self.keys.contains(&neg) as i32 as f32
            };
            let pi = std::f32::consts::PI;
            let turned = (self.camera.yaw - self.last_yaw + pi).rem_euclid(pi * 2.0) - pi;
            self.last_yaw = self.camera.yaw;
            let controls = Controls {
                wish: fwd * axis(KeyCode::KeyW, KeyCode::KeyS)
                    + right * axis(KeyCode::KeyD, KeyCode::KeyA),
                jump: self.keys.contains(&KeyCode::Space),
                sprint: self.keys.contains(&KeyCode::ShiftLeft),
                deploy: std::mem::take(&mut self.deploy),
                deploy_clear: std::mem::take(&mut self.deploy_clear),
                look: self.camera.forward(),
                turn: if dt > 0.0 { turned / dt } else { 0.0 },
            };
            self.player.update(&mut self.voxels, controls, dt);
            self.camera.pos = self.player.eye();
            // The figure: its stride from how fast it goes over the ground, and facing the
            // way it goes, or the way its glider or boat does.
            let over_ground = glam::Vec2::new(self.player.vel.x, self.player.vel.z);
            let speed = over_ground.length() / worldgen::scale::VOXELS_PER_METRE;
            let heading = match self.player.mode {
                Mode::Glide => Some(self.camera.yaw),
                Mode::Boat => Some(self.player.heading),
                Mode::Walk if speed > 0.5 => Some(over_ground.y.atan2(over_ground.x)),
                Mode::Walk if !self.third => Some(self.camera.yaw),
                Mode::Walk => None,
            };
            let doing = character::Doing::of(self.player.mode, self.player.on_ground, self.player.swimming, speed);
            // Two steps to a turn of the legs, so only whether the count is odd matters.
            let stride = (self.steps % 2) as f32 + self.stride / STRIDE;
            self.gait.update(doing, stride, heading, self.player.mode == Mode::Glide, dt);
            if self.third {
                // Behind and looking past the figure, as far back as the ground and walls allow.
                use player::Voxels;
                let back = -self.camera.forward().as_dvec3();
                let eye = self.camera.pos + DVec3::new(0.0, m64(0.25), 0.0);
                let (reach, step) = (m64(3.6), 0.25);
                let mut d = 0.0;
                while d < reach {
                    let ahead = (eye + back * (d + 0.75)).floor().as_ivec3();
                    if self.voxels.kind(ahead.x, ahead.y, ahead.z) == voxel::Kind::Solid {
                        break;
                    }
                    d += step;
                }
                self.camera.pos = eye + back * d;
            }
            self.camera.roll = self.player.bank;
        } else {
            self.camera.roll = 0.0;
            self.fly.update(&mut self.camera, &self.keys, dt);
        }
        self.fleet.update(self.player.pos, dt);
        self.cpu.lap(0);
        self.streamer.update(&self.gpu, self.camera.pos, UPLOAD_MS);
        self.far.update(&self.gpu, self.camera.pos);
        self.impostors.update(&self.gpu, self.camera.pos);
        self.cpu.lap(1);

        // The day turns.
        let days = if self.settings.day_minutes > 0.0 { dt / (self.settings.day_minutes * 60.0) } else { 0.0 };
        if days > 0.0 {
            let date = season::advance(self.env.date, days, self.settings.year_days);
            self.env = Environment::at(self.env.time + days * 24.0, date);
        }
        self.save_timer -= dt;
        self.weather.update(dt, days, self.env.date, self.settings.year_days, self.warmth, self.settings.seed);
        self.warmth += (season::warmth(self.biome) - self.warmth) * (1.0 - (-0.4 * dt).exp());
        // The air changes more slowly than the ground: haze does not end at a biome's edge.
        self.humidity += (season::humidity(self.biome) - self.humidity) * (1.0 - (-0.08 * dt).exp());
        if let Some(floor) = &mut self.mist_floor {
            *floor += (self.mist_target - *floor) * (1.0 - (-0.25 * dt).exp());
        }

        // Ambient life. Every so often, look around for leaves and flowers to spawn from.
        self.scan_timer -= dt;
        if self.scan_timer <= 0.0 {
            self.scan_timer = 2.0;
            self.scanner.request(self.camera.pos, self.frame as u32);
        }
        if let Some(found) = self.scanner.poll() {
            let low;
            (self.biome, self.leaves, self.flowers, low) = found;
            self.mist_floor.get_or_insert(low);
            self.mist_target = low;
        }
        let entered_water = (self.player.swimming && !self.was_swimming && self.walking)
            .then(|| self.player.pos + DVec3::new(0.0, m64(0.8), 0.0));
        self.was_swimming = self.player.swimming;
        let cascades: Vec<_> = self.streamer.cascades_near(self.camera.pos, m(120.0)).copied().collect();
        let springs: Vec<_> = self.streamer.springs_near(self.camera.pos, m(40.0)).copied().collect();
        let surf: Vec<_> = self.streamer.surf_near(self.camera.pos, m(60.0)).copied().collect();
        let surroundings = Surroundings {
            camera: self.camera.pos,
            biome: self.biome,
            sun_elevation: self.env.sun_elevation,
            cascades: &cascades,
            springs: &springs,
            surf: &surf,
            leaves: &self.leaves,
            flowers: &self.flowers,
            player_entered_water: entered_water,
            leaf_fall: season::leaf_fall(self.env.date),
            date: self.env.date,
            rain: self.weather.now.rain,
            snow: self.weather.now.snow,
            storm: self.weather.now.storm,
            time: self.last_frame.duration_since(self.start).as_secs_f64(),
        };
        // Leaves and rain can wait a frame for ground that is not generated yet.
        self.voxels.trim(self.camera.pos);
        self.particles.update(dt, &surroundings, &mut self.voxels.lazily());
        self.update_footsteps(dt);

        let clock = self.last_frame.duration_since(self.start).as_secs_f32();
        self.props.clear();
        self.shades.clear();
        for boat in self.fleet.boats() {
            vehicles::draw_boat(boat.pos, boat.heading, clock, self.camera.pos, &mut self.props);
        }
        if self.walking {
            match self.player.mode {
                Mode::Boat => vehicles::draw_boat(self.player.pos, self.player.heading, clock, self.camera.pos, &mut self.props),
                Mode::Glide | Mode::Walk => {}
            }
            // The glider swings open as it is taken to the air and folds away on landing.
            let flying = self.player.mode == Mode::Glide;
            self.glider = (self.glider + if flying { dt / 0.5 } else { -dt / 0.45 }).clamp(0.0, 1.0);
            if self.glider > GLIDER_UP {
                let open = (self.glider - GLIDER_UP) / (1.0 - GLIDER_UP);
                vehicles::draw_glider(self.player.pos, self.camera.yaw, open, self.camera.pos, &mut self.props);
            }
            {
                // Sitting, the hips are where the thwart is.
                // On foot they rise and fall over steps as smoothly as the eye does.
                let feet = match self.player.mode {
                    Mode::Boat => self.player.pos - DVec3::new(0.0, m64(0.43), 0.0),
                    _ => self.player.eye() - DVec3::new(0.0, player::EYE_HEIGHT, 0.0),
                };
                // Seen from behind, or through its own eyes only by its shadow.
                let into = if self.third { &mut self.props } else { &mut self.shades };
                // The glider is a rod on the back until the wing itself is out.
                let furled = (self.glider <= GLIDER_UP).then_some(self.glider / GLIDER_UP);
                character::draw(&character::WANDERER, feet, self.gait.facing, &self.gait.pose(), furled, self.camera.pos, into);
            }
        }

        self.map.corner = self.settings.minimap;
        self.map.hour = self.env.time;
        let here = if self.walking { self.player.eye() } else { self.camera.pos };
        self.map.update(&self.gpu, here, self.camera.forward(), self.walking, dt);

        self.cpu.lap(2);
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        let lamps: Vec<DVec3> = self.streamer.lamps_near(self.camera.pos, m(24.0)).map(|p| p.as_dvec3()).collect();
        let mut lit = self.env.clone();
        self.weather.apply(&mut lit);
        lit.air(self.humidity, self.weather.wet * (1.0 - (self.weather.now.rain + self.weather.now.snow).min(1.0)));
        // The eye takes a couple of seconds to open up in the dark, and less to close
        // again in the light.
        let shut_in = player::enclosure(self.camera.pos, &mut self.voxels.lazily());
        let rate = if shut_in > self.shut_in { 0.6 } else { 2.0 };
        self.shut_in += (shut_in - self.shut_in) * (1.0 - (-rate * dt).exp());
        lit.adapt(self.shut_in);
        lit.mist_floor = self.mist_floor.unwrap_or(self.camera.pos.y as f32 - m(12.0));
        let ground = self.weather.places(self.env.date, self.settings.year_days, self.warmth, self.settings.seed);
        let scene = render::Scene {
            camera: &self.camera,
            env: &lit,
            terrain: &self.streamer,
            far: &self.far,
            impostors: &self.impostors,
            particles: self.particles.instances(),
            props: &self.props,
            shades: &self.shades,
            wind: self.particles.wind.lagged * OLD_VOXEL,
            ground,
            lamps: &lamps,
            shafts: self.settings.shafts,
        };
        // The scene is drawn into the renderer's own textures and handed to the GPU
        // before waiting for the window's next image, which is not free until the last
        // frame is done with: the GPU then draws this frame through the wait, rather
        // than standing idle until the frame has been put together after it.
        self.renderer.draw(&self.gpu, &mut encoder, &scene);
        self.gpu.queue.submit([encoder.finish()]);
        self.cpu.lap(4);
        let Some(frame) = self.surface.acquire(&self.gpu) else {
            return;
        };
        self.cpu.lap(3);
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        self.renderer.finish(&self.gpu, &mut encoder, &view);
        self.map.draw(&self.gpu, &mut encoder, &view, (self.surface.config.width, self.surface.config.height));

        let size = [self.surface.config.width, self.surface.config.height];
        let cam = &self.camera;
        let (frame_ms, speed) = (self.frame_ms, self.fly.speed);
        let gpu_name = &self.gpu.adapter_name;
        let (s, d) = (self.streamer.stats, self.renderer.stats);
        let (particle_count, biome, seed) = (self.particles.len(), self.biome, self.settings.seed);
        let (gpu_line, cpu_line) = (self.renderer.timer.as_ref().map(profile::GpuTimer::summary), self.cpu.summary());
        let mode = match (self.walking, self.player.mode, self.player.swimming, self.player.on_ground) {
            (false, ..) => "flying",
            (true, Mode::Boat, ..) => "sailing",
            (true, Mode::Glide, ..) => "gliding",
            (true, Mode::Walk, true, _) => "swimming",
            (true, Mode::Walk, false, true) => "walking",
            (true, Mode::Walk, false, false) => "in the air",
        };
        let prompt = match self.player.mode {
            _ if !self.walking => None,
            Mode::Boat => Some("E to go ashore   W to sail   Shift for full sail"),
            Mode::Walk if self.fleet.nearest(self.player.pos, BOARD_REACH).is_some() => Some("E to board the boat"),
            _ => None,
        };
        let mut time = self.env.time;
        let mut date = self.env.date;
        let mut settings = self.settings;
        let mut choice = overlay::MenuChoice::default();
        let held = Some(self.streamer.radius()).filter(|r| *r < settings.radius);
        let (show_debug, show_menu, grabbed) = (settings.show_debug, self.show_settings, self.grabbed);
        let (labels, map_open) = (self.map.labels(), self.map.open);
        self.overlay
            .draw(&self.window, &self.gpu, &mut encoder, &view, size, |ctx| {
                overlay::map_labels(ctx, labels);
                if show_menu {
                    choice = overlay::settings_menu(ctx, &mut settings, held, (size[0], size[1]));
                } else if !grabbed && !map_open {
                    egui::Area::new(egui::Id::unique("hint"))
                        .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -40.0])
                        .show(ctx, |ui| {
                            ui.label(egui::RichText::new("Click the view (or press Tab) to look around").size(18.0));
                        });
                }
                if let Some(text) = prompt {
                    egui::Area::new(egui::Id::unique("prompt"))
                        .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -80.0])
                        .show(ctx, |ui| {
                            ui.label(egui::RichText::new(text).size(18.0).color(egui::Color32::from_gray(235)));
                        });
                }
                if !show_debug {
                    return;
                }
                egui::Window::new("Debug")
                    .default_pos([8.0, 8.0])
                    .show(ctx, |ui| {
                        ui.label(format!(
                            "{:.0} fps ({frame_ms:.1} ms)  |  {gpu_name}",
                            1000.0 / frame_ms
                        ));
                        ui.label(format!(
                            "pos {:.1} {:.1} {:.1}   yaw {:.0} pitch {:.0}   speed {speed:.0}",
                            cam.pos.x,
                            cam.pos.y,
                            cam.pos.z,
                            cam.yaw.to_degrees(),
                            cam.pitch.to_degrees()
                        ));
                        ui.label(format!(
                            "chunks {} in {} columns, drawn {} (culled {}), far tiles {}",
                            s.chunks, s.columns, d.chunks_drawn, d.chunks_culled, d.far_tiles
                        ));
                        ui.label(format!(
                            "quads {:.2} M loaded, {:.2} M drawn   mesh {:.0} MB",
                            s.quads as f64 / 1e6,
                            d.quads_drawn as f64 / 1e6,
                            s.mesh_bytes as f64 / 1048576.0
                        ));
                        ui.label(format!(
                            "per column: gen {:.2} ms, mesh {:.2} ms   queued {} (+{} running)",
                            s.gen_ms, s.mesh_ms, s.queued, s.in_flight
                        ));
                        if let Some(line) = &gpu_line {
                            ui.label(line);
                        }
                        ui.label(&cpu_line);
                        ui.add(egui::Slider::new(&mut time, 0.0..=24.0).text("time of day"));
                        ui.add(egui::Slider::new(&mut date, 0.0..=1.0).text("time of year"));
                        ui.label(format!("mode: {mode}   particles {particle_count}   biome {biome:?}   seed {seed}"));
                        ui.label("Esc = settings. Click or Tab to look. WASD, Space = jump/swim up, Shift = sprint");
                        ui.label("F = toggle flying (Space/Ctrl up/down, wheel = speed). F3 = hide this panel, F11 = fullscreen");
                        ui.label("G in the air, or Space when high up = glider. E = board or leave a boat at a landing");
                        ui.label("M = the map (wheel zooms, drag moves it). N = show or hide the corner map");
                    });
            });
        if time != self.env.time || date != self.env.date {
            self.env = Environment::at(time, date);
        }
        if choice.resume {
            self.set_menu(false);
        }
        self.quit |= choice.quit;
        self.cpu.lap(5);

        self.gpu.queue.submit([encoder.finish()]);
        self.window.pre_present_notify();
        self.gpu.queue.present(frame);
        // Only now: the surface can't be reconfigured while a frame from it is held.
        let was = std::mem::replace(&mut self.settings, settings);
        self.apply_settings(was);
        if let Some(timer) = &mut self.renderer.timer {
            timer.after_submit(&self.gpu);
        }
        self.cpu.lap(6);
        self.frame += 1;
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.run.is_some() {
            return;
        }
        match Running::new(el, &self.opts) {
            Ok(r) => self.run = Some(r),
            Err(e) => {
                log::error!("startup failed: {e:#}");
                el.exit();
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(r) = self.run.as_mut() else { return };
        // Esc always reaches the game, even while a slider has the keyboard.
        if let WindowEvent::KeyboardInput { event: key, .. } = &event
            && key.state == ElementState::Pressed
            && !key.repeat
            && key.physical_key == PhysicalKey::Code(KeyCode::Escape)
        {
            if r.map.open {
                r.set_map(false);
            } else {
                r.set_menu(!r.show_settings);
            }
            return;
        }
        // While the map is up the mouse is the map's, wherever on it the cursor is: egui
        // still hears of it, but may not keep it.
        let for_map = r.map.open
            && !r.show_settings
            && matches!(event, WindowEvent::CursorMoved { .. } | WindowEvent::MouseInput { .. } | WindowEvent::MouseWheel { .. });
        if !r.grabbed && r.overlay.on_event(&r.window, &event) && !for_map {
            return;
        }
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(s) => {
                r.resize(s.width, s.height);
                // Remember only a size the player would want the window to come back at.
                if s.width > 0 && s.height > 0 && r.window.fullscreen().is_none() && !r.window.is_maximized() {
                    r.settings.window = Some((s.width, s.height));
                }
            }
            WindowEvent::Focused(false) => {
                r.set_grab(false);
                r.drag = None;
                r.turning = None;
            }
            WindowEvent::Focused(true) => {
                if r.want_grab {
                    r.set_grab(true);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                // A drag begun before the cursor was ever seen starts from here.
                if let Some((x, y)) = r.drag {
                    if x.is_finite() {
                        r.map.drag((position.x - x) as f32, (position.y - y) as f32);
                    }
                    r.drag = Some((position.x, position.y));
                }
                if let Some((x, y)) = r.turning {
                    if x.is_finite() {
                        r.map.turn((position.x - x) as f32, (position.y - y) as f32);
                    }
                    r.turning = Some((position.x, position.y));
                }
                r.last_cursor_seen = Some((position.x, position.y));
                r.cursor_moved(position.x, position.y);
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } if r.map.open && !r.show_settings => {
                r.drag = (state == ElementState::Pressed).then(|| r.last_cursor_seen.unwrap_or((f64::NAN, f64::NAN)));
            }
            WindowEvent::MouseInput { state, button: MouseButton::Right, .. } if r.map.open && !r.show_settings => {
                r.turning = (state == ElementState::Pressed).then(|| r.last_cursor_seen.unwrap_or((f64::NAN, f64::NAN)));
            }
            WindowEvent::MouseWheel { delta, .. } if r.map.open => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                r.map.zoom(lines);
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } if !r.grabbed && !r.show_settings => {
                r.want_grab = true;
                r.set_grab(true);
            }
            WindowEvent::MouseWheel { delta, .. } if r.grabbed => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                r.fly.scroll(lines);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    if event.state == ElementState::Pressed {
                        if code == KeyCode::F3 && !event.repeat {
                            r.settings.show_debug = !r.settings.show_debug;
                        } else if code == KeyCode::F11 && !event.repeat {
                            r.settings.fullscreen = !r.settings.fullscreen;
                            r.window.set_fullscreen(fullscreen(r.settings.fullscreen));
                        } else if r.show_settings {
                            // The rest of the keyboard is for playing.
                        } else if code == KeyCode::KeyM && !event.repeat {
                            r.set_map(!r.map.open);
                        } else if code == KeyCode::KeyN && !event.repeat {
                            r.settings.minimap = !r.settings.minimap;
                        } else if r.map.open {
                            // The rest waits until the map is put away.
                        } else if code == KeyCode::Tab && !event.repeat {
                            r.want_grab = !r.grabbed;
                            r.set_grab(r.want_grab);
                        } else if code == KeyCode::KeyV && !event.repeat {
                            r.third = !r.third;
                        } else if code == KeyCode::KeyF && !event.repeat {
                            r.toggle_flying();
                        } else if code == KeyCode::KeyE && !event.repeat {
                            r.interact();
                        } else if r.grabbed {
                            // G opens the glider anywhere in the air. A fresh press of Space
                            // does too, but only high up: Space is also the jump key, and
                            // hopping up a hill must not open it.
                            if !event.repeat && code == KeyCode::KeyG {
                                r.deploy = true;
                            }
                            if !event.repeat && code == KeyCode::Space {
                                r.deploy_clear = true;
                            }
                            r.keys.insert(code);
                        }
                    } else {
                        r.keys.remove(&code);
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                r.redraw();
                if !r.loaded && r.streamer.is_idle() {
                    r.loaded = true;
                    log::info!("startup: all terrain in view {:.2} s after the first frame", r.start.elapsed().as_secs_f32());
                }
                if r.quit {
                    el.exit();
                }
                if r.save_timer <= 0.0 {
                    r.save_timer = AUTOSAVE;
                    r.persist(&self.opts.overrides);
                }
                if self.opts.frames.is_some() && r.frame % 120 == 0 {
                    log::info!("frame {}: {}; {}", r.frame, r.streamer.progress(), r.impostors.progress());
                }
                if self.opts.frames.is_some_and(|n| r.frame >= n) {
                    let (s, d) = (r.streamer.stats, r.renderer.stats);
                    let mut times = r.frame_log.take().unwrap_or_default();
                    times.sort_unstable_by(f32::total_cmp);
                    log::info!(
                        "frame ms: median {:.1}, p95 {:.1}, p99 {:.1}, worst {:.1}",
                        profile::quantile(&times, 0.5),
                        profile::quantile(&times, 0.95),
                        profile::quantile(&times, 0.99),
                        profile::quantile(&times, 1.0)
                    );
                    log::info!("{}", r.cpu.summary());
                    log::info!("{}", r.cpu.worst_summary());
                    if let Some(timer) = &r.renderer.timer {
                        log::info!("{}", timer.summary());
                    }
                    for (level, (n, generate, mesh)) in s.built.iter().enumerate().filter(|(_, b)| b.0 > 0) {
                        let n = *n as f32;
                        log::info!("level {level}: {n} columns built, {:.2} ms to generate and {:.2} ms to mesh each", generate / n, mesh / n);
                    }
                    log::info!(
                        "after {} frames: {:.1} ms/frame ({:.0} fps), {} chunks, {:.2} M quads drawn, {} far tiles, {} far tree boxes, arena {} MiB",
                        r.frame,
                        r.frame_ms,
                        1000.0 / r.frame_ms,
                        s.chunks,
                        d.quads_drawn as f64 / 1e6,
                        d.far_tiles,
                        d.impostor_boxes,
                        s.arena_bytes >> 20
                    );
                    el.exit();
                }
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _el: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        let Some(r) = self.run.as_mut() else { return };
        if let DeviceEvent::MouseMotion { delta } = event
            && r.grabbed
        {
            r.raw_motion_seen = true;
            r.camera.add_look(delta.0 as f32, delta.1 as f32);
        }
    }

    fn exiting(&mut self, _el: &ActiveEventLoop) {
        if let Some(r) = self.run.as_mut() {
            r.map.save(true);
            r.persist(&self.opts.overrides);
        }
    }

    fn about_to_wait(&mut self, _el: &ActiveEventLoop) {
        if let Some(r) = &self.run {
            r.window.request_redraw();
        }
    }
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .filter_module("wgpu_core", log::LevelFilter::Warn)
        .filter_module("wgpu_hal", log::LevelFilter::Warn)
        // Vulkan loader chatter about other programs' broken overlay layers; not ours to fix.
        .filter_module("wgpu_hal::vulkan::instance", log::LevelFilter::Off)
        .filter_module("naga", log::LevelFilter::Warn)
        .init();
    let opts = parse_args()?;

    if let Some(path) = &opts.map {
        let settings = opts.overrides.applied_to(Settings::default());
        let world = GameWorld::new(settings.seed);
        let camera = make_camera(&opts, &world, None);
        minimap::build(&world, camera.pos).save(path)?;
        log::info!("map {}", path.display());
        return Ok(());
    }

    if let Some(path) = &opts.screenshot {
        // Screenshots neither read nor write anything saved, so they are repeatable.
        let settings = opts.overrides.applied_to(Settings::default());
        let gpu = Gpu::headless()?;
        let world = Arc::new(GameWorld::new(settings.seed));
        let mut camera = make_camera(&opts, &world, None);
        // The figure stands where the camera was, mid stride, and the camera steps back.
        let figure = opts.third.map(|turned| {
            let feet = camera.pos - DVec3::new(0.0, player::EYE_HEIGHT, 0.0);
            camera.pos += -camera.forward().as_dvec3() * m64(3.6) + DVec3::new(0.0, m64(0.25), 0.0);
            let facing = camera.yaw + turned.to_radians();
            let gait = if opts.figure_glide.is_some() { character::Gait::gliding(facing) } else { character::Gait::striding(facing, opts.figure_speed) };
            (feet, gait)
        });
        let mut far = FarView::new(&gpu, world.clone());
        let mut streamer = Streamer::new(&gpu, world.clone(), settings.radius);
        let mut impostors = Impostors::new(&gpu, world.clone());
        let t = Instant::now();
        let mut env = Environment::at(opts.time.unwrap_or(10.0), opts.date.unwrap_or(season::DEFAULT_DATE));
        // Screenshots are clear unless a weather is asked for.
        let mut weather = weather::Weather::new(opts.weather);
        weather.apply(&mut env);
        // A few seconds of ambient life, so stills show spray, leaves and fireflies.
        streamer.wait_idle(&gpu, camera.pos);
        let mut voxels = WorldVoxels::new(world.clone());
        let (biome, leaves, flowers, low) = scan_surroundings(&world, &mut voxels, camera.pos, 1);
        env.mist_floor = low;
        // The snow and ice the season has left lying here, as the game would have them.
        let warmth = season::warmth(biome);
        weather.settle(env.date, settings.year_days, warmth, settings.seed);
        env.air(season::humidity(biome), weather.wet * (1.0 - (weather.now.rain + weather.now.snow).min(1.0)));
        env.adapt(player::enclosure(camera.pos, &mut voxels));
        let cascades: Vec<_> = streamer.cascades_near(camera.pos, m(120.0)).copied().collect();
        let springs: Vec<_> = streamer.springs_near(camera.pos, m(40.0)).copied().collect();
        let surf: Vec<_> = streamer.surf_near(camera.pos, m(60.0)).copied().collect();
        let mut fleet = Fleet::new(world.clone());
        fleet.refresh(camera.pos);
        let mut props = Vec::new();
        for boat in fleet.boats() {
            vehicles::draw_boat(boat.pos, boat.heading, 0.0, camera.pos, &mut props);
        }
        if let Some((feet, gait)) = &figure {
            let furled = opts.figure_glide.is_none().then_some(opts.figure_furl);
            character::draw(&character::WANDERER, *feet, gait.facing, &gait.pose(), furled, camera.pos, &mut props);
            if let Some(open) = opts.figure_glide {
                vehicles::draw_glider(*feet, gait.facing, open, camera.pos, &mut props);
            }
        }
        let mut particles = Particles::new();
        for step in 0..(opts.ambient * 60.0) as u32 {
            let surroundings = Surroundings {
                camera: camera.pos,
                biome,
                sun_elevation: env.sun_elevation,
                cascades: &cascades,
                springs: &springs,
                surf: &surf,
                leaves: &leaves,
                flowers: &flowers,
                player_entered_water: None,
                leaf_fall: season::leaf_fall(env.date),
                date: env.date,
                rain: weather.now.rain,
                snow: weather.now.snow,
                storm: weather.now.storm,
                time: step as f64 / 60.0,
            };
            particles.update(1.0 / 60.0, &surroundings, &mut voxels);
        }
        log::info!("season {:.2}, biome {biome:?}, ground {:?}", env.date, weather.ground(env.date, warmth));
        for (at, n) in particles.flocks() {
            let off = at - camera.pos;
            log::info!("flock of {n} at {:.0} {:.0} {:.0} from the camera", off.x, off.y, off.z);
        }
        let lamps: Vec<DVec3> = streamer.lamps_near(camera.pos, m(24.0)).map(|p| p.as_dvec3()).collect();
        // The map, as the player would have it after walking here.
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut map = opts.shot_map.map(|large| {
            let mut map = Minimap::new(&gpu, format, world.clone(), None);
            map.settle(&gpu, camera.pos, camera.forward());
            let mut path = opts.walked.clone();
            path.push(glam::DVec2::new(camera.pos.x, camera.pos.z));
            map.walk(&path);
            if let Some(span) = large {
                map.set_open(true);
                map.set_span(span);
            }
            map.settle(&gpu, camera.pos, camera.forward());
            map
        });
        let mut hud = |encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, text: bool| {
            let Some(map) = &mut map else { return };
            map.hour = env.time;
            map.update(&gpu, camera.pos, camera.forward(), true, 0.0);
            map.draw(&gpu, encoder, view, opts.size);
            if text {
                overlay::draw_headless(&gpu, encoder, view, format, opts.size, |ctx| overlay::map_labels(ctx, map.labels()));
            }
        };
        render::screenshot(
            &gpu,
            path,
            opts.size,
            &camera,
            &env,
            &mut streamer,
            &mut far,
            &mut impostors,
            particles.instances(),
            &props,
            particles.wind.lagged * OLD_VOXEL,
            weather.places(env.date, settings.year_days, warmth, settings.seed),
            &lamps,
            &mut hud,
        )?;
        let s = streamer.stats;
        log::info!(
            "{} chunks in {} columns, {:.2} M quads, {:.0} MB mesh, {:.1} s total; per column gen {:.2} ms, mesh {:.2} ms",
            s.chunks,
            s.columns,
            s.quads as f64 / 1e6,
            s.mesh_bytes as f64 / 1048576.0,
            t.elapsed().as_secs_f32(),
            s.gen_ms,
            s.mesh_ms
        );
        return Ok(());
    }

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App { opts, run: None };
    event_loop.run_app(&mut app)?;
    Ok(())
}
