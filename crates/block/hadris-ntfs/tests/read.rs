//! Volumes made by ntfsprogs (`mkntfs`, `ntfscp`) and populated through
//! `ntfs-3g`, read back. Each test is skipped when its tools are missing;
//! `scripts/test-ntfs.sh` runs them in a container that has them.

use hadris_fs::MountOptions;
use hadris_fs::sync::FileSystem;
use hadris_fs::{ErrorKind, FileType, Name};
use hadris_ntfs::sync::NtfsFs;
use hadris_storage::host::FileDevice;

mod common;
#[path = "support/paths.rs"]
mod paths;
use common::NtfsTestImage;
use paths::PathOps;

macro_rules! require_image {
    ($label:expr) => {
        match NtfsTestImage::new($label) {
            Some(img) => img,
            None => return,
        }
    };
}

fn open(img: &NtfsTestImage) -> NtfsFs<FileDevice> {
    NtfsFs::mount(FileDevice::open(img.path()).unwrap(), MountOptions::new()).unwrap()
}

fn names(fs: &mut NtfsFs<FileDevice>, path: &str) -> Vec<String> {
    fs.entries(path)
        .unwrap()
        .iter()
        .map(|entry| entry.name().to_str().unwrap().to_string())
        .collect()
}

fn name(s: &str) -> &Name {
    Name::new(s)
}

#[test]
fn open_blank_volume() {
    let img = require_image!("BlankVol");
    let mut fs = open(&img);
    assert!(fs.cluster_size() >= 512 && fs.cluster_size().is_power_of_two());
    assert_ne!(fs.volume_serial(), 0);
    assert!(fs.total_sectors() > 0);
    assert!(fs.mft_record_size() >= 512);
    let mut label = [0u8; 64];
    assert_eq!(fs.label(&mut label).unwrap(), Some("BlankVol"));
    let stats = fs.statfs().unwrap();
    assert!(stats.free_blocks() > 0 && stats.free_blocks() < stats.total_blocks());
    assert!(names(&mut fs, "/").is_empty());
}

#[test]
fn metadata_files_are_hidden_but_found() {
    let img = require_image!("SysFiles");
    let mut fs = open(&img);
    let root = fs.root();
    for system in ["$MFT", "$MFTMirr", "$Volume", "$Boot", "$Bitmap", "$UpCase"] {
        let node = fs.lookup(root, name(system)).unwrap();
        assert_eq!(fs.stat(node).unwrap().file_type(), FileType::File);
    }
    let extend = fs.lookup(root, name("$Extend")).unwrap();
    assert_eq!(fs.stat(extend).unwrap().file_type(), FileType::Dir);
    assert!(fs.lookup(root, name("$mft")).is_ok());
    assert!(fs.lookup(root, name("$upcase")).is_ok());
}

#[test]
fn files_read_back() {
    let img = require_image!("Files");
    let large: Vec<u8> = (0..=255u8).cycle().take(65536).collect();
    let odd: Vec<u8> = (0..8192u32).map(|i| (i % 251) as u8).collect();
    let files: &[(&str, &[u8])] = &[
        ("hello.txt", b"Hello, NTFS!"),
        ("large.bin", &large),
        ("empty.txt", b""),
        ("stream.bin", &odd),
        ("echo.bin", &[0xDE, 0xAD, 0xBE, 0xEF]),
    ];
    for (file, content) in files {
        assert!(img.add_file(file, content), "ntfscp failed for {file}");
    }
    let mut fs = open(&img);
    let mut listed = names(&mut fs, "/");
    listed.sort();
    let mut expected: Vec<_> = files.iter().map(|(file, _)| file.to_string()).collect();
    expected.sort();
    assert_eq!(listed, expected);
    for (file, content) in files {
        let path = format!("/{file}");
        assert_eq!(fs.read_to_vec(&path).unwrap(), *content, "{file}");
        assert_eq!(fs.metadata(&path).unwrap().len(), content.len() as u64);
    }
    let root = fs.root();
    let node = fs.lookup(root, name("stream.bin")).unwrap();
    let mut collected = Vec::new();
    let mut buf = [0u8; 137];
    loop {
        let n = fs.read(node, collected.len() as u64, &mut buf).unwrap();
        if n == 0 {
            break;
        }
        collected.extend_from_slice(&buf[..n]);
    }
    assert_eq!(collected, odd);
    let meta = fs.stat(node).unwrap();
    assert!(meta.modified().is_some());
    assert_eq!(meta.nlink(), 1);
}

