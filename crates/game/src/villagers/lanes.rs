//! The ways people walk in a settlement: its own lanes, and the paths from its doors and
//! wells and stalls out to them, as a small graph. Nobody walks anywhere else, so no
//! general pathfinding is needed: a walk is the shortest way along the graph.

use std::{cmp::Ordering, collections::BinaryHeap};

use glam::{Vec2, Vec3};

/// Every how many points of a lane (two voxels apart) a node is kept: often enough to
/// follow its bends and its rise and fall.
const EVERY: usize = 2;
/// How near two lanes' nodes must come to be joined where the lanes meet or cross, in
/// voxels.
const JOIN: f32 = 5.0;
/// How far a broken piece of lane may be from the rest to be joined to it all the same.
const BRIDGE: f32 = 9.0;

/// The lanes of one settlement as a graph: nodes on the ground, continent-local voxels
/// (feet level), and the ways between them.
#[derive(Clone, Debug, Default)]
pub struct Lanes {
    pub nodes: Vec<Vec3>,
    edges: Vec<Vec<(u32, f32)>>,
    /// Which lane each node is on: the green's middle and the paths have their own.
    lane: Vec<u32>,
}

#[derive(Clone, Copy, PartialEq)]
struct Next {
    cost: f32,
    node: u32,
}

impl Eq for Next {}

impl Ord for Next {
    fn cmp(&self, o: &Self) -> Ordering {
        // Least first, and the lower node of two as near, so that a walk is the same
        // whatever order things were looked at.
        o.cost.total_cmp(&self.cost).then(o.node.cmp(&self.node))
    }
}

impl PartialOrd for Next {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl Lanes {
    /// The graph of `lanes` (lines of points two voxels apart, continent-local), with
    /// `height` giving the feet's level at a column, and the green at `centre` (feet
    /// level) `green` voxels across: every lane node on the green is joined to its middle,
    /// since it is open ground to cross.
    pub fn new(lanes: &[Vec<Vec2>], centre: Vec3, green: f32, height: &mut dyn FnMut(Vec2) -> f32) -> Self {
        let mut g = Lanes::default();
        let mid = g.add(centre, 0);
        for (k, pts) in lanes.iter().enumerate() {
            if pts.len() < 2 {
                continue;
            }
            let mut last: Option<u32> = None;
            let keep: Vec<usize> = (0..pts.len()).filter(|i| i % EVERY == 0 || *i == pts.len() - 1).collect();
            for i in keep {
                let p = pts[i];
                let n = g.add(Vec3::new(p.x, height(p), p.y), k as u32 + 1);
                if let Some(l) = last {
                    g.join(l, n);
                }
                last = Some(n);
            }
        }
        // Where lanes meet: each node to the nearest of every other lane near it.
        let n = g.nodes.len();
        for i in 1..n {
            let mut best: Vec<(u32, u32, f32)> = Vec::new();
            for j in 1..n {
                if g.lane[i] == g.lane[j] {
                    continue;
                }
                let d = flat(g.nodes[i]).distance(flat(g.nodes[j]));
                if d < JOIN {
                    match best.iter_mut().find(|b| b.0 == g.lane[j]) {
                        Some(b) if d < b.2 => *b = (g.lane[j], j as u32, d),
                        Some(_) => {}
                        None => best.push((g.lane[j], j as u32, d)),
                    }
                }
            }
            for (_, j, _) in best {
                g.join(i as u32, j);
            }
        }
        // Across the green.
        for i in 1..n {
            if flat(g.nodes[i]).distance(flat(centre)) < green * 0.5 + 2.0 {
                g.join(mid, i as u32);
            }
        }
        g.bridge();
        g
    }

    fn add(&mut self, p: Vec3, lane: u32) -> u32 {
        self.nodes.push(p);
        self.edges.push(Vec::new());
        self.lane.push(lane);
        (self.nodes.len() - 1) as u32
    }

    fn join(&mut self, a: u32, b: u32) {
        if a == b || self.edges[a as usize].iter().any(|e| e.0 == b) {
            return;
        }
        let d = self.nodes[a as usize].distance(self.nodes[b as usize]);
        self.edges[a as usize].push((b, d));
        self.edges[b as usize].push((a, d));
    }

    /// Joins any piece of lane that reaches nothing else to the nearest node of the rest,
    /// if it lies close: a lane laid in pieces where water or a plot broke it.
    fn bridge(&mut self) {
        loop {
            let reached = self.reach(0);
            let Some((a, b, d)) = (0..self.nodes.len())
                .filter(|&i| !reached[i])
                .flat_map(|i| (0..self.nodes.len()).filter(|&j| reached[j]).map(move |j| (i, j)))
                .map(|(i, j)| (i, j, flat(self.nodes[i]).distance(flat(self.nodes[j]))))
                .min_by(|x, y| x.2.total_cmp(&y.2).then((x.0, x.1).cmp(&(y.0, y.1))))
            else {
                return;
            };
            if d > BRIDGE {
                return;
            }
            self.join(a as u32, b as u32);
        }
    }

