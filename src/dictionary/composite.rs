use std::sync::Arc;

use smol_str::SmolStr;

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
    history_dict: HistoryDict,
    user_dict: UserDict,
}

impl CompositeDict {
    pub fn new(
        static_dict: StaticDict,
        history_dict: HistoryDict,
        user_dict: UserDict,
    ) -> CompositeDict {
        CompositeDict {
            inner: Arc::new(CompositeDictInner {
                static_dict,
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
}
