//! Renders a continent's simulation output to PNG maps, for judging world generation
//! without running the game.

use std::{path::PathBuf, time::Instant};

use anyhow::{Context, Result, bail};
use image::{Rgb, RgbImage};
use worldgen::{
    biome::Biome,
    continent::{self, Continent, NO_LAKE, SimConfig},
    macro_layout::{MACRO_SIZE, cell_desc},
};

const USAGE: &str = "\
usage: mapgen [options]
  --seed N           world seed (default 1)
  --cell MX,MZ       macro-cell to generate (default 0,0)
  --size N           simulation grid size, power of two (default 1024)
  --out DIR          output directory (default maps)
  --zoom CX,CZ,SPAN  also render a close-up centred on cell CX,CZ, SPAN cells wide
  --px N             output image size in pixels (default 1024)
  --poi              list points of interest with world voxel coordinates
  --atlas MX,MZ,N    instead, render NxN macro-cells from MX,MZ stitched together
  --cell-px N        pixels per macro-cell in an atlas (default 200)";

struct Options {
    seed: u32,
    cell: (i32, i32),
    size: usize,
    out: PathBuf,
    zoom: Option<(f32, f32, f32)>,
    px: u32,
    poi: bool,
    atlas: Option<(i32, i32, i32)>,
    cell_px: u32,
}

fn parse_args() -> Result<Options> {
    let mut o = Options {
        seed: 1,
        cell: (0, 0),
        size: 1024,
        out: "maps".into(),
        zoom: None,
        px: 1024,
        poi: false,
        atlas: None,
        cell_px: 200,
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = || args.next().with_context(|| format!("{a} needs a value"));
        let nums = |s: String| -> Result<Vec<f32>> {
            s.split(',')
                .map(|p| p.trim().parse::<f32>().with_context(|| format!("bad number in '{s}'")))
                .collect()
        };
        match a.as_str() {
            "--seed" => o.seed = value()?.parse()?,
            "--cell" => {
                let v = nums(value()?)?;
                if v.len() != 2 {
                    bail!("--cell needs MX,MZ");
                }
                o.cell = (v[0] as i32, v[1] as i32);
            }
            "--size" => o.size = value()?.parse()?,
            "--out" => o.out = value()?.into(),
            "--zoom" => {
                let v = nums(value()?)?;
                if v.len() != 3 {
                    bail!("--zoom needs CX,CZ,SPAN");
                }
                o.zoom = Some((v[0], v[1], v[2]));
            }
            "--px" => o.px = value()?.parse()?,
            "--poi" => o.poi = true,
            "--atlas" => {
                let v = nums(value()?)?;
                if v.len() != 3 {
                    bail!("--atlas needs MX,MZ,N");
                }
                o.atlas = Some((v[0] as i32, v[1] as i32, v[2] as i32));
            }
            "--cell-px" => o.cell_px = value()?.parse()?,
            "--help" | "-h" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => bail!("unknown argument '{other}'\n{USAGE}"),
        }
    }
    Ok(o)
}

/// A square window onto the continent grid, in cell coordinates.
#[derive(Clone, Copy)]
struct View {
    x0: f32,
    z0: f32,
    span: f32,
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t)
}

/// Piecewise-linear colour ramp over `(value, colour)` stops.
fn ramp(stops: &[(f32, [f32; 3])], v: f32) -> [f32; 3] {
    if v <= stops[0].0 {
        return stops[0].1;
    }
    for pair in stops.windows(2) {
        if v <= pair[1].0 {
            let t = (v - pair[0].0) / (pair[1].0 - pair[0].0);
            return lerp3(pair[0].1, pair[1].1, t);
        }
    }
    stops[stops.len() - 1].1
}

