use std::ffi::CString;
use std::ffi::OsStr;
use std::fs::read_link;
use std::io::Cursor;
use std::io::Error;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Seek;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::SystemTime;

use crate::receipt::BomInfo;
use crate::receipt::Context;
use crate::receipt::CrcReader;
use crate::receipt::EntryType;
use crate::receipt::FileType;
use crate::BigEndianIo;
use crate::BlockIo;
use crate::Blocks;

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub enum Metadata {
    File(File),
    Executable(Executable),
    Directory(Directory),
    Link(Link),
    Device(Device),
    Entry(Entry),
}

impl Metadata {
    pub fn file_type(&self) -> FileType {
        // TODO error ???
        FileType::new(self.mode()).unwrap_or(FileType::Regular)
    }

    pub fn entry_type(&self) -> EntryType {
        use Metadata::*;
        match self {
            File(..) | Executable(..) => EntryType::File,
            Link { .. } => EntryType::Link,
            Directory(..) => EntryType::Directory,
            Device(..) => EntryType::Device,
            Entry(self::Entry { entry_type }) => *entry_type,
        }
    }

    pub fn mode(&self) -> u16 {
        get_common_field!(self, mode, 0)
    }

    pub fn uid(&self) -> u32 {
        get_common_field!(self, uid, 0)
    }

    pub fn gid(&self) -> u32 {
        get_common_field!(self, gid, 0)
    }

    pub fn mtime(&self) -> u32 {
        get_common_field!(self, mtime, 0)
    }

    pub fn size(&self) -> u64 {
        get_common_field!(self, size, 0)
    }

    fn set_size(&mut self, value: u64) {
        set_common_field!(self, size, value);
    }

    /// Last modification time.
    pub fn modified(&self) -> Result<SystemTime, Error> {
        let dt = Duration::from_secs(self.mtime().into());
        SystemTime::UNIX_EPOCH
            .checked_add(dt)
            .ok_or_else(|| Error::new(ErrorKind::InvalidData, "out of range timestamp"))
    }

    pub fn checksum(&self) -> u32 {
        match self {
            Metadata::File(File { checksum, .. }) => *checksum,
            Metadata::Executable(Executable { checksum, .. }) => *checksum,
            Metadata::Directory(..) => 0,
            Metadata::Link(Link { checksum, .. }) => *checksum,
            Metadata::Device(Device { .. }) => 0,
            Metadata::Entry { .. } => 0,
        }
    }

    pub fn from_path(path: &Path, path_only: bool) -> Result<Self, Error> {
        let metadata = std::fs::symlink_metadata(path)?;
        if path_only {
            return Ok(Self::Entry(Entry {
                entry_type: metadata.file_type().try_into()?,
            }));
        }
        let mut metadata: Metadata = metadata.try_into()?;
        match metadata {
            Metadata::File(File {
                ref mut checksum, ..
            }) => {
                let crc_reader = CrcReader::new(std::fs::File::open(path)?);
                *checksum = crc_reader.digest()?;
            }
            Metadata::Link(Link {
                ref mut name,
                ref mut checksum,
                ..
            }) => {
                *name = read_link(path)?;
                let crc_reader = CrcReader::new(name.as_os_str().as_bytes());
                *checksum = crc_reader.digest()?;
            }
            _ => {}
        }
        Ok(metadata)
    }

