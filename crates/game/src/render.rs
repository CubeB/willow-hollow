//! Scene renderer. Each frame: shadow maps from the sun, then sky, far view, voxel
//! terrain and water into an HDR target, then bloom and a tonemap pass to whatever the
//! output is (window or screenshot texture).

use std::ops::Range;

use bytemuck::{Pod, Zeroable};
use glam::{DVec3, Mat4, Vec2, Vec3, Vec4};
use voxel::{CHUNK, mesh::Vertex};
use worldgen::scale::{VOXELS_PER_METRE, m};
use wgpu::util::DeviceExt;

use crate::{
    arena::MeshSpan,
    camera::Camera,
    env::Environment,
    farview::{FarVertex, FarView},
    particles::ParticleInstance,
    gfx::{DEPTH_FORMAT, Gpu, create_depth},
    impostors::{self, BoxInstance, Impostors},
    profile::GpuTimer,
    terrain::{LEVELS, PLANT_REACH, Streamer},
};

const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const SHADOW_SIZE: u32 = 2048;
const CASCADES: usize = 3;
/// View-depth at which each shadow cascade ends.
const CASCADE_ENDS: [f32; CASCADES] = [m(32.0), m(128.0), m(512.0)];
/// Height of the cloud layer above sea level.
const CLOUD_HEIGHT: f32 = m(240.0);
/// How many of the shadow cascades, nearest first, include small plants.
const PLANT_SHADOW_CASCADES: usize = 2;
/// A cached shadow map is redrawn early once the view has turned this far from it
/// (the cosine of about ten degrees).
const CASCADE_SWING: f32 = 0.985;
/// Or once the camera has travelled this share of the cascade's depth.
const CASCADE_DRIFT: f64 = 0.06;
/// Sun heights above which the far cascade can do with only the tops of things.
const HIGH_SUN: f32 = 0.4;

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
    mist: [f32; 4],
    cascades: [f32; 4],
    wind: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Shadows {
    light_vp: [[[f32; 4]; 4]; CASCADES],
    texel: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CascadeUniform {
    light_vp: [[f32; 4]; 4],
}

/// What one cascade's shadow map was last drawn from.
#[derive(Clone, Copy)]
struct CascadeView {
    /// Camera-relative, for the camera as it was then.
    light_vp: Mat4,
    texel: f32,
    camera: DVec3,
    fwd: Vec3,
}

/// Whether cascade `i` takes its turn this frame. The far ones cover so much that they
/// are redrawn in rotation, never two heavy ones in the same frame.
fn cascade_due(i: usize, frame: u64) -> bool {
    match i {
        0 => true,
        1 => frame % 2 == 1,
        _ => frame.is_multiple_of(4),
    }
}

/// Per-chunk (or per-continent) instance data.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Instance {
    /// Origin relative to the camera.
    offset: [f32; 3],
    /// Origin in world voxels (for stable per-voxel hashing).
    origin: [i32; 3],
    /// World voxels per cell of its mesh.
    scale: f32,
    /// The range of horizontal distances this chunk's ring is drawn in.
    band: [f32; 2],
}

/// The camera's position for the far trees' whole-voxel placement.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ImpostorCamera {
    voxel: [i32; 4],
    frac: [f32; 4],
}

/// Most cubes of solid moving models drawn in one frame.
const PROP_CAPACITY: usize = 1024;

/// Everything one frame draws.
pub struct Scene<'a> {
    pub camera: &'a Camera,
    pub env: &'a Environment,
    pub terrain: &'a Streamer,
    pub far: &'a FarView,
    pub impostors: &'a Impostors,
    pub particles: &'a [ParticleInstance],
    /// Solid moving models (boats, the glider), drawn with the opaque scene.
    pub props: &'a [ParticleInstance],
    pub wind: Vec3,
}

#[derive(Default, Clone, Copy)]
pub struct DrawStats {
    pub chunks_drawn: u32,
    pub chunks_culled: u32,
    pub quads_drawn: u64,
    pub far_tiles: u32,
    pub impostor_boxes: u32,
    pub shadow_draws: [u32; CASCADES],
}

/// A chunk draw as the GPU reads it from the indirect buffer: index count, instance
/// count, first index, base vertex, first instance.
type ChunkDraw = [u32; 5];

fn chunk_draw(mesh: &MeshSpan, instance: u32) -> ChunkDraw {
    [mesh.quads * 6, 1, 0, mesh.first_vertex(), instance]
}

/// Adds draws for the face directions marked in `show`, merging neighbouring directions
/// into one draw.
fn push_faces(list: &mut Vec<ChunkDraw>, mesh: &MeshSpan, faces: &[u32], show: [bool; 6], instance: u32) {
    let mut f = 0;
    while f < 6 {
        if !show[f] {
            f += 1;
            continue;
        }
        let first = faces[f];
        while f < 6 && show[f] {
            f += 1;
        }
        let quads = faces[f] - first;
        if quads > 0 {
            list.push([quads * 6, 1, 0, mesh.first_vertex() + first * 4, instance]);
        }
    }
}

/// Issues part of this frame's chunk draws. `indirect` is None on GPUs that cannot set
/// the first instance from an indirect buffer.
fn draw_chunks(pass: &mut wgpu::RenderPass, indirect: Option<&wgpu::Buffer>, draws: &[ChunkDraw], list: Range<usize>) {
    if list.is_empty() {
        return;
    }
    match indirect {
        Some(buffer) => pass.multi_draw_indexed_indirect(buffer, (list.start * size_of::<ChunkDraw>()) as u64, list.len() as u32),
        None => {
            for [indices, count, _, base, instance] in &draws[list] {
                pass.draw_indexed(0..*indices, *base as i32, *instance..*instance + *count);
            }
        }
    }
}

