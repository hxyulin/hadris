//! # hadris-fat
//!
//! A pure Rust, `no_std`-compatible library for reading, writing, and formatting
//! FAT12, FAT16, FAT32 and exFAT filesystems.
//! It is suitable for disk-image tools, bootloaders, kernels, firmware,
//! embedded devices, SD cards, and USB drives.
//!
//! ## The driver: `FatFs`
//!
//! `FatFs` is the node-based driver, generated for each mode
//! (`sync::FatFs`, `r#async::FatFs`). It mounts any
//! `hadris_storage` block device, needs `alloc` for its node table, and implements the
//! `hadris_fs` `FileSystem` trait, so `Volume`, its handles and the
//! `hadris-fs` tree helpers work on it. It reads and writes files and
//! directories: `create`, `mkdir`, `unlink`, `rmdir`, `rename`, `write`,
//! `truncate`, `setattr`, `close`, `fsync` and `sync`. Sizes of pinned
//! files are kept in the node table until one of the last three; see the
//! `FatFs` docs for durability and crash safety.
//!
//! ```rust,no_run
//! # #[cfg(all(feature = "sync", feature = "std"))]
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use std::io::{Read, Write};
//!
//! use hadris_fat::sync::FatFs;
//! use hadris_fs::sync::{FileSystem, Volume};
//! use hadris_fs::{MountOptions, OpenOptions};
//! use hadris_storage::{BlockSize, MemDevice};
//!
//! let image = std::fs::read("disk.img")?;
//! let dev = MemDevice::new(image, BlockSize::new(512).unwrap());
//! let fs = FatFs::mount(dev, MountOptions::new())?;
//! let vol = Volume::new(fs);
//! for entry in vol.read_dir("/EFI")? {
//!     println!("{:?}", entry?.name());
//! }
//! let mut config = Vec::new();
//! vol.open("/boot/grub.cfg", OpenOptions::new().read())?
//!     .read_to_end(&mut config)?;
//! let mut backup = vol.open(
//!     "/boot/grub.cfg.bak",
//!     OpenOptions::new().write().create().truncate(),
//! )?;
//! backup.write_all(&config)?;
//! backup.close()?;
//! vol.lock().sync()?;
//! # Ok(())
//! # }
//! # #[cfg(not(all(feature = "sync", feature = "std")))]
//! # fn main() {}
//! ```
//!
//! [`MountOptions`](hadris_fs::MountOptions) choose read-only, the
//! [`Clock`](hadris_fs::Clock) that stamps entries, the UTC offset of the
//! volume's timestamps, the [`CodePage`](hadris_fs::CodePage) of short
//! names (CP437 by default) and a cap on pinned nodes. A failed mount
//! returns a [`MountError`](hadris_fs::MountError) that gives the device
//! back, and `unmount` syncs and gives it back too.
//!
//! ## exFAT: `ExFatFs`
//!
//! [`exfat`] holds `ExFatFs`, a sibling of `FatFs` with the same shape:
//! `exfat::sync::ExFatFs` and its `r#async` twin, each
//! with `check` and, with `write`, `format`. It needs `alloc`
//! and implements `FileSystem`.
//!
//! ## Formatting with `FatFs`
//!
//! With the `write` feature, `format` (in each mode) lays out a FAT12,
//! FAT16 or FAT32 volume that fills a block device and mounts it, so it
//! needs `alloc`. [`FormatOptions`] sets the variant, label, volume id,
//! sector and cluster size and the other boot sector fields; everything
//! defaults from the device's size. A failed format also returns a
//! [`MountError`](hadris_fs::MountError) with the device.
//!
//! ```rust
//! # #[cfg(all(feature = "sync", feature = "write", feature = "std"))]
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use hadris_fat::sync::format;
//! use hadris_fat::{FatKind, FormatOptions, VolumeLabel};
//! use hadris_fs::OpenOptions;
//! use hadris_fs::sync::Volume;
//! use hadris_storage::{BlockSize, MemDevice};
//!
//! let dev = MemDevice::new(vec![0u8; 8 << 20], BlockSize::new(512).unwrap());
//! let fs = format(dev, FormatOptions::new().with_label(VolumeLabel::new("DATA")?))?;
//! assert_eq!(fs.kind(), FatKind::Fat12);
//! let vol = Volume::new(fs);
//! let mut file = vol.open("/hello.txt", OpenOptions::new().write().create())?;
//! file.write(b"hello")?;
//! file.close()?;
//! # Ok(())
//! # }
//! # #[cfg(not(all(feature = "sync", feature = "write", feature = "std")))]
//! # fn main() {}
//! ```
//!
//! ## Checking
//!
//! `check` (in each mode, no allocator) reads an unmounted volume without
//! changing it and passes each `hadris_fs::Finding` to a callback: boot
//! sector, FSInfo and FAT copy problems, a dirty volume, broken, cyclic,
//! cross-linked and lost chains, chains that do not fit their file, bad
//! names, dot entries, misplaced labels and broken long-name runs. Each
//! finding has a [`Detail`] code and the path of its entry.
//!
//! ```rust
//! # #[cfg(all(feature = "sync", feature = "write", feature = "std"))]
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use hadris_fat::sync::{check, format};
//! use hadris_fat::FormatOptions;
//! use hadris_storage::{BlockSize, MemDevice};
//!
//! let dev = MemDevice::new(vec![0u8; 8 << 20], BlockSize::new(512).unwrap());
//! let mut dev = format(dev, FormatOptions::new())?.into_inner();
//! let mut scratch = [0u8; 4096];
//! let report = check(&mut dev, &mut scratch, |finding| println!("{finding}"))?;
//! assert!(report.is_clean());
//! # Ok(())
//! # }
//! # #[cfg(not(all(feature = "sync", feature = "write", feature = "std")))]
//! # fn main() {}
//! ```
//!
//! ## Feature Flags
//!
//! | Feature  | Default | Description |
//! |----------|---------|-------------|
//! | `std`    | Yes     | Standard library support (enables `alloc`); `hadris_storage::host::FileDevice` and `SystemClock` from the storage and fs crates |
//! | `alloc`  | No      | `FatFs`, `ExFatFs` and `format`; without it only `check` and the raw layer |
//! | `sync`   | Yes     | Synchronous API in `sync` |
//! | `async`  | No      | Asynchronous API with `Send` futures in `r#async` |
//! | `write`  | Yes     | `format` in each mode; `FatFs` and `ExFatFs` write without it |
//! | `defmt`  | No      | `defmt::Format` for `FatKind` |
//!
//! No feature changes what an item does: `FatFs` always reads and writes long
//! names.
//!
//! ## Sync and async
//!
//! The same source is compiled once per enabled mode: `sync` and `r#async`,
//! whose futures are `Send` when the device is. Each holds `FatFs`, `check` and, with `write`, `format`. The crate root holds only the mode-independent types.
//!
//! ## Modules
//!
//! - `sync::FatFs`, `r#async::FatFs`: the driver
//! - `sync::format` and its `async` versions: the formatter (requires `write`)
//! - `sync::check` and its `async` versions: the checker, from
//!   `hadris-fat-raw`
//! - `exfat`: the exFAT driver, `ExFatFs`, with its own `sync` and
//!   `r#async` modes, formatter and checker
//! - `Detail` and `exfat::Detail`: what exactly is wrong with a volume, read
//!   from mount and read errors with `Detail::of`
//!
//! ## The raw layer
//!
//! The on-disk layouts, the I/O-free codecs and the device primitives the
//! drivers are built on are the separate `hadris-fat-raw` crate, which has
//! its own version. This crate re-exports only what its own signatures use:
//! [`FatKind`], [`Detail`], `exfat::Detail` and the `check` functions.
//! Depend on `hadris-fat-raw` directly to use the rest.

