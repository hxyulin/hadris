//! `ExFatFs` writes: names, times and attributes, labels, directory growth,
//! rename and remove, allocation, the volume flags, refused and failing
//! writes, and random operations checked against a model, the checker and
//! the native tools.

#[path = "common/exfat.rs"]
mod common;

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use common::{Device, Fs, Geometry, Tool, clean, fsck, le32};
use hadris_fat::exfat::sync::ExFatFs;
use hadris_fat::exfat::{MountOptions, VolumeLabel};
use hadris_fs::sync::{DriverExt, FsDriver};
use hadris_fs::{
    Attributes, CivilDate, CivilTime, Clock, DateTime, DirCursor, ErrorKind, FileTimes, FileType,
    HeapTable, Name, NameBuf, NewNode, NodeId, RemoveKind, RenameFlags, SetMetadata,
};
use hadris_storage::OutOfRange;
use hadris_storage::sync::BlockDevice;
use hadris_storage::{BlockIndex, BlockSize, WriteError};

fn name(text: &str) -> &Name {
    Name::new(text).unwrap()
}

fn create(fs: &mut Fs, dir: NodeId, text: &str, kind: NewNode<'_>) -> NodeId {
    fs.create(dir, name(text), kind, &SetMetadata::new())
        .unwrap()
}

fn read_all<D: BlockDevice, C: Clock>(fs: &mut ExFatFs<D, HeapTable, C>, node: NodeId) -> Vec<u8> {
    let len = fs.node_metadata(node).unwrap().len() as usize;
    let mut out = vec![0u8; len];
    let mut done = 0;
    while done < len {
        let n = fs.read_at(node, done as u64, &mut out[done..]).unwrap();
        assert!(n > 0);
        done += n;
    }
    out
}

