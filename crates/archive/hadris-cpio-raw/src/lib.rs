//! On-disk layouts and I/O-free codecs for newc, crc, odc and binary cpio.
//!
//! This crate needs neither an allocator nor an I/O mode. The filesystem
//! driver uses these same types and preserves its existing `raw` paths.

#![no_std]
#![deny(missing_docs)]

pub mod raw;

pub use raw::*;
