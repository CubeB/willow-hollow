//! Barns out in the fields (#64): where a settlement's fields lie well away from its
//! houses, the farm keeps a barn at the edge of them, so the hay need not be carted home
//! and the cattle can winter near their grazing.
//!
//! What a barn is depends on the field it stands by and the country: a hay barn by the
//! meadows and the corn, a byre by the pasture, and a field shed for the tools in poorer
//! or drier country, by the vines and the orchards, and on stilts in the marsh and the
//! rainforest. Each is built in its region's way (see [`special::barn`]).
//!
//! A barn stands on ground levelled for it as a building in the village is, among its
//! fields and off them: never on a trail or near enough to tilt one, nor by a river, in a
//! yard or on another building's ground. Its door is turned toward the nearest way, which
//! runs a short walk off, and the ground before it is trodden.
//!
//! Barns are not among a settlement's buildings (`Network::plots`), so they leave its
//! count of walk-in buildings alone; they are kept in [`Land::steads`](super::Land).

use glam::{IVec2, IVec3, Vec2, vec2};
use voxel::Voxel;

use super::{
    Crop,
    fields::worked,
    influence::{CROPS, Farm, ROUGH},
};
use crate::{
    continent::Continent,
    features::{parts, shape::rotate},
    noise::{hash2, unit},
    scale::m,
    settlements::{
        Age, Kind, Region,
        special::barn::{self, BarnKind, BarnSpec},
    },
    surface::Sampler,
    trails::{Pad, build::{in_parallel, palette_at, turns_toward}},
};

/// A barn out in a settlement's fields, as raised.
#[derive(Clone, Debug, PartialEq)]
pub struct Stead {
    /// The settlement whose fields it serves, by its index in `Network::settlements` and
    /// [`Land::farms`](super::Land::farms).
    pub owner: usize,
    pub kind: BarnKind,
    /// Where the barn's model origin stands, continent-local: the middle of its site, a
    /// layer over its levelled ground.
    pub origin: IVec3,
    pub turns: u32,
    /// The columns its walls stand on, continent-local, inclusive.
    pub walls: (IVec2, IVec2),
    /// Every column it covers, its eaves and what stands by it included.
    pub footprint: (IVec2, IVec2),
    /// The doorway's floor voxel, continent-local, and the way out through it.
    pub door: IVec3,
    pub facing: IVec3,
    /// Where each beast of a byre stands in its stall, continent-local, and the way it
    /// faces.
    pub stalls: Vec<(IVec3, IVec3)>,
}

impl Stead {
    /// Whether a stamp spanning `lo` to `hi`, continent-local, is this barn's: it holds
    /// the walls and keeps within what the barn covers. Nothing else stands there, the
    /// barn's site being clear of every way, yard and pad.
    pub fn stamped(&self, lo: IVec3, hi: IVec3) -> bool {
        let (lo, hi) = (IVec2::new(lo.x, lo.z), IVec2::new(hi.x, hi.z));
        let (a, b) = (self.footprint.0 - IVec2::splat(2), self.footprint.1 + IVec2::splat(2));
        lo.cmple(self.walls.0).all() && hi.cmpge(self.walls.1).all() && lo.cmpge(a).all() && hi.cmple(b).all()
    }
}

/// How far out among its fields a barn stands, as a share of how far they reach beyond
/// the houses: not among the nearest, which are carted from the village, nor at the
/// edge of the wild.
const OUT: (f32, f32) = (0.3, 0.75);
/// How far a barn stands from the nearest way, to the edge of its walk: near enough to
/// cart to, far enough that its levelled ground leaves the way as it was laid.
const WAY: (f32, f32) = (m(9.0), m(60.0));
/// What a barn's site keeps clear of: the edge of any trail or other building's ground,
/// further than a site's levelling reaches (`surface`'s pads blend over 4.8 m).
const CLEAR: f32 = m(6.5);
/// And of the edge of a river.
const RIVER: f32 = m(10.0);
/// How far apart two barns stand, at the least.
const APART: f32 = m(60.0);
/// The most the ground may rise across a barn's site, in voxels.
const RISE: f32 = 6.0;
/// Places looked at for each settlement's barns.
const LOOKS: u32 = 96;

