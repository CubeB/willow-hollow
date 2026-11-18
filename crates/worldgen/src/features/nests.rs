//! Wild bees' and wasps' nests, on the trees and boulders the world already grows.
//!
//! A colony of wild bees lives in a hollow: a slit in the trunk of an old tree, or a
//! crevice in a big boulder, dark inside with a glimpse of comb at the top. Wasps hang
//! their papery grey globes under a branch, clear of the leaves. Neither is a feature of
//! its own: each belongs to the tree or boulder it is in, chosen by a hash of where that
//! stands, so every chunk that stamps the host stamps its nest too, and
//! `GameWorld::nests_near` can say where they are without building any chunk. That list is
//! what the bees in the air fly from, and what a keeper of hives will one day look for.

use glam::IVec3;
use voxel::{AIR, Kind, Materials, Voxel};

use super::{Library, Placement, shape::rotate, trees::Species};
use crate::{
    biome::Biome,
    materials as mat,
    noise::{hash3, unit},
    scale::{BLOCK, m},
};

/// What lives in a nest.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum NestKind {
    /// A colony of wild honey bees.
    Bees,
    /// A colony of wasps.
    Wasps,
}

/// What a nest is in or under.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Host {
    /// A hollow in the trunk of an old tree.
    Hollow,
    /// A crevice in a boulder.
    Crevice,
    /// Hanging under a branch.
    Branch,
    /// A hive someone keeps: a skep, a log hive or a box hive, in a village's apiary or
    /// at a field's edge (#64, #202). Its door is the hive's own, already built with it.
    Hive,
}

/// One nest in the world.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Nest {
    pub kind: NestKind,
    pub host: Host,
    /// The world voxel the insects come and go by: the open mouth of a hollow or a
    /// crevice (air), or the bottom of a wasps' globe, where its entrance is.
    pub door: IVec3,
    /// The way out of the door, one voxel: outward from a trunk or a rock, or down from a
    /// hanging globe.
    pub out: IVec3,
    /// Where the tree or boulder holding it stands (its placement's origin).
    pub host_at: IVec3,
}

impl Nest {
    /// The point just outside the door, in world voxels, where a bee leaving is first in
    /// the open: what the insects in the air fly from.
    pub fn mouth(&self) -> glam::DVec3 {
        self.door.as_dvec3() + glam::DVec3::splat(0.5) + self.out.as_dvec3() * 0.9
    }

    /// The voxels it puts into the world, each at its world position: written over the
    /// host after the host is stamped.
    pub fn voxels(&self) -> Vec<(IVec3, Voxel)> {
        let up = IVec3::Y;
        match self.host {
            // A slit two voxels tall, open to the air, and behind it the dark of the
            // hollow with the comb showing at its top.
            Host::Hollow | Host::Crevice => {
                let back = self.door - self.out;
                vec![(self.door, AIR), (self.door + up, AIR), (back, mat::NEST_HOLLOW), (back + up, mat::HONEYCOMB)]
            }
            // The globe: a stalk where it is fixed to the branch, a body two voxels
            // square and two tall, its paper in bands of grey, and narrowing again to
            // the dark entrance at its foot.
            Host::Branch => {
                let mut out = Vec::with_capacity(10);
                out.push((self.door + IVec3::new(0, 3, 0), mat::WASP_PAPER_DARK));
                for (dy, v) in [(2, mat::WASP_PAPER), (1, mat::WASP_PAPER_DARK)] {
                    for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        out.push((self.door + IVec3::new(dx, dy, dz), v));
                    }
                }
                out.push((self.door, mat::NEST_HOLLOW));
                out
            }
            // A kept hive is built whole with its door.
            Host::Hive => Vec::new(),
        }
    }
}

