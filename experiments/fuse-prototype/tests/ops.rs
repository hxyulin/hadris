//! The FUSE request sequences the kernel sends, replayed against the adapter
//! without a mount.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use fuser::{Errno, INodeNo};
use hadris_fat::sync::{FatFs, format};
use hadris_fat::{FatKind, FormatOptions, MountOptions};
use hadris_fs::sync::{FileSystem, StdMutex, Volume};
use hadris_fs::{ErrorKind, HeapTable, Name, NoClock};
use hadris_fuse_prototype::Adapter;

type Fs = Volume<FatFs<File, HeapTable, NoClock>, StdMutex>;

const ROOT: u64 = INodeNo::ROOT.0;
const RDWR: i32 = libc::O_RDWR;

fn image() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let path = std::env::temp_dir().join(format!(
        "hadris-fuse-ops-{}-{}.img",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .unwrap();
    file.set_len(64 << 20).unwrap();
    format(file, FormatOptions::new().with_kind(FatKind::Fat16)).unwrap();
    path
}

fn mount() -> Adapter<Fs> {
    let path = image();
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    let _ = std::fs::remove_file(&path);
    let fs = FatFs::open_with(file, MountOptions::new().with_table(HeapTable::new())).unwrap();
    Adapter::new(Volume::new(fs), 0, 0)
}

fn os(s: &str) -> &OsStr {
    OsStr::new(s)
}

fn open_nodes(a: &Adapter<Fs>) -> usize {
    a.fs().lock().open_nodes()
}

fn touch(a: &Adapter<Fs>, dir: u64, name: &str, data: &[u8]) -> u64 {
    let (attr, fh) = a.create(dir, os(name), RDWR).unwrap();
    if !data.is_empty() {
        a.write(fh, 0, data).unwrap();
    }
    a.release(fh).unwrap();
    attr.ino.0
}

fn list(a: &Adapter<Fs>, dir: u64, page: usize) -> Vec<(u64, Vec<u8>)> {
    let mut out = Vec::new();
    let mut offset = 0;
    loop {
        let mut taken = 0;
        let mut last = None;
        a.readdir(dir, offset, |ino, next, _, name| {
            if taken == page {
                return true;
            }
            taken += 1;
            out.push((ino, name.to_vec()));
            last = Some(next);
            false
        })
        .unwrap();
        match last {
            Some(next) => offset = next,
            None => return out,
        }
    }
}

#[test]
fn lookup_pins_match_kernel_counts() {
    let a = mount();
    let base = open_nodes(&a);
    let ino = touch(&a, ROOT, "a.txt", b"x");
    for _ in 0..3 {
        assert_eq!(a.lookup(ROOT, os("a.txt")).unwrap().ino.0, ino);
    }
    assert_eq!(a.lookup(ROOT, os("A.TXT")).unwrap().ino.0, ino);
    assert_eq!(open_nodes(&a), base + 1);
    a.forget(ino, 5);
    assert_eq!(open_nodes(&a), base);
    assert_eq!(a.tracked_inodes(), 0);
}

#[test]
fn driver_refuses_remove_of_a_looked_up_name() {
    let a = mount();
    let ino = touch(&a, ROOT, "a.txt", b"x");
    let err = a
        .fs()
        .remove(a.fs().root(), Name::new("a.txt").unwrap())
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Busy);
    a.unlink(ROOT, os("a.txt")).unwrap();
    assert_eq!(a.lookup(ROOT, os("a.txt")).unwrap_err(), Errno::ENOENT);
    a.forget(ino, 1);
    assert_eq!(a.tracked_inodes(), 0);
}

#[test]
fn unlink_of_an_open_file_is_busy() {
    let a = mount();
    let (attr, fh) = a.create(ROOT, os("open.txt"), RDWR).unwrap();
    assert_eq!(a.unlink(ROOT, os("open.txt")).unwrap_err(), Errno::EBUSY);
    a.release(fh).unwrap();
    a.unlink(ROOT, os("open.txt")).unwrap();
    a.forget(attr.ino.0, 1);
}

#[test]
fn unlink_and_rmdir_check_the_type() {
    let a = mount();
    let dir = a.mkdir(ROOT, os("d")).unwrap().ino.0;
    touch(&a, dir, "f", b"1");
    assert_eq!(a.unlink(ROOT, os("d")).unwrap_err(), Errno::EISDIR);
    assert_eq!(a.rmdir(ROOT, os("d")).unwrap_err(), Errno::ENOTEMPTY);
    assert_eq!(a.rmdir(dir, os("f")).unwrap_err(), Errno::ENOTDIR);
    a.unlink(dir, os("f")).unwrap();
    a.rmdir(ROOT, os("d")).unwrap();
}

#[test]
fn rename_replaces_a_looked_up_target() {
    let a = mount();
    let base = open_nodes(&a);
    let src = touch(&a, ROOT, "new", b"new data");
    let dst = touch(&a, ROOT, "old", b"old");
    a.rename(ROOT, os("new"), ROOT, os("old"), 0).unwrap();
    let attr = a.lookup(ROOT, os("old")).unwrap();
    assert_eq!(attr.ino.0, src, "the moved node keeps its inode number");
    assert_eq!(attr.size, 8);
    a.forget(src, 2);
    a.forget(dst, 1);
    assert_eq!(open_nodes(&a), base);
}

