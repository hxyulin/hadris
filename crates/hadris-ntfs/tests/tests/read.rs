//! Integration tests for hadris-ntfs read-only support.
//!
//! Uses ntfsprogs (`mkntfs`, `ntfscp`) and optionally `ntfs-3g` (FUSE) to
//! create test NTFS images, then verifies that hadris-ntfs reads them back
//! correctly.

use std::fs::File;

use hadris_ntfs::sync::{NtfsFs, NtfsFsReadExt};
use hadris_ntfs_tests::NtfsTestImage;

macro_rules! require_image {
    ($label:expr) => {
        match NtfsTestImage::new($label) {
            Some(img) => img,
            None => return,
        }
    };
}

// ---------------------------------------------------------------------------
// Blank-volume tests (only mkntfs required)
// ---------------------------------------------------------------------------

#[test]
fn open_blank_volume() {
    let img = require_image!("BlankVol");
    let file = File::open(img.path()).unwrap();
    let _fs = NtfsFs::open(file).unwrap();
}

#[test]
fn volume_metadata() {
    let img = require_image!("MetaVol");
    let file = File::open(img.path()).unwrap();
    let fs = NtfsFs::open(file).unwrap();

    assert!(fs.cluster_size() >= 512, "cluster size too small");
    assert!(fs.cluster_size().is_power_of_two(), "cluster size not power-of-two");
    assert_ne!(fs.volume_serial(), 0, "serial should be non-zero");
    assert!(fs.total_sectors() > 0);
    assert!(fs.mft_record_size() >= 512);
}

#[test]
fn root_dir_lists_system_metafiles() {
    let img = require_image!("RootDir");
    let file = File::open(img.path()).unwrap();
    let fs = NtfsFs::open(file).unwrap();

    let root = fs.root_dir();
    let entries = root.entries().unwrap();

    let names: Vec<&str> = entries.iter().map(|e| e.name()).collect();

    // Every NTFS volume has these metafiles in the root directory.
    for expected in ["$MFT", "$MFTMirr", "$Volume", "$Boot", "$Bitmap", "$UpCase"] {
        assert!(
            names.contains(&expected),
            "root dir missing {expected}; found: {names:?}"
        );
    }
}

#[test]
fn root_system_files_are_not_regular_files() {
    let img = require_image!("SysFiles");
    let file = File::open(img.path()).unwrap();
    let fs = NtfsFs::open(file).unwrap();

    let entries = fs.root_dir().entries().unwrap();

    // $MFT is a regular (non-directory) metafile
    let mft = entries.iter().find(|e| e.name() == "$MFT").unwrap();
    assert!(mft.is_file());

    // $Extend is a directory container for metadata extensions
    if let Some(extend) = entries.iter().find(|e| e.name() == "$Extend") {
        assert!(extend.is_directory());
    }
}

// ---------------------------------------------------------------------------
// File-read tests (mkntfs + ntfscp required)
// ---------------------------------------------------------------------------

#[test]
fn read_small_resident_file() {
    let img = require_image!("SmallFile");
    let content = b"Hello, NTFS!";
    assert!(img.add_file("hello.txt", content), "ntfscp failed");

    let file = File::open(img.path()).unwrap();
    let fs = NtfsFs::open(file).unwrap();

    let root = fs.root_dir();
    let entries = root.entries().unwrap();

    let entry = entries.iter().find(|e| e.name() == "hello.txt");
    assert!(entry.is_some(), "hello.txt not found in root dir");

    let entry = entry.unwrap();
    assert!(entry.is_file());

    let mut reader = fs.read_file(entry).unwrap();
    let data = reader.read_to_vec().unwrap();
    assert_eq!(data, content);
}

#[test]
fn read_large_nonresident_file() {
    let img = require_image!("LargeFile");

    // 64 KiB of repeating bytes — guaranteed non-resident.
    let content: Vec<u8> = (0..=255u8).cycle().take(65536).collect();
    assert!(img.add_file("large.bin", &content), "ntfscp failed");

    let file = File::open(img.path()).unwrap();
    let fs = NtfsFs::open(file).unwrap();

    let entry = fs
        .root_dir()
        .entries()
        .unwrap()
        .into_iter()
        .find(|e| e.name() == "large.bin")
        .expect("large.bin not found");

    assert!(entry.is_file());

    let mut reader = fs.read_file(&entry).unwrap();
    assert_eq!(reader.size(), 65536);

    let data = reader.read_to_vec().unwrap();
    assert_eq!(data.len(), content.len());
    assert_eq!(data, content);
}

