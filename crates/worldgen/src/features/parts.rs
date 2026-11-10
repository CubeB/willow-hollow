//! Parts for built things, modelled directly in 0.4 m voxels (not the 0.8 m blocks the
//! older landmark code lays out in): lantern posts in five styles, and the trims that
//! stop a block structure reading as a heap of cubes.
//!
//! Every part takes its colours from a [`Palette`], so a lantern in the desert is
//! sandstone and acacia and one in the redwoods is mossy rock and red timber.
//!
//! A lantern is two things. Its voxels are a sturdy silhouette that reads from far off
//! and holds the lit voxel the renderer lights the ground from. Its fine parts, 0.1 m
//! boxes drawn like plants (see [`voxel::mesh::PlantBox`]), are what make it look made
//! up close: framed glass, a cap with an overhang and finial, a bracket, hook and chain,
//! a moulded base and a small flame under the cap. Fine parts never hold anything up:
//! every voxel touches another, so a lantern is whole even where plants are not drawn.

use glam::IVec3;
use voxel::{Voxel, mesh::{CELLS, PlantBox}};

use super::{landmarks::Palette, shape::{Rng, Shape, rotate}};
use crate::materials as mat;

/// How a lantern is built.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LampStyle {
    /// A post with a framed lamp on top, under a vented cap and finial.
    Post,
    /// A shepherd's crook: an arm with a hooked end, the lamp hanging in the hook.
    Crook,
    /// A stone lantern: tiered base, latticed fire box, a roof with upturned corners.
    Stone,
    /// A crossbar with a hooked end each side, a lamp hanging in each.
    Pair,
    /// Two posts under a lintel, a lamp on a tie beam between them, chained above.
    Gate,
}

impl LampStyle {
    pub const ALL: [LampStyle; 5] = [LampStyle::Post, LampStyle::Crook, LampStyle::Stone, LampStyle::Pair, LampStyle::Gate];

    /// A style for a lamp post at a landmark, from a hash, favouring the plain post. The
    /// gate is left out: it is too wide for the corners of a site.
    pub fn pick(hash: u32) -> Self {
        match hash % 8 {
            0..=2 => LampStyle::Post,
            3..=4 => LampStyle::Crook,
            5 => LampStyle::Stone,
            _ => LampStyle::Pair,
        }
    }

    /// A style for a lantern beside a trail, from a hash: any of the five.
    pub fn pick_trail(hash: u32) -> Self {
        match hash % 16 {
            0..=4 => LampStyle::Post,
            5..=8 => LampStyle::Crook,
            9..=10 => LampStyle::Stone,
            11..=12 => LampStyle::Pair,
            _ => LampStyle::Gate,
        }
    }

    /// Whether the style's long axis is wide enough to want laying along a trail.
    pub fn is_wide(self) -> bool {
        matches!(self, LampStyle::Pair | LampStyle::Gate)
    }
}

/// The voxels of a lantern post `height` voxels tall to the lamp, standing on the ground
/// with its footing below. The origin column is the post (for a gate, the middle between
/// its posts), the first layer above the ground is `y = 0`, and the arm of a crook points
/// along +X unless `turns` says otherwise.
pub fn lantern(style: LampStyle, p: &Palette, height: i32, turns: u32) -> Vec<(IVec3, Voxel)> {
    lantern_parts(style, p, height, turns).0
}

/// A lantern's voxels and its fine parts, the parts in cells (1/[`CELLS`] voxel) of the
/// same model space.
pub fn lantern_parts(style: LampStyle, p: &Palette, height: i32, turns: u32) -> (Vec<(IVec3, Voxel)>, Vec<PlantBox>) {
    lantern_parts_spanning(style, p, height, turns, GATE_HALF)
}

/// The usual distance of a gate's posts either side of its middle.
pub const GATE_HALF: i32 = 2;
/// The furthest a gate's posts stand either side of its middle: its lintel, one voxel
/// further out each side, is then 11 voxels wide.
pub const GATE_HALF_MAX: i32 = 4;

