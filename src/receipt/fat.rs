#![allow(unused)]
use std::io::Error;
use std::io::ErrorKind;
use std::io::Read;

use crate::BigEndianRead;
use crate::BigEndianWrite;

pub struct FatBinary {
    arches: Vec<FatArch>,
}

impl FatBinary {
    pub fn read_be<R: Read>(mut reader: R) -> Result<Self, Error> {
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
    align: u32,
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
        let align = u32::read_be(reader.by_ref())?;
        Ok(Self {
            cpu_type,
            cpu_sub_type,
            offset,
            size,
            align,
        })
    }
}

const HEADER_LEN: usize = 8;
const MAGIC_32: u32 = 0xcafebabe;
const MAGIC_64: u32 = 0xcafebabf;
