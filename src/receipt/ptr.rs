use std::io::Error;
use std::io::Seek;
use std::io::Write;

use crate::receipt::Context;
use crate::BlockIo;
use crate::Blocks;
use crate::BigEndianIo;

pub struct Ptr<T>(T);

impl<T> Ptr<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T: BlockIo<Context>> From<T> for Ptr<T> {
    fn from(other: T) -> Ptr<T> {
        Self(other)
    }
}

impl<T: BlockIo<Context>> BlockIo<Context> for Ptr<T> {
    fn write_block<W: Write + Seek>(
        &self,
        mut writer: W,
        blocks: &mut Blocks,
        context: &mut Context,
    ) -> Result<u32, Error> {
        let i = self.0.write_block(writer.by_ref(), blocks, context)?;
        blocks.append(writer, |writer| i.write(writer))
    }

    fn read_block(
        i: u32,
        file: &[u8],
        blocks: &mut Blocks,
        context: &mut Context,
    ) -> Result<Self, Error> {
        let mut reader = blocks.slice(i, file)?;
        let i = u32::read(reader)?;
        let value = T::read_block(i, file, blocks, context)?;
        Ok(value.into())
    }
}
