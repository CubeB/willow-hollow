//! The sawmill: a long shed open to the lane along its front, where logs come in and
//! boards go out. On a stream a water wheel turns at its side, fed over the top by a
//! wooden flume, and drives a frame saw inside through its axle; without one, two sawyers
//! work a long saw over a pit, one above the log and one below it. Logs wait on a deck
//! along the back wall and sawn boards in heaps.
//!
//! The wheel turns no faster than the world does: it stands still, as a mill does between
//! loads.

use glam::{IVec2, IVec3};
use voxel::AIR;

use super::{
    Built, Use, Wheel,
    frame::{Block, Frame, Roof, v3},
};
use crate::{
    features::{fittings::Fitting, props::Prop},
    materials as mat,
    settlements::kit::{Area, RoofShape},
};

/// The wheel's radius to the tips of its paddles, and the height of its hub.
const RADIUS: i32 = 6;
const HUB: i32 = 4;

pub(super) fn build(mut f: Frame) -> (Built, Vec<String>) {
    let (ix0, ix1, iz0, iz1) = (-7, 7, -5, 5);
    let fy = 0;
    let b = Block::new((ix0 - 1, iz0 - 1), (ix1 + 1, iz1 + 1), fy, fy + 7);
    let feet = b.feet();
    f.shell(&b);
    // The front is open between posts every four columns, the plate carried over them.
    for z in iz0..=iz1 {
        if (z - b.lo.y) % 4 != 0 {
            f.hollow(IVec3::new(b.hi.x, feet, z), IVec3::new(b.hi.x, b.plate - 1, z));
        }
    }
    let shape = if f.look.shape == RoofShape::Flat { RoofShape::Flat } else { RoofShape::Gable };
    f.roof(&b, &Roof { shape, along_z: true, overhang: f.look.overhang.min(2), pitch: f.look.pitch, cover: f.look.cover });
    f.front_door(&b, -1, 3, 5);
    // The whole open front is a way in; the door is its middle.
    f.clear.push((IVec3::new(b.hi.x - 2, feet, iz0), IVec3::new(b.hi.x + 3, feet + 4, iz1)));

    let stream = f.spec.stream.map(|s| match (s.x.signum(), s.z.signum()) {
        (-1, 0) => IVec2::NEG_X,
        (0, -1) => IVec2::NEG_Y,
        _ => IVec2::Y,
    });
    // Where the logs wait: along the back wall, unless the wheel is there.
    if stream == Some(IVec2::NEG_X) {
        f.fit(Fitting::LogDeck, IVec3::new(-4, feet, iz0), IVec3::Z, false);
    } else {
        f.fit(Fitting::LogDeck, IVec3::new(ix0, feet, -4), IVec3::X, false);
    }
    match stream {
        Some(side) => wheel(&mut f, &b, side),
        None => saw_pit(&mut f, &b),
    }
    // Sawn boards in a heap toward the front, and a sawhorse, at whichever end the saw
    // and the logs leave free.
    let free = if matches!(stream, Some(IVec2::NEG_Y) | Some(IVec2::NEG_X)) { iz1 - 1 } else { iz0 };
    f.prop(Prop::PlankHeap, IVec3::new(1, feet, free), 0, false);
    f.fit(Fitting::Sawhorse, IVec3::new(5, feet, free.min(iz1 - 2)), IVec3::NEG_X, false);
    f.porch(&b, -1, 3);
    f.finish(Area { lo: b.lo, hi: b.hi }, fy)
}

