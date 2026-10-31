//! Automatic learned user vocabulary list
//!
//! The auto user vocabulary list stores new words learned from user interactions

use std::{
    collections::{BTreeMap, btree_map::Entry},
    fs::File,
    io::{BufRead, BufReader, Write},
    ops::Bound::{Excluded, Included},
    path::Path,
    sync::{Arc, RwLock},
};

use bstr::ByteSlice;
use scoped_error::{bail, expect_error, impl_context_error};

use crate::{
    bare::{BareDecoder, BareEncoder},
    dictionary::{LookupStrategy, StringTable},
    model::WordId,
    zhuyin::{Syllable, SyllableVec},
};

/// Automatic learned user vocabulary list
#[derive(Debug, Clone)]
pub struct HistoryDict {
    inner: Arc<RwLock<HistoryDictInner>>,
}

#[derive(Debug)]
struct HistoryDictInner {
    string_table: StringTable,
    half_life: u32,
    generation: u64,
    unigrams: BTreeMap<SyllableVec, Vec<UniEntry>>,
    bigrams: BTreeMap<(WordId, WordId), BiEntry>,
    unigram_total: f64,
    unigram_total_last_gen: u64,
    /// prev_wid -> (total, last_gen)
    bigram_totals: BTreeMap<WordId, (f64, u64)>,
}

#[derive(Debug)]
struct UniEntry {
    wid: WordId,
    count: f32,
    last_gen: u64,
}

#[derive(Debug)]
struct BiEntry {
    count: f32,
    last_gen: u64,
}

impl HistoryDict {
    pub const HALF_LIFE: u32 = 50_000;
    pub const PRUNE_EPS: f32 = 0.25;
    pub const UNIGRAM_THRESHOLD: f64 = 100_000.0;
    pub const BIGRAM_THRESHOLD: f64 = 100_000.0;

