mod camera;
mod env;
mod gfx;
mod overlay;
mod player;
mod render;
mod terrain;

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
use worldgen::world::GameWorld;

use camera::{Camera, FlyController};
use env::Environment;
use gfx::{Gpu, WindowSurface};
use overlay::Overlay;
use player::{Controls, Player, WorldVoxels};
use render::Renderer;
use terrain::Streamer;

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

struct Running {
    window: Arc<Window>,
    gpu: Gpu,
    surface: WindowSurface,
    overlay: Overlay,
    renderer: Renderer,
    streamer: Streamer,
    env: Environment,
    camera: Camera,
    /// On foot, or flying free of collision.
    walking: bool,
    player: Player,
    voxels: WorldVoxels,
    fly: FlyController,
    keys: HashSet<KeyCode>,
    grabbed: bool,
    last_frame: Instant,
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
            .with_title("Voxel")
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
            streamer: Streamer::new(world, opts.radius),
            env: Environment::at_time(opts.time),
            fly: FlyController { speed: 20.0 },
            keys: HashSet::new(),
            grabbed: false,
            last_frame: Instant::now(),
            frame: 0,
            frame_ms: 16.0,
            gpu,
            surface,
        })
    }

    fn set_grab(&mut self, grab: bool) {
        if grab {
            let ok = self
                .window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| self.window.set_cursor_grab(CursorGrabMode::Confined))
                .is_ok();
            if !ok {
                return;
            }
        } else {
            let _ = self.window.set_cursor_grab(CursorGrabMode::None);
            self.keys.clear();
        }
        self.window.set_cursor_visible(!grab);
        self.grabbed = grab;
    }

    fn toggle_flying(&mut self) {
        self.walking = !self.walking;
        if self.walking {
            // Drop in wherever the camera is.
            let feet = self.camera.pos - DVec3::new(0.0, player::EYE_HEIGHT, 0.0);
            self.player = Player::new(feet);
        }
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.surface.resize(&self.gpu, width, height);
        self.renderer.resize(&self.gpu, width, height);
    }

    fn redraw(&mut self) {
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
            let controls = Controls {
                wish: fwd * axis(KeyCode::KeyW, KeyCode::KeyS)
                    + right * axis(KeyCode::KeyD, KeyCode::KeyA),
                jump: self.keys.contains(&KeyCode::Space),
                sprint: self.keys.contains(&KeyCode::ShiftLeft),
            };
            self.player.update(&mut self.voxels, controls, dt);
            self.voxels.trim(self.player.pos);
            self.camera.pos = self.player.eye();
        } else {
            self.fly.update(&mut self.camera, &self.keys, dt);
        }
        self.streamer.update(&self.gpu, self.camera.pos, 6);

        let Some(frame) = self.surface.acquire(&self.gpu) else {
            return;
        };
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        self.renderer.render(
            &self.gpu,
            &mut encoder,
            &view,
            &self.camera,
            &self.env,
            self.streamer.chunks(),
        );

        let size = [self.surface.config.width, self.surface.config.height];
        let cam = &self.camera;
        let (frame_ms, speed) = (self.frame_ms, self.fly.speed);
        let gpu_name = &self.gpu.adapter_name;
        let (s, d) = (self.streamer.stats, self.renderer.stats);
        let mode = match (self.walking, self.player.swimming, self.player.on_ground) {
            (false, ..) => "flying",
            (true, true, _) => "swimming",
            (true, false, true) => "walking",
            (true, false, false) => "in the air",
        };
        let mut time = self.env.time;
        let mut radius = self.streamer.radius();
        self.overlay
            .draw(&self.window, &self.gpu, &mut encoder, &view, size, |ctx| {
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
                            "chunks {} in {} columns, drawn {} (culled {})",
                            s.chunks, s.columns, d.chunks_drawn, d.chunks_culled
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
                        ui.label(format!("mode: {mode}"));
                        ui.label("Click to look, Esc to release. WASD, Space = jump/swim up, Shift = sprint");
                        ui.label("F = toggle flying (Space/Ctrl up/down, wheel = speed)");
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
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } if !r.grabbed => r.set_grab(true),
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
                            r.set_grab(false);
                        } else if code == KeyCode::KeyF && !event.repeat {
                            r.toggle_flying();
                        } else if r.grabbed {
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
        let mut streamer = Streamer::new(world, opts.radius);
        let t = Instant::now();
        render::screenshot(
            &gpu,
            path,
            opts.size,
            &camera,
            &Environment::at_time(opts.time),
            &mut streamer,
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
