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
//! (`sync::FatFs`, `r#async::FatFs`, `async_send::FatFs`). It mounts any
//! `hadris_storage` block device, needs no allocator, and implements the
//! `hadris_fs` `FsDriver` trait, so the `hadris-fs` path helpers, `Volume`
//! and handles work on it. It reads and writes files and directories:
//! `create`, `remove`, `rename`, `write_at`, `set_len`, `set_metadata`,
//! `publish_node`, `sync_node` and `sync`. Sizes of pinned files are kept
//! in the node table until one of the last three; see the `FatFs` docs for durability and
//! crash safety.
//!
//! ```rust,no_run
//! # #[cfg(all(feature = "sync", feature = "std"))]
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use hadris_fat::sync::FatFs;
//! use hadris_fs::sync::{FileSystem, PathExt, Volume};
//! use hadris_storage::{BlockSize, MemDevice};
//!
//! let image = std::fs::read("disk.img")?;
//! let fs = FatFs::open(MemDevice::new(image, BlockSize::new(512).unwrap()))?;
//! let vol = Volume::new(fs);
//! for entry in vol.read_dir("/EFI")? {
//!     println!("{}", entry?.name_str().unwrap_or("?"));
//! }
//! let config = vol.read_to_vec("/boot/grub.cfg")?;
//! vol.write_file("/boot/grub.cfg.bak", &config)?;
//! vol.sync()?;
//! # Ok(())
//! # }
//! # #[cfg(not(all(feature = "sync", feature = "std")))]
//! # fn main() {}
//! ```
//!
//! `FatFs<D, T, C, P>` also takes the node table, the [`Clock`](hadris_fs::Clock)
//! that stamps entries and the [`CodePage`] of short names as type
//! parameters, chosen with [`MountOptions`] and `FatFs::open_with`. A failed
//! mount returns a [`MountError`](hadris_fs::MountError) that gives the
//! device back.
//!
//! ## exFAT: `ExFatFs`
//!
//! [`exfat`] holds `ExFatFs`, a sibling of `FatFs` with the same shape:
//! `exfat::sync::ExFatFs` and its `r#async` and `async_send` twins, each
//! with `check`, `check_with` and, with `write`, `format`. It needs no
//! allocator and implements `FsDriver`.
//!
//! ## Formatting with `FatFs`
//!
//! With the `write` feature, `format` (in each mode) lays out a FAT12,
//! FAT16 or FAT32 volume that fills a block device and mounts it. It needs
//! no allocator. [`FormatOptions`] sets the variant, label, volume id,
//! sector and cluster size and the other boot sector fields; everything
//! defaults from the device's size. A failed format also returns a
//! [`MountError`](hadris_fs::MountError) with the device.
//!
//! ```rust
//! # #[cfg(all(feature = "sync", feature = "write", feature = "std"))]
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use hadris_fat::sync::format;
//! use hadris_fat::{FatKind, FormatOptions, VolumeLabel};
//! use hadris_fs::sync::{PathExt, Volume};
//! use hadris_storage::{BlockSize, MemDevice};
//!
//! let dev = MemDevice::new(vec![0u8; 8 << 20], BlockSize::new(512).unwrap());
//! let fs = format(dev, FormatOptions::new().with_label(VolumeLabel::new("DATA")?))?;
//! assert_eq!(fs.kind(), FatKind::Fat12);
//! let vol = Volume::new(fs);
//! vol.write_file("/hello.txt", b"hello")?;
//! # Ok(())
//! # }
//! # #[cfg(not(all(feature = "sync", feature = "write", feature = "std")))]
//! # fn main() {}
//! ```
//!
//! ## Checking with `FatFs`
//!
//! `check` and `check_with` (in each mode, no allocator) read the volume
//! without changing it and report [`Finding`]s and a [`CheckReport`]: boot
//! sector, FSInfo and FAT copy problems, broken, cyclic, cross-linked and
//! lost chains, chains that do not fit their file, bad names, dot entries,
//! misplaced labels and broken long-name runs.
//!
//! ```rust
//! # #[cfg(all(feature = "sync", feature = "write", feature = "std"))]
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use hadris_fat::sync::{check, format};
//! use hadris_fat::FormatOptions;
//! use hadris_storage::{BlockSize, MemDevice};
//!
//! let dev = MemDevice::new(vec![0u8; 8 << 20], BlockSize::new(512).unwrap());
//! let mut fs = format(dev, FormatOptions::new())?;
//! assert!(check(&mut fs)?.is_clean());
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
//! | `alloc`  | No      | Heap-backed conveniences of `hadris-fs`, such as `HeapTable` |
//! | `sync`   | Yes     | Synchronous API in `sync` |
//! | `async`  | No      | Asynchronous API in `r#async` |
//! | `async-send` | No  | Asynchronous API with `Send` futures in `async_send` |
//! | `write`  | Yes     | `format` in each mode; `FatFs` and `ExFatFs` write without it |
//! | `defmt`  | No      | `defmt::Format` for `FatKind` and `Finding` |
//!
//! No feature changes what an item does: `FatFs` always reads and writes long
//! names, and neither `FatFs` nor `ExFatFs` needs an allocator in any mode.
//!
//! ## Sync, async and `Send` async
//!
//! The same source is compiled once per enabled mode: `sync`, `r#async` and
//! `async_send`. Each holds `FatFs`, `check`, `check_with` and, with `write`,
//! `format`. The crate root holds only the mode-independent types.
//!
//! ## Modules
//!
//! - `sync::FatFs`, `r#async::FatFs`, `async_send::FatFs`: the driver
//! - `sync::format` and its `async` versions: the formatter (requires `write`)
//! - `sync::check`, `sync::check_with` and their `async` versions: the
//!   checker
//! - `raw`: on-disk boot sector, BPB, FSInfo and directory entry layouts
//! - `exfat`: the exFAT driver, `ExFatFs`, with its own `sync`, `r#async`
//!   and `async_send` modes, formatter, checker and `raw` layouts

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