/// The colony in a kept hive, from what the hive was built of once placed in the world
/// (`voxels` and `fine`, in world voxels and cells): its door is the dark of its
/// entrance (`NEST_HOLLOW`), on the face the entrance looks out of, and the way out is
/// the way that face looks, away from the middle of the hive. None for a hive left
/// empty: grey with age (its wood dead), with no dark entrance, or taken over by wasps.
pub fn in_hive(voxels: &[(IVec3, Voxel)], fine: &[voxel::mesh::PlantBox]) -> Option<Nest> {
    use voxel::mesh::CELLS;
    let empty = |v: Voxel| v == mat::WASP_PAPER_DARK || v == mat::WOOD_DEAD;
    if fine.iter().any(|b| empty(b.voxel)) || voxels.iter().any(|&(_, v)| empty(v)) {
        return None;
    }
    let door = fine.iter().find(|b| b.voxel == mat::NEST_HOLLOW)?;
    let (lo, hi) = (IVec3::from_array(door.lo), IVec3::from_array(door.hi));
    // The middle of the whole hive across the ground, in cells.
    let (mut a, mut b) = (lo, hi);
    for p in fine {
        a = a.min(IVec3::from_array(p.lo));
        b = b.max(IVec3::from_array(p.hi));
    }
    for &(q, _) in voxels {
        a = a.min(q * CELLS);
        b = b.max((q + 1) * CELLS);
    }
    let off = (lo + hi) - (a + b);
    let out = if off.x.abs() >= off.z.abs() { IVec3::new(off.x.signum(), 0, 0) } else { IVec3::new(0, 0, off.z.signum()) };
    if out == IVec3::ZERO {
        return None;
    }
    // The entrance's outer face, and the middle of it across.
    let mid = (lo + hi).div_euclid(IVec3::splat(2));
    let face = IVec3::new(
        if out.x > 0 { hi.x } else if out.x < 0 { lo.x } else { mid.x },
        mid.y,
        if out.z > 0 { hi.z } else if out.z < 0 { lo.z } else { mid.z },
    );
    let door = face.div_euclid(IVec3::splat(CELLS));
    Some(Nest { kind: NestKind::Bees, host: Host::Hive, door, out, host_at: a.div_euclid(IVec3::splat(CELLS)) })
}

/// The trees old and big enough to have a hollow a colony of bees could take: the
/// broadleaf giants, the ancient yews, and the dead.
fn hollow_tree(sp: Species) -> bool {
    matches!(
        sp,
        Species::ElderOak
            | Species::Oak
            | Species::Maple
            | Species::Yew
            | Species::Redwood
            | Species::Baobab
            | Species::Kapok
            | Species::Willow
            | Species::HolmOak
            | Species::DeadSnag
            | Species::Stump
    )
}

/// The trees whose branches wasps hang their nests under: spreading broadleaves with
/// limbs clear of their leaves.
fn wasp_tree(sp: Species) -> bool {
    matches!(sp, Species::Oak | Species::ElderOak | Species::Maple | Species::Cherry | Species::Willow | Species::HolmOak | Species::Acacia)
}

/// Whether bees or wasps live in this country: neither in the cold, nor in the
/// driest deserts, nor on ash.
pub fn lives_in(kind: NestKind, biome: Biome) -> bool {
    match biome {
        Biome::Ocean | Biome::Alpine | Biome::Tundra | Biome::SnowyPine | Biome::Bog | Biome::Desert | Biome::DuneSea | Biome::Volcanic => false,
        // Spruce woods are too cold and dark for a colony to live through the winter in
        // a hollow, but wasps' nests last only the summer.
        Biome::Taiga => kind == NestKind::Wasps,
        _ => true,
    }
}

/// Of the trees fit for one, how many have a colony of bees in a hollow: rare, so that
/// one found is a find, but not so rare that a wood of old oaks lacks one.
pub const HOLLOW_SHARE: f32 = 1.0 / 45.0;
/// Of the boulders big enough, how many have bees in a crevice.
pub const CREVICE_SHARE: f32 = 1.0 / 14.0;
/// Of the trees fit for one, how many have a wasps' nest under a branch.
pub const BRANCH_SHARE: f32 = 1.0 / 55.0;

