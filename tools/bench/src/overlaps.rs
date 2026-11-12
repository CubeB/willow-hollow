//! Counts plant faces that lie in the same plane as another face turned the same way
//! and overlap it. The two are drawn at the same depth, so which of them shows is
//! settled only by the hair's breadth each colour is drawn nearer than the last (see
//! `vs` in terrain.wgsl): few is good.

use std::collections::HashMap;

use voxel::{
    CHUNK, Kind, Materials, Voxel,
    mesh::{CELLS, PlantBox},
};
use worldgen::ColumnOutput;

/// A rectangle in a plane, in half cells: low and high on one axis, then on the other.
type Rect = [i32; 4];

/// Face `f` of a box as it is drawn: its plane, and its rectangle on the other two axes.
fn face(b: &PlantBox, f: usize) -> (i32, Rect) {
    let axis = f / 2;
    let inset = |a: usize, hi: bool| (b.thin >> (2 * a + !hi as usize) & 1) as i32;
    let lo = |a: usize| 2 * b.lo[a] + inset(a, false);
    let hi = |a: usize| 2 * b.hi[a] - inset(a, true);
    let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
    (if f.is_multiple_of(2) { hi(axis) } else { lo(axis) }, [lo(u), hi(u), lo(v), hi(v)])
}

#[derive(Default)]
pub struct Tally {
    /// Pairs of overlapping faces, by what the two boxes are made of.
    pairs: HashMap<(Voxel, Voxel), u64>,
    /// Faces flush with the same face of a voxel, by what the box and the voxel are.
    on_voxels: HashMap<(Voxel, Voxel), u64>,
    faces: u64,
    /// Faces wholly against opaque voxels.
    hidden: u64,
}

/// Adds the plants of the column `out`, which is chunk column `(cx, cz)`.
pub fn count(out: &ColumnOutput, mats: &Materials, cx: i32, cz: i32, t: &mut Tally) {
    let voxel_at = |x: i32, y: i32, z: i32| -> Voxel {
        let (lx, lz) = (x - cx * CHUNK, z - cz * CHUNK);
        if lx < -1 || lz < -1 || lx > CHUNK || lz > CHUNK {
            return voxel::AIR;
        }
        let cy = y.div_euclid(CHUNK);
        match out.chunks.iter().find(|(c, _)| *c == cy) {
            Some((_, chunk)) => chunk.get((lx + 1) as usize, y.rem_euclid(CHUNK) as usize + 1, (lz + 1) as usize),
            None => voxel::AIR,
        }
    };
    let opaque = |v: Voxel| mats.get(v).kind.is_opaque();
    let mut planes: HashMap<(usize, i32), Vec<(Rect, usize)>> = HashMap::new();
    let step = 2 * CELLS;
    for (i, b) in out.plants.iter().enumerate() {
        for f in 0..6 {
            let (plane, rect) = face(b, f);
            planes.entry((f, plane)).or_default().push((rect, i));
            t.faces += 1;
            // The voxels just outside the face and, where it stands on a voxel boundary,
            // those just inside: a voxel with a face of its own there.
            let axis = f / 2;
            let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
            let outside = if f.is_multiple_of(2) { plane.div_euclid(step) } else { (plane - 1).div_euclid(step) };
            let (mut all_hidden, mut flush) = (true, None);
            for uu in rect[0].div_euclid(step)..=(rect[1] - 1).div_euclid(step) {
                for vv in rect[2].div_euclid(step)..=(rect[3] - 1).div_euclid(step) {
                    let mut p = [0i32; 3];
                    (p[u], p[v], p[axis]) = (uu, vv, outside);
                    let beyond = voxel_at(p[0], p[1], p[2]);
                    all_hidden &= opaque(beyond);
                    if plane.rem_euclid(step) == 0 {
                        p[axis] = if f.is_multiple_of(2) { outside - 1 } else { outside + 1 };
                        let within = voxel_at(p[0], p[1], p[2]);
                        if opaque(within) && !opaque(beyond) {
                            flush = Some(within);
                        }
                    }
                }
            }
            if all_hidden {
                t.hidden += 1;
            } else if let Some(w) = flush {
                *t.on_voxels.entry((b.voxel, w)).or_default() += 1;
            }
        }
    }
    for list in planes.values() {
        for (k, (a, i)) in list.iter().enumerate() {
            for (b, j) in &list[k + 1..] {
                if a[1].min(b[1]) > a[0].max(b[0]) && a[3].min(b[3]) > a[2].max(b[2]) {
                    let (p, q) = (out.plants[*i].voxel, out.plants[*j].voxel);
                    *t.pairs.entry((p.min(q), p.max(q))).or_default() += 1;
                }
            }
        }
    }
}

pub fn report(t: &Tally, mats: &Materials) {
    let name = |v: Voxel| {
        let m = mats.get(v);
        let kind = match m.kind {
            Kind::Air => "air",
            Kind::Solid => "solid",
            Kind::Water => "water",
            Kind::Foliage => "foliage",
        };
        format!("#{v} {:02x}{:02x}{:02x} {kind}, plant kind {}", m.color[0], m.color[1], m.color[2], m.plant)
    };
    println!("  {} plant faces, {} of them wholly against opaque voxels", t.faces, t.hidden);
    let mut pairs: Vec<_> = t.pairs.iter().collect();
    pairs.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
    for ((a, b), n) in pairs.iter().take(12) {
        println!("  {n:7} pairs in one plane: {} and {}", name(*a), name(*b));
    }
    let mut on: Vec<_> = t.on_voxels.iter().collect();
    on.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
    for ((a, b), n) in on.iter().take(8) {
        println!("  {n:7} faces flush with a voxel's: {} on {}", name(*a), name(*b));
    }
}
