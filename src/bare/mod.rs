//! Binary Application Record Encoding (BARE)
//!
//! Ref: <https://www.ietf.org/archive/id/draft-devault-bare-15.html>

use std::io::{Read, Write};

use scoped_error::impl_context_error;

pub(crate) mod primitives;

pub(crate) struct BareDecoder<R> {
    reader: R,
}

impl<R: Read> BareDecoder<R> {
    pub(crate) fn new(reader: R) -> Self {
        BareDecoder { reader }
    }
}

pub(crate) struct BareEncoder<W> {
    writer: W,
}

impl<W: Write> BareEncoder<W> {
    pub(crate) fn new(writer: W) -> Self {
        BareEncoder { writer }
    }
}

impl BareEncoder<Vec<u8>> {
    pub(crate) fn len(&self) -> usize {
        self.writer.len()
    }
}

impl<W> BareEncoder<W> {
    pub(crate) fn into_inner(self) -> W {
        self.writer
    }
}

impl<W> AsRef<W> for BareEncoder<W> {
    fn as_ref(&self) -> &W {
        &self.writer
    }
}

impl_context_error!(BareError);
