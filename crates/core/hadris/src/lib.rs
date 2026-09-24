//! # Hadris
//!
//! **The Rust storage stack.**
//!
//! Hadris is a pure Rust toolkit for block devices, partition tables,
//! filesystems, archives, and disk images. It serves desktop tools and
//! `no_std` bootloaders, kernels, firmware, and embedded systems.
//!
//! The umbrella crate re-exports the libraries under flat paths, so one
//! dependency reaches all of them:
//!
//! - [`io`], [`storage`] and [`fs`]: I/O traits, block devices and
//!   adapters, and the shared filesystem vocabulary, traits, handles and
//!   input tree. Always present.
//! - [`Error`], [`ErrorKind`], [`Location`], [`DetailCode`], [`Errno`],
//!   [`FsResult`] and [`MountError`] at the root: the one error type every
//!   device and filesystem operation returns, and with `alloc`
//!   `PathError`, which adds the path that failed.
//! - `fat`, `part`, `iso`, `udf`, `cd`, `cpio`: one format crate each.
//! - `block` and `optical`: detection and opening of whatever volume or
//!   image a device holds.
//! - `ntfs`: the NTFS preview, behind `unstable-ntfs`.
//!
//! # Feature flags
//!
//! One feature per format (`fat`, `part`, `iso`, `udf`, `cd`, `cpio`) adds
//! that crate. `block` adds `hadris-block` with `fat` and `part`,
//! `optical` adds `hadris-optical` with `iso`, `udf` and `cd`, and
//! `archive` adds `cpio`. The platform (`std`, `alloc`), mode (`sync`,
//! `async`, `async-send`) and `write` features are forwarded to every
//! enabled crate. `unstable-ntfs` adds the NTFS preview, whose native API
//! may change in 3.x minors. The default set is `std`, `sync`, `write`,
//! `fat`, `iso` and `cpio`. No feature changes what an item does.
//!
//! # Quick start
//!
//! ```rust,no_run
//! # #[cfg(all(feature = "iso", feature = "std", feature = "sync"))]
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use hadris::fs::sync::DriverExt;
//! use hadris::iso::Namespace;
//! use hadris::iso::sync::IsoImage;
//!
//! let file = std::fs::File::open("image.iso")?;
//! let mut iso = IsoImage::open(file)?;
//! let mut view = iso.view(Namespace::Preferred)?;
//! for entry in view.read_dir("/")? {
//!     println!("{:?}", entry?.name());
//! }
//! # Ok(())
//! # }
//! # #[cfg(not(all(feature = "iso", feature = "std", feature = "sync")))]
//! # fn main() {}
//! ```

#![no_std]
#![deny(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

/// I/O traits with the device's own error, and adapters.
pub use hadris_io as io;

/// Block devices, adapters, slices and the block cache.
pub use hadris_storage as storage;

/// The shared filesystem vocabulary, errors, driver traits, handles and
/// input tree.
pub use hadris_fs as fs;

/// The error of writers and code that mixes devices, with the path that
/// failed.
#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
pub use hadris_fs::PathError;
pub use hadris_fs::{DetailCode, Errno, Error, ErrorKind, FsResult, Location, MountError};

/// FAT12, FAT16, FAT32 and exFAT.
#[cfg(feature = "fat")]
#[cfg_attr(docsrs, doc(cfg(feature = "fat")))]
pub use hadris_fat as fat;

/// MBR, GPT and hybrid partition tables.
#[cfg(feature = "part")]
#[cfg_attr(docsrs, doc(cfg(feature = "part")))]
pub use hadris_part as part;

/// The NTFS preview. Its native API may change in 3.x minors.
#[cfg(feature = "unstable-ntfs")]
#[cfg_attr(docsrs, doc(cfg(feature = "unstable-ntfs")))]
pub use hadris_ntfs as ntfs;

/// ISO 9660 images with Joliet, Rock Ridge and El Torito.
#[cfg(feature = "iso")]
#[cfg_attr(docsrs, doc(cfg(feature = "iso")))]
pub use hadris_iso as iso;

/// Universal Disk Format volumes.
#[cfg(feature = "udf")]
#[cfg_attr(docsrs, doc(cfg(feature = "udf")))]
pub use hadris_udf as udf;

/// Hybrid ISO 9660 and UDF images.
#[cfg(feature = "cd")]
#[cfg_attr(docsrs, doc(cfg(feature = "cd")))]
pub use hadris_cd as cd;

/// CPIO newc, CRC, odc and binary archives.
#[cfg(feature = "cpio")]
#[cfg_attr(docsrs, doc(cfg(feature = "cpio")))]
pub use hadris_cpio as cpio;

/// Detection and opening of FAT, exFAT and NTFS volumes.
#[cfg(feature = "block")]
#[cfg_attr(docsrs, doc(cfg(feature = "block")))]
pub use hadris_block as block;

/// Detection and opening of ISO 9660 and UDF images.
#[cfg(feature = "optical")]
#[cfg_attr(docsrs, doc(cfg(feature = "optical")))]
pub use hadris_optical as optical;
