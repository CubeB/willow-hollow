//! The granary, kept dry and out of the reach of rats in each region's own way:
//!
//! - temperate country sets a weatherboarded store on staddle stones, mushrooms of stone
//!   whose caps no rat climbs past;
//! - the Alps build a log raccard on the same, under a shingle roof weighted with stones;
//! - the north raises a log store high on posts, its upper walls jutting out;
//! - the uplands lift a stone store on a plinth, up steps of stone;
//! - the desert builds a tall round tower of mud brick under a cone of thatch, the ends of
//!   the beams that tie it standing out of its walls;
//! - the marsh and the rainforest stand theirs high on stilts under a deep roof, a disc on
//!   each stilt to turn the rats back;
//! - the old forests raise theirs on roots.
//!
//! Each is walked into up its steps: bins of grain along the back, sacks and the measure.

use glam::{IVec2, IVec3};
use voxel::AIR;

use super::{
    Built, Use,
    frame::{Block, FOOT, Frame, Roof, v3},
};
use crate::{
    features::{fittings::Fitting, props::Prop},
    materials as mat,
    settlements::{
        Region,
        kit::{Area, Cover, RoofShape, WallKind, rules::Under},
    },
};

pub(super) fn build(f: Frame) -> (Built, Vec<String>) {
    match f.region() {
        Region::Desert => tower(f),
        _ => raised(f),
    }
}

/// A store raised off the ground, of the region's timber or stone.
fn raised(mut f: Frame) -> (Built, Vec<String>) {
    let region = f.region();
    let (fy, staddles) = match region {
        Region::Temperate | Region::Alpine => (3, true),
        Region::Upland => (2, false),
        Region::Northern => (4, false),
        Region::Wetland | Region::Rainforest => (5, false),
        _ => (3, false),
    };
    match region {
        Region::Temperate => f.look.walls = WallKind::Plank,
        Region::Alpine | Region::Northern => f.look.walls = WallKind::Log,
        Region::Upland => {
            f.look.walls = WallKind::Stone;
            f.look.under = Under::Plinth;
        }
        Region::Wetland | Region::Rainforest => f.look.under = Under::Stilts,
        Region::OldGrowth => f.look.under = Under::Roots,
        Region::Desert => {}
    }
    f.look.floor = crate::settlements::kit::rules::FloorKind::Boards;
    f.look.base = 0;
    if f.look.shape == RoofShape::Flat {
        f.look.shape = RoofShape::Hip;
        f.look.cover = Cover::Thatch;
    }
    let (ix0, ix1, iz0, iz1) = (-4, 4, -3, 3);
    let b = Block::new((ix0 - 1, iz0 - 1), (ix1 + 1, iz1 + 1), fy, fy + 6);
    let feet = b.feet();
    if staddles {
        // The floor on a frame of sills, the sills on staddle stones.
        f.look.under = Under::Stilts;
        for c in b.inner() {
            let v = f.floor_voxel(v3(c, fy));
            f.put(v3(c, fy), v);
        }
        for c in b.ring() {
            f.put(v3(c, fy), f.pal.wood);
        }
        for z in [b.lo.y, 0, b.hi.y] {
            for x in [b.lo.x, -1, 1, b.hi.x] {
                staddle(&mut f, IVec2::new(x, z), fy);
            }
        }
        let start = fy + 1;
        for c in b.ring() {
            for y in start..=b.plate {
                let at = v3(c, y);
                let v = f.wall_voxel(at, &b);
                f.put(at, v);
            }
        }
        f.register(&b);
        // Under the sills, open to the air so nothing nests there.
        for c in b.columns() {
            for y in 0..fy {
                if f.get(v3(c, y)).is_none() {
                    f.put(v3(c, y), AIR);
                }
            }
        }
    } else {
        f.shell(&b);
    }
    let overhang = match region {
        Region::Wetland | Region::Rainforest => f.look.overhang.max(3),
        _ => f.look.overhang,
    };
    let shape = if f.look.shape == RoofShape::Gable && matches!(region, Region::Temperate | Region::Rainforest) { RoofShape::Hip } else { f.look.shape };
    f.roof(&b, &Roof { shape, along_z: true, overhang, pitch: f.look.pitch.max(1.0), cover: f.look.cover });
    // The north's store juts out over its lower walls: a course of logs proud all round,
    // high enough over the platform before the door to pass under.
    if region == Region::Northern {
        for c in b.ring() {
            let n = IVec2::new(if c.x == b.lo.x { -1 } else if c.x == b.hi.x { 1 } else { 0 }, if c.y == b.lo.y { -1 } else if c.y == b.hi.y { 1 } else { 0 });
            f.put(v3(c + n, b.plate), f.pal.wood);
        }
    }
    // Rats turned back by discs on the stilts.
    if matches!(region, Region::Wetland | Region::Rainforest) {
        for z in [b.lo.y, b.hi.y] {
            for x in [b.lo.x, b.hi.x] {
                let c = IVec3::new(x * 4 - 2, (fy - 2) * 4, z * 4 - 2);
                f.bx(c, c + IVec3::new(8, 1, 8), f.pal.plank);
            }
        }
    }
    let dz = -1;
    f.front_door(&b, dz, 2, 5);
    f.door_leaf(&b, dz, 2, 5);
    f.front_steps(&b, dz, 2, 1);
    f.window(IVec2::new(b.lo.x, 0), feet + 2, 1, 1, IVec3::NEG_X);
    // Bins of grain along the back wall, the boards of their fronts, and sacks.
    for z in iz0..=iz1 {
        for x in ix0..ix0 + 2 {
            f.put(IVec3::new(x, feet, z), if (x + z) % 3 == 0 { mat::DRIED_GOLD } else { mat::HAY });
        }
    }
    f.bx(IVec3::new((ix0 + 2) * 4, feet * 4, iz0 * 4), IVec3::new((ix0 + 2) * 4 + 1, feet * 4 + 7, (iz1 + 1) * 4), f.pal.plank);
    for (x, z) in [(1, iz0), (1, iz1 - 1), (ix1 - 1, iz0)] {
        f.prop(Prop::Sack, IVec3::new(x, feet, z), 0, false);
    }
    f.uses.push((Use::Store, IVec3::new(0, fy, 0)));
    f.furniture_spot(IVec3::new(ix0, feet, iz0), IVec3::Z, IVec3::X, iz1 - iz0 + 1, 2);
    f.finish(Area { lo: b.lo, hi: b.hi }, fy)
}