#[test]
fn read_empty_file() {
    let img = require_image!("EmptyFile");
    assert!(img.add_file("empty.txt", b""), "ntfscp failed");

    let file = File::open(img.path()).unwrap();
    let fs = NtfsFs::open(file).unwrap();

    let entry = fs
        .root_dir()
        .entries()
        .unwrap()
        .into_iter()
        .find(|e| e.name() == "empty.txt")
        .expect("empty.txt not found");

    let mut reader = fs.read_file(&entry).unwrap();
    assert_eq!(reader.size(), 0);
    assert_eq!(reader.remaining(), 0);
    let data = reader.read_to_vec().unwrap();
    assert!(data.is_empty());
}

#[test]
fn read_file_incrementally() {
    let img = require_image!("IncrRead");

    let content: Vec<u8> = (0..8192u32).map(|i| (i % 251) as u8).collect();
    assert!(img.add_file("stream.bin", &content), "ntfscp failed");

    let file = File::open(img.path()).unwrap();
    let fs = NtfsFs::open(file).unwrap();

    let entry = fs
        .root_dir()
        .entries()
        .unwrap()
        .into_iter()
        .find(|e| e.name() == "stream.bin")
        .expect("stream.bin not found");

    let mut reader = fs.read_file(&entry).unwrap();
    let mut collected = Vec::new();
    let mut buf = [0u8; 137]; // odd chunk size to exercise boundary handling
    loop {
        let n = reader.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        collected.extend_from_slice(&buf[..n]);
    }
    assert_eq!(collected.len(), content.len());
    assert_eq!(collected, content);
}

#[test]
fn find_file_case_insensitive() {
    let img = require_image!("CaseFind");
    assert!(img.add_file("CamelCase.Txt", b"data"), "ntfscp failed");

    let file = File::open(img.path()).unwrap();
    let fs = NtfsFs::open(file).unwrap();
    let root = fs.root_dir();

    // Exact case
    assert!(root.find("CamelCase.Txt").unwrap().is_some());
    // Lower case
    assert!(root.find("camelcase.txt").unwrap().is_some());
    // Upper case
    assert!(root.find("CAMELCASE.TXT").unwrap().is_some());
}

#[test]
fn find_nonexistent_returns_none() {
    let img = require_image!("NoFile");
    let file = File::open(img.path()).unwrap();
    let fs = NtfsFs::open(file).unwrap();

    let result = fs.root_dir().find("does_not_exist.txt").unwrap();
    assert!(result.is_none());
}

#[test]
fn long_filename() {
    let img = require_image!("LongName");
    let name = "This is a very long filename that exceeds the 8.3 DOS limit.txt";
    assert!(img.add_file(name, b"long name content"), "ntfscp failed");

    let file = File::open(img.path()).unwrap();
    let fs = NtfsFs::open(file).unwrap();

    let entry = fs
        .root_dir()
        .entries()
        .unwrap()
        .into_iter()
        .find(|e| e.name() == name)
        .expect("long filename not found");

    assert!(entry.is_file());
    let mut reader = fs.read_file(&entry).unwrap();
    let data = reader.read_to_vec().unwrap();
    assert_eq!(data, b"long name content");
}

#[test]
fn multiple_files_in_root() {
    let img = require_image!("MultiFile");

    let files: &[(&str, &[u8])] = &[
        ("alpha.txt", b"aaa"),
        ("bravo.txt", b"bbb"),
        ("charlie.txt", b"ccc"),
        ("delta.dat", b"ddd"),
        ("echo.bin", &[0xDE, 0xAD, 0xBE, 0xEF]),
    ];

    for (name, content) in files {
        assert!(img.add_file(name, content), "ntfscp failed for {name}");
    }

    let file = File::open(img.path()).unwrap();
    let fs = NtfsFs::open(file).unwrap();
    let entries = fs.root_dir().entries().unwrap();

    for (name, expected_content) in files {
        let entry = entries
            .iter()
            .find(|e| e.name() == *name)
            .unwrap_or_else(|| panic!("{name} not found"));

        let mut reader = fs.read_file(entry).unwrap();
        let data = reader.read_to_vec().unwrap();
        assert_eq!(&data, expected_content, "content mismatch for {name}");
    }
}

#[test]
fn open_directory_as_file_fails() {
    let img = require_image!("DirAsFile");
    let file = File::open(img.path()).unwrap();
    let fs = NtfsFs::open(file).unwrap();

    let entries = fs.root_dir().entries().unwrap();
    if let Some(dir_entry) = entries.iter().find(|e| e.is_directory()) {
        let result = fs.read_file(dir_entry);
        assert!(result.is_err(), "read_file on a directory should fail");
    }
}

// ---------------------------------------------------------------------------
// Directory tests (requires ntfs-3g FUSE mount)
// ---------------------------------------------------------------------------

