struct VIn {
    @location(0) a: u32,
    @location(1) b: u32,
    // Per chunk: position of the chunk origin relative to the camera, and in world voxels.
    @location(2) offset: vec3<f32>,
    @location(3) origin: vec3<i32>,
    // Per chunk: voxels per cell of its mesh, and the distances between which it is shown.
    // The scale is negative for a chunk's small plants, which give out with distance.
    @location(4) scale: f32,
    @location(5) band: vec2<f32>,
    // Per chunk: a grid of the biomes over it, for the snow (see grid_snow in ground.wgsl).
    @location(6) grid0: vec4<u32>,
    @location(7) grid1: vec4<u32>,
    // Per chunk: the lie of the land at its four corners, x fastest (terrain::GpuChunk::
    // slopes): the x of each normal in the first word, the z in the second, signed bytes.
    @location(8) slopes: vec2<u32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) rel: vec3<f32>,
    @location(1) local: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) ao: f32,
    @location(4) @interpolate(flat) face: u32,
    @location(5) @interpolate(flat) origin: vec3<i32>,
    @location(6) @interpolate(flat) flags: u32,
    @location(7) @interpolate(flat) band: vec2<f32>,
    // Water only: the current, metres per second along x and z, and how much of a river
    // (rather than a lake or the sea) the water is, 0..1. Zero for everything else.
    @location(8) current: vec3<f32>,
    // The ground here: its row of G.snow, blended between the chunk's corners.
    @location(9) snow: vec4<f32>,
    // Voxels per cell of the chunk's mesh.
    @location(10) @interpolate(flat) scale: f32,
    // The chunk's grid of biomes, as in VIn.
    @location(11) @interpolate(flat) grid0: vec4<u32>,
    @location(12) @interpolate(flat) grid1: vec4<u32>,
    // Water only, where it falls. x: how near the foot of a fall, 0..1. y: on a surface,
    // 1 at the lip of a fall; on the falling sheet, how far below the surface it fell
    // from, in voxels of its chunk's mesh.
    @location(13) spill: vec2<f32>,
    // Where this point is with no wind: `rel` before leaf_sway. The shadow maps draw every
    // leaf and plant standing still, so shadows are looked up here. It is passed down
    // rather than worked out again per pixel, because only the vertex shader knows which
    // corners sway (a grass blade's foot stays put while its tip moves) and the chunk's
    // origin in voxels, so any per-pixel guess slid in and out of its own shadow.
    @location(14) still: vec3<f32>,
    // The x and z of the normal of the land this point is part of (see chunk_slope).
    @location(15) slope: vec2<f32>,
};

// The lie of the land at a point of a terrain chunk: the x and z of its unit normal,
// blended between the chunk's corners, which its neighbours share. `t` is 0..1 across it.
fn chunk_slope(slopes: vec2<u32>, t: vec2<f32>) -> vec2<f32> {
    let f = clamp(t, vec2<f32>(0.0), vec2<f32>(1.0));
    let shifts = vec4<u32>(0u, 8u, 16u, 24u);
    // Shifted up and back down as signed, to bring each byte's sign with it.
    let x = vec4<f32>((vec4<i32>(vec4<u32>(slopes.x) >> shifts) << vec4<u32>(24u)) >> vec4<u32>(24u)) / 127.0;
    let z = vec4<f32>((vec4<i32>(vec4<u32>(slopes.y) >> shifts) << vec4<u32>(24u)) >> vec4<u32>(24u)) / 127.0;
    return vec2<f32>(mix(mix(x.x, x.y, f.x), mix(x.z, x.w, f.x), f.y), mix(mix(z.x, z.y, f.x), mix(z.z, z.w, f.x), f.y));
}

// A vertex's position in its chunk, in voxels (see `Vertex` in voxel/src/mesh.rs): whole
// voxels, quarters `q`, and per axis a bit that, with `q` set, moves the corner an eighth
// of a voxel toward the nearer face of its voxel (q 1, 2, 3 -> 1/8, 5/8, 7/8), for skins
// half a cell thick. shadow.wgsl has the same two functions.
fn vertex_local(a: u32) -> vec3<f32> {
    let q = vec3<u32>((a >> 23u) & 3u, (a >> 25u) & 3u, (a >> 27u) & 3u);
    let bit = vec3<u32>((a >> 29u) & 1u, (a >> 30u) & 1u, (a >> 31u) & 1u);
    let half = (bit == vec3<u32>(1u)) & (q != vec3<u32>(0u));
    let nudge = select(vec3<f32>(-0.125), vec3<f32>(0.125), q >= vec3<u32>(2u));
    return vec3<f32>(f32(a & 63u), f32((a >> 6u) & 63u), f32((a >> 12u) & 63u))
        + vec3<f32>(q) * 0.25 + select(vec3<f32>(0.0), nudge, half);
}

