//! Format-neutral block-storage interfaces for Hadris.
//!
//! This crate describes storage in logical blocks with an explicit, non-zero
//! block size. It intentionally does not define filesystem concepts such as FAT
//! clusters or ISO logical sectors.
//!
//! Every device names its own error through [`hadris_io::ErrorType`], and
//! every block operation returns [`hadris_io::Error`] over it: a device
//! failure is [`Error::device`](hadris_io::Error::device), a device that
//! refuses writes answers [`ErrorKind::ReadOnly`](hadris_io::ErrorKind::ReadOnly),
//! and an adapter that refuses a request itself, such as one past the end of
//! a [`Partition`], answers an [`ErrorKind`](hadris_io::ErrorKind) with the
//! block it concerns. Adapters keep the error type of the device underneath.
//!
//! The device trait exists once per mode: `sync`, `r#async`,
//! `async_send` (futures are `Send`) and `local` (futures need not be
//! `Send`, for single-threaded executors). Mode-independent devices such as
//! [`MemDevice`], [`Partition`] and `Vec<u8>` implement every mode's trait.
//! With `std` and `sync`, `host::FileDevice` is a host image file or disk
//! device.

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
mod geometry;
#[cfg(feature = "std")]
pub mod host;
mod scratch;

#[cfg(feature = "async")]
/// Asynchronous adapters.
pub mod r#async;
#[cfg(feature = "async-send")]
/// Asynchronous adapters whose futures are `Send`, generated from the same
/// source as `r#async`. `BlockDevice` has `Send` as a supertrait here, so
/// `D: BlockDevice` alone proves a device's futures `Send`.
pub mod async_send;
#[cfg(feature = "async")]
/// Asynchronous adapters whose futures need not be `Send`, generated from
/// the same source as `r#async`, for single-threaded executors such as
/// embassy.
pub mod local;
#[cfg(feature = "sync")]
/// Synchronous adapters.
///
/// ```rust
/// use hadris_io::ErrorKind;
/// use hadris_storage::{BlockIndex, BlockSize, MemDevice};
/// use hadris_storage::sync::BlockDevice;
///
/// let image = [7u8; 2048];
/// let mut device = MemDevice::new(&image[..], BlockSize::new(512).unwrap());
/// let mut block = [0u8; 512];
/// device.read_blocks(BlockIndex::new(3), &mut block).unwrap();
/// assert_eq!(block, [7; 512]);
/// let err = device.write_blocks(BlockIndex::new(0), &block).unwrap_err();
/// assert_eq!(err.kind(), ErrorKind::ReadOnly);
/// ```
pub mod sync;

pub use device::{MemBuffer, MemDevice, Partition, ReadOnly};
pub use geometry::{BlockCount, BlockGeometry, BlockIndex, BlockRange, BlockSize};
