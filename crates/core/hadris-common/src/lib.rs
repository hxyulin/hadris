//! # Hadris Common
//!
//! Internal support crate for the Hadris filesystem crates. It is not meant
//! for direct use: its API follows the needs of the other Hadris crates and
//! can change in any release. Depend on `hadris` or a format crate instead.
//!
//! It provides endian-aware integers, extents, layout helpers, fixed-capacity
//! byte, text and collection types ([`types::fixed`]), and optical media
//! constants.
//!
//! ## Feature Flags
//!
//! | Feature    | Default | Description |
//! |------------|---------|-------------|
//! | `std`      | yes     | Standard library support (CRC, chrono, rand) |
//! | `alloc`    | via std | Heap allocation (`String`, `Vec` types) |
//! | `bytemuck` | yes     | Zero-copy serialization for number types |
//! | `optical`  | no      | Optical media types for CD/DVD/Blu-ray |
//! | `sync`     | no      | Synchronous I/O (forwarded to `hadris-io`) |
//! | `async`    | no      | Asynchronous I/O (forwarded to `hadris-io`) |
//!
//! ## Key Types
//!
//! - **Endian numbers**: [`types::number::U16`], [`types::number::U32`],
//!   [`types::number::U64`] — unsigned integers parameterized by endianness.
//! - **Extent**: [`types::extent::Extent`] — a contiguous region on disk
//!   (sector + length).
//! - **Fixed-capacity storage**: [`types::fixed::FixedBytes`],
//!   [`types::fixed::FixedStr`], [`types::fixed::FixedUtf16`],
//!   [`types::fixed::ArrayVec`].
//! - **EndianType / Endianness**: [`types::endian::EndianType`],
//!   [`types::endian::Endianness`] — runtime and compile-time endianness.
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

/// Algorithms (requires std for CRC and random)
#[cfg(feature = "std")]
pub mod alg;
/// Types
pub mod types;

/// Optical media types (requires `optical` feature)
#[cfg(feature = "optical")]
pub mod optical;

/// A generic 512-byte boot sector binary.
///
/// When written to the start of a disk image, this boot sector displays a
/// message informing the user that the image is not directly bootable.
///
/// ```rust
/// assert_eq!(hadris_common::BOOT_SECTOR_BIN.len(), 512);
/// // Boot sector signature at end
/// assert_eq!(hadris_common::BOOT_SECTOR_BIN[510], 0x55);
/// assert_eq!(hadris_common::BOOT_SECTOR_BIN[511], 0xAA);
/// ```
pub static BOOT_SECTOR_BIN: &[u8] = include_bytes!("boot_sector.bin");

#[cfg(test)]
mod tests {
    use super::*;

    static_assertions::const_assert!(BOOT_SECTOR_BIN.len() == 512);
}
