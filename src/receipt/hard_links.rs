use std::collections::HashMap;
use std::ffi::CString;
use std::io::Error;
use std::io::Seek;
use std::io::Write;
use std::ops::Deref;
use std::ops::DerefMut;

use crate::receipt::Context;
use crate::receipt::Ptr;
use crate::receipt::VecTree;
use crate::BlockRead;
use crate::BlockWrite;
use crate::Blocks;

/// Metadata block index to 64-bit file size mapping.
#[derive(Debug, Default)]
#[cfg_attr(test, derive(arbitrary::Arbitrary, PartialEq, Eq))]
pub struct HardLinks(HashMap<u32, Vec<CString>>);

impl HardLinks {
    const OUTER_BLOCK_LEN: usize = 4096;
    const INNER_BLOCK_LEN: usize = 128;

    /// Transform into inner representation.
    pub fn into_inner(self) -> HashMap<u32, Vec<CString>> {
        self.0
    }
}

impl Deref for HardLinks {
    type Target = HashMap<u32, Vec<CString>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for HardLinks {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl BlockWrite<Context> for HardLinks {
    fn write_block<W: Write + Seek>(
        &self,
        mut writer: W,
        blocks: &mut Blocks,
        context: &mut Context,
    ) -> Result<u32, Error> {
        let mut hard_links = Vec::with_capacity(self.0.len());
        for (block, paths) in self.0.iter() {
            let paths_tree = PathsTree::new(
                paths.iter().map(|path| ((), path.clone())).collect(),
                Self::INNER_BLOCK_LEN,
            );
            let paths_tree = Ptr::new(paths_tree);
            hard_links.push((paths_tree, *block));
        }
        let hard_links_tree = HardLinkTree::new(hard_links, Self::OUTER_BLOCK_LEN);
        let i = hard_links_tree.write_block(writer.by_ref(), blocks, context)?;
        Ok(i)
    }
}

impl BlockRead<Context> for HardLinks {
    fn read_block(
        i: u32,
        file: &[u8],
        blocks: &mut Blocks,
        context: &mut Context,
    ) -> Result<Self, Error> {
        let tree = HardLinkTree::read_block(i, file, blocks, context)?;
        let mut hard_links: HashMap<u32, Vec<CString>> = HashMap::new();
        for (hard_links_tree, metadata_index) in tree.into_inner().into_iter() {
            let names = hard_links.entry(metadata_index).or_default();
            for (_, name) in hard_links_tree.into_inner().into_inner().into_iter() {
                names.push(name);
            }
        }
        Ok(Self(hard_links))
    }
}

/// Key is a tree of hard link names, value is metadata block index.
type HardLinkTree = VecTree<Ptr<PathsTree>, u32>;
type PathsTree = VecTree<(), CString>;

#[cfg(test)]
mod tests {

    use super::*;
    use crate::test::block_io_symmetry;

    #[test]
    fn write_read_symmetry() {
        block_io_symmetry::<HardLinks>();
    }
}