    /// Returns an empty HistoryDict
    pub fn new(string_table: StringTable) -> HistoryDict {
        HistoryDict {
            inner: Arc::new(RwLock::new(HistoryDictInner {
                string_table,
                half_life: Self::HALF_LIFE,
                generation: 0,
                unigrams: BTreeMap::new(),
                bigrams: BTreeMap::new(),
                unigram_total: 0.0,
                unigram_total_last_gen: 0,
                bigram_totals: BTreeMap::new(),
            })),
        }
    }
    /// Initialize an empty HistoryDict on the filesystem.
    ///
    /// If a file already exists then it will be truncated.
    pub fn init<P: AsRef<Path>>(path: P) -> Result<(), HistoryDictError> {
        expect_error("Failed to initialize HistoryDict", || {
            let dict = Self::new(StringTable::new());
            let file = File::create(path)?;
            dict.to_writer(file)?;
            Ok(())
        })
    }
    /// Open an HistoryDict file and read from it.
    pub fn open<P: AsRef<Path>>(
        path: P,
        string_table: StringTable,
    ) -> Result<HistoryDict, HistoryDictError> {
        expect_error("Failed to open user history dictionary", || {
            let file = File::open(path)?;
            let reader = BufReader::new(file);
            Ok(HistoryDict::from_reader(reader, string_table)?)
        })
    }
    /// Reads history dict from the IO stream
    pub fn from_reader<R: BufRead>(
        reader: R,
        string_table: StringTable,
    ) -> Result<HistoryDict, HistoryDictError> {
        expect_error("Failed to parse history dict", || {
            let mut decoder = BareDecoder::new(reader);

            // Read file magic
            let magic = decoder.read_data_exact(4)?;
            if magic != b"CHHD" {
                bail!("Invalid file header");
            }
            // Read HistoryDictFile union version
            let version = decoder.read_uint()?;
            if version != 0 {
                bail!("Incompatible file format: {version}, expected: 0");
            }
            let half_life = decoder.read_u32()?;
            let generation = decoder.read_u64()?;
            let unigram_total = decoder.read_f64()?;
            let unigram_total_last_gen = decoder.read_u64()?;

            let num_extensions = decoder.read_uint()?;
            // Ignore all unknown extensions
            for _ in 0..num_extensions {
                let _tag = decoder.read_u32()?;
                let _data = decoder.skip_data()?;
            }

            let word_count = decoder.read_uint()? as u32;
            let mut file_wid_map = BTreeMap::new();
            for file_wid in 0..word_count {
                let raw_word = decoder.read_data()?;
                let word = str::from_utf8(&raw_word)?;
                let wid = string_table.intern(word);
                file_wid_map.insert(file_wid, wid);
            }

            let uni_count = decoder.read_uint()?;
            let mut unigrams = BTreeMap::new();
            for _ in 0..uni_count {
                let len = decoder.read_uint()? as usize;
                if len > SyllableVec::MAX_LEN {
                    bail!(
                        "Syllable length too long. Max is {} but read {}",
                        SyllableVec::MAX_LEN,
                        len
                    );
                }
                let mut syllables = SyllableVec::new();
                for _ in 0..len {
                    let syl = Syllable::try_from(decoder.read_u16()?)?;
                    syllables.push(syl)?;
                }
                let file_wid = decoder.read_u32()?;
                let count = decoder.read_f32()?;
                let last_gen = decoder.read_u64()?;
                if let Some(&wid) = file_wid_map.get(&file_wid) {
                    unigrams.entry(syllables).or_insert(vec![]).push(UniEntry {
                        wid,
                        count,
                        last_gen,
                    });
                };
            }

            let bi_count = decoder.read_uint()?;
            let mut bigrams = BTreeMap::new();
            for _ in 0..bi_count {
                let file_prev_wid = decoder.read_u32()?;
                let file_wid = decoder.read_u32()?;
                let count = decoder.read_f32()?;
                let last_gen = decoder.read_u64()?;
                if let (Some(&prev_wid), Some(&wid)) = (
                    file_wid_map.get(&file_prev_wid),
                    file_wid_map.get(&file_wid),
                ) {
                    bigrams.insert((prev_wid, wid), BiEntry { count, last_gen });
                };
            }

            let mut bigram_totals = BTreeMap::new();
            for ((prev_wid, _), e) in &bigrams {
                let c_eff = effective_count_f64(e.count as f64, generation, e.last_gen, half_life);
                let (t, lg) = bigram_totals.entry(*prev_wid).or_insert((0.0, 0));
                *t += c_eff;
                *lg = (*lg).max(e.last_gen);
            }

            Ok(HistoryDict {
                inner: Arc::new(RwLock::new(HistoryDictInner {
                    string_table,
                    half_life,
                    generation,
                    unigrams,
                    bigrams,
                    unigram_total,
                    unigram_total_last_gen,
                    bigram_totals,
                })),
            })
        })
    }
    pub fn to_writer<W: Write>(&self, writer: W) -> Result<(), HistoryDictError> {
        expect_error("Unable to serialize the history dictionary", || {
            let lock = self
                .inner
                .read()
                .expect("Unable to acquire HistoryDict reader lock");
            let mut encoder = BareEncoder::new(writer);

            // Write file magic
            encoder.write_data_exact(b"CHHD")?;
            // Write HistoryDictFile union version
            encoder.write_uint(0)?;
            encoder.write_u32(lock.half_life)?;
            encoder.write_u64(lock.generation)?;
            encoder.write_f64(lock.unigram_total)?;
            encoder.write_u64(lock.unigram_total_last_gen)?;
            // Write 0 extensions
            encoder.write_uint(0)?;

            let g = lock.generation;
            let h = lock.half_life;

            // Prepare words section and file_wid mapping
            let mut wid_file_map = BTreeMap::new();
            let mut words = vec![];
            let mut uni_count = 0;
            for (_, entries) in &lock.unigrams {
                for entry in entries {
                    if effective_count(entry.count, g, entry.last_gen, h) < Self::PRUNE_EPS {
                        continue;
                    }
                    uni_count += 1;
                    let e = wid_file_map.entry(entry.wid);
                    if matches!(e, Entry::Vacant(_)) {
                        if let Some(word) = lock.string_table.get_text(entry.wid) {
                            e.insert_entry(words.len() as u32);
                            words.push(word);
                        };
                    }
                }
            }
            let mut bi_count = 0;
            for ((prev_wid, wid), entry) in &lock.bigrams {
                if effective_count(entry.count, g, entry.last_gen, h) < Self::PRUNE_EPS {
                    continue;
                }
                bi_count += 1;
                let e = wid_file_map.entry(*prev_wid);
                if matches!(e, Entry::Vacant(_)) {
                    if let Some(prev_word) = lock.string_table.get_text(*prev_wid) {
                        e.insert_entry(words.len() as u32);
                        words.push(prev_word);
                    };
                }
                let e = wid_file_map.entry(*wid);
                if matches!(e, Entry::Vacant(_)) {
                    if let Some(word) = lock.string_table.get_text(*wid) {
                        e.insert_entry(words.len() as u32);
                        words.push(word);
                    };
                }
            }

            // Write words section
            encoder.write_uint(words.len() as u64)?;
            for word in words {
                encoder.write_data(word.as_bytes())?;
            }

            // Write unigrams
            encoder.write_uint(uni_count)?;
            for (syllables, entries) in &lock.unigrams {
                for entry in entries {
                    if effective_count(entry.count, g, entry.last_gen, h) < Self::PRUNE_EPS {
                        continue;
                    }
                    encoder.write_uint(syllables.len() as u64)?;
                    for syl in syllables {
                        encoder.write_u16(syl.to_u16())?;
                    }
                    let file_wid = wid_file_map.get(&entry.wid).expect("Corrupted state");
                    encoder.write_u32(*file_wid)?;
                    encoder.write_f32(entry.count)?;
                    encoder.write_u64(entry.last_gen)?;
                }
            }

            // Write bigrams
            encoder.write_uint(bi_count)?;
            for ((prev_wid, wid), entry) in &lock.bigrams {
                if effective_count(entry.count, g, entry.last_gen, h) < Self::PRUNE_EPS {
                    continue;
                }
                let file_prev_wid = wid_file_map.get(prev_wid).expect("Corrupted state");
                let file_wid = wid_file_map.get(wid).expect("Corrupted state");
                encoder.write_u32(*file_prev_wid)?;
                encoder.write_u32(*file_wid)?;
                encoder.write_f32(entry.count)?;
                encoder.write_u64(entry.last_gen)?;
            }
            Ok(())
        })
    }
    pub(crate) fn new_gen(&self) -> u64 {
        let mut lock = self
            .inner
            .write()
            .expect("Unable to acquire HistoryDict writer lock");
        lock.generation += 1;
        lock.generation
    }
    pub(crate) fn observe_unigram(&self, g: u64, syllables: &[Syllable], word: &str) {
        let mut lock = self
            .inner
            .write()
            .expect("Unable to acquire HistoryDict writer lock");
        let Ok(syl_vec) = syllables.try_into() else {
            return;
        };
        let wid = lock.string_table.intern(word);
        let h = lock.half_life;

        let uni_t = lock.unigram_total;
        let uni_lg = lock.unigram_total_last_gen;

        lock.unigram_total = effective_count_f64(uni_t, g, uni_lg, h) + 1.0;
        lock.unigram_total_last_gen = g;

        let uni_entries = lock.unigrams.entry(syl_vec).or_default();
        if let Some(pos) = uni_entries.iter().position(|e| e.wid == wid) {
            let e = &mut uni_entries[pos];
            e.count = effective_count(e.count, g, e.last_gen, h) + 1.0;
            e.last_gen = g;
        } else {
            uni_entries.push(UniEntry {
                wid,
                count: 1.0,
                last_gen: g,
            });
        }
    }
    pub(crate) fn observe_bigram(&self, g: u64, prev: &str, word: &str) {
        let mut lock = self
            .inner
            .write()
            .expect("Unable to acquire HistoryDict writer lock");
        let prev_wid = lock.string_table.intern(prev);
        let wid = lock.string_table.intern(word);
        let h = lock.half_life;

        match lock.bigrams.entry((prev_wid, wid)) {
            Entry::Occupied(mut e) => {
                let e = e.get_mut();
                e.count = effective_count(e.count, g, e.last_gen, h) + 1.0;
                e.last_gen = g;
            }
            Entry::Vacant(e) => {
                e.insert(BiEntry {
                    count: 1.0,
                    last_gen: g,
                });
            }
        }
        match lock.bigram_totals.entry(prev_wid) {
            Entry::Occupied(mut e) => {
                let (t, lg) = *e.get();
                e.insert((effective_count_f64(t, g, lg, h) + 1.0, g));
            }
            Entry::Vacant(e) => {
                e.insert((1.0, g));
            }
        }
    }

