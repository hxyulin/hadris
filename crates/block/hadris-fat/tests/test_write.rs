//! Tests for write operations.
//!
//! These tests require the "write" feature.

#![cfg(feature = "write")]

use hadris_fat::file::ShortFileName;

#[test]
fn test_short_filename_from_long_name_simple() {
    let sfn = ShortFileName::from_long_name("test.txt", 0).unwrap();
    assert_eq!(sfn.as_str(), "TEST    .TXT");
}

#[test]
fn test_short_filename_from_long_name_with_suffix() {
    let sfn = ShortFileName::from_long_name("test.txt", 1).unwrap();
    assert!(sfn.as_str().contains("~1"));
}

#[test]
fn test_short_filename_from_long_name_no_extension() {
    let sfn = ShortFileName::from_long_name("README", 0).unwrap();
    assert!(sfn.as_str().starts_with("README"));
}

#[test]
fn test_short_filename_from_long_name_long_base() {
    let sfn = ShortFileName::from_long_name("verylongfilename.txt", 1).unwrap();
    // Should truncate to 6 chars + ~1
    assert!(sfn.as_str().starts_with("VERYLO~1"));
}

#[test]
fn test_short_filename_from_long_name_lowercase() {
    let sfn = ShortFileName::from_long_name("lowercase.dat", 0).unwrap();
    // Should be converted to uppercase
    assert_eq!(sfn.as_str(), "LOWERCAS.DAT");
}

#[test]
fn test_short_filename_from_long_name_special_chars() {
    let sfn = ShortFileName::from_long_name("my file.txt", 0).unwrap();
    // Spaces should be stripped
    assert!(!sfn.as_str().contains(' ') || sfn.as_str().chars().filter(|c| *c == ' ').count() <= 3);
}

#[cfg(feature = "std")]
mod datetime_tests {
    use hadris_fat::FatDateTime;

    #[test]
    fn test_fat_datetime_now() {
        let dt = FatDateTime::now();
        // Date should be non-zero (after 1980)
        assert!(dt.date > 0);
    }

    #[test]
    fn test_fat_datetime_new() {
        let dt = FatDateTime::new(2024, 6, 15, 14, 30, 45);
        let (date, time, _) = dt.to_raw();

        // Decode date: (year-1980)<<9 | month<<5 | day
        let year = ((date >> 9) & 0x7F) + 1980;
        let month = (date >> 5) & 0x0F;
        let day = date & 0x1F;

        assert_eq!(year, 2024);
        assert_eq!(month, 6);
        assert_eq!(day, 15);

        // Decode time: hour<<11 | minute<<5 | (second/2)
        let hour = (time >> 11) & 0x1F;
        let minute = (time >> 5) & 0x3F;
        let second = (time & 0x1F) * 2;

        assert_eq!(hour, 14);
        assert_eq!(minute, 30);
        // Second is stored with 2-second granularity
        assert!(second == 44 || second == 46);
    }
}

/// Helper module to create FAT32 images in memory for testing
#[cfg(feature = "std")]
mod fat32_image {
    use std::io::Cursor;

    /// Sector size in bytes
    const SECTOR_SIZE: usize = 512;
    /// Sectors per cluster
    const SECTORS_PER_CLUSTER: u8 = 1;
    /// Cluster size in bytes
    const CLUSTER_SIZE: usize = SECTOR_SIZE * SECTORS_PER_CLUSTER as usize;
    /// Reserved sectors (including boot and FSInfo)
    const RESERVED_SECTORS: u16 = 32;
    /// Number of FAT copies
    const FAT_COUNT: u8 = 2;
    /// Sectors per FAT (enough for our small test image)
    const SECTORS_PER_FAT: u32 = 128;
    /// Root directory cluster
    const ROOT_CLUSTER: u32 = 2;

    /// FSInfo signature constants
    const FSINFO_LEAD_SIG: u32 = 0x41615252;
    const FSINFO_STRUC_SIG: u32 = 0x61417272;
    const FSINFO_TRAIL_SIG: u32 = 0xAA550000;

    /// Create a minimal FAT32 image in memory.
    ///
    /// The image has:
    /// - 512 byte sectors
    /// - 1 sector per cluster
    /// - 32 reserved sectors
    /// - 2 FATs, each 128 sectors
    /// - Root directory at cluster 2
    /// - Total size: ~150KB
    pub fn create_fat32_image() -> Cursor<Vec<u8>> {
        // Calculate total size
        let data_start_sector = RESERVED_SECTORS as u32 + FAT_COUNT as u32 * SECTORS_PER_FAT;
        let total_data_clusters: u32 = 256; // Enough for testing
        let total_sectors = data_start_sector + total_data_clusters;
        let total_size = total_sectors as usize * SECTOR_SIZE;

        let mut image = vec![0u8; total_size];

        // Write boot sector (sector 0)
        write_boot_sector(&mut image);

        // Write FSInfo sector (sector 1)
        write_fsinfo_sector(&mut image, total_data_clusters - 1);

        // Write FAT tables
        let fat_start = RESERVED_SECTORS as usize * SECTOR_SIZE;
        write_fat_table(&mut image, fat_start);
        // Second FAT copy
        let fat2_start = fat_start + SECTORS_PER_FAT as usize * SECTOR_SIZE;
        write_fat_table(&mut image, fat2_start);

        // Initialize root directory cluster (cluster 2) - already zeroed

        Cursor::new(image)
    }

    fn write_boot_sector(image: &mut [u8]) {
        // Jump instruction
        image[0] = 0xEB;
        image[1] = 0x58;
        image[2] = 0x90;

        // OEM name
        image[3..11].copy_from_slice(b"HADRISFT");

        // BPB_BytsPerSec
        image[11..13].copy_from_slice(&(SECTOR_SIZE as u16).to_le_bytes());

        // BPB_SecPerClus
        image[13] = SECTORS_PER_CLUSTER;

        // BPB_RsvdSecCnt
        image[14..16].copy_from_slice(&RESERVED_SECTORS.to_le_bytes());

        // BPB_NumFATs
        image[16] = FAT_COUNT;

        // BPB_RootEntCnt (must be 0 for FAT32)
        image[17..19].copy_from_slice(&0u16.to_le_bytes());

        // BPB_TotSec16 (must be 0 for FAT32)
        image[19..21].copy_from_slice(&0u16.to_le_bytes());

        // BPB_Media (0xF8 = fixed disk)
        image[21] = 0xF8;

        // BPB_FATSz16 (must be 0 for FAT32)
        image[22..24].copy_from_slice(&0u16.to_le_bytes());

        // BPB_SecPerTrk
        image[24..26].copy_from_slice(&63u16.to_le_bytes());

        // BPB_NumHeads
        image[26..28].copy_from_slice(&255u16.to_le_bytes());

        // BPB_HiddSec
        image[28..32].copy_from_slice(&0u32.to_le_bytes());

        // BPB_TotSec32
        let total_sectors = RESERVED_SECTORS as u32 + FAT_COUNT as u32 * SECTORS_PER_FAT + 256;
        image[32..36].copy_from_slice(&total_sectors.to_le_bytes());

        // FAT32 extended fields (offset 36)
        // BPB_FATSz32
        image[36..40].copy_from_slice(&SECTORS_PER_FAT.to_le_bytes());

        // BPB_ExtFlags (mirror FATs)
        image[40..42].copy_from_slice(&0u16.to_le_bytes());

        // BPB_FSVer
        image[42..44].copy_from_slice(&0u16.to_le_bytes());

        // BPB_RootClus
        image[44..48].copy_from_slice(&ROOT_CLUSTER.to_le_bytes());

        // BPB_FSInfo (sector 1)
        image[48..50].copy_from_slice(&1u16.to_le_bytes());

        // BPB_BkBootSec (sector 6)
        image[50..52].copy_from_slice(&6u16.to_le_bytes());

        // Reserved (52-63)
        // Already zeroed

        // BS_DrvNum
        image[64] = 0x80;

        // BS_Reserved1
        image[65] = 0;

        // BS_BootSig
        image[66] = 0x29;

        // BS_VolID
        image[67..71].copy_from_slice(&0x12345678u32.to_le_bytes());

        // BS_VolLab
        image[71..82].copy_from_slice(b"TEST       ");

        // BS_FilSysType
        image[82..90].copy_from_slice(b"FAT32   ");

        // Boot signature at 510-511
        image[510] = 0x55;
        image[511] = 0xAA;
    }

    fn write_fsinfo_sector(image: &mut [u8], free_clusters: u32) {
        let offset = SECTOR_SIZE; // Sector 1

        // FSI_LeadSig
        image[offset..offset + 4].copy_from_slice(&FSINFO_LEAD_SIG.to_le_bytes());

        // Reserved (4-483) - already zeroed

        // FSI_StrucSig (offset 484)
        image[offset + 484..offset + 488].copy_from_slice(&FSINFO_STRUC_SIG.to_le_bytes());

        // FSI_Free_Count
        image[offset + 488..offset + 492].copy_from_slice(&free_clusters.to_le_bytes());

        // FSI_Nxt_Free (start search at cluster 3, since 2 is root)
        image[offset + 492..offset + 496].copy_from_slice(&3u32.to_le_bytes());

        // Reserved (496-507) - already zeroed

        // FSI_TrailSig
        image[offset + 508..offset + 512].copy_from_slice(&FSINFO_TRAIL_SIG.to_le_bytes());
    }

    fn write_fat_table(image: &mut [u8], fat_start: usize) {
        // Entry 0: Media type in low byte, 0xFF in rest
        image[fat_start..fat_start + 4].copy_from_slice(&0x0FFFFFF8u32.to_le_bytes());

        // Entry 1: End of chain marker (clean shutdown)
        image[fat_start + 4..fat_start + 8].copy_from_slice(&0x0FFFFFFFu32.to_le_bytes());

        // Entry 2: End of chain for root directory (single cluster)
        image[fat_start + 8..fat_start + 12].copy_from_slice(&0x0FFFFFF8u32.to_le_bytes());

        // Rest of FAT entries are 0 (free clusters)
    }

    /// Get cluster size for the test image
    pub const fn cluster_size() -> usize {
        CLUSTER_SIZE
    }

    /// Byte offset of FAT[0] (primary FAT) in the image.
    pub const fn fat_start_bytes() -> usize {
        RESERVED_SECTORS as usize * SECTOR_SIZE
    }

    /// Byte offset of FAT[1] (backup FAT) in the image.
    pub const fn fat2_start_bytes() -> usize {
        fat_start_bytes() + SECTORS_PER_FAT as usize * SECTOR_SIZE
    }

    /// Byte offset of the data area (cluster 2) in the image.
    pub const fn data_start_bytes() -> usize {
        fat2_start_bytes() + SECTORS_PER_FAT as usize * SECTOR_SIZE
    }
}

/// Helper module to create FAT16 images in memory for testing
#[cfg(feature = "std")]
mod fat16_image {
    use std::io::Cursor;

    /// Sector size in bytes
    const SECTOR_SIZE: usize = 512;
    /// Sectors per cluster
    const SECTORS_PER_CLUSTER: u8 = 1;
    /// Cluster size in bytes
    const CLUSTER_SIZE: usize = SECTOR_SIZE * SECTORS_PER_CLUSTER as usize;
    /// Reserved sectors (just boot sector for FAT16)
    const RESERVED_SECTORS: u16 = 1;
    /// Number of FAT copies
    const FAT_COUNT: u8 = 2;
    /// Sectors per FAT (enough for our small test image)
    const SECTORS_PER_FAT: u16 = 32;
    /// Root directory entry count
    const ROOT_ENTRY_COUNT: u16 = 512;

