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
//! (`sync::FatFs`, `async_::FatFs`). It mounts any
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
//! `exfat::sync::ExFatFs` and its `async_` twin, each
//! with `check` and, with `write`, `format` and `write`. It needs `alloc`
//! and implements `FileSystem`.
//!
//! ## Firmware: the embedded API
//!
//! [`embedded`] holds `Fat<'mount, D, const FILES: usize = 4>`, a handle-based
//! FAT12/16/32 driver for firmware without an allocator, in
//! `embedded::sync` and `embedded::async_` (over an `async_::BlockDevice`,
//! whose futures need not be `Send`). It is built on the raw layer, not on
//! `FatFs`: one 512-byte block buffer, no node table, ASCII name folding
//! unless asked for Unicode, and under 1 KiB of state with four file
//! slots. [`exfat::embedded`] holds `ExFat`, its read-only exFAT
//! counterpart.
//!
//! ```rust
//! # #[cfg(all(feature = "sync", feature = "write", feature = "std"))]
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use core::ops::ControlFlow;
//!
//! use hadris_fat::embedded::{MountToken, sync::Fat};
//! use hadris_fat::{FatOptions, sync::format};
//! use hadris_fs::{DirCursor, OpenOptions};
//! use hadris_storage::{BlockSize, MemDevice};
//!
//! let mut dev = MemDevice::new(vec![0u8; 4 << 20], BlockSize::new(512).unwrap());
//! format(&mut dev, &FatOptions::new())?;
//! let mut token = MountToken::new();
//! let mut fat: Fat<_> = Fat::mount(dev, &mut token)?;
//! let logs = fat.create_dir_all(fat.root(), "data/logs")?;
//! let log = fat.open(logs, "boot.txt", OpenOptions::new().write().create().append())?;
//! fat.write(&log, b"booted\n")?;
//! fat.close(log)?;
//! fat.list(logs, DirCursor::START, |entry| {
//!     assert!(entry.chars().eq("boot.txt".chars()));
//!     ControlFlow::Continue(())
//! })?;
//! let dev = fat.unmount()?;
//! # let _ = dev;
//! # Ok(())
//! # }
//! # #[cfg(not(all(feature = "sync", feature = "write", feature = "std")))]
//! # fn main() {}
//! ```
//!
//! ## Formatting and writing trees
//!
//! With the `write` feature, `format(&mut dev, &opts)` (in each mode) lays
//! out a FAT12, FAT16 or FAT32 volume and returns its [`Geometry`]; it needs
//! no allocator, and the caller mounts the volume with its own
//! `MountOptions`. [`FatOptions`] sets the variant, size, label, time,
//! seed or serial, sector and cluster size, alignment, partition offset and
//! the other boot sector fields; everything defaults from the device. With
//! `alloc`, `write(dev, &tree, &opts)` formats and copies a
//! `hadris_fs::Tree` into the volume, returning a `hadris_fs::Report`.
//!
//! ```rust
//! # #[cfg(all(feature = "sync", feature = "write", feature = "std"))]
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use hadris_fat::sync::{FatFs, format, write};
//! use hadris_fat::{FatKind, FatOptions, VolumeLabel};
//! use hadris_fs::sync::Volume;
//! use hadris_fs::{Content, MountOptions, Node, OpenOptions, Tree};
//! use hadris_storage::{BlockSize, MemDevice};
//!
//! let mut dev = MemDevice::new(vec![0u8; 8 << 20], BlockSize::new(512).unwrap());
//! let options = FatOptions::new().with_label(VolumeLabel::new("DATA")?);
//! assert_eq!(format(&mut dev, &options)?.kind(), FatKind::Fat12);
//! let vol = Volume::new(FatFs::mount(dev, MountOptions::new())?);
//! let mut file = vol.open("/hello.txt", OpenOptions::new().write().create())?;
//! file.write(b"hello")?;
//! file.close()?;
//!
//! let mut tree = Tree::new();
//! tree.insert("docs/readme.txt", Node::file(Content::bytes("hi")))?;
//! let mut image = Vec::new();
//! let report = write(&mut image, &tree, &options.with_size(4 << 20))?;
//! assert_eq!(report.size(), 4 << 20);
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
//! use hadris_fat::FatOptions;
//! use hadris_storage::{BlockSize, MemDevice};
//!
//! let mut dev = MemDevice::new(vec![0u8; 8 << 20], BlockSize::new(512).unwrap());
//! format(&mut dev, &FatOptions::new())?;
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
//! | `alloc`  | No      | `FatFs`, `ExFatFs` and the tree writers `write`; without it the embedded API, `check`, `format` and the raw layer |
//! | `sync`   | Yes     | Synchronous API in `sync` |
//! | `async`  | No      | Asynchronous API for local and `Send` devices in `async_` |
//! | `write`  | Yes     | `format`, and with `alloc` `write`, in each mode; `FatFs` and `ExFatFs` write without it |
//! | `defmt`  | No      | `defmt::Format` for `FatKind` |
//! | `tracing` | No | Function spans for FAT/exFAT operations and FAT allocation/write paths; enables `std` |
//!
//! No feature changes what an item does: `FatFs` always reads and writes long
//! names.
//!
//! ## Sync and async
//!
//! The same source is compiled once per enabled mode: `sync` and `async_`,
//! whose futures are `Send` when the device and its operation state are. Each holds `FatFs`, `check` and, with `write`, `format` and `write`. The crate root holds only the mode-independent types.
//!
//! ## Modules
//!
//! - `sync::FatFs`, `async_::FatFs`: the driver
//! - `sync::format`, `sync::write` and their `async` versions: the
//!   formatter and the tree writer (require `write`)
//! - `sync::check` and its `async` versions: the checker, from
//!   `hadris-fat-raw`
//! - `exfat`: the exFAT driver, `ExFatFs`, with its own `sync` and
//!   `async_` modes, formatter and checker
//! - `Detail` and `exfat::Detail`: what exactly is wrong with a volume, read
//!   from mount and read errors with `Detail::of`
//!
//! ## The raw layer
//!
//! The on-disk layouts, the I/O-free codecs and the device primitives the
//! drivers are built on are the separate `hadris-fat-raw` crate, which has
//! its own version. This crate re-exports only what its own signatures use:
//! [`FatKind`], [`Geometry`], [`Detail`], `exfat::Geometry`,
//! `exfat::Detail` and the `check` functions.
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

