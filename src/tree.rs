use std::io::Error;
use std::io::Read;
use std::io::Seek;
use std::io::Write;
use std::marker::PhantomData;

use crate::io::*;
use crate::BlockIo;
use crate::Blocks;

#[derive(Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub struct TreeV2<K, V, C> {
    root: TreeNode<K, V, C>,
}

impl<C, K: BlockIo<C>, V: BlockIo<C>> TreeV2<K, V, C> {
    const VERSION: u32 = 1;

    pub fn new_leaf() -> Self {
        Self {
            root: TreeNode::new_leaf(),
        }
    }

    pub fn new<I: IntoIterator<Item = (K, V)>>(entries: I) -> Self {
        // TODO group by block size
        let entries = entries.into_iter().collect();
        Self {
            root: TreeNode::Node {
                forward: 0,
                backward: 0,
                entries,
            },
        }
    }

    /// Some trees in BOM files are inverted, i.e. swap values with keys.
    pub fn new_inverted<I: IntoIterator<Item = (V, K)>>(entries: I) -> Self {
        // TODO group by block size
        let entries = entries.into_iter().map(|(k, v)| (v, k)).collect();
        Self {
            root: TreeNode::Node {
                forward: 0,
                backward: 0,
                entries,
            },
        }
    }

    pub fn into_inner(self) -> TreeNode<K, V, C> {
        self.root
    }
}

impl<C, K: BlockIo<C>, V: BlockIo<C>> BlockIo<C> for TreeV2<K, V, C> {
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
        entries: Vec<(u32, V)>,
        _phantom: PhantomData<C>,
    },
    Node {
        forward: u32,
        backward: u32,
        entries: Vec<(K, V)>,
    },
}

impl<C, K: BlockIo<C>, V: BlockIo<C>> TreeNode<K, V, C> {
    pub fn new_leaf() -> Self {
        Self::Node {
            forward: 0,
            backward: 0,
            entries: Default::default(),
        }
    }

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
                forward,
                backward,
            } => {
                debug_assert!(forward == 0);
                debug_assert!(backward == 0);
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
                Self::Root { entries, .. } => {
                    let forward = 0_u32;
                    let backward = 0_u32;
                    write_be(w.by_ref(), forward)?;
                    write_be(w.by_ref(), backward)?;
                    for (key, value) in entries.iter() {
                        let i = blocks.append(writer.by_ref(), |writer| write_be(writer, *key))?;
                        let j = value.write_block(writer.by_ref(), blocks, context)?;
                        write_be(w.by_ref(), i)?;
                        write_be(w.by_ref(), j)?;
                    }
                }
                Self::Node {
                    entries,
                    forward,
                    backward,
                } => {
                    write_be(w.by_ref(), *forward)?;
                    write_be(w.by_ref(), *backward)?;
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
        let forward = u32::read(reader.by_ref())?;
        let backward = u32::read(reader.by_ref())?;
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
                    forward,
                    backward,
                    entries,
                }
            }
            false => {
                debug_assert!(forward == 0);
                debug_assert!(backward == 0);
                let mut entries = Vec::new();
                for _ in 0..count {
                    let key = u32::read(reader.by_ref())?;
                    let i = u32::read(reader.by_ref())?;
                    let value = V::read_block(i, file, blocks, context)?;
                    entries.push((key, value));
                }
                Self::Root {
                    entries,
                    _phantom: Default::default(),
                }
            }
        };
        Ok(node)
    }
}

const TREE_MAGIC: [u8; 4] = *b"tree";
