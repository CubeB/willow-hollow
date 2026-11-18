//! What stands at a town's edges and in it besides its buildings (#203): gates on its
//! ways in and never on a plot, old walls only round ancient towns and never over a plot,
//! a lane or a path, boat sheds backing onto the water, parks clear of every plot; the
//! same every time and across chunk borders.

use glam::{IVec2, Vec2};
use voxel::{CHUNK, PAD};

use super::{Purpose, special::wall};
use crate::{
    continent::SimConfig,
    features::designs::Special,
    settlements::{Age, Kind, Livelihood},
    surface::Sampler,
    world::GameWorld,
};

fn world(seed: u32) -> GameWorld {
    GameWorld::with_config(seed, SimConfig { size: 256 })
}

/// The worlds looked at: enough small continents that some town on one is ancient,
/// some young, and some by water.
const SEEDS: [u32; 4] = [1, 2, 3, 4];

#[test]
fn gates_stand_on_the_ways_in_and_never_on_a_plot() {
    let mut gates = 0;
    for seed in SEEDS {
        let c = world(seed).continent(0, 0);
        let net = &c.trails;
        for (k, s) in net.settlements.iter().enumerate() {
            let l = &net.layouts[k];
            if s.kind < Kind::Town {
                assert!(l.gates.is_empty(), "seed {seed}: a {:?} has a gate", s.kind);
                continue;
            }
            for g in &l.gates {
                gates += 1;
                assert_eq!(g.which, if s.age == Age::Ancient { Special::Gatehouse } else { Special::Gate }, "seed {seed} settlement {k}");
                // On its way in: the way runs through the middle of its way through.
                let mid = Vec2::new(g.origin.x as f32 + 0.5, g.origin.z as f32 + 0.5);
                let lane = &l.lanes[g.lane];
                let near = lane.iter().map(|p| p.distance(mid)).fold(f32::MAX, f32::min);
                assert!(near < 2.0, "seed {seed} settlement {k}: the gate at {} stands {near} off its way", g.origin);
                assert!(g.lane < l.lanes.len());
                // Off every plot: no building under it, and no pad.
                for p in &net.plots[l.plots.clone()] {
                    let apart = g.hi.x < p.lo.x || g.lo.x > p.hi.x || g.hi.y < p.lo.y || g.lo.y > p.hi.y;
                    assert!(apart, "seed {seed} settlement {k}: the gate at {} is over the {} at {}", g.origin, p.purpose.word(), p.origin);
                    for &(at, r) in &p.pads {
                        let nearest = at.clamp(g.lo.as_vec2(), g.hi.as_vec2() + 1.0);
                        assert!(nearest.distance(at) >= r, "seed {seed} settlement {k}: the gate at {} is on the pad of the {} at {}", g.origin, p.purpose.word(), p.origin);
                    }
                }
                assert!(g.passage % 2 == 1 && g.passage >= special_passage_least());
            }
        }
    }
    assert!(gates > 0, "no town on the test worlds has a gate");
}

/// Every way in a gate is wanted on has its gate: a town's main way in, every way in of
/// a great town, and an ancient town's gatehouse on its main way, whether the way ran
/// square to the world where the gate stands or slantwise (and was laid anew to pass
/// square through it), or crossed water about the ring.
#[test]
fn every_main_way_in_has_its_gate() {
    let mut wanted = 0;
    for seed in 1..=8 {
        let c = world(seed).continent(0, 0);
        let net = &c.trails;
        for (k, s) in net.settlements.iter().enumerate() {
            let l = &net.layouts[k];
            if s.kind < Kind::Town {
                continue;
            }
            for li in 0..l.arrivals {
                if !(li == 0 && l.main || s.kind == Kind::GreatTown) || l.lanes[li].len() < 20 {
                    continue;
                }
                wanted += 1;
                let gate = l.gates.iter().find(|g| g.lane == li);
                assert!(gate.is_some(), "seed {seed}: the {:?} {:?} at {},{} has no gate on its way in {li}", s.age, s.kind, s.x, s.z);
            }
        }
    }
    assert!(wanted >= 4, "only {wanted} ways in want a gate on the test worlds");
}

fn special_passage_least() -> i32 {
    crate::settlements::special::gate::PASSAGE
}