    /// Create a minimal FAT16 image in memory.
    ///
    /// The image has:
    /// - 512 byte sectors
    /// - 1 sector per cluster
    /// - 1 reserved sector
    /// - 2 FATs, each 32 sectors
    /// - 512 root directory entries (32 sectors)
    /// - Total size: ~2MB (enough clusters to be detected as FAT16)
    pub fn create_fat16_image() -> Cursor<Vec<u8>> {
        // Calculate layout
        let root_dir_sectors = (ROOT_ENTRY_COUNT as usize * 32).div_ceil(SECTOR_SIZE);
        let data_start_sector = RESERVED_SECTORS as usize
            + FAT_COUNT as usize * SECTORS_PER_FAT as usize
            + root_dir_sectors;

        // Need enough clusters to be FAT16 (>= 4085 clusters)
        let total_data_clusters: usize = 8192; // Well into FAT16 range
        let total_sectors = data_start_sector + total_data_clusters;
        let total_size = total_sectors * SECTOR_SIZE;

        let mut image = vec![0u8; total_size];

        // Write boot sector (sector 0)
        write_boot_sector(&mut image, total_sectors as u32);

        // Write FAT tables
        let fat_start = RESERVED_SECTORS as usize * SECTOR_SIZE;
        write_fat_table(&mut image, fat_start);
        // Second FAT copy
        let fat2_start = fat_start + SECTORS_PER_FAT as usize * SECTOR_SIZE;
        write_fat_table(&mut image, fat2_start);

        // Root directory is initially empty (zeroed)

        Cursor::new(image)
    }

    fn write_boot_sector(image: &mut [u8], total_sectors: u32) {
        // Jump instruction
        image[0] = 0xEB;
        image[1] = 0x3C;
        image[2] = 0x90;

        // OEM name
        image[3..11].copy_from_slice(b"HADRISFT");

        // BPB_BytsPerSec
        image[11..13].copy_from_slice(&(SECTOR_SIZE as u16).to_le_bytes());

        // BPB_SecPerClus
        image[13] = SECTORS_PER_CLUSTER;

        // BPB_RsvdSecCnt
        image[14..16].copy_from_slice(&RESERVED_SECTORS.to_le_bytes());

        // BPB_NumFATs
        image[16] = FAT_COUNT;

        // BPB_RootEntCnt (non-zero for FAT16)
        image[17..19].copy_from_slice(&ROOT_ENTRY_COUNT.to_le_bytes());

        // BPB_TotSec16 (0 if > 65535)
        if total_sectors <= 65535 {
            image[19..21].copy_from_slice(&(total_sectors as u16).to_le_bytes());
        } else {
            image[19..21].copy_from_slice(&0u16.to_le_bytes());
        }

        // BPB_Media (0xF8 = fixed disk)
        image[21] = 0xF8;

        // BPB_FATSz16 (non-zero for FAT16)
        image[22..24].copy_from_slice(&SECTORS_PER_FAT.to_le_bytes());

        // BPB_SecPerTrk
        image[24..26].copy_from_slice(&63u16.to_le_bytes());

        // BPB_NumHeads
        image[26..28].copy_from_slice(&255u16.to_le_bytes());

        // BPB_HiddSec
        image[28..32].copy_from_slice(&0u32.to_le_bytes());

        // BPB_TotSec32
        if total_sectors > 65535 {
            image[32..36].copy_from_slice(&total_sectors.to_le_bytes());
        } else {
            image[32..36].copy_from_slice(&0u32.to_le_bytes());
        }

        // FAT16 extended fields (offset 36)
        // BS_DrvNum
        image[36] = 0x80;

        // BS_Reserved1
        image[37] = 0;

        // BS_BootSig
        image[38] = 0x29;

        // BS_VolID
        image[39..43].copy_from_slice(&0x12345678u32.to_le_bytes());

        // BS_VolLab
        image[43..54].copy_from_slice(b"TEST       ");

        // BS_FilSysType
        image[54..62].copy_from_slice(b"FAT16   ");

        // Boot signature at 510-511
        image[510] = 0x55;
        image[511] = 0xAA;
    }

    fn write_fat_table(image: &mut [u8], fat_start: usize) {
        // Entry 0: Media type in low byte
        image[fat_start..fat_start + 2].copy_from_slice(&0xFFF8u16.to_le_bytes());

        // Entry 1: End of chain marker (clean shutdown)
        image[fat_start + 2..fat_start + 4].copy_from_slice(&0xFFFFu16.to_le_bytes());

        // Rest of FAT entries are 0 (free clusters)
    }

    /// Get cluster size for the test image
    pub const fn cluster_size() -> usize {
        CLUSTER_SIZE
    }
}

/// Helper module to create FAT12 images in memory for testing
#[cfg(feature = "std")]
mod fat12_image {
    use std::io::Cursor;

    /// Sector size in bytes
    const SECTOR_SIZE: usize = 512;
    /// Sectors per cluster
    const SECTORS_PER_CLUSTER: u8 = 1;
    /// Cluster size in bytes
    const CLUSTER_SIZE: usize = SECTOR_SIZE * SECTORS_PER_CLUSTER as usize;
    /// Reserved sectors (just boot sector for FAT12)
    const RESERVED_SECTORS: u16 = 1;
    /// Number of FAT copies
    const FAT_COUNT: u8 = 2;
    /// Sectors per FAT
    const SECTORS_PER_FAT: u16 = 9;
    /// Root directory entry count (224 is typical for 1.44MB floppy)
    const ROOT_ENTRY_COUNT: u16 = 224;

    /// Create a minimal FAT12 image in memory (1.44MB floppy-like).
    ///
    /// The image has:
    /// - 512 byte sectors
    /// - 1 sector per cluster
    /// - 1 reserved sector
    /// - 2 FATs, each 9 sectors
    /// - 224 root directory entries (14 sectors)
    /// - ~2880 total sectors (1.44MB)
    /// - Cluster count < 4085 (FAT12 range)
    pub fn create_fat12_image() -> Cursor<Vec<u8>> {
        // Calculate layout
        let root_dir_sectors = (ROOT_ENTRY_COUNT as usize * 32).div_ceil(SECTOR_SIZE);
        let _data_start_sector = RESERVED_SECTORS as usize
            + FAT_COUNT as usize * SECTORS_PER_FAT as usize
            + root_dir_sectors;

        // Use 2880 sectors total (1.44MB floppy size) - results in ~2847 clusters (FAT12)
        let total_sectors: usize = 2880;
        let total_size = total_sectors * SECTOR_SIZE;

        let mut image = vec![0u8; total_size];

        // Write boot sector (sector 0)
        write_boot_sector(&mut image, total_sectors as u16);

        // Write FAT tables
        let fat_start = RESERVED_SECTORS as usize * SECTOR_SIZE;
        write_fat_table(&mut image, fat_start);
        // Second FAT copy
        let fat2_start = fat_start + SECTORS_PER_FAT as usize * SECTOR_SIZE;
        write_fat_table(&mut image, fat2_start);

        // Root directory is initially empty (zeroed)

        Cursor::new(image)
    }

    fn write_boot_sector(image: &mut [u8], total_sectors: u16) {
        // Jump instruction
        image[0] = 0xEB;
        image[1] = 0x3C;
        image[2] = 0x90;

        // OEM name
        image[3..11].copy_from_slice(b"HADRISFT");

        // BPB_BytsPerSec
        image[11..13].copy_from_slice(&(SECTOR_SIZE as u16).to_le_bytes());

        // BPB_SecPerClus
        image[13] = SECTORS_PER_CLUSTER;

        // BPB_RsvdSecCnt
        image[14..16].copy_from_slice(&RESERVED_SECTORS.to_le_bytes());

        // BPB_NumFATs
        image[16] = FAT_COUNT;

        // BPB_RootEntCnt (non-zero for FAT12)
        image[17..19].copy_from_slice(&ROOT_ENTRY_COUNT.to_le_bytes());

        // BPB_TotSec16
        image[19..21].copy_from_slice(&total_sectors.to_le_bytes());

        // BPB_Media (0xF0 = removable media for floppy)
        image[21] = 0xF0;

        // BPB_FATSz16 (non-zero for FAT12)
        image[22..24].copy_from_slice(&SECTORS_PER_FAT.to_le_bytes());

        // BPB_SecPerTrk (18 for 1.44MB floppy)
        image[24..26].copy_from_slice(&18u16.to_le_bytes());

        // BPB_NumHeads (2 for floppy)
        image[26..28].copy_from_slice(&2u16.to_le_bytes());

        // BPB_HiddSec
        image[28..32].copy_from_slice(&0u32.to_le_bytes());

        // BPB_TotSec32 (0 for small volumes)
        image[32..36].copy_from_slice(&0u32.to_le_bytes());

        // FAT12/16 extended fields (offset 36)
        // BS_DrvNum
        image[36] = 0x00; // Floppy

        // BS_Reserved1
        image[37] = 0;

        // BS_BootSig
        image[38] = 0x29;

        // BS_VolID
        image[39..43].copy_from_slice(&0x12345678u32.to_le_bytes());

        // BS_VolLab
        image[43..54].copy_from_slice(b"TEST       ");

        // BS_FilSysType
        image[54..62].copy_from_slice(b"FAT12   ");

        // Boot signature at 510-511
        image[510] = 0x55;
        image[511] = 0xAA;
    }

    fn write_fat_table(image: &mut [u8], fat_start: usize) {
        // FAT12 packs 2 entries into 3 bytes
        // Entry 0 and 1: media type and end of chain marker

        // Entries 0, 1: 0xFF0, 0xFFF (packed as: F0 FF FF)
        image[fat_start] = 0xF0; // Low 8 bits of entry 0
        image[fat_start + 1] = 0xFF; // High 4 bits of entry 0 (0xF) + Low 4 bits of entry 1 (0xF)
        image[fat_start + 2] = 0xFF; // High 8 bits of entry 1

        // Rest of FAT entries are 0 (free clusters)
    }

    /// Get cluster size for the test image
    pub const fn cluster_size() -> usize {
        CLUSTER_SIZE
    }
}

/// Integration tests using in-memory FAT32 images
#[cfg(feature = "std")]
mod integration_tests {
    use super::fat32_image::{cluster_size, create_fat32_image};
    use hadris_fat::time::StaticTimeProvider;
    use hadris_fat::{FatDateTime, FatError, FatFs, FatFsWriteExt};

    /// Builder-driven open should accept a custom `TimeProvider`, and the
    /// timestamps it returns should land verbatim in newly-created entries.
    /// Catches regressions in the FatFs → FileWriter → directory-entry path
    /// without needing to read raw bytes.
    #[test]
    fn test_builder_static_time_provider_propagates_to_created_entries() {
        // Pin to a specific date so the assertion is deterministic regardless
        // of when this test runs.
        let pinned = FatDateTime::new(2030, 6, 15, 12, 0, 0);
        // The builder takes `&'static dyn TimeProvider`, so leak the provider
        // for the test's lifetime — same pattern users follow on embedded.
        let provider: &'static StaticTimeProvider = Box::leak(Box::new(StaticTimeProvider(pinned)));

        let image = create_fat32_image();
        let fs = FatFs::builder(image)
            .time_provider(provider)
            .open()
            .expect("builder open should succeed");

        let root = fs.root_dir();
        let entry = fs
            .create_file(&root, "STAMP.TXT")
            .expect("create_file should succeed");

        // Timestamps stamped during create_file should match the static clock.
        assert_eq!(entry.created().date, pinned.date);
        assert_eq!(entry.created().time, pinned.time);
        assert_eq!(entry.modified().date, pinned.date);
        assert_eq!(entry.accessed_date(), pinned.date);
    }

