// Shared by every scene shader (prepended at pipeline creation).

struct Globals {
    view_proj: mat4x4<f32>,
    // Camera basis scaled so that fwd + ndc.x * right + ndc.y * up is the view ray.
    cam_right: vec4<f32>,
    // w: how tall a pixel is at one voxel of distance, for a screen 1080 lines high.
    cam_up: vec4<f32>,
    // w: the height in metres up to which no ground holds snow this frame, in any biome
    // (see snow_depth in ground.wgsl), or far below the world where some may lie anywhere.
    cam_fwd: vec4<f32>,
    // xyz: direction toward the sun (or moon at night). w: time in seconds.
    sun_dir: vec4<f32>,
    // rgb: light colour. w: 1 by day, 0 by night (moonlight).
    sun_color: vec4<f32>,
    // w: how far round the clouds' cycle of forming and clearing the sky is, radians
    // (see cloud_mix).
    sky_zenith: vec4<f32>,
    // w: how tall the clouds stand for their cover, 0..1, a half the usual (cloud_heap).
    sky_horizon: vec4<f32>,
    // x: fog density per voxel. y: horizontal distance at which voxel terrain hands
    // over to the far view. z: cloud cover (0..1), near enough the share of the sky the
    // clouds fill. w: cloud drift in voxels (already in weather.zw).
    fog: vec4<f32>,
    // xyz: camera world position, wrapped to keep it small. Adding it to a
    // camera-relative position gives a stable world-anchored coordinate.
    cam_wrapped: vec4<f32>,
    // Ground mist: x strength, y height falloff, z the level it is thickest below
    // (camera-relative y). w: the level of the clouds' field above which there is cloud
    // (see cloud_column), which the cover sets.
    mist: vec4<f32>,
    // Cascade far distances (view depth) in xyz; w = cloud layer height, camera-relative.
    cascades: vec4<f32>,
    // xyz: wind vector in voxels per second.
    wind: vec4<f32>,
    // Nearest glowing fireflies: camera-relative xyz, glow strength in w (0 = unused).
    lights: array<vec4<f32>, 4>,
    // The nearest lanterns and fires: camera-relative xyz, w how brightly each burns
    // (1 for a lantern, 0 where there is none).
    lamps: array<vec4<f32>, 4>,
    // x: date, 0..1 from the spring equinox (see season.wgsl). y: snow cover at sea level.
    // z: ice on still water. w: the summit snowline, metres (see ground.wgsl).
    season: vec4<f32>,
    // x: how wet the ground is (0..1). y: how much more snow the hills hold this winter.
    // zw: where the camera stands on the drifting cloud grid, in voxels (see cloud_wrap).
    // (season.yzw and weather.y are the ground where the camera is.)
    weather: vec4<f32>,
    // rgb: the horizon toward the sun. w: how far into dusk or dawn the sky is.
    sky_glow: vec4<f32>,
    // xyz: toward the sun itself, by night too (sun_dir is then the moon). w: how far
    // the stars have wheeled round, radians.
    sun_pos: vec4<f32>,
    // Haze: x density per voxel at sea level, y the height it thins over, w how hazy
    // the air is (0..1), for the sun's glow in it.
    air: vec4<f32>,
    // Each biome's ground, by biome id: x snow at sea level, y ice on still water, z how
    // much more snow the hills hold, w the summit snowline (see ground.wgsl).
    snow: array<vec4<f32>, 24>,
    // The colours small plants give the ground they stand on from afar, by the number a
    // washed face carries (see `wash` in terrain.wgsl): the plant's leaves, then its
    // flowers. rgb: the colour. w: the plant kind in its whole part; in the rest, half of
    // how much of the ground's colour the plant takes (leaves), or half the flowers' share
    // of the plant's colour.
    // The whole part of the flowers' w also holds, above the kind's three bits, the row
    // of `flowering` they keep (0 for none).
    wash_body: array<vec4<f32>, 128>,
    wash_bloom: array<vec4<f32>, 128>,
    // The seasons of the plants that have their own, by the number their vertices carry:
    // start, length, bud, seed (see season_phase in season.wgsl).
    flowering: array<vec4<f32>, 128>,
    // How the rivers run today by their regime (voxel::mesh::Current::regime, 0 steady to
    // 3 fed by snow): x how full with the spring's melt and rain, y how low in the late
    // summer's drought, each 0..1 (season::stream). The water's level never changes; only
    // how it is drawn (see fs_water in terrain.wgsl).
    streams: array<vec4<f32>, 4>,
    // Bare earth today, by biome id: x how muddy in the thaw, y how dusty in a dry
    // summer, 0..1 (season::soil).
    soil: array<vec4<f32>, 24>,
};

struct Shadows {
    light_vp: array<mat4x4<f32>, 3>,
    // For each cascade, how wide its texels are as the sun sees them, and how far along
    // the sun's rays its depth runs, both in voxels (see Renderer::cascade_matrix).
    texel: vec4<f32>,
    span: vec4<f32>,
};

@group(0) @binding(0) var<uniform> G: Globals;
@group(0) @binding(1) var<uniform> S: Shadows;
@group(0) @binding(2) var shadow_maps: texture_depth_2d_array;
@group(0) @binding(3) var shadow_sampler: sampler_comparison;
// The clouds' field (render::cloud_field): for each cell of their grid two patterns of
// heaps and where the cell is in the cycle from the one to the other (see cloud_mix).
// Each coarser level holds enough of the squares of cells under it to say how high the
// field can stand in them (cloud_bound): level CLOUD_LEVEL in squares of CLOUD_COARSE,
// the last level (CLOUD_LEVELS - 1) in the whole grid.
@group(0) @binding(4) var cloud_field: texture_2d<f32>;

