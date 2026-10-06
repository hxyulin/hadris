//! The read-only `FatFs` driver on images filled through `FatFs`.

#[path = "common/fatfs.rs"]
mod common;
use common::{FsPaths, VolumePaths};
use hadris_fs::{Ascii, MountOptions};

use std::collections::BTreeMap;
use std::io::Read as _;

use common::{CASES, Case, Device, INNER, INNER_FILES, KANJI_NAME, LONG_NAME, UNICODE_NAME};
use hadris_fat::FatKind;
use hadris_fat::sync::FatFs;
use hadris_fs::sync::{FileSystem, Volume};
use hadris_fs::{
    Attributes, CaseRule, Charset, DirCursor, ErrorKind, FileType, Name, NodeId, OpenOptions,
    SetAttr,
};
use hadris_storage::{BlockSize, MemDevice};

type Fs = FatFs<Device>;

fn open(case: Case, image: Vec<u8>) -> Fs {
    FatFs::mount(common::device(case, image), MountOptions::new()).unwrap()
}

fn name(text: &str) -> &Name {
    Name::new(text)
}

fn swap_case(text: &str) -> String {
    text.chars()
        .flat_map(|ch| -> Vec<char> {
            if ch.is_uppercase() {
                ch.to_lowercase().collect()
            } else {
                ch.to_uppercase().collect()
            }
        })
        .collect()
}

fn list(fs: &mut Fs, dir: NodeId) -> Vec<(String, hadris_fs::DirEntry)> {
    let mut cursor = DirCursor::START;
    let mut out = Vec::new();
    while let Some(entry) = fs.readdir(dir, cursor).unwrap() {
        cursor = entry.next_cursor();
        out.push((entry.name().to_str().unwrap().to_owned(), entry));
    }
    out
}

fn read_all(fs: &mut Fs, node: NodeId) -> Vec<u8> {
    let mut out = Vec::new();
    let mut chunk = [0u8; 777];
    loop {
        let n = fs.read(node, out.len() as u64, &mut chunk).unwrap();
        if n == 0 {
            return out;
        }
        out.extend_from_slice(&chunk[..n]);
    }
}

/// The files the fixture holds and their contents, by path.
fn expected_files() -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut add = |path: &str, data: &[u8]| files.insert(path.to_owned(), data.to_vec());
    add("/README.TXT", b"hello fat");
    add("/lower.txt", b"lowercase short name");
    add(&format!("/{LONG_NAME}"), &common::payload(5000, 1));
    add(&format!("/{UNICODE_NAME}"), b"unicode");
    add(&format!("/{}", common::huge_name()), b"huge name");
    add("/empty.dat", b"");
    add("/hidden.sys", b"h");
    let mut frag = common::payload(100, 2);
    frag.extend(common::payload(40_000, 4));
    add("/frag.bin", &frag);
    add("/spacer.bin", &common::payload(100, 3));
    add("/Nested Dir/sibling.txt", b"sibling");
    add(&format!("{INNER}/deep.bin"), &common::payload(70_000, 5));
    for i in 0..INNER_FILES - 1 {
        let name = format!("file number {i:02}.txt");
        add(&format!("{INNER}/{name}"), name.as_bytes());
    }
    add(&format!("/{KANJI_NAME}"), b"kanji");
    files
}

