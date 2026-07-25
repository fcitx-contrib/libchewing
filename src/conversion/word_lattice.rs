//! Builds word lattice

use log::trace;

use crate::{
    conversion::{Composition, Gap, Symbol},
    dictionary::LookupStrategy,
    model::Seg,
    sys::StaticDict,
    user::{HistoryDict, UserDict},
    zhuyin::Syllable,
};

pub(crate) struct WordLatticeBuilder {
    static_dict: StaticDict,
    user_dict: UserDict,
    history_dict: HistoryDict,
}

pub(crate) struct WordLattice {
    pub(crate) len: usize,
    pub(crate) edges: Vec<Vec<Edge>>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Edge {
    pub(crate) start: u8,
    pub(crate) end: u8,
    pub(crate) seg: Seg,
}

impl WordLatticeBuilder {
    // Assume no words in the dictionary are longer than MAX_PHRASE_LEN syllables.
    const MAX_PHRASE_LEN: usize = 15;

    pub(crate) fn to_lattice(&self, com: &Composition) -> WordLattice {
        let len = com.len();
        let mut edges = vec![vec![]; len];
        for start in 0..com.symbols.len() {
            let max_end = usize::min(start + Self::MAX_PHRASE_LEN, com.symbols.len());
            for end in (start + 1)..=max_end {
                for word in self.find_words(start, &com.symbols[start..end], com) {
                    edges[start].push(Edge {
                        start: start as u8,
                        end: end as u8,
                        seg: word,
                    });
                }
            }
        }
        WordLattice { len, edges }
    }

    fn dict_lookup(&self, syllables: &[Syllable]) -> Vec<Seg> {
        let mut words = vec![];
        words.extend(
            self.user_dict
                .lookup(syllables, LookupStrategy::Standard)
                .iter()
                .map(|w| Seg::Word(*w)),
        );
        words.extend(
            self.history_dict
                .lookup(syllables, LookupStrategy::Standard)
                .iter()
                .map(|w| Seg::Word(*w)),
        );
        words.extend(
            self.static_dict
                .lookup(syllables, LookupStrategy::Standard)
                .iter()
                .map(|w| Seg::Word(*w)),
        );
        words
    }

    fn find_words(&self, start: usize, symbols: &[Symbol], com: &Composition) -> Vec<Seg> {
        if symbols.len() == 1
            && let Some(sym) = symbols[0].to_char()
        {
            return vec![Seg::Char(sym)];
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
            if selection.intersect_range(start, end) && !selection.is_contained_by(start, end) {
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
