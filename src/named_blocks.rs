use std::ffi::CStr;
use std::ffi::CString;
use std::io::Error;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Write;

use crate::BigEndianIo;

/// Blocks addressed by a well-known name.
#[derive(Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub struct NamedBlocks {
    /// Block name to block index mapping.
    blocks: Vec<(CString, u32)>,
}

impl NamedBlocks {
    pub fn new() -> Self {
        Self {
            blocks: Default::default(),
        }
    }

    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub fn insert(&mut self, name: CString, block: u32) {
        self.blocks.push((name, block));
    }

    pub fn remove(&mut self, name: &CStr) -> Option<u32> {
        self.blocks
            .iter()
            .position(|(block_name, _block)| block_name.as_c_str() == name)
            .map(|i| self.blocks.remove(i).1)
    }

    pub fn get(&self, name: &CStr) -> Option<u32> {
        self.blocks.iter().find_map(|(block_name, block)| {
            if block_name.as_c_str() == name {
                Some(*block)
            } else {
                None
            }
        })
    }

    pub fn into_inner(self) -> Vec<(CString, u32)> {
        self.blocks
    }
}

impl BigEndianIo for NamedBlocks {
    fn read_be<R: Read>(mut reader: R) -> Result<Self, Error> {
        let num_named_blocks = u32::read_be(reader.by_ref())? as usize;
        let mut blocks = Vec::with_capacity(num_named_blocks);
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
            blocks.push((name, index));
        }
        //eprintln!("blocks {:?}", blocks);
        Ok(Self { blocks })
    }

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
    use crate::test::test_write_read;

    #[test]
    fn write_read() {
        test_write_read::<NamedBlocks>();
    }
}
