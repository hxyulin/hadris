use crate::{BlockIndex, BlockSize};
use hadris_io::{Error, ErrorKind, Result};

/// Whether a device accepts writes.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Access {
    /// Only reads are supported.
    ReadOnly,
    /// Reads and writes are supported.
    ReadWrite,
}

impl Access {
    /// Returns whether writes are supported.
    pub const fn is_writable(self) -> bool {
        matches!(self, Self::ReadWrite)
    }
}

/// Byte storage backing a [`MemDevice`].
///
/// Implemented for `&[u8]` (read-only), `&mut [u8]`, `[u8; N]` and, with
/// `alloc`, `Vec<u8>` and `Box<[u8]>`.
pub trait MemBuffer {
    /// Whether [`bytes_mut`](Self::bytes_mut) returns `Some`.
    const WRITABLE: bool;

    /// Borrow the bytes.
    fn bytes(&self) -> &[u8];

    /// Mutably borrow the bytes, or `None` when the buffer is read-only.
    fn bytes_mut(&mut self) -> Option<&mut [u8]>;
}

impl MemBuffer for &[u8] {
    const WRITABLE: bool = false;

    fn bytes(&self) -> &[u8] {
        self
    }

    fn bytes_mut(&mut self) -> Option<&mut [u8]> {
        None
    }
}

impl MemBuffer for &mut [u8] {
    const WRITABLE: bool = true;

    fn bytes(&self) -> &[u8] {
        self
    }

    fn bytes_mut(&mut self) -> Option<&mut [u8]> {
        Some(self)
    }
}

impl<const N: usize> MemBuffer for [u8; N] {
    const WRITABLE: bool = true;

    fn bytes(&self) -> &[u8] {
        self
    }

    fn bytes_mut(&mut self) -> Option<&mut [u8]> {
        Some(self)
    }
}

#[cfg(feature = "alloc")]
impl MemBuffer for alloc::vec::Vec<u8> {
    const WRITABLE: bool = true;

    fn bytes(&self) -> &[u8] {
        self
    }

    fn bytes_mut(&mut self) -> Option<&mut [u8]> {
        Some(self)
    }
}

#[cfg(feature = "alloc")]
impl MemBuffer for alloc::boxed::Box<[u8]> {
    const WRITABLE: bool = true;

    fn bytes(&self) -> &[u8] {
        self
    }

    fn bytes_mut(&mut self) -> Option<&mut [u8]> {
        Some(self)
    }
}

/// A block device over memory.
///
/// The block count is the buffer length divided by the block size. Trailing
/// bytes that do not fill a block are not addressable.
#[derive(Debug, Clone)]
pub struct MemDevice<B> {
    pub(crate) buffer: B,
    pub(crate) block_size: BlockSize,
}

impl<B: MemBuffer> MemDevice<B> {
    /// Wrap `buffer` as a device with `block_size` blocks.
    pub fn new(buffer: B, block_size: BlockSize) -> Self {
        Self { buffer, block_size }
    }

    /// Recover the buffer.
    pub fn into_inner(self) -> B {
        self.buffer
    }

    /// Borrow the buffer.
    pub fn get_ref(&self) -> &B {
        &self.buffer
    }

    /// Mutably borrow the buffer.
    pub fn get_mut(&mut self) -> &mut B {
        &mut self.buffer
    }

    #[cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]
    pub(crate) fn block_count(&self) -> u64 {
        self.buffer.bytes().len() as u64 / u64::from(self.block_size.get())
    }

    #[cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]
    pub(crate) fn range(&self, first: BlockIndex, len: usize) -> Result<core::ops::Range<usize>> {
        check_blocks(self.block_size, self.block_count(), first, len)?;
        let start = (first.0 * u64::from(self.block_size.get())) as usize;
        Ok(start..start + len)
    }
}

/// Marks a stream as read-only for `StreamDevice`.
///
/// A `StreamDevice` over `ReadOnly<T>` reports [`Access::ReadOnly`] and needs
/// only `T: Read + Seek`.
#[derive(Debug, Clone, Default)]
pub struct ReadOnly<T>(pub(crate) T);

impl<T> ReadOnly<T> {
    /// Wrap a stream.
    pub const fn new(inner: T) -> Self {
        Self(inner)
    }

    /// Recover the stream.
    pub fn into_inner(self) -> T {
        self.0
    }

    /// Borrow the stream.
    pub const fn get_ref(&self) -> &T {
        &self.0
    }

    /// Mutably borrow the stream.
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

#[cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]
pub(crate) fn check_blocks(
    block_size: BlockSize,
    block_count: u64,
    first: BlockIndex,
    len: usize,
) -> Result<u64> {
    let size = u64::from(block_size.get());
    if len as u64 % size != 0 {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "buffer is not a whole number of blocks",
        ));
    }
    let count = len as u64 / size;
    match first.0.checked_add(count) {
        Some(end) if end <= block_count => Ok(count),
        _ => Err(Error::new(
            ErrorKind::InvalidInput,
            "block request is out of range",
        )),
    }
}

#[cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]
pub(crate) fn read_only() -> Error {
    Error::new(ErrorKind::Unsupported, "device is read-only")
}

#[cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]
pub(crate) fn byte_offset(block_size: BlockSize, first: BlockIndex) -> Result<u64> {
    first
        .0
        .checked_mul(u64::from(block_size.get()))
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "block offset overflows"))
}
