//! Bigram + unigram in delta-encoded compressed sparse row (CSR) format
//!
//! Column indexes are delta-encoded within each row and stored as LEB128
//! varints on disk. At load time, the caller can choose between:
//!
//! - **Eager mode**: decode all varints into a flat `u32` array for O(log n)
//!   binary search per row. Best for high-throughput lookups (training).
//! - **Lazy mode**: keep the raw varint blob and decode per-row on `get()`.
//!   Best for memory-constrained or interactive use.

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

/// Controls how column indexes are stored in memory after loading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadMode {
    /// Decode all varints at load time into a flat `u32` array.
    /// Enables O(log n) binary search per row.
    /// Uses 4 times more memory for the column index, but lookups are fast.
    Eager,

    /// Keep the raw LEB128 varint blob and decode per-row on `get()`.
    /// Minimal memory footprint, but slower per lookup.
    Lazy,
}

fn encode_varint(mut value: u32, buf: &mut Vec<u8>) {
    while value >= 0x80 {
        buf.push((value as u8) | 0x80);
        value >>= 7;
    }
    buf.push(value as u8);
}

/// Decode a varint starting at `offset` in `data`.
/// Returns `(decoded_value, bytes_consumed)`.
fn decode_varint(data: &[u8], offset: usize) -> (u32, usize) {
    let mut result: u32 = 0;
    let mut shift: u32 = 0;
    let mut pos = offset;
    loop {
        let byte = data[pos];
        pos += 1;
        result |= ((byte & 0x7F) as u32) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
        debug_assert!(shift < 35, "varint overflow");
    }
    (result, pos - offset)
}

// TODO: implement Clone by Arc
// TODO: always use direct array for unigram so Lazy mode is fast.
#[derive(Debug)]
pub struct StaticLm {
    /// Cumulative entry counts: row_index[i+1] - row_index[i] = nnz in row i.
    row_index: Box<[u32]>,

    /// Quantized log-probabilities, one per entry.
    values: Box<[u8]>,

    /// Column storage - either decoded or raw varints depending on load mode.
    cols: ColStorage,
}

#[derive(Debug)]
enum ColStorage {
    /// Decoded absolute column IDs as a flat `u32` array.
    /// Enables O(log n) binary search per row.
    Decoded(Box<[u32]>),

    /// Raw LEB128 varint blob + byte offset table for per-row decoding.
    Varint {
        /// Byte offsets into `col_deltas` for each row boundary.
        row_byte_offsets: Box<[u32]>,
        /// LEB128-encoded column deltas (first entry per row is absolute column).
        col_deltas: Box<[u8]>,
    },
}

impl StaticLm {
    pub fn new() -> StaticLm {
        StaticLm {
            row_index: Box::new([]),
            values: Box::new([]),
            cols: ColStorage::Decoded(Box::new([])),
        }
    }

    pub fn from_reader<R>(reader: R, mode: LoadMode) -> Result<StaticLm, StaticLmError>
    where
        R: Read,
    {
        expect_error("Failed to read static language model", || {
            let mut decoder = BareDecoder::new(reader);

            let magic = decoder.read_data_exact(4)?;
            if magic != b"CHLM" {
                bail!("Unknown file format");
            }

            let version = decoder.read_uint()?;
            if version != 0 {
                bail!("Unknown file version {}", version);
            }

            let flags = decoder.read_u32()?;
            if flags != 0 {
                warn!("Unknown StaticLm flags {:x}", flags);
            }

            let num_rows = decoder.read_u32()?;
            let num_values = decoder.read_u64()?;
            let col_deltas_len = decoder.read_u64()?;

            let row_index: Box<[u32]> = decoder
                .read_list_u32_exact((num_rows + 1) as usize)?
                .into_boxed_slice();

            let row_byte_offsets: Box<[u32]> = decoder
                .read_list_u32_exact((num_rows + 1) as usize)?
                .into_boxed_slice();

            let col_deltas = decoder
                .read_data_exact(col_deltas_len as usize)?
                .into_boxed_slice();

            let values = decoder
                .read_data_exact(num_values as usize)?
                .into_boxed_slice();

            // Build column storage based on load mode
            let cols = match mode {
                LoadMode::Eager => {
                    let decoded = decode_all_columns(
                        &row_index,
                        &row_byte_offsets,
                        &col_deltas,
                        num_rows,
                        num_values,
                    );
                    ColStorage::Decoded(decoded.into_boxed_slice())
                }
                LoadMode::Lazy => ColStorage::Varint {
                    row_byte_offsets,
                    col_deltas,
                },
            };

            Ok(StaticLm {
                row_index,
                values,
                cols,
            })
        })
    }

