// The map's flat parts, drawn over the finished frame: the veil behind the large map,
// the panel the corner map lies on, the marks on both, the player's arrow, the compass.
//
// Everything is one instanced quad per thing drawn. Colours are sRGB, as the target is
// written without encoding. The land itself is a model (relief.wgsl).

struct Uniforms {
    screen: vec2<f32>,
    // The map's middle and half its size, in pixels.
    centre: vec2<f32>,
    half: vec2<f32>,
    // Voxels of the world per pixel of the map.
    voxels_per_px: f32,
    // 1 for a round map, 0 for a rounded square.
    round: f32,
    time: f32,
    corner: f32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

const VEIL: u32 = 0u;
const GLOW: u32 = 2u;
const ARROW: u32 = 3u;
const CIRCLE: u32 = 4u;
const TRIANGLE: u32 = 5u;
const DIAMOND: u32 = 6u;
const RING: u32 = 7u;
const LINE: u32 = 9u;
const PANEL: u32 = 10u;

const RIM: vec3<f32> = vec3<f32>(0.95, 0.935, 0.89);
const INK: vec3<f32> = vec3<f32>(0.16, 0.15, 0.14);

struct Instance {
    @location(0) pos: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) dir: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) kind: u32,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    // Position within the quad, before turning: -1..1 each way.
    @location(0) q: vec2<f32>,
    @location(1) px: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) @interpolate(flat) kind: u32,
    @location(4) @interpolate(flat) size: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32, inst: Instance) -> VOut {
    var corners = array<vec2<f32>, 4>(vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(-1.0, 1.0), vec2(1.0, 1.0));
    let q = corners[vi];
    let a = q * inst.size;
    let d = inst.dir;
    let px = inst.pos + vec2(a.x * d.x - a.y * d.y, a.x * d.y + a.y * d.x);
    var out: VOut;
    out.clip = vec4(px.x / u.screen.x * 2.0 - 1.0, 1.0 - px.y / u.screen.y * 2.0, 0.0, 1.0);
    out.q = q;
    out.px = px;
    out.color = inst.color;
    out.kind = inst.kind;
    out.size = inst.size;
    return out;
}

// Distance in pixels from the map's edge, negative inside.
fn map_edge(px: vec2<f32>) -> f32 {
    let o = px - u.centre;
    if u.round > 0.5 {
        return length(o) - u.half.x;
    }
    let q = abs(o) - u.half + vec2(u.corner);
    return length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - u.corner;
}

fn sd_triangle(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, c: vec2<f32>) -> f32 {
    let e0 = b - a;
    let e1 = c - b;
    let e2 = a - c;
    let v0 = p - a;
    let v1 = p - b;
    let v2 = p - c;
    let pq0 = v0 - e0 * clamp(dot(v0, e0) / dot(e0, e0), 0.0, 1.0);
    let pq1 = v1 - e1 * clamp(dot(v1, e1) / dot(e1, e1), 0.0, 1.0);
    let pq2 = v2 - e2 * clamp(dot(v2, e2) / dot(e2, e2), 0.0, 1.0);
    let s = sign(e0.x * e2.y - e0.y * e2.x);
    let d = min(min(vec2(dot(pq0, pq0), s * (v0.x * e0.y - v0.y * e0.x)), vec2(dot(pq1, pq1), s * (v1.x * e1.y - v1.y * e1.x))), vec2(dot(pq2, pq2), s * (v2.x * e2.y - v2.y * e2.x)));
    return -sqrt(d.x) * sign(d.y);
}

fn over(under: vec4<f32>, top: vec4<f32>) -> vec4<f32> {
    let a = top.a + under.a * (1.0 - top.a);
    if a <= 0.0 {
        return vec4(0.0);
    }
    return vec4((top.rgb * top.a + under.rgb * under.a * (1.0 - top.a)) / a, a);
}

// A filled shape with a thin light outline, from its signed distance in pixels.
fn icon(d: f32, fill: vec4<f32>, outline: vec3<f32>) -> vec4<f32> {
    let inside = clamp(0.5 - d, 0.0, 1.0);
    let rim = clamp(0.5 - (d - 1.3), 0.0, 1.0);
    let c = mix(outline, fill.rgb, inside);
    return vec4(c, rim * fill.a);
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    if in.kind == VEIL {
        return in.color;
    }
    let edge = map_edge(in.px);
    if in.kind == PANEL {
        var col = vec4(0.0);
        if edge < 0.5 {
            col = in.color;
            col.a *= clamp(0.5 - edge, 0.0, 1.0);
        } else {
            // A soft shadow under the map.
            col = vec4(0.0, 0.0, 0.0, 0.22 * exp(-edge / 2.5));
        }
        // The frame: a thin pale line along the edge.
        let ring = clamp(1.0 - abs(edge + 0.4) / 0.9, 0.0, 1.0);
        return over(col, vec4(RIM, 0.9 * ring));
    }
    // Everything else stays on the map.
    let clip = clamp(0.5 - (edge + 1.5), 0.0, 1.0);
    if clip <= 0.0 {
        discard;
    }
    let p = in.q * in.size;
    var out = vec4(0.0);
    switch in.kind {
        case GLOW: {
            let r2 = dot(in.q, in.q);
            let fade = 1.0 - smoothstep(0.7, 1.0, sqrt(r2));
            let glow = exp(-r2 * 4.0) * fade;
            let core = exp(-r2 * 30.0);
            out = vec4(mix(in.color.rgb, vec3(1.0), core * 0.5), in.color.a * (glow * 0.8 + core * 0.4));
        }
        case ARROW: {
            let s = in.size.x;
            let body = sd_triangle(p, vec2(s, 0.0), vec2(-0.75 * s, 0.68 * s), vec2(-0.75 * s, -0.68 * s));
            let notch = sd_triangle(p, vec2(-0.3 * s, 0.0), vec2(-1.2 * s, 1.0 * s), vec2(-1.2 * s, -1.0 * s));
            let d = max(body, -notch) + 0.5;
            let inside = clamp(0.5 - d, 0.0, 1.0);
            let rim = clamp(0.5 - (d - 1.4), 0.0, 1.0);
            out = vec4(mix(INK, in.color.rgb, inside), rim * in.color.a);
        }
        case CIRCLE: {
            out = icon(length(p) - in.size.x * 0.62, in.color, RIM);
        }
        case TRIANGLE: {
            let s = in.size.x * 0.8;
            out = icon(sd_triangle(p, vec2(0.0, -s), vec2(s, 0.75 * s), vec2(-s, 0.75 * s)), in.color, RIM);
        }
        case DIAMOND: {
            out = icon((abs(p.x) + abs(p.y)) * 0.7071 - in.size.x * 0.5, in.color, RIM);
        }
        case RING: {
            let r = length(p);
            out = icon(max(r - in.size.x * 0.68, in.size.x * 0.28 - r), in.color, RIM);
        }
        case LINE: {
            let b = abs(p) - in.size + vec2(1.0);
            let d = length(max(b, vec2(0.0))) + min(max(b.x, b.y), 0.0);
            out = vec4(in.color.rgb, in.color.a * clamp(0.5 - d, 0.0, 1.0));
        }
        default: {
            discard;
        }
    }
    out.a *= clip;
    return out;
}
