//! Endian types for cross-platform compatibility.
//!
//! This module provides a set of types that can be used to read and write data in different endian
//! formats. The `EndianType` enum represents the endianness of the system, and the `Endianness`
//! trait provides methods to read and write data in the specified endianness.
//!
//! The number types, [`u16`], [`u32`], and [`u64`], have a counterpart with endianness, which are
//! [`crate::types::number::U16`], [`crate::types::number::U32`], and [`crate::types::number::U64`]. These types are used to read and write data in the specified
//! endianness, defined at the type level.

/// The endianness of the system.
///
/// This enum represents the endianness of the system at runtime. It can be used
/// to read and write data in the specified endianness.
///
/// NativeEndian is the default, and the fastest endianness, due to compatibility with the
/// current architecture. However, compiler optimizations will also optimize away LittleEndian and
/// BigEndian if the system is the same endianness.
#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum EndianType {
    /// Native endianness.
    #[default]
    NativeEndian,
    /// Little endianness.
    ///
    /// This means that the least significant byte is stored at the lowest address.
    /// For example, the byte order of the number `0x1234` is `0x3412`.
    LittleEndian,
    /// Big endianness.
    ///
    /// This means that the most significant byte is stored at the lowest address.
    /// For example, the byte order of the number `0x1234` is `0x1234`.
    BigEndian,
}

impl EndianType {
    pub const fn is_le(&self) -> bool {
        #[cfg(target_endian = "little")]
        {
            matches!(self, Self::LittleEndian | Self::NativeEndian)
        }
        #[cfg(target_endian = "big")]
        {
            matches!(self, Self::LittleEndian)
        }
    }
    /// Reads a `u16` from the given bytes in the specified endianness.
    pub fn read_u16(&self, bytes: [u8; 2]) -> u16 {
        match self {
            EndianType::NativeEndian => u16::from_ne_bytes(bytes),
            EndianType::LittleEndian => u16::from_le_bytes(bytes),
            EndianType::BigEndian => u16::from_be_bytes(bytes),
        }
    }

    /// Reads a `u32` from the given bytes in the specified endianness.
    pub fn read_u32(&self, bytes: [u8; 4]) -> u32 {
        match self {
            EndianType::NativeEndian => u32::from_ne_bytes(bytes),
            EndianType::LittleEndian => u32::from_le_bytes(bytes),
            EndianType::BigEndian => u32::from_be_bytes(bytes),
        }
    }

    /// Writes a `u32` to the given bytes in the specified endianness.
    pub fn read_u64(&self, bytes: [u8; 8]) -> u64 {
        match self {
            EndianType::NativeEndian => u64::from_ne_bytes(bytes),
            EndianType::LittleEndian => u64::from_le_bytes(bytes),
            EndianType::BigEndian => u64::from_be_bytes(bytes),
        }
    }

    /// Returns the byte representation of a `u16` in the specified endianness.
    pub fn u16_bytes(&self, value: u16) -> [u8; 2] {
        match self {
            EndianType::NativeEndian => value.to_ne_bytes(),
            EndianType::LittleEndian => value.to_le_bytes(),
            EndianType::BigEndian => value.to_be_bytes(),
        }
    }

    /// Returns the byte representation of a `u32` in the specified endianness.
    pub fn u32_bytes(&self, value: u32) -> [u8; 4] {
        match self {
            EndianType::NativeEndian => value.to_ne_bytes(),
            EndianType::LittleEndian => value.to_le_bytes(),
            EndianType::BigEndian => value.to_be_bytes(),
        }
    }

    /// Returns the byte representation of a `u64` in the specified endianness.
    pub fn u64_bytes(&self, value: u64) -> [u8; 8] {
        match self {
            EndianType::NativeEndian => value.to_ne_bytes(),
            EndianType::LittleEndian => value.to_le_bytes(),
            EndianType::BigEndian => value.to_be_bytes(),
        }
    }
}

/// A trait that represents the endianness of a type.
///
/// This trait shouldn`t be implemented directly, but rather through the [`Endian`] trait.
/// See [`crate::types::number::U16`], [`crate::types::number::U32`], and [`crate::types::number::U64`] for examples.
pub trait Endianness: Copy + Sized {
    /// Returns the endianness at runtime.
    fn get() -> EndianType;

