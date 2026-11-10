struct VIn {
    @location(0) a: u32,
    @location(1) b: u32,
    // Per chunk: position of the chunk origin relative to the camera, and in world voxels.
    @location(2) offset: vec3<f32>,
    @location(3) origin: vec3<i32>,
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
};

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
    var local = vec3<f32>(f32(in.a & 63u), f32((in.a >> 6u) & 63u), f32((in.a >> 12u) & 63u))
        + vec3<f32>(f32((in.a >> 23u) & 3u), f32((in.a >> 25u) & 3u), f32((in.a >> 27u) & 3u)) * 0.25;
    out.face = (in.a >> 18u) & 7u;
    out.ao = f32((in.a >> 21u) & 3u) / 3.0;
    out.flags = in.b >> 24u;
    out.local = local;
    var rel = local + in.offset;
    if ((out.flags & FLAG_WATER) != 0u && out.face == 2u) {
        // Water surfaces sit a little below the top of their voxel.
        rel.y -= 0.125;
    }
    if ((out.flags & FLAG_FOLIAGE) != 0u) {
        // Leaves stir in the wind. The offset is a smooth function of world position,
        // so neighbouring leaf voxels move together and no gaps open between them.
        let world = (vec3<f32>(in.origin) + local) / VOXELS_PER_METRE;
        let t = G.sun_dir.w;
        let wave = sin(world.x * 0.4375 + world.z * 0.3375 + world.y * 0.25 + t * 1.7)
            + 0.5 * sin(world.x * 1.125 - world.z * 0.75 + t * 2.9);
        let strength = length(G.wind.xz) / VOXELS_PER_METRE * 0.0375;
        rel += vec3<f32>(G.wind.x, 0.0, G.wind.z) * (0.02 + wave * strength)
            + vec3<f32>(0.0, wave * strength * 0.4 * VOXELS_PER_METRE, 0.0);
    }
    out.rel = rel;
    out.clip = G.view_proj * vec4<f32>(rel, 1.0);
    let rgb = vec3<f32>(f32(in.b & 255u), f32((in.b >> 8u) & 255u), f32((in.b >> 16u) & 255u));
    out.color = srgb_to_linear(rgb / 255.0);
    out.origin = in.origin;
    return out;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    if (beyond_voxels(in.rel)) {
        discard;
    }
    return shade(in);
}

// For chunks wholly inside the voxel radius. Without a discard the GPU can settle depth
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
    let seasoned = season_apply(base, voxel, h, (in.flags & FLAG_FOLIAGE) != 0u, G.season.x);
    if (seasoned.a < 0.5) {
        discard;
    }
    base = seasoned.rgb;

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

fn ripple_height(p: vec2<f32>, t: f32) -> f32 {
    let q1 = p + vec2<f32>(t * 0.35, t * 0.22);
    let q2 = p * 1.9 - vec2<f32>(t * 0.27, -t * 0.31);
    return vnoise(q1) * 0.6 + vnoise(q2) * 0.4;
}

// Small moving ripples on the water surface, as a normal in world space.
fn ripple_normal(world_xz: vec2<f32>, t: f32) -> vec3<f32> {
    let p = world_xz / VOXELS_PER_METRE;
    let d = 0.08;
    let dx = ripple_height(p + vec2<f32>(d, 0.0), t) - ripple_height(p - vec2<f32>(d, 0.0), t);
    let dz = ripple_height(p + vec2<f32>(0.0, d), t) - ripple_height(p - vec2<f32>(0.0, d), t);
    return normalize(vec3<f32>(-dx * 1.4, 1.0, -dz * 1.4));
}

@fragment
fn fs_water(in: VOut) -> @location(0) vec4<f32> {
    if (beyond_voxels(in.rel)) {
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
        let n = ripple_normal(world_xz, t);
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
        color += G.sun_color.rgb * glint * 3.0 * sun_visibility(in.rel, n_face);
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
