//! The `FatFs` write path, read back through a fresh `FatFs` mount and the
//! raw directory entries, and checked with the host `fsck` when it is
//! installed.

#[path = "common/fatfs.rs"]
mod common;

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::io::{Read as _, Write as _};

use common::{CASES, Case, Device, KANJI_NAME, fsck};
use hadris_fat::sync::FatFs;
use hadris_fat::{Ascii, CodePage, Cp437, MountOptions};
use hadris_fs::sync::{DriverExt, FileSystem, FsDriver, PathExt, Volume, copy_tree};
use hadris_fs::{
    Attributes, CivilDate, CivilTime, Clock, DateTime, DirCursor, ErrorKind, FileTimes, FileType,
    FixedTable, HeapTable, Name, NameBuf, NewNode, NoClock, NodeId, NodeTable, OpenOptions,
    RemoveKind, RenameFlags, SetMetadata,
};
use hadris_storage::sync::BlockDevice;
use hadris_storage::{BlockIndex, BlockSize, OutOfRange, WriteError};

type Fs<T = HeapTable, C = NoClock, P = Ascii> = FatFs<Device, T, C, P>;

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

fn list<D: BlockDevice, T: NodeTable, C: Clock, P: CodePage>(
    fs: &mut FatFs<D, T, C, P>,
    dir: NodeId,
) -> Vec<(String, FileType)> {
    let mut cursor = DirCursor::start();
    let mut buf = NameBuf::new();
    let mut out = Vec::new();
    while let Some(entry) = fs.read_dir_entry(dir, &mut cursor, &mut buf).unwrap() {
        out.push((
            buf.as_name().unwrap().to_str().unwrap().to_owned(),
            entry.file_type(),
        ));
    }
    out
}

fn read_all<D: BlockDevice, T: NodeTable, C: Clock, P: CodePage>(
    fs: &mut FatFs<D, T, C, P>,
    node: NodeId,
) -> Vec<u8> {
    let mut out = Vec::new();
    let mut chunk = [0u8; 1000];
    loop {
        let n = fs.read_at(node, out.len() as u64, &mut chunk).unwrap();
        if n == 0 {
            return out;
        }
        out.extend_from_slice(&chunk[..n]);
    }
}

fn create<T: NodeTable, C: Clock, P: CodePage>(
    fs: &mut Fs<T, C, P>,
    dir: NodeId,
    text: &str,
    kind: NewNode<'_>,
) -> NodeId {
    fs.create(dir, name(text), kind, &SetMetadata::new())
        .unwrap()
}

fn write_all<T: NodeTable, C: Clock, P: CodePage>(
    fs: &mut Fs<T, C, P>,
    node: NodeId,
    offset: u64,
    data: &[u8],
) {
    let mut done = 0;
    for chunk in data.chunks(777) {
        assert_eq!(
            fs.write_at(node, offset + done, chunk).unwrap(),
            chunk.len()
        );
        done += chunk.len() as u64;
    }
}

fn free(fs: &mut Fs<impl NodeTable>) -> u64 {
    fs.stats().unwrap().free_blocks()
}

fn image<T: NodeTable, C: Clock, P: CodePage>(fs: Fs<T, C, P>) -> Vec<u8> {
    fs.into_inner().into_inner()
}

/// The names in the directory at `path`, read through a fresh mount.
fn fresh_names(case: Case, image: &[u8], path: &str) -> Vec<String> {
    common::names(&mut common::mount(case, image), path)
}

/// The contents of the file at `path`, read through a fresh mount.
fn fresh_read(case: Case, image: &[u8], path: &str) -> Vec<u8> {
    common::read(&mut common::mount(case, image), path)
}

/// The raw short name of every live entry, keyed by the long name when it
/// has one and by its 8.3 name with the NT case bits applied otherwise. The
/// whole image is scanned, and long-name fragments are matched to short
/// entries by checksum, so directories need not be contiguous.
fn short_names(image: &[u8]) -> Vec<(String, [u8; 11])> {
    let live = |entry: &&[u8]| entry[0] != 0 && entry[0] != 0xE5;
    let fragments: Vec<(u8, u8, Vec<u16>)> = image
        .chunks_exact(32)
        .filter(live)
        .filter(|entry| entry[11] == 0x0F && entry[12] == 0)
        .map(|entry| {
            let units = [1..11, 14..26, 28..32]
                .into_iter()
                .flat_map(|range| entry[range].chunks_exact(2))
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .take_while(|&unit| unit != 0)
                .collect();
            (entry[13], entry[0] & 0x1F, units)
        })
        .collect();
    let mut out = Vec::new();
    for entry in image.chunks_exact(32).filter(live) {
        if entry[11] == 0x0F {
            continue;
        }
        let short: [u8; 11] = entry[..11].try_into().unwrap();
        let sum = short
            .iter()
            .fold(0u8, |sum, &byte| sum.rotate_right(1).wrapping_add(byte));
        let mut parts: Vec<&(u8, u8, Vec<u16>)> = fragments
            .iter()
            .filter(|(check, ..)| *check == sum)
            .collect();
        parts.sort_by_key(|(_, seq, _)| *seq);
        let text = if parts.is_empty() {
            let case = |bytes: &[u8], lower: bool| {
                let text = String::from_utf8_lossy(bytes).trim_end().to_owned();
                if lower { text.to_lowercase() } else { text }
            };
            let base = case(&short[..8], entry[12] & 0x08 != 0);
            let ext = case(&short[8..], entry[12] & 0x10 != 0);
            if ext.is_empty() {
                base
            } else {
                format!("{base}.{ext}")
            }
        } else {
            let units: Vec<u16> = parts.iter().flat_map(|(.., units)| units.clone()).collect();
            String::from_utf16_lossy(&units)
        };
        out.push((text, short));
    }
    out
}

fn short_of(image: &[u8], text: &str) -> [u8; 11] {
    short_names(image)
        .into_iter()
        .find(|(name, _)| name == text)
        .unwrap_or_else(|| panic!("no entry named {text:?} in {:?}", short_names(image)))
        .1
}

fn fat_time(year: i32, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> DateTime {
    DateTime::from_civil(
        CivilDate::new(year, month, day).unwrap(),
        CivilTime::new(hour, minute, second).unwrap(),
        None,
    )
    .unwrap()
}

/// Writes a tree through `FatFs` on a blank volume and returns the image.
fn populate(case: Case) -> Vec<u8> {
    let mut fs = open(case, common::blank(case));
    let root = fs.root();
    let readme = create(&mut fs, root, "README.TXT", NewNode::File);
    write_all(&mut fs, readme, 0, b"hello fat");
    let lower = create(&mut fs, root, "lower.txt", NewNode::File);
    write_all(&mut fs, lower, 0, b"lowercase");
    let long = create(&mut fs, root, "A long file name.txt", NewNode::File);
    write_all(&mut fs, long, 0, &common::payload(70_000, 1));
    let unicode = create(&mut fs, root, common::UNICODE_NAME, NewNode::File);
    write_all(&mut fs, unicode, 0, b"unicode");
    let sparse = create(&mut fs, root, "sparse.bin", NewNode::File);
    write_all(&mut fs, sparse, 10_000, b"tail");
    let grown = create(&mut fs, root, "grown.bin", NewNode::File);
    write_all(&mut fs, grown, 0, b"head");
    fs.set_len(grown, 9_000).unwrap();
    let empty = create(&mut fs, root, "empty.dat", NewNode::File);
    let dir = create(&mut fs, root, "Nested Dir", NewNode::Dir);
    let inner = create(&mut fs, dir, "inner", NewNode::Dir);
    let deep = create(&mut fs, inner, "deep.bin", NewNode::File);
    write_all(&mut fs, deep, 0, &common::payload(33_333, 5));
    for node in [
        readme, lower, long, unicode, sparse, grown, empty, dir, inner, deep,
    ] {
        fs.forget(node);
    }
    fs.sync().unwrap();
    assert_eq!(fs.open_nodes(), 1);
    image(fs)
}

fn sparse_contents() -> Vec<u8> {
    let mut data = vec![0u8; 10_000];
    data.extend_from_slice(b"tail");
    data
}

fn grown_contents() -> Vec<u8> {
    let mut data = b"head".to_vec();
    data.resize(9_000, 0);
    data
}

#[test]
fn written_trees_read_back_through_a_fresh_mount() {
    for case in CASES {
        let image = populate(case);
        let mut fs = open(case, image.clone());
        let root = fs.root();
        let names: Vec<String> = list(&mut fs, root).into_iter().map(|(n, _)| n).collect();
        assert_eq!(
            names,
            [
                "README.TXT",
                "lower.txt",
                "A long file name.txt",
                common::UNICODE_NAME,
                "sparse.bin",
                "grown.bin",
                "empty.dat",
                "Nested Dir",
            ],
            "{}",
            case.name
        );
        assert_eq!(fs.read_to_vec("/sparse.bin").unwrap(), sparse_contents());
        assert_eq!(fs.read_to_vec("/grown.bin").unwrap(), grown_contents());
        assert_eq!(
            fs.read_to_vec("/nested dir/INNER/deep.bin").unwrap(),
            common::payload(33_333, 5)
        );
        let inner = fs.resolve("/Nested Dir/inner").unwrap();
        let up = fs.parent(inner).unwrap();
        assert_eq!(up, fs.resolve("/Nested Dir").unwrap());

        let mut fresh = common::mount(case, &image);
        assert_eq!(common::names(&mut fresh, "/"), names, "{}", case.name);
        assert_eq!(common::read(&mut fresh, "/README.TXT"), b"hello fat");
        assert_eq!(common::read(&mut fresh, "/lower.txt"), b"lowercase");
        assert_eq!(
            common::read(&mut fresh, "/A long file name.txt"),
            common::payload(70_000, 1)
        );
        assert_eq!(
            common::read(&mut fresh, &format!("/{}", common::UNICODE_NAME)),
            b"unicode"
        );
        assert_eq!(common::read(&mut fresh, "/empty.dat"), b"");
        assert_eq!(
            common::read(&mut fresh, "/Nested Dir/inner/deep.bin"),
            common::payload(33_333, 5)
        );
        let free = hadris_fat::sync::check(&mut fresh).unwrap().free_clusters();
        assert_eq!(u64::from(free), fs.stats().unwrap().free_blocks());
        fsck(&image, case.name);
    }
}

#[test]
fn short_names_and_case_bits() {
    let case = CASES[1];
    let image = populate(case);
    let short = |text: &str| short_of(&image, text);
    assert_eq!(&short("README.TXT"), b"README  TXT");
    assert_eq!(&short("lower.txt"), b"LOWER   TXT");
    let at = image
        .chunks_exact(32)
        .position(|entry| &entry[..11] == b"LOWER   TXT")
        .unwrap()
        * 32;
    assert_eq!(image[at + 12] & 0x18, 0x18, "lowercase NT case bits");
    assert_ne!(image[at - 32 + 11], 0x0F, "lower.txt has no long name");
    assert_eq!(&short("A long file name.txt"), b"ALONGF~1TXT");
    assert_eq!(&short("Nested Dir"), b"NESTED~1   ");
}

#[test]
fn short_name_collisions_get_numeric_then_hashed_tails() {
    for case in [CASES[0], CASES[2]] {
        let mut fs = open(case, common::blank(case));
        let root = fs.root();
        let texts: Vec<String> = (0..15).map(|i| format!("long file name {i}.txt")).collect();
        for text in &texts {
            let node = create(&mut fs, root, text, NewNode::File);
            write_all(&mut fs, node, 0, text.as_bytes());
            fs.forget(node);
        }
        fs.sync().unwrap();
        let second = fs.lookup(root, name("LONGFI~2.TXT")).unwrap();
        assert_eq!(read_all(&mut fs, second), texts[1].as_bytes());
        assert_eq!(
            fs.create(
                root,
                name("LONGFI~1.TXT"),
                NewNode::File,
                &SetMetadata::new()
            )
            .unwrap_err()
            .kind(),
            ErrorKind::AlreadyExists
        );
        fs.forget(second);

        let image = image(fs);
        let shorts: Vec<[u8; 11]> = texts.iter().map(|text| short_of(&image, text)).collect();
        for (i, short) in shorts.iter().take(4).enumerate() {
            assert_eq!(short, format!("LONGFI~{}TXT", i + 1).as_bytes());
        }
        for short in &shorts[4..] {
            let shown = String::from_utf8_lossy(short);
            assert_eq!(&short[..2], b"LO", "{shown}");
            assert!(short[2..6].iter().all(u8::is_ascii_hexdigit), "{shown}");
            assert_eq!(short[6], b'~', "{shown}");
            assert!((b'1'..=b'9').contains(&short[7]), "{shown}");
            assert_eq!(&short[8..], b"TXT", "{shown}");
        }
        let mut unique = shorts.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), shorts.len());
        fsck(&image, case.name);
    }
}

