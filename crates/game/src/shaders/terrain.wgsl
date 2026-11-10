struct VIn {
    @location(0) a: u32,
    @location(1) b: u32,
    // Per chunk: position of the chunk origin relative to the camera, and in world voxels.
    @location(2) offset: vec3<f32>,
    @location(3) origin: vec3<i32>,
    // Per chunk: voxels per cell of its mesh, and the distances between which it is shown.
    @location(4) scale: f32,
    @location(5) band: vec2<f32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) rel: vec3<f32>,
    @location(1) local: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) ao: f32,
    @location(4) @interpolate(flat) face: u32,
    @location(5) @interpolate(flat) origin: vec3<i32>,
    @location(6) @interpolate(flat) flags: u32,
    @location(7) @interpolate(flat) band: vec2<f32>,
    // Water only: the current, metres per second along x and z, and how much of a river
    // (rather than a lake or the sea) the water is, 0..1. Zero for everything else.
    @location(8) current: vec3<f32>,
};

// A vertex's position in its chunk, in voxels (see `Vertex` in voxel/src/mesh.rs): whole
// voxels, quarters `q`, and per axis a bit that, with `q` set, moves the corner an eighth
// of a voxel toward the nearer face of its voxel (q 1, 2, 3 -> 1/8, 5/8, 7/8), for skins
// half a cell thick. shadow.wgsl has the same two functions.
fn vertex_local(a: u32) -> vec3<f32> {
    let q = vec3<u32>((a >> 23u) & 3u, (a >> 25u) & 3u, (a >> 27u) & 3u);
    let bit = vec3<u32>((a >> 29u) & 1u, (a >> 30u) & 1u, (a >> 31u) & 1u);
    let half = (bit == vec3<u32>(1u)) & (q != vec3<u32>(0u));
    let nudge = select(vec3<f32>(-0.125), vec3<f32>(0.125), q >= vec3<u32>(2u));
    return vec3<f32>(f32(a & 63u), f32((a >> 6u) & 63u), f32((a >> 12u) & 63u))
        + vec3<f32>(q) * 0.25 + select(vec3<f32>(0.0), nudge, half);
}

// Per axis, 1 where a corner on a voxel boundary belongs to the voxel before it.
fn vertex_corner(a: u32) -> vec3<i32> {
    let q = vec3<u32>((a >> 23u) & 3u, (a >> 25u) & 3u, (a >> 27u) & 3u);
    let bit = vec3<i32>(i32((a >> 29u) & 1u), i32((a >> 30u) & 1u), i32((a >> 31u) & 1u));
    return select(vec3<i32>(0), bit, q == vec3<u32>(0u));
}

// True if this ring of terrain has given way to another at this distance. Each pixel is
// drawn by exactly one ring.
fn outside_band(rel: vec3<f32>, band: vec2<f32>) -> bool {
    let d = length(rel.xz);
    return d < band.x || d >= band.y;
}

fn face_normal(face: u32) -> vec3<f32> {
    switch face {
        case 0u: { return vec3<f32>(1.0, 0.0, 0.0); }
        case 1u: { return vec3<f32>(-1.0, 0.0, 0.0); }
        case 2u: { return vec3<f32>(0.0, 1.0, 0.0); }
        case 3u: { return vec3<f32>(0.0, -1.0, 0.0); }
        case 4u: { return vec3<f32>(0.0, 0.0, 1.0); }
        default: { return vec3<f32>(0.0, 0.0, -1.0); }
    }
}

const FLAG_WATER: u32 = 1u;
const FLAG_FOLIAGE: u32 = 8u;
const FLAG_EMISSIVE: u32 = 16u;
const NEAR: f32 = 0.08 * VOXELS_PER_METRE;