/// [`lantern_parts`], with a gate's posts `half` voxels either side of its middle (up to
/// [`GATE_HALF_MAX`]), so it spans a wider trail. Other styles ignore `half`.
pub fn lantern_parts_spanning(style: LampStyle, p: &Palette, height: i32, turns: u32, half: i32) -> (Vec<(IVec3, Voxel)>, Vec<PlantBox>) {
    let mut s = Shape::raw();
    paint_lantern_spanning(&mut s, style, p, IVec3::ZERO, height, turns, half);
    let fine = s.take_fine();
    (s.into_voxels(), fine)
}

/// Turns a box of model cells (`hi` exclusive) by quarter turns about the middle of voxel
/// column 0, just as [`rotate`] turns voxels.
pub fn rotate_box(lo: IVec3, hi: IVec3, turns: u32) -> (IVec3, IVec3) {
    let c = IVec3::new(CELLS / 2, 0, CELLS / 2);
    let (a, b) = (rotate(lo - c, turns) + c, rotate(hi - c, turns) + c);
    (a.min(b), a.max(b))
}

/// Moves fine parts made in model space to where the model stands: turned, then with
/// model voxel 0 at voxel `origin`.
pub fn place_fine(parts: &[PlantBox], origin: IVec3, turns: u32) -> Vec<PlantBox> {
    parts
        .iter()
        .map(|b| {
            let (lo, hi) = rotate_box(IVec3::from_array(b.lo), IVec3::from_array(b.hi), turns);
            PlantBox { lo: (lo + origin * CELLS).to_array(), hi: (hi + origin * CELLS).to_array(), ..*b }
        })
        .collect()
}

/// Lays fine parts into a shape for one lantern: each box is given in cells from the
/// corner of a voxel of the lantern, before the lantern is turned.
struct Fine<'a> {
    s: &'a mut Shape,
    at: IVec3,
    turns: u32,
}

