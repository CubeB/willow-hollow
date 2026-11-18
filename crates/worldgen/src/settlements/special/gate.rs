//! What stands at a town's edges and in its park rather than being gone into (#203): the
//! gate on each main road in, in its country's way, the stone gatehouse in an ancient
//! town's old wall, the beacon at the head of a harbour wall, and the park's pavilion.
//!
//! Each is built as the settlement buildings are, in their model space with the way
//! through or in at +X, and is always open: no gate here has a door to shut. A gate's
//! way through runs along x, `PASSAGE` voxels wide about z = 0 unless the layout asks
//! for a wider one to take its road, and nothing spans it lower than `CLEAR` voxels up,
//! so a handcart passes under with a walker beside it. Their lamps hang lit, glowing at
//! night.

use glam::{IVec2, IVec3};
use voxel::{Voxel, mesh::CELLS};

use super::{
    Built,
    frame::{Block, FOOT, Frame, Roof},
};
use crate::{
    features::{designs::Special, fittings::Fitting, parts, props::Prop},
    materials as mat,
    noise::{hash2, unit},
    settlements::{
        Region,
        kit::{Area, Cover, Door, RoofShape},
    },
};

/// The width of a gate's way through when nothing asks for more: 2.8 m, room for a
/// handcart to pass someone on foot.
pub const PASSAGE: i32 = 7;
/// The least clear height under whatever spans a gate's way: 3.2 m.
pub const CLEAR: i32 = 8;
/// The widest way through a gate is built with, for the busiest road.
pub const WIDEST: i32 = 13;

/// The most a gate's piers reach either side of its way and before and behind its middle,
/// beyond the way itself: what the layout keeps clear for one.
pub fn reach(which: Special) -> (i32, i32) {
    match which {
        Special::Gatehouse => (3, 4),
        _ => (3, 6),
    }
}

/// A gate or gatehouse with a way through `passage` voxels wide (made odd, and kept
/// between [`PASSAGE`] less two and [`WIDEST`]).
pub(super) fn build(mut f: Frame, passage: i32) -> (Built, Vec<String>) {
    let w = passage.clamp(PASSAGE - 2, WIDEST) | 1;
    let h = w / 2;
    let walls = match (f.spec.which, f.region()) {
        (Special::Gatehouse, _) => gatehouse(&mut f, h),
        (_, Region::Desert) => arch(&mut f, h),
        (_, Region::Northern | Region::Alpine | Region::OldGrowth) => logs(&mut f, h),
        _ => timber(&mut f, h),
    };
    // The way through, its ground left as the road has it.
    f.door = Some(Door { floor: IVec3::new(walls.hi.x, -1, 0), facing: IVec3::X, width: w, height: CLEAR });
    f.clear.push((IVec3::new(walls.lo.x - 2, 0, -h), IVec3::new(walls.hi.x + 2, CLEAR - 1, h)));
    f.finish(walls, 0)
}

/// A lit lamp round voxel `l`, hung from the underside of the voxel `drop` above it.
fn lamp(f: &mut Frame, l: IVec3, drop: i32) {
    f.put(l, mat::LANTERN);
    f.fine.extend(parts::hung_lamp(l, drop, mat::CHARCOAL));
}

/// An iron bracket out from a face at column `x` (the face toward +x when `out` is 1,
/// toward -x when -1), at layer `y` over column `z`: what a lamp hangs from.
fn bracket(f: &mut Frame, x: i32, y: i32, z: i32, out: i32) {
    let (a, b) = if out > 0 { (x * CELLS, (x + 1) * CELLS) } else { ((x + 1) * CELLS - CELLS, (x + 1) * CELLS) };
    let zc = z * CELLS + 1;
    f.bx(IVec3::new(a, y * CELLS, zc), IVec3::new(b, y * CELLS + 1, zc + 2), mat::IRON);
    let foot = if out > 0 { a } else { b - 1 };
    f.bx(IVec3::new(foot, y * CELLS - 3, zc), IVec3::new(foot + 1, y * CELLS, zc + 2), mat::IRON);
}

/// The roof a gate's region covers with, for a small pitched roof: a flat roof's mud is
/// no good on one, so it takes the thatch or palm about it.
fn pitched_cover(f: &Frame) -> Cover {
    match f.look.cover {
        Cover::Mud => match f.region() {
            Region::Rainforest => Cover::Palm,
            Region::Wetland => Cover::Reed,
            _ => Cover::Thatch,
        },
        c => c,
    }
}

