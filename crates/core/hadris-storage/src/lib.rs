//! Format-neutral block-storage interfaces for Hadris.
//!
//! This crate describes storage in logical blocks with an explicit, non-zero
//! block size. It intentionally does not define filesystem concepts such as FAT
//! clusters or ISO logical sectors.
//!
//! Every device reports its own error through
//! [`hadris_io::ErrorType`]. Writes return [`WriteError`], whose
//! [`ReadOnly`](WriteError::ReadOnly) variant is how a device refuses writes;
//! there is no separate query. Adapters that can refuse a request themselves
//! report [`StorageError`].

#![no_std]
#![allow(async_fn_in_trait)]
#![deny(missing_docs)]

#[cfg(feature = "alloc")]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

#[cfg(feature = "alloc")]
mod cache;
mod device;
mod error;
mod geometry;
mod scratch;

#[cfg(feature = "async")]
/// Asynchronous adapters.
pub mod r#async;
#[cfg(feature = "async-send")]
/// Asynchronous adapters whose futures are `Send`, generated from the same
/// source as `r#async`. `BlockDevice` has `Send` as a supertrait here, so
/// `D: BlockDevice` alone proves a device's futures `Send`.
pub mod async_send;
#[cfg(feature = "sync")]
/// Synchronous adapters.
///
/// ```rust
/// use hadris_storage::{BlockIndex, BlockSize, MemDevice, WriteError};
/// use hadris_storage::sync::BlockDevice;
///
/// let image = [7u8; 2048];
/// let mut device = MemDevice::new(&image[..], BlockSize::new(512).unwrap());
/// let mut block = [0u8; 512];
/// device.read_blocks(BlockIndex::new(3), &mut block).unwrap();
/// assert_eq!(block, [7; 512]);
/// assert_eq!(device.write_blocks(BlockIndex::new(0), &block), Err(WriteError::ReadOnly));
/// ```
pub mod sync;

pub use device::{MemBuffer, MemDevice, ReadOnly};
pub use error::{OutOfRange, StorageError, WriteError};
pub use geometry::{BlockCount, BlockGeometry, BlockIndex, BlockRange, BlockSize};