/// Walks one directory, checking every entry, then recurses into
/// subdirectories. Returns the files and directories seen.
fn walk(
    case: Case,
    fs: &mut Fs,
    dir: NodeId,
    path: &str,
    files: &BTreeMap<String, Vec<u8>>,
) -> usize {
    let listed = list(fs, dir);
    let mut seen = 0;
    for (text, entry) in &listed {
        let context = format!("{}: {path}{text}", case.name);
        let child_path = format!("{path}{text}");
        let meta = fs.stat(entry.node()).unwrap();
        assert_eq!(meta, *entry.metadata(), "{context}");
        let expected_attrs = if text == "hidden.sys" {
            Attributes::HIDDEN | Attributes::SYSTEM | Attributes::READ_ONLY
        } else if entry.file_type() == FileType::File {
            Attributes::ARCHIVE
        } else {
            Attributes::empty()
        };
        assert_eq!(meta.attributes(), expected_attrs, "{context}");
        assert!(meta.modified().is_some(), "{context}");

        let node = fs.lookup(dir, name(&swap_case(text))).unwrap();
        assert_eq!(node, entry.node(), "{context}");
        assert_eq!(fs.stat(node).unwrap(), meta, "{context}");

        if entry.file_type() == FileType::Dir {
            assert_eq!(meta.len(), 0, "{context}");
            seen += walk(case, fs, node, &format!("{child_path}/"), files);
            let parent = fs.parent(node).unwrap();
            assert_eq!(parent, dir, "{context}");
            fs.forget(parent, 1);
        } else {
            let data = files
                .get(&child_path)
                .unwrap_or_else(|| panic!("{context}"));
            assert_eq!(meta.len(), data.len() as u64, "{context}");
            assert_eq!(&read_all(fs, node), data, "{context}");
        }
        fs.forget(node, 1);
        seen += 1;
    }
    seen
}

#[test]
fn listings_metadata_and_contents_match_the_fixture() {
    let files = expected_files();
    for case in CASES {
        let mut fs = open(case, common::build(case));
        assert_eq!(fs.info().kind(), case.kind, "{}", case.name);
        let root = fs.root();
        let seen = walk(case, &mut fs, root, "/", &files);
        assert_eq!(seen, 11 + 2 + INNER_FILES, "{}", case.name);
        assert_eq!(fs.open_nodes(), 1, "{}", case.name);
    }
}

#[test]
fn names_long_unicode_huge_and_kanji() {
    let case = CASES[0];
    let mut fs = open(case, common::build(case));
    let root = fs.root();
    let names: Vec<String> = list(&mut fs, root).into_iter().map(|(n, _)| n).collect();
    for expected in [
        "README.TXT",
        "lower.txt",
        LONG_NAME,
        UNICODE_NAME,
        common::huge_name().as_str(),
        "empty.dat",
        "hidden.sys",
        "Nested Dir",
        KANJI_NAME,
    ] {
        assert!(
            names.iter().any(|n| n == expected),
            "{expected} in {names:?}"
        );
    }
    assert!(common::huge_name().len() > 255);
    assert!(!names.iter().any(|n| n.starts_with('\u{5}')));

    let kanji = fs.lookup(root, name(KANJI_NAME)).unwrap();
    assert_eq!(read_all(&mut fs, kanji), b"kanji");
    let huge = fs
        .lookup(root, name(&common::huge_name().to_uppercase()))
        .unwrap();
    assert_eq!(read_all(&mut fs, huge), b"huge name");
    let unicode = fs.lookup(root, name(&UNICODE_NAME.to_lowercase())).unwrap();
    assert_eq!(read_all(&mut fs, unicode), b"unicode");
}

#[test]
fn lookup_matches_short_names_and_ignores_case() {
    for case in CASES {
        let image = common::build(case);
        let short: [u8; 11] = image
            .chunks_exact(32)
            .find(|entry| entry.starts_with(b"ALONGF") && entry[11] == 0x20)
            .expect("short entry of the long name")[..11]
            .try_into()
            .unwrap();
        let base = String::from_utf8_lossy(&short[..8]).trim_end().to_owned();
        let ext = String::from_utf8_lossy(&short[8..]).trim_end().to_owned();
        let short = format!("{base}.{ext}");
        assert_ne!(short, LONG_NAME);

        let mut fs = open(case, image);
        let root = fs.root();
        let by_long = fs.lookup(root, name(LONG_NAME)).unwrap();
        assert_eq!(fs.lookup(root, name(&short)).unwrap(), by_long);
        assert_eq!(
            fs.lookup(root, name(&short.to_lowercase())).unwrap(),
            by_long
        );
        assert_eq!(
            fs.lookup(root, name("readme.txt")).unwrap(),
            fs.lookup(root, name("README.TXT")).unwrap()
        );
        assert_eq!(
            fs.lookup(root, name("missing.txt")).unwrap_err().kind(),
            ErrorKind::NotFound
        );
        assert_eq!(
            fs.lookup(root, name("HADRIS")).unwrap_err().kind(),
            ErrorKind::NotFound,
            "the volume label is not a file"
        );
    }
}

