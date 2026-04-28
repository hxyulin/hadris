//! Roundtrip tests for hadris-fat FAT12/FAT16/FAT32 write + read paths.
//!
//! Each test formats a fresh image with `FatVolumeFormatter::format`, writes
//! content using the hadris-fat write API, then reads it back through the
//! same API to verify byte-for-byte equality. When `fsck.fat` (from
//! `dosfstools`) is available on the host, the image is also validated
//! externally — both right after format and after every write.
//!
//! These mirror the exFAT roundtrip tests added in commit 1f7405b for
//! issue #25.

#![cfg(feature = "write")]

use std::fs::OpenOptions;
use std::io::Seek as _;
use std::path::Path;
use tempfile::TempDir;

use hadris_fat::format::{FatTypeSelection, FatVolumeFormatter, FormatOptions};
use hadris_fat::{FatFs, FatFsReadExt, FatFsWriteExt, FatType};

mod fat_helpers;
use fat_helpers::{fsck_check, fsck_fat_available};

/// Build a fresh, formatted FAT image at `path` with the requested FAT type.
fn make_image(path: &Path, size: u64, fat_type: FatTypeSelection, label: &str) {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .expect("create image file");
    file.set_len(size).expect("set image length");

    let options = FormatOptions::new(size)
        .with_label(label)
        .with_fat_type(fat_type);

    let _fs = FatVolumeFormatter::format(file, options).expect("FatVolumeFormatter::format");
    // Dropping `_fs` flushes its underlying File handle.
}

/// Open an image file at the start, ready for FatFs::open.
fn open_image(path: &Path) -> std::fs::File {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .expect("open image");
    file.seek(std::io::SeekFrom::Start(0)).unwrap();
    file
}

/// Write `name` with `contents` into the root directory.
fn write_root_file(image_path: &Path, name: &str, contents: &[u8]) {
    let file = open_image(image_path);
    let fs = FatFs::open(file).expect("open FAT");

    let entry = {
        let root = fs.root_dir();
        fs.create_file(&root, name).expect("create_file")
    };
    {
        let mut writer = fs.write_file(&entry).expect("write_file");
        writer.write(contents).expect("writer.write");
        writer.finish().expect("writer.finish");
    }
}

/// Create a subdirectory `name` in the root directory.
fn create_root_dir(image_path: &Path, name: &str) {
    let file = open_image(image_path);
    let fs = FatFs::open(file).expect("open FAT");

    let root = fs.root_dir();
    fs.create_dir(&root, name).expect("create_dir");
}

/// Truncate `name` in the root directory to `new_size`.
fn truncate_root_file(image_path: &Path, name: &str, new_size: usize) {
    let file = open_image(image_path);
    let fs = FatFs::open(file).expect("open FAT");

    let entry = {
        let root = fs.root_dir();
        root.find(name).expect("find").expect("present")
    };
    fs.truncate(&entry, new_size).expect("truncate");
}

/// Delete `name` from the root directory.
fn delete_root_file(image_path: &Path, name: &str) {
    let file = open_image(image_path);
    let fs = FatFs::open(file).expect("open FAT");

    let entry = {
        let root = fs.root_dir();
        root.find(name).expect("find").expect("present")
    };
    fs.delete(&entry).expect("delete");
}

/// Read the full contents of `name` from the root directory.
fn read_root_file(image_path: &Path, name: &str) -> Vec<u8> {
    let file = open_image(image_path);
    let fs = FatFs::open(file).expect("reopen FAT");

    let entry = {
        let root = fs.root_dir();
        root.find(name).expect("find").expect("present")
    };
    let mut reader = fs.read_file(&entry).expect("read_file");

    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = reader.read(&mut buf).expect("reader.read");
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    out
}

fn maybe_fsck(image_path: &Path) {
    if !fsck_fat_available() {
        eprintln!("note: fsck.fat not available, skipping external validation");
        return;
    }
    if let Err(e) = fsck_check(image_path) {
        panic!("fsck.fat rejected the image: {e}");
    }
}

/// Sanity-check that the chosen size + selection actually produced the
/// expected on-disk FAT type. Catches silent fallbacks.
fn assert_fat_type(image_path: &Path, expected: FatType) {
    let file = open_image(image_path);
    let fs = FatFs::open(file).expect("open for fat_type check");
    assert_eq!(fs.fat_type(), expected, "unexpected FAT type on disk");
}

// =============================================================================
// FAT12 — small volume (~2 MB)
// =============================================================================

const FAT12_SIZE: u64 = 2 * 1024 * 1024;

#[test]
fn fat12_format_is_clean() {
    let tmp = TempDir::new().unwrap();
    let img = tmp.path().join("fat12_clean.img");
    make_image(&img, FAT12_SIZE, FatTypeSelection::Fat12, "FAT12CLEAN");
    assert_fat_type(&img, FatType::Fat12);
    maybe_fsck(&img);
}

