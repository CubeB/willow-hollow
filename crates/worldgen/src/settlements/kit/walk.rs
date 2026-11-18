//! Whether a home is walked into: from the open ground before its door, in through the
//! doorway, to every bed. The kit makes every home so; a home edited by hand is checked
//! here, and its floor found again from where a person can stand in it.

use std::collections::{HashSet, VecDeque};

use glam::{IVec2, IVec3};
use voxel::AIR;

use super::Building;

/// A person's body in voxels: 0.512 m wide is two columns, 1.76 m tall is five layers.
const BODY: (i32, i32) = (2, 5);
/// How high a person steps up without jumping, and how far they drop without care.
const STEP_UP: i32 = 2;
const DROP: i32 = 3;

/// What walking into a home found.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Walk {
    /// Every voxel inside the walls stood on by someone walking in from outside, no lower
    /// than the door's floor, sorted by layer and then along x and z.
    pub floor: Vec<IVec3>,
    /// Whether anyone gets through the doorway.
    pub door: bool,
    /// The beds (indices into [`super::Inside::beds`]) nobody can get to from the door.
    pub cut_off: Vec<usize>,
    /// What is wrong, in words for the editor and the tests; empty when nothing is.
    pub problems: Vec<String>,
}

impl Walk {
    pub fn ok(&self) -> bool {
        self.problems.is_empty()
    }
}

/// Which voxels about a building are solid, in a box held densely so a walk through it
/// is quick. Below the ground (`y < 0`) is earth wherever the building says nothing.
struct Solid {
    lo: IVec3,
    size: IVec3,
    /// 0 where the building says nothing, 1 air, 2 solid.
    cells: Vec<u8>,
}

impl Solid {
    fn new(b: &Building, lo: IVec3, hi: IVec3) -> Self {
        let size = hi - lo + IVec3::ONE;
        let mut s = Solid { lo, size, cells: vec![0; (size.x * size.y * size.z).max(0) as usize] };
        for &(q, v) in &b.voxels {
            if let Some(i) = s.index(q) {
                s.cells[i] = if v == AIR { 1 } else { 2 };
            }
        }
        s
    }

    fn index(&self, q: IVec3) -> Option<usize> {
        let d = q - self.lo;
        if d.min_element() < 0 || d.x >= self.size.x || d.y >= self.size.y || d.z >= self.size.z {
            return None;
        }
        Some(((d.y * self.size.z + d.z) * self.size.x + d.x) as usize)
    }

    fn at(&self, q: IVec3) -> bool {
        match self.index(q).map(|i| self.cells[i]) {
            Some(2) => true,
            Some(1) => false,
            _ => q.y < 0,
        }
    }
}

/// Walks into `b` (as built, standing on flat ground) from the open ground before its
/// door: through the doorway, round every room a person fits in, up steps of two voxels
/// and down drops of three.
pub fn walk(b: &Building) -> Walk {
    let top = b.voxels.iter().map(|(q, _)| q.y).max().unwrap_or(0);
    let bottom = b.voxels.iter().map(|(q, _)| q.y).min().unwrap_or(0).min(-2);
    let (lo, hi) = (b.footprint.lo - IVec2::splat(4), b.footprint.hi + IVec2::splat(4));
    let solid = Solid::new(b, IVec3::new(lo.x - 2, bottom - DROP - 1, lo.y - 2), IVec3::new(hi.x + 2, top + BODY.1 + 2, hi.y + 2));
    let fits = |p: IVec3| (0..BODY.1).all(|y| (0..BODY.0).all(|z| (0..BODY.0).all(|x| !solid.at(p + IVec3::new(x, y, z)))));
    let stands = |p: IVec3| (0..BODY.0).any(|z| (0..BODY.0).any(|x| solid.at(p + IVec3::new(x, -1, z))));
    let inside = |p: IVec3| p.x >= lo.x && p.z >= lo.y && p.x <= hi.x && p.z <= hi.y && p.y >= -2 && p.y <= top + 2;
    // Out from the door past everything built, on the ground.
    let out = IVec3::new(b.door.facing.x.signum(), 0, if b.door.facing.x != 0 { 0 } else { b.door.facing.z.signum() });
    let covers = |q: IVec3| q.x >= b.footprint.lo.x && q.z >= b.footprint.lo.y && q.x <= b.footprint.hi.x && q.z <= b.footprint.hi.y;
    let mut start = IVec3::new(b.door.floor.x, 0, b.door.floor.z);
    while covers(start) && out != IVec3::ZERO {
        start += out;
    }
    start += out * 2;
    let mut walk = Walk::default();
    if !fits(start) || !stands(start) {
        walk.problems.push("Nobody can stand on the ground before the door: something is built there.".into());
        walk.cut_off = (0..b.inside.beds.len()).collect();
        return walk;
    }
    let mut seen = HashSet::from([start]);
    let mut queue = VecDeque::from([start]);
    // Every solid voxel stood on.
    let mut under = HashSet::new();
    while let Some(p) = queue.pop_front() {
        for x in 0..BODY.0 {
            for z in 0..BODY.0 {
                let f = p + IVec3::new(x, -1, z);
                if solid.at(f) {
                    under.insert(f);
                }
            }
        }
        for d in [IVec3::X, IVec3::NEG_X, IVec3::Z, IVec3::NEG_Z] {
            for dy in -DROP..=STEP_UP {
                let q = p + d + IVec3::Y * dy;
                if !inside(q) || seen.contains(&q) || !fits(q) || !stands(q) {
                    continue;
                }
                // Room overhead to rise before stepping across, or to step across before
                // dropping.
                let between = if dy > 0 { p + IVec3::Y * dy } else { p + d };
                if !fits(between) {
                    continue;
                }
                seen.insert(q);
                queue.push_back(q);
            }
        }
    }
    let walls = b.walls;
    let mut floor: Vec<IVec3> = under
        .iter()
        .copied()
        .filter(|f| f.x >= walls.lo.x && f.z >= walls.lo.y && f.x <= walls.hi.x && f.z <= walls.hi.y && f.y >= b.door.floor.y)
        .collect();
    floor.sort_unstable_by_key(|q| (q.y, q.x, q.z));
    walk.floor = floor;
    walk.door = under.contains(&b.door.floor);
    if !walk.door {
        walk.problems.push("Nobody gets in through the doorway: it is blocked.".into());
    }
    walk.cut_off = b.inside.beds.iter().enumerate().filter(|(_, bed)| !bed.cells().any(|c| under.contains(&(c - IVec3::Y)))).map(|(i, _)| i).collect();
    match (walk.cut_off.len(), b.inside.beds.len()) {
        (0, _) => {}
        (1, 1) => walk.problems.push("The bed cannot be reached from the door.".into()),
        (n, m) if n == m => walk.problems.push(format!("None of the {m} beds can be reached from the door.")),
        (1, m) => walk.problems.push(format!("One of the {m} beds cannot be reached from the door.")),
        (n, m) => walk.problems.push(format!("{n} of the {m} beds cannot be reached from the door.")),
    }
    walk
}