/// How many barns a settlement keeps out in its fields: none for a young hamlet, a few
/// for an old town whose fields reach far.
fn wanted(farm: &Farm) -> usize {
    let n: usize = match farm.kind {
        Kind::Hamlet => 1,
        Kind::Village => 2,
        Kind::LargeVillage => 2,
        Kind::Town => 3,
        Kind::GreatTown => 4,
    };
    match farm.age {
        Age::Young => n - 1,
        Age::Established => n,
        Age::Ancient => n + 1,
    }
}

/// What kind of barn `farm` keeps by a field of `crop`; `roll` is a chance, 0 to 1.
fn kind_by(farm: &Farm, crop: Crop, roll: f32) -> BarnKind {
    // Poor or dry country, and the wet, keep a shed for the tools and no more.
    if farm.region == Region::Desert || farm.biome.arid() || farm.prosperity < 0.22 || matches!(farm.region, Region::Wetland | Region::Rainforest) {
        return BarnKind::Shed;
    }
    match crop {
        Crop::Pasture | Crop::Rough => BarnKind::Byre,
        Crop::Hay => BarnKind::Hay,
        Crop::Wheat | Crop::Barley | Crop::Rye | Crop::Roots | Crop::Flax | Crop::Ploughed | Crop::Fallow => {
            if roll < 0.7 { BarnKind::Hay } else { BarnKind::Shed }
        }
        Crop::Vines | Crop::Olives | Crop::Orchard | Crop::Coppice => BarnKind::Shed,
    }
}

/// How old a barn of a place of `age` looks, as [`BarnSpec::age`].
fn weathered(age: Age, roll: f32) -> f32 {
    let base = match age {
        Age::Young => 0.15,
        Age::Established => 0.45,
        Age::Ancient => 0.75,
    };
    (base + roll * 0.25).min(1.0)
}

/// A barn chosen for a site, before it is stamped into the world.
struct Chosen {
    stead: Stead,
    barn: barn::Barn,
    pads: Vec<Pad>,
}

/// Finds the barns of every settlement of continent `c` a site, and raises them: levels
/// their ground and stamps them into the trails' network as the village's buildings
/// are, and keeps them in [`Land::steads`](super::Land). For a continent whose trails and
/// land are built; `seed` is the world's.
pub fn raise(c: &mut Continent, seed: u32) {
    let farms: Vec<usize> = (0..c.land.farms.len()).collect();
    let chosen: Vec<Vec<Chosen>> = in_parallel(&farms, |&k| choose(c, seed, k));
    let mut steads = Vec::new();
    for barn in chosen.into_iter().flatten() {
        // Each settlement's barns stand apart; those of two neighbours may not have.
        if steads.iter().any(|s: &Stead| centre(s).distance(centre(&barn.stead)) < APART) {
            continue;
        }
        let Chosen { stead, barn, pads } = barn;
        c.trails.pads.extend(pads);
        let b = &barn.building;
        let voxels: Vec<(IVec3, Voxel)> = b.voxels.iter().map(|&(q, v)| (stead.origin + rotate(q, stead.turns), v)).collect();
        let fine = parts::place_fine(&b.fine, stead.origin, stead.turns);
        c.trails.add_stamp_with(voxels, fine);
        steads.push(stead);
    }
    c.land.steads = steads;
}

fn centre(s: &Stead) -> Vec2 {
    (s.walls.0 + s.walls.1).as_vec2() * 0.5
}