#[test]
fn fat12_roundtrip_small_file() {
    let tmp = TempDir::new().unwrap();
    let img = tmp.path().join("fat12_small.img");
    make_image(&img, FAT12_SIZE, FatTypeSelection::Fat12, "FAT12SMALL");
    assert_fat_type(&img, FatType::Fat12);

    let payload = b"hello, FAT12 roundtrip\n";
    write_root_file(&img, "HELLO.TXT", payload);
    maybe_fsck(&img);

    let got = read_root_file(&img, "HELLO.TXT");
    assert_eq!(got, payload, "FAT12 roundtrip content mismatch");
}

#[test]
fn fat12_roundtrip_multi_cluster_file() {
    let tmp = TempDir::new().unwrap();
    let img = tmp.path().join("fat12_multi.img");
    make_image(&img, FAT12_SIZE, FatTypeSelection::Fat12, "FAT12MULTI");

    // ~32 KiB of pseudo-random bytes — guaranteed to span several clusters
    // for any reasonable cluster size on a 2 MiB FAT12 image.
    let payload: Vec<u8> = (0..(32 * 1024)).map(|i| (i * 31 + 7) as u8).collect();
    write_root_file(&img, "BLOB.BIN", &payload);
    maybe_fsck(&img);

    let got = read_root_file(&img, "BLOB.BIN");
    assert_eq!(got.len(), payload.len(), "FAT12 size mismatch");
    assert_eq!(got, payload, "FAT12 content mismatch");
}

// =============================================================================
// FAT16 — medium volume (~16 MB)
// =============================================================================

const FAT16_SIZE: u64 = 16 * 1024 * 1024;

#[test]
fn fat16_format_is_clean() {
    let tmp = TempDir::new().unwrap();
    let img = tmp.path().join("fat16_clean.img");
    make_image(&img, FAT16_SIZE, FatTypeSelection::Fat16, "FAT16CLEAN");
    assert_fat_type(&img, FatType::Fat16);
    maybe_fsck(&img);
}

#[test]
fn fat16_roundtrip_small_file() {
    let tmp = TempDir::new().unwrap();
    let img = tmp.path().join("fat16_small.img");
    make_image(&img, FAT16_SIZE, FatTypeSelection::Fat16, "FAT16SMALL");
    assert_fat_type(&img, FatType::Fat16);

    let payload = b"hello, FAT16 roundtrip\n";
    write_root_file(&img, "HELLO.TXT", payload);
    maybe_fsck(&img);

    let got = read_root_file(&img, "HELLO.TXT");
    assert_eq!(got, payload, "FAT16 roundtrip content mismatch");
}

#[test]
fn fat16_roundtrip_multi_cluster_file() {
    let tmp = TempDir::new().unwrap();
    let img = tmp.path().join("fat16_multi.img");
    make_image(&img, FAT16_SIZE, FatTypeSelection::Fat16, "FAT16MULTI");

    let payload: Vec<u8> = (0..(256 * 1024)).map(|i| (i * 13 + 1) as u8).collect();
    write_root_file(&img, "BLOB.BIN", &payload);
    maybe_fsck(&img);

    let got = read_root_file(&img, "BLOB.BIN");
    assert_eq!(got.len(), payload.len(), "FAT16 size mismatch");
    assert_eq!(got, payload, "FAT16 content mismatch");
}

// =============================================================================
// FAT32 — minimum-sized volume (forced; auto would pick FAT16)
// =============================================================================

const FAT32_SIZE: u64 = 64 * 1024 * 1024;

#[test]
fn fat32_format_is_clean() {
    let tmp = TempDir::new().unwrap();
    let img = tmp.path().join("fat32_clean.img");
    make_image(&img, FAT32_SIZE, FatTypeSelection::Fat32, "FAT32CLEAN");
    assert_fat_type(&img, FatType::Fat32);
    maybe_fsck(&img);
}

#[test]
fn fat32_roundtrip_small_file() {
    let tmp = TempDir::new().unwrap();
    let img = tmp.path().join("fat32_small.img");
    make_image(&img, FAT32_SIZE, FatTypeSelection::Fat32, "FAT32SMALL");
    assert_fat_type(&img, FatType::Fat32);

    let payload = b"hello, FAT32 roundtrip\n";
    write_root_file(&img, "HELLO.TXT", payload);
    maybe_fsck(&img);

    let got = read_root_file(&img, "HELLO.TXT");
    assert_eq!(got, payload, "FAT32 roundtrip content mismatch");
}

#[test]
fn fat32_roundtrip_multi_cluster_file() {
    let tmp = TempDir::new().unwrap();
    let img = tmp.path().join("fat32_multi.img");
    make_image(&img, FAT32_SIZE, FatTypeSelection::Fat32, "FAT32MULTI");

    // 1 MiB — comfortably spans multiple clusters at any cluster size
    // FAT32 picks for a 64 MiB volume.
    let payload: Vec<u8> = (0..(1024 * 1024)).map(|i| (i * 7 + 3) as u8).collect();
    write_root_file(&img, "BLOB.BIN", &payload);
    maybe_fsck(&img);

    let got = read_root_file(&img, "BLOB.BIN");
    assert_eq!(got.len(), payload.len(), "FAT32 size mismatch");
    assert_eq!(got, payload, "FAT32 content mismatch");
}

