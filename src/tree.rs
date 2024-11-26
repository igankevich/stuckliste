use std::io::Error;
use std::io::Read;
use std::io::Seek;
use std::io::Write;

use crate::io::*;
use crate::BlockIo;
use crate::Blocks;

pub struct TreeV2<K: BlockIo, V: BlockIo> {
    root: TreeNode<K, V>,
}

impl<K: BlockIo, V: BlockIo> TreeV2<K, V> {
    const VERSION: u32 = 1;

    pub fn into_inner(self) -> TreeNode<K, V> {
        self.root
    }
}

impl<K: BlockIo, V: BlockIo> BlockIo for TreeV2<K, V> {
    fn write<W: Write + Seek>(&self, mut writer: W, blocks: &mut Blocks) -> Result<u32, Error> {
        let i = self.root.write(writer.by_ref(), blocks)?;
        let block_size = blocks.block(i).len;
        let i = blocks.write_block(writer.by_ref(), |writer| {
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

    fn read(i: u32, file: &[u8], blocks: &mut Blocks) -> Result<Self, Error> {
        // tree
        let mut reader = blocks.slice(i, file)?;
        let block_len = reader.len();
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
        let num_paths = u32::read(reader.by_ref())?;
        let _x = u8::read(reader.by_ref())?;
        let root = TreeNode::read(child, file, blocks)?;
        // TODO this is total number of paths
        //debug_assert!(num_paths as usize == root.num_entries(), "num_paths = {num_paths}, num_entries = {}", root.num_entries());
        Ok(Self { root })
    }
}

// TODO Swap key and value ???
pub enum TreeNode<K: BlockIo, V: BlockIo> {
    Root {
        entries: Vec<(u32, V)>,
    },
    Node {
        forward: u32,
        backward: u32,
        entries: Vec<(K, V)>,
    },
}

impl<K: BlockIo, V: BlockIo> TreeNode<K, V> {
    pub fn is_leaf(&self) -> bool {
        match self {
            Self::Root { .. } => false,
            Self::Node { .. } => true,
        }
    }

    pub fn num_entries(&self) -> usize {
        match self {
            Self::Root { entries } => entries.len(),
            Self::Node { entries, .. } => entries.len(),
        }
    }
}

impl<K: BlockIo, V: BlockIo> BlockIo for TreeNode<K, V> {
    fn write<W: Write + Seek>(&self, mut writer: W, blocks: &mut Blocks) -> Result<u32, Error> {
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
                Self::Root { entries } => {
                    let forward = 0_u32;
                    let backward = 0_u32;
                    write_be(w.by_ref(), forward)?;
                    write_be(w.by_ref(), backward)?;
                    for (key, value) in entries.iter() {
                        let i =
                            blocks.write_block(writer.by_ref(), |writer| write_be(writer, *key))?;
                        let j = value.write(writer.by_ref(), blocks)?;
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
                        let i = key.write(writer.by_ref(), blocks)?;
                        let j = value.write(writer.by_ref(), blocks)?;
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
        let i = blocks.write_block(writer.by_ref(), |writer| writer.write_all(&entries_bytes))?;
        Ok(i)
    }

    fn read(i: u32, file: &[u8], blocks: &mut Blocks) -> Result<Self, Error> {
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
                    let key = K::read(i, file, blocks)?;
                    let i = u32::read(reader.by_ref())?;
                    let value = V::read(i, file, blocks)?;
                    entries.push((key, value));
                }
                Self::Node {
                    forward,
                    backward,
                    entries,
                }
            }
            false => {
                let mut entries = Vec::new();
                for _ in 0..count {
                    let key = u32::read(reader.by_ref())?;
                    let i = u32::read(reader.by_ref())?;
                    let value = V::read(i, file, blocks)?;
                    entries.push((key, value));
                }
                Self::Root { entries }
            }
        };
        Ok(node)
    }
}

// TODO hide
pub(crate) const TREE_MAGIC: [u8; 4] = *b"tree";
