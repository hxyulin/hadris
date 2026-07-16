//! Block-storage entry point for Hadris.
//!
//! This crate groups format-neutral block-device interfaces, partition tables,
//! and block filesystems without hiding their concrete APIs.

#![no_std]

#[cfg(feature = "std")]
extern crate std;

#[cfg(feature = "detect")]
pub mod detect;
#[cfg(all(feature = "detect", feature = "fat"))]
mod error;
#[cfg(all(feature = "detect", feature = "part", feature = "storage"))]
pub mod partition;

#[cfg(all(feature = "detect", feature = "fat"))]
pub use error::{Error, Result};

#[cfg(all(feature = "detect", feature = "fat", feature = "sync"))]
#[path = "volume_sync.rs"]
pub mod sync;

#[cfg(all(feature = "detect", feature = "fat", feature = "async"))]
#[path = "volume_async.rs"]
pub mod r#async;

/// Format-neutral block geometry and device capabilities.
#[cfg(feature = "storage")]
pub use hadris_storage as storage;

/// FAT12/16/32 filesystem support and exFAT format detection.
///
/// The unified volume opener supports FAT12/16/32. The leaf `hadris-fat`
/// crate carries a separate unstable exFAT preview.
#[cfg(feature = "fat")]
pub use hadris_fat as fat;

/// MBR, GPT, and hybrid partition-table support.
#[cfg(feature = "part")]
pub use hadris_part as part;
