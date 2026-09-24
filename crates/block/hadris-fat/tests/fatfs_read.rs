//! The read-only `FatFs` driver on images filled through `FatFs`.

#[path = "common/fatfs.rs"]
mod common;

use std::collections::BTreeMap;
use std::io::Read as _;

use common::{CASES, Case, Device, INNER, INNER_FILES, KANJI_NAME, LONG_NAME, UNICODE_NAME};
use hadris_fat::sync::FatFs;
use hadris_fat::{FatKind, MountOptions};
use hadris_fs::sync::{DriverExt, File, FsDriver, PathExt, Volume};
use hadris_fs::{
    Attributes, CaseSensitivity, DirCursor, ErrorKind, FileType, FixedTable, HeapTable, Name,
    NameBuf, NameCharset, NewNode, NodeId, NodeTable, OpenOptions, SetMetadata,
};
use hadris_storage::{BlockSize, MemDevice};

type Fs<T = HeapTable> = FatFs<Device, T>;

fn open(case: Case, image: Vec<u8>) -> Fs {
    FatFs::open_with(
        common::device(case, image),
        MountOptions::new().with_table(HeapTable::new()),
    )
    .unwrap()
}

fn name(text: &str) -> &Name {
    Name::new(text).unwrap()
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

fn list<T: NodeTable>(fs: &mut Fs<T>, dir: NodeId) -> Vec<(String, hadris_fs::DirEntry)> {
    let mut cursor = DirCursor::start();
    let mut buf = NameBuf::new();
    let mut out = Vec::new();
    while let Some(entry) = fs.read_dir_entry(dir, &mut cursor, &mut buf).unwrap() {
        assert_eq!(entry.name_len(), buf.len());
        out.push((buf.as_name().unwrap().to_str().unwrap().to_owned(), entry));
    }
    out
}

fn read_all<T: NodeTable>(fs: &mut Fs<T>, node: NodeId) -> Vec<u8> {
    let mut out = Vec::new();
    let mut chunk = [0u8; 777];
    loop {
        let n = fs.read_at(node, out.len() as u64, &mut chunk).unwrap();
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
        let meta = fs.node_metadata(entry.node()).unwrap();
        assert_eq!(meta.file_type(), entry.file_type(), "{context}");
        let expected_attrs = if text == "hidden.sys" {
            Attributes::HIDDEN | Attributes::SYSTEM | Attributes::READ_ONLY
        } else if entry.file_type() == FileType::File {
            Attributes::ARCHIVE
        } else {
            Attributes::empty()
        };
        assert_eq!(meta.attributes(), expected_attrs, "{context}");
        assert!(meta.times().modified().is_some(), "{context}");

        let node = fs.lookup(dir, name(&swap_case(text))).unwrap();
        assert_eq!(node, entry.node(), "{context}");
        assert_eq!(fs.node_metadata(node).unwrap(), meta, "{context}");

        if entry.file_type() == FileType::Dir {
            assert_eq!(meta.len(), 0, "{context}");
            seen += walk(case, fs, node, &format!("{child_path}/"), files);
            let parent = fs.parent(node).unwrap();
            assert_eq!(parent, dir, "{context}");
            fs.forget(parent);
        } else {
            let data = files
                .get(&child_path)
                .unwrap_or_else(|| panic!("{context}"));
            assert_eq!(meta.len(), data.len() as u64, "{context}");
            assert_eq!(&read_all(fs, node), data, "{context}");
        }
        fs.forget(node);
        seen += 1;
    }
    seen
}

#[test]
fn listings_metadata_and_contents_match_the_fixture() {
    let files = expected_files();
    for case in CASES {
        let mut fs = open(case, common::build(case));
        assert_eq!(fs.kind(), case.kind, "{}", case.name);
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
            let n = fs.read_at(frag, offset, &mut buf).unwrap();
            let start = offset as usize;
            let end = (start + buf.len()).min(expected.len());
            assert_eq!(
                &buf[..n],
                &expected[start..end],
                "{} at {offset}",
                case.name
            );
        }
        assert_eq!(fs.read_at(frag, 40_100, &mut buf).unwrap(), 0);
        assert_eq!(fs.read_at(frag, u64::MAX, &mut buf).unwrap(), 0);
        assert_eq!(fs.read_at(frag, 0, &mut []).unwrap(), 0);

        let empty = fs.lookup(root, name("empty.dat")).unwrap();
        assert_eq!(fs.read_at(empty, 0, &mut buf).unwrap(), 0);
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
        fs.forget(up);
        let file = fs.lookup(inner, name("deep.bin")).unwrap();
        assert_eq!(
            fs.parent(file).unwrap_err().kind(),
            ErrorKind::NotADirectory
        );
        for node in [file, inner, nested] {
            fs.forget(node);
        }
        assert_eq!(fs.open_nodes(), 1);

        let inner = fs.resolve(INNER).unwrap();
        let listed = list(&mut fs, inner);
        assert_eq!(listed.len(), INNER_FILES, "{}", case.name);
        fs.forget(inner);
    }
}

