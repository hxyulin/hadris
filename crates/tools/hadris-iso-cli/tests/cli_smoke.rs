//! Smoke tests for the ISO CLI binary.

#[test]
fn help_succeeds() {
    let bin = env!("CARGO_BIN_EXE_hadris-iso");
    let output = std::process::Command::new(bin)
        .arg("--help")
        .output()
        .expect("run hadris-iso-cli --help");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("info"));
    assert!(stdout.contains("create"));
}

#[test]
fn version_succeeds() {
    let bin = env!("CARGO_BIN_EXE_hadris-iso-cli");
    let status = std::process::Command::new(bin)
        .arg("--version")
        .status()
        .expect("run --version");
    assert!(status.success());
}

#[test]
fn rock_ridge_names_are_used_by_listing_commands() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("cidata");
    let image = temp.path().join("cidata.iso");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("user-data"), "#user-data\n").unwrap();
    std::fs::write(source.join("meta-data"), "#meta-data\n").unwrap();

    let bin = env!("CARGO_BIN_EXE_hadris-iso-cli");
    let create = std::process::Command::new(bin)
        .args(["create", "--joliet", "--rock-ridge", "--output"])
        .arg(&image)
        .arg(&source)
        .output()
        .expect("create test ISO");
    assert!(
        create.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&create.stderr)
    );

    for command in [["tree"], ["ls"]] {
        let output = std::process::Command::new(bin)
            .args(command)
            .arg(&image)
            .output()
            .expect("list test ISO");
        assert!(
            output.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("meta-data"), "stdout: {stdout}");
        assert!(stdout.contains("user-data"), "stdout: {stdout}");
        assert!(!stdout.contains("META_DAT"), "stdout: {stdout}");
        assert!(!stdout.contains("USER_DAT"), "stdout: {stdout}");
    }
}

#[test]
fn joliet_names_are_decoded_by_listing_commands() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let image = temp.path().join("joliet.iso");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("lowercase-long-name.txt"), "payload").unwrap();

    let bin = env!("CARGO_BIN_EXE_hadris-iso-cli");
    let create = std::process::Command::new(bin)
        .args(["create", "--joliet", "--output"])
        .arg(&image)
        .arg(&source)
        .output()
        .expect("create Joliet test ISO");
    assert!(
        create.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&create.stderr)
    );

    let output = std::process::Command::new(bin)
        .arg("tree")
        .arg(&image)
        .output()
        .expect("list Joliet test ISO");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("lowercase-long-name.txt"),
        "stdout: {stdout}"
    );
    assert!(!stdout.contains('\0'), "stdout contains NULs: {stdout:?}");
}
