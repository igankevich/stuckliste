# Receipt file format reference


## Named blocks

Receipt files store file system paths' metadata of installed packages
and are usually found in `/Library/Receipts` directory.
This is the most common sub-format: usually when people refer to BOM file they actually mean receipt files.

Receipt file contains the following named blocks.

| Name | Type | Explanation |
|------|------|-------------|
| `Paths` | [`Paths`](#paths) | A list of all path components and their metadata. |
| `BomInfo` | [`BomInfo`](#bom-info) | Per-architecture statistics of the files. |
| `HLIndex` | [`HardLinks`](#hard-links) | A list of all hard links. |
| `Size64` | [`FileSizes64`](#size64) | A list of all file sizes that are larger than 4 GiB. |
| `VIndex` | [`VirtualPaths`](#virtual-paths) | A list of paths defined by regular expressions. |


### <a name="paths"></a>Paths

Paths are stored as a tree with the following keys and values.
The information is stored for each component of the original path,
i.e. for all parent directories of the file.
This tree uses 4096 block size.

#### <a name="paths-key"></a>Paths key

| Field | Type | Explanation |
|-------|------|-------------|
| `seq_no` | `u32` | Entry's sequential number. Starts from 1. |
| `metadata` | `u32` | Block index of the path component's [metadata](#metadata). |

#### <a name="paths-value"></a>Paths value

| Field | Type | Explanation |
|-------|------|-------------|
| `parent` | `u32` | Sequential number of the parent directory. Zero means no parent. |
| `name` | `CStr` | Path component's name. |

### <a name="metadata"></a>Metadata

Metadata stores file's metadata, i.e. ownership, permissions etc.
The actual fields depend on the file and
the fact that this BOM file is path-only (i.e. produced by `mkbom -s`) or not.
If the entry is path-only there are no more fields other that specified in the following table.
For all other entries there are separate tables with the additional fields.

| Field | Type | Explanation | Value |
|-------|------|-------------|-------|
| `entry_type` | `u8` | Entry type. Can be one of the following: `File(1)`, `Directory(2)`, `Link(3)`, `Device(4)`. This type has to correspond to the file type stored in `mode`. | |
| `unknown` | `u8` | | 1 |
| `flags` | `u16` | The first four bits determine whether the entry is path-only (`0b0000`) or not (`0b1111`). The last four bits determine binary type: regular file (`0b0000`), regular executable (`0b0001`), universal binary (`0b0010`). If this entry is path-only, there are no more fields. | |

#### <a name="metadata-file"></a>File

| Field | Type | Explanation | Value |
|-------|------|-------------|-------|
| `mode` | `u16` | File mode. This includes file type bits that have to correspond to the entry type. |
| `uid` | `u32` | User id of the file's owner. |
| `gid` | `u32` | Group id of the file's owner. |
| `mtime` | `u32` | Last modification timestamp. |
| `size` | `u32` | File size. For files larger than 4 GiB the size is stored in [`FileSizes64`](#size64) and this field stores the overflown `u32` value. |
| `unknown` | `u8` | | 1 |
| `checksum` | `u32` | [CRC checksum](https://pubs.opengroup.org/onlinepubs/9699919799/utilities/cksum.html) of the file. Same as produced by MacOS's `cksum` command. |

#### <a name="metadata-directory"></a>Directory

| Field | Type | Explanation | Value |
|-------|------|-------------|-------|
| `mode` | `u16` | File mode. This includes file type bits that have to correspond to the entry type. |
| `uid` | `u32` | User id of the file's owner. |
| `gid` | `u32` | Group id of the file's owner. |
| `mtime` | `u32` | Last modification timestamp. |
| `size` | `u32` | File size. For files larger than 4 GiB the size is stored in [`FileSizes64`](#size64) and this field stores the overflown `u32` value. |
| `unknown` | `u8` | | 1 |

#### <a name="metadata-link"></a>Link

| Field | Type | Explanation | Value |
|-------|------|-------------|-------|
| `mode` | `u16` | File mode. This includes file type bits that have to correspond to the entry type. |
| `uid` | `u32` | User id of the file's owner. |
| `gid` | `u32` | Group id of the file's owner. |
| `mtime` | `u32` | Last modification timestamp. |
| `size` | `u32` | File size. For files larger than 4 GiB the size is stored in [`FileSizes64`](#size64) and this field stores the overflown `u32` value. |
| `unknown` | `u8` | | 1 |
| `checksum` | `u32` | [CRC checksum](https://pubs.opengroup.org/onlinepubs/9699919799/utilities/cksum.html) of the target's path without the nul byte. |
| `target_len` | `u32` | The length of the target path including the nul byte. |
| `target` | `CStr` | Nul-terminated target path. |

#### <a name="metadata-device"></a>Device

| Field | Type | Explanation | Value |
|-------|------|-------------|-------|
| `mode` | `u16` | File mode. This includes file type bits that have to correspond to the entry type. |
| `uid` | `u32` | User id of the file's owner. |
| `gid` | `u32` | Group id of the file's owner. |
| `mtime` | `u32` | Last modification timestamp. |
| `size` | `u32` | File size. For files larger than 4 GiB the size is stored in [`FileSizes64`](#size64) and this field stores the overflown `u32` value. |
| `unknown` | `u8` | | 1 |
| `dev` | `u32` | Device number. |