mod code_page;
#[cfg_attr(
    not(any(feature = "sync", feature = "async")),
    allow(dead_code, unused_imports)
)]
mod codec;
mod findings;
mod options;
/// Raw on-disk FAT layouts: the boot sector, BPB, FSInfo and the short and
/// long-name directory entries, with their constants.
///
/// The items mirror the FAT specification. The module may gain items; the
/// existing ones follow the specification and stay exhaustive. The crate
/// root never re-exports them.
pub mod raw;

/// The exFAT driver, `ExFatFs`, its formatter and checker.
pub mod exfat;

#[cfg(feature = "sync")]
#[path = ""]
pub mod sync {
    //! The synchronous API.

    macro_rules! io_transform {
        ($($item:tt)*) => { hadris_macros::strip_async!{ $($item)* } };
    }

    use hadris_storage::sync as storage;

    macro_rules! impl_fat_driver {
        ($($t:tt)*) => { hadris_fs::impl_fs_driver!(sync, $($t)*); };
    }

    #[path = "block_io.rs"]
    pub(crate) mod block_io;
    #[path = "fatfs.rs"]
    mod fatfs;
    pub use fatfs::{FatFs, check, check_with};
    #[cfg(feature = "write")]
    #[path = "mkfs.rs"]
    mod mkfs;
    #[cfg(feature = "write")]
    pub use mkfs::format;
}

#[cfg(feature = "async")]
#[path = ""]
pub mod r#async {
    //! The asynchronous API.

    macro_rules! io_transform {
        ($($item:tt)*) => { $($item)* };
    }

    use hadris_storage::r#async as storage;

    macro_rules! impl_fat_driver {
        ($($t:tt)*) => { hadris_fs::impl_fs_driver!(async, $($t)*); };
    }

    #[path = "block_io.rs"]
    pub(crate) mod block_io;
    #[path = "fatfs.rs"]
    mod fatfs;
    pub use fatfs::{FatFs, check, check_with};
    #[cfg(feature = "write")]
    #[path = "mkfs.rs"]
    mod mkfs;
    #[cfg(feature = "write")]
    pub use mkfs::format;
}

/// The asynchronous API with `Send` futures, for generic code on
/// multi-threaded executors.
///
/// Generated a third time from the same source as `r#async`, following
/// `hadris_fs::async_send`. Its [`FatFs`](async_send::FatFs) futures are
/// `Send` when the device is and the node table holds `Send` values, as
/// `FixedTable` and `HeapTable` do.
#[cfg(feature = "async-send")]
pub mod async_send;

pub use code_page::{Ascii, CodePage, Cp437};
pub use codec::entry::FatKind;
pub use findings::{CheckReport, Finding, FindingKind};
#[cfg(feature = "write")]
pub use options::FormatOptions;
pub use options::{MountOptions, VolumeLabel};
