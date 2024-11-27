use std::io::Error;
use std::io::ErrorKind;
use std::io::Seek;
use std::io::Write;

use crate::BigEndianIo;
use crate::Block;
use crate::Blocks;
use crate::NamedBlocks;

#[derive(Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub struct Bom {
    /// Regular blocks. Addressed by an index.
    pub blocks: Blocks,
    /// Named blocks. Addressed by a well-known name.
    pub named_blocks: NamedBlocks,
}

impl Bom {
    const VERSION: u32 = 1;

    /// Bom length with padding.
    pub(crate) const LEN: usize = 512;

    pub fn read(file: &[u8]) -> Result<Self, Error> {
        if file.len() < Bom::LEN {
            return Err(ErrorKind::UnexpectedEof.into());
        }
        if file[..BOM_MAGIC.len()] != BOM_MAGIC[..] {
            return Err(Error::other("not a bom store"));
        }
        let version = u32_read(&file[8..12]);
        if version != 1 {
            return Err(Error::other(format!(
                "unsupported BOM store version: {}",
                version
            )));
        }
        let num_non_null_blocks = u32_read(&file[12..16]);
        let blocks = Block {
            offset: u32_read(&file[16..20]),
            len: u32_read(&file[20..24]),
        };
        let named_blocks = Block {
            offset: u32_read(&file[24..28]),
            len: u32_read(&file[28..32]),
        };
        let blocks = Blocks::read(blocks.slice(&file))?;
        let named_blocks = NamedBlocks::read(named_blocks.slice(&file))?;
        // TODO ???
        debug_assert!(num_non_null_blocks as usize >= blocks.num_non_null_blocks(),
            "num_non_null_blocks = {num_non_null_blocks}, \
            blocks.num_non_null_blocks = {}", blocks.num_non_null_blocks());
        Ok(Self {
            blocks,
            named_blocks,
        })
    }

    pub fn write<W: Write + Seek>(&self, mut writer: W) -> Result<(), Error> {
        let named_blocks =
            Block::from_write(writer.by_ref(), |writer| self.named_blocks.write(writer))?;
        let blocks = Block::from_write(writer.by_ref(), |writer| self.blocks.write(writer))?;
        writer.write_all(&BOM_MAGIC[..])?;
        Self::VERSION.write(writer.by_ref())?;
        (self.blocks.num_non_null_blocks() as u32).write(writer.by_ref())?;
        blocks.offset.write(writer.by_ref())?;
        blocks.len.write(writer.by_ref())?;
        named_blocks.offset.write(writer.by_ref())?;
        named_blocks.len.write(writer.by_ref())?;
        writer.write_all(&[0_u8; HEADER_PADDING])?;
        Ok(())
    }
}

fn u32_read(data: &[u8]) -> u32 {
    u32::from_be_bytes([data[0], data[1], data[2], data[3]])
}

const BOM_MAGIC: [u8; 8] = *b"BOMStore";

/// Bom length without padding.
const REAL_HEADER_LEN: usize = 32;
const HEADER_PADDING: usize = Bom::LEN - REAL_HEADER_LEN;