/// ntfsprogs and ntfs-3g create names in the case-sensitive POSIX
/// namespace; the metadata files have Win32 names, which fold case.
#[test]
fn names_fold_case_only_in_the_win32_namespace() {
    let img = require_image!("CaseFind");
    assert!(img.add_file("CamelCase.Txt", b"data"));
    assert!(img.add_file("R\u{E9}sum\u{E9}.Txt", b"data"));
    let mut fs = open(&img);
    let root = fs.root();
    assert!(fs.lookup(root, name("CamelCase.Txt")).is_ok());
    assert!(fs.lookup(root, name("R\u{E9}sum\u{E9}.Txt")).is_ok());
    for query in [
        "camelcase.txt",
        "CAMELCASE.TXT",
        "R\u{C9}SUM\u{C9}.TXT",
        "missing.txt",
    ] {
        assert_eq!(
            fs.lookup(root, name(query)).unwrap_err().kind(),
            ErrorKind::NotFound,
            "{query}"
        );
    }
    let upcase = fs.lookup(root, name("$UpCase")).unwrap();
    assert_eq!(fs.lookup(root, name("$upcase")).unwrap(), upcase);
    assert_eq!(fs.lookup(root, name("$UPCASE")).unwrap(), upcase);
}

#[test]
fn long_and_supplementary_names() {
    let img = require_image!("Names");
    let long = "This is a very long filename that exceeds the 8.3 DOS limit.txt";
    let crab = "report-\u{1F980}.txt";
    assert!(img.add_file(long, b"long name content"));
    assert!(img.add_file(crab, b"unicode content"));
    let mut fs = open(&img);
    let mut listed = names(&mut fs, "/");
    listed.sort();
    assert_eq!(listed, [long, crab]);
    assert_eq!(
        fs.read_to_vec(&format!("/{long}")).unwrap(),
        b"long name content"
    );
    assert_eq!(
        fs.read_to_vec(&format!("/{crab}")).unwrap(),
        b"unicode content"
    );
}

#[test]
fn contract_holds_on_a_blank_volume() {
    let img = require_image!("Contract");
    assert!(img.add_file("a.txt", b"a"));
    let mut fs = open(&img);
    hadris_fs::sync::contract::check_read_only(&mut fs).unwrap();
}

#[test]
fn nested_directories() {
    let img = require_image!("Nested");
    let mounted = img.with_mounted(|mnt| {
        std::fs::create_dir_all(mnt.join("a/b/c")).unwrap();
        std::fs::write(mnt.join("a/readme.md"), "# A").unwrap();
        std::fs::write(mnt.join("a/b/data.bin"), [0xCA, 0xFE]).unwrap();
        std::fs::write(mnt.join("a/b/c/deep.txt"), "deep content").unwrap();
        std::fs::hard_link(mnt.join("a/readme.md"), mnt.join("a/b/linked.md")).unwrap();
    });
    if mounted.is_none() {
        return;
    }
    let mut fs = open(&img);
    let mut a = names(&mut fs, "/a");
    a.sort();
    assert_eq!(a, ["b", "readme.md"]);
    assert_eq!(fs.metadata("/a/b").unwrap().file_type(), FileType::Dir);
    assert_eq!(fs.read_to_vec("/a/b/data.bin").unwrap(), [0xCA, 0xFE]);
    assert_eq!(fs.read_to_vec("/a/b/c/deep.txt").unwrap(), b"deep content");
    assert_eq!(fs.metadata("/a/readme.md").unwrap().nlink(), 2);
    let root = fs.root();
    let a = fs.lookup(root, name("a")).unwrap();
    let readme = fs.lookup(a, name("readme.md")).unwrap();
    let b = fs.lookup(a, name("b")).unwrap();
    assert_eq!(fs.lookup(b, name("linked.md")).unwrap(), readme);
    assert_eq!(fs.parent(b).unwrap(), a);
    assert_eq!(fs.parent(a).unwrap(), root);
    assert_eq!(
        fs.metadata("/a/nope").unwrap_err().kind(),
        ErrorKind::NotFound
    );
    assert_eq!(
        fs.metadata("/a/readme.md/x").unwrap_err().kind(),
        ErrorKind::NotADirectory
    );
    hadris_fs::sync::contract::check_read_only(&mut fs).unwrap();
}