    fn flags(&self) -> u16 {
        // flags 0xN00P
        // N - no. of architectures in a fat binary
        // P - 0xf for regular bom, 0 for path-only bom
        let path_only = match self {
            Metadata::Entry { .. } => 0_u16,
            _ => 0xf_u16,
        };
        let binary_type = match self {
            Metadata::Executable(Executable { ref arches, .. }) => {
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
        match self {
            Metadata::Executable(Executable { ref arches, .. }) => {
                for arch in arches.iter() {
                    stats.accumulate(arch.cpu_type, arch.size);
                }
            }
            // BomInfo wraps around file size if it's larger than u32::MAX
            _ => stats.accumulate(0, self.size() as u32),
        }
    }

    fn read<R: Read>(mut reader: R) -> Result<Self, Error> {
        let entry_type = EntryType::read_be(reader.by_ref())?;
        let _x0 = u8::read_be(reader.by_ref())?;
        debug_assert!(_x0 == 1, "x0 {:?}", _x0);
        let flags = u16::read_be(reader.by_ref())?;
        if is_path_only(flags) {
            // This BOM stores paths only.
            let metadata = Self::Entry(Entry { entry_type });
            return Ok(metadata);
        }
        let binary_type = get_binary_type(flags);
        let common = Common::read_be(reader.by_ref())?;
        let file_type = FileType::new(common.mode)?;
        debug_assert!(
            file_type.to_entry_type() == entry_type,
            "entry_type = {:?}, file_type = {:?}",
            entry_type,
            file_type
        );
        let metadata = match file_type {
            FileType::Regular if binary_type != BinaryType::Unknown => {
                let checksum = u32::read_be(reader.by_ref())?;
                let flag = u8::read_be(reader.by_ref())?;
                debug_assert!(flag == 1, "flag = {flag}");
                let num_arch_again = u32::read_be(reader.by_ref())?;
                let mut arches = Vec::with_capacity(num_arch_again as usize);
                for _ in 0..num_arch_again {
                    arches.push(ExecutableArch::read_be(reader.by_ref())?);
                }
                Metadata::Executable(Executable {
                    common,
                    checksum,
                    arches,
                })
            }
            FileType::Regular => {
                debug_assert!(
                    binary_type == BinaryType::Unknown,
                    "unexpected binary type {:?}",
                    binary_type
                );
                let checksum = u32::read_be(reader.by_ref())?;
                Metadata::File(File { common, checksum })
            }
            FileType::Directory => {
                debug_assert!(
                    binary_type == BinaryType::Unknown,
                    "unexpected binary type {:?}",
                    binary_type
                );
                Metadata::Directory(Directory { common })
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
                Metadata::Link(Link {
                    common,
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
                Metadata::Device(Device {
                    common,
                    dev: dev as i32,
                })
            }
        };
        // We ignore 8 zero bytes here. Bomutils' `mkbom` doesn't write them but the original `mkbom` does.
        Ok(metadata)
    }

    fn write<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        self.entry_type().write_be(writer.by_ref())?;
        1_u8.write_be(writer.by_ref())?;
        let flags = self.flags();
        flags.write_be(writer.by_ref())?;
        match self {
            Metadata::File(File { common, checksum }) => {
                common.write_be(writer.by_ref())?;
                checksum.write_be(writer.by_ref())?;
            }
            Metadata::Executable(Executable {
                common,
                checksum,
                arches,
            }) => {
                common.write_be(writer.by_ref())?;
                checksum.write_be(writer.by_ref())?;
                1_u8.write_be(writer.by_ref())?;
                let num_arches = arches.len() as u32;
                num_arches.write_be(writer.by_ref())?;
                for arch in arches.iter() {
                    arch.write_be(writer.by_ref())?;
                }
            }
            Metadata::Directory(Directory { common }) => {
                common.write_be(writer.by_ref())?;
            }
            Metadata::Link(Link {
                common,
                checksum,
                name,
            }) => {
                common.write_be(writer.by_ref())?;
                checksum.write_be(writer.by_ref())?;
                let name_bytes = name.as_os_str().as_bytes();
                // +1 because of the nul byte
                ((name_bytes.len() + 1) as u32).write_be(writer.by_ref())?;
                writer.write_all(name_bytes)?;
                writer.write_all(&[0_u8])?;
            }
            Metadata::Device(Device { common, dev }) => {
                common.write_be(writer.by_ref())?;
                (*dev as u32).write_be(writer.by_ref())?;
            }
            Metadata::Entry(..) => {}
        }
        if !matches!(self, Metadata::Entry(..)) {
            // Block always ends with 8 zeroes.
            writer.write_all(&[0_u8; 8])?;
        }
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
            metadata.set_size(*size);
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
        let common = Common {
            mode: other.mode() as u16,
            uid: other.uid(),
            gid: other.gid(),
            mtime: other.mtime().try_into().unwrap_or(0),
            size: other.size(),
        };
        let metadata = match kind {
            FileType::Regular => Metadata::File(File {
                common,
                checksum: 0,
            }),
            FileType::Directory => Metadata::Directory(Directory { common }),
            FileType::Symlink => Metadata::Link(Link {
                common,
                checksum: 0,
                name: Default::default(),
            }),
            FileType::CharDevice | FileType::BlockDevice => Metadata::Device(Device {
                common,
                dev: other.rdev() as i32,
            }),
        };
        Ok(metadata)
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct File {
    common: Common,
    checksum: u32,
}

impl File {
    pub fn checksum(&self) -> u32 {
        self.checksum
    }
}

impl_common!(File);

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct Executable {
    common: Common,
    checksum: u32,
    arches: Vec<ExecutableArch>,
}

impl Executable {
    pub fn checksum(&self) -> u32 {
        self.checksum
    }

    pub fn arches(&self) -> &[ExecutableArch] {
        &self.arches[..]
    }

    pub fn into_arches(self) -> Vec<ExecutableArch> {
        self.arches
    }
}

impl_common!(Executable);

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct Directory {
    common: Common,
}

impl_common!(Directory);

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct Link {
    common: Common,
    checksum: u32,
    name: PathBuf,
}

impl Link {
    pub fn checksum(&self) -> u32 {
        self.checksum
    }

    pub fn name(&self) -> &Path {
        &self.name
    }

    pub fn into_name(self) -> PathBuf {
        self.name
    }
}

impl_common!(Link);

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct Device {
    common: Common,
    dev: i32,
}

impl Device {
    pub fn rdev(&self) -> i32 {
        self.dev
    }
}

impl_common!(Device);

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub struct Entry {
    entry_type: EntryType,
}

impl Entry {
    pub fn kind(&self) -> EntryType {
        self.entry_type
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub struct ExecutableArch {
    cpu_type: u32,
    cpu_sub_type: u32,
    // If the actual binary size is u64 then this field overflows.
    size: u32,
    checksum: u32,
}

impl ExecutableArch {
    pub fn cpu_type(&self) -> u32 {
        self.cpu_type
    }

    pub fn cpu_sub_type(&self) -> u32 {
        self.cpu_sub_type
    }

    pub fn size(&self) -> u32 {
        self.size
    }

    pub fn checksum(&self) -> u32 {
        self.checksum
    }
}

impl BigEndianIo for ExecutableArch {
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

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
struct Common {
    mode: u16,
    uid: u32,
    gid: u32,
    mtime: u32,
    size: u64,
}

impl BigEndianIo for Common {
    fn read_be<R: Read>(mut reader: R) -> Result<Self, Error> {
        let mode = u16::read_be(reader.by_ref())?;
        let uid = u32::read_be(reader.by_ref())?;
        let gid = u32::read_be(reader.by_ref())?;
        let mtime = u32::read_be(reader.by_ref())?;
        let size = u32::read_be(reader.by_ref())?;
        let _x1 = u8::read_be(reader.by_ref())?;
        debug_assert!(_x1 == 1, "x1 {:?}", _x1);
        Ok(Self {
            mode,
            uid,
            gid,
            mtime,
            size: size as u64,
        })
    }

    fn write_be<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        self.mode.write_be(writer.by_ref())?;
        self.uid.write_be(writer.by_ref())?;
        self.gid.write_be(writer.by_ref())?;
        self.mtime.write_be(writer.by_ref())?;
        (self.size as u32).write_be(writer.by_ref())?; // truncate the size
        1_u8.write_be(writer.by_ref())?;
        Ok(())
    }
}

const fn is_path_only(flags: u16) -> bool {
    (flags & 0xf) == 0
}

macro_rules! get_common_field {
    ($self:ident, $field:ident, $default:expr) => {{
        use Metadata::*;
        match $self {
            File(x) => x.common.$field,
            Directory(x) => x.common.$field,
            Executable(x) => x.common.$field,
            Link(x) => x.common.$field,
            Device(x) => x.common.$field,
            Entry(_) => $default,
        }
    }};
}

use get_common_field;

macro_rules! set_common_field {
    ($self:ident, $field:ident, $value:expr) => {{
        use Metadata::*;
        match $self {
            File(x) => x.common.$field = $value,
            Directory(x) => x.common.$field = $value,
            Executable(x) => x.common.$field = $value,
            Link(x) => x.common.$field = $value,
            Device(x) => x.common.$field = $value,
            Entry(_) => {}
        }
    }};
}

use set_common_field;

macro_rules! impl_common {
    ($type:ty) => {
        impl $type {
            pub fn mode(&self) -> u16 {
                self.common.mode
            }

            pub fn uid(&self) -> u32 {
                self.common.uid
            }

            pub fn gid(&self) -> u32 {
                self.common.gid
            }

            pub fn mtime(&self) -> u32 {
                self.common.mtime
            }

            pub fn size(&self) -> u64 {
                self.common.size
            }
        }
    };
}

use impl_common;

#[cfg(test)]
mod tests {
    use arbitrary::Arbitrary;
    use arbitrary::Unstructured;

    use super::*;
    use crate::test::block_io_symmetry_convert;
    use crate::test::test_be_io_symmetry;

    #[test]
    fn write_read_symmetry() {
        block_io_symmetry_convert::<Metadata32, Metadata>();
        test_be_io_symmetry::<ExecutableArch>();
    }

    //impl<'a> Arbitrary<'a> for Metadata {
    //    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
    //        let mut metadata: Metadata = u.arbitrary()?;
    //        // make file mode correspond to entry type
    //        let file_type = to_file_type(metadata.entry_type());
    //        metadata.set_mode(u.int_in_range(0_u16..=0o7777_u16)? | file_type.to_mode_bits());
    //        Ok(metadata)
    //    }
    //}

    impl<'a> Arbitrary<'a> for File {
        fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
            let mut common: Common = u.arbitrary()?;
            common.mode = FileType::Regular.set(common.mode);
            Ok(File {
                common,
                checksum: u.arbitrary()?,
            })
        }
    }

    impl<'a> Arbitrary<'a> for Executable {
        fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
            let mut common: Common = u.arbitrary()?;
            common.mode = FileType::Regular.set(common.mode);
            let mut arches = Vec::new();
            let num_arches = u.int_in_range(1..=0xf)?;
            for _ in 0..num_arches {
                arches.push(u.arbitrary()?);
            }
            Ok(Self {
                common,
                checksum: u.arbitrary()?,
                arches,
            })
        }
    }

    impl<'a> Arbitrary<'a> for Directory {
        fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
            let mut common: Common = u.arbitrary()?;
            common.mode = FileType::Directory.set(common.mode);
            Ok(Directory { common })
        }
    }

    impl<'a> Arbitrary<'a> for Device {
        fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
            let mut common: Common = u.arbitrary()?;
            common.mode = FileType::CharDevice.set(common.mode);
            Ok(Self {
                common,
                dev: u.arbitrary()?,
            })
        }
    }

    impl<'a> Arbitrary<'a> for Link {
        fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
            let mut common: Common = u.arbitrary()?;
            common.mode = FileType::Symlink.set(common.mode);
            Ok(Self {
                common,
                checksum: u.arbitrary()?,
                name: OsStr::from_bytes(u.arbitrary::<CString>()?.to_bytes()).into(),
            })
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    struct Metadata32(Metadata);

    impl<'a> Arbitrary<'a> for Metadata32 {
        fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
            let mut metadata: Metadata = u.arbitrary()?;
            // enforce 32-bit file size
            metadata.set_size(u.arbitrary::<u32>()? as u64);
            Ok(Self(metadata))
        }
    }

    impl From<Metadata32> for Metadata {
        fn from(other: Metadata32) -> Self {
            other.0
        }
    }
}
