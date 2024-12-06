use std::ffi::CStr;
use std::ffi::CString;
use std::io::Error;
use std::io::Seek;
use std::io::Write;

use crate::BigEndianRead;
use crate::BigEndianWrite;
use crate::Blocks;

/// Read values from BOM blocks.
pub trait BlockRead<C = ()> {
    /// Read `Self` from block `i`.
    fn read_block(i: u32, file: &[u8], blocks: &mut Blocks, context: &mut C) -> Result<Self, Error>
    where
        Self: Sized;
}

/// Read values into BOM blocks.
pub trait BlockWrite<C = ()> {
    /// Write `self` to a block and return its index.
    fn write_block<W: Write + Seek>(
        &self,
        writer: W,
        blocks: &mut Blocks,
        context: &mut C,
    ) -> Result<u32, Error>;
}

impl<T: BigEndianWrite, C> BlockWrite<C> for T {
    fn write_block<W: Write + Seek>(
        &self,
        writer: W,
        blocks: &mut Blocks,
        _context: &mut C,
    ) -> Result<u32, Error> {
        blocks.append(writer, |writer| BigEndianWrite::write_be(self, writer))
    }
}

impl<T: BigEndianRead, C> BlockRead<C> for T {
    fn read_block(i: u32, file: &[u8], blocks: &mut Blocks, _context: &mut C) -> Result<Self, Error>
    where
        Self: Sized,
    {
        BigEndianRead::read_be(blocks.slice(i, file)?)
    }
}

impl<C> BlockWrite<C> for CString {
    fn write_block<W: Write + Seek>(
        &self,
        writer: W,
        blocks: &mut Blocks,
        _context: &mut C,
    ) -> Result<u32, Error> {
        blocks.append(writer, |writer| writer.write_all(self.to_bytes_with_nul()))
    }
}

impl<C> BlockRead<C> for CString {
    fn read_block(
        i: u32,
        file: &[u8],
        blocks: &mut Blocks,
        _context: &mut C,
    ) -> Result<Self, Error> {
        let block = blocks.slice(i, file)?;
        let c_str = CStr::from_bytes_with_nul(block).map_err(Error::other)?;
        Ok(c_str.into())
    }
}

impl<C, T: BlockWrite<C>> BlockWrite<C> for Option<T> {
    fn write_block<W: Write + Seek>(
        &self,
        mut writer: W,
        blocks: &mut Blocks,
        context: &mut C,
    ) -> Result<u32, Error> {
        let i = match self {
            Some(value) => value.write_block(writer.by_ref(), blocks, context)?,
            None => blocks.append_null(writer.by_ref())?,
        };
        blocks.append(writer, |writer| i.write_be(writer))
    }
}

impl<C, T: BlockRead<C>> BlockRead<C> for Option<T> {
    fn read_block(
        i: u32,
        file: &[u8],
        blocks: &mut Blocks,
        context: &mut C,
    ) -> Result<Self, Error> {
        let reader = blocks.slice(i, file)?;
        let i = u32::read_be(reader)?;
        if blocks.slice(i, file)?.is_empty() {
            Ok(None)
        } else {
            let value = T::read_block(i, file, blocks, context)?;
            Ok(value.into())
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::test::block_io_symmetry;

    #[test]
    fn write_read_symmetry() {
        // Option<()> does not work due to BOM design.
        block_io_symmetry::<Option<u32>>();
        block_io_symmetry::<CString>();
        block_io_symmetry::<()>();
    }
}
