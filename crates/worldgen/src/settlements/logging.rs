//! The logging camp out in the woods (#203, #207). A settlement that lives by felling
//! timber works a camp some way out beyond its fields, in the woods themselves: a yard
//! trodden bare in a clearing, a lean-to roofed with bark slabs over the bench and the
//! cordwood, a saw pit with a squared log across it on its trestles, the logs waiting
//! to be sawn, a sawhorse, a chopping block and a cart to take the timber home.
//!
//! The clearing grows year by year as the woods are felled back ([`Camp::reach`]),
//! leaving stumps where the trees stood and the oldest of them grubbed out, until it
//! reaches as far as the land lets it: water, a river's bank, steep ground, the edge of
//! the woods or of the fields. Which trees are down in a year is [`fell`], a pure
//! function of the continent and the moment that every reader of the trees calls, so
//! the chunks, the far rings, the far trees and the climbers agree.
//!
//! A camp is not among a settlement's buildings (`Network::plots`), so it leaves its
//! count of walk-in buildings alone: camps are kept in `Network::years`, and raised once
//! the fields and their barns are (see `farmland::farmstead`).

use glam::{IVec2, IVec3, Vec2, vec2};
use voxel::{AIR, Voxel, mesh::{CELLS, PlantBox}};

use super::{Age, Livelihood, years::When};
use crate::{
    biome::Biome,
    continent::Continent,
    features::{
        self, Library, Placement,
        fittings::{self, Fitting},
        landmarks::Palette,
        parts,
        shape::rotate,
        trees::Species,
    },
    macro_layout::MACRO_SIZE,
    materials as mat,
    noise::{hash2, unit},
    scale::m,
    surface::{Cover, Sampler},
    trails::{
        Pad,
        build::{in_parallel, palette_at, turns_toward},
    },
};

/// A settlement's logging camp, as raised.
#[derive(Clone, Debug, PartialEq)]
pub struct Camp {
    /// The settlement it works for, by its index in `Network::settlements`.
    pub owner: usize,
    /// The middle of its yard, continent-local, a layer over its levelled ground.
    pub at: IVec3,
    /// Quarter turns from the camp as built: its front, where the saw pit is, faces the
    /// nearest way.
    pub turns: u32,
    /// How far the yard reaches from the middle, in voxels: nothing grows on it.
    pub yard: f32,
    /// How far out from the middle the woods had been felled when the world began, and
    /// the furthest the land lets the clearing grow, in voxels.
    pub first: f32,
    pub most: f32,
    /// Every column its stamp covers, continent-local, inclusive.
    pub footprint: (IVec2, IVec2),
    pub seed: u32,
}

/// How much further the woods are felled back each year.
pub const GROWS: f32 = m(1.6);
/// The most a clearing ever grows to, from the middle of its yard: a few acres felled.
const MOST: f32 = m(36.0);
/// How far the yard reaches from the middle.
const YARD: f32 = m(7.5);
/// How many years a stump stands before it may be grubbed out.
const GRUBBED: i32 = 6;
/// How far out beyond its fields a settlement's camp may be, at the most.
const OUT: f32 = m(240.0);
/// Places looked at for each settlement's camp.
const LOOKS: u32 = 64;
/// The fewest trees standing about a place for it to be in the woods.
const WOODED: usize = 10;

impl Camp {
    /// The middle of its yard, continent-local.
    pub fn middle(&self) -> Vec2 {
        vec2(self.at.x as f32 + 0.5, self.at.z as f32 + 0.5)
    }

    /// How far from the middle of its yard the woods have been felled by `when`: the
    /// first clearing and a ring more each year, up to as far as the land lets it.
    pub fn reach(&self, when: When) -> f32 {
        (self.first + GROWS * when.seen().year as f32).min(self.most)
    }

    /// The year the woods `d` from the middle were felled in, if they ever are: before
    /// year 0 within the first clearing, the nearer the middle the longer ago.
    pub fn felled_in(&self, d: f32) -> Option<i32> {
        (d <= self.most).then(|| ((d - self.first) / GROWS).ceil() as i32)
    }

