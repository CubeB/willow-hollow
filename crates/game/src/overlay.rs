use winit::{event::WindowEvent, window::Window};

use crate::{
    gfx::Gpu,
    settings::{self, Settings},
};

/// Window sizes offered in the settings menu.
const WINDOW_SIZES: [(u32, u32); 4] = [(1280, 720), (1600, 900), (1920, 1080), (2560, 1440)];

/// A chunk's width in metres, for showing the load radius as a distance.
const CHUNK_METRES: f32 = voxel::CHUNK as f32 / worldgen::scale::VOXELS_PER_METRE;

/// What the player asked of the settings menu this frame.
#[derive(Default)]
pub struct MenuChoice {
    pub resume: bool,
    pub quit: bool,
}

/// The settings menu. `held` is the radius in force, if graphics memory is holding the
/// view closer than the setting; `window` is the current size of the window.
pub fn settings_menu(ctx: &egui::Context, s: &mut Settings, held: Option<i32>, window: (u32, u32)) -> MenuChoice {
    let mut choice = MenuChoice::default();
    egui::Window::new("Settings")
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .resizable(false)
        .collapsible(false)
        .show(ctx, |ui| {
            egui::Grid::new("settings").num_columns(2).spacing([24.0, 10.0]).show(ui, |ui| {
                ui.label("Field of view");
                ui.add(egui::Slider::new(&mut s.fov, settings::FOV).fixed_decimals(0).suffix("\u{b0}"));
                ui.end_row();

                ui.label("Mouse sensitivity");
                ui.add(egui::Slider::new(&mut s.sensitivity, settings::SENSITIVITY).fixed_decimals(2));
                ui.end_row();

                ui.label("Volume");
                ui.add(egui::Slider::new(&mut s.volume, settings::VOLUME).fixed_decimals(2));
                ui.end_row();

                ui.label("Load radius");
                ui.vertical(|ui| {
                    ui.add(egui::Slider::new(&mut s.radius, settings::RADIUS).suffix(" chunks"));
                    ui.small(match held {
                        Some(r) => format!("Held to {r} chunks, about {:.0} m, by graphics memory", r as f32 * CHUNK_METRES),
                        None => format!("About {:.0} m", s.radius as f32 * CHUNK_METRES),
                    });
                });
                ui.end_row();

                ui.label("Length of a day");
                ui.vertical(|ui| {
                    ui.add(egui::Slider::new(&mut s.day_minutes, settings::DAY_MINUTES).fixed_decimals(0).suffix(" min"));
                    if s.day_minutes == 0.0 {
                        ui.small("The sun stays where it is");
                    }
                });
                ui.end_row();

                ui.label("Length of a year");
                ui.add(egui::Slider::new(&mut s.year_days, settings::YEAR_DAYS).fixed_decimals(0).suffix(" days"));
                ui.end_row();

                ui.label("Window size");
                ui.add_enabled_ui(!s.fullscreen, |ui| {
                    let mut pick = window;
                    egui::ComboBox::from_id_salt("window size")
                        .selected_text(format!("{} x {}", window.0, window.1))
                        .show_ui(ui, |ui| {
                            for size in WINDOW_SIZES {
                                ui.selectable_value(&mut pick, size, format!("{} x {}", size.0, size.1));
                            }
                        });
                    if pick != window {
                        s.window = Some(pick);
                    }
                });
                ui.end_row();

                ui.label("");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut s.fullscreen, "Fullscreen");
                    ui.checkbox(&mut s.vsync, "VSync");
                });
                ui.end_row();

                ui.label("");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut s.shafts, "Light shafts");
                    ui.checkbox(&mut s.minimap, "Map in the corner");
                });
                ui.end_row();
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                choice.resume = ui.button("Back to the game").clicked();
                choice.quit = ui.button("Quit").clicked();
            });
        });
    choice
}

/// egui debug overlay drawn on top of the final image.
pub struct Overlay {
    ctx: egui::Context,
    state: egui_winit::State,
    renderer: egui_wgpu::Renderer,
}

