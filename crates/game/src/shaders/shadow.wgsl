// Depth-only pass of the voxel terrain from the sun, one cascade at a time.

struct Cascade {
    light_vp: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> C: Cascade;

struct VIn {
    @location(0) a: u32,
    @location(1) b: u32,
    @location(2) offset: vec3<f32>,
    @location(3) origin: vec3<i32>,
};

@vertex
fn vs(in: VIn) -> @builtin(position) vec4<f32> {
    let local = vec3<f32>(f32(in.a & 63u), f32((in.a >> 6u) & 63u), f32((in.a >> 12u) & 63u))
        + vec3<f32>(f32((in.a >> 23u) & 3u), f32((in.a >> 25u) & 3u), f32((in.a >> 27u) & 3u)) * 0.25;
    return C.light_vp * vec4<f32>(local + in.offset, 1.0);
}
