//! Assembles a continent's network: settles the sites on real ground, joins them up,
//! lays each trail over the terrain and builds what it needs to cross water.

use std::collections::HashMap;

use glam::{IVec3, Vec2, Vec3, vec2};
use voxel::Voxel;

use super::{
    Crossing, CrossingKind, Mooring, Network, Pad, Site, TrailPoint,
    route::{self, Router},
    sites,
};
use crate::{
    continent::Continent,
    features::{
        landmarks::{self, Landmark, Palette},
        shape::{Rng, Shape, rotate},
    },
    noise::{hash2, unit},
    surface::{Column, Cover, Sampler},
};

/// Paved half-width of a trail.
const HALF_WIDTH: f32 = 1.5;
/// Steepest a trail may rise, in voxels per voxel.
const MAX_SLOPE: f32 = 0.45;
/// Longest water a trail will bridge.
const MAX_SPAN: usize = 90;
/// Longest link in the site graph.
const MAX_LINK: f32 = 6000.0;
/// Lanterns stand this far apart near landmarks, and much further apart elsewhere.
const LANTERN_NEAR: usize = 64;
const LANTERN_FAR: usize = 170;
/// How far from a landmark counts as near it.
const NEAR_SITE: f32 = 300.0;

/// Columns under a list of points. Reads the bare terrain: the network being built is
/// not attached to the continent yet.
fn sample(c: &Continent, seed: u32, pts: &[Vec2]) -> Vec<Column> {
    let mut out = Vec::with_capacity(pts.len());
    for run in pts.chunks(48) {
        let (mut lo, mut hi) = (Vec2::MAX, Vec2::MIN);
        for p in run {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        let (x0, z0) = (lo.x.floor() as i32, lo.y.floor() as i32);
        let span = ((hi.x.ceil() as i32 - x0).max(hi.y.ceil() as i32 - z0)).max(1) + 2;
        let sampler = Sampler::new(c, seed, x0, z0, span);
        out.extend(run.iter().map(|p| sampler.column(p.x.floor() as i32, p.y.floor() as i32)));
    }
    out
}

fn is_wet(col: &Column) -> bool {
    col.water_surface().is_some()
}

/// Index ranges of water along a trail, with brief dry interruptions merged.
fn wet_runs(cols: &[Column]) -> Vec<(usize, usize)> {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for (i, col) in cols.iter().enumerate() {
        if !is_wet(col) {
            continue;
        }
        match runs.last_mut() {
            Some(last) if i - last.1 <= 4 => last.1 = i,
            _ => runs.push((i, i)),
        }
    }
    runs
}

/// Rewrites the approach to each crossing as a straight line, so a bridge goes directly
/// over the water instead of curving with the path.
fn straighten(pts: &[Vec2], runs: &[(usize, usize)]) -> Vec<Vec2> {
    let mut out = Vec::with_capacity(pts.len());
    let mut at = 0;
    for &(s, e) in runs {
        let a = s.saturating_sub(5);
        let b = (e + 5).min(pts.len() - 1);
        if a < at || b <= a {
            continue;
        }
        out.extend_from_slice(&pts[at..a]);
        let (pa, pb) = (pts[a], pts[b]);
        let len = (pb - pa).length().ceil().max(1.0) as usize;
        out.extend((0..len).map(|k| pa + (pb - pa) * (k as f32 / len as f32)));
        at = b;
    }
    out.extend_from_slice(&pts[at..]);
    out
}

/// A world voxel on the deck grid, offset sideways from the centreline.
fn deck_voxel(p: Vec2, side: Vec2, offset: f32, y: i32) -> IVec3 {
    let q = p + side * offset;
    IVec3::new(q.x.floor() as i32, y, q.y.floor() as i32)
}

/// Stones across a narrow stream, proud of the water, with a flush ford between them so
/// walking across never leaves you stuck.
fn stepping_stones(pal: &Palette, rng: &mut Rng, pts: &[Vec2], cols: &[Column], run: (usize, usize)) -> Vec<(IVec3, Voxel)> {
    let mut s = Shape::new();
    let dir = (pts[run.1] - pts[run.0]).normalize_or_zero();
    let side = vec2(-dir.y, dir.x);
    for i in run.0..=run.1 {
        let Some(k) = cols[i].water_surface() else { continue };
        let proud = (i - run.0) % 3 != 2;
        for offset in [-0.5f32, 0.5] {
            for y in cols[i].top() + 1..=k + proud as i32 - 1 {
                s.set(deck_voxel(pts[i], side, offset, y), pal.block(rng));
            }
        }
    }
    s.into_voxels()
}

/// Bridges a run of water. Returns the voxels and the sample range the structure covers,
/// ramps included: the trail is carried by the bridge across that range.
#[allow(clippy::too_many_arguments)]
fn bridge(
    kind: CrossingKind,
    pal: &Palette,
    rng: &mut Rng,
    pts: &[Vec2],
    cols: &[Column],
    run: (usize, usize),
) -> (Vec<(IVec3, Voxel)>, (usize, usize)) {
    let (s0, e0) = run;
    let kmax = (s0..=e0).filter_map(|i| cols[i].water_surface()).max().unwrap_or(0);
    let deck = kmax + if kind == CrossingKind::Arch { 2 } else { 1 };
    let half = if kind == CrossingKind::Arch { 2i32 } else { 1 };
    let span = (e0 - s0 + 1) as f32;
    let dir = (pts[e0.min(pts.len() - 1)] - pts[s0]).normalize_or_zero();
    let side = vec2(-dir.y, dir.x);

    // Ramps run down each side at one voxel in two until they meet the ground.
    let ramp = |from: usize, step: isize| -> (usize, Vec<i32>) {
        let (mut i, mut ys) = (from, Vec::new());
        for k in 1..=16 {
            let next = from as isize + step * k;
            if next < 0 || next as usize >= pts.len() {
                break;
            }
            let y = deck - (k as i32 + 1) / 2;
            i = next as usize;
            let ground = cols[i].top();
            if is_wet(&cols[i]) || y > ground {
                ys.push(y.max(ground));
            } else {
                break;
            }
        }
        (i, ys)
    };
    let (foot_a, up) = ramp(s0, -1);
    let (foot_b, down) = ramp(e0, 1);

    // Height of the deck at each sample, with a gentle hump on the bigger bridges.
    let rise = if kind == CrossingKind::Arch { (span / 10.0).clamp(1.0, 4.0) } else { 0.0 };
    let mut samples: Vec<(usize, i32)> = Vec::new();
    for (k, &y) in up.iter().enumerate().rev() {
        samples.push((s0 - (k + 1), y));
    }
    for i in s0..=e0 {
        let t = (i - s0) as f32 / span.max(1.0);
        samples.push((i, deck + (rise * (t * std::f32::consts::PI).sin()).round() as i32));
    }
    for (k, &y) in down.iter().enumerate() {
        samples.push((e0 + k + 1, y));
    }

    let mut s = Shape::new();
    let surface = |rng: &mut Rng| match kind {
        CrossingKind::Arch => pal.block(rng),
        CrossingKind::Log => pal.wood,
        _ => pal.plank,
    };
    let mut last_rail = 0;
    for (n, &(i, y)) in samples.iter().enumerate() {
        let col = &cols[i];
        let wet = is_wet(col);
        let over_water = (s0..=e0).contains(&i);
        // Half-steps along and across so a diagonal deck has no holes.
        for along in [0.0f32, 0.5] {
            let p = pts[i] + dir * along;
            let mut offset = -(half as f32);
            while offset <= half as f32 {
                let q = deck_voxel(p, side, offset, y);
                s.set(q, surface(rng));
                // Solid underneath wherever the deck rests on ground.
                if !wet {
                    for fill in col.top() + 1..y {
                        s.set(IVec3::new(q.x, fill, q.z), pal.stone_dark);
                    }
                }
                offset += 0.5;
            }
        }
        let edge = half as f32 + 1.0;
        match kind {
            CrossingKind::Arch => {
                for sign in [-1.0, 1.0] {
                    s.set(deck_voxel(pts[i], side, sign * edge, y), pal.block(rng));
                    s.set(deck_voxel(pts[i], side, sign * edge, y + 1), pal.block(rng));
                }
            }
            _ => {
                if n >= last_rail + 3 {
                    last_rail = n;
                    for sign in [-1.0, 1.0] {
                        for h in 0..=1 {
                            s.set(deck_voxel(pts[i], side, sign * edge, y + h), pal.wood);
                        }
                    }
                }
            }
        }
        // Piers go down to the bed.
        if over_water && (i - s0) % 6 == 3 {
            let width = half as f32 + if kind == CrossingKind::Arch { 1.0 } else { 0.0 };
            let mut offset = -width;
            while offset <= width {
                for yy in col.top() - 1..y {
                    s.set(deck_voxel(pts[i], side, offset, yy), if kind == CrossingKind::Arch { pal.block(rng) } else { pal.wood });
                }
                offset += 0.5;
            }
        }
    }
    (s.into_voxels(), (foot_a, foot_b))
}

/// Finds where a landmark can actually stand near a map cell.
fn settle(c: &Continent, seed: u32, kind: Landmark, cell: usize) -> Option<(i32, i32, f32)> {
    let cs = c.cell_size();
    let n = c.size;
    let (cx, cz) = (((cell % n) as f32 + 0.5) * cs, ((cell / n) as f32 + 0.5) * cs);
    let radius = kind.radius();
    let probe = |x: i32, z: i32| -> Option<f32> {
        let r = radius as i32 + 1;
        let sampler = Sampler::new(c, seed, x - r, z - r, 2 * r + 1);
        let (mut lo, mut hi, mut sum) = (f32::MAX, f32::MIN, 0.0);
        let rim = [(0, 0), (r, 0), (-r, 0), (0, r), (0, -r), (r * 2 / 3, r * 2 / 3), (-r * 2 / 3, r * 2 / 3), (r * 2 / 3, -r * 2 / 3), (-r * 2 / 3, -r * 2 / 3)];
        for (dx, dz) in rim {
            let col = sampler.column(x + dx, z + dz);
            if is_wet(&col) || col.river < 6.0 || !matches!(col.cover, Cover::Land | Cover::Beach) {
                return None;
            }
            lo = lo.min(col.ground);
            hi = hi.max(col.ground);
            sum += col.ground;
        }
        (hi - lo <= 7.0).then_some(sum / rim.len() as f32)
    };
    for ring in 0..12 {
        let reach = ring as f32 * cs * 0.25;
        let steps = if ring == 0 { 1 } else { 8 + ring * 2 };
        for k in 0..steps {
            let a = k as f32 / steps as f32 * std::f32::consts::TAU;
            let (x, z) = ((cx + a.cos() * reach) as i32, (cz + a.sin() * reach) as i32);
            if let Some(y) = probe(x, z) {
                return Some((x, z, y));
            }
        }
    }
    None
}

/// A landing needs a shoreline and open water straight out from it.
struct Shore {
    x: i32,
    z: i32,
    level: i32,
    turns: u32,
    length: i32,
}

fn find_shore(c: &Continent, seed: u32, cell: usize) -> Option<Shore> {
    let cs = c.cell_size();
    let n = c.size;
    let (cx, cz) = (((cell % n) as f32 + 0.5) * cs, ((cell / n) as f32 + 0.5) * cs);
    let reach = (cs * 4.0) as i32;
    let mut best: Option<Shore> = None;
    for offset in [0, 12, -12, 24, -24] {
        for (turns, (dx, dz)) in [(1i32, 0i32), (0, 1), (-1, 0), (0, -1)].into_iter().enumerate() {
            let (sx, sz) = (dz * offset + cx as i32, dx * offset + cz as i32);
            let (lo_x, lo_z) = (sx.min(sx + dx * reach) - 1, sz.min(sz + dz * reach) - 1);
            let sampler = Sampler::new(c, seed, lo_x, lo_z, reach + 3);
            // March out to the water's edge, then see how far the water goes.
            let mut last_dry = None;
            let mut run = 0;
            let mut level = 0;
            for d in 0..reach {
                let col = sampler.column(sx + dx * d, sz + dz * d);
                match col.water_surface() {
                    Some(k) if last_dry.is_some() => {
                        if col.river < 0.0 {
                            break;
                        }
                        level = k;
                        run += 1;
                        if run >= 22 {
                            break;
                        }
                    }
                    Some(_) => {}
                    None if run > 0 => break,
                    None => last_dry = Some(d),
                }
            }
            if let Some(d) = last_dry
                && run >= 14
                && best.as_ref().is_none_or(|b| run > b.length + 2)
            {
                best = Some(Shore {
                    x: sx + dx * d,
                    z: sz + dz * d,
                    level,
                    turns: turns as u32,
                    length: run - 2,
                });
            }
        }
    }
    best
}

fn absolute(voxels: Vec<(IVec3, Voxel)>, origin: IVec3, turns: u32) -> Vec<(IVec3, Voxel)> {
    voxels.into_iter().map(|(p, v)| (origin + rotate(p, turns), v)).collect()
}

/// Settled sites with their map cell.
fn place_sites(c: &Continent, seed: u32, comp: &[u32], net: &mut Network) -> Vec<usize> {
    let mut lists = sites::candidates(c, comp);
    let mut placed: Vec<(f32, f32)> = Vec::new();
    let mut cells = Vec::new();
    let mut remaining: Vec<usize> = lists.iter().map(|l| l.1).collect();
    let mut cursor = vec![0usize; lists.len()];
    let cs = c.cell_size();
    // Take turns, so scarce kinds are not crowded out by whichever scores highest.
    let mut progress = true;
    while progress {
        progress = false;
        for (k, list) in lists.iter_mut().enumerate() {
            if remaining[k] == 0 {
                continue;
            }
            while cursor[k] < list.2.len() {
                let cell = list.2[cursor[k]];
                cursor[k] += 1;
                let (wx, wz) = (((cell % c.size) as f32 + 0.5) * cs, ((cell / c.size) as f32 + 0.5) * cs);
                if !sites::spaced(&placed, wx, wz) {
                    continue;
                }
                let site_seed = hash2(c.desc.seed ^ 0x517e, cell as i32, k as i32);
                let site = if list.0 == Landmark::Landing {
                    place_landing(c, seed, cell, site_seed, net)
                } else {
                    place_land(c, seed, list.0, cell, site_seed, net)
                };
                if let Some(site) = site {
                    placed.push((site.x as f32, site.z as f32));
                    net.sites.push(site);
                    cells.push(cell);
                    remaining[k] -= 1;
                    progress = true;
                    break;
                }
            }
        }
    }
    cells
}

fn place_land(c: &Continent, seed: u32, kind: Landmark, cell: usize, site_seed: u32, net: &mut Network) -> Option<Site> {
    let (x, z, ground) = settle(c, seed, kind, cell)?;
    let biome = Sampler::new(c, seed, x, z, 1).column(x, z).biome;
    let palette = Palette::of(biome);
    let y = ground.round();
    net.pads.push(Pad {
        x: x as f32 + 0.5,
        z: z as f32 + 0.5,
        radius: kind.radius(),
        y,
    });
    let origin = IVec3::new(x, y as i32 + 1, z);
    let turns = site_seed & 3;
    net.add_stamp(absolute(landmarks::build(kind, &palette, site_seed, 0), origin, turns));
    Some(Site {
        kind,
        x,
        z,
        y: origin.y,
        mooring: None,
    })
}

fn place_landing(c: &Continent, seed: u32, cell: usize, site_seed: u32, net: &mut Network) -> Option<Site> {
    let shore = find_shore(c, seed, cell)?;
    let biome = Sampler::new(c, seed, shore.x, shore.z, 1).column(shore.x, shore.z).biome;
    let palette = Palette::of(biome);
    let y = shore.level as f32 + 0.5;
    net.pads.push(Pad {
        x: shore.x as f32 + 0.5,
        z: shore.z as f32 + 0.5,
        radius: Landmark::Landing.radius(),
        y,
    });
    let origin = IVec3::new(shore.x, shore.level + 1, shore.z);
    let pier = landmarks::build(Landmark::Landing, &palette, site_seed, shore.length);
    net.add_stamp(absolute(pier, origin, shore.turns));
    let along = rotate(IVec3::new(shore.length - 3, 0, 4), shore.turns);
    let heading = shore.turns as f32 * std::f32::consts::FRAC_PI_2;
    Some(Site {
        kind: Landmark::Landing,
        x: shore.x,
        z: shore.z,
        y: origin.y,
        mooring: Some(Mooring {
            pos: Vec3::new(
                (shore.x + along.x) as f32 + 0.5,
                shore.level as f32,
                (shore.z + along.z) as f32 + 0.5,
            ),
            heading,
        }),
    })
}

/// Pairs of sites to connect: a relative neighbourhood graph within each landmass, which
/// is sparse but always connected as long as no link is too long.
fn links(sites_at: &[(f32, f32, u32)]) -> Vec<(usize, usize, f32)> {
    let dist = |a: usize, b: usize| (sites_at[a].0 - sites_at[b].0).hypot(sites_at[a].1 - sites_at[b].1);
    let n = sites_at.len();
    let mut out = Vec::new();
    for a in 0..n {
        for b in a + 1..n {
            if sites_at[a].2 != sites_at[b].2 {
                continue;
            }
            let d = dist(a, b);
            if d > MAX_LINK {
                continue;
            }
            let shadowed = (0..n).any(|k| k != a && k != b && dist(a, k).max(dist(b, k)) < d);
            if !shadowed {
                out.push((a, b, d));
            }
        }
    }
    out.sort_by(|p, q| p.2.total_cmp(&q.2));
    out
}

/// Heights along a trail: terrain smoothed by a moving average, then clamped so the
/// slope never exceeds what is comfortable to walk.
fn profile(cols: &[Column], pins: &[(usize, f32)]) -> Vec<f32> {
    const WINDOW: usize = 12;
    let n = cols.len();
    let mut smoothed = Vec::with_capacity(n);
    for i in 0..n {
        let (lo, hi) = (i.saturating_sub(WINDOW), (i + WINDOW + 1).min(n));
        let (mut sum, mut count) = (0.0, 0);
        for col in &cols[lo..hi] {
            if !is_wet(col) {
                sum += col.ground;
                count += 1;
            }
        }
        smoothed.push(if count > 0 { sum / count as f32 } else { cols[i].ground });
    }
    for i in 1..n {
        smoothed[i] = smoothed[i].clamp(smoothed[i - 1] - MAX_SLOPE, smoothed[i - 1] + MAX_SLOPE);
    }
    for i in (0..n.saturating_sub(1)).rev() {
        smoothed[i] = smoothed[i].clamp(smoothed[i + 1] - MAX_SLOPE, smoothed[i + 1] + MAX_SLOPE);
    }
    // Pinned points (landmarks, bridge feet) draw the profile to them within reach.
    for &(at, y) in pins {
        let reach = 64usize;
        for i in at.saturating_sub(reach)..(at + reach + 1).min(n) {
            let room = i.abs_diff(at) as f32 * MAX_SLOPE;
            smoothed[i] = smoothed[i].clamp(y - room, y + room);
        }
    }
    smoothed
}

/// Lays one stretch of trail over the land and builds its crossings.
struct Layer<'a> {
    c: &'a Continent,
    seed: u32,
    /// Landmark centres, for lantern spacing.
    site_points: Vec<Vec2>,
}

