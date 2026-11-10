mod camera;
mod env;
mod farview;
mod gfx;
mod overlay;
mod particles;
mod player;
mod render;
mod terrain;
mod vehicles;

use std::{collections::HashSet, path::PathBuf, sync::Arc, time::Instant};

use anyhow::{Context, Result, bail};
use glam::DVec3;
use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, DeviceId, ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{CursorGrabMode, Window, WindowId},
};
use worldgen::{World, world::GameWorld};

use camera::{Camera, FlyController};
use env::Environment;
use farview::FarView;
use gfx::{Gpu, WindowSurface};
use overlay::Overlay;
use particles::{Particles, Surroundings};
use player::{Controls, Mode, Player, WorldVoxels};
use render::Renderer;
use terrain::Streamer;
use vehicles::Fleet;

struct Options {
    seed: u32,
    /// Camera start: world position, then yaw and pitch in degrees.
    pos: DVec3,
    /// If set, start at this (x, z) and this far above the ground or water there.
    over: Option<(i32, i32, f64)>,
    /// Start flying instead of on foot. Implied by --pos and --over.
    fly: bool,
    look: (f32, f32),
    /// Time of day in hours.
    time: f32,
    /// Chunk load radius.
    radius: i32,
    /// Render one frame to this PNG without opening a window, then exit.
    screenshot: Option<PathBuf>,
    size: (u32, u32),
    /// Real minutes per in-game day; 0 freezes the time of day.
    day_minutes: f32,
    /// Exit after this many frames (smoke tests).
    frames: Option<u64>,
    no_vsync: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            seed: 1,
            pos: DVec3::new(0.0, 90.0, 0.0),
            over: None,
            fly: false,
            look: (35.0, -18.0),
            time: 10.0,
            radius: 12,
            screenshot: None,
            size: (1280, 720),
            day_minutes: 40.0,
            frames: None,
            no_vsync: false,
        }
    }
}

const USAGE: &str = "\
usage: game [options]
  --seed N              world seed (default 1)
  --pos X,Y,Z           camera start position in voxels
  --over X,Z,H          start H voxels above the ground (or water) at X,Z
  --fly                 start in free-fly mode (F toggles in game)
  --look YAW,PITCH      camera direction in degrees
  --time H              time of day, 0-24 (default 10)
  --radius N            chunk load radius (default 12)
  --screenshot FILE     render one frame to a PNG without a window, then exit
  --size WxH            screenshot size (default 1280x720)
  --day-minutes M       real minutes per day, 0 to freeze time (default 40)
  --frames N            exit after N frames
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
            "--seed" => o.seed = value()?.parse()?,
            "--pos" => {
                let v = list::<f64>(&value()?, ',', 3)?;
                o.pos = DVec3::new(v[0], v[1], v[2]);
                o.over = None;
                o.fly = true;
            }
            "--fly" => o.fly = true,
            "--over" => {
                let v = list::<f64>(&value()?, ',', 3)?;
                o.over = Some((v[0] as i32, v[1] as i32, v[2]));
                o.fly = true;
            }
            "--look" => {
                let v = list::<f32>(&value()?, ',', 2)?;
                o.look = (v[0], v[1]);
            }
            "--time" => o.time = value()?.parse()?,
            "--radius" => o.radius = value()?.parse::<i32>()?.clamp(1, 64),
            "--screenshot" => o.screenshot = Some(value()?.into()),
            "--size" => {
                let v = list::<u32>(&value()?, 'x', 2)?;
                o.size = (v[0].max(1), v[1].max(1));
            }
            "--day-minutes" => o.day_minutes = value()?.parse()?,
            "--frames" => o.frames = Some(value()?.parse()?),
            "--no-vsync" => o.no_vsync = true,
            "--help" | "-h" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => bail!("unknown argument '{other}'\n{USAGE}"),
        }
    }
    Ok(o)
}

fn make_world(opts: &Options) -> Arc<GameWorld> {
    Arc::new(GameWorld::new(opts.seed))
}

/// Height of the walkable or swimmable surface at a column.
fn surface_height(world: &GameWorld, x: i32, z: i32) -> f64 {
    let col = world.column(x, z);
    col.water_surface().unwrap_or(col.top() + 1).max(col.top() + 1) as f64
}

