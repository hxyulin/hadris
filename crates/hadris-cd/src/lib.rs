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

pub mod error;
pub mod layout;
pub mod options;
pub mod tree;
pub mod writer;

pub use error::{CdError, CdResult};
pub use layout::{LayoutInfo, LayoutManager};
pub use options::{CdOptions, IsoOptions, UdfOptions};
pub use tree::{Directory, FileData, FileEntry, FileExtent, FileTree};
pub use writer::CdWriter;
