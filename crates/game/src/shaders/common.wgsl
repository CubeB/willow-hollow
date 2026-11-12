// Shared by every scene shader (prepended at pipeline creation).

struct Globals {
    view_proj: mat4x4<f32>,
    // Camera basis scaled so that fwd + ndc.x * right + ndc.y * up is the view ray.
    cam_right: vec4<f32>,
    cam_up: vec4<f32>,
    cam_fwd: vec4<f32>,
    // xyz: direction toward the sun (or moon at night). w: time in seconds.
    sun_dir: vec4<f32>,
    // rgb: light colour. w: 1 by day, 0 by night (moonlight).
    sun_color: vec4<f32>,
    sky_zenith: vec4<f32>,
    sky_horizon: vec4<f32>,
    // x: fog density per voxel. y: horizontal distance at which voxel terrain hands
    // over to the far view. z: cloud cover (0..1). w: cloud drift in voxels.
    fog: vec4<f32>,
    // xyz: camera world position, wrapped to keep it small. Adding it to a
    // camera-relative position gives a stable world-anchored coordinate.
    cam_wrapped: vec4<f32>,
    // Ground mist: x strength, y height falloff, z the level it is thickest below
    // (camera-relative y), w unused.
    mist: vec4<f32>,
    // Cascade far distances (view depth) in xyz; w = cloud layer height, camera-relative.
    cascades: vec4<f32>,
    // xyz: wind vector in voxels per second.
    wind: vec4<f32>,
    // Nearest glowing fireflies: camera-relative xyz, glow strength in w (0 = unused).
    lights: array<vec4<f32>, 4>,
    // The nearest lanterns: camera-relative xyz, w 1 where there is one.
    lamps: array<vec4<f32>, 4>,
    // x: date, 0..1 from the spring equinox (see season.wgsl). y: snow cover at sea level.
    // z: ice on still water. w: the summit snowline, metres (see ground.wgsl).
    season: vec4<f32>,
    // x: how wet the ground is (0..1). y: how much more snow the hills hold this winter.
    // (season.yzw and weather.y are the ground where the camera is.)
    weather: vec4<f32>,
    // rgb: the horizon toward the sun. w: how far into dusk or dawn the sky is.
    sky_glow: vec4<f32>,
    // xyz: toward the sun itself, by night too (sun_dir is then the moon).
    sun_pos: vec4<f32>,
    // Haze: x density per voxel at sea level, y the height it thins over, w how hazy
    // the air is (0..1), for the sun's glow in it.
    air: vec4<f32>,
    // Each biome's ground, by biome id: x snow at sea level, y ice on still water, z how
    // much more snow the hills hold, w the summit snowline (see ground.wgsl).
    snow: array<vec4<f32>, 24>,
};

struct Shadows {
    light_vp: array<mat4x4<f32>, 3>,
    // For each cascade, how wide its texels are as the sun sees them, and how far along
    // the sun's rays its depth runs, both in voxels (see Renderer::cascade_matrix).
    texel: vec4<f32>,
    span: vec4<f32>,
};

@group(0) @binding(0) var<uniform> G: Globals;
@group(0) @binding(1) var<uniform> S: Shadows;
@group(0) @binding(2) var shadow_maps: texture_depth_2d_array;
@group(0) @binding(3) var shadow_sampler: sampler_comparison;

fn hash3(p: vec3<i32>) -> f32 {
    var h = bitcast<u32>(p.x) * 0x8da6b343u
          + bitcast<u32>(p.y) * 0xd8163841u
          + bitcast<u32>(p.z) * 0xcb1ab31fu;
    h ^= h >> 15u;
    h *= 0x2c1b3c6du;
    h ^= h >> 12u;
    h *= 0x297a2d39u;
    h ^= h >> 15u;
    return f32(h >> 8u) * (1.0 / 16777216.0);
}

