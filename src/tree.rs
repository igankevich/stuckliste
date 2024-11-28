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

/// Some trees in BOM files are inverted, i.e. swap values with keys.
pub type InvertedTree<K, V, C> = Tree<V, K, C>;

#[derive(Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub struct Tree<K, V, C> {
    root: TreeNode<K, V, C>,
}

impl<C, K: BlockIo<C>, V: BlockIo<C>> Tree<K, V, C> {
    const VERSION: u32 = 1;

    pub fn new_leaf() -> Self {
        Self {
            root: TreeNode::new_leaf(),
        }
    }

    pub fn new<W: Write + Seek, I: IntoIterator<Item = (K, V)>>(
        entries: I,
        block_len: usize,
        mut writer: W,
        blocks: &mut Blocks,
        context: &mut C,
    ) -> Result<Self, Error>
    where
        <I as IntoIterator>::IntoIter: ExactSizeIterator,
    {
        let block_len = block_len.max(MIN_BLOCK_LEN);
        let n = max_enties_per_block(block_len);
        let iter = entries.into_iter();
        let num_entries = iter.len();
        if num_entries <= n {
            // One data node is enough, no need to introduce meta nodes.
            return Ok(Self {
                root: TreeNode::Node {
                    next: 0,
                    prev: 0,
                    entries: iter.collect(),
                },
            });
        }
        let num_blocks = num_entries.div_ceil(n);
        if num_blocks <= n {
            // One meta node is enough.
            let num_data_nodes = num_blocks;
            let mut data_nodes = Vec::with_capacity(num_data_nodes);
            let mut iter = iter;
            let first_block = blocks.next_block_index();
            let last_block = first_block + num_blocks as u32 - 1;
            let mut current_block = first_block;
            while iter.len() != 0 {
                let per_block_entries = collect_n(&mut iter, n);
                data_nodes.push(TreeNode::Node {
                    next: next(current_block, last_block),
                    prev: prev(current_block, first_block),
                    entries: per_block_entries,
                });
                current_block += 1;
            }
            let mut meta_entries = Vec::with_capacity(data_nodes.len());
            for data_node in data_nodes.into_iter() {
                let data_node_block = data_node.write_block(writer.by_ref(), blocks, context)?;
                // Here we rely on the fact that the last written block is the last value in the
                // TreeNode.
                let last_value_block = blocks
                    .last_block_index()
                    .expect("`while` guarantees that we don't have empty data nodes");
                meta_entries.push((data_node_block, last_value_block));
            }
            let meta_node = TreeNode::Root::<K, V, C> {
                next: 0,
                prev: 0,
                entries: meta_entries,
                _phantom: Default::default(),
            };
            return Ok(Self { root: meta_node });
        }
        // We need multiple meta nodes.
        // TODO
        unimplemented!()
    }

    pub fn into_inner(self) -> TreeNode<K, V, C> {
        self.root
    }
}

impl<C, K: BlockIo<C>, V: BlockIo<C>> BlockIo<C> for Tree<K, V, C> {
    fn write_block<W: Write + Seek>(
        &self,
        mut writer: W,
        blocks: &mut Blocks,
        context: &mut C,
    ) -> Result<u32, Error> {
        let i = self.root.write_block(writer.by_ref(), blocks, context)?;
        let block_size = blocks.block(i).len;
        let i = blocks.append(writer.by_ref(), |writer| {
            writer.write_all(&TREE_MAGIC[..])?;
            write_be(writer.by_ref(), Self::VERSION)?;
            write_be(writer.by_ref(), i)?;
            write_be(writer.by_ref(), block_size)?;
            write_be(writer.by_ref(), self.root.num_entries() as u32)?;
            0_u8.write(writer.by_ref())?;
            Ok(())
        })?;
        Ok(i)
    }

