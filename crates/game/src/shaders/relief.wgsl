// The large map's model: blocks of land, water and trees, lit flat from the north-west
// like a relief map. Colours are sRGB, as the target is written without encoding.

struct Uniforms {
    // From the model's columns and blocks to the screen.
    transform: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec3<f32>,
};

@vertex
fn vs(@location(0) pos: vec3<f32>, @location(1) color: vec4<f32>) -> VOut {
    // Up, east, west, south, north.
    var shades = array<f32, 5>(1.0, 0.70, 0.90, 0.80, 0.62);
    let face = u32(color.a * 255.0 + 0.5);
    var out: VOut;
    out.clip = u.transform * vec4(pos, 1.0);
    out.color = color.rgb * shades[min(face, 4u)];
    return out;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    return vec4(in.color, 1.0);
}
