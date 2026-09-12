use std::{
    collections::VecDeque,
    error::Error,
    fmt::Debug,
    fs::File,
    io::{self, Read},
    iter,
    path::{Path, PathBuf},
};

use der::{
    DecodeValue, Document, Encode, EncodeValue, ErrorKind, FixedTag, Length, Reader, Sequence,
    SliceReader, Tag, TagMode, TagNumber, Tagged, Writer,
    asn1::{ContextSpecificRef, OctetStringRef, Utf8StringRef},
};
use log::error;

use super::{Entries, LookupStrategy, Phrase};
use crate::zhuyin::Syllable;

const DICT_FORMAT_VERSION: u8 = 0;

struct TrieNodeView<'a>(&'a [u8]);

impl TrieNodeView<'_> {
    const SIZE: usize = 8;
    fn syllable(&self) -> u16 {
        u16::from_be_bytes(self.0[6..8].try_into().unwrap())
    }
    fn child_begin(&self) -> usize {
        u32::from_be_bytes(self.0[..4].try_into().unwrap()) as usize * Self::SIZE
    }
    fn child_end(&self) -> usize {
        (u32::from_be_bytes(self.0[..4].try_into().unwrap()) as usize)
            .saturating_add(u16::from_be_bytes(self.0[4..6].try_into().unwrap()) as usize)
            * Self::SIZE
    }
}

struct TrieLeafView<'a>(&'a [u8]);

impl TrieLeafView<'_> {
    const SIZE: usize = 8;
    fn reserved_zero(&self) -> u16 {
        u16::from_be_bytes(self.0[6..8].try_into().unwrap())
    }
    fn data_begin(&self) -> usize {
        u32::from_be_bytes(self.0[..4].try_into().unwrap()) as usize
    }
    fn data_end(&self) -> usize {
        (u32::from_be_bytes(self.0[..4].try_into().unwrap()) as usize)
            .saturating_add(u16::from_be_bytes(self.0[4..6].try_into().unwrap()) as usize)
    }
}

/// A read-only dictionary using a pre-built [Trie][] index that is both space
/// efficient and fast to lookup.
///
/// # Examples
///
/// Read a dictionary from a [File][`std::fs::File`]:
///
/// ```no_run
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use std::fs::File;
///
/// use chewing::{syl, zhuyin::{Bopomofo, Syllable}};
/// use chewing::dictionary::{LookupStrategy, Trie};
///
/// let mut file = File::open("dict.dat")?;
/// let dict = Trie::new(&mut file)?;
///
/// // Find the phrase ㄗˋㄉ一ㄢˇ (dictionary)
/// let phrase = dict.lookup(&[
///     syl![Bopomofo::Z, Bopomofo::TONE4],
///     syl![Bopomofo::D, Bopomofo::I, Bopomofo::AN, Bopomofo::TONE3]
/// ], LookupStrategy::Standard);
/// assert_eq!("字典", phrase.first().unwrap().as_str());
/// # Ok(())
/// # }
/// ```
///
/// [Trie]: https://en.m.wikipedia.org/wiki/Trie
/// [DER]: https://en.m.wikipedia.org/wiki/X.690#DER_encoding
#[derive(Debug, Clone)]
pub struct Trie {
    path: Option<PathBuf>,
    index: Box<[u8]>,
    phrase_seq: Box<[u8]>,

    fuzzy_search: bool,
}

fn io_error(e: impl Into<Box<dyn Error + Send + Sync>>) -> io::Error {
    io::Error::other(e)
}

impl Trie {
    /// Creates a new `Trie` instance from a file.
    ///
    /// The data in the file must conform to the dictionary format spec.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use chewing::dictionary::Trie;
    ///
    /// let dict = Trie::open("dict.dat")?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Trie> {
        TrieOpenOptions::new().open(path)
    }
    /// Creates a new `Trie` instance from a input stream.
    ///
    /// The underlying data of the input stream must conform to the dictionary
    /// format spec.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use std::fs::File;
    ///
    /// use chewing::dictionary::Trie;
    ///
    /// let mut file = File::open("dict.dat")?;
    /// let dict = Trie::new(&mut file)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn new<T>(stream: T) -> io::Result<Trie>
    where
        T: Read,
    {
        TrieOpenOptions::new().read_from(stream)
    }
    /// Enable or disable fuzzy search.
    pub fn enable_fuzzy_search(&mut self, fuzzy_search: bool) {
        self.fuzzy_search = fuzzy_search;
    }
}

/// Options and flags which can be used to configure how a trie dictionary is
/// opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TrieOpenOptions {
    fuzzy_search: bool,
}

