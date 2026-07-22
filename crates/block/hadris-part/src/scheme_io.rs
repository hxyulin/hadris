io_transform! {

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(all(feature = "alloc", feature = "read"))]
use super::super::Read;
#[cfg(all(feature = "alloc", any(feature = "read", feature = "write")))]
use super::super::{Seek, SeekFrom};
#[cfg(all(feature = "alloc", feature = "write"))]
use super::super::Write;
#[cfg(all(feature = "alloc", any(feature = "read", feature = "write")))]
use crate::error::{Error, Result};
#[cfg(all(feature = "alloc", feature = "read"))]
use crate::gpt::{GptHeader, GptPartitionEntry};
#[cfg(all(feature = "alloc", feature = "read"))]
use crate::mbr::MasterBootRecord;
#[cfg(all(feature = "alloc", feature = "read"))]
use crate::scheme::{PartitionSchemeType, detect_scheme_from_mbr};

#[cfg(all(feature = "alloc", feature = "read"))]
use super::gpt_io::GptHeaderReadExt;
#[cfg(all(feature = "alloc", feature = "write"))]
use super::gpt_io::GptHeaderWriteExt;
#[cfg(all(feature = "alloc", feature = "read"))]
use super::mbr_io::MasterBootRecordReadExt;
#[cfg(all(feature = "alloc", feature = "write"))]
use super::mbr_io::MasterBootRecordWriteExt;

#[cfg(feature = "alloc")]
use crate::scheme::GptDisk;

#[cfg(feature = "alloc")]
use crate::scheme::PartitionTable;

// I/O operations for GptDisk

/// Extension trait for reading [`GptDisk`] from I/O sources.
#[cfg(all(feature = "alloc", feature = "read"))]
#[cfg_attr(docsrs, doc(cfg(all(feature = "alloc", feature = "read"))))]
pub trait GptDiskReadExt: Sized {
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
    async fn read_from<R: Read + Seek>(
        reader: &mut R,
        block_size: u32,
    ) -> Result<Self>;
}

#[cfg(all(feature = "alloc", feature = "read"))]
impl GptDiskReadExt for GptDisk {
    async fn read_from<R: Read + Seek>(
        reader: &mut R,
        block_size: u32,
    ) -> Result<Self> {
        // Read primary GPT header at LBA 1
        let primary_header = GptHeader::read_from_lba(reader, 1, block_size).await?;

        // Validate header CRC if feature enabled
        #[cfg(feature = "crc")]
        if !primary_header.verify_crc32() {
            return Err(Error::GptHeaderCrcMismatch {
                expected: primary_header.header_crc32.to_ne(),
                actual: primary_header.calculate_crc32(),
            });
        }

        // Validate partition entry size
        let entry_size = primary_header.size_of_partition_entry.to_ne();
        if entry_size != core::mem::size_of::<GptPartitionEntry>() as u32 {
            return Err(Error::InvalidPartitionEntrySize { size: entry_size });
        }

        // Read partition entries
        let num_entries = primary_header.num_partition_entries.to_ne() as usize;
        let mut entries = alloc::vec![GptPartitionEntry::default(); num_entries];

        reader
            .seek(SeekFrom::Start(
                primary_header.partition_entry_lba.to_ne() * block_size as u64,
            ))
            .await
            .map_err(Error::from)?;

        for entry in entries.iter_mut() {
            let mut buf = [0u8; 128];
            reader
                .read_exact(&mut buf)
                .await
                .map_err(Error::from)?;
            *entry = bytemuck::cast(buf);
        }

        // Verify partition array CRC if feature enabled
        #[cfg(feature = "crc")]
        {
            let entries_crc = crate::gpt::calculate_partition_array_crc32(&entries);
            if primary_header.partition_entry_array_crc32.to_ne() != entries_crc {
                return Err(Error::GptEntriesCrcMismatch {
                    expected: primary_header.partition_entry_array_crc32.to_ne(),
                    actual: entries_crc,
                });
            }
        }

        let backup_lba = primary_header.alternate_lba.to_ne();
        let backup_header = match GptHeader::read_from_lba(reader, backup_lba, block_size).await {
            Ok(header) => header,
            Err(Error::Io(source)) => {
                return Err(Error::BackupHeaderIo {
                    lba: backup_lba,
                    source,
                });
            }
            Err(Error::InvalidGptSignature { found }) => {
                return Err(Error::InvalidBackupGptSignature { found });
            }
            Err(error) => return Err(error),
        };

        #[cfg(feature = "crc")]
        if !backup_header.verify_crc32() {
            return Err(Error::BackupGptHeaderCrcMismatch {
                expected: backup_header.header_crc32.to_ne(),
                actual: backup_header.calculate_crc32(),
            });
        }

        let entry_array_bytes = u64::from(primary_header.num_partition_entries.to_ne())
            .checked_mul(u64::from(primary_header.size_of_partition_entry.to_ne()))
            .ok_or(Error::BackupHeaderMismatch)?;
        let entry_array_blocks = entry_array_bytes.div_ceil(u64::from(block_size));
        let expected_backup_entries_lba = backup_lba
            .checked_sub(entry_array_blocks)
            .ok_or(Error::BackupHeaderMismatch)?;

        if backup_header.my_lba != primary_header.alternate_lba
            || backup_header.alternate_lba != primary_header.my_lba
            || backup_header.revision != primary_header.revision
            || backup_header.header_size != primary_header.header_size
            || backup_header.first_usable_lba != primary_header.first_usable_lba
            || backup_header.last_usable_lba != primary_header.last_usable_lba
            || backup_header.disk_guid != primary_header.disk_guid
            || backup_header.num_partition_entries != primary_header.num_partition_entries
            || backup_header.size_of_partition_entry != primary_header.size_of_partition_entry
            || backup_header.partition_entry_array_crc32
                != primary_header.partition_entry_array_crc32
            || backup_header.partition_entry_lba.to_ne() != expected_backup_entries_lba
        {
            return Err(Error::BackupHeaderMismatch);
        }

        Ok(Self {
            primary_header,
            backup_header,
            entries,
            block_size,
        })
    }
}