#[test]
fn large_directory_lists_every_entry() {
    let img = require_image!("LargeDir");
    let mounted = img.with_mounted(|mnt| {
        std::fs::create_dir(mnt.join("many")).unwrap();
        for index in 0..500 {
            std::fs::write(mnt.join(format!("many/file-{index:03}.txt")), [index as u8]).unwrap();
        }
    });
    if mounted.is_none() {
        return;
    }
    let mut fs = open(&img);
    let mut listed = names(&mut fs, "/many");
    listed.sort();
    let expected: Vec<_> = (0..500)
        .map(|index| format!("file-{index:03}.txt"))
        .collect();
    assert_eq!(listed, expected);
    assert_eq!(
        fs.read_to_vec("/many/file-499.txt").unwrap(),
        [(499 % 256) as u8]
    );
    hadris_fs::sync::contract::check_read_only(&mut fs).unwrap();
}

#[test]
fn named_streams_read_back() {
    let img = require_image!("Streams");
    let mounted = img.with_mounted(|mnt| {
        std::fs::write(mnt.join("file.txt"), "main").unwrap();
        NtfsTestImage::setfattr(&mnt.join("file.txt"), "user.extra", "side data")
    });
    if mounted != Some(true) {
        return;
    }
    let mut fs = open(&img);
    let root = fs.root();
    let file = fs.lookup(root, name("file.txt")).unwrap();
    let mut streams = Vec::new();
    fs.streams(file, |stream, len| streams.push((stream.to_string(), len)))
        .unwrap();
    assert_eq!(streams, [("extra".to_string(), 9)]);
    let mut buf = [0u8; 32];
    let n = fs.read_stream_at(file, "extra", 0, &mut buf).unwrap();
    assert_eq!(&buf[..n], b"side data");
    assert_eq!(fs.read_to_vec("/file.txt").unwrap(), b"main");
}

/// A sparse file with 1500 data fragments has more mapping pairs than a
/// record holds, and 100 hard links more names, so both spill into
/// extension records behind an `$ATTRIBUTE_LIST`.
#[test]
fn attribute_lists_on_a_real_volume() {
    let Some(img) = NtfsTestImage::with_size("Lists", 64 * 1024 * 1024) else {
        return;
    };
    const STRIDE: u64 = 16 * 1024;
    const FRAGMENTS: u64 = 1500;
    let mounted = img.with_mounted(|mnt| {
        use std::io::{Seek, SeekFrom, Write};
        let mut file = std::fs::File::create(mnt.join("sparse.bin")).unwrap();
        for i in 0..FRAGMENTS {
            file.seek(SeekFrom::Start(i * STRIDE)).unwrap();
            file.write_all(&[(i % 251) as u8 + 1; 7]).unwrap();
        }
        drop(file);
        std::fs::write(mnt.join("linked.txt"), "shared").unwrap();
        for i in 0..100 {
            let link = format!("a-hard-link-with-a-rather-long-name-{i:03}.txt");
            std::fs::hard_link(mnt.join("linked.txt"), mnt.join(link)).unwrap();
        }
    });
    if mounted.is_none() {
        return;
    }
    let mut fs = open(&img);
    let data = fs.read_to_vec("/sparse.bin").unwrap();
    assert_eq!(data.len() as u64, (FRAGMENTS - 1) * STRIDE + 7);
    for i in 0..FRAGMENTS {
        let at = (i * STRIDE) as usize;
        assert_eq!(&data[at..at + 7], &[(i % 251) as u8 + 1; 7], "fragment {i}");
        if i + 1 < FRAGMENTS {
            assert!(data[at + 7..at + STRIDE as usize].iter().all(|&b| b == 0));
        }
    }
    let meta = fs.metadata("/linked.txt").unwrap();
    assert_eq!(meta.nlink(), 101);
    assert_eq!(
        fs.read_to_vec("/a-hard-link-with-a-rather-long-name-099.txt")
            .unwrap(),
        b"shared"
    );
    hadris_fs::sync::contract::check_read_only(&mut fs).unwrap();
}