#[test]
fn stats_count_clusters() {
    for case in CASES {
        let mut fs = FatFs::open(common::device(case, common::blank(case))).unwrap();
        let empty = fs.stats().unwrap();
        let reserved = u64::from(case.kind == FatKind::Fat32);
        assert_eq!(
            empty.free_blocks(),
            empty.total_blocks() - reserved,
            "{}",
            case.name
        );
        assert!(empty.block_size() >= 512);

        let image = common::build(case);
        let free = hadris_fat::sync::check(&mut common::mount(case, &image))
            .unwrap()
            .free_clusters();
        let mut fs = open(case, image);
        let stats = fs.stats().unwrap();
        assert_eq!(stats.total_blocks(), empty.total_blocks());
        assert_eq!(stats.block_size(), empty.block_size());
        let used = empty.free_blocks() - stats.free_blocks();
        let min_used = (40_100 + 70_000) / u64::from(stats.block_size());
        assert!(used > min_used, "{}: {used} clusters used", case.name);
        assert_eq!(stats.free_blocks(), u64::from(free), "{}", case.name);
        assert_eq!(fs.stats().unwrap(), stats);
    }
}

#[test]
fn full_table_limits_pins_without_touching_the_disk() {
    let case = CASES[1];
    let image = common::build(case);
    let mut fs: Fs<FixedTable<2>> = FatFs::open_with(
        common::device(case, image.clone()),
        MountOptions::new().with_table(FixedTable::new()),
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

    fs.forget(a);
    fs.forget(a);
    assert_eq!(fs.open_nodes(), 3, "one pin on README.TXT is left");
    fs.forget(a);
    assert_eq!(fs.open_nodes(), 2);
    let c = fs.lookup(root, name("empty.dat")).unwrap();
    assert_eq!(fs.node_metadata(c).unwrap().len(), 0);

    fs.forget(root);
    fs.forget(NodeId::new(12_345));
    fs.forget(a);
    for node in [b, c] {
        fs.forget(node);
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
    assert_eq!(fs.node_metadata(entry.node()).unwrap().len(), 40_100);
    let mut buf = [0u8; 4];
    assert_eq!(fs.read_at(entry.node(), 0, &mut buf).unwrap(), 4);
    assert_eq!(buf[..], common::payload(4, 2)[..]);

    for bad in [
        NodeId::new(0),
        NodeId::new(2),
        NodeId::new(1 << 59),
        NodeId::new(1 << 63),
        NodeId::new(u64::MAX),
    ] {
        assert_eq!(
            fs.node_metadata(bad).unwrap_err().kind(),
            ErrorKind::InvalidHandle
        );
        assert_eq!(
            fs.read_at(bad, 0, &mut buf).unwrap_err().kind(),
            ErrorKind::InvalidHandle
        );
    }
    let mut cursor = DirCursor::start();
    let mut text = NameBuf::new();
    let file = fs.lookup(root, name("README.TXT")).unwrap();
    assert_eq!(
        fs.read_dir_entry(file, &mut cursor, &mut text)
            .unwrap_err()
            .kind(),
        ErrorKind::NotADirectory
    );
    assert!(cursor.is_start());
    assert_eq!(
        fs.lookup(file, name("x")).unwrap_err().kind(),
        ErrorKind::NotADirectory
    );
    assert_eq!(
        fs.read_at(root, 0, &mut buf).unwrap_err().kind(),
        ErrorKind::IsADirectory
    );
    let dir = fs.lookup(root, name("Nested Dir")).unwrap();
    assert_eq!(
        fs.read_at(dir, 0, &mut buf).unwrap_err().kind(),
        ErrorKind::IsADirectory
    );
    assert_eq!(fs.node_metadata(root).unwrap().file_type(), FileType::Dir);
}

#[test]
fn cursors_resume_and_stay_at_the_end() {
    let case = CASES[0];
    let mut fs = open(case, common::build(case));
    let inner = fs.resolve(INNER).unwrap();
    let all = list(&mut fs, inner);
    let mut cursor = DirCursor::start();
    let mut text = NameBuf::new();
    for _ in 0..7 {
        fs.read_dir_entry(inner, &mut cursor, &mut text)
            .unwrap()
            .unwrap();
    }
    let mut resumed = DirCursor::from_raw(cursor.into_raw());
    let mut rest = Vec::new();
    while let Some(entry) = fs.read_dir_entry(inner, &mut resumed, &mut text).unwrap() {
        rest.push(entry);
    }
    let tail: Vec<_> = all[7..].iter().map(|(_, entry)| *entry).collect();
    assert_eq!(rest, tail);
    assert_eq!(
        fs.read_dir_entry(inner, &mut resumed, &mut text).unwrap(),
        None
    );
    let mut far = DirCursor::from_raw(u64::MAX);
    assert_eq!(fs.read_dir_entry(inner, &mut far, &mut text).unwrap(), None);
}

#[test]
fn capabilities_and_write_methods_are_read_only() {
    let case = CASES[0];
    let image = common::build(case);
    let mut fs = FatFs::open_with(
        common::device(case, image.clone()),
        MountOptions::new().with_read_only(),
    )
    .unwrap();
    assert!(fs.is_read_only());
    let caps = FsDriver::capabilities(&fs);
    assert!(!caps.is_writable());
    assert_eq!(
        caps.case_sensitivity(),
        CaseSensitivity::InsensitivePreserving
    );
    assert_eq!(caps.name_charset(), NameCharset::Utf16);
    assert_eq!(caps.max_name_len(), 765);
    assert_eq!(caps.timestamp_resolution_ns(), 2_000_000_000);

    let root = FsDriver::root(&fs);
    let file = FsDriver::lookup(&mut fs, root, name("README.TXT")).unwrap();
    let meta = SetMetadata::new();
    let kinds = [
        FsDriver::create(&mut fs, root, name("new"), NewNode::File, &meta).map(|_| ()),
        FsDriver::remove(
            &mut fs,
            root,
            name("README.TXT"),
            hadris_fs::RemoveKind::Any,
        ),
        FsDriver::write_at(&mut fs, file, 0, b"x").map(|_| ()),
        FsDriver::set_len(&mut fs, file, 0),
        FsDriver::set_metadata(&mut fs, file, &meta),
    ]
    .map(|result| result.unwrap_err().kind());
    assert_eq!(kinds, [ErrorKind::ReadOnly; 5]);
    FsDriver::forget(&mut fs, file);
    assert_eq!(fs.into_inner().into_inner(), image);

    let fs = FatFs::open(common::device(case, common::build(case))).unwrap();
    assert!(!fs.is_read_only());
    assert!(format!("{fs:?}").contains("Fat12"));
}

#[test]
fn rejects_what_it_cannot_mount() {
    let case = CASES[0];
    let image = common::build(case);
    let big_blocks = MemDevice::new(image.clone(), BlockSize::new(8192).unwrap());
    assert_eq!(
        FatFs::open(big_blocks).unwrap_err().kind(),
        ErrorKind::Unsupported
    );
    let blank = common::device(case, vec![0u8; 64 * 1024]);
    assert_eq!(FatFs::open(blank).unwrap_err().kind(), ErrorKind::Corrupt);
    let truncated = common::device(case, image[..image.len() / 2].to_vec());
    assert_eq!(
        FatFs::open(truncated).unwrap_err().kind(),
        ErrorKind::Corrupt
    );
    let short = common::device(case, image[..512].to_vec());
    assert_eq!(FatFs::open(short).unwrap_err().kind(), ErrorKind::Corrupt);
}

#[test]
fn failed_opens_give_the_device_back() {
    let case = CASES[0];
    let mut corrupt = common::build(case);
    corrupt[11..13].copy_from_slice(&0u16.to_le_bytes());
    let images = [vec![0u8; 64 * 1024], corrupt];
    for image in images {
        let err = FatFs::open(common::device(case, image.clone())).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Corrupt);
        assert_eq!(err.device().get_ref(), &image);
        let (error, dev) = err.into_parts();
        assert_eq!(error.kind(), ErrorKind::Corrupt);
        assert_eq!(dev.into_inner(), image);

        let options = MountOptions::new().with_read_only();
        let err = FatFs::open_with(common::device(case, image.clone()), options).unwrap_err();
        assert_eq!(err.into_device().into_inner(), image);
    }
    let big_blocks = MemDevice::new(common::build(case), BlockSize::new(8192).unwrap());
    let err = FatFs::open(big_blocks).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported);
    assert_eq!(err.into_device().get_ref(), &common::build(case));
}

