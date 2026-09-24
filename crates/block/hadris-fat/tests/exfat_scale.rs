//! `ExFatFs` at scale: names with equal hashes, multi-cluster reads and
//! writes over fragmented free space, listings of growing directories and
//! many pinned nodes. These check results, not timing.

#[path = "common/exfat.rs"]
mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::{Device, Fs, block_on, clean, payload, small};
use hadris_fat::exfat::sync::ExFatFs;
use hadris_fs::{
    DirCursor, ErrorKind, Name, NameBuf, NewNode, NodeId, NodeTable, RemoveKind, RenameFlags,
    SetMetadata,
};

const SIZE: usize = 32 << 20;
const CLUSTER: usize = 4096;
const SMALL: usize = 8 << 20;

fn name(text: &str) -> &Name {
    Name::new(text).unwrap()
}

fn create(fs: &mut Fs, dir: NodeId, text: &str, kind: NewNode<'_>) -> NodeId {
    fs.create(dir, name(text), kind, &SetMetadata::new())
        .unwrap()
}

fn write(fs: &mut Fs, node: NodeId, mut at: u64, mut data: &[u8]) {
    while !data.is_empty() {
        let n = fs.write_at(node, at, data).unwrap();
        at += n as u64;
        data = &data[n..];
    }
}

fn read(fs: &mut Fs, node: NodeId, len: usize) -> Vec<u8> {
    let mut out = vec![0u8; len];
    let mut done = 0;
    while done < len {
        let n = fs.read_at(node, done as u64, &mut out[done..]).unwrap();
        assert!(n > 0);
        done += n;
    }
    out
}

fn free(fs: &mut Fs) -> u64 {
    fs.stats().unwrap().free_blocks()
}

fn settle(fs: &mut Fs, what: &str) {
    fs.sync().unwrap();
    clean(fs, what);
}

/// The exFAT `NameHash` of an ASCII name, which up-cases as ASCII does.
fn name_hash(text: &str) -> u16 {
    let mut hash = 0u16;
    for unit in text.to_ascii_uppercase().encode_utf16() {
        for byte in unit.to_le_bytes() {
            hash = hash.rotate_right(1).wrapping_add(byte as u16);
        }
    }
    hash
}

#[test]
fn names_with_equal_hashes_stay_apart() {
    let mut seen = BTreeMap::new();
    let (a, b) = (0..100_000)
        .map(|i| format!("n{i:05}"))
        .find_map(|text| {
            let hash = name_hash(&text);
            seen.insert(hash, text.clone()).map(|other| (other, text))
        })
        .unwrap();
    assert_eq!(name_hash(&a), name_hash(&b));
    let mut fs = small(SIZE, CLUSTER as u32);
    let root = fs.root();
    for i in 0..50 {
        let node = create(&mut fs, root, &format!("filler {i}"), NewNode::File);
        fs.forget(node);
    }
    let first = create(&mut fs, root, &a, NewNode::File);
    write(&mut fs, first, 0, b"first");
    let second = create(&mut fs, root, &b, NewNode::File);
    write(&mut fs, second, 0, b"second");
    fs.forget(first);
    fs.forget(second);
    for (text, body) in [(&a, &b"first"[..]), (&b, b"second")] {
        let node = fs.lookup(root, name(&text.to_uppercase())).unwrap();
        assert_eq!(read(&mut fs, node, body.len()), body);
        fs.forget(node);
        let err = fs
            .create(
                root,
                name(&text.to_uppercase()),
                NewNode::File,
                &SetMetadata::new(),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::AlreadyExists);
    }
    assert_eq!(
        fs.lookup(root, name("n99999x")).unwrap_err().kind(),
        ErrorKind::NotFound
    );
    settle(&mut fs, "hash collision");
}

