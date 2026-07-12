//! Indexed in-memory vocabulary list
//!
//! Shared implementation for LearnedDict and UserDict.

use std::collections::BTreeMap;

use smol_str::SmolStr;
use tinyvec::{TinyVec, tiny_vec};

use crate::{
    model::WordId,
    zhuyin::{Syllable, SyllableVec},
};

/// Shared implementation for LearnedDict and UserDict
#[derive(Debug, Clone)]
pub(crate) struct IndexedDict {
    next_wid: WordId,
    widx: BTreeMap<WordId, SmolStr>,
    sidx: BTreeMap<SyllableVec, TinyVec<[WordId; 3]>>,
}

impl IndexedDict {
    pub(crate) fn new(base: WordId) -> IndexedDict {
        IndexedDict {
            next_wid: base,
            widx: BTreeMap::default(),
            sidx: BTreeMap::default(),
        }
    }
    pub(crate) fn insert(&mut self, syllables: SyllableVec, word: SmolStr) {
        self.widx.insert(self.next_wid, word);
        self.sidx
            .entry(syllables)
            .and_modify(|v| v.push(self.next_wid))
            .or_insert(tiny_vec!([WordId; 3] => self.next_wid));
        self.next_wid.inc();
    }
    /// Gets the text of a WordId
    pub(crate) fn get_text(&self, wid: WordId) -> Option<SmolStr> {
        self.widx.get(&wid).cloned()
    }
    /// Gets the WordId from (syllables, word)
    pub(crate) fn get_wid(&self, syllables: &[Syllable], word: &str) -> Option<WordId> {
        let wids = self.sidx.get(syllables)?;
        wids.iter()
            .find(|wid| self.widx.get(wid).is_some_and(|w| w == word))
            .copied()
    }
    /// Returns an iterator for all (syllable, word) pairs
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&SyllableVec, &SmolStr)> {
        self.sidx.iter().flat_map(|(syllables, words)| {
            std::iter::repeat(syllables).zip(words.iter().filter_map(|wid| self.widx.get(wid)))
        })
    }
    /// Returns the number of (syllable, word_id) pairs in the dictionary
    pub(crate) fn len(&self) -> usize {
        self.sidx.values().map(|v| v.len()).sum()
    }
}