impl Layer<'_> {
    fn palette_at(&self, col: &Column) -> Palette {
        Palette::of(col.biome)
    }

    fn lay(&self, raw: Vec<Vec2>, start_y: Option<f32>, end_y: Option<f32>, salt: u32, net: &mut Network) {
        if raw.len() < 8 {
            return;
        }
        // Over water the trail must run straight, which needs the water found first.
        let cols = sample(self.c, self.seed, &raw);
        let pts = straighten(&raw, &wet_runs(&cols));
        let mut cols = sample(self.c, self.seed, &pts);
        let runs = wet_runs(&cols);

        // Water too wide to bridge ends the trail on each side of it.
        if let Some(&(s, e)) = runs.iter().find(|&&(s, e)| e - s + 1 > MAX_SPAN) {
            if s > 8 {
                self.lay(pts[..s - 6].to_vec(), start_y, None, salt.wrapping_add(11), net);
            }
            if e + 8 < pts.len() {
                self.lay(pts[e + 6..].to_vec(), None, end_y, salt.wrapping_add(23), net);
            }
            return;
        }

        let mut rng = Rng::new(hash2(self.c.desc.seed ^ 0xb71d, salt as i32, pts.len() as i32));
        // (start, end) of sample ranges carried by a bridge.
        let mut carried: Vec<(usize, usize)> = Vec::new();
        let mut pins: Vec<(usize, f32)> = Vec::new();
        for &(s, e) in &runs {
            let span = e - s + 1;
            let mid = (s + e) / 2;
            let palette = self.palette_at(&cols[mid]);
            let depth = (s..=e)
                .filter_map(|i| cols[i].water_surface().map(|k| k - cols[i].top()))
                .max()
                .unwrap_or(0);
            let kind = if span <= 5 && depth <= 3 {
                CrossingKind::Stones
            } else if span <= 16 {
                if rng.unit() < 0.5 { CrossingKind::Log } else { CrossingKind::Plank }
            } else {
                CrossingKind::Arch
            };
            net.crossings.push(Crossing {
                kind,
                x: pts[mid].x as i32,
                z: pts[mid].y as i32,
                span: span as i32,
            });
            if kind == CrossingKind::Stones {
                net.add_stamp(stepping_stones(&palette, &mut rng, &pts, &cols, (s, e)));
            } else {
                let (voxels, range) = bridge(kind, &palette, &mut rng, &pts, &cols, (s, e));
                net.add_stamp(voxels);
                carried.push(range);
                pins.push((range.0.saturating_sub(1), cols[range.0].ground));
                pins.push(((range.1 + 1).min(pts.len() - 1), cols[range.1].ground));
            }
        }
        if let Some(y) = start_y {
            pins.push((0, y));
        }
        if let Some(y) = end_y {
            pins.push((pts.len() - 1, y));
        }

        let heights = profile(&cols, &pins);
        let is_carried = |i: usize| carried.iter().any(|&(a, b)| (a..=b).contains(&i));
        let points: Vec<TrailPoint> = pts
            .iter()
            .enumerate()
            .map(|(i, p)| TrailPoint {
                x: p.x,
                z: p.y,
                y: heights[i],
                half_width: if is_carried(i) { 0.0 } else { HALF_WIDTH },
            })
            .collect();
        net.add_points(&points);
        net.lines.push(pts.iter().step_by(8).copied().collect());
        cols.truncate(pts.len());
        self.furnish(&pts, &heights, &cols, &carried, salt, net);
    }

    /// Lanterns along the way, closer together near landmarks and at bridge heads.
    fn furnish(&self, pts: &[Vec2], heights: &[f32], cols: &[Column], carried: &[(usize, usize)], salt: u32, net: &mut Network) {
        let mut since = 30usize;
        let mut side = if salt & 1 == 0 { 1.0 } else { -1.0 };
        for i in 8..pts.len().saturating_sub(8) {
            since += 1;
            let near_site = self.site_points.iter().any(|s| s.distance_squared(pts[i]) < NEAR_SITE * NEAR_SITE);
            let at_bridge = carried.iter().any(|&(a, b)| i + 3 == a || i == b + 3);
            let due = since >= if near_site { LANTERN_NEAR } else { LANTERN_FAR };
            if !(at_bridge || due) || carried.iter().any(|&(a, b)| (a..=b).contains(&i)) {
                continue;
            }
            let tangent = (pts[i + 4] - pts[i - 4]).normalize_or_zero();
            let normal = vec2(-tangent.y, tangent.x);
            let q = pts[i] + normal * side * (HALF_WIDTH + 1.5);
            let col = cols[i];
            if is_wet(&col) || col.river < 3.0 {
                continue;
            }
            let (x, z) = (q.x.floor() as i32, q.y.floor() as i32);
            let y = heights[i].floor() as i32 + 1;
            let height = 3 + (unit(hash2(salt, x, z)) * 2.0) as i32;
            net.add_stamp(absolute(landmarks::trail_lantern(&self.palette_at(&col), height), IVec3::new(x, y, z), 0));
            since = 0;
            side = -side;
        }
    }
}

