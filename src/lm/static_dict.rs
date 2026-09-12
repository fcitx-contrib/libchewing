//! Static Dictionary

use std::{
    collections::VecDeque,
    fs::File,
    io::{BufRead, BufReader, Write},
    num::NonZeroU32,
    path::Path,
    sync::Arc,
};

use scoped_error::{bail, expect_error, impl_context_error};

use crate::{
    bare::{BareDecoder, BareEncoder},
    dictionary::LookupStrategy,
    model::WordId,
    zhuyin::Syllable,
};

/// A read-only dictionary using a pre-built trie index that is both space
/// efficient and fast to lookup.
///
/// A new dictionary can be built using a [`StaticDictBuilder`].
///
/// [Trie]: https://en.m.wikipedia.org/wiki/Trie
#[derive(Debug, Clone)]
pub struct StaticDict {
    inner: Arc<StaticDictInner>,
}

#[derive(Debug)]
struct StaticDictInner {
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
        u16::from_le_bytes(self.0[6..8].try_into().unwrap())
    }
    fn child_begin(&self) -> usize {
        u32::from_le_bytes(self.0[..4].try_into().unwrap()) as usize * Self::SIZE
    }
    fn child_end(&self) -> usize {
        (u32::from_le_bytes(self.0[..4].try_into().unwrap()) as usize)
            .saturating_add(u16::from_le_bytes(self.0[4..6].try_into().unwrap()) as usize)
            * Self::SIZE
    }
}

struct TrieLeafView<'a>(&'a [u8]);

impl TrieLeafView<'_> {
    const SIZE: usize = 8;
    fn reserved_zero(&self) -> u16 {
        u16::from_le_bytes(self.0[6..8].try_into().unwrap())
    }
    fn data_begin(&self) -> usize {
        u32::from_le_bytes(self.0[..4].try_into().unwrap()) as usize
    }
    fn data_end(&self) -> usize {
        (u32::from_le_bytes(self.0[..4].try_into().unwrap()) as usize)
            .saturating_add(u16::from_le_bytes(self.0[4..6].try_into().unwrap()) as usize)
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
        let mut bytes = [0u8; 4];
        bytes[0] = self.reader.read_u8().ok()?;
        bytes[1] = self.reader.read_u8().ok()?;
        bytes[2] = self.reader.read_u8().ok()?;
        Some(WordId(u32::from_le_bytes(bytes)))
    }
}

impl StaticDict {
    pub fn new() -> StaticDict {
        StaticDict {
            inner: Arc::new(StaticDictInner {
                index: Box::new([]),
                words: Box::new([]),
            }),
        }
    }

