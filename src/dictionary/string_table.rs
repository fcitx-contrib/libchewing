use std::{
    collections::BTreeMap,
    fmt::Debug,
    fs::File,
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    sync::{Arc, RwLock},
};

use scoped_error::{bail, expect_error, impl_context_error};

use crate::{
    bare::{BareDecoder, BareEncoder},
    model::{WordId, WordOrig},
};

/// Fast and compact indexing of LF delimited strings
#[derive(Clone)]
pub struct StringTable {
    inner_ro: Arc<StringTableInnerRo>,
    inner_mut: Arc<RwLock<StringTableInnerMut>>,
}

struct StringTableInnerRo {
    chd: Chd,
    buffer: Box<str>,
    offset: Box<[u32]>,
}

struct StringTableInnerMut {
    vec: Vec<String>,
    map: BTreeMap<String, u32>,
}

impl Debug for StringTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        struct IntList<'a, T>(&'a [T]);
        impl<T> Debug for IntList<'_, T>
        where
            T: Debug,
        {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                let end = self.0.len().min(5);
                f.debug_list()
                    .entries(&self.0[..end])
                    .finish_non_exhaustive()
            }
        }
        let ro = self.inner_ro.as_ref();
        let end = ro.buffer.len().min(100);
        let buffer_prefix = format!("{}...", &ro.buffer[..end]);
        f.debug_struct("StringTable")
            .field("buffer", &buffer_prefix)
            .field("offset", &IntList(&ro.offset))
            .finish()
    }
}

impl StringTable {
    /// Creates an empty StringTable
    pub fn new() -> StringTable {
        StringTable {
            inner_ro: Arc::new(StringTableInnerRo {
                chd: Chd::new(),
                buffer: String::new().into_boxed_str(),
                offset: vec![].into_boxed_slice(),
            }),
            inner_mut: Arc::new(RwLock::new(StringTableInnerMut {
                vec: vec![],
                map: BTreeMap::new(),
            })),
        }
    }
    /// Reads strings from a binary string table file and constructs a StringTable
    pub fn open_bin<P: AsRef<Path>>(path: P) -> Result<StringTable, StringTableError> {
        expect_error("Failed to open string table", || {
            Ok(Self::from_reader(BufReader::new(File::open(path)?))?)
        })
    }
    /// Reads strings from a txt file and constructs a StringTable
    pub fn open_txt<P: AsRef<Path>>(path: P) -> Result<StringTable, StringTableError> {
        expect_error("Failed to open string table", || {
            let reader = BufReader::new(File::open(path)?);
            let mut builder = StringTableBuilder::new();
            for io in reader.lines() {
                let line = io?;
                builder.insert(line.trim());
            }
            Ok(builder.build())
        })
    }
    /// Parses the buffer and constructs a StringTable
    pub fn from_reader<T>(reader: T) -> Result<StringTable, StringTableError>
    where
        T: Read,
    {
        expect_error("Failed to read string table", || {
            let mut decoder = BareDecoder::new(reader);
            let magic = decoder.read_data_exact(4)?;
            if magic != b"CHSW" {
                bail!("Invalid file header");
            }
            let version = decoder.read_uint()?;
            if version != 0 {
                bail!("Unknown file version");
            }
            let flags = decoder.read_u32()?;
            if flags != 0 {
                bail!("Unknown flags");
            }
            let num_keys = decoder.read_u32()? as usize;
            let num_buckets = decoder.read_u32()? as usize;
            let seed = decoder.read_u32()?;
            let displacements_len = decoder.read_uint()?;
            let mut displacements = Vec::with_capacity(displacements_len as usize);
            for _ in 0..displacements_len {
                displacements.push(decoder.read_u16()?);
            }
            let chd = Chd {
                num_keys,
                num_buckets,
                displacements,
                seed,
            };
            let raw_buffer = decoder.read_data()?;
            let buffer = String::from_utf8(raw_buffer)?;
            let buffer = buffer.into_boxed_str();
            let bob = buffer.as_ptr() as usize;
            let mut offset = vec![];
            for line in buffer.lines() {
                offset.push((line.as_ptr() as usize - bob) as u32);
            }
            let offset = offset.into_boxed_slice();
            Ok(StringTable {
                inner_ro: Arc::new(StringTableInnerRo {
                    chd,
                    buffer,
                    offset,
                }),
                inner_mut: Arc::new(RwLock::new(StringTableInnerMut {
                    vec: vec![],
                    map: BTreeMap::new(),
                })),
            })
        })
    }
    /// Returns the number of strings in the table
    pub fn len(&self) -> usize {
        let ro = self.inner_ro.as_ref();
        let lock = self.inner_mut.read().expect("StringTable lock posioned");
        lock.vec.len() + ro.offset.len()
    }
    pub fn intern(&self, word: &str) -> WordId {
        // check existing mapping
        if let Some(wid) = self.get_wid(word) {
            return wid;
        }
        let mut lock = self.inner_mut.write().expect("StringTable lock posioned");
        let wid = WordId::MIN_USER + lock.vec.len() as u32;
        lock.vec.push(word.to_owned());
        lock.map.insert(word.to_owned(), wid);
        WordId(wid)
    }
    pub fn get_wid(&self, word: &str) -> Option<WordId> {
        let ro = self.inner_ro.as_ref();
        if !ro.chd.is_empty() {
            let pos = ro.chd.lookup(word);
            if let Some(w) = self.get_text(WordId(pos as u32))
                && w == word
            {
                return Some(WordId(pos as u32));
            }
        }
        let lock = self.inner_mut.read().expect("StringTable lock posioned");
        lock.map.get(word).map(|wid| WordId(*wid))
    }
    /// Returns the index-th string in the table as &str
    pub fn get_text(&self, wid: WordId) -> Option<String> {
        match wid.orig() {
            WordOrig::Static => {
                let ro = self.inner_ro.as_ref();
                let offset = ro.offset.get(wid.0 as usize).map(|o| *o as usize)?;
                let offset_1 = ro
                    .offset
                    .get(wid.0 as usize + 1)
                    .map(|o| *o as usize)
                    .unwrap_or(ro.buffer.len());
                let s = offset;
                let e = offset_1;
                Some(ro.buffer[s..e].trim_ascii_end().to_owned())
            }
            WordOrig::User => {
                let lock = self.inner_mut.read().expect("StringTable lock posioned");
                let offset = wid.as_offset();
                lock.vec.get(offset).map(|s| s.to_owned())
            }
            _ => panic!("unsupported"),
        }
    }
}

