//! The map's textures and its one pipeline (shaders/minimap.wgsl).

use bytemuck::{Pod, Zeroable};

use crate::gfx::Gpu;

pub const VEIL: u32 = 0;
pub const GLOW: u32 = 2;
pub const ARROW: u32 = 3;
pub const CIRCLE: u32 = 4;
pub const TRIANGLE: u32 = 5;
pub const DIAMOND: u32 = 6;
pub const RING: u32 = 7;
pub const BAR: u32 = 8;
/// A plain bar, as long and as thick as its size.
pub const LINE: u32 = 9;
/// The map's own shape, filled with one colour: what the corner map's model lies on.
pub const PANEL: u32 = 10;

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
    bind: wgpu::BindGroup,
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
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("minimap"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/minimap.wgsl").into()),
        });
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
            ],
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("minimap"),
            size: size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("minimap"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() },
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
        Self { pipeline, uniforms, instances: instance_buffer(device, capacity), capacity, bind }
    }

    /// Sets what the passes of this frame draw.
    pub fn load(&mut self, gpu: &Gpu, uniforms: &Uniforms, instances: &[Instance]) {
        if instances.len() > self.capacity {
            self.capacity = instances.len().next_power_of_two();
            self.instances = instance_buffer(&gpu.device, self.capacity);
        }
        gpu.queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(uniforms));
        gpu.queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(instances));
    }

    /// Draws some of the instances loaded this frame.
    pub fn pass(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, range: std::ops::Range<u32>) {
        if range.is_empty() {
            return;
        }
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
        pass.draw(0..4, range);
    }
}

/// The large map's model and the pipeline that draws it (shaders/relief.wgsl).
pub struct ReliefGpu {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind: wgpu::BindGroup,
    mesh: Option<(wgpu::Buffer, wgpu::Buffer, u32)>,
    depth: Option<((u32, u32), wgpu::TextureView)>,
}

impl ReliefGpu {
    pub fn new(gpu: &Gpu, format: wgpu::TextureFormat) -> Self {
        let device = &gpu.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("map relief"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/relief.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("map relief"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("map relief"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("map relief"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("map relief"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Unorm8x4];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("map relief"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<super::relief::Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attrs,
                })],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: crate::gfx::DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self { pipeline, uniforms, bind, mesh: None, depth: None }
    }

    /// Takes a new model in place of the last.
    pub fn upload(&mut self, gpu: &Gpu, vertices: &[super::relief::Vertex], indices: &[u32]) {
        use wgpu::util::DeviceExt;
        if indices.is_empty() {
            self.mesh = None;
            return;
        }
        let buffer = |label, contents, usage| gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents, usage });
        self.mesh = Some((
            buffer("map relief vertices", bytemuck::cast_slice(vertices), wgpu::BufferUsages::VERTEX),
            buffer("map relief indices", bytemuck::cast_slice(indices), wgpu::BufferUsages::INDEX),
            indices.len() as u32,
        ));
    }

    /// Draws the model over `target`, `size` pixels, through `transform`: over all of
    /// it, or within `within`, a rectangle of it given by its corner and size in pixels.
    pub fn draw(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        size: (u32, u32),
        transform: glam::Mat4,
        within: Option<[u32; 4]>,
    ) {
        let Some((vertices, indices, count)) = &self.mesh else { return };
        if self.depth.as_ref().is_none_or(|d| d.0 != size) {
            self.depth = Some((size, crate::gfx::create_depth(&gpu.device, size.0, size.1)));
        }
        let depth = &self.depth.as_ref().expect("just made").1;
        gpu.queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&transform.to_cols_array()));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("map relief"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if let Some([x, y, w, h]) = within {
            let (w, h) = (w.min(size.0.saturating_sub(x)), h.min(size.1.saturating_sub(y)));
            if w == 0 || h == 0 {
                return;
            }
            pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
            pass.set_scissor_rect(x, y, w, h);
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind, &[]);
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..*count, 0, 0..1);
    }
}
