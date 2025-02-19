use std::ffi::CString;
use std::io::Error;
use std::io::Read;
use std::io::Seek;
use std::io::Write;

use crate::receipt::Context;
use crate::receipt::VecTree;
use crate::BigEndianRead;
use crate::BigEndianWrite;
use crate::BlockRead;
use crate::BlockWrite;
use crate::Blocks;

/// Directory name to regex mapping.
#[derive(Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub struct VirtualPathTree {
    tree: VecTree<Option<VecTree<(), CString>>, CString>,
}

impl VirtualPathTree {
    const VERSION: u32 = 1;

    /// Create an empty tree.
    pub fn new() -> Self {
        Self {
            tree: Default::default(),
        }
    }
}

impl BlockRead<Context> for VirtualPathTree {
    fn read_block(
        i: u32,
        file: &[u8],
        blocks: &mut Blocks,
        context: &mut Context,
    ) -> Result<Self, Error> {
        let mut reader = blocks.slice(i, file)?;
        let version = u32::read_be(reader.by_ref())?;
        if version != Self::VERSION {
            return Err(Error::other("unsupported vindex version"));
        }
        let i = u32::read_be(reader.by_ref())?;
        let _x0 = u32::read_be(reader.by_ref())?;
        // TODO
        //debug_assert!(_x0 == 0, "x0 = {}", _x0);
        let _x1 = u8::read_be(reader.by_ref())?;
        // TODO
        //debug_assert!(_x1 == DEFAULT_X1, "x1 = {_x1}");
        let tree = VecTree::read_block(i, file, blocks, context)?;
        Ok(Self { tree })
    }
}

impl BlockWrite<Context> for VirtualPathTree {
    fn write_block<W: Write + Seek>(
        &self,
        mut writer: W,
        blocks: &mut Blocks,
        context: &mut Context,
    ) -> Result<u32, Error> {
        let tree_index = self.tree.write_block(writer.by_ref(), blocks, context)?;
        let i = blocks.append(writer.by_ref(), |writer| {
            Self::VERSION.write_be(writer.by_ref())?;
            tree_index.write_be(writer.by_ref())?;
            0_u32.write_be(writer.by_ref())?;
            DEFAULT_X1.write_be(writer.by_ref())?;
            Ok(())
        })?;
        Ok(i)
    }
}

impl Default for VirtualPathTree {
    fn default() -> Self {
        Self::new()
    }
}

const DEFAULT_X1: u8 = 1;
