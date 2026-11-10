// Depth-only pass of the voxel terrain from the sun, one cascade at a time.

struct Cascade {
    light_vp: mat4x4<f32>,
    // x: the date.
    season: vec4<f32>,
};

@group(0) @binding(0) var<uniform> C: Cascade;

struct VIn {
    @location(0) a: u32,
    @location(1) b: u32,
    @location(2) offset: vec3<f32>,
    @location(3) origin: vec3<i32>,
    // Per chunk: voxels per cell of its mesh.
    @location(4) scale: f32,
};

// Leaves that have fallen cast no shadow.
@vertex
fn vs(in: VIn) -> @builtin(position) vec4<f32> {
    let local = vec3<f32>(f32(in.a & 63u), f32((in.a >> 6u) & 63u), f32((in.a >> 12u) & 63u))
        + vec3<f32>(f32((in.a >> 23u) & 3u), f32((in.a >> 25u) & 3u), f32((in.a >> 27u) & 3u)) * 0.25;
    let kind = (in.b >> 29u) & 7u;
    if ((kind == PLANT_DECIDUOUS || kind == PLANT_AUTUMN) && (C.season.x < 0.27 || C.season.x > 0.40)) {
        // The same voxel the terrain shader names, so a leaf and its shadow fall together.
        let corner = vec3<i32>(i32((in.a >> 29u) & 1u), i32((in.a >> 30u) & 1u), i32((in.a >> 31u) & 1u));
        let vox = in.origin / i32(in.scale) + vec3<i32>(floor(local)) - corner;
        if (season_gone(C.season.x, kind, vox)) {
            return vec4<f32>(0.0, 0.0, 2.0, 1.0);
        }
    }
    return C.light_vp * vec4<f32>(local * in.scale + in.offset, 1.0);
}