impl TrieOpenOptions {
    pub fn new() -> TrieOpenOptions {
        TrieOpenOptions::default()
    }
    pub fn fuzzy_search(&mut self, fuzzy_search: bool) -> &mut Self {
        self.fuzzy_search = fuzzy_search;
        self
    }
    pub fn open<P: AsRef<Path>>(&self, path: P) -> io::Result<Trie> {
        let path = path.as_ref().to_path_buf();
        let mut file = File::open(&path)?;
        let mut trie = self.read_from(&mut file)?;
        trie.path = Some(path);
        Ok(trie)
    }
    pub fn read_from<T>(&self, mut stream: T) -> io::Result<Trie>
    where
        T: Read,
    {
        let mut buf = vec![];
        stream.read_to_end(&mut buf)?;
        let trie_dict_doc = Document::try_from(buf).map_err(io_error)?;
        let trie_ref: TrieFileRef<'_> = trie_dict_doc.decode_msg().map_err(io_error)?;
        let index = trie_ref.index.as_bytes().into();
        let phrase_seq = trie_ref.phrase_seq.der_bytes.into();
        Ok(Trie {
            path: None,
            index,
            phrase_seq,
            fuzzy_search: self.fuzzy_search,
        })
    }
}

struct PhrasesIter<'a> {
    reader: SliceReader<'a>,
}

impl PhrasesIter<'_> {
    fn new(bytes: &[u8]) -> PhrasesIter<'_> {
        PhrasesIter {
            reader: SliceReader::new(bytes).unwrap(),
        }
    }
}

impl Iterator for PhrasesIter<'_> {
    type Item = Phrase;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        if self.reader.is_finished() {
            return None;
        }
        self.reader.decode().ok()
    }
}

macro_rules! bail_if_oob {
    ($begin:expr, $end:expr, $len:expr) => {
        if $begin >= $end || $end > $len {
            error!("[!] file corruption detected: index out of bound.");
            return vec![];
        }
    };
}

macro_rules! iter_bail_if_oob {
    ($begin:expr, $end:expr, $len:expr) => {
        if $begin >= $end || $end > $len {
            error!("[!] file corruption detected: index out of bound.");
            return None;
        }
    };
}

impl Trie {
    pub fn lookup(&self, syllables: &[Syllable], strategy: LookupStrategy) -> Vec<Phrase> {
        let dict = self.index.as_ref();
        let data = self.phrase_seq.as_ref();

        bail_if_oob!(0, TrieNodeView::SIZE, dict.len());
        let root = TrieNodeView(&dict[..TrieNodeView::SIZE]);

        // Return early for empty dictionary
        if root.child_begin() == root.child_end() {
            return vec![];
        }

        let search_predicate = match strategy {
            LookupStrategy::Standard => |n: u16, syl: &Syllable| n == syl.to_u16(),
            LookupStrategy::FuzzyPartialPrefix => |n: u16, syl: &Syllable| {
                if n == 0 {
                    return false;
                }
                if let Ok(syllable) = Syllable::try_from(n) {
                    syllable.starts_with(*syl)
                } else {
                    false
                }
            },
        };

        // Perform a BFS search to find all leaf nodes
        let mut threads: VecDeque<TrieNodeView<'_>> = VecDeque::new();
        threads.push_back(root);
        for syl in syllables {
            debug_assert!(syl.to_u16() != 0);
            for _ in 0..threads.len() {
                let node = threads.pop_front().unwrap();
                bail_if_oob!(node.child_begin(), node.child_end(), dict.len());
                let child_nodes = dict[node.child_begin()..node.child_end()]
                    .chunks_exact(TrieNodeView::SIZE)
                    .map(TrieNodeView);
                for n in child_nodes {
                    if search_predicate(n.syllable(), syl) {
                        threads.push_back(n);
                    }
                }
            }
            if threads.is_empty() {
                return vec![];
            }
        }

        // Collect result from all threads
        let mut result = vec![];
        for node in threads.into_iter() {
            bail_if_oob!(node.child_begin(), node.child_end(), dict.len());
            let leaf_data = &dict[node.child_begin()..];
            bail_if_oob!(0, TrieLeafView::SIZE, leaf_data.len());
            let leaf = TrieLeafView(&leaf_data[..TrieLeafView::SIZE]);
            if leaf.reserved_zero() != 0 {
                // Skip non leaf nodes
                continue;
            }
            bail_if_oob!(leaf.data_begin(), leaf.data_end(), data.len());
            result.extend(PhrasesIter::new(&data[leaf.data_begin()..leaf.data_end()]));
        }
        result
    }

    pub fn entries(&self) -> Entries<'_> {
        let dict = self.index.as_ref();
        let data = self.phrase_seq.as_ref();
        let mut results = Vec::new();
        let mut stack = Vec::new();
        let mut syllables = Vec::new();
        if dict.len() < TrieNodeView::SIZE {
            error!("[!] file corruption detected: index out of bound.");
            return Box::new(iter::empty());
        }
        let root = TrieNodeView(&dict[..TrieNodeView::SIZE]);
        let mut node = root;
        if node.child_begin() == node.child_end() {
            return Box::new(iter::empty());
        }