/// The temperate gate, and the uplands', the wetlands' and the rainforest's: a roofed
/// gateway of timber, two posts either side of the way on stone pads, tie beams across,
/// and a pitched roof of the country's own over them, its gables to the road, a lamp
/// hung from each end of its plates.
fn timber(f: &mut Frame, h: i32) -> Area {
    let side = h + 1;
    // The plate high enough that the eaves, which fall below it as far as they reach
    // out, still clear the way through.
    let (overhang, pitch) = (f.look.overhang.clamp(1, 2), f.look.pitch.clamp(0.8, 1.2));
    let t = CLEAR + (overhang as f32 * pitch).ceil() as i32;
    let (wood, stone, dark) = (f.pal.wood, f.pal.stone, f.pal.stone_dark);
    for z in [-side, side] {
        for x in [-1, 1] {
            for y in -FOOT - 2..t {
                let v = if y < 0 { dark } else if y == 0 { stone } else { wood };
                f.put(IVec3::new(x, y, z), v);
            }
        }
        // A rail between each side's posts, and a plate along their tops past them.
        f.put(IVec3::new(0, 3, z), wood);
        for x in -2..=2 {
            f.put(IVec3::new(x, t, z), wood);
        }
        // A lamp hung from a stub of each plate's end, out beyond the way.
        let out = z + z.signum();
        for x in [-2, 2] {
            f.put(IVec3::new(x, t, out), wood);
            lamp(f, IVec3::new(x, t - 2, out), 2);
        }
    }
    for x in [-1, 1] {
        for z in -side..=side {
            f.put(IVec3::new(x, t, z), wood);
        }
    }
    let b = Block::new((-1, -side), (1, side), -1, t);
    let shape = match f.look.shape {
        RoofShape::Flat | RoofShape::Cone => RoofShape::Gable,
        s => s,
    };
    let cover = pitched_cover(f);
    f.roof(&b, &Roof { shape, along_z: false, overhang, pitch, cover });
    Area { lo: IVec2::new(-1, -side), hi: IVec2::new(1, side) }
}

/// The northern gate, and the mountains' and the old forests': two great logs either
/// side of the way, a heavy lintel across them whose ends are carved into heads, and a
/// deck of boards on it grown over with turf; bands carved round the posts, and lamps
/// hung under the deck.
fn logs(f: &mut Frame, h: i32) -> Area {
    let side = h + 1;
    let t = CLEAR;
    let (wood, dark, accent) = (f.pal.wood, f.pal.stone_dark, f.pal.accent);
    for s in [-1, 1] {
        for dz in 0..2 {
            let z = s * (side + dz);
            for x in 0..2 {
                for y in -FOOT - 2..t {
                    // Carved bands round the posts every few layers, on their faces.
                    let band = y >= 2 && y % 3 == 0 && (x == 1 || dz == 1);
                    let v = if y < 0 { dark } else if band { accent } else { wood };
                    f.put(IVec3::new(x, y, z), v);
                }
            }
        }
    }
    let reach = side + 2;
    for y in [t, t + 1] {
        for z in -reach..=reach {
            for x in 0..2 {
                f.put(IVec3::new(x, y, z), wood);
            }
        }
    }
    // The heads at the lintel's ends, looking out along the road both ways.
    for s in [-1, 1] {
        let z = s * reach;
        f.put(IVec3::new(0, t + 2, z), accent);
        f.put(IVec3::new(1, t + 2, z), accent);
        f.put(IVec3::new(2, t + 1, z), accent);
        f.put(IVec3::new(-1, t + 1, z), accent);
    }
    // The deck, and the turf on it with a low ridge.
    for z in -(side + 1)..=side + 1 {
        for x in -1..=2 {
            f.put(IVec3::new(x, t + 2, z), f.pal.plank);
            f.put(IVec3::new(x, t + 3, z), mat::ROOF_TURF);
        }
        if z.abs() <= side {
            for x in 0..2 {
                f.put(IVec3::new(x, t + 4, z), mat::ROOF_TURF);
            }
        }
    }
    for z in [-(side + 1), side + 1] {
        for x in [-1, 2] {
            lamp(f, IVec3::new(x, t - 1, z), 3);
        }
    }
    Area { lo: IVec2::new(0, -(side + 1)), hi: IVec2::new(1, side + 1) }
}