@vertex
fn vs(in: VIn) -> VOut {
    var out: VOut;
    var local = vertex_local(in.a);
    out.face = (in.a >> 18u) & 7u;
    out.ao = f32((in.a >> 21u) & 3u) / 3.0;
    out.flags = in.b >> 24u;
    out.local = local;
    out.band = in.band;
    var rel = local * in.scale + in.offset;
    if ((out.flags & FLAG_WATER) != 0u && out.face == 2u) {
        // Water surfaces sit a little below the top of their voxel.
        rel.y -= 0.125;
    }
    if ((out.flags & FLAG_FOLIAGE) != 0u) {
        // Leaves stir in the wind. The offset is a smooth function of world position,
        // so neighbouring leaf voxels move together and no gaps open between them.
        let world = (vec3<f32>(in.origin) + local * in.scale) / VOXELS_PER_METRE;
        let t = G.sun_dir.w;
        let wave = sin(world.x * 0.4375 + world.z * 0.3375 + world.y * 0.25 + t * 1.7)
            + 0.5 * sin(world.x * 1.125 - world.z * 0.75 + t * 2.9);
        let strength = length(G.wind.xz) / VOXELS_PER_METRE * 0.0375;
        rel += vec3<f32>(G.wind.x, 0.0, G.wind.z) * (0.02 + wave * strength)
            + vec3<f32>(0.0, wave * strength * 0.4 * VOXELS_PER_METRE, 0.0);
    }
    out.rel = rel;
    out.clip = G.view_proj * vec4<f32>(rel, 1.0);
    // Plants that are gone this season (fallen leaves, dead flowers, tufts under snow) are
    // dropped here: every corner of the face names the same voxel, so the whole face goes.
    let kind = (in.b >> 29u) & 7u;
    if (kind != 0u && kind != PLANT_EVERGREEN && kind < 6u) {
        var vox = in.origin / i32(in.scale) + vec3<i32>(floor(local)) - vertex_corner(in.a);
        if (kind >= PLANT_GRASS) {
            vox.y = 0;
        }
        var gone = season_gone(G.season.x, kind, vox);
        if (kind == PLANT_GRASS && !gone) {
            gone = plant_buried(rel, season_hash(vox + vec3<i32>(5, 0, 9)));
        }
        if (gone) {
            out.clip = vec4<f32>(0.0, 0.0, 2.0, 1.0);
        }
    }
    let rgb = vec3<f32>(f32(in.b & 255u), f32((in.b >> 8u) & 255u), f32((in.b >> 16u) & 255u));
    out.color = srgb_to_linear(rgb / 255.0);
    // Hashed per cell, so a coarse chunk counts its cells from its own corner.
    out.origin = in.origin / i32(in.scale);
    return out;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    if (outside_band(in.rel, in.band)) {
        discard;
    }
    return shade(in);
}

// For chunks wholly inside their ring. Without a discard the GPU can settle depth
// before running the shader, so hidden surfaces cost nothing.
@fragment
fn fs_inside(in: VOut) -> @location(0) vec4<f32> {
    return shade(in);
}

fn shade(in: VOut) -> vec4<f32> {
    let n = face_normal(in.face);
    // The voxel this fragment belongs to: step half a voxel inward from the face.
    let voxel = vec3<i32>(floor(in.local - n * 0.5)) + in.origin;
    let jitter = f32((in.flags >> 1u) & 3u);
    let h = hash3(voxel);
    let h2 = hash3(voxel.zxy + vec3<i32>(17, 31, 59));
    // Each cube gets its own slight brightness and warmth so merged faces still read as cubes.
    var base = in.color * (1.0 + (h - 0.5) * 0.09 * jitter);
    base *= vec3<f32>(1.0 + (h2 - 0.5) * 0.03 * jitter, 1.0, 1.0 - (h2 - 0.5) * 0.03 * jitter);
    // The season: leaf colour and fall, dry or wintry grass.
    base = season_apply(base, voxel, (in.flags >> 5u) & 7u, G.season.x);
    base = ground_weather(base, n, in.rel, h, (in.flags & FLAG_FOLIAGE) != 0u, (in.flags & FLAG_EMISSIVE) != 0u, ((in.flags >> 5u) & 7u) == 6u, fract(in.local.y));

    let ao = in.ao * in.ao * 0.75 + 0.25;
    let ndl = max(dot(n, G.sun_dir.xyz), 0.0);
    let direct = G.sun_color.rgb * ndl * sun_visibility(in.rel, n) * mix(0.7, 1.0, in.ao);
    var light = direct + ambient(n) * ao + local_lights(in.rel, n) * ao;
    if ((in.flags & FLAG_EMISSIVE) != 0u) {
        // Lanterns and embers: faint by day, bright enough to bloom after dark.
        light += vec3<f32>(mix(14.0, 1.2, G.sun_color.w));
    }
    let color = apply_fog(base * light, in.rel);
    return vec4<f32>(color, 1.0);
}