// Per axis, 1 where a corner on a voxel boundary belongs to the voxel before it.
fn vertex_corner(a: u32) -> vec3<i32> {
    let q = vec3<u32>((a >> 23u) & 3u, (a >> 25u) & 3u, (a >> 27u) & 3u);
    let bit = vec3<i32>(i32((a >> 29u) & 1u), i32((a >> 30u) & 1u), i32((a >> 31u) & 1u));
    return select(vec3<i32>(0), bit, q == vec3<u32>(0u));
}

// True if this ring of terrain has given way to another at this distance. Each pixel is
// drawn by exactly one ring.
fn outside_band(rel: vec3<f32>, band: vec2<f32>) -> bool {
    let d = length(rel.xz);
    return d < band.x || d >= band.y;
}

fn face_normal(face: u32) -> vec3<f32> {
    switch face {
        case 0u: { return vec3<f32>(1.0, 0.0, 0.0); }
        case 1u: { return vec3<f32>(-1.0, 0.0, 0.0); }
        case 2u: { return vec3<f32>(0.0, 1.0, 0.0); }
        case 3u: { return vec3<f32>(0.0, -1.0, 0.0); }
        case 4u: { return vec3<f32>(0.0, 0.0, 1.0); }
        default: { return vec3<f32>(0.0, 0.0, -1.0); }
    }
}

// On opaque faces: the voxel is open to the sky (voxel::flags::SKY).
const FLAG_SKY: u32 = 1u;
const FLAG_FOLIAGE: u32 = 8u;
const FLAG_EMISSIVE: u32 = 16u;
// Both at once, which nothing is: ground that takes the colour of the plant standing on
// it when seen from afar (voxel::flags::WASH). Its vertices keep the colour's number where
// others keep fractions of a voxel, and `vs` hands it on above the flags (<< 16).
const FLAG_WASH: u32 = 24u;

// The colour of ground that a small plant stands on, `voxel` its voxel and `centre` the
// middle of it, camera-relative. Near to, the plant itself is drawn and the ground is its
// own colour; as the plant gives out with distance (plant_reach) the ground takes its
// colour instead, and keeps it from there to the horizon. So heather, bracken and flowers
// lie over a far hillside in the same drifts they are found in on it, and the land does
// not go bare a hundred metres from the camera.
fn wash(ground: vec3<f32>, code: u32, voxel: vec3<i32>, centre: vec3<f32>, scale: f32) -> vec3<f32> {
    let body = G.wash_body[min(code, 63u)];
    let bloom = G.wash_bloom[min(code, 63u)];
    let kinds = vec2<u32>(u32(body.a), u32(bloom.a) & 7u);
    // The row of G.flowering the flowers keep, if they have a season of their own.
    let flowering = (u32(bloom.a) >> 3u) & 63u;
    let share = fract(bloom.a) * 2.0;
    var amount = fract(body.a) * 2.0;
    if (scale < 1.5) {
        let reach = PLANT_REACH * plant_reach(voxel.xz);
        amount *= smoothstep(reach - PLANT_REACH * 0.22, reach, length(centre.xz));
    }
    if (amount <= 0.0) {
        return ground;
    }
    // Plants are known to the seasons by their column alone.
    let col = vec3<i32>(voxel.x, 0, voxel.z);
    let s = G.season.x;
    // No two plants quite alike. And darker than their own colours: a plant standing
    // shades itself and the ground round it, where this is flat ground in the sun.
    let h = hash3(voxel.zyx + vec3<i32>(3, 11, 5));
    var c = season_apply(body.rgb, col, kinds.x, s, 0u) * (0.5 + 0.2 * h);
    // Its flowers show on some columns and not on others, as bright specks in the dark
    // of the leaves: mixed into every one, heather went a flat lilac.
    if (fract(h * 17.0) < share) {
        if (flowering != 0u) {
            // In flower, or over and brown, on the day the plant itself is.
            let phase = season_phase(G.flowering[flowering], s, voxel.xz);
            if (phase.x > 1.5) {
                c = mix(c, season_bloom(bloom.rgb, phase), 0.8);
            }
        } else if (!season_gone(s, kinds.y, col, 0u)) {
            c = mix(c, season_apply(bloom.rgb, col, kinds.y, s, 0u), 0.8);
        }
    }
    return mix(ground, c, amount);
}
const NEAR: f32 = 0.08 * VOXELS_PER_METRE;
// How far in from a face its voxel is looked for, in voxels: less than the thinnest
// thing drawn (a skin an eighth of a voxel thick), and well clear of rounding.
const INWARD: f32 = 0.0625;

