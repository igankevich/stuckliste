#[cfg(test)]
use std::collections::HashSet;
use std::io::Error;
use std::io::Read;
use std::io::Seek;
use std::io::Write;

use crate::BigEndianIo;

/// Block of data in the BOM file.
///
/// Blocks are means of storage for both internal BOM structures and for user-supplied data.
#[derive(Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary))]
pub struct Blocks {
    /// Occupied blocks.
    blocks: Vec<Block>,
    /// Empty blocks, i.e. blocks with _len = offset = 0_.
    null_blocks: Vec<Block>,
    #[cfg(test)]
    unread_blocks: HashSet<usize>,
}

impl Blocks {
    pub fn new() -> Self {
        Self {
            // start with the null block
            blocks: vec![Block::null()],
            // write two empty blocks at the end
            null_blocks: vec![Block::null(), Block::null()],
            #[cfg(test)]
            unread_blocks: Default::default(),
        }
    }

    pub fn slice<'a>(&mut self, index: u32, file: &'a [u8]) -> Result<&'a [u8], Error> {
        let block = self
            .blocks
            .get(index as usize)
            .ok_or_else(|| Error::other("invalid block index"))?;
        #[cfg(test)]
        self.unread_blocks.remove(&(index as usize));
        let slice = block.slice(file);
        eprintln!(
            "read block index {} block {:?} slice {:?}",
            index, block, slice
        );
        Ok(slice)
    }

    pub fn block(&self, i: u32) -> &Block {
        &self.blocks[i as usize]
    }

    /// No. of non-null blocks.
    ///
    /// The space for null blocks is allocated in the index,
    /// but not in the file itself.
    pub fn num_non_null_blocks(&self) -> usize {
        self.blocks.iter().filter(|b| !b.is_null()).count()
    }

    pub fn append<W: Write + Seek, F: FnOnce(&mut W) -> Result<(), Error>>(
        &mut self,
        writer: W,
        f: F,
    ) -> Result<u32, Error> {
        let index = self.next_block_index();
        let block = Block::from_write(writer, f)?;
        //eprintln!("write block index {} block {:?}", index, block);
        self.blocks.push(block);
        Ok(index)
    }

    pub fn append_null<W: Write + Seek>(&mut self, mut writer: W) -> Result<u32, Error> {
        let index = self.next_block_index();
        let offset = writer.stream_position()? as u32;
        self.blocks.push(Block { offset, len: 0 });
        Ok(index)
    }

    pub fn next_block_index(&self) -> u32 {
        let index = self.blocks.len();
        index as u32
    }

    pub fn last_block_index(&self) -> Option<u32> {
        let len = self.blocks.len();
        (len != 0).then_some(len as u32 - 1)
    }
}

impl BigEndianIo for Blocks {
    fn read_be<R: Read>(mut reader: R) -> Result<Self, Error> {
        let num_blocks = u32::read_be(reader.by_ref())? as usize;
        let mut blocks = Vec::with_capacity(num_blocks);
        for _ in 0..num_blocks {
            let block = Block::read_be(reader.by_ref())?;
            blocks.push(block);
        }
        let num_free_blocks = u32::read_be(reader.by_ref())? as usize;
        let mut null_blocks = Vec::with_capacity(num_free_blocks);
        for _ in 0..num_free_blocks {
            let block = Block::read_be(reader.by_ref())?;
            null_blocks.push(block);
        }
        #[cfg(test)]
        let unread_blocks = blocks
            .iter()
            .enumerate()
            .filter_map(|(i, block)| if block.is_null() { None } else { Some(i) })
            .collect::<HashSet<_>>();
        Ok(Self {
            blocks,
            null_blocks,
            #[cfg(test)]
            unread_blocks,
        })
    }

    fn write_be<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        let num_blocks = self.blocks.len() as u32;
        num_blocks.write_be(writer.by_ref())?;
        for block in self.blocks.iter() {
            block.write_be(writer.by_ref())?;
        }
        let num_free_blocks = self.null_blocks.len() as u32;
        num_free_blocks.write_be(writer.by_ref())?;
        for block in self.null_blocks.iter() {
            block.write_be(writer.by_ref())?;
        }
        Ok(())
    }
}

impl Default for Blocks {
    fn default() -> Self {
        Self::new()
    }
}

/// A block of data.
#[derive(Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub struct Block {
    /// Byte offset from the start of the file.
    pub offset: u32,
    /// Size in bytes.
    pub len: u32,
}

impl Block {
    pub fn slice<'a>(&self, file: &'a [u8]) -> &'a [u8] {
        let i = self.offset as usize;
        let j = i + self.len as usize;
        //eprintln!("read block {:?}", &file[i..j]);
        &file[i..j]
    }

    pub fn is_null(&self) -> bool {
        self.offset == 0 && self.len == 0
    }

    pub fn null() -> Self {
        Self { offset: 0, len: 0 }
    }

    pub fn from_write<W: Write + Seek, F: FnOnce(&mut W) -> Result<(), Error>>(
        mut writer: W,
        f: F,
    ) -> Result<Self, Error> {
        let offset = writer.stream_position()?;
        f(writer.by_ref())?;
        let len = writer.stream_position()? - offset;
        Ok(Self {
            offset: offset
                .try_into()
                .map_err(|_| Error::other("the file is too large"))?,
            len: len
                .try_into()
                .map_err(|_| Error::other("the file is too large"))?,
        })
    }
}

impl BigEndianIo for Block {
    fn read_be<R: Read>(mut reader: R) -> Result<Self, Error> {
        let offset = u32::read_be(reader.by_ref())?;
        let len = u32::read_be(reader.by_ref())?;
        Ok(Self { offset, len })
    }

    fn write_be<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        self.offset.write_be(writer.by_ref())?;
        self.len.write_be(writer.by_ref())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test::test_be_io_symmetry;

    #[test]
    fn write_read() {
        test_be_io_symmetry::<Blocks>();
        test_be_io_symmetry::<Block>();
    }

    impl Blocks {
        fn print_unread_blocks(&self) {
            for i in self.unread_blocks.iter() {
                eprintln!("unread block {}: {:?}", i, self.blocks.get(*i));
            }
        }
    }

    impl PartialEq for Blocks {
        fn eq(&self, other: &Self) -> bool {
            (&self.blocks, &self.null_blocks).eq(&(&other.blocks, &other.null_blocks))
        }
    }

    impl Eq for Blocks {}

    impl Drop for Blocks {
        fn drop(&mut self) {
            self.print_unread_blocks();
        }
    }
}
