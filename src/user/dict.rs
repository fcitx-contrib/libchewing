//! User editable dictionary source

use std::{
    collections::BTreeMap,
    fmt::Display,
    fs::File,
    io::{BufRead, BufReader, Write},
    ops::Bound::{Excluded, Included},
    path::Path,
    str::FromStr,
    sync::{Arc, RwLock},
};

use scoped_error::{expect_error, impl_context_error};

use crate::{
    dictionary::{LookupStrategy, StringTable},
    model::WordId,
    zhuyin::{Syllable, SyllableVec},
};

/// User provided dictionary
///
/// The UserDict type implements [`Clone`] and can be cheaply cloned and
/// shared between components.
#[derive(Debug, Clone)]
pub struct UserDict {
    inner: Arc<RwLock<UserDictInner>>,
}

#[derive(Debug)]
struct UserDictInner {
    string_table: StringTable,
    records: BTreeMap<SyllableVec, Vec<UserDictEntry>>,
}

#[derive(Debug, Clone, Copy)]
struct UserDictEntry {
    wid: WordId,
    boost: i8,
}

impl UserDict {
    pub const MIN: i8 = -100;
    pub const MAX: i8 = 100;

    /// Returns an empty UserDict
    pub fn new(string_table: StringTable) -> UserDict {
        UserDict {
            inner: Arc::new(RwLock::new(UserDictInner {
                string_table,
                records: BTreeMap::new(),
            })),
        }
    }
    /// Initialize an empty UserDict on the filesystem.
    ///
    /// If a file already exists then it will be truncated.
    pub fn init<P: AsRef<Path>>(path: P) -> Result<(), UserDictError> {
        expect_error("Failed to initialize UserDict", || {
            let dict = Self::new(StringTable::new());
            let file = File::create(path)?;
            dict.to_writer(file)?;
            Ok(())
        })
    }
    /// Open an UserDict file and read from it.
    pub fn open<P: AsRef<Path>>(
        path: P,
        string_table: StringTable,
    ) -> Result<UserDict, UserDictError> {
        expect_error("Failed to open user dictionary", || {
            let file = File::open(path)?;
            let reader = BufReader::new(file);
            Ok(UserDict::from_reader(reader, string_table)?)
        })
    }
    /// Reads user dictionary from an IO stream
    pub fn from_reader<R: BufRead>(
        readr: R,
        string_table: StringTable,
    ) -> Result<UserDict, UserDictError> {
        expect_error("Failed to parse user dictionary", || {
            let mut records = BTreeMap::new();

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
                    .map(|b| i8::from_str(b).unwrap_or(0).clamp(Self::MIN, Self::MAX))
                    .unwrap_or(0);
                let syllables: SyllableVec = bopomofo.trim().parse()?;

                let wid = string_table.intern(word);

                let word_entries = records.entry(syllables).or_insert(vec![]);
                word_entries.push(UserDictEntry { wid, boost });
            }
            Ok(UserDict {
                inner: Arc::new(RwLock::new(UserDictInner {
                    string_table,
                    records,
                })),
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
            for (syllables, entries) in lock.records.iter() {
                for entry in entries {
                    let word = lock
                        .string_table
                        .get_text(entry.wid)
                        .expect("Should have this word");
                    writeln!(
                        writer,
                        "{},{},{}",
                        word,
                        display_syllables(syllables),
                        entry.boost
                    )?;
                }
            }
            Ok(())
        })
    }
    pub fn get_text(&self, wid: WordId) -> Option<String> {
        let lock = self
            .inner
            .read()
            .expect("Unable to acquire UserDict reader lock");
        lock.string_table.get_text(wid)
    }
    pub fn insert(&self, syllables: &[Syllable], word: &str) {
        let mut lock = self
            .inner
            .write()
            .expect("Unable to acquire UserDict writer lock");
        let wid = lock.string_table.intern(word);
        log::debug!("intern {} => {}", word, wid);
        let word_entries = lock.records.entry(syllables.into()).or_default();
        if word_entries.iter().find(|e| e.wid == wid).is_none() {
            word_entries.push(UserDictEntry { wid, boost: 10 });
        }
    }
    pub fn boost(&self, syllables: &[Syllable], word: &str) {
        let mut lock = self
            .inner
            .write()
            .expect("Unable to acquire UserDict writer lock");
        let wid = lock.string_table.intern(word);
        let word_entries = lock.records.entry(syllables.into()).or_default();
        if let Some(pos) = word_entries.iter().position(|e| e.wid == wid) {
            word_entries[pos].boost = (word_entries[pos].boost + 100).clamp(Self::MIN, Self::MAX);
        } else {
            word_entries.push(UserDictEntry { wid, boost: 10 });
        }
    }
    pub fn deboost(&self, syllables: &[Syllable], word: &str) {
        let mut lock = self
            .inner
            .write()
            .expect("Unable to acquire UserDict writer lock");
        let wid = lock.string_table.intern(word);
        let word_entries = lock.records.entry(syllables.into()).or_default();
        if let Some(pos) = word_entries.iter().position(|e| e.wid == wid) {
            word_entries[pos].boost = (word_entries[pos].boost - 10).clamp(Self::MIN, Self::MAX);
        } else {
            word_entries.push(UserDictEntry { wid, boost: -10 });
        }
    }
    pub fn remove(&self, syllables: &[Syllable], word: &str) {
        let mut lock = self
            .inner
            .write()
            .expect("Unable to acquire UserDict writer lock");
        let wid = lock.string_table.intern(word);
        let word_entries = lock.records.entry(syllables.into()).or_default();
        if let Some(pos) = word_entries.iter().position(|e| e.wid == wid) {
            word_entries.remove(pos);
        }
    }
    pub fn lookup(&self, syllables: &[Syllable], strategy: LookupStrategy) -> Vec<(WordId, i8)> {
        let lock = self
            .inner
            .read()
            .expect("Unable to acquire UserDict reader lock");
        match strategy {
            LookupStrategy::Standard => lock
                .records
                .get(syllables)
                .map(|entries| entries.iter().map(|e| (e.wid, e.boost)).collect())
                .unwrap_or_default(),
            LookupStrategy::FuzzyPartialPrefix => {
                let mut end = syllables.to_vec();
                // NB: relies on the syllable encoding to
                // ensure Syllable::EMPTY is greater than all real syllables.
                end.push(Syllable::new());
                lock.records
                    .range::<[Syllable], _>((Included(syllables), Excluded(end.as_slice())))
                    .flat_map(|(_, entries)| entries.iter().map(|e| (e.wid, e.boost)))
                    .collect()
            }
        }
    }
    pub fn entries(&self) -> impl Iterator<Item = (SyllableVec, String)> + '_ {
        let lock = self
            .inner
            .read()
            .expect("Unable to acquire UserDict reader lock");
        lock.records.clone().into_iter().flat_map(move |(k, v)| {
            let st = lock.string_table.clone();
            v.into_iter()
                .map(move |ent| (k, st.get_text(ent.wid).unwrap()))
        })
    }
}

fn display_syllables(syllables: &[Syllable]) -> impl Display {
    syllables
        .iter()
        .map(|syl| syl.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

impl_context_error!(pub UserDictError);
