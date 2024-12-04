# BOM file format reference


## Regular blocks, named blocks and file header

Bill-of-materials (BOM) files store arbitrary data in regular and named blocks.
A _regular block_ is characterized by its offset from the start of the file and its size in bytes.
A _named block_ is a regular block that is also characterized by its human-readable name.
BOM file starts with the header which is padded to 512 bytes.
The header includes magic bytes and block specification (offset and size) for regular blocks and named blocks —
special blocks that store information on regular and named blocks respectively.
Usually these special blocks are located at the end of the file to simplify updates.

The unsigned integers in the files are stored in big-endian order,
strings are stored as null-terminated C-strings, and byte arrays are stored as is.

The following tables summarize the internal structure of the aforementioned entities.

### <a name="header"></a>File header

| Field | Type | Explanation | Value |
|------------|------|-------|-------------|
| `magic` | `[u8; 8]` | File signature. | `"BOMStore"` |
| `version` | `u32` | Entity version. | 1 |
| `num_non_null_blocks` | `u32` | No. of unoccupied blocks. |
| `regular_blocks` | [`Block`](#block) | Block that stores information on [regular blocks](#regular-blocks). |
| `named_blocks` | [`Block`](#block) | Block that stores information on [named blocks](#named-blocks). |

### <a name="block"></a>Block

| Field | Type | Explanation |
|------------|------|-------------|
| `offset` | `u32` | Block offset from the start of the file. | 
| `size` | `u32` | Block size. | 

### <a name="regular-blocks"></a>Regular blocks

The first block is always null, i.e. has zero offset and zero size to be able to use block index 0 as a "null" value. This block is nevertheless included in `num_occupied_blocks`.

| Field | Type | Explanation |
|------------|------|-------------|
| `num_occupied_blocks` | `u32` | No. of occupied regular blocks, i.e. blocks with non-zero length that store some other entity. |
| `occupied_block[0]` | [`Block`](#block) | Null block. |
| `occupied_block[1]` | [`Block`](#block) | First occupied block. |
| ... | | |
| `occupied_block[num_occupied_blocks-1]` | [`Block`](#block) | Last occupied block. |
| `num_free_blocks` | `u32` | No. of free regular blocks, i.e. blocks with zero length that can be used to store new entities on file update. |
| `free_block[0]` | [`Block`](#block) | First free block. |
| `free_block[1]` | [`Block`](#block) | Second free block. |
| ... | | |
| `free_block[num_free_blocks - 1]` | [`Block`](#block) | Last free block. |

### <a name="named-blocks"></a>Named blocks

Block index here and everywhere else means the index in the list of [regular blocks](#regular-blocks).

| Field | Type | Explanation |
|------------|------|-------------|
| `num_named_blocks` | `u32` | No. of named blocks. |
| `name[0]` | `CStr` | Null-terminated first block name. |
| `index[0]` | `u32` | First block index. |
| `name[1]` | `CStr` | Null-terminated second block name. |
| `index[1]` | `u32` | Second block index. |
| ... | | |
| `name[num_named_blocks-1]` | `CStr` | Null-terminated last block name. |
| `index[num_named_blocks-1]` | `u32` | Last block index. |


## Trees

The information in the blocks can be stored in any format,
but usually it is stored as key/value tables called _trees_.
A tree is itself stored in several blocks depending on how many entries it contains.
There are _data nodes_ that store key/value block indices and
_meta nodes_ that store block indices that point to data nodes.
Probably such a hierarchical structure was the reason for calling them _trees_.

The notion of keys and values is blurred in the trees.
Some trees have all keys point to zero-length blocks, others have duplicate keys.
Logically both keys and values might be used to store a unique key.

Meta node is not required if the tree fits into one data node page.
Given that the block size of the tree can be any value,
the meta nodes can be omitted completely.
Probably meta nodes and data nodes were used as an efficient way of updating the tree in-place.

The following tables summarize the internal structure of the trees.

### <a name="tree"></a>Tree

| Field | Type | Explanation | Value |
|------------|------|-------|-------------|
| `magic` | `[u8; 4]` | Tree signature. | `"tree"` |
| `version` | `u32` | Entity version. | 1 |
| `root` | `u32` | Block index of the root node. Can point to either meta or data node. | |
| `block_size` | `u32` | Block size that is used to allocate tree nodes. Usually it is 4096 for large trees and 128 for small ones. | |
| `num_entries` | `u32` | Total no. of key/value entries in the tree. | |
| `unknown` | `u8` | | 0 |

### <a name="tree-data-node"></a>Data node

| Field | Type | Explanation |
|------------|------|-------|
| `flags` | `u16` | Equal 1 if it is a data node, 0 otherwise. | 1 |
| `num_entries` | `u16` | No. of entries in this particular tree node. |
| `next` | `u32` | Block index of the next data node. |
| `prev` | `u32` | Block index of the previous data node. |
| `key[0]` | `u32` | Block index of the first entry's key. |
| `value[0]` | `u32` | Block index of the first entry's value. |
| ... | | |
| `key[num_entries-1]` | `u32` | Block index of the last entry's key. |
| `value[num_entries-1]` | `u32` | Block index of the last entry's value. |

### <a name="tree-meta-node"></a>Meta node

In this node keys always point to data nodes and
values point to the last value in the corresponding data node.
It is unclear how the values are used.

| Field | Type | Explanation |
|------------|------|-------|
| `flags` | `u16` | Equal 1 if it is a data node, 0 otherwise. | 0 |
| `num_entries` | `u16` | No. of entries in this particular tree node. |
| `next` | `u32` | Block index of the next meta node. |
| `prev` | `u32` | Block index of the previous meta node. |
| `key[0]` | `u32` | Block index of the first entry's key. |
| `value[0]` | `u32` | Block index of the first entry's value. |
| ... | | |
| `key[num_entries-1]` | `u32` | Block index of the last entry's key. |
| `value[num_entries-1]` | `u32` | Block index of the last entry's value. |


## BOM-based formats

BOM file format is used as a base for other application-specific formats.
The most common among them are _receipt_ and _CAR_ formats.
There are probably others, and it is easy to create your own custom format based on BOM.

Receipt files store file system paths' metadata of installed packages
and are usually found in `/Library/Receipts` directory.
This is the most common format: usually when people refer to BOM file they actually mean receipt files.
This format is explained in a separate [document](./receipt-file-format-reference.md).

Compiled asset catalog (CAR) files store application's asset information.
This format is documented in the following blog posts:
- [Reverse engineering the .car file format (compiled Asset Catalogs)](https://blog.timac.org/2018/1018-reverse-engineering-the-car-file-format/)
- [QuickLook plugin to visualize .car files (compiled Asset Catalogs)](https://blog.timac.org/2018/1112-quicklook-plugin-to-visualize-car-files/)
