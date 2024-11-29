use std::collections::HashSet;
use std::collections::VecDeque;
use std::io::Error;
use std::io::Read;
use std::io::Seek;
use std::io::Write;
use std::marker::PhantomData;
use std::ops::Deref;
use std::ops::DerefMut;

use crate::io::*;
use crate::BlockIo;
use crate::Blocks;

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct VecTree<K, V, C> {
    entries: Vec<(K, V)>,
    block_len: usize,
    #[allow(unused)]
    phantom: PhantomData<C>,
}

impl<K, V, C> VecTree<K, V, C> {
    pub fn new(entries: Vec<(K, V)>, block_len: usize) -> Self {
        let block_len = block_len.clamp(MIN_BLOCK_LEN, MAX_BLOCK_LEN);
        Self {
            entries,
            block_len,
            phantom: Default::default(),
        }
    }

    pub fn into_inner(self) -> Vec<(K, V)> {
        self.entries
    }
}

impl<K, V, C> Default for VecTree<K, V, C> {
    fn default() -> Self {
        Self {
            entries: Default::default(),
            block_len: 4096,
            phantom: Default::default(),
        }
    }
}

impl<K, V, C> Deref for VecTree<K, V, C> {
    type Target = Vec<(K, V)>;

    fn deref(&self) -> &Self::Target {
        &self.entries
    }
}

impl<K, V, C> DerefMut for VecTree<K, V, C> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.entries
    }
}

impl<C, K: BlockIo<C>, V: BlockIo<C>> BlockIo<C> for VecTree<K, V, C> {
    fn write_block<W: Write + Seek>(
        &self,
        mut writer: W,
        blocks: &mut Blocks,
        context: &mut C,
    ) -> Result<u32, Error> {
        let n = max_enties_per_block(self.block_len);
        let num_entries = self.entries.len();
        let root = if num_entries <= n {
            // One data node is enough, no need to introduce meta nodes.
            let mut raw_entries = Vec::with_capacity(num_entries);
            for (key, value) in self.entries.iter() {
                let key = key.write_block(writer.by_ref(), blocks, context)?;
                let value = value.write_block(writer.by_ref(), blocks, context)?;
                raw_entries.push((key, value));
            }
            eprintln!("data entries: {:?}", raw_entries);
            let data_node = RawTreeNode {
                next: 0,
                prev: 0,
                entries: raw_entries,
                is_data: true,
            };
            blocks.append(writer.by_ref(), |writer| data_node.write(writer))?
        } else {
            let num_data_nodes = num_entries.div_ceil(n);
            let num_meta_nodes = num_data_nodes.div_ceil(n);
            let max_data_nodes_per_meta_node = num_data_nodes.div_ceil(num_meta_nodes);
            let mut iter = self.entries.iter();
            let mut meta_nodes = Vec::with_capacity(num_meta_nodes);
            eprintln!("num meta nodes {}", num_meta_nodes);
            eprintln!("num data nodes {}", num_data_nodes);
            eprintln!(
                "max data nodes per meta node {}",
                max_data_nodes_per_meta_node
            );
            for _ in 0..num_meta_nodes {
                let mut data_nodes = Vec::with_capacity(max_data_nodes_per_meta_node);
                for _ in 0..max_data_nodes_per_meta_node {
                    let entries = collect_n(&mut iter, n);
                    if entries.is_empty() {
                        break;
                    }
                    let mut raw_entries = Vec::with_capacity(entries.len());
                    for (key, value) in entries.iter() {
                        let key = key.write_block(writer.by_ref(), blocks, context)?;
                        let value = value.write_block(writer.by_ref(), blocks, context)?;
                        raw_entries.push((key, value));
                    }
                    let last_value_block =
                        raw_entries.last().expect("We have at least one entry").1;
                    let data_node = RawTreeNode {
                        next: 0,
                        prev: 0,
                        entries: raw_entries,
                        is_data: true,
                    };
                    data_nodes.push((data_node, last_value_block));
                }
                // set next/prev and generate meta node entries
                let mut raw_entries = Vec::with_capacity(data_nodes.len());
                let first_block = blocks.next_block_index();
                let last_block = first_block + data_nodes.len() as u32 - 1;
                let mut current_block = first_block;
                for (mut data_node, last_value_block) in data_nodes.into_iter() {
                    data_node.prev = if current_block == first_block {
                        0
                    } else {
                        current_block - 1
                    };
                    data_node.next = if current_block == last_block {
                        0
                    } else {
                        current_block + 1
                    };
                    let block = blocks.append(writer.by_ref(), |writer| data_node.write(writer))?;
                    debug_assert!(block == current_block);
                    current_block += 1;
                    raw_entries.push((block, last_value_block));
                }
                eprintln!("meta entries: {:?}", raw_entries);
                meta_nodes.push(RawTreeNode {
                    next: 0,
                    prev: 0,
                    entries: raw_entries,
                    is_data: false,
                });
            }
            // set next/prev for meta nodes
            let first_block = blocks.next_block_index();
            let last_block = first_block + meta_nodes.len() as u32 - 1;
            let mut current_block = first_block;
            for mut meta_node in meta_nodes.into_iter() {
                meta_node.prev = if current_block == first_block {
                    0
                } else {
                    current_block - 1
                };
                meta_node.next = if current_block == last_block {
                    0
                } else {
                    current_block + 1
                };
                let block = blocks.append(writer.by_ref(), |writer| meta_node.write(writer))?;
                debug_assert!(block == current_block);
                current_block += 1;
            }
            first_block
        };
        let tree = RawTree {
            root,
            block_len: self.block_len as u32,
            num_entries: num_entries as u32,
        };
        blocks.append(writer.by_ref(), |writer| tree.write(writer))
    }

