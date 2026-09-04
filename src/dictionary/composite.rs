use std::sync::Arc;

use crate::{
    dictionary::LookupStrategy,
    lm::StaticDict,
    model::{WordId, WordOrig},
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

    pub fn lookup(&self, syllables: &[Syllable], strategy: LookupStrategy) -> Vec<WordId> {
        let mut res = vec![];
        res.extend(
            self.inner
                .static_dict
                .lookup(syllables, strategy)
                .into_iter()
                .map(|w| w),
        );
        res.extend(
            self.inner
                .history_dict
                .lookup(syllables, strategy)
                .iter()
                .filter_map(|(w, _)| {
                    if matches!(w.orig(), WordOrig::User) {
                        Some(w)
                    } else {
                        None
                    }
                }),
        );
        res.extend(
            self.inner
                .user_dict
                .lookup(syllables, strategy)
                .iter()
                .filter_map(|(w, _)| {
                    if matches!(w.orig(), WordOrig::User) {
                        Some(w)
                    } else {
                        None
                    }
                }),
        );
        res
    }
}
