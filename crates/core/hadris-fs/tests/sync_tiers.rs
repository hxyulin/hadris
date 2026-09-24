//! Every tier does every job (S15), with pins balanced after each step.

#![cfg(all(feature = "sync", feature = "std"))]

mod common;

use std::io::{Read as _, Write as _};
use std::sync::Arc;

use common::sync::{MemFs, fixture};
use hadris_fs::sync::lock::LockKind;
use hadris_fs::sync::{
    DriverExt, File, FileSystem, Local, OpenFile, PathExt, Spin, StdMutex, Volume,
};
use hadris_fs::{ErrorKind, OpenOptions};

fn names<A: hadris_fs::sync::Access>(dir: hadris_fs::sync::Dir<A>) -> Vec<String> {
    let mut names: Vec<String> = dir
        .map(|item| item.unwrap().name_str().unwrap().to_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn raw_tier_does_everything() {
    let mut fs = MemFs::new();
    fs.create_dir_all("/var/log").unwrap();
    fs.write_file("/var/log/a.txt", b"alpha").unwrap();
    assert!(fs.exists("/var/log/a.txt").unwrap());
    assert!(!fs.exists("/var/log/b.txt").unwrap());
    assert_eq!(fs.metadata("/var/log/a.txt").unwrap().len(), 5);
    assert_eq!(fs.read_to_vec("/var/log/a.txt").unwrap(), b"alpha");

    let mut f = fs
        .open("/var/log/b.txt", OpenOptions::write().create())
        .unwrap();
    writeln!(f, "beta").unwrap();
    f.close().unwrap();
    let mut text = String::new();
    fs.open("/var/log/b.txt", OpenOptions::read())
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    assert_eq!(text, "beta\n");
    assert_eq!(names(fs.read_dir("/var/log").unwrap()), ["a.txt", "b.txt"]);

    fs.rename_path("/var/log/b.txt", "/var/b.txt").unwrap();
    assert_eq!(fs.read_to_vec("/var/b.txt").unwrap(), b"beta\n");
    fs.remove_file("/var/b.txt").unwrap();
    assert_eq!(
        fs.remove_dir("/var").unwrap_err().kind(),
        ErrorKind::DirectoryNotEmpty
    );
    fs.remove_dir_all("/var").unwrap();
    assert!(!fs.exists("/var").unwrap());
    assert_eq!(fs.open_nodes(), 1);
}

#[test]
fn file_table_holds_many_open_files_beside_one_driver() {
    let mut fs = fixture();
    let mut table = [None::<OpenFile>; 4];
    table[0] = Some(OpenFile::open(&mut fs, "/a.txt", OpenOptions::read()).unwrap());
    table[1] = Some(OpenFile::open(&mut fs, "/etc/conf", OpenOptions::read()).unwrap());
    let mut buf = [0u8; 4];
    assert_eq!(
        table[1].as_mut().unwrap().read(&mut fs, &mut buf).unwrap(),
        4
    );
    assert_eq!(&buf, b"key=");
    assert_eq!(
        table[0].as_mut().unwrap().read(&mut fs, &mut buf).unwrap(),
        4
    );
    assert_eq!(&buf, b"root");
    for slot in &mut table {
        if let Some(file) = slot.take() {
            file.close(&mut fs, Ok(())).unwrap();
        }
    }
    assert_eq!(fs.open_nodes(), 1);
}

#[test]
fn a_raw_file_lends_the_driver_back() {
    let mut fs = fixture();
    let mut file = fs.open("/a.txt", OpenOptions::read()).unwrap();
    assert!(file.driver().exists("/etc/conf").unwrap());
    let mut text = String::new();
    file.read_to_string(&mut text).unwrap();
    assert_eq!(text, "root a");
    drop(file);
    assert_eq!(fs.open_nodes(), 1);
}

#[test]
fn shared_and_owned_tiers_do_the_same() {
    let vol = Volume::new(MemFs::new());
    vol.create_dir_all("/etc").unwrap();
    vol.write_file("/etc/a.txt", b"alpha").unwrap();
    let mut a = vol.open("/etc/a.txt", OpenOptions::read()).unwrap();
    let mut b = vol
        .open("/etc/b.txt", OpenOptions::write().create())
        .unwrap();
    std::io::copy(&mut a, &mut b).unwrap();
    drop((a, b));
    assert_eq!(names(vol.read_dir("/etc").unwrap()), ["a.txt", "b.txt"]);

    let owned = Arc::new(vol);
    let worker = {
        let owned = Arc::clone(&owned);
        std::thread::spawn(move || {
            let mut file = File::open(owned, "/etc/b.txt", OpenOptions::read()).unwrap();
            let mut text = Vec::new();
            file.read_to_end(&mut text).unwrap();
            text
        })
    };
    assert_eq!(worker.join().unwrap(), b"alpha");
    let vol = Arc::into_inner(owned).unwrap();
    assert_eq!(vol.into_inner().open_nodes(), 1);
}

#[test]
fn every_lock_kind_builds_the_same_volume() {
    let spin = Volume::spin(fixture());
    let local = Volume::local(fixture());
    assert_eq!(spin.read_to_vec("/a.txt").unwrap(), b"root a");
    assert_eq!(local.read_to_vec("/a.txt").unwrap(), b"root a");
    assert!(local.capabilities().is_writable());
    assert_eq!(FileSystem::root(&local), local.lock().root());
}

#[test]
fn forgets_queued_during_a_lock_are_applied() {
    let vol = Volume::spin(fixture());
    let file = vol.open("/a.txt", OpenOptions::read()).unwrap();
    {
        let guard = vol.lock();
        drop(file);
        assert_eq!(guard.open_nodes(), 2);
    }
    assert!(vol.exists("/etc").unwrap());
    assert_eq!(vol.lock().open_nodes(), 1);
}

#[test]
fn a_borrowed_volume_hands_queued_forgets_back_on_drop() {
    let mut fs = fixture();
    {
        let vol = Volume::spin(&mut fs);
        let file = vol.open("/a.txt", OpenOptions::read()).unwrap();
        let guard = vol.lock();
        drop(file);
        drop(guard);
    }
    assert_eq!(fs.open_nodes(), 1);
}

#[test]
fn into_inner_applies_queued_forgets() {
    let vol = Volume::spin(fixture());
    let file = vol.open("/a.txt", OpenOptions::read()).unwrap();
    let guard = vol.lock();
    drop(file);
    drop(guard);
    assert_eq!(vol.into_inner().open_nodes(), 1);
}

#[test]
fn dropping_a_written_file_publishes_it() {
    let mut fs = fixture();
    let mut file = fs.open("/log", OpenOptions::write().create()).unwrap();
    file.write_all(b"entry").unwrap();
    drop(file);
    assert_eq!(fs.publishes(), 1);
    drop(fs.open("/log", OpenOptions::read()).unwrap());
    assert_eq!(fs.publishes(), 1);
    let mut file = fs.open("/log", OpenOptions::write().append()).unwrap();
    file.write_all(b" more").unwrap();
    file.close().unwrap();
    assert_eq!(fs.publishes(), 2);
    assert_eq!(fs.open_nodes(), 1);
}

#[test]
fn a_file_dropped_under_the_lock_publishes_with_its_queued_close() {
    let vol = Volume::local(fixture());
    let mut file = vol.open("/log", OpenOptions::write().create()).unwrap();
    file.write_all(b"entry").unwrap();
    {
        let guard = vol.lock();
        drop(file);
        assert_eq!(guard.publishes(), 0);
    }
    assert_eq!(vol.lock().publishes(), 1);
    assert_eq!(vol.into_inner().open_nodes(), 1);

    let vol = Volume::new(fixture());
    let mut file = vol.open("/log", OpenOptions::write().create()).unwrap();
    file.write_all(b"entry").unwrap();
    drop(file);
    assert_eq!(vol.lock().publishes(), 1);
}

fn drop_many_under_the_lock<K: LockKind>() {
    let mut fs = fixture();
    for i in 0..40 {
        fs.write_file(&format!("/h{i}"), b"x").unwrap();
    }
    let vol = Volume::<_, K>::with_lock(fs);
    let files: Vec<_> = (0..40)
        .map(|i| vol.open(&format!("/h{i}"), OpenOptions::read()).unwrap())
        .collect();
    let guard = vol.lock();
    drop(files);
    drop(guard);
    assert_eq!(vol.into_inner().open_nodes(), 1);
}

#[test]
fn many_handles_dropped_under_the_lock_are_queued() {
    drop_many_under_the_lock::<StdMutex>();
    drop_many_under_the_lock::<Spin>();
    drop_many_under_the_lock::<Local>();
}

#[test]
#[should_panic(expected = "while its `lock()` guard is")]
fn a_local_call_under_the_lock_names_the_cause() {
    let vol = Volume::local(fixture());
    let _guard = vol.lock();
    let _ = vol.metadata("/a.txt");
}