#[test]
fn long_names_up_to_255_units() {
    let case = CASES[2];
    let mut fs = open(case, common::blank(case));
    let root = fs.root();
    let longest = "\u{E9}".repeat(251) + ".txt";
    let texts = [
        longest.clone(),
        "x".repeat(255),
        "\u{1F600}".repeat(127),
        "twelve chars".to_owned(),
        "thirteen char".to_owned(),
    ];
    for text in &texts {
        let node = create(&mut fs, root, text, NewNode::File);
        write_all(&mut fs, node, 0, text.as_bytes());
        fs.forget(node);
    }
    for bad in ["y".repeat(256), "\u{1F600}".repeat(128)] {
        assert_eq!(
            fs.create(root, name(&bad), NewNode::File, &SetMetadata::new())
                .unwrap_err()
                .kind(),
            ErrorKind::NameTooLong
        );
    }
    for bad in [
        "a:b",
        "trailing.",
        "trailing ",
        "trailing. .",
        "tab\tname",
        "q?",
    ] {
        assert_eq!(
            fs.create(root, name(bad), NewNode::File, &SetMetadata::new())
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidInput,
            "{bad}"
        );
        assert_eq!(
            fs.rename(
                root,
                name("twelve chars"),
                root,
                name(bad),
                RenameFlags::empty()
            )
            .unwrap_err()
            .kind(),
            ErrorKind::InvalidInput,
            "{bad}"
        );
    }
    assert_eq!(
        fs.lookup(root, name("twelve chars.")).unwrap_err().kind(),
        ErrorKind::NotFound,
        "trailing dots are not stripped on lookup either"
    );
    fs.sync().unwrap();
    let names: Vec<String> = list(&mut fs, root).into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, texts);
    let node = fs.lookup(root, name(&longest.to_uppercase())).unwrap();
    assert_eq!(read_all(&mut fs, node), longest.as_bytes());
    fs.forget(node);

    let image = image(fs);
    assert_eq!(fresh_names(case, &image, "/"), texts);
    fsck(&image, "long names");
}

#[test]
fn directories_grow_past_one_cluster() {
    for case in CASES {
        let mut fs = open(case, common::blank(case));
        let root = fs.root();
        let dir = create(&mut fs, root, "many", NewNode::Dir);
        let before = free(&mut fs);
        let count = 120;
        let texts: Vec<String> = (0..count)
            .map(|i| format!("a fairly long file name number {i:04}.data"))
            .collect();
        for text in &texts {
            let node = create(&mut fs, dir, text, NewNode::File);
            fs.forget(node);
        }
        let bytes = count as u64 * 5 * 32;
        let cluster = u64::from(fs.stats().unwrap().block_size());
        let used = before - free(&mut fs);
        assert_eq!(used + 1, bytes.div_ceil(cluster), "{}", case.name);
        let names: Vec<String> = list(&mut fs, dir).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, texts, "{}", case.name);
        let last = fs.lookup(dir, name(texts.last().unwrap())).unwrap();
        fs.forget(last);
        fs.forget(dir);
        fs.sync().unwrap();

        let image = image(fs);
        assert_eq!(fresh_names(case, &image, "/many"), texts);
        fsck(&image, case.name);
    }
}

#[test]
fn fixed_root_fills_up_with_no_space_and_no_change() {
    let case = CASES[0];
    let mut fs = open(case, common::blank(case));
    let root = fs.root();
    let mut created = 0;
    let err = loop {
        match fs.create(
            root,
            name(&format!("F{created}.TXT")),
            NewNode::File,
            &SetMetadata::new(),
        ) {
            Ok(node) => {
                fs.forget(node);
                created += 1;
            }
            Err(err) => break err,
        }
    };
    assert_eq!(err.kind(), ErrorKind::NoSpace);
    assert!(created >= 200, "{created}");
    let full = image(fs);
    let mut fs = open(case, full.clone());
    let root = fs.root();
    let stats = fs.stats().unwrap();
    for text in ["G.TXT", "a long name that needs slots.txt"] {
        assert_eq!(
            fs.create(root, name(text), NewNode::File, &SetMetadata::new())
                .unwrap_err()
                .kind(),
            ErrorKind::NoSpace
        );
        assert_eq!(
            fs.create(root, name(text), NewNode::Dir, &SetMetadata::new())
                .unwrap_err()
                .kind(),
            ErrorKind::NoSpace
        );
    }
    assert_eq!(fs.stats().unwrap(), stats);
    assert_eq!(fs.open_nodes(), 1);
    assert_eq!(image(fs), full);
    fsck(&full, "full root");
}

/// Replacing a target reuses its slots, so a full FAT12/16 root directory
/// still takes a new name that fits them.
#[test]
fn replace_in_a_full_root_reuses_the_target_slots() {
    let case = CASES[0];
    let mut fs = open(case, common::blank(case));
    let root = fs.root();
    for text in ["Target with a long name.txt", "SHORT.TXT"] {
        let node = create(&mut fs, root, text, NewNode::File);
        fs.forget(node);
    }
    let mut created = 0;
    while let Ok(node) = fs.create(
        root,
        name(&format!("F{created}.TXT")),
        NewNode::File,
        &SetMetadata::new(),
    ) {
        fs.forget(node);
        created += 1;
    }
    let full = image(fs);

    let mut fs = open(case, full.clone());
    let root = fs.root();
    assert_eq!(
        fs.rename(
            root,
            name("F0.TXT"),
            root,
            name("Short.txt"),
            RenameFlags::empty(),
        )
        .unwrap_err()
        .kind(),
        ErrorKind::NoSpace,
        "a long name does not fit the one slot of SHORT.TXT"
    );
    assert_eq!(image(fs), full);

    let mut fs = open(case, full);
    let root = fs.root();
    fs.rename(
        root,
        name("F0.TXT"),
        root,
        name("TARGET WITH A LONG NAME.txt"),
        RenameFlags::empty(),
    )
    .unwrap();
    fs.sync().unwrap();
    let image = image(fs);
    let names = fresh_names(case, &image, "/");
    assert_eq!(names[0], "TARGET WITH A LONG NAME.txt");
    assert_eq!(names.len(), created + 1);
    fsck(&image, "replace in a full root");
}

