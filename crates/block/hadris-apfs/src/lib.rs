#![cfg_attr(not(feature = "std"), no_std)]
#![allow(async_fn_in_trait)]
#![deny(missing_docs)]

//! An experimental, read-only APFS container and volume reader.
//!
//! The API is outside the Hadris 2.x stability promise and may change in a
//! minor release. Compressed files, encryption, snapshots and writes are not
//! supported; see the README for the full scope.
//!
//! On-disk structures live in [`types`]. The readers in the `sync` and
//! `async` modules walk them over a `hadris-storage` block device.
//!
//! ```rust,no_run
//! use std::fs::File;
//!
//! use hadris_apfs::sync::Container;
//! use hadris_storage::sync::SeekBlockDevice;
//! use hadris_storage::{BlockCount, BlockGeometry, BlockSize};
//!
//! let file = File::open("container.img")?;
//! let sectors = file.metadata()?.len() / 512;
//! let geometry = BlockGeometry::new(BlockSize::new(512).unwrap(), BlockCount(sectors));
//! let mut container = Container::open(SeekBlockDevice::new(file, geometry))?;
//! let latest = container.latest_superblock()?;
//! for volume in container.volume_superblocks(&latest)? {
//!     if let Some(entry) = container.resolve_path(&volume, "/notes.txt")? {
//!         let data = container.read_file(&volume, entry.file_id, 1 << 20)?;
//!         println!("{}: {} bytes", volume.name()?, data.len());
//!     }
//! }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

#[cfg(any(feature = "alloc", feature = "std"))]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

/// Error types returned by APFS readers and writers.
pub mod error;
pub mod types;

#[cfg(all(feature = "read", feature = "async"))]
pub mod r#async;
#[cfg(feature = "read")]
pub mod read;
#[cfg(all(feature = "read", feature = "sync"))]
pub mod sync;

pub use error::{ApfsError, Result};