    pub fn remove(&self, syllables: &[Syllable], word: &str) {
        let mut lock = self
            .inner
            .write()
            .expect("Unable to acquire UserDict writer lock");
        let Ok(syl_vec) = syllables.try_into() else {
            return;
        };
        let wid = lock.string_table.intern(word);
        let uni_entries = lock.unigrams.entry(syl_vec).or_default();
        if let Some(pos) = uni_entries.iter().position(|e| e.wid == wid) {
            uni_entries.swap_remove(pos);
        }
        lock.bigrams
            .retain(|(prev, cur), _| *prev != wid && *cur != wid);
        lock.bigram_totals.remove(&wid);
    }
    // Return all words and their history based unigram log10 prob
    pub(crate) fn unigram(
        &self,
        syllables: &[Syllable],
        strategy: LookupStrategy,
    ) -> Vec<(WordId, f64)> {
        let lock = self
            .inner
            .read()
            .expect("Unable to acquire HistoryDict reader lock");
        let h = lock.half_life;
        let uni_t = lock.unigram_total;
        let g = lock.generation;
        let uni_lg = lock.unigram_total_last_gen;

        let total_now = effective_count_f64(uni_t, g, uni_lg, h);
        let denominator = total_now + Self::UNIGRAM_THRESHOLD;

        match strategy {
            LookupStrategy::Standard => lock
                .unigrams
                .get(syllables)
                .map(|entries| {
                    entries
                        .iter()
                        .map(|e| {
                            let wid = e.wid;
                            let count = effective_count(e.count, g, e.last_gen, h);
                            let logprob10 = (count as f64 / denominator).log10();
                            (wid, logprob10)
                        })
                        .collect()
                })
                .unwrap_or_default(),
            LookupStrategy::FuzzyPartialPrefix => {
                let mut end = syllables.to_vec();
                // NB: relies on the syllable encoding to
                // ensure Syllable::EMPTY is greater than all real syllables.
                end.push(Syllable::new());
                lock.unigrams
                    .range::<[Syllable], _>((Included(syllables), Excluded(end.as_slice())))
                    .flat_map(|(_, entries)| {
                        entries
                            .iter()
                            .filter(|e| {
                                lock.string_table.get_text(e.wid).is_some_and(|s| {
                                    s.as_bytes().graphemes().count() == syllables.len()
                                })
                            })
                            .map(|e| {
                                let wid = e.wid;
                                let count = effective_count(e.count, g, e.last_gen, h);
                                let logprob10 = (count as f64 / denominator).log10();
                                (wid, logprob10)
                            })
                    })
                    .collect()
            }
        }
    }
    /// Returns bigram probability in log10 space.
    pub(crate) fn bigram(&self, prev_wid: WordId, wid: WordId) -> f64 {
        let lock = self
            .inner
            .read()
            .expect("Unable to acquire HistoryDict reader lock");
        let Some(e) = lock.bigrams.get(&(prev_wid, wid)) else {
            return f64::NEG_INFINITY;
        };
        let Some(&(t, t_lg)) = lock.bigram_totals.get(&prev_wid) else {
            return f64::NEG_INFINITY;
        };
        let c_eff = effective_count(e.count, lock.generation, e.last_gen, lock.half_life) as f64;
        let t_eff = effective_count_f64(t, lock.generation, t_lg, lock.half_life);
        (c_eff / (t_eff + Self::BIGRAM_THRESHOLD)).log10()
    }
}