    fn read_block(
        i: u32,
        file: &[u8],
        blocks: &mut Blocks,
        context: &mut C,
    ) -> Result<Self, Error> {
        let tree = RawTree::read(blocks.slice(i, file)?)?;
        let mut entries = Vec::new();
        let mut visited = HashSet::new();
        let mut nodes = VecDeque::new();
        nodes.push_back(tree.root);
        while let Some(node) = nodes.pop_front() {
            if !visited.insert(node) {
                // loop
                continue;
            }
            let node = RawTreeNode::read(blocks.slice(node, file)?)?;
            if node.is_data {
                // data node
                for (key, value) in node.entries.into_iter() {
                    let key = K::read_block(key, file, blocks, context)?;
                    let value = V::read_block(value, file, blocks, context)?;
                    entries.push((key, value));
                }
            } else {
                // meta node
                for (key, _value) in node.entries.into_iter() {
                    // value equals to the last entry of the data node referenced by key
                    nodes.push_back(key);
                }
            }
            if node.next != 0 {
                nodes.push_back(node.next);
            }
            if node.prev != 0 {
                nodes.push_back(node.prev);
            }
        }
        let block_len = tree.block_len as usize;
        Ok(Self {
            entries,
            block_len,
            phantom: Default::default(),
        })
    }
}

struct RawTree {
    root: u32,
    block_len: u32,
    num_entries: u32,
}

impl RawTree {
    const VERSION: u32 = 1;
}

impl BigEndianIo for RawTree {
    fn read<R: Read>(mut reader: R) -> Result<Self, Error> {
        let mut magic = [0_u8; 4];
        reader.read_exact(&mut magic[..])?;
        if TREE_MAGIC[..] != magic[..] {
            return Err(Error::other("invalid tree magic"));
        }
        let version = u32::read(reader.by_ref())?;
        if version != Self::VERSION {
            return Err(Error::other(format!(
                "unsupported tree version: {}",
                version
            )));
        }
        let root = u32::read(reader.by_ref())?;
        let block_len = u32::read(reader.by_ref())?;
        let num_entries = u32::read(reader.by_ref())?;
        let _x = u8::read(reader.by_ref())?;
        Ok(Self {
            root,
            block_len,
            num_entries,
        })
    }

    fn write<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        writer.write_all(&TREE_MAGIC[..])?;
        write_be(writer.by_ref(), Self::VERSION)?;
        write_be(writer.by_ref(), self.root)?;
        write_be(writer.by_ref(), self.block_len)?;
        write_be(writer.by_ref(), self.num_entries)?;
        0_u8.write(writer.by_ref())?;
        Ok(())
    }
}

struct RawTreeNode {
    next: u32,
    prev: u32,
    entries: Vec<(u32, u32)>,
    /// Is data node or meta node?
    is_data: bool,
}