// How far the wind has moved a leaf from where it grows. The offset is a smooth
// function of world position, so neighbouring leaf voxels move together and no gaps
// open between them.
fn leaf_sway(origin: vec3<i32>, local: vec3<f32>, scale: f32) -> vec3<f32> {
    let world = (vec3<f32>(origin) + local * scale) / VOXELS_PER_METRE;
    let t = G.sun_dir.w;
    let wave = sin(world.x * 0.4375 + world.z * 0.3375 + world.y * 0.25 + t * 1.7)
        + 0.5 * sin(world.x * 1.125 - world.z * 0.75 + t * 2.9);
    let strength = length(G.wind.xz) / VOXELS_PER_METRE * 0.0375;
    return vec3<f32>(G.wind.x, 0.0, G.wind.z) * (0.02 + wave * strength)
        + vec3<f32>(0.0, wave * strength * 0.4 * VOXELS_PER_METRE, 0.0);
}

@vertex
fn vs(in: VIn) -> VOut {
    var out: VOut;
    var local = vertex_local(in.a);
    out.face = (in.a >> 18u) & 7u;
    out.ao = f32((in.a >> 21u) & 3u) / 3.0;
    // A broadleaf leaf's tree tag (season_tag) rides above the vertex flags.
    let tag = season_tag(in.b);
    out.flags = (in.b >> 24u) | (tag << 8u);
    if ((out.flags & FLAG_WASH) == FLAG_WASH) {
        local = vec3<f32>(f32(in.a & 63u), f32((in.a >> 6u) & 63u), f32((in.a >> 12u) & 63u));
        out.flags = (out.flags & ~FLAG_WASH) | (((in.a >> 23u) & 511u) << 16u);
    }
    let scale = abs(in.scale);
    out.local = local;
    out.band = in.band;
    out.snow = grid_snow(in.grid0, in.grid1, local.xz / 32.0);
    out.grid0 = in.grid0;
    out.grid1 = in.grid1;
    out.slope = chunk_slope(in.slopes, local.xz / 32.0);
    out.scale = scale;
    var rel = local * scale + in.offset;
    out.still = rel;
    if ((out.flags & FLAG_FOLIAGE) != 0u) {
        rel += leaf_sway(in.origin, local, scale);
    }
    out.rel = rel;
    out.clip = G.view_proj * vec4<f32>(rel, 1.0);
    // Two things with faces in one plane (a blade of grass standing in a clump of moss)
    // would flicker there, each winning some pixels. Each colour is drawn a hair nearer
    // than the last instead, up to 3 mm in 100 m, so that one of them wins them all.
    out.clip.z *= 1.0 + f32((in.b * 2654435761u) >> 27u) * 1e-6;
    // Plants that are gone this season (fallen leaves, dead flowers, tufts under snow) are
    // dropped here: every corner of the face names the same voxel, so the whole face goes.
    // Everything the choice rests on must come from that voxel alone, never from where the
    // corner itself is, or a face could lose some corners and not others and be torn into
    // a sliver reaching across the screen.
    let kind = (in.b >> 29u) & 7u;
    if (kind != 0u && kind != PLANT_EVERGREEN && kind < 6u) {
        let cell = vec3<i32>(floor(local)) - vertex_corner(in.a);
        var vox = in.origin / i32(scale) + cell;
        let alt_m = f32(vox.y) * scale / VOXELS_PER_METRE;
        if (kind >= PLANT_GRASS) {
            vox.y = 0;
        }
        var gone = false;
        if (kind == PLANT_FLOWER && tag != 0u) {
            // A flower with a season of its own (the tag is its row of G.flowering).
            gone = season_phase(G.flowering[tag], G.season.x, vox.xz).x < 0.5;
        } else {
            gone = season_gone(G.season.x, kind, vox, tag);
        }
        if (kind == PLANT_GRASS && !gone) {
            let snow = grid_snow(in.grid0, in.grid1, (vec2<f32>(cell.xz) + 0.5) / 32.0);
            gone = plant_buried(alt_m, season_hash(vox + vec3<i32>(5, 0, 9)), snow);
        }
        if (gone) {
            out.clip = vec4<f32>(0.0, 0.0, 2.0, 1.0);
        }
    }
    if (in.scale < 0.0) {
        // Small plants give out with distance a column at a time, each at its own (every
        // corner of a plant's faces names its column: see split_at_voxels in mesh.rs).
        let cell = vec3<i32>(floor(local)) - vertex_corner(in.a);
        if (length(in.offset.xz + vec2<f32>(cell.xz) + 0.5) >= PLANT_REACH * plant_reach(in.origin.xz + cell.xz)) {
            out.clip = vec4<f32>(0.0, 0.0, 2.0, 1.0);
        }
    }
    let rgb = vec3<f32>(f32(in.b & 255u), f32((in.b >> 8u) & 255u), f32((in.b >> 16u) & 255u));
    out.color = srgb_to_linear(rgb / 255.0);
    // Hashed per cell, so a coarse chunk counts its cells from its own corner.
    out.origin = in.origin / i32(scale);
    return out;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    if (outside_band(in.rel, in.band)) {
        discard;
    }
    return shade(in);
}

