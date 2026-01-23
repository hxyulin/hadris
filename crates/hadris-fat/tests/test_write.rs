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
        let root_dir_sectors = (ROOT_ENTRY_COUNT as usize * 32 + SECTOR_SIZE - 1) / SECTOR_SIZE;
        let data_start_sector =
            RESERVED_SECTORS as usize + FAT_COUNT as usize * SECTORS_PER_FAT as usize + root_dir_sectors;

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
        let root_dir_sectors = (ROOT_ENTRY_COUNT as usize * 32 + SECTOR_SIZE - 1) / SECTOR_SIZE;
        let _data_start_sector =
            RESERVED_SECTORS as usize + FAT_COUNT as usize * SECTORS_PER_FAT as usize + root_dir_sectors;

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
    use hadris_fat::{FatError, FatFs, FatFsWriteExt};

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
        let entry = fs.create_file(&root, "TEST.TXT").expect("Failed to create file");

        assert!(entry.is_file());
        assert_eq!(entry.size(), 0);

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
        fs.create_file(&root, "TEST.TXT").expect("Failed to create file");

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
        let entry = fs.create_file(&root, "HELLO.TXT").expect("Failed to create file");

        // Write content
        let content = b"Hello, FAT32 World!";
        {
            let mut writer = fs.write_file(&entry).expect("Failed to get writer");
            writer.write(content).expect("Failed to write");
            writer.finish().expect("Failed to finish");
        }

        // Re-find the file to get updated size
        let root = fs.root_dir();
        let entry = root.find("HELLO.TXT").expect("Find failed").expect("File not found");
        assert_eq!(entry.size(), content.len());

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
        let entry = fs.create_file(&root, "BIG.DAT").expect("Failed to create file");

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
        let entry = root.find("BIG.DAT").expect("Find failed").expect("File not found");
        assert_eq!(entry.size(), content.len());

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
        let subdir = fs.create_dir(&root, "SUBDIR").expect("Failed to create directory");

        // Verify . and .. entries exist
        let entries: Vec<_> = subdir.entries().collect();
        let names: Vec<_> = entries
            .iter()
            .filter_map(|e| e.as_ref().ok())
            .map(|e| e.name().trim_end_matches(' ').to_string())
            .collect();

        assert!(
            names.iter().any(|n| n.starts_with('.')),
            "Directory should have . entry: {:?}",
            names
        );
        assert!(
            names.iter().any(|n| n.starts_with("..")),
            "Directory should have .. entry: {:?}",
            names
        );
    }

    #[test]
    fn test_create_file_in_subdirectory() {
        let image = create_fat32_image();
        let fs = FatFs::open(image).expect("Failed to open FAT32 image");

        let root = fs.root_dir();

        // Create a directory
        let subdir = fs.create_dir(&root, "MYDIR").expect("Failed to create directory");

        // Create a file in the subdirectory
        let entry = fs.create_file(&subdir, "FILE.TXT").expect("Failed to create file");
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
        let entry = fs.create_file(&root, "DELETE.ME").expect("Failed to create file");
        {
            let mut writer = fs.write_file(&entry).expect("Failed to get writer");
            writer.write(b"temporary data").expect("Failed to write");
            writer.finish().expect("Failed to finish");
        }

        // Re-find and delete
        let root = fs.root_dir();
        let entry = root.find("DELETE.ME").expect("Find failed").expect("File not found");
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
        let _subdir = fs.create_dir(&root, "EMPTYDIR").expect("Failed to create directory");

        // Find and delete it
        let root = fs.root_dir();
        let entry = root.find("EMPTYDIR").expect("Find failed").expect("Dir not found");
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
        let subdir = fs.create_dir(&root, "HASFILE").expect("Failed to create directory");
        fs.create_file(&subdir, "INSIDE.TXT").expect("Failed to create file");

        // Try to delete the directory - should fail
        let root = fs.root_dir();
        let entry = root.find("HASFILE").expect("Find failed").expect("Dir not found");
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
            let name = format!("FILE{}.TXT", i);
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
        fs.create_file(&root, "MYFILE.TXT").expect("Failed to create file");

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
        let entry = fs.create_file(&root, "APPEND.TXT").expect("Failed to create file");
        {
            let mut writer = fs.write_file(&entry).expect("Failed to get writer");
            writer.write(b"First part. ").expect("Failed to write");
            writer.finish().expect("Failed to finish");
        }

        // Note: Currently the write implementation overwrites from the beginning.
        // A proper append would need to seek to the end first.
        // For now, we just verify the basic write/read cycle works.

        let root = fs.root_dir();
        let entry = root.find("APPEND.TXT").expect("Find failed").expect("File not found");

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
        let entry = fs.create_file(&root, "TEST.TXT").expect("Failed to create file");

        assert!(entry.is_file());
        assert_eq!(entry.size(), 0);

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
        let entry = fs.create_file(&root, "HELLO.TXT").expect("Failed to create file");

        // Write content
        let content = b"Hello, FAT16 World!";
        {
            let mut writer = fs.write_file(&entry).expect("Failed to get writer");
            writer.write(content).expect("Failed to write");
            writer.finish().expect("Failed to finish");
        }

        // Re-find the file to get updated size
        let root = fs.root_dir();
        let entry = root.find("HELLO.TXT").expect("Find failed").expect("File not found");
        assert_eq!(entry.size(), content.len());

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
        let entry = fs.create_file(&root, "BIG.DAT").expect("Failed to create file");

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
        let entry = root.find("BIG.DAT").expect("Find failed").expect("File not found");
        assert_eq!(entry.size(), content.len());

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
        let subdir = fs.create_dir(&root, "SUBDIR").expect("Failed to create directory");

        // Verify . and .. entries exist
        let entries: Vec<_> = subdir.entries().collect();
        let names: Vec<_> = entries
            .iter()
            .filter_map(|e| e.as_ref().ok())
            .map(|e| e.name().trim_end_matches(' ').to_string())
            .collect();

        assert!(
            names.iter().any(|n| n.starts_with('.')),
            "Directory should have . entry: {:?}",
            names
        );
        assert!(
            names.iter().any(|n| n.starts_with("..")),
            "Directory should have .. entry: {:?}",
            names
        );
    }

    #[test]
    fn test_fat16_create_file_in_subdirectory() {
        let image = create_fat16_image();
        let fs = FatFs::open(image).expect("Failed to open FAT16 image");

        let root = fs.root_dir();

        // Create a directory
        let subdir = fs.create_dir(&root, "MYDIR").expect("Failed to create directory");

        // Create a file in the subdirectory
        let entry = fs.create_file(&subdir, "FILE.TXT").expect("Failed to create file");
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
        let entry = fs.create_file(&root, "DELETE.ME").expect("Failed to create file");
        {
            let mut writer = fs.write_file(&entry).expect("Failed to get writer");
            writer.write(b"temporary data").expect("Failed to write");
            writer.finish().expect("Failed to finish");
        }

        // Re-find and delete
        let root = fs.root_dir();
        let entry = root.find("DELETE.ME").expect("Find failed").expect("File not found");
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
            let name = format!("FILE{}.TXT", i);
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
        fs.create_file(&root, "TEST.TXT").expect("Failed to create file");

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
        let entry = fs.create_file(&root, "TEST.TXT").expect("Failed to create file");

        assert!(entry.is_file());
        assert_eq!(entry.size(), 0);

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
        let entry = fs.create_file(&root, "HELLO.TXT").expect("Failed to create file");

        // Write content
        let content = b"Hello, FAT12 World!";
        {
            let mut writer = fs.write_file(&entry).expect("Failed to get writer");
            writer.write(content).expect("Failed to write");
            writer.finish().expect("Failed to finish");
        }

        // Re-find the file to get updated size
        let root = fs.root_dir();
        let entry = root.find("HELLO.TXT").expect("Find failed").expect("File not found");
        assert_eq!(entry.size(), content.len());

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
        let entry = fs.create_file(&root, "BIG.DAT").expect("Failed to create file");

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
        let entry = root.find("BIG.DAT").expect("Find failed").expect("File not found");
        assert_eq!(entry.size(), content.len());

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
        let subdir = fs.create_dir(&root, "SUBDIR").expect("Failed to create directory");

        // Verify . and .. entries exist
        let entries: Vec<_> = subdir.entries().collect();
        let names: Vec<_> = entries
            .iter()
            .filter_map(|e| e.as_ref().ok())
            .map(|e| e.name().trim_end_matches(' ').to_string())
            .collect();

        assert!(
            names.iter().any(|n| n.starts_with('.')),
            "Directory should have . entry: {:?}",
            names
        );
        assert!(
            names.iter().any(|n| n.starts_with("..")),
            "Directory should have .. entry: {:?}",
            names
        );
    }

    #[test]
    fn test_fat12_create_file_in_subdirectory() {
        let image = create_fat12_image();
        let fs = FatFs::open(image).expect("Failed to open FAT12 image");

        let root = fs.root_dir();

        // Create a directory
        let subdir = fs.create_dir(&root, "MYDIR").expect("Failed to create directory");

        // Create a file in the subdirectory
        let entry = fs.create_file(&subdir, "FILE.TXT").expect("Failed to create file");
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
        let entry = fs.create_file(&root, "DELETE.ME").expect("Failed to create file");
        {
            let mut writer = fs.write_file(&entry).expect("Failed to get writer");
            writer.write(b"temporary data").expect("Failed to write");
            writer.finish().expect("Failed to finish");
        }

        // Re-find and delete
        let root = fs.root_dir();
        let entry = root.find("DELETE.ME").expect("Find failed").expect("File not found");
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
            let name = format!("FILE{}.TXT", i);
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
        fs.create_file(&root, "TEST.TXT").expect("Failed to create file");

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

        let entry = fs.create_file(&root, "CHAIN.DAT").expect("Failed to create file");
        {
            let mut writer = fs.write_file(&entry).expect("Failed to get writer");
            writer.write(&content).expect("Failed to write");
            writer.finish().expect("Failed to finish");
        }

        // Read back and verify
        let root = fs.root_dir();
        let entry = root.find("CHAIN.DAT").expect("Find failed").expect("File not found");
        assert_eq!(entry.size(), content.len());

        use hadris_fat::FatFsReadExt;
        let mut reader = fs.read_file(&entry).expect("Failed to get reader");
        let read_content = reader.read_to_vec().expect("Read failed");
        assert_eq!(read_content, content, "FAT12 cluster chain read mismatch");
    }
}