#[test]
fn rename_keeps_the_id_and_moves_directories() {
    for case in CASES {
        let mut fs = open(case, populate(case));
        let root = fs.root();
        let file = fs.lookup(root, name("README.TXT")).unwrap();
        let dir = fs.lookup(root, name("Nested Dir")).unwrap();
        let inner = fs.lookup(dir, name("inner")).unwrap();

        fs.rename(
            root,
            name("readme.txt"),
            dir,
            name("Moved Readme.txt"),
            RenameFlags::empty(),
        )
        .unwrap();
        assert_eq!(
            fs.lookup(root, name("README.TXT")).unwrap_err().kind(),
            ErrorKind::NotFound
        );
        assert_eq!(fs.lookup(dir, name("moved readme.TXT")).unwrap(), file);
        fs.forget(file);
        assert_eq!(read_all(&mut fs, file), b"hello fat");
        write_all(&mut fs, file, 9, b"!");

        fs.rename(
            dir,
            name("inner"),
            root,
            name("Top Level"),
            RenameFlags::empty(),
        )
        .unwrap();
        assert_eq!(fs.lookup(root, name("top level")).unwrap(), inner);
        fs.forget(inner);
        assert_eq!(fs.parent(inner).unwrap(), root);
        let deep = fs.lookup(inner, name("deep.bin")).unwrap();
        assert_eq!(read_all(&mut fs, deep), common::payload(33_333, 5));
        fs.forget(deep);

        fs.rename(
            root,
            name("lower.txt"),
            root,
            name("LOWER.txt"),
            RenameFlags::empty(),
        )
        .unwrap();
        fs.rename(
            root,
            name("sparse.bin"),
            root,
            name("sparse.bin"),
            RenameFlags::empty(),
        )
        .unwrap();
        assert_eq!(
            fs.rename(
                root,
                name("Top Level"),
                inner,
                name("x"),
                RenameFlags::empty()
            )
            .unwrap_err()
            .kind(),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            fs.rename(root, name("missing"), root, name("x"), RenameFlags::empty())
                .unwrap_err()
                .kind(),
            ErrorKind::NotFound
        );
        let names: Vec<String> = list(&mut fs, root).into_iter().map(|(n, _)| n).collect();
        assert!(names.contains(&"LOWER.txt".to_owned()), "{names:?}");
        assert!(names.contains(&"Top Level".to_owned()), "{names:?}");
        for node in [file, dir, inner] {
            fs.forget(node);
        }
        fs.sync().unwrap();
        assert_eq!(fs.open_nodes(), 1);

        let image = image(fs);
        let mut fresh = common::mount(case, &image);
        assert_eq!(
            common::read(&mut fresh, "/Top Level/deep.bin"),
            common::payload(33_333, 5)
        );
        assert_eq!(
            common::read(&mut fresh, "/Nested Dir/Moved Readme.txt"),
            b"hello fat!"
        );
        assert_eq!(
            common::names(&mut fresh, "/Nested Dir"),
            ["Moved Readme.txt"]
        );
        assert_eq!(common::read(&mut fresh, "/LOWER.txt"), b"lowercase");
        fsck(&image, case.name);
    }
}

/// The id `read_dir_entry` reports for `text` in `dir`.
fn listed_id(fs: &mut Fs, dir: NodeId, text: &str) -> NodeId {
    let mut cursor = DirCursor::start();
    let mut buf = NameBuf::new();
    while let Some(entry) = fs.read_dir_entry(dir, &mut cursor, &mut buf).unwrap() {
        if buf.as_bytes() == text.as_bytes() {
            return entry.node();
        }
    }
    panic!("{text} is not listed");
}

#[test]
fn listed_ids_are_the_ids_lookup_pins() {
    let case = CASES[1];
    let mut fs = open(case, common::blank(case));
    let root = fs.root();
    let a = create(&mut fs, root, "a", NewNode::File);
    fs.rename(root, name("a"), root, name("b"), RenameFlags::empty())
        .unwrap();
    let c = create(&mut fs, root, "c", NewNode::File);
    assert_eq!(c.get() & ((1 << 40) - 1), a.get(), "c takes a's old slot");
    assert_eq!(c.get() >> 40, 1, "a still holds the slot's first id");
    fs.forget(c);
    let listed = listed_id(&mut fs, root, "c");
    let looked_up = fs.lookup(root, name("c")).unwrap();
    assert_eq!(listed, looked_up);
    assert_eq!(listed_id(&mut fs, root, "b"), a);
    assert!(listed.get() != 0 && listed.get() < 1 << 63);
    fs.forget(looked_up);
    fs.forget(a);
    assert_eq!(fs.open_nodes(), 1);
    assert_eq!(
        listed_id(&mut fs, root, "c"),
        a,
        "with a forgotten, c takes the slot's first id"
    );
}

#[test]
fn replacing_a_pinned_target_unlinks_it() {
    let case = CASES[1];
    let mut fs = open(case, populate(case));
    let root = fs.root();
    let target = fs.lookup(root, name("lower.txt")).unwrap();
    let source = fs.lookup(root, name("README.TXT")).unwrap();
    fs.rename(
        root,
        name("README.TXT"),
        root,
        name("lower.txt"),
        RenameFlags::empty(),
    )
    .unwrap();
    assert_eq!(
        fs.node_metadata(target).unwrap_err().kind(),
        ErrorKind::NotFound
    );
    assert_eq!(fs.lookup(root, name("lower.txt")).unwrap(), source);
    fs.forget(source);
    fs.forget(source);
    fs.forget(target);
    assert_eq!(fs.open_nodes(), 1);
    fs.sync().unwrap();
    fsck(&image(fs), case.name);
}

#[test]
fn rename_replaces_or_refuses_existing_targets() {
    let case = CASES[2];
    let mut fs = open(case, populate(case));
    let root = fs.root();
    let long = fs.lookup(root, name("A long file name.txt")).unwrap();
    fs.open_node(long).unwrap();
    assert_eq!(
        fs.rename(
            root,
            name("README.TXT"),
            root,
            name("lower.txt"),
            RenameFlags::NO_REPLACE
        )
        .unwrap_err()
        .kind(),
        ErrorKind::AlreadyExists
    );
    assert_eq!(
        fs.rename(
            root,
            name("README.TXT"),
            root,
            name("A LONG FILE NAME.TXT"),
            RenameFlags::empty()
        )
        .unwrap_err()
        .kind(),
        ErrorKind::Busy
    );
    assert_eq!(
        fs.rename(
            root,
            name("README.TXT"),
            root,
            name("Nested Dir"),
            RenameFlags::empty()
        )
        .unwrap_err()
        .kind(),
        ErrorKind::IsADirectory
    );
    let empty_dir = create(&mut fs, root, "empty dir", NewNode::Dir);
    fs.forget(empty_dir);
    assert_eq!(
        fs.rename(
            root,
            name("empty dir"),
            root,
            name("lower.txt"),
            RenameFlags::empty()
        )
        .unwrap_err()
        .kind(),
        ErrorKind::NotADirectory
    );
    let nested = fs.resolve("/Nested Dir").unwrap();
    fs.open_node(nested).unwrap();
    assert_eq!(
        fs.rename(
            root,
            name("empty dir"),
            root,
            name("nested dir"),
            RenameFlags::empty()
        )
        .unwrap_err()
        .kind(),
        ErrorKind::Busy
    );
    fs.close_node(nested);
    fs.forget(nested);
    assert_eq!(
        fs.rename(
            root,
            name("empty dir"),
            root,
            name("nested dir"),
            RenameFlags::empty()
        )
        .unwrap_err()
        .kind(),
        ErrorKind::DirectoryNotEmpty
    );
    assert_eq!(
        fs.rename(
            root,
            name("a"),
            root,
            name("b"),
            RenameFlags::from_bits_retain(2)
        )
        .unwrap_err()
        .kind(),
        ErrorKind::Unsupported
    );

    fs.close_node(long);
    fs.forget(long);
    let before = free(&mut fs);
    fs.rename(
        root,
        name("A long file name.txt"),
        root,
        name("LOWER.TXT"),
        RenameFlags::empty(),
    )
    .unwrap();
    assert_eq!(
        fs.read_to_vec("/lower.txt").unwrap(),
        common::payload(70_000, 1)
    );
    let freed = free(&mut fs) - before;
    assert_eq!(freed, 1, "the old lower.txt cluster");
    let names: Vec<String> = list(&mut fs, root).into_iter().map(|(n, _)| n).collect();
    assert!(names.contains(&"LOWER.TXT".to_owned()), "{names:?}");
    assert!(!names.contains(&"lower.txt".to_owned()), "{names:?}");
    fs.sync().unwrap();
    let replaced = image(fs);
    assert_eq!(&short_of(&replaced, "LOWER.TXT"), b"LOWER   TXT");
    fsck(&replaced, "replace in another case");

    let mut fs = open(case, common::build(case));
    let root = fs.root();
    fs.rename(
        root,
        name("README.TXT"),
        root,
        name(KANJI_NAME),
        RenameFlags::empty(),
    )
    .unwrap();
    assert_eq!(
        fs.read_to_vec(&format!("/{KANJI_NAME}")).unwrap(),
        b"hello fat"
    );
    fs.sync().unwrap();
    let image = image(fs);
    assert!(
        !image
            .chunks_exact(32)
            .any(|entry| &entry[..11] == b"\x05ABC    TXT"),
        "the replaced entry is gone"
    );
    let names = fresh_names(case, &image, "/");
    assert!(!names.contains(&"README.TXT".to_owned()));
    assert_eq!(names.iter().filter(|n| *n == KANJI_NAME).count(), 1);
    fsck(&image, "replace");
}

