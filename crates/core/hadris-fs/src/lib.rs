//! Shared filesystem vocabulary for the Hadris crates.
//!
//! This crate defines the mode-independent types every Hadris filesystem
//! speaks: node identity, file types, byte names, timestamps and clocks,
//! metadata, capabilities, errors, directory cursors, open options and
//! lexical virtual paths. It performs no I/O.
//!
//! [`Error<E>`] is the error of every filesystem operation. `E` is the
//! device's own error, so it survives without allocation; [`AnyError`]
//! erases it for code that mixes devices.
//!
//! # Features
//!
//! | Feature | Default | Purpose |
//! |---|---:|---|
//! | `alloc` | No | [`OwnedName`], [`AnyError`] and owned path normalization |
//! | `std` | No | Implies `alloc`; adds [`SystemClock`] and conversions to `std::io::Error` |

#![no_std]
#![deny(missing_docs)]

#[cfg(any(feature = "alloc", test))]
extern crate alloc;
#[cfg(any(feature = "std", test))]
extern crate std;

mod caps;
mod dir;
mod error;
mod meta;
mod name;
mod node;
mod ops;
pub mod path;
mod time;

pub use caps::{Capabilities, CaseSensitivity, FsStats, NameCharset};
pub use dir::{DirCursor, DirEntry};
#[cfg(feature = "alloc")]
pub use error::AnyError;
pub use error::{Error, ErrorKind, FsResult};
pub use meta::{Attributes, Metadata, Mode, SetMetadata};
#[cfg(feature = "alloc")]
pub use name::OwnedName;
pub use name::{Name, NameBuf, NameError};
pub use node::{FileType, NodeId};
pub use ops::{DeviceKind, DeviceNumber, NewNode, OpenOptions, OpenOptionsError, RenameFlags};
#[cfg(feature = "std")]
pub use time::SystemClock;
pub use time::{CivilDate, CivilTime, Clock, DateTime, DateTimeError, FileTimes, NoClock};