#[test]
fn old_walls_stand_only_round_ancient_towns_clear_of_plots_lanes_and_paths() {
    let mut pieces = 0;
    for seed in SEEDS {
        let c = world(seed).continent(0, 0);
        let net = &c.trails;
        let points: Vec<(Vec2, f32)> = net.points().filter(|p| p.half_width > 0.0).map(|p| (Vec2::new(p.x, p.z), p.half_width)).collect();
        for (k, s) in net.settlements.iter().enumerate() {
            let l = &net.layouts[k];
            if s.age != Age::Ancient || s.kind < Kind::Town {
                assert!(l.wall.is_empty(), "seed {seed}: a {:?} {:?} has an old wall", s.age, s.kind);
                continue;
            }
            for line in &l.wall {
                pieces += 1;
                for &c in line {
                    for dz in -wall::HALF..=wall::HALF {
                        for dx in -wall::HALF..=wall::HALF {
                            let q = c + IVec2::new(dx, dz);
                            let p = q.as_vec2() + 0.5;
                            for plot in &net.plots[l.plots.clone()] {
                                assert!(!(q.cmpge(plot.lo).all() && q.cmple(plot.hi).all()), "seed {seed} settlement {k}: the old wall at {q} cuts the {} at {}", plot.purpose.word(), plot.origin);
                                for &(at, r) in &plot.pads {
                                    assert!(at.distance(p) >= r - 0.5, "seed {seed} settlement {k}: the old wall at {q} is on the plot of the {} at {}", plot.purpose.word(), plot.origin);
                                }
                            }
                            for &(at, hw) in &points {
                                assert!(at.distance(p) >= hw, "seed {seed} settlement {k}: the old wall at {q} is on a way at {at}");
                            }
                        }
                    }
                }
            }
        }
    }
    println!("{pieces} pieces of old wall");
}

#[test]
fn boat_sheds_back_onto_the_water() {
    let mut sheds = 0;
    for seed in SEEDS {
        let w = world(seed);
        let c = w.continent(0, 0);
        let net = &c.trails;
        for (k, s) in net.settlements.iter().enumerate() {
            let l = &net.layouts[k];
            if !l.sheds.is_empty() {
                assert!(s.has(Livelihood::Fisher) || s.has(Livelihood::Port));
            }
            for shed in &l.sheds {
                sheds += 1;
                let plot = net.plots[l.plots.clone()].iter().find(|p| p.origin == shed.origin).expect("a shed's plot");
                assert_eq!(plot.purpose, Purpose::Workplace(Livelihood::Fisher));
                assert_eq!(plot.sleepers, 2);
                // Its back is the side away from its door, and the slipway runs out that way
                // to the water.
                assert_eq!(shed.out, -plot.facing, "seed {seed}: a shed's slipway is not at its back");
                let sam = Sampler::with_network(&c, w.seed(), shed.slip.x - 8, shed.slip.z - 8, 17, net);
                let wet = (-2..=3).any(|d| {
                    let q = IVec2::new(shed.slip.x, shed.slip.z) + shed.out * d;
                    let col = sam.column(q.x, q.y);
                    col.water_surface().is_some() || col.river < 0.0
                });
                assert!(wet, "seed {seed} settlement {k}: the shed at {} has no water at the foot of its slipway {}", shed.origin, shed.slip);
            }
        }
    }
    println!("{sheds} boat sheds");
}

