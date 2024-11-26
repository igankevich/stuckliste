use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::ffi::CStr;
use std::ffi::CString;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::fs::File;
use std::io::Error;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::ops::Deref;
use std::ops::DerefMut;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::path::PathBuf;

use normalize_path::NormalizePath;
use walkdir::WalkDir;

use crate::CrcReader;
use crate::FileType;

#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq, Debug))]
pub struct Bom {
    nodes: Nodes,
}

impl Bom {
    pub fn paths(&self) -> Result<HashMap<PathBuf, Metadata>, Error> {
        self.nodes.to_paths()
    }

    pub fn from_directory<P: AsRef<Path>>(directory: P) -> Result<Self, Error> {
        let nodes = Nodes::from_directory(directory)?;
        Ok(Self { nodes })
    }

    pub fn write<W: Write + Seek>(&self, mut writer: W) -> Result<(), Error> {
        // skip the header
        writer.seek(SeekFrom::Start(HEADER_LEN as u64))?;
        let mut blocks = Blocks::new();
        let mut named_blocks = NamedBlocks::new();
        // v index
        {
            let paths = Paths::null();
            let i = blocks.write_block(writer.by_ref(), |writer| paths.write(writer))?;
            let tree = Tree::new_v_index(i);
            let i = blocks.write_block(writer.by_ref(), |writer| tree.write(writer))?;
            let vindex = VIndex::new(i);
            let i = blocks.write_block(writer.by_ref(), |writer| vindex.write(writer))?;
            named_blocks.insert(V_INDEX.into(), i);
        }
        // hl index
        {
            let paths = Paths::null();
            let i = blocks.write_block(writer.by_ref(), |writer| paths.write(writer))?;
            let tree = Tree::null(i);
            let i = blocks.write_block(writer.by_ref(), |writer| tree.write(writer))?;
            named_blocks.insert(HL_INDEX.into(), i);
        }
        let mut file_size_64 = HashMap::new();
        // paths
        let num_paths = {
            let edges = self.nodes.edges();
            eprintln!("write edges {:?}", edges);
            let num_paths = edges.len() as u32;
            let mut roots = Vec::new();
            let mut all_paths = Vec::new();
            for (parent, children) in edges.iter() {
                let mut indices = Vec::new();
                for child in children.iter() {
                    let node = self.nodes.nodes.get(child).unwrap();
                    // node metadata
                    let i = blocks
                        .write_block(writer.by_ref(), |writer| node.metadata.write(writer))?;
                    if node.metadata.size > u32::MAX as u64 {
                        file_size_64.insert(i, node.metadata.size);
                    }
                    // node id -> index mapping
                    let index0 = blocks.write_block(writer.by_ref(), |writer| {
                        u32_write(writer.by_ref(), node.id)?;
                        u32_write(writer.by_ref(), i)?;
                        Ok(())
                    })?;
                    // parent + name
                    let index1 = blocks.write_block(writer.by_ref(), |writer| {
                        u32_write(writer.by_ref(), node.parent)?;
                        writer.write_all(node.name.as_os_str().as_bytes())?;
                        writer.write_all(&[0_u8])?;
                        Ok(())
                    })?;
                    indices.push((index0, index1));
                }
                let last_index = indices.last().cloned().unwrap();
                let paths = Paths::from_indices(indices);
                all_paths.push((parent, last_index, paths));
            }
            let block_index = blocks.next_block_index();
            let n = all_paths.len();
            for (i, (_, _, paths)) in all_paths.iter_mut().enumerate() {
                paths.backward = if i == 0 {
                    0
                } else {
                    block_index + (i - 1) as u32
                };
                paths.forward = if i == n - 1 {
                    0
                } else {
                    block_index + (i + 1) as u32
                };
            }
            for (j, (parent, last_index, paths)) in all_paths.into_iter().enumerate() {
                let i = blocks.write_block(writer.by_ref(), |writer| paths.write(writer))?;
                debug_assert!(i == block_index + j as u32);
                eprintln!("write index {} paths {:?}", i, paths);
                // if root
                if *parent == 0 {
                    // take the last file (can be any file probably)
                    let index1 = last_index.1;
                    roots.push((i, index1));
                }
            }
            // paths (is_leaf == 0)
            {
                let num_paths = roots.len() as u32;
                let mut paths = Paths::from_indices(roots);
                paths.is_leaf = false;
                let i = blocks.write_block(writer.by_ref(), |writer| paths.write(writer))?;
                let tree = Tree::new(i, num_paths);
                let i = blocks.write_block(writer.by_ref(), |writer| tree.write(writer))?;
                named_blocks.insert(PATHS.into(), i);
            }
            num_paths
        };
        // size 64
        {
            let mut indices = Vec::new();
            eprintln!("write file_size_64 {:#?}", file_size_64);
            for (metadata_index, file_size) in file_size_64.into_iter() {
                let i =
                    blocks.write_block(writer.by_ref(), |writer| u64_write(writer, file_size))?;
                let j = blocks
                    .write_block(writer.by_ref(), |writer| u32_write(writer, metadata_index))?;
                indices.push((i, j));
            }
            let num_paths = indices.len() as u32;
            let paths = Paths::from_indices(indices);
            let i = blocks.write_block(writer.by_ref(), |writer| paths.write(writer))?;
            let tree = Tree::new(i, num_paths);
            let i = blocks.write_block(writer.by_ref(), |writer| tree.write(writer))?;
            named_blocks.insert(SIZE_64.into(), i);
        }
        // bom info
        {
            let bom_info = BomInfo {
                num_paths,
                entries: Default::default(),
            };
            let i = blocks.write_block(writer.by_ref(), |writer| bom_info.write(writer))?;
            named_blocks.insert(BOM_INFO.into(), i);
        }
        // named_blocks
        let named_blocks_block =
            Block::from_write(writer.by_ref(), |writer| named_blocks.write(writer))?;
        let index_block = Block::from_write(writer.by_ref(), |writer| blocks.write(writer))?;
        // write the header
        writer.seek(SeekFrom::Start(0))?;
        let header = Header {
            num_non_null_blocks: blocks.num_non_null_blocks() as u32,
            index: Block {
                offset: index_block.offset,
                len: index_block.len,
            },
            named_blocks: Block {
                offset: named_blocks_block.offset,
                len: named_blocks_block.len,
            },
        };
        header.write(writer.by_ref())?;
        let paths = self.nodes.to_paths()?;
        for (path, metadata) in paths.iter() {
            eprintln!("write path {:?} metadata {:?}", path, metadata);
        }
        Ok(())
    }

