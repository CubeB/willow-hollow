//! A sweep for crashes: makes many worlds the way the game does and reports every one
//! that panics, leaves a number that is not finite where the ground is built from it,
//! or comes out differently when made a second time.
//!
//! Of each cell of each world it makes the continent with its trails, settlements and
//! farmland, as [`GameWorld::continent`] does; the people of its settlements, a week of
//! their days, their walks and what they say; chunk columns at its settlements,
//! landmarks and quarries, along its edges and scattered over it, each meshed as the
//! game meshes it; the far view, the map's picture and its marks; and the travellers,
//! boats and animals. Nothing here draws anything: a few `--frames` runs of the game
//! itself are still the only check of what is drawn and of what moves.
//!
//! It is slow, so it is ignored. Run it on its own, on one thread:
//!
//! ```sh
//! SWEEP_SEEDS=1..200 SWEEP_SIZE=256 cargo test --profile fast -p game -j 2 sweep -- --ignored --nocapture --test-threads=1
//! ```
//!
//! - `SWEEP_SEEDS=A..B` the seeds, B left out (default `1..9`).
//! - `SWEEP_SIZE=N` the continent's grid (default 256, about 6 s a cell with
//!   `SWEEP_TOWNS=6`; the game's is 1024, about 15 s a cell and little over 100 MB).
//! - `SWEEP_CELLS="0,0;1,0;-1,2"` the map cells made for each seed. Left out, each seed
//!   makes `0,0`, the one every world starts on, and up to three of the cells about
//!   it: two that a mountain range runs out of over an edge, where there are any, since
//!   those have land right up to the edge of their grids, and one more. Only the cells
//!   about the first are ever islands, open ocean or any continent but a massif.
//! - `SWEEP_COLUMNS=N` how many chunk columns to generate and mesh in each cell besides
//!   those at its settlements, landmarks and quarries and along any edge a range
//!   runs out over (default 12).
//! - `SWEEP_TOWNS=N` makes the people of only N settlements in each cell, spread over
//!   its towns, villages and hamlets (default all of them, which is most of the time).
//! - `SWEEP_TWICE=1` makes each cell again, its columns in the other order, and reports
//!   any that differ.
//! - `SWEEP_YEAR=N` also works out the settlements as they stand N years on.
//!
//! Each failure is printed as it is found with its seed and cell, and the test fails at
//! the end if there were any. The `fast` and release builds do not check their
//! arithmetic for overflow; to sweep with it checked, as a plain debug build checks it,
//! add `CARGO_PROFILE_FAST_OVERFLOW_CHECKS=true CARGO_PROFILE_FAST_DEBUG_ASSERTIONS=true`
//! (which builds everything again, beside the usual build).

use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Mutex},
    time::Instant,
};

use glam::IVec3;
use voxel::{
    CHUNK,
    mesh::{CELLS, Falls, add_plants, cover_columns, mesh_chunk_under_sky, tag_trees},
};
use worldgen::{
    ColumnOutput, World,
    continent::{Continent, SimConfig},
    features::LeafTags,
    macro_layout::MACRO_SIZE,
    noise::hash2,
    settlements::years::{Boons, When},
    world::GameWorld,
};

use crate::villagers::town::Town;

/// FNV-1a over 32-bit words.
struct Sum(u64);

impl Sum {
    fn new() -> Self {
        Sum(0xcbf2_9ce4_8422_2325)
    }

    fn add(&mut self, v: u32) {
        self.0 = (self.0 ^ v as u64).wrapping_mul(0x100_0000_01b3);
    }

    fn floats(&mut self, what: &str, values: impl IntoIterator<Item = f32>, bad: &mut Vec<String>) {
        let mut wrong = 0;
        for v in values {
            wrong += !v.is_finite() as usize;
            self.add(v.to_bits());
        }
        if wrong > 0 {
            bad.push(format!("{wrong} values of {what} are not finite"));
        }
    }

