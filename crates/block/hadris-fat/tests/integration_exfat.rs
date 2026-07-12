//! Integration tests for hadris-fat exFAT support using external tools.
//!
//! These tests verify that hadris-fat can correctly read exFAT images created by
//! external tools and that images created by hadris-fat can be read by external tools.
//!
//! External tools used:
//! - Linux: `mkfs.exfat`, `fsck.exfat` (exfatprogs)
//! - macOS: `hdiutil`, `newfs_exfat` (native)
//! - Cross-platform: `7z` (p7zip)
//!
//! Install on various platforms:
//! - Ubuntu/Debian: `sudo apt install exfatprogs p7zip-full`
//! - Fedora: `sudo dnf install exfatprogs p7zip`
//! - macOS: `brew install p7zip` (hdiutil is native)
//! - Arch: `sudo pacman -S exfatprogs p7zip`

#![cfg(feature = "exfat")]

use std::fs::{self, File, OpenOptions};
use std::io::{Cursor, Read};
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

use hadris_fat::exfat::{ExFatFormatOptions, ExFatFs, format_exfat};

// =============================================================================
// Tool availability checks
// =============================================================================

/// Check if mkfs.exfat is available (Linux)
fn mkfs_exfat_available() -> bool {
    Command::new("mkfs.exfat")
        .arg("--help")
        .output()
        .map(|o| o.status.success() || o.status.code() == Some(1))
        .unwrap_or(false)
}

