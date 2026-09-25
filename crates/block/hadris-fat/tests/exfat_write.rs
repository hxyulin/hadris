//! `ExFatFs` writes: names, times and attributes, labels, directory growth,
//! rename and remove, allocation, the volume flags, refused and failing
//! writes, and random operations checked against a model, the checker and
//! the native tools.

#[path = "common/exfat.rs"]
mod common;
use common::FsPaths;
use hadris_fs::MountOptions;

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use common::{Device, Fs, Geometry, Tool, clean, fsck, le32};
use hadris_fat::exfat::VolumeLabel;
use hadris_fat::exfat::sync::ExFatFs;
use hadris_fs::sync::FileSystem;
use hadris_fs::{
    Attributes, CivilDate, CivilTime, Clock, DateTime, DirCursor, ErrorKind, FileType, Name,
    NodeId, OpenMode, Owner, Permissions, RenameMode, SetAttr,
};
use hadris_io::Error;
use hadris_storage::sync::BlockDevice;
use hadris_storage::{BlockIndex, BlockSize};

fn name(text: &str) -> &Name {
    Name::new(text)
}

fn create(fs: &mut Fs, dir: NodeId, text: &str, kind: FileType) -> NodeId {
    match kind {
        FileType::Dir => fs.mkdir(dir, name(text), &SetAttr::new()),
        _ => fs.create(dir, name(text), &SetAttr::new()),
    }
    .unwrap()
}

fn read_all<D: BlockDevice>(fs: &mut ExFatFs<D>, node: NodeId) -> Vec<u8> {
    let len = fs.stat(node).unwrap().len() as usize;
    let mut out = vec![0u8; len];
    let mut done = 0;
    while done < len {
        let n = fs.read(node, done as u64, &mut out[done..]).unwrap();
        assert!(n > 0);
        done += n;
    }
    out
}

fn list(fs: &mut Fs, dir: NodeId) -> Vec<(String, FileType)> {
    let mut cursor = DirCursor::START;
    let mut out = Vec::new();
    while let Some(entry) = fs.readdir(dir, cursor).unwrap() {
        cursor = entry.next_cursor();
        out.push((entry.name().to_str().unwrap().to_owned(), entry.file_type()));
    }
    out
}

fn free(fs: &mut Fs) -> u64 {
    fs.statfs().unwrap().free_blocks()
}

fn civil(year: i32, hour: u8, offset: Option<i16>) -> DateTime {
    DateTime::from_civil(
        CivilDate::new(year, 6, 15).unwrap(),
        CivilTime::new(hour, 30, 45).unwrap(),
        offset,
    )
    .unwrap()
}

#[test]
fn times_and_attributes_round_trip() {
    let mut fs = common::small(4 << 20, 4096);
    let root = fs.root();
    let created = civil(2001, 3, Some(-150))
        .with_nanoseconds(560_000_000)
        .unwrap();
    let modified = civil(2030, 22, Some(330));
    let accessed = civil(1990, 1, None);
    let meta = SetAttr::new()
        .with_created(created)
        .with_modified(modified)
        .with_accessed(accessed)
        .with_attributes(Attributes::HIDDEN | Attributes::SYSTEM);
    let node = fs.create(root, name("stamped.txt"), &meta).unwrap();
    let dir = fs.mkdir(root, name("stamped dir"), &meta).unwrap();
    fs.forget(dir, 1);
    fs.forget(node, 1);
    fs.sync().unwrap();
    let mut fs = common::mount(&common::image(fs));
    let node = fs.resolve_path("/STAMPED.TXT").unwrap();
    let got = fs.stat(node).unwrap();
    assert_eq!(got.created(), Some(created));
    assert_eq!(got.modified(), Some(modified));
    assert_eq!(
        got.accessed().map(|t| t.to_civil().0),
        Some(accessed.to_civil().0)
    );
    assert_eq!(got.attributes(), Attributes::HIDDEN | Attributes::SYSTEM);
    let dir = fs.resolve_path("/stamped dir").unwrap();
    assert_eq!(
        fs.stat(dir).unwrap().attributes(),
        Attributes::HIDDEN | Attributes::SYSTEM
    );
    assert_eq!(fs.stat(dir).unwrap().file_type(), FileType::Dir);

    fs.setattr(node, &SetAttr::new().with_attributes(Attributes::READ_ONLY))
        .unwrap();
    assert_eq!(fs.stat(node).unwrap().attributes(), Attributes::READ_ONLY);
    fs.write(node, 0, b"x").unwrap();
    fs.fsync(node).unwrap();
    assert_eq!(
        fs.stat(node).unwrap().attributes(),
        Attributes::READ_ONLY | Attributes::ARCHIVE
    );
    assert_eq!(
        fs.stat(node).unwrap().permissions(),
        Permissions::new(0o444)
    );
    fs.setattr(
        node,
        &SetAttr::new().with_permissions(Permissions::new(0o644)),
    )
    .unwrap();
    assert_eq!(fs.stat(node).unwrap().attributes(), Attributes::ARCHIVE);
    fs.setattr(
        dir,
        &SetAttr::new().with_permissions(Permissions::new(0o555)),
    )
    .unwrap();
    assert!(
        fs.stat(dir)
            .unwrap()
            .attributes()
            .contains(Attributes::READ_ONLY)
    );
    for changes in [
        SetAttr::new().with_permissions(Permissions::new(0o600)),
        SetAttr::new().with_owner(Owner::new(1000, 100)),
    ] {
        assert_eq!(
            fs.setattr(node, &changes).unwrap_err().kind(),
            ErrorKind::Unsupported
        );
    }
    assert_eq!(
        fs.setattr(root, &SetAttr::new().with_attributes(Attributes::HIDDEN))
            .unwrap_err()
            .kind(),
        ErrorKind::Unsupported
    );
    fs.forget(node, 1);
    fs.forget(dir, 1);
    fs.sync().unwrap();
    clean(&mut fs, "times");
    fsck(&common::image(fs), "times");
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
    let image = common::image(common::small(4 << 20, 4096));
    let now = civil(2024, 12, Some(60));
    let mut fs = ExFatFs::mount(
        common::device(image, 512),
        MountOptions::new().with_clock(Box::leak(Box::new(FixedClock(now)))),
    )
    .unwrap();
    let root = fs.root();
    let node = fs.create(root, name("now.txt"), &SetAttr::new()).unwrap();
    let times = fs.stat(node).unwrap();
    assert_eq!(times.created(), Some(now));
    assert_eq!(times.modified(), Some(now));
    fs.forget(node, 1);
}