#[test]
fn many_names_in_one_directory() {
    let mut fs = small(SIZE, CLUSTER as u32);
    let root = fs.root();
    let dir = create(&mut fs, root, "many", NewNode::Dir);
    let count = 600;
    for i in 0..count {
        let node = create(&mut fs, dir, &format!("file_{i:05}.txt"), NewNode::File);
        fs.forget(node);
    }
    for i in (0..count).step_by(37) {
        let err = fs
            .create(
                dir,
                name(&format!("FILE_{i:05}.TXT")),
                NewNode::File,
                &SetMetadata::new(),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::AlreadyExists);
        let node = fs.lookup(dir, name(&format!("File_{i:05}.Txt"))).unwrap();
        fs.forget(node);
    }
    fs.forget(dir);
    assert_eq!(common::names(&mut fs, "/many").len(), count);
    settle(&mut fs, "many names");
}

/// Leaves `interleaved.bin` in runs of three clusters with one-cluster holes
/// between them, fills the rest of the volume with `filler.bin`, and
/// returns the contents of `interleaved.bin`. The next allocation wraps
/// to the start and takes the holes.
fn fragment(fs: &mut Fs) -> Vec<u8> {
    let cluster = CLUSTER;
    let root = fs.root();
    let runs = create(fs, root, "interleaved.bin", NewNode::File);
    let gaps = create(fs, root, "gaps.bin", NewNode::File);
    let data = payload(30 * cluster, 7);
    for i in 0..10 {
        let at = 3 * i * cluster;
        write(fs, runs, at as u64, &data[at..at + 3 * cluster]);
        write(fs, gaps, (i * cluster) as u64, &payload(cluster, 1));
    }
    fs.forget(runs);
    fs.forget(gaps);
    fs.remove(root, name("gaps.bin"), RemoveKind::File).unwrap();
    let tail = free(fs) as usize - 10;
    let filler = create(fs, root, "filler.bin", NewNode::File);
    write(fs, filler, 0, &payload(tail * cluster, 2));
    fs.forget(filler);
    assert_eq!(free(fs), 10);
    data
}

#[test]
fn multi_cluster_io_over_fragmented_free_space() {
    let cluster = CLUSTER;
    let size = 30 * cluster;
    let mut fs = small(SMALL, CLUSTER as u32);
    let runs = fragment(&mut fs);
    settle(&mut fs, "fragmented");
    let root = fs.root();
    let node = fs.lookup(root, name("interleaved.bin")).unwrap();
    assert_eq!(read(&mut fs, node, size), runs);
    let mut odd = vec![0u8; 5 * cluster + 7];
    let at = cluster as u64 / 2;
    assert_eq!(fs.read_at(node, at, &mut odd).unwrap(), odd.len());
    assert_eq!(odd[..], runs[at as usize..at as usize + odd.len()]);
    let patch = payload(7 * cluster, 3);
    write(&mut fs, node, 2 * cluster as u64 + 9, &patch);
    let mut expected = runs.clone();
    expected[2 * cluster + 9..9 * cluster + 9].copy_from_slice(&patch);
    assert_eq!(read(&mut fs, node, size), expected);
    fs.forget(node);

    let data = payload(10 * cluster, 9);
    let node = create(&mut fs, root, "big.bin", NewNode::File);
    write(&mut fs, node, 0, &data);
    assert_eq!(free(&mut fs), 0);
    let chain = common::chain(&mut fs, node);
    assert_eq!(chain.len(), 10);
    assert!(
        chain.windows(2).all(|pair| pair[1] > pair[0] + 1),
        "the chain must take the holes"
    );
    assert_eq!(read(&mut fs, node, data.len()), data);
    fs.forget(node);
    settle(&mut fs, "written");

    let mut fs = common::mount(&common::image(fs));
    let node = fs.lookup(fs.root(), name("big.bin")).unwrap();
    assert_eq!(read(&mut fs, node, data.len()), data);
    fs.forget(node);
    let root = fs.root();
    fs.remove(root, name("big.bin"), RemoveKind::File).unwrap();
    fs.remove(root, name("filler.bin"), RemoveKind::File)
        .unwrap();
    assert_eq!(free(&mut fs), free(&mut small(SMALL, CLUSTER as u32)) - 30);
    settle(&mut fs, "removed");
    let image = common::image(fs);
    common::fsck(&image, "multi-cluster");
}