fn hash3(p: vec3<i32>) -> f32 {
    var h = bitcast<u32>(p.x) * 0x8da6b343u
          + bitcast<u32>(p.y) * 0xd8163841u
          + bitcast<u32>(p.z) * 0xcb1ab31fu;
    h ^= h >> 15u;
    h *= 0x2c1b3c6du;
    h ^= h >> 12u;
    h *= 0x297a2d39u;
    h ^= h >> 15u;
    return f32(h >> 8u) * (1.0 / 16777216.0);
}

// Smooth value noise over a 2D lattice.
fn vnoise(p: vec2<f32>) -> f32 {
    let i = vec2<i32>(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash3(vec3<i32>(i.x, i.y, 7));
    let b = hash3(vec3<i32>(i.x + 1, i.y, 7));
    let c = hash3(vec3<i32>(i.x, i.y + 1, 7));
    let d = hash3(vec3<i32>(i.x + 1, i.y + 1, 7));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var a = 0.5;
    var q = p;
    for (var i = 0; i < 4; i++) {
        v += a * vnoise(q);
        q = q * 2.03 + vec2<f32>(17.0, 31.0);
        a *= 0.5;
    }
    return v;
}

// Clouds are built of cubes, like the land, CLOUD_CELL across and CLOUD_STEP tall, on a
// grid that is anchored to the world and drifts with the wind. Over each cell of the grid
// stands one column of them: up from the cloud base (G.cascades.w, camera-relative) in a
// heap, highest in the middle of a cloud and stepping down to its edge, and a block or two
// down below the base under the thick of it. So a cloud has a flat bottom with a stepped
// rim, sides that stand straight, and a top that rises in terraces. G.weather.zw is where
// the camera stands on the grid, in voxels, wrapped every CLOUD_PERIOD cells (and so is
// the pattern, so the wrap never shows).

// A cell's place on the grid, wrapped into 0..CLOUD_PERIOD.
fn cloud_wrap(c: vec2<i32>) -> vec2<i32> {
    return ((c % CLOUD_PERIOD) + CLOUD_PERIOD) % CLOUD_PERIOD;
}

// The column of cloud over a (wrapped) cell: how many blocks below the base it hangs
// (x, zero or less) and how many above it stands (y); both 0 where there is none.
// The field (render::cloud_field) is high in the middle of each heap, and G.mist.w is the
// level the cover leaves its edge at: all the field above that is cloud, thicker the
// further above. The square root makes the sides steep and the tops broad, as a heap of
// cumulus is.
fn cloud_column(c: vec2<i32>) -> vec2<f32> {
    return cloud_heap(cloud_mix(textureLoad(cloud_field, c, 0)));
}

// The field in a cell (a texel of its finest level) as the cycle stands now: part of the
// way from the one pattern of heaps to the other, as far as this stretch of sky has got.
// Where the second pattern rises past the first a cloud forms, from its core outward, and
// where it falls away one thins and breaks up. As render::cloud_mix.
fn cloud_mix(t: vec4<f32>) -> f32 {
    return mix(t.r, t.g, 0.5 - 0.5 * cos(G.sky_zenith.w + t.b * CLOUD_TURNS));
}

// The highest the field can stand anywhere in the square of cells a texel of a coarser
// level covers, as the cycle stands now: as cloud_mix at the middle of the square's
// places in the cycle, the highest of each pattern, and room for how far from the
// middle any cell's place is.
fn cloud_bound(t: vec4<f32>) -> f32 {
    let top = max(t.r, t.g);
    let off = min(t.a * CLOUD_TURNS * 0.5, 1.0);
    return min(cloud_mix(t) + off * top, top);
}

// The column where the field stands at `v`. It only grows with `v`, so the highest value
// in a stretch of the grid gives the tallest and deepest column in it. Heaps tower over
// the same cover ahead of rain and sink as it clears (G.sky_horizon.w).
fn cloud_heap(v: f32) -> vec2<f32> {
    let d = v - G.mist.w;
    if (d <= 0.0) {
        return vec2<f32>(0.0);
    }
    let top = clamp(ceil(CLOUD_STEPS * sqrt(d * 3.2 * (0.4 + 1.2 * G.sky_horizon.w))), 1.0, CLOUD_STEPS);
    let belly = floor(CLOUD_BELLY * smoothstep(0.03, 0.14, d) + 0.5);
    return vec2<f32>(-belly, top);
}

// The cloud over a camera-relative xz position: its column, as cloud_column gives it.
fn cloud_column_at(rel_xz: vec2<f32>) -> vec2<f32> {
    return cloud_column(cloud_wrap(vec2<i32>(floor((rel_xz + G.weather.zw) / CLOUD_CELL))));
}

// The clouds as they fall on the ground: darker under a thick cloud, with an edge that
// fades over a few cells, not the cloud's own blocky one. Light through the rim of a
// cloud is softened by the cloud's thin stuff and by the breadth of the sun, and as a
// cloud drifts over the sun the light goes, and comes back, over a few seconds rather
// than at once. The field between the four nearest cells, as this runs for every pixel
// of terrain.
fn cloud_shadow(rel_xz: vec2<f32>) -> f32 {
    let p = (rel_xz + G.weather.zw) / CLOUD_CELL - 0.5;
    let c = vec2<i32>(floor(p));
    let f = p - floor(p);
    let t = mix(
        mix(textureLoad(cloud_field, cloud_wrap(c), 0), textureLoad(cloud_field, cloud_wrap(c + vec2<i32>(1, 0)), 0), f.x),
        mix(textureLoad(cloud_field, cloud_wrap(c + vec2<i32>(0, 1)), 0), textureLoad(cloud_field, cloud_wrap(c + vec2<i32>(1, 1)), 0), f.x),
        f.y);
    let d = cloud_mix(t) - G.mist.w;
    // How thick the cloud is, as cloud_heap has it but without its steps.
    let thick = CLOUD_STEPS * sqrt(max(d, 0.0) * 3.2 * (0.4 + 1.2 * G.sky_horizon.w)) + CLOUD_BELLY * smoothstep(0.03, 0.14, d);
    return smoothstep(-0.035, 0.045, d) * min(0.5 + 0.03 * thick, 0.86);
}

// The fine part of cloud_trace: a view ray's way across the cells of one coarse square of
// the grid, from `ta` to `tb` along it, having come in through a face whose normal is `n`.
fn cloud_trace_fine(dir: vec3<f32>, ta: f32, tb: f32, n_in: vec3<f32>) -> vec4<f32> {
    let b0 = G.cascades.w;
    let pc = (dir.xz * (ta + 0.01) + G.weather.zw) / CLOUD_CELL;
    var cell = vec2<i32>(floor(pc));
    let up = dir.xz >= vec2<f32>(0.0);
    let step = select(vec2<i32>(-1), vec2<i32>(1), up);
    let span = CLOUD_CELL / max(abs(dir.xz), vec2<f32>(1e-6));
    let into = pc - floor(pc);
    var next = ta + select(into, 1.0 - into, up) * span;
    var t = ta;
    var n = n_in;
    for (var i = 0; i < 2 * CLOUD_COARSE + 2; i++) {
        let out = min(min(next.x, next.y), tb);
        let col = cloud_column(cloud_wrap(cell));
        if (col.y > 0.0) {
            let top = b0 + col.y * CLOUD_STEP;
            let bottom = b0 + col.x * CLOUD_STEP;
            let ya = dir.y * t;
            let yb = dir.y * out;
            // In the column on coming into the cell: through its side (or, at the very
            // start, the camera is in it).
            if (ya <= top && ya >= bottom) {
                return vec4<f32>(n, t);
            }
            // Or down through its top, or up through its bottom, before leaving the cell.
            if (ya > top && yb <= top) {
                return vec4<f32>(0.0, 1.0, 0.0, top / dir.y);
            }
            if (ya < bottom && yb >= bottom) {
                return vec4<f32>(0.0, -1.0, 0.0, bottom / dir.y);
            }
        }
        if (out >= tb) {
            break;
        }
        if (next.x < next.y) {
            cell.x += step.x;
            t = next.x;
            next.x += span.x;
            n = vec3<f32>(-f32(step.x), 0.0, 0.0);
        } else {
            cell.y += step.y;
            t = next.y;
            next.y += span.y;
            n = vec3<f32>(0.0, 0.0, -f32(step.y));
        }
    }
    return vec4<f32>(0.0, 0.0, 0.0, -1.0);
}

// Where a view ray from the camera first meets a cloud, within `far`: xyz the face it
// meets (zero if the camera is inside one), w how far along the ray, or -1 for none.
// It steps across the grid as a voxel raycast does, through the slab the clouds stand
// in: first over squares of CLOUD_COARSE cells, whose highest value the field's coarser
// level holds, and only in a square that holds any cloud from cell to cell. It gives up
// after CLOUD_MARCH squares: that far off, a ray grazing the slab is lost in haze anyway.
fn cloud_trace(dir: vec3<f32>, far: f32) -> vec4<f32> {
    let miss = vec4<f32>(0.0, 0.0, 0.0, -1.0);
    if (G.fog.z < 0.01) {
        return miss;
    }
    // The slab the clouds stand in today: as deep and as tall as the tallest of them all,
    // which the field's last level holds.
    let whole = textureLoad(cloud_field, vec2<i32>(0), CLOUD_LEVELS - 1);
    let tallest = cloud_heap(max(whole.r, whole.g));
    if (tallest.y <= 0.0) {
        return miss;
    }
    let b0 = G.cascades.w + tallest.x * CLOUD_STEP;
    let b1 = G.cascades.w + tallest.y * CLOUD_STEP;
    var t0 = 0.0;
    var t1 = far;
    var n = vec3<f32>(0.0);
    if (abs(dir.y) < 1e-5) {
        if (b0 > 0.0 || b1 < 0.0) {
            return miss;
        }
    } else {
        let ta = b0 / dir.y;
        let tb = b1 / dir.y;
        let lo = min(ta, tb);
        if (lo > 0.0) {
            // From outside the slab, in through its floor or its roof.
            t0 = lo;
            n = vec3<f32>(0.0, -sign(dir.y), 0.0);
        }
        t1 = min(t1, max(ta, tb));
    }
    if (t0 >= t1) {
        return miss;
    }
    let size = CLOUD_CELL * f32(CLOUD_COARSE);
    let pc = (dir.xz * t0 + G.weather.zw) / size;
    var cell = vec2<i32>(floor(pc));
    let up = dir.xz >= vec2<f32>(0.0);
    let step = select(vec2<i32>(-1), vec2<i32>(1), up);
    // How far along the ray one square is, each way, and to the first edge each way.
    let span = size / max(abs(dir.xz), vec2<f32>(1e-6));
    let into = pc - floor(pc);
    var next = t0 + select(into, 1.0 - into, up) * span;
    var t = t0;
    let squares = CLOUD_PERIOD / CLOUD_COARSE;
    for (var i = 0; i < CLOUD_MARCH; i++) {
        let out = min(min(next.x, next.y), t1);
        let c = ((cell % squares) + squares) % squares;
        // The tallest column in the square: the ray may pass clean over or under it all.
        let heap = cloud_heap(cloud_bound(textureLoad(cloud_field, c, CLOUD_LEVEL)));
        if (heap.y > 0.0) {
            let top = G.cascades.w + heap.y * CLOUD_STEP;
            let bottom = G.cascades.w + heap.x * CLOUD_STEP;
            let ya = dir.y * t;
            let yb = dir.y * out;
            if (min(ya, yb) <= top && max(ya, yb) >= bottom) {
                let hit = cloud_trace_fine(dir, t, out, n);
                if (hit.w >= 0.0) {
                    return hit;
                }
            }
        }
        if (out >= t1) {
            break;
        }
        if (next.x < next.y) {
            cell.x += step.x;
            t = next.x;
            next.x += span.x;
            n = vec3<f32>(-f32(step.x), 0.0, 0.0);
        } else {
            cell.y += step.y;
            t = next.y;
            next.y += span.y;
            n = vec3<f32>(0.0, 0.0, -f32(step.y));
        }
    }
    return miss;
}

// How much of the sun gets to a point on a cloud past the cloud between it and the sun:
// the heaps toward the sun, looked for at three distances, each taking some of it. So
// the sunward side of a heap is bright, the far side and the hollows between heaps grey.
fn cloud_sunlit(p: vec3<f32>, sun: vec3<f32>) -> f32 {
    var lit = 1.0;
    let flat_sun = sun / max(length(sun.xz), 0.25);
    for (var i = 1; i <= 3; i++) {
        let q = p + flat_sun * (CLOUD_CELL * (f32(i * i) + 0.5));
        let col = cloud_column_at(q.xz);
        let over = G.cascades.w + col.y * CLOUD_STEP - q.y;
        if (col.y > 0.0 && over > 0.0) {
            lit *= 1.0 - 0.32 * smoothstep(0.0, 2.0 * CLOUD_STEP, over);
        }
    }
    return lit;
}

// How much of a cloud shows where a view ray meets it: all of it, but for the thin outer
// rim, which the sky shows through a little. Set by cloud_seen.
var<private> cloud_alpha: f32;

// The cloud a view ray meets within `far`: its colour in rgb, before any haze, and in w
// how far along the ray, or -1 for none.
fn cloud_seen(dir: vec3<f32>, far: f32) -> vec4<f32> {
    cloud_alpha = 1.0;
    let hit = cloud_trace(dir, far);
    if (hit.w < 0.0) {
        return hit;
    }
    let n = hit.xyz;
    let p = dir * hit.w;
    let base = G.cascades.w;
    // The column the face belongs to, a little way in from it.
    let inside = p - n * (0.25 * CLOUD_CELL);
    let cell = vec2<i32>(floor((inside.xz + G.weather.zw) / CLOUD_CELL));
    let col = cloud_column(cloud_wrap(cell));
    let thick = max(col.y - col.x, 1.0);
    // Which block up the column, and how far up that block.
    let level = (inside.y - base) / CLOUD_STEP;
    let row = floor(level);
    // How high up its own cloud, 0 at the bottom and 1 at the top of the column.
    let up = clamp((level - col.x) / thick, 0.0, 1.0);
    // And how high in the whole layer: the tall heaps stand in more light.
    let high = clamp(level / CLOUD_STEPS, 0.0, 1.0);
    let sun = G.sun_dir.xyz;
    let shade = cloud_sunlit(p + n * (0.3 * CLOUD_CELL), sun);
    // A cloud is lit through as well as on: its faces turned from the sun are not dark.
    let facing = dot(n, sun);
    let lit_by = (0.3 + 0.7 * max(facing, 0.0)) * shade;
    // The sky's light, most from above. Cloud is white, so it takes the sky's colour
    // where the sun does not reach: blue-grey in the shade and violet underneath.
    let sky = mix(G.sky_horizon.rgb, G.sky_zenith.rgb, 0.45 + 0.35 * n.y);
    // Each block a shade its own, and each darker toward its foot, so the blocks tell.
    let own = 0.96 + 0.08 * hash3(vec3<i32>(cell.x, i32(row), cell.y));
    let foot = 0.93 + 0.07 * fract(level);
    // How far above the edge of its cloud the field stands here.
    let d = cloud_mix(textureLoad(cloud_field, cloud_wrap(cell), 0)) - G.mist.w;
    var color = vec3<f32>(0.0);
    if (n.y < -0.5) {
        // The flat underside, in the cloud's own shade: greyer the thicker the cover, and
        // the thicker the cloud above, so from below its heaps show in the grey.
        let deep = clamp(thick / (CLOUD_STEPS + CLOUD_BELLY), 0.0, 1.0);
        // Under a sky all cloud every column is as thick as the next, and the heavier
        // stretches of it show by the field itself, in steps.
        let heavy = floor(clamp(d - 0.3, 0.0, 0.3) * 20.0) / 6.0;
        color = (G.sun_color.rgb * 0.13 + sky * vec3<f32>(0.5, 0.5, 0.58)) * (1.0 - 0.22 * G.fog.z) * (1.0 - 0.3 * deep) * (1.0 - 0.22 * heavy) * own;
    } else if (dot(n, n) < 0.5) {
        // Inside the cloud: grey-white all round.
        color = G.sun_color.rgb * 0.45 + sky * 0.7;
    } else if (n.y > 0.5) {
        // Tops: the brightest of it, the low ones among taller heaps a little less.
        color = (G.sun_color.rgb * (0.3 + 1.05 * lit_by) + sky * 0.5) * (0.9 + 0.1 * high) * own;
    } else {
        // Sides: darker toward the foot of the cloud, where less of the sky is seen.
        // The foot of a cloud is nearly as dark as its underside.
        let rise = smoothstep(0.0, 1.0, up);
        color = (G.sun_color.rgb * (0.12 + 0.2 * rise + 0.95 * lit_by * (0.45 + 0.55 * rise)) + sky * vec3<f32>(0.5, 0.5, 0.56)) * foot * own;
    }
    // At dusk and dawn the low sun catches the cloud on its own side of the sky, and it
    // takes the glow's colour in place of its own, not on top of it: added, the two came
    // out white.
    let toward = sunward(dir);
    let glow = G.sky_glow.rgb * (0.75 + 0.35 * lit_by + 0.1 * n.y);
    color = mix(color, glow, G.sky_glow.w * 0.7 * toward * toward * select(1.0, 0.5, n.y < -0.5));
    // The outermost blocks of a cloud are thin stuff, and the sky shows through them.
    cloud_alpha = 0.72 + 0.28 * smoothstep(0.0, 0.035, d);
    // Moonlit cloud is only a little lighter than the night sky.
    return vec4<f32>(color * (0.55 + 0.45 * G.sun_color.w), hit.w);
}

// Where voxel terrain hands over to the far view. Both sides call this, so every
// pixel is drawn by exactly one of them.
//
// The edge has to be a plain circle around the camera. Voxel terrain has no walls
// where it is cut off, so if far terrain ever lay between the camera and voxel
// terrain (as it would along a ragged or dithered edge) the view would pass under the
// voxels' cut edge and out the other side.
fn beyond_voxels(rel: vec3<f32>) -> bool {
    return length(rel.xz) >= G.fog.y;
}

// How far round toward the sun a direction lies: 1 facing it, 0 with it behind.
fn sunward(dir: vec3<f32>) -> f32 {
    let d = dir.xz / max(length(dir.xz), 1e-4);
    let s = G.sun_pos.xz / max(length(G.sun_pos.xz), 1e-4);
    return dot(d, s) * 0.5 + 0.5;
}

// The glow of hazy air around the sun (or moon), for the cosine of the angle from it.
fn sun_haze(s: f32) -> vec3<f32> {
    let c = max(s, 0.0);
    let c4 = c * c * c * c;
    let c32 = c4 * c4 * c4 * c4 * c4 * c4 * c4 * c4;
    return G.sun_color.rgb * (c4 * 0.05 + c32 * 0.20) * G.air.w;
}

// The sky without the sun disc or the glow of haze near the horizon.
fn sky_base(dir: vec3<f32>) -> vec3<f32> {
    let up = clamp(dir.y, -1.0, 1.0);
    let t = pow(1.0 - max(up, 0.0), 3.0);
    // The horizon is warm round the sun at dusk and dawn, and cool opposite it.
    let toward = sunward(dir);
    let horizon = mix(G.sky_horizon.rgb, G.sky_glow.rgb, toward * toward);
    var c = mix(G.sky_zenith.rgb, horizon, t);
    if (up < 0.0) {
        c = mix(c, horizon * 0.7, clamp(-up * 4.0, 0.0, 1.0));
    }
    let s = max(dot(dir, G.sun_dir.xyz), 0.0);
    c += G.sun_color.rgb * (pow(s, 6.0) * 0.03 + pow(s, 48.0) * 0.12);
    return c;
}

// Sky radiance without the sun disc. Its horizon is what the far distance fades to
// (see apply_fog), so land dissolves into exactly the sky behind it.
fn sky_color(dir: vec3<f32>) -> vec3<f32> {
    let low = 1.0 - max(dir.y, 0.0);
    let low2 = low * low;
    return sky_base(dir) + sun_haze(dot(dir, G.sun_dir.xyz)) * low2 * low2 * low2;
}

// Clear air scatters blue most, so it is the blue of a distant thing that goes first.
const AIR_TINT: vec3<f32> = vec3<f32>(0.45, 0.9, 1.7);

// The mean along a straight path of a layer that is 1 up to height 0 and falls off as
// exp(-y) above it, between heights y0 and y1 (in units of its falloff).
fn layer_mean(y0: f32, y1: f32) -> f32 {
    let lo = min(y0, y1);
    let hi = max(y0, y1);
    if (hi - lo < 0.01) {
        return exp(-max(lo, 0.0));
    }
    // Below the level the layer is full; above it, the integral of exp(-y).
    let under = max(min(hi, 0.0) - lo, 0.0);
    let over = exp(-max(lo, 0.0)) - exp(-max(hi, 0.0));
    return (under + over) / (hi - lo);
}

// How much haze and mist lie between the camera and a point. Both thin with height, so
// a valley fills while the ridge above stands clear, and seen from above the mist is a
// thin sheet lying in the low ground.
fn haze_depth(rel: vec3<f32>, dist: f32) -> f32 {
    let haze = G.air.x * layer_mean(G.cam_wrapped.y / G.air.y, (G.cam_wrapped.y + rel.y) / G.air.y);
    let mist = G.mist.x * layer_mean(-G.mist.z / G.mist.y, (rel.y - G.mist.z) / G.mist.y);
    return (haze + mist) * dist;
}

// How far into the night it is, 0 by day to 1 once the sun is well down: smooth through
// dusk, where the switch from sun to moon is not.
fn darkness() -> f32 {
    return smoothstep(0.08, -0.14, G.sun_pos.y);
}

// The eye after dark sees little colour: what only the moon and the night sky light goes
// a cool grey, while whatever a fire or a lantern lights, being brighter and warmer than
// the moon could make it, keeps its own. So the warm light stands out in a cold, dim world.
fn night_eye(c: vec3<f32>) -> vec3<f32> {
    let dark = darkness();
    if (dark <= 0.0) {
        return c;
    }
    let lum = dot(c, vec3<f32>(0.3, 0.59, 0.11));
    let warm = smoothstep(0.0, 0.6, (c.r - c.b) / max(lum, 1e-5));
    let dim = (1.0 - smoothstep(0.03, 0.2, lum)) * (1.0 - warm);
    return mix(c, vec3<f32>(0.78, 0.9, 1.18) * lum, dark * dim * 0.7);
}

fn apply_fog(surface: vec3<f32>, at: vec3<f32>) -> vec3<f32> {
    return apply_fog_lit(surface, at, 1.0);
}

// How much the light the air adds before a far surface goes with how well lit the surface
// is, by day (see apply_fog_lit).
const AIR_SHADE: f32 = 0.4;

// How well lit a surface is beside level open ground in full sun, for apply_fog_lit:
// `light` is the light falling on it.
fn lit_beside_level(light: vec3<f32>) -> f32 {
    let level = G.sun_color.rgb * max(G.sun_dir.y, 0.0) + ambient(vec3<f32>(0.0, 1.0, 0.0));
    let w = vec3<f32>(0.3, 0.59, 0.11);
    return clamp(dot(light, w) / max(dot(level, w), 1e-4), 0.25, 1.4);
}

// apply_fog for a surface `lit` as well as level ground in the sun is (lit_beside_level).
// The air just before a hillside in shade, or down in a valley, is in that shade too, so
// it adds less light than the air before one in the sun: the shade on far hills shows
// through the haze as a darker blue, where with the same veil over everything the
// distance went one pale brightness. The sun's glow in the haze is from all along the
// way and keeps its brightness. Through so much air that the land is all but gone the
// veil is the sky's colour again, so the farthest hills still melt into it.
fn apply_fog_lit(surface: vec3<f32>, at: vec3<f32>, lit: f32) -> vec3<f32> {
    var color = night_eye(surface);
    var rel = at;
    var dist = length(rel);
    // Cloud between the camera and the point (from above the clouds, or a summit up in
    // them) hides it, and is what the haze lies over instead. Only where the line of
    // sight reaches the cloud slab, which from the ground under the clouds it never does.
    let b0 = G.cascades.w - CLOUD_BELLY * CLOUD_STEP;
    if (G.fog.z >= 0.01 && max(rel.y, 0.0) >= b0 && min(rel.y, 0.0) <= G.cascades.w + CLOUD_STEPS * CLOUD_STEP) {
        let dir = rel / max(dist, 1e-4);
        let cloud = cloud_seen(dir, dist);
        if (cloud.w >= 0.0) {
            color = mix(color, cloud.rgb, cloud_alpha);
            // (Not quite nothing inside a cloud, so the haze below has a direction.)
            dist = max(cloud.w, 1.0);
            rel = dir * dist;
        }
    }
    let haze = haze_depth(rel, dist);
    let through = exp(-(dist * G.fog.x * AIR_TINT + haze));
    // What the air adds is the sky at the horizon in that direction, and where there
    // is haze, the sun's glow in it.
    let dir = normalize(vec3<f32>(rel.x, abs(rel.y) * 0.15 + dist * 0.02, rel.z));
    let glow = sun_haze(dot(rel, G.sun_dir.xyz) / max(dist, 1e-4)) * (1.0 - exp(-haze));
    let seen = dot(through, vec3<f32>(0.3, 0.59, 0.11));
    let shade = mix(1.0, lit, AIR_SHADE * G.sun_color.w * smoothstep(0.02, 0.35, seen));
    return color * through + (air_light(dir, through) * shade + glow) * (1.0 - through);
}

// The light the air between the eye and a far thing gives off toward the eye. While the
// sun is up it lights the low air as it lights the sky, and that is the horizon's colour.
// Once it has set, the bright horizon is high air the sun still reaches, far off over
// the edge of the world; the air down among the hills has only the dark sky above to
// light it. A ridge a few kilometres off at dusk, or under the moon, is then a dim shape
// against the sky, where it had faded into the glow and so looked lit as if by day.
// Only through so much air that the land is all but gone (`through` near 0) is the
// colour the sky's again, so that the farthest land still dissolves into the sky behind.
fn air_light(dir: vec3<f32>, through: vec3<f32>) -> vec3<f32> {
    let sky = sky_base(dir);
    let sunk = smoothstep(0.0, -0.1, G.sun_pos.y);
    if (sunk <= 0.0) {
        return sky;
    }
    // The sky overhead and round about, as lights the ground (see `ambient`).
    let dome = min(mix(mix(G.sky_horizon.rgb, G.sky_glow.rgb, 0.4), G.sky_zenith.rgb, 0.6), sky);
    let gone = 1.0 - through;
    let deep = gone * gone * gone * gone;
    return mix(sky, mix(dome, sky, deep), sunk);
}

// Warm light from nearby fireflies, falling off over about eight metres.
fn local_lights(rel: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    for (var i = 0; i < 4; i++) {
        let l = G.lights[i];
        let d = l.xyz - rel;
        let dist = length(d) / VOXELS_PER_METRE;
        let wrap = 0.35 + 0.65 * max(dot(n, d / max(length(d), 0.001)), 0.0);
        let reach = clamp(1.0 - dist / 8.0, 0.0, 1.0);
        sum += vec3<f32>(0.85, 1.0, 0.35) * (l.w * 0.045) * wrap * reach * reach / (1.0 + dist * dist * 0.6);
    }
    // Lanterns: a warm light that carries a long way after dark, and hardly shows by day.
    // It comes up through dusk rather than all at once when the moon takes over.
    let night = 1.0 - 0.9 * (1.0 - darkness());
    for (var i = 0; i < 4; i++) {
        let l = G.lamps[i];
        if (l.w > 0.01) {
            let d = l.xyz - rel;
            let dist = length(d) / VOXELS_PER_METRE;
            let wrap = 0.3 + 0.7 * max(dot(n, d / max(length(d), 0.001)), 0.0);
            let reach = clamp(1.0 - dist / 16.0, 0.0, 1.0);
            sum += vec3<f32>(1.0, 0.68, 0.32) * (2.2 * night * l.w) * wrap * reach * reach / (1.0 + dist * dist * 0.25);
        }
    }
    return sum;
}

fn ambient(n: vec3<f32>) -> vec3<f32> {
    let sky = mix(mix(G.sky_horizon.rgb, G.sky_glow.rgb, 0.4), G.sky_zenith.rgb, 0.6) * (0.55 + 0.45 * n.y) * 0.8;
    // Light bounced off the ground, tinted warm, scaled by how much sun there is.
    // Reaches sideways and downward faces, which the sky lights least.
    let bounce = G.sun_color.rgb * vec3<f32>(0.20, 0.17, 0.12) * (0.65 - 0.35 * n.y)
        * max(G.sun_dir.y, 0.0);
    return sky + bounce;
}

// A hollow byte (farview::hollow) per unit of its measure: the rise from a point to the
// land round it, over the distance to that land (HOLLOW_SCALE in farview.rs).
const HOLLOW_SCALE: f32 = 320.0;
// How much less of the sky lights land per unit of that measure.
const HOLLOW_SHADE: f32 = 3.5;

// A hollow byte, 0..255, as its measure.
fn hollow_measure(byte: f32) -> f32 {
    return (byte - 128.0) / HOLLOW_SCALE;
}

// How much of the sky (x) and of the sun (y) reaches land lying `hollow` deep in a fold of
// the hills, negative on a ridge, out where the shadow maps and the voxels' own corners no
// longer shade it. Less of the sky reaches down into a valley than onto a ridge, and less
// of the light bounced about; at a low sun a valley is half in the shade of its sides.
// That is what shows the shape of distant hills, whichever way the sun is: without it a
// far hillside was one brightness from its ridge to the stream at its foot.
fn hollow_light(hollow: f32) -> vec2<f32> {
    let sky = clamp(1.0 - hollow * HOLLOW_SHADE, 0.4, 1.3);
    return vec2<f32>(sky, mix(1.0, sky, 0.6));
}

// How much sun the clouds let through to a point.
fn cloud_light(rel: vec3<f32>) -> f32 {
    // Where the sun's ray through this point crosses the cloud layer.
    // Not above it: a point up among the clouds takes the cloud it is in.
    let to_cloud = max(G.cascades.w - rel.y, 0.0) / max(G.sun_dir.y, 0.05);
    return 1.0 - cloud_shadow(rel.xz + G.sun_dir.xz * to_cloud);
}

// How much sun reaches ground beyond the shadow maps, whose skylines toward sunrise and
// sunset are known (farview::Skylines, each byte as 0..1): dark once the sun is below
// the hills on its side, with an edge that is softer the higher the hills.
fn skyline_light(skylines: vec2<f32>) -> f32 {
    return skyline_side_light(select(skylines.y, skylines.x, G.sun_dir.x > 0.0));
}

// The same for the one skyline on the sun's side, as its byte, 0..1.
fn skyline_side_light(packed: f32) -> f32 {
    let skyline = packed * packed * 1.5;
    let sun = G.sun_dir.y / max(length(G.sun_dir.xz), 1e-3);
    let soft = 0.025 + skyline * 0.12;
    return smoothstep(skyline - soft, skyline + soft, sun);
}

fn sun_visibility(rel: vec3<f32>, n: vec3<f32>) -> f32 {
    return sun_reaching(rel, n, 1.0);
}

const SHADOW_SIZE: f32 = 2048.0;
// How wide the sun is taken to be: how far a shadow's edge spreads for each voxel
// between what casts it and what it falls on. Wider than the real sun, for soft light.
const SUN_WIDTH: f32 = 0.045;

// What one shadow map says of a point. x: how much sun reaches it. y: 1 if the point
// lies in the map at all.
fn cascade_light(c: i32, rel: vec3<f32>, n: vec3<f32>) -> vec2<f32> {
    let texel = S.texel[c];
    // Push the lookup off the surface along its normal and toward the light so
    // flat faces do not shadow themselves.
    // No more than 1: a normal that is the sun's own direction (light through a leaf)
    // rounds to a hair over it on some frames, and the slope below would have no value.
    let ndl = clamp(dot(n, G.sun_dir.xyz), 0.0, 1.0);
    let offset = rel + n * texel * 1.5 * (1.0 - ndl * 0.5) + G.sun_dir.xyz * texel * 0.5;
    let lp = S.light_vp[c] * vec4<f32>(offset, 1.0);
    let uv = lp.xy * vec2<f32>(0.5, -0.5) + 0.5;
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) {
        return vec2<f32>(1.0, 0.0);
    }
    let span = S.span[c];
    // A shadow is sharp where it meets what casts it and spreads with distance from it:
    // a trunk's is crisp at its foot, a high canopy's is soft. So look for what stands
    // between this point and the sun, and how far off it is.
    var radius = 1.0;
    if (c < 2) {
        let at = vec2<i32>(uv * SHADOW_SIZE);
        var looks = array<vec2<i32>, 5>(
            vec2<i32>(0, 0), vec2<i32>(3, 2), vec2<i32>(-2, 3), vec2<i32>(-3, -2), vec2<i32>(2, -3));
        var gap = 0.0;
        var found = 0.0;
        for (var k = 0; k < 5; k++) {
            let p = clamp(at + looks[k], vec2<i32>(0), vec2<i32>(2047));
            let z = textureLoad(shadow_maps, p, c, 0);
            if (z < lp.z) {
                gap += lp.z - z;
                found += 1.0;
            }
        }
        if (found > 0.0) {
            radius = clamp(gap / found * span * SUN_WIDTH / texel, 1.0, select(2.5, 5.0, c == 0));
        }
    }
    // A wide filter on a face turned from the sun would reach under the face itself.
    let slope = sqrt(1.0 - ndl * ndl) / max(ndl, 0.25);
    let z = lp.z - (radius - 1.0) * texel * 0.5 * slope / span;
    // Taps spread evenly over a disc, more of them where it is widest. Each cascade has a
    // loop of its own with a fixed count, so it unrolls and its offsets are constants.
    let reach = radius / SHADOW_SIZE;
    var sum = 0.0;
    var taps = 5.0;
    if (c == 0) {
        taps = 12.0;
        for (var i = 0; i < 12; i++) {
            sum += textureSampleCompareLevel(shadow_maps, shadow_sampler, uv + PCF_TAPS_12[i] * reach, c, z);
        }
    } else if (c == 1) {
        taps = 8.0;
        for (var i = 0; i < 8; i++) {
            sum += textureSampleCompareLevel(shadow_maps, shadow_sampler, uv + PCF_TAPS_8[i] * reach, c, z);
        }
    } else {
        for (var i = 0; i < 5; i++) {
            sum += textureSampleCompareLevel(shadow_maps, shadow_sampler, uv + PCF_TAPS_5[i] * reach, c, z);
        }
    }
    return vec2<f32>(sum / taps, 1.0);
}