#[test]
fn long_names_up_to_255_units() {
    let mut fs = common::small(4 << 20, 4096);
    let root = fs.root();
    let longest = "\u{1F600}".repeat(127) + "e";
    assert_eq!(longest.encode_utf16().count(), 255);
    let node = create(&mut fs, root, &longest, FileType::File);
    fs.forget(node, 1);
    let too_long = "\u{1F600}".repeat(128);
    assert_eq!(
        fs.create(root, name(&too_long), &SetAttr::new())
            .unwrap_err()
            .kind(),
        ErrorKind::NameTooLong
    );
    for bad in [
        "a:b",
        "a*",
        "q?",
        "tail.",
        "tail ",
        "a\u{1}b",
        "back\\slash",
        "pipe|",
        "<x>",
        "\"q\"",
    ] {
        assert_eq!(
            fs.create(root, name(bad), &SetAttr::new())
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidInput,
            "{bad:?}"
        );
    }
    let node = create(&mut fs, root, "Straße Ωmega", FileType::File);
    fs.forget(node, 1);
    for twin in ["STRAßE ΩMEGA", "straße ωmega"] {
        assert_eq!(
            fs.create(root, name(twin), &SetAttr::new())
                .unwrap_err()
                .kind(),
            ErrorKind::AlreadyExists,
            "{twin}"
        );
    }
    let names: Vec<String> = list(&mut fs, root).into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, [longest.clone(), "Straße Ωmega".to_owned()]);
    fs.sync().unwrap();
    let image = common::image(fs);
    let mut fs = common::mount(&image);
    assert!(
        fs.resolve_path(&format!("/{}", longest.to_uppercase()))
            .is_ok()
    );
    fsck(&image, "long names");
}

#[test]
fn labels_are_set_and_removed() {
    let mut fs = common::small(4 << 20, 4096);
    assert!(fs.label_text().unwrap().is_none());
    fs.set_label(Some(VolumeLabel::new("Première").unwrap()))
        .unwrap();
    assert_eq!(fs.label_text().unwrap().unwrap(), "Première");
    fs.set_label(Some(VolumeLabel::new("Second").unwrap()))
        .unwrap();
    fs.sync().unwrap();
    let image = common::image(fs);
    let geo = Geometry::of(&image);
    assert_eq!(geo.root_entries(&image, 0x83).len(), 1);
    fsck(&image, "label");
    let mut fs = common::mount(&image);
    assert_eq!(fs.label_text().unwrap().unwrap(), "Second");
    fs.set_label(None).unwrap();
    assert!(fs.label_text().unwrap().is_none());
    fs.sync().unwrap();
    clean(&mut fs, "no label");
    fsck(&common::image(fs), "no label");
}

#[test]
fn directories_grow_across_clusters() {
    let mut fs = common::small(8 << 20, 512);
    let root = fs.root();
    let dir = create(&mut fs, root, "grown", FileType::Dir);
    for i in 0..200 {
        let node = create(
            &mut fs,
            dir,
            &format!("entry number {i:03}"),
            FileType::File,
        );
        fs.forget(node, 1);
        if i % 50 == 0 {
            let spacer = common::write(&mut fs, root, &format!("spacer {i}"), &[7; 700]);
            fs.forget(spacer, 1);
        }
    }
    for i in 0..60 {
        let node = create(
            &mut fs,
            root,
            &format!("root entry number {i:03}"),
            FileType::File,
        );
        fs.forget(node, 1);
    }
    let clusters = common::chain(&mut fs, dir);
    assert_eq!(clusters.len(), 200 * 4 * 32 / 512);
    assert!(clusters.windows(2).any(|pair| pair[1] != pair[0] + 1));
    assert!(common::chain(&mut fs, root).len() > 1);
    assert_eq!(list(&mut fs, dir).len(), 200);
    fs.forget(dir, 1);
    fs.sync().unwrap();
    let image = common::image(fs);
    let geo = Geometry::of(&image);
    let set = geo.set(&image, geo.root, "grown");
    let size = u64::from_le_bytes(image[set[1] + 24..set[1] + 32].try_into().unwrap());
    assert_eq!(size, clusters.len() as u64 * 512);
    let mut fs = common::mount(&image);
    clean(&mut fs, "grown");
    assert_eq!(common::names(&mut fs, "/grown").len(), 200);
    fsck(&image, "grown");
}

#[test]
fn contiguous_files_grow_into_chains() {
    let mut fs = common::small(8 << 20, 512);
    let root = fs.root();
    for (text, len) in [("grow.bin", 3000), ("shrink.bin", 5000), ("gone.bin", 2000)] {
        let node = common::write(&mut fs, root, text, &common::payload(len, 1));
        fs.forget(node, 1);
    }
    fs.sync().unwrap();
    let mut image = common::image(fs);
    let geo = Geometry::of(&image);
    for text in ["grow.bin", "shrink.bin", "gone.bin"] {
        let set = geo.set(&image, geo.root, text);
        geo.unchain(&mut image, &set);
    }
    let mut fs = common::mount(&image);
    let before = free(&mut fs);
    let grow = fs.resolve_path("/grow.bin").unwrap();
    common::append(&mut fs, grow, 3000, &common::payload(4000, 2));
    let mut expected = common::payload(3000, 1);
    expected.extend(common::payload(4000, 2));
    assert_eq!(read_all(&mut fs, grow), expected);
    fs.forget(grow, 1);
    let shrink = fs.resolve_path("/shrink.bin").unwrap();
    fs.truncate(shrink, 1000).unwrap();
    fs.forget(shrink, 1);
    fs.unlink(root, name("gone.bin")).unwrap();
    assert_eq!(free(&mut fs), before - 8 + 8 + 4);
    fs.sync().unwrap();
    clean(&mut fs, "contiguous");
    let image = common::image(fs);
    let grown = geo.set(&image, geo.root, "grow.bin");
    assert_eq!(image[grown[1] + 1] & 2, 0, "grown files get a FAT chain");
    let shrunk = geo.set(&image, geo.root, "shrink.bin");
    assert_eq!(image[shrunk[1] + 1] & 2, 2, "shrunk files stay contiguous");
    let mut fs = common::mount(&image);
    assert_eq!(fs.read_to_vec("/grow.bin").unwrap(), expected);
    assert_eq!(
        fs.read_to_vec("/shrink.bin").unwrap(),
        common::payload(1000, 1)
    );
    fsck(&image, "contiguous");
}

