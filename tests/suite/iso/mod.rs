mod boot;
mod directory;
mod hybrid;
mod multi_extent;
mod native;
mod peers;
mod relocation;
mod rock_ridge;
mod spec;
mod volume_descriptors;

use std::path::{Path, PathBuf};

use hadris_fs::sync::DriverExt;
use hadris_fs::{Metadata, NodeId};
use hadris_iso::raw::VolumeDescriptor;
use hadris_iso::sync::IsoView;
use hadris_tests::iso::hadris::Image;
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

fn open(bytes: Vec<u8>) -> Image {
    hadris_tests::iso::hadris::open(bytes).expect("failed to open ISO image")
}

fn open_file(path: &Path) -> Image {
    open(std::fs::read(path).unwrap())
}

/// The volume identifier of the primary volume descriptor.
fn volume_id(image: &mut Image) -> String {
    let pvd = image.primary_descriptor().unwrap();
    String::from_utf8_lossy(pvd.volume_identifier.trimmed()).into_owned()
}

/// The volume descriptor set, up to and including the terminator.
fn descriptors(image: &mut Image) -> Vec<VolumeDescriptor> {
    let mut all = Vec::new();
    while let Some(descriptor) = image.descriptor(all.len() as u32).unwrap() {
        all.push(descriptor);
    }
    all
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

/// The entries of the directory `path`: name, node and metadata.
fn list<D: hadris_storage::sync::BlockDevice>(
    view: &mut IsoView<D>,
    path: &str,
) -> Vec<(String, NodeId, Metadata)> {
    let mut items = Vec::new();
    for item in view.read_dir(path).unwrap() {
        let item = item.unwrap();
        items.push((
            String::from_utf8_lossy(item.name_bytes()).into_owned(),
            item.entry().node(),
        ));
    }
    items
        .into_iter()
        .map(|(name, node)| {
            let meta = view.node_metadata(node).unwrap();
            (name, node, meta)
        })
        .collect()
}

/// The byte range of the first extent of `node`.
fn first_extent<D: hadris_storage::sync::BlockDevice>(
    view: &mut IsoView<D>,
    node: NodeId,
) -> hadris_fs::Extent {
    let mut first = None;
    view.extents(node, |extent| {
        first.get_or_insert(extent);
    })
    .unwrap();
    first.unwrap()
}
