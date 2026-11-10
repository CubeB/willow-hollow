// The far view: continent-scale terrain, standing water and river ribbons drawn
// beyond the radius where voxels are loaded.

const LIFT_UNIT: f32 = 0.75;

struct VIn {
    @location(0) pos: vec3<f32>,
    // rgb: sRGB colour. a: below 0.4 land, with how far its forest stands above the
    // ground; ~0.5 standing water; 1 river ribbon.
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
    var rel = in.pos + in.offset;
    if (in.color.a < 0.4) {
        let dist = length(rel.xz);
        // Close to where the tree models end, forests are the models, so the surface
        // only rises to forest height out beyond them.
        let lift = in.color.a * 255.0 * LIFT_UNIT;
        rel.y += lift * smoothstep(G.cam_wrapped.w * 0.85, G.cam_wrapped.w, dist);
        // Just past the voxel terrain the surface keeps below it, so that the coarsest
        // ring, whose steps round down, always meets it from above.
        // Land just above the sea stays above it, or the shore would drown.
        let sea = 0.05 - G.cam_wrapped.y;
        let sunk = rel.y - (1.0 - smoothstep(G.fog.y, G.fog.y + 4000.0, dist)) * 3.0;
        rel.y = max(sunk, min(rel.y, sea));
    }
    out.rel = rel;
    var depth_pos = rel;
    if (in.color.a > 0.75) {
        // A ribbon lies on terrain too coarse to have a channel cut in it. Sliding it
        // toward the eye along the view ray leaves it in the same place on screen but
        // lets it win the depth test against the ground just around it.
        let dist = length(rel);
        depth_pos = rel * (1.0 - min(9.6 * VOXELS_PER_METRE, dist * 0.05) / max(dist, 1.0));
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
    if (in.kind > 0.42) {
        // Water, shaded as the voxel water is but opaque: nothing is drawn beneath.
        let view = normalize(in.rel);
        let fresnel = 0.03 + 0.97 * pow(1.0 - max(dot(-view, up), 0.0), 5.0);
        let body = in.color * (G.sun_color.rgb * max(G.sun_dir.y, 0.0) * 0.5 + ambient(up));
        color = mix(body, sky_color(reflect(view, up)), fresnel);
        // Still water freezes over (rivers too: there is no flow to tell them apart).
        let ice = ice_amount(in.rel.xz + G.cam_wrapped.xz);
        color = mix(color, vec3<f32>(0.62, 0.78, 0.88) * (G.sun_color.rgb * 0.7 * max(G.sun_dir.y, 0.0) + ambient(up)), ice);
    } else {
        // Voxel terrain is all flat tops and vertical sides. From far away a slope
        // reads as a blend of the two, the sides counting for more the steeper it is.
        let steep = length(in.grad);
        let side_n = normalize(vec3<f32>(-in.grad.x, 0.0, -in.grad.y) + vec3<f32>(1e-5, 0.0, 0.0));
        let vis = sun_visibility(in.rel, up);
        let top = G.sun_color.rgb * max(G.sun_dir.y, 0.0) * vis + ambient(up);
        let side = G.sun_color.rgb * max(dot(side_n, G.sun_dir.xyz), 0.0) * 0.8 * vis + ambient(side_n) * 0.7;
        // The season: a far forest turns and goes bare, grass dries, snow settles.
        let world = in.rel.xz + G.cam_wrapped.xz;
        var base = season_far_ground(in.color, in.kind > 0.002, world, G.season.x);
        base = ground_weather(base, up, in.rel, 0.5, false, false);
        color = base * mix(top, side, clamp(steep * 0.5, 0.0, 0.6));
        // Stepped ground also carries the shadowed edges of all its steps.
        color *= 1.0 - clamp(steep * 0.9, 0.0, 0.35);
    }
    return vec4<f32>(apply_fog(color, in.rel), 1.0);
}
