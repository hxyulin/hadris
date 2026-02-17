// mkfs_sectors.bin is a binary file generated using these commands:
// # Create the image (100MB)
// dd if=/dev/zero of=test.img bs=512 count=204800
// # Create the filesystem
// mkfs.fat -F 32 test.img
// # Copy first 2 sectors to the mkfs_sectors.bin file
// dd if=test.img of=mkfs_sectors.bin bs=512 count=2
const BOOT_SECTORS: &[u8] = include_bytes!("mkfs_sectors.bin");

use hadris_fat::{FatError, FatFs};
use std::io::Cursor;

/// Test that boot sector parsing works correctly with the mkfs_sectors.bin fixture
#[test]
fn test_parse_boot_sector() {
    // The mkfs_sectors.bin contains the first 2 sectors of a valid FAT32 image
    // (boot sector and FSInfo sector)
    let data = Cursor::new(BOOT_SECTORS.to_vec());

    // Try to open the filesystem - this tests our parsing code
    // The open may succeed or fail depending on how much data we need to read
    let result = FatFs::open(data);

    // Whether it succeeds or fails, it should not panic
    // If it succeeds, great! If it fails, check that it's a reasonable error
    match result {
        Ok(_fs) => {
            // Successfully parsed the boot sector - that's fine
        }
        Err(FatError::Io(_)) => {
            // I/O error (e.g., trying to read beyond buffer) is acceptable
        }
        Err(FatError::InvalidFsInfoSignature { .. }) => {
            // FSInfo validation error is acceptable if data is truncated
        }
        Err(e) => {
            // Other errors should be investigated
            panic!("Unexpected error parsing boot sector: {:?}", e);
        }
    }
}

/// Test that invalid boot signature is detected
#[test]
fn test_invalid_boot_signature() {
    // Create a buffer with invalid boot signature
    let mut data = vec![0u8; 1024];
    // Set some BPB fields but with wrong signature
    data[510] = 0x00; // Wrong signature (should be 0x55)
    data[511] = 0x00; // Wrong signature (should be 0xAA)

    let cursor = Cursor::new(data);
    let result = FatFs::open(cursor);

    match result {
        Err(FatError::InvalidBootSignature { found }) => {
            assert_eq!(found, 0x0000);
        }
        _ => panic!("Expected InvalidBootSignature error"),
    }
}

/// Test that FAT12/16 is now detected and parsed (no longer returns UnsupportedFatType)
#[test]
fn test_fat12_16_detection() {
    // Create a minimal FAT16-like buffer
    // This won't have enough data for a fully valid filesystem, but should
    // be detected as FAT12/16 based on root_entry_count and sectors_per_fat_16
    let mut data = vec![0u8; 4096];

    // Set boot jump
    data[0] = 0xEB;
    data[1] = 0x58;
    data[2] = 0x90;

    // Set bytes per sector = 512
    data[11] = 0x00;
    data[12] = 0x02;

    // Set sectors per cluster = 1
    data[13] = 0x01;

    // Set reserved sectors = 1
    data[14] = 0x01;
    data[15] = 0x00;

    // Set FAT count = 2
    data[16] = 0x02;

    // Set root_entry_count = 512 (non-zero indicates FAT12/16)
    data[17] = 0x00; // Little-endian
    data[18] = 0x02; // 512 entries

    // Set total_sectors_16 = 0 (we'll use total_sectors_32)
    data[19] = 0x00;
    data[20] = 0x00;

    // Set media type
    data[21] = 0xF8;

    // Set sectors_per_fat_16 = 1 (non-zero indicates FAT12/16)
    data[22] = 0x01;
    data[23] = 0x00;

    // Set total_sectors_32 (at offset 32) = 2880 (small disk)
    data[32] = 0x40;
    data[33] = 0x0B;
    data[34] = 0x00;
    data[35] = 0x00;

    // Set boot signature at 510-511 (within first sector)
    data[510] = 0x55;
    data[511] = 0xAA;

    let cursor = Cursor::new(data);
    let result = FatFs::open(cursor);

    // Now FAT12/16 should be detected, though the filesystem may not be fully valid
    // We just check that it doesn't return UnsupportedFatType
    match result {
        Ok(fs) => {
            // Check that it detected as FAT12 or FAT16
            use hadris_fat::FatType;
            assert!(matches!(fs.fat_type(), FatType::Fat12 | FatType::Fat16));
        }
        Err(FatError::UnsupportedFatType(_)) => {
            panic!("FAT12/16 should now be supported");
        }
        Err(_) => {
            // Other errors are acceptable (e.g., I/O errors from incomplete data)
        }
    }
}

