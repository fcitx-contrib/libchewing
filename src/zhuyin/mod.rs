//! Chinese syllables and bopomofo phonetic symbols.

pub use self::bopomofo::{Bopomofo, BopomofoErrorKind, BopomofoKind, ParseBopomofoError};
pub use self::syllable::{
    BuildSyllableError, DecodeSyllableError, ParseSyllableError, Syllable, SyllableBuilder,
    SyllableErrorKind,
};

pub(crate) use self::syllable::SyllableVec;
pub(crate) use self::syllable::parse_syllable_vec;

mod bopomofo;
mod syllable;