/// A replaced target gives way to the name the caller asked for, even when
/// that name is the target's own short alias or needs more or fewer slots.
#[test]
fn rename_onto_a_target_takes_the_requested_name() {
    for case in [CASES[0], CASES[2]] {
        let mut fs = open(case, common::blank(case));
        let root = fs.root();
        let dir = create(&mut fs, root, "Sub Directory", NewNode::Dir);
        for (parent, text) in [
            (root, "Target long name.txt"),
            (root, "short.txt"),
            (root, "source one.bin"),
            (root, "a"),
            (dir, "Moved Target Name.txt"),
        ] {
            let node = create(&mut fs, parent, text, NewNode::File);
            write_all(&mut fs, node, 0, text.as_bytes());
            fs.forget(node);
        }
        fs.forget(dir);

        fs.rename(
            root,
            name("short.txt"),
            root,
            name("TARGET~1.TXT"),
            RenameFlags::empty(),
        )
        .unwrap();
        fs.rename(
            root,
            name("source one.bin"),
            root,
            name("A"),
            RenameFlags::empty(),
        )
        .unwrap();
        fs.rename(
            root,
            name("A"),
            dir,
            name("MOVED TARGET NAME.TXT"),
            RenameFlags::empty(),
        )
        .unwrap();
        fs.sync().unwrap();

        let image = image(fs);
        assert_eq!(
            fresh_names(case, &image, "/"),
            ["Sub Directory", "TARGET~1.TXT"]
        );
        assert_eq!(fresh_read(case, &image, "/TARGET~1.TXT"), b"short.txt");
        assert_eq!(
            fresh_names(case, &image, "/Sub Directory"),
            ["MOVED TARGET NAME.TXT"]
        );
        assert_eq!(
            fresh_read(case, &image, "/Sub Directory/moved target name.txt"),
            b"source one.bin"
        );
        assert_eq!(&short_of(&image, "TARGET~1.TXT"), b"TARGET~1TXT");
        fsck(&image, case.name);
    }
}

#[test]
fn remove_files_and_directories() {
    let case = CASES[1];
    let mut fs = open(case, populate(case));
    let root = fs.root();
    let before = free(&mut fs);
    let file = fs.lookup(root, name("a long file name.txt")).unwrap();
    fs.open_node(file).unwrap();
    assert_eq!(
        fs.remove(root, name("A long file name.txt"), RemoveKind::Any)
            .unwrap_err()
            .kind(),
        ErrorKind::Busy
    );
    fs.close_node(file);
    fs.remove(root, name("ALONGF~1.TXT"), RemoveKind::File)
        .unwrap();
    assert_eq!(
        fs.node_metadata(file).unwrap_err().kind(),
        ErrorKind::NotFound
    );
    assert_eq!(
        fs.read_at(file, 0, &mut [0u8; 4]).unwrap_err().kind(),
        ErrorKind::NotFound
    );
    assert_eq!(fs.open_node(file).unwrap_err().kind(), ErrorKind::NotFound);
    assert_eq!(fs.open_nodes(), 2, "a removed node keeps its pin");
    fs.forget(file);
    assert!(free(&mut fs) > before);
    assert_eq!(
        fs.lookup(root, name("A long file name.txt"))
            .unwrap_err()
            .kind(),
        ErrorKind::NotFound
    );
    assert_eq!(
        fs.remove(root, name("Nested Dir"), RemoveKind::Any)
            .unwrap_err()
            .kind(),
        ErrorKind::DirectoryNotEmpty
    );
    assert_eq!(
        fs.remove(root, name("missing"), RemoveKind::Any)
            .unwrap_err()
            .kind(),
        ErrorKind::NotFound
    );
    assert_eq!(
        fs.remove(root, name("Nested Dir"), RemoveKind::File)
            .unwrap_err()
            .kind(),
        ErrorKind::IsADirectory
    );
    assert_eq!(
        fs.remove(root, name("README.TXT"), RemoveKind::Dir)
            .unwrap_err()
            .kind(),
        ErrorKind::NotADirectory
    );

    let pending = fs.lookup(root, name("grown.bin")).unwrap();
    write_all(&mut fs, pending, 20_000, b"more");
    fs.forget(pending);
    assert_eq!(fs.open_nodes(), 2, "an unsynced size keeps the node");
    fs.remove(root, name("grown.bin"), RemoveKind::Any).unwrap();
    assert_eq!(fs.open_nodes(), 1);

    let dir = fs.resolve("/Nested Dir").unwrap();
    let inner = fs.lookup(dir, name("inner")).unwrap();
    fs.remove(inner, name("deep.bin"), RemoveKind::File)
        .unwrap();
    fs.forget(inner);
    fs.remove(dir, name("inner"), RemoveKind::Dir).unwrap();
    fs.forget(dir);
    fs.remove(root, name("Nested Dir"), RemoveKind::Any)
        .unwrap();
    for text in [
        "README.TXT",
        "lower.txt",
        common::UNICODE_NAME,
        "sparse.bin",
        "empty.dat",
    ] {
        fs.remove(root, name(text), RemoveKind::Any).unwrap();
    }
    assert!(list(&mut fs, root).is_empty());
    fs.sync().unwrap();
    let image = image(fs);
    let mut blank = open(case, common::blank(case));
    let mut fs = open(case, image.clone());
    assert_eq!(fs.stats().unwrap(), blank.stats().unwrap());
    assert!(fresh_names(case, &image, "/").is_empty());
    fsck(&image, "removed");
}

#[test]
fn set_len_shrinks_frees_and_grows_zeroed() {
    for case in CASES {
        let mut fs = open(case, common::blank(case));
        let root = fs.root();
        let empty = free(&mut fs);
        let cluster = u64::from(fs.stats().unwrap().block_size());
        let node = create(&mut fs, root, "data.bin", NewNode::File);
        let data = common::payload(100_000, 9);
        write_all(&mut fs, node, 0, &data);
        assert_eq!(empty - free(&mut fs), 100_000u64.div_ceil(cluster));

        fs.set_len(node, 1_000).unwrap();
        assert_eq!(empty - free(&mut fs), 1_000u64.div_ceil(cluster));
        fs.set_len(node, 5_000).unwrap();
        let mut expected = data[..1_000].to_vec();
        expected.resize(5_000, 0);
        assert_eq!(read_all(&mut fs, node), expected, "{}", case.name);
        fs.set_len(node, 0).unwrap();
        assert_eq!(free(&mut fs), empty);
        assert_eq!(fs.node_metadata(node).unwrap().len(), 0);
        write_all(&mut fs, node, 3, b"abc");
        assert_eq!(read_all(&mut fs, node), b"\0\0\0abc");
        assert_eq!(
            fs.set_len(node, u64::from(u32::MAX) + 1)
                .unwrap_err()
                .kind(),
            ErrorKind::FileTooLarge
        );
        assert_eq!(
            fs.write_at(node, u64::from(u32::MAX), b"x")
                .unwrap_err()
                .kind(),
            ErrorKind::FileTooLarge
        );
        fs.forget(node);
        fs.sync().unwrap();
        let image = image(fs);
        assert_eq!(fresh_read(case, &image, "/data.bin"), b"\0\0\0abc");
        fsck(&image, case.name);
    }
}

#[test]
fn no_space_changes_nothing() {
    let case = CASES[0];
    let mut fs = open(case, common::blank(case));
    let root = fs.root();
    let node = create(&mut fs, root, "big.bin", NewNode::File);
    let free_bytes = fs.stats().unwrap().free_bytes();
    write_all(&mut fs, node, 0, b"start");
    fs.sync_node(node).unwrap();
    fs.forget(node);
    let before = image(fs);
    let mut fs = open(case, before.clone());
    let node = fs.lookup(root, name("big.bin")).unwrap();
    let stats = fs.stats().unwrap();
    let huge = vec![7u8; free_bytes as usize + 1];
    assert_eq!(
        fs.write_at(node, 0, &huge).unwrap_err().kind(),
        ErrorKind::NoSpace
    );
    assert_eq!(
        fs.set_len(node, free_bytes + 1).unwrap_err().kind(),
        ErrorKind::NoSpace
    );
    assert_eq!(fs.stats().unwrap(), stats);
    assert_eq!(read_all(&mut fs, node), b"start");
    fs.forget(node);
    fs.sync().unwrap();
    let after = image(fs);
    let mut fs = open(case, after);
    assert_eq!(fs.stats().unwrap(), stats);
    let node = fs.lookup(root, name("big.bin")).unwrap();
    fs.write_at(node, 0, &vec![1u8; free_bytes as usize - 512])
        .unwrap();
}

#[test]
fn full_table_changes_nothing_on_disk() {
    let case = CASES[1];
    let before = populate(case);
    let mut fs: Fs<FixedTable<1>> = FatFs::open_with(
        common::device(case, before.clone()),
        MountOptions::new().with_table(FixedTable::new()),
    )
    .unwrap();
    let root = fs.root();
    let held = fs.lookup(root, name("README.TXT")).unwrap();
    for kind in [NewNode::File, NewNode::Dir] {
        assert_eq!(
            fs.create(root, name("new entry"), kind, &SetMetadata::new())
                .unwrap_err()
                .kind(),
            ErrorKind::LimitExceeded
        );
    }
    assert_eq!(
        fs.lookup(root, name("lower.txt")).unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );
    fs.forget(held);
    assert_eq!(fs.open_nodes(), 1);
    assert_eq!(image(fs), before);
}

