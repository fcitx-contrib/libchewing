use std::{borrow::Borrow, fmt::Display};

use crate::zhuyin::Syllable;

/// A type containing a phrase string and its frequency.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Phrase {
    pub(crate) text: Box<str>,
    pub(crate) freq: i32,
    pub(crate) last_used: Option<u64>,
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

impl Display for Phrase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

pub type Phrases<'a> = Box<dyn Iterator<Item = Phrase> + 'a>;

pub type Entries<'a> = Box<dyn Iterator<Item = (Vec<Syllable>, Phrase)> + 'a>;
