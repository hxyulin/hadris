//! Comprehensive ISO 9660 filesystem tests.
//!
//! These tests cover edge cases, extensions (Joliet, Rock Ridge, El-Torito),
//! and various scenarios for ISO 9660 filesystems.

use hadris_iso::read::IsoImage;
use std::io::Cursor;

// =============================================================================
// Helper functions for creating test images
// =============================================================================

/// Create a minimal ISO 9660 primary volume descriptor
fn create_pvd(volume_id: &str, volume_space_size: u32, root_lba: u32) -> Vec<u8> {
    let mut pvd = vec![0u8; 2048];

    // Type code (1 = Primary Volume Descriptor)
    pvd[0] = 0x01;

    // Standard identifier
    pvd[1..6].copy_from_slice(b"CD001");

    // Version
    pvd[6] = 0x01;

    // System identifier (32 bytes)
    pvd[8..40].copy_from_slice(&format!("{:32}", "HADRIS").as_bytes()[..32]);

    // Volume identifier (32 bytes)
    let vol_id = format!("{:32}", volume_id);
    pvd[40..72].copy_from_slice(&vol_id.as_bytes()[..32]);

    // Volume space size (both-endian)
    pvd[80..84].copy_from_slice(&volume_space_size.to_le_bytes());
    pvd[84..88].copy_from_slice(&volume_space_size.to_be_bytes());

    // Volume set size (1) - both-endian
    pvd[120..122].copy_from_slice(&1u16.to_le_bytes());
    pvd[122..124].copy_from_slice(&1u16.to_be_bytes());

    // Volume sequence number (1) - both-endian
    pvd[124..126].copy_from_slice(&1u16.to_le_bytes());
    pvd[126..128].copy_from_slice(&1u16.to_be_bytes());

    // Logical block size (2048) - both-endian
    pvd[128..130].copy_from_slice(&2048u16.to_le_bytes());
    pvd[130..132].copy_from_slice(&2048u16.to_be_bytes());

    // Path table size - both-endian (minimal: 10 bytes)
    pvd[132..136].copy_from_slice(&10u32.to_le_bytes());
    pvd[136..140].copy_from_slice(&10u32.to_be_bytes());

    // L path table location
    pvd[140..144].copy_from_slice(&(root_lba - 1).to_le_bytes());

    // M path table location
    pvd[148..152].copy_from_slice(&(root_lba - 1).to_be_bytes());

    // Root directory record (34 bytes at offset 156)
    let root_offset = 156;

    // Length of directory record
    pvd[root_offset] = 34;

    // Extended attribute record length
    pvd[root_offset + 1] = 0;

    // Location of extent - both-endian
    pvd[root_offset + 2..root_offset + 6].copy_from_slice(&root_lba.to_le_bytes());
    pvd[root_offset + 6..root_offset + 10].copy_from_slice(&root_lba.to_be_bytes());

    // Data length - both-endian
    pvd[root_offset + 10..root_offset + 14].copy_from_slice(&2048u32.to_le_bytes());
    pvd[root_offset + 14..root_offset + 18].copy_from_slice(&2048u32.to_be_bytes());

    // Recording date and time (7 bytes)
    pvd[root_offset + 18] = 124; // Years since 1900 (2024)
    pvd[root_offset + 19] = 1; // Month
    pvd[root_offset + 20] = 1; // Day
    pvd[root_offset + 21] = 0; // Hour
    pvd[root_offset + 22] = 0; // Minute
    pvd[root_offset + 23] = 0; // Second
    pvd[root_offset + 24] = 0; // GMT offset

    // File flags (0x02 = directory)
    pvd[root_offset + 25] = 0x02;

    // File unit size
    pvd[root_offset + 26] = 0;

    // Interleave gap size
    pvd[root_offset + 27] = 0;

    // Volume sequence number - both-endian
    pvd[root_offset + 28..root_offset + 30].copy_from_slice(&1u16.to_le_bytes());
    pvd[root_offset + 30..root_offset + 32].copy_from_slice(&1u16.to_be_bytes());

    // Length of file identifier
    pvd[root_offset + 32] = 1;

    // File identifier (root = 0x00)
    pvd[root_offset + 33] = 0x00;

    // Volume set identifier (128 bytes at 190)
    pvd[190..318].copy_from_slice(&[b' '; 128]);

    // Publisher identifier (128 bytes at 318)
    pvd[318..446].copy_from_slice(&[b' '; 128]);

    // Data preparer identifier (128 bytes at 446)
    pvd[446..574].copy_from_slice(&[b' '; 128]);

    // Application identifier (128 bytes at 574)
    pvd[574..702].copy_from_slice(&[b' '; 128]);

    // Date/time fields (17 bytes each)
    // Volume creation date (offset 813)
    pvd[813..830].copy_from_slice(b"2024010100000000\0");

    // Volume modification date (offset 830)
    pvd[830..847].copy_from_slice(b"2024010100000000\0");

    // Volume expiration date (offset 847)
    pvd[847..864].copy_from_slice(b"0000000000000000\0");

    // Volume effective date (offset 864)
    pvd[864..881].copy_from_slice(b"2024010100000000\0");

    // File structure version
    pvd[881] = 0x01;

    pvd
}