impl Overlay {
    pub fn new(window: &Window, gpu: &Gpu, format: wgpu::TextureFormat) -> Self {
        let ctx = egui::Context::default();
        let state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            None,
            None,
        );
        let renderer = egui_wgpu::Renderer::new(&gpu.device, format, Default::default());
        Self {
            ctx,
            state,
            renderer,
        }
    }

    /// Returns true if egui consumed the event.
    pub fn on_event(&mut self, window: &Window, event: &WindowEvent) -> bool {
        self.state.on_window_event(window, event).consumed
    }

    pub fn draw(
        &mut self,
        window: &Window,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        size: [u32; 2],
        build: impl FnOnce(&egui::Context),
    ) {
        let input = self.state.take_egui_input(window);
        self.ctx.begin_pass(input);
        build(&self.ctx);
        let mut out = self.ctx.end_pass();
        self.state.handle_platform_output(window, std::mem::take(&mut out.platform_output));
        paint(&self.ctx, &mut self.renderer, gpu, encoder, target, size, out);
    }
}

/// Hands what egui drew to the GPU, over `target`.
fn paint(
    ctx: &egui::Context,
    renderer: &mut egui_wgpu::Renderer,
    gpu: &Gpu,
    encoder: &mut wgpu::CommandEncoder,
    target: &wgpu::TextureView,
    size: [u32; 2],
    out: egui::FullOutput,
) {
    let mut textures = out.textures_delta;
    let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
    let screen = egui_wgpu::ScreenDescriptor {
        size_in_pixels: size,
        pixels_per_point: out.pixels_per_point,
    };
    for (id, deltas) in textures.set.drain() {
        for delta in &deltas {
            renderer.update_texture(&gpu.device, &gpu.queue, id, delta);
        }
    }
    renderer.update_buffers(&gpu.device, &gpu.queue, encoder, &prims, &screen);
    {
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("overlay"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime();
        renderer.render(&mut pass, &prims, &screen);
    }
    for id in textures.free.drain() {
        renderer.free_texture(&id);
    }
}

/// Draws the map's names, given in pixels, with a soft halo so they read on any ground.
pub fn map_labels(ctx: &egui::Context, labels: &[crate::minimap::Label]) {
    let ppp = ctx.pixels_per_point();
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("map labels")));
    for l in labels {
        let [r, g, b, a] = l.color;
        let dark = (r as u32 + g as u32 + b as u32) < 384;
        let halo = if dark {
            egui::Color32::from_rgba_unmultiplied(246, 242, 232, 140)
        } else {
            egui::Color32::from_rgba_unmultiplied(20, 20, 22, 140)
        };
        let font = egui::FontId::proportional(l.size / ppp);
        let at = egui::pos2(l.pos[0] / ppp, l.pos[1] / ppp);
        let align = if l.left {
            egui::Align2::LEFT_CENTER
        } else if l.right {
            egui::Align2::RIGHT_CENTER
        } else {
            egui::Align2::CENTER_CENTER
        };
        let step = 1.0 / ppp;
        for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
            painter.text(at + egui::vec2(dx * step, dy * step), align, &l.text, font.clone(), halo);
        }
        painter.text(at, align, &l.text, font, egui::Color32::from_rgba_unmultiplied(r, g, b, a));
    }
}

/// Draws egui over a frame without a window, for screenshots.
pub fn draw_headless(
    gpu: &Gpu,
    encoder: &mut wgpu::CommandEncoder,
    target: &wgpu::TextureView,
    format: wgpu::TextureFormat,
    size: (u32, u32),
    mut build: impl FnMut(&egui::Context),
) {
    let ctx = egui::Context::default();
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0 as f32, size.1 as f32))),
        ..Default::default()
    };
    // Twice over: a window is only measured the first time it is shown, and drawn the next.
    ctx.begin_pass(input.clone());
    build(&ctx);
    let mut first = ctx.end_pass();
    ctx.begin_pass(input);
    build(&ctx);
    let mut out = ctx.end_pass();
    // The first pass made the textures the second is drawn with.
    let mut set = std::mem::take(&mut first.textures_delta.set);
    for (id, mut deltas) in out.textures_delta.set.drain() {
        set.entry(id).or_default().append(&mut deltas);
    }
    out.textures_delta.set = set;
    let mut renderer = egui_wgpu::Renderer::new(&gpu.device, format, Default::default());
    paint(&ctx, &mut renderer, gpu, encoder, target, [size.0, size.1], out);
}