/// The nest in or under the feature `p`, if it has one. `biome` gives the biome where it
/// stands, asked only of the few features that roll a nest. Depends only on the
/// placement and the seed, never on the chunk asking.
pub fn of(lib: &Library, mats: &Materials, p: &Placement, biome: impl Fn() -> Biome, seed: u32) -> Option<Nest> {
    // Nothing that lies along the ground: a fallen log's hollow is the beetles'.
    if p.lie.is_some() {
        return None;
    }
    let roll = unit(hash3(seed ^ 0x6e_e575, p.pos.x, p.pos.y, p.pos.z));
    let pick = hash3(seed ^ 0xbee5, p.pos.x, 0, p.pos.z);
    let t = lib.get(p.species, p.variant);
    let has = |q: IVec3| t.voxels.binary_search_by_key(&(q.y, q.x, q.z), |(v, _)| (v.y, v.x, v.z)).ok().map(|i| t.voxels[i].1);
    let solid = |q: IVec3| has(q).is_some_and(|v| mats.get(v).kind == Kind::Solid);
    let place = |kind, host, door: IVec3, out: IVec3| Nest {
        kind,
        host,
        door: p.pos + rotate(door, p.turns),
        out: rotate(out, p.turns),
        host_at: p.pos,
    };
    let rock = matches!(p.species, Species::Boulder | Species::MossyBoulder);
    if (hollow_tree(p.species) && roll < HOLLOW_SHARE) || (rock && roll < CREVICE_SHARE) {
        if !lives_in(NestKind::Bees, biome()) {
            return None;
        }
        // Head high or a little over on a tree, low on a rock, facing whichever way the
        // hash says, and the first height up from there where the trunk or the rock is
        // deep enough behind its face to hold a hollow.
        let dirs = [IVec3::X, IVec3::NEG_X, IVec3::Z, IVec3::NEG_Z];
        let out = dirs[(pick % 4) as usize];
        let heights = if rock { 1..(m(1.6) as i32) } else { (m(1.4) as i32)..(m(3.6) as i32) };
        // The model's own axis runs up the middle of its first block.
        let axis = IVec3::new(BLOCK / 2, 0, BLOCK / 2);
        for y in heights {
            let centre = axis + IVec3::Y * y;
            if !solid(centre) {
                continue;
            }
            // Out along the face to its last voxel.
            let mut k = 0;
            while k < 16 && solid(centre + out * (k + 1)) {
                k += 1;
            }
            let door = centre + out * k;
            // Deep enough behind it, standing a voxel taller, and the air in front.
            let deep = (1..=2).all(|d| solid(door - out * d) && solid(door - out * d + IVec3::Y)) && solid(door + IVec3::Y) && solid(door + IVec3::Y * 2);
            if k >= 2 && deep && has(door + out).is_none() && has(door + out + IVec3::Y).is_none() {
                return Some(place(NestKind::Bees, if rock { Host::Crevice } else { Host::Hollow }, door, out));
            }
        }
        return None;
    }
    // The roll for wasps is the other end of the same number, so a tree never has both.
    if wasp_tree(p.species) && roll > 1.0 - BRANCH_SHARE {
        if !lives_in(NestKind::Wasps, biome()) {
            return None;
        }
        // A limb voxel from two metres up to six, out from the trunk, with clear air
        // under it for the globe to hang in and a little below that. The hash picks
        // among them, so the nest is on no particular side.
        let (lo, hi) = (m(2.4) as i32, m(6.0) as i32);
        let reach = m(1.2) as i32;
        // The globe is two voxels square, which a quarter turn may put on any side of
        // the limb voxel, so the air must be clear a voxel all round under it.
        let clear = |q: IVec3| (1..=5).all(|d| (-1..=1).all(|dx| (-1..=1).all(|dz| has(q + IVec3::new(dx, -d, dz)).is_none())));
        let best = t
            .voxels
            .iter()
            .filter(|(q, v)| {
                q.y >= lo + 4 && q.y <= hi + 4 && q.x * q.x + q.z * q.z >= reach * reach && mats.get(*v).kind == Kind::Solid && clear(*q)
            })
            .min_by_key(|(q, _)| hash3(pick, q.x, q.y, q.z))?;
        let door = best.0 - IVec3::Y * 4;
        return Some(place(NestKind::Wasps, Host::Branch, door, IVec3::NEG_Y));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::Library;

    /// Every species placed at many spots: the nests they turn out to have.
    fn nests(lib: &Library, mats: &Materials) -> Vec<(Species, Nest)> {
        let mut out = Vec::new();
        for &species in &Species::ALL {
            for i in 0..4000 {
                let p = Placement {
                    pos: IVec3::new(i * 37 - 70_000, 60, (i * 7919) % 50_000),
                    species,
                    variant: i as usize,
                    turns: (i as u32 / 3) % 4,
                    lie: None,
                    roots: None,
                };
                if let Some(n) = of(lib, mats, &p, || Biome::Deciduous, 11) {
                    out.push((species, n));
                }
            }
        }
        out
    }

    #[test]
    fn bees_live_in_hollows_and_crevices_and_wasps_under_branches() {
        let (lib, mats) = (Library::build(), mat::build());
        let found = nests(&lib, &mats);
        let count = |kind, host| found.iter().filter(|(_, n)| n.kind == kind && n.host == host).count();
        // Enough of each kind turn up, and not too many.
        let (hollows, crevices, branches) = (count(NestKind::Bees, Host::Hollow), count(NestKind::Bees, Host::Crevice), count(NestKind::Wasps, Host::Branch));
        assert!(hollows > 20 && crevices > 20 && branches > 20, "hollows {hollows}, crevices {crevices}, branches {branches}");
        assert_eq!(count(NestKind::Bees, Host::Branch) + count(NestKind::Wasps, Host::Hollow) + count(NestKind::Wasps, Host::Crevice), 0);
        let trees = Species::ALL.iter().filter(|&&s| hollow_tree(s)).count() * 4000;
        assert!((hollows as f32) < trees as f32 * HOLLOW_SHARE * 1.5, "{hollows} hollows in {trees} trees");
        for (species, n) in &found {
            match n.host {
                Host::Hollow => assert!(hollow_tree(*species), "{species:?} has a hollow"),
                Host::Crevice => assert!(matches!(species, Species::Boulder | Species::MossyBoulder)),
                Host::Branch => assert!(wasp_tree(*species), "{species:?} has a wasps' nest"),
                Host::Hive => panic!("a wild nest in a kept hive"),
            }
        }
    }

    #[test]
    fn a_nest_sits_in_its_host_and_opens_to_the_air() {
        let (lib, mats) = (Library::build(), mat::build());
        for (species, n) in nests(&lib, &mats) {
            // The host as the world would have it, before the nest.
            let i = (0..4000).find(|&i| IVec3::new(i * 37 - 70_000, 60, (i * 7919) % 50_000) == n.host_at).unwrap();
            let p = Placement { pos: n.host_at, species, variant: i as usize, turns: (i as u32 / 3) % 4, lie: None, roots: None };
            let host = |q: IVec3| super::super::model_voxel(&lib, &p, q);
            match n.host {
                Host::Hollow | Host::Crevice => {
                    // Cut into the trunk or the rock, with open air in front of it.
                    assert!(host(n.door).is_some_and(|v| mats.get(v).kind == Kind::Solid), "{species:?}: the door is not in the host");
                    assert!(host(n.door + n.out).is_none(), "{species:?}: the door opens into the host");
                    assert_eq!(n.out.y, 0);
                    let v = n.voxels();
                    assert!(v.iter().any(|&(_, v)| v == mat::HONEYCOMB));
                    assert!(v.iter().all(|&(q, _)| host(q).is_some()), "{species:?}: the nest pokes out of its host");
                    // Bees in a tree are above the ground, out of reach of the badgers.
                    if n.host == Host::Hollow {
                        assert!(n.door.y - n.host_at.y >= m(1.4) as i32);
                    }
                }
                Host::Branch => {
                    // Hangs from wood, the globe and the air below it clear of the tree.
                    let top = n.door + IVec3::Y * 4;
                    assert!(host(top).is_some_and(|v| mats.get(v).kind == Kind::Solid), "{species:?}: not under a branch");
                    for (q, _) in n.voxels() {
                        assert!(host(q).is_none(), "{species:?}: the nest is inside the tree at {q}");
                    }
                    assert!(host(n.door - IVec3::Y).is_none());
                    assert!(n.door.y - n.host_at.y >= m(2.4) as i32);
                }
                Host::Hive => panic!("a wild nest in a kept hive"),
            }
        }
    }

    #[test]
    fn nests_keep_to_country_they_can_live_in() {
        assert!(!lives_in(NestKind::Bees, Biome::Tundra) && !lives_in(NestKind::Wasps, Biome::Alpine));
        assert!(lives_in(NestKind::Bees, Biome::Grassland) && lives_in(NestKind::Wasps, Biome::Deciduous));
        let (lib, mats) = (Library::build(), mat::build());
        let p = Placement { pos: IVec3::ZERO, species: Species::Oak, variant: 0, turns: 0, lie: None, roots: None };
        for i in 0..20_000 {
            let p = Placement { pos: IVec3::new(i, 0, 0), ..p };
            assert!(of(&lib, &mats, &p, || Biome::Tundra, 1).is_none());
        }
    }

    /// A kept hive's colony comes and goes by the hive's own entrance, out the way the
    /// hive faces, however it is turned; an empty hive has none.
    #[test]
    fn a_kept_hives_bees_use_its_door() {
        use crate::features::{
            fittings::{Fitting, Hook, build, front},
            landmarks::Palette,
        };
        for biome in [Biome::Deciduous, Biome::Taiga, Biome::Grassland] {
            let p = Palette::of(biome);
            for kind in [Fitting::Skep, Fitting::LogHive, Fitting::BoxHive] {
                for turns in 0..4 {
                    let at = IVec3::new(40, 7, -13);
                    let f = build(kind, &p, 5, false).place(at, turns);
                    let n = in_hive(&f.voxels, &f.fine).unwrap_or_else(|| panic!("{kind:?} turned {turns} has no colony"));
                    let Some(Hook::Hive(h)) = f.hooks.iter().copied().find(|h| matches!(h, Hook::Hive(_))) else { panic!("{kind:?} has no door") };
                    let way = front(turns);
                    assert_eq!(n.out, IVec3::new(way.x, 0, way.y), "{kind:?} turned {turns}");
                    assert!((n.door - h.div_euclid(IVec3::splat(4))).abs().max_element() <= 1, "{kind:?} turned {turns}: door {} for a hook at {h}", n.door);
                    assert_eq!((n.kind, n.host), (NestKind::Bees, Host::Hive));
                }
                let empty = build(kind, &p, 5, true);
                assert!(in_hive(&empty.voxels, &empty.fine).is_none(), "{kind:?} left empty has bees");
            }
        }
    }
}
