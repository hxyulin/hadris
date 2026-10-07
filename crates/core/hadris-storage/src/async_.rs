//! Async block-device contracts enabled together by the `async` feature.
//!
//! Implement [`BlockDevice`](crate::async_::BlockDevice) for potentially non-Send operations, or
//! [`SendBlockDevice`](crate::async_::SendBlockDevice) for Send operations. Send implementations also satisfy
//! the common contract. Existing Send adapters are reexported here;
//! [`Local`](crate::async_::Local) adapts existing local devices and adapter chains.
//!
//! A device being Send does not guarantee that its operation futures are Send.
//! Generic callers requiring Send futures must use [`SendBlockDevice`](crate::async_::SendBlockDevice).
//!
//! ```rust
//! use core::future::Future;
//! use hadris_storage::{BlockIndex, async_::SendBlockDevice};
//!
//! fn require_send(_: impl Future + Send) {}
//! fn check<D: SendBlockDevice>(device: &mut D, buf: &mut [u8]) {
//!     require_send(device.read_blocks(BlockIndex::new(0), buf));
//! }
//! ```
//!
//! A Send device value with only the common contract does not suffice:
//!
//! ```compile_fail
//! use core::future::Future;
//! use hadris_storage::{BlockIndex, async_::BlockDevice};
//!
//! fn require_send(_: impl Future + Send) {}
//! fn check<D: BlockDevice + Send>(device: &mut D, buf: &mut [u8]) {
//!     require_send(device.read_blocks(BlockIndex::new(0), buf));
//! }
//! ```

use crate::{BlockIndex, BlockSize};
use hadris_io::{Error, ErrorKind, ErrorType};

pub use crate::r#async::{BlockDevice as SendBlockDevice, ByteView, StreamDevice, StreamWrite};
#[cfg(feature = "alloc")]
pub use crate::r#async::{Cache, ReadAhead};

/// A device addressed in whole logical blocks, with potentially non-Send futures.
///
/// Buffers must contain a whole number of blocks. Reads must lie within
/// `block_count`, and writes within `max_block_count`. Implementations accept
/// any buffer address and adapt hardware alignment and transfer-size limits.
/// A failed write may have transferred some blocks; errors do not promise
/// rollback. Async implementations must finish or stop hardware access to
/// borrowed buffers before returning or when their future is dropped.
pub trait BlockDevice: ErrorType {
    /// Size of one block.
    fn block_size(&self) -> BlockSize;

    /// Number of addressable blocks.
    fn block_count(&self) -> u64;

    /// Number of blocks the device can hold when written past its end.
    ///
    /// A device that grows on write, such as a `Vec<u8>` or a host image
    /// file, reports more than [`block_count`](Self::block_count), which it
    /// defaults to. A writer checks the size it plans against this before
    /// writing anything.
    fn max_block_count(&self) -> u64 {
        self.block_count()
    }

    /// Byte offset of block 0 within the disk this device is a window of.
    ///
    /// 0 unless the device is a window of another, such as a
    /// [`crate::Partition`], which adds its offset.
    fn disk_offset(&self) -> u64 {
        0
    }

    /// Whether the device accepts writes at all.
    ///
    /// False unless overridden. A driver mounts a device that says false
    /// read-only. A device that says true may still refuse a later write
    /// with [`ErrorKind::ReadOnly`], for example when media are
    /// write-protected while mounted.
    fn writable(&self) -> bool {
        false
    }

    /// Reads whole blocks starting at `first`.
    async fn read_blocks(
        &mut self,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Result<(), Error<Self::Error>>;

    /// Writes whole blocks starting at `first`.
    ///
    /// A device that refuses writes returns [`ErrorKind::ReadOnly`].
    async fn write_blocks(
        &mut self,
        first: BlockIndex,
        buf: &[u8],
    ) -> Result<(), Error<Self::Error>> {
        let _ = (first, buf);
        Err(Error::new(ErrorKind::ReadOnly, "device is read-only"))
    }

    /// Makes earlier writes durable on the underlying storage.
    ///
    /// A write-back device writes here, so it can report
    /// [`ErrorKind::ReadOnly`] too.
    async fn flush(&mut self) -> Result<(), Error<Self::Error>> {
        Ok(())
    }
}

impl<D: SendBlockDevice + ?Sized> BlockDevice for D {
    fn block_size(&self) -> BlockSize {
        SendBlockDevice::block_size(self)
    }
    fn block_count(&self) -> u64 {
        SendBlockDevice::block_count(self)
    }
    fn max_block_count(&self) -> u64 {
        SendBlockDevice::max_block_count(self)
    }
    fn disk_offset(&self) -> u64 {
        SendBlockDevice::disk_offset(self)
    }
    fn writable(&self) -> bool {
        SendBlockDevice::writable(self)
    }
    async fn read_blocks(
        &mut self,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Result<(), Error<Self::Error>> {
        SendBlockDevice::read_blocks(self, first, buf).await
    }
    async fn write_blocks(
        &mut self,
        first: BlockIndex,
        buf: &[u8],
    ) -> Result<(), Error<Self::Error>> {
        SendBlockDevice::write_blocks(self, first, buf).await
    }
    async fn flush(&mut self) -> Result<(), Error<Self::Error>> {
        SendBlockDevice::flush(self).await
    }
}

/// Explicitly adapts a local device, including borrowed and partition devices.
/// Wrap after constructing existing storage adapters to preserve their APIs.
#[derive(Debug)]
#[non_exhaustive]
pub struct Local<D>(D);

impl<D> Local<D> {
    /// Wraps a device whose I/O futures need not be Send.
    pub fn new(device: D) -> Self {
        Self(device)
    }
    /// Returns the device and any adapter state.
    pub fn into_inner(self) -> D {
        self.0
    }
}

impl<D: ErrorType> ErrorType for Local<D> {
    type Error = D::Error;
}

impl<D: super::local::BlockDevice> BlockDevice for Local<D> {
    fn block_size(&self) -> BlockSize {
        super::local::BlockDevice::block_size(&self.0)
    }
    fn block_count(&self) -> u64 {
        super::local::BlockDevice::block_count(&self.0)
    }
    fn max_block_count(&self) -> u64 {
        super::local::BlockDevice::max_block_count(&self.0)
    }
    fn disk_offset(&self) -> u64 {
        super::local::BlockDevice::disk_offset(&self.0)
    }
    fn writable(&self) -> bool {
        super::local::BlockDevice::writable(&self.0)
    }
    async fn read_blocks(
        &mut self,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Result<(), Error<Self::Error>> {
        super::local::BlockDevice::read_blocks(&mut self.0, first, buf).await
    }
    async fn write_blocks(
        &mut self,
        first: BlockIndex,
        buf: &[u8],
    ) -> Result<(), Error<Self::Error>> {
        super::local::BlockDevice::write_blocks(&mut self.0, first, buf).await
    }
    async fn flush(&mut self) -> Result<(), Error<Self::Error>> {
        super::local::BlockDevice::flush(&mut self.0).await
    }
}
