//! What a settlement puts out on its days (#207): on market day its stalls' awnings and
//! the goods on their counters, at Midwinter lanterns on the village tree and along the
//! lanes, lit at night, at Midsummer garlands on its doors, its well and its stalls; and
//! all the year the ribbons tied on the tree at the last festival, bright when new and
//! fading until the next.
//!
//! All of it is drawn here as props rather than stamped into the world, so the world is
//! the same whatever the day: the stalls stand bare in it (the frame and the counter, see
//! `fittings::bare_stall`), and what is dressed on them comes and goes with the day
//! without anything being built again. Props do not flutter as cloth does, which suits
//! an awning stretched tight and a garland; the ribbons sway a little by themselves.
//! Everything keeps to [`BUDGET`] boxes, the nearest first.

use std::sync::OnceLock;

use glam::{DVec3, Vec2, Vec3};
use voxel::{Materials, Voxel};
use worldgen::{
    features::fittings::Fitting,
    materials as mat,
    noise::{hash2, unit},
    scale::{m, m64},
    settlements::{
        Region,
        calendar::{self, Festival},
    },
};

use super::town::{Dress, Town};
use crate::{
    particles::{self, ParticleInstance},
    people::Trade,
};

/// The most boxes a frame's dressing takes, beside the settlements' people's own
/// budget: render.rs makes room for this many.
pub const BUDGET: usize = 1792;

/// How far off each kind of thing is drawn: the stalls and garlands are small, and a
/// lantern lit at night is seen from further than one by day.
const STALL_SIGHT: f32 = 150.0;
const GARLAND_SIGHT: f32 = 90.0;
const LANTERN_SIGHT: f32 = 220.0;
const RIBBON_SIGHT: f32 = 120.0;
/// How far a lit lantern lights what is about it, for the frame's lamps.
const LANTERN_LIGHTS: f32 = 30.0;

/// When it is: the day (as `villagers::day_number`), the time of year, the hour, how
/// brightly a lantern burns (0 by day, about 1 at night, flickering) and a clock for
/// what sways.
#[derive(Clone, Copy, Debug)]
pub struct When {
    pub day: i64,
    pub date: f32,
    pub year_days: f32,
    pub hour: f32,
    pub lit: f32,
    pub clock: f32,
}

impl When {
    /// The festival on today, if any.
    pub fn festival(&self) -> Option<Festival> {
        calendar::festival_on(self.day, self.year_days)
    }
}

/// Whether the stalls of `town` are dressed at `w`: on its market day, from a little
/// before the market opens until a little after it shuts.
pub fn market_open(town: &Town, w: &When) -> bool {
    town.market_on(w.day) && w.hour > calendar::MARKET_OPENS - 0.3 && w.hour < calendar::MARKET_CLOSES + 0.4
}

/// One thing to dress, for putting them nearest first.
#[derive(Clone, Copy)]
enum Piece {
    Stall(usize),
    TreeLantern(usize),
    Post(usize),
    Ribbon(usize),
    Door(usize),
    Well(usize),
}

/// Everything about `town` dressed at `w` within sight of `cam`, each with how far it is.
fn pieces(town: &Town, w: &When, cam: DVec3) -> Vec<(f32, Piece, f32)> {
    let eye = (cam - town.origin).as_vec3();
    let mut out = Vec::new();
    let mut add = |at: Vec3, sight: f32, p: Piece| {
        let d = at.distance(eye);
        if d < m(sight) {
            out.push((d, p, m(sight)));
        }
    };
    let festival = w.festival();
    let open = market_open(town, w);
    for (s, d) in town.market.iter().enumerate() {
        let dressed = open && d.keeper.is_some() && d.awning.is_some();
        if dressed || festival == Some(Festival::Midsummer) {
            add(d.counter.0, STALL_SIGHT, Piece::Stall(s));
        }
    }
    for (k, &at) in town.knots.iter().enumerate() {
        add(at, RIBBON_SIGHT, Piece::Ribbon(k));
    }
    match festival {
        Some(Festival::Midwinter) => {
            for (k, &at) in town.spare.iter().enumerate() {
                add(at, LANTERN_SIGHT, Piece::TreeLantern(k));
            }
            for (k, p) in town.posts.iter().enumerate() {
                add(p.0, LANTERN_SIGHT, Piece::Post(k));
            }
        }
        Some(Festival::Midsummer) => {
            for (k, d) in town.doors.iter().enumerate() {
                add(d.0, GARLAND_SIGHT, Piece::Door(k));
            }
            for (k, wh) in town.well_heads.iter().enumerate() {
                if wh.0 != Fitting::Shaduf {
                    add(wh.1, GARLAND_SIGHT, Piece::Well(k));
                }
            }
        }
        None => {}
    }
    out
}

