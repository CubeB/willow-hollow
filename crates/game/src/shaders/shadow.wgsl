// Depth-only pass of the voxel terrain from the sun, one cascade at a time.

struct Cascade {
    light_vp: mat4x4<f32>,
    // x: the date. y: 1 if this cascade lets flecks of sun through the leaves.
    season: vec4<f32>,
    // xy: how far the sun's ray carries along x and z for each voxel it falls. z: the
    // level of the ground it is followed down to, relative to the camera.
    lean: vec4<f32>,
    // xy: the camera's wrapped world x and z.
    camera: vec4<f32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    // xy: where on the ground the sun's ray through this point comes down, in voxels.
    // z: 1 on the leaves of a tree, where flecks of sun get through.
    @location(0) fleck: vec3<f32>,
};

const FLAG_FOLIAGE: u32 = 8u;

@group(0) @binding(0) var<uniform> C: Cascade;

struct VIn {
    @location(0) a: u32,
    @location(1) b: u32,
    @location(2) offset: vec3<f32>,
    @location(3) origin: vec3<i32>,
    // Per chunk: voxels per cell of its mesh.
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
fn vs(in: VIn) -> VOut {
    var out: VOut;
    let local = vertex_local(in.a);
    let kind = (in.b >> 29u) & 7u;
    let rel = local * in.scale + in.offset;
    let leaf = ((in.b >> 24u) & FLAG_FOLIAGE) != 0u && kind >= PLANT_DECIDUOUS && kind <= PLANT_AUTUMN;
    out.fleck = vec3<f32>(rel.xz - C.lean.xy * (rel.y - C.lean.z) + C.camera.xy, select(0.0, C.season.y, leaf));
    if ((kind == PLANT_DECIDUOUS || kind == PLANT_AUTUMN) && (C.season.x < 0.27 || C.season.x > 0.40)) {
        // The same voxel the terrain shader names, so a leaf and its shadow fall together.
        let vox = in.origin / i32(in.scale) + vec3<i32>(floor(local)) - vertex_corner(in.a);
        if (season_gone(C.season.x, kind, vox, season_tag(in.b))) {
            out.clip = vec4<f32>(0.0, 0.0, 2.0, 1.0);
            return out;
        }
    }
    out.clip = C.light_vp * vec4<f32>(rel, 1.0);
    return out;
}

fn fleck_noise(p: vec2<f32>) -> f32 {
    let i = vec2<i32>(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = season_hash(vec3<i32>(i.x, 91, i.y));
    let b = season_hash(vec3<i32>(i.x + 1, 91, i.y));
    let c = season_hash(vec3<i32>(i.x, 91, i.y + 1));
    let d = season_hash(vec3<i32>(i.x + 1, 91, i.y + 1));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// A crown of leaves is not a solid lid: here and there the sun finds a way down through
// it. The leaves are whole cubes, so the gaps are made here, by leaving leaves out of
// the shadow map wherever the sun's ray through them comes down on a patch of ground
// picked out by a pattern laid over it. Every leaf along one ray gives the same answer,
// so the gap goes right through the crown, and the flecks lie still on the ground as
// the sun moves.
@fragment
fn fs(in: VOut) {
    if (in.fleck.z > 0.5) {
        let p = in.fleck.xy;
        // Gaps a step or so across, more of them where the crowns are thin.
        let thin = fleck_noise(p / 17.0);
        let gap = fleck_noise(p / 2.1) * 0.65 + fleck_noise(p / 0.9 + vec2<f32>(37.0, 11.0)) * 0.35;
        if (gap > 0.69 - 0.12 * thin) {
            discard;
        }
    }
}
