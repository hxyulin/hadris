//! Optical-media entry point for Hadris.
//!
//! Concrete format crates remain directly accessible so callers do not lose
//! format-specific functionality.
//! Optical detection reports ISO 9660 and UDF independently because bridge
//! images may validly contain both filesystems.

#![no_std]
#![deny(missing_docs)]

#[cfg(feature = "std")]
extern crate std;

#[cfg(feature = "detect")]
pub mod detect;
#[cfg(feature = "open")]
mod error;
#[cfg(feature = "open")]
mod image;

#[cfg(feature = "open")]
pub use error::{Error, OpticalFormat, Result};
#[cfg(feature = "open")]
pub use image::OpenPolicy;

#[cfg(all(feature = "open", feature = "sync"))]
#[path = "image_sync.rs"]
/// Synchronous optical-image detection and opening.
pub mod sync;

#[cfg(all(feature = "open", feature = "async"))]
#[path = "image_async.rs"]
/// Asynchronous optical-image detection and opening.
pub mod r#async;

/// ISO 9660 filesystem support.
#[cfg(feature = "iso")]
pub use hadris_iso as iso;

/// Universal Disk Format filesystem support.
#[cfg(feature = "udf")]
pub use hadris_udf as udf;

/// Hybrid ISO+UDF optical-disc image creation.
#[cfg(feature = "cd")]
pub use hadris_cd as cd;