// Smooth value noise over a 2D lattice.
fn vnoise(p: vec2<f32>) -> f32 {
    let i = vec2<i32>(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash3(vec3<i32>(i.x, i.y, 7));
    let b = hash3(vec3<i32>(i.x + 1, i.y, 7));
    let c = hash3(vec3<i32>(i.x, i.y + 1, 7));
    let d = hash3(vec3<i32>(i.x + 1, i.y + 1, 7));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var a = 0.5;
    var q = p;
    for (var i = 0; i < 4; i++) {
        v += a * vnoise(q);
        q = q * 2.03 + vec2<f32>(17.0, 31.0);
        a *= 0.5;
    }
    return v;
}

// Cloud density (0..1) over a world xz position.
fn cloud_density(world_xz: vec2<f32>) -> f32 {
    let p = (world_xz + vec2<f32>(G.fog.w, G.fog.w * 0.35)) / (336.0 * VOXELS_PER_METRE);
    let n = fbm(p);
    return smoothstep(0.62 - G.fog.z * 0.42, 0.78 - G.fog.z * 0.2, n);
}

// The same clouds as they fall on the ground, where their fine detail can't be seen:
// this runs for every pixel of terrain, so it does a quarter of the work.
fn cloud_shadow(world_xz: vec2<f32>) -> f32 {
    let p = (world_xz + vec2<f32>(G.fog.w, G.fog.w * 0.35)) / (336.0 * VOXELS_PER_METRE);
    let n = vnoise(p) * 0.625;
    return smoothstep(0.62 - G.fog.z * 0.42, 0.78 - G.fog.z * 0.2, n + vnoise(p * 2.03 + vec2<f32>(17.0, 31.0)) * 0.3125);
}

// Where voxel terrain hands over to the far view. Both sides call this, so every
// pixel is drawn by exactly one of them.
//
// The edge has to be a plain circle around the camera. Voxel terrain has no walls
// where it is cut off, so if far terrain ever lay between the camera and voxel
// terrain (as it would along a ragged or dithered edge) the view would pass under the
// voxels' cut edge and out the other side.
fn beyond_voxels(rel: vec3<f32>) -> bool {
    return length(rel.xz) >= G.fog.y;
}

// How far round toward the sun a direction lies: 1 facing it, 0 with it behind.
fn sunward(dir: vec3<f32>) -> f32 {
    let d = dir.xz / max(length(dir.xz), 1e-4);
    let s = G.sun_pos.xz / max(length(G.sun_pos.xz), 1e-4);
    return dot(d, s) * 0.5 + 0.5;
}

// The glow of hazy air around the sun (or moon), for the cosine of the angle from it.
fn sun_haze(s: f32) -> vec3<f32> {
    let c = max(s, 0.0);
    let c4 = c * c * c * c;
    let c32 = c4 * c4 * c4 * c4 * c4 * c4 * c4 * c4;
    return G.sun_color.rgb * (c4 * 0.05 + c32 * 0.20) * G.air.w;
}

// The sky without the sun disc or the glow of haze near the horizon.
fn sky_base(dir: vec3<f32>) -> vec3<f32> {
    let up = clamp(dir.y, -1.0, 1.0);
    let t = pow(1.0 - max(up, 0.0), 3.0);
    // The horizon is warm round the sun at dusk and dawn, and cool opposite it.
    let toward = sunward(dir);
    let horizon = mix(G.sky_horizon.rgb, G.sky_glow.rgb, toward * toward);
    var c = mix(G.sky_zenith.rgb, horizon, t);
    if (up < 0.0) {
        c = mix(c, horizon * 0.7, clamp(-up * 4.0, 0.0, 1.0));
    }
    let s = max(dot(dir, G.sun_dir.xyz), 0.0);
    c += G.sun_color.rgb * (pow(s, 6.0) * 0.03 + pow(s, 48.0) * 0.12);
    return c;
}

// Sky radiance without the sun disc. Its horizon is what the far distance fades to
// (see apply_fog), so land dissolves into exactly the sky behind it.
fn sky_color(dir: vec3<f32>) -> vec3<f32> {
    let low = 1.0 - max(dir.y, 0.0);
    let low2 = low * low;
    return sky_base(dir) + sun_haze(dot(dir, G.sun_dir.xyz)) * low2 * low2 * low2;
}

// Clear air scatters blue most, so it is the blue of a distant thing that goes first.
const AIR_TINT: vec3<f32> = vec3<f32>(0.45, 0.9, 1.7);

// The mean along a straight path of a layer that is 1 up to height 0 and falls off as
// exp(-y) above it, between heights y0 and y1 (in units of its falloff).
fn layer_mean(y0: f32, y1: f32) -> f32 {
    let lo = min(y0, y1);
    let hi = max(y0, y1);
    if (hi - lo < 0.01) {
        return exp(-max(lo, 0.0));
    }
    // Below the level the layer is full; above it, the integral of exp(-y).
    let under = max(min(hi, 0.0) - lo, 0.0);
    let over = exp(-max(lo, 0.0)) - exp(-max(hi, 0.0));
    return (under + over) / (hi - lo);
}

// How much haze and mist lie between the camera and a point. Both thin with height, so
// a valley fills while the ridge above stands clear, and seen from above the mist is a
// thin sheet lying in the low ground.
fn haze_depth(rel: vec3<f32>, dist: f32) -> f32 {
    let haze = G.air.x * layer_mean(G.cam_wrapped.y / G.air.y, (G.cam_wrapped.y + rel.y) / G.air.y);
    let mist = G.mist.x * layer_mean(-G.mist.z / G.mist.y, (rel.y - G.mist.z) / G.mist.y);
    return (haze + mist) * dist;
}

fn apply_fog(color: vec3<f32>, rel: vec3<f32>) -> vec3<f32> {
    let dist = length(rel);
    let haze = haze_depth(rel, dist);
    let through = exp(-(dist * G.fog.x * AIR_TINT + haze));
    // What the air adds is the sky at the horizon in that direction, and where there
    // is haze, the sun's glow in it.
    let dir = normalize(vec3<f32>(rel.x, abs(rel.y) * 0.15 + dist * 0.02, rel.z));
    let glow = sun_haze(dot(rel, G.sun_dir.xyz) / max(dist, 1e-4)) * (1.0 - exp(-haze));
    return color * through + (sky_base(dir) + glow) * (1.0 - through);
}

// Warm light from nearby fireflies, falling off over about eight metres.
fn local_lights(rel: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    for (var i = 0; i < 4; i++) {
        let l = G.lights[i];
        let d = l.xyz - rel;
        let dist = length(d) / VOXELS_PER_METRE;
        let wrap = 0.35 + 0.65 * max(dot(n, d / max(length(d), 0.001)), 0.0);
        let reach = clamp(1.0 - dist / 8.0, 0.0, 1.0);
        sum += vec3<f32>(0.85, 1.0, 0.35) * (l.w * 0.045) * wrap * reach * reach / (1.0 + dist * dist * 0.6);
    }
    // Lanterns: a warm light that carries a long way after dark, and hardly shows by day.
    let night = 1.0 - 0.9 * G.sun_color.w;
    for (var i = 0; i < 4; i++) {
        let l = G.lamps[i];
        if (l.w > 0.5) {
            let d = l.xyz - rel;
            let dist = length(d) / VOXELS_PER_METRE;
            let wrap = 0.3 + 0.7 * max(dot(n, d / max(length(d), 0.001)), 0.0);
            let reach = clamp(1.0 - dist / 16.0, 0.0, 1.0);
            sum += vec3<f32>(1.0, 0.68, 0.32) * (2.2 * night) * wrap * reach * reach / (1.0 + dist * dist * 0.25);
        }
    }
    return sum;
}

fn ambient(n: vec3<f32>) -> vec3<f32> {
    let sky = mix(mix(G.sky_horizon.rgb, G.sky_glow.rgb, 0.4), G.sky_zenith.rgb, 0.6) * (0.55 + 0.45 * n.y) * 0.8;
    // Light bounced off the ground, tinted warm, scaled by how much sun there is.
    // Reaches sideways and downward faces, which the sky lights least.
    let bounce = G.sun_color.rgb * vec3<f32>(0.20, 0.17, 0.12) * (0.65 - 0.35 * n.y)
        * max(G.sun_dir.y, 0.0);
    return sky + bounce;
}

// How much sun the clouds let through to a point.
fn cloud_light(rel: vec3<f32>) -> f32 {
    // Where the sun's ray through this point crosses the cloud layer.
    let to_cloud = (G.cascades.w - rel.y) / max(G.sun_dir.y, 0.05);
    let at_cloud = rel.xz + G.sun_dir.xz * to_cloud + G.cam_wrapped.xz;
    return 1.0 - cloud_shadow(at_cloud) * 0.75;
}

// How much sun reaches ground beyond the shadow maps, whose skylines toward sunrise and
// sunset are known (farview::Skylines, each byte as 0..1): dark once the sun is below
// the hills on its side, with an edge that is softer the higher the hills.
fn skyline_light(skylines: vec2<f32>) -> f32 {
    let packed = select(skylines.y, skylines.x, G.sun_dir.x > 0.0);
    let skyline = packed * packed * 1.5;
    let sun = G.sun_dir.y / max(length(G.sun_dir.xz), 1e-3);
    let soft = 0.025 + skyline * 0.12;
    return smoothstep(skyline - soft, skyline + soft, sun);
}

fn sun_visibility(rel: vec3<f32>, n: vec3<f32>) -> f32 {
    return sun_reaching(rel, n, 1.0);
}

const SHADOW_SIZE: f32 = 2048.0;
// How wide the sun is taken to be: how far a shadow's edge spreads for each voxel
// between what casts it and what it falls on. Wider than the real sun, for soft light.
const SUN_WIDTH: f32 = 0.045;

// What one shadow map says of a point. x: how much sun reaches it. y: 1 if the point
// lies in the map at all.
fn cascade_light(c: i32, rel: vec3<f32>, n: vec3<f32>) -> vec2<f32> {
    let texel = S.texel[c];
    // Push the lookup off the surface along its normal and toward the light so
    // flat faces do not shadow themselves.
    // No more than 1: a normal that is the sun's own direction (light through a leaf)
    // rounds to a hair over it on some frames, and the slope below would have no value.
    let ndl = clamp(dot(n, G.sun_dir.xyz), 0.0, 1.0);
    let offset = rel + n * texel * 1.5 * (1.0 - ndl * 0.5) + G.sun_dir.xyz * texel * 0.5;
    let lp = S.light_vp[c] * vec4<f32>(offset, 1.0);
    let uv = lp.xy * vec2<f32>(0.5, -0.5) + 0.5;
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) {
        return vec2<f32>(1.0, 0.0);
    }
    let span = S.span[c];
    // A shadow is sharp where it meets what casts it and spreads with distance from it:
    // a trunk's is crisp at its foot, a high canopy's is soft. So look for what stands
    // between this point and the sun, and how far off it is.
    var radius = 1.0;
    if (c < 2) {
        let at = vec2<i32>(uv * SHADOW_SIZE);
        var looks = array<vec2<i32>, 5>(
            vec2<i32>(0, 0), vec2<i32>(3, 2), vec2<i32>(-2, 3), vec2<i32>(-3, -2), vec2<i32>(2, -3));
        var gap = 0.0;
        var found = 0.0;
        for (var k = 0; k < 5; k++) {
            let p = clamp(at + looks[k], vec2<i32>(0), vec2<i32>(2047));
            let z = textureLoad(shadow_maps, p, c, 0);
            if (z < lp.z) {
                gap += lp.z - z;
                found += 1.0;
            }
        }
        if (found > 0.0) {
            radius = clamp(gap / found * span * SUN_WIDTH / texel, 1.0, select(2.5, 5.0, c == 0));
        }
    }
    // A wide filter on a face turned from the sun would reach under the face itself.
    let slope = sqrt(1.0 - ndl * ndl) / max(ndl, 0.25);
    let z = lp.z - (radius - 1.0) * texel * 0.5 * slope / span;
    // Taps spread evenly over a disc, more of them where it is widest.
    let taps = select(select(5, 8, c == 1), 12, c == 0);
    var sum = 0.0;
    for (var i = 0; i < taps; i++) {
        let turn = f32(i) * 2.39996;
        let out = sqrt((f32(i) + 0.5) / f32(taps)) * radius / SHADOW_SIZE;
        sum += textureSampleCompareLevel(shadow_maps, shadow_sampler, uv + vec2<f32>(cos(turn), sin(turn)) * out, c, z);
    }
    return vec2<f32>(sum / f32(taps), 1.0);
}

