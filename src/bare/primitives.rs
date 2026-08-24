use std::io::Read;
use std::io::Write;

use der::Writer;
use scoped_error::bail;
use scoped_error::expect_error;

use super::BareDecoder;
use super::BareEncoder;
use super::BareError;

impl<R: Read> BareDecoder<R> {
    pub(crate) fn read_uint(&mut self) -> Result<u64, BareError> {
        expect_error("Failed to decode uint from buffer", || {
            let mut acc = 0;
            for i in 0.. {
                let mut buf = [0u8; 1];
                self.reader.read_exact(&mut buf)?;
                let b = buf[0];

                if i > 9 || i == 9 && b > 1 {
                    bail!("invalid uint encoding");
                }
                if b < 0x80 {
                    return Ok(acc | ((b as u64) << (i * 7)));
                }
                acc |= ((b & 0x7f) as u64) << (i * 7);
            }
            unreachable!()
        })
    }
    pub(crate) fn read_u8(&mut self) -> Result<u8, BareError> {
        expect_error("Failed to decode u8 from buffer", || {
            let mut buf = [0u8; 1];
            self.reader.read_exact(&mut buf)?;
            Ok(buf[0])
        })
    }
    pub(crate) fn read_u16(&mut self) -> Result<u16, BareError> {
        expect_error("Failed to decode u16 from buffer", || {
            let mut buf = [0u8; 2];
            self.reader.read_exact(&mut buf)?;
            Ok(u16::from_le_bytes(buf.try_into()?))
        })
    }
    pub(crate) fn read_u32(&mut self) -> Result<u32, BareError> {
        expect_error("Failed to decode u32 from buffer", || {
            let mut buf = [0u8; 4];
            self.reader.read_exact(&mut buf)?;
            Ok(u32::from_le_bytes(buf.try_into()?))
        })
    }
    pub(crate) fn read_u64(&mut self) -> Result<u64, BareError> {
        expect_error("Failed to decode u64 from buffer", || {
            let mut buf = [0u8; 8];
            self.reader.read_exact(&mut buf)?;
            Ok(u64::from_le_bytes(buf.try_into()?))
        })
    }
    pub(crate) fn read_data(&mut self) -> Result<Vec<u8>, BareError> {
        expect_error("Failed to decode data from buffer", || {
            let len = self.read_uint()? as usize;
            let mut buf = vec![0; len];
            self.reader.read_exact(&mut buf)?;
            Ok(buf)
        })
    }
    pub(crate) fn read_data_exact(&mut self, len: usize) -> Result<Vec<u8>, BareError> {
        expect_error("Failed to decode data[length] from buffer", || {
            let mut buf = vec![0; len];
            self.reader.read_exact(&mut buf)?;
            Ok(buf)
        })
    }
    pub(crate) fn read_list_u32_exact(&mut self, count: usize) -> Result<Vec<u32>, BareError> {
        expect_error("Failed to decode typed data from buffer", || {
            let mut buf: Vec<u32> = vec![0u32; count];

            // SAFETY: buf is always u8 aligned and all u32 bit patterns are valid u8
            #[allow(unsafe_code)]
            let (prefix, bytes, suffix) = unsafe { buf.align_to_mut::<u8>() };

            // bytes should be perfectly aligned
            assert!(prefix.is_empty() && suffix.is_empty());

            // MSRV: use read_buf_exact when available
            self.reader.read_exact(bytes)?;

            Ok(buf)
        })
    }
}