    pub fn read<R: Read>(mut reader: R) -> Result<Self, Error> {
        let mut file = Vec::new();
        reader.read_to_end(&mut file)?;
        let header = Header::read(&file[..HEADER_LEN])?;
        eprintln!("{header:#?}");
        let mut named_blocks = NamedBlocks::read(header.named_blocks.slice(&file))?;
        eprintln!("{:#?}", named_blocks);
        let mut blocks = Blocks::read(header.index.slice(&file))?;
        {
            let name = BOM_INFO;
            let index = named_blocks
                .remove(name)
                .ok_or_else(|| Error::other(format!("{:?} is missing", name)))?;
            let bom_info = BomInfo::read(blocks.slice(index, &file)?)?;
            eprintln!("{:?}", bom_info);
        }
        {
            let name = V_INDEX;
            let index = named_blocks
                .remove(name)
                .ok_or_else(|| Error::other(format!("{:?} is missing", name)))?;
            let v_index = VIndex::read(blocks.slice(index, &file)?)?;
            let name: CString = c"VIndex.index".into();
            let tree = Tree::read(blocks.slice(v_index.index, &file)?)?;
            eprintln!("tree {:?} {:?}", name, tree);
            let paths = Paths::read(blocks.slice(tree.child, &file)?)?;
            for (index0, index1) in paths.indices.into_iter() {
                let block_bytes = blocks.slice(index0, &file)?;
                let index = u32_read_v2(&block_bytes[..])?;
                if index != 0 {
                    let tree = Tree::read(blocks.slice(index, &file)?)?;
                    eprintln!("vindex inner tree {:?}", tree);
                    let paths = Paths::read(blocks.slice(tree.child, &file)?)?;
                    for (index0, index1) in paths.indices.into_iter() {
                        let block_bytes = blocks.slice(index0, &file)?;
                        debug_assert!(block_bytes.is_empty());
                        let block_bytes = blocks.slice(index1, &file)?;
                        let name =
                            CStr::from_bytes_with_nul(&block_bytes[..]).map_err(Error::other)?;
                        eprintln!(
                            "vindex inner block name {:?} {}: {:?}",
                            name, index1, block_bytes
                        );
                    }
                }
                let block_bytes = blocks.slice(index1, &file)?;
                let name = CStr::from_bytes_with_nul(&block_bytes[..]).map_err(Error::other)?;
                eprintln!("vindex name {:?} block {}: {:?}", name, index1, block_bytes);
            }
        }
        // block id -> file size
        let mut file_size_64 = HashMap::new();
        if let Some(index) = named_blocks.remove(SIZE_64) {
            let tree = Tree::read(blocks.slice(index, &file)?)?;
            eprintln!("tree {:?} {:?}", SIZE_64, tree);
            let paths = Paths::read(blocks.slice(tree.child, &file)?)?;
            eprintln!("size64 paths {:?}", paths);
            for (index0, index1) in paths.indices.into_iter() {
                let block_bytes = blocks.slice(index0, &file)?;
                let file_size = u64_read(&block_bytes[..])?;
                eprintln!("block {}: {:?}", index0, block_bytes);
                let block_bytes = blocks.slice(index1, &file)?;
                eprintln!("block {}: {:?}", index1, block_bytes);
                let metadata_index = u32_read_v2(&block_bytes[..])?;
                file_size_64.insert(metadata_index, file_size);
            }
        }
        if let Some(index) = named_blocks.remove(HL_INDEX) {
            let tree = Tree::read(blocks.slice(index, &file)?)?;
            eprintln!("tree {:?} {:?}", HL_INDEX, tree);
            let paths = Paths::read(blocks.slice(tree.child, &file)?)?;
            eprintln!("hl-index paths {:?}", paths);
            for (index0, index1) in paths.indices.into_iter() {
                let block_bytes = blocks.slice(index0, &file)?;
                let id = u32_read(&block_bytes[..4]);
                // id points to a tree
                let tree = Tree::read(blocks.slice(id, &file)?)?;
                eprintln!("tree {:?} {:?}", "hard-link", tree);
                let block_bytes = blocks.slice(index1, &file)?;
                let metadata_index = u32_read(&block_bytes[0..4]);
                let index = tree.child;
                let path = Paths::read(blocks.slice(index, &file)?)?;
                debug_assert!(path.is_leaf);
                eprintln!("read index {} paths {:?} hard-link", index, path);
                for (index0, index1) in path.indices.into_iter() {
                    let block_bytes = blocks.slice(index0, &file)?;
                    eprintln!("block {}: {:?}", index0, block_bytes);
                    debug_assert!(block_bytes.is_empty());
                    let block_bytes = blocks.slice(index1, &file)?;
                    let name = CStr::from_bytes_with_nul(&block_bytes[..]).map_err(Error::other)?;
                    let name = OsStr::from_bytes(name.to_bytes());
                    eprintln!("hard-link {:?} -> {}", name.to_str(), metadata_index);
                }
            }
        }
        // id -> data
        let mut nodes = HashMap::new();
        if let Some(index) = named_blocks.remove(PATHS) {
            let tree = Tree::read(blocks.slice(index, &file)?)?;
            eprintln!("tree {:?} {:?}", PATHS, tree);
            let mut paths = VecDeque::new();
            paths.push_back((PATHS, tree.child));
            let mut visited = HashSet::new();
            while let Some((name, index)) = paths.pop_front() {
                if !visited.insert(index) {
                    //eprintln!("loop {}", index);
                    continue;
                }
                let path = Paths::read(blocks.slice(index, &file)?)?;
                if !path.is_leaf {
                    eprintln!(
                        "branch id {} forward {} backward {} indices {:?}",
                        index, path.forward, path.backward, path.indices
                    );
                }
                eprintln!("read index {} paths {:?} name {:?}", index, path, name);
                for (index0, index1) in path.indices.into_iter() {
                    let child = if !path.is_leaf {
                        paths.push_back((c"paths.root", index0));
                        // index1 appears to be irrelevant here
                        // (equals to index1 of the last file in the referenced Paths)
                        None
                    } else {
                        let block_bytes = blocks.slice(index0, &file)?;
                        eprintln!("block {}: {:?}", index0, block_bytes);
                        if block_bytes.len() == 0 {
                            None
                        } else {
                            let id = u32_read(&block_bytes[0..4]);
                            eprintln!("id {}", id);
                            let index = u32_read(&block_bytes[4..8]);
                            let block_bytes = blocks.slice(index, &file)?;
                            let mut cursor = std::io::Cursor::new(block_bytes);
                            let mut metadata = Metadata::read(cursor.by_ref())?;
                            if let Some(size) = file_size_64.get(&index) {
                                metadata.size = *size;
                            }
                            let unread_bytes = block_bytes.len() - cursor.position() as usize;
                            debug_assert!(unread_bytes == 0, "unread_bytes = {unread_bytes}");
                            let node = Node {
                                id,
                                metadata,
                                parent: 0,
                                name: Default::default(),
                            };
                            Some(node)
                        }
                    };
                    {
                        let block_bytes = blocks.slice(index1, &file)?;
                        eprintln!("block {}: {:?}", index1, block_bytes);
                        let parent = u32_read(&block_bytes[0..4]);
                        let name =
                            CStr::from_bytes_with_nul(&block_bytes[4..]).map_err(Error::other)?;
                        let name = OsStr::from_bytes(name.to_bytes());
                        if !path.is_leaf {
                            eprintln!("branch ({},{}) parent {} name {:?}", index0, index1, parent, name.to_str());
                        }
                        //eprintln!("file parent {} name {}", parent, name,);
                        if let Some(mut child) = child {
                            eprintln!(
                                "name {:?} id {} parent {} kind {:?} metadata {:?}",
                                name,
                                child.id,
                                parent,
                                child.metadata.file_type(),
                                child.metadata,
                            );
                            child.name = name.into();
                            child.parent = parent;
                            nodes.insert(child.id, child);
                        }
                    }
                }
                if name != c"paths.root" {
                    debug_assert!(path.forward == 0);
                    debug_assert!(path.backward == 0);
                }
                if path.forward != 0 {
                    paths.push_back((name, path.forward));
                }
                if path.backward != 0 {
                    paths.push_back((name, path.backward));
                }
            }
        }
        debug_assert!(named_blocks.named_blocks.is_empty());
        #[cfg(test)]
        blocks.print_unread_blocks();
        let nodes = Nodes { nodes };
        eprintln!("nodes {:#?}", nodes);
        let paths = nodes.to_paths()?;
        for (path, metadata) in paths.iter() {
            eprintln!("read path {:?} metadata {:?}", path, metadata);
        }
        Ok(Self { nodes })
    }
}

