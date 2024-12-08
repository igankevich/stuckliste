use crate::receipt::FileSizes64;
use crate::receipt::HardLinks;

/// File i/o context for receipts.
///
/// Holds file-wide data. Currently this includes 64-bit file sizes and hard links.
///
/// The same instances of context should be passed to
/// [`read_block`](crate::BlockRead::read_block) and
/// [`write_block`](crate::BlockWrite::write_block) functions
/// to get the correct results.
#[derive(Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub struct Context {
    /// 64-bit file sizes.
    pub file_size_64: FileSizes64,

    /// Metadata block index to path mapping.
    pub hard_links: HardLinks,
}

impl Context {
    /// Create an empty context.
    pub fn new() -> Self {
        Self {
            file_size_64: Default::default(),
            hard_links: Default::default(),
        }
    }
}

impl Default for Context {
    fn default() -> Self {
        Self::new()
    }
}

/// An alias to [`VecTree`](crate::VecTree] with `Context` plugged in.
pub type VecTree<K, V> = crate::VecTree<K, V, Context>;
