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
    pub(crate) const MIN_STATIC: u32 = 0x00000000;
    pub(crate) const MIN_USER: u32 = 0x01000000;
    pub(crate) fn as_offset(&self) -> usize {
        (self.0 & 0x00FFFFFF) as usize
    }
}

/// Identify the origin of a WordId
#[derive(Debug)]
pub enum WordOrig {
    /// The word is defined in the static word list
    Static,
    /// The word is defined in the user vocabulary
    User,
    /// Unknown origin, might be invalid
    Unknown,
}

impl WordId {
    pub fn orig(&self) -> WordOrig {
        let prefix = self.0 & 0xFF000000;
        match prefix {
            WordId::MIN_STATIC => WordOrig::Static,
            WordId::MIN_USER => WordOrig::User,
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
