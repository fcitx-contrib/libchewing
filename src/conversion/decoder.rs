//! Decode WordLattice to ranked hypotheses

use std::{cmp::Ordering, ops::Neg};

use crate::{
    conversion::word_lattice::{Edge, Lattice},
    lm::static_lm::StaticLm,
    model::{Candidate, WordId},
};

/// Converts word lattice to possible sentence hypotheses.
///
/// The current algorithm runs with complexity O(L·E·K) where *L* is
/// the length of the sentence, *E* is the number of words in the lattice,
/// *K* is the number of returned results.
#[derive(Clone, Debug)]
pub struct Decoder {
    /// Bigram and unigram language model.
    pub lm: StaticLm,
    /// Bigram to unigram back-off weight.
    ///
    /// [`Decoder::LAMBDA`] can be used as the default.
    ///
    /// Valid lambda should be between 0.1 and 0.9. Values outside of that
    /// range will ruin the decoding accuracy.
    pub lambda: f64,
}

/// A possible sentence output.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Hypothesis {
    /// Candidate word or string in input order.
    pub candidates: Vec<Candidate>,
    /// Cost calculated from probabilities. Lower is better.
    pub cost: f64,
}

impl Decoder {
    /// Default bigram to unigram back-off weight
    pub const LAMBDA: f64 = 0.6;

    /// Decodes the word lattice and returns the n-best hypotheses.
    ///
    /// # Parameters
    ///
    /// - `lattice`: a word lattice
    /// - `n`: decode the n-best hypotheses
    ///
    /// # Returns
    ///
    /// A list of [`Hypothesis`] sorted from most likely to least likely.
    pub fn decoden(&self, mut lattice: Lattice, n: u8) -> Vec<Hypothesis> {
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
                        OrderedF64(self.cost_fun(Candidate::None, e.cand)),
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

        let paths = find_k_paths(n, &lattice, |w1, w2| self.cost_fun(w1, w2));

        debug_assert!(!paths.is_empty());
        paths
    }
    /// Rank the candidate list using the decoder's cost function.
    ///
    /// # Returns
    ///
    /// Candidates sorted from most likely to least likely.
    pub fn rank(&self, candidates: Vec<Candidate>) -> Vec<Candidate> {
        let mut ranked: Vec<_> = candidates
            .iter()
            .map(|c| (OrderedF64(self.cost_fun(Candidate::None, *c)), c))
            .collect();
        ranked.sort_by_key(|&(cost, _)| cost);
        ranked.into_iter().map(|(_, w)| *w).collect()
    }
    fn cost_fun(&self, w1: Candidate, w2: Candidate) -> f64 {
        let (wid1, wid2) = match (w1, w2) {
            (Candidate::Word { wid: a, .. }, Candidate::Word { wid: b, .. }) => (a, b),
            (Candidate::Word { wid: b, .. }, _) | (_, Candidate::Word { wid: b, .. }) => {
                (WordId(0), b)
            }
            _ => return ERROR_FLOOR.neg(),
        };
        let (hist_prob, user_pref) = match w2 {
            Candidate::Word {
                wid: _,
                hist_prob,
                user_pref,
            } => (hist_prob, user_pref),
            _ => (f64::NEG_INFINITY, None),
        };
        // Linear interpolation base unigram and history unigram
        let p_uni = log10_sum_exp(
            LOG10_LAMBDA_BASE_UNIGRAM + self.lm.unigram(wid2),
            LOG10_LAMBDA_HIST_UNIGRAM + hist_prob,
        );
        // Linear interpolation unigram and bigram
        let bigram_weight = self.lambda.log10();
        let unigram_weight = (1.0 - self.lambda).log10();
        let mixed = log10_sum_exp(
            bigram_weight + self.lm.bigram(wid1, wid2),
            unigram_weight + p_uni,
        );
        let cost = -mixed;
        let manual_gain = user_pref.unwrap_or(0) as f64 / 100.0;
        cost - MANUAL_BOOST_FACTOR * manual_gain
    }
}

const ERROR_FLOOR: f64 = -30.0;
const MANUAL_BOOST_FACTOR: f64 = 5.0;
const LOG10_LAMBDA_HIST_UNIGRAM: f64 = -0.221849;
const LOG10_LAMBDA_BASE_UNIGRAM: f64 = -0.30103;

#[inline]
fn log10_sum_exp(a: f64, b: f64) -> f64 {
    let hi = a.max(b);
    let lo = a.min(b);
    hi + (1.0 + 10f64.powf(lo - hi)).log10()
}

#[derive(Debug, Clone, Copy)]
struct KEntry {
    cost: f64,
    tid: usize,
}

