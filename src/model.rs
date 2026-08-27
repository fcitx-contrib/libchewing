//! Common models shared by all components

use std::{fmt::Display, ops::Deref};

/// Locally unique id for a word
#[derive(Debug, Default, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct WordId(pub u32);

impl Deref for WordId {
    type Target = u32;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Display for WordId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl From<u32> for WordId {
    fn from(value: u32) -> Self {
        WordId(value)
    }
}

impl WordId {
    pub(crate) const MIN_STATIC: WordId = WordId(0x00000000);
    pub(crate) const MIN_HISTORY: WordId = WordId(0x01000000);
    pub(crate) const MIN_EXTRA: WordId = WordId(0x02000000);
    pub(crate) const MIN_USER: WordId = WordId(0x0F000000);
    pub(crate) fn inc(&mut self) {
        self.0 += 1;
    }
    pub(crate) fn as_offset(&self) -> usize {
        (self.0 & 0x00FFFFFF) as usize
    }
    pub(crate) fn from_user(id: u32) -> WordId {
        WordId(Self::MIN_USER.0 + id)
    }
}

/// Identify the origin of a WordId
#[derive(Debug)]
pub enum WordOrig {
    /// The word is defined in the static word list
    Static,
    /// The word is learned from user inputs
    History,
    /// The word is defined in an extra dictionary
    Extra,
    /// The word is defined in the user vocabulary
    User,
    /// Unknown origin, might be invalid
    Unknown,
}

impl WordId {
    pub const MIN: WordId = WordId(u32::MIN);
    pub const MAX: WordId = WordId(u32::MAX);

    pub fn orig(&self) -> WordOrig {
        let prefix = self.0 & 0xFF000000;
        match prefix {
            0x00000000 => WordOrig::Static,
            0x01000000 => WordOrig::History,
            0x02000000 => WordOrig::Extra,
            0x0F000000 => WordOrig::User,
            _ => WordOrig::Unknown,
        }
    }
}

/// A possible intepretation of the input state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Surface {
    Word(WordId),
    Char(char),
    #[default]
    None,
}