// For chunks wholly inside their ring. Without a discard the GPU can settle depth
// before running the shader, so hidden surfaces cost nothing.
@fragment
fn fs_inside(in: VOut) -> @location(0) vec4<f32> {
    return shade(in);
}

// How far a voxel has shrunk toward a pixel, 0 while it is a few pixels across, 1 once it
// is about one. Out there the grid of tops and sides is finer than the pixels and would
// shimmer in bands (moire) as the view moves, so faces are drawn more alike: sides lit
// and coloured more like the tops they stand under, as the far view draws a slope.
// The screen height is not in the globals, so this takes 1080 lines.
fn distant(rel: vec3<f32>, scale: f32) -> f32 {
    let tan_half = length(G.cam_up.xyz) / max(length(G.cam_fwd.xyz), 1e-6);
    let pixel = length(rel) * 2.0 * tan_half / 1080.0;
    return 1.0 - smoothstep(1.2, 4.0, scale / max(pixel, 1e-6));
}

fn shade(in: VOut) -> vec4<f32> {
    let n = face_normal(in.face);
    let far = distant(in.rel, in.scale);
    // The voxel this fragment belongs to: a step inward from the face. A short one: the
    // faces of small plants stand at eighths of a voxel, and half a voxel in from one in
    // the middle of its voxel is the very edge of the next, where rounding would give
    // each pixel one voxel or the other and the face would be speckled with both.
    let cell = floor(in.local - n * INWARD);
    let voxel = vec3<i32>(cell) + in.origin;
    let jitter = f32((in.flags >> 1u) & 3u);
    let h = hash3(voxel);
    let h2 = hash3(voxel.zxy + vec3<i32>(17, 31, 59));
    // Each cube gets its own slight brightness and warmth so merged faces still read as cubes.
    var base = in.color * (1.0 + (h - 0.5) * 0.09 * jitter);
    base *= vec3<f32>(1.0 + (h2 - 0.5) * 0.03 * jitter, 1.0, 1.0 - (h2 - 0.5) * 0.03 * jitter);
    // The season: leaf colour and fall, dry or wintry grass.
    let kind = (in.flags >> 5u) & 7u;
    let tag = (in.flags >> 8u) & 63u;
    if (kind == PLANT_FLOWER && tag != 0u) {
        // A flower in bud, out or gone to seed, by its own season.
        base = season_bloom(base, season_phase(G.flowering[tag], G.season.x, voxel.xz));
    } else {
        base = season_apply(base, voxel, kind, G.season.x, tag);
    }
    // The middle of the voxel, camera-relative, for snow that is the same all over it.
    let centre = in.rel + (cell + 0.5 - in.local) * in.scale;
    let washed = (in.flags >> 16u) & 511u;
    if (washed != 0u) {
        base = wash(base, washed, voxel, centre, in.scale);
    }
    let snow = voxel_snow(in.grid0, in.grid1, (cell.xz + 0.5) / 32.0, centre.xz + G.cam_wrapped.xz, vec2<f32>(h2, fract(h * 7.31)));
    base = block_weather(base, n, in.rel, centre, fract(in.local.xz), h, h2, (in.flags & FLAG_FOLIAGE) != 0u, (in.flags & FLAG_EMISSIVE) != 0u, ((in.flags >> 5u) & 7u) == 6u, (in.flags & FLAG_SKY) != 0u, fract(in.local.y), snow, far);

    let ao = mix(in.ao * in.ao * 0.75 + 0.25, 0.85, far);
    // Far off, where tops and sides are too small to tell apart, the ground is lit as
    // the slope it is part of: a hillside facing the sun is bright and one turned from
    // it is dim, where every top lit alike and every side lit alike made the land all
    // one brightness. Leaves keep to the level: a crown stands clear of the slope.
    var lit_n = n;
    if (far > 0.0 && n.y > -0.5) {
        var land = vec3<f32>(0.0, 1.0, 0.0);
        if ((in.flags & FLAG_FOLIAGE) == 0u) {
            land = vec3<f32>(in.slope.x, sqrt(max(1.0 - dot(in.slope, in.slope), 0.04)), in.slope.y);
        }
        if (abs(n.y) < 0.5) {
            lit_n = normalize(mix(n, land, far * 0.65));
        } else if ((in.flags & FLAG_FOLIAGE) == 0u) {
            // Not all the way: the tops are level, and lit wholly as the slope a hillside
            // turned from the sun would go black, and brighten as it was walked up to.
            lit_n = normalize(mix(n, land, far * 0.7));
        }
    }
    let facing = dot(lit_n, G.sun_dir.xyz);
    let foliage = (in.flags & FLAG_FOLIAGE) != 0u;
    // Leaves are not flat boards: light wraps a little way round a clump of them.
    let ndl = select(max(facing, 0.0), max((facing + 0.25) / 1.25, 0.0), foliage);
    // Past the shadow maps, ground and trunks under a canopy are taken to be in its
    // shade, so a far forest does not light up where the maps end.
    // And hills shade it at low sun, by their skylines, as they do the far view.
    let open = (in.flags & FLAG_SKY) != 0u;
    let beyond = select(0.3, 1.0, open) * skyline_side_light(grid_skyline(in.grid1, in.local.xz / 32.0));
    var direct = vec3<f32>(0.0);
    // The shadow maps are drawn with every leaf where it grows, not where the wind has
    // it: a leaf looked up where it sways to would slide in and out of its own shadow.
    let still = in.still;
    if (ndl > 0.0) {
        direct = G.sun_color.rgb * ndl * sun_reaching(still, n, beyond) * mix(mix(0.7, 1.0, in.ao), 1.0, far);
    } else if (foliage && G.sun_color.w > 0.5) {
        // A leaf with the sun behind it glows with the light that comes through, if
        // the sun reaches the other side: thin crowns and the edges of thick ones.
        let through = max(sun_reaching(still + G.sun_dir.xyz * (1.1 * in.scale), G.sun_dir.xyz, beyond), select(0.0, 0.3, open));
        direct = G.sun_color.rgb * vec3<f32>(0.85, 1.0, 0.45) * (0.45 * through * min(-facing * 2.0 + 0.35, 1.0));
    }
    var light = direct + ambient(lit_n) * ao + local_lights(in.rel, n) * ao;
    if (!open) {
        // Under a canopy the shade is not grey: it is lit by sun that has been through
        // and bounced among the leaves overhead.
        light += G.sun_color.rgb * vec3<f32>(0.045, 0.075, 0.028) * (max(G.sun_dir.y, 0.0) * ao * G.sun_color.w);
    }
    if ((in.flags & FLAG_EMISSIVE) != 0u) {
        // Lanterns and embers: faint by day, bright enough to bloom after dark.
        light += vec3<f32>(mix(14.0, 1.2, G.sun_color.w));
    }
    let color = apply_fog(base * light, in.rel);
    return vec4<f32>(color, 1.0);
}

