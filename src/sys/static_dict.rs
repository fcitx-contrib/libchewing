//! Static Dictionary

use std::{collections::VecDeque, io::BufRead};

use scoped_error::{bail, expect_error, impl_context_error};

use crate::{bare::BareDecoder, dictionary::LookupStrategy, model::WordId, zhuyin::Syllable};

/// A read-only dictionary using a pre-built [Trie][] index that is both space
/// efficient and fast to lookup.
///
/// `Trie`s can be used as system dictionaries or shared dictionaries.
/// The file format is defined using the platform independent [DER][DER]
/// encoding format, allowing them to be versioned and shared easily.
///
/// A new dictionary can be built using a [`TrieBuilder`].
///
/// # Examples
///
/// Read a dictionary from a [File][`std::fs::File`]:
///
/// ```
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// # let tmpdir = tempfile::tempdir()?;
/// # std::env::set_current_dir(&tmpdir.path())?;
/// use std::fs::File;
///
/// use chewing::{syl, zhuyin::{Bopomofo, Syllable}};
/// # use chewing::dictionary::{DictionaryBuilder, TrieBuilder};
/// use chewing::dictionary::{Dictionary, LookupStrategy, Trie};
/// # let mut tempfile = File::create("dict.dat")?;
/// # let mut builder = TrieBuilder::new();
/// # builder.insert(&[
/// #     syl![Bopomofo::Z, Bopomofo::TONE4],
/// #     syl![Bopomofo::D, Bopomofo::I, Bopomofo::AN, Bopomofo::TONE3]
/// # ], ("字典", 0).into());
/// # builder.write(&mut tempfile)?;
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
pub struct StaticDict {
    index: Box<[u8]>,
    words: Box<[u8]>,
}

macro_rules! bail_if_oob {
    ($begin:expr, $end:expr, $len:expr) => {
        if $begin >= $end || $end > $len {
            log::error!("[!] file corruption detected: index out of bound.");
            return vec![];
        }
    };
}

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

struct WordsIter<'a> {
    reader: BareDecoder<&'a [u8]>,
}

impl WordsIter<'_> {
    fn new(bytes: &[u8]) -> WordsIter<'_> {
        WordsIter {
            reader: BareDecoder::new(bytes),
        }
    }
}

impl Iterator for WordsIter<'_> {
    type Item = WordId;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        self.reader.read_u32().ok().map(|v| WordId(v))
    }
}

impl StaticDict {
    pub fn from_reader<R: BufRead>(reader: R) -> Result<StaticDict, StaticDictError> {
        expect_error("Failed to read static dictionary", || {
            let mut decoder = BareDecoder::new(reader);
            let magic = decoder.read_data_exact(4)?;
            if magic != b"CHSD" {
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
            let index = decoder.read_data()?.into_boxed_slice();
            let words = decoder.read_data()?.into_boxed_slice();
            Ok(StaticDict { index, words })
        })
    }

    pub(crate) fn lookup(&self, syllables: &[Syllable], strategy: LookupStrategy) -> Vec<WordId> {
        let dict = self.index.as_ref();
        let data = self.words.as_ref();

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
            result.extend(WordsIter::new(&data[leaf.data_begin()..leaf.data_end()]));
        }
        result
    }
}

impl_context_error!(pub StaticDictError);