// --- Water ---------------------------------------------------------------------------

@group(1) @binding(0) var scene_depth: texture_depth_2d;

// Fastest current a water vertex carries, metres per second (MAX_CURRENT in mesh.rs).
const MAX_CURRENT: f32 = 2.0;
// Seconds the pattern on running water is carried downstream before it starts afresh.
const FLOW_CYCLE: f32 = 1.6;

// The current a water vertex carries in `a` bits 23..=31 (see mesh.rs): direction in 32
// steps from +X toward +Z, speed in 8 steps spaced as (step / 7)^1.5, and a river bit.
// Returns metres per second along x and z, and 1 for a river.
fn water_current(a: u32) -> vec3<f32> {
    let angle = f32((a >> 23u) & 31u) * (6.2831853 / 32.0);
    let step = f32((a >> 28u) & 7u) / 7.0;
    let speed = MAX_CURRENT * step * sqrt(step);
    return vec3<f32>(cos(angle) * speed, sin(angle) * speed, f32(a >> 31u));
}

// The water's own vertex shader. Water faces carry their current where other faces keep
// quarter-voxel fractions and corner bits, and never sway or go with the season.
@vertex
fn vs_water(in: VIn) -> VOut {
    var out: VOut;
    let local = vec3<f32>(f32(in.a & 63u), f32((in.a >> 6u) & 63u), f32((in.a >> 12u) & 63u));
    out.face = (in.a >> 18u) & 7u;
    out.ao = f32((in.a >> 21u) & 3u) / 3.0;
    out.flags = in.b >> 24u;
    out.local = local;
    out.band = in.band;
    out.current = water_current(in.a);
    var rel = local * in.scale + in.offset;
    if (out.face == 2u) {
        // Water surfaces sit a little below the top of their voxel.
        rel.y -= 0.125;
    }
    out.rel = rel;
    out.clip = G.view_proj * vec4<f32>(rel, 1.0);
    let rgb = vec3<f32>(f32(in.b & 255u), f32((in.b >> 8u) & 255u), f32((in.b >> 16u) & 255u));
    out.color = srgb_to_linear(rgb / 255.0);
    out.origin = in.origin / i32(in.scale);
    return out;
}

// Still water: two faint sets of ripples drifting slowly across each other, so the
// surface shivers without going anywhere.
fn still_height(p: vec2<f32>, t: f32) -> f32 {
    let q1 = p + vec2<f32>(t * 0.12, t * 0.05);
    let q2 = p * 1.9 - vec2<f32>(t * 0.09, -t * 0.14);
    return vnoise(q1) * 0.6 + vnoise(q2) * 0.4;
}

// How far the pattern on running water has been carried in one of the two phases of
// the flow (`phase` 0 or 0.5), metres. Carried for ever it would smear wherever the
// current turns or changes speed, so each phase starts afresh every cycle, somewhere
// new in the pattern so it never visibly repeats, while the other is at its strongest.
fn flow_shift(flow: vec2<f32>, t: f32, phase: f32) -> vec2<f32> {
    let c = t / FLOW_CYCLE + phase;
    let jump = fract(floor(c) * vec2<f32>(0.618, 0.382) + phase) * vec2<f32>(37.0, 23.0);
    return jump - flow * (fract(c) * FLOW_CYCLE);
}

// How much the second phase shows: none while it starts afresh, all while the first does.
fn flow_blend(t: f32) -> f32 {
    return abs(fract(t / FLOW_CYCLE) * 2.0 - 1.0);
}

