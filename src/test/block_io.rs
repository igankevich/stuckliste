use std::fmt::Debug;
use std::io::Cursor;

use arbitrary::Arbitrary;
use arbtest::arbtest;

use crate::receipt::Context;
use crate::BlockIo;
use crate::Blocks;

pub fn block_io_symmetry<T: for<'a> Arbitrary<'a> + Debug + Eq + BlockIo<Context>>() {
    block_io_symmetry_convert::<T, T>();
}

pub fn block_io_symmetry_convert<
    X: for<'a> Arbitrary<'a>,
    T: From<X> + Debug + Eq + BlockIo<Context>,
>() {
    arbtest(|u| {
        let expected: X = u.arbitrary()?;
        test_block_io_symmetry_convert::<X, T>(expected);
        Ok(())
    });
}

pub fn test_block_io_symmetry<T: Debug + Eq + BlockIo<Context>>(expected: T) {
    test_block_io_symmetry_convert::<T, T>(expected);
}

pub fn test_block_io_symmetry_convert<X, T: From<X> + Debug + Eq + BlockIo<Context>>(expected: X) {
    let mut blocks = Blocks::new();
    let mut context = Context::new();
    let expected: T = expected.into();
    let mut writer = Cursor::new(Vec::new());
    let i = expected
        .write_block(&mut writer, &mut blocks, &mut context)
        .unwrap();
    let bytes = writer.into_inner();
    let actual = T::read_block(i, &bytes[..], &mut blocks, &mut context).unwrap();
    assert_eq!(expected, actual);
}
