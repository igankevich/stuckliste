use std::collections::HashSet;
use std::collections::VecDeque;
use std::io::Error;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
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
        let num_data_blocks = num_entries.div_ceil(n);
        let num_data_nodes = num_data_blocks;
        let mut data_nodes = Vec::with_capacity(num_data_nodes);
        let mut iter = iter;
        while iter.len() != 0 {
            let per_block_entries = collect_n(&mut iter, n);
            data_nodes.push(TreeNode::Node {
                next: 0,
                prev: 0,
                entries: per_block_entries,
            });
        }
        let mut meta_entries = Vec::with_capacity(data_nodes.len());
        let mut data_blocks = Vec::new();
        for data_node in data_nodes.iter() {
            let data_node_block = data_node.write_block(writer.by_ref(), blocks, context)?;
            data_blocks.push(data_node_block);
            // Here we rely on the fact that the last written block is the last value in the
            // TreeNode.
            let last_value_block = blocks
                .last_block_index()
                .expect("`while` guarantees that we don't have empty data nodes");
            meta_entries.push((data_node_block, last_value_block));
        }
        // overwrite next/prev fields
        for (i, block) in data_blocks.iter().enumerate() {
            let next = if i == data_blocks.len() - 1 {
                0
            } else {
                data_blocks[i + 1]
            };
            let prev = if i == 0 { 0 } else { data_blocks[i - 1] };
            data_nodes[i].overwrite_next_prev(next, prev, *block, writer.by_ref(), blocks)?;
        }
        if num_data_blocks <= n {
            // One meta node is enough.
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

    pub fn new_debug<W: Write + Seek, I: IntoIterator<Item = (K, V)>>(
        entries: I,
        block_len: usize,
        mut writer: W,
        blocks: &mut Blocks,
        context: &mut C,
    ) -> Result<Self, Error>
    where
        <I as IntoIterator>::IntoIter: ExactSizeIterator,
        K: std::fmt::Debug,
        V: std::fmt::Debug,
        C: std::fmt::Debug,
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
        let num_data_blocks = num_entries.div_ceil(n);
        let num_data_nodes = num_data_blocks;
        let mut data_nodes = Vec::with_capacity(num_data_nodes);
        let mut iter = iter;
        while iter.len() != 0 {
            let per_block_entries = collect_n(&mut iter, n);
            data_nodes.push(TreeNode::Node {
                next: 0,
                prev: 0,
                entries: per_block_entries,
            });
        }
        eprintln!("data nodes {:#?}", data_nodes);
        let mut meta_entries = Vec::with_capacity(data_nodes.len());
        let mut data_blocks = Vec::new();
        for data_node in data_nodes.iter() {
            let data_node_block = data_node.write_block(writer.by_ref(), blocks, context)?;
            data_blocks.push(data_node_block);
            // Here we rely on the fact that the last written block is the last value in the
            // TreeNode.
            let last_value_block = blocks
                .last_block_index()
                .expect("`while` guarantees that we don't have empty data nodes");
            meta_entries.push((data_node_block, last_value_block - 1));
        }
        // overwrite next/prev fields
        for (i, block) in data_blocks.iter().enumerate() {
            let next = if i == data_blocks.len() - 1 {
                0
            } else {
                data_blocks[i + 1]
            };
            let prev = if i == 0 { 0 } else { data_blocks[i - 1] };
            data_nodes[i].overwrite_next_prev(next, prev, *block, writer.by_ref(), blocks)?;
        }
        if num_data_blocks <= n {
            // One meta node is enough.
            let meta_node = TreeNode::Root::<K, V, C> {
                next: 0,
                prev: 0,
                entries: meta_entries,
                _phantom: Default::default(),
            };
            eprintln!("meta nodes {:#?}", meta_node);
            return Ok(Self { root: meta_node });
        }
        // We need multiple meta nodes.
        let num_meta_blocks = num_data_blocks.div_ceil(n);
        let num_meta_nodes = num_meta_blocks;
        let mut meta_nodes = Vec::with_capacity(num_meta_nodes);
        let mut iter = meta_entries.into_iter();
        while iter.len() != 0 {
            let per_block_entries = collect_n(&mut iter, n);
            meta_nodes.push(TreeNode::Root {
                next: 0,
                prev: 0,
                entries: per_block_entries,
                _phantom: Default::default(),
            });
        }
        // TODO we need to retain only Tree structure, TreeNode complicates things...
        // TODO split into meta nodes first, then into data nodes
        eprintln!("meta nodes {:#?}", meta_nodes);
        Ok(Self {
            root: meta_nodes.remove(0),
        })
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

    pub fn read_into<T: FromIterator<(K, V)>>(
        self,
        file: &[u8],
        blocks: &mut Blocks,
        context: &mut C,
    ) -> Result<T, Error>
    where
        K: std::fmt::Debug,
        V: std::fmt::Debug,
        C: std::fmt::Debug,
    {
        let mut all_entries = Vec::new();
        let mut paths = VecDeque::new();
        // TODO not zero
        paths.push_back(self);
        let mut visited = HashSet::new();
        while let Some(tree_node) = paths.pop_front() {
            eprintln!("tree node {:?}", tree_node);
            match tree_node {
                TreeNode::Root { entries, .. } => {
                    for (index, last_entry) in entries.into_iter() {
                        if visited.insert(index) {
                            eprintln!("read tree node {:?} last entry {:?}", index, last_entry);
                            let tree_node = TreeNode::read_block(index, &file, blocks, context)?;
                            paths.push_back(tree_node);
                        }
                    }
                }
                TreeNode::Node {
                    entries,
                    next,
                    prev,
                } => {
                    all_entries.extend(entries);
                    if next != 0 {
                        let i = next;
                        // TODO TreeNode::read only once
                        if visited.insert(i) {
                            let tree_node = TreeNode::read_block(i, &file, blocks, context)?;
                            paths.push_back(tree_node);
                        }
                    }
                    if prev != 0 {
                        let i = prev;
                        if visited.insert(i) {
                            let tree_node = TreeNode::read_block(i, &file, blocks, context)?;
                            paths.push_back(tree_node);
                        }
                    }
                }
            }
        }
        Ok(T::from_iter(all_entries.into_iter()))
    }

    fn overwrite_next_prev<W: Write + Seek>(
        &self,
        next: u32,
        prev: u32,
        i: u32,
        mut writer: W,
        blocks: &mut Blocks,
    ) -> Result<(), Error> {
        const NEXT_OFFSET: u64 = 2 + 2;
        let old_position = writer.stream_position()?;
        writer.seek(SeekFrom::Start(blocks.block(i).offset as u64 + NEXT_OFFSET))?;
        write_be(writer.by_ref(), next)?;
        write_be(writer.by_ref(), prev)?;
        writer.seek(SeekFrom::Start(old_position))?;
        Ok(())
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
                        write_be(w.by_ref(), *key)?;
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
        // TODO
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
pub struct TreeBased<I, K, V, C>(I, usize, PhantomData<C>)
where
    I: IntoIterator<Item = (K, V)> + Clone + From<Vec<(K, V)>>,
    <I as IntoIterator>::IntoIter: ExactSizeIterator,
    K: BigEndianIo,
    V: BigEndianIo;

impl<I, K, V, C> TreeBased<I, K, V, C>
where
    I: IntoIterator<Item = (K, V)> + Clone + From<Vec<(K, V)>>,
    <I as IntoIterator>::IntoIter: ExactSizeIterator,
    K: BigEndianIo,
    V: BigEndianIo,
{
    pub fn new(other: I, block_len: usize) -> Self {
        Self(other, block_len, Default::default())
    }

    const BLOCK_LEN: usize = 4096;
}

impl<I, K, V, C> From<I> for TreeBased<I, K, V, C>
where
    I: IntoIterator<Item = (K, V)> + Clone + From<Vec<(K, V)>>,
    <I as IntoIterator>::IntoIter: ExactSizeIterator,
    K: BigEndianIo,
    V: BigEndianIo,
{
    fn from(other: I) -> Self {
        Self(other, Self::BLOCK_LEN, Default::default())
    }
}

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
        let block_len = self.1;
        let tree =
            Tree::<K, V, C>::new(self.0.clone(), block_len, writer.by_ref(), blocks, context)?;
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
        // TODO block len
        Ok(Self(entries.into(), Self::BLOCK_LEN, Default::default()))
    }
}