#[test]
fn rename_keeps_the_id_and_moves_directories() {
    let mut fs = common::small(8 << 20, 4096);
    let root = fs.root();
    let a = create(&mut fs, root, "a", FileType::Dir);
    let b = create(&mut fs, root, "b", FileType::Dir);
    let inner = create(&mut fs, a, "inner", FileType::Dir);
    let file = common::write(&mut fs, inner, "file.txt", b"moved");
    fs.rename(root, name("a"), b, name("A moved"), RenameMode::Replace)
        .unwrap();
    assert_eq!(read_all(&mut fs, file), b"moved");
    assert_eq!(
        fs.read_to_vec("/b/a moved/INNER/file.txt").unwrap(),
        b"moved"
    );
    assert_eq!(fs.parent(a).unwrap(), b);
    fs.forget(b, 1);
    assert_eq!(fs.parent(inner).unwrap(), a);
    fs.forget(a, 1);
    assert_eq!(
        fs.rename(root, name("b"), inner, name("loop"), RenameMode::Replace)
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidInput
    );
    fs.rename(
        inner,
        name("file.txt"),
        inner,
        name("FILE.TXT"),
        RenameMode::Replace,
    )
    .unwrap();
    assert_eq!(list(&mut fs, inner)[0].0, "FILE.TXT");
    assert_eq!(read_all(&mut fs, file), b"moved");
    fs.rename(
        inner,
        name("FILE.TXT"),
        inner,
        name("FILE.TXT"),
        RenameMode::Replace,
    )
    .unwrap();
    fs.forget(file, 1);
    fs.forget(inner, 1);
    fs.sync().unwrap();
    let image = common::image(fs);
    let mut fs = common::mount(&image);
    let deep = fs.resolve_path("/b/A moved/inner").unwrap();
    let parent = fs.parent(deep).unwrap();
    assert_eq!(fs.parent(parent).unwrap(), fs.resolve_path("/b").unwrap());
    clean(&mut fs, "renamed");
    fsck(&image, "renamed");
}

#[test]
fn rename_replaces_or_refuses_existing_targets() {
    let mut fs = common::small(8 << 20, 4096);
    let root = fs.root();
    for (text, data) in [("one", &b"1"[..]), ("two", b"22"), ("three", b"333")] {
        let node = common::write(&mut fs, root, text, data);
        fs.forget(node, 1);
    }
    let empty = create(&mut fs, root, "empty", FileType::Dir);
    let full = create(&mut fs, root, "full", FileType::Dir);
    let child = create(&mut fs, full, "child", FileType::File);
    fs.forget(child, 1);
    let other = create(&mut fs, root, "other", FileType::Dir);
    fs.forget(other, 1);
    let before = free(&mut fs);
    assert_eq!(
        fs.rename(root, name("one"), root, name("TWO"), RenameMode::NoReplace)
            .unwrap_err()
            .kind(),
        ErrorKind::AlreadyExists
    );
    fs.rename(root, name("one"), root, name("TWO"), RenameMode::Replace)
        .unwrap();
    assert_eq!(fs.read_to_vec("/two").unwrap(), b"1");
    assert!(list(&mut fs, root).iter().any(|(n, _)| n == "TWO"));
    assert_eq!(free(&mut fs), before + 1);
    assert_eq!(
        fs.rename(
            root,
            name("three"),
            root,
            name("empty"),
            RenameMode::Replace
        )
        .unwrap_err()
        .kind(),
        ErrorKind::IsADirectory
    );
    assert_eq!(
        fs.rename(
            root,
            name("empty"),
            root,
            name("three"),
            RenameMode::Replace
        )
        .unwrap_err()
        .kind(),
        ErrorKind::NotADirectory
    );
    assert_eq!(
        fs.rename(root, name("other"), root, name("full"), RenameMode::Replace)
            .unwrap_err()
            .kind(),
        ErrorKind::DirectoryNotEmpty
    );
    fs.rename(
        root,
        name("other"),
        root,
        name("empty"),
        RenameMode::Replace,
    )
    .unwrap();
    assert_eq!(fs.stat(empty).unwrap_err().kind(), ErrorKind::NotFound);
    fs.forget(empty, 1);
    let three = fs.resolve_path("/three").unwrap();
    fs.open(three, OpenMode::Write).unwrap();
    assert_eq!(
        fs.rename(root, name("TWO"), root, name("three"), RenameMode::Replace)
            .unwrap_err()
            .kind(),
        ErrorKind::Busy
    );
    assert_eq!(
        fs.unlink(root, name("three")).unwrap_err().kind(),
        ErrorKind::Busy
    );
    fs.close(three).unwrap();
    fs.forget(three, 1);
    assert_eq!(
        fs.rename(
            root,
            name("TWO"),
            root,
            name("bad:name"),
            RenameMode::Replace
        )
        .unwrap_err()
        .kind(),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        fs.rename(root, name("missing"), root, name("x"), RenameMode::Replace)
            .unwrap_err()
            .kind(),
        ErrorKind::NotFound
    );
    fs.forget(full, 1);
    fs.sync().unwrap();
    clean(&mut fs, "replaced");
    fsck(&common::image(fs), "replaced");
}