#[test]
fn unsupported_kinds_and_metadata() {
    let case = CASES[2];
    let mut fs = open(case, populate(case));
    let root = fs.root();
    let meta = SetMetadata::new();
    assert_eq!(
        fs.create(root, name("link"), NewNode::Symlink(b"target"), &meta)
            .unwrap_err()
            .kind(),
        ErrorKind::Unsupported
    );
    assert_eq!(
        fs.create(root, name("README.TXT"), NewNode::File, &meta)
            .unwrap_err()
            .kind(),
        ErrorKind::AlreadyExists
    );
    assert_eq!(
        fs.create(root, name("readme.txt"), NewNode::Dir, &meta)
            .unwrap_err()
            .kind(),
        ErrorKind::AlreadyExists
    );
    let created = fat_time(2001, 2, 3, 4, 5, 6);
    let modified = fat_time(2024, 12, 31, 23, 59, 58);
    let accessed = fat_time(2020, 6, 1, 0, 0, 0);
    let times = FileTimes::new()
        .with_created(created)
        .with_modified(modified)
        .with_accessed(accessed);
    let changes = SetMetadata::new()
        .with_times(times)
        .with_attributes(Attributes::HIDDEN | Attributes::READ_ONLY)
        .with_mode(hadris_fs::Mode::new(0o600))
        .with_uid(1000);
    let node = fs
        .create(root, name("stamped.txt"), NewNode::File, &changes)
        .unwrap();
    let meta = fs.node_metadata(node).unwrap();
    assert_eq!(meta.times().created(), Some(created));
    assert_eq!(meta.times().modified(), Some(modified));
    assert_eq!(meta.times().accessed(), Some(accessed));
    assert_eq!(
        meta.attributes(),
        Attributes::HIDDEN | Attributes::READ_ONLY
    );
    assert_eq!(meta.permissions(), None);

    write_all(&mut fs, node, 0, b"data");
    let later = fat_time(2030, 1, 1, 12, 0, 0);
    fs.set_metadata(
        node,
        &SetMetadata::new()
            .with_times(FileTimes::new().with_modified(later))
            .with_attributes(Attributes::ARCHIVE),
    )
    .unwrap();
    fs.forget(node);
    assert_eq!(fs.open_nodes(), 1, "set_metadata wrote the pending size");
    let meta = fs.metadata("/stamped.txt").unwrap();
    assert_eq!(meta.len(), 4);
    assert_eq!(meta.times().modified(), Some(later));
    assert_eq!(meta.times().created(), Some(created));
    assert_eq!(meta.attributes(), Attributes::ARCHIVE);
    fs.set_metadata(root, &changes).unwrap();

    let dir = fs
        .create(root, name("dir"), NewNode::Dir, &SetMetadata::new())
        .unwrap();
    fs.set_metadata(dir, &SetMetadata::new().with_attributes(Attributes::HIDDEN))
        .unwrap();
    assert_eq!(fs.node_metadata(dir).unwrap().file_type(), FileType::Dir);
    assert_eq!(
        fs.node_metadata(dir).unwrap().attributes(),
        Attributes::HIDDEN
    );
    assert_eq!(
        fs.write_at(dir, 0, b"x").unwrap_err().kind(),
        ErrorKind::IsADirectory
    );
    assert_eq!(
        fs.set_len(root, 0).unwrap_err().kind(),
        ErrorKind::IsADirectory
    );
    fs.forget(dir);
    fs.sync().unwrap();
    fsck(&image(fs), "metadata");
}

/// `FsDriver::set_metadata` ignores fields the format cannot store, and
/// `copy_tree` and `import_from_host` pass a mode, so mode and owner must
/// not fail on FAT.
#[test]
fn mode_and_owner_are_ignored() {
    let case = CASES[1];
    let fs = open(case, populate(case));
    let caps = fs.capabilities();
    assert!(!caps.supports_permissions());
    assert!(!caps.supports_owners());
    let before = image(fs);
    let changes = SetMetadata::new()
        .with_mode(hadris_fs::Mode::new(0o4755))
        .with_uid(1000)
        .with_gid(100);

    let mut fs = open(case, before.clone());
    let root = fs.root();
    let node = fs.lookup(root, name("README.TXT")).unwrap();
    let dir = fs.lookup(root, name("Nested Dir")).unwrap();
    fs.set_metadata(node, &changes).unwrap();
    fs.set_metadata(dir, &changes).unwrap();
    fs.set_metadata(root, &changes).unwrap();
    let meta = fs.node_metadata(node).unwrap();
    assert_eq!(meta.permissions(), None);
    assert_eq!(meta.owner(), None);
    fs.forget(node);
    fs.forget(dir);
    fs.sync().unwrap();
    let after = image(fs);
    assert_eq!(after, before, "mode and owner changed the image");

    let mut fs = open(case, after);
    let root = fs.root();
    let created = fs
        .create(root, name("owned.txt"), NewNode::File, &changes)
        .unwrap();
    let meta = fs.node_metadata(created).unwrap();
    assert_eq!(meta.permissions(), None);
    assert_eq!(meta.owner(), None);
    fs.forget(created);
    fsck(&image(fs), "mode and owner");
}

/// The FAT specification sets the archive attribute when a file is
/// created, renamed or modified; directories keep theirs.
#[test]
fn archive_marks_created_renamed_and_modified_files() {
    let case = CASES[2];
    let mut fs = open(case, populate(case));
    let root = fs.root();
    let hidden = SetMetadata::new().with_attributes(Attributes::HIDDEN);
    let attributes = |fs: &mut Fs, path: &str| fs.metadata(path).unwrap().attributes();
    let archived = Attributes::HIDDEN | Attributes::ARCHIVE;

    let dir = fs.resolve("/Nested Dir").unwrap();
    fs.set_metadata(dir, &hidden).unwrap();
    fs.forget(dir);
    fs.rename(
        root,
        name("Nested Dir"),
        root,
        name("Moved Dir"),
        RenameFlags::empty(),
    )
    .unwrap();
    assert_eq!(attributes(&mut fs, "/Moved Dir"), Attributes::HIDDEN);

    let clear = |fs: &mut Fs, path: &str| {
        let node = fs.resolve(path).unwrap();
        fs.set_metadata(node, &hidden).unwrap();
        fs.forget(node);
    };
    clear(&mut fs, "/lower.txt");
    fs.rename(
        root,
        name("lower.txt"),
        root,
        name("lower.txt"),
        RenameFlags::empty(),
    )
    .unwrap();
    assert_eq!(
        attributes(&mut fs, "/lower.txt"),
        Attributes::HIDDEN,
        "no-op rename"
    );
    fs.rename(
        root,
        name("lower.txt"),
        root,
        name("Renamed.txt"),
        RenameFlags::empty(),
    )
    .unwrap();
    assert_eq!(attributes(&mut fs, "/Renamed.txt"), archived);

    clear(&mut fs, "/Renamed.txt");
    let node = fs.resolve("/Renamed.txt").unwrap();
    fs.set_len(node, 9).unwrap();
    assert_eq!(
        fs.node_metadata(node).unwrap().attributes(),
        Attributes::HIDDEN,
        "same size"
    );
    fs.set_len(node, 4).unwrap();
    assert_eq!(fs.node_metadata(node).unwrap().attributes(), archived);
    fs.set_metadata(node, &hidden).unwrap();
    write_all(&mut fs, node, 4, b"more");
    fs.sync_node(node).unwrap();
    assert_eq!(fs.node_metadata(node).unwrap().attributes(), archived);
    fs.forget(node);

    let created = create(&mut fs, root, "created.txt", NewNode::File);
    assert_eq!(
        fs.node_metadata(created).unwrap().attributes(),
        Attributes::ARCHIVE
    );
    fs.forget(created);
    fs.sync().unwrap();
    fsck(&image(fs), "archive");
}

#[test]
fn sync_persists_sizes_for_a_fresh_mount() {
    for case in CASES {
        let mut fs = open(case, common::blank(case));
        let root = fs.root();
        let node = create(&mut fs, root, "log.txt", NewNode::File);
        write_all(&mut fs, node, 0, b"first");
        write_all(&mut fs, node, 5, b" second");
        fs.forget(node);
        assert_eq!(fs.open_nodes(), 2);
        assert_eq!(fs.metadata("/log.txt").unwrap().len(), 12);
        let unsynced = image(fs);
        let mut stale = open(case, unsynced.clone());
        assert_eq!(
            stale.read_to_vec("/log.txt").unwrap(),
            b"first",
            "{}: the entry still has the size of the first write",
            case.name
        );

        let mut fs = open(case, unsynced);
        let node = fs.lookup(fs.root(), name("log.txt")).unwrap();
        write_all(&mut fs, node, 5, b" second");
        fs.sync_node(node).unwrap();
        fs.forget(node);
        assert_eq!(fs.open_nodes(), 1);
        let node = fs.lookup(fs.root(), name("log.txt")).unwrap();
        write_all(&mut fs, node, 12, b" third");
        fs.forget(node);
        fs.sync().unwrap();
        assert_eq!(fs.open_nodes(), 1);
        let synced = image(fs);
        let mut fresh = open(case, synced.clone());
        assert_eq!(
            fresh.read_to_vec("/log.txt").unwrap(),
            b"first second third"
        );
    }
}

#[test]
fn unpinned_ids_write_through() {
    let case = CASES[0];
    let mut fs = open(case, populate(case));
    let root = fs.root();
    let mut cursor = DirCursor::start();
    let mut buf = NameBuf::new();
    let entry = loop {
        let entry = fs
            .read_dir_entry(root, &mut cursor, &mut buf)
            .unwrap()
            .unwrap();
        if buf.as_bytes() == b"empty.dat" {
            break entry;
        }
    };
    write_all(&mut fs, entry.node(), 0, b"direct");
    assert_eq!(fs.open_nodes(), 1);
    let mut fresh = open(case, image(fs));
    assert_eq!(fresh.read_to_vec("/empty.dat").unwrap(), b"direct");
}

