use crate::types::endian::Endianness;
use core::marker::PhantomData;

macro_rules! endian_int {
    ($($ty:ident, $out:ty, $n:literal, $get:ident, $set:ident, $w:literal, $name:literal;)*) => {$(
        #[doc = concat!("A ", $name, " unsigned integer stored in byte order `E`.")]
        #[repr(transparent)]
        #[derive(Clone, Copy)]
        #[cfg_attr(feature = "bytemuck", derive(bytemuck::Zeroable, bytemuck::Pod))]
        pub struct $ty<E>
        where
            E: Endianness,
        {
            bytes: [u8; $n],
            _marker: PhantomData<E>,
        }

        impl<E: Endianness> $ty<E> {
            /// Stores `value` in byte order `E`.
            #[inline]
            pub fn new(value: $out) -> Self {
                let mut bytes = [0; $n];
                E::$set(value, &mut bytes);
                Self {
                    bytes,
                    _marker: PhantomData,
                }
            }

            /// The stored value.
            #[inline]
            pub fn get(&self) -> $out {
                E::$get(self.bytes)
            }

            /// Replaces the stored value.
            #[inline]
            pub fn set(&mut self, value: $out) {
                E::$set(value, &mut self.bytes);
            }
        }

        impl<E: Endianness> core::fmt::Debug for $ty<E> {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.debug_tuple(stringify!($ty)).field(&self.get()).finish()
            }
        }

        impl<E: Endianness> core::fmt::Display for $ty<E> {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "{}", self.get())
            }
        }

        impl<E: Endianness> core::fmt::LowerHex for $ty<E> {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "0x{:0w$x}", self.get(), w = $w)
            }
        }

        impl<E: Endianness> core::fmt::UpperHex for $ty<E> {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "0x{:0w$X}", self.get(), w = $w)
            }
        }
    )*};
}

endian_int! {
    U16, u16, 2, get_u16, set_u16, 4, "16-bit";
    U32, u32, 4, get_u32, set_u32, 8, "32-bit";
    U64, u64, 8, get_u64, set_u64, 16, "64-bit";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::endian::LittleEndian;

    #[test]
    fn little_endian_repr() {
        assert_eq!(U16::<LittleEndian>::new(0x1234).bytes, [0x34, 0x12]);
        assert_eq!(
            U32::<LittleEndian>::new(0x12345678).bytes,
            [0x78, 0x56, 0x34, 0x12]
        );
        assert_eq!(
            U64::<LittleEndian>::new(0x123456789abcdef0).bytes,
            [0xf0, 0xde, 0xbc, 0x9a, 0x78, 0x56, 0x34, 0x12]
        );
    }
}