    /// `set_times` should patch each timestamp independently, and `None`
    /// fields should preserve the on-disk value.
    #[test]
    fn test_set_times_patches_only_supplied_fields() {
        let initial = FatDateTime::new(2030, 1, 1, 0, 0, 0);
        let provider: &'static StaticTimeProvider =
            Box::leak(Box::new(StaticTimeProvider(initial)));
        let image = create_fat32_image();
        let fs = FatFs::builder(image)
            .time_provider(provider)
            .open()
            .expect("builder open should succeed");

        let root = fs.root_dir();
        let _ = fs
            .create_file(&root, "PATCH.TXT")
            .expect("create_file should succeed");

        // Patch only the modified time. Creation and access should stay put.
        let new_modified = FatDateTime::new(2031, 12, 31, 23, 59, 58);
        let root = fs.root_dir();
        let entry = root
            .find("PATCH.TXT")
            .expect("find should succeed")
            .expect("entry must exist after create");
        fs.set_times(&entry, Some(new_modified), None, None)
            .expect("set_times should succeed");

        // Re-read the entry from disk and verify.
        let root = fs.root_dir();
        let after = root
            .find("PATCH.TXT")
            .expect("find should succeed")
            .expect("entry must still exist");
        assert_eq!(after.modified().date, new_modified.date);
        assert_eq!(after.modified().time, new_modified.time);
        // Created should be unchanged from initial.
        assert_eq!(after.created().date, initial.date);
        // Accessed should also be unchanged.
        assert_eq!(after.accessed_date(), initial.date);
    }

    /// Smoke test for `read_status_flags` on a freshly formatted volume.
    /// The fixture initializes FAT[1] with the high bits set, so a clean
    /// volume must report `dirty: false, io_errors: false`.
    #[test]
    fn test_read_status_flags_on_clean_volume() {
        let image = create_fat32_image();
        let fs = FatFs::open(image).expect("open");
        let flags = fs.read_status_flags().expect("read_status_flags");
        // Without explicit dirty marking, the volume is clean.
        assert!(!flags.dirty, "freshly created volume must not be dirty");
        assert!(
            !flags.io_errors,
            "freshly created volume must report no I/O errors"
        );
    }

    /// Cross-directory rename: move a file from /SRC/A.TXT to /DST/B.TXT
    /// and verify the data follows. Regression-tests that the cluster chain
    /// is preserved (no data copy) and the original entry is reclaimed.
    #[test]
    fn test_rename_across_directories_preserves_cluster_chain() {
        let image = create_fat32_image();
        let fs = FatFs::open(image).expect("open");
        let root = fs.root_dir();

        // Build /SRC/ and /DST/ subdirs.
        let _ = fs.create_dir(&root, "SRC").expect("mkdir SRC");
        let _ = fs.create_dir(&root, "DST").expect("mkdir DST");

        // Re-open dirs after creation.
        let src = fs.open_dir_path("/SRC").expect("open SRC");
        let _ = fs.create_file(&src, "A.TXT").expect("create A.TXT");

        // Find the source entry and capture its first cluster.
        let a_entry = src
            .find("A.TXT")
            .expect("find ok")
            .expect("A.TXT must exist");
        let original_cluster = a_entry.cluster();

        // Rename /SRC/A.TXT to /DST/B.TXT.
        let dst = fs.open_dir_path("/DST").expect("open DST");
        let new_entry = fs
            .rename(&a_entry, &dst, "B.TXT")
            .expect("cross-dir rename should succeed");

        // Source must no longer have A.TXT; destination must have B.TXT.
        let src = fs.open_dir_path("/SRC").expect("re-open SRC");
        assert!(
            src.find("A.TXT").expect("find ok").is_none(),
            "A.TXT must be gone from SRC after rename"
        );
        let dst = fs.open_dir_path("/DST").expect("re-open DST");
        let found = dst
            .find("B.TXT")
            .expect("find ok")
            .expect("B.TXT must exist in DST");

        // The cluster chain MUST be the same — rename moves metadata only.
        assert_eq!(
            new_entry.cluster(),
            original_cluster,
            "rename returned entry should keep the original cluster"
        );
        assert_eq!(
            found.cluster(),
            original_cluster,
            "on-disk B.TXT should keep the original cluster"
        );
    }

    /// Mutating user-visible bits on an existing entry should succeed; trying
    /// to flip the immutable kind bits (`DIRECTORY` / `VOLUME_ID`) must error
    /// rather than silently corrupting the entry kind.
    #[test]
    fn test_set_attributes_blocks_immutable_bit_flips() {
        use hadris_fat::raw::DirEntryAttrFlags;

        let image = create_fat32_image();
        let fs = FatFs::open(image).expect("open");
        let root = fs.root_dir();
        let _ = fs
            .create_file(&root, "PROT.TXT")
            .expect("create_file should succeed");

        // Re-find so we have a fresh FileEntry from disk.
        let root = fs.root_dir();
        let entry = root
            .find("PROT.TXT")
            .expect("find ok")
            .expect("entry must exist");

        // Allowed: toggling READ_ONLY should succeed.
        let with_ro = entry.attributes() | DirEntryAttrFlags::READ_ONLY;
        fs.set_attributes(&entry, with_ro)
            .expect("toggling READ_ONLY should be allowed");

        // Forbidden: trying to flip DIRECTORY on a regular file.
        let bad = entry.attributes() | DirEntryAttrFlags::DIRECTORY;
        match fs.set_attributes(&entry, bad) {
            Err(FatError::InvalidAttributeChange { bit: "DIRECTORY" }) => {}
            other => panic!("expected DIRECTORY rejection, got {other:?}"),
        }

        // Forbidden: trying to set VOLUME_ID on a regular file.
        let bad = entry.attributes() | DirEntryAttrFlags::VOLUME_ID;
        match fs.set_attributes(&entry, bad) {
            Err(FatError::InvalidAttributeChange { bit: "VOLUME_ID" }) => {}
            other => panic!("expected VOLUME_ID rejection, got {other:?}"),
        }
    }

    #[test]
    fn test_open_fat32_image() {
        let image = create_fat32_image();
        let fs = FatFs::open(image).expect("Failed to open FAT32 image");

        // Should be able to get root directory
        let root = fs.root_dir();
        let entries: Vec<_> = root.entries().collect();
        // Root directory should be empty initially
        assert!(entries.is_empty(), "Root directory should be empty");
    }

    #[test]
    fn test_create_file() {
        let image = create_fat32_image();
        let fs = FatFs::open(image).expect("Failed to open FAT32 image");

        let root = fs.root_dir();

        // Create a new file
        let entry = fs
            .create_file(&root, "TEST.TXT")
            .expect("Failed to create file");

        assert!(entry.is_file());
        assert_eq!(entry.len(), 0);

        // Verify file appears in directory listing
        let root = fs.root_dir();
        let found = root.find("TEST.TXT").expect("Find failed");
        assert!(found.is_some(), "File should be found in directory");
    }

    #[test]
    fn test_create_file_already_exists() {
        let image = create_fat32_image();
        let fs = FatFs::open(image).expect("Failed to open FAT32 image");

        let root = fs.root_dir();

        // Create a file
        fs.create_file(&root, "TEST.TXT")
            .expect("Failed to create file");

        // Try to create again - should fail
        let result = fs.create_file(&root, "TEST.TXT");
        match result {
            Err(FatError::AlreadyExists) => {}
            _ => panic!("Expected AlreadyExists error"),
        }
    }

    #[test]
    fn test_write_file_content() {
        let image = create_fat32_image();
        let fs = FatFs::open(image).expect("Failed to open FAT32 image");

        let root = fs.root_dir();

        // Create a file
        let entry = fs
            .create_file(&root, "HELLO.TXT")
            .expect("Failed to create file");

        // Write content
        let content = b"Hello, FAT32 World!";
        {
            let mut writer = fs.write_file(&entry).expect("Failed to get writer");
            writer.write(content).expect("Failed to write");
            writer.finish().expect("Failed to finish");
        }

        // Re-find the file to get updated size
        let root = fs.root_dir();
        let entry = root
            .find("HELLO.TXT")
            .expect("Find failed")
            .expect("File not found");
        assert_eq!(entry.len(), content.len() as u64);

        // Read back the content
        use hadris_fat::FatFsReadExt;
        let mut reader = fs.read_file(&entry).expect("Failed to get reader");
        let mut buf = vec![0u8; content.len()];
        let mut total = 0;
        while total < buf.len() {
            let n = reader.read(&mut buf[total..]).expect("Read failed");
            if n == 0 {
                break;
            }
            total += n;
        }
        assert_eq!(&buf[..total], content);
    }

    #[test]
    fn test_write_file_multiple_clusters() {
        let image = create_fat32_image();
        let fs = FatFs::open(image).expect("Failed to open FAT32 image");

        let root = fs.root_dir();

        // Create a file
        let entry = fs
            .create_file(&root, "BIG.DAT")
            .expect("Failed to create file");

        // Write content larger than one cluster
        let cluster_sz = cluster_size();
        let content: Vec<u8> = (0..cluster_sz * 3).map(|i| (i % 256) as u8).collect();

        {
            let mut writer = fs.write_file(&entry).expect("Failed to get writer");
            writer.write(&content).expect("Failed to write");
            writer.finish().expect("Failed to finish");
        }

        // Read back and verify
        let root = fs.root_dir();
        let entry = root
            .find("BIG.DAT")
            .expect("Find failed")
            .expect("File not found");
        assert_eq!(entry.len(), content.len() as u64);

        use hadris_fat::FatFsReadExt;
        let mut reader = fs.read_file(&entry).expect("Failed to get reader");
        let read_content = reader.read_to_vec().expect("Read failed");
        assert_eq!(read_content, content);
    }

    #[test]
    fn test_create_directory() {
        let image = create_fat32_image();
        let fs = FatFs::open(image).expect("Failed to open FAT32 image");

        let root = fs.root_dir();

        // Create a directory
        let subdir = fs
            .create_dir(&root, "SUBDIR")
            .expect("Failed to create directory");

        // Verify . and .. entries exist
        let entries: Vec<_> = subdir.entries().collect();
        let names: Vec<_> = entries
            .iter()
            .filter_map(|e| e.as_ref().ok())
            .map(|e| e.name().trim_end_matches(' ').to_string())
            .collect();

        assert!(
            names.iter().any(|n| n.starts_with('.')),
            "Directory should have . entry: {names:?}"
        );
        assert!(
            names.iter().any(|n| n.starts_with("..")),
            "Directory should have .. entry: {names:?}"
        );
    }

    #[test]
    fn test_create_file_in_subdirectory() {
        let image = create_fat32_image();
        let fs = FatFs::open(image).expect("Failed to open FAT32 image");

        let root = fs.root_dir();

        // Create a directory
        let subdir = fs
            .create_dir(&root, "MYDIR")
            .expect("Failed to create directory");

        // Create a file in the subdirectory
        let entry = fs
            .create_file(&subdir, "FILE.TXT")
            .expect("Failed to create file");
        assert!(entry.is_file());

        // Verify file exists in subdirectory
        let found = subdir.find("FILE.TXT").expect("Find failed");
        assert!(found.is_some());

        // Verify file does NOT exist in root
        let root = fs.root_dir();
        let found_in_root = root.find("FILE.TXT").expect("Find failed");
        assert!(found_in_root.is_none());
    }

    #[test]
    fn test_delete_file() {
        let image = create_fat32_image();
        let fs = FatFs::open(image).expect("Failed to open FAT32 image");

        let root = fs.root_dir();

        // Create and write a file
        let entry = fs
            .create_file(&root, "DELETE.ME")
            .expect("Failed to create file");
        {
            let mut writer = fs.write_file(&entry).expect("Failed to get writer");
            writer.write(b"temporary data").expect("Failed to write");
            writer.finish().expect("Failed to finish");
        }

        // Re-find and delete
        let root = fs.root_dir();
        let entry = root
            .find("DELETE.ME")
            .expect("Find failed")
            .expect("File not found");
        fs.delete(&entry).expect("Failed to delete");

        // Verify file no longer exists
        let root = fs.root_dir();
        let found = root.find("DELETE.ME").expect("Find failed");
        assert!(found.is_none(), "File should be deleted");
    }

