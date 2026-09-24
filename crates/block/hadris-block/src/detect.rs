//! Lightweight block-format detection.
//!
//! Detection reads the boot metadata of a block device. It does not validate
//! an entire filesystem or partition table; `OpenVolume` or the format crate
//! does that when it opens the volume.

/// A recognized block-storage layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BlockFormat {
    /// A FAT filesystem occupying the probed device or bounded partition.
    Fat(FatVariant),
    /// An NTFS filesystem occupying the probed device or bounded partition.
    Ntfs,
    /// A disk partition table.
    PartitionTable(PartitionTableKind),
}

/// FAT family identified from its BIOS parameter block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FatVariant {
    /// A FAT12 filesystem.
    Fat12,
    /// A FAT16 filesystem.
    Fat16,
    /// A FAT32 filesystem.
    Fat32,
    /// exFAT was recognized; `OpenVolume` does not open it yet.
    ExFat,
}

/// Partition-table family identified from sector-zero and GPT metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
/// device detectors additionally check for the GPT header signature. The
/// NTFS and exFAT OEM identifiers are checked first, since their boot code
/// can look like partition entries.
pub fn detect_sector(sector: &[u8; 512]) -> Option<BlockFormat> {
    if sector[510..512] == [0x55, 0xaa] {
        match &sector[3..11] {
            b"NTFS    " => return Some(BlockFormat::Ntfs),
            b"EXFAT   " => return Some(BlockFormat::Fat(FatVariant::ExFat)),
            _ => {}
        }
    }
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

/// The largest device block the detectors read.
#[cfg(any(feature = "sync", feature = "async"))]
const MAX_BLOCK: usize = 4096;

#[cfg(any(feature = "sync", feature = "async"))]
macro_rules! probe {
    ($dev:ident $(, $aw:tt)?) => {{
        let size = $dev.block_size().get() as usize;
        if size > MAX_BLOCK {
            return Ok(None);
        }
        let mut buf = [0u8; MAX_BLOCK];
        let len = 512usize.div_ceil(size) * size;
        if ((len / size) as u64) > $dev.block_count() {
            return Ok(None);
        }
        $dev.read_blocks(BlockIndex::new(0), &mut buf[..len])$(.$aw)??;
        let mut sector = [0u8; 512];
        sector.copy_from_slice(&buf[..512]);
        let detected = detect_sector(&sector);
        if matches!(
            detected,
            Some(BlockFormat::PartitionTable(PartitionTableKind::Gpt))
        ) && size >= 512
        {
            if $dev.block_count() < 2 {
                return Ok(None);
            }
            $dev.read_blocks(BlockIndex::new(1), &mut buf[..size])$(.$aw)??;
            if &buf[..8] != b"EFI PART" {
                return Ok(None);
            }
        }
        Ok(detected)
    }};
}

#[cfg(feature = "sync")]
/// Synchronous block-format detection.
pub mod sync {
    use super::{BlockFormat, MAX_BLOCK, PartitionTableKind, detect_sector};
    use hadris_storage::BlockIndex;
    use hadris_storage::sync::BlockDevice;

    /// Detects the layout of `dev` from its first 512 bytes and, for a GPT,
    /// the header signature in block 1.
    ///
    /// Devices too small to hold a boot sector, and devices whose blocks are
    /// larger than 4096 bytes, give `None`.
    pub fn detect<D: BlockDevice + ?Sized>(dev: &mut D) -> Result<Option<BlockFormat>, D::Error> {
        probe!(dev)
    }
}

#[cfg(feature = "async")]
/// Asynchronous block-format detection.
pub mod r#async {
    use super::{BlockFormat, MAX_BLOCK, PartitionTableKind, detect_sector};
    use hadris_storage::BlockIndex;
    use hadris_storage::r#async::BlockDevice;

    /// Detects the layout of `dev` from its first 512 bytes and, for a GPT,
    /// the header signature in block 1.
    ///
    /// Devices too small to hold a boot sector, and devices whose blocks are
    /// larger than 4096 bytes, give `None`.
    pub async fn detect<D: BlockDevice + ?Sized>(
        dev: &mut D,
    ) -> Result<Option<BlockFormat>, D::Error> {
        probe!(dev, await)
    }
}

