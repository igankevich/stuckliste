use std::ffi::CString;
use std::io::Error;
use std::io::Read;
use std::io::Write;

use crate::io::*;
use crate::receipt::BomInfo;
use crate::BigEndianIo;
use crate::FileType;

/*
Device len 35
Directory len 31
File len 35
Link len 45


01, // is executable flag?
00, 00, 00, 02, // num arch
01, 00, 00, 07, // cpu_type_t
00, 00, 00, 03, // cpu_subtype_t
00, 00, df, 50, // offset
f1, 7d, 04, dd, // checksum
01, 00, 00, 0c, // cpu_type_t
80, 00, 00, 02, // cpu_subtype_t
00, 00, de, 80, // offset
5d, 06, d1, ec, // checksum
00, 00, 00, 00, 00, 00, 00, 00 // trailer



*/
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
        use MetadataExtra::*;
        match self.extra {
            File { .. } | Executable { .. } => FileType::File,
            Directory { .. } => FileType::Directory,
            Link { .. } => FileType::Link,
            Device { .. } => FileType::Device,
            PathOnly { file_type } => file_type,
        }
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
}

impl BigEndianIo for Metadata {
    fn read<R: Read>(mut reader: R) -> Result<Self, Error> {
        let kind: FileType = u8::read(reader.by_ref())?.try_into()?;
        eprintln!("kind {:?}", kind);
        let _x0 = u8::read(reader.by_ref())?;
        debug_assert!(_x0 == 1, "x0 {:?}", _x0);
        let flags = u16::read(reader.by_ref())?;
        if is_path_only(flags) {
            // This BOM stores paths only.
            let metadata = Self {
                mode: 0,
                uid: 0,
                gid: 0,
                mtime: 0,
                size: 0,
                extra: MetadataExtra::PathOnly { file_type: kind },
            };
            return Ok(metadata);
        }
        let binary_type = get_binary_type(flags);
        let mode = u16::read(reader.by_ref())?;
        let mode = mode & MODE_MASK;
        let uid = u32::read(reader.by_ref())?;
        let gid = u32::read(reader.by_ref())?;
        let mtime = u32::read(reader.by_ref())?;
        let size = u32::read(reader.by_ref())?;
        let _x1 = u8::read(reader.by_ref())?;
        debug_assert!(_x1 == 1, "x1 {:?}", _x1);
        let extra = match kind {
            FileType::File if binary_type != BinaryType::Unknown => {
                let checksum = u32::read(reader.by_ref())?;
                let flag = u8::read(reader.by_ref())?;
                debug_assert!(flag == 1, "flag = {flag}");
                let num_arch_again = u32::read(reader.by_ref())?;
                let mut arches = Vec::with_capacity(num_arch_again as usize);
                for _ in 0..num_arch_again {
                    arches.push(ExeArch::read(reader.by_ref())?);
                }
                MetadataExtra::Executable(Executable { checksum, arches })
            }
            FileType::File => {
                debug_assert!(
                    binary_type == BinaryType::Unknown,
                    "unexpected binary type {:?}",
                    binary_type
                );
                let checksum = u32::read(reader.by_ref())?;
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
            FileType::Link => {
                debug_assert!(
                    binary_type == BinaryType::Unknown,
                    "unexpected binary type {:?}",
                    binary_type
                );
                let checksum = u32::read(reader.by_ref())?;
                let name_len = u32::read(reader.by_ref())?;
                debug_assert!(
                    name_len == 0 && kind != FileType::Link || kind == FileType::Link,
                    "kind = {:?}, name_len = {}",
                    kind,
                    name_len
                );
                let mut name = vec![0_u8; name_len as usize];
                reader.read_exact(&mut name[..])?;
                let name = CString::from_vec_with_nul(name).map_err(Error::other)?;
                MetadataExtra::Link { checksum, name }
            }
            FileType::Device => {
                debug_assert!(
                    binary_type == BinaryType::Unknown,
                    "unexpected binary type {:?}",
                    binary_type
                );
                let dev = u32::read(reader.by_ref())?;
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
        // We ignore 8 zero bytes here.
        let trailer = u64::read(reader.by_ref())?;
        debug_assert!(trailer == 0, "trailer = {trailer}, metadata = {metadata:?}");
        Ok(metadata)
    }

    fn write<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        (self.file_type() as u8).write(writer.by_ref())?;
        1_u8.write(writer.by_ref())?;
        let flags = self.flags();
        flags.write(writer.by_ref())?;
        if is_path_only(flags) {
            return Ok(());
        }
        (self.mode & MODE_MASK).write(writer.by_ref())?;
        write_be(writer.by_ref(), self.uid)?;
        write_be(writer.by_ref(), self.gid)?;
        write_be(writer.by_ref(), self.mtime)?;
        write_be(writer.by_ref(), self.size as u32)?; // truncate the size
        1_u8.write(writer.by_ref())?;
        match &self.extra {
            MetadataExtra::File { checksum } => {
                write_be(writer.by_ref(), *checksum)?;
            }
            MetadataExtra::Executable(Executable { checksum, arches }) => {
                write_be(writer.by_ref(), *checksum)?;
                1_u8.write(writer.by_ref())?;
                let num_arches = arches.len() as u32;
                num_arches.write(writer.by_ref())?;
                for arch in arches.iter() {
                    arch.write(writer.by_ref())?;
                }
            }
            MetadataExtra::Directory => {}
            MetadataExtra::Link { checksum, name } => {
                write_be(writer.by_ref(), *checksum)?;
                let name_bytes = name.as_bytes_with_nul();
                write_be(writer.by_ref(), name_bytes.len() as u32)?;
                writer.write_all(name_bytes)?;
            }
            MetadataExtra::Device(Device { dev }) => {
                write_be(writer.by_ref(), *dev)?;
            }
            MetadataExtra::PathOnly { .. } => {}
        }
        // Block always ends with 8 zeroes.
        writer.write_all(&[0_u8; 8])?;
        Ok(())
    }
}

impl TryFrom<std::fs::Metadata> for Metadata {
    type Error = Error;
    fn try_from(other: std::fs::Metadata) -> Result<Self, Self::Error> {
        use std::os::unix::fs::MetadataExt;
        let kind: FileType = other.file_type().try_into()?;
        let extra = match kind {
            FileType::File => MetadataExtra::File { checksum: 0 },
            FileType::Directory => MetadataExtra::Directory,
            FileType::Link => MetadataExtra::Link {
                checksum: 0,
                name: Default::default(),
            },
            FileType::Device => MetadataExtra::Device(Device {
                dev: libc_dev_to_bom_dev(other.rdev()),
            }),
        };
        Ok(Self {
            mode: (other.mode() & 0o7777) as u16,
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
    Link { checksum: u32, name: CString },
    Device(Device),
    PathOnly { file_type: FileType },
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
    dev: u32,
}

impl Device {
    pub fn rdev(&self) -> u64 {
        bom_dev_to_libc_dev(self.dev)
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub struct ExeArch {
    pub cpu_type: u32,
    pub cpu_sub_type: u32,
    // TODO what if the size is u64?
    pub size: u32,
    pub checksum: u32,
}

impl BigEndianIo for ExeArch {
    fn read<R: Read>(mut reader: R) -> Result<Self, Error> {
        let cpu_type = u32::read(reader.by_ref())?;
        let cpu_sub_type = u32::read(reader.by_ref())?;
        let size = u32::read(reader.by_ref())?;
        let checksum = u32::read(reader.by_ref())?;
        Ok(Self {
            cpu_type,
            cpu_sub_type,
            size,
            checksum,
        })
    }

    fn write<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        write_be(writer.by_ref(), self.cpu_type)?;
        write_be(writer.by_ref(), self.cpu_sub_type)?;
        write_be(writer.by_ref(), self.size)?;
        write_be(writer.by_ref(), self.checksum)?;
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
        other => {
            eprintln!("unknown binary type: {}", other);
            BinaryType::Unknown
        }
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

const MODE_MASK: u16 = 0o7777;

#[cfg(test)]
mod tests {
    use arbitrary::Arbitrary;
    use arbitrary::Unstructured;
    use arbtest::arbtest;

    use super::*;
    use crate::test::test_write_read;
    use crate::test::test_write_read_convert;

    #[test]
    fn write_read_symmetry() {
        test_write_read_convert::<Metadata32, Metadata>();
        test_write_read::<ExeArch>();
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
                Ok(Self {
                    mode: u.arbitrary::<u16>()? & MODE_MASK,
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
                Ok(Self(Metadata {
                    mode: 0,
                    uid: 0,
                    gid: 0,
                    mtime: 0,
                    size: 0,
                    extra,
                }))
            } else {
                Ok(Self(Metadata {
                    mode: u.arbitrary::<u16>()? & MODE_MASK,
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
            Ok(Executable {
                checksum: u.arbitrary()?,
                arches,
            })
        }
    }
}