#[cfg(feature = "alloc")]
mod cache;
#[cfg(any(feature = "sync", feature = "async"))]
mod names;
mod options;
#[cfg(feature = "alloc")]
mod table;
#[cfg(feature = "alloc")]
pub use cache::CacheOptions;

#[cfg(feature = "alloc")]
macro_rules! filesystem_impl_for {
    ($driver:ident, $device:path, $trait:path) => {
        io_transform! {
            impl<D: $device> $trait for $driver<D> {
                type DeviceError = D::Error;
                fn capabilities(&self) -> Capabilities {
                    self.capabilities()
                }
                fn root(&self) -> NodeId {
                    self.root()
                }
                async fn statfs(&mut self) -> FsResult<FsStats, D::Error> {
                    self.statfs().await
                }
                async fn label<'b>(&mut self, buf: &'b mut [u8]) -> FsResult<Option<&'b str>, D::Error> {
                    self.label(buf).await
                }
                async fn lookup(&mut self, dir: NodeId, name: &Name) -> FsResult<NodeId, D::Error> {
                    self.lookup(dir, name).await
                }
                fn forget(&mut self, node: NodeId, count: u64) {
                    self.forget(node, count)
                }
                async fn parent(&mut self, dir: NodeId) -> FsResult<NodeId, D::Error> {
                    self.parent(dir).await
                }
                async fn stat(&mut self, node: NodeId) -> FsResult<Metadata, D::Error> {
                    self.stat(node).await
                }
                async fn readdir(
                    &mut self,
                    dir: NodeId,
                    from: DirCursor,
                ) -> FsResult<Option<DirEntry>, D::Error> {
                    self.readdir(dir, from).await
                }
                async fn readlink<'b>(
                    &mut self,
                    node: NodeId,
                    buf: &'b mut [u8],
                ) -> FsResult<&'b [u8], D::Error> {
                    self.readlink(node, buf).await
                }
                async fn open(&mut self, node: NodeId, mode: OpenMode) -> FsResult<(), D::Error> {
                    self.open(node, mode).await
                }
                async fn close(&mut self, node: NodeId) -> FsResult<(), D::Error> {
                    self.close(node).await
                }
                async fn read(
                    &mut self,
                    node: NodeId,
                    offset: u64,
                    buf: &mut [u8],
                ) -> FsResult<usize, D::Error> {
                    self.read(node, offset, buf).await
                }
                async fn setattr(&mut self, node: NodeId, changes: &SetAttr) -> FsResult<(), D::Error> {
                    self.setattr(node, changes).await
                }
                async fn write(&mut self, node: NodeId, offset: u64, buf: &[u8]) -> FsResult<usize, D::Error> {
                    self.write(node, offset, buf).await
                }
                async fn truncate(&mut self, node: NodeId, len: u64) -> FsResult<(), D::Error> {
                    self.truncate(node, len).await
                }
                async fn fsync(&mut self, node: NodeId) -> FsResult<(), D::Error> {
                    self.fsync(node).await
                }
                async fn create(
                    &mut self,
                    dir: NodeId,
                    name: &Name,
                    attrs: &SetAttr,
                ) -> FsResult<NodeId, D::Error> {
                    self.create(dir, name, attrs).await
                }
                async fn mkdir(
                    &mut self,
                    dir: NodeId,
                    name: &Name,
                    attrs: &SetAttr,
                ) -> FsResult<NodeId, D::Error> {
                    self.mkdir(dir, name, attrs).await
                }
                async fn unlink(&mut self, dir: NodeId, name: &Name) -> FsResult<(), D::Error> {
                    self.unlink(dir, name).await
                }
                async fn rmdir(&mut self, dir: NodeId, name: &Name) -> FsResult<(), D::Error> {
                    self.rmdir(dir, name).await
                }
                async fn rename(
                    &mut self,
                    from_dir: NodeId,
                    from: &Name,
                    to_dir: NodeId,
                    to: &Name,
                    mode: RenameMode,
                ) -> FsResult<(), D::Error> {
                    self.rename(from_dir, from, to_dir, to, mode).await
                }
                async fn sync(&mut self) -> FsResult<(), D::Error> {
                    self.sync().await
                }
            }
        }
    };
}
#[cfg(feature = "alloc")]
macro_rules! filesystem_impl {
    ($driver:ident) => {
        filesystem_impl_for!($driver, BlockDevice, FileSystem);
        async_only! {
            filesystem_impl_for!($driver, hadris_storage::async_::SendBlockDevice, hadris_fs::async_::FileSystem);
        }
    };
}