    fn column(&mut self, out: &ColumnOutput, bad: &mut Vec<String>) {
        for (cy, chunk) in &out.chunks {
            self.add(*cy as u32);
            for &v in chunk.voxels() {
                self.add(v as u32);
            }
        }
        for p in &out.plants {
            for v in p.lo.into_iter().chain(p.hi) {
                self.add(v as u32);
            }
            self.add(p.voxel as u32);
        }
        self.floats("a current", out.currents.iter().flat_map(|c| [c.x, c.z]), bad);
        self.floats("a cascade", out.cascades.iter().flat_map(|c| c.pos.to_array().into_iter().chain(c.dir.to_array()).chain([c.drop, c.speed])), bad);
        self.floats("a spring", out.springs.iter().flat_map(|s| s.pos.to_array().into_iter().chain([s.radius])), bad);
        self.floats("the surf", out.surf.iter().flat_map(|s| s.pos.to_array().into_iter().chain(s.dir.to_array())), bad);
        for t in &out.trees {
            for v in t.pos.to_array() {
                self.add(v as u32);
            }
            self.add(t.variant as u32);
            self.add(t.turns);
        }
    }
}

/// Generates the chunk column and meshes it as the game does, checking what comes out.
fn column(world: &GameWorld, cx: i32, cz: i32, sum: &mut Sum, bad: &mut Vec<String>) {
    let out = world.generate_column(cx, cz);
    let mut falls = Falls::default();
    let mut roof = vec![false; voxel::PAD * voxel::PAD];
    let (lib, mats) = (world.library(), world.materials());
    for (cy, chunk) in out.chunks.iter().rev() {
        let wash = voxel::mesh::wash_map(chunk, mats, *cy, &out.wash);
        let mut m = mesh_chunk_under_sky(chunk, mats, &out.currents, &roof, &wash, &mut falls);
        let corner = [cx * CHUNK, *cy * CHUNK, cz * CHUNK];
        let inside = out.plants.iter().filter(|p| p.lo[1].div_euclid(CHUNK * CELLS) == *cy);
        add_plants(&mut m, inside, chunk, corner.map(|v| v * CELLS), mats, &roof);
        cover_columns(chunk, mats, &mut roof);
        let mut tags = LeafTags::new(lib, mats, &out.trees, IVec3::from_array(corner));
        tag_trees(&mut m, corner, |q| tags.tag(IVec3::from_array(q)));
        sum.add(m.opaque.len() as u32);
        sum.add(m.water.len() as u32);
    }
    let before = bad.len();
    sum.column(&out, bad);
    for b in &mut bad[before..] {
        *b = format!("column {cx},{cz}: {b}");
    }
    let bare = world.generate_column_bare(cx, cz);
    sum.add(bare.chunks.len() as u32);
}

/// The chunk columns of a cell worth making: the middle of every settlement and a
/// column or two out in it, every landmark, quarry and barn, the spawn, and `extra`
/// more scattered over the cell, land and sea.
fn columns(c: &Continent, seed: u32, (mx, mz): (i32, i32), extra: usize) -> Vec<(i32, i32)> {
    let net = &c.trails;
    let mut local: Vec<(i32, i32)> = Vec::new();
    for (k, s) in net.settlements.iter().enumerate() {
        local.push((s.x, s.z));
        let r = s.radius as i32;
        for j in 0..2 {
            let h = hash2(seed ^ 0x51ee9, k as i32, j);
            local.push((s.x + (h % (2 * r as u32 + 1)) as i32 - r, s.z + ((h >> 12) % (2 * r as u32 + 1)) as i32 - r));
        }
    }
    local.extend(net.sites.iter().map(|s| (s.x, s.z)));
    local.extend(c.land.quarries.iter().map(|q| (q.floor.x, q.floor.z)));
    for k in 0..extra {
        let h = hash2(seed ^ 0xc01_0000, k as i32, mx * 31 + mz);
        let (x, z) = ((h % MACRO_SIZE as u32) as i32, (hash2(h, 1, 2) % MACRO_SIZE as u32) as i32);
        local.push((x, z));
    }
    // Where a range runs on over the edge of the cell, the land right at the edge and
    // the rivers leaving over it, and the same ground from the next cell's side.
    let n = c.size;
    let edge: Vec<usize> = (0..n * n).filter(|&i| !c.ocean[i] && (i % n == 0 || i / n == 0 || i % n == n - 1 || i / n == n - 1)).collect();
    let mut wettest = edge.clone();
    wettest.sort_by(|&a, &b| c.discharge[b].total_cmp(&c.discharge[a]).then(a.cmp(&b)));
    let cs = c.cell_size();
    for &i in edge.iter().step_by((edge.len() / 6).max(1)).chain(wettest.iter().take(4)) {
        let (x, z) = (((i % n) as f32 + 0.5) * cs, ((i / n) as f32 + 0.5) * cs);
        let (x, z) = (x as i32, z as i32);
        local.push((x, z));
        // One chunk further out, whichever edge it is on.
        let out = |v: i32| if v < CHUNK { v - CHUNK } else if v >= MACRO_SIZE - CHUNK { v + CHUNK } else { v };
        local.push((out(x), out(z)));
    }
    // The corners and edges of the cell, where one continent's ground meets the next.
    local.extend([(0, 0), (MACRO_SIZE - 1, MACRO_SIZE / 2), (MACRO_SIZE / 2, 0), (MACRO_SIZE - 1, MACRO_SIZE - 1)]);
    let mut out: Vec<(i32, i32)> = local.into_iter().map(|(x, z)| ((x + mx * MACRO_SIZE).div_euclid(CHUNK), (z + mz * MACRO_SIZE).div_euclid(CHUNK))).collect();
    out.sort();
    out.dedup();
    out
}