// Running water: finer ripples, carried downstream with the current.
fn running_height(p: vec2<f32>, flow: vec2<f32>, t: f32) -> f32 {
    let q0 = p + flow_shift(flow, t, 0.0);
    let q1 = p + flow_shift(flow, t, 0.5);
    let a = vnoise(q0 * 1.3) * 0.6 + vnoise(q0 * 2.7 + vec2<f32>(9.1, 4.7)) * 0.4;
    let b = vnoise(q1 * 1.3) * 0.6 + vnoise(q1 * 2.7 + vec2<f32>(9.1, 4.7)) * 0.4;
    return mix(a, b, flow_blend(t));
}

// Height of the ripples at `p` (metres) on water running at `flow` (metres per second).
fn ripple_height(p: vec2<f32>, flow: vec2<f32>, t: f32) -> f32 {
    let run = smoothstep(0.03, 0.3, length(flow));
    var h = 0.0;
    if (run < 1.0) {
        // Calm: lower than the ripples on a current.
        h += still_height(p, t) * (1.0 - run) * 0.6;
    }
    if (run > 0.0) {
        h += running_height(p, flow, t) * run;
    }
    return h;
}

// Small moving ripples on the water surface, as a normal in world space. Faster water
// is rougher.
fn ripple_normal(world_xz: vec2<f32>, flow: vec2<f32>, t: f32) -> vec3<f32> {
    let p = world_xz / VOXELS_PER_METRE;
    let d = 0.08;
    let dx = ripple_height(p + vec2<f32>(d, 0.0), flow, t) - ripple_height(p - vec2<f32>(d, 0.0), flow, t);
    let dz = ripple_height(p + vec2<f32>(0.0, d), flow, t) - ripple_height(p - vec2<f32>(0.0, d), flow, t);
    let rough = 1.4 * (1.0 + 0.3 * length(flow));
    return normalize(vec3<f32>(-dx * rough, 1.0, -dz * rough));
}

// Foam on fast water, 0..1: streaks drawn out along the current and carried with it.
// The noise is smeared downstream by averaging it along the flow, which needs no turning
// of the pattern to the current's direction.
fn foam_streaks(world_xz: vec2<f32>, flow: vec2<f32>, t: f32) -> f32 {
    let speed = length(flow);
    if (speed < 0.7) {
        return 0.0;
    }
    let p = world_xz / VOXELS_PER_METRE;
    let along = flow / speed * 0.75;
    let q0 = (p + flow_shift(flow, t, 0.0)) * 2.5;
    let q1 = (p + flow_shift(flow, t, 0.5)) * 2.5;
    var a = 0.0;
    var b = 0.0;
    for (var k = 0; k < 4; k++) {
        a += vnoise(q0 - along * f32(k));
        b += vnoise(q1 - along * f32(k));
    }
    let streak = mix(a, b, flow_blend(t)) * 0.25;
    return smoothstep(0.6, 0.78, streak) * smoothstep(0.7, 1.6, speed);
}

