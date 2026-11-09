// Shared by every scene shader (prepended at pipeline creation).

struct Globals {
    view_proj: mat4x4<f32>,
    // Camera basis scaled so that fwd + ndc.x * right + ndc.y * up is the view ray.
    cam_right: vec4<f32>,
    cam_up: vec4<f32>,
    cam_fwd: vec4<f32>,
    // xyz: direction toward the sun. w: time in seconds.
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    sky_zenith: vec4<f32>,
    sky_horizon: vec4<f32>,
    // x: fog density per voxel. y: horizontal distance at which voxel terrain hands
    // over to the far view.
    fog: vec4<f32>,
    // xyz: camera world position, wrapped to keep it small. Adding it to a
    // camera-relative position gives a stable world-anchored coordinate.
    cam_wrapped: vec4<f32>,
};

@group(0) @binding(0) var<uniform> G: Globals;

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
    let amount = 1.0 - exp(-dist * G.fog.x);
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

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return pow(c, vec3<f32>(2.2));
}