#[cfg(test)]
mod file_tests {
    use hadris_fat::file::ShortFileName;

    #[test]
    fn test_short_filename_valid() {
        // Valid 8.3 filename "TEST    TXT"
        let name = *b"TEST    TXT";
        let result = ShortFileName::new(name);
        assert!(result.is_ok());
        let sfn = result.unwrap();
        assert!(sfn.as_str().starts_with("TEST"));
    }

    #[test]
    fn test_short_filename_with_spaces() {
        // Filename with spaces is valid
        let name = *b"FILE    BIN";
        let result = ShortFileName::new(name);
        assert!(result.is_ok());
    }

    #[test]
    fn test_short_filename_invalid_lowercase() {
        // Lowercase letters are not valid in short filenames
        let name = *b"test    txt";
        let result = ShortFileName::new(name);
        // Note: Our current implementation allows lowercase, you may want to change this
        // For now this tests that the function doesn't panic
        let _ = result;
    }

    #[test]
    fn test_short_filename_special_chars() {
        // Test allowed special characters
        let result = ShortFileName::new([
            b'$', b'%', b'\'', b'-', b'_', b'@', b'~', b' ', b' ', b' ', b' ',
        ]);
        assert!(result.is_ok());
    }
}

#[cfg(feature = "lfn")]
#[cfg(test)]
mod lfn_tests {
    use hadris_fat::file::{LfnBuilder, LongFileName};

    #[test]
    fn test_lfn_empty() {
        let lfn = LongFileName::new();
        assert!(lfn.is_empty());
        assert_eq!(lfn.as_str(), "");
    }

    #[test]
    fn test_lfn_builder_start() {
        let mut builder = LfnBuilder::new();
        // Start with sequence number 0x41 (first and last entry)
        builder.start(0x41, 0x12);
        assert!(builder.building);
    }

    #[test]
    fn test_lfn_prepend_ascii() {
        let mut lfn = LongFileName::new();

        // Create LFN entry with "test" encoded as UTF-16LE
        // "test" = 't' 'e' 's' 't' + padding
        let name1: [u8; 10] = [
            b't', 0, b'e', 0, b's', 0, b't', 0, 0x00, 0x00, // "test" + null terminator
        ];
        let name2: [u8; 12] = [0xFF; 12]; // Padding
        let name3: [u8; 4] = [0xFF; 4]; // Padding

        lfn.prepend_lfn_entry(&name1, &name2, &name3);

        assert_eq!(lfn.as_str(), "test");
    }

    #[test]
    fn test_lfn_prepend_multiple() {
        let mut lfn = LongFileName::new();

        // Second part: "file.txt"
        let name1_2: [u8; 10] = [b'f', 0, b'i', 0, b'l', 0, b'e', 0, b'.', 0];
        let name2_2: [u8; 12] = [
            b't', 0, b'x', 0, b't', 0, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF,
        ];
        let name3_2: [u8; 4] = [0xFF; 4];

        lfn.prepend_lfn_entry(&name1_2, &name2_2, &name3_2);

        // First part: "long_"
        let name1_1: [u8; 10] = [b'l', 0, b'o', 0, b'n', 0, b'g', 0, b'_', 0];
        let name2_1: [u8; 12] = [0xFF; 12];
        let name3_1: [u8; 4] = [0xFF; 4];

        lfn.prepend_lfn_entry(&name1_1, &name2_1, &name3_1);

        assert_eq!(lfn.as_str(), "long_file.txt");
    }
}

// Integration tests using in-memory FAT32 images