fn list(fs: &mut Fs, dir: NodeId) -> Vec<(String, FileType)> {
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

fn free(fs: &mut Fs) -> u64 {
    fs.stats().unwrap().free_blocks()
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
    let meta = SetMetadata::new()
        .with_times(
            FileTimes::new()
                .with_created(Some(created))
                .with_modified(Some(modified))
                .with_accessed(Some(accessed)),
        )
        .with_attributes(Attributes::HIDDEN | Attributes::SYSTEM);
    let node = fs
        .create(root, name("stamped.txt"), NewNode::File, &meta)
        .unwrap();
    let dir = fs
        .create(root, name("stamped dir"), NewNode::Dir, &meta)
        .unwrap();
    fs.forget(dir);
    fs.forget(node);
    fs.sync().unwrap();
    let mut fs = common::mount(&common::image(fs));
    let node = fs.resolve("/STAMPED.TXT").unwrap();
    let got = fs.node_metadata(node).unwrap();
    assert_eq!(got.times().created(), Some(created));
    assert_eq!(got.times().modified(), Some(modified));
    assert_eq!(
        got.times().accessed().map(|t| t.to_civil().0),
        Some(accessed.to_civil().0)
    );
    assert_eq!(got.attributes(), Attributes::HIDDEN | Attributes::SYSTEM);
    let dir = fs.resolve("/stamped dir").unwrap();
    assert_eq!(
        fs.node_metadata(dir).unwrap().attributes(),
        Attributes::HIDDEN | Attributes::SYSTEM
    );
    assert_eq!(fs.node_metadata(dir).unwrap().file_type(), FileType::Dir);

    fs.set_metadata(
        node,
        &SetMetadata::new().with_attributes(Attributes::READ_ONLY),
    )
    .unwrap();
    assert_eq!(
        fs.node_metadata(node).unwrap().attributes(),
        Attributes::READ_ONLY
    );
    fs.write_at(node, 0, b"x").unwrap();
    fs.sync_node(node).unwrap();
    assert_eq!(
        fs.node_metadata(node).unwrap().attributes(),
        Attributes::READ_ONLY | Attributes::ARCHIVE
    );
    fs.forget(node);
    fs.forget(dir);
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
    let mut fs = ExFatFs::open_with(
        common::device(image, 512),
        MountOptions::new()
            .with_table(HeapTable::new())
            .with_clock(FixedClock(now)),
    )
    .unwrap();
    let root = fs.root();
    let node = fs
        .create(root, name("now.txt"), NewNode::File, &SetMetadata::new())
        .unwrap();
    let times = fs.node_metadata(node).unwrap().times();
    assert_eq!(times.created(), Some(now));
    assert_eq!(times.modified(), Some(now));
    fs.forget(node);
}

#[test]
fn long_names_up_to_255_units() {
    let mut fs = common::small(4 << 20, 4096);
    let root = fs.root();
    let longest = "\u{1F600}".repeat(127) + "e";
    assert_eq!(longest.encode_utf16().count(), 255);
    let node = create(&mut fs, root, &longest, NewNode::File);
    fs.forget(node);
    let too_long = "\u{1F600}".repeat(128);
    assert_eq!(
        fs.create(root, name(&too_long), NewNode::File, &SetMetadata::new())
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
            fs.create(root, name(bad), NewNode::File, &SetMetadata::new())
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidInput,
            "{bad:?}"
        );
    }
    let node = create(&mut fs, root, "Straße Ωmega", NewNode::File);
    fs.forget(node);
    for twin in ["STRAßE ΩMEGA", "straße ωmega"] {
        assert_eq!(
            fs.create(root, name(twin), NewNode::File, &SetMetadata::new())
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
    assert!(fs.resolve(&format!("/{}", longest.to_uppercase())).is_ok());
    fsck(&image, "long names");
}

#[test]
fn labels_are_set_and_removed() {
    let mut fs = common::small(4 << 20, 4096);
    assert!(fs.label().unwrap().is_none());
    fs.set_label(Some(VolumeLabel::new("Première").unwrap()))
        .unwrap();
    assert_eq!(fs.label().unwrap().unwrap().to_string(), "Première");
    fs.set_label(Some(VolumeLabel::new("Second").unwrap()))
        .unwrap();
    fs.sync().unwrap();
    let image = common::image(fs);
    let geo = Geometry::of(&image);
    assert_eq!(geo.root_entries(&image, 0x83).len(), 1);
    fsck(&image, "label");
    let mut fs = common::mount(&image);
    assert_eq!(fs.label().unwrap().unwrap().to_string(), "Second");
    fs.set_label(None).unwrap();
    assert!(fs.label().unwrap().is_none());
    fs.sync().unwrap();
    clean(&mut fs, "no label");
    fsck(&common::image(fs), "no label");
}

#[test]
fn directories_grow_across_clusters() {
    let mut fs = common::small(8 << 20, 512);
    let root = fs.root();
    let dir = create(&mut fs, root, "grown", NewNode::Dir);
    for i in 0..200 {
        let node = create(&mut fs, dir, &format!("entry number {i:03}"), NewNode::File);
        fs.forget(node);
        if i % 50 == 0 {
            let spacer = common::write(&mut fs, root, &format!("spacer {i}"), &[7; 700]);
            fs.forget(spacer);
        }
    }
    for i in 0..60 {
        let node = create(
            &mut fs,
            root,
            &format!("root entry number {i:03}"),
            NewNode::File,
        );
        fs.forget(node);
    }
    let clusters = common::chain(&mut fs, dir);
    assert_eq!(clusters.len(), 200 * 4 * 32 / 512);
    assert!(clusters.windows(2).any(|pair| pair[1] != pair[0] + 1));
    assert!(common::chain(&mut fs, root).len() > 1);
    assert_eq!(list(&mut fs, dir).len(), 200);
    fs.forget(dir);
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
        fs.forget(node);
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
    let grow = fs.resolve("/grow.bin").unwrap();
    common::append(&mut fs, grow, 3000, &common::payload(4000, 2));
    let mut expected = common::payload(3000, 1);
    expected.extend(common::payload(4000, 2));
    assert_eq!(read_all(&mut fs, grow), expected);
    fs.forget(grow);
    let shrink = fs.resolve("/shrink.bin").unwrap();
    fs.set_len(shrink, 1000).unwrap();
    fs.forget(shrink);
    fs.remove(root, name("gone.bin"), RemoveKind::File).unwrap();
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
    let a = create(&mut fs, root, "a", NewNode::Dir);
    let b = create(&mut fs, root, "b", NewNode::Dir);
    let inner = create(&mut fs, a, "inner", NewNode::Dir);
    let file = common::write(&mut fs, inner, "file.txt", b"moved");
    fs.rename(root, name("a"), b, name("A moved"), RenameFlags::empty())
        .unwrap();
    assert_eq!(read_all(&mut fs, file), b"moved");
    assert_eq!(
        fs.read_to_vec("/b/a moved/INNER/file.txt").unwrap(),
        b"moved"
    );
    assert_eq!(fs.parent(a).unwrap(), b);
    fs.forget(b);
    assert_eq!(fs.parent(inner).unwrap(), a);
    fs.forget(a);
    assert_eq!(
        fs.rename(root, name("b"), inner, name("loop"), RenameFlags::empty())
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidInput
    );
    fs.rename(
        inner,
        name("file.txt"),
        inner,
        name("FILE.TXT"),
        RenameFlags::empty(),
    )
    .unwrap();
    assert_eq!(list(&mut fs, inner)[0].0, "FILE.TXT");
    assert_eq!(read_all(&mut fs, file), b"moved");
    fs.rename(
        inner,
        name("FILE.TXT"),
        inner,
        name("FILE.TXT"),
        RenameFlags::empty(),
    )
    .unwrap();
    fs.forget(file);
    fs.forget(inner);
    fs.sync().unwrap();
    let image = common::image(fs);
    let mut fs = common::mount(&image);
    let deep = fs.resolve("/b/A moved/inner").unwrap();
    let parent = fs.parent(deep).unwrap();
    assert_eq!(fs.parent(parent).unwrap(), fs.resolve("/b").unwrap());
    clean(&mut fs, "renamed");
    fsck(&image, "renamed");
}

#[test]
fn rename_replaces_or_refuses_existing_targets() {
    let mut fs = common::small(8 << 20, 4096);
    let root = fs.root();
    for (text, data) in [("one", &b"1"[..]), ("two", b"22"), ("three", b"333")] {
        let node = common::write(&mut fs, root, text, data);
        fs.forget(node);
    }
    let empty = create(&mut fs, root, "empty", NewNode::Dir);
    let full = create(&mut fs, root, "full", NewNode::Dir);
    let child = create(&mut fs, full, "child", NewNode::File);
    fs.forget(child);
    let other = create(&mut fs, root, "other", NewNode::Dir);
    fs.forget(other);
    let before = free(&mut fs);
    assert_eq!(
        fs.rename(
            root,
            name("one"),
            root,
            name("TWO"),
            RenameFlags::NO_REPLACE
        )
        .unwrap_err()
        .kind(),
        ErrorKind::AlreadyExists
    );
    fs.rename(root, name("one"), root, name("TWO"), RenameFlags::empty())
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
            RenameFlags::empty()
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
            RenameFlags::empty()
        )
        .unwrap_err()
        .kind(),
        ErrorKind::NotADirectory
    );
    assert_eq!(
        fs.rename(
            root,
            name("other"),
            root,
            name("full"),
            RenameFlags::empty()
        )
        .unwrap_err()
        .kind(),
        ErrorKind::DirectoryNotEmpty
    );
    fs.rename(
        root,
        name("other"),
        root,
        name("empty"),
        RenameFlags::empty(),
    )
    .unwrap();
    assert_eq!(
        fs.node_metadata(empty).unwrap_err().kind(),
        ErrorKind::NotFound
    );
    fs.forget(empty);
    let three = fs.resolve("/three").unwrap();
    fs.open_node(three).unwrap();
    assert_eq!(
        fs.rename(root, name("TWO"), root, name("three"), RenameFlags::empty())
            .unwrap_err()
            .kind(),
        ErrorKind::Busy
    );
    assert_eq!(
        fs.remove(root, name("three"), RemoveKind::File)
            .unwrap_err()
            .kind(),
        ErrorKind::Busy
    );
    fs.close_node(three);
    fs.forget(three);
    assert_eq!(
        fs.rename(
            root,
            name("TWO"),
            root,
            name("bad:name"),
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
    fs.forget(full);
    fs.sync().unwrap();
    clean(&mut fs, "replaced");
    fsck(&common::image(fs), "replaced");
}

#[test]
fn rename_keeps_benign_secondary_entries() {
    let mut fs = common::small(4 << 20, 4096);
    let root = fs.root();
    let node = common::write(&mut fs, root, "vendor", b"data");
    fs.forget(node);
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
        RenameFlags::empty(),
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
        fs.forget(node);
    }
    fs.sync().unwrap();
    let mut image = common::image(fs);
    let geo = Geometry::of(&image);
    vendor_allocation(&mut image, &geo, "vendor", "blob1", false);
    vendor_allocation(&mut image, &geo, "target", "blob2", true);
    let mut fs = common::mount(&image);
    clean(&mut fs, "vendor allocations");
    assert_eq!(free(&mut fs), before - 7);
    fs.remove(root, name("vendor"), RemoveKind::File).unwrap();
    assert_eq!(free(&mut fs), before - 4);
    fs.rename(
        root,
        name("other"),
        root,
        name("target"),
        RenameFlags::empty(),
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
    let dir = create(&mut fs, root, "dir", NewNode::Dir);
    let file = common::write(&mut fs, dir, "file", &common::payload(10_000, 3));
    assert_eq!(free(&mut fs), before - 1 - 3);
    assert_eq!(
        fs.remove(root, name("dir"), RemoveKind::Any)
            .unwrap_err()
            .kind(),
        ErrorKind::DirectoryNotEmpty
    );
    assert_eq!(
        fs.remove(root, name("dir"), RemoveKind::File)
            .unwrap_err()
            .kind(),
        ErrorKind::IsADirectory
    );
    assert_eq!(
        fs.remove(dir, name("file"), RemoveKind::Dir)
            .unwrap_err()
            .kind(),
        ErrorKind::NotADirectory
    );
    fs.remove(dir, name("FILE"), RemoveKind::File).unwrap();
    assert_eq!(
        fs.read_at(file, 0, &mut [0; 4]).unwrap_err().kind(),
        ErrorKind::NotFound
    );
    fs.forget(file);
    fs.forget(dir);
    fs.remove(root, name("dir"), RemoveKind::Dir).unwrap();
    assert_eq!(free(&mut fs), before);
    assert_eq!(
        fs.remove(root, name("dir"), RemoveKind::Any)
            .unwrap_err()
            .kind(),
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
    fs.set_len(node, 5000).unwrap();
    assert_eq!(free(&mut fs), before - 2);
    fs.set_len(node, 9000).unwrap();
    let data = read_all(&mut fs, node);
    assert_eq!(&data[..5000], &common::payload(20_000, 1)[..5000]);
    assert!(data[5000..].iter().all(|&b| b == 0));
    fs.set_len(node, 0).unwrap();
    assert_eq!(free(&mut fs), before);
    fs.forget(node);
    fs.sync().unwrap();
    clean(&mut fs, "set_len");
    fsck(&common::image(fs), "set_len");
}

#[test]
fn no_space_changes_nothing_and_space_is_reclaimed() {
    let mut fs = common::small(2 << 20, 4096);
    let root = fs.root();
    let total = free(&mut fs);
    let node = common::write(&mut fs, root, "fill", &[]);
    let huge = vec![5u8; (total as usize + 1) * 4096];
    assert_eq!(
        fs.write_at(node, 0, &huge).unwrap_err().kind(),
        ErrorKind::NoSpace
    );
    assert_eq!(free(&mut fs), total);
    assert_eq!(fs.node_metadata(node).unwrap().len(), 0);
    fs.write_at(node, 0, &huge[..total as usize * 4096])
        .unwrap();
    assert_eq!(free(&mut fs), 0);
    assert_eq!(
        fs.write_at(node, total * 4096, b"x").unwrap_err().kind(),
        ErrorKind::NoSpace
    );
    assert_eq!(
        fs.create(root, name("dir"), NewNode::Dir, &SetMetadata::new())
            .unwrap_err()
            .kind(),
        ErrorKind::NoSpace
    );
    fs.forget(node);
    fs.sync().unwrap();
    clean(&mut fs, "full");
    fs.remove(root, name("fill"), RemoveKind::File).unwrap();
    assert_eq!(free(&mut fs), total);
    let again = common::write(&mut fs, root, "again", &huge[..total as usize * 4096]);
    fs.forget(again);
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
    fs.forget(node);
    let dirty = fs.into_inner().into_inner();
    assert_eq!(dirty[106] & 2, 2, "the first write sets VolumeDirty");
    let mut fs = common::mount(&dirty);
    fs.sync().unwrap();
    let still = common::image(fs);
    assert_eq!(still[106] & 2, 2, "a volume dirty at mount stays dirty");
    let mut fs = common::mount(&image);
    let node = common::write(&mut fs, root, "f", &common::payload(1_000_000, 1));
    fs.forget(node);
    fs.sync().unwrap();
    let total = fs.stats().unwrap().total_blocks();
    let used = total - fs.stats().unwrap().free_blocks();
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
            Some(0) => Err(WriteError::Device(OutOfRange)),
            Some(left) => {
                *left -= 1;
                self.inner.write_blocks(first, buf)
            }
            None => self.inner.write_blocks(first, buf),
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
        fs.forget(node);
    }
    let dir = create(&mut fs, root, "Nested Dir", NewNode::Dir);
    for i in 0..20 {
        let node = create(&mut fs, dir, &format!("child {i}"), NewNode::File);
        fs.forget(node);
    }
    fs.forget(dir);
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
    let mut fs = ExFatFs::open_with(dev, MountOptions::new().with_table(HeapTable::new())).unwrap();
    assert!(FsDriver::capabilities(&fs).is_writable());
    let root = fs.root();
    let file = fs.lookup(root, name("lower.txt")).unwrap();
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
        fs.set_metadata(file, &meta.with_attributes(Attributes::HIDDEN)),
        fs.set_label(None),
    ]
    .map(|result| result.unwrap_err().kind());
    assert_eq!(kinds, [ErrorKind::ReadOnly; 6]);
    assert_eq!(read_all(&mut fs, file), common::payload(10, 4));
    fs.sync().unwrap();
    fs.forget(file);
    assert_eq!(fs.into_inner().inner.into_inner(), before);

    let dev = common::device(before.clone(), 512);
    let mut fs = ExFatFs::open_with(dev, MountOptions::new().with_read_only()).unwrap();
    assert_eq!(fs.set_label(None).unwrap_err().kind(), ErrorKind::ReadOnly);
    assert_eq!(fs.into_inner().into_inner(), before);
}

/// Every operation interrupted after each of its writes leaves a volume
/// that mounts and reads, and whose only findings are the leftovers an
/// interruption may leave.
#[test]
fn interrupted_operations_leave_readable_volumes() {
    let before = populated();
    let meta = SetMetadata::new();
    for op in 0..7 {
        for budget in 0..60 {
            let dev = Faulty {
                inner: common::device(before.clone(), 512),
                budget: Some(budget),
                refuse: false,
            };
            let mut fs =
                ExFatFs::open_with(dev, MountOptions::new().with_table(HeapTable::new())).unwrap();
            let root = fs.root();
            let result = match op {
                0 => fs
                    .create(root, name("a new directory"), NewNode::Dir, &meta)
                    .map(|_| ()),
                1 => fs.remove(root, name("a long file name.txt"), RemoveKind::Any),
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
                    name("grown.bin"),
                    RenameFlags::empty(),
                ),
                4 => fs
                    .resolve("/grown.bin")
                    .and_then(|node| fs.write_at(node, 8_000, &[3u8; 20_000]).map(|_| ())),
                5 => fs
                    .resolve("/a long file name.txt")
                    .and_then(|node| fs.set_len(node, 100)),
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
    type Error = OutOfRange;
}

impl BlockDevice for Shared {
    fn block_size(&self) -> BlockSize {
        BlockSize::new(512).unwrap()
    }

    fn block_count(&self) -> u64 {
        self.0.borrow().len() as u64 / 512
    }

    fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), OutOfRange> {
        let at = first.get() as usize * 512;
        buf.copy_from_slice(&self.0.borrow()[at..at + buf.len()]);
        Ok(())
    }

    fn write_blocks(
        &mut self,
        first: BlockIndex,
        buf: &[u8],
    ) -> Result<(), WriteError<OutOfRange>> {
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
    let mut fs = ExFatFs::open_with(
        Shared(image.clone()),
        MountOptions::new().with_table(HeapTable::new()),
    )
    .unwrap();
    let root = fs.root();
    let texts = ["first.bin", "second.bin", "third.bin", "fourth.bin"];
    let nodes: Vec<NodeId> = texts
        .iter()
        .map(|text| {
            fs.create(root, name(text), NewNode::File, &SetMetadata::new())
                .unwrap()
        })
        .collect();
    for &node in &nodes {
        fs.write_at(node, 0, &[7u8; 100]).unwrap();
        fs.write_at(node, 100, &[7u8; 4900]).unwrap();
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
        let sub = create(&mut fs, root, "sub", NewNode::Dir);
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
                        NewNode::Dir
                    } else {
                        NewNode::File
                    };
                    let result = fs.create(dirs[d], name(text), kind, &SetMetadata::new());
                    match model.entry(key) {
                        Entry::Occupied(_) => {
                            assert_eq!(
                                result.unwrap_err().kind(),
                                ErrorKind::AlreadyExists,
                                "{context}"
                            )
                        }
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
                    common::append(&mut fs, node, offset, &bytes);
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
                clean(&mut fs, &context);
            }
            check_model(&mut fs, &dirs, &model, &context);
        }
        fs.forget(sub);
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
            Model::Dir => assert_eq!(fs.node_metadata(node).unwrap().file_type(), FileType::Dir),
        }
        fs.forget(node);
    }
}

