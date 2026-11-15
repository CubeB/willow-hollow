// The structure editor's view: voxels lit on the CPU, with a faint line round each face
// so voxels can be counted, outlines for the voxel under the cursor, and a grid for the
// ground and the layer being worked on. Colours arrive already encoded as sRGB.

struct Uniforms {
    view_proj: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) style: f32,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) style: u32,
};

@vertex
fn vs(v: VsIn) -> VsOut {
    var o: VsOut;
    o.clip = u.view_proj * vec4<f32>(v.pos, 1.0);
    o.uv = v.uv;
    o.color = v.color;
    o.style = u32(v.style + 0.5);
    return o;
}

// How near a whole-number line of `uv` this pixel is, 1 on it, about `width` pixels
// wide. Lines fade out where cells are too small on screen for them to help.
fn edge(uv: vec2<f32>, width: f32) -> f32 {
    let f = fract(uv);
    let d = min(f, vec2<f32>(1.0) - f);
    let w = max(fwidth(uv), vec2<f32>(1e-5));
    let l = vec2<f32>(1.0) - smoothstep(vec2<f32>(0.0), w * width, d);
    let small = 1.0 - smoothstep(0.08, 0.25, max(w.x, w.y));
    return max(l.x, l.y) * small;
}

@fragment
fn fs(i: VsOut) -> @location(0) vec4<f32> {
    switch i.style {
        // Voxel faces.
        case 0u: {
            return vec4<f32>(i.color.rgb * (1.0 - 0.2 * edge(i.uv, 1.0)), 1.0);
        }
        // Outlines: a faint fill and a strong edge.
        case 1u: {
            return vec4<f32>(i.color.rgb, max(i.color.a, edge(i.uv, 2.0)));
        }
        // A grid: a fill with lines on the cell edges.
        case 2u: {
            return vec4<f32>(i.color.rgb, mix(i.color.a, 0.6, edge(i.uv, 1.0)));
        }
        // Flat colour.
        default: {
            return i.color;
        }
    }
}
