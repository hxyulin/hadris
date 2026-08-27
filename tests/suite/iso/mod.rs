mod boot;
mod directory;
mod hybrid;
mod native;
mod peers;
mod rock_ridge;
mod spec;
mod volume_descriptors;

use std::io::Cursor;
use std::path::{Path, PathBuf};

use hadris_iso::read::IsoImage;
use hadris_tests::iso::xorriso;
use tempfile::TempDir;

/// Builds an image with xorriso from the sample tree. Returns `None` when the
/// test should be skipped because xorriso is unavailable.
fn xorriso_sample_image(
    create: impl FnOnce(&Path, &Path) -> Result<(), String>,
) -> Option<(TempDir, PathBuf)> {
    if !xorriso::require() {
        return None;
    }
    let temp_dir = TempDir::new().unwrap();
    let content_dir = temp_dir.path().join("content");
    std::fs::create_dir(&content_dir).unwrap();
    xorriso::write_sample_tree(&content_dir);
    let iso_path = temp_dir.path().join("image.iso");
    create(&content_dir, &iso_path).expect("xorriso should create the image");
    Some((temp_dir, iso_path))
}

fn open(bytes: Vec<u8>) -> IsoImage<Cursor<Vec<u8>>> {
    IsoImage::open(Cursor::new(bytes)).expect("failed to open ISO image")
}

fn open_file(path: &Path) -> IsoImage<Cursor<Vec<u8>>> {
    open(std::fs::read(path).unwrap())
}

/// Locates the El Torito boot catalog through the boot record volume
/// descriptor. Returns `(boot record sector, catalog LBA)`.
fn find_boot_catalog(data: &[u8]) -> Option<(usize, usize)> {
    for sector in 16..32 {
        let offset = sector * 2048;
        if data.len() <= offset + 75 {
            return None;
        }
        if data[offset] == 0x00 && &data[offset + 1..offset + 6] == b"CD001" {
            let pointer: [u8; 4] = data[offset + 71..offset + 75].try_into().ok()?;
            return Some((sector, u32::from_le_bytes(pointer) as usize));
        }
        if data[offset] == 0xFF {
            return None;
        }
    }
    None
}

fn validation_checksum(entry: &[u8]) -> u16 {
    (0..32).step_by(2).fold(0u16, |sum, i| {
        sum.wrapping_add(u16::from_le_bytes([entry[i], entry[i + 1]]))
    })
}
