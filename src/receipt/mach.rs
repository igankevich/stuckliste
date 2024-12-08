use std::io::Error;
use std::io::ErrorKind;
use std::io::Read;

use crate::receipt::ExecutableArch;
use crate::BigEndianRead;

pub struct MachObject {
    cpu_type: u32,
    cpu_sub_type: u32,
}

impl BigEndianRead for MachObject {
    fn read_be<R: Read>(mut reader: R) -> Result<Self, Error> {
        let magic = u32::read_be(reader.by_ref())?;
        let _is_64_bit = match magic {
            MAGIC_32 | CIGAM_32 => false,
            MAGIC_64 | CIGAM_64 => true,
            _ => return Err(ErrorKind::InvalidInput.into()),
        };
        let cpu_type = u32::read_be(reader.by_ref())?;
        let cpu_sub_type = u32::read_be(reader.by_ref())?;
        Ok(Self {
            cpu_type,
            cpu_sub_type,
        })
    }
}

impl From<MachObject> for ExecutableArch {
    fn from(other: MachObject) -> Self {
        ExecutableArch {
            cpu_type: other.cpu_type,
            cpu_sub_type: other.cpu_sub_type,
            size: 0,
            checksum: 0,
        }
    }
}

const MAGIC_32: u32 = 0xfeedface;
const CIGAM_32: u32 = 0xcefaedfe;
const MAGIC_64: u32 = 0xfeedfacf;
const CIGAM_64: u32 = 0xcffaedfe;
