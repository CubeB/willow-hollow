// The map's model: blocks of land, water, trees and buildings under one fixed sun, in
// the afternoon and to the west-south-west. The shadows are worked out when the model is
// built (minimap/light.rs) and come with each corner; here they are only put together
// with the way each face looks. Colours are sRGB, as the target is written without
// encoding.

struct Uniforms {
    // From the model's columns and blocks to the screen.
    transform: mat4x4<f32>,
    // What the hour makes of the colours, and in w how far into the night it is.
    grade: vec4<f32>,
    // Seconds, for the glints on the water.
    time: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

// The way to the sun: east, up, south. minimap/light.rs has the same.
const SUN = vec3<f32>(-0.66, 0.68, 0.32);
// What a corner's sun is stored as for full sun on level ground (relief::SUN_SCALE).
const SUN_SCALE = 170.0;
// The light of the sky, cool, on tops and on sides, and the sun's, warm.
const SKY = vec3<f32>(0.95, 0.99, 1.07);
const SUNLIGHT = vec3<f32>(1.06, 1.0, 0.91);
const SKY_TOP = 0.64;
const SKY_SIDE = 0.56;
const SUN_FULL = 0.43;

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec3<f32>,
    // The sun on it and the sky's light on it.
    @location(1) light: vec2<f32>,
    // Columns east and south of the model's corner.
    @location(2) at: vec2<f32>,
    // 1 for the top of water, 2 for what gives off light after dark.
    @location(3) @interpolate(flat) kind: u32,
};

@vertex
fn vs(@location(0) pos: vec3<f32>, @location(1) color: vec4<f32>, @location(2) light: vec4<f32>) -> VOut {
    // Up, east, west, south, north.
    let face = u32(color.a * 255.0 + 0.5);
    var toward = SUN.y;
    var sky = SKY_SIDE;
    if face == 0u {
        sky = SKY_TOP;
    } else if face == 1u {
        toward = SUN.x;
    } else if face == 2u {
        toward = -SUN.x;
    } else if face == 3u {
        toward = SUN.z;
    } else {
        toward = -SUN.z;
    }
    var out: VOut;
    out.clip = u.transform * vec4(pos, 1.0);
    out.color = color.rgb;
    out.light = vec2(light.r * 255.0 / SUN_SCALE * max(toward, 0.0) / SUN.y, sky * light.g);
    out.at = pos.xz;
    let kind = u32(light.b * 255.0 + 0.5);
    out.kind = select(kind, 0u, kind == 1u && face != 0u);
    return out;
}

fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2(12.9898, 78.233))) * 43758.5453);
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let night = u.grade.w;
    var sun = in.light.x;
    if in.kind == 1u {
        // Water lies under a shadow less darkly than ground does.
        sun = mix(1.0, sun, 0.55);
    }
    var c = in.color * (SKY * in.light.y + SUNLIGHT * SUN_FULL * sun);
    if in.kind == 1u {
        // Glints, a block of light here and there, coming and going.
        let cell = floor(in.at * 2.0);
        let beat = floor(u.time.x * 1.6 + hash(cell) * 8.0);
        let glint = step(0.984, hash(cell + vec2(beat * 7.0, beat * 3.0)));
        c += vec3(0.20, 0.21, 0.20) * glint * sun * (1.0 - night);
    }
    c *= u.grade.rgb;
    if in.kind == 2u {
        // A lit window or a lantern keeps its colour, and shows by night.
        c = mix(c, min(in.color * 1.35 + vec3(0.12, 0.08, 0.0), vec3(1.0)), 0.35 + 0.65 * night);
    }
    return vec4(c, 1.0);
}
