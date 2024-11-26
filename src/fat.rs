#![allow(unused)]
use std::io::Error;
use std::io::ErrorKind;
use std::io::Read;

pub struct FatBinary {
    arches: Vec<FatArch>,
}

impl FatBinary {
    pub fn read<R: Read>(mut reader: R) -> Result<Self, Error> {
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
            arches.push(FatArch::read(reader.by_ref(), is_64_bit)?);
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
    pub fn read<R: Read>(mut reader: R, is_64_bit: bool) -> Result<Self, Error> {
        let cpu_type = u32_read(reader.by_ref())?;
        let cpu_sub_type = u32_read(reader.by_ref())?;
        let (offset, size) = if is_64_bit {
            let offset = u64_read(reader.by_ref())?;
            let size = u64_read(reader.by_ref())?;
            (offset, size)
        } else {
            let offset = u32_read(reader.by_ref())?;
            let size = u32_read(reader.by_ref())?;
            (offset as u64, size as u64)
        };
        let align = u32_read(reader.by_ref())?;
        Ok(Self {
            cpu_type,
            cpu_sub_type,
            offset,
            size,
            align,
        })
    }
}

fn u32_read<R: Read>(mut reader: R) -> Result<u32, Error> {
    let mut data = [0_u8; 4];
    reader.read_exact(&mut data[..])?;
    Ok(u32::from_be_bytes([data[0], data[1], data[2], data[3]]))
}

fn u64_read<R: Read>(mut reader: R) -> Result<u64, Error> {
    let mut data = [0_u8; 8];
    reader.read_exact(&mut data[..])?;
    Ok(u64::from_be_bytes([
        data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
    ]))
}

const HEADER_LEN: usize = 8;
const MAGIC_32: u32 = 0xcafebabe;
const MAGIC_64: u32 = 0xcafebabf;
