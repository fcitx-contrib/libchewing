//! Automatic learned user vocabulary list
//!
//! The auto user vocabulary list stores new words learned from user interactions

use std::{
    fs::File,
    io::{BufRead, BufReader, Write},
    path::Path,
    sync::{Arc, RwLock},
};

use log::warn;
use scoped_error::{bail, expect_error, impl_context_error};
use smol_str::{SmolStr, ToSmolStr};
use tinyvec::{TinyVec, tiny_vec};

use super::IndexedDict;
use crate::{
    bare::{BareDecoder, BareEncoder},
    dictionary::LookupStrategy,
    model::WordId,
    zhuyin::Syllable,
};

/// Automatic learned user vocabulary list
#[derive(Debug, Clone)]
pub struct HistoryDict {
    inner: Arc<RwLock<IndexedDict>>,
}

impl HistoryDict {
    /// Returns an empty HistoryDict
    pub fn new() -> HistoryDict {
        HistoryDict {
            inner: Arc::new(RwLock::new(IndexedDict::new(WordId::MIN_HISTORY))),
        }
    }
    /// Initialize an empty HistoryDict on the filesystem.
    ///
    /// If a file already exists then it will be truncated.
    pub fn init<P: AsRef<Path>>(path: P) -> Result<(), HistoryDictError> {
        expect_error("Failed to initialize UserDict", || {
            let dict = Self::new();
            let file = File::create(path)?;
            dict.to_writer(file)?;
            Ok(())
        })
    }
    /// Open an HistoryDict file and read from it.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<HistoryDict, HistoryDictError> {
        expect_error("Failed to open user dictionary", || {
            let file = File::open(path)?;
            let reader = BufReader::new(file);
            Ok(HistoryDict::from_reader(reader)?)
        })
    }
    /// Reads history dict from the IO stream
    pub fn from_reader<R: BufRead>(reader: R) -> Result<HistoryDict, HistoryDictError> {
        expect_error("Failed to parse history dict", || {
            let mut decoder = BareDecoder::new(reader);

            let mut idict = IndexedDict::new(WordId::MIN_HISTORY);

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
            let records_len = decoder.read_u32()? as usize;

            // Read record frames
            for _ in 0..records_len {
                let len = decoder.read_uint()? as usize;
                let mut syllables = tiny_vec!([Syllable; 5]);
                for _ in 0..len {
                    let syl = Syllable::try_from(decoder.read_u16()?)?;
                    syllables.push(syl);
                }
                let raw_word = decoder.read_data()?;
                let word = str::from_utf8(&raw_word)?;

                idict.insert(syllables, word.to_smolstr(), 0);
            }
            Ok(HistoryDict {
                inner: Arc::new(RwLock::new(idict)),
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
            let len = lock.len();
            if len > u32::MAX as usize {
                warn!("writing more than 2^32 records");
            }
            // Write the records length
            encoder.write_u32(lock.len() as u32)?;
            for (syllables, word) in lock.iter() {
                encoder.write_uint(syllables.len() as u64)?;
                for syl in syllables {
                    encoder.write_u16(syl.to_u16())?;
                }
                encoder.write_data(word.as_bytes())?;
            }
            Ok(())
        })
    }
    /// Gets the text of a WordId
    pub fn get_text(&self, wid: WordId) -> Option<SmolStr> {
        let lock = self
            .inner
            .read()
            .expect("Unable to acquire UserVocab reader lock");
        lock.get_text(wid)
    }
    /// Gets the WordId from (syllables, word)
    pub fn get_wid(&self, syllables: &[Syllable], word: &str) -> Option<(WordId, i32)> {
        let lock = self
            .inner
            .read()
            .expect("Unable to acquire UserVocab reader lock");
        lock.get_wid(syllables, word)
    }
    pub(crate) fn lookup(
        &self,
        syllables: &[Syllable],
        strategy: LookupStrategy,
    ) -> TinyVec<[(WordId, i32); 3]> {
        let lock = self
            .inner
            .read()
            .expect("Unable to acquire UserDict reader lock");
        lock.lookup(syllables, strategy)
    }
}

impl_context_error!(pub HistoryDictError);
