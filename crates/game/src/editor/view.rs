//! What the structure editor draws with: a camera that turns about the structure, meshes
//! of voxels lit on the CPU (a sun, and shading in the corners), and a ray from the
//! cursor to the voxel under it.
//!
//! It has its own small pipeline rather than the game's renderer, which is built round
//! streaming a world of chunks; a structure is a few tens of thousands of voxels on a
//! plain patch of ground, and meshing it again after every click takes milliseconds.

use bytemuck::{Pod, Zeroable};
use glam::{IVec3, Mat4, Vec3, Vec4};
use voxel::{Materials, Voxel, mesh::PlantBox};

use crate::gfx::{self, Gpu};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 4],
    /// 0 voxel face, 1 outline, 2 grid, 3 flat (see editor.wgsl).
    pub style: f32,
}

pub const FACE: f32 = 0.0;
pub const OUTLINE: f32 = 1.0;
pub const GRID: f32 = 2.0;
pub const FLAT: f32 = 3.0;

/// A camera turning about a point: dragged round, wheeled in and out.
#[derive(Clone, Copy, Debug)]
pub struct Orbit {
    pub target: Vec3,
    /// Radians about the vertical, 0 looking from +X.
    pub yaw: f32,
    /// Radians above the horizon.
    pub pitch: f32,
    /// Voxels from the target.
    pub distance: f32,
}

impl Orbit {
    pub fn eye(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        self.target + Vec3::new(cp * cy, sp, cp * sy) * self.distance
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        // Written out by hand, as the game's camera is: right-handed, view space looking
        // down -Z, depth 0 at the near plane and 1 at the far one.
        let near = (self.distance * 0.01).clamp(0.05, 2.0);
        let far = self.distance * 20.0 + 500.0;
        let eye = self.eye();
        let f = (self.target - eye).normalize();
        let s = f.cross(Vec3::Y).normalize_or(Vec3::X);
        let u = s.cross(f);
        let view = Mat4::from_cols(
            Vec4::new(s.x, u.x, -f.x, 0.0),
            Vec4::new(s.y, u.y, -f.y, 0.0),
            Vec4::new(s.z, u.z, -f.z, 0.0),
            Vec4::new(-s.dot(eye), -u.dot(eye), f.dot(eye), 1.0),
        );
        let g = 1.0 / (20f32.to_radians()).tan();
        let depth = far / (near - far);
        let proj = Mat4::from_cols(
            Vec4::new(g / aspect.max(0.01), 0.0, 0.0, 0.0),
            Vec4::new(0.0, g, 0.0, 0.0),
            Vec4::new(0.0, 0.0, depth, -1.0),
            Vec4::new(0.0, 0.0, depth * near, 0.0),
        );
        proj * view
    }

    /// Turns by a drag of `dx`, `dy` pixels.
    pub fn turn(&mut self, dx: f32, dy: f32) {
        self.yaw += dx * 0.008;
        self.pitch = (self.pitch + dy * 0.008).clamp(-1.45, 1.52);
    }

    /// Slides the target across the view by a drag of `dx`, `dy` pixels in a view
    /// `height` pixels tall, so what was under the cursor stays under it.
    pub fn pan(&mut self, dx: f32, dy: f32, height: f32) {
        let forward = (self.target - self.eye()).normalize();
        let right = forward.cross(Vec3::Y).normalize_or_zero();
        let up = right.cross(forward);
        let per_pixel = 2.0 * self.distance * (20f32.to_radians()).tan() / height.max(1.0);
        self.target += (-right * dx + up * dy) * per_pixel;
    }

    pub fn zoom(&mut self, steps: f32) {
        self.distance = (self.distance * (1.0 - steps * 0.1).clamp(0.5, 1.5)).clamp(4.0, 1500.0);
    }

