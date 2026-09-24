//! `FatFs` at scale: many names in one directory, multi-cluster reads and
//! writes over fragmented free space, listings of growing directories and
//! many pinned nodes. These check results, not timing.

#[path = "common/fatfs.rs"]
mod common;

use std::collections::BTreeSet;

use common::{CASES, Case, Device, Fs, block_on, formatted, payload};
use hadris_fat::FormatOptions;
use hadris_fat::sync::FatFs;
use hadris_fs::{
    DirCursor, ErrorKind, Name, NameBuf, NewNode, NodeId, NodeTable, RemoveKind, RenameFlags,
    SetMetadata,
};

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

/// Checks the volume `fs` has written, and mounts it again.
fn assert_clean(fs: &mut Fs, what: &str) {
    fs.sync().unwrap();
    let expected = free(fs);
    let placeholder = common::mount(CASES[0], &common::blank(CASES[0]));
    let mut dev = std::mem::replace(fs, placeholder).into_inner();
    let (_, found) = common::check_dev(&mut dev, 8192);
    assert_eq!(found, [], "{what}");
    assert_eq!(u64::from(common::scan_free(&mut dev)), expected, "{what}");
    *fs = common::mount_dev(dev);
}

fn list(fs: &mut Fs, dir: NodeId) -> Vec<String> {
    let mut cursor = DirCursor::start();
    let mut buf = NameBuf::new();
    let mut out = Vec::new();
    while fs
        .read_dir_entry(dir, &mut cursor, &mut buf)
        .unwrap()
        .is_some()
    {
        out.push(buf.as_name().unwrap().to_str().unwrap().to_owned());
    }
    out
}

/// Short entries whose name starts with `prefix`.
fn short_names(image: &[u8], prefix: &[u8]) -> Vec<[u8; 11]> {
    image
        .chunks_exact(32)
        .filter(|entry| entry.starts_with(prefix) && entry[11] & 0x3F != 0x0F)
        .map(|entry| entry[..11].try_into().unwrap())
        .collect()
}

#[test]
fn names_sharing_a_prefix_get_distinct_short_names() {
    for case in [CASES[1], CASES[2]] {
        let mut fs = formatted(case, FormatOptions::new());
        let root = fs.root();
        let dir = create(&mut fs, root, "reports", NewNode::Dir);
        let count = 40;
        let mut nodes = BTreeSet::new();
        for i in 0..count {
            let node = create(
                &mut fs,
                dir,
                &format!("Quarterly report {i:02}.txt"),
                NewNode::File,
            );
            assert!(nodes.insert(node));
            fs.forget(node);
        }
        for i in 0..count {
            let text = format!("QUARTERLY REPORT {i:02}.TXT");
            let err = fs
                .create(dir, name(&text), NewNode::File, &SetMetadata::new())
                .unwrap_err();
            assert_eq!(
                err.kind(),
                ErrorKind::AlreadyExists,
                "{}: {text}",
                case.name
            );
        }
        for tail in 1..=4 {
            let alias = format!("QUARTE~{tail}.TXT");
            let node = fs.lookup(dir, name(&alias)).unwrap();
            let long = fs
                .lookup(dir, name(&format!("quarterly report {:02}.txt", tail - 1)))
                .unwrap();
            assert_eq!(node, long, "{}: {alias}", case.name);
            fs.forget(node);
            fs.forget(long);
            let err = fs
                .create(dir, name(&alias), NewNode::File, &SetMetadata::new())
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::AlreadyExists);
        }
        assert_eq!(list(&mut fs, dir).len(), count);
        assert_clean(&mut fs, case.name);
        fs.forget(dir);
        let image = fs.into_inner().into_inner();
        let shorts = short_names(&image, b"QU");
        assert_eq!(shorts.len(), count, "{}", case.name);
        assert_eq!(
            shorts.iter().collect::<BTreeSet<_>>().len(),
            count,
            "{}: short names repeat",
            case.name
        );
        common::fsck(&image, case.name);
    }
}