        let make_dict_entry =
            |syllables: &[u16], leaf: &TrieLeafView<'_>| -> (Vec<Syllable>, Vec<Phrase>) {
                debug_assert_eq!(leaf.reserved_zero(), 0);
                (
                    syllables
                        .iter()
                        // FIXME - skip invalid entry?
                        .map(|&syl_u16| Syllable::try_from(syl_u16).unwrap())
                        .collect::<Vec<_>>(),
                    PhrasesIter::new(&data[leaf.data_begin()..leaf.data_end()]).collect::<Vec<_>>(),
                )
            };

        let mut done = false;
        let it = iter::from_fn(move || {
            if !results.is_empty() {
                return results.pop();
            }
            if done {
                return None;
            }
            // descend until find a leaf node which is not also a internal node.
            loop {
                iter_bail_if_oob!(node.child_begin(), node.child_end(), dict.len());
                let mut child_iter = dict[node.child_begin()..node.child_end()]
                    .chunks_exact(TrieNodeView::SIZE)
                    .map(TrieNodeView);
                let mut next = child_iter
                    .next()
                    .expect("syllable node should have at least one child node");
                if next.syllable() == 0 {
                    // found a leaf syllable node
                    iter_bail_if_oob!(node.child_begin(), node.child_end(), dict.len());
                    let leaf_data = &dict[node.child_begin()..];
                    let leaf = TrieLeafView(&leaf_data[..TrieLeafView::SIZE]);
                    iter_bail_if_oob!(leaf.data_begin(), leaf.data_end(), data.len());
                    results.push(make_dict_entry(&syllables, &leaf));
                    if let Some(second) = child_iter.next() {
                        next = second;
                    } else {
                        break;
                    }
                }
                node = next;
                syllables.push(node.syllable());
                stack.push(child_iter);
            }
            // ascend until we can go down again
            loop {
                if let Some(mut child_nodes) = stack.pop() {
                    syllables.pop();
                    if let Some(next) = child_nodes.next() {
                        debug_assert_ne!(next.syllable(), 0);
                        node = next;
                        stack.push(child_nodes);
                        syllables.push(node.syllable());
                        break;
                    }
                } else {
                    done = true;
                    break;
                }
            }
            results.pop()
        });
        let entries = it.flat_map(|(syllables, phrases)| {
            phrases
                .into_iter()
                .map(move |phrase| (syllables.clone(), phrase))
        });
        Box::new(entries)
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_ref().map(|p| p as &Path)
    }
}

fn context_specific<T: EncodeValue + Tagged>(
    tag_number: u8,
    tag_mode: TagMode,
    value: &T,
) -> ContextSpecificRef<'_, T> {
    ContextSpecificRef {
        tag_number: TagNumber::new(tag_number),
        tag_mode,
        value,
    }
}

fn context_specific_opt<T: EncodeValue + Tagged>(
    tag_number: u8,
    tag_mode: TagMode,
    value: &Option<T>,
) -> Option<ContextSpecificRef<'_, T>> {
    value
        .as_ref()
        .map(|value| context_specific(tag_number, tag_mode, value))
}

struct DictionaryInfoRef<'a> {
    name: Utf8StringRef<'a>,
    copyright: Utf8StringRef<'a>,
    license: Utf8StringRef<'a>,
    version: Utf8StringRef<'a>,
    software: Utf8StringRef<'a>,
}

impl FixedTag for DictionaryInfoRef<'_> {
    const TAG: Tag = Tag::Sequence;
}

impl<'a> DecodeValue<'a> for DictionaryInfoRef<'a> {
    fn decode_value<R: Reader<'a>>(reader: &mut R, header: der::Header) -> der::Result<Self> {
        reader.read_nested(header.length, |reader| {
            let name = reader.decode()?;
            let copyright = reader.decode()?;
            let license = reader.decode()?;
            let version = reader.decode()?;
            let software = reader.decode()?;
            let _raw_usage = reader
                .context_specific(TagNumber::N0, TagMode::Explicit)?
                .unwrap_or(0);
            // consume the remaining unknown data
            let _ = reader.read_slice(reader.remaining_len());
            Ok(DictionaryInfoRef {
                name,
                copyright,
                license,
                version,
                software,
            })
        })
    }
}