    /// The ray through pixel `(x, y)` of a view `w` by `h`, as origin and direction.
    pub fn ray(&self, x: f32, y: f32, w: f32, h: f32) -> (Vec3, Vec3) {
        let inv = self.view_proj(w / h).inverse();
        let ndc = (2.0 * x / w - 1.0, 1.0 - 2.0 * y / h);
        let near = inv * Vec4::new(ndc.0, ndc.1, 0.0, 1.0);
        let far = inv * Vec4::new(ndc.0, ndc.1, 1.0, 1.0);
        let (a, b) = (near.truncate() / near.w, far.truncate() / far.w);
        (a, (b - a).normalize())
    }

    /// Where world point `p` lands in a view `w` by `h`, in pixels, if in front.
    pub fn project(&self, p: Vec3, w: f32, h: f32) -> Option<(f32, f32)> {
        let c = self.view_proj(w / h) * p.extend(1.0);
        if c.w <= 0.0 {
            return None;
        }
        Some(((c.x / c.w + 1.0) * 0.5 * w, (1.0 - c.y / c.w) * 0.5 * h))
    }
}

/// Which voxels are filled, in a dense box, for the lookups meshing and picking make.
pub struct Grid {
    lo: IVec3,
    size: IVec3,
    filled: Vec<bool>,
}

impl Grid {
    pub fn new(cells: impl Iterator<Item = IVec3> + Clone) -> Self {
        let (mut lo, mut hi) = (IVec3::ZERO, IVec3::ZERO);
        for q in cells.clone() {
            lo = lo.min(q);
            hi = hi.max(q);
        }
        let (lo, hi) = (lo - IVec3::ONE, hi + IVec3::ONE);
        let size = hi - lo + IVec3::ONE;
        let mut g = Grid { lo, size, filled: vec![false; (size.x * size.y * size.z) as usize] };
        for q in cells {
            let i = g.index(q).unwrap();
            g.filled[i] = true;
        }
        g
    }

    fn index(&self, q: IVec3) -> Option<usize> {
        let d = q - self.lo;
        (d.cmpge(IVec3::ZERO).all() && d.cmplt(self.size).all()).then(|| ((d.y * self.size.z + d.z) * self.size.x + d.x) as usize)
    }

    pub fn get(&self, q: IVec3) -> bool {
        self.index(q).is_some_and(|i| self.filled[i])
    }

    /// The first filled voxel along a ray, with the face it was entered by, and how far
    /// along the ray that is.
    pub fn cast(&self, origin: Vec3, dir: Vec3, max: f32) -> Option<(IVec3, IVec3, f32)> {
        // Start where the ray enters the box, if it starts outside it.
        let (blo, bhi) = (self.lo.as_vec3(), (self.lo + self.size).as_vec3());
        let inv = Vec3::ONE / dir;
        let (t0, t1) = ((blo - origin) * inv, (bhi - origin) * inv);
        let enter = t0.min(t1).max_element().max(0.0);
        let leave = t0.max(t1).min_element().min(max);
        if enter > leave {
            return None;
        }
        let start = origin + dir * (enter + 1e-4);
        let mut q = start.floor().as_ivec3();
        let step = dir.signum().as_ivec3();
        let next = |q: IVec3| (q.as_vec3() + step.max(IVec3::ZERO).as_vec3() - origin) * inv;
        let mut t_max = next(q);
        let t_delta = inv.abs();
        let mut normal = IVec3::ZERO;
        // The face it entered by, if it starts in the box from outside.
        if enter > 0.0 {
            let t = t0.min(t1);
            normal = if t.x >= t.y && t.x >= t.z {
                IVec3::new(-step.x, 0, 0)
            } else if t.y >= t.z {
                IVec3::new(0, -step.y, 0)
            } else {
                IVec3::new(0, 0, -step.z)
            };
        }
        let mut t = enter;
        for _ in 0..4096 {
            if self.get(q) {
                return Some((q, normal, t));
            }
            if t_max.x < t_max.y && t_max.x < t_max.z {
                t = t_max.x;
                q.x += step.x;
                t_max.x += t_delta.x;
                normal = IVec3::new(-step.x, 0, 0);
            } else if t_max.y < t_max.z {
                t = t_max.y;
                q.y += step.y;
                t_max.y += t_delta.y;
                normal = IVec3::new(0, -step.y, 0);
            } else {
                t = t_max.z;
                q.z += step.z;
                t_max.z += t_delta.z;
                normal = IVec3::new(0, 0, -step.z);
            }
            if t > leave {
                return None;
            }
        }
        None
    }
}