/// The barns of settlement `k` of `c`, each on its site.
fn choose(c: &Continent, seed: u32, k: usize) -> Vec<Chosen> {
    let land = &c.land;
    let farm = &land.farms[k];
    let want = wanted(farm);
    let mut out: Vec<Chosen> = Vec::new();
    if want == 0 {
        return out;
    }
    // Places about the settlement, out among its fields, nearest a way first.
    let turn0 = unit(hash2(farm.seed ^ 0xba4e, k as i32, 1)) * std::f32::consts::TAU;
    let mut looks: Vec<(f32, Vec2, Vec2, Crop)> = Vec::new();
    for i in 0..LOOKS {
        let u = unit(hash2(farm.seed ^ 0xba4f, i as i32, 2));
        let angle = turn0 + i as f32 * 2.399_963;
        let p = farm.at + Vec2::from_angle(angle) * (farm.radius + farm.reach * (OUT.0 + (OUT.1 - OUT.0) * u));
        let (lx, lz) = (p.x.floor() as i32, p.y.floor() as i32);
        let managed = land.managed(lx, lz);
        if land.owner(lx, lz) != Some(k) || !(ROUGH + 0.05..=CROPS + 0.1).contains(&managed) || land.yard(lx, lz) {
            continue;
        }
        if land.farms.iter().any(|f| f.at.distance(p) < f.radius + m(30.0)) {
            continue;
        }
        let Some(field) = super::plot(c, seed, lx, lz) else { continue };
        // The nearest way, from the edge of its walk.
        let (points, _) = c.trails.gather(p - Vec2::splat(WAY.1 + m(8.0)), p + Vec2::splat(WAY.1 + m(8.0)));
        let Some((d, to)) = points
            .iter()
            .filter(|t| t.half_width > 0.0)
            .map(|t| (vec2(t.x, t.z).distance(p) - t.half_width, vec2(t.x, t.z)))
            .min_by(|a, b| a.0.total_cmp(&b.0))
        else {
            continue;
        };
        if !(WAY.0..=WAY.1).contains(&d) {
            continue;
        }
        looks.push((d, p, to, field.crop));
    }
    looks.sort_by(|a, b| a.0.total_cmp(&b.0));
    // Built of what the place's houses are: its stone, from the rock under its middle.
    let s = &c.trails.settlements[k];
    let pal = palette_at(c, seed, farm.biome, s.x, s.y - 1, s.z).varied(farm.seed ^ 0xba57);
    // The barn of each kind the settlement would build next, built once whatever site it
    // is tried on.
    let mut next: Vec<(BarnKind, barn::Barn)> = Vec::new();
    for (p, to, crop) in looks.into_iter().map(|(_, p, to, crop)| (p, to, crop)) {
        if out.len() >= want {
            break;
        }
        if out.iter().any(|o| centre(&o.stead).distance(p) < APART) {
            continue;
        }
        let n = out.len() as i32;
        let kind = kind_by(farm, crop, unit(hash2(farm.seed ^ 0xba50, p.x as i32, p.y as i32)));
        if !next.iter().any(|(k, _)| *k == kind) {
            let roll = |salt: u32| unit(hash2(farm.seed ^ salt, n, kind as i32));
            let spec = BarnSpec {
                kind,
                region: farm.region,
                age: weathered(farm.age, roll(0xba51)),
                wealth: (farm.prosperity * 0.8 + roll(0xba52) * 0.2).clamp(0.0, 1.0),
                seed: farm.seed ^ hash2(0xba53, n, kind as i32),
            };
            next.push((kind, barn::build(&spec, &pal)));
        }
        let built = &next.iter().find(|(k, _)| *k == kind).expect("built above").1;
        let origin = p.floor().as_ivec2();
        let turns = turns_toward(to - p);
        let turned = |(lo, hi): (IVec2, IVec2)| {
            let (a, b) = (rotate(IVec3::new(lo.x, 0, lo.y), turns), rotate(IVec3::new(hi.x, 0, hi.y), turns));
            (origin + IVec2::new(a.x.min(b.x), a.z.min(b.z)), origin + IVec2::new(a.x.max(b.x), a.z.max(b.z)))
        };
        let footprint = turned((built.building.footprint.lo, built.building.footprint.hi));
        let Some(y) = site(c, seed, footprint) else { continue };
        let built = next.remove(next.iter().position(|(k, _)| *k == kind).expect("built above")).1;
        // The next barn is another.
        next.clear();
        let walls = turned((built.building.walls.lo, built.building.walls.hi));
        let at = IVec3::new(origin.x, y + 1, origin.y);
        let door = built.building.door;
        let facing = rotate(door.facing, turns);
        let pads: Vec<Pad> = cover(footprint)
            .into_iter()
            .map(|(c, r)| {
                let mut pad = Pad::plain(c.x, c.y, r, y as f32);
                // Trodden before the door, where the cart stands and the cattle go in.
                pad.wear = 0.3;
                let step = (at + rotate(door.floor, turns) + facing * 2).as_vec3();
                let d = vec2(step.x + 0.5, step.z + 0.5) - c;
                pad.front = d.y.atan2(d.x);
                pad.door = d.length();
                pad
            })
            .collect();
        let stead = Stead {
            owner: k,
            kind,
            origin: at,
            turns,
            walls,
            footprint,
            door: at + rotate(door.floor, turns),
            facing,
            stalls: built.stalls.iter().map(|&(q, d)| (at + rotate(q, turns), rotate(d, turns))).collect(),
        };
        out.push(Chosen { stead, barn: built, pads });
    }
    out
}

