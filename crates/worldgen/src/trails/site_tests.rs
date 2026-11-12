//! Lived-in landmarks (#67): each sited for a reason, terraced into its slope, cleared
//! about, worn where people walk, its door turned to the way people come, and joined to
//! the trails.

use std::collections::HashMap;

use glam::{IVec3, vec2};
use voxel::{AIR, CHUNK};

use super::{CLEARING, Network, Pad, Reason};
use crate::{
    continent::{Continent, SimConfig},
    features::{self, landmarks::Landmark, trees::Species},
    scale::m,
    surface::{Cover, Sampler, TERRACE_RING},
    world::GameWorld,
};

fn world() -> GameWorld {
    GameWorld::with_config(3, SimConfig { size: 256 })
}

/// Each site with its pad.
fn sites(net: &Network) -> impl Iterator<Item = (&super::Site, &Pad)> {
    net.sites.iter().zip(&net.pads)
}

/// The columns of the ring just outside a pad where its terrace steps, with the ground
/// there and the highest ground beside it.
fn steps(c: &Continent, seed: u32, pad: &Pad) -> Vec<(i32, i32, i32, i32)> {
    let reach = (pad.radius + TERRACE_RING).ceil() as i32 + 1;
    let (cx, cz) = (pad.x.floor() as i32, pad.z.floor() as i32);
    let s = Sampler::new(c, seed, cx - reach - 1, cz - reach - 1, 2 * reach + 3);
    let mut out = Vec::new();
    for z in cz - reach..=cz + reach {
        for x in cx - reach..=cx + reach {
            let d = vec2(x as f32 + 0.5, z as f32 + 0.5).distance(vec2(pad.x, pad.z)) - pad.radius;
            let col = s.column(x, z);
            if !(0.0..TERRACE_RING).contains(&d) || col.water_surface().is_some() {
                continue;
            }
            let top = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .iter()
                .map(|&(dx, dz)| s.column(x + dx, z + dz))
                .filter(|n| n.water_surface().is_none())
                .map(|n| n.top())
                .fold(col.top(), i32::max);
            if top - col.top() >= 2 {
                out.push((x, z, col.top(), top));
            }
        }
    }
    out
}

#[test]
fn landmarks_stand_where_they_do_for_a_reason() {
    // At the game's own resolution: on the coarse test grid a map cell is wider than
    // the ground a site looks over to settle, so the water it was chosen for can lie
    // out of its sight.
    let w = GameWorld::with_config(3, SimConfig { size: 1024 });
    let c = w.continent(0, 0);
    let net = &c.trails;
    let land: Vec<_> = net.sites.iter().filter(|s| s.kind != Landmark::Landing).collect();
    let mut reasons: HashMap<Reason, usize> = HashMap::new();
    for s in &land {
        *reasons.entry(s.reason).or_default() += 1;
    }
    println!("{} land sites: {reasons:?}", land.len());
    let mut open_kinds: HashMap<Landmark, usize> = HashMap::new();
    for s in land.iter().filter(|s| s.reason == Reason::Open) {
        *open_kinds.entry(s.kind).or_default() += 1;
    }
    println!("open: {open_kinds:?}");
    let open = reasons.get(&Reason::Open).copied().unwrap_or(0);
    assert!(open * 3 < land.len(), "{open} of {} sites stand where they do for no reason", land.len());
    assert!(net.sites.iter().filter(|s| s.kind == Landmark::Landing).all(|s| s.reason == Reason::Shore));

    for s in &land {
        let sampler = Sampler::new(&c, w.seed(), s.x - m(50.0) as i32, s.z - m(50.0) as i32, m(100.0) as i32);
        let ground = |dx: f32, dz: f32| sampler.column(s.x + dx as i32, s.z + dz as i32).ground;
        match s.reason {
            // A view: the land falls away about it.
            Reason::View => {
                let around: f32 = (0..8)
                    .map(|k| {
                        let a = k as f32 * std::f32::consts::FRAC_PI_4;
                        ground(a.cos() * m(30.0), a.sin() * m(30.0))
                    })
                    .sum::<f32>()
                    / 8.0;
                assert!(around < s.y as f32, "{:?} at {},{} has no view: ground about it {around:.0}, it {}", s.kind, s.x, s.z, s.y);
            }
            // Water: some within a short walk.
            Reason::Water | Reason::Spring | Reason::Ford => {
                let near = (0..=40).any(|i| {
                    (0..=40).any(|j| {
                        let col = sampler.column(s.x - m(40.0) as i32 + i * 5, s.z - m(40.0) as i32 + j * 5);
                        col.water_surface().is_some_and(|k| k > 0) || col.river < m(2.0)
                    })
                }) || sampler.springs().iter().any(|sp| sp.pos.distance(vec2(s.x as f32, s.z as f32)) < m(45.0))
                    || sampler.heads().iter().any(|h| h.distance(vec2(s.x as f32, s.z as f32)) < m(45.0));
                assert!(near, "{:?} at {},{} stands for {:?} with none near", s.kind, s.x, s.z, s.reason);
            }
            _ => {}
        }
    }
    // Lookouts are there for the view.
    for s in land.iter().filter(|s| s.kind == Landmark::Lookout) {
        assert_eq!(s.reason, Reason::View);
    }
}

