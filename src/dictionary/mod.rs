//! Systems and user phrase dictionaries.
#![allow(deprecated)]

use std::{
    any::Any,
    borrow::Borrow,
    cmp::Ordering,
    error::Error,
    fmt::{Debug, Display},
    path::Path,
};

pub use self::composite::CompositeDict;
pub use self::string_table::StringTable;
pub use self::trie::{Trie, TrieBuilder, TrieOpenOptions, TrieStatistics};
pub use self::usage::DictionaryUsage;
use crate::exn::Exn;
use crate::zhuyin::Syllable;

mod composite;
mod string_table;
mod trie;
mod usage;

/// A collection of metadata of a dictionary.
///
/// The dictionary version and copyright information can be used in
/// configuration application.
///
/// # Examples
///
/// ```no_run
/// # use chewing::dictionary::{Dictionary, Trie};
/// # let dictionary = Trie::new(&[][..]).unwrap();
/// let about = dictionary.about();
/// assert_eq!("libchewing default", about.name);
/// assert_eq!("Copyright (c) 2022 libchewing Core Team", about.copyright);
/// assert_eq!("LGPL-2.1-or-later", about.license);
/// assert_eq!("init_database 0.5.1", about.software);
/// ```
#[derive(Debug, Clone, Default)]
pub struct DictionaryInfo {
    /// The name of the dictionary.
    pub name: String,
    /// The copyright information of the dictionary.
    ///
    /// It's recommended to include the copyright holders' names and email
    /// addresses, separated by semicolons.
    pub copyright: String,
    /// The license information of the dictionary.
    ///
    /// It's recommended to use the [SPDX license identifier](https://spdx.org/licenses/).
    pub license: String,
    /// The version of the dictionary.
    ///
    /// It's recommended to use the commit hash or revision if the dictionary is
    /// managed in a source control repository.
    pub version: String,
    /// The name of the software used to generate the dictionary.
    ///
    /// It's recommended to include the name and the version number.
    pub software: String,
    /// The intended usage of the dictionary.
    pub usage: DictionaryUsage,
}

/// A type containing a phrase string and its frequency.
///
/// # Examples
///
/// A `Phrase` can be created from/to a tuple.
///
/// ```
/// use chewing::dictionary::Phrase;
///
/// let phrase = Phrase::new("測", 1);
/// assert_eq!(phrase, ("測", 1).into());
/// assert_eq!(("測".to_string(), 1i32), phrase.into());
/// ```
///
/// Phrases are ordered by their frequency.
///
/// ```
/// use chewing::dictionary::Phrase;
///
/// assert!(Phrase::new("測", 100) > Phrase::new("冊", 1));
/// ```
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Phrase {
    text: Box<str>,
    freq: i32,
    last_used: Option<u64>,
}

impl Phrase {
    /// Creates a new `Phrase`.
    ///
    /// # Examples
    ///
    /// ```
    /// use chewing::dictionary::Phrase;
    ///
    /// let phrase = Phrase::new("新", 1);
    /// ```
    pub fn new<S>(phrase: S, freq: i32) -> Phrase
    where
        S: Into<Box<str>>,
    {
        Phrase {
            text: phrase.into(),
            freq,
            last_used: None,
        }
    }
    /// Sets the last used time of the phrase.
    pub fn with_time(mut self, last_used: u64) -> Phrase {
        self.last_used = Some(last_used);
        self
    }
    /// Returns the frequency of the phrase.
    ///
    /// # Examples
    ///
    /// ```
    /// use chewing::dictionary::Phrase;
    ///
    /// let phrase = Phrase::new("詞頻", 100);
    ///
    /// assert_eq!(100, phrase.freq());
    /// ```
    pub fn freq(&self) -> i32 {
        self.freq
    }
    /// Returns the last time this phrase was selected as user phrase.
    ///
    /// The time is a counter increased by one for each keystroke.
    pub fn last_used(&self) -> Option<u64> {
        self.last_used
    }
    /// Returns the inner str of the phrase.
    ///
    /// # Examples
    ///
    /// ```
    /// use chewing::dictionary::Phrase;
    ///
    /// let phrase = Phrase::new("詞", 100);
    ///
    /// assert_eq!("詞", phrase.as_str());
    /// ```
    pub fn as_str(&self) -> &str {
        self.text.borrow()
    }
}

/// Phrases are compared by their frequency first, followed by their phrase
/// string.
impl PartialOrd for Phrase {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Phrases are compared by their frequency first, followed by their phrase
/// string.
impl Ord for Phrase {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.freq.cmp(&other.freq) {
            Ordering::Equal => {}
            ord => return ord,
        }
        self.text.cmp(&other.text)
    }
}

