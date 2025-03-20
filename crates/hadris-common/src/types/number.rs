use crate::types::endian::{BigEndian, Endian, Endianness, LittleEndian};
use core::marker::PhantomData;

#[repr(transparent)]
#[derive(Clone, Copy, bytemuck::Zeroable, bytemuck::Pod)]
pub struct U16<E: Endianness> {
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

#[repr(transparent)]
#[derive(Clone, Copy, bytemuck::Zeroable, bytemuck::Pod)]
pub struct U32<E: Endianness> {
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

#[repr(transparent)]
#[derive(Clone, Copy, bytemuck::Zeroable, bytemuck::Pod)]
pub struct U64<E: Endianness> {
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
    use super::*;
    use crate::types::endian::NativeEndian;

    #[test]
    fn test_u16() {
        let mut value = U16::<NativeEndian>::new(0x1234);
        assert_eq!(value.get(), 0x1234);
        value.set(0x5678);
        assert_eq!(value.get(), 0x5678);
    }

    #[test]
    fn test_u32() {
        let mut value = U32::<NativeEndian>::new(0x12345678);
        assert_eq!(value.get(), 0x12345678);
        value.set(0x9abcdef0);
        assert_eq!(value.get(), 0x9abcdef0);
    }

    #[test]
    fn test_u64() {
        let mut value = U64::<NativeEndian>::new(0x123456789abcdef0);
        assert_eq!(value.get(), 0x123456789abcdef0);
        value.set(0x0123456789abcdef);
        assert_eq!(value.get(), 0x0123456789abcdef);
    }
}
