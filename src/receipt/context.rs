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
    pub file_sizes: FileSizes64,

    /// Metadata block index to path mapping.
    pub hard_links: HardLinks,

    /// Current metadata block index that was written in `PathComponentKey::write_block`.
    ///
    /// This value is subsequently used by `PathComponentValue::write_block` to generate
    /// hard links.
    pub(crate) current_metadata_block_index: Option<u32>,
}

impl Context {
    /// Create an empty context.
    pub fn new() -> Self {
        Self {
            file_sizes: Default::default(),
            hard_links: Default::default(),
            current_metadata_block_index: Default::default(),
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