/// A week of the days of everyone in `town`, the walks of a few of them between the
/// places they stay at, what they say when greeted and what is on the board: every hour
/// and length in them, to be checked.
fn days(town: &Town) -> Vec<f32> {
    use crate::villagers::{day, notices, talk};
    let year = crate::season::DEFAULT_YEAR_DAYS;
    let mut out = Vec::new();
    // A week holds a market day and a day of rest, whichever days the place keeps them.
    for d in 280..287 {
        let plans = day::plans(town, d, year);
        for (n, plan) in plans.iter().enumerate() {
            out.extend(plan.stays.iter().flat_map(|s| [s.arrive, s.leave]));
            if n % 5 == (d % 5) as usize {
                for pair in plan.stays.windows(2) {
                    out.push(town.walk_length((pair[0].place, &pair[0].spot), (pair[1].place, &pair[1].spot)));
                    out.extend(town.last_steps(pair[1].place, &pair[1].spot).iter().flat_map(|p| p.to_array()));
                }
            }
        }
        let (_, asked) = notices::board_page(town, d, year);
        out.push(asked.len() as f32);
    }
    for (n, r) in town.residents.iter().enumerate() {
        out.push(talk::greeting(town, r, n as u32 % 3, 6.0 + (n % 16) as f32, (n % 24) as f32 / 24.0).len() as f32);
    }
    out
}

struct Options {
    size: usize,
    extra: usize,
    year: i32,
    towns: usize,
}

