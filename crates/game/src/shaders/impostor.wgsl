// Distant trees: a few boxes each, positioned in whole world voxels so that they stay
// exact however far the camera has travelled.

struct Camera {
    // The camera's position: whole voxels, and the remainder.
    voxel: vec4<i32>,
    frac: vec4<f32>,
};

@group(1) @binding(0) var<uniform> C: Camera;

struct VIn {
    // 0..1 corner of the unit box, and its outward normal.
    @location(0) corner: vec3<f32>,
    @location(1) normal: vec3<f32>,
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
    let rel = low + in.corner * size;
    out.rel = rel;
    out.color = srgb_to_linear(in.color.rgb);
    out.normal = in.normal;
    out.snow = biome_snow(in.size.w);
    out.sun = skyline_light(vec2<f32>(in.tree.yz) / 255.0) * cloud_light(low + size * 0.5);
    // The season: leaves turn and fall with the near tree's (a whole box at once, so every
    // corner agrees). The box's plant kind rides in the colour's alpha.
    // Kind 8 and up is a broadleaf's bare crown, drawn just when its leaves are down. The
    // tree's tag (season_tree) rides in its own instance byte.
    let kind = u32(round(in.color.a * 255.0));
    let bare = kind >= 8u;
    let leaf = season_box_leaf(out.color, in.lo, kind & 7u, G.season.x, in.tree.x);
    if (!bare) {
        out.color = leaf.rgb;
    }
    let fallen = (leaf.a < 0.5) != bare;
    // Nearer than this the real voxel trees are drawn instead.
    if (fallen || length((low + size * 0.5).xz) < G.fog.y) {
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
    return vec4<f32>(apply_fog(base * light, in.rel), 1.0);
}
