//! Automatic learned user vocabulary list
//!
//! The auto user vocabulary list stores new words learned from user interactions

use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufRead, BufReader, Write},
    ops::Bound::{Excluded, Included},
    path::Path,
    sync::{Arc, RwLock},
};

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
    records: BTreeMap<SyllableVec, Vec<HistoryDictEntry>>,
}

#[derive(Debug)]
struct HistoryDictEntry {
    wid: WordId,
    seen: u32,
    epoch: u64,
}

impl HistoryDict {
    pub const HALF_LIFE: u32 = 50_000;
    pub const COLD_THRESHOLD: f64 = 500_000.0;

    /// Returns an empty HistoryDict
    pub fn new(string_table: StringTable) -> HistoryDict {
        HistoryDict {
            inner: Arc::new(RwLock::new(HistoryDictInner {
                string_table,
                half_life: Self::HALF_LIFE,
                generation: 0,
                records: BTreeMap::new(),
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
                bail!("Unknown file version");
            }
            // Read HistoryDictHeader flags
            let flags = decoder.read_u32()?;
            if flags != 0 {
                bail!("Unknown header flags");
            }
            let half_life = decoder.read_u32()?;
            let generation = decoder.read_u64()?;
            let count = decoder.read_u32()?;
            let mut records = BTreeMap::new();

            // Read record frames
            for _ in 0..count {
                let len = decoder.read_uint()? as usize;
                let mut syllables = SyllableVec::new();
                for _ in 0..len {
                    let syl = Syllable::try_from(decoder.read_u16()?)?;
                    syllables.push(syl);
                }
                let raw_word = decoder.read_data()?;
                let word = str::from_utf8(&raw_word)?;
                let seen = decoder.read_u32()?;
                let epoch = decoder.read_u64()?;

                let wid = string_table.intern(word);

                let word_entries = records.entry(syllables).or_insert(vec![]);
                word_entries.push(HistoryDictEntry { wid, seen, epoch });
            }

            Ok(HistoryDict {
                inner: Arc::new(RwLock::new(HistoryDictInner {
                    string_table,
                    half_life,
                    generation,
                    records,
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
            // Write HistoryDictHeader flags
            encoder.write_u32(0)?;
            encoder.write_u32(lock.half_life)?;
            encoder.write_u64(lock.generation)?;
            encoder.write_u32(lock.records.len() as u32)?;

            for (syllables, entries) in lock.records.iter() {
                for entry in entries {
                    encoder.write_uint(syllables.len() as u64)?;
                    for syl in syllables {
                        encoder.write_u16(syl.to_u16())?;
                    }
                    let word = lock
                        .string_table
                        .get_text(entry.wid)
                        .expect("Should have this word");
                    encoder.write_data(word.as_bytes())?;
                    encoder.write_u32(entry.seen)?;
                    encoder.write_u64(entry.epoch)?;
                }
            }
            Ok(())
        })
    }
    pub(crate) fn observe(&self, syllables: &[Syllable], word: &str) {
        let mut lock = self
            .inner
            .write()
            .expect("Unable to acquire HistoryDict writer lock");
        lock.generation += 1;
        let wid = lock.string_table.intern(word);
        let generation = lock.generation;
        let hist_entries = lock.records.entry(syllables.into()).or_default();
        if let Some(pos) = hist_entries.iter().position(|e| e.wid == wid) {
            hist_entries[pos].seen += 1;
        } else {
            hist_entries.push(HistoryDictEntry {
                wid,
                seen: 1,
                epoch: generation,
            });
        }
    }
    pub fn remove(&self, syllables: &[Syllable], word: &str) {
        let mut lock = self
            .inner
            .write()
            .expect("Unable to acquire UserDict writer lock");
        let wid = lock.string_table.intern(word);
        let hist_entries = lock.records.entry(syllables.into()).or_default();
        if let Some(pos) = hist_entries.iter().position(|e| e.wid == wid) {
            hist_entries.remove(pos);
        }
    }
    // Return all words and their history based unigram log10 prob
    pub(crate) fn lookup(
        &self,
        syllables: &[Syllable],
        strategy: LookupStrategy,
    ) -> Vec<(WordId, f64)> {
        let lock = self
            .inner
            .read()
            .expect("Unable to acquire HistoryDict reader lock");
        let total = lock.generation as f64 + Self::COLD_THRESHOLD;
        match strategy {
            LookupStrategy::Standard => lock
                .records
                .get(syllables)
                .map(|entries| {
                    entries
                        .iter()
                        .map(|e| {
                            let count =
                                true_count(lock.half_life, lock.generation, e.seen, e.epoch);
                            let log10prob = (count as f64 / total).log10();
                            (e.wid, log10prob)
                        })
                        .collect()
                })
                .unwrap_or_default(),
            LookupStrategy::FuzzyPartialPrefix => {
                let mut end = syllables.to_vec();
                // NB: relies on the syllable encoding to
                // ensure Syllable::EMPTY is greater than all real syllables.
                end.push(Syllable::new());
                lock.records
                    .range::<[Syllable], _>((Included(syllables), Excluded(end.as_slice())))
                    .flat_map(|(_, entries)| {
                        entries.iter().map(|e| {
                            let count =
                                true_count(lock.half_life, lock.generation, e.seen, e.epoch);
                            let log10prob = (count as f64 / total).log10();
                            (e.wid, log10prob)
                        })
                    })
                    .collect()
            }
        }
    }
}

// Linear approximation (first-order)
fn true_count(h: u32, g: u64, c_i: u32, b_i: u64) -> u32 {
    let h = h as u64;
    let elapsed = g - b_i;
    let num_halvings = elapsed / h;
    let reminder = elapsed % h;
    let base = c_i as u64 >> num_halvings;
    (base - (base * reminder) / (2 * h)) as u32
}

impl_context_error!(pub HistoryDictError);

#[cfg(test)]
mod tests {
    use crate::{syl, zhuyin::Bopomofo};

    use super::*;

    #[test]
    fn learning_curve() {
        let st = StringTable::new();
        let hist = HistoryDict::new(st);
        let test = &[
            syl![Bopomofo::C, Bopomofo::TONE4],
            syl![Bopomofo::SH, Bopomofo::TONE4],
        ];

        assert_eq!(hist.lookup(test, LookupStrategy::Standard), vec![]);

        hist.observe(test, "測試");
        let log10prob = hist.lookup(test, LookupStrategy::Standard)[0].1;
        assert!(
            (log10prob - -5.69).abs() < 1e-2,
            "log10prob = {}",
            log10prob
        );

        hist.observe(test, "測試");
        let log10prob = hist.lookup(test, LookupStrategy::Standard)[0].1;
        assert!(
            (log10prob - -5.39).abs() < 1e-2,
            "log10prob = {}",
            log10prob
        );

        hist.observe(test, "測試");
        let log10prob = hist.lookup(test, LookupStrategy::Standard)[0].1;
        assert!(
            (log10prob - -5.22).abs() < 1e-2,
            "log10prob = {}",
            log10prob
        );

        for _ in 0..1000 {
            hist.observe(test, "測試");
        }
        let log10prob = hist.lookup(test, LookupStrategy::Standard)[0].1;
        assert!(
            (log10prob - -2.70).abs() < 1e-2,
            "log10prob = {}",
            log10prob
        );
    }
}