/// Makes one cell of one world, everything the game would make of it, and returns its
/// checksum and what was wrong with it. `backwards` generates the columns in the other order.
fn cell(seed: u32, at: (i32, i32), o: &Options, backwards: bool) -> (u64, Vec<String>, String) {
    let mut bad = Vec::new();
    let mut sum = Sum::new();
    let world = GameWorld::with_config(seed, SimConfig { size: o.size });
    let t = Instant::now();
    let c = world.continent(at.0, at.1);
    let made = t.elapsed().as_secs_f64();
    sum.floats("height", c.height.iter().copied(), &mut bad);
    sum.floats("lake", c.lake.iter().copied(), &mut bad);
    sum.floats("water level", c.water_level.iter().copied(), &mut bad);
    sum.floats("discharge", c.discharge.iter().copied(), &mut bad);
    sum.floats("rain", c.rain.iter().copied(), &mut bad);
    sum.floats("temperature", c.temperature.iter().copied(), &mut bad);
    for (&r, &b) in c.rcv.iter().zip(&c.biome) {
        sum.add(r);
        sum.add(b as u32);
    }
    if c.rcv.iter().any(|&r| r as usize >= c.size * c.size) {
        bad.push("a cell drains off the grid".into());
    }
    let net = &c.trails;
    for s in &net.settlements {
        for v in [s.x, s.y, s.z, s.seed as i32, s.people as i32, s.buildings as i32] {
            sum.add(v as u32);
        }
        sum.floats("a settlement", [s.radius, s.prosperity], &mut bad);
    }
    for s in &net.sites {
        for v in [s.x, s.z] {
            sum.add(v as u32);
        }
    }
    sum.floats("a trail", net.lines.iter().flatten().flat_map(|p| [p.x, p.y]), &mut bad);
    sum.floats("a trail's width", net.line_widths.iter().flatten().copied(), &mut bad);
    sum.floats("a chimney", net.chimneys.iter().flat_map(|p| p.to_array()), &mut bad);
    sum.floats("a guest bed", net.guest_beds.iter().flat_map(|p| p.to_array()), &mut bad);
    for q in &c.land.quarries {
        for v in q.floor.to_array() {
            sum.add(v as u32);
        }
    }
    sum.add(c.land.farms.len() as u32);
    sum.add(c.land.steads.len() as u32);
    if at == (0, 0) {
        let s = world.spawn_point();
        for v in [s.x, s.z] {
            sum.add(v as u32);
        }
    }

    // Every settlement's people, today and, if asked, some years on.
    let t = Instant::now();
    let mut people = 0;
    let mut whens = vec![When::default()];
    if o.year > 0 {
        whens.push(When::new(o.year, 2));
    }
    for when in whens {
        world.set_now(when, &Boons::default());
        let lives = world.lives(at.0, at.1, &c);
        let every = net.settlements.len().div_ceil(o.towns.max(1)).max(1);
        for k in (0..net.settlements.len()).step_by(every) {
            match catch_unwind(AssertUnwindSafe(|| Town::build_in(&c, seed, at, k, &lives).map(|town| (days(&town), town)))) {
                Ok(Some((lived, town))) => {
                    sum.floats("a villager's day", lived, &mut bad);
                    people += town.residents.len();
                    sum.add(town.residents.len() as u32);
                    sum.add(town.places.len() as u32);
                    sum.floats("a resident's home", town.places.iter().flat_map(|p| p.at.to_array()), &mut bad);
                }
                Ok(None) => {}
                Err(_) => bad.push(format!("settlement {k} ({:?} {:?} at {},{}): its people panicked: {}", net.settlements[k].kind, net.settlements[k].age, net.settlements[k].x, net.settlements[k].z, last_panic())),
            }
        }
    }
    let towns = t.elapsed().as_secs_f64();

    let t = Instant::now();
    let mut cols = columns(&c, seed, at, o.extra);
    if backwards {
        cols.reverse();
    }
    // Summed in an order that does not depend on the order they were made in.
    let mut each = Vec::new();
    for &(cx, cz) in &cols {
        let mut s = Sum::new();
        if catch_unwind(AssertUnwindSafe(|| column(&world, cx, cz, &mut s, &mut bad))).is_err() {
            bad.push(format!("column {cx},{cz} (voxel {},{}) panicked: {}", cx * CHUNK, cz * CHUNK, last_panic()));
        }
        each.push(((cx, cz), s.0));
    }
    each.sort();
    for (_, s) in each {
        sum.add(s as u32);
        sum.add((s >> 32) as u32);
    }
    // What the game asks of the land about a place besides its columns.
    for s in net.settlements.iter().take(8) {
        let (x, z) = (s.x + at.0 * MACRO_SIZE, s.z + at.1 * MACRO_SIZE);
        let asked = catch_unwind(AssertUnwindSafe(|| {
            (world.nests_near(x, z, 96).len(), world.hives_near(x, z, 96).len(), world.flowers_standing(x - 32, z - 32, 64).len(), world.delights_near(x, z, 400).len(), world.chimneys_near(x, z, 400).len())
        }));
        match asked {
            Ok((a, b, f, d, ch)) => [a, b, f, d, ch].into_iter().for_each(|v| sum.add(v as u32)),
            Err(_) => bad.push(format!("asking about the land at {x},{z} panicked: {}", last_panic())),
        }
    }
    // The far view, the map's picture and its marks, which are made of every continent in sight.
    let far = catch_unwind(AssertUnwindSafe(|| {
        let far = crate::farview::swept(&world, &c, at);
        let sky = crate::farview::Skylines::get(&c);
        let lines = sky.blended(MACRO_SIZE as f32 * 0.5, MACRO_SIZE as f32 * 0.5);
        let land = crate::farview::map_land(&world, &c);
        let picture = crate::minimap::picture::picture(&world, &c);
        let marks = crate::minimap::marks::find(&c, seed);
        (far, lines, land, picture, marks.len())
    }));
    match far {
        Ok((far, lines, land, picture, marks)) => {
            sum.floats("the far view", far, &mut bad);
            sum.floats("the map's land", land.into_iter().flatten(), &mut bad);
            picture.chunks(4).for_each(|p| sum.add(u32::from_le_bytes([p[0], p[1], p[2], p[3]])));
            sum.add(u32::from_le_bytes([lines[0], lines[1], 0, 0]));
            sum.add(marks as u32);
        }
        Err(_) => bad.push(format!("the far view or the map panicked: {}", last_panic())),
    }
    // The travellers on the roads and the boats on the rivers, over some days of the
    // year, calm and rough, and the animals wild and kept.
    let abroad = catch_unwind(AssertUnwindSafe(|| {
        use worldgen::settlements::travel::{Travel, Water};
        let travel = Travel::of(&c, seed);
        let middle = glam::Vec2::splat(MACRO_SIZE as f32 * 0.5);
        let mut out = Vec::new();
        for day in [0i64, 1, 5, 11, 17, 23, 200] {
            for plan in travel.near(&c, day, middle, MACRO_SIZE as f32) {
                for (hour, water) in [(0.5, Water::CALM), (7.0, Water::CALM), (12.0, Water::rough_since(11.0)), (16.5, Water::CALM), (21.0, Water::ROUGH)] {
                    let Some(scene) = plan.at(hour, day as f32 / 24.0 % 1.0, water) else { continue };
                    out.extend(scene.at.to_array());
                    out.extend(scene.people.iter().flat_map(|f| f.feet.to_array().into_iter().chain([f.facing])));
                    out.push((scene.carts.len() + scene.beasts.len() + scene.boats.len() + scene.tents.len()) as f32);
                }
            }
        }
        for k in 0..net.settlements.len() {
            out.push(worldgen::fauna::kept_about(&c, seed, k).len() as f32);
        }
        for s in net.settlements.iter().step_by(3) {
            let near = worldgen::fauna::sites_near(&world, s.x + at.0 * MACRO_SIZE, s.z + at.1 * MACRO_SIZE, 1200);
            out.extend(near.iter().flat_map(|site| [site.home.x as f32, site.home.z as f32, site.count as f32]));
        }
        out
    }));
    match abroad {
        Ok(out) => sum.floats("the travellers and the animals", out, &mut bad),
        Err(_) => bad.push(format!("the travellers or the animals panicked: {}", last_panic())),
    }
    let note = format!(
        "{:?} {:?}, {} cells of land at its edge, {} settlements, {} sites, {people} people, {} columns; continent {made:.1} s, people {towns:.1} s, columns {:.1} s",
        c.desc.kind,
        c.desc.archetype,
        (0..c.size * c.size).filter(|&i| !c.ocean[i] && (i % c.size == 0 || i / c.size == 0 || i % c.size == c.size - 1 || i / c.size == c.size - 1)).count(),
        net.settlements.len(),
        net.sites.len(),
        cols.len(),
        t.elapsed().as_secs_f64()
    );
    (sum.0, bad, note)
}

