use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use anyhow::{Context, Result};
use winit::window::Window;

pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Device and queue, independent of any window so screenshots can render headless.
pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub adapter_name: String,
    /// Set once the GPU is lost or reports an error nothing was waiting for.
    failed: Arc<AtomicBool>,
}

/// A window's swapchain.
pub struct WindowSurface {
    pub surface: wgpu::Surface<'static>,
    pub config: wgpu::SurfaceConfiguration,
}

fn instance() -> wgpu::Instance {
    let descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    wgpu::Instance::new(descriptor)
}

async fn request(
    instance: &wgpu::Instance,
    surface: Option<&wgpu::Surface<'static>>,
) -> Result<(wgpu::Adapter, Gpu)> {
    // HighPerformance matters on laptops: it selects the discrete GPU over the integrated one.
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: surface,
            ..Default::default()
        })
        .await
        .context("no suitable GPU adapter")?;
    let info = adapter.get_info();
    log::info!("GPU: {} ({:?}, {:?})", info.name, info.device_type, info.backend);
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("device"),
            // Lets the renderer submit chunk lists as multi-draws where available.
            // Timestamps let the debug panel show where each frame's time goes.
            required_features: adapter.features()
                & (wgpu::Features::INDIRECT_FIRST_INSTANCE
                    | wgpu::Features::TIMESTAMP_QUERY
                    | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS
                    | wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES),
            // The chunk arena is one buffer, and can outgrow the 256 MiB default.
            required_limits: wgpu::Limits {
                max_buffer_size: adapter.limits().max_buffer_size,
                ..Default::default()
            },
            ..Default::default()
        })
        .await
        .context("device request failed")?;
    let failed = Arc::new(AtomicBool::new(false));
    let flag = failed.clone();
    device.set_device_lost_callback(move |reason, message| {
        // Destroyed is just the game shutting down.
        if reason != wgpu::DeviceLostReason::Destroyed {
            log::error!("GPU device lost ({reason:?}): {message}");
            flag.store(true, Ordering::Release);
        }
    });
    // wgpu panics on these by default, which would lose the player's position.
    let flag = failed.clone();
    device.on_uncaptured_error(Arc::new(move |error| {
        log::error!("GPU error: {error}");
        flag.store(true, Ordering::Release);
    }));
    let gpu = Gpu {
        device,
        queue,
        adapter_name: info.name,
        failed,
    };
    Ok((adapter, gpu))
}

impl Gpu {
    pub fn has_failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }

    /// A vertex buffer holding `data`, or None if the GPU has no room for it.
    pub fn try_vertex_buffer(&self, label: &str, data: &[u8]) -> Option<wgpu::Buffer> {
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: data.len() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        if pollster::block_on(scope.pop()).is_some() {
            log::warn!("no GPU memory for a {} KiB {label} buffer", data.len() >> 10);
            return None;
        }
        self.queue.write_buffer(&buffer, 0, data);
        Some(buffer)
    }

    pub fn headless() -> Result<Gpu> {
        let instance = instance();
        let (_, gpu) = pollster::block_on(request(&instance, None))?;
        Ok(gpu)
    }

    pub fn for_window(window: Arc<Window>, vsync: bool) -> Result<(Gpu, WindowSurface)> {
        let instance = instance();
        let size = window.inner_size();
        let surface = instance.create_surface(window)?;
        let (adapter, gpu) = pollster::block_on(request(&instance, Some(&surface)))?;
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .context("surface not supported by adapter")?;
        let caps = surface.get_capabilities(&adapter);
        // A plain (non-sRGB) format: the tonemap pass encodes sRGB itself, and egui
        // expects to write already-encoded colours.
        if let Some(f) = caps.formats.iter().copied().find(|f| !f.is_srgb()) {
            config.format = f;
        }
        config.present_mode = present_mode(vsync);
        // One frame queued at most, so what's on screen answers the mouse sooner.
        config.desired_maximum_frame_latency = 1;
        surface.configure(&gpu.device, &config);
        Ok((gpu, WindowSurface { surface, config }))
    }
}

fn present_mode(vsync: bool) -> wgpu::PresentMode {
    if vsync {
        wgpu::PresentMode::AutoVsync
    } else {
        wgpu::PresentMode::AutoNoVsync
    }
}

impl WindowSurface {
    pub fn set_vsync(&mut self, gpu: &Gpu, vsync: bool) {
        self.config.present_mode = present_mode(vsync);
        self.surface.configure(&gpu.device, &self.config);
    }

    pub fn resize(&mut self, gpu: &Gpu, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&gpu.device, &self.config);
    }

    /// Next swapchain image, or None if this frame should be skipped.
    pub fn acquire(&mut self, gpu: &Gpu) -> Option<wgpu::SurfaceTexture> {
        use wgpu::CurrentSurfaceTexture as C;
        match self.surface.get_current_texture() {
            C::Success(t) | C::Suboptimal(t) => Some(t),
            C::Outdated | C::Lost => {
                self.surface.configure(&gpu.device, &self.config);
                None
            }
            _ => None,
        }
    }
}

pub fn create_depth(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    tex.create_view(&Default::default())
}