const LAND_RAMP: [(f32, [f32; 3]); 7] = [
    (0.0, [176.0, 190.0, 128.0]),
    (25.0, [124.0, 168.0, 96.0]),
    (90.0, [96.0, 146.0, 84.0]),
    (180.0, [142.0, 150.0, 96.0]),
    (280.0, [150.0, 128.0, 100.0]),
    (400.0, [150.0, 146.0, 144.0]),
    (520.0, [244.0, 244.0, 248.0]),
];
const SEA_RAMP: [(f32, [f32; 3]); 3] = [
    (-80.0, [28.0, 62.0, 112.0]),
    (-20.0, [50.0, 110.0, 160.0]),
    (0.0, [92.0, 170.0, 196.0]),
];
const WATER: [f32; 3] = [44.0, 112.0, 196.0];

fn sample(c: &Continent, field: &[f32], x: f32, z: f32) -> f32 {
    let max = (c.size - 1) as f32;
    let (fx, fz) = ((x - 0.5).clamp(0.0, max), (z - 0.5).clamp(0.0, max));
    let (x0, z0) = (fx as usize, fz as usize);
    let (x1, z1) = ((x0 + 1).min(c.size - 1), (z0 + 1).min(c.size - 1));
    let (tx, tz) = (fx.fract(), fz.fract());
    let a = field[z0 * c.size + x0] + (field[z0 * c.size + x1] - field[z0 * c.size + x0]) * tx;
    let b = field[z1 * c.size + x0] + (field[z1 * c.size + x1] - field[z1 * c.size + x0]) * tx;
    a + (b - a) * tz
}

#[derive(Clone, Copy, PartialEq)]
enum Layer {
    /// Land coloured by elevation.
    Elevation,
    /// Land coloured by biome.
    Biome,
    /// Land coloured by rainfall, dry (tan) to wet (deep blue-green).
    Rain,
}

const RAIN_RAMP: [(f32, [f32; 3]); 4] = [
    (0.1, [226.0, 204.0, 150.0]),
    (0.7, [196.0, 206.0, 120.0]),
    (1.3, [96.0, 170.0, 110.0]),
    (2.6, [30.0, 96.0, 130.0]),
];