#[test]
fn two_handles_on_a_volume_see_one_size() {
    let case = CASES[2];
    let vol = Volume::new(open(case, common::blank(case)));
    let mut writer = vol
        .open("/shared.txt", OpenOptions::read_write().create())
        .unwrap();
    let mut reader = vol.open("/shared.txt", OpenOptions::read()).unwrap();
    assert_eq!(writer.node(), reader.node());
    writer.write_all(b"visible to both").unwrap();
    assert_eq!(reader.len().unwrap(), 15);
    let mut text = String::new();
    reader.read_to_string(&mut text).unwrap();
    assert_eq!(text, "visible to both");
    writer.set_len(7).unwrap();
    assert_eq!(reader.len().unwrap(), 7);
    assert_eq!(vol.metadata("/shared.txt").unwrap().len(), 7);
    reader.close().unwrap();
    writer.close().unwrap();

    vol.create_dir_all("/a/b/c").unwrap();
    vol.write_file("/a/b/c/file.txt", b"deep").unwrap();
    vol.rename_path("/a/b/c/file.txt", "/a/moved.txt").unwrap();
    assert_eq!(vol.read_to_vec("/a/moved.txt").unwrap(), b"deep");
    vol.remove_dir_all("/a/b").unwrap();
    assert!(!vol.exists("/a/b").unwrap());
    vol.sync().unwrap();
    let fs = vol.into_inner();
    assert_eq!(fs.open_nodes(), 1);
    let image = image(fs);
    assert_eq!(fresh_read(case, &image, "/shared.txt"), b"visible");
    fsck(&image, "volume");
}

#[test]
fn copy_tree_between_two_volumes() {
    let case = CASES[2];
    let mut src = open(case, common::build(case));
    let mut dst = open(CASES[1], common::blank(CASES[1]));
    copy_tree(&mut src, "/", &mut dst, "/").unwrap();
    let (src_root, dst_root) = (src.root(), dst.root());
    let copied = compare_trees(&mut src, src_root, &mut dst, dst_root);
    assert!(copied > 50, "{copied}");
    copy_tree(&mut src, "/Nested Dir", &mut dst, "/second").unwrap();
    assert_eq!(
        dst.read_to_vec("/second/inner/deep.bin").unwrap(),
        common::payload(70_000, 5)
    );
    dst.sync().unwrap();
    assert_eq!(dst.open_nodes(), 1);
    assert_eq!(src.open_nodes(), 1);
    let image = image(dst);
    let mut fresh = open(CASES[1], image.clone());
    let root = fresh.root();
    let src_root = src.root();
    compare_trees(&mut src, src_root, &mut fresh, root);
    fsck(&image, "copy_tree");
}

fn compare_trees(a: &mut Fs, a_dir: NodeId, b: &mut Fs, b_dir: NodeId) -> usize {
    let a_list = list(a, a_dir);
    let b_list = list(b, b_dir);
    for entry in &a_list {
        assert!(b_list.contains(entry), "{entry:?} in {b_list:?}");
    }
    let mut count = 0;
    for (text, file_type) in a_list {
        let a_node = a.lookup(a_dir, name(&text)).unwrap();
        let b_node = b.lookup(b_dir, name(&text)).unwrap();
        let (a_meta, b_meta) = (
            a.node_metadata(a_node).unwrap(),
            b.node_metadata(b_node).unwrap(),
        );
        assert_eq!(a_meta.attributes(), b_meta.attributes(), "{text}");
        assert_eq!(
            a_meta.times().modified(),
            b_meta.times().modified(),
            "{text}"
        );
        if file_type == FileType::Dir {
            count += compare_trees(a, a_node, b, b_node);
        } else {
            assert_eq!(read_all(a, a_node), read_all(b, b_node), "{text}");
        }
        a.forget(a_node);
        b.forget(b_node);
        count += 1;
    }
    count
}

/// A device that refuses writes, or fails them with a device error after
/// `budget` writes: every later write, or with `once` only the next one.
struct Faulty {
    inner: Device,
    budget: Option<usize>,
    refuse: bool,
    once: bool,
}

impl hadris_io::ErrorType for Faulty {
    type Error = OutOfRange;
}

impl BlockDevice for Faulty {
    fn block_size(&self) -> BlockSize {
        self.inner.block_size()
    }

    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }

    fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), OutOfRange> {
        self.inner.read_blocks(first, buf)
    }

    fn write_blocks(
        &mut self,
        first: BlockIndex,
        buf: &[u8],
    ) -> Result<(), WriteError<OutOfRange>> {
        if self.refuse {
            return Err(WriteError::ReadOnly);
        }
        match &mut self.budget {
            Some(0) => {
                if self.once {
                    self.budget = None;
                }
                Err(WriteError::Device(OutOfRange))
            }
            Some(left) => {
                *left -= 1;
                self.inner.write_blocks(first, buf)
            }
            None => self.inner.write_blocks(first, buf),
        }
    }
}

/// A device that counts flushes.
struct Flushes {
    inner: Device,
    flushes: usize,
}

impl hadris_io::ErrorType for Flushes {
    type Error = OutOfRange;
}

impl BlockDevice for Flushes {
    fn block_size(&self) -> BlockSize {
        self.inner.block_size()
    }

    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }

    fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), OutOfRange> {
        self.inner.read_blocks(first, buf)
    }

    fn write_blocks(
        &mut self,
        first: BlockIndex,
        buf: &[u8],
    ) -> Result<(), WriteError<OutOfRange>> {
        self.inner.write_blocks(first, buf)
    }

    fn flush(&mut self) -> Result<(), WriteError<OutOfRange>> {
        self.flushes += 1;
        Ok(())
    }
}

#[test]
fn publish_node_writes_the_entry_without_a_flush() {
    let case = CASES[1];
    let dev = Flushes {
        inner: common::device(case, common::blank(case)),
        flushes: 0,
    };
    let mut fs = FatFs::open(dev).unwrap();
    let root = fs.root();
    let node = fs
        .create(root, name("log.txt"), NewNode::File, &SetMetadata::new())
        .unwrap();
    fs.write_at(node, 0, b"published").unwrap();
    fs.publish_node(node).unwrap();
    fs.forget(node);
    assert_eq!(fs.open_nodes(), 1, "a published node is clean");
    let dev = fs.into_inner();
    assert_eq!(dev.flushes, 0);
    let image = dev.inner.into_inner();
    let mut fresh = open(case, image.clone());
    assert_eq!(fresh.read_to_vec("/log.txt").unwrap(), b"published");

    let mut fs = FatFs::open(Flushes {
        inner: common::device(case, image),
        flushes: 0,
    })
    .unwrap();
    let node = fs.lookup(fs.root(), name("log.txt")).unwrap();
    fs.sync_node(node).unwrap();
    fs.forget(node);
    assert_eq!(fs.into_inner().flushes, 1);
}

#[test]
fn refused_writes_make_the_volume_read_only() {
    let case = CASES[1];
    let before = populate(case);
    let dev = Faulty {
        inner: common::device(case, before.clone()),
        budget: None,
        refuse: true,
        once: false,
    };
    let mut fs = FatFs::open(dev).unwrap();
    assert!(FsDriver::capabilities(&fs).is_writable());
    let root = fs.root();
    let stats = fs.stats().unwrap();
    let file = fs.lookup(root, name("README.TXT")).unwrap();
    assert_eq!(
        fs.write_at(file, 0, b"x").unwrap_err().kind(),
        ErrorKind::ReadOnly
    );
    assert!(fs.is_read_only());
    assert!(!FsDriver::capabilities(&fs).is_writable());
    let meta = SetMetadata::new();
    let kinds = [
        fs.create(root, name("new"), NewNode::File, &meta)
            .map(|_| ()),
        fs.remove(root, name("lower.txt"), RemoveKind::Any),
        fs.rename(
            root,
            name("lower.txt"),
            root,
            name("x"),
            RenameFlags::empty(),
        ),
        fs.set_len(file, 0),
        fs.set_metadata(file, &meta),
    ]
    .map(|result| result.unwrap_err().kind());
    assert_eq!(kinds, [ErrorKind::ReadOnly; 5]);
    assert_eq!(fs.stats().unwrap(), stats);
    assert_eq!(read_all(&mut fs, file), b"hello fat");
    fs.sync().unwrap();
    fs.forget(file);
    assert_eq!(fs.open_nodes(), 1);
    assert_eq!(fs.into_inner().inner.into_inner(), before);

    for op in 0..3 {
        let dev = Faulty {
            inner: common::device(case, before.clone()),
            budget: None,
            refuse: true,
            once: false,
        };
        let mut fs = FatFs::open(dev).unwrap();
        let root = fs.root();
        let err = match op {
            0 => fs
                .create(root, name("a long new name"), NewNode::Dir, &meta)
                .map(|_| ()),
            1 => fs.remove(root, name("A long file name.txt"), RemoveKind::Any),
            _ => fs.rename(
                root,
                name("lower.txt"),
                root,
                name("Other Name"),
                RenameFlags::empty(),
            ),
        };
        assert_eq!(err.unwrap_err().kind(), ErrorKind::ReadOnly);
        assert_eq!(fs.stats().unwrap(), stats);
        assert_eq!(fs.open_nodes(), 1);
        assert_eq!(fs.into_inner().inner.into_inner(), before);
    }
}

