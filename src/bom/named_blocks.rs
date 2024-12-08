use std::collections::HashMap;
use std::ffi::CStr;
use std::ffi::CString;
use std::io::Error;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Write;

use crate::BigEndianRead;
use crate::BigEndianWrite;

/// Blocks addressed by a well-known name.
#[derive(Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub struct NamedBlocks {
    /// Block name to block index mapping.
    blocks: HashMap<CString, u32>,
}

impl NamedBlocks {
    /// Construct empty named blocks.
    pub fn new() -> Self {
        Self {
            blocks: Default::default(),
        }
    }

    /// Get the number of named blocks.
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    /// Is there are no blocks?
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Add a new named block.
    pub fn insert(&mut self, name: CString, block: u32) {
        self.blocks.insert(name, block);
    }

    /// Remove existing named block.
    pub fn remove(&mut self, name: &CStr) -> Option<u32> {
        self.blocks.remove(name)
    }

    /// Get named block.
    pub fn get(&self, name: &CStr) -> Option<u32> {
        self.blocks.get(name).copied()
    }

    /// Transform into inner representation.
    pub fn into_inner(self) -> HashMap<CString, u32> {
        self.blocks
    }
}

impl BigEndianRead for NamedBlocks {
    fn read_be<R: Read>(mut reader: R) -> Result<Self, Error> {
        let num_named_blocks = u32::read_be(reader.by_ref())? as usize;
        let mut blocks = HashMap::with_capacity(num_named_blocks);
        for _ in 0..num_named_blocks {
            let index = u32::read_be(reader.by_ref())?;
            let len = u8::read_be(reader.by_ref())? as usize;
            let mut name = vec![0_u8; len];
            reader.read_exact(&mut name[..])?;
            // remove the null character if any
            if let Some(i) = name.iter().position(|b| *b == 0) {
                name.truncate(i);
            };
            let name = CString::new(name).map_err(|_| ErrorKind::InvalidData)?;
            blocks.insert(name, index);
        }
        Ok(Self { blocks })
    }
}

impl BigEndianWrite for NamedBlocks {
    fn write_be<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        let num_named_blocks = self.blocks.len() as u32;
        num_named_blocks.write_be(writer.by_ref())?;
        for (name, index) in self.blocks.iter() {
            let name = name.to_bytes();
            let len = name.len();
            if len > u8::MAX as usize {
                return Err(ErrorKind::InvalidData.into());
            }
            index.write_be(writer.by_ref())?;
            writer.write_all(&[len as u8])?;
            writer.write_all(name)?;
        }
        Ok(())
    }
}

impl Default for NamedBlocks {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test::test_be_io_symmetry;

    #[test]
    fn write_read() {
        test_be_io_symmetry::<NamedBlocks>();
    }
}
