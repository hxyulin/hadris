//! `cpio extract` on hostile and unusual archives.

#![cfg(unix)]

use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::process::Output;

/// One newc entry.
struct Entry<'a> {
    name: &'a str,
    mode: u32,
    ino: u32,
    nlink: u32,
    dev: (u32, u32),
    data: &'a [u8],
}

fn entry<'a>(name: &'a str, mode: u32, data: &'a [u8]) -> Entry<'a> {
    Entry {
        name,
        mode,
        ino: 0,
        nlink: 1,
        dev: (0, 0),
        data,
    }
}

fn link<'a>(name: &'a str, ino: u32, data: &'a [u8]) -> Entry<'a> {
    Entry {
        name,
        mode: 0o100644,
        ino,
        nlink: 2,
        dev: (0, 0),
        data,
    }
}

fn newc(entries: &[Entry<'_>]) -> Vec<u8> {
    let mut archive = Vec::new();
    for entry in entries.iter().chain([&entry("TRAILER!!!", 0, b"")]) {
        archive.extend_from_slice(b"070701");
        for value in [
            entry.ino,
            entry.mode,
            0,
            0,
            entry.nlink,
            0,
            entry.data.len() as u32,
            entry.dev.0,
            entry.dev.1,
            0,
            0,
            entry.name.len() as u32 + 1,
            0,
        ] {
            archive.extend_from_slice(format!("{value:08X}").as_bytes());
        }
        archive.extend_from_slice(entry.name.as_bytes());
        archive.push(0);
        while archive.len() % 4 != 0 {
            archive.push(0);
        }
        archive.extend_from_slice(entry.data);
        while archive.len() % 4 != 0 {
            archive.push(0);
        }
    }
    archive
}

fn extract(dir: &Path, archive: &[u8], args: &[&str]) -> Output {
    let path = dir.join("a.cpio");
    std::fs::write(&path, archive).unwrap();
    std::process::Command::new(env!("CARGO_BIN_EXE_hadris"))
        .args(["cpio", "extract", "-o"])
        .arg(dir.join("out"))
        .args(args)
        .arg(&path)
        .output()
        .unwrap()
}

fn mode(path: &Path) -> u32 {
    std::fs::symlink_metadata(path)
        .unwrap()
        .permissions()
        .mode()
        & 0o7777
}

fn victim(dir: &Path) -> (std::path::PathBuf, String) {
    let victim = dir.join("victim.txt");
    std::fs::write(&victim, b"original").unwrap();
    std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o644)).unwrap();
    let target = victim.to_str().unwrap().to_owned();
    (victim, target)
}

