use std::io::Error;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Write;

use crate::BigEndianRead;
use crate::BigEndianWrite;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(test, derive(arbitrary::Arbitrary))]
#[repr(u8)]
pub enum FileType {
    /// Symbolic link.
    Symlink = 0o12,
    /// Regular file.
    Regular = 0o10,
    /// Block device.
    BlockDevice = 0o06,
    /// A directory.
    Directory = 0o4,
    /// Character device.
    CharDevice = 0o2,
}

impl FileType {
    /// Get file type from file mode.
    pub fn new(mode: u16) -> Result<Self, Error> {
        use FileType::*;
        const SYMLINK: u8 = FileType::Symlink as u8;
        const REGULAR: u8 = FileType::Regular as u8;
        const BLOCK: u8 = FileType::BlockDevice as u8;
        const DIRECTORY: u8 = FileType::Directory as u8;
        const CHAR: u8 = FileType::CharDevice as u8;
        match mode_to_file_type(mode) {
            SYMLINK => Ok(Symlink),
            REGULAR => Ok(Regular),
            BLOCK => Ok(BlockDevice),
            DIRECTORY => Ok(Directory),
            CHAR => Ok(CharDevice),
            _ => Err(ErrorKind::InvalidData.into()),
        }
    }

    pub fn to_entry_type(self) -> EntryType {
        use FileType::*;
        match self {
            Regular => EntryType::File,
            Directory => EntryType::Directory,
            Symlink => EntryType::Link,
            BlockDevice => EntryType::Device,
            CharDevice => EntryType::Device,
        }
    }

    #[cfg(test)]
    pub(crate) fn to_mode_bits(self) -> u16 {
        (self as u16) << 12
    }

    #[cfg(test)]
    pub(crate) fn set(self, mode: u16) -> u16 {
        const FILE_TYPE_MASK: u16 = 0b1111_0000_0000_0000_u16;
        (mode & !FILE_TYPE_MASK) | self.to_mode_bits()
    }
}

impl TryFrom<std::fs::FileType> for FileType {
    type Error = Error;
    fn try_from(other: std::fs::FileType) -> Result<Self, Self::Error> {
        use std::os::unix::fs::FileTypeExt;
        if other.is_dir() {
            Ok(Self::Directory)
        } else if other.is_symlink() {
            Ok(Self::Symlink)
        } else if other.is_block_device() {
            Ok(Self::BlockDevice)
        } else if other.is_char_device() {
            Ok(Self::CharDevice)
        } else if other.is_file() {
            Ok(Self::Regular)
        } else {
            // named pipes and sockets are not supported
            Err(ErrorKind::InvalidData.into())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(test, derive(arbitrary::Arbitrary))]
#[repr(u8)]
pub enum EntryType {
    File = 1,
    Directory = 2,
    Link = 3,
    Device = 4,
}

impl TryFrom<u8> for EntryType {
    type Error = Error;
    fn try_from(other: u8) -> Result<Self, Self::Error> {
        use EntryType::*;
        const FILE: u8 = File as u8;
        const DIRECTORY: u8 = Directory as u8;
        const LINK: u8 = Link as u8;
        const DEVICE: u8 = Device as u8;
        match other {
            FILE => Ok(File),
            DIRECTORY => Ok(Directory),
            LINK => Ok(Link),
            DEVICE => Ok(Device),
            _ => Err(ErrorKind::InvalidData.into()),
        }
    }
}

impl TryFrom<std::fs::FileType> for EntryType {
    type Error = Error;
    fn try_from(other: std::fs::FileType) -> Result<Self, Self::Error> {
        use std::os::unix::fs::FileTypeExt;
        if other.is_dir() {
            Ok(Self::Directory)
        } else if other.is_symlink() {
            Ok(Self::Link)
        } else if other.is_block_device() || other.is_char_device() {
            Ok(Self::Device)
        } else if other.is_file() {
            Ok(Self::File)
        } else {
            // named pipes and sockets are not supported
            Err(ErrorKind::InvalidData.into())
        }
    }
}

impl BigEndianRead for EntryType {
    fn read_be<R: Read>(reader: R) -> Result<Self, Error> {
        u8::read_be(reader)?.try_into()
    }
}

impl BigEndianWrite for EntryType {
    fn write_be<W: Write>(&self, writer: W) -> Result<(), Error> {
        (*self as u8).write_be(writer)
    }
}

pub(crate) fn mode_to_file_type(mode: u16) -> u8 {
    (mode >> 12) as u8
}
