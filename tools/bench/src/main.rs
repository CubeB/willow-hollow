//! Times generating and meshing patches of chunk columns on one thread, and prints a
//! checksum of everything generated, so a change meant only to make generation faster
//! can be shown to have left the world as it was.
//!
//! ```sh
//! cargo run --release -p bench -- --seed 1
//! ```

use std::time::Instant;

use anyhow::{Context, Result, bail};
use voxel::{
    CHUNK,
    mesh::{Falls, mesh_chunk_under_sky},
};
use worldgen::{ColumnOutput, World, world::GameWorld};

struct Options {
    seed: u32,
    /// Columns along each side of a patch.
    size: i32,
    /// Extra patches, by the world voxel at their middle.
    at: Vec<(i32, i32)>,
}

const HELP: &str = "\
Usage: bench [options]

  --seed N     world seed (default 1)
  --size N     columns along each side of a patch (default 12)
  --at X,Z     also time a patch around this world voxel (may be repeated)

Always times a patch at the spawn point, and one each on a coast, in the hills and
at sea, found from the continent.";

fn parse() -> Result<Options> {
    let mut o = Options { seed: 1, size: 12, at: Vec::new() };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().with_context(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--seed" => o.seed = value()?.parse()?,
            "--size" => o.size = value()?.parse()?,
            "--at" => {
                let v = value()?;
                let (x, z) = v.split_once(',').context("--at takes X,Z")?;
                o.at.push((x.trim().parse()?, z.trim().parse()?));
            }
            "--help" | "-h" => {
                println!("{HELP}");
                std::process::exit(0);
            }
            other => bail!("unknown option {other}\n\n{HELP}"),
        }
    }
    Ok(o)
}

/// FNV-1a over 32-bit words.
struct Sum(u64);

impl Sum {
    fn add(&mut self, v: u32) {
        self.0 = (self.0 ^ v as u64).wrapping_mul(0x100_0000_01b3);
    }

    fn column(&mut self, out: &ColumnOutput) {
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
            self.add(p.voxel as u32 | (p.sway_lo as u32) << 16 | (p.sway_hi as u32) << 17 | (p.thin as u32) << 18);
        }
        for c in &out.currents {
            self.add(c.x.to_bits());
            self.add(c.z.to_bits());
            self.add(c.river as u32);
        }
        for c in &out.cascades {
            for v in c.pos.to_array().into_iter().chain(c.dir.to_array()).chain([c.drop, c.speed]) {
                self.add(v.to_bits());
            }
        }
        for s in &out.springs {
            for v in s.pos.to_array().into_iter().chain([s.radius]) {
                self.add(v.to_bits());
            }
        }
        for s in &out.surf {
            for v in s.pos.to_array().into_iter().chain(s.dir.to_array()) {
                self.add(v.to_bits());
            }
        }
        for t in &out.trees {
            for v in t.pos.to_array() {
                self.add(v as u32);
            }
            self.add(t.variant as u32);
            self.add(t.turns);
        }
    }
}

/// Generates and meshes the patch of columns around world voxel `(x, z)`.
fn patch(world: &GameWorld, name: &str, (x, z): (i32, i32), size: i32) {
    let (cx, cz) = (x.div_euclid(CHUNK) - size / 2, z.div_euclid(CHUNK) - size / 2);
    let mut sum = Sum(0xcbf2_9ce4_8422_2325);
    let (mut generate, mut mesh, mut slowest, mut bare) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let mut bare_sum = Sum(0xcbf2_9ce4_8422_2325);
    let (mut plants, mut quads) = (0usize, 0usize);
    for dz in 0..size {
        for dx in 0..size {
            let t0 = Instant::now();
            let out = world.generate_column(cx + dx, cz + dz);
            let t1 = Instant::now();
            let mut falls = Falls::default();
            let roof = vec![false; voxel::PAD * voxel::PAD];
            for (_, chunk) in out.chunks.iter().rev() {
                let m = mesh_chunk_under_sky(chunk, world.materials(), &out.currents, &roof, &mut falls);
                quads += (m.opaque.len() + m.water.len()) / 4;
            }
            let t2 = Instant::now();
            let ms = (t1 - t0).as_secs_f64() * 1000.0;
            generate += ms;
            slowest = slowest.max(ms);
            mesh += (t2 - t1).as_secs_f64() * 1000.0;
            plants += out.plants.len();
            sum.column(&out);
            let t3 = Instant::now();
            let unplanted = world.generate_column_bare(cx + dx, cz + dz);
            bare += t3.elapsed().as_secs_f64() * 1000.0;
            bare_sum.column(&unplanted);
        }
    }
    let n = (size * size) as f64;
    println!(
        "{name:<8} at {x:>6},{z:<6} generate {:6.2} ms (slowest {slowest:6.1}), bare {:5.2} ms, mesh {:5.2} ms, {:6.0} plant boxes, {:6.0} quads a column, checksums {:016x} {:016x}",
        generate / n,
        bare / n,
        mesh / n,
        plants as f64 / n,
        quads as f64 / n,
        sum.0,
        bare_sum.0
    );
}

fn main() -> Result<()> {
    let o = parse()?;
    let world = GameWorld::new(o.seed);
    let t = Instant::now();
    let spawn = world.spawn_point();
    let c = world.continent(0, 0);
    println!("continent in {:.2} s", t.elapsed().as_secs_f64());
    let t = Instant::now();
    world.library();
    println!("library in {:.2} s", t.elapsed().as_secs_f64());

    // A coast, the hills and open sea, from the continent's own grid: the first cell of
    // each kind found scanning out from the middle.
    let n = c.size;
    let to_world = |i: usize| {
        let scale = worldgen::macro_layout::MACRO_SIZE as f32 / n as f32;
        (((i % n) as f32 + 0.5) * scale) as i32
    };
    let cell = |i: usize| (to_world(i), ((i / n) as f32 + 0.5) as i32 * (worldgen::macro_layout::MACRO_SIZE / n as i32));
    let order = || (0..n * n).map(|k| (k * 7919) % (n * n));
    let inland = |i: usize| i % n > 2 && i / n > 2 && i % n < n - 3 && i / n < n - 3;
    let coast = order().find(|&i| inland(i) && !c.ocean[i] && (c.ocean[i + 1] || c.ocean[i - 1] || c.ocean[i + n] || c.ocean[i - n]));
    let highest = c.height.iter().cloned().fold(0.0f32, f32::max);
    let hills = order().find(|&i| inland(i) && !c.ocean[i] && c.height[i] > highest * 0.55);
    let sea = order().find(|&i| inland(i) && c.ocean[i] && c.ocean[i + 3] && c.ocean[i - 3] && c.ocean[i + 3 * n] && c.ocean[i - 3 * n]);

    patch(&world, "spawn", (spawn.x, spawn.z), o.size);
    for (name, found) in [("coast", coast), ("hills", hills), ("sea", sea)] {
        if let Some(i) = found {
            patch(&world, name, cell(i), o.size);
        }
    }
    for at in &o.at {
        patch(&world, "chosen", *at, o.size);
    }
    Ok(())
}