/// Calculates the effective count using a decay function
///
/// # Parameters
/// - count: number observed at last generation *lg*
/// - g: current generation
/// - lg: last seen generation
/// - h: half-life
fn effective_count(count: f32, g: u64, lg: u64, h: u32) -> f32 {
    let dt = (g - lg) as f32 / h as f32;
    count * (-dt).exp2()
}

fn effective_count_f64(count: f64, g: u64, lg: u64, h: u32) -> f64 {
    let dt = (g - lg) as f64 / h as f64;
    count * (-dt).exp2()
}

impl_context_error!(pub HistoryDictError);

#[cfg(test)]
mod tests {
    use crate::{syl, zhuyin::Bopomofo};

    use super::*;

    #[test]
    fn unigram_learning_curve() {
        let st = StringTable::new();
        let hist = HistoryDict::new(st);
        let test = &[
            syl![Bopomofo::C, Bopomofo::TONE4],
            syl![Bopomofo::SH, Bopomofo::TONE4],
        ];

        assert_eq!(hist.unigram(test, LookupStrategy::Standard), vec![]);

        hist.observe_unigram(hist.new_gen(), test, "測試");
        let log10prob = hist.unigram(test, LookupStrategy::Standard)[0].1;
        assert!(
            (log10prob - -5.00).abs() < 1e-2,
            "log10prob = {}",
            log10prob
        );

        hist.observe_unigram(hist.new_gen(), test, "測試");
        let log10prob = hist.unigram(test, LookupStrategy::Standard)[0].1;
        assert!(
            (log10prob - -4.69).abs() < 1e-2,
            "log10prob = {}",
            log10prob
        );

        hist.observe_unigram(hist.new_gen(), test, "測試");
        let log10prob = hist.unigram(test, LookupStrategy::Standard)[0].1;
        assert!(
            (log10prob - -4.52).abs() < 1e-2,
            "log10prob = {}",
            log10prob
        );

        for _ in 0..1000 {
            hist.observe_unigram(hist.new_gen(), test, "測試");
        }
        let log10prob = hist.unigram(test, LookupStrategy::Standard)[0].1;
        assert!(
            (log10prob - -2.00).abs() < 1e-2,
            "log10prob = {}",
            log10prob
        );
    }

