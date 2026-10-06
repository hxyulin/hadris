//! The shared tier: paths and handles named after `std::fs`, path policies,
//! and handles dropped while the lock is held.

#![cfg(all(feature = "sync", feature = "std"))]

mod common;

use std::io::{Read as _, Write as _};

use common::files::{exists, names, read, write};
use common::sync::{MemFs, fixture};
use hadris_fs::sync::{FileSystem, Volume};
use hadris_fs::{ErrorKind, OpenOptions, Resolve, SeekFrom, SetAttr};

fn open_nodes(vol: Volume<MemFs>) -> usize {
    vol.into_inner().unwrap().open_nodes()
}

#[test]
fn paths_and_handles() {
    let vol = Volume::new(MemFs::new());
    vol.create_dir_all("/var/log").unwrap();
    write(&vol, "/var/log/a.txt", b"alpha").unwrap();
    assert!(exists(&vol, "/var/log/a.txt"));
    assert!(!exists(&vol, "/var/log/b.txt"));
    assert_eq!(vol.metadata("/var/log/a.txt").unwrap().len(), 5);
    assert_eq!(read(&vol, "/var/log/a.txt").unwrap(), b"alpha");

    let mut log = vol
        .open(
            "/var/log/b.txt",
            OpenOptions::new().write().create().append(),
        )
        .unwrap();
    writeln!(log, "beta").unwrap();
    let mut other = vol
        .open("/var/log/b.txt", OpenOptions::new().read().write())
        .unwrap();
    other.write_all(b"B").unwrap();
    writeln!(log, "gamma").unwrap();
    log.close().unwrap();
    other.seek(SeekFrom::Start(0)).unwrap();
    let mut text = String::new();
    other.read_to_string(&mut text).unwrap();
    assert_eq!(text, "Beta\ngamma\n");
    assert_eq!(other.seek(SeekFrom::End(-1)).unwrap(), 10);
    other.set_len(4).unwrap();
    assert_eq!(other.metadata().unwrap().len(), 4);
    other.sync_all().unwrap();
    drop(other);

    let mut listed = names(&vol, "/var/log");
    listed.sort();
    assert_eq!(listed, ["a.txt", "b.txt"]);
    vol.rename("/var/log/b.txt", "/var/log/a.txt").unwrap();
    assert_eq!(read(&vol, "/var/log/a.txt").unwrap(), b"Beta");
    vol.set_attr("/var/log/a.txt", &SetAttr::new()).unwrap();
    vol.create_dir("/var/tmp").unwrap();
    assert_eq!(
        vol.create_dir("/var/tmp").unwrap_err().kind(),
        ErrorKind::AlreadyExists
    );
    vol.remove_dir("/var/tmp").unwrap();
    vol.remove_file("/var/log/a.txt").unwrap();
    vol.create_dir_all("/var/log/deep/er").unwrap();
    write(&vol, "/var/log/deep/er/c", b"c").unwrap();
    assert_eq!(
        vol.remove_dir("/var").unwrap_err().kind(),
        ErrorKind::DirectoryNotEmpty
    );
    vol.remove_dir_all("/var").unwrap();
    assert!(names(&vol, "/").is_empty());
    assert_eq!(
        vol.remove_dir_all("/").unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    assert_eq!(open_nodes(vol), 1);
}

#[test]
fn opens_check_their_options() {
    let vol = Volume::new(fixture());
    let kind = |path: &str, options: OpenOptions| vol.open(path, options).unwrap_err().kind();
    assert_eq!(kind("/a.txt", OpenOptions::new()), ErrorKind::InvalidInput);
    assert_eq!(
        kind("/a.txt", OpenOptions::new().write().create_new()),
        ErrorKind::AlreadyExists
    );
    assert_eq!(
        kind("/etc", OpenOptions::new().read()),
        ErrorKind::IsADirectory
    );
    assert_eq!(kind("/abs", OpenOptions::new().read()), ErrorKind::Symlink);
    assert_eq!(
        kind("/nope", OpenOptions::new().read()),
        ErrorKind::NotFound
    );
    let mut file = vol.open("/a.txt", OpenOptions::new().read()).unwrap();
    assert_eq!(
        file.write(b"x").unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    assert_eq!(file.seek(SeekFrom::End(-2)).unwrap(), 4);
    drop(file);
    write(&vol, "/a.txt", b"xy").unwrap();
    assert_eq!(read(&vol, "/a.txt").unwrap(), b"xy");
    assert_eq!(open_nodes(vol), 1);
}

#[test]
fn lexical_and_follow_side_by_side() {
    let lexical = Volume::new(fixture());
    let follow = Volume::with_resolve(fixture(), Resolve::Follow);

    assert_eq!(read(&lexical, "/missing/../a.txt").unwrap(), b"root a");
    assert_eq!(
        read(&follow, "/missing/../a.txt").unwrap_err().kind(),
        ErrorKind::NotFound
    );
    assert!(lexical.metadata("/a.txt/..").unwrap().file_type().is_dir());
    assert_eq!(
        follow.metadata("/a.txt/..").unwrap_err().kind(),
        ErrorKind::NotADirectory
    );
    assert_eq!(
        follow.metadata("/a.txt/").unwrap_err().kind(),
        ErrorKind::NotADirectory
    );
    for path in [
        "/etc/up/conf",
        "/link/conf",
        "/abs",
        "/long",
        "/link/../link/conf",
    ] {
        assert_eq!(read(&follow, path).unwrap(), b"key=value", "{path}");
    }
    assert_eq!(
        read(&lexical, "/link/conf").unwrap_err().kind(),
        ErrorKind::NotADirectory
    );
    assert_eq!(
        read(&follow, "/loop").unwrap_err().kind(),
        ErrorKind::Symlink
    );

    assert!(follow.metadata("/link").unwrap().file_type().is_dir());
    assert!(
        follow
            .symlink_metadata("/link")
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(lexical.metadata("/link").unwrap().file_type().is_symlink());
    assert_eq!(follow.read_link("/link").unwrap(), b"etc");
    assert_eq!(lexical.read_link("/etc/up").unwrap(), b"../etc");
    assert_eq!(
        lexical.read_link("/a.txt").unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    assert_eq!(open_nodes(lexical), 1);
    assert_eq!(open_nodes(follow), 1);
}

#[test]
fn the_bare_tier_picks_a_policy_per_call() {
    let mut fs = fixture();
    let node = fs.resolve(b"/etc/../a.txt", Resolve::Follow).unwrap();
    assert_eq!(fs.stat(node).unwrap().len(), 6);
    fs.forget(node, 1);
    let link = fs.resolve(b"/link", Resolve::NoFollow).unwrap();
    assert!(fs.stat(link).unwrap().file_type().is_symlink());
    fs.forget(link, 1);
    let dir = fs.resolve(b"/link/", Resolve::NoFollow).unwrap();
    assert!(fs.stat(dir).unwrap().file_type().is_dir());
    fs.forget(dir, 1);
    let mut long = String::from("/link");
    for _ in 0..600 {
        long.push_str("/.");
    }
    long.push_str("/conf");
    let err = fs.resolve(long.as_bytes(), Resolve::Follow).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::LimitExceeded);
    assert_eq!(fs.open_nodes(), 1);
}

#[test]
fn clones_share_the_volume_and_hold_it() {
    let vol = Volume::new(fixture());
    let clone = vol.clone();
    let Err(vol) = vol.into_inner() else {
        panic!("a clone holds the volume")
    };
    std::thread::spawn(move || write(&clone, "/from-thread", b"t").unwrap())
        .join()
        .unwrap();
    assert_eq!(read(&vol, "/from-thread").unwrap(), b"t");
    let file = vol.open("/a.txt", OpenOptions::new().read()).unwrap();
    let Err(vol) = vol.into_inner() else {
        panic!("a clone holds the volume")
    };
    drop(file);
    assert_eq!(open_nodes(vol), 1);
}

#[test]
fn handles_dropped_under_the_lock_are_queued() {
    let vol = Volume::new(fixture());
    for i in 0..40 {
        write(&vol, &format!("/h{i}"), b"x").unwrap();
    }
    let mut files = Vec::new();
    for i in 0..40 {
        files.push(
            vol.open(format!("/h{i}"), OpenOptions::new().read())
                .unwrap(),
        );
    }
    let dir = vol.read_dir("/etc").unwrap();
    let guard = vol.lock();
    assert_eq!(guard.open_files(), 40);
    drop(files);
    drop(dir);
    drop(guard);
    let fs = vol.into_inner().unwrap();
    assert_eq!((fs.open_nodes(), fs.open_files()), (1, 0));
}

#[test]
fn dropping_a_written_file_closes_it() {
    let vol = Volume::new(fixture());
    let before = vol.lock().closes();
    let mut file = vol
        .open("/log", OpenOptions::new().write().create())
        .unwrap();
    file.write_all(b"entry").unwrap();
    drop(file);
    assert_eq!(vol.lock().closes(), before + 1);
    let file = vol.open("/log", OpenOptions::new().write()).unwrap();
    let guard = vol.lock();
    drop(file);
    assert_eq!(guard.closes(), before + 1);
    drop(guard);
    assert_eq!(vol.lock().closes(), before + 2);
    assert_eq!(open_nodes(vol), 1);
}
