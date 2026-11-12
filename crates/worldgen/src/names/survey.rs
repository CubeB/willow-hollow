//! Reads a continent for the things worth naming, and what each is like.

use glam::Vec2;

use super::{Facts, Gazetteer, PlaceKind, Request};
use crate::{
    biome::Biome,
    continent::{Continent, NO_LAKE},
    macro_layout::{CellKind, MACRO_SIZE},
    noise::{fbm2, hash2},
    scale::{OLD_VOXEL, VOXELS_PER_METRE, m, m64},
    structures::{self, Field},
    trails::sites,
};

/// Names every place worth naming on a continent whose trails have been built.
pub fn survey(c: &Continent, world_seed: u32) -> Gazetteer {
    let s = Survey::new(c, world_seed);
    let mut g = Gazetteer::new(c.desc.seed);
    if s.land_km2 <= 0.0 {
        return g;
    }
    // Natural features first, so what is built beside them can borrow their names.
    s.regions(&mut g);
    s.structures(&mut g);
    s.islands(&mut g);
    s.lakes(&mut g);
    s.rivers(&mut g);
    s.falls(&mut g);
    s.peaks(&mut g);
    s.bays(&mut g);
    s.landmarks(&mut g);
    g
}

struct Survey<'a> {
    c: &'a Continent,
    world_seed: u32,
    n: usize,
    /// Voxels per grid cell.
    cs: f32,
    /// Grid cells per cell of the full 1024 grid, so sizes hold at any resolution.
    scale: f32,
    land_km2: f32,
}

impl<'a> Survey<'a> {
    fn new(c: &'a Continent, world_seed: u32) -> Self {
        let n = c.size;
        let cs = c.cell_size();
        let land = c.ocean.iter().filter(|&&o| !o).count();
        Self {
            c,
            world_seed,
            n,
            cs,
            scale: n as f32 / 1024.0,
            land_km2: land as f32 * (cs / VOXELS_PER_METRE / 1000.0).powi(2),
        }
    }

    /// Continent-local voxel position of a cell's centre.
    fn at(&self, i: usize) -> Vec2 {
        Vec2::new(((i % self.n) as f32 + 0.5) * self.cs, ((i / self.n) as f32 + 0.5) * self.cs)
    }

    /// The cell holding a continent-local voxel position.
    fn index(&self, p: Vec2) -> usize {
        let last = self.n as i32 - 1;
        self.cell((p.x / self.cs) as i32, (p.y / self.cs) as i32).min(self.cell(last, last))
    }

    fn cell(&self, x: i32, z: i32) -> usize {
        let last = self.n as i32 - 1;
        (z.clamp(0, last) * self.n as i32 + x.clamp(0, last)) as usize
    }

    /// A distance given in cells of the 1024 grid, in cells of this one.
    fn cells(&self, r: f32) -> i32 {
        ((r * self.scale).round() as i32).max(1)
    }

    /// Discharge as it would be counted on the 1024 grid.
    fn q(&self, i: usize) -> f32 {
        self.c.discharge[i] / (self.scale * self.scale)
    }

    /// Eight cells `r` away from `i`.
    fn ring(&self, i: usize, r: i32) -> [usize; 8] {
        let (x, z) = ((i % self.n) as i32, (i / self.n) as i32);
        [(r, 0), (-r, 0), (0, r), (0, -r), (r, r), (r, -r), (-r, r), (-r, -r)].map(|(dx, dz)| self.cell(x + dx, z + dz))
    }

