//! # hadris-fat
//!
//! A pure Rust, `no_std`-compatible library for reading, writing, and formatting
//! FAT12, FAT16, and FAT32 filesystems, plus an opt-in unstable exFAT preview.
//! It is suitable for disk-image tools, bootloaders, kernels, firmware,
//! embedded devices, SD cards, and USB drives.
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use std::fs::File;
//! use hadris_fat::sync::FatVolume;
//!
//! let file = File::open("disk.img").unwrap();
//! let fs = FatVolume::open(file).unwrap();
//! let root = fs.root_dir();
//! let mut iter = root.entries();
//! while let Some(Ok(entry)) = iter.next_entry() {
//!     println!("{}", entry.name());
//! }
//! ```
//!
//! ## Builder: custom providers and FAT caching
//!
//! [`FatVolume::builder`] configures the clock and
//! OEM-codepage providers — and, with the `cache` feature, an LRU FAT-sector
//! cache — before mounting:
//!
//! ```rust,no_run
//! # #[cfg(feature = "cache")]
//! # {
//! use hadris_fat::sync::FatVolume;
//! use std::fs::OpenOptions;
//!
//! let disk = OpenOptions::new()
//!     .read(true)
//!     .write(true)
//!     .open("disk.img")
//!     .unwrap();
//! let fs = FatVolume::builder(disk)
//!     .fat_cache(16)
//!     .open()
//!     .unwrap();
//!
//! // Normal FatVolume operations use the installed cache transparently.
//! let _root = fs.root_dir();
//!
//! // After cached writes, flush before dropping the volume.
//! fs.flush().unwrap();
//! # }
//! ```
//!
//! Without `cache`, omit `.fat_cache(...)`. A zero capacity also disables the
//! cache. The cache is sync-only; async operations access the FAT directly.
//! See [`FatVolumeBuilder`].
//!
//! ## Feature Flags
//!
//! | Feature  | Default | Description |
//! |----------|---------|-------------|
//! | `std`    | Yes     | Standard library support (enables `alloc` and chrono clock) |
//! | `alloc`  | No      | Heap allocation without full std |
//! | `sync`   | No      | Synchronous API via `hadris-io` sync traits |
//! | `async`  | No      | Asynchronous API via `hadris-io` async traits |
//! | `read`   | Yes     | Read operations |
//! | `write`  | Yes     | Write operations (requires `alloc` + `read`) |
//! | `lfn`    | Yes     | Long filename (VFAT) support |
//! | `cache`  | No      | FAT sector caching for reduced I/O |
//! | `tool`   | No      | Analysis and diagnostic utilities |
//! | `unstable-exfat` | No | Unstable, sync-only exFAT preview |
//!
//! ## Known Limitations
//!
//! - **async + cache:** The FAT-sector cache is sync-only; async operations
//!   access the FAT directly.
//! - **exFAT:** The `unstable-exfat` preview is outside the V2 API stability
//!   promise and is not recommended for irreplaceable data. It is sync-only
//!   and does not support fragmented allocation bitmap / upcase metadata,
//!   directory growth, general cross-cluster entry-set placement, TexFAT, or
//!   repair workflows. Enable the preview and see the `exfat` module for its
//!   qualified scope.
//!
//! ## Dual Sync/Async Architecture
//!
//! This crate provides both synchronous and asynchronous APIs through
//! a compile-time code transformation system. The same implementation
//! source is compiled twice:
//!
//! - **`sync`** module: synchronous API (enabled by `sync` feature)
//! - **`async`** module: asynchronous API (enabled by `async` feature)
//!
//! `std` does not select an I/O mode. The default feature set enables `sync`
//! explicitly, and synchronous API types are re-exported at the crate root
//! whenever `sync` is enabled.
//!
//! ## Modules
//!
//! - `error` — Error types for FAT operations
//! - `file` — Short filename (8.3) types and validation
//! - `raw` — On-disk structures: boot sector, BPB, directory entries
//! - `sync::fs` — Filesystem handle and metadata
//! - `sync::dir` — Directory iteration and entry types
//! - `sync::read` — Read extension trait for file content
//! - `sync::write` — Write extension trait for file modification
//! - `sync::fat_table` — FAT table access (FAT12/16/32)
//! - `sync::cache` — Optional FAT sector caching
//! - `sync::format` — Filesystem formatting (requires `write`)
//! - `sync::tool` — Analysis and verification (requires `tool`)

