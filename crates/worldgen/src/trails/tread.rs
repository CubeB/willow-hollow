//! What a trail's surface is made of: whatever feet would wear the ground beneath it
//! down to, and how much of that shows, by how much the trail is used.
//!
//! The ground comes from what the trail was laid over (see [`Worn`]): packed earth on
//! soil, sand on beaches and dunes, loose gravel on scree, bare rock with pale worn
//! scratches on stone, duckboards over bog, leaf litter in broadleaf woods and needles
//! under conifers, packed snow in the cold. Traffic decides how it wears: a busy track is
//! a continuous firm strip, sometimes with grass down the middle between its ruts; a
//! faint trace is broken by the ground cover creeping back in, and goes to moss in the
//! woods. Where a used path climbs steeply, each step of the voxel ground is edged with
//! a log or a stone, so it reads as a flight of rough steps.
//!
//! Everything here is decided by the column, its neighbours and the world position, so
//! chunks agree at their borders.

use voxel::{
    Voxel,
    mesh::{CELLS, PlantBox},
};

use super::build::{TrailClass, class_of};
use crate::{
    biome::Biome,
    materials as mat,
    noise::{hash2, perlin2},
    scale::{m, m64},
    surface::{Column, Cover, Worn},
};

/// The ground a trail is worn into.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Tread {
    /// Packed earth.
    Soil,
    /// Trodden sand: beaches, dunes and desert.
    Sand,
    /// Loose stone over scree, and the stony ground of the tundra.
    Gravel,
    /// Bare rock, scratched pale where feet go.
    Rock,
    /// Bog, marsh and mangrove: duckboards on a used path, mud on a faint one.
    Bog,
    /// Leaf litter in a broadleaf wood.
    Litter,
    /// Fallen needles under conifers.
    Needles,
    /// Packed snow.
    Snow,
    /// Volcanic ash.
    Ash,
}

/// Steepness, in voxels of rise per voxel, from which a used trail climbs by steps.
pub const STEP_GRADE: f32 = 0.24;