/// The wheel on the stream's side of the shed, the flume over it, the pit it turns in,
/// and the frame saw inside its wall.
fn wheel(f: &mut Frame, b: &Block, side: IVec2) {
    let feet = b.feet();
    // Out from the wall along `side`, along it by `t`.
    let t = IVec2::new(side.y.abs(), side.x.abs());
    let mid = (b.lo + b.hi) / 2;
    let wall = if side.x < 0 {
        IVec2::new(b.lo.x, mid.y)
    } else if side.y < 0 {
        IVec2::new(mid.x, b.lo.y)
    } else {
        IVec2::new(mid.x, b.hi.y)
    };
    // Clear of the eaves at that end, and two voxels thick.
    let out = f.look.overhang.min(2) + 2;
    let at = |s: i32, u: i32, y: i32| v3(wall + side * s + t * u, y);
    let wood = f.pal.wood;
    let board = f.pal.plank;
    // The pit it turns in, lined with stone, still water in the bottom.
    for s in out - 1..=out + 2 {
        for u in -RADIUS - 1..=RADIUS + 1 {
            let edge = s == out - 1 || s == out + 2 || u.abs() == RADIUS + 1;
            for y in -3..=-1 {
                let q = at(s, u, y);
                f.put(q, if edge { f.stonework(q) } else { AIR });
            }
            f.put(at(s, u, -4), if edge { f.pal.stone_dark } else { mat::WATER_STILL });
            if edge {
                f.put(at(s, u, 0), f.pal.stone_dark);
            }
        }
    }
    // The wheel: a rim of boards, spokes from the hub, paddles all round.
    let r = RADIUS as f32 - 1.0;
    for s in out..=out + 1 {
        for u in -RADIUS..=RADIUS {
            for dy in -RADIUS..=RADIUS {
                let d = ((u * u + dy * dy) as f32).sqrt();
                let angle = (dy as f32).atan2(u as f32);
                let spoke = (0..4).any(|k| {
                    let a = k as f32 * std::f32::consts::FRAC_PI_4 * 2.0 + 0.3;
                    let (c, sn) = (a.cos(), a.sin());
                    // Distance from the line through the hub at angle a.
                    ((u as f32) * sn - (dy as f32) * c).abs() < 0.6
                });
                let rim = d >= r - 0.5 && d < r + 0.5;
                // Paddles stand out past the rim every eighth of the way round.
                let paddle = d >= r + 0.5 && d < r + 1.6 && ((angle / std::f32::consts::FRAC_PI_4).rem_euclid(1.0) < 0.22);
                let y = HUB + dy;
                // Dark with the wet, so it reads against the timbers of the wall behind.
                if rim || paddle {
                    f.put(at(s, u, y), mat::PLANK_DARK);
                } else if spoke && d < r && s == out {
                    f.put(at(s, u, y), board);
                }
            }
        }
    }
    // The hub and the axle through the wall into the shed.
    for s in -3..=out + 2 {
        f.put(at(s, 0, HUB), if s > 0 && s < out { mat::IRON } else { wood });
    }
    f.wheel = Some(Wheel { hub: at(out + 1, 0, HUB), side: v3(side, 0), radius: RADIUS });
    // The flume: a trough on posts bringing water to the top of the wheel from upstream.
    let top = HUB + RADIUS + 1;
    for u in -RADIUS - 6..=0 {
        for s in out..=out + 1 {
            f.put(at(s, u, top), board);
            f.put(at(s, u, top + 1), AIR);
        }
        for s in [out - 1, out + 2] {
            f.put(at(s, u, top + 1), board);
        }
        if u % 4 == 0 && u < -RADIUS {
            for s in [out - 1, out + 2] {
                for y in -1..top {
                    f.put(at(s, u, y), wood);
                }
            }
        }
    }
    // Water in the flume, a thin skin of it in the trough.
    let (a, c) = (at(out, -RADIUS - 6, top + 1) * 4, at(out + 1, 0, top + 1) * 4 + IVec3::new(4, 0, 4));
    f.bx(a.min(c).with_y((top + 1) * 4), a.max(c).with_y((top + 1) * 4 + 1), mat::WATER_STILL);
    // The frame saw inside the wall: two posts either side of the log's path, a beam over
    // them, the sash with its blades between, and the carriage the log rides on.
    let line = 3;
    for s in [-line - 1, -line + 1] {
        for y in feet..b.plate {
            f.put(at(s, 0, y), wood);
        }
    }
    for s in -line - 1..=-line + 1 {
        f.put(at(s, 0, b.plate - 1), wood);
    }
    let cell = |q: IVec3| q * 4;
    let (blade_lo, blade_hi) = (cell(at(-line, 0, feet + 1)) + IVec3::new(1, 0, 1), cell(at(-line, 0, feet + 4)) + IVec3::new(3, 0, 3));
    f.bx(blade_lo.min(blade_hi), blade_lo.max(blade_hi) + IVec3::new(0, 1, 0), mat::IRON);
    let (sash_lo, sash_hi) = (cell(at(-line, 0, feet + 4)), cell(at(-line, 0, feet + 4)) + IVec3::new(4, 2, 4));
    f.bx(sash_lo, sash_hi, wood);
    // The carriage: two rails along `t` through the frame, and a log on them.
    for u in -5..=5 {
        if u == 0 {
            continue;
        }
        let q = cell(at(-line, u, feet));
        let across = IVec3::new(t.y.abs(), 0, t.x.abs());
        f.bx(q, q + IVec3::new(4, 1, 4) - across * 3, board);
        f.bx(q + across * 3, q + IVec3::new(4, 1, 4), board);
    }
    for u in 1..=4 {
        let q = cell(at(-line, u, feet)) + IVec3::new(0, 1, 0);
        let across = IVec3::new(t.y.abs(), 0, t.x.abs());
        f.bx(q + across, q + IVec3::new(4, 4, 4) - across, mat::BARK_OAK);
    }
    f.uses.push((Use::Saw, (at(-line - 3, 2, 0) * IVec3::new(1, 0, 1)).with_y(b.floor)));
}