fn indirect_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("chunk draws"),
        size: (capacity * size_of::<ChunkDraw>()) as u64,
        usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// A texture used as a render target and later sampled.
struct Target {
    view: wgpu::TextureView,
}

fn make_target(device: &wgpu::Device, label: &str, w: u32, h: u32, format: wgpu::TextureFormat) -> Target {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: w.max(1),
            height: h.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    Target {
        view: tex.create_view(&Default::default()),
    }
}

pub struct Renderer {
    width: u32,
    height: u32,
    globals: wgpu::Buffer,
    shadows: wgpu::Buffer,
    scene_bind: wgpu::BindGroup,
    cascade_binds: Vec<(wgpu::Buffer, wgpu::BindGroup)>,
    shadow_views: Vec<wgpu::TextureView>,
    hdr: Target,
    depth: Target,
    bloom_a: Target,
    bloom_b: Target,
    image_layout: wgpu::BindGroupLayout,
    depth_layout: wgpu::BindGroupLayout,
    depth_bind: wgpu::BindGroup,
    linear_sampler: wgpu::Sampler,
    sky: wgpu::RenderPipeline,
    opaque: wgpu::RenderPipeline,
    opaque_inside: wgpu::RenderPipeline,
    water: wgpu::RenderPipeline,
    far: wgpu::RenderPipeline,
    impostor: wgpu::RenderPipeline,
    impostor_cube: wgpu::Buffer,
    impostor_camera: wgpu::Buffer,
    impostor_bind: wgpu::BindGroup,
    shadow: wgpu::RenderPipeline,
    bright: wgpu::RenderPipeline,
    blur_h: wgpu::RenderPipeline,
    blur_v: wgpu::RenderPipeline,
    tonemap: wgpu::RenderPipeline,
    particles: wgpu::RenderPipeline,
    props: wgpu::RenderPipeline,
    prop_instances: wgpu::Buffer,
    cube: wgpu::Buffer,
    particle_instances: wgpu::Buffer,
    particle_capacity: usize,
    sea: wgpu::Buffer,
    instances: wgpu::Buffer,
    instance_capacity: usize,
    quad_indices: wgpu::Buffer,
    quad_capacity: u32,
    indirect: wgpu::Buffer,
    indirect_capacity: usize,
    multi_draw: bool,
    cascade_views: [Option<CascadeView>; CASCADES],
    frame: u64,
    pub timer: Option<GpuTimer>,
    pub stats: DrawStats,
    start: std::time::Instant,
}

fn shader(device: &wgpu::Device, label: &str, source: String) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
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
        label: Some("instances"),
        size: (capacity * size_of::<Instance>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn texture_entry(binding: u32, sample_type: wgpu::TextureSampleType, dim: wgpu::TextureViewDimension) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: dim,
            multisampled: false,
        },
        count: None,
    }
}

fn fullscreen_triangle() -> wgpu::PrimitiveState {
    Default::default()
}