#[test]
fn mount_errors_convert_with_the_question_mark() {
    fn plain(dev: Device) -> hadris_fs::FsResult<Fs, hadris_storage::OutOfRange> {
        Ok(FatFs::open_with(
            dev,
            MountOptions::new().with_table(HeapTable::new()),
        )?)
    }
    fn io(dev: Device) -> std::io::Result<()> {
        FatFs::open(dev)?;
        Ok(())
    }
    fn boxed(dev: Device) -> Result<(), Box<dyn std::error::Error>> {
        FatFs::open(dev)?;
        Ok(())
    }
    let case = CASES[0];
    let blank = || common::device(case, vec![0u8; 4096]);
    assert_eq!(plain(blank()).unwrap_err().kind(), ErrorKind::Corrupt);
    assert_eq!(
        io(blank()).unwrap_err().kind(),
        std::io::ErrorKind::InvalidData
    );
    let err = boxed(blank()).unwrap_err();
    assert_eq!(err.to_string(), ErrorKind::Corrupt.to_string());
    assert!(format!("{:?}", FatFs::open(blank()).unwrap_err()).starts_with("MountError"));
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

    let mut file = File::open(&vol, "/nested dir/inner/deep.bin", OpenOptions::read()).unwrap();
    let mut data = Vec::new();
    file.read_to_end(&mut data).unwrap();
    assert_eq!(data, deep);
    file.close().unwrap();

    let names: Vec<String> = vol
        .read_dir(INNER)
        .unwrap()
        .map(|item| item.unwrap().name_str().unwrap().to_owned())
        .collect();
    assert_eq!(names.len(), INNER_FILES);
    assert!(names.contains(&"file number 07.txt".to_owned()));
    vol.open("/README.TXT", OpenOptions::write())
        .unwrap()
        .close()
        .unwrap();
    assert_eq!(vol.into_inner().open_nodes(), 1);
}

