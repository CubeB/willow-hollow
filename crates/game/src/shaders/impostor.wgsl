// Distant trees: a few boxes each, positioned in whole world voxels so that they stay
// exact however far the camera has travelled.

struct Camera {
    // The camera's position: whole voxels, and the remainder.
    voxel: vec4<i32>,
    frac: vec4<f32>,
};

@group(1) @binding(0) var<uniform> C: Camera;

struct VIn {
    // 0..1 corner of the unit box on one side, and its outward normal; then the same
    // corner of the opposite side. Only one of the two can face the camera.
    @location(0) corner: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(6) opposite: vec3<f32>,
    @location(7) opposite_normal: vec3<f32>,
    // The colour of a bare broadleaf crown (impostors::BARE_CROWN).
    @location(8) bare: vec4<f32>,
    // Per box: world voxel position of the low corner, size in voxels, colour.
    @location(2) lo: vec3<i32>,
    @location(3) size: vec4<u32>,
    @location(4) color: vec4<f32>,
    // The tree's tag (season_tree) in x. yz: the skylines where it stands.
    @location(5) tree: vec4<u32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) rel: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) normal: vec3<f32>,
    // The ground where the tree stands: its biome's row of G.snow (the size's w is the biome).
    @location(3) snow: vec4<f32>,
    // How much sun reaches the tree past the hills and clouds.
    @location(4) sun: f32,
};

@vertex
fn vs(in: VIn) -> VOut {
    var out: VOut;
    let low = vec3<f32>(in.lo - C.voxel.xyz) - C.frac.xyz;
    let size = vec3<f32>(in.size.xyz);
    // The side whose outward normal points toward the camera, judged from the box's
    // middle: if either side of the pair faces the camera, this is that one; if neither
    // does, the one taken is culled as a back face, as both were.
    let toward = dot(low + size * 0.5, in.normal) < 0.0;
    let corner = select(in.opposite, in.corner, toward);
    let rel = low + corner * size;
    out.rel = rel;
    out.color = srgb_to_linear(in.color.rgb);
    out.normal = select(in.opposite_normal, in.normal, toward);
    out.snow = biome_snow(in.size.w);
    out.sun = skyline_light(vec2<f32>(in.tree.yz) / 255.0) * cloud_light(low + size * 0.5);
    // The season: leaves turn and fall with the near tree's (a whole box at once, so every
    // corner agrees). The box's plant kind rides in the colour's alpha, the tree's tag
    // (season_tree) in its own instance byte. A broadleaf's crown with its leaves down is
    // the grey-brown of bare twigs.
    let kind = u32(round(in.color.a * 255.0));
    let leaf = season_box_leaf(out.color, in.lo, kind, G.season.x, in.tree.x);
    if (leaf.a < 0.5) {
        out.color = srgb_to_linear(in.bare.rgb);
    } else {
        out.color = leaf.rgb;
    }
    // Nearer than this the real voxel trees are drawn instead.
    if (length((low + size * 0.5).xz) < G.fog.y) {
        out.clip = vec4<f32>(0.0, 0.0, -2.0, 1.0);
    } else {
        out.clip = G.view_proj * vec4<f32>(rel, 1.0);
    }
    return out;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let n = in.normal;
    let ndl = max(dot(n, G.sun_dir.xyz), 0.0);
    let light = G.sun_color.rgb * ndl * 0.85 * in.sun + ambient(n);
    let base = ground_weather(in.color, n, in.rel, 0.5, true, false, true, 0.0, in.snow);
    return vec4<f32>(apply_fog_lit(base * light, in.rel, lit_beside_level(light)), 1.0);
}

// --- What people have built (farbuilt.rs) ---------------------------------------------
// The same boxes, made from the stamps of landmarks, bridges and buildings. Their cells
// are those of the coarsest voxel ring, and lit as it lights them, so they are cut off at
// its edge pixel by pixel, where the ring draws the same cells; and they break up in the
// ordered pattern things leaving the world do as they go out of sight (C.frac.w).

// A built box's flags (farbuilt::flags), in the last byte of its `tree`.
const BUILT_GLOW: u32 = 1u;
const BUILT_FOLIAGE: u32 = 2u;
const BUILT_ROOFED: u32 = 4u;
// The edge of the cells built boxes are made of, in voxels (farbuilt::CELL).
const BUILT_CELL: f32 = 4.0;
// Wood, which snow only dusts (voxel::plant::VINE).
const BUILT_WOOD: u32 = 6u;
// How many pixels across a point of light is drawn at least, after dark.
const GLOW_PIXELS: f32 = 2.0;
// How far beyond the voxel rings it takes to grow to that, in voxels: 120 m.
const GLOW_GROWS: f32 = 300.0;
// The last stretch of sight over which built things break up (farbuilt::FADE).
const BUILT_FADE: f32 = 1000.0;

