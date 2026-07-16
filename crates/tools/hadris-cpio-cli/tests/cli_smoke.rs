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