/// Shaded relief with lakes and the river network drawn on top.
fn render_relief(c: &Continent, view: View, px: u32, layer: Layer) -> RgbImage {
    let mut img = RgbImage::new(px, px);
    let scale = view.span / px as f32; // cells per pixel
    let cell = c.cell_size();
    for py in 0..px {
        for pxi in 0..px {
            let x = view.x0 + (pxi as f32 + 0.5) * scale;
            let z = view.z0 + (py as f32 + 0.5) * scale;
            let h = sample(c, &c.height, x, z);
            let (ix, iz) = (
                (x as usize).min(c.size - 1),
                (z as usize).min(c.size - 1),
            );
            let i = iz * c.size + ix;
            let color = if c.ocean[i] {
                ramp(&SEA_RAMP, h)
            } else if c.lake[i] != NO_LAKE {
                WATER
            } else {
                // Light from the north-west, as on a printed relief map.
                // Differences over a cell either way, one-sided at the grid's edge: a
                // centred difference there would read the clamped edge as flat and
                // draw a line along every border a range crosses.
                let (lo, hi) = (0.5, c.size as f32 - 0.5);
                let (x0, x1) = ((x - 1.0).max(lo), (x + 1.0).min(hi));
                let (z0, z1) = ((z - 1.0).max(lo), (z + 1.0).min(hi));
                let gx = (sample(c, &c.height, x1, z) - sample(c, &c.height, x0, z)) / ((x1 - x0).max(0.5) * cell);
                let gz = (sample(c, &c.height, x, z1) - sample(c, &c.height, x, z0)) / ((z1 - z0).max(0.5) * cell);
                let n = {
                    let (nx, ny, nz) = (-gx * 2.5, 1.0, -gz * 2.5);
                    let len = (nx * nx + ny * ny + nz * nz).sqrt();
                    (nx / len, ny / len, nz / len)
                };
                let light = (-0.5f32, 0.62, -0.6);
                let shade = (n.0 * light.0 + n.1 * light.1 + n.2 * light.2).clamp(0.0, 1.0);
                let base = match layer {
                    Layer::Elevation => ramp(&LAND_RAMP, h),
                    Layer::Biome => Biome::from_u8(c.biome[i]).map_color().map(|v| v as f32),
                    Layer::Rain => ramp(&RAIN_RAMP, c.rain[i]),
                };
                base.map(|v| v * (0.45 + 0.95 * shade))
            };
            img.put_pixel(pxi, py, Rgb(color.map(|v| v.clamp(0.0, 255.0) as u8)));
        }
    }

    // Rivers: a line from each carrying cell to the cell it drains into.
    let to_px = |cx: f32, cz: f32| ((cx - view.x0) / scale, (cz - view.z0) / scale);
    let (lo_x, hi_x) = (view.x0 - 2.0, view.x0 + view.span + 2.0);
    let (lo_z, hi_z) = (view.z0 - 2.0, view.z0 + view.span + 2.0);
    for z in 0..c.size {
        for x in 0..c.size {
            let i = z * c.size + x;
            let q = c.discharge[i];
            if c.ocean[i] || q < MIN_RIVER {
                continue;
            }
            let (fx, fz) = (x as f32 + 0.5, z as f32 + 0.5);
            if fx < lo_x || fx > hi_x || fz < lo_z || fz > hi_z {
                continue;
            }
            let r = c.rcv[i] as usize;
            let (rx, rz) = ((r % c.size) as f32 + 0.5, (r / c.size) as f32 + 0.5);
            let width_px = Continent::river_width(q) / c.sim_cell_size() / scale;
            // Thin streams are drawn fainter rather than thinner than a pixel.
            let alpha = (width_px / 0.6).clamp(0.25, 1.0);
            let radius = (width_px * 0.5).max(0.5);
            let (a, b) = (to_px(fx, fz), to_px(rx, rz));
            let steps = ((b.0 - a.0).hypot(b.1 - a.1) * 2.0).ceil().max(1.0) as i32;
            for s in 0..=steps {
                let t = s as f32 / steps as f32;
                let (cx, cy) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
                let rr = radius.ceil() as i32;
                for dy in -rr..=rr {
                    for dx in -rr..=rr {
                        let (qx, qy) = (cx as i32 + dx, cy as i32 + dy);
                        if qx < 0 || qy < 0 || qx >= px as i32 || qy >= px as i32 {
                            continue;
                        }
                        if ((dx * dx + dy * dy) as f32).sqrt() > radius + 0.3 {
                            continue;
                        }
                        let p = img.get_pixel_mut(qx as u32, qy as u32);
                        for k in 0..3 {
                            p.0[k] = (p.0[k] as f32 + (WATER[k] - p.0[k] as f32) * alpha) as u8;
                        }
                    }
                }
            }
        }
    }
    draw_trails(c, view, scale, &mut img);
    img
}

/// Trails as thin dark lines, with a coloured dot per landmark.
fn draw_trails(c: &Continent, view: View, scale: f32, img: &mut RgbImage) {
    let cell = c.cell_size();
    let px = img.width() as i32;
    let to_px = |x: f32, z: f32| (((x / cell - view.x0) / scale) as i32, ((z / cell - view.z0) / scale) as i32);
    let dot = |x: i32, z: i32, r: i32, color: [u8; 3], img: &mut RgbImage| {
        for dz in -r..=r {
            for dx in -r..=r {
                let (qx, qz) = (x + dx, z + dz);
                if qx >= 0 && qz >= 0 && qx < px && qz < px && dx * dx + dz * dz <= r * r {
                    img.put_pixel(qx as u32, qz as u32, Rgb(color));
                }
            }
        }
    };
    // Coloured by how used each stretch is: roads dark red and broad, tracks brown,
    // footpaths dark, faint traces pale and broken.
    use worldgen::trails::build::{TrailClass, class_of};
    for (line, widths) in c.trails.lines.iter().zip(&c.trails.line_widths) {
        for (k, (p, w)) in line.iter().zip(widths).enumerate() {
            let (x, z) = to_px(p.x, p.y);
            let (r, color) = match class_of(*w) {
                TrailClass::Road => (1, [150, 30, 20]),
                TrailClass::Track => (1, [120, 72, 30]),
                TrailClass::Footpath => (0, [56, 38, 24]),
                TrailClass::Trace if k % 2 == 0 => (0, [235, 225, 200]),
                TrailClass::Trace => continue,
            };
            dot(x, z, r, color, img);
        }
    }
    for s in &c.trails.sites {
        let (x, z) = to_px(s.x as f32, s.z as f32);
        dot(x, z, 4, [20, 20, 20], img);
        dot(x, z, 3, s.kind.map_color(), img);
    }
}

