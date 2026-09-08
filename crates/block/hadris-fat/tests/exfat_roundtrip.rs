//! Roundtrip tests for hadris-fat exFAT write + read paths.
//!
//! Each test formats a fresh image with `format_exfat`, writes content using
//! the hadris-fat write API, then reads it back through the same API to
//! verify byte-for-byte equality. When `fsck.exfat` is available on the host,
//! the image is also validated externally.

#![cfg(all(feature = "unstable-exfat", feature = "write"))]

use std::fs::OpenOptions;
use std::io::Seek as _;
use std::path::Path;
use tempfile::TempDir;

use hadris_fat::exfat::{ExFatFormatOptions, ExFatVolume, format_exfat};
use hadris_fat::io::{Read as HadrisRead, Write as HadrisWrite};

#[path = "common/exfat.rs"]
mod exfat_helpers;
use exfat_helpers::{fsck_check, fsck_exfat_available};

const IMAGE_SIZE: u64 = 32 * 1024 * 1024;

/// Build a fresh, formatted exFAT image at `path`.
fn make_image(path: &Path, label: &str) {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .expect("create image file");
    file.set_len(IMAGE_SIZE).expect("set image length");

    let opts = ExFatFormatOptions::default().volume_label(label);
    format_exfat(&mut file, IMAGE_SIZE, &opts).expect("format_exfat");
    file.sync_all().expect("sync");
}

/// Open an image file at the start, ready for ExFatVolume::open.
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
    let fs = ExFatVolume::open(file).expect("open exFAT");

    let entry = {
        let root = fs.root_dir();
        fs.create_file(&root, name).expect("create_file")
    };
    let mut writer = fs.write_file(&entry).expect("write_file");
    writer.write_all(contents).expect("write_all");
    writer.finish().expect("writer.finish");
}

/// Read the full contents of `name` from the root directory.
fn read_root_file(image_path: &Path, name: &str) -> Vec<u8> {
    let file = open_image(image_path);
    let fs = ExFatVolume::open(file).expect("reopen exFAT");
    let mut reader = fs.open_file(name).expect("open_file");

    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = HadrisRead::read(&mut reader, &mut buf).expect("read");
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    out
}

fn maybe_fsck(image_path: &Path) {
    if !fsck_exfat_available() {
        eprintln!("note: fsck.exfat not available, skipping external validation");
        return;
    }
    if let Err(e) = fsck_check(image_path) {
        panic!("fsck.exfat rejected the image: {e}");
    }
}

#[test]
fn roundtrip_small_file() {
    let tmp = TempDir::new().unwrap();
    let img = tmp.path().join("small.img");
    make_image(&img, "SMALL");

    let payload = b"hello, exFAT roundtrip\n";
    write_root_file(&img, "hello.txt", payload);
    maybe_fsck(&img);

    let got = read_root_file(&img, "hello.txt");
    assert_eq!(got, payload, "roundtrip content mismatch");
}

#[test]
fn roundtrip_zero_byte_file() {
    let tmp = TempDir::new().unwrap();
    let img = tmp.path().join("empty.img");
    make_image(&img, "EMPTY");

    write_root_file(&img, "empty.bin", b"");
    maybe_fsck(&img);

    let got = read_root_file(&img, "empty.bin");
    assert!(
        got.is_empty(),
        "expected empty file, got {} bytes",
        got.len()
    );
}

#[test]
fn roundtrip_multi_cluster_file() {
    let tmp = TempDir::new().unwrap();
    let img = tmp.path().join("large.img");
    make_image(&img, "LARGE");

    // 1 MiB of pseudo-random bytes — guaranteed to span multiple clusters
    // for any reasonable exFAT cluster size on a 32 MiB image.
    let payload: Vec<u8> = (0..(1024 * 1024)).map(|i| (i * 31 + 7) as u8).collect();
    write_root_file(&img, "blob.bin", &payload);
    maybe_fsck(&img);

    let got = read_root_file(&img, "blob.bin");
    assert_eq!(got.len(), payload.len(), "size mismatch");
    assert_eq!(got, payload, "content mismatch");
}

#[test]
fn roundtrip_unicode_name() {
    let tmp = TempDir::new().unwrap();
    let img = tmp.path().join("unicode.img");
    make_image(&img, "UNICODE");

    let name = "café-日本語-🦀.txt";
    let payload = "exFAT supports Unicode filenames up to 255 chars.".as_bytes();
    write_root_file(&img, name, payload);
    maybe_fsck(&img);

    let got = read_root_file(&img, name);
    assert_eq!(got, payload);
}

