use std::{collections::BTreeMap, sync::Arc};

use crate::{
    dictionary::LookupStrategy,
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
    rare_dict: StaticDict,
    history_dict: HistoryDict,
    user_dict: UserDict,
}

impl CompositeDict {
    pub fn new(
        static_dict: StaticDict,
        rare_dict: StaticDict,
        history_dict: HistoryDict,
        user_dict: UserDict,
    ) -> CompositeDict {
        CompositeDict {
            inner: Arc::new(CompositeDictInner {
                static_dict,
                rare_dict,
                history_dict,
                user_dict,
            }),
        }
    }

    pub fn lookup(&self, syllables: &[Syllable], strategy: LookupStrategy) -> Vec<(WordId, i32)> {
        let mut res = BTreeMap::new();
        // base value
        for wid in self.inner.static_dict.lookup(syllables, strategy) {
            res.insert(wid, 0);
        }
        for wid in self.inner.rare_dict.lookup(syllables, strategy) {
            res.insert(wid, -100000);
        }
        // rare boost
        // history boost
        for (wid, boost) in self.inner.history_dict.lookup(syllables, strategy) {
            res.insert(wid, boost);
        }
        // user boost
        for (wid, boost) in self.inner.user_dict.lookup(syllables, strategy) {
            res.insert(wid, boost);
        }
        res.into_iter().collect()
    }
}