    /// Which nodes can be walked to from `from`.
    pub fn reach(&self, from: u32) -> Vec<bool> {
        let mut seen = vec![false; self.nodes.len()];
        let mut todo = vec![from];
        while let Some(n) = todo.pop() {
            if std::mem::replace(&mut seen[n as usize], true) {
                continue;
            }
            todo.extend(self.edges[n as usize].iter().map(|e| e.0).filter(|&m| !seen[m as usize]));
        }
        seen
    }

    /// A node for a path from `p` to join the lanes at, and how far it is (across the
    /// ground), among those `reached` from the green: the nearest within `reach` of `p`
    /// that `ok` says yes to, else the nearest of all.
    pub fn nearest_where(&self, p: Vec3, reached: &[bool], reach: f32, ok: impl Fn(Vec3) -> bool) -> Option<(u32, f32)> {
        let mut by: Vec<(u32, f32)> = (0..self.nodes.len()).filter(|&i| reached[i]).map(|i| (i as u32, flat(self.nodes[i]).distance(flat(p)))).collect();
        by.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        by.iter().take_while(|n| n.1 <= reach).copied().find(|&(i, _)| ok(self.nodes[i as usize])).or(by.first().copied())
    }

    /// How far it is along the lanes from `from` to every node (infinite where none
    /// reaches).
    pub fn distances(&self, from: u32) -> Vec<f32> {
        self.search(from, None).0
    }

    /// The shortest way from `a` to `b`, node by node, both ends included; None where
    /// they are not joined.
    pub fn route(&self, a: u32, b: u32) -> Option<Vec<u32>> {
        let (dist, back) = self.search(a, Some(b));
        if !dist[b as usize].is_finite() {
            return None;
        }
        let mut out = vec![b];
        let mut n = b;
        while n != a {
            n = back[n as usize];
            out.push(n);
        }
        out.reverse();
        Some(out)
    }

    fn search(&self, from: u32, to: Option<u32>) -> (Vec<f32>, Vec<u32>) {
        let mut dist = vec![f32::INFINITY; self.nodes.len()];
        let mut back = vec![u32::MAX; self.nodes.len()];
        let mut heap = BinaryHeap::new();
        dist[from as usize] = 0.0;
        heap.push(Next { cost: 0.0, node: from });
        while let Some(Next { cost, node }) = heap.pop() {
            if Some(node) == to {
                break;
            }
            if cost > dist[node as usize] {
                continue;
            }
            for &(m, d) in &self.edges[node as usize] {
                let c = cost + d;
                if c < dist[m as usize] {
                    dist[m as usize] = c;
                    back[m as usize] = node;
                    heap.push(Next { cost: c, node: m });
                }
            }
        }
        (dist, back)
    }

    /// Every way between two nodes, as its two ends: what a walk may follow.
    #[cfg(test)]
    pub fn ways(&self) -> impl Iterator<Item = (Vec3, Vec3)> + '_ {
        self.edges.iter().enumerate().flat_map(move |(a, es)| es.iter().filter(move |e| e.0 as usize > a).map(move |e| (self.nodes[a], self.nodes[e.0 as usize])))
    }
}

pub fn flat(p: Vec3) -> Vec2 {
    Vec2::new(p.x, p.z)
}

/// A walk as a line of points, and how far along it each point is.
#[derive(Clone, Debug, Default)]
pub struct Walk {
    pub pts: Vec<Vec3>,
    along: Vec<f32>,
}

impl Walk {
    pub fn new(pts: Vec<Vec3>) -> Self {
        let mut pts = pts;
        pts.dedup_by(|a, b| a.distance_squared(*b) < 1e-6);
        let mut along = Vec::with_capacity(pts.len());
        let mut d = 0.0;
        for (i, p) in pts.iter().enumerate() {
            if i > 0 {
                d += p.distance(pts[i - 1]);
            }
            along.push(d);
        }
        Walk { pts, along }
    }

    pub fn length(&self) -> f32 {
        self.along.last().copied().unwrap_or(0.0)
    }

    /// Where the walk has got to `d` voxels along it, and which way it is going there.
    pub fn at(&self, d: f32) -> (Vec3, Vec2) {
        match self.pts.len() {
            0 => (Vec3::ZERO, Vec2::X),
            1 => (self.pts[0], Vec2::X),
            n => {
                let d = d.clamp(0.0, self.length());
                let i = self.along.partition_point(|&a| a <= d).clamp(1, n - 1);
                let (a, b) = (self.pts[i - 1], self.pts[i]);
                let span = (self.along[i] - self.along[i - 1]).max(1e-6);
                let t = ((d - self.along[i - 1]) / span).clamp(0.0, 1.0);
                (a.lerp(b, t), (flat(b) - flat(a)).normalize_or(Vec2::X))
            }
        }
    }
}