/// A harbour wall shelters the town's one landing, which is its fishing quay too, and no
/// boat putting out from there to its fishing grounds goes through the wall.
#[test]
fn harbour_walls_shelter_the_landing_the_boats_put_out_from() {
    let (mut walls, mut grounds) = (0, 0);
    for seed in [1, 2, 3, 4, 5, 6, 7, 8] {
        let c = world(seed).continent(0, 0);
        let net = &c.trails;
        for (k, s) in net.settlements.iter().enumerate() {
            let Some(h) = &net.layouts[k].harbour else { continue };
            walls += 1;
            assert_eq!(s.kind, Kind::GreatTown);
            assert_eq!(Some(h.landing), s.port, "seed {seed}: the harbour wall shelters another landing");
            let wall: std::collections::HashSet<IVec2> = h.line.iter().flat_map(|&c| (-3..=3).flat_map(move |z| (-3..=3).map(move |x| c + IVec2::new(x, z)))).collect();
            let ways = &net.waterways;
            // The wall stands up out of the sea, higher than the landing's boards, and the
            // quay along the shore climbs to it.
            assert!(h.top > net.sites[h.landing].y, "seed {seed}: the harbour wall's top is no higher than the landing");
            for (q, quay) in ways.quays.iter().enumerate().filter(|(_, q)| q.settlement == k) {
                assert!(quay.footprint.iter().all(|c| !wall.contains(c)), "seed {seed}: a jetty stands on the harbour wall");
                let from = Vec2::new(quay.berth.x, quay.berth.z);
                for g in ways.grounds_of(q) {
                    grounds += 1;
                    let to = Vec2::new(g.at.x, g.at.z);
                    let n = (from.distance(to) * 2.0) as i32;
                    for i in 0..=n {
                        let p = from.lerp(to, i as f32 / n.max(1) as f32);
                        // Clear of the wall's breadth and parapet, of its round head, and
                        // of the quay along the shore.
                        let near = |cols: &[IVec2]| cols.iter().map(|c| (c.as_vec2() + 0.5).distance(p)).fold(f32::MAX, f32::min);
                        let head = (h.line[h.line.len() - 1].as_vec2() + 0.5).distance(p);
                        assert!(near(&h.line) > 3.5 && head > 5.0 && near(&h.quay) > 2.0, "seed {seed}: a boat from the landing at {from} to its ground at {to} goes over the harbour wall or its quay at {p}");
                    }
                }
            }
        }
    }
    println!("{walls} harbour walls, {grounds} fishing grounds out from them");
}

#[test]
fn parks_are_clear_of_every_plot() {
    for seed in SEEDS {
        let c = world(seed).continent(0, 0);
        let net = &c.trails;
        for (k, s) in net.settlements.iter().enumerate() {
            let l = &net.layouts[k];
            let Some(park) = &l.park else { continue };
            assert_eq!(s.kind, Kind::GreatTown, "seed {seed}: a {:?} has a park", s.kind);
            for p in &net.plots[l.plots.clone()] {
                let nearest = park.at.clamp(p.lo.as_vec2(), p.hi.as_vec2() + 1.0);
                assert!(nearest.distance(park.at) >= park.radius, "seed {seed} settlement {k}: the park is over the {} at {}", p.purpose.word(), p.origin);
            }
            if let Some((pond, _)) = park.pond {
                assert!(Vec2::new(pond.x as f32, pond.z as f32).distance(park.at) < park.radius);
            }
        }
    }
}

#[test]
fn edges_are_laid_out_the_same_every_time() {
    let key = |seed: u32| {
        let c = world(seed).continent(0, 0);
        c.trails
            .layouts
            .iter()
            .map(|l| {
                let gates: Vec<_> = l.gates.iter().map(|g| (g.origin, g.turns, g.passage, g.lo, g.hi)).collect();
                let sheds: Vec<_> = l.sheds.iter().map(|s| (s.origin, s.slip)).collect();
                (gates, l.wall.clone(), sheds, l.harbour.as_ref().map(|h| h.line.clone()), l.park.as_ref().map(|p| (p.pond, p.pavilion, p.lawn, p.benches.clone())))
            })
            .collect::<Vec<_>>()
    };
    for seed in [1, 3] {
        assert!(key(seed) == key(seed), "seed {seed}: laid out differently");
    }
}

