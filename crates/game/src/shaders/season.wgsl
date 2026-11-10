// The turn of the year, as draw-time colour and presence. Self-contained: the terrain and
// shadow shaders both include it. `s` is the date, 0..1 from the spring equinox (0.25
// midsummer, 0.5 autumn equinox, 0.75 midwinter); see season.rs.
//
// Vertices carry no species, so plants are told apart by colour: a deciduous leaf or
// tuft is green with little blue, an evergreen needle is bluer, a jungle leaf is
// redless, a maple is orange or red. Each tree turns and drops at its own date, from a
// hash of the 4 m cell it stands in, and each leaf cube a little differently again.

fn season_hash(p: vec3<i32>) -> f32 {
    var h = u32(p.x) * 374761393u + u32(p.y) * 668265263u + u32(p.z) * 2246822519u;
    h = (h ^ (h >> 13u)) * 1274126177u;
    h = h ^ (h >> 16u);
    return f32(h & 0xffffffu) / 16777216.0;
}

struct Leaf {
    // False once this leaf has fallen, or before it has budded.
    keep: bool,
    // 0..1 through the colours of autumn.
    turn: f32,
    // 1 for a new leaf, easing to 0 as it matures.
    bud: f32,
    // Per-tree random, for varying the autumn colour.
    tree: f32,
    // Whether this is a deciduous leaf or tuft at all.
    plant: bool,
    // Whether it is orange or red already, so only its fall is seasonal.
    maple: bool,
}

// `q` is the colour in gamma space. `vox` is the voxel. `h` is the leaf's own random.
fn season_leaf(s: f32, q: vec3<f32>, vox: vec3<i32>, h: f32) -> Leaf {
    var out: Leaf;
    out.keep = true;
    out.turn = 0.0;
    out.bud = 0.0;
    let green = q.g > q.r * 1.05 && q.g > q.b * 1.2;
    let evergreen = q.b > q.g * 0.5 || q.r < q.g * 0.4;
    let maple = q.r > q.g * 1.5 && q.b < q.g * 0.8 && q.r > 0.6;
    out.plant = (green && !evergreen) || maple;
    out.maple = maple;
    let cell = vec3<i32>(vec2<i32>(floor(vec2<f32>(f32(vox.x), f32(vox.z)) / 10.0)), 7).xzy;
    out.tree = season_hash(cell);
    if (!out.plant) {
        return out;
    }
    let bud_at = 0.02 + 0.10 * out.tree + 0.03 * h;
    let fall_at = 0.62 + 0.14 * out.tree + 0.07 * h;
    out.keep = s >= bud_at && s < fall_at;
    out.turn = smoothstep(0.48 + 0.08 * out.tree, 0.62 + 0.08 * out.tree, s);
    out.bud = 1.0 - smoothstep(bud_at, bud_at + 0.12, s);
    return out;
}

// Colour of a leaf or tuft that is still on: new and pale in spring, gold and red in autumn.
fn season_leaf_colour(c: vec3<f32>, leaf: Leaf, maple: bool) -> vec3<f32> {
    var o = c;
    if (!maple) {
        o = mix(o, o * vec3<f32>(1.45, 1.3, 0.6) + vec3<f32>(0.02, 0.03, 0.0), leaf.bud * 0.7);
        let gold = vec3<f32>(0.62, 0.36, 0.04);
        let red = vec3<f32>(0.5, 0.1, 0.03);
        let tint = mix(gold, red, smoothstep(0.3, 0.8, leaf.tree)) * (0.45 + c.g * 2.0);
        o = mix(o, tint, leaf.turn * 0.92);
    }
    return o;
}

// Ground vegetation (and any other green): bleached and dry by late summer, flattened and
// yellow-brown through winter.
fn season_grass(c: vec3<f32>, q: vec3<f32>, s: f32) -> vec3<f32> {
    if (!(q.g > q.r * 1.05 && q.g > q.b * 1.2)) {
        return c;
    }
    let dry = max(smoothstep(0.40, 0.56, s), 1.0 - smoothstep(0.0, 0.07, s));
    let winter = max(smoothstep(0.72, 0.82, s), 1.0 - smoothstep(0.0, 0.05, s));
    // Late summer grass dries to a deep straw: olive and russet, darker than it was green.
    let straw = vec3<f32>(0.30, 0.19, 0.05) * (0.55 + c.g * 1.7);
    var o = mix(c, straw, dry * 0.85);
    o = mix(o, o * vec3<f32>(0.8, 0.7, 0.62), winter);
    return o;
}

// The colour and presence (alpha 1 or 0) of a surface this date. `foliage` is the
// vertex flag for things that sway: leaves, vines and small plants.
fn season_apply(c: vec3<f32>, vox: vec3<i32>, h: f32, foliage: bool, s: f32) -> vec4<f32> {
    // Midsummer is the colours as modelled: nothing to do.
    if (s > 0.27 && s < 0.40) {
        return vec4<f32>(c, 1.0);
    }
    let q = sqrt(c);
    if (foliage) {
        let leaf = season_leaf(s, q, vox, h);
        if (leaf.plant) {
            return vec4<f32>(season_leaf_colour(c, leaf, leaf.maple), select(0.0, 1.0, leaf.keep));
        }
        return vec4<f32>(c, 1.0);
    }
    return vec4<f32>(season_grass(c, q, s), 1.0);
}
