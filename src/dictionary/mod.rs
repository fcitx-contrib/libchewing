//! Systems and user phrase dictionaries.
#![allow(deprecated)]

use std::fmt::Debug;

pub use self::composite::CompositeDict;
pub use self::phrase::{Entries, Phrase, Phrases};
pub use self::string_table::StringTable;
pub use self::string_table::StringTableBuilder;
pub use self::trie::{Trie, TrieOpenOptions};

mod composite;
mod phrase;
mod string_table;
mod trie;

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
