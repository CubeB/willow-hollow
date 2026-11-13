// Depth-only pass of the moving models (people, boats, the glider) from the sun, one
// cascade at a time: the cubes of particles.wgsl, turned the same way.

struct Cascade {
    light_vp: mat4x4<f32>,
    season: vec4<f32>,
};

@group(0) @binding(0) var<uniform> C: Cascade;

struct VIn {
    @location(0) corner: vec3<f32>,
    @location(2) pos: vec3<f32>,
    @location(3) yaw: f32,
    @location(4) size: vec3<f32>,
    @location(7) tilt: vec2<f32>,
};

@vertex
fn vs(in: VIn) -> @builtin(position) vec4<f32> {
    // Roll about z, then pitch about x, then yaw about the vertical.
    let v = in.corner * in.size;
    let cr = cos(in.tilt.y);
    let sr = sin(in.tilt.y);
    let a = vec3<f32>(v.x * cr - v.y * sr, v.x * sr + v.y * cr, v.z);
    let cp = cos(in.tilt.x);
    let sp = sin(in.tilt.x);
    let b = vec3<f32>(a.x, a.y * cp - a.z * sp, a.y * sp + a.z * cp);
    let c = cos(in.yaw);
    let s = sin(in.yaw);
    let turned = vec3<f32>(b.x * c - b.z * s, b.y, b.x * s + b.z * c);
    return C.light_vp * vec4<f32>(in.pos + turned, 1.0);
}