/// The starting view: an explicit position if one was given, else the spawn point.
fn make_camera(opts: &Options, world: &GameWorld) -> Camera {
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

/// How close a boat must be to climb aboard.
const BOARD_REACH: f64 = 6.0;

/// Voxel positions with their linear colours.
type Spots = Vec<(DVec3, [f32; 3])>;

/// Samples voxels around the camera for leaves (to shed) and flowers (for butterflies),
/// and reads the biome. Cheap enough to do every couple of seconds.
fn scan_surroundings(
    world: &GameWorld,
    voxels: &mut WorldVoxels,
    camera: DVec3,
    salt: u32,
) -> (worldgen::biome::Biome, Spots, Spots) {
    use worldgen::materials as mat;
    let (cx, cz) = (camera.x.floor() as i32, camera.z.floor() as i32);
    let biome = world.column(cx, cz).biome;
    let (mut leaves, mut flowers) = (Vec::new(), Vec::new());
    let mats = world.materials();
    let to_linear = |c: [u8; 3]| c.map(|v| (v as f32 / 255.0).powf(2.2));
    let mut rng = salt.wrapping_mul(0x9e37_79b9) ^ (cx as u32).wrapping_mul(31) ^ (cz as u32).wrapping_mul(17) | 1;
    for _ in 0..700 {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        let x = cx + (rng % 61) as i32 - 30;
        let z = cz + ((rng >> 8) % 61) as i32 - 30;
        let top = camera.y as i32 + 40;
        let mut below = voxel::AIR;
        for y in ((camera.y as i32 - 12)..=top).rev() {
            let v = voxels.voxel(x, y, z);
            let kind = mats.get(v).kind;
            if kind == voxel::Kind::Foliage && mats.get(below).kind == voxel::Kind::Air {
                leaves.push((DVec3::new(x as f64, y as f64, z as f64), to_linear(mats.get(v).color)));
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
    leaves: Vec<(DVec3, [f32; 3])>,
    flowers: Vec<(DVec3, [f32; 3])>,
    scan_timer: f32,
    biome: worldgen::biome::Biome,
    day_minutes: f32,
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
    show_debug: bool,
    last_frame: Instant,
    start: Instant,
    frame: u64,
    frame_ms: f32,
}

struct App {
    opts: Options,
    run: Option<Running>,
}

impl Running {
    fn new(el: &ActiveEventLoop, opts: &Options) -> Result<Self> {
        let attrs = Window::default_attributes()
            .with_title("Willow Hollow")
            .with_inner_size(winit::dpi::LogicalSize::new(1280, 720));
        let window = Arc::new(el.create_window(attrs)?);
        let (gpu, surface) = Gpu::for_window(window.clone(), !opts.no_vsync)?;
        let overlay = Overlay::new(&window, &gpu, surface.config.format);
        let renderer = Renderer::new(
            &gpu,
            surface.config.format,
            surface.config.width,
            surface.config.height,
        );
        let world = make_world(opts);
        let camera = make_camera(opts, &world);
        let feet = camera.pos - DVec3::new(0.0, player::EYE_HEIGHT, 0.0);
        Ok(Self {
            window,
            overlay,
            renderer,
            camera,
            walking: !opts.fly,
            player: Player::new(feet),
            voxels: WorldVoxels::new(world.clone()),
            far: FarView::new(&gpu, world.clone()),
            streamer: Streamer::new(world.clone(), opts.radius),
            env: Environment::at_time(opts.time),
            particles: Particles::new(),
            leaves: Vec::new(),
            flowers: Vec::new(),
            scan_timer: 0.0,
            biome: worldgen::biome::Biome::Grassland,
            day_minutes: opts.day_minutes,
            was_swimming: false,
            fleet: Fleet::new(world.clone()),
            props: Vec::new(),
            deploy: false,
            last_yaw: 0.0,
            world,
            fly: FlyController { speed: 20.0 },
            keys: HashSet::new(),
            grabbed: false,
            want_grab: true,
            raw_motion_seen: false,
            last_cursor: None,
            show_debug: true,
            last_frame: Instant::now(),
            start: Instant::now(),
            frame: 0,
            frame_ms: 16.0,
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

    fn scan_surroundings(&mut self) {
        let found = scan_surroundings(&self.world, &mut self.voxels, self.camera.pos, self.frame as u32);
        (self.biome, self.leaves, self.flowers) = found;
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

    fn resize(&mut self, width: u32, height: u32) {
        self.surface.resize(&self.gpu, width, height);
        self.renderer.resize(&self.gpu, width, height);
    }

    fn redraw(&mut self) {
        if self.want_grab && !self.grabbed {
            self.set_grab(true);
        }
        let now = Instant::now();
        let raw_dt = (now - self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.frame_ms += (raw_dt * 1000.0 - self.frame_ms) * 0.05;
        let dt = raw_dt.min(0.1);

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
        self.streamer.update(&self.gpu, self.camera.pos, 6);
        self.far.update(&self.gpu, self.camera.pos);

        // The day turns.
        if self.day_minutes > 0.0 {
            self.env = Environment::at_time(self.env.time + dt * 24.0 / (self.day_minutes * 60.0));
        }

        // Ambient life. Every so often, look around for leaves and flowers to spawn from.
        self.scan_timer -= dt;
        if self.scan_timer <= 0.0 {
            self.scan_timer = 2.0;
            self.scan_surroundings();
        }
        let entered_water = (self.player.swimming && !self.was_swimming && self.walking)
            .then(|| self.player.pos + DVec3::new(0.0, 1.0, 0.0));
        self.was_swimming = self.player.swimming;
        let cascades: Vec<_> = self.streamer.cascades_near(self.camera.pos, 150.0).copied().collect();
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

        let Some(frame) = self.surface.acquire(&self.gpu) else {
            return;
        };
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        let scene = render::Scene {
            camera: &self.camera,
            env: &self.env,
            terrain: &self.streamer,
            far: &self.far,
            particles: self.particles.instances(),
            props: &self.props,
            wind: self.particles.wind.lagged,
        };
        self.renderer.render(&self.gpu, &mut encoder, &view, &scene);

        let size = [self.surface.config.width, self.surface.config.height];
        let cam = &self.camera;
        let (frame_ms, speed) = (self.frame_ms, self.fly.speed);
        let gpu_name = &self.gpu.adapter_name;
        let (s, d) = (self.streamer.stats, self.renderer.stats);
        let (particle_count, biome) = (self.particles.len(), self.biome);
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
        let mut radius = self.streamer.radius();
        let (show_debug, grabbed) = (self.show_debug, self.grabbed);
        self.overlay
            .draw(&self.window, &self.gpu, &mut encoder, &view, size, |ctx| {
                if !grabbed {
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
                        ui.add(egui::Slider::new(&mut time, 0.0..=24.0).text("time of day"));
                        ui.add(egui::Slider::new(&mut radius, 2..=32).text("load radius"));
                        ui.label(format!("mode: {mode}   particles {particle_count}   biome {biome:?}"));
                        ui.label("Click or Tab to look, Esc to free the mouse. WASD, Space = jump/swim up, Shift = sprint");
                        ui.label("F = toggle flying (Space/Ctrl up/down, wheel = speed). F3 = hide this panel");
                        ui.label("G or Space in the air = glider. E = board or leave a boat at a landing");
                    });
            });
        if time != self.env.time {
            self.env = Environment::at_time(time);
        }
        self.streamer.set_radius(radius);

        self.gpu.queue.submit([encoder.finish()]);
        self.window.pre_present_notify();
        self.gpu.queue.present(frame);
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
        if !r.grabbed && r.overlay.on_event(&r.window, &event) {
            return;
        }
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(s) => r.resize(s.width, s.height),
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
            } if !r.grabbed => {
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
                        if code == KeyCode::Escape {
                            r.want_grab = false;
                            r.set_grab(false);
                        } else if code == KeyCode::Tab && !event.repeat {
                            r.want_grab = !r.grabbed;
                            r.set_grab(r.want_grab);
                        } else if code == KeyCode::F3 && !event.repeat {
                            r.show_debug = !r.show_debug;
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
                if self.opts.frames.is_some_and(|n| r.frame >= n) {
                    let (s, d) = (r.streamer.stats, r.renderer.stats);
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
        let gpu = Gpu::headless()?;
        let world = make_world(&opts);
        let camera = make_camera(&opts, &world);
        let mut far = FarView::new(&gpu, world.clone());
        let mut streamer = Streamer::new(world.clone(), opts.radius);
        let t = Instant::now();
        let env = Environment::at_time(opts.time);
        // A few seconds of ambient life, so stills show spray, leaves and fireflies.
        streamer.wait_idle(&gpu, camera.pos);
        let mut voxels = WorldVoxels::new(world.clone());
        let (biome, leaves, flowers) = scan_surroundings(&world, &mut voxels, camera.pos, 1);
        let cascades: Vec<_> = streamer.cascades_near(camera.pos, 150.0).copied().collect();
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
            particles.wind.lagged,
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
