//! The fine roots of the mangroves (#69): thin roots, a tenth of a metre or two across,
//! dropping from the undersides of the stilts to the mud or the water's bed, as the
//! young roots of a red mangrove do before they thicken into stilts themselves. The
//! stilts and the trunk are the tree's voxels (`trees::mangrove`); these finish them.
//!
//! Each root stays in the one column of voxels under the stilt it drops from, hangs from
//! that stilt's underside and reaches down through nothing but air and water until it
//! stands on the ground, so it can never float: it is decided from that column of voxels
//! alone, which the chunk column holding it has, and so neighbouring columns agree.

use std::collections::HashSet;

use glam::IVec3;
use voxel::{
    AIR, CHUNK, Kind, Materials, Voxel,
    mesh::{CELLS, PlantBox},
};

use super::attach::{ColumnVoxels, Voxels};
use super::{Placement, trees::Species};
use crate::{
    materials as mat,
    noise::{hash3, unit},
};

/// Share of the stilts' undersides over open air or water that drop a fine root.
const DROPS: f32 = 0.3;
/// The longest drop, in voxels: the roots come from the stilts, not the crown.
const LONGEST: i32 = 7;
/// How far above the ground a root is dark with the wet, in voxels, as the stilts are.
const TIDE: i32 = 3;

fn is_stilt(v: Voxel) -> bool {
    matches!(v, mat::BARK_MANGROVE | mat::BARK_MANGROVE_WET)
}

/// Adds the fine roots of the mangroves among `placements` that stand in the chunk
/// column whose voxels `voxels` reads. `taken` holds the voxels other fine pieces are in
/// already, which the roots keep out of.
pub fn grow(placements: &[Placement], voxels: &ColumnVoxels, mats: &Materials, seed: u32, taken: &HashSet<IVec3>, out: &mut Vec<PlantBox>) {
    let IVec3 { x: x0, z: z0, .. } = voxels.corner();
    let (x1, z1, x2, z2) = (x0 + 1, z0 + 1, x0 + CHUNK, z0 + CHUNK);
    // The heights the stilts of the mangroves reaching this column stand between.
    let mut span: Option<(i32, i32)> = None;
    for p in placements.iter().filter(|p| p.species == Species::Mangrove) {
        let reach = 10 * crate::scale::BLOCK;
        if p.pos.x + reach < x1 || p.pos.x - reach > x2 || p.pos.z + reach < z1 || p.pos.z - reach > z2 {
            continue;
        }
        let (lo, hi) = (p.pos.y - 3, p.pos.y + 12);
        span = Some(span.map_or((lo, hi), |(a, b)| (a.min(lo), b.max(hi))));
    }
    let Some((y_lo, y_hi)) = span else { return };
    let open = |v: Voxel| v == AIR || mats.get(v).kind == Kind::Water;
    for x in x1..=x2 {
        for z in z1..=z2 {
            for y in (y_lo..=y_hi).rev() {
                let at = IVec3::new(x, y, z);
                if !voxels.has_height(y) || !voxels.has_height(y - LONGEST - 1) || !is_stilt(voxels.voxel(at)) {
                    continue;
                }
                let h = hash3(seed ^ 0x3e0f_d1, x, y, z);
                if unit(h) >= DROPS {
                    continue;
                }
                // Down through the open to the ground, or no root.
                let mut ground = None;
                for k in 1..=LONGEST + 1 {
                    let q = at - IVec3::Y * k;
                    let v = voxels.voxel(q);
                    if open(v) && !taken.contains(&q) {
                        continue;
                    }
                    if k > 1 && mats.get(v).kind == Kind::Solid && !taken.contains(&q) {
                        ground = Some(q.y);
                    }
                    break;
                }
                let Some(g) = ground else { continue };
                drop_root(IVec3::new(x, g, z), y, h, out);
            }
        }
    }
}