    /// Whether a stamp spanning `lo` to `hi`, continent-local, is this camp's: it keeps
    /// within what the camp covers, which nothing else does, the camp's yard being clear
    /// of every way and building.
    pub fn stamped(&self, lo: IVec3, hi: IVec3) -> bool {
        let (a, b) = (self.footprint.0 - IVec2::splat(2), self.footprint.1 + IVec2::splat(2));
        lo.x >= a.x && lo.z >= a.y && hi.x <= b.x && hi.z <= b.y
    }

    /// Whether anything felled about the camp in any year reaches the square of
    /// continent-local columns from `(lx0, lz0)`, `span` wide: a felled tree's crown
    /// reaches out from its trunk.
    pub fn reaches(&self, lx0: i32, lz0: i32, span: i32) -> bool {
        let mid = self.middle();
        let near = vec2(mid.x.clamp(lx0 as f32, (lx0 + span) as f32), mid.y.clamp(lz0 as f32, (lz0 + span) as f32));
        near.distance(mid) <= self.most + features::MAX_REACH as f32 + 1.0
    }
}

/// Whether a loggers would fell what grows as `species`: the trees, but not the
/// undergrowth, nor what was never a tree.
fn timber(species: Species) -> bool {
    !matches!(
        species,
        Species::Bush
            | Species::Fern
            | Species::Boulder
            | Species::MossyBoulder
            | Species::FallenLog
            | Species::Stump
            | Species::HollyBush
            | Species::Laurel
            | Species::Gorse
            | Species::Cactus
    )
}

/// The features of a square as they stand at `when` about the logging camps of `c`:
/// the trees the loggers have felled are stumps, some of the oldest grubbed out, and
/// nothing at all grows on a camp's yard. `origin` is the world column of the
/// continent's corner, as `features::place` takes it. The same for every square that
/// holds a feature, so every reader of the trees agrees, and costs nothing on a
/// continent without a camp.
pub fn fell(c: &Continent, when: When, origin: (i32, i32), placements: &mut Vec<Placement>) {
    let camps = &c.trails.years.camps;
    if camps.is_empty() {
        return;
    }
    let when = when.seen();
    placements.retain_mut(|p| {
        let at = vec2((p.pos.x - origin.0) as f32 + 0.5, (p.pos.z - origin.1) as f32 + 0.5);
        let Some((camp, d)) = camps.iter().map(|k| (k, at.distance(k.middle()))).find(|&(k, d)| d <= k.most) else { return true };
        // The yard and its edge, where the work is done.
        if d < camp.yard + m(1.5) {
            return false;
        }
        if !timber(p.species) || d > camp.reach(when) {
            return true;
        }
        let felled = camp.felled_in(d).unwrap_or(when.year);
        let roll = hash2(camp.seed ^ 0x57a3_9f01, p.pos.x, p.pos.z);
        if when.year - felled >= GRUBBED && unit(roll) < 0.45 {
            return false;
        }
        // Bedded a voxel into the ground the trunk stood on, as a stump grown wild is.
        *p = Placement { pos: p.pos - IVec3::Y, species: Species::Stump, variant: (roll >> 8) as usize % 64, turns: p.turns, lie: None, roots: None };
        true
    });
}

/// What is raised for a camp, before it is stamped into the world.
struct Chosen {
    camp: Camp,
    voxels: Vec<(IVec3, Voxel)>,
    fine: Vec<PlantBox>,
    pad: Pad,
}

/// Finds the logging camp of every settlement of continent `c` that lives by one a
/// site in the woods, and raises it: levels its yard and stamps it into the trails'
/// network, and keeps it in `Network::years`. For a continent whose trails, land and
/// barns are built; `seed` is the world's.
pub fn raise(c: &mut Continent, seed: u32) {
    let owners: Vec<usize> = (0..c.trails.settlements.len()).filter(|&k| c.trails.settlements[k].livelihoods.contains(&Livelihood::LoggingCamp)).collect();
    let chosen: Vec<Option<Chosen>> = in_parallel(&owners, |&k| choose(c, seed, k));
    let mut camps: Vec<Camp> = Vec::new();
    for Chosen { camp, voxels, fine, pad } in chosen.into_iter().flatten() {
        // Two neighbours' clearings never run into one another.
        if camps.iter().any(|o| o.middle().distance(camp.middle()) < o.most + camp.most + m(12.0)) {
            continue;
        }
        c.trails.pads.push(pad);
        c.trails.add_stamp_with(voxels, fine);
        camps.push(camp);
    }
    c.trails.years.camps = camps;
}