fn to_linear(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// An sRGB colour lit by `light` (1 is as it is).
pub fn lit(rgb: [u8; 3], light: f32, alpha: f32) -> [f32; 4] {
    [to_srgb(to_linear(rgb[0]) * light), to_srgb(to_linear(rgb[1]) * light), to_srgb(to_linear(rgb[2]) * light), alpha]
}

/// The six faces: their normal, and the two axes across them.
const FACES: [(IVec3, IVec3, IVec3); 6] = [
    (IVec3::X, IVec3::Z, IVec3::Y),
    (IVec3::NEG_X, IVec3::Z, IVec3::Y),
    (IVec3::Y, IVec3::X, IVec3::Z),
    (IVec3::NEG_Y, IVec3::X, IVec3::Z),
    (IVec3::Z, IVec3::X, IVec3::Y),
    (IVec3::NEG_Z, IVec3::X, IVec3::Y),
];

/// How much light falls on a face with normal `n`: a sun high in the south-east, and
/// the sky all round.
fn face_light(n: IVec3) -> f32 {
    let sun = Vec3::new(0.45, 0.8, 0.38).normalize();
    0.42 + 0.68 * n.as_vec3().dot(sun).max(0.0) + if n.y < 0 { -0.08 } else { 0.0 }
}

fn quad(out: &mut Vec<Vertex>, corners: [Vec3; 4], uvs: [[f32; 2]; 4], colors: [[f32; 4]; 4], style: f32, flip: bool) {
    let order: [usize; 6] = if flip { [1, 2, 3, 1, 3, 0] } else { [0, 1, 2, 0, 2, 3] };
    for k in order {
        out.push(Vertex { pos: corners[k].to_array(), uv: uvs[k], color: colors[k], style });
    }
}

/// Faces of every voxel that are open to the air, each corner darkened by what crowds
/// it. `filled` says what counts as solid for that.
pub fn voxels(out: &mut Vec<Vertex>, voxels: &[(IVec3, Voxel)], filled: &Grid, mats: &Materials) {
    for &(q, v) in voxels {
        let m = mats.get(v);
        // A little variation voxel to voxel, as in the game.
        let h = worldgen::noise::hash3(0x51ab, q.x, q.y, q.z);
        let jitter = 1.0 + (worldgen::noise::unit(h) - 0.5) * 0.05 * m.jitter as f32;
        for &(n, a, b) in &FACES {
            if filled.get(q + n) {
                continue;
            }
            let base = if m.emissive { 1.25 } else { face_light(n) * jitter };
            let mut corners = [Vec3::ZERO; 4];
            let mut colors = [[0.0; 4]; 4];
            let mut ao = [0u8; 4];
            let centre = q.as_vec3() + Vec3::splat(0.5) + n.as_vec3() * 0.5;
            for (k, (da, db)) in [(-1, -1), (1, -1), (1, 1), (-1, 1)].into_iter().enumerate() {
                corners[k] = centre + (a * da + b * db).as_vec3() * 0.5;
                let side_a = filled.get(q + n + a * da);
                let side_b = filled.get(q + n + b * db);
                let corner = filled.get(q + n + a * da + b * db);
                ao[k] = if side_a && side_b { 0 } else { 3 - side_a as u8 - side_b as u8 - corner as u8 };
                let shade = if m.emissive { 1.0 } else { [0.5, 0.66, 0.83, 1.0][ao[k] as usize] };
                colors[k] = lit(m.color, base * shade, 1.0);
            }
            // Split the quad along the diagonal that keeps the corner shading even.
            let flip = ao[0] as u32 + ao[2] as u32 > ao[1] as u32 + ao[3] as u32;
            quad(out, corners, [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]], colors, FACE, flip);
        }
    }
}

