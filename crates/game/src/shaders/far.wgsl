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
    // (voxel::plant) in its low three bits, 0 with none, and above them how much of the
    // forest keeps its leaves in winter, in 31sts. zw: its skylines toward sunrise and
    // sunset.
    @location(5) place: vec4<u32>,
    // Land: the wash of the flowers or heather lying over it (its row of G.wash_body and
    // G.wash_bloom, 0 for none); how much of the ground's colour it takes, in 255ths; and
    // how much of that still shows under the land's forest.
    @location(6) wash: vec4<u32>,
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
    // The plant kind of the canopy and the share of it that is evergreen, as in `place.y`.
    @location(7) @interpolate(flat) plant: u32,
    // The skylines toward sunrise and sunset (see skyline_light).
    @location(8) skylines: vec2<f32>,
    // The colour the flowers lying over the land give it this date, times how much of
    // the ground's colour they take, which is w.
    @location(9) wash: vec4<f32>,
    // How much of that shows under the forest, out beyond the tree models.
    @location(10) wash_far: f32,
    // How far the land lies below the land around it, or above it if negative (see
    // farview::hollow), as the measure (see hollow_light).
    @location(11) hollow: f32,
};

// The colour of the wash numbered `code` this date, seen from too far to tell one plant
// from the next: its leaves, and as much of its flowers as are out (each plant on its
// own day: see season_phase), or over and brown. As `wash` in terrain.wgsl colours the
// ground under each plant nearer to.
fn far_wash(code: u32, s: f32) -> vec3<f32> {
    let body = G.wash_body[min(code, 127u)];
    let bloom = G.wash_bloom[min(code, 127u)];
    var c = body.rgb;
    if (u32(body.a) == PLANT_GRASS) {
        c = season_grass(c, s);
    }
    let share = fract(bloom.a) * 2.0;
    let flowering = (u32(bloom.a) >> 3u) & 63u;
    var out = 1.0;
    var over = 0.0;
    if (flowering != 0u) {
        let row = G.flowering[flowering];
        if (row.y < 1.0) {
            // How far into the kind's season the date is, and so how many of its plants
            // have come out, and how many of those have gone over since.
            let d = fract(s - row.x);
            let opened = clamp(d / SEASON_STAGGER, 0.0, 1.0);
            let shut = clamp((d - row.y) / SEASON_STAGGER, 0.0, 1.0);
            let bare = clamp((d - row.y - row.w) / SEASON_STAGGER, 0.0, 1.0);
            out = opened - shut;
            over = select(0.0, shut - bare, row.w > 0.0);
        }
    }
    c = mix(c, bloom.rgb, share * 0.7 * out);
    c = mix(c, vec3<f32>(0.2, 0.105, 0.07), share * 0.5 * over);
    // Darker than the plant's own colours, as its wash on the nearer ground is.
    return c * 0.62;
}

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
    out.wash = vec4<f32>(0.0);
    out.wash_far = 0.0;
    out.hollow = hollow_measure(f32(in.wash.w));
    if (in.wash.x != 0u && in.wash.y != 0u) {
        let amount = f32(in.wash.y) / 255.0;
        out.wash = vec4<f32>(far_wash(in.wash.x, G.season.x) * amount, amount);
        out.wash_far = f32(in.wash.z) / 255.0;
    }
    return out;
}

// Water, shaded as the voxel water is but opaque: nothing is drawn beneath.
fn water_shade(c: vec3<f32>, rel: vec3<f32>, frozen: f32) -> vec3<f32> {
    let up = vec3<f32>(0.0, 1.0, 0.0);
    let view = normalize(rel);
    let fresnel = 0.03 + 0.97 * pow(1.0 - clamp(dot(-view, up), 0.0, 1.0), 5.0);
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
    // How well lit the land is, for the air before it (see apply_fog_lit).
    var lit = 1.0;
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
        // The tops are lit mostly as the slope they make up, as the far voxels are (see
        // shade in terrain.wgsl): lit as level ground, a hillside turned from the sun was
        // as bright as one facing it, and distant land was all one brightness.
        let slope_n = normalize(vec3<f32>(-in.grad.x, 1.0, -in.grad.y));
        let top_n = normalize(mix(up, slope_n, 0.7));
        // Valleys darker and ridges lighter, as the far voxels are.
        let fold = hollow_light(in.hollow);
        let sun = vis * fold.y;
        let top = G.sun_color.rgb * max(dot(top_n, G.sun_dir.xyz), 0.0) * sun + ambient(top_n) * fold.x;
        let side = G.sun_color.rgb * max(dot(side_n, G.sun_dir.xyz), 0.0) * 0.8 * sun + ambient(side_n) * 0.7 * fold.x;
        // The season: a far forest turns and goes bare, grass dries, snow settles.
        let world = in.rel.xz + G.cam_wrapped.xz;
        // While the tree models stand on it, the ground is seen between them; out beyond,
        // the forest they drew is seen as its canopy, over the same reach the surface
        // rises to the canopy's height.
        var base = season_far_ground(in.color, false, 0.0, world, G.season.x);
        // The flowers and heather that lie over it, in this date's colours.
        base = base * (1.0 - in.wash.a) + in.wash.rgb;
        let canopy = smoothstep(G.cam_wrapped.w * 0.85, G.cam_wrapped.w, length(in.rel.xz));
        if (canopy > 0.0) {
            // A canopy of palms or needles keeps its leaves, whatever its colour says.
            let plant = in.plant & 7u;
            let evergreen = plant != 0u && plant != PLANT_DECIDUOUS && plant != PLANT_AUTUMN;
            let forest = in.kind > 0.002 && !evergreen;
            // The hollies, yews and pines among a broadleaf forest's trees keep it partly
            // green when the rest is bare.
            let keeps = f32(in.plant >> 3u) / 31.0;
            var wooded = select(season_far_ground(in.far, forest, keeps, world, G.season.x), in.far, evergreen);
            wooded = wooded * (1.0 - in.wash.a * in.wash_far) + in.wash.rgb * in.wash_far;
            base = mix(base, wooded, canopy);
        }
        base = ground_weather(base, up, in.rel, 0.5, false, false, false, 0.0, in.snow);
        // Stepped ground also carries the shadowed edges of all its steps.
        let light = mix(top, side, clamp(steep * 0.5, 0.0, 0.6)) * (1.0 - clamp(steep * 0.9, 0.0, 0.35));
        color = base * light;
        lit = lit_beside_level(light);
        if (lake > 0.0) {
            color = mix(color, water_shade(srgb_to_linear(LAKE_WATER), in.rel, in.snow.y), lake);
        }
    }
    return vec4<f32>(apply_fog_lit(color, in.rel, mix(lit, 1.0, lake)), 1.0);
}
