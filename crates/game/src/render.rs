//! Scene renderer: sky, opaque terrain and water into an HDR target, then a tonemap
//! pass to whatever the output is (window or screenshot texture).

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use voxel::{CHUNK, mesh::Vertex};
use wgpu::util::DeviceExt;

use crate::{
    camera::Camera,
    env::Environment,
    farview::{FarVertex, FarView},
    gfx::{DEPTH_FORMAT, Gpu, create_depth},
    terrain::{MeshBuf, Streamer},
};

const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    cam_right: [f32; 4],
    cam_up: [f32; 4],
    cam_fwd: [f32; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    sky_zenith: [f32; 4],
    sky_horizon: [f32; 4],
    fog: [f32; 4],
    cam_wrapped: [f32; 4],
}

/// Per-chunk instance data.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Instance {
    /// Chunk origin relative to the camera.
    offset: [f32; 3],
    /// Chunk origin in world voxels (for stable per-voxel hashing).
    origin: [i32; 3],
}

/// Everything one frame draws.
pub struct Scene<'a> {
    pub camera: &'a Camera,
    pub env: &'a Environment,
    pub terrain: &'a Streamer,
    pub far: &'a FarView,
}

#[derive(Default, Clone, Copy)]
pub struct DrawStats {
    pub chunks_drawn: u32,
    pub chunks_culled: u32,
    pub quads_drawn: u64,
    pub far_tiles: u32,
}

pub struct Renderer {
    width: u32,
    height: u32,
    globals: wgpu::Buffer,
    globals_bind: wgpu::BindGroup,
    hdr_view: wgpu::TextureView,
    hdr_layout: wgpu::BindGroupLayout,
    hdr_bind: wgpu::BindGroup,
    depth_view: wgpu::TextureView,
    sky: wgpu::RenderPipeline,
    opaque: wgpu::RenderPipeline,
    water: wgpu::RenderPipeline,
    far: wgpu::RenderPipeline,
    /// One huge quad at sea level.
    sea: wgpu::Buffer,
    tonemap: wgpu::RenderPipeline,
    instances: wgpu::Buffer,
    instance_capacity: usize,
    quad_indices: wgpu::Buffer,
    quad_capacity: u32,
    pub stats: DrawStats,
    start: std::time::Instant,
}

fn shader(device: &wgpu::Device, label: &str, source: String) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
}

fn hdr_target(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    width: u32,
    height: u32,
) -> (wgpu::TextureView, wgpu::BindGroup) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("hdr"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: HDR_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = tex.create_view(&Default::default());
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("hdr"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(&view),
        }],
    });
    (view, bind)
}

fn quad_index_buffer(device: &wgpu::Device, quads: u32) -> wgpu::Buffer {
    let mut idx = Vec::with_capacity(quads as usize * 6);
    for q in 0..quads {
        let b = q * 4;
        idx.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
    }
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("quad indices"),
        contents: bytemuck::cast_slice(&idx),
        usage: wgpu::BufferUsages::INDEX,
    })
}

