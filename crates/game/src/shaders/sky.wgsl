
// The northern lights (see aurora.rs). x: how bright they are now, 0 to 1. y: where
// their curtains start in their slow folding, kept at 0 for now.
struct AuroraLights {
    a: vec4<f32>,
};
@group(1) @binding(0) var<uniform> AURORA: AuroraLights;

// Where along the ground's north-south the curtain stands, at east-west position x:
// a slow meander that drifts and folds over the minutes.
fn aurora_fold(x: f32, t: f32, k: f32) -> f32 {
    return 0.55 * sin(x * 0.42 + t * 0.021 + k) + 0.28 * sin(x * 1.07 - t * 0.037 + k * 2.3) + 0.5 * (fbm(vec2<f32>(x * 0.35 + t * 0.012, k)) - 0.5);
}

// Curtains of light hanging in the north of the sky, green at their lower edge and going
// to violet higher up, rippling. The sky is at infinity, so the curtains are sheets
// standing on a curving line on a plane a unit above the eye; a ray's points at a few
// heights through them add up to the glow, which gives the sheets their depth and their
// rays running up them.
fn aurora(dir: vec3<f32>) -> vec3<f32> {
    let strength = AURORA.a.x;
    if (strength <= 0.001 || dir.y <= 0.015) {
        return vec3<f32>(0.0);
    }
    let t = G.sun_dir.w;
    let k = AURORA.a.y;
    var sum = vec3<f32>(0.0);
    let steps = 14;
    for (var i = 0; i < steps; i++) {
        let hn = (f32(i) + 0.5) / f32(steps);
        let h = 1.0 + hn * 1.6;
        let p = dir.xz * (h / dir.y);
        // Toward the north (-z), a couple of curtains one behind the other.
        var lit = 0.0;
        for (var c = 0; c < 2; c++) {
            let cf = f32(c);
            let line = -2.4 - cf * 1.3 + aurora_fold(p.x + cf * 7.0, t, k + cf * 5.3);
            let d = (p.y - line) / (0.16 + 0.08 * cf);
            // Brightening and dimming along its length, in waves that run along it,
            // and the fine rays up it.
            let along = 0.55 + 0.45 * sin(p.x * 1.6 - t * 0.35 + cf * 2.0 + k);
            let rays = 0.45 + 0.55 * vnoise(vec2<f32>(p.x * 9.0 + t * 0.08, cf * 13.0 + k));
            lit += exp(-d * d) * along * rays * (1.0 - 0.45 * cf);
        }
        // A sharp lower edge, fading away above.
        let profile = smoothstep(0.0, 0.07, hn) * exp(-hn * 2.4);
        let colour = mix(vec3<f32>(0.12, 1.0, 0.38), vec3<f32>(0.55, 0.16, 0.85), smoothstep(0.3, 0.95, hn));
        sum += colour * lit * profile;
    }
    // Low in the sky, through more air, the lights thin into the haze.
    let low = smoothstep(0.015, 0.12, dir.y);
    return sum / f32(steps) * strength * low * 1.1;
}
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

// A sparse field of stars, wheeling about the pole the sun and moon go round: once a
// night, and once more over the year.
fn stars(view: vec3<f32>) -> f32 {
    let c = cos(G.sun_pos.w);
    let s = sin(G.sun_pos.w);
    let dir = vec3<f32>(view.x * c + view.y * s, view.y * c - view.x * s, view.z);
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
        // They come out as the dusk fades, and shine bright on a dark night.
        c += vec3<f32>(0.9, 0.95, 1.0) * stars(dir) * smoothstep(0.0, 0.25, dir.y) * 0.14 * darkness();
        // And on some nights in the cold country, the northern lights.
        c += aurora(dir);
    }
    return vec4<f32>(c, 1.0);
}
