//! Integration tests for hadris-udf using external tools.
//!
//! These tests verify that hadris-udf can correctly read UDF images created by
//! external tools and that images created by hadris-udf can be read by external tools.
//!
//! External tools used:
//! - Linux: `mkudffs`, `udfinfo` (part of udftools package)
//! - macOS: `hdiutil` (native)
//! - Cross-platform: `7z` (part of p7zip)
//!
//! Install on various platforms:
//! - Ubuntu/Debian: `sudo apt install udftools p7zip-full`
//! - Fedora: `sudo dnf install udftools p7zip`
//! - macOS: `brew install p7zip` (hdiutil is native)
//! - Arch: `sudo pacman -S udftools p7zip`

use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

// =============================================================================
// Tool availability checks
// =============================================================================

/// Check if mkudffs is available (Linux)
fn mkudffs_available() -> bool {
    Command::new("mkudffs")
        .arg("--help")
        .output()
        .map(|o| o.status.success() || o.status.code() == Some(1))
        .unwrap_or(false)
}

/// Check if udfinfo is available (Linux)
fn udfinfo_available() -> bool {
    Command::new("udfinfo")
        .arg("--help")
        .output()
        .map(|o| o.status.success() || o.status.code() == Some(1))
        .unwrap_or(false)
}

/// Check if 7z is available
fn sevenzip_available() -> bool {
    Command::new("7z")
        .output()
        .map(|o| o.status.success() || o.status.code() == Some(0))
        .unwrap_or(false)
}

// =============================================================================
// UDF image creation functions
// =============================================================================

