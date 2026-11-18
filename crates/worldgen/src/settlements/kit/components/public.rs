//! A settlement's open spaces dressed as its homes are: sail shades strung across lanes,
//! awnings over market stalls, lines of lanterns or bunting, lanterns hung from
//! branches, and boardwalks over wet ground; and any component set on its own.
//!
//! Each takes points in the caller's own space and gives back parts in it, ready to
//! stamp. Points given in cells (tenths of a metre, [`CELLS`] to a voxel) are corners of
//! cells, as a fitting's [`Hook`]s are; so a stall's awning or a line's lanterns are made
//! straight from the hook, in the fitting's model space, and placed with it.

use glam::{IVec2, IVec3};
use voxel::{
    Voxel,
    mesh::{CELLS, PlantBox, RIBBON_BENDS},
};

use super::{Component, Cx, Made, Request, Site, build_one};
use crate::{
    features::{cloth, fittings::Hook, landmarks::Palette, shape::Rng},
    materials as mat,
    settlements::{
        Region,
        kit::{Anchor, AnchorKind, Area, Building, Door, Inside, Step, Style, rules},
    },
};

fn fixed(lo: IVec3, hi: IVec3, voxel: Voxel) -> PlantBox {
    PlantBox { lo: lo.min(hi).to_array(), hi: lo.max(hi).to_array(), voxel, sway_lo: false, sway_hi: false, thin: 0, season: 0, flap: 0 }
}

/// A colour of cloth for a place `wealth` rich (0 to 1), faded where it is `old`.
fn dye(rng: &mut Rng, wealth: f32, old: bool) -> Voxel {
    if old {
        return if rng.unit() < 0.5 { mat::CLOTH_PALE } else { mat::SACKCLOTH };
    }
    match wealth + rng.range(-0.2, 0.2) {
        w if w < 0.3 => mat::SACKCLOTH,
        w if w < 0.5 => mat::CLOTH_PALE,
        _ => [mat::CLOTH_RED, mat::CLOTH_BLUE, mat::CLOTH_YELLOW, mat::CLOTH_GREEN][(rng.next_u32() % 4) as usize],
    }
}

/// A sail shade across a lane, tied between two walls or posts facing each other across
/// it. `a` and `b` are the open voxels against what it is tied to, at the height of the
/// ties, either side of the lane (along x or along z, whichever they differ in more); it
/// spans from one to the other, `wide` voxels the other way. It is tied high at `a` and
/// lower at `b`, twisted as sails are, and sags and billows in the wind (cloth, see
/// [`cloth::sheet`]). Where the place is old (`age` over a half) it hangs lower and
/// faded. In the caller's cells.
pub fn lane_shade(a: IVec3, b: IVec3, wide: i32, wealth: f32, age: f32, seed: u32) -> Vec<PlantBox> {
    let mut rng = Rng::new(seed ^ 0x5a11);
    let old = age > 0.5 && rng.unit() < age;
    let along_x = (b.x - a.x).abs() >= (b.z - a.z).abs();
    let c = CELLS;
    let (lo, hi) = (a.min(b), a.max(b));
    let wide = wide.max(1);
    let fall = if old { 6 } else { 3 };
    let (high, low) = (a.y * c + c - 1, a.y.min(b.y) * c + c - 1 - fall);
    // Across the lane from the near face of one voxel to the far face of the other.
    let corners = if along_x {
        let (z0, z1) = (a.z * c - (wide - 1) / 2 * c, a.z * c + (wide / 2 + 1) * c);
        let (xa, xb) = (lo.x * c, hi.x * c + c);
        let near = |x: i32| if (x == xa) == (a.x <= b.x) { high } else { low };
        [IVec3::new(xa, near(xa), z0), IVec3::new(xb, near(xb) - 2, z0), IVec3::new(xb, near(xb), z1), IVec3::new(xa, near(xa) - 2, z1)]
    } else {
        let (x0, x1) = (a.x * c - (wide - 1) / 2 * c, a.x * c + (wide / 2 + 1) * c);
        let (za, zb) = (lo.z * c, hi.z * c + c);
        let near = |z: i32| if (z == za) == (a.z <= b.z) { high } else { low };
        [IVec3::new(x0, near(za), za), IVec3::new(x1, near(za) - 2, za), IVec3::new(x1, near(zb), zb), IVec3::new(x0, near(zb) - 2, zb)]
    };
    cloth::sheet(corners, dye(&mut rng, wealth, old))
}