#[derive(Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
struct Header {
    num_non_null_blocks: u32,
    index: Block,
    named_blocks: Block,
}

impl BigEndianIo for Header {
    fn read<R: Read>(mut reader: R) -> Result<Self, Error> {
        let mut file = [0_u8; HEADER_LEN];
        reader.read_exact(&mut file[..])?;
        if file[..BOM_MAGIC.len()] != BOM_MAGIC[..] {
            return Err(Error::other("not a bom store"));
        }
        let version = u32_read(&file[8..12]);
        if version != 1 {
            return Err(Error::other(format!(
                "unsupported BOM store version: {}",
                version
            )));
        }
        let num_non_null_blocks = u32_read(&file[12..16]);
        let index_offset = u32_read(&file[16..20]);
        let index_len = u32_read(&file[20..24]);
        let named_blocks_offset = u32_read(&file[24..28]);
        let named_blocks_len = u32_read(&file[28..32]);
        Ok(Self {
            num_non_null_blocks,
            index: Block {
                offset: index_offset,
                len: index_len,
            },
            named_blocks: Block {
                offset: named_blocks_offset,
                len: named_blocks_len,
            },
        })
    }

    fn write<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        writer.write_all(&BOM_MAGIC[..])?;
        u32_write(writer.by_ref(), VERSION)?;
        u32_write(writer.by_ref(), self.num_non_null_blocks)?;
        u32_write(writer.by_ref(), self.index.offset)?;
        u32_write(writer.by_ref(), self.index.len)?;
        u32_write(writer.by_ref(), self.named_blocks.offset)?;
        u32_write(writer.by_ref(), self.named_blocks.len)?;
        writer.write_all(&[0_u8; HEADER_PADDING])?;
        Ok(())
    }
}

#[derive(Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
struct NamedBlocks {
    /// Name -> index.
    named_blocks: HashMap<CString, u32>,
}

impl NamedBlocks {
    fn new() -> Self {
        Self {
            named_blocks: Default::default(),
        }
    }
}

impl BigEndianIo for NamedBlocks {
    fn read<R: Read>(mut reader: R) -> Result<Self, Error> {
        let num_named_blocks = u32_read_v2(reader.by_ref())? as usize;
        let mut named_blocks = HashMap::with_capacity(num_named_blocks);
        for _ in 0..num_named_blocks {
            let index = u32_read_v2(reader.by_ref())?;
            let len = u8_read(reader.by_ref())? as usize;
            let mut name = vec![0_u8; len];
            reader.read_exact(&mut name[..])?;
            // remove the null character if any
            if let Some(i) = name.iter().position(|b| *b == 0) {
                name.truncate(i);
            };
            let name = CString::new(name).map_err(|_| Error::other("invalid variable name"))?;
            named_blocks.insert(name, index);
        }
        //eprintln!("named_blocks {:?}", named_blocks);
        Ok(Self { named_blocks })
    }

    fn write<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        let num_named_blocks = self.named_blocks.len() as u32;
        u32_write(writer.by_ref(), num_named_blocks)?;
        for (name, index) in self.named_blocks.iter() {
            let name = name.to_bytes();
            let len = name.len();
            if len > u8::MAX as usize {
                return Err(Error::other("variable name is too long"));
            }
            u32_write(writer.by_ref(), *index)?;
            writer.write_all(&[len as u8])?;
            writer.write_all(name)?;
        }
        Ok(())
    }
}

impl Deref for NamedBlocks {
    type Target = HashMap<CString, u32>;
    fn deref(&self) -> &Self::Target {
        &self.named_blocks
    }
}

impl DerefMut for NamedBlocks {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.named_blocks
    }
}

#[derive(Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary))]
struct Blocks {
    blocks: Vec<Block>,
    free_blocks: Vec<Block>,
    #[cfg(test)]
    unread_blocks: HashSet<usize>,
}

impl Blocks {
    fn new() -> Self {
        Self {
            // start with the null block
            blocks: vec![Block::null()],
            // write two empty blocks at the end
            free_blocks: vec![Block::null(), Block::null()],
            #[cfg(test)]
            unread_blocks: Default::default(),
        }
    }

    fn slice<'a>(&mut self, index: u32, file: &'a [u8]) -> Result<&'a [u8], Error> {
        let block = self
            .blocks
            .get(index as usize)
            .ok_or_else(|| Error::other("invalid block index"))?;
        #[cfg(test)]
        self.unread_blocks.remove(&(index as usize));
        let slice = block.slice(file);
        eprintln!(
            "read block index {} block {:?} slice {:?}",
            index, block, slice
        );
        if slice.len() > 4096 {
            eprintln!("BIG BLOCK");
        }
        Ok(slice)
    }

    fn block(&self, i: u32) -> &Block {
        &self.blocks[i as usize]
    }

    fn num_non_null_blocks(&self) -> usize {
        self.blocks.iter().filter(|b| !b.is_null()).count()
    }

    fn write_block<W: Write + Seek, F: FnOnce(&mut W) -> Result<(), Error>>(
        &mut self,
        writer: W,
        f: F,
    ) -> Result<u32, Error> {
        let index = self.next_block_index();
        let block = Block::from_write(writer, f)?;
        //eprintln!("write block index {} block {:?}", index, block);
        self.blocks.push(block);
        Ok(index)
    }

    fn next_block_index(&self) -> u32 {
        let index = self.blocks.len();
        index as u32
    }

    #[cfg(test)]
    fn print_unread_blocks(&self) {
        for i in self.unread_blocks.iter() {
            eprintln!("unread block {}: {:?}", i, self.blocks[*i]);
        }
    }
}