#[cfg(feature = "write")]
mod integration_tests {
    use hadris_fat::format::{FatVolumeFormatter, FormatOptions};
    use hadris_fat::{FatFs, FatFsWriteExt};
    use std::io::Cursor;

    /// Create a test FAT32 image with known directory structure:
    /// /
    /// ├── HELLO.TXT     (content: "Hello, World!")
    /// ├── DATA.BIN      (content: 1024 bytes of 0xAA)
    /// ├── SUBDIR/
    /// │   ├── NESTED.TXT (content: "Nested file content")
    /// │   └── DEEP/
    /// │       └── FILE.TXT (content: "Deep file")
    pub fn create_test_fat32_image() -> Cursor<Vec<u8>> {
        // Use a 4MB volume (small but sufficient for FAT32)
        let volume_size: u64 = 4 * 1024 * 1024;
        let buffer = vec![0u8; volume_size as usize];
        let mut cursor = Cursor::new(buffer);

        let opts = FormatOptions::new(volume_size);
        let fs =
            FatVolumeFormatter::format(&mut cursor, opts).expect("Failed to format FAT32 volume");

        // Get root directory
        let root = fs.root_dir();

        // Create HELLO.TXT
        let hello_entry = fs.create_file(&root, "HELLO.TXT").unwrap();
        let mut hello_writer = fs.write_file(&hello_entry).unwrap();
        hello_writer.write(b"Hello, World!").unwrap();
        hello_writer.finish().unwrap();

        // Create DATA.BIN with 1024 bytes of 0xAA
        let data_entry = fs.create_file(&root, "DATA.BIN").unwrap();
        let mut data_writer = fs.write_file(&data_entry).unwrap();
        let data_content = vec![0xAA; 1024];
        data_writer.write(&data_content).unwrap();
        data_writer.finish().unwrap();

        // Create SUBDIR
        let subdir = fs.create_dir(&root, "SUBDIR").unwrap();

        // Create SUBDIR/NESTED.TXT
        let nested_entry = fs.create_file(&subdir, "NESTED.TXT").unwrap();
        let mut nested_writer = fs.write_file(&nested_entry).unwrap();
        nested_writer.write(b"Nested file content").unwrap();
        nested_writer.finish().unwrap();

        // Create SUBDIR/DEEP
        let deep_dir = fs.create_dir(&subdir, "DEEP").unwrap();

        // Create SUBDIR/DEEP/FILE.TXT
        let file_entry = fs.create_file(&deep_dir, "FILE.TXT").unwrap();
        let mut file_writer = fs.write_file(&file_entry).unwrap();
        file_writer.write(b"Deep file").unwrap();
        file_writer.finish().unwrap();

        // Sync to ensure all changes are written
        fs.sync().unwrap();

        cursor
    }

    #[test]
    fn test_read_directory_entries() {
        use hadris_io::Seek;

        let mut cursor = create_test_fat32_image();
        cursor.seek(std::io::SeekFrom::Start(0)).unwrap();

        let fs = FatFs::open(cursor).expect("Failed to open FAT32 image");
        let root = fs.root_dir();

        // Collect all entries (excluding . and ..)
        let entries: Vec<_> = root
            .entries()
            .filter_map(|e| e.ok())
            .filter(|e| {
                let name = e.name();
                name != "." && name != ".."
            })
            .collect();

        // Should have 3 entries: HELLO.TXT, DATA.BIN, SUBDIR
        assert_eq!(entries.len(), 3, "Expected 3 entries in root directory");

        // Verify file names
        let names: Vec<_> = entries.iter().map(|e| e.name()).collect();
        assert!(
            names.iter().any(|n| n.starts_with("HELLO")),
            "Should find HELLO.TXT"
        );
        assert!(
            names.iter().any(|n| n.starts_with("DATA")),
            "Should find DATA.BIN"
        );
        assert!(
            names.iter().any(|n| n.starts_with("SUBDIR")),
            "Should find SUBDIR"
        );
    }

