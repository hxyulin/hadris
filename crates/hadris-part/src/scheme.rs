//! Unified partition scheme handling.
//!
//! This module provides a unified API for working with different partition schemes:
//! - MBR (Master Boot Record)
//! - GPT (GUID Partition Table)
//! - Hybrid MBR (GPT with MBR entries for BIOS compatibility)

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

#[cfg(feature = "alloc")]
use crate::error::{PartitionError, Result};
use crate::gpt::Guid;
#[cfg(feature = "alloc")]
use crate::gpt::{GptHeader, GptPartitionEntry};
#[cfg(feature = "alloc")]
use endian_num::Le;
use crate::hybrid::is_hybrid_mbr;
use crate::mbr::MasterBootRecord;

/// The type of partition scheme detected or to be created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartitionSchemeType {
    /// Master Boot Record (legacy BIOS).
    Mbr,
    /// GUID Partition Table (UEFI).
    Gpt,
    /// Hybrid MBR (GPT with MBR entries for dual BIOS/UEFI boot).
    Hybrid,
}

impl core::fmt::Display for PartitionSchemeType {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Mbr => write!(f, "MBR"),
            Self::Gpt => write!(f, "GPT"),
            Self::Hybrid => write!(f, "Hybrid MBR"),
        }
    }
}

/// Information about a partition, independent of the underlying scheme.
#[derive(Debug, Clone, Copy)]
pub struct PartitionInfo {
    /// Partition index (0-based).
    pub index: usize,
    /// Starting LBA.
    pub start_lba: u64,
    /// Ending LBA (inclusive).
    pub end_lba: u64,
    /// Size in sectors.
    pub size_sectors: u64,
    /// Whether the partition is bootable/active.
    pub bootable: bool,
    /// Partition type (MBR type code or GPT type GUID).
    pub partition_type: PartitionType,
}

/// Partition type information.
#[derive(Debug, Clone, Copy)]
pub enum PartitionType {
    /// MBR partition type code.
    Mbr(u8),
    /// GPT partition type GUID.
    Gpt(Guid),
}

impl PartitionInfo {
    /// Returns the size in bytes (assuming 512-byte sectors).
    pub const fn size_bytes(&self) -> u64 {
        self.size_sectors * 512
    }

    /// Returns the size in bytes for a given sector size.
    pub const fn size_bytes_with_sector_size(&self, sector_size: u32) -> u64 {
        self.size_sectors * sector_size as u64
    }
}

/// A complete GPT disk structure.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct GptDisk {
    /// The primary GPT header (at LBA 1).
    pub primary_header: GptHeader,
    /// The backup GPT header (at last LBA).
    pub backup_header: GptHeader,
    /// Partition entries.
    pub entries: Vec<GptPartitionEntry>,
    /// Logical block size in bytes.
    pub block_size: u32,
}

#[cfg(feature = "alloc")]
impl GptDisk {
    /// Default number of partition entries.
    pub const DEFAULT_ENTRY_COUNT: u32 = 128;

