use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::ffi::CStr;
use std::ffi::CString;
use std::io::Error;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::path::PathBuf;

use crate::io::*;
use crate::receipt::BomInfo;
use crate::receipt::Context;
use crate::receipt::Metadata;
use crate::receipt::PathComponent;
use crate::receipt::PathComponentKey;
use crate::receipt::PathComponentValue;
use crate::receipt::PathTree;
use crate::receipt::Ptr;
use crate::BigEndianIo;
use crate::BlockIo;
use crate::Blocks;
use crate::Bom;
use crate::NamedBlocks;
use crate::TreeNode;
use crate::TreeV2;
use crate::TREE_MAGIC;

#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq, Debug))]
pub struct Receipt {
    tree: PathTree,
}

impl Receipt {
    pub fn paths(&self) -> Result<HashMap<PathBuf, Metadata>, Error> {
        self.tree.to_paths()
    }

    pub fn from_directory<P: AsRef<Path>>(directory: P) -> Result<Self, Error> {
        let tree = PathTree::from_directory(directory)?;
        Ok(Self { tree })
    }

    pub fn write<W: Write + Seek>(&self, mut writer: W) -> Result<(), Error> {
        // skip the header
        writer.seek(SeekFrom::Start(Bom::LEN as u64))?;
        let mut blocks = Blocks::new();
        let mut named_blocks = NamedBlocks::new();
        // v index
        {
            let paths = Paths::null();
            let i = blocks.append(writer.by_ref(), |writer| paths.write(writer))?;
            let tree = Tree::new_v_index(i);
            let i = blocks.append(writer.by_ref(), |writer| tree.write(writer))?;
            let vindex = VIndex::new(i);
            let i = blocks.append(writer.by_ref(), |writer| vindex.write(writer))?;
            named_blocks.insert(V_INDEX.into(), i);
        }
        // hl index
        {
            let paths = Paths::null();
            let i = blocks.append(writer.by_ref(), |writer| paths.write(writer))?;
            let tree = Tree::null(i);
            let i = blocks.append(writer.by_ref(), |writer| tree.write(writer))?;
            named_blocks.insert(HL_INDEX.into(), i);
        }
        let mut file_size_64 = HashMap::new();
        // paths
        {
            let edges = self.tree.edges();
            eprintln!("write edges {:?}", edges);
            let mut roots = Vec::new();
            let mut all_paths = Vec::new();
            for (parent, children) in edges.iter() {
                let mut indices = Vec::new();
                for child in children.iter() {
                    let node = self.tree.nodes().get(child).unwrap();
                    // node metadata
                    let i = blocks.append(writer.by_ref(), |writer| node.metadata.write(writer))?;
                    if node.metadata.size() > u32::MAX as u64 {
                        file_size_64.insert(i, node.metadata.size());
                    }
                    // node id -> index mapping
                    let index0 = blocks.append(writer.by_ref(), |writer| {
                        write_be(writer.by_ref(), node.id)?;
                        write_be(writer.by_ref(), i)?;
                        Ok(())
                    })?;
                    // parent + name
                    let index1 = blocks.append(writer.by_ref(), |writer| {
                        write_be(writer.by_ref(), node.parent)?;
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
                let i = blocks.append(writer.by_ref(), |writer| paths.write(writer))?;
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
                let i = blocks.append(writer.by_ref(), |writer| paths.write(writer))?;
                let tree = Tree::new(i, num_paths);
                let i = blocks.append(writer.by_ref(), |writer| tree.write(writer))?;
                named_blocks.insert(PATHS.into(), i);
            }
        };
        // size 64
        {
            let mut indices = Vec::new();
            eprintln!("write file_size_64 {:#?}", file_size_64);
            for (metadata_index, file_size) in file_size_64.into_iter() {
                let i = blocks.append(writer.by_ref(), |writer| file_size.write(writer))?;
                let j =
                    blocks.append(writer.by_ref(), |writer| write_be(writer, metadata_index))?;
                indices.push((i, j));
            }
            let num_paths = indices.len() as u32;
            let paths = Paths::from_indices(indices);
            let i = blocks.append(writer.by_ref(), |writer| paths.write(writer))?;
            let tree = Tree::new(i, num_paths);
            let i = blocks.append(writer.by_ref(), |writer| tree.write(writer))?;
            named_blocks.insert(SIZE_64.into(), i);
        }
        // bom info
        {
            let bom_info = BomInfo::new(&self.tree);
            let i = blocks.append(writer.by_ref(), |writer| bom_info.write(writer))?;
            named_blocks.insert(BOM_INFO.into(), i);
        }
        // write the header
        writer.seek(SeekFrom::Start(0))?;
        let header = Bom {
            blocks,
            named_blocks,
        };
        header.write(writer.by_ref())?;
        let paths = self.tree.to_paths()?;
        for (path, metadata) in paths.iter() {
            eprintln!("write path {:?} metadata {:?}", path, metadata);
        }
        Ok(())
    }

    pub fn read<R: Read>(mut reader: R) -> Result<Self, Error> {
        let mut file = Vec::new();
        reader.read_to_end(&mut file)?;
        let header = Bom::read(&file[..])?;
        let mut blocks = header.blocks;
        let mut named_blocks = header.named_blocks;
        eprintln!("{:#?}", named_blocks);
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
                let index = u32::read(&block_bytes[..])?;
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
        let mut context = Context::new();
        // block id -> file size
        if let Some(index) = named_blocks.remove(SIZE_64) {
            let tree = FileSizeTree::read_block(index, &file, &mut blocks, &mut context)?;
            let mut file_size_64 = HashMap::new();
            for (file_size, metadata_index) in tree.into_inner().into_entries() {
                file_size_64.insert(metadata_index, file_size);
            }
            eprintln!("file_size64 = {:#?}", file_size_64);
            context.file_size_64 = file_size_64;
        }
        if let Some(index) = named_blocks.remove(HL_INDEX) {
            let tree = HardLinkTree::read_block(index, &file, &mut blocks, &mut context)?;
            let mut hard_links = HashMap::new();
            for (hard_links_tree, metadata_index) in tree.into_inner().into_entries() {
                for (_, name) in hard_links_tree.into_inner().into_inner().into_entries() {
                    hard_links.insert(metadata_index, name);
                }
            }
            eprintln!("hard links {:#?}", hard_links);
            context.hard_links = hard_links;
        }
        // id -> data
        let mut graph = HashMap::new();
        if let Some(index) = named_blocks.remove(PATHS) {
            let tree = TreeV2::<PathComponentKey, PathComponentValue, Context>::read_block(
                index,
                &file,
                &mut blocks,
                &mut context,
            )?;
            let mut paths = VecDeque::new();
            paths.push_back((PATHS, tree.into_inner(), index));
            let mut visited = HashSet::new();
            while let Some((name, tree_node, index)) = paths.pop_front() {
                if !visited.insert(index) {
                    //eprintln!("loop {}", index);
                    continue;
                }
                match tree_node {
                    TreeNode::Root { entries, .. } => {
                        for (index, _last_entry) in entries.into_iter() {
                            let tree_node =
                                TreeNode::read_block(index, &file, &mut blocks, &mut context)?;
                            paths.push_back((c"paths.root", tree_node, index));
                        }
                    }
                    TreeNode::Node {
                        entries,
                        forward,
                        backward,
                    } => {
                        for (path_key, path_value) in entries.into_iter() {
                            let comp = PathComponent::new(path_key, path_value);
                            graph.insert(comp.id, comp);
                        }
                        if forward != 0 {
                            let i = forward;
                            // TODO TreeNode::read only once
                            let tree_node =
                                TreeNode::read_block(i, &file, &mut blocks, &mut context)?;
                            paths.push_back((name, tree_node, i));
                        }
                        if backward != 0 {
                            let i = backward;
                            let tree_node =
                                TreeNode::read_block(i, &file, &mut blocks, &mut context)?;
                            paths.push_back((name, tree_node, i));
                        }
                    }
                }
                //if name != c"paths.root" {
                //    debug_assert!(path.forward == 0);
                //    debug_assert!(path.backward == 0);
                //}
            }
        }
        debug_assert!(named_blocks.is_empty());
        let tree = PathTree::new(graph);
        eprintln!("tree {:#?}", tree);
        let paths = tree.to_paths()?;
        for (path, metadata) in paths.iter() {
            eprintln!("read path {:?} metadata {:?}", path, metadata);
        }
        Ok(Self { tree })
    }
}

#[derive(Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
struct VIndex {
    index: u32,
}

impl VIndex {
    const VERSION: u32 = 1;

    fn new(index: u32) -> Self {
        Self { index }
    }
}

impl BigEndianIo for VIndex {
    fn read<R: Read>(mut reader: R) -> Result<Self, Error> {
        let version = u32::read(reader.by_ref())?;
        if version != Self::VERSION {
            return Err(Error::other(format!(
                "unsupported VIndex version: {}",
                version
            )));
        }
        let index = u32::read(reader.by_ref())?;
        let _x0 = u32::read(reader.by_ref())?;
        let _x1 = u8::read(reader.by_ref())?;
        Ok(Self { index })
    }

    fn write<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        write_be(writer.by_ref(), Self::VERSION)?;
        write_be(writer.by_ref(), self.index)?;
        write_be(writer.by_ref(), 0_u32)?;
        0_u8.write(writer.by_ref())?;
        Ok(())
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
    const VERSION: u32 = 1;

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
        let version = u32::read(reader.by_ref())?;
        if version != Self::VERSION {
            return Err(Error::other(format!(
                "unsupported tree version: {}",
                version
            )));
        }
        let child = u32::read(reader.by_ref())?;
        let block_size = u32::read(reader.by_ref())?;
        let num_paths = u32::read(reader.by_ref())?;
        let _x = u8::read(reader.by_ref())?;
        Ok(Self {
            child,
            block_size,
            num_paths,
        })
    }

    fn write<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        writer.write_all(&TREE_MAGIC[..])?;
        write_be(writer.by_ref(), Self::VERSION)?;
        write_be(writer.by_ref(), self.child)?;
        write_be(writer.by_ref(), self.block_size)?;
        write_be(writer.by_ref(), self.num_paths)?;
        0_u8.write(writer.by_ref())?;
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
        let is_leaf = u16::read(reader.by_ref())? != 0;
        let count = u16::read(reader.by_ref())?;
        let forward = u32::read(reader.by_ref())?;
        let backward = u32::read(reader.by_ref())?;
        let mut indices = Vec::new();
        for _ in 0..count {
            let index0 = u32::read(reader.by_ref())?;
            let index1 = u32::read(reader.by_ref())?;
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
        is_leaf.write(writer.by_ref())?;
        (count as u16).write(writer.by_ref())?;
        write_be(writer.by_ref(), self.forward)?;
        write_be(writer.by_ref(), self.backward)?;
        for (index0, index1) in self.indices.iter() {
            write_be(writer.by_ref(), *index0)?;
            write_be(writer.by_ref(), *index1)?;
        }
        Ok(())
    }
}

fn u32_read(data: &[u8]) -> u32 {
    u32::from_be_bytes([data[0], data[1], data[2], data[3]])
}

/// Virtual paths (i.e. paths defined with regular expressions).
pub const V_INDEX: &CStr = c"VIndex";

/// Hard links.
pub const HL_INDEX: &CStr = c"HLIndex";

/// 64-bit file sizes.
pub const SIZE_64: &CStr = c"Size64";

/// Per-architecture file statistics.
pub const BOM_INFO: &CStr = c"BomInfo";

/// File path components tree.
pub const PATHS: &CStr = c"Paths";

/// File size to metadata block index mapping.
pub type FileSizeTree = TreeV2<u64, u32, Context>;

/// Hard links to metadata block index mapping.
pub type HardLinkTree = TreeV2<Ptr<TreeV2<(), CString, Context>>, u32, Context>;

/// File path components tree.
pub type PathComponentTree = TreeV2<PathComponentKey, PathComponentValue, Context>;

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::io::Cursor;

    use arbtest::arbtest;

    use super::*;
    use crate::receipt::BomInfoEntry;
    use crate::test::test_write_read;
    use crate::Block;

    #[test]
    fn bom_read() {
        for filename in [
            //"block.bom",
            //"char.bom",
            //"dir.bom",
            //"file.bom",
            "hardlink.bom",
            //"symlink.bom",
            //"exe.bom",
            //"size64.bom",
        ] {
            Receipt::read(File::open(filename).unwrap()).unwrap();
        }
        //Receipt::read(File::open("boms/com.apple.pkg.MAContent10_PremiumPreLoopsDeepHouse.bom").unwrap()).unwrap();
        //Receipt::read(File::open("boms/com.apple.pkg.CLTools_SDK_macOS12.bom").unwrap()).unwrap();
        //Receipt::read(File::open("cars/0E9C2921-1D9F-4EE8-8E47-A8AB1737DF6E.car").unwrap()).unwrap();
        //for entry in WalkDir::new("boms").into_iter() {
        //    let entry = entry.unwrap();
        //    if entry.file_type().is_dir() {
        //        continue;
        //    }
        //    eprintln!("reading {:?}", entry.path());
        //    Receipt::read(File::open(entry.path()).unwrap()).unwrap();
        //}
    }

    #[test]
    fn write_read() {
        //test_write_read::<Bom>();
        test_write_read::<NamedBlocks>();
        test_write_read::<Blocks>();
        test_write_read::<Block>();
        test_write_read::<BomInfo>();
        test_write_read::<BomInfoEntry>();
        test_write_read::<VIndex>();
        test_write_read::<Tree>();
        test_write_read::<Paths>();
    }

    #[test]
    fn bom_write_read() {
        arbtest(|u| {
            let expected: Receipt = u.arbitrary()?;
            let mut writer = Cursor::new(Vec::new());
            expected.write(&mut writer).unwrap();
            let bytes = writer.into_inner();
            let actual = Receipt::read(&bytes[..]).unwrap();
            assert_eq!(expected, actual);
            Ok(())
        });
    }
}