/// Draws what every one of `towns` has out at `w`, seen from `cam`, the nearest first and
/// never more than [`BUDGET`] boxes in all. Returns how many boxes it drew.
pub fn draw<'a>(towns: impl Iterator<Item = (&'a Town, When)>, cam: DVec3, out: &mut Vec<ParticleInstance>) -> usize {
    let from = out.len();
    let mut all: Vec<(f32, &Town, When, Piece, f32)> = Vec::new();
    for (town, w) in towns {
        all.extend(pieces(town, &w, cam).into_iter().map(|(d, p, sight)| (d, town, w, p, sight)));
    }
    all.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut scratch = Vec::with_capacity(64);
    for (d, town, w, piece, sight) in all {
        scratch.clear();
        let mut b = Boxes { rel: (town.origin - cam).as_vec3(), out: &mut scratch };
        piece_of(town, &w, piece, &mut b);
        if out.len() - from + scratch.len() > BUDGET {
            continue;
        }
        let start = out.len();
        out.extend_from_slice(&scratch);
        particles::dither_since(out, start, particles::dither_edge(d as f64, sight as f64, m64(20.0)));
    }
    out.len() - from
}

/// The lit lanterns near `cam` among `towns`, as the frame's lamps: where each burns, and
/// how brightly.
pub fn lamps<'a>(towns: impl Iterator<Item = (&'a Town, When)>, cam: DVec3) -> Vec<(DVec3, f32)> {
    let mut lamps = Vec::new();
    for (town, w) in towns {
        if w.lit <= 0.0 || w.festival() != Some(Festival::Midwinter) {
            continue;
        }
        let eye = (cam - town.origin).as_vec3();
        let mut near = |at: Vec3| {
            if at.distance(eye) < m(LANTERN_LIGHTS) {
                lamps.push((town.origin + at.as_dvec3(), 0.7 * w.lit));
            }
        };
        for (k, &at) in town.spare.iter().enumerate() {
            near(at - Vec3::Y * (tree_drop(town, k) + 0.35));
        }
        for p in &town.posts {
            let (_, hang) = post_arm(p.0, p.1);
            near(hang - Vec3::Y * 0.75);
        }
    }
    lamps
}

fn piece_of(town: &Town, w: &When, piece: Piece, b: &mut Boxes) {
    let summer = w.festival() == Some(Festival::Midsummer);
    match piece {
        Piece::Stall(s) => {
            let d = &town.market[s];
            let dressed = market_open(town, w) && d.keeper.is_some();
            stall(town, d, dressed, summer, b);
        }
        Piece::TreeLantern(k) => {
            let top = town.spare[k];
            let drop = tree_drop(town, k);
            b.put(top - Vec3::Y * drop * 0.5, Vec3::new(0.05, drop, 0.05), 0.0, [0.0; 2], CORD, 0.0);
            lantern(top - Vec3::Y * drop, hash2(town.seed ^ 0x1a77, k as i32, 1), w.lit, b);
        }
        Piece::Post(k) => {
            let (foot, arm) = town.posts[k];
            let (top, hang) = post_arm(foot, arm);
            let yaw = yaw_along(arm);
            b.put(foot + Vec3::Y * (top.y - foot.y) * 0.5, Vec3::new(0.24, top.y - foot.y, 0.24), yaw, [0.0; 2], POST, 0.0);
            b.put((top + hang) * 0.5 + Vec3::Y * 0.1, Vec3::new(0.18, 0.18, top.distance(hang) + 0.2), yaw, [0.0; 2], POST, 0.0);
            // A brace under the arm, from the post to half way along it.
            let brace = top.lerp(hang, 0.25) - Vec3::Y * 0.35;
            b.put(brace, Vec3::new(0.12, 0.12, 0.85), yaw, [-std::f32::consts::FRAC_PI_4, 0.0], POST, 0.0);
            b.put(hang - Vec3::Y * 0.2, Vec3::new(0.05, 0.4, 0.05), 0.0, [0.0; 2], CORD, 0.0);
            lantern(hang - Vec3::Y * 0.4, hash2(town.seed ^ 0x1a78, k as i32, 2), w.lit, b);
        }
        Piece::Ribbon(k) => ribbon(town, w, k, b),
        Piece::Door(k) => {
            let (face, out) = town.doors[k];
            let across = Vec2::new(-out.y, out.x);
            let mid = face + Vec3::new(out.x, 0.0, out.y) * 0.12 + Vec3::Y * 5.2;
            let half = Vec3::new(across.x, 0.0, across.y) * 1.3;
            swag(mid - half, mid + half, 0.4, hash2(town.seed ^ 0x9a71, k as i32, 3), true, b);
        }
        Piece::Well(k) => {
            let (f, mid, _, half) = town.well_heads[k];
            // On the low ring of an oasis, or the coping round a well's shaft.
            let (rim, r) = match f {
                Fitting::OasisWell => (0.8, 2.4),
                _ => (2.3, 1.65_f32.min(half.min_element())),
            };
            wreath(mid + Vec3::Y * rim, r, hash2(town.seed ^ 0x9a72, k as i32, 4), b);
        }
    }
}

