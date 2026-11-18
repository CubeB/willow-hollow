use voxel::{CHUNK, PAD};

use crate::{
    World,
    continent::SimConfig,
    macro_layout::MACRO_SIZE,
    materials as mat,
    scale::BLOCK,
    surface::{MIN_RIVER_Q, Sampler, node},
    world::GameWorld,
};

/// A smaller simulation grid keeps these quick. Cells are then 128 voxels across, but
/// everything under test works the same way.
fn world() -> GameWorld {
    GameWorld::with_config(3, SimConfig { size: 256 })
}

/// Chunk columns that contain rivers, found by walking the drainage network.
fn river_chunks(w: &GameWorld, count: usize) -> Vec<(i32, i32)> {
    let c = w.continent(0, 0);
    let mut cells: Vec<usize> = (0..c.size * c.size)
        .filter(|&i| !c.ocean[i] && c.discharge[i] >= MIN_RIVER_Q * 4.0)
        .collect();
    cells.sort_by(|&a, &b| c.discharge[b].total_cmp(&c.discharge[a]));
    assert!(cells.len() > count, "test continent has too few rivers");
    // Spread the sample over rivers of every size.
    let step = cells.len() / count;
    cells
        .iter()
        .step_by(step)
        .take(count)
        .map(|&i| {
            // A river's curve bends around its nodes but always passes exactly halfway
            // between one and the next.
            let p = (node(&c, i) + node(&c, c.rcv[i] as usize)) * 0.5;
            (p.x as i32 / CHUNK, p.y as i32 / CHUNK)
        })
        .collect()
}

#[test]
fn neighbouring_chunks_agree_on_their_shared_border() {
    // Each chunk samples its own footprint plus a one-voxel border, using its own
    // list of nearby river pieces. Two chunks must still compute identical columns
    // where their footprints overlap, or seams would open between them.
    let w = world();
    let c = w.continent(0, 0);
    for (cx, cz) in river_chunks(&w, 24) {
        let a0 = (cx * CHUNK - 1, cz * CHUNK - 1);
        let b0 = ((cx + 1) * CHUNK - 1, cz * CHUNK - 1);
        let a = Sampler::new(&c, w.seed(), a0.0, a0.1, PAD as i32);
        let b = Sampler::new(&c, w.seed(), b0.0, b0.1, PAD as i32);
        // Columns x = b0.0 and b0.0 + 1 are in both footprints.
        for lx in [b0.0, b0.0 + 1] {
            for lz in a0.1..a0.1 + PAD as i32 {
                let (ca, cb) = (a.column(lx, lz), b.column(lx, lz));
                assert_eq!(ca.ground, cb.ground, "ground differs at ({lx}, {lz})");
                assert_eq!(ca.water, cb.water, "water differs at ({lx}, {lz})");
                assert_eq!(ca.cover, cb.cover);
                let single = w.column(lx, lz);
                assert_eq!(single.ground, ca.ground, "single-column sample differs");
                assert_eq!(single.water, ca.water);
            }
        }
    }
}

#[test]
fn ground_is_continuous_across_a_range_border() {
    // Find a seed and border where a range crosses, then walk the voxels either side.
    let found = (0u32..64).find_map(|seed| {
        (-6..6).find_map(|mx| {
            let field = crate::structures::Field::new(seed, mx, 0);
            (1..16)
                .find(|k| field.land(1.0, -1.0 + *k as f32 / 8.0).is_some_and(|l| l > 0.3))
                .map(|k| (seed, mx, k))
        })
    });
    let (seed, mx, k) = found.expect("no border-crossing range in the seeds scanned");
    let w = GameWorld::with_config(seed, SimConfig { size: 256 });
    let border_x = (mx + 1) * MACRO_SIZE;
    let z0 = (k * MACRO_SIZE) / 16;
    let mut largest = 0.0f32;
    let mut land = 0;
    for z in (z0 - 3000..z0 + 3000).step_by(40) {
        let (a, b) = (w.column(border_x - 1, z), w.column(border_x, z));
        if a.ground > 0.0 {
            land += 1;
        }
        largest = largest.max((a.ground - b.ground).abs());
    }
    assert!(land > 0, "no land at the sampled border");
    // Neighbouring columns differ by normal fine terrain at most, never a cell-wide step.
    assert!(largest < 6.0, "step of {largest} voxels across the border");
}

