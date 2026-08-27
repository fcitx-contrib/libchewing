use std::sync::Arc;

use smol_str::SmolStr;

use crate::{
    dictionary::{LookupStrategy, StringTable},
    lm::StaticDict,
    model::WordId,
    user::{HistoryDict, UserDict},
    zhuyin::Syllable,
};

#[derive(Debug)]
pub struct CompositeDict {
    inner: Arc<CompositeDictInner>,
}

#[derive(Debug)]
struct CompositeDictInner {
    static_dict: StaticDict,
    static_words: StringTable,
    history_dict: HistoryDict,
    user_dict: UserDict,
}

impl CompositeDict {
    pub fn new(
        static_dict: StaticDict,
        static_words: StringTable,
        history_dict: HistoryDict,
        user_dict: UserDict,
    ) -> CompositeDict {
        CompositeDict {
            inner: Arc::new(CompositeDictInner {
                static_dict,
                static_words,
                history_dict,
                user_dict,
            }),
        }
    }

    pub fn lookup(&self, syllables: &[Syllable], strategy: LookupStrategy) -> Vec<(WordId, i32)> {
        let mut res = vec![];
        res.extend(
            self.inner
                .static_dict
                .lookup(syllables, strategy)
                .into_iter()
                .map(|w| (w, 0)),
        );
        res.extend(self.inner.history_dict.lookup(syllables, strategy));
        res.extend(self.inner.user_dict.lookup(syllables, strategy));
        res
    }

    pub fn get_text(&self, wid: WordId) -> Option<SmolStr> {
        self.inner
            .static_words
            .get(wid)
            .map(|s| s.into())
            .or_else(|| self.inner.history_dict.get_text(wid))
            .or_else(|| self.inner.user_dict.get_text(wid))
    }
}