/// Boxes as they are put into the frame, from places continent-local.
struct Boxes<'a> {
    /// The continent's origin from the camera.
    rel: Vec3,
    out: &'a mut Vec<ParticleInstance>,
}

impl Boxes<'_> {
    fn put(&mut self, at: Vec3, size: Vec3, yaw: f32, tilt: [f32; 2], rgb: [u8; 3], glow: f32) {
        self.out.push(ParticleInstance { pos: (self.rel + at).into(), yaw, size: size.into(), glow, color: linear(rgb), tilt, dissolve: 0.0 });
    }
}

fn linear(c: [u8; 3]) -> [f32; 4] {
    let l = |v: u8| (v as f32 / 255.0).powf(2.2);
    [l(c[0]), l(c[1]), l(c[2]), 1.0]
}

/// A material's colour, as the world has it.
fn rgb(v: Voxel) -> [u8; 3] {
    static M: OnceLock<Materials> = OnceLock::new();
    M.get_or_init(mat::build).get(v).color
}

fn blend(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    [0, 1, 2].map(|k| (a[k] as f32 + (b[k] as f32 - a[k] as f32) * t.clamp(0.0, 1.0)).round() as u8)
}

/// The yaw that turns a box's own +z to `dir` across the ground, and its +x to
/// `(dir.y, -dir.x)` (see particles.wgsl's `turn`).
fn yaw_along(dir: Vec2) -> f32 {
    (-dir.x).atan2(dir.y)
}

const CORD: [u8; 3] = [92, 78, 60];
const POST: [u8; 3] = [86, 62, 42];
const IRON: [u8; 3] = [52, 50, 52];
const LEAVES: [u8; 3] = [58, 104, 46];
const LEAVES_DARK: [u8; 3] = [40, 80, 36];
const BLOOMS: [[u8; 3]; 5] = [[236, 206, 66], [244, 240, 228], [206, 64, 70], [232, 140, 186], [150, 112, 200]];

/// How far a lantern hangs below the tree's limb, on its cord.
fn tree_drop(town: &Town, k: usize) -> f32 {
    0.5 + 0.9 * unit(hash2(town.seed ^ 0x1a79, k as i32, 0))
}

/// A lantern post's top and the end of its arm, for a post at `foot` reaching `arm`.
fn post_arm(foot: Vec3, arm: Vec2) -> (Vec3, Vec3) {
    let top = foot + Vec3::Y * 6.2;
    (top, top + Vec3::new(arm.x, 0.0, arm.y) * 1.4)
}

/// A paper lantern hung from `top`: a cap, a round body of paper in one of a few
/// colours, glowing when `lit`, and a foot.
fn lantern(top: Vec3, h: u32, lit: f32, b: &mut Boxes) {
    const PAPER: [[u8; 3]; 3] = [[214, 72, 52], [236, 196, 112], [240, 226, 196]];
    let paper = PAPER[(h % 3) as usize];
    let glow = if lit > 0.0 { 2.6 * lit } else { 0.0 };
    let body = if lit > 0.0 { blend(paper, [255, 214, 150], 0.45) } else { paper };
    let yaw = unit(h) * std::f32::consts::FRAC_PI_2;
    b.put(top - Vec3::Y * 0.05, Vec3::new(0.32, 0.1, 0.32), yaw, [0.0; 2], IRON, 0.0);
    b.put(top - Vec3::Y * 0.42, Vec3::new(0.6, 0.48, 0.6), yaw, [0.0; 2], body, glow);
    b.put(top - Vec3::Y * 0.42, Vec3::new(0.44, 0.66, 0.44), yaw, [0.0; 2], body, glow);
    b.put(top - Vec3::Y * 0.8, Vec3::new(0.26, 0.08, 0.26), yaw, [0.0; 2], IRON, 0.0);
}

