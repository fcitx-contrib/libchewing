use std::{
    collections::BTreeMap,
    fmt::Debug,
    fs, io,
    path::Path,
    sync::{Arc, RwLock},
};

use crate::model::{WordId, WordOrig};

/// Fast and compact indexing of LF delimited strings
#[derive(Clone)]
pub struct StringTable {
    inner: Arc<RwLock<StringTableInner>>,
}

struct StringTableInner {
    buffer: Box<str>,
    offset: Box<[u32]>,
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
        let lock = self.inner.read().expect("Failed to acquire reader lock");
        let end = lock.buffer.len().min(100);
        let buffer_prefix = format!("{}...", &lock.buffer[..end]);
        f.debug_struct("StringTable")
            .field("buffer", &buffer_prefix)
            .field("offset", &IntList(&lock.offset))
            .finish()
    }
}

impl StringTable {
    /// Creates an empty StringTable
    pub fn new() -> StringTable {
        StringTable {
            inner: Arc::new(RwLock::new(StringTableInner {
                buffer: String::new().into_boxed_str(),
                offset: vec![].into_boxed_slice(),
                vec: vec![],
                map: BTreeMap::new(),
            })),
        }
    }
    /// Reads strings from a file and constructs a StringTable
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<StringTable> {
        let buffer = fs::read_to_string(path)?;
        Ok(Self::from_string(buffer))
    }
    /// Parses the lines in a buffer and constructs a StringTable
    pub fn from_string(buffer: String) -> StringTable {
        let buffer = buffer.into_boxed_str();
        let bob = buffer.as_ptr() as usize;
        let mut offset = vec![];
        let mut map = BTreeMap::new();
        for line in buffer.lines() {
            offset.push((line.as_ptr() as usize - bob) as u32);
            map.insert(line.to_owned(), offset.len() as u32);
        }
        let offset = offset.into_boxed_slice();
        StringTable {
            inner: Arc::new(RwLock::new(StringTableInner {
                buffer,
                offset,
                vec: vec![],
                map,
            })),
        }
    }
    /// Returns the number of strings in the table
    pub fn len(&self) -> usize {
        let lock = self.inner.read().expect("StringTable lock posioned");
        lock.vec.len() + lock.offset.len()
    }
    pub fn intern(&self, word: &str) -> WordId {
        // check existing mapping
        if let Some(wid) = self.get_wid(word) {
            return wid;
        }
        let mut lock = self.inner.write().expect("StringTable lock posioned");
        let wid = WordId::MIN_USER + lock.vec.len() as u32;
        lock.vec.push(word.to_owned());
        lock.map.insert(word.to_owned(), wid);
        WordId(wid)
    }
    pub fn get_wid(&self, word: &str) -> Option<WordId> {
        let lock = self.inner.read().expect("StringTable lock posioned");
        lock.map.get(word).map(|wid| WordId(*wid))
    }
    /// Returns the index-th string in the table as &str
    pub fn get_text(&self, wid: WordId) -> Option<String> {
        let lock = self.inner.read().expect("StringTable lock posioned");
        match wid.orig() {
            WordOrig::Static => {
                let offset = lock.offset.get(wid.0 as usize).map(|o| *o as usize)?;
                let offset_1 = lock
                    .offset
                    .get(wid.0 as usize + 1)
                    .map(|o| *o as usize)
                    .unwrap_or(lock.buffer.len());
                let s = offset;
                let e = offset_1;
                Some(lock.buffer[s..e].trim_ascii_end().to_owned())
            }
            WordOrig::User => {
                let offset = wid.as_offset();
                lock.vec.get(offset).map(|s| s.to_owned())
            }
            _ => panic!("unsupported"),
        }
    }
}

#[cfg(test)]
mod test {
    use super::StringTable;
    use crate::model::WordId;

    #[test]
    fn empty_buffer() {
        let st = StringTable::from_string("".to_string());
        assert_eq!(0, st.len());
        assert_eq!(None, st.get_text(0.into()));
        assert_eq!(None, st.get_text(WordId(100)));
    }
    #[test]
    fn oneline() {
        let st = StringTable::from_string("test\n".to_string());
        assert_eq!(1, st.len());
        assert_eq!("test", st.get_text(0.into()).unwrap());
        assert_eq!(None, st.get_text(WordId(100)));
    }
    #[test]
    fn oneline_no_lf() {
        let st = StringTable::from_string("test".to_string());
        assert_eq!(1, st.len());
        assert_eq!("test", st.get_text(0.into()).unwrap());
        assert_eq!(None, st.get_text(WordId(100)));
    }
    #[test]
    fn multi_lines() {
        let st = StringTable::from_string("test\nline2\nline3\n".to_string());
        assert_eq!(3, st.len());
        assert_eq!("test", st.get_text(0.into()).unwrap());
        assert_eq!("line3", st.get_text(2.into()).unwrap());
        assert_eq!(None, st.get_text(WordId(100)));
    }
    #[test]
    fn multi_lines_no_last_lf() {
        let st = StringTable::from_string("test\nline2\nline3".to_string());
        assert_eq!(3, st.len());
        assert_eq!("test", st.get_text(0.into()).unwrap());
        assert_eq!("line3", st.get_text(2.into()).unwrap());
        assert_eq!(None, st.get_text(WordId(100)));
    }
    #[test]
    fn debug() {
        let st = StringTable::from_string("test\nline2\nline3".to_string());
        dbg!(st);
    }
}