#[test]
fn rename_keeps_benign_secondary_entries() {
    let mut fs = common::small(4 << 20, 4096);
    let root = fs.root();
    let node = common::write(&mut fs, root, "vendor", b"data");
    fs.forget(node, 1);
    fs.sync().unwrap();
    let mut image = common::image(fs);
    let geo = Geometry::of(&image);
    let set = geo.set(&image, geo.root, "vendor");
    let extra = set[2] + 32;
    assert_eq!(image[extra], 0);
    image[extra] = 0xE0;
    image[extra + 2..extra + 18].copy_from_slice(b"hadris-vendor-id");
    image[set[0] + 1] = 3;
    geo.reseal(&mut image, &[set[0], set[1], set[2], extra]);
    let mut fs = common::mount(&image);
    clean(&mut fs, "vendor entry");
    fs.rename(
        root,
        name("vendor"),
        root,
        name("a longer name for the vendor file"),
        RenameMode::Replace,
    )
    .unwrap();
    fs.sync().unwrap();
    let image = common::image(fs);
    let set = geo.set(&image, geo.root, "a longer name for the vendor file");
    assert_eq!(set.len(), 6);
    assert_eq!(image[set[5]], 0xE0);
    assert_eq!(&image[set[5] + 2..set[5] + 18], b"hadris-vendor-id");
    let mut fs = common::mount(&image);
    clean(&mut fs, "vendor entry kept");
    assert_eq!(
        fs.read_to_vec("/A LONGER NAME FOR THE VENDOR FILE")
            .unwrap(),
        b"data"
    );
    // exfatprogs before 1.2.3 takes every secondary entry after the stream
    // extension as a name entry and rejects the vendor extension entry that
    // the spec (7.8) allows, so it cannot check this image.
    let tools: Vec<Tool> = [Tool::Local, Tool::Docker, Tool::MacOs]
        .into_iter()
        .filter(|tool| common::exfatprogs_version(tool).is_none_or(|v| v >= (1, 2, 3)))
        .collect();
    common::fsck_with(&image, "vendor entry", &tools);
}

/// Gives the file `owner` a Vendor Allocation entry that takes over the
/// clusters of `blob`, whose set directly follows it and is cleared.
fn vendor_allocation(image: &mut [u8], geo: &Geometry, owner: &str, blob: &str, contiguous: bool) {
    let set = geo.set(image, geo.root, owner);
    let blob = geo.set(image, geo.root, blob);
    assert_eq!(blob[0], set[2] + 32);
    if contiguous {
        geo.unchain(image, &blob);
    }
    let first = le32(image, blob[1] + 20);
    let len = image[blob[1] + 24..blob[1] + 32].to_vec();
    for &at in &blob {
        image[at] &= 0x7F;
    }
    let extra = blob[0];
    image[extra..extra + 32].fill(0);
    image[extra] = 0xE1;
    image[extra + 1] = if contiguous { 0x03 } else { 0x01 };
    image[extra + 2..extra + 18].copy_from_slice(b"hadris-vendor-id");
    image[extra + 20..extra + 24].copy_from_slice(&first.to_le_bytes());
    image[extra + 24..extra + 32].copy_from_slice(&len);
    image[set[0] + 1] = 3;
    geo.reseal(image, &[set[0], set[1], set[2], extra]);
}

#[test]
fn remove_and_replace_free_vendor_allocations() {
    let mut fs = common::small(4 << 20, 4096);
    let root = fs.root();
    let before = free(&mut fs);
    for (text, len) in [
        ("vendor", 10),
        ("blob1", 6000),
        ("target", 10),
        ("blob2", 6000),
        ("other", 10),
    ] {
        let node = common::write(&mut fs, root, text, &common::payload(len, 1));
        fs.forget(node, 1);
    }
    fs.sync().unwrap();
    let mut image = common::image(fs);
    let geo = Geometry::of(&image);
    vendor_allocation(&mut image, &geo, "vendor", "blob1", false);
    vendor_allocation(&mut image, &geo, "target", "blob2", true);
    let mut fs = common::mount(&image);
    clean(&mut fs, "vendor allocations");
    assert_eq!(free(&mut fs), before - 7);
    fs.unlink(root, name("vendor")).unwrap();
    assert_eq!(free(&mut fs), before - 4);
    fs.rename(
        root,
        name("other"),
        root,
        name("target"),
        RenameMode::Replace,
    )
    .unwrap();
    assert_eq!(free(&mut fs), before - 1);
    fs.sync().unwrap();
    clean(&mut fs, "vendor allocations freed");
    let mut fs = common::mount(&common::image(fs));
    assert_eq!(free(&mut fs), before - 1);
    let tools: Vec<Tool> = [Tool::Local, Tool::Docker, Tool::MacOs]
        .into_iter()
        .filter(|tool| common::exfatprogs_version(tool).is_none_or(|v| v >= (1, 2, 3)))
        .collect();
    common::fsck_with(&common::image(fs), "vendor allocations freed", &tools);
}

#[test]
fn remove_files_and_directories() {
    let mut fs = common::small(8 << 20, 4096);
    let root = fs.root();
    let before = free(&mut fs);
    let dir = create(&mut fs, root, "dir", FileType::Dir);
    let file = common::write(&mut fs, dir, "file", &common::payload(10_000, 3));
    assert_eq!(free(&mut fs), before - 1 - 3);
    assert_eq!(
        fs.remove_any(root, name("dir")).unwrap_err().kind(),
        ErrorKind::DirectoryNotEmpty
    );
    assert_eq!(
        fs.unlink(root, name("dir")).unwrap_err().kind(),
        ErrorKind::IsADirectory
    );
    assert_eq!(
        fs.rmdir(dir, name("file")).unwrap_err().kind(),
        ErrorKind::NotADirectory
    );
    fs.unlink(dir, name("FILE")).unwrap();
    assert_eq!(
        fs.read(file, 0, &mut [0; 4]).unwrap_err().kind(),
        ErrorKind::NotFound
    );
    fs.forget(file, 1);
    fs.forget(dir, 1);
    fs.rmdir(root, name("dir")).unwrap();
    assert_eq!(free(&mut fs), before);
    assert_eq!(
        fs.remove_any(root, name("dir")).unwrap_err().kind(),
        ErrorKind::NotFound
    );
    assert_eq!(fs.open_nodes(), 1);
    fs.sync().unwrap();
    clean(&mut fs, "removed");
    fsck(&common::image(fs), "removed");
}