#[test]
fn rename_flags() {
    let a = mount();
    touch(&a, ROOT, "a", b"a");
    touch(&a, ROOT, "b", b"b");
    assert_eq!(
        a.rename(ROOT, os("a"), ROOT, os("b"), 1).unwrap_err(),
        Errno::EEXIST
    );
    assert_eq!(
        a.rename(ROOT, os("a"), ROOT, os("b"), 2).unwrap_err(),
        Errno::EINVAL
    );
    a.rename(ROOT, os("a"), ROOT, os("c"), 1).unwrap();
    a.rename(ROOT, os("c"), ROOT, os("C"), 0).unwrap();
    let names: Vec<_> = list(&a, ROOT, 100).into_iter().map(|e| e.1).collect();
    assert!(names.contains(&b"C".to_vec()), "{names:?}");
}

#[test]
fn rename_over_an_open_target_is_busy() {
    let a = mount();
    touch(&a, ROOT, "a", b"a");
    let (_, fh) = a.create(ROOT, os("b"), RDWR).unwrap();
    assert_eq!(
        a.rename(ROOT, os("a"), ROOT, os("b"), 0).unwrap_err(),
        Errno::EBUSY
    );
    a.release(fh).unwrap();
}

#[test]
fn readdir_pages_resume_from_offsets() {
    let a = mount();
    let dir = a.mkdir(ROOT, os("big")).unwrap().ino.0;
    for i in 0..300 {
        touch(&a, dir, &format!("file-with-a-long-name-{i:04}"), b"");
    }
    let entries = list(&a, dir, 7);
    assert_eq!(entries.len(), 302);
    let names: BTreeSet<_> = entries.iter().map(|e| e.1.clone()).collect();
    assert_eq!(names.len(), 302, "no entry is listed twice");
}

#[test]
fn readdir_survives_changes_between_pages() {
    let a = mount();
    let dir = a.mkdir(ROOT, os("d")).unwrap().ino.0;
    for i in 0..50 {
        touch(&a, dir, &format!("f{i:02}"), b"");
    }
    let mut seen = Vec::new();
    let mut offset = 0;
    let mut round = 0;
    loop {
        let mut last = None;
        let mut taken = 0;
        a.readdir(dir, offset, |_, next, _, name| {
            if taken == 5 {
                return true;
            }
            taken += 1;
            seen.push(String::from_utf8(name.to_vec()).unwrap());
            last = Some(next);
            false
        })
        .unwrap();
        let Some(next) = last else { break };
        offset = next;
        round += 1;
        touch(&a, dir, &format!("new{round:02}"), b"");
        let _ = a.unlink(dir, os(&format!("f{:02}", 49 - round)));
    }
    let unique: BTreeSet<_> = seen.iter().collect();
    assert_eq!(unique.len(), seen.len(), "duplicates: {seen:?}");
    for i in 0..10 {
        assert!(
            seen.contains(&format!("f{i:02}")),
            "f{i:02} unchanged but missed"
        );
    }
}

#[test]
fn readdir_and_lookup_can_disagree_on_the_inode_number() {
    let a = mount();
    let a_ino = touch(&a, ROOT, "a", b"a");
    a.rename(ROOT, os("a"), ROOT, os("b"), 0).unwrap();
    let c_ino = touch(&a, ROOT, "c", b"c");
    a.forget(c_ino, 1);
    let listed = list(&a, ROOT, 100)
        .into_iter()
        .find(|e| e.1 == b"c")
        .unwrap()
        .0;
    let looked_up = a.lookup(ROOT, os("c")).unwrap().ino.0;
    eprintln!("a/b pinned as {a_ino:#x}; c listed as {listed:#x}, looked up as {looked_up:#x}");
    assert_ne!(listed, looked_up);
}

#[test]
fn size_is_visible_before_flush_and_mtime_after() {
    let a = mount();
    let (attr, fh) = a.create(ROOT, os("w"), RDWR).unwrap();
    let ino = attr.ino.0;
    a.write(fh, 0, &[7u8; 5000]).unwrap();
    assert_eq!(a.getattr(ino).unwrap().size, 5000);
    assert_eq!(a.read(fh, 4990, 100).unwrap(), vec![7u8; 10]);
    a.flush(fh).unwrap();
    a.release(fh).unwrap();
    let t = a
        .setattr(ino, Some(10), None, Some(fuser::TimeOrNow::Now))
        .unwrap();
    assert_eq!(t.size, 10);
}

#[test]
fn append_writes_go_to_the_end() {
    let a = mount();
    let (attr, fh) = a.create(ROOT, os("log"), RDWR).unwrap();
    a.write(fh, 0, b"one\n").unwrap();
    a.release(fh).unwrap();
    let fh = a.open(attr.ino.0, libc::O_WRONLY | libc::O_APPEND).unwrap();
    a.write(fh, 0, b"two\n").unwrap();
    a.release(fh).unwrap();
    let fh = a.open(attr.ino.0, libc::O_RDONLY).unwrap();
    assert_eq!(a.read(fh, 0, 100).unwrap(), b"one\ntwo\n");
    a.release(fh).unwrap();
}

#[test]
fn file_size_limit_maps_to_efbig() {
    let a = mount();
    let (_, fh) = a.create(ROOT, os("big"), RDWR).unwrap();
    assert_eq!(
        a.write(fh, u32::MAX as u64, b"x").unwrap_err(),
        Errno::EFBIG
    );
    a.release(fh).unwrap();
}

#[test]
fn unsupported_node_kinds_are_eperm() {
    let a = mount();
    let err = a
        .make(ROOT, os("link"), hadris_fs::NewNode::Symlink(b"target"))
        .unwrap_err();
    assert_eq!(err, Errno::EPERM);
}

#[test]
fn statfs_reports_clusters() {
    let a = mount();
    let s = a.statfs().unwrap();
    assert!(s.blocks > 0 && s.bfree <= s.blocks);
    assert_eq!(s.namelen, 255);
}