impl BigEndianIo for RawTreeNode {
    fn read<R: Read>(mut reader: R) -> Result<Self, Error> {
        let is_data = u16::read(reader.by_ref())? != 0;
        let num_entries = u16::read(reader.by_ref())?;
        let next = u32::read(reader.by_ref())?;
        let prev = u32::read(reader.by_ref())?;
        let mut entries = Vec::with_capacity(num_entries as usize);
        for _ in 0..num_entries {
            let key = u32::read(reader.by_ref())?;
            let value = u32::read(reader.by_ref())?;
            entries.push((key, value));
        }
        Ok(Self {
            next,
            prev,
            entries,
            is_data,
        })
    }

    fn write<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        let is_data: u16 = if self.is_data { 1 } else { 0 };
        let num_entries: u16 = self
            .entries
            .len()
            .try_into()
            .map_err(|_| Error::other("too many entries"))?;
        is_data.write(writer.by_ref())?;
        num_entries.write(writer.by_ref())?;
        self.next.write(writer.by_ref())?;
        self.prev.write(writer.by_ref())?;
        for (key, value) in self.entries.iter() {
            key.write(writer.by_ref())?;
            value.write(writer.by_ref())?;
        }
        Ok(())
    }
}

const fn max_enties_per_block(block_len: usize) -> usize {
    (block_len - NODE_HEADER_LEN) / ENTRY_LEN
}

// Collect N items (or less if unavailable) from the iterator into vector.
fn collect_n<T, I: Iterator<Item = T>>(iter: &mut I, n: usize) -> Vec<T> {
    let mut items = Vec::with_capacity(n);
    loop {
        if items.len() == n {
            break;
        }
        match iter.next() {
            Some(item) => items.push(item),
            None => break,
        }
    }
    items
}

const TREE_MAGIC: [u8; 4] = *b"tree";
const NODE_HEADER_LEN: usize = 2 + 2 + 4 + 4;
const ENTRY_LEN: usize = 4 + 4;

/// The size of the block that can hold one entry maximum.
const MIN_BLOCK_LEN: usize = NODE_HEADER_LEN + ENTRY_LEN;
const MAX_BLOCK_LEN: usize = 4096 * 16;

#[cfg(test)]
mod tests {

    use arbitrary::Arbitrary;
    use arbitrary::Unstructured;

    use super::*;
    use crate::receipt::Context;
    use crate::test::block_io_symmetry;
    use crate::test::test_block_io_symmetry;

    #[test]
    fn edge_cases() {
        const BLOCK_LEN: usize = 128;
        const MAX_LEN_FOR_SINGLE_DATA_NODE: usize = max_enties_per_block(BLOCK_LEN);
        const MAX_LEN_FOR_SINGLE_META_NODE: usize =
            MAX_LEN_FOR_SINGLE_DATA_NODE * MAX_LEN_FOR_SINGLE_DATA_NODE;
        test_specific_no_of_entries(MAX_LEN_FOR_SINGLE_DATA_NODE, BLOCK_LEN);
        test_specific_no_of_entries(MAX_LEN_FOR_SINGLE_DATA_NODE + 1, BLOCK_LEN);
        test_specific_no_of_entries(2 * MAX_LEN_FOR_SINGLE_DATA_NODE + 1, BLOCK_LEN);
        test_specific_no_of_entries(MAX_LEN_FOR_SINGLE_META_NODE, BLOCK_LEN);
        test_specific_no_of_entries(MAX_LEN_FOR_SINGLE_META_NODE + 1, BLOCK_LEN);
        test_specific_no_of_entries(2 * MAX_LEN_FOR_SINGLE_META_NODE + 1, BLOCK_LEN);
    }

    #[test]
    fn write_read_symmetry() {
        block_io_symmetry::<VecTree<(), (), Context>>();
        block_io_symmetry::<VecTree<u64, u8, Context>>();
        block_io_symmetry::<VecTree<VecTree<(), (), Context>, u8, Context>>();
        block_io_symmetry::<VecTree<VecTree<(), (), Context>, VecTree<(), (), Context>, Context>>();
    }

    fn test_specific_no_of_entries(num_entries: usize, block_len: usize) {
        let entries = vec![(123_u32, 456_u32); num_entries];
        let tree = VecTree::new(entries, block_len);
        test_block_io_symmetry(tree);
    }

    impl<'a, K: for<'b> Arbitrary<'b>, V: for<'c> Arbitrary<'c>, C> Arbitrary<'a> for VecTree<K, V, C> {
        fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
            Ok(Self {
                entries: u.arbitrary()?,
                block_len: u.int_in_range(MIN_BLOCK_LEN..=MAX_BLOCK_LEN)?,
                phantom: Default::default(),
            })
        }
    }
}