/// Leaves `interleaved.bin` in runs of three clusters with one-cluster holes
/// between them, fills the rest of the volume with `filler.bin`, and
/// returns the contents of `interleaved.bin`. The next allocation wraps
/// to the start and takes the holes.
fn fragment(fs: &mut Fs, cluster: usize) -> Vec<u8> {
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

fn cluster_size(case: Case) -> usize {
    formatted(case, FormatOptions::new())
        .stats()
        .unwrap()
        .block_size() as usize
}

#[test]
fn multi_cluster_io_over_fragmented_free_space() {
    for case in CASES {
        let cluster = cluster_size(case);
        let size = 30 * cluster;
        let mut small = formatted(case, FormatOptions::new());
        let runs = fragment(&mut small, cluster);
        assert_clean(&mut small, case.name);
        let root = small.root();
        let node = small.lookup(root, name("interleaved.bin")).unwrap();
        assert_eq!(read(&mut small, node, size), runs, "{}", case.name);
        let mut odd = vec![0u8; 5 * cluster + 7];
        let at = cluster as u64 / 2;
        assert_eq!(small.read_at(node, at, &mut odd).unwrap(), odd.len());
        assert_eq!(odd[..], runs[at as usize..at as usize + odd.len()]);
        let patch = payload(7 * cluster, 3);
        write(&mut small, node, 2 * cluster as u64 + 9, &patch);
        let mut expected = runs.clone();
        expected[2 * cluster + 9..9 * cluster + 9].copy_from_slice(&patch);
        assert_eq!(read(&mut small, node, size), expected, "{}", case.name);
        small.forget(node);

        let data = payload(10 * cluster, 9);
        let node = create(&mut small, root, "big.bin", NewNode::File);
        write(&mut small, node, 0, &data);
        assert_eq!(free(&mut small), 0, "{}", case.name);
        let chain = common::chain(&mut small, node);
        assert_eq!(chain.len(), 10, "{}", case.name);
        assert!(
            chain.windows(2).all(|pair| pair[1] > pair[0] + 1),
            "{}: the chain must take the holes",
            case.name
        );
        assert_eq!(read(&mut small, node, data.len()), data, "{}", case.name);
        small.forget(node);
        assert_clean(&mut small, case.name);

        let image = small.into_inner().into_inner();
        let mut fs = common::mount(case, &image);
        let node = fs.lookup(fs.root(), name("big.bin")).unwrap();
        assert_eq!(
            read(&mut fs, node, data.len()),
            data,
            "{}: remounted",
            case.name
        );
        fs.forget(node);
        let root = fs.root();
        fs.remove(root, name("big.bin"), RemoveKind::File).unwrap();
        fs.remove(root, name("filler.bin"), RemoveKind::File)
            .unwrap();
        assert_eq!(
            free(&mut fs),
            free(&mut formatted(case, FormatOptions::new())) - 30,
            "{}",
            case.name
        );
        assert_clean(&mut fs, case.name);
        common::fsck(&fs.into_inner().into_inner(), case.name);
    }
}

#[test]
fn allocation_wraps_to_the_start_and_fails_cleanly_when_full() {
    let case = CASES[1];
    let cluster = cluster_size(case);
    let mut fs = formatted(case, FormatOptions::new());
    let root = fs.root();
    let total = free(&mut fs) as usize;
    let first = create(&mut fs, root, "first.bin", NewNode::File);
    write(&mut fs, first, 0, &payload(total / 2 * cluster, 1));
    let second = create(&mut fs, root, "second.bin", NewNode::File);
    write(&mut fs, second, 0, &payload(total / 4 * cluster, 2));
    fs.forget(first);
    fs.remove(root, name("first.bin"), RemoveKind::File)
        .unwrap();
    let left = free(&mut fs) as usize;
    let wrap = create(&mut fs, root, "wrap.bin", NewNode::File);
    let data = payload((left - 8) * cluster, 3);
    write(&mut fs, wrap, 0, &data);
    let chain = common::chain(&mut fs, wrap);
    assert!(
        chain.windows(2).any(|pair| pair[1] < pair[0]),
        "the chain must wrap"
    );
    assert_eq!(read(&mut fs, wrap, data.len()), data);
    assert_clean(&mut fs, "wrapped");

    let before = free(&mut fs);
    let err = fs
        .write_at(
            second,
            (total / 4 * cluster) as u64,
            &payload(9 * cluster, 4),
        )
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NoSpace);
    assert_eq!(free(&mut fs), before);
    assert_eq!(
        fs.node_metadata(second).unwrap().len(),
        (total / 4 * cluster) as u64
    );
    assert_clean(&mut fs, "full");
    fs.forget(second);
    fs.forget(wrap);
}

