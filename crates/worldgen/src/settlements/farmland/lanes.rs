//! Lanes between hedges (#64): in the green lowlands and the uplands, a lane running out
//! through a settlement's fields is hedged or walled on both sides, of what the fields
//! are bounded with there (see [`super::bounds`]), with a strip of grass verge between
//! the worn way and the hedge, and gaps with a post either side where a field opens onto
//! it.
//!
//! Only lanes: a footpath across the fields is left open, as is any way through the
//! rough. A hedge keeps off the ground of a building and away from whatever was set
//! beside the way (a lantern, a milestone, a seat, a signpost at a junction), and where a
//! boundary between two fields comes down to the lane it meets the hedge instead of
//! ending in a gatepost as it does by an open trail.
//!
//! How a column lies by a lane is worked out from the trails and the column alone, so two
//! chunks agree on every hedge along their border.

use glam::{IVec2, Vec2, vec2};

use super::influence::{Farm, PASTURE};
use crate::{
    continent::Continent,
    macro_layout::MACRO_SIZE,
    noise::perlin2,
    scale::{m, m64},
    settlements::Region,
    trails::{Pad, TrailPoint},
};

/// How a column lies by a hedged lane.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lane {
    /// The grass between the lane's worn edge and its hedge, which nothing is sown on.
    Verge,
    /// The line the hedge or wall stands on: open where `gap`, and a post where `post`.
    Hedge { gap: bool, post: bool },
    /// The back of the hedge, toward the field: open where the hedge is.
    Back { gap: bool },
    /// The strip of field beyond the hedge, where a boundary between two fields meets it.
    Beyond,
}

/// The narrowest way that is hedged: half its worn width. A lane, not a footpath.
pub const LANE: f32 = m(0.55);
/// How far from a lane's worn edge its hedge stands: the verge.
pub const VERGE: f32 = m(1.4);
/// How far beyond the hedge a boundary between fields meets it, and the most a column
/// lies from a lane's edge to be by it at all.
const MEETS: f32 = m(2.4);
const NEAR: f32 = VERGE + 2.0 + MEETS;
/// The most a column by a lane has in its `Column::clear`, which counts from 1.2 m about
/// a way's middle rather than from its worn edge (at most 1.6 m out, on the busiest
/// roads): the columns further out need not be looked at.
pub(super) const LOOK: f32 = NEAR + m(1.6) - m(1.2);
/// How far a hedge keeps from the edge of a building's levelled ground.
const KEEP: f32 = m(6.0);
/// And from anything set beside the way.
const BESIDE: f32 = m(2.4);
/// Where a signpost stands from the point two trails meet (as `trails::build` sets it).
const SIGNPOST: Vec2 = Vec2::splat(m(2.4));

/// Whether the lanes through the fields of `farm` are hedged: in the green lowlands and
/// the uplands, the inner fields, those worked at least as hard as pasture.
pub fn hedged(farm: &Farm, managed: f32) -> bool {
    matches!(farm.region, Region::Temperate | Region::Upland) && managed >= PASTURE
}

/// The ways and what stands by them about a square of columns, for finding the lanes in
/// it.
pub(super) struct Lanes {
    ways: Vec<TrailPoint>,
    pads: Vec<Pad>,
    /// Things set beside the ways, which a hedge goes round.
    beside: Vec<Vec2>,
    /// World voxels of continent-local column 0.
    offset: IVec2,
}

