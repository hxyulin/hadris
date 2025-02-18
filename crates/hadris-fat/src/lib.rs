//! A library for working with FAT32 file systems
//! Supports reading and writing to FAT32 file systems,
//! with no-std support
//!
//! When used with no features, the crate act as a place for providing the structures used in the
//! FAT32 file system.
//!
//! ## Cargo Features
//!
//! - **alloc**: Enables the 'alloc' feature, which allows for dynamic allocation of memory
//! - **std**: Enables the 'std' feature, which requires an 'std' environment
//! - **read**: Enables the 'read' feature, which allows for reading from FAT32 file systems
//! - **write**: Enables the 'write' feature, which allows for writing to FAT32 file systems
//! - **lfn**: Enables the 'lfn' feature, which allows for reading and writing long file names,
//! which is an optional extension to the FAT32 specification

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "alloc")]
extern crate alloc;

// TODO: Add support for big endian, because we currently just reinterpret the bytes as little endian

#[cfg(not(target_endian = "little"))]
compile_error!("This crate only supports little endian systems");

pub mod structures;
#[cfg(feature = "read")]
pub mod fs;
#[cfg(feature = "read")]
pub use fs::*;

