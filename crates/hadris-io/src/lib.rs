//! Hadris IO
//!
//! Provides I/O trait abstractions for the Hadris filesystem crates.
//!
//! In `std` mode, synchronous traits re-export from `std::io`.
//! In `no_std` mode, minimal custom trait definitions are provided.
//!
//! The crate exposes `sync` and `async` modules (behind feature flags)
//! containing the respective trait families. The default re-export is
//! from the `sync` module for backwards compatibility.

#![no_std]
#![allow(async_fn_in_trait)]

#[cfg(feature = "std")]
extern crate std;

// ---------------------------------------------------------------------------
// Shared types (always available)
// ---------------------------------------------------------------------------

/// No-std compatible I/O error types.
#[cfg(not(feature = "std"))]
mod error;
#[cfg(not(feature = "std"))]
pub use error::{Error, ErrorKind, Result};

/// Re-export std error types when std is available.
#[cfg(feature = "std")]
pub use std::io::{Error, ErrorKind, Result};

/// Re-export std path types when std is available.
#[cfg(feature = "std")]
pub use std::path::{Path, PathBuf};

/// `SeekFrom` — re-exported from std or defined for no-std.
#[cfg(feature = "std")]
pub use std::io::SeekFrom;

#[cfg(not(feature = "std"))]
mod traits;
#[cfg(not(feature = "std"))]
pub use traits::SeekFrom;

/// Helper macro: short-circuit an `Err` by returning `Some(Err(..))`.
#[macro_export]
macro_rules! try_io_result_option {
    ($expr:expr) => {
        match $expr {
            Ok(val) => val,
            Err(err) => return Some(Err(err)),
        }
    };
}

// ---------------------------------------------------------------------------
// Cursor (shared, works with both sync and async)
// ---------------------------------------------------------------------------

/// A no-std compatible Cursor for reading from byte slices.
pub struct Cursor<'a> {
    data: &'a [u8],
    cursor: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, cursor: 0 }
    }

    pub fn position(&self) -> usize {
        self.cursor
    }

    pub fn set_position(&mut self, pos: usize) {
        self.cursor = pos;
    }

    pub fn get_ref(&self) -> &'a [u8] {
        self.data
    }

    fn read_impl(&mut self, buf: &mut [u8]) -> Result<usize> {
        let to_read = buf.len().min(self.data.len() - self.cursor);
        buf[0..to_read].copy_from_slice(&self.data[self.cursor..self.cursor + to_read]);
        self.cursor += to_read;
        Ok(to_read)
    }

    fn seek_impl(&mut self, pos: SeekFrom) -> Result<u64> {
        let new_pos = match pos {
            SeekFrom::Start(offset) => offset as i64,
            SeekFrom::End(offset) => self.data.len() as i64 + offset,
            SeekFrom::Current(offset) => self.cursor as i64 + offset,
        };

        if new_pos < 0 {
            #[cfg(feature = "std")]
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "invalid seek to negative position",
            ));
            #[cfg(not(feature = "std"))]
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "invalid seek to negative position",
            ));
        }

        self.cursor = new_pos as usize;
        Ok(self.cursor as u64)
    }
}

// ---------------------------------------------------------------------------
// Sync module
// ---------------------------------------------------------------------------

#[cfg(feature = "sync")]
mod sync_api;

/// Synchronous I/O traits.
///
/// Contains [`Read`](sync::Read), [`Write`](sync::Write), [`Seek`](sync::Seek),
/// plus extension traits [`ReadExt`](sync::ReadExt), [`Parsable`](sync::Parsable),
/// [`Writable`](sync::Writable).
#[cfg(feature = "sync")]
pub mod sync {
    pub use super::sync_api::*;
}

// Cursor: sync trait impls
#[cfg(feature = "sync")]
impl sync::Read for Cursor<'_> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.read_impl(buf)
    }
}

#[cfg(all(feature = "sync", not(feature = "std")))]
impl sync::Seek for Cursor<'_> {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        self.seek_impl(pos)
    }
}

#[cfg(all(feature = "sync", feature = "std"))]
impl std::io::Seek for Cursor<'_> {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        // Convert std SeekFrom to our result
        let our_pos = match pos {
            std::io::SeekFrom::Start(n) => SeekFrom::Start(n),
            std::io::SeekFrom::End(n) => SeekFrom::End(n),
            std::io::SeekFrom::Current(n) => SeekFrom::Current(n),
        };
        self.seek_impl(our_pos)
    }
}

// Default re-export for backwards compatibility
#[cfg(feature = "sync")]
pub use sync::*;

// ---------------------------------------------------------------------------
// Async module
// ---------------------------------------------------------------------------

#[cfg(feature = "async")]
mod async_api;

/// Asynchronous I/O traits (using async fn in trait).
///
/// Contains async versions of [`Read`](r#async::Read),
/// [`Write`](r#async::Write), [`Seek`](r#async::Seek),
/// plus async extension traits.
#[cfg(feature = "async")]
pub mod r#async {
    pub use super::async_api::*;
}

// Cursor: async trait impls
#[cfg(feature = "async")]
impl r#async::Read for Cursor<'_> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.read_impl(buf)
    }
}

#[cfg(feature = "async")]
impl r#async::Seek for Cursor<'_> {
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        self.seek_impl(pos)
    }
}
