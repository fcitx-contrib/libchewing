//! Session-scoped recency layer
//!
//! Remembers, per syllable key, the words the user chose within a rolling
//! time window, so that the same override is effective immediately for
//! the rest of the session.

use std::{
    collections::{BTreeMap, VecDeque},
    ops::Bound::{Excluded, Included},
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use crate::{
    dictionary::{LookupStrategy, StringTable},
    model::WordId,
    zhuyin::{Syllable, SyllableVec},
};

/// Session-scoped recency layer.
#[derive(Debug, Clone)]
pub struct SessionHistoryDict {
    inner: Arc<RwLock<SessionInner>>,
}

#[derive(Debug)]
struct SessionInner {
    string_table: StringTable,
    window: Duration,
    half_life: Duration,
    /// syllable key -> entries, head = most recent
    by_syllables: BTreeMap<SyllableVec, Vec<RecencyEntry>>,
    /// oldest-first event log, used for eviction
    by_time: VecDeque<Event>,
}

#[derive(Debug)]
struct RecencyEntry {
    wid: WordId,
    at: Instant,
}

#[derive(Debug)]
struct Event {
    at: Instant,
    key: SyllableVec,
    wid: WordId,
}

impl SessionHistoryDict {
    /// Rolling window during which an override stays in effect.
    pub const WINDOW: Duration = Duration::from_secs(3600);
    /// Half-life of an override's strength within the window.
    pub const HALF_LIFE: Duration = Duration::from_secs(900);
    /// Maximum log10 bonus for the most recent override.
    pub const BONUS_MAX: f64 = 3.0;
    /// Damping for single-syllable keys: the candidate set is most ambiguous
    /// there, so a recency override is most likely to be a mistake.
    pub const SINGLE_SYLLABLE_FACTOR: f64 = 0.2;
    /// Rank decay between successive entries of the same key.
    pub const RANK_DECAY: f64 = 0.5;
    /// Hard cap on distinct words remembered per syllable key.
    pub const MAX_PER_KEY: usize = 8;
    /// Safety net against pathological growth.
    pub const MAX_EVENTS: usize = 100_000;

    pub fn new(string_table: StringTable) -> SessionHistoryDict {
        SessionHistoryDict {
            inner: Arc::new(RwLock::new(SessionInner {
                string_table,
                window: Self::WINDOW,
                half_life: Self::HALF_LIFE,
                by_syllables: BTreeMap::new(),
                by_time: VecDeque::new(),
            })),
        }
    }

    /// Clear all session state.
    pub fn clear(&self) {
        let mut lock = self
            .inner
            .write()
            .expect("Unable to acquire SessionHistoryDict writer lock");
        lock.by_syllables.clear();
        lock.by_time.clear();
    }

    /// Record that the user committed `word` for `syllables`.
    pub(crate) fn observe(&self, now: Instant, syllables: &[Syllable], word: &str) {
        let mut lock = self
            .inner
            .write()
            .expect("Unable to acquire SessionHistoryDict writer lock");
        lock.evict(now);
        let wid = lock.string_table.intern(word);
        let Ok(key): Result<SyllableVec, _> = syllables.try_into() else {
            return;
        };
        let entries = lock.by_syllables.entry(key).or_default();
        if let Some(pos) = entries.iter().position(|e| e.wid == wid) {
            // Refresh
            entries.remove(pos);
        }
        entries.insert(0, RecencyEntry { wid, at: now });
        entries.truncate(Self::MAX_PER_KEY);
        lock.by_time.push_back(Event { at: now, key, wid });
    }

    /// Returns `(WordId, log10 bonus)` for every session entry under
    /// `syllables`. Add the bonus to the candidate's log10 score; a word with
    /// no session entry simply gets no bonus.
    pub(crate) fn lookup(
        &self,
        now: Instant,
        syllables: &[Syllable],
        strategy: LookupStrategy,
    ) -> Vec<(WordId, f64)> {
        let lock = self
            .inner
            .read()
            .expect("Unable to acquire SessionHistoryDict reader lock");
        let mut out = Vec::new();
        match strategy {
            LookupStrategy::Standard => {
                if let Some(entries) = lock.by_syllables.get(syllables) {
                    push_bonuses(
                        lock.half_life,
                        lock.window,
                        now,
                        entries,
                        syllables.len(),
                        &mut out,
                    );
                }
            }
            LookupStrategy::FuzzyPartialPrefix => {
                let mut end = syllables.to_vec();
                // NB: relies on Syllable::EMPTY sorting above real syllables.
                end.push(Syllable::new());
                for (key, entries) in lock
                    .by_syllables
                    .range::<[Syllable], _>((Included(syllables), Excluded(end.as_slice())))
                {
                    push_bonuses(
                        lock.half_life,
                        lock.window,
                        now,
                        entries,
                        key.len(),
                        &mut out,
                    );
                }
            }
        }
        out
    }
}

fn push_bonuses(
    half_life: Duration,
    window: Duration,
    now: Instant,
    entries: &[RecencyEntry],
    key_len: usize,
    out: &mut Vec<(WordId, f64)>,
) {
    let length_factor = if key_len >= 2 {
        1.0
    } else {
        SessionHistoryDict::SINGLE_SYLLABLE_FACTOR
    };
    let half_life = half_life.as_secs_f64();
    for (rank, e) in entries.iter().enumerate() {
        let age = now.saturating_duration_since(e.at);
        if age >= window {
            // outside the window: no bonus at all
            continue;
        }
        let recency = (-age.as_secs_f64() / half_life).exp2();
        let rank = SessionHistoryDict::RANK_DECAY.powi(rank as i32);
        out.push((
            e.wid,
            SessionHistoryDict::BONUS_MAX * length_factor * rank * recency,
        ));
    }
}

impl SessionInner {
    fn evict(&mut self, now: Instant) {
        while let Some(front) = self.by_time.front() {
            if now.saturating_duration_since(front.at) < self.window {
                break;
            }
            let Event { at, key, wid } = self.by_time.pop_front().expect("front() succeeded");
            // Guard: skip if a newer observation of the same word replaced it.
            self.remove_event(&key, |e| e.wid == wid && e.at == at);
        }
        while self.by_time.len() > SessionHistoryDict::MAX_EVENTS {
            let Event { key, wid, .. } = self.by_time.pop_front().expect("len() > 0");
            self.remove_event(&key, |e| e.wid == wid);
        }
    }

    fn remove_event(&mut self, key: &SyllableVec, pred: impl Fn(&RecencyEntry) -> bool) {
        let mut empty = false;
        if let Some(entries) = self.by_syllables.get_mut(key) {
            if let Some(pos) = entries.iter().position(|e| pred(e)) {
                entries.remove(pos);
            }
            empty = entries.is_empty();
        }
        if empty {
            self.by_syllables.remove(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{syl, zhuyin::Bopomofo};

    fn key2() -> Vec<Syllable> {
        vec![
            syl![Bopomofo::C, Bopomofo::TONE4],
            syl![Bopomofo::SH, Bopomofo::TONE4],
        ]
    }
    fn key1() -> Vec<Syllable> {
        vec![syl![Bopomofo::SH, Bopomofo::TONE4]]
    }

    #[test]
    fn head_bonus_is_max_then_halves() {
        let st = StringTable::new();
        let s = SessionHistoryDict::new(st);
        let (key, now) = (key2(), Instant::now());
        s.observe(now, &key, "城市");

        let out = s.lookup(now, &key, LookupStrategy::Standard);
        assert_eq!(out.len(), 1);
        assert!((out[0].1 - SessionHistoryDict::BONUS_MAX).abs() < 1e-9);

        let out = s.lookup(
            now + SessionHistoryDict::HALF_LIFE,
            &key,
            LookupStrategy::Standard,
        );
        assert!((out[0].1 - SessionHistoryDict::BONUS_MAX / 2.0).abs() < 1e-6);
    }

    #[test]
    fn expires_after_window() {
        let st = StringTable::new();
        let s = SessionHistoryDict::new(st);
        let (key, now) = (key2(), Instant::now());
        s.observe(now, &key, "城市");

        let just_inside = now + SessionHistoryDict::WINDOW - Duration::from_secs(1);
        assert_eq!(
            s.lookup(just_inside, &key, LookupStrategy::Standard).len(),
            1
        );

        let outside = now + SessionHistoryDict::WINDOW + Duration::from_secs(1);
        assert!(s.lookup(outside, &key, LookupStrategy::Standard).is_empty());
    }

    #[test]
    fn single_syllable_is_damped() {
        let st = StringTable::new();
        let s = SessionHistoryDict::new(st);
        let (key, now) = (key1(), Instant::now());
        s.observe(now, &key, "是");
        let out = s.lookup(now, &key, LookupStrategy::Standard);
        let expected = SessionHistoryDict::BONUS_MAX * SessionHistoryDict::SINGLE_SYLLABLE_FACTOR;
        assert!((out[0].1 - expected).abs() < 1e-9);
    }

    #[test]
    fn most_recent_override_wins() {
        let st = StringTable::new();
        let s = SessionHistoryDict::new(st.clone());
        let (key, now) = (key2(), Instant::now());
        s.observe(now, &key, "城市");
        s.observe(now + Duration::from_secs(60), &key, "程式");

        let out = s.lookup(
            now + Duration::from_secs(60),
            &key,
            LookupStrategy::Standard,
        );
        assert_eq!(out[0].0, st.intern("程式"));
    }

    #[test]
    fn fuzzy_prefix_finds_longer_keys() {
        let st = StringTable::new();
        let s = SessionHistoryDict::new(st);
        let long = vec![
            syl![Bopomofo::C, Bopomofo::TONE4],
            syl![Bopomofo::SH, Bopomofo::TONE4],
            syl![Bopomofo::J, Bopomofo::I, Bopomofo::TONE1],
        ];
        let now = Instant::now();
        s.observe(now, &long, "測試機");

        let out = s.lookup(now, &long[..2], LookupStrategy::FuzzyPartialPrefix);
        assert_eq!(out.len(), 1);
        // key_len == 3, so the full multi-syllable factor applies
        assert!((out[0].1 - SessionHistoryDict::BONUS_MAX).abs() < 1e-9);
    }

    #[test]
    fn eviction_keeps_indexes_consistent() {
        let st = StringTable::new();
        let s = SessionHistoryDict::new(st);
        let (key, now) = (key2(), Instant::now());
        for i in 0..1000 {
            s.observe(now + Duration::from_secs(i), &key, &format!("字{i}"));
        }
        // Force an eviction pass far past the window.
        let late = now + SessionHistoryDict::WINDOW * 3;
        s.observe(late, &key1(), "是");

        let lock = s.inner.read().unwrap();
        let entries: usize = lock.by_syllables.values().map(|v| v.len()).sum();
        assert_eq!(entries, 1, "only the fresh override should remain");
        assert_eq!(lock.by_time.len(), 1, "stale events should be evicted");
    }
}