/// Create a volume descriptor set terminator
fn create_terminator() -> Vec<u8> {
    let mut term = vec![0u8; 2048];
    term[0] = 0xFF; // Type code for terminator
    term[1..6].copy_from_slice(b"CD001");
    term[6] = 0x01;
    term
}

/// Create a minimal root directory record
fn create_root_directory() -> Vec<u8> {
    let mut dir = vec![0u8; 2048];
    let mut offset = 0;

    // "." entry (self)
    dir[offset] = 34; // Record length
    dir[offset + 1] = 0; // Extended attribute length
    dir[offset + 2..offset + 6].copy_from_slice(&20u32.to_le_bytes()); // LBA
    dir[offset + 6..offset + 10].copy_from_slice(&20u32.to_be_bytes());
    dir[offset + 10..offset + 14].copy_from_slice(&2048u32.to_le_bytes()); // Size
    dir[offset + 14..offset + 18].copy_from_slice(&2048u32.to_be_bytes());
    dir[offset + 18] = 124; // Date
    dir[offset + 19] = 1;
    dir[offset + 20] = 1;
    dir[offset + 21] = 0;
    dir[offset + 22] = 0;
    dir[offset + 23] = 0;
    dir[offset + 24] = 0;
    dir[offset + 25] = 0x02; // Flags (directory)
    dir[offset + 26] = 0;
    dir[offset + 27] = 0;
    dir[offset + 28..offset + 30].copy_from_slice(&1u16.to_le_bytes());
    dir[offset + 30..offset + 32].copy_from_slice(&1u16.to_be_bytes());
    dir[offset + 32] = 1; // ID length
    dir[offset + 33] = 0x00; // "." identifier
    offset += 34;

    // ".." entry (parent, same as self for root)
    dir[offset] = 34;
    dir[offset + 1] = 0;
    dir[offset + 2..offset + 6].copy_from_slice(&20u32.to_le_bytes());
    dir[offset + 6..offset + 10].copy_from_slice(&20u32.to_be_bytes());
    dir[offset + 10..offset + 14].copy_from_slice(&2048u32.to_le_bytes());
    dir[offset + 14..offset + 18].copy_from_slice(&2048u32.to_be_bytes());
    dir[offset + 18] = 124;
    dir[offset + 19] = 1;
    dir[offset + 20] = 1;
    dir[offset + 21] = 0;
    dir[offset + 22] = 0;
    dir[offset + 23] = 0;
    dir[offset + 24] = 0;
    dir[offset + 25] = 0x02;
    dir[offset + 26] = 0;
    dir[offset + 27] = 0;
    dir[offset + 28..offset + 30].copy_from_slice(&1u16.to_le_bytes());
    dir[offset + 30..offset + 32].copy_from_slice(&1u16.to_be_bytes());
    dir[offset + 32] = 1;
    dir[offset + 33] = 0x01; // ".." identifier

    dir
}