/// Every operation interrupted after each of its writes leaves a volume
/// that still mounts and reads.
#[test]
fn interrupted_operations_leave_readable_volumes() {
    let case = CASES[0];
    let before = populate(case);
    let meta = SetMetadata::new();
    for op in 0..6 {
        for budget in 0..40 {
            let dev = Faulty {
                inner: common::device(case, before.clone()),
                budget: Some(budget),
                refuse: false,
                once: false,
            };
            let mut fs =
                FatFs::open_with(dev, MountOptions::new().with_table(HeapTable::<()>::new()))
                    .unwrap();
            let root = fs.root();
            let result = match op {
                0 => fs
                    .create(root, name("a new directory"), NewNode::Dir, &meta)
                    .map(|_| ()),
                1 => fs.remove(root, name("A long file name.txt"), RemoveKind::Any),
                2 => fs.rename(
                    root,
                    name("Nested Dir"),
                    root,
                    name("Renamed Dir"),
                    RenameFlags::empty(),
                ),
                3 => fs.rename(
                    root,
                    name("lower.txt"),
                    root,
                    name("README.TXT"),
                    RenameFlags::empty(),
                ),
                4 => fs
                    .resolve("/grown.bin")
                    .and_then(|node| fs.write_at(node, 8_000, &[3u8; 20_000]).map(|_| ())),
                _ => fs
                    .resolve("/A long file name.txt")
                    .and_then(|node| fs.set_len(node, 100)),
            };
            let finished = result.is_ok();
            if let Err(err) = &result {
                assert_eq!(err.kind(), ErrorKind::Io, "op {op} budget {budget}");
                assert!(err.device_error().is_some());
            }
            let image = fs.into_inner().inner.into_inner();
            let mut fresh = open(case, image);
            walk(&mut fresh);
            if finished {
                break;
            }
        }
    }
}

/// A replace that fails at any one write either changes nothing visible
/// or, when only its cleanup failed, has already renamed; the target is
/// never lost on its own.
#[test]
fn a_failed_replace_keeps_the_target() {
    let case = CASES[0];
    let before = populate(case);
    let state = |image: &[u8]| {
        let names = fresh_names(case, image, "/");
        let contents: Vec<Vec<u8>> = names
            .iter()
            .filter(|n| *n != "Nested Dir")
            .map(|n| fresh_read(case, image, &format!("/{n}")))
            .collect();
        (names, contents)
    };
    let untouched = state(&before);
    for to in ["A LONG FILE NAME.TXT", "ALONGF~1.TXT"] {
        let rename = |fs: &mut FatFs<Faulty, HeapTable>| {
            let root = fs.root();
            fs.rename(
                root,
                name("lower.txt"),
                root,
                name(to),
                RenameFlags::empty(),
            )
        };
        let mut fs = FatFs::open_with(
            Faulty {
                inner: common::device(case, before.clone()),
                budget: None,
                refuse: false,
                once: false,
            },
            MountOptions::new().with_table(HeapTable::new()),
        )
        .unwrap();
        rename(&mut fs).unwrap();
        let renamed = state(&fs.into_inner().inner.into_inner());
        assert!(renamed.0.contains(&to.to_owned()));
        let (mut failures, mut undone) = (0, 0);
        for budget in 0..40 {
            let mut fs = FatFs::open_with(
                Faulty {
                    inner: common::device(case, before.clone()),
                    budget: Some(budget),
                    refuse: false,
                    once: true,
                },
                MountOptions::new().with_table(HeapTable::new()),
            )
            .unwrap();
            let result = rename(&mut fs);
            let image = fs.into_inner().inner.into_inner();
            let after = state(&image);
            match result {
                Ok(()) => {
                    assert_eq!(after, renamed, "{to} budget {budget}");
                    fsck(&image, "replace");
                    break;
                }
                Err(err) => {
                    failures += 1;
                    undone += usize::from(after == untouched);
                    assert_eq!(err.kind(), ErrorKind::Io);
                    assert!(
                        after == untouched || after == renamed,
                        "{to} budget {budget}: {:?}",
                        after.0
                    );
                }
            }
        }
        assert!(failures >= 3 && undone >= 2, "{to}: {failures} {undone}");
    }
}

fn walk(fs: &mut Fs) {
    let mut stack = vec![fs.root()];
    while let Some(dir) = stack.pop() {
        for (text, file_type) in list(fs, dir) {
            let node = fs.lookup(dir, name(&text)).unwrap();
            if file_type == FileType::Dir {
                stack.push(node);
            } else {
                read_all(fs, node);
            }
        }
    }
}

/// A tiny deterministic generator for the differential test.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Model {
    File(Vec<u8>),
    Dir,
}

const POOL: [&str; 8] = [
    "alpha.txt",
    "Beta File.bin",
    "GAMMA",
    "delta with a long name.dat",
    "e",
    "Zeta.TXT",
    "\u{3B7}ta.txt",
    "theta.theta",
];

/// Random operations on two directories, checked against a map after each
/// step and against a fresh mount and `fsck` at the end.
#[test]
fn random_operations_match_a_model() {
    for (seed, case) in [
        (1u64, CASES[0]),
        (7, CASES[1]),
        (99, CASES[2]),
        (5, CASES[4]),
    ] {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut fs = open(case, common::blank(case));
        let root = fs.root();
        let sub = create(&mut fs, root, "sub", NewNode::Dir);
        let dirs = [root, sub];
        let mut model: BTreeMap<(usize, usize), Model> = BTreeMap::new();
        for step in 0..400 {
            let d = rng.below(2) as usize;
            let i = rng.below(POOL.len() as u64) as usize;
            let key = (d, i);
            let text = POOL[i];
            let context = format!("{} seed {seed} step {step}", case.name);
            match rng.below(6) {
                0 => {
                    let kind = if rng.below(4) == 0 {
                        NewNode::Dir
                    } else {
                        NewNode::File
                    };
                    let result = fs.create(dirs[d], name(text), kind, &SetMetadata::new());
                    match model.entry(key) {
                        Entry::Occupied(_) => assert_eq!(
                            result.unwrap_err().kind(),
                            ErrorKind::AlreadyExists,
                            "{context}"
                        ),
                        Entry::Vacant(slot) => {
                            fs.forget(result.unwrap());
                            slot.insert(match kind {
                                NewNode::Dir => Model::Dir,
                                _ => Model::File(Vec::new()),
                            });
                        }
                    }
                }
                1 | 2 => {
                    let Some(Model::File(data)) = model.get_mut(&key) else {
                        continue;
                    };
                    let node = fs.lookup(dirs[d], name(text)).unwrap();
                    let offset = rng.below(data.len() as u64 + 3000);
                    let len = rng.below(9000) as usize;
                    let bytes: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
                    write_all(&mut fs, node, offset, &bytes);
                    let end = offset as usize + len;
                    if data.len() < end && len > 0 {
                        data.resize(end, 0);
                    }
                    data[offset as usize..end].copy_from_slice(&bytes);
                    fs.forget(node);
                }
                3 => {
                    let Some(Model::File(data)) = model.get_mut(&key) else {
                        continue;
                    };
                    let node = fs.lookup(dirs[d], name(text)).unwrap();
                    let len = rng.below(data.len() as u64 * 2 + 100) as usize;
                    fs.set_len(node, len as u64).unwrap();
                    data.resize(len, 0);
                    fs.forget(node);
                }
                4 => {
                    let result = fs.remove(dirs[d], name(text), RemoveKind::Any);
                    match model.get(&key) {
                        None => {
                            assert_eq!(result.unwrap_err().kind(), ErrorKind::NotFound, "{context}")
                        }
                        Some(Model::Dir) => {
                            result.unwrap();
                            model.remove(&key);
                        }
                        Some(Model::File(_)) => {
                            result.unwrap();
                            model.remove(&key);
                        }
                    }
                }
                _ => {
                    let to_d = rng.below(2) as usize;
                    let j = rng.below(POOL.len() as u64) as usize;
                    let result = fs.rename(
                        dirs[d],
                        name(text),
                        dirs[to_d],
                        name(POOL[j]),
                        RenameFlags::empty(),
                    );
                    let src = model.get(&key).cloned();
                    let dst = model.get(&(to_d, j)).cloned();
                    match (src, dst) {
                        (None, _) => {
                            assert_eq!(result.unwrap_err().kind(), ErrorKind::NotFound, "{context}")
                        }
                        (Some(_), _) if key == (to_d, j) => result.unwrap(),
                        (Some(Model::Dir), Some(Model::File(_))) => {
                            assert_eq!(
                                result.unwrap_err().kind(),
                                ErrorKind::NotADirectory,
                                "{context}"
                            )
                        }
                        (Some(Model::File(_)), Some(Model::Dir)) => {
                            assert_eq!(
                                result.unwrap_err().kind(),
                                ErrorKind::IsADirectory,
                                "{context}"
                            )
                        }
                        (Some(value), _) => {
                            result.unwrap();
                            model.remove(&key);
                            model.insert((to_d, j), value);
                        }
                    }
                }
            }
            if step % 50 == 49 {
                fs.sync().unwrap();
            }
            check_model(&mut fs, &dirs, &model, &context);
        }
        fs.forget(sub);
        fs.sync().unwrap();
        assert_eq!(fs.open_nodes(), 1);
        let image = image(fs);
        let mut fresh = common::mount(case, &image);
        for ((d, i), value) in &model {
            if let Model::File(data) = value {
                let dir = if *d == 0 { "" } else { "/sub" };
                assert_eq!(
                    &common::read(&mut fresh, &format!("{dir}/{}", POOL[*i])),
                    data,
                    "{} {}",
                    case.name,
                    POOL[*i]
                );
            }
        }
        fsck(&image, case.name);
    }
}

fn check_model(
    fs: &mut Fs,
    dirs: &[NodeId; 2],
    model: &BTreeMap<(usize, usize), Model>,
    context: &str,
) {
    for (d, dir) in dirs.iter().enumerate() {
        let mut listed: Vec<String> = list(fs, *dir)
            .into_iter()
            .map(|(n, _)| n)
            .filter(|n| d != 0 || n != "sub")
            .collect();
        listed.sort();
        let mut expected: Vec<String> = model
            .keys()
            .filter(|(md, _)| *md == d)
            .map(|(_, i)| POOL[*i].to_owned())
            .collect();
        expected.sort();
        assert_eq!(listed, expected, "{context}");
    }
    for ((d, i), value) in model {
        let node = fs.lookup(dirs[*d], name(POOL[*i])).unwrap();
        match value {
            Model::File(data) => assert_eq!(&read_all(fs, node), data, "{context} {}", POOL[*i]),
            Model::Dir => assert_eq!(fs.node_metadata(node).unwrap().file_type(), FileType::Dir),
        }
        fs.forget(node);
    }
}