/// Length of trail in each traffic class, and crossings by kind.
fn print_trail_stats(c: &Continent) {
    use worldgen::trails::build::class_of;
    let mut km = std::collections::BTreeMap::new();
    for (line, widths) in c.trails.lines.iter().zip(&c.trails.line_widths) {
        for pair in line.windows(2).zip(widths) {
            let len = (pair.0[1] - pair.0[0]).length() / worldgen::scale::VOXELS_PER_METRE / 1000.0;
            *km.entry(format!("{:?}", class_of(*pair.1))).or_insert(0.0f32) += len;
        }
    }
    let classes: Vec<String> = km.iter().map(|(k, v)| format!("{k} {v:.1} km")).collect();
    let mut kinds = std::collections::BTreeMap::new();
    for x in &c.trails.crossings {
        *kinds.entry(format!("{:?}", x.kind)).or_insert(0) += 1;
    }
    println!("  trails: {}   crossings: {kinds:?}", classes.join(", "));
}

/// Smallest discharge drawn as a stream.
const MIN_RIVER: f32 = 24.0;

fn print_stats(c: &Continent) {
    let n = c.size * c.size;
    let land: Vec<usize> = (0..n).filter(|&i| !c.ocean[i]).collect();
    if land.is_empty() {
        println!("  all ocean");
        return;
    }
    let max_h = land.iter().map(|&i| c.height[i]).fold(f32::MIN, f32::max);
    let mean_h = land.iter().map(|&i| c.height[i]).sum::<f32>() / land.len() as f32;
    let lakes = land.iter().filter(|&&i| c.lake[i] != NO_LAKE).count();
    let streams = land.iter().filter(|&&i| c.discharge[i] >= MIN_RIVER).count();
    let max_q = land.iter().map(|&i| c.discharge[i]).fold(0.0, f32::max);
    let mut heights: Vec<f32> = land.iter().map(|&i| c.height[i]).collect();
    heights.sort_unstable_by(f32::total_cmp);
    let pct = |p: f32| heights[((heights.len() - 1) as f32 * p) as usize];
    println!(
        "  land {:.0}% of cell, {:.0} km2   height mean {mean_h:.0}, median {:.0}, p90 {:.0}, p99 {:.0}, max {max_h:.0}",
        land.len() as f32 / n as f32 * 100.0,
        land.len() as f32 * c.sim_cell_size() * c.sim_cell_size() / 1e6,
        pct(0.5),
        pct(0.9),
        pct(0.99),
    );
    let mut counts = [0usize; Biome::ALL.len()];
    for &i in &land {
        counts[c.biome[i] as usize] += 1;
    }
    let mut shares: Vec<(Biome, f32)> = Biome::ALL
        .iter()
        .map(|&b| (b, counts[b as usize] as f32 / land.len() as f32 * 100.0))
        .filter(|&(_, share)| share >= 0.5)
        .collect();
    shares.sort_by(|a, b| b.1.total_cmp(&a.1));
    let shares: Vec<String> = shares.iter().map(|(b, s)| format!("{b:?} {s:.0}%")).collect();
    println!("  biomes: {}", shares.join(", "));
    let rains: Vec<f32> = land.iter().map(|&i| c.rain[i]).collect();
    println!(
        "  rain min {:.2}, max {:.2}",
        rains.iter().copied().fold(f32::MAX, f32::min),
        rains.iter().copied().fold(f32::MIN, f32::max)
    );
    println!(
        "  lake cells {lakes} ({:.2}% of land)   stream cells {streams} ({:.1}% of land)   largest river: discharge {max_q:.0}, width {:.0} voxels",
        lakes as f32 / land.len() as f32 * 100.0,
        streams as f32 / land.len() as f32 * 100.0,
        Continent::river_width(max_q),
    );
}

