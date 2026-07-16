//! Lightweight block-format detection.
//!
//! Detection examines boot metadata and restores the stream's original
//! position. It does not validate an entire filesystem or partition table;
//! callers should open the corresponding concrete crate to perform full
//! validation.

/// A recognized block-storage layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BlockFormat {
    /// A FAT filesystem occupying the probed device or bounded partition.
    Fat(FatVariant),
    /// A disk partition table.
    PartitionTable(PartitionTableKind),
}

/// FAT family identified from its BIOS parameter block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FatVariant {
    /// A FAT12 filesystem.
    Fat12,
    /// A FAT16 filesystem.
    Fat16,
    /// A FAT32 filesystem.
    Fat32,
    /// exFAT was recognized; the stable unified opener does not open it.
    ExFat,
}

/// Partition-table family identified from sector-zero and GPT metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PartitionTableKind {
    /// A legacy Master Boot Record partition table.
    Mbr,
    /// A GUID Partition Table, including its protective MBR.
    Gpt,
    /// A GPT with both protective and ordinary MBR entries.
    Hybrid,
}

/// Probe a 512-byte logical sector without performing I/O.
///
/// A protective MBR is reported as GPT based on its partition entries. The
/// stream-based detectors additionally check for the GPT header signature.
pub fn detect_sector(sector: &[u8; 512]) -> Option<BlockFormat> {
    if let Some(kind) = partition_kind(sector) {
        return Some(BlockFormat::PartitionTable(kind));
    }
    fat_variant(sector).map(BlockFormat::Fat)
}

fn partition_kind(sector: &[u8; 512]) -> Option<PartitionTableKind> {
    if sector[510..512] != [0x55, 0xaa] {
        return None;
    }

    let mut used = 0u8;
    let mut protective = false;
    let mut ordinary = false;
    for entry in sector[446..510].chunks_exact(16) {
        if !matches!(entry[0], 0x00 | 0x80) {
            return None;
        }
        let ty = entry[4];
        let sectors = u32::from_le_bytes([entry[12], entry[13], entry[14], entry[15]]);
        if ty == 0 || sectors == 0 {
            continue;
        }
        used += 1;
        protective |= ty == 0xee;
        ordinary |= ty != 0xee;
    }

    if used == 0 {
        None
    } else if protective && ordinary {
        Some(PartitionTableKind::Hybrid)
    } else if protective {
        Some(PartitionTableKind::Gpt)
    } else {
        Some(PartitionTableKind::Mbr)
    }
}

fn fat_variant(sector: &[u8; 512]) -> Option<FatVariant> {
    if sector[510..512] != [0x55, 0xaa] {
        return None;
    }
    if &sector[3..11] == b"EXFAT   " {
        return Some(FatVariant::ExFat);
    }

    let bytes_per_sector = u16::from_le_bytes([sector[11], sector[12]]) as u32;
    let sectors_per_cluster = sector[13] as u32;
    let reserved = u16::from_le_bytes([sector[14], sector[15]]) as u32;
    let fats = sector[16] as u32;
    let root_entries = u16::from_le_bytes([sector[17], sector[18]]) as u32;
    let total16 = u16::from_le_bytes([sector[19], sector[20]]) as u32;
    let total32 = u32::from_le_bytes([sector[32], sector[33], sector[34], sector[35]]);
    let fat16 = u16::from_le_bytes([sector[22], sector[23]]) as u32;
    let fat32 = u32::from_le_bytes([sector[36], sector[37], sector[38], sector[39]]);

    if !matches!(bytes_per_sector, 512 | 1024 | 2048 | 4096)
        || sectors_per_cluster == 0
        || !sectors_per_cluster.is_power_of_two()
        || reserved == 0
        || fats == 0
    {
        return None;
    }

    let total = if total16 != 0 { total16 } else { total32 };
    let fat_size = if fat16 != 0 { fat16 } else { fat32 };
    let root_sectors = (root_entries * 32).div_ceil(bytes_per_sector);
    let metadata = reserved
        .checked_add(fats.checked_mul(fat_size)?)?
        .checked_add(root_sectors)?;
    let data_sectors = total.checked_sub(metadata)?;
    let clusters = data_sectors / sectors_per_cluster;

    Some(if clusters < 4_085 {
        FatVariant::Fat12
    } else if clusters < 65_525 {
        FatVariant::Fat16
    } else {
        FatVariant::Fat32
    })
}

#[cfg(feature = "sync")]
/// Synchronous block-format detection.
pub mod sync {
    use super::{BlockFormat, PartitionTableKind, detect_sector};
    use hadris_io::sync::{Read, Seek};
    use hadris_io::{Result, SeekFrom};

    /// Detect a layout and restore the reader's original position.
    pub fn detect<R>(reader: &mut R, logical_block_size: u32) -> Result<Option<BlockFormat>>
    where
        R: Read + Seek<Error = <R as Read>::Error>,
    {
        let original = reader.stream_position().map_err(|error| error.erase())?;
        let result = detect_at_start(reader, logical_block_size);
        reader
            .seek(SeekFrom::Start(original))
            .map_err(|error| error.erase())?;
        result
    }

    fn detect_at_start<R>(reader: &mut R, logical_block_size: u32) -> Result<Option<BlockFormat>>
    where
        R: Read + Seek<Error = <R as Read>::Error>,
    {
        reader
            .seek(SeekFrom::Start(0))
            .map_err(|error| error.erase())?;
        let mut sector = [0u8; 512];
        reader.read_exact(&mut sector)?;
        let detected = detect_sector(&sector);
        if matches!(
            detected,
            Some(BlockFormat::PartitionTable(PartitionTableKind::Gpt))
        ) && logical_block_size >= 512
        {
            reader
                .seek(SeekFrom::Start(logical_block_size as u64))
                .map_err(|error| error.erase())?;
            let mut signature = [0u8; 8];
            reader.read_exact(&mut signature)?;
            if &signature != b"EFI PART" {
                return Ok(None);
            }
        }
        Ok(detected)
    }
}