#[test]
fn reads_at_offsets_across_clusters_and_fragments() {
    for case in CASES {
        let mut fs = open(case, common::build(case));
        let root = fs.root();
        let frag = fs.lookup(root, name("frag.bin")).unwrap();
        let mut expected = common::payload(100, 2);
        expected.extend(common::payload(40_000, 4));
        assert_eq!(read_all(&mut fs, frag), expected, "{}", case.name);

        let mut buf = vec![0u8; 5000];
        for offset in [39_999u64, 12_345, 0, 33_000, 99] {
            let n = fs.read(frag, offset, &mut buf).unwrap();
            let start = offset as usize;
            let end = (start + buf.len()).min(expected.len());
            assert_eq!(
                &buf[..n],
                &expected[start..end],
                "{} at {offset}",
                case.name
            );
        }
        assert_eq!(fs.read(frag, 40_100, &mut buf).unwrap(), 0);
        assert_eq!(fs.read(frag, u64::MAX, &mut buf).unwrap(), 0);
        assert_eq!(fs.read(frag, 0, &mut []).unwrap(), 0);

        let empty = fs.lookup(root, name("empty.dat")).unwrap();
        assert_eq!(fs.read(empty, 0, &mut buf).unwrap(), 0);
    }
}

#[test]
fn parent_walks_up_to_the_root() {
    for case in CASES {
        let mut fs = open(case, common::build(case));
        let root = fs.root();
        assert_eq!(fs.parent(root).unwrap(), root);
        let nested = fs.lookup(root, name("nested dir")).unwrap();
        let inner = fs.lookup(nested, name("INNER")).unwrap();
        assert_eq!(fs.parent(nested).unwrap(), root, "{}", case.name);
        let up = fs.parent(inner).unwrap();
        assert_eq!(up, nested, "{}", case.name);
        fs.forget(up, 1);
        let file = fs.lookup(inner, name("deep.bin")).unwrap();
        assert_eq!(
            fs.parent(file).unwrap_err().kind(),
            ErrorKind::NotADirectory
        );
        for node in [file, inner, nested] {
            fs.forget(node, 1);
        }
        assert_eq!(fs.open_nodes(), 1);

        let inner = fs.resolve_path(INNER).unwrap();
        let listed = list(&mut fs, inner);
        assert_eq!(listed.len(), INNER_FILES, "{}", case.name);
        fs.forget(inner, 1);
    }
}

#[test]
fn stats_count_clusters() {
    for case in CASES {
        let mut fs = FatFs::mount(
            common::device(case, common::blank(case)),
            MountOptions::new(),
        )
        .unwrap();
        let empty = fs.statfs().unwrap();
        let reserved = u64::from(case.kind == FatKind::Fat32);
        assert_eq!(
            empty.free_blocks(),
            empty.total_blocks() - reserved,
            "{}",
            case.name
        );
        assert!(empty.block_size() >= 512);

        let image = common::build(case);
        let free = common::scan_free(&mut common::device(case, image.clone()));
        let mut fs = open(case, image);
        let stats = fs.statfs().unwrap();
        assert_eq!(stats.total_blocks(), empty.total_blocks());
        assert_eq!(stats.block_size(), empty.block_size());
        let used = empty.free_blocks() - stats.free_blocks();
        let min_used = (40_100 + 70_000) / u64::from(stats.block_size());
        assert!(used > min_used, "{}: {used} clusters used", case.name);
        assert_eq!(stats.free_blocks(), u64::from(free), "{}", case.name);
        assert_eq!(fs.statfs().unwrap(), stats);
    }
}