#[test]
fn set_len_shrinks_frees_and_grows_zeroed() {
    let mut fs = common::small(4 << 20, 4096);
    let root = fs.root();
    let before = free(&mut fs);
    let node = common::write(&mut fs, root, "f", &common::payload(20_000, 1));
    assert_eq!(free(&mut fs), before - 5);
    fs.truncate(node, 5000).unwrap();
    assert_eq!(free(&mut fs), before - 2);
    fs.truncate(node, 9000).unwrap();
    let data = read_all(&mut fs, node);
    assert_eq!(&data[..5000], &common::payload(20_000, 1)[..5000]);
    assert!(data[5000..].iter().all(|&b| b == 0));
    fs.truncate(node, 0).unwrap();
    assert_eq!(free(&mut fs), before);
    fs.forget(node, 1);
    fs.sync().unwrap();
    clean(&mut fs, "truncate");
    fsck(&common::image(fs), "truncate");
}

#[test]
fn no_space_changes_nothing_and_space_is_reclaimed() {
    let mut fs = common::small(2 << 20, 4096);
    let root = fs.root();
    let total = free(&mut fs);
    let node = common::write(&mut fs, root, "fill", &[]);
    let huge = vec![5u8; (total as usize + 1) * 4096];
    assert_eq!(
        fs.write(node, 0, &huge).unwrap_err().kind(),
        ErrorKind::NoSpace
    );
    assert_eq!(free(&mut fs), total);
    assert_eq!(fs.stat(node).unwrap().len(), 0);
    fs.write(node, 0, &huge[..total as usize * 4096]).unwrap();
    assert_eq!(free(&mut fs), 0);
    assert_eq!(
        fs.write(node, total * 4096, b"x").unwrap_err().kind(),
        ErrorKind::NoSpace
    );
    assert_eq!(
        fs.mkdir(root, name("dir"), &SetAttr::new())
            .unwrap_err()
            .kind(),
        ErrorKind::NoSpace
    );
    fs.forget(node, 1);
    fs.sync().unwrap();
    clean(&mut fs, "full");
    fs.unlink(root, name("fill")).unwrap();
    assert_eq!(free(&mut fs), total);
    let again = common::write(&mut fs, root, "again", &huge[..total as usize * 4096]);
    fs.forget(again, 1);
    fs.sync().unwrap();
    clean(&mut fs, "refilled");
    fsck(&common::image(fs), "refilled");
}

#[test]
fn volume_flags_and_percent_in_use() {
    let image = common::image(common::small(4 << 20, 4096));
    assert_eq!(image[106] & 2, 0);
    let mut fs = common::mount(&image);
    let root = fs.root();
    let node = common::write(&mut fs, root, "f", &common::payload(100_000, 1));
    fs.forget(node, 1);
    let dirty = fs.into_inner().into_inner();
    assert_eq!(dirty[106] & 2, 2, "the first write sets VolumeDirty");
    let mut fs = common::mount(&dirty);
    fs.sync().unwrap();
    let still = common::image(fs);
    assert_eq!(still[106] & 2, 2, "a volume dirty at mount stays dirty");
    let mut fs = common::mount(&image);
    let node = common::write(&mut fs, root, "f", &common::payload(1_000_000, 1));
    fs.forget(node, 1);
    fs.sync().unwrap();
    let total = fs.statfs().unwrap().total_blocks();
    let used = total - fs.statfs().unwrap().free_blocks();
    let synced = common::image(fs);
    assert_eq!(synced[106] & 2, 0, "sync clears VolumeDirty");
    assert_eq!(synced[112] as u64, used * 100 / total);
    fsck(&synced, "flags");
}

/// A device that refuses writes, or fails them with a device error after
/// `budget` writes.
struct Faulty {
    inner: Device,
    budget: Option<usize>,
    refuse: bool,
}

impl hadris_io::ErrorType for Faulty {
    type Error = std::io::Error;
}

impl BlockDevice for Faulty {
    fn block_size(&self) -> BlockSize {
        self.inner.block_size()
    }

    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }

    fn writable(&self) -> bool {
        self.inner.writable()
    }

    fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), Error<Self::Error>> {
        self.inner
            .read_blocks(first, buf)
            .map_err(|err| err.map_device(|never| match never {}))
    }

    fn write_blocks(&mut self, first: BlockIndex, buf: &[u8]) -> Result<(), Error<Self::Error>> {
        if self.refuse {
            return Err(Error::new(
                hadris_fs::ErrorKind::ReadOnly,
                "write protected",
            ));
        }
        match &mut self.budget {
            Some(0) => Err(Error::device(
                std::io::Error::other("injected fault"),
                "write failed",
            )),
            Some(left) => {
                *left -= 1;
                self.inner
                    .write_blocks(first, buf)
                    .map_err(|err| err.map_device(|never| match never {}))
            }
            None => self
                .inner
                .write_blocks(first, buf)
                .map_err(|err| err.map_device(|never| match never {})),
        }
    }
}

fn populated() -> Vec<u8> {
    let mut fs = common::small(4 << 20, 512);
    let root = fs.root();
    for (text, len) in [
        ("a long file name.txt", 3000),
        ("lower.txt", 10),
        ("grown.bin", 9000),
    ] {
        let node = common::write(&mut fs, root, text, &common::payload(len, 4));
        fs.forget(node, 1);
    }
    let dir = create(&mut fs, root, "Nested Dir", FileType::Dir);
    for i in 0..20 {
        let node = create(&mut fs, dir, &format!("child {i}"), FileType::File);
        fs.forget(node, 1);
    }
    fs.forget(dir, 1);
    fs.sync().unwrap();
    common::image(fs)
}