/// Whether woods of this biome are worked for timber.
fn forest(biome: Biome) -> bool {
    use Biome::*;
    matches!(biome, Deciduous | Woodland | BirchGrove | Taiga | SnowyPine | OldGrowth | Rainforest)
}

/// The camp of settlement `k` of `c`, on its site, or None if the woods about it have
/// nowhere for one.
fn choose(c: &Continent, seed: u32, k: usize) -> Option<Chosen> {
    let s = &c.trails.settlements[k];
    let farm = c.land.farms.get(k)?;
    let lib = Library::shared();
    let turn0 = unit(hash2(s.seed ^ 0x1066, k as i32, 1)) * std::f32::consts::TAU;
    let edge = m(48.0);
    for i in 0..LOOKS {
        let u = unit(hash2(s.seed ^ 0x1067, i as i32, 2));
        let angle = turn0 + i as f32 * 2.399_963;
        // Out beyond the fields, where the woods begin, and further the more it looks.
        let out = farm.radius + farm.reach + m(30.0) + u * OUT * (0.3 + 0.7 * i as f32 / LOOKS as f32);
        let p = farm.at + Vec2::from_angle(angle) * out;
        if p.min_element() < edge || p.max_element() > MACRO_SIZE as f32 - edge {
            continue;
        }
        let (lx, lz) = (p.x.floor() as i32, p.y.floor() as i32);
        // Out in the wild, beyond every settlement's fields and barns.
        if c.land.managed(lx, lz) > 0.0 {
            continue;
        }
        let Some(y) = site(c, seed, p) else { continue };
        if !wooded(c, seed, lib, p) {
            continue;
        }
        let first = match s.age {
            Age::Young => m(11.0),
            Age::Established => m(14.0),
            Age::Ancient => m(17.0),
        } + unit(hash2(s.seed ^ 0x1068, k as i32, 3)) * m(2.0);
        let most = room(c, seed, p, y, first);
        // The way out: toward the nearest trail, or home.
        let (points, _) = c.trails.gather(p - Vec2::splat(m(120.0)), p + Vec2::splat(m(120.0)));
        let to = points.iter().filter(|t| t.half_width > 0.0).map(|t| vec2(t.x, t.z)).min_by(|a, b| a.distance(p).total_cmp(&b.distance(p))).unwrap_or(farm.at);
        let turns = turns_toward(to - p);
        let pal = palette_at(c, seed, farm.biome, s.x, s.y - 1, s.z).varied(farm.seed ^ 0x1069);
        let at = IVec3::new(lx, y + 1, lz);
        let seed_k = hash2(s.seed ^ 0x106a, k as i32, 4);
        let (local, fine) = build(&pal, bark(sampler_biome(c, seed, lx, lz)), seed_k);
        let voxels: Vec<(IVec3, Voxel)> = local.into_iter().map(|(q, v)| (at + rotate(q, turns), v)).collect();
        let fine = parts::place_fine(&fine, at, turns);
        let (mut lo, mut hi) = (IVec2::splat(i32::MAX), IVec2::splat(i32::MIN));
        for q in voxels.iter().map(|&(q, _)| q).chain(fine.iter().map(|f| IVec3::from_array(f.lo).div_euclid(IVec3::splat(CELLS)))) {
            lo = lo.min(IVec2::new(q.x, q.z));
            hi = hi.max(IVec2::new(q.x, q.z));
        }
        let mut pad = Pad::plain(p.x.floor() + 0.5, p.y.floor() + 0.5, YARD + 1.0, y as f32);
        // Trodden bare where the work is done, from the lean-to over the saw pit: so no
        // moss grows on the log being sawn, nor on the pit's boards (see `features::moss`,
        // which keeps off a trodden way).
        pad.wear = 0.75;
        pad.hub = m(6.0);
        let camp = Camp { owner: k, at, turns, yard: YARD, first, most, footprint: (lo, hi), seed: seed_k };
        return Some(Chosen { camp, voxels, fine, pad });
    }
    None
}