#[test]
fn full_table_limits_pins_without_touching_the_disk() {
    let case = CASES[1];
    let image = common::build(case);
    let mut fs: Fs = FatFs::mount(
        common::device(case, image.clone()),
        MountOptions::new().with_node_limit(2),
    )
    .unwrap();
    let root = fs.root();
    let a = fs.lookup(root, name("README.TXT")).unwrap();
    assert_eq!(fs.lookup(root, name("readme.txt")).unwrap(), a);
    assert_eq!(fs.open_nodes(), 2);
    let b = fs.lookup(root, name("lower.txt")).unwrap();
    assert_ne!(a, b);
    assert_eq!(fs.open_nodes(), 3);
    assert_eq!(
        fs.lookup(root, name("empty.dat")).unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );
    assert_eq!(fs.open_nodes(), 3);
    assert_eq!(fs.lookup(root, name("README.TXT")).unwrap(), a);

    fs.forget(a, 1);
    fs.forget(a, 1);
    assert_eq!(fs.open_nodes(), 3, "one pin on README.TXT is left");
    fs.forget(a, 1);
    assert_eq!(fs.open_nodes(), 2);
    let c = fs.lookup(root, name("empty.dat")).unwrap();
    assert_eq!(fs.stat(c).unwrap().len(), 0);

    fs.forget(root, 1);
    fs.forget(NodeId::new(12_345).unwrap(), 1);
    fs.forget(a, 1);
    for node in [b, c] {
        fs.forget(node, 1);
    }
    assert_eq!(fs.open_nodes(), 1);
    assert_eq!(fs.into_inner().into_inner(), image);
}

#[test]
fn unpinned_ids_from_listings_and_bad_handles() {
    let case = CASES[2];
    let mut fs = open(case, common::build(case));
    let root = fs.root();
    let (_, entry) = list(&mut fs, root)
        .into_iter()
        .find(|(n, _)| n == "frag.bin")
        .unwrap();
    assert_eq!(fs.open_nodes(), 1);
    assert_eq!(fs.stat(entry.node()).unwrap().len(), 40_100);
    let mut buf = [0u8; 4];
    assert_eq!(fs.read(entry.node(), 0, &mut buf).unwrap(), 4);
    assert_eq!(buf[..], common::payload(4, 2)[..]);

    for bad in [
        NodeId::new(2).unwrap(),
        NodeId::new(1 << 59).unwrap(),
        NodeId::new(1 << 63).unwrap(),
        NodeId::new(u64::MAX).unwrap(),
    ] {
        assert_eq!(fs.stat(bad).unwrap_err().kind(), ErrorKind::InvalidHandle);
        assert_eq!(
            fs.read(bad, 0, &mut buf).unwrap_err().kind(),
            ErrorKind::InvalidHandle
        );
    }
    let file = fs.lookup(root, name("README.TXT")).unwrap();
    assert_eq!(
        fs.readdir(file, DirCursor::START).unwrap_err().kind(),
        ErrorKind::NotADirectory
    );
    assert_eq!(
        fs.lookup(file, name("x")).unwrap_err().kind(),
        ErrorKind::NotADirectory
    );
    assert_eq!(
        fs.read(root, 0, &mut buf).unwrap_err().kind(),
        ErrorKind::IsADirectory
    );
    let dir = fs.lookup(root, name("Nested Dir")).unwrap();
    assert_eq!(
        fs.read(dir, 0, &mut buf).unwrap_err().kind(),
        ErrorKind::IsADirectory
    );
    assert_eq!(fs.stat(root).unwrap().file_type(), FileType::Dir);
}

#[test]
fn cursors_resume_and_stay_at_the_end() {
    let case = CASES[0];
    let mut fs = open(case, common::build(case));
    let inner = fs.resolve_path(INNER).unwrap();
    let all = list(&mut fs, inner);
    let mut cursor = DirCursor::START;
    for _ in 0..7 {
        cursor = fs.readdir(inner, cursor).unwrap().unwrap().next_cursor();
    }
    let mut resumed = DirCursor::from_raw(cursor.into_raw());
    let mut rest = Vec::new();
    while let Some(entry) = fs.readdir(inner, resumed).unwrap() {
        resumed = entry.next_cursor();
        rest.push(entry);
    }
    let tail: Vec<_> = all[7..].iter().map(|(_, entry)| *entry).collect();
    assert_eq!(rest, tail);
    assert_eq!(fs.readdir(inner, resumed).unwrap(), None);
    let far = DirCursor::from_raw(u64::MAX);
    assert_eq!(fs.readdir(inner, far).unwrap(), None);
}