fn smoothstep(lo: f32, hi: f32, x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The ground under a trail column. `patch` is smooth noise over several voxels, so
/// rock breaks through in patches rather than speckle.
pub fn tread(col: &Column, w: &Worn, patch: f32) -> Tread {
    use Biome::*;
    match w.under {
        Cover::Beach => return if col.biome == Volcanic { Tread::Ash } else { Tread::Sand },
        Cover::Scree => return Tread::Gravel,
        Cover::Cliff => return Tread::Rock,
        _ => {}
    }
    // Rock shows through where the land was steep, the more of it the steeper.
    if w.slope > 0.5 + patch * 0.3 {
        return Tread::Rock;
    }
    match col.biome {
        // High bare country: rock and the stones broken off it.
        Alpine => return if patch < 0.55 { Tread::Rock } else { Tread::Gravel },
        Volcanic => return Tread::Ash,
        Desert | DuneSea | RedRock | Ocean => return Tread::Sand,
        _ => {}
    }
    // Where the land lies under snow, so does the path.
    if col.temperature < 0.10 || mat::ground_cover(col.biome) == mat::SNOW {
        return Tread::Snow;
    }
    match col.biome {
        Bog | Marsh | Mangrove => Tread::Bog,
        Tundra => Tread::Gravel,
        OldGrowth | Rainforest | Deciduous | Woodland | BirchGrove => Tread::Litter,
        Taiga | SnowyPine => Tread::Needles,
        _ => Tread::Soil,
    }
}

/// The surface voxel of a trail column and the layer under it. `roll` is per-voxel
/// noise and `patch` smooth noise over several voxels (as for any other ground).
pub fn surface(col: &Column, roll: f32, patch: f32) -> (Voxel, Voxel) {
    let Some(w) = col.worn else { return (mat::PATH_DIRT, mat::DIRT) };
    let cover = mat::ground_cover(col.biome);
    let class = class_of(w.half_width);
    // 0 on a busy track, 1 on the faintest trace.
    let faint = 1.0 - smoothstep(m(0.3), m(0.95), w.half_width);
    // 0 on the centreline, 1 at the worn edge (a little beyond it, where it is ragged).
    // A path a voxel or two wide is all edge, and frays enough already.
    let edge = if w.half_width >= 1.5 { w.offset / w.half_width } else { 0.0 };
    // The ground cover creeps back in from the edges, and across a faint path.
    let encroached = roll * 0.7 + patch * 0.3 < faint * 0.2 + (edge - 0.75).max(0.0) * 0.8;
    let mix = |main: Voxel, other: Voxel, share: f32| if roll < share { other } else { main };
    // Mud where the path comes down to a stream.
    let muddy = col.river < m(2.0) && roll < 0.55;
    let ground = tread(col, &w, patch);
    match ground {
        Tread::Soil => {
            let busy = matches!(class, TrailClass::Track | TrailClass::Road);
            let top = if muddy {
                mat::MUD
            } else if encroached || (busy && w.offset < m(0.3) && patch > 0.45) {
                // A track's two ruts with grass down the middle, in places.
                cover
            } else {
                mix(mat::PATH_DIRT, mat::PATH_GRAVEL, 0.12 + 0.18 * (1.0 - faint))
            };
            (top, mat::DIRT)
        }
        Tread::Sand => {
            let loose = match col.biome {
                Biome::RedRock => mat::RED_SAND,
                Biome::Desert | Biome::DuneSea => mat::DESERT_SAND,
                _ => mat::SAND,
            };
            let top = if encroached { loose } else { mix(mat::PATH_SAND, loose, 0.25) };
            (top, loose)
        }
        Tread::Gravel => {
            let top = if encroached && w.under != Cover::Scree {
                cover
            } else if roll < 0.25 {
                mat::SCREE
            } else if roll < 0.4 {
                mat::GRAVEL
            } else {
                mat::PATH_GRAVEL
            };
            (top, mat::GRAVEL)
        }
        Tread::Rock => {
            let (light, dark) = mat::bedrock(col.biome);
            // Feet scratch the rock paler along the way; a faint trace barely marks it.
            let worn = match light {
                mat::SANDSTONE => mat::PATH_SAND,
                mat::RED_ROCK => mat::RED_SAND,
                mat::BASALT => mat::ROCK_DARK,
                _ => mat::ROCK_PALE,
            };
            let top = if roll < 0.35 * (1.0 - faint * 0.6) {
                worn
            } else if roll > 0.85 {
                dark
            } else {
                light
            };
            (top, light)
        }
        // Trodden mud; on a used path the duckboards lie over it (see [`dress`]).
        Tread::Bog => (if encroached { cover } else { mix(mat::MUD, mat::BOG_MOSS, 0.3) }, mat::MUD),
        Tread::Litter | Tread::Needles => {
            let litter = if ground == Tread::Litter { mat::PATH_LITTER } else { mat::PATH_NEEDLES };
            let top = if muddy {
                mat::MUD
            } else if encroached {
                cover
            } else if patch > 0.6 && roll < faint * 0.7 {
                // Paths hardly walked go to moss.
                mat::MOSS
            } else {
                mix(litter, mat::PATH_DARK, 0.35)
            };
            (top, mat::DIRT)
        }
        Tread::Snow => {
            let top = if encroached { mat::SNOW } else { mix(mat::PATH_SNOW, mat::PATH_GRAVEL, 0.1 * (1.0 - faint)) };
            (top, mat::STONE)
        }
        Tread::Ash => (if encroached { cover } else { mix(mat::BLACK_SAND, mat::BASALT, 0.2) }, mat::BASALT),
    }
}

/// The voxel a trail column shows from far off: its commonest surface, without speckle.
pub fn far(col: &Column) -> Voxel {
    surface(col, 0.99, 0.0).0
}

/// What edges a step of a steep trail.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Step {
    /// A log laid across the lip of the step, as fine parts on the column.
    Log(Voxel),
    /// The step's edge is a block of stone, in place of the column's top voxel.
    Stone(Voxel),
}