fn sampler_biome(c: &Continent, seed: u32, lx: i32, lz: i32) -> Biome {
    Sampler::new(c, seed, lx, lz, 1).column(lx, lz).biome
}

/// The bark of the woods of `biome`, which the camp's logs are.
fn bark(biome: Biome) -> Voxel {
    use Biome::*;
    match biome {
        Taiga | SnowyPine => mat::BARK_PINE,
        BirchGrove => mat::BARK_BIRCH,
        Rainforest => mat::BARK_JUNGLE,
        OldGrowth => mat::BARK_REDWOOD,
        _ => mat::BARK_OAK,
    }
}

/// Whether the yard about `p` (continent-local) is a site for a camp, and if so the
/// height to level it to: dry woodland ground clear of every trail, landmark and river,
/// rising no more than a little across.
fn site(c: &Continent, seed: u32, p: Vec2) -> Option<i32> {
    let r = (YARD + 2.0).ceil() as i32;
    let (x0, z0) = (p.x.floor() as i32 - r, p.y.floor() as i32 - r);
    let sampler = Sampler::new(c, seed, x0, z0, 2 * r + 1);
    let (mut sum, mut n, mut low, mut high) = (0.0, 0.0, f32::MAX, f32::MIN);
    for z in (z0..=z0 + 2 * r).step_by(2) {
        for x in (x0..=x0 + 2 * r).step_by(2) {
            if vec2(x as f32 + 0.5, z as f32 + 0.5).distance(p) > r as f32 {
                continue;
            }
            let col = sampler.column(x, z);
            if col.cover != Cover::Land || col.water_surface().is_some() || !forest(col.biome) || col.clear < m(8.0) || col.river < m(14.0) || c.land.yard(x, z) {
                return None;
            }
            sum += col.ground;
            n += 1.0;
            low = low.min(col.ground);
            high = high.max(col.ground);
        }
    }
    (n > 0.0 && high - low <= 5.0).then(|| (sum / n).floor() as i32)
}

/// Whether trees stand thick about `p`: whether it is in the woods.
fn wooded(c: &Continent, seed: u32, lib: &Library, p: Vec2) -> bool {
    let r = m(22.0) as i32;
    let (lx0, lz0) = (p.x as i32 - r, p.y as i32 - r);
    let margin = features::MAX_REACH + 1;
    let sampler = Sampler::new(c, seed, lx0 - margin, lz0 - margin, 2 * r + 2 * margin);
    let origin = (c.desc.mx * MACRO_SIZE, c.desc.mz * MACRO_SIZE);
    let trees = features::place_trees(seed, lib, &sampler, lx0, lz0, 2 * r, origin);
    let standing = trees
        .iter()
        .filter(|t| timber(t.species) && vec2((t.pos.x - origin.0) as f32, (t.pos.z - origin.1) as f32).distance(p) <= r as f32)
        .count();
    standing >= WOODED
}

/// How far out from `p` the woods may be felled, from `first` (always) to [`MOST`]: as
/// far as dry, gentle woodland of nobody's fields goes, ring by ring.
fn room(c: &Continent, seed: u32, p: Vec2, y: i32, first: f32) -> f32 {
    let span = (2.0 * MOST) as i32 + 4;
    let (x0, z0) = (p.x as i32 - span / 2, p.y as i32 - span / 2);
    let sampler = Sampler::new(c, seed, x0, z0, span);
    let mut most = first;
    let mut r = first + m(2.0);
    while r <= MOST {
        for j in 0..24 {
            let q = p + Vec2::from_angle(j as f32 / 24.0 * std::f32::consts::TAU) * r;
            let (x, z) = (q.x.floor() as i32, q.y.floor() as i32);
            let col = sampler.column(x, z);
            if col.cover != Cover::Land || col.water_surface().is_some() || col.river < m(5.0) || !forest(col.biome) || c.land.managed(x, z) > 0.0 || (col.ground - y as f32).abs() > m(5.0) {
                return most;
            }
        }
        most = r;
        r += m(2.0);
    }
    most
}