#[cfg(test)]
impl PartialEq for Blocks {
    fn eq(&self, other: &Self) -> bool {
        (&self.blocks, &self.free_blocks).eq(&(&other.blocks, &other.free_blocks))
    }
}

#[cfg(test)]
impl Eq for Blocks {}

impl BigEndianIo for Blocks {
    fn read<R: Read>(mut reader: R) -> Result<Self, Error> {
        let num_blocks = u32_read_v2(reader.by_ref())? as usize;
        let mut blocks = Vec::with_capacity(num_blocks);
        for _ in 0..num_blocks {
            let block = Block::read(reader.by_ref())?;
            blocks.push(block);
        }
        let num_free_blocks = u32_read_v2(reader.by_ref())? as usize;
        let mut free_blocks = Vec::with_capacity(num_free_blocks);
        for _ in 0..num_free_blocks {
            let block = Block::read(reader.by_ref())?;
            free_blocks.push(block);
        }
        #[cfg(test)]
        let unread_blocks = blocks
            .iter()
            .enumerate()
            .filter_map(|(i, block)| if block.is_null() { None } else { Some(i) })
            .collect::<HashSet<_>>();
        Ok(Self {
            blocks,
            free_blocks,
            #[cfg(test)]
            unread_blocks,
        })
    }

    fn write<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        let num_blocks = self.blocks.len() as u32;
        u32_write(writer.by_ref(), num_blocks)?;
        for block in self.blocks.iter() {
            block.write(writer.by_ref())?;
        }
        let num_free_blocks = self.free_blocks.len() as u32;
        u32_write(writer.by_ref(), num_free_blocks)?;
        for block in self.free_blocks.iter() {
            block.write(writer.by_ref())?;
        }
        Ok(())
    }
}

/// A block of data.
#[derive(Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
struct Block {
    /// Byte offset from the start of the file.
    offset: u32,
    /// Size in bytes.
    len: u32,
}

impl Block {
    fn slice<'a>(&self, file: &'a [u8]) -> &'a [u8] {
        let i = self.offset as usize;
        let j = i + self.len as usize;
        //eprintln!("read block {:?}", &file[i..j]);
        &file[i..j]
    }

    fn is_null(&self) -> bool {
        self.offset == 0 && self.len == 0
    }

    fn null() -> Self {
        Self { offset: 0, len: 0 }
    }

    fn from_write<W: Write + Seek, F: FnOnce(&mut W) -> Result<(), Error>>(
        mut writer: W,
        f: F,
    ) -> Result<Self, Error> {
        let offset = writer.stream_position()?;
        f(writer.by_ref())?;
        let len = writer.stream_position()? - offset;
        if offset > u32::MAX as u64 {
            return Err(Error::other("the file is too large"));
        }
        if len > u32::MAX as u64 {
            return Err(Error::other("the block is too large"));
        }
        Ok(Self {
            offset: offset as u32,
            len: len as u32,
        })
    }
}

impl BigEndianIo for Block {
    fn read<R: Read>(mut reader: R) -> Result<Self, Error> {
        let offset = u32_read_v2(reader.by_ref())?;
        let len = u32_read_v2(reader.by_ref())?;
        Ok(Self { offset, len })
    }

    fn write<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        u32_write(writer.by_ref(), self.offset)?;
        u32_write(writer.by_ref(), self.len)?;
        Ok(())
    }
}

#[derive(Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
struct BomInfo {
    num_paths: u32,
    entries: Vec<BomInfoEntry>,
}

impl BigEndianIo for BomInfo {
    fn read<R: Read>(mut reader: R) -> Result<Self, Error> {
        let version = u32_read_v2(reader.by_ref())?;
        if version != VERSION {
            return Err(Error::other(format!(
                "unsupported BOMInfo version: {}",
                version
            )));
        }
        let num_paths = u32_read_v2(reader.by_ref())?;
        let num_entries = u32_read_v2(reader.by_ref())?;
        //eprintln!("num paths {}", num_paths);
        //eprintln!("num entries {}", num_entries);
        let mut entries = Vec::new();
        for _ in 0..num_entries {
            entries.push(BomInfoEntry::read(reader.by_ref())?);
        }
        Ok(Self { num_paths, entries })
    }

    fn write<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        u32_write(writer.by_ref(), VERSION)?;
        u32_write(writer.by_ref(), self.num_paths)?;
        u32_write(writer.by_ref(), self.entries.len() as u32)?;
        for entry in self.entries.iter() {
            entry.write(writer.by_ref())?;
        }
        Ok(())
    }
}

#[derive(Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
struct BomInfoEntry {
    cpu_type: u32,
    reserved1: u32,
    file_size: u32,
    reserved2: u32,
}

impl BigEndianIo for BomInfoEntry {
    fn read<R: Read>(mut reader: R) -> Result<Self, Error> {
        let mut data = [0_u8; 16];
        reader.read_exact(&mut data[..])?;
        let cpu_type = u32_read(&data[0..4]);
        let reserved1 = u32_read(&data[4..8]);
        let file_size = u32_read(&data[8..12]);
        let reserved2 = u32_read(&data[12..16]);
        let entry = BomInfoEntry {
            cpu_type,
            reserved1,
            file_size,
            reserved2,
        };
        debug_assert!(reserved1 == 0 && reserved2 == 0, "entry = {entry:?}");
        Ok(entry)
    }

    fn write<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        u32_write(writer.by_ref(), self.cpu_type)?;
        u32_write(writer.by_ref(), self.reserved1)?;
        u32_write(writer.by_ref(), self.file_size)?;
        u32_write(writer.by_ref(), self.reserved2)?;
        Ok(())
    }
}

#[derive(Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
struct VIndex {
    index: u32,
}

impl VIndex {
    fn new(index: u32) -> Self {
        Self { index }
    }
}

impl BigEndianIo for VIndex {
    fn read<R: Read>(mut reader: R) -> Result<Self, Error> {
        let version = u32_read_v2(reader.by_ref())?;
        if version != VERSION {
            return Err(Error::other(format!(
                "unsupported VIndex version: {}",
                version
            )));
        }
        let index = u32_read_v2(reader.by_ref())?;
        let _x0 = u32_read_v2(reader.by_ref())?;
        let _x1 = u8_read(reader.by_ref())?;
        Ok(Self { index })
    }

