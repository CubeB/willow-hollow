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
