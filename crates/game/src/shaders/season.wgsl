// The turn of the year, as draw-time colour and presence. Self-contained: the terrain and
// shadow shaders both include it. `s` is the date, 0..1 from the spring equinox (0.25
// midsummer, 0.5 autumn equinox, 0.75 midwinter); see season.rs.
//
// Every plant vertex carries a plant kind (voxel::plant) in its flags, so a maple, a tuft
// of grass and a pine each follow their own year. Holly, yew, ferns and the other winter
// evergreens are PLANT_EVERGREEN with the pines: never recoloured, dropped or buried. Deciduous and maple leaves are meshed
// one voxel to a quad, and each corner says which voxel it belongs to, so a leaf that has
// fallen is dropped in the vertex shader (all four corners agree) and costs no fragment
// work. Each tree turns and drops at its own date, from its tree tag (season_tree), each
// branch a little differently, and each leaf cube a little differently again.

const PLANT_DECIDUOUS: u32 = 1u;
const PLANT_EVERGREEN: u32 = 2u;
const PLANT_AUTUMN: u32 = 3u;
const PLANT_GRASS: u32 = 4u;
const PLANT_FLOWER: u32 = 5u;
const PLANT_GROUND: u32 = 7u;

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
}

// A tree's own random, 0..1. `tag` is the tree's number, 1..63, worked out from its trunk
// when it was meshed (mesh.rs carries it in the low bits of a leaf's colour, see
// `season_tag`; impostors in a spare byte). 0 where it is not known, as in the far view,
// which falls back on the 4 m cell the voxel is in.
fn season_tree(vox: vec3<i32>, tag: u32) -> f32 {
    if (tag == 0u) {
        return season_hash(vec3<i32>(vec2<i32>(floor(vec2<f32>(f32(vox.x), f32(vox.z)) / 10.0)), 7).xzy);
    }
    return f32(tag - 1u) / 62.0;
}

// The tree tag a terrain vertex carries in the two low bits of each colour channel of `b`.
fn season_tag(b: u32) -> u32 {
    return (b & 3u) | ((b >> 6u) & 12u) | ((b >> 12u) & 48u);
}

// The year of a broadleaf: it turns from late summer, and every leaf is down by early
// winter (0.70, before midwinter at 0.75). Each tree keeps its leaves until its own date,
// 0.625 to 0.675, so in late autumn one tree is bare while its neighbour still holds half
// its leaves; a branch (a 1.2 m clump) goes up to 0.008 either side of that and each leaf
// up to 0.015, so the last leaf of the latest tree falls at 0.698. A maple turns and drops
// 0.025 ahead of the rest, so the last red is gone first. Keep in step with
// season::leaf_fall and season::LEAVES_DOWN.
fn season_leaf(s: f32, vox: vec3<i32>, maple: bool, tag: u32) -> Leaf {
    var out: Leaf;
    let h = season_hash(vox);
    let branch = season_hash(vec3<i32>(floor(vec3<f32>(vox) / 3.0)) + vec3<i32>(11, 3, 7));
    out.tree = season_tree(vox, tag);
    let early = select(0.0, 0.025, maple);
    let bud_at = 0.02 + 0.10 * out.tree + 0.03 * h;
    let fall_at = 0.625 + 0.05 * out.tree + 0.016 * (branch - 0.5) + 0.03 * (h - 0.5) - early;
    out.keep = s >= bud_at && s < fall_at;
    // A tree that drops late turns late too.
    let turn_at = 0.47 + 0.06 * out.tree + 0.02 * (branch - 0.5) - early;
    out.turn = smoothstep(turn_at, turn_at + 0.10, s);
    out.bud = 1.0 - smoothstep(bud_at, bud_at + 0.12, s);
    return out;
}

// How far off the small plants standing on the column `col` (world voxels) are still
// drawn, as a share of PLANT_REACH. Each column has its own distance, so the plants thin
// out over the last third of their reach, one here and one there, where all giving out
// at one distance made an edge that moved over the ground with the camera. The ground
// under each takes its colour as it goes (see `wash` in terrain.wgsl).
fn plant_reach(col: vec2<i32>) -> f32 {
    return 0.66 + 0.32 * season_hash(vec3<i32>(col.x, 77, col.y));
}

// Whether a plant of this kind has gone from the voxel `vox` this date: leaves that have
// fallen, flowers that have died back. Used by the vertex shaders, so all the corners of a
// face must name the same voxel. `tag` is its tree's (season_tree).
fn season_gone(s: f32, kind: u32, vox: vec3<i32>, tag: u32) -> bool {
    if (kind == PLANT_DECIDUOUS || kind == PLANT_AUTUMN) {
        // Midsummer: every leaf is on.
        if (s > 0.27 && s < 0.40) {
            return false;
        }
        return !season_leaf(s, vox, kind == PLANT_AUTUMN, tag).keep;
    }
    if (kind == PLANT_FLOWER) {
        let h = season_hash(vox);
        return !(s >= 0.04 + 0.08 * h && s < 0.66 + 0.06 * h);
    }
    return false;
}

// Colour of a leaf still on: new and pale in spring, gold and red in autumn. A maple is
// modelled in its autumn colours, so it is green until its own autumn.
fn season_leaf_colour(c: vec3<f32>, leaf: Leaf, maple: bool) -> vec3<f32> {
    var o = c;
    if (maple) {
        o = vec3<f32>(0.30, 0.95, 0.20) * dot(c, vec3<f32>(0.3, 0.59, 0.11)) * 1.1;
    }
    o = mix(o, o * vec3<f32>(1.45, 1.3, 0.6) + vec3<f32>(0.02, 0.03, 0.0), leaf.bud * 0.7);
    if (maple) {
        return mix(o, c, leaf.turn);
    }
    let gold = vec3<f32>(0.62, 0.36, 0.04);
    let red = vec3<f32>(0.5, 0.1, 0.03);
    let tint = mix(gold, red, smoothstep(0.3, 0.8, leaf.tree)) * (0.45 + c.g * 2.0);
    return mix(o, tint, leaf.turn * 0.92);
}