    fn write<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        u32_write(writer.by_ref(), VERSION)?;
        u32_write(writer.by_ref(), self.index)?;
        u32_write(writer.by_ref(), 0_u32)?;
        u8_write(writer.by_ref(), 0_u8)?;
        Ok(())
    }
}

struct TreeV2<K: BigEndianIo, V: BigEndianIo> {
    root: TreeNode<K, V>,
}

impl<K: BigEndianIo, V: BigEndianIo> TreeV2<K, V> {
    fn write<W: Write + Seek>(&self, mut writer: W, blocks: &mut Blocks) -> Result<u32, Error> {
        let i = self.root.write(writer.by_ref(), blocks)?;
        let block_size = blocks.block(i).len;
        let i = blocks.write_block(writer.by_ref(), |writer| {
            writer.write_all(&TREE_MAGIC[..])?;
            u32_write(writer.by_ref(), VERSION)?;
            u32_write(writer.by_ref(), i)?;
            u32_write(writer.by_ref(), block_size)?;
            u32_write(writer.by_ref(), self.root.entries.len() as u32)?;
            u8_write(writer.by_ref(), 0_u8)?;
            Ok(())
        })?;
        Ok(i)
    }

    fn read(i: u32, file: &[u8], blocks: &mut Blocks) -> Result<Self, Error> {
        // tree
        let mut reader = blocks.slice(i, file)?;
        let mut magic = [0_u8; 4];
        reader.read_exact(&mut magic[..])?;
        if TREE_MAGIC[..] != magic[..] {
            return Err(Error::other("invalid tree magic"));
        }
        let version = u32_read_v2(reader.by_ref())?;
        if version != VERSION {
            return Err(Error::other(format!(
                "unsupported tree version: {}",
                version
            )));
        }
        let child = u32_read_v2(reader.by_ref())?;
        let block_size = u32_read_v2(reader.by_ref())?;
        debug_assert!(block_size as usize == reader.len());
        let num_paths = u32_read_v2(reader.by_ref())?;
        let _x = u8_read(reader.by_ref())?;
        let root = TreeNode::read(child, file, blocks)?;
        debug_assert!(num_paths as usize == root.entries.len());
        Ok(Self { root })
    }
}

struct TreeNode<K: BigEndianIo, V: BigEndianIo> {
    entries: Vec<(K, V)>,
    forward: u32,
    backward: u32,
    is_leaf: bool,
}

impl<K: BigEndianIo, V: BigEndianIo> TreeNode<K, V> {
    fn write<W: Write + Seek>(&self, mut writer: W, blocks: &mut Blocks) -> Result<u32, Error> {
        let mut entries_bytes = {
            let mut w = Vec::new();
            let count = self.entries.len();
            if count > u16::MAX as usize {
                return Err(Error::other("too many entries"));
            }
            let is_leaf = if self.is_leaf { 1 } else { 0 };
            u16_write(w.by_ref(), is_leaf)?;
            u16_write(w.by_ref(), count as u16)?;
            u32_write(w.by_ref(), self.forward)?;
            u32_write(w.by_ref(), self.backward)?;
            for (key, value) in self.entries.iter() {
                let i = blocks.write_block(writer.by_ref(), |writer| key.write(writer))?;
                let j = blocks.write_block(writer.by_ref(), |writer| value.write(writer))?;
                u32_write(w.by_ref(), i)?;
                u32_write(w.by_ref(), j)?;
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
        let is_leaf = u16_read(reader.by_ref())? != 0;
        let count = u16_read(reader.by_ref())?;
        let forward = u32_read_v2(reader.by_ref())?;
        let backward = u32_read_v2(reader.by_ref())?;
        let mut entries = Vec::new();
        for _ in 0..count {
            let i = u32_read_v2(reader.by_ref())?;
            let key = K::read(blocks.slice(i, file)?)?;
            let i = u32_read_v2(reader.by_ref())?;
            let value = V::read(blocks.slice(i, file)?)?;
            entries.push((key, value));
        }
        Ok(Self {
            entries,
            forward,
            backward,
            is_leaf,
        })
    }
}

#[derive(Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
struct Tree {
    child: u32,
    block_size: u32,
    num_paths: u32,
}

impl Tree {
    fn new_v_index(child: u32) -> Self {
        Self {
            child,
            block_size: 128,
            num_paths: 0,
        }
    }

    fn null(child: u32) -> Self {
        Self {
            child,
            block_size: 4096,
            num_paths: 0,
        }
    }

    fn new(child: u32, num_paths: u32) -> Self {
        Self {
            child,
            block_size: 4096,
            num_paths,
        }
    }
}

impl BigEndianIo for Tree {
    fn read<R: Read>(mut reader: R) -> Result<Self, Error> {
        let mut magic = [0_u8; 4];
        reader.read_exact(&mut magic[..])?;
        if TREE_MAGIC[..] != magic[..] {
            return Err(Error::other("invalid tree magic"));
        }
        let version = u32_read_v2(reader.by_ref())?;
        if version != VERSION {
            return Err(Error::other(format!(
                "unsupported tree version: {}",
                version
            )));
        }
        let child = u32_read_v2(reader.by_ref())?;
        let block_size = u32_read_v2(reader.by_ref())?;
        let num_paths = u32_read_v2(reader.by_ref())?;
        let _x = u8_read(reader.by_ref())?;
        Ok(Self {
            child,
            block_size,
            num_paths,
        })
    }

    fn write<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        writer.write_all(&TREE_MAGIC[..])?;
        u32_write(writer.by_ref(), VERSION)?;
        u32_write(writer.by_ref(), self.child)?;
        u32_write(writer.by_ref(), self.block_size)?;
        u32_write(writer.by_ref(), self.num_paths)?;
        u8_write(writer.by_ref(), 0_u8)?;
        Ok(())
    }
}

#[derive(Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
struct Paths {
    forward: u32,
    backward: u32,
    indices: Vec<(u32, u32)>,
    // TODO is root?
    is_leaf: bool,
}

impl Paths {
    fn null() -> Self {
        Self::from_indices(Default::default())
    }

    fn from_indices(indices: Vec<(u32, u32)>) -> Self {
        Self {
            forward: 0,
            backward: 0,
            indices,
            is_leaf: true,
        }
    }
}

impl BigEndianIo for Paths {
    fn read<R: Read>(mut reader: R) -> Result<Self, Error> {
        let is_leaf = u16_read(reader.by_ref())? != 0;
        let count = u16_read(reader.by_ref())?;
        let forward = u32_read_v2(reader.by_ref())?;
        let backward = u32_read_v2(reader.by_ref())?;
        let mut indices = Vec::new();
        for _ in 0..count {
            let index0 = u32_read_v2(reader.by_ref())?;
            let index1 = u32_read_v2(reader.by_ref())?;
            indices.push((index0, index1));
        }
        Ok(Self {
            forward,
            backward,
            indices,
            is_leaf,
        })
    }

