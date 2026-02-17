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
            revision: GptHeader::REVISION_1_0,
            header_size: GptHeader::STANDARD_HEADER_SIZE,
            header_crc32: 0,
            reserved: 0,
            my_lba: 1,
            alternate_lba: disk_sectors - 1,
            first_usable_lba: first_usable,
            last_usable_lba: last_usable,
            disk_guid,
            partition_entry_lba: 2,
            num_partition_entries: entry_count,
            size_of_partition_entry: entry_size,
            partition_entry_array_crc32: 0,
        };

        #[cfg_attr(not(feature = "crc"), allow(unused_mut))]
        let mut backup_header = GptHeader {
            my_lba: disk_sectors - 1,
            alternate_lba: 1,
            partition_entry_lba: disk_sectors - 1 - entry_sectors as u64,
            ..primary_header
        };

        let entries = alloc::vec![GptPartitionEntry::default(); entry_count as usize];

        // Update CRCs
        #[cfg(feature = "crc")]
        {
            let entries_crc = crate::gpt::calculate_partition_array_crc32(&entries);
            primary_header.partition_entry_array_crc32 = entries_crc;
            backup_header.partition_entry_array_crc32 = entries_crc;
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
                    expected: self.primary_header.header_crc32,
                    actual: self.primary_header.calculate_crc32(),
                });
            }

            let entries_crc = crate::gpt::calculate_partition_array_crc32(&self.entries);
            if self.primary_header.partition_entry_array_crc32 != entries_crc {
                return Err(PartitionError::GptEntriesCrcMismatch {
                    expected: self.primary_header.partition_entry_array_crc32,
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
                if p1.first_lba <= p2.last_lba && p2.first_lba <= p1.last_lba {
                    let overlap_start = p1.first_lba.max(p2.first_lba);
                    let overlap_end = p1.last_lba.min(p2.last_lba);
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
            if entry.first_lba < self.primary_header.first_usable_lba
                || entry.last_lba > self.primary_header.last_usable_lba
            {
                return Err(PartitionError::PartitionOutOfBounds {
                    index: idx,
                    partition_end: entry.last_lba,
                    disk_end: self.primary_header.last_usable_lba,
                });
            }
        }

        Ok(())
    }

    /// Updates all CRCs in the headers.
    #[cfg(feature = "crc")]
    pub fn update_crcs(&mut self) {
        let entries_crc = crate::gpt::calculate_partition_array_crc32(&self.entries);
        self.primary_header.partition_entry_array_crc32 = entries_crc;
        self.backup_header.partition_entry_array_crc32 = entries_crc;
        self.primary_header.update_crc32();
        self.backup_header.update_crc32();
    }

    #[cfg(not(feature = "crc"))]
    pub fn update_crcs(&mut self) {
        // No-op without CRC feature
    }

    /// Creates a protective MBR for this GPT disk.
    pub fn create_protective_mbr(&self) -> MasterBootRecord {
        let disk_sectors = self.backup_header.my_lba + 1;
        MasterBootRecord::protective(disk_sectors)
    }
}

// I/O operations for GptDisk
#[cfg(all(feature = "alloc", feature = "read"))]
impl GptDisk {
    /// Reads a GPT disk structure from a reader.
    ///
    /// Reads the primary GPT header at LBA 1 and the partition entry array.
    /// The reader should be positioned at the beginning of the disk (LBA 0).
    ///
    /// # Arguments
    ///
    /// * `reader` - The reader to read from
    /// * `block_size` - The logical block size in bytes (typically 512)
    ///
    /// # Errors
    ///
    /// Returns an error if reading fails or if the GPT structure is invalid.
    pub fn read_from<R: hadris_io::Read + hadris_io::Seek>(
        reader: &mut R,
        block_size: u32,
    ) -> Result<Self> {
        // Read primary GPT header at LBA 1
        let primary_header = GptHeader::read_from_lba(reader, 1, block_size)?;

        // Validate header CRC if feature enabled
        #[cfg(feature = "crc")]
        if !primary_header.verify_crc32() {
            return Err(PartitionError::GptHeaderCrcMismatch {
                expected: primary_header.header_crc32,
                actual: primary_header.calculate_crc32(),
            });
        }

        // Validate partition entry size
        let entry_size = primary_header.size_of_partition_entry;
        if entry_size != core::mem::size_of::<GptPartitionEntry>() as u32 {
            return Err(PartitionError::InvalidPartitionEntrySize { size: entry_size });
        }

        // Read partition entries
        let num_entries = primary_header.num_partition_entries as usize;
        let mut entries = alloc::vec![GptPartitionEntry::default(); num_entries];

        reader
            .seek(hadris_io::SeekFrom::Start(
                primary_header.partition_entry_lba * block_size as u64,
            ))
            .map_err(|_| PartitionError::Io)?;

        for entry in entries.iter_mut() {
            let mut buf = [0u8; 128];
            reader
                .read_exact(&mut buf)
                .map_err(|_| PartitionError::Io)?;
            *entry = bytemuck::cast(buf);
        }

        // Verify partition array CRC if feature enabled
        #[cfg(feature = "crc")]
        {
            let entries_crc = crate::gpt::calculate_partition_array_crc32(&entries);
            if primary_header.partition_entry_array_crc32 != entries_crc {
                return Err(PartitionError::GptEntriesCrcMismatch {
                    expected: primary_header.partition_entry_array_crc32,
                    actual: entries_crc,
                });
            }
        }

        // Try to read backup header
        let backup_header =
            GptHeader::read_from_lba(reader, primary_header.alternate_lba, block_size).unwrap_or(
                GptHeader {
                    // If backup header read fails, construct it from primary
                    my_lba: primary_header.alternate_lba,
                    alternate_lba: primary_header.my_lba,
                    ..primary_header
                },
            );

        Ok(Self {
            primary_header,
            backup_header,
            entries,
            block_size,
        })
    }
}

#[cfg(all(feature = "alloc", feature = "write"))]
impl GptDisk {
    /// Writes the complete GPT structure to a writer.
    ///
    /// Writes:
    /// 1. Protective MBR at LBA 0
    /// 2. Primary GPT header at LBA 1
    /// 3. Primary partition entry array starting at LBA 2
    /// 4. Backup partition entry array before backup header
    /// 5. Backup GPT header at the last LBA
    ///
    /// # Arguments
    ///
    /// * `writer` - The writer to write to
    ///
    /// # Errors
    ///
    /// Returns an error if writing fails.
    pub fn write_to<W: hadris_io::Write + hadris_io::Seek>(&self, writer: &mut W) -> Result<()> {
        // Write protective MBR at LBA 0
        writer
            .seek(hadris_io::SeekFrom::Start(0))
            .map_err(|_| PartitionError::Io)?;
        let protective_mbr = self.create_protective_mbr();
        protective_mbr.write_to(writer)?;

        // Write primary header at LBA 1
        self.primary_header
            .write_to_lba(writer, 1, self.block_size)?;

        // Write primary partition entries starting at partition_entry_lba
        writer
            .seek(hadris_io::SeekFrom::Start(
                self.primary_header.partition_entry_lba * self.block_size as u64,
            ))
            .map_err(|_| PartitionError::Io)?;

        for entry in &self.entries {
            writer
                .write_all(bytemuck::bytes_of(entry))
                .map_err(|_| PartitionError::Io)?;
        }

        // Write backup partition entries
        writer
            .seek(hadris_io::SeekFrom::Start(
                self.backup_header.partition_entry_lba * self.block_size as u64,
            ))
            .map_err(|_| PartitionError::Io)?;

        for entry in &self.entries {
            writer
                .write_all(bytemuck::bytes_of(entry))
                .map_err(|_| PartitionError::Io)?;
        }

        // Write backup header at last LBA
        self.backup_header
            .write_to_lba(writer, self.backup_header.my_lba, self.block_size)?;

        Ok(())
    }

    /// Writes the complete GPT structure with a custom MBR.
    ///
    /// This is useful for hybrid MBR configurations.
    ///
    /// # Arguments
    ///
    /// * `writer` - The writer to write to
    /// * `mbr` - The MBR to write (protective or hybrid)
    ///
    /// # Errors
    ///
    /// Returns an error if writing fails.
    pub fn write_to_with_mbr<W: hadris_io::Write + hadris_io::Seek>(
        &self,
        writer: &mut W,
        mbr: &MasterBootRecord,
    ) -> Result<()> {
        // Write MBR at LBA 0
        writer
            .seek(hadris_io::SeekFrom::Start(0))
            .map_err(|_| PartitionError::Io)?;
        mbr.write_to(writer)?;

        // Write primary header at LBA 1
        self.primary_header
            .write_to_lba(writer, 1, self.block_size)?;

        // Write primary partition entries
        writer
            .seek(hadris_io::SeekFrom::Start(
                self.primary_header.partition_entry_lba * self.block_size as u64,
            ))
            .map_err(|_| PartitionError::Io)?;

        for entry in &self.entries {
            writer
                .write_all(bytemuck::bytes_of(entry))
                .map_err(|_| PartitionError::Io)?;
        }

        // Write backup partition entries
        writer
            .seek(hadris_io::SeekFrom::Start(
                self.backup_header.partition_entry_lba * self.block_size as u64,
            ))
            .map_err(|_| PartitionError::Io)?;

        for entry in &self.entries {
            writer
                .write_all(bytemuck::bytes_of(entry))
                .map_err(|_| PartitionError::Io)?;
        }

        // Write backup header at last LBA
        self.backup_header
            .write_to_lba(writer, self.backup_header.my_lba, self.block_size)?;

        Ok(())
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
                        start_lba: p.start_lba as u64,
                        end_lba: p.end_lba() as u64,
                        size_sectors: p.sector_count as u64,
                        bootable: p.is_bootable(),
                        partition_type: PartitionType::Mbr(p.part_type),
                    })
                    .collect()
            }
            Self::Gpt { gpt, .. } | Self::Hybrid { gpt, .. } => gpt
                .partitions()
                .map(|(i, e)| PartitionInfo {
                    index: i,
                    start_lba: e.first_lba,
                    end_lba: e.last_lba,
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

// I/O operations for DiskPartitionScheme
#[cfg(all(feature = "alloc", feature = "read"))]
impl DiskPartitionScheme {
    /// Detects and reads a partition scheme from a disk image.
    ///
    /// This method:
    /// 1. Reads the MBR at LBA 0
    /// 2. Detects if it's a protective MBR (GPT) or hybrid MBR
    /// 3. If protective/hybrid, reads the GPT structure
    /// 4. Returns the appropriate partition scheme
    ///
    /// # Arguments
    ///
    /// * `reader` - The reader to read from (should be positioned at LBA 0)
    /// * `block_size` - The logical block size in bytes (typically 512)
    ///
    /// # Errors
    ///
    /// Returns an error if reading fails or if the partition structure is invalid.
    pub fn read_from<R: hadris_io::Read + hadris_io::Seek>(
        reader: &mut R,
        block_size: u32,
    ) -> Result<Self> {
        // Seek to beginning and read MBR
        reader
            .seek(hadris_io::SeekFrom::Start(0))
            .map_err(|_| PartitionError::Io)?;

        let mbr = MasterBootRecord::read_from(reader)?;
        let scheme_type = detect_scheme_from_mbr(&mbr);

        match scheme_type {
            PartitionSchemeType::Mbr => Ok(Self::Mbr(mbr)),
            PartitionSchemeType::Gpt => {
                let gpt = GptDisk::read_from(reader, block_size)?;
                Ok(Self::Gpt {
                    protective_mbr: mbr,
                    gpt,
                })
            }
            PartitionSchemeType::Hybrid => {
                let gpt = GptDisk::read_from(reader, block_size)?;
                Ok(Self::Hybrid {
                    hybrid_mbr: mbr,
                    gpt,
                })
            }
        }
    }
}

#[cfg(all(feature = "alloc", feature = "write"))]
impl DiskPartitionScheme {
    /// Writes the partition scheme to a writer.
    ///
    /// # Arguments
    ///
    /// * `writer` - The writer to write to
    ///
    /// # Errors
    ///
    /// Returns an error if writing fails.
    pub fn write_to<W: hadris_io::Write + hadris_io::Seek>(&self, writer: &mut W) -> Result<()> {
        match self {
            Self::Mbr(mbr) => {
                writer
                    .seek(hadris_io::SeekFrom::Start(0))
                    .map_err(|_| PartitionError::Io)?;
                mbr.write_to(writer)
            }
            Self::Gpt { gpt, .. } => gpt.write_to(writer),
            Self::Hybrid { hybrid_mbr, gpt } => gpt.write_to_with_mbr(writer, hybrid_mbr),
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
