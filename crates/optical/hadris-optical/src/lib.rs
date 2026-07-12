//! Optical-media entry point for Hadris.
//!
//! Concrete format crates remain directly accessible so callers do not lose
//! format-specific functionality.

#![no_std]

/// ISO 9660 filesystem support.
#[cfg(feature = "iso")]
pub use hadris_iso as iso;

/// Universal Disk Format filesystem support.
#[cfg(feature = "udf")]
pub use hadris_udf as udf;

/// Hybrid ISO+UDF optical-disc image creation.
#[cfg(feature = "cd")]
pub use hadris_cd as cd;