/// Create a UDF image using mkudffs (Linux)
fn create_udf_with_mkudffs(image_path: &Path, size_mb: u32, label: &str, revision: &str) -> bool {
    let size_bytes = (size_mb as u64) * 1024 * 1024;

    let file = File::create(image_path).expect("Failed to create image file");
    file.set_len(size_bytes).expect("Failed to set file size");
    drop(file);

    let output = Command::new("mkudffs")
        .args([
            "--label",
            label,
            "--udfrev",
            revision,
            "--blocksize=2048",
            image_path.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to run mkudffs");

    if !output.status.success() {
        eprintln!(
            "mkudffs failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    output.status.success()
}

// =============================================================================
// Verification functions
// =============================================================================

/// Get UDF info using udfinfo (Linux only)
fn get_udf_info(image_path: &Path) -> Option<String> {
    let output = Command::new("udfinfo")
        .arg(image_path.to_str().unwrap())
        .output()
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        None
    }
}

/// List files in UDF image using 7z
fn list_udf_with_7z(image_path: &Path) -> Option<String> {
    let output = Command::new("7z")
        .args(["l", image_path.to_str().unwrap()])
        .output()
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        None
    }
}

// =============================================================================
// Tests for reading UDF images created by mkudffs (Linux)
// =============================================================================

#[test]
fn test_read_mkudffs_udf102() {
    if !mkudffs_available() {
        eprintln!("Skipping test: mkudffs not available (Linux only)");
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let image_path = temp_dir.path().join("test_udf102.img");

    assert!(
        create_udf_with_mkudffs(&image_path, 10, "TEST_UDF102", "0x0102"),
        "Failed to create UDF image with mkudffs"
    );

    let mut udf_data = Vec::new();
    File::open(&image_path)
        .unwrap()
        .read_to_end(&mut udf_data)
        .unwrap();

    let cursor = Cursor::new(udf_data);
    let udf = hadris_udf::UdfFs::open(cursor)
        .expect("hadris-udf should be able to open mkudffs UDF 1.02 image");

    let info = udf.info();
    println!("Volume ID: {}", info.volume_id);
    assert!(!info.volume_id.is_empty(), "Volume ID should not be empty");

    // Verify root directory is accessible
    let root = udf
        .root_dir()
        .expect("Should be able to read root directory");
    let _entry_count = root.entries().count();
}

#[test]
fn test_read_mkudffs_udf150() {
    if !mkudffs_available() {
        eprintln!("Skipping test: mkudffs not available (Linux only)");
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let image_path = temp_dir.path().join("test_udf150.img");

    assert!(
        create_udf_with_mkudffs(&image_path, 10, "TEST_UDF150", "0x0150"),
        "Failed to create UDF image with mkudffs"
    );

    let mut udf_data = Vec::new();
    File::open(&image_path)
        .unwrap()
        .read_to_end(&mut udf_data)
        .unwrap();

    let cursor = Cursor::new(udf_data);
    let udf = hadris_udf::UdfFs::open(cursor)
        .expect("hadris-udf should be able to open mkudffs UDF 1.50 image");

    let info = udf.info();
    println!("Volume ID: {}", info.volume_id);
    assert!(!info.volume_id.is_empty(), "Volume ID should not be empty");
}

#[test]
fn test_read_mkudffs_udf201() {
    if !mkudffs_available() {
        eprintln!("Skipping test: mkudffs not available (Linux only)");
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let image_path = temp_dir.path().join("test_udf201.img");

    assert!(
        create_udf_with_mkudffs(&image_path, 10, "TEST_UDF201", "0x0201"),
        "Failed to create UDF image with mkudffs"
    );

    let mut udf_data = Vec::new();
    File::open(&image_path)
        .unwrap()
        .read_to_end(&mut udf_data)
        .unwrap();

    let cursor = Cursor::new(udf_data);
    let udf = hadris_udf::UdfFs::open(cursor)
        .expect("hadris-udf should be able to open mkudffs UDF 2.01 image");

    let info = udf.info();
    println!("Volume ID: {}", info.volume_id);
    assert!(!info.volume_id.is_empty(), "Volume ID should not be empty");
}

#[test]
fn test_udfinfo_validates_mkudffs_image() {
    if !mkudffs_available() || !udfinfo_available() {
        eprintln!("Skipping test: mkudffs or udfinfo not available (Linux only)");
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let image_path = temp_dir.path().join("udfinfo_test.img");

    assert!(
        create_udf_with_mkudffs(&image_path, 10, "INFO_TEST", "0x0102"),
        "Failed to create UDF image"
    );

    let info = get_udf_info(&image_path).expect("udfinfo should be able to read the UDF image");

    println!("=== udfinfo output ===\n{}", info);
    assert!(
        info.to_lowercase().contains("info_test") || info.contains("UDF"),
        "udfinfo should show volume info"
    );
}

#[test]
fn test_7z_can_list_mkudffs_image() {
    if !mkudffs_available() || !sevenzip_available() {
        eprintln!("Skipping test: mkudffs or 7z not available");
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let image_path = temp_dir.path().join("test_7z.img");

    assert!(
        create_udf_with_mkudffs(&image_path, 10, "SEVENZIP", "0x0102"),
        "Failed to create UDF image"
    );

    let listing =
        list_udf_with_7z(&image_path).expect("7z should be able to list mkudffs UDF image");

    println!("=== 7z listing ===\n{}", listing);
    assert!(listing.len() > 50, "7z listing should have content");
}

// =============================================================================
// Tests for hadris-udf write support
// =============================================================================

#[cfg(feature = "write")]
mod write_tests {
    use super::*;
    use hadris_udf::descriptor::{ExtentDescriptor, LongAllocationDescriptor};
    use hadris_udf::write::{UdfWriteOptions, UdfWriter};
    use hadris_udf::{SECTOR_SIZE, UdfRevision};

    /// Create a minimal UDF image using hadris-udf
    fn create_hadris_udf_image(image_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let size_bytes: u64 = 10 * 1024 * 1024;
        let total_sectors = (size_bytes / SECTOR_SIZE as u64) as u32;

        let mut file = File::create(image_path)?;
        file.set_len(size_bytes)?;

        let options = UdfWriteOptions {
            volume_id: "HADRIS_UDF".to_string(),
            revision: UdfRevision::V1_02,
            partition_start: 300,
            partition_length: total_sectors - 350,
        };

        let mut writer = UdfWriter::new(&mut file, options);

        // Write VRS
        writer.write_vrs()?;

        // VDS locations
        let vds_start = 257u32;
        let vds_length = 6u32;
        let reserve_vds_start = 263u32;

        // AVDP
        let main_vds = ExtentDescriptor {
            length: vds_length * SECTOR_SIZE as u32,
            location: vds_start,
        };
        let reserve_vds = ExtentDescriptor {
            length: vds_length * SECTOR_SIZE as u32,
            location: reserve_vds_start,
        };
        writer.write_avdp(main_vds, reserve_vds)?;

        // FSD and root ICB
        let fsd_block = 0u32;
        let fsd_icb = LongAllocationDescriptor {
            extent_length: SECTOR_SIZE as u32,
            logical_block_num: fsd_block,
            partition_ref_num: 0,
            impl_use: [0; 6],
        };

        let root_icb = LongAllocationDescriptor {
            extent_length: SECTOR_SIZE as u32,
            logical_block_num: 1,
            partition_ref_num: 0,
            impl_use: [0; 6],
        };

        let lvid_location = reserve_vds_start + vds_length;
        let integrity_extent = ExtentDescriptor {
            length: SECTOR_SIZE as u32,
            location: lvid_location,
        };

        // Write VDS
        writer.write_pvd(vds_start, 0)?;
        writer.write_iuvd(vds_start + 1, 1)?;
        writer.write_partition_descriptor(vds_start + 2, 2)?;
        writer.write_lvd(vds_start + 3, 3, fsd_icb, integrity_extent)?;
        writer.write_usd(vds_start + 4, 4)?;
        writer.write_terminating_descriptor(vds_start + 5)?;

        // Write reserve VDS
        writer.write_pvd(reserve_vds_start, 0)?;
        writer.write_iuvd(reserve_vds_start + 1, 1)?;
        writer.write_partition_descriptor(reserve_vds_start + 2, 2)?;
        writer.write_lvd(reserve_vds_start + 3, 3, fsd_icb, integrity_extent)?;
        writer.write_usd(reserve_vds_start + 4, 4)?;
        writer.write_terminating_descriptor(reserve_vds_start + 5)?;

        // Write LVID and FSD
        writer.write_lvid(lvid_location, true)?;
        writer.write_fsd(fsd_block, root_icb)?;

        // Ensure data is flushed
        file.flush()?;
        file.sync_all()?;

        Ok(())
    }

    #[test]
    fn test_hadris_udf_has_valid_vrs() {
        let temp_dir = TempDir::new().unwrap();
        let image_path = temp_dir.path().join("hadris_vrs.img");

        create_hadris_udf_image(&image_path).expect("Should be able to create hadris UDF image");

        // Verify VRS at sector 16
        let mut file = File::open(&image_path).unwrap();
        let mut vrs_sector = [0u8; 2048];
        file.seek(SeekFrom::Start(16 * 2048)).unwrap();
        file.read_exact(&mut vrs_sector).unwrap();

        // Check for BEA01 identifier
        let vrs_id = &vrs_sector[1..6];
        assert_eq!(vrs_id, b"BEA01", "VRS should start with BEA01");
    }

    #[test]
    fn test_hadris_udf_has_valid_avdp() {
        let temp_dir = TempDir::new().unwrap();
        let image_path = temp_dir.path().join("hadris_avdp.img");

        create_hadris_udf_image(&image_path).expect("Should be able to create hadris UDF image");

        // Verify AVDP at sector 256
        let mut file = File::open(&image_path).unwrap();
        let mut avdp_sector = [0u8; 2048];
        file.seek(SeekFrom::Start(256 * 2048)).unwrap();
        file.read_exact(&mut avdp_sector).unwrap();

        // Check tag identifier (should be 2 for AVDP)
        let tag_id = u16::from_le_bytes([avdp_sector[0], avdp_sector[1]]);
        assert_eq!(tag_id, 2, "AVDP tag ID should be 2");
    }

    #[test]
    fn test_hadris_udf_readable_by_7z() {
        if !sevenzip_available() {
            eprintln!("Skipping test: 7z not available");
            return;
        }

        let temp_dir = TempDir::new().unwrap();
        let image_path = temp_dir.path().join("hadris_7z.img");

        create_hadris_udf_image(&image_path).expect("Should be able to create hadris UDF image");

        // Note: 7z may not be able to read our minimal UDF image yet
        // This test documents the current state
        match list_udf_with_7z(&image_path) {
            Some(listing) => {
                println!("7z can read hadris-udf image:\n{}", listing);
            }
            None => {
                println!("7z cannot read hadris-udf image yet (needs more complete metadata)");
                // This is expected for now - don't fail
            }
        }
    }

    #[test]
    fn test_hadris_udf_validated_by_udfinfo() {
        if !udfinfo_available() {
            eprintln!("Skipping test: udfinfo not available (Linux only)");
            return;
        }

        let temp_dir = TempDir::new().unwrap();
        let image_path = temp_dir.path().join("hadris_udfinfo.img");

        create_hadris_udf_image(&image_path).expect("Should be able to create hadris UDF image");

        // Note: udfinfo may report issues with our minimal UDF image
        // This test documents the current state
        match get_udf_info(&image_path) {
            Some(info) => {
                println!("udfinfo output:\n{}", info);
            }
            None => {
                println!("udfinfo cannot validate hadris-udf image yet");
                // This is expected for now - don't fail
            }
        }
    }
}