#![cfg_attr(not(test), no_std)]
#![deny(missing_docs)]
#![allow(async_fn_in_trait)]
// Sync and async APIs intentionally compile the same source modules twice.
#![allow(clippy::duplicate_mod)]

#[cfg(all(feature = "std", not(test)))]
extern crate std;

#[cfg(test)]
extern crate self as hadris_fat;

#[cfg(feature = "alloc")]
extern crate alloc;

// ---------------------------------------------------------------------------
// Shared types (compiled once, not duplicated by sync/async modules)
// ---------------------------------------------------------------------------

pub mod error;
/// FAT filename types, including 8.3 and long-file-name helpers.
pub mod file;
pub mod oem;
/// Raw on-disk FAT structures and attribute flags.
pub mod raw;
pub mod time;

// Unstable exFAT preview, intentionally outside the sync/async stable surface.
#[cfg(feature = "unstable-exfat")]
pub mod exfat;

// ---------------------------------------------------------------------------
// Sync module
// ---------------------------------------------------------------------------

#[cfg(feature = "sync")]
#[path = ""]
pub mod sync {
    //! Synchronous FAT filesystem API.
    //!
    //! All I/O operations use synchronous `Read`/`Write`/`Seek` traits.

    pub use hadris_io::Result as IoResult;
    pub use hadris_io::sync::{Parsable, Read, ReadExt, Seek, Writable, Write};
    pub use hadris_io::{Error, ErrorKind, SeekFrom};

    macro_rules! io_transform {
        ($($item:tt)*) => { hadris_macros::strip_async!{ $($item)* } };
    }

    #[allow(unused_macros)]
    macro_rules! sync_only {
        ($($item:tt)*) => { $($item)* };
    }

    #[allow(unused_macros)]
    macro_rules! async_only {
        ($($item:tt)*) => {};
    }

    #[path = "."]
    mod __inner {
        #[cfg(feature = "cache")]
        pub mod cache;
        /// Directory traversal and directory-entry types.
        pub mod dir;
        /// FAT12, FAT16, and FAT32 allocation-table access.
        pub mod fat_table;
        #[cfg(feature = "write")]
        pub mod format;
        /// Mounted FAT filesystem handles and builders.
        pub mod fs;
        /// FAT-specific I/O positioning utilities.
        pub mod io;
        pub mod read;
        #[cfg(feature = "tool")]
        pub mod tool;
        pub mod write;
    }
    pub use __inner::*;

    #[cfg(feature = "write")]
    pub use crate::time::FatDateTime;
    pub use __inner::dir::{DirectoryEntry, FatDir, FileEntry};
    pub use __inner::fat_table::{Fat, Fat12, Fat16, Fat32, FatType};
    pub use __inner::fs::{FatVolume, FatVolumeBuilder};
    pub use __inner::read::FatVolumeReadExt;
    #[cfg(feature = "tool")]
    pub use __inner::tool::analysis::FatAnalysisExt;
    #[cfg(feature = "tool")]
    pub use __inner::tool::verify::FatVerifyExt;
    #[cfg(feature = "write")]
    pub use __inner::write::FatVolumeWriteExt;
}

// ---------------------------------------------------------------------------
// Async module
// ---------------------------------------------------------------------------

