use std::collections::HashMap;
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
use crate::receipt::PathTree;
use crate::receipt::VirtualPathTree;
use crate::BigEndianIo;
use crate::BlockIo;
use crate::Blocks;
use crate::Bom;
use crate::NamedBlocks;

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
        let mut context = Context::new();
        {
            let vindex = VirtualPathTree::new();
            let i = vindex.write_block(writer.by_ref(), &mut blocks, &mut context)?;
            named_blocks.insert(V_INDEX.into(), i);
        }
        // hl index
        {
            eprintln!("write hard links {:#?}", context.hard_links);
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
            eprintln!("write file_size_64 {:#?}", context.file_size_64);
            let file_size_64 = std::mem::take(&mut context.file_size_64);
            let i = file_size_64.write_block(writer.by_ref(), &mut blocks, &mut context)?;
            named_blocks.insert(SIZE_64.into(), i);
        }
        // bom info
        {
            let bom_info = BomInfo::new(&self.tree);
            let i = blocks.append(writer.by_ref(), |writer| bom_info.write(writer))?;
            named_blocks.insert(BOM_INFO.into(), i);
        }
        // write the header
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
        if let Some(index) = named_blocks.remove(BOM_INFO) {
            let bom_info = BomInfo::read(blocks.slice(index, &file)?)?;
            eprintln!("{:?}", bom_info);
        }
        let mut context = Context::new();
        if let Some(index) = named_blocks.remove(V_INDEX) {
            let vindex = VirtualPathTree::read_block(index, &file, &mut blocks, &mut context)?;
            eprintln!("vindex {:#?}", vindex);
        }
        // block id -> file size
        if let Some(index) = named_blocks.remove(SIZE_64) {
            let file_size_64 = FileSizes64::read_block(index, &file, &mut blocks, &mut context)?;
            context.file_size_64 = file_size_64;
        }
        if let Some(index) = named_blocks.remove(HL_INDEX) {
            let hard_links = HardLinks::read_block(index, &file, &mut blocks, &mut context)?;
            eprintln!("hard links {:#?}", hard_links);
            context.hard_links = hard_links;
        }
        // id -> data
        let i = named_blocks
            .remove(PATHS)
            .ok_or_else(|| Error::other(format!("`{:?}` block not found", PATHS)))?;
        let tree = PathTree::read_block(i, &file, &mut blocks, &mut context)?;
        debug_assert!(named_blocks.is_empty(), "named blocks {:?}", named_blocks);
        eprintln!("paths {:#?}", tree);
        let paths = tree.to_paths()?;
        for (path, metadata) in paths.iter() {
            eprintln!("read path {:?} metadata {:?}", path, metadata);
        }
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
    use std::fs::File;
    use std::io::Cursor;

    use arbtest::arbtest;

    use super::*;

    #[test]
    fn bom_read() {
        {
            let filename = "exe-path-only.bom";
            Receipt::read(File::open(filename).unwrap()).unwrap();
        }
        //Receipt::read(
        //    File::open("boms/com.apple.pkg.MAContent10_PremiumPreLoopsDeepHouse.bom").unwrap(),
        //)
        //.unwrap();
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
        arbtest(|u| {
            let expected: Receipt = u.arbitrary()?;
            let mut writer = Cursor::new(Vec::new());
            expected.write(&mut writer).unwrap();
            let bytes = writer.into_inner();
            eprintln!("magic {:x?}", &bytes[..8]);
            let actual = Receipt::read(&bytes[..]).unwrap();
            assert_eq!(expected, actual);
            Ok(())
        });
    }
}
