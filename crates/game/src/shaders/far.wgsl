// The far view: continent-scale terrain, standing water and river ribbons drawn
// beyond the radius where voxels are loaded.

struct VIn {
    @location(0) pos: vec3<f32>,
    // rgb: sRGB colour. a: 0 land, ~0.5 standing water, 1 river ribbon.
    @location(1) color: vec4<f32>,
    // Terrain gradient / 4.
    @location(2) grad: vec2<f32>,
    // Per continent: its origin corner relative to the camera.
    @location(3) offset: vec3<f32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) rel: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) grad: vec2<f32>,
    @location(3) kind: f32,
};

@vertex
fn vs(in: VIn) -> VOut {
    var out: VOut;
    let rel = in.pos + in.offset;
    out.rel = rel;
    var depth_pos = rel;
    if (in.color.a > 0.75) {
        // A ribbon lies on terrain too coarse to have a channel cut in it. Sliding it
        // toward the eye along the view ray leaves it in the same place on screen but
        // lets it win the depth test against the ground just around it.
        let dist = length(rel);
        depth_pos = rel * (1.0 - min(12.0, dist * 0.05) / max(dist, 1.0));
    }
    out.clip = G.view_proj * vec4<f32>(depth_pos, 1.0);
    out.color = srgb_to_linear(in.color.rgb);
    out.grad = in.grad * 4.0;
    out.kind = in.color.a;
    return out;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    // Nearer than this the voxel terrain is drawn instead.
    if (!beyond_voxels(in.rel)) {
        discard;
    }
    let up = vec3<f32>(0.0, 1.0, 0.0);
    var color: vec3<f32>;
    if (in.kind > 0.25) {
        // Water, shaded as the voxel water is but opaque: nothing is drawn beneath.
        let view = normalize(in.rel);
        let fresnel = 0.03 + 0.97 * pow(1.0 - max(dot(-view, up), 0.0), 5.0);
        let body = in.color * (G.sun_color.rgb * max(G.sun_dir.y, 0.0) * 0.5 + ambient(up));
        color = mix(body, sky_color(reflect(view, up)), fresnel);
    } else {
        // Voxel terrain is all flat tops and vertical sides. From far away a slope
        // reads as a blend of the two, the sides counting for more the steeper it is.
        let steep = length(in.grad);
        let side_n = normalize(vec3<f32>(-in.grad.x, 0.0, -in.grad.y) + vec3<f32>(1e-5, 0.0, 0.0));
        let top = G.sun_color.rgb * max(G.sun_dir.y, 0.0) + ambient(up);
        let side = G.sun_color.rgb * max(dot(side_n, G.sun_dir.xyz), 0.0) * 0.8 + ambient(side_n) * 0.7;
        color = in.color * mix(top, side, clamp(steep * 0.5, 0.0, 0.6));
        // Stepped ground also carries the shadowed edges of all its steps.
        color *= 1.0 - clamp(steep * 0.9, 0.0, 0.35);
    }
    return vec4<f32>(apply_fog(color, in.rel), 1.0);
}