// --- Water ---------------------------------------------------------------------------

@group(1) @binding(0) var scene_depth: texture_depth_2d;

// Fastest current a water vertex carries, metres per second (MAX_CURRENT in mesh.rs).
const MAX_CURRENT: f32 = 2.0;
// Seconds the pattern on running water is carried downstream before it starts afresh.
const FLOW_CYCLE: f32 = 1.6;

// The current a water vertex carries in `a` bits 23..=31 (see mesh.rs): direction in 32
// steps from +X toward +Z, speed in 8 steps spaced as (step / 7)^1.5, and a river bit.
// Returns metres per second along x and z, and 1 for a river.
fn water_current(a: u32) -> vec3<f32> {
    let angle = f32((a >> 23u) & 31u) * (6.2831853 / 32.0);
    let step = f32((a >> 28u) & 7u) / 7.0;
    let speed = MAX_CURRENT * step * sqrt(step);
    return vec3<f32>(cos(angle) * speed, sin(angle) * speed, f32(a >> 31u));
}

// How far a water surface is drawn down at the lip of a fall, where the sheet of falling
// water takes over from it, in voxels.
const LIP_DIP: f32 = 0.25;
// `face` of the sheet of water falling toward +X; -X, +Z and -Z follow.
const FACE_SPILL: u32 = 8u;

// The way a sheet of water falls (SPILL_DIRS in mesh.rs).
fn spill_dir(d: u32) -> vec3<f32> {
    switch d {
        case 0u: { return vec3<f32>(1.0, 0.0, 0.0); }
        case 1u: { return vec3<f32>(-1.0, 0.0, 0.0); }
        case 2u: { return vec3<f32>(0.0, 0.0, 1.0); }
        default: { return vec3<f32>(0.0, 0.0, -1.0); }
    }
}

// How far out from the face it left a sheet of falling water has got, `s` voxels below
// the surface it fell from: the curve of water thrown off a lip, which leaves it level
// and steepens, kept within the voxel it falls through.
fn spill_reach(s: f32) -> f32 {
    return min(0.5 * sqrt(max(s - LIP_DIP, 0.0)), 0.95);
}