#[test]
fn water_is_held_in_by_its_banks() {
    // Wherever a column holds water, each neighbour must either hold water too (a lower
    // level there is a step in the river: a cascade) or have ground reaching the
    // water's surface. Anything else is a leak: water standing against open air.
    let w = world();
    let c = w.continent(0, 0);
    let (mut wet, mut leaks, mut cascades) = (0u64, 0u64, 0u64);
    for (cx, cz) in river_chunks(&w, 40) {
        let (x0, z0) = (cx * CHUNK - 1, cz * CHUNK - 1);
        let s = Sampler::new(&c, w.seed(), x0, z0, PAD as i32);
        let cols: Vec<_> = (0..PAD as i32)
            .flat_map(|dx| (0..PAD as i32).map(move |dz| (dx, dz)))
            .map(|(dx, dz)| s.column(x0 + dx, z0 + dz))
            .collect();
        let at = |dx: i32, dz: i32| &cols[(dx * PAD as i32 + dz) as usize];
        for dx in 1..PAD as i32 - 1 {
            for dz in 1..PAD as i32 - 1 {
                let Some(k) = at(dx, dz).water_surface() else { continue };
                wet += 1;
                for (nx, nz) in [(dx + 1, dz), (dx - 1, dz), (dx, dz + 1), (dx, dz - 1)] {
                    let n = at(nx, nz);
                    match n.water_surface() {
                        Some(nk) if nk < k => cascades += 1,
                        Some(_) => {}
                        // Ground top at k - 1 is flush with the water surface plane.
                        None if n.top() >= k - 1 => {}
                        None => leaks += 1,
                    }
                }
            }
        }
    }
    assert!(wet > 2000, "sampled only {wet} water columns");
    assert!(cascades > 0, "no steps found in any river");
    let rate = leaks as f64 / wet as f64;
    println!("{wet} water columns, {cascades} cascade edges, {leaks} leaks ({:.3}%)", rate * 100.0);
    assert!(rate < 0.002, "{leaks} leaks in {wet} water columns");
}

#[test]
fn chunks_are_generated_only_where_there_is_a_surface() {
    let w = world();
    let (cx, cz) = river_chunks(&w, 4)[1];
    let chunks = w.generate_column(cx, cz).chunks;
    assert!(!chunks.is_empty() && chunks.len() <= 8 * BLOCK as usize, "{} chunks", chunks.len());
    let has_water = chunks
        .iter()
        .any(|(_, chunk)| chunk.voxels().contains(&mat::WATER));
    assert!(has_water, "river chunk column contains no water");
    // Lowest first with no gaps, as collision lookups rely on.
    for pair in chunks.windows(2) {
        assert_eq!(pair[1].0, pair[0].0 + 1);
    }
    // Far out to sea: a single chunk holding the sea bed and surface.
    let sea = w.generate_column(2, MACRO_SIZE / CHUNK / 2).chunks;
    assert!(sea.len() <= 4 * BLOCK as usize);
    assert!(sea.iter().all(|(_, chunk)| chunk.voxels().contains(&mat::WATER)));
}

#[test]
fn spawn_is_on_dry_land_beside_a_river() {
    let w = world();
    let spawn = w.spawn_point();
    let here = w.column(spawn.x, spawn.z);
    assert!(here.water_surface().is_none(), "spawn is in water");
    assert!(here.top() >= 0);
    // Water within a short walk, in the direction the player faces.
    let (dx, dz) = (spawn.yaw.to_radians().cos(), spawn.yaw.to_radians().sin());
    let found = (1..80).any(|d| {
        let col = w.column(spawn.x + (dx * d as f32) as i32, spawn.z + (dz * d as f32) as i32);
        col.water_surface().is_some()
    });
    assert!(found, "no water ahead of the spawn point");
}