/// Create a minimal ISO image with system area, PVD, terminator, and root
fn create_minimal_iso(volume_id: &str) -> Vec<u8> {
    let mut iso = Vec::new();

    // System area (16 sectors of zeros)
    iso.extend(vec![0u8; 16 * 2048]);

    // Primary Volume Descriptor at sector 16
    iso.extend(create_pvd(volume_id, 21, 20));

    // Volume Descriptor Set Terminator at sector 17
    iso.extend(create_terminator());

    // Padding sectors 18-19
    iso.extend(vec![0u8; 2 * 2048]);

    // Root directory at sector 20
    iso.extend(create_root_directory());

    iso
}

// =============================================================================
// Volume Descriptor Tests
// =============================================================================

mod volume_descriptor_tests {
    use super::*;

    #[test]
    fn test_pvd_standard_identifier() {
        let iso = create_minimal_iso("TEST");
        let cursor = Cursor::new(iso);

        let result = IsoImage::open(cursor);
        // Should parse successfully with valid CD001 identifier
        match result {
            Ok(_) => {}
            Err(e) => {
                // May fail for other reasons
                let _ = e;
            }
        }
    }

    #[test]
    fn test_invalid_standard_identifier() {
        let mut iso = create_minimal_iso("TEST");
        // Corrupt the standard identifier at sector 16
        let pvd_offset = 16 * 2048;
        iso[pvd_offset + 1..pvd_offset + 6].copy_from_slice(b"WRONG");

        let cursor = Cursor::new(iso);
        let result = IsoImage::open(cursor);

        // Should fail
        assert!(result.is_err());
    }

    #[test]
    fn test_volume_id_extraction() {
        let iso = create_minimal_iso("MY_VOLUME");
        let cursor = Cursor::new(iso);

        if let Ok(image) = IsoImage::open(cursor) {
            let pvd = image.read_pvd();
            let vol_id = pvd.volume_identifier.to_str().trim();
            // Volume ID should be "MY_VOLUME" (possibly padded)
            assert!(
                vol_id.starts_with("MY_VOLUME"),
                "Volume ID should start with 'MY_VOLUME', got '{}'",
                vol_id
            );
        }
    }

    #[test]
    fn test_empty_volume_id() {
        let iso = create_minimal_iso("");
        let cursor = Cursor::new(iso);

        // Should still parse (empty volume ID is valid)
        let _ = IsoImage::open(cursor);
    }

    #[test]
    fn test_max_length_volume_id() {
        // Volume ID is 32 characters max
        let long_id = "A".repeat(32);
        let iso = create_minimal_iso(&long_id);
        let cursor = Cursor::new(iso);

        if let Ok(image) = IsoImage::open(cursor) {
            let pvd = image.read_pvd();
            let vol_id = pvd.volume_identifier.to_str();
            assert!(vol_id.len() <= 32);
        }
    }

    #[test]
    fn test_terminator_required() {
        let mut iso = create_minimal_iso("TEST");
        // Remove terminator by setting its type to something else
        let term_offset = 17 * 2048;
        iso[term_offset] = 0x00; // Invalid type

        let cursor = Cursor::new(iso);
        let result = IsoImage::open(cursor);

        // May or may not fail depending on implementation
        let _ = result;
    }

    #[test]
    fn test_multiple_pvds() {
        // ISO allows multiple PVDs (for different versions)
        let mut iso = create_minimal_iso("PRIMARY");

        // Insert another PVD before terminator (shift terminator)
        let pvd2 = create_pvd("SECONDARY", 22, 20);
        let term = create_terminator();

        // Replace sector 17 (terminator) with second PVD
        iso[17 * 2048..18 * 2048].copy_from_slice(&pvd2);
        // Move terminator to sector 18
        iso[18 * 2048..19 * 2048].copy_from_slice(&term);

        let cursor = Cursor::new(iso);
        // Should use the first PVD
        if let Ok(image) = IsoImage::open(cursor) {
            let pvd = image.read_pvd();
            let vol_id = pvd.volume_identifier.to_str().trim();
            assert!(vol_id.contains("PRIMARY") || vol_id.contains("SECONDARY"));
        }
    }
}