// The water's own vertex shader. Water surfaces carry their current where other faces
// keep quarter-voxel fractions and corner bits, and where they meet a fall where other
// faces keep AO; the sheets of falling water (face 6) have a layout of their own (see
// spill_vertex in mesh.rs). Water never sways or goes with the season.
@vertex
fn vs_water(in: VIn) -> VOut {
    var out: VOut;
    var local = vec3<f32>(f32(in.a & 63u), f32((in.a >> 6u) & 63u), f32((in.a >> 12u) & 63u));
    out.face = (in.a >> 18u) & 7u;
    out.ao = 1.0;
    if (out.face == 6u) {
        // Rows of a sheet are quarter voxels apart for the first two voxels of the fall,
        // then whole voxels.
        let code = (in.a >> 23u) & 63u;
        var s = f32(code) - 6.0;
        if (code <= 8u) {
            s = f32(code) * 0.25;
            local.y -= f32(code & 3u) * 0.25;
        }
        let d = (in.a >> 21u) & 3u;
        if ((in.a >> 31u) == 0u) {
            local += spill_dir(d) * spill_reach(s);
        }
        out.face = FACE_SPILL + d;
        out.spill = vec2<f32>(f32((in.a >> 29u) & 3u) / 3.0, s);
    } else {
        out.current = water_current(in.a);
        let lip = f32((in.a >> 22u) & 1u);
        local.y -= lip * LIP_DIP;
        out.spill = vec2<f32>(f32((in.a >> 21u) & 1u), lip);
    }
    out.flags = in.b >> 24u;
    out.local = local;
    out.band = in.band;
    out.snow = grid_snow(in.grid0, in.grid1, local.xz / 32.0);
    out.grid0 = in.grid0;
    out.grid1 = in.grid1;
    out.slope = chunk_slope(in.slopes, local.xz / 32.0);
    out.scale = in.scale;
    var rel = local * in.scale + in.offset;
    // Water surfaces sit a little below the top of their voxel, and falls start there.
    rel.y -= 0.125;
    out.rel = rel;
    out.still = rel;
    out.clip = G.view_proj * vec4<f32>(rel, 1.0);
    let rgb = vec3<f32>(f32(in.b & 255u), f32((in.b >> 8u) & 255u), f32((in.b >> 16u) & 255u));
    out.color = srgb_to_linear(rgb / 255.0);
    out.origin = in.origin / i32(in.scale);
    return out;
}

// Still water: two faint sets of ripples drifting slowly across each other, so the
// surface shivers without going anywhere.
fn still_height(p: vec2<f32>, t: f32) -> f32 {
    let q1 = p + vec2<f32>(t * 0.12, t * 0.05);
    let q2 = p * 1.9 - vec2<f32>(t * 0.09, -t * 0.14);
    return vnoise(q1) * 0.6 + vnoise(q2) * 0.4;
}

// How far the pattern on running water has been carried in one of the two phases of
// the flow (`phase` 0 or 0.5), metres. Carried for ever it would smear wherever the
// current turns or changes speed, so each phase starts afresh every cycle, somewhere
// new in the pattern so it never visibly repeats, while the other is at its strongest.
fn flow_shift(flow: vec2<f32>, t: f32, phase: f32) -> vec2<f32> {
    let c = t / FLOW_CYCLE + phase;
    let jump = fract(floor(c) * vec2<f32>(0.618, 0.382) + phase) * vec2<f32>(37.0, 23.0);
    return jump - flow * (fract(c) * FLOW_CYCLE);
}

// How much the second phase shows: none while it starts afresh, all while the first does.
fn flow_blend(t: f32) -> f32 {
    return abs(fract(t / FLOW_CYCLE) * 2.0 - 1.0);
}

// Running water: finer ripples, carried downstream with the current.
fn running_height(p: vec2<f32>, flow: vec2<f32>, t: f32) -> f32 {
    let q0 = p + flow_shift(flow, t, 0.0);
    let q1 = p + flow_shift(flow, t, 0.5);
    let a = vnoise(q0 * 1.3) * 0.6 + vnoise(q0 * 2.7 + vec2<f32>(9.1, 4.7)) * 0.4;
    let b = vnoise(q1 * 1.3) * 0.6 + vnoise(q1 * 2.7 + vec2<f32>(9.1, 4.7)) * 0.4;
    return mix(a, b, flow_blend(t));
}

// Height of the ripples at `p` (metres) on water running at `flow` (metres per second).
fn ripple_height(p: vec2<f32>, flow: vec2<f32>, t: f32) -> f32 {
    let run = smoothstep(0.03, 0.3, length(flow));
    var h = 0.0;
    if (run < 1.0) {
        // Calm: lower than the ripples on a current.
        h += still_height(p, t) * (1.0 - run) * 0.6;
    }
    if (run > 0.0) {
        h += running_height(p, flow, t) * run;
    }
    return h;
}

