use crate::receipt::FileSizes64;
use crate::receipt::HardLinks;

#[derive(Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub struct Context {
    /// 64-bit file sizes.
    pub file_size_64: FileSizes64,

    /// Metadata block index to path mapping.
    pub hard_links: HardLinks,
}
// TODO add blocks?

impl Context {
    pub fn new() -> Self {
        Self {
            file_size_64: Default::default(),
            hard_links: Default::default(),
        }
    }
}

pub type Tree<K, V> = crate::Tree<K, V, Context>;
pub type InvertedTree<K, V> = crate::InvertedTree<K, V, Context>;
