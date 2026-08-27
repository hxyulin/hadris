use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub const PROGRAM: &str = "qemu-system-x86_64";

pub fn available() -> bool {
    super::command::program_available(PROGRAM, "--version")
}

/// Boots `iso` headlessly with the serial console on stdout and returns what
/// the guest printed before it halted or the timeout expired.
pub fn boot_serial_output(iso: &Path, timeout: Duration) -> Option<String> {
    let mut child = Command::new(PROGRAM)
        .args([
            "-cdrom",
            iso.to_str()?,
            "-boot",
            "d",
            "-nographic",
            "-serial",
            "stdio",
            "-no-reboot",
            "-m",
            "16",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;

    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                break;
            }
            Ok(None) => thread::sleep(Duration::from_millis(100)),
            Err(_) => break,
        }
    }

    let mut stdout = String::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_string(&mut stdout);
    }
    Some(stdout)
}
