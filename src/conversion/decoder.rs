//! Decode WordLattice to ranked hypotheses

use std::{
    cmp::{Ordering, Reverse},
    collections::{BTreeMap, BinaryHeap},
};

use crate::{
    conversion::word_lattice::{Edge, WordLattice},
    lm::static_lm::StaticLm,
    model::Seg,
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
    pub(crate) const MAX_OUT_HYPOTHESES: u8 = 10;

    pub fn decode(&self, lattice: &WordLattice) -> Vec<Hypothesis> {
        if lattice.edges.is_empty() {
            return vec![Hypothesis::default()];
        }
        let paths = find_k_paths(Self::MAX_OUT_HYPOTHESES, lattice, |w1, w2| match (w1, w2) {
            (Seg::Word(wid1), Seg::Word(wid2)) => {
                self.lm.get(wid1.0, wid2.0).unwrap_or_default() as f64
            }
            (Seg::Word(wid), Seg::Char(_))
            | (Seg::Word(wid), Seg::None)
            | (Seg::Char(_), Seg::Word(wid))
            | (Seg::None, Seg::Word(wid)) => self.lm.get(0, wid.0).unwrap_or_default() as f64,
            _ => 0.0,
        });
        debug_assert!(!paths.is_empty());
        paths
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct StateCoord {
    prev: Seg,
    start: u8,
    curr: Seg,
}

#[derive(Debug, Clone, Copy)]
struct StateValue<'a> {
    cost: f64,
    times: u8,
    parent: StateCoord,
    edge: &'a Edge,
}

/// Modified m-A* algorithm to find the N-best distinct result strings
///
/// Natalia Flerova, Radu Marinescu, and Rina
/// Dechter. 2016. Searching for the M best solutions in graphical
/// models. J. Artif. Int. Res. 55, 1 (January 2016), 889–952.
/// https://jair.org/index.php/jair/article/view/10995
fn find_k_paths<F>(k: u8, lattice: &WordLattice, cost_fn: F) -> Vec<Hypothesis>
where
    F: Fn(Seg, Seg) -> f64,
{
    let h = future_cost(lattice, &cost_fn);
    let len = lattice.len;
    let mut arena: BTreeMap<StateCoord, StateValue<'_>> = BTreeMap::new();
    let mut open = BinaryHeap::new();
    let source_state = StateCoord {
        prev: Seg::None,
        start: 0,
        curr: Seg::None,
    };
    arena.insert(
        source_state,
        StateValue {
            cost: 0.0,
            times: 0,
            parent: source_state,
            edge: &Edge {
                start: 0,
                end: 0,
                seg: Seg::None,
            },
        },
    );
    open.push(Reverse((OrderedF64(h[0]), source_state)));

    let mut results = Vec::with_capacity(k as usize);

    while let Some(Reverse((_, coord))) = open.pop() {
        let mut state = *arena.get(&coord).expect("");
        if state.times >= k {
            continue;
        } else {
            state.times += 1;
            arena.insert(coord, state);
        }

        if state.edge.end as usize == len {
            results.push(reconstruct(&arena, coord));
            if results.len() == k as usize {
                break;
            }
            continue;
        }

        for e in &lattice.edges[state.edge.end as usize] {
            let cost = state.cost + cost_fn(state.edge.seg, e.seg);
            let next_coord = StateCoord {
                prev: state.edge.seg,
                start: state.edge.end,
                curr: e.seg,
            };
            let next_state = StateValue {
                cost,
                times: 0,
                parent: coord,
                edge: e,
            };
            if let Some(v) = arena.get(&next_coord) {
                if v.times < k && v.cost > cost {
                    arena.insert(next_coord, next_state);
                }
            } else {
                arena.insert(next_coord, next_state);
            }
            open.push(Reverse((OrderedF64(cost + h[e.end as usize]), next_coord)));
        }
    }
    results
        .into_iter()
        .map(|edges| Hypothesis { edges })
        .collect()
}

fn reconstruct(arena: &BTreeMap<StateCoord, StateValue<'_>>, mut coord: StateCoord) -> Vec<Edge> {
    let mut edges = Vec::new();
    while let Some(v) = arena.get(&coord) {
        if v.edge.seg == Seg::None {
            break;
        }
        edges.push(*v.edge);
        coord = v.parent;
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