/// The way a fitting at a camp faces, toward `d` in the camp's own frame.
fn facing(d: Vec2) -> u32 {
    (0..4).max_by(|&a, &b| fittings::front(a).as_vec2().dot(d).total_cmp(&fittings::front(b).as_vec2().dot(d))).unwrap_or(0)
}

/// The camp as built, in its own frame: from the middle of its yard, a layer over the
/// ground, its front (the saw pit's end, toward the way) along +x.
fn build(pal: &Palette, bark: Voxel, seed: u32) -> (Vec<(IVec3, Voxel)>, Vec<PlantBox>) {
    let mut v: Vec<(IVec3, Voxel)> = Vec::new();
    let mut fine: Vec<PlantBox> = Vec::new();
    // What is taken on the yard's ground, low corner and high corner exclusive, and a
    // voxel about each kept clear.
    let mut taken: Vec<(IVec2, IVec2)> = Vec::new();

    // The lean-to at the back: a row of tall posts at the front, short at the back, and
    // a roof of bark slabs sloping down between them.
    let (front, back) = (-7, -12);
    for z in [-5, 0, 5] {
        for y in 0..=6 {
            v.push((IVec3::new(front, y, z), pal.wood));
        }
        for y in 0..=3 {
            v.push((IVec3::new(back, y, z), pal.wood));
        }
    }
    for x in back - 1..=front + 1 {
        let y = 4 + (x - back) * 3 / 5;
        for z in -6..=6 {
            v.push((IVec3::new(x, y, z), mat::BARK_PINE));
        }
    }
    taken.push((IVec2::new(back - 1, -6), IVec2::new(front + 2, 7)));

    // The saw pit, dug along the front and boarded, a squared log across it on its
    // trestles, its sawn ends pale.
    let (x0, x1) = (1, 12);
    for x in x0 - 1..=x1 + 1 {
        for z in -2i32..=2 {
            for y in -4..=-1 {
                let wall = x < x0 || x > x1 || z.abs() == 2;
                v.push((IVec3::new(x, y, z), if wall { pal.plank } else { AIR }));
            }
        }
    }
    for x in [x0 + 2, x1 - 2] {
        for z in -2i32..=2 {
            v.push((IVec3::new(x, 0, z), pal.plank));
        }
    }
    for x in x0..=x1 {
        for (y, z) in [(1, 0), (1, 1), (2, 0), (2, 1)] {
            let end = x == x0 || x == x1;
            v.push((IVec3::new(x, y, z), if end { mat::PLANK_PALE } else { bark }));
        }
    }
    taken.push((IVec2::new(x0 - 1, -2), IVec2::new(x1 + 2, 3)));

    // The chopping block, a round of trunk on end.
    for (x, z) in [(-3, -3), (-2, -3), (-3, -2), (-2, -2)] {
        v.push((IVec3::new(x, 0, z), bark));
        v.push((IVec3::new(x, 1, z), mat::PLANK_PALE));
    }
    taken.push((IVec2::new(-3, -3), IVec2::new(-1, -1)));

    // What stands about the yard, each where it first fits in its corner of it.
    let herbs: &[features::plants::Flower] = &[];
    let corners: [(Fitting, (IVec2, IVec2), Vec2); 7] = [
        // The bench and the cordwood under the lean-to, facing out.
        (Fitting::Workbench, (IVec2::new(back + 1, -4), IVec2::new(front - 1, 4)), vec2(1.0, 0.0)),
        (Fitting::LogStack, (IVec2::new(back + 1, -4), IVec2::new(front - 1, 4)), vec2(1.0, 0.0)),
        // The logs waiting to be sawn, beside the pit.
        (Fitting::LogDeck, (IVec2::new(1, 4), IVec2::new(14, 13)), vec2(0.0, -1.0)),
        (Fitting::RoundStack, (IVec2::new(-6, 4), IVec2::new(0, 12)), vec2(0.0, -1.0)),
        (Fitting::Sawhorse, (IVec2::new(-6, -12), IVec2::new(0, -4)), vec2(0.0, 1.0)),
        (Fitting::Handcart, (IVec2::new(2, -13), IVec2::new(14, -4)), vec2(1.0, 0.0)),
        (Fitting::LogStack, (IVec2::new(2, -13), IVec2::new(14, -4)), vec2(0.0, 1.0)),
    ];
    for (n, (kind, (lo, hi), toward)) in corners.into_iter().enumerate() {
        let ft = facing(toward);
        'fit: for z in lo.y..=hi.y {
            for x in lo.x..=hi.x {
                let (a, b) = kind.room(IVec3::new(x, 0, z), ft);
                let (a, b) = (IVec2::new(a.x, a.z), IVec2::new(b.x, b.z));
                if a.x < lo.x || a.y < lo.y || b.x > hi.x + 1 || b.y > hi.y + 1 {
                    continue;
                }
                if taken.iter().any(|&(p, q)| a.x < q.x + 1 && p.x - 1 < b.x && a.y < q.y + 1 && p.y - 1 < b.y) {
                    continue;
                }
                let f = fittings::build_with(kind, pal, hash2(seed, n as i32, 5), false, herbs).place(IVec3::new(x, 0, z), ft);
                v.extend(f.voxels);
                fine.extend(f.fine);
                taken.push((a, b));
                break 'fit;
            }
        }
    }
    (v, fine)
}