#[test]
fn landmark_layout_is_repeatable() {
    let (a, b) = (world(), world());
    let (ca, cb) = (a.continent(0, 0), b.continent(0, 0));
    assert_eq!(ca.trails.pads.len(), cb.trails.pads.len());
    for (p, q) in ca.trails.pads.iter().zip(&cb.trails.pads) {
        assert_eq!((p.x, p.z, p.y, p.wear, p.neglect, p.arrival_count), (q.x, q.z, q.y, q.wear, q.neglect, q.arrival_count));
        assert_eq!(p.front.to_bits(), q.front.to_bits());
        assert_eq!(p.arrivals, q.arrivals);
    }
    for (s, t) in ca.trails.sites.iter().zip(&cb.trails.sites) {
        assert_eq!(s.reason, t.reason);
    }
}

#[test]
fn doors_face_the_way_people_come() {
    let w = world();
    let c = w.continent(0, 0);
    let mut checked = 0;
    for (s, pad) in sites(&c.trails) {
        if pad.front.is_nan() || pad.arrival_count == 0 {
            continue;
        }
        let off = |a: f32| ((a - pad.front + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI).abs();
        let best = pad.arrivals[..pad.arrival_count as usize].iter().map(|&a| off(a)).fold(f32::MAX, f32::min);
        assert!(best <= std::f32::consts::FRAC_PI_4 + 0.01, "{:?} at {},{}: door {:.2} off every way in", s.kind, s.x, s.z, best);
        checked += 1;
    }
    println!("{checked} doors");
    assert!(checked >= 5);
}

#[test]
fn trails_stop_at_the_way_round_a_building() {
    let w = world();
    let c = w.continent(0, 0);
    let paved: Vec<_> = c.trails.points().filter(|p| p.half_width > 0.0).collect();
    let mut checked = 0;
    for (s, pad) in sites(&c.trails) {
        if !matches!(s.kind, Landmark::Lookout | Landmark::Hut) {
            continue;
        }
        for p in &paved {
            let d = vec2(p.x, p.z).distance(vec2(pad.x, pad.z));
            assert!(d >= pad.ring - 0.5, "a trail runs into the {:?} at {},{}, {d:.1} from its middle", s.kind, s.x, s.z);
        }
        checked += 1;
    }
    assert!(checked > 0);
}

#[test]
fn terraces_are_held_by_walls_that_meet_the_ground() {
    let w = world();
    let c = w.continent(0, 0);
    let (mut walled, mut columns) = (0, 0);
    for (s, pad) in sites(&c.trails) {
        if !pad.walled {
            continue;
        }
        let steps = steps(&c, w.seed(), pad);
        if steps.is_empty() {
            continue;
        }
        walled += 1;
        let mut stamped: HashMap<IVec3, voxel::Voxel> = HashMap::new();
        for &(x, z, _, _) in &steps {
            for st in c.trails.stamps_at(x.div_euclid(CHUNK), z.div_euclid(CHUNK)) {
                for &(p, v) in &st.voxels {
                    if p.x == x && p.z == z {
                        stamped.insert(p, v);
                    }
                }
            }
        }
        for &(x, z, g, top) in &steps {
            columns += 1;
            // From the ground's own top up to a voxel short of the higher side (a fallen
            // coping can take the last) is stone, with no gap.
            for y in g..top {
                let v = stamped.get(&IVec3::new(x, y, z)).copied().unwrap_or(AIR);
                assert_ne!(v, AIR, "{:?} at {},{}: the terrace wall at {x},{z} has a gap at {y} (ground {g}, top {top})", s.kind, s.x, s.z);
            }
        }
    }
    println!("{walled} terraced sites, {columns} columns of wall");
    assert!(walled >= 3, "only {walled} sites are terraced");
}

#[test]
fn pads_are_level_and_agree_across_chunk_borders() {
    let w = world();
    let c = w.continent(0, 0);
    let (mut level, mut total, mut compared) = (0, 0, 0);
    for (_, pad) in sites(&c.trails).filter(|(_, p)| p.walled) {
        let (cx, cz) = (pad.x.floor() as i32, pad.z.floor() as i32);
        let r = pad.radius as i32 + 3;
        let s = Sampler::new(&c, w.seed(), cx - r, cz - r, 2 * r + 1);
        for z in cz - r..=cz + r {
            for x in cx - r..=cx + r {
                let d = vec2(x as f32 + 0.5, z as f32 + 0.5).distance(vec2(pad.x, pad.z)) - pad.radius;
                if d < -1.0 {
                    total += 1;
                    level += ((s.column(x, z).top() - pad.y.floor() as i32).abs() <= 1) as i32;
                }
            }
        }
        // Across the borders of the chunks either side of the middle, worn ground and
        // terrace edge alike read the same from either chunk.
        let border = cx.div_euclid(CHUNK) * CHUNK;
        for z in (cz - r..=cz + r).step_by(3) {
            let a = Sampler::new(&c, w.seed(), border - CHUNK, z - 1, CHUNK + 2);
            let b = Sampler::new(&c, w.seed(), border - 1, z - 1, CHUNK + 2);
            for x in [border - 1, border] {
                let (p, q) = (a.column(x, z), b.column(x, z));
                assert_eq!(p.ground, q.ground, "ground differs at {x},{z}");
                assert_eq!(p.cover, q.cover, "cover differs at {x},{z}");
                compared += 1;
            }
        }
    }
    println!("{level} of {total} pad columns level, {compared} compared across borders");
    assert!(level as f32 >= total as f32 * 0.97, "only {level} of {total} pad columns are level");
    assert!(compared > 50);
}

#[test]
fn ground_is_worn_where_a_place_is_used() {
    let w = world();
    let c = w.continent(0, 0);
    // Share of each pad's ground worn bare, beside how much it is used.
    let mut shares = Vec::new();
    for (s, pad) in sites(&c.trails).filter(|(_, p)| p.walled) {
        let (cx, cz) = (pad.x.floor() as i32, pad.z.floor() as i32);
        let r = pad.radius as i32;
        let sampler = Sampler::new(&c, w.seed(), cx - r, cz - r, 2 * r + 1);
        // Leaving out the trails themselves, which run on into some sites.
        let paved: Vec<_> = c
            .trails
            .points()
            .filter(|t| t.half_width > 0.0 && vec2(t.x, t.z).distance(vec2(pad.x, pad.z)) < pad.radius + m(4.0))
            .collect();
        let (mut worn, mut all) = (0, 0);
        for z in cz - r..=cz + r {
            for x in cx - r..=cx + r {
                let p = vec2(x as f32 + 0.5, z as f32 + 0.5);
                let on_trail = paved.iter().any(|t| vec2(t.x, t.z).distance(p) < t.half_width + m(1.0));
                if p.distance(vec2(pad.x, pad.z)) < pad.radius - 1.0 && !on_trail {
                    all += 1;
                    worn += (sampler.column(x, z).cover == Cover::Trail) as i32;
                }
            }
        }
        shares.push((s.kind, pad.wear, worn as f32 / all.max(1) as f32));
    }
    let mean = |lo: f32, hi: f32| {
        let v: Vec<f32> = shares.iter().filter(|s| (lo..hi).contains(&s.1)).map(|s| s.2).collect();
        (v.iter().sum::<f32>() / v.len().max(1) as f32, v.len())
    };
    let (busy, nb) = mean(0.6, 1.1);
    let (idle, ni) = mean(0.0, 0.05);
    println!("worn share: {busy:.3} over {nb} busy sites, {idle:.3} over {ni} idle");
    assert!(nb > 0 && ni > 0);
    assert!(busy > 0.04, "busy sites are hardly worn: {busy}");
    assert!(busy > idle * 3.0, "busy {busy} and idle {idle} sites are worn alike");
}

#[test]
fn trees_are_felled_about_kept_sites_and_grow_back_about_old_ones() {
    let w = world();
    let c = w.continent(0, 0);
    let is_tree = |s: Species| {
        !matches!(
            s,
            Species::Stump | Species::Bush | Species::Fern | Species::Boulder | Species::MossyBoulder | Species::FallenLog | Species::HollyBush | Species::Laurel | Species::Gorse | Species::Juniper
        )
    };
    // Trees per kept site, near (within the inner half of the clearing) and further out,
    // by area; and the stumps and young growth near each kind of site.
    let (mut near, mut far, mut near_area, mut far_area) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let (mut stumps, mut young_old, mut young_kept) = (0, 0, 0);
    for (_, pad) in sites(&c.trails).filter(|(_, p)| p.walled) {
        let (r_in, r_out) = (pad.radius + CLEARING * 0.5, pad.radius + CLEARING * 2.0);
        let (x, z) = (pad.x as i32, pad.z as i32);
        let span = r_out.ceil() as i32;
        let margin = features::MAX_REACH + 1;
        let s = Sampler::new(&c, w.seed(), x - span - margin, z - span - margin, 2 * (span + margin));
        let kept = pad.neglect < 0.3;
        for placed in features::place(w.seed(), w.library(), &s, x - span, z - span, 2 * span, (0, 0)) {
            let d = vec2(placed.pos.x as f32, placed.pos.z as f32).distance(vec2(pad.x, pad.z));
            if d < r_in {
                match placed.species {
                    Species::Stump if kept => stumps += 1,
                    Species::Bush if pad.neglect > 0.6 => young_old += 1,
                    Species::Bush if kept => young_kept += 1,
                    sp if kept && is_tree(sp) => near += 1.0,
                    _ => {}
                }
            } else if d < r_out && kept && is_tree(placed.species) {
                far += 1.0;
            }
        }
        if kept {
            let ring = |a: f32, b: f32| std::f32::consts::PI * (b * b - a * a);
            near_area += ring(pad.radius, r_in);
            far_area += ring(r_in, r_out);
        }
    }
    let (near_d, far_d) = (near / near_area.max(1.0) * 1e4, far / far_area.max(1.0) * 1e4);
    println!("trees per 10^4 columns: {near_d:.2} near kept sites, {far_d:.2} further out; {stumps} stumps; young growth {young_old} about old sites, {young_kept} about kept ones");
    assert!(far > 20.0, "too few trees about the sites to judge: {far}");
    assert!(near_d < far_d * 0.6, "trees stand as thick by kept sites ({near_d:.2}) as further out ({far_d:.2})");
    assert!(stumps > 0, "no stumps left about kept sites");
}
