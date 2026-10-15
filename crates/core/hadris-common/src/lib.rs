//! # Hadris Common
//!
//! Endian-aware integers for the on-disk layouts of the Hadris raw crates.
//!
//! This is a 0.x support crate, versioned like the raw crates. The public
//! fields of `hadris-fat-raw` layouts use its types, so a breaking change here
//! is a breaking change there; it never appears in the API of a stable 3.x
//! crate. Depend on `hadris` or a format crate unless you read raw layouts.
//!
//! ## Feature Flags
//!
//! | Feature    | Default | Description |
//! |------------|---------|-------------|
//! | `bytemuck` | yes     | `Pod` and `Zeroable` for the number and endian types |
//!
//! ## Key Types
//!
//! - **Endian numbers**: [`types::number::U16`], [`types::number::U32`] and
//!   [`types::number::U64`], unsigned integers stored in a fixed byte order.
//! - **Byte order**: [`types::endian::LittleEndian`], the only
//!   [`types::endian::Endianness`] the layouts need.
//!
//! ## Example
//!
//! ```rust
//! use hadris_common::types::endian::LittleEndian;
//! use hadris_common::types::number::U32;
//!
//! let value = U32::<LittleEndian>::new(0x12345678);
//! assert_eq!(value.get(), 0x12345678);
//! ```

#![no_std]
#![deny(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

/// Types
pub mod types;