#[test]
fn rocky_shores_have_surf() {
    let w = world();
    let c = w.continent(0, 0);
    // Coastal cells: sea with land beside it.
    let coast: Vec<usize> = (c.size + 1..c.size * (c.size - 1) - 1)
        .filter(|&i| c.ocean[i] && !c.ocean[i + 1])
        .collect();
    assert!(!coast.is_empty());
    let cs = c.cell_size();
    let (mut found, mut checked) = (0, 0);
    for &i in coast.iter().step_by((coast.len() / 150).max(1)) {
        let p = ((i % c.size) as f32 + 1.0) * cs;
        let (cx, cz) = (p as i32 / CHUNK, ((i / c.size) as f32 + 0.5) as i32 * cs as i32 / CHUNK);
        let out = w.generate_column(cx, cz);
        checked += 1;
        assert!(out.surf.len() <= 8);
        if !out.surf.is_empty() {
            found += 1;
        }
    }
    eprintln!("surf in {found} of {checked} coastal chunk columns");
    assert!(found > 0, "no surf on any of {checked} coastal chunk columns");
}

#[test]
fn chunk_columns_report_their_springs() {
    let w = world();
    let c = w.continent(0, 0);
    let springs: Vec<_> = (0..c.size * c.size).filter_map(|i| crate::surface::Spring::at(&c, 3, i)).take(6).collect();
    assert!(!springs.is_empty(), "no springs on the test continent");
    for sp in &springs {
        let (cx, cz) = (sp.pos.x.floor() as i32 / CHUNK, sp.pos.y.floor() as i32 / CHUNK);
        let out = w.generate_column(cx, cz);
        let mark = out.springs.iter().find(|m| (m.pos.x - sp.pos.x).abs() < 0.01 && (m.pos.z - sp.pos.y).abs() < 0.01);
        let mark = mark.unwrap_or_else(|| panic!("spring at {:?} missing from column {cx},{cz}", sp.pos));
        assert_eq!(mark.pos.y, sp.water as f32);
    }
}

#[test]
fn overhangs_hang_from_faces_and_agree_across_chunks() {
    use crate::surface::Cover;
    let w = world();
    let c = w.continent(0, 0);
    // Chunk columns with cliff in them, from a coarse look over the continent.
    let mut cliffs = Vec::new();
    for tz in (0..MACRO_SIZE).step_by(2048) {
        for tx in (0..MACRO_SIZE).step_by(2048) {
            let s = Sampler::new(&c, w.seed(), tx, tz, 2048);
            for z in (tz..tz + 2048).step_by(24) {
                for x in (tx..tx + 2048).step_by(24) {
                    if s.column(x, z).cover == Cover::Cliff {
                        cliffs.push((x / CHUNK, z / CHUNK));
                    }
                }
            }
        }
    }
    assert!(!cliffs.is_empty(), "no cliffs on the test continent");
    let (mut found, mut slabs) = (0, 0);
    for &(cx, cz) in cliffs.iter().step_by((cliffs.len() / 400).max(1)) {
        let (l0x, l0z) = (cx * CHUNK - 1, cz * CHUNK - 1);
        let s = Sampler::new(&c, w.seed(), l0x - 8, l0z - 8, PAD as i32 + 16);
        let columns: Vec<_> = (0..PAD as i32)
            .flat_map(|px| (0..PAD as i32).map(move |pz| (px, pz)))
            .map(|(px, pz)| s.column(l0x + px, l0z + pz))
            .collect();
        let hangs = crate::world::overhangs(w.seed(), &s, &columns, (l0x, l0z), (l0x, l0z));
        if hangs.iter().all(Option::is_none) {
            continue;
        }
        found += 1;
        // Every slab is at least two voxels thick, clear of the ground under it, and
        // held up by rock or another slab beside it.
        for px in 1..PAD - 1 {
            for pz in 1..PAD - 1 {
                let Some((a, b)) = hangs[px * PAD + pz] else { continue };
                slabs += 1;
                assert!(b > a, "slab one voxel thick");
                assert!(a > columns[px * PAD + pz].top() + 1, "slab resting on the ground");
                let held = [(px + 1, pz), (px - 1, pz), (px, pz + 1), (px, pz - 1)].iter().any(|&(x, z)| {
                    columns[x * PAD + z].top() >= b || hangs[x * PAD + z].is_some_and(|(c, d)| c <= b && d >= a)
                });
                assert!(held, "loose slab at {},{}", l0x + px as i32, l0z + pz as i32);
            }
        }
        // The chunk columns either side agree on the voxels they share.
        let (here, east) = (w.generate_column(cx, cz).chunks, w.generate_column(cx + 1, cz).chunks);
        for (cy, a) in &here {
            let Some((_, b)) = east.iter().find(|e| e.0 == *cy) else { continue };
            for (ax, bx) in [(PAD - 2, 0), (PAD - 1, 1)] {
                for y in 0..PAD {
                    for z in 0..PAD {
                        assert_eq!(a.get(ax, y, z), b.get(bx, y, z), "chunks {cx},{cz} and east differ at ax {ax} y {y} z {z} cy {cy} top {} cover {:?}", columns[ax * PAD + z].top(), columns[ax * PAD + z].cover);
                    }
                }
            }
        }
        if found == 4 {
            break;
        }
    }
    eprintln!("{found} chunk columns with overhangs, {slabs} slab columns");
    assert!(found > 0, "no overhangs near any cliff");
}