#[cfg(any(feature = "sync", feature = "async"))]
pub mod embedded;
/// The exFAT driver, `ExFatFs`, its formatter and checker.
pub mod exfat;

#[cfg(feature = "sync")]
#[path = ""]
pub mod sync {
    //! The synchronous API.

    #[allow(unused_macros)]
    macro_rules! sync_only { ($($item:tt)*) => { $($item)* }; }
    #[allow(unused_macros)]
    macro_rules! async_only {
        ($($item:tt)*) => {};
    }

    #[allow(unused_macros)]
    macro_rules! io_transform {
        ($($item:tt)*) => { hadris_macros::strip_async!{ $($item)* } };
    }

    use hadris_fat_raw::io::sync as rawio;
    #[cfg(feature = "alloc")]
    use hadris_fs::sync as fsapi;
    #[cfg(any(feature = "alloc", feature = "write"))]
    use hadris_storage::sync as storage;

    #[cfg(any(feature = "alloc", feature = "write"))]
    #[path = "block_io.rs"]
    pub(crate) mod block_io;
    #[cfg(feature = "alloc")]
    #[path = "fatfs.rs"]
    mod fatfs;
    #[cfg(feature = "alloc")]
    pub use fatfs::FatFs;
    pub use rawio::check;
    #[cfg(feature = "write")]
    #[path = "mkfs.rs"]
    pub(crate) mod mkfs;
    #[cfg(feature = "write")]
    pub use mkfs::format;
    #[cfg(all(feature = "alloc", feature = "write"))]
    pub use mkfs::write;
}

/// The asynchronous API for local and multi-threaded executors.
///
/// Generated from the same source as `sync`, following `hadris_fs::async_`.
/// Its `FatFs` futures are `Send` when the device and its operation state are.
#[cfg(feature = "async")]
#[path = "async.rs"]
pub mod async_;

#[cfg(all(feature = "alloc", any(feature = "sync", feature = "async")))]
use names::{permissions, read_only_bit};

/// Adds the run `(file offset, device offset, bytes)` to `out` from
/// `*count` on when it holds bytes of the file from `from`, cut to the
/// file's `len` and split where its bytes past `valid` read as zeros.
/// Returns true when `out` was full before the run was added whole.
#[cfg(all(feature = "alloc", any(feature = "sync", feature = "async")))]
fn push_run(
    out: &mut [hadris_fs::Extent],
    count: &mut usize,
    (file, disk, bytes): (u64, u64, u64),
    from: u64,
    len: u64,
    valid: u64,
) -> bool {
    let end = file + bytes.min(len - file);
    let split = valid.clamp(file, end);
    for (start, stop, unwritten) in [(file, split, false), (split, end, true)] {
        if stop <= start || stop <= from {
            continue;
        }
        let Some(slot) = out.get_mut(*count) else {
            return true;
        };
        let extent =
            hadris_fs::Extent::new(disk + (start - file), stop - start).with_file_offset(start);
        *slot = if unwritten {
            extent.with_unwritten()
        } else {
            extent
        };
        *count += 1;
    }
    false
}

pub use hadris_fat_raw::{Detail, FatKind, Geometry};
#[cfg(feature = "write")]
pub use options::FatOptions;
pub use options::VolumeLabel;

/// Compatibility alias for the asynchronous API.
#[cfg(feature = "async")]
pub use async_ as r#async;
