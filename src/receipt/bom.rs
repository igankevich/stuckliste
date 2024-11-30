use std::ffi::CStr;
use std::io::Error;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use crate::receipt::BomInfo;
use crate::receipt::Context;
use crate::receipt::FileSizes64;
use crate::receipt::HardLinks;
use crate::receipt::Metadata;
use crate::receipt::PathComponentVec;
use crate::receipt::VirtualPathTree;
use crate::BigEndianIo;
use crate::BlockIo;
use crate::Blocks;
use crate::Bom;
use crate::NamedBlocks;

#[derive(Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub struct Receipt {
    tree: PathComponentVec,
}

impl Receipt {
    pub fn paths(&self) -> Result<Vec<(PathBuf, Metadata)>, Error> {
        self.tree.to_paths()
    }

    pub fn from_directory<P: AsRef<Path>>(directory: P, paths_only: bool) -> Result<Self, Error> {
        let tree = PathComponentVec::from_directory(directory, paths_only)?;
        Ok(Self { tree })
    }

    pub fn write<W: Write + Seek>(&self, mut writer: W) -> Result<(), Error> {
        // skip the header
        writer.seek(SeekFrom::Start(Bom::LEN as u64))?;
        let mut blocks = Blocks::new();
        let mut named_blocks = NamedBlocks::new();
        let mut context = Context::new();
        {
            let vindex = VirtualPathTree::new();
            let i = vindex.write_block(writer.by_ref(), &mut blocks, &mut context)?;
            named_blocks.insert(V_INDEX.into(), i);
        }
        // hl index
        {
            let hard_links = std::mem::take(&mut context.hard_links);
            let i = hard_links.write_block(writer.by_ref(), &mut blocks, &mut context)?;
            named_blocks.insert(HL_INDEX.into(), i);
        }
        // paths
        {
            let i = self
                .tree
                .write_block(writer.by_ref(), &mut blocks, &mut context)?;
            named_blocks.insert(PATHS.into(), i);
        };
        // size 64
        {
            let file_size_64 = std::mem::take(&mut context.file_size_64);
            let i = file_size_64.write_block(writer.by_ref(), &mut blocks, &mut context)?;
            named_blocks.insert(SIZE_64.into(), i);
        }
        // bom info
        {
            let bom_info = BomInfo::new(&self.tree);
            let i = blocks.append(writer.by_ref(), |writer| bom_info.write_be(writer))?;
            named_blocks.insert(BOM_INFO.into(), i);
        }
        // write the header
        let header = Bom {
            blocks,
            named_blocks,
        };
        header.write(writer.by_ref())?;
        Ok(())
    }

    pub fn read<R: Read>(mut reader: R) -> Result<Self, Error> {
        let mut file = Vec::new();
        reader.read_to_end(&mut file)?;
        let header = Bom::read(&file[..])?;
        let mut blocks = header.blocks;
        let mut named_blocks = header.named_blocks;
        if let Some(index) = named_blocks.remove(BOM_INFO) {
            let _bom_info = BomInfo::read_be(blocks.slice(index, &file)?)?;
        }
        let mut context = Context::new();
        if let Some(index) = named_blocks.remove(V_INDEX) {
            let _vindex = VirtualPathTree::read_block(index, &file, &mut blocks, &mut context)?;
        }
        // block id -> file size
        if let Some(index) = named_blocks.remove(SIZE_64) {
            let file_size_64 = FileSizes64::read_block(index, &file, &mut blocks, &mut context)?;
            context.file_size_64 = file_size_64;
        }
        if let Some(index) = named_blocks.remove(HL_INDEX) {
            let hard_links = HardLinks::read_block(index, &file, &mut blocks, &mut context)?;
            context.hard_links = hard_links;
        }
        // id -> data
        let i = named_blocks
            .remove(PATHS)
            .ok_or_else(|| Error::other(format!("`{:?}` block not found", PATHS)))?;
        let tree = PathComponentVec::read_block(i, &file, &mut blocks, &mut context)?;
        debug_assert!(named_blocks.is_empty(), "named blocks {:?}", named_blocks);
        Ok(Self { tree })
    }
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

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use arbtest::arbtest;

    use super::*;

    #[test]
    fn write_read() {
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

    #[test]
    fn bom_read() {
        let bom = Receipt::read(std::fs::File::open("our-good.bom").unwrap()).unwrap();
        eprintln!("good bom {:#?}", bom);
        let bom = Receipt::read(std::fs::File::open("our-bad.bom").unwrap()).unwrap();
        eprintln!("bad bom {:#?}", bom);
    }
}