#[test]
fn capabilities_and_write_methods_are_read_only() {
    let case = CASES[0];
    let image = common::build(case);
    let mut fs = FatFs::mount(
        common::device(case, image.clone()),
        MountOptions::new().read_only(),
    )
    .unwrap();
    assert!(fs.is_read_only());
    let caps = FileSystem::capabilities(&fs);
    assert!(!caps.writable());
    assert_eq!(caps.case(), CaseRule::InsensitivePreserving);
    assert_eq!(caps.charset(), Charset::Unicode);
    assert_eq!(caps.max_name_bytes(), 765);
    assert_eq!(caps.timestamp_resolution_ns(), 2_000_000_000);

    let root = FileSystem::root(&fs);
    let file = FileSystem::lookup(&mut fs, root, name("README.TXT")).unwrap();
    let meta = SetAttr::new();
    let kinds = [
        FileSystem::create(&mut fs, root, name("new"), &meta).map(|_| ()),
        FileSystem::mkdir(&mut fs, root, name("new"), &meta).map(|_| ()),
        FileSystem::unlink(&mut fs, root, name("README.TXT")),
        FileSystem::rmdir(&mut fs, root, name("Nested Dir")),
        FileSystem::write(&mut fs, file, 0, b"x").map(|_| ()),
        FileSystem::truncate(&mut fs, file, 0),
        FileSystem::setattr(&mut fs, file, &meta),
        FileSystem::open(&mut fs, file, hadris_fs::OpenMode::Write),
    ]
    .map(|result| result.unwrap_err().kind());
    assert_eq!(kinds, [ErrorKind::ReadOnly; 8]);
    FileSystem::open(&mut fs, file, hadris_fs::OpenMode::Read).unwrap();
    FileSystem::close(&mut fs, file).unwrap();
    FileSystem::forget(&mut fs, file, 1);
    assert_eq!(fs.into_inner().into_inner(), image);

    let fs = FatFs::mount(
        common::device(case, common::build(case)),
        MountOptions::new(),
    )
    .unwrap();
    assert!(!fs.is_read_only());
    assert!(format!("{fs:?}").contains("Fat12"));
}

#[test]
fn rejects_what_it_cannot_mount() {
    let case = CASES[0];
    let image = common::build(case);
    let big_blocks = MemDevice::new(image.clone(), BlockSize::new(8192).unwrap());
    assert_eq!(
        FatFs::mount(big_blocks, MountOptions::new())
            .unwrap_err()
            .kind(),
        ErrorKind::Unsupported
    );
    let blank = common::device(case, vec![0u8; 64 * 1024]);
    assert_eq!(
        FatFs::mount(blank, MountOptions::new()).unwrap_err().kind(),
        ErrorKind::NotRecognized
    );
    let truncated = common::device(case, image[..image.len() / 2].to_vec());
    assert_eq!(
        FatFs::mount(truncated, MountOptions::new())
            .unwrap_err()
            .kind(),
        ErrorKind::Corrupt
    );
    let short = common::device(case, image[..512].to_vec());
    let err = FatFs::mount(short, MountOptions::new()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Corrupt);
    assert_eq!(
        hadris_fat::Detail::of(err.error()),
        Some(hadris_fat::Detail::BootSector)
    );
}

#[test]
fn failed_opens_give_the_device_back() {
    let case = CASES[0];
    let mut corrupt = common::build(case);
    corrupt[11..13].copy_from_slice(&0u16.to_le_bytes());
    let images = [vec![0u8; 64 * 1024], corrupt];
    for image in images {
        let err =
            FatFs::mount(common::device(case, image.clone()), MountOptions::new()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotRecognized);
        assert_eq!(err.device().get_ref(), &image);
        let (error, dev) = err.into_parts();
        assert_eq!(error.kind(), ErrorKind::NotRecognized);
        assert_eq!(dev.into_inner(), image);

        let options = MountOptions::new().read_only();
        let err = FatFs::mount(common::device(case, image.clone()), options).unwrap_err();
        assert_eq!(err.into_device().into_inner(), image);
    }
    let big_blocks = MemDevice::new(common::build(case), BlockSize::new(8192).unwrap());
    let err = FatFs::mount(big_blocks, MountOptions::new()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported);
    assert_eq!(err.into_device().get_ref(), &common::build(case));
}

