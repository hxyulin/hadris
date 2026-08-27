use std::ffi::{OsStr, OsString};
use std::process::{Command, Output};

/// Environment variable that turns a missing external tool into a failure
/// instead of a skipped test. CI sets it on jobs that install the tools.
pub const REQUIRE_TOOLS_ENV: &str = "HADRIS_REQUIRE_EXTERNAL_TOOLS";

pub fn run_command(program: &str, args: Vec<OsString>) -> Result<Output, String> {
    run_command_with_env(program, args, &[])
}

pub fn run_command_with_env(
    program: &str,
    args: Vec<OsString>,
    env: &[(&str, &OsStr)],
) -> Result<Output, String> {
    let printable = args
        .iter()
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    let mut command = Command::new(program);
    command.args(&args).env("LC_ALL", "C.UTF-8");
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command
        .output()
        .map_err(|error| format!("failed to run {program} {printable}: {error}"))?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(format!(
            "{program} {printable} failed with {:?}\nstdout:\n{}\nstderr:\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

pub fn command_value(program: &str, arg: &str) -> Result<String, String> {
    let output = run_command(program, vec![arg.into()])?;
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_string())
        .map_err(|error| error.to_string())
}

pub fn program_available(program: &str, version_arg: &str) -> bool {
    Command::new(program)
        .arg(version_arg)
        .output()
        .is_ok_and(|output| output.status.success())
}

/// Returns whether `program` is usable. A missing program skips the calling
/// test unless [`REQUIRE_TOOLS_ENV`] is set, in which case it panics.
pub fn require_or_skip(program: &str, version_arg: &str) -> bool {
    if program_available(program, version_arg) {
        return true;
    }
    if std::env::var_os(REQUIRE_TOOLS_ENV).is_some() {
        panic!("required external tool {program} is unavailable");
    }
    eprintln!("skipping: {program} is not available");
    false
}
