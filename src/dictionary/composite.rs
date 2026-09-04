use std::sync::Arc;

use crate::{
    dictionary::LookupStrategy,
    lm::StaticDict,
    model::WordId,
    user::{HistoryDict, UserDict},
    zhuyin::Syllable,
};

#[derive(Clone, Debug)]
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
        // base value
        let mut res: Vec<_> = self
            .inner
            .static_dict
            .lookup(syllables, strategy)
            .into_iter()
            .map(|w| (w, 0))
            .collect();
        // rare boost
        for wid in self.inner.rare_dict.lookup(syllables, strategy) {
            const RARE_BOOST: i32 = -100000;
            if let Some(pos) = res.iter().position(|w| w.0 == wid) {
                res[pos].1 = RARE_BOOST;
            } else {
                res.push((wid, RARE_BOOST));
            }
        }
        // history boost
        for (wid, boost) in self.inner.history_dict.lookup(syllables, strategy) {
            if let Some(pos) = res.iter().position(|w| w.0 == wid) {
                res[pos].1 = boost;
            } else {
                res.push((wid, boost));
            }
        }
        // user boost
        for (wid, boost) in self.inner.user_dict.lookup(syllables, strategy) {
            if let Some(pos) = res.iter().position(|w| w.0 == wid) {
                res[pos].1 = boost;
            } else {
                res.push((wid, boost));
            }
        }
        res
    }
}
