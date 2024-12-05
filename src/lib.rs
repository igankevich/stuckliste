#![doc = include_str!("../README.md")]

mod bom;
pub mod receipt;
#[cfg(test)]
pub mod test;

pub use self::bom::*;
