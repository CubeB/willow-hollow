// Particles: small cubes, lit like terrain unless they glow.

struct VIn {
    // Unit cube vertex and its face normal.
    @location(0) corner: vec3<f32>,
    @location(1) normal: vec3<f32>,
    // Per particle.
    @location(2) pos: vec3<f32>,
    @location(3) yaw: f32,
    @location(4) size: vec3<f32>,
    @location(5) glow: f32,
    @location(6) color: vec4<f32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) rel: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) glow: f32,
};

@vertex
fn vs(in: VIn) -> VOut {
    let c = cos(in.yaw);
    let s = sin(in.yaw);
    let local = in.corner * in.size;
    let turned = vec3<f32>(local.x * c - local.z * s, local.y, local.x * s + local.z * c);
    let n = vec3<f32>(in.normal.x * c - in.normal.z * s, in.normal.y, in.normal.x * s + in.normal.z * c);
    var out: VOut;
    out.rel = in.pos + turned;
    out.clip = G.view_proj * vec4<f32>(out.rel, 1.0);
    out.normal = n;
    out.color = in.color;
    out.glow = in.glow;
    return out;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    var color: vec3<f32>;
    if (in.glow > 0.0) {
        color = in.color.rgb * in.glow;
    } else {
        let ndl = max(dot(in.normal, G.sun_dir.xyz), 0.0);
        let light = G.sun_color.rgb * ndl * sun_visibility(in.rel, in.normal) + ambient(in.normal);
        color = in.color.rgb * light;
    }
    return vec4<f32>(apply_fog(color, in.rel), in.color.a);
}