const TREE_MAGIC: [u8; 4] = *b"tree";
const NODE_HEADER_LEN: usize = 2 + 2 + 4 + 4;
const ENTRY_LEN: usize = 4 + 4;

/// The size of the block that can hold one entry maximum.
const MIN_BLOCK_LEN: usize = NODE_HEADER_LEN + ENTRY_LEN;
const MAX_BLOCK_LEN: usize = 4096 * 16;

#[cfg(test)]
mod tests {

    use std::fmt::Debug;
    use std::io::Cursor;

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
        eprintln!(
            "MAX_LEN_FOR_SINGLE_DATA_NODE = {}",
            MAX_LEN_FOR_SINGLE_DATA_NODE
        );
        eprintln!(
            "MAX_LEN_FOR_SINGLE_META_NODE = {}",
            MAX_LEN_FOR_SINGLE_META_NODE
        );
        test_specific_no_of_entries(MAX_LEN_FOR_SINGLE_DATA_NODE - 1, BLOCK_LEN);
        test_specific_no_of_entries(MAX_LEN_FOR_SINGLE_DATA_NODE, BLOCK_LEN);
        test_specific_no_of_entries(MAX_LEN_FOR_SINGLE_DATA_NODE + 1, BLOCK_LEN);
        test_specific_no_of_entries(2 * MAX_LEN_FOR_SINGLE_DATA_NODE + 1, BLOCK_LEN);
        test_specific_no_of_entries(MAX_LEN_FOR_SINGLE_META_NODE - 1, BLOCK_LEN);
        test_specific_no_of_entries(MAX_LEN_FOR_SINGLE_META_NODE, BLOCK_LEN);
        test_specific_no_of_entries(MAX_LEN_FOR_SINGLE_META_NODE + 1, BLOCK_LEN);
    }

    fn test_specific_no_of_entries(num_entries: usize, block_len: usize) {
        let entries = vec![(123_u32, 456_u32); num_entries];
        tree_block_io_symmetry(entries, block_len);
    }

    #[test]
    fn edge_cases_v2() {
        const BLOCK_LEN: usize = 128;
        const MAX_LEN_FOR_SINGLE_DATA_NODE: usize = max_enties_per_block(BLOCK_LEN);
        const MAX_LEN_FOR_SINGLE_META_NODE: usize =
            MAX_LEN_FOR_SINGLE_DATA_NODE * MAX_LEN_FOR_SINGLE_DATA_NODE;
        test_specific_no_of_entries_v2(MAX_LEN_FOR_SINGLE_DATA_NODE, BLOCK_LEN);
        test_specific_no_of_entries_v2(MAX_LEN_FOR_SINGLE_DATA_NODE + 1, BLOCK_LEN);
        test_specific_no_of_entries_v2(2 * MAX_LEN_FOR_SINGLE_DATA_NODE + 1, BLOCK_LEN);
        test_specific_no_of_entries_v2(MAX_LEN_FOR_SINGLE_META_NODE, BLOCK_LEN);
        test_specific_no_of_entries_v2(MAX_LEN_FOR_SINGLE_META_NODE + 1, BLOCK_LEN);
        test_specific_no_of_entries_v2(2 * MAX_LEN_FOR_SINGLE_META_NODE + 1, BLOCK_LEN);
    }

    #[test]
    fn write_read_symmetry() {
        block_io_symmetry::<VecTree<(), (), Context>>();
        block_io_symmetry::<VecTree<u64, u8, Context>>();
        block_io_symmetry::<VecTree<VecTree<(), (), Context>, u8, Context>>();
        block_io_symmetry::<VecTree<VecTree<(), (), Context>, VecTree<(), (), Context>, Context>>();
    }

    fn test_specific_no_of_entries_v2(num_entries: usize, block_len: usize) {
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

    fn tree_block_io_symmetry<K, V>(expected: Vec<(K, V)>, block_len: usize)
    where
        K: BigEndianIo + Clone + Debug + PartialEq,
        V: BigEndianIo + Clone + Debug + PartialEq,
    {
        let mut blocks = Blocks::new();
        let mut context = Context::new();
        let mut writer = Cursor::new(Vec::new());
        let tree = Tree::new_debug(
            expected.clone(),
            block_len,
            &mut writer,
            &mut blocks,
            &mut context,
        )
        .unwrap();
        let i = tree
            .write_block(&mut writer, &mut blocks, &mut context)
            .unwrap();
        let bytes = writer.into_inner();
        let actual_tree =
            Tree::<K, V, Context>::read_block(i, &bytes[..], &mut blocks, &mut context).unwrap();
        let actual: Vec<(K, V)> = actual_tree
            .into_inner()
            .read_into(&bytes[..], &mut blocks, &mut context)
            .unwrap();
        assert_eq!(expected, actual);
    }
}