#[test]
fn allocation_wraps_to_the_start_and_fails_cleanly_when_full() {
    let mut fs = small(8 << 20, CLUSTER as u32);
    let root = fs.root();
    let total = free(&mut fs) as usize;
    let first = create(&mut fs, root, "first.bin", NewNode::File);
    write(&mut fs, first, 0, &payload(total / 2 * CLUSTER, 1));
    let second = create(&mut fs, root, "second.bin", NewNode::File);
    write(&mut fs, second, 0, &payload(total / 4 * CLUSTER, 2));
    fs.forget(first);
    fs.remove(root, name("first.bin"), RemoveKind::File)
        .unwrap();
    let left = free(&mut fs) as usize;
    let wrap = create(&mut fs, root, "wrap.bin", NewNode::File);
    let data = payload((left - 4) * CLUSTER, 3);
    write(&mut fs, wrap, 0, &data);
    let chain = common::chain(&mut fs, wrap);
    assert!(
        chain.windows(2).any(|pair| pair[1] < pair[0]),
        "the chain must wrap"
    );
    assert_eq!(read(&mut fs, wrap, data.len()), data);
    settle(&mut fs, "wrapped");

    let before = free(&mut fs);
    let err = fs
        .write_at(
            second,
            (total / 4 * CLUSTER) as u64,
            &payload(9 * CLUSTER, 4),
        )
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NoSpace);
    assert_eq!(free(&mut fs), before);
    fs.forget(second);
    fs.forget(wrap);
    settle(&mut fs, "full");
}

#[test]
fn async_multi_cluster_io_matches_sync() {
    use hadris_fat::exfat::MountOptions;
    use hadris_fat::exfat::r#async::ExFatFs;
    use hadris_fs::HeapTable;

    let data = payload(20 * CLUSTER + 1, 5);
    let mut fs = small(SMALL, CLUSTER as u32);
    fragment(&mut fs);
    let root = fs.root();
    fs.remove(root, name("filler.bin"), RemoveKind::File)
        .unwrap();
    fs.sync().unwrap();
    let image = common::image(fs);
    let mut fs = common::mount(&image);
    let root = fs.root();
    let node = create(&mut fs, root, "x.bin", NewNode::File);
    write(&mut fs, node, 0, &data);
    fs.forget(node);
    fs.sync().unwrap();
    let expected = common::image(fs);

    let image = block_on(async {
        let dev = common::device(image, 512);
        let options = MountOptions::new().with_table(HeapTable::new());
        let mut fs = ExFatFs::open_with(dev, options).await.unwrap();
        let root = fs.root();
        let node = fs
            .create(root, name("x.bin"), NewNode::File, &SetMetadata::new())
            .await
            .unwrap();
        let mut at = 0;
        while at < data.len() {
            at += fs.write_at(node, at as u64, &data[at..]).await.unwrap();
        }
        let mut back = vec![0u8; data.len()];
        let mut done = 0;
        while done < back.len() {
            done += fs
                .read_at(node, done as u64, &mut back[done..])
                .await
                .unwrap();
        }
        assert_eq!(back, data);
        fs.forget(node);
        fs.sync().await.unwrap();
        fs.into_inner().into_inner()
    });
    assert_eq!(image, expected);
}

