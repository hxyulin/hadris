//! Fixed-capacity byte, text and collection types for allocation-free parsing.

use core::fmt;
use core::marker::PhantomData;
use core::ops::{Index, IndexMut, Range};

use super::endian::{BigEndian, Endianness, LittleEndian};

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

    /// Returns the number of initialized bytes.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Returns whether the buffer contains no initialized bytes.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns the maximum number of bytes the buffer can hold.
    pub const fn capacity(&self) -> usize {
        N
    }

    /// Returns the number of bytes that can still be appended.
    pub const fn remaining_capacity(&self) -> usize {
        N - self.len
    }

    /// Returns the initialized portion of the buffer.
    pub fn as_bytes(&self) -> &[u8] {
        &self.data[..self.len]
    }

    /// Returns the initialized portion of the buffer mutably.
    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        &mut self.data[..self.len]
    }

    /// Interprets the initialized bytes as UTF-8.
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

    /// Removes all initialized bytes.
    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// Shortens the initialized portion to `new_len`.
    ///
    /// # Panics
    /// Panics if `new_len` exceeds the current length.
    pub fn truncate(&mut self, new_len: usize) {
        assert!(new_len <= self.len);
        self.len = new_len;
    }

    /// Extends the initialized portion with zero-filled bytes.
    ///
    /// # Panics
    /// Panics if the requested bytes exceed the remaining capacity.
    pub fn allocate(&mut self, bytes: usize) {
        assert!(bytes <= self.remaining_capacity());
        self.len += bytes;
    }

    /// Copies a byte slice into a new buffer.
    pub fn try_from_slice(value: &[u8]) -> Result<Self, CapacityError> {
        let mut result = Self::new();
        result.try_push_slice(value)?;
        Ok(result)
    }

    /// Appends a slice and returns the occupied range.
    pub fn try_push_slice(&mut self, value: &[u8]) -> Result<Range<usize>, CapacityError> {
        if value.len() > self.remaining_capacity() {
            return Err(CapacityError);
        }
        let start = self.len;
        self.len += value.len();
        self.data[start..self.len].copy_from_slice(value);
        Ok(start..self.len)
    }

    /// Appends a slice and returns the occupied range.
    ///
    /// # Panics
    /// Panics if the slice exceeds the remaining capacity.
    pub fn push_slice(&mut self, value: &[u8]) -> Range<usize> {
        self.try_push_slice(value)
            .expect("FixedBytes capacity exceeded")
    }

    /// Appends one byte and returns its index.
    pub fn try_push_byte(&mut self, value: u8) -> Result<usize, CapacityError> {
        if self.len == N {
            return Err(CapacityError);
        }
        let index = self.len;
        self.data[index] = value;
        self.len += 1;
        Ok(index)
    }

    /// Appends one byte and returns its index.
    ///
    /// # Panics
    /// Panics if the buffer is full.
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
    /// # Panics
    /// Panics if the slice is longer than `N`. Use
    /// [`FixedBytes::try_from_slice`] for a fallible conversion.
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
    /// Creates an empty string.
    pub const fn new() -> Self {
        Self(FixedBytes::new())
    }

    /// Returns the UTF-8 byte length.
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns whether the string is empty.
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the maximum UTF-8 byte capacity.
    pub const fn capacity(&self) -> usize {
        N
    }

    /// Returns the remaining UTF-8 byte capacity.
    pub const fn remaining_capacity(&self) -> usize {
        self.0.remaining_capacity()
    }

    /// Returns the contents as a string slice.
    pub fn as_str(&self) -> &str {
        // Construction and mutation accept only valid UTF-8.
        unsafe { core::str::from_utf8_unchecked(self.0.as_bytes()) }
    }

    /// Returns the UTF-8 bytes.
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    /// Removes all text.
    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// Appends text and returns its byte range.
    pub fn try_push_str(&mut self, value: &str) -> Result<Range<usize>, CapacityError> {
        self.0.try_push_slice(value.as_bytes())
    }

    /// Appends text and returns its byte range.
    ///
    /// # Panics
    /// Panics if the text exceeds the remaining capacity.
    pub fn push_str(&mut self, value: &str) -> Range<usize> {
        self.try_push_str(value)
            .expect("FixedStr capacity exceeded")
    }

    /// Appends one Unicode scalar and returns its UTF-8 byte range.
    pub fn try_push(&mut self, value: char) -> Result<Range<usize>, CapacityError> {
        let mut bytes = [0; 4];
        self.try_push_str(value.encode_utf8(&mut bytes))
    }

    /// Shortens the string to `new_len` UTF-8 bytes.
    ///
    /// # Panics
    /// Panics unless `new_len` is a character boundary within the string.
    pub fn truncate(&mut self, new_len: usize) {
        assert!(self.as_str().is_char_boundary(new_len));
        self.0.truncate(new_len);
    }

    /// Converts the string into its fixed byte buffer.
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