    pub fn get(&self, row: u32, col: u32) -> Option<f64> {
        match &self.cols {
            ColStorage::Decoded(decoded) => self.get_eager(row, col, decoded),
            ColStorage::Varint {
                row_byte_offsets,
                col_deltas,
            } => self.get_lazy(row, col, row_byte_offsets, col_deltas),
        }
    }

    /// Eager lookup: binary search on decoded `&[u32]` columns.
    fn get_eager(&self, row: u32, col: u32, decoded: &[u32]) -> Option<f64> {
        let row_start = *self.row_index.get(row as usize)? as usize;
        let row_end = *self.row_index.get(row as usize + 1)? as usize;

        let cols = &decoded[row_start..row_end];
        let vals = &self.values[row_start..row_end];

        cols.binary_search(&col)
            .ok()
            .map(|pos| vals[pos])
            .map(|q| unquantize_log_prob(q))
    }

    /// Lazy lookup: decode varints sequentially within the row.
    fn get_lazy(
        &self,
        row: u32,
        col: u32,
        row_byte_offsets: &[u32],
        col_deltas: &[u8],
    ) -> Option<f64> {
        let entry_start = *self.row_index.get(row as usize)? as usize;
        let entry_end = *self.row_index.get(row as usize + 1)? as usize;
        if entry_start == entry_end {
            return None;
        }

        let byte_start = *row_byte_offsets.get(row as usize)? as usize;
        let byte_end = *row_byte_offsets.get(row as usize + 1)? as usize;

        let mut offset = byte_start;
        let mut abs_col: u32 = 0;
        let mut entry_idx = entry_start;

        while offset < byte_end && entry_idx < entry_end {
            let (delta, consumed) = decode_varint(col_deltas, offset);
            abs_col = abs_col.wrapping_add(delta);
            offset += consumed;

            if abs_col == col {
                return Some(unquantize_log_prob(self.values[entry_idx]));
            }
            // Since columns are sorted, we can stop early
            if abs_col > col {
                return None;
            }

            entry_idx += 1;
        }

        None
    }
}

