use std::collections::HashMap;
use std::io::Error;
use std::io::Seek;
use std::io::Write;
use std::ops::Deref;
use std::ops::DerefMut;

use crate::receipt::Context;
use crate::receipt::Tree;
use crate::BlockIo;
use crate::Blocks;

/// Metadata block index to 64-bit file size mapping.
#[derive(Debug, Default)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub struct FileSizes64(HashMap<u32, u64>);

impl FileSizes64 {
    const BLOCK_LEN: usize = 128;

    pub fn into_inner(self) -> HashMap<u32, u64> {
        self.0
    }
}

impl Deref for FileSizes64 {
    type Target = HashMap<u32, u64>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for FileSizes64 {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl BlockIo<Context> for FileSizes64 {
    fn write_block<W: Write + Seek>(
        &self,
        mut writer: W,
        blocks: &mut Blocks,
        context: &mut Context,
    ) -> Result<u32, Error> {
        let file_size_tree = FileSizeTree::new(
            self.0.iter().map(|(k, v)| (*v, *k)),
            Self::BLOCK_LEN,
            writer.by_ref(),
            blocks,
            context,
        )?;
        let i = file_size_tree.write_block(writer.by_ref(), blocks, context)?;
        Ok(i)
    }

    fn read_block(
        i: u32,
        file: &[u8],
        blocks: &mut Blocks,
        context: &mut Context,
    ) -> Result<Self, Error> {
        let tree = FileSizeTree::read_block(i, file, blocks, context)?;
        let mut file_size_64 = HashMap::new();
        for (file_size, metadata_index) in tree.into_inner().into_entries() {
            file_size_64.insert(metadata_index, file_size);
        }
        Ok(Self(file_size_64))
    }
}

/// Key is file size, valus is metadata block index.
type FileSizeTree = Tree<u64, u32>;

#[cfg(test)]
mod tests {

    use super::*;
    use crate::test::block_io_symmetry;

    #[test]
    fn write_read_symmetry() {
        block_io_symmetry::<FileSizes64>();
    }
}