#[test]
fn a_bare_column_is_the_planted_one_without_its_plants() {
    let w = world();
    let spawn = w.spawn_point();
    let (cx, cz) = (spawn.x.div_euclid(CHUNK), spawn.z.div_euclid(CHUNK));
    let mut planted = 0;
    for (dx, dz) in (-3..3).flat_map(|dz| (-3..3).map(move |dx| (dx, dz))) {
        let (full, bare) = (w.generate_column(cx + dx, cz + dz), w.generate_column_bare(cx + dx, cz + dz));
        planted += full.plants.len();
        assert!(bare.plants.is_empty());
        assert!(bare.chunks.len() <= full.chunks.len());
        for (i, (cy, chunk)) in full.chunks.iter().enumerate() {
            match bare.chunks.get(i) {
                Some((by, b)) => assert!(by == cy && b.voxels() == chunk.voxels(), "chunk {cy} of column {dx},{dz} differs"),
                // Only plants reached up here.
                None => assert!(chunk.voxels().iter().all(|&v| v == voxel::AIR), "chunk {cy} of column {dx},{dz} is missing"),
            }
        }
        assert_eq!(full.currents, bare.currents);
        assert_eq!(full.trees.len(), bare.trees.len());
        assert_eq!((full.cascades.len(), full.springs.len(), full.surf.len()), (bare.cascades.len(), bare.springs.len(), bare.surf.len()));
    }
    assert!(planted > 0, "nothing grows around the spawn point");
}