static LAST_PANIC: Mutex<String> = Mutex::new(String::new());

fn last_panic() -> String {
    LAST_PANIC.lock().map(|s| s.clone()).unwrap_or_default()
}

fn env<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

#[test]
#[ignore]
fn sweep() {
    let seeds = std::env::var("SWEEP_SEEDS").unwrap_or_else(|_| "1..9".into());
    let (a, b) = seeds.split_once("..").expect("SWEEP_SEEDS is A..B");
    let (a, b): (u32, u32) = (a.parse().expect("a seed"), b.parse().expect("a seed"));
    let chosen: Option<Vec<(i32, i32)>> = std::env::var("SWEEP_CELLS").ok().map(|s| {
        s.split(';')
            .map(|p| {
                let (x, z) = p.split_once(',').expect("SWEEP_CELLS is X,Z;X,Z");
                (x.trim().parse().expect("a cell"), z.trim().parse().expect("a cell"))
            })
            .collect()
    });
    let o = Options { size: env("SWEEP_SIZE", 256), extra: env("SWEEP_COLUMNS", 12), year: env("SWEEP_YEAR", 0), towns: env("SWEEP_TOWNS", usize::MAX) };
    let twice = env("SWEEP_TWICE", 0) != 0;
    // Where a panic was, kept for the report: the message alone seldom says.
    std::panic::set_hook(Box::new(|info| {
        if let Ok(mut last) = LAST_PANIC.lock() {
            let said = info.payload().downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_default();
            let at = info.location().map(|l| l.to_string()).unwrap_or_default();
            // A panic inside the standard library says nothing of who called it: the
            // nearest few of the game's own functions on the stack do.
            let mut callers = String::new();
            if !at.starts_with("crates") {
                let trace = std::backtrace::Backtrace::force_capture().to_string();
                let ours = trace.lines().map(str::trim).filter(|l| ["worldgen::", "game::", "voxel::"].iter().any(|c| l.contains(c)) && !l.contains("sweep::"));
                callers = ours.take(3).map(|l| l.split_once(": ").map_or(l, |(_, f)| f)).collect::<Vec<_>>().join(" < ");
            }
            *last = format!("{said} [{at}] {callers}");
        }
    }));
    let failures = Arc::new(Mutex::new(Vec::<String>::new()));
    let fail = |line: String| {
        println!("FAIL {line}");
        failures.lock().unwrap().push(line);
    };
    let start = Instant::now();
    for seed in a..b {
        let cells = chosen.clone().unwrap_or_else(|| {
            let mut cells = vec![(0, 0)];
            let about = (-2..=2).flat_map(|z| (-2..=2).map(move |x| (x, z))).filter(|&at| at != (0, 0));
            // A range holds land where it meets an edge of its cell.
            let crossed = |&(x, z): &(i32, i32)| {
                let field = worldgen::structures::Field::new(seed, x, z);
                (0..=16).any(|k| {
                    let t = -1.0 + k as f32 / 8.0;
                    [(1.0, t), (-1.0, t), (t, 1.0), (t, -1.0)].into_iter().any(|(px, pz)| field.land(px, pz).is_some_and(|l| l > 0.3))
                })
            };
            cells.extend(about.filter(crossed).take(2));
            let h = hash2(seed, 0x5eed, 7);
            let other = ((h % 7) as i32 - 3, ((h >> 8) % 7) as i32 - 3);
            if !cells.contains(&other) {
                cells.push(other);
            }
            cells
        });
        for &at in &cells {
            let t = Instant::now();
            let first = catch_unwind(AssertUnwindSafe(|| cell(seed, at, &o, false)));
            let sum = match first {
                Ok((sum, bad, note)) => {
                    println!("seed {seed} cell {},{} size {}: {note}; {:.1} s in all, checksum {sum:016x}", at.0, at.1, o.size, t.elapsed().as_secs_f64());
                    for b in bad {
                        fail(format!("seed {seed} cell {},{} size {}: {b}", at.0, at.1, o.size));
                    }
                    Some(sum)
                }
                Err(_) => {
                    fail(format!("seed {seed} cell {},{} size {}: panicked: {}", at.0, at.1, o.size, last_panic()));
                    None
                }
            };
            if let (true, Some(sum)) = (twice, sum) {
                match catch_unwind(AssertUnwindSafe(|| cell(seed, at, &o, true))) {
                    Ok((again, ..)) if again != sum => fail(format!("seed {seed} cell {},{} size {}: made again it differs, {sum:016x} then {again:016x}", at.0, at.1, o.size)),
                    Ok(_) => {}
                    Err(_) => fail(format!("seed {seed} cell {},{} size {}: panicked the second time only: {}", at.0, at.1, o.size, last_panic())),
                }
            }
        }
    }
    let _ = std::panic::take_hook();
    let failures = failures.lock().unwrap();
    println!("{} failures in {:.0} s", failures.len(), start.elapsed().as_secs_f64());
    for f in failures.iter() {
        println!("  {f}");
    }
    assert!(failures.is_empty(), "the sweep found {} failures", failures.len());
}
