use std::ffi::CStr;
use std::ffi::CString;
use std::io::Error;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;

use crate::BigEndianRead;
use crate::BigEndianWrite;
use crate::Block;
use crate::BlockRead;
use crate::BlockWrite;
use crate::Blocks;
use crate::NamedBlocks;

/// BOM file low-level representation.
///
/// Contains regular and named blocks.
#[derive(Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub struct Bom {
    /// Regular blocks. Addressed by an index.
    blocks: Blocks,
    /// Named blocks. Addressed by a well-known name.
    named_blocks: NamedBlocks,
}

impl Default for Bom {
    fn default() -> Self {
        Self::new()
    }
}

impl Bom {
    const VERSION: u32 = 1;

    /// Bom length with padding.
    pub(crate) const LEN: usize = 512;

    /// Create a new empty BOM.
    pub fn new() -> Self {
        Self {
            blocks: Blocks::new(),
            named_blocks: NamedBlocks::new(),
        }
    }

    /// Get all regular blocks.
    pub fn blocks(&self) -> &Blocks {
        &self.blocks
    }

    /// Get all named blocks.
    pub fn named_blocks(&self) -> &NamedBlocks {
        &self.named_blocks
    }

    /// Write `value` into a new named block.
    pub fn write_named<N, W, C, T>(
        &mut self,
        name: N,
        writer: W,
        value: &T,
        context: &mut C,
    ) -> Result<(), Error>
    where
        N: Into<CString>,
        W: Write + Seek,
        T: BlockWrite<C>,
    {
        let i = value.write_block(writer, &mut self.blocks, context)?;
        self.named_blocks.insert(name.into(), i);
        Ok(())
    }

    /// Get block index by name.
    pub fn get_named(&self, name: &CStr) -> Option<u32> {
        self.named_blocks.get(name)
    }

    /// Read a value of type `T` from a named block.
    pub fn read_named<C, T: BlockRead<C>>(
        &mut self,
        name: &CStr,
        file: &[u8],
        context: &mut C,
    ) -> Result<T, Error> {
        let i = self
            .named_blocks
            .get(name)
            .ok_or_else(|| Error::other(format!("`{:?}` block not found", name)))?;
        T::read_block(i, file, &mut self.blocks, context)
    }

    /// Read a value of type `T` from a regular block.
    pub fn read_regular<C, T: BlockRead<C>>(
        &mut self,
        block_index: u32,
        file: &[u8],
        context: &mut C,
    ) -> Result<T, Error> {
        T::read_block(block_index, file, &mut self.blocks, context)
    }

    /// Read BOM header from `file`.
    pub fn read(file: &[u8]) -> Result<Self, Error> {
        let mut reader = file;
        let mut magic = [0_u8; BOM_MAGIC.len()];
        reader.read_exact(&mut magic[..])?;
        if magic != BOM_MAGIC {
            return Err(Error::other("not a bom store"));
        }
        let version = u32::read_be(reader.by_ref())?;
        if version != 1 {
            return Err(Error::other(format!(
                "unsupported BOM store version: {}",
                version
            )));
        }
        let num_non_null_blocks = u32::read_be(reader.by_ref())?;
        let blocks = Block {
            offset: u32::read_be(reader.by_ref())?,
            len: u32::read_be(reader.by_ref())?,
        };
        let named_blocks = Block {
            offset: u32::read_be(reader.by_ref())?,
            len: u32::read_be(reader.by_ref())?,
        };
        let blocks = Blocks::read_be(blocks.slice(file))?;
        let named_blocks = NamedBlocks::read_be(named_blocks.slice(file))?;
        // TODO ???
        debug_assert!(
            num_non_null_blocks as usize >= blocks.num_non_null_blocks(),
            "num_non_null_blocks = {num_non_null_blocks}, \
            blocks.num_non_null_blocks = {}",
            blocks.num_non_null_blocks()
        );
        Ok(Self {
            blocks,
            named_blocks,
        })
    }

    /// Write BOM header at the beginning of `writer`.
    ///
    /// This method requires 512 byte to be reserved at the beginning of the file to not overwrite
    /// any important data.
    pub fn write<W: Write + Seek>(&self, mut writer: W) -> Result<(), Error> {
        // append blocks at the current position
        let position = writer.stream_position()?;
        if position < Bom::LEN as u64 {
            // ensure that we have enough space for the header
            writer.seek(SeekFrom::Start(Bom::LEN as u64))?;
        }
        let named_blocks =
            Block::from_write(writer.by_ref(), |writer| self.named_blocks.write_be(writer))?;
        let blocks = Block::from_write(writer.by_ref(), |writer| self.blocks.write_be(writer))?;
        // write the header at the beginning
        writer.rewind()?;
        writer.write_all(&BOM_MAGIC[..])?;
        Self::VERSION.write_be(writer.by_ref())?;
        (self.blocks.num_non_null_blocks() as u32).write_be(writer.by_ref())?;
        blocks.offset.write_be(writer.by_ref())?;
        blocks.len.write_be(writer.by_ref())?;
        named_blocks.offset.write_be(writer.by_ref())?;
        named_blocks.len.write_be(writer.by_ref())?;
        writer.write_all(&[0_u8; HEADER_PADDING])?;
        Ok(())
    }
}

const BOM_MAGIC: [u8; 8] = *b"BOMStore";

/// Bom length without padding.
const REAL_HEADER_LEN: usize = 32;
const HEADER_PADDING: usize = Bom::LEN - REAL_HEADER_LEN;

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use arbtest::arbtest;

    use super::*;

    #[test]
    fn write_read() {
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
}