#[test]
fn nothing_small_grows_up_through_a_bridge() {
    // Plants are grown before the bridges go down, and a reed on the bank under a deck
    // would stand up through its planks.
    use crate::features::attach::{ColumnVoxels, Voxels};
    use glam::IVec3;
    let w = world();
    let c = w.continent(0, 0);
    let cells = IVec3::splat(voxel::mesh::CELLS);
    let mut columns: Vec<(i32, i32)> = c.trails.crossings.iter().map(|x| (x.x.div_euclid(CHUNK), x.z.div_euclid(CHUNK))).collect();
    columns.sort_unstable();
    columns.dedup();
    let (mut looked, mut under) = (0, 0);
    for &(cx, cz) in columns.iter().take(40) {
        let out = w.generate_column(cx, cz);
        let voxels = ColumnVoxels::new(&out.chunks, cx * CHUNK - 1, cz * CHUNK - 1);
        for b in &out.plants {
            let (lo, hi) = (IVec3::from_array(b.lo).div_euclid(cells), (IVec3::from_array(b.hi) - 1).div_euclid(cells));
            let ground = w.column(lo.x, lo.z).top();
            // Only what grows on the ground: not the fine parts of the bridge itself.
            if lo.y != ground + 1 || w.materials().get(b.voxel).kind != voxel::Kind::Foliage || !voxels.has_height(hi.y) {
                continue;
            }
            // Nor a climber, which clings to what it climbs: the ivy up a willow by the
            // bank where a road crosses touches its bark by design.
            use crate::features::climber::{FLOWERING_VINE, IVY, LIANA, TRAILING_CREEPER, VIRGINIA_CREEPER};
            if [IVY, VIRGINIA_CREEPER, FLOWERING_VINE, LIANA, TRAILING_CREEPER].iter().any(|c| c.leaves.contains(&b.voxel)) {
                continue;
            }
            looked += 1;
            if let Some(y) = (lo.y..=hi.y).find(|&y| w.materials().get(voxels.voxel(IVec3::new(lo.x, y, lo.z))).kind == voxel::Kind::Solid) {
                under += 1;
                println!("{} at {lo} grows up into voxel {} at height {y}", b.voxel, voxels.voxel(IVec3::new(lo.x, y, lo.z)));
            }
        }
    }
    assert!(looked > 0);
    assert_eq!(under, 0, "{under} plants grow up into something solid");
}

#[test]
fn nests_are_found_where_the_chunks_build_them() {
    use crate::features::nests::{Host, NestKind};
    let w = world();
    let c = w.continent(0, 0);
    // Land all over the continent, wooded or not, until a good few nests have turned up.
    let mut nests = Vec::new();
    let n = c.size;
    let cs = c.cell_size();
    'look: for i in (0..n * n).step_by(97) {
        if c.ocean[i] {
            continue;
        }
        let (x, z) = (((i % n) as f32 + 0.5) * cs, ((i / n) as f32 + 0.5) * cs);
        let found = w.nests_near(x as i32, z as i32, 160);
        // Asked again from a little way off, the same nests are where they were.
        let again = w.nests_near(x as i32 + 40, z as i32 - 30, 220);
        for nest in &found {
            assert!(again.contains(nest), "{nest:?} not found again from nearby");
        }
        for nest in found {
            if !nests.contains(&nest) {
                nests.push(nest);
            }
            if nests.len() >= 24 {
                break 'look;
            }
        }
    }
    assert!(nests.len() >= 6, "only {} nests on the test continent", nests.len());
    let kinds = |k| nests.iter().filter(|n| n.kind == k).count();
    eprintln!("{} bees' nests, {} wasps'", kinds(NestKind::Bees), kinds(NestKind::Wasps));
    // The chunks hold each where it was said to be, unless something built stands there.
    let at = |q: glam::IVec3| {
        let (cx, cz) = (q.x.div_euclid(CHUNK), q.z.div_euclid(CHUNK));
        let chunks = w.generate_column(cx, cz).chunks;
        let (cy, ly) = (q.y.div_euclid(CHUNK), q.y.rem_euclid(CHUNK));
        let (px, pz) = ((q.x - (cx * CHUNK - 1)) as usize, (q.z - (cz * CHUNK - 1)) as usize);
        chunks.iter().find(|(y, _)| *y == cy).map(|(_, ch)| ch.get(px, ly as usize + 1, pz))
    };
    let mut built = 0;
    for nest in &nests {
        let as_said = match nest.host {
            Host::Hollow | Host::Crevice => at(nest.door) == Some(voxel::AIR) && at(nest.door - nest.out) == Some(mat::NEST_HOLLOW),
            Host::Branch => at(nest.door) == Some(mat::NEST_HOLLOW) && at(nest.door + glam::IVec3::Y * 2) == Some(mat::WASP_PAPER),
            Host::Hive => false,
        };
        if as_said {
            built += 1;
        } else {
            eprintln!("{nest:?} is not in the chunks");
        }
    }
    assert!(built * 10 >= nests.len() * 9, "only {built} of {} nests are in the chunks", nests.len());
}
