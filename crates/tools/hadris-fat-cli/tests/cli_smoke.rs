//! Smoke tests for the canonical and compatibility FAT binaries.

#[test]
fn help_succeeds() {
    let bin = env!("CARGO_BIN_EXE_hadris-fat");
    let output = std::process::Command::new(bin)
        .arg("--help")
        .output()
        .expect("run fatutil --help");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("info"));
    assert!(stdout.contains("verify"));
    assert!(stdout.contains("create"));
    assert!(stdout.contains("extract"));
}

#[test]
fn compatibility_alias_succeeds() {
    let bin = env!("CARGO_BIN_EXE_fatutil");
    let status = std::process::Command::new(bin)
        .arg("--version")
        .status()
        .expect("run --version");
    assert!(status.success());
}

#[test]
fn create_cat_and_extract_roundtrip() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir_all(source.join("nested")).unwrap();
    std::fs::write(source.join("nested/hello.txt"), b"hello from FAT").unwrap();
    let image = temp.path().join("disk.img");

    let status = std::process::Command::new(env!("CARGO_BIN_EXE_hadris-fat"))
        .args([
            "create",
            source.to_str().unwrap(),
            "--output",
            image.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_hadris-fat"))
        .args(["cat", image.to_str().unwrap(), "/nested/hello.txt"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"hello from FAT");

    let extracted = temp.path().join("out");
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_hadris-fat"))
        .args([
            "extract",
            image.to_str().unwrap(),
            "--output",
            extracted.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(
        std::fs::read(extracted.join("nested/hello.txt")).unwrap(),
        b"hello from FAT"
    );
}

#[cfg(unix)]
#[test]
fn create_rejects_symbolic_links() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("target.txt"), b"target").unwrap();
    symlink("target.txt", source.join("link.txt")).unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_hadris-fat"))
        .args([
            "create",
            source.to_str().unwrap(),
            "--output",
            temp.path().join("disk.img").to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Symbolic links"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