@fragment
fn fs_water(in: VOut) -> @location(0) vec4<f32> {
    if (outside_band(in.rel, in.band)) {
        discard;
    }
    let n_face = face_normal(in.face);
    let view = normalize(in.rel);
    let world_xz = in.rel.xz + G.cam_wrapped.xz;
    let t = G.sun_dir.w;

    // How much water the view ray passes through before hitting the bed, from the
    // depth of what was drawn behind this surface (reverse-Z: depth = near / dist).
    let behind = textureLoad(scene_depth, vec2<i32>(in.clip.xy), 0);
    let dist_behind = NEAR / max(behind, 1e-6);
    let dist_here = dot(in.rel, G.cam_fwd.xyz);
    let thickness = max(dist_behind - dist_here, 0.0) * max(abs(view.y), 0.25);

    var color: vec3<f32>;
    var alpha: f32;
    if (in.face == 2u) {
        // Ripples are far finer than a pixel out in the coarser rings, and would only
        // sparkle there.
        let calm = smoothstep(80.0 * VOXELS_PER_METRE, 300.0 * VOXELS_PER_METRE, length(in.rel.xz));
        let flow = in.current.xy;
        let n = normalize(mix(ripple_normal(world_xz, flow, t), vec3<f32>(0.0, 1.0, 0.0), calm));
        let fresnel = 0.03 + 0.97 * pow(1.0 - max(dot(-view, n), 0.0), 5.0);
        let reflected = sky_color(reflect(view, n));
        // Clear and bright-bedded in the shallows, deep blue-green further out.
        let shallow = in.color * 1.35;
        let deep = vec3<f32>(0.02, 0.09, 0.14);
        let body = mix(shallow, deep, 1.0 - exp(-thickness / VOXELS_PER_METRE * 0.2))
            * (G.sun_color.rgb * 0.5 * max(G.sun_dir.y, 0.0) + ambient(n_face));
        color = mix(body, reflected, fresnel);
        alpha = mix(1.0 - exp(-thickness / VOXELS_PER_METRE * 0.5625), 1.0, fresnel);
        // Sun glitter.
        let glint = pow(max(dot(reflect(view, n), G.sun_dir.xyz), 0.0), 240.0);
        color += G.sun_color.rgb * glint * 3.0 * sun_visibility(in.rel, n_face) * (1.0 - calm);
        // Foam streaks on fast water.
        let foam = foam_streaks(world_xz, flow, t) * 0.6 * (1.0 - calm);
        if (foam > 0.0) {
            let up = vec3<f32>(0.0, 1.0, 0.0);
            color = mix(color, vec3<f32>(0.85) * (G.sun_color.rgb * 0.6 * max(G.sun_dir.y, 0.0) + ambient(up)), foam);
            alpha = mix(alpha, 0.95, foam);
        }
        // Further out, water looks the way the far view paints it: opaque, with no bed
        // showing through to mark the rings.
        let far = smoothstep(150.0 * VOXELS_PER_METRE, 300.0 * VOXELS_PER_METRE, length(in.rel.xz));
        let up = vec3<f32>(0.0, 1.0, 0.0);
        let far_fresnel = 0.03 + 0.97 * pow(1.0 - max(dot(-view, up), 0.0), 5.0);
        let far_body = in.color * (G.sun_color.rgb * max(G.sun_dir.y, 0.0) * 0.5 + ambient(up));
        color = mix(color, mix(far_body, sky_color(reflect(view, up)), far_fresnel), far);
        alpha = mix(alpha, 1.0, far);
        // In deep cold the surface is ice: pale, opaque and still.
        var ice = ice_amount(world_xz);
        // Running water freezes later than still water, and less: ice creeps out from
        // the slow water by the banks, and the fast middle of a river stays open.
        let swift = length(flow) / MAX_CURRENT;
        ice *= mix(1.0, smoothstep(0.55 + 0.35 * swift, 1.0, G.season.z) * (1.0 - 0.6 * swift), in.current.z);
        if (ice > 0.0) {
            let sheet = vec3<f32>(0.62, 0.78, 0.88) * (G.sun_color.rgb * 0.7 * max(G.sun_dir.y, 0.0) + ambient(n_face));
            color = mix(color, sheet, ice);
            alpha = mix(alpha, 1.0, ice);
        }
    } else {
        // A vertical face of water is a fall or the step of a cascade: white and streaky,
        // the streaks running downward.
        let world_y = (in.rel.y + G.cam_wrapped.y) / VOXELS_PER_METRE;
        let along = select(world_xz.x, world_xz.y, abs(n_face.x) > 0.5) / VOXELS_PER_METRE;
        let streak = vnoise(vec2<f32>(along * 2.0, world_y * 0.4375 + t * 6.0))
            * vnoise(vec2<f32>(along * 3.875 + 9.0, world_y * 0.875 + t * 9.0));
        let foam = smoothstep(0.12, 0.45, streak);
        let body = in.color * (G.sun_color.rgb * 0.4 + ambient(n_face));
        color = mix(body, vec3<f32>(0.85) * (G.sun_color.rgb * 0.6 + ambient(n_face)), foam * 0.85);
        alpha = mix(0.6, 0.95, foam);
    }
    return vec4<f32>(apply_fog(color, in.rel), alpha);
}