#![cfg_attr(not(test), no_std)]
#![deny(missing_docs)]
#![allow(async_fn_in_trait)]
// Sync and async APIs intentionally compile the same source modules twice.
#![allow(clippy::duplicate_mod)]

#[cfg(all(feature = "std", not(test)))]
extern crate std;

#[cfg(test)]
extern crate self as hadris_fat;

#[cfg(feature = "alloc")]
extern crate alloc;

mod options;
#[cfg(feature = "alloc")]
mod table;

/// The exFAT driver, `ExFatFs`, its formatter and checker.
pub mod exfat;

#[cfg(feature = "sync")]
#[path = ""]
pub mod sync {
    //! The synchronous API.

    #[allow(unused_macros)]
    macro_rules! io_transform {
        ($($item:tt)*) => { hadris_macros::strip_async!{ $($item)* } };
    }

    use hadris_fat_raw::io::sync as rawio;
    #[cfg(feature = "alloc")]
    use hadris_fs::sync as fsapi;
    #[cfg(feature = "alloc")]
    use hadris_storage::sync as storage;

    #[cfg(feature = "alloc")]
    #[path = "block_io.rs"]
    pub(crate) mod block_io;
    #[cfg(feature = "alloc")]
    #[path = "fatfs.rs"]
    mod fatfs;
    #[cfg(feature = "alloc")]
    pub use fatfs::FatFs;
    pub use rawio::check;
    #[cfg(all(feature = "alloc", feature = "write"))]
    #[path = "mkfs.rs"]
    mod mkfs;
    #[cfg(all(feature = "alloc", feature = "write"))]
    pub use mkfs::format;
}

/// The asynchronous API with `Send` futures, for generic code on
/// multi-threaded executors.
///
/// Generated from the same source as `sync`, following `hadris_fs::r#async`.
/// Its `FatFs` futures are `Send` when the device is.
#[cfg(feature = "async")]
pub mod r#async;

/// The permissions FAT and exFAT derive: `rwx` for directories, `rw` for
/// files, and no write bits when the entry is read-only.
#[cfg(all(feature = "alloc", any(feature = "sync", feature = "async")))]
fn permissions(dir: bool, read_only: bool) -> hadris_fs::Permissions {
    let mode = if dir { 0o755 } else { 0o644 };
    hadris_fs::Permissions::new(if read_only { mode & !0o222 } else { mode })
}

/// The read-only bit `wanted` stands for: `None` when the volume cannot
/// report those permissions for a node of this kind.
#[cfg(all(feature = "alloc", any(feature = "sync", feature = "async")))]
fn read_only_bit(dir: bool, wanted: hadris_fs::Permissions) -> Option<bool> {
    let read_only = wanted.bits() & 0o222 == 0;
    (permissions(dir, read_only) == wanted).then_some(read_only)
}

pub use hadris_fat_raw::{Detail, FatKind};
#[cfg(feature = "write")]
pub use options::FormatOptions;
pub use options::VolumeLabel;