impl Lanes {
    /// The lanes about the `n`-wide square of continent-local columns from `(x0, z0)` of
    /// `c`, if any lane runs near enough to it.
    pub(super) fn near(c: &Continent, x0: i32, z0: i32, n: usize) -> Option<Lanes> {
        let (lo, hi) = (vec2(x0 as f32, z0 as f32), vec2((x0 + n as i32) as f32, (z0 + n as i32) as f32));
        let (points, pads) = c.trails.gather(lo - Vec2::splat(NEAR), hi + Vec2::splat(NEAR));
        let ways: Vec<TrailPoint> = points.into_iter().filter(|t| t.half_width > 0.0).collect();
        if !ways.iter().any(|t| t.half_width >= LANE) {
            return None;
        }
        let (a, b) = (lo - Vec2::splat(BESIDE + 2.0), hi + Vec2::splat(BESIDE + 2.0));
        let inside = |p: Vec2| p.cmpge(a).all() && p.cmple(b).all();
        let net = &c.trails;
        let column = |q: IVec2| q.as_vec2() + 0.5;
        let mut beside: Vec<Vec2> = Vec::new();
        beside.extend(net.lanterns.iter().flat_map(|l| l.footprint.iter().map(|&q| column(q))).filter(|&p| inside(p)));
        beside.extend(net.markers.iter().map(|(_, at)| column(IVec2::new(at.x, at.z))).filter(|&p| inside(p)));
        beside.extend(net.delights.iter().flat_map(|d| d.footprint.iter().map(|&q| column(q))).filter(|&p| inside(p)));
        beside.extend(net.junctions.iter().map(|&j| j + SIGNPOST).filter(|&p| inside(p)));
        let offset = IVec2::new(c.desc.mx * MACRO_SIZE, c.desc.mz * MACRO_SIZE);
        Some(Lanes { ways, pads, beside, offset })
    }

    /// How the column at continent-local `(lx, lz)` lies by a hedged lane, if it does.
    /// `seed` is the world's.
    pub(super) fn at(&self, seed: u32, lx: i32, lz: i32) -> Option<Lane> {
        let p = vec2(lx as f32 + 0.5, lz as f32 + 0.5);
        // The nearest way, from its worn edge.
        let (mut edge, mut half) = (f32::MAX, 0.0);
        for t in &self.ways {
            let d = vec2(t.x, t.z).distance(p) - t.half_width;
            if d < edge {
                (edge, half) = (d, t.half_width);
            }
        }
        if edge >= NEAR || half < LANE {
            return None;
        }
        if self.pads.iter().any(|pad| p.distance(vec2(pad.x, pad.z)) - pad.radius < edge + KEEP) {
            return None;
        }
        if self.beside.iter().any(|b| b.distance(p) < BESIDE) {
            return None;
        }
        if edge < VERGE {
            return Some(Lane::Verge);
        }
        // A gap, a gate's width, here and there along each side, and a post either side.
        let (wx, wz) = ((self.offset.x + lx) as f64, (self.offset.y + lz) as f64);
        let gate = perlin2(seed ^ 0x1a4e, wx / m64(9.0), wz / m64(9.0));
        let (gap, post) = (gate > 0.4, gate > 0.32);
        Some(if edge < VERGE + 1.0 {
            Lane::Hedge { gap, post: post && !gap }
        } else if edge < VERGE + 2.0 {
            Lane::Back { gap: post }
        } else {
            Lane::Beyond
        })
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
        surface::{Cover, Sampler},
        world::GameWorld,
    };

    /// Chunk columns a hedged lane runs through on the test continent.
    fn lane_chunks(w: &GameWorld) -> Vec<(i32, i32)> {
        let c = w.continent(0, 0);
        let mut out: Vec<(i32, i32)> = c
            .trails
            .points()
            .filter(|t| t.half_width >= LANE)
            .filter(|t| {
                let (x, z) = (t.x as i32 + m(3.0) as i32, t.z as i32);
                super::super::plot(&c, w.seed(), x, z).is_some_and(|p| hedged(&c.land.farms[p.owner], p.managed))
            })
            .map(|t| ((t.x as i32).div_euclid(CHUNK), (t.z as i32).div_euclid(CHUNK)))
            .collect();
        out.sort();
        out.dedup();
        // Spread over the continent, not all along one lane.
        out.into_iter().step_by(7).take(16).collect()
    }