/// A box from `lo` to `hi` in voxels, all six faces, in one colour lit by the sun.
pub fn cuboid(out: &mut Vec<Vertex>, lo: Vec3, hi: Vec3, rgb: [u8; 3], alpha: f32, style: f32, emissive: bool) {
    let size = hi - lo;
    for &(n, a, b) in &FACES {
        let light = if emissive { 1.25 } else if style == FACE || style == FLAT { face_light(n) } else { 1.0 };
        let color = lit(rgb, light, alpha);
        let nf = n.as_vec3();
        let (af, bf) = (a.as_vec3(), b.as_vec3());
        let mid = (lo + hi) * 0.5 + nf * size * 0.5;
        let (ha, hb) = (af * size * 0.5, bf * size * 0.5);
        // Outlines and grids count their cells in whole voxels along each side.
        let (ua, ub) = ((size * af).length(), (size * bf).length());
        let uv = if style == OUTLINE { [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]] } else { [[0.0, 0.0], [ua, 0.0], [ua, ub], [0.0, ub]] };
        quad(out, [mid - ha - hb, mid + ha - hb, mid + ha + hb, mid - ha + hb], uv, [color; 4], style, false);
    }
}

/// The fine parts, as small boxes.
pub fn fine(out: &mut Vec<Vertex>, parts: &[PlantBox], mats: &Materials) {
    let cell = 1.0 / voxel::mesh::CELLS as f32;
    for b in parts {
        let m = mats.get(b.voxel);
        let (lo, hi) = (IVec3::from_array(b.lo).as_vec3() * cell, IVec3::from_array(b.hi).as_vec3() * cell);
        cuboid(out, lo, hi, m.color, 1.0, FLAT, m.emissive);
    }
}

/// A flat square of ground or grid at height `y`, `reach` voxels each way from the
/// middle, one grid cell to a voxel.
pub fn plane(out: &mut Vec<Vertex>, y: f32, reach: i32, rgb: [u8; 3], alpha: f32, style: f32) {
    let r = reach as f32;
    let color = lit(rgb, if style == FACE { face_light(IVec3::Y) } else { 1.0 }, alpha);
    let corners = [Vec3::new(-r, y, -r), Vec3::new(r, y, -r), Vec3::new(r, y, r), Vec3::new(-r, y, r)];
    let uv = corners.map(|c| [c.x, c.z]);
    quad(out, corners, uv, [color; 4], style, false);
}

/// An arrow lying on the ground from `from` toward `dir`, `length` long.
pub fn arrow(out: &mut Vec<Vertex>, from: Vec3, dir: Vec3, length: f32, rgb: [u8; 3], alpha: f32) {
    let side = Vec3::new(-dir.z, 0.0, dir.x);
    let color = [lit(rgb, 1.0, alpha); 4];
    let uv = [[0.5, 0.5]; 4];
    let neck = from + dir * length * 0.6;
    let w = length * 0.12;
    quad(out, [from - side * w, neck - side * w, neck + side * w, from + side * w], uv, color, FLAT, false);
    let tip = from + dir * length;
    quad(out, [neck - side * w * 2.6, tip, tip, neck + side * w * 2.6], uv, color, FLAT, false);
}

/// A vertex buffer that grows to fit what it is given.
struct Buffer {
    buffer: Option<wgpu::Buffer>,
    capacity: usize,
    count: u32,
}

