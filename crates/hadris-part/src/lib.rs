//! Partition table support for MBR, GPT, and Hybrid MBR.
//!
//! This crate provides types and utilities for working with disk partition tables:
//!
//! - **MBR (Master Boot Record)**: Legacy BIOS partition table format supporting up to 4 primary
//!   partitions. See the [`mbr`] module.
//!
//! - **GPT (GUID Partition Table)**: Modern UEFI partition table format supporting up to 128
//!   partitions with GUIDs for type identification. See the [`gpt`] module.
//!
//! - **Hybrid MBR**: A non-standard configuration that combines GPT with MBR entries for
//!   dual BIOS/UEFI compatibility. See the [`hybrid`] module.
//!
//! # Features
//!
//! - `std` (default): Enables standard library support and includes `alloc`.
//! - `alloc`: Enables heap allocation for `Vec`-based APIs (e.g., `GptDisk`, `DiskPartitionScheme`).
//! - `read`: Enables reading partition tables (currently a marker feature).
//! - `write`: Enables writing partition tables (requires `alloc`).
//! - `crc`: Enables CRC32 calculation for GPT headers (via the `crc` crate).
//! - `rand`: Enables random GUID generation (via the `rand` crate).
//!
//! # Examples
//!
//! ## Creating a protective MBR for a GPT disk
//!
//! ```rust
//! use hadris_part::mbr::MasterBootRecord;
//!
//! // Create a protective MBR for a 1TB disk (in 512-byte sectors)
//! let disk_sectors = 1_953_525_168u64; // ~1TB
//! let mbr = MasterBootRecord::protective(disk_sectors);
//!
//! assert!(mbr.has_valid_signature());
//! assert!(mbr.get_partition_table().is_protective());
//! ```
//!
//! ## Working with GPT partition entries
//!
//! ```rust
//! use hadris_part::gpt::{Guid, GptPartitionEntry};
//!
//! // Create an EFI System Partition entry
//! let esp = GptPartitionEntry::new(
//!     Guid::EFI_SYSTEM,
//!     Guid::UNUSED, // Would normally be a unique GUID
//!     2048,         // Start at 1MB (2048 * 512 bytes)
//!     206847,       // ~100MB partition
//! );
//!
//! assert!(!esp.is_unused());
//! assert_eq!(esp.size_sectors(), 204800);
//! ```

#![no_std]
#![allow(async_fn_in_trait)]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

// ---------------------------------------------------------------------------
// Shared types (compiled once, not duplicated by sync/async modules)
// ---------------------------------------------------------------------------

pub mod error;
pub mod geometry;
pub mod gpt;
pub mod hybrid;
pub mod mbr;
pub mod scheme;

// ---------------------------------------------------------------------------
// Sync module
// ---------------------------------------------------------------------------

#[cfg(feature = "sync")]
#[path = ""]
pub mod sync {
    //! Synchronous partition table API.
    //!
    //! All I/O operations use synchronous `Read`/`Write`/`Seek` traits.

    pub use hadris_io::Result as IoResult;
    pub use hadris_io::sync::{Parsable, Read, ReadExt, Seek, Writable, Write};
    pub use hadris_io::{Error, ErrorKind, SeekFrom};

    macro_rules! io_transform {
        ($($item:tt)*) => { hadris_macros::strip_async!{ $($item)* } };
    }

    #[allow(unused_macros)]
    macro_rules! sync_only {
        ($($item:tt)*) => { $($item)* };
    }

    #[allow(unused_macros)]
    macro_rules! async_only {
        ($($item:tt)*) => {};
    }

    #[path = "."]
    mod __inner {
        pub mod gpt_io;
        pub mod mbr_io;
        pub mod scheme_io;
    }
    pub use __inner::*;
}

// ---------------------------------------------------------------------------
// Async module
// ---------------------------------------------------------------------------

#[cfg(feature = "async")]
#[path = ""]
pub mod r#async {
    //! Asynchronous partition table API.
    //!
    //! All I/O operations use async `Read`/`Write`/`Seek` traits.

    pub use hadris_io::Result as IoResult;
    pub use hadris_io::r#async::{Parsable, Read, ReadExt, Seek, Writable, Write};
    pub use hadris_io::{Error, ErrorKind, SeekFrom};

    macro_rules! io_transform {
        ($($item:tt)*) => { $($item)* };
    }

    #[allow(unused_macros)]
    macro_rules! sync_only {
        ($($item:tt)*) => {};
    }

    #[allow(unused_macros)]
    macro_rules! async_only {
        ($($item:tt)*) => { $($item)* };
    }

    #[path = "."]
    mod __inner {
        pub mod gpt_io;
        pub mod mbr_io;
        pub mod scheme_io;
    }
    pub use __inner::*;
}

// ---------------------------------------------------------------------------
// Default re-exports for backwards compatibility (sync)
// ---------------------------------------------------------------------------

#[cfg(feature = "sync")]
pub use sync::*;