#[cfg(feature = "async")]
/// Asynchronous block-format detection.
pub mod r#async {
    use super::{BlockFormat, PartitionTableKind, detect_sector};
    use hadris_io::r#async::{Read, Seek};
    use hadris_io::{Result, SeekFrom};

    /// Detect a layout asynchronously and restore the reader's original position.
    pub async fn detect<R>(reader: &mut R, logical_block_size: u32) -> Result<Option<BlockFormat>>
    where
        R: Read + Seek<Error = <R as Read>::Error>,
    {
        let original = reader
            .stream_position()
            .await
            .map_err(|error| error.erase())?;
        let result = detect_at_start(reader, logical_block_size).await;
        reader
            .seek(SeekFrom::Start(original))
            .await
            .map_err(|error| error.erase())?;
        result
    }

    async fn detect_at_start<R>(
        reader: &mut R,
        logical_block_size: u32,
    ) -> Result<Option<BlockFormat>>
    where
        R: Read + Seek<Error = <R as Read>::Error>,
    {
        reader
            .seek(SeekFrom::Start(0))
            .await
            .map_err(|error| error.erase())?;
        let mut sector = [0u8; 512];
        reader.read_exact(&mut sector).await?;
        let detected = detect_sector(&sector);
        if matches!(
            detected,
            Some(BlockFormat::PartitionTable(PartitionTableKind::Gpt))
        ) && logical_block_size >= 512
        {
            reader
                .seek(SeekFrom::Start(logical_block_size as u64))
                .await
                .map_err(|error| error.erase())?;
            let mut signature = [0u8; 8];
            reader.read_exact(&mut signature).await?;
            if &signature != b"EFI PART" {
                return Ok(None);
            }
        }
        Ok(detected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fat_sector(total: u32, fat_size: u16, sectors_per_cluster: u8) -> [u8; 512] {
        let mut sector = [0u8; 512];
        sector[0] = 0xeb;
        sector[3..11].copy_from_slice(b"HADRIS  ");
        sector[11..13].copy_from_slice(&512u16.to_le_bytes());
        sector[13] = sectors_per_cluster;
        sector[14..16].copy_from_slice(&1u16.to_le_bytes());
        sector[16] = 2;
        sector[17..19].copy_from_slice(&512u16.to_le_bytes());
        sector[19..21].copy_from_slice(&(total as u16).to_le_bytes());
        sector[22..24].copy_from_slice(&fat_size.to_le_bytes());
        sector[510..512].copy_from_slice(&[0x55, 0xaa]);
        sector
    }

    #[test]
    fn recognizes_fat_without_mistaking_boot_signature_for_mbr() {
        let sector = fat_sector(4_000, 12, 1);
        assert_eq!(
            detect_sector(&sector),
            Some(BlockFormat::Fat(FatVariant::Fat12))
        );
    }

    #[test]
    fn recognizes_mbr_and_gpt_partition_entries() {
        let mut sector = [0u8; 512];
        sector[446 + 4] = 0x83;
        sector[446 + 12..446 + 16].copy_from_slice(&100u32.to_le_bytes());
        sector[510..512].copy_from_slice(&[0x55, 0xaa]);
        assert_eq!(
            detect_sector(&sector),
            Some(BlockFormat::PartitionTable(PartitionTableKind::Mbr))
        );

        sector[446 + 4] = 0xee;
        assert_eq!(
            detect_sector(&sector),
            Some(BlockFormat::PartitionTable(PartitionTableKind::Gpt))
        );
    }

    #[cfg(feature = "sync")]
    #[test]
    fn stream_probe_validates_gpt_signature_and_restores_position() {
        use hadris_io::SeekFrom;
        use hadris_io::sync::Seek;

        let mut image = [0u8; 1024];
        image[446 + 4] = 0xee;
        image[446 + 12..446 + 16].copy_from_slice(&100u32.to_le_bytes());
        image[510..512].copy_from_slice(&[0x55, 0xaa]);
        image[512..520].copy_from_slice(b"EFI PART");

        let mut cursor = hadris_io::Cursor::new(&image);
        cursor.seek(SeekFrom::Start(17)).unwrap();
        assert_eq!(
            sync::detect(&mut cursor, 512).unwrap(),
            Some(BlockFormat::PartitionTable(PartitionTableKind::Gpt))
        );
        assert_eq!(cursor.stream_position().unwrap(), 17);

        image[512..520].fill(0);
        let mut cursor = hadris_io::Cursor::new(&image);
        assert_eq!(sync::detect(&mut cursor, 512).unwrap(), None);
    }

    #[cfg(all(feature = "std", feature = "sync", feature = "write", feature = "fat"))]
    #[test]
    fn recognizes_volume_created_by_fat_formatter() {
        use hadris_fat::format::{FatFormatOptions, FatTypeSelection, FatVolumeFormatter};

        let mut image = std::vec![0u8; 2 * 1024 * 1024];
        let options = FatFormatOptions::new(image.len() as u64).fat_type(FatTypeSelection::Fat12);
        let fs = FatVolumeFormatter::format(std::io::Cursor::new(&mut image[..]), options).unwrap();
        drop(fs);

        let mut cursor = std::io::Cursor::new(image);
        assert_eq!(
            sync::detect(&mut cursor, 512).unwrap(),
            Some(BlockFormat::Fat(FatVariant::Fat12))
        );
    }
}
