//! A great town's park (#203): green ground kept round the village tree, which was
//! there before the town and has the room to grow great; a raised stone pond with
//! lilies, beds of the country's own wild flowers about the tree's ground, the open
//! pavilion to stand under, a games lawn set out for skittles, and benches.
//!
//! The ground is kept before any lane or plot ([`Town::keep_for_park`]), so the lanes
//! go round it, and what is in it is laid once every plot and path is down
//! ([`Town::park`]), each thing only where nothing else is.

use glam::{IVec2, IVec3, Vec2, vec2};
use voxel::{
    Voxel,
    mesh::{CELLS, PlantBox},
};

use super::{Disc, GREEN, KEEP, PLOT, THING, Town, WAY, edges::Park, turn_rect};
use crate::{
    features::{
        designs::Special,
        fittings,
        parts,
        plants::{self, Flower},
        props::{self, Prop},
        shape::rotate,
    },
    materials as mat,
    noise::{hash2, unit},
    scale::m,
    settlements::{
        Kind,
        special::{self, Facts, SpecialSpec},
    },
    surface::Sampler,
    trails::{Network, build::turns_toward},
};

/// The pond's radius, to the inside of its rim.
const POND: i32 = 5;

impl Town<'_> {
    /// Keeps the ground for a great town's park round the ground kept for its tree,
    /// reaching out from the green away from the middle: as big as there is room for,
    /// up to thirteen metres across from the middle, on ground near enough level.
    pub(super) fn keep_for_park(&mut self) {
        if self.s.kind != Kind::GreatTown {
            return;
        }
        let Some((tree, tr)) = self.layout.tree else { return };
        let out = (tree - self.centre).normalize_or(Vec2::X);
        for r in [m(13.0), m(11.0), m(9.0)] {
            for j in 0..9 {
                let turn = (j as f32 / 2.0).ceil() * if j % 2 == 0 { 1.0 } else { -1.0 } * 0.3;
                let d = Vec2::from_angle(out.to_angle() + turn);
                let at = tree + d * (r - tr - 2.0);
                let from = at.distance(self.centre);
                if from + r > self.s.radius - 2.0 || from - r < self.green.radius - 2.0 {
                    continue;
                }
                if !self.worn.clear(at, r, 2.0) || self.discs.iter().any(|q| q.at != tree && q.at != self.centre && q.at.distance(at) < q.r + r + 1.0) {
                    continue;
                }
                let Some((_, lo, hi)) = self.probe(at, r) else { continue };
                if hi - lo > m(2.4) {
                    continue;
                }
                self.discs.push(Disc { at, r, y: None, owner: None });
                self.grid.disc(at, r, KEEP);
                self.edge.park = Some((at, r));
                return;
            }
        }
    }

    /// Lays out the park kept for: the pond out from the tree, the pavilion to one side
    /// and the games lawn to the other, flower beds round the tree's ground and benches
    /// by the pond and facing the tree, each where its ground is free and near level.
    pub(super) fn park(&mut self, net: &mut Network, fin: &Sampler) {
        let Some((at, r)) = self.edge.park else { return };
        let Some((tree, tr)) = self.layout.tree else { return };
        let mut park = Park { at, radius: r, pond: None, pavilion: None, lawn: None, benches: Vec::new() };
        let u = (at - tree).normalize_or(Vec2::X);
        let v = vec2(-u.y, u.x);
        let seed = self.s.seed ^ 0x9a7c_0000;
        let top = |c: IVec2| fin.column(c.x, c.y).top();
        let dry = |c: IVec2| fin.column(c.x, c.y).water_surface().is_none();
        let free = |t: &Self, c: IVec2| {
            let p = c.as_vec2() + 0.5;
            t.grid.get(c) & (WAY | PLOT | THING | GREEN) == 0 && p.distance(at) < r - 1.0 && p.distance(tree) > tr && dry(c)
        };
        let level = |cols: &[IVec2]| {
            let (lo, hi) = cols.iter().fold((i32::MAX, i32::MIN), |(a, b), &c| (a.min(top(c)), b.max(top(c))));
            (hi - lo <= 2).then_some(hi)
        };
        let disc = |c: IVec2, rad: i32| -> Vec<IVec2> { (-rad..=rad).flat_map(|z| (-rad..=rad).map(move |x| IVec2::new(x, z))).filter(|d| d.length_squared() <= rad * rad + rad).map(|d| c + d).collect() };

        // The pond, out from the tree: a basin of stone raised a little on the lawn, its
        // water two layers over the highest ground under it so nothing the ground grows
        // comes up through it.
        'pond: for f in [0.45, 0.3, 0.6] {
            for side in [0.0, 0.3, -0.3] {
                let c = (at + u * r * f + v * r * side).floor().as_ivec2();
                let cols = disc(c, POND + 1);
                if !cols.iter().all(|&q| free(self, q)) {
                    continue;
                }
                let Some(hi) = level(&cols) else { continue };
                let fl = hi + 1;
                let mut voxels: Vec<(IVec3, Voxel)> = Vec::new();
                for &q in &cols {
                    let d = (q - c).as_vec2().length();
                    let g = top(q);
                    if d <= POND as f32 - 0.25 {
                        for y in g + 1..fl {
                            voxels.push((IVec3::new(q.x, y, q.y), self.pal.stone_dark));
                        }
                        voxels.push((IVec3::new(q.x, fl, q.y), mat::PATH_GRAVEL));
                        let lily = unit(hash2(seed ^ 0x1117, q.x, q.y)) < 0.12 && d < POND as f32 - 1.5;
                        voxels.push((IVec3::new(q.x, fl + 1, q.y), if lily { mat::LILY } else { mat::WATER }));
                    } else {
                        for y in g + 1..=fl + 1 {
                            voxels.push((IVec3::new(q.x, y, q.y), if y == fl + 1 { self.pal.stone } else { self.pal.stone_dark }));
                        }
                    }
                    self.grid.mark(q, THING);
                }
                net.add_stamp_with(voxels, Vec::new());
                park.pond = Some((IVec3::new(c.x, fl + 1, c.y), POND));
                // Benches round it, facing in.
                for k in 0..3 {
                    let a = u.to_angle() + std::f32::consts::PI + (k as f32 - 1.0) * 0.9;
                    let p = c.as_vec2() + 0.5 + Vec2::from_angle(a) * (POND as f32 + 3.5);
                    self.bench(net, fin, &mut park, p, Vec2::from_angle(a), seed ^ k);
                }
                break 'pond;
            }
        }

        // The pavilion to one side, its open front to the park's middle.
        let mut pavilion_side = 1.0;
        'pavilion: for side in [1.0, -1.0] {
            for f in [0.5, 0.4, 0.6] {
                let c = (at + v * r * f * side).floor().as_ivec2();
                let turns = turns_toward(at - (c.as_vec2() + 0.5));
                let spec = SpecialSpec { which: Special::Pavilion, region: self.s.region, age: 0.3, wealth: self.s.prosperity, seed: hash2(seed, c.x, c.y), stream: None, facts: Facts::default() };
                let Some(built) = special::build(&spec, &self.pal) else { break 'pavilion };
                let b = &built.building;
                let (lo, hi) = turn_rect((b.footprint.lo, b.footprint.hi), turns);
                let cols: Vec<IVec2> = (lo.y..=hi.y).flat_map(|z| (lo.x..=hi.x).map(move |x| IVec2::new(x, z) + c)).collect();
                if !cols.iter().all(|&q| free(self, q)) {
                    continue;
                }
                let Some(hi_y) = level(&cols) else { continue };
                let origin = IVec3::new(c.x, hi_y + 1, c.y);
                let voxels = b.voxels.iter().map(|&(q, v)| (origin + rotate(q, turns), v)).collect();
                net.add_stamp_with(voxels, parts::place_fine(&b.fine, origin, turns));
                for &q in &cols {
                    self.grid.mark(q, THING);
                }
                park.pavilion = Some((origin, turns));
                pavilion_side = side;
                break 'pavilion;
            }
        }

        // The games lawn on the other side: a green kept short, pegged at its corners,
        // with skittles set up at one end and the ball lying by them.
        let (w, d) = if v.x.abs() > v.y.abs() { (8, 14) } else { (14, 8) };
        'lawn: for f in [0.5, 0.4, 0.6] {
            let c = (at - v * r * f * pavilion_side).floor().as_ivec2();
            let lo = c - IVec2::new(w / 2, d / 2);
            let hi = lo + IVec2::new(w - 1, d - 1);
            let cols: Vec<IVec2> = (lo.y - 1..=hi.y + 1).flat_map(|z| (lo.x - 1..=hi.x + 1).map(move |x| IVec2::new(x, z))).collect();
            if !cols.iter().all(|&q| free(self, q)) || level(&cols).is_none() {
                continue;
            }
            let mut voxels: Vec<(IVec3, Voxel)> = Vec::new();
            let mut fine: Vec<PlantBox> = Vec::new();
            for q in [lo, IVec2::new(hi.x, lo.y), IVec2::new(lo.x, hi.y), hi] {
                voxels.push((IVec3::new(q.x, top(q) + 1, q.y), self.pal.wood));
            }
            // Nine pins in a diamond at the far end, the ball at the near.
            let long_x = w > d;
            let (end, start) = if long_x { (IVec2::new(hi.x - 2, c.y), IVec2::new(lo.x + 2, c.y)) } else { (IVec2::new(c.x, hi.y - 2), IVec2::new(c.x, lo.y + 2)) };
            let gy = top(end) + 1;
            for i in -1..=1 {
                for j in -1..=1 {
                    let (dx, dz) = ((i + j) * 2, (i - j) * 2);
                    let (x, z) = (end.x * CELLS + 2 + dx, end.y * CELLS + 2 + dz);
                    fine.push(PlantBox { lo: [x, gy * CELLS, z], hi: [x + 1, gy * CELLS + 4, z + 1], voxel: mat::PLANK_PALE, sway_lo: false, sway_hi: false, thin: 0, season: 0, flap: 0 });
                }
            }
            let by = top(start) + 1;
            let (x, z) = (start.x * CELLS + 1, start.y * CELLS + 1);
            fine.push(PlantBox { lo: [x, by * CELLS, z], hi: [x + 2, by * CELLS + 2, z + 2], voxel: mat::PLANK_DARK, sway_lo: false, sway_hi: false, thin: 0, season: 0, flap: 0 });
            for &q in &cols {
                self.grid.mark(q, THING);
            }
            net.add_stamp_with(voxels, fine);
            park.lawn = Some((lo, hi));
            break 'lawn;
        }

        // Beds of the country's wild flowers round the tree's ground, a kerb about each
        // and the bed raised on it, two kinds to a bed.
        let kinds: Vec<Flower> = fittings::herbs_of(self.s.region).iter().copied().filter(|f| !matches!(f, Flower::PennyBun | Flower::FlyAgaric)).collect();
        if !kinds.is_empty() {
            for k in 0..8 {
                let a = u.to_angle() + std::f32::consts::FRAC_PI_4 * k as f32 + 0.4;
                let c = (tree + Vec2::from_angle(a) * (tr + 3.0)).floor().as_ivec2();
                let (bw, bd) = if k % 2 == 0 { (3, 2) } else { (2, 3) };
                let lo = c - IVec2::ONE;
                let hi = lo + IVec2::new(bw + 1, bd + 1);
                let cols: Vec<IVec2> = (lo.y..=hi.y).flat_map(|z| (lo.x..=hi.x).map(move |x| IVec2::new(x, z))).collect();
                if !cols.iter().all(|&q| free(self, q)) {
                    continue;
                }
                let Some(hi_y) = level(&cols) else { continue };
                let (mut voxels, mut fine): (Vec<(IVec3, Voxel)>, Vec<PlantBox>) = (Vec::new(), Vec::new());
                let pair = [kinds[(hash2(seed, k, 1) as usize) % kinds.len()], kinds[(hash2(seed, k, 2) as usize) % kinds.len()]];
                for &q in &cols {
                    let edge = q.x == lo.x || q.y == lo.y || q.x == hi.x || q.y == hi.y;
                    for y in top(q) + 1..=hi_y + 1 {
                        voxels.push((IVec3::new(q.x, y, q.y), if edge { self.pal.stone_dark } else { mat::DIRT }));
                    }
                    if !edge {
                        let h = hash2(seed ^ 0xf10e, q.x, q.y);
                        plants::plant_flower(pair[(h % 2) as usize], [q.x, hi_y + 2, q.y], h, &mut fine);
                    }
                    self.grid.mark(q, THING);
                }
                net.add_stamp_with(voxels, fine);
            }
        }

        // Two benches looking at the tree from the park's far side.
        for k in [-1.0f32, 1.0] {
            let p = tree + Vec2::from_angle(u.to_angle() + k * 0.5) * (tr + 5.0);
            self.bench(net, fin, &mut park, p, (p - tree).normalize_or(u), seed ^ 0xbe0c ^ k.to_bits());
        }
        self.layout.park = Some(park);
    }

    /// A bench at `p`, its length across the way `out` (from what it looks at), where its
    /// ground is free; held with the props so no way is left with one on it.
    fn bench(&mut self, net: &mut Network, fin: &Sampler, park: &mut Park, p: Vec2, out: Vec2, seed: u32) {
        let c = p.floor().as_ivec2();
        let turns = if out.x.abs() > out.y.abs() { 1 } else { 0 };
        let y = fin.column(c.x, c.y).top() + 1;
        let placed = props::build(Prop::Bench, &self.pal, IVec3::new(c.x, y, c.y), turns, false, seed);
        let foot = placed.footprint();
        let (at, r) = (park.at, park.radius);
        let free = foot.iter().all(|&q| self.grid.get(q) & (WAY | PLOT | THING | GREEN) == 0 && (q.as_vec2() + 0.5).distance(at) < r - 1.0 && (fin.column(q.x, q.y).top() + 1 - y).abs() <= 1);
        if !free {
            return;
        }
        for &q in &foot {
            self.grid.mark(q, THING);
        }
        net.held_props.push((placed.voxels, placed.fine, foot));
        park.benches.push(IVec3::new(c.x, y, c.y));
    }
}
