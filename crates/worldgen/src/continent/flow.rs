//! Flow routing on a square grid: which cell each land cell drains to, in what order
//! to process cells, and how much water each one carries.

use std::{cmp::Reverse, collections::BinaryHeap};

use crate::noise::hash2;

/// Offsets to the 8 neighbours as (dx, dz, distance in cells).
pub const NEIGHBOURS: [(i32, i32, f32); 8] = [
    (1, 0, 1.0),
    (-1, 0, 1.0),
    (0, 1, 1.0),
    (0, -1, 1.0),
    (1, 1, std::f32::consts::SQRT_2),
    (-1, 1, std::f32::consts::SQRT_2),
    (1, -1, std::f32::consts::SQRT_2),
    (-1, -1, std::f32::consts::SQRT_2),
];

/// Rise added per cell when filling depressions, so filled areas still drain.
const EPSILON: f32 = 1e-3;

/// Reusable routing state for one grid size.
pub struct Flow {
    pub w: usize,
    /// Index offsets matching [`NEIGHBOURS`].
    pub offsets: [isize; 8],
    /// Surface with every depression filled to its spill level (0 for ocean cells).
    /// Strictly decreasing along each cell's flow path.
    pub filled: Vec<f32>,
    /// Downstream neighbour of each land cell; ocean cells point to themselves.
    pub rcv: Vec<u32>,
    /// Distance to the receiver in cells (1 or sqrt 2).
    pub rcv_dist: Vec<f32>,
    /// Land cells ordered so every cell comes after its receiver.
    pub order: Vec<u32>,
    /// Water passing through each cell (its own rain plus everything upstream).
    pub discharge: Vec<f32>,
    /// Extra land cells, with their heights, that the flood starts from as well as the
    /// coast: where a range crosses the border, water may leave the cell there.
    pub outlets: Vec<(u32, f32)>,
    closed: Vec<bool>,
    heap: BinaryHeap<Reverse<u64>>,
    seed: u32,
}

impl Flow {
    pub fn new(w: usize, seed: u32) -> Self {
        let n = w * w;
        let wi = w as isize;
        Self {
            w,
            offsets: NEIGHBOURS.map(|(dx, dz, _)| dz as isize * wi + dx as isize),
            filled: vec![0.0; n],
            rcv: vec![0; n],
            rcv_dist: vec![1.0; n],
            order: Vec::with_capacity(n),
            discharge: vec![0.0; n],
            outlets: Vec::new(),
            closed: vec![false; n],
            heap: BinaryHeap::new(),
            seed,
        }
    }

    /// Routes flow over `height`. `ocean` cells are fixed outlets at sea level 0; the
    /// outermost ring of the grid must be ocean. `rain` is the water each cell adds.
    ///
    /// Depressions are handled with a priority flood (Barnes et al. 2014): the land is
    /// flooded inward from the coast, lowest cell first, raising each cell to at least
    /// the level it was reached from. Every land cell therefore has a strictly lower
    /// neighbour on the filled surface, and cells are visited in downstream-first order.
    pub fn route(&mut self, height: &[f32], ocean: &[bool], rain: &[f32], cell_area: f32) {
        let (w, n) = (self.w, self.w * self.w);
        self.order.clear();
        self.heap.clear();
        // Heap entries pack (level bits, index): non-negative floats order like their bits.
        let key = |level: f32, i: usize| Reverse((level.to_bits() as u64) << 32 | i as u64);

        for i in 0..n {
            self.closed[i] = ocean[i];
            if ocean[i] {
                self.filled[i] = 0.0;
                self.rcv[i] = i as u32;
            }
        }
        // Seed with the ocean cells that touch land.
        for z in 1..w - 1 {
            for x in 1..w - 1 {
                let i = z * w + x;
                if ocean[i] && self.offsets.iter().any(|&o| !ocean[(i as isize + o) as usize]) {
                    self.heap.push(key(0.0, i));
                }
            }
        }
        for &(i, level) in &self.outlets {
            let i = i as usize;
            if !self.closed[i] {
                self.closed[i] = true;
                self.filled[i] = level.max(EPSILON);
                self.heap.push(key(self.filled[i], i));
            }
        }
        while let Some(Reverse(k)) = self.heap.pop() {
            let level = f32::from_bits((k >> 32) as u32);
            let c = (k & 0xFFFF_FFFF) as usize;
            // Cells leave the heap in order of filled level, and a receiver is always
            // lower than its donors, so pop order is downstream-first.
            if !ocean[c] {
                self.order.push(c as u32);
            }
            for &o in &self.offsets {
                let j = (c as isize + o) as usize;
                if !self.closed[j] {
                    self.closed[j] = true;
                    let f = height[j].max(level + EPSILON);
                    self.filled[j] = f;
                    self.heap.push(key(f, j));
                }
            }
        }

        // Steepest descent on the filled surface, with each direction's slope scaled by a
        // fixed per-cell random weight. Pure steepest descent on a grid can only run in
        // 8 directions and locks valleys into long straight lines; the weights let paths
        // wander where slopes are similar while still following any clear gradient.
        // They never change between calls, so channels stay put and can incise.
        for &i in &self.order {
            let i = i as usize;
            let here = self.filled[i];
            let bits = hash2(self.seed, i as i32, self.w as i32);
            let (mut best, mut best_score, mut best_dist) = (i, 0.0, 1.0);
            for (k, &o) in self.offsets.iter().enumerate() {
                let j = (i as isize + o) as usize;
                let dist = NEIGHBOURS[k].2;
                let slope = (here - self.filled[j]) / dist;
                let weight = 0.55 + ((bits >> (k * 4)) & 15) as f32 * (0.9 / 15.0);
                let score = slope * weight;
                if score > best_score {
                    (best, best_score, best_dist) = (j, score, dist);
                }
            }
            debug_assert!(best != i, "land cell without a lower neighbour");
            self.rcv[i] = best as u32;
            self.rcv_dist[i] = best_dist;
        }

        // Accumulate water from the divides down to the sea.
        for i in 0..n {
            self.discharge[i] = if ocean[i] { 0.0 } else { rain[i] * cell_area };
        }
        for &i in self.order.iter().rev() {
            let r = self.rcv[i as usize] as usize;
            self.discharge[r] += self.discharge[i as usize];
        }
    }
}