    #[test]
    fn test_delete_empty_directory() {
        let image = create_fat32_image();
        let fs = FatFs::open(image).expect("Failed to open FAT32 image");

        let root = fs.root_dir();

        // Create a directory
        let _subdir = fs
            .create_dir(&root, "EMPTYDIR")
            .expect("Failed to create directory");

        // Find and delete it
        let root = fs.root_dir();
        let entry = root
            .find("EMPTYDIR")
            .expect("Find failed")
            .expect("Dir not found");
        fs.delete(&entry).expect("Failed to delete empty directory");

        // Verify directory no longer exists
        let root = fs.root_dir();
        let found = root.find("EMPTYDIR").expect("Find failed");
        assert!(found.is_none(), "Directory should be deleted");
    }

    #[test]
    fn test_delete_non_empty_directory_fails() {
        let image = create_fat32_image();
        let fs = FatFs::open(image).expect("Failed to open FAT32 image");

        let root = fs.root_dir();

        // Create a directory with a file inside
        let subdir = fs
            .create_dir(&root, "HASFILE")
            .expect("Failed to create directory");
        fs.create_file(&subdir, "INSIDE.TXT")
            .expect("Failed to create file");

        // Try to delete the directory - should fail
        let root = fs.root_dir();
        let entry = root
            .find("HASFILE")
            .expect("Find failed")
            .expect("Dir not found");
        let result = fs.delete(&entry);

        match result {
            Err(FatError::DirectoryNotEmpty) => {}
            _ => panic!("Expected DirectoryNotEmpty error"),
        }
    }

    #[test]
    fn test_create_multiple_files() {
        let image = create_fat32_image();
        let fs = FatFs::open(image).expect("Failed to open FAT32 image");

        let root = fs.root_dir();

        // Create several files
        for i in 0..10 {
            let name = format!("FILE{i}.TXT");
            // Need to truncate/pad to 8.3 format for the find
            fs.create_file(&root, &name).expect("Failed to create file");
        }

        // Verify all files exist
        let root = fs.root_dir();
        let entries: Vec<_> = root.entries().collect();
        assert_eq!(entries.len(), 10, "Should have 10 files");
    }

    #[test]
    fn test_case_insensitive_find() {
        let image = create_fat32_image();
        let fs = FatFs::open(image).expect("Failed to open FAT32 image");

        let root = fs.root_dir();

        // Create a file
        fs.create_file(&root, "MYFILE.TXT")
            .expect("Failed to create file");

        // Should find it with different cases
        let root = fs.root_dir();
        assert!(root.find("MYFILE.TXT").expect("Find failed").is_some());
        assert!(root.find("myfile.txt").expect("Find failed").is_some());
        assert!(root.find("MyFile.Txt").expect("Find failed").is_some());
    }

    #[test]
    fn test_write_then_append() {
        let image = create_fat32_image();
        let fs = FatFs::open(image).expect("Failed to open FAT32 image");

        let root = fs.root_dir();

        // Create and write initial content
        let entry = fs
            .create_file(&root, "APPEND.TXT")
            .expect("Failed to create file");
        {
            let mut writer = fs.write_file(&entry).expect("Failed to get writer");
            writer.write(b"First part. ").expect("Failed to write");
            writer.finish().expect("Failed to finish");
        }

        // Note: Currently the write implementation overwrites from the beginning.
        // A proper append would need to seek to the end first.
        // For now, we just verify the basic write/read cycle works.

        let root = fs.root_dir();
        let entry = root
            .find("APPEND.TXT")
            .expect("Find failed")
            .expect("File not found");

        use hadris_fat::FatFsReadExt;
        let mut reader = fs.read_file(&entry).expect("Failed to get reader");
        let content = reader.read_to_vec().expect("Read failed");
        assert_eq!(&content, b"First part. ");
    }
}

/// Integration tests using in-memory FAT16 images
#[cfg(feature = "std")]
mod fat16_integration_tests {
    use super::fat16_image::{cluster_size, create_fat16_image};
    use hadris_fat::{FatError, FatFs, FatFsWriteExt, FatType};

    #[test]
    fn test_open_fat16_image() {
        let image = create_fat16_image();
        let fs = FatFs::open(image).expect("Failed to open FAT16 image");

        // Verify FAT type is FAT16
        assert!(
            matches!(fs.fat_type(), FatType::Fat16),
            "Expected FAT16, got {:?}",
            fs.fat_type()
        );

        // Should be able to get root directory
        let root = fs.root_dir();
        let entries: Vec<_> = root.entries().collect();
        // Root directory should be empty initially
        assert!(entries.is_empty(), "Root directory should be empty");
    }

    #[test]
    fn test_fat16_create_file() {
        let image = create_fat16_image();
        let fs = FatFs::open(image).expect("Failed to open FAT16 image");

        let root = fs.root_dir();

        // Create a new file
        let entry = fs
            .create_file(&root, "TEST.TXT")
            .expect("Failed to create file");

        assert!(entry.is_file());
        assert_eq!(entry.len(), 0);

        // Verify file appears in directory listing
        let root = fs.root_dir();
        let found = root.find("TEST.TXT").expect("Find failed");
        assert!(found.is_some(), "File should be found in directory");
    }

    #[test]
    fn test_fat16_write_file_content() {
        let image = create_fat16_image();
        let fs = FatFs::open(image).expect("Failed to open FAT16 image");

        let root = fs.root_dir();

        // Create a file
        let entry = fs
            .create_file(&root, "HELLO.TXT")
            .expect("Failed to create file");

        // Write content
        let content = b"Hello, FAT16 World!";
        {
            let mut writer = fs.write_file(&entry).expect("Failed to get writer");
            writer.write(content).expect("Failed to write");
            writer.finish().expect("Failed to finish");
        }

        // Re-find the file to get updated size
        let root = fs.root_dir();
        let entry = root
            .find("HELLO.TXT")
            .expect("Find failed")
            .expect("File not found");
        assert_eq!(entry.len(), content.len() as u64);

        // Read back the content
        use hadris_fat::FatFsReadExt;
        let mut reader = fs.read_file(&entry).expect("Failed to get reader");
        let mut buf = vec![0u8; content.len()];
        let mut total = 0;
        while total < buf.len() {
            let n = reader.read(&mut buf[total..]).expect("Read failed");
            if n == 0 {
                break;
            }
            total += n;
        }
        assert_eq!(&buf[..total], content);
    }

    #[test]
    fn test_fat16_write_file_multiple_clusters() {
        let image = create_fat16_image();
        let fs = FatFs::open(image).expect("Failed to open FAT16 image");

        let root = fs.root_dir();

        // Create a file
        let entry = fs
            .create_file(&root, "BIG.DAT")
            .expect("Failed to create file");

        // Write content larger than one cluster
        let cluster_sz = cluster_size();
        let content: Vec<u8> = (0..cluster_sz * 3).map(|i| (i % 256) as u8).collect();

        {
            let mut writer = fs.write_file(&entry).expect("Failed to get writer");
            writer.write(&content).expect("Failed to write");
            writer.finish().expect("Failed to finish");
        }

        // Read back and verify
        let root = fs.root_dir();
        let entry = root
            .find("BIG.DAT")
            .expect("Find failed")
            .expect("File not found");
        assert_eq!(entry.len(), content.len() as u64);

        use hadris_fat::FatFsReadExt;
        let mut reader = fs.read_file(&entry).expect("Failed to get reader");
        let read_content = reader.read_to_vec().expect("Read failed");
        assert_eq!(read_content, content);
    }

    #[test]
    fn test_fat16_create_directory() {
        let image = create_fat16_image();
        let fs = FatFs::open(image).expect("Failed to open FAT16 image");

        let root = fs.root_dir();

        // Create a directory
        let subdir = fs
            .create_dir(&root, "SUBDIR")
            .expect("Failed to create directory");

        // Verify . and .. entries exist
        let entries: Vec<_> = subdir.entries().collect();
        let names: Vec<_> = entries
            .iter()
            .filter_map(|e| e.as_ref().ok())
            .map(|e| e.name().trim_end_matches(' ').to_string())
            .collect();

        assert!(
            names.iter().any(|n| n.starts_with('.')),
            "Directory should have . entry: {names:?}"
        );
        assert!(
            names.iter().any(|n| n.starts_with("..")),
            "Directory should have .. entry: {names:?}"
        );
    }

    #[test]
    fn test_fat16_create_file_in_subdirectory() {
        let image = create_fat16_image();
        let fs = FatFs::open(image).expect("Failed to open FAT16 image");

        let root = fs.root_dir();

        // Create a directory
        let subdir = fs
            .create_dir(&root, "MYDIR")
            .expect("Failed to create directory");

        // Create a file in the subdirectory
        let entry = fs
            .create_file(&subdir, "FILE.TXT")
            .expect("Failed to create file");
        assert!(entry.is_file());

        // Verify file exists in subdirectory
        let found = subdir.find("FILE.TXT").expect("Find failed");
        assert!(found.is_some());

        // Verify file does NOT exist in root
        let root = fs.root_dir();
        let found_in_root = root.find("FILE.TXT").expect("Find failed");
        assert!(found_in_root.is_none());
    }

    #[test]
    fn test_fat16_delete_file() {
        let image = create_fat16_image();
        let fs = FatFs::open(image).expect("Failed to open FAT16 image");

        let root = fs.root_dir();

        // Create and write a file
        let entry = fs
            .create_file(&root, "DELETE.ME")
            .expect("Failed to create file");
        {
            let mut writer = fs.write_file(&entry).expect("Failed to get writer");
            writer.write(b"temporary data").expect("Failed to write");
            writer.finish().expect("Failed to finish");
        }

        // Re-find and delete
        let root = fs.root_dir();
        let entry = root
            .find("DELETE.ME")
            .expect("Find failed")
            .expect("File not found");
        fs.delete(&entry).expect("Failed to delete");

        // Verify file no longer exists
        let root = fs.root_dir();
        let found = root.find("DELETE.ME").expect("Find failed");
        assert!(found.is_none(), "File should be deleted");
    }

    #[test]
    fn test_fat16_create_multiple_files() {
        let image = create_fat16_image();
        let fs = FatFs::open(image).expect("Failed to open FAT16 image");

        let root = fs.root_dir();

        // Create several files
        for i in 0..10 {
            let name = format!("FILE{i}.TXT");
            fs.create_file(&root, &name).expect("Failed to create file");
        }

        // Verify all files exist
        let root = fs.root_dir();
        let entries: Vec<_> = root.entries().collect();
        assert_eq!(entries.len(), 10, "Should have 10 files");
    }

    #[test]
    fn test_fat16_file_already_exists() {
        let image = create_fat16_image();
        let fs = FatFs::open(image).expect("Failed to open FAT16 image");

        let root = fs.root_dir();

        // Create a file
        fs.create_file(&root, "TEST.TXT")
            .expect("Failed to create file");

        // Try to create again - should fail
        let result = fs.create_file(&root, "TEST.TXT");
        match result {
            Err(FatError::AlreadyExists) => {}
            _ => panic!("Expected AlreadyExists error"),
        }
    }
}

/// Integration tests using in-memory FAT12 images
#[cfg(feature = "std")]
mod fat12_integration_tests {
    use super::fat12_image::{cluster_size, create_fat12_image};
    use hadris_fat::{FatError, FatFs, FatFsWriteExt, FatType};