#[test]
fn gates_and_walls_agree_across_chunk_borders() {
    let mut compared = 0;
    for seed in SEEDS {
        let w = world(seed);
        let c = w.continent(0, 0);
        let net = &c.trails;
        let mut spans: Vec<(IVec2, IVec2)> = net.layouts.iter().flat_map(|l| l.gates.iter().map(|g| (g.lo, g.hi))).collect();
        spans.extend(net.layouts.iter().flat_map(|l| l.wall.iter().map(|line| (line[0] - IVec2::splat(2), line[0] + IVec2::splat(2)))));
        for (lo, hi) in spans {
            let (cx0, cx1) = (lo.x.div_euclid(CHUNK), hi.x.div_euclid(CHUNK));
            let (cz0, cz1) = (lo.y.div_euclid(CHUNK), hi.y.div_euclid(CHUNK));
            let pair = if cx0 != cx1 {
                ((cx0, cz0), (cx0 + 1, cz0), true)
            } else if cz0 != cz1 {
                ((cx0, cz0), (cx0, cz0 + 1), false)
            } else {
                continue;
            };
            let (a, b) = (w.generate_column_bare(pair.0.0, pair.0.1).chunks, w.generate_column_bare(pair.1.0, pair.1.1).chunks);
            for (cy, ca) in &a {
                let Some((_, cb)) = b.iter().find(|e| e.0 == *cy) else { continue };
                for (i, j) in [(PAD - 2, 0), (PAD - 1, 1)] {
                    for y in 0..PAD {
                        for o in 0..PAD {
                            let (va, vb) = if pair.2 { (ca.get(i, y, o), cb.get(j, y, o)) } else { (ca.get(o, y, i), cb.get(o, y, j)) };
                            assert_eq!(va, vb, "seed {seed}: chunks {:?} and {:?} differ at {i} y {y} o {o} cy {cy}", pair.0, pair.1);
                        }
                    }
                }
            }
            compared += 1;
            if compared >= 6 {
                return;
            }
        }
    }
    assert!(compared > 0, "no gate or wall straddles a chunk border");
}

/// Lists every town of a full-size world (seed from `SEED`, 1 if unset) with its gates,
/// old wall, boat sheds, harbour and park, for aiming screenshots: `cargo test -p
/// worldgen list_edges -- --ignored --nocapture`.
#[test]
#[ignore]
fn list_edges() {
    let seed = std::env::var("SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
    let w = GameWorld::new(seed);
    let c = w.continent(0, 0);
    let net = &c.trails;
    for (k, s) in net.settlements.iter().enumerate() {
        let l = &net.layouts[k];
        if s.kind < Kind::Town && l.gates.is_empty() && l.wall.is_empty() && l.sheds.is_empty() && l.harbour.is_none() && l.park.is_none() {
            continue;
        }
        println!("{k} {:?} {:?} {:?} at {},{} y {} r {:.0} coast {} port {:?}, {} lanes", s.kind, s.region, s.age, s.x, s.z, s.y, s.radius, s.coast, s.port, l.lanes.len());
        for g in &l.gates {
            println!("    {:?} at {} out {} passage {} lane {}", g.which, g.origin, g.out, g.passage, g.lane);
        }
        for line in &l.wall {
            println!("    wall of {} from {} to {}", line.len(), line[0], line[line.len() - 1]);
        }
        for sh in &l.sheds {
            let slip = Vec2::new(sh.slip.x as f32, sh.slip.z as f32);
            let quay = net.waterways.quays.iter().filter(|q| q.settlement == k).map(|q| q.foot.distance(slip).min(q.head.distance(slip))).fold(f32::MAX, f32::min);
            println!("    boat shed at {} slip {} out {}, the nearest quay {quay:.0} off", sh.origin, sh.slip, sh.out);
        }
        if let Some(h) = &l.harbour {
            let site = &net.sites[h.landing];
            println!("    harbour wall of {} from {} to beacon {}, sheltering landing {} at {},{} moored at {:?}, quay of {}", h.line.len(), h.root, h.beacon, h.landing, site.x, site.z, site.mooring.map(|m| m.pos), h.quay.len());
            for (q, quay) in net.waterways.quays.iter().enumerate().filter(|(_, q)| q.settlement == k) {
                let from = Vec2::new(quay.berth.x, quay.berth.z);
                let through = net.waterways.grounds_of(q).filter(|g| (0..=200).any(|i| h.line.contains(&from.lerp(Vec2::new(g.at.x, g.at.z), i as f32 / 200.0).floor().as_ivec2()))).count();
                println!("    quay {q} {:?} landing {:?} berth {}: {} grounds, {through} through the wall", quay.serves, quay.landing, quay.berth, net.waterways.grounds_of(q).count());
            }
        }
        if let Some(p) = &l.park {
            println!("    park at {} r {:.0}: pond {:?} pavilion {:?} lawn {:?} benches {}", p.at, p.radius, p.pond, p.pavilion, p.lawn, p.benches.len());
        }
    }
}
