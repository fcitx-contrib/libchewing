//! Binary Application Record Encoding (BARE)
//!
//! Ref: <https://www.ietf.org/archive/id/draft-devault-bare-15.html>

use std::io::{Read, Write};

use scoped_error::impl_context_error;

pub(crate) mod primitives;

pub(crate) struct BareDecoder<R: Read> {
    reader: R,
}

impl<R: Read> BareDecoder<R> {
    pub(crate) fn new(reader: R) -> Self {
        BareDecoder { reader }
    }
}

pub(crate) struct BareEncoder<W: Write> {
    writer: W,
}

impl<W: Write> BareEncoder<W> {
    pub(crate) fn new(writer: W) -> Self {
        BareEncoder { writer }
    }
}

impl_context_error!(BareError);