#[cfg(test)]
mod tests {
    use voxel::CHUNK;

    use super::*;
    use crate::{continent::SimConfig, world::GameWorld};

    fn world() -> GameWorld {
        GameWorld::with_config(3, SimConfig { size: 256 })
    }

    /// Lists the logging camps of a world's first continent, and how long raising them
    /// takes (`SEED=1 cargo test -p worldgen -- list_camps --ignored --nocapture`).
    #[test]
    #[ignore]
    fn list_camps() {
        let seed = std::env::var("SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
        let w = GameWorld::new(seed);
        let c = w.continent(0, 0);
        for camp in &c.trails.years.camps {
            let s = &c.trails.settlements[camp.owner];
            println!("camp of {:?} at {},{}: yard at {}, cleared to {:.0} of {:.0} voxels", s.kind, s.x, s.z, camp.at, camp.first, camp.most);
        }
    }

    /// Each settlement that lives by logging works a camp out in the woods beyond its
    /// fields, on level ground off every trail, and the camp is none of its buildings.
    #[test]
    fn a_logging_settlement_works_a_camp_out_in_the_woods() {
        let w = world();
        let c = w.continent(0, 0);
        let camps = &c.trails.years.camps;
        let loggers = c.trails.settlements.iter().filter(|s| s.livelihoods.contains(&Livelihood::LoggingCamp)).count();
        println!("{} camps for {loggers} settlements that log", camps.len());
        assert!(loggers == 0 || !camps.is_empty(), "no camp for {loggers} settlements that log");
        for camp in camps {
            let farm = &c.land.farms[camp.owner];
            assert!(farm.at.distance(camp.middle()) > farm.radius + farm.reach, "a camp among the fields");
            assert_eq!(c.land.managed(camp.at.x, camp.at.z), 0.0, "a camp in worked land");
            assert!(camp.first > camp.yard && camp.first <= camp.most && camp.most <= MOST);
            // Level under the yard, as the world has it.
            let r = 3;
            let sampler = Sampler::new(&c, w.seed(), camp.at.x - r, camp.at.z - r, 2 * r + 1);
            for z in -r..=r {
                for x in -r..=r {
                    assert_eq!(sampler.column(camp.at.x + x, camp.at.z + z).top(), camp.at.y - 1, "the yard is not level");
                }
            }
            // Its stamp is known for its own, and no building's is taken for it.
            let ours = c.trails.stamps().filter(|s| camp.stamped(s.lo, s.hi)).count();
            assert_eq!(ours, 1, "{ours} stamps taken for the camp at {}", camp.at);
        }
    }

    /// The clearing grows a ring a year to as far as the land lets it and no further:
    /// what was felled stays felled, the stumps stand where the trees stood, and the
    /// yard has nothing growing on it in any year.
    #[test]
    fn the_clearing_grows_year_by_year_as_far_as_the_land_lets_it() {
        let w = world();
        let c = w.continent(0, 0);
        let Some(camp) = c.trails.years.camps.first() else {
            println!("no camp on this continent");
            return;
        };
        let lib = w.library();
        let r = camp.most as i32 + features::MAX_REACH;
        let (lx0, lz0) = (camp.at.x - r, camp.at.z - r);
        let margin = features::MAX_REACH + 1;
        let sampler = Sampler::new(&c, w.seed(), lx0 - margin, lz0 - margin, 2 * r + 2 * margin);
        let origin = (c.desc.mx * MACRO_SIZE, c.desc.mz * MACRO_SIZE);
        let wild = features::place(w.seed(), lib, &sampler, lx0, lz0, 2 * r, origin);
        let standing = |year: i32| {
            let mut p = wild.clone();
            fell(&c, When::new(year, 2), origin, &mut p);
            p
        };
        let trees = |p: &[Placement]| p.iter().filter(|t| timber(t.species)).count();
        let mut last = usize::MAX;
        for year in 0..=50 {
            let now = standing(year);
            let reach = camp.reach(When::new(year, 2));
            assert!(reach <= camp.most && reach <= MOST, "cleared past what the land allows");
            let n = trees(&now);
            assert!(n <= last, "trees grew back in year {year}");
            last = n;
            for t in &now {
                let d = vec2((t.pos.x - origin.0) as f32 + 0.5, (t.pos.z - origin.1) as f32 + 0.5).distance(camp.middle());
                assert!(d >= camp.yard + m(1.5), "a {:?} on the yard in year {year}", t.species);
                assert!(!timber(t.species) || d > reach, "a tree standing in the clearing in year {year}");
            }
        }
        // It grew while there was room.
        if camp.most > camp.first + GROWS * 2.0 {
            assert!(trees(&standing(0)) > trees(&standing(20)), "the clearing never grew");
        }
        // The felled stand as stumps where the trees stood, or have been grubbed out.
        let then = standing(30);
        for t in wild.iter().filter(|t| timber(t.species)) {
            let d = vec2((t.pos.x - origin.0) as f32 + 0.5, (t.pos.z - origin.1) as f32 + 0.5).distance(camp.middle());
            if d < camp.reach(When::new(30, 2)) && d >= camp.yard + m(1.5) {
                assert!(then.iter().all(|s| s.pos.x != t.pos.x || s.pos.z != t.pos.z || s.species == Species::Stump));
            }
        }
    }

    /// The camp's chunks agree with one another across their borders in every year, as
    /// two chunks either side of one draw the same stumps and trees.
    #[test]
    fn the_camps_chunks_agree_across_their_borders_in_every_year() {
        let w = world();
        let c = w.continent(0, 0);
        let Some(camp) = c.trails.years.camps.first().cloned() else { return };
        let boons = super::super::years::Boons::default();
        let origin = (c.desc.mx * MACRO_SIZE, c.desc.mz * MACRO_SIZE);
        for year in [0, 3, 12] {
            w.set_now(When::new(year, 1), &boons);
            // Each border the clearing's edge crosses on the way out from the yard.
            let reach = camp.reach(When::new(year, 1)) as i32;
            let cz = (origin.1 + camp.at.z).div_euclid(CHUNK);
            let (c0, c1) = ((origin.0 + camp.at.x - reach - 8).div_euclid(CHUNK), (origin.0 + camp.at.x).div_euclid(CHUNK));
            for cx in c0..c1 {
                let (a, b) = (w.generate_column_bare(cx, cz).chunks, w.generate_column_bare(cx + 1, cz).chunks);
                for (cy, ca) in &a {
                    let Some((_, cb)) = b.iter().find(|e| e.0 == *cy) else { continue };
                    for (i, j) in [(voxel::PAD - 2, 0), (voxel::PAD - 1, 1)] {
                        for y in 0..voxel::PAD {
                            for o in 0..voxel::PAD {
                                assert_eq!(ca.get(i, y, o), cb.get(j, y, o), "year {year}: chunks {cx} and {} differ", cx + 1);
                            }
                        }
                    }
                }
            }
        }
        w.set_now(When::TODAY, &boons);
    }
}
