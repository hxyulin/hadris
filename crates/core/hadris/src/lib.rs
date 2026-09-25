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
//! - `sync::detect` and `r#async::detect` (with `detect`): every format a
//!   device holds, as `ImageFormat`s in a `Detection`, each with the
//!   damage a mount would report. With `alloc`, `open` mounts the first
//!   filesystem found as an `AnyFs`.
//! - `ntfs`: the NTFS preview, behind `unstable-ntfs`.
//! - `host` (with `std` and `sync`): host files, directories and image
//!   files for builders and tools, and `host::open` (with `detect`).
//!
//! # Feature flags
//!
//! One feature per format (`fat`, `part`, `iso`, `udf`, `cd`, `cpio`) adds
//! that crate. `block` adds `hadris-block` with `fat` and `part`,
//! `optical` adds `hadris-optical` with `iso`, `udf` and `cd`, `archive`
//! adds `cpio`, and `detect` adds `detect`, `open` and `AnyFs` with `fat`,
//! `iso`, `udf` and `cpio`. The platform (`std`, `alloc`), mode (`sync`,
//! `async`) and `write` features are forwarded to every enabled crate.
//! `unstable-ntfs` adds the NTFS preview, whose native API may change in
//! 3.x minors. The default set is `std`, `sync`, `write`,
//! `fat`, `iso`, `cpio` and `detect`. No feature changes what an item does.
//!
//! # Quick start
//!
//! ```rust,no_run
//! # #[cfg(all(feature = "iso", feature = "std", feature = "sync"))]
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use hadris::fs::MountOptions;
//! use hadris::fs::sync::Volume;
//! use hadris::iso::sync::IsoFs;
//!
//! let file = hadris::storage::host::FileDevice::open("image.iso")?;
//! let vol = Volume::new(IsoFs::mount(file, MountOptions::new())?);
//! for entry in vol.read_dir("/")? {
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
#![allow(clippy::duplicate_mod)]

#[cfg(all(feature = "std", feature = "sync", feature = "detect"))]
extern crate std;

/// I/O traits with the device's own error, and adapters.
pub use hadris_io as io;

/// Block devices, adapters, slices and the block cache.
pub use hadris_storage as storage;

/// The shared filesystem vocabulary, errors, the `FileSystem` trait,
/// `Volume` with its handles, and the input tree.
pub use hadris_fs as fs;

/// The error of writers and code that mixes devices, with the path that
/// failed.
#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
pub use hadris_fs::PathError;
pub use hadris_fs::{DetailCode, Errno, Error, ErrorKind, FsResult, Location, MountError};

/// Host files, directories and image files, for builders and tools.
///
/// `read_tree` and `write_tree` move trees between host directories and
/// writers, `file` makes tree content of a host file, `source_date_epoch`
/// reads the build time, `mount_options` gives the host clock and time
/// zone, `FileDevice` opens image files and devices, and `StdIo` adapts
/// `std::io` streams. Errors are `PathError` with the host path set.
#[cfg(all(feature = "std", feature = "sync"))]
#[cfg_attr(docsrs, doc(cfg(all(feature = "std", feature = "sync"))))]
pub mod host {
    pub use hadris_fs::host::{
        OnError, Symlinks, TreeOptions, file, local_utc_offset, mount_options, read_tree,
        source_date_epoch, write_tree,
    };
    pub use hadris_io::StdIo;
    pub use hadris_storage::host::FileDevice;

    /// Detects the format of the image or device at `path` and mounts it
    /// read-only with [`mount_options`], as
    /// [`sync::open`](crate::sync::open) does.
    #[cfg(feature = "detect")]
    #[cfg_attr(docsrs, doc(cfg(feature = "detect")))]
    pub fn open(
        path: impl AsRef<std::path::Path>,
    ) -> Result<crate::sync::AnyFs<FileDevice>, crate::PathError> {
        use crate::{Error, ErrorKind, PathError};
        let path = path.as_ref();
        let dev = match FileDevice::open(path) {
            Ok(dev) => dev,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Err(
                    PathError::new(ErrorKind::NotFound, "no such image").with_host_path(path)
                );
            }
            Err(err) => {
                return Err(PathError::from(Error::device(err, "cannot open the image"))
                    .with_host_path(path));
            }
        };
        crate::sync::open(dev, mount_options().read_only())
            .map_err(|err| PathError::from(err).with_host_path(path))
    }
}

#[cfg(feature = "detect")]
mod detect;
#[cfg(feature = "detect")]
#[cfg_attr(docsrs, doc(cfg(feature = "detect")))]
pub use detect::{Candidate, Detection, ImageFormat};

/// Detection and opening with blocking I/O.
#[cfg(all(feature = "detect", feature = "sync"))]
#[cfg_attr(docsrs, doc(cfg(all(feature = "detect", feature = "sync"))))]
#[path = ""]
pub mod sync {
    #[allow(unused_macros)]
    macro_rules! io_transform {
        ($($item:tt)*) => { hadris_macros::strip_async!{ $($item)* } };
    }

    #[cfg(feature = "alloc")]
    use hadris_fat::exfat::sync::ExFatFs;
    #[cfg(feature = "alloc")]
    use hadris_fat::sync::FatFs;
    use hadris_fat_raw::exfat::io::sync as exio;
    use hadris_fat_raw::io::sync as rawio;
    #[cfg(feature = "alloc")]
    use hadris_fs::sync::FileSystem;
    use hadris_iso::sync::IsoFs;
    use hadris_storage::sync::BlockDevice;
    use hadris_udf::sync::UdfFs;

    #[path = "open.rs"]
    mod open;
    pub use open::detect;
    #[cfg(feature = "alloc")]
    pub use open::{AnyFs, open};
}

/// Detection and opening with `Send` futures, generated from the same
/// source as `sync`.
#[cfg(all(feature = "detect", feature = "async"))]
#[cfg_attr(docsrs, doc(cfg(all(feature = "detect", feature = "async"))))]
pub mod r#async;

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
