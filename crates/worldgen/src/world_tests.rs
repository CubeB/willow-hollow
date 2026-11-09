use voxel::{CHUNK, PAD};

use crate::{
    World,
    continent::SimConfig,
    macro_layout::MACRO_SIZE,
    materials as mat,
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
            // Rivers pass through their node, which is not the cell centre.
            let p = node(&c, i);
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
    let chunks = w.generate_column(cx, cz);
    assert!(!chunks.is_empty() && chunks.len() <= 4, "{} chunks", chunks.len());
    let has_water = chunks
        .iter()
        .any(|(_, chunk)| chunk.voxels().contains(&mat::WATER));
    assert!(has_water, "river chunk column contains no water");
    for (_, chunk) in &chunks {
        assert!(!chunk.is_uniform());
    }
    // Far out to sea: a single chunk holding the sea bed and surface.
    let sea = w.generate_column(2, MACRO_SIZE / CHUNK / 2);
    assert!(sea.len() <= 4);
    assert!(sea.iter().all(|(_, chunk)| chunk.voxels().contains(&mat::WATER)));
}