    /// Reads a `u16` from the given bytes in the specified endianness.
    fn get_u16(bytes: [u8; 2]) -> u16;
    /// Writes a `u16` to the given bytes in the specified endianness.
    fn set_u16(value: u16, bytes: &mut [u8; 2]);
    /// Reads a `u32` from the given bytes in the specified endianness.
    fn get_u32(bytes: [u8; 4]) -> u32;
    /// Writes a `u32` to the given bytes in the specified endianness.
    fn set_u32(value: u32, bytes: &mut [u8; 4]);
    /// Reads a `u64` from the given bytes in the specified endianness.
    fn get_u64(bytes: [u8; 8]) -> u64;
    /// Writes a `u64` to the given bytes in the specified endianness.
    fn set_u64(value: u64, bytes: &mut [u8; 8]);
}

/// A type that represents the native endianness.
///
/// This zero-sized-type can be used where a generic type parameter is expected for endianness.
#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
#[cfg_attr(feature = "bytemuck", derive(bytemuck::Zeroable, bytemuck::Pod))]
pub struct NativeEndian;

/// A type that represents the little endianness.
///
/// This zero-sized-type can be used where a generic type parameter is expected for endianness.
#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
#[cfg_attr(feature = "bytemuck", derive(bytemuck::Zeroable, bytemuck::Pod))]
pub struct LittleEndian;

/// A type that represents the big endianness.
///
/// This zero-sized-type can be used where a generic type parameter is expected for endianness.
#[repr(transparent)]
#[derive(Debug, Copy, Clone)]
#[cfg_attr(feature = "bytemuck", derive(bytemuck::Zeroable, bytemuck::Pod))]
pub struct BigEndian;

impl Endianness for NativeEndian {
    #[inline]
    fn get() -> EndianType {
        EndianType::NativeEndian
    }

    #[inline]
    fn get_u16(bytes: [u8; 2]) -> u16 {
        u16::from_ne_bytes(bytes)
    }

    #[inline]
    fn set_u16(value: u16, bytes: &mut [u8; 2]) {
        bytes.copy_from_slice(&value.to_ne_bytes());
    }

    #[inline]
    fn get_u32(bytes: [u8; 4]) -> u32 {
        u32::from_ne_bytes(bytes)
    }

    #[inline]
    fn set_u32(value: u32, bytes: &mut [u8; 4]) {
        bytes.copy_from_slice(&value.to_ne_bytes());
    }

    #[inline]
    fn get_u64(bytes: [u8; 8]) -> u64 {
        u64::from_ne_bytes(bytes)
    }

    #[inline]
    fn set_u64(value: u64, bytes: &mut [u8; 8]) {
        bytes.copy_from_slice(&value.to_ne_bytes());
    }
}

impl Endianness for LittleEndian {
    #[inline]
    fn get() -> EndianType {
        EndianType::LittleEndian
    }

    #[inline]
    fn get_u16(bytes: [u8; 2]) -> u16 {
        u16::from_le_bytes(bytes)
    }

    #[inline]
    fn set_u16(value: u16, bytes: &mut [u8; 2]) {
        bytes.copy_from_slice(&value.to_le_bytes());
    }

    #[inline]
    fn get_u32(bytes: [u8; 4]) -> u32 {
        u32::from_le_bytes(bytes)
    }

    #[inline]
    fn set_u32(value: u32, bytes: &mut [u8; 4]) {
        bytes.copy_from_slice(&value.to_le_bytes());
    }

    #[inline]
    fn get_u64(bytes: [u8; 8]) -> u64 {
        u64::from_le_bytes(bytes)
    }

    #[inline]
    fn set_u64(value: u64, bytes: &mut [u8; 8]) {
        bytes.copy_from_slice(&value.to_le_bytes());
    }
}
impl Endianness for BigEndian {
    #[inline]
    fn get() -> EndianType {
        EndianType::BigEndian
    }

    #[inline]
    fn get_u16(bytes: [u8; 2]) -> u16 {
        u16::from_be_bytes(bytes)
    }