/// Fixed-width, NUL-padded UTF-16 code units in a specified byte order.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedUtf16<const N: usize, E: Endianness> {
    data: [[u8; 2]; N],
    byte_order: PhantomData<E>,
}

/// Fixed-width little-endian UTF-16 text.
pub type FixedUtf16Le<const N: usize> = FixedUtf16<N, LittleEndian>;
/// Fixed-width big-endian UTF-16 text.
pub type FixedUtf16Be<const N: usize> = FixedUtf16<N, BigEndian>;

impl<const N: usize, E: Endianness> FixedUtf16<N, E> {
    /// Creates a NUL-filled value.
    pub const fn new() -> Self {
        Self {
            data: [[0; 2]; N],
            byte_order: PhantomData,
        }
    }

    /// Encodes a string into fixed-width UTF-16.
    pub fn try_from_str(value: &str) -> Result<Self, CapacityError> {
        let mut result = Self::new();
        for (index, unit) in value.encode_utf16().enumerate() {
            if index == N {
                return Err(CapacityError);
            }
            result.data[index] = {
                let mut bytes = [0; 2];
                E::set_u16(unit, &mut bytes);
                bytes
            };
        }
        Ok(result)
    }

    /// Returns all encoded code-unit bytes, including NUL padding.
    pub fn as_bytes(&self) -> &[[u8; 2]; N] {
        &self.data
    }

    /// Decodes code units up to the first NUL.
    pub fn decode(&self) -> impl Iterator<Item = Result<char, core::char::DecodeUtf16Error>> + '_ {
        char::decode_utf16(
            self.data
                .iter()
                .map(|bytes| E::get_u16(*bytes))
                .take_while(|unit| *unit != 0),
        )
    }

    /// Decodes the value into an allocated UTF-8 string.
    #[cfg(feature = "alloc")]
    pub fn to_string(&self) -> Result<alloc::string::String, core::char::DecodeUtf16Error> {
        self.decode().collect()
    }
}

impl<const N: usize, E: Endianness> Default for FixedUtf16<N, E> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "bytemuck")]
unsafe impl<const N: usize, E: Endianness + 'static> bytemuck::Zeroable for FixedUtf16<N, E> {}
#[cfg(feature = "bytemuck")]
unsafe impl<const N: usize, E: Endianness + 'static> bytemuck::Pod for FixedUtf16<N, E> {}

/// A fixed-capacity vector.
#[derive(Debug)]
pub struct ArrayVec<T, const N: usize> {
    inner: heapless::Vec<T, N>,
}

impl<T, const N: usize> ArrayVec<T, N> {
    /// Creates an empty vector.
    pub const fn new() -> Self {
        Self {
            inner: heapless::Vec::new(),
        }
    }

    /// Appends a value, returning an error when the vector is full.
    pub fn try_push(&mut self, value: T) -> Result<(), CapacityError> {
        self.inner.push(value).map_err(|_| CapacityError)
    }

    /// Appends a value.
    ///
    /// # Panics
    ///
    /// Panics when the vector is full.
    pub fn push(&mut self, value: T) {
        self.try_push(value).expect("ArrayVec: ran out of capacity");
    }

    /// Returns the number of stored values.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns whether the vector is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns the stored values as a slice.
    pub fn as_slice(&self) -> &[T] {
        self.inner.as_slice()
    }

    /// Returns the stored values as a mutable slice.
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        self.inner.as_mut_slice()
    }

    /// Returns an iterator over the stored values.
    pub fn iter(&self) -> core::slice::Iter<'_, T> {
        self.inner.iter()
    }

    /// Returns a mutable iterator over the stored values.
    pub fn iter_mut(&mut self) -> core::slice::IterMut<'_, T> {
        self.inner.iter_mut()
    }

    /// Reverses the stored values.
    pub fn reverse(&mut self) {
        self.inner.reverse();
    }

    /// Returns a pointer to the vector's storage.
    pub fn as_ptr(&self) -> *const T {
        self.inner.as_ptr()
    }

    /// Returns a mutable pointer to the vector's storage.
    pub fn as_mut_ptr(&mut self) -> *mut T {
        self.inner.as_mut_ptr()
    }
}

impl<T, const N: usize> Default for ArrayVec<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const N: usize> ArrayVec<T, N>
where
    T: Copy + PartialEq,
{
    /// Returns whether the vector contains the given value.
    pub fn contains(&self, value: &T) -> bool {
        self.inner.contains(value)
    }
}

impl<T, const N: usize> ArrayVec<T, N>
where
    T: Copy + Ord,
{
    /// Sorts the vector without preserving the order of equal values.
    pub fn sort_unstable(&mut self) {
        self.inner.sort_unstable();
    }
}

impl<T, const N: usize> Index<usize> for ArrayVec<T, N>
where
    T: Copy,
{
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        &self.inner[index]
    }
}

impl<T, const N: usize> IndexMut<usize> for ArrayVec<T, N>
where
    T: Copy,
{
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.inner[index]
    }
}

