//! User editable dictionary source

use std::{
    io::{BufRead, Write},
    sync::{Arc, RwLock},
};

use scoped_error::{expect_error, impl_context_error};
use smol_str::{SmolStr, ToSmolStr};

use crate::{
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
    /// Returns an empty UserDict
    pub fn new() -> UserDict {
        UserDict {
            inner: Arc::new(RwLock::new(IndexedDict::new(WordId::MIN_USER))),
        }
    }
    /// Reads user dictionary from an IO stream
    pub fn from_reader<R: BufRead>(readr: R) -> Result<UserDict, UserDictError> {
        expect_error("Failed to parse user dictionary", || {
            let mut idict = IndexedDict::new(WordId::MIN_USER);
            for (i, io) in readr.lines().enumerate() {
                let line = io?;
                let (word, bopomofo) = line
                    .split_once(',')
                    .ok_or_else(|| format!("invalid format at line {i}: {line}"))?;
                let syllables: SyllableVec = parse_syllable_vec(bopomofo.trim())?;
                idict.insert(syllables, word.to_smolstr());
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
    pub fn get_wid(&self, syllables: &[Syllable], word: &str) -> Option<WordId> {
        let lock = self
            .inner
            .read()
            .expect("Unable to acquire UserDict reader lock");
        lock.get_wid(syllables, word)
    }
}

impl_context_error!(pub UserDictError);