#[test]
fn mount_errors_convert_with_the_question_mark() {
    fn plain(dev: Device) -> hadris_fs::FsResult<Fs, core::convert::Infallible> {
        Ok(FatFs::mount(dev, MountOptions::new())?)
    }
    fn io(dev: Device) -> std::io::Result<()> {
        FatFs::mount(dev, MountOptions::new())?;
        Ok(())
    }
    fn boxed(dev: Device) -> Result<(), Box<dyn std::error::Error>> {
        FatFs::mount(dev, MountOptions::new())?;
        Ok(())
    }
    let case = CASES[0];
    let blank = || common::device(case, vec![0u8; 4096]);
    assert_eq!(plain(blank()).unwrap_err().kind(), ErrorKind::NotRecognized);
    assert_eq!(
        io(blank()).unwrap_err().kind(),
        std::io::ErrorKind::InvalidData
    );
    let err = plain(blank()).unwrap_err();
    assert_eq!(
        hadris_fat::Detail::of(&err),
        Some(hadris_fat::Detail::BootSector)
    );
    assert_eq!(boxed(blank()).unwrap_err().to_string(), err.to_string());
    assert!(
        format!(
            "{:?}",
            FatFs::mount(blank(), MountOptions::new()).unwrap_err()
        )
        .starts_with("MountError")
    );
}

#[test]
fn volume_paths_and_handles() {
    let case = CASES[2];
    let mut fs = open(case, common::build(case));
    assert_eq!(
        fs.read_to_vec("/Nested Dir/sibling.txt").unwrap(),
        b"sibling"
    );
    assert!(fs.exists("/nested dir/INNER/DEEP.BIN").unwrap());
    assert_eq!(fs.open_nodes(), 1);

    let vol = Volume::new(fs);
    let deep = common::payload(70_000, 5);
    assert_eq!(vol.read_to_vec("/Nested Dir/inner/deep.bin").unwrap(), deep);
    assert_eq!(vol.metadata("/frag.bin").unwrap().len(), 40_100);

    let mut file = vol
        .open("/nested dir/inner/deep.bin", OpenOptions::new().read())
        .unwrap();
    let mut data = Vec::new();
    file.read_to_end(&mut data).unwrap();
    assert_eq!(data, deep);
    file.close().unwrap();

    let names: Vec<String> = vol
        .read_dir(INNER)
        .unwrap()
        .map(|item| item.unwrap().name().to_str().unwrap().to_owned())
        .collect();
    assert_eq!(names.len(), INNER_FILES);
    assert!(names.contains(&"file number 07.txt".to_owned()));
    vol.open("/README.TXT", OpenOptions::new().write())
        .unwrap()
        .close()
        .unwrap();
    assert_eq!(vol.into_inner().unwrap().open_nodes(), 1);
}

#[test]
fn ascii_short_names_with_high_bytes_stay_distinct() {
    let case = CASES[0];
    let mut fs = common::formatted(case, hadris_fat::FatOptions::new());
    let root = fs.root();
    for (text, data) in [("PAT1.TXT", b"first"), ("PAT2.TXT", b"other")] {
        let node = fs.create(root, name(text), &SetAttr::new()).unwrap();
        fs.write(node, 0, data).unwrap();
        fs.forget(node, 1);
    }
    fs.sync().unwrap();
    let mut image = fs.into_inner().into_inner();
    for (from, to) in [
        (b"PAT1    TXT", b"\x82AB     TXT"),
        (b"PAT2    TXT", b"\x83AB     TXT"),
    ] {
        let at = image
            .chunks_exact(32)
            .position(|entry| &entry[..11] == from)
            .unwrap()
            * 32;
        image[at..at + 11].copy_from_slice(to);
    }
    let options = MountOptions::new().with_code_page(&Ascii);
    let mut fs = FatFs::mount(common::device(case, image), options).unwrap();
    let root = fs.root();
    let names: Vec<String> = list(&mut fs, root).into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["\u{F782}AB.TXT", "\u{F783}AB.TXT"]);
    for (text, data) in [("\u{F782}ab.txt", b"first"), ("\u{F783}AB.TXT", b"other")] {
        let node = fs.lookup(root, name(text)).unwrap();
        assert_eq!(read_all(&mut fs, node), data);
        fs.forget(node, 1);
    }
}