    /// What describes the surroundings of cell `i`, after the tags in `first`.
    fn facts(&self, i: usize, first: &[&'static str]) -> Facts {
        let c = self.c;
        let mut tags: Vec<&'static str> = first.to_vec();
        let mut add = |t: &'static str| {
            if !tags.contains(&t) {
                tags.push(t);
            }
        };
        let biome = Biome::from_u8(c.biome[i]);
        // Wet ground and water close by say the most about a place.
        match biome {
            Biome::Marsh => add("marsh"),
            Biome::Bog => add("bog"),
            Biome::Mangrove => add("mangrove"),
            _ => {}
        }
        let near = self.ring(i, self.cells(3.0));
        if c.lake[i] != NO_LAKE || near.iter().any(|&j| c.lake[j] != NO_LAKE) {
            add("lake");
        }
        if near.iter().any(|&j| c.ocean[j]) {
            add(if c.height[i] < 4.0 { "shore" } else { "sea" });
        }
        if near.iter().map(|&j| self.q(j)).fold(self.q(i), f32::max) >= 400.0 {
            add("river");
        }
        if let Some(t) = self.biome_tag(i, biome) {
            add(t);
        }
        let around = self.ring(i, self.cells(12.0));
        let relief = c.height[i] - around.iter().map(|&j| c.height[j]).sum::<f32>() / 8.0;
        if c.height[i] > 220.0 && relief > 25.0 {
            add("peak");
        } else if relief > 8.0 {
            add("hill");
        } else if relief < -8.0 {
            add("valley");
        }
        Facts { tags, residents: Vec::new() }
    }

