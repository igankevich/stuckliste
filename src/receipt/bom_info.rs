use std::io::Error;
use std::io::Read;
use std::io::Write;

use crate::receipt::PathComponentVec;
use crate::BigEndianIo;

/// File paths statistics.
#[derive(Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub struct BomInfo {
    /// Total no. of paths.
    num_paths: u32,
    /// Per-architecture paths statistics.
    entries: Vec<BomInfoEntry>,
}

impl BomInfo {
    const VERSION: u32 = 1;

    pub fn new(tree: &PathComponentVec) -> Self {
        let mut stats = Self {
            num_paths: 0,
            entries: Default::default(),
        };
        for component in tree.iter() {
            component.accumulate(&mut stats);
        }
        stats
    }

    pub fn accumulate(&mut self, cpu_type: u32, file_size: u32) {
        match self
            .entries
            .iter_mut()
            .find(|entry| entry.cpu_type == cpu_type)
        {
            Some(ref mut entry) => entry.file_size += file_size,
            None => {
                self.entries.push(BomInfoEntry {
                    cpu_type,
                    file_size,
                });
            }
        }
        self.num_paths += 1;
    }
}

impl BigEndianIo for BomInfo {
    fn read_be<R: Read>(mut reader: R) -> Result<Self, Error> {
        let version = u32::read_be(reader.by_ref())?;
        if version != Self::VERSION {
            return Err(Error::other(format!(
                "unsupported BOMInfo version: {}",
                version
            )));
        }
        let num_paths = u32::read_be(reader.by_ref())?;
        let num_entries = u32::read_be(reader.by_ref())?;
        let mut entries = Vec::new();
        for _ in 0..num_entries {
            entries.push(BomInfoEntry::read_be(reader.by_ref())?);
        }
        Ok(Self { num_paths, entries })
    }

    fn write_be<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        Self::VERSION.write_be(writer.by_ref())?;
        self.num_paths.write_be(writer.by_ref())?;
        (self.entries.len() as u32).write_be(writer.by_ref())?;
        for entry in self.entries.iter() {
            entry.write_be(writer.by_ref())?;
        }
        Ok(())
    }
}

#[derive(Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub(crate) struct BomInfoEntry {
    cpu_type: u32,
    file_size: u32,
}

impl BigEndianIo for BomInfoEntry {
    fn read_be<R: Read>(mut reader: R) -> Result<Self, Error> {
        let cpu_type = u32::read_be(reader.by_ref())?;
        let _x1 = u32::read_be(reader.by_ref())?;
        let file_size = u32::read_be(reader.by_ref())?;
        let _x2 = u32::read_be(reader.by_ref())?;
        let entry = BomInfoEntry {
            cpu_type,
            file_size,
        };
        debug_assert!(_x1 == DEFAULT_X1 && _x2 == DEFAULT_X2, "entry = {entry:?}");
        Ok(entry)
    }

    fn write_be<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        self.cpu_type.write_be(writer.by_ref())?;
        DEFAULT_X1.write_be(writer.by_ref())?;
        self.file_size.write_be(writer.by_ref())?;
        DEFAULT_X2.write_be(writer.by_ref())?;
        Ok(())
    }
}

const DEFAULT_X1: u32 = 0;
const DEFAULT_X2: u32 = 0;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test::test_be_io_symmetry;

    #[test]
    fn write_read_symmetry() {
        test_be_io_symmetry::<BomInfo>();
        test_be_io_symmetry::<BomInfoEntry>();
    }
}
