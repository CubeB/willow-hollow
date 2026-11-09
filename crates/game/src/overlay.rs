use winit::{event::WindowEvent, window::Window};

use crate::gfx::Gpu;

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