/// Whether the columns `lo` to `hi` of `c` (continent-local, inclusive) are a site for
/// a barn, and if so the height to level them to: dry, worked ground clear of every
/// trail, building and river, rising no more than a little across.
fn site(c: &Continent, seed: u32, (lo, hi): (IVec2, IVec2)) -> Option<i32> {
    let (lo, hi) = (lo - IVec2::splat(2), hi + IVec2::splat(2));
    let span = (hi - lo).max_element() + 1;
    let sampler = Sampler::new(c, seed, lo.x, lo.y, span);
    let (mut sum, mut n, mut low, mut high) = (0.0, 0.0, f32::MAX, f32::MIN);
    // Every other column: a building's site is wider than anything that would spoil it.
    for z in (lo.y..=hi.y).step_by(2) {
        for x in (lo.x..=hi.x).step_by(2) {
            let col = sampler.column(x, z);
            if !worked(&col) || col.clear < CLEAR || col.river < RIVER || c.land.yard(x, z) {
                return None;
            }
            sum += col.ground;
            n += 1.0;
            low = low.min(col.ground);
            high = high.max(col.ground);
        }
    }
    if high - low > RISE || n == 0.0 {
        return None;
    }
    Some((sum / n).floor() as i32)
}

/// Discs covering columns `lo` to `hi` (inclusive) with a column to spare beyond their
/// corners: pieces about as long as they are wide along the longer side, so a long barn
/// is not levelled as one wide round (as the village's buildings are).
pub(super) fn cover((lo, hi): (IVec2, IVec2)) -> Vec<(Vec2, f32)> {
    let size = (hi - lo + IVec2::ONE).as_vec2();
    let long_x = size.x >= size.y;
    let (long, short) = if long_x { (size.x, size.y) } else { (size.y, size.x) };
    let n = (long / short).round().max(1.0) as i32;
    let piece = long / n as f32;
    (0..n)
        .map(|i| {
            let along = (i as f32 + 0.5) * piece;
            let (at, half) = if long_x {
                (vec2(lo.x as f32 + along, lo.y as f32 + size.y * 0.5), vec2(piece, short) * 0.5)
            } else {
                (vec2(lo.x as f32 + size.x * 0.5, lo.y as f32 + along), vec2(short, piece) * 0.5)
            };
            (at, half.length() + 1.0)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use voxel::CHUNK;

    use super::*;
    use crate::{World, continent::SimConfig, world::GameWorld};

    fn world() -> GameWorld {
        GameWorld::with_config(3, SimConfig { size: 256 })
    }

    /// Barns stand out among the fields, off every trail, river, yard and building's
    /// ground, each walked into through its door from the ground before it, and they are
    /// none of them among the settlements' buildings.
    #[test]
    fn barns_stand_in_the_fields_off_trails_rivers_and_plots_and_are_walked_into() {
        let w = world();
        let c = w.continent(0, 0);
        let steads = &c.land.steads;
        assert!(steads.len() >= 3, "only {} barns on the continent", steads.len());
        let kinds: std::collections::HashSet<BarnKind> = steads.iter().map(|s| s.kind).collect();
        println!("{} barns: {:?}", steads.len(), steads.iter().map(|s| (s.kind, s.origin)).collect::<Vec<_>>());
        for s in steads {
            let farm = &c.land.farms[s.owner];
            let mid = centre(s);
            assert!(farm.at.distance(mid) > farm.radius + m(20.0), "a barn at {mid} is in among the houses");
            assert!(c.land.managed(mid.x as i32, mid.y as i32) > 0.0, "a barn at {mid} is out in the wild");
            // Off the trails, well clear of the edge of every one: the ground trodden
            // before its door is its own.
            let (lo, hi) = (s.footprint.0 - IVec2::splat(3), s.footprint.1 + IVec2::splat(3));
            let (points, _) = c.trails.gather(lo.as_vec2(), hi.as_vec2());
            for t in points.iter().filter(|t| t.half_width > 0.0) {
                let p = Vec2::new(t.x, t.z).clamp(s.footprint.0.as_vec2(), (s.footprint.1 + IVec2::ONE).as_vec2());
                assert!(p.distance(Vec2::new(t.x, t.z)) - t.half_width > m(4.8), "a trail at {},{} by the barn at {mid}", t.x, t.z);
            }
            // Off the rivers and out of the yards: the columns about it, as the world has
            // them with the barn's ground levelled.
            let span = (hi - lo).max_element() + 1;
            let sampler = Sampler::new(&c, w.seed(), lo.x, lo.y, span);
            for z in lo.y..=hi.y {
                for x in lo.x..=hi.x {
                    let col = sampler.column(x, z);
                    assert!(col.water_surface().is_none() && col.river > m(5.0), "water at {x},{z} by the barn at {mid}");
                    assert!(!c.land.yard(x, z), "a yard at {x},{z} under the barn at {mid}");
                }
            }
            // Clear of the settlements' buildings.
            for p in &c.trails.plots {
                let d = (IVec2::new(p.origin.x, p.origin.z).as_vec2() - mid).length();
                assert!(d > m(30.0), "a barn at {mid} by a building at {}", p.origin);
            }
            // Standing level on its ground: every column under its walls at the ground
            // the barn was raised over.
            for z in s.walls.0.y..=s.walls.1.y {
                for x in s.walls.0.x..=s.walls.1.x {
                    assert_eq!(sampler.column(x, z).top(), s.origin.y - 1, "the ground under the barn at {mid} is not level at {x},{z}");
                }
            }
            // Its stamp is known for its own, and nothing else stamped near it is taken
            // for it (the climbers keep off it by this).
            let mut near: Vec<&crate::trails::Stamp> = Vec::new();
            for cz in s.footprint.0.y.div_euclid(CHUNK) - 1..=s.footprint.1.y.div_euclid(CHUNK) + 1 {
                for cx in s.footprint.0.x.div_euclid(CHUNK) - 1..=s.footprint.1.x.div_euclid(CHUNK) + 1 {
                    for t in c.trails.stamps_at(cx, cz) {
                        if !near.iter().any(|&n| std::ptr::eq(n, t)) {
                            near.push(t);
                        }
                    }
                }
            }
            let ours = near.iter().filter(|t| s.stamped(t.lo, t.hi)).count();
            assert_eq!(ours, 1, "{ours} stamps taken for the barn at {mid}, of {} near", near.len());
        }
        // The barns are walked into: each kind as built is (see `special::barn`'s tests);
        // here, that what was stamped is that barn, door and all.
        for s in steads.iter().take(6) {
            let (cx, cz) = (s.door.x.div_euclid(CHUNK), s.door.z.div_euclid(CHUNK));
            let column = w.generate_column(cx, cz);
            let get = |q: IVec3| {
                let (cy, ly) = (q.y.div_euclid(CHUNK), q.y.rem_euclid(CHUNK));
                column.chunks.iter().find(|(k, _)| *k == cy).map(|(_, ch)| ch.get((q.x - cx * CHUNK + 1) as usize, ly as usize + 1, (q.z - cz * CHUNK + 1) as usize))
            };
            // The doorway is open, a person's height, over the floor.
            for y in 1..=4 {
                assert_eq!(get(s.door + IVec3::Y * y), Some(voxel::AIR), "the {:?} at {} has its door blocked", s.kind, s.door);
            }
        }
        assert!(kinds.len() >= 2, "only {kinds:?}");
    }

    /// Lists the barns of a world's first continent, and how long finding them sites
    /// takes (`SEED=1 cargo test -p worldgen -- list_barns --ignored --nocapture`).
    #[test]
    #[ignore]
    fn list_barns() {
        let seed = std::env::var("SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
        let mut c = crate::continent::generate(crate::macro_layout::cell_desc(seed, 0, 0), SimConfig::default());
        c.trails = crate::trails::build(&c, seed);
        c.land = super::super::influence::build(&c, seed);
        let start = std::time::Instant::now();
        raise(&mut c, seed);
        println!("{} barns for {} settlements in {:.1} ms", c.land.steads.len(), c.land.farms.len(), start.elapsed().as_secs_f64() * 1000.0);
        for s in &c.land.steads {
            let home = &c.trails.settlements[s.owner];
            println!("{:10} of {:?} {:?} at {},{}: door {} facing {} walls {:?}", s.kind.word(), home.kind, home.region, home.x, home.z, s.door, s.facing, s.walls);
        }
    }

    /// A byre's cattle graze beside it, and nothing kept by a settlement is penned over a
    /// barn.
    #[test]
    fn a_byres_cattle_graze_beside_it_and_no_pen_is_over_a_barn() {
        let w = world();
        let c = w.continent(0, 0);
        let (mut byres, mut grazed) = (0, 0);
        for k in 0..c.trails.settlements.len() {
            let kept = crate::fauna::kept_about(&c, w.seed(), k);
            for s in &c.land.steads {
                let (a, b) = (s.footprint.0.as_vec2(), (s.footprint.1 + IVec2::ONE).as_vec2());
                for p in &kept {
                    let (lo, hi) = (p.middle - p.half, p.middle + p.half);
                    assert!(hi.x <= a.x || lo.x >= b.x || hi.y <= a.y || lo.y >= b.y, "{:?} kept over the {:?} at {:?}", p.kind, s.kind, s.walls);
                }
                if s.owner == k && s.kind == BarnKind::Byre && crate::fauna::kept_by(&c.trails.settlements[k]).contains(&crate::fauna::Kind::Cattle) {
                    byres += 1;
                    grazed += kept.iter().any(|p| p.kind == crate::fauna::Kind::Cattle && p.middle.distance(centre(s)) < m(20.0)) as u32;
                }
            }
        }
        println!("{grazed} of {byres} byres with cattle by them");
        assert!(byres == 0 || grazed * 2 >= byres, "only {grazed} of {byres} byres have their cattle by them");
    }

    /// The barns are the same however often the continent is made.
    #[test]
    fn barns_are_the_same_every_time() {
        let a = world();
        let b = world();
        assert_eq!(a.continent(0, 0).land.steads, b.continent(0, 0).land.steads);
    }
}
