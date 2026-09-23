//! The read-only `FatFs` driver against the V2 `FatVolume` on the same
//! images.

#[path = "common/fatfs.rs"]
mod common;

use std::io::Read as _;

use common::{CASES, Case, Device, INNER, INNER_FILES, KANJI_NAME, LONG_NAME, UNICODE_NAME};
use hadris_fat::sync::FatFs;
use hadris_fat::{FatDir, FatKind, FatVolume, FatVolumeReadExt, FileEntry, MountOptions};
use hadris_fs::sync::{DriverExt, File, FsDriver, PathExt, Volume};
use hadris_fs::{
    Attributes, CaseSensitivity, CivilDate, CivilTime, DateTime, DirCursor, ErrorKind, FileType,
    FixedTable, HeapTable, Name, NameBuf, NameCharset, NewNode, NodeId, NodeTable, OpenOptions,
    SetMetadata,
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

fn fat_time(date: u16, time: u16, tenths: u8) -> Option<DateTime> {
    let date = CivilDate::new(
        1980 + (date >> 9) as i32,
        ((date >> 5) & 0x0F) as u8,
        (date & 0x1F) as u8,
    )
    .ok()?;
    let time = CivilTime::new(
        (time >> 11) as u8,
        ((time >> 5) & 0x3F) as u8,
        ((time & 0x1F) * 2) as u8,
    )
    .ok()?;
    let base = DateTime::from_civil(date, time, None).ok()?;
    DateTime::new(
        base.unix_seconds() + (tenths / 100) as i64,
        (tenths % 100) as u32 * 10_000_000,
    )
    .ok()
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

fn v2_list(dir: &FatDir<'_, common::Image>) -> Vec<FileEntry> {
    let mut iter = dir.entries();
    let mut out = Vec::new();
    while let Some(entry) = iter.next_entry() {
        let entry = entry.unwrap();
        let file = entry.as_entry().unwrap();
        if file.name() != "." && file.name() != ".." {
            out.push(file.clone());
        }
    }
    out
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

/// Compares one directory, then recurses into subdirectories.
fn compare(
    case: Case,
    v2: &FatVolume<common::Image>,
    v2_dir: &FatDir<'_, common::Image>,
    fs: &mut Fs,
    dir: NodeId,
) -> usize {
    let expected = v2_list(v2_dir);
    let listed = list(fs, dir);
    let expected_names: Vec<String> = expected
        .iter()
        .map(|file| file.name().into_owned())
        .collect();
    let listed_names: Vec<&str> = listed.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(listed_names, expected_names, "{}", case.name);

    let mut compared = 0;
    for (file, (text, entry)) in expected.iter().zip(&listed) {
        let context = format!("{}: {text}", case.name);
        let meta = fs.node_metadata(entry.node()).unwrap();
        let file_type = if file.is_directory() {
            FileType::Dir
        } else {
            FileType::File
        };
        assert_eq!(entry.file_type(), file_type, "{context}");
        assert_eq!(meta.file_type(), file_type, "{context}");
        assert_eq!(meta.len(), file.len(), "{context}");
        assert_eq!(
            meta.attributes(),
            Attributes::from_bits_truncate(u32::from(file.attributes().bits())),
            "{context}"
        );
        let created = file.created();
        let modified = file.modified();
        assert_eq!(
            meta.times().created(),
            fat_time(created.date, created.time, created.time_tenth),
            "{context}"
        );
        assert_eq!(
            meta.times().modified(),
            fat_time(modified.date, modified.time, 0),
            "{context}"
        );
        assert_eq!(
            meta.times().accessed(),
            fat_time(file.accessed_date(), 0, 0),
            "{context}"
        );
        assert!(meta.times().modified().is_some(), "{context}");

        let node = fs.lookup(dir, name(&swap_case(text))).unwrap();
        assert_eq!(node, entry.node(), "{context}");
        assert_eq!(fs.node_metadata(node).unwrap(), meta, "{context}");

        if file.is_directory() {
            let v2_child = v2_dir.open_entry(file).unwrap();
            compared += compare(case, v2, &v2_child, fs, node);
            let parent = fs.parent(node).unwrap();
            assert_eq!(parent, dir, "{context}");
            fs.forget(parent);
        } else {
            let data = v2.read_file(file).unwrap().read_to_vec().unwrap();
            assert_eq!(read_all(fs, node), data, "{context}");
        }
        fs.forget(node);
        compared += 1;
    }
    compared
}

#[test]
fn listings_metadata_and_contents_match_v2() {
    for case in CASES {
        let image = common::build(case);
        let v2 = common::open_v2(&image);
        let mut fs = open(case, image);
        assert_eq!(fs.kind(), case.kind, "{}", case.name);
        let root = fs.root();
        let compared = compare(case, &v2, &v2.root_dir(), &mut fs, root);
        assert_eq!(compared, 11 + 2 + INNER_FILES, "{}", case.name);
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
        let v2 = common::open_v2(&image);
        let short = v2
            .root_dir()
            .find(LONG_NAME)
            .unwrap()
            .unwrap()
            .short_name()
            .raw_bytes();
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
        let v2 = common::open_v2(&image);
        let mut fs = open(case, image);
        let stats = fs.stats().unwrap();
        assert_eq!(stats.total_blocks(), empty.total_blocks());
        assert_eq!(stats.block_size(), empty.block_size());
        let used = empty.free_blocks() - stats.free_blocks();
        let min_used = (40_100 + 70_000) / u64::from(stats.block_size());
        assert!(used > min_used, "{}: {used} clusters used", case.name);
        if let Some(free) = v2.free_cluster_count() {
            assert_eq!(stats.free_blocks(), u64::from(free), "{}", case.name);
        }
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
        MountOptions::new().with_read_only(true),
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
        FsDriver::remove(&mut fs, root, name("README.TXT")),
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
fn cyclic_chains_end_instead_of_hanging() {
    let case = CASES[1];
    let mut image = common::build(case);
    let (inner, deep) = {
        let v2 = common::open_v2(&image);
        let nested = v2.root_dir().open_dir("Nested Dir").unwrap();
        let inner = nested.open_dir("inner").unwrap();
        let deep = inner.find("deep.bin").unwrap().unwrap().cluster().0;
        (nested.find("inner").unwrap().unwrap().cluster().0, deep)
    };
    let u16_at = |at: usize| u16::from_le_bytes([image[at], image[at + 1]]) as usize;
    let fat_start = u16_at(14) * u16_at(11);
    let fat_len = u16_at(22) * u16_at(11);
    for copy in 0..image[16] as usize {
        for cluster in [inner, deep] {
            let at = fat_start + copy * fat_len + cluster * 2;
            image[at..at + 2].copy_from_slice(&(cluster as u16).to_le_bytes());
        }
    }
    let mut fs = open(case, image);
    let root = fs.root();
    let nested = fs.lookup(root, name("Nested Dir")).unwrap();
    let inner = fs.lookup(nested, name("inner")).unwrap();
    assert!(list(&mut fs, inner).len() >= INNER_FILES);
    let file = fs.lookup(inner, name("deep.bin")).unwrap();
    assert_eq!(read_all(&mut fs, file).len(), 70_000);
}
