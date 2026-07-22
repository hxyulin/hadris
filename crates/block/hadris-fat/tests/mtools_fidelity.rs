//! External-tool fidelity checks against `mtools`.
//!
//! `fsck.fat` validates structure but not filename/content fidelity. These
//! tests write an image with hadris and read it back with `mdir`/`mtype` so a
//! reference FAT driver confirms what hadris produced — in particular that a
//! lowercase 8.3 name (stored via the NT `DIR_NTRes` case bits) is presented in
//! its original case.
//!
//! All tests skip cleanly when `mtools` is not installed, so they are inert
//! locally and only assert in CI where the tool is present.

#![cfg(feature = "write")]

use std::io::Seek;
use std::path::Path;
use std::process::Command;

use hadris_fat::format::{FatFormatOptions, FatTypeSelection, FatVolumeFormatter};
use hadris_fat::{FatVolume, FatVolumeWriteExt};

fn mtools_available() -> bool {
    Command::new("mdir")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// hadris writes the image; the closure is invoked to populate it.
fn make_image_with(path: &Path, size: u64, files: &[(&str, &[u8])]) {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .expect("create image");
    file.set_len(size).expect("set length");
    let options = FatFormatOptions::new(size)
        .volume_label("MTOOLS")
        .fat_type(FatTypeSelection::Fat16);
    drop(FatVolumeFormatter::format(file, options).expect("format"));

    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .expect("reopen image");
    file.seek(std::io::SeekFrom::Start(0)).unwrap();
    let fs = FatVolume::open(file).expect("open FAT");
    for (name, contents) in files {
        let entry = fs.create_file(&fs.root_dir(), name).expect("create_file");
        let mut writer = fs.write_file(&entry).expect("write_file");
        writer.write(contents).expect("write");
        writer.finish().expect("finish");
    }
    // Dropping `fs` flushes its underlying File handle.
    drop(fs);
}

/// `-i <image>` drives mtools against a raw FAT image, using `::` as the drive.
fn mtools(image: &Path, args: &[&str]) -> String {
    let mut command = Command::new(args[0]);
    command.arg("-i").arg(image);
    command.args(&args[1..]);
    let output = command.output().expect("run mtools");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn mtools_reads_lowercase_short_name_in_original_case() {
    if !mtools_available() {
        eprintln!("note: mtools not available, skipping external validation");
        return;
    }
    let tmp = tempfile::TempDir::new().unwrap();
    let img = tmp.path().join("case.img");
    // FAT16 requires at least 4 MiB; give some headroom above the minimum.
    make_image_with(&img, 8 * 1024 * 1024, &[("readme.txt", b"hadris")]);

    // `mdir ::` lists the root directory. mtools prints 8.3 short names in
    // column form ("readme   txt", no dot) and honors the NT case bits, so the
    // base and extension must appear lowercased — and specifically not as the
    // uppercase on-disk form a driver ignoring the case bits would show.
    let listing = mtools(&img, &["mdir", "::"]);
    assert!(
        listing.contains("readme") && listing.contains("txt"),
        "mtools should list the short name lowercased; got:\n{listing}"
    );
    assert!(
        !listing.contains("README"),
        "mtools should honor the NT case bits and not uppercase the name; got:\n{listing}"
    );

    // `mtype ::readme.txt` prints the file's contents.
    let body = mtools(&img, &["mtype", "::readme.txt"]);
    assert!(
        body.contains("hadris"),
        "mtools should read back the file contents; got:\n{body}"
    );
}
