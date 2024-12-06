use std::io::Error;
use std::io::Seek;
use std::io::Write;

use crate::receipt::Context;
use crate::BigEndianIo;
use crate::BlockIo;
use crate::Blocks;

#[derive(Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub struct Ptr<T>(T);

impl<T> Ptr<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }

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
        blocks.append(writer, |writer| i.write_be(writer))
    }

    fn read_block(
        i: u32,
        file: &[u8],
        blocks: &mut Blocks,
        context: &mut Context,
    ) -> Result<Self, Error> {
        let reader = blocks.slice(i, file)?;
        let i = u32::read_be(reader)?;
        let value = T::read_block(i, file, blocks, context)?;
        Ok(value.into())
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::test::block_io_symmetry;

    #[test]
    fn write_read_symmetry() {
        block_io_symmetry::<Ptr<()>>();
        block_io_symmetry::<Ptr<u32>>();
    }
}