    #[test]
    fn unigram_decay() {
        let st = StringTable::new();
        let hist = HistoryDict::new(st);
        let test = &[
            syl![Bopomofo::C, Bopomofo::TONE4],
            syl![Bopomofo::SH, Bopomofo::TONE4],
        ];
        let remove = &[
            syl![Bopomofo::SH, Bopomofo::AN],
            syl![Bopomofo::CH, Bopomofo::U, Bopomofo::TONE2],
        ];
        hist.observe_unigram(hist.new_gen(), test, "測試");
        let log10prob = hist.unigram(test, LookupStrategy::Standard)[0].1;
        assert!(
            (log10prob - -5.00).abs() < 1e-2,
            "log10prob = {}",
            log10prob
        );
        for _ in 0..50_000 {
            hist.observe_unigram(hist.new_gen(), remove, "刪除");
        }
        let log10prob = hist.unigram(test, LookupStrategy::Standard)[0].1;
        assert!(
            (log10prob - -5.43).abs() < 1e-2,
            "log10prob = {}",
            log10prob
        );
    }

    #[test]
    fn bigram_learning_curve() {
        let st = StringTable::new();
        let test = st.intern("測試");
        let program = st.intern("程式");

        let hist = HistoryDict::new(st);

        assert!(hist.bigram(test, program).is_infinite());

        hist.observe_bigram(hist.new_gen(), "測試", "程式");
        let log10prob = hist.bigram(test, program);
        assert!(
            (log10prob - -5.00).abs() < 1e-2,
            "log10prob = {}",
            log10prob
        );

        hist.observe_bigram(hist.new_gen(), "測試", "程式");
        let log10prob = hist.bigram(test, program);
        assert!(
            (log10prob - -4.69).abs() < 1e-2,
            "log10prob = {}",
            log10prob
        );

        hist.observe_bigram(hist.new_gen(), "測試", "程式");
        let log10prob = hist.bigram(test, program);
        assert!(
            (log10prob - -4.52).abs() < 1e-2,
            "log10prob = {}",
            log10prob
        );

        for _ in 0..1000 {
            hist.observe_bigram(hist.new_gen(), "測試", "程式");
        }
        let log10prob = hist.bigram(test, program);
        assert!(
            (log10prob - -2.00).abs() < 1e-2,
            "log10prob = {}",
            log10prob
        );
    }

