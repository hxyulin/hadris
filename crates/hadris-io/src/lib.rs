//! Hadris IO
//!
//! This provides the std::io implementations for no-std environments.
//! For use with std, the standard library types are re-exported.

#![no_std]

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;
#[cfg(feature = "std")]
pub use std::io::{Error, ErrorKind, Result, Read, Write, Seek, SeekFrom};

#[cfg(not(feature = "std"))]
mod error;
#[cfg(not(feature = "std"))]
pub use error::Error;
