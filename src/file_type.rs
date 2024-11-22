use std::io::Error;
use std::io::ErrorKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(test, derive(arbitrary::Arbitrary))]
#[repr(u8)]
pub enum FileType {
    /// Regular file.
    File = 1,
    /// A directory.
    Directory = 2,
    /// Symbolic link.
    Link = 3,
    /// Block or character device.
    Device = 4,
}

impl TryFrom<u8> for FileType {
    type Error = Error;
    fn try_from(other: u8) -> Result<Self, Self::Error> {
        use FileType::*;
        match other {
            1 => Ok(File),
            2 => Ok(Directory),
            3 => Ok(Link),
            4 => Ok(Device),
            _ => Err(ErrorKind::InvalidData.into()),
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
