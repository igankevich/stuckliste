
use std::io::Error;
use std::io::Seek;
use std::io::Write;
use std::ffi::CString;
use std::ffi::CStr;

use crate::BigEndianIo;
use crate::Blocks;

pub trait BlockIo<C = ()> {
    fn write_block<W: Write + Seek>(
        &self,
        writer: W,
        blocks: &mut Blocks,
        context: &mut C,
    ) -> Result<u32, Error>;

    fn read_block(i: u32, file: &[u8], blocks: &mut Blocks, context: &mut C) -> Result<Self, Error>
    where
        Self: Sized;
}

impl<T: BigEndianIo, C> BlockIo<C> for T {
    fn write_block<W: Write + Seek>(
        &self,
        writer: W,
        blocks: &mut Blocks,
        _context: &mut C,
    ) -> Result<u32, Error> {
        blocks.append(writer, |writer| BigEndianIo::write(self, writer))
    }

    fn read_block(i: u32, file: &[u8], blocks: &mut Blocks, _context: &mut C) -> Result<Self, Error>
    where
        Self: Sized,
    {
        BigEndianIo::read(blocks.slice(i, &file)?)
    }
}

impl<C> BlockIo<C> for CString {
    fn write_block<W: Write + Seek>(
        &self,
        writer: W,
        blocks: &mut Blocks,
        _context: &mut C,
    ) -> Result<u32, Error> {
        blocks.append(writer, |writer| writer.write_all(self.to_bytes_with_nul()))
    }

    fn read_block(i: u32, file: &[u8], blocks: &mut Blocks, _context: &mut C) -> Result<Self, Error> {
        let block = blocks.slice(i, &file)?;
        let c_str = CStr::from_bytes_with_nul(&block[..]).map_err(Error::other)?;
        Ok(c_str.into())
    }
}
