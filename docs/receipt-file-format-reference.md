# Receipt file format reference


## Named blocks

Receipt files store file system paths' metadata of installed packages
and are usually found in `/Library/Receipts` directory.
This is the most common sub-format: usually when people refer to BOM file they actually mean receipt files.

Receipt file contains the following named blocks.

| Name | Type | Explanation |
|------|------|-------------|
| `Paths` | [`Paths`](#paths) | A list of all path components and their metadata. |
| `Size64` | [`FileSizes64`](#size64) | A list of all file sizes that are larger than 4 GiB. |
| `HLIndex` | [`HardLinks`](#hard-links) | A list of all hard links. |
| `BomInfo` | [`BomInfo`](#bom-info) | Per-architecture statistics of the files. |
| `VIndex` | [`VirtualPaths`](#virtual-paths) | A list of paths defined by regular expressions. |


### <a name="paths"></a>Paths

Paths are stored as a tree with the following keys and values.
The information is stored for each component of the original path,
i.e. for all parent directories of the file.
This tree uses 4096-byte blocks.

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
| `zeroes` | `[u8; 8]` | The block ends with zeroes. |

#### <a name="metadata-directory"></a>Directory

| Field | Type | Explanation | Value |
|-------|------|-------------|-------|
| `mode` | `u16` | File mode. This includes file type bits that have to correspond to the entry type. |
| `uid` | `u32` | User id of the file's owner. |
| `gid` | `u32` | Group id of the file's owner. |
| `mtime` | `u32` | Last modification timestamp. |
| `size` | `u32` | File size. For files larger than 4 GiB the size is stored in [`FileSizes64`](#size64) and this field stores the overflown `u32` value. |
| `unknown` | `u8` | | 1 |
| `zeroes` | `[u8; 8]` | The block ends with zeroes. |

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
| `zeroes` | `[u8; 8]` | The block ends with zeroes. |

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
| `zeroes` | `[u8; 8]` | The block ends with zeroes. |


### <a name="size64"></a>FileSizes64

64-bit file sizes are stored as a tree with file size (`u64`) as the key and metadata block index as the value (`u32`).
This is an example of a tree with keys and values logically swapped.
The sane way of storing such data in a program is `HashMap<u32, u64>` with metadata block index as the key.
The information is stored only for those files that are larger than 4 GiB,
i.e. their size does not fit into 32-bit integer.
This tree uses 128-byte blocks.


### <a name="hard-links"></a>HardLinks

Hard links are stored as a tree with a [pointer](#pointer) to a tree with [file paths tree](#file-paths-tree) as the key and
metadata block index `u32` as the value.
It is unclear why the pointer is used.
The sane way of storing such data in a program is `HashMap<u32, Vec<CString>>` with metadata block index as the key.
The hard links tree uses 4096-byte blocks, whereas file paths tree uses 128-byte blocks.

#### <a name="file-paths-tree"></a>FilePathsTree

This tree stores file paths as `CStr` values, the keys are empty blocks.
Such blocks are considered occupied, i.e. have non-zero offsets but zero sizes.


### <a name="pointer"></a>Pointer

A pointer is a block that stores the index of some other block as `u32`.
It is unclear whether a pointer can store index `0` or not and whehter it can point to a zero-sized block or not.


### <a name="bom-info"></a>BomInfo

| Field | Type | Explanation | Value |
|-------|------|-------------|-------|
| `version` | `u32` | Entity version. | 1 |
| `num_paths` | `u32` | Total no. of paths in the receipt. | |
| `num_entries` | `u32` | No. of entries in `BomInfo` | |
| `entry[0]` | [`BomInfoEntry`](#bom-info-entry) | The first entry. | |
| ... | | | |
| `entry[num_entries-1]` | [`BomInfoEntry`](#bom-info-entry) | The last entry. | |

#### <a name="bom-info-entry"></a>BomInfoEntry

| Field | Type | Explanation | Value |
|-------|------|-------------|-------|
| `cpu_type` | `u32` | CPU type as defined in [`mach/machine.h`](https://github.com/opensource-apple/cctools/blob/master/include/mach/machine.h). Equals 0 for non-executable files. | |
| `unknown` | `u32` | | 0 |
| `total_size` | `u32` | Total size of executables for the specified `cpu_type`. For universal binaries (fat binaries) this includes only the size of the portion of the file for `cpu_type`. For regular files (`cpu_type == 0`) the whole file size is included. | |
| `unknown` | `u32` | | 0 |


### <a name="virtual-paths"></a>VirtualPaths

| Field | Type | Explanation | Value |
|-------|------|-------------|-------|
| `version` | `u32` | Entity version. | 1 |
| `tree` | `u32` | Block index of the [virtual paths tree](#virtual-paths-tree). | |
| `unknown` | `u32` | Equals 0 for empty tree, non-zero otherwise. | |
| `unknown` | `u8` | Equals 1 for empty tree. | |

#### <a name="virtual-paths-tree"></a>VirtualPathsTree

This tree uses an optional [list of regular expressions](#reg-exp-tree) as the key and an XML path as the value (`CStr`).
Optional means that the referenced block either contains the tree or is empty.
The regular expressions look like `LANGUAGE\\.lproj$` and paths look like `"lang/LANGUAGE/xtra/path"`.
For paths like `"lang/LANGUAGE/xtra/size"` the list of regular expressions is empty.
The sane way of storing such data in a program is probably `HashMap<CString, Vec<CString>>` with an XML path as the key and a list of regular expressions as the value.
Both trees use 128-byte blocks.

The purpose of this tree is largerly unknown, but is probably related to CF bundles and property lists.

#### <a name="reg-exp-tree"></a>RegExpTree

This tree stores regular expressions as the values and the keys are empty.
