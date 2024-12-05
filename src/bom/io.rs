use std::io::Error;
use std::io::Read;
use std::io::Write;

pub trait BigEndianIo {
    fn read_be<R: Read>(reader: R) -> Result<Self, Error>
    where
        Self: Sized;

    fn write_be<W: Write>(&self, writer: W) -> Result<(), Error>;
}

impl BigEndianIo for u8 {
    fn read_be<R: Read>(mut reader: R) -> Result<Self, Error> {
        let mut data = [0_u8; 1];
        reader.read_exact(&mut data[..])?;
        Ok(data[0])
    }

    fn write_be<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        writer.write_all(self.to_be_bytes().as_slice())
    }
}

impl BigEndianIo for u16 {
    fn read_be<R: Read>(mut reader: R) -> Result<Self, Error> {
        let mut data = [0_u8; 2];
        reader.read_exact(&mut data[..])?;
        Ok(u16::from_be_bytes([data[0], data[1]]))
    }

    fn write_be<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        writer.write_all(self.to_be_bytes().as_slice())
    }
}

impl BigEndianIo for u32 {
    fn read_be<R: Read>(mut reader: R) -> Result<Self, Error> {
        let mut data = [0_u8; 4];
        reader.read_exact(&mut data[..])?;
        Ok(u32::from_be_bytes([data[0], data[1], data[2], data[3]]))
    }

    fn write_be<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        writer.write_all(self.to_be_bytes().as_slice())
    }
}

impl BigEndianIo for u64 {
    fn read_be<R: Read>(mut reader: R) -> Result<Self, Error> {
        let mut data = [0_u8; 8];
        reader.read_exact(&mut data[..])?;
        Ok(u64::from_be_bytes([
            data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
        ]))
    }

    fn write_be<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        writer.write_all(self.to_be_bytes().as_slice())
    }
}

impl BigEndianIo for () {
    fn read_be<R: Read>(_reader: R) -> Result<Self, Error> {
        Ok(())
    }

    fn write_be<W: Write>(&self, _writer: W) -> Result<(), Error> {
        Ok(())
    }
}