#[test]
fn ascii_short_names_with_high_bytes_stay_distinct() {
    let case = CASES[0];
    let mut fs = common::formatted(case, hadris_fat::FormatOptions::new());
    let root = fs.root();
    for (text, data) in [("PAT1.TXT", b"first"), ("PAT2.TXT", b"other")] {
        let node = fs
            .create(root, name(text), NewNode::File, &SetMetadata::new())
            .unwrap();
        fs.write_at(node, 0, data).unwrap();
        fs.forget(node);
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
    let mut fs = open(case, image);
    let root = fs.root();
    let names: Vec<String> = list(&mut fs, root).into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["\u{F782}AB.TXT", "\u{F783}AB.TXT"]);
    for (text, data) in [("\u{F782}ab.txt", b"first"), ("\u{F783}AB.TXT", b"other")] {
        let node = fs.lookup(root, name(text)).unwrap();
        assert_eq!(read_all(&mut fs, node), data);
        fs.forget(node);
    }
}

fn try_list(fs: &mut Fs, dir: NodeId) -> Result<usize, ErrorKind> {
    let mut cursor = DirCursor::start();
    let mut buf = NameBuf::new();
    let mut count = 0;
    while fs
        .read_dir_entry(dir, &mut cursor, &mut buf)
        .map_err(|err| err.kind())?
        .is_some()
    {
        count += 1;
    }
    Ok(count)
}

