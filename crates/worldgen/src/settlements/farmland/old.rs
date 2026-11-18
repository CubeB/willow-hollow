//! Old fields (#64): an ancient place once worked its land further out than it does now,
//! and beyond its fields of today lie the fields it gave up long ago, gone back to the
//! wild. What is left of them is their edges: first a rough hedgerow grown out and full of
//! gaps, further out a line of scrub where one was, and furthest out a stone here and
//! there where a wall ran, or a low grassed bank. About a ruin, the fields of the people
//! who lived there lie grown over, and their tumbled walls run on through the young wood.
//!
//! The fields themselves are the wild's now: nothing is sown, grazed or cleared in them,
//! and they are no settlement's land (see [`super::fields::plot`], which knows nothing of
//! them). Only their edges are built, with the fields' own boundaries (see
//! [`super::bounds`]).
//!
//! Like every field, an old one is worked out from its column alone, so two chunks agree
//! about every edge on their border.

use glam::{Vec2, vec2};

use super::{
    bounds::Style,
    fields::bend,
    influence::{EDGE, Farm, Land, sample},
};
use crate::{
    continent::Continent,
    features::landmarks::{Landmark, Palette},
    macro_layout::MACRO_SIZE,
    materials as mat,
    noise::{hash2, hash3, perlin2, unit},
    scale::{m, m64},
    settlements::{Age, Kind, Region},
    surface::Column,
};

/// Where fields were once worked that are worked no longer: beyond an ancient place's
/// fields, or about a ruin.
#[derive(Clone, Debug)]
pub struct Old {
    /// The middle of the old land, continent-local.
    pub at: Vec2,
    /// How far out from `at` the fields of today reach, toward each of [`RAYS`] ways
    /// round (the first along +x, turning toward +z): the old fields begin there. A ruin
    /// has its own ground's edge all round instead.
    pub reach: Vec<f32>,
    /// How far beyond `reach` the last of the old fields lies, in voxels.
    pub span: f32,
    /// What their walls and banks were built of: the ancient place's own, or for a ruin,
    /// what the ruin is built of (see [`Land::farm`]).
    pub farm: Farm,
    /// The settlement it is the old land of, by its index in [`Land::farms`]; none for a
    /// ruin.
    pub owner: Option<usize>,
}

/// How many ways round an ancient place the reach of its fields of today is kept.
pub const RAYS: usize = 48;

/// One column of an old field.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct OldPlot {
    /// The field's own number: the same on every column of it, and different from every
    /// old field about it.
    pub id: u32,
    /// How far out among the old fields it lies: 0 where they begin, 1 at the last of
    /// them.
    pub out: f32,
    /// Whose old land it is, by its index in [`Land::old`].
    pub source: usize,
}

/// Spacing of the seeded points the old fields are cut about, in voxels: smaller fields
/// than today's, as they were cut long ago.
const LATTICE: f32 = 52.0;

/// The old land of continent `c`, whose settlements' `farms` are worked out: every
/// ancient place's, and every ruin's. `seed` is the world's.
pub(super) fn gather(c: &Continent, seed: u32, land: &Land) -> Vec<Old> {
    let mut out = Vec::new();
    for (k, f) in land.farms.iter().enumerate() {
        if f.age != Age::Ancient {
            continue;
        }
        // The furthest out the fields of today reach each way round, so the old ones
        // begin where they end; and they run on out half as far again.
        let far = f.radius + f.reach * 1.3;
        let reach = (0..RAYS)
            .map(|a| {
                let dir = Vec2::from_angle(a as f32 / RAYS as f32 * std::f32::consts::TAU);
                let mut last = f.radius;
                let mut d = f.radius;
                while d < far {
                    let p = f.at + dir * d;
                    if land.managed_at(p) >= EDGE && land.owner(p.x as i32, p.y as i32) == Some(k) {
                        last = d;
                    }
                    d += 8.0;
                }
                last
            })
            .collect();
        out.push(Old { at: f.at, reach, span: f.reach * 0.55, farm: f.clone(), owner: Some(k) });
    }
    for s in c.trails.sites.iter().filter(|s| s.kind == Landmark::Ruins) {
        let at = vec2(s.x as f32 + 0.5, s.z as f32 + 0.5);
        let g = sample(c, at);
        if g.water {
            continue;
        }
        let (wx, wz) = (c.desc.mx * MACRO_SIZE + s.x, c.desc.mz * MACRO_SIZE + s.z);
        let pal = Palette::at(seed, g.biome, wx, s.y - 1, wz);
        let farm = Farm {
            at,
            radius: m(12.0),
            reach: m(60.0),
            kind: Kind::Hamlet,
            age: Age::Ancient,
            region: if g.biome.arid() { Region::Desert } else { Region::Temperate },
            prosperity: 0.0,
            coast: false,
            seed: hash2(seed ^ 0x01d0, s.x, s.z),
            biome: g.biome,
            temperature: g.temperature,
            stone: pal.stone,
            stone_dark: pal.stone_dark,
            wood: crate::features::timber(g.biome).unwrap_or(mat::WOOD_DEAD),
        };
        out.push(Old { at, reach: vec![m(16.0)], span: m(58.0), farm, owner: None });
    }
    out
}