#[test]
fn async_multi_cluster_io_matches_sync() {
    use hadris_fat::r#async::FatFs;
    use hadris_fs::HeapTable;

    let case = CASES[2];
    let cluster = cluster_size(case);
    let data = payload(20 * cluster + 1, 5);
    let mut sync_fs = formatted(case, FormatOptions::new());
    let root = sync_fs.root();
    fragment(&mut sync_fs, cluster);
    sync_fs
        .remove(root, name("filler.bin"), RemoveKind::File)
        .unwrap();
    let image = sync_fs.into_inner().into_inner();
    let mut sync_fs = common::mount(case, &image);
    let root = sync_fs.root();
    let node = create(&mut sync_fs, root, "x.bin", NewNode::File);
    write(&mut sync_fs, node, 0, &data);
    sync_fs.forget(node);
    sync_fs.sync().unwrap();
    let expected = sync_fs.into_inner().into_inner();

    let image = block_on(async {
        let dev = common::device(case, image);
        let options = hadris_fat::MountOptions::new().with_table(HeapTable::new());
        let mut fs = FatFs::open_with(dev, options).await.unwrap();
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
    for case in [CASES[0], CASES[2]] {
        let mut fs = formatted(case, FormatOptions::new());
        let root = fs.root();
        for dir in [None, Some("sub")] {
            let dir = match dir {
                Some(text) => create(&mut fs, root, text, NewNode::Dir),
                None => root,
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
                seen.push(buf.as_name().unwrap().to_str().unwrap().to_owned());
                if added < 30 {
                    let text = format!("during {added:03} long");
                    let node = create(&mut fs, dir, &text, NewNode::File);
                    fs.forget(node);
                    expected.insert(text);
                    added += 1;
                }
            }
            let unique: BTreeSet<_> = seen.iter().cloned().collect();
            assert_eq!(unique.len(), seen.len(), "{}: a name repeats", case.name);
            assert_eq!(unique, expected, "{}", case.name);
            let mut again = DirCursor::from_raw(0);
            fs.read_dir_entry(dir, &mut again, &mut buf)
                .unwrap()
                .unwrap();
            assert_eq!(buf.as_name().unwrap().to_str().unwrap(), seen[0]);
            if dir != root {
                fs.forget(dir);
            }
        }
        assert_clean(&mut fs, case.name);
    }
}

fn pins_follow_renames_and_removals<T: NodeTable>(mut fs: FatFs<Device, T>, count: usize) {
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
    let (_, found) = common::check_dev(&mut fs.into_inner(), 8192);
    assert_eq!(found, []);
}

#[test]
fn pinned_lookups_follow_renames_and_removals() {
    let case = CASES[2];
    let image = common::blank(case);
    pins_follow_renames_and_removals(common::mount(case, &image), 300);
    let fixed = FatFs::open(common::device(case, image)).unwrap();
    pins_follow_renames_and_removals(fixed, 40);
}
