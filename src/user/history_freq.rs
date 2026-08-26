//! Word frequency learned from user history

use std::{
    collections::BTreeMap,
    io::{Read, Write},
    sync::{Arc, RwLock},
};

use scoped_error::{bail, expect_error, impl_context_error};

use crate::{
    bare::{BareDecoder, BareEncoder},
    model::WordId,
};

#[derive(Debug, Clone)]
pub struct HistoryFreq {
    inner: Arc<RwLock<HistoryFreqInner>>,
}

#[derive(Debug)]
struct HistoryFreqInner {
    half_life: u32,
    generation: u64,
    history: BTreeMap<WordId, HistoryCount>,
}

#[derive(Debug)]
struct HistoryCount {
    /// How many times this word has been observed
    c_i: u32,
    /// The first generation this word was observed
    b_i: u64,
}

impl HistoryFreq {
    pub const HALF_LIFE: u32 = 50_000;

    /// Creates an empty HistoryFreq
    pub fn new() -> HistoryFreq {
        HistoryFreq {
            inner: Arc::new(RwLock::new(HistoryFreqInner {
                half_life: Self::HALF_LIFE,
                generation: 0,
                history: BTreeMap::new(),
            })),
        }
    }
    /// Reads the history freq from an IO stream
    pub fn from_reader<R, F>(reader: R, widmap: F) -> Result<HistoryFreq, HistoryFreqError>
    where
        R: Read,
        F: Fn(&str) -> WordId,
    {
        expect_error("Failed to parse history freq", || {
            let mut decoder = BareDecoder::new(reader);

            let mut history = BTreeMap::new();

            // Read file magic
            let magic = decoder.read_data_exact(4)?;
            if magic != b"CHHF" {
                bail!("Invalid file header");
            }
            // Read HistoryFreqFile union version
            let version = decoder.read_uint()?;
            if version != 0 {
                bail!("Unknown file version");
            }
            // Read HistoryFreqHeader flags
            let flags = decoder.read_u32()?;
            if flags != 0 {
                bail!("Unknown header flags");
            }
            let half_life = decoder.read_u32()?;
            let generation = decoder.read_u64()?;
            let records_len = decoder.read_u32()? as usize;
            for _ in 0..records_len {
                let raw_word = decoder.read_data()?;
                let word = str::from_utf8(&raw_word)?;
                let c_i = decoder.read_u32()?;
                let b_i = decoder.read_u64()?;
                let wid = widmap(&word);
                history.insert(wid, HistoryCount { c_i, b_i });
            }
            Ok(HistoryFreq {
                inner: Arc::new(RwLock::new(HistoryFreqInner {
                    half_life,
                    generation,
                    history,
                })),
            })
        })
    }
    /// Serialize the history freq to an IO stream
    pub fn to_writer<'a, W, F>(&self, writer: W, widmap: F) -> Result<(), HistoryFreqError>
    where
        W: Write,
        F: Fn(WordId) -> &'a str,
    {
        expect_error("Failed to serialize history freq", || {
            let lock = self
                .inner
                .read()
                .expect("Unable to acquire HistoryFreq reade lock");
            let mut encoder = BareEncoder::new(writer);

            // Wrtie file magic
            encoder.write_data_exact(b"CHHF")?;
            // Write HistoryFreqFile union version
            encoder.write_uint(0)?;
            // Write HistoryFreqHeader
            // flags
            encoder.write_u32(0)?;
            encoder.write_u32(lock.half_life)?;
            encoder.write_u64(lock.generation)?;

            let len = lock
                .history
                .values()
                .map(|c| true_count(lock.half_life as u64, lock.generation, c.c_i, c.b_i))
                .filter(|tc| *tc > 0)
                .count();
            encoder.write_uint(len as u64)?;

            for (wid, c) in lock.history.iter() {
                let tc = true_count(lock.half_life as u64, lock.generation, c.c_i, c.b_i);
                if tc == 0 {
                    continue;
                }
                let word = widmap(*wid);
                encoder.write_data(word.as_bytes())?;
                encoder.write_u32(c.c_i)?;
                encoder.write_u64(c.b_i)?;
            }

            Ok(())
        })
    }
    // Gets the log10 probability of word from history
    pub fn get(&self, wid: WordId) -> Option<f64> {
        let lock = self
            .inner
            .read()
            .expect("Unable to acquire UserFreq reader lock");
        lock.history
            .get(&wid)
            .map(|c| true_count(lock.half_life as u64, lock.generation, c.c_i, c.b_i))
            .map(|c| (c as f64 / lock.generation as f64).log10())
    }
}

// Linear approximation (first-order)
fn true_count(h: u64, g: u64, c_i: u32, b_i: u64) -> u32 {
    let elapsed = g - b_i;
    let num_halvings = elapsed / h;
    let reminder = elapsed % h;
    let base = c_i as u64 >> num_halvings;
    (base - (base * reminder) / (2 * h)) as u32
}

impl_context_error!(pub HistoryFreqError);
