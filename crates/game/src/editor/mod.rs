//! The structure editor (`game --structures`): a window for looking at the landmarks the
//! generator makes, one at a time on a patch of plain ground, and for drawing structures
//! of one's own from them, which the world then builds (see
//! `worldgen::features::designs`).

mod houses;
mod state;
mod view;

use std::sync::Arc;

use anyhow::Result;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId},
};

use crate::{
    gfx::{Gpu, WindowSurface},
    overlay::{self, Overlay},
};
pub use state::Options;
use state::Editor;
use view::View;

/// Opens the editor in a window, or with `opts.screenshot` draws it once to a PNG.
pub fn run(opts: Options) -> Result<()> {
    if let Some(path) = opts.screenshot.clone() {
        return screenshot(&opts, &path);
    }
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App { opts, run: None };
    event_loop.run_app(&mut app)?;
    Ok(())
}

struct App {
    opts: Options,
    run: Option<Running>,
}

struct Running {
    window: Arc<Window>,
    gpu: Gpu,
    surface: WindowSurface,
    overlay: Overlay,
    view: View,
    editor: Editor,
}

impl Running {
    fn new(el: &ActiveEventLoop, opts: &Options) -> Result<Self> {
        let attrs = Window::default_attributes()
            .with_title("Willow Hollow: structures")
            .with_inner_size(winit::dpi::LogicalSize::new(1440, 900));
        let window = Arc::new(el.create_window(attrs)?);
        let (gpu, surface) = Gpu::for_window(window.clone(), true)?;
        let overlay = Overlay::new(&window, &gpu, surface.config.format);
        let view = View::new(&gpu, surface.config.format);
        let editor = Editor::new(opts);
        Ok(Running { window, gpu, surface, overlay, view, editor })
    }

    fn frame(&mut self) {
        let Some(frame) = self.surface.acquire(&self.gpu) else { return };
        let target = frame.texture.create_view(&Default::default());
        let size = (self.surface.config.width, self.surface.config.height);
        let ppp = self.window.scale_factor() as f32;
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        self.editor.prepare(&self.gpu, &mut self.view);
        let r = self.editor.viewport;
        self.view.draw(&self.gpu, &mut encoder, &target, size, [r.min.x * ppp, r.min.y * ppp, r.width() * ppp, r.height() * ppp], &self.editor.orbit);
        let editor = &mut self.editor;
        self.overlay.draw(&self.window, &self.gpu, &mut encoder, &target, [size.0, size.1], |ctx| editor.ui(ctx));
        self.gpu.queue.submit([encoder.finish()]);
        self.window.pre_present_notify();
        self.gpu.queue.present(frame);
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
                log::error!("the structure editor could not start: {e:#}");
                el.exit();
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(r) = self.run.as_mut() else { return };
        r.overlay.on_event(&r.window, &event);
        match event {
            WindowEvent::CloseRequested => {
                if r.editor.unsaved() {
                    r.editor.closing = true;
                } else {
                    el.exit();
                }
            }
            WindowEvent::Resized(s) => r.surface.resize(&r.gpu, s.width, s.height),
            WindowEvent::RedrawRequested => r.frame(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if let Some(r) = &self.run {
            if r.editor.quit {
                el.exit();
            }
            r.window.request_redraw();
        }
    }
}

/// Draws the editor once, as it would look in a window `opts.size` big, to a PNG.
fn screenshot(opts: &Options, path: &std::path::Path) -> Result<()> {
    let gpu = Gpu::headless()?;
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let (width, height) = opts.size;
    let mut view = View::new(&gpu, format);
    let mut editor = Editor::new(opts);
    // Once through the panels to find where the view goes and build the model; then
    // the model is framed, and the panels drawn for real over it.
    let layout = |editor: &mut Editor| {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width as f32, height as f32))),
            ..Default::default()
        };
        for _ in 0..2 {
            ctx.begin_pass(input.clone());
            editor.ui(&ctx);
            let _ = ctx.end_pass();
        }
    };
    layout(&mut editor);
    editor.prepare(&gpu, &mut view);
    layout(&mut editor);
    editor.prepare(&gpu, &mut view);
    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("structure editor shot"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target_view = target.create_view(&Default::default());
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    let r = editor.viewport;
    view.draw(&gpu, &mut encoder, &target_view, (width, height), [r.min.x, r.min.y, r.width(), r.height()], &editor.orbit);
    overlay::draw_headless(&gpu, &mut encoder, &target_view, format, (width, height), |ctx| editor.ui(ctx));
    let row = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("structure editor readback"),
        size: (row * height) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(height) } },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    );
    gpu.queue.submit([encoder.finish()]);
    buffer.slice(..).map_async(wgpu::MapMode::Read, |r| r.expect("screenshot buffer map failed"));
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
    log::info!("structure editor screenshot {}", path.display());
    Ok(())
}