/// The awning over a market stall: cloth stretched over the rails its [`Hook::Awning`]
/// gives, sloping to the front, with a short fringe along its front edge that flutters.
/// Where the stall is old, the cloth is faded and its front sags. In the cells the hook
/// is in (the stall's own model space, as a settlement lays the stall's parts); empty
/// for a fitting without an awning hook.
pub fn stall_awning(hooks: &[Hook], wealth: f32, age: f32, seed: u32) -> Vec<PlantBox> {
    let Some(&Hook::Awning { back, front }) = hooks.iter().find(|h| matches!(h, Hook::Awning { .. })) else { return Vec::new() };
    let mut rng = Rng::new(seed ^ 0xa1e1);
    let old = age > 0.5 && rng.unit() < age;
    let fy = front.y - if old { 3 } else { 0 };
    let colour = dye(&mut rng, wealth, old);
    let corners = [back, IVec3::new(front.x, back.y, back.z), IVec3::new(front.x, fy, front.z), IVec3::new(back.x, fy, front.z)];
    let mut out = cloth::sheet(corners, colour);
    // The fringe, in the colour or a pale stripe, a ribbon every other pair of cells.
    let fringe = if rng.unit() < 0.5 { colour } else { mat::CLOTH_PALE };
    let (x0, x1) = (back.x.min(front.x), back.x.max(front.x));
    for x in (x0..x1 - 1).step_by(4) {
        if old && rng.unit() < 0.4 {
            continue;
        }
        out.extend(cloth::ribbon(IVec3::new(x, fy - 1, front.z - 1), 2 + rng.int(0, 1), fringe, 0));
    }
    out
}

/// What can hang along a line.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Strung {
    /// Paper lanterns, lit for an evening, or not.
    Lanterns { lit: bool },
    /// Little flags of every colour, which flutter.
    Bunting,
}

/// A line strung between two points, as a [`Hook::Line`] gives them (`from` and `to` in
/// cells, sagging `sag` in the middle), hung with lanterns or bunting in place of the
/// washing on it: a market's line on market day, or a feast's. The rope is drawn too. In
/// the hook's cells.
pub fn strung(line: Hook, what: Strung, seed: u32) -> Vec<PlantBox> {
    let Hook::Line { from, to, sag } = line else { return Vec::new() };
    let mut rng = Rng::new(seed ^ 0x57e1);
    let along_x = (to.x - from.x).abs() >= (to.z - from.z).abs();
    let (s0, s1) = if along_x { (from.x.min(to.x), from.x.max(to.x)) } else { (from.z.min(to.z), from.z.max(to.z)) };
    if s1 - s0 < 4 {
        return Vec::new();
    }
    let at = |s: i32| {
        let t = (s - s0) as f32 / (s1 - s0) as f32;
        let y = from.y as f32 + (to.y - from.y) as f32 * t - sag as f32 * 4.0 * t * (1.0 - t);
        let y = y.round() as i32;
        if along_x { IVec3::new(s, y, from.z) } else { IVec3::new(from.x, y, s) }
    };
    let mut out = Vec::new();
    for s in s0..s1 {
        let p = at(s);
        out.push(fixed(p - IVec3::Y, p + IVec3::new(1, 0, 1), mat::ROPE));
    }
    let every = match what {
        Strung::Lanterns { .. } => 6,
        Strung::Bunting => 3,
    };
    for s in (s0 + every / 2..s1 - 1).step_by(every as usize) {
        let p = at(s) - IVec3::Y;
        match what {
            Strung::Lanterns { lit } => {
                let glass = if lit { mat::LANTERN } else { mat::LANTERN_COLD };
                let paper = [mat::CLOTH_RED, mat::CLOTH_YELLOW, glass][(rng.next_u32() % 3) as usize];
                out.push(fixed(p - IVec3::new(1, 3, 1), p + IVec3::new(2, 0, 2), if lit { glass } else { paper }));
            }
            Strung::Bunting => {
                let colour = dye(&mut rng, 1.0, false);
                out.extend(cloth::ribbon(p, 3, colour, !along_x as u32));
            }
        }
    }
    out
}

/// A lantern hung from a branch: a cord from `knot` (in cells, on the underside of the
/// branch) `drop` cells down to a lamp in a frame of the place's wood, lit or not.
/// Glowing, it lights its own glass at night, as a lit window does.
pub fn branch_lantern(knot: IVec3, drop: i32, pal: &Palette, lit: bool) -> Vec<PlantBox> {
    let drop = drop.clamp(1, RIBBON_BENDS);
    let low = knot.y - drop;
    let c = IVec3::new(knot.x, 0, knot.z);
    let glass = if lit { mat::LANTERN } else { mat::LANTERN_COLD };
    vec![
        fixed(c + IVec3::new(0, low, 0), c + IVec3::new(1, knot.y, 1), mat::ROPE),
        fixed(c + IVec3::new(-1, low - 1, -1), c + IVec3::new(2, low, 2), pal.wood),
        fixed(c + IVec3::new(-1, low - 4, -1), c + IVec3::new(2, low - 1, 2), glass),
        fixed(c + IVec3::new(-1, low - 5, -1), c + IVec3::new(2, low - 4, 2), pal.wood),
    ]
}