    /// Creates a new empty GPT disk structure.
    pub fn new(disk_sectors: u64, block_size: u32) -> Self {
        let entry_count = Self::DEFAULT_ENTRY_COUNT;
        let entry_size = core::mem::size_of::<GptPartitionEntry>() as u32;
        let entries_per_sector = block_size / entry_size;
        let entry_sectors = entry_count.div_ceil(entries_per_sector);

        // First usable LBA is after: MBR (1) + GPT header (1) + entries
        let first_usable = 2 + entry_sectors as u64;
        // Last usable LBA is before: backup entries + backup header (1)
        let last_usable = disk_sectors - 2 - entry_sectors as u64;

        let disk_guid = {
            #[cfg(feature = "rand")]
            {
                Guid::generate_v4()
            }
            #[cfg(not(feature = "rand"))]
            {
                Guid::UNUSED
            }
        };

        #[cfg_attr(not(feature = "crc"), allow(unused_mut))]
        let mut primary_header = GptHeader {
            signature: GptHeader::SIGNATURE,
            revision: Le::<u32>::from_ne(GptHeader::REVISION_1_0),
            header_size: Le::<u32>::from_ne(GptHeader::STANDARD_HEADER_SIZE),
            header_crc32: Le::<u32>::from_ne(0),
            reserved: Le::<u32>::from_ne(0),
            my_lba: Le::<u64>::from_ne(1),
            alternate_lba: Le::<u64>::from_ne(disk_sectors - 1),
            first_usable_lba: Le::<u64>::from_ne(first_usable),
            last_usable_lba: Le::<u64>::from_ne(last_usable),
            disk_guid,
            partition_entry_lba: Le::<u64>::from_ne(2),
            num_partition_entries: Le::<u32>::from_ne(entry_count),
            size_of_partition_entry: Le::<u32>::from_ne(entry_size),
            partition_entry_array_crc32: Le::<u32>::from_ne(0),
        };

        #[cfg_attr(not(feature = "crc"), allow(unused_mut))]
        let mut backup_header = GptHeader {
            my_lba: Le::<u64>::from_ne(disk_sectors - 1),
            alternate_lba: Le::<u64>::from_ne(1),
            partition_entry_lba: Le::<u64>::from_ne(disk_sectors - 1 - entry_sectors as u64),
            ..primary_header
        };

        let entries = alloc::vec![GptPartitionEntry::default(); entry_count as usize];

        // Update CRCs
        #[cfg(feature = "crc")]
        {
            let entries_crc = crate::gpt::calculate_partition_array_crc32(&entries);
            primary_header.partition_entry_array_crc32 = Le::<u32>::from_ne(entries_crc);
            backup_header.partition_entry_array_crc32 = Le::<u32>::from_ne(entries_crc);
            primary_header.update_crc32();
            backup_header.update_crc32();
        }

        Self {
            primary_header,
            backup_header,
            entries,
            block_size,
        }
    }

    /// Returns the number of used (non-empty) partition entries.
    pub fn partition_count(&self) -> usize {
        self.entries.iter().filter(|e| !e.is_unused()).count()
    }

