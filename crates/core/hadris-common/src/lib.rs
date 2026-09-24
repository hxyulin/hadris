//! # Hadris Common
//!
//! Internal support crate for the Hadris filesystem crates. It is not meant
//! for direct use: its API follows the needs of the other Hadris crates and
//! can change in any release. Depend on `hadris` or a format crate instead.
//!
//! It provides endian-aware integers, extents and layout helpers.
//!
//! ## Feature Flags
//!
//! | Feature    | Default | Description |
//! |------------|---------|-------------|
//! | `std`      | yes     | Standard library support (implies `alloc`) |
//! | `alloc`    | via std | Heap allocation (`String`, `Vec` types) |
//! | `bytemuck` | yes     | `Pod` and `Zeroable` for the number and endian types |
//! | `sync`     | no      | Forwarded to `hadris-io` |
//! | `async`    | no      | Forwarded to `hadris-io` |
//!
//! ## Key Types
//!
//! - **Endian numbers**: [`types::number::U16`], [`types::number::U32`],
//!   [`types::number::U64`] — unsigned integers parameterized by endianness.
//! - **Extent**: [`types::extent::Extent`] — a contiguous region on disk
//!   (sector + length).
//! - **Endianness**: [`types::endian::Endianness`], the compile-time byte
//!   order of the number types.
//!
//! ## Example
//!
//! ```rust
//! use hadris_common::types::endian::{Endian, LittleEndian};
//! use hadris_common::types::number::U32;
//!
//! let value = U32::<LittleEndian>::new(0x12345678);
//! assert_eq!(value.get(), 0x12345678);
//! ```

#![no_std]
#![deny(missing_docs)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

/// Types
pub mod types;