/// The desert gate: an arch of mud brick between two piers, rendered and whitewashed
/// where the place keeps it so, its opening framed in colour and its top stepped; a
/// cloth awning on poles inside, over a great water jar for those who come in off the
/// road, and lamps hung from brackets on the piers.
fn arch(f: &mut Frame, h: i32) -> Area {
    let (p0, p1) = (h + 1, h + 2);
    let t = CLEAR;
    let top = t + 3;
    let flaked = |f: &Frame, q: IVec3| f.patch(q, 2, 0xf1a4) < f.spec.age * 0.45;
    let render = |f: &Frame, q: IVec3| -> Voxel {
        if f.look.whitewash && !flaked(f, q) {
            mat::WHITEWASH
        } else if f.patch(q, 2, 0xb41c) < f.spec.age * 0.4 {
            mat::MUD_BRICK
        } else {
            mat::MUD_PLASTER
        }
    };
    let trim = if f.spec.seed % 2 == 0 { mat::CLOTH_BLUE } else { mat::PLANK_RED };
    for s in [-1, 1] {
        for z in p0..=p1 {
            for x in -2..=2 {
                for y in -FOOT - 2..=top {
                    let q = IVec3::new(x, y, s * z);
                    let v = if y < 0 { f.pal.stone_dark } else { render(f, q) };
                    f.put(q, v);
                }
            }
        }
    }
    // The arch: lowest at its springing by the piers, highest over the middle.
    let under = |z: i32| t - 1 + (2.0 * (1.0 - (z as f32 / (h + 1) as f32).powi(2)).max(0.0).sqrt()).round() as i32;
    for z in -h..=h {
        let u = under(z);
        for x in -2..=2 {
            for y in u..=top {
                let q = IVec3::new(x, y, z);
                let v = if y == u && x.abs() == 2 { trim } else { render(f, q) };
                f.put(q, v);
            }
        }
    }
    // The opening's frame up the piers' faces either side.
    for s in [-1, 1] {
        for x in [-2, 2] {
            for y in 0..under(s * h) {
                f.put(IVec3::new(x, y, s * p0), trim);
            }
        }
    }
    // A stepped top along the front and back.
    for z in -p1..=p1 {
        if z.rem_euclid(2) == 0 {
            for x in [-2, 2] {
                let q = IVec3::new(x, top + 1, z);
                let v = render(f, q);
                f.put(q, v);
            }
        }
    }
    for x in [-2, 2] {
        let q = IVec3::new(x, top + 2, 0);
        let v = render(f, q);
        f.put(q, v);
    }
    // The awning inside, over the jar by the pier on the +z side.
    let colour = [mat::CLOTH_BLUE, mat::CLOTH_RED, mat::CLOTH_YELLOW][(f.spec.seed % 3) as usize];
    let face = -2 * CELLS;
    let (za, zb) = (p0 * CELLS, (p1 + 3) * CELLS);
    let (high, low) = ((t - 1) * CELLS, (t - 3) * CELLS);
    let out = face - 4 * CELLS;
    f.fine.extend(crate::features::cloth::sheet([IVec3::new(face, high, za), IVec3::new(face, high, zb), IVec3::new(out, low, za), IVec3::new(out, low, zb)], colour));
    for z in [za, zb - 1] {
        f.bx(IVec3::new(out, 0, z), IVec3::new(out + 1, low, z + 1), f.pal.wood);
    }
    f.fit(Fitting::WaterJar, IVec3::new(-5, 0, p0 + 1), IVec3::NEG_X, false);
    for s in [-1, 1] {
        bracket(f, 3, t - 1, s * p1, 1);
        lamp(f, IVec3::new(3, t - 3, s * p1), 2);
    }
    bracket(f, -3, t - 1, -p1, -1);
    lamp(f, IVec3::new(-3, t - 3, -p1), 2);
    Area { lo: IVec2::new(-2, -p1), hi: IVec2::new(2, p1) }
}

