use std::process::Command;

#[test]
fn help_and_version_are_available() {
    let binary = env!("CARGO_BIN_EXE_hadris-cd");
    assert!(
        Command::new(binary)
            .arg("--help")
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new(binary)
            .arg("--version")
            .status()
            .unwrap()
            .success()
    );
}

#[test]
fn create_info_and_verify_bridge() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir_all(source.join("docs")).unwrap();
    std::fs::write(source.join("empty.txt"), []).unwrap();
    std::fs::write(
        source.join("docs/readme.txt"),
        b"hello from both namespaces",
    )
    .unwrap();
    let image = temp.path().join("bridge.iso");
    let binary = env!("CARGO_BIN_EXE_hadris-cd");

    assert!(
        Command::new(binary)
            .args(["create", source.to_str().unwrap(), "--output"])
            .arg(&image)
            .args(["--volume-name", "BRIDGE_TEST"])
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new(binary)
            .arg("info")
            .arg(&image)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new(binary)
            .arg("verify")
            .arg(&image)
            .status()
            .unwrap()
            .success()
    );
}

#[test]
fn create_supports_efi_only_boot_catalog() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("efi.img"), vec![0xA5; 4096]).unwrap();
    let image = temp.path().join("efi-only.iso");
    let binary = env!("CARGO_BIN_EXE_hadris-cd");

    assert!(
        Command::new(binary)
            .args(["create", source.to_str().unwrap(), "--output"])
            .arg(&image)
            .args(["--efi-boot", "efi.img"])
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new(binary)
            .arg("info")
            .arg(&image)
            .status()
            .unwrap()
            .success()
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

    let output = Command::new(env!("CARGO_BIN_EXE_hadris-cd"))
        .args(["create", source.to_str().unwrap(), "--output"])
        .arg(temp.path().join("bridge.iso"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("symbolic links are not supported"));
}
