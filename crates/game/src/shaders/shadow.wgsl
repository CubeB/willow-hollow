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
    // Per chunk: voxels per cell of its mesh; negative for a chunk's small plants.
    @location(4) scale: f32,
};

// A vertex's position in its chunk, in voxels: as in terrain.wgsl, which explains it.
fn vertex_local(a: u32) -> vec3<f32> {
    let q = vec3<u32>((a >> 23u) & 3u, (a >> 25u) & 3u, (a >> 27u) & 3u);
    let bit = vec3<u32>((a >> 29u) & 1u, (a >> 30u) & 1u, (a >> 31u) & 1u);
    let half = (bit == vec3<u32>(1u)) & (q != vec3<u32>(0u));
    let nudge = select(vec3<f32>(-0.125), vec3<f32>(0.125), q >= vec3<u32>(2u));
    return vec3<f32>(f32(a & 63u), f32((a >> 6u) & 63u), f32((a >> 12u) & 63u))
        + vec3<f32>(q) * 0.25 + select(vec3<f32>(0.0), nudge, half);
}

fn vertex_corner(a: u32) -> vec3<i32> {
    let q = vec3<u32>((a >> 23u) & 3u, (a >> 25u) & 3u, (a >> 27u) & 3u);
    let bit = vec3<i32>(i32((a >> 29u) & 1u), i32((a >> 30u) & 1u), i32((a >> 31u) & 1u));
    return select(vec3<i32>(0), bit, q == vec3<u32>(0u));
}

// Leaves that have fallen cast no shadow.
@vertex
fn vs(in: VIn) -> @builtin(position) vec4<f32> {
    var local = vertex_local(in.a);
    if (((in.b >> 24u) & 24u) == 24u) {
        // Washed ground keeps a colour's number where other faces keep fractions.
        local = vec3<f32>(f32(in.a & 63u), f32((in.a >> 6u) & 63u), f32((in.a >> 12u) & 63u));
    }
    let scale = abs(in.scale);
    if (in.scale < 0.0) {
        // Small plants past their reach cast nothing, as they are not drawn.
        let cell = vec3<i32>(floor(local)) - vertex_corner(in.a);
        if (length(in.offset.xz + vec2<f32>(cell.xz) + 0.5) >= PLANT_REACH * plant_reach(in.origin.xz + cell.xz)) {
            return vec4<f32>(0.0, 0.0, 2.0, 1.0);
        }
    }
    let kind = (in.b >> 29u) & 7u;
    if ((kind == PLANT_DECIDUOUS || kind == PLANT_AUTUMN) && (C.season.x < 0.27 || C.season.x > 0.40)) {
        // The same voxel the terrain shader names, so a leaf and its shadow fall together.
        let vox = in.origin / i32(scale) + vec3<i32>(floor(local)) - vertex_corner(in.a);
        if (season_gone(C.season.x, kind, vox, season_tag(in.b))) {
            return vec4<f32>(0.0, 0.0, 2.0, 1.0);
        }
    }
    return C.light_vp * vec4<f32>(local * scale + in.offset, 1.0);
}