#[test]
fn refused_writes_make_the_volume_read_only() {
    let before = populated();
    let dev = Faulty {
        inner: common::device(before.clone(), 512),
        budget: None,
        refuse: true,
    };
    let mut fs = ExFatFs::mount(dev, MountOptions::new()).unwrap();
    assert!(FileSystem::capabilities(&fs).writable());
    let root = fs.root();
    let file = fs.lookup(root, name("lower.txt")).unwrap();
    assert_eq!(
        fs.write(file, 0, b"x").unwrap_err().kind(),
        ErrorKind::ReadOnly
    );
    assert!(fs.is_read_only());
    assert!(!FileSystem::capabilities(&fs).writable());
    let meta = SetAttr::new();
    let kinds = [
        fs.create(root, name("new"), &meta).map(|_| ()),
        fs.remove_any(root, name("lower.txt")),
        fs.rename(
            root,
            name("lower.txt"),
            root,
            name("x"),
            RenameMode::Replace,
        ),
        fs.truncate(file, 0),
        fs.setattr(file, &meta.with_attributes(Attributes::HIDDEN)),
        fs.set_label(None),
    ]
    .map(|result| result.unwrap_err().kind());
    assert_eq!(kinds, [ErrorKind::ReadOnly; 6]);
    assert_eq!(read_all(&mut fs, file), common::payload(10, 4));
    fs.sync().unwrap();
    fs.forget(file, 1);
    assert_eq!(fs.into_inner().inner.into_inner(), before);

    let dev = common::device(before.clone(), 512);
    let mut fs = ExFatFs::mount(dev, MountOptions::new().read_only()).unwrap();
    assert_eq!(fs.set_label(None).unwrap_err().kind(), ErrorKind::ReadOnly);
    assert_eq!(fs.into_inner().into_inner(), before);
}

/// Every operation interrupted after each of its writes leaves a volume
/// that mounts and reads, and whose only findings are the leftovers an
/// interruption may leave.
#[test]
fn interrupted_operations_leave_readable_volumes() {
    let before = populated();
    let meta = SetAttr::new();
    for op in 0..7 {
        for budget in 0..60 {
            let dev = Faulty {
                inner: common::device(before.clone(), 512),
                budget: Some(budget),
                refuse: false,
            };
            let mut fs = ExFatFs::mount(dev, MountOptions::new()).unwrap();
            let root = fs.root();
            let result = match op {
                0 => fs.mkdir(root, name("a new directory"), &meta).map(|_| ()),
                1 => fs.remove_any(root, name("a long file name.txt")),
                2 => fs.rename(
                    root,
                    name("Nested Dir"),
                    root,
                    name("Renamed Dir"),
                    RenameMode::Replace,
                ),
                3 => fs.rename(
                    root,
                    name("lower.txt"),
                    root,
                    name("grown.bin"),
                    RenameMode::Replace,
                ),
                4 => fs
                    .resolve_path("/grown.bin")
                    .and_then(|node| fs.write(node, 8_000, &[3u8; 20_000]).map(|_| ())),
                5 => fs
                    .resolve_path("/a long file name.txt")
                    .and_then(|node| fs.truncate(node, 100)),
                _ => fs.set_label(Some(VolumeLabel::new("Label").unwrap())),
            };
            let finished = result.is_ok();
            if let Err(err) = &result {
                assert_eq!(err.kind(), ErrorKind::Io, "op {op} budget {budget}");
            }
            let image = fs.into_inner().inner.into_inner();
            let mut fresh = common::mount(&image);
            for path in ["/lower.txt", "/grown.bin", "/a long file name.txt"] {
                let _ = fresh.read_to_vec(path);
            }
            let _ = common::names(&mut fresh, "/");
            if finished {
                break;
            }
        }
    }
}

/// A device whose bytes the test can change under a mounted volume.
struct Shared(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);

impl hadris_io::ErrorType for Shared {
    type Error = core::convert::Infallible;
}

impl BlockDevice for Shared {
    fn block_size(&self) -> BlockSize {
        BlockSize::new(512).unwrap()
    }

    fn block_count(&self) -> u64 {
        self.0.borrow().len() as u64 / 512
    }

    fn writable(&self) -> bool {
        true
    }

    fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), Error<Self::Error>> {
        let at = first.get() as usize * 512;
        buf.copy_from_slice(&self.0.borrow()[at..at + buf.len()]);
        Ok(())
    }

    fn write_blocks(&mut self, first: BlockIndex, buf: &[u8]) -> Result<(), Error<Self::Error>> {
        let at = first.get() as usize * 512;
        self.0.borrow_mut()[at..at + buf.len()].copy_from_slice(buf);
        Ok(())
    }
}