/// Whether the ground under a used trail is the kind that gets steps, and which: a log
/// or a stone. `seed` and the world position choose between them where either would do,
/// over stretches of tens of metres so a flight is all one kind.
pub fn step(col: &Column, w: &Worn, patch: f32, seed: u32, wx: i32, wz: i32) -> Option<Step> {
    if class_of(w.half_width) == TrailClass::Trace {
        return None;
    }
    let ground = tread(col, w, patch);
    let log = Step::Log(if ground == Tread::Needles { mat::BARK_PINE } else { mat::BARK_OAK });
    // Stone steps are of the rock the path climbs over.
    let stone = Step::Stone(mat::masonry(crate::surface::local_rock(seed, col.biome, wx, col.top(), wz)).0);
    Some(match ground {
        Tread::Bog | Tread::Snow | Tread::Ash => return None,
        Tread::Rock | Tread::Gravel | Tread::Sand => stone,
        Tread::Litter | Tread::Needles => log,
        Tread::Soil => {
            let n = perlin2(seed ^ 0x57e9, wx as f64 / m64(24.0), wz as f64 / m64(24.0));
            if n < 0.0 { log } else { stone }
        }
    })
}

/// A fixed fine part, in cells.
fn part(lo: [i32; 3], hi: [i32; 3], voxel: Voxel) -> PlantBox {
    PlantBox { lo, hi, voxel, sway_lo: false, sway_hi: false, thin: 0 }
}

/// Where a step is and how it lies: what edges it, which way is down from it (a unit
/// step in x or z), and whether the log ends on either side across the way (toward
/// lower x or z, then higher), where it shows its cut end.
pub type StepAt = (Step, (i32, i32), [bool; 2]);