/// Lists places worth looking at, with world voxel coordinates for `game --over`.
fn print_poi(c: &Continent) {
    let n = c.size * c.size;
    let cell = c.cell_size();
    let world = |i: usize| {
        let x = c.desc.mx as f32 * MACRO_SIZE as f32 + ((i % c.size) as f32 + 0.5) * cell;
        let z = c.desc.mz as f32 * MACRO_SIZE as f32 + ((i / c.size) as f32 + 0.5) * cell;
        format!("{x:.0},{z:.0}")
    };
    let local = |x: i32, z: i32| (c.desc.mx * MACRO_SIZE + x, c.desc.mz * MACRO_SIZE + z);
    for (k, s) in c.trails.sites.iter().enumerate() {
        let (x, z) = local(s.x, s.z);
        let name = c.trails.names.site(k).map_or("", |p| p.name.as_str());
        println!("  poi site       {x},{z}  {:?}, ground {}  \"{name}\"", s.kind, s.y);
    }
    // Every named place, with the tongue spoken there.
    for p in &c.trails.names.places {
        let (x, z) = local(p.at.x as i32, p.at.y as i32);
        println!(
            "  poi name       {x},{z}  {:<8} {:<10} {:>4.1}  {}",
            c.trails.names.tongue_at(p.at),
            p.kind.key(),
            p.importance,
            p.name
        );
    }
    for x in &c.trails.crossings {
        let (wx, wz) = local(x.x, x.z);
        println!("  poi crossing   {wx},{wz}  {:?}, span {}", x.kind, x.span);
    }
    // Tors on the moor, the great ones first.
    {
        use worldgen::features::tor;
        let cells = MACRO_SIZE / tor::CELL;
        let mut tors = Vec::new();
        for gz in c.desc.mz * cells..(c.desc.mz + 1) * cells {
            for gx in c.desc.mx * cells..(c.desc.mx + 1) * cells {
                tors.extend(tor::site(c.desc.world_seed, c, gx, gz));
            }
        }
        tors.sort_by(|a, b| (b.class as u8).cmp(&(a.class as u8)).then(b.dominance.total_cmp(&a.dominance)));
        for t in tors.iter().take(40) {
            let (x, z) = local(t.at.x as i32, t.at.y as i32);
            println!("  poi tor        {x},{z}  {:?}, {:.0} m from higher ground", t.class, t.dominance / 2.5);
        }
    }
    let land: Vec<usize> = (0..n).filter(|&i| !c.ocean[i]).collect();
    // Highest-scoring cells that are at least `spacing` cells apart.
    let top = |mut v: Vec<(f32, usize)>, count: usize, spacing: f32| -> Vec<(f32, usize)> {
        v.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut out: Vec<(f32, usize)> = Vec::new();
        for (score, i) in v {
            let far = out.iter().all(|&(_, j)| {
                let dx = (i % c.size) as f32 - (j % c.size) as f32;
                let dz = (i / c.size) as f32 - (j / c.size) as f32;
                dx.hypot(dz) > spacing
            });
            if far {
                out.push((score, i));
                if out.len() == count {
                    break;
                }
            }
        }
        out
    };

    let mouths = land
        .iter()
        .filter(|&&i| c.ocean[c.rcv[i] as usize])
        .map(|&i| (c.discharge[i], i))
        .collect();
    for (q, i) in top(mouths, 3, 60.0) {
        println!("  poi mouth      {}  width {:.0}", world(i), Continent::river_width(q));
    }

    let falls = land
        .iter()
        .filter(|&&i| c.discharge[i] > 600.0 && !c.ocean[c.rcv[i] as usize])
        .map(|&i| (c.water_level[i] - c.water_level[c.rcv[i] as usize], i))
        .collect();
    for (drop, i) in top(falls, 5, 40.0) {
        println!(
            "  poi falls      {}  drop {drop:.0}, width {:.0}, level {:.0}",
            world(i),
            Continent::river_width(c.discharge[i]),
            c.water_level[i]
        );
    }

    // Confluences: a cell whose second-largest tributary is itself a real river.
    let mut second = vec![0.0f32; n];
    let mut first = vec![0.0f32; n];
    for &i in &land {
        let (r, q) = (c.rcv[i] as usize, c.discharge[i]);
        if q > first[r] {
            second[r] = first[r];
            first[r] = q;
        } else if q > second[r] {
            second[r] = q;
        }
    }
    let joins = land.iter().map(|&i| (second[i], i)).collect();
    for (q, i) in top(joins, 3, 60.0) {
        println!(
            "  poi confluence {}  widths {:.0} + {:.0}",
            world(i),
            Continent::river_width(first[i]),
            Continent::river_width(q)
        );
    }

    // One solidly typical spot per biome: the land cell whose 9x9 surroundings are
    // most uniformly that biome, searched on a coarse stride.
    for b in Biome::ALL {
        let mut best: Option<(usize, usize)> = None;
        for z in (8..c.size - 8).step_by(6) {
            for x in (8..c.size - 8).step_by(6) {
                let i = z * c.size + x;
                if c.ocean[i] || c.biome[i] != b as u8 {
                    continue;
                }
                let same = (-4..=4i32)
                    .flat_map(|dz| (-4..=4i32).map(move |dx| (dx, dz)))
                    .filter(|&(dx, dz)| {
                        c.biome[((z as i32 + dz) as usize) * c.size + (x as i32 + dx) as usize] == b as u8
                    })
                    .count();
                if best.is_none_or(|(s, _)| same > s) {
                    best = Some((same, i));
                }
                if same == 81 {
                    break;
                }
            }
        }
        if let Some((_, i)) = best {
            println!("  poi biome      {}  {b:?}", world(i));
        }
    }

    let mut lakes: std::collections::HashMap<u32, (usize, f32, f32)> = Default::default();
    for &i in &land {
        if c.lake[i] != NO_LAKE {
            let e = lakes.entry(c.lake[i].to_bits()).or_default();
            e.0 += 1;
            e.1 += (i % c.size) as f32;
            e.2 += (i / c.size) as f32;
        }
    }
    let mut lakes: Vec<_> = lakes.into_iter().collect();
    lakes.sort_by_key(|l| std::cmp::Reverse(l.1.0));
    for (level, (count, sx, sz)) in lakes.into_iter().take(4) {
        let i = (sz / count as f32) as usize * c.size + (sx / count as f32) as usize;
        println!(
            "  poi lake       {}  {count} cells, level {:.0}",
            world(i),
            f32::from_bits(level)
        );
    }
}