/// A fixed-capacity FIFO ring buffer.
///
/// A buffer with storage size `N` can hold at most `N - 1` values.
#[derive(Clone, Copy)]
pub struct RingBuf<T: Copy, const N: usize> {
    buf: [Option<T>; N],
    head: usize,
    tail: usize,
}

impl<T: Copy, const N: usize> RingBuf<T, N> {
    /// The number of storage slots in the buffer.
    pub const SIZE: usize = N;

    /// Creates an empty ring buffer.
    pub const fn new() -> Self {
        Self {
            buf: [None; N],
            head: 0,
            tail: 0,
        }
    }

    /// Returns whether the buffer is empty.
    pub const fn is_empty(&self) -> bool {
        self.head == self.tail
    }

    /// Returns the number of stored values.
    pub const fn len(&self) -> usize {
        (self.head + Self::SIZE - self.tail) % Self::SIZE
    }

    /// Returns whether the buffer is full.
    pub const fn is_full(&self) -> bool {
        (self.head + 1) % N == self.tail
    }

    /// Returns the maximum number of values the buffer can hold.
    pub const fn max_capacity(&self) -> usize {
        Self::SIZE - 1
    }

    /// Appends a value, returning it when the buffer is full.
    pub fn try_push(&mut self, value: T) -> Result<(), T> {
        if self.is_full() {
            return Err(value);
        }

        // SAFETY: The buffer was checked for remaining capacity.
        unsafe { self.push_unchecked(value) };
        Ok(())
    }

    /// Appends a value.
    ///
    /// # Panics
    ///
    /// Panics when the buffer is full.
    pub fn push(&mut self, value: T) {
        if self.try_push(value).is_err() {
            panic!("ringbuf is full");
        }
    }

    /// Appends a value without checking whether the buffer is full.
    ///
    /// # Safety
    ///
    /// The caller must ensure the buffer is not full.
    pub unsafe fn push_unchecked(&mut self, value: T) {
        self.buf[self.head] = Some(value);
        self.head = (self.head + 1) % N;
    }

    /// Removes and returns the oldest value.
    pub fn pop(&mut self) -> Option<T> {
        if self.is_empty() {
            return None;
        }

        let value = self.buf[self.tail].take();
        self.tail = (self.tail + 1) % N;
        value
    }
}

impl<T: Copy, const N: usize> Default for RingBuf<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicUsize, Ordering};

    static_assertions::assert_impl_all!(RingBuf<u8, 4>: Clone, Copy);

    #[test]
    fn array_vec_preserves_values_and_capacity_errors() {
        let mut values = ArrayVec::<u8, 2>::new();
        values.push(2);
        values.push(1);
        assert!(matches!(values.try_push(3), Err(CapacityError)));
        values.sort_unstable();
        assert_eq!(values.as_slice(), &[1, 2]);
    }

    #[test]
    fn array_vec_drops_stored_values() {
        static DROPS: AtomicUsize = AtomicUsize::new(0);

        struct DropCounter;

        impl Drop for DropCounter {
            fn drop(&mut self) {
                DROPS.fetch_add(1, Ordering::Relaxed);
            }
        }

        DROPS.store(0, Ordering::Relaxed);
        {
            let mut values = ArrayVec::<DropCounter, 2>::new();
            values.push(DropCounter);
            values.push(DropCounter);
        }
        assert_eq!(DROPS.load(Ordering::Relaxed), 2);
    }

    #[test]
    #[should_panic]
    fn array_vec_rejects_uninitialized_index() {
        let mut values = ArrayVec::<u8, 2>::new();
        values.push(1);
        let _ = values[1];
    }

    #[test]
    fn ring_buffer_wraps_and_preserves_fifo_order() {
        let mut values = RingBuf::<u8, 4>::new();
        values.push(1);
        values.push(2);
        values.push(3);
        assert!(values.is_full());
        assert_eq!(values.try_push(4), Err(4));
        assert_eq!(values.pop(), Some(1));
        values.push(4);
        assert_eq!(values.pop(), Some(2));
        assert_eq!(values.pop(), Some(3));
        assert_eq!(values.pop(), Some(4));
        assert_eq!(values.pop(), None);
    }

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
    fn utf16_encodes_in_requested_byte_order() {
        let le = FixedUtf16Le::<2>::try_from_str("A").unwrap();
        let be = FixedUtf16Be::<2>::try_from_str("A").unwrap();
        assert_eq!(le.as_bytes(), &[[0x41, 0], [0, 0]]);
        assert_eq!(be.as_bytes(), &[[0, 0x41], [0, 0]]);
        assert!(FixedUtf16Le::<1>::try_from_str("AB").is_err());
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn utf16_round_trips_both_orders() {
        let le = FixedUtf16Le::<8>::try_from_str("A😀").unwrap();
        let be = FixedUtf16Be::<8>::try_from_str("A😀").unwrap();
        assert_eq!(le.to_string().unwrap(), "A😀");
        assert_eq!(be.to_string().unwrap(), "A😀");
    }
}
