//! # Hadris CPIO
//!
//! A Rust implementation of the CPIO archive format (newc/SVR4) with support for
//! no-std environments, streaming reads, and archive creation from in-memory trees
//! or the host filesystem.
//!
//! CPIO archives are commonly used for Linux initramfs images, RPM packages, and
//! general-purpose file archiving. This crate supports the "new" (newc) ASCII format
//! (`070701`) and its CRC variant (`070702`), which are the formats used by modern
//! Linux tools.
//!
//! ## Quick Start
//!
//! ### Reading an Archive
//!
//! ```rust,no_run
//! use std::fs::File;
//! use std::io::BufReader;
//! use hadris_cpio::CpioReader;
//!
//! let file = File::open("archive.cpio").unwrap();
//! let mut reader = CpioReader::new(BufReader::new(file));
//!
//! while let Some(entry) = reader.next_entry_alloc().unwrap() {
//!     let name = entry.name_str().unwrap();
//!     println!("{} ({} bytes)", name, entry.file_size());
//!     reader.skip_entry_data_owned(&entry).unwrap();
//! }
//! ```
//!
//! ### Creating an Archive
//!
//! ```rust,no_run
//! use std::fs::File;
//! use std::io::BufWriter;
//! use hadris_cpio::{CpioWriteOptions, CpioWriter, FileTree};
//!
//! let tree = FileTree::from_fs(std::path::Path::new("./my-directory")).unwrap();
//! let writer = CpioWriter::new(CpioWriteOptions::default());
//!
//! let mut out = BufWriter::new(File::create("archive.cpio").unwrap());
//! writer.write(&mut out, &tree).unwrap();
//! ```
//!
//! ## Feature Flags
//!
//! | Feature | Description | Dependencies |
//! |---------|-------------|--------------|
//! | `read` | Streaming archive reader | None |
//! | `alloc` | Heap allocation without full std | `alloc` crate |
//! | `std` | Full standard library support | `std`, `alloc` |
//! | `write` | Archive creation | `alloc`, `read` |
//!
//! Default features: `std`, `read`, `write`
//!
//! ### For Bootloaders / Kernels (minimal footprint)
//!
//! ```toml
//! [dependencies]
//! hadris-cpio = { version = "1.0", default-features = false, features = ["read"] }
//! ```
//!
//! ### For Kernels with Heap (no-std + alloc)
//!
//! ```toml
//! [dependencies]
//! hadris-cpio = { version = "1.0", default-features = false, features = ["read", "alloc"] }
//! ```
//!
//! ### For Desktop Applications (full features)
//!
//! ```toml
//! [dependencies]
//! hadris-cpio = { version = "1.0" }  # Uses default features
//! ```
//!
//! ## Archive Format
//!
//! The newc format stores entries sequentially. Each entry consists of:
//!
//! 1. A 110-byte ASCII header (all numeric fields in uppercase hex)
//! 2. The filename (NUL-terminated, padded to 4-byte boundary)
//! 3. The file data (padded to 4-byte boundary)
//!
//! The archive ends with a special `TRAILER!!!` sentinel entry.
//!
//! Two magic numbers are supported:
//! - `070701` — Standard newc format
//! - `070702` — newc with per-file CRC checksums
//!
//! ## Architecture
//!
//! - [`error`] — Error types and result alias
//! - [`header`] — Raw 110-byte header parsing and construction
//! - [`entry`] — Decoded entry header with typed fields
//! - [`mode`] — Unix file type extraction from mode bits
//! - [`read`] — Streaming archive reader (`CpioReader`)
//! - [`mod@write`] — Archive writer and in-memory file tree
//!
//! ## Specification References
//!
//! - `cpio(5)` man page — newc format definition
//! - Linux kernel `usr/gen_init_cpio.c` — Reference implementation
//! - RPM file format specification — CPIO payload format

#![no_std]
#![allow(async_fn_in_trait)]

#[cfg(feature = "std")]
extern crate std;

#[cfg(feature = "alloc")]
extern crate alloc;

// ---------------------------------------------------------------------------
// Shared types (compiled once, not duplicated by sync/async modules)
// ---------------------------------------------------------------------------

/// Error types for CPIO operations.
pub mod error;
/// Unix file type constants and mode bit manipulation.
pub mod mode;

// ---------------------------------------------------------------------------
// Sync module
// ---------------------------------------------------------------------------

#[cfg(feature = "sync")]
#[path = ""]
pub mod sync {
    //! Synchronous CPIO archive API.
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
        /// Decoded entry header with typed fields.
        pub mod entry;
        /// Raw 110-byte ASCII newc header parsing and construction.
        pub mod header;
        /// Streaming CPIO archive reader.
        #[cfg(feature = "read")]
        pub mod read;
        /// CPIO archive writer and in-memory file tree.
        #[cfg(feature = "write")]
        pub mod write;
    }
    pub use __inner::*;

    // Convenience re-exports
    pub use __inner::entry::CpioEntryHeader;
    pub use __inner::header::{
        CpioMagic, HEADER_SIZE, MAGIC_NEWC, MAGIC_NEWC_CRC, RawNewcHeader, TRAILER_NAME,
    };
    #[cfg(all(feature = "read", feature = "alloc"))]
    pub use __inner::read::CpioEntryOwned;
    #[cfg(feature = "read")]
    pub use __inner::read::{CpioEntry, CpioReader};
    #[cfg(feature = "write")]
    pub use __inner::write::file_tree::{FileNode, FileTree};
    #[cfg(feature = "write")]
    pub use __inner::write::{CpioWriteOptions, CpioWriter};
}

// ---------------------------------------------------------------------------
// Async module
// ---------------------------------------------------------------------------

#[cfg(feature = "async")]
#[path = ""]
pub mod r#async {
    //! Asynchronous CPIO archive API.
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
        pub mod entry;
        pub mod header;
        #[cfg(feature = "read")]
        pub mod read;
        #[cfg(feature = "write")]
        pub mod write;
    }
    pub use __inner::*;
}

// ---------------------------------------------------------------------------
// Default re-exports for backwards compatibility (sync)
// ---------------------------------------------------------------------------

#[cfg(feature = "sync")]
pub use sync::*;

// Re-exports from shared types
pub use error::{CpioError, Result};
pub use mode::FileType;
