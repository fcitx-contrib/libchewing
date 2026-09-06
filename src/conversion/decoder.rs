//! Decode WordLattice to ranked hypotheses

use std::{
    cmp::{Ordering, Reverse},
    collections::BinaryHeap,
    ops::Neg,
};

use crate::{
    conversion::word_lattice::{Edge, WordLattice},
    lm::static_lm::StaticLm,
    model::{Surface, WordId, WordOrig},
};

#[derive(Clone, Debug)]
pub struct Decoder {
    pub lm: StaticLm,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Hypothesis {
    pub edges: Vec<Edge>,
    pub cost: f64,
}

impl Decoder {
    pub fn decoden(&self, mut lattice: WordLattice, n: u8) -> Vec<Hypothesis> {
        if lattice.edges.is_empty() {
            return vec![Hypothesis::default()];
        }

        // Prune
        const KEEP_PER_SPAN: usize = 10;
        for es in lattice.edges.iter_mut() {
            if es.len() <= KEEP_PER_SPAN {
                continue;
            }
            let mut ranked: Vec<_> = es
                .iter()
                .map(|e| {
                    (
                        e.end,
                        OrderedF64(cost_fun(&self.lm, Surface::None, e.surface, e.boost)),
                        *e,
                    )
                })
                .collect();
            // group by span end, cheapest-first within each span
            ranked.sort_unstable_by_key(|&(end, cost, _)| (end, cost));

            es.clear();
            for group in ranked.chunk_by(|a, b| a.0 == b.0) {
                es.extend(group.iter().take(KEEP_PER_SPAN).map(|&(_, _, e)| e));
            }
        }

        let paths = find_k_paths(n, &lattice, |w1, w2, w2boost| {
            cost_fun(&self.lm, w1, w2, w2boost)
        });

        debug_assert!(!paths.is_empty());
        paths
    }
    pub fn rank(&self, candidates: Vec<(WordId, i32)>) -> Vec<WordId> {
        let mut ranked: Vec<_> = candidates
            .iter()
            .map(|c| {
                (
                    OrderedF64(cost_fun(&self.lm, Surface::None, Surface::Word(c.0), c.1)),
                    c.0,
                )
            })
            .collect();
        ranked.sort_unstable_by_key(|&(cost, _)| cost);
        ranked.into_iter().map(|(_, w)| w).collect()
    }
}

const LOG10_ALPHA_0_4: f64 = -0.39794;
const USER_FLOOR: f64 = -2.0;
const UNIGRAM_FLOOR: f64 = -20.0;
const ERROR_FLOOR: f64 = -30.0;
const HISTORY_BOOST_FACTOR: f64 = 0.5;
const MANUAL_BOOST_FACTOR: f64 = 2.0;

fn cost_fun(lm: &StaticLm, w1: Surface, w2: Surface, w2boost: i32) -> f64 {
    let (wid1, wid2) = match (w1, w2) {
        (Surface::Word(wid1), Surface::Word(wid2)) => (wid1, wid2),
        (Surface::Word(wid), _) | (_, Surface::Word(wid)) => (WordId(0), wid),
        _ => return ERROR_FLOOR.neg(),
    };
    let unigram_prob = if let Some(prob) = lm.get(0, wid2.0) {
        prob
    } else if matches!(wid2.orig(), WordOrig::User) {
        USER_FLOOR
    } else {
        UNIGRAM_FLOOR
    };
    // Attempt to get the bigram probability
    let general_cost = if let Some(bigram_prob) = lm.get(wid1.0, wid2.0) {
        // Use the bigram probability directly
        bigram_prob.neg()
    } else {
        // Stupid back-off: penalty + unigram
        (LOG10_ALPHA_0_4 + unigram_prob).neg()
    };
    // let hist_unigram_prob = self.history_freq.get(wid2).unwrap_or(unigram_prob);
    // let hist_gain = (hist_unigram_prob - unigram_prob).neg();
    let hist_gain = 0.0;
    let manual_freq = w2boost as f64;
    let manual_gain = if manual_freq >= 0.0 {
        (manual_freq + 1.0).log10()
    } else {
        -manual_freq.abs().log10()
    };
    let cost = general_cost - HISTORY_BOOST_FACTOR * hist_gain - MANUAL_BOOST_FACTOR * manual_gain;
    cost
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
    F: Fn(Surface, Surface, i32) -> f64,
{
    let h = future_cost(lattice, &cost_fn);
    let len = lattice.len;
    let mut trails: Vec<(usize, Edge)> = vec![];
    let mut open = BinaryHeap::new();

    trails.push((0, Edge::default()));
    open.push(Path {
        priority: Reverse(OrderedF64(h[0])),
        cost: 0.0,
        front: StateCoord::default(),
        tid: 0,
    });

    let mut results = Vec::with_capacity(k as usize);

    while let Some(path) = open.pop() {
        if path.front.curr.end as usize == len {
            results.push((reconstruct(&trails, path.tid), path.cost));
            if results.len() == k as usize {
                // should already be sorted but just in case the heuristic
                // is not admissible.
                // TODO: emitt warning in that case.
                results.sort_by_key(|r| OrderedF64(r.1));
                break;
            }
            continue;
        }

        for e in &lattice.edges[path.front.curr.end as usize] {
            let cost = path.cost + cost_fn(path.front.curr.surface, e.surface, e.boost);
            let tid = trails.len();

            trails.push((path.tid, *e));
            open.push(Path {
                priority: Reverse(OrderedF64(cost + h[e.end as usize])),
                cost,
                front: StateCoord {
                    prev: path.front.curr.surface,
                    curr: *e,
                },
                tid,
            });
        }
    }
    results
        .into_iter()
        .map(|(edges, cost)| Hypothesis { edges, cost })
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

// h[v] = cost lower bound of any path from node v to the sink (len).
// DAG with start < end, so process nodes in decreasing order.
fn future_cost<F>(lattice: &WordLattice, cost_fn: F) -> Vec<f64>
where
    F: Fn(Surface, Surface, i32) -> f64,
{
    let len = lattice.len;

    // Precompute incoming edges: for each position, which surfaces can reach it
    let mut incoming: Vec<Vec<(usize, Surface, i32)>> = vec![vec![]; len + 1];
    for u in 0..len {
        for e in &lattice.edges[u] {
            incoming[e.end as usize].push((u, e.surface, e.boost));
        }
    }

    let mut h = vec![f64::INFINITY; len + 1];
    h[len] = 0.0;

    for v in (0..len).rev() {
        for e in &lattice.edges[v] {
            let curr = e.surface;
            let boost = e.boost;

            // Minimum cost to generate `curr` at position v,
            // considering all possible predecessors
            let min_step = if v == 0 {
                // Start of input buffer: unigram only
                cost_fn(Surface::None, curr, boost)
            } else {
                // Min over all predecessors
                let mut best = f64::INFINITY;
                for &(_, prev_surf, _) in &incoming[v] {
                    let c = cost_fn(prev_surf, curr, boost);
                    if c < best {
                        best = c;
                    }
                }
                best
            };

            let c = min_step + h[e.end as usize];
            if c < h[v] {
                h[v] = c;
            }
        }
    }
    h
}

#[cfg(test)]
mod test {
    use crate::{
        conversion::{Hypothesis, WordLattice, decoder::find_k_paths, word_lattice::Edge},
        model::{Surface, WordId},
    };

    #[test]
    fn simple_shortest_path() {
        let lattice = WordLattice {
            len: 2,
            edges: vec![
                vec![
                    Edge {
                        start: 0,
                        end: 1,
                        surface: Surface::Word(WordId(1)),
                        boost: 0,
                    },
                    Edge {
                        start: 0,
                        end: 2,
                        surface: Surface::Word(WordId(3)),
                        boost: 0,
                    },
                ],
                vec![Edge {
                    start: 1,
                    end: 2,
                    surface: Surface::Word(WordId(2)),
                    boost: 0,
                }],
            ],
        };

        let cost_fn = |_w1, _w2, _b| 1.0;

        assert_eq!(
            vec![Hypothesis {
                edges: vec![Edge {
                    start: 0,
                    end: 2,
                    surface: Surface::Word(WordId(3)),
                    boost: 0,
                },],
                cost: 1.0
            }],
            find_k_paths(1, &lattice, cost_fn)
        );
    }

    #[test]
    fn multi_edge_shortest_path() {
        let lattice = WordLattice {
            len: 2,
            edges: vec![
                vec![
                    Edge {
                        start: 0,
                        end: 1,
                        surface: Surface::Word(WordId(1)),
                        boost: -1,
                    },
                    Edge {
                        start: 0,
                        end: 1,
                        surface: Surface::Word(WordId(4)),
                        boost: -2,
                    },
                    Edge {
                        start: 0,
                        end: 2,
                        surface: Surface::Word(WordId(3)),
                        boost: -3,
                    },
                ],
                vec![Edge {
                    start: 1,
                    end: 2,
                    surface: Surface::Word(WordId(2)),
                    boost: -1,
                }],
            ],
        };

        let cost_fn = |_w1, _w2, b| -(b as f64);

        assert_eq!(
            vec![Hypothesis {
                edges: vec![
                    Edge {
                        start: 0,
                        end: 1,
                        surface: Surface::Word(WordId(1)),
                        boost: -1,
                    },
                    Edge {
                        start: 1,
                        end: 2,
                        surface: Surface::Word(WordId(2)),
                        boost: -1,
                    }
                ],
                cost: 2.0
            }],
            find_k_paths(1, &lattice, cost_fn)
        );
    }

    #[test]
    fn decode_empty_lattice() {
        let lattice = WordLattice {
            len: 0,
            edges: vec![],
        };

        assert_eq!(
            vec![Hypothesis {
                edges: vec![Edge {
                    start: 0,
                    end: 0,
                    surface: Surface::None,
                    boost: 0
                }],
                cost: 0.0
            }],
            find_k_paths(1, &lattice, |_, _, _| 1.0)
        );
    }
}
