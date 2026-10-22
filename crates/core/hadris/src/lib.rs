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
//! - `fat`, `part`, `iso`, `udf`, `cpio`: one format crate each. The ISO
//!   9660 and UDF bridge writer is `udf::plan_bridge` and
//!   `udf::sync::write_bridge` (with `write`).
//! - `sync::detect` and `async_::detect` (with `detect`): every format a
//!   device holds, as `ImageFormat`s in a `Detection`, each with the
//!   damage a mount would report. With `alloc`, `open` mounts the first
//!   filesystem found as an `AnyFs`, `open_partition` mounts the MBR or GPT
//!   partition a `Select` names, `detect_partitions` gives a `Detection` per
//!   partition, and with `write`, `format` writes an empty FAT, exFAT or UDF
//!   volume from a `FormatOptions`. `AnyFs` reaches the format extras
//!   `extents`, `volume_serial` and `set_label` by `match`.
//! - `apfs`: the read-only APFS preview, behind `unstable-apfs`. With
//!   `detect` it also adds `AnyFs::Apfs`, APFS mounting in `open` and
//!   `open_apfs`.
//! - `ntfs`: the NTFS preview, behind `unstable-ntfs`.
//! - `host` (with `std`): host files, directories and image files for
//!   builders and tools; the tree functions need `sync`, and `host::open`
//!   and `host::open_with` need `sync` and `detect`.
//!
//! # Feature flags
//!
//! One feature per format (`fat`, `part`, `iso`, `udf`, `cpio`) adds
//! that crate. `detect` adds `detect`, `open`, `open_partition`,
//! `detect_partitions` and `AnyFs` with `fat`, `part`, `iso`, `udf` and
//! `cpio`. `detect` lists APFS containers and NTFS volumes from their
//! signatures but does not mount them. The platform (`std`, `alloc`) and
//! mode (`sync`, `async`) features are forwarded to every enabled crate.
//! `write` adds the writers of every enabled format: FAT and exFAT
//! formatting, the ISO 9660, UDF (with the bridge) and cpio writers, and
//! with `detect`, `format`. `tracing` adds function spans to every enabled
//! format. The default set is `std`, `sync`, `write`, `fat`, `iso`, `cpio`
//! and `detect`.
//!
//! `unstable-ntfs` adds the NTFS preview. `unstable-apfs` adds the
//! read-only APFS preview and, with `detect`, `AnyFs::Apfs`, APFS damage
//! checks in `detect`, single-volume APFS mounting in `open` and
//! `open_apfs` for containers with multiple volumes. Everything an
//! `unstable-*` feature adds is outside semver and may change in any
//! release; `hadris-ntfs` and `hadris-apfs` may break in a minor release.
//! No other feature changes what an item does.
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

#[cfg(all(feature = "alloc", feature = "detect"))]
extern crate alloc;

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
/// `std::io` streams. Errors are `PathError` with the host path set. The
/// tree functions and `open` need `sync`.
#[cfg(feature = "std")]
#[cfg_attr(docsrs, doc(cfg(feature = "std")))]
pub mod host {
    #[cfg(feature = "sync")]
    #[cfg_attr(docsrs, doc(cfg(feature = "sync")))]
    pub use hadris_fs::host::{OnError, Symlinks, TreeOptions, file, read_tree, write_tree};
    pub use hadris_fs::host::{local_utc_offset, mount_options, source_date_epoch};
    pub use hadris_io::StdIo;
    pub use hadris_storage::host::FileDevice;

    /// Detects the format of the image or device at `path` and mounts it
    /// read-only with [`mount_options`], as
    /// [`sync::open`](crate::sync::open) does.
    #[cfg(all(feature = "detect", feature = "sync"))]
    #[cfg_attr(docsrs, doc(cfg(all(feature = "detect", feature = "sync"))))]
    pub fn open(
        path: impl AsRef<std::path::Path>,
    ) -> Result<crate::sync::AnyFs<FileDevice>, crate::PathError> {
        open_with(path, mount_options().read_only())
    }

