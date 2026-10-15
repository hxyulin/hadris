//! The shared tier: paths and handles named after `std::fs`, path policies,
//! and handles dropped while the lock is held.

#![cfg(all(feature = "sync", feature = "std"))]

mod common;

use std::io::{Read as _, Write as _};

use common::files::{exists, names, read, write};
use common::sync::{MemFs, fixture};
use hadris_fs::sync::{FileSystem, Volume};
use hadris_fs::{ErrorKind, FileType, OpenOptions, Resolve, SeekFrom, SetAttr};

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
    for dot in ["/var/log/.", "/var/log/deep/..", "/var/log/deep/./"] {
        assert_eq!(
            vol.remove_dir_all(dot).unwrap_err().kind(),
            ErrorKind::InvalidInput
        );
    }
    assert_eq!(read(&vol, "/var/log/deep/er/c").unwrap(), b"c");
    vol.remove_dir_all("/var").unwrap();
    assert!(names(&vol, "/").is_empty());
    assert_eq!(
        vol.remove_dir_all("/").unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    assert_eq!(open_nodes(vol), 1);
}

#[test]
fn remove_dir_all_lists_each_directory_once() {
    let mut fs = MemFs::new();
    fs.add("/", "many", FileType::Dir, b"");
    fs.add("/many", "sub", FileType::Dir, b"");
    for i in 0..200 {
        fs.add("/many", &format!("f{i}"), FileType::File, b"");
        fs.add("/many/sub", &format!("g{i}"), FileType::File, b"");
    }
    let vol = Volume::new(fs);
    vol.remove_dir_all("/many").unwrap();
    assert!(names(&vol, "/").is_empty());
    let fs = vol.into_inner().unwrap();
    assert!(fs.scanned() < 3000, "{}", fs.scanned());
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
    let guard = vol.lock().unwrap();
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
    let before = vol.lock().unwrap().closes();
    let mut file = vol
        .open("/log", OpenOptions::new().write().create())
        .unwrap();
    file.write_all(b"entry").unwrap();
    drop(file);
    assert_eq!(vol.lock().unwrap().closes(), before + 1);
    let file = vol.open("/log", OpenOptions::new().write()).unwrap();
    let guard = vol.lock().unwrap();
    drop(file);
    assert_eq!(guard.closes(), before + 1);
    drop(guard);
    assert_eq!(vol.lock().unwrap().closes(), before + 2);
    assert_eq!(open_nodes(vol), 1);
}

#[test]
fn whole_file_helpers() {
    let vol = Volume::new(fixture());
    assert_eq!(vol.read("/etc/conf").unwrap(), b"key=value");
    vol.write("/new.txt", b"fresh").unwrap();
    assert_eq!(vol.read("/new.txt").unwrap(), b"fresh");
    vol.write("/new.txt", "x").unwrap();
    assert_eq!(vol.read("/new.txt").unwrap(), b"x");
    vol.write("/empty", []).unwrap();
    assert_eq!(vol.read("/empty").unwrap(), b"");
    assert!(vol.exists("/new.txt").unwrap());
    assert!(vol.exists("/etc").unwrap());
    assert!(!vol.exists("/missing").unwrap());
    assert_eq!(
        vol.exists("/a.txt/x").unwrap_err().kind(),
        ErrorKind::NotADirectory
    );
    assert_eq!(
        vol.read("/etc").unwrap_err().kind(),
        ErrorKind::IsADirectory
    );
    assert_eq!(vol.read("/nope").unwrap_err().kind(), ErrorKind::NotFound);
    assert_eq!(
        vol.write("/etc", b"x").unwrap_err().kind(),
        ErrorKind::IsADirectory
    );
    let read_only = Volume::new(fixture().read_only());
    assert_eq!(
        read_only.write("/a.txt", b"x").unwrap_err().kind(),
        ErrorKind::ReadOnly
    );
    assert_eq!(read_only.read("/a.txt").unwrap(), b"root a");
    let fs = vol.into_inner().unwrap();
    assert_eq!((fs.open_nodes(), fs.open_files()), (1, 0));
}