    fn write<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        let is_leaf: u16 = if self.is_leaf { 1 } else { 0 };
        let count = self.indices.len();
        if count > u16::MAX as usize {
            return Err(Error::other("too many path indices"));
        }
        u16_write(writer.by_ref(), is_leaf)?;
        u16_write(writer.by_ref(), count as u16)?;
        u32_write(writer.by_ref(), self.forward)?;
        u32_write(writer.by_ref(), self.backward)?;
        for (index0, index1) in self.indices.iter() {
            u32_write(writer.by_ref(), *index0)?;
            u32_write(writer.by_ref(), *index1)?;
        }
        Ok(())
    }
}

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq, Eq))]
struct Nodes {
    nodes: HashMap<u32, Node>,
}

impl Nodes {
    fn path(&self, mut id: u32) -> Result<PathBuf, Error> {
        let mut visited = HashSet::new();
        let mut components = Vec::new();
        loop {
            if !visited.insert(id) {
                return Err(Error::other("loop"));
            }
            let Some(node) = self.nodes.get(&id) else {
                break;
            };
            components.push(node.name.as_os_str());
            id = node.parent;
        }
        let mut path = PathBuf::new();
        path.extend(components.into_iter().rev());
        Ok(path)
    }

    fn to_paths(&self) -> Result<HashMap<PathBuf, Metadata>, Error> {
        let mut paths = HashMap::new();
        for (id, node) in self.nodes.iter() {
            let path = self.path(*id)?;
            paths.insert(path, node.metadata.clone());
        }
        Ok(paths)
    }

    // parent -> children
    fn edges(&self) -> HashMap<u32, Vec<u32>> {
        let mut edges: HashMap<u32, Vec<u32>> = HashMap::new();
        for node in self.nodes.values() {
            edges.entry(node.parent).or_default().push(node.id);
        }
        edges
    }

    fn from_directory<P: AsRef<Path>>(directory: P) -> Result<Self, Error> {
        let directory = directory.as_ref();
        let mut nodes: HashMap<PathBuf, Node> = HashMap::new();
        let mut id: u32 = 1;
        for entry in WalkDir::new(directory).into_iter() {
            let entry = entry?;
            let entry_path = entry
                .path()
                .strip_prefix(directory)
                .map_err(Error::other)?
                .normalize();
            if entry_path == Path::new("") {
                continue;
            }
            eprintln!("entry {:?}", entry_path);
            let relative_path = Path::new(".").join(entry_path);
            let dirname = relative_path.parent();
            let basename = relative_path.file_name();
            let metadata = std::fs::metadata(entry.path())?;
            let mut metadata: Metadata = metadata.try_into()?;
            match metadata.extra {
                MetadataExtra::File {
                    ref mut checksum, ..
                }
                | MetadataExtra::Link {
                    ref mut checksum, ..
                } => {
                    let crc_reader = CrcReader::new(File::open(entry.path())?);
                    *checksum = crc_reader.digest()?;
                }
                _ => {}
            }
            let node = Node {
                id,
                parent: match dirname {
                    Some(d) => nodes.get(d).map(|node| node.id).unwrap_or(0),
                    None => 0,
                },
                name: match basename {
                    Some(s) => s.into(),
                    None => relative_path.clone().into(),
                },
                metadata,
            };
            nodes.insert(relative_path, node);
            id += 1;
        }
        let nodes = nodes.into_values().map(|node| (node.id, node)).collect();
        Ok(Self { nodes })
    }
}

#[derive(Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
struct Node {
    id: u32,
    parent: u32,
    metadata: Metadata,
    name: OsString,
}

/*
Device len 35
Directory len 31
File len 35
Link len 45


01, // is executable flag?
00, 00, 00, 02, // num arch
01, 00, 00, 07, // cpu_type_t
00, 00, 00, 03, // cpu_subtype_t
00, 00, df, 50, // offset
f1, 7d, 04, dd, // checksum
01, 00, 00, 0c, // cpu_type_t
80, 00, 00, 02, // cpu_subtype_t
00, 00, de, 80, // offset
5d, 06, d1, ec, // checksum
00, 00, 00, 00, 00, 00, 00, 00 // trailer



*/
#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct Metadata {
    mode: u16,
    uid: u32,
    gid: u32,
    mtime: u32,
    size: u64,
    extra: MetadataExtra,
}

impl Metadata {
    pub fn file_type(&self) -> FileType {
        use MetadataExtra::*;
        match self.extra {
            File { .. } | Executable { .. } => FileType::File,
            Directory { .. } => FileType::Directory,
            Link { .. } => FileType::Link,
            Device { .. } => FileType::Device,
        }
    }

    pub fn mode(&self) -> u16 {
        self.mode
    }

    pub fn uid(&self) -> u32 {
        self.uid
    }

    pub fn gid(&self) -> u32 {
        self.gid
    }

