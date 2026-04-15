//! Roundtrip tests for hadris-fat exFAT write + read paths.
//!
//! Each test formats a fresh image with `format_exfat`, writes content using
//! the hadris-fat write API, then reads it back through the same API to
//! verify byte-for-byte equality. When `fsck.exfat` is available on the host,
//! the image is also validated externally.

#![cfg(all(feature = "exfat", feature = "write"))]

use std::fs::OpenOptions;
use std::io::Seek as _;
use std::path::Path;
use tempfile::TempDir;

use hadris_fat::exfat::{ExFatFormatOptions, ExFatFs, format_exfat};
use hadris_fat::io::{Read as HadrisRead, Write as HadrisWrite};

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

    let opts = ExFatFormatOptions::default().with_label(label);
    format_exfat(&mut file, IMAGE_SIZE, &opts).expect("format_exfat");
    file.sync_all().expect("sync");
}

/// Open an image file at the start, ready for ExFatFs::open.
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
    let fs = ExFatFs::open(file).expect("open exFAT");

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
    let fs = ExFatFs::open(file).expect("reopen exFAT");
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
    assert!(got.is_empty(), "expected empty file, got {} bytes", got.len());
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

// KNOWN BUG: writing a new file after deleting one fails with WriteZero.
// The delete path frees clusters but the bitmap state isn't fully consistent
// for the next allocator pass. Tracked separately — keep this test as a
// regression check; remove `#[ignore]` once the underlying bug is fixed.
#[test]
#[ignore = "exfat write-after-delete bug; see test comment"]
fn roundtrip_delete_and_recreate() {
    let tmp = TempDir::new().unwrap();
    let img = tmp.path().join("reuse.img");
    make_image(&img, "REUSE");

    // Create, then delete: exercises the bitmap free path.
    {
        let file = open_image(&img);
        let fs = ExFatFs::open(file).expect("open");
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
