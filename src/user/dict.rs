//! User editable dictionary source

use std::{
    fs::File,
    io::{BufRead, BufReader, Write},
    path::Path,
    str::FromStr,
    sync::{Arc, RwLock},
};

use scoped_error::{expect_error, impl_context_error};
use smol_str::{SmolStr, ToSmolStr};
use tinyvec::TinyVec;

use crate::{
    dictionary::LookupStrategy,
    model::WordId,
    user::indexed_dict::IndexedDict,
    zhuyin::{Syllable, SyllableVec, parse_syllable_vec},
};

/// User provided dictionary
///
/// The UserDict type implements [`Clone`] and can be cheaply cloned and
/// shared between components.
#[derive(Debug, Clone)]
pub struct UserDict {
    inner: Arc<RwLock<IndexedDict>>,
}

impl UserDict {
    pub const MIN: i32 = -9_999_999;
    pub const MAX: i32 = 9_999_999;

    /// Returns an empty UserDict
    pub fn new() -> UserDict {
        UserDict {
            inner: Arc::new(RwLock::new(IndexedDict::new(WordId::MIN_USER))),
        }
    }
    /// Initialize an empty UserDict on the filesystem.
    ///
    /// If a file already exists then it will be truncated.
    pub fn init<P: AsRef<Path>>(path: P) -> Result<(), UserDictError> {
        expect_error("Failed to initialize UserDict", || {
            let dict = Self::new();
            let file = File::create(path)?;
            dict.to_writer(file)?;
            Ok(())
        })
    }
    /// Open an UserDict file and read from it.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<UserDict, UserDictError> {
        expect_error("Failed to open user dictionary", || {
            let file = File::open(path)?;
            let reader = BufReader::new(file);
            Ok(UserDict::from_reader(reader)?)
        })
    }
    /// Reads user dictionary from an IO stream
    pub fn from_reader<R: BufRead>(readr: R) -> Result<UserDict, UserDictError> {
        expect_error("Failed to parse user dictionary", || {
            let mut idict = IndexedDict::new(WordId::MIN_USER);
            for (i, io) in readr.lines().enumerate() {
                let line = io?;
                let mut parts = line.split(',');
                let word = parts
                    .next()
                    .ok_or_else(|| format!("invalid format at line {i}: {line}"))?;
                let bopomofo = parts
                    .next()
                    .ok_or_else(|| format!("invalid format at line {i}: {line}"))?;
                let boost = parts
                    .next()
                    .map(|b| i32::from_str(b).unwrap_or(0).clamp(Self::MIN, Self::MAX))
                    .unwrap_or(0);
                let syllables: SyllableVec = parse_syllable_vec(bopomofo.trim())?;
                idict.insert(syllables, word.to_smolstr(), boost);
            }
            Ok(UserDict {
                inner: Arc::new(RwLock::new(idict)),
            })
        })
    }
    /// Writes the user dictionary to an IO stream
    pub fn to_writer<W: Write>(&self, mut writer: W) -> Result<(), UserDictError> {
        expect_error("Unable to serialize the user dictionary", || {
            let lock = self
                .inner
                .read()
                .expect("Unable to acquire UserDict reader lock");
            for (syllables, word) in lock.iter() {
                writeln!(writer, "{},{}", word, syllables)?;
            }
            Ok(())
        })
    }
    /// Gets the text of a WordId
    pub fn get_text(&self, wid: WordId) -> Option<SmolStr> {
        let lock = self
            .inner
            .read()
            .expect("Unable to acquire UserDict reader lock");
        lock.get_text(wid)
    }
    /// Gets the WordId from (syllables, word)
    pub fn get_wid(&self, syllables: &[Syllable], word: &str) -> Option<(WordId, i32)> {
        let lock = self
            .inner
            .read()
            .expect("Unable to acquire UserDict reader lock");
        lock.get_wid(syllables, word)
    }
    pub fn lookup(
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

impl_context_error!(pub UserDictError);