/// A boardwalk over wet ground from column `from` to `to` (along x or along z, whichever
/// they differ in more), `wide` voxels across, its deck in layer `deck`. `ground(x, z)`
/// is the caller's top solid layer in each column: posts go down to it every third
/// column along each edge where the deck stands above it, and their tops show a little
/// above the deck, with a rope between them. Where the place is old some boards are
/// darker. Its voxels, and its fine parts in cells.
pub fn boardwalk(from: IVec2, to: IVec2, wide: i32, deck: i32, ground: impl Fn(i32, i32) -> i32, pal: &Palette, age: f32, seed: u32) -> (Vec<(IVec3, Voxel)>, Vec<PlantBox>) {
    let mut rng = Rng::new(seed ^ 0xb0a2);
    let along_x = (to.x - from.x).abs() >= (to.y - from.y).abs();
    let (s0, s1) = if along_x { (from.x.min(to.x), from.x.max(to.x)) } else { (from.y.min(to.y), from.y.max(to.y)) };
    let across = if along_x { from.y } else { from.x };
    let (w0, w1) = (across - (wide - 1) / 2, across + wide / 2);
    let col = |s: i32, w: i32| if along_x { IVec2::new(s, w) } else { IVec2::new(w, s) };
    let (mut voxels, mut fine) = (Vec::new(), Vec::new());
    for s in s0..=s1 {
        for w in w0..=w1 {
            let q = col(s, w);
            let board = if age > 0.5 && rng.unit() < age * 0.4 { mat::PLANK_DARK } else { pal.plank };
            voxels.push((IVec3::new(q.x, deck, q.y), board));
            let edge = w == w0 || w == w1;
            if edge && (s - s0) % 3 == 0 {
                for y in ground(q.x, q.y) + 1..deck {
                    voxels.push((IVec3::new(q.x, y, q.y), pal.wood));
                }
                let c = IVec3::new(q.x * CELLS + 1, (deck + 1) * CELLS, q.y * CELLS + 1);
                fine.push(fixed(c, c + IVec3::new(2, 6, 2), pal.wood));
            }
        }
    }
    for w in [w0, w1] {
        let a = col(s0, w);
        let b = col(s1, w);
        let y = (deck + 1) * CELLS + 4;
        let (lo, hi) = (IVec3::new(a.x * CELLS + 1, y, a.y * CELLS + 1), IVec3::new(b.x * CELLS + 3, y + 1, b.y * CELLS + 3));
        fine.push(fixed(lo, hi, mat::ROPE));
    }
    (voxels, fine)
}

/// A building with nothing to it, standing for open ground about `at`.
fn nowhere(at: IVec3) -> Building {
    let here = Area { lo: IVec2::new(at.x, at.z), hi: IVec2::new(at.x, at.z) };
    Building {
        voxels: Vec::new(),
        fine: Vec::new(),
        footprint: here,
        walls: here,
        door: Door { floor: at, facing: IVec3::X, width: 2, height: 5 },
        anchors: Vec::new(),
        inside: Inside::default(),
        clear: Vec::new(),
        smoke: at,
        style: Style { form: rules::Form::Rect, walls: rules::WallKind::Plank, roof: rules::Cover::Thatch, shape: rules::RoofShape::Gable, raised: 0 },
    }
}

/// A component set on its own on open, level ground, as it would be in a yard: `at` is
/// the voxel it stands in on the first layer above the ground (the caller's `y`), and
/// `facing` (+X, -X, +Z or -Z) the way its front looks. Only what stands free comes out
/// (a water jar, a bench, an oven, a hen coop, a sail shade, a punt); what needs a wall,
/// a window or a roof gives None. The ground beneath and about it is the caller's to
/// keep clear.
#[allow(clippy::too_many_arguments)]
pub fn standalone(c: Component, at: IVec3, facing: IVec3, region: Region, pal: &Palette, wealth: f32, age: f32, seed: u32) -> Option<Made> {
    // Worked out about the origin, as on the ground in a yard, and moved to `at` after.
    let anchor = Anchor { kind: AnchorKind::Yard, at: IVec3::ZERO, normal: facing };
    let b = nowhere(IVec3::ZERO);
    let site = Site::open();
    let req = Request {
        anchor,
        region,
        step: Step::Hut,
        style: b.style,
        pal,
        wealth,
        age,
        decayed: age > 0.6 && Rng::new(seed).unit() < age - 0.3,
        great_town: false,
        seed,
    };
    let mut cx = Cx::new(&req, &b, &site);
    build_one(c, &mut cx)?;
    let made = cx.finish();
    if made.is_empty() || !site.fits(&made, false) {
        return None;
    }
    let d = at;
    Some(Made {
        voxels: made.voxels.into_iter().map(|(q, v)| (q + d, v)).collect(),
        fine: made.fine.into_iter().map(|p| PlantBox { lo: (IVec3::from(p.lo) + d * CELLS).to_array(), hi: (IVec3::from(p.hi) + d * CELLS).to_array(), ..p }).collect(),
        paint: Vec::new(),
        refit: Vec::new(),
    })
}
