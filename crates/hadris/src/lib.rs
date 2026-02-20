//! # Hadris
//!
//! A unified package for working with filesystem and disk image formats.
//!
//! This meta-crate re-exports the individual hadris crates, providing
//! a single dependency for accessing all supported formats:
//!
//! - **`iso`** — ISO 9660 with Joliet, Rock Ridge, and El-Torito boot support
//! - **`fat`** — FAT12/16/32 with long filenames and optional caching
//! - **`udf`** — Universal Disk Format (UDF) 1.02–2.60
//! - **`cpio`** — CPIO newc/CRC archive format
//!
//! ## Feature Flags
//!
//! | Feature   | Default | Description |
//! |-----------|---------|-------------|
//! | `iso9660` | Yes     | ISO 9660 filesystem support |
//! | `fat`     | Yes     | FAT12/16/32 filesystem support |
//! | `cpio`    | Yes     | CPIO archive support |
//! | `udf`     | No      | UDF filesystem support |
//! | `sync`    | No      | Synchronous API for all enabled formats |
//! | `async`   | No      | Asynchronous API for all enabled formats |
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! // Read an ISO image
//! use hadris::iso::sync::IsoImage;
//! let file = std::fs::File::open("image.iso").unwrap();
//! let iso = IsoImage::open(file).unwrap();
//! let pvd = iso.read_pvd();
//! println!("Volume: {}", pvd.volume_identifier);
//! ```

#[cfg(feature = "iso9660")]
pub use hadris_iso as iso;

#[cfg(feature = "fat")]
pub use hadris_fat as fat;

#[cfg(feature = "udf")]
pub use hadris_udf as udf;

#[cfg(feature = "cpio")]
pub use hadris_cpio as cpio;