impl Renderer {
    /// `output_format` is the format of the texture the final image is written to.
    pub fn new(gpu: &Gpu, output_format: wgpu::TextureFormat, width: u32, height: u32) -> Self {
        let device = &gpu.device;
        // Shaders work in voxels but know how big one is.
        let common = format!("const VOXELS_PER_METRE: f32 = {VOXELS_PER_METRE:?};\n{}", include_str!("shaders/common.wgsl"));

        // --- Uniforms and the scene bind group (globals, shadows) -----------------
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shadows = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadows"),
            size: size_of::<Shadows>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shadow_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow maps"),
            size: wgpu::Extent3d {
                width: SHADOW_SIZE,
                height: SHADOW_SIZE,
                depth_or_array_layers: CASCADES as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let shadow_array_view = shadow_tex.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let shadow_views = (0..CASCADES as u32)
            .map(|layer| {
                shadow_tex.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: layer,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let scene_layout_bg = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT),
                uniform_entry(1, wgpu::ShaderStages::FRAGMENT),
                texture_entry(2, wgpu::TextureSampleType::Depth, wgpu::TextureViewDimension::D2Array),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ],
        });
        let scene_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene"),
            layout: &scene_layout_bg,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: shadows.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&shadow_array_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&shadow_sampler),
                },
            ],
        });
        let depth_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene depth"),
            entries: &[texture_entry(0, wgpu::TextureSampleType::Depth, wgpu::TextureViewDimension::D2)],
        });
        let scene_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene"),
            bind_group_layouts: &[Some(&scene_layout_bg)],
            immediate_size: 0,
        });
        let water_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("water"),
            bind_group_layouts: &[Some(&scene_layout_bg), Some(&depth_layout)],
            immediate_size: 0,
        });

        // --- Shadow cascades -----------------------------------------------------
        let cascade_layout_bg = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cascade"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX)],
        });
        let cascade_binds = (0..CASCADES)
            .map(|_| {
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("cascade"),
                    size: size_of::<CascadeUniform>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("cascade"),
                    layout: &cascade_layout_bg,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    }],
                });
                (buffer, bind)
            })
            .collect();
        let shadow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow"),
            bind_group_layouts: &[Some(&cascade_layout_bg)],
            immediate_size: 0,
        });

        // --- Targets -------------------------------------------------------------
        let hdr = make_target(device, "hdr", width, height, HDR_FORMAT);
        let depth = Target {
            view: create_depth(device, width, height),
        };
        let bloom_a = make_target(device, "bloom a", width / 4, height / 4, HDR_FORMAT);
        let bloom_b = make_target(device, "bloom b", width / 4, height / 4, HDR_FORMAT);
        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let image_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("image"),
            entries: &[
                texture_entry(0, wgpu::TextureSampleType::Float { filterable: true }, wgpu::TextureViewDimension::D2),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                texture_entry(2, wgpu::TextureSampleType::Float { filterable: true }, wgpu::TextureViewDimension::D2),
            ],
        });
        let depth_bind = Self::depth_bind_group(device, &depth_layout, &depth.view);

        // --- Pipelines -----------------------------------------------------------
        let depth_state = |write: bool, compare: wgpu::CompareFunction| {
            Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(write),
                depth_compare: Some(compare),
                stencil: Default::default(),
                bias: Default::default(),
            })
        };
        let hdr_targets = |blend: Option<wgpu::BlendState>| {
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
            primitive: fullscreen_triangle(),
            depth_stencil: depth_state(false, wgpu::CompareFunction::Always),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &sky_module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &hdr_targets(None),
            }),
            multiview_mask: None,
            cache: None,
        });

        let terrain_module = shader(device, "terrain", format!("{common}\n{}", include_str!("shaders/terrain.wgsl")));
        let vertex_attrs = wgpu::vertex_attr_array![0 => Uint32, 1 => Uint32];
        let instance_attrs = wgpu::vertex_attr_array![2 => Float32x3, 3 => Sint32x3, 4 => Float32, 5 => Float32x2];
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
        let opaque = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("opaque"),
            layout: Some(&scene_layout),
            vertex: wgpu::VertexState {
                module: &terrain_module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &terrain_buffers,
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: depth_state(true, wgpu::CompareFunction::Greater),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &terrain_module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &hdr_targets(None),
            }),
            multiview_mask: None,
            cache: None,
        });
        let opaque_inside = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("opaque inside"),
            layout: Some(&scene_layout),
            vertex: wgpu::VertexState {
                module: &terrain_module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &terrain_buffers,
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: depth_state(true, wgpu::CompareFunction::Greater),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &terrain_module,
                entry_point: Some("fs_inside"),
                compilation_options: Default::default(),
                targets: &hdr_targets(None),
            }),
            multiview_mask: None,
            cache: None,
        });
        let water = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("water"),
            layout: Some(&water_layout),
            vertex: wgpu::VertexState {
                module: &terrain_module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &terrain_buffers,
            },
            primitive: Default::default(),
            depth_stencil: depth_state(false, wgpu::CompareFunction::Greater),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &terrain_module,
                entry_point: Some("fs_water"),
                compilation_options: Default::default(),
                targets: &hdr_targets(Some(wgpu::BlendState::ALPHA_BLENDING)),
            }),
            multiview_mask: None,
            cache: None,
        });

        let shadow_module = shader(device, "shadow", include_str!("shaders/shadow.wgsl").to_string());
        let shadow = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow"),
            layout: Some(&shadow_layout),
            vertex: wgpu::VertexState {
                module: &shadow_module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &terrain_buffers,
            },
            // Both faces: thin leaf shells must cast from either side.
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
            }),
            multisample: Default::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });

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
            depth_stencil: depth_state(true, wgpu::CompareFunction::Greater),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &far_module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &hdr_targets(None),
            }),
            multiview_mask: None,
            cache: None,
        });
        // Far trees: instanced boxes, the box itself as four corners per side.
        let impostor_camera = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("impostor camera"),
            size: size_of::<ImpostorCamera>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let impostor_layout_bg = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("impostor camera"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX)],
        });
        let impostor_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("impostor camera"),
            layout: &impostor_layout_bg,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: impostor_camera.as_entire_binding(),
            }],
        });
        let impostor_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("impostor"),
            bind_group_layouts: &[Some(&scene_layout_bg), Some(&impostor_layout_bg)],
            immediate_size: 0,
        });
        let impostor_module = shader(device, "impostor", format!("{common}\n{}", include_str!("shaders/impostor.wgsl")));
        let impostor_box_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
        let impostor_instance_attrs = wgpu::vertex_attr_array![2 => Sint32x3, 3 => Uint8x4, 4 => Unorm8x4];
        let impostor = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("impostor"),
            layout: Some(&impostor_layout),
            vertex: wgpu::VertexState {
                module: &impostor_module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[
                    Some(wgpu::VertexBufferLayout {
                        array_stride: 24,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &impostor_box_attrs,
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<BoxInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &impostor_instance_attrs,
                    }),
                ],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: depth_state(true, wgpu::CompareFunction::Greater),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &impostor_module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &hdr_targets(None),
            }),
            multiview_mask: None,
            cache: None,
        });
        let sides: [(Vec3, Vec3, Vec3); 6] = [
            (Vec3::X, Vec3::Y, Vec3::Z),
            (Vec3::NEG_X, Vec3::Z, Vec3::Y),
            (Vec3::Y, Vec3::Z, Vec3::X),
            (Vec3::NEG_Y, Vec3::X, Vec3::Z),
            (Vec3::Z, Vec3::X, Vec3::Y),
            (Vec3::NEG_Z, Vec3::Y, Vec3::X),
        ];
        let mut box_corners: Vec<[f32; 6]> = Vec::with_capacity(24);
        for (n, u, v) in sides {
            for (su, sv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let p = Vec3::splat(0.5) + (n + u * su + v * sv) * 0.5;
                box_corners.push([p.x, p.y, p.z, n.x, n.y, n.z]);
            }
        }
        let impostor_cube = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("impostor box"),
            contents: bytemuck::cast_slice(&box_corners),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let sea_color = worldgen::materials::build().get(worldgen::materials::WATER).color;
        let sea_vertex = |x: f32, z: f32| FarVertex {
            pos: [x, -0.125, z],
            color: sea_color,
            flags: crate::farview::FLAG_WATER,
            grad: [0, 0],
        };
        const SEA_EXTENT: f32 = m(320_000.0);
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

        // Particles: a unit cube with normals, instanced.
        let particle_module = shader(device, "particles", format!("{common}\n{}", include_str!("shaders/particles.wgsl")));
        let mut cube_verts: Vec<[f32; 6]> = Vec::new();
        for (n, u, v) in [
            ([1.0f32, 0.0, 0.0], [0.0f32, 1.0, 0.0], [0.0f32, 0.0, 1.0]),
            ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
            ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
            ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
            ([0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]),
        ] {
            let (n, u, v) = (Vec3::from(n), Vec3::from(u), Vec3::from(v));
            let corner = |su: f32, sv: f32| {
                let p = n * 0.5 + u * su * 0.5 + v * sv * 0.5;
                [p.x, p.y, p.z, n.x, n.y, n.z]
            };
            let (a, b, c, d) = (corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0));
            cube_verts.extend_from_slice(&[a, b, c, a, c, d]);
        }
        let cube = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("cube"),
            contents: bytemuck::cast_slice(&cube_verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let cube_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
        let particle_attrs = wgpu::vertex_attr_array![2 => Float32x3, 3 => Float32, 4 => Float32x3, 5 => Float32, 6 => Float32x4];
        let particles = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("particles"),
            layout: Some(&scene_layout),
            vertex: wgpu::VertexState {
                module: &particle_module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[
                    Some(wgpu::VertexBufferLayout {
                        array_stride: 24,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &cube_attrs,
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<ParticleInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &particle_attrs,
                    }),
                ],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: depth_state(false, wgpu::CompareFunction::Greater),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &particle_module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &hdr_targets(Some(wgpu::BlendState::ALPHA_BLENDING)),
            }),
            multiview_mask: None,
            cache: None,
        });
        let props = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("props"),
            layout: Some(&scene_layout),
            vertex: wgpu::VertexState {
                module: &particle_module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[
                    Some(wgpu::VertexBufferLayout {
                        array_stride: 24,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &cube_attrs,
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<ParticleInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &particle_attrs,
                    }),
                ],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: depth_state(true, wgpu::CompareFunction::Greater),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &particle_module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &hdr_targets(None),
            }),
            multiview_mask: None,
            cache: None,
        });
        let prop_instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("prop instances"),
            size: (PROP_CAPACITY * size_of::<ParticleInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let particle_capacity = 8192;
        let particle_instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particle instances"),
            size: (particle_capacity * size_of::<ParticleInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Post-processing: one layout, several entry points.
        let post_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("post"),
            bind_group_layouts: &[Some(&image_layout)],
            immediate_size: 0,
        });
        let post = |label: &str, entry: &str, defines: &str, format: wgpu::TextureFormat| {
            let module = shader(
                device,
                label,
                format!("{defines}\n{}", include_str!("shaders/post.wgsl")),
            );
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&post_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: fullscreen_triangle(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let no_dir = "const DIR: vec2<f32> = vec2<f32>(0.0, 0.0);\nconst ENCODE_SRGB: bool = false;";
        let bright = post("bright", "fs_bright", no_dir, HDR_FORMAT);
        let blur_h = post(
            "blur h",
            "fs_blur",
            "const DIR: vec2<f32> = vec2<f32>(1.0, 0.0);\nconst ENCODE_SRGB: bool = false;",
            HDR_FORMAT,
        );
        let blur_v = post(
            "blur v",
            "fs_blur",
            "const DIR: vec2<f32> = vec2<f32>(0.0, 1.0);\nconst ENCODE_SRGB: bool = false;",
            HDR_FORMAT,
        );
        let tonemap = post(
            "tonemap",
            "fs_tonemap",
            &format!(
                "const DIR: vec2<f32> = vec2<f32>(0.0, 0.0);\nconst ENCODE_SRGB: bool = {};",
                !output_format.is_srgb()
            ),
            output_format,
        );

        let instance_capacity = 4096;
        let quad_capacity = 16384;
        Self {
            width,
            height,
            globals,
            shadows,
            scene_bind,
            cascade_binds,
            shadow_views,
            hdr,
            depth,
            bloom_a,
            bloom_b,
            image_layout,
            depth_layout,
            depth_bind,
            linear_sampler,
            sky,
            opaque,
            opaque_inside,
            water,
            far,
            impostor,
            impostor_cube,
            impostor_camera,
            impostor_bind,
            shadow,
            bright,
            blur_h,
            blur_v,
            tonemap,
            particles,
            props,
            prop_instances,
            cube,
            particle_instances,
            particle_capacity,
            sea,
            instances: instance_buffer(device, instance_capacity),
            instance_capacity,
            quad_indices: quad_index_buffer(device, quad_capacity),
            quad_capacity,
            indirect: indirect_buffer(device, 4096),
            indirect_capacity: 4096,
            multi_draw: device.features().contains(wgpu::Features::INDIRECT_FIRST_INSTANCE),
            cascade_views: [None; CASCADES],
            frame: 0,
            timer: GpuTimer::new(gpu),
            stats: DrawStats::default(),
            start: std::time::Instant::now(),
        }
    }

    fn depth_bind_group(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, view: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene depth"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            }],
        })
    }

    fn image_bind_group(&self, device: &wgpu::Device, src: &wgpu::TextureView, extra: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("image"),
            layout: &self.image_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(src),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.linear_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(extra),
                },
            ],
        })
    }

    pub fn resize(&mut self, gpu: &Gpu, width: u32, height: u32) {
        if width == 0 || height == 0 || (width, height) == (self.width, self.height) {
            return;
        }
        let device = &gpu.device;
        (self.width, self.height) = (width, height);
        self.hdr = make_target(device, "hdr", width, height, HDR_FORMAT);
        self.depth = Target {
            view: create_depth(device, width, height),
        };
        self.depth_bind = Self::depth_bind_group(device, &self.depth_layout, &self.depth.view);
        self.bloom_a = make_target(device, "bloom a", width / 4, height / 4, HDR_FORMAT);
        self.bloom_b = make_target(device, "bloom b", width / 4, height / 4, HDR_FORMAT);
    }

    /// Light view-projection for one cascade, in camera-relative space, and its texel
    /// size in voxels.
    fn cascade_matrix(camera: &Camera, aspect: f32, sun: Vec3, near: f32, far: f32) -> (Mat4, f32) {
        // Bounding sphere of this slice of the view frustum.
        let fwd = camera.forward();
        let right = camera.right();
        let up = right.cross(fwd);
        let tan_y = (camera.fov_y * 0.5).tan();
        let tan_x = tan_y * aspect;
        let corner = |d: f32, sx: f32, sy: f32| fwd * d + right * (d * tan_x * sx) + up * (d * tan_y * sy);
        let corners = [
            corner(near, -1.0, -1.0),
            corner(near, 1.0, 1.0),
            corner(far, -1.0, -1.0),
            corner(far, 1.0, 1.0),
            corner(far, 1.0, -1.0),
            corner(far, -1.0, 1.0),
        ];
        let center = fwd * ((near + far) * 0.5);
        let radius = corners.iter().map(|c| (*c - center).length()).fold(0.0, f32::max) * 1.05;
        let texel = 2.0 * radius / SHADOW_SIZE as f32;

        // Light view looking along the sun direction; snap the centre to texels so
        // the map does not shimmer as the camera moves.
        let light_up = if sun.y.abs() > 0.95 { Vec3::Z } else { Vec3::Y };
        let s = sun.cross(light_up).normalize();
        let u = s.cross(sun);
        let snapped = s * ((center.dot(s) / texel).floor() * texel) + u * ((center.dot(u) / texel).floor() * texel)
            + sun * center.dot(sun);
        // Casters well outside the slice (tall trees, hills toward the sun) still count.
        let reach = radius + m(320.0);
        let eye = snapped + sun * reach;
        let view = Mat4::from_cols(
            Vec4::new(s.x, u.x, sun.x, 0.0),
            Vec4::new(s.y, u.y, sun.y, 0.0),
            Vec4::new(s.z, u.z, sun.z, 0.0),
            Vec4::new(-eye.dot(s), -eye.dot(u), -eye.dot(sun), 1.0),
        );
        // Orthographic, depth 0..1 over [0, 2 * reach] along -z (toward the ground).
        let depth_range = reach * 2.0;
        let proj = Mat4::from_cols(
            Vec4::new(1.0 / radius, 0.0, 0.0, 0.0),
            Vec4::new(0.0, 1.0 / radius, 0.0, 0.0),
            Vec4::new(0.0, 0.0, -1.0 / depth_range, 0.0),
            Vec4::new(0.0, 0.0, 0.0, 1.0),
        );
        (proj * view, texel)
    }

    /// Renders the scene and writes the tonemapped image to `target`.
    pub fn render(&mut self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, scene: &Scene) {
        let Scene { camera, env, far, .. } = *scene;
        let device = &gpu.device;
        // Horizontal distances out to which each ring of voxel terrain is fully loaded.
        let bands = scene.terrain.bands();
        let voxel_radius = bands[LEVELS - 1];
        let aspect = self.width as f32 / self.height as f32;
        let view_proj = camera.view_proj(aspect);
        let tan = (camera.fov_y * 0.5).tan();
        let fwd = camera.forward();
        let right = camera.right();
        let up = right.cross(fwd);
        let time = self.start.elapsed().as_secs_f32();
        let v4 = |v: Vec3, w: f32| [v.x, v.y, v.z, w];

        // Shadow cascades.
        let mut light_vp = [[[0.0; 4]; 4]; CASCADES];
        let mut light = Vec::with_capacity(CASCADES);
        let mut texels = [0.0; 4];
        let mut fresh = [false; CASCADES];
        let mut near = 0.1;
        let frame = self.frame;
        self.frame += 1;
        for (i, &far_end) in CASCADE_ENDS.iter().enumerate() {
            let (m, texel) = Self::cascade_matrix(camera, aspect, env.sun_dir, near, far_end);
            light.push(Frustum::new(m));
            near = far_end;
            fresh[i] = self.cascade_views[i].is_none_or(|v| {
                cascade_due(i, frame) || v.fwd.dot(fwd) < CASCADE_SWING || v.camera.distance(camera.pos) > far_end as f64 * CASCADE_DRIFT
            });
            if fresh[i] {
                self.cascade_views[i] = Some(CascadeView {
                    light_vp: m,
                    texel,
                    camera: camera.pos,
                    fwd,
                });
                gpu.queue.write_buffer(
                    &self.cascade_binds[i].0,
                    0,
                    bytemuck::bytes_of(&CascadeUniform { light_vp: m.to_cols_array_2d() }),
                );
            }
            // A map drawn earlier is still right: it is fixed in the world, so only the
            // camera's movement since then needs allowing for.
            let view = self.cascade_views[i].expect("just set");
            let moved = (camera.pos - view.camera).as_vec3();
            light_vp[i] = (view.light_vp * Mat4::from_translation(moved)).to_cols_array_2d();
            texels[i] = view.texel;
        }
        gpu.queue
            .write_buffer(&self.shadows, 0, bytemuck::bytes_of(&Shadows { light_vp, texel: texels }));

        let cam_wrapped = camera.pos.rem_euclid(glam::DVec3::splat(4096.0)).as_vec3();
        let globals = Globals {
            view_proj: view_proj.to_cols_array_2d(),
            cam_right: v4(right * tan * aspect, 0.0),
            cam_up: v4(up * tan, 0.0),
            cam_fwd: v4(fwd, 0.0),
            sun_dir: v4(env.sun_dir, time),
            sun_color: v4(env.sun_color, env.daylight as u8 as f32),
            sky_zenith: v4(env.sky_zenith, 0.0),
            sky_horizon: v4(env.sky_horizon, 0.0),
            fog: [env.fog_density, voxel_radius, env.clouds, time * m(4.8)],
            // w: how far the tree models reach, for the far view's forest height.
            cam_wrapped: [cam_wrapped.x, camera.pos.y as f32, cam_wrapped.z, impostors::REACH],
            mist: [env.mist, m(28.0), m(-12.0), 0.0],
            cascades: [
                CASCADE_ENDS[0],
                CASCADE_ENDS[1],
                CASCADE_ENDS[2],
                CLOUD_HEIGHT - camera.pos.y as f32,
            ],
            wind: v4(scene.wind, 0.0),
        };
        gpu.queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));

        // Visible chunks, with their camera-relative offsets computed in f64.
        let frustum = Frustum::new(view_proj);
        // Only faces turned toward the sun can be the nearest thing it sees.
        let toward_sun = [
            env.sun_dir.x > -0.001,
            env.sun_dir.x < 0.001,
            env.sun_dir.y > -0.001,
            env.sun_dir.y < 0.001,
            env.sun_dir.z > -0.001,
            env.sun_dir.z < 0.001,
        ];
        // The far cascade is coarser than a voxel, so the tops of things are enough for
        // it until the sun gets low and the sides start to catch the light.
        let far_casters = if env.sun_dir.y > HIGH_SUN { [false, false, true, false, false, false] } else { toward_sun };
        // How far a shadow can reach across the ground, for finding what can cast into view.
        let sweep = -env.sun_dir * (m(120.0) / env.sun_dir.y.clamp(0.15, 1.0));
        let chunk_size = CHUNK as f32;
        let mut instances: Vec<Instance> = Vec::new();
        let mut opaque: Vec<ChunkDraw> = Vec::new();
        let mut opaque_edge: Vec<ChunkDraw> = Vec::new();
        let mut casters: [Vec<ChunkDraw>; CASCADES] = Default::default();
        let mut water: Vec<(ChunkDraw, f32)> = Vec::new();
        let mut stats = DrawStats::default();
        let mut max_quads = 0;
        for chunk in scene.terrain.chunks() {
            let scale = 1 << chunk.level;
            let band = [if chunk.level == 0 { 0.0 } else { bands[chunk.level as usize - 1] }, bands[chunk.level as usize]];
            let size = chunk_size * scale as f32;
            let origin = chunk.pos * CHUNK * scale;
            let offset = (origin.as_dvec3() - camera.pos).as_vec3();
            let far_corner = offset + Vec3::splat(size);
            // How far across the ground the chunk spans from the camera.
            let nearest = Vec2::new(offset.x.max(-far_corner.x).max(0.0), offset.z.max(-far_corner.z).max(0.0)).length();
            let reach = Vec2::new(offset.x.abs().max(far_corner.x.abs()), offset.z.abs().max(far_corner.z.abs())).length();
            // A ring draws only between its two circles.
            if band[0] >= band[1] || reach < band[0] || nearest >= band[1] {
                stats.chunks_culled += 1;
                continue;
            }
            let visible = frustum.intersects_box(offset, far_corner);
            let center = offset + Vec3::splat(size * 0.5);
            // Each cascade only draws what falls inside its own box of light, and only
            // if its shadow could land somewhere the camera can see.
            let reaches_view = env.daylight && frustum.intersects_swept_box(offset, far_corner, sweep);
            let casts: [bool; CASCADES] = std::array::from_fn(|i| fresh[i] && reaches_view && light[i].intersects_box(offset, far_corner));
            // Directions whose faces could be seen from the camera. Foliage sways, so
            // leave a little slack.
            let slack = m(1.0);
            let seen = [
                offset.x < slack,
                far_corner.x > -slack,
                offset.y < slack,
                far_corner.y > -slack,
                offset.z < slack,
                far_corner.z > -slack,
            ];
            if !visible && !casts.contains(&true) {
                stats.chunks_culled += 1;
                continue;
            }
            let index = instances.len() as u32;
            instances.push(Instance {
                offset: offset.to_array(),
                origin: origin.to_array(),
                scale: scale as f32,
                band,
            });
            if let Some(mesh) = &chunk.opaque {
                max_quads = max_quads.max(mesh.quads);
                let (terrain, plants) = (&chunk.faces[..=6], &chunk.faces[6..]);
                // Plants are too small to matter from further away.
                let near = center.length() < PLANT_REACH;
                for (i, (list, _)) in casters.iter_mut().zip(casts).enumerate().filter(|(_, (_, casts))| *casts) {
                    push_faces(list, mesh, terrain, if i == CASCADES - 1 { far_casters } else { toward_sun }, index);
                    if near && i < PLANT_SHADOW_CASCADES {
                        push_faces(list, mesh, plants, toward_sun, index);
                    }
                }
                if visible {
                    // Chunks entirely inside their ring never need trimming to it.
                    let inside = reach + m(1.0) < band[1] && nearest >= band[0];
                    let list = if inside { &mut opaque } else { &mut opaque_edge };
                    push_faces(list, mesh, terrain, seen, index);
                    if near {
                        push_faces(list, mesh, plants, seen, index);
                    }
                }
            }
            if visible {
                stats.chunks_drawn += 1;
                if let Some(mesh) = &chunk.water {
                    water.push((chunk_draw(mesh, index), center.length_squared()));
                    stats.quads_drawn += mesh.quads as u64;
                    max_quads = max_quads.max(mesh.quads);
                }
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
                scale: 1.0,
                band: [0.0; 2],
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
            scale: 1.0,
            band: [0.0; 2],
        });
        stats.far_tiles = far_tiles.len() as u32;

        // Far trees, a square of them at a time.
        let mut impostor_draws: Vec<ChunkDraw> = Vec::new();
        for tile in scene.impostors.tiles() {
            let lo = (tile.lo.as_dvec3() - camera.pos).as_vec3();
            let hi = (tile.hi.as_dvec3() - camera.pos).as_vec3();
            let reach = Vec2::new(lo.x.abs().max(hi.x.abs()), lo.z.abs().max(hi.z.abs())).length();
            // Squares wholly inside the voxel terrain are left to it.
            if reach + m(40.0) >= voxel_radius && frustum.intersects_box(lo, hi) {
                impostor_draws.push([36, tile.span.quads, 0, 0, tile.span.first()]);
                stats.impostor_boxes += tile.span.quads;
            }
        }

        // Translucent water blends correctly only when drawn far to near.
        water.sort_unstable_by(|a, b| b.1.total_cmp(&a.1));
        stats.shadow_draws = std::array::from_fn(|i| casters[i].len() as u32);
        stats.quads_drawn += opaque.iter().chain(&opaque_edge).map(|d| d[0] as u64 / 6).sum::<u64>();
        // Nearest first, so that hidden surfaces are rejected instead of shaded.
        let distance_of = |d: &ChunkDraw| {
            let [x, y, z] = instances[d[4] as usize].offset;
            (Vec3::new(x, y, z) + Vec3::splat(chunk_size * 0.5)).length_squared()
        };
        opaque.sort_by_cached_key(|d| distance_of(d) as u32);
        self.stats = stats;

        // All of this frame's chunk draws, in one buffer the passes take slices of.
        let mut draws: Vec<ChunkDraw> = Vec::with_capacity(opaque.len() + opaque_edge.len() + water.len() + casters.iter().map(Vec::len).sum::<usize>());
        let mut append = |list: &mut dyn Iterator<Item = ChunkDraw>| {
            let start = draws.len();
            draws.extend(list);
            start..draws.len()
        };
        let shadow_lists: [Range<usize>; CASCADES] = std::array::from_fn(|i| append(&mut casters[i].iter().copied()));
        let opaque_list = append(&mut opaque.iter().copied());
        let edge_list = append(&mut opaque_edge.iter().copied());
        let water_list = append(&mut water.iter().map(|w| w.0));
        let impostor_list = append(&mut impostor_draws.iter().copied());
        if draws.len() > self.indirect_capacity {
            self.indirect_capacity = draws.len().next_power_of_two();
            self.indirect = indirect_buffer(device, self.indirect_capacity);
        }
        gpu.queue.write_buffer(&self.indirect, 0, bytemuck::cast_slice(&draws));
        let indirect = self.multi_draw.then_some(&self.indirect);
        let chunk_vertices = scene.terrain.vertices();

        if instances.len() > self.instance_capacity {
            self.instance_capacity = instances.len().next_power_of_two();
            self.instances = instance_buffer(device, self.instance_capacity);
        }
        if max_quads > self.quad_capacity {
            self.quad_capacity = max_quads.next_power_of_two();
            self.quad_indices = quad_index_buffer(device, self.quad_capacity);
        }
        gpu.queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&instances));
        let whole = camera.pos.floor();
        let (voxel, frac) = (whole.as_ivec3(), (camera.pos - whole).as_vec3());
        gpu.queue.write_buffer(
            &self.impostor_camera,
            0,
            bytemuck::bytes_of(&ImpostorCamera {
                voxel: [voxel.x, voxel.y, voxel.z, 0],
                frac: [frac.x, frac.y, frac.z, 0.0],
            }),
        );

        if let Some(t) = &self.timer {
            t.mark(encoder, 0);
        }

        // --- Shadow passes -------------------------------------------------------
        for (i, view) in self.shadow_views.iter().enumerate() {
            if !fresh[i] {
                if let Some(t) = &self.timer {
                    t.mark(encoder, i as u32 + 1);
                }
                continue;
            }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if !shadow_lists[i].is_empty() {
                pass.set_pipeline(&self.shadow);
                pass.set_bind_group(0, &self.cascade_binds[i].1, &[]);
                pass.set_index_buffer(self.quad_indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.set_vertex_buffer(0, chunk_vertices.slice(..));
                pass.set_vertex_buffer(1, self.instances.slice(..));
                draw_chunks(&mut pass, indirect, &draws, shadow_lists[i].clone());
            }
            drop(pass);
            if let Some(t) = &self.timer {
                t.mark(encoder, i as u32 + 1);
            }
        }

        // --- Opaque scene --------------------------------------------------------
        let prop_count = scene.props.len().min(PROP_CAPACITY);
        if prop_count > 0 {
            gpu.queue.write_buffer(&self.prop_instances, 0, bytemuck::cast_slice(&scene.props[..prop_count]));
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.hdr.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth.view,
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
            pass.set_bind_group(0, &self.scene_bind, &[]);
            pass.set_pipeline(&self.sky);
            pass.draw(0..3, 0..1);
            if let Some(t) = &self.timer {
                t.mark_in_pass(&mut pass, 4);
            }

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

            // Far trees stand on the far view, drawn with it.
            if !impostor_list.is_empty() {
                pass.set_pipeline(&self.impostor);
                pass.set_bind_group(1, &self.impostor_bind, &[]);
                pass.set_vertex_buffer(0, self.impostor_cube.slice(..));
                pass.set_vertex_buffer(1, scene.impostors.buffer().slice(..));
                draw_chunks(&mut pass, indirect, &draws, impostor_list.clone());
                pass.set_vertex_buffer(1, self.instances.slice(..));
            }
            if let Some(t) = &self.timer {
                t.mark_in_pass(&mut pass, 5);
            }

            pass.set_pipeline(&self.opaque_inside);
            pass.set_vertex_buffer(0, chunk_vertices.slice(..));
            draw_chunks(&mut pass, indirect, &draws, opaque_list);
            pass.set_pipeline(&self.opaque);
            draw_chunks(&mut pass, indirect, &draws, edge_list);

            if prop_count > 0 {
                pass.set_pipeline(&self.props);
                pass.set_vertex_buffer(0, self.cube.slice(..));
                pass.set_vertex_buffer(1, self.prop_instances.slice(..));
                pass.draw(0..36, 0..prop_count as u32);
            }
            if let Some(t) = &self.timer {
                t.mark_in_pass(&mut pass, 6);
            }
        }

        // --- Water, reading the depth of what lies beneath, then particles ---------
        let particle_count = scene.particles.len().min(self.particle_capacity);
        if particle_count > 0 {
            gpu.queue.write_buffer(&self.particle_instances, 0, bytemuck::cast_slice(&scene.particles[..particle_count]));
        }
        if !water_list.is_empty() || particle_count > 0 {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("water"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.hdr.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth.view,
                    // Read-only, so the same texture can be sampled in the shader.
                    depth_ops: None,
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.water);
            pass.set_bind_group(0, &self.scene_bind, &[]);
            pass.set_bind_group(1, &self.depth_bind, &[]);
            pass.set_index_buffer(self.quad_indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.set_vertex_buffer(0, chunk_vertices.slice(..));
            pass.set_vertex_buffer(1, self.instances.slice(..));
            draw_chunks(&mut pass, indirect, &draws, water_list.clone());
            if particle_count > 0 {
                pass.set_pipeline(&self.particles);
                pass.set_bind_group(0, &self.scene_bind, &[]);
                pass.set_vertex_buffer(0, self.cube.slice(..));
                pass.set_vertex_buffer(1, self.particle_instances.slice(..));
                pass.draw(0..36, 0..particle_count as u32);
            }
        }
        if let Some(t) = &self.timer {
            t.mark(encoder, 7);
        }

        // --- Bloom and tonemap ---------------------------------------------------
        let fullscreen = |encoder: &mut wgpu::CommandEncoder, label: &str, pipeline: &wgpu::RenderPipeline, bind: &wgpu::BindGroup, target: &wgpu::TextureView| {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
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
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind, &[]);
            pass.draw(0..3, 0..1);
        };
        // The third slot of each bind group must not be the pass's own target, so an
        // unused texture is bound there.
        let hdr_bind = self.image_bind_group(device, &self.hdr.view, &self.bloom_b.view);
        fullscreen(encoder, "bright", &self.bright, &hdr_bind, &self.bloom_a.view);
        let a_bind = self.image_bind_group(device, &self.bloom_a.view, &self.hdr.view);
        fullscreen(encoder, "blur h", &self.blur_h, &a_bind, &self.bloom_b.view);
        let b_bind = self.image_bind_group(device, &self.bloom_b.view, &self.hdr.view);
        fullscreen(encoder, "blur v", &self.blur_v, &b_bind, &self.bloom_a.view);
        if let Some(t) = &self.timer {
            t.mark(encoder, 8);
        }
        let final_bind = self.image_bind_group(device, &self.hdr.view, &self.bloom_a.view);
        fullscreen(encoder, "tonemap", &self.tonemap, &final_bind, target);
        if let Some(t) = &mut self.timer {
            t.mark(encoder, 9);
            t.resolve(encoder);
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
        let (r0, r1, r2, r3) = (view_proj.row(0), view_proj.row(1), view_proj.row(2), view_proj.row(3));
        Self {
            planes: [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r3 - r2],
        }
    }

    /// Whether a box could touch the frustum at any point as it slides along `sweep`.
    fn intersects_swept_box(&self, min: Vec3, max: Vec3, sweep: Vec3) -> bool {
        self.planes.iter().all(|p| {
            let n = p.truncate();
            let v = Vec3::select(n.cmpge(Vec3::ZERO), max, min);
            n.dot(v) + p.w + n.dot(sweep).max(0.0) >= 0.0
        })
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
#[allow(clippy::too_many_arguments)]
pub fn screenshot(
    gpu: &Gpu,
    path: &std::path::Path,
    size: (u32, u32),
    camera: &Camera,
    env: &Environment,
    streamer: &mut Streamer,
    far: &mut FarView,
    impostors: &mut Impostors,
    particles: &[ParticleInstance],
    props: &[ParticleInstance],
    wind: Vec3,
) -> anyhow::Result<()> {
    let (width, height) = size;
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut renderer = Renderer::new(gpu, format, width, height);
    streamer.wait_idle(gpu, camera.pos);
    far.wait_ready(gpu, camera.pos);
    impostors.wait_idle(gpu, camera.pos);

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
        impostors,
        particles,
        props,
        wind,
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
        "screenshot {} ({} chunks drawn, {} quads, shadow draws {:?}, multi-draw {}, arena {} MiB, meshes {} MiB, far trees {}/{} boxes)",
        path.display(),
        renderer.stats.chunks_drawn,
        renderer.stats.quads_drawn,
        renderer.stats.shadow_draws,
        renderer.multi_draw,
        streamer.stats.arena_bytes >> 20,
        streamer.stats.mesh_bytes >> 20,
        renderer.stats.impostor_boxes,
        impostors.boxes
    );
    Ok(())
}