    fn read_block(
        i: u32,
        file: &[u8],
        blocks: &mut Blocks,
        context: &mut C,
    ) -> Result<Self, Error> {
        // tree
        let mut reader = blocks.slice(i, file)?;
        let _block_len = reader.len();
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
        let child = u32::read(reader.by_ref())?;
        let _block_size = u32::read(reader.by_ref())?;
        eprintln!("block len {}", _block_size);
        // TODO
        //debug_assert!(block_size as usize == block_len, "block_size = {block_size}, block_len = {block_len}");
        let _num_paths = u32::read(reader.by_ref())?;
        let _x = u8::read(reader.by_ref())?;
        let root = TreeNode::read_block(child, file, blocks, context)?;
        // TODO this is total number of paths
        //debug_assert!(num_paths as usize == root.num_entries(), "num_paths = {num_paths}, num_entries = {}", root.num_entries());
        Ok(Self { root })
    }
}

#[derive(Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub enum TreeNode<K, V, C> {
    Root {
        next: u32,
        prev: u32,
        entries: Vec<(u32, u32)>,
        _phantom: PhantomData<C>,
    },
    Node {
        next: u32,
        prev: u32,
        entries: Vec<(K, V)>,
    },
}

impl<C, K: BlockIo<C>, V: BlockIo<C>> TreeNode<K, V, C> {
    pub fn new_leaf() -> Self {
        Self::Node {
            next: 0,
            prev: 0,
            entries: Default::default(),
        }
    }

    // TODO s/leaf/data s/root/metadata
    pub fn is_leaf(&self) -> bool {
        match self {
            Self::Root { .. } => false,
            Self::Node { .. } => true,
        }
    }

    pub fn num_entries(&self) -> usize {
        match self {
            Self::Root { entries, .. } => entries.len(),
            Self::Node { entries, .. } => entries.len(),
        }
    }

    pub fn into_entries(self) -> Vec<(K, V)> {
        match self {
            Self::Root { .. } => {
                // TODO this should be an iterator
                panic!("invalid entries");
            }
            Self::Node {
                entries,
                next,
                prev,
            } => {
                debug_assert!(next == 0);
                debug_assert!(prev == 0);
                entries
            }
        }
    }
}

impl<C, K: BlockIo<C>, V: BlockIo<C>> BlockIo<C> for TreeNode<K, V, C> {
    fn write_block<W: Write + Seek>(
        &self,
        mut writer: W,
        blocks: &mut Blocks,
        context: &mut C,
    ) -> Result<u32, Error> {
        let mut entries_bytes = {
            let mut w = Vec::new();
            let count = self.num_entries();
            if count > u16::MAX as usize {
                return Err(Error::other("too many entries"));
            }
            let is_leaf: u16 = if self.is_leaf() { 1 } else { 0 };
            is_leaf.write(w.by_ref())?;
            (count as u16).write(w.by_ref())?;
            match self {
                Self::Root {
                    entries,
                    next,
                    prev,
                    ..
                } => {
                    write_be(w.by_ref(), *next)?;
                    write_be(w.by_ref(), *prev)?;
                    for (key, value) in entries.iter() {
                        let i = blocks.append(writer.by_ref(), |writer| write_be(writer, *key))?;
                        write_be(w.by_ref(), i)?;
                        write_be(w.by_ref(), *value)?;
                    }
                }
                Self::Node {
                    entries,
                    next,
                    prev,
                } => {
                    write_be(w.by_ref(), *next)?;
                    write_be(w.by_ref(), *prev)?;
                    // N.B. Tree relies on the fact that the last written block is the value.
                    for (key, value) in entries.iter() {
                        let i = key.write_block(writer.by_ref(), blocks, context)?;
                        let j = value.write_block(writer.by_ref(), blocks, context)?;
                        write_be(w.by_ref(), i)?;
                        write_be(w.by_ref(), j)?;
                    }
                }
            }
            w
        };
        let block_size = entries_bytes.len() as u32;
        let block_size = block_size
            .checked_next_multiple_of(4096)
            .unwrap_or(block_size);
        entries_bytes.resize(block_size as usize, 0_u8);
        let i = blocks.append(writer.by_ref(), |writer| writer.write_all(&entries_bytes))?;
        Ok(i)
    }

