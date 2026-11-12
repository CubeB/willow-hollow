// The far view: continent-scale terrain, standing water and river ribbons drawn
// beyond the radius where voxels are loaded.

const LIFT_UNIT: f32 = 0.75;
// The colour of lakes in the surface: materials::WATER, sRGB.
const LAKE_WATER: vec3<f32> = vec3<f32>(46.0, 112.0, 168.0) / 255.0;

struct VIn {
    @location(0) pos: vec3<f32>,
    // rgb: sRGB colour. a: below 0.4 land, with how far its forest stands above the
    // ground; ~0.5 standing water; 1 river ribbon.
    @location(1) color: vec4<f32>,
    // Terrain gradient / 4.
    @location(2) grad: vec2<f32>,
    // Per continent: its origin corner relative to the camera.
    @location(3) offset: vec3<f32>,
    // Land: rgb, sRGB colour out beyond the tree models, the forest seen as a canopy.
    // a: how far standing water rises above the ground, 128/255 at the shore.
    @location(4) far: vec4<f32>,
    // x: the biome, for the snow lying there. y: the plant kind of its forest's canopy
    // (voxel::plant), 0 with none. zw: its skylines toward sunrise and sunset.
    @location(5) place: vec4<u32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) rel: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) grad: vec2<f32>,
    @location(3) kind: f32,
    @location(4) far: vec3<f32>,
    // Above 0 under standing water; linear across a triangle, so 0 is the shore.
    @location(5) wet: f32,
    // The ground here: its biome's row of G.snow.
    @location(6) snow: vec4<f32>,
    // The plant kind of the canopy (voxel::plant), 0 with none.
    @location(7) @interpolate(flat) plant: u32,
    // The skylines toward sunrise and sunset (see skyline_light).
    @location(8) skylines: vec2<f32>,
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
    if (in.color.a > 0.75) {
        // A ribbon's grad is its side, outward from the centreline. Narrow rivers are
        // far under a pixel wide a few km out and would break up into dashes, so each
        // side is pushed out by about half a pixel (at 1080 lines; the resolution is
        // not in the globals). Wide rivers barely change.
        let tan_half = length(G.cam_up.xyz) / max(length(G.cam_fwd.xyz), 1e-6);
        let pixel = length(rel) * 2.0 * tan_half / 1080.0;
        rel.x += in.grad.x * pixel * 0.6;
        rel.z += in.grad.y * pixel * 0.6;
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
    out.far = srgb_to_linear(in.far.rgb);
    out.wet = in.far.a - 128.0 / 255.0;
    out.grad = in.grad * 4.0;
    out.kind = in.color.a;
    out.snow = biome_snow(in.place.x);
    out.plant = in.place.y;
    out.skylines = vec2<f32>(in.place.zw) / 255.0;
    return out;
}

// Water, shaded as the voxel water is but opaque: nothing is drawn beneath.
fn water_shade(c: vec3<f32>, rel: vec3<f32>, frozen: f32) -> vec3<f32> {
    let up = vec3<f32>(0.0, 1.0, 0.0);
    let view = normalize(rel);
    let fresnel = 0.03 + 0.97 * pow(1.0 - max(dot(-view, up), 0.0), 5.0);
    let body = c * (G.sun_color.rgb * max(G.sun_dir.y, 0.0) * 0.5 + ambient(up));
    let color = mix(body, sky_color(reflect(view, up)), fresnel);
    // Still water freezes over (rivers too: there is no flow to tell them apart).
    let ice = ice_amount(rel.xz + G.cam_wrapped.xz, frozen);
    return mix(color, vec3<f32>(0.62, 0.78, 0.88) * (G.sun_color.rgb * 0.7 * max(G.sun_dir.y, 0.0) + ambient(up)), ice);
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    // How much of this pixel a lake covers: its shore is a hard line, a pixel soft so
    // that it does not crawl. Taken before anything that branches.
    let lake = clamp(in.wet / max(fwidth(in.wet), 1e-6) + 0.5, 0.0, 1.0);
    // Nearer than this the voxel terrain is drawn instead.
    if (!beyond_voxels(in.rel)) {
        discard;
    }
    let up = vec3<f32>(0.0, 1.0, 0.0);
    var color = vec3<f32>(0.0);
    if (in.kind > 0.42) {
        color = water_shade(in.color, in.rel, in.snow.y);
    } else if (lake >= 1.0) {
        color = water_shade(srgb_to_linear(LAKE_WATER), in.rel, in.snow.y);
    } else {
        // Voxel terrain is all flat tops and vertical sides. From far away a slope
        // reads as a blend of the two, the sides counting for more the steeper it is.
        let steep = length(in.grad);
        let side_n = normalize(vec3<f32>(-in.grad.x, 0.0, -in.grad.y) + vec3<f32>(1e-5, 0.0, 0.0));
        // The shadow maps end long before here: hills shade it by their skylines.
        let vis = skyline_light(in.skylines) * cloud_light(in.rel);
        let top = G.sun_color.rgb * max(G.sun_dir.y, 0.0) * vis + ambient(up);
        let side = G.sun_color.rgb * max(dot(side_n, G.sun_dir.xyz), 0.0) * 0.8 * vis + ambient(side_n) * 0.7;
        // The season: a far forest turns and goes bare, grass dries, snow settles.
        let world = in.rel.xz + G.cam_wrapped.xz;
        // While the tree models stand on it, the ground is seen between them; out beyond,
        // the forest they drew is seen as its canopy, over the same reach the surface
        // rises to the canopy's height.
        var base = season_far_ground(in.color, false, world, G.season.x);
        let canopy = smoothstep(G.cam_wrapped.w * 0.85, G.cam_wrapped.w, length(in.rel.xz));
        if (canopy > 0.0) {
            // A canopy of palms or needles keeps its leaves, whatever its colour says.
            let evergreen = in.plant != 0u && in.plant != PLANT_DECIDUOUS && in.plant != PLANT_AUTUMN;
            let forest = in.kind > 0.002 && !evergreen;
            base = mix(base, select(season_far_ground(in.far, forest, world, G.season.x), in.far, evergreen), canopy);
        }
        base = ground_weather(base, up, in.rel, 0.5, false, false, false, 0.0, in.snow);
        color = base * mix(top, side, clamp(steep * 0.5, 0.0, 0.6));
        // Stepped ground also carries the shadowed edges of all its steps.
        color *= 1.0 - clamp(steep * 0.9, 0.0, 0.35);
        if (lake > 0.0) {
            color = mix(color, water_shade(srgb_to_linear(LAKE_WATER), in.rel, in.snow.y), lake);
        }
    }
    return vec4<f32>(apply_fog(color, in.rel), 1.0);
}
