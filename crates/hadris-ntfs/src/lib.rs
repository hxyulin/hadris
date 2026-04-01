//! # hadris-ntfs
//!
//! A `no_std`-compatible library for reading NTFS filesystems (read-only).
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use std::fs::File;
//! use hadris_ntfs::sync::{NtfsFs, NtfsFsReadExt};
//!
//! let file = File::open("disk.img").unwrap();
//! let fs = NtfsFs::open(file).unwrap();
//! let root = fs.root_dir();
//! let entries = root.entries().unwrap();
//! for entry in &entries {
//!     println!("{} ({})", entry.name(), if entry.is_directory() { "dir" } else { "file" });
//! }
//! ```
//!
//! ## Feature Flags
//!
//! | Feature  | Default | Description |
//! |----------|---------|-------------|
//! | `std`    | Yes     | Standard library support (enables `alloc`, `sync`) |
//! | `alloc`  | No      | Heap allocation without full std |
//! | `sync`   | No      | Synchronous API via `hadris-io` sync traits |
//! | `async`  | No      | Asynchronous API via `hadris-io` async traits |
//! | `read`   | Yes     | Read operations (requires `alloc`) |
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
pub mod raw;
#[cfg(feature = "read")]
pub mod attr;

// ---------------------------------------------------------------------------
// Sync module
// ---------------------------------------------------------------------------

#[cfg(all(feature = "sync", feature = "read"))]
#[path = ""]
pub mod sync {
    //! Synchronous NTFS filesystem API.
    //!
    //! All I/O operations use synchronous `Read`/`Seek` traits.

    pub use hadris_io::Result as IoResult;
    pub use hadris_io::sync::{Parsable, Read, ReadExt, Seek};
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
        pub mod io;
        pub mod fs;
        pub mod dir;
        pub mod read;
    }
    pub use __inner::*;

    pub use __inner::dir::{NtfsDir, NtfsEntry};
    pub use __inner::fs::NtfsFs;
    pub use __inner::read::{FileReader, NtfsFsReadExt};
}

// ---------------------------------------------------------------------------
// Async module
// ---------------------------------------------------------------------------

#[cfg(all(feature = "async", feature = "read"))]
#[path = ""]
pub mod r#async {
    //! Asynchronous NTFS filesystem API.
    //!
    //! All I/O operations use async `Read`/`Seek` traits.

    pub use hadris_io::Result as IoResult;
    pub use hadris_io::r#async::{Parsable, Read, ReadExt, Seek};
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
        pub mod io;
        pub mod fs;
        pub mod dir;
        pub mod read;
    }
    pub use __inner::*;
}

// ---------------------------------------------------------------------------
// Default re-exports (sync)
// ---------------------------------------------------------------------------

#[cfg(all(feature = "sync", feature = "read"))]
pub use sync::*;

pub use error::{NtfsError, Result};
pub use raw::*;