    fn read_block(
        i: u32,
        file: &[u8],
        blocks: &mut Blocks,
        context: &mut C,
    ) -> Result<Self, Error> {
        let mut reader = blocks.slice(i, file)?;
        let is_leaf = u16::read(reader.by_ref())? != 0;
        let count = u16::read(reader.by_ref())?;
        let next = u32::read(reader.by_ref())?;
        let prev = u32::read(reader.by_ref())?;
        let node = match is_leaf {
            true => {
                let mut entries = Vec::new();
                for _ in 0..count {
                    let i = u32::read(reader.by_ref())?;
                    let key = K::read_block(i, file, blocks, context)?;
                    let i = u32::read(reader.by_ref())?;
                    let value = V::read_block(i, file, blocks, context)?;
                    entries.push((key, value));
                }
                Self::Node {
                    next,
                    prev,
                    entries,
                }
            }
            false => {
                let mut entries = Vec::new();
                for _ in 0..count {
                    let key = u32::read(reader.by_ref())?;
                    let i = u32::read(reader.by_ref())?;
                    entries.push((key, i));
                }
                Self::Root {
                    next,
                    prev,
                    entries,
                    _phantom: Default::default(),
                }
            }
        };
        Ok(node)
    }
}

const fn max_enties_per_block(block_len: usize) -> usize {
    (block_len - NODE_HEADER_LEN) / ENTRY_LEN
}

const fn next(current: u32, last: u32) -> u32 {
    if current == last {
        0
    } else {
        current + 1
    }
}

const fn prev(current: u32, first: u32) -> u32 {
    if current == first {
        0
    } else {
        current - 1
    }
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

#[derive(Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub struct TreeBased<I, K, V, C>(I, PhantomData<C>)
where
    I: IntoIterator<Item = (K, V)> + Clone + From<Vec<(K, V)>>,
    <I as IntoIterator>::IntoIter: ExactSizeIterator,
    K: BigEndianIo,
    V: BigEndianIo;

impl<I, K, V, C> Deref for TreeBased<I, K, V, C>
where
    I: IntoIterator<Item = (K, V)> + Clone + From<Vec<(K, V)>>,
    <I as IntoIterator>::IntoIter: ExactSizeIterator,
    K: BigEndianIo,
    V: BigEndianIo,
{
    type Target = I;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<I, K, V, C> DerefMut for TreeBased<I, K, V, C>
where
    I: IntoIterator<Item = (K, V)> + Clone + From<Vec<(K, V)>>,
    <I as IntoIterator>::IntoIter: ExactSizeIterator,
    K: BigEndianIo,
    V: BigEndianIo,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<I, K, V, C> BlockIo<C> for TreeBased<I, K, V, C>
where
    I: IntoIterator<Item = (K, V)> + Clone + From<Vec<(K, V)>>,
    <I as IntoIterator>::IntoIter: ExactSizeIterator,
    K: BigEndianIo,
    V: BigEndianIo,
{
    fn write_block<W: Write + Seek>(
        &self,
        mut writer: W,
        blocks: &mut Blocks,
        context: &mut C,
    ) -> Result<u32, Error> {
        let tree = Tree::<K, V, C>::new(self.0.clone(), 4096, writer.by_ref(), blocks, context)?;
        tree.write_block(writer.by_ref(), blocks, context)
    }

    fn read_block(
        i: u32,
        file: &[u8],
        blocks: &mut Blocks,
        context: &mut C,
    ) -> Result<Self, Error> {
        let tree = Tree::<K, V, C>::read_block(i, file, blocks, context)?;
        let mut entries = Vec::new();
        for (k, v) in tree.into_inner().into_entries() {
            entries.push((k, v));
        }
        Ok(Self(entries.into(), Default::default()))
    }
}

const TREE_MAGIC: [u8; 4] = *b"tree";
const NODE_HEADER_LEN: usize = 2 + 2 + 4 + 4;
const ENTRY_LEN: usize = 4 + 4;

/// The size of the block that can hold one entry maximum.
const MIN_BLOCK_LEN: usize = NODE_HEADER_LEN + ENTRY_LEN;

#[cfg(test)]
mod tests {

    use super::*;
    use crate::receipt::Context;
    use crate::test::block_io_symmetry;

    #[test]
    fn write_read_symmetry() {
        block_io_symmetry::<TreeBased<Vec<((), ())>, (), (), Context>>();
        // TODO large tree
    }
}
