//! Bigram + unigram in compressed sparse row format
//!
//! Ref: <https://en.wikipedia.org/wiki/Sparse_matrix#Compressed_sparse_row_(CSR,_CRS_or_Yale_format)>

use std::{
    collections::BTreeMap,
    io::{Read, Write},
    ops::Neg,
};

use log::warn;
use scoped_error::{bail, expect_error, impl_context_error};

use crate::{
    bare::{BareDecoder, BareEncoder},
    model::WordId,
};

#[derive(Debug)]
pub struct StaticLm {
    row_index: Box<[u8]>,
    col_index: Box<[u8]>,
    values: Box<[u8]>,
}

impl StaticLm {
    pub fn from_reader<R>(reader: R) -> Result<StaticLm, StaticLmError>
    where
        R: Read,
    {
        expect_error("Failed to read static language model", || {
            let mut decoder = BareDecoder::new(reader);

            // Read file magic
            let magic = decoder.read_data_exact(4)?;
            if magic != b"CHLM" {
                bail!("Unknown file format");
            }
            // Read file version
            let version = decoder.read_uint()?;
            if version != 0 {
                bail!("Unknown file version");
            }
            // Read header flags
            let flags = decoder.read_u32()?;
            if flags != 0 {
                warn!("Unknown StaticLm flags {:x}", flags);
            }
            let num_rows = decoder.read_u32()?;
            let num_values = decoder.read_u64()?;
            // Read data
            let row_index = decoder
                .read_data_exact((num_rows + 1) as usize * size_of::<u32>())?
                .into_boxed_slice();
            let col_index = decoder
                .read_data_exact(num_values as usize * size_of::<u32>())?
                .into_boxed_slice();
            let values = decoder
                .read_data_exact(num_values as usize)?
                .into_boxed_slice();

            Ok(StaticLm {
                values,
                col_index,
                row_index,
            })
        })
    }
    #[allow(unsafe_code)]
    fn view(&self) -> (&[u32], &[u32], &[u8]) {
        // SAFETY: it's safe to transmute [u8; 4] to u32
        let (_, row_index, _) = unsafe { self.row_index.align_to::<u32>() };
        // SAFETY: it's safe to transmute [u8; 4] to u32
        let (_, col_index, _) = unsafe { self.col_index.align_to::<u32>() };
        (row_index, col_index, &self.values)
    }
    pub fn get(&self, row: u32, col: u32) -> Option<f64> {
        let (row_index, col_index, values) = self.view();
        let row_start = *row_index.get(row as usize)? as usize;
        let row_end = *row_index.get(row as usize + 1)? as usize;
        let cols = &col_index[row_start..row_end];
        let vals = &values[row_start..row_end];
        cols.iter()
            .position(|c| *c == col)
            .map(|pos| vals[pos])
            .map(|q| unquantize_log_prob(q))
    }
}

#[derive(Debug)]
pub struct StaticLmCompiler {
    matrix: BTreeMap<(WordId, WordId), f64>,
    rows: u32,
    unigram_len: u64,
    bigram_len: u64,
}

impl StaticLmCompiler {
    pub fn new() -> StaticLmCompiler {
        StaticLmCompiler {
            matrix: BTreeMap::new(),
            rows: 0,
            unigram_len: 0,
            bigram_len: 0,
        }
    }
    pub fn insert(&mut self, row: WordId, col: WordId, value: f64) -> Result<(), StaticLmError> {
        expect_error("Failed to compile language model", || {
            if let Some(_) = self.matrix.insert((row, col), value) {
                bail!("Multiple entries for ({}, {})", row, col);
            }
            self.rows = self.rows.max(row.0 + 1);
            if row.0 == 0 {
                self.unigram_len += 1;
            } else {
                self.bigram_len += 1;
            }
            Ok(())
        })
    }
    pub fn to_writer<W>(&self, writer: W) -> Result<(), StaticLmError>
    where
        W: Write,
    {
        expect_error("Failed to serialize StaticLm", || {
            let mut encoder = BareEncoder::new(writer);

            let q_matrix: BTreeMap<(u32, u32), u8> = self
                .matrix
                .iter()
                .filter_map(|(&k, &log10_prob)| {
                    let quantized = quantize_log_prob(log10_prob);
                    if quantized == 0 {
                        None
                    } else {
                        Some(((k.0.0, k.1.0), quantized))
                    }
                })
                .collect();

            // Write file magic
            encoder.write_data_exact(b"CHLM")?;
            // Write version
            encoder.write_uint(0)?;
            // Write header flags
            encoder.write_u32(0)?;
            // Write num_rows
            encoder.write_u32(self.rows)?;
            // Write num_values
            encoder.write_u64(q_matrix.len() as u64)?;

            // Write row index
            let mut offset = 0;
            let mut current_row = 0;
            for (row, _) in q_matrix.keys() {
                if *row == current_row {
                    encoder.write_u32(offset)?;
                    current_row += 1;
                }
                offset += 1;
            }
            encoder.write_u32(offset)?;
            // Write col index
            for (_, col) in q_matrix.keys() {
                encoder.write_u32(*col)?;
            }
            // Write quantized values
            for value in q_matrix.values() {
                encoder.write_u8(*value)?;
            }

            Ok(())
        })
    }
}

