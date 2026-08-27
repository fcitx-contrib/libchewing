//! Indexed in-memory vocabulary list
//!
//! Shared implementation for LearnedDict and UserDict.

use std::collections::BTreeMap;

use smol_str::SmolStr;
use tinyvec::{TinyVec, tiny_vec};

use crate::{
    dictionary::LookupStrategy,
    model::WordId,
    zhuyin::{Syllable, SyllableVec},
};

/// Shared implementation for LearnedDict and UserDict
#[derive(Debug, Clone)]
pub(crate) struct IndexedDict {
    next_wid: WordId,
    widx: BTreeMap<WordId, SmolStr>,
    sidx: BTreeMap<SyllableVec, TinyVec<[(WordId, i32); 3]>>,
}

impl IndexedDict {
    pub(crate) fn new(base: WordId) -> IndexedDict {
        IndexedDict {
            next_wid: base,
            widx: BTreeMap::default(),
            sidx: BTreeMap::default(),
        }
    }
    pub(crate) fn insert(&mut self, syllables: SyllableVec, word: SmolStr, boost: i32) {
        self.widx.insert(self.next_wid, word);
        let wb = (self.next_wid, boost);
        self.sidx
            .entry(syllables)
            .and_modify(|v| v.push(wb))
            .or_insert(tiny_vec!([(WordId, i32); 3] => wb));
        self.next_wid.inc();
    }
    /// Gets the text of a WordId
    pub(crate) fn get_text(&self, wid: WordId) -> Option<SmolStr> {
        self.widx.get(&wid).cloned()
    }
    /// Gets the WordId from (syllables, word)
    pub(crate) fn get_wid(&self, syllables: &[Syllable], word: &str) -> Option<(WordId, i32)> {
        let wids = self.sidx.get(syllables)?;
        wids.iter()
            .find(|wb| self.widx.get(&wb.0).is_some_and(|w| w == word))
            .copied()
    }
    pub(crate) fn lookup(
        &self,
        syllables: &[Syllable],
        _strategy: LookupStrategy,
    ) -> TinyVec<[(WordId, i32); 3]> {
        self.sidx.get(syllables).cloned().unwrap_or_default()
    }
    /// Returns an iterator for all (syllable, word) pairs
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&SyllableVec, &SmolStr)> {
        self.sidx.iter().flat_map(|(syllables, words)| {
            std::iter::repeat(syllables).zip(words.iter().filter_map(|wb| self.widx.get(&wb.0)))
        })
    }
    /// Returns the number of (syllable, word_id) pairs in the dictionary
    pub(crate) fn len(&self) -> usize {
        self.sidx.values().map(|v| v.len()).sum()
    }
}