/// k-best Viterbi over the state space `(position, last surface)`.
///
/// R. Schwartz and Y. . -L. Chow, "The N-best algorithms: an efficient
/// and exact procedure for finding the N most likely sentence
/// hypotheses," International Conference on Acoustics, Speech, and
/// Signal Processing, Albuquerque, NM, USA, 1990, pp. 81-84 vol.1, doi:
/// 10.1109/ICASSP.1990.115542. keywords: {Natural languages;Acoustic
/// beams;Speech},
fn find_k_paths<F>(k: u8, lattice: &Lattice, cost_fn: F) -> Vec<Hypothesis>
where
    F: Fn(Candidate, Candidate) -> f64,
{
    let len = lattice.len;
    let keep = k as usize;

    let mut trails: Vec<(usize, Edge)> = vec![];
    // layers[p]: up to k-best (cost, tid) prefixes ending at p with a surface.
    let mut layers: Vec<Vec<(Candidate, Vec<KEntry>)>> = vec![vec![]; len + 1];

    trails.push((0, Edge::default()));
    layers[0].push((Candidate::None, vec![KEntry { cost: 0.0, tid: 0 }]));

    for p in 0..len {
        let layer = std::mem::take(&mut layers[p]);
        for (prev, entries) in layer {
            for e in &lattice.edges[p] {
                let cost = cost_fn(prev, e.cand);
                let keep_list = get_keep_list(&mut layers[e.end as usize], e.cand);
                for ent in &entries {
                    insert_keep_k(keep_list, ent.cost + cost, ent.tid, e, &mut trails, keep);
                }
            }
        }
    }

    let mut finals: Vec<KEntry> = layers[len]
        .iter()
        .flat_map(|(_, es)| es.iter().copied())
        .collect();
    finals.sort_by_key(|e| OrderedF64(e.cost));
    finals
        .into_iter()
        .take(keep)
        .map(|e| Hypothesis {
            candidates: reconstruct(&trails, e.tid),
            cost: e.cost,
        })
        .collect()
}

fn get_keep_list<'a>(
    layer: &'a mut Vec<(Candidate, Vec<KEntry>)>,
    surface: Candidate,
) -> &'a mut Vec<KEntry> {
    if let Some(i) = layer.iter().position(|(s, _)| *s == surface) {
        &mut layer[i].1
    } else {
        layer.push((surface, vec![]));
        let last = layer.len() - 1;
        &mut layer[last].1
    }
}

fn insert_keep_k(
    keep_list: &mut Vec<KEntry>,
    cost: f64,
    parent_tid: usize,
    e: &Edge,
    trails: &mut Vec<(usize, Edge)>,
    keep: usize,
) {
    if keep == 0 {
        return;
    }
    if keep_list.len() == keep && keep_list[keep - 1].cost <= cost {
        return;
    }
    let pos = keep_list.partition_point(|x| x.cost < cost);
    if pos == keep {
        return;
    }
    let tid = trails.len();
    trails.push((parent_tid, *e));
    keep_list.insert(pos, KEntry { cost, tid });
    keep_list.truncate(keep);
}

fn reconstruct(trails: &[(usize, Edge)], tid: usize) -> Vec<Candidate> {
    let mut index = tid;
    let mut acc = vec![];
    while let Some(&(tid, edge)) = trails.get(index) {
        acc.push(edge.cand);
        index = tid;
        if index == 0 {
            break;
        }
    }
    acc.reverse();
    acc
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

#[cfg(test)]
mod test {
    use crate::{
        conversion::{Hypothesis, Lattice, decoder::find_k_paths, word_lattice::Edge},
        model::{Candidate, WordId},
    };

    fn word(wid: u32) -> Candidate {
        Candidate::Word {
            wid: WordId(wid),
            hist_prob: 0.0,
            user_pref: None,
        }
    }

    #[test]
    fn simple_shortest_path() {
        let lattice = Lattice {
            len: 2,
            edges: vec![
                vec![
                    Edge {
                        end: 1,
                        cand: word(1),
                    },
                    Edge {
                        end: 2,
                        cand: word(3),
                    },
                ],
                vec![Edge {
                    end: 2,
                    cand: word(2),
                }],
            ],
        };

        let cost_fn = |_w1, _w2| 1.0;

        assert_eq!(
            vec![Hypothesis {
                candidates: vec![word(3),],
                cost: 1.0
            }],
            find_k_paths(1, &lattice, cost_fn)
        );
    }

    #[test]
    fn multi_edge_shortest_path() {
        let lattice = Lattice {
            len: 2,
            edges: vec![
                vec![
                    Edge {
                        end: 1,
                        cand: word(1),
                    },
                    Edge {
                        end: 1,
                        cand: word(4),
                    },
                    Edge {
                        end: 2,
                        cand: word(5),
                    },
                ],
                vec![Edge {
                    end: 2,
                    cand: word(2),
                }],
            ],
        };

        let cost_fn = |_w1, w2| match w2 {
            Candidate::Word { wid, .. } => wid.0 as f64,
            _ => f64::INFINITY,
        };

        assert_eq!(
            vec![Hypothesis {
                candidates: vec![word(1), word(2),],
                cost: 3.0
            }],
            find_k_paths(1, &lattice, cost_fn)
        );
    }

    #[test]
    fn decode_empty_lattice() {
        let lattice = Lattice {
            len: 0,
            edges: vec![],
        };

        assert_eq!(
            vec![Hypothesis {
                candidates: vec![Candidate::None,],
                cost: 0.0
            }],
            find_k_paths(1, &lattice, |_, _| 1.0)
        );
    }
}
