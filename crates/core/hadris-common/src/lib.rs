//! # Hadris Common
//!
//! Internal support crate for the Hadris filesystem crates. It is not meant
//! for direct use: its API follows the needs of the other Hadris crates and
//! can change in any release. Depend on `hadris` or a format crate instead.
//!
//! It provides endian-aware integers, which the FAT and NTFS crates use
//! for their on-disk layouts.
//!
//! ## Feature Flags
//!
//! | Feature    | Default | Description |
//! |------------|---------|-------------|
//! | `bytemuck` | yes     | `Pod` and `Zeroable` for the number and endian types |
//!
//! ## Key Types
//!
//! - **Endian numbers**: [`types::number::U16`], [`types::number::U32`],
//!   [`types::number::U64`], unsigned integers parameterized by endianness.
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

/// Types
pub mod types;