/// Decode all column deltas into absolute column IDs.
///
/// Uses `row_byte_offsets` to detect row boundaries (where the delta
/// accumulator resets) and `row_index` to know how many entries each row has.
#[allow(unsafe_code)]
fn decode_all_columns(
    row_index: &[u32],
    row_byte_offsets: &[u32],
    col_deltas: &[u8],
    num_rows: u32,
    num_values: u64,
) -> Vec<u32> {
    let mut result: Vec<u32> = Vec::with_capacity(num_values as usize);
    let mut byte_pos: usize = 0;

    for row in 0..num_rows as usize {
        let entry_start = row_index[row] as usize;
        let entry_end = row_index[row + 1] as usize;
        let row_byte_start = row_byte_offsets[row] as usize;
        let row_byte_end = row_byte_offsets[row + 1] as usize;

        debug_assert_eq!(byte_pos, row_byte_start);

        let mut abs_col: u32 = 0;
        for _ in entry_start..entry_end {
            let (delta, consumed) = decode_varint(col_deltas, byte_pos);
            abs_col = abs_col.wrapping_add(delta);
            result.push(abs_col);
            byte_pos += consumed;
        }

        debug_assert_eq!(byte_pos, row_byte_end);
    }

    result
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
            if self.matrix.contains_key(&(row, col)) {
                eprintln!("Multiple entries for ({}, {})", row, col);
                return Ok(());
            }
            self.matrix.insert((row, col), value);
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

            // Build CSR arrays in a single pass.
            // Within each row, columns are ascending, so deltas are non-negative.

            let mut row_ptr = vec![0u32; (self.rows + 1) as usize];
            let mut row_byte_offsets = vec![0u32; (self.rows + 1) as usize];
            let mut col_deltas_buf: Vec<u8> = Vec::new();
            let mut values_buf: Vec<u8> = Vec::new();

            let mut entry_count: u32 = 0;
            let mut current_row: u32 = 0;
            let mut prev_col: Option<u32> = None;

            for (&(row, col), &val) in q_matrix.iter() {
                // Advance past any empty rows
                while current_row < row {
                    row_ptr[current_row as usize + 1] = entry_count;
                    row_byte_offsets[current_row as usize + 1] = col_deltas_buf.len() as u32;
                    current_row += 1;
                    prev_col = None;
                }

                // Delta-encode the column
                let delta = match prev_col {
                    None => col,              // first entry: absolute column
                    Some(prev) => col - prev, // subsequent: delta from previous
                };
                encode_varint(delta, &mut col_deltas_buf);
                values_buf.push(val);

                prev_col = Some(col);
                entry_count += 1;
            }

            // Fill remaining empty rows
            while current_row < self.rows {
                row_ptr[current_row as usize + 1] = entry_count;
                row_byte_offsets[current_row as usize + 1] = col_deltas_buf.len() as u32;
                current_row += 1;
            }
            row_byte_offsets[self.rows as usize] = col_deltas_buf.len() as u32;

            // Magic
            encoder.write_data_exact(b"CHLM")?;
            // Version 0
            encoder.write_uint(0)?;
            // Flags
            encoder.write_u32(0)?;
            // num_rows
            encoder.write_u32(self.rows)?;
            // num_values
            encoder.write_u64(entry_count as u64)?;
            // col_deltas byte length
            encoder.write_u64(col_deltas_buf.len() as u64)?;

            // row_index (cumulative entry counts)
            for &ptr in &row_ptr {
                encoder.write_u32(ptr)?;
            }
            // row_byte_offsets
            for &off in &row_byte_offsets {
                encoder.write_u32(off)?;
            }
            // col_deltas (varint stream)
            encoder.write_data_exact(&col_deltas_buf)?;
            // values
            encoder.write_data_exact(&values_buf)?;

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
    use crate::{
        lm::{
            StaticLmCompiler,
            static_lm::{LoadMode, StaticLm, quantize_log_prob, unquantize_log_prob},
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

    /// Helper: compile, serialize, deserialize in the given mode, and return the LM.
    fn roundtrip(mode: LoadMode) -> StaticLm {
        let mut compiler = StaticLmCompiler::new();
        compiler
            .insert(WordId(0), WordId(0), 0.01_f64.log10())
            .unwrap();
        compiler
            .insert(WordId(1), WordId(1), 0.02_f64.log10())
            .unwrap();
        compiler
            .insert(WordId(2), WordId(2), 0.03_f64.log10())
            .unwrap();
        compiler
            .insert(WordId(3), WordId(1), 0.04_f64.log10())
            .unwrap();
        compiler
            .insert(WordId(3), WordId(2), 0.05_f64.log10())
            .unwrap();

        let mut buf: Vec<u8> = vec![];
        compiler.to_writer(&mut buf).unwrap();

        StaticLm::from_reader(buf.as_slice(), mode).unwrap()
    }

    #[test]
    fn eager_mode_roundtrip() {
        let lm = roundtrip(LoadMode::Eager);

        let entries = [
            (0, 0, 0.01_f64.log10()),
            (1, 1, 0.02_f64.log10()),
            (2, 2, 0.03_f64.log10()),
            (3, 1, 0.04_f64.log10()),
            (3, 2, 0.05_f64.log10()),
        ];

        for &(row, col, expected_log10) in &entries {
            let got = lm
                .get(row, col)
                .expect(&format!("missing ({}, {})", row, col));
            let err = (got - expected_log10).abs();
            assert!(
                err < 0.1,
                "({},{}) expected ~{}, got {} (err={})",
                row,
                col,
                expected_log10,
                got,
                err
            );
        }

        assert_eq!(None, lm.get(2, 1));
        assert_eq!(None, lm.get(0, 3));
        assert_eq!(None, lm.get(5, 0));
    }

    #[test]
    fn lazy_mode_roundtrip() {
        let lm = roundtrip(LoadMode::Lazy);

        let entries = [
            (0, 0, 0.01_f64.log10()),
            (1, 1, 0.02_f64.log10()),
            (2, 2, 0.03_f64.log10()),
            (3, 1, 0.04_f64.log10()),
            (3, 2, 0.05_f64.log10()),
        ];

        for &(row, col, expected_log10) in &entries {
            let got = lm
                .get(row, col)
                .expect(&format!("missing ({}, {})", row, col));
            let err = (got - expected_log10).abs();
            assert!(
                err < 0.1,
                "({},{}) expected ~{}, got {} (err={})",
                row,
                col,
                expected_log10,
                got,
                err
            );
        }

        assert_eq!(None, lm.get(2, 1));
        assert_eq!(None, lm.get(0, 3));
        assert_eq!(None, lm.get(5, 0));
    }

    #[test]
    fn eager_and_lazy_agree() {
        // Both modes should return identical results for the same queries
        let eager = roundtrip(LoadMode::Eager);
        let lazy = roundtrip(LoadMode::Lazy);

        // Check all existing entries
        for row in 0..4 {
            for col in 0..4 {
                assert_eq!(
                    eager.get(row, col),
                    lazy.get(row, col),
                    "mismatch at ({}, {})",
                    row,
                    col
                );
            }
        }

        // Check some non-existing entries
        assert_eq!(eager.get(0, 1), lazy.get(0, 1));
        assert_eq!(eager.get(2, 0), lazy.get(2, 0));
        assert_eq!(eager.get(100, 0), lazy.get(100, 0));
    }

    #[test]
    fn delta_encoding_correctness() {
        let mut compiler = StaticLmCompiler::new();

        // Row 10: columns [100, 105, 200, 201, 5000]
        // Deltas:  [100,   5,  95,   1, 4799]
        for &col in &[100u32, 105, 200, 201, 5000] {
            compiler.insert(WordId(10), WordId(col), -2.0).unwrap();
        }

        let mut buf: Vec<u8> = vec![];
        compiler.to_writer(&mut buf).unwrap();

        // Test both modes
        for mode in [LoadMode::Eager, LoadMode::Lazy] {
            let lm = StaticLm::from_reader(buf.as_slice(), mode).unwrap();

            for &col in &[100u32, 105, 200, 201, 5000] {
                assert!(
                    lm.get(10, col).is_some(),
                    "missing column {} in {:?} mode",
                    col,
                    mode
                );
            }

            assert!(lm.get(10, 99).is_none());
            assert!(lm.get(10, 101).is_none());
            assert!(lm.get(10, 5001).is_none());
        }
    }

    #[test]
    fn dcsr_compression_columns() {
        let mut compiler = StaticLmCompiler::new();

        // Row 0: 1000 unigram entries (columns 0..1000)
        for col in 0..1000u32 {
            compiler.insert(WordId(0), WordId(col), -3.0).unwrap();
        }

        // Row 1: 500 bigram entries with clustered columns
        for col in (1000..2000u32).step_by(2) {
            compiler.insert(WordId(1), WordId(col), -4.0).unwrap();
        }

        let mut buf: Vec<u8> = vec![];
        compiler.to_writer(&mut buf).unwrap();

        for mode in [LoadMode::Eager, LoadMode::Lazy] {
            let lm = StaticLm::from_reader(buf.as_slice(), mode).unwrap();

            assert!(lm.get(0, 0).is_some(), "mode {:?}", mode);
            assert!(lm.get(0, 999).is_some(), "mode {:?}", mode);
            assert!(lm.get(1, 1000).is_some(), "mode {:?}", mode);
            assert!(lm.get(1, 1998).is_some(), "mode {:?}", mode);
            assert!(lm.get(1, 1001).is_none(), "mode {:?}", mode);
        }
    }
}
