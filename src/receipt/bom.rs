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
        Ok(Receipt {
            entries,
            stats: None,
        })
    }
}

impl Default for ReceiptBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Receipt read options.
pub struct ReceiptOptions {
    stats: bool,
}

impl ReceiptOptions {
    /// Get default options.
    pub fn new() -> Self {
        Self { stats: false }
    }

    /// Read `BomInfo` block.
    pub fn stats(mut self, value: bool) -> Self {
        self.stats = value;
        self
    }

    /// Read a receipt using the provided parameters.
    pub fn open<P: AsRef<Path>>(self, path: P) -> Result<Receipt, Error> {
        let file = File::open(path)?;
        Receipt::do_read(file, self)
    }

    /// Read a receipt using the provided parameters.
    pub fn read<R: Read>(self, reader: R) -> Result<Receipt, Error> {
        Receipt::do_read(reader, self)
    }
}

impl Default for ReceiptOptions {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub struct Receipt {
    entries: PathComponentVec,
    // TODO split into reader/writer??
    stats: Option<BomInfo>,
}

impl Receipt {
    /// Get paths and the corresponding metadata.
    pub fn entries(&self) -> Result<Vec<(PathBuf, Metadata)>, Error> {
        self.entries.to_paths()
    }

    /// Get per-architecture file statistics.
    pub fn stats(&self) -> Option<&BomInfo> {
        self.stats.as_ref()
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
            &std::mem::take(&mut context.file_size_64),
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

    pub fn options() -> ReceiptOptions {
        Default::default()
    }

    /// Read a receipt from `reader` using bill-of-materials (BOM) format.
    pub fn read<R: Read>(reader: R) -> Result<Self, Error> {
        Self::do_read(reader, Default::default())
    }

    fn do_read<R: Read>(mut reader: R, options: ReceiptOptions) -> Result<Self, Error> {
        let mut file = Vec::new();
        reader.read_to_end(&mut file)?;
        let mut bom = Bom::read(&file[..])?;
        let mut context = Context::new();
        let stats: Option<BomInfo> = if options.stats {
            Some(bom.read_named(Self::BOM_INFO, &file, &mut context)?)
        } else {
            None
        };
        let _vindex: VirtualPathTree = bom.read_named(Self::V_INDEX, &file, &mut context)?;
        if let Some(i) = bom.get_named(Self::SIZE_64) {
            let file_size_64: FileSizes64 = bom.read_regular(i, &file, &mut context)?;
            context.file_size_64 = file_size_64;
        }
        if let Some(i) = bom.get_named(Self::HL_INDEX) {
            let hard_links: HardLinks = bom.read_regular(i, &file, &mut context)?;
            context.hard_links = hard_links;
        }
        let entries: PathComponentVec = bom.read_named(Self::PATHS, &file, &mut context)?;
        Ok(Self { entries, stats })
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
            let actual = Receipt::options().stats(true).read(&bytes[..]).unwrap();
            // TODO stats?
            assert_eq!(expected.entries, actual.entries);
            Ok(())
        });
    }
}
