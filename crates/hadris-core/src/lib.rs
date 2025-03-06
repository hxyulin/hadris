//! hadris-core is the core library for the Hadris filesystem library.
//!
//! This library contains the core types and traits for Hadris, as well as some helper functions
//! and utilities.
//! Types include:
//! [`File`]: A file on the filesystem.
//! [`FileAttributes`]: Attributes of a file.
//! [`FileSystem`]: A trait for interacting with a filesystem.
//! [`FileSystemError`]: An error type for interacting with a filesystem.
//! [`Path`]: A path to a file on the filesystem.
//! [`UtcTime`]: A UTC time.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "alloc")]
extern crate alloc;

pub mod bpb;
pub mod disk;
pub mod file;
pub mod path;
pub mod str;
pub mod time;

/// Errors that can occur when interacting with a filesystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FileSystemError {
    /// An error occurred while interacting with the disk.
    #[error("Disk error: {0}")]
    DiskError(#[from] disk::DiskError),
    #[error("File error: {0}")]
    /// An error occurred while interacting with the disk.
    FileError(#[from] file::FileError),
    #[error("Operation not supported")]
    /// The operation is not supported by the filesystem.
    OperationNotSupported,
}

/// Errors that can occur when creating or reading a filesystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FsCreationError {
    #[error("Disk error: {0}")]
    /// An error occurred while interacting with the disk.
    DiskError(#[from] disk::DiskError),
    #[error("File error: {0}")]
    InvalidFileSystem(&'static str),
}