fn instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("chunk instances"),
        size: (capacity * size_of::<Instance>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

impl Renderer {
    /// `output_format` is the format of the texture the final image is written to.
    pub fn new(gpu: &Gpu, output_format: wgpu::TextureFormat, width: u32, height: u32) -> Self {
        let device = &gpu.device;
        let common = include_str!("shaders/common.wgsl");

        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let globals_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });
        let scene_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene"),
            bind_group_layouts: &[Some(&globals_layout)],
            immediate_size: 0,
        });

        let hdr_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hdr"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let (hdr_view, hdr_bind) = hdr_target(device, &hdr_layout, width, height);
        let depth_view = create_depth(device, width, height);

        // Reverse-Z: nearer is greater. The sky draws first and ignores depth.
        let depth = |write: bool, compare: wgpu::CompareFunction| {
            Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(write),
                depth_compare: Some(compare),
                stencil: Default::default(),
                bias: Default::default(),
            })
        };
        let hdr_target_state = |blend: Option<wgpu::BlendState>| {
            [Some(wgpu::ColorTargetState {
                format: HDR_FORMAT,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })]
        };

        let sky_module = shader(device, "sky", format!("{common}\n{}", include_str!("shaders/sky.wgsl")));
        let sky = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky"),
            layout: Some(&scene_layout),
            vertex: wgpu::VertexState {
                module: &sky_module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: depth(false, wgpu::CompareFunction::Always),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &sky_module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &hdr_target_state(None),
            }),
            multiview_mask: None,
            cache: None,
        });

        let terrain_module = shader(
            device,
            "terrain",
            format!("{common}\n{}", include_str!("shaders/terrain.wgsl")),
        );
        let vertex_attrs = wgpu::vertex_attr_array![0 => Uint32, 1 => Uint32];
        let instance_attrs = wgpu::vertex_attr_array![2 => Float32x3, 3 => Sint32x3];
        let terrain_buffers = [
            Some(wgpu::VertexBufferLayout {
                array_stride: size_of::<Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &vertex_attrs,
            }),
            Some(wgpu::VertexBufferLayout {
                array_stride: size_of::<Instance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &instance_attrs,
            }),
        ];
        let terrain_pipeline = |label: &str,
                                fs: &str,
                                cull: Option<wgpu::Face>,
                                depth_write: bool,
                                blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&scene_layout),
                vertex: wgpu::VertexState {
                    module: &terrain_module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &terrain_buffers,
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: cull,
                    ..Default::default()
                },
                depth_stencil: depth(depth_write, wgpu::CompareFunction::Greater),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &terrain_module,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &hdr_target_state(blend),
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let opaque = terrain_pipeline("opaque", "fs", Some(wgpu::Face::Back), true, None);
        let water = terrain_pipeline(
            "water",
            "fs_water",
            None,
            false,
            Some(wgpu::BlendState::ALPHA_BLENDING),
        );

        let far_module = shader(device, "far", format!("{common}\n{}", include_str!("shaders/far.wgsl")));
        let far_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Unorm8x4, 2 => Snorm16x2];
        let far_instance_attrs = wgpu::vertex_attr_array![3 => Float32x3];
        let far = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("far"),
            layout: Some(&scene_layout),
            vertex: wgpu::VertexState {
                module: &far_module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[
                    Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<FarVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &far_attrs,
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<Instance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &far_instance_attrs,
                    }),
                ],
            },
            primitive: Default::default(),
            depth_stencil: depth(true, wgpu::CompareFunction::Greater),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &far_module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &hdr_target_state(None),
            }),
            multiview_mask: None,
            cache: None,
        });
        // The sea: a quad far larger than the view distance, centred under the camera.
        let sea_color = worldgen::materials::build().get(worldgen::materials::WATER).color;
        let sea_vertex = |x: f32, z: f32| FarVertex {
            pos: [x, -0.125, z],
            color: sea_color,
            flags: crate::farview::FLAG_WATER,
            grad: [0, 0],
        };
        const SEA_EXTENT: f32 = 400_000.0;
        let sea = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("sea"),
            contents: bytemuck::cast_slice(&[
                sea_vertex(-SEA_EXTENT, -SEA_EXTENT),
                sea_vertex(-SEA_EXTENT, SEA_EXTENT),
                sea_vertex(SEA_EXTENT, SEA_EXTENT),
                sea_vertex(SEA_EXTENT, -SEA_EXTENT),
            ]),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let tonemap_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tonemap"),
            bind_group_layouts: &[Some(&hdr_layout)],
            immediate_size: 0,
        });
        let tonemap_module = shader(
            device,
            "tonemap",
            format!(
                "const ENCODE_SRGB: bool = {};\n{}",
                !output_format.is_srgb(),
                include_str!("shaders/tonemap.wgsl")
            ),
        );
        let tonemap = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tonemap"),
            layout: Some(&tonemap_layout),
            vertex: wgpu::VertexState {
                module: &tonemap_module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &tonemap_module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: output_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let instance_capacity = 4096;
        let quad_capacity = 16384;
        Self {
            width,
            height,
            globals,
            globals_bind,
            hdr_view,
            hdr_layout,
            hdr_bind,
            depth_view,
            sky,
            opaque,
            water,
            far,
            sea,
            tonemap,
            instances: instance_buffer(device, instance_capacity),
            instance_capacity,
            quad_indices: quad_index_buffer(device, quad_capacity),
            quad_capacity,
            stats: DrawStats::default(),
            start: std::time::Instant::now(),
        }
    }

    pub fn resize(&mut self, gpu: &Gpu, width: u32, height: u32) {
        if width == 0 || height == 0 || (width, height) == (self.width, self.height) {
            return;
        }
        (self.width, self.height) = (width, height);
        (self.hdr_view, self.hdr_bind) = hdr_target(&gpu.device, &self.hdr_layout, width, height);
        self.depth_view = create_depth(&gpu.device, width, height);
    }

    /// Renders the scene and writes the tonemapped image to `target`.
    pub fn render(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        scene: &Scene,
    ) {
        let Scene { camera, env, far, .. } = *scene;
        // Horizontal distance out to which voxel terrain is fully loaded.
        let voxel_radius = scene.terrain.complete_radius();
        let aspect = self.width as f32 / self.height as f32;
        let view_proj = camera.view_proj(aspect);
        let tan = (camera.fov_y * 0.5).tan();
        let fwd = camera.forward();
        let right = camera.right();
        let up = right.cross(fwd);
        let v4 = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
        let globals = Globals {
            view_proj: view_proj.to_cols_array_2d(),
            cam_right: v4(right * tan * aspect, 0.0),
            cam_up: v4(up * tan, 0.0),
            cam_fwd: v4(fwd, 0.0),
            sun_dir: v4(env.sun_dir, self.start.elapsed().as_secs_f32()),
            sun_color: v4(env.sun_color, 0.0),
            sky_zenith: v4(env.sky_zenith, 0.0),
            sky_horizon: v4(env.sky_horizon, 0.0),
            fog: [env.fog_density, voxel_radius, 0.0, 0.0],
            cam_wrapped: {
                let p = camera.pos.rem_euclid(glam::DVec3::splat(4096.0)).as_vec3();
                [p.x, p.y, p.z, 0.0]
            },
        };
        gpu.queue
            .write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));

        // Visible chunks, with their camera-relative offsets computed in f64.
        let frustum = Frustum::new(view_proj);
        let size = CHUNK as f32;
        let mut instances: Vec<Instance> = Vec::new();
        let mut opaque: Vec<(&MeshBuf, u32)> = Vec::new();
        let mut water: Vec<(&MeshBuf, u32, f32)> = Vec::new();
        let mut stats = DrawStats::default();
        let mut max_quads = 0;
        for chunk in scene.terrain.chunks() {
            let origin = chunk.pos * CHUNK;
            let offset = (origin.as_dvec3() - camera.pos).as_vec3();
            if !frustum.intersects_box(offset, offset + Vec3::splat(size)) {
                stats.chunks_culled += 1;
                continue;
            }
            let index = instances.len() as u32;
            instances.push(Instance {
                offset: offset.to_array(),
                origin: origin.to_array(),
            });
            stats.chunks_drawn += 1;
            if let Some(m) = &chunk.opaque {
                opaque.push((m, index));
                stats.quads_drawn += m.quads as u64;
                max_quads = max_quads.max(m.quads);
            }
            if let Some(m) = &chunk.water {
                let center = offset + Vec3::splat(size * 0.5);
                water.push((m, index, center.length_squared()));
                stats.quads_drawn += m.quads as u64;
                max_quads = max_quads.max(m.quads);
            }
        }
        // Far view: one instance per continent (plus one for the sea), tiles culled
        // against the frustum.
        let mut far_tiles: Vec<(&wgpu::Buffer, usize, u32)> = Vec::new();
        let mut far_rivers: Vec<(&wgpu::Buffer, u32, u32)> = Vec::new();
        for cell in far.cells() {
            let offset = cell.offset(camera.pos);
            let index = instances.len() as u32;
            instances.push(Instance {
                offset: offset.to_array(),
                origin: [0; 3],
            });
            for tile in &cell.tiles {
                if frustum.intersects_box(tile.lo + offset, tile.hi + offset) {
                    far_tiles.push((&tile.buffer, cell.stride, index));
                }
            }
            if let Some((buffer, quads)) = &cell.rivers {
                far_rivers.push((buffer, *quads, index));
                max_quads = max_quads.max(*quads);
            }
        }
        let sea_instance = instances.len() as u32;
        instances.push(Instance {
            offset: [0.0, -camera.pos.y as f32, 0.0],
            origin: [0; 3],
        });
        stats.far_tiles = far_tiles.len() as u32;

        // Translucent water blends correctly only when drawn far to near.
        water.sort_unstable_by(|a, b| b.2.total_cmp(&a.2));
        self.stats = stats;

        if instances.len() > self.instance_capacity {
            self.instance_capacity = instances.len().next_power_of_two();
            self.instances = instance_buffer(&gpu.device, self.instance_capacity);
        }
        if max_quads > self.quad_capacity {
            self.quad_capacity = max_quads.next_power_of_two();
            self.quad_indices = quad_index_buffer(&gpu.device, self.quad_capacity);
        }
        gpu.queue
            .write_buffer(&self.instances, 0, bytemuck::cast_slice(&instances));

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.hdr_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.globals_bind, &[]);
            pass.set_pipeline(&self.sky);
            pass.draw(0..3, 0..1);

            // Far view first: land tiles, then river ribbons and the sea over them.
            pass.set_vertex_buffer(1, self.instances.slice(..));
            pass.set_pipeline(&self.far);
            let mut bound_stride = 0;
            let mut grid_count = 0;
            for (buffer, stride, index) in &far_tiles {
                if *stride != bound_stride {
                    let (grid, count) = far.grid(*stride);
                    pass.set_index_buffer(grid.slice(..), wgpu::IndexFormat::Uint16);
                    (bound_stride, grid_count) = (*stride, count);
                }
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw_indexed(0..grid_count, 0, *index..*index + 1);
            }
            pass.set_index_buffer(self.quad_indices.slice(..), wgpu::IndexFormat::Uint32);
            for (buffer, quads, index) in &far_rivers {
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw_indexed(0..quads * 6, 0, *index..*index + 1);
            }
            pass.set_vertex_buffer(0, self.sea.slice(..));
            pass.draw_indexed(0..6, 0, sea_instance..sea_instance + 1);

            if !opaque.is_empty() || !water.is_empty() {
                pass.set_pipeline(&self.opaque);
                for (mesh, index) in &opaque {
                    pass.set_vertex_buffer(0, mesh.buffer.slice(..));
                    pass.draw_indexed(0..mesh.quads * 6, 0, *index..*index + 1);
                }
                pass.set_pipeline(&self.water);
                for (mesh, index, _) in &water {
                    pass.set_vertex_buffer(0, mesh.buffer.slice(..));
                    pass.draw_indexed(0..mesh.quads * 6, 0, *index..*index + 1);
                }
            }
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("tonemap"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.tonemap);
            pass.set_bind_group(0, &self.hdr_bind, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

/// View frustum in camera-relative space. With an infinite reverse-Z projection there
/// is no far plane, so five planes.
struct Frustum {
    planes: [Vec4; 5],
}

impl Frustum {
    fn new(view_proj: Mat4) -> Self {
        let (r0, r1, r2, r3) = (
            view_proj.row(0),
            view_proj.row(1),
            view_proj.row(2),
            view_proj.row(3),
        );
        Self {
            planes: [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r3 - r2],
        }
    }

    fn intersects_box(&self, min: Vec3, max: Vec3) -> bool {
        self.planes.iter().all(|p| {
            // The box corner farthest along the plane normal.
            let v = Vec3::select(p.truncate().cmpge(Vec3::ZERO), max, min);
            p.truncate().dot(v) + p.w >= 0.0
        })
    }
}

/// Renders one frame headless and saves it as a PNG.
pub fn screenshot(
    gpu: &Gpu,
    path: &std::path::Path,
    size: (u32, u32),
    camera: &Camera,
    env: &Environment,
    streamer: &mut Streamer,
    far: &mut FarView,
) -> anyhow::Result<()> {
    let (width, height) = size;
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut renderer = Renderer::new(gpu, format, width, height);
    streamer.wait_idle(gpu, camera.pos);
    far.wait_ready(gpu, camera.pos);

    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("screenshot"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    // Rows in a texture-to-buffer copy must be padded to a multiple of 256 bytes.
    let row = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("screenshot readback"),
        size: (row * height) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    let scene = Scene {
        camera,
        env,
        terrain: streamer,
        far,
    };
    renderer.render(gpu, &mut encoder, &view, &scene);
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit([encoder.finish()]);

    buffer.slice(..).map_async(wgpu::MapMode::Read, |r| {
        r.expect("screenshot buffer map failed");
    });
    gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
    let data = buffer.slice(..).get_mapped_range()?;
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        let start = (y * row) as usize;
        pixels.extend_from_slice(&data[start..start + (width * 4) as usize]);
    }
    drop(data);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    image::save_buffer(path, &pixels, width, height, image::ColorType::Rgba8)?;
    log::info!(
        "screenshot {} ({} chunks drawn, {} quads)",
        path.display(),
        renderer.stats.chunks_drawn,
        renderer.stats.quads_drawn
    );
    Ok(())
}