    /// Detects the format of the image or device at `path` and mounts it
    /// with `options`, as [`sync::open`](crate::sync::open) does. The file
    /// is opened for reading and writing unless `options` is read-only.
    /// Start from [`mount_options`] for the host clock and time zone.
    ///
    /// ```no_run
    /// let fs = hadris::host::open_with("disk.img", hadris::host::mount_options())?;
    /// # let _ = fs;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[cfg(all(feature = "detect", feature = "sync"))]
    #[cfg_attr(docsrs, doc(cfg(all(feature = "detect", feature = "sync"))))]
    pub fn open_with(
        path: impl AsRef<std::path::Path>,
        options: crate::fs::MountOptions,
    ) -> Result<crate::sync::AnyFs<FileDevice>, crate::PathError> {
        use crate::{Error, ErrorKind, PathError};
        let path = path.as_ref();
        let opened = if options.is_read_only() {
            FileDevice::open(path)
        } else {
            std::fs::File::options()
                .read(true)
                .write(true)
                .open(path)
                .and_then(FileDevice::new)
        };
        let dev = match opened {
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
        crate::sync::open(dev, options).map_err(|err| PathError::from(err).with_host_path(path))
    }
}

#[cfg(feature = "detect")]
mod detect;
#[cfg(feature = "detect")]
#[cfg_attr(docsrs, doc(cfg(feature = "detect")))]
pub use detect::{Candidate, Detection, ImageFormat, Select};

/// The filesystem `sync::format` and `async_::format` write, with its
/// options.
#[cfg(all(feature = "detect", feature = "alloc", feature = "write"))]
#[cfg_attr(
    docsrs,
    doc(cfg(all(feature = "detect", feature = "alloc", feature = "write")))
)]
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum FormatOptions {
    /// A FAT12, FAT16 or FAT32 volume.
    Fat(hadris_fat::FatOptions),
    /// An exFAT volume.
    ExFat(hadris_fat::exfat::ExFatOptions),
    /// A UDF volume.
    Udf(hadris_udf::UdfOptions),
}

/// Detection, opening and formatting with blocking I/O.
#[cfg(all(feature = "detect", feature = "sync"))]
#[cfg_attr(docsrs, doc(cfg(all(feature = "detect", feature = "sync"))))]
#[path = ""]
pub mod sync {
    #[allow(unused_macros)]
    macro_rules! io_transform {
        ($($item:tt)*) => { hadris_macros::strip_async!{ $($item)* } };
    }

    #[cfg(all(feature = "alloc", feature = "unstable-apfs"))]
    use hadris_apfs::sync::ApfsFs;
    #[cfg(feature = "unstable-apfs")]
    use hadris_apfs::sync::Container as ApfsContainer;
    #[cfg(feature = "alloc")]
    use hadris_fat::exfat::sync::ExFatFs;
    #[cfg(feature = "alloc")]
    use hadris_fat::sync::FatFs;
    use hadris_fat_raw::exfat::io::sync as exio;
    use hadris_fat_raw::io::sync as rawio;
    #[allow(unused_macros)]
    macro_rules! send_filesystem {
        ($($item:tt)*) => {};
    }
    #[cfg(feature = "alloc")]
    use hadris_fs::sync::FileSystem;
    use hadris_iso::sync::IsoFs;
    use hadris_storage::sync::BlockDevice;
    use hadris_udf::sync::UdfFs;

    #[cfg(all(feature = "alloc", feature = "write"))]
    use hadris_fat::{exfat::sync::format as exfat_format, sync::format as fat_format};
    #[cfg(all(feature = "alloc", feature = "write"))]
    use hadris_udf::sync::write as udf_write;

    #[cfg(feature = "alloc")]
    use hadris_part::sync::{open as part_open, read as part_read};

    #[path = "open.rs"]
    mod open;
    pub use open::detect;
    #[cfg(all(feature = "alloc", feature = "write"))]
    pub use open::format;
    #[cfg(all(feature = "alloc", feature = "unstable-apfs"))]
    pub use open::open_apfs;
    #[cfg(feature = "alloc")]
    pub use open::{AnyFs, detect_partitions, open, open_partition};
}

/// Detection, opening and formatting for local and Send devices, generated
/// source as `sync`. Futures are Send when the device and its state are Send.
#[cfg(all(feature = "detect", feature = "async"))]
#[cfg_attr(docsrs, doc(cfg(all(feature = "detect", feature = "async"))))]
#[path = "async.rs"]
pub mod async_;

/// FAT12, FAT16, FAT32 and exFAT.
#[cfg(feature = "fat")]
#[cfg_attr(docsrs, doc(cfg(feature = "fat")))]
pub use hadris_fat as fat;

/// MBR, GPT and hybrid partition tables.
#[cfg(feature = "part")]
#[cfg_attr(docsrs, doc(cfg(feature = "part")))]
pub use hadris_part as part;

/// The NTFS preview, outside semver.
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

/// CPIO newc, CRC, odc and binary archives.
#[cfg(feature = "cpio")]
#[cfg_attr(docsrs, doc(cfg(feature = "cpio")))]
pub use hadris_cpio as cpio;

/// The read-only APFS preview, outside semver.
#[cfg(feature = "unstable-apfs")]
#[cfg_attr(docsrs, doc(cfg(feature = "unstable-apfs")))]
pub use hadris_apfs as apfs;