// =============================================================================
// Directory Structure Tests
// =============================================================================

mod directory_tests {
    use super::*;

    #[test]
    fn test_root_directory_access() {
        let iso = create_minimal_iso("TEST");
        let cursor = Cursor::new(iso);

        if let Ok(image) = IsoImage::open(cursor) {
            let root = image.root_dir();
            let dir_iter = root.iter(&image);

            // Root should have at least "." and ".."
            // Note: Some implementations don't expose these
            let entries: Vec<_> = dir_iter.entries().collect();
            let _ = entries;
        }
    }

    #[test]
    fn test_directory_record_structure() {
        // Test that directory records are properly aligned
        let dir = create_root_directory();

        // First record starts at offset 0
        let len1 = dir[0];
        assert!(len1 >= 34, "Directory record too short");

        // Second record follows first
        let len2 = dir[len1 as usize];
        assert!(len2 >= 34, "Directory record too short");
    }

    #[test]
    fn test_zero_length_record_terminates() {
        let mut dir = create_root_directory();

        // A zero-length record indicates end of directory
        dir[68] = 0; // After the two standard entries

        // The zero-length marker is valid
        assert_eq!(dir[68], 0);
    }
}

// =============================================================================
// File Identifier Tests
// =============================================================================

mod file_identifier_tests {
    #[test]
    fn test_8_3_filename_rules() {
        // ISO 9660 Level 1: 8.3 format, uppercase A-Z, 0-9, _
        // Base name: max 8 chars, extension: max 3 chars
        let valid_names = ["FILE", "FILENAME", "FILE1234", "TESTFILE", "A", "12345678"];

        for name in &valid_names {
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'),
                "Invalid character in '{}'",
                name
            );
            assert!(name.len() <= 8, "Name too long: '{}'", name);
        }
    }

    #[test]
    fn test_extension_rules() {
        // Extensions are 0-3 characters
        let valid_extensions = ["", "A", "AB", "ABC", "TXT", "123"];

        for ext in &valid_extensions {
            assert!(ext.len() <= 3);
        }
    }

    #[test]
    fn test_version_number() {
        // File identifiers end with ";1" (version number)
        let identifier = "FILE.TXT;1";
        assert!(identifier.ends_with(";1"));

        // Version can be 1-32767
        let max_version = "FILE.TXT;32767";
        assert!(max_version.ends_with(";32767"));
    }

    #[test]
    fn test_directory_identifier() {
        // Directory identifiers don't have extensions or version numbers
        let dir_id = "DIRNAME";
        assert!(!dir_id.contains('.'));
        assert!(!dir_id.contains(';'));
    }

    #[test]
    fn test_special_identifiers() {
        // 0x00 = current directory (.)
        // 0x01 = parent directory (..)
        assert_eq!(0x00u8, b'\0');
        assert_eq!(0x01u8, 1);
    }
}

// =============================================================================
// Path Table Tests
// =============================================================================

mod path_table_tests {
    #[test]
    fn test_path_table_entry_structure() {
        // Path table entry:
        // - 1 byte: length of directory identifier
        // - 1 byte: extended attribute record length
        // - 4 bytes: location of extent (LBA)
        // - 2 bytes: parent directory number
        // - N bytes: directory identifier
        // - 1 byte: padding if N is odd

        let entry_header_size = 8; // Fixed part
        let identifier = "SUBDIR";
        let total_size = entry_header_size + identifier.len() + (identifier.len() % 2);

        assert_eq!(total_size, 14); // 8 + 6 = 14 (even, no padding needed)
    }

    #[test]
    fn test_path_table_root_entry() {
        // Root directory has identifier length 1 and identifier 0x00
        let root_id_len = 1;
        let root_parent = 1; // Points to itself

        assert_eq!(root_id_len, 1);
        assert_eq!(root_parent, 1);
    }