// Fraction of direct sunlight reaching a point: cascaded shadow maps, filtered wider
// the further the shadow has fallen, then cloud shadow. `beyond` is what to take it as
// past the last shadow map.
fn sun_reaching(rel: vec3<f32>, n: vec3<f32>, beyond: f32) -> f32 {
    let depth = dot(rel, G.cam_fwd.xyz);
    var vis = beyond;
    if (depth < G.cascades.z) {
        var c = 0;
        var end = G.cascades.x;
        if (depth >= G.cascades.y) {
            c = 2;
            end = G.cascades.z;
        } else if (depth >= G.cascades.x) {
            c = 1;
            end = G.cascades.y;
        }
        let here = cascade_light(c, rel, n);
        vis = mix(beyond, here.x, here.y);
        if (c == 2) {
            // The last cascade fades out rather than ending in a line.
            vis = mix(vis, beyond, smoothstep(end * 0.7, end, depth));
        } else if (depth > end * 0.85) {
            // Each map is coarser than the last: one fades into the next over the end
            // of its reach, so the change is not a line across the ground.
            let next = cascade_light(c + 1, rel, n);
            vis = mix(vis, next.x, smoothstep(end * 0.85, end, depth) * next.y);
        }
    }
    return vis * cloud_light(rel);
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return pow(c, vec3<f32>(2.2));
}