#[test]
fn unpinned_ids_write_through() {
    let mut fs = common::small(4 << 20, 4096);
    let root = fs.root();
    let node = common::write(&mut fs, root, "f", b"abc");
    fs.forget(node);
    let mut cursor = DirCursor::start();
    let mut buf = NameBuf::new();
    let entry = fs
        .read_dir_entry(root, &mut cursor, &mut buf)
        .unwrap()
        .unwrap();
    fs.write_at(entry.node(), 3, b"def").unwrap();
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
        fs.forget(node);
    }
    let doomed = fs.resolve(&format!("/dir {tag}/{tag} 7")).unwrap();
    fs.remove(
        dir,
        Name::new(&format!("{tag} 7")).unwrap(),
        RemoveKind::File,
    )
    .unwrap();
    fs.forget(doomed);
    fs.forget(dir);
    fs.sync().unwrap();
}

/// exfatprogs refuses a FAT count of 2, so only macOS `fsck_exfat` checks
/// the image.
#[test]
fn texfat_keeps_both_fats_and_bitmaps() {
    let options = hadris_fat::exfat::FormatOptions::new()
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
    let options = hadris_fat::exfat::FormatOptions::new().with_fat_count(2);
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
        ExFatFs::open(common::device(image, 512))
            .unwrap_err()
            .error()
            .kind(),
        ErrorKind::Corrupt,
        "the active FAT has no bitmap"
    );
}
