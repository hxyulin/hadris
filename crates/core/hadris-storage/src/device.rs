use hadris_io::{Error, ErrorKind, Location};

use crate::{BlockIndex, BlockSize};

/// Byte storage backing a [`MemDevice`].
///
/// Implemented for `&[u8]` (read-only), `&mut [u8]`, `[u8; N]` and, with
/// `alloc`, `Vec<u8>` and `Box<[u8]>`.
pub trait MemBuffer {
    /// Borrow the bytes.
    fn bytes(&self) -> &[u8];

    /// Mutably borrow the bytes, or `None` when the buffer is read-only.
    fn bytes_mut(&mut self) -> Option<&mut [u8]>;

    /// Whether [`bytes_mut`](Self::bytes_mut) returns the bytes. True
    /// unless overridden.
    fn writable(&self) -> bool {
        true
    }
}

impl MemBuffer for &[u8] {
    fn bytes(&self) -> &[u8] {
        self
    }

    fn bytes_mut(&mut self) -> Option<&mut [u8]> {
        None
    }

    fn writable(&self) -> bool {
        false
    }
}

impl MemBuffer for &mut [u8] {
    fn bytes(&self) -> &[u8] {
        self
    }

    fn bytes_mut(&mut self) -> Option<&mut [u8]> {
        Some(self)
    }
}

impl<const N: usize> MemBuffer for [u8; N] {
    fn bytes(&self) -> &[u8] {
        self
    }

    fn bytes_mut(&mut self) -> Option<&mut [u8]> {
        Some(self)
    }
}

#[cfg(feature = "alloc")]
impl MemBuffer for alloc::vec::Vec<u8> {
    fn bytes(&self) -> &[u8] {
        self
    }

    fn bytes_mut(&mut self) -> Option<&mut [u8]> {
        Some(self)
    }
}

#[cfg(feature = "alloc")]
impl MemBuffer for alloc::boxed::Box<[u8]> {
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
    pub(crate) fn range<E>(
        &self,
        first: BlockIndex,
        len: usize,
    ) -> Result<core::ops::Range<usize>, Error<E>> {
        check_blocks(self.block_size, self.block_count(), first, len)?;
        let start = (first.get() * u64::from(self.block_size.get())) as usize;
        Ok(start..start + len)
    }
}

/// A byte window of another device, such as an MBR or GPT partition or the
/// partition of a hybrid ISO.
///
/// Block 0 of the partition is the device block at byte `offset`. `D` can
/// be owned or `&mut`. The offset and length must be multiples of the
/// device block size; a request to a partition that is not, or one past
/// the partition's end, fails with [`ErrorKind::InvalidInput`] and never
/// reaches the device. `disk_offset` adds `offset` to the device's own, and
/// the partition is writable when the device is.
#[derive(Debug, Clone)]
pub struct Partition<D> {
    pub(crate) inner: D,
    offset: u64,
    len: u64,
}

impl<D> Partition<D> {
    /// The `len` bytes of `inner` from byte `offset`.
    pub const fn new(inner: D, offset: u64, len: u64) -> Self {
        Self { inner, offset, len }
    }

    /// Byte offset of the partition on the device.
    pub const fn offset(&self) -> u64 {
        self.offset
    }

    /// Length of the partition in bytes.
    pub const fn len(&self) -> u64 {
        self.len
    }

    /// Whether the partition is empty.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Recovers the device.
    pub fn into_inner(self) -> D {
        self.inner
    }

    /// Borrows the device.
    pub const fn get_ref(&self) -> &D {
        &self.inner
    }

    /// Mutably borrows the device.
    pub fn get_mut(&mut self) -> &mut D {
        &mut self.inner
    }

    /// The device block that holds block `first` of the partition, once the
    /// `len` bytes from there are known to lie within it.
    #[cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]
    pub(crate) fn locate<E>(
        &self,
        block_size: BlockSize,
        first: BlockIndex,
        len: usize,
    ) -> Result<BlockIndex, Error<E>> {
        let size = u64::from(block_size.get());
        if self.offset % size != 0 || self.len % size != 0 {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "partition is not aligned to the device blocks",
            ));
        }
        check_blocks(block_size, self.len / size, first, len)?;
        (self.offset / size)
            .checked_add(first.get())
            .map(BlockIndex::new)
            .ok_or_else(|| out_of_range(first))
    }
}

impl<D: hadris_io::ErrorType> hadris_io::ErrorType for Partition<D> {
    type Error = D::Error;
}

/// Marks a stream as read-only for `StreamDevice`.
///
/// A `StreamDevice` over `ReadOnly<T>` needs only `T: Read + Seek` and answers
/// writes with [`ErrorKind::ReadOnly`].
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

#[cfg(feature = "alloc")]
#[cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]
pub(crate) const BLOCK_512: BlockSize = match BlockSize::new(512) {
    Some(size) => size,
    None => panic!(),
};

#[cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]
pub(crate) fn check_blocks<E>(
    block_size: BlockSize,
    block_count: u64,
    first: BlockIndex,
    len: usize,
) -> Result<u64, Error<E>> {
    let size = u64::from(block_size.get());
    if len as u64 % size != 0 {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "buffer is not a whole number of blocks",
        ));
    }
    let count = len as u64 / size;
    match first.get().checked_add(count) {
        Some(end) if end <= block_count => Ok(count),
        _ => Err(out_of_range(first)),
    }
}

pub(crate) fn out_of_range<E>(first: BlockIndex) -> Error<E> {
    Error::new(
        ErrorKind::InvalidInput,
        "block request past the end of the device",
    )
    .with_location(Location::Block(first.get()))
}

#[cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]
pub(crate) fn byte_offset<E>(block_size: BlockSize, first: BlockIndex) -> Result<u64, Error<E>> {
    first
        .get()
        .checked_mul(u64::from(block_size.get()))
        .ok_or_else(|| out_of_range(first))
}

impl<B> hadris_io::ErrorType for MemDevice<B> {
    type Error = core::convert::Infallible;
}

impl<T: hadris_io::ErrorType> hadris_io::ErrorType for ReadOnly<T> {
    type Error = T::Error;
}
