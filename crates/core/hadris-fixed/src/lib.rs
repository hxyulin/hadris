//! Fixed-capacity byte and text types for `no_std` applications.

#![no_std]

#[cfg(any(feature = "alloc", test))]
extern crate alloc;

use core::fmt;
use core::marker::PhantomData;
use core::ops::Range;

/// The fixed-capacity buffer cannot hold the requested data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapacityError;

impl fmt::Display for CapacityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("fixed-capacity buffer is full")
    }
}

impl core::error::Error for CapacityError {}

/// Stack-allocated arbitrary bytes with a fixed maximum capacity.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct FixedBytes<const N: usize> {
    data: [u8; N],
    len: usize,
}

impl<const N: usize> FixedBytes<N> {
    /// Creates an empty buffer.
    pub const fn new() -> Self {
        Self {
            data: [0; N],
            len: 0,
        }
    }

    /// Compatibility name for [`new`](Self::new).
    pub const fn empty() -> Self {
        Self::new()
    }

    /// Creates a zero-filled buffer with an initialized length.
    ///
    /// # Panics
    /// Panics when `size` exceeds the capacity.
    pub const fn with_size(size: usize) -> Self {
        assert!(size <= N);
        Self {
            data: [0; N],
            len: size,
        }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub const fn capacity(&self) -> usize {
        N
    }

    pub const fn remaining_capacity(&self) -> usize {
        N - self.len
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.data[..self.len]
    }

    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        &mut self.data[..self.len]
    }

    pub fn try_as_str(&self) -> Result<&str, core::str::Utf8Error> {
        core::str::from_utf8(self.as_bytes())
    }

    /// Returns the initialized bytes as UTF-8.
    ///
    /// # Panics
    /// Panics if the buffer does not contain valid UTF-8.
    pub fn as_str(&self) -> &str {
        self.try_as_str()
            .expect("FixedBytes contains invalid UTF-8")
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    pub fn truncate(&mut self, new_len: usize) {
        assert!(new_len <= self.len);
        self.len = new_len;
    }

    pub fn allocate(&mut self, bytes: usize) {
        assert!(bytes <= self.remaining_capacity());
        self.len += bytes;
    }

    pub fn try_from_slice(value: &[u8]) -> Result<Self, CapacityError> {
        let mut result = Self::new();
        result.try_push_slice(value)?;
        Ok(result)
    }

    pub fn try_push_slice(&mut self, value: &[u8]) -> Result<Range<usize>, CapacityError> {
        if value.len() > self.remaining_capacity() {
            return Err(CapacityError);
        }
        let start = self.len;
        self.len += value.len();
        self.data[start..self.len].copy_from_slice(value);
        Ok(start..self.len)
    }

    pub fn push_slice(&mut self, value: &[u8]) -> Range<usize> {
        self.try_push_slice(value)
            .expect("FixedBytes capacity exceeded")
    }

    pub fn try_push_byte(&mut self, value: u8) -> Result<usize, CapacityError> {
        if self.len == N {
            return Err(CapacityError);
        }
        let index = self.len;
        self.data[index] = value;
        self.len += 1;
        Ok(index)
    }

    pub fn push_byte(&mut self, value: u8) -> usize {
        self.try_push_byte(value)
            .expect("FixedBytes capacity exceeded")
    }
}

impl<const N: usize> Default for FixedBytes<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> From<&[u8]> for FixedBytes<N> {
    fn from(value: &[u8]) -> Self {
        Self::try_from_slice(value).expect("FixedBytes capacity exceeded")
    }
}

impl<const N: usize> From<&[u8; N]> for FixedBytes<N> {
    fn from(value: &[u8; N]) -> Self {
        Self {
            data: *value,
            len: N,
        }
    }
}

impl<const N: usize> fmt::Debug for FixedBytes<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("FixedBytes").field(&self.as_bytes()).finish()
    }
}

impl<const N: usize> fmt::Display for FixedBytes<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.try_as_str() {
            Ok(value) => f.write_str(value),
            Err(_) => write!(f, "{:?}", self.as_bytes()),
        }
    }
}

/// Stack-allocated, valid UTF-8 text with a fixed byte capacity.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct FixedStr<const N: usize>(FixedBytes<N>);

impl<const N: usize> FixedStr<N> {
    pub const fn new() -> Self {
        Self(FixedBytes::new())
    }

    pub const fn len(&self) -> usize {
        self.0.len()
    }

    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub const fn capacity(&self) -> usize {
        N
    }

    pub const fn remaining_capacity(&self) -> usize {
        self.0.remaining_capacity()
    }

