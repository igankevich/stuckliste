use std::io::Error;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;

use crate::receipt::CrcReader;
use crate::receipt::ExecutableArch;
use crate::BigEndianRead;

pub struct FatBinary {
    arches: Vec<FatArch>,
}

impl FatBinary {
    pub fn to_executable_arches<R: Read + Seek>(
        &self,
        mut file: R,
    ) -> Result<Vec<ExecutableArch>, Error> {
        let mut arches = Vec::with_capacity(self.arches.len());
        for arch in self.arches.iter() {
            arches.push(arch.to_executable_arch(file.by_ref())?);
        }
        Ok(arches)
    }
}

impl BigEndianRead for FatBinary {
    fn read_be<R: Read>(mut reader: R) -> Result<Self, Error> {
        let mut header = [0_u8; HEADER_LEN];
        reader.read_exact(&mut header[..])?;
        let magic = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
        let is_64_bit = match magic {
            MAGIC_32 => false,
            MAGIC_64 => true,
            _ => return Err(ErrorKind::InvalidInput.into()),
        };
        let num_arches = u32::from_be_bytes([header[4], header[5], header[6], header[7]]);
        let mut arches = Vec::with_capacity(num_arches as usize);
        for _ in 0..num_arches {
            arches.push(FatArch::read_be(reader.by_ref(), is_64_bit)?);
        }
        Ok(Self { arches })
    }
}

pub struct FatArch {
    cpu_type: u32,
    cpu_sub_type: u32,
    offset: u64,
    size: u64,
}

impl FatArch {
    pub fn read_be<R: Read>(mut reader: R, is_64_bit: bool) -> Result<Self, Error> {
        let cpu_type = u32::read_be(reader.by_ref())?;
        let cpu_sub_type = u32::read_be(reader.by_ref())?;
        let (offset, size) = if is_64_bit {
            let offset = u64::read_be(reader.by_ref())?;
            let size = u64::read_be(reader.by_ref())?;
            (offset, size)
        } else {
            let offset = u32::read_be(reader.by_ref())?;
            let size = u32::read_be(reader.by_ref())?;
            (offset as u64, size as u64)
        };
        Ok(Self {
            cpu_type,
            cpu_sub_type,
            offset,
            size,
        })
    }

    pub fn to_executable_arch<R: Read + Seek>(&self, mut file: R) -> Result<ExecutableArch, Error> {
        file.seek(SeekFrom::Start(self.offset))?;
        let file_slice = file.take(self.size);
        let crc_reader = CrcReader::new(file_slice);
        let checksum = crc_reader.digest()?;
        Ok(ExecutableArch {
            cpu_type: self.cpu_type,
            cpu_sub_type: self.cpu_sub_type,
            // This value overflows for files larger than 4 GiB.
            size: self.size as u32,
            checksum,
        })
    }
}

const HEADER_LEN: usize = 8;
const MAGIC_32: u32 = 0xcafebabe;
const MAGIC_64: u32 = 0xcafebabf;
