use std::collections::HashMap;
use std::collections::HashSet;
use std::ffi::CStr;
use std::ffi::OsStr;
use std::ffi::OsString;
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
use crate::BigEndianIo;
use crate::BlockIo;
use crate::Blocks;

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
        Ok(0)
    }

    fn read_block(
        i: u32,
        file: &[u8],
        blocks: &mut Blocks,
        context: &mut Context,
    ) -> Result<Self, Error> {
        let mut reader = blocks.slice(i, file)?;
        let id = u32::read(reader.by_ref())?;
        eprintln!("id {}", id);
        let i = u32::read(reader.by_ref())?;
        let reader = blocks.slice(i, &file)?;
        let block_len = reader.len();
        let mut reader = std::io::Cursor::new(reader);
        let mut metadata = Metadata::read(reader.by_ref())?;
        // TODO move to Metadata?? need to implement BlockIo for Metadata
        if let Some(size) = context.file_size_64.get(&i) {
            metadata.size = *size;
        }
        let unread_bytes = block_len - reader.position() as usize;
        debug_assert!(unread_bytes == 0, "unread_bytes = {unread_bytes}");
        Ok(Self { id, metadata })
    }
}

pub struct PathComponentValue {
    parent: u32,
    name: OsString,
}

impl<C> BlockIo<C> for PathComponentValue {
    fn write_block<W: Write + Seek>(
        &self,
        mut writer: W,
        blocks: &mut Blocks,
        context: &mut C,
    ) -> Result<u32, Error> {
        Ok(0)
    }

    fn read_block(
        i: u32,
        file: &[u8],
        blocks: &mut Blocks,
        context: &mut C,
    ) -> Result<Self, Error> {
        let mut reader = blocks.slice(i, &file)?;
        eprintln!("block {}: {:?}", i, reader);
        let parent = u32::read(reader.by_ref())?;
        let name = CStr::from_bytes_with_nul(reader).map_err(Error::other)?;
        let name = OsStr::from_bytes(name.to_bytes());
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

#[derive(Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub struct PathComponent {
    // TODO make private
    pub id: u32,
    pub parent: u32,
    pub metadata: Metadata,
    pub name: OsString,
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
}

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct PathTree {
    nodes: HashMap<u32, PathComponent>,
}

impl PathTree {
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
            components.push(node.name.as_os_str());
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
            let node = PathComponent {
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

#[cfg(test)]
mod tests {
    use arbitrary::Arbitrary;
    use arbitrary::Unstructured;
    use cpio_test::DirectoryOfFiles;

    use super::*;

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