impl EncodeValue for DictionaryInfoRef<'_> {
    fn value_len(&self) -> der::Result<Length> {
        self.name.encoded_len()?
            + self.copyright.encoded_len()?
            + self.license.encoded_len()?
            + self.version.encoded_len()?
            + self.software.encoded_len()?
        // TODO - enable this will break chewing <= 0.11.0 because old
        // parser did not handle extension marker properly
        // + context_specific(0, TagMode::Explicit, &(self.usage as u8)).encoded_len()?
    }

    fn encode_value(&self, encoder: &mut impl Writer) -> der::Result<()> {
        self.name.encode(encoder)?;
        self.copyright.encode(encoder)?;
        self.license.encode(encoder)?;
        self.version.encode(encoder)?;
        self.software.encode(encoder)?;
        // TODO - enable this will break chewing <= 0.11.0 because old
        // parser did not handle extension marker properly
        // context_specific(0, TagMode::Explicit, &(self.usage as u8)).encode(encoder)?;
        Ok(())
    }
}

struct TrieFileRef<'a> {
    info: DictionaryInfoRef<'a>,
    index: OctetStringRef<'a>,
    phrase_seq: PhraseSeqRef<'a>,
}

struct PhraseSeqRef<'a> {
    der_bytes: &'a [u8],
}

impl<'a> Sequence<'a> for TrieFileRef<'a> {}

impl<'a> DecodeValue<'a> for TrieFileRef<'a> {
    fn decode_value<R: Reader<'a>>(reader: &mut R, header: der::Header) -> der::Result<Self> {
        reader.read_nested(header.length, |reader| {
            let magic: Utf8StringRef<'_> = reader.decode()?;
            let version: u8 = reader.decode()?;
            if magic.as_str() != "CHEW" || version != DICT_FORMAT_VERSION {
                return Err(ErrorKind::Value { tag: header.tag }.at(reader.position()));
            }
            let info = reader.decode()?;
            let index = reader.decode()?;
            let phrase_seq = reader.decode()?;
            // consume the remaining unknown data
            let _ = reader.read_slice(reader.remaining_len());
            Ok(Self {
                info,
                index,
                phrase_seq,
            })
        })
    }
}

impl EncodeValue for TrieFileRef<'_> {
    fn value_len(&self) -> der::Result<Length> {
        Utf8StringRef::new("CHEW")?.encoded_len()?
            + DICT_FORMAT_VERSION.encoded_len()?
            + self.info.encoded_len()?
            + self.index.encoded_len()?
            + self.phrase_seq.encoded_len()?
    }

    fn encode_value(&self, encoder: &mut impl Writer) -> der::Result<()> {
        Utf8StringRef::new("CHEW")?.encode(encoder)?;
        DICT_FORMAT_VERSION.encode(encoder)?;
        self.info.encode(encoder)?;
        self.index.encode(encoder)?;
        self.phrase_seq.encode(encoder)?;
        Ok(())
    }
}

impl FixedTag for Phrase {
    const TAG: Tag = Tag::Sequence;
}

impl<'a> DecodeValue<'a> for Phrase {
    fn decode_value<R: Reader<'a>>(reader: &mut R, header: der::Header) -> der::Result<Self> {
        reader.read_nested(header.length, |reader| {
            let phrase: Utf8StringRef<'_> = reader.decode()?;
            let freq = reader.decode()?;
            let last_used = reader.context_specific(TagNumber::N0, TagMode::Implicit)?;
            // consume the remaining unknown data
            let _ = reader.read_slice(reader.remaining_len());
            Ok(Phrase {
                text: String::from(phrase).into_boxed_str(),
                freq,
                last_used,
            })
        })
    }
}

impl EncodeValue for Phrase {
    fn value_len(&self) -> der::Result<Length> {
        Utf8StringRef::new(self.as_str())?.encoded_len()?
            + self.freq.encoded_len()?
            + context_specific_opt(0, TagMode::Implicit, &self.last_used).encoded_len()?
    }

    fn encode_value(&self, encoder: &mut impl Writer) -> der::Result<()> {
        Utf8StringRef::new(self.as_str())?.encode(encoder)?;
        self.freq.encode(encoder)?;
        context_specific_opt(0, TagMode::Implicit, &self.last_used).encode(encoder)?;
        Ok(())
    }
}

impl FixedTag for PhraseSeqRef<'_> {
    const TAG: Tag = Tag::Sequence;
}

impl EncodeValue for PhraseSeqRef<'_> {
    fn value_len(&self) -> der::Result<Length> {
        self.der_bytes.len().try_into()
    }

    fn encode_value(&self, encoder: &mut impl Writer) -> der::Result<()> {
        encoder.write(self.der_bytes)
    }
}

impl<'a> DecodeValue<'a> for PhraseSeqRef<'a> {
    fn decode_value<R: Reader<'a>>(reader: &mut R, header: der::Header) -> der::Result<Self> {
        reader.read_nested(header.length, |reader| {
            let der_bytes = reader.read_slice(header.length)?;
            Ok(Self { der_bytes })
        })
    }
}