#[derive(Debug)]
pub struct StringTableBuilder {
    words: Vec<String>,
}

impl StringTableBuilder {
    pub fn new() -> StringTableBuilder {
        Self { words: vec![] }
    }
    pub fn to_writer<T>(&self, writer: T) -> Result<(), StringTableError>
    where
        T: Write,
    {
        expect_error("Failed to serialize StaticDict", || {
            let chd = Chd::build(&self.words, 4);
            let mut words = self.words.clone();
            words.sort_unstable_by_key(|s| chd.lookup(s));

            let mut encoder = BareEncoder::new(writer);
            // Write magic
            encoder.write_data_exact(b"CHSW")?;
            // Write file version
            encoder.write_uint(0)?;
            // Write flags
            encoder.write_u32(0)?;
            // Write num_keys
            encoder.write_u32(chd.num_keys as u32)?;
            // Write num_buckets
            encoder.write_u32(chd.num_buckets as u32)?;
            // Write seed
            encoder.write_u32(chd.seed)?;
            // Write displacements
            encoder.write_uint(chd.displacements.len() as u64)?;
            for d in &chd.displacements {
                encoder.write_u16(*d)?;
            }
            // Write words
            let size: usize = words.iter().map(|s| s.len() + 1).sum();
            encoder.write_uint(size as u64)?;
            for s in &words {
                encoder.write_data_exact(s.as_bytes())?;
                encoder.write_u8(b'\n')?;
            }

            Ok(())
        })
    }

    pub fn build(self) -> StringTable {
        let mut buf = vec![];
        self.to_writer(&mut buf)
            .expect("Failed to serialize in-memory StringTable");
        StringTable::from_reader(buf.as_slice()).expect("Failed to build im-memory StringTable")
    }

    pub fn insert(&mut self, word: &str) {
        self.words.push(word.to_owned());
    }
}

// 64-bit FNV1a hash, returns the hash in two halfs
#[inline]
fn fnv1a_64(bytes: &[u8], seed: u64) -> (u32, u32) {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET ^ seed;
    for &b in bytes {
        hash = (hash ^ (b as u64)).wrapping_mul(FNV_PRIME);
    }
    ((hash >> 32) as u32, hash as u32)
}

// CHD perfect hash table, but without compression
//
// https://cmph.sourceforge.net/chd.html
// https://cmph.sourceforge.net/papers/esa09.pdf
#[derive(Debug)]
pub(crate) struct Chd {
    num_keys: usize,
    num_buckets: usize,
    displacements: Vec<u16>,
    seed: u32,
}

struct KeyHash {
    h1: u32,
    h2: u32,
}

struct Bucket {
    id: usize,
    keys: Vec<KeyHash>,
}