    pub fn open<P: AsRef<Path>>(path: P) -> Result<StaticDict, StaticDictError> {
        expect_error("Failed to open static dictionary", || {
            let reader = BufReader::new(File::open(path)?);
            Ok(Self::from_reader(reader)?)
        })
    }

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
            Ok(StaticDict {
                inner: Arc::new(StaticDictInner { index, words }),
            })
        })
    }

    pub(crate) fn lookup(&self, syllables: &[Syllable], strategy: LookupStrategy) -> Vec<WordId> {
        let dict = self.inner.index.as_ref();
        let data = self.inner.words.as_ref();

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

#[derive(Debug)]
pub struct StaticDictBuilder {
    // The builder uses an arena to allocate nodes and reference each node with
    // node index.
    arena: Vec<TrieBuilderNode>,
}

#[derive(Debug, PartialEq, Default)]
struct TrieBuilderNode {
    id: u32,
    syllable: Option<Syllable>,
    children: Vec<u32>,
    leaf_id: Option<NonZeroU32>,
    words: Vec<WordId>,
}

impl StaticDictBuilder {
    pub fn new() -> StaticDictBuilder {
        let root = TrieBuilderNode::default();
        Self { arena: vec![root] }
    }

    /// Allocates a new leaf node and returns the new node id.
    fn alloc_leaf(&mut self) -> u32 {
        let next_id = self.arena.len() as u32;
        let leaf = TrieBuilderNode {
            id: next_id,
            ..Default::default()
        };
        self.arena.push(leaf);
        next_id
    }

    /// Allocates a new internal node and returns the new node id.
    fn alloc_internal(&mut self, syl: Syllable) -> u32 {
        let next_id = self.arena.len() as u32;
        let internal = TrieBuilderNode {
            id: next_id,
            syllable: Some(syl),
            ..Default::default()
        };
        self.arena.push(internal);
        next_id
    }

    /// Iterates through the syllables and insert all missing internal nodes.
    ///
    /// Returns the id to the leaf node so that we can append the phrase to it.
    fn find_or_insert_internal(&mut self, syllables: &[Syllable]) -> u32 {
        let mut node_id = 0u32;
        'next: for &syl in syllables {
            for &child_node_id in &self.arena[node_id as usize].children {
                if self.arena[child_node_id as usize].syllable == Some(syl) {
                    node_id = child_node_id;
                    continue 'next;
                }
            }
            // We didn't find the child node so insert a new one
            let next_id = self.alloc_internal(syl);
            self.arena[node_id as usize].children.push(next_id);
            node_id = next_id;
        }
        if let Some(leaf_id) = self.arena[node_id as usize].leaf_id {
            node_id = leaf_id.get();
        } else {
            let leaf_id = self.alloc_leaf();
            self.arena[node_id as usize].leaf_id = NonZeroU32::new(leaf_id);
            node_id = leaf_id;
        }
        node_id
    }

    pub fn to_writer<T>(&self, writer: T) -> Result<(), StaticDictError>
    where
        T: Write,
    {
        expect_error("Failed to serialize StaticDict", || {
            const ROOT_ID: u32 = 0;
            let mut dict_encoder = BareEncoder::new(Vec::new());
            let mut data_encoder = BareEncoder::new(Vec::new());
            let mut queue = VecDeque::new();

            // The root node's child index starts from 1 (0 is the root).
            let mut child_begin = 1;

            // Walk the tree in BFS order and write the nodes to the dict buffer.
            queue.push_back(ROOT_ID);
            while !queue.is_empty() {
                // Insert nodes layer by layer.
                let layer_nodes_count = queue.len();
                for _ in 0..layer_nodes_count {
                    // OK to unwrap, we always have at least one queued item.
                    let id = queue.pop_front().unwrap();
                    let node = &self.arena[id as usize];

                    // An internal node has an associated syllable. The root node is
                    // a special case with no syllable.
                    if node.syllable.is_some() || id == ROOT_ID {
                        let syllable_u16 = node.syllable.map_or(0, |v| v.to_u16());
                        let child_len =
                            node.children.len() + if node.leaf_id.is_some() { 1 } else { 0 };
                        dict_encoder.write_u32(child_begin)?;
                        dict_encoder.write_u16(child_len as u16)?;
                        dict_encoder.write_u16(syllable_u16)?;
                    } else {
                        let data_begin = data_encoder.len();

                        for wid in &node.words {
                            let bytes = wid.0.to_le_bytes();
                            data_encoder.write_data_exact(&bytes[..3])?;
                        }

                        let data_len = data_encoder.len() - data_begin;
                        dict_encoder.write_u32(data_begin as u32)?;
                        dict_encoder.write_u16(data_len as u16)?;
                        dict_encoder.write_u16(0)?;
                    }

                    // Sort the children nodes by their syllables. Not really required,
                    // but it makes using binary search possible in the future.
                    let mut children = node.children.clone();
                    children.sort_by(|&a, &b| {
                        self.arena[a as usize]
                            .syllable
                            .cmp(&self.arena[b as usize].syllable)
                    });
                    if let Some(leaf_id) = node.leaf_id {
                        child_begin += 1;
                        queue.push_back(leaf_id.get());
                    }
                    for child_id in children {
                        child_begin += 1;
                        queue.push_back(child_id);
                    }
                }
            }

            let mut encoder = BareEncoder::new(writer);

            // Write magic
            encoder.write_data_exact(b"CHSD")?;
            // Write file version
            encoder.write_uint(0)?;
            // Write flags
            encoder.write_u32(0)?;
            // Write index
            encoder.write_data(&dict_encoder.into_inner())?;
            // Write words
            encoder.write_data(&data_encoder.into_inner())?;

            Ok(())
        })
    }

    pub fn build(self) -> StaticDict {
        let mut buf = vec![];
        self.to_writer(&mut buf)
            .expect("Failed to serialize in-memory StaticDict");
        StaticDict::from_reader(buf.as_slice()).expect("Failed to build im-memory StaticDict")
    }

    pub fn insert(&mut self, syllables: &[Syllable], wid: WordId) {
        let leaf_id = self.find_or_insert_internal(syllables) as usize;
        if let Some(it) = self.arena[leaf_id].words.iter_mut().find(|it| **it == wid) {
            *it = wid;
        } else {
            self.arena[leaf_id].words.push(wid);
        }
    }
}