#[test]
fn host_fsck_runs_when_installed() {
    let ran = fsck(&populate(CASES[2]), "populate");
    eprintln!("{ran} host fsck tools checked the image");
}

#[derive(Debug, Clone, Copy)]
struct FixedClock(DateTime);

impl Clock for FixedClock {
    fn now(&self) -> DateTime {
        self.0
    }
}

#[test]
fn clock_stamps_new_and_modified_entries() {
    let case = CASES[0];
    let midnight = |time: DateTime| {
        let (date, _) = time.to_civil();
        DateTime::from_civil(date, CivilTime::new(0, 0, 0).unwrap(), None).unwrap()
    };

    let mut fs = open(case, common::blank(case));
    let root = fs.root();
    let file = create(&mut fs, root, "default.txt", NewNode::File);
    let times = fs.node_metadata(file).unwrap().times();
    assert_eq!(times.created(), Some(NoClock::TIME));
    assert_eq!(times.modified(), Some(NoClock::TIME));

    let created = fat_time(2031, 7, 4, 12, 30, 44);
    let written = fat_time(2032, 1, 2, 3, 4, 6);
    let options = MountOptions::new()
        .with_table(HeapTable::new())
        .with_clock(FixedClock(created));
    let mut fs = FatFs::open_with(common::device(case, image(fs)), options).unwrap();
    assert_eq!(fs.clock().now(), created);
    let root = fs.root();
    let file = fs
        .create(
            root,
            name("clocked.txt"),
            NewNode::File,
            &SetMetadata::new(),
        )
        .unwrap();
    let times = fs.node_metadata(file).unwrap().times();
    assert_eq!(times.created(), Some(created));
    assert_eq!(times.modified(), Some(created));
    assert_eq!(times.accessed(), Some(midnight(created)));
    fs.forget(file);
    fs.sync().unwrap();

    let options = MountOptions::new().with_clock(FixedClock(written));
    let mut fs = FatFs::open_with(fs.into_inner(), options).unwrap();
    let root = fs.root();
    let file = fs.lookup(root, name("clocked.txt")).unwrap();
    fs.write_at(file, 0, b"tick").unwrap();
    fs.sync().unwrap();
    let times = fs.node_metadata(file).unwrap().times();
    assert_eq!(times.created(), Some(created));
    assert_eq!(times.modified(), Some(written));
    assert_eq!(times.accessed(), Some(midnight(written)));
}

#[test]
fn code_page_reads_and_generates_short_names() {
    let case = CASES[0];
    let built = common::build(case);
    let cp437 = MountOptions::new()
        .with_table(HeapTable::new())
        .with_code_page(Cp437);
    let mut fs = FatFs::open_with(common::device(case, built.clone()), cp437).unwrap();
    let root = fs.root();
    let names: Vec<String> = list(&mut fs, root).into_iter().map(|(n, _)| n).collect();
    assert!(names.iter().any(|n| n == "\u{3C3}ABC.TXT"), "{names:?}");
    assert_eq!(
        fs.lookup(root, name(KANJI_NAME)).unwrap_err().kind(),
        ErrorKind::NotFound
    );
    let kanji = fs.lookup(root, name("\u{3C3}abc.txt")).unwrap();
    assert_eq!(read_all(&mut fs, kanji), b"kanji");
    fs.forget(kanji);

    let cafe = create(&mut fs, root, "caf\u{E9}.txt", NewNode::File);
    write_all(&mut fs, cafe, 0, b"cp437");
    fs.forget(cafe);
    fs.sync().unwrap();
    assert!(
        list(&mut fs, root)
            .iter()
            .any(|(n, _)| n == "caf\u{E9}.txt")
    );
    let cafe = fs.lookup(root, name("CAF\u{C9}.TXT")).unwrap();
    assert_eq!(read_all(&mut fs, cafe), b"cp437");
    fs.forget(cafe);
    let short = fs.lookup(root, name("caf\u{E9}~1.txt")).unwrap();
    assert_eq!(short, cafe);
    fs.forget(short);
    let cp437_image = image(fs);
    assert!(
        cp437_image
            .chunks_exact(32)
            .any(|entry| &entry[..11] == b"CAF\x90~1  TXT")
    );
    let mut fresh = FatFs::open_with(
        common::device(case, cp437_image.clone()),
        MountOptions::new().with_code_page(Cp437),
    )
    .unwrap();
    assert_eq!(fresh.read_to_vec("/caf\u{E9}.txt").unwrap(), b"cp437");

    let mut fs = open(case, built);
    let root = fs.root();
    let cafe = create(&mut fs, root, "caf\u{E9}.txt", NewNode::File);
    fs.forget(cafe);
    fs.sync().unwrap();
    assert!(
        image(fs)
            .chunks_exact(32)
            .any(|entry| &entry[..11] == b"CAF_~1  TXT")
    );
    fsck(&cp437_image, "cp437 short names");
}

#[test]
fn listing_never_hands_out_the_id_of_a_moved_pinned_node() {
    for case in CASES {
        let mut fs = open(case, common::blank(case));
        let root = fs.root();
        let a = create(&mut fs, root, "A.TXT", NewNode::File);
        write_all(&mut fs, a, 0, b"aaaaa");
        fs.sync_node(a).unwrap();
        fs.rename(
            root,
            name("A.TXT"),
            root,
            name("C.TXT"),
            RenameFlags::empty(),
        )
        .unwrap();
        let b = create(&mut fs, root, "B.TXT", NewNode::File);
        write_all(&mut fs, b, 0, b"bbb");
        fs.forget(b);

        let listed: Vec<_> = {
            let mut cursor = DirCursor::start();
            let mut buf = NameBuf::new();
            let mut out = Vec::new();
            while let Some(entry) = fs.read_dir_entry(root, &mut cursor, &mut buf).unwrap() {
                out.push((
                    buf.as_name().unwrap().to_str().unwrap().to_owned(),
                    entry.node(),
                ));
            }
            out
        };
        let id = |text: &str| listed.iter().find(|(n, _)| n == text).unwrap().1;
        assert_eq!(id("C.TXT"), a, "{}", case.name);
        let b = id("B.TXT");
        assert_ne!(b, a, "{}", case.name);
        assert_eq!(fs.node_metadata(b).unwrap().len(), 3, "{}", case.name);
        assert_eq!(read_all(&mut fs, b), b"bbb", "{}", case.name);
        write_all(&mut fs, b, 3, b"B");
        assert_eq!(read_all(&mut fs, a), b"aaaaa", "{}", case.name);
        assert_eq!(read_all(&mut fs, b), b"bbbB", "{}", case.name);

        fs.forget(a);
        assert_eq!(read_all(&mut fs, b), b"bbbB", "{}", case.name);
        fs.sync().unwrap();
        let image = image(fs);
        assert_eq!(fresh_read(case, &image, "/C.TXT"), b"aaaaa");
        assert_eq!(fresh_read(case, &image, "/B.TXT"), b"bbbB");
    }
}

#[test]
fn a_zero_fsinfo_free_count_does_not_stop_allocation() {
    let case = CASES[2];
    assert_eq!(case.kind, hadris_fat::FatKind::Fat32);
    let mut blank = common::blank(case);
    let actual = free(&mut common::mount(case, &blank));
    let sector = u16::from_le_bytes([blank[11], blank[12]]) as usize;
    let fs_info = u16::from_le_bytes([blank[48], blank[49]]) as usize * sector;
    blank[fs_info + 488..fs_info + 492].copy_from_slice(&0u32.to_le_bytes());
    let mut fs = open(case, blank);
    let root = fs.root();
    let file = create(&mut fs, root, "X.BIN", NewNode::File);
    write_all(&mut fs, file, 0, b"hi");
    assert_eq!(free(&mut fs), actual - 1);
    fs.forget(file);
    fs.sync().unwrap();
    let image = image(fs);
    assert_eq!(fresh_read(case, &image, "/X.BIN"), b"hi");
    assert_eq!(free(&mut common::mount(case, &image)), actual - 1);
}

#[test]
fn renaming_a_directory_with_a_reserved_first_cluster_fails_cleanly() {
    for case in &CASES[..3] {
        for bad in [0u32, 1] {
            let mut fs = open(*case, common::blank(*case));
            let root = fs.root();
            for text in ["D", "S", "T"] {
                let node = create(&mut fs, root, text, NewNode::Dir);
                fs.forget(node);
            }
            let s = fs.lookup(root, name("S")).unwrap();
            let t = create(&mut fs, s, "T", NewNode::Dir);
            fs.forget(t);
            fs.forget(s);
            fs.sync().unwrap();
            let mut image = image(fs);
            let at = image
                .chunks_exact(32)
                .position(|entry| &entry[..11] == b"D          ")
                .unwrap()
                * 32;
            image[at + 20..at + 22].fill(0);
            image[at + 26..at + 28].copy_from_slice(&(bad as u16).to_le_bytes());

            let mut fs = open(*case, image.clone());
            let root = fs.root();
            let s = fs.lookup(root, name("S")).unwrap();
            for (to_dir, to) in [(s, "D"), (s, "T"), (root, "E"), (root, "T")] {
                assert_eq!(
                    fs.rename(root, name("D"), to_dir, name(to), RenameFlags::empty())
                        .unwrap_err()
                        .kind(),
                    ErrorKind::Corrupt,
                    "{} cluster {bad} to {to}",
                    case.name
                );
            }
            fs.forget(s);
            fs.sync().unwrap();
            assert!(fs.into_inner().into_inner() == image, "{}", case.name);
        }
    }
}