impl AsRef<str> for Phrase {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl From<Phrase> for String {
    fn from(phrase: Phrase) -> Self {
        phrase.text.into_string()
    }
}

impl From<Phrase> for Box<str> {
    fn from(phrase: Phrase) -> Self {
        phrase.text
    }
}

impl From<Phrase> for (String, i32) {
    fn from(phrase: Phrase) -> Self {
        (phrase.text.into_string(), phrase.freq)
    }
}

impl<S> From<(S, i32)> for Phrase
where
    S: Into<Box<str>>,
{
    fn from(tuple: (S, i32)) -> Self {
        Phrase::new(tuple.0, tuple.1)
    }
}

impl<S> From<(S, i32, u64)> for Phrase
where
    S: Into<Box<str>>,
{
    fn from(tuple: (S, i32, u64)) -> Self {
        Phrase::new(tuple.0, tuple.1).with_time(tuple.2)
    }
}

impl Display for Phrase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A boxed iterator over the phrases and their frequency in a dictionary.
///
/// # Examples
///
/// ```no_run
/// use chewing::{dictionary::{Dictionary, LookupStrategy, Trie}, syl, zhuyin::Bopomofo};
///
/// # let dict = Trie::new(&[][..]).unwrap();
///
/// for phrase in dict.lookup(
///     &[syl![Bopomofo::C, Bopomofo::E, Bopomofo::TONE4]], LookupStrategy::Standard
/// ) {
///     assert_eq!("測", phrase.as_str());
///     assert_eq!(100, phrase.freq());
/// }
/// ```
pub type Phrases<'a> = Box<dyn Iterator<Item = Phrase> + 'a>;

/// A boxed iterator over all the entries in a dictionary.
///
/// # Examples
///
/// ```no_run
/// use chewing::{dictionary::{Dictionary, Trie}, syl, zhuyin::Bopomofo};
///
/// # let dict = Trie::new(&[][..]).unwrap();
///
/// for (syllables, phrase) in dict.entries() {
///     for bopomofos in syllables {
///         println!("{bopomofos} -> {phrase}");
///     }
/// }
/// ```
pub type Entries<'a> = Box<dyn Iterator<Item = (Vec<Syllable>, Phrase)> + 'a>;

/// The lookup strategy hint for dictionary.
///
/// If the dictionary supports the lookup strategy it should try to use.
/// Otherwise fallback to standard.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookupStrategy {
    /// The native lookup strategy supported by the dictionary.
    #[default]
    Standard,
    /// Try to fuzzy match partial syllables using only preffix.
    FuzzyPartialPrefix,
}

/// An interface for looking up dictionaries.
///
/// This is the main dictionary trait. For more about the concept of
/// dictionaries generally, please see the [module-level
/// documentation][crate::dictionary].
pub trait Dictionary: Debug {
    /// Returns all phrases matched by the syllables.
    ///
    /// The result should use a stable order each time for the same input.
    fn lookup(&self, syllables: &[Syllable], strategy: LookupStrategy) -> Vec<Phrase>;
    /// Returns an iterator to all phrases in the dictionary.
    fn entries(&self) -> Entries<'_>;
    /// Returns information about the dictionary instance.
    fn about(&self) -> DictionaryInfo;
    /// Returns the dictionary file path if it's backed by a file.
    fn path(&self) -> Option<&Path>;
    /// Set the runtime usage of the dictionary
    fn set_usage(&mut self, usage: DictionaryUsage);
    /// Reopens the dictionary if it was changed by a different process
    ///
    /// It should not fail if the dictionary is read-only or able to sync across
    /// processes automatically.
    fn reopen(&mut self) -> Result<(), UpdateDictionaryError> {
        Err(UpdateDictionaryError::new("unimplemented"))
    }
    /// Flushes all the changes back to the filesystem
    ///
    /// The change made to the dictionary might not be persisted without
    /// calling this method.
    fn flush(&mut self) -> Result<(), UpdateDictionaryError> {
        Err(UpdateDictionaryError::new("unimplemented"))
    }
    /// An method for updating dictionaries.
    ///
    /// For more about the concept of dictionaries generally, please see the
    /// [module-level documentation][crate::dictionary].
    fn add_phrase(
        &mut self,
        _syllables: &[Syllable],
        _phrase: Phrase,
    ) -> Result<(), UpdateDictionaryError> {
        Err(UpdateDictionaryError::new("unimplemented"))
    }
    /// TODO: doc
    fn update_phrase(
        &mut self,
        _syllables: &[Syllable],
        _phrase: Phrase,
        _user_freq: u32,
        _time: u64,
    ) -> Result<(), UpdateDictionaryError> {
        Err(UpdateDictionaryError::new("unimplemented"))
    }
    /// TODO: doc
    fn remove_phrase(
        &mut self,
        _syllables: &[Syllable],
        _phrase_str: &str,
    ) -> Result<(), UpdateDictionaryError> {
        Err(UpdateDictionaryError::new("unimplemented"))
    }
}

/// TODO: doc
pub trait DictionaryBuilder: Any {
    /// TODO: doc
    fn set_info(&mut self, info: DictionaryInfo) -> Result<(), BuildDictionaryError>;
    /// TODO: doc
    fn insert(
        &mut self,
        syllables: &[Syllable],
        phrase: Phrase,
    ) -> Result<(), BuildDictionaryError>;
    /// TODO: doc
    fn build(&mut self, path: &Path) -> Result<(), BuildDictionaryError>;
}

/// The error type which is returned from updating a dictionary.
#[derive(Debug)]
pub struct UpdateDictionaryError {
    /// TODO: doc
    message: &'static str,
    source: Option<Box<dyn Error + Send + Sync>>,
}

impl UpdateDictionaryError {
    pub(crate) fn new(message: &'static str) -> UpdateDictionaryError {
        UpdateDictionaryError {
            message,
            source: None,
        }
    }
}

impl Display for UpdateDictionaryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "update dictionary failed: {}", self.message)
    }
}

impl_exn!(UpdateDictionaryError);

/// Errors during dictionary construction.
#[derive(Debug)]
pub struct BuildDictionaryError {
    msg: String,
    source: Option<Box<dyn Error + Send + Sync + 'static>>,
}

impl BuildDictionaryError {
    fn new(msg: &str) -> BuildDictionaryError {
        BuildDictionaryError {
            msg: msg.to_string(),
            source: None,
        }
    }
}

impl Display for BuildDictionaryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "build dictionary error: {}", self.msg)
    }
}

impl_exn!(BuildDictionaryError);

#[cfg(test)]
mod tests {
    use crate::dictionary::{Dictionary, DictionaryBuilder};

    #[test]
    fn ensure_object_safe() {
        const _: Option<&dyn Dictionary> = None;
        const _: Option<&dyn DictionaryBuilder> = None;
    }
}
