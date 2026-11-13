struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VOut {
    // One triangle covering the screen, at the far plane (reverse-Z depth 0).
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    var out: VOut;
    out.clip = vec4<f32>(p, 0.0, 1.0);
    out.ndc = p;
    return out;
}

// A sparse field of stars, fixed to the sky.
fn stars(dir: vec3<f32>) -> f32 {
    let cell = vec3<i32>(floor(dir * 160.0));
    let h = hash3(cell);
    if (h > 0.015) {
        return 0.0;
    }
    // Where in its cell the star sits, and how bright.
    let centre = (vec3<f32>(cell) + vec3<f32>(hash3(cell + 11), hash3(cell + 23), hash3(cell + 37))) / 160.0;
    let d = length(normalize(centre) - dir) * 160.0;
    let twinkle = 0.75 + 0.25 * sin(G.sun_dir.w * 3.0 + h * 400.0);
    return smoothstep(0.5, 0.0, d) * (0.4 + hash3(cell + 5) * 1.2) * twinkle;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let dir = normalize(G.cam_fwd.xyz + in.ndc.x * G.cam_right.xyz + in.ndc.y * G.cam_up.xyz);
    var c = sky_color(dir);

    // Clouds: blocks standing on a grid at a fixed height (see cloud_trace), out to where
    // the haze has all but swallowed them. A cloud hides the sun, moon and stars behind
    // it, however far off and hazy it is.
    let cloud = cloud_seen(dir, 4.0 / max(G.fog.x * 2.5, 1e-7));
    if (cloud.w >= 0.0) {
        let fade = exp(-cloud.w * G.fog.x * 2.5 - haze_depth(dir * cloud.w, cloud.w) * 0.5);
        let hidden = fade * cloud_alpha;
        // The sun behind the thin edge of a cloud lights it up.
        let s = dot(dir, G.sun_dir.xyz);
        let behind = G.sun_color.rgb * (pow(max(s, 0.0), 300.0) * 2.0) * (1.0 - cloud_alpha * 0.6);
        return vec4<f32>(mix(c, cloud.rgb, hidden) + behind * fade, 1.0);
    }

    let s = dot(dir, G.sun_dir.xyz);
    if (G.sun_color.w > 0.5) {
        // The sun.
        c += G.sun_color.rgb * (smoothstep(0.99985, 0.99995, s) * 30.0 + pow(max(s, 0.0), 900.0) * 1.5);
    } else {
        // The moon, and stars that fade near the horizon.
        c += G.sun_color.rgb * (smoothstep(0.99975, 0.9999, s) * 60.0 + pow(max(s, 0.0), 300.0) * 3.0);
        c += vec3<f32>(0.9, 0.95, 1.0) * stars(dir) * smoothstep(0.0, 0.25, dir.y) * 0.08;
    }
    return vec4<f32>(c, 1.0);
}
