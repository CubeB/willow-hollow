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
    // Ground mist: x strength, y height falloff, z level (camera-relative y), w unused.
    mist: vec4<f32>,
    // Cascade far distances (view depth) in xyz; w = cloud layer height, camera-relative.
    cascades: vec4<f32>,
};

struct Shadows {
    light_vp: array<mat4x4<f32>, 3>,
    // Shadow-map texel size in voxels for each cascade.
    texel: vec4<f32>,
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
    let p = (world_xz + vec2<f32>(G.fog.w, G.fog.w * 0.35)) / 420.0;
    let n = fbm(p);
    return smoothstep(0.62 - G.fog.z * 0.42, 0.78 - G.fog.z * 0.2, n);
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

// Sky radiance without the sun disc. Also used as the fog colour so distant terrain
// dissolves into exactly the sky behind it.
fn sky_color(dir: vec3<f32>) -> vec3<f32> {
    let up = clamp(dir.y, -1.0, 1.0);
    let t = pow(1.0 - max(up, 0.0), 3.0);
    var c = mix(G.sky_zenith.rgb, G.sky_horizon.rgb, t);
    if (up < 0.0) {
        c = mix(c, G.sky_horizon.rgb * 0.7, clamp(-up * 4.0, 0.0, 1.0));
    }
    let s = max(dot(dir, G.sun_dir.xyz), 0.0);
    c += G.sun_color.rgb * (pow(s, 6.0) * 0.03 + pow(s, 48.0) * 0.12);
    return c;
}

fn apply_fog(color: vec3<f32>, rel: vec3<f32>) -> vec3<f32> {
    let dist = length(rel);
    // Ground mist: densest below the mist level, thinning upward. Averaged between
    // the camera and the point.
    let m0 = exp(-max(0.0 - G.mist.z, 0.0) / G.mist.y);
    let m1 = exp(-max(rel.y - G.mist.z, 0.0) / G.mist.y);
    let density = G.fog.x + G.mist.x * (m0 + m1) * 0.5;
    let amount = 1.0 - exp(-dist * density);
    // Fog takes the colour of the sky at the horizon in that direction.
    let dir = normalize(vec3<f32>(rel.x, abs(rel.y) * 0.15 + dist * 0.02, rel.z));
    return mix(color, sky_color(dir), amount);
}

fn ambient(n: vec3<f32>) -> vec3<f32> {
    let sky = mix(G.sky_horizon.rgb, G.sky_zenith.rgb, 0.6) * (0.55 + 0.45 * n.y) * 0.8;
    // Light bounced off the ground, tinted warm, scaled by how much sun there is.
    // Reaches sideways and downward faces, which the sky lights least.
    let bounce = G.sun_color.rgb * vec3<f32>(0.20, 0.17, 0.12) * (0.65 - 0.35 * n.y)
        * max(G.sun_dir.y, 0.0);
    return sky + bounce;
}

// Fraction of direct sunlight reaching a point: cascaded shadow maps with a small
// filter, then cloud shadow.
fn sun_visibility(rel: vec3<f32>, n: vec3<f32>) -> f32 {
    let depth = dot(rel, G.cam_fwd.xyz);
    var vis = 1.0;
    if (depth < G.cascades.z) {
        var c = 0;
        if (depth >= G.cascades.y) {
            c = 2;
        } else if (depth >= G.cascades.x) {
            c = 1;
        }
        let texel = S.texel[c];
        // Push the lookup off the surface along its normal and toward the light so
        // flat faces do not shadow themselves.
        let ndl = max(dot(n, G.sun_dir.xyz), 0.0);
        let offset = rel + n * texel * 1.5 * (1.0 - ndl * 0.5) + G.sun_dir.xyz * texel * 0.5;
        let lp = S.light_vp[c] * vec4<f32>(offset, 1.0);
        let uv = lp.xy * vec2<f32>(0.5, -0.5) + 0.5;
        if (all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0))) {
            var sum = 0.0;
            let step = 1.0 / 2048.0;
            for (var dy = -1; dy <= 1; dy++) {
                for (var dx = -1; dx <= 1; dx++) {
                    let o = vec2<f32>(f32(dx), f32(dy)) * step;
                    sum += textureSampleCompareLevel(shadow_maps, shadow_sampler, uv + o, c, lp.z);
                }
            }
            vis = sum / 9.0;
            // The last cascade fades out rather than ending in a line.
            vis = mix(vis, 1.0, smoothstep(G.cascades.z * 0.85, G.cascades.z, depth));
        }
    }
    // Where the sun's ray through this point crosses the cloud layer.
    let to_cloud = (G.cascades.w - rel.y) / max(G.sun_dir.y, 0.05);
    let at_cloud = rel.xz + G.sun_dir.xz * to_cloud + G.cam_wrapped.xz;
    vis *= 1.0 - cloud_density(at_cloud) * 0.75;
    return vis;
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return pow(c, vec3<f32>(2.2));
}