#[test]
fn unselected_link_data_does_not_follow_a_replaced_name() {
    let dir = tempfile::tempdir().unwrap();
    let (victim, target) = victim(dir.path());
    let archive = newc(&[
        entry("sel", 0o040755, b""),
        link("sel/a", 5, b""),
        entry("sel/a", 0o120777, target.as_bytes()),
        link("other/b", 5, b"PWNED"),
    ]);
    let output = extract(dir.path(), &archive, &["-p", "sel"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(std::fs::read(&victim).unwrap(), b"original");
    assert!(
        dir.path()
            .join("out/sel/a")
            .symlink_metadata()
            .unwrap()
            .is_symlink()
    );
}

#[test]
fn a_link_does_not_chmod_through_a_replaced_name() {
    let dir = tempfile::tempdir().unwrap();
    let (victim, target) = victim(dir.path());
    let mut g = link("g", 7, b"");
    g.mode = 0o100777;
    let archive = newc(&[
        link("f", 7, b"x"),
        entry("f", 0o120777, target.as_bytes()),
        g,
    ]);
    let output = extract(dir.path(), &archive, &[]);
    assert_eq!(mode(&victim), 0o644);
    assert_eq!(std::fs::read(&victim).unwrap(), b"original");
    assert!(output.status.success(), "{output:?}");
    let out = dir.path().join("out");
    assert!(out.join("f").symlink_metadata().unwrap().is_symlink());
    assert!(out.join("g").symlink_metadata().unwrap().is_file());
}

#[test]
fn links_join_names_of_one_device_and_inode() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = link("a", 9, b"A");
    a.dev = (8, 1);
    let mut b = link("b", 9, b"B");
    b.dev = (8, 2);
    let mut c = link("c", 9, b"");
    c.dev = (8, 2);
    let archive = newc(&[c, a, b, link("d", 10, b""), link("e", 10, b"E")]);
    let output = extract(dir.path(), &archive, &[]);
    assert!(output.status.success(), "{output:?}");
    let out = dir.path().join("out");
    let read = |name: &str| std::fs::read(out.join(name)).unwrap();
    let ino = |name: &str| std::fs::metadata(out.join(name)).unwrap().ino();
    assert_eq!(read("a"), b"A");
    assert_eq!(read("b"), b"B");
    assert_eq!(read("c"), b"B");
    assert_ne!(ino("a"), ino("b"));
    assert_eq!(ino("b"), ino("c"));
    assert_eq!(read("d"), b"E");
    assert_eq!(ino("d"), ino("e"));
}

#[test]
fn a_read_only_directory_gets_its_mode_after_its_children() {
    let dir = tempfile::tempdir().unwrap();
    let archive = newc(&[
        entry("d", 0o040500, b""),
        entry("d/e", 0o040555, b""),
        entry("d/e/f", 0o100444, b"data"),
    ]);
    let output = extract(dir.path(), &archive, &[]);
    assert!(output.status.success(), "{output:?}");
    let out = dir.path().join("out");
    assert_eq!(std::fs::read(out.join("d/e/f")).unwrap(), b"data");
    assert_eq!(mode(&out.join("d")), 0o500);
    assert_eq!(mode(&out.join("d/e")), 0o555);
    assert_eq!(mode(&out.join("d/e/f")), 0o444);
    for dir in ["d/e", "d"] {
        std::fs::set_permissions(out.join(dir), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[test]
fn an_existing_directory_keeps_its_mode() {
    let dir = tempfile::tempdir().unwrap();
    let existing = dir.path().join("out/existing");
    std::fs::create_dir_all(&existing).unwrap();
    std::fs::set_permissions(&existing, std::fs::Permissions::from_mode(0o700)).unwrap();
    let archive = newc(&[
        entry("existing", 0o040777, b""),
        entry("existing/f", 0o100644, b"data"),
    ]);
    let output = extract(dir.path(), &archive, &[]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(mode(&existing), 0o700);
    assert_eq!(std::fs::read(existing.join("f")).unwrap(), b"data");
}

#[test]
fn special_bits_are_kept_only_on_request() {
    let dir = tempfile::tempdir().unwrap();
    let archive = newc(&[entry("d", 0o041777, b""), entry("d/suid", 0o104755, b"x")]);
    let output = extract(dir.path(), &archive, &[]);
    assert!(output.status.success(), "{output:?}");
    let out = dir.path().join("out");
    assert_eq!(mode(&out.join("d/suid")), 0o755);
    assert_eq!(mode(&out.join("d")), 0o777);

    std::fs::remove_dir_all(&out).unwrap();
    let output = extract(dir.path(), &archive, &["--preserve-special-bits"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(mode(&out.join("d/suid")), 0o4755);
    assert_eq!(mode(&out.join("d")), 0o1777);
}

#[test]
fn cat_reads_a_hard_link_from_the_name_with_data() {
    let dir = tempfile::tempdir().unwrap();
    let mut other = link("b", 3, b"other");
    other.dev = (1, 0);
    let path = dir.path().join("a.cpio");
    let archive = newc(&[link("a", 3, b""), other, link("c", 3, b"data")]);
    std::fs::write(&path, archive).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_hadris"))
        .args(["cpio", "cat"])
        .arg(&path)
        .arg("a")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"data");
}

fn cat(archive: &[u8], name: &str) -> (Output, Output) {
    use std::io::Write as _;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.cpio");
    std::fs::write(&path, archive).unwrap();
    let file = std::process::Command::new(env!("CARGO_BIN_EXE_hadris"))
        .args(["cpio", "cat"])
        .arg(&path)
        .arg(name)
        .output()
        .unwrap();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_hadris"))
        .args(["cpio", "cat", "-", name])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(archive).unwrap();
    (file, child.wait_with_output().unwrap())
}

#[test]
fn cat_reads_a_hard_link_whose_data_came_first() {
    let mut other = link("b", 3, b"other");
    other.dev = (1, 0);
    let archive = newc(&[link("c", 3, b"data"), other, link("a", 3, b"")]);
    for output in <[Output; 2]>::from(cat(&archive, "a")) {
        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout, b"data");
    }

    let archive = newc(&[
        link("c", 3, b"data"),
        link("a", 3, b""),
        entry("a", 0o100644, b"new"),
    ]);
    for output in <[Output; 2]>::from(cat(&archive, "a")) {
        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout, b"new");
    }

    let archive = newc(&[
        link("c", 3, b"old"),
        link("a", 3, b""),
        link("d", 3, b"late"),
    ]);
    for output in <[Output; 2]>::from(cat(&archive, "a")) {
        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout, b"late");
    }
}

#[test]
fn selected_hard_link_retains_earlier_unselected_data() {
    for data_first in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let entries = if data_first {
            [link("outside", 3, b"payload"), link("selected", 3, b"")]
        } else {
            [link("selected", 3, b""), link("outside", 3, b"payload")]
        };
        let out = extract(dir.path(), &newc(&entries), &["--path", "selected"]);
        assert!(out.status.success(), "{out:?}");
        assert_eq!(
            std::fs::read(dir.path().join("out/selected")).unwrap(),
            b"payload"
        );
        assert!(!dir.path().join("out/outside").exists());
    }
}

#[test]
fn selected_hard_link_spools_stdin_and_resets_at_trailers() {
    use std::io::Write;
    use std::process::Stdio;
    let dir = tempfile::tempdir().unwrap();
    let mut archive = newc(&[
        link("outside", 7, b"payload"),
        link("selected/a", 7, b""),
        link("selected/b", 7, b""),
    ]);
    archive.extend(newc(&[link("selected/c", 7, b"")]));
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_hadris"))
        .args(["cpio", "extract", "-", "--path", "selected", "-o"])
        .arg(dir.path().join("out"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&archive).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{out:?}");
    let out = dir.path().join("out/selected");
    assert_eq!(std::fs::read(out.join("a")).unwrap(), b"payload");
    assert_eq!(
        std::fs::metadata(out.join("a")).unwrap().ino(),
        std::fs::metadata(out.join("b")).unwrap().ino()
    );
    assert_eq!(std::fs::read(out.join("c")).unwrap(), b"");
}

#[test]
fn hard_link_groups_do_not_hold_descriptors_open() {
    let dir = tempfile::tempdir().unwrap();
    let names: Vec<String> = (0..200).map(|i| format!("file{i}")).collect();
    let entries: Vec<_> = names
        .iter()
        .zip(1..)
        .map(|(name, ino)| link(name, ino, b"payload"))
        .collect();
    let path = dir.path().join("a.cpio");
    std::fs::write(&path, newc(&entries)).unwrap();
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(r#"ulimit -n 64 && exec "$0" "$@""#)
        .arg(env!("CARGO_BIN_EXE_hadris"))
        .args(["cpio", "extract", "-o"])
        .arg(dir.path().join("out"))
        .arg(&path)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    for name in &names {
        assert_eq!(
            std::fs::read(dir.path().join("out").join(name)).unwrap(),
            b"payload"
        );
    }
}

#[test]
fn selected_hard_link_retains_latest_data_after_names_are_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let archive = newc(&[
        link("outside", 4, b"old"),
        link("selected/a", 4, b""),
        link("selected/b", 4, b"new"),
        entry("selected/a", 0o100644, b"replacement"),
        entry("selected/b", 0o100644, b"replacement"),
        link("selected/c", 4, b""),
    ]);
    let out = extract(dir.path(), &archive, &["--path", "selected"]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        std::fs::read(dir.path().join("out/selected/c")).unwrap(),
        b"new"
    );
}