#[test]
fn listing_resumes_while_the_directory_grows() {
    let mut fs = small(SIZE, 512);
    let root = fs.root();
    for sub in [false, true] {
        let dir = if sub {
            create(&mut fs, root, "sub", NewNode::Dir)
        } else {
            root
        };
        let mut expected = BTreeSet::new();
        for i in 0..20 {
            let text = format!("before {i:03} long");
            let node = create(&mut fs, dir, &text, NewNode::File);
            fs.forget(node);
            expected.insert(text);
        }
        let mut cursor = DirCursor::start();
        let mut buf = NameBuf::new();
        let mut seen = Vec::new();
        let mut added = 0;
        while fs
            .read_dir_entry(dir, &mut cursor, &mut buf)
            .unwrap()
            .is_some()
        {
            let text = buf.as_name().unwrap().to_str().unwrap().to_owned();
            if text != "sub" {
                seen.push(text);
            }
            if added < 30 {
                let text = format!("during {added:03} long");
                let node = create(&mut fs, dir, &text, NewNode::File);
                fs.forget(node);
                expected.insert(text);
                added += 1;
            }
        }
        let unique: BTreeSet<_> = seen.iter().cloned().collect();
        assert_eq!(unique.len(), seen.len(), "a name repeats");
        assert_eq!(unique, expected);
        let mut again = DirCursor::from_raw(0);
        fs.read_dir_entry(dir, &mut again, &mut buf)
            .unwrap()
            .unwrap();
        if sub {
            assert_eq!(buf.as_name().unwrap().to_str().unwrap(), seen[0]);
            fs.forget(dir);
        }
    }
    settle(&mut fs, "grown");
}

fn pins_follow_renames_and_removals<T: NodeTable>(mut fs: ExFatFs<Device, T>, count: usize) {
    let root = fs.root();
    let new = SetMetadata::new();
    let dir = fs.create(root, name("pins"), NewNode::Dir, &new).unwrap();
    let mut pinned = Vec::new();
    for i in 0..count {
        let node = fs
            .create(dir, name(&format!("f{i}")), NewNode::File, &new)
            .unwrap();
        pinned.push(node);
    }
    for (i, &node) in pinned.iter().enumerate() {
        let again = fs.lookup(dir, name(&format!("f{i}"))).unwrap();
        assert_eq!(again, node);
        fs.forget(again);
    }
    fs.rename(
        dir,
        name("f7"),
        dir,
        name("renamed seven"),
        RenameFlags::empty(),
    )
    .unwrap();
    assert_eq!(fs.lookup(dir, name("renamed seven")).unwrap(), pinned[7]);
    fs.forget(pinned[7]);
    let fresh = fs.create(dir, name("f7"), NewNode::File, &new).unwrap();
    assert_ne!(fresh, pinned[7]);
    assert_eq!(fs.lookup(dir, name("f7")).unwrap(), fresh);
    fs.forget(fresh);
    fs.remove(dir, name("f8"), RemoveKind::File).unwrap();
    assert_eq!(
        fs.node_metadata(pinned[8]).unwrap_err().kind(),
        ErrorKind::NotFound
    );
    let over = fs
        .create(dir, name("f8 again"), NewNode::File, &new)
        .unwrap();
    assert_ne!(over, pinned[8]);
    for i in [0, 9, count - 1] {
        let again = fs.lookup(dir, name(&format!("f{i}"))).unwrap();
        assert_eq!(again, pinned[i]);
        fs.forget(again);
    }
    for node in pinned.into_iter().chain([fresh, over]) {
        fs.forget(node);
    }
    fs.forget(dir);
    assert_eq!(fs.open_nodes(), 1);
    let dir = fs.lookup(root, name("pins")).unwrap();
    let a = fs.lookup(dir, name("f1")).unwrap();
    let b = fs.lookup(dir, name("renamed seven")).unwrap();
    assert_ne!(a, b);
    assert_eq!(fs.lookup(dir, name("f1")).unwrap(), a);
    for node in [a, a, b, dir] {
        fs.forget(node);
    }
    assert_eq!(fs.open_nodes(), 1);
    fs.sync().unwrap();
    let (_, found) = common::check_dev(&mut fs.into_inner(), 4096);
    assert_eq!(found, []);
}

#[test]
fn pinned_lookups_follow_renames_and_removals() {
    let image = common::image(small(SIZE, CLUSTER as u32));
    pins_follow_renames_and_removals(common::mount(&image), 300);
    let fixed = ExFatFs::open(common::device(image, 512)).unwrap();
    pins_follow_renames_and_removals(fixed, 40);
}
