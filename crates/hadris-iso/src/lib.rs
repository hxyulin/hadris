//! Hadris ISO
//! Terminology and spec are followed by the specifications described in
//! the [non official ISO9660 specification included](https://github.com/hxyulin/hadris/tree/main/crates/hadris-iso/spec)

// Known Bugs:
//  - Zero size files causes a lot of issues
//
//  TODO: There is a lot of bugs with mixing file interchanges!!!

#![no_std]

pub mod directory;
pub mod path;
pub mod types;
pub mod volume;

pub mod boot;
pub mod file;
pub mod read;

#[cfg(feature = "write")]
pub mod write;

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

pub mod joliet;
pub mod rrip;
pub mod susp;

pub mod io;
