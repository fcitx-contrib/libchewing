//! Chinese syllables and bopomofo phonetic symbols.

pub use self::bopomofo::{Bopomofo, BopomofoErrorKind, BopomofoKind, ParseBopomofoError};
pub use self::syllable::{
    BuildSyllableError, DecodeSyllableError, ParseSyllableError, Syllable, SyllableBuilder,
    SyllableErrorKind,
};

pub use self::syllable::SyllableVec;

mod bopomofo;
mod syllable;
