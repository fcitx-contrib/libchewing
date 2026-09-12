//! Word lattice represents many possible intepretations of the phonetic input.

use log::trace;

use crate::{
    conversion::{Composition, Gap, Symbol},
    dictionary::{CompositeDict, LookupStrategy},
    model::{Candidate, WordId},
    zhuyin::{Syllable, SyllableVec},
};

/// Word lattice contains different word sequences discovered from input.
#[derive(Debug)]
pub struct Lattice {
    pub(crate) len: usize,
    pub(crate) edges: Vec<Vec<Edge>>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Edge {
    pub(crate) end: u8,
    pub(crate) cand: Candidate,
}

impl Lattice {
    /// Builds a word lattice from any string, instead of phonetic input.
    ///
    /// # Parameters
    ///
    /// - `find_words`: A closure that maps a sub-string to a [`WordId`]
    pub fn from_str<F>(s: &str, find_words: F) -> Lattice
    where
        F: Fn(&str) -> Option<WordId>,
    {
        // Cache char indexes
        let mut chars_vec: Vec<usize> = s.char_indices().map(|ci| ci.0).collect();
        // Add end of string offset
        chars_vec.push(s.len());
        // Number of chars
        let len = chars_vec.len() - 1;

        let mut edges = vec![vec![]; len];
        for start in 0..len {
            let max_end = usize::min(start + SyllableVec::MAX_LEN, len);
            for end in (start + 1)..=max_end {
                let s_start = chars_vec[start];
                let s_end = chars_vec[end];
                let substr = &s[s_start..s_end];
                if let Some(wid) = find_words(&substr) {
                    edges[start].push(Edge {
                        end: end as u8,
                        cand: Candidate::Word {
                            wid,
                            hist_prob: f64::NEG_INFINITY,
                            user_pref: None,
                        },
                    });
                } else if (end - start) == 1 {
                    edges[start].push(Edge {
                        end: end as u8,
                        cand: Candidate::Grapheme(substr.chars().next().unwrap()),
                    });
                }
            }
        }
        Lattice { len, edges }
    }
}

/// Builds word lattice from a composition.
#[derive(Debug, Clone)]
pub struct LatticeBuilder {
    pub dict: CompositeDict,
    pub lookup_strategy: LookupStrategy,
}

impl LatticeBuilder {
    /// Finds all possible words sequences from a composition then creates a word lattice.
    pub fn build_lattice(&self, com: &Composition) -> Lattice {
        let len = com.len();
        let mut edges = vec![vec![]; len];
        for start in 0..com.symbols.len() {
            let max_end = usize::min(start + SyllableVec::MAX_LEN, com.symbols.len());
            for end in (start + 1)..=max_end {
                for cand in self.find_words(start, &com.symbols[start..end], com) {
                    edges[start].push(Edge {
                        end: end as u8,
                        cand,
                    });
                }
            }
        }
        Lattice { len, edges }
    }

    fn find_words(&self, start: usize, symbols: &[Symbol], com: &Composition) -> Vec<Candidate> {
        if symbols.len() == 1
            && let Some(sym) = symbols[0].to_char()
        {
            return vec![Candidate::Grapheme(sym)];
        }

        if symbols.iter().any(|sym| sym.is_char()) {
            return vec![];
        }

        let end = start + symbols.len();

        for i in (start..end).skip(1) {
            if let Some(Gap::Break) = com.gap(i) {
                // There exists a break point that forbids connecting these
                // syllables.
                trace!("No viable words for {:?} due to break point", symbols);
                return vec![];
            }
        }

        for selection in &com.selections {
            if selection.start == start && selection.end == end {
                return vec![Candidate::Word {
                    wid: selection.wid,
                    hist_prob: f64::NEG_INFINITY,
                    user_pref: None,
                }];
            }
            if selection.intersect_range(start, end) {
                // There's a conflicting partial intersecting selection.
                trace!(
                    "No viable word for {:?} due to conflicting user selection {:?}",
                    symbols, selection
                );
                return vec![];
            }
        }

        let syllables: Vec<Syllable> = symbols
            .iter()
            .map(|s| s.to_syllable().unwrap_or_default())
            .collect();

        self.dict.lookup(&syllables, self.lookup_strategy)
    }
}