// The taps of cascade_light's filter over a disc of radius 1, for 12, 8 and 5 of them:
// tap i of n at sqrt((i + 0.5) / n) from the middle, turned i * 2.39996 radians (the golden
// angle), so they spread evenly whatever their number. Worked out once, in f32, rather
// than with a cos, sin and sqrt for every tap of every pixel.
const PCF_TAPS_12 = array<vec2<f32>, 12>(
    vec2<f32>(0.204124153, 0.0),
    vec2<f32>(-0.260698497, 0.238822713),
    vec2<f32>(0.0399013013, -0.454688042),
    vec2<f32>(0.328598619, 0.428590238),
    vec2<f32>(-0.6030128, -0.106656536),
    vec2<f32>(0.571219087, -0.363375992),
    vec2<f32>(-0.191050008, 0.710750699),
    vec2<f32>(-0.364394337, -0.701581657),
    vec2<f32>(0.790564001, 0.288689852),
    vec2<f32>(-0.822432637, 0.339516312),
    vec2<f32>(0.396443814, -0.847249866),
    vec2<f32>(0.293014765, 0.93406409));
const PCF_TAPS_8 = array<vec2<f32>, 8>(
    vec2<f32>(0.25, 0.0),
    vec2<f32>(-0.319289148, 0.29249689),
    vec2<f32>(0.0488689169, -0.556876838),
    vec2<f32>(0.402449489, 0.524913669),
    vec2<f32>(-0.738536775, -0.130627036),
    vec2<f32>(0.699597657, -0.445042878),
    vec2<f32>(-0.233987495, 0.870488286),
    vec2<f32>(-0.446290076, -0.859258533));