fn try_read(fs: &mut Fs, node: NodeId) -> Result<usize, ErrorKind> {
    let mut done = 0;
    let mut chunk = [0u8; 777];
    loop {
        match fs
            .read_at(node, done as u64, &mut chunk)
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
        let inner = fs.resolve(INNER).unwrap();
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
        let inner = fs.resolve(INNER).unwrap();
        assert_eq!(try_list(&mut fs, inner), Err(ErrorKind::Corrupt));

        let mut file_image = image.clone();
        link(&mut file_image, file_from, file_to);
        let mut fs = open(case, file_image);
        let file = fs.resolve(&format!("{INNER}/deep.bin")).unwrap();
        assert_eq!(try_read(&mut fs, file), Err(ErrorKind::Corrupt));
        let mut all = vec![0u8; 70_000];
        assert_eq!(
            fs.read_at(file, 0, &mut all).map_err(|err| err.kind()),
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
    let free = hadris_fat::sync::check(&mut common::mount(case, &image))
        .unwrap()
        .free_clusters();
    let (_, _, fs_info, _) = fat32_layout(&image);
    image[fs_info + 488..fs_info + 496].fill(0xFF);
    let mut fs = open(case, image);
    assert_eq!(fs.stats().unwrap().free_blocks(), u64::from(free));
    let file = fs.resolve("/README.TXT").unwrap();
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
    let file = fs.resolve(&format!("/{LONG_NAME}")).unwrap();
    assert_eq!(read_all(&mut fs, file), common::payload(5000, 1));
    fs.forget(file);
    let root = fs.root();
    let node = fs
        .create(root, name("new.bin"), NewNode::File, &SetMetadata::new())
        .unwrap();
    assert_eq!(fs.write_at(node, 0, &[7u8; 5000]).unwrap(), 5000);
    fs.forget(node);
    fs.sync().unwrap();

    let image = fs.into_inner().into_inner();
    assert_eq!(&image[fat_start..fat_start + 8], &first[..]);
    assert!(
        image[fat_start + 8..fat_start + fat_len]
            .iter()
            .all(|&b| b == 0)
    );
    let mut fs = open(case, image);
    let node = fs.resolve("/new.bin").unwrap();
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
    let mut fs = FatFs::open(device(fat16_layout(65_524))).unwrap();
    assert_eq!(fs.kind(), FatKind::Fat16);
    assert_eq!(fs.stats().unwrap().total_blocks(), 65_524);
    let mut fs = FatFs::open(device(fat16_layout(4_084))).unwrap();
    assert_eq!(fs.kind(), FatKind::Fat12);
    assert_eq!(fs.stats().unwrap().total_blocks(), 4_084);
    for clusters in [65_525, 65_600] {
        assert_eq!(
            FatFs::open(device(fat16_layout(clusters)))
                .unwrap_err()
                .kind(),
            ErrorKind::Corrupt,
            "{clusters} clusters"
        );
    }
}