/// Extension trait for writing [`GptDisk`] to I/O sinks.
#[cfg(all(feature = "alloc", feature = "write"))]
#[cfg_attr(docsrs, doc(cfg(all(feature = "alloc", feature = "write"))))]
pub trait GptDiskWriteExt {
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
    async fn write_to<W: Write + Seek>(&self, writer: &mut W) -> Result<()>;

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
    async fn write_to_with_mbr<W: Write + Seek>(
        &self,
        writer: &mut W,
        mbr: &MasterBootRecord,
    ) -> Result<()>;
}

#[cfg(all(feature = "alloc", feature = "write"))]
impl GptDiskWriteExt for GptDisk {
    async fn write_to<W: Write + Seek>(&self, writer: &mut W) -> Result<()> {
        // Write protective MBR at LBA 0
        writer
            .seek(SeekFrom::Start(0))
            .await
            .map_err(Error::from)?;
        let protective_mbr = self.create_protective_mbr();
        protective_mbr.write_to(writer).await?;

        // Write primary header at LBA 1
        self.primary_header
            .write_to_lba(writer, 1, self.block_size)
            .await?;

        // Write primary partition entries starting at partition_entry_lba
        writer
            .seek(SeekFrom::Start(
                self.primary_header.partition_entry_lba.to_ne() * self.block_size as u64,
            ))
            .await
            .map_err(Error::from)?;

        for entry in &self.entries {
            writer
                .write_all(bytemuck::bytes_of(entry))
                .await
                .map_err(Error::from)?;
        }

        // Write backup partition entries
        writer
            .seek(SeekFrom::Start(
                self.backup_header.partition_entry_lba.to_ne() * self.block_size as u64,
            ))
            .await
            .map_err(Error::from)?;

        for entry in &self.entries {
            writer
                .write_all(bytemuck::bytes_of(entry))
                .await
                .map_err(Error::from)?;
        }

        // Write backup header at last LBA
        self.backup_header
            .write_to_lba(writer, self.backup_header.my_lba.to_ne(), self.block_size)
            .await?;

        Ok(())
    }

