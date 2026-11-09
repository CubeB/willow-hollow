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

@vertex
fn vs(in: VIn) -> VOut {
    var out: VOut;
    var local = vec3<f32>(f32(in.a & 63u), f32((in.a >> 6u) & 63u), f32((in.a >> 12u) & 63u));
    out.face = (in.a >> 18u) & 7u;
    out.ao = f32((in.a >> 21u) & 3u) / 3.0;
    out.flags = in.b >> 24u;
    out.local = local;
    var rel = local + in.offset;
    if ((out.flags & FLAG_WATER) != 0u && out.face == 2u) {
        // Water surfaces sit a little below the top of their voxel.
        rel.y -= 0.125;
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
    let n = face_normal(in.face);
    // The voxel this fragment belongs to: step half a voxel inward from the face.
    let voxel = vec3<i32>(floor(in.local - n * 0.5)) + in.origin;
    let jitter = f32((in.flags >> 1u) & 3u);
    let h = hash3(voxel);
    let h2 = hash3(voxel.zxy + vec3<i32>(17, 31, 59));
    // Each cube gets its own slight brightness and warmth so merged faces still read as cubes.
    var base = in.color * (1.0 + (h - 0.5) * 0.09 * jitter);
    base *= vec3<f32>(1.0 + (h2 - 0.5) * 0.03 * jitter, 1.0, 1.0 - (h2 - 0.5) * 0.03 * jitter);

    let ao = in.ao * in.ao * 0.75 + 0.25;
    let ndl = max(dot(n, G.sun_dir.xyz), 0.0);
    let direct = G.sun_color.rgb * ndl * mix(0.55, 1.0, in.ao);
    let light = direct + ambient(n) * ao;
    let color = apply_fog(base * light, in.rel);
    return vec4<f32>(color, 1.0);
}

@fragment
fn fs_water(in: VOut) -> @location(0) vec4<f32> {
    if (beyond_voxels(in.rel)) {
        discard;
    }
    let n = face_normal(in.face);
    let view = normalize(in.rel);
    let fresnel = 0.03 + 0.97 * pow(1.0 - max(dot(-view, n), 0.0), 5.0);
    let reflected = sky_color(reflect(view, n));
    let ndl = max(dot(n, G.sun_dir.xyz), 0.0);
    let body = in.color * (G.sun_color.rgb * ndl * 0.5 + ambient(n));
    let color = mix(body, reflected, fresnel);
    let alpha = mix(0.82, 0.98, fresnel);
    return vec4<f32>(apply_fog(color, in.rel), alpha);
}