/// The fresh ribbon tied beside the old at knot `k` of the village tree: in bright new
/// colours at a festival, fading through the half year until the next.
fn ribbon(town: &Town, w: &When, k: usize, b: &mut Boxes) {
    const BRIGHT: [Voxel; 5] = [mat::CLOTH_RED, mat::CLOTH_BLUE, mat::CLOTH_YELLOW, mat::CLOTH_GREEN, mat::CLOTH_PALE];
    let h = hash2(town.seed ^ 0x7e1b, k as i32, 5);
    let colour = faded(rgb(BRIGHT[(h % BRIGHT.len() as u32) as usize]), w.date);
    let knot = town.knots[k];
    let a = unit(h) * std::f32::consts::TAU;
    let top = knot + Vec3::new(a.cos(), 0.0, a.sin()) * 0.22;
    let len = 1.2 + 1.0 * unit(hash2(h, 1, 7));
    // It sways a little on its own, from its knot.
    let t = w.clock * (0.9 + 0.4 * unit(h >> 3)) + unit(h >> 7) * 6.0;
    let (p, r) = (0.10 * t.sin(), 0.07 * (t * 1.3 + 1.0).sin());
    let yaw = a;
    let half = -len * 0.5;
    // The middle of the ribbon, turned about its top as the shader turns a box about its
    // middle: roll about z, pitch about x, then yaw.
    let v = Vec3::new(-half * r.sin(), half * r.cos(), 0.0);
    let v = Vec3::new(v.x, v.y * p.cos(), v.y * p.sin());
    let (s, c) = yaw.sin_cos();
    let v = Vec3::new(v.x * c - v.z * s, v.y, v.x * s + v.z * c);
    b.put(top + v, Vec3::new(0.2, len, 0.035), yaw, [p, r], colour, 0.0);
}

/// A ribbon of colour `new` tied at the last festival before `date`, as it is by then:
/// a little brighter than the cloth when new, and taken by sun and rain toward the pale
/// of old cloth through the half year to the next.
fn faded(new: [u8; 3], date: f32) -> [u8; 3] {
    let since = Festival::ALL.iter().map(|f| f.since(date)).fold(1.0_f32, f32::min);
    blend(blend(new, [255, 255, 255], 0.08), [196, 186, 166], since * 1.5)
}

/// A garland of leaves hung from `a` to `b`, sagging `sag` in the middle, with flowers
/// along it and its ends hanging down if `tails`.
fn swag(a: Vec3, b_: Vec3, sag: f32, h: u32, tails: bool, b: &mut Boxes) {
    const N: usize = 5;
    let point = |t: f32| a.lerp(b_, t) - Vec3::Y * sag * 4.0 * t * (1.0 - t);
    let along = Vec2::new(b_.x - a.x, b_.z - a.z);
    // The box's own x runs along the garland.
    let yaw = along.y.atan2(along.x);
    for i in 0..N {
        let (p, q) = (point(i as f32 / N as f32), point((i + 1) as f32 / N as f32));
        let run = Vec2::new(q.x - p.x, q.z - p.z).length();
        let roll = (q.y - p.y).atan2(run);
        let leaves = if i % 2 == 0 { LEAVES } else { LEAVES_DARK };
        b.put((p + q) * 0.5, Vec3::new(p.distance(q) + 0.08, 0.26, 0.26), yaw, [0.0, roll], leaves, 0.0);
        if i % 2 == 1 || N == 1 {
            let bloom = BLOOMS[((h >> (i * 3)) % BLOOMS.len() as u32) as usize];
            b.put((p + q) * 0.5 + Vec3::Y * 0.12, Vec3::new(0.24, 0.2, 0.3), yaw, [0.0, roll], bloom, 0.0);
        }
    }
    if tails {
        for end in [a, b_] {
            b.put(end - Vec3::Y * 0.45, Vec3::new(0.2, 0.9, 0.2), yaw, [0.0; 2], LEAVES, 0.0);
        }
    }
}

/// A ring of leaves and flowers laid round a well's rim at `mid`, radius `r`.
fn wreath(mid: Vec3, r: f32, h: u32, b: &mut Boxes) {
    const N: usize = 10;
    for i in 0..N {
        let a = i as f32 / N as f32 * std::f32::consts::TAU;
        let at = mid + Vec3::new(a.cos(), 0.0, a.sin()) * r;
        let size = Vec3::new(2.0 * r * (std::f32::consts::PI / N as f32) + 0.1, 0.24, 0.32);
        // Its own x along the ring, which runs (-sin a, cos a) here.
        let yaw = a + std::f32::consts::FRAC_PI_2;
        b.put(at + Vec3::Y * 0.12, size, yaw, [0.0; 2], if i % 2 == 0 { LEAVES } else { LEAVES_DARK }, 0.0);
        if i % 2 == 0 {
            b.put(at + Vec3::Y * 0.3, Vec3::new(0.26, 0.16, 0.26), yaw, [0.0; 2], BLOOMS[((h >> i) % BLOOMS.len() as u32) as usize], 0.0);
        }
    }
}

