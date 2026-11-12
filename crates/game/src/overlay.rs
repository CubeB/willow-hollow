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
                ui.checkbox(&mut s.shafts, "Light shafts");
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
        let out = self.ctx.end_pass();
        let mut textures = out.textures_delta;
        self.state.handle_platform_output(window, out.platform_output);

        let prims = self.ctx.tessellate(out.shapes, out.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: size,
            pixels_per_point: out.pixels_per_point,
        };
        for (id, deltas) in textures.set.drain() {
            for delta in &deltas {
                self.renderer
                    .update_texture(&gpu.device, &gpu.queue, id, delta);
            }
        }
        self.renderer
            .update_buffers(&gpu.device, &gpu.queue, encoder, &prims, &screen);
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
            self.renderer.render(&mut pass, &prims, &screen);
        }
        for id in textures.free.drain() {
            self.renderer.free_texture(&id);
        }
    }
}
