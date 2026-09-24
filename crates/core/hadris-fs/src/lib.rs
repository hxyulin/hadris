//! Shared filesystem vocabulary and the filesystem trait for the Hadris
//! crates.
//!
//! The crate root defines the mode-independent types every Hadris filesystem
//! speaks: node identity, file types, byte names, timestamps and clocks,
//! metadata and attribute changes, capabilities, errors, directory entries
//! and cursors, open and mount options, path policies, FAT code pages, and what checkers
//! report ([`Finding`], [`Severity`], [`CheckReport`]). None of them does
//! I/O.
//!
//! The mode modules (`sync`, `r#async`) hold what does: the `FileSystem`
//! trait every format implements, on node ids and `&mut self`; `Volume`,
//! which shares a filesystem between threads and tasks with paths and
//! `File` and `ReadDir` handles named after `std::fs`; and `copy_tree`.
//!
//! [`Error<E>`] is the error of every filesystem operation, re-exported from
//! `hadris-io` with [`ErrorKind`], [`Location`], [`DetailCode`] and
//! [`Errno`], so block devices return the same type. `E` is the device's own
//! error, so it survives without allocation. [`PathError`] (`alloc`) erases
//! it and adds the path that failed, for writers and code that mixes
//! devices. A driver that takes its device
//! by value fails to mount or format with [`MountError`], which gives the
//! device back.
//!
//! # Features
//!
//! | Feature | Default | Purpose |
//! |---|---:|---|
//! | `alloc` | No | [`OwnedName`], [`PathError`], `copy_tree`, the async `Volume`, and the writer input [`tree`] with `ContentReader` and `TreeExt` in each mode |
//! | `std` | No | Implies `alloc`; adds [`SystemClock`], the sync `Volume` and its `std::io` handles, `extract_to_host` and `import_from_host` in `sync`, `Content::path` and `Tree::from_fs`, and conversions to `std::io::Error` |
//! | `sync` | No | The blocking API in `sync` |
//! | `async` | No | The same API with `Send` futures in `r#async` |
//! | `contract` | No | The driver contract kit, `contract::check` in each mode, and `ContractViolation` |
//!
//! No feature changes what an item does.

#![no_std]
#![deny(missing_docs)]
#![allow(async_fn_in_trait)]

#[cfg(any(feature = "alloc", test))]
extern crate alloc;
#[cfg(any(feature = "std", test))]
extern crate std;

mod caps;
mod check;
mod code_page;
#[cfg(feature = "contract")]
mod contract;
mod dir;
mod error;
mod extent;
mod fuse;
mod meta;
mod mount;
mod name;
mod node;
mod ops;
mod time;
#[cfg(feature = "alloc")]
pub mod tree;

pub use caps::{Capabilities, CaseRule, Charset, Field, FsStats, Stored};
pub use check::{CheckReport, Finding, Severity};
pub use code_page::{Ascii, CodePage, Cp437};
#[cfg(feature = "contract")]
pub use contract::ContractViolation;
pub use dir::{DirCursor, DirEntry};
#[cfg(feature = "alloc")]
pub use error::PathError;
pub use error::{DetailCode, Errno, Error, ErrorKind, FsResult, Location, MountError};
pub use extent::Extent;
pub use fuse::FuseOnError;
pub use hadris_io::SeekFrom;
pub use meta::{Attributes, Metadata, Owner, Permissions, SetAttr, SetMetadata};
pub use mount::MountOptions;
#[cfg(feature = "alloc")]
pub use name::OwnedName;
pub use name::{Name, NameBuf, NameError};
pub use node::{FileType, NodeId};
pub use ops::{
    DeviceKind, DeviceNumber, OpenMode, OpenOptions, OpenOptionsError, RenameMode, Resolve,
};
#[cfg(feature = "std")]
pub use time::SystemClock;
pub use time::{CivilDate, CivilTime, Clock, DateTime, DateTimeError, FileTimes, NoClock};

/// The blocking API: the `FileSystem` trait, and with `std` the `Volume`
/// with its `File` and `ReadDir` handles.
#[cfg(feature = "sync")]
pub mod sync;

/// The asynchronous API, generated from the same source as [`sync`], with
/// `Send` futures for generic code on multi-threaded executors.
///
/// Every trait has `Send` (and `Sync` when it has `&self` async methods) as
/// a supertrait, so `F: FileSystem + 'static` alone lets a generic function
/// spawn work over `F`. Implementations are written with `async fn`. The
/// `Volume` needs `alloc`.
#[cfg(feature = "async")]
pub mod r#async;