impl<W: Write> BareEncoder<W> {
    pub(crate) fn write_uint(&mut self, value: u64) -> Result<(), BareError> {
        expect_error("Failed to encode uint", || {
            let mut x = value;
            while x >= 0x80 {
                let b = (x as u8) | 0x80;
                self.writer.write_byte(b)?;
                x >>= 7;
            }
            self.writer.write_byte(x as u8)?;
            Ok(())
        })
    }
    pub(crate) fn write_u8(&mut self, value: u8) -> Result<(), BareError> {
        expect_error("Failed to encode u8", || {
            self.writer.write_byte(value)?;
            Ok(())
        })
    }
    pub(crate) fn write_u16(&mut self, value: u16) -> Result<(), BareError> {
        expect_error("Failed to encode u16", || {
            self.writer.write_all(&value.to_le_bytes())?;
            Ok(())
        })
    }
    pub(crate) fn write_u32(&mut self, value: u32) -> Result<(), BareError> {
        expect_error("Failed to encode u32", || {
            self.writer.write_all(&value.to_le_bytes())?;
            Ok(())
        })
    }
    pub(crate) fn write_u64(&mut self, value: u64) -> Result<(), BareError> {
        expect_error("Failed to encode u64", || {
            self.writer.write_all(&value.to_le_bytes())?;
            Ok(())
        })
    }
    pub(crate) fn write_data(&mut self, buf: &[u8]) -> Result<(), BareError> {
        expect_error("Failed to encode data", || {
            self.write_uint(buf.len() as u64)?;
            self.writer.write_all(buf)?;
            Ok(())
        })
    }
    pub(crate) fn write_data_exact(&mut self, buf: &[u8]) -> Result<(), BareError> {
        expect_error("Failed to encode data[length]", || {
            self.writer.write_all(buf)?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod test {
    use crate::bare::BareEncoder;

    use super::BareDecoder;
    use super::BareError;

    #[test]
    fn decode_uint() -> Result<(), BareError> {
        let buf = [
            0x00, // 0
            0x01, // 1
            0x7e, // 126
            0x7f, // 127
            0x80, 0x01, // 128
            0x81, 0x01, // 129
            0xff, 0x01, // 255,
        ];
        let mut d = BareDecoder::new(&buf[..]);
        assert_eq!(0, d.read_uint()?);
        assert_eq!(1, d.read_uint()?);
        assert_eq!(126, d.read_uint()?);
        assert_eq!(127, d.read_uint()?);
        assert_eq!(128, d.read_uint()?);
        assert_eq!(129, d.read_uint()?);
        assert_eq!(255, d.read_uint()?);
        Ok(())
    }
    #[test]
    fn encode_uint() -> Result<(), BareError> {
        let buf = [
            0x00, // 0
            0x01, // 1
            0x7e, // 126
            0x7f, // 127
            0x80, 0x01, // 128
            0x81, 0x01, // 129
            0xff, 0x01, // 255,
        ];
        let mut ob = vec![];
        let mut e = BareEncoder::new(&mut ob);
        e.write_uint(0)?;
        e.write_uint(1)?;
        e.write_uint(126)?;
        e.write_uint(127)?;
        e.write_uint(128)?;
        e.write_uint(129)?;
        e.write_uint(255)?;
        assert_eq!(&buf, ob.as_slice());
        Ok(())
    }
    #[test]
    fn decode_u8() -> Result<(), BareError> {
        let buf = [
            0x00, // 0
            0x01, // 1
            0xff, // 255,
        ];
        let mut d = BareDecoder::new(&buf[..]);
        assert_eq!(0, d.read_u8()?);
        assert_eq!(1, d.read_u8()?);
        assert_eq!(255, d.read_u8()?);
        Ok(())
    }
    #[test]
    fn encode_u8() -> Result<(), BareError> {
        let buf = [
            0x00, // 0
            0x01, // 1
            0xff, // 255,
        ];
        let mut ob = vec![];
        let mut e = BareEncoder::new(&mut ob);
        e.write_u8(0)?;
        e.write_u8(1)?;
        e.write_u8(255)?;
        assert_eq!(&buf, ob.as_slice());
        Ok(())
    }
    #[test]
    fn decode_u16() -> Result<(), BareError> {
        let buf = [
            0x00, 0x00, // 0
            0x01, 0x00, // 1
            0xff, 0x00, // 255,
        ];
        let mut d = BareDecoder::new(&buf[..]);
        assert_eq!(0, d.read_u16()?);
        assert_eq!(1, d.read_u16()?);
        assert_eq!(255, d.read_u16()?);
        Ok(())
    }
    #[test]
    fn encode_u16() -> Result<(), BareError> {
        let buf = [
            0x00, 0x00, // 0
            0x01, 0x00, // 1
            0xff, 0x00, // 255,
        ];
        let mut ob = vec![];
        let mut e = BareEncoder::new(&mut ob);
        e.write_u16(0)?;
        e.write_u16(1)?;
        e.write_u16(255)?;
        assert_eq!(&buf, ob.as_slice());
        Ok(())
    }
    #[test]
    fn decode_u32() -> Result<(), BareError> {
        let buf = [
            0x00, 0x00, 0x00, 0x00, // 0
            0x01, 0x00, 0x00, 0x00, // 1
            0xff, 0x00, 0x00, 0x00, // 255,
        ];
        let mut d = BareDecoder::new(&buf[..]);
        assert_eq!(0, d.read_u32()?);
        assert_eq!(1, d.read_u32()?);
        assert_eq!(255, d.read_u32()?);
        Ok(())
    }
    #[test]
    fn encode_u32() -> Result<(), BareError> {
        let buf = [
            0x00, 0x00, 0x00, 0x00, // 0
            0x01, 0x00, 0x00, 0x00, // 1
            0xff, 0x00, 0x00, 0x00, // 255,
        ];
        let mut ob = vec![];
        let mut e = BareEncoder::new(&mut ob);
        e.write_u32(0)?;
        e.write_u32(1)?;
        e.write_u32(255)?;
        assert_eq!(&buf, ob.as_slice());
        Ok(())
    }
    #[test]
    fn decode_u64() -> Result<(), BareError> {
        let buf = [
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // 0
            0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // 1
            0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // 255,
        ];
        let mut d = BareDecoder::new(&buf[..]);
        assert_eq!(0, d.read_u64()?);
        assert_eq!(1, d.read_u64()?);
        assert_eq!(255, d.read_u64()?);
        Ok(())
    }
    #[test]
    fn encode_u64() -> Result<(), BareError> {
        let buf = [
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // 0
            0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // 1
            0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // 255,
        ];
        let mut ob = vec![];
        let mut e = BareEncoder::new(&mut ob);
        e.write_u64(0)?;
        e.write_u64(1)?;
        e.write_u64(255)?;
        assert_eq!(&buf, ob.as_slice());
        Ok(())
    }
    #[test]
    fn decode_data() -> Result<(), BareError> {
        let buf = [
            0x10, 0xaa, 0xee, 0xff, 0xee, 0xdd, 0xcc, 0xbb, 0xaa, 0xee, 0xdd, 0xcc, 0xbb, 0xee,
            0xdd, 0xcc, 0xbb,
        ];
        let mut d = BareDecoder::new(&buf[..]);
        assert_eq!(
            vec![
                0xaa, 0xee, 0xff, 0xee, 0xdd, 0xcc, 0xbb, 0xaa, 0xee, 0xdd, 0xcc, 0xbb, 0xee, 0xdd,
                0xcc, 0xbb
            ],
            d.read_data()?
        );
        Ok(())
    }
    #[test]
    fn encode_data() -> Result<(), BareError> {
        let buf = [
            0xaa, 0xee, 0xff, 0xee, 0xdd, 0xcc, 0xbb, 0xaa, 0xee, 0xdd, 0xcc, 0xbb, 0xee, 0xdd,
            0xcc, 0xbb,
        ];
        let mut ob = vec![];
        let mut e = BareEncoder::new(&mut ob);
        e.write_data(&buf)?;
        assert_eq!(
            vec![
                0x10, 0xaa, 0xee, 0xff, 0xee, 0xdd, 0xcc, 0xbb, 0xaa, 0xee, 0xdd, 0xcc, 0xbb, 0xee,
                0xdd, 0xcc, 0xbb
            ],
            ob
        );
        Ok(())
    }
    #[test]
    fn decode_data_exact() -> Result<(), BareError> {
        let buf = [
            0xaa, 0xee, 0xff, 0xee, 0xdd, 0xcc, 0xbb, 0xaa, 0xee, 0xdd, 0xcc, 0xbb, 0xee, 0xdd,
            0xcc, 0xbb,
        ];
        let mut d = BareDecoder::new(&buf[..]);
        assert_eq!(
            vec![
                0xaa, 0xee, 0xff, 0xee, 0xdd, 0xcc, 0xbb, 0xaa, 0xee, 0xdd, 0xcc, 0xbb, 0xee, 0xdd,
                0xcc, 0xbb
            ],
            d.read_data_exact(0x10)?
        );
        Ok(())
    }
    #[test]
    fn encode_data_exact() -> Result<(), BareError> {
        let buf = [
            0xaa, 0xee, 0xff, 0xee, 0xdd, 0xcc, 0xbb, 0xaa, 0xee, 0xdd, 0xcc, 0xbb, 0xee, 0xdd,
            0xcc, 0xbb,
        ];
        let mut ob = vec![];
        let mut e = BareEncoder::new(&mut ob);
        e.write_data_exact(&buf)?;
        assert_eq!(
            vec![
                0xaa, 0xee, 0xff, 0xee, 0xdd, 0xcc, 0xbb, 0xaa, 0xee, 0xdd, 0xcc, 0xbb, 0xee, 0xdd,
                0xcc, 0xbb
            ],
            ob
        );
        Ok(())
    }
}
