use std::{sync::Arc, time::Instant};

use crate::{
    dictionary::LookupStrategy,
    lm::StaticDict,
    model::{Candidate, Span, WordId},
    user::{HistoryDict, SessionHistoryDict, UserDict},
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
    session_dict: SessionHistoryDict,
    user_dict: UserDict,
}

#[derive(Debug, Clone, Copy)]
pub struct CompositeWord {
    pub wid: WordId,
    pub hist_prob: f64,
    pub user_pref: Option<i8>,
}

impl CompositeWord {
    pub(crate) fn to_candidate(self, span: Span) -> Candidate {
        Candidate::Word {
            wid: self.wid,
            hist_prob: self.hist_prob,
            user_pref: self.user_pref,
            span,
        }
    }
}

impl CompositeDict {
    pub fn new(
        static_dict: StaticDict,
        rare_dict: StaticDict,
        history_dict: HistoryDict,
        session_dict: SessionHistoryDict,
        user_dict: UserDict,
    ) -> CompositeDict {
        CompositeDict {
            inner: Arc::new(CompositeDictInner {
                static_dict,
                rare_dict,
                history_dict,
                session_dict,
                user_dict,
            }),
        }
    }

    pub fn lookup(&self, syllables: &[Syllable], strategy: LookupStrategy) -> Vec<CompositeWord> {
        // base value
        let mut res: Vec<_> = self
            .inner
            .static_dict
            .lookup(syllables, strategy)
            .into_iter()
            .map(|wid| (wid, f64::NEG_INFINITY, None))
            .collect();
        // rare boost
        for wid in self.inner.rare_dict.lookup(syllables, strategy) {
            const RARE_BOOST: i8 = UserDict::MIN + 1;
            if let Some(pos) = res.iter().position(|cand| cand.0 == wid) {
                res[pos].2 = Some(RARE_BOOST);
            } else {
                res.push((wid, f64::NEG_INFINITY, Some(RARE_BOOST)));
            }
        }
        // history boost
        for (wid, hist_prob) in self.inner.history_dict.unigram(syllables, strategy) {
            if let Some(pos) = res.iter().position(|cand| cand.0 == wid) {
                res[pos].1 = hist_prob;
            } else {
                res.push((wid, hist_prob, None));
            }
        }
        let now = Instant::now();
        for (wid, bonus) in self.inner.session_dict.lookup(now, syllables, strategy) {
            if let Some(pos) = res.iter().position(|cand| cand.0 == wid) {
                res[pos].1 += bonus;
            } else {
                res.push((wid, bonus, None));
            }
        }
        // user boost
        for (wid, user_pref) in self.inner.user_dict.lookup(syllables, strategy) {
            if let Some(pos) = res.iter().position(|w| w.0 == wid) {
                res[pos].2 = Some(user_pref);
            } else {
                res.push((wid, f64::NEG_INFINITY, Some(user_pref)));
            }
        }
        let banned = self.inner.user_dict.lookup_banned(syllables, strategy);
        res.into_iter()
            .filter(|cand| !banned.contains(&cand.0))
            .map(|cand| CompositeWord {
                wid: cand.0,
                hist_prob: cand.1,
                user_pref: cand.2,
            })
            .collect()
    }
}