/// A staddle stone at column `c` under a floor at layer `fy` (3 at least): a post of
/// stone, a neck and a cap wider than it, and a block of timber on the cap under the sill.
fn staddle(f: &mut Frame, c: IVec2, fy: i32) {
    for y in -2..=fy - 3 {
        let q = v3(c, y);
        let v = if y < 0 { f.pal.stone_dark } else { f.stonework(q) };
        f.put(q, v);
    }
    let y = (fy - 2) * 4;
    f.bx(IVec3::new(c.x * 4, y, c.y * 4), IVec3::new(c.x * 4 + 4, y + 1, c.y * 4 + 4), f.pal.stone);
    f.bx(IVec3::new(c.x * 4 - 2, y + 1, c.y * 4 - 2), IVec3::new(c.x * 4 + 6, y + 4, c.y * 4 + 6), f.pal.stone);
    f.put(v3(c, fy - 1), f.pal.wood);
}

/// The desert's granary: a round tower of mud brick under a steep cone of thatch.
fn tower(mut f: Frame) -> (Built, Vec<String>) {
    let r = 5.5_f32;
    let fy = 0;
    let plate = fy + 11;
    let b = Block::new((-5, -5), (5, 5), fy, plate);
    let feet = b.feet();
    let dist = |c: IVec2| (c.as_vec2()).length();
    let wall = |c: IVec2| dist(c) >= r - 1.0 && dist(c) < r;
    let inner = |c: IVec2| dist(c) < r - 1.0;
    for c in b.columns() {
        if wall(c) {
            for y in -FOOT..=plate {
                let q = v3(c, y);
                // A plaster that flakes from the brick under it with age.
                let v = if y <= 0 { f.pal.stone_dark } else if f.patch(q, 2, 0xb41c) < f.spec.age * 0.5 { mat::MUD_BRICK } else { mat::MUD_PLASTER };
                f.put(q, v);
            }
        } else if inner(c) {
            f.put(v3(c, fy), mat::PATH_DIRT);
        }
    }
    // The ends of the beams that tie it, standing out of the wall in rows; none over the
    // way in, where they would meet the head of whoever comes.
    let dz = -1;
    let by_door = |c: IVec2| c.x >= 2 && (c.y - dz).abs() <= 3;
    for y in [4, 8] {
        for c in b.columns() {
            if wall(c) && !by_door(c) && (c.x * 7 + c.y * 3 + y).rem_euclid(4) == 0 {
                let out = (c.as_vec2() / dist(c) * (r + 0.6)).round().as_ivec2();
                if !wall(out) && !inner(out) {
                    f.put(v3(out, y), f.pal.wood);
                }
            }
        }
    }
    let cover = if f.spec.wealth > 0.7 { Cover::Reed } else { Cover::Thatch };
    f.roof(&b, &Roof { shape: RoofShape::Cone, along_z: true, overhang: 1, pitch: 1.4, cover });
    // The way in, and the vents high in the wall.
    for x in 3..=5 {
        for z in dz..dz + 2 {
            f.hollow(IVec3::new(x, feet, z), IVec3::new(x, fy + 5, z));
            if !inner(IVec2::new(x, z)) {
                f.put(IVec3::new(x, fy, z), f.pal.stone_dark);
            }
        }
    }
    f.door = Some(crate::settlements::kit::Door { floor: IVec3::new(5, fy, dz), facing: IVec3::X, width: 2, height: 5 });
    f.clear.push((IVec3::new(3, feet, dz), IVec3::new(8, fy + 5, dz + 1)));
    for (c, n) in [(IVec2::new(-5, 0), IVec3::NEG_X), (IVec2::new(0, 5), IVec3::Z), (IVec2::new(0, -5), IVec3::NEG_Z)] {
        f.window(c, fy + 8, 1, 1, n);
    }
    // Jars of grain round the wall, and sacks.
    f.fit(Fitting::WaterJar, IVec3::new(-3, feet, -2), IVec3::X, false);
    f.fit(Fitting::WaterJar, IVec3::new(-2, feet, 1), IVec3::NEG_Z, false);
    f.prop(Prop::Sack, IVec3::new(1, feet, 2), 0, false);
    f.prop(Prop::Sack, IVec3::new(2, feet, -3), 0, false);
    f.uses.push((Use::Store, IVec3::new(1, fy, -1)));
    f.porch(&b, dz, 2);
    let area = Area { lo: b.lo, hi: b.hi };
    f.finish(area, fy)
}
