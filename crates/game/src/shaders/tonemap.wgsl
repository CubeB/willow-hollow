// Final pass: HDR scene -> display. ENCODE_SRGB is substituted at pipeline creation:
// true when the target is a plain (non-sRGB) format, so the shader must encode.

@group(0) @binding(0) var hdr: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    return vec4<f32>(p, 0.0, 1.0);
}

// ACES filmic fit (Narkowicz).
fn aces(x: vec3<f32>) -> vec3<f32> {
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let c = textureLoad(hdr, vec2<i32>(pos.xy), 0).rgb;
    var o = aces(c * 0.9);
    if (ENCODE_SRGB) {
        o = linear_to_srgb(o);
    }
    return vec4<f32>(o, 1.0);
}