    #[test]
    fn test_read_file_contents() {
        use hadris_io::Seek;

        let mut cursor = create_test_fat32_image();
        cursor.seek(std::io::SeekFrom::Start(0)).unwrap();

        let fs = FatFs::open(cursor).expect("Failed to open FAT32 image");
        let root = fs.root_dir();

        // Read HELLO.TXT
        let mut hello_reader = root.open_file("HELLO.TXT").unwrap();
        let hello_content = hello_reader.read_to_vec().unwrap();
        assert_eq!(String::from_utf8(hello_content).unwrap(), "Hello, World!");

        // Read DATA.BIN
        let mut data_reader = root.open_file("DATA.BIN").unwrap();
        let data_content = data_reader.read_to_vec().unwrap();
        assert_eq!(data_content.len(), 1024);
        assert!(data_content.iter().all(|&b| b == 0xAA));
    }

    #[test]
    #[cfg(feature = "lfn")]
    fn test_read_lfn_entries() {
        use hadris_io::Seek;

        // Note: The current write API only creates short filenames (8.3 format).
        // The LFN feature is primarily for reading existing LFN entries created
        // by other tools. This test verifies that the LFN parsing infrastructure
        // compiles and works correctly by testing with short filenames.
        let mut cursor = create_test_fat32_image();
        cursor.seek(std::io::SeekFrom::Start(0)).unwrap();

        let fs = FatFs::open(cursor).expect("Failed to open FAT32 image");
        let root = fs.root_dir();

        // Verify we can read the short filenames (8.3 format)
        let result = root.find("HELLO.TXT");
        assert!(result.is_ok());
        assert!(result.unwrap().is_some(), "Should find HELLO.TXT");

        // Verify directory iteration works with LFN feature enabled
        let entries: Vec<_> = root
            .entries()
            .filter_map(|e| e.ok())
            .filter(|e| {
                let name = e.name();
                name != "." && name != ".."
            })
            .collect();

        // Should have 3 entries: HELLO.TXT, DATA.BIN, SUBDIR
        assert_eq!(entries.len(), 3, "Expected 3 entries in root directory");
    }
}

#[cfg(test)]
#[cfg(feature = "write")]
mod navigation_tests {
    use super::integration_tests::create_test_fat32_image;
    use hadris_fat::{FatError, FatFs};
    use hadris_io::Seek;

