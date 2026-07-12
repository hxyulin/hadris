//! Smoke tests for the cpioutil binary.

#[test]
fn help_succeeds() {
    let bin = env!("CARGO_BIN_EXE_cpioutil");
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
    assert!(stdout.contains("list") || stdout.contains("create"));
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
