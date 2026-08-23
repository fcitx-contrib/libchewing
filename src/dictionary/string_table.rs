use std::{collections::BTreeMap, fmt::Debug, fs, io, path::Path};

/// Fast and compact indexing of LF delimited strings
pub struct StringTable {
    buffer: Box<str>,
    offset: Box<[u32]>,
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
        let end = self.buffer.len().min(100);
        let buffer_prefix = format!("{}...", &self.buffer[..end]);
        f.debug_struct("StringTable")
            .field("buffer", &buffer_prefix)
            .field("offset", &IntList(&self.offset))
            .finish()
    }
}

impl StringTable {
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
        StringTable { buffer, offset }
    }
    /// Returns the number of strings in the table
    pub fn len(&self) -> usize {
        self.offset.len()
    }
    /// Returns the index-th string in the table as &str
    pub fn get(&self, index: u32) -> Option<&str> {
        let offset = self.offset.get(index as usize).map(|o| *o as usize)?;
        let offset_1 = self
            .offset
            .get(index as usize + 1)
            .map(|o| *o as usize)
            .unwrap_or(self.buffer.len());
        let s = offset;
        let e = offset_1;
        Some(&self.buffer[s..e].trim_ascii_end())
    }
    pub fn iter(&self) -> impl Iterator<Item = (&str, u32)> {
        (0..self.offset.len()).filter_map(|i| {
            let i = i as u32;
            self.get(i).map(|s| (s, i))
        })
    }
    /// Creates an inverse map from strings to indexes
    pub fn to_map(&self) -> BTreeMap<&str, u32> {
        let mut map = BTreeMap::new();
        for i in 0..self.offset.len() {
            let i = i as u32;
            if let Some(string) = self.get(i) {
                map.insert(string, i);
            }
        }
        map
    }
}

#[cfg(test)]
mod test {
    use std::u32;

    use super::StringTable;

    #[test]
    fn empty_buffer() {
        let st = StringTable::from_string("".to_string());
        assert_eq!(0, st.len());
        assert_eq!(None, st.get(0));
        assert_eq!(None, st.get(u32::MAX));
    }
    #[test]
    fn oneline() {
        let st = StringTable::from_string("test\n".to_string());
        assert_eq!(1, st.len());
        assert_eq!(Some("test"), st.get(0));
        assert_eq!(None, st.get(u32::MAX));
    }
    #[test]
    fn oneline_no_lf() {
        let st = StringTable::from_string("test".to_string());
        assert_eq!(1, st.len());
        assert_eq!(Some("test"), st.get(0));
        assert_eq!(None, st.get(u32::MAX));
    }
    #[test]
    fn multi_lines() {
        let st = StringTable::from_string("test\nline2\nline3\n".to_string());
        assert_eq!(3, st.len());
        assert_eq!(Some("test"), st.get(0));
        assert_eq!(Some("line3"), st.get(2));
        assert_eq!(None, st.get(u32::MAX));
    }
    #[test]
    fn multi_lines_no_last_lf() {
        let st = StringTable::from_string("test\nline2\nline3".to_string());
        assert_eq!(3, st.len());
        assert_eq!(Some("test"), st.get(0));
        assert_eq!(Some("line3"), st.get(2));
        assert_eq!(None, st.get(u32::MAX));
    }
    #[test]
    fn debug() {
        let st = StringTable::from_string("test\nline2\nline3".to_string());
        dbg!(st);
    }
}
