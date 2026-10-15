//! Concatenated cpio archives, as an x86 microcode archive followed by an
//! initramfs.

use std::io::Write;
use std::path::Path;
use std::process::{Output, Stdio};

fn newc(entries: &[(&str, u32, &[u8])]) -> Vec<u8> {
    let mut archive = Vec::new();
    for &(name, mode, data) in entries.iter().chain([&("TRAILER!!!", 0, &b""[..])]) {
        archive.extend_from_slice(b"070701");
        for value in [
            1u32,
            mode,
            0,
            0,
            1,
            0,
            data.len() as u32,
            0,
            0,
            0,
            0,
            name.len() as u32 + 1,
            0,
        ] {
            archive.extend_from_slice(format!("{value:08X}").as_bytes());
        }
        archive.extend_from_slice(name.as_bytes());
        archive.push(0);
        while archive.len() % 4 != 0 {
            archive.push(0);
        }
        archive.extend_from_slice(data);
        while archive.len() % 4 != 0 {
            archive.push(0);
        }
    }
    archive
}

/// A microcode-style archive padded to 512 bytes, then an initramfs that
/// also has `etc/motd`.
fn concatenated() -> Vec<u8> {
    let mut archive = newc(&[
        ("kernel", 0o040755, b""),
        ("kernel/x86", 0o040755, b""),
        ("kernel/x86/microcode", 0o040755, b""),
        ("kernel/x86/microcode/GenuineIntel.bin", 0o100644, b"ucode"),
        ("etc/motd", 0o100644, b"old"),
    ]);
    archive.resize(archive.len().div_ceil(512) * 512, 0);
    archive.extend(newc(&[
        ("init", 0o100755, b"#!/bin/sh"),
        ("etc/motd", 0o100644, b"new"),
    ]));
    archive
}

fn hadris(args: &[&str], archive: &Path) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_hadris"))
        .arg("cpio")
        .args(args)
        .arg(archive)
        .output()
        .unwrap()
}

fn write(dir: &Path, archive: &[u8]) -> std::path::PathBuf {
    let path = dir.join("c.cpio");
    std::fs::write(&path, archive).unwrap();
    path
}

fn stdout(output: &Output) -> String {
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[test]
fn ls_lists_every_segment() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), &concatenated());
    let out = stdout(&hadris(&["ls"], &path));
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        [
            "kernel",
            "kernel/x86",
            "kernel/x86/microcode",
            "kernel/x86/microcode/GenuineIntel.bin",
            "etc/motd",
            "",
            "segment 2:",
            "init",
            "etc/motd",
        ]
    );
    let single = write(dir.path(), &newc(&[("a", 0o100644, b"a")]));
    assert_eq!(stdout(&hadris(&["ls"], &single)), "a\n");
}

#[test]
fn info_counts_segments() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), &concatenated());
    let out = stdout(&hadris(&["info"], &path));
    assert!(out.contains("Format:       newc (070701)"), "{out}");
    assert!(out.contains("Segments:     2"), "{out}");
    assert!(out.contains("  1: newc (070701), 5 entries"), "{out}");
    assert!(out.contains("  2: newc (070701), 2 entries"), "{out}");
    assert!(out.contains("Entries:      7"), "{out}");
}

#[test]
fn cat_reads_later_segments_and_the_last_name_wins() {
    let dir = tempfile::tempdir().unwrap();
    let archive = concatenated();
    let path = write(dir.path(), &archive);
    let cat = |name: &str| {
        std::process::Command::new(env!("CARGO_BIN_EXE_hadris"))
            .args(["cpio", "cat"])
            .arg(&path)
            .arg(name)
            .output()
            .unwrap()
    };
    assert_eq!(stdout(&cat("init")), "#!/bin/sh");
    assert_eq!(stdout(&cat("etc/motd")), "new");
    assert_eq!(
        stdout(&cat("kernel/x86/microcode/GenuineIntel.bin")),
        "ucode"
    );

    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_hadris"))
        .args(["cpio", "cat", "-", "/etc/motd"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&archive).unwrap();
    assert_eq!(stdout(&child.wait_with_output().unwrap()), "new");
}

#[test]
fn extract_writes_every_segment_and_later_entries_win() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), &concatenated());
    let out = dir.path().join("out");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_hadris"))
        .args(["cpio", "extract", "-o"])
        .arg(&out)
        .arg(&path)
        .output()
        .unwrap();
    assert!(stdout(&output).contains("Extracted 7 entries"));
    assert_eq!(
        std::fs::read(out.join("kernel/x86/microcode/GenuineIntel.bin")).unwrap(),
        b"ucode"
    );
    assert_eq!(std::fs::read(out.join("init")).unwrap(), b"#!/bin/sh");
    assert_eq!(std::fs::read(out.join("etc/motd")).unwrap(), b"new");
}

#[test]
fn data_after_a_trailer_that_is_not_cpio_ends_the_archive() {
    let dir = tempfile::tempdir().unwrap();
    let mut archive = newc(&[("a", 0o100644, b"a")]);
    archive.extend_from_slice(&[0x1f, 0x8b, 0x08, 0x00, 1, 2, 3, 4]);
    let path = write(dir.path(), &archive);
    let output = hadris(&["ls"], &path);
    assert_eq!(stdout(&output), "a\n");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("not a cpio archive"), "{stderr}");
}