    #[test]
    fn test_open_fat12_image() {
        let image = create_fat12_image();
        let fs = FatFs::open(image).expect("Failed to open FAT12 image");

        // Verify FAT type is FAT12
        assert!(
            matches!(fs.fat_type(), FatType::Fat12),
            "Expected FAT12, got {:?}",
            fs.fat_type()
        );

        // Should be able to get root directory
        let root = fs.root_dir();
        let entries: Vec<_> = root.entries().collect();
        // Root directory should be empty initially
        assert!(entries.is_empty(), "Root directory should be empty");
    }

    #[test]
    fn test_fat12_create_file() {
        let image = create_fat12_image();
        let fs = FatFs::open(image).expect("Failed to open FAT12 image");

        let root = fs.root_dir();

        // Create a new file
        let entry = fs
            .create_file(&root, "TEST.TXT")
            .expect("Failed to create file");

        assert!(entry.is_file());
        assert_eq!(entry.len(), 0);

        // Verify file appears in directory listing
        let root = fs.root_dir();
        let found = root.find("TEST.TXT").expect("Find failed");
        assert!(found.is_some(), "File should be found in directory");
    }

    #[test]
    fn test_fat12_write_file_content() {
        let image = create_fat12_image();
        let fs = FatFs::open(image).expect("Failed to open FAT12 image");

        let root = fs.root_dir();

        // Create a file
        let entry = fs
            .create_file(&root, "HELLO.TXT")
            .expect("Failed to create file");

        // Write content
        let content = b"Hello, FAT12 World!";
        {
            let mut writer = fs.write_file(&entry).expect("Failed to get writer");
            writer.write(content).expect("Failed to write");
            writer.finish().expect("Failed to finish");
        }

        // Re-find the file to get updated size
        let root = fs.root_dir();
        let entry = root
            .find("HELLO.TXT")
            .expect("Find failed")
            .expect("File not found");
        assert_eq!(entry.len(), content.len() as u64);

        // Read back the content
        use hadris_fat::FatFsReadExt;
        let mut reader = fs.read_file(&entry).expect("Failed to get reader");
        let mut buf = vec![0u8; content.len()];
        let mut total = 0;
        while total < buf.len() {
            let n = reader.read(&mut buf[total..]).expect("Read failed");
            if n == 0 {
                break;
            }
            total += n;
        }
        assert_eq!(&buf[..total], content);
    }

    #[test]
    fn test_fat12_write_file_multiple_clusters() {
        let image = create_fat12_image();
        let fs = FatFs::open(image).expect("Failed to open FAT12 image");

        let root = fs.root_dir();

        // Create a file
        let entry = fs
            .create_file(&root, "BIG.DAT")
            .expect("Failed to create file");

        // Write content larger than one cluster
        let cluster_sz = cluster_size();
        let content: Vec<u8> = (0..cluster_sz * 3).map(|i| (i % 256) as u8).collect();

        {
            let mut writer = fs.write_file(&entry).expect("Failed to get writer");
            writer.write(&content).expect("Failed to write");
            writer.finish().expect("Failed to finish");
        }

        // Read back and verify
        let root = fs.root_dir();
        let entry = root
            .find("BIG.DAT")
            .expect("Find failed")
            .expect("File not found");
        assert_eq!(entry.len(), content.len() as u64);

        use hadris_fat::FatFsReadExt;
        let mut reader = fs.read_file(&entry).expect("Failed to get reader");
        let read_content = reader.read_to_vec().expect("Read failed");
        assert_eq!(read_content, content);
    }

    #[test]
    fn test_fat12_create_directory() {
        let image = create_fat12_image();
        let fs = FatFs::open(image).expect("Failed to open FAT12 image");

        let root = fs.root_dir();

        // Create a directory
        let subdir = fs
            .create_dir(&root, "SUBDIR")
            .expect("Failed to create directory");

        // Verify . and .. entries exist
        let entries: Vec<_> = subdir.entries().collect();
        let names: Vec<_> = entries
            .iter()
            .filter_map(|e| e.as_ref().ok())
            .map(|e| e.name().trim_end_matches(' ').to_string())
            .collect();

        assert!(
            names.iter().any(|n| n.starts_with('.')),
            "Directory should have . entry: {names:?}"
        );
        assert!(
            names.iter().any(|n| n.starts_with("..")),
            "Directory should have .. entry: {names:?}"
        );
    }

    #[test]
    fn test_fat12_create_file_in_subdirectory() {
        let image = create_fat12_image();
        let fs = FatFs::open(image).expect("Failed to open FAT12 image");

        let root = fs.root_dir();

        // Create a directory
        let subdir = fs
            .create_dir(&root, "MYDIR")
            .expect("Failed to create directory");

        // Create a file in the subdirectory
        let entry = fs
            .create_file(&subdir, "FILE.TXT")
            .expect("Failed to create file");
        assert!(entry.is_file());

        // Verify file exists in subdirectory
        let found = subdir.find("FILE.TXT").expect("Find failed");
        assert!(found.is_some());

        // Verify file does NOT exist in root
        let root = fs.root_dir();
        let found_in_root = root.find("FILE.TXT").expect("Find failed");
        assert!(found_in_root.is_none());
    }

    #[test]
    fn test_fat12_delete_file() {
        let image = create_fat12_image();
        let fs = FatFs::open(image).expect("Failed to open FAT12 image");

        let root = fs.root_dir();

        // Create and write a file
        let entry = fs
            .create_file(&root, "DELETE.ME")
            .expect("Failed to create file");
        {
            let mut writer = fs.write_file(&entry).expect("Failed to get writer");
            writer.write(b"temporary data").expect("Failed to write");
            writer.finish().expect("Failed to finish");
        }

        // Re-find and delete
        let root = fs.root_dir();
        let entry = root
            .find("DELETE.ME")
            .expect("Find failed")
            .expect("File not found");
        fs.delete(&entry).expect("Failed to delete");

        // Verify file no longer exists
        let root = fs.root_dir();
        let found = root.find("DELETE.ME").expect("Find failed");
        assert!(found.is_none(), "File should be deleted");
    }

    #[test]
    fn test_fat12_create_multiple_files() {
        let image = create_fat12_image();
        let fs = FatFs::open(image).expect("Failed to open FAT12 image");

        let root = fs.root_dir();

        // Create several files
        for i in 0..10 {
            let name = format!("FILE{i}.TXT");
            fs.create_file(&root, &name).expect("Failed to create file");
        }

        // Verify all files exist
        let root = fs.root_dir();
        let entries: Vec<_> = root.entries().collect();
        assert_eq!(entries.len(), 10, "Should have 10 files");
    }

    #[test]
    fn test_fat12_file_already_exists() {
        let image = create_fat12_image();
        let fs = FatFs::open(image).expect("Failed to open FAT12 image");

        let root = fs.root_dir();

        // Create a file
        fs.create_file(&root, "TEST.TXT")
            .expect("Failed to create file");

        // Try to create again - should fail
        let result = fs.create_file(&root, "TEST.TXT");
        match result {
            Err(FatError::AlreadyExists) => {}
            _ => panic!("Expected AlreadyExists error"),
        }
    }

    /// Test FAT12 specific: 12-bit entry encoding edge cases
    #[test]
    fn test_fat12_cluster_chain() {
        let image = create_fat12_image();
        let fs = FatFs::open(image).expect("Failed to open FAT12 image");

        let root = fs.root_dir();

        // Create a file that spans multiple clusters to test FAT12 cluster chain traversal
        let cluster_sz = cluster_size();
        // Use 5 clusters to test both even and odd cluster number handling
        let content: Vec<u8> = (0..cluster_sz * 5).map(|i| (i % 256) as u8).collect();

        let entry = fs
            .create_file(&root, "CHAIN.DAT")
            .expect("Failed to create file");
        {
            let mut writer = fs.write_file(&entry).expect("Failed to get writer");
            writer.write(&content).expect("Failed to write");
            writer.finish().expect("Failed to finish");
        }

        // Read back and verify
        let root = fs.root_dir();
        let entry = root
            .find("CHAIN.DAT")
            .expect("Find failed")
            .expect("File not found");
        assert_eq!(entry.len(), content.len() as u64);

        use hadris_fat::FatFsReadExt;
        let mut reader = fs.read_file(&entry).expect("Failed to get reader");
        let read_content = reader.read_to_vec().expect("Read failed");
        assert_eq!(read_content, content, "FAT12 cluster chain read mismatch");
    }
}

/// Corrupt-image robustness tests: cycles in the FAT chain must surface as
/// `FatError::ClusterLoop` instead of hanging the reader. Regression coverage
/// for issue B4 (chain-walk loop guard).
#[cfg(feature = "std")]
mod corrupt_image_tests {
    use super::fat32_image::{
        cluster_size, create_fat32_image, data_start_bytes, fat_start_bytes, fat2_start_bytes,
    };
    use hadris_fat::{FatError, FatFs, FatFsReadExt};
    use std::io::Cursor;

    /// Patch a FAT32 entry in *both* FAT copies (primary and backup), so the
    /// fallback path can't paper over our deliberate cycle.
    fn patch_fat_entry(image: &mut [u8], cluster: u32, value: u32) {
        let fat1 = fat_start_bytes() + cluster as usize * 4;
        let fat2 = fat2_start_bytes() + cluster as usize * 4;
        let bytes = value.to_le_bytes();
        image[fat1..fat1 + 4].copy_from_slice(&bytes);
        image[fat2..fat2 + 4].copy_from_slice(&bytes);
    }

    /// FAT[3] -> 4, FAT[4] -> 3 is a 2-cluster cycle. Mounting an image with
    /// such a cycle and then reading a file claiming first_cluster=3 must
    /// return `ClusterLoop` rather than hang or panic.
    #[test]
    fn test_read_returns_cluster_loop_on_2cycle() {
        let image = create_fat32_image();
        let mut bytes = image.into_inner();

        // Install the FAT cycle.
        patch_fat_entry(&mut bytes, 3, 4);
        patch_fat_entry(&mut bytes, 4, 3);

        // Hand-build a directory entry at root cluster (cluster 2), offset 0,
        // for a file "LOOP.DAT" claiming to span four clusters starting at
        // cluster 3. We don't go through `create_file` because we need the
        // first_cluster to point at our planted cycle.
        let entry_offset = data_start_bytes();
        let mut entry = [0u8; 32];
        entry[0..11].copy_from_slice(b"LOOP    DAT");
        entry[11] = 0x20; // ARCHIVE
        // first_cluster_high (offset 20..22) = 0
        // first_cluster_low (offset 26..28) = 3
        entry[26..28].copy_from_slice(&3u16.to_le_bytes());
        // size (offset 28..32) — set large enough to force more cluster
        // transitions than the FS contains, so the cycle is provably hit.
        // Without this, a small `size` lets the reader stop before walking
        // past the planted loop.
        entry[28..32].copy_from_slice(&u32::MAX.to_le_bytes());
        bytes[entry_offset..entry_offset + 32].copy_from_slice(&entry);

        // Mount and read.
        let fs = FatFs::open(Cursor::new(bytes)).expect("mount");
        let root = fs.root_dir();
        let file = root.find("LOOP.DAT").expect("find").expect("entry present");
        let mut reader = fs.read_file(&file).expect("reader");

        // Read into a small buffer in a loop so we don't try to allocate
        // u32::MAX bytes up front. The cycle should be detected within ~max
        // cluster transitions; bail after a generous-but-finite number.
        let mut scratch = vec![0u8; cluster_size()];
        let mut last = Ok(0usize);
        for _ in 0..10_000 {
            match reader.read(&mut scratch) {
                Ok(0) => panic!("reader hit EOF without detecting cycle"),
                Ok(_) => continue,
                Err(e) => {
                    last = Err(e);
                    break;
                }
            }
        }
        match last {
            Err(FatError::ClusterLoop { .. }) => {}
            Err(other) => panic!("expected ClusterLoop, got {other:?}"),
            Ok(_) => panic!("expected ClusterLoop, no error after 10k reads"),
        }
    }

