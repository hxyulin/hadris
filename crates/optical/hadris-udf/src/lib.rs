//! # Hadris UDF
//!
//! A pure Rust Universal Disk Format (UDF) filesystem library for optical media
//! and disk images. It supports hosted applications and `no_std` bootloaders,
//! kernels, firmware, and embedded systems.
//!
//! UDF (ECMA-167) is the filesystem used for:
//! - DVD-ROM, DVD-Video, DVD-RAM
//! - Blu-ray discs
//! - Large USB drives (files >4GB)
//! - Packet writing to CD/DVD-RW
//!
//! ## Features
//!
//! This crate supports:
//! - **UDF 1.02**: DVD-ROM (read-only)
//! - **UDF 1.50**: DVD-RAM, packet writing (planned)
//! - **UDF 2.01**: DVD-RW, streaming (planned)
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use std::fs::File;
//! use std::io::BufReader;
//! use hadris_udf::UdfVolume;
//!
//! // Open a UDF image file
//! let file = File::open("movie.udf").unwrap();
//! let reader = BufReader::new(file);
//! let udf = UdfVolume::open(reader).unwrap();
//!
//! // Read volume info
//! let info = udf.info();
//! println!("Volume: {}", info.volume_id);
//!
//! // List root directory
//! let root = udf.root_dir().unwrap();
//! for entry in root.entries() {
//!     println!("{} ({})", entry.name(), entry.size);
//! }
//!
//! // Read a file's contents
//! # let entry = root.entries().next().unwrap();
//! let bytes = udf.read_file(&entry).unwrap();
//! # let _ = bytes;
//! ```
//!
//! ## Feature Flags
//!
//! | Feature | Description |
//! |---------|-------------|
//! | `read` | Read support (default) |
//! | `alloc` | Heap allocation without full std |
//! | `std` | Full standard library support |
//! | `write` | Write/format support (requires std) |
//! | `sync` | Synchronous API under [`sync`] (default) |
//! | `async` | Asynchronous read API under `hadris_udf::r#async` |
//!
//! `std` does not select an I/O mode. The `write` implementation is currently
//! synchronous-only; enabling `write` and `async` together does not expose an
//! async write API.
//!
//! ## Known Limitations
//!
//! - Extended allocation descriptors and stream directories are not supported.
//! - Packet writing / sparing tables / Blu-ray-specific features are not implemented.
//! - Directory listing reads each file ICB to populate
//!   [`dir::UdfDirEntry::size`] (one extra seek per file).
//!
//! ## Specification References
//!
//! - ECMA-167: Volume and File Structure for Write-Once and Rewritable Media
//! - OSTA UDF Specification (udf260.pdf)

#![no_std]
#![allow(async_fn_in_trait)]
#![deny(missing_docs)]
// Sync and async APIs intentionally compile the same source modules twice.
#![allow(clippy::duplicate_mod)]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

// ---------------------------------------------------------------------------
// Shared types (compiled once, not duplicated by sync/async modules)
// ---------------------------------------------------------------------------

mod error;
mod time;

pub use error::{Error, Result};
pub use time::UdfTimestamp;

// ---------------------------------------------------------------------------
// Sync module
// ---------------------------------------------------------------------------

#[cfg(feature = "sync")]
#[path = ""]
pub mod sync {
    //! Synchronous UDF filesystem API.
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
        pub mod descriptor;
        #[cfg(feature = "alloc")]
        pub mod dir;
        #[cfg(feature = "alloc")]
        pub mod file;
        #[cfg(feature = "alloc")]
        pub mod fs;
        sync_only! {
            #[cfg(feature = "write")]
            pub mod write;
        }
    }
    #[cfg(feature = "alloc")]
    pub use __inner::dir::UdfDir;
    #[cfg(feature = "alloc")]
    pub use __inner::file::FileType;
    pub use __inner::*;

    #[cfg(feature = "alloc")]
    pub use __inner::fs::{UdfVolume, UdfVolumeInfo};
}

// ---------------------------------------------------------------------------
// Async module
// ---------------------------------------------------------------------------

#[cfg(feature = "async")]
#[path = ""]
pub mod r#async {
    //! Asynchronous UDF filesystem API.
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
        pub mod descriptor;
        #[cfg(feature = "alloc")]
        pub mod dir;
        #[cfg(feature = "alloc")]
        pub mod file;
        #[cfg(feature = "alloc")]
        pub mod fs;
    }
    #[cfg(feature = "alloc")]
    pub use __inner::dir::UdfDir;
    #[cfg(feature = "alloc")]
    pub use __inner::file::FileType;
    pub use __inner::*;

    #[cfg(feature = "alloc")]
    pub use __inner::fs::{UdfVolume, UdfVolumeInfo};
}

// ---------------------------------------------------------------------------
// Default re-exports for backwards compatibility (sync)
// ---------------------------------------------------------------------------

#[cfg(feature = "sync")]
pub use sync::*;

// When only async is enabled (no sync), re-export async module contents
// so that shared modules (dir.rs, file.rs) can use `crate::descriptor::*`.
#[cfg(all(feature = "async", not(feature = "sync")))]
pub use r#async::*;

/// UDF revision numbers
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct UdfRevision(u16);

impl UdfRevision {
    /// UDF 1.02 - DVD-ROM
    pub const V1_02: Self = Self(0x0102);
    /// UDF 1.50 - DVD-RAM, packet writing
    pub const V1_50: Self = Self(0x0150);
    /// UDF 2.00 - DVD-RW
    pub const V2_00: Self = Self(0x0200);
    /// UDF 2.01 - DVD-RW streaming
    pub const V2_01: Self = Self(0x0201);
    /// UDF 2.50 - Blu-ray
    pub const V2_50: Self = Self(0x0250);
    /// UDF 2.60 - Blu-ray pseudo-overwrite
    pub const V2_60: Self = Self(0x0260);

    /// Create a revision from raw value
    pub const fn from_raw(value: u16) -> Self {
        Self(value)
    }

    /// Get the raw revision value
    pub const fn to_raw(self) -> u16 {
        self.0
    }

    /// Get the major version number
    pub const fn major(self) -> u8 {
        ((self.0 >> 8) & 0xFF) as u8
    }

    /// Get the minor version number
    pub const fn minor(self) -> u8 {
        (self.0 & 0xFF) as u8
    }
}

impl core::fmt::Display for UdfRevision {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}.{:02x}", self.major(), self.minor())
    }
}

/// Sector size for UDF (always 2048 bytes for optical media)
pub const SECTOR_SIZE: usize = 2048;

/// Location of the first Anchor Volume Descriptor Pointer
pub const AVDP_LOCATION: u32 = 256;

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::format;

    #[test]
    fn test_udf_revision() {
        let rev = UdfRevision::V2_01;
        assert_eq!(rev.major(), 2);
        assert_eq!(rev.minor(), 1);
        assert_eq!(rev.to_raw(), 0x0201);
    }

    #[test]
    fn test_udf_revision_display() {
        assert_eq!(format!("{}", UdfRevision::V1_02), "1.02");
        assert_eq!(format!("{}", UdfRevision::V2_50), "2.50");
    }
}
