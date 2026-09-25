//! Smoke tests for the canonical and compatibility CPIO binaries.

#[test]
fn help_succeeds() {
    let output = hadris("cpio").arg("--help").output().expect("run --help");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ls"));
}

#[test]
fn legacy_list_alias_is_accepted() {
    let output = hadris("cpio")
        .args(["list", "missing.cpio"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("unrecognized subcommand"));
}

#[test]
fn create_then_extract_through_pipes() {
    use std::io::Write;
    use std::process::Stdio;

    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("src");
    std::fs::create_dir_all(source.join("sub")).unwrap();
    std::fs::write(source.join("sub/file.txt"), b"hello").unwrap();
    let archive = hadris("cpio")
        .args(["create", "-o", "-"])
        .arg(&source)
        .output()
        .unwrap();
    assert!(archive.status.success());
    let out = dir.path().join("out");
    let mut child = hadris("cpio")
        .args(["extract", "-o"])
        .arg(&out)
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&archive.stdout)
        .unwrap();
    assert!(child.wait().unwrap().success());
    assert_eq!(std::fs::read(out.join("sub/file.txt")).unwrap(), b"hello");
}

fn newc_entry(archive: &mut Vec<u8>, name: &str, mode: u32, data: &[u8]) {
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

fn extract(dir: &std::path::Path, archive: &[u8]) -> std::process::Output {
    let path = dir.join("evil.cpio");
    std::fs::write(&path, archive).unwrap();
    hadris("cpio")
        .args(["extract", "-o"])
        .arg(dir.join("out"))
        .arg(&path)
        .output()
        .unwrap()
}

#[test]
fn extract_refuses_names_outside_the_output() {
    let dir = tempfile::tempdir().unwrap();
    let mut archive = Vec::new();
    newc_entry(&mut archive, "../escape", 0o100644, b"");
    let output = extract(dir.path(), &archive);
    assert!(!output.status.success());
    assert!(!dir.path().join("escape").exists());
}

#[cfg(unix)]
#[test]
fn extract_refuses_paths_through_an_extracted_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    let mut archive = Vec::new();
    newc_entry(
        &mut archive,
        "link",
        0o120777,
        outside.to_str().unwrap().as_bytes(),
    );
    newc_entry(&mut archive, "link/escaped.txt", 0o100644, b"evil");
    newc_entry(&mut archive, "TRAILER!!!", 0, b"");
    let output = extract(dir.path(), &archive);
    assert!(!output.status.success());
    assert!(!outside.join("escaped.txt").exists());
}

#[cfg(unix)]
#[test]
fn extract_replaces_a_symlink_with_a_directory_entry() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut archive = Vec::new();
    newc_entry(
        &mut archive,
        "link",
        0o120777,
        outside.to_str().unwrap().as_bytes(),
    );
    newc_entry(&mut archive, "link", 0o040700, b"");
    newc_entry(&mut archive, "link/inside.txt", 0o100644, b"data");
    newc_entry(&mut archive, "TRAILER!!!", 0, b"");
    let output = extract(dir.path(), &archive);
    let mode = std::fs::metadata(&outside).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o755);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let link = dir.path().join("out/link");
    assert!(link.symlink_metadata().unwrap().is_dir());
    assert_eq!(std::fs::read(link.join("inside.txt")).unwrap(), b"data");
    assert!(!outside.join("inside.txt").exists());
}

#[test]
fn extract_refuses_to_replace_an_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out");
    std::fs::create_dir(&out).unwrap();
    std::fs::write(out.join("a.txt"), b"keep").unwrap();
    let mut archive = Vec::new();
    newc_entry(&mut archive, "a.txt", 0o100644, b"new");
    newc_entry(&mut archive, "TRAILER!!!", 0, b"");
    let output = extract(dir.path(), &archive);
    assert!(!output.status.success(), "{output:?}");
    assert_eq!(std::fs::read(out.join("a.txt")).unwrap(), b"keep");
}

#[test]
fn cat_accepts_the_path_syntax_of_the_other_formats() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.cpio");
    let mut archive = Vec::new();
    newc_entry(&mut archive, "./etc/motd", 0o100644, b"hello");
    newc_entry(&mut archive, "TRAILER!!!", 0, b"");
    std::fs::write(&path, archive).unwrap();
    for name in ["/etc/motd", "etc/motd", "./etc/motd"] {
        let output = hadris("cpio")
            .arg("cat")
            .arg(&path)
            .arg(name)
            .output()
            .unwrap();
        assert!(output.status.success(), "{name}: {output:?}");
        assert_eq!(output.stdout, b"hello");
    }
}

/// The `hadris` binary with the format subcommand `format`.
fn hadris(format: &str) -> std::process::Command {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_hadris"));
    command.arg(format);
    command
}
