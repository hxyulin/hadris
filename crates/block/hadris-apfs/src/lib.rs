#![cfg_attr(not(feature = "std"), no_std)]
#![allow(async_fn_in_trait)]
#![deny(missing_docs)]
#![allow(clippy::duplicate_mod)]

//! An experimental, read-only APFS container and volume reader.
//!
//! The API is outside the Hadris 3.x stability promise and may change in a
//! minor release. Compressed files, hardware/per-file encryption, snapshots and writes are not
//! supported; see the README for the full scope.
//!
//! On-disk structures live in [`types`]. The readers in the `sync` and
//! `async` modules walk them over a `hadris-storage` block device.
//!
//! With `alloc`, each mode also exposes `ApfsFs`, a read-only
//! shared filesystem driver. Mounting without a selector requires
//! exactly one volume; [`VolumeSelector`] identifies an explicit volume by
//! UUID, object identifier, container slot or exact name. Metadata is validated
//! and indexed once during mount. Inode identifiers remain stable across
//! lookups, directory cursors are reusable, and data reads use bounded block
//! scratch space for sparse and arbitrarily large files. Generic `Volume`
//! handles follow symbolic links and expose hard-linked inodes consistently.
//! Native container inspection remains available independently of mounting.
//! With `encryption`, password mounts unlock software-encrypted single-key
//! volumes. `ApfsFs::mount_with_password` and `mount_volume_with_password` borrow
//! credentials independently of volume selection. Native encrypted reads require
//! an explicit volume through `read_volume_extents_at` or
//! `read_volume_btree_node_with_flags`. Keybags and password-derivation work are
//! bounded; wrong credentials report [`Detail::Credentials`] and preserve the
//! device. Retained keys are redacted and wiped on drop.
//!
//! Native I/O and driver errors are [`hadris_fs::Error`] values retaining the
//! backend error. [`Detail::of`] identifies APFS-specific failures. Pure
//! on-disk parsers return [`ApfsError`]. Mount failures return the original
//! device in [`hadris_fs::MountError`].
//!
//! `std` implies `alloc`. `read` enables readers, `sync` enables blocking
//! APIs, and `async` enables asynchronous APIs with `Send` futures. Without
//! `alloc`, the native container header and block reader remains usable.
//! `encryption` implies `read` and `alloc` and adds optional crypto dependencies.
//!
//! ```rust,no_run
//! use std::fs::File;
//!
//! use hadris_apfs::sync::Container;
//! use hadris_storage::sync::StreamDevice;
//! use hadris_storage::{BlockCount, BlockGeometry, BlockSize};
//!
//! let file = File::open("container.img")?;
//! let sectors = file.metadata()?.len() / 512;
//! let geometry = BlockGeometry::new(BlockSize::new(512).unwrap(), BlockCount::new(sectors));
//! let mut container = Container::open(StreamDevice::with_block_count(
//!     hadris_storage::ReadOnly::new(hadris_io::StdIo::new(file)),
//!     geometry.logical_block_size(), geometry.block_count().get(),
//! ))?;
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

#[cfg(all(feature = "encryption", any(feature = "sync", feature = "async")))]
mod crypto;

/// Error types returned by APFS readers and writers.
pub mod error;
pub mod types;

#[cfg(all(feature = "read", feature = "async"))]
pub mod r#async;
#[cfg(feature = "read")]
pub mod read;
#[cfg(all(feature = "read", feature = "sync"))]
pub mod sync;

pub use error::{ApfsError, Detail, Result};

/// Selects the volume to mount within an APFS container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum VolumeSelector<'a> {
    /// The volume's container slot index (`fs_index`).
    Index(u32),
    /// The volume superblock's stable object identifier.
    ObjectId(u64),
    /// The volume UUID as sixteen bytes.
    Uuid([u8; 16]),
    /// The exact UTF-8 volume name. Duplicate names are ambiguous.
    Name(&'a str),
}
