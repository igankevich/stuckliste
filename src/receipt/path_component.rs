use std::collections::HashMap;
use std::collections::HashSet;
use std::ffi::CStr;
use std::ffi::CString;
use std::ffi::OsStr;
use std::fs::File;
use std::io::Error;
use std::io::Read;
use std::io::Seek;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::path::PathBuf;

use normalize_path::NormalizePath;
use walkdir::WalkDir;

use crate::receipt::Context;
use crate::receipt::CrcReader;
use crate::receipt::Metadata;
use crate::receipt::MetadataExtra;
use crate::receipt::VecTree;
use crate::BigEndianIo;
use crate::BlockIo;
use crate::Blocks;

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub struct PathComponentKey {
    id: u32,
    metadata: Metadata,
}

impl PathComponentKey {
    pub fn id(&self) -> u32 {
        self.id
    }
}

impl BlockIo<Context> for PathComponentKey {
    fn write_block<W: Write + Seek>(
        &self,
        mut writer: W,
        blocks: &mut Blocks,
        context: &mut Context,
    ) -> Result<u32, Error> {
        let metadata_index = self
            .metadata
            .write_block(writer.by_ref(), blocks, context)?;
        let i = blocks.append(writer.by_ref(), |writer| {
            self.id.write_be(writer.by_ref())?;
            metadata_index.write_be(writer.by_ref())?;
            Ok(())
        })?;
        Ok(i)
    }

    fn read_block(
        i: u32,
        file: &[u8],
        blocks: &mut Blocks,
        context: &mut Context,
    ) -> Result<Self, Error> {
        let mut reader = blocks.slice(i, file)?;
        let id = u32::read_be(reader.by_ref())?;
        let i = u32::read_be(reader.by_ref())?;
        let metadata = Metadata::read_block(i, file, blocks, context)?;
        Ok(Self { id, metadata })
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub struct PathComponentValue {
    parent: u32,
    name: CString,
}

impl BlockIo<Context> for PathComponentValue {
    fn write_block<W: Write + Seek>(
        &self,
        mut writer: W,
        blocks: &mut Blocks,
        _context: &mut Context,
    ) -> Result<u32, Error> {
        let i = blocks.append(writer.by_ref(), |writer| {
            self.parent.write_be(writer.by_ref())?;
            writer.write_all(self.name.to_bytes_with_nul())?;
            Ok(())
        })?;
        Ok(i)
    }

    fn read_block(
        i: u32,
        file: &[u8],
        blocks: &mut Blocks,
        _context: &mut Context,
    ) -> Result<Self, Error> {
        let mut reader = blocks.slice(i, file)?;
        eprintln!("block {}: {:?}", i, reader);
        let parent = u32::read_be(reader.by_ref())?;
        let name = CStr::from_bytes_with_nul(reader).map_err(Error::other)?;
        //eprintln!(
        //    "name {:?} id {} parent {} kind {:?} metadata {:?}",
        //    name,
        //    child.id,
        //    parent,
        //    child.metadata.file_type(),
        //    child.metadata,
        //);
        Ok(Self {
            parent,
            name: name.into(),
        })
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub struct PathComponent {
    // TODO make private
    pub id: u32,
    pub parent: u32,
    pub metadata: Metadata,
    pub name: CString,
}

impl PathComponent {
    pub fn new(key: PathComponentKey, value: PathComponentValue) -> Self {
        Self {
            id: key.id,
            metadata: key.metadata,
            parent: value.parent,
            name: value.name,
        }
    }

    pub fn into_key_and_value(self) -> (PathComponentKey, PathComponentValue) {
        let key = PathComponentKey {
            id: self.id,
            metadata: self.metadata.clone(),
        };
        let value = PathComponentValue {
            parent: self.parent,
            name: self.name.clone(),
        };
        (key, value)
    }
}

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct PathTree {
    nodes: HashMap<u32, PathComponent>,
}

impl PathTree {
    const BLOCK_LEN: usize = 4096;

    pub fn new(nodes: HashMap<u32, PathComponent>) -> Self {
        Self { nodes }
    }

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
            let name = OsStr::from_bytes(node.name.to_bytes());
            components.push(name);
            id = node.parent;
        }
        let mut path = PathBuf::new();
        path.extend(components.into_iter().rev());
        Ok(path)
    }

    pub fn to_paths(&self) -> Result<HashMap<PathBuf, Metadata>, Error> {
        let mut paths = HashMap::new();
        for (id, node) in self.nodes.iter() {
            let path = self.path(*id)?;
            paths.insert(path, node.metadata.clone());
        }
        Ok(paths)
    }

    pub fn nodes(&self) -> &HashMap<u32, PathComponent> {
        &self.nodes
    }

    // parent -> children
    pub fn edges(&self) -> HashMap<u32, Vec<u32>> {
        let mut edges: HashMap<u32, Vec<u32>> = HashMap::new();
        for node in self.nodes.values() {
            edges.entry(node.parent).or_default().push(node.id);
        }
        edges
    }

    pub fn from_directory<P: AsRef<Path>>(directory: P) -> Result<Self, Error> {
        let directory = directory.as_ref();
        let mut nodes: HashMap<PathBuf, PathComponent> = HashMap::new();
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
            let parent = match dirname {
                Some(d) => nodes.get(d).map(|node| node.id).unwrap_or(0),
                None => 0,
            };
            let name = match basename {
                Some(s) => s.as_bytes(),
                None => relative_path.as_os_str().as_bytes(),
            };
            let name = CString::new(name).map_err(Error::other)?;
            let node = PathComponent {
                id,
                parent,
                name,
                metadata,
            };
            nodes.insert(relative_path, node);
            id += 1;
        }
        let nodes = nodes.into_values().map(|node| (node.id, node)).collect();
        Ok(Self { nodes })
    }
}

impl BlockIo<Context> for PathTree {
    fn write_block<W: Write + Seek>(
        &self,
        writer: W,
        blocks: &mut Blocks,
        context: &mut Context,
    ) -> Result<u32, Error> {
        let paths = PathComponentTree::new(
            self.nodes()
                .values()
                .cloned()
                .map(|component| component.into_key_and_value())
                .collect(),
            Self::BLOCK_LEN,
        );
        paths.write_block(writer, blocks, context)
    }

    fn read_block(
        i: u32,
        file: &[u8],
        blocks: &mut Blocks,
        context: &mut Context,
    ) -> Result<Self, Error> {
        let tree = PathComponentTree::read_block(i, file, blocks, context)?;
        let graph = tree
            .into_inner()
            .into_iter()
            .map(|(k, v)| {
                let comp = PathComponent::new(k, v);
                (comp.id, comp)
            })
            .collect();
        Ok(PathTree::new(graph))
    }
}

type PathComponentTree = VecTree<PathComponentKey, PathComponentValue>;

#[cfg(test)]
mod tests {

    use arbitrary::Arbitrary;
    use arbitrary::Unstructured;
    use cpio_test::DirectoryOfFiles;

    use super::*;
    use crate::test::block_io_symmetry;

    #[test]
    fn write_read_symmetry() {
        block_io_symmetry::<PathComponentKey>();
        //block_io_symmetry::<PathComponentValue>();
        //block_io_symmetry::<PathTree>();
    }

    impl<'a> Arbitrary<'a> for PathTree {
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
            let nodes = PathTree::from_directory(directory.path()).unwrap();
            Ok(nodes)
        }
    }
}