impl Fine<'_> {
    fn put(&mut self, voxel: IVec3, lo: (i32, i32, i32), hi: (i32, i32, i32), v: Voxel) {
        let base = voxel * CELLS;
        let (lo, hi) = rotate_box(base + IVec3::from(lo), base + IVec3::from(hi), self.turns);
        let at = self.at * CELLS;
        debug_assert!(lo.cmplt(hi).all(), "empty fine part");
        // Fixed to the lantern, never swaying.
        self.s.add_fine(PlantBox { lo: (lo + at).to_array(), hi: (hi + at).to_array(), voxel: v, sway_lo: false, sway_hi: false, thin: 0 });
    }

    /// Thin frames round the lit voxel `l`, so it reads as four panes of glass: a rail
    /// round its foot and its head, and a mullion up each corner between the panes.
    fn glazing(&mut self, l: IVec3, frame: Voxel) {
        self.put(l, (-1, -1, -1), (5, 1, 5), frame);
        self.put(l, (-1, 3, -1), (5, 5, 5), frame);
        for (x, z) in [(0, 0), (4, 0), (0, 4), (4, 4)] {
            self.put(l, (x - 1, 1, z - 1), (x + 1, 3, z + 1), frame);
        }
    }

    /// A whole lamp round the lit voxel `l`: the glazing, a vent `vent` cells tall above
    /// it with the flame showing through it on its candle, and a cap of `tiers` steps
    /// whose first overhangs the frame. Returns the height of the top of the cap, in
    /// cells above the foot of `l`.
    fn lamp(&mut self, l: IVec3, frame: Voxel, cap: Voxel, vent: i32, tiers: i32) -> i32 {
        self.glazing(l, frame);
        let top = 5 + vent;
        for (x, z) in [(0, 0), (4, 0), (0, 4), (4, 4)] {
            self.put(l, (x - 1, 5, z - 1), (x + 1, top, z + 1), frame);
        }
        // The candle's flame, a little off centre, as flames are.
        self.put(l, (1, 5, 1), (3, 6, 3), mat::LANTERN);
        if vent > 1 {
            self.put(l, (1, 6, 2), (2, 7, 3), mat::LANTERN);
        }
        for k in 0..tiers {
            let r = 2 - k;
            self.put(l, (-r, top + k, -r), (4 + r, top + k + 1, 4 + r), cap);
        }
        top + tiers
    }

    /// A lamp hung from the underside of voxel `l + (0, drop, 0)`: a drip below the
    /// glass, a ring on the cap, chain links up to a plate under the arm.
    fn hanging(&mut self, l: IVec3, drop: i32, frame: Voxel, cap: Voxel) {
        self.put(l, (1, -3, 1), (3, -1, 3), frame);
        let top = self.lamp(l, frame, cap, 1, 2);
        let arm = drop * CELLS;
        self.chain(l, top, arm, frame);
    }

    /// A ring on a cap at height `from` and chain links up to a plate under an arm at
    /// `to`, all in cells above the foot of `l`. Links alternate their flat side.
    fn chain(&mut self, l: IVec3, from: i32, to: i32, metal: Voxel) {
        self.put(l, (1, from, 1), (3, from + 1, 3), metal);
        let mut y = from + 1;
        let mut flat = false;
        while y < to - 1 {
            let next = (y + 2).min(to - 1);
            if flat {
                self.put(l, (2, y, 1), (3, next, 3), metal);
            } else {
                self.put(l, (1, y, 2), (3, next, 3), metal);
            }
            flat = !flat;
            y = next;
        }
        // The hook's plate under the arm.
        self.put(l, (0, to - 1, 1), (4, to, 3), metal);
    }

    /// A stepped strut under an arm, from the side of the post (voxel column `post`) up
    /// into the arm's underside at voxel layer `arm`, reaching one voxel out along `dir`
    /// (+1 or -1 in x).
    fn strut(&mut self, post: IVec3, arm: i32, dir: i32, wood: Voxel) {
        let at = IVec3::new(post.x, arm - 2, post.z);
        for k in 0..4 {
            let (x0, x1) = if dir > 0 { (4 + k, 5 + k) } else { (-1 - k, -k) };
            self.put(at, (x0, 4 + k, 1), (x1, 6 + k, 3), wood);
        }
    }

    /// A moulded foot round the post at voxel `post`: a broad step and a narrower bead.
    fn moulding(&mut self, post: IVec3, stone: Voxel) {
        self.put(post, (-2, 0, -2), (6, 1, 6), stone);
        self.put(post, (-1, 1, -1), (5, 2, 5), stone);
    }

    /// A finial on top of voxel `on`, or at `y` cells above its foot.
    fn finial(&mut self, on: IVec3, y: i32, neck: Voxel, knob: Voxel) {
        self.put(on, (1, y, 1), (3, y + 1, 3), neck);
        self.put(on, (1, y + 1, 1), (3, y + 3, 3), knob);
    }
}

/// Puts a lantern into a shape, with its foot at voxel `at` (the first layer above the
/// ground), and its fine parts with it.
pub fn paint_lantern(s: &mut Shape, style: LampStyle, p: &Palette, at: IVec3, height: i32, turns: u32) {
    paint_lantern_spanning(s, style, p, at, height, turns, GATE_HALF);
}