    /// Returns an iterator over non-empty partition entries.
    pub fn partitions(&self) -> impl Iterator<Item = (usize, &GptPartitionEntry)> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.is_unused())
    }

    /// Adds a partition to the first available slot.
    ///
    /// Returns the index of the new partition, or an error if no slots are available.
    pub fn add_partition(&mut self, entry: GptPartitionEntry) -> Result<usize> {
        for (i, slot) in self.entries.iter_mut().enumerate() {
            if slot.is_unused() {
                *slot = entry;
                self.update_crcs();
                return Ok(i);
            }
        }
        Err(PartitionError::TooManyPartitions {
            max: self.entries.len(),
            requested: self.entries.len() + 1,
        })
    }

    /// Validates the GPT structure.
    pub fn validate(&self) -> Result<()> {
        // Check signature
        if !self.primary_header.has_valid_signature() {
            return Err(PartitionError::InvalidGptSignature {
                found: self.primary_header.signature,
            });
        }

        // Check CRCs
        #[cfg(feature = "crc")]
        {
            if !self.primary_header.verify_crc32() {
                return Err(PartitionError::GptHeaderCrcMismatch {
                    expected: self.primary_header.header_crc32.to_ne(),
                    actual: self.primary_header.calculate_crc32(),
                });
            }

            let entries_crc = crate::gpt::calculate_partition_array_crc32(&self.entries);
            if self.primary_header.partition_entry_array_crc32.to_ne() != entries_crc {
                return Err(PartitionError::GptEntriesCrcMismatch {
                    expected: self.primary_header.partition_entry_array_crc32.to_ne(),
                    actual: entries_crc,
                });
            }
        }

        // Check for overlapping partitions
        let used: Vec<_> = self.partitions().collect();
        for i in 0..used.len() {
            for j in (i + 1)..used.len() {
                let (idx1, p1) = used[i];
                let (idx2, p2) = used[j];
                if p1.first_lba.to_ne() <= p2.last_lba.to_ne()
                    && p2.first_lba.to_ne() <= p1.last_lba.to_ne()
                {
                    let overlap_start = p1.first_lba.to_ne().max(p2.first_lba.to_ne());
                    let overlap_end = p1.last_lba.to_ne().min(p2.last_lba.to_ne());
                    return Err(PartitionError::PartitionOverlap {
                        index1: idx1,
                        index2: idx2,
                        overlap_start,
                        overlap_end,
                    });
                }
            }
        }

        // Check partitions are within usable area
        for (idx, entry) in self.partitions() {
            if entry.first_lba.to_ne() < self.primary_header.first_usable_lba.to_ne()
                || entry.last_lba.to_ne() > self.primary_header.last_usable_lba.to_ne()
            {
                return Err(PartitionError::PartitionOutOfBounds {
                    index: idx,
                    partition_end: entry.last_lba.to_ne(),
                    disk_end: self.primary_header.last_usable_lba.to_ne(),
                });
            }
        }

        Ok(())
    }

    /// Updates all CRCs in the headers.
    #[cfg(feature = "crc")]
    pub fn update_crcs(&mut self) {
        let entries_crc = crate::gpt::calculate_partition_array_crc32(&self.entries);
        self.primary_header.partition_entry_array_crc32 = Le::<u32>::from_ne(entries_crc);
        self.backup_header.partition_entry_array_crc32 = Le::<u32>::from_ne(entries_crc);
        self.primary_header.update_crc32();
        self.backup_header.update_crc32();
    }

    #[cfg(not(feature = "crc"))]
    pub fn update_crcs(&mut self) {
        // No-op without CRC feature
    }

    /// Creates a protective MBR for this GPT disk.
    pub fn create_protective_mbr(&self) -> MasterBootRecord {
        let disk_sectors = self.backup_header.my_lba.to_ne().saturating_add(1);
        MasterBootRecord::protective(disk_sectors)
    }
}

/// A unified partition scheme that can be MBR, GPT, or Hybrid.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub enum DiskPartitionScheme {
    /// Pure MBR partitioning.
    Mbr(MasterBootRecord),
    /// GPT partitioning with protective MBR.
    Gpt {
        /// The protective MBR.
        protective_mbr: MasterBootRecord,
        /// The GPT disk structure.
        gpt: GptDisk,
    },
    /// Hybrid MBR + GPT partitioning.
    Hybrid {
        /// The hybrid MBR.
        hybrid_mbr: MasterBootRecord,
        /// The GPT disk structure.
        gpt: GptDisk,
    },
}

#[cfg(feature = "alloc")]
impl DiskPartitionScheme {
    /// Creates a new MBR-only partition scheme.
    pub fn new_mbr() -> Self {
        Self::Mbr(MasterBootRecord::default())
    }

    /// Creates a new GPT partition scheme.
    pub fn new_gpt(disk_sectors: u64, block_size: u32) -> Self {
        let gpt = GptDisk::new(disk_sectors, block_size);
        let protective_mbr = gpt.create_protective_mbr();
        Self::Gpt {
            protective_mbr,
            gpt,
        }
    }

    /// Returns the partition scheme type.
    pub fn scheme_type(&self) -> PartitionSchemeType {
        match self {
            Self::Mbr(_) => PartitionSchemeType::Mbr,
            Self::Gpt { .. } => PartitionSchemeType::Gpt,
            Self::Hybrid { .. } => PartitionSchemeType::Hybrid,
        }
    }