#[test]
fn subdirectory_listing() {
    let img = require_image!("SubDir");

    let mounted = img.with_mounted(|mnt| {
        std::fs::create_dir(mnt.join("mydir")).unwrap();
        std::fs::write(mnt.join("mydir/one.txt"), "1").unwrap();
        std::fs::write(mnt.join("mydir/two.txt"), "22").unwrap();
        std::fs::write(mnt.join("mydir/three.txt"), "333").unwrap();
    });
    if mounted.is_none() {
        eprintln!("SKIP: ntfs-3g FUSE mount not available");
        return;
    }

    let file = File::open(img.path()).unwrap();
    let fs = NtfsFs::open(file).unwrap();

    // Root should contain "mydir"
    let root_entries = fs.root_dir().entries().unwrap();
    let dir_entry = root_entries
        .iter()
        .find(|e| e.name() == "mydir")
        .expect("mydir not in root");
    assert!(dir_entry.is_directory());

    // Open the subdirectory and list entries
    let subdir = fs.root_dir().open_dir("mydir").unwrap();
    let sub_entries = subdir.entries().unwrap();
    let sub_names: Vec<&str> = sub_entries.iter().map(|e| e.name()).collect();

    assert!(sub_names.contains(&"one.txt"), "missing one.txt: {sub_names:?}");
    assert!(sub_names.contains(&"two.txt"), "missing two.txt: {sub_names:?}");
    assert!(sub_names.contains(&"three.txt"), "missing three.txt: {sub_names:?}");

    // Read content through the subdirectory handle
    let mut reader = subdir.open_file("two.txt").unwrap();
    let data = reader.read_to_vec().unwrap();
    assert_eq!(data, b"22");
}

#[test]
fn nested_directories_and_open_path() {
    let img = require_image!("Nested");

    let mounted = img.with_mounted(|mnt| {
        std::fs::create_dir_all(mnt.join("a/b/c")).unwrap();
        std::fs::write(mnt.join("a/readme.md"), "# A").unwrap();
        std::fs::write(mnt.join("a/b/data.bin"), &[0xCA, 0xFE]).unwrap();
        std::fs::write(mnt.join("a/b/c/deep.txt"), "deep content").unwrap();
    });
    if mounted.is_none() {
        eprintln!("SKIP: ntfs-3g FUSE mount not available");
        return;
    }

    let file = File::open(img.path()).unwrap();
    let fs = NtfsFs::open(file).unwrap();

    // Traverse step by step
    let dir_a = fs.root_dir().open_dir("a").unwrap();
    let a_entries = dir_a.entries().unwrap();
    assert!(a_entries.iter().any(|e| e.name() == "readme.md"));
    assert!(a_entries.iter().any(|e| e.name() == "b" && e.is_directory()));

    let dir_b = dir_a.open_dir("b").unwrap();
    let b_entries = dir_b.entries().unwrap();
    assert!(b_entries.iter().any(|e| e.name() == "data.bin"));
    assert!(b_entries.iter().any(|e| e.name() == "c" && e.is_directory()));

    // Read file in nested dir
    let mut reader = dir_b.open_file("data.bin").unwrap();
    assert_eq!(reader.read_to_vec().unwrap(), &[0xCA, 0xFE]);

    // Use open_path for deep navigation
    let deep = fs.open_path("a/b/c/deep.txt").unwrap();
    assert!(deep.is_file());
    assert_eq!(deep.name(), "deep.txt");

    let mut reader = fs.read_file(&deep).unwrap();
    assert_eq!(reader.read_to_vec().unwrap(), b"deep content");

    // open_path with backslash separators
    let deep2 = fs.open_path("a\\b\\c\\deep.txt").unwrap();
    assert_eq!(deep2.name(), "deep.txt");
}

#[test]
fn open_nonexistent_path_fails() {
    let img = require_image!("NoPath");

    let mounted = img.with_mounted(|mnt| {
        std::fs::create_dir(mnt.join("real")).unwrap();
        std::fs::write(mnt.join("real/file.txt"), "x").unwrap();
    });
    if mounted.is_none() {
        eprintln!("SKIP: ntfs-3g FUSE mount not available");
        return;
    }

    let file = File::open(img.path()).unwrap();
    let fs = NtfsFs::open(file).unwrap();

    assert!(fs.open_path("real/file.txt").is_ok());
    assert!(fs.open_path("real/nope.txt").is_err());
    assert!(fs.open_path("fake/file.txt").is_err());
}

#[test]
fn open_file_as_directory_fails() {
    let img = require_image!("FileAsDir");
    assert!(img.add_file("plain.txt", b"x"), "ntfscp failed");

    let file = File::open(img.path()).unwrap();
    let fs = NtfsFs::open(file).unwrap();

    let result = fs.root_dir().open_dir("plain.txt");
    assert!(result.is_err(), "open_dir on a file should fail");
}
