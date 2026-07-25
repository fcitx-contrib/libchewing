//! Decode WordLattice to ranked hypotheses

use std::{
    cmp::{Ordering, Reverse},
    collections::{BTreeMap, BinaryHeap},
};

use crate::{
    conversion::word_lattice::{Edge, WordLattice},
    model::Seg,
    user::{HistoryFreq, UserFreq},
};

pub(crate) struct Decoder {
    user_freq: UserFreq,
    history_freq: HistoryFreq,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct Hypothesis {
    pub(crate) edges: Vec<Edge>,
}

impl Decoder {
    pub(crate) const MAX_OUT_HYPOTHESES: usize = 10;

    pub(crate) fn decode(lattice: &WordLattice) -> Vec<Hypothesis> {
        if lattice.edges.is_empty() {
            return vec![Hypothesis::default()];
        }
        let paths = find_k_paths(Self::MAX_OUT_HYPOTHESES, lattice, |w1, w2| 1.0);
        debug_assert!(!paths.is_empty());
        paths
    }
}

struct Hyp<'a> {
    /// cost so far (source -> node)
    g: f64,
    node: u8,
    /// index into arena, for path reconstruction
    parent: u8,
    edge: &'a Edge,
}

/// Modified m-A* algorithm to find the N-best distinct result strings
///
/// Natalia Flerova, Radu Marinescu, and Rina
/// Dechter. 2016. Searching for the M best solutions in graphical
/// models. J. Artif. Int. Res. 55, 1 (January 2016), 889–952.
/// https://jair.org/index.php/jair/article/view/10995
fn find_k_paths<F>(k: usize, lattice: &WordLattice, cost_fn: F) -> Vec<Hypothesis>
where
    F: Fn(Seg, Seg) -> f64,
{
    let h = future_cost(lattice, &cost_fn);
    let len = lattice.len;
    let mut arena: Vec<Hyp<'_>> = Vec::new();
    // Min-heap on f = g + h[node]; store Reverse((f_bits, arena_idx)).
    let mut open = BinaryHeap::new();
    let dummy_edge = Edge {
        start: 0,
        end: 0,
        seg: Seg::None,
    };

    arena.push(Hyp {
        g: 0.0,
        node: 0,
        parent: u8::MAX,
        edge: &dummy_edge,
    });
    open.push(Reverse((OrderedF64(h[0]), 0u8)));

    // How many times a node has been visited
    let mut closed: BTreeMap<u8, u8> = BTreeMap::new();
    let mut results = Vec::with_capacity(k);

    while let Some(Reverse((_, idx))) = open.pop() {
        let (node, g) = (arena[idx as usize].node, arena[idx as usize].g);
        if let Some(&times) = closed.get(&node) {
            if times as usize >= k {
                // a cheaper path to this node already won
                continue;
            } else {
                closed.insert(node, times + 1);
            }
        }

        if node as usize == len {
            // a new distinct reading
            results.push(reconstruct(&arena, idx));
            if results.len() == k {
                break;
            }
            continue;
        }

        for e in &lattice.edges[node as usize] {
            // Prune states already finalized (cheaper)
            if let Some(&times) = closed.get(&e.end) {
                if times as usize >= k {
                    continue;
                }
            }
            let ng = g + cost_fn(arena[idx as usize].edge.seg, e.seg);
            let child = arena.len() as u8;
            arena.push(Hyp {
                g: ng,
                node: e.end,
                parent: idx,
                edge: &e,
            });
            open.push(Reverse((OrderedF64(ng + h[e.end as usize]), child)));
        }
    }
    results
        .into_iter()
        .map(|edges| Hypothesis { edges })
        .collect()
}

fn reconstruct(arena: &[Hyp<'_>], idx: u8) -> Vec<Edge> {
    let mut idx = idx as usize;
    let mut edges = Vec::new();
    // usize::MAX marks the root sentinel
    while arena[idx].parent != u8::MAX {
        let p = arena[idx].parent as usize;
        edges.push(Edge {
            start: arena[p].node,
            end: arena[idx].node,
            seg: arena[idx].edge.seg,
        });
        idx = p;
    }
    // leaf -> root collected above; flip to source -> sink
    edges.reverse();
    edges
}

#[derive(Clone, Copy, PartialEq)]
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
    F: Fn(Seg, Seg) -> f64,
{
    let len = lattice.len;
    let mut h = vec![f64::INFINITY; len + 1];
    h[len] = 0.0;
    for v in (0..len).rev() {
        for e in &lattice.edges[v] {
            let cost = cost_fn(Seg::None, e.seg);
            let c = cost + h[e.end as usize];
            if c < h[v] {
                h[v] = c;
            }
        }
    }
    h
}
