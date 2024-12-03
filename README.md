# stuckliste

[![Crates.io Version](https://img.shields.io/crates/v/stuckliste)](https://crates.io/crates/stuckliste)
[![Docs](https://docs.rs/stuckliste/badge.svg)](https://docs.rs/stuckliste)
[![dependency status](https://deps.rs/repo/github/igankevich/stuckliste/status.svg)](https://deps.rs/repo/github/igankevich/stuckliste)

MacOS's bill-of-materials (BOM) files reader/writer library that is
fuzz-tested against the original `mkbom` and `lsbom` utilities.
Includes re-implementation of these utilities as well.


## Introduction

`stuckliste` is a library that offers types and methods for reading/writing MacOS bill-of-materials (BOM) files.
These files are generic storage container for various type of information
with the most common type being _receipts_ —
files that contains a list of all files and directories owned by a package and
that are usually stored under `/Library/Receipts`.
The library is fuzz-tested against MacOS's `mkbom` and `lsbom` utilities ensuring that 
it produces structurely the same output.


## Installation

The easiest way to use `stuckliste` is via command line interface.

```bash
cargo install stuckliste-cli
```


## Usage


### As a command-line application

```bash
mkbom /tmp /tmp/receipt.bom
lsbom /tmp/receipt.bom
```


### As a library

```rust
use std::fs::File;
use std::io::Error;
use stuckliste::receipt::{Receipt, ReceiptBuilder};

fn create_receipt() -> Result<(), Error> {
    let file = File::create("/tmp/receipt.bom")?;
    let receipt = ReceiptBuilder::new().create("/tmp")?;
    receipt.write(file)?;
    Ok(())
}

fn read_receipt() -> Result<(), Error> {
    let file = File::open("/tmp/receipt.bom")?;
    let receipt = Receipt::read(file)?;
    for (path, metadata) in receipt.entries()?.into_iter() {
        println!("{:?}: {:?}", path, metadata);
    }
    Ok(())
}
```

## References

This work is based on the following reverse-engineering efforts.
- [Bomutils](https://github.com/hogliux/bomutils)
- [Darling](https://github.com/darlinghq/darling-installer)
- [Bom](https://github.com/iineva/bom)
- [Reverse engineering the .car file format (compiled Asset Catalogs)](https://blog.timac.org/2018/1018-reverse-engineering-the-car-file-format/)
- [QuickLook plugin to visualize .car files (compiled Asset Catalogs)](https://blog.timac.org/2018/1112-quicklook-plugin-to-visualize-car-files/)
