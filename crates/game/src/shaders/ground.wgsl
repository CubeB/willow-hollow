// Snow and wet ground, drawn on top of what the surface already is. Terrain only, after
// common.wgsl: it reads the globals. G.season.y is the snow cover at sea level, 0..1
// (see season.rs); G.weather.x is how wet the ground is, 0..1.

const SNOW_COLOUR: vec3<f32> = vec3<f32>(0.80, 0.86, 0.93);

// Snow settles thicker the higher the ground: a little more cover for each metre of
// height, and the very highest ground keeps a cap through the summer.
fn snow_depth(alt_m: f32) -> f32 {
    return G.season.y * (1.0 + alt_m * 0.01) + max(alt_m - 90.0, 0.0) * 0.008;
}

fn ground_weather(c: vec3<f32>, n: vec3<f32>, rel: vec3<f32>, h: f32, foliage: bool, emissive: bool) -> vec3<f32> {
    var o = c * (1.0 - 0.3 * G.weather.x);
    if (emissive) {
        return o;
    }
    let alt_m = (G.cam_wrapped.y + rel.y) / VOXELS_PER_METRE;
    let depth = snow_depth(alt_m);
    if (depth <= 0.0) {
        return o;
    }
    // Drifts: patchy where the cover is thin, even where it is thick.
    let world_xz = rel.xz + G.cam_wrapped.xz;
    let drift = vnoise(world_xz / (VOXELS_PER_METRE * 1.8));
    if (n.y < 0.6) {
        // The sides of snowy ground look buried: dusted and dulled toward a cold earth.
        let buried = smoothstep(0.05, 0.5, depth - 0.3 * drift) * select(0.85, 0.5, foliage);
        let cold = o * vec3<f32>(0.5, 0.52, 0.58) + vec3<f32>(0.025, 0.03, 0.04);
        return mix(o, mix(cold, SNOW_COLOUR * 0.55, 0.3), buried);
    }
    let cover = smoothstep(0.0, 0.3, depth - 0.5 * drift - 0.12 * h)
        * smoothstep(0.6, 0.95, n.y) * select(1.0, 0.85, foliage);
    return mix(o, SNOW_COLOUR * (0.9 + 0.1 * h), cover);
}

// Ice on still water: whole sheets in deep cold, broken at the edges before that.
fn ice_amount(world_xz: vec2<f32>) -> f32 {
    let frozen = G.season.z;
    if (frozen <= 0.0) {
        return 0.0;
    }
    let broken = vnoise(world_xz / (VOXELS_PER_METRE * 2.4));
    return smoothstep(0.0, 0.25, frozen * 1.2 - 0.5 * broken);
}