fn try_list(fs: &mut Fs, dir: NodeId) -> Result<usize, ErrorKind> {
    let mut cursor = DirCursor::START;
    let mut count = 0;
    while let Some(entry) = fs.readdir(dir, cursor).map_err(|err| err.kind())? {
        cursor = entry.next_cursor();
        count += 1;
    }
    Ok(count)
}

fn try_read(fs: &mut Fs, node: NodeId) -> Result<usize, ErrorKind> {
    let mut done = 0;
    let mut chunk = [0u8; 777];
    loop {
        match fs
            .read(node, done as u64, &mut chunk)
            .map_err(|err| err.kind())?
        {
            0 => return Ok(done),
            n => done += n,
        }
    }
}

#[test]
fn cyclic_chains_are_corrupt_instead_of_repeating() {
    let case = CASES[1];
    let image = common::build(case);
    let (inner, deep) = {
        let mut fs = common::mount(case, &image);
        let inner = fs.resolve_path(INNER).unwrap();
        let deep = fs.lookup(inner, name("deep.bin")).unwrap();
        (common::chain(&mut fs, inner), common::chain(&mut fs, deep))
    };
    assert!(deep.len() > 4);
    let u16_at = |image: &[u8], at: usize| u16::from_le_bytes([image[at], image[at + 1]]) as usize;
    let link = |image: &mut Vec<u8>, from: u32, to: u32| {
        let fat_start = u16_at(image, 14) * u16_at(image, 11);
        let fat_len = u16_at(image, 22) * u16_at(image, 11);
        for copy in 0..image[16] as usize {
            let at = fat_start + copy * fat_len + from as usize * 2;
            image[at..at + 2].copy_from_slice(&(to as u16).to_le_bytes());
        }
    };
    let loops = [
        (inner[0], inner[0], deep[0], deep[0]),
        (inner[1], inner[0], deep[3], deep[1]),
    ];
    for (dir_from, dir_to, file_from, file_to) in loops {
        let mut dir_image = image.clone();
        link(&mut dir_image, dir_from, dir_to);
        let mut fs = open(case, dir_image);
        let inner = fs.resolve_path(INNER).unwrap();
        assert_eq!(try_list(&mut fs, inner), Err(ErrorKind::Corrupt));

        let mut file_image = image.clone();
        link(&mut file_image, file_from, file_to);
        let mut fs = open(case, file_image);
        let file = fs.resolve_path(&format!("{INNER}/deep.bin")).unwrap();
        assert_eq!(try_read(&mut fs, file), Err(ErrorKind::Corrupt));
        let mut all = vec![0u8; 70_000];
        assert_eq!(
            fs.read(file, 0, &mut all).map_err(|err| err.kind()),
            Err(ErrorKind::Corrupt)
        );
    }
}

fn fat32_layout(image: &[u8]) -> (usize, usize, usize, u8) {
    let u16_at = |at: usize| u16::from_le_bytes([image[at], image[at + 1]]) as usize;
    let u32_at = |at: usize| u32::from_le_bytes(image[at..at + 4].try_into().unwrap()) as usize;
    let sector = u16_at(11);
    (
        u16_at(14) * sector,
        u32_at(36) * sector,
        u16_at(48) * sector,
        image[16],
    )
}

