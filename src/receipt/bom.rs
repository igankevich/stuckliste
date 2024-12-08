use std::ffi::CStr;
use std::fs::File;
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
use crate::Bom;

/// Configuration for creating a receipt.
pub struct ReceiptBuilder {
    paths_only: bool,
}

impl ReceiptBuilder {
    /// Create receipt builder with the default parameters.
    pub fn new() -> Self {
        Self { paths_only: false }
    }

    /// Do not include metadata in the receipt, include only file paths.
    pub fn paths_only(mut self, value: bool) -> Self {
        self.paths_only = value;
        self
    }

    /// Create a receipt using the provided parameters.
    pub fn create<P: AsRef<Path>>(self, directory: P) -> Result<Receipt, Error> {
        let entries = PathComponentVec::from_directory(directory, self.paths_only)?;
        Ok(Receipt { entries })
    }
}

impl Default for ReceiptBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// This is what is usually called a BOM file.
///
/// This file contains a list of file paths and metadata for an installed package.
#[derive(Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub struct Receipt {
    entries: PathComponentVec,
}

impl Receipt {
    /// Get paths and the corresponding metadata.
    pub fn entries(&self) -> Result<Vec<(PathBuf, Metadata)>, Error> {
        self.entries.to_paths()
    }

    /// Compute and return per-architecture file statistics.
    pub fn stats(&self) -> BomInfo {
        BomInfo::new(&self.entries)
    }

    /// Write receipt to `writer` in bill-of-materials (BOM) format.
    pub fn write<W: Write + Seek>(&self, mut writer: W) -> Result<(), Error> {
        // skip the header
        writer.seek(SeekFrom::Start(Bom::LEN as u64))?;
        let mut bom = Bom::new();
        let mut context = Context::new();
        bom.write_named(
            Self::V_INDEX,
            writer.by_ref(),
            &VirtualPathTree::new(),
            &mut context,
        )?;
        bom.write_named(
            Self::HL_INDEX,
            writer.by_ref(),
            &std::mem::take(&mut context.hard_links),
            &mut context,
        )?;
        bom.write_named(Self::PATHS, writer.by_ref(), &self.entries, &mut context)?;
        bom.write_named(
            Self::SIZE_64,
            writer.by_ref(),
            &std::mem::take(&mut context.file_sizes),
            &mut context,
        )?;
        bom.write_named(
            Self::BOM_INFO,
            writer.by_ref(),
            &BomInfo::new(&self.entries),
            &mut context,
        )?;
        // write the header
        bom.write(writer.by_ref())?;
        Ok(())
    }

    /// Read a receipt from file under `path`.
    pub fn open<P: AsRef<Path>>(self, path: P) -> Result<Receipt, Error> {
        let file = File::open(path)?;
        Self::read(file)
    }

    /// Read a receipt from `reader`.
    pub fn read<R: Read>(mut reader: R) -> Result<Self, Error> {
        let mut file = Vec::new();
        reader.read_to_end(&mut file)?;
        let mut bom = Bom::read(&file[..])?;
        let mut context = Context::new();
        //let _stats: BomInfo = bom.read_named(Self::BOM_INFO, &file, &mut context)?;
        //let _vindex: VirtualPathTree = bom.read_named(Self::V_INDEX, &file, &mut context)?;
        if let Some(i) = bom.get_named(Self::SIZE_64) {
            let file_sizes: FileSizes64 = bom.read_regular(i, &file, &mut context)?;
            context.file_sizes = file_sizes;
        }
        if let Some(i) = bom.get_named(Self::HL_INDEX) {
            let hard_links: HardLinks = bom.read_regular(i, &file, &mut context)?;
            context.hard_links = hard_links;
        }
        let entries: PathComponentVec = bom.read_named(Self::PATHS, &file, &mut context)?;
        Ok(Self { entries })
    }

    /// Virtual paths named block.
    ///
    /// Virtual paths (i.e. paths defined with regular expressions).
    pub const V_INDEX: &'static CStr = c"VIndex";

    /// Hard links named block.
    pub const HL_INDEX: &'static CStr = c"HLIndex";

    /// 64-bit file sizes named block.
    pub const SIZE_64: &'static CStr = c"Size64";

    /// Per-architecture file statistics named block,
    pub const BOM_INFO: &'static CStr = c"BomInfo";

    /// File path components tree named block.
    pub const PATHS: &'static CStr = c"Paths";
}

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
            assert_eq!(expected.stats(), actual.stats());
            Ok(())
        });
    }
}
