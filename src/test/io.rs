use std::fmt::Debug;

use arbitrary::Arbitrary;
use arbtest::arbtest;

use crate::BigEndianIo;

pub fn test_be_io_symmetry<T: for<'a> Arbitrary<'a> + Debug + Eq + BigEndianIo>() {
    test_be_io_symmetry_convert::<T, T>();
}

pub fn test_be_io_symmetry_convert<
    X: for<'a> Arbitrary<'a>,
    T: From<X> + Debug + Eq + BigEndianIo,
>() {
    arbtest(|u| {
        let expected: X = u.arbitrary()?;
        let expected: T = expected.into();
        let mut bytes = Vec::new();
        expected.write_be(&mut bytes).unwrap();
        let actual = T::read_be(&bytes[..]).unwrap();
        assert_eq!(expected, actual);
        Ok(())
    });
}