/// Runs of cells not yet on a trail, each with what it connects to at either end.
fn fresh_runs(path: &[u32], used: &[u32]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < path.len() {
        if used[path[i] as usize] != 0 {
            i += 1;
            continue;
        }
        let start = i;
        while i < path.len() && used[path[i] as usize] == 0 {
            i += 1;
        }
        out.push((start, i - 1));
    }
    out
}

/// Builds the whole network for a continent. `world_seed` is needed to sample the same
/// surface the game will generate.
pub fn build(c: &Continent, world_seed: u32) -> Network {
    let mut net = Network::default();
    if c.ocean.iter().all(|&o| o) {
        return net;
    }
    let comp = sites::land_components(c);
    let cells = place_sites(c, world_seed, &comp, &mut net);
    let at: Vec<(f32, f32, u32)> = net
        .sites
        .iter()
        .zip(&cells)
        .map(|(s, &cell)| (s.x as f32 + 0.5, s.z as f32 + 0.5, comp[cell]))
        .collect();

    let layer = Layer {
        c,
        seed: world_seed,
        site_points: at.iter().map(|&(x, z, _)| vec2(x, z)).collect(),
    };
    let cs = c.cell_size();
    let cell_of = |x: f32, z: f32| (z / cs) as usize * c.size + (x / cs) as usize;
    let mut router = Router::new(c);
    let mut lines: HashMap<u32, Vec<Vec2>> = HashMap::new();
    let mut next_id = 1u32;

    for (a, b, _) in links(&at) {
        let (sa, sb) = (net.sites[a], net.sites[b]);
        let (ca, cb) = (cell_of(at[a].0, at[a].1), cell_of(at[b].0, at[b].1));
        let Some(path) = router.search(ca, cb) else { continue };

        for (first, last) in fresh_runs(&path, &router.used) {
            // Where this piece begins and ends: at a site, or on the trail it joins.
            let join = |index: usize, lines: &HashMap<u32, Vec<Vec2>>, used: &[u32]| -> Option<Vec2> {
                let neighbour = path.get(index)?;
                let owner = used[*neighbour as usize];
                let cell = *neighbour;
                let target = vec2(((cell as usize % c.size) as f32 + 0.5) * cs, ((cell as usize / c.size) as f32 + 0.5) * cs);
                lines
                    .get(&owner)?
                    .iter()
                    .min_by(|p, q| p.distance_squared(target).total_cmp(&q.distance_squared(target)))
                    .copied()
            };
            let from_site = first == 0;
            let to_site = last == path.len() - 1;
            let start = if from_site {
                Some(vec2(at[a].0, at[a].1))
            } else {
                join(first - 1, &lines, &router.used)
            };
            let end = if to_site {
                Some(vec2(at[b].0, at[b].1))
            } else {
                join(last + 1, &lines, &router.used)
            };
            let (Some(start), Some(end)) = (start, end) else { continue };
            if last - first < 2 && !(from_site || to_site) {
                continue;
            }
            let cells_here = &path[first..=last];
            let pts = route::smooth(c, cells_here, start, end, next_id);
            if !from_site {
                net.junctions.push(start);
            }
            if !to_site {
                net.junctions.push(end);
            }
            let start_y = from_site.then_some(sa.y as f32 - 1.0);
            let end_y = to_site.then_some(sb.y as f32 - 1.0);
            let id = next_id;
            next_id += 1;
            lines.insert(id, pts.clone());
            for &cell in cells_here {
                router.used[cell as usize] = id;
            }
            layer.lay(pts, start_y, end_y, id, &mut net);
        }
    }

    // Signposts where trails meet.
    let mut junction_cells = std::collections::HashSet::new();
    for j in net.junctions.clone() {
        if !junction_cells.insert(((j.x / 8.0) as i32, (j.y / 8.0) as i32)) {
            continue;
        }
        let near: Vec<f32> = lines
            .values()
            .filter_map(|line| {
                let k = line.iter().position(|p| p.distance_squared(j) < 400.0 && p.distance_squared(j) > 49.0)?;
                let d = line[k] - j;
                Some(d.y.atan2(d.x))
            })
            .collect();
        let x = (j.x + 3.0) as i32;
        let z = (j.y + 3.0) as i32;
        let col = Sampler::new(c, world_seed, x, z, 1).column(x, z);
        if is_wet(&col) || col.river < 3.0 {
            continue;
        }
        let (trail_y, _) = net
            .points()
            .fold((col.ground, f32::MAX), |(y, best), p| {
                let d = (p.x - j.x).hypot(p.z - j.y);
                if d < best { (p.y, d) } else { (y, best) }
            });
        let directions: &[f32] = if near.is_empty() { &[0.0, 2.1] } else { &near };
        let voxels = landmarks::signpost(&Palette::of(col.biome), directions);
        net.add_stamp(absolute(voxels, IVec3::new(x, trail_y.floor() as i32 + 1, z), 0));
    }

    net
}
