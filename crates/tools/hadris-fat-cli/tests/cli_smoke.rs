//! Smoke tests for the fatutil binary.

#[test]
fn help_succeeds() {
    let bin = env!("CARGO_BIN_EXE_fatutil");
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
}

#[test]
fn version_succeeds() {
    let bin = env!("CARGO_BIN_EXE_fatutil");
    let status = std::process::Command::new(bin)
        .arg("--version")
        .status()
        .expect("run --version");
    assert!(status.success());
}