/// A stall: dressed for market, its awning stretched over its rails and its counter
/// laid with its keeper's goods; at Midsummer a garland along its front whether or not.
fn stall(town: &Town, d: &Dress, dressed: bool, summer: bool, b: &mut Boxes) {
    let f = d.front.normalize_or(Vec2::Y);
    let g = Vec2::new(f.y, -f.x);
    let (f3, g3) = (Vec3::new(f.x, 0.0, f.y), Vec3::new(g.x, 0.0, g.y));
    let yaw = yaw_along(f);
    let flat = |p: Vec3| Vec2::new(p.x, p.z);
    // The counter's top: across it, from front to back, and its level.
    let (c0, c1) = d.counter;
    let (ca, cb) = (flat(c0).dot(g), flat(c1).dot(g));
    let (fa, fb) = (flat(c0).dot(f), flat(c1).dot(f));
    let (across, deep) = ((ca.min(cb), ca.max(cb)), (fa.min(fb), fa.max(fb)));
    let top = c0.y.max(c1.y);
    let at = |x: f32, z: f32, y: f32| g3 * x + f3 * z + Vec3::Y * y;
    let mut front_edge = at((across.0 + across.1) * 0.5, deep.1 + 0.05, top - 0.25);
    let mut width = across.1 - across.0;

    if let (true, Some((back, front))) = (dressed, d.awning) {
        let (da, db) = (flat(back).dot(f), flat(front).dot(f));
        let (xa, xb) = (flat(back).dot(g), flat(front).dot(g));
        let run = (db - da).abs().max(0.1);
        let rise = back.y - front.y;
        let pitch = rise.atan2(run);
        let len = (run * run + rise * rise).sqrt() + 0.3;
        let w = (xb - xa).abs() + 0.2;
        let mid = at((xa + xb) * 0.5, (da + db) * 0.5 + 0.15 * pitch.cos(), (back.y + front.y) * 0.5 + 0.06 - 0.15 * pitch.sin());
        let main = rgb(d.colour);
        let other = if d.colour == mat::CLOTH_PALE {
            rgb([mat::CLOTH_RED, mat::CLOTH_BLUE, mat::CLOTH_GREEN, mat::CLOTH_YELLOW][(d.seed % 4) as usize])
        } else {
            rgb(mat::CLOTH_PALE)
        };
        let stripes = ((w / 0.45).round() as usize).clamp(4, 9);
        let sw = w / stripes as f32;
        let down = Vec3::new(f.x * pitch.cos(), -pitch.sin(), f.y * pitch.cos());
        let edge = mid + down * (len * 0.5);
        for i in 0..stripes {
            let x = (i as f32 + 0.5) * sw - w * 0.5;
            let (cloth, hem) = if i % 2 == 0 { (main, other) } else { (other, main) };
            b.put(mid + g3 * x, Vec3::new(sw + 0.004, 0.07, len), yaw, [pitch, 0.0], cloth, 0.0);
            // The valance along its front, scalloped.
            let drop = if i % 2 == 0 { 0.5 } else { 0.36 };
            b.put(edge + g3 * x - Vec3::Y * drop * 0.5, Vec3::new(sw + 0.004, drop, 0.06), yaw, [0.0; 2], hem, 0.0);
        }
        front_edge = edge + f3 * 0.06;
        width = w;
        let keeper = d.keeper.and_then(|n| town.residents.get(n)).map(|r| r.person.trade);
        goods(town.region, keeper, d.seed, across, deep, top, &at, yaw, b);
    }
    if summer {
        let half = g3 * (width * 0.5);
        swag(front_edge - half, front_edge + half, 0.25, d.seed, false, b);
    }
}

/// One kind of thing laid out on a counter.
#[derive(Clone, Copy)]
enum Ware {
    Logs,
    Planks,
    Basket([u8; 3]),
    Fish([u8; 3]),
    Hides,
    Furs,
    Tools,
    Cloth([u8; 3], [u8; 3]),
    Honey,
    Candles,
    Sacks,
    Heap([u8; 3]),
    Cheese,
    Fleece,
    Eggs,
    Pots,
    Stones,
    Flowers([u8; 3]),
    Bread,
    Spice([u8; 3], [u8; 3]),
}

