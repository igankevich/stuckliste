use std::collections::HashMap;
use std::collections::HashSet;
use std::ffi::CStr;
use std::ffi::CString;
use std::ffi::OsStr;
use std::fs::read_link;
use std::fs::File;
use std::io::Error;
use std::io::Read;
use std::io::Seek;
use std::io::Write;
use std::ops::Deref;
use std::ops::DerefMut;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::path::PathBuf;

use normalize_path::NormalizePath;
use walkdir::WalkDir;

use crate::receipt::BomInfo;
use crate::receipt::Context;
use crate::receipt::CrcReader;
use crate::receipt::Link;
use crate::receipt::Metadata;
use crate::receipt::MetadataExtra;
use crate::receipt::VecTree;
use crate::BigEndianIo;
use crate::BlockIo;
use crate::Blocks;

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub struct PathComponentKey {
    /// Sequential number of the path in the tree.
    seq_no: u32,
    metadata: Metadata,
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
            self.seq_no.write_be(writer.by_ref())?;
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
        let seq_no = u32::read_be(reader.by_ref())?;
        let i = u32::read_be(reader.by_ref())?;
        let metadata = Metadata::read_block(i, file, blocks, context)?;
        Ok(Self { seq_no, metadata })
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
        let parent = u32::read_be(reader.by_ref())?;
        let name = CStr::from_bytes_with_nul(reader).map_err(Error::other)?;
        Ok(Self {
            parent,
            name: name.into(),
        })
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub struct PathComponent {
    seq_no: u32,
    parent: u32,
    metadata: Metadata,
    name: CString,
}

impl PathComponent {
    pub fn new(key: PathComponentKey, value: PathComponentValue) -> Self {
        Self {
            seq_no: key.seq_no,
            metadata: key.metadata,
            parent: value.parent,
            name: value.name,
        }
    }

    pub fn into_key_and_value(self) -> (PathComponentKey, PathComponentValue) {
        let key = PathComponentKey {
            seq_no: self.seq_no,
            metadata: self.metadata.clone(),
        };
        let value = PathComponentValue {
            parent: self.parent,
            name: self.name.clone(),
        };
        (key, value)
    }

    pub(crate) fn accumulate(&self, stats: &mut BomInfo) {
        self.metadata.accumulate(stats);
    }
}

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct PathComponentVec {
    components: Vec<PathComponent>,
}

impl PathComponentVec {
    const BLOCK_LEN: usize = 4096;

    pub fn new(components: Vec<PathComponent>) -> Self {
        Self { components }
    }

    fn path(&self, mut seq_no: u32) -> Result<PathBuf, Error> {
        let mut visited = HashSet::new();
        let mut components = Vec::new();
        loop {
            if !visited.insert(seq_no) {
                return Err(Error::other("loop"));
            }
            if seq_no == 0 {
                break;
            }
            let i = seq_no - 1;
            let Some(node) = self.components.get(i as usize) else {
                break;
            };
            let name = OsStr::from_bytes(node.name.to_bytes());
            components.push(name);
            seq_no = node.parent;
        }
        let mut path = PathBuf::new();
        path.extend(components.into_iter().rev());
        Ok(path)
    }

    pub fn to_paths(&self) -> Result<Vec<(PathBuf, Metadata)>, Error> {
        let mut paths = Vec::new();
        for component in self.components.iter() {
            let path = self.path(component.seq_no)?;
            paths.push((path, component.metadata.clone()));
        }
        Ok(paths)
    }

    pub fn from_directory<P: AsRef<Path>>(directory: P, paths_only: bool) -> Result<Self, Error> {
        let directory = directory.as_ref();
        let mut components: HashMap<PathBuf, PathComponent> = HashMap::new();
        // Id starts with 1.
        let mut seq_no: u32 = 1;
        for entry in WalkDir::new(directory).into_iter() {
            let entry = entry?;
            let entry_path = entry
                .path()
                .strip_prefix(directory)
                .map_err(Error::other)?
                .normalize();
            //if entry_path == Path::new("") {
            //    continue;
            //}
            let relative_path = if entry_path == Path::new("") {
                Path::new(".").to_path_buf()
            } else {
                Path::new(".").join(entry_path)
            };
            let dirname = relative_path.parent();
            let basename = relative_path.file_name();
            let metadata = std::fs::symlink_metadata(entry.path())?;
            let mut metadata: Metadata = metadata.try_into()?;
            if paths_only {
                metadata.mode = 0;
                metadata.uid = 0;
                metadata.gid = 0;
                metadata.mtime = 0;
                metadata.size = 0;
                metadata.extra = MetadataExtra::PathOnly {
                    entry_type: metadata.file_type().to_entry_type(),
                };
            } else {
                match metadata.extra {
                    MetadataExtra::File {
                        ref mut checksum, ..
                    } => {
                        let crc_reader = CrcReader::new(File::open(entry.path())?);
                        *checksum = crc_reader.digest()?;
                    }
                    MetadataExtra::Link(Link {
                        ref mut name,
                        ref mut checksum,
                    }) => {
                        *name = read_link(entry.path())?;
                        let crc_reader = CrcReader::new(name.as_os_str().as_bytes());
                        *checksum = crc_reader.digest()?;
                    }
                    _ => {}
                }
            }
            let parent = match dirname {
                Some(d) => components.get(d).map(|node| node.seq_no).unwrap_or(0),
                None => 0,
            };
            let name = match basename {
                Some(s) => s.as_bytes(),
                None => relative_path.as_os_str().as_bytes(),
            };
            let name = CString::new(name).map_err(Error::other)?;
            let node = PathComponent {
                seq_no,
                parent,
                name,
                metadata,
            };
            components.insert(relative_path, node);
            seq_no += 1;
        }
        let mut components: Vec<_> = components.into_values().collect();
        components.sort_unstable_by(|a, b| a.seq_no.cmp(&b.seq_no));
        Ok(Self { components })
    }
}

impl BlockIo<Context> for PathComponentVec {
    fn write_block<W: Write + Seek>(
        &self,
        writer: W,
        blocks: &mut Blocks,
        context: &mut Context,
    ) -> Result<u32, Error> {
        eprintln!("write tree {:#?}", self.components);
        let paths = PathComponentTree::new(
            self.iter()
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
        let components: Vec<_> = tree
            .into_inner()
            .into_iter()
            .map(|(k, v)| PathComponent::new(k, v))
            .collect();
        eprintln!("read tree {:#?}", components);
        Ok(PathComponentVec::new(components))
    }
}

impl Deref for PathComponentVec {
    type Target = Vec<PathComponent>;

    fn deref(&self) -> &Self::Target {
        &self.components
    }
}

impl DerefMut for PathComponentVec {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.components
    }
}

type PathComponentTree = VecTree<PathComponentKey, PathComponentValue>;

#[cfg(test)]
mod tests {

    use arbitrary::Arbitrary;
    use arbitrary::Unstructured;
    use random_dir::DirBuilder;

    use super::*;
    use crate::test::block_io_symmetry;

    #[test]
    fn write_read_symmetry() {
        block_io_symmetry::<PathComponentKey>();
        block_io_symmetry::<PathComponentValue>();
        block_io_symmetry::<PathComponentVec>();
    }

    impl<'a> Arbitrary<'a> for PathComponentVec {
        fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
            use random_dir::FileType::*;
            let directory = DirBuilder::new()
                .file_types([
                    Regular,
                    Directory,
                    #[cfg(not(target_os = "macos"))]
                    BlockDevice,
                    #[cfg(not(target_os = "macos"))]
                    CharDevice,
                    Symlink,
                    HardLink,
                ])
                .create(u)?;
            let paths_only = u.arbitrary()?;
            let nodes = PathComponentVec::from_directory(directory.path(), paths_only).unwrap();
            Ok(nodes)
        }
    }
}