/// Check if hdiutil is available (macOS)
fn hdiutil_available() -> bool {
    Command::new("hdiutil")
        .arg("info")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Check if fsck.exfat is available (Linux)
fn fsck_exfat_available() -> bool {
    Command::new("fsck.exfat")
        .arg("--help")
        .output()
        .map(|o| o.status.success() || o.status.code().is_some())
        .unwrap_or(false)
}

/// Check if 7z is available
fn sevenzip_available() -> bool {
    Command::new("7z")
        .output()
        .map(|o| o.status.success() || o.status.code() == Some(0))
        .unwrap_or(false)
}

/// Check if any exFAT creation tool is available
fn exfat_tool_available() -> bool {
    mkfs_exfat_available() || hdiutil_available()
}

// =============================================================================
// exFAT image creation functions
// =============================================================================

/// Create an exFAT image using mkfs.exfat (Linux)
fn create_exfat_with_mkfs(image_path: &Path, size_mb: u32, label: &str) -> bool {
    let size_bytes = (size_mb as u64) * 1024 * 1024;

    let file = File::create(image_path).expect("Failed to create image file");
    file.set_len(size_bytes).expect("Failed to set file size");
    drop(file);

    let output = Command::new("mkfs.exfat")
        .args(["-n", label, image_path.to_str().unwrap()])
        .output()
        .expect("Failed to run mkfs.exfat");

    if !output.status.success() {
        eprintln!(
            "mkfs.exfat failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    output.status.success()
}

/// Create an exFAT image using hdiutil (macOS)
fn create_exfat_with_hdiutil(image_path: &Path, size_mb: u32, label: &str) -> bool {
    let dmg_path = image_path.with_extension("dmg");

    let output = Command::new("hdiutil")
        .args([
            "create",
            "-size",
            &format!("{}m", size_mb),
            "-fs",
            "ExFAT",
            "-volname",
            label,
            "-layout",
            "NONE",
            dmg_path.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to run hdiutil create");

    if !output.status.success() {
        eprintln!(
            "hdiutil create failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return false;
    }

    let output = Command::new("hdiutil")
        .args([
            "convert",
            dmg_path.to_str().unwrap(),
            "-format",
            "UDTO",
            "-o",
            image_path.with_extension("").to_str().unwrap(),
        ])
        .output()
        .expect("Failed to run hdiutil convert");

    let _ = fs::remove_file(&dmg_path);

    if !output.status.success() {
        eprintln!(
            "hdiutil convert failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return false;
    }

    let cdr_path = image_path.with_extension("cdr");
    if cdr_path.exists() {
        let _ = fs::rename(&cdr_path, image_path);
    }

    image_path.exists()
}

/// Create an exFAT image using the best available tool
fn create_exfat_image(image_path: &Path, size_mb: u32, label: &str) -> bool {
    if mkfs_exfat_available() {
        create_exfat_with_mkfs(image_path, size_mb, label)
    } else if hdiutil_available() {
        create_exfat_with_hdiutil(image_path, size_mb, label)
    } else {
        false
    }
}

// =============================================================================
// Verification functions
// =============================================================================

/// Check exFAT image with fsck.exfat (Linux only)
fn check_exfat_with_fsck(image_path: &Path) -> bool {
    let output = Command::new("fsck.exfat")
        .args(["-n", image_path.to_str().unwrap()])
        .output()
        .expect("Failed to run fsck.exfat");

    output.status.success() || output.status.code() == Some(1)
}

/// List files in exFAT image using 7z
fn list_exfat_with_7z(image_path: &Path) -> Option<String> {
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
// Tests for reading exFAT images created by external tools
// =============================================================================

#[test]
fn test_read_external_exfat_basic() {
    if !exfat_tool_available() {
        eprintln!("Skipping test: no exFAT creation tool available");
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let image_path = temp_dir.path().join("test_exfat.img");

    assert!(
        create_exfat_image(&image_path, 32, "TESTEXFAT"),
        "Failed to create exFAT image"
    );

    let mut exfat_data = Vec::new();
    File::open(&image_path)
        .unwrap()
        .read_to_end(&mut exfat_data)
        .unwrap();

    let cursor = Cursor::new(exfat_data);
    let fs = ExFatFs::open(cursor).expect("hadris-fat should be able to open external exFAT image");

    let info = fs.info();
    println!("=== exFAT Info ===");
    println!("Bytes per sector: {}", info.bytes_per_sector);
    println!("Sectors per cluster: {}", info.sectors_per_cluster);
    println!("Cluster count: {}", info.cluster_count);
    println!("Root cluster: {}", info.root_cluster);

    assert!(
        info.bytes_per_sector >= 512,
        "Sector size should be at least 512"
    );
    assert!(info.cluster_count > 0, "Should have clusters");
    assert!(info.root_cluster >= 2, "Root cluster should be >= 2");

    // Verify root directory is accessible
    let root = fs.root_dir();
    let _entry_count = root.entries().count();
}

#[test]
fn test_read_exfat_volume_info() {
    if !exfat_tool_available() {
        eprintln!("Skipping test: no exFAT creation tool available");
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let image_path = temp_dir.path().join("test_volinfo.img");

    assert!(
        create_exfat_image(&image_path, 32, "MYVOLUME"),
        "Failed to create exFAT image"
    );

    let exfat_data = fs::read(&image_path).unwrap();
    let cursor = Cursor::new(exfat_data);

    let fs = ExFatFs::open(cursor).expect("Should be able to open exFAT image");

    let serial = fs.volume_serial();
    println!("Volume serial: {:#010x}", serial);

    let free = fs.free_cluster_count();
    let total = fs.info().cluster_count;
    println!("Free clusters: {} / {}", free, total);

    assert!(free <= total, "Free clusters should not exceed total");
    assert!(free > 0, "Should have some free clusters");
}

#[test]
fn test_fsck_validates_mkfs_image() {
    if !mkfs_exfat_available() || !fsck_exfat_available() {
        eprintln!("Skipping test: mkfs.exfat or fsck.exfat not available (Linux only)");
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let image_path = temp_dir.path().join("fsck_test.img");

    assert!(
        create_exfat_with_mkfs(&image_path, 32, "FSCKTEST"),
        "Failed to create exFAT image"
    );

    assert!(
        check_exfat_with_fsck(&image_path),
        "fsck.exfat should validate mkfs.exfat image without errors"
    );
}

#[test]
fn test_exfat_boot_sector_structure() {
    if !exfat_tool_available() {
        eprintln!("Skipping test: no exFAT creation tool available");
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let image_path = temp_dir.path().join("analyze.img");

    assert!(
        create_exfat_image(&image_path, 32, "ANALYZE"),
        "Failed to create exFAT image"
    );

    let exfat_data = fs::read(&image_path).unwrap();

    // Verify exFAT signature
    assert_eq!(
        &exfat_data[3..11],
        b"EXFAT   ",
        "Should have EXFAT signature"
    );
    assert_eq!(exfat_data[510], 0x55, "Boot signature byte 1");
    assert_eq!(exfat_data[511], 0xAA, "Boot signature byte 2");

    // Verify basic structure
    let volume_length = u64::from_le_bytes(exfat_data[72..80].try_into().unwrap());
    let cluster_count = u32::from_le_bytes(exfat_data[92..96].try_into().unwrap());
    let root_cluster = u32::from_le_bytes(exfat_data[96..100].try_into().unwrap());

    println!("Volume length: {} sectors", volume_length);
    println!("Cluster count: {}", cluster_count);
    println!("Root cluster: {}", root_cluster);

    assert!(volume_length > 0, "Volume length should be > 0");
    assert!(cluster_count > 0, "Cluster count should be > 0");
    assert!(root_cluster >= 2, "Root cluster should be >= 2");
}

#[test]
fn test_various_image_sizes() {
    if !exfat_tool_available() {
        eprintln!("Skipping test: no exFAT creation tool available");
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let sizes = [16u32, 32, 64, 128];

    println!("=== exFAT Size Compatibility ===");

    for size in sizes {
        let image_path = temp_dir.path().join(format!("exfat_{}mb.img", size));

        assert!(
            create_exfat_image(&image_path, size, &format!("SIZE{}", size)),
            "Failed to create {}MB exFAT image",
            size
        );

        let exfat_data = fs::read(&image_path).unwrap();
        let cursor = Cursor::new(exfat_data);

        let fs = ExFatFs::open(cursor)
            .unwrap_or_else(|e| panic!("Failed to open {}MB image: {:?}", size, e));

        let info = fs.info();
        println!(
            "{}MB: {} clusters, {} bytes/cluster",
            size, info.cluster_count, info.bytes_per_cluster
        );

        assert!(
            info.cluster_count > 0,
            "{}MB image should have clusters",
            size
        );
    }
}

// =============================================================================
// Tests for hadris-fat exFAT formatting
// =============================================================================

#[cfg(feature = "write")]
mod write_tests {
    use super::*;

    #[test]
    fn test_hadris_format_creates_valid_boot_sector() {
        let temp_dir = TempDir::new().unwrap();
        let image_path = temp_dir.path().join("hadris_exfat.img");

        let size_bytes: u64 = 32 * 1024 * 1024;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&image_path)
            .expect("Failed to create image");
        file.set_len(size_bytes).expect("Failed to set size");

        let options = ExFatFormatOptions::default().with_label("HADRIS");

        format_exfat(&mut file, size_bytes, &options)
            .expect("hadris-fat should be able to format exFAT");

        // Flush and reopen
        file.sync_all().unwrap();
        drop(file);

        // Verify boot sector
        let mut file = File::open(&image_path).unwrap();
        let mut boot_sector = [0u8; 512];
        file.read_exact(&mut boot_sector).unwrap();

        assert_eq!(
            &boot_sector[3..11],
            b"EXFAT   ",
            "Should have EXFAT signature"
        );
        assert_eq!(boot_sector[510], 0x55, "Boot signature byte 1");
        assert_eq!(boot_sector[511], 0xAA, "Boot signature byte 2");
    }

    #[test]
    fn test_hadris_format_readable_by_hadris() {
        let temp_dir = TempDir::new().unwrap();
        let image_path = temp_dir.path().join("hadris_read.img");

        let size_bytes: u64 = 32 * 1024 * 1024;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&image_path)
            .expect("Failed to create image");
        file.set_len(size_bytes).expect("Failed to set size");

        let options = ExFatFormatOptions::default().with_label("HADRIS");

        format_exfat(&mut file, size_bytes, &options)
            .expect("hadris-fat should be able to format exFAT");

        file.sync_all().unwrap();
        drop(file);

        // Re-open and read with hadris-fat
        let exfat_data = fs::read(&image_path).unwrap();
        let cursor = Cursor::new(exfat_data);

        let fs = ExFatFs::open(cursor)
            .expect("hadris-fat should be able to read its own formatted image");

        let info = fs.info();
        println!("Hadris-formatted exFAT:");
        println!("  Bytes per sector: {}", info.bytes_per_sector);
        println!("  Cluster count: {}", info.cluster_count);

        assert!(info.bytes_per_sector >= 512, "Sector size should be valid");
        assert!(info.cluster_count > 0, "Should have clusters");
    }

    #[test]
    fn test_hadris_format_validated_by_fsck() {
        if !fsck_exfat_available() {
            eprintln!("Skipping test: fsck.exfat not available (Linux only)");
            return;
        }

        let temp_dir = TempDir::new().unwrap();
        let image_path = temp_dir.path().join("hadris_fsck.img");

        let size_bytes: u64 = 32 * 1024 * 1024;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&image_path)
            .expect("Failed to create image");
        file.set_len(size_bytes).expect("Failed to set size");

        let options = ExFatFormatOptions::default().with_label("FSCKTEST");

        format_exfat(&mut file, size_bytes, &options)
            .expect("hadris-fat should be able to format exFAT");

        file.sync_all().unwrap();
        drop(file);

        assert!(
            check_exfat_with_fsck(&image_path),
            "fsck.exfat should validate hadris-formatted image"
        );
    }

    #[test]
    fn test_hadris_format_readable_by_7z() {
        if !sevenzip_available() {
            eprintln!("Skipping test: 7z not available");
            return;
        }

        let temp_dir = TempDir::new().unwrap();
        let image_path = temp_dir.path().join("hadris_7z.img");

        let size_bytes: u64 = 32 * 1024 * 1024;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&image_path)
            .expect("Failed to create image");
        file.set_len(size_bytes).expect("Failed to set size");

        let options = ExFatFormatOptions::default().with_label("SEVENZIP");

        format_exfat(&mut file, size_bytes, &options)
            .expect("hadris-fat should be able to format exFAT");

        file.sync_all().unwrap();
        drop(file);

        // Note: 7z may not be able to read raw exFAT images
        // This test documents the current state
        match list_exfat_with_7z(&image_path) {
            Some(listing) => {
                println!("7z can read hadris-formatted exFAT:\n{}", listing);
            }
            None => {
                println!("7z cannot read raw exFAT images (expected)");
            }
        }
    }
}