    async fn write_to_with_mbr<W: Write + Seek>(
        &self,
        writer: &mut W,
        mbr: &MasterBootRecord,
    ) -> Result<()> {
        // Write MBR at LBA 0
        writer
            .seek(SeekFrom::Start(0))
            .await
            .map_err(Error::from)?;
        mbr.write_to(writer).await?;

        // Write primary header at LBA 1
        self.primary_header
            .write_to_lba(writer, 1, self.block_size)
            .await?;

        // Write primary partition entries
        writer
            .seek(SeekFrom::Start(
                self.primary_header.partition_entry_lba.to_ne() * self.block_size as u64,
            ))
            .await
            .map_err(Error::from)?;

        for entry in &self.entries {
            writer
                .write_all(bytemuck::bytes_of(entry))
                .await
                .map_err(Error::from)?;
        }

        // Write backup partition entries
        writer
            .seek(SeekFrom::Start(
                self.backup_header.partition_entry_lba.to_ne() * self.block_size as u64,
            ))
            .await
            .map_err(Error::from)?;

        for entry in &self.entries {
            writer
                .write_all(bytemuck::bytes_of(entry))
                .await
                .map_err(Error::from)?;
        }

        // Write backup header at last LBA
        self.backup_header
            .write_to_lba(writer, self.backup_header.my_lba.to_ne(), self.block_size)
            .await?;

        Ok(())
    }
}

// I/O operations for PartitionTable

/// Extension trait for reading [`PartitionTable`] from I/O sources.
#[cfg(all(feature = "alloc", feature = "read"))]
#[cfg_attr(docsrs, doc(cfg(all(feature = "alloc", feature = "read"))))]
pub trait PartitionTableReadExt: Sized {
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
    async fn read_from<R: Read + Seek>(
        reader: &mut R,
        block_size: u32,
    ) -> Result<Self>;
}

#[cfg(all(feature = "alloc", feature = "read"))]
impl PartitionTableReadExt for PartitionTable {
    async fn read_from<R: Read + Seek>(
        reader: &mut R,
        block_size: u32,
    ) -> Result<Self> {
        // Seek to beginning and read MBR
        reader
            .seek(SeekFrom::Start(0))
            .await
            .map_err(Error::from)?;

        let mbr = MasterBootRecord::read_from(reader).await?;
        let scheme_type = detect_scheme_from_mbr(&mbr);

        match scheme_type {
            PartitionSchemeType::Mbr => Ok(Self::Mbr(mbr)),
            PartitionSchemeType::Gpt => {
                let gpt = GptDisk::read_from(reader, block_size).await?;
                Ok(Self::Gpt {
                    protective_mbr: mbr,
                    gpt,
                })
            }
            PartitionSchemeType::Hybrid => {
                let gpt = GptDisk::read_from(reader, block_size).await?;
                Ok(Self::Hybrid {
                    hybrid_mbr: mbr,
                    gpt,
                })
            }
        }
    }
}

/// Extension trait for writing [`PartitionTable`] to I/O sinks.
#[cfg(all(feature = "alloc", feature = "write"))]
#[cfg_attr(docsrs, doc(cfg(all(feature = "alloc", feature = "write"))))]
pub trait PartitionTableWriteExt {
    /// Writes the partition scheme to a writer.
    ///
    /// # Arguments
    ///
    /// * `writer` - The writer to write to
    ///
    /// # Errors
    ///
    /// Returns an error if writing fails.
    async fn write_to<W: Write + Seek>(&self, writer: &mut W) -> Result<()>;
}

#[cfg(all(feature = "alloc", feature = "write"))]
impl PartitionTableWriteExt for PartitionTable {
    async fn write_to<W: Write + Seek>(&self, writer: &mut W) -> Result<()> {
        match self {
            Self::Mbr(mbr) => {
                writer
                    .seek(SeekFrom::Start(0))
                    .await
                    .map_err(Error::from)?;
                mbr.write_to(writer).await
            }
            Self::Gpt { gpt, .. } => gpt.write_to(writer).await,
            Self::Hybrid { hybrid_mbr, gpt } => gpt.write_to_with_mbr(writer, hybrid_mbr).await,
        }
    }
}

} // io_transform!