/// Renders many macro-cells side by side on a coarse grid, to judge the world at scale.
fn atlas(o: &Options, (mx0, mz0, n): (i32, i32, i32)) -> Result<()> {
    let size = if o.size == 1024 { 256 } else { o.size };
    let px = o.cell_px;
    let cells: Vec<(i32, i32)> = (0..n)
        .flat_map(|z| (0..n).map(move |x| (mx0 + x, mz0 + z)))
        .collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map_or(4, |p| p.get()).min(8);
    let done = std::sync::Mutex::new(Vec::with_capacity(cells.len()));
    let t = Instant::now();
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(&(mx, mz)) = cells.get(k) else { break };
                    let c = continent::generate(cell_desc(o.seed, mx, mz), SimConfig { size });
                    let view = View { x0: 0.0, z0: 0.0, span: c.size as f32 };
                    let relief = render_relief(&c, view, px, Layer::Elevation);
                    let biome = render_relief(&c, view, px, Layer::Biome);
                    done.lock().unwrap().push((mx, mz, relief, biome));
                }
            });
        }
    });
    let side = px * n as u32;
    let (mut relief_img, mut biome_img) = (RgbImage::new(side, side), RgbImage::new(side, side));
    for (mx, mz, relief, biome) in done.into_inner().unwrap() {
        let (ox, oz) = (((mx - mx0) as u32) * px, ((mz - mz0) as u32) * px);
        for y in 0..px {
            for x in 0..px {
                relief_img.put_pixel(ox + x, oz + y, *relief.get_pixel(x, y));
                biome_img.put_pixel(ox + x, oz + y, *biome.get_pixel(x, y));
            }
        }
    }
    println!("atlas {n}x{n} of {size}x{size} cells in {:.1} s", t.elapsed().as_secs_f32());
    std::fs::create_dir_all(&o.out)?;
    let name = |kind: &str| o.out.join(format!("s{}_atlas_{mx0}_{mz0}_{n}_{kind}.png", o.seed));
    relief_img.save(name("relief"))?;
    biome_img.save(name("biome"))?;
    println!("  wrote {}", name("relief").display());

    // Every structure touching the atlas, once each, for finding them in the game.
    let mut seen = std::collections::HashSet::new();
    for z in 0..n {
        for x in 0..n {
            let (mx, mz) = (mx0 + x, mz0 + z);
            for s in worldgen::structures::Field::new(o.seed, mx, mz).structures() {
                if seen.insert(s.seed) {
                    let (a, b) = (s.points[0], s.points[s.points.len() - 1]);
                    println!(
                        "  {:?} length {:.0}, half-width {:.2}, amplitude {:.1}, cell {mx},{mz}: from voxel {:.0},{:.0} to {:.0},{:.0}",
                        s.kind,
                        s.length,
                        s.half_width,
                        s.amplitude,
                        a.0 * MACRO_SIZE as f64,
                        a.1 * MACRO_SIZE as f64,
                        b.0 * MACRO_SIZE as f64,
                        b.1 * MACRO_SIZE as f64
                    );
                }
            }
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let o = parse_args()?;
    if let Some(a) = o.atlas {
        return atlas(&o, a);
    }
    let desc = cell_desc(o.seed, o.cell.0, o.cell.1);
    println!(
        "seed {} cell {:?}: {:?} {:?}, ruggedness {:.2}, temperature {:.2}, wetness {:.2}",
        o.seed, o.cell, desc.kind, desc.archetype, desc.ruggedness, desc.temperature, desc.wetness
    );
    let t = Instant::now();
    let mut c = continent::generate(desc, SimConfig { size: o.size });
    println!("  simulated {0}x{0} in {1:.2} s", o.size, t.elapsed().as_secs_f32());
    let t = Instant::now();
    c.trails = worldgen::trails::build(&c, o.seed);
    c.trails.names = worldgen::names::survey(&c, o.seed);
    println!(
        "  {} landmarks, {} trails, {} crossings, {} structures in {:.2} s",
        c.trails.sites.len(),
        c.trails.lines.len(),
        c.trails.crossings.len(),
        c.trails.stamp_count(),
        t.elapsed().as_secs_f32()
    );
    print_trail_stats(&c);
    print_stats(&c);
    if o.poi {
        print_poi(&c);
    }

    std::fs::create_dir_all(&o.out)?;
    let name = |kind: &str| o.out.join(format!("s{}_{}_{}_{kind}.png", o.seed, o.cell.0, o.cell.1));
    let full = View {
        x0: 0.0,
        z0: 0.0,
        span: c.size as f32,
    };
    for (kind, layer) in [("relief", Layer::Elevation), ("biome", Layer::Biome), ("rain", Layer::Rain)] {
        let path = name(kind);
        render_relief(&c, full, o.px, layer).save(&path)?;
        println!("  wrote {}", path.display());
    }
    if let Some((cx, cz, span)) = o.zoom {
        let view = View {
            x0: cx - span * 0.5,
            z0: cz - span * 0.5,
            span,
        };
        let path = name("zoom");
        render_relief(&c, view, o.px, Layer::Biome).save(&path)?;
        println!("  wrote {}", path.display());
    }
    Ok(())
}
