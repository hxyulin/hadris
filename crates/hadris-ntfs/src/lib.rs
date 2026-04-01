//! # hadris-ntfs
//!
//! A `no_std`-compatible library for reading NTFS filesystems (read-only).
//!
//! ## Feature Flags
//!
//! | Feature  | Default | Description |
//! |----------|---------|-------------|
//! | `std`    | Yes     | Standard library support (enables `alloc`, `sync`) |
//! | `alloc`  | No      | Heap allocation without full std |
//! | `sync`   | No      | Synchronous API via `hadris-io` sync traits |
//! | `async`  | No      | Asynchronous API via `hadris-io` async traits |
//! | `read`   | Yes     | Read operations (requires `alloc`) |

#![no_std]
#![allow(async_fn_in_trait)]

#[cfg(feature = "std")]
extern crate std;

#[cfg(feature = "alloc")]
extern crate alloc;

// ---------------------------------------------------------------------------
// Shared types (compiled once, not duplicated by sync/async modules)
// ---------------------------------------------------------------------------

pub mod error;
pub mod raw;
#[cfg(feature = "read")]
pub mod attr;

pub use error::{NtfsError, Result};
pub use raw::*;
