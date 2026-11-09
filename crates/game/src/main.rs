mod camera;
mod gfx;
mod overlay;

use std::{collections::HashSet, sync::Arc, time::Instant};

use anyhow::Result;
use glam::DVec3;
use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, DeviceId, ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{CursorGrabMode, Window, WindowId},
};

use camera::{Camera, FlyController};
use gfx::{Gpu, WindowSurface};
use overlay::Overlay;

#[derive(Default)]
struct Options {
    /// Exit after this many frames (smoke tests).
    frames: Option<u64>,
    no_vsync: bool,
}

fn parse_args() -> Options {
    let mut o = Options::default();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--frames" => o.frames = args.next().and_then(|v| v.parse().ok()),
            "--no-vsync" => o.no_vsync = true,
            other => log::warn!("unknown argument: {other}"),
        }
    }
    o
}

struct Running {
    window: Arc<Window>,
    gpu: Gpu,
    surface: WindowSurface,
    overlay: Overlay,
    camera: Camera,
    fly: FlyController,
    keys: HashSet<KeyCode>,
    grabbed: bool,
    last_frame: Instant,
    frame: u64,
    fps: f32,
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
        Ok(Self {
            window,
            gpu,
            surface,
            overlay,
            camera: Camera::new(DVec3::new(0.0, 80.0, 0.0), 0.0, -15.0),
            fly: FlyController { speed: 20.0 },
            keys: HashSet::new(),
            grabbed: false,
            last_frame: Instant::now(),
            frame: 0,
            fps: 0.0,
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

    fn redraw(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        if dt > 0.0 {
            self.fps += (1.0 / dt - self.fps) * 0.05;
        }
        self.fly.update(&mut self.camera, &self.keys, dt);

        let Some(frame) = self.surface.acquire(&self.gpu) else {
            return;
        };
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&Default::default());
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.25,
                            g: 0.45,
                            b: 0.75,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        let size = [self.surface.config.width, self.surface.config.height];
        let (cam, fps, speed, gpu_name) = (
            &self.camera,
            self.fps,
            self.fly.speed,
            self.gpu.adapter_name.clone(),
        );
        self.overlay
            .draw(&self.window, &self.gpu, &mut encoder, &view, size, |ctx| {
                egui::Window::new("Debug")
                    .default_pos([8.0, 8.0])
                    .show(ctx, |ui| {
                        ui.label(format!("{fps:.0} fps  |  {gpu_name}"));
                        ui.label(format!(
                            "pos {:.1} {:.1} {:.1}",
                            cam.pos.x, cam.pos.y, cam.pos.z
                        ));
                        ui.label(format!(
                            "yaw {:.0}  pitch {:.0}  speed {speed:.0}",
                            cam.yaw.to_degrees(),
                            cam.pitch.to_degrees()
                        ));
                        ui.label("Click to look, Esc to release, WASD/Space/Ctrl, Shift = fast");
                    });
            });
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
            WindowEvent::Resized(s) => r.surface.resize(&r.gpu, s.width, s.height),
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
        if let DeviceEvent::MouseMotion { delta } = event {
            if r.grabbed {
                r.camera.add_look(delta.0 as f32, delta.1 as f32);
            }
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
    let opts = parse_args();
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App { opts, run: None };
    event_loop.run_app(&mut app)?;
    Ok(())
}
