#![cfg(feature = "write")]
//! Regression POCs for the original PR #89 implementation (`bdb46ce`).
//! The first returned `InvalidInput`; the second read from the cluster where
//! caching was enabled instead of the file's first cluster.

use std::fs::{File, OpenOptions};
use std::io::Read as _;
use std::path::PathBuf;

use hadris_fat::format::{FatFormatOptions, FatTypeSelection, FatVolumeFormatter};
use hadris_fat::{FatVolume, FatVolumeReadExt, FatVolumeWriteExt};
use hadris_io::SeekFrom;
use tempfile::TempDir;

const IMAGE_SIZE: u64 = 2 * 1024 * 1024;

fn fixture() -> (TempDir, PathBuf, Vec<u8>, usize) {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("seek-poc.img");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .unwrap();
    file.set_len(IMAGE_SIZE).unwrap();
    FatVolumeFormatter::format(
        file,
        FatFormatOptions::new(IMAGE_SIZE).fat_type(FatTypeSelection::Fat12),
    )
    .unwrap();

    let mut boot = [0_u8; 64];
    File::open(&path).unwrap().read_exact(&mut boot).unwrap();
    let cluster_size = u16::from_le_bytes([boot[11], boot[12]]) as usize * boot[13] as usize;
    let payload: Vec<u8> = (0..cluster_size * 4)
        .map(|offset| (offset / cluster_size) as u8)
        .collect();

    let fs = FatVolume::open(
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap(),
    )
    .unwrap();
    let entry = fs.create_file(&fs.root_dir(), "SEEK.BIN").unwrap();
    let mut writer = fs.write_file(&entry).unwrap();
    writer.write(&payload).unwrap();
    writer.finish().unwrap();
    drop(fs);

    (tmp, path, payload, cluster_size)
}

#[test]
fn relative_seek_after_large_absolute_seek() {
    let (_tmp, path, _payload, _cluster_size) = fixture();
    let fs = FatVolume::open(OpenOptions::new().read(true).open(path).unwrap()).unwrap();
    let entry = fs.root_dir().find("SEEK.BIN").unwrap().unwrap();
    let mut reader = fs.read_file(&entry).unwrap();

    assert_eq!(reader.seek(SeekFrom::Start(u64::MAX)).unwrap(), u64::MAX);
    assert_eq!(reader.seek(SeekFrom::Current(-1)).unwrap(), u64::MAX - 1);
}

#[test]
fn cache_enabled_after_seek_keeps_absolute_chain_origin() {
    let (_tmp, path, payload, cluster_size) = fixture();
    let fs = FatVolume::open(OpenOptions::new().read(true).open(path).unwrap()).unwrap();
    let entry = fs.root_dir().find("SEEK.BIN").unwrap().unwrap();
    let mut reader = fs.read_file(&entry).unwrap();

    reader
        .seek(SeekFrom::Start(cluster_size as u64 + 7))
        .unwrap();
    let mut reader = reader.with_cached_chain().unwrap();
    reader.seek(SeekFrom::Start(0)).unwrap();

    let mut byte = [0_u8; 1];
    assert_eq!(reader.read(&mut byte).unwrap(), 1);
    assert_eq!(byte[0], payload[0]);
}