// Grass: yellows a little in the dry end of summer, greens up again with the autumn rain
// and stays green through winter, only a shade duller in the cold.
fn season_grass(c: vec3<f32>, s: f32) -> vec3<f32> {
    let dry = smoothstep(0.40, 0.54, s) * (1.0 - smoothstep(0.60, 0.72, s));
    let winter = max(smoothstep(0.72, 0.82, s), 1.0 - smoothstep(0.0, 0.07, s));
    let straw = vec3<f32>(0.30, 0.19, 0.05) * (0.55 + c.g * 1.7);
    var o = mix(c, straw, dry * 0.5);
    let grey = vec3<f32>(dot(o, vec3<f32>(0.3, 0.6, 0.1)));
    o = mix(o, mix(grey, o, 0.72) * vec3<f32>(0.9, 0.86, 0.78), winter);
    return o;
}

// The colour of a surface of this plant kind this date. `vox` is its voxel, `tag` its
// tree's (season_tree).
fn season_apply(c: vec3<f32>, vox: vec3<i32>, kind: u32, s: f32, tag: u32) -> vec3<f32> {
    // Midsummer is the colours as modelled (a maple, modelled in autumn, aside).
    if (kind != PLANT_AUTUMN && s > 0.27 && s < 0.40) {
        return c;
    }
    if (kind == PLANT_DECIDUOUS || kind == PLANT_AUTUMN) {
        return season_leaf_colour(c, season_leaf(s, vox, kind == PLANT_AUTUMN, tag), kind == PLANT_AUTUMN);
    }
    if (kind == PLANT_GRASS || kind == PLANT_GROUND) {
        return season_grass(c, s);
    }
    return c;
}

// A far tree's box (impostors), which carries the plant kind of the voxels it stands for:
// its colour this date, and alpha 0 once its leaves have fallen, on the near leaves' dates.
// `tag` is the tree's, as the near leaves have it.
fn season_box_leaf(c: vec3<f32>, lo: vec3<i32>, kind: u32, s: f32, tag: u32) -> vec4<f32> {
    if (kind != PLANT_DECIDUOUS && kind != PLANT_AUTUMN) {
        return vec4<f32>(c, 1.0);
    }
    let maple = kind == PLANT_AUTUMN;
    if (!maple && s > 0.27 && s < 0.40) {
        return vec4<f32>(c, 1.0);
    }
    let leaf = season_leaf(s, lo, maple, tag);
    return vec4<f32>(season_leaf_colour(c, leaf, maple), select(0.0, 1.0, leaf.keep));
}

// --- Distant things, which carry no plant kind -------------------------------------------
// The far terrain and the far tree models have only a colour, so here a plant is told by it:
// a deciduous leaf is green with little blue and some red, a maple orange or red, an
// evergreen bluer or redless. `world` is a position in voxels; trees turn and drop in
// patches of about 40 m, so a far forest goes patchy before it goes gold.

fn season_green(q: vec3<f32>) -> bool {
    return q.g > q.r * 1.05 && q.g > q.b * 1.2;
}

fn season_evergreen(q: vec3<f32>) -> bool {
    return q.b > q.g * 0.5 || q.r < q.g * 0.4;
}

fn season_maple(q: vec3<f32>) -> bool {
    return q.r > q.g * 1.5 && q.b < q.g * 0.8 && q.r > 0.6;
}

// Cherry blossom pink: a broadleaf too.
fn season_blossom(q: vec3<f32>) -> bool {
    return q.r > q.g * 1.2 && q.b > q.g * 1.05 && q.r > 0.6;
}

// A far canopy: its colour this date, and alpha 0 where its leaves have fallen.
fn season_far_leaf(c: vec3<f32>, world: vec2<f32>, s: f32) -> vec4<f32> {
    let q = sqrt(c);
    let maple = season_maple(q);
    if (!((season_green(q) && !season_evergreen(q)) || maple || season_blossom(q))) {
        return vec4<f32>(c, 1.0);
    }
    if (s > 0.27 && s < 0.40 && !maple) {
        return vec4<f32>(c, 1.0);
    }
    let cell = vec3<i32>(i32(floor(world.x / 40.0)) * 10, 0, i32(floor(world.y / 40.0)) * 10);
    let leaf = season_leaf(s, cell, maple, 0u);
    return vec4<f32>(season_leaf_colour(c, leaf, maple), select(0.0, 1.0, leaf.keep));
}

// The far ground: grass dries and yellows, and a far forest of broadleaves turns, then goes
// to bare brown branches, but for the share `keeps` of it that is evergreen (the hollies,
// yews and pines among its trees), which shows dark green between them.
fn season_far_ground(c: vec3<f32>, forest: bool, keeps: f32, world: vec2<f32>, s: f32) -> vec3<f32> {
    let q = sqrt(c);
    if (forest) {
        let leaf = season_far_leaf(c, world, s);
        // Seen from above an evergreen hides more of the wood than its share of the trunks.
        let bare = mix(vec3<f32>(0.09, 0.07, 0.06), vec3<f32>(0.022, 0.075, 0.038), min(keeps * 1.5, 1.0));
        return mix(bare, leaf.rgb, leaf.a);
    }
    if (season_green(q)) {
        return season_grass(c, s);
    }
    return c;
}