#[test]
fn fat32_rename_dir_into_root_clean_fsck() {
    // FAT32 spec quirk: a directory's ".." entry must store cluster 0 when its
    // parent is the FAT32 root, even though the root has a real cluster. When
    // a directory is renamed/moved into the root, rename() must rewrite the
    // ".." entry following the same rule.
    let tmp = TempDir::new().unwrap();
    let img = tmp.path().join("fat32_rename.img");
    make_image(&img, FAT32_SIZE, FatTypeSelection::Fat32, "FAT32REN");

    {
        let file = open_image(&img);
        let fs = FatFs::open(file).expect("open FAT");
        let root = fs.root_dir();
        let _sub1 = fs.create_dir(&root, "SUB1").expect("create SUB1");
    }
    {
        let file = open_image(&img);
        let fs = FatFs::open(file).expect("open FAT");
        let root = fs.root_dir();
        let sub1 = root.open_dir("SUB1").expect("open SUB1");
        let _sub2 = fs.create_dir(&sub1, "SUB2").expect("create SUB2");
    }
    {
        let file = open_image(&img);
        let fs = FatFs::open(file).expect("open FAT");
        let root = fs.root_dir();
        let sub1 = root.open_dir("SUB1").expect("open SUB1");
        let sub2_entry = sub1.find("SUB2").expect("find SUB2").expect("present");
        fs.rename(&sub2_entry, &root, "SUB2").expect("rename");
    }
    maybe_fsck(&img);
}

#[test]
fn fat32_root_extension_clean_fsck() {
    // Creating enough short-named (8.3) files in the FAT32 root to overflow
    // its initial single cluster forces find_free_entry_slot_in_dir to
    // allocate a new cluster mid-create_file. That allocator path must keep
    // the on-disk FSInfo free_count consistent.
    let tmp = TempDir::new().unwrap();
    let img = tmp.path().join("fat32_rootext.img");
    make_image(&img, FAT32_SIZE, FatTypeSelection::Fat32, "FAT32EXT");

    {
        let file = open_image(&img);
        let fs = FatFs::open(file).expect("open FAT");
        let root = fs.root_dir();
        // 64 MiB / 512-byte cluster → 16 entries per cluster initially;
        // 32 files guarantees at least one extension.
        for i in 0..32 {
            let name = format!("F{i:02}.TXT");
            fs.create_file(&root, &name).expect("create_file");
        }
    }
    maybe_fsck(&img);
}

#[test]
fn fat32_create_dir_clean_fsck() {
    let tmp = TempDir::new().unwrap();
    let img = tmp.path().join("fat32_mkdir.img");
    make_image(&img, FAT32_SIZE, FatTypeSelection::Fat32, "FAT32MKDIR");

    create_root_dir(&img, "SUBDIR");
    maybe_fsck(&img);
}

#[test]
fn fat32_delete_clean_fsck() {
    let tmp = TempDir::new().unwrap();
    let img = tmp.path().join("fat32_delete.img");
    make_image(&img, FAT32_SIZE, FatTypeSelection::Fat32, "FAT32DEL");

    let payload: Vec<u8> = (0..(32 * 1024)).map(|i| (i * 17 + 9) as u8).collect();
    write_root_file(&img, "DOOMED.BIN", &payload);
    maybe_fsck(&img);

    delete_root_file(&img, "DOOMED.BIN");
    maybe_fsck(&img);
}

#[test]
fn fat32_truncate_to_zero_clean_fsck() {
    let tmp = TempDir::new().unwrap();
    let img = tmp.path().join("fat32_truncate.img");
    make_image(&img, FAT32_SIZE, FatTypeSelection::Fat32, "FAT32TRUNC");

    // Multi-cluster file — truncating to 0 must free all of them and the
    // FSInfo free_count must reflect that on disk for fsck.fat.
    let payload: Vec<u8> = (0..(64 * 1024)).map(|i| (i * 11 + 5) as u8).collect();
    write_root_file(&img, "BIG.BIN", &payload);
    maybe_fsck(&img);

    truncate_root_file(&img, "BIG.BIN", 0);
    maybe_fsck(&img);
}

// =============================================================================
// Cross-FAT: multiple files in the same root directory
// =============================================================================

#[test]
fn fat32_roundtrip_multiple_files() {
    let tmp = TempDir::new().unwrap();
    let img = tmp.path().join("fat32_multifile.img");
    make_image(&img, FAT32_SIZE, FatTypeSelection::Fat32, "FAT32MANY");
    assert_fat_type(&img, FatType::Fat32);

    let files: &[(&str, &[u8])] = &[
        ("A.TXT", b"alpha"),
        ("B.TXT", b"bravo"),
        ("C.BIN", &[0xCC; 8192]),
    ];

    for (name, payload) in files {
        write_root_file(&img, name, payload);
    }
    maybe_fsck(&img);

    for (name, payload) in files {
        let got = read_root_file(&img, name);
        assert_eq!(&got, payload, "mismatch for {name}");
    }
}