#[test]
fn sync_writes_the_other_nodes_past_a_damaged_entry_set() {
    let image = std::rc::Rc::new(std::cell::RefCell::new(common::image(common::small(
        4 << 20,
        4096,
    ))));
    let mut fs = ExFatFs::mount(Shared(image.clone()), MountOptions::new()).unwrap();
    let root = fs.root();
    let texts = ["first.bin", "second.bin", "third.bin", "fourth.bin"];
    let nodes: Vec<NodeId> = texts
        .iter()
        .map(|text| fs.create(root, name(text), &SetAttr::new()).unwrap())
        .collect();
    for &node in &nodes {
        fs.write(node, 0, &[7u8; 100]).unwrap();
        fs.write(node, 100, &[7u8; 4900]).unwrap();
    }
    let damaged = nodes[0].get() as usize * 32;
    image.borrow_mut()[damaged + 32 + 4] ^= 0xFF;
    assert_eq!(fs.sync().unwrap_err().kind(), ErrorKind::Corrupt);
    assert_eq!(fs.open_nodes(), 1 + nodes.len());
    fs.sync().unwrap();
    let mut fresh = common::mount(&image.borrow());
    for text in &texts[1..] {
        assert_eq!(fresh.read_to_vec(&format!("/{text}")).unwrap(), [7u8; 5000]);
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
    "delta with a much longer name than fifteen.dat",
    "e",
    "Zeta.TXT",
    "\u{3B7}ta.txt",
    "theta.theta",
];

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

/// Random operations on two directories, checked against a map after each
/// step and against a fresh mount, `check` and the native tools at the end.
#[test]
fn random_operations_match_a_model() {
    for (seed, size, cluster) in [
        (1u64, 4 << 20, 512u32),
        (7, 8 << 20, 4096),
        (99, 16 << 20, 1024),
    ] {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut fs = common::small(size, cluster);
        let root = fs.root();
        let sub = create(&mut fs, root, "sub", FileType::Dir);
        let dirs = [root, sub];
        let mut model: BTreeMap<(usize, usize), Model> = BTreeMap::new();
        for step in 0..400 {
            let d = rng.below(2) as usize;
            let i = rng.below(POOL.len() as u64) as usize;
            let key = (d, i);
            let text = POOL[i];
            let context = format!("seed {seed} step {step}");
            match rng.below(6) {
                0 => {
                    let kind = if rng.below(4) == 0 {
                        FileType::Dir
                    } else {
                        FileType::File
                    };
                    let result = match kind {
                        FileType::Dir => fs.mkdir(dirs[d], name(text), &SetAttr::new()),
                        _ => fs.create(dirs[d], name(text), &SetAttr::new()),
                    };
                    match model.entry(key) {
                        Entry::Occupied(_) => {
                            assert_eq!(
                                result.unwrap_err().kind(),
                                ErrorKind::AlreadyExists,
                                "{context}"
                            )
                        }
                        Entry::Vacant(slot) => {
                            fs.forget(result.unwrap(), 1);
                            slot.insert(match kind {
                                FileType::Dir => Model::Dir,
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
                    common::append(&mut fs, node, offset, &bytes);
                    let end = offset as usize + len;
                    if data.len() < end && len > 0 {
                        data.resize(end, 0);
                    }
                    data[offset as usize..end].copy_from_slice(&bytes);
                    fs.forget(node, 1);
                }
                3 => {
                    let Some(Model::File(data)) = model.get_mut(&key) else {
                        continue;
                    };
                    let node = fs.lookup(dirs[d], name(text)).unwrap();
                    let len = rng.below(data.len() as u64 * 2 + 100) as usize;
                    fs.truncate(node, len as u64).unwrap();
                    data.resize(len, 0);
                    fs.forget(node, 1);
                }
                4 => {
                    let result = fs.remove_any(dirs[d], name(text));
                    match model.get(&key) {
                        None => {
                            assert_eq!(result.unwrap_err().kind(), ErrorKind::NotFound, "{context}")
                        }
                        Some(_) => {
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
                        RenameMode::Replace,
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
                clean(&mut fs, &context);
            }
            check_model(&mut fs, &dirs, &model, &context);
        }
        fs.forget(sub, 1);
        fs.sync().unwrap();
        assert_eq!(fs.open_nodes(), 1);
        clean(&mut fs, "model");
        let image = common::image(fs);
        let mut fresh = common::mount(&image);
        for ((d, i), value) in &model {
            if let Model::File(data) = value {
                let dir = if *d == 0 { "" } else { "/sub" };
                assert_eq!(
                    &common::read(&mut fresh, &format!("{dir}/{}", POOL[*i])),
                    data
                );
            }
        }
        fsck(&image, &format!("model seed {seed}"));
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
            Model::Dir => assert_eq!(fs.stat(node).unwrap().file_type(), FileType::Dir),
        }
        fs.forget(node, 1);
    }
}

#[test]
fn unpinned_ids_write_through() {
    let mut fs = common::small(4 << 20, 4096);
    let root = fs.root();
    let node = common::write(&mut fs, root, "f", b"abc");
    fs.forget(node, 1);
    let entry = fs.readdir(root, DirCursor::START).unwrap().unwrap();
    fs.write(entry.node(), 3, b"def").unwrap();
    assert_eq!(fs.read_to_vec("/f").unwrap(), b"abcdef");
    assert_eq!(fs.open_nodes(), 1);
    let _ = le32;
}

/// Both FATs and both Allocation Bitmaps of a TexFAT volume.
fn texfat_copies(image: &[u8]) -> ([&[u8]; 2], [Vec<u8>; 2]) {
    let geo = Geometry::of(image);
    let sector = 1usize << image[108];
    let fat_len = le32(image, 84) as usize * sector;
    let fats = [
        &image[geo.fat..geo.fat + fat_len],
        &image[geo.fat + fat_len..geo.fat + 2 * fat_len],
    ];
    let entries = geo.root_entries(image, 0x81);
    assert_eq!(entries.len(), 2);
    let bitmaps: Vec<Vec<u8>> = entries
        .iter()
        .map(|&at| {
            let len = common::le64(image, at + 24) as usize;
            let mut bits = Vec::new();
            for cluster in geo.chain(image, le32(image, at + 20)) {
                bits.extend_from_slice(&image[geo.at(cluster)..geo.at(cluster) + geo.cluster]);
            }
            bits.truncate(len);
            bits
        })
        .collect();
    assert_eq!([image[entries[0] + 1], image[entries[1] + 1]], [0, 1]);
    (fats, bitmaps.try_into().unwrap())
}

fn texfat_workload(fs: &mut Fs, tag: &str) {
    let root = fs.root();
    let dir = common::mkdir(fs, root, &format!("dir {tag}"));
    for index in 0..40 {
        let node = common::write(
            fs,
            dir,
            &format!("{tag} {index}"),
            &common::payload(index * 700, index as u8),
        );
        fs.forget(node, 1);
    }
    let doomed = fs.resolve_path(&format!("/dir {tag}/{tag} 7")).unwrap();
    fs.unlink(dir, Name::new(&format!("{tag} 7"))).unwrap();
    fs.forget(doomed, 1);
    fs.forget(dir, 1);
    fs.sync().unwrap();
}

/// exfatprogs refuses a FAT count of 2, so only macOS `fsck_exfat` checks
/// the image.
#[test]
fn texfat_keeps_both_fats_and_bitmaps() {
    let options = hadris_fat::exfat::ExFatOptions::new()
        .with_fat_count(2)
        .with_cluster_size(4096);
    let mut fs = common::formatted(16 << 20, options);
    clean(&mut fs, "formatted");
    texfat_workload(&mut fs, "a");
    clean(&mut fs, "after writes");
    let mut image = common::image(fs);
    assert_eq!(image[110], 2);
    let (fats, bitmaps) = texfat_copies(&image);
    assert_eq!(fats[0], fats[1]);
    assert_eq!(bitmaps[0], bitmaps[1]);
    common::fsck_with(&image, "texfat", &[common::Tool::MacOs]);

    image[106] |= 1;
    let mut fs = common::mount(&image);
    assert_eq!(
        common::read(&mut fs, "/dir a/a 39"),
        common::payload(39 * 700, 39)
    );
    texfat_workload(&mut fs, "b");
    clean(&mut fs, "second FAT active");
    let image = common::image(fs);
    assert_eq!(image[106] & 1, 1, "ActiveFat is kept");
    let (fats, bitmaps) = texfat_copies(&image);
    assert_eq!(fats[0], fats[1]);
    assert_eq!(bitmaps[0], bitmaps[1]);
}

#[test]
fn texfat_with_one_bitmap_mounts() {
    let options = hadris_fat::exfat::ExFatOptions::new().with_fat_count(2);
    let fs = common::formatted(8 << 20, options);
    let mut image = common::image(fs);
    let geo = Geometry::of(&image);
    let second = geo.root_entries(&image, 0x81)[1];
    image[second] &= 0x7F;
    let mut fs = common::mount(&image);
    texfat_workload(&mut fs, "c");
    assert_eq!(
        common::read(&mut fs, "/dir c/c 3"),
        common::payload(2100, 3)
    );
    image[106] |= 1;
    assert_eq!(
        ExFatFs::mount(common::device(image, 512), MountOptions::new())
            .unwrap_err()
            .error()
            .kind(),
        ErrorKind::Corrupt,
        "the active FAT has no bitmap"
    );
}

#[test]
fn utc_offset_stamps_new_times() {
    let image = common::image(common::small(4 << 20, 4096));
    let utc = civil(2030, 8, None);
    let options = MountOptions::new()
        .with_clock(Box::leak(Box::new(FixedClock(utc))))
        .with_utc_offset(-300)
        .unwrap();
    let mut fs = ExFatFs::mount(common::device(image, 512), options).unwrap();
    let root = fs.root();
    let node = fs.create(root, name("zoned.txt"), &SetAttr::new()).unwrap();
    fs.forget(node, 1);
    let image = common::image(fs);
    let mut fs = common::mount(&image);
    let root = fs.root();
    let node = fs.lookup(root, name("zoned.txt")).unwrap();
    let created = fs.stat(node).unwrap().created().unwrap();
    assert_eq!(created.unix_seconds(), utc.unix_seconds());
    assert_eq!(created.utc_offset_minutes(), Some(-300));
    fs.forget(node, 1);
}

#[test]
fn node_limit_caps_pinned_nodes() {
    let image = common::image(common::small(4 << 20, 4096));
    let options = MountOptions::new().with_node_limit(1);
    let mut fs = ExFatFs::mount(common::device(image, 512), options).unwrap();
    let root = fs.root();
    let held = fs.create(root, name("a.txt"), &SetAttr::new()).unwrap();
    assert_eq!(
        fs.create(root, name("b.txt"), &SetAttr::new())
            .unwrap_err()
            .kind(),
        ErrorKind::LimitExceeded
    );
    fs.forget(held, 1);
    let b = fs.create(root, name("b.txt"), &SetAttr::new()).unwrap();
    assert_eq!(
        fs.lookup(root, name("a.txt")).unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );
    fs.forget(b, 1);
}

#[test]
fn unmount_syncs_and_returns_the_device() {
    let image = common::image(common::small(4 << 20, 4096));
    let mut fs = common::mount(&image);
    let root = fs.root();
    let node = fs.create(root, name("kept.bin"), &SetAttr::new()).unwrap();
    assert_eq!(fs.write(node, 0, &[9u8; 9000]).unwrap(), 9000);
    let bytes = fs.unmount().unwrap().into_inner();
    let mut fs = common::mount(&bytes);
    let root = fs.root();
    let node = fs.lookup(root, name("kept.bin")).unwrap();
    assert_eq!(fs.stat(node).unwrap().len(), 9000);
    fs.forget(node, 1);
    clean(&mut fs, "unmount");
}

#[test]
fn extras_map_files_and_set_the_serial() {
    let mut fs = common::small(8 << 20, 4096);
    assert!(!fs.was_dirty());
    let root = fs.root();
    let data = common::payload(4096 + 100, 3);
    let node = common::write(&mut fs, root, "f.bin", &data);
    fs.truncate(node, 3 * 4096).unwrap();
    let mut out = [hadris_fs::Extent::new(0, 0); 4];
    let n = fs.extents(node, 0, &mut out).unwrap();
    let mut mapped = Vec::new();
    for extent in &out[..n] {
        assert_eq!(extent.file_offset(), mapped.len() as u64);
        let mut buf = vec![0; extent.len() as usize];
        fs.read_raw(extent.offset(), &mut buf).unwrap();
        if extent.is_unwritten() {
            buf.fill(0);
        }
        mapped.extend(buf);
    }
    assert!(out[..n].last().unwrap().is_unwritten());
    let mut expected = data.clone();
    expected.resize(3 * 4096, 0);
    assert_eq!(mapped, expected);
    assert_eq!(fs.extents(node, 3 * 4096, &mut out).unwrap(), 0);

    let n = fs.records(node, &mut out).unwrap();
    assert_eq!(n, 1);
    assert_eq!(out[0].len(), 3 * 32);
    let mut entry = [0u8; 32];
    fs.read_raw(out[0].offset(), &mut entry).unwrap();
    assert_eq!(entry[0], 0x85);
    assert_eq!(fs.records(root, &mut out).unwrap(), 0);
    assert_eq!(
        fs.records(node, &mut []).unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );

    fs.set_volume_serial(0x0BAD_F00D).unwrap();
    assert_eq!(fs.info().volume_serial(), 0x0BAD_F00D);
    let image = fs.unmount().unwrap().into_inner();
    assert_eq!(common::mount(&image).info().volume_serial(), 0x0BAD_F00D);
    let backup = ExFatFs::mount(
        common::device(image.clone(), 512),
        MountOptions::new().backup_boot(),
    )
    .unwrap();
    assert_eq!(backup.info().volume_serial(), 0x0BAD_F00D);
    fsck(&image, "new serial");
}

#[test]
fn was_dirty_reads_volume_dirty() {
    let mut image = common::image(common::small(4 << 20, 4096));
    assert!(!common::mount(&image).was_dirty());
    image[106] |= 0x02;
    assert!(common::mount(&image).was_dirty());
}
