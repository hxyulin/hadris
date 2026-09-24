//! Smoke tests for the canonical and compatibility CPIO binaries.

#[test]
fn help_succeeds() {
    let bin = env!("CARGO_BIN_EXE_hadris-cpio");
    let output = std::process::Command::new(bin)
        .arg("--help")
        .output()
        .expect("run cpioutil --help");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ls"));
}

#[test]
fn version_succeeds() {
    let bin = env!("CARGO_BIN_EXE_cpioutil");
    let status = std::process::Command::new(bin)
        .arg("--version")
        .status()
        .expect("run --version");
    assert!(status.success());
}

#[test]
fn legacy_list_alias_is_accepted() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_hadris-cpio"))
        .args(["list", "missing.cpio"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("unrecognized subcommand"));
}

#[test]
fn create_then_extract_through_pipes() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("src");
    std::fs::create_dir_all(source.join("sub")).unwrap();
    std::fs::write(source.join("sub/file.txt"), b"hello").unwrap();
    let bin = env!("CARGO_BIN_EXE_hadris-cpio");
    let archive = Command::new(bin)
        .args(["create", "-o", "-"])
        .arg(&source)
        .output()
        .unwrap();
    assert!(archive.status.success());
    let out = dir.path().join("out");
    let mut child = Command::new(bin)
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

#[test]
fn extract_refuses_names_outside_the_output() {
    let dir = tempfile::tempdir().unwrap();
    let mut archive = Vec::new();
    let name = b"../escape\0";
    archive.extend_from_slice(b"070701");
    for value in [
        1u32,
        0o100644,
        0,
        0,
        1,
        0,
        0,
        0,
        0,
        0,
        0,
        name.len() as u32,
        0,
    ] {
        archive.extend_from_slice(format!("{value:08X}").as_bytes());
    }
    archive.extend_from_slice(name);
    while archive.len() % 4 != 0 {
        archive.push(0);
    }
    let path = dir.path().join("evil.cpio");
    std::fs::write(&path, &archive).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_hadris-cpio"))
        .args(["extract", "-o"])
        .arg(dir.path().join("out"))
        .arg(&path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!dir.path().join("escape").exists());
}