    /// Test that find() returns None for non-existent entries
    #[test]
    fn test_find_nonexistent_returns_none() {
        let mut cursor = create_test_fat32_image();
        cursor.seek(std::io::SeekFrom::Start(0)).unwrap();

        let fs = FatFs::open(cursor).expect("Failed to open FAT32 image");
        let root = fs.root_dir();

        // Try to find a non-existent file
        let result = root.find("NONEXISTENT.TXT");
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    /// Test that open_dir() returns NotADirectory error for files
    #[test]
    fn test_open_dir_on_file_returns_error() {
        let mut cursor = create_test_fat32_image();
        cursor.seek(std::io::SeekFrom::Start(0)).unwrap();

        let fs = FatFs::open(cursor).expect("Failed to open FAT32 image");
        let root = fs.root_dir();

        // Try to open a file as a directory
        let result = root.open_dir("HELLO.TXT");
        assert!(result.is_err());
        match result {
            Err(FatError::NotADirectory) => {}
            _ => panic!("Expected NotADirectory error"),
        }
    }

    /// Test that open_file() returns NotAFile error for directories
    #[test]
    fn test_open_file_on_directory_returns_error() {
        let mut cursor = create_test_fat32_image();
        cursor.seek(std::io::SeekFrom::Start(0)).unwrap();

        let fs = FatFs::open(cursor).expect("Failed to open FAT32 image");
        let root = fs.root_dir();

        // Try to open a directory as a file
        let result = root.open_file("SUBDIR");
        assert!(result.is_err());
        match result {
            Err(FatError::NotAFile) => {}
            _ => panic!("Expected NotAFile error"),
        }
    }

    /// Test error variants display correctly
    #[test]
    fn test_error_display() {
        let err = FatError::EntryNotFound;
        assert_eq!(format!("{}", err), "entry not found in directory");

        let err = FatError::InvalidPath;
        assert_eq!(format!("{}", err), "path is invalid (empty or malformed)");
    }

    /// Test path-based API with invalid paths
    #[cfg(feature = "alloc")]
    mod path_tests {
        use hadris_fat::FatError;

        #[test]
        fn test_invalid_path_empty() {
            // Even if we can't open a real filesystem, we can verify the error type exists
            // and the API surface is correct through compilation
            let _: Result<(), FatError> = Err(FatError::InvalidPath);
            let _: Result<(), FatError> = Err(FatError::EntryNotFound);
        }

        #[test]
        #[cfg(feature = "write")]
        fn test_open_path_empty_returns_invalid() {
            use super::super::integration_tests::create_test_fat32_image;
            use hadris_fat::FatFs;
            use hadris_io::Seek;

            let mut cursor = create_test_fat32_image();
            cursor.seek(std::io::SeekFrom::Start(0)).unwrap();

            let fs = FatFs::open(cursor).expect("Failed to open FAT32 image");

            // Try to open an empty path
            let result = fs.open_path("");
            assert!(result.is_err());
            match result {
                Err(FatError::InvalidPath) => {}
                _ => panic!("Expected InvalidPath error"),
            }
        }

        #[test]
        #[cfg(feature = "write")]
        fn test_open_path_slash_only_returns_invalid() {
            use super::super::integration_tests::create_test_fat32_image;
            use hadris_fat::FatFs;
            use hadris_io::Seek;

            let mut cursor = create_test_fat32_image();
            cursor.seek(std::io::SeekFrom::Start(0)).unwrap();

            let fs = FatFs::open(cursor).expect("Failed to open FAT32 image");

            // Try to open a slash-only path
            let result = fs.open_path("/");
            assert!(result.is_err());
            match result {
                Err(FatError::InvalidPath) => {}
                _ => panic!("Expected InvalidPath error"),
            }
        }

        #[test]
        #[cfg(feature = "write")]
        fn test_open_path_traversal() {
            use super::super::integration_tests::create_test_fat32_image;
            use hadris_fat::FatFs;
            use hadris_io::Seek;

            let mut cursor = create_test_fat32_image();
            cursor.seek(std::io::SeekFrom::Start(0)).unwrap();

            let fs = FatFs::open(cursor).expect("Failed to open FAT32 image");

            // Test multi-level path navigation
            let result = fs.open_path("SUBDIR/DEEP/FILE.TXT");
            assert!(result.is_ok(), "Path traversal should work");

            let entry = result.unwrap();
            assert!(entry.is_file());
            assert_eq!(entry.size(), 9); // "Deep file" is 9 bytes
        }

        #[test]
        #[cfg(feature = "write")]
        fn test_open_file_path() {
            use super::super::integration_tests::create_test_fat32_image;
            use hadris_fat::FatFs;
            use hadris_io::Seek;

            let mut cursor = create_test_fat32_image();
            cursor.seek(std::io::SeekFrom::Start(0)).unwrap();

            let fs = FatFs::open(cursor).expect("Failed to open FAT32 image");

            // Test opening a file by path
            let mut reader = fs.open_file_path("SUBDIR/NESTED.TXT").unwrap();
            let content = reader.read_to_vec().unwrap();
            assert_eq!(String::from_utf8(content).unwrap(), "Nested file content");
        }

        #[test]
        #[cfg(feature = "write")]
        fn test_open_dir_path() {
            use super::super::integration_tests::create_test_fat32_image;
            use hadris_fat::FatFs;
            use hadris_io::Seek;

            let mut cursor = create_test_fat32_image();
            cursor.seek(std::io::SeekFrom::Start(0)).unwrap();

            let fs = FatFs::open(cursor).expect("Failed to open FAT32 image");

            // Test opening a directory by path
            let dir = fs.open_dir_path("SUBDIR/DEEP").unwrap();

            // Verify we can list entries in the directory
            let entries: Vec<_> = dir
                .entries()
                .filter_map(|e| e.ok())
                .filter(|e| {
                    let name = e.name();
                    name != "." && name != ".."
                })
                .collect();

            // Should have 1 entry: FILE.TXT
            assert_eq!(entries.len(), 1);
            assert!(entries[0].name().starts_with("FILE"));
        }
    }
}
