// Full-screen passes: bloom extraction and blur, and the final tonemap.
// MODE and ENCODE_SRGB are substituted at pipeline creation.

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;
@group(0) @binding(2) var bloom: texture_2d<f32>;

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    var out: VOut;
    out.clip = vec4<f32>(p, 0.0, 1.0);
    out.uv = p * vec2<f32>(0.5, -0.5) + 0.5;
    return out;
}

// Keeps only what is brighter than white, averaged over a 2x2 block.
@fragment
fn fs_bright(in: VOut) -> @location(0) vec4<f32> {
    let c = textureSample(src, src_sampler, in.uv).rgb;
    let lum = dot(c, vec3<f32>(0.3, 0.59, 0.11));
    let keep = smoothstep(0.9, 1.6, lum);
    return vec4<f32>(c * keep, 1.0);
}

// One axis of a Gaussian blur; DIR is substituted.
@fragment
fn fs_blur(in: VOut) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(src));
    let step = DIR / size;
    let w = array<f32, 5>(0.227, 0.194, 0.121, 0.054, 0.016);
    var c = textureSample(src, src_sampler, in.uv).rgb * w[0];
    for (var i = 1; i < 5; i++) {
        let o = step * f32(i) * 1.5;
        c += (textureSample(src, src_sampler, in.uv + o).rgb + textureSample(src, src_sampler, in.uv - o).rgb) * w[i];
    }
    return vec4<f32>(c, 1.0);
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
fn fs_tonemap(in: VOut) -> @location(0) vec4<f32> {
    let c = textureLoad(src, vec2<i32>(in.clip.xy), 0).rgb
        + textureSample(bloom, src_sampler, in.uv).rgb * 0.35;
    var o = aces(c * 0.9);
    if (ENCODE_SRGB) {
        o = linear_to_srgb(o);
    }
    return vec4<f32>(o, 1.0);
}