impl Buffer {
    fn load(&mut self, gpu: &Gpu, verts: &[Vertex]) {
        let bytes: &[u8] = bytemuck::cast_slice(verts);
        if bytes.len() > self.capacity || self.buffer.is_none() {
            self.capacity = bytes.len().next_power_of_two().max(4096);
            self.buffer = Some(gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("structure editor"),
                size: self.capacity as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        if !bytes.is_empty() {
            gpu.queue.write_buffer(self.buffer.as_ref().unwrap(), 0, bytes);
        }
        self.count = verts.len() as u32;
    }
}

pub struct View {
    solid: wgpu::RenderPipeline,
    glass: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind: wgpu::BindGroup,
    opaque: Buffer,
    clear: Buffer,
    overlay: Buffer,
    depth: Option<(wgpu::TextureView, (u32, u32))>,
}

/// The colour behind everything: a pale sky.
pub const SKY: [u8; 3] = [196, 214, 228];

impl View {
    pub fn new(gpu: &Gpu, format: wgpu::TextureFormat) -> Self {
        let device = &gpu.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("structure editor"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/editor.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("structure editor"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("structure editor"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("structure editor"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("structure editor"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x4, 3 => Float32];
        let pipeline = |blend: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(if blend { "structure editor glass" } else { "structure editor" }),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &attrs,
                    })],
                },
                // Faces are drawn from both sides, so their winding never matters.
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: gfx::DEPTH_FORMAT,
                    depth_write_enabled: Some(!blend),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: blend.then_some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let empty = || Buffer { buffer: None, capacity: 0, count: 0 };
        View { solid: pipeline(false), glass: pipeline(true), uniforms, bind, opaque: empty(), clear: empty(), overlay: empty(), depth: None }
    }

    /// Sets what is drawn: solid things, and see-through ones drawn over them.
    pub fn load(&mut self, gpu: &Gpu, opaque: &[Vertex], clear: &[Vertex]) {
        self.opaque.load(gpu, opaque);
        self.clear.load(gpu, clear);
    }

    /// Sets what is drawn over everything else, see-through: the cursor and the like.
    pub fn load_overlay(&mut self, gpu: &Gpu, verts: &[Vertex]) {
        self.overlay.load(gpu, verts);
    }

    /// Draws into `viewport` (x, y, width, height in pixels) of a target `size` big,
    /// clearing the whole of it to the sky first.
    pub fn draw(&mut self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, size: (u32, u32), viewport: [f32; 4], orbit: &Orbit) {
        if self.depth.as_ref().is_none_or(|(_, s)| *s != size) {
            self.depth = Some((gfx::create_depth(&gpu.device, size.0, size.1), size));
        }
        let [x, y, w, h] = clamp_viewport(viewport, size);
        let vp = orbit.view_proj(w / h);
        gpu.queue.write_buffer(&self.uniforms, 0, bytemuck::cast_slice(&vp.to_cols_array()));
        let sky = lit(SKY, 1.0, 1.0);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("structure editor"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color { r: sky[0] as f64, g: sky[1] as f64, b: sky[2] as f64, a: 1.0 }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth.as_ref().unwrap().0,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_viewport(x, y, w, h, 0.0, 1.0);
        pass.set_bind_group(0, &self.bind, &[]);
        for (pipeline, buffer) in [(&self.solid, &self.opaque), (&self.glass, &self.clear), (&self.glass, &self.overlay)] {
            if let Some(b) = &buffer.buffer
                && buffer.count > 0
            {
                pass.set_pipeline(pipeline);
                pass.set_vertex_buffer(0, b.slice(..));
                pass.draw(0..buffer.count, 0..1);
            }
        }
    }
}

/// A viewport kept inside the target and at least a pixel each way.
pub fn clamp_viewport(v: [f32; 4], size: (u32, u32)) -> [f32; 4] {
    let (sw, sh) = (size.0 as f32, size.1 as f32);
    let x = v[0].clamp(0.0, sw - 1.0);
    let y = v[1].clamp(0.0, sh - 1.0);
    [x, y, v[2].clamp(1.0, sw - x), v[3].clamp(1.0, sh - y)]
}