impl Old {
    /// How far out from its middle the old fields begin toward continent-local `p`.
    fn begin(&self, p: Vec2) -> f32 {
        if self.reach.len() == 1 {
            return self.reach[0];
        }
        let d = p - self.at;
        let a = d.y.atan2(d.x).rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU * RAYS as f32;
        let (k, t) = (a.floor() as usize % RAYS, a.fract());
        self.reach[k] + (self.reach[(k + 1) % RAYS] - self.reach[k]) * t
    }

    /// Whether any of it lies within the square of continent-local columns from
    /// `(x0, z0)`, `n` wide.
    fn near(&self, x0: i32, z0: i32, n: i32) -> bool {
        let far = self.reach.iter().copied().fold(0.0f32, f32::max) + self.span;
        let lo = vec2(x0 as f32, z0 as f32);
        let hi = lo + n as f32;
        self.at.clamp(lo, hi).distance(self.at) < far
    }
}

/// Whether any old field lies within the square of continent-local columns from
/// `(x0, z0)`, `n` wide: the quick way for a chunk to know it has none.
pub fn near(land: &Land, x0: i32, z0: i32, n: i32) -> bool {
    land.old.iter().any(|o| o.near(x0, z0, n))
}

/// The old field continent-local column `(lx, lz)` of `c` lies in, whose ground is `col`,
/// if any: never where a field is worked today, and never on ground that could not have
/// been worked. `seed` is the world's.
pub fn at(c: &Continent, seed: u32, lx: i32, lz: i32, col: &Column) -> Option<OldPlot> {
    let land = &c.land;
    if land.old.is_empty() || !super::fields::worked(col) || land.managed(lx, lz) >= EDGE {
        return None;
    }
    let p = vec2(lx as f32 + 0.5, lz as f32 + 0.5);
    let (wx, wz) = ((c.desc.mx * MACRO_SIZE + lx) as f64, (c.desc.mz * MACRO_SIZE + lz) as f64);
    // The last of them lies further out here and less far there.
    let fray = perlin2(seed ^ 0x01d1, wx / m64(45.0), wz / m64(45.0)) * 0.3;
    let (source, out) = land
        .old
        .iter()
        .enumerate()
        .filter_map(|(k, o)| {
            let out = (o.at.distance(p) - o.begin(p)) / o.span;
            (out >= 0.0 && out + fray < 1.0).then_some((k, out.max(0.0)))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))?;
    let key = c.desc.seed ^ 0x01d_f1e1 ^ (source as u32).wrapping_mul(0x9e37_79b9);
    let q = p + bend(seed, wx, wz);
    let (gi, gj) = ((q.x / LATTICE).floor() as i32, (q.y / LATTICE).floor() as i32);
    let mut best = (f32::MAX, 0, 0);
    for dj in -1..=1 {
        for di in -1..=1 {
            let (i, j) = (gi + di, gj + dj);
            let s = vec2(i as f32 + 0.15 + 0.7 * unit(hash3(key, i, j, 1)), j as f32 + 0.15 + 0.7 * unit(hash3(key, i, j, 2))) * LATTICE;
            let d = s.distance_squared(q);
            if d < best.0 {
                best = (d, i, j);
            }
        }
    }
    Some(OldPlot { id: hash3(key, best.1, best.2, 0x01d) | 1, out, source })
}

