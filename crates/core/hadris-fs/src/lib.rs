//! Shared filesystem vocabulary and driver traits for the Hadris crates.
//!
//! The crate root defines the mode-independent types every Hadris filesystem
//! speaks: node identity, file types, byte names, timestamps and clocks,
//! metadata, capabilities, errors, directory cursors, open options,
//! node tables, lexical virtual paths, and what checkers report
//! ([`Finding`], [`Severity`], [`CheckReport`]). None of them does I/O.
//!
//! The mode modules (`sync`, `r#async`) hold the driver
//! layer: the `FsDriver` trait that format crates implement, the
//! `FileSystem` trait for shared code, `Volume`, the path resolvers, the
//! `DriverExt` and `PathExt` helpers, and the `File` and `Dir` handles.
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
//! | `alloc` | No | [`OwnedName`], [`PathError`], [`HeapTable`], `copy_tree`, owned path normalization, and the writer input [`tree`] with `ContentReader` and `TreeExt` in each mode |
//! | `std` | No | Implies `alloc`; adds [`SystemClock`], `extract_to_host` and `import_from_host` in `sync`, `Content::path` and `Tree::from_fs`, and conversions to `std::io::Error` |
//! | `sync` | No | The blocking driver layer in `sync` |
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
#[cfg(feature = "contract")]
mod contract;
mod dir;
mod error;
mod extent;
#[cfg(any(feature = "sync", feature = "async"))]
mod forget_queue;
mod fuse;
mod macros;
mod meta;
mod name;
mod node;
mod ops;
pub mod path;
mod table;
mod time;
#[cfg(feature = "alloc")]
pub mod tree;

pub use caps::{Capabilities, CaseSensitivity, FsStats, NameCharset};
pub use check::{CheckReport, Finding, Severity};
#[cfg(feature = "contract")]
pub use contract::ContractViolation;
pub use dir::{DirCursor, DirEntry, DirItem};
#[cfg(feature = "alloc")]
pub use error::PathError;
pub use error::{DetailCode, Errno, Error, ErrorKind, FsResult, Location, MountError};
pub use extent::Extent;
pub use fuse::FuseOnError;
pub use meta::{Attributes, Metadata, Mode, SetMetadata};
#[cfg(feature = "alloc")]
pub use name::OwnedName;
pub use name::{Name, NameBuf, NameError};
pub use node::{FileType, NodeId};
pub use ops::{
    DeviceKind, DeviceNumber, NewNode, OpenOptions, OpenOptionsError, RemoveKind, RenameFlags,
};
#[cfg(feature = "alloc")]
pub use table::HeapTable;
pub use table::{FixedTable, NodeTable, TableFull};
#[cfg(feature = "std")]
pub use time::SystemClock;
pub use time::{CivilDate, CivilTime, Clock, DateTime, DateTimeError, FileTimes, NoClock};

/// The blocking driver traits, [`Volume`](sync::Volume), resolvers, path
/// helpers and handles.
#[cfg(feature = "sync")]
pub mod sync;

/// The asynchronous driver traits, `Volume`, resolvers, path helpers and
/// handles, generated from the same source as [`sync`], with `Send`
/// futures for generic code on multi-threaded executors.
///
/// Every trait has `Send` (and `Sync` when it has `&self` async methods) as
/// a supertrait, so `F: FileSystem + 'static` alone lets a generic function
/// spawn work over `F`. Implementations are written with `async fn`. `Rc`
/// has no impls here.
#[cfg(feature = "async")]
pub mod r#async;