#[test]
fn walk_yields_paths_depth_first() {
    let vol = Volume::new(fixture());
    let paths = |path: &str| -> Vec<String> {
        vol.walk(path)
            .unwrap()
            .map(|entry| String::from_utf8(entry.unwrap().into_path()).unwrap())
            .collect()
    };
    let all = [
        "/etc",
        "/etc/conf",
        "/etc/up",
        "/link",
        "/abs",
        "/loop",
        "/long",
        "/a.txt",
    ];
    assert_eq!(paths("/"), all);
    assert_eq!(paths(""), all);
    assert_eq!(paths("/etc/"), ["/etc/conf", "/etc/up"]);
    assert_eq!(paths("etc"), ["etc/conf", "etc/up"]);

    let mut walk = vol.walk("/").unwrap();
    let first = walk.next().unwrap().unwrap();
    assert_eq!((first.path(), first.depth()), (&b"/etc"[..], 1));
    assert!(first.entry().file_type().is_dir());
    walk.skip_dir();
    let next = walk.next().unwrap().unwrap();
    assert_eq!(next.path(), b"/link");
    assert_eq!(next.dir(), vol.lock().unwrap().root());
    drop(walk);

    let deep = vol.walk("/etc").unwrap().next().unwrap().unwrap();
    assert_eq!(deep.depth(), 1);
    assert_eq!(
        vol.walk("/a.txt").unwrap_err().kind(),
        ErrorKind::NotADirectory
    );
    assert_eq!(vol.walk("/nope").unwrap_err().kind(), ErrorKind::NotFound);
    assert_eq!(open_nodes(vol), 1);
}

#[test]
fn walk_stops_at_a_directory_loop() {
    for (dir, target) in [("/etc", "/etc"), ("/etc", "/")] {
        let mut fs = fixture();
        fs.alias(dir, "back", target);
        let vol = Volume::new(fs);
        let mut walk = vol.walk("/").unwrap();
        let err = walk.by_ref().find_map(Result::err).unwrap();
        assert_eq!(err.kind(), ErrorKind::Corrupt, "{dir} -> {target}");
        assert!(walk.next().is_none(), "fused");
        drop(walk);
        assert_eq!(open_nodes(vol), 1);
    }
}

#[test]
fn open_node_opens_a_listed_entry() {
    let vol = Volume::new(fixture());
    let mut dir = vol.read_dir("/etc").unwrap();
    let entries: Vec<_> = dir.by_ref().map(Result::unwrap).collect();
    let entry = |name: &[u8]| {
        entries
            .iter()
            .find(|e| e.name().as_bytes() == name)
            .unwrap()
    };
    let mut file = dir.open(entry(b"conf"), OpenOptions::new().read()).unwrap();
    let mut text = String::new();
    file.read_to_string(&mut text).unwrap();
    assert_eq!(text, "key=value");
    drop(file);
    assert_eq!(
        dir.open(entry(b"up"), OpenOptions::new().read())
            .unwrap_err()
            .kind(),
        ErrorKind::Symlink
    );
    assert_eq!(
        dir.open(entry(b"conf"), OpenOptions::new().write().create_new())
            .unwrap_err()
            .kind(),
        ErrorKind::AlreadyExists
    );
    assert_eq!(
        dir.open(entry(b"conf"), OpenOptions::new())
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidInput
    );
    let file = dir
        .open(
            entry(b"conf"),
            OpenOptions::new().write().create().truncate(),
        )
        .unwrap();
    assert_eq!(file.metadata().unwrap().len(), 0);
    file.close().unwrap();

    vol.remove_file("/etc/conf").unwrap();
    write(&vol, "/etc/conf", b"replaced").unwrap();
    assert_eq!(
        dir.open(entry(b"conf"), OpenOptions::new().read())
            .unwrap_err()
            .kind(),
        ErrorKind::NotFound,
        "the name leads to another node"
    );
    vol.rename("/etc/conf", "/etc/moved").unwrap();
    let moved = vol
        .read_dir("/etc")
        .unwrap()
        .map(Result::unwrap)
        .find(|e| e.name().as_bytes() == b"moved")
        .unwrap();
    vol.rename("/etc/moved", "/etc/conf").unwrap();
    assert_eq!(
        dir.open(&moved, OpenOptions::new().read())
            .unwrap_err()
            .kind(),
        ErrorKind::NotFound,
        "the name is gone"
    );
    drop(dir);

    let found = vol
        .walk("/")
        .unwrap()
        .map(Result::unwrap)
        .find(|e| e.path() == b"/etc/conf")
        .unwrap();
    let mut file = vol
        .open_node(
            found.dir(),
            found.entry(),
            OpenOptions::new().read().write(),
        )
        .unwrap();
    file.write_all(b"via walk").unwrap();
    file.close().unwrap();
    assert_eq!(read(&vol, "/etc/conf").unwrap(), b"via walk");

    let read_only = Volume::new(fixture().read_only());
    let root = read_only.lock().unwrap().root();
    let a = read_only
        .read_dir("/")
        .unwrap()
        .map(Result::unwrap)
        .find(|e| e.name().as_bytes() == b"a.txt")
        .unwrap();
    assert_eq!(
        read_only
            .open_node(root, &a, OpenOptions::new().write())
            .unwrap_err()
            .kind(),
        ErrorKind::ReadOnly
    );
    read_only
        .open_node(root, &a, OpenOptions::new().read())
        .unwrap()
        .close()
        .unwrap();
    assert_eq!(open_nodes(read_only), 1);
    let fs = vol.into_inner().unwrap();
    assert_eq!((fs.open_nodes(), fs.open_files()), (1, 0));
}