/// What is left of the edge between old fields `a` and `b` of `land`, whose line runs
/// over ground `col`: a rough hedgerow, a line of scrub, or a stone or two or a low bank,
/// the further out the less; about a ruin, a tumbled wall.
pub(super) fn style(land: &Land, a: &OldPlot, b: &OldPlot, col: &Column) -> Style {
    let (lo, hi) = if a.id < b.id { (a, b) } else { (b, a) };
    let old = &land.old[lo.source];
    let farm = &old.farm;
    // Each stretch was let go in its own time.
    let out = (lo.out + hi.out) * 0.5 + (unit(hash2(lo.id, hi.id as i32, 0x01d2)) - 0.5) * 0.2;
    use crate::biome::Biome::*;
    let stony = matches!(farm.region, Region::Upland | Region::Alpine)
        || matches!(col.biome, Moor | Heath | Tundra | AlpineMeadow | Volcanic | Steppe)
        || col.biome.arid()
        || col.biome == Savanna;
    if old.owner.is_none() {
        return if out < 0.5 { Style::Wall } else if stony || out < 0.75 { Style::Stones } else { Style::Bank };
    }
    match out {
        o if o < 0.32 => if stony { Style::Wall } else { Style::Hedge },
        o if o < 0.62 => if stony { Style::Stones } else { Style::Scrub },
        _ => if stony { Style::Stones } else { Style::Bank },
    }
}

#[cfg(test)]
mod tests {
    use voxel::{CHUNK, PAD};

    use super::*;
    use crate::{
        World,
        continent::SimConfig,
        settlements::farmland::{bounds, fields::Patch},
        surface::Sampler,
        world::GameWorld,
    };

    /// Old field edges lie only where no field is worked today, beyond an ancient place's
    /// fields or about a ruin; they go from hedgerow to scrub to stones or banks further
    /// out; and the chunks either side of a border agree on them.
    #[test]
    fn old_field_edges_lie_beyond_ancient_places_and_about_ruins() {
        let mut found = (0, 0);
        let mut styles = std::collections::HashMap::new();
        for seed in 1..6 {
            let w = GameWorld::with_config(seed, SimConfig { size: 256 });
            let c = w.continent(0, 0);
            let mut chunks = Vec::new();
            for (k, o) in c.land.old.iter().enumerate() {
                // A few squares out among each one's old fields.
                for a in 0..4 {
                    let dir = Vec2::from_angle(a as f32 * 1.9 + k as f32);
                    let d = o.begin(o.at + dir) + o.span * (0.15 + 0.2 * a as f32);
                    let p = o.at + dir * d;
                    let n = 48;
                    let (x0, z0) = (p.x as i32 - n / 2, p.y as i32 - n / 2);
                    let sampler = Sampler::new(&c, w.seed(), x0, z0, n);
                    let columns: Vec<Column> = (0..n * n).map(|i| sampler.column(x0 + i / n, z0 + i % n)).collect();
                    let patch = Patch::new(&c, w.seed(), x0, z0, n as usize, &columns);
                    for i in 1..n as usize - 1 {
                        for j in 1..n as usize - 1 {
                            let col = &columns[i * n as usize + j];
                            let Some(plot) = patch.old(i, j) else { continue };
                            let (x, z) = (x0 + i as i32, z0 + j as i32);
                            assert!(c.land.managed(x, z) < EDGE, "an old field at {x},{z} in the land worked today");
                            assert!(patch.at(i, j).is_none());
                            let src = &c.land.old[plot.source];
                            let from = src.at.distance(vec2(x as f32 + 0.5, z as f32 + 0.5));
                            assert!(from >= src.begin(vec2(x as f32 + 0.5, z as f32 + 0.5)) && from < src.begin(vec2(x as f32 + 0.5, z as f32 + 0.5)) + src.span * 1.31);
                            match src.owner {
                                Some(f) => assert_eq!(c.land.farms[f].age, Age::Ancient),
                                None => found.1 += 1,
                            }
                            let Some(e) = bounds::edge(&c.land, &patch, col, i, j) else { continue };
                            found.0 += 1;
                            assert!(e.decay >= 1.0 && !e.gate);
                            *styles.entry(e.style).or_insert(0) += 1;
                            chunks.push((x.div_euclid(CHUNK), z.div_euclid(CHUNK)));
                        }
                    }
                }
            }
            chunks.sort();
            chunks.dedup();
            for &(cx, cz) in chunks.iter().take(3) {
                let a = w.generate_column(cx, cz);
                let b = w.generate_column(cx + 1, cz);
                for (cy, ca) in &a.chunks {
                    let Some((_, cb)) = b.chunks.iter().find(|(y, _)| y == cy) else { continue };
                    for z in 0..PAD {
                        for y in 0..PAD {
                            assert_eq!(ca.get(PAD - 2, y, z), cb.get(0, y, z), "seed {seed}: chunks {cx},{cz} and east differ at y {y} z {z} of {cy}");
                            assert_eq!(ca.get(PAD - 1, y, z), cb.get(1, y, z), "seed {seed}: chunks {cx},{cz} and east differ at y {y} z {z} of {cy}");
                        }
                    }
                }
            }
        }
        println!("{} columns of old field edge, {} columns of old field about ruins; {styles:?}", found.0, found.1);
        assert!(found.0 > 200, "only {} columns of old field edge", found.0);
        assert!(found.1 > 0, "no old fields about a ruin");
        assert!(styles.len() >= 3, "only {styles:?}");
    }