    /// The forest or ground of a biome. Broadleaf forest is told apart by the groves
    /// that make it up, by the same noise the tree placement uses (`features::place`),
    /// so a maple grove that turns gold in autumn gives a gold name.
    fn biome_tag(&self, i: usize, biome: Biome) -> Option<&'static str> {
        let p = self.at(i);
        let (wx, wz) = (
            self.c.desc.mx as f64 * MACRO_SIZE as f64 + p.x as f64,
            self.c.desc.mz as f64 * MACRO_SIZE as f64 + p.y as f64,
        );
        let grove = || fbm2(self.world_seed ^ 0x9e0f, wx / m64(112.0), wz / m64(112.0), 2, 2.0, 0.5);
        Some(match biome {
            Biome::Deciduous => match grove() {
                g if g > 0.38 => "maple",
                g if g < -0.42 => "birch",
                _ => "oak",
            },
            Biome::Woodland => match grove() {
                g if g > 0.38 => "maple",
                g if g < -0.40 => "cherry",
                _ => "woodland",
            },
            Biome::BirchGrove => "birch",
            Biome::Taiga => "taiga",
            Biome::SnowyPine => "snowypine",
            Biome::OldGrowth => "oldgrowth",
            Biome::Rainforest => "rainforest",
            Biome::Heath => "heath",
            Biome::Grassland => "grassland",
            Biome::Steppe => "steppe",
            Biome::Savanna => "savanna",
            Biome::Desert => "desert",
            Biome::DuneSea => "dunesea",
            Biome::RedRock => "redrock",
            Biome::Volcanic => "volcanic",
            Biome::Tundra => "tundra",
            Biome::Alpine => "alpine",
            Biome::AlpineMeadow => "alpinemeadow",
            Biome::Marsh => "marsh",
            Biome::Bog => "bog",
            Biome::Mangrove => "mangrove",
            Biome::Moor => "moor",
            Biome::Ocean => return None,
        })
    }

    /// The tongue a climate speaks: `t` its temperature, `wet` its rainfall.
    fn tongue_for(&self, t: f32, wet: f32) -> &'static str {
        let islands = self.c.desc.kind == CellKind::Islands;
        match () {
            _ if t < 0.26 => "north",
            _ if islands && t > 0.45 => "isle",
            _ if t < 0.58 => {
                if wet >= 1.05 {
                    "vale"
                } else {
                    "old"
                }
            }
            _ if wet < 0.5 => "dune",
            _ if wet < 1.1 => "sun",
            _ => "isle",
        }
    }

    /// A few regions spread over the land, each speaking the tongue of its climate.
    fn regions(&self, g: &mut Gazetteer) {
        let (c, n) = (self.c, self.n);
        let stride = (n / 128).max(1);
        let land: Vec<usize> = (0..n)
            .step_by(stride)
            .flat_map(|z| (0..n).step_by(stride).map(move |x| z * n + x))
            .filter(|&i| !c.ocean[i])
            .collect();
        if land.is_empty() {
            return;
        }
        let count = ((1.0 + self.land_km2 / 120.0) as usize).clamp(1, 5).min(land.len());
        // Spread out by picking the farthest cell each time, then settled toward the
        // middle of the land each one holds.
        let mut points = vec![self.at(land[hash2(c.desc.seed, 41, 0) as usize % land.len()])];
        while points.len() < count {
            let far = |i: usize| points.iter().map(|p| p.distance(self.at(i))).fold(f32::MAX, f32::min);
            let next = land.iter().copied().max_by(|&a, &b| far(a).total_cmp(&far(b)).then(b.cmp(&a))).unwrap_or(land[0]);
            points.push(self.at(next));
        }
        let nearest = |points: &[Vec2], p: Vec2| {
            (0..points.len()).min_by(|&a, &b| points[a].distance(p).total_cmp(&points[b].distance(p))).unwrap_or(0)
        };
        for _ in 0..3 {
            let mut sum = vec![(Vec2::ZERO, 0usize); points.len()];
            for &i in &land {
                let k = nearest(&points, self.at(i));
                sum[k].0 += self.at(i);
                sum[k].1 += 1;
            }
            for (p, (s, count)) in points.iter_mut().zip(sum) {
                if count > 0 {
                    *p = s / count as f32;
                }
            }
        }
        // Each region's climate, the commonest ground in it, and the land cell nearest
        // its middle, where its name goes.
        g.set_regions(&points.iter().map(|&p| (p, "")).collect::<Vec<_>>());
        struct Stats {
            t: f32,
            wet: f32,
            count: usize,
            biomes: [u32; Biome::ALL.len()],
            label: Option<(f32, usize)>,
        }
        let mut stats: Vec<Stats> = points
            .iter()
            .map(|_| Stats { t: 0.0, wet: 0.0, count: 0, biomes: [0; Biome::ALL.len()], label: None })
            .collect();
        for &i in &land {
            let Some(k) = g.region_index(self.at(i)) else { continue };
            let s = &mut stats[k];
            s.t += c.temperature[i];
            s.wet += c.rain[i];
            s.count += 1;
            s.biomes[c.biome[i] as usize] += 1;
            let d = self.at(i).distance(points[k]);
            if s.label.is_none_or(|(best, _)| d < best) {
                s.label = Some((d, i));
            }
        }
        for (k, s) in stats.iter().enumerate() {
            let tongue = if s.count == 0 {
                "old"
            } else {
                self.tongue_for(s.t / s.count as f32, s.wet / s.count as f32 * c.desc.wetness)
            };
            g.regions[k].tongue = super::words().tongue_named(tongue);
            let Some((_, i)) = s.label else { continue };
            let common = (0..s.biomes.len()).max_by_key(|&b| (s.biomes[b], std::cmp::Reverse(b))).unwrap_or(0);
            let facts = Facts {
                tags: self.biome_tag(i, Biome::from_u8(common as u8)).into_iter().collect(),
                residents: Vec::new(),
            };
            let spacing = stride as f32 * self.cs;
            let place = g.add(Request {
                kind: PlaceKind::Region,
                at: self.at(i),
                radius: (s.count as f32 / std::f32::consts::PI).sqrt() * spacing,
                line: Vec::new(),
                importance: 30.0,
                facts: &facts,
                salt: 0x2e61 ^ (k as u32).wrapping_mul(7919),
            });
            g.regions[k].place = Some(place);
        }
    }

    /// Mountain ranges, faults and rifts, which are far larger than one continent and
    /// keep the same name on each they cross.
    fn structures(&self, g: &mut Gazetteer) {
        let c = self.c;
        let (mx, mz) = (c.desc.mx, c.desc.mz);
        let size = MACRO_SIZE as f32;
        for s in Field::new(self.world_seed, mx, mz).structures() {
            let (kind, tag, importance, radius) = match s.kind {
                structures::Kind::Range => (PlaceKind::Range, "range", 20.0, s.half_width as f32 * size * 0.5),
                structures::Kind::Fault => (PlaceKind::Fault, "fault", 12.0, m(400.0)),
                structures::Kind::Rift => (PlaceKind::Rift, "rift", 12.0, s.half_width as f32 * size),
            };
            let line: Vec<Vec2> = s
                .points
                .iter()
                .map(|&(x, z)| Vec2::new(((x - mx as f64) * MACRO_SIZE as f64) as f32, ((z - mz as f64) * MACRO_SIZE as f64) as f32))
                .filter(|p| p.x >= 0.0 && p.y >= 0.0 && p.x < size && p.y < size && !c.ocean[self.index(*p)])
                .collect();
            if line.len() < 4 {
                continue;
            }
            let facts = Facts { tags: vec![tag], residents: Vec::new() };
            g.add(Request {
                kind,
                at: line[line.len() / 2],
                radius,
                line,
                importance,
                facts: &facts,
                salt: s.seed,
            });
        }
    }

    /// Connected cells for which `inside` holds, by 4-neighbour flood fill: a label per
    /// cell (0 outside) and the size of each labelled group, label k + 1 at index k.
    fn groups(&self, inside: impl Fn(usize) -> bool) -> (Vec<u32>, Vec<usize>) {
        let n = self.n;
        let mut label = vec![0u32; n * n];
        let (mut sizes, mut stack) = (Vec::new(), Vec::new());
        for start in 0..n * n {
            if label[start] != 0 || !inside(start) {
                continue;
            }
            sizes.push(0usize);
            let id = sizes.len() as u32;
            label[start] = id;
            stack.push(start);
            while let Some(i) = stack.pop() {
                sizes[id as usize - 1] += 1;
                let (x, z) = (i % n, i / n);
                let mut visit = |j: usize| {
                    if label[j] == 0 && inside(j) {
                        label[j] = id;
                        stack.push(j);
                    }
                };
                if x > 0 {
                    visit(i - 1);
                }
                if x + 1 < n {
                    visit(i + 1);
                }
                if z > 0 {
                    visit(i - n);
                }
                if z + 1 < n {
                    visit(i + n);
                }
            }
        }
        (label, sizes)
    }

    /// The cell of each wanted group nearest that group's centroid, by group index.
    fn middles(&self, label: &[u32], wanted: &[usize]) -> Vec<usize> {
        let groups = label.iter().copied().max().unwrap_or(0) as usize;
        let mut slot = vec![usize::MAX; groups];
        for (k, &g) in wanted.iter().enumerate() {
            slot[g] = k;
        }
        let mut sum = vec![(Vec2::ZERO, 0usize); wanted.len()];
        for (i, &l) in label.iter().enumerate() {
            if l != 0 && slot[l as usize - 1] != usize::MAX {
                let k = slot[l as usize - 1];
                sum[k].0 += self.at(i);
                sum[k].1 += 1;
            }
        }
        let centre: Vec<Vec2> = sum.iter().map(|&(s, n)| s / n.max(1) as f32).collect();
        let mut best = vec![(f32::MAX, 0usize); wanted.len()];
        for (i, &l) in label.iter().enumerate() {
            if l != 0 && slot[l as usize - 1] != usize::MAX {
                let k = slot[l as usize - 1];
                let d = self.at(i).distance(centre[k]);
                if d < best[k].0 {
                    best[k] = (d, i);
                }
            }
        }
        best.into_iter().map(|(_, i)| i).collect()
    }

    /// The largest groups of at least `min` cells, largest first, at most `most`.
    fn largest(sizes: &[usize], min: usize, most: usize) -> Vec<usize> {
        let mut order: Vec<usize> = (0..sizes.len()).filter(|&k| sizes[k] >= min).collect();
        order.sort_by(|&a, &b| sizes[b].cmp(&sizes[a]).then(a.cmp(&b)));
        order.truncate(most);
        order
    }

    fn islands(&self, g: &mut Gazetteer) {
        let c = self.c;
        let (label, sizes) = self.groups(|i| !c.ocean[i]);
        let min = ((60.0 * self.scale * self.scale) as usize).max(4);
        let mut order = Self::largest(&sizes, min, 15);
        // On a continent the largest landmass is the mainland, not an island.
        let land: usize = sizes.iter().sum();
        if c.desc.kind == CellKind::Continent && order.first().is_some_and(|&k| sizes[k] * 10 > land * 3) {
            order.remove(0);
        }
        for (&k, i) in order.iter().zip(self.middles(&label, &order)) {
            let facts = self.facts(i, &[]);
            g.add(Request {
                kind: PlaceKind::Island,
                at: self.at(i),
                radius: (sizes[k] as f32 / std::f32::consts::PI).sqrt() * self.cs,
                line: Vec::new(),
                importance: (4.0 + (sizes[k] as f32 / min as f32).ln() * 2.0).clamp(4.0, 16.0),
                facts: &facts,
                salt: 0x1517 ^ i as u32,
            });
        }
    }

    fn lakes(&self, g: &mut Gazetteer) {
        let c = self.c;
        let (label, sizes) = self.groups(|i| c.lake[i] != NO_LAKE);
        let min = ((20.0 * self.scale * self.scale) as usize).max(3);
        let order = Self::largest(&sizes, min, 14);
        for (&k, i) in order.iter().zip(self.middles(&label, &order)) {
            let facts = self.facts(i, &["lake"]);
            g.add(Request {
                kind: PlaceKind::Lake,
                at: self.at(i),
                radius: (sizes[k] as f32 / std::f32::consts::PI).sqrt() * self.cs,
                line: Vec::new(),
                importance: (4.0 + (sizes[k] as f32 / min as f32).ln() * 1.5).clamp(4.0, 12.0),
                facts: &facts,
                salt: 0x1a4e ^ i as u32,
            });
        }
    }

    /// The great rivers, each traced from its mouth up its largest branch, then their
    /// largest tributaries.
    fn rivers(&self, g: &mut Gazetteer) {
        let c = self.c;
        let cells = self.n * self.n;
        // The largest stream flowing into each cell.
        let mut up = vec![u32::MAX; cells];
        for i in 0..cells {
            let r = c.rcv[i] as usize;
            if c.ocean[i] || r == i {
                continue;
            }
            if up[r] == u32::MAX || c.discharge[i] > c.discharge[up[r] as usize] {
                up[r] = i as u32;
            }
        }
        let mut on_river = vec![false; cells];
        let by_size = |mut v: Vec<usize>| {
            v.sort_by(|&a, &b| c.discharge[b].total_cmp(&c.discharge[a]).then(a.cmp(&b)));
            v
        };
        let mouths = by_size(
            (0..cells)
                .filter(|&i| {
                    let r = c.rcv[i] as usize;
                    !c.ocean[i] && c.lake[i] == NO_LAKE && r != i && (c.ocean[r] || c.lake[r] != NO_LAKE) && self.q(i) >= 2000.0
                })
                .collect(),
        );
        let mut named = 0;
        for mouth in mouths {
            if named >= 10 {
                break;
            }
            if self.add_river(g, mouth, &up, &mut on_river) {
                named += 1;
            }
        }
        let branches = by_size(
            (0..cells)
                .filter(|&i| !c.ocean[i] && !on_river[i] && on_river[c.rcv[i] as usize] && self.q(i) >= 2500.0)
                .collect(),
        );
        let mut named = 0;
        for start in branches {
            if named >= 6 {
                break;
            }
            if self.add_river(g, start, &up, &mut on_river) {
                named += 1;
            }
        }
    }

    /// Traces and names the river whose lowest cell is `start`, unless it is too short
    /// or already named. Returns whether it was named.
    fn add_river(&self, g: &mut Gazetteer, start: usize, up: &[u32], on_river: &mut [bool]) -> bool {
        let c = self.c;
        if on_river[start] {
            return false;
        }
        // Up the largest branch until the river has dwindled to a stream.
        let floor = (c.discharge[start] * 0.04).max(150.0 * self.scale * self.scale);
        let (mut path, mut cur) = (Vec::new(), start);
        loop {
            if on_river[cur] || path.len() > 4 * self.n {
                break;
            }
            path.push(cur);
            let next = up[cur];
            if next == u32::MAX || c.discharge[next as usize] < floor {
                break;
            }
            cur = next as usize;
        }
        if path.len() < self.cells(24.0) as usize {
            return false;
        }
        for &i in &path {
            on_river[i] = true;
        }
        let step = self.cells(6.0) as usize;
        let mut line: Vec<Vec2> = path.iter().step_by(step).map(|&i| self.at(i)).collect();
        if (path.len() - 1) % step != 0 {
            line.push(self.at(path[path.len() - 1]));
        }
        let label = path[path.len() * 2 / 5];
        let facts = self.facts(label, &["river"]);
        let q = self.q(start);
        g.add(Request {
            kind: PlaceKind::River,
            at: self.at(label),
            radius: (Continent::river_width(c.discharge[start]) * OLD_VOXEL * 0.5).max(m(6.0)),
            line,
            importance: (4.0 + (q / 2000.0).ln() * 2.0).clamp(4.0, 14.0),
            facts: &facts,
            salt: 0x5171 ^ start as u32,
        });
        true
    }

    /// The biggest drops of a sizeable stream.
    fn falls(&self, g: &mut Gazetteer) {
        let c = self.c;
        let mut found: Vec<(f32, usize)> = (0..self.n * self.n)
            .filter_map(|i| {
                let r = c.rcv[i] as usize;
                if c.ocean[i] || c.ocean[r] || self.q(i) < 600.0 {
                    return None;
                }
                let drop = c.water_level[i] - c.water_level[r];
                (drop >= 6.0).then_some((drop, i))
            })
            .collect();
        found.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        let gap = self.cells(40.0) as f32 * self.cs;
        let mut kept: Vec<Vec2> = Vec::new();
        for (_, i) in found {
            if kept.len() >= 8 {
                break;
            }
            let p = self.at(i);
            if kept.iter().any(|k| k.distance(p) < gap) {
                continue;
            }
            kept.push(p);
            let facts = self.facts(i, &["falls"]);
            g.add(Request {
                kind: PlaceKind::Falls,
                at: p,
                radius: m(20.0),
                line: Vec::new(),
                importance: 5.0,
                facts: &facts,
                salt: 0xfa11 ^ i as u32,
            });
        }
    }

    /// Summits standing well above everything around them.
    fn peaks(&self, g: &mut Gazetteer) {
        let (c, n) = (self.c, self.n as i32);
        let stride = self.cells(2.0) as usize;
        let rings = [2.0, 4.0, 7.0, 10.0].map(|r| self.cells(r));
        let far = self.cells(24.0);
        let mut found = Vec::new();
        for z in (far..n - far).step_by(stride) {
            for x in (far..n - far).step_by(stride) {
                let i = self.cell(x, z);
                let h = c.height[i];
                if c.ocean[i] || h < 120.0 {
                    continue;
                }
                if rings.iter().any(|&r| self.ring(i, r).iter().any(|&j| c.height[j] > h)) {
                    continue;
                }
                let base = self.ring(i, far).iter().map(|&j| c.height[j]).sum::<f32>() / 8.0;
                if h - base >= 50.0 {
                    found.push((h, i));
                }
            }
        }
        found.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        let gap = self.cells(60.0) as f32 * self.cs;
        let most = ((self.land_km2 / 25.0) as usize).clamp(3, 14);
        let mut kept: Vec<Vec2> = Vec::new();
        for (h, i) in found {
            if kept.len() >= most {
                break;
            }
            let p = self.at(i);
            if kept.iter().any(|k| k.distance(p) < gap) {
                continue;
            }
            kept.push(p);
            let facts = self.facts(i, &["peak"]);
            g.add(Request {
                kind: PlaceKind::Peak,
                at: p,
                radius: m(60.0),
                line: Vec::new(),
                importance: 5.0 + (h / 200.0).min(4.0),
                facts: &facts,
                salt: 0x9eac ^ i as u32,
            });
        }
    }

    /// Sea mostly enclosed by land: bays, and long narrow inlets.
    fn bays(&self, g: &mut Gazetteer) {
        let (c, n) = (self.c, self.n as i32);
        let b = self.cells(2.0);
        let w = (n / b) as usize;
        let (reach, step, coast) = (self.cells(48.0), self.cells(2.0), self.cells(4.0));
        let mut bay = vec![false; w * w];
        for gz in 0..w {
            for gx in 0..w {
                let (x, z) = (gx as i32 * b, gz as i32 * b);
                let i = self.cell(x, z);
                if !c.ocean[i] || self.ring(i, coast).iter().all(|&j| c.ocean[j]) {
                    continue;
                }
                let mut hits = 0;
                for k in 0..16 {
                    let (sin, cos) = (k as f32 * std::f32::consts::TAU / 16.0).sin_cos();
                    for s in (step..=reach).step_by(step as usize) {
                        let (px, pz) = (x + (cos * s as f32) as i32, z + (sin * s as f32) as i32);
                        if px < 0 || pz < 0 || px >= n || pz >= n {
                            break;
                        }
                        if !c.ocean[self.cell(px, pz)] {
                            hits += 1;
                            break;
                        }
                    }
                }
                bay[gz * w + gx] = hits >= 12;
            }
        }
        // Group the enclosed water on the coarse grid.
        let mut label = vec![0u32; w * w];
        let mut members: Vec<Vec<usize>> = Vec::new();
        for start in 0..w * w {
            if !bay[start] || label[start] != 0 {
                continue;
            }
            members.push(Vec::new());
            let id = members.len() as u32;
            label[start] = id;
            let mut stack = vec![start];
            while let Some(k) = stack.pop() {
                members[id as usize - 1].push(k);
                let (x, z) = (k % w, k / w);
                for (dx, dz) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                    let (nx, nz) = (x as i32 + dx, z as i32 + dz);
                    if nx < 0 || nz < 0 || nx >= w as i32 || nz >= w as i32 {
                        continue;
                    }
                    let j = nz as usize * w + nx as usize;
                    if bay[j] && label[j] == 0 {
                        label[j] = id;
                        stack.push(j);
                    }
                }
            }
        }
        let sizes: Vec<usize> = members.iter().map(Vec::len).collect();
        let spacing = b as f32 * self.cs;
        for k in Self::largest(&sizes, 10, 8) {
            let pts: Vec<Vec2> = members[k]
                .iter()
                .map(|&j| Vec2::new(((j % w) as f32 + 0.5) * spacing, ((j / w) as f32 + 0.5) * spacing))
                .collect();
            let centre = pts.iter().copied().sum::<Vec2>() / pts.len() as f32;
            // Long and narrow if the spread along one axis far exceeds the other.
            let (mut sxx, mut sxz, mut szz) = (0.0f32, 0.0f32, 0.0f32);
            for p in &pts {
                let d = *p - centre;
                sxx += d.x * d.x;
                sxz += d.x * d.y;
                szz += d.y * d.y;
            }
            let (mean, diff) = ((sxx + szz) * 0.5, ((sxx - szz) * 0.5).hypot(sxz));
            let narrow = (mean + diff) > 5.0 * (mean - diff).max(1e-3);
            let label_at = pts.iter().copied().min_by(|a, b| a.distance(centre).total_cmp(&b.distance(centre))).unwrap_or(centre);
            let i = self.index(label_at);
            let facts = self.facts(i, &["sea"]);
            g.add(Request {
                kind: if narrow { PlaceKind::Inlet } else { PlaceKind::Bay },
                at: label_at,
                radius: (pts.len() as f32 / std::f32::consts::PI).sqrt() * spacing,
                line: Vec::new(),
                importance: (6.0 + (pts.len() as f32 / 10.0).ln()).clamp(6.0, 10.0),
                facts: &facts,
                salt: 0x0ba7 ^ i as u32,
            });
        }
    }

    /// Every landmark on the trails.
    fn landmarks(&self, g: &mut Gazetteer) {
        for (k, site) in self.c.trails.sites.iter().enumerate() {
            let at = Vec2::new(site.x as f32, site.z as f32);
            let facts = self.facts(self.index(at), &[]);
            g.add_site(
                k,
                Request {
                    kind: PlaceKind::Landmark(site.kind),
                    at,
                    radius: site.kind.radius(),
                    line: Vec::new(),
                    importance: sites::importance(site.kind),
                    facts: &facts,
                    salt: hash2(0x517e, site.x, site.z),
                },
            );
        }
    }
}