impl_context_error!(pub StaticDictError);

#[cfg(test)]
mod test {
    use crate::dictionary::LookupStrategy;
    use crate::model::WordId;
    use crate::syl;
    use crate::zhuyin::Bopomofo;

    use super::StaticDict;
    use super::StaticDictBuilder;

    #[test]
    fn build() {
        let mut buf = Vec::<u8>::new();
        let mut builder = StaticDictBuilder::new();

        builder.insert(
            &[syl![Bopomofo::C, Bopomofo::E, Bopomofo::TONE4]],
            WordId(1),
        );
        builder.insert(&[syl![Bopomofo::SH, Bopomofo::TONE4]], WordId(2));
        builder.insert(
            &[
                syl![Bopomofo::C, Bopomofo::E, Bopomofo::TONE4],
                syl![Bopomofo::SH, Bopomofo::TONE4],
            ],
            WordId(3),
        );
        builder.insert(
            &[
                syl![Bopomofo::C, Bopomofo::E, Bopomofo::TONE4],
                syl![Bopomofo::I, Bopomofo::AN, Bopomofo::TONE4],
            ],
            WordId(4),
        );

        builder.to_writer(&mut buf).unwrap();

        assert_eq!(
            &[
                67, 72, 83, 68, 0, 0, 0, 0, 0, 72, 1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 1, 0, 4, 34,
                4, 0, 0, 0, 3, 0, 28, 40, 0, 0, 0, 0, 3, 0, 0, 0, 3, 0, 0, 0, 3, 0, 0, 0, 7, 0, 0,
                0, 1, 0, 204, 0, 8, 0, 0, 0, 1, 0, 4, 34, 6, 0, 0, 0, 3, 0, 0, 0, 9, 0, 0, 0, 3, 0,
                0, 0, 12, 2, 0, 0, 1, 0, 0, 4, 0, 0, 3, 0, 0
            ],
            buf.as_slice()
        );
    }

    #[test]
    fn read() {
        let mut buf = Vec::<u8>::new();
        let mut builder = StaticDictBuilder::new();

        let entries = [
            (
                &[syl![Bopomofo::C, Bopomofo::E, Bopomofo::TONE4]][..],
                WordId(1),
            ),
            (&[syl![Bopomofo::SH, Bopomofo::TONE4]], WordId(2)),
            (
                &[
                    syl![Bopomofo::C, Bopomofo::E, Bopomofo::TONE4],
                    syl![Bopomofo::SH, Bopomofo::TONE4],
                ],
                WordId(3),
            ),
            (
                &[
                    syl![Bopomofo::C, Bopomofo::E, Bopomofo::TONE4],
                    syl![Bopomofo::I, Bopomofo::AN, Bopomofo::TONE4],
                ],
                WordId(4),
            ),
            (
                &[
                    syl![Bopomofo::C, Bopomofo::E, Bopomofo::TONE4],
                    syl![Bopomofo::I, Bopomofo::AN, Bopomofo::TONE4],
                ],
                WordId(5),
            ),
        ];

        for (syllables, wid) in &entries {
            builder.insert(syllables, *wid);
        }

        builder.to_writer(&mut buf).unwrap();

        let dict = StaticDict::from_reader(buf.as_slice()).unwrap();

        assert_eq!(
            dict.lookup(
                &[syl![Bopomofo::C, Bopomofo::E, Bopomofo::TONE4]],
                LookupStrategy::Standard
            ),
            vec![WordId(1)],
        );
        assert_eq!(
            dict.lookup(
                &[syl![Bopomofo::SH, Bopomofo::TONE4]],
                LookupStrategy::Standard
            ),
            vec![WordId(2)],
        );
        assert_eq!(
            dict.lookup(
                &[
                    syl![Bopomofo::C, Bopomofo::E, Bopomofo::TONE4],
                    syl![Bopomofo::SH, Bopomofo::TONE4],
                ],
                LookupStrategy::Standard
            ),
            vec![WordId(3)],
        );
        assert_eq!(
            dict.lookup(
                &[
                    syl![Bopomofo::C, Bopomofo::E, Bopomofo::TONE4],
                    syl![Bopomofo::I, Bopomofo::AN, Bopomofo::TONE4],
                ],
                LookupStrategy::Standard
            ),
            vec![WordId(4), WordId(5)],
        );
    }
}