    /// Lists the old land of a full-size world (seed from `SEED`, 1 if unset), with a
    /// column of old field edge out among each, for aiming screenshots:
    /// `cargo test -p worldgen list_old -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn list_old() {
        let seed = std::env::var("SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
        let mut c = crate::continent::generate(crate::macro_layout::cell_desc(seed, 0, 0), SimConfig::default());
        c.trails = crate::trails::build(&c, seed);
        c.land = super::super::influence::build(&c, seed);
        for (k, o) in c.land.old.iter().enumerate() {
            let mut hits = Vec::new();
            for a in 0..12 {
                let dir = Vec2::from_angle(a as f32 * 0.52);
                for t in [0.1, 0.3, 0.5, 0.7] {
                    let p = o.at + dir * (o.begin(o.at + dir) + o.span * t);
                    let n = 24;
                    let (x0, z0) = (p.x as i32 - n / 2, p.y as i32 - n / 2);
                    let sampler = Sampler::new(&c, seed, x0, z0, n);
                    let columns: Vec<Column> = (0..n * n).map(|i| sampler.column(x0 + i / n, z0 + i % n)).collect();
                    let patch = Patch::new(&c, seed, x0, z0, n as usize, &columns);
                    let edge = (1..n as usize - 1)
                        .flat_map(|i| (1..n as usize - 1).map(move |j| (i, j)))
                        .find_map(|(i, j)| bounds::edge(&c.land, &patch, &columns[i * n as usize + j], i, j).filter(|e| e.core).map(|e| (x0 + i as i32, z0 + j as i32, e.style)));
                    if let Some(e) = edge {
                        hits.push((t, e));
                    }
                }
            }
            println!("old land {k} at {} ({:?}), span {:.0}: {hits:?}", o.at, o.owner, o.span);
        }
    }

    /// The same old fields wherever they are asked about.
    #[test]
    fn old_fields_are_the_same_every_time() {
        let w = GameWorld::with_config(3, SimConfig { size: 256 });
        let c = w.continent(0, 0);
        let again = GameWorld::with_config(3, SimConfig { size: 256 });
        let d = again.continent(0, 0);
        assert_eq!(c.land.old.len(), d.land.old.len());
        let mut some = 0;
        for o in &c.land.old {
            for k in 0..200 {
                let p = o.at + Vec2::from_angle(k as f32 * 0.37) * (o.begin(o.at + Vec2::X) + k as f32 * 2.0);
                let (x, z) = (p.x as i32, p.y as i32);
                let col = Sampler::new(&c, w.seed(), x, z, 1).column(x, z);
                let a = at(&c, w.seed(), x, z, &col);
                assert_eq!(a, at(&d, w.seed(), x, z, &col));
                some += a.is_some() as u32;
            }
        }
        assert!(some > 50, "only {some} columns in old fields");
    }
}
