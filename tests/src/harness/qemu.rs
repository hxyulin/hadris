use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub const PROGRAM: &str = "qemu-system-x86_64";

pub fn available() -> bool {
    super::command::program_available(PROGRAM, "--version")
}

/// Skips the calling test when QEMU is absent, or panics when
/// [`REQUIRE_TOOLS_ENV`](super::command::REQUIRE_TOOLS_ENV) is set.
pub fn require() -> bool {
    super::command::require_or_skip(PROGRAM, "--version")
}

/// Boots `iso` headlessly with the serial console on stdout and returns what
/// the guest printed. Returns as soon as the output contains `marker`, or when
/// QEMU exits or `timeout` expires; QEMU is killed and reaped either way.
pub fn boot_serial_output(iso: &Path, marker: &str, timeout: Duration) -> Option<String> {
    let mut child = Command::new(PROGRAM)
        .args([
            "-cdrom",
            iso.to_str()?,
            "-boot",
            "d",
            "-nographic",
            "-serial",
            "stdio",
            "-monitor",
            "none",
            "-no-reboot",
            "-m",
            "16",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;

    let output = Arc::new(Mutex::new(Vec::new()));
    let reader = {
        let output = Arc::clone(&output);
        let mut stdout = child.stdout.take()?;
        thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            while let Ok(read) = stdout.read(&mut chunk) {
                if read == 0 {
                    break;
                }
                output.lock().unwrap().extend_from_slice(&chunk[..read]);
            }
        })
    };

    let start = Instant::now();
    loop {
        if String::from_utf8_lossy(&output.lock().unwrap()).contains(marker) {
            break;
        }
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() > timeout => break,
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(_) => break,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    let _ = reader.join();

    let output = output.lock().unwrap();
    Some(String::from_utf8_lossy(&output).into_owned())
}