    /// A lane through the fields has its hedge on both sides, a verge before it that
    /// nothing is sown on, gaps with posts by them, and nothing of it on the way itself;
    /// and the chunks either side of a border agree on it.
    #[test]
    fn lanes_are_hedged_with_a_verge_and_gaps_and_agree_across_chunks() {
        let w = GameWorld::with_config(3, SimConfig { size: 256 });
        let c = w.continent(0, 0);
        let chunks = lane_chunks(&w);
        let (mut hedge, mut verge, mut gaps, mut posts, mut built) = (0, 0, 0, 0, 0);
        for &(cx, cz) in &chunks {
            let (x0, z0) = (cx * CHUNK - 2, cz * CHUNK - 2);
            let n = PAD + 2;
            let sampler = Sampler::new(&c, w.seed(), x0, z0, n as i32);
            let columns: Vec<_> = (0..n * n).map(|i| sampler.column(x0 + (i / n) as i32, z0 + (i % n) as i32)).collect();
            let patch = Patch::new(&c, w.seed(), x0, z0, n, &columns);
            let out = w.generate_column(cx, cz);
            for i in 2..n - 2 {
                for j in 2..n - 2 {
                    let (x, z) = (x0 + i as i32, z0 + j as i32);
                    let col = &columns[i * n + j];
                    let Some(lane) = patch.lane(i, j) else { continue };
                    assert_ne!(col.cover, Cover::Trail, "a lane's hedge or verge on the way at {x},{z}");
                    match lane {
                        Lane::Verge => {
                            verge += 1;
                            assert!(patch.at(i, j).is_none(), "a field on the verge at {x},{z}");
                            let crops = out.plants.iter().any(|p| p.lo[0].div_euclid(4) == x && p.lo[2].div_euclid(4) == z && (55..=61).contains(&p.season));
                            assert!(!crops, "crops on the verge at {x},{z}");
                        }
                        Lane::Hedge { gap, post } => {
                            hedge += 1;
                            gaps += gap as u32;
                            posts += post as u32;
                            let edge = bounds::edge(&c.land, &patch, col, i, j);
                            assert_eq!(edge.is_none(), gap, "at {x},{z}");
                            // Something of the hedge or the wall stands on the column.
                            let top = col.top() + 1;
                            let (cy, ly) = (top.div_euclid(CHUNK), top.rem_euclid(CHUNK));
                            let above = out.chunks.iter().find(|(k, _)| *k == cy).map(|(_, ch)| ch.get(i - 1, ly as usize + 1, j - 1));
                            built += above.is_some_and(|v| v != voxel::AIR) as u32;
                        }
                        Lane::Back { .. } | Lane::Beyond => {}
                    }
                }
            }
            // The chunks either side of the east border agree.
            let east = w.generate_column(cx + 1, cz);
            for (cy, ca) in &out.chunks {
                let Some((_, cb)) = east.chunks.iter().find(|(y, _)| y == cy) else { continue };
                for z in 0..PAD {
                    for y in 0..PAD {
                        assert_eq!(ca.get(PAD - 2, y, z), cb.get(0, y, z), "chunks {cx},{cz} and east differ at y {y} z {z} of {cy}");
                        assert_eq!(ca.get(PAD - 1, y, z), cb.get(1, y, z), "chunks {cx},{cz} and east differ at y {y} z {z} of {cy}");
                    }
                }
            }
        }
        println!("{} chunks by lanes: {hedge} columns of hedge line ({gaps} open, {posts} posts, {built} built on), {verge} of verge", chunks.len());
        assert!(hedge > 300 && verge > 300, "only {hedge} columns of hedge and {verge} of verge in {} chunks", chunks.len());
        assert!(gaps > 0 && posts > 0, "no gaps ({gaps}) or no posts ({posts})");
        assert!(built * 2 > hedge - gaps, "only {built} of {} columns of hedge line have anything on them", hedge - gaps);
    }
}
