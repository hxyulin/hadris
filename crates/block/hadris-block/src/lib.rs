//! Block devices, partition tables, and FAT filesystems for the Hadris Rust
//! storage stack.
//!
//! This `no_std`-compatible facade groups format-neutral sector and block-device
//! interfaces, GPT and MBR partition tables, filesystem detection, and
//! FAT12/16/32 without hiding their concrete APIs.

#![no_std]
#![deny(missing_docs)]

#[cfg(feature = "std")]
extern crate std;

#[cfg(feature = "detect")]
/// Lightweight, non-destructive block-format detection.
pub mod detect;
#[cfg(all(feature = "detect", feature = "fat"))]
mod error;
#[cfg(all(feature = "detect", feature = "part", feature = "storage"))]
/// Checked partition views for opening filesystems inside partitioned disks.
pub mod partition;

#[cfg(all(feature = "detect", feature = "fat"))]
pub use error::{Error, Result};

#[cfg(all(feature = "detect", feature = "fat", feature = "sync"))]
#[path = "volume_sync.rs"]
/// Synchronous detection and unified volume opening.
pub mod sync;

#[cfg(all(feature = "detect", feature = "fat", feature = "async"))]
#[path = "volume_async.rs"]
/// Asynchronous detection and unified volume opening.
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