    /// `create_file` with a long, mixed-case, multi-segment name must persist
    /// LFN entries so the original name round-trips. Without LFN write support
    /// the name was silently truncated to 8.3 — this test catches that
    /// regression. Coverage for issue A1 (LFN write).
    #[cfg(feature = "lfn")]
    #[test]
    fn test_long_name_roundtrips_via_lfn_entries() {
        let image = create_fat32_image();
        let fs = FatFs::open(image).expect("open");
        let root = fs.root_dir();

        let long_name = "My Long Notes.txt";
        fs.create_file(&root, long_name).expect("create_file");

        // Re-open by re-iterating root: the long name should match exactly.
        let root = fs.root_dir();
        let entry = root
            .find(long_name)
            .expect("find did not error")
            .expect("entry should be found by long name");

        // Either the LFN matches or, for a short-enough name that doesn't
        // require LFN, the short name should equal it.
        let long_matched = entry
            .long_name()
            .map(|lfn| lfn.eq_str(long_name))
            .unwrap_or(false);
        assert!(
            long_matched,
            "LongFileName should round-trip the original name; got {:?}",
            entry.long_name()
        );
    }

    /// Names that already fit 8.3 cleanly (e.g. "TEST.TXT") shouldn't get LFN
    /// entries written — that would waste 32 bytes per slot for no benefit
    /// and creates extra entries the on-disk layout doesn't need.
    #[cfg(feature = "lfn")]
    #[test]
    fn test_short_compliant_name_skips_lfn() {
        let image = create_fat32_image();
        let fs = FatFs::open(image).expect("open");
        let root = fs.root_dir();

        fs.create_file(&root, "TEST.TXT").expect("create");

        let root = fs.root_dir();
        let entry = root.find("TEST.TXT").expect("find").expect("entry");
        assert!(
            entry.long_name().is_none(),
            "names that fit 8.3 should not produce LFN entries"
        );
    }

    /// Mounting via `FatFsBuilder::with_fat_cache` should install the cache
    /// without changing observable read/write behaviour. Compares writing
    /// the same file with cache on vs off and asserts both files round-trip
    /// to identical bytes. Regression coverage for issue C (Phase C cache
    /// integration plumbing).
    #[cfg(feature = "cache")]
    #[test]
    fn test_with_fat_cache_roundtrips_byte_identical() {
        use hadris_fat::FatFsWriteExt;

        let payload: Vec<u8> = (0..(cluster_size() * 3))
            .map(|i| (i & 0xFF) as u8)
            .collect();

        let read_back = |with_cache: bool| -> Vec<u8> {
            let image = create_fat32_image();
            let fs = if with_cache {
                FatFs::builder(image)
                    .fat_cache(8)
                    .open()
                    .expect("builder open")
            } else {
                FatFs::builder(image).open().expect("builder open")
            };
            let root = fs.root_dir();
            let entry = fs.create_file(&root, "CACHE.DAT").expect("create");
            {
                let mut writer = fs.write_file(&entry).expect("writer");
                writer.write(&payload).expect("write");
                writer.finish().expect("finish");
            }
            // Re-find the entry so we get the persisted size + first cluster.
            let root = fs.root_dir();
            let entry = root.find("CACHE.DAT").expect("find").expect("entry");
            let mut reader = fs.read_file(&entry).expect("reader");
            reader.read_to_vec().expect("read")
        };

        let without = read_back(false);
        let with = read_back(true);
        assert_eq!(
            with, without,
            "cache should not affect read/write semantics"
        );
        assert_eq!(with, payload, "round-trip mismatch with cache enabled");
    }

    /// FSInfo's `FSI_Free_Count` and `FSI_Nxt_Free` are allowed to be the
    /// sentinel `0xFFFFFFFF` ("unknown" per the FAT32 spec). A mount must
    /// accept that value gracefully — bumping it to fatal would make many
    /// real-world Windows images unreadable. Regression coverage for issue B2.
    #[test]
    fn test_fsinfo_unknown_sentinels_mount_successfully() {
        let image = create_fat32_image();
        let mut bytes = image.into_inner();

        // FSInfo lives at sector 1 (512..1024). The relevant fields:
        //   offset 488..492 — FSI_Free_Count
        //   offset 492..496 — FSI_Nxt_Free
        // Set both to the "unknown" sentinel.
        let fsi = 512;
        bytes[fsi + 488..fsi + 492].copy_from_slice(&0xFFFFFFFFu32.to_le_bytes());
        bytes[fsi + 492..fsi + 496].copy_from_slice(&0xFFFFFFFFu32.to_le_bytes());

        let fs =
            FatFs::open(Cursor::new(bytes)).expect("mount must succeed with unknown sentinels");
        assert_eq!(
            fs.free_cluster_count(),
            None,
            "free_cluster_count() should report None when FSInfo says unknown"
        );
        assert_eq!(
            fs.next_free_cluster_hint(),
            None,
            "next_free_cluster_hint() should report None when FSInfo says unknown"
        );
    }

    /// A truncated image — too small to even read the boot sector — should
    /// surface an [`FatError::IoContext`] mentioning "boot sector" so users
    /// know which structure failed, rather than the bare `Io(...)` they used
    /// to get. Regression coverage for issue B3.
    #[test]
    fn test_truncated_image_returns_boot_sector_context() {
        // 16 bytes is far less than a boot sector (512 bytes). Any sane FS
        // open must fail here — the question is *how*.
        let tiny = Cursor::new(vec![0u8; 16]);
        let err = FatFs::open(tiny).expect_err("mount must fail on tiny image");
        match &err {
            FatError::IoContext { op, .. } => {
                assert!(
                    op.contains("boot sector"),
                    "expected op to mention 'boot sector', got {op:?}"
                );
            }
            other => panic!("expected IoContext, got {other:?}"),
        }
        // Display should also mention the operation.
        assert!(
            format!("{err}").contains("boot sector"),
            "Display should mention 'boot sector', got {err}"
        );
    }

    /// Same cycle, but exercise it through directory iteration: turn the root
    /// directory chain itself into a cycle by extending it to cluster 3 and
    /// looping cluster 3 back to cluster 2. Iterating root entries must
    /// surface `ClusterLoop`.
    #[test]
    fn test_dir_iter_returns_cluster_loop_on_root_cycle() {
        let image = create_fat32_image();
        let mut bytes = image.into_inner();

        // Make root extend to cluster 3, and cluster 3 back to cluster 2.
        patch_fat_entry(&mut bytes, 2, 3);
        patch_fat_entry(&mut bytes, 3, 2);

        // Fill root cluster (cluster 2) with non-zero entries so iteration
        // doesn't terminate before the cluster transition. We use 16 deleted
        // entries (0xE5 in name[0]) — they're skipped semantically but force
        // the iterator to scan past the end of the cluster.
        let cs = cluster_size();
        let cluster2_start = data_start_bytes();
        for i in 0..(cs / 32) {
            bytes[cluster2_start + i * 32] = 0xE5;
        }
        // Cluster 3 starts at data_start + cluster_size; same fill.
        let cluster3_start = data_start_bytes() + cs;
        for i in 0..(cs / 32) {
            bytes[cluster3_start + i * 32] = 0xE5;
        }

        let fs = FatFs::open(Cursor::new(bytes)).expect("mount");
        let root = fs.root_dir();

        // Iterate. The cycle should surface as ClusterLoop within
        // max_cluster steps.
        let mut saw_loop = false;
        for entry in root.entries() {
            if let Err(FatError::ClusterLoop { .. }) = entry {
                saw_loop = true;
                break;
            }
        }
        assert!(saw_loop, "expected directory iter to surface ClusterLoop");
    }
}

/// LFN-write edge-case tests. The MVP `build_lfn_entries` path added in
/// commit b4630d6 emits entries for non-8.3 names; these tests cover the
/// corners that the basic round-trip test doesn't:
///   * supplementary-plane chars (surrogate pairs) round-trip
///   * exactly 20 LFN entries works; 21 fails
///   * names that exceed the 255-UTF-16-unit spec cap surface as
///     `InvalidFilename` (rather than silently truncating)
///   * padding invariant: 0x0000 terminator + 0xFFFF tail filler
///   * every LFN entry's checksum byte equals the short name's checksum
///   * delete cleans up preceding LFN slots, not just the short entry
#[cfg(all(feature = "std", feature = "lfn"))]
mod lfn_write_edge_tests {
    use super::fat32_image::{create_fat32_image, data_start_bytes};
    use hadris_fat::raw::{DirEntryAttrFlags, RawDirectoryEntry};
    use hadris_fat::{FatError, FatFs};
    use std::io::Cursor;

    /// LFN entry attribute byte (READ_ONLY | HIDDEN | SYSTEM | VOLUME_ID).
    const LFN_ATTR: u8 = DirEntryAttrFlags::LONG_NAME.bits();

    /// Open a FAT32 image and return the inner Vec so the caller can inspect
    /// the bytes between `FatFs` operations.
    fn fresh_fat32_bytes() -> Vec<u8> {
        create_fat32_image().into_inner()
    }

    /// Read the directory entry at slot `i` of the root directory (cluster 2).
    fn read_root_slot(bytes: &[u8], i: usize) -> [u8; 32] {
        let pos = data_start_bytes() + i * 32;
        bytes[pos..pos + 32].try_into().unwrap()
    }

    /// Decode the `n`th UTF-16LE code unit of an LFN entry's three-part name.
    fn lfn_unit(slot: &[u8; 32], n: usize) -> u16 {
        // name1[0..5] -> bytes 1..11
        // name2[0..6] -> bytes 14..26
        // name3[0..2] -> bytes 28..32
        let off = match n {
            0..=4 => 1 + n * 2,
            5..=10 => 14 + (n - 5) * 2,
            11..=12 => 28 + (n - 11) * 2,
            _ => panic!("LFN unit index out of range: {n}"),
        };
        u16::from_le_bytes([slot[off], slot[off + 1]])
    }

    /// FAT-spec short-name checksum algorithm (matches
    /// `ShortFileName::lfn_checksum`).
    fn short_checksum(name: &[u8; 11]) -> u8 {
        let mut sum: u8 = 0;
        for &b in name {
            sum = sum.rotate_right(1).wrapping_add(b);
        }
        sum
    }

    #[test]
    fn lfn_supplementary_plane_emoji_roundtrips() {
        // U+1F31F (🌟) encodes as a 2-unit UTF-16 surrogate pair. A name
        // mixing ASCII + emoji + extension must round-trip *exactly*; if the
        // encoder normalized or replaced the surrogates the read-back string
        // would no longer compare equal.
        let mut bytes = fresh_fat32_bytes();
        let long_name = "Star \u{1F31F} Notes.txt";
        {
            let cursor = Cursor::new(&mut bytes[..]);
            let fs = FatFs::open(cursor).expect("open");
            fs.create_file(&fs.root_dir(), long_name).expect("create");
        }
        let cursor = Cursor::new(&bytes[..]);
        let fs = FatFs::open(cursor).expect("re-open");
        let entry = fs.root_dir().find(long_name).expect("find").expect("entry");
        let lfn = entry.long_name().expect("LFN required");
        assert!(lfn.eq_str(long_name), "LFN must round-trip surrogate pairs");
    }