// Small moving ripples on the water surface, as a normal in world space. Faster water
// is rougher.
fn ripple_normal(world_xz: vec2<f32>, flow: vec2<f32>, t: f32) -> vec3<f32> {
    let p = world_xz / VOXELS_PER_METRE;
    let d = 0.08;
    let dx = ripple_height(p + vec2<f32>(d, 0.0), flow, t) - ripple_height(p - vec2<f32>(d, 0.0), flow, t);
    let dz = ripple_height(p + vec2<f32>(0.0, d), flow, t) - ripple_height(p - vec2<f32>(0.0, d), flow, t);
    let rough = 1.4 * (1.0 + 0.3 * length(flow));
    return normalize(vec3<f32>(-dx * rough, 1.0, -dz * rough));
}

// Foam on fast water, 0..1: streaks drawn out along the current and carried with it.
// The noise is smeared downstream by averaging it along the flow, which needs no turning
// of the pattern to the current's direction.
fn foam_streaks(world_xz: vec2<f32>, flow: vec2<f32>, t: f32) -> f32 {
    let speed = length(flow);
    if (speed < 0.7) {
        return 0.0;
    }
    let p = world_xz / VOXELS_PER_METRE;
    let along = flow / speed * 0.75;
    let q0 = (p + flow_shift(flow, t, 0.0)) * 2.5;
    let q1 = (p + flow_shift(flow, t, 0.5)) * 2.5;
    var a = 0.0;
    var b = 0.0;
    for (var k = 0; k < 4; k++) {
        a += vnoise(q0 - along * f32(k));
        b += vnoise(q1 - along * f32(k));
    }
    let streak = mix(a, b, flow_blend(t)) * 0.25;
    return smoothstep(0.6, 0.78, streak) * smoothstep(0.7, 1.6, speed);
}

