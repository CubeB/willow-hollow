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
    // Pitch about the cube's own x, then roll about its own z, before the yaw.
    @location(7) tilt: vec2<f32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) rel: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) glow: f32,
};

// Roll about z, then pitch about x, then yaw about the vertical.
fn turn(v: vec3<f32>, yaw: f32, tilt: vec2<f32>) -> vec3<f32> {
    let cr = cos(tilt.y);
    let sr = sin(tilt.y);
    let a = vec3<f32>(v.x * cr - v.y * sr, v.x * sr + v.y * cr, v.z);
    let cp = cos(tilt.x);
    let sp = sin(tilt.x);
    let b = vec3<f32>(a.x, a.y * cp - a.z * sp, a.y * sp + a.z * cp);
    let c = cos(yaw);
    let s = sin(yaw);
    return vec3<f32>(b.x * c - b.z * s, b.y, b.x * s + b.z * c);
}

@vertex
fn vs(in: VIn) -> VOut {
    let turned = turn(in.corner * in.size, in.yaw, in.tilt);
    let n = turn(in.normal, in.yaw, in.tilt);
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
        // A lantern or a fire lights what stands by it, as it does the ground.
        let light = G.sun_color.rgb * ndl * sun_visibility(in.rel, in.normal) + ambient(in.normal) + local_lights(in.rel, in.normal);
        color = in.color.rgb * light;
    }
    return vec4<f32>(apply_fog(color, in.rel), in.color.a);
}