    #[test]
    fn lfn_overlong_name_returns_invalid_filename() {
        // 256 UTF-16 code units exceeds the FAT spec cap of 255. The
        // encoder must surface this as `InvalidFilename` rather than
        // silently truncating — quietly losing bytes from a filename is
        // worse than refusing to create the file.
        let mut bytes = fresh_fat32_bytes();
        let long: String = std::iter::repeat_n('a', 256).collect();
        let cursor = Cursor::new(&mut bytes[..]);
        let fs = FatFs::open(cursor).expect("open");
        match fs.create_file(&fs.root_dir(), &long) {
            Err(FatError::InvalidFilename) => {}
            Err(other) => panic!("expected InvalidFilename, got {other:?}"),
            Ok(_) => panic!("expected error for 256-unit name"),
        }
    }

    #[test]
    fn lfn_long_name_15_entries_exact_fill_roundtrips() {
        // 15 entries × 13 chars/entry = 195 chars exactly. Tests two
        // invariants at once: long names with many LFN entries round-trip,
        // and the "exact fill" code path emits no terminator/filler bytes.
        //
        // The default test fixture uses 1 sector/cluster (16 entries per
        // cluster) so the run cap (`count <= entries_per_cluster` at
        // write.rs:788) limits us to 15 LFN + 1 short = 16 slots. Testing
        // the spec maximum of 20 LFN entries (21 slots) would need a
        // fixture with a larger cluster; the related cap is exercised by
        // `lfn_run_too_long_returns_directory_full` below.
        let mut bytes = fresh_fat32_bytes();
        let long: String = std::iter::repeat_n('a', 195).collect();
        {
            let cursor = Cursor::new(&mut bytes[..]);
            let fs = FatFs::open(cursor).expect("open");
            fs.create_file(&fs.root_dir(), &long)
                .expect("create 195-char name");
        }

        // Slot 0 is the highest-seq LFN entry, holding the *last* 13-char
        // chunk. With exact-fill (195 chars), every slot of that chunk is
        // a real char — no 0x0000 terminator, no 0xFFFF filler.
        let slot0 = read_root_slot(&bytes, 0);
        assert_eq!(slot0[11], LFN_ATTR);
        for i in 0..13 {
            let unit = lfn_unit(&slot0, i);
            assert_eq!(
                unit, b'a' as u16,
                "exact-fill last entry must contain only real chars; unit {i} was {unit:#06x}"
            );
        }

        let cursor = Cursor::new(&bytes[..]);
        let fs = FatFs::open(cursor).expect("re-open");
        let entry = fs.root_dir().find(&long).expect("find").expect("entry");
        assert!(entry.long_name().expect("lfn").eq_str(&long));
    }

    #[test]
    fn lfn_run_crosses_cluster_boundary() {
        let mut bytes = fresh_fat32_bytes();
        // 16 LFN entries + one short entry requires 17 slots, crossing from
        // the 16-slot root cluster into a newly allocated cluster.
        let too_long: String = std::iter::repeat_n('a', 208).collect();
        {
            let cursor = Cursor::new(&mut bytes[..]);
            let fs = FatFs::open(cursor).expect("open");
            fs.create_file(&fs.root_dir(), &too_long)
                .expect("cross-cluster LFN create");
        }

        let cursor = Cursor::new(&bytes[..]);
        let fs = FatFs::open(cursor).expect("re-open");
        let entry = fs.root_dir().find(&too_long).expect("find").expect("entry");
        assert!(entry.long_name().expect("lfn").eq_str(&too_long));
    }

    #[test]
    fn lfn_padding_uses_terminator_then_filler() {
        // A 14-char name needs 2 LFN entries: entry 1 holds chars 1..13,
        // entry 2 holds char 14. The remaining 12 slots in entry 2 must be
        // `0x0000` (terminator, immediately after the last real char) then
        // all-`0xFFFF` (per spec). A reader trips on this padding to detect
        // the LFN length without needing the original short-entry checksum.
        let mut bytes = fresh_fat32_bytes();
        let name = "longishname.tx"; // 14 ASCII chars
        {
            let cursor = Cursor::new(&mut bytes[..]);
            let fs = FatFs::open(cursor).expect("open");
            fs.create_file(&fs.root_dir(), name).expect("create");
        }

        // Slot 0 holds the highest-seq LFN entry (last 13-unit chunk); for a
        // 14-char name that's chunk #2 = 1 char + padding. Verify padding.
        let slot0 = read_root_slot(&bytes, 0);
        assert_eq!(slot0[11], LFN_ATTR, "slot 0 must be an LFN entry");
        // Char 14 is the only real char in this entry (index 0 in chunk).
        assert_eq!(lfn_unit(&slot0, 0), b'x' as u16);
        // Char index 1 must be the 0x0000 terminator.
        assert_eq!(lfn_unit(&slot0, 1), 0x0000, "expected terminator at unit 1");
        // Remaining 11 units must all be 0xFFFF filler.
        for i in 2..13 {
            assert_eq!(
                lfn_unit(&slot0, i),
                0xFFFF,
                "expected 0xFFFF filler at unit {i}, got {:#06x}",
                lfn_unit(&slot0, i)
            );
        }
    }

    #[test]
    fn lfn_checksum_matches_short_name() {
        // Every LFN entry's `checksum` byte must match the short-name
        // checksum stored in the trailing short entry. A single-byte
        // mismatch invalidates the entire LFN sequence per spec, so this
        // is critical to round-trip on any conforming FAT reader.
        let mut bytes = fresh_fat32_bytes();
        let name = "Mixed-Case Name.dat";
        {
            let cursor = Cursor::new(&mut bytes[..]);
            let fs = FatFs::open(cursor).expect("open");
            fs.create_file(&fs.root_dir(), name).expect("create");
        }

        // Walk slots 0..N until we hit the non-LFN short entry; collect
        // checksums along the way and compare.
        let mut lfn_checksums = vec![];
        let mut short_name_bytes = [0u8; 11];
        for i in 0..21 {
            let slot = read_root_slot(&bytes, i);
            if slot[11] == LFN_ATTR {
                lfn_checksums.push(slot[13]);
            } else if slot[0] != 0x00 && slot[0] != 0xE5 {
                short_name_bytes.copy_from_slice(&slot[0..11]);
                break;
            }
        }
        assert!(
            !lfn_checksums.is_empty(),
            "expected LFN entries for {name:?}"
        );
        let expected = short_checksum(&short_name_bytes);
        for (i, c) in lfn_checksums.iter().enumerate() {
            assert_eq!(
                *c, expected,
                "LFN slot {i} checksum {c:#04x} != short-name checksum {expected:#04x}"
            );
        }
    }

    /// REGRESSION TEST for the orphaned-LFN-slot bug: before this commit,
    /// `delete` only marked the short entry as 0xE5, leaving the preceding
    /// LFN entries untouched on disk. fsck.fat flags those as "stray
    /// long-name slots". The fix walks backward over the LFN run and marks
    /// each slot deleted too.
    #[test]
    fn lfn_create_then_delete_marks_all_lfn_slots() {
        let mut bytes = fresh_fat32_bytes();
        let name = "Long Mixed Notes.txt"; // forces LFN
        {
            let cursor = Cursor::new(&mut bytes[..]);
            let fs = FatFs::open(cursor).expect("open");
            let _entry = fs.create_file(&fs.root_dir(), name).expect("create");
            // Re-find then delete (FileEntry from create may pre-date the
            // disk write order; re-finding gets a fresh, consistent handle).
            let entry = fs.root_dir().find(name).expect("find").expect("entry");
            fs.delete(&entry).expect("delete");
        }

        // Every slot that was either LFN or the short entry must now have
        // its first byte == 0xE5. Walk the first 21 slots (max LFN run + 1)
        // and verify any non-zero first byte is 0xE5.
        for i in 0..21 {
            let slot = read_root_slot(&bytes, i);
            if slot[0] == 0x00 {
                // End of directory — everything after is unallocated.
                break;
            }
            assert_eq!(
                slot[0], 0xE5,
                "slot {i} should be marked deleted (got first byte {:#04x})",
                slot[0]
            );
        }
    }

    /// Mostly redundant given `lfn_create_then_delete_marks_all_lfn_slots`,
    /// but worth a separate test: the existing `find` API must NOT see the
    /// deleted name (or any of its LFN bytes leaking through).
    #[test]
    fn lfn_find_does_not_resurrect_deleted_long_name() {
        let mut bytes = fresh_fat32_bytes();
        let name = "Long Mixed Notes.txt";
        {
            let cursor = Cursor::new(&mut bytes[..]);
            let fs = FatFs::open(cursor).expect("open");
            let _ = fs.create_file(&fs.root_dir(), name).expect("create");
            let entry = fs.root_dir().find(name).expect("find").expect("entry");
            fs.delete(&entry).expect("delete");
        }
        let cursor = Cursor::new(&bytes[..]);
        let fs = FatFs::open(cursor).expect("re-open");
        assert!(
            fs.root_dir().find(name).expect("find").is_none(),
            "deleted long-name file must not be findable"
        );
    }

    /// Use `RawDirectoryEntry` size to confirm our 32-byte slot assumption
    /// hasn't drifted — this protects the `read_root_slot` math above.
    #[test]
    fn raw_directory_entry_is_32_bytes() {
        assert_eq!(core::mem::size_of::<RawDirectoryEntry>(), 32);
    }
}

/// Tests for the volume-metadata APIs added in commit 3d637ee
/// (`read_root_label`, `set_root_label`, `read_status_flags`) and the
/// pluggable OEM converter (`Cp437OemCpConverter`).
#[cfg(feature = "std")]
mod fs_metadata_tests {
    use super::fat32_image::{
        create_fat32_image, data_start_bytes, fat_start_bytes, fat2_start_bytes,
    };
    use hadris_fat::FatFs;
    use hadris_fat::oem::Cp437OemCpConverter;
    use std::io::Cursor;

    /// Patch FAT32's FAT[1] entry in both copies. Status bits live in the
    /// high nibble of cluster 1's 32-bit entry: bit 27 = "clean", bit 26 =
    /// "no I/O errors" (cleared = trouble).
    fn patch_fat1_both_copies(bytes: &mut [u8], value: u32) {
        let v = value.to_le_bytes();
        let off1 = fat_start_bytes() + 4; // FAT[0], cluster 1
        let off2 = fat2_start_bytes() + 4; // FAT[1], cluster 1
        bytes[off1..off1 + 4].copy_from_slice(&v);
        bytes[off2..off2 + 4].copy_from_slice(&v);
    }

    #[test]
    fn read_root_label_returns_none_when_no_label_entry() {
        let bytes = create_fat32_image().into_inner();
        let fs = FatFs::open(Cursor::new(bytes)).expect("open");
        assert!(
            fs.read_root_label().expect("read_root_label ok").is_none(),
            "default fixture has no root label entry"
        );
    }

    #[test]
    fn set_root_label_then_read_root_label_round_trips() {
        // Plant a volume-label entry at the start of the root directory:
        // 11-byte name + attributes = VOLUME_ID (0x08) + 20 zero bytes.
        let mut bytes = create_fat32_image().into_inner();
        let dir = data_start_bytes();
        bytes[dir..dir + 11].copy_from_slice(b"OLD_LABEL  ");
        bytes[dir + 11] = 0x08; // VOLUME_ID

        // Open, set a new label, drop. Re-mount and assert the new label
        // round-tripped through both `read_root_label` and the byte-level
        // representation (so we know the write hit the disk).
        {
            let cursor = Cursor::new(&mut bytes[..]);
            let fs = FatFs::open(cursor).expect("open");
            fs.set_root_label(b"NEW_LABEL  ").expect("set_root_label");
        }
        // The label-name bytes were written verbatim.
        assert_eq!(&bytes[dir..dir + 11], b"NEW_LABEL  ");
        // And the label entry is found again after re-mount.
        let fs = FatFs::open(Cursor::new(&bytes[..])).expect("re-open");
        assert_eq!(
            fs.read_root_label().expect("read_root_label").unwrap(),
            *b"NEW_LABEL  "
        );
    }