    pub fn mtime(&self) -> u32 {
        self.mtime
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    fn arch(&self) -> u16 {
        // arch 0xN00f
        // N - no. of architectures in a fat binary
        // f - always 0xf
        match self.extra {
            MetadataExtra::Executable { ref arches, .. } => {
                0x0f_u16 | (((arches.len() & 0xf) as u16) << 12)
            }
            _ => 0x0f_u16,
        }
    }
}

impl BigEndianIo for Metadata {
    fn read<R: Read>(mut reader: R) -> Result<Self, Error> {
        let kind: FileType = u8_read(reader.by_ref())?.try_into()?;
        let _x0 = u8_read(reader.by_ref())?;
        debug_assert!(_x0 == 1, "x0 {:?}", _x0);
        let arch = u16_read(reader.by_ref())?;
        if arch != 15 {
            eprintln!("arch {:#x}", arch);
        }
        let num_arch = (arch >> 12) & 0xf;
        let mode = u16_read(reader.by_ref())?;
        let mode = mode & MODE_MASK;
        let uid = u32_read_v2(reader.by_ref())?;
        let gid = u32_read_v2(reader.by_ref())?;
        let mtime = u32_read_v2(reader.by_ref())?;
        let size = u32_read_v2(reader.by_ref())?;
        let _x1 = u8_read(reader.by_ref())?;
        debug_assert!(_x1 == 1, "x1 {:?}", _x1);
        let extra = match kind {
            FileType::File if num_arch != 0 => {
                let checksum = u32_read_v2(reader.by_ref())?;
                let flag = u8_read(reader.by_ref())?;
                debug_assert!(flag == 1, "flag = {flag}");
                let num_arch_again = u32_read_v2(reader.by_ref())?;
                debug_assert!(
                    num_arch_again.min(2) == num_arch as u32,
                    "num_arch = {num_arch}, num_arch_again = {num_arch_again}",
                );
                let mut arches = Vec::with_capacity(num_arch_again as usize);
                for _ in 0..num_arch_again {
                    arches.push(ExeArch::read(reader.by_ref())?);
                }
                MetadataExtra::Executable { checksum, arches }
            }
            FileType::File => {
                debug_assert!(num_arch == 0, "num_arch = {num_arch}");
                let checksum = u32_read_v2(reader.by_ref())?;
                MetadataExtra::File { checksum }
            }
            FileType::Directory => {
                debug_assert!(num_arch == 0, "num_arch = {num_arch}");
                MetadataExtra::Directory
            }
            FileType::Link => {
                debug_assert!(num_arch == 0, "num_arch = {num_arch}");
                let checksum = u32_read_v2(reader.by_ref())?;
                let name_len = u32_read_v2(reader.by_ref())?;
                debug_assert!(
                    name_len == 0 && kind != FileType::Link || kind == FileType::Link,
                    "kind = {:?}, name_len = {}",
                    kind,
                    name_len
                );
                let mut name = vec![0_u8; name_len as usize];
                reader.read_exact(&mut name[..])?;
                let name = CString::from_vec_with_nul(name).map_err(Error::other)?;
                MetadataExtra::Link { checksum, name }
            }
            FileType::Device => {
                debug_assert!(num_arch == 0, "num_arch = {num_arch}");
                let dev = u32_read_v2(reader.by_ref())?;
                MetadataExtra::Device(Device { dev })
            }
        };
        let metadata = Self {
            mode,
            uid,
            gid,
            mtime,
            size: size as u64,
            extra,
        };
        // We ignore 8 zero bytes here.
        let trailer = u64_read(reader.by_ref())?;
        debug_assert!(trailer == 0, "trailer = {trailer}, metadata = {metadata:?}");
        Ok(metadata)
    }

    fn write<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        u8_write(writer.by_ref(), self.file_type() as u8)?;
        u8_write(writer.by_ref(), 1_u8)?;
        u16_write(writer.by_ref(), self.arch())?;
        u16_write(writer.by_ref(), self.mode & MODE_MASK)?;
        u32_write(writer.by_ref(), self.uid)?;
        u32_write(writer.by_ref(), self.gid)?;
        u32_write(writer.by_ref(), self.mtime)?;
        u32_write(writer.by_ref(), self.size as u32)?; // truncate the size
        u8_write(writer.by_ref(), 1_u8)?;
        match &self.extra {
            MetadataExtra::File { checksum } => {
                u32_write(writer.by_ref(), *checksum)?;
            }
            MetadataExtra::Executable { checksum, arches } => {
                u32_write(writer.by_ref(), *checksum)?;
                for arch in arches.iter() {
                    arch.write(writer.by_ref())?;
                }
            }
            MetadataExtra::Directory => {}
            MetadataExtra::Link { checksum, name } => {
                u32_write(writer.by_ref(), *checksum)?;
                let name_bytes = name.as_bytes_with_nul();
                u32_write(writer.by_ref(), name_bytes.len() as u32)?;
                writer.write_all(name_bytes)?;
            }
            MetadataExtra::Device(Device { dev }) => {
                u32_write(writer.by_ref(), *dev)?;
            }
        }
        // Block always ends with 8 zeroes.
        writer.write_all(&[0_u8; 8])?;
        Ok(())
    }
}

impl TryFrom<std::fs::Metadata> for Metadata {
    type Error = Error;
    fn try_from(other: std::fs::Metadata) -> Result<Self, Self::Error> {
        use std::os::unix::fs::MetadataExt;
        let kind: FileType = other.file_type().try_into()?;
        let extra = match kind {
            FileType::File => MetadataExtra::File { checksum: 0 },
            FileType::Directory => MetadataExtra::Directory,
            FileType::Link => MetadataExtra::Link {
                checksum: 0,
                name: Default::default(),
            },
            FileType::Device => MetadataExtra::Device(Device {
                dev: libc_dev_to_bom_dev(other.rdev()),
            }),
        };
        Ok(Self {
            mode: (other.mode() & 0o7777) as u16,
            uid: other.uid(),
            gid: other.gid(),
            mtime: other.mtime().try_into().unwrap_or(0),
            size: other.size(),
            extra,
        })
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub enum MetadataExtra {
    File { checksum: u32 },
    Executable { checksum: u32, arches: Vec<ExeArch> },
    Directory,
    Link { checksum: u32, name: CString },
    Device(Device),
}

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub struct Device {
    dev: u32,
}

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub struct ExeArch {
    cpu_type: u32,
    cpu_sub_type: u32,
    // TODO what if the size is u64?
    size: u32,
    checksum: u32,
}

impl BigEndianIo for ExeArch {
    fn read<R: Read>(mut reader: R) -> Result<Self, Error> {
        let cpu_type = u32_read_v2(reader.by_ref())?;
        let cpu_sub_type = u32_read_v2(reader.by_ref())?;
        let size = u32_read_v2(reader.by_ref())?;
        let checksum = u32_read_v2(reader.by_ref())?;
        Ok(Self {
            cpu_type,
            cpu_sub_type,
            size,
            checksum,
        })
    }

    fn write<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        u32_write(writer.by_ref(), self.cpu_type)?;
        u32_write(writer.by_ref(), self.cpu_sub_type)?;
        u32_write(writer.by_ref(), self.size)?;
        u32_write(writer.by_ref(), self.checksum)?;
        Ok(())
    }
}

impl Device {
    pub fn rdev(&self) -> u64 {
        bom_dev_to_libc_dev(self.dev)
    }
}

const fn bom_dev_to_libc_dev(dev: u32) -> libc::dev_t {
    let major = ((dev >> 24) & 0xff) as libc::c_uint;
    let minor = (dev & 0xff_ff_ff) as libc::c_uint;
    libc::makedev(major, minor)
}

fn libc_dev_to_bom_dev(dev: libc::dev_t) -> u32 {
    let major = unsafe { libc::major(dev) };
    let minor = unsafe { libc::minor(dev) };
    ((major << 24) & 0xff) as u32 | (minor & 0xff_ff_ff) as u32
}

fn u8_read<R: Read>(mut reader: R) -> Result<u8, Error> {
    let mut data = [0_u8; 1];
    reader.read_exact(&mut data[..])?;
    Ok(data[0])
}

fn u16_read<R: Read>(mut reader: R) -> Result<u16, Error> {
    let mut data = [0_u8; 2];
    reader.read_exact(&mut data[..])?;
    Ok(u16::from_be_bytes([data[0], data[1]]))
}

fn u32_read(data: &[u8]) -> u32 {
    u32::from_be_bytes([data[0], data[1], data[2], data[3]])
}

fn u32_read_v2<R: Read>(mut reader: R) -> Result<u32, Error> {
    let mut data = [0_u8; 4];
    reader.read_exact(&mut data[..])?;
    Ok(u32::from_be_bytes([data[0], data[1], data[2], data[3]]))
}

fn u64_read<R: Read>(mut reader: R) -> Result<u64, Error> {
    let mut data = [0_u8; 8];
    reader.read_exact(&mut data[..])?;
    Ok(u64::from_be_bytes([
        data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
    ]))
}

fn u8_write<W: Write>(mut writer: W, value: u8) -> Result<(), Error> {
    writer.write_all(&[value])
}

fn u16_write<W: Write>(mut writer: W, value: u16) -> Result<(), Error> {
    writer.write_all(value.to_be_bytes().as_slice())
}

fn u32_write<W: Write>(mut writer: W, value: u32) -> Result<(), Error> {
    writer.write_all(value.to_be_bytes().as_slice())
}

fn u64_write<W: Write>(mut writer: W, value: u64) -> Result<(), Error> {
    writer.write_all(value.to_be_bytes().as_slice())
}

pub trait BigEndianIo {
    fn read<R: Read>(reader: R) -> Result<Self, Error>
    where
        Self: Sized;
    fn write<W: Write>(&self, writer: W) -> Result<(), Error>;
}

//fn os_string_read<R: Read>(mut reader: R) -> Result<OsString, Error> {
//    reader.read_to_end()?;
//    let name =
//        CStr::from_bytes_with_nul(&block_bytes[4..]).map_err(Error::other)?;
//    let name = OsStr::from_bytes(name.to_bytes());
//}

const BOM_MAGIC: [u8; 8] = *b"BOMStore";
const TREE_MAGIC: [u8; 4] = *b"tree";
const HEADER_LEN: usize = 512;
const REAL_HEADER_LEN: usize = 32;
const HEADER_PADDING: usize = HEADER_LEN - REAL_HEADER_LEN;
const VERSION: u32 = 1;
const MODE_MASK: u16 = 0o7777;

// Named blocks.
const V_INDEX: &CStr = c"VIndex";
const HL_INDEX: &CStr = c"HLIndex";
const SIZE_64: &CStr = c"Size64";
const BOM_INFO: &CStr = c"BomInfo";
const PATHS: &CStr = c"Paths";

#[cfg(test)]
mod tests {
    use std::fmt::Debug;
    use std::fs::File;
    use std::io::Cursor;