    #[inline]
    fn set_u16(value: u16, bytes: &mut [u8; 2]) {
        bytes.copy_from_slice(&value.to_be_bytes());
    }

    #[inline]
    fn get_u32(bytes: [u8; 4]) -> u32 {
        u32::from_be_bytes(bytes)
    }

    #[inline]
    fn set_u32(value: u32, bytes: &mut [u8; 4]) {
        bytes.copy_from_slice(&value.to_be_bytes());
    }

    #[inline]
    fn get_u64(bytes: [u8; 8]) -> u64 {
        u64::from_be_bytes(bytes)
    }

    #[inline]
    fn set_u64(value: u64, bytes: &mut [u8; 8]) {
        bytes.copy_from_slice(&value.to_be_bytes());
    }
}

/// A trait that represents a type that can be bytemuck::Pod and bytemuck::Zeroable, if the
/// `bytemuck` feature is enabled.
#[cfg(feature = "bytemuck")]
pub trait MaybePod: bytemuck::Pod + bytemuck::Zeroable {}
#[cfg(feature = "bytemuck")]
impl<T: bytemuck::Pod + bytemuck::Zeroable> MaybePod for T {}
#[cfg(not(feature = "bytemuck"))]
pub trait MaybePod {}
#[cfg(not(feature = "bytemuck"))]
impl<T> MaybePod for T {}

/// A trait that represents a type with endianness.
///
/// This trait is used to read and write data in the specified endianness.
/// It is implemented for all number types, and can be used to read and write data in the specified
/// endianness.
///
/// The `Output` type parameter represents the type that the trait will return when reading or
/// writing data. This type should be a primitive type or a struct that implements the `Pod` and
/// `Zeroable` traits from the `bytemuck` crate, if the `bytemuck` feature is enabled.
///
/// The `LsbType` and `MsbType` type parameters are variants of the type that the trait will return
/// when reading or writing data.
pub trait Endian {
    /// The type that the trait will return when reading or writing data.
    ///
    /// This type should return a primitive type or a struct that implements the `Pod` and
    /// `Zeroable` traits from the `bytemuck` crate, if the `bytemuck` feature is enabled.
    /// This type can be endianness-specific, for example, the `crate::types::number::U16` type is a struct that outputs
    /// a `u16` value in the specified endianness.
    type Output: MaybePod;

    /// The Little Endian variant of the type.
    ///
    /// This type should return a little-endian variant of the type, for example, the LSB type for
    /// a `crate::types::number::U16` is a `crate::types::number::U16<LittleEndian>` type.
    type LsbType: MaybePod + Endian<Output = Self::Output>;

    /// The Big Endian variant of the type.
    ///
    /// This type should return a big-endian variant of the type, for example, the MSB type for
    /// a `crate::types::number::U16` is a `crate::types::number::U16<BigEndian>` type.
    type MsbType: MaybePod + Endian<Output = Self::Output>;

    /// Creates a new instance of the type with the given value.
    fn new(value: Self::Output) -> Self;
    /// Returns the value of the type.
    fn get(&self) -> Self::Output;
    /// Sets the value of the type.
    fn set(&mut self, value: Self::Output);
}

#[cfg(all(test, feature = "std"))]
mod tests {
    #[test]
    fn test_from_le_bytes() {
        let value = u16::from_le_bytes([0x12, 0x34]);
        assert_eq!(value, 0x3412);

        let value = u32::from_le_bytes([0x12, 0x34, 0x56, 0x78]);
        assert_eq!(value, 0x78563412);

        let value = u64::from_le_bytes([0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0]);
        assert_eq!(value, 0xf0debc9a78563412);
    }

    #[test]
    fn test_from_be_bytes() {
        let value = u16::from_be_bytes([0x12, 0x34]);
        assert_eq!(value, 0x1234);

        let value = u32::from_be_bytes([0x12, 0x34, 0x56, 0x78]);
        assert_eq!(value, 0x12345678);

        let value = u64::from_be_bytes([0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0]);
        assert_eq!(value, 0x123456789abcdef0);
    }
}
