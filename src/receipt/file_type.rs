use std::io::Error;
use std::io::ErrorKind;

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

    pub(crate) fn to_entry_type(self) -> u8 {
        use FileType::*;
        match self {
            Regular => 1,
            Directory => 2,
            Symlink => 3,
            BlockDevice => 4,
            CharDevice => 4,
        }
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

pub(crate) fn mode_to_file_type(mode: u16) -> u8 {
    (mode >> 12) as u8
}