/// A root from the underside of the stilt voxel at height `top` down to the ground whose
/// top voxel is `foot`: two cells across where it leaves the stilt, leaning a cell aside
/// on its way down, a cell across lower down, and splayed a little where it meets the mud.
fn drop_root(foot: IVec3, top: i32, h: u32, out: &mut Vec<PlantBox>) {
    let (x, z) = (foot.x * CELLS, foot.z * CELLS);
    let (bottom, top) = ((foot.y + 1) * CELLS, top * CELLS);
    let pick = |k: u32| (h.rotate_left(k * 7) % (CELLS as u32 - 1)) as i32;
    // Where it leaves the stilt, two cells across, and where it leans to.
    let (ax, az) = (pick(1), pick(2));
    let (bx, bz) = ((ax + pick(3) % 2).min(CELLS - 2), az);
    let mid = bottom + (top - bottom) / 2;
    let lean = mid + (top - mid) / 2;
    let wet = |y: i32| if y < bottom + TIDE * CELLS { mat::BARK_MANGROVE_WET } else { mat::BARK_MANGROVE };
    let mut add = |lo: [i32; 3], hi: [i32; 3], voxel: Voxel| {
        if (0..3).all(|k| lo[k] < hi[k]) {
            out.push(PlantBox { lo, hi, voxel, sway_lo: false, sway_hi: false, thin: 0, season: 0, flap: 0 });
        }
    };
    // Each box stays inside one voxel, as every fine piece does.
    let mut runs = |x0: i32, z0: i32, w: i32, from: i32, to: i32| {
        let mut y = from;
        while y < to {
            let next = ((y.div_euclid(CELLS) + 1) * CELLS).min(to);
            add([x + x0, y, z + z0], [x + x0 + w, next, z + z0 + w], wet(y));
            y = next;
        }
    };
    // Thick from the stilt, then leaning over by a cell where the two overlap, then thin.
    runs(ax, az, 2, lean, top);
    runs(bx, bz, 2, mid, lean);
    runs(bx, bz, 1, bottom + 1, mid);
    // The foot, splayed a cell each way on the mud.
    let (fx0, fz0) = ((bx - 1).max(0), (bz - 1).max(0));
    add([x + fx0, bottom, z + fz0], [x + (bx + 2).min(CELLS), bottom + 1, z + (bz + 2).min(CELLS)], wet(bottom));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::World;

    #[test]
    fn the_roots_of_a_mangrove_coast_stand_on_the_ground_under_their_stilts() {
        // A sheltered tropical bay of world 1 with mangroves standing in its shallows.
        let w = crate::world::GameWorld::new(1);
        let mats = mat::build();
        let (mut roots, mut columns) = (0, 0);
        for (cx, cz) in [(491, -2826), (492, -2826), (491, -2825), (490, -2826)] {
            let out = w.generate_column(cx, cz);
            let voxels = ColumnVoxels::new(&out.chunks, cx * CHUNK - 1, cz * CHUNK - 1);
            let mine: Vec<&PlantBox> = out.plants.iter().filter(|b| is_stilt(b.voxel)).collect();
            columns += !mine.is_empty() as i32;
            // Stacked up each voxel column they are in, each from the ground to a stilt.
            let mut stacks: std::collections::BTreeMap<(i32, i32), Vec<&PlantBox>> = Default::default();
            for b in &mine {
                let at = IVec3::from_array(b.lo).div_euclid(IVec3::splat(CELLS));
                assert!(open_or_water(voxels.voxel(at), &mats), "{b:?} is not in air or water");
                stacks.entry((at.x, at.z)).or_default().push(b);
            }
            for ((x, z), mut boxes) in stacks {
                boxes.sort_by_key(|b| b.lo[1]);
                let mut start = 0;
                for i in 1..=boxes.len() {
                    // A root ends where the next box does not carry straight on from it.
                    if i < boxes.len() && boxes[i].lo[1] <= boxes[i - 1].hi[1] {
                        continue;
                    }
                    let (low, high) = (boxes[start], boxes[i - 1]);
                    assert_eq!(low.lo[1] % CELLS, 0, "a root at {x},{z} stops short of the ground");
                    assert_eq!(high.hi[1] % CELLS, 0, "a root at {x},{z} stops short of its stilt");
                    let below = voxels.voxel(IVec3::new(x, low.lo[1] / CELLS - 1, z));
                    let above = voxels.voxel(IVec3::new(x, high.hi[1] / CELLS, z));
                    assert_eq!(mats.get(below).kind, Kind::Solid, "a root at {x},{z} does not stand on anything");
                    assert!(is_stilt(above), "a root at {x},{z} hangs from {above}, not a stilt");
                    roots += 1;
                    start = i;
                }
            }
        }
        assert!(roots > 3 && columns > 0, "found only {roots} fine roots");
    }

    fn open_or_water(v: Voxel, mats: &Materials) -> bool {
        v == AIR || mats.get(v).kind == Kind::Water
    }

    #[test]
    fn a_root_reaches_from_its_stilt_to_the_ground_in_one_piece() {
        for h in [0u32, 7, 0x9e37_79b9, 12345, u32::MAX] {
            for (foot_y, top) in [(-1, 2), (0, 6), (3, 10)] {
                let mut out = Vec::new();
                drop_root(IVec3::new(-3, foot_y, 5), top, h, &mut out);
                assert!(!out.is_empty());
                // Inside the one column, each box inside one voxel.
                for b in &out {
                    let (lo, hi) = (IVec3::from_array(b.lo).div_euclid(IVec3::splat(CELLS)), (IVec3::from_array(b.hi) - 1).div_euclid(IVec3::splat(CELLS)));
                    assert_eq!(lo, hi, "{b:?} spans voxels");
                    assert_eq!((lo.x, lo.z), (-3, 5));
                }
                // Every cell height from the ground's top to the stilt's underside is filled,
                // and each box overlaps the one above it in plan, so nothing hangs apart.
                let (bottom, roof) = ((foot_y + 1) * CELLS, top * CELLS);
                for y in bottom..roof {
                    let here: Vec<_> = out.iter().filter(|b| b.lo[1] <= y && y < b.hi[1]).collect();
                    assert!(!here.is_empty(), "nothing at cell height {y}");
                    if y + 1 < roof {
                        let above: Vec<_> = out.iter().filter(|b| b.lo[1] <= y + 1 && y + 1 < b.hi[1]).collect();
                        let meets = here.iter().any(|a| above.iter().any(|b| (0..3).step_by(2).all(|k| a.lo[k] < b.hi[k] && b.lo[k] < a.hi[k])));
                        assert!(meets, "the root breaks between cells {y} and {}", y + 1);
                    }
                }
            }
        }
    }
}
