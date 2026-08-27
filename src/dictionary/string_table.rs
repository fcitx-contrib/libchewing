use std::{
    borrow::Cow,
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
    inner: Arc<StringTableInner>,
}

struct StringTableInner {
    buffer: Box<str>,
    offset: Box<[u32]>,
    intern: RwLock<Vec<String>>,
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
        let end = self.inner.buffer.len().min(100);
        let buffer_prefix = format!("{}...", &self.inner.buffer[..end]);
        f.debug_struct("StringTable")
            .field("buffer", &buffer_prefix)
            .field("offset", &IntList(&self.inner.offset))
            .finish()
    }
}

impl StringTable {
    /// Creates an empty StringTable
    pub fn new() -> StringTable {
        StringTable {
            inner: Arc::new(StringTableInner {
                buffer: String::new().into_boxed_str(),
                offset: vec![].into_boxed_slice(),
                intern: RwLock::new(vec![]),
            }),
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
        for line in buffer.lines() {
            offset.push((line.as_ptr() as usize - bob) as u32);
        }
        let offset = offset.into_boxed_slice();
        let intern = RwLock::new(vec![]);
        StringTable {
            inner: Arc::new(StringTableInner {
                buffer,
                offset,
                intern,
            }),
        }
    }
    pub fn intern(&self, word: &str) -> WordId {
        let mut intern = self
            .inner
            .intern
            .write()
            .expect("StringTable lock posioned");
        let wid = intern.len();
        intern.push(word.to_owned());
        WordId::from_user(wid as u32)
    }
    /// Returns the number of strings in the table
    pub fn len(&self) -> usize {
        let intern = self.inner.intern.read().expect("StringTable lock posioned");
        self.inner.offset.len() + intern.len()
    }
    /// Returns the index-th string in the table as &str
    pub fn get(&self, wid: WordId) -> Option<Cow<'_, str>> {
        match wid.orig() {
            WordOrig::Static => {
                let offset = self.inner.offset.get(wid.0 as usize).map(|o| *o as usize)?;
                let offset_1 = self
                    .inner
                    .offset
                    .get(wid.0 as usize + 1)
                    .map(|o| *o as usize)
                    .unwrap_or(self.inner.buffer.len());
                let s = offset;
                let e = offset_1;
                Some(Cow::Borrowed(&self.inner.buffer[s..e].trim_ascii_end()))
            }
            WordOrig::User => {
                let intern = self.inner.intern.read().expect("StringTable lock posioned");
                let offset = wid.as_offset();
                intern.get(offset).map(|s| Cow::Owned(s.clone()))
            }
            _ => panic!("unsupported"),
        }
    }
    // FIXME: remove this or make it work with intern table
    /// Returns an iterator of the static string table
    pub fn iter(&self) -> impl Iterator<Item = (Cow<'_, str>, u32)> {
        (0..self.inner.offset.len()).filter_map(|i| {
            let i = i as u32;
            self.get(WordId(i)).map(|s| (s, i))
        })
    }
    /// Creates an inverse map from strings to indexes
    pub fn to_map(&self) -> BTreeMap<Cow<'_, str>, u32> {
        let mut map = BTreeMap::new();
        for i in 0..self.inner.offset.len() {
            let i = i as u32;
            if let Some(string) = self.get(WordId(i)) {
                map.insert(string, i);
            }
        }
        let intern = self.inner.intern.read().expect("StringTable lock posioned");
        for (i, word) in intern.iter().enumerate() {
            map.insert(Cow::Owned(word.clone()), WordId::MIN_USER.0 + i as u32);
        }
        map
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
        assert_eq!(None, st.get(0.into()));
        assert_eq!(None, st.get(WordId::MAX));
    }
    #[test]
    fn oneline() {
        let st = StringTable::from_string("test\n".to_string());
        assert_eq!(1, st.len());
        assert_eq!("test", st.get(0.into()).unwrap());
        assert_eq!(None, st.get(WordId::MAX));
    }
    #[test]
    fn oneline_no_lf() {
        let st = StringTable::from_string("test".to_string());
        assert_eq!(1, st.len());
        assert_eq!("test", st.get(0.into()).unwrap());
        assert_eq!(None, st.get(WordId::MAX));
    }
    #[test]
    fn multi_lines() {
        let st = StringTable::from_string("test\nline2\nline3\n".to_string());
        assert_eq!(3, st.len());
        assert_eq!("test", st.get(0.into()).unwrap());
        assert_eq!("line3", st.get(2.into()).unwrap());
        assert_eq!(None, st.get(WordId::MAX));
    }
    #[test]
    fn multi_lines_no_last_lf() {
        let st = StringTable::from_string("test\nline2\nline3".to_string());
        assert_eq!(3, st.len());
        assert_eq!("test", st.get(0.into()).unwrap());
        assert_eq!("line3", st.get(2.into()).unwrap());
        assert_eq!(None, st.get(WordId::MAX));
    }
    #[test]
    fn debug() {
        let st = StringTable::from_string("test\nline2\nline3".to_string());
        dbg!(st);
    }
}