#[test]
fn positional_io_and_attributes() {
    let time = hadris_fs::DateTime::from_unix_seconds(1_700_000_000).unwrap();
    let later = hadris_fs::DateTime::from_unix_seconds(1_800_000_000).unwrap();
    let vol = Volume::new(fixture());
    let mut file = vol
        .open("/a.txt", OpenOptions::new().read().write())
        .unwrap();
    file.seek(SeekFrom::Start(2)).unwrap();
    assert_eq!(file.write_at(4, b"AB").unwrap(), 2);
    let mut buf = [0u8; 3];
    assert_eq!(file.read_at(0, &mut buf).unwrap(), 3);
    assert_eq!(&buf, b"roo");
    assert_eq!(file.read_at(6, &mut buf).unwrap(), 0);
    file.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"otA", "the position is untouched");
    file.set_attr(&SetAttr::new().with_modified(time)).unwrap();
    assert_eq!(file.metadata().unwrap().modified(), Some(time));
    file.close().unwrap();

    let reader = vol.open("/a.txt", OpenOptions::new().read()).unwrap();
    assert_eq!(
        reader.write_at(0, b"x").unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    reader
        .set_attr(&SetAttr::new().with_modified(later))
        .unwrap();
    drop(reader);
    let writer = vol.open("/a.txt", OpenOptions::new().write()).unwrap();
    assert_eq!(
        writer.read_at(0, &mut buf).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    drop(writer);

    let stamped = SetAttr::new().with_modified(time);
    let made = vol
        .open(
            "/made",
            OpenOptions::new().write().create().with_attr(stamped),
        )
        .unwrap();
    assert_eq!(made.metadata().unwrap().modified(), Some(time));
    made.close().unwrap();
    let kept = vol
        .open(
            "/a.txt",
            OpenOptions::new().write().create().with_attr(stamped),
        )
        .unwrap();
    assert_eq!(
        kept.metadata().unwrap().modified(),
        Some(later),
        "an existing file keeps its attributes"
    );
    kept.close().unwrap();
    let fresh = vol
        .open(
            "/fresh",
            OpenOptions::new().write().create_new().with_attr(stamped),
        )
        .unwrap();
    assert_eq!(fresh.metadata().unwrap().modified(), Some(time));
    drop(fresh);
    assert!(vol.metadata("/plain").is_err());
    write(&vol, "/plain", b"").unwrap();
    assert_eq!(vol.metadata("/plain").unwrap().modified(), None);
    let fs = vol.into_inner().unwrap();
    assert_eq!((fs.open_nodes(), fs.open_files()), (1, 0));
}

#[test]
fn read_takes_the_length_as_a_hint() {
    let mut fs = fixture();
    fs.claim_len("/a.txt", u64::MAX);
    fs.claim_len("/etc/conf", 2);
    let vol = Volume::new(fs);
    assert_eq!(vol.read("/a.txt").unwrap(), b"root a");
    assert_eq!(vol.read("/etc/conf").unwrap(), b"key=value");
    let big: Vec<u8> = (0..3 << 20).map(|i| i as u8).collect();
    vol.write("/big", &big).unwrap();
    assert_eq!(vol.read("/big").unwrap(), big);
    let fs = vol.into_inner().unwrap();
    assert_eq!((fs.open_nodes(), fs.open_files()), (1, 0));
}
