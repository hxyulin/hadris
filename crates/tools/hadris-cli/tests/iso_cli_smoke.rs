//! Smoke tests for the ISO CLI binary.

#[test]
fn help_succeeds() {
    let output = hadris("iso").arg("--help").output().expect("run --help");
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
fn rock_ridge_names_are_used_by_listing_commands() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("cidata");
    let image = temp.path().join("cidata.iso");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("user-data"), "#user-data\n").unwrap();
    std::fs::write(source.join("meta-data"), "#meta-data\n").unwrap();
    let create = hadris("iso")
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
        let output = hadris("iso")
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
    let create = hadris("iso")
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

    let output = hadris("iso")
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

/// The `hadris` binary with the format subcommand `format`.
fn hadris(format: &str) -> std::process::Command {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_hadris"));
    command.arg(format);
    command
}
