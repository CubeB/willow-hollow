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

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let dir = normalize(G.cam_fwd.xyz + in.ndc.x * G.cam_right.xyz + in.ndc.y * G.cam_up.xyz);
    var c = sky_color(dir);
    let s = dot(dir, G.sun_dir.xyz);
    c += G.sun_color.rgb * (smoothstep(0.99985, 0.99995, s) * 30.0 + pow(max(s, 0.0), 900.0) * 1.5);
    return vec4<f32>(c, 1.0);
}
