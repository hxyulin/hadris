//! Smoke tests for the UDF CLI binary.

#[test]
fn help_succeeds() {
    let bin = env!("CARGO_BIN_EXE_hadris-udf");
    let output = std::process::Command::new(bin)
        .arg("--help")
        .output()
        .expect("run hadris-udf-cli --help");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("info"));
    assert!(stdout.contains("cat"));
    assert!(stdout.contains("extract"));
}

#[test]
fn version_succeeds() {
    let bin = env!("CARGO_BIN_EXE_hadris-udf-cli");
    let status = std::process::Command::new(bin)
        .arg("--version")
        .status()
        .expect("run --version");
    assert!(status.success());
}
