//! The map's textures and its one pipeline (shaders/minimap.wgsl).

use bytemuck::{Pod, Zeroable};
use worldgen::macro_layout::MACRO_SIZE;

use super::picture::TEX;
use super::reveal::Rect;
use crate::gfx::Gpu;

/// Continents the map can show at once: the two by two around its middle.
pub const SLOTS: usize = 4;

pub const VEIL: u32 = 0;
pub const MAP: u32 = 1;
pub const GLOW: u32 = 2;
pub const ARROW: u32 = 3;
pub const CIRCLE: u32 = 4;
pub const TRIANGLE: u32 = 5;
pub const DIAMOND: u32 = 6;
pub const RING: u32 = 7;
pub const BAR: u32 = 8;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
pub struct Uniforms {
    pub screen: [f32; 2],
    pub centre: [f32; 2],
    pub half: [f32; 2],
    pub voxels_per_px: f32,
    pub round: f32,
    pub time: f32,
    pub corner: f32,
    /// The world at the map's middle, wrapped, for the grain of the blank.
    pub look: [f32; 2],
    pub slots: [[f32; 4]; SLOTS],
}

/// One quad: the map, a mark, the arrow.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct Instance {
    /// Middle, in pixels.
    pub pos: [f32; 2],
    /// Half its width and height, in pixels, before turning.
    pub size: [f32; 2],
    /// Which way its x axis points on the screen, a unit vector.
    pub dir: [f32; 2],
    /// sRGB and alpha.
    pub color: [f32; 4],
    pub kind: u32,
    pub pad: u32,
}

impl Instance {
    pub fn new(kind: u32, pos: [f32; 2], size: f32, color: [f32; 4]) -> Self {
        Self { pos, size: [size, size], dir: [1.0, 0.0], color, kind, pad: 0 }
    }
}

pub struct MapGpu {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    instances: wgpu::Buffer,
    capacity: usize,
    pictures: wgpu::Texture,
    reveals: wgpu::Texture,
    bind: wgpu::BindGroup,
}

fn layers(device: &wgpu::Device, label: &str, format: wgpu::TextureFormat) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: TEX as u32, height: TEX as u32, depth_or_array_layers: SLOTS as u32 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("map marks"),
        size: (capacity * size_of::<Instance>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

impl MapGpu {
    pub fn new(gpu: &Gpu, format: wgpu::TextureFormat) -> Self {
        let device = &gpu.device;
        let source = format!("const MACRO_SIZE: f32 = {:?};\n{}", MACRO_SIZE as f32, include_str!("../shaders/minimap.wgsl"));
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("minimap"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2Array,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("minimap"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture(1),
                texture(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("minimap"),
            size: size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let pictures = layers(device, "map pictures", wgpu::TextureFormat::Rgba8Unorm);
        let reveals = layers(device, "map reveals", wgpu::TextureFormat::Rg8Unorm);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("minimap"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let array = |t: &wgpu::Texture| {
            t.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() })
        };
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("minimap"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&array(&pictures)) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&array(&reveals)) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("minimap"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Float32x4, 4 => Uint32];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("minimap"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<Instance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &attrs,
                })],
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let capacity = 256;
        Self { pipeline, uniforms, instances: instance_buffer(device, capacity), capacity, pictures, reveals, bind }
    }

    fn write(gpu: &Gpu, texture: &wgpu::Texture, layer: usize, rect: Rect, bytes_per_texel: usize, data: &[u8]) {
        let (w, h) = ((rect.2 - rect.0) as u32, (rect.3 - rect.1) as u32);
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: rect.0 as u32, y: rect.1 as u32, z: layer as u32 },
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * bytes_per_texel as u32), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
    }

    /// A continent's picture (TEX square RGBA) into a layer.
    pub fn picture(&self, gpu: &Gpu, layer: usize, rgba: &[u8]) {
        Self::write(gpu, &self.pictures, layer, (0, 0, TEX, TEX), 4, rgba);
    }

    /// Part of a continent's reveal mask (two bytes a texel, `rect` row by row).
    pub fn reveal(&self, gpu: &Gpu, layer: usize, rect: Rect, rows: &[u8]) {
        Self::write(gpu, &self.reveals, layer, rect, 2, rows);
    }

    pub fn draw(&mut self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, uniforms: &Uniforms, instances: &[Instance]) {
        if instances.is_empty() {
            return;
        }
        if instances.len() > self.capacity {
            self.capacity = instances.len().next_power_of_two();
            self.instances = instance_buffer(&gpu.device, self.capacity);
        }
        gpu.queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(uniforms));
        gpu.queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(instances));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("minimap"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind, &[]);
        pass.set_vertex_buffer(0, self.instances.slice(..));
        pass.draw(0..4, 0..instances.len() as u32);
    }
}