const MIN_LOGLOG: f64 = -0.3;
const MAX_LOGLOG: f64 = 1.3;

pub(crate) fn quantize_log_prob(log10prob: f64) -> u8 {
    let loglog = log10prob.neg().log10().clamp(MIN_LOGLOG, MAX_LOGLOG);
    let quantized = ((loglog - MIN_LOGLOG) / (MAX_LOGLOG - MIN_LOGLOG) * 255.0) as u8;
    quantized
}

pub(crate) fn unquantize_log_prob(quantum: u8) -> f64 {
    let loglog = (quantum as f64) / 255.0 * (MAX_LOGLOG - MIN_LOGLOG) + MIN_LOGLOG;
    10.0_f64.powf(loglog).neg()
}

impl_context_error!(pub StaticLmError);

#[cfg(test)]
mod test {
    use std::{error::Error, ops::Sub};

    use crate::{
        lm::{
            StaticLmCompiler,
            static_lm::{StaticLm, quantize_log_prob, unquantize_log_prob},
        },
        model::WordId,
    };

    #[test]
    fn quantize_unquantize() {
        let e = 0.6;
        assert!((0.0 - unquantize_log_prob(quantize_log_prob(0.0))).abs() < e);
        assert!((-1.0 - unquantize_log_prob(quantize_log_prob(-1.0))).abs() < e);
        assert!((-5.0 - unquantize_log_prob(quantize_log_prob(-5.0))).abs() < e);
        assert!((-10.0 - unquantize_log_prob(quantize_log_prob(-10.0))).abs() < e);
    }

    #[test]
    fn compile_static_lm() -> Result<(), Box<dyn Error>> {
        let mut compiler = StaticLmCompiler::new();
        let mut buf: Vec<u8> = vec![];
        compiler.insert(WordId(0), WordId(0), 0.01_f64.log10())?;
        compiler.insert(WordId(1), WordId(1), 0.02_f64.log10())?;
        compiler.insert(WordId(2), WordId(2), 0.03_f64.log10())?;
        compiler.insert(WordId(3), WordId(1), 0.04_f64.log10())?;
        compiler.insert(WordId(3), WordId(2), 0.05_f64.log10())?;
        compiler.to_writer(&mut buf).unwrap();

        assert_eq!(
            &[
                b'C', b'H', b'L', b'M', 0, 0, 0, 0, 0, 4, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0,
                0, 1, 0, 0, 0, 2, 0, 0, 0, 95, 84, 76, 70, 66
            ][..],
            &buf
        );
        Ok(())
    }

    #[test]
    fn read_static_lm() {
        let lm = &[
            b'C', b'H', b'L', b'M', 0, 0, 0, 0, 0, 4, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 1,
            0, 0, 0, 220, 231, 209, 225,
        ][..];
        let static_lm = StaticLm::from_reader(lm).unwrap();

        assert!(static_lm.get(0, 0).unwrap().sub(-12.0335).abs() < 1e-3);
        assert!(static_lm.get(1, 1).unwrap().sub(-14.1062).abs() < 1e-3);
        assert!(static_lm.get(2, 2).unwrap().sub(-10.2653).abs() < 1e-3);
        assert!(static_lm.get(3, 1).unwrap().sub(-12.9349).abs() < 1e-3);
        assert_eq!(None, static_lm.get(2, 1));
    }
}
