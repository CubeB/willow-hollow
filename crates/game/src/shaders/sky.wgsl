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
    let s = dot(dir, G.sun_dir.xyz);
    if (G.sun_color.w > 0.5) {
        // The sun.
        c += G.sun_color.rgb * (smoothstep(0.99985, 0.99995, s) * 30.0 + pow(max(s, 0.0), 900.0) * 1.5);
    } else {
        // The moon, and stars that fade near the horizon.
        c += G.sun_color.rgb * (smoothstep(0.99975, 0.9999, s) * 60.0 + pow(max(s, 0.0), 300.0) * 3.0);
        c += vec3<f32>(0.9, 0.95, 1.0) * stars(dir) * smoothstep(0.0, 0.25, dir.y) * 0.08;
    }

    // Clouds: a layer at a fixed height, seen where the view ray meets it.
    let plane_y = G.cascades.w;
    if (abs(dir.y) > 0.01 && (plane_y > 0.0) == (dir.y > 0.0)) {
        let t = plane_y / dir.y;
        let hit = dir.xz * t;
        let dist = length(vec3<f32>(hit.x, plane_y, hit.y));
        let density = cloud_density(hit + G.cam_wrapped.xz);
        // Lit from above by the sun, with the underside in sky-coloured shade.
        // At dusk and dawn the low sun catches the cloud on its own side of the sky.
        let toward = sunward(dir);
        let lit = G.sun_color.rgb * (0.9 + 0.3 * max(G.sun_dir.y, 0.0)) * (1.0 - density * 0.5)
            + G.sky_zenith.rgb * 0.8
            + G.sky_glow.rgb * (G.sky_glow.w * 0.7 * toward * toward);
        let fade = exp(-dist * G.fog.x * 2.5) * smoothstep(0.0, 0.03, abs(dir.y));
        c = mix(c, lit, clamp(density * 1.4, 0.0, 1.0) * fade);
    }
    return vec4<f32>(c, 1.0);
}
