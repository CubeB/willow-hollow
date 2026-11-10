// Snow and wet ground, drawn on top of what the surface already is. Terrain only, after
// common.wgsl: it reads the globals, so the near terrain, the far terrain and the far
// trees all show the same snow. Snow depends on the place: each biome's ground is a row of
// G.snow (season::Places), x the snow lying at sea level, 0..1, from the weather
// (weather::lying), y the ice on still water, z how much the cold of height adds this
// winter and w the summit snowline in metres (no snow, no hill snow and only a very high
// snowline where it never snows). Each surface looks up the biome it is in with
// `biome_snow` and passes that row on. G.weather.x is how wet the ground is.

const SNOW_COLOUR: vec3<f32> = vec3<f32>(0.80, 0.86, 0.93);

// The ground in biome `id` (biome::Biome as u8); id 0, the sea, is the camera's own.
fn biome_snow(id: u32) -> vec4<f32> {
    return G.snow[min(id, 23u)];
}

// The ground at a point of a terrain chunk, from the biomes at its four corners (packed a
// byte each, see terrain::GpuChunk::biomes), blended across it so that neighbouring
// chunks, which share their corners, meet without a seam. `t` is 0..1 across the chunk.
fn corner_snow(biomes: u32, t: vec2<f32>) -> vec4<f32> {
    let a = biome_snow(biomes & 255u);
    let b = biome_snow((biomes >> 8u) & 255u);
    let c = biome_snow((biomes >> 16u) & 255u);
    let d = biome_snow(biomes >> 24u);
    let u = clamp(t, vec2<f32>(0.0), vec2<f32>(1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// Snow settles thicker the higher the ground: a little more of what lies for each metre of
// height, hills that hold some through a winter thaw, and summits above the snowline that
// keep a cap through the summer.
fn snow_depth(alt_m: f32, snow: vec4<f32>) -> f32 {
    let lying = snow.x * (1.0 + max(alt_m, 0.0) * 0.01);
    let hills = snow.z * max(alt_m - 25.0, 0.0) * 0.003;
    return lying + hills + max(alt_m - snow.w, 0.0) * 0.008;
}

// `wood` is bark and planks, which keep their colour; `edge` is the height up the face within its voxel.
// `snow` is the place's row of G.snow (see biome_snow).
fn ground_weather(c: vec3<f32>, n: vec3<f32>, rel: vec3<f32>, h: f32, foliage: bool, emissive: bool, wood: bool, edge: f32, snow: vec4<f32>) -> vec3<f32> {
    var o = c * (1.0 - 0.3 * G.weather.x);
    if (emissive) {
        return o;
    }
    let alt_m = (G.cam_wrapped.y + rel.y) / VOXELS_PER_METRE;
    let depth = snow_depth(alt_m, snow);
    if (depth <= 0.0) {
        return o;
    }
    // Drifts: patchy where the cover is thin, even where it is thick.
    let world_xz = rel.xz + G.cam_wrapped.xz;
    let drift = vnoise(world_xz / (VOXELS_PER_METRE * 1.8));
    if (n.y < 0.6) {
        // The sides of snowy ground look buried: a cold brown, with the snow lying over
        // the top edge of each face like an overhang. Wood keeps its colour, bar a dusting.
        let buried = smoothstep(0.05, 0.5, depth - 0.3 * drift) * select(0.85, 0.5, foliage);
        if (wood) {
            return mix(o, SNOW_COLOUR * 0.6, buried * 0.12);
        }
        let cold = o * vec3<f32>(0.72, 0.64, 0.58) + vec3<f32>(0.02, 0.018, 0.02);
        let lip = smoothstep(0.6, 0.92, edge) * 0.75;
        return mix(o, mix(cold, SNOW_COLOUR * 0.8, lip), buried);
    }
    let cover = smoothstep(0.0, 0.3, depth - 0.5 * drift - 0.12 * h)
        * smoothstep(0.6, 0.95, n.y) * select(1.0, 0.85, foliage);
    return mix(o, SNOW_COLOUR * (0.9 + 0.1 * h), cover);
}

// Small plants lost in deep snow: most tufts are buried, the rest are capped by the snow
// colour on their tops.
fn plant_buried(rel: vec3<f32>, h: f32, snow: vec4<f32>) -> bool {
    let alt_m = (G.cam_wrapped.y + rel.y) / VOXELS_PER_METRE;
    return h < smoothstep(0.25, 0.9, snow_depth(alt_m, snow)) * 0.85;
}

// Ice on still water: whole sheets in deep cold, broken at the edges before that.
// `frozen` is the place's ice (biome_snow's y).
fn ice_amount(world_xz: vec2<f32>, frozen: f32) -> f32 {
    if (frozen <= 0.0) {
        return 0.0;
    }
    let broken = vnoise(world_xz / (VOXELS_PER_METRE * 2.4));
    return smoothstep(0.0, 0.25, frozen * 1.2 - 0.5 * broken);
}
