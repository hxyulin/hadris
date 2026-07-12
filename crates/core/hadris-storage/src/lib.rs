//! Format-neutral block-storage interfaces for Hadris.
//!
//! This crate describes storage in logical blocks with an explicit, non-zero
//! block size. It intentionally does not define filesystem concepts such as FAT
//! clusters or ISO logical sectors.

#![no_std]
#![allow(async_fn_in_trait)]

#[cfg(feature = "std")]
extern crate std;

mod error;
mod geometry;
mod view;

#[cfg(feature = "async")]
pub mod r#async;
#[cfg(feature = "sync")]
pub mod sync;

pub use error::{BlockError, Result};
pub use geometry::{BlockCount, BlockGeometry, BlockIndex, BlockRange, BlockSize};
pub use view::PartitionView;

#[cfg(feature = "sync")]
pub use sync::{BlockDevice, BlockDeviceMut, SeekBlockDevice};