const CARROTS: [u8; 3] = [222, 118, 40];
const CABBAGES: [u8; 3] = [112, 162, 72];
const APPLES: [u8; 3] = [184, 48, 40];
const POTATOES: [u8; 3] = [184, 154, 104];
const ONIONS: [u8; 3] = [208, 172, 112];
const BERRIES: [u8; 3] = [112, 38, 84];
const MUSHROOMS: [u8; 3] = [178, 140, 100];
const DATES: [u8; 3] = [112, 60, 30];
const BANANAS: [u8; 3] = [232, 200, 58];
const ORANGES: [u8; 3] = [240, 148, 40];
const LIMES: [u8; 3] = [118, 170, 60];
const PEPPERS: [u8; 3] = [190, 40, 30];
const SAFFRON: [u8; 3] = [220, 150, 40];
const CUMIN: [u8; 3] = [150, 108, 60];
const SILVER: [u8; 3] = [168, 176, 182];
const SMOKED: [u8; 3] = [150, 104, 60];

/// What a stall's keeper sells, by their trade and the country: four things for its
/// counter, in an order of the stall's own.
fn wares(region: Region, trade: Option<Trade>, seed: u32) -> [Ware; 4] {
    use Region as R;
    use Ware as W;
    let dry = matches!(region, R::Desert);
    let hot = matches!(region, R::Rainforest | R::Wetland);
    let cold = matches!(region, R::Northern | R::Alpine | R::Upland);
    let cloth = |k: u32| rgb([mat::CLOTH_RED, mat::CLOTH_BLUE, mat::CLOTH_GREEN, mat::CLOTH_YELLOW, mat::CLOTH_PALE][((seed >> k) % 5) as usize]);
    let mut w = match trade {
        Some(Trade::Woodcutter) => [W::Logs, W::Planks, W::Basket(rgb(mat::WICKER)), W::Logs],
        Some(Trade::Forager) => [W::Basket(BERRIES), W::Basket(MUSHROOMS), W::Flowers(BLOOMS[(seed % 5) as usize]), W::Basket(if hot { LIMES } else { BERRIES })],
        Some(Trade::Fisher) => {
            let fish = if cold { SMOKED } else { SILVER };
            [W::Fish(SILVER), W::Fish(fish), W::Basket(SILVER), W::Fish(SILVER)]
        }
        Some(Trade::Hunter) => [W::Hides, if cold { W::Furs } else { W::Hides }, W::Furs, W::Hides],
        Some(Trade::Smith) => [W::Tools, W::Pots, W::Tools, W::Stones],
        Some(Trade::Weaver) => [W::Cloth(cloth(0), cloth(3)), W::Cloth(cloth(6), cloth(9)), W::Fleece, W::Cloth(cloth(12), cloth(15))],
        Some(Trade::Beekeeper) => [W::Honey, W::Candles, W::Honey, W::Flowers(BLOOMS[0])],
        Some(Trade::Herder) => [W::Cheese, W::Fleece, W::Eggs, W::Cheese],
        Some(Trade::Mason) => [W::Pots, W::Stones, W::Pots, W::Stones],
        // A farmer, and anyone else with a garden, sells what grows there.
        _ if dry => [W::Heap(DATES), W::Spice(SAFFRON, PEPPERS), W::Sacks, W::Spice(CUMIN, SAFFRON)],
        _ if hot => [W::Heap(BANANAS), W::Heap(ORANGES), W::Heap(LIMES), W::Sacks],
        _ if cold => [W::Heap(POTATOES), W::Sacks, W::Heap(ONIONS), W::Bread],
        _ => [W::Heap(APPLES), W::Heap(CARROTS), W::Heap(CABBAGES), W::Sacks],
    };
    if matches!(trade, Some(Trade::Farmer)) && !dry && !hot {
        w[3] = W::Eggs;
    }
    if dry && matches!(trade, Some(Trade::Mason) | Some(Trade::Smith)) {
        w[2] = W::Spice(PEPPERS, CUMIN);
    }
    // In its own order: the same stall the same way every market day.
    w.rotate_left((seed % 4) as usize);
    w
}

