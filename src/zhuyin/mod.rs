//! Chinese syllables and bopomofo phonetic symbols.

pub use self::bopomofo::{Bopomofo, BopomofoErrorKind, BopomofoKind, ParseBopomofoError};
/// Errors during decoding a syllable from a u16.
pub use self::syllable::DecodeSyllableError;
/// Errors when parsing a str to a syllable.
pub use self::syllable::ParseSyllableError;
pub use self::syllable::{BuildSyllableError, Syllable, SyllableBuilder, SyllableErrorKind};

pub use self::syllable::SyllableVec;

mod bopomofo;
mod syllable;
