//! Format-neutral block-storage interfaces for Hadris.
//!
//! This crate describes storage in logical blocks with an explicit, non-zero
//! block size. It intentionally does not define filesystem concepts such as FAT
//! clusters or ISO logical sectors.

#![no_std]
#![allow(async_fn_in_trait)]
#![deny(missing_docs)]

#[cfg(feature = "std")]
extern crate std;

mod geometry;
mod view;

#[cfg(feature = "async")]
/// Asynchronous adapters.
pub mod r#async;
#[cfg(feature = "sync")]
/// Synchronous adapters.
pub mod sync;

pub use geometry::{BlockCount, BlockGeometry, BlockIndex, BlockRange, BlockSize};
pub use hadris_io::{Error, Result};
pub use view::PartitionView;