/// [`paint_lantern`], with a gate's posts `half` voxels either side of its middle.
pub fn paint_lantern_spanning(s: &mut Shape, style: LampStyle, p: &Palette, at: IVec3, height: i32, turns: u32, half: i32) {
    let half = half.clamp(GATE_HALF, GATE_HALF_MAX);
    let h = height.max(5);
    let mut voxels: Vec<(IVec3, Voxel)> = Vec::new();
    let mut put = |x: i32, y: i32, z: i32, v: Voxel| voxels.push((IVec3::new(x, y, z), v));
    let v = IVec3::new;
    let iron = mat::CHARCOAL;
    // A footing running down into the ground, so nothing floats on uneven land, and a
    // plus-shaped stone base on it; but a gate stands on its two posts' own footings,
    // leaving the way between them clear.
    if style != LampStyle::Gate {
        for y in -3..0 {
            put(0, y, 0, p.stone_dark);
        }
        for (x, z) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
            put(x, 0, z, p.stone_dark);
        }
    }
    // The voxels first, turned as they go in; the fine parts after, in their own pass.
    match style {
        LampStyle::Post => {
            put(0, 1, 0, p.stone);
            for y in 2..h {
                put(0, y, 0, p.wood);
            }
            put(0, h, 0, mat::LANTERN);
        }
        LampStyle::Crook => {
            // The lamp hangs two voxels below the arm, in the curl of its end.
            let (l, arm) = (h - 2, h + 1);
            put(0, 1, 0, p.stone);
            for y in 2..=arm {
                put(0, y, 0, p.wood);
            }
            put(0, arm + 1, 0, p.roof);
            for x in 1..=3 {
                put(x, arm, 0, p.wood);
            }
            for y in l..arm {
                put(3, y, 0, p.wood);
            }
            put(2, l, 0, mat::LANTERN);
        }
        LampStyle::Stone => {
            // A tiered stone lantern.
            for y in 1..=2 {
                put(0, y, 0, p.stone);
            }
            for (x, z) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
                put(x, 3, z, p.stone_dark);
            }
            put(0, 4, 0, mat::LANTERN);
            for (x, z) in [(1, 1), (-1, 1), (1, -1), (-1, -1)] {
                put(x, 4, z, p.stone);
            }
            for x in -1..=1 {
                for z in -1..=1 {
                    put(x, 5, z, p.roof);
                }
            }
            put(0, 6, 0, p.roof);
            put(0, 7, 0, p.stone);
        }
        LampStyle::Pair => {
            let (l, arm) = (h - 2, h + 1);
            put(0, 1, 0, p.stone);
            for y in 2..=arm {
                put(0, y, 0, p.wood);
            }
            put(0, arm + 1, 0, p.accent);
            for x in -3..=3 {
                put(x, arm, 0, p.wood);
            }
            for x in [-3, 3] {
                for y in l..arm {
                    put(x, y, 0, p.wood);
                }
            }
            for x in [-2, 2] {
                put(x, l, 0, mat::LANTERN);
            }
        }
        LampStyle::Gate => {
            // Posts `half` voxels either side, the lamp on a tie beam between them.
            let (l, lintel) = (h - 2, h + 1);
            for x in [-half, half] {
                for y in -3..0 {
                    put(x, y, 0, p.stone_dark);
                }
                put(x, 0, 0, p.stone_dark);
                put(x, 1, 0, p.stone);
                for y in 2..lintel {
                    put(x, y, 0, p.wood);
                }
            }
            for x in -(half - 1)..=half - 1 {
                put(x, l - 1, 0, p.wood);
            }
            put(0, l, 0, mat::LANTERN);
            for x in -(half + 1)..=half + 1 {
                put(x, lintel, 0, p.wood);
            }
            for x in -half..=half {
                put(x, lintel + 1, 0, p.roof);
            }
        }
    }
    for (q, voxel) in voxels {
        let r = rotate(q, turns);
        s.paint(at + IVec3::new(r.x, q.y, r.z), voxel);
    }

    let mut f = Fine { s, at, turns };
    match style {
        LampStyle::Post => {
            f.moulding(v(0, 1, 0), p.stone);
            // A collar where the post meets the lamp.
            f.put(v(0, h - 1, 0), (-1, 1, -1), (5, 3, 5), p.wood);
            let top = f.lamp(v(0, h, 0), iron, p.roof, 2, 3);
            f.finial(v(0, h, 0), top, iron, p.accent);
        }
        LampStyle::Crook => {
            let (l, arm) = (h - 2, h + 1);
            f.moulding(v(0, 1, 0), p.stone);
            f.strut(v(0, 0, 0), arm, 1, p.wood);
            f.hanging(v(2, l, 0), arm - l, iron, iron);
            f.finial(v(0, arm + 1, 0), CELLS, iron, p.accent);
        }
        LampStyle::Stone => {
            f.moulding(v(0, 1, 0), p.stone);
            // A latticed window in the fire box.
            f.glazing(v(0, 4, 0), p.wood);
            // A drip along the roof's edge, and its corners turned up.
            f.put(v(0, 5, 0), (-5, 1, -5), (9, 3, 9), p.roof);
            for (x, z) in [(-5, -5), (7, -5), (-5, 7), (7, 7)] {
                f.put(v(0, 5, 0), (x, 3, z), (x + 2, 6, z + 2), p.roof);
            }
            // A jewel finial on the top stone.
            f.put(v(0, 7, 0), (1, 4, 1), (3, 5, 3), p.stone_dark);
            f.put(v(0, 7, 0), (0, 5, 0), (4, 7, 4), p.stone);
            f.put(v(0, 7, 0), (1, 7, 1), (3, 9, 3), p.stone);
        }
        LampStyle::Pair => {
            let (l, arm) = (h - 2, h + 1);
            f.moulding(v(0, 1, 0), p.stone);
            for (x, dir) in [(-2, -1), (2, 1)] {
                f.strut(v(0, 0, 0), arm, dir, p.wood);
                f.hanging(v(x, l, 0), arm - l, iron, iron);
            }
            f.finial(v(0, arm + 1, 0), CELLS, iron, p.accent);
        }
        LampStyle::Gate => {
            let (l, lintel) = (h - 2, h + 1);
            for x in [-half, half] {
                f.moulding(v(x, 1, 0), p.stone);
            }
            // Knee braces from the posts in under the lintel.
            f.strut(v(-half, 0, 0), lintel, 1, p.wood);
            f.strut(v(half, 0, 0), lintel, -1, p.wood);
            let top = f.lamp(v(0, l, 0), iron, p.roof, 1, 2);
            f.chain(v(0, l, 0), top, (lintel - l) * CELLS, iron);
            f.finial(v(0, lintel + 1, 0), CELLS, iron, p.accent);
        }
    }
}