/// The gatehouse of an ancient town's old wall: a deep block of the country's stone
/// with a round arch through it, its arch stones and corners darker, a storey of blind
/// stone over the arch with shuttered windows, and a roof of the country's own; lamps
/// on brackets either side of the arch, front and back. Old, it is mossy and some of
/// its top stones are gone.
fn gatehouse(f: &mut Frame, h: i32) -> Area {
    let (p0, p1) = (h + 1, h + 3);
    let t = CLEAR;
    let plate = t + 5;
    let (dark, wood) = (f.pal.stone_dark, f.pal.wood);
    let under = |z: i32| t - 2 + (3.0 * (1.0 - (z as f32 / (h + 1) as f32).powi(2)).max(0.0).sqrt()).round() as i32;
    for z in -p1..=p1 {
        for x in -3..=3 {
            let low = if z.abs() < p0 { under(z) } else { -FOOT - 2 };
            for y in low..=plate {
                let q = IVec3::new(x, y, z);
                let corner = (x.abs() == 3 && z.abs() == p1) || (x.abs() == 3 && z.abs() == p0 && y < under(h));
                let ring = z.abs() < p0 && y <= under(z) + 1 && x.abs() == 3;
                let v = if y < 0 || corner || ring || y == plate { dark } else { f.stonework(q) };
                f.put(q, v);
            }
        }
    }
    // Shuttered windows in the storey over the arch, front and back.
    for x in [-3, 3] {
        for z in [-(h / 2 + 1), h / 2 + 1] {
            for y in t + 2..=t + 3 {
                f.put(IVec3::new(x, y, z), mat::PLANK_DARK);
            }
        }
    }
    let b = Block::new((-3, -p1), (3, p1), -1, plate);
    let shape = match f.look.shape {
        RoofShape::Cone => RoofShape::Hip,
        s => s,
    };
    let cover = if shape == RoofShape::Flat { Cover::Mud } else { pitched_cover(f) };
    if shape == RoofShape::Flat {
        // A plain parapet round a flat roof of beaten earth.
        for z in -p1..=p1 {
            for x in -3..=3 {
                f.put(IVec3::new(x, plate + 1, z), mat::ROOF_MUD);
                if x.abs() == 3 || z.abs() == p1 {
                    let q = IVec3::new(x, plate + 2, z);
                    let v = f.stonework(q);
                    f.put(q, v);
                }
            }
        }
    } else {
        f.roof(&b, &Roof { shape, along_z: true, overhang: f.look.overhang.clamp(1, 2), pitch: f.look.pitch.clamp(0.7, 1.1), cover });
    }
    // Worn with age: top stones fallen from the eaves' course where the place lets it go.
    if f.spec.age > 0.6 {
        for z in -p1..=p1 {
            for x in [-3, 3] {
                let q = IVec3::new(x, plate, z);
                if unit(hash2(f.spec.seed ^ 0x6a7e, x * 31 + z, 1)) < (f.spec.age - 0.6) * 0.8 {
                    f.put(q, voxel::AIR);
                }
            }
        }
    }
    for s in [-1, 1] {
        for (x, out) in [(4, 1), (-4, -1)] {
            bracket(f, x, t - 1, s * (p0 + 1), out);
            lamp(f, IVec3::new(x, t - 3, s * (p0 + 1)), 2);
        }
    }
    // A beam at the arch's springing, inside, where a gate once hung and hangs no more.
    for z in [-p0, p0] {
        f.put(IVec3::new(0, under(h) - 1, z), wood);
    }
    Area { lo: IVec2::new(-3, -p1), hi: IVec2::new(3, p1) }
}