#[test]
fn fsinfo_unknown_values_mount_and_count_by_scanning() {
    let case = CASES[2];
    assert_eq!(case.kind, FatKind::Fat32);
    let mut image = common::build(case);
    let free = common::scan_free(&mut common::device(case, image.clone()));
    let (_, _, fs_info, _) = fat32_layout(&image);
    image[fs_info + 488..fs_info + 496].fill(0xFF);
    let mut fs = open(case, image);
    assert_eq!(fs.statfs().unwrap().free_blocks(), u64::from(free));
    let file = fs.resolve_path("/README.TXT").unwrap();
    assert_eq!(read_all(&mut fs, file), b"hello fat");
}

#[test]
fn fat32_uses_only_the_active_fat_when_mirroring_is_disabled() {
    let case = CASES[2];
    let mut image = common::build(case);
    let (fat_start, fat_len, _, copies) = fat32_layout(&image);
    assert!(copies >= 2);
    image[40..42].copy_from_slice(&(0x80u16 | 1).to_le_bytes());
    let first = image[fat_start..fat_start + 8].to_vec();
    image[fat_start + 8..fat_start + fat_len].fill(0);

    let mut fs = open(case, image);
    let file = fs.resolve_path(&format!("/{LONG_NAME}")).unwrap();
    assert_eq!(read_all(&mut fs, file), common::payload(5000, 1));
    fs.forget(file, 1);
    let root = fs.root();
    let node = fs.create(root, name("new.bin"), &SetAttr::new()).unwrap();
    assert_eq!(fs.write(node, 0, &[7u8; 5000]).unwrap(), 5000);
    fs.forget(node, 1);
    fs.sync().unwrap();

    let image = fs.into_inner().into_inner();
    assert_eq!(&image[fat_start..fat_start + 8], &first[..]);
    assert!(
        image[fat_start + 8..fat_start + fat_len]
            .iter()
            .all(|&b| b == 0)
    );
    let mut fs = open(case, image);
    let node = fs.resolve_path("/new.bin").unwrap();
    assert_eq!(read_all(&mut fs, node), [7u8; 5000]);
}

/// A blank FAT12/16 layout of 512-byte sectors, one per cluster, with 512
/// root entries and `clusters` data clusters.
fn fat16_layout(clusters: u32) -> Vec<u8> {
    let fat_sectors = (clusters + 2).div_ceil(256);
    let total = 1 + 2 * fat_sectors + 32 + clusters;
    let mut image = vec![0u8; total as usize * 512];
    image[..3].copy_from_slice(&[0xEB, 0x3C, 0x90]);
    image[11..13].copy_from_slice(&512u16.to_le_bytes());
    image[13] = 1;
    image[14..16].copy_from_slice(&1u16.to_le_bytes());
    image[16] = 2;
    image[17..19].copy_from_slice(&512u16.to_le_bytes());
    image[21] = 0xF8;
    image[22..24].copy_from_slice(&(fat_sectors as u16).to_le_bytes());
    image[32..36].copy_from_slice(&total.to_le_bytes());
    image[510..512].copy_from_slice(&[0x55, 0xAA]);
    for copy in 0..2 {
        let at = 512 * (1 + copy * fat_sectors as usize);
        image[at..at + 4].copy_from_slice(&[0xF8, 0xFF, 0xFF, 0xFF]);
    }
    image
}

#[test]
fn fat16_layouts_are_limited_to_what_fat16_addresses() {
    let device = |image| MemDevice::new(image, BlockSize::new(512).unwrap());
    let mut fs = FatFs::mount(device(fat16_layout(65_524)), MountOptions::new()).unwrap();
    assert_eq!(fs.info().kind(), FatKind::Fat16);
    assert_eq!(fs.statfs().unwrap().total_blocks(), 65_524);
    let mut fs = FatFs::mount(device(fat16_layout(4_084)), MountOptions::new()).unwrap();
    assert_eq!(fs.info().kind(), FatKind::Fat12);
    assert_eq!(fs.statfs().unwrap().total_blocks(), 4_084);
    for clusters in [65_525, 65_600] {
        assert_eq!(
            FatFs::mount(device(fat16_layout(clusters)), MountOptions::new())
                .unwrap_err()
                .kind(),
            ErrorKind::Corrupt,
            "{clusters} clusters"
        );
    }
}