// Re-export commonly used types at the crate root
pub use endian_num::Le;
pub use error::{PartitionError, Result};
pub use geometry::{DiskGeometry, validate_partition_alignment};
pub use gpt::{GptHeader, GptPartitionEntry, Guid};
pub use mbr::{Chs, MasterBootRecord, MbrPartition, MbrPartitionTable, MbrPartitionType};
pub use scheme::{PartitionInfo, PartitionSchemeType, PartitionType};

#[cfg(feature = "alloc")]
pub use scheme::{DiskPartitionScheme, GptDisk};

/// Trait for types that represent partition information.
///
/// This trait provides a common interface for accessing basic partition properties
/// regardless of the underlying partition table format (MBR or GPT).
pub trait PartitionInfoTrait {
    /// Returns the starting LBA of the partition.
    fn start_lba(&self) -> u64;

    /// Returns the size of the partition in sectors.
    fn size_sectors(&self) -> u64;

    /// Returns the ending LBA of the partition (inclusive).
    fn end_lba(&self) -> u64 {
        let size = self.size_sectors();
        if size == 0 {
            self.start_lba()
        } else {
            self.start_lba() + size - 1
        }
    }

    /// Returns the size of the partition in bytes (assuming 512-byte sectors).
    fn size_bytes(&self) -> u64 {
        self.size_sectors() * 512
    }

    /// Returns the size of the partition in bytes for a given sector size.
    fn size_bytes_with_sector_size(&self, sector_size: u32) -> u64 {
        self.size_sectors() * sector_size as u64
    }
}

impl PartitionInfoTrait for MbrPartition {
    fn start_lba(&self) -> u64 {
        self.start_lba.to_ne() as u64
    }

    fn size_sectors(&self) -> u64 {
        self.sector_count.to_ne() as u64
    }
}

impl PartitionInfoTrait for GptPartitionEntry {
    fn start_lba(&self) -> u64 {
        self.first_lba.to_ne()
    }

    fn size_sectors(&self) -> u64 {
        let first = self.first_lba.to_ne();
        let last = self.last_lba.to_ne();
        if self.is_unused() || last < first {
            0
        } else {
            last - first + 1
        }
    }

    fn end_lba(&self) -> u64 {
        self.last_lba.to_ne()
    }
}

impl PartitionInfoTrait for PartitionInfo {
    fn start_lba(&self) -> u64 {
        self.start_lba
    }

    fn size_sectors(&self) -> u64 {
        self.size_sectors
    }

    fn end_lba(&self) -> u64 {
        self.end_lba
    }
}

/// Trait for partition table types that support reading.
pub trait PartitionTableRead {
    /// The partition entry type for this table.
    type Partition: PartitionInfoTrait;

    /// Returns the number of partitions in the table.
    fn partition_count(&self) -> usize;

    /// Returns a reference to a partition by index.
    fn partition(&self, index: usize) -> Option<&Self::Partition>;
}

impl PartitionTableRead for MbrPartitionTable {
    type Partition = MbrPartition;

    fn partition_count(&self) -> usize {
        self.count()
    }

    fn partition(&self, index: usize) -> Option<&Self::Partition> {
        if index < 4 && !self.partitions[index].is_empty() {
            Some(&self.partitions[index])
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mbr_partition_trait() {
        let partition = MbrPartition::new(MbrPartitionType::Fat32, 2048, 204800);
        assert_eq!(partition.start_lba(), 2048);
        assert_eq!(partition.size_sectors(), 204800);
        assert_eq!(partition.end_lba(), 2048 + 204800 - 1);
        assert_eq!(partition.size_bytes(), 204800 * 512);
    }

    #[test]
    fn test_gpt_partition_trait() {
        let partition = GptPartitionEntry::new(Guid::EFI_SYSTEM, Guid::UNUSED, 2048, 206847);
        assert_eq!(partition.start_lba(), 2048);
        assert_eq!(partition.size_sectors(), 204800);
        assert_eq!(partition.end_lba(), 206847);
    }

    #[test]
    fn test_mbr_table_read_trait() {
        let mut table = MbrPartitionTable::new();
        table[0] = MbrPartition::new(MbrPartitionType::Fat32, 2048, 204800);
        table[1] = MbrPartition::new(MbrPartitionType::LinuxNative, 206848, 1000000);

        assert_eq!(table.partition_count(), 2);
        assert!(table.partition(0).is_some());
        assert!(table.partition(1).is_some());
        assert!(table.partition(2).is_none());
        assert!(table.partition(3).is_none());
    }

    #[test]
    fn test_struct_sizes() {
        // Verify that our structures have the expected sizes
        assert_eq!(core::mem::size_of::<MbrPartition>(), 16);
        assert_eq!(core::mem::size_of::<MbrPartitionTable>(), 64);
        assert_eq!(core::mem::size_of::<MasterBootRecord>(), 512);
        // GptHeader uses native alignment so it may be larger than 92 bytes.
        // The on-disk format is 92 bytes; serialization should handle this.
        assert!(core::mem::size_of::<GptHeader>() >= 92);
        assert_eq!(core::mem::size_of::<GptPartitionEntry>(), 128);
    }
}
