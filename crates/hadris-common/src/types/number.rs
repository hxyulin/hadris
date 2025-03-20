use crate::types::endian::{BigEndian, Endian, Endianness, LittleEndian};
use core::marker::PhantomData;

/// A 16-bit unsigned integer with a specified endianness.
#[repr(transparent)]
#[derive(Clone, Copy)]
#[cfg_attr(feature = "bytemuck", derive(bytemuck::Zeroable, bytemuck::Pod))]
pub struct U16<E>
where
    E: Endianness,
{
    bytes: [u8; 2],
    _marker: PhantomData<E>,
}

impl<E: Endianness> Endian for U16<E> {
    type Output = u16;
    type LsbType = U16<LittleEndian>;
    type MsbType = U16<BigEndian>;

    fn new(value: u16) -> Self {
        let mut bytes = [0; 2];
        E::set_u16(value, &mut bytes);
        Self {
            bytes,
            _marker: PhantomData,
        }
    }

    fn get(&self) -> u16 {
        E::get_u16(self.bytes)
    }

    fn set(&mut self, value: u16) {
        E::set_u16(value, &mut self.bytes);
    }
}

impl<E: Endianness> core::fmt::Debug for U16<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("U16").field(&self.get()).finish()
    }
}

impl<E: Endianness> core::fmt::LowerHex for U16<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let value = self.get();
        write!(f, "0x{:04x}", value)
    }
}

impl<E: Endianness> core::fmt::UpperHex for U16<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let value = self.get();
        write!(f, "0x{:04X}", value)
    }
}

/// A 32-bit unsigned integer with a specified endianness.
#[repr(transparent)]
#[derive(Clone, Copy)]
#[cfg_attr(feature = "bytemuck", derive(bytemuck::Zeroable, bytemuck::Pod))]
pub struct U32<E>
where
    E: Endianness,
{
    bytes: [u8; 4],
    _marker: PhantomData<E>,
}

impl<E: Endianness> Endian for U32<E> {
    type Output = u32;
    type LsbType = U32<LittleEndian>;
    type MsbType = U32<BigEndian>;

    fn new(value: u32) -> Self {
        let mut bytes = [0; 4];
        E::set_u32(value, &mut bytes);
        Self {
            bytes,
            _marker: PhantomData,
        }
    }

    fn get(&self) -> u32 {
        E::get_u32(self.bytes)
    }

    fn set(&mut self, value: u32) {
        E::set_u32(value, &mut self.bytes);
    }
}

impl<E: Endianness> core::fmt::Debug for U32<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("U32").field(&self.get()).finish()
    }
}

impl<E: Endianness> core::fmt::LowerHex for U32<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let value = self.get();
        write!(f, "0x{:08x}", value)
    }
}

impl<E: Endianness> core::fmt::UpperHex for U32<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let value = self.get();
        write!(f, "0x{:08X}", value)
    }
}

/// A 64-bit unsigned integer with a specified endianness.
#[repr(transparent)]
#[derive(Clone, Copy)]
#[cfg_attr(feature = "bytemuck", derive(bytemuck::Zeroable, bytemuck::Pod))]
pub struct U64<E>
where
    E: Endianness,
{
    bytes: [u8; 8],
    _marker: PhantomData<E>,
}

impl<E: Endianness> Endian for U64<E> {
    type Output = u64;
    type LsbType = U64<LittleEndian>;
    type MsbType = U64<BigEndian>;

    fn new(value: u64) -> Self {
        let mut bytes = [0; 8];
        E::set_u64(value, &mut bytes);
        Self {
            bytes,
            _marker: PhantomData,
        }
    }

    fn get(&self) -> u64 {
        E::get_u64(self.bytes)
    }

    fn set(&mut self, value: u64) {
        E::set_u64(value, &mut self.bytes);
    }
}

impl<E: Endianness> core::fmt::Debug for U64<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("U64").field(&self.get()).finish()
    }
}

impl<E: Endianness> core::fmt::LowerHex for U64<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let value = self.get();
        write!(f, "0x{:016x}", value)
    }
}

impl<E: Endianness> core::fmt::UpperHex for U64<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let value = self.get();
        write!(f, "0x{:016X}", value)
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    //! Tests for the number types.
    //!
    //! We can't really test NativeEndian, so there are tests for LittleEndian and BigEndian.

    use super::*;

    #[test]
    fn test_u16_repr() {
        let value = U16::<LittleEndian>::new(0x1234);
        assert_eq!(value.bytes, [0x34, 0x12]);
        let value = U16::<BigEndian>::new(0x1234);
        assert_eq!(value.bytes, [0x12, 0x34]);
    }

    #[test]
    fn test_u32_repr() {
        let value = U32::<LittleEndian>::new(0x12345678);
        assert_eq!(value.bytes, [0x78, 0x56, 0x34, 0x12]);
        let value = U32::<BigEndian>::new(0x12345678);
        assert_eq!(value.bytes, [0x12, 0x34, 0x56, 0x78]);
    }

    #[test]
    fn test_u64_repr() {
        let value = U64::<LittleEndian>::new(0x123456789abcdef0);
        assert_eq!(value.bytes, [0x0f, 0xde, 0xbc, 0x9a, 0x78, 0x56, 0x34, 0x12]);
        let value = U64::<BigEndian>::new(0x123456789abcdef0);
        assert_eq!(value.bytes, [0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0]);
    }
}
