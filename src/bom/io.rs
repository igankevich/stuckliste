use std::io::Error;
use std::io::Read;
use std::io::Write;

/// Read binary values in big-endian order.
pub trait BigEndianRead {
    /// Read `Self` from `reader` using big-endian byte order.
    fn read_be<R: Read>(reader: R) -> Result<Self, Error>
    where
        Self: Sized;
}

/// Write binary values in big-endian order.
pub trait BigEndianWrite {
    /// Write `self` to `write` using big-endian byte order.
    fn write_be<W: Write>(&self, writer: W) -> Result<(), Error>;
}

impl BigEndianRead for u8 {
    fn read_be<R: Read>(mut reader: R) -> Result<Self, Error> {
        let mut data = [0_u8; 1];
        reader.read_exact(&mut data[..])?;
        Ok(data[0])
    }
}

impl BigEndianWrite for u8 {
    fn write_be<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        writer.write_all(self.to_be_bytes().as_slice())
    }
}

impl BigEndianRead for u16 {
    fn read_be<R: Read>(mut reader: R) -> Result<Self, Error> {
        let mut data = [0_u8; 2];
        reader.read_exact(&mut data[..])?;
        Ok(u16::from_be_bytes([data[0], data[1]]))
    }
}

impl BigEndianWrite for u16 {
    fn write_be<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        writer.write_all(self.to_be_bytes().as_slice())
    }
}

impl BigEndianRead for u32 {
    fn read_be<R: Read>(mut reader: R) -> Result<Self, Error> {
        let mut data = [0_u8; 4];
        reader.read_exact(&mut data[..])?;
        Ok(u32::from_be_bytes([data[0], data[1], data[2], data[3]]))
    }
}

impl BigEndianWrite for u32 {
    fn write_be<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        writer.write_all(self.to_be_bytes().as_slice())
    }
}

impl BigEndianRead for u64 {
    fn read_be<R: Read>(mut reader: R) -> Result<Self, Error> {
        let mut data = [0_u8; 8];
        reader.read_exact(&mut data[..])?;
        Ok(u64::from_be_bytes([
            data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
        ]))
    }
}

impl BigEndianWrite for u64 {
    fn write_be<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        writer.write_all(self.to_be_bytes().as_slice())
    }
}

impl BigEndianRead for () {
    fn read_be<R: Read>(_reader: R) -> Result<Self, Error> {
        Ok(())
    }
}

impl BigEndianWrite for () {
    fn write_be<W: Write>(&self, _writer: W) -> Result<(), Error> {
        Ok(())
    }
}
