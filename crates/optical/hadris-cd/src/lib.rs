//! # Hadris CD
//!
//! A Rust library for creating hybrid ISO+UDF optical disc images.
//!
//! ## Overview
//!
//! This crate creates "UDF Bridge" format images that contain both
//! ISO 9660 and UDF filesystems. This provides maximum compatibility:
//! - Legacy systems read ISO 9660
//! - Modern systems read UDF
//! - **Both filesystems share the same file data on disk**
//!
//! ## Quick Start
//!
//! ```rust
//! use hadris_cd::{CdWriter, CdOptions, FileTree, FileEntry};
//! # use std::io::Cursor;
//!
//! // Create a file tree
//! let mut tree = FileTree::new();
//! tree.add_file(FileEntry::from_buffer("readme.txt", b"Hello, World!".to_vec()));
//!
//! // Create the hybrid image
//! # // Use a Cursor for the doctest instead of a real file
//! # let buffer = vec![0u8; 2 * 1024 * 1024]; // 2MB buffer
//! # let file = Cursor::new(buffer);
//! let options = CdOptions::with_volume_id("MY_DISC")
//!     .with_joliet();
//!
//! CdWriter::new(file, options)
//!     .write(tree)
//!     .unwrap();
//! ```
//!
//! ## Disk Layout
//!
//! The UDF Bridge format interleaves ISO 9660 and UDF structures:
//!
//! ```text
//! Sector 0-15:    System area (boot code, partition tables)
//! Sector 16-...:  ISO 9660 Volume Descriptors
//! Sector 17-19:   UDF Volume Recognition Sequence (BEA01, NSR02, TEA01)
//! Sector 256:     UDF Anchor Volume Descriptor Pointer
//! Sector 257+:    UDF Volume Descriptor Sequence
//! File data:      Shared between ISO and UDF (both point to same sectors)
//! ```
//!
//! ## Features
//!
//! - ISO 9660 with Joliet (Windows long filenames) and Rock Ridge (POSIX)
//! - UDF 1.02/1.50/2.00+ support
//! - El-Torito bootable images
//! - Hybrid MBR+GPT for USB booting
//!
//! Hybrid image creation currently uses the synchronous ISO and UDF writers.
//! This crate therefore exposes a sync-only writer API; `std` selects hosted
//! platform support, while the default feature set selects `sync` explicitly.

#![allow(async_fn_in_trait)]

// ---------------------------------------------------------------------------
// Shared types (compiled once)
// ---------------------------------------------------------------------------

#[cfg(feature = "sync")]
pub mod error;
#[cfg(feature = "sync")]
pub mod layout;
#[cfg(feature = "sync")]
pub mod options;
#[cfg(feature = "sync")]
pub mod tree;

// ---------------------------------------------------------------------------
// Sync module
// ---------------------------------------------------------------------------

#[cfg(feature = "sync")]
#[path = ""]
pub mod sync {
    pub use hadris_io::SeekFrom;
    pub use hadris_io::sync::{Borrowed, Read, Seek, Write};

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
        pub mod writer;
    }
    pub use __inner::*;

    // Convenience re-exports
    pub use __inner::writer::CdWriter;
}

// ---------------------------------------------------------------------------
// Default re-exports for backwards compatibility (sync)
// ---------------------------------------------------------------------------

#[cfg(feature = "sync")]
pub use sync::*;

// Re-exports from shared types
#[cfg(feature = "sync")]
pub use error::{CdError, CdResult};
#[cfg(feature = "sync")]
pub use layout::{LayoutInfo, LayoutManager};
#[cfg(feature = "sync")]
pub use options::{CdOptions, IsoOptions, UdfOptions};
#[cfg(feature = "sync")]
pub use tree::{Directory, FileData, FileEntry, FileExtent, FileTree};
