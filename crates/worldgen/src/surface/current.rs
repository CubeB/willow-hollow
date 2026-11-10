//! Which way, and how fast, the water in a river runs: for the shading of its surface,
//! the spray off its falls and whatever floats on it.

use glam::{Vec2, vec2};
use voxel::mesh::MAX_CURRENT;

use super::{Column, Piece, SEGS, Sampler, pool_level, smoothstep};
use crate::scale::{OLD_VOXEL, m};

/// Fall of the water surface, in voxels per voxel, at which a stream runs flat out: a
/// mountain brook dropping a metre in every twelve.
const STEEP: f32 = 0.08;
/// How far up and down the curve the fall is measured over, in voxels, so the speed
/// changes smoothly rather than stepping where one stretch of level meets the next.
const WINDOW: f32 = m(8.0);

impl Piece {
    /// Downstream direction of the curve at parameter `t` (unit length). Pieces run from
    /// upstream to downstream, and neighbouring pieces share tangents where they meet.
    fn tangent(&self, t: f32) -> Vec2 {
        const { assert!(SEGS % 2 == 0, "the curve's midpoint must be one of its points") };
        // The curve's middle point is a quarter of each end plus half the control point.
        let (p0, p2) = (self.pts[0], self.pts[SEGS]);
        let p1 = self.pts[SEGS / 2] * 2.0 - (p0 + p2) * 0.5;
        ((p1 - p0) * (2.0 * (1.0 - t)) + (p2 - p1) * (2.0 * t)).normalize_or_zero()
    }

    /// How steeply the simulated water surface falls along the curve near `t`, in voxels
    /// per voxel. Pools are flat, but the river still runs through them as fast as the
    /// valley falls.
    fn fall(&self, t: f32) -> f32 {
        let span = WINDOW / self.length;
        let (a, b) = ((t - span).max(0.0), (t + span).min(1.0));
        ((self.level(a) - self.level(b)) * OLD_VOXEL / ((b - a) * self.length).max(1.0)).max(0.0)
    }
}

/// Mid-stream speed of a river, metres per second, from how steeply it falls and how
/// wide it is. A lowland river slides along at a slow walk; a steep brook races.
fn speed(fall: f32, half_width: f32) -> f32 {
    let steep = (fall / STEEP).clamp(0.0, 1.0).sqrt();
    (0.3 + 1.5 * steep + 0.3 * smoothstep(m(2.0), m(12.0), half_width)).min(MAX_CURRENT)
}

impl Sampler<'_> {
    /// The current in the water of column `(lx, lz)` (continent-local voxels), which
    /// [`column`](Self::column) sampled as `col`: velocity in metres per second along x
    /// and z. `None` where the water is still: off the rivers, and where a lake stands
    /// over a river's channel. Water the sea holds back in a tidal reach still runs, slowly.
    pub fn current(&self, lx: i32, lz: i32, col: &Column) -> Option<Vec2> {
        let water = col.water_surface()?;
        let p = vec2(lx as f32 + 0.5, lz as f32 + 0.5);
        // The piece whose channel edge is nearest, chosen as `column` chooses it.
        let mut nearest: Option<(f32, &Piece, f32)> = None;
        for piece in &self.pieces {
            if p.x < piece.lo.x || p.x > piece.hi.x || p.y < piece.lo.y || p.y > piece.hi.y {
                continue;
            }
            let (dist, t) = piece.nearest(p);
            let sd = dist - piece.half_width(t);
            if nearest.is_none_or(|n| sd < n.0) {
                nearest = Some((sd, piece, t));
            }
        }
        let (sd, piece, t) = nearest?;
        // Only the channel holds river water: the banks meet it flush with its surface.
        if sd >= 0.0 || water != pool_level(self.c.desc.seed, piece.level(t)) {
            return None;
        }
        let hw = piece.half_width(t);
        let across = 1.0 + sd / hw; // 0 mid-stream, 1 at the edge
        // Slowest against the banks.
        let mut v = speed(piece.fall(t), hw) * (1.0 - 0.55 * across * across);
        if water == 0 {
            // A tidal reach: the sea holds it back, and out past the mouth it spreads
            // and fades into the sea.
            v *= 0.4;
            if piece.nodes[1] == piece.nodes[2] {
                v *= 1.0 - smoothstep(0.2, 0.95, t);
                if v <= 0.0 {
                    return None;
                }
            }
        }
        Some(piece.tangent(t) * v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn piece(p0: Vec2, p1: Vec2, p2: Vec2) -> Piece {
        Piece::new(p0, p1, p2, (m(4.0), m(4.0)), (40.0, 39.0, 38.0), 40.0, [0, 1, 2])
    }

    #[test]
    fn tangent_points_downstream_along_the_curve() {
        let p = piece(vec2(0.0, 0.0), vec2(100.0, 0.0), vec2(100.0, 100.0));
        assert!(p.tangent(0.0).abs_diff_eq(vec2(1.0, 0.0), 1e-4));
        assert!(p.tangent(1.0).abs_diff_eq(vec2(0.0, 1.0), 1e-4));
        let mid = p.tangent(0.5);
        assert!(mid.abs_diff_eq(vec2(1.0, 1.0).normalize(), 1e-4), "{mid}");
    }

    #[test]
    fn steeper_and_wider_rivers_run_faster() {
        let flat = piece(vec2(0.0, 0.0), vec2(50.0, 0.0), vec2(100.0, 0.0));
        let steep = Piece::new(vec2(0.0, 0.0), vec2(50.0, 0.0), vec2(100.0, 0.0), (m(4.0), m(4.0)), (44.0, 42.0, 40.0), 44.0, [0, 1, 2]);
        assert!(steep.fall(0.5) > flat.fall(0.5) && flat.fall(0.5) > 0.0);
        assert!(speed(steep.fall(0.5), m(1.0)) > speed(flat.fall(0.5), m(1.0)));
        assert!(speed(0.01, m(10.0)) > speed(0.01, m(1.0)));
        assert!(speed(1.0, m(20.0)) <= MAX_CURRENT);
        assert!(speed(0.0, m(1.0)) > 0.0, "a river always runs");
    }
}
