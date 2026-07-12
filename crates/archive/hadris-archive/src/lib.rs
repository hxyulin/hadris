//! Sequential-archive entry point for Hadris.

#![no_std]

/// CPIO newc/CRC archive support.
#[cfg(feature = "cpio")]
pub use hadris_cpio as cpio;
