use std::ffi::CString;
use std::ffi::OsStr;
use std::io::Cursor;
use std::io::Error;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Seek;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::time::Duration;
use std::time::SystemTime;

use crate::receipt::BomInfo;
use crate::receipt::Context;
use crate::receipt::EntryType;
use crate::receipt::FileType;
use crate::BigEndianIo;
use crate::BlockIo;
use crate::Blocks;

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct Metadata {
    mode: u16,
    uid: u32,
    gid: u32,
    mtime: u32,
    pub(crate) size: u64,
    pub extra: MetadataExtra,
}

impl Metadata {
    pub fn file_type(&self) -> FileType {
        // TODO
        //use MetadataExtra::*;
        //match self.extra {
        //    PathOnly { entry_type } => ,
        FileType::new(self.mode).unwrap_or(FileType::Regular)
    }

    pub fn entry_type(&self) -> EntryType {
        self.extra.entry_type()
    }

    pub fn mode(&self) -> u16 {
        self.mode
    }

    pub fn uid(&self) -> u32 {
        self.uid
    }

    pub fn gid(&self) -> u32 {
        self.gid
    }

    pub fn mtime(&self) -> u32 {
        self.mtime
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    /// Last modification time.
    pub fn modified(&self) -> Result<SystemTime, Error> {
        let dt = Duration::from_secs(self.mtime.into());
        SystemTime::UNIX_EPOCH
            .checked_add(dt)
            .ok_or_else(|| Error::new(ErrorKind::InvalidData, "out of range timestamp"))
    }

    pub fn checksum(&self) -> u32 {
        match self.extra {
            MetadataExtra::File { checksum } => checksum,
            MetadataExtra::Executable(Executable { checksum, .. }) => checksum,
            MetadataExtra::Directory => 0,
            MetadataExtra::Link(Link { checksum, .. }) => checksum,
            MetadataExtra::Device(Device { .. }) => 0,
            MetadataExtra::PathOnly { .. } => 0,
        }
    }

    fn flags(&self) -> u16 {
        // flags 0xN00P
        // N - no. of architectures in a fat binary
        // P - 0xf for regular bom, 0 for path-only bom
        let path_only = match self.extra {
            MetadataExtra::PathOnly { .. } => 0_u16,
            _ => 0xf_u16,
        };
        let binary_type = match self.extra {
            MetadataExtra::Executable(Executable { ref arches, .. }) => {
                // TODO this probably depends on file magic, not on the number of arches
                if arches.len() == 1 {
                    BinaryType::Executable as u16
                } else {
                    BinaryType::Fat as u16
                }
            }
            _ => BinaryType::Unknown as u16,
        };
        ((binary_type & 0xf) << 12) | (path_only & 0xf)
    }

    pub(crate) fn accumulate(&self, stats: &mut BomInfo) {
        match self.extra {
            MetadataExtra::Executable(Executable { ref arches, .. }) => {
                for arch in arches.iter() {
                    stats.accumulate(arch.cpu_type, arch.size);
                }
            }
            // BomInfo wraps around file size if it's larger than u32::MAX
            _ => stats.accumulate(0, self.size as u32),
        }
    }

    fn read<R: Read>(mut reader: R) -> Result<Self, Error> {
        let entry_type = EntryType::read_be(reader.by_ref())?;
        let _x0 = u8::read_be(reader.by_ref())?;
        debug_assert!(_x0 == 1, "x0 {:?}", _x0);
        let flags = u16::read_be(reader.by_ref())?;
        if is_path_only(flags) {
            // This BOM stores paths only.
            let metadata = Self {
                mode: 0,
                uid: 0,
                gid: 0,
                mtime: 0,
                size: 0,
                extra: MetadataExtra::PathOnly { entry_type },
            };
            return Ok(metadata);
        }
        let binary_type = get_binary_type(flags);
        let mode = u16::read_be(reader.by_ref())?;
        let uid = u32::read_be(reader.by_ref())?;
        let gid = u32::read_be(reader.by_ref())?;
        let mtime = u32::read_be(reader.by_ref())?;
        let size = u32::read_be(reader.by_ref())?;
        let _x1 = u8::read_be(reader.by_ref())?;
        debug_assert!(_x1 == 1, "x1 {:?}", _x1);
        let file_type = FileType::new(mode)?;
        debug_assert!(file_type.to_entry_type() == entry_type);
        let extra = match file_type {
            FileType::Regular if binary_type != BinaryType::Unknown => {
                let checksum = u32::read_be(reader.by_ref())?;
                let flag = u8::read_be(reader.by_ref())?;
                debug_assert!(flag == 1, "flag = {flag}");
                let num_arch_again = u32::read_be(reader.by_ref())?;
                let mut arches = Vec::with_capacity(num_arch_again as usize);
                for _ in 0..num_arch_again {
                    arches.push(ExeArch::read_be(reader.by_ref())?);
                }
                MetadataExtra::Executable(Executable { checksum, arches })
            }
            FileType::Regular => {
                debug_assert!(
                    binary_type == BinaryType::Unknown,
                    "unexpected binary type {:?}",
                    binary_type
                );
                let checksum = u32::read_be(reader.by_ref())?;
                MetadataExtra::File { checksum }
            }
            FileType::Directory => {
                debug_assert!(
                    binary_type == BinaryType::Unknown,
                    "unexpected binary type {:?}",
                    binary_type
                );
                MetadataExtra::Directory
            }
            FileType::Symlink => {
                let checksum = u32::read_be(reader.by_ref())?;
                let name_len = u32::read_be(reader.by_ref())?;
                debug_assert!(
                    name_len == 0 || file_type == FileType::Symlink,
                    "file_type = {:?}, name_len = {}",
                    file_type,
                    name_len
                );
                let mut name = vec![0_u8; name_len as usize];
                reader.read_exact(&mut name[..])?;
                let name = CString::from_vec_with_nul(name).map_err(Error::other)?;
                let name = OsStr::from_bytes(name.to_bytes());
                MetadataExtra::Link(Link {
                    checksum,
                    name: name.into(),
                })
            }
            FileType::CharDevice | FileType::BlockDevice => {
                debug_assert!(
                    binary_type == BinaryType::Unknown,
                    "unexpected binary type {:?}",
                    binary_type
                );
                let dev = u32::read_be(reader.by_ref())?;
                MetadataExtra::Device(Device { dev })
            }
        };
        let metadata = Self {
            mode,
            uid,
            gid,
            mtime,
            size: size as u64,
            extra,
        };
        // We ignore 8 zero bytes here. Bomutils' `mkbom` doesn't write them but the original `mkbom` does.
        Ok(metadata)
    }

    fn write<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        self.entry_type().write_be(writer.by_ref())?;
        1_u8.write_be(writer.by_ref())?;
        let flags = self.flags();
        flags.write_be(writer.by_ref())?;
        if is_path_only(flags) {
            return Ok(());
        }
        self.mode.write_be(writer.by_ref())?;
        self.uid.write_be(writer.by_ref())?;
        self.gid.write_be(writer.by_ref())?;
        self.mtime.write_be(writer.by_ref())?;
        (self.size as u32).write_be(writer.by_ref())?; // truncate the size
        1_u8.write_be(writer.by_ref())?;
        match &self.extra {
            MetadataExtra::File { checksum } => {
                checksum.write_be(writer.by_ref())?;
            }
            MetadataExtra::Executable(Executable { checksum, arches }) => {
                checksum.write_be(writer.by_ref())?;
                1_u8.write_be(writer.by_ref())?;
                let num_arches = arches.len() as u32;
                num_arches.write_be(writer.by_ref())?;
                for arch in arches.iter() {
                    arch.write_be(writer.by_ref())?;
                }
            }
            MetadataExtra::Directory => {}
            MetadataExtra::Link(Link { checksum, name }) => {
                checksum.write_be(writer.by_ref())?;
                let name_bytes = name.as_os_str().as_bytes();
                // +1 because of the nul byte
                ((name_bytes.len() + 1) as u32).write_be(writer.by_ref())?;
                writer.write_all(name_bytes)?;
                writer.write_all(&[0_u8])?;
            }
            MetadataExtra::Device(Device { dev }) => {
                dev.write_be(writer.by_ref())?;
            }
            MetadataExtra::PathOnly { .. } => {}
        }
        // Block always ends with 8 zeroes.
        writer.write_all(&[0_u8; 8])?;
        Ok(())
    }
}