@fragment
fn fs_water(in: VOut) -> @location(0) vec4<f32> {
    if (outside_band(in.rel, in.band)) {
        discard;
    }
    let n_face = face_normal(in.face);
    let view = normalize(in.rel);
    let world_xz = in.rel.xz + G.cam_wrapped.xz;
    let t = G.sun_dir.w;
    let up = vec3<f32>(0.0, 1.0, 0.0);
    // The way the surface itself faces, which is not straight up where it bends over a lip.
    let slope = cross(dpdx(in.rel), dpdy(in.rel));
    // White water, lit.
    let white = vec3<f32>(0.85) * (G.sun_color.rgb * 0.6 * max(G.sun_dir.y, 0.0) + ambient(up));

    // How much water the view ray passes through before hitting the bed, from the
    // depth of what was drawn behind this surface (reverse-Z: depth = near / dist).
    let behind = textureLoad(scene_depth, vec2<i32>(in.clip.xy), 0);
    let dist_behind = NEAR / max(behind, 1e-6);
    let dist_here = dot(in.rel, G.cam_fwd.xyz);
    let thickness = max(dist_behind - dist_here, 0.0) * max(abs(view.y), 0.25);

    var color: vec3<f32>;
    var alpha: f32;
    if (in.face == 2u) {
        // Ripples are far finer than a pixel out in the coarser rings, and would only
        // sparkle there.
        let calm = smoothstep(80.0 * VOXELS_PER_METRE, 300.0 * VOXELS_PER_METRE, length(in.rel.xz));
        let flow = in.current.xy;
        var lie = up;
        if (in.spill.y > 0.0 && abs(slope.y) > 0.0) {
            lie = normalize(slope * sign(slope.y));
        }
        let n = normalize(mix(ripple_normal(world_xz, flow, t), up, calm) + lie - up);
        let fresnel = 0.03 + 0.97 * pow(1.0 - clamp(dot(-view, n), 0.0, 1.0), 5.0);
        let reflected = sky_color(reflect(view, n));
        // Clear and bright-bedded in the shallows, deep blue-green further out.
        let shallow = in.color * 1.35;
        let deep = vec3<f32>(0.02, 0.09, 0.14);
        let body = mix(shallow, deep, 1.0 - exp(-thickness / VOXELS_PER_METRE * 0.2))
            * (G.sun_color.rgb * 0.5 * max(G.sun_dir.y, 0.0) + ambient(n_face));
        color = mix(body, reflected, fresnel);
        alpha = mix(1.0 - exp(-thickness / VOXELS_PER_METRE * 0.5625), 1.0, fresnel);
        // Sun glitter.
        let glint = pow(max(dot(reflect(view, n), G.sun_dir.xyz), 0.0), 240.0);
        color += G.sun_color.rgb * glint * 3.0 * sun_visibility(in.rel, n_face) * (1.0 - calm);
        // Foam streaks on fast water.
        let foam = foam_streaks(world_xz, flow, t) * 0.6 * (1.0 - calm);
        if (foam > 0.0) {
            color = mix(color, white, foam);
            alpha = mix(alpha, 0.95, foam);
        }
        // Where a fall lands the water is churned white, thinning out away from it.
        let foot = in.spill.x;
        if (foot > 0.0) {
            let p = world_xz / VOXELS_PER_METRE;
            let churn = vnoise(p * 7.0 + vec2<f32>(t * 1.3, -t * 0.9)) * 0.6 + vnoise(p * 15.0 - vec2<f32>(t * 1.7, t * 1.1)) * 0.4;
            let wash = smoothstep(0.85 - foot * 1.1, 1.0 - foot * 0.8, churn) * 0.9;
            color = mix(color, white, wash);
            alpha = mix(alpha, 0.95, wash);
        }
        // Further out, water looks the way the far view paints it: opaque, with no bed
        // showing through to mark the rings.
        let far = smoothstep(150.0 * VOXELS_PER_METRE, 300.0 * VOXELS_PER_METRE, length(in.rel.xz));
        let far_fresnel = 0.03 + 0.97 * pow(1.0 - clamp(dot(-view, up), 0.0, 1.0), 5.0);
        let far_body = in.color * (G.sun_color.rgb * max(G.sun_dir.y, 0.0) * 0.5 + ambient(up));
        color = mix(color, mix(far_body, sky_color(reflect(view, up)), far_fresnel), far);
        alpha = mix(alpha, 1.0, far);
        // In deep cold the surface is ice: pale, opaque and still.
        var ice = ice_amount(world_xz, in.snow.y);
        // Running water freezes later than still water, and less: ice creeps out from
        // the slow water by the banks, and the fast middle of a river stays open.
        let swift = length(flow) / MAX_CURRENT;
        ice *= mix(1.0, smoothstep(0.55 + 0.35 * swift, 1.0, in.snow.y) * (1.0 - 0.6 * swift), in.current.z);
        if (ice > 0.0) {
            let sheet = vec3<f32>(0.62, 0.78, 0.88) * (G.sun_color.rgb * 0.7 * max(G.sun_dir.y, 0.0) + ambient(n_face));
            color = mix(color, sheet, ice);
            alpha = mix(alpha, 1.0, ice);
        }
    } else {
        // A sheet of falling water: the surface carried on over the lip, glassy where it
        // leaves and breaking into white streaks as it gathers speed, down to the foam
        // where it lands.
        let d = spill_dir(in.face - FACE_SPILL);
        let s = in.spill.y;
        // The curve's own normal: straight up at the lip, turning outward as it steepens.
        var n = normalize(d * sqrt(max(s - LIP_DIP, 0.0)) + up * 0.25 * (1.0 - smoothstep(2.5, 3.86, s)));
        if (dot(n, view) > 0.0) {
            n = -n;
        }
        let fresnel = 0.03 + 0.97 * pow(1.0 - clamp(dot(-view, n), 0.0, 1.0), 5.0);
        let body = in.color * 1.2 * (G.sun_color.rgb * 0.5 * max(G.sun_dir.y, 0.0) + ambient(up));
        color = mix(body, sky_color(reflect(view, n)), fresnel);
        alpha = mix(0.3, 1.0, fresnel);
        let glint = pow(max(dot(reflect(view, n), G.sun_dir.xyz), 0.0), 240.0);
        color += G.sun_color.rgb * glint * 3.0 * sun_visibility(in.rel, up);
        // The streaks ride down with the water: each is drawn where water that left the
        // lip when it did has fallen to, so they stretch out as they speed up.
        let fall = max(s - LIP_DIP, 0.0) * in.scale / VOXELS_PER_METRE;
        let left = (sqrt(0.64 + 19.6 * fall) - 0.8) / 9.8 - t;
        let along = select(world_xz.x, world_xz.y, abs(d.x) > 0.5) / VOXELS_PER_METRE;
        let streak = vnoise(vec2<f32>(along * 16.0, left * 6.0)) * 0.6 + vnoise(vec2<f32>(along * 37.0 + 9.0, left * 10.0)) * 0.4;
        let broken = 0.2 * smoothstep(0.0, 1.5, fall) + 0.25 * smoothstep(1.5, 6.0, fall) + 0.15 * in.spill.x;
        // At the foot it is all thrown up as spray, to a ragged, shifting edge.
        let spray = smoothstep(0.45, 0.95, in.spill.x + (vnoise(vec2<f32>(along * 9.0, t * 2.5)) - 0.5) * 0.5);
        let foam = max(smoothstep(0.66 - broken, 0.86 - broken, streak), spray) * 0.9;
        color = mix(color, white, foam);
        alpha = mix(alpha, 0.95, foam);
        // In deep cold a fall freezes over too, with a little water still running down it.
        let ice = ice_amount(world_xz, in.snow.y) * 0.75;
        if (ice > 0.0) {
            let sheet = vec3<f32>(0.62, 0.78, 0.88) * (G.sun_color.rgb * 0.7 * max(G.sun_dir.y, 0.0) + ambient(up));
            color = mix(color, sheet, ice);
            alpha = mix(alpha, 1.0, ice);
        }
    }
    return vec4<f32>(apply_fog(color, in.rel), alpha);
}
