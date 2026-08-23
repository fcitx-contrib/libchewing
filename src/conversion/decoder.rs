//! Decode WordLattice to ranked hypotheses

use std::{
    cmp::{Ordering, Reverse},
    collections::{BTreeMap, BinaryHeap},
    ops::Neg,
};

use crate::{
    conversion::word_lattice::{Edge, WordLattice},
    lm::static_lm::StaticLm,
    model::Surface,
    user::{HistoryFreq, UserFreq},
};

#[derive(Debug)]
pub struct Decoder {
    pub user_freq: UserFreq,
    pub history_freq: HistoryFreq,
    pub lm: StaticLm,
}

#[derive(Debug, Default, Clone)]
pub struct Hypothesis {
    pub edges: Vec<Edge>,
}

impl Decoder {
    pub fn decoden(&self, lattice: &WordLattice, n: u8) -> Vec<Hypothesis> {
        if lattice.edges.is_empty() {
            return vec![Hypothesis::default()];
        }

        // This should match the ALPHA used in the training logic
        const LOG10_ALPHA: f64 = -0.39794; // log10(0.4)
        const UNIGRAM_FLOOR: f64 = -10.0;

        let paths = find_k_paths(n, lattice, |w1, w2| match (w1, w2) {
            (Surface::Word(wid1), Surface::Word(wid2)) => {
                // Attempt to get the bigram probability
                if let Some(bigram_prob) = self.lm.get(wid1.0, wid2.0) {
                    // Use the bigram probability directly
                    bigram_prob.neg()
                } else {
                    // Stupid back-off: penalty + unigram
                    let unigram_prob = self.lm.get(0, wid2.0).unwrap_or(UNIGRAM_FLOOR);
                    (LOG10_ALPHA + unigram_prob).neg()
                }
            }
            (Surface::Word(wid), _) | (_, Surface::Word(wid)) => {
                // Handle non-word transitions (edges to/from start/end)
                self.lm.get(0, wid.0).unwrap_or(UNIGRAM_FLOOR).neg()
            }
            _ => UNIGRAM_FLOOR.neg(),
        });

        debug_assert!(!paths.is_empty());
        paths
    }
}

#[derive(Debug)]
struct Path {
    priority: Reverse<OrderedF64>,
    cost: f64,
    front: StateCoord,
    tid: usize,
}

impl Eq for Path {}

impl PartialEq for Path {
    fn eq(&self, other: &Self) -> bool {
        self.priority.eq(&other.priority)
    }
}

impl PartialOrd for Path {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Path {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority.cmp(&other.priority)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
struct StateCoord {
    prev: Surface,
    curr: Edge,
}

/// Modified m-A* algorithm to find the N-best distinct result strings
///
/// Natalia Flerova, Radu Marinescu, and Rina
/// Dechter. 2016. Searching for the M best solutions in graphical
/// models. J. Artif. Int. Res. 55, 1 (January 2016), 889–952.
/// https://jair.org/index.php/jair/article/view/10995
fn find_k_paths<F>(k: u8, lattice: &WordLattice, cost_fn: F) -> Vec<Hypothesis>
where
    F: Fn(Surface, Surface) -> f64,
{
    let h = future_cost(lattice, &cost_fn);
    let len = lattice.len;
    let mut counter: BTreeMap<StateCoord, u8> = BTreeMap::new();
    let mut trails: Vec<(usize, Edge)> = vec![];
    let mut open = BinaryHeap::new();

    counter.insert(StateCoord::default(), 0);
    trails.push((0, Edge::default()));
    open.push(Path {
        priority: Reverse(OrderedF64(h[0])),
        cost: 0.0,
        front: StateCoord::default(),
        tid: 0,
    });

    let mut results = Vec::with_capacity(k as usize);

    while let Some(path) = open.pop() {
        let times = *counter.get(&path.front).expect("");
        if times >= k {
            continue;
        } else {
            counter.insert(path.front, times + 1);
        }

        if path.front.curr.end as usize == len {
            results.push(reconstruct(&trails, path.tid));
            if results.len() == k as usize {
                break;
            }
            continue;
        }

        for e in &lattice.edges[path.front.curr.end as usize] {
            let cost = path.cost + cost_fn(path.front.curr.surface, e.surface);
            let front = StateCoord {
                prev: path.front.curr.surface,
                curr: *e,
            };
            counter.entry(front).or_insert(0);

            let tid = trails.len();
            trails.push((path.tid, *e));

            open.push(Path {
                priority: Reverse(OrderedF64(cost + h[e.end as usize])),
                cost,
                front,
                tid,
            });
        }
    }
    results
        .into_iter()
        .map(|edges| Hypothesis { edges })
        .collect()
}

fn reconstruct(trails: &[(usize, Edge)], tid: usize) -> Vec<Edge> {
    let mut index = tid;
    let mut edges = vec![];
    while let Some(&(tid, edge)) = trails.get(index) {
        edges.push(edge);
        index = tid;
        if index == 0 {
            break;
        }
    }
    edges.reverse();
    edges
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct OrderedF64(f64);

impl Eq for OrderedF64 {}

impl PartialOrd for OrderedF64 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrderedF64 {
    fn cmp(&self, other: &Self) -> Ordering {
        // total_cmp is a total order over every f64 bit pattern (incl. ±inf, NaN),
        // so Eq/Ord invariants hold and the heap can never panic on comparison.
        self.0.total_cmp(&other.0)
    }
}

// h[v] = min cost of any path from node v to the sink (len).
// DAG with start < end, so process nodes in decreasing order.
fn future_cost<F>(lattice: &WordLattice, cost_fn: F) -> Vec<f64>
where
    F: Fn(Surface, Surface) -> f64,
{
    let len = lattice.len;
    let mut h = vec![f64::INFINITY; len + 1];
    h[len] = 0.0;
    for v in (0..len).rev() {
        for e in &lattice.edges[v] {
            let cost = cost_fn(Surface::None, e.surface);
            let c = cost + h[e.end as usize];
            if c < h[v] {
                h[v] = c;
            }
        }
    }
    h
}