    use arbitrary::Arbitrary;
    use arbitrary::Unstructured;
    use arbtest::arbtest;
    use cpio_test::DirectoryOfFiles;

    use super::*;

    #[test]
    fn bom_read() {
        //for filename in [
        //    //"block.bom",
        //    //"char.bom",
        //    "dir.bom",
        //    //"file.bom",
        //    //"hardlink.bom",
        //    //"symlink.bom",
        //    //"exe.bom",
        //    //"size64.bom",
        //] {
        //    Bom::read(File::open(filename).unwrap()).unwrap();
        //}
        //Bom::read(File::open("boms/com.apple.pkg.MAContent10_PremiumPreLoopsDeepHouse.bom").unwrap()).unwrap();
        Bom::read(File::open("boms/com.apple.pkg.CLTools_SDK_macOS12.bom").unwrap()).unwrap();
        //for entry in WalkDir::new("boms").into_iter() {
        //    let entry = entry.unwrap();
        //    if entry.file_type().is_dir() {
        //        continue;
        //    }
        //    eprintln!("reading {:?}", entry.path());
        //    Bom::read(File::open(entry.path()).unwrap()).unwrap();
        //}
    }

    #[test]
    fn write_read() {
        test_write_read::<Header>();
        test_write_read::<NamedBlocks>();
        test_write_read::<Blocks>();
        test_write_read::<Block>();
        test_write_read::<BomInfo>();
        test_write_read::<BomInfoEntry>();
        test_write_read::<VIndex>();
        test_write_read::<Tree>();
        test_write_read::<Paths>();
        test_write_read_convert::<Metadata32, Metadata>();
    }

    #[test]
    fn bom_write_read() {
        arbtest(|u| {
            let expected: Bom = u.arbitrary()?;
            let mut writer = Cursor::new(Vec::new());
            expected.write(&mut writer).unwrap();
            let bytes = writer.into_inner();
            let actual = Bom::read(&bytes[..]).unwrap();
            assert_eq!(expected, actual);
            Ok(())
        });
    }

    fn test_write_read<T: for<'a> Arbitrary<'a> + Debug + Eq + BigEndianIo>() {
        test_write_read_convert::<T, T>();
    }

    fn test_write_read_convert<X: for<'a> Arbitrary<'a>, T: From<X> + Debug + Eq + BigEndianIo>() {
        arbtest(|u| {
            let expected: X = u.arbitrary()?;
            let expected: T = expected.into();
            let mut bytes = Vec::new();
            expected.write(&mut bytes).unwrap();
            let actual = T::read(&bytes[..]).unwrap();
            assert_eq!(expected, actual);
            Ok(())
        });
    }

    impl<'a> Arbitrary<'a> for Metadata {
        fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
            Ok(Self {
                mode: u.arbitrary::<u16>()? & MODE_MASK,
                uid: u.arbitrary()?,
                gid: u.arbitrary()?,
                mtime: u.arbitrary()?,
                size: u.arbitrary()?,
                extra: u.arbitrary()?,
            })
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    struct Metadata32(Metadata);

    impl<'a> Arbitrary<'a> for Metadata32 {
        fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
            Ok(Self(Metadata {
                mode: u.arbitrary::<u16>()? & MODE_MASK,
                uid: u.arbitrary()?,
                gid: u.arbitrary()?,
                mtime: u.arbitrary()?,
                size: u.arbitrary::<u32>()? as u64,
                extra: u.arbitrary()?,
            }))
        }
    }

    impl From<Metadata32> for Metadata {
        fn from(other: Metadata32) -> Self {
            other.0
        }
    }

    impl<'a> Arbitrary<'a> for Nodes {
        fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
            use cpio_test::FileType::*;
            let directory = DirectoryOfFiles::new(
                &[
                    Regular,
                    Directory,
                    BlockDevice,
                    CharDevice,
                    Symlink,
                    HardLink,
                ],
                u,
            )?;
            let nodes = Nodes::from_directory(directory.path()).unwrap();
            Ok(nodes)
        }
    }
}
