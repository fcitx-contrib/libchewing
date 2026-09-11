//! Systems and user phrase dictionaries.
#![allow(deprecated)]

use std::{
    borrow::Borrow,
    cmp::Ordering,
    fmt::{Debug, Display},
};

pub use self::composite::CompositeDict;
pub use self::string_table::StringTable;
pub use self::string_table::StringTableBuilder;
pub use self::trie::{Trie, TrieOpenOptions};
use crate::zhuyin::Syllable;

mod composite;
mod string_table;
mod trie;

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