const PCF_TAPS_5 = array<vec2<f32>, 5>(
    vec2<f32>(0.316227764, 0.0),
    vec2<f32>(-0.403872401, 0.36998257),
    vec2<f32>(0.0618148297, -0.704399705),
    vec2<f32>(0.509062827, 0.663969159),
    vec2<f32>(-0.9341833, -0.165231586));

// Fraction of direct sunlight reaching a point: cascaded shadow maps, filtered wider
// the further the shadow has fallen, then cloud shadow. `beyond` is what to take it as
// past the last shadow map.
fn sun_reaching(rel: vec3<f32>, n: vec3<f32>, beyond: f32) -> f32 {
    let depth = dot(rel, G.cam_fwd.xyz);
    var vis = beyond;
    if (depth < G.cascades.z) {
        var c = 0;
        var end = G.cascades.x;
        if (depth >= G.cascades.y) {
            c = 2;
            end = G.cascades.z;
        } else if (depth >= G.cascades.x) {
            c = 1;
            end = G.cascades.y;
        }
        let here = cascade_light(c, rel, n);
        vis = mix(beyond, here.x, here.y);
        if (c == 2) {
            // The last cascade fades out rather than ending in a line.
            vis = mix(vis, beyond, smoothstep(end * 0.7, end, depth));
        } else if (depth > end * 0.85) {
            // Each map is coarser than the last: one fades into the next over the end
            // of its reach, so the change is not a line across the ground.
            let next = cascade_light(c + 1, rel, n);
            vis = mix(vis, next.x, smoothstep(end * 0.85, end, depth) * next.y);
        }
    }
    return vis * cloud_light(rel);
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return pow(c, vec3<f32>(2.2));
}
