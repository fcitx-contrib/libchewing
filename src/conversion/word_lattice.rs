//! Builds word lattice

use log::trace;

use crate::{
    conversion::{Composition, Gap, Symbol},
    dictionary::LookupStrategy,
    lm::StaticDict,
    model::{Surface, WordId},
    user::{HistoryDict, UserDict},
    zhuyin::Syllable,
};

#[derive(Debug)]
pub struct WordLatticeBuilder {
    pub static_dict: StaticDict,
    pub rare_dict: StaticDict,
    pub user_dict: UserDict,
    pub history_dict: HistoryDict,
}

#[derive(Debug)]
pub struct WordLattice {
    pub(crate) len: usize,
    pub(crate) edges: Vec<Vec<Edge>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Edge {
    pub start: u8,
    pub end: u8,
    pub surface: Surface,
    pub boost: i32,
}

// Assume no words in the dictionary are longer than MAX_PHRASE_LEN syllables.
const MAX_PHRASE_LEN: usize = 15;

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
            let max_end = usize::min(start + MAX_PHRASE_LEN, len);
            for end in (start + 1)..=max_end {
                let s_start = chars_vec[start];
                let s_end = chars_vec[end];
                let substr = &s[s_start..s_end];
                if let Some(wid) = find_words(&substr) {
                    edges[start].push(Edge {
                        start: start as u8,
                        end: end as u8,
                        surface: Surface::Word(wid),
                        boost: 0,
                    });
                } else if (end - start) == 1 {
                    edges[start].push(Edge {
                        start: start as u8,
                        end: end as u8,
                        surface: Surface::Char(substr.chars().next().unwrap()),
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
            let max_end = usize::min(start + MAX_PHRASE_LEN, com.symbols.len());
            for end in (start + 1)..=max_end {
                for (surface, boost) in self.find_words(start, &com.symbols[start..end], com) {
                    edges[start].push(Edge {
                        start: start as u8,
                        end: end as u8,
                        surface,
                        boost,
                    });
                }
            }
        }
        WordLattice { len, edges }
    }

    fn dict_lookup(&self, syllables: &[Syllable]) -> Vec<(Surface, i32)> {
        let mut words = vec![];
        // TODO: load this as part of static_dict?
        let rare_words = self.rare_dict.lookup(syllables, LookupStrategy::Standard);
        words.extend(
            self.user_dict
                .lookup(syllables, LookupStrategy::Standard)
                .iter()
                .map(|&(w, b)| (Surface::Word(w), b)),
        );
        words.extend(
            self.history_dict
                .lookup(syllables, LookupStrategy::Standard)
                .iter()
                .map(|&(w, b)| (Surface::Word(w), b)),
        );
        words.extend(
            self.static_dict
                .lookup(syllables, LookupStrategy::Standard)
                .iter()
                .map(|&w| {
                    if rare_words.contains(&w) {
                        (Surface::Word(w), -100000)
                    } else {
                        (Surface::Word(w), 0)
                    }
                }),
        );
        words
    }

    fn find_words(
        &self,
        start: usize,
        symbols: &[Symbol],
        com: &Composition,
    ) -> Vec<(Surface, i32)> {
        if symbols.len() == 1
            && let Some(sym) = symbols[0].to_char()
        {
            return vec![(Surface::Char(sym), 0)];
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
                return vec![(Surface::Word(selection.wid), 0)];
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
