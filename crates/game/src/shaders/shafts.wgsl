// Light shafts: sunlight caught by the haze and mist in the air near the camera, where
// the shadow maps say the sun reaches it. Worked out at a quarter of the screen's size
// (fs_march), then laid over the scene at full size (fs_lay).

const NEAR: f32 = 0.08 * VOXELS_PER_METRE;
const STEPS: i32 = 10;
// The furthest view depth kept: the sky's, and all a half float need hold.
const FAR: f32 = 60000.0;
// Lit air shows far more than its true thinness over so short a way would allow.
const SHOW: f32 = 22.0;
// The distance over which the air's share of a shaft falls to about a third. A shaft is
// seen in the air near the eye; lit air a long way off, summed along the view, is only a
// pale wash over whatever stands behind it.
const NEARBY: f32 = 45.0 * VOXELS_PER_METRE;

@group(1) @binding(0) var scene_depth: texture_depth_2d;
@group(1) @binding(1) var marched: texture_2d<f32>;

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    var out: VOut;
    out.clip = vec4<f32>(p, 0.0, 1.0);
    out.ndc = p;
    return out;
}

fn view_depth(at: vec2<i32>) -> f32 {
    let size = vec2<i32>(textureDimensions(scene_depth));
    // Reverse-Z: depth = near / distance along the view.
    return min(NEAR / max(textureLoad(scene_depth, clamp(at, vec2<i32>(0), size - 1), 0), 1e-6), FAR);
}

// A different fraction for each pixel, evenly spread among neighbours.
fn scatter(p: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

// x: how much lit haze and mist lie along the view ray, as far as the first two shadow
// maps reach or to what the ray hits, where shade lies along it too: a shaft shows only
// against shaded air beside it, and open air is left as the fog already has it.
// y: the view depth this was worked out for.
@fragment
fn fs_march(in: VOut) -> @location(0) vec4<f32> {
    let depth = view_depth(vec2<i32>(in.clip.xy) * 4 + 2);
    // One unit along this is one voxel of view depth.
    let ray = G.cam_fwd.xyz + in.ndc.x * G.cam_right.xyz + in.ndc.y * G.cam_up.xyz;
    let reach = min(depth, G.cascades.y);
    let shift = scatter(in.clip.xy);
    let n = f32(STEPS);
    var lit = 0.0;
    var all = 0.0;
    for (var i = 0; i < STEPS; i++) {
        // Steps lengthen with distance: shafts are finest close by.
        let f = f32(i);
        let t = reach * (f * f + shift * (2.0 * f + 1.0)) / (n * n);
        let p = ray * t;
        let c = select(0, 1, t >= G.cascades.x);
        let lp = S.light_vp[c] * vec4<f32>(p, 1.0);
        let uv = lp.xy * vec2<f32>(0.5, -0.5) + 0.5;
        let sun = textureSampleCompareLevel(shadow_maps, shadow_sampler, uv, c, lp.z);
        let haze = G.air.x * exp(-max(G.cam_wrapped.y + p.y, 0.0) / G.air.y);
        let mist = G.mist.x * exp(-max(p.y - G.mist.z, 0.0) / G.mist.y);
        let air = (haze + mist) * (2.0 * f + 1.0) * exp(-t / NEARBY);
        lit += sun * air;
        all += air;
    }
    // A shaft shows from the shade: with the eye in the open and the sun on most of the
    // air along the view, that air is only a pale wash over what stands behind it.
    let shade = 1.0 - lit / max(all, 1e-12);
    lit *= min(4.0 * shade * shade, 1.0);
    lit *= reach * length(ray) / (n * n) * cloud_light(ray * reach * 0.5);
    return vec4<f32>(lit, depth, 0.0, 1.0);
}

// Added to the scene: the marched light, taken from the quarter-size pixels round this
// one that lie at its own depth, so shafts do not bleed over the edges of what stands
// in front of them.
@fragment
fn fs_lay(in: VOut) -> @location(0) vec4<f32> {
    let depth = view_depth(vec2<i32>(in.clip.xy));
    let size = vec2<i32>(textureDimensions(marched));
    let at = in.clip.xy / 4.0 - 0.5;
    let near = vec2<i32>(round(at));
    var sum = 0.0;
    var weight = 0.0;
    // Wide enough to smooth away the grain of so few steps along each ray.
    for (var j = -2; j <= 2; j++) {
        for (var i = -2; i <= 2; i++) {
            let q = near + vec2<i32>(i, j);
            let m = textureLoad(marched, clamp(q, vec2<i32>(0), size - 1), 0);
            let off = vec2<f32>(q) - at;
            let w = exp(-dot(off, off) * 0.22) * (max(1.0 - abs(m.y - depth) / (depth * 0.12 + 1.0), 0.0) + 0.002);
            sum += m.x * w;
            weight += w;
        }
    }
    let lit = sum / weight;
    let dir = normalize(G.cam_fwd.xyz + in.ndc.x * G.cam_right.xyz + in.ndc.y * G.cam_up.xyz);
    // Haze throws nearly all of its light onward, so shafts show looking up them toward
    // the sun, and hardly at all across them or with the sun behind.
    let c = max(dot(dir, G.sun_dir.xyz), 0.0);
    let c2 = c * c;
    let onward = 0.03 + 0.3 * c2 + 1.3 * c2 * c2 * c2 * c2;
    let shown = lit * SHOW;
    return vec4<f32>(G.sun_color.rgb * (shown / (1.0 + shown) * 0.42 * onward * G.air.z), 0.0);
}
