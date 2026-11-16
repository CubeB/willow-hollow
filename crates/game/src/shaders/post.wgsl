// Full-screen passes: bloom extraction and blur, and the final tonemap.
// MODE and ENCODE_SRGB are substituted at pipeline creation.

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;
@group(0) @binding(2) var bloom: texture_2d<f32>;
@group(0) @binding(3) var depth: texture_depth_2d;

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

// True if something was drawn at pixel `p` (the depth no longer as it was cleared), and
// false for the sky and off the edge of the picture.
fn land(p: vec2<i32>) -> bool {
    let size = vec2<i32>(textureDimensions(depth));
    return all(p >= vec2<i32>(0)) && all(p < size) && textureLoad(depth, p, 0) != 0.0;
}

// The scene at pixel `p`, with pinholes closed. A merged face of the voxel terrain runs
// past the corners of the smaller faces beside it (a T-junction), and where their edges
// meet the GPU rounds each corner on its own, so now and then a pixel's centre falls in
// the hair's-breadth crack between them and the bright sky behind the hill shows through,
// a speck that flickers white as the view turns. Such a speck is one pixel of sky, or two
// side by side, with land on every side, which open sky never is (a notch in a hill's
// outline is open to the sky beyond it); it takes the land around it instead. Everything
// else is left exactly as it was drawn.
fn scene(p: vec2<i32>) -> vec3<f32> {
    let c = textureLoad(src, p, 0).rgb;
    if (land(p)) {
        return c;
    }
    var around = array<vec2<i32>, 4>(vec2<i32>(-1, 0), vec2<i32>(1, 0), vec2<i32>(0, -1), vec2<i32>(0, 1));
    var sum = vec3<f32>(0.0);
    var closed = 0;
    var open = vec2<i32>(0);
    for (var i = 0; i < 4; i++) {
        let q = p + around[i];
        if (land(q)) {
            sum += textureLoad(src, q, 0).rgb;
            closed += 1;
        } else {
            open = around[i];
        }
    }
    if (closed == 4) {
        return sum * 0.25;
    }
    // Open on one side only: the other pixel of a pair, if it is closed on its other three.
    let q = p + open;
    let side = open.yx;
    if (closed == 3 && land(q + open) && land(q + side) && land(q - side)) {
        return sum / 3.0;
    }
    return c;
}

@fragment
fn fs_tonemap(in: VOut) -> @location(0) vec4<f32> {
    let c = scene(vec2<i32>(in.clip.xy))
        + textureSample(bloom, src_sampler, in.uv).rgb * 0.35;
    // The grade: shade leans a little cool and bright light a little warm, which
    // gives depth between sunlit ground and what lies in shadow.
    let lum = dot(c, vec3<f32>(0.3, 0.59, 0.11));
    let graded = c * mix(vec3<f32>(0.94, 0.99, 1.08), vec3<f32>(1.03, 1.0, 0.96), smoothstep(0.02, 0.6, lum));
    var o = aces(graded * 0.9);
    if (ENCODE_SRGB) {
        o = linear_to_srgb(o);
    }
    return vec4<f32>(o, 1.0);
}