/// Breaks up the hard outline of blocky stonework: some of the voxels standing out at the
/// corners and top edges of a stone block are chipped away, a few more worn down to the
/// darker rock, and rubble (a loose voxel) lies at the foot of some walls. Only stone is
/// touched, never timber, so stairs, floors and doors keep their shape.
pub fn weather_stone(s: &mut Shape, rng: &mut Rng, p: &Palette, chip: f32) {
    let stone = [p.stone, p.stone_dark, mat::MOSSY_STONE, mat::SANDSTONE, mat::SANDSTONE_DARK, mat::SLATE];
    let is_stone = |v: Option<Voxel>| v.is_some_and(|v| stone.contains(&v));
    let at = |s: &Shape, q: IVec3| s.get(q).filter(|v| *v != voxel::AIR);
    let mut chips = Vec::new();
    let mut worn = Vec::new();
    for q in s.snapshot() {
        let v = s.get(q);
        if !is_stone(v) {
            continue;
        }
        let open = |d: IVec3| at(s, q + d).is_none();
        let top = open(IVec3::Y);
        // Exposed on top and at least one side: an edge or a corner.
        let sides = [IVec3::X, IVec3::NEG_X, IVec3::Z, IVec3::NEG_Z].iter().filter(|d| open(**d)).count();
        if top && sides >= 2 && rng.unit() < chip {
            chips.push(q);
        } else if (top || sides >= 1) && rng.unit() < chip * 0.5 {
            worn.push(q);
        }
    }
    for q in chips {
        s.paint(q, voxel::AIR);
    }
    for q in worn {
        let darker = if s.get(q) == Some(p.stone_dark) { p.stone } else { p.stone_dark };
        s.paint(q, darker);
    }
}
