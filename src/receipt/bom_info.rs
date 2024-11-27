use std::io::Error;
use std::io::Read;
use std::io::Write;

use crate::receipt::PathTree;
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

    pub fn new(tree: &PathTree) -> Self {
        let mut stats = Self {
            num_paths: 0,
            entries: Default::default(),
        };
        for component in tree.nodes().values() {
            component.metadata.accumulate(&mut stats);
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
    fn read<R: Read>(mut reader: R) -> Result<Self, Error> {
        let version = u32::read(reader.by_ref())?;
        if version != Self::VERSION {
            return Err(Error::other(format!(
                "unsupported BOMInfo version: {}",
                version
            )));
        }
        let num_paths = u32::read(reader.by_ref())?;
        let num_entries = u32::read(reader.by_ref())?;
        //eprintln!("num paths {}", num_paths);
        //eprintln!("num entries {}", num_entries);
        let mut entries = Vec::new();
        for _ in 0..num_entries {
            entries.push(BomInfoEntry::read(reader.by_ref())?);
        }
        Ok(Self { num_paths, entries })
    }

    fn write<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        Self::VERSION.write(writer.by_ref())?;
        self.num_paths.write(writer.by_ref())?;
        (self.entries.len() as u32).write(writer.by_ref())?;
        for entry in self.entries.iter() {
            entry.write(writer.by_ref())?;
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
    fn read<R: Read>(mut reader: R) -> Result<Self, Error> {
        let cpu_type = u32::read(reader.by_ref())?;
        let reserved1 = u32::read(reader.by_ref())?;
        let file_size = u32::read(reader.by_ref())?;
        let reserved2 = u32::read(reader.by_ref())?;
        let entry = BomInfoEntry {
            cpu_type,
            file_size,
        };
        debug_assert!(reserved1 == 0 && reserved2 == 0, "entry = {entry:?}");
        Ok(entry)
    }

    fn write<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        let reserved1 = 0_u32;
        let reserved2 = 0_u32;
        self.cpu_type.write(writer.by_ref())?;
        reserved1.write(writer.by_ref())?;
        self.file_size.write(writer.by_ref())?;
        reserved2.write(writer.by_ref())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test::test_write_read;

    #[test]
    fn write_read_symmetry() {
        test_write_read::<BomInfo>();
        test_write_read::<BomInfoEntry>();
    }
}