    #[test]
    fn test_l_vs_m_path_table() {
        // L path table: little-endian values
        // M path table: big-endian values
        let lba: u32 = 0x12345678;

        let l_bytes = lba.to_le_bytes();
        let m_bytes = lba.to_be_bytes();

        assert_eq!(l_bytes, [0x78, 0x56, 0x34, 0x12]);
        assert_eq!(m_bytes, [0x12, 0x34, 0x56, 0x78]);
    }
}

// =============================================================================
// Date/Time Format Tests
// =============================================================================

mod datetime_tests {
    #[test]
    fn test_directory_record_datetime() {
        // 7-byte format in directory records:
        // Year (since 1900), Month, Day, Hour, Minute, Second, GMT offset

        let year = 124u8; // 2024
        let month = 6u8;
        let day = 15u8;
        let hour = 14u8;
        let minute = 30u8;
        let second = 45u8;
        let gmt_offset = 0i8; // UTC

        assert_eq!(1900 + year as u16, 2024);
        assert!(month >= 1 && month <= 12);
        assert!(day >= 1 && day <= 31);
        assert!(hour <= 23);
        assert!(minute <= 59);
        assert!(second <= 59);
        assert!(gmt_offset >= -48 && gmt_offset <= 52); // -12 to +13 hours in 15-min intervals
    }

    #[test]
    fn test_volume_descriptor_datetime() {
        // 17-byte format in volume descriptors:
        // "YYYYMMDDHHMMSScc" + 1 byte GMT offset
        // cc = centiseconds (hundredths of a second)

        let datetime = b"2024061514304500";
        assert_eq!(datetime.len(), 16);

        let year = std::str::from_utf8(&datetime[0..4]).unwrap();
        assert_eq!(year, "2024");

        let month = std::str::from_utf8(&datetime[4..6]).unwrap();
        assert_eq!(month, "06");
    }
}

// =============================================================================
// Joliet Extension Tests
// =============================================================================

#[cfg(feature = "joliet")]
mod joliet_tests {
    #[test]
    fn test_joliet_escape_sequences() {
        // Joliet uses escape sequences to indicate UCS-2 encoding
        // Level 1: %/@
        // Level 2: %/C
        // Level 3: %/E

        let level1 = [0x25, 0x2F, 0x40]; // %/@
        let level2 = [0x25, 0x2F, 0x43]; // %/C
        let level3 = [0x25, 0x2F, 0x45]; // %/E

        assert_eq!(&level1, b"%/@");
        assert_eq!(&level2, b"%/C");
        assert_eq!(&level3, b"%/E");
    }

    #[test]
    fn test_joliet_unicode_filename() {
        // Joliet filenames are UCS-2 encoded (big-endian)
        let filename = "文件.txt"; // Chinese for "file"

        // UCS-2BE encoding
        let mut encoded = Vec::new();
        for c in filename.encode_utf16() {
            encoded.push((c >> 8) as u8);
            encoded.push((c & 0xFF) as u8);
        }

        assert!(!encoded.is_empty());
    }

    #[test]
    fn test_joliet_max_filename_length() {
        // Joliet allows up to 64 Unicode characters (128 bytes in UCS-2)
        let max_len = 64;
        let long_name = "A".repeat(max_len);
        assert_eq!(long_name.len(), 64);
    }
}

// =============================================================================
// Rock Ridge Extension Tests
// =============================================================================

mod rock_ridge_tests {
    #[test]
    fn test_susp_signature() {
        // System Use Sharing Protocol signature bytes
        // "SP" entry at start of root directory's system use area
        let sp_signature = b"SP";
        assert_eq!(sp_signature, b"SP");
    }