/// A pit dug in the floor along x, a log over it on bearers and the long saw through it.
fn saw_pit(f: &mut Frame, b: &Block) {
    let feet = b.feet();
    let (x0, x1, z0, z1) = (-4, 3, -1, 0);
    for x in x0 - 1..=x1 + 1 {
        for z in z0 - 1..=z1 + 1 {
            let edge = x < x0 || x > x1 || z < z0 || z > z1;
            for y in -3..=0 {
                if !edge {
                    f.put(IVec3::new(x, y, z), AIR);
                } else if y < 0 {
                    f.put(IVec3::new(x, y, z), f.pal.plank);
                }
            }
            if !edge {
                f.put(IVec3::new(x, -4, z), mat::PATH_DIRT);
            }
        }
    }
    // Two bearers across the pit, and the log along it.
    let wood = f.pal.wood;
    for x in [x0, x1] {
        f.bx(IVec3::new(x * 4 + 1, feet * 4 - 2, (z0 - 1) * 4 + 2), IVec3::new(x * 4 + 3, feet * 4, (z1 + 2) * 4 - 2), wood);
    }
    f.bx(IVec3::new(x0 * 4 - 2, feet * 4, z0 * 4 + 2), IVec3::new((x1 + 1) * 4 + 2, feet * 4 + 5, z1 * 4 + 2), mat::BARK_OAK);
    f.bx(IVec3::new(x0 * 4 - 2, feet * 4 + 5, z0 * 4 + 3), IVec3::new((x1 + 1) * 4 - 6, feet * 4 + 6, z1 * 4 + 1), mat::PLANK_PALE);
    // The saw, upright through the log, its tiller above and its box below.
    let sx = 0;
    f.bx(IVec3::new(sx * 4 + 1, -3 * 4, z0 * 4 + 3), IVec3::new(sx * 4 + 2, (feet + 3) * 4, z0 * 4 + 5), mat::IRON);
    f.bx(IVec3::new(sx * 4 - 2, (feet + 3) * 4, z0 * 4 + 3), IVec3::new(sx * 4 + 5, (feet + 3) * 4 + 1, z0 * 4 + 5), wood);
    f.bx(IVec3::new(sx * 4 - 1, -3 * 4 - 1, z0 * 4 + 3), IVec3::new(sx * 4 + 4, -3 * 4, z0 * 4 + 5), wood);
    f.uses.push((Use::Saw, IVec3::new(0, b.floor, z1 + 2)));
}