impl Chd {
    pub(crate) fn new() -> Self {
        Chd {
            num_keys: 0,
            num_buckets: 0,
            displacements: vec![],
            seed: 0,
        }
    }
    /// Builds a CHD minimal perfect hash table.
    /// `avg_bucket_size` of 4 or 5 typically yields very small displacements.
    pub(crate) fn build<T>(keys: &[T], avg_bucket_size: usize) -> Self
    where
        T: AsRef<str>,
    {
        let num_keys = keys.len();
        let num_buckets = usize::max(1, (num_keys + avg_bucket_size - 1) / avg_bucket_size);
        let mut seed = 0u32;

        loop {
            if let Some(displacements) = Self::try_build(keys, num_buckets, seed) {
                return Self {
                    num_keys,
                    num_buckets,
                    displacements,
                    seed,
                };
            }
            seed = seed.wrapping_add(1);
        }
    }

    fn try_build<T>(keys: &[T], num_buckets: usize, seed: u32) -> Option<Vec<u16>>
    where
        T: AsRef<str>,
    {
        let num_keys = keys.len();
        let mut buckets: Vec<Bucket> = (0..num_buckets)
            .map(|id| Bucket {
                id,
                keys: Vec::new(),
            })
            .collect();

        // Phase 1: Partition keys into buckets using h1
        for key in keys {
            let (h1, mut h2) = fnv1a_64(key.as_ref().as_bytes(), seed as u64);
            // h2 must be non-zero (or odd) so step size is valid
            if h2 == 0 || h2 % 2 == 0 {
                h2 = h2.wrapping_add(1);
            }
            let b_idx = (h1 as usize) % num_buckets;
            buckets[b_idx].keys.push(KeyHash { h1, h2 });
        }

        // Sort buckets descending by size (largest buckets placed first)
        buckets.sort_by(|a, b| b.keys.len().cmp(&a.keys.len()));

        let mut displacements = vec![0u16; num_buckets];
        let mut occupied = vec![false; num_keys];
        let mut slots_in_use = Vec::with_capacity(num_keys);

        // Phase 2: Displace each bucket into empty slots
        for bucket in &buckets {
            if bucket.keys.is_empty() {
                continue;
            }

            let mut d: u32 = 0;
            'search: loop {
                if d > u16::MAX as u32 {
                    // Displacement overflowed 16 bits; retry with new seed
                    return None;
                }

                slots_in_use.clear();
                for k in &bucket.keys {
                    let slot = (k.h1.wrapping_add(d.wrapping_mul(k.h2)) as usize) % num_keys;
                    if occupied[slot] || slots_in_use.contains(&slot) {
                        d += 1;
                        continue 'search;
                    }
                    slots_in_use.push(slot);
                }

                // Found a valid displacement for all keys in this bucket
                displacements[bucket.id] = d as u16;
                for &slot in &slots_in_use {
                    occupied[slot] = true;
                }
                break;
            }
        }

        Some(displacements)
    }

    /// Looks up a key and returns its unique index in `0..num_keys`.
    #[inline]
    pub(crate) fn lookup(&self, key: &str) -> usize {
        let (h1, mut h2) = fnv1a_64(key.as_bytes(), self.seed as u64);
        if h2 == 0 || h2 % 2 == 0 {
            h2 = h2.wrapping_add(1);
        }
        let b_idx = (h1 as usize) % self.num_buckets;
        let d = self.displacements[b_idx] as u32;
        (h1.wrapping_add(d.wrapping_mul(h2)) as usize) % self.num_keys
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.num_keys == 0
    }
}

impl_context_error!(pub StringTableError);

#[cfg(test)]
mod test {
    use super::StringTable;
    use crate::{dictionary::StringTableBuilder, model::WordId};

    #[test]
    fn empty_buffer() {
        let st = StringTable::new();
        assert_eq!(0, st.len());
        assert_eq!(None, st.get_text(0.into()));
        assert_eq!(None, st.get_text(WordId(100)));
    }
    #[test]
    fn oneline() {
        let mut builder = StringTableBuilder::new();
        builder.insert("test");
        let st = builder.build();
        assert_eq!(1, st.len());
        assert_eq!("test", st.get_text(0.into()).unwrap());
        assert_eq!(None, st.get_text(WordId(100)));
    }
    #[test]
    fn multi_lines() {
        let mut builder = StringTableBuilder::new();
        builder.insert("test");
        builder.insert("line2");
        builder.insert("line3");
        let st = builder.build();
        assert_eq!(3, st.len());
        assert_eq!("test", st.get_text(0.into()).unwrap());
        assert_eq!("line3", st.get_text(st.get_wid("line3").unwrap()).unwrap());
        assert_eq!(None, st.get_text(WordId(100)));
    }
}
