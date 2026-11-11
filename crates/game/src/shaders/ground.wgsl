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

// How much of flat ground snow covers at a height of `alt_m` metres and a world position
// of `world_xz` voxels, 0..1, with `h` a random of the surface's own. Drifts: patchy where
// the cover is thin, even where it is thick.
fn snow_cover(alt_m: f32, world_xz: vec2<f32>, h: f32, snow: vec4<f32>) -> f32 {
    let depth = snow_depth(alt_m, snow);
    if (depth <= 0.0) {
        return 0.0;
    }
    let drift = vnoise(world_xz / (VOXELS_PER_METRE * 1.8));
    return smoothstep(0.0, 0.3, depth - 0.5 * drift - 0.12 * h);
}

// The far terrain and the far trees, where a block is far under a pixel: the snow blends.
// `wood` is bark and planks, which keep their colour; `edge` is the height up the face
// within its voxel. `snow` is the place's row of G.snow (see biome_snow).
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
    let cover = snow_cover(alt_m, world_xz, h, snow) * smoothstep(0.6, 0.95, n.y) * select(1.0, 0.85, foliage);
    return mix(o, SNOW_COLOUR * (0.9 + 0.1 * h), cover);
}

// Snow on a voxel of the near terrain. It settles block by block: a voxel open to the sky
// (`sky`, see voxel::flags::SKY) is bare, or a quarter, half, three quarters or wholly
// covered, the same all over it, and a voxel with anything above it is never capped.
// `centre` is the voxel's middle and `rel` the point shaded, camera-relative; `fine` is
// where the point lies across the voxel's top, 0..1 each way; `h` and `h2` are the
// voxel's randoms; `edge` is the height up a side face within its voxel.
fn block_weather(c: vec3<f32>, n: vec3<f32>, rel: vec3<f32>, centre: vec3<f32>, fine: vec2<f32>, h: f32, h2: f32, foliage: bool, emissive: bool, wood: bool, sky: bool, edge: f32, snow: vec4<f32>) -> vec3<f32> {
    var o = c * (1.0 - 0.3 * G.weather.x);
    if (emissive) {
        return o;
    }
    let alt_m = (G.cam_wrapped.y + centre.y) / VOXELS_PER_METRE;
    let depth = snow_depth(alt_m, snow);
    if (depth <= 0.0) {
        return o;
    }
    // The cover over the whole voxel, in quarters, dithered so a fading cover breaks up
    // block by block rather than in rings.
    let cover = snow_cover(alt_m, centre.xz + G.cam_wrapped.xz, h, snow);
    let quarters = select(0.0, clamp(floor(cover * 4.0 + (h2 - 0.5) * 0.9 + 0.5), 0.0, 4.0), sky);
    if (n.y < 0.6) {
        // The sides of snowy ground look cold: a cold brown. A capped voxel's snow lies
        // over the top edge of each side like an overhang. Wood keeps its colour, bar a
        // dusting.
        if (wood) {
            return mix(o, SNOW_COLOUR * 0.6, quarters * 0.03);
        }
        let cold_amount = smoothstep(0.05, 0.5, depth) * select(0.6, 0.35, foliage);
        let cold = o * vec3<f32>(0.72, 0.64, 0.58) + vec3<f32>(0.02, 0.018, 0.02);
        o = mix(o, cold, cold_amount);
        let lip = select(0.0, 0.8, edge > 1.0 - 0.06 * quarters) * step(0.5, quarters);
        return mix(o, SNOW_COLOUR * 0.8, lip);
    }
    if (quarters <= 0.0) {
        return o;
    }
    // Which quarters of the top are snowed on: they fill in a turn around the voxel from
    // a corner of its own. Far off, where a quarter is under a pixel, the cover blends.
    let q = u32(fine.x >= 0.5) + 2u * u32(fine.y >= 0.5);
    var turn = q;
    if (q >= 2u) {
        turn = 5u - q;
    }
    let rank = f32((turn + u32(h2 * 4.0)) % 4u);
    let covered = select(0.0, 1.0, rank < quarters);
    let far = smoothstep(40.0 * VOXELS_PER_METRE, 90.0 * VOXELS_PER_METRE, length(rel));
    let amount = mix(covered, quarters * 0.25, far) * select(1.0, 0.9, foliage);
    return mix(o, SNOW_COLOUR * (0.9 + 0.1 * h), amount);
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