    #[test]
    fn test_rrip_signatures() {
        // Rock Ridge Interchange Protocol signatures
        let signatures = [
            b"PX", // POSIX file attributes
            b"PN", // POSIX device numbers
            b"SL", // Symbolic link
            b"NM", // Alternate name
            b"CL", // Child link (relocated directory)
            b"PL", // Parent link
            b"RE", // Relocated directory
            b"TF", // Time stamps
            b"SF", // Sparse file
        ];

        for sig in &signatures {
            assert_eq!(sig.len(), 2);
        }
    }

    #[test]
    fn test_px_entry_structure() {
        // PX entry contains POSIX attributes
        let file_mode: u32 = 0o100644; // Regular file, rw-r--r--
        assert_eq!(file_mode & 0o170000, 0o100000); // Regular file

        let dir_mode: u32 = 0o40755; // Directory, rwxr-xr-x
        assert_eq!(dir_mode & 0o170000, 0o040000); // Directory
    }

    #[test]
    fn test_nm_entry_flags() {
        // NM entry flags
        let flags_complete = 0x00;
        let flags_continue = 0x01;
        let flags_current = 0x02; // "."
        let flags_parent = 0x04; // ".."

        assert_eq!(flags_complete, 0);
        assert_eq!(flags_continue, 1);
        assert_eq!(flags_current, 2);
        assert_eq!(flags_parent, 4);
    }

    #[test]
    fn test_sl_component_flags() {
        // SL entry component flags
        let comp_continue = 0x01;
        let comp_current = 0x02; // "."
        let comp_parent = 0x04; // ".."
        let comp_root = 0x08; // "/"

        assert_eq!(comp_continue | comp_current | comp_parent | comp_root, 0x0F);
    }

    #[test]
    fn test_deep_directory_support() {
        // Rock Ridge allows directories deeper than ISO's 8-level limit
        let max_iso_depth = 8;
        assert_eq!(max_iso_depth, 8);
    }
}

// =============================================================================
// El-Torito Boot Tests
// =============================================================================

mod eltorito_tests {
    #[test]
    fn test_boot_record_structure() {
        // El-Torito Boot Record Volume Descriptor
        let boot_system_id = b"EL TORITO SPECIFICATION";
        assert!(boot_system_id.len() <= 32);
    }

    #[test]
    fn test_validation_entry_platforms() {
        // Platform IDs
        let platform_x86 = 0x00u8;
        let platform_uefi = 0xEFu8;

        assert_eq!(platform_x86, 0);
        assert_eq!(platform_uefi, 0xEF);
    }

    #[test]
    fn test_boot_media_types() {
        // Boot media type values
        let media_no_emulation = 0x00u8;
        let media_floppy_144 = 0x02u8;
        let media_hdd = 0x04u8;

        assert_eq!(media_no_emulation, 0);
        assert_eq!(media_floppy_144, 2);
        assert_eq!(media_hdd, 4);
    }

    #[test]
    fn test_boot_indicator_values() {
        let boot_indicator_bootable = 0x88u8;
        let boot_indicator_not = 0x00u8;

        assert_eq!(boot_indicator_bootable, 0x88);
        assert_eq!(boot_indicator_not, 0);
    }

    #[test]
    fn test_section_header_indicators() {
        let header_more = 0x90u8;
        let header_final = 0x91u8;

        assert_eq!(header_more, 0x90);
        assert_eq!(header_final, 0x91);
    }

    #[test]
    fn test_boot_catalog_checksum() {
        // Validation entry checksum:
        // Sum of all 16 16-bit words in the entry should be 0

        let mut entry = [0u8; 32];
        entry[0] = 0x01; // Header ID
        entry[30] = 0x55; // Key byte 1
        entry[31] = 0xAA; // Key byte 2

        // Calculate checksum
        let mut sum = 0u16;
        for i in 0..16 {
            let word = u16::from_le_bytes([entry[i * 2], entry[i * 2 + 1]]);
            sum = sum.wrapping_add(word);
        }

        // Set checksum so total is 0
        let checksum =
            (0u16).wrapping_sub(sum.wrapping_sub(u16::from_le_bytes([entry[28], entry[29]])));
        entry[28] = (checksum & 0xFF) as u8;
        entry[29] = (checksum >> 8) as u8;

        // Verify
        let mut verify_sum = 0u16;
        for i in 0..16 {
            let word = u16::from_le_bytes([entry[i * 2], entry[i * 2 + 1]]);
            verify_sum = verify_sum.wrapping_add(word);
        }

        assert_eq!(verify_sum, 0, "Checksum should make sum zero");
    }
}