/// Lays out the keeper's goods along a counter: `across` and `deep` its top's extent,
/// `top` its level, `at` from (across, along the front, up) to continent-local.
#[allow(clippy::too_many_arguments)]
fn goods(region: Region, trade: Option<Trade>, seed: u32, across: (f32, f32), deep: (f32, f32), top: f32, at: &dyn Fn(f32, f32, f32) -> Vec3, yaw: f32, b: &mut Boxes) {
    let wares = wares(region, trade, seed);
    let slot = (across.1 - across.0 - 0.3) / wares.len() as f32;
    let z = (deep.0 + deep.1) * 0.5;
    for (i, ware) in wares.into_iter().enumerate() {
        let x = across.0 + 0.15 + slot * (i as f32 + 0.5);
        // Each part of a ware: where it sits on the counter (across, up from the top,
        // toward the front) and how big it is.
        let mut part = |dx: f32, dy: f32, dz: f32, size: [f32; 3], rgb: [u8; 3]| {
            b.put(at(x + dx, z + dz, top + dy), Vec3::from(size), yaw, [0.0; 2], rgb, 0.0);
        };
        let wood = [128, 88, 52];
        let wicker = rgb(mat::WICKER);
        match ware {
            Ware::Logs => {
                part(0.0, 0.15, -0.18, [0.95, 0.3, 0.3], wood);
                part(0.0, 0.15, 0.16, [0.95, 0.3, 0.3], [112, 78, 46]);
                part(0.0, 0.42, 0.0, [0.9, 0.28, 0.28], wood);
            }
            Ware::Planks => {
                for k in 0..3 {
                    part(0.04 * k as f32, 0.05 + 0.09 * k as f32, 0.0, [0.95, 0.08, 0.36], rgb(mat::PLANK_PALE));
                }
            }
            Ware::Basket(c) => {
                part(0.0, 0.15, 0.0, [0.7, 0.3, 0.55], wicker);
                part(0.0, 0.32, 0.0, [0.6, 0.08, 0.45], c);
                part(0.12, 0.38, -0.05, [0.25, 0.06, 0.2], blend(c, [255, 255, 255], 0.2));
            }
            Ware::Fish(c) => {
                part(0.0, 0.03, 0.0, [0.95, 0.06, 0.85], rgb(mat::PLANK_PALE));
                for k in 0..3 {
                    part(0.0, 0.11, -0.28 + 0.28 * k as f32, [0.72, 0.1, 0.18], blend(c, [80, 90, 100], 0.2 * k as f32));
                }
            }
            Ware::Hides => {
                part(0.0, 0.06, 0.0, [0.9, 0.12, 0.72], rgb(mat::LEATHER));
                part(0.05, 0.16, -0.04, [0.78, 0.08, 0.6], blend(rgb(mat::LEATHER), [255, 230, 200], 0.25));
            }
            Ware::Furs => {
                part(0.0, 0.09, 0.0, [0.92, 0.18, 0.72], [118, 96, 76]);
                part(-0.04, 0.24, 0.03, [0.7, 0.12, 0.55], [222, 218, 208]);
            }
            Ware::Tools => {
                part(-0.2, 0.05, -0.1, [0.5, 0.1, 0.5], IRON);
                part(0.25, 0.03, 0.2, [0.06, 0.06, 0.62], wood);
                part(0.25, 0.07, -0.12, [0.26, 0.13, 0.13], IRON);
                part(0.25, 0.02, -0.3, [0.24, 0.04, 0.24], [86, 84, 86]);
                part(-0.2, 0.02, 0.3, [0.24, 0.04, 0.24], [86, 84, 86]);
            }
            Ware::Cloth(a, c) => {
                part(0.0, 0.06, -0.1, [0.82, 0.12, 0.5], a);
                part(0.02, 0.18, -0.1, [0.76, 0.12, 0.48], c);
                part(-0.02, 0.3, -0.1, [0.7, 0.12, 0.46], blend(a, c, 0.5));
                part(0.0, 0.12, 0.3, [0.9, 0.24, 0.24], c);
            }
            Ware::Honey => {
                for k in 0..3 {
                    let dx = -0.3 + 0.3 * k as f32;
                    part(dx, 0.15, 0.0, [0.22, 0.3, 0.22], [206, 138, 36]);
                    part(dx, 0.32, 0.0, [0.25, 0.05, 0.25], rgb(mat::CLOTH_PALE));
                }
            }
            Ware::Candles => {
                part(0.0, 0.06, 0.1, [0.6, 0.12, 0.4], [236, 222, 180]);
                for k in 0..3 {
                    part(-0.2 + 0.2 * k as f32, 0.2, -0.2, [0.08, 0.38, 0.08], [240, 230, 196]);
                }
            }
            Ware::Sacks => {
                part(-0.15, 0.25, 0.0, [0.5, 0.5, 0.45], rgb(mat::SACKCLOTH));
                part(-0.15, 0.53, 0.0, [0.36, 0.08, 0.32], [214, 190, 120]);
                part(0.3, 0.18, 0.05, [0.36, 0.36, 0.34], rgb(mat::SACKCLOTH));
            }
            Ware::Heap(c) => {
                part(0.0, 0.05, 0.0, [0.9, 0.1, 0.7], wood);
                part(0.0, 0.17, 0.0, [0.8, 0.16, 0.6], blend(c, [0, 0, 0], 0.15));
                part(0.02, 0.3, -0.02, [0.55, 0.14, 0.4], c);
                part(-0.05, 0.4, 0.03, [0.28, 0.1, 0.2], blend(c, [255, 255, 255], 0.12));
            }
            Ware::Cheese => {
                part(-0.15, 0.1, 0.0, [0.46, 0.2, 0.46], [214, 178, 92]);
                part(-0.15, 0.3, 0.0, [0.42, 0.2, 0.42], [226, 196, 110]);
                part(0.3, 0.1, 0.1, [0.3, 0.2, 0.22], [236, 214, 140]);
            }
            Ware::Fleece => {
                part(0.0, 0.13, 0.0, [0.86, 0.26, 0.66], [226, 220, 200]);
                part(0.04, 0.3, 0.02, [0.6, 0.14, 0.48], [214, 206, 186]);
            }
            Ware::Eggs => {
                part(0.0, 0.1, 0.0, [0.55, 0.2, 0.42], wicker);
                part(0.0, 0.22, 0.0, [0.42, 0.08, 0.3], [236, 226, 204]);
            }
            Ware::Pots => {
                let clay = rgb(mat::CLAY);
                part(-0.2, 0.2, 0.0, [0.4, 0.4, 0.4], clay);
                part(-0.2, 0.46, 0.0, [0.24, 0.12, 0.24], clay);
                part(0.28, 0.15, 0.05, [0.3, 0.3, 0.3], blend(clay, [0, 0, 0], 0.15));
                part(0.28, 0.34, 0.05, [0.2, 0.08, 0.2], blend(clay, [0, 0, 0], 0.15));
            }
            Ware::Stones => {
                part(-0.1, 0.1, 0.0, [0.5, 0.2, 0.5], [128, 126, 120]);
                part(0.3, 0.07, 0.1, [0.2, 0.14, 0.55], [104, 102, 98]);
            }
            Ware::Flowers(c) => {
                part(0.0, 0.17, 0.0, [0.36, 0.34, 0.36], wood);
                part(0.0, 0.46, 0.0, [0.26, 0.3, 0.26], LEAVES);
                part(0.0, 0.66, 0.0, [0.42, 0.14, 0.42], c);
            }
            Ware::Bread => {
                for k in 0..3 {
                    part(-0.3 + 0.3 * k as f32, 0.1, 0.05 * (k % 2) as f32, [0.26, 0.2, 0.42], [192, 132, 70]);
                }
            }
            Ware::Spice(a, c) => {
                for (k, col) in [a, c].into_iter().enumerate() {
                    let dx = -0.22 + 0.44 * k as f32;
                    part(dx, 0.05, 0.0, [0.36, 0.1, 0.36], rgb(mat::CLAY));
                    part(dx, 0.13, 0.0, [0.26, 0.1, 0.26], col);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ribbons_are_brightest_at_a_festival_and_fade_until_the_next() {
        let spread = |c: [u8; 3]| c.iter().max().unwrap() - c.iter().min().unwrap();
        for cloth in [mat::CLOTH_RED, mat::CLOTH_BLUE, mat::CLOTH_YELLOW, mat::CLOTH_GREEN] {
            let new = rgb(cloth);
            let mut last = u8::MAX;
            // From Midsummer on, day by day of a year of 24, to the eve of Midwinter.
            for k in 0..12 {
                let s = spread(faded(new, Festival::Midsummer.solstice() + k as f32 / 24.0));
                assert!(s <= last, "{cloth}: brighter again {k} days on");
                last = s;
            }
            assert!(spread(faded(new, 0.74)) * 2 < spread(faded(new, 0.25)));
            // And fresh again at Midwinter.
            assert_eq!(faded(new, 0.75), faded(new, 0.25));
        }
    }

    #[test]
    fn every_stall_lays_its_goods_on_its_counter() {
        // A counter five voxels across and a voxel and a quarter deep, as a stall's is:
        // nothing laid on it overhangs its edges or sinks into it, whoever keeps it and
        // wherever.
        let mut out = Vec::new();
        let mut b = Boxes { rel: Vec3::ZERO, out: &mut out };
        let at = |x: f32, z: f32, y: f32| Vec3::new(x, y, z);
        for region in Region::ALL {
            for trade in Trade::ALL.into_iter().map(Some).chain([None]) {
                b.out.clear();
                goods(region, trade, 7, (0.0, 5.0), (0.0, 1.25), 0.0, &at, 0.0, &mut b);
                assert!(!b.out.is_empty());
                for p in b.out.iter() {
                    let (c, s) = (Vec3::from(p.pos), Vec3::from(p.size) * 0.5);
                    assert!(c.x - s.x > -0.05 && c.x + s.x < 5.05 && c.z - s.z > -0.1 && c.z + s.z < 1.35 && c.y - s.y > -0.01, "{region:?} {trade:?}: {c} {s}");
                }
            }
        }
    }
}
