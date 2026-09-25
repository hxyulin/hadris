//! Smoke tests for the UDF CLI binary.

#[test]
fn help_succeeds() {
    let output = hadris("udf").arg("--help").output().expect("run --help");
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

/// The `hadris` binary with the format subcommand `format`.
fn hadris(format: &str) -> std::process::Command {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_hadris"));
    command.arg(format);
    command
}