/// The beacon at the head of a harbour wall: a stone pillar on a broad plinth, banded,
/// with a cap and an iron basket on it where a fire burns all night.
pub(super) fn beacon(mut f: Frame) -> (Built, Vec<String>) {
    let (dark, iron) = (f.pal.stone_dark, mat::IRON);
    for z in -2..=2 {
        for x in -2..=2 {
            for y in -FOOT - 2..=0 {
                f.put(IVec3::new(x, y, z), dark);
            }
        }
    }
    for z in -1..=1 {
        for x in -1..=1 {
            for y in 1..=11 {
                let q = IVec3::new(x, y, z);
                let v = if y == 6 || (x.abs() == 1 && z.abs() == 1) { dark } else { f.stonework(q) };
                f.put(q, v);
            }
        }
    }
    for z in -2i32..=2 {
        for x in -2i32..=2 {
            if x.abs() + z.abs() < 4 {
                f.put(IVec3::new(x, 12, z), dark);
            }
        }
    }
    f.fire(IVec3::new(0, 13, 0), true);
    // Flames over the coals, and the basket's bars and hoops round them.
    let c = CELLS;
    f.bx(IVec3::new(1, 14 * c + 2, 1), IVec3::new(3, 15 * c + 2, 3), mat::LANTERN);
    f.bx(IVec3::new(0, 14 * c + 2, 2), IVec3::new(1, 15 * c, 3), mat::EMBER);
    for (x, z) in [(-c + 1, -c + 1), (2 * c - 2, -c + 1), (-c + 1, 2 * c - 2), (2 * c - 2, 2 * c - 2)] {
        f.bx(IVec3::new(x, 13 * c, z), IVec3::new(x + 1, 15 * c + 2, z + 1), iron);
    }
    for y in [13 * c + 1, 15 * c + 1] {
        for (lo, hi) in [((-c + 1, -c + 1), (2 * c - 1, -c + 2)), ((-c + 1, 2 * c - 2), (2 * c - 1, 2 * c - 1)), ((-c + 1, -c + 1), (-c + 2, 2 * c - 1)), ((2 * c - 2, -c + 1), (2 * c - 1, 2 * c - 1))] {
            f.bx(IVec3::new(lo.0, y, lo.1), IVec3::new(hi.0, y + 1, hi.1), iron);
        }
    }
    f.door = Some(Door { floor: IVec3::new(2, 0, 0), facing: IVec3::X, width: 1, height: 1 });
    f.finish(Area { lo: IVec2::new(-2, -2), hi: IVec2::new(2, 2) }, 0)
}

/// The park's pavilion: a raised floor open on every side under a roof on four posts,
/// round in a country whose homes lean round, with benches along two sides and a lamp
/// hung from a beam in the middle. Stood under out of the rain, or the sun.
pub(super) fn pavilion(mut f: Frame) -> (Built, Vec<String>) {
    let r = 4;
    let plate = 8;
    let (wood, stone, dark) = (f.pal.wood, f.pal.stone, f.pal.stone_dark);
    let b = Block::new((-r, -r), (r, r), 0, plate);
    for z in -r..=r {
        for x in -r..=r {
            let q = IVec3::new(x, 0, z);
            let v = if x.abs() == r || z.abs() == r { dark } else if f.patch(q, 2, 0xf1a6) < 0.5 { stone } else { dark };
            f.put(q, v);
            for y in -2..0 {
                f.put(IVec3::new(x, y, z), dark);
            }
        }
    }
    for (x, z) in [(-r, -r), (r, -r), (-r, r), (r, r)] {
        for y in -FOOT..0 {
            f.put(IVec3::new(x, y, z), dark);
        }
        for y in 1..plate {
            f.put(IVec3::new(x, y, z), wood);
        }
    }
    for k in -r..=r {
        for (x, z) in [(k, -r), (k, r), (-r, k), (r, k)] {
            f.put(IVec3::new(x, plate, z), wood);
        }
        f.put(IVec3::new(k, plate, 0), wood);
    }
    let round = !matches!(f.region(), Region::Desert);
    let shape = if round { RoofShape::Cone } else { RoofShape::Flat };
    let cover = if round { pitched_cover(&f) } else { Cover::Mud };
    f.roof(&b, &Roof { shape, along_z: false, overhang: 2, pitch: f.look.pitch.clamp(0.8, 1.1), cover });
    lamp(&mut f, IVec3::new(0, plate - 2, 0), 2);
    // Benches along the sides away from the ways in.
    for (at, turns) in [(IVec3::new(-3, 1, -3), 0), (IVec3::new(1, 1, -3), 0), (IVec3::new(3, 1, 3), 2), (IVec3::new(-1, 1, 3), 2)] {
        f.prop(Prop::Bench, at, turns, false);
    }
    f.door = Some(Door { floor: IVec3::new(r, 0, 0), facing: IVec3::X, width: 3, height: plate - 1 });
    f.clear.push((IVec3::new(-r + 1, 1, -1), IVec3::new(r + 2, 5, 1)));
    f.finish(Area { lo: b.lo, hi: b.hi }, 0)
}
