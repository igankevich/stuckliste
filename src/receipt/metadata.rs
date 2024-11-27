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

    fn arch(&self) -> u16 {
        // arch 0xN00f
        // N - no. of architectures in a fat binary
        // f - always 0xf
        match self.extra {
            MetadataExtra::Executable { ref arches, .. } => {
                0x0f_u16 | (((arches.len() & 0xf) as u16) << 12)
            }
            _ => 0x0f_u16,
        }
    }

    pub(crate) fn accumulate(&self, stats: &mut BomInfo) {
        match self.extra {
            MetadataExtra::Executable { ref arches, .. } => {
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
        let _x0 = u8::read(reader.by_ref())?;
        debug_assert!(_x0 == 1, "x0 {:?}", _x0);
        let arch = u16::read(reader.by_ref())?;
        if arch != 15 {
            eprintln!("arch {:#x}", arch);
        }
        let num_arch = (arch >> 12) & 0xf;
        let mode = u16::read(reader.by_ref())?;
        let mode = mode & MODE_MASK;
        let uid = u32::read(reader.by_ref())?;
        let gid = u32::read(reader.by_ref())?;
        let mtime = u32::read(reader.by_ref())?;
        let size = u32::read(reader.by_ref())?;
        let _x1 = u8::read(reader.by_ref())?;
        debug_assert!(_x1 == 1, "x1 {:?}", _x1);
        let extra = match kind {
            FileType::File if num_arch != 0 => {
                let checksum = u32::read(reader.by_ref())?;
                let flag = u8::read(reader.by_ref())?;
                debug_assert!(flag == 1, "flag = {flag}");
                let num_arch_again = u32::read(reader.by_ref())?;
                debug_assert!(
                    num_arch_again.min(2) == num_arch as u32,
                    "num_arch = {num_arch}, num_arch_again = {num_arch_again}",
                );
                let mut arches = Vec::with_capacity(num_arch_again as usize);
                for _ in 0..num_arch_again {
                    arches.push(ExeArch::read(reader.by_ref())?);
                }
                MetadataExtra::Executable { checksum, arches }
            }
            FileType::File => {
                debug_assert!(num_arch == 0, "num_arch = {num_arch}");
                let checksum = u32::read(reader.by_ref())?;
                MetadataExtra::File { checksum }
            }
            FileType::Directory => {
                debug_assert!(num_arch == 0, "num_arch = {num_arch}");
                MetadataExtra::Directory
            }
            FileType::Link => {
                debug_assert!(num_arch == 0, "num_arch = {num_arch}");
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
                debug_assert!(num_arch == 0, "num_arch = {num_arch}");
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
        self.arch().write(writer.by_ref())?;
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
            MetadataExtra::Executable { checksum, arches } => {
                write_be(writer.by_ref(), *checksum)?;
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
    Executable { checksum: u32, arches: Vec<ExeArch> },
    Directory,
    Link { checksum: u32, name: CString },
    Device(Device),
}

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub struct Device {
    dev: u32,
}

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub struct ExeArch {
    cpu_type: u32,
    cpu_sub_type: u32,
    // TODO what if the size is u64?
    size: u32,
    checksum: u32,
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

impl Device {
    pub fn rdev(&self) -> u64 {
        bom_dev_to_libc_dev(self.dev)
    }
}

const fn bom_dev_to_libc_dev(dev: u32) -> libc::dev_t {
    let major = ((dev >> 24) & 0xff) as libc::c_uint;
    let minor = (dev & 0xff_ff_ff) as libc::c_uint;
    libc::makedev(major, minor)
}

fn libc_dev_to_bom_dev(dev: libc::dev_t) -> u32 {
    let major = unsafe { libc::major(dev) };
    let minor = unsafe { libc::minor(dev) };
    ((major << 24) & 0xff) as u32 | (minor & 0xff_ff_ff) as u32
}

const MODE_MASK: u16 = 0o7777;

#[cfg(test)]
mod tests {
    use arbitrary::Arbitrary;
    use arbitrary::Unstructured;

    use super::*;
    use crate::test::test_write_read_convert;

    #[test]
    fn write_read() {
        test_write_read_convert::<Metadata32, Metadata>();
    }

    impl<'a> Arbitrary<'a> for Metadata {
        fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
            Ok(Self {
                mode: u.arbitrary::<u16>()? & MODE_MASK,
                uid: u.arbitrary()?,
                gid: u.arbitrary()?,
                mtime: u.arbitrary()?,
                size: u.arbitrary()?,
                extra: u.arbitrary()?,
            })
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    struct Metadata32(Metadata);

    impl<'a> Arbitrary<'a> for Metadata32 {
        fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
            Ok(Self(Metadata {
                mode: u.arbitrary::<u16>()? & MODE_MASK,
                uid: u.arbitrary()?,
                gid: u.arbitrary()?,
                mtime: u.arbitrary()?,
                size: u.arbitrary::<u32>()? as u64,
                extra: u.arbitrary()?,
            }))
        }
    }

    impl From<Metadata32> for Metadata {
        fn from(other: Metadata32) -> Self {
            other.0
        }
    }
}