struct BOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) rel: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) snow: vec4<f32>,
    @location(4) sun: f32,
    // How brightly it glows (0 if lit by the sun), how far it has broken up going out of
    // sight, and its flags with its plant kind above them.
    @location(5) @interpolate(flat) look: vec3<f32>,
    // How much of the sky and of the sun reach it, down in a fold of the hills or up on a
    // ridge (hollow_light), from the byte where a tree's box carries its tag.
    @location(6) @interpolate(flat) fold: vec2<f32>,
};

@vertex
fn vs_built(in: VIn) -> BOut {
    var out: BOut;
    var low = vec3<f32>(in.lo - C.voxel.xyz) - C.frac.xyz;
    var size = vec3<f32>(in.size.xyz);
    let middle = low + size * 0.5;
    var glow = 0.0;
    if ((in.tree.w & BUILT_GLOW) != 0u) {
        // The cell of a flame: by day just that, faintly lit by itself as the voxel rings
        // light it, and after dark a point of light a couple of pixels across at least
        // however far off, so that it neither goes nor twinkles as the view moves,
        // standing on its cell. It grows to that over the first stretch beyond the rings,
        // so as not to jump from the cell's size where they give way to it.
        let dark = darkness();
        let grown = clamp((length(middle.xz) - G.fog.y) / GLOW_GROWS, 0.0, 1.0) * dark;
        let least = mix(size.x, GLOW_PIXELS * length(middle) * G.cam_up.w, grown);
        let wide = max(size.x, least);
        size = vec3<f32>(wide, max(size.y, least), wide);
        low = vec3<f32>(middle.x - wide * 0.5, low.y, middle.z - wide * 0.5);
        // As bright as a flame's voxel in the voxel rings (shade in terrain.wgsl).
        glow = mix(1.2, 14.0, dark);
    }
    let toward = dot(low + size * 0.5, in.normal) < 0.0;
    let corner = select(in.opposite, in.corner, toward);
    out.rel = low + corner * size;
    out.normal = select(in.opposite_normal, in.normal, toward);
    out.color = srgb_to_linear(in.color.rgb);
    out.snow = biome_snow(in.size.w);
    out.sun = skyline_light(vec2<f32>(in.tree.yz) / 255.0) * cloud_light(middle);
    let kind = u32(round(in.color.a * 255.0));
    let gone = clamp((length(middle.xz) - (C.frac.w - BUILT_FADE)) / BUILT_FADE, 0.0, 1.0);
    out.look = vec3<f32>(glow, gone, f32(in.tree.w | (kind << 4u)));
    out.fold = hollow_light(hollow_measure(f32(in.tree.x)));
    if (gone >= 1.0) {
        out.clip = vec4<f32>(0.0, 0.0, -2.0, 1.0);
    } else {
        out.clip = G.view_proj * vec4<f32>(out.rel, 1.0);
    }
    return out;
}

@fragment
fn fs_built(in: BOut) -> @location(0) vec4<f32> {
    if (!beyond_voxels(in.rel)) {
        discard;
    }
    // The ordered 4x4 pattern of particles.wgsl, worked out rather than looked up.
    if (in.look.y > 0.0) {
        let p = vec2<u32>(in.clip.xy) & vec2<u32>(3u);
        let a = p.x ^ p.y;
        let bayer = f32(((a & 1u) << 3u) | ((p.y & 1u) << 2u) | (a & 2u) | (p.y >> 1u));
        if ((bayer + 0.5) / 16.0 < in.look.y) {
            discard;
        }
    }
    let bits = u32(in.look.z);
    let foliage = (bits & BUILT_FOLIAGE) != 0u;
    let n = in.normal;
    // Lit as the coarsest ring lights the same cells where it gives way to these (shade
    // in terrain.wgsl), so that its edge passes across a structure unseen: once a cell is
    // about a pixel, its sides are lit more as the level ground they stand on, the sky
    // reaches it a little less, a fold of the hills shades it, and whatever has more of
    // the same building over it is in that one's shade.
    let far = 1.0 - smoothstep(1.2, 4.0, BUILT_CELL / max(length(in.rel) * G.cam_up.w, 1e-6));
    var lit_n = n;
    if (abs(n.y) < 0.5) {
        lit_n = normalize(mix(n, vec3<f32>(0.0, 1.0, 0.0), far * 0.65));
    }
    let facing = dot(lit_n, G.sun_dir.xyz);
    let ndl = select(max(facing, 0.0), max((facing + 0.25) / 1.25, 0.0), foliage);
    let open = select(1.0, 0.3, (bits & BUILT_ROOFED) != 0u);
    let fold = mix(vec2<f32>(1.0), in.fold, far);
    let light = G.sun_color.rgb * ndl * (in.sun * open * fold.y) + ambient(lit_n) * (mix(1.0, 0.85, far) * fold.x);
    let glows = in.look.x > 0.0;
    let base = ground_weather(in.color, n, in.rel, 0.5, foliage, glows, (bits >> 4u) == BUILT_WOOD, 0.0, in.snow);
    return vec4<f32>(apply_fog_lit(base * (light + vec3<f32>(in.look.x)), in.rel, mix(1.0, lit_beside_level(light), far)), 1.0);
}