impl BlockIo<Context> for Metadata {
    fn read_block(
        i: u32,
        file: &[u8],
        blocks: &mut Blocks,
        context: &mut Context,
    ) -> Result<Self, Error> {
        let reader = blocks.slice(i, file)?;
        let block_len = reader.len();
        let mut cursor = Cursor::new(reader);
        let mut metadata = Self::read(cursor.by_ref())?;
        if let Some(size) = context.file_size_64.get(&i) {
            metadata.size = *size;
        }
        let unread_bytes = block_len - reader.len();
        debug_assert!(unread_bytes == 0, "unread_bytes = {unread_bytes}");
        Ok(metadata)
    }

    fn write_block<W: Write + Seek>(
        &self,
        mut writer: W,
        blocks: &mut Blocks,
        context: &mut Context,
    ) -> Result<u32, Error> {
        let i = blocks.append(writer.by_ref(), |writer| self.write(writer))?;
        let file_size = self.size();
        if file_size > u32::MAX as u64 {
            context.file_size_64.insert(i, file_size);
        }
        Ok(i)
    }
}

impl TryFrom<std::fs::Metadata> for Metadata {
    type Error = Error;
    fn try_from(other: std::fs::Metadata) -> Result<Self, Self::Error> {
        use std::os::unix::fs::MetadataExt;
        let kind: FileType = other.file_type().try_into()?;
        let extra = match kind {
            FileType::Regular => MetadataExtra::File { checksum: 0 },
            FileType::Directory => MetadataExtra::Directory,
            FileType::Symlink => MetadataExtra::Link(Link {
                checksum: 0,
                name: Default::default(),
            }),
            FileType::CharDevice | FileType::BlockDevice => MetadataExtra::Device(Device {
                dev: libc_dev_to_bom_dev(other.rdev()),
            }),
        };
        Ok(Self {
            mode: other.mode() as u16,
            uid: other.uid(),
            gid: other.gid(),
            mtime: other.mtime().try_into().unwrap_or(0),
            size: other.size(),
            extra,
        })
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub enum MetadataExtra {
    File { checksum: u32 },
    Executable(Executable),
    Directory,
    Link(Link),
    Device(Device),
    PathOnly { entry_type: EntryType },
}

impl MetadataExtra {
    fn entry_type(&self) -> EntryType {
        use MetadataExtra::*;
        match self {
            File { .. } | Executable { .. } => EntryType::File,
            Link { .. } => EntryType::Link,
            Directory => EntryType::Directory,
            Device { .. } => EntryType::Device,
            PathOnly { entry_type } => *entry_type,
        }
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct Executable {
    pub checksum: u32,
    pub arches: Vec<ExeArch>,
}

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub struct Device {
    pub dev: u32,
}

impl Device {
    pub fn rdev(&self) -> u64 {
        bom_dev_to_libc_dev(self.dev)
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct Link {
    pub checksum: u32,
    pub name: PathBuf,
}

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub struct ExeArch {
    pub cpu_type: u32,
    pub cpu_sub_type: u32,
    // If the actual binary size is u64 then this field overflows.
    pub size: u32,
    pub checksum: u32,
}

impl BigEndianIo for ExeArch {
    fn read_be<R: Read>(mut reader: R) -> Result<Self, Error> {
        let cpu_type = u32::read_be(reader.by_ref())?;
        let cpu_sub_type = u32::read_be(reader.by_ref())?;
        let size = u32::read_be(reader.by_ref())?;
        let checksum = u32::read_be(reader.by_ref())?;
        Ok(Self {
            cpu_type,
            cpu_sub_type,
            size,
            checksum,
        })
    }

    fn write_be<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        self.cpu_type.write_be(writer.by_ref())?;
        self.cpu_sub_type.write_be(writer.by_ref())?;
        self.size.write_be(writer.by_ref())?;
        self.checksum.write_be(writer.by_ref())?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(test, derive(arbitrary::Arbitrary))]
#[repr(u8)]
pub enum BinaryType {
    /// Regular file.
    Unknown = 0,
    /// Single-architecture executable file.
    Executable = 1,
    /// Multiple-architectures executable file (universal binary).
    Fat = 2,
}

fn get_binary_type(flags: u16) -> BinaryType {
    const UNKNOWN: u8 = BinaryType::Unknown as u8;
    const EXECUTABLE: u8 = BinaryType::Executable as u8;
    const FAT: u8 = BinaryType::Fat as u8;
    match ((flags >> 12) & 0xf) as u8 {
        UNKNOWN => BinaryType::Unknown,
        EXECUTABLE => BinaryType::Executable,
        FAT => BinaryType::Fat,
        _ => BinaryType::Unknown,
    }
}

const fn is_path_only(flags: u16) -> bool {
    (flags & 0xf) == 0
}

const fn bom_dev_to_libc_dev(dev: u32) -> libc::dev_t {
    let major = ((dev >> 24) & 0xff) as libc::c_uint;
    let minor = (dev & 0xff_ff_ff) as libc::c_uint;
    libc::makedev(major, minor)
}

fn libc_dev_to_bom_dev(dev: libc::dev_t) -> u32 {
    let major = unsafe { libc::major(dev) };
    let minor = unsafe { libc::minor(dev) };
    ((major & 0xff) << 24) as u32 | (minor & 0xff_ff_ff) as u32
}

#[cfg(test)]
mod tests {
    use arbitrary::Arbitrary;
    use arbitrary::Unstructured;
    use arbtest::arbtest;

    use super::*;
    use crate::test::block_io_symmetry_convert;
    use crate::test::test_be_io_symmetry;

    #[test]
    fn write_read_symmetry() {
        block_io_symmetry_convert::<Metadata32, Metadata>();
        test_be_io_symmetry::<ExeArch>();
    }

    #[test]
    fn bom_to_libc_symmetry() {
        arbtest(|u| {
            let expected_bom_dev: u32 = u.arbitrary()?;
            let libc_dev = bom_dev_to_libc_dev(expected_bom_dev);
            let actual_bom_dev = libc_dev_to_bom_dev(libc_dev);
            assert_eq!(expected_bom_dev, actual_bom_dev);
            Ok(())
        });
    }

    impl<'a> Arbitrary<'a> for Metadata {
        fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
            let extra: MetadataExtra = u.arbitrary()?;
            if matches!(extra, MetadataExtra::PathOnly { .. }) {
                Ok(Metadata {
                    mode: 0,
                    uid: 0,
                    gid: 0,
                    mtime: 0,
                    size: 0,
                    extra,
                })
            } else {
                let file_type = to_file_type(extra.entry_type());
                Ok(Self {
                    mode: u.int_in_range(0_u16..=0o7777_u16)? | file_type.to_mode_bits(),
                    uid: u.arbitrary()?,
                    gid: u.arbitrary()?,
                    mtime: u.arbitrary()?,
                    size: u.arbitrary()?,
                    extra,
                })
            }
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    struct Metadata32(Metadata);

    impl<'a> Arbitrary<'a> for Metadata32 {
        fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
            let extra: MetadataExtra = u.arbitrary()?;
            if matches!(extra, MetadataExtra::PathOnly { .. }) {
                // TODO merge metadata and metadata extra
                Ok(Self(Metadata {
                    mode: 0,
                    uid: 0,
                    gid: 0,
                    mtime: 0,
                    size: 0,
                    extra,
                }))
            } else {
                let file_type = to_file_type(extra.entry_type());
                Ok(Self(Metadata {
                    mode: u.int_in_range(0_u16..=0o7777_u16)? | file_type.to_mode_bits(),
                    uid: u.arbitrary()?,
                    gid: u.arbitrary()?,
                    mtime: u.arbitrary()?,
                    size: u.arbitrary::<u32>()? as u64,
                    extra,
                }))
            }
        }
    }

    impl From<Metadata32> for Metadata {
        fn from(other: Metadata32) -> Self {
            other.0
        }
    }

    impl<'a> Arbitrary<'a> for Executable {
        fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
            let mut arches = Vec::new();
            let num_arches = u.int_in_range(1..=0xf)?;
            for _ in 0..num_arches {
                arches.push(u.arbitrary()?);
            }
            Ok(Self {
                checksum: u.arbitrary()?,
                arches,
            })
        }
    }

    impl<'a> Arbitrary<'a> for Link {
        fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
            Ok(Self {
                checksum: u.arbitrary()?,
                name: OsStr::from_bytes(u.arbitrary::<CString>()?.to_bytes()).into(),
            })
        }
    }

    const fn to_file_type(entry_type: EntryType) -> FileType {
        use EntryType::*;
        match entry_type {
            File => FileType::Regular,
            Directory => FileType::Directory,
            Link => FileType::Symlink,
            Device => FileType::BlockDevice,
        }
    }
}
