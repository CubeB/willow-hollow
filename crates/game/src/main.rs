mod arena;
mod audio;
mod camera;
mod env;
mod farview;
mod gfx;
mod overlay;
mod particles;
mod player;
mod profile;
mod render;
mod settings;
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
    look: (f32, f32),
    /// Time of day in hours, if not the one left off at.
    time: Option<f32>,
    /// Start from the spawn point, ignoring any saved position.
    fresh: bool,
    /// Render one frame to this PNG without opening a window, then exit.
    screenshot: Option<PathBuf>,
    audio: bool,
    size: (u32, u32),
    /// Exit after this many frames (smoke tests).
    frames: Option<u64>,
    /// Fly straight ahead at this many metres a second, to measure streaming.
    drift: Option<f32>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            overrides: Overrides::default(),
            pos: DVec3::new(0.0, 90.0, 0.0),
            over: None,
            fly: false,
            look: (35.0, -18.0),
            time: None,
            fresh: false,
            screenshot: None,
            audio: true,
            size: (1280, 720),
            frames: None,
            drift: None,
        }
    }
}

const USAGE: &str = "\
usage: game [options]
Options hold for one run. Settings and your place in each world are saved between runs.
  --seed N              world seed (default: the last one played)
  --fresh               start from the spawn point instead of where you left off
  --pos X,Y,Z           camera start position in voxels
  --over X,Z,H          start H voxels above the ground (or water) at X,Z
  --fly                 start in free-fly mode (F toggles in game)
  --look YAW,PITCH      camera direction in degrees
  --time H              time of day, 0-24
  --radius N            chunk load radius
  --no-audio            run without sound
  --screenshot FILE     render one frame to a PNG without a window, then exit
  --size WxH            screenshot size, or window size with --frames (default 1280x720)
  --day-minutes M       real minutes per day, 0 to freeze time
  --frames N            exit after N frames, logging where the time went
  --drift V             fly straight ahead at V metres a second
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
            "--radius" => o.overrides.radius = Some(value()?.parse::<i32>()?.clamp(1, 64)),
            "--no-audio" => o.audio = false,
            "--screenshot" => o.screenshot = Some(value()?.into()),
            "--size" => {
                let v = list::<u32>(&value()?, 'x', 2)?;
                o.size = (v[0].max(1), v[1].max(1));
            }
            "--day-minutes" => o.overrides.day_minutes = Some(value()?.parse()?),
            "--frames" => o.frames = Some(value()?.parse()?),
            "--drift" => o.drift = Some(value()?.parse()?),
            "--no-vsync" => o.overrides.vsync = Some(false),
            "--help" | "-h" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => bail!("unknown argument '{other}'\n{USAGE}"),
        }
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
type Leaves = Vec<(DVec3, [f32; 3], particles::LeafShape)>;