#[cfg(feature = "async")]
#[path = ""]
pub mod r#async {
    //! Asynchronous FAT filesystem API.
    //!
    //! All I/O operations use async `Read`/`Write`/`Seek` traits.

    pub use hadris_io::Result as IoResult;
    pub use hadris_io::r#async::{Parsable, Read, ReadExt, Seek, Writable, Write};
    pub use hadris_io::{Error, ErrorKind, SeekFrom};

    macro_rules! io_transform {
        ($($item:tt)*) => { $($item)* };
    }

    #[allow(unused_macros)]
    macro_rules! sync_only {
        ($($item:tt)*) => {};
    }

    #[allow(unused_macros)]
    macro_rules! async_only {
        ($($item:tt)*) => { $($item)* };
    }

    #[path = "."]
    mod __inner {
        // Note: `cache` is intentionally absent here. The cache module uses
        // synchronous I/O traits and is not yet async-aware; the `cache`
        // feature is gated to `sync` in Cargo.toml, so this module never
        // exposes cache APIs.
        /// Directory traversal and directory-entry types.
        pub mod dir;
        /// FAT12, FAT16, and FAT32 allocation-table access.
        pub mod fat_table;
        #[cfg(feature = "write")]
        pub mod format;
        /// Mounted FAT filesystem handles and builders.
        pub mod fs;
        /// FAT-specific I/O positioning utilities.
        pub mod io;
        pub mod read;
        // Note: `tool` is intentionally absent here. The analysis/verify
        // utilities iterate directories synchronously and are not
        // async-aware; the `tool` feature is gated to `sync` in Cargo.toml
        // so this combination is unreachable.
        pub mod write;
    }
    #[cfg(feature = "write")]
    pub use crate::time::FatDateTime;
    pub use __inner::dir::{DirectoryEntry, FatDir, FileEntry};
    pub use __inner::fat_table::{Fat, Fat12, Fat16, Fat32, FatType};
    pub use __inner::fs::{FatVolume, FatVolumeBuilder};
    pub use __inner::read::FatVolumeReadExt;
    #[cfg(feature = "write")]
    pub use __inner::write::FatVolumeWriteExt;
    pub use __inner::*;
}

// ---------------------------------------------------------------------------
// Default re-exports for backwards compatibility (sync)
// ---------------------------------------------------------------------------

#[cfg(feature = "sync")]
pub use sync::*;

// Re-exports from shared types
pub use error::{Error, Result};

#[cfg(all(test, feature = "async", feature = "alloc", feature = "read"))]
#[path = "../tests/async_roundtrip.rs"]
mod async_roundtrip;
#[cfg(all(test, feature = "cache", feature = "write", feature = "std"))]
#[path = "../tests/cache_integration.rs"]
mod cache_integration;
#[cfg(all(test, feature = "sync", feature = "write"))]
#[path = "../tests/comprehensive_fat.rs"]
mod comprehensive_fat;
#[cfg(all(test, feature = "unstable-exfat", feature = "write"))]
#[path = "../tests/exfat_roundtrip.rs"]
mod exfat_roundtrip;
#[cfg(all(test, feature = "sync", feature = "write"))]
#[path = "../tests/fat_roundtrip.rs"]
mod fat_roundtrip;
#[cfg(all(test, feature = "unstable-exfat"))]
#[path = "../tests/integration_exfat.rs"]
mod integration_exfat;
#[cfg(all(test, feature = "sync", feature = "read"))]
#[path = "../tests/poc_audit_fat.rs"]
mod poc_audit_fat;
#[cfg(all(test, feature = "tool"))]
#[path = "../tests/poc_audit_fat_recursion.rs"]
mod poc_audit_fat_recursion;
#[cfg(all(test, feature = "sync", feature = "write"))]
#[path = "../tests/poc_seek.rs"]
mod poc_seek;
#[cfg(all(test, feature = "unstable-exfat", feature = "write", feature = "std"))]
#[path = "../tests/regression_audit_exfat.rs"]
mod regression_audit_exfat;
#[cfg(all(test, feature = "sync", feature = "write", feature = "std"))]
#[path = "../tests/regression_audit_fat.rs"]
mod regression_audit_fat;
#[cfg(all(test, feature = "unstable-exfat"))]
#[path = "../tests/test_exfat.rs"]
mod test_exfat;
#[cfg(all(test, feature = "sync", feature = "read"))]
#[path = "../tests/test_read.rs"]
mod test_read;
#[cfg(all(test, feature = "sync", feature = "write"))]
#[path = "../tests/test_write.rs"]
mod test_write;
#[cfg(all(test, feature = "sync", feature = "read"))]
#[path = "../tests/v2_api.rs"]
mod v2_api;
