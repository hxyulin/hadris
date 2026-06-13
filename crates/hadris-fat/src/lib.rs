//! # hadris-fat
//!
//! A `no_std`-compatible library for reading and writing FAT filesystems
//! (FAT12, FAT16, FAT32) with optional exFAT support.
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use std::fs::File;
//! use hadris_fat::sync::FatFs;
//!
//! let file = File::open("disk.img").unwrap();
//! let fs = FatFs::open(file).unwrap();
//! let root = fs.root_dir();
//! let mut iter = root.entries();
//! while let Some(Ok(entry)) = iter.next_entry() {
//!     println!("{}", entry.name());
//! }
//! ```
//!
//! ## Builder: custom providers and FAT caching
//!
//! [`FatFs::builder`] configures the clock and
//! OEM-codepage providers — and, with the `cache` feature, an LRU FAT-sector
//! cache — before mounting:
//!
//! ```rust,no_run
//! use hadris_fat::sync::FatFs;
//!
//! let file = std::fs::File::open("disk.img").unwrap();
//! let fs = FatFs::builder(file).open().unwrap();
//! # let _ = fs;
//! ```
//!
//! With the `cache` feature, chain `.with_fat_cache(capacity_sectors)` before
//! `.open()` to back FAT reads and writes with an LRU cache. The cache is
//! sync-only: under the async API it is silently bypassed. See
//! [`FatFsBuilder`].
//!
//! ## Feature Flags
//!
//! | Feature  | Default | Description |
//! |----------|---------|-------------|
//! | `std`    | Yes     | Standard library support (enables `alloc`, `sync`, chrono clock) |
//! | `alloc`  | No      | Heap allocation without full std |
//! | `sync`   | No      | Synchronous API via `hadris-io` sync traits |
//! | `async`  | No      | Asynchronous API via `hadris-io` async traits |
//! | `read`   | Yes     | Read operations |
//! | `write`  | Yes     | Write operations (requires `alloc` + `read`) |
//! | `lfn`    | Yes     | Long filename (VFAT) support |
//! | `cache`  | No      | FAT sector caching for reduced I/O |
//! | `tool`   | No      | Analysis and diagnostic utilities |
//! | `exfat`  | No      | exFAT filesystem support (WIP) |
//!
//! ## Dual Sync/Async Architecture
//!
//! This crate provides both synchronous and asynchronous APIs through
//! a compile-time code transformation system. The same implementation
//! source is compiled twice:
//!
//! - **`sync`** module: synchronous API (enabled by `sync` or `std` feature)
//! - **`async`** module: asynchronous API (enabled by `async` feature)
//!
//! When the `std` feature is enabled (default), the synchronous API types
//! are re-exported at the crate root for convenience.
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

#![no_std]
#![allow(async_fn_in_trait)]

#[cfg(feature = "std")]
extern crate std;

#[cfg(feature = "alloc")]
extern crate alloc;

// ---------------------------------------------------------------------------
// Shared types (compiled once, not duplicated by sync/async modules)
// ---------------------------------------------------------------------------

pub mod error;
pub mod file;
pub mod oem;
pub mod raw;
pub mod time;

// ExFAT (WIP, stays at crate root for now)
#[cfg(feature = "exfat")]
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
        pub mod dir;
        pub mod fat_table;
        #[cfg(feature = "write")]
        pub mod format;
        pub mod fs;
        pub mod io;
        pub mod read;
        #[cfg(feature = "tool")]
        pub mod tool;
        pub mod write;
    }
    pub use __inner::*;

    // Convenience re-exports for backwards compatibility
    #[cfg(feature = "write")]
    pub use crate::time::FatDateTime;
    pub use __inner::dir::{DirectoryEntry, FatDir, FileEntry};
    pub use __inner::fat_table::{Fat, Fat12, Fat16, Fat32, FatType};
    pub use __inner::fs::{FatFs, FatFsBuilder};
    pub use __inner::read::FatFsReadExt;
    #[cfg(feature = "tool")]
    pub use __inner::tool::analysis::FatAnalysisExt;
    #[cfg(feature = "tool")]
    pub use __inner::tool::verify::FatVerifyExt;
    #[cfg(feature = "write")]
    pub use __inner::write::FatFsWriteExt;
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
        // feature is gated to `sync` in Cargo.toml so this combination is
        // unreachable. Async-aware caching is deferred to phase C5b.
        pub mod dir;
        pub mod fat_table;
        #[cfg(feature = "write")]
        pub mod format;
        pub mod fs;
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
    pub use __inner::*;
}

// ---------------------------------------------------------------------------
// Default re-exports for backwards compatibility (sync)
// ---------------------------------------------------------------------------

#[cfg(feature = "sync")]
pub use sync::*;

// Re-exports from shared types
pub use error::{FatError, Result};
pub use raw::*;