/// Which shape of leaf a tree's foliage sheds; plants and vines shed none.
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
) -> (worldgen::biome::Biome, Leaves, Spots) {
    use worldgen::materials as mat;
    let (cx, cz) = (camera.x.floor() as i32, camera.z.floor() as i32);
    let biome = world.column(cx, cz).biome;
    let (mut leaves, mut flowers) = (Vec::new(), Vec::new());
    let mats = world.materials();
    let to_linear = |c: [u8; 3]| c.map(|v| (v as f32 / 255.0).powf(2.2));
    let mut rng = salt.wrapping_mul(0x9e37_79b9) ^ (cx as u32).wrapping_mul(31) ^ (cz as u32).wrapping_mul(17) | 1;
    let half = (30.0 * OLD_VOXEL) as i32;
    let span = 2 * half as u32 + 1;
    let (under, above) = ((12.0 * OLD_VOXEL) as i32, (40.0 * OLD_VOXEL) as i32);
    for _ in 0..700 {
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
                    leaves.push((DVec3::new(x as f64, y as f64, z as f64), to_linear(mats.get(v).color), shape));
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
    (biome, leaves, flowers)
}

type Scan = (worldgen::biome::Biome, Leaves, Spots);

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
                    let found = scan_surroundings(&world, &mut voxels, camera, salt);
                    voxels.trim(camera);
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
    env: Environment,
    particles: Particles,
    /// Leaf and flower voxels near the player, refreshed now and then.
    leaves: Leaves,
    flowers: Vec<(DVec3, [f32; 3])>,
    scan_timer: f32,
    scanner: Scanner,
    biome: worldgen::biome::Biome,
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
    /// Distance walked since the last footstep.
    stride: f32,
    last_fall_speed: f32,
    was_grounded: bool,
    was_swimming: bool,
    fleet: Fleet,
    /// Solid models to draw this frame.
    props: Vec<particles::ParticleInstance>,
    /// Glider key pressed since the last frame.
    deploy: bool,
    last_yaw: f32,
    world: Arc<GameWorld>,
    camera: Camera,
    /// On foot, or flying free of collision.
    walking: bool,
    player: Player,
    voxels: WorldVoxels,
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
    last_frame: Instant,
    start: Instant,
    frame: u64,
    frame_ms: f32,
    loaded: bool,
    cpu: CpuTimer,
    /// Every frame's length in milliseconds, kept only when measuring.
    frame_log: Option<Vec<f32>>,
    drift: Option<f32>,
}

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
        Ok(Self {
            window,
            overlay,
            renderer,
            last_yaw: camera.yaw,
            camera,
            walking: !flying,
            player,
            voxels: WorldVoxels::new(world.clone()),
            far: FarView::new(&gpu, world.clone()),
            streamer: Streamer::new(&gpu, world.clone(), settings.radius),
            env: Environment::at_time(time),
            particles: Particles::new(),
            leaves: Vec::new(),
            flowers: Vec::new(),
            scan_timer: 0.0,
            scanner: Scanner::new(world.clone()),
            biome: worldgen::biome::Biome::Grassland,
            audio: if opts.audio { Audio::start(settings.volume) } else { None },
            settings,
            saved,
            store,
            save_timer: AUTOSAVE,
            show_settings: false,
            quit: false,
            stride: 0.0,
            last_fall_speed: 0.0,
            was_grounded: false,
            was_swimming: false,
            fleet: Fleet::new(world.clone()),
            props: Vec::new(),
            deploy: false,
            world,
            fly: FlyController { speed: m(16.0) },
            keys: HashSet::new(),
            grabbed: false,
            // Measurements must not be disturbed by whatever the mouse is doing.
            want_grab: opts.frames.is_none(),
            raw_motion_seen: false,
            last_cursor: None,
            last_frame: Instant::now(),
            start: Instant::now(),
            frame: 0,
            frame_ms: 16.0,
            loaded: false,
            cpu: CpuTimer::new(),
            frame_log: opts.frames.map(|n| Vec::with_capacity(n as usize)),
            drift: opts.drift,
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

    /// Plays a footstep each stride, and a heavier one on landing.
    fn update_footsteps(&mut self, dt: f32) {
        let Some(audio) = &self.audio else { return };
        let on_foot = self.walking && self.player.mode == Mode::Walk && !self.player.swimming;
        let grounded = on_foot && self.player.on_ground;
        if grounded {
            let (x, y, z) = (self.player.pos.x.floor() as i32, self.player.pos.y.floor() as i32, self.player.pos.z.floor() as i32);
            let feet_in_water = self.world.materials().get(self.voxels.voxel(x, y, z)).kind == voxel::Kind::Water;
            let ground = if feet_in_water { Ground::Shallows } else { audio::ground_of(self.voxels.voxel(x, y - 1, z)) };
            if !self.was_grounded && self.last_fall_speed < -m(6.0) {
                audio.step(ground, (-self.last_fall_speed / m(16.0)).min(1.0));
                self.stride = 0.0;
            }
            self.stride += glam::Vec2::new(self.player.vel.x, self.player.vel.z).length() * dt;
            if self.stride >= STRIDE {
                self.stride -= STRIDE;
                audio.step(ground, 0.7);
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
                look: self.camera.forward(),
                turn: if dt > 0.0 { turned / dt } else { 0.0 },
            };
            self.player.update(&mut self.voxels, controls, dt);
            self.voxels.trim(self.player.pos);
            self.camera.pos = self.player.eye();
            self.camera.roll = self.player.bank;
        } else {
            self.camera.roll = 0.0;
            self.fly.update(&mut self.camera, &self.keys, dt);
        }
        self.fleet.update(self.player.pos, dt);
        self.cpu.lap(0);
        self.streamer.update(&self.gpu, self.camera.pos, UPLOAD_MS);
        self.far.update(&self.gpu, self.camera.pos);
        self.cpu.lap(1);

        // The day turns.
        if self.settings.day_minutes > 0.0 {
            self.env = Environment::at_time(self.env.time + dt * 24.0 / (self.settings.day_minutes * 60.0));
        }
        self.save_timer -= dt;

        // Ambient life. Every so often, look around for leaves and flowers to spawn from.
        self.scan_timer -= dt;
        if self.scan_timer <= 0.0 {
            self.scan_timer = 2.0;
            self.scanner.request(self.camera.pos, self.frame as u32);
        }
        if let Some(found) = self.scanner.poll() {
            (self.biome, self.leaves, self.flowers) = found;
        }
        let entered_water = (self.player.swimming && !self.was_swimming && self.walking)
            .then(|| self.player.pos + DVec3::new(0.0, m64(0.8), 0.0));
        self.was_swimming = self.player.swimming;
        let cascades: Vec<_> = self.streamer.cascades_near(self.camera.pos, m(120.0)).copied().collect();
        let surroundings = Surroundings {
            camera: self.camera.pos,
            biome: self.biome,
            sun_elevation: self.env.sun_elevation,
            cascades: &cascades,
            leaves: &self.leaves,
            flowers: &self.flowers,
            player_entered_water: entered_water,
            time: self.last_frame.duration_since(self.start).as_secs_f64(),
        };
        self.particles.update(dt, &surroundings, &mut self.voxels);
        self.update_footsteps(dt);

        let clock = self.last_frame.duration_since(self.start).as_secs_f32();
        self.props.clear();
        for boat in self.fleet.boats() {
            vehicles::draw_boat(boat.pos, boat.heading, clock, self.camera.pos, &mut self.props);
        }
        if self.walking {
            match self.player.mode {
                Mode::Boat => vehicles::draw_boat(self.player.pos, self.player.heading, clock, self.camera.pos, &mut self.props),
                Mode::Glide => vehicles::draw_glider(self.player.pos, self.camera.yaw, self.camera.pos, &mut self.props),
                Mode::Walk => {}
            }
        }

        self.cpu.lap(2);
        let Some(frame) = self.surface.acquire(&self.gpu) else {
            return;
        };
        self.cpu.lap(3);
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        let scene = render::Scene {
            camera: &self.camera,
            env: &self.env,
            terrain: &self.streamer,
            far: &self.far,
            particles: self.particles.instances(),
            props: &self.props,
            wind: self.particles.wind.lagged * OLD_VOXEL,
        };
        self.renderer.render(&self.gpu, &mut encoder, &view, &scene);
        self.cpu.lap(4);

        let size = [self.surface.config.width, self.surface.config.height];
        let cam = &self.camera;
        let (frame_ms, speed) = (self.frame_ms, self.fly.speed);
        let gpu_name = &self.gpu.adapter_name;
        let (s, d) = (self.streamer.stats, self.renderer.stats);
        let (particle_count, biome) = (self.particles.len(), self.biome);
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
        let mut settings = self.settings;
        let mut choice = overlay::MenuChoice::default();
        let held = Some(self.streamer.radius()).filter(|r| *r < settings.radius);
        let (show_debug, show_menu, grabbed) = (settings.show_debug, self.show_settings, self.grabbed);
        self.overlay
            .draw(&self.window, &self.gpu, &mut encoder, &view, size, |ctx| {
                if show_menu {
                    choice = overlay::settings_menu(ctx, &mut settings, held, (size[0], size[1]));
                } else if !grabbed {
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
                        ui.label(format!("mode: {mode}   particles {particle_count}   biome {biome:?}"));
                        ui.label("Esc = settings. Click or Tab to look. WASD, Space = jump/swim up, Shift = sprint");
                        ui.label("F = toggle flying (Space/Ctrl up/down, wheel = speed). F3 = hide this panel, F11 = fullscreen");
                        ui.label("G or Space in the air = glider. E = board or leave a boat at a landing");
                    });
            });
        if time != self.env.time {
            self.env = Environment::at_time(time);
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
            r.set_menu(!r.show_settings);
            return;
        }
        if !r.grabbed && r.overlay.on_event(&r.window, &event) {
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
            WindowEvent::Focused(false) => r.set_grab(false),
            WindowEvent::Focused(true) => {
                if r.want_grab {
                    r.set_grab(true);
                }
            }
            WindowEvent::CursorMoved { position, .. } => r.cursor_moved(position.x, position.y),
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
                        } else if code == KeyCode::Tab && !event.repeat {
                            r.want_grab = !r.grabbed;
                            r.set_grab(r.want_grab);
                        } else if code == KeyCode::KeyF && !event.repeat {
                            r.toggle_flying();
                        } else if code == KeyCode::KeyE && !event.repeat {
                            r.interact();
                        } else if r.grabbed {
                            // A fresh press of Space in mid-air opens the glider.
                            if !event.repeat && matches!(code, KeyCode::KeyG | KeyCode::Space) {
                                r.deploy = true;
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
                    log::info!(
                        "after {} frames: {:.1} ms/frame ({:.0} fps), {} chunks, {:.2} M quads drawn, {} far tiles",
                        r.frame,
                        r.frame_ms,
                        1000.0 / r.frame_ms,
                        s.chunks,
                        d.quads_drawn as f64 / 1e6,
                        d.far_tiles
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

    if let Some(path) = &opts.screenshot {
        // Screenshots neither read nor write anything saved, so they are repeatable.
        let settings = opts.overrides.applied_to(Settings::default());
        let gpu = Gpu::headless()?;
        let world = Arc::new(GameWorld::new(settings.seed));
        let camera = make_camera(&opts, &world, None);
        let mut far = FarView::new(&gpu, world.clone());
        let mut streamer = Streamer::new(&gpu, world.clone(), settings.radius);
        let t = Instant::now();
        let env = Environment::at_time(opts.time.unwrap_or(10.0));
        // A few seconds of ambient life, so stills show spray, leaves and fireflies.
        streamer.wait_idle(&gpu, camera.pos);
        let mut voxels = WorldVoxels::new(world.clone());
        let (biome, leaves, flowers) = scan_surroundings(&world, &mut voxels, camera.pos, 1);
        let cascades: Vec<_> = streamer.cascades_near(camera.pos, m(120.0)).copied().collect();
        let mut fleet = Fleet::new(world.clone());
        fleet.refresh(camera.pos);
        let mut props = Vec::new();
        for boat in fleet.boats() {
            vehicles::draw_boat(boat.pos, boat.heading, 0.0, camera.pos, &mut props);
        }
        let mut particles = Particles::new();
        for step in 0..480 {
            let surroundings = Surroundings {
                camera: camera.pos,
                biome,
                sun_elevation: env.sun_elevation,
                cascades: &cascades,
                leaves: &leaves,
                flowers: &flowers,
                player_entered_water: None,
                time: step as f64 / 60.0,
            };
            particles.update(1.0 / 60.0, &surroundings, &mut voxels);
        }
        render::screenshot(
            &gpu,
            path,
            opts.size,
            &camera,
            &env,
            &mut streamer,
            &mut far,
            particles.instances(),
            &props,
            particles.wind.lagged * OLD_VOXEL,
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