    /// Returns partition information for all partitions.
    pub fn partitions(&self) -> Vec<PartitionInfo> {
        match self {
            Self::Mbr(mbr) => {
                let pt = mbr.get_partition_table();
                pt.partitions
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| !p.is_empty())
                    .map(|(i, p)| PartitionInfo {
                        index: i,
                        start_lba: p.start_lba.to_ne() as u64,
                        end_lba: p.end_lba() as u64,
                        size_sectors: p.sector_count.to_ne() as u64,
                        bootable: p.is_bootable(),
                        partition_type: PartitionType::Mbr(p.part_type),
                    })
                    .collect()
            }
            Self::Gpt { gpt, .. } | Self::Hybrid { gpt, .. } => gpt
                .partitions()
                .map(|(i, e)| PartitionInfo {
                    index: i,
                    start_lba: e.first_lba.to_ne(),
                    end_lba: e.last_lba.to_ne(),
                    size_sectors: e.size_sectors(),
                    bootable: e.attributes.is_legacy_bios_bootable(),
                    partition_type: PartitionType::Gpt(e.type_guid),
                })
                .collect(),
        }
    }

    /// Validates the partition scheme.
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Mbr(mbr) => {
                if !mbr.has_valid_signature() {
                    return Err(PartitionError::InvalidMbrSignature {
                        found: mbr.signature,
                    });
                }
                let pt = mbr.get_partition_table();
                if !pt.is_valid() {
                    return Err(PartitionError::InvalidHybridMbr {
                        reason: "invalid MBR partition table",
                    });
                }
                Ok(())
            }
            Self::Gpt {
                protective_mbr,
                gpt,
            } => {
                if !protective_mbr.has_valid_signature() {
                    return Err(PartitionError::InvalidMbrSignature {
                        found: protective_mbr.signature,
                    });
                }
                let pt = protective_mbr.get_partition_table();
                if !pt.is_protective() {
                    return Err(PartitionError::NoProtectiveMbr);
                }
                gpt.validate()
            }
            Self::Hybrid { hybrid_mbr, gpt } => {
                if !hybrid_mbr.has_valid_signature() {
                    return Err(PartitionError::InvalidMbrSignature {
                        found: hybrid_mbr.signature,
                    });
                }
                if !is_hybrid_mbr(hybrid_mbr) {
                    return Err(PartitionError::InvalidHybridMbr {
                        reason: "not a valid hybrid MBR",
                    });
                }
                gpt.validate()
            }
        }
    }
}

/// Detects the partition scheme type from an MBR.
///
/// This is a preliminary detection based only on the MBR.
/// To fully detect GPT, you need to also read and validate the GPT header.
pub fn detect_scheme_from_mbr(mbr: &MasterBootRecord) -> PartitionSchemeType {
    if !mbr.has_valid_signature() {
        return PartitionSchemeType::Mbr;
    }

    let pt = mbr.get_partition_table();
    if is_hybrid_mbr(mbr) {
        PartitionSchemeType::Hybrid
    } else if pt.is_protective() {
        PartitionSchemeType::Gpt
    } else {
        PartitionSchemeType::Mbr
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mbr::MbrPartition;

    #[cfg(feature = "alloc")]
    #[test]
    fn test_gpt_disk_creation() {
        let disk = GptDisk::new(1000000, 512);
        assert!(disk.primary_header.has_valid_signature());
        assert_eq!(disk.entries.len(), 128);
        assert_eq!(disk.partition_count(), 0);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_scheme_detection() {
        // MBR
        let mbr = MasterBootRecord::default();
        assert_eq!(detect_scheme_from_mbr(&mbr), PartitionSchemeType::Mbr);

        // Protective MBR (GPT)
        let protective = MasterBootRecord::protective(1000000);
        assert_eq!(
            detect_scheme_from_mbr(&protective),
            PartitionSchemeType::Gpt
        );

        // Hybrid MBR
        let mut hybrid = protective;
        hybrid.with_partition_table(|pt| {
            pt[1] = MbrPartition::new(crate::mbr::MbrPartitionType::Fat32, 2048, 100000);
        });
        assert_eq!(detect_scheme_from_mbr(&hybrid), PartitionSchemeType::Hybrid);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_partition_scheme_new_gpt() {
        let scheme = DiskPartitionScheme::new_gpt(1000000, 512);
        assert_eq!(scheme.scheme_type(), PartitionSchemeType::Gpt);
        assert!(scheme.validate().is_ok());
        assert!(scheme.partitions().is_empty());
    }
}
