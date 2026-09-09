//! Builds word lattice

use log::trace;

use crate::{
    conversion::{Composition, Gap, Symbol},
    dictionary::{CompositeDict, LookupStrategy},
    model::{Candidate, WordId},
    zhuyin::{Syllable, SyllableVec},
};

#[derive(Debug)]
pub struct WordLatticeBuilder {
    pub dict: CompositeDict,
    pub lookup_strategy: LookupStrategy,
}

#[derive(Debug)]
pub struct WordLattice {
    pub(crate) len: usize,
    pub(crate) edges: Vec<Vec<Edge>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Edge {
    pub start: u8,
    pub end: u8,
    pub cand: Candidate,
    pub boost: i32,
}

impl WordLattice {
    pub fn from_str<F>(s: &str, find_words: F) -> WordLattice
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
                        start: start as u8,
                        end: end as u8,
                        cand: Candidate::Word {
                            wid,
                            hist_count: 0,
                            user_pref: None,
                        },
                        boost: 0,
                    });
                } else if (end - start) == 1 {
                    edges[start].push(Edge {
                        start: start as u8,
                        end: end as u8,
                        cand: Candidate::Grapheme(substr.chars().next().unwrap()),
                        boost: 0,
                    });
                }
            }
        }
        WordLattice { len, edges }
    }
}

impl WordLatticeBuilder {
    pub(crate) fn to_lattice(&self, com: &Composition) -> WordLattice {
        let len = com.len();
        let mut edges = vec![vec![]; len];
        for start in 0..com.symbols.len() {
            let max_end = usize::min(start + SyllableVec::MAX_LEN, com.symbols.len());
            for end in (start + 1)..=max_end {
                for (surface, boost) in self.find_words(start, &com.symbols[start..end], com) {
                    edges[start].push(Edge {
                        start: start as u8,
                        end: end as u8,
                        cand: surface,
                        boost,
                    });
                }
            }
        }
        WordLattice { len, edges }
    }

    fn dict_lookup(&self, syllables: &[Syllable]) -> Vec<(Candidate, i32)> {
        self.dict
            .lookup(syllables, self.lookup_strategy)
            .into_iter()
            .map(|(wid, b)| {
                (
                    Candidate::Word {
                        wid,
                        hist_count: 0,
                        user_pref: None,
                    },
                    b,
                )
            })
            .collect()
    }

    fn find_words(
        &self,
        start: usize,
        symbols: &[Symbol],
        com: &Composition,
    ) -> Vec<(Candidate, i32)> {
        if symbols.len() == 1
            && let Some(sym) = symbols[0].to_char()
        {
            return vec![(Candidate::Grapheme(sym), 0)];
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
                return vec![(
                    Candidate::Word {
                        wid: selection.wid,
                        hist_count: 0,
                        user_pref: None,
                    },
                    0,
                )];
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

        self.dict_lookup(&syllables)
    }
}
