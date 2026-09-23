//! Shared filesystem vocabulary for the Hadris crates.
//!
//! This crate defines the mode-independent types every Hadris filesystem
//! speaks: node identity, file types, byte names, timestamps and clocks,
//! metadata, capabilities, error kinds, directory cursors, open options and
//! lexical virtual paths. It performs no I/O.
//!
//! # Features
//!
//! | Feature | Default | Purpose |
//! |---|---:|---|
//! | `alloc` | No | [`OwnedName`] and owned path normalization |
//! | `std` | No | Implies `alloc`; adds [`SystemClock`] |

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
pub use error::ErrorKind;
pub use meta::{Attributes, Metadata, Mode, SetMetadata};
#[cfg(feature = "alloc")]
pub use name::OwnedName;
pub use name::{Name, NameBuf, NameError};
pub use node::{FileType, NodeId};
pub use ops::{DeviceKind, DeviceNumber, NewNode, OpenOptions, OpenOptionsError, RenameFlags};
#[cfg(feature = "std")]
pub use time::SystemClock;
pub use time::{CivilDate, CivilTime, Clock, DateTime, DateTimeError, FileTimes, NoClock};