    pub fn as_str(&self) -> &str {
        // Construction and mutation accept only valid UTF-8.
        unsafe { core::str::from_utf8_unchecked(self.0.as_bytes()) }
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn try_push_str(&mut self, value: &str) -> Result<Range<usize>, CapacityError> {
        self.0.try_push_slice(value.as_bytes())
    }

    pub fn push_str(&mut self, value: &str) -> Range<usize> {
        self.try_push_str(value)
            .expect("FixedStr capacity exceeded")
    }

    pub fn try_push(&mut self, value: char) -> Result<Range<usize>, CapacityError> {
        let mut bytes = [0; 4];
        self.try_push_str(value.encode_utf8(&mut bytes))
    }

    pub fn truncate(&mut self, new_len: usize) {
        assert!(self.as_str().is_char_boundary(new_len));
        self.0.truncate(new_len);
    }

    pub const fn into_bytes(self) -> FixedBytes<N> {
        self.0
    }
}

impl<const N: usize> Default for FixedStr<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> TryFrom<&str> for FixedStr<N> {
    type Error = CapacityError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Ok(Self(FixedBytes::try_from_slice(value.as_bytes())?))
    }
}

impl<const N: usize> TryFrom<FixedBytes<N>> for FixedStr<N> {
    type Error = core::str::Utf8Error;

    fn try_from(value: FixedBytes<N>) -> Result<Self, Self::Error> {
        value.try_as_str()?;
        Ok(Self(value))
    }
}

impl<const N: usize> fmt::Debug for FixedStr<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("FixedStr").field(&self.as_str()).finish()
    }
}

impl<const N: usize> fmt::Display for FixedStr<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// UTF-16 byte order used by [`FixedUtf16`].
pub trait Utf16ByteOrder: Copy {
    fn read(bytes: [u8; 2]) -> u16;
    fn write(value: u16) -> [u8; 2];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LittleEndian;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BigEndian;

impl Utf16ByteOrder for LittleEndian {
    fn read(bytes: [u8; 2]) -> u16 {
        u16::from_le_bytes(bytes)
    }
    fn write(value: u16) -> [u8; 2] {
        value.to_le_bytes()
    }
}

impl Utf16ByteOrder for BigEndian {
    fn read(bytes: [u8; 2]) -> u16 {
        u16::from_be_bytes(bytes)
    }
    fn write(value: u16) -> [u8; 2] {
        value.to_be_bytes()
    }
}

/// Fixed-width, NUL-padded UTF-16 code units in a specified byte order.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedUtf16<const N: usize, E: Utf16ByteOrder> {
    data: [[u8; 2]; N],
    byte_order: PhantomData<E>,
}

pub type FixedUtf16Le<const N: usize> = FixedUtf16<N, LittleEndian>;
pub type FixedUtf16Be<const N: usize> = FixedUtf16<N, BigEndian>;

impl<const N: usize, E: Utf16ByteOrder> FixedUtf16<N, E> {
    pub const fn new() -> Self {
        Self {
            data: [[0; 2]; N],
            byte_order: PhantomData,
        }
    }

    pub fn try_from_str(value: &str) -> Result<Self, CapacityError> {
        let mut result = Self::new();
        let mut index = 0;
        for unit in value.encode_utf16() {
            if index == N {
                return Err(CapacityError);
            }
            result.data[index] = E::write(unit);
            index += 1;
        }
        Ok(result)
    }

    pub fn as_bytes(&self) -> &[[u8; 2]; N] {
        &self.data
    }

    pub fn decode(&self) -> impl Iterator<Item = Result<char, core::char::DecodeUtf16Error>> + '_ {
        char::decode_utf16(
            self.data
                .iter()
                .map(|bytes| E::read(*bytes))
                .take_while(|unit| *unit != 0),
        )
    }

    #[cfg(feature = "alloc")]
    pub fn to_string(&self) -> Result<alloc::string::String, core::char::DecodeUtf16Error> {
        self.decode().collect()
    }
}

impl<const N: usize, E: Utf16ByteOrder> Default for FixedUtf16<N, E> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "bytemuck")]
unsafe impl<const N: usize, E: Utf16ByteOrder + 'static> bytemuck::Zeroable for FixedUtf16<N, E> {}
#[cfg(feature = "bytemuck")]
unsafe impl<const N: usize, E: Utf16ByteOrder + 'static> bytemuck::Pod for FixedUtf16<N, E> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_allow_non_utf8() {
        let bytes = FixedBytes::<2>::try_from_slice([0xff, 0].as_slice()).unwrap();
        assert!(bytes.try_as_str().is_err());
    }

    #[test]
    fn fixed_str_preserves_utf8() {
        let mut text = FixedStr::<8>::try_from("é").unwrap();
        text.try_push('!').unwrap();
        assert_eq!(text.as_str(), "é!");
    }

    #[test]
    fn utf16_round_trips_both_orders() {
        let le = FixedUtf16Le::<8>::try_from_str("A😀").unwrap();
        let be = FixedUtf16Be::<8>::try_from_str("A😀").unwrap();
        assert_eq!(le.to_string().unwrap(), "A😀");
        assert_eq!(be.to_string().unwrap(), "A😀");
    }
}