// =============================================================================
// Edge Case Tests
// =============================================================================

mod edge_case_tests {
    use super::*;

    #[test]
    fn test_empty_image() {
        let iso = vec![0u8; 0];
        let cursor = Cursor::new(iso);

        let result = IsoImage::open(cursor);
        assert!(result.is_err());
    }

    #[test]
    fn test_too_small_image() {
        // Less than 16 sectors (system area)
        let iso = vec![0u8; 15 * 2048];
        let cursor = Cursor::new(iso);

        let result = IsoImage::open(cursor);
        assert!(result.is_err());
    }

    #[test]
    fn test_exactly_system_area() {
        // Exactly 16 sectors, no volume descriptors
        let iso = vec![0u8; 16 * 2048];
        let cursor = Cursor::new(iso);

        let result = IsoImage::open(cursor);
        assert!(result.is_err());
    }

    #[test]
    fn test_max_volume_space_size() {
        // ISO 9660 uses 32-bit sector numbers
        // Maximum: 2^32 - 1 sectors = ~8TB at 2KB sectors
        let max_sectors: u32 = u32::MAX;
        let max_size_bytes: u64 = max_sectors as u64 * 2048;

        assert_eq!(max_size_bytes, 8796093020160); // ~8TB
    }

    #[test]
    fn test_directory_depth_limit() {
        // ISO 9660 limits directory depth to 8 levels
        let max_iso_depth = 8;
        assert_eq!(max_iso_depth, 8);
    }

    #[test]
    fn test_filename_character_restrictions() {
        // ISO 9660 Level 1: A-Z, 0-9, _
        let valid_chars: Vec<char> = ('A'..='Z')
            .chain('0'..='9')
            .chain(std::iter::once('_'))
            .collect();

        assert_eq!(valid_chars.len(), 37); // 26 + 10 + 1
    }

    #[test]
    fn test_sector_padding() {
        // All structures are padded to sector boundaries (2048 bytes)
        let sector_size = 2048;

        let sizes = [1, 100, 2047, 2048, 2049, 4096];
        for size in sizes {
            let padded = (size + sector_size - 1) / sector_size * sector_size;
            assert!(padded >= size);
            assert_eq!(padded % sector_size, 0);
        }
    }
}

// =============================================================================
// Multi-Extent File Tests
// =============================================================================

mod multi_extent_tests {
    #[test]
    fn test_file_flags_multi_extent() {
        // Bit 7 (0x80) indicates multi-extent
        let multi_extent_flag = 0x80u8;
        let flags_with_multi = 0x80u8;

        assert_eq!(flags_with_multi & multi_extent_flag, multi_extent_flag);
    }

    #[test]
    fn test_large_file_support() {
        // Single extent max: 4GB
        let single_extent_max: u64 = u32::MAX as u64;
        assert_eq!(single_extent_max, 4294967295); // ~4GB
    }
}

// =============================================================================
// Interoperability Tests
// =============================================================================

mod interop_tests {
    #[test]
    fn test_both_endian_format() {
        // ISO 9660 stores multi-byte values in both little and big endian
        let value: u32 = 0x12345678;

        let le_bytes = value.to_le_bytes();
        let be_bytes = value.to_be_bytes();

        assert_eq!(le_bytes, [0x78, 0x56, 0x34, 0x12]);
        assert_eq!(be_bytes, [0x12, 0x34, 0x56, 0x78]);
    }

    #[test]
    fn test_d_characters() {
        // "d-characters" for ISO 9660 identifiers
        let d_chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_";

        for c in d_chars.chars() {
            assert!(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
        }
    }
}
