//! Byte orders for the endian-aware integers in [`crate::types::number`].

/// A byte order, chosen at the type level by [`crate::types::number::U16`],
/// [`crate::types::number::U32`] and [`crate::types::number::U64`].
pub trait Endianness: Copy + Sized {
    /// Reads a `u16` from the given bytes in this byte order.
    fn get_u16(bytes: [u8; 2]) -> u16;
    /// Writes a `u16` to the given bytes in this byte order.
    fn set_u16(value: u16, bytes: &mut [u8; 2]);
    /// Reads a `u32` from the given bytes in this byte order.
    fn get_u32(bytes: [u8; 4]) -> u32;
    /// Writes a `u32` to the given bytes in this byte order.
    fn set_u32(value: u32, bytes: &mut [u8; 4]);
    /// Reads a `u64` from the given bytes in this byte order.
    fn get_u64(bytes: [u8; 8]) -> u64;
    /// Writes a `u64` to the given bytes in this byte order.
    fn set_u64(value: u64, bytes: &mut [u8; 8]);
}

/// Little-endian byte order.
#[repr(transparent)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "bytemuck", derive(bytemuck::Zeroable, bytemuck::Pod))]
pub struct LittleEndian;

impl Endianness for LittleEndian {
    #[inline]
    fn get_u16(bytes: [u8; 2]) -> u16 {
        u16::from_le_bytes(bytes)
    }

    #[inline]
    fn set_u16(value: u16, bytes: &mut [u8; 2]) {
        *bytes = value.to_le_bytes();
    }

    #[inline]
    fn get_u32(bytes: [u8; 4]) -> u32 {
        u32::from_le_bytes(bytes)
    }

    #[inline]
    fn set_u32(value: u32, bytes: &mut [u8; 4]) {
        *bytes = value.to_le_bytes();
    }

    #[inline]
    fn get_u64(bytes: [u8; 8]) -> u64 {
        u64::from_le_bytes(bytes)
    }

    #[inline]
    fn set_u64(value: u64, bytes: &mut [u8; 8]) {
        *bytes = value.to_le_bytes();
    }
}