#[test]
fn roundtrip_delete_and_recreate() {
    let tmp = TempDir::new().unwrap();
    let img = tmp.path().join("reuse.img");
    make_image(&img, "REUSE");

    // Create, then delete: exercises the bitmap free path.
    {
        let file = open_image(&img);
        let fs = ExFatVolume::open(file).expect("open");
        let entry = {
            let root = fs.root_dir();
            fs.create_file(&root, "victim.bin").expect("create")
        };
        let mut w = fs.write_file(&entry).expect("write_file");
        w.write_all(&vec![0xAB; 64 * 1024]).expect("write");
        w.finish().expect("finish");

        // Re-read entry after write to pick up cluster/size updates.
        let entry = {
            let root = fs.root_dir();
            root.find("victim.bin").expect("find").expect("present")
        };
        fs.delete(&entry).expect("delete");
    }
    maybe_fsck(&img);

    // Recreate with new content — should reuse freed clusters cleanly.
    let payload = b"reborn";
    write_root_file(&img, "phoenix.txt", payload);
    maybe_fsck(&img);

    let got = read_root_file(&img, "phoenix.txt");
    assert_eq!(got, payload);
}

#[test]
fn roundtrip_multiple_files() {
    let tmp = TempDir::new().unwrap();
    let img = tmp.path().join("multi.img");
    make_image(&img, "MULTI");

    let files: &[(&str, &[u8])] = &[
        ("a.txt", b"alpha"),
        ("b.txt", b"bravo"),
        ("c.bin", &[0xCC; 8192]),
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

/// The stream extension's DataLength is the file's size, not the clusters
/// allocated for it: other implementations take DataLength as the size, and a
/// cluster-rounded one shows a small file with a tail of zeros.
#[test]
fn data_length_is_the_file_size_not_the_allocation() {
    let dir = TempDir::new().expect("tempdir");
    let image_path = dir.path().join("length.img");
    make_image(&image_path, "LENGTH");
    write_root_file(&image_path, "small.txt", b"Hello, exFAT!");

    let fs = ExFatVolume::open(open_image(&image_path)).expect("open exFAT");
    let entry = fs
        .root_dir()
        .find("small.txt")
        .expect("find")
        .expect("small.txt exists");
    assert_eq!(entry.valid_data_length, 13);
    assert_eq!(entry.data_length, 13, "DataLength must be the file's size");

    // Truncating keeps the rule.
    fs.truncate(&entry, 5).expect("truncate");
    let entry = fs
        .root_dir()
        .find("small.txt")
        .expect("find")
        .expect("small.txt exists");
    assert_eq!(entry.valid_data_length, 5);
    assert_eq!(entry.data_length, 5);
}

/// An empty file still has a stream extension that allows allocation (exFAT
/// 7.6.2) with no first cluster; fsck_exfat reports "no stream allocation"
/// otherwise. NoFatChain stays clear: exfatprogs rejects an empty file that
/// claims a contiguous allocation ("empty, but has no Fat chain").
#[test]
fn an_empty_file_keeps_allocation_possible() {
    let dir = TempDir::new().expect("tempdir");
    let image_path = dir.path().join("empty.img");
    make_image(&image_path, "EMPTY");
    write_root_file(&image_path, "empty.txt", b"");

    let fs = ExFatVolume::open(open_image(&image_path)).expect("open exFAT");
    let entry = fs
        .root_dir()
        .find("empty.txt")
        .expect("find")
        .expect("empty.txt exists");
    assert_eq!(entry.data_length, 0);
    assert_eq!(entry.first_cluster, 0);
    assert!(
        !entry.no_fat_chain,
        "an empty file must not claim a contiguous allocation"
    );

    let flags = root_stream_flags(&image_path, fs.info().root_cluster, |stream| {
        u64::from_le_bytes(stream[24..32].try_into().unwrap()) == 0
            && u32::from_le_bytes(stream[20..24].try_into().unwrap()) == 0
    });
    assert_eq!(flags & 0x01, 0x01, "AllocationPossible must be set");
    assert_eq!(flags & 0x02, 0x00, "NoFatChain must be clear");
    assert_eq!(flags, 0x01);
}

/// GeneralSecondaryFlags of the first stream extension in the root directory
/// cluster that `matches` the raw 32-byte entry.
fn root_stream_flags(image_path: &Path, root_cluster: u32, matches: impl Fn(&[u8]) -> bool) -> u8 {
    let fs = ExFatVolume::open(open_image(image_path)).expect("open exFAT");
    let info = fs.info();
    let offset = info.cluster_to_offset(root_cluster);
    let mut cluster = vec![0u8; info.bytes_per_cluster];
    let mut file = open_image(image_path);
    file.seek(std::io::SeekFrom::Start(offset)).unwrap();
    std::io::Read::read_exact(&mut file, &mut cluster).expect("read root cluster");
    cluster
        .chunks_exact(32)
        .find(|raw| raw[0] == 0xC0 && matches(raw))
        .map(|raw| raw[1])
        .expect("stream extension entry")
}