/// The fine parts a trail column at world `(x, z)` carries on top: the duckboards of a
/// path over bog, and the log of a step. Like the plants, they stay inside the column.
pub fn dress(col: &Column, step: Option<StepAt>, x: i32, z: i32, seed: u32, out: &mut Vec<PlantBox>) {
    let Some(w) = col.worn else { return };
    let y = (col.top() + 1) * CELLS;
    let (x0, z0) = (x * CELLS, z * CELLS);
    if let Some((Step::Log(v), (dx, dz), open)) = step {
        // A log 0.2 m thick along the lip of the step, across the column, its cut ends
        // pale where it stops.
        let down = if dx != 0 { dx } else { dz };
        let along = if down > 0 { (CELLS - 2, CELLS) } else { (0, 2) };
        let lo = if open[0] { 1 } else { 0 };
        let hi = if open[1] { CELLS - 1 } else { CELLS };
        let mut put = |a: (i32, i32), c: (i32, i32), v: Voxel| {
            let (xs, zs) = if dx != 0 { (a, c) } else { (c, a) };
            out.push(part([x0 + xs.0, y, z0 + zs.0], [x0 + xs.1, y + 2, z0 + zs.1], v));
        };
        put(along, (lo, hi), v);
        if open[0] {
            put(along, (0, 1), mat::PLANK_PALE);
        }
        if open[1] {
            put(along, (CELLS - 1, CELLS), mat::PLANK_PALE);
        }
        return;
    }
    if tread(col, &w, crate::surface::ground_patch(seed, x as f64, z as f64)) != Tread::Bog
        || class_of(w.half_width) == TrailClass::Trace
        || w.offset > w.half_width - 0.3
    {
        return;
    }
    // Duckboards: boards 0.3 m wide laid across the way with a gap of 0.1 m, so each
    // voxel along the trail holds one board and its neighbours across carry it on.
    let along_x = w.dir.x.abs() >= w.dir.y.abs();
    let board = if along_x { x } else { z };
    let h = hash2(seed ^ 0xd0c6, board, if along_x { z >> 4 } else { x >> 4 });
    // Now and then one is missing.
    if h % 13 == 0 {
        return;
    }
    let v = if h % 5 == 0 { mat::PLANK } else { mat::PLANK_DARK };
    let (lo, hi) = if along_x {
        ([x0, y, z0], [x0 + 3, y + 1, z0 + CELLS])
    } else {
        ([x0, y, z0], [x0 + CELLS, y + 1, z0 + 3])
    };
    out.push(part(lo, hi, v));
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use voxel::PAD;

    use super::*;

    fn trail(biome: Biome, under: Cover, slope: f32, half_width: f32) -> Column {
        let worn = Worn { under, slope, half_width, offset: 0.4, dir: glam::Vec2::X, point: 7 };
        Column { ground: 40.0, water: None, cover: Cover::Trail, biome, temperature: 0.5, river: f32::MAX, clear: -1.0, worn: Some(worn) }
    }

    /// Every surface voxel a trail column shows over many rolls of the noise.
    fn surfaces(col: &Column) -> HashSet<Voxel> {
        let mut out = HashSet::new();
        for k in 0..200 {
            let roll = k as f32 / 200.0;
            for patch in [0.1, 0.5, 0.9] {
                out.insert(surface(col, roll, patch).0);
            }
        }
        out
    }

    fn tread_of(col: &Column) -> Tread {
        tread(col, &col.worn.unwrap(), 0.5)
    }

    #[test]
    fn the_ground_beneath_decides_the_tread() {
        let track = m(0.95);
        let land = |b| tread_of(&trail(b, Cover::Land, 0.1, track));
        assert_eq!(land(Biome::Grassland), Tread::Soil);
        assert_eq!(land(Biome::Deciduous), Tread::Litter);
        assert_eq!(land(Biome::Taiga), Tread::Needles);
        assert_eq!(land(Biome::Bog), Tread::Bog);
        assert_eq!(land(Biome::DuneSea), Tread::Sand);
        assert_eq!(land(Biome::Volcanic), Tread::Ash);
        // The same biome, different ground.
        let on = |under, slope| tread_of(&trail(Biome::Grassland, under, slope, track));
        assert_eq!(on(Cover::Beach, 0.1), Tread::Sand);
        assert_eq!(on(Cover::Scree, 0.1), Tread::Gravel);
        assert_eq!(on(Cover::Cliff, 0.1), Tread::Rock);
        assert_eq!(on(Cover::Land, 1.0), Tread::Rock, "steep land is bare rock");
        let mut cold = trail(Biome::Grassland, Cover::Land, 0.1, track);
        cold.temperature = 0.05;
        assert_eq!(tread_of(&cold), Tread::Snow);
    }

    #[test]
    fn each_tread_wears_to_its_own_surface() {
        let track = m(0.95);
        let earth = surfaces(&trail(Biome::Grassland, Cover::Land, 0.1, track));
        assert!(earth.contains(&mat::PATH_DIRT));
        let litter = surfaces(&trail(Biome::Deciduous, Cover::Land, 0.1, track));
        assert!(litter.contains(&mat::PATH_LITTER) && !litter.contains(&mat::PATH_DIRT));
        let needles = surfaces(&trail(Biome::Taiga, Cover::Land, 0.1, track));
        assert!(needles.contains(&mat::PATH_NEEDLES));
        let sand = surfaces(&trail(Biome::Grassland, Cover::Beach, 0.1, track));
        assert!(sand.contains(&mat::PATH_SAND) && !sand.contains(&mat::PATH_DIRT));
        let scree = surfaces(&trail(Biome::Grassland, Cover::Scree, 0.1, track));
        assert!(scree.contains(&mat::PATH_GRAVEL) && scree.contains(&mat::SCREE));
        let rock = surfaces(&trail(Biome::Grassland, Cover::Cliff, 0.1, track));
        assert!(rock.contains(&mat::STONE) && rock.contains(&mat::ROCK_PALE), "rock scratched pale: {rock:?}");
        // Over bog, mud and moss underfoot.
        let bog = surfaces(&trail(Biome::Bog, Cover::Land, 0.0, track));
        assert!(bog.contains(&mat::MUD) && !bog.contains(&mat::PATH_DIRT));
    }

    #[test]
    fn duckboards_cross_the_bog_on_used_paths() {
        // A path running along X: boards across it, one to each voxel along the way with
        // a gap between, each one carried on unbroken by the columns beside it.
        let boards = |half_width: f32, offset: f32, dir: glam::Vec2| {
            let mut col = trail(Biome::Bog, Cover::Land, 0.0, half_width);
            let w = col.worn.as_mut().unwrap();
            (w.offset, w.dir) = (offset, dir);
            let mut out = Vec::new();
            for x in 0..40 {
                for z in 0..3 {
                    dress(&col, None, x, z, 1, &mut out);
                }
            }
            out
        };
        let along = boards(m(0.95), 0.5, glam::Vec2::X);
        assert!(along.len() > 90, "{} boards", along.len());
        for b in &along {
            assert_eq!(b.hi[0] - b.lo[0], 3, "a board is 0.3 m wide along the way");
            assert_eq!(b.hi[2] - b.lo[2], CELLS, "and runs right across its column");
            assert_eq!(b.lo[1], 41 * CELLS, "lying on the ground");
            assert_eq!(b.thin, 0);
        }
        // Boards side by side across the way are one board, one colour.
        for b in &along {
            if let Some(n) = along.iter().find(|n| n.lo[0] == b.lo[0] && n.lo[2] == b.lo[2] + CELLS) {
                assert_eq!(n.voxel, b.voxel);
            }
        }
        // Turned with the trail.
        let across = boards(m(0.95), 0.5, glam::Vec2::Y);
        assert!(across.iter().all(|b| b.hi[2] - b.lo[2] == 3 && b.hi[0] - b.lo[0] == CELLS));
        // None on a trace, nor at the frayed edge of a path.
        assert!(boards(m(0.25), 0.0, glam::Vec2::X).is_empty());
        assert!(boards(m(0.95), m(0.95), glam::Vec2::X).is_empty());
    }

    #[test]
    fn faint_paths_are_overgrown_and_busy_tracks_grassed_down_the_middle() {
        let share = |col: &Column, v: Voxel| {
            let n = (0..400).filter(|&k| surface(col, (k % 20) as f32 / 20.0, (k / 20) as f32 / 20.0).0 == v).count();
            n as f32 / 400.0
        };
        let grass = mat::ground_cover(Biome::Grassland);
        let mut trace = trail(Biome::Grassland, Cover::Land, 0.1, m(0.25));
        trace.worn.as_mut().unwrap().offset = 0.3;
        let mut track = trail(Biome::Grassland, Cover::Land, 0.1, m(0.95));
        track.worn.as_mut().unwrap().offset = 0.2;
        let mut rut = track;
        rut.worn.as_mut().unwrap().offset = m(0.6);
        assert!(share(&trace, grass) > 0.05, "a trace is broken by grass");
        assert!(share(&track, grass) > 0.25, "a track has grass down its middle");
        assert!(share(&rut, grass) < 0.05, "a track's ruts are bare");
        // Faint woodland paths go to moss; busy ones do not.
        let mut faint = trail(Biome::Deciduous, Cover::Land, 0.1, m(0.25));
        faint.worn.as_mut().unwrap().offset = 0.0;
        let busy = trail(Biome::Deciduous, Cover::Land, 0.1, m(0.95));
        assert!(share(&faint, mat::MOSS) > 0.05);
        assert_eq!(share(&busy, mat::MOSS), 0.0);
    }

    #[test]
    fn steep_used_paths_climb_by_rows_of_steps() {
        // A footpath climbing +X at 0.5 voxels a voxel through a wood: every other
        // column steps up, and each step's edge is a log across the way.
        let col = |x: usize, half_width: f32| {
            let mut c = trail(Biome::Deciduous, Cover::Land, 0.5, half_width);
            c.ground = 40.0 + x as f32 * 0.5;
            c
        };
        let grid = |half_width: f32| -> Vec<Column> { (0..PAD * PAD).map(|i| col(i / PAD, half_width)).collect() };
        let nose = |cols: &[Column], px: usize, pz: usize, steep: f32| crate::world::step_nose(cols, PAD, px, pz, steep, 0.3, 1, 0, 0);
        let path = grid(m(0.6));
        let mut rows = 0;
        for px in 1..PAD - 1 {
            let noses: Vec<_> = (1..PAD - 1).map(|pz| nose(&path, px, pz, 0.5)).collect();
            // A whole row across the path is one kind, all steps or none.
            assert!(noses.iter().all(|n| n.map(|s| (s.0, s.1)) == noses[0].map(|s| (s.0, s.1))), "row {px} broken: {noses:?}");
            let edge = path[px * PAD].top() > path[(px - 1) * PAD].top();
            assert_eq!(noses[0].is_some(), edge, "row {px}");
            if let Some(step) = noses[4] {
                assert_eq!((step.0, step.1), (Step::Log(mat::BARK_OAK), (-1, 0)), "a woodland step is a log, down the slope");
                assert_eq!(step.2, [false, false], "the log runs on across the way");
                rows += 1;
                // Its log lies along the lip, on top of the column.
                let mut parts = Vec::new();
                dress(&path[px * PAD + 5], Some(step), 7, 9, 1, &mut parts);
                assert_eq!(parts.len(), 1);
                let b = parts[0];
                assert_eq!((b.lo[0], b.hi[0]), (7 * CELLS, 7 * CELLS + 2), "at the downhill edge");
                assert_eq!((b.lo[2], b.hi[2]), (9 * CELLS, 10 * CELLS), "right across the column");
                assert_eq!(b.lo[1], (path[px * PAD + 5].top() + 1) * CELLS);
                // Where the way ends, so does the log, showing its cut end.
                let mut parts = Vec::new();
                dress(&path[px * PAD + 5], Some((step.0, step.1, [true, false])), 7, 9, 1, &mut parts);
                assert_eq!(parts.len(), 2);
                assert_eq!(parts[0].lo[2], 9 * CELLS + 1);
                assert_eq!((parts[1].lo[2], parts[1].hi[2], parts[1].voxel), (9 * CELLS, 9 * CELLS + 1, mat::PLANK_PALE));
            }
        }
        assert!(rows > 8);
        // Not on gentle ground, nor on a faint trace.
        assert!((1..PAD - 1).all(|px| nose(&path, px, 5, 0.1).is_none()));
        let trace = grid(m(0.25));
        assert!((1..PAD - 1).all(|px| nose(&trace, px, 5, 0.5).is_none()));
        // Over scree the steps are stone, in place of the top voxel.
        let mut scree = grid(m(0.6));
        for c in &mut scree {
            c.worn.as_mut().unwrap().under = Cover::Scree;
        }
        let stone = (1..PAD - 1).find_map(|px| nose(&scree, px, 5, 0.5));
        // Of the rock in the ground there: one of the grey beds of a broadleaf wood.
        let beds = [mat::STONE, mat::ROCK_DARK, mat::ROCK_PALE, mat::ROCK_BROWN];
        assert!(matches!(stone.map(|s| s.0), Some(Step::Stone(v)) if beds.contains(&v)), "{:?}", stone.map(|s| s.0));
    }

    #[test]
    fn trail_columns_know_what_they_were_worn_into() {
        // In a real world: every trail column knows its ground, nothing else claims one,
        // and the continent's trails wear to more than one kind of surface.
        let w = crate::world::GameWorld::with_config(3, crate::continent::SimConfig { size: 256 });
        let c = w.continent(0, 0);
        let mut treads = HashSet::new();
        let mut trail_cols = 0;
        for line in &c.trails.lines {
            for p in line.iter().step_by(3) {
                for (dx, dz) in [(0, 0), (3, 0), (0, 3)] {
                    let col = w.column(p.x as i32 + dx, p.y as i32 + dz);
                    assert_eq!(col.worn.is_some(), col.cover == Cover::Trail, "{col:?}");
                    if let Some(worn) = col.worn {
                        trail_cols += 1;
                        assert!(matches!(worn.under, Cover::Land | Cover::Bank | Cover::Beach | Cover::Scree | Cover::Cliff));
                        assert!(worn.half_width > 0.0);
                        treads.insert(tread(&col, &worn, 0.5));
                    }
                }
            }
        }
        println!("{trail_cols} trail columns, treads {treads:?}");
        assert!(trail_cols > 500);
        assert!(treads.len() >= 3, "only {treads:?}");
    }
}