    #[test]
    fn read_status_flags_dirty_bit_surfaces_when_cleared() {
        // Default FAT[1] is 0x0FFFFFFF (both status bits set = clean).
        // Clearing bit 27 plants "dirty".
        let mut bytes = create_fat32_image().into_inner();
        patch_fat1_both_copies(&mut bytes, 0x0FFFFFFFu32 & !0x0800_0000);

        let fs = FatFs::open(Cursor::new(bytes)).expect("open");
        let flags = fs.read_status_flags().expect("read_status_flags");
        assert!(
            flags.dirty,
            "dirty bit cleared on disk must surface as dirty=true"
        );
        assert!(!flags.io_errors);
    }

    #[test]
    fn read_status_flags_io_errors_bit_surfaces_when_cleared() {
        let mut bytes = create_fat32_image().into_inner();
        patch_fat1_both_copies(&mut bytes, 0x0FFFFFFFu32 & !0x0400_0000);

        let fs = FatFs::open(Cursor::new(bytes)).expect("open");
        let flags = fs.read_status_flags().expect("read_status_flags");
        assert!(!flags.dirty);
        assert!(
            flags.io_errors,
            "io_errors bit cleared on disk must surface as io_errors=true"
        );
    }

    #[test]
    fn read_status_flags_both_bits_cleared_reports_both() {
        let mut bytes = create_fat32_image().into_inner();
        patch_fat1_both_copies(&mut bytes, 0x0FFFFFFFu32 & !0x0C00_0000);

        let fs = FatFs::open(Cursor::new(bytes)).expect("open");
        let flags = fs.read_status_flags().expect("read_status_flags");
        assert!(flags.dirty && flags.io_errors);
    }

    /// `Cp437OemCpConverter` is a static. Pluggable through the builder.
    static CP437: Cp437OemCpConverter = Cp437OemCpConverter;

    #[test]
    fn cp437_oem_converter_encodes_latin_in_short_name() {
        // With the default `LossyAsciiOemCpConverter`, `é` would encode to
        // `_` (0x5F) in the 8.3 short name. With CP437 it encodes to 0x82,
        // round-tripping cleanly through DOS/Windows tools that interpret
        // short names in CP437.
        let mut bytes = create_fat32_image().into_inner();
        {
            let cursor = Cursor::new(&mut bytes[..]);
            let fs = FatFs::builder(cursor)
                .oem_converter(&CP437)
                .open()
                .expect("open");
            // The name needs LFN to round-trip the lowercase 'é'; the short
            // entry is uppercased + encoded via the configured converter.
            fs.create_file(&fs.root_dir(), "café.txt").expect("create");
        }

        // Walk root slots until the non-LFN short entry. Its name[3] should
        // be 0x82 (CP437 'É' uppercase), not 0x5F (lossy underscore).
        let mut found = false;
        for i in 0..16 {
            let pos = data_start_bytes() + i * 32;
            let attr = bytes[pos + 11];
            if bytes[pos] == 0x00 || bytes[pos] == 0xE5 {
                continue;
            }
            if attr == 0x0F {
                continue; // LFN
            }
            // Short entry. The short-name builder uppercases ASCII via
            // `to_ascii_uppercase` (no-op on non-ASCII), so 'é' (CP437
            // 0x82) survives in lowercase form. The point of this test is
            // that the *converter* mapped 'é' to the correct CP437 byte
            // (0x82) — not the lossy `_` (0x5F) the default converter
            // would have produced.
            let third_byte = bytes[pos + 3];
            assert_eq!(
                third_byte, 0x82,
                "with CP437 converter, name[3] should be CP437 'é' (0x82), got {third_byte:#04x}"
            );
            found = true;
            break;
        }
        assert!(found, "expected to find a short entry for café.txt");
    }
}

/// Tests for the IoContext error wrapper added in commit b4630d6 — the
/// boot-sector path is already covered by
/// `test_truncated_image_returns_boot_sector_context`. Here we exercise
/// the FSInfo path: a truncated FAT32 image with a valid boot sector but
/// missing FSInfo sector must surface as
/// `IoContext { op: "FSInfo", .. }`, not the bare `Io(...)` users got
/// before the wrapper existed.
#[cfg(feature = "std")]
mod iocontext_tests {
    use super::fat32_image::create_fat32_image;
    use hadris_fat::{FatError, FatFs};
    use std::io::Cursor;

    #[test]
    fn truncated_fsinfo_returns_iocontext_with_fsinfo_op() {
        // Build a normal image, then truncate so the boot sector reads
        // cleanly (it's the first 512 bytes) but reading the FSInfo at
        // sector 1 (bytes 512..1024) hits EOF.
        let bytes = create_fat32_image().into_inner();
        // Boot sector + a few more bytes — enough that reading struct fields
        // through the boot-sector path succeeds, but not enough for the
        // FSInfo sector.
        let truncated = &bytes[..512];
        let cursor = Cursor::new(truncated.to_vec());
        let err = FatFs::open(cursor).expect_err("must fail on missing FSInfo");
        match &err {
            FatError::IoContext { op, sector, .. } => {
                assert!(
                    op.contains("FSInfo"),
                    "expected op to mention 'FSInfo', got {op:?}"
                );
                // The fixture's FSInfo sector is sector 1.
                assert_eq!(*sector, Some(1));
            }
            other => panic!("expected IoContext, got {other:?}"),
        }
        // Display includes the operation label.
        assert!(format!("{err}").contains("FSInfo"));
    }
}

/// Tests for the optional `dirty-file-panic` feature added in commit
/// 3d637ee. When the feature is on, dropping a `FileWriter` without
/// calling `finish()` panics — the user's just-written size and timestamp
/// updates would otherwise be silently lost.
#[cfg(all(feature = "std", feature = "dirty-file-panic"))]
mod dirty_file_panic_tests {
    use super::fat32_image::create_fat32_image;
    use hadris_fat::{FatFs, FatFsWriteExt};
    use std::io::Cursor;

    #[test]
    #[should_panic]
    fn dropping_writer_without_finish_panics() {
        let bytes = create_fat32_image().into_inner();
        let fs = FatFs::open(Cursor::new(bytes)).expect("open");
        let entry = fs.create_file(&fs.root_dir(), "TEST.TXT").expect("create");
        let mut writer = fs.write_file(&entry).expect("writer");
        writer.write(b"oops").expect("write");
        // No finish(). Drop here must panic per the feature contract.
    }

    #[test]
    fn calling_finish_does_not_panic() {
        // Sanity: the panic path must NOT fire on the happy path.
        let bytes = create_fat32_image().into_inner();
        let fs = FatFs::open(Cursor::new(bytes)).expect("open");
        let entry = fs.create_file(&fs.root_dir(), "OK.TXT").expect("create");
        let mut writer = fs.write_file(&entry).expect("writer");
        writer.write(b"clean exit").expect("write");
        writer.finish().expect("finish");
    }
}

/// Regression tests for LFN runs spanning directory cluster boundaries.
#[cfg(all(feature = "std", feature = "lfn"))]
mod lfn_cluster_boundary_tests {
    use hadris_fat::format::{FatTypeSelection, FatVolumeFormatter, FormatOptions};
    use std::io::Cursor;

    /// FAT32 at 1 sector/cluster (512 B) → 16 directory entries per cluster.
    /// FAT32 needs ≥ 65525 clusters, i.e. ≥ ~33.5 MB of data region, so size
    /// the image at 48 MB. Returns the mounted, ready-to-use filesystem.
    fn fat32_spc1_fs() -> hadris_fat::FatFs<Cursor<Vec<u8>>> {
        let size = 48 * 1024 * 1024;
        let opts = FormatOptions::new(size as u64)
            .fat_type(FatTypeSelection::Fat32)
            .sectors_per_cluster(1)
            .volume_label("SPC1");
        FatVolumeFormatter::format(Cursor::new(vec![0u8; size]), opts).expect("format FAT32 spc=1")
    }

    #[test]
    fn long_name_exceeding_one_cluster_roundtrips_and_deletes() {
        let fs = fat32_spc1_fs();

        // 200 'A' + ".txt" = 204 UTF-16 units → 16 LFN entries + 1 short = 17
        // entries, one more than the 16 a 512 B cluster holds. The name is
        // well under the 255-unit spec cap, so it fails on geometry, not
        // validation.
        let long_name = format!("{}.txt", "A".repeat(200));

        let entry = fs
            .create_file(&fs.root_dir(), &long_name)
            .expect("a 17-entry run should span two clusters");
        assert!(
            fs.root_dir().find(&long_name).expect("find").is_some(),
            "cross-cluster name must round-trip"
        );
        fs.delete(&entry).expect("delete cross-cluster LFN run");
        fs.create_file(&fs.root_dir(), "OK.TXT")
            .expect("short-name create must succeed after deletion");

        let root = fs.root_dir();
        let mut names = Vec::new();
        let mut it = root.entries();
        while let Some(Ok(entry)) = it.next_entry() {
            // Short-name-only entries list as padded 8.3 (e.g. "OK      .TXT");
            // strip spaces so the assertion compares logical names.
            names.push(entry.name().replace(' ', ""));
        }
        assert!(
            names.iter().any(|n| n == "OK.TXT"),
            "OK.TXT should be listed (directory intact): {names:?}",
        );
        assert!(
            !names.iter().any(|n| n.starts_with("AAAA")),
            "the deleted long name must leave no visible entries behind: {names:?}",
        );
    }

    #[test]
    fn maximum_length_name_spans_clusters() {
        let fs = fat32_spc1_fs();
        let name: String = std::iter::repeat_n('m', 255).collect();

        let entry = fs
            .create_file(&fs.root_dir(), &name)
            .expect("255-unit LFN should use 20 LFN slots plus one short slot");
        assert!(
            fs.root_dir().find(&name).expect("find").is_some(),
            "maximum-length name must round-trip"
        );
        fs.delete(&entry).expect("delete maximum-length name");
        assert!(
            fs.root_dir()
                .find(&name)
                .expect("find after delete")
                .is_none(),
            "maximum-length name must be fully removed"
        );
    }

    #[test]
    fn create_directory_run_crosses_cluster_boundary() {
        let fs = fat32_spc1_fs();
        for i in 0..15 {
            fs.create_file(&fs.root_dir(), &format!("F{i:02}.TXT"))
                .expect("fill root slot");
        }

        let name = "Long Folder Name";
        fs.create_dir(&fs.root_dir(), name)
            .expect("directory LFN should cross into a new cluster");
        let entry = fs
            .root_dir()
            .find(name)
            .expect("find")
            .expect("directory entry");
        assert!(entry.is_directory());
        fs.delete(&entry).expect("delete cross-cluster directory");
        assert!(
            fs.root_dir()
                .find(name)
                .expect("find after delete")
                .is_none()
        );
    }

    #[test]
    fn rename_run_crosses_cluster_boundary() {
        let fs = fat32_spc1_fs();
        let source = fs
            .create_file(&fs.root_dir(), "SOURCE.TXT")
            .expect("create source");
        for i in 0..14 {
            fs.create_file(&fs.root_dir(), &format!("F{i:02}.TXT"))
                .expect("fill root slot");
        }

        let new_name = "Renamed Across Boundary.txt";
        let renamed = fs
            .rename(&source, &fs.root_dir(), new_name)
            .expect("rename LFN should cross into a new cluster");
        assert!(
            fs.root_dir()
                .find("SOURCE.TXT")
                .expect("find old")
                .is_none()
        );
        assert!(fs.root_dir().find(new_name).expect("find new").is_some());
        fs.delete(&renamed).expect("delete renamed entry");
    }
}