    #[test]
    fn bigram_decay() {
        let st = StringTable::new();
        let test = st.intern("測試");
        let program = st.intern("程式");

        let hist = HistoryDict::new(st);

        assert!(hist.bigram(test, program).is_infinite());

        hist.observe_bigram(hist.new_gen(), "測試", "程式");
        let log10prob = hist.bigram(test, program);
        assert!(
            (log10prob - -5.00).abs() < 1e-2,
            "log10prob = {}",
            log10prob
        );

        hist.observe_bigram(hist.new_gen(), "測試", "程式");
        let log10prob = hist.bigram(test, program);
        assert!(
            (log10prob - -4.69).abs() < 1e-2,
            "log10prob = {}",
            log10prob
        );

        hist.observe_bigram(hist.new_gen(), "測試", "程式");
        let log10prob = hist.bigram(test, program);
        assert!(
            (log10prob - -4.52).abs() < 1e-2,
            "log10prob = {}",
            log10prob
        );

        for _ in 0..1000 {
            hist.observe_bigram(hist.new_gen(), "測試", "城市");
        }
        let log10prob = hist.bigram(test, program);
        assert!(
            (log10prob - -4.53).abs() < 1e-2,
            "log10prob = {}",
            log10prob
        );
    }

    #[test]
    fn save_restore() {
        let st = StringTable::new();
        let test_wid = st.intern("測試");
        let program_wid = st.intern("程式");

        let test = &[
            syl![Bopomofo::C, Bopomofo::TONE4],
            syl![Bopomofo::SH, Bopomofo::TONE4],
        ];

        let hist = HistoryDict::new(st.clone());

        assert_eq!(hist.unigram(test, LookupStrategy::Standard), vec![]);

        hist.observe_unigram(hist.new_gen(), test, "測試");
        let log10prob = hist.unigram(test, LookupStrategy::Standard)[0].1;
        assert!(
            (log10prob - -5.00).abs() < 1e-2,
            "log10prob = {}",
            log10prob
        );

        assert!(hist.bigram(test_wid, program_wid).is_infinite());

        hist.observe_bigram(hist.new_gen(), "測試", "程式");
        let log10prob = hist.bigram(test_wid, program_wid);
        assert!(
            (log10prob - -5.00).abs() < 1e-2,
            "log10prob = {}",
            log10prob
        );

        let mut buffer = vec![];
        hist.to_writer(&mut buffer).unwrap();

        let restored = HistoryDict::from_reader(buffer.as_slice(), st).unwrap();
        let log10prob = restored.unigram(test, LookupStrategy::Standard)[0].1;
        assert!(
            (log10prob - -5.00).abs() < 1e-2,
            "log10prob = {}",
            log10prob
        );
        let log10prob = restored.bigram(test_wid, program_wid);
        assert!(
            (log10prob - -5.00).abs() < 1e-2,
            "log10prob = {}",
            log10prob
        );

        let mut buffer2 = vec![];
        restored.to_writer(&mut buffer2).unwrap();

        assert_eq!(buffer, buffer2, "history should serialize bit identical");
    }

    #[test]
    fn remove() {
        let st = StringTable::new();
        let test_wid = st.intern("測試");
        let program_wid = st.intern("程式");

        let test = &[
            syl![Bopomofo::C, Bopomofo::TONE4],
            syl![Bopomofo::SH, Bopomofo::TONE4],
        ];

        let hist = HistoryDict::new(st.clone());

        assert_eq!(hist.unigram(test, LookupStrategy::Standard), vec![]);

        hist.observe_unigram(hist.new_gen(), test, "測試");
        let log10prob = hist.unigram(test, LookupStrategy::Standard)[0].1;
        assert!(
            (log10prob - -5.00).abs() < 1e-2,
            "log10prob = {}",
            log10prob
        );

        assert!(hist.bigram(test_wid, program_wid).is_infinite());

        hist.observe_bigram(hist.new_gen(), "測試", "程式");
        let log10prob = hist.bigram(test_wid, program_wid);
        assert!(
            (log10prob - -5.00).abs() < 1e-2,
            "log10prob = {}",
            log10prob
        );

        hist.remove(test, "測試");

        assert!(hist.unigram(test, LookupStrategy::Standard).is_empty());
        assert!(hist.bigram(test_wid, program_wid).is_infinite());
    }
}