#[cfg(feature = "async-send")]
/// Asynchronous block-format detection over the `Send` devices of
/// `hadris_storage::async_send`.
pub mod async_send {
    use super::{BlockFormat, MAX_BLOCK, PartitionTableKind, detect_sector};
    use hadris_storage::BlockIndex;
    use hadris_storage::async_send::BlockDevice;

    /// Detects the layout of `dev` from its first 512 bytes and, for a GPT,
    /// the header signature in block 1.
    ///
    /// Devices too small to hold a boot sector, and devices whose blocks are
    /// larger than 4096 bytes, give `None`.
    pub async fn detect<D: BlockDevice + ?Sized>(
        dev: &mut D,
    ) -> Result<Option<BlockFormat>, D::Error> {
        probe!(dev, await)
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

    #[test]
    fn recognizes_ntfs_and_exfat_before_partition_entries() {
        let mut sector = [0u8; 512];
        sector[446 + 4] = 0x07;
        sector[446 + 12..446 + 16].copy_from_slice(&100u32.to_le_bytes());
        sector[510..512].copy_from_slice(&[0x55, 0xaa]);
        sector[3..11].copy_from_slice(b"NTFS    ");
        assert_eq!(detect_sector(&sector), Some(BlockFormat::Ntfs));
        sector[3..11].copy_from_slice(b"EXFAT   ");
        assert_eq!(
            detect_sector(&sector),
            Some(BlockFormat::Fat(FatVariant::ExFat))
        );
        sector[510] = 0;
        assert_eq!(detect_sector(&sector), None);
    }

    #[cfg(feature = "sync")]
    #[test]
    fn device_probe_validates_gpt_signature() {
        use hadris_storage::{BlockSize, MemDevice};

        let mut image = [0u8; 1024];
        image[446 + 4] = 0xee;
        image[446 + 12..446 + 16].copy_from_slice(&100u32.to_le_bytes());
        image[510..512].copy_from_slice(&[0x55, 0xaa]);
        image[512..520].copy_from_slice(b"EFI PART");

        let block = BlockSize::new(512).unwrap();
        assert_eq!(
            sync::detect(&mut MemDevice::new(&image[..], block)).unwrap(),
            Some(BlockFormat::PartitionTable(PartitionTableKind::Gpt))
        );

        image[512..520].fill(0);
        assert_eq!(
            sync::detect(&mut MemDevice::new(&image[..], block)).unwrap(),
            None
        );
        assert_eq!(
            sync::detect(&mut MemDevice::new(&image[..256], block)).unwrap(),
            None
        );
    }

    #[cfg(feature = "sync")]
    #[test]
    fn device_probe_reads_a_sector_across_small_blocks() {
        use hadris_storage::{BlockSize, MemDevice};

        let sector = fat_sector(4_000, 12, 1);
        let mut dev = MemDevice::new(&sector[..], BlockSize::new(64).unwrap());
        assert_eq!(
            sync::detect(&mut dev).unwrap(),
            Some(BlockFormat::Fat(FatVariant::Fat12))
        );
    }

    #[cfg(all(feature = "std", feature = "sync", feature = "write"))]
    #[test]
    fn recognizes_volume_created_by_fat_formatter() {
        use hadris_fat::{FatKind, FormatOptions};
        use hadris_storage::{BlockSize, MemDevice};

        let dev = MemDevice::new(
            std::vec![0u8; 2 * 1024 * 1024],
            BlockSize::new(512).unwrap(),
        );
        let fs =
            hadris_fat::sync::format(dev, FormatOptions::new().with_kind(FatKind::Fat12)).unwrap();
        let mut dev = fs.into_inner();
        assert_eq!(
            sync::detect(&mut dev).unwrap(),
            Some(BlockFormat::Fat(FatVariant::Fat12))
        );
    }
}